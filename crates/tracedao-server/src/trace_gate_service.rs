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

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::trace_artifact_kek::WrappedDek;
use crate::trace_artifact_store::TraceArtifactKind;

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
