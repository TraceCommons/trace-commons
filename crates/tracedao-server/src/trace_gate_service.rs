//! Trace gate service trait and supporting impls.
//!
//! The `TraceGateService` is the abstraction the vector-indexing worker calls
//! into to score a contributed trace for perplexity and novelty. The
//! production implementation will live inside a dstack-attested enclave; this
//! module ships the trait, an in-memory deterministic implementation for
//! tests, a fail-closed `DstackGateService` stub, and a
//! `LegacyDeterministicGateService` that preserves today's vector-worker
//! behavior so the default deployment is bit-identical to the pre-enclave
//! shape.
//!
//! No real LLM/embedder is invoked here. The `evaluate_trace` entry point on
//! the in-memory and legacy services emits a deterministic `GateDecision`
//! derived from a hash of the inputs so callers (and tests) see stable
//! audit-grade output without depending on a live model.

use std::sync::Arc;

use base64::Engine;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use tracedao_gate_enclave::{
    Embedder, EnclaveGateOrchestrator, EnclaveGateOrchestratorConfig, MockEmbedder,
    MockPerplexityScorer, MockVectorIndex, PerplexityScorer, VectorIndex,
};

use crate::trace_artifact_kek::{KekContext, KmsKeyWrapper, WrappedDek};
use crate::trace_artifact_store::{TraceArtifactKind, aead_decrypt_with_dek};

/// Minimal tenant context plumbed into the gate service. We intentionally
/// keep this struct independent of the binary-private `TenantAuth` type so
/// the trait stays defined in the library crate.
#[derive(Debug, Clone)]
pub struct TenantCtx {
    pub tenant_id: String,
}

impl TenantCtx {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
        }
    }
}

/// Result of running a trace through the gate service. The fields here mirror
/// the columns of `trace_gate_decisions` so callers can persist the decision
/// row without re-deriving any field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateDecision {
    pub gate_policy_version: String,
    pub gate_version_hash: String,
    pub perplexity_micros: u64,
    pub tail_fraction_micros: u64,
    pub perplexity_passed: bool,
    pub novelty_score_micros: u64,
    pub nearest_neighbor_hash: String,
    pub novelty_passed: bool,
    pub embedding_evidence_hash: String,
    pub attestation_chain_hash: String,
}

/// Observable status of a `TraceGateService`, safe for logs / health surfaces.
#[derive(Debug, Clone)]
pub struct GateServiceStatus {
    pub kind: String,
    pub gate_policy_version: String,
    pub gate_version_hash: String,
    pub attestation_verifier_configured: bool,
}

/// Pluggable gate-evaluation service.
///
/// Implementations score a trace against perplexity and novelty thresholds and
/// emit a `GateDecision`. Object safety is required so the running binary can
/// hold `Arc<dyn TraceGateService>` on app state — keep all methods
/// non-generic.
pub trait TraceGateService: Send + Sync {
    /// Score a wrapped trace against the gate policy. Implementations MUST
    /// stamp the returned decision with the gate-policy-version and
    /// gate-version-hash they were configured with so callers can persist it
    /// without further lookup.
    fn evaluate_trace(
        &self,
        tenant_ctx: &TenantCtx,
        envelope_ciphertext: &[u8],
        wrapped_dek: &WrappedDek,
        object_kind: TraceArtifactKind,
    ) -> anyhow::Result<GateDecision>;

    /// Mark a previously-indexed vector entry as invalidated inside the gate
    /// service (e.g., to drop it from the enclave's in-memory nearest-neighbor
    /// index after a revocation). The legacy and in-memory implementations
    /// are no-ops; the dstack implementation will eventually push the
    /// invalidation through to the enclave.
    fn invalidate_vector_entry(
        &self,
        tenant_ctx: &TenantCtx,
        vector_entry_id: Uuid,
    ) -> anyhow::Result<()>;

    /// Return observable status suitable for logs / health endpoints.
    fn safe_status(&self) -> GateServiceStatus;
}

// ---------------------------------------------------------------------------
// Shared deterministic-derivation helpers
// ---------------------------------------------------------------------------

/// Produce a stable 32-byte digest binding tenant, artifact kind, and the
/// wrapped envelope. Used by the deterministic services to derive
/// reproducible decision fields without ever invoking a real model.
fn deterministic_decision_digest(
    tenant_ctx: &TenantCtx,
    envelope_ciphertext: &[u8],
    wrapped_dek: &WrappedDek,
    object_kind: &TraceArtifactKind,
    gate_policy_version: &str,
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"trace_gate_service_decision.v1\n");
    h.update(gate_policy_version.as_bytes());
    h.update(b"\n");
    h.update(tenant_ctx.tenant_id.as_bytes());
    h.update(b"\n");
    h.update(object_kind.as_path_segment().as_bytes());
    h.update(b"\n");
    h.update(wrapped_dek.context_hash.as_bytes());
    h.update(b"\n");
    h.update(wrapped_dek.key_ref_hash.as_bytes());
    h.update(b"\n");
    h.update(wrapped_dek.wrapper_kind.as_bytes());
    h.update(b"\n");
    h.update(envelope_ciphertext);
    let out = h.finalize();
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&out);
    bytes
}

fn sha256_hex_prefixed(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("sha256:{:x}", h.finalize())
}

fn u64_from_digest_prefix(digest: &[u8; 32], offset: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&digest[offset..offset + 8]);
    u64::from_be_bytes(buf)
}

fn build_deterministic_decision(
    tenant_ctx: &TenantCtx,
    envelope_ciphertext: &[u8],
    wrapped_dek: &WrappedDek,
    object_kind: TraceArtifactKind,
    gate_policy_version: &str,
    gate_version_hash: &str,
) -> GateDecision {
    let digest = deterministic_decision_digest(
        tenant_ctx,
        envelope_ciphertext,
        wrapped_dek,
        &object_kind,
        gate_policy_version,
    );
    // Derive perplexity / novelty / tail-fraction numbers from disjoint
    // 8-byte windows of the digest, then squeeze to a stable micros range.
    let perplexity_micros = u64_from_digest_prefix(&digest, 0) % 10_000_000;
    let tail_fraction_micros = u64_from_digest_prefix(&digest, 8) % 1_000_000;
    let novelty_score_micros = u64_from_digest_prefix(&digest, 16) % 1_000_000;
    let nearest_neighbor_hash = sha256_hex_prefixed(&digest);
    let embedding_evidence_hash = sha256_hex_prefixed(
        format!(
            "trace_gate_service_embedding_evidence.v1:{}:{}",
            gate_policy_version,
            sha256_hex_prefixed(envelope_ciphertext)
        )
        .as_bytes(),
    );
    let attestation_chain_hash = sha256_hex_prefixed(
        format!(
            "trace_gate_service_attestation_chain.v1:{gate_policy_version}:{gate_version_hash}"
        )
        .as_bytes(),
    );
    GateDecision {
        gate_policy_version: gate_policy_version.to_string(),
        gate_version_hash: gate_version_hash.to_string(),
        perplexity_micros,
        tail_fraction_micros,
        perplexity_passed: true,
        novelty_score_micros,
        nearest_neighbor_hash,
        novelty_passed: true,
        embedding_evidence_hash,
        attestation_chain_hash,
    }
}

// ---------------------------------------------------------------------------
// InMemoryGateService
// ---------------------------------------------------------------------------

/// Deterministic in-process gate service for tests and local development.
///
/// Always emits a passing `GateDecision` whose numeric fields are derived
/// from a hash of the inputs, so identical inputs produce identical decisions
/// across runs.
pub struct InMemoryGateService {
    gate_policy_version: String,
    gate_version_hash: String,
}

impl InMemoryGateService {
    pub fn new(
        gate_policy_version: impl Into<String>,
        gate_version_hash: impl Into<String>,
    ) -> Self {
        Self {
            gate_policy_version: gate_policy_version.into(),
            gate_version_hash: gate_version_hash.into(),
        }
    }
}

impl TraceGateService for InMemoryGateService {
    fn evaluate_trace(
        &self,
        tenant_ctx: &TenantCtx,
        envelope_ciphertext: &[u8],
        wrapped_dek: &WrappedDek,
        object_kind: TraceArtifactKind,
    ) -> anyhow::Result<GateDecision> {
        Ok(build_deterministic_decision(
            tenant_ctx,
            envelope_ciphertext,
            wrapped_dek,
            object_kind,
            &self.gate_policy_version,
            &self.gate_version_hash,
        ))
    }

    fn invalidate_vector_entry(
        &self,
        _tenant_ctx: &TenantCtx,
        _vector_entry_id: Uuid,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn safe_status(&self) -> GateServiceStatus {
        GateServiceStatus {
            kind: "in_memory".into(),
            gate_policy_version: self.gate_policy_version.clone(),
            gate_version_hash: self.gate_version_hash.clone(),
            attestation_verifier_configured: false,
        }
    }
}

// ---------------------------------------------------------------------------
// LegacyDeterministicGateService
// ---------------------------------------------------------------------------

/// Drop-in service that preserves today's pre-enclave vector-worker behavior.
///
/// The numeric outputs are derived from the same deterministic digest the
/// in-memory service uses; the differentiator is the kind / version strings
/// reported through `safe_status` and stamped on decision rows. This is the
/// default service when `TRACE_COMMONS_GATE_SERVICE` is unset so existing
/// deployments see no behavior change after this migration.
pub struct LegacyDeterministicGateService;

impl LegacyDeterministicGateService {
    pub const GATE_POLICY_VERSION: &'static str = "legacy_deterministic";
    pub const GATE_VERSION_HASH: &'static str = "sha256:legacy_deterministic";

    pub fn new() -> Self {
        Self
    }
}

impl Default for LegacyDeterministicGateService {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceGateService for LegacyDeterministicGateService {
    fn evaluate_trace(
        &self,
        tenant_ctx: &TenantCtx,
        envelope_ciphertext: &[u8],
        wrapped_dek: &WrappedDek,
        object_kind: TraceArtifactKind,
    ) -> anyhow::Result<GateDecision> {
        Ok(build_deterministic_decision(
            tenant_ctx,
            envelope_ciphertext,
            wrapped_dek,
            object_kind,
            Self::GATE_POLICY_VERSION,
            Self::GATE_VERSION_HASH,
        ))
    }

    fn invalidate_vector_entry(
        &self,
        _tenant_ctx: &TenantCtx,
        _vector_entry_id: Uuid,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn safe_status(&self) -> GateServiceStatus {
        GateServiceStatus {
            kind: "legacy_deterministic".into(),
            gate_policy_version: Self::GATE_POLICY_VERSION.into(),
            gate_version_hash: Self::GATE_VERSION_HASH.into(),
            attestation_verifier_configured: false,
        }
    }
}

// ---------------------------------------------------------------------------
// DstackGateService (stub)
// ---------------------------------------------------------------------------

/// Fail-closed stub for the dstack-resident gate service.
///
/// Construction takes the operator-configured enclave endpoint URL and
/// attestation-verifier label. Neither field is validated here — the operator
/// is responsible for supplying live values once the enclave binary lands.
/// Today every `evaluate_trace` / `invalidate_vector_entry` call returns a
/// stable `DstackGateServiceUnavailable` error so a deployment that toggles
/// the gate service to `dstack` before the enclave is wired up fails closed
/// with a clear, hash-only signal.
pub struct DstackGateService {
    #[allow(dead_code)]
    enclave_endpoint: String,
    #[allow(dead_code)]
    attestation_verifier_label: String,
}

impl DstackGateService {
    pub fn new(
        enclave_endpoint: impl Into<String>,
        attestation_verifier_label: impl Into<String>,
    ) -> Self {
        Self {
            enclave_endpoint: enclave_endpoint.into(),
            attestation_verifier_label: attestation_verifier_label.into(),
        }
    }
}

impl TraceGateService for DstackGateService {
    fn evaluate_trace(
        &self,
        _tenant_ctx: &TenantCtx,
        _envelope_ciphertext: &[u8],
        _wrapped_dek: &WrappedDek,
        _object_kind: TraceArtifactKind,
    ) -> anyhow::Result<GateDecision> {
        anyhow::bail!("DstackGateServiceUnavailable: dstack gate service not yet wired")
    }

    fn invalidate_vector_entry(
        &self,
        _tenant_ctx: &TenantCtx,
        _vector_entry_id: Uuid,
    ) -> anyhow::Result<()> {
        anyhow::bail!("DstackGateServiceUnavailable: dstack gate service not yet wired")
    }

    fn safe_status(&self) -> GateServiceStatus {
        GateServiceStatus {
            kind: "dstack_stub".into(),
            gate_policy_version: "stub".into(),
            gate_version_hash: "sha256:stub".into(),
            attestation_verifier_configured: true,
        }
    }
}

// ---------------------------------------------------------------------------
// EnclaveGateService
// ---------------------------------------------------------------------------

/// Gate service that wraps an `EnclaveGateOrchestrator` from
/// `tracedao-gate-enclave`. The orchestrator carries the
/// perplexity/embedder/vector-index pipeline; this adapter is responsible for
/// unwrapping the per-envelope DEK and decrypting the ciphertext before
/// handing the plaintext off to the orchestrator. The unwrapped DEK is held
/// in a `Zeroizing` buffer through `unwrap_dek`'s return type so plaintext key
/// material is scrubbed on drop.
///
/// Production deployments will wrap a real perplexity scorer + embedder +
/// ANN index here; today the public `mock_with_local_kek` constructor stands
/// up a fully mocked pipeline so callers can exercise the trait shape without
/// any hardware in the loop.
pub struct EnclaveGateService<P, E, V> {
    orchestrator: EnclaveGateOrchestrator<P, E, V>,
    decryptor: Arc<dyn KmsKeyWrapper>,
    safe_kind: String,
}

impl<P, E, V> EnclaveGateService<P, E, V>
where
    P: PerplexityScorer,
    E: Embedder,
    V: VectorIndex,
{
    pub fn new(
        orchestrator: EnclaveGateOrchestrator<P, E, V>,
        decryptor: Arc<dyn KmsKeyWrapper>,
        safe_kind: impl Into<String>,
    ) -> Self {
        Self {
            orchestrator,
            decryptor,
            safe_kind: safe_kind.into(),
        }
    }
}

impl EnclaveGateService<MockPerplexityScorer, MockEmbedder, MockVectorIndex> {
    /// Convenience constructor for tests / dev: build an `EnclaveGateService`
    /// composed of the mock perplexity scorer, mock embedder, and in-memory
    /// vector index, paired with the caller-supplied `KmsKeyWrapper`.
    pub fn mock_with_decryptor(decryptor: Arc<dyn KmsKeyWrapper>) -> Self {
        let cfg = EnclaveGateOrchestratorConfig::mock_default();
        let orchestrator = EnclaveGateOrchestrator::new(
            MockPerplexityScorer::new(),
            MockEmbedder::new(),
            MockVectorIndex::new(),
            cfg,
        );
        Self::new(orchestrator, decryptor, "enclave_mock")
    }
}

impl<P, E, V> TraceGateService for EnclaveGateService<P, E, V>
where
    P: PerplexityScorer + Send + Sync,
    E: Embedder + Send + Sync,
    V: VectorIndex + Send + Sync,
{
    fn evaluate_trace(
        &self,
        tenant_ctx: &TenantCtx,
        envelope_ciphertext: &[u8],
        wrapped_dek: &WrappedDek,
        object_kind: TraceArtifactKind,
    ) -> anyhow::Result<GateDecision> {
        // The envelope ciphertext on disk is base64-encoded; the gate-worker
        // route is expected to pass it through as raw bytes (already decoded).
        // We accept either: if the bytes look like base64 (ASCII), try to
        // decode first and fall back to raw on failure. Production callers
        // should pass raw bytes.
        let ciphertext = decode_envelope_ciphertext(envelope_ciphertext);

        let ctx = KekContext {
            tenant_storage_ref: tenant_ctx.tenant_id.clone(),
            artifact_kind: object_kind.clone(),
        };
        let dek = self.decryptor.unwrap_dek(wrapped_dek, &ctx)?;
        let plaintext = aead_decrypt_with_dek(&dek, &ciphertext)?;

        let decision = self
            .orchestrator
            .evaluate(&plaintext, &tenant_ctx.tenant_id)?;
        Ok(GateDecision {
            gate_policy_version: decision.gate_policy_version,
            gate_version_hash: decision.gate_version_hash,
            perplexity_micros: decision.perplexity_micros,
            tail_fraction_micros: decision.tail_fraction_micros,
            perplexity_passed: decision.perplexity_passed,
            novelty_score_micros: decision.novelty_score_micros,
            nearest_neighbor_hash: decision.nearest_neighbor_hash,
            novelty_passed: decision.novelty_passed,
            embedding_evidence_hash: decision.embedding_evidence_hash,
            attestation_chain_hash: decision.attestation_chain_hash,
        })
    }

    fn invalidate_vector_entry(
        &self,
        _tenant_ctx: &TenantCtx,
        vector_entry_id: Uuid,
    ) -> anyhow::Result<()> {
        // Best-effort propagation into the orchestrator's vector index. The
        // orchestrator field is private to this module; expose deletion
        // through a thin accessor by adding a helper on the orchestrator.
        // For now we treat this as a no-op when the index doesn't know the
        // id — matching `MockVectorIndex::delete` semantics.
        let _ = vector_entry_id;
        // The orchestrator owns the index; we don't have a direct handle
        // from the service layer. The orchestrator does not currently expose
        // index access, so we surface a clean no-op here. The dstack
        // implementation will route this call into the enclave's index.
        Ok(())
    }

    fn safe_status(&self) -> GateServiceStatus {
        let cfg = self.orchestrator.config();
        GateServiceStatus {
            kind: self.safe_kind.clone(),
            gate_policy_version: cfg.gate_policy_version.clone(),
            gate_version_hash: cfg.gate_version_hash.clone(),
            attestation_verifier_configured: false,
        }
    }
}

/// If the bytes are valid base64, decode; otherwise treat them as raw.
///
/// The artifact store persists ciphertext as base64, and the gate-worker route
/// loads the artifact through `decrypt_artifact_json_with_kek` indirectly — by
/// the time the gate worker hands us bytes, they may be either form depending
/// on the call path. Accepting both keeps the trait flexible without forcing
/// the worker to commit to a single representation.
fn decode_envelope_ciphertext(bytes: &[u8]) -> Vec<u8> {
    if bytes.is_empty() {
        return Vec::new();
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(s.trim()) {
            return decoded;
        }
    }
    bytes.to_vec()
}

// ---------------------------------------------------------------------------
// EnclaveGateService inline tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod enclave_gate_service_tests {
    use super::*;

    use secrecy::SecretString;
    use zeroize::Zeroizing;

    use crate::secrets::SecretsCrypto;
    use crate::trace_artifact_kek::{KekContext, LocalMasterKeyWrapper};
    use crate::trace_artifact_store::aead_encrypt_with_dek;

    fn fixture_decryptor() -> Arc<dyn KmsKeyWrapper> {
        let crypto = SecretsCrypto::new(SecretString::new("a".repeat(32).into()))
            .expect("fixture SecretsCrypto");
        Arc::new(LocalMasterKeyWrapper::new(
            crypto,
            "enclave-gate-fixture",
        ))
    }

    fn wrap_fixture_dek(
        decryptor: &dyn KmsKeyWrapper,
        tenant_storage_ref: &str,
    ) -> (Zeroizing<[u8; 32]>, WrappedDek) {
        let dek = Zeroizing::new([7u8; 32]);
        let ctx = KekContext {
            tenant_storage_ref: tenant_storage_ref.into(),
            artifact_kind: TraceArtifactKind::ContributionEnvelope,
        };
        let wrapped = decryptor
            .wrap_dek(&dek, &ctx)
            .expect("LocalMasterKeyWrapper should wrap test DEK");
        (dek, wrapped)
    }

    #[test]
    fn enclave_gate_service_evaluates_passing_trace() {
        let decryptor = fixture_decryptor();
        let svc = EnclaveGateService::mock_with_decryptor(Arc::clone(&decryptor));

        let tenant = TenantCtx::new("tenant-a");
        let (dek, wrapped) = wrap_fixture_dek(decryptor.as_ref(), &tenant.tenant_id);
        let ciphertext =
            aead_encrypt_with_dek(&dek, b"a fresh trace plaintext").expect("encrypt fixture");

        let decision = svc
            .evaluate_trace(
                &tenant,
                &ciphertext,
                &wrapped,
                TraceArtifactKind::ContributionEnvelope,
            )
            .expect("evaluate_trace should succeed");
        assert_eq!(decision.gate_policy_version, "enclave_mock_v1");
        assert!(decision.perplexity_passed);
        assert!(decision.novelty_passed);
        assert!(decision.nearest_neighbor_hash.starts_with("sha256:"));
    }

    #[test]
    fn enclave_gate_service_rejects_bad_dek_context() {
        let decryptor = fixture_decryptor();
        let svc = EnclaveGateService::mock_with_decryptor(Arc::clone(&decryptor));

        let tenant = TenantCtx::new("tenant-a");
        let (dek, mut wrapped) = wrap_fixture_dek(decryptor.as_ref(), &tenant.tenant_id);
        let ciphertext = aead_encrypt_with_dek(&dek, b"a fresh trace plaintext").unwrap();

        // Tamper with the wrapped DEK's context_hash so the unwrap fails.
        wrapped.context_hash = "sha256:tampered".into();

        let err = svc
            .evaluate_trace(
                &tenant,
                &ciphertext,
                &wrapped,
                TraceArtifactKind::ContributionEnvelope,
            )
            .expect_err("tampered context_hash must fail");
        assert!(
            format!("{err}").contains("KekContextMismatch"),
            "expected KekContextMismatch, got: {err}"
        );
    }

    #[test]
    fn enclave_gate_service_safe_status_reports_mock_kind() {
        let decryptor = fixture_decryptor();
        let svc = EnclaveGateService::mock_with_decryptor(decryptor);
        let status = svc.safe_status();
        assert_eq!(status.kind, "enclave_mock");
        assert_eq!(status.gate_policy_version, "enclave_mock_v1");
        assert!(status.gate_version_hash.starts_with("sha256:"));
        assert!(!status.attestation_verifier_configured);
    }

    #[test]
    fn enclave_gate_service_invalidate_returns_ok() {
        let decryptor = fixture_decryptor();
        let svc = EnclaveGateService::mock_with_decryptor(decryptor);
        svc.invalidate_vector_entry(&TenantCtx::new("tenant-a"), Uuid::new_v4())
            .expect("invalidate should succeed");
    }
}
