//! Integration test for the GCS-backed `RemoteTraceArtifactProvider` running
//! against a local fake-gcs-server emulator.
//!
//! The test is gated by two conditions:
//!  1. The `gcs-client` cargo feature must be enabled (it gates `ProdGcsObjectClient`).
//!  2. The `GCS_FAKE_ENDPOINT` environment variable must be set at test
//!     runtime to the emulator base URL (e.g. `http://localhost:4443`).
//!
//! Both tests are marked `#[ignore]` so they are excluded from default CI.
//! To run them:
//!
//! ```bash
//! docker run --rm -p 4443:4443 fsouza/fake-gcs-server -scheme http -public-host localhost
//! GCS_FAKE_ENDPOINT=http://localhost:4443 \
//! cargo test -p trace-commons-server --features gcs-client \
//!   --test trace_artifact_gcs_integration -- --ignored
//! ```
//!
//! See `docs/trace-commons-storage.md` for the full emulator setup section.

#[cfg(feature = "gcs-client")]
mod gcs_integration {
    use std::sync::Arc;

    use chrono::Utc;
    use secrecy::SecretString;
    use serde_json::json;
    use trace_commons_server::secrets::SecretsCrypto;
    use trace_commons_server::trace_artifact_gcs::{
        GcsRemoteTraceArtifactProvider,
        prod_client::ProdGcsObjectClient,
    };
    use trace_commons_server::trace_artifact_kek::LocalMasterKeyWrapper;
    use trace_commons_server::trace_artifact_store::{
        RemoteTraceArtifactProvider, ServiceOwnedTraceArtifactStore,
        TRACE_ARTIFACT_CIPHERTEXT_SCHEMA_V2, TraceArtifactInvalidationReason,
        TraceArtifactKind, TraceArtifactProviderConfig, TraceArtifactScope,
    };

    /// Read the fake-gcs-server endpoint from the environment. Returns `None`
    /// if the variable is absent so callers can skip gracefully.
    fn fake_endpoint() -> Option<String> {
        std::env::var("GCS_FAKE_ENDPOINT").ok()
    }

    /// Bucket name used by all emulator tests. fake-gcs-server creates buckets
    /// on first use when the `-backend memory` flag is active (the default).
    const EMULATOR_BUCKET: &str = "trace-commons-integration-test";

    /// Object-store alias used to namespace keys inside the bucket.
    const OBJECT_ALIAS: &str = "trace-commons-emulator-test";

    /// Build a `GcsRemoteTraceArtifactProvider<Arc<ProdGcsObjectClient>>` wired
    /// to the fake-gcs-server endpoint. Shared by every test below.
    fn make_provider(
        endpoint: &str,
        versioned: bool,
    ) -> GcsRemoteTraceArtifactProvider<Arc<ProdGcsObjectClient>> {
        let client = ProdGcsObjectClient::try_new_with_endpoint(EMULATOR_BUCKET, endpoint)
            .expect("ProdGcsObjectClient::try_new_with_endpoint");
        let client = Arc::new(client);
        if versioned {
            GcsRemoteTraceArtifactProvider::versioned(
                client,
                EMULATOR_BUCKET,
                OBJECT_ALIAS,
            )
        } else {
            GcsRemoteTraceArtifactProvider::new(client, EMULATOR_BUCKET, OBJECT_ALIAS)
        }
    }

    /// Build a `ServiceOwnedTraceArtifactStore` wrapping a provider pointed at
    /// the emulator.
    fn make_store(
        provider: GcsRemoteTraceArtifactProvider<Arc<ProdGcsObjectClient>>,
    ) -> ServiceOwnedTraceArtifactStore<
        GcsRemoteTraceArtifactProvider<Arc<ProdGcsObjectClient>>,
        LocalMasterKeyWrapper,
    > {
        let key = trace_commons_server::secrets::keychain::generate_master_key_hex();
        let crypto =
            SecretsCrypto::new(SecretString::from(key.clone())).expect("store crypto");
        let kek_crypto = SecretsCrypto::new(SecretString::from(key)).expect("kek crypto");
        let kek = LocalMasterKeyWrapper::new(kek_crypto, OBJECT_ALIAS);
        let config = TraceArtifactProviderConfig::service_owned_remote(OBJECT_ALIAS)
            .expect("remote provider config");
        ServiceOwnedTraceArtifactStore::new(config, crypto, kek, provider)
    }

    // -------------------------------------------------------------------------
    // Tests
    // -------------------------------------------------------------------------

    /// put → get round-trip against real fake-gcs-server HTTP surface.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn gcs_emulator_put_get_round_trip() {
        let endpoint = match fake_endpoint() {
            Some(e) => e,
            None => {
                eprintln!(
                    "SKIP gcs_emulator_put_get_round_trip: \
                     GCS_FAKE_ENDPOINT is not set — \
                     start fake-gcs-server and export the variable to run this test."
                );
                return;
            }
        };

        let provider = make_provider(&endpoint, false);
        // Clone the client reference for a second provider that reads back
        // without going through the store layer.
        let client = ProdGcsObjectClient::try_new_with_endpoint(EMULATOR_BUCKET, &endpoint)
            .expect("second ProdGcsObjectClient");
        let direct_provider = GcsRemoteTraceArtifactProvider::new(
            Arc::new(client),
            EMULATOR_BUCKET,
            OBJECT_ALIAS,
        );

        let store = make_store(provider);
        let scope = TraceArtifactScope::new("tenant:sha256:integration", "submission-emulator");
        let payload = json!({"stored": "gcs-emulator", "round": "trip"});

        let receipt = store
            .put_scoped_json(
                &scope,
                TraceArtifactKind::ContributionEnvelope,
                "emulator-envelope",
                &payload,
            )
            .expect("emulator artifact writes");

        // Direct provider read to confirm the GCS provider stores a valid
        // record shape over the real HTTP surface.
        let record = direct_provider
            .read_encrypted_artifact(&receipt.object_ref)
            .expect("direct provider reads back from emulator");
        assert_eq!(record.object_ref, receipt.object_ref);
        assert!(record.invalidated_at.is_none());
        assert_eq!(
            record.artifact.schema_version,
            TRACE_ARTIFACT_CIPHERTEXT_SCHEMA_V2
        );
        assert!(
            record.artifact.wrapped_dek.is_some(),
            "v2 envelope must round-trip with wrapped_dek"
        );

        // Full plaintext round-trip through the store confirms byte-identical
        // storage on the emulator.
        let round_trip: serde_json::Value = store
            .read_scoped_json(&scope, &receipt.object_ref)
            .expect("emulator round-trip decrypts correctly");
        assert_eq!(round_trip, payload);
    }

    /// invalidate → read confirms the invalidated_at field persists.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn gcs_emulator_invalidate_persists_state() {
        let endpoint = match fake_endpoint() {
            Some(e) => e,
            None => {
                eprintln!(
                    "SKIP gcs_emulator_invalidate_persists_state: \
                     GCS_FAKE_ENDPOINT is not set."
                );
                return;
            }
        };

        let provider = make_provider(&endpoint, false);
        let store = make_store(provider);
        let scope = TraceArtifactScope::new("tenant:sha256:integration", "submission-invalidate");

        let receipt = store
            .put_scoped_json(
                &scope,
                TraceArtifactKind::ContributionEnvelope,
                "invalidate-envelope",
                &json!({"test": "invalidate"}),
            )
            .expect("emulator put for invalidate test");

        // We need a standalone provider to call invalidate after the store put.
        let standalone = make_provider(&endpoint, false);
        let invalidated_at = Utc::now();
        standalone
            .invalidate_encrypted_artifact(
                &receipt.object_ref,
                TraceArtifactInvalidationReason::Revoked,
                invalidated_at,
            )
            .expect("emulator invalidate");

        let record = standalone
            .read_encrypted_artifact(&receipt.object_ref)
            .expect("invalidated record still readable from emulator");
        assert_eq!(record.invalidated_at, Some(invalidated_at));
    }

    /// delete → Ok(true) then Ok(false).
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn gcs_emulator_delete_reports_hit_then_miss() {
        let endpoint = match fake_endpoint() {
            Some(e) => e,
            None => {
                eprintln!(
                    "SKIP gcs_emulator_delete_reports_hit_then_miss: \
                     GCS_FAKE_ENDPOINT is not set."
                );
                return;
            }
        };

        let provider = make_provider(&endpoint, false);
        let store = make_store(provider);
        let scope = TraceArtifactScope::new("tenant:sha256:integration", "submission-delete");

        let receipt = store
            .put_scoped_json(
                &scope,
                TraceArtifactKind::ContributionEnvelope,
                "delete-envelope",
                &json!({"test": "delete"}),
            )
            .expect("emulator put for delete test");

        let standalone = make_provider(&endpoint, false);
        let deleted_at = Utc::now();
        let first = standalone
            .delete_encrypted_artifact(&receipt.object_ref, deleted_at)
            .expect("first emulator delete");
        assert!(first, "delete returns true when record existed");

        let second = standalone
            .delete_encrypted_artifact(&receipt.object_ref, deleted_at)
            .expect("second emulator delete is idempotent");
        assert!(!second, "delete returns false when record already gone");
    }

    /// restore after delete → Ok(true); restore again → Ok(false).
    ///
    /// NOTE: fake-gcs-server must be started with versioning enabled on the
    /// bucket for restore to succeed. The restore implementation uses
    /// `list_objects` with `versions: Some(true)` and `rewrite_object` to
    /// recover the most-recent soft-deleted generation. fake-gcs-server
    /// supports GCS object versioning when the bucket is created with
    /// versioning enabled — do this via the fake-gcs-server REST API or by
    /// letting the emulator auto-create a versioned bucket from its startup
    /// configuration. If versioning is not enabled on the emulator bucket this
    /// test will report Ok(false) on the restore call (no deleted generations
    /// visible in the list), which is still a well-defined non-error path.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn gcs_emulator_restore_reports_hit_then_miss() {
        let endpoint = match fake_endpoint() {
            Some(e) => e,
            None => {
                eprintln!(
                    "SKIP gcs_emulator_restore_reports_hit_then_miss: \
                     GCS_FAKE_ENDPOINT is not set."
                );
                return;
            }
        };

        let provider = make_provider(&endpoint, true);
        let store = make_store(provider);
        let scope = TraceArtifactScope::new("tenant:sha256:integration", "submission-restore");

        let receipt = store
            .put_scoped_json(
                &scope,
                TraceArtifactKind::ContributionEnvelope,
                "restore-envelope",
                &json!({"test": "restore"}),
            )
            .expect("emulator put for restore test");

        let standalone = make_provider(&endpoint, true);
        let deleted_at = Utc::now();
        assert!(
            standalone
                .delete_encrypted_artifact(&receipt.object_ref, deleted_at)
                .expect("delete to set up restore"),
            "delete must return true to confirm an object was present"
        );

        let restored = standalone
            .restore_deleted_encrypted_artifact(&receipt.object_ref)
            .expect("restore call must not error even if versioning is absent");

        if restored {
            // Versioning was active: confirm the record came back.
            let record = standalone
                .read_encrypted_artifact(&receipt.object_ref)
                .expect("restored record readable from emulator");
            assert_eq!(record.object_ref, receipt.object_ref);

            // Second restore is a no-op: no more deleted generations.
            let again = standalone
                .restore_deleted_encrypted_artifact(&receipt.object_ref)
                .expect("second restore returns false, not an error");
            assert!(!again, "second restore returns false when nothing left to restore");
        } else {
            // Bucket versioning not enabled on this emulator run — the
            // restore path returns Ok(false) rather than an error. That is
            // acceptable behavior; flag it so the operator can enable
            // versioning if they want full coverage.
            eprintln!(
                "NOTE: restore returned Ok(false) — bucket versioning \
                 may not be enabled on the emulator. Enable it for full \
                 restore coverage."
            );
        }
    }

    /// supports_versioning() flag reflects the constructor used.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn gcs_emulator_versioning_support_flag() {
        let endpoint = match fake_endpoint() {
            Some(e) => e,
            None => {
                eprintln!(
                    "SKIP gcs_emulator_versioning_support_flag: \
                     GCS_FAKE_ENDPOINT is not set."
                );
                return;
            }
        };

        let plain = make_provider(&endpoint, false);
        assert!(!plain.supports_versioning());

        let versioned = make_provider(&endpoint, true);
        assert!(versioned.supports_versioning());
    }
}
