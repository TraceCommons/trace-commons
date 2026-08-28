// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use uuid::Uuid;

/// Config for `EnclaveGateOrchestrator`. Floors are inclusive lower bounds in
/// micros; passing means `value >= floor`.
#[derive(Debug, Clone)]
pub struct EnclaveGateOrchestratorConfig {
    pub gate_policy_version: String,
    pub gate_version_hash: String,
    pub perplexity_floor_micros: u64,
    pub tail_fraction_floor_micros: u64,
    pub novelty_floor_micros: u64,
    pub top_k: usize,
    /// Greedy chunk-packing target, in tokens (char proxy). Default 2048.
    pub chunk_target_tokens: usize,
    /// Hard per-chunk max, in tokens. Default 3072.
    pub chunk_max_tokens: usize,
    /// Hard cap on chunks per trace. Default 16.
    pub chunk_cap: usize,
    /// Min scored tokens for a chunk to be peak-eligible. Default 64.
    pub chunk_min_tokens: u64,
    /// Per-chunk index-insert dedup threshold: a chunk whose novelty is
    /// below this is a near-duplicate and is not inserted. Default 50000.
    pub embed_insert_novelty_micros: u64,
}

impl EnclaveGateOrchestratorConfig {
    /// Sensible defaults for tests / dev: floors set so the mock distribution
    /// emits a mix of pass / fail without callers having to tune.
    pub fn mock_default() -> Self {
        Self {
            gate_policy_version: "enclave_mock_v1".into(),
            gate_version_hash: "sha256:enclave_mock_v1".into(),
            perplexity_floor_micros: 0,
            tail_fraction_floor_micros: 0,
            novelty_floor_micros: 0,
            top_k: 8,
            chunk_target_tokens: 2048,
            chunk_max_tokens: 3072,
            chunk_cap: 16,
            chunk_min_tokens: 64,
            embed_insert_novelty_micros: 50_000,
        }
    }
}

/// Output of `EnclaveGateOrchestrator::evaluate`. The host-side
/// `EnclaveGateService` maps this into the audit-row shape stored in
/// `trace_gate_decisions`.
#[derive(Debug, Clone, PartialEq)]
pub struct OrchestrationDecision {
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
    /// `Some(id)` when both gates passed and the orchestrator inserted the
    /// embedding into the vector index; `None` otherwise.
    pub inserted_entry_id: Option<Uuid>,
    /// Peak (most-surprising min-content-guarded chunk) perplexity.
    /// Equals `perplexity_micros` for single-chunk traces.
    pub peak_perplexity_micros: u64,
    /// Peak (most-novel min-content-guarded chunk) novelty. Equals
    /// `novelty_score_micros` for single-chunk traces.
    pub peak_novelty_micros: u64,
    /// Number of chunks scored (>= 1).
    pub chunk_count: u32,
    /// Total chunks the trace produced BEFORE the per-trace cap was applied
    /// (>= `chunk_count`). The denominator of the coverage this decision
    /// actually has: a capped decision scored `chunk_count` of
    /// `total_chunk_count` chunks. Equals `chunk_count` when nothing was
    /// dropped.
    pub total_chunk_count: u32,
    /// True when the per-trace chunk cap dropped trailing chunks.
    pub chunks_capped: bool,
    /// Every chunk entry inserted into the vector index (both gates passed,
    /// per-chunk novelty at or above the insert threshold). Empty on fail.
    pub inserted_chunk_entries: Vec<InsertedChunkEntry>,
}

/// Output of `EnclaveGateOrchestrator::evaluate_perplexity_only`. Carries
/// only the perplexity-derived fields — there is deliberately no novelty,
/// embedding, or vector-entry state, because the perplexity-only path never
/// touches the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerplexityOnlyOutcome {
    /// Representative (token-weighted) perplexity in micros.
    pub perplexity_micros: u64,
    /// Peak (most-surprising min-content-guarded chunk) perplexity in micros.
    pub peak_perplexity_micros: u64,
    /// Tail-fraction in micros, aggregated across chunks.
    pub tail_fraction_micros: u64,
    /// Whether the representative perplexity clears the configured floor (and
    /// tail-fraction clears its floor) — the same predicate `evaluate` uses.
    pub perplexity_passed: bool,
    /// Number of chunks scored (>= 1).
    pub chunk_count: u32,
    /// True when the per-trace chunk cap dropped trailing chunks.
    pub chunks_capped: bool,
}

/// A per-chunk vector-index entry the orchestrator inserted. The host maps
/// these to `(submission_id, chunk_index)` rows in
/// `trace_gate_chunk_vector_entries` for revocation tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertedChunkEntry {
    pub chunk_index: u32,
    pub entry_id: Uuid,
}
