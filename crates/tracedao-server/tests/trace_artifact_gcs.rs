//! Round-trip test for the GCS-backed `RemoteTraceArtifactProvider` using the
//! in-memory `GcsObjectClient` shim.
//!
//! Mirrors the filesystem-provider round-trip test pattern in
//! `trace_artifact_store.rs` (search for
//! `file_remote_provider_persists_service_owned_remote_artifacts_across_instances`).

use std::sync::Arc;

use chrono::Utc;
use secrecy::SecretString;
use serde_json::json;
use tracedao_server::secrets::SecretsCrypto;
use tracedao_server::trace_artifact_gcs::{
    GcsRemoteTraceArtifactProvider, InMemoryGcsObjectClient,
};
use tracedao_server::trace_artifact_kek::LocalMasterKeyWrapper;
use tracedao_server::trace_artifact_store::{
    RemoteTraceArtifactProvider, ServiceOwnedTraceArtifactStore,
    TRACE_ARTIFACT_CIPHERTEXT_SCHEMA_V2, TraceArtifactInvalidationReason, TraceArtifactKind,
    TraceArtifactProviderConfig, TraceArtifactScope,
};

#[test]
fn gcs_remote_provider_put_get_round_trip_via_in_memory_object_client() {
    let key = tracedao_server::secrets::keychain::generate_master_key_hex();
    let crypto =
        SecretsCrypto::new(SecretString::from(key.clone())).expect("crypto for gcs round trip");
    let kek_crypto = SecretsCrypto::new(SecretString::from(key)).expect("kek crypto");
    let kek = LocalMasterKeyWrapper::new(kek_crypto, "trace-commons-gcs-test");

    let config = TraceArtifactProviderConfig::service_owned_remote("trace-commons-prod")
        .expect("remote provider config");

    let client = Arc::new(InMemoryGcsObjectClient::default());
    let provider = GcsRemoteTraceArtifactProvider::new(
        Arc::clone(&client),
        "trace-commons-prod-bucket",
        "trace-commons-prod",
    );

    let store = ServiceOwnedTraceArtifactStore::new(config, crypto, kek, provider);

    let scope = TraceArtifactScope::new("tenant:sha256:alpha", "submission-alpha");
    let payload = json!({"stored": "gcs", "round": "trip"});

    let receipt = store
        .put_scoped_json(
            &scope,
            TraceArtifactKind::ContributionEnvelope,
            "submitted-envelope",
            &payload,
        )
        .expect("gcs artifact writes");

    // Direct provider read to confirm the GCS provider round-trips the record
    // shape (object_ref + artifact) without going through the store.
    let direct_provider = GcsRemoteTraceArtifactProvider::new(
        Arc::clone(&client),
        "trace-commons-prod-bucket",
        "trace-commons-prod",
    );
    let record = direct_provider
        .read_encrypted_artifact(&receipt.object_ref)
        .expect("direct provider reads back record");
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

    // Full plaintext round-trip via the store confirms the GCS provider record
    // is byte-identical to what the store wrote.
    let round_trip: serde_json::Value = store
        .read_scoped_json(&scope, &receipt.object_ref)
        .expect("gcs artifact round-trips through KEK unwrap");
    assert_eq!(round_trip, payload);
}

fn seed_artifact(
    client: Arc<InMemoryGcsObjectClient>,
) -> (
    ServiceOwnedTraceArtifactStore<
        GcsRemoteTraceArtifactProvider<Arc<InMemoryGcsObjectClient>>,
        LocalMasterKeyWrapper,
    >,
    GcsRemoteTraceArtifactProvider<Arc<InMemoryGcsObjectClient>>,
    tracedao_server::trace_artifact_store::TraceArtifactPutReceipt,
    TraceArtifactScope,
) {
    let key = tracedao_server::secrets::keychain::generate_master_key_hex();
    let crypto =
        SecretsCrypto::new(SecretString::from(key.clone())).expect("crypto for gcs lifecycle");
    let kek_crypto = SecretsCrypto::new(SecretString::from(key)).expect("kek crypto");
    let kek = LocalMasterKeyWrapper::new(kek_crypto, "trace-commons-gcs-test");

    let config = TraceArtifactProviderConfig::service_owned_remote("trace-commons-prod")
        .expect("remote provider config");

    let provider = GcsRemoteTraceArtifactProvider::new(
        Arc::clone(&client),
        "trace-commons-prod-bucket",
        "trace-commons-prod",
    );
    let direct_provider = GcsRemoteTraceArtifactProvider::new(
        Arc::clone(&client),
        "trace-commons-prod-bucket",
        "trace-commons-prod",
    );
    let store = ServiceOwnedTraceArtifactStore::new(config, crypto, kek, provider);
    let scope = TraceArtifactScope::new("tenant:sha256:alpha", "submission-alpha");

    let receipt = store
        .put_scoped_json(
            &scope,
            TraceArtifactKind::ContributionEnvelope,
            "submitted-envelope",
            &json!({"stored": "gcs"}),
        )
        .expect("gcs artifact writes");

    (store, direct_provider, receipt, scope)
}

#[test]
fn gcs_remote_provider_invalidate_persists_invalidated_state() {
    let client = Arc::new(InMemoryGcsObjectClient::default());
    let (_store, provider, receipt, _scope) = seed_artifact(Arc::clone(&client));

    let invalidated_at = Utc::now();
    provider
        .invalidate_encrypted_artifact(
            &receipt.object_ref,
            TraceArtifactInvalidationReason::Revoked,
            invalidated_at,
        )
        .expect("gcs artifact invalidates");

    let record = provider
        .read_encrypted_artifact(&receipt.object_ref)
        .expect("invalidated record still readable");
    assert_eq!(record.invalidated_at, Some(invalidated_at));
}

#[test]
fn gcs_remote_provider_delete_reports_hit_then_miss() {
    let client = Arc::new(InMemoryGcsObjectClient::default());
    let (_store, provider, receipt, _scope) = seed_artifact(Arc::clone(&client));

    let deleted_at = Utc::now();
    let first = provider
        .delete_encrypted_artifact(&receipt.object_ref, deleted_at)
        .expect("first delete succeeds");
    assert!(first, "delete returns true when record existed");

    let second = provider
        .delete_encrypted_artifact(&receipt.object_ref, deleted_at)
        .expect("second delete is idempotent");
    assert!(!second, "delete returns false when record already gone");
}

#[test]
fn gcs_remote_provider_restore_reports_hit_then_miss() {
    let client = Arc::new(InMemoryGcsObjectClient::default());
    let (_store, provider, receipt, _scope) = seed_artifact(Arc::clone(&client));

    let deleted_at = Utc::now();
    assert!(
        provider
            .delete_encrypted_artifact(&receipt.object_ref, deleted_at)
            .expect("delete to set up restore")
    );

    let restored = provider
        .restore_deleted_encrypted_artifact(&receipt.object_ref)
        .expect("restore succeeds when deleted version exists");
    assert!(restored, "restore returns true when record was recovered");

    let record = provider
        .read_encrypted_artifact(&receipt.object_ref)
        .expect("restored record reads back");
    assert_eq!(record.object_ref, receipt.object_ref);

    // After successful restore, the deleted-side is empty so a second restore is a no-op.
    let again = provider
        .restore_deleted_encrypted_artifact(&receipt.object_ref)
        .expect("restore-with-nothing-to-restore returns false, not an error");
    assert!(!again);
}

#[test]
fn gcs_remote_provider_exposes_versioning_support_flag() {
    let client = Arc::new(InMemoryGcsObjectClient::default());
    let plain = GcsRemoteTraceArtifactProvider::new(
        Arc::clone(&client),
        "trace-commons-prod-bucket",
        "trace-commons-prod",
    );
    assert!(!plain.supports_versioning());

    let versioned = GcsRemoteTraceArtifactProvider::versioned(
        Arc::clone(&client),
        "trace-commons-prod-bucket",
        "trace-commons-prod",
    );
    assert!(versioned.supports_versioning());
}
