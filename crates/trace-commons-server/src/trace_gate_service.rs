// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

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

use trace_commons_gate_enclave::{
    Embedder, EnclaveGateOrchestrator, EnclaveGateOrchestratorConfig, MockEmbedder,
    MockPerplexityScorer, MockVectorIndex, PerplexityScorer, VectorIndex,
};

use crate::trace_artifact_kek::{KekContext, KmsKeyWrapper, WrappedDek};
use crate::trace_artifact_store::{TraceArtifactKind, aead_decrypt_with_dek};

/// Minimal tenant context plumbed into the gate service. We intentionally
/// keep this struct independent of the binary-private `TenantAuth` type so
/// the trait stays defined in the library crate.
///
/// The `tenant_storage_ref` is the canonical `"tenant_sha256:..."` form used
/// across the artifact store, KEK context binding, and vector-index sharding.
/// Constructing this struct from anything but the canonical form would make
/// the gate worker's KEK context disagree with the wrapped DEK that was
/// produced under the canonical ref → `KekContextMismatch` at first read.
#[derive(Debug, Clone)]
pub struct TenantCtx {
    tenant_storage_ref: String,
}

impl TenantCtx {
    /// Construct from a value that is already the canonical
    /// `tenant_sha256:...` storage ref. Callers MUST NOT pass a raw tenant id
    /// here; use the binary's `tenant_storage_ref(&tenant.tenant_id)` helper to
    /// canonicalize first.
    pub fn from_canonical(tenant_storage_ref: impl Into<String>) -> Self {
        Self {
            tenant_storage_ref: tenant_storage_ref.into(),
        }
    }

    /// Test/dev constructor. Treats the argument as already-canonical and
    /// records it verbatim. Production code MUST go through `from_canonical`
    /// with the output of `tenant_storage_ref(&tenant.tenant_id)`.
    pub fn new(tenant_storage_ref: impl Into<String>) -> Self {
        Self::from_canonical(tenant_storage_ref)
    }

    /// The canonical storage ref this context represents. Use this for KEK
    /// context binding, vector-index sharding, and any other store/index keying.
    pub fn tenant_storage_ref(&self) -> &str {
        &self.tenant_storage_ref
    }
}

/// One inserted per-chunk vector-index entry, host-facing form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateChunkVectorEntry {
    pub chunk_index: u32,
    pub vector_entry_id: Uuid,
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
    /// `Some(id)` when both gates passed and the service inserted the
    /// embedding into the vector index; `None` otherwise. The operator needs
    /// this id to call `invalidate_vector_entry` later (e.g. from the
    /// revocation worker). It is stored in `trace_gate_decisions` via a
    /// nullable `vector_entry_id` column (migration V24).
    pub vector_entry_id: Option<Uuid>,
    /// Peak (most-surprising min-content-guarded chunk) perplexity.
    pub peak_perplexity_micros: u64,
    /// Peak per-chunk novelty.
    pub peak_novelty_micros: u64,
    /// Number of chunks scored (>= 1; deterministic services report 1).
    pub chunk_count: u32,
    /// Total chunks before the per-trace cap dropped any (>= `chunk_count`;
    /// deterministic services report 1). The denominator behind a capped
    /// decision's coverage.
    pub total_chunk_count: u32,
    /// True when the per-trace chunk cap dropped trailing chunks.
    pub chunks_capped: bool,
    /// Every per-chunk vector-index entry the gate inserted. Empty for
    /// deterministic/legacy services and failed gates. The host persists
    /// these as (submission_id, chunk_index)-tagged rows for revocation.
    pub chunk_vector_entries: Vec<GateChunkVectorEntry>,
    /// 64-bit token simhash of the trace's decrypted text, for cross-trace
    /// dedup clustering; computed inside the service so plaintext never
    /// crosses the boundary (only the hash does), like `nearest_neighbor_hash`.
    pub dedup_simhash: i64,
    /// 64-bit token simhash of `outcome.human_correction`, when the envelope
    /// carries one. `None` means "this service did not observe a correction" —
    /// either the envelope has none, or the service never sees plaintext (the
    /// deterministic services). Same trust-boundary rule as `dedup_simhash`:
    /// computed inside the service, only the hash crosses back.
    ///
    /// Feeds the SHADOW-ONLY correction value (`crate::correction_value`).
    /// Nothing downstream of it gates, settles, or pays.
    pub correction_simhash: Option<i64>,
}

/// Token simhash of `outcome.human_correction` in a decrypted envelope
/// plaintext, or `None` when the envelope carries no correction (absent,
/// null, or whitespace-only).
///
/// Deliberately over the correction text ALONE, not the canonical correction
/// representation: the value being scored is what the contributor wrote, and
/// folding in surrounding metadata would let an unchanged correction score as
/// novel because the trace around it differed.
///
/// A malformed or non-JSON plaintext yields `None` rather than an error — the
/// correction value is shadow-only, so a parse failure must degrade to "no
/// correction observed" and never block a gate decision.
pub fn correction_simhash_from_plaintext(plaintext: &[u8]) -> Option<i64> {
    let value: serde_json::Value = serde_json::from_slice(plaintext).ok()?;
    let correction = value
        .get("outcome")?
        .get("human_correction")?
        .as_str()?
        .trim();
    if correction.is_empty() {
        return None;
    }
    Some(crate::dedup_simhash::trace_simhash(correction) as i64)
}

/// Observable status of a `TraceGateService`, safe for logs / health surfaces.
#[derive(Debug, Clone)]
pub struct GateServiceStatus {
    pub kind: String,
    pub gate_policy_version: String,
    pub gate_version_hash: String,
    pub attestation_verifier_configured: bool,
}

/// Perplexity-only outcome returned by
/// [`TraceGateService::evaluate_trace_perplexity_only`]. Carries only the
/// perplexity-derived fields the re-score maintenance path updates; there is
/// deliberately no novelty, embedding, or vector-entry state because the
/// perplexity-only path never touches the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerplexityOnlyGateOutcome {
    /// Representative (token-weighted) perplexity in micros.
    pub perplexity_micros: u64,
    /// Peak (most-surprising chunk) perplexity in micros.
    pub peak_perplexity_micros: u64,
    /// Whether the perplexity cleared the configured floor(s) — the same
    /// predicate a full evaluation applies.
    pub perplexity_passed: bool,
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

    /// Recompute ONLY the perplexity of a wrapped trace, running the exact same
    /// canonical-representation + chunking + scoring path as
    /// [`Self::evaluate_trace`] but performing NO embedding, NO nearest-neighbor
    /// query, and NO vector-index insertion. Used by the perplexity re-score
    /// maintenance task so re-scored values are byte-comparable to production
    /// perplexity without mutating the novelty/vector index.
    ///
    /// The default implementation refuses — only backends that carry a real
    /// perplexity scorer (`EnclaveGateService`) and the deterministic
    /// in-memory service override it. A backend that cannot isolate perplexity
    /// from its novelty/vector side effects MUST leave this defaulted so the
    /// re-score task fails closed rather than corrupting the index.
    fn evaluate_trace_perplexity_only(
        &self,
        _tenant_ctx: &TenantCtx,
        _envelope_ciphertext: &[u8],
        _wrapped_dek: &WrappedDek,
        _object_kind: TraceArtifactKind,
    ) -> anyhow::Result<PerplexityOnlyGateOutcome> {
        anyhow::bail!("PerplexityOnlyRescoreUnsupported")
    }

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

    /// Make the novelty corpus durable before the process exits.
    ///
    /// Backends whose index is purely in-memory (the deterministic and mock
    /// services) keep the no-op default. `EnclaveGateService` overrides it so a
    /// graceful shutdown persists whatever the periodic flush has not written
    /// yet — without this, a restart silently changes what "duplicate" means
    /// for every trace scored after it.
    fn flush_vector_index(&self) -> anyhow::Result<()> {
        Ok(())
    }
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
    h.update(tenant_ctx.tenant_storage_ref().as_bytes());
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
    // Deterministic services (`InMemoryGateService`, `LegacyDeterministicGateService`)
    // never see plaintext — only the AEAD ciphertext, which is nonce-randomized
    // per encryption, so a real `dedup_simhash::trace_simhash` over "the text"
    // is not available here. Fall back to an unused 8-byte window of the same
    // input digest every other deterministic field is derived from: this keeps
    // `dedup_simhash` stable for byte-identical `evaluate_trace` calls (the
    // deterministic-service contract callers already rely on) without
    // pretending to detect duplicate PLAINTEXT under independent encryptions.
    // Real cross-trace duplicate detection requires `EnclaveGateService`,
    // which has the decrypted plaintext in scope.
    let dedup_simhash = u64_from_digest_prefix(&digest, 24) as i64;
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
        // Deterministic services do not actually insert into a vector index,
        // so there is no entry id to surface.
        vector_entry_id: None,
        // Deterministic services score the whole trace as a single chunk;
        // peak == representative.
        peak_perplexity_micros: perplexity_micros,
        peak_novelty_micros: novelty_score_micros,
        chunk_count: 1,
        total_chunk_count: 1,
        chunks_capped: false,
        chunk_vector_entries: Vec::new(),
        dedup_simhash,
        // Deterministic services never see plaintext (see `dedup_simhash`
        // above), so they cannot know whether the envelope carries a
        // correction. `None` says exactly that; a digest-derived stand-in
        // would fabricate a correction signal for every trace and seed the
        // shadow corpus with phantom corrections.
        correction_simhash: None,
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

    fn evaluate_trace_perplexity_only(
        &self,
        tenant_ctx: &TenantCtx,
        envelope_ciphertext: &[u8],
        wrapped_dek: &WrappedDek,
        object_kind: TraceArtifactKind,
    ) -> anyhow::Result<PerplexityOnlyGateOutcome> {
        // Derive the perplexity fields from the same deterministic decision the
        // full path would produce, so re-scored values stay consistent with a
        // full evaluation. No novelty/vector state is read or written.
        let decision = build_deterministic_decision(
            tenant_ctx,
            envelope_ciphertext,
            wrapped_dek,
            object_kind,
            &self.gate_policy_version,
            &self.gate_version_hash,
        );
        Ok(PerplexityOnlyGateOutcome {
            perplexity_micros: decision.perplexity_micros,
            peak_perplexity_micros: decision.peak_perplexity_micros,
            perplexity_passed: decision.perplexity_passed,
        })
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
/// `trace-commons-gate-enclave`. The orchestrator carries the
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

        // Critical: use the canonical tenant_storage_ref for BOTH the KEK
        // context binding and the orchestrator's per-tenant index sharding.
        // The wrapped DEK was produced under the canonical ref by the
        // artifact-store path; constructing the KekContext with anything else
        // (e.g. raw tenant id) makes the unwrap fail with KekContextMismatch.
        let ctx = KekContext {
            tenant_storage_ref: tenant_ctx.tenant_storage_ref().to_string(),
            artifact_kind: object_kind.clone(),
        };
        let dek = self.decryptor.unwrap_dek(wrapped_dek, &ctx)?;
        let plaintext = aead_decrypt_with_dek(&dek, &ciphertext)?;

        let decision = self
            .orchestrator
            .evaluate(&plaintext, tenant_ctx.tenant_storage_ref())?;
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
            vector_entry_id: decision.inserted_entry_id,
            peak_perplexity_micros: decision.peak_perplexity_micros,
            peak_novelty_micros: decision.peak_novelty_micros,
            chunk_count: decision.chunk_count,
            total_chunk_count: decision.total_chunk_count,
            chunks_capped: decision.chunks_capped,
            chunk_vector_entries: decision
                .inserted_chunk_entries
                .iter()
                .map(|e| GateChunkVectorEntry {
                    chunk_index: e.chunk_index,
                    vector_entry_id: e.entry_id,
                })
                .collect(),
            // Computed from the AEAD-decrypted plaintext (in scope above as
            // `plaintext`) so the cross-trace dedup signal is derived inside
            // the same trust boundary as every other decrypted-content field
            // here — only the hash crosses back to the caller.
            //
            // Must be over the CANONICAL RENDERED EVENT TEXT
            // (metadata-free), not the raw envelope JSON: the envelope
            // carries per-submission-unique fields (submission_id, trace_id,
            // created_at, per-event event_id/timestamp), so hashing the raw
            // JSON means byte-identical-content resubmissions never collide.
            // This mirrors the same rendering the chunker uses to build the
            // text the scorer/embedder actually consume
            // (`chunk_envelope_plaintext` / `chunk_plaintext`), so the
            // simhash is over the same metadata-free text.
            dedup_simhash: {
                let dedup_canonical_text =
                    trace_commons_gate_enclave::chunker::parse_envelope_rendered_events(&plaintext)
                        .map(|events| events.join("\n"))
                        .unwrap_or_else(|| String::from_utf8_lossy(&plaintext).into_owned());
                crate::dedup_simhash::trace_simhash(&dedup_canonical_text) as i64
            },
            // Same trust boundary and the same `plaintext` in scope: the
            // correction's simhash is computed here so the correction text
            // itself never crosses back to the caller.
            correction_simhash: correction_simhash_from_plaintext(&plaintext),
        })
    }

    fn evaluate_trace_perplexity_only(
        &self,
        tenant_ctx: &TenantCtx,
        envelope_ciphertext: &[u8],
        wrapped_dek: &WrappedDek,
        object_kind: TraceArtifactKind,
    ) -> anyhow::Result<PerplexityOnlyGateOutcome> {
        // Identical decrypt path to `evaluate_trace` — same DEK unwrap under the
        // canonical KEK context and the same AEAD decrypt — so the plaintext fed
        // to the scorer is byte-identical to a full evaluation.
        let ciphertext = decode_envelope_ciphertext(envelope_ciphertext);
        let ctx = KekContext {
            tenant_storage_ref: tenant_ctx.tenant_storage_ref().to_string(),
            artifact_kind: object_kind.clone(),
        };
        let dek = self.decryptor.unwrap_dek(wrapped_dek, &ctx)?;
        let plaintext = aead_decrypt_with_dek(&dek, &ciphertext)?;

        // Perplexity-only: chunk + score + aggregate with the SAME orchestrator
        // config, but no embedding, no nearest-neighbor query, no index insert.
        let outcome = self.orchestrator.evaluate_perplexity_only(&plaintext)?;
        Ok(PerplexityOnlyGateOutcome {
            perplexity_micros: outcome.perplexity_micros,
            peak_perplexity_micros: outcome.peak_perplexity_micros,
            perplexity_passed: outcome.perplexity_passed,
        })
    }

    fn invalidate_vector_entry(
        &self,
        tenant_ctx: &TenantCtx,
        vector_entry_id: Uuid,
    ) -> anyhow::Result<()> {
        // Route deletion through the orchestrator into the underlying index.
        // The tenant_storage_ref is required so per-tenant implementations
        // (e.g. UsearchVectorIndex) can route the deletion to the right shard
        // without doing a global scan. `delete` returns Ok(true) for a hit and
        // Ok(false) for a miss; both satisfy the "make sure it's gone"
        // postcondition, so we discard the bool.
        let _ = self
            .orchestrator
            .delete_vector_entry(tenant_ctx.tenant_storage_ref(), vector_entry_id)?;
        Ok(())
    }

    fn flush_vector_index(&self) -> anyhow::Result<()> {
        self.orchestrator.flush_vector_index()
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
        Arc::new(LocalMasterKeyWrapper::new(crypto, "enclave-gate-fixture"))
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
        let (dek, wrapped) = wrap_fixture_dek(decryptor.as_ref(), tenant.tenant_storage_ref());
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
        let (dek, mut wrapped) = wrap_fixture_dek(decryptor.as_ref(), tenant.tenant_storage_ref());
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

    #[test]
    fn enclave_gate_service_invalidate_unknown_entry_is_idempotent() {
        // Calling invalidate with a UUID that was never inserted must succeed
        // (Ok(false) from the index is a satisfied postcondition: "it's gone").
        let decryptor = fixture_decryptor();
        let svc = EnclaveGateService::mock_with_decryptor(decryptor);
        let random_id = Uuid::new_v4();
        svc.invalidate_vector_entry(&TenantCtx::new("tenant-a"), random_id)
            .expect("invalidate of unknown entry must succeed (idempotent)");
    }

    #[test]
    fn enclave_gate_service_delete_after_insert_restores_novelty() {
        // 1. Evaluate a fresh plaintext — both gates pass, entry is inserted.
        // 2. Call invalidate_vector_entry with the returned entry id.
        // 3. Evaluate the SAME plaintext again — the index is empty again so
        //    novelty score should be at maximum (the prior entry was deleted).
        //
        // Note: we do NOT do an intermediate evaluation between steps 1 and 2
        // because with novelty_floor = 0 every evaluation passes and inserts
        // its own entry. We insert exactly once, delete exactly that entry,
        // then verify the index is empty again.
        let decryptor = fixture_decryptor();
        let svc = EnclaveGateService::mock_with_decryptor(Arc::clone(&decryptor));

        let tenant = TenantCtx::new("tenant-del-test");
        let (dek, wrapped) = wrap_fixture_dek(decryptor.as_ref(), tenant.tenant_storage_ref());
        let ciphertext = aead_encrypt_with_dek(&dek, b"delete-restore-novelty-plaintext")
            .expect("encrypt fixture");

        // First evaluation — entry should be inserted and entry_id surfaced.
        let first = svc
            .evaluate_trace(
                &tenant,
                &ciphertext,
                &wrapped,
                TraceArtifactKind::ContributionEnvelope,
            )
            .expect("first evaluate_trace must succeed");
        assert!(
            first.perplexity_passed && first.novelty_passed,
            "first evaluation must pass both gates"
        );
        let entry_id = first
            .vector_entry_id
            .expect("first evaluation must surface the inserted entry_id");

        // Delete the entry before any further evaluations can add more entries.
        svc.invalidate_vector_entry(&tenant, entry_id)
            .expect("invalidate_vector_entry must succeed");

        // Second evaluation after deletion — index is empty again for this
        // tenant, so novelty is restored to maximum (1_000_000 micros).
        let second = svc
            .evaluate_trace(
                &tenant,
                &ciphertext,
                &wrapped,
                TraceArtifactKind::ContributionEnvelope,
            )
            .expect("second evaluate_trace after deletion must succeed");
        assert!(
            second.novelty_score_micros >= 900_000,
            "novelty must be high again after deletion, got {}",
            second.novelty_score_micros
        );
    }

    /// The shutdown path calls `TraceGateService::flush_vector_index`; it has
    /// to reach the index the orchestrator actually holds, or a graceful
    /// restart quietly loses the novelty corpus.
    #[test]
    fn enclave_gate_service_flush_reaches_the_vector_index() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use trace_commons_gate_enclave::{EnclaveGateOrchestrator, EnclaveGateOrchestratorConfig};

        #[derive(Default)]
        struct CountingIndex {
            flushes: Arc<AtomicUsize>,
        }
        impl VectorIndex for CountingIndex {
            fn insert(&self, _: Uuid, _: &str, _: &[f32]) -> anyhow::Result<()> {
                Ok(())
            }
            fn nearest(
                &self,
                _: &str,
                _: &[f32],
                _: usize,
            ) -> anyhow::Result<Vec<trace_commons_gate_enclave::NearestNeighbor>> {
                Ok(Vec::new())
            }
            fn delete(&self, _: &str, _: Uuid) -> anyhow::Result<bool> {
                Ok(false)
            }
            fn flush(&self) -> anyhow::Result<()> {
                self.flushes.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }

        let flushes = Arc::new(AtomicUsize::new(0));
        let orchestrator = EnclaveGateOrchestrator::new(
            MockPerplexityScorer::new(),
            trace_commons_gate_enclave::MockEmbedder::new(),
            CountingIndex {
                flushes: Arc::clone(&flushes),
            },
            EnclaveGateOrchestratorConfig::mock_default(),
        );
        let svc = EnclaveGateService::new(orchestrator, fixture_decryptor(), "enclave_test_flush");

        assert_eq!(flushes.load(Ordering::SeqCst), 0);
        svc.flush_vector_index().expect("flush");
        assert_eq!(flushes.load(Ordering::SeqCst), 1);
    }

    /// Phase A audit fix: an embedder inference failure MUST propagate as an
    /// `evaluate_trace` error so the gate worker fails closed. The previous
    /// fail-open shape returned a zero vector → novelty math would interpret
    /// that as "maximally novel" → gate trivially passes despite the failure.
    #[test]
    fn enclave_gate_service_embedder_error_propagates_as_evaluate_failure() {
        use trace_commons_gate_enclave::{
            EnclaveGateOrchestrator, EnclaveGateOrchestratorConfig, MockVectorIndex,
        };

        /// Always-failing embedder: every call returns `Err`.
        struct FailingEmbedder;
        impl trace_commons_gate_enclave::Embedder for FailingEmbedder {
            fn embed(&self, _plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
                anyhow::bail!("EmbedderInferenceFailed: synthetic test failure")
            }
        }

        let decryptor = fixture_decryptor();
        let cfg = EnclaveGateOrchestratorConfig::mock_default();
        let orchestrator = EnclaveGateOrchestrator::new(
            MockPerplexityScorer::new(),
            FailingEmbedder,
            MockVectorIndex::new(),
            cfg,
        );
        let svc = EnclaveGateService::new(
            orchestrator,
            Arc::clone(&decryptor),
            "enclave_test_failing_embedder",
        );

        let tenant = TenantCtx::new("tenant-failing-embedder");
        let (dek, wrapped) = wrap_fixture_dek(decryptor.as_ref(), tenant.tenant_storage_ref());
        let ciphertext =
            aead_encrypt_with_dek(&dek, b"fail-closed-fixture-plaintext").expect("encrypt");

        let err = svc
            .evaluate_trace(
                &tenant,
                &ciphertext,
                &wrapped,
                TraceArtifactKind::ContributionEnvelope,
            )
            .expect_err("embedder error must propagate as evaluate_trace failure");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("EmbedderInferenceFailed"),
            "expected EmbedderInferenceFailed error class, got: {msg}"
        );
    }

    /// Phase A audit fix: TenantCtx now carries the canonical
    /// `tenant_sha256:...` form. A wrapped DEK produced under canonical-X must
    /// unwrap under canonical-X (success), and must FAIL when the gate
    /// service is handed a non-canonical ref — this is the regression that
    /// caused KekContextMismatch in real deployments when the worker passed
    /// raw `tenant.tenant_id`.
    #[test]
    fn enclave_gate_service_uses_canonical_tenant_ref() {
        let decryptor = fixture_decryptor();
        let svc = EnclaveGateService::mock_with_decryptor(Arc::clone(&decryptor));

        // Wrap a DEK under the canonical-form ref the artifact store uses.
        let canonical = "tenant_sha256:abcd1234";
        let (dek, wrapped) = wrap_fixture_dek(decryptor.as_ref(), canonical);
        let ciphertext = aead_encrypt_with_dek(&dek, b"canonical-fixture").expect("encrypt");

        // Canonical-ref ctx: evaluate succeeds (KEK context matches).
        let canonical_ctx = TenantCtx::from_canonical(canonical);
        svc.evaluate_trace(
            &canonical_ctx,
            &ciphertext,
            &wrapped,
            TraceArtifactKind::ContributionEnvelope,
        )
        .expect("canonical TenantCtx must unwrap the DEK and evaluate");

        // Non-canonical ref (raw tenant id form): KekContext disagrees with
        // the wrapped DEK's context binding → `KekContextMismatch` is the
        // exact regression the canonical-threading fix prevents in
        // production.
        let raw_ctx = TenantCtx::from_canonical("raw-tenant-id");
        let err = svc
            .evaluate_trace(
                &raw_ctx,
                &ciphertext,
                &wrapped,
                TraceArtifactKind::ContributionEnvelope,
            )
            .expect_err("non-canonical ref must fail with KekContextMismatch");
        assert!(
            format!("{err:#}").contains("KekContextMismatch"),
            "expected KekContextMismatch, got: {err}"
        );
    }

    /// End-to-end: a large multi-chunk trace through the mock enclave
    /// pipeline exercises cap enforcement, per-chunk dedup insert, and
    /// representative-vs-peak recording all at once.
    #[test]
    fn multi_chunk_trace_records_representative_and_peak_end_to_end() {
        // Build a multi-chunk envelope: 20 events x 8000 chars -> 20 target
        // chunks -> capped at 16.
        let pad = "a".repeat(8_000);
        let events: Vec<serde_json::Value> = (0..20)
            .map(|i| {
                serde_json::json!({
                    "event_type": "assistant_message",
                    "redacted_content": format!("{i}:{pad}"),
                })
            })
            .collect();
        let plaintext = serde_json::to_vec(&serde_json::json!({ "events": events })).unwrap();

        let decryptor = fixture_decryptor();
        let svc = EnclaveGateService::mock_with_decryptor(Arc::clone(&decryptor));
        let tenant = TenantCtx::new("tenant-e2e");

        // Seed the tenant's index with a standalone trace whose sole chunk
        // is byte-identical to the main trace's chunk 0 (same rendered
        // event text). This makes chunk 0 a genuine near-duplicate *within*
        // the scored call below, while chunks 1..15 stay fresh — otherwise
        // every chunk in a single `evaluate()` call is scored against the
        // same pre-call index snapshot and peak == representative
        // trivially, which would not catch a peak-novelty regression.
        let seed_plaintext = serde_json::to_vec(&serde_json::json!({
            "events": [events[0].clone()],
        }))
        .unwrap();
        let (seed_dek, seed_wrapped) =
            wrap_fixture_dek(decryptor.as_ref(), tenant.tenant_storage_ref());
        let seed_ciphertext =
            aead_encrypt_with_dek(&seed_dek, &seed_plaintext).expect("encrypt seed fixture");
        svc.evaluate_trace(
            &tenant,
            &seed_ciphertext,
            &seed_wrapped,
            TraceArtifactKind::ContributionEnvelope,
        )
        .expect("seed evaluate_trace succeeds");

        let (dek, wrapped) = wrap_fixture_dek(decryptor.as_ref(), tenant.tenant_storage_ref());
        let ciphertext = aead_encrypt_with_dek(&dek, &plaintext).expect("encrypt fixture");

        let d = svc
            .evaluate_trace(
                &tenant,
                &ciphertext,
                &wrapped,
                TraceArtifactKind::ContributionEnvelope,
            )
            .expect("multi-chunk evaluate_trace succeeds");

        assert_eq!(d.chunk_count, 16, "cap must bound the chunk count");
        assert!(d.chunks_capped);
        assert!(d.perplexity_micros > 0);
        assert!(d.peak_perplexity_micros >= d.perplexity_micros || d.chunk_count == 1);
        // Chunk 0 duplicates the seed trace's sole chunk (near-zero
        // novelty); chunks 1..15 are fresh against the index snapshot taken
        // before this call. Peak must reflect a fresh chunk while the
        // representative is dragged down by the one duplicate — a strict
        // inequality, exercising peak-vs-representative divergence the same
        // way the perplexity assertion above does.
        assert!(
            d.peak_novelty_micros > d.novelty_score_micros,
            "peak novelty must strictly exceed the representative when one \
             of 16 chunks is a near-duplicate seeded ahead of this call"
        );
        // Both gates still pass at zero floors: only 1 of 16 chunks is a
        // near-duplicate, so the token-weighted representative stays well
        // above zero. The duplicate chunk (index 0) clears the perplexity
        // and novelty *gates* but falls below the insert-dedup threshold,
        // so only 15 of 16 chunks land new per-chunk entries.
        assert!(d.perplexity_passed && d.novelty_passed);
        assert_eq!(d.chunk_vector_entries.len(), 15);
        assert_eq!(
            d.vector_entry_id,
            Some(d.chunk_vector_entries[0].vector_entry_id)
        );

        // Re-submit the same trace: every chunk is now a near-duplicate.
        let (dek2, wrapped2) = wrap_fixture_dek(decryptor.as_ref(), tenant.tenant_storage_ref());
        let ciphertext2 = aead_encrypt_with_dek(&dek2, &plaintext).expect("encrypt fixture");
        let d2 = svc
            .evaluate_trace(
                &tenant,
                &ciphertext2,
                &wrapped2,
                TraceArtifactKind::ContributionEnvelope,
            )
            .expect("duplicate evaluate_trace succeeds");
        assert!(
            d2.novelty_score_micros < 50_000,
            "duplicate trace novelty must collapse below the insert threshold"
        );
        assert!(
            d2.chunk_vector_entries.is_empty() || !d2.novelty_passed,
            "duplicate chunks must be deduped on insert"
        );
    }

    // ---- correction simhash (S5 shadow value) ----

    #[test]
    fn correction_simhash_is_over_the_correction_text_alone() {
        let correction = "the agent wrote to config/prod.toml when the task said staging";
        let envelope = serde_json::json!({
            "submission_id": "11111111-1111-1111-1111-111111111111",
            "events": [{"event_type": "user_message", "redacted_content": "some session text"}],
            "outcome": {"task_success": "failure", "human_correction": correction},
        });
        let plaintext = serde_json::to_vec(&envelope).expect("envelope serializes");
        assert_eq!(
            correction_simhash_from_plaintext(&plaintext),
            Some(crate::dedup_simhash::trace_simhash(correction) as i64),
            "the correction simhash must be over the correction text alone"
        );

        // The SAME correction inside a different session must produce the same
        // signature, or a contributor could re-earn by pasting it into a new
        // trace.
        let other_session = serde_json::json!({
            "submission_id": "22222222-2222-2222-2222-222222222222",
            "events": [{"event_type": "user_message", "redacted_content": "entirely other text"}],
            "outcome": {"task_success": "partial", "human_correction": correction},
        });
        let other_plaintext = serde_json::to_vec(&other_session).expect("envelope serializes");
        assert_eq!(
            correction_simhash_from_plaintext(&plaintext),
            correction_simhash_from_plaintext(&other_plaintext)
        );
    }

    #[test]
    fn no_correction_yields_no_signal() {
        for outcome in [
            serde_json::json!({"task_success": "success"}),
            serde_json::json!({"task_success": "failure", "human_correction": serde_json::Value::Null}),
            serde_json::json!({"task_success": "failure", "human_correction": "   "}),
        ] {
            let plaintext = serde_json::to_vec(&serde_json::json!({"outcome": outcome}))
                .expect("envelope serializes");
            assert_eq!(
                correction_simhash_from_plaintext(&plaintext),
                None,
                "an envelope with no correction must produce no correction signal"
            );
        }
        // No outcome at all, and non-JSON plaintext, both degrade to None
        // rather than erroring: the correction value is shadow-only.
        assert_eq!(correction_simhash_from_plaintext(b"{}"), None);
        assert_eq!(correction_simhash_from_plaintext(b"not json at all"), None);
    }

    #[test]
    fn enclave_evaluation_carries_the_correction_signal_end_to_end() {
        // Drives the real decrypt path: the correction signal must come out of
        // `evaluate_trace` for an envelope that carries one, and stay absent
        // for one that does not.
        let decryptor = fixture_decryptor();
        let svc = EnclaveGateService::mock_with_decryptor(Arc::clone(&decryptor));
        let tenant = TenantCtx::new("tenant-a");
        let correction = "the agent should have run the failing test before closing the bug";

        let evaluate = |body: serde_json::Value| {
            let (dek, wrapped) = wrap_fixture_dek(decryptor.as_ref(), tenant.tenant_storage_ref());
            let plaintext = serde_json::to_vec(&body).expect("envelope serializes");
            let ciphertext = aead_encrypt_with_dek(&dek, &plaintext).expect("encrypt fixture");
            svc.evaluate_trace(
                &tenant,
                &ciphertext,
                &wrapped,
                TraceArtifactKind::ContributionEnvelope,
            )
            .expect("evaluate_trace should succeed")
        };

        let with_correction = evaluate(serde_json::json!({
            "events": [{"event_type": "user_message", "redacted_content": "session text"}],
            "outcome": {"task_success": "failure", "human_correction": correction},
        }));
        assert_eq!(
            with_correction.correction_simhash,
            Some(crate::dedup_simhash::trace_simhash(correction) as i64)
        );

        let without_correction = evaluate(serde_json::json!({
            "events": [{"event_type": "user_message", "redacted_content": "session text"}],
            "outcome": {"task_success": "success"},
        }));
        assert_eq!(
            without_correction.correction_simhash, None,
            "an envelope with no correction must not produce a correction signal"
        );
    }
}
