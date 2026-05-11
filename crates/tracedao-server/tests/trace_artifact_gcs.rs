//! Round-trip test for the GCS-backed `RemoteTraceArtifactProvider` using the
//! in-memory `GcsObjectClient` shim.
//!
//! Mirrors the filesystem-provider round-trip test pattern in
//! `trace_artifact_store.rs` (search for
//! `file_remote_provider_persists_service_owned_remote_artifacts_across_instances`).

use std::sync::Arc;

use secrecy::SecretString;
use serde_json::json;
use tracedao_server::secrets::SecretsCrypto;
use tracedao_server::trace_artifact_gcs::{GcsRemoteTraceArtifactProvider, InMemoryGcsObjectClient};
use tracedao_server::trace_artifact_kek::LocalMasterKeyWrapper;
use tracedao_server::trace_artifact_store::{
    RemoteTraceArtifactProvider, ServiceOwnedTraceArtifactStore, TRACE_ARTIFACT_CIPHERTEXT_SCHEMA_V2,
    TraceArtifactKind, TraceArtifactProviderConfig, TraceArtifactScope,
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
