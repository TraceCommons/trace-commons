//! Trait-surface tests for `TraceGateService` and the dstack-stub `KmsKeyWrapper`.
//!
//! These tests exercise the foundation pieces of the dstack enclave slice: the
//! in-memory gate service is deterministic and passes; the dstack gate service
//! and dstack KEK wrapper fail closed with their stable error class names;
//! the dstack KEK wrapper still declares production trust boundary by type.

use trace_commons_server::trace_artifact_kek::{
    DstackKekWrapper, KekContext, KmsKeyWrapper, WrappedDek,
};
use trace_commons_server::trace_artifact_store::TraceArtifactKind;
use trace_commons_server::trace_gate_service::{
    DstackGateService, InMemoryGateService, LegacyDeterministicGateService, TenantCtx,
    TraceGateService,
};
use uuid::Uuid;

fn fixture_wrapped_dek() -> WrappedDek {
    WrappedDek {
        wrapper_kind: "local_master_key".into(),
        key_ref_hash: "sha256:fixture-key".into(),
        ciphertext_base64: "AAAA".into(),
        context_hash: KekContext {
            tenant_storage_ref: "fixture-tenant".into(),
            artifact_kind: TraceArtifactKind::ContributionEnvelope,
        }
        .canonical_hash(),
    }
}

fn fixture_tenant() -> TenantCtx {
    TenantCtx::new("fixture-tenant")
}

#[test]
fn in_memory_gate_service_is_deterministic_across_calls() {
    let svc = InMemoryGateService::new("test-policy-v1", "sha256:test-policy-v1");
    let wrapped = fixture_wrapped_dek();
    let tenant = fixture_tenant();
    let envelope = b"deterministic-fixture-envelope-ciphertext";

    let first = svc
        .evaluate_trace(
            &tenant,
            envelope,
            &wrapped,
            TraceArtifactKind::ContributionEnvelope,
        )
        .expect("evaluate_trace should succeed for in-memory service");
    let second = svc
        .evaluate_trace(
            &tenant,
            envelope,
            &wrapped,
            TraceArtifactKind::ContributionEnvelope,
        )
        .expect("evaluate_trace should succeed for in-memory service");

    assert_eq!(first, second);
    assert_eq!(first.gate_policy_version, "test-policy-v1");
    assert_eq!(first.gate_version_hash, "sha256:test-policy-v1");
}

#[test]
fn in_memory_gate_service_passes_perplexity_and_novelty() {
    let svc = InMemoryGateService::new("test-policy-v1", "sha256:test-policy-v1");
    let decision = svc
        .evaluate_trace(
            &fixture_tenant(),
            b"fixture",
            &fixture_wrapped_dek(),
            TraceArtifactKind::ContributionEnvelope,
        )
        .unwrap();
    assert!(decision.perplexity_passed);
    assert!(decision.novelty_passed);
    assert!(decision.nearest_neighbor_hash.starts_with("sha256:"));
    assert!(decision.embedding_evidence_hash.starts_with("sha256:"));
    assert!(decision.attestation_chain_hash.starts_with("sha256:"));
}

#[test]
fn in_memory_gate_service_invalidate_is_noop_success() {
    let svc = InMemoryGateService::new("test-policy-v1", "sha256:test-policy-v1");
    svc.invalidate_vector_entry(&fixture_tenant(), Uuid::new_v4())
        .expect("invalidate should succeed for in-memory service");
}

#[test]
fn legacy_deterministic_gate_service_stamps_legacy_version() {
    let svc = LegacyDeterministicGateService::new();
    let decision = svc
        .evaluate_trace(
            &fixture_tenant(),
            b"fixture",
            &fixture_wrapped_dek(),
            TraceArtifactKind::ContributionEnvelope,
        )
        .unwrap();
    assert_eq!(decision.gate_policy_version, "legacy_deterministic");
    assert_eq!(decision.gate_version_hash, "sha256:legacy_deterministic");
    assert!(decision.perplexity_passed);
    assert!(decision.novelty_passed);
    let status = svc.safe_status();
    assert_eq!(status.kind, "legacy_deterministic");
    assert!(!status.attestation_verifier_configured);
}

#[test]
fn dstack_gate_service_evaluate_fails_closed_with_stable_error() {
    let svc = DstackGateService::new("https://stub-endpoint.invalid", "fixture-verifier");
    let err = svc
        .evaluate_trace(
            &fixture_tenant(),
            b"fixture",
            &fixture_wrapped_dek(),
            TraceArtifactKind::ContributionEnvelope,
        )
        .expect_err("dstack gate service stub must fail closed");
    assert!(
        format!("{err}").contains("DstackGateServiceUnavailable"),
        "expected stable DstackGateServiceUnavailable class, got: {err}"
    );
}

#[test]
fn dstack_gate_service_invalidate_fails_closed_with_stable_error() {
    let svc = DstackGateService::new("https://stub-endpoint.invalid", "fixture-verifier");
    let err = svc
        .invalidate_vector_entry(&fixture_tenant(), Uuid::new_v4())
        .expect_err("dstack gate service stub must fail closed on invalidation");
    assert!(format!("{err}").contains("DstackGateServiceUnavailable"));
}

#[test]
fn dstack_gate_service_safe_status_reports_stub_kind() {
    let svc = DstackGateService::new("https://stub-endpoint.invalid", "fixture-verifier");
    let status = svc.safe_status();
    assert_eq!(status.kind, "dstack_stub");
    assert_eq!(status.gate_policy_version, "stub");
    assert_eq!(status.gate_version_hash, "sha256:stub");
    assert!(status.attestation_verifier_configured);
}

#[test]
fn dstack_kek_wrapper_wrap_fails_closed_with_stable_error() {
    let wrapper = DstackKekWrapper::new("fixture-enclave-measurement");
    let ctx = KekContext {
        tenant_storage_ref: "tenant-a".into(),
        artifact_kind: TraceArtifactKind::ContributionEnvelope,
    };
    let err = wrapper
        .wrap_dek(&[0u8; 32], &ctx)
        .expect_err("dstack KEK wrapper stub must fail closed on wrap");
    assert!(format!("{err}").contains("DstackKekUnavailable"));
}

#[test]
fn dstack_kek_wrapper_unwrap_fails_closed_with_stable_error() {
    let wrapper = DstackKekWrapper::new("fixture-enclave-measurement");
    let ctx = KekContext {
        tenant_storage_ref: "tenant-a".into(),
        artifact_kind: TraceArtifactKind::ContributionEnvelope,
    };
    let wrapped = WrappedDek {
        wrapper_kind: "dstack_kek".into(),
        key_ref_hash: "sha256:irrelevant".into(),
        ciphertext_base64: "AAAA".into(),
        context_hash: ctx.canonical_hash(),
    };
    let err = wrapper
        .unwrap_dek(&wrapped, &ctx)
        .expect_err("dstack KEK wrapper stub must fail closed on unwrap");
    assert!(format!("{err}").contains("DstackKekUnavailable"));
}

#[test]
fn dstack_kek_wrapper_claims_production_trust_boundary() {
    let wrapper = DstackKekWrapper::new("fixture-enclave-measurement");
    assert!(wrapper.is_production_trust_boundary());
    let status = wrapper.safe_status();
    assert_eq!(status.kind, "dstack_kek");
    assert!(status.is_production_trust_boundary);
    assert!(status.key_ref_hash.starts_with("sha256:"));
}
