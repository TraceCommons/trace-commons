// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The artifact half of `pipeline_remote_restore` (spec 2026-10-08, B-D2):
//! a restore of a remote store's objects into a scratch store, measured.
//!
//! The copy works on ciphertext at the `RemoteTraceArtifactProvider` layer:
//! `read_encrypted_artifact` from the source, `put_encrypted_artifact` into
//! the scratch store, with the object ref unchanged. A copy that decrypted
//! and stored again would encrypt under a fresh DEK, and the stored bytes
//! would never match (PR #1293, gap 3). Nothing here holds plaintext except
//! `ServiceOwnedTraceArtifactStore::verify_scoped_artifact_decrypts`, which
//! drops it.
//!
//! What it measures, for `promote remote-restore`'s report:
//!
//! - the stored-object fingerprint of the source before the copy and of the
//!   scratch store after it (`stored_artifact_fingerprint`), the remote form
//!   of the local drill's `artifact_fingerprint`: SHA-256 over each object's
//!   key under the store and the SHA-256 of its bytes as stored, in key
//!   order;
//! - how many restored objects decrypt under the configured key wrapper;
//! - whether both buckets report object versioning on.
//!
//! Every error is a safe label: no key, bucket, or object name.

use sha2::{Digest, Sha256};

use crate::secrets::SecretsCrypto;
use crate::trace_artifact_gcs::{GcsObjectClient, GcsRemoteTraceArtifactProvider};
use crate::trace_artifact_kek::KmsKeyWrapper;
use crate::trace_artifact_store::{
    RemoteTraceArtifactProvider, ServiceOwnedTraceArtifactStore, TraceArtifactProviderConfig,
    TraceArtifactScope,
};

/// The copy step's own report, which `promote remote-restore` folds into
/// `trace_commons.pipeline_remote_restore_report.v1`.
pub const REMOTE_RESTORE_COPY_SCHEMA: &str = "trace_commons.pipeline_remote_restore_copy.v1";

/// A store's stored-object fingerprint and how many objects it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredArtifactFingerprint {
    pub object_count: usize,
    pub fingerprint: String,
}

/// What one restore measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRestoreCopy {
    pub object_count: usize,
    /// The source's fingerprint, taken before anything was copied.
    pub artifact_fingerprint: String,
    /// The scratch store's fingerprint, read back after the copy.
    pub restored_artifact_fingerprint: String,
    pub kek_unwrap_verified_count: usize,
    /// Both buckets report object versioning on.
    pub versioning_enabled: bool,
}

/// SHA-256 over each stored object's key (relative to the store, so the
/// same objects under another bucket or prefix fingerprint the same) and the
/// SHA-256 of its stored bytes, in key order.
pub fn stored_artifact_fingerprint<C: GcsObjectClient>(
    provider: &GcsRemoteTraceArtifactProvider<C>,
) -> anyhow::Result<StoredArtifactFingerprint> {
    let keys = provider
        .stored_object_keys()
        .map_err(|_| anyhow::anyhow!("remote_restore_list_failed"))?;
    let mut digest = Sha256::new();
    for key in &keys {
        let bytes = provider
            .stored_object_bytes(key)
            .map_err(|_| anyhow::anyhow!("remote_restore_read_failed"))?;
        digest.update(key.as_bytes());
        digest.update(Sha256::digest(&bytes));
    }
    Ok(StoredArtifactFingerprint {
        object_count: keys.len(),
        fingerprint: format!("sha256:{}", hex::encode(digest.finalize())),
    })
}

/// Restores every object `source` stores into `scratch`, which must hold
/// nothing yet (`remote_restore_scratch_not_empty`), and measures the
/// restore. `object_store` is the store name the objects' refs carry; the
/// unwrap check reads them through a `ServiceOwnedTraceArtifactStore` over
/// `scratch` with `crypto` and `kek`. An object that does not decrypt is not
/// counted; it does not stop the copy, so the count says how many did.
pub fn restore_remote_artifacts<S, D, K>(
    source: &GcsRemoteTraceArtifactProvider<S>,
    scratch: GcsRemoteTraceArtifactProvider<D>,
    object_store: &str,
    crypto: SecretsCrypto,
    kek: K,
) -> anyhow::Result<RemoteRestoreCopy>
where
    S: GcsObjectClient,
    D: GcsObjectClient,
    K: KmsKeyWrapper,
{
    let occupied = scratch
        .stored_object_keys()
        .map_err(|_| anyhow::anyhow!("remote_restore_list_failed"))?;
    anyhow::ensure!(occupied.is_empty(), "remote_restore_scratch_not_empty");
    let versioning_enabled = source
        .bucket_versioning_enabled()
        .map_err(|_| anyhow::anyhow!("remote_restore_versioning_unreadable"))?
        && scratch
            .bucket_versioning_enabled()
            .map_err(|_| anyhow::anyhow!("remote_restore_versioning_unreadable"))?;

    let before = stored_artifact_fingerprint(source)?;
    let keys = source
        .stored_object_keys()
        .map_err(|_| anyhow::anyhow!("remote_restore_list_failed"))?;
    anyhow::ensure!(
        keys.len() == before.object_count,
        "remote_restore_source_changed"
    );
    let mut refs = Vec::with_capacity(keys.len());
    for key in &keys {
        let object_ref = source
            .stored_object_ref(key)
            .map_err(|_| anyhow::anyhow!("remote_restore_read_failed"))?;
        let record = source
            .read_encrypted_artifact(&object_ref)
            .map_err(|_| anyhow::anyhow!("remote_restore_read_failed"))?;
        scratch
            .put_encrypted_artifact(object_ref.clone(), record.artifact)
            .map_err(|_| anyhow::anyhow!("remote_restore_write_failed"))?;
        refs.push(object_ref);
    }
    let after = stored_artifact_fingerprint(&scratch)?;
    anyhow::ensure!(
        stored_artifact_fingerprint(source)? == before,
        "remote_restore_source_changed"
    );

    let store = ServiceOwnedTraceArtifactStore::new(
        TraceArtifactProviderConfig::service_owned_remote(object_store)
            .map_err(|_| anyhow::anyhow!("remote_restore_object_store_invalid"))?,
        crypto,
        kek,
        scratch,
    );
    let kek_unwrap_verified_count = refs
        .iter()
        .filter(|object_ref| {
            let scope = TraceArtifactScope::new(
                object_ref.tenant_storage_ref.clone(),
                object_ref.submission_storage_ref.clone(),
            );
            store
                .verify_scoped_artifact_decrypts(&scope, object_ref)
                .is_ok()
        })
        .count();

    Ok(RemoteRestoreCopy {
        object_count: before.object_count,
        artifact_fingerprint: before.fingerprint,
        restored_artifact_fingerprint: after.fingerprint,
        kek_unwrap_verified_count,
        versioning_enabled,
    })
}
