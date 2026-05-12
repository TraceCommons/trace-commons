//! Unit tests for `CloudKmsKeyWrapper` against the hermetic
//! `InMemoryCloudKmsClient`. The in-memory client uses real AES-256-GCM with
//! the canonical KEK-context hash bound as AAD, so the round-trip exercises
//! the same encryption-context binding pattern that the GCP Cloud KMS adapter
//! will use against `additionalAuthenticatedData`.

use tracedao_server::trace_artifact_kek::{
    CloudKmsClient, CloudKmsKeyWrapper, InMemoryCloudKmsClient, KekContext, KmsKeyWrapper,
};
use tracedao_server::trace_artifact_store::TraceArtifactKind;

const WRAPPER_KIND: &str = "gcp_cloud_kms";
const KEY_REF: &str = "projects/test/locations/global/keyRings/r/cryptoKeys/k";

fn fixture_ctx() -> KekContext {
    KekContext {
        tenant_storage_ref: "tenant-a-storage-ref".into(),
        artifact_kind: TraceArtifactKind::ContributionEnvelope,
    }
}

fn fixture_wrapper() -> CloudKmsKeyWrapper<InMemoryCloudKmsClient> {
    let client = InMemoryCloudKmsClient::new_with_master([0x42u8; 32], KEY_REF);
    CloudKmsKeyWrapper::new(client, WRAPPER_KIND)
}

#[test]
fn cloud_kms_wrapper_round_trips_dek() {
    let wrapper = fixture_wrapper();
    let dek = [7u8; 32];
    let ctx = fixture_ctx();
    let wrapped = wrapper.wrap_dek(&dek, &ctx).unwrap();
    assert_eq!(wrapped.wrapper_kind, WRAPPER_KIND);
    assert_eq!(wrapped.context_hash, ctx.canonical_hash());
    let recovered = wrapper.unwrap_dek(&wrapped, &ctx).unwrap();
    assert_eq!(dek, *recovered);
}

#[test]
fn unwrap_rejects_swapped_context() {
    let wrapper = fixture_wrapper();
    let ctx_a = fixture_ctx();
    let ctx_b = KekContext {
        tenant_storage_ref: "tenant-b".into(),
        artifact_kind: TraceArtifactKind::ContributionEnvelope,
    };
    let wrapped = wrapper.wrap_dek(&[0u8; 32], &ctx_a).unwrap();
    let err = wrapper.unwrap_dek(&wrapped, &ctx_b).unwrap_err();
    assert!(
        format!("{err}").contains("KekContextMismatch"),
        "expected KekContextMismatch, got: {err}"
    );
}

#[test]
fn unwrap_rejects_tampered_ciphertext() {
    let wrapper = fixture_wrapper();
    let ctx = fixture_ctx();
    let mut wrapped = wrapper.wrap_dek(&[0u8; 32], &ctx).unwrap();
    // Flip a byte in the base64 ciphertext. Decode/re-encode so the result
    // is still valid base64 but the underlying bytes differ.
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut raw = STANDARD.decode(&wrapped.ciphertext_base64).unwrap();
    let last = raw.len() - 1;
    raw[last] ^= 0x01;
    wrapped.ciphertext_base64 = STANDARD.encode(&raw);
    let err = wrapper.unwrap_dek(&wrapped, &ctx).unwrap_err();
    assert!(
        format!("{err}").contains("KekUnwrapFailed"),
        "expected KekUnwrapFailed, got: {err}"
    );
}

#[test]
fn unwrap_rejects_wrong_wrapper_kind() {
    let wrapper = fixture_wrapper();
    let ctx = fixture_ctx();
    let mut wrapped = wrapper.wrap_dek(&[0u8; 32], &ctx).unwrap();
    wrapped.wrapper_kind = "local_master_key".into();
    let err = wrapper.unwrap_dek(&wrapped, &ctx).unwrap_err();
    assert!(
        format!("{err}").contains("wrapper kind mismatch"),
        "expected wrapper kind mismatch, got: {err}"
    );
}

#[test]
fn cloud_kms_wrapper_is_production_trust_boundary() {
    let wrapper = fixture_wrapper();
    assert!(wrapper.is_production_trust_boundary());
    let status = wrapper.safe_status();
    assert_eq!(status.kind, WRAPPER_KIND);
    assert!(status.is_production_trust_boundary);
    // `key_ref_hash` is the sha256 of the operator-supplied key ref, prefixed
    // with `sha256:`. The raw key ref must never appear.
    assert!(status.key_ref_hash.starts_with("sha256:"));
    assert!(!status.key_ref_hash.contains(KEY_REF));
}

#[test]
fn safe_status_does_not_leak_raw_key_ref_or_master_key() {
    let wrapper = fixture_wrapper();
    let status = wrapper.safe_status();
    let rendered = serde_json::to_string(&status).expect("status serializes");
    assert!(
        !rendered.contains(KEY_REF),
        "raw key ref leaked into safe_status: {rendered}"
    );
}

#[test]
fn in_memory_client_aad_binding_holds_at_client_level() {
    // Sanity check that the in-memory client itself rejects a mismatched AAD,
    // matching the AAD semantics of real cloud KMS (GCP's
    // additionalAuthenticatedData rejects mismatches with INVALID_ARGUMENT).
    let client = InMemoryCloudKmsClient::new_with_master([0x42u8; 32], KEY_REF);
    let plaintext = [9u8; 32];
    let aad_a = b"context-hash-a";
    let aad_b = b"context-hash-b";
    let ct = client.encrypt(&plaintext, aad_a).unwrap();
    let pt = client.decrypt(&ct, aad_a).unwrap();
    assert_eq!(pt.as_slice(), &plaintext);
    let err = client.decrypt(&ct, aad_b).unwrap_err();
    assert!(format!("{err}").contains("decrypt failed"));
}
