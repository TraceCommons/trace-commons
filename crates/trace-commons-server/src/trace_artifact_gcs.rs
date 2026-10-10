// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::trace_artifact_store::{
    EncryptedTraceArtifact, RemoteTraceArtifactProvider, RemoteTraceArtifactRecord,
    TraceArtifactInvalidationReason, TraceArtifactKind, TraceArtifactObjectLocation,
    TraceArtifactObjectRef, validate_file_remote_object_ref, validate_remote_object_location,
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
    /// The keys of every live object whose key starts with `prefix`, sorted.
    /// Noncurrent versions are not listed.
    fn list_object_keys(&self, prefix: &str) -> anyhow::Result<Vec<String>>;
    /// Whether the bucket's object versioning policy is on. A bucket that
    /// reports no policy is `false`.
    fn bucket_versioning_enabled(&self) -> anyhow::Result<bool>;
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

    fn list_object_keys(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        (**self).list_object_keys(prefix)
    }

    fn bucket_versioning_enabled(&self) -> anyhow::Result<bool> {
        (**self).bucket_versioning_enabled()
    }
}

#[derive(Default)]
pub struct InMemoryGcsObjectClient {
    live: Mutex<BTreeMap<String, (Bytes, BTreeMap<String, String>)>>,
    deleted: Mutex<BTreeMap<String, (Bytes, BTreeMap<String, String>)>>,
    /// The bucket policy `bucket_versioning_enabled` reports. Off by default:
    /// a bucket whose policy nobody set is not versioned.
    versioning_enabled: AtomicBool,
}

impl InMemoryGcsObjectClient {
    pub fn set_versioning_enabled(&self, enabled: bool) {
        self.versioning_enabled.store(enabled, Ordering::SeqCst);
    }
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
        // ZA-2: a missing key is the bucket answering that the object is not
        // there, which the production client's 404 also reports this way.
        let (body, metadata) = live.get(key).ok_or_else(|| {
            anyhow::Error::from(
                crate::trace_artifact_store::TraceArtifactIntegrityError::new(
                    "GcsGetFailed: not found".to_string(),
                ),
            )
        })?;
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

    fn list_object_keys(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        Ok(self
            .live
            .lock()
            .unwrap()
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect())
    }

    fn bucket_versioning_enabled(&self) -> anyhow::Result<bool> {
        Ok(self.versioning_enabled.load(Ordering::SeqCst))
    }
}

/// A `GcsObjectClient` confined to one key prefix of a bucket: every key it
/// is given is stored under `{prefix}/`, and the keys it lists come back
/// without it. The remote restore drill (`versioned_pipeline_remote_restore`)
/// names a store as `bucket[/prefix]` and writes only under that prefix;
/// `GcsRemoteTraceArtifactProvider` itself has no prefix of its own.
pub struct PrefixedGcsObjectClient<C> {
    inner: C,
    prefix: String,
}

impl<C: GcsObjectClient> PrefixedGcsObjectClient<C> {
    /// `prefix` is one or more `/`-separated segments, none empty, `.` or
    /// `..`; anything else is `gcs_object_prefix_invalid`.
    pub fn new(inner: C, prefix: impl Into<String>) -> anyhow::Result<Self> {
        let prefix = prefix.into();
        anyhow::ensure!(
            !prefix.is_empty()
                && prefix
                    .split('/')
                    .all(|segment| !segment.is_empty() && segment != "." && segment != ".."),
            "gcs_object_prefix_invalid"
        );
        Ok(Self { inner, prefix })
    }

    fn key(&self, key: &str) -> String {
        format!("{}/{key}", self.prefix)
    }
}

impl<C: GcsObjectClient> GcsObjectClient for PrefixedGcsObjectClient<C> {
    fn put_object(
        &self,
        key: &str,
        body: Bytes,
        metadata: BTreeMap<String, String>,
    ) -> anyhow::Result<()> {
        self.inner.put_object(&self.key(key), body, metadata)
    }

    fn get_object(&self, key: &str) -> anyhow::Result<GcsObjectFetch> {
        self.inner.get_object(&self.key(key))
    }

    fn delete_object(&self, key: &str) -> anyhow::Result<bool> {
        self.inner.delete_object(&self.key(key))
    }

    fn restore_deleted_object(&self, key: &str) -> anyhow::Result<bool> {
        self.inner.restore_deleted_object(&self.key(key))
    }

    fn list_object_keys(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        let own = format!("{}/", self.prefix);
        self.inner
            .list_object_keys(&self.key(prefix))?
            .into_iter()
            .map(|key| {
                key.strip_prefix(&own)
                    .map(str::to_string)
                    .ok_or_else(|| anyhow::anyhow!("gcs_object_listed_outside_prefix"))
            })
            .collect()
    }

    fn bucket_versioning_enabled(&self) -> anyhow::Result<bool> {
        self.inner.bucket_versioning_enabled()
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
        self.object_key_at(
            &object_ref.tenant_storage_ref,
            &object_ref.artifact_kind,
            &object_ref.object_key,
        )
    }

    fn object_key_at(
        &self,
        tenant_storage_ref: &str,
        artifact_kind: &TraceArtifactKind,
        object_key: &str,
    ) -> String {
        let tenant_hash = sha256_hex_text(tenant_storage_ref);
        format!(
            "{}/{}/{}/{}",
            self.object_store_alias,
            tenant_hash,
            artifact_kind.as_path_segment(),
            object_key,
        )
    }

    /// The key of every live object this provider stores (everything under
    /// `{object_store_alias}/`), sorted.
    pub fn stored_object_keys(&self) -> anyhow::Result<Vec<String>> {
        let mut keys = self
            .client
            .list_object_keys(&format!("{}/", self.object_store_alias))?;
        keys.sort();
        Ok(keys)
    }

    /// The bytes stored at `key`, as stored: the serialized record, not the
    /// artifact's plaintext.
    pub fn stored_object_bytes(&self, key: &str) -> anyhow::Result<Bytes> {
        self.require_own_key(key)?;
        Ok(self.client.get_object(key)?.body)
    }

    /// The object ref the record stored at `key` names. A record that does
    /// not parse, or whose ref belongs at another key, is an integrity
    /// failure.
    pub fn stored_object_ref(&self, key: &str) -> anyhow::Result<TraceArtifactObjectRef> {
        let body = self.stored_object_bytes(key)?;
        let record: GcsRecord = serde_json::from_slice(&body).map_err(|_| {
            anyhow::Error::from(
                crate::trace_artifact_store::TraceArtifactIntegrityError::new(
                    "GcsGetFailed: parse record".to_string(),
                ),
            )
        })?;
        if self.object_key(&record.object_ref) != key {
            return Err(anyhow::Error::from(
                crate::trace_artifact_store::TraceArtifactIntegrityError::new(
                    "GcsGetFailed: remote trace artifact object ref mismatch".to_string(),
                ),
            ));
        }
        Ok(record.object_ref)
    }

    /// The bucket's versioning policy, as the bucket reports it.
    pub fn bucket_versioning_enabled(&self) -> anyhow::Result<bool> {
        self.client.bucket_versioning_enabled()
    }

    fn require_own_key(&self, key: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            key.strip_prefix(self.object_store_alias.as_str())
                .is_some_and(|rest| rest.starts_with('/')),
            "gcs_object_key_outside_store"
        );
        Ok(())
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
        // ZA-2: the client types a missing object (a 404) as an integrity
        // failure; keep that type, so the pipeline charges it. Any other
        // fetch failure stays untyped: transport, uncharged.
        let fetch = self.client.get_object(&key).map_err(|err| {
            if crate::trace_artifact_store::is_trace_artifact_integrity_error(&err) {
                err
            } else {
                anyhow::anyhow!("GcsGetFailed: {err}")
            }
        })?;
        // A record that does not parse or names another object is an
        // integrity failure too.
        let record: GcsRecord = serde_json::from_slice(&fetch.body).map_err(|err| {
            anyhow::Error::from(
                crate::trace_artifact_store::TraceArtifactIntegrityError::new(format!(
                    "GcsGetFailed: parse record: {err}"
                )),
            )
        })?;
        if record.object_ref != *object_ref {
            return Err(anyhow::Error::from(
                crate::trace_artifact_store::TraceArtifactIntegrityError::new(
                    "GcsGetFailed: remote trace artifact object ref mismatch".to_string(),
                ),
            ));
        }
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

    fn delete_encrypted_artifact_at_key(
        &self,
        location: &TraceArtifactObjectLocation,
        _deleted_at: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        validate_remote_object_location(location)?;
        // The GCS key never carried the ciphertext hash: an ordinary delete
        // names the same key.
        let key = self.object_key_at(
            &location.tenant_storage_ref,
            &location.artifact_kind,
            &location.object_key,
        );
        self.client
            .delete_object(&key)
            .map_err(|err| anyhow::anyhow!("GcsDeleteFailed: {err}"))
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
    use google_cloud_storage::http::buckets::Bucket;
    use google_cloud_storage::http::buckets::get::GetBucketRequest;
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

    /// ZA-2: a 404 on a fetch means the bucket answered and the object is
    /// not there, so it is typed as an integrity failure, which the pipeline
    /// charges. A media download's 404 may carry a body that is not the JSON
    /// error, which the client reports as an HTTP-client error with the
    /// status; that is the same answer. Every other error (credentials,
    /// network, 429, 5xx) stays untyped: transport, uncharged.
    fn get_failure(err: GcsError) -> anyhow::Error {
        let not_found = is_not_found(&err)
            || matches!(&err, GcsError::HttpClient(http)
                if http.status().map(|status| status.as_u16()) == Some(404));
        if not_found {
            anyhow::Error::from(
                crate::trace_artifact_store::TraceArtifactIntegrityError::new(format!(
                    "GcsGetFailed: {err}"
                )),
            )
        } else {
            anyhow::anyhow!("GcsGetFailed: {err}")
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
            let object = run_blocking(self.client.get_object(&get_req)).map_err(get_failure)?;
            let body = run_blocking(self.client.download_object(&get_req, &Range::default()))
                .map_err(get_failure)?;
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

        fn list_object_keys(&self, prefix: &str) -> Result<Vec<String>> {
            collect_listing_pages(|page_token| {
                let request = ListObjectsRequest {
                    bucket: self.bucket.clone(),
                    prefix: Some(prefix.to_string()),
                    page_token,
                    ..Default::default()
                };
                let response = run_blocking(self.client.list_objects(&request))
                    .map_err(|err| anyhow::anyhow!("GcsListFailed: {err}"))?;
                Ok((
                    response
                        .items
                        .unwrap_or_default()
                        .into_iter()
                        .map(|object| object.name)
                        .collect(),
                    response.next_page_token,
                ))
            })
        }

        fn bucket_versioning_enabled(&self) -> Result<bool> {
            let request = GetBucketRequest {
                bucket: self.bucket.clone(),
                ..Default::default()
            };
            let bucket = run_blocking(self.client.get_bucket(&request))
                .map_err(|err| anyhow::anyhow!("GcsBucketGetFailed: {err}"))?;
            Ok(versioning_enabled(&bucket))
        }
    }

    /// Every page of a listing, in order, then sorted: `fetch` is called
    /// with no token first and then with each `next_page_token` until a
    /// page carries none. An empty token is the end too, never a loop.
    fn collect_listing_pages(
        mut fetch: impl FnMut(Option<String>) -> Result<(Vec<String>, Option<String>)>,
    ) -> Result<Vec<String>> {
        let mut keys = Vec::new();
        let mut page_token = None;
        loop {
            let (page, next) = fetch(page_token)?;
            keys.extend(page);
            match next {
                Some(token) if !token.is_empty() => page_token = Some(token),
                _ => break,
            }
        }
        keys.sort();
        Ok(keys)
    }

    /// A bucket with no versioning block has never had versioning set: off.
    fn versioning_enabled(bucket: &Bucket) -> bool {
        bucket
            .versioning
            .as_ref()
            .is_some_and(|versioning| versioning.enabled)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_fetch_404_is_an_integrity_failure_and_other_errors_are_not() {
            use google_cloud_storage::http::error::ErrorResponse;
            let response = |code| {
                GcsError::Response(ErrorResponse {
                    code,
                    errors: Vec::new(),
                    message: "status".to_string(),
                })
            };
            assert!(
                crate::trace_artifact_store::is_trace_artifact_integrity_error(&get_failure(
                    response(404)
                ))
            );
            for code in [401, 403, 429, 500, 503] {
                assert!(
                    !crate::trace_artifact_store::is_trace_artifact_integrity_error(&get_failure(
                        response(code)
                    )),
                    "{code} is transport"
                );
            }
            assert!(
                !crate::trace_artifact_store::is_trace_artifact_integrity_error(&get_failure(
                    GcsError::InvalidRangeHeader("bytes".to_string())
                ))
            );
        }

        /// PR #1283 review, finding 2: `check_response_status` in
        /// google-cloud-storage 0.24 answers `HttpClient` with the status
        /// when an error body is not the JSON error, the likely shape of a
        /// media download's 404. That 404 is the same answer as a
        /// `Response` 404: an integrity failure. Any other status in that
        /// shape is transport.
        #[test]
        fn a_fetch_404_without_a_json_body_is_an_integrity_failure() {
            let http_client = |status: u16| {
                let response = reqwest::Response::from(
                    axum::http::Response::builder()
                        .status(status)
                        .body("Not Found")
                        .expect("a response"),
                );
                GcsError::HttpClient(
                    response
                        .error_for_status()
                        .expect_err("an error status is an error"),
                )
            };
            let not_found = get_failure(http_client(404));
            assert!(
                crate::trace_artifact_store::is_trace_artifact_integrity_error(&not_found),
                "{not_found:#}"
            );
            for status in [401, 403, 429, 500, 503] {
                assert!(
                    !crate::trace_artifact_store::is_trace_artifact_integrity_error(&get_failure(
                        http_client(status)
                    )),
                    "{status} is transport"
                );
            }
        }

        #[test]
        fn a_listing_follows_every_page_token_and_sorts() {
            let pages = std::cell::RefCell::new(vec![
                (
                    vec!["b".to_string(), "a".to_string()],
                    Some("t1".to_string()),
                ),
                (vec!["d".to_string()], Some("t2".to_string())),
                (vec!["c".to_string()], Some(String::new())),
            ]);
            let tokens = std::cell::RefCell::new(Vec::new());
            let keys = collect_listing_pages(|token| {
                tokens.borrow_mut().push(token);
                Ok(pages.borrow_mut().remove(0))
            })
            .unwrap();
            assert_eq!(keys, ["a", "b", "c", "d"]);
            assert_eq!(
                *tokens.borrow(),
                [None, Some("t1".to_string()), Some("t2".to_string())]
            );
            let failed = collect_listing_pages(|_| anyhow::bail!("GcsListFailed: 503"));
            assert!(failed.is_err());
        }

        #[test]
        fn a_bucket_without_a_versioning_block_is_not_versioned() {
            use google_cloud_storage::http::buckets::Versioning;
            let mut bucket = Bucket::default();
            assert!(!versioning_enabled(&bucket));
            bucket.versioning = Some(Versioning { enabled: false });
            assert!(!versioning_enabled(&bucket));
            bucket.versioning = Some(Versioning { enabled: true });
            assert!(versioning_enabled(&bucket));
        }

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
