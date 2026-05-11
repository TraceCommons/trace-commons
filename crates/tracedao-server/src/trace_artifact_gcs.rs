use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::trace_artifact_store::{
    EncryptedTraceArtifact, RemoteTraceArtifactProvider, RemoteTraceArtifactRecord,
    TraceArtifactInvalidationReason, TraceArtifactObjectRef, validate_file_remote_object_ref,
    verify_encrypted_artifact,
};

pub struct GcsObjectFetch {
    pub body: Bytes,
    pub metadata: BTreeMap<String, String>,
}

pub trait GcsObjectClient: Send + Sync {
    fn put_object(
        &self,
        key: &str,
        body: Bytes,
        metadata: BTreeMap<String, String>,
    ) -> anyhow::Result<()>;
    fn get_object(&self, key: &str) -> anyhow::Result<GcsObjectFetch>;
    fn delete_object(&self, key: &str) -> anyhow::Result<bool>;
    fn restore_deleted_object(&self, key: &str) -> anyhow::Result<bool>;
}

impl<T: GcsObjectClient + ?Sized> GcsObjectClient for Arc<T> {
    fn put_object(
        &self,
        key: &str,
        body: Bytes,
        metadata: BTreeMap<String, String>,
    ) -> anyhow::Result<()> {
        (**self).put_object(key, body, metadata)
    }

    fn get_object(&self, key: &str) -> anyhow::Result<GcsObjectFetch> {
        (**self).get_object(key)
    }

    fn delete_object(&self, key: &str) -> anyhow::Result<bool> {
        (**self).delete_object(key)
    }

    fn restore_deleted_object(&self, key: &str) -> anyhow::Result<bool> {
        (**self).restore_deleted_object(key)
    }
}

#[derive(Default)]
pub struct InMemoryGcsObjectClient {
    live: Mutex<BTreeMap<String, (Bytes, BTreeMap<String, String>)>>,
    deleted: Mutex<BTreeMap<String, (Bytes, BTreeMap<String, String>)>>,
}

impl GcsObjectClient for InMemoryGcsObjectClient {
    fn put_object(
        &self,
        key: &str,
        body: Bytes,
        metadata: BTreeMap<String, String>,
    ) -> anyhow::Result<()> {
        self.live
            .lock()
            .unwrap()
            .insert(key.to_string(), (body, metadata));
        Ok(())
    }

    fn get_object(&self, key: &str) -> anyhow::Result<GcsObjectFetch> {
        let live = self.live.lock().unwrap();
        let (body, metadata) = live
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("GcsGetFailed: not found"))?;
        Ok(GcsObjectFetch {
            body: body.clone(),
            metadata: metadata.clone(),
        })
    }

    fn delete_object(&self, key: &str) -> anyhow::Result<bool> {
        if let Some(record) = self.live.lock().unwrap().remove(key) {
            self.deleted
                .lock()
                .unwrap()
                .insert(key.to_string(), record);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn restore_deleted_object(&self, key: &str) -> anyhow::Result<bool> {
        if let Some(record) = self.deleted.lock().unwrap().remove(key) {
            self.live.lock().unwrap().insert(key.to_string(), record);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// `RemoteTraceArtifactProvider` implementation that serializes the artifact +
/// object-ref into a single JSON object stored under one GCS key per
/// artifact. Object-key partitioning mirrors the filesystem-remote provider:
/// `{object_store_alias}/{tenant_storage_ref_hash}/{artifact_kind}/{object_key}`.
///
/// Task 9 implements `put_encrypted_artifact` + `read_encrypted_artifact` only.
/// The remaining trait methods bail with a Task-10 marker so call sites that
/// reach them fail-closed rather than silently succeeding.
pub struct GcsRemoteTraceArtifactProvider<C> {
    client: C,
    /// Bucket label, stored for prod call sites that will need it once the
    /// real GCS client lands in Task 11. Not used by the in-memory client.
    #[allow(dead_code)]
    bucket: String,
    object_store_alias: String,
    #[allow(dead_code)]
    require_versioning: bool,
}

impl<C: GcsObjectClient> GcsRemoteTraceArtifactProvider<C> {
    pub fn new(
        client: C,
        bucket: impl Into<String>,
        object_store_alias: impl Into<String>,
    ) -> Self {
        Self {
            client,
            bucket: bucket.into(),
            object_store_alias: object_store_alias.into(),
            require_versioning: false,
        }
    }

    pub fn versioned(
        client: C,
        bucket: impl Into<String>,
        object_store_alias: impl Into<String>,
    ) -> Self {
        Self {
            client,
            bucket: bucket.into(),
            object_store_alias: object_store_alias.into(),
            require_versioning: true,
        }
    }

    fn object_key(&self, object_ref: &TraceArtifactObjectRef) -> String {
        let tenant_hash = sha256_hex_text(&object_ref.tenant_storage_ref);
        format!(
            "{}/{}/{}/{}",
            self.object_store_alias,
            tenant_hash,
            object_ref.artifact_kind.as_path_segment(),
            object_ref.object_key,
        )
    }
}

fn sha256_hex_text(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    hex::encode(digest)
}

/// Serialized GCS payload. Mirrors `FileRemoteTraceArtifactRecord` in
/// `trace_artifact_store.rs` so the JSON shape on the wire stays consistent
/// across providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GcsRecord {
    object_ref: TraceArtifactObjectRef,
    artifact: EncryptedTraceArtifact,
    invalidated_at: Option<DateTime<Utc>>,
    invalidation_reason: Option<TraceArtifactInvalidationReason>,
}

impl<C: GcsObjectClient> RemoteTraceArtifactProvider for GcsRemoteTraceArtifactProvider<C> {
    fn put_encrypted_artifact(
        &self,
        object_ref: TraceArtifactObjectRef,
        artifact: EncryptedTraceArtifact,
    ) -> anyhow::Result<()> {
        validate_file_remote_object_ref(&object_ref)?;
        verify_encrypted_artifact(
            &artifact,
            object_ref.tenant_storage_ref.as_str(),
            &object_ref.artifact_kind,
            object_ref.object_key.as_str(),
            object_ref.ciphertext_sha256.as_str(),
        )?;
        let key = self.object_key(&object_ref);
        let record = GcsRecord {
            object_ref,
            artifact,
            invalidated_at: None,
            invalidation_reason: None,
        };
        let body = serde_json::to_vec(&record)
            .map_err(|err| anyhow::anyhow!("GcsPutFailed: serialize record: {err}"))?;
        self.client
            .put_object(&key, Bytes::from(body), BTreeMap::new())
            .map_err(|err| anyhow::anyhow!("GcsPutFailed: {err}"))
    }

    fn read_encrypted_artifact(
        &self,
        object_ref: &TraceArtifactObjectRef,
    ) -> anyhow::Result<RemoteTraceArtifactRecord> {
        validate_file_remote_object_ref(object_ref)?;
        let key = self.object_key(object_ref);
        let fetch = self
            .client
            .get_object(&key)
            .map_err(|err| anyhow::anyhow!("GcsGetFailed: {err}"))?;
        let record: GcsRecord = serde_json::from_slice(&fetch.body)
            .map_err(|err| anyhow::anyhow!("GcsGetFailed: parse record: {err}"))?;
        anyhow::ensure!(
            record.object_ref == *object_ref,
            "GcsGetFailed: remote trace artifact object ref mismatch"
        );
        Ok(RemoteTraceArtifactRecord {
            object_ref: record.object_ref,
            artifact: record.artifact,
            invalidated_at: record.invalidated_at,
        })
    }

    fn invalidate_encrypted_artifact(
        &self,
        _object_ref: &TraceArtifactObjectRef,
        _reason: TraceArtifactInvalidationReason,
        _invalidated_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        anyhow::bail!("GcsInvalidateFailed: not yet implemented (Task 10)")
    }

    fn delete_encrypted_artifact(
        &self,
        _object_ref: &TraceArtifactObjectRef,
        _deleted_at: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        anyhow::bail!("GcsDeleteFailed: not yet implemented (Task 10)")
    }

    fn restore_deleted_encrypted_artifact(
        &self,
        _object_ref: &TraceArtifactObjectRef,
    ) -> anyhow::Result<bool> {
        anyhow::bail!("GcsRestoreFailed: not yet implemented (Task 10)")
    }
}
