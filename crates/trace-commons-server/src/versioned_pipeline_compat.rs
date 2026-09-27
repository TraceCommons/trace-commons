// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Compatibility Score policy for the versioned pipeline (#971 / PR 3).
//!
//! Reproduces `main`'s gate path as a read-only Score policy: chunked
//! perplexity and novelty, the configured `NoveltyUtility` credit delta
//! (`0` by default, so no award unless an operator opts in), shadow
//! credit-quality evidence (computed and recorded, never gating), and a
//! stored index command holding exact embeddings under their original chunk
//! numbers. Score never writes the live index; Settle applies the stored
//! command later from committed Score evidence only.
//!
//! The compatibility Settle policy, the bundle constructor that binds this
//! policy to a `BundlePackage`, and the baseline-comparison helper against
//! the current fixture land in a later task; this module holds only the
//! config and [`CompatibilityScorePolicy`].

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use trace_commons_gate_api::pipeline::{
    AtomicUnits, BundleManifest, InstrumentAward, InstrumentAwards, InstrumentDescriptor,
    InstrumentId, InstrumentKind, MAX_TRACE_CREDIT_MICROCREDITS, PhaseResult, PolicyError,
    ScoreDecision, ScoreEvaluation, ScoreEvidence, ScoreInput, ScoreOutput, ScorePolicy,
    SealedIndexCommand, SealedIndexEntry, TRACE_CREDIT_DECIMALS, TRACE_CREDIT_INSTRUMENT_ID,
};
use trace_commons_gate_api::{
    IdentifiedEmbedder, IdentifiedIndexReader, IdentifiedPerplexityScorer, NearestNeighbor,
};
use trace_commons_gate_enclave::chunk_aggregate::{
    aggregate_chunked_novelty, aggregate_chunked_perplexity,
};
use trace_commons_gate_enclave::chunker::{ChunkerConfig, chunk_envelope_plaintext};
use trace_commons_gate_enclave::embedder::embed_chunk_mean_pooled;

use crate::credit_quality::{CREDIT_QUALITY_ACTIVE, constants_at, credit_quality};
use crate::versioned_pipeline_bundle::{
    MINIMAL_INDEX_ID, MINIMAL_PROJECTION_ID, PipelineInstrumentAwardConfig, dependency_content_hash,
};

/// Implementation id this policy family answers to in a bundle manifest
/// (P3-D5). Admission and Review are pass-through for the compatibility
/// family, same as the minimal family, but under their own implementation
/// identity so a package cannot silently mix families.
pub const COMPATIBILITY_ADMISSION_IMPLEMENTATION: &str = "trace_commons.admission.compatibility.v1";
pub const COMPATIBILITY_REVIEW_IMPLEMENTATION: &str = "trace_commons.review.compatibility.v1";
pub const COMPATIBILITY_SCORE_IMPLEMENTATION: &str = "trace_commons.score.compatibility.v1";
pub const COMPATIBILITY_SETTLE_IMPLEMENTATION: &str = "trace_commons.settle.compatibility.v1";
/// Safe label for a scorer, embedder, or index dependency call that failed.
/// The Score policy itself reports a more specific label per dependency
/// (`scorer_unavailable`, `embedder_unavailable`, `index_unavailable`); this
/// stays for callers that only need one generic dependency-failure label.
pub const PIPELINE_SCORE_DEPENDENCY_LABEL: &str = "score_dependency_failed";
pub const COMPATIBILITY_SCORE_RULE: &str = "compatibility_quality_novelty_v1";
pub const COMPATIBILITY_SETTLE_RULE: &str = "compatibility_membership_from_score_v1";
pub const COMPATIBILITY_EXCLUDE_REASON: &str = "compatibility_not_eligible";
pub const COMPATIBILITY_ZERO_FLOOR_LABEL: &str = "compatibility_zero_floor";
/// Safe label for a `novelty_utility_microcredits` delta above the trace
/// credit ledger's bound.
pub const COMPATIBILITY_DELTA_OUT_OF_RANGE_LABEL: &str = "compatibility_delta_out_of_range";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityQualification {
    LocalSyntheticNonQualifiable,
    ProductionCompatible,
}

/// Default trace-credit instrument pin shared by [`CompatibilityBundleConfig::local_reference`]
/// and [`CompatibilityBundleConfig::production_compatible`]. `atomic_units` is
/// a placeholder: this task's Score policy never awards from it directly (it
/// awards `novelty_utility_microcredits` instead); a later task's bundle
/// constructor pins the manifest's `trace_credit` descriptor from this value.
fn default_trace_credit_instrument() -> PipelineInstrumentAwardConfig {
    PipelineInstrumentAwardConfig {
        instrument_id: TRACE_CREDIT_INSTRUMENT_ID.to_string(),
        atomic_units: AtomicUnits::ZERO,
        descriptor: InstrumentDescriptor {
            kind: InstrumentKind::Nep141,
            network: "testnet".to_string(),
            contract: "trace-credit.testnet".to_string(),
            decimals: TRACE_CREDIT_DECIMALS,
        },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityBundleConfig {
    pub qualification: CompatibilityQualification,
    pub scorer_model_id: String,
    pub embedder_model_id: String,
    pub projection_id: String,
    pub index_id: String,
    pub perplexity_floor_micros: u64,
    pub tail_fraction_floor_micros: u64,
    pub novelty_floor_micros: u64,
    pub embed_insert_novelty_micros: u64,
    pub top_k: u32,
    pub chunk_target_tokens: u32,
    pub chunk_max_tokens: u32,
    pub chunk_cap: u32,
    pub chunk_min_tokens: u64,
    /// Informational only: shadow credit quality is scored with the era
    /// constants in force when the decision is made ([`constants_at`]), never
    /// with a version pinned ahead of time in config.
    pub credit_quality_version: i32,
    /// The `NoveltyUtility` flat credit delta, in trace-credit microcredits,
    /// awarded when both floors pass. Default `0`: no award unless an
    /// operator opts in (P3-D6 — this is a configured delta, never `q`).
    pub novelty_utility_microcredits: u64,
    /// The instrument this policy's award (if any) is pinned against. A
    /// later task's bundle constructor uses `instrument.descriptor` to pin
    /// `trace_credit` in the bundle manifest.
    pub instrument: PipelineInstrumentAwardConfig,
}

impl CompatibilityBundleConfig {
    pub fn local_reference() -> Self {
        Self {
            qualification: CompatibilityQualification::LocalSyntheticNonQualifiable,
            scorer_model_id: "reference_perplexity.v1".to_string(),
            embedder_model_id: "reference_embedder.v1".to_string(),
            projection_id: MINIMAL_PROJECTION_ID.to_string(),
            index_id: MINIMAL_INDEX_ID.to_string(),
            perplexity_floor_micros: 0,
            tail_fraction_floor_micros: 0,
            novelty_floor_micros: 0,
            embed_insert_novelty_micros: 50_000,
            top_k: 8,
            chunk_target_tokens: 2048,
            chunk_max_tokens: 3072,
            chunk_cap: 16,
            chunk_min_tokens: 64,
            credit_quality_version: CREDIT_QUALITY_ACTIVE.version,
            novelty_utility_microcredits: 0,
            instrument: default_trace_credit_instrument(),
        }
    }

    pub fn production_compatible(
        scorer_model_id: String,
        embedder_model_id: String,
        projection_id: String,
        index_id: String,
        perplexity_floor_micros: u64,
        tail_fraction_floor_micros: u64,
        novelty_floor_micros: u64,
    ) -> anyhow::Result<Self> {
        let config = Self {
            qualification: CompatibilityQualification::ProductionCompatible,
            scorer_model_id,
            embedder_model_id,
            projection_id,
            index_id,
            perplexity_floor_micros,
            tail_fraction_floor_micros,
            novelty_floor_micros,
            embed_insert_novelty_micros: novelty_floor_micros,
            top_k: 8,
            chunk_target_tokens: 2048,
            chunk_max_tokens: 3072,
            chunk_cap: 16,
            chunk_min_tokens: 64,
            credit_quality_version: CREDIT_QUALITY_ACTIVE.version,
            novelty_utility_microcredits: 0,
            instrument: default_trace_credit_instrument(),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.scorer_model_id.trim().is_empty()
                && !self.embedder_model_id.trim().is_empty()
                && !self.projection_id.trim().is_empty()
                && !self.index_id.trim().is_empty(),
            "compatibility dependency identity is missing"
        );
        anyhow::ensure!(
            self.top_k > 0
                && self.chunk_target_tokens > 0
                && self.chunk_max_tokens >= self.chunk_target_tokens
                && self.chunk_cap > 0
                && self.chunk_min_tokens > 0,
            "compatibility scoring bounds are invalid"
        );
        if self.qualification == CompatibilityQualification::ProductionCompatible {
            anyhow::ensure!(
                self.perplexity_floor_micros > 0
                    && self.tail_fraction_floor_micros > 0
                    && self.novelty_floor_micros > 0,
                COMPATIBILITY_ZERO_FLOOR_LABEL
            );
        }
        anyhow::ensure!(
            self.novelty_utility_microcredits <= MAX_TRACE_CREDIT_MICROCREDITS,
            COMPATIBILITY_DELTA_OUT_OF_RANGE_LABEL
        );
        Ok(())
    }

    pub fn is_qualifiable(&self) -> bool {
        self.qualification == CompatibilityQualification::ProductionCompatible
    }
}

/// Reproduces `main`'s gate path as a read-only Score policy: chunked
/// perplexity and novelty aggregation, the configured flat `NoveltyUtility`
/// credit delta, shadow credit-quality evidence, and a stored index command.
/// Holds its dependencies only as trait objects (never a concrete scorer,
/// embedder, or index type), and holds the bundle manifest it was
/// constructed under so every decision it builds is proven pinned
/// (`ScoreDecision::for_bundle`).
pub struct CompatibilityScorePolicy {
    manifest: BundleManifest,
    config: CompatibilityBundleConfig,
    scorer: Arc<dyn IdentifiedPerplexityScorer>,
    embedder: Arc<dyn IdentifiedEmbedder>,
    index_reader: Arc<dyn IdentifiedIndexReader>,
}

impl CompatibilityScorePolicy {
    pub fn new(
        manifest: BundleManifest,
        config: CompatibilityBundleConfig,
        scorer: Arc<dyn IdentifiedPerplexityScorer>,
        embedder: Arc<dyn IdentifiedEmbedder>,
        index_reader: Arc<dyn IdentifiedIndexReader>,
    ) -> anyhow::Result<Self> {
        config.validate()?;
        Ok(Self {
            manifest,
            config,
            scorer,
            embedder,
            index_reader,
        })
    }
}

fn transient(label: &'static str) -> PolicyError {
    PolicyError::transient(label).expect("static label")
}

fn permanent(label: &'static str) -> PolicyError {
    PolicyError::permanent(label).expect("static label")
}

#[async_trait]
impl ScorePolicy for CompatibilityScorePolicy {
    async fn execute(&self, input: &ScoreInput) -> Result<ScoreOutput, PolicyError> {
        if input.reviewed_artifact.is_empty() {
            return Err(permanent("score_input_empty"));
        }
        let tenant = &input.tenant_storage_ref;
        let plan = chunk_envelope_plaintext(
            &input.reviewed_artifact,
            &ChunkerConfig {
                target_tokens: self.config.chunk_target_tokens as usize,
                max_tokens: self.config.chunk_max_tokens as usize,
                chunk_cap: self.config.chunk_cap as usize,
            },
        );
        let mut chunk_scores = Vec::with_capacity(plan.chunks.len());
        for chunk in &plan.chunks {
            chunk_scores.push(
                self.scorer
                    .score_chunk(chunk.text.as_bytes())
                    .map_err(|_| transient("scorer_unavailable"))?,
            );
        }
        let perplexity = aggregate_chunked_perplexity(
            &chunk_scores,
            self.config.chunk_min_tokens,
            self.config.perplexity_floor_micros,
        );
        let snapshot = self
            .index_reader
            .snapshot(tenant, &self.config.index_id)
            .map_err(|_| transient("index_unavailable"))?;
        let mut embeddings = Vec::with_capacity(plan.chunks.len());
        let mut chunk_novelty_micros = Vec::with_capacity(plan.chunks.len());
        let mut neighbors = Vec::new();
        for chunk in &plan.chunks {
            let embedding = embed_chunk_mean_pooled(self.embedder.as_ref(), &chunk.text)
                .map_err(|_| transient("embedder_unavailable"))?;
            let found = self
                .index_reader
                .nearest(
                    tenant,
                    &self.config.index_id,
                    &embedding,
                    self.config.top_k as usize,
                    Some(input.registry_revision_id),
                )
                .map_err(|_| transient("index_unavailable"))?;
            let max_similarity = found
                .iter()
                .map(|n| n.similarity)
                .fold(f32::NEG_INFINITY, f32::max);
            let novelty = if max_similarity.is_finite() {
                (1.0 - max_similarity).max(0.0)
            } else {
                1.0
            };
            chunk_novelty_micros.push((novelty.clamp(0.0, 2.0) * 1_000_000.0) as u64);
            neighbors.extend(found);
            embeddings.push(embedding);
        }
        let token_counts: Vec<u64> = chunk_scores.iter().map(|s| s.tokens).collect();
        let (novelty_score_micros, peak_novelty_micros) = aggregate_chunked_novelty(
            &chunk_novelty_micros,
            &token_counts,
            self.config.chunk_min_tokens,
        );
        let quality_passed = perplexity.representative_perplexity_micros
            >= self.config.perplexity_floor_micros
            && perplexity.tail_fraction_micros >= self.config.tail_fraction_floor_micros;
        let novelty_passed = novelty_score_micros >= self.config.novelty_floor_micros;
        let constants = constants_at(chrono::Utc::now().timestamp());
        let shadow = credit_quality(
            i64::try_from(perplexity.representative_perplexity_micros).unwrap_or(i64::MAX),
            i64::try_from(perplexity.peak_perplexity_micros).unwrap_or(i64::MAX),
            i64::try_from(novelty_score_micros).unwrap_or(i64::MAX),
            constants,
        );
        // P3-D7: membership ignores anomaly_withheld, as main does. It is
        // shadow-only credit-quality evidence, never a gate.
        let mut entries = Vec::new();
        if quality_passed && novelty_passed {
            for (index, (chunk, embedding)) in plan.chunks.iter().zip(embeddings).enumerate() {
                if chunk_novelty_micros[index] < self.config.embed_insert_novelty_micros {
                    continue;
                }
                entries.push(SealedIndexEntry {
                    // The chunk's ORIGINAL position in the trace, not its
                    // position among the surviving (post-cap) chunks: the
                    // stored command must hold exact original chunk numbers.
                    chunk: chunk.chunk_index,
                    content_hash: dependency_content_hash(chunk.text.as_bytes()),
                    embedding,
                });
            }
        }
        let command = if entries.is_empty() {
            None
        } else {
            Some(
                SealedIndexCommand::new(
                    &self.config.index_id,
                    input.registry_revision_id,
                    &self.config.projection_id,
                    self.embedder.model_id(),
                    entries,
                )
                .map_err(|_| permanent("score_output_invalid"))?,
            )
        };
        // P3-D6: the award is the configured NoveltyUtility delta, never q.
        let awards =
            if quality_passed && novelty_passed && self.config.novelty_utility_microcredits > 0 {
                InstrumentAwards::new(vec![
                    InstrumentAward::new(
                        InstrumentId::trace_credit(),
                        AtomicUnits::from_raw(u128::from(self.config.novelty_utility_microcredits)),
                    )
                    .map_err(|_| permanent("score_output_invalid"))?,
                ])
            } else {
                InstrumentAwards::new(Vec::new())
            }
            .map_err(|_| permanent("score_output_invalid"))?;
        let neighbor_bytes = serde_json::to_vec(
            &neighbors
                .iter()
                .map(|n| {
                    serde_json::json!({
                        "entry_id": n.entry_id,
                        "similarity_micros": (n.similarity.clamp(-1.0, 1.0) * 1_000_000.0).round() as i64,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .map_err(|_| permanent("score_output_invalid"))?;
        let mut evidence = ScoreEvidence::fixed(awards.clone());
        evidence.embedding_artifact_hash = command
            .as_ref()
            .map(SealedIndexCommand::content_hash)
            .transpose()
            .map_err(|_| permanent("score_output_invalid"))?;
        evidence.index_id = Some(self.config.index_id.clone());
        evidence.index_snapshot_id = Some(snapshot.snapshot_id);
        evidence.index_snapshot_hash = Some(snapshot.snapshot_hash);
        evidence.index_cardinality = Some(snapshot.cardinality);
        evidence.scorer_model_id = Some(self.config.scorer_model_id.clone());
        evidence.embedder_model_id = Some(self.embedder.model_id().to_string());
        evidence.projection_id = Some(self.config.projection_id.clone());
        evidence.projection_input_hash = Some(input.source_content_hash.clone());
        evidence.perplexity_micros = Some(perplexity.representative_perplexity_micros);
        evidence.tail_fraction_micros = Some(perplexity.tail_fraction_micros);
        evidence.novelty_score_micros = Some(novelty_score_micros);
        evidence.peak_perplexity_micros = Some(perplexity.peak_perplexity_micros);
        evidence.peak_novelty_micros = Some(peak_novelty_micros);
        evidence.quality_passed = Some(quality_passed);
        evidence.novelty_passed = Some(novelty_passed);
        evidence.nearest_neighbor_hash = Some(hash_neighbors(&neighbors));
        evidence.coverage_tokens = Some(perplexity.tokens_scored);
        let chunk_count =
            u32::try_from(plan.chunks.len()).map_err(|_| permanent("score_output_invalid"))?;
        evidence.chunk_count = Some(chunk_count);
        evidence.total_chunk_count = Some(chunk_count.saturating_add(plan.dropped_chunk_count));
        evidence.chunks_capped = Some(plan.chunks_capped);
        evidence.include_eligible = Some(command.is_some());
        evidence.credit_quality_micros = Some(u64::try_from(shadow.q_micros.max(0)).unwrap_or(0));
        evidence.credit_quality_version = Some(constants.version);
        evidence.neighbor_artifact_hash = Some(dependency_content_hash(&neighbor_bytes));
        let decision = ScoreDecision::for_bundle(&self.manifest, awards.clone())
            .map_err(|_| permanent("score_output_invalid"))?;
        let result = PhaseResult {
            decision,
            evidence,
            evaluation: ScoreEvaluation {
                rule_id: COMPATIBILITY_SCORE_RULE.to_string(),
                awards,
            },
        };
        ScoreOutput::new(result, command, Some(neighbor_bytes))
            .map_err(|_| permanent("score_output_invalid"))
    }
}

fn hash_neighbors(neighbors: &[NearestNeighbor]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"trace_commons.pipeline.neighbors.v1\n");
    for neighbor in neighbors {
        hasher.update(neighbor.entry_id.as_bytes());
        let similarity_micros = (neighbor.similarity.clamp(-1.0, 1.0) * 1_000_000.0).round() as i32;
        hasher.update(similarity_micros.to_be_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use trace_commons_gate_api::pipeline::{
        BUNDLE_MANIFEST_FORMAT_VERSION, PolicyRef, TenantStorageRef,
    };
    use trace_commons_gate_api::{
        ChunkPerplexity, Embedder, IndexSnapshot, PerplexityResult, PerplexityScorer,
        ReferenceEmbedder, ReferencePerplexityScorer, VectorIndexReader,
    };
    use uuid::Uuid;

    use crate::versioned_pipeline::pipeline_tenant_storage_ref;

    // ---- test doubles ----------------------------------------------------

    /// Records the `tenant_storage_ref` argument of every `snapshot` and
    /// `nearest` call (R5). Returns an empty index by default; a test may
    /// queue specific per-call neighbor responses (consumed FIFO) or set a
    /// fallback returned once the queue is drained. `failing()` makes every
    /// call return an error, for the `index_unavailable` dependency case.
    struct RecordingIndexReader {
        snapshot_args: Mutex<Vec<String>>,
        nearest_args: Mutex<Vec<String>>,
        neighbor_queue: Mutex<VecDeque<Vec<NearestNeighbor>>>,
        fallback_neighbors: Mutex<Vec<NearestNeighbor>>,
        fail: bool,
    }

    impl RecordingIndexReader {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                snapshot_args: Mutex::new(Vec::new()),
                nearest_args: Mutex::new(Vec::new()),
                neighbor_queue: Mutex::new(VecDeque::new()),
                fallback_neighbors: Mutex::new(Vec::new()),
                fail: false,
            })
        }

        fn failing() -> Arc<Self> {
            Arc::new(Self {
                snapshot_args: Mutex::new(Vec::new()),
                nearest_args: Mutex::new(Vec::new()),
                neighbor_queue: Mutex::new(VecDeque::new()),
                fallback_neighbors: Mutex::new(Vec::new()),
                fail: true,
            })
        }

        fn queue_neighbors(&self, neighbors: Vec<NearestNeighbor>) {
            self.neighbor_queue
                .lock()
                .expect("queue mutex")
                .push_back(neighbors);
        }

        fn set_fallback_neighbors(&self, neighbors: Vec<NearestNeighbor>) {
            *self.fallback_neighbors.lock().expect("fallback mutex") = neighbors;
        }

        fn recorded_tenant_args(&self) -> Vec<String> {
            let mut all = self.snapshot_args.lock().expect("snapshot mutex").clone();
            all.extend(self.nearest_args.lock().expect("nearest mutex").clone());
            all
        }
    }

    impl VectorIndexReader for RecordingIndexReader {
        fn snapshot(
            &self,
            tenant_storage_ref: &TenantStorageRef,
            _index_id: &str,
        ) -> anyhow::Result<IndexSnapshot> {
            self.snapshot_args
                .lock()
                .expect("snapshot mutex")
                .push(tenant_storage_ref.as_str().to_string());
            if self.fail {
                anyhow::bail!("index_unavailable_test_double");
            }
            Ok(IndexSnapshot {
                snapshot_id: "recording-index-reader-empty".to_string(),
                snapshot_hash: dependency_content_hash(b"recording-index-reader-empty"),
                cardinality: 0,
            })
        }

        fn nearest(
            &self,
            tenant_storage_ref: &TenantStorageRef,
            _index_id: &str,
            _embedding: &[f32],
            _k: usize,
            _exclude_revision: Option<Uuid>,
        ) -> anyhow::Result<Vec<NearestNeighbor>> {
            self.nearest_args
                .lock()
                .expect("nearest mutex")
                .push(tenant_storage_ref.as_str().to_string());
            if self.fail {
                anyhow::bail!("index_unavailable_test_double");
            }
            let queued = self.neighbor_queue.lock().expect("queue mutex").pop_front();
            Ok(queued.unwrap_or_else(|| {
                self.fallback_neighbors
                    .lock()
                    .expect("fallback mutex")
                    .clone()
            }))
        }
    }

    impl IdentifiedIndexReader for RecordingIndexReader {
        fn dependency_identity(&self) -> &str {
            "recording_index_reader_test_only"
        }
    }

    fn chunk_perplexity(tokens: u64, perplexity: f64, tail_fraction: f64) -> ChunkPerplexity {
        ChunkPerplexity {
            sum_nll: perplexity.ln() * tokens as f64,
            tokens,
            tail_tokens: (tail_fraction * tokens as f64).round() as u64,
            logprobs: Vec::new(),
            token_char_lens: Vec::new(),
        }
    }

    /// Returns chosen token counts and perplexities per chunk, consumed FIFO;
    /// once drained, repeats the last (or a default) value.
    struct FixedPerplexityScorer {
        queue: Mutex<VecDeque<ChunkPerplexity>>,
        fallback: ChunkPerplexity,
    }

    impl FixedPerplexityScorer {
        /// The same `(tokens, perplexity, tail_fraction)` for every chunk.
        fn uniform(tokens: u64, perplexity: f64, tail_fraction: f64) -> Self {
            Self {
                queue: Mutex::new(VecDeque::new()),
                fallback: chunk_perplexity(tokens, perplexity, tail_fraction),
            }
        }

        /// A distinct value per chunk, in call order.
        fn sequence(chunks: Vec<ChunkPerplexity>) -> Self {
            let fallback = chunks
                .last()
                .cloned()
                .unwrap_or_else(|| chunk_perplexity(64, 1.0, 0.0));
            Self {
                queue: Mutex::new(chunks.into()),
                fallback,
            }
        }
    }

    impl PerplexityScorer for FixedPerplexityScorer {
        fn score(&self, _plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
            // The compatibility policy calls only `score_chunk`; this exists
            // to satisfy the trait and is never exercised here.
            Ok(PerplexityResult {
                aggregate_perplexity_micros: 0,
                tail_fraction_micros: 0,
                tokens_scored: 0,
            })
        }

        fn score_chunk(&self, _chunk: &[u8]) -> anyhow::Result<ChunkPerplexity> {
            Ok(self
                .queue
                .lock()
                .expect("scorer mutex")
                .pop_front()
                .unwrap_or_else(|| self.fallback.clone()))
        }
    }

    impl IdentifiedPerplexityScorer for FixedPerplexityScorer {
        fn dependency_identity(&self) -> &str {
            "fixed_perplexity_scorer_test_only"
        }
        fn content_descriptor(&self) -> Vec<u8> {
            b"fixed-perplexity-scorer-test-only.v1".to_vec()
        }
    }

    /// Ported from `ef97a459` unchanged (moved into the test module): wraps
    /// the reference scorer with an `AtomicBool` fail switch, for the
    /// `scorer_unavailable` dependency case.
    struct TogglePerplexityScorer {
        inner: ReferencePerplexityScorer,
        fail: Arc<AtomicBool>,
    }

    impl TogglePerplexityScorer {
        fn new(fail: Arc<AtomicBool>) -> Self {
            Self {
                inner: ReferencePerplexityScorer::new(),
                fail,
            }
        }
    }

    impl PerplexityScorer for TogglePerplexityScorer {
        fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
            if self.fail.load(Ordering::SeqCst) {
                anyhow::bail!("scorer_unavailable_test_double");
            }
            self.inner.score(plaintext)
        }

        fn score_chunk(&self, chunk: &[u8]) -> anyhow::Result<ChunkPerplexity> {
            if self.fail.load(Ordering::SeqCst) {
                anyhow::bail!("scorer_unavailable_test_double");
            }
            self.inner.score_chunk(chunk)
        }
    }

    impl IdentifiedPerplexityScorer for TogglePerplexityScorer {
        fn dependency_identity(&self) -> &str {
            "toggle_perplexity_scorer_test_only"
        }
        fn content_descriptor(&self) -> Vec<u8> {
            b"toggle-perplexity-scorer-test-only.v1".to_vec()
        }
    }

    /// Always fails, for the `embedder_unavailable` dependency case.
    struct FailingEmbedder;

    impl Embedder for FailingEmbedder {
        fn embed(&self, _plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
            anyhow::bail!("embedder_unavailable_test_double")
        }
    }

    impl IdentifiedEmbedder for FailingEmbedder {
        fn dependency_identity(&self) -> &str {
            "failing_embedder_test_only"
        }
        fn model_id(&self) -> &str {
            "failing-embedder-v1"
        }
        fn content_descriptor(&self) -> Vec<u8> {
            b"failing-embedder-test-only.v1".to_vec()
        }
    }

    // ---- fixtures ----------------------------------------------------------

    fn score_input(bytes: &[u8]) -> ScoreInput {
        ScoreInput {
            run_id: Uuid::from_u128(1),
            tenant_storage_ref: pipeline_tenant_storage_ref("tenant-a"),
            trace_id: Uuid::from_u128(2),
            registry_revision_id: Uuid::from_u128(3),
            source_content_hash: dependency_content_hash(bytes),
            reviewed_artifact: bytes.to_vec(),
        }
    }

    fn dummy_policy_ref(phase: &str) -> PolicyRef {
        PolicyRef {
            policy_id: format!("trace_commons.{phase}.compatibility"),
            implementation_id: format!("trace_commons.{phase}.compatibility.v1"),
            configuration_hash: dependency_content_hash(phase.as_bytes()),
            data_artifact_hashes: Vec::new(),
            projection_ids: Vec::new(),
        }
    }

    /// A manifest that pins `trace_credit` to `config.instrument.descriptor`
    /// (Ruling S3), so `ScoreDecision::for_bundle` accepts this policy's
    /// awards.
    fn manifest_pinning_trace_credit(config: &CompatibilityBundleConfig) -> BundleManifest {
        let mut instruments = BTreeMap::new();
        instruments.insert(
            InstrumentId::trace_credit(),
            config.instrument.descriptor.clone(),
        );
        BundleManifest {
            format_version: BUNDLE_MANIFEST_FORMAT_VERSION,
            admission: dummy_policy_ref("admission"),
            review: dummy_policy_ref("review"),
            score: dummy_policy_ref("score"),
            settle: dummy_policy_ref("settle"),
            instruments,
        }
    }

    async fn run_score(
        config: &CompatibilityBundleConfig,
        scorer: FixedPerplexityScorer,
        index_reader: Arc<RecordingIndexReader>,
    ) -> ScoreOutput {
        let manifest = manifest_pinning_trace_credit(config);
        let policy = CompatibilityScorePolicy::new(
            manifest,
            config.clone(),
            Arc::new(scorer),
            Arc::new(ReferenceEmbedder::new()),
            index_reader,
        )
        .unwrap();
        policy
            .execute(&score_input(b"hello world, this is a compatibility trace"))
            .await
            .unwrap()
    }

    // ---- tests ---------------------------------------------------------

    #[tokio::test]
    async fn score_queries_the_reader_with_the_tenant_storage_ref() {
        let config = CompatibilityBundleConfig::local_reference();
        let manifest = manifest_pinning_trace_credit(&config);
        let reader = RecordingIndexReader::new();
        let policy = CompatibilityScorePolicy::new(
            manifest,
            config,
            Arc::new(ReferencePerplexityScorer::new()),
            Arc::new(ReferenceEmbedder::new()),
            reader.clone(),
        )
        .unwrap();
        let input = score_input(b"hello world, this is a compatibility trace");
        policy.execute(&input).await.unwrap();

        let recorded = reader.recorded_tenant_args();
        assert!(!recorded.is_empty());
        for arg in &recorded {
            assert_eq!(arg, input.tenant_storage_ref.as_str());
            assert_ne!(
                arg, "tenant-a",
                "the raw tenant id must never reach the reader"
            );
        }
    }

    #[tokio::test]
    async fn default_delta_makes_no_award() {
        let config = CompatibilityBundleConfig::local_reference();
        assert_eq!(config.novelty_utility_microcredits, 0);
        assert_eq!(config.perplexity_floor_micros, 0);
        assert_eq!(config.tail_fraction_floor_micros, 0);
        assert_eq!(config.novelty_floor_micros, 0);
        let manifest = manifest_pinning_trace_credit(&config);
        let policy = CompatibilityScorePolicy::new(
            manifest,
            config,
            Arc::new(ReferencePerplexityScorer::new()),
            Arc::new(ReferenceEmbedder::new()),
            RecordingIndexReader::new(),
        )
        .unwrap();
        let output = policy
            .execute(&score_input(b"hello world, this is a compatibility trace"))
            .await
            .unwrap();
        let result = output.result();
        assert!(result.evidence.quality_passed.unwrap());
        assert!(result.evidence.novelty_passed.unwrap());
        assert!(result.decision.awards().is_empty());
        assert!(result.evidence.fixed_awards.is_empty());
        assert!(result.evaluation.awards.is_empty());
    }

    #[tokio::test]
    async fn positive_delta_awards_trace_credit_only_when_both_floors_pass() {
        let mut config = CompatibilityBundleConfig::local_reference();
        config.novelty_utility_microcredits = 2_500_000;
        config.perplexity_floor_micros = 5_000_000; // 5.0
        config.tail_fraction_floor_micros = 0;
        config.novelty_floor_micros = 500_000; // 0.5
        config.chunk_min_tokens = 1;

        // Both floors pass: representative perplexity above 5.0, and the
        // reader's empty index gives every chunk novelty 1.0 (above 0.5).
        let passing = run_score(
            &config,
            FixedPerplexityScorer::uniform(100, 10.0, 0.0),
            RecordingIndexReader::new(),
        )
        .await;
        let awards = passing.result().decision.awards();
        assert_eq!(awards.iter().len(), 1);
        let award = awards.get(&InstrumentId::trace_credit()).unwrap();
        assert_eq!(award.atomic_units(), AtomicUnits::from_raw(2_500_000));

        // Perplexity floor fails: representative perplexity below 5.0.
        let perplexity_fails = run_score(
            &config,
            FixedPerplexityScorer::uniform(100, 1.0, 0.0),
            RecordingIndexReader::new(),
        )
        .await;
        assert!(perplexity_fails.result().decision.awards().is_empty());

        // Novelty floor fails: every chunk finds a near-identical neighbor
        // (similarity 0.9 -> novelty 0.1, below the 0.5 floor).
        let novelty_failing_reader = RecordingIndexReader::new();
        novelty_failing_reader.set_fallback_neighbors(vec![NearestNeighbor {
            entry_id: Uuid::nil(),
            similarity: 0.9,
        }]);
        let novelty_fails = run_score(
            &config,
            FixedPerplexityScorer::uniform(100, 10.0, 0.0),
            novelty_failing_reader,
        )
        .await;
        assert!(novelty_fails.result().decision.awards().is_empty());
    }

    #[tokio::test]
    async fn shadow_quality_uses_the_era_constants() {
        let mut config = CompatibilityBundleConfig::local_reference();
        config.novelty_utility_microcredits = 1_000_000;
        let now = chrono::Utc::now().timestamp();
        let expected_version = constants_at(now).version;

        let low_quality = run_score(
            &config,
            FixedPerplexityScorer::uniform(200, 2.0, 0.0),
            RecordingIndexReader::new(),
        )
        .await;
        let high_quality = run_score(
            &config,
            FixedPerplexityScorer::uniform(200, 40.0, 0.0),
            RecordingIndexReader::new(),
        )
        .await;

        for output in [&low_quality, &high_quality] {
            let evidence = &output.result().evidence;
            assert_eq!(evidence.credit_quality_version, Some(expected_version));
            assert!(evidence.credit_quality_micros.is_some());
        }
        assert_ne!(
            low_quality.result().evidence.credit_quality_micros,
            high_quality.result().evidence.credit_quality_micros,
            "the two scorers must actually differ in shadow quality for this test to mean anything"
        );
        assert_eq!(
            low_quality.result().decision.awards(),
            high_quality.result().decision.awards(),
            "the award is the configured delta, never a function of q"
        );
    }

    #[tokio::test]
    async fn anomaly_withheld_does_not_change_membership() {
        let mut config = CompatibilityBundleConfig::local_reference();
        config.chunk_target_tokens = 1;
        config.chunk_max_tokens = 1;
        config.chunk_min_tokens = 1;

        // Exactly two 4-char fixed-window chunks ("aaaa", "bbbb"): the input
        // length must match the scorer's two-item sequence exactly, or the
        // extra chunks would repeat the sequence's last (spike) value and
        // dilute the ratio this test depends on. Chunk 0: low perplexity,
        // most of the trace's tokens. Chunk 1: a tiny but enormous
        // perplexity spike, well past the V3 hard ratio (310) once weighed
        // against chunk 0's representative value.
        let scorer = FixedPerplexityScorer::sequence(vec![
            chunk_perplexity(6_400, 2.0, 0.0),
            chunk_perplexity(64, 700.0, 0.0),
        ]);
        let manifest = manifest_pinning_trace_credit(&config);
        let policy = CompatibilityScorePolicy::new(
            manifest,
            config.clone(),
            Arc::new(scorer),
            Arc::new(ReferenceEmbedder::new()),
            RecordingIndexReader::new(),
        )
        .unwrap();
        let output = policy.execute(&score_input(b"aaaabbbb")).await.unwrap();

        let evidence = &output.result().evidence;
        let now = chrono::Utc::now().timestamp();
        let shadow = credit_quality(
            evidence.perplexity_micros.unwrap() as i64,
            evidence.peak_perplexity_micros.unwrap() as i64,
            evidence.novelty_score_micros.unwrap() as i64,
            constants_at(now),
        );
        assert!(
            shadow.anomaly_withheld,
            "test setup must actually trigger the anomaly guard"
        );
        assert!(
            output.index_command().is_some(),
            "membership must ignore anomaly_withheld, as main does"
        );
    }

    #[tokio::test]
    async fn command_keeps_original_chunk_numbers_and_exact_embeddings() {
        let mut config = CompatibilityBundleConfig::local_reference();
        config.chunk_target_tokens = 1;
        config.chunk_max_tokens = 1;

        // Three 4-char fixed-window chunks: "aaaa" (chunk 0), "bbbb" (chunk
        // 1), "cccc" (chunk 2). Chunk 0 finds a near-identical neighbor
        // (novelty below the insert threshold); chunks 1 and 2 find none.
        let reader = RecordingIndexReader::new();
        reader.queue_neighbors(vec![NearestNeighbor {
            entry_id: Uuid::nil(),
            similarity: 1.0,
        }]);
        let embedder = Arc::new(ReferenceEmbedder::new());
        let manifest = manifest_pinning_trace_credit(&config);
        let policy = CompatibilityScorePolicy::new(
            manifest,
            config.clone(),
            Arc::new(FixedPerplexityScorer::uniform(100, 10.0, 0.0)),
            embedder.clone(),
            reader,
        )
        .unwrap();

        let output = policy.execute(&score_input(b"aaaabbbbcccc")).await.unwrap();
        let command = output
            .index_command()
            .expect("both floors pass by default under local_reference()");
        assert_eq!(
            command
                .entries()
                .iter()
                .map(|e| e.chunk)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        for entry in command.entries() {
            let chunk_text = match entry.chunk {
                1 => "bbbb",
                2 => "cccc",
                other => panic!("unexpected chunk {other}"),
            };
            let expected = embed_chunk_mean_pooled(embedder.as_ref(), chunk_text).unwrap();
            assert_eq!(entry.embedding, expected);
        }
        let evidence = &output.result().evidence;
        assert_eq!(
            evidence.embedding_artifact_hash.as_deref(),
            Some(command.content_hash().unwrap().as_str())
        );
        assert_eq!(evidence.total_chunk_count, Some(3));
        assert_eq!(evidence.chunks_capped, Some(false));
    }

    #[tokio::test]
    async fn dependency_failures_are_transient() {
        let config = CompatibilityBundleConfig::local_reference();

        // scorer_unavailable
        let fail = Arc::new(AtomicBool::new(true));
        let manifest = manifest_pinning_trace_credit(&config);
        let policy = CompatibilityScorePolicy::new(
            manifest,
            config.clone(),
            Arc::new(TogglePerplexityScorer::new(fail)),
            Arc::new(ReferenceEmbedder::new()),
            RecordingIndexReader::new(),
        )
        .unwrap();
        let error = policy
            .execute(&score_input(b"hello world"))
            .await
            .unwrap_err();
        assert!(error.is_transient());
        assert_eq!(error.label(), "scorer_unavailable");

        // embedder_unavailable
        let manifest = manifest_pinning_trace_credit(&config);
        let policy = CompatibilityScorePolicy::new(
            manifest,
            config.clone(),
            Arc::new(FixedPerplexityScorer::uniform(100, 10.0, 0.0)),
            Arc::new(FailingEmbedder),
            RecordingIndexReader::new(),
        )
        .unwrap();
        let error = policy
            .execute(&score_input(b"hello world"))
            .await
            .unwrap_err();
        assert!(error.is_transient());
        assert_eq!(error.label(), "embedder_unavailable");

        // index_unavailable
        let manifest = manifest_pinning_trace_credit(&config);
        let policy = CompatibilityScorePolicy::new(
            manifest,
            config,
            Arc::new(FixedPerplexityScorer::uniform(100, 10.0, 0.0)),
            Arc::new(ReferenceEmbedder::new()),
            RecordingIndexReader::failing(),
        )
        .unwrap();
        let error = policy
            .execute(&score_input(b"hello world"))
            .await
            .unwrap_err();
        assert!(error.is_transient());
        assert_eq!(error.label(), "index_unavailable");
    }
}
