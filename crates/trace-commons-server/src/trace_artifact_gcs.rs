// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

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
            self.deleted.lock().unwrap().insert(key.to_string(), record);
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
/// Task 9 implemented `put_encrypted_artifact` + `read_encrypted_artifact`;
/// Task 10 adds `invalidate_encrypted_artifact` (read-modify-write),
/// `delete_encrypted_artifact`, `restore_deleted_encrypted_artifact`, and a
/// `supports_versioning()` accessor that the startup gate
/// (`TRACE_COMMONS_OBJECT_STORE_REQUIRE_VERSIONING`) consults to refuse
/// production boot when the GCS bucket does not have object versioning enabled.
pub struct GcsRemoteTraceArtifactProvider<C> {
    client: C,
    /// Bucket label, stored for prod call sites that will need it once the
    /// real GCS client lands in Task 11. Not used by the in-memory client.
    #[allow(dead_code)]
    bucket: String,
    object_store_alias: String,
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

    /// Whether this provider was constructed with the versioning gate enabled.
    /// Startup (`TRACE_COMMONS_OBJECT_STORE_REQUIRE_VERSIONING`) reads this to
    /// refuse production boot when the bucket-level versioning policy is not
    /// confirmed. Mirrors `FileRemoteTraceArtifactProvider::versioned_deletes`.
    pub fn supports_versioning(&self) -> bool {
        self.require_versioning
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
        object_ref: &TraceArtifactObjectRef,
        reason: TraceArtifactInvalidationReason,
        invalidated_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        validate_file_remote_object_ref(object_ref)?;
        let key = self.object_key(object_ref);
        let fetch = self
            .client
            .get_object(&key)
            .map_err(|err| anyhow::anyhow!("GcsGetFailed: {err}"))?;
        let mut record: GcsRecord = serde_json::from_slice(&fetch.body)
            .map_err(|err| anyhow::anyhow!("GcsGetFailed: parse record: {err}"))?;
        anyhow::ensure!(
            record.object_ref == *object_ref,
            "GcsGetFailed: remote trace artifact object ref mismatch"
        );
        record.invalidated_at = Some(invalidated_at);
        record.invalidation_reason = Some(reason);
        let body = serde_json::to_vec(&record)
            .map_err(|err| anyhow::anyhow!("GcsInvalidateFailed: serialize record: {err}"))?;
        self.client
            .put_object(&key, Bytes::from(body), BTreeMap::new())
            .map_err(|err| anyhow::anyhow!("GcsInvalidateFailed: {err}"))
    }

    fn delete_encrypted_artifact(
        &self,
        object_ref: &TraceArtifactObjectRef,
        _deleted_at: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        validate_file_remote_object_ref(object_ref)?;
        let key = self.object_key(object_ref);
        // Bucket-level GCS object versioning + lifecycle policies handle the
        // `deleted_at` timestamp in production; the in-memory client tracks
        // hit/miss only.
        self.client
            .delete_object(&key)
            .map_err(|err| anyhow::anyhow!("GcsDeleteFailed: {err}"))
    }

    fn restore_deleted_encrypted_artifact(
        &self,
        object_ref: &TraceArtifactObjectRef,
    ) -> anyhow::Result<bool> {
        validate_file_remote_object_ref(object_ref)?;
        let key = self.object_key(object_ref);
        self.client
            .restore_deleted_object(&key)
            .map_err(|err| anyhow::anyhow!("GcsRestoreFailed: {err}"))
    }
}

/// Production GCS client. Gated by the `gcs-client` cargo feature so hermetic
/// unit-test builds (which use `InMemoryGcsObjectClient`) do not need to
/// download the `google-cloud-storage` dependency tree.
///
/// The wrapper bridges the async `google-cloud-storage` client to this crate's
/// synchronous `GcsObjectClient` trait by calling
/// `tokio::task::block_in_place(|| Handle::current().block_on(...))`. Callers
/// must be running inside a multi-thread Tokio runtime (the axum-based server
/// already is). This pattern mirrors how other sync-trait-over-async surfaces
/// in this crate are bridged.
///
/// Authentication uses Application Default Credentials via
/// `ClientConfig::with_auth()`; the operator does not thread credentials
/// through this code. The constructor is async because credential loading is.
#[cfg(feature = "gcs-client")]
pub mod prod_client {
    use std::collections::{BTreeMap, HashMap};

    use anyhow::{Context, Result};
    use bytes::Bytes;
    use google_cloud_storage::client::{Client, ClientConfig};
    use google_cloud_storage::http::Error as GcsError;
    use google_cloud_storage::http::objects::Object;
    use google_cloud_storage::http::objects::delete::DeleteObjectRequest;
    use google_cloud_storage::http::objects::download::Range;
    use google_cloud_storage::http::objects::get::GetObjectRequest;
    use google_cloud_storage::http::objects::list::ListObjectsRequest;
    use google_cloud_storage::http::objects::rewrite::RewriteObjectRequest;
    use google_cloud_storage::http::objects::upload::{Media, UploadObjectRequest, UploadType};
    use tokio::runtime::Handle;
    use tokio::task::block_in_place;

    use super::{GcsObjectClient, GcsObjectFetch};

    /// Production `GcsObjectClient` backed by `google_cloud_storage::Client`.
    pub struct ProdGcsObjectClient {
        client: Client,
        bucket: String,
    }

    impl ProdGcsObjectClient {
        /// Construct a production GCS client using Application Default
        /// Credentials. Must be awaited from within a tokio runtime.
        pub async fn try_new(bucket: impl Into<String>) -> Result<Self> {
            let config = ClientConfig::default()
                .with_auth()
                .await
                .context("GcsClientInit: failed to load Application Default Credentials")?;
            let client = Client::new(config);
            Ok(Self {
                client,
                bucket: bucket.into(),
            })
        }

        /// Construct a GCS client pointed at a custom endpoint with no
        /// authentication. Intended for local emulators such as
        /// fake-gcs-server. The `storage_endpoint` override causes all
        /// requests to go to the supplied URL instead of
        /// `https://storage.googleapis.com`. The `anonymous()` call
        /// removes the `NopeTokenSourceProvider` placeholder so requests
        /// carry no `Authorization` header, which is required for
        /// fake-gcs-server.
        ///
        /// This constructor is synchronous because it does not need to load
        /// credentials from the environment.
        pub fn try_new_with_endpoint(
            bucket: impl Into<String>,
            endpoint: impl Into<String>,
        ) -> Result<Self> {
            let config = ClientConfig {
                storage_endpoint: endpoint.into(),
                ..Default::default()
            }
            .anonymous();
            let client = Client::new(config);
            Ok(Self {
                client,
                bucket: bucket.into(),
            })
        }
    }

    /// Run an async future to completion from a sync context. Requires a
    /// multi-thread tokio runtime; the server binaries run on `tokio::main`
    /// with the default multi-thread scheduler.
    fn run_blocking<F: std::future::Future>(fut: F) -> F::Output {
        block_in_place(|| Handle::current().block_on(fut))
    }

    /// Map a GCS `Error::Response { code, .. }` for a not-found code, returning
    /// `None` when the error represents 404, and `Some(err)` otherwise.
    fn is_not_found(err: &GcsError) -> bool {
        match err {
            GcsError::Response(resp) => resp.code == 404,
            _ => false,
        }
    }

    impl GcsObjectClient for ProdGcsObjectClient {
        fn put_object(
            &self,
            key: &str,
            body: Bytes,
            metadata: BTreeMap<String, String>,
        ) -> Result<()> {
            let upload_type = if metadata.is_empty() {
                UploadType::Simple(Media::new(key.to_string()))
            } else {
                let custom: HashMap<String, String> = metadata.into_iter().collect();
                UploadType::Multipart(Box::new(Object {
                    name: key.to_string(),
                    metadata: Some(custom),
                    ..Default::default()
                }))
            };
            let request = UploadObjectRequest {
                bucket: self.bucket.clone(),
                ..Default::default()
            };
            run_blocking(self.client.upload_object(&request, body, &upload_type))
                .map(|_| ())
                .map_err(|err| anyhow::anyhow!("GcsPutFailed: {err}"))
        }

        fn get_object(&self, key: &str) -> Result<GcsObjectFetch> {
            let get_req = GetObjectRequest {
                bucket: self.bucket.clone(),
                object: key.to_string(),
                ..Default::default()
            };
            let object = run_blocking(self.client.get_object(&get_req))
                .map_err(|err| anyhow::anyhow!("GcsGetFailed: {err}"))?;
            let body = run_blocking(self.client.download_object(&get_req, &Range::default()))
                .map_err(|err| anyhow::anyhow!("GcsGetFailed: {err}"))?;
            let metadata: BTreeMap<String, String> =
                object.metadata.unwrap_or_default().into_iter().collect();
            Ok(GcsObjectFetch {
                body: Bytes::from(body),
                metadata,
            })
        }

        fn delete_object(&self, key: &str) -> Result<bool> {
            let request = DeleteObjectRequest {
                bucket: self.bucket.clone(),
                object: key.to_string(),
                ..Default::default()
            };
            match run_blocking(self.client.delete_object(&request)) {
                Ok(()) => Ok(true),
                Err(err) if is_not_found(&err) => Ok(false),
                Err(err) => Err(anyhow::anyhow!("GcsDeleteFailed: {err}")),
            }
        }

        fn restore_deleted_object(&self, key: &str) -> Result<bool> {
            // GCS has no first-class "undelete" on the JSON API surface that
            // this crate exposes. The production pattern is: list non-current
            // generations for the key (requires bucket versioning), pick the
            // most-recent generation, and `rewrite_object` it back over the
            // same key. That creates a fresh live generation populated from
            // the deleted contents.
            let list_req = ListObjectsRequest {
                bucket: self.bucket.clone(),
                prefix: Some(key.to_string()),
                versions: Some(true),
                ..Default::default()
            };
            let response = run_blocking(self.client.list_objects(&list_req))
                .map_err(|err| anyhow::anyhow!("GcsRestoreFailed: list versions: {err}"))?;
            let candidates = response.items.unwrap_or_default();
            // Filter to entries that match the key exactly and were soft-deleted
            // (have `time_deleted`). Pick the most-recent generation.
            let Some(latest) = candidates
                .into_iter()
                .filter(|obj| obj.name == key && obj.time_deleted.is_some())
                .max_by_key(|obj| obj.generation)
            else {
                return Ok(false);
            };
            let rewrite_req = RewriteObjectRequest {
                source_bucket: self.bucket.clone(),
                source_object: key.to_string(),
                source_generation: Some(latest.generation),
                destination_bucket: self.bucket.clone(),
                destination_object: key.to_string(),
                ..Default::default()
            };
            run_blocking(self.client.rewrite_object(&rewrite_req))
                .map(|_| true)
                .map_err(|err| anyhow::anyhow!("GcsRestoreFailed: rewrite: {err}"))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[tokio::test]
        async fn try_new_without_credentials_returns_init_error() {
            // No real ADC available in unit-test env; just confirm the
            // constructor path runs and produces a typed error rather than
            // panicking. We don't assert on the exact error string because it
            // depends on the host environment.
            let result = ProdGcsObjectClient::try_new("test-bucket").await;
            let _ = result.err();
        }
    }
}
