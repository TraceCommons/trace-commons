//! Compose `PerplexityScorer` + `Embedder` + `VectorIndex` into a single
//! gate-decision pipeline.

use anyhow::Context;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::embedder::Embedder;
use crate::perplexity::PerplexityScorer;
use crate::vector_index::VectorIndex;

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
    /// True when the per-trace chunk cap dropped trailing chunks.
    pub chunks_capped: bool,
}

/// Pipelines a plaintext through perplexity + embedding + novelty gates and
/// returns a deterministic `OrchestrationDecision`.
pub struct EnclaveGateOrchestrator<P, E, V> {
    perplexity: P,
    embedder: E,
    index: V,
    cfg: EnclaveGateOrchestratorConfig,
}

impl<P, E, V> EnclaveGateOrchestrator<P, E, V>
where
    P: PerplexityScorer,
    E: Embedder,
    V: VectorIndex,
{
    pub fn new(perplexity: P, embedder: E, index: V, cfg: EnclaveGateOrchestratorConfig) -> Self {
        Self {
            perplexity,
            embedder,
            index,
            cfg,
        }
    }

    pub fn config(&self) -> &EnclaveGateOrchestratorConfig {
        &self.cfg
    }

    /// Remove an entry from the orchestrator's vector index by `entry_id`.
    ///
    /// Returns `Ok(true)` if the entry was found and removed, `Ok(false)` if
    /// no such entry existed (idempotent: already-absent is a satisfied
    /// postcondition). Callers that only need "make sure it's gone" can safely
    /// discard the bool.
    pub fn delete_vector_entry(
        &self,
        tenant_storage_ref: &str,
        entry_id: Uuid,
    ) -> anyhow::Result<bool> {
        self.index.delete(tenant_storage_ref, entry_id)
    }

    /// Evaluate `plaintext` under the orchestrator's gate policy.
    ///
    /// Steps:
    ///  1. Score for perplexity.
    ///  2. Embed.
    ///  3. Query top-k nearest neighbors and compute novelty as
    ///     `1 - max(cosine_similarity)`.
    ///  4. Apply floors → bool pass flags.
    ///  5. If both pass, insert embedding into the index so future traces see
    ///     it; otherwise leave the index untouched.
    pub fn evaluate(
        &self,
        plaintext: &[u8],
        tenant_storage_ref: &str,
    ) -> anyhow::Result<OrchestrationDecision> {
        // Chunk the parsed envelope. Every backend request downstream is now
        // bounded — the root-cause fix for the TEE OOM crashes.
        let chunker_cfg = crate::chunker::ChunkerConfig {
            target_tokens: self.cfg.chunk_target_tokens,
            max_tokens: self.cfg.chunk_max_tokens,
            chunk_cap: self.cfg.chunk_cap,
        };
        let plan = crate::chunker::chunk_envelope_plaintext(plaintext, &chunker_cfg);
        if plan.chunks_capped {
            // Hash-only: counts and a fixed label only. Never chunk content.
            tracing::warn!(
                error_class = "TraceChunkCapExceeded",
                chunk_count = plan.chunks.len(),
                dropped_chunk_count = plan.dropped_chunk_count,
                "trace chunk cap enforced"
            );
        }

        // Fail-closed inference: any chunk's scorer error refuses the whole
        // evaluation (v1 semantics). Sequential — never a concurrent burst
        // against one pinned backend.
        let mut chunk_scores: Vec<crate::perplexity::ChunkPerplexity> =
            Vec::with_capacity(plan.chunks.len());
        for chunk in &plan.chunks {
            let cs = self
                .perplexity
                .score_chunk(chunk.text.as_bytes())
                .context("PerplexityScorerInferenceFailed")?;
            chunk_scores.push(cs);
        }
        let perp_agg = crate::chunk_aggregate::aggregate_chunked_perplexity(
            &chunk_scores,
            self.cfg.chunk_min_tokens,
        );

        // Embedding path: whole-plaintext for now (per-chunk embedding lands
        // with the embedding half of the chunked-scoring slice).
        let embedding = self
            .embedder
            .embed(plaintext)
            .context("EmbedderInferenceFailed")?;
        let neighbors = self
            .index
            .nearest(tenant_storage_ref, &embedding, self.cfg.top_k)?;

        // novelty_score = 1 - max similarity, scaled to micros. Cosine sim
        // lives in [-1, 1]; clamp before mapping so the micros stay in
        // [0, 2_000_000].
        let max_sim = neighbors
            .iter()
            .map(|n| n.similarity)
            .fold(f32::NEG_INFINITY, f32::max);
        let novelty_score_f = if max_sim.is_finite() {
            (1.0 - max_sim).max(0.0)
        } else {
            // No existing entries → maximally novel.
            1.0
        };
        let novelty_score_micros = (novelty_score_f.clamp(0.0, 2.0) * 1_000_000.0) as u64;

        let perplexity_passed = perp_agg.representative_perplexity_micros
            >= self.cfg.perplexity_floor_micros
            && perp_agg.tail_fraction_micros >= self.cfg.tail_fraction_floor_micros;
        let novelty_passed = novelty_score_micros >= self.cfg.novelty_floor_micros;

        let nearest_neighbor_hash = hash_neighbors(&neighbors);
        let embedding_evidence_hash = hash_embedding_evidence(
            &self.cfg.gate_policy_version,
            tenant_storage_ref,
            &embedding,
        );
        let attestation_chain_hash =
            hash_attestation_chain(&self.cfg.gate_policy_version, &self.cfg.gate_version_hash);

        let mut inserted_entry_id = None;
        if perplexity_passed && novelty_passed {
            let entry_id = Uuid::new_v4();
            self.index
                .insert(entry_id, tenant_storage_ref, &embedding)?;
            inserted_entry_id = Some(entry_id);
        }

        Ok(OrchestrationDecision {
            gate_policy_version: self.cfg.gate_policy_version.clone(),
            gate_version_hash: self.cfg.gate_version_hash.clone(),
            perplexity_micros: perp_agg.representative_perplexity_micros,
            tail_fraction_micros: perp_agg.tail_fraction_micros,
            perplexity_passed,
            novelty_score_micros,
            nearest_neighbor_hash,
            novelty_passed,
            embedding_evidence_hash,
            attestation_chain_hash,
            inserted_entry_id,
            peak_perplexity_micros: perp_agg.peak_perplexity_micros,
            // Real per-chunk novelty peak lands with the embedding half;
            // until then peak == representative (exact for 1 chunk).
            peak_novelty_micros: novelty_score_micros,
            chunk_count: plan.chunks.len() as u32,
            chunks_capped: plan.chunks_capped,
        })
    }
}

fn hash_neighbors(neighbors: &[crate::vector_index::NearestNeighbor]) -> String {
    let mut h = Sha256::new();
    h.update(b"trace_gate_enclave.nearest_neighbors.v1\n");
    for n in neighbors {
        h.update(n.entry_id.as_bytes());
        h.update(n.similarity.to_be_bytes());
    }
    format!("sha256:{:x}", h.finalize())
}

fn hash_embedding_evidence(
    gate_policy_version: &str,
    tenant_storage_ref: &str,
    embedding: &[f32],
) -> String {
    let mut h = Sha256::new();
    h.update(b"trace_gate_enclave.embedding_evidence.v1\n");
    h.update(gate_policy_version.as_bytes());
    h.update(b"\n");
    h.update(tenant_storage_ref.as_bytes());
    h.update(b"\n");
    for x in embedding {
        h.update(x.to_be_bytes());
    }
    format!("sha256:{:x}", h.finalize())
}

fn hash_attestation_chain(gate_policy_version: &str, gate_version_hash: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"trace_gate_enclave.attestation_chain.v1\n");
    h.update(gate_policy_version.as_bytes());
    h.update(b"\n");
    h.update(gate_version_hash.as_bytes());
    format!("sha256:{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::MockEmbedder;
    use crate::perplexity::MockPerplexityScorer;
    use crate::vector_index::MockVectorIndex;

    fn orch_with_floors(
        perplexity_floor_micros: u64,
        tail_fraction_floor_micros: u64,
        novelty_floor_micros: u64,
    ) -> EnclaveGateOrchestrator<MockPerplexityScorer, MockEmbedder, MockVectorIndex> {
        let mut cfg = EnclaveGateOrchestratorConfig::mock_default();
        cfg.perplexity_floor_micros = perplexity_floor_micros;
        cfg.tail_fraction_floor_micros = tail_fraction_floor_micros;
        cfg.novelty_floor_micros = novelty_floor_micros;
        EnclaveGateOrchestrator::new(
            MockPerplexityScorer::new(),
            MockEmbedder::new(),
            MockVectorIndex::new(),
            cfg,
        )
    }

    #[test]
    fn deterministic_for_same_input() {
        let orch_a = orch_with_floors(0, 0, 0);
        let orch_b = orch_with_floors(0, 0, 0);
        let a = orch_a.evaluate(b"hello world", "tenant_a").unwrap();
        let b = orch_b.evaluate(b"hello world", "tenant_a").unwrap();
        // inserted_entry_id is a fresh UUID each run; everything else stable.
        assert_eq!(a.perplexity_micros, b.perplexity_micros);
        assert_eq!(a.tail_fraction_micros, b.tail_fraction_micros);
        assert_eq!(a.novelty_score_micros, b.novelty_score_micros);
        assert_eq!(a.embedding_evidence_hash, b.embedding_evidence_hash);
        assert_eq!(a.attestation_chain_hash, b.attestation_chain_hash);
        assert_eq!(a.perplexity_passed, b.perplexity_passed);
        assert_eq!(a.novelty_passed, b.novelty_passed);
    }

    #[test]
    fn inserted_trace_is_its_own_nearest_neighbor_next_time() {
        let orch = orch_with_floors(0, 0, 0);
        let first = orch.evaluate(b"hello world", "tenant_a").unwrap();
        assert!(first.perplexity_passed && first.novelty_passed);
        assert!(first.inserted_entry_id.is_some());

        // Second evaluation of the SAME plaintext should see the prior entry
        // as a near-perfect (cosine ~ 1.0) neighbor → novelty collapses to
        // near zero.
        let second = orch.evaluate(b"hello world", "tenant_a").unwrap();
        assert!(
            second.novelty_score_micros < 10_000,
            "expected near-zero novelty after self-insert, got {}",
            second.novelty_score_micros
        );
    }

    #[test]
    fn high_novelty_floor_fails_a_duplicate_trace() {
        // Very high novelty floor: a re-submitted trace fails because the
        // index already contains its embedding.
        let orch = orch_with_floors(0, 0, 900_000);
        let first = orch.evaluate(b"hello world", "tenant_a").unwrap();
        assert!(first.novelty_passed, "fresh trace should pass novelty");
        let second = orch.evaluate(b"hello world", "tenant_a").unwrap();
        assert!(
            !second.novelty_passed,
            "duplicate trace should fail under a high novelty floor"
        );
        // Failed → orchestrator did NOT insert.
        assert!(second.inserted_entry_id.is_none());
    }

    use crate::perplexity::PerplexityResult;

    /// Fixed-value scorer for exact-identity assertions.
    struct StubScorer;
    impl crate::perplexity::PerplexityScorer for StubScorer {
        fn score(&self, _plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
            Ok(PerplexityResult {
                aggregate_perplexity_micros: 2_718_281,
                tail_fraction_micros: 250_000,
                tokens_scored: 100,
            })
        }
    }

    /// Scorer that fails on its Nth call — proves fail-closed mid-loop.
    struct FailOnNthScorer {
        n: std::sync::atomic::AtomicUsize,
        fail_at: usize,
    }
    impl crate::perplexity::PerplexityScorer for FailOnNthScorer {
        fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
            let call = self
                .n
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == self.fail_at {
                anyhow::bail!("ChunkScorerInjectedFailure");
            }
            MockPerplexityScorer::new().score(plaintext)
        }
    }

    fn multi_chunk_envelope(event_count: usize, event_chars: usize) -> Vec<u8> {
        let content = "a".repeat(event_chars);
        let events: Vec<serde_json::Value> = (0..event_count)
            .map(|_| {
                serde_json::json!({
                    "event_type": "assistant_message",
                    "redacted_content": content,
                })
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({ "events": events })).unwrap()
    }

    #[test]
    fn single_chunk_trace_matches_whole_call_on_rendered_text() {
        // A small trace is one chunk; its representative and peak must equal
        // the scorer's single-call values on that same chunk text (identity
        // within ln/exp round-trip tolerance for the derived default
        // score_chunk path).
        let mut cfg = EnclaveGateOrchestratorConfig::mock_default();
        cfg.chunk_min_tokens = 1;
        let orch = EnclaveGateOrchestrator::new(
            StubScorer,
            MockEmbedder::new(),
            MockVectorIndex::new(),
            cfg,
        );
        let d = orch
            .evaluate(&multi_chunk_envelope(1, 40), "tenant_a")
            .unwrap();
        assert_eq!(d.chunk_count, 1);
        assert!(!d.chunks_capped);
        assert!(d.perplexity_micros.abs_diff(2_718_281) <= 2);
        assert!(d.peak_perplexity_micros.abs_diff(2_718_281) <= 2);
        assert!(d.tail_fraction_micros.abs_diff(250_000) <= 2_500);
        assert_eq!(d.peak_novelty_micros, d.novelty_score_micros);
    }

    #[test]
    fn large_trace_produces_multiple_chunks_and_cap() {
        // 20 events of 8000 chars each: every event fills a ~2048-token
        // (8192-char) target chunk alone -> 20 chunks -> capped at 16.
        let orch = orch_with_floors(0, 0, 0);
        let d = orch
            .evaluate(&multi_chunk_envelope(20, 8_000), "tenant_a")
            .unwrap();
        assert_eq!(d.chunk_count, 16);
        assert!(d.chunks_capped);
    }

    #[test]
    fn chunk_scorer_error_fails_the_whole_evaluation() {
        let mut cfg = EnclaveGateOrchestratorConfig::mock_default();
        cfg.chunk_min_tokens = 1;
        let orch = EnclaveGateOrchestrator::new(
            FailOnNthScorer {
                n: std::sync::atomic::AtomicUsize::new(0),
                fail_at: 2,
            },
            MockEmbedder::new(),
            MockVectorIndex::new(),
            cfg,
        );
        let err = orch
            .evaluate(&multi_chunk_envelope(20, 8_000), "tenant_a")
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("PerplexityScorerInferenceFailed"),
            "fail-closed error context missing: {err:#}"
        );
    }

    #[test]
    fn impossible_perplexity_floor_fails() {
        let orch = orch_with_floors(u64::MAX, 0, 0);
        let d = orch.evaluate(b"hello world", "tenant_a").unwrap();
        assert!(!d.perplexity_passed);
        assert!(d.inserted_entry_id.is_none());
    }

    #[test]
    fn distinct_tenants_do_not_cross_pollinate() {
        let orch = orch_with_floors(0, 0, 900_000);
        // Insert under tenant_a.
        let a = orch.evaluate(b"hello world", "tenant_a").unwrap();
        assert!(a.novelty_passed);
        // Same plaintext under tenant_b → tenant_b's index is empty, so
        // novelty stays at 1.0.
        let b = orch.evaluate(b"hello world", "tenant_b").unwrap();
        assert!(b.novelty_passed);
    }
}
