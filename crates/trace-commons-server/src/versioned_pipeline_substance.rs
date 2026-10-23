// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The model-free substance formula: a deduplicated tool-call projection of an
//! envelope, and the work, replay and composite terms over it.
//!
//! It also holds the substance Score policy (`trace_commons.score.substance.v1`)
//! and its package builders. Substance is a built-in Score policy of the
//! compatibility family (SQ-1): the bundle runs the compatibility Admission,
//! Review and Settle policies, and `MinimalPolicyBundle` binds this Score
//! policy in its own arm. The policy calls no perplexity scorer. Novelty runs
//! the compatibility code (`evaluate_novelty`), with each chunk weighted by
//! the chunker's character estimate ([`estimated_chunk_tokens`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use trace_commons_gate_api::pipeline::{
    BundleManifest, BundlePackage, MAX_TRACE_CREDIT_MICROCREDITS, PolicyError, ScoreInput,
    ScoreOutput, ScorePolicy, ScoreWitnessClass,
};
use trace_commons_gate_api::{
    IdentifiedEmbedder, IdentifiedIndexReader, IdentifiedPerplexityScorer, ReferenceEmbedder,
    ReferencePerplexityScorer,
};
use trace_commons_gate_enclave::chunker::{
    APPROX_CHARS_PER_TOKEN, ChunkerConfig, TraceChunk, chunk_envelope_plaintext,
};
use trace_commons_protocol::canonical_json::to_canonical_string;
use trace_commons_protocol::trace_contribution::{
    TraceContributionEnvelope, TraceContributionEventType, event_carries_result, replay_call_id,
    replay_sufficiency, tool_call_arguments,
};
use uuid::Uuid;

use crate::credit_quality::{
    CREDIT_QUALITY_VERSION_SUBSTANCE, NoveltyCreditParams, substance_credit_quality,
};
use crate::versioned_pipeline_bundle::{
    PIPELINE_BUNDLE_INVALID_LABEL, PipelineInstrumentAwardConfig, compatibility_family_package,
};
use crate::versioned_pipeline_compat::{
    COMPATIBILITY_ZERO_FLOOR_LABEL, CompatibilityBundleConfig, CompatibilityQualification,
    NoveltyScoringConfig, apply_novelty_rule, evaluate_novelty, novelty_score_output,
};

pub const SUBSTANCE_SCORE_IMPLEMENTATION: &str = "trace_commons.score.substance.v1";
pub const SUBSTANCE_SCORE_RULE: &str = "substance_novelty_v1";
pub const SUBSTANCE_VERSION: u32 = 1;
/// The Score `policy_id` of a substance package.
pub const SUBSTANCE_SCORE_POLICY_ID: &str = "trace_commons.score.substance";
/// Safe label for a reviewed artifact that is not a contribution envelope.
/// Permanent: a retry reads the same bytes (Review Focus 4).
pub const SUBSTANCE_ENVELOPE_INVALID_LABEL: &str = "substance_envelope_invalid";
/// The `NoveltyUtility` delta of [`substance_candidate_package`], the same
/// delta the compatibility qualification candidate carries.
pub const SUBSTANCE_CANDIDATE_NOVELTY_UTILITY_MICROCREDITS: u64 = 2_500_000;
/// The percentile of the pinned corpus that a qualifiable `sub_ceil` comes
/// from (spec section 6).
pub const SUBSTANCE_SUB_CEIL_PERCENTILE: u32 = 90;

const MICROS: u32 = 1_000_000;

/// The set parameters of the formula, in micros where fractional.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceParams {
    pub pc_ceil: u32,
    pub dt_ceil: u32,
    pub w_pc_micros: u32,
    pub w_dt_micros: u32,
    pub w_hp_micros: u32,
    pub w_sa_micros: u32,
    pub w_sr_micros: u32,
    pub m_work_micros: u32,
    pub m_rep_micros: u32,
    pub att_floor_mult_micros: u32,
}

impl SubstanceParams {
    /// The values of the RLVR persona (spec section 6).
    pub fn rlvr_v1() -> Self {
        Self {
            pc_ceil: 20,
            dt_ceil: 8,
            w_pc_micros: 600_000,
            w_dt_micros: 400_000,
            w_hp_micros: 400_000,
            w_sa_micros: 300_000,
            w_sr_micros: 300_000,
            m_work_micros: 300_000,
            m_rep_micros: 100_000,
            att_floor_mult_micros: 900_000,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        let micros = [
            self.w_pc_micros,
            self.w_dt_micros,
            self.w_hp_micros,
            self.w_sa_micros,
            self.w_sr_micros,
            self.m_work_micros,
            self.m_rep_micros,
            self.att_floor_mult_micros,
        ];
        let floors = [
            self.m_work_micros,
            self.m_rep_micros,
            self.att_floor_mult_micros,
        ];
        if self.pc_ceil == 0
            || self.dt_ceil == 0
            || micros.iter().any(|value| *value > MICROS)
            || floors.contains(&0)
        {
            return Err("substance_params_invalid");
        }
        Ok(())
    }
}

/// Counts over the deduplicated tool calls of one envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ToolCallProjection {
    pub tool_calls: u64,
    pub paired_calls: u64,
    pub distinct_tools: u64,
    pub calls_with_arguments: u64,
    pub calls_with_results: u64,
    pub has_initial_prompt: bool,
}

struct ResultSlot<'a> {
    call_id: Option<&'a str>,
    parent_event_id: Option<Uuid>,
    used: bool,
}

#[derive(Default)]
struct CallGroup {
    has_arguments: bool,
    has_result: bool,
}

/// Project an envelope onto deduplicated tool calls (SQ-4 and SQ-5).
pub fn project_tool_calls(envelope: &TraceContributionEnvelope) -> ToolCallProjection {
    let mut results: Vec<ResultSlot<'_>> = envelope
        .events
        .iter()
        .filter(|event| event.event_type == TraceContributionEventType::ToolResult)
        .filter(|event| event_carries_result(event))
        .map(|event| ResultSlot {
            call_id: replay_call_id(event),
            parent_event_id: event.parent_event_id,
            used: false,
        })
        .collect();

    let mut groups: BTreeMap<(String, String), CallGroup> = BTreeMap::new();
    for call in envelope
        .events
        .iter()
        .filter(|event| event.event_type == TraceContributionEventType::ToolCall)
    {
        let name = call.tool_name.clone().unwrap_or_default();
        let arguments = tool_call_arguments(call);
        let canonical = arguments
            .map(|value| to_canonical_string(value).unwrap_or_else(|_| value.to_string()))
            .unwrap_or_default();
        let call_id = replay_call_id(call);
        let slot = results.iter_mut().find(|slot| {
            if slot.used {
                return false;
            }
            match (call_id, slot.call_id) {
                (Some(call_id), Some(result_id)) => call_id == result_id,
                _ => slot.parent_event_id == Some(call.event_id),
            }
        });
        let matched = slot.is_some_and(|slot| {
            slot.used = true;
            true
        });
        let group = groups.entry((name, canonical)).or_default();
        group.has_arguments = arguments.is_some();
        group.has_result |= matched;
    }

    let mut projection = ToolCallProjection {
        tool_calls: groups.len() as u64,
        has_initial_prompt: replay_sufficiency(envelope).has_initial_prompt,
        ..ToolCallProjection::default()
    };
    let mut paired_tools: BTreeSet<&str> = BTreeSet::new();
    for ((name, _), group) in &groups {
        projection.calls_with_arguments += u64::from(group.has_arguments);
        projection.calls_with_results += u64::from(group.has_result);
        if group.has_arguments && group.has_result {
            projection.paired_calls += 1;
            paired_tools.insert(name.as_str());
        }
    }
    projection.distinct_tools = paired_tools.len() as u64;
    projection
}

/// Raw terms, each clamped to `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SubstanceTerms {
    pub work: f64,
    pub replay: f64,
    pub v: f64,
}

/// `ln(1 + x) / ln(1 + ceil)`, clamped to `[0, 1]`.
pub fn sat(x: f64, ceil: f64) -> f64 {
    (x.ln_1p() / ceil.ln_1p()).clamp(0.0, 1.0)
}

/// `m + (1 - m) * v`: lifts `[0, 1]` onto `[m, 1]`.
pub fn graded(v: f64, m: f64) -> f64 {
    m + (1.0 - m) * v
}

/// The token weight of a chunk for the novelty aggregate (SQ-3). The
/// substance Score calls no perplexity scorer, so the chunker's character
/// budget gives the count: characters over [`APPROX_CHARS_PER_TOKEN`],
/// rounded up, at least 1.
pub fn estimated_chunk_tokens(chunk: &TraceChunk) -> u64 {
    (chunk.text.chars().count().div_ceil(APPROX_CHARS_PER_TOKEN) as u64).max(1)
}

/// Terms in micros: clamp to `[0, 1]`, then round.
pub fn to_micros(x: f64) -> u64 {
    (x.clamp(0.0, 1.0) * f64::from(MICROS)).round() as u64
}

fn weight(micros: u32) -> f64 {
    f64::from(micros) / f64::from(MICROS)
}

pub fn substance_terms(
    projection: &ToolCallProjection,
    witness: ScoreWitnessClass,
    params: &SubstanceParams,
) -> SubstanceTerms {
    let work = weight(params.w_pc_micros)
        * sat(projection.paired_calls as f64, f64::from(params.pc_ceil))
        + weight(params.w_dt_micros)
            * sat(projection.distinct_tools as f64, f64::from(params.dt_ceil));
    let (share_args, share_results) = if projection.tool_calls == 0 {
        (0.0, 0.0)
    } else {
        let calls = projection.tool_calls as f64;
        (
            projection.calls_with_arguments as f64 / calls,
            projection.calls_with_results as f64 / calls,
        )
    };
    let replay = weight(params.w_hp_micros) * f64::from(u8::from(projection.has_initial_prompt))
        + weight(params.w_sa_micros) * share_args
        + weight(params.w_sr_micros) * share_results;
    SubstanceTerms {
        work: work.clamp(0.0, 1.0),
        replay: replay.clamp(0.0, 1.0),
        v: f64::from(u8::from(witness.is_attested_final_call())),
    }
}

pub fn composite(terms: &SubstanceTerms, params: &SubstanceParams) -> f64 {
    graded(terms.work, weight(params.m_work_micros))
        * graded(terms.replay, weight(params.m_rep_micros))
        * graded(terms.v, weight(params.att_floor_mult_micros))
}

/// Where the substance Score takes the witness class from (SQ-15). This cut
/// does not read the stored class, so every trace scores as
/// [`ScoreWitnessClass::Unattested`] (SQ-13). The bundle id hashes the
/// variant, so a package of this cut keeps its scores when a later variant
/// reads the stored class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubstanceWitnessSource {
    NotRead,
}

impl SubstanceWitnessSource {
    fn witness_class(self) -> ScoreWitnessClass {
        match self {
            Self::NotRead => ScoreWitnessClass::Unattested,
        }
    }
}

/// The novelty half of a substance configuration: the novelty gate, the
/// index command threshold, the chunk knobs, the novelty parameters of
/// `cq4`, and the `NoveltyUtility` award with its instrument pin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceNoveltyConfig {
    pub projection_id: String,
    pub index_id: String,
    pub novelty_floor_micros: u64,
    pub embed_insert_novelty_micros: u64,
    pub top_k: u32,
    pub chunk_target_tokens: u32,
    pub chunk_max_tokens: u32,
    pub chunk_cap: u32,
    pub chunk_min_tokens: u64,
    pub credit: NoveltyCreditParams,
    /// The flat `NoveltyUtility` delta, in trace-credit microcredits,
    /// awarded when both gates pass. `0`: no award.
    pub novelty_utility_microcredits: u64,
    /// The instrument the award is pinned against; the package pins
    /// `trace_credit` to `instrument.descriptor`.
    pub instrument: PipelineInstrumentAwardConfig,
}

impl SubstanceNoveltyConfig {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.projection_id.trim().is_empty() && !self.index_id.trim().is_empty(),
            PIPELINE_BUNDLE_INVALID_LABEL
        );
        anyhow::ensure!(
            self.top_k > 0
                && self.chunk_target_tokens > 0
                && self.chunk_max_tokens >= self.chunk_target_tokens
                && self.chunk_cap > 0
                && self.chunk_min_tokens > 0,
            PIPELINE_BUNDLE_INVALID_LABEL
        );
        anyhow::ensure!(
            self.credit.nov_floor_micros >= 0
                && self.credit.nov_ceil_micros > self.credit.nov_floor_micros
                && (0..=i64::from(MICROS)).contains(&self.credit.nov_floor_mult_micros),
            PIPELINE_BUNDLE_INVALID_LABEL
        );
        anyhow::ensure!(
            self.novelty_utility_microcredits <= MAX_TRACE_CREDIT_MICROCREDITS,
            PIPELINE_BUNDLE_INVALID_LABEL
        );
        Ok(())
    }

    fn novelty_scoring(&self) -> NoveltyScoringConfig {
        NoveltyScoringConfig {
            index_id: self.index_id.clone(),
            projection_id: self.projection_id.clone(),
            top_k: self.top_k,
            chunk_min_tokens: self.chunk_min_tokens,
            embed_insert_novelty_micros: self.embed_insert_novelty_micros,
            novelty_floor_micros: self.novelty_floor_micros,
            novelty_utility_microcredits: self.novelty_utility_microcredits,
        }
    }

    fn chunker(&self) -> ChunkerConfig {
        ChunkerConfig {
            target_tokens: self.chunk_target_tokens as usize,
            max_tokens: self.chunk_max_tokens as usize,
            chunk_cap: self.chunk_cap as usize,
        }
    }
}

/// What the offline calibration (`evaluate`) measured the calibrated values
/// on: the pinned corpus, the report, and the two percentiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceCalibration {
    pub corpus_pin_digest: String,
    pub report_digest: String,
    pub sub_ceil_percentile: u32,
    pub substance_floor_percentile: u32,
}

/// The values the offline calibration writes, with their provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibratedSubstanceValues {
    pub sub_ceil_micros: u64,
    pub substance_floor_micros: u64,
    pub calibration: SubstanceCalibration,
}

/// The configuration of the substance Score policy (spec section 8.2). The
/// bundle id hashes all of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceScoreConfig {
    pub qualification: CompatibilityQualification,
    pub substance_version: u32,
    pub witness_source: SubstanceWitnessSource,
    pub params: SubstanceParams,
    /// The ceiling of the substance half of `cq4`. Calibrated.
    pub sub_ceil_micros: u64,
    /// The substance gate: a trace passes at or above it. Calibrated.
    pub substance_floor_micros: u64,
    pub sub_floor_mult_micros: u32,
    pub calibration: Option<SubstanceCalibration>,
    pub novelty: SubstanceNoveltyConfig,
}

impl SubstanceScoreConfig {
    /// A local, non-qualifiable configuration: the RLVR parameters, an
    /// uncalibrated ceiling of one whole and a zero floor, and the novelty
    /// values of [`CompatibilityBundleConfig::local_reference`] with the
    /// novelty parameters of `cq3`.
    pub fn local_reference() -> Self {
        let compatibility = CompatibilityBundleConfig::local_reference();
        Self {
            qualification: CompatibilityQualification::LocalSyntheticNonQualifiable,
            substance_version: SUBSTANCE_VERSION,
            witness_source: SubstanceWitnessSource::NotRead,
            params: SubstanceParams::rlvr_v1(),
            sub_ceil_micros: 1_000_000,
            substance_floor_micros: 0,
            sub_floor_mult_micros: 250_000,
            calibration: None,
            novelty: SubstanceNoveltyConfig {
                projection_id: compatibility.projection_id,
                index_id: compatibility.index_id,
                novelty_floor_micros: compatibility.novelty_floor_micros,
                embed_insert_novelty_micros: compatibility.embed_insert_novelty_micros,
                top_k: compatibility.top_k,
                chunk_target_tokens: compatibility.chunk_target_tokens,
                chunk_max_tokens: compatibility.chunk_max_tokens,
                chunk_cap: compatibility.chunk_cap,
                chunk_min_tokens: compatibility.chunk_min_tokens,
                credit: NoveltyCreditParams::cq3(),
                novelty_utility_microcredits: compatibility.novelty_utility_microcredits,
                instrument: compatibility.instrument,
            },
        }
    }

    /// This configuration with the calibrated ceiling and floor, and the
    /// calibration they came from.
    pub fn with_calibrated(mut self, values: &CalibratedSubstanceValues) -> Self {
        self.sub_ceil_micros = values.sub_ceil_micros;
        self.substance_floor_micros = values.substance_floor_micros;
        self.calibration = Some(values.calibration.clone());
        self
    }

    /// Refuses a production-compatible configuration whose substance and
    /// novelty floors are both zero (`compatibility_zero_floor`), and refuses
    /// (`bundle_package_invalid`) a configuration the formula cannot run
    /// safely: invalid parameters (a zero ceiling gives a NaN
    /// term, which `to_micros` turns into 0), a `sub_ceil` of 0 (the credit
    /// half would count it as one micro and raise credit) or above one
    /// whole, a floor or floor multiplier above one whole, another
    /// `substance_version`, a calibration out of range, or invalid novelty
    /// values.
    pub fn validate(&self) -> anyhow::Result<()> {
        self.params
            .validate()
            .map_err(|_| anyhow::anyhow!(PIPELINE_BUNDLE_INVALID_LABEL))?;
        anyhow::ensure!(
            self.substance_version == SUBSTANCE_VERSION,
            PIPELINE_BUNDLE_INVALID_LABEL
        );
        anyhow::ensure!(
            self.sub_ceil_micros > 0
                && self.sub_ceil_micros <= u64::from(MICROS)
                && self.substance_floor_micros <= u64::from(MICROS)
                && self.sub_floor_mult_micros <= MICROS,
            PIPELINE_BUNDLE_INVALID_LABEL
        );
        // As the compatibility guard: a production-compatible configuration
        // whose two floors are both zero would pass every trace through both
        // gates, so it is refused. Either floor positive is enough.
        if self.qualification == CompatibilityQualification::ProductionCompatible {
            anyhow::ensure!(
                self.substance_floor_micros > 0 || self.novelty.novelty_floor_micros > 0,
                COMPATIBILITY_ZERO_FLOOR_LABEL
            );
        }
        if let Some(calibration) = &self.calibration {
            anyhow::ensure!(
                !calibration.corpus_pin_digest.trim().is_empty()
                    && !calibration.report_digest.trim().is_empty()
                    && calibration.sub_ceil_percentile <= 100
                    && calibration.substance_floor_percentile <= 100,
                PIPELINE_BUNDLE_INVALID_LABEL
            );
        }
        self.novelty.validate()
    }

    /// Whether a runtime that routes or drains tenants may bind this
    /// configuration: production-compatible, calibrated at the 90th
    /// percentile, and valid.
    pub fn is_qualifiable(&self) -> bool {
        self.qualification == CompatibilityQualification::ProductionCompatible
            && self.calibration.as_ref().is_some_and(|calibration| {
                calibration.sub_ceil_percentile == SUBSTANCE_SUB_CEIL_PERCENTILE
            })
            && self.validate().is_ok()
    }
}

fn permanent(label: &'static str) -> PolicyError {
    PolicyError::permanent(label).expect("static label")
}

/// The substance Score policy. Holds its dependencies only as trait
/// objects, and holds no perplexity scorer: substance calls none. Holds the
/// bundle manifest it was constructed under, so every decision it builds is
/// proven pinned (`ScoreDecision::for_bundle`).
#[derive(Clone)]
pub struct SubstanceScorePolicy {
    manifest: BundleManifest,
    config: SubstanceScoreConfig,
    embedder: Arc<dyn IdentifiedEmbedder>,
    index_reader: Arc<dyn IdentifiedIndexReader>,
}

impl SubstanceScorePolicy {
    pub fn new(
        manifest: BundleManifest,
        config: SubstanceScoreConfig,
        embedder: Arc<dyn IdentifiedEmbedder>,
        index_reader: Arc<dyn IdentifiedIndexReader>,
    ) -> anyhow::Result<Self> {
        config.validate()?;
        Ok(Self {
            manifest,
            config,
            embedder,
            index_reader,
        })
    }
}

#[async_trait]
impl ScorePolicy for SubstanceScorePolicy {
    /// The embedder and the index reader are synchronous, so the evaluation
    /// runs on the blocking pool under the pipeline's bound on Score
    /// evaluations, as the compatibility Score does. A task that panicked
    /// or was cancelled is `score_task_failed`.
    async fn execute(&self, input: &ScoreInput) -> Result<ScoreOutput, PolicyError> {
        let policy = self.clone();
        let input = input.clone();
        crate::versioned_pipeline_blocking::run_score_evaluation(move || policy.evaluate(&input))
            .await
            .map_err(|_| permanent("score_task_failed"))?
    }
}

impl SubstanceScorePolicy {
    /// The evaluation `execute` runs on the blocking pool (spec section
    /// 8.2). `input.operator_review` is ignored, as in the compatibility
    /// Score: the policy never holds a trace.
    fn evaluate(&self, input: &ScoreInput) -> Result<ScoreOutput, PolicyError> {
        if input.reviewed_artifact.is_empty() {
            return Err(permanent("score_input_empty"));
        }
        let envelope: TraceContributionEnvelope = serde_json::from_slice(&input.reviewed_artifact)
            .map_err(|_| permanent(SUBSTANCE_ENVELOPE_INVALID_LABEL))?;
        let witness = self.config.witness_source.witness_class();
        let projection = project_tool_calls(&envelope);
        drop(envelope);
        let terms = substance_terms(&projection, witness, &self.config.params);
        let substance_micros = to_micros(composite(&terms, &self.config.params));
        let substance_passed = substance_micros >= self.config.substance_floor_micros;

        let novelty_config = self.config.novelty.novelty_scoring();
        let plan =
            chunk_envelope_plaintext(&input.reviewed_artifact, &self.config.novelty.chunker());
        // One weight per chunk, over the same chunk slice that
        // `evaluate_novelty` and `apply_novelty_rule` read (SQ-3).
        let token_weights: Vec<u64> = plan.chunks.iter().map(estimated_chunk_tokens).collect();
        let novelty = evaluate_novelty(
            self.embedder.as_ref(),
            self.index_reader.as_ref(),
            &input.tenant_storage_ref,
            input.registry_revision_id,
            &plan.chunks,
            &token_weights,
            &novelty_config,
        )?;
        let credit_quality_micros = substance_credit_quality(
            substance_micros,
            i64::try_from(novelty.novelty_score_micros).unwrap_or(i64::MAX),
            self.config.sub_ceil_micros,
            self.config.sub_floor_mult_micros,
            &self.config.novelty.credit,
        );
        // The compatibility rule, with `substance_passed` in place of
        // `quality_passed`: index membership and the award need both gates.
        let mut outcome = apply_novelty_rule(
            substance_passed,
            novelty,
            &plan,
            &novelty_config,
            self.embedder.model_id(),
            input,
        )?;
        let evidence = &mut outcome.evidence;
        // The legacy gate-decision row needs a perplexity and a tail
        // fraction; the family runs no scorer, so both are the fixed
        // sentinel 0 (spec section 8.1), told apart by the credit quality
        // version.
        evidence.perplexity_micros = Some(0);
        evidence.tail_fraction_micros = Some(0);
        evidence.quality_passed = Some(substance_passed);
        evidence.coverage_tokens = Some(
            token_weights
                .iter()
                .fold(0_u64, |total, tokens| total.saturating_add(*tokens)),
        );
        evidence.credit_quality_micros = Some(credit_quality_micros);
        evidence.credit_quality_version = Some(CREDIT_QUALITY_VERSION_SUBSTANCE);
        evidence.substance_micros = Some(substance_micros);
        evidence.substance_passed = Some(substance_passed);
        evidence.substance_version = Some(self.config.substance_version);
        evidence.substance_work_micros = Some(to_micros(terms.work));
        evidence.substance_replay_micros = Some(to_micros(terms.replay));
        evidence.substance_tool_calls = Some(projection.tool_calls);
        evidence.witness_provenance_class = Some(witness);
        novelty_score_output(&self.manifest, SUBSTANCE_SCORE_RULE, outcome)
    }
}

/// Builds an unsigned substance package naming `config`, `scorer`, and
/// `embedder` by content hash, with the builder of
/// `MinimalPolicyBundle::compatibility_package`: the compatibility
/// Admission, Review and Settle policies, the substance Score policy, and
/// `trace_credit` pinned to `config.novelty.instrument.descriptor`. The package names the scorer so
/// that it binds with the same runtime dependencies as a compatibility
/// package; the substance Score never calls it.
pub fn substance_package(
    config: &SubstanceScoreConfig,
    scorer: &dyn IdentifiedPerplexityScorer,
    embedder: &dyn IdentifiedEmbedder,
) -> anyhow::Result<BundlePackage> {
    config.validate()?;
    compatibility_family_package(
        serde_json::to_vec(config)?,
        SUBSTANCE_SCORE_POLICY_ID,
        SUBSTANCE_SCORE_IMPLEMENTATION,
        &config.novelty.projection_id,
        &config.novelty.instrument.descriptor,
        scorer,
        embedder,
    )
}

/// The substance candidate package: [`SubstanceScoreConfig::local_reference`]
/// with the `NoveltyUtility` delta of the compatibility qualification
/// candidate, over the reference scorer and the reference embedder.
pub fn substance_candidate_package() -> anyhow::Result<BundlePackage> {
    let mut config = SubstanceScoreConfig::local_reference();
    config.novelty.novelty_utility_microcredits = SUBSTANCE_CANDIDATE_NOVELTY_UTILITY_MICROCREDITS;
    substance_package(
        &config,
        &ReferencePerplexityScorer::new(),
        &ReferenceEmbedder::new(),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{DateTime, TimeZone, Utc};
    use serde_json::{Value, json};
    use trace_commons_protocol::trace_contribution::{
        ConsentMetadata, ConsentScope, ContributorMetadata,
        DETERMINISTIC_REDACTION_PIPELINE_VERSION, IronclawTraceMetadata, OutcomeMetadata,
        PrivacyMetadata, ReplayMetadata, ResidualPiiRisk, SideEffectLevel,
        TRACE_CONTRIBUTION_POLICY_VERSION, TRACE_CONTRIBUTION_SCHEMA_VERSION, TraceCard,
        TraceChannel, TraceContributionEnvelope, TraceContributionEvent,
        TraceContributionEventType, TraceValueCard, ValueMetadata,
    };

    use super::*;
    use trace_commons_gate_api::pipeline::{
        AtomicUnits, InstrumentId, SealedIndexCommand, SealedIndexEntry,
    };
    use trace_commons_gate_api::{
        ChunkPerplexity, PerplexityResult, PerplexityScorer, VectorIndexWriter,
    };
    use trace_commons_gate_enclave::chunk_aggregate::aggregate_chunked_novelty;
    use trace_commons_gate_enclave::embedder::embed_chunk_mean_pooled;

    use crate::versioned_pipeline::{PipelineGateDecisionScoreValues, pipeline_tenant_storage_ref};
    use crate::versioned_pipeline_bundle::{
        MinimalPolicyBundle, PIPELINE_DEPENDENCY_MISSING_LABEL, PipelineTraceCreditEvent,
        dependency_content_hash, package_configuration_is_qualifiable, package_substance_config,
    };
    use crate::versioned_pipeline_compat::{
        COMPATIBILITY_ADMISSION_IMPLEMENTATION, COMPATIBILITY_REVIEW_IMPLEMENTATION,
        COMPATIBILITY_SETTLE_IMPLEMENTATION,
    };
    use crate::versioned_pipeline_index::IsolatedPipelineIndex;
    use crate::versioned_pipeline_score_registry::{
        SCORE_POLICY_IMPLEMENTATION_RESERVED_LABEL, ScorePolicyDependencies, ScorePolicyFactory,
        ScorePolicyRegistry, is_built_in_score_implementation,
    };

    const CLASSES: [ScoreWitnessClass; 4] = [
        ScoreWitnessClass::ProviderTeeFinalCall,
        ScoreWitnessClass::GatewayFinalCall,
        ScoreWitnessClass::Unattested,
        ScoreWitnessClass::LegacyV1,
    ];

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 23, 12, 0, 0).unwrap()
    }

    fn id(label: &str) -> Uuid {
        Uuid::new_v5(&Uuid::NAMESPACE_OID, label.as_bytes())
    }

    fn event(label: &str, event_type: TraceContributionEventType) -> TraceContributionEvent {
        TraceContributionEvent {
            event_id: id(label),
            parent_event_id: None,
            event_type,
            timestamp: at(),
            redacted_content: None,
            structured_payload: Value::Null,
            tool_name: None,
            tool_category: None,
            tool_call_id: None,
            latency_ms: None,
            token_counts: None,
            cost_usd: None,
            success: None,
            failure_modes: Vec::new(),
            side_effect: SideEffectLevel::None,
        }
    }

    fn prompt() -> TraceContributionEvent {
        let mut e = event("prompt", TraceContributionEventType::UserMessage);
        e.redacted_content = Some("fix the failing test".to_string());
        e
    }

    fn call(
        label: &str,
        tool: &str,
        payload: Value,
        call_id: Option<&str>,
    ) -> TraceContributionEvent {
        let mut e = event(label, TraceContributionEventType::ToolCall);
        e.tool_name = Some(tool.to_string());
        e.structured_payload = payload;
        e.tool_call_id = call_id.map(str::to_string);
        e
    }

    fn result(label: &str, call_id: Option<&str>, text: Option<&str>) -> TraceContributionEvent {
        let mut e = event(label, TraceContributionEventType::ToolResult);
        e.tool_call_id = call_id.map(str::to_string);
        e.redacted_content = text.map(str::to_string);
        e
    }

    fn args(value: Value) -> Value {
        json!({ "arguments": value })
    }

    fn envelope(events: Vec<TraceContributionEvent>) -> TraceContributionEnvelope {
        TraceContributionEnvelope {
            schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
            trace_id: id("trace"),
            submission_id: id("submission"),
            created_at: at(),
            ironclaw: IronclawTraceMetadata {
                version: "1".to_string(),
                engine_version: None,
                feature_flags: BTreeMap::new(),
                channel: TraceChannel::Cli,
                model_name: None,
            },
            consent: ConsentMetadata {
                policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
                scopes: vec![ConsentScope::DebuggingEvaluation],
                message_text_included: true,
                tool_payloads_included: false,
                correction_included: false,
                routing_metadata_included: false,
                revocable: true,
            },
            contributor: ContributorMetadata {
                pseudonymous_contributor_id: None,
                tenant_scope_ref: None,
                credit_account_ref: None,
                revocation_handle: id("handle"),
            },
            privacy: PrivacyMetadata {
                redaction_pipeline_version: DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
                redaction_counts: BTreeMap::new(),
                redaction_distinct_counts: BTreeMap::new(),
                privacy_filter_summary: None,
                pii_labels_present: Vec::new(),
                residual_pii_risk: ResidualPiiRisk::Low,
                redaction_hash: "sha256:placeholder".to_string(),
                warnings: Vec::new(),
            },
            events,
            outcome: OutcomeMetadata::default(),
            replay: ReplayMetadata {
                replayable: false,
                required_tools: Vec::new(),
                tool_manifest_hashes: BTreeMap::new(),
                expected_assertions: Vec::new(),
                replay_notes: Vec::new(),
            },
            embedding_analysis: None,
            value: ValueMetadata::default(),
            conversation_id: None,
            source_session: None,
            trace_card: TraceCard::default(),
            value_card: TraceValueCard::default(),
            hindsight: None,
            training_dynamics: None,
            process_evaluation: None,
        }
    }

    fn paired(n: usize, tool: &str, arguments: Value) -> Vec<TraceContributionEvent> {
        let call_id = format!("c{n}");
        vec![
            call(
                &format!("call{n}"),
                tool,
                args(arguments),
                Some(call_id.as_str()),
            ),
            result(&format!("result{n}"), Some(call_id.as_str()), Some("ok")),
        ]
    }

    fn one_paired() -> TraceContributionEnvelope {
        let mut events = vec![prompt()];
        events.extend(paired(0, "read", json!({"path": "a"})));
        envelope(events)
    }

    fn twenty_paired() -> TraceContributionEnvelope {
        let mut events = vec![prompt()];
        for n in 0..20 {
            events.extend(paired(n, &format!("tool{}", n % 8), json!({ "n": n })));
        }
        envelope(events)
    }

    fn withheld() -> TraceContributionEnvelope {
        let marker = json!({"has_arguments": true, "has_result": true, "has_error": false});
        let mut events = vec![prompt()];
        for n in 0..3 {
            let call_id = format!("c{n}");
            events.push(call(
                &format!("call{n}"),
                "read",
                marker.clone(),
                Some(call_id.as_str()),
            ));
            events.push(result(&format!("result{n}"), Some(call_id.as_str()), None));
        }
        envelope(events)
    }

    fn one_paired_one_bare() -> TraceContributionEnvelope {
        let mut events = vec![prompt()];
        events.extend(paired(0, "read", json!({"path": "a"})));
        events.push(call("bare", "grep", Value::Null, None));
        envelope(events)
    }

    fn prompt_only() -> TraceContributionEnvelope {
        envelope(vec![prompt()])
    }

    fn goldens() -> Vec<TraceContributionEnvelope> {
        vec![
            envelope(Vec::new()),
            prompt_only(),
            one_paired(),
            twenty_paired(),
            withheld(),
            one_paired_one_bare(),
        ]
    }

    fn golden(envelope: &TraceContributionEnvelope, class: ScoreWitnessClass) -> (u64, u64, u64) {
        let params = SubstanceParams::rlvr_v1();
        let terms = substance_terms(&project_tool_calls(envelope), class, &params);
        (
            to_micros(terms.work),
            to_micros(terms.replay),
            to_micros(composite(&terms, &params)),
        )
    }

    fn substance(envelope: &TraceContributionEnvelope, class: ScoreWitnessClass) -> f64 {
        let params = SubstanceParams::rlvr_v1();
        let terms = substance_terms(&project_tool_calls(envelope), class, &params);
        composite(&terms, &params)
    }

    #[test]
    fn golden_empty_unattested() {
        assert_eq!(
            golden(&envelope(Vec::new()), ScoreWitnessClass::Unattested),
            (0, 0, 27_000)
        );
    }

    #[test]
    fn golden_prompt_only_unattested() {
        assert_eq!(
            golden(&prompt_only(), ScoreWitnessClass::Unattested),
            (0, 400_000, 124_200)
        );
    }

    #[test]
    fn golden_one_paired_call_unattested() {
        assert_eq!(
            golden(&one_paired(), ScoreWitnessClass::Unattested),
            (262_788, 1_000_000, 435_557)
        );
    }

    #[test]
    fn golden_one_paired_call_provider_tee() {
        assert_eq!(
            golden(&one_paired(), ScoreWitnessClass::ProviderTeeFinalCall),
            (262_788, 1_000_000, 483_952)
        );
    }

    #[test]
    fn golden_twenty_paired_eight_tools_gateway() {
        assert_eq!(
            golden(&twenty_paired(), ScoreWitnessClass::GatewayFinalCall),
            (1_000_000, 1_000_000, 1_000_000)
        );
    }

    #[test]
    fn golden_withheld_markers_unattested() {
        let envelope = withheld();
        assert_eq!(project_tool_calls(&envelope).tool_calls, 1);
        assert_eq!(
            golden(&envelope, ScoreWitnessClass::Unattested),
            (0, 400_000, 124_200)
        );
    }

    #[test]
    fn golden_one_paired_one_bare_unattested() {
        assert_eq!(
            golden(&one_paired_one_bare(), ScoreWitnessClass::Unattested),
            (262_788, 700_000, 317_956)
        );
    }

    #[test]
    fn golden_legacy_v1_is_unattested() {
        assert_eq!(
            golden(&one_paired(), ScoreWitnessClass::LegacyV1),
            (262_788, 1_000_000, 435_557)
        );
    }

    #[test]
    fn dedup_ignores_argument_key_order() {
        let mut events = vec![prompt()];
        events.extend(paired(0, "read", json!({"a": 1, "b": 2})));
        events.extend(paired(1, "read", json!({"b": 2, "a": 1})));
        let projection = project_tool_calls(&envelope(events));
        assert_eq!(projection.tool_calls, 1);
        assert_eq!(projection.paired_calls, 1);
    }

    #[test]
    fn a_varied_argument_still_counts() {
        let mut events = vec![prompt()];
        events.extend(paired(0, "read", json!({"n": 1})));
        events.extend(paired(1, "read", json!({"n": 2})));
        let projection = project_tool_calls(&envelope(events));
        assert_eq!(projection.paired_calls, 2);
        assert_eq!(projection.distinct_tools, 1);
    }

    #[test]
    fn orphan_and_repeated_results_pair_once() {
        let target = call("call", "read", args(json!({"path": "a"})), Some("c1"));
        let mut early = result("early", None, Some("early"));
        early.parent_event_id = Some(target.event_id);
        let events = vec![
            prompt(),
            early,
            target,
            call("call2", "read", args(json!({"path": "b"})), Some("c2")),
            call("call3", "read", args(json!({"path": "c"})), None),
            result("r1", Some("c1"), Some("one")),
            result("r2", Some("c1"), Some("two")),
            result("r9", Some("c9"), Some("orphan")),
        ];
        let projection = project_tool_calls(&envelope(events));
        assert_eq!(projection.tool_calls, 3);
        assert_eq!(projection.paired_calls, 1);
        assert_eq!(projection.calls_with_results, 1);
    }

    #[test]
    fn results_match_by_parent_event_id_without_ids() {
        let target = call("call", "read", args(json!({"path": "a"})), None);
        let mut reply = result("reply", None, Some("ok"));
        reply.parent_event_id = Some(target.event_id);
        let projection = project_tool_calls(&envelope(vec![prompt(), target, reply]));
        assert_eq!(projection.paired_calls, 1);
        assert_eq!(projection.calls_with_results, 1);
    }

    #[test]
    fn a_tool_free_trace_earns_at_most_the_prompt_share() {
        let params = SubstanceParams::rlvr_v1();
        let projection = project_tool_calls(&prompt_only());
        let terms = substance_terms(&projection, ScoreWitnessClass::Unattested, &params);
        assert!((terms.replay - 0.4).abs() < 1e-12);
    }

    #[test]
    fn more_paired_calls_never_lower_substance() {
        let mut events = vec![prompt()];
        let mut previous = substance(&envelope(events.clone()), ScoreWitnessClass::Unattested);
        for n in 1..=40usize {
            events.extend(paired(n, &format!("tool{}", n / 3), json!({ "n": n })));
            let current = substance(&envelope(events.clone()), ScoreWitnessClass::Unattested);
            assert!(current >= previous, "substance fell at n = {n}");
            previous = current;
        }
    }

    #[test]
    fn no_term_vetoes() {
        let params = SubstanceParams::rlvr_v1();
        for envelope in goldens() {
            for class in CLASSES {
                let terms = substance_terms(&project_tool_calls(&envelope), class, &params);
                assert!(composite(&terms, &params) >= 0.3 * 0.1 * 0.9 - 1e-12);
            }
        }
    }

    #[test]
    fn rlvr_v1_matches_the_spec_table() {
        let params = SubstanceParams::rlvr_v1();
        assert_eq!(params.pc_ceil, 20);
        assert_eq!(params.dt_ceil, 8);
        assert_eq!(params.w_pc_micros, 600_000);
        assert_eq!(params.w_dt_micros, 400_000);
        assert_eq!(params.w_hp_micros, 400_000);
        assert_eq!(params.w_sa_micros, 300_000);
        assert_eq!(params.w_sr_micros, 300_000);
        assert_eq!(params.m_work_micros, 300_000);
        assert_eq!(params.m_rep_micros, 100_000);
        assert_eq!(params.att_floor_mult_micros, 900_000);
        assert_eq!(params.validate(), Ok(()));
    }

    #[test]
    fn params_refuse_a_zero_floor() {
        let mut params = SubstanceParams::rlvr_v1();
        params.m_rep_micros = 0;
        assert_eq!(params.validate(), Err("substance_params_invalid"));
    }

    #[test]
    fn estimated_chunk_tokens_uses_four_chars() {
        let chunk = |text: &str| TraceChunk {
            chunk_index: 0,
            text: text.to_string(),
            spans: Vec::new(),
        };
        assert_eq!(estimated_chunk_tokens(&chunk("abcdefghi")), 3);
        assert_eq!(estimated_chunk_tokens(&chunk("")), 1);
        assert_eq!(estimated_chunk_tokens(&chunk("abcdefgh")), 2);
        // Characters, not bytes: nine two-byte characters are still 3.
        assert_eq!(estimated_chunk_tokens(&chunk("ééééééééé")), 3);
    }

    // ---- the substance Score policy -----------------------------------

    /// The row-3 golden envelope (`one_paired`) as the reviewed artifact.
    fn row_three_artifact() -> Vec<u8> {
        serde_json::to_vec(&one_paired()).unwrap()
    }

    fn substance_input(bytes: &[u8]) -> ScoreInput {
        ScoreInput {
            run_id: Uuid::from_u128(1),
            tenant_storage_ref: pipeline_tenant_storage_ref("tenant-a"),
            trace_id: Uuid::from_u128(2),
            registry_revision_id: Uuid::from_u128(3),
            source_content_hash: dependency_content_hash(bytes),
            reviewed_artifact: bytes.to_vec(),
            operator_review: None,
        }
    }

    /// A policy under the manifest `substance_package` builds for `config`,
    /// so `ScoreDecision::for_bundle` accepts its awards.
    fn substance_policy(
        config: &SubstanceScoreConfig,
        index: Arc<IsolatedPipelineIndex>,
    ) -> SubstanceScorePolicy {
        let manifest = substance_package(
            config,
            &ReferencePerplexityScorer::new(),
            &ReferenceEmbedder::new(),
        )
        .unwrap()
        .manifest;
        SubstanceScorePolicy::new(
            manifest,
            config.clone(),
            Arc::new(ReferenceEmbedder::new()),
            index,
        )
        .unwrap()
    }

    async fn score(
        config: &SubstanceScoreConfig,
        bytes: &[u8],
    ) -> Result<ScoreOutput, PolicyError> {
        substance_policy(config, IsolatedPipelineIndex::new())
            .execute(&substance_input(bytes))
            .await
    }

    /// A scorer that must never be called: both entry points panic, and a
    /// panic on the blocking pool fails the Score as `score_task_failed`.
    struct PanickingScorer;

    impl PerplexityScorer for PanickingScorer {
        fn score(&self, _plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
            panic!("the substance Score must not call the scorer")
        }

        fn score_chunk(&self, _chunk: &[u8]) -> anyhow::Result<ChunkPerplexity> {
            panic!("the substance Score must not call the scorer")
        }
    }

    impl IdentifiedPerplexityScorer for PanickingScorer {
        fn dependency_identity(&self) -> &str {
            "panicking_scorer_test_only"
        }
        fn content_descriptor(&self) -> Vec<u8> {
            b"panicking-scorer-test-only.v1".to_vec()
        }
    }

    /// Review Focus 4: a reviewed artifact that is not an envelope is a
    /// permanent failure, never a retry and never a score.
    #[tokio::test]
    async fn substance_score_of_a_non_envelope_is_permanent() {
        let config = SubstanceScoreConfig::local_reference();
        assert_eq!(
            score(&config, b"not json").await.err(),
            Some(PolicyError::permanent("substance_envelope_invalid").unwrap())
        );
        assert_eq!(
            score(&config, br#"{"events":[]}"#).await.err(),
            Some(PolicyError::permanent("substance_envelope_invalid").unwrap()),
            "JSON that is not an envelope is not one either"
        );
        assert_eq!(
            score(&config, b"").await.err(),
            Some(PolicyError::permanent("score_input_empty").unwrap())
        );
    }

    #[tokio::test]
    async fn substance_score_fills_its_evidence() {
        let config = SubstanceScoreConfig::local_reference();
        let output = score(&config, &row_three_artifact()).await.unwrap();
        let result = output.result();
        let evidence = &result.evidence;
        assert_eq!(evidence.substance_micros, Some(435_557));
        assert_eq!(evidence.substance_work_micros, Some(262_788));
        assert_eq!(evidence.substance_replay_micros, Some(1_000_000));
        assert_eq!(evidence.substance_tool_calls, Some(1));
        assert_eq!(evidence.substance_version, Some(1));
        assert_eq!(
            evidence.witness_provenance_class,
            Some(ScoreWitnessClass::Unattested)
        );
        assert_eq!(evidence.credit_quality_version, Some(4));
        assert_eq!(evidence.substance_passed, Some(true));
        assert_eq!(evidence.quality_passed, evidence.substance_passed);
        assert_eq!(evidence.perplexity_micros, Some(0));
        assert_eq!(evidence.tail_fraction_micros, Some(0));
        assert_eq!(evidence.scorer_model_id, None);
        assert_eq!(evidence.peak_perplexity_micros, None);
        assert_eq!(result.evaluation.rule_id, "substance_novelty_v1");
        let novelty = evidence.novelty_score_micros.expect("novelty is filled");
        assert_eq!(
            evidence.credit_quality_micros,
            Some(substance_credit_quality(
                435_557,
                i64::try_from(novelty).unwrap(),
                config.sub_ceil_micros,
                config.sub_floor_mult_micros,
                &config.novelty.credit,
            ))
        );
        assert!(evidence.novelty_passed.is_some());
        assert!(evidence.nearest_neighbor_hash.is_some());
        assert!(evidence.index_snapshot_hash.is_some());
        assert!(evidence.chunk_count.is_some());
        assert!(evidence.coverage_tokens.is_some());
        assert_eq!(
            evidence.include_eligible,
            Some(output.index_command().is_some())
        );
    }

    /// SQ-13: while the witness is not read, every trace scores and records
    /// as unattested, never with the attested factor.
    #[tokio::test]
    async fn substance_score_records_unattested_while_the_witness_is_not_read() {
        let config = SubstanceScoreConfig::local_reference();
        assert_eq!(config.witness_source, SubstanceWitnessSource::NotRead);
        let output = score(&config, &row_three_artifact()).await.unwrap();
        let evidence = &output.result().evidence;
        assert_eq!(
            evidence.witness_provenance_class,
            Some(ScoreWitnessClass::Unattested)
        );
        assert_eq!(evidence.substance_micros, Some(435_557));
        assert_ne!(evidence.substance_micros, Some(483_952));
    }

    #[tokio::test]
    async fn substance_gate_uses_the_floor() {
        let mut config = SubstanceScoreConfig::local_reference();
        config.novelty.novelty_utility_microcredits =
            SUBSTANCE_CANDIDATE_NOVELTY_UTILITY_MICROCREDITS;

        config.substance_floor_micros = 435_557;
        let passed = score(&config, &row_three_artifact()).await.unwrap();
        let evidence = &passed.result().evidence;
        assert_eq!(evidence.substance_passed, Some(true));
        assert_eq!(evidence.novelty_passed, Some(true));
        assert!(passed.index_command().is_some());
        assert!(!passed.result().decision.awards().is_empty());

        config.substance_floor_micros = 435_558;
        let failed = score(&config, &row_three_artifact()).await.unwrap();
        let result = failed.result();
        assert_eq!(result.evidence.substance_passed, Some(false));
        assert_eq!(result.evidence.quality_passed, Some(false));
        assert_eq!(result.evidence.novelty_passed, Some(true));
        assert!(failed.index_command().is_none());
        assert_eq!(result.evidence.include_eligible, Some(false));
        assert_eq!(result.evidence.embedding_artifact_hash, None);
        assert!(result.decision.awards().is_empty());
        assert!(result.evaluation.awards.is_empty());
        assert!(result.evidence.fixed_awards.is_empty());
    }

    /// Spec section 8.3, Settle: the legacy gate-decision row builds from
    /// substance evidence, with the perplexity sentinel and `cq4`.
    #[tokio::test]
    async fn substance_legacy_row_fields_are_complete() {
        let config = SubstanceScoreConfig::local_reference();
        let output = score(&config, &row_three_artifact()).await.unwrap();
        let evidence = &output.result().evidence;
        let values = PipelineGateDecisionScoreValues::from_evidence(evidence)
            .expect("the legacy row builds from substance evidence");
        assert_eq!(values.perplexity_micros, 0);
        assert_eq!(values.tail_fraction_micros, 0);
        assert_eq!(
            values.perplexity_passed,
            evidence
                .substance_passed
                .expect("substance_passed is filled")
        );
        assert_eq!(values.credit_quality_version, Some(4));
    }

    #[tokio::test]
    async fn substance_score_never_calls_the_scorer() {
        let config = SubstanceScoreConfig::local_reference();
        let package = substance_package(&config, &PanickingScorer, &ReferenceEmbedder::new())
            .expect("a package names the panicking scorer");
        let bundle = MinimalPolicyBundle::from_package_with_runtime(
            package,
            Arc::new(PanickingScorer),
            Arc::new(ReferenceEmbedder::new()),
            IsolatedPipelineIndex::new(),
        )
        .expect("the package binds");
        let output = bundle
            .score
            .execute(&substance_input(&row_three_artifact()))
            .await
            .expect("the Score never reaches the panicking scorer");
        assert_eq!(output.result().evidence.substance_micros, Some(435_557));
    }

    /// Repeated words up to exactly `len` characters.
    fn text_of(len: usize, words: &str) -> String {
        words
            .split(' ')
            .cycle()
            .flat_map(|word| word.chars().chain(std::iter::once(' ')))
            .take(len)
            .collect()
    }

    /// SQ-3: the novelty aggregate weights each chunk by the character
    /// estimate, never by a scorer's token count.
    #[tokio::test]
    async fn novelty_weights_come_from_the_character_estimate() {
        let mut config = SubstanceScoreConfig::local_reference();
        config.novelty.chunk_target_tokens = 100;
        config.novelty.chunk_max_tokens = 100;
        // `user_message: ` (14) + content + `\n` (1): 40 and 400 characters.
        let mut short = event("short", TraceContributionEventType::UserMessage);
        short.redacted_content = Some(text_of(25, "alpha beta gamma"));
        let mut long = event("long", TraceContributionEventType::UserMessage);
        long.redacted_content = Some(text_of(385, "zulu yankee xray whiskey victor"));
        let bytes = serde_json::to_vec(&envelope(vec![short, long])).unwrap();
        let input = substance_input(&bytes);

        let plan = chunk_envelope_plaintext(&bytes, &config.novelty.chunker());
        assert_eq!(
            plan.chunks
                .iter()
                .map(|chunk| chunk.text.chars().count())
                .collect::<Vec<_>>(),
            vec![40, 400]
        );
        let weights: Vec<u64> = plan.chunks.iter().map(estimated_chunk_tokens).collect();
        assert_eq!(weights, vec![10, 100]);

        // Seed the index with the first chunk, so the two chunks' novelty
        // differ and the weights decide the aggregate.
        let embedder = ReferenceEmbedder::new();
        let index = IsolatedPipelineIndex::new();
        let seed = SealedIndexCommand::new(
            &config.novelty.index_id,
            Uuid::from_u128(9),
            &config.novelty.projection_id,
            embedder.model_id(),
            vec![SealedIndexEntry {
                chunk: 0,
                content_hash: dependency_content_hash(plan.chunks[0].text.as_bytes()),
                embedding: embed_chunk_mean_pooled(&embedder, &plan.chunks[0].text).unwrap(),
            }],
        )
        .unwrap();
        let (key, entry) = seed
            .keyed_entries(&input.tenant_storage_ref)
            .next()
            .unwrap();
        index
            .upsert(&key, &entry.embedding, &entry.content_hash)
            .unwrap();

        let per_chunk = evaluate_novelty(
            &embedder,
            index.as_ref(),
            &input.tenant_storage_ref,
            input.registry_revision_id,
            &plan.chunks,
            &weights,
            &config.novelty.novelty_scoring(),
        )
        .unwrap()
        .chunk_novelty_micros;
        assert_ne!(per_chunk[0], per_chunk[1], "the seeded chunk is not novel");
        let (expected, _) =
            aggregate_chunked_novelty(&per_chunk, &[10, 100], config.novelty.chunk_min_tokens);
        let (unweighted, _) =
            aggregate_chunked_novelty(&per_chunk, &[1, 1], config.novelty.chunk_min_tokens);
        assert_ne!(expected, unweighted, "the weights must matter");

        let output = substance_policy(&config, index)
            .execute(&input)
            .await
            .unwrap();
        let evidence = &output.result().evidence;
        assert_eq!(evidence.novelty_score_micros, Some(expected));
        assert_eq!(evidence.coverage_tokens, Some(110));
    }

    #[test]
    fn substance_config_refuses_out_of_range_values() {
        let refused = |change: fn(&mut SubstanceScoreConfig)| {
            let mut config = SubstanceScoreConfig::local_reference();
            change(&mut config);
            let error = config.validate().expect_err("the change is refused");
            assert_eq!(error.to_string(), PIPELINE_BUNDLE_INVALID_LABEL);
            assert!(
                SubstanceScorePolicy::new(
                    substance_candidate_package().unwrap().manifest,
                    config,
                    Arc::new(ReferenceEmbedder::new()),
                    IsolatedPipelineIndex::new(),
                )
                .is_err(),
                "the policy refuses what validate refuses"
            );
        };
        refused(|config| config.sub_ceil_micros = 0);
        refused(|config| config.sub_ceil_micros = 1_000_001);
        refused(|config| config.substance_floor_micros = 1_000_001);
        refused(|config| config.sub_floor_mult_micros = 1_000_001);
        refused(|config| config.substance_version = SUBSTANCE_VERSION + 1);
        refused(|config| config.params.pc_ceil = 0);
        refused(|config| config.params.m_work_micros = 0);
        refused(|config| config.novelty.chunk_target_tokens = 0);
        refused(|config| config.novelty.credit.nov_ceil_micros = 0);
        refused(|config| {
            config.novelty.novelty_utility_microcredits = MAX_TRACE_CREDIT_MICROCREDITS + 1
        });
        assert!(SubstanceScoreConfig::local_reference().validate().is_ok());
    }

    // ---- the substance bundle ------------------------------------------

    fn bind(
        package: BundlePackage,
        scorer: Arc<dyn IdentifiedPerplexityScorer>,
    ) -> anyhow::Result<MinimalPolicyBundle> {
        MinimalPolicyBundle::from_package_with_runtime(
            package,
            scorer,
            Arc::new(ReferenceEmbedder::new()),
            IsolatedPipelineIndex::new(),
        )
    }

    #[tokio::test]
    async fn substance_package_binds_with_the_compatibility_stages() {
        let package = substance_candidate_package().unwrap();
        let manifest = &package.manifest;
        assert_eq!(
            manifest.admission.implementation_id,
            COMPATIBILITY_ADMISSION_IMPLEMENTATION
        );
        assert_eq!(
            manifest.review.implementation_id,
            COMPATIBILITY_REVIEW_IMPLEMENTATION
        );
        assert_eq!(
            manifest.settle.implementation_id,
            COMPATIBILITY_SETTLE_IMPLEMENTATION
        );
        assert_eq!(
            manifest.score.implementation_id,
            SUBSTANCE_SCORE_IMPLEMENTATION
        );
        assert_eq!(manifest.score.policy_id, "trace_commons.score.substance");
        let config = SubstanceScoreConfig::local_reference();
        assert_eq!(
            manifest.score.projection_ids,
            vec![config.novelty.projection_id.clone()]
        );
        assert_eq!(
            manifest.instruments.get(&InstrumentId::trace_credit()),
            Some(&config.novelty.instrument.descriptor)
        );

        let bundle = bind(package.clone(), Arc::new(ReferencePerplexityScorer::new()))
            .expect("the substance package binds");
        assert_eq!(
            bundle.trace_credit_event,
            PipelineTraceCreditEvent::NoveltyUtility
        );
        assert!(bundle.score_runs_under_runtime_controls());

        // The candidate's award flows through under the pinned descriptor.
        let output = bundle
            .score
            .execute(&substance_input(&row_three_artifact()))
            .await
            .unwrap();
        let decision = &output.result().decision;
        decision
            .require_bound(&package.manifest)
            .expect("the award is bound to the package's manifest");
        assert_eq!(
            decision
                .awards()
                .iter()
                .map(|award| (
                    award.instrument_id().as_str().to_string(),
                    award.atomic_units()
                ))
                .collect::<Vec<_>>(),
            vec![(
                "trace_credit".to_string(),
                AtomicUnits::from_raw(u128::from(SUBSTANCE_CANDIDATE_NOVELTY_UTILITY_MICROCREDITS))
            )]
        );
    }

    #[test]
    fn substance_package_refuses_a_changed_scorer_descriptor() {
        let error = bind(
            substance_candidate_package().unwrap(),
            Arc::new(PanickingScorer),
        )
        .err()
        .expect("a scorer the package does not name must not bind");
        assert_eq!(error.to_string(), PIPELINE_DEPENDENCY_MISSING_LABEL);
        assert_eq!(error.to_string(), "bundle_dependency_missing");
    }

    #[test]
    fn substance_package_refuses_an_unpinned_trace_credit() {
        let mut unpinned = substance_candidate_package().unwrap();
        unpinned
            .manifest
            .instruments
            .remove(&InstrumentId::trace_credit());
        unpinned.bundle_id = unpinned.manifest.bundle_id().unwrap();
        let error = bind(unpinned, Arc::new(ReferencePerplexityScorer::new()))
            .err()
            .expect("a package with no trace_credit pin must not bind");
        assert_eq!(error.to_string(), PIPELINE_BUNDLE_INVALID_LABEL);

        let mut mismatched = substance_candidate_package().unwrap();
        let mut descriptor = SubstanceScoreConfig::local_reference()
            .novelty
            .instrument
            .descriptor;
        descriptor.contract = "different-trace-credit.testnet".to_string();
        mismatched
            .manifest
            .instruments
            .insert(InstrumentId::trace_credit(), descriptor);
        mismatched.bundle_id = mismatched.manifest.bundle_id().unwrap();
        let error = bind(mismatched, Arc::new(ReferencePerplexityScorer::new()))
            .err()
            .expect("a package pinning another descriptor must not bind");
        assert_eq!(error.to_string(), PIPELINE_BUNDLE_INVALID_LABEL);
    }

    struct NeverBuiltFactory;

    impl ScorePolicyFactory for NeverBuiltFactory {
        fn dependency_identity(&self) -> &str {
            "never_built_factory_test_only"
        }

        fn trace_credit_event(&self) -> PipelineTraceCreditEvent {
            PipelineTraceCreditEvent::NoveltyUtility
        }

        fn build(
            &self,
            _dependencies: ScorePolicyDependencies<'_>,
        ) -> Result<Arc<dyn ScorePolicy>, &'static str> {
            Err("never_built_factory_test_only")
        }
    }

    /// SQ-1: substance is built in, so no registry can override it.
    #[test]
    fn substance_id_cannot_be_registered() {
        assert!(is_built_in_score_implementation(
            SUBSTANCE_SCORE_IMPLEMENTATION
        ));
        assert_eq!(
            ScorePolicyRegistry::new()
                .with_factory(SUBSTANCE_SCORE_IMPLEMENTATION, Arc::new(NeverBuiltFactory))
                .err(),
            Some(SCORE_POLICY_IMPLEMENTATION_RESERVED_LABEL)
        );
    }

    #[test]
    fn local_reference_is_not_qualifiable() {
        let config = SubstanceScoreConfig::local_reference();
        assert!(config.validate().is_ok());
        assert!(!config.is_qualifiable());
        assert!(!package_configuration_is_qualifiable(
            &substance_candidate_package().unwrap()
        ));
    }

    fn calibrated_values() -> CalibratedSubstanceValues {
        CalibratedSubstanceValues {
            sub_ceil_micros: 600_000,
            substance_floor_micros: 300_000,
            calibration: SubstanceCalibration {
                corpus_pin_digest: format!("sha256:{}", "a".repeat(64)),
                report_digest: format!("sha256:{}", "b".repeat(64)),
                sub_ceil_percentile: 90,
                substance_floor_percentile: 25,
            },
        }
    }

    fn production(values: &CalibratedSubstanceValues) -> SubstanceScoreConfig {
        let mut config = SubstanceScoreConfig::local_reference();
        config.qualification = CompatibilityQualification::ProductionCompatible;
        config.with_calibrated(values)
    }

    #[test]
    fn a_calibrated_production_config_is_qualifiable() {
        let values = calibrated_values();
        let config = production(&values);
        assert_eq!(config.sub_ceil_micros, 600_000);
        assert_eq!(config.substance_floor_micros, 300_000);
        assert_eq!(config.calibration, Some(values.calibration.clone()));
        assert!(config.is_qualifiable());
        let package = substance_package(
            &config,
            &ReferencePerplexityScorer::new(),
            &ReferenceEmbedder::new(),
        )
        .unwrap();
        assert_eq!(package_substance_config(&package), Some(config.clone()));
        assert!(package_configuration_is_qualifiable(&package));

        let mut uncalibrated = SubstanceScoreConfig::local_reference();
        uncalibrated.qualification = CompatibilityQualification::ProductionCompatible;
        uncalibrated.substance_floor_micros = 300_000;
        assert!(uncalibrated.validate().is_ok());
        assert!(!uncalibrated.is_qualifiable());

        let mut other_percentile = values.clone();
        other_percentile.calibration.sub_ceil_percentile = 80;
        assert!(!production(&other_percentile).is_qualifiable());

        let mut local = config.clone();
        local.qualification = CompatibilityQualification::LocalSyntheticNonQualifiable;
        assert!(!local.is_qualifiable());

        // A substance package whose configuration is missing or does not
        // decode is not qualifiable: it fails closed.
        let config_hash = package.manifest.score.configuration_hash.clone();
        let mut missing = package.clone();
        missing.artifacts.remove(&config_hash);
        let mut undecodable = package.clone();
        undecodable
            .artifacts
            .insert(config_hash, br#"{"unknown_field":1}"#.to_vec());
        for package in [&missing, &undecodable] {
            assert_eq!(package_substance_config(package), None);
            assert!(!package_configuration_is_qualifiable(package));
        }
    }

    /// As the compatibility guard (`compatibility_zero_floor`): a
    /// production-compatible configuration whose substance and novelty
    /// floors are both zero passes every trace, so it is refused. Either
    /// floor positive is accepted, and the local reference, whose floors
    /// are both zero, stays valid because it is not production-compatible.
    #[test]
    fn a_production_config_with_zero_floors_is_refused() {
        let mut values = calibrated_values();
        values.substance_floor_micros = 0;
        let zero = production(&values);
        assert_eq!(zero.novelty.novelty_floor_micros, 0);
        let error = zero.validate().expect_err("both floors zero is refused");
        assert_eq!(error.to_string(), COMPATIBILITY_ZERO_FLOOR_LABEL);
        assert!(!zero.is_qualifiable());
        assert!(
            substance_package(
                &zero,
                &ReferencePerplexityScorer::new(),
                &ReferenceEmbedder::new(),
            )
            .is_err(),
            "no package is built for it"
        );

        let mut novelty_only = zero.clone();
        novelty_only.novelty.novelty_floor_micros = 500_000;
        assert!(novelty_only.validate().is_ok());
        assert!(novelty_only.is_qualifiable());
        assert!(production(&calibrated_values()).validate().is_ok());

        let local = SubstanceScoreConfig::local_reference();
        assert_eq!(local.substance_floor_micros, 0);
        assert_eq!(local.novelty.novelty_floor_micros, 0);
        assert!(local.validate().is_ok());
    }

    #[test]
    fn substance_bundle_id_hashes_every_parameter() {
        let bundle_id = |config: &SubstanceScoreConfig| {
            substance_package(
                config,
                &ReferencePerplexityScorer::new(),
                &ReferenceEmbedder::new(),
            )
            .unwrap()
            .bundle_id
        };
        let base = SubstanceScoreConfig::local_reference();
        let mut weight = base.clone();
        weight.params.w_pc_micros += 1;
        let mut floor = base.clone();
        floor.substance_floor_micros += 1;
        assert_ne!(bundle_id(&base), bundle_id(&weight));
        assert_ne!(bundle_id(&base), bundle_id(&floor));
        assert_ne!(bundle_id(&weight), bundle_id(&floor));
        assert_eq!(bundle_id(&base), bundle_id(&base.clone()));
    }

    #[test]
    fn the_witness_source_serializes_as_not_read() {
        let config = SubstanceScoreConfig::local_reference();
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains(r#""witness_source":"not_read""#));
        let mut value = serde_json::to_value(&config).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("witness_source")
            .expect("the field is present");
        assert!(serde_json::from_value::<SubstanceScoreConfig>(value).is_err());
    }
}
