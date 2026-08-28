// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Compose `PerplexityScorer` + `Embedder` + `VectorIndex` into a single
//! gate-decision pipeline.

use anyhow::Context;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::embedder::Embedder;
use crate::perplexity::PerplexityScorer;
use crate::vector_index::VectorIndex;
use trace_commons_gate_api::{
    EnclaveGateOrchestratorConfig, InsertedChunkEntry, OrchestrationDecision, PerplexityOnlyOutcome,
};

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

    /// Make every pending vector-index write durable.
    ///
    /// The novelty corpus is the only thing that defines what "duplicate"
    /// means, so a shutdown path that skips this quietly resets the gate.
    /// Called on graceful shutdown; a no-op for in-memory indexes.
    pub fn flush_vector_index(&self) -> anyhow::Result<()> {
        self.index.flush()
    }

    /// Chunk `plaintext` and score every chunk for perplexity, returning the
    /// chunk plan, the per-chunk scores, and the aggregated perplexity. This is
    /// the perplexity-only prefix shared by [`Self::evaluate`] and
    /// [`Self::evaluate_perplexity_only`]; it performs NO embedding and NEVER
    /// reads from or writes to the vector index, so callers that only need
    /// perplexity are guaranteed to run the exact same chunking + scoring path
    /// as a full evaluation without any novelty/vector side effects.
    fn chunk_and_score_perplexity(
        &self,
        plaintext: &[u8],
    ) -> anyhow::Result<(
        crate::chunker::ChunkPlan,
        Vec<crate::perplexity::ChunkPerplexity>,
        crate::chunk_aggregate::ChunkedPerplexityAggregate,
    )> {
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
            self.cfg.qualifying_chunk_floor_micros,
        );
        Ok((plan, chunk_scores, perp_agg))
    }

    /// Compute ONLY the perplexity portion of a gate evaluation.
    ///
    /// Runs the identical chunking + per-chunk scoring + aggregation the full
    /// [`Self::evaluate`] pipeline runs (via the shared
    /// [`Self::chunk_and_score_perplexity`]) and applies the same configured
    /// perplexity/tail floors to derive `perplexity_passed`. It does NOT embed,
    /// does NOT query nearest neighbors, and does NOT insert into the vector
    /// index — the novelty/embedding state is left completely untouched. This
    /// is the surgical path the perplexity re-score maintenance task calls so
    /// re-scored values are byte-comparable to production perplexity without
    /// mutating the vector index.
    pub fn evaluate_perplexity_only(
        &self,
        plaintext: &[u8],
    ) -> anyhow::Result<PerplexityOnlyOutcome> {
        let (plan, _chunk_scores, perp_agg) = self.chunk_and_score_perplexity(plaintext)?;
        let perplexity_passed = perp_agg.representative_perplexity_micros
            >= self.cfg.perplexity_floor_micros
            && perp_agg.tail_fraction_micros >= self.cfg.tail_fraction_floor_micros;
        Ok(PerplexityOnlyOutcome {
            perplexity_micros: perp_agg.representative_perplexity_micros,
            peak_perplexity_micros: perp_agg.peak_perplexity_micros,
            tail_fraction_micros: perp_agg.tail_fraction_micros,
            perplexity_passed,
            chunk_count: plan.chunks.len() as u32,
            chunks_capped: plan.chunks_capped,
        })
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
        let (plan, chunk_scores, perp_agg) = self.chunk_and_score_perplexity(plaintext)?;

        // Per-chunk embedding: mean-pooled sub-windows so the vector covers
        // the whole chunk (no 512-token truncation), then per-chunk novelty
        // against the tenant's existing per-chunk entries. Fail-closed: any
        // chunk's embedder/index error refuses the evaluation.
        // Instrumentation (#199), captured HERE: before the first `nearest`
        // query and therefore before any of this trace's own entries reach
        // the shard. A decision has to describe the corpus its novelty was
        // measured against, not the one it went on to enlarge.
        let index_snapshot = self.index.snapshot(tenant_storage_ref);

        let mut chunk_embeddings: Vec<Vec<f32>> = Vec::with_capacity(plan.chunks.len());
        let mut chunk_novelty_micros: Vec<u64> = Vec::with_capacity(plan.chunks.len());
        let mut all_neighbors: Vec<crate::vector_index::NearestNeighbor> = Vec::new();
        for chunk in &plan.chunks {
            let emb = crate::embedder::embed_chunk_mean_pooled(&self.embedder, &chunk.text)
                .context("EmbedderInferenceFailed")?;
            let neighbors = self
                .index
                .nearest(tenant_storage_ref, &emb, self.cfg.top_k)?;
            let max_sim = neighbors
                .iter()
                .map(|n| n.similarity)
                .fold(f32::NEG_INFINITY, f32::max);
            let novelty_f = if max_sim.is_finite() {
                (1.0 - max_sim).max(0.0)
            } else {
                1.0
            };
            chunk_novelty_micros.push((novelty_f.clamp(0.0, 2.0) * 1_000_000.0) as u64);
            all_neighbors.extend(neighbors);
            chunk_embeddings.push(emb);
        }
        let chunk_token_counts: Vec<u64> = chunk_scores.iter().map(|c| c.tokens).collect();
        let (novelty_score_micros, peak_novelty_micros) =
            crate::chunk_aggregate::aggregate_chunked_novelty(
                &chunk_novelty_micros,
                &chunk_token_counts,
                self.cfg.chunk_min_tokens,
            );

        let perplexity_passed = perp_agg.representative_perplexity_micros
            >= self.cfg.perplexity_floor_micros
            && perp_agg.tail_fraction_micros >= self.cfg.tail_fraction_floor_micros;
        let novelty_passed = novelty_score_micros >= self.cfg.novelty_floor_micros;

        let nearest_neighbor_hash = hash_neighbors(&all_neighbors);
        let embedding_evidence_hash = hash_chunk_embedding_evidence(
            &self.cfg.gate_policy_version,
            tenant_storage_ref,
            &chunk_embeddings,
        );
        let attestation_chain_hash =
            hash_attestation_chain(&self.cfg.gate_policy_version, &self.cfg.gate_version_hash);

        // Insert per-chunk entries: both gates passed AND the chunk clears
        // the near-duplicate threshold. Novelties were all computed against
        // the pre-trace index (matches the design pseudocode), so intra-
        // trace duplicate chunks each measure against prior traces only.
        let mut inserted_chunk_entries: Vec<InsertedChunkEntry> = Vec::new();
        if perplexity_passed && novelty_passed {
            for (i, emb) in chunk_embeddings.iter().enumerate() {
                if chunk_novelty_micros[i] < self.cfg.embed_insert_novelty_micros {
                    continue;
                }
                let entry_id = Uuid::new_v4();
                self.index.insert(entry_id, tenant_storage_ref, emb)?;
                inserted_chunk_entries.push(InsertedChunkEntry {
                    chunk_index: plan.chunks[i].chunk_index,
                    entry_id,
                });
            }
        }
        let inserted_entry_id = inserted_chunk_entries.first().map(|e| e.entry_id);

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
            peak_novelty_micros,
            chunk_count: plan.chunks.len() as u32,
            // Pre-cap denominator: what was scored plus what the cap dropped.
            // `ChunkPlan` has always computed the dropped count; until now it
            // was only logged, so a capped decision could never state its own
            // coverage.
            total_chunk_count: (plan.chunks.len() as u32).saturating_add(plan.dropped_chunk_count),
            chunks_capped: plan.chunks_capped,
            inserted_chunk_entries,
            vector_index_snapshot_id: index_snapshot.map(|s| s.snapshot_id),
            index_cardinality_at_scoring: index_snapshot.map(|s| s.cardinality),
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

fn hash_chunk_embedding_evidence(
    gate_policy_version: &str,
    tenant_storage_ref: &str,
    chunk_embeddings: &[Vec<f32>],
) -> String {
    let mut h = Sha256::new();
    h.update(b"trace_gate_enclave.embedding_evidence.v2\n");
    h.update(gate_policy_version.as_bytes());
    h.update(b"\n");
    h.update(tenant_storage_ref.as_bytes());
    h.update(b"\n");
    for (i, embedding) in chunk_embeddings.iter().enumerate() {
        h.update((i as u32).to_be_bytes());
        h.update(b"\n");
        for x in embedding {
            h.update(x.to_be_bytes());
        }
        h.update(b"\n");
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

    /// #199: novelty is `1 - max cosine similarity` against whatever was in
    /// the tenant's shard at scoring time, so a stored novelty score is not
    /// reproducible without knowing which shard and how full it was. Both are
    /// captured BEFORE this trace's own chunks are inserted -- a decision must
    /// describe the index it was scored against, not the one it created.
    #[test]
    fn decision_records_the_index_state_novelty_was_scored_against() {
        let orch = orch_with_floors(0, 0, 0);
        let first = orch.evaluate(b"hello world", "tenant_a").unwrap();
        assert_eq!(
            first.index_cardinality_at_scoring,
            Some(0),
            "the first trace of a tenant scores against an empty shard; 0 is a \
             real observation and must not be reported as a missing one"
        );
        let snapshot = first
            .vector_index_snapshot_id
            .expect("a real index must identify the shard it scored against");
        assert!(first.inserted_entry_id.is_some());

        let second = orch
            .evaluate(
                b"an entirely different trace about other things",
                "tenant_a",
            )
            .unwrap();
        assert_eq!(
            second.vector_index_snapshot_id,
            Some(snapshot),
            "same shard generation: the id changes on reopen or rebuild, not per write"
        );
        assert!(
            second.index_cardinality_at_scoring.unwrap() > 0,
            "the second trace scored against a shard the first one filled"
        );
    }

    /// The shard, not the process. Novelty is per-tenant, so two tenants
    /// scored in the same process are two different corpora and must not be
    /// averaged across.
    #[test]
    fn snapshot_ids_are_per_tenant_shard() {
        let orch = orch_with_floors(0, 0, 0);
        let a = orch.evaluate(b"hello world", "tenant_a").unwrap();
        let b = orch.evaluate(b"hello world", "tenant_b").unwrap();
        assert_ne!(a.vector_index_snapshot_id, b.vector_index_snapshot_id);
        assert_eq!(b.index_cardinality_at_scoring, Some(0));
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
    fn perplexity_only_matches_evaluate_perplexity_and_leaves_index_untouched() {
        // A high novelty floor means a full re-evaluation of an already-indexed
        // trace would FAIL novelty (and therefore not insert). We use this to
        // prove the index is never mutated by evaluate_perplexity_only: after
        // any number of perplexity-only calls, a subsequent full evaluate of a
        // fresh trace still passes novelty (index still empty).
        let orch = orch_with_floors(0, 0, 900_000);

        // Reference full evaluation captures the production perplexity values.
        let full = orch.evaluate(b"hello world", "tenant_a").unwrap();
        assert!(full.perplexity_passed && full.novelty_passed);
        assert!(full.inserted_entry_id.is_some());

        // A brand-new orchestrator (empty index) drives perplexity-only many
        // times; values must match the full path's perplexity fields exactly.
        let orch2 = orch_with_floors(0, 0, 900_000);
        for _ in 0..5 {
            let po = orch2.evaluate_perplexity_only(b"hello world").unwrap();
            assert_eq!(po.perplexity_micros, full.perplexity_micros);
            assert_eq!(po.peak_perplexity_micros, full.peak_perplexity_micros);
            assert_eq!(po.tail_fraction_micros, full.tail_fraction_micros);
            assert_eq!(po.chunk_count, full.chunk_count);
        }

        // The index was never written by the perplexity-only calls: a fresh
        // trace still measures full novelty and passes under the high floor.
        let after = orch2.evaluate(b"hello world", "tenant_a").unwrap();
        assert!(
            after.novelty_passed,
            "perplexity-only must not have inserted anything into the index"
        );
        assert!(after.inserted_entry_id.is_some());
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
            let call = self.n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
        assert_eq!(
            d.total_chunk_count, 1,
            "an uncapped trace's total equals what was scored"
        );
        assert!(d.perplexity_micros.abs_diff(2_718_281) <= 2);
        assert!(d.peak_perplexity_micros.abs_diff(2_718_281) <= 2);
        assert!(d.tail_fraction_micros.abs_diff(250_000) <= 2_500);
        assert_eq!(d.peak_novelty_micros, d.novelty_score_micros);
        assert_eq!(d.inserted_chunk_entries.len(), 1);
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
        // The pre-cap denominator reaches the decision, so a capped trace can
        // be reported as "16 of 20" instead of silently as "16".
        assert_eq!(d.total_chunk_count, 20);
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

    #[test]
    fn duplicate_chunk_is_deduped_on_insert_but_novel_chunks_insert_per_chunk() {
        // First trace: 2 distinct large chunks -> both insert.
        let orch = orch_with_floors(0, 0, 0);
        let first = orch
            .evaluate(&two_distinct_chunk_envelope("alpha", "beta"), "tenant_a")
            .unwrap();
        assert_eq!(first.chunk_count, 2);
        assert_eq!(first.inserted_chunk_entries.len(), 2);
        assert_eq!(first.inserted_chunk_entries[0].chunk_index, 0);
        assert_eq!(first.inserted_chunk_entries[1].chunk_index, 1);
        assert_eq!(
            first.inserted_entry_id,
            Some(first.inserted_chunk_entries[0].entry_id)
        );

        // Second trace: chunk 0 duplicates the first trace's chunk 0
        // (novelty ~0 < insert threshold -> skipped); chunk 1 is new.
        let second = orch
            .evaluate(&two_distinct_chunk_envelope("alpha", "gamma"), "tenant_a")
            .unwrap();
        assert_eq!(second.chunk_count, 2);
        assert_eq!(
            second.inserted_chunk_entries.len(),
            1,
            "near-duplicate chunk must be skipped at the insert threshold"
        );
        assert_eq!(second.inserted_chunk_entries[0].chunk_index, 1);
    }

    #[test]
    fn peak_novelty_is_max_over_chunks_and_representative_is_weighted() {
        let orch = orch_with_floors(0, 0, 0);
        // Seed the index with one trace.
        orch.evaluate(&two_distinct_chunk_envelope("alpha", "beta"), "tenant_a")
            .unwrap();
        // Re-submit: chunk "alpha" is a duplicate (novelty ~0), chunk
        // "delta" is fresh (novelty ~1.0). Peak must reflect the fresh
        // chunk; representative must sit strictly between.
        let d = orch
            .evaluate(&two_distinct_chunk_envelope("alpha", "delta"), "tenant_a")
            .unwrap();
        assert!(
            d.peak_novelty_micros > 900_000,
            "fresh chunk drives the peak"
        );
        assert!(
            d.novelty_score_micros < d.peak_novelty_micros,
            "representative must be dragged down by the duplicate chunk"
        );
    }

    #[test]
    fn failed_gate_inserts_no_chunk_entries() {
        let orch = orch_with_floors(u64::MAX, 0, 0);
        let d = orch
            .evaluate(&two_distinct_chunk_envelope("alpha", "beta"), "tenant_a")
            .unwrap();
        assert!(!d.perplexity_passed);
        assert!(d.inserted_chunk_entries.is_empty());
        assert!(d.inserted_entry_id.is_none());
    }

    /// Envelope with exactly two chunks: each event's rendered form fills a
    /// whole target chunk (8192 chars at the default 2048-token target).
    fn two_distinct_chunk_envelope(seed_a: &str, seed_b: &str) -> Vec<u8> {
        let pad = |seed: &str| {
            let mut s = String::new();
            while s.len() < 8_000 {
                s.push_str(seed);
                s.push(' ');
            }
            s
        };
        serde_json::to_vec(&serde_json::json!({
            "events": [
                { "event_type": "assistant_message", "redacted_content": pad(seed_a) },
                { "event_type": "assistant_message", "redacted_content": pad(seed_b) },
            ]
        }))
        .unwrap()
    }
}
