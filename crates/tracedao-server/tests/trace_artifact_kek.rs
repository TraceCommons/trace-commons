use secrecy::SecretString;
use tracedao_server::secrets::SecretsCrypto;
use tracedao_server::trace_artifact_kek::{KekContext, KmsKeyWrapper, LocalMasterKeyWrapper};
use tracedao_server::trace_artifact_store::TraceArtifactKind;

fn fixture_crypto() -> SecretsCrypto {
    SecretsCrypto::new(SecretString::new("a".repeat(32).into())).unwrap()
}

#[test]
fn local_master_key_wrapper_round_trips_dek() {
    let wrapper = LocalMasterKeyWrapper::new(fixture_crypto(), "local-test".to_string());
    let dek = [7u8; 32];
    let ctx = KekContext {
        tenant_storage_ref: "tenant-a-storage-ref".into(),
        artifact_kind: TraceArtifactKind::ContributionEnvelope,
    };
    let wrapped = wrapper.wrap_dek(&dek, &ctx).unwrap();
    let recovered = wrapper.unwrap_dek(&wrapped, &ctx).unwrap();
    assert_eq!(dek, recovered);
    assert_eq!(wrapped.wrapper_kind, "local_master_key");
    assert_eq!(wrapped.context_hash, ctx.canonical_hash());
    assert!(!wrapper.is_production_trust_boundary());
}
