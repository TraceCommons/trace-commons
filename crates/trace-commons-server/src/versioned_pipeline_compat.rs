//! Compatibility Score and Settle policies for the versioned pipeline.
//!
//! Score holds a read-only index. Settle uses committed Score evidence only.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use trace_commons_gate_api::pipeline::{
    IndexMembershipDecision, Microcredits, PhaseResult, PolicyError, ScoreDecision,
    ScoreEvaluation, ScoreEvidence, ScoreInput, ScorePolicy, SettleDecision, SettleEvaluation,
    SettleEvidence, SettleInput, SettlePolicy,
};
use trace_commons_gate_api::{
    Embedder, NearestNeighbor, OrchestrationDecision, PerplexityScorer, ReferenceEmbedder,
    ReferencePerplexityScorer, VectorIndexReader,
};
use trace_commons_gate_enclave::chunk_aggregate::{
    aggregate_chunked_novelty, aggregate_chunked_perplexity,
};
use trace_commons_gate_enclave::chunker::{ChunkerConfig, chunk_envelope_plaintext};
use trace_commons_gate_enclave::embedder::embed_chunk_mean_pooled;

use crate::credit_quality::{CREDIT_QUALITY_ACTIVE, CreditQualityScore, credit_quality};
use crate::versioned_pipeline_index::{
    IsolatedPipelineIndex, PIPELINE_INDEX_ID, PIPELINE_PROJECTION_ID,
};

pub const COMPATIBILITY_SCORE_IMPLEMENTATION: &str = "trace_commons.score.compatibility.v1";
pub const COMPATIBILITY_SETTLE_IMPLEMENTATION: &str = "trace_commons.settle.compatibility.v1";
pub const COMPATIBILITY_SCORE_CODE: &[u8] = b"compatibility-score-policy-v1";
pub const COMPATIBILITY_SETTLE_CODE: &[u8] = b"compatibility-settle-policy-v1";
pub const PIPELINE_SCORE_DEPENDENCY_LABEL: &str = "score_dependency_failed";
pub const COMPATIBILITY_SCORE_RULE: &str = "compatibility_quality_novelty_v1";
pub const COMPATIBILITY_SETTLE_RULE: &str = "compatibility_membership_from_score_v1";
pub const COMPATIBILITY_EXCLUDE_REASON: &str = "compatibility_not_eligible";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CompatibilityBundleConfig {
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
    pub credit_quality_version: i32,
}

impl CompatibilityBundleConfig {
    pub fn local_reference() -> Self {
        Self {
            scorer_model_id: "reference_perplexity.v1".to_string(),
            embedder_model_id: "reference_embedder.v1".to_string(),
            projection_id: PIPELINE_PROJECTION_ID.to_string(),
            index_id: PIPELINE_INDEX_ID.to_string(),
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
        }
    }
}

pub struct CompatibilityScoreRuntime {
    pub scorer: Arc<dyn PerplexityScorer>,
    pub embedder: Arc<dyn Embedder>,
    pub index: Arc<dyn VectorIndexReader>,
}

impl CompatibilityScoreRuntime {
    pub fn reference(index: Arc<dyn VectorIndexReader>) -> Self {
        Self {
            scorer: Arc::new(ReferencePerplexityScorer::new()),
            embedder: Arc::new(ReferenceEmbedder::new()),
            index,
        }
    }

    pub fn reference_unbound() -> Self {
        Self::reference(IsolatedPipelineIndex::new())
    }
}

pub struct TogglePerplexityScorer {
    inner: ReferencePerplexityScorer,
    fail: Arc<AtomicBool>,
}

impl TogglePerplexityScorer {
    pub fn new(fail: Arc<AtomicBool>) -> Self {
        Self {
            inner: ReferencePerplexityScorer::new(),
            fail,
        }
    }
}

impl PerplexityScorer for TogglePerplexityScorer {
    fn score(&self, plaintext: &[u8]) -> anyhow::Result<trace_commons_gate_api::PerplexityResult> {
        if self.fail.load(Ordering::SeqCst) {
            anyhow::bail!("scorer_unavailable");
        }
        self.inner.score(plaintext)
    }

    fn score_chunk(&self, chunk: &[u8]) -> anyhow::Result<trace_commons_gate_api::ChunkPerplexity> {
        if self.fail.load(Ordering::SeqCst) {
            anyhow::bail!("scorer_unavailable");
        }
        self.inner.score_chunk(chunk)
    }
}

pub struct CompatibilityScorePolicy {
    scorer: Arc<dyn PerplexityScorer>,
    embedder: Arc<dyn Embedder>,
    index: Arc<dyn VectorIndexReader>,
    config: CompatibilityBundleConfig,
}

impl CompatibilityScorePolicy {
    pub fn new(runtime: &CompatibilityScoreRuntime, config: CompatibilityBundleConfig) -> Self {
        Self {
            scorer: runtime.scorer.clone(),
            embedder: runtime.embedder.clone(),
            index: runtime.index.clone(),
            config,
        }
    }

    fn dependency_error() -> PolicyError {
        PolicyError::new(PIPELINE_SCORE_DEPENDENCY_LABEL).expect("static safe label")
    }
}

#[async_trait]
impl ScorePolicy for CompatibilityScorePolicy {
    async fn execute(
        &self,
        input: &ScoreInput,
    ) -> Result<PhaseResult<ScoreDecision, ScoreEvidence, ScoreEvaluation>, PolicyError> {
        if input.reviewed_artifact.is_empty() {
            return Err(Self::dependency_error());
        }
        let chunker_cfg = ChunkerConfig {
            target_tokens: self.config.chunk_target_tokens as usize,
            max_tokens: self.config.chunk_max_tokens as usize,
            chunk_cap: self.config.chunk_cap as usize,
        };
        let plan = chunk_envelope_plaintext(&input.reviewed_artifact, &chunker_cfg);
        let mut chunk_scores = Vec::with_capacity(plan.chunks.len());
        for chunk in &plan.chunks {
            let score = self
                .scorer
                .score_chunk(chunk.text.as_bytes())
                .map_err(|_| Self::dependency_error())?;
            chunk_scores.push(score);
        }
        let perp_agg = aggregate_chunked_perplexity(&chunk_scores, self.config.chunk_min_tokens);
        let snapshot = self
            .index
            .snapshot(&input.tenant_id, &self.config.index_id)
            .map_err(|_| Self::dependency_error())?;

        let mut chunk_embeddings = Vec::with_capacity(plan.chunks.len());
        let mut chunk_novelty_micros = Vec::with_capacity(plan.chunks.len());
        let mut all_neighbors: Vec<NearestNeighbor> = Vec::new();
        for chunk in &plan.chunks {
            let embedding = embed_chunk_mean_pooled(self.embedder.as_ref(), &chunk.text)
                .map_err(|_| Self::dependency_error())?;
            let neighbors = self
                .index
                .nearest(
                    &input.tenant_id,
                    &self.config.index_id,
                    &embedding,
                    self.config.top_k as usize,
                    Some(input.registry_revision_id),
                )
                .map_err(|_| Self::dependency_error())?;
            let max_sim = neighbors
                .iter()
                .map(|neighbor| neighbor.similarity)
                .fold(f32::NEG_INFINITY, f32::max);
            let novelty = if max_sim.is_finite() {
                (1.0 - max_sim).max(0.0)
            } else {
                1.0
            };
            chunk_novelty_micros.push((novelty.clamp(0.0, 2.0) * 1_000_000.0) as u64);
            all_neighbors.extend(neighbors);
            chunk_embeddings.push(embedding);
        }
        let chunk_token_counts: Vec<u64> = chunk_scores.iter().map(|chunk| chunk.tokens).collect();
        let (novelty_score_micros, peak_novelty_micros) = aggregate_chunked_novelty(
            &chunk_novelty_micros,
            &chunk_token_counts,
            self.config.chunk_min_tokens,
        );
        let quality_passed = perp_agg.representative_perplexity_micros
            >= self.config.perplexity_floor_micros
            && perp_agg.tail_fraction_micros >= self.config.tail_fraction_floor_micros;
        let novelty_passed = novelty_score_micros >= self.config.novelty_floor_micros;
        let quality = credit_quality(
            i64::try_from(perp_agg.representative_perplexity_micros).unwrap_or(i64::MAX),
            i64::try_from(perp_agg.peak_perplexity_micros).unwrap_or(i64::MAX),
            i64::try_from(novelty_score_micros).unwrap_or(i64::MAX),
            &CREDIT_QUALITY_ACTIVE,
        );
        let mut include_embeddings = Vec::new();
        if quality_passed && novelty_passed && !quality.anomaly_withheld {
            for (index, embedding) in chunk_embeddings.into_iter().enumerate() {
                if chunk_novelty_micros[index] < self.config.embed_insert_novelty_micros {
                    continue;
                }
                include_embeddings.push(embedding);
            }
        }
        let include_eligible = !include_embeddings.is_empty();
        let credit = mapped_credit_microcredits(&quality);
        let neighbor_bytes = serde_json::to_vec(&all_neighbors.iter().map(|neighbor| {
            serde_json::json!({
                "entry_id": neighbor.entry_id,
                "similarity_micros": (neighbor.similarity.clamp(-1.0, 1.0) * 1_000_000.0) as i64
            })
        }).collect::<Vec<_>>())
        .ok();
        Ok(PhaseResult {
            decision: ScoreDecision {
                credit_microcredits: Microcredits::from_raw(credit),
            },
            evidence: ScoreEvidence {
                fixed_credit_microcredits: Microcredits::from_raw(credit),
                embedding_artifact_hash: None,
                embedding_object_key: None,
                index_id: Some(self.config.index_id.clone()),
                index_snapshot_id: Some(snapshot.snapshot_id),
                index_snapshot_hash: Some(snapshot.snapshot_hash),
                scorer_model_id: Some(self.config.scorer_model_id.clone()),
                embedder_model_id: Some(self.config.embedder_model_id.clone()),
                projection_id: Some(self.config.projection_id.clone()),
                perplexity_micros: Some(perp_agg.representative_perplexity_micros),
                tail_fraction_micros: Some(perp_agg.tail_fraction_micros),
                novelty_score_micros: Some(novelty_score_micros),
                peak_perplexity_micros: Some(perp_agg.peak_perplexity_micros),
                peak_novelty_micros: Some(peak_novelty_micros),
                quality_passed: Some(quality_passed),
                novelty_passed: Some(novelty_passed),
                nearest_neighbor_hash: Some(hash_neighbors(&all_neighbors)),
                index_cardinality: Some(snapshot.cardinality),
                coverage_tokens: Some(perp_agg.tokens_scored),
                chunk_count: Some(plan.chunks.len() as u32),
                chunks_capped: Some(plan.chunks_capped),
                include_eligible: Some(include_eligible),
                credit_quality_micros: Some(quality.q_micros.max(0) as u64),
                credit_quality_version: Some(self.config.credit_quality_version),
                neighbor_artifact_hash: None,
                pending_embeddings: include_embeddings,
                pending_neighbor_bytes: neighbor_bytes,
            },
            evaluation: ScoreEvaluation {
                rule_id: COMPATIBILITY_SCORE_RULE.to_string(),
                credit_microcredits: Microcredits::from_raw(credit),
            },
        })
    }
}

pub struct CompatibilitySettlePolicy;

#[async_trait]
impl SettlePolicy for CompatibilitySettlePolicy {
    async fn execute(
        &self,
        input: &SettleInput,
    ) -> Result<PhaseResult<SettleDecision, SettleEvidence, SettleEvaluation>, PolicyError> {
        let credit_required = input.score.credit_microcredits != Microcredits::ZERO;
        let include = input.score_evidence.include_eligible.unwrap_or(false);
        let index_membership = if include {
            IndexMembershipDecision::Include {
                command_hash: String::new(),
                entry_count: input.score_evidence.chunk_count.unwrap_or(1).max(1),
            }
        } else {
            IndexMembershipDecision::Exclude {
                reason: trace_commons_gate_api::pipeline::ReasonCode::new(
                    COMPATIBILITY_EXCLUDE_REASON,
                )
                .expect("static safe label"),
            }
        };
        Ok(PhaseResult {
            decision: SettleDecision {
                index_membership,
                credit_microcredits_finalized: Microcredits::ZERO,
                settlement_batch_ref_hash: None,
            },
            evidence: SettleEvidence::operations(include, credit_required),
            evaluation: SettleEvaluation {
                rule_id: COMPATIBILITY_SETTLE_RULE.to_string(),
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityMappedResult {
    pub credit_microcredits: u64,
    pub include_eligible: bool,
    pub quality_passed: bool,
    pub novelty_passed: bool,
}

pub fn map_legacy_orchestration(
    decision: &OrchestrationDecision,
    quality: CreditQualityScore,
) -> CompatibilityMappedResult {
    CompatibilityMappedResult {
        credit_microcredits: mapped_credit_microcredits(&quality),
        include_eligible: decision.perplexity_passed
            && decision.novelty_passed
            && !quality.anomaly_withheld
            && !decision.inserted_chunk_entries.is_empty(),
        quality_passed: decision.perplexity_passed,
        novelty_passed: decision.novelty_passed,
    }
}

fn mapped_credit_microcredits(quality: &CreditQualityScore) -> u64 {
    if quality.anomaly_withheld {
        0
    } else {
        quality.q_micros.max(0) as u64
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
    use crate::versioned_pipeline_index::WriteDetectingIndex;
    use trace_commons_gate_api::VectorIndexWriter;
    use trace_commons_gate_api::pipeline::ReasonCode;
    use trace_commons_gate_enclave::{
        EnclaveGateOrchestrator, EnclaveGateOrchestratorConfig, MockVectorIndex,
    };
    use uuid::Uuid;

    fn score_input(bytes: &[u8]) -> ScoreInput {
        ScoreInput {
            run_id: Uuid::new_v4(),
            trace_id: Uuid::new_v4(),
            registry_revision_id: Uuid::from_u128(7),
            source_content_hash: format!("sha256:{:x}", Sha256::digest(bytes)),
            tenant_id: "tenant-a".to_string(),
            reviewed_artifact: bytes.to_vec(),
        }
    }

    #[tokio::test]
    async fn compatibility_score_queries_reader_without_writes() {
        let inner = IsolatedPipelineIndex::new();
        let detector = WriteDetectingIndex::wrap(inner.clone());
        let reader: Arc<dyn VectorIndexReader> = detector.clone();
        let policy = CompatibilityScorePolicy::new(
            &CompatibilityScoreRuntime::reference(reader),
            CompatibilityBundleConfig::local_reference(),
        );
        let result = policy.execute(&score_input(b"hello world")).await.unwrap();
        assert!(result.evidence.quality_passed.unwrap());
        assert!(result.evidence.novelty_passed.unwrap());
        assert!(result.evidence.include_eligible.unwrap());
        assert!(result.evidence.scorer_model_id.is_some());
        assert_eq!(detector.writes(), 0);
        assert_eq!(inner.writer_calls(), 0);
        assert_eq!(inner.entry_count("tenant-a", PIPELINE_INDEX_ID), 0);
    }

    #[tokio::test]
    async fn write_detector_records_settle_side_upserts_only() {
        let inner = IsolatedPipelineIndex::new();
        let detector = WriteDetectingIndex::wrap(inner);
        let reader: Arc<dyn VectorIndexReader> = detector.clone();
        let policy = CompatibilityScorePolicy::new(
            &CompatibilityScoreRuntime::reference(reader),
            CompatibilityBundleConfig::local_reference(),
        );
        let scored = policy.execute(&score_input(b"hello world")).await.unwrap();
        assert_eq!(detector.writes(), 0);
        let writer: Arc<dyn VectorIndexWriter> = detector.clone();
        let key = trace_commons_gate_api::IndexEntryKey {
            tenant_id: "tenant-a".to_string(),
            index_id: PIPELINE_INDEX_ID.to_string(),
            revision_id: Uuid::from_u128(7),
            projection_id: PIPELINE_PROJECTION_ID.to_string(),
            model_id: "reference_embedder.v1".to_string(),
            chunk: 0,
        };
        writer
            .upsert(
                &key,
                &scored.evidence.pending_embeddings[0],
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap();
        assert_eq!(detector.writes(), 1);
    }

    #[tokio::test]
    async fn score_dependency_failure_is_operational() {
        let fail = Arc::new(AtomicBool::new(true));
        let runtime = CompatibilityScoreRuntime {
            scorer: Arc::new(TogglePerplexityScorer::new(fail)),
            embedder: Arc::new(ReferenceEmbedder::new()),
            index: IsolatedPipelineIndex::new(),
        };
        let policy =
            CompatibilityScorePolicy::new(&runtime, CompatibilityBundleConfig::local_reference());
        let error = policy
            .execute(&score_input(b"hello world"))
            .await
            .unwrap_err();
        assert_eq!(error.label(), PIPELINE_SCORE_DEPENDENCY_LABEL);
    }

    #[tokio::test]
    async fn settle_uses_committed_include_flag_not_live_index() {
        let settle = CompatibilitySettlePolicy;
        let mut evidence = ScoreEvidence::fixed(Microcredits::from_raw(1_000_000));
        evidence.include_eligible = Some(true);
        let included = settle
            .execute(&SettleInput {
                run_id: Uuid::new_v4(),
                trace_id: Uuid::new_v4(),
                registry_revision_id: Uuid::nil(),
                source_content_hash:
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_string(),
                score: ScoreDecision {
                    credit_microcredits: Microcredits::from_raw(1_000_000),
                },
                score_evidence: evidence.clone(),
            })
            .await
            .unwrap();
        assert!(matches!(
            included.decision.index_membership,
            IndexMembershipDecision::Include { .. }
        ));
        evidence.include_eligible = Some(false);
        let excluded = settle
            .execute(&SettleInput {
                run_id: Uuid::new_v4(),
                trace_id: Uuid::new_v4(),
                registry_revision_id: Uuid::nil(),
                source_content_hash:
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_string(),
                score: ScoreDecision {
                    credit_microcredits: Microcredits::from_raw(1_000_000),
                },
                score_evidence: evidence,
            })
            .await
            .unwrap();
        match excluded.decision.index_membership {
            IndexMembershipDecision::Exclude { reason } => {
                assert_eq!(
                    reason,
                    ReasonCode::new(COMPATIBILITY_EXCLUDE_REASON).unwrap()
                );
            }
            IndexMembershipDecision::Include { .. } => {
                panic!("live index must not change membership")
            }
        }
    }

    #[tokio::test]
    async fn shadow_score_does_not_write_the_active_index() {
        let active = IsolatedPipelineIndex::new();
        let shadow = IsolatedPipelineIndex::new();
        let policy = CompatibilityScorePolicy::new(
            &CompatibilityScoreRuntime::reference(shadow.clone()),
            CompatibilityBundleConfig::local_reference(),
        );
        let _ = policy.execute(&score_input(b"hello world")).await.unwrap();
        assert_eq!(active.writer_calls(), 0);
        assert_eq!(shadow.writer_calls(), 0);
        assert_eq!(active.entry_count("tenant-a", PIPELINE_INDEX_ID), 0);
        assert_eq!(shadow.entry_count("tenant-a", PIPELINE_INDEX_ID), 0);
    }

    #[test]
    fn legacy_mapping_keeps_inserts_out_of_score() {
        let mut cfg = EnclaveGateOrchestratorConfig::mock_default();
        cfg.perplexity_floor_micros = 0;
        cfg.tail_fraction_floor_micros = 0;
        cfg.novelty_floor_micros = 0;
        let orchestrator = EnclaveGateOrchestrator::new(
            ReferencePerplexityScorer::new(),
            ReferenceEmbedder::new(),
            MockVectorIndex::new(),
            cfg,
        );
        let legacy = orchestrator.evaluate(b"hello world", "tenant-a").unwrap();
        assert!(legacy.inserted_entry_id.is_some());
        let quality = credit_quality(
            i64::try_from(legacy.perplexity_micros).unwrap_or(0),
            i64::try_from(legacy.peak_perplexity_micros).unwrap_or(0),
            i64::try_from(legacy.novelty_score_micros).unwrap_or(0),
            &CREDIT_QUALITY_ACTIVE,
        );
        let mapped = map_legacy_orchestration(&legacy, quality);
        assert_eq!(mapped.quality_passed, legacy.perplexity_passed);
        assert_eq!(mapped.novelty_passed, legacy.novelty_passed);
        assert_eq!(
            mapped.include_eligible,
            !legacy.inserted_chunk_entries.is_empty()
        );
    }
}
