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
//! The bundle constructor that binds these policies to a `BundlePackage`
//! lives in `versioned_pipeline_bundle.rs`
//! (`MinimalPolicyBundle::compatibility_package`).

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use trace_commons_gate_api::pipeline::{
    AtomicUnits, BundleManifest, IndexMembershipDecision, InstrumentAward, InstrumentAwards,
    InstrumentDescriptor, InstrumentId, InstrumentKind, MAX_TRACE_CREDIT_MICROCREDITS, PhaseResult,
    PolicyError, ReasonCode, ScoreDecision, ScoreEvaluation, ScoreEvidence, ScoreInput,
    ScoreOutput, ScorePolicy, SealedIndexCommand, SealedIndexEntry, SettleDecision,
    SettleEvaluation, SettleEvidence, SettleInput, SettlePolicy, TRACE_CREDIT_DECIMALS,
    TRACE_CREDIT_INSTRUMENT_ID, TenantStorageRef,
};
use trace_commons_gate_api::{
    IdentifiedEmbedder, IdentifiedIndexReader, IdentifiedPerplexityScorer, IndexSnapshot,
    NearestNeighbor, VectorIndexReader,
};
use trace_commons_gate_enclave::chunk_aggregate::{
    aggregate_chunked_novelty, aggregate_chunked_perplexity,
};
use trace_commons_gate_enclave::chunker::{ChunkerConfig, chunk_envelope_plaintext};
use trace_commons_gate_enclave::embedder::embed_chunk_mean_pooled;
use uuid::Uuid;

use crate::credit_quality::{constants_at, credit_quality};
use crate::versioned_pipeline_bundle::{
    MINIMAL_INDEX_ID, MINIMAL_PROJECTION_ID, PipelineInstrumentAwardConfig,
    dependency_content_hash, settlement_operations,
};

/// Implementation id this policy family answers to in a bundle manifest
/// (P3-D5). Admission and Review are pass-through for the compatibility
/// family, same as the minimal family, but under their own implementation
/// identity so a package cannot silently mix families.
pub const COMPATIBILITY_ADMISSION_IMPLEMENTATION: &str = "trace_commons.admission.compatibility.v1";
pub const COMPATIBILITY_REVIEW_IMPLEMENTATION: &str = "trace_commons.review.compatibility.v1";
pub const COMPATIBILITY_SCORE_IMPLEMENTATION: &str = "trace_commons.score.compatibility.v1";
pub const COMPATIBILITY_SETTLE_IMPLEMENTATION: &str = "trace_commons.settle.compatibility.v1";
pub const COMPATIBILITY_SCORE_RULE: &str = "compatibility_quality_novelty_v1";
pub const COMPATIBILITY_SETTLE_RULE: &str = "compatibility_membership_from_score_v1";
pub const COMPATIBILITY_EXCLUDE_REASON: &str = "compatibility_not_eligible";
pub const COMPATIBILITY_ZERO_FLOOR_LABEL: &str = "compatibility_zero_floor";
/// Safe label for a `novelty_utility_microcredits` delta above the trace
/// credit ledger's bound.
pub const COMPATIBILITY_DELTA_OUT_OF_RANGE_LABEL: &str = "compatibility_delta_out_of_range";

/// Safe label for a production-compatible configuration built from a
/// `main` gate configuration that has no floors: `main` reads them only on
/// its production gate path, and a compatibility bundle cannot claim to hold
/// values `main` does not have.
pub const COMPATIBILITY_MAIN_GATE_FLOORS_MISSING_LABEL: &str =
    "compatibility_main_gate_floors_missing";

/// `main`'s gate configuration, as ingest parses it, that a compatibility
/// bundle must hold (multi-lens review L5-4, Zaki review 3, Z3-3): the three
/// floors (`TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS`,
/// `..._TAIL_FRACTION_FLOOR_MICROS`, `..._NOVELTY_FLOOR_MICROS`), the
/// index-insert threshold and the four chunk knobs
/// (`parse_gate_chunking_config_from_env`), top-k
/// (`TRACE_COMMONS_GATE_TOP_K`), and the `NoveltyUtility` delta
/// (`TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA`, in microcredits). A
/// floor `main` has no value for is `None`, and no compatibility
/// configuration matches it. Ingest hands it to the assembly and refuses a
/// runtime whose compatibility bundle holds anything else
/// ([`CompatibilityBundleConfig::matches_main_gate`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MainGateConfig {
    pub perplexity_floor_micros: Option<u64>,
    pub tail_fraction_floor_micros: Option<u64>,
    pub novelty_floor_micros: Option<u64>,
    pub embed_insert_novelty_micros: u64,
    pub top_k: u32,
    pub chunk_target_tokens: u32,
    pub chunk_max_tokens: u32,
    pub chunk_cap: u32,
    pub chunk_min_tokens: u64,
    pub novelty_utility_microcredits: u64,
}

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
            novelty_utility_microcredits: 0,
            instrument: default_trace_credit_instrument(),
        }
    }

    /// A production-compatible configuration with `main`'s gate settings,
    /// every one taken from `gate`, the configuration ingest hands the
    /// assembly (multi-lens review L5-4, Zaki review 3, Z3-3): the three
    /// floors, `main`'s own index-insert threshold (never the novelty floor;
    /// Zaki review 1, round 2, finding 14), top-k, the four chunk knobs, and
    /// the `NoveltyUtility` delta. `main` without floors refuses
    /// (`compatibility_main_gate_floors_missing`).
    pub fn production_compatible(
        scorer_model_id: String,
        projection_id: String,
        index_id: String,
        gate: &MainGateConfig,
    ) -> anyhow::Result<Self> {
        let (
            Some(perplexity_floor_micros),
            Some(tail_fraction_floor_micros),
            Some(novelty_floor_micros),
        ) = (
            gate.perplexity_floor_micros,
            gate.tail_fraction_floor_micros,
            gate.novelty_floor_micros,
        )
        else {
            anyhow::bail!(COMPATIBILITY_MAIN_GATE_FLOORS_MISSING_LABEL);
        };
        let config = Self {
            qualification: CompatibilityQualification::ProductionCompatible,
            scorer_model_id,
            projection_id,
            index_id,
            perplexity_floor_micros,
            tail_fraction_floor_micros,
            novelty_floor_micros,
            embed_insert_novelty_micros: gate.embed_insert_novelty_micros,
            top_k: gate.top_k,
            chunk_target_tokens: gate.chunk_target_tokens,
            chunk_max_tokens: gate.chunk_max_tokens,
            chunk_cap: gate.chunk_cap,
            chunk_min_tokens: gate.chunk_min_tokens,
            novelty_utility_microcredits: gate.novelty_utility_microcredits,
            instrument: default_trace_credit_instrument(),
        };
        config.validate()?;
        Ok(config)
    }

    /// The `main` gate configuration this configuration holds: the inverse
    /// of [`Self::production_compatible`], every floor present. The
    /// qualification harness reads a production package's gate settings
    /// back through it (spec 2026-10-08, B-D1).
    pub fn main_gate(&self) -> MainGateConfig {
        MainGateConfig {
            perplexity_floor_micros: Some(self.perplexity_floor_micros),
            tail_fraction_floor_micros: Some(self.tail_fraction_floor_micros),
            novelty_floor_micros: Some(self.novelty_floor_micros),
            embed_insert_novelty_micros: self.embed_insert_novelty_micros,
            top_k: self.top_k,
            chunk_target_tokens: self.chunk_target_tokens,
            chunk_max_tokens: self.chunk_max_tokens,
            chunk_cap: self.chunk_cap,
            chunk_min_tokens: self.chunk_min_tokens,
            novelty_utility_microcredits: self.novelty_utility_microcredits,
        }
    }

    /// Whether this configuration holds every value of `main`'s gate
    /// configuration `gate`. A floor `main` has no value for matches nothing.
    pub fn matches_main_gate(&self, gate: &MainGateConfig) -> bool {
        gate.perplexity_floor_micros == Some(self.perplexity_floor_micros)
            && gate.tail_fraction_floor_micros == Some(self.tail_fraction_floor_micros)
            && gate.novelty_floor_micros == Some(self.novelty_floor_micros)
            && gate.embed_insert_novelty_micros == self.embed_insert_novelty_micros
            && gate.top_k == self.top_k
            && gate.chunk_target_tokens == self.chunk_target_tokens
            && gate.chunk_max_tokens == self.chunk_max_tokens
            && gate.chunk_cap == self.chunk_cap
            && gate.chunk_min_tokens == self.chunk_min_tokens
            && gate.novelty_utility_microcredits == self.novelty_utility_microcredits
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.scorer_model_id.trim().is_empty()
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
        // Zaki review 1, round 2, finding 11: `main`'s gate refuses to start
        // only when every floor is zero (every trace would pass), and runs
        // with any one floor positive -- the pilot template
        // (`deploy/pilot-gcp/ingest.env.template`) sets 0, 0 and 500000; a
        // production-compatible configuration is validated the same way.
        if self.qualification == CompatibilityQualification::ProductionCompatible {
            anyhow::ensure!(
                self.perplexity_floor_micros > 0
                    || self.tail_fraction_floor_micros > 0
                    || self.novelty_floor_micros > 0,
                COMPATIBILITY_ZERO_FLOOR_LABEL
            );
        }
        anyhow::ensure!(
            self.novelty_utility_microcredits <= MAX_TRACE_CREDIT_MICROCREDITS,
            COMPATIBILITY_DELTA_OUT_OF_RANGE_LABEL
        );
        Ok(())
    }

    /// Whether a runtime that routes or drains tenants may bind this
    /// configuration: it is production-compatible and valid. The runtime's
    /// qualification gate and `qualify_bundle` read it through the bundle
    /// qualification's configuration term
    /// (`versioned_pipeline_bundle::package_configuration_is_qualifiable`,
    /// `PipelineBundleQualification::configuration_qualifiable`), so the
    /// local reference configuration, whose floors are all zero, never binds
    /// there.
    pub fn is_qualifiable(&self) -> bool {
        self.qualification == CompatibilityQualification::ProductionCompatible
            && self.validate().is_ok()
    }
}

/// Reproduces `main`'s gate path as a read-only Score policy: chunked
/// perplexity and novelty aggregation, the configured flat `NoveltyUtility`
/// credit delta, shadow credit-quality evidence, and a stored index command.
/// Holds its dependencies only as trait objects (never a concrete scorer,
/// embedder, or index type), and holds the bundle manifest it was
/// constructed under so every decision it builds is proven pinned
/// (`ScoreDecision::for_bundle`).
#[derive(Clone)]
pub struct CompatibilityScorePolicy {
    manifest: BundleManifest,
    config: CompatibilityBundleConfig,
    scorer: Arc<dyn IdentifiedPerplexityScorer>,
    embedder: Arc<dyn IdentifiedEmbedder>,
    index_reader: Arc<dyn IdentifiedIndexReader>,
    /// The Score start time, read once per `execute` call and handed to
    /// [`constants_at`]. Defaults to the real clock; a test may substitute a
    /// fixed instant with [`Self::with_clock`], which production code cannot
    /// reach (it is `#[cfg(test)]`).
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
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
            clock: Arc::new(|| chrono::Utc::now().timestamp()),
        })
    }

    /// Test-only: pins the Score start time `execute` reads, so a test can
    /// exercise a specific credit-quality calibration era deterministically.
    #[cfg(test)]
    fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }
}

fn transient(label: &'static str) -> PolicyError {
    PolicyError::transient(label).expect("static label")
}

fn permanent(label: &'static str) -> PolicyError {
    PolicyError::permanent(label).expect("static label")
}

/// A generic error-mapping function, not a closure: `Settle::execute` maps
/// two different error types (`ContractError` and `TryFromIntError`) to the
/// same permanent label, and a plain closure is monomorphic over its one
/// inferred parameter type, so one closure cannot serve both call sites (as
/// `versioned_pipeline_bundle.rs`'s `settle_invalid`/`score_invalid` already
/// do it this way).
fn settle_invalid<E>(_: E) -> PolicyError {
    permanent("settle_output_invalid")
}

#[async_trait]
impl ScorePolicy for CompatibilityScorePolicy {
    /// The scorer, the embedder and the index reader are synchronous, and a
    /// production scorer makes network calls (the NEAR AI scorer uses a
    /// blocking HTTP client), so the whole evaluation runs on the blocking
    /// pool, never on a runtime worker, as `main` runs the same scorer class
    /// (Zaki review 1, round 2, N-6). A task that panicked or was cancelled
    /// is `score_task_failed`.
    async fn execute(&self, input: &ScoreInput) -> Result<ScoreOutput, PolicyError> {
        let policy = self.clone();
        let input = input.clone();
        tokio::task::spawn_blocking(move || policy.evaluate(&input))
            .await
            .map_err(|_| permanent("score_task_failed"))?
    }
}

impl CompatibilityScorePolicy {
    /// The evaluation `execute` runs on the blocking pool.
    fn evaluate(&self, input: &ScoreInput) -> Result<ScoreOutput, PolicyError> {
        let now = (self.clock)();
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
        let constants = constants_at(now);
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

/// The index a compatibility Score reads (Zaki review 1, round 2, finding
/// 12, and the owner ruling of 2026-09-30): the live index, together with
/// the entries of the index commands that the tenant's other runs committed
/// at Score and that Settle has not applied yet. Score is serialized per
/// tenant and builds this reader under the tenant's Score lock
/// (`PipelineService::commit_score_phase`), so of several near-duplicate
/// traces received together the first is novel and each later one finds the
/// first one's entries here: it earns no award and proposes no index
/// command, as on `main`, whose gate inserts a trace's entries before it
/// scores the next one.
///
/// It holds each unapplied entry under the entry id Settle will write it
/// under (`IndexEntryKey::entry_id`), with its exact embedding. `nearest`
/// measures an unapplied entry by cosine similarity, as `NearestNeighbor`
/// defines it, and does not count twice an entry the live index already
/// holds (a Settle dispatch that applied part of its command). `snapshot`
/// keeps the live index's generation (`snapshot_id`) and counts the
/// unapplied entries into the cardinality and the hash, so the Score
/// evidence describes the entries the neighbour read compared against (an
/// entry that a partly applied write already put in the live index counts
/// twice there).
pub(crate) struct UnappliedIndexCommandsReader {
    live: Arc<dyn IdentifiedIndexReader>,
    tenant_storage_ref: TenantStorageRef,
    entries: Vec<UnappliedIndexEntry>,
}

struct UnappliedIndexEntry {
    index_id: String,
    revision_id: Uuid,
    entry_id: Uuid,
    embedding: Vec<f32>,
}

impl UnappliedIndexCommandsReader {
    /// `commands` are the unapplied index commands of `tenant_storage_ref`'s
    /// other runs; `live` is the index Settle writes.
    pub(crate) fn new(
        live: Arc<dyn IdentifiedIndexReader>,
        tenant_storage_ref: TenantStorageRef,
        commands: &[SealedIndexCommand],
    ) -> Self {
        let entries = commands
            .iter()
            .flat_map(|command| {
                command
                    .keyed_entries(&tenant_storage_ref)
                    .map(|(key, entry)| UnappliedIndexEntry {
                        entry_id: key.entry_id(),
                        index_id: key.index_id,
                        revision_id: key.revision_id,
                        embedding: entry.embedding.clone(),
                    })
            })
            .collect();
        Self {
            live,
            tenant_storage_ref,
            entries,
        }
    }

    fn entries_for<'a>(
        &'a self,
        tenant_storage_ref: &'a TenantStorageRef,
        index_id: &'a str,
    ) -> impl Iterator<Item = &'a UnappliedIndexEntry> + 'a {
        let tenant_matches = *tenant_storage_ref == self.tenant_storage_ref;
        self.entries
            .iter()
            .filter(move |entry| tenant_matches && entry.index_id == index_id)
    }
}

/// Cosine similarity in `[-1.0, 1.0]`; `0.0` when either vector is zero.
fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let (mut dot, mut left_norm, mut right_norm) = (0.0_f64, 0.0_f64, 0.0_f64);
    for (a, b) in left.iter().zip(right) {
        let (a, b) = (f64::from(*a), f64::from(*b));
        dot += a * b;
        left_norm += a * a;
        right_norm += b * b;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return 0.0;
    }
    (dot / (left_norm.sqrt() * right_norm.sqrt())).clamp(-1.0, 1.0) as f32
}

impl VectorIndexReader for UnappliedIndexCommandsReader {
    fn snapshot(
        &self,
        tenant_storage_ref: &TenantStorageRef,
        index_id: &str,
    ) -> anyhow::Result<IndexSnapshot> {
        let live = self.live.snapshot(tenant_storage_ref, index_id)?;
        let unapplied = self
            .entries_for(tenant_storage_ref, index_id)
            .collect::<Vec<_>>();
        if unapplied.is_empty() {
            return Ok(live);
        }
        let mut hasher = Sha256::new();
        hasher.update(b"trace_commons.pipeline.unapplied_index_snapshot.v1\n");
        hasher.update(live.snapshot_hash.as_bytes());
        for entry in &unapplied {
            hasher.update(entry.entry_id.as_bytes());
        }
        Ok(IndexSnapshot {
            snapshot_id: live.snapshot_id,
            snapshot_hash: format!("sha256:{:x}", hasher.finalize()),
            cardinality: live
                .cardinality
                .saturating_add(u64::try_from(unapplied.len()).unwrap_or(u64::MAX)),
        })
    }

    fn nearest(
        &self,
        tenant_storage_ref: &TenantStorageRef,
        index_id: &str,
        embedding: &[f32],
        k: usize,
        exclude_revision: Option<Uuid>,
    ) -> anyhow::Result<Vec<NearestNeighbor>> {
        let mut neighbors =
            self.live
                .nearest(tenant_storage_ref, index_id, embedding, k, exclude_revision)?;
        let live_ids = neighbors
            .iter()
            .map(|neighbor| neighbor.entry_id)
            .collect::<std::collections::BTreeSet<_>>();
        neighbors.extend(
            self.entries_for(tenant_storage_ref, index_id)
                .filter(|entry| {
                    entry.embedding.len() == embedding.len()
                        && exclude_revision != Some(entry.revision_id)
                        && !live_ids.contains(&entry.entry_id)
                })
                .map(|entry| NearestNeighbor {
                    entry_id: entry.entry_id,
                    similarity: cosine_similarity(&entry.embedding, embedding),
                }),
        );
        neighbors.sort_by(|left, right| {
            right
                .similarity
                .partial_cmp(&left.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        neighbors.truncate(k);
        Ok(neighbors)
    }
}

impl IdentifiedIndexReader for UnappliedIndexCommandsReader {
    fn dependency_identity(&self) -> &str {
        self.live.dependency_identity()
    }

    fn production_qualified(&self) -> bool {
        self.live.production_qualified()
    }
}

/// Applies the stored Score command from committed evidence only -- it never
/// re-queries the live index (the whole point of a shadow Score policy).
/// Membership is `Include` only when Score actually stored a command *and*
/// its own evidence says the run was eligible; either alone is not enough
/// (a command with a since-changed `include_eligible` flag, or a flag with
/// no command to back it, both fail closed to `Exclude`).
pub struct CompatibilitySettlePolicy;

#[async_trait]
impl SettlePolicy for CompatibilitySettlePolicy {
    async fn execute(
        &self,
        input: &SettleInput,
    ) -> Result<PhaseResult<SettleDecision, SettleEvidence, SettleEvaluation>, PolicyError> {
        let include =
            input.index_command.is_some() && input.score_evidence.include_eligible == Some(true);
        let membership = match (&input.index_command, include) {
            (Some(command), true) => IndexMembershipDecision::Include {
                command_hash: command.content_hash().map_err(settle_invalid)?,
                entry_count: u32::try_from(command.entries().len()).map_err(settle_invalid)?,
            },
            _ => IndexMembershipDecision::Exclude {
                reason: ReasonCode::new(COMPATIBILITY_EXCLUDE_REASON).expect("static label"),
            },
        };
        let operations =
            settlement_operations(input.run_id, input.score.awards()).map_err(settle_invalid)?;
        let decision =
            SettleDecision::new(membership, &input.score, operations).map_err(settle_invalid)?;
        Ok(PhaseResult {
            decision,
            evidence: SettleEvidence::operations(
                include,
                u32::try_from(input.score.awards().iter().len()).map_err(settle_invalid)?,
            ),
            evaluation: SettleEvaluation {
                rule_id: COMPATIBILITY_SETTLE_RULE.to_string(),
            },
        })
    }
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
        ReferenceEmbedder, ReferencePerplexityScorer, VectorIndexReader, VectorIndexWriter,
    };
    use uuid::Uuid;

    use crate::credit_quality::{CREDIT_QUALITY_ACTIVE, QWEN3_8_EFFECTIVE_FROM_UNIX};
    use crate::versioned_pipeline::pipeline_tenant_storage_ref;
    use crate::versioned_pipeline_index::IsolatedPipelineIndex;

    // ---- the compatibility baseline comparison ---------------------------
    //
    // Test-only: it reads the pinned baseline fixture and the corpus file it
    // names from this checkout (Ruling F-M3), so it is never part of a
    // release binary.

    /// One documented fixture's expected outcome at each phase, as the baseline
    /// fixture states it (`expected_fixture_classes`).
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct CompatibilityFixtureDecision {
        pub admission: String,
        pub review: String,
        pub score: String,
        pub settle: String,
    }

    /// What this bundle's `local_reference()` configuration is observed to
    /// produce, checked against the current baseline fixture
    /// (`docs/superpowers/specs/versioned-pipeline-compatibility-baseline-v1.json`)
    /// by [`compare_compatibility_baseline`]. Holds only the fields the
    /// comparison actually reads from that fixture (Ruling T4-1): the corpus
    /// order and its declared starting index state, three of the fixture's
    /// `configuration_identities` labels, and the per-fixture expected outcomes.
    ///
    /// `configuration_identities.embedder_model_id` is deliberately not a field
    /// here: see the comment on [`compare_compatibility_baseline`]'s own
    /// `embedder_model_id` handling (Rulings T4-3/T4-4).
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct CompatibilityBaselineObservation {
        pub fixture_order: Vec<String>,
        pub initial_index: String,
        pub scorer_model_id: String,
        pub projection_id: String,
        pub index_id: String,
        pub gate_floors: String,
        pub fixture_classes: BTreeMap<String, CompatibilityFixtureDecision>,
    }

    /// Deserializes only the parts of the baseline fixture the comparison reads.
    /// `embedder_model_id` is intentionally absent -- see
    /// [`compare_compatibility_baseline`].
    #[derive(Deserialize)]
    struct CompatibilityBaselineDocument {
        corpus: CompatibilityBaselineCorpus,
        configuration_identities: CompatibilityBaselineIdentities,
        expected_fixture_classes: BTreeMap<String, CompatibilityFixtureDecision>,
    }

    #[derive(Deserialize)]
    struct CompatibilityBaselineCorpus {
        /// Relative to the repository root, as the fixture states it. Read at
        /// runtime (not `include_str!`) so the comparison actually reads "the
        /// corpus file it names" rather than a second hardcoded path that could
        /// silently drift from this one.
        path: String,
        initial_index: String,
    }

    #[derive(Deserialize)]
    struct CompatibilityBaselineIdentities {
        scorer_model_id: String,
        projection_id: String,
        index_id: String,
        gate_floors: String,
    }

    #[derive(Deserialize)]
    struct CompatibilityCorpusDocument {
        fixtures: Vec<CompatibilityCorpusFixture>,
    }

    #[derive(Deserialize)]
    struct CompatibilityCorpusFixture {
        label: String,
    }

    /// Compares `observation` against the current compatibility baseline
    /// fixture and the corpus file it names, at their present (R1-corrected)
    /// shape: `corpus.path`/`corpus.order`/`corpus.initial_index`, a handful of
    /// `configuration_identities` labels, and `expected_fixture_classes`. Never
    /// reads or writes either file except by loading them (Ruling T4-1); the
    /// fixture and corpus are read-only inputs pinned under `docs/superpowers/specs`.
    pub fn compare_compatibility_baseline(
        observation: &CompatibilityBaselineObservation,
    ) -> anyhow::Result<()> {
        let baseline: CompatibilityBaselineDocument = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/superpowers/specs/versioned-pipeline-compatibility-baseline-v1.json"
        )))?;
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let corpus_path = manifest_dir.join("../..").join(&baseline.corpus.path);
        let corpus_bytes = std::fs::read_to_string(&corpus_path)
            .map_err(|_| anyhow::anyhow!("compatibility corpus fixture unreadable"))?;
        let corpus: CompatibilityCorpusDocument = serde_json::from_str(&corpus_bytes)?;
        let fixture_order = corpus
            .fixtures
            .into_iter()
            .map(|fixture| fixture.label)
            .collect::<Vec<_>>();
        // T4-3/T4-4: `configuration_identities.embedder_model_id` in the fixture
        // is "reference_embedder.v1", but `ReferenceEmbedder::model_id()`
        // reports "reference-embedder-v1" (underscore vs. hyphen) -- a contract
        // mismatch tracked as a request to PR 1 (#971). This comparison does not
        // check `embedder_model_id` against the running embedder until that
        // lands.
        let expected = CompatibilityBaselineObservation {
            fixture_order,
            initial_index: baseline.corpus.initial_index,
            scorer_model_id: baseline.configuration_identities.scorer_model_id,
            projection_id: baseline.configuration_identities.projection_id,
            index_id: baseline.configuration_identities.index_id,
            gate_floors: baseline.configuration_identities.gate_floors,
            fixture_classes: baseline.expected_fixture_classes,
        };
        anyhow::ensure!(observation == &expected, "compatibility baseline mismatch");
        Ok(())
    }

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

    /// The reference scorer, failing every call made on a Tokio runtime
    /// worker (N-6).
    struct BlockingPoolOnlyScorer(ReferencePerplexityScorer);

    impl PerplexityScorer for BlockingPoolOnlyScorer {
        fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
            anyhow::ensure!(
                !crate::versioned_pipeline_index::called_on_a_runtime_worker(),
                "called on a runtime worker"
            );
            self.0.score(plaintext)
        }

        fn score_chunk(&self, chunk: &[u8]) -> anyhow::Result<ChunkPerplexity> {
            anyhow::ensure!(
                !crate::versioned_pipeline_index::called_on_a_runtime_worker(),
                "called on a runtime worker"
            );
            self.0.score_chunk(chunk)
        }
    }

    impl IdentifiedPerplexityScorer for BlockingPoolOnlyScorer {
        fn dependency_identity(&self) -> &str {
            "blocking_pool_only_scorer_test_only"
        }
        fn content_descriptor(&self) -> Vec<u8> {
            b"blocking-pool-only-scorer-test-only.v1".to_vec()
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

    /// As [`run_score`], but pins the Score start time to `at` instead of
    /// the real clock.
    async fn run_score_at(
        config: &CompatibilityBundleConfig,
        scorer: FixedPerplexityScorer,
        index_reader: Arc<RecordingIndexReader>,
        at: i64,
    ) -> ScoreOutput {
        let manifest = manifest_pinning_trace_credit(config);
        let policy = CompatibilityScorePolicy::new(
            manifest,
            config.clone(),
            Arc::new(scorer),
            Arc::new(ReferenceEmbedder::new()),
            index_reader,
        )
        .unwrap()
        .with_clock(move || at);
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
        // Pin the Score start time to before the Qwen3.8 switch, so this
        // test exercises a non-active era: `constants_at(now)`,
        // `CREDIT_QUALITY_ACTIVE`, and `config.credit_quality_version` (now
        // removed) all agreed on V3 "today", which let each of them pass
        // silently even if `execute` used the wrong one.
        let before_qwen3_8 = QWEN3_8_EFFECTIVE_FROM_UNIX - 1;
        let expected_version = constants_at(before_qwen3_8).version;
        assert_ne!(
            expected_version, CREDIT_QUALITY_ACTIVE.version,
            "test setup must exercise a non-active era, or it can't catch execute() reading the wrong constants"
        );

        let low_quality = run_score_at(
            &config,
            FixedPerplexityScorer::uniform(200, 2.0, 0.0),
            RecordingIndexReader::new(),
            before_qwen3_8,
        )
        .await;
        let high_quality = run_score_at(
            &config,
            FixedPerplexityScorer::uniform(200, 40.0, 0.0),
            RecordingIndexReader::new(),
            before_qwen3_8,
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
        // The config no longer carries an `embedder_model_id` (T4-3): the
        // evidence and the stored command both take the model id straight
        // from the embedder dependency itself.
        assert_eq!(
            evidence.embedder_model_id.as_deref(),
            Some(embedder.model_id())
        );
        assert_eq!(command.model_id(), embedder.model_id());
    }

    /// The uncapped test above cannot tell `chunk.chunk_index` (the ORIGINAL
    /// position) apart from the chunks-iteration loop position, because
    /// nothing is dropped: reverting `execute` to the loop index still
    /// passes it. Here the plan is capped, so the two diverge.
    #[tokio::test]
    async fn command_keeps_original_chunk_numbers_when_the_plan_is_capped() {
        let mut config = CompatibilityBundleConfig::local_reference();
        config.chunk_target_tokens = 1;
        config.chunk_max_tokens = 1;
        config.chunk_cap = 2;

        // Three 4-char fixed-window chunks with original indices 0 ("aaaa"),
        // 1 ("bbbb"), 2 ("cccc"); capped to 2 survivors.
        // `strided_selection_indices(3, 2)` (chunker.rs) keeps original
        // indices [0, 2] and drops "bbbb" (index 1). The first surviving
        // chunk in iteration order ("aaaa", original index 0) finds a
        // near-identical neighbor and is excluded from the command; only
        // "cccc" (original index 2) is inserted. Using the loop position
        // (0, since "aaaa" was excluded and "cccc" is the second chunk
        // *iterated*, position 1) instead of `chunk.chunk_index` would
        // wrongly record that surviving entry as chunk `1`.
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
            embedder,
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
            vec![2],
            "the surviving entry must keep its ORIGINAL chunk number, not its position among survivors"
        );
        let evidence = &output.result().evidence;
        assert_eq!(evidence.chunk_count, Some(2), "two chunks survive the cap");
        assert_eq!(
            evidence.total_chunk_count,
            Some(3),
            "three chunks existed before the cap"
        );
        assert_eq!(evidence.chunks_capped, Some(true));
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

    /// Zaki review 1, round 2, N-6: the scorer, the embedder and the index
    /// reader are synchronous, and a production scorer makes network calls,
    /// so a Score runs them on the blocking pool, never on a runtime worker:
    /// a scorer and an index that fail every call made on a worker do not
    /// stop the Score.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn score_runs_its_synchronous_dependencies_on_the_blocking_pool() {
        let config = CompatibilityBundleConfig::local_reference();
        let index = IsolatedPipelineIndex::new();
        index.refuse_calls_on_runtime_workers();
        let policy = CompatibilityScorePolicy::new(
            manifest_pinning_trace_credit(&config),
            config.clone(),
            Arc::new(BlockingPoolOnlyScorer(ReferencePerplexityScorer::new())),
            Arc::new(ReferenceEmbedder::new()),
            index,
        )
        .unwrap();
        let output = policy
            .execute(&score_input(b"hello world, this is a compatibility trace"))
            .await
            .expect("the Score runs its dependencies off the runtime workers");
        let (result, _, _) = output.into_parts();
        assert_eq!(result.evidence.quality_passed, Some(true));
    }

    /// Finding 12: the reader a compatibility Score uses counts an
    /// unapplied command's entries as neighbours with the live index, under
    /// the entry ids Settle will write, by cosine similarity. It never
    /// returns the querying revision's own entries, another tenant's or
    /// another index's, or an entry the live index already returned; and
    /// its snapshot keeps the live generation and counts the unapplied
    /// entries.
    #[test]
    fn unapplied_index_commands_count_as_neighbours_with_the_live_index() {
        let tenant = pipeline_tenant_storage_ref("tenant-unapplied");
        let other_tenant = pipeline_tenant_storage_ref("tenant-unapplied-other");
        let live = IsolatedPipelineIndex::new();
        let command = |revision: u128, embedding: Vec<f32>| {
            SealedIndexCommand::new(
                MINIMAL_INDEX_ID,
                Uuid::from_u128(revision),
                MINIMAL_PROJECTION_ID,
                "reference-embedder-v1",
                vec![SealedIndexEntry {
                    chunk: 0,
                    content_hash: format!("sha256:{}", "a".repeat(64)),
                    embedding,
                }],
            )
            .unwrap()
        };
        let applied = command(1, vec![0.0, 2.0]);
        let unapplied = command(2, vec![2.0, 0.0]);
        let (applied_key, applied_entry) = applied.keyed_entries(&tenant).next().unwrap();
        live.upsert(
            &applied_key,
            &applied_entry.embedding,
            &applied_entry.content_hash,
        )
        .unwrap();
        let reader = UnappliedIndexCommandsReader::new(
            live.clone(),
            tenant.clone(),
            &[applied.clone(), unapplied.clone()],
        );
        let unapplied_id = unapplied
            .keyed_entries(&tenant)
            .next()
            .unwrap()
            .0
            .entry_id();

        let found = reader
            .nearest(
                &tenant,
                MINIMAL_INDEX_ID,
                &[1.0, 0.0],
                8,
                Some(Uuid::from_u128(3)),
            )
            .unwrap();
        assert_eq!(
            found.iter().map(|n| n.entry_id).collect::<Vec<_>>(),
            vec![unapplied_id, applied_key.entry_id()],
            "the unapplied entry ranks first, and the applied one is not counted twice"
        );
        assert!((found[0].similarity - 1.0).abs() < 1e-6, "cosine, not dot");
        assert!(
            reader
                .nearest(
                    &tenant,
                    MINIMAL_INDEX_ID,
                    &[1.0, 0.0],
                    8,
                    Some(Uuid::from_u128(2))
                )
                .unwrap()
                .iter()
                .all(|n| n.entry_id != unapplied_id),
            "the querying revision's own entries are left out"
        );
        assert!(
            reader
                .nearest(&other_tenant, MINIMAL_INDEX_ID, &[1.0, 0.0], 8, None)
                .unwrap()
                .is_empty(),
            "another tenant sees no unapplied entry"
        );
        assert!(
            reader
                .nearest(&tenant, "another-index", &[1.0, 0.0], 8, None)
                .unwrap()
                .is_empty(),
            "another index sees no unapplied entry"
        );

        let live_snapshot = live.snapshot(&tenant, MINIMAL_INDEX_ID).unwrap();
        let snapshot =
            UnappliedIndexCommandsReader::new(live.clone(), tenant.clone(), &[unapplied])
                .snapshot(&tenant, MINIMAL_INDEX_ID)
                .unwrap();
        assert_eq!(snapshot.snapshot_id, live_snapshot.snapshot_id);
        assert_ne!(snapshot.snapshot_hash, live_snapshot.snapshot_hash);
        assert_eq!(snapshot.cardinality, live_snapshot.cardinality + 1);
        assert_eq!(
            reader.snapshot(&other_tenant, MINIMAL_INDEX_ID).unwrap(),
            live.snapshot(&other_tenant, MINIMAL_INDEX_ID).unwrap(),
            "with no unapplied entry the snapshot is the live one"
        );
    }

    /// Finding 11: a production-compatible configuration is validated as
    /// `main` validates its gate configuration at startup. All-zero floors
    /// are refused; any single positive floor is accepted, so the pilot
    /// template's floors (perplexity 0, tail fraction 0, novelty 500000;
    /// poldsam P-4) are, and so is a zero tail-fraction floor with the other
    /// two positive.
    #[test]
    fn production_compatibility_is_validated_as_mains_gate_configuration() {
        let config = |perplexity, tail, novelty| {
            CompatibilityBundleConfig::production_compatible(
                "scorer.v1".to_string(),
                MINIMAL_PROJECTION_ID.to_string(),
                MINIMAL_INDEX_ID.to_string(),
                &MainGateConfig {
                    perplexity_floor_micros: Some(perplexity),
                    tail_fraction_floor_micros: Some(tail),
                    novelty_floor_micros: Some(novelty),
                    ..main_gate()
                },
            )
        };
        assert_eq!(
            config(0, 0, 0).unwrap_err().to_string(),
            COMPATIBILITY_ZERO_FLOOR_LABEL,
            "all-zero floors are refused, as main refuses them"
        );
        config(0, 0, 500_000).expect("the pilot template's floors are accepted");
        config(2_000_000, 0, 500_000).expect("a zero tail floor alone is accepted");
        config(1, 0, 0).expect("one positive floor is enough, as on main");
        config(0, 0, 1).expect("one positive floor is enough, as on main");
    }

    /// `main`'s defaults for every gate value but the floors (the pilot
    /// template's 0/0/500000), with a non-zero delta.
    fn main_gate() -> MainGateConfig {
        MainGateConfig {
            perplexity_floor_micros: Some(0),
            tail_fraction_floor_micros: Some(0),
            novelty_floor_micros: Some(500_000),
            embed_insert_novelty_micros: 50_000,
            top_k: 5,
            chunk_target_tokens: 2048,
            chunk_max_tokens: 3072,
            chunk_cap: 16,
            chunk_min_tokens: 64,
            novelty_utility_microcredits: 2_500_000,
        }
    }

    /// Finding 14, multi-lens review L5-4 and Zaki review 3, Z3-3: every
    /// gate value is `main`'s own setting, passed in -- the index-insert
    /// threshold (not the novelty floor), top-k, the chunk knobs and the
    /// delta -- and the result matches only that configuration. `main`
    /// without floors builds nothing.
    #[test]
    fn production_compatibility_takes_every_value_from_mains_gate() {
        let gate = main_gate();
        let config = CompatibilityBundleConfig::production_compatible(
            "scorer.v1".to_string(),
            MINIMAL_PROJECTION_ID.to_string(),
            MINIMAL_INDEX_ID.to_string(),
            &gate,
        )
        .unwrap();
        assert_eq!(config.embed_insert_novelty_micros, 50_000);
        assert_eq!(config.novelty_floor_micros, 500_000);
        assert_eq!(config.top_k, 5);
        assert_eq!(config.novelty_utility_microcredits, 2_500_000);
        assert!(config.matches_main_gate(&gate));
        for other in [
            MainGateConfig { top_k: 8, ..gate },
            MainGateConfig {
                chunk_cap: 8,
                ..gate
            },
            MainGateConfig {
                novelty_utility_microcredits: 0,
                ..gate
            },
            MainGateConfig {
                perplexity_floor_micros: None,
                ..gate
            },
        ] {
            assert!(!config.matches_main_gate(&other), "{other:?}");
        }
        assert_eq!(
            CompatibilityBundleConfig::production_compatible(
                "scorer.v1".to_string(),
                MINIMAL_PROJECTION_ID.to_string(),
                MINIMAL_INDEX_ID.to_string(),
                &MainGateConfig {
                    novelty_floor_micros: None,
                    ..gate
                },
            )
            .unwrap_err()
            .to_string(),
            COMPATIBILITY_MAIN_GATE_FLOORS_MISSING_LABEL
        );
    }

    #[test]
    fn production_compatibility_rejects_zero_floors() {
        let mut config = CompatibilityBundleConfig::local_reference();
        assert!(!config.is_qualifiable());
        config.qualification = CompatibilityQualification::ProductionCompatible;
        assert_eq!(
            config.validate().unwrap_err().to_string(),
            COMPATIBILITY_ZERO_FLOOR_LABEL
        );

        config.perplexity_floor_micros = 1;
        config.tail_fraction_floor_micros = 1;
        config.novelty_floor_micros = 1;
        config.validate().unwrap();
        assert!(config.is_qualifiable());
    }

    fn settle_input(
        score: ScoreDecision,
        evidence: ScoreEvidence,
        index_command: Option<SealedIndexCommand>,
    ) -> SettleInput {
        SettleInput {
            run_id: Uuid::new_v4(),
            tenant_storage_ref: pipeline_tenant_storage_ref("tenant-a"),
            trace_id: Uuid::new_v4(),
            registry_revision_id: Uuid::nil(),
            source_content_hash: dependency_content_hash(b"settle-input-source"),
            score,
            score_evidence: evidence,
            index_command,
        }
    }

    #[tokio::test]
    async fn settle_uses_committed_include_flag_not_live_index() {
        let settle = CompatibilitySettlePolicy;
        let config = CompatibilityBundleConfig::local_reference();
        let manifest = manifest_pinning_trace_credit(&config);
        let awards = InstrumentAwards::new(vec![
            InstrumentAward::new(
                InstrumentId::trace_credit(),
                AtomicUnits::from_raw(1_000_000),
            )
            .unwrap(),
        ])
        .unwrap();
        let score = ScoreDecision::for_bundle(&manifest, awards.clone()).unwrap();
        let mut evidence = ScoreEvidence::fixed(awards);
        let command = SealedIndexCommand::new(
            &config.index_id,
            Uuid::nil(),
            &config.projection_id,
            "reference-embedder-v1",
            vec![SealedIndexEntry {
                chunk: 0,
                content_hash: dependency_content_hash(b"chunk-0"),
                embedding: vec![0.0, 1.0],
            }],
        )
        .unwrap();

        // A stored command AND a committed `include_eligible: true` --
        // membership is `Include`.
        evidence.include_eligible = Some(true);
        let included = settle
            .execute(&settle_input(
                score.clone(),
                evidence.clone(),
                Some(command.clone()),
            ))
            .await
            .unwrap();
        assert!(matches!(
            included.decision.index_membership,
            IndexMembershipDecision::Include { .. }
        ));

        // A stored command, but the committed flag is `false`: membership
        // must follow the committed flag, not the mere presence of a
        // command (which is what "not the live index" means here -- there
        // is no live index in this policy at all, only committed evidence).
        evidence.include_eligible = Some(false);
        let excluded_by_flag = settle
            .execute(&settle_input(
                score.clone(),
                evidence.clone(),
                Some(command.clone()),
            ))
            .await
            .unwrap();
        match excluded_by_flag.decision.index_membership {
            IndexMembershipDecision::Exclude { reason } => {
                assert_eq!(
                    reason,
                    ReasonCode::new(COMPATIBILITY_EXCLUDE_REASON).unwrap()
                );
            }
            IndexMembershipDecision::Include { .. } => {
                panic!("a false committed flag must exclude even with a stored command")
            }
        }

        // The committed flag says `true`, but there is no stored command at
        // all: membership must not fabricate one from the flag alone.
        evidence.include_eligible = Some(true);
        let excluded_by_missing_command = settle
            .execute(&settle_input(score, evidence, None))
            .await
            .unwrap();
        assert!(
            matches!(
                excluded_by_missing_command.decision.index_membership,
                IndexMembershipDecision::Exclude { .. }
            ),
            "a true flag with no stored command must not fabricate membership"
        );
    }

    /// The `configuration_identities.gate_floors` label the compatibility
    /// baseline fixture pins for [`CompatibilityBundleConfig::local_reference`]:
    /// uncalibrated, all-zero floors, not production qualifiable.
    const COMPATIBILITY_LOCAL_GATE_FLOORS_LABEL: &str = "local_zero_uncalibrated_reference";

    /// The five fixture classes' expected per-phase outcomes, exactly as
    /// `docs/superpowers/specs/versioned-pipeline-compatibility-baseline-v1.json`
    /// states them under `expected_fixture_classes`.
    fn compatibility_local_reference_observation(
        config: &CompatibilityBundleConfig,
    ) -> CompatibilityBaselineObservation {
        let corpus_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../docs/superpowers/specs/fixtures/versioned-pipeline-minimal-corpus-v1.json",
        );
        let corpus_bytes =
            std::fs::read_to_string(&corpus_path).expect("read compatibility corpus fixture");
        let corpus: CompatibilityCorpusDocument =
            serde_json::from_str(&corpus_bytes).expect("parse compatibility corpus fixture");
        let fixture_classes = BTreeMap::from([
            (
                "clean_tool_plan".to_string(),
                CompatibilityFixtureDecision {
                    admission: "admit".to_string(),
                    review: "approved".to_string(),
                    score: "completed".to_string(),
                    settle: "completed".to_string(),
                },
            ),
            (
                "locally_redacted_secret".to_string(),
                CompatibilityFixtureDecision {
                    admission: "admit".to_string(),
                    review: "approved".to_string(),
                    score: "completed".to_string(),
                    settle: "completed".to_string(),
                },
            ),
            (
                "privacy_quarantine_approved".to_string(),
                CompatibilityFixtureDecision {
                    admission: "quarantine".to_string(),
                    review: "approved".to_string(),
                    score: "completed".to_string(),
                    settle: "completed".to_string(),
                },
            ),
            (
                "privacy_quarantine_rejected".to_string(),
                CompatibilityFixtureDecision {
                    admission: "quarantine".to_string(),
                    review: "rejected".to_string(),
                    score: "skipped".to_string(),
                    settle: "skipped".to_string(),
                },
            ),
            (
                "privacy_risk_rejected".to_string(),
                CompatibilityFixtureDecision {
                    admission: "reject".to_string(),
                    review: "skipped".to_string(),
                    score: "skipped".to_string(),
                    settle: "skipped".to_string(),
                },
            ),
        ]);
        CompatibilityBaselineObservation {
            fixture_order: corpus
                .fixtures
                .into_iter()
                .map(|fixture| fixture.label)
                .collect(),
            initial_index: if IsolatedPipelineIndex::new().entry_count(
                &pipeline_tenant_storage_ref("baseline-tenant"),
                MINIMAL_INDEX_ID,
            ) == 0
            {
                "empty".to_string()
            } else {
                "seeded".to_string()
            },
            scorer_model_id: config.scorer_model_id.clone(),
            projection_id: config.projection_id.clone(),
            index_id: config.index_id.clone(),
            gate_floors: COMPATIBILITY_LOCAL_GATE_FLOORS_LABEL.to_string(),
            fixture_classes,
        }
    }

    /// Ruling T4-1: written against the R1-corrected fixture as it stands
    /// now (a `gate_floors` label, `corpus.path`/`order`/`initial_index`,
    /// no `fixture_order` list). Proves the comparison (a) passes for an
    /// observation built from this bundle's `local_reference()` config and
    /// the corpus file's own order, and (b) actually detects drift -- a
    /// changed fixture class, identity label, corpus order, or starting
    /// index state each make it fail, so the equality check above is not a
    /// tautology.
    #[test]
    fn compatibility_baseline_is_executable_and_detects_drift() {
        let config = CompatibilityBundleConfig::local_reference();
        // The fixture's `gate_floors` label describes exactly this
        // configuration; if `local_reference()` ever stops being the
        // all-zero, non-qualifiable reference, this constant (and the
        // fixture) need a fresh look, not a silent pass.
        assert_eq!(
            config.qualification,
            CompatibilityQualification::LocalSyntheticNonQualifiable
        );
        assert_eq!(config.perplexity_floor_micros, 0);
        assert_eq!(config.tail_fraction_floor_micros, 0);
        assert_eq!(config.novelty_floor_micros, 0);

        let expected = compatibility_local_reference_observation(&config);
        compare_compatibility_baseline(&expected).unwrap();

        let mut changed_class = expected.clone();
        changed_class
            .fixture_classes
            .get_mut("clean_tool_plan")
            .unwrap()
            .score = "failed".to_string();
        assert!(compare_compatibility_baseline(&changed_class).is_err());

        let mut changed_identity = expected.clone();
        changed_identity.gate_floors = "changed".to_string();
        assert!(compare_compatibility_baseline(&changed_identity).is_err());

        let mut changed_order = expected.clone();
        changed_order.fixture_order.swap(0, 1);
        assert!(compare_compatibility_baseline(&changed_order).is_err());

        let mut changed_index = expected;
        changed_index.initial_index = "seeded".to_string();
        assert!(compare_compatibility_baseline(&changed_index).is_err());
    }
}
