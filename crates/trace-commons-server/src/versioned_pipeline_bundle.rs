// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The minimal policy bundle for the versioned pipeline (#971).
//!
//! This module builds the smallest runnable policy set from a
//! [`BundlePackage`] and the dependencies the package names by content hash
//! (decision D8): a scorer, an embedder, and an index reader. The four
//! minimal implementations (`trace_commons.{admission,review,score,settle}.minimal.v1`)
//! are deliberately simple reference behavior, not production policy.
//!
//! Construction only: no store, no runner. Those arrive in later tasks.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use trace_commons_gate_api::pipeline::{
    AdmissionDecision, AdmissionEvaluation, AdmissionEvidence, AdmissionInput, AdmissionPolicy,
    ApprovedContent, AtomicUnits, BUNDLE_MANIFEST_FORMAT_VERSION, BundleManifest, BundlePackage,
    ContractError, IndexMembershipDecision, InstrumentAward, InstrumentAwards,
    InstrumentDescriptor, InstrumentId, InstrumentSettlement, PhaseResult, PolicyError, PolicyRef,
    PrivacyRisk, ReasonCode, ReviewDecision, ReviewEvaluation, ReviewEvidence, ReviewInput,
    ReviewOutput, ReviewPolicy, ReviewRecommendation, ScoreDecision, ScoreEvaluation,
    ScoreEvidence, ScoreInput, ScoreOutput, ScorePolicy, SealedIndexCommand, SealedIndexEntry,
    SettleDecision, SettleEvaluation, SettleEvidence, SettleInput, SettlePolicy,
};
use trace_commons_gate_api::{
    IdentifiedEmbedder, IdentifiedIndexReader, IdentifiedIndexWriter, IdentifiedPerplexityScorer,
};

use crate::versioned_pipeline_blocking::run_score_evaluation;
use crate::versioned_pipeline_compat::{
    COMPATIBILITY_ADMISSION_IMPLEMENTATION, COMPATIBILITY_REVIEW_IMPLEMENTATION,
    COMPATIBILITY_SCORE_IMPLEMENTATION, COMPATIBILITY_SETTLE_IMPLEMENTATION,
    CompatibilityBundleConfig, CompatibilityScorePolicy, CompatibilitySettlePolicy,
};
use crate::versioned_pipeline_index::IsolatedPipelineIndex;

/// Test-index identity the minimal Score policy proposes entries against.
pub const MINIMAL_INDEX_ID: &str = "pipeline-test-index-v1";
/// Test-projection identity the minimal Score policy embeds under.
pub const MINIMAL_PROJECTION_ID: &str = "pipeline-test-projection-v1";
/// Chunk width the minimal Score policy splits the reviewed artifact into.
pub const MINIMAL_INDEX_CHUNK_BYTES: usize = 256;
/// Safe label for a package that fails its own manifest/artifact validation.
pub const PIPELINE_BUNDLE_INVALID_LABEL: &str = "bundle_package_invalid";
/// Safe label for a dependency whose descriptor the package does not name.
pub const PIPELINE_DEPENDENCY_MISSING_LABEL: &str = "bundle_dependency_missing";
/// Safe label for a Review quarantine with no human assessment yet. In the
/// Review phase this label parks the run in
/// `PipelineRunState::AwaitingReview` instead of retrying it hourly forever.
pub const PIPELINE_REVIEW_ASSESSMENT_REQUIRED_LABEL: &str = "review_assessment_required";

impl IdentifiedIndexReader for IsolatedPipelineIndex {
    fn dependency_identity(&self) -> &str {
        "isolated_index_reader_test_only"
    }
}

impl IdentifiedIndexWriter for IsolatedPipelineIndex {
    fn dependency_identity(&self) -> &str {
        "isolated_index_writer_test_only"
    }
}

pub struct MinimalAdmissionPolicy;

#[async_trait]
impl AdmissionPolicy for MinimalAdmissionPolicy {
    async fn execute(
        &self,
        input: &AdmissionInput,
    ) -> Result<PhaseResult<AdmissionDecision, AdmissionEvidence, AdmissionEvaluation>, PolicyError>
    {
        if !input.authenticated || !input.authority_valid {
            return Err(PolicyError::permanent("authority_missing").expect("static label"));
        }
        let (decision, schema_valid, reason) = if input.schema_version
            != "ironclaw.trace_contribution.v1"
        {
            (
                AdmissionDecision::Reject {
                    reason: ReasonCode::new("schema_invalid").expect("static label"),
                },
                false,
                Some("schema_invalid"),
            )
        } else if input.tombstoned {
            (
                AdmissionDecision::Reject {
                    reason: ReasonCode::new("content_tombstoned").expect("static label"),
                },
                true,
                Some("content_tombstoned"),
            )
        } else if !input.contribution_path_valid {
            (
                AdmissionDecision::Reject {
                    reason: ReasonCode::new("contribution_path_invalid").expect("static label"),
                },
                true,
                Some("contribution_path_invalid"),
            )
        } else if !input.grant_valid {
            (
                AdmissionDecision::Reject {
                    reason: ReasonCode::new("grant_invalid").expect("static label"),
                },
                true,
                Some("grant_invalid"),
            )
        } else if !input.consent_valid {
            (
                AdmissionDecision::Reject {
                    reason: ReasonCode::new("consent_invalid").expect("static label"),
                },
                true,
                Some("consent_invalid"),
            )
        } else if !input.allowed_uses_valid {
            (
                AdmissionDecision::Reject {
                    reason: ReasonCode::new("allowed_use_invalid").expect("static label"),
                },
                true,
                Some("allowed_use_invalid"),
            )
        } else if !input.quota_available {
            (
                AdmissionDecision::Reject {
                    reason: ReasonCode::new("admission_limit_exceeded").expect("static label"),
                },
                true,
                Some("admission_limit_exceeded"),
            )
        } else {
            match input.privacy_risk {
                PrivacyRisk::Low => (AdmissionDecision::Admit, true, None),
                PrivacyRisk::Medium => (
                    AdmissionDecision::Quarantine {
                        reason: ReasonCode::new("privacy_review_required").expect("static label"),
                    },
                    true,
                    Some("privacy_review_required"),
                ),
                // Owner decision PC-D27 (2026-10-11): a receipt-time High is
                // held for a human like a Medium, never rejected here -- as
                // `main`'s legacy path quarantines it and the Review-start
                // privacy pass holds a High it finds (D1). Its own label
                // lets an operator tell it from a Medium hold.
                PrivacyRisk::High => (
                    AdmissionDecision::Quarantine {
                        reason: ReasonCode::new("privacy_risk_high_review_required")
                            .expect("static label"),
                    },
                    true,
                    Some("privacy_risk_high_review_required"),
                ),
            }
        };
        Ok(PhaseResult {
            decision,
            evidence: AdmissionEvidence {
                request_content_hash: input.request_content_hash.clone(),
                schema_valid,
                authority_valid: true,
                contribution_path_valid: input.contribution_path_valid,
                grant_valid: input.grant_valid,
                consent_valid: input.consent_valid,
                allowed_uses_valid: input.allowed_uses_valid,
                quota_counted: input.quota_available,
                detector_ids: Vec::new(),
                privacy_risk: Some(input.privacy_risk),
            },
            evaluation: AdmissionEvaluation {
                rule_id: reason.unwrap_or("minimal_admission_v1").to_string(),
            },
        })
    }
}

fn review_invalid<E>(_: E) -> PolicyError {
    PolicyError::permanent("review_output_invalid").expect("static label")
}

pub struct MinimalReviewPolicy;

#[async_trait]
impl ReviewPolicy for MinimalReviewPolicy {
    async fn execute(&self, input: &ReviewInput) -> Result<ReviewOutput, PolicyError> {
        let result_hash = dependency_content_hash(&input.source_artifact);
        if result_hash != input.source_content_hash {
            return Err(PolicyError::permanent("source_hash_mismatch").expect("static label"));
        }
        let (assessment_hash, resolved_quarantine_reasons) = match &input.admission {
            AdmissionDecision::Admit => (None, Vec::new()),
            AdmissionDecision::Reject { .. } => {
                return Err(PolicyError::permanent("admission_rejected").expect("static label"));
            }
            AdmissionDecision::Quarantine { reason } => {
                // Human assessments arrive in PR 3 (decision D9); until then a
                // quarantine can only be retried, not resolved.
                let assessment = input.human_assessment.as_ref().ok_or_else(|| {
                    PolicyError::transient(PIPELINE_REVIEW_ASSESSMENT_REQUIRED_LABEL)
                        .expect("static label")
                })?;
                if assessment.recommendation == ReviewRecommendation::Reject {
                    let result = PhaseResult {
                        decision: ReviewDecision::Rejected {
                            reason: assessment.reason.clone(),
                        },
                        evidence: ReviewEvidence {
                            source_content_hash: input.source_content_hash.clone(),
                            result_content_hash: result_hash,
                            content_changed: false,
                            worker_identity: None,
                            transformation_metadata_hash: None,
                            human_assessment_hash: Some(assessment.evidence_hash.clone()),
                            resolved_quarantine_reasons: Vec::new(),
                        },
                        evaluation: ReviewEvaluation {
                            rule_id: "human_review_rejected_v1".to_string(),
                        },
                    };
                    return ReviewOutput::rejected(result).map_err(review_invalid);
                }
                if !assessment
                    .resolved_quarantine_reasons
                    .iter()
                    .any(|resolved| resolved == reason)
                {
                    return Err(
                        PolicyError::permanent("review_resolution_missing").expect("static label")
                    );
                }
                (
                    Some(assessment.evidence_hash.clone()),
                    assessment.resolved_quarantine_reasons.clone(),
                )
            }
        };
        let registry_revision_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("tracecommons:pipeline-review:{}", input.run_id).as_bytes(),
        );
        let result = PhaseResult {
            decision: ReviewDecision::Approved {
                registry_revision_id,
            },
            evidence: ReviewEvidence {
                source_content_hash: input.source_content_hash.clone(),
                result_content_hash: result_hash,
                content_changed: false,
                worker_identity: Some("minimal_review_passthrough".to_string()),
                transformation_metadata_hash: None,
                human_assessment_hash: assessment_hash,
                resolved_quarantine_reasons,
            },
            evaluation: ReviewEvaluation {
                rule_id: "minimal_review_passthrough_v1".to_string(),
            },
        };
        ReviewOutput::approved(
            result,
            ApprovedContent::new(
                input.source_artifact.clone(),
                "minimal_review_passthrough",
                None,
            )
            .map_err(review_invalid)?,
        )
        .map_err(review_invalid)
    }
}

pub fn dependency_content_hash(descriptor: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(descriptor))
}

pub fn pipeline_operation_ref(run_id: Uuid, award: &InstrumentAward) -> String {
    dependency_content_hash(
        format!(
            "pipeline-operation-v1:{run_id}:{}:{}",
            award.instrument_id().as_str(),
            award.atomic_units().get()
        )
        .as_bytes(),
    )
}

pub fn pipeline_result_ref(run_id: Uuid, award: &InstrumentAward) -> String {
    dependency_content_hash(
        format!(
            "pipeline-result-v1:{run_id}:{}:{}",
            award.instrument_id().as_str(),
            award.atomic_units().get()
        )
        .as_bytes(),
    )
}

pub fn settlement_operations(
    run_id: Uuid,
    awards: &InstrumentAwards,
) -> Result<Vec<InstrumentSettlement>, ContractError> {
    awards
        .iter()
        .map(|award| {
            InstrumentSettlement::new(
                award.instrument_id().clone(),
                award.atomic_units(),
                pipeline_operation_ref(run_id, award),
                pipeline_result_ref(run_id, award),
            )
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PipelineInstrumentAwardConfig {
    pub instrument_id: String,
    pub atomic_units: AtomicUnits,
    pub descriptor: InstrumentDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PipelineBundleConfig {
    pub instrument_awards: Vec<PipelineInstrumentAwardConfig>,
    pub include_index: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

#[derive(Clone)]
struct FixedIndexSpec {
    embedder: Arc<dyn IdentifiedEmbedder>,
    index_reader: Arc<dyn IdentifiedIndexReader>,
}

#[derive(Clone)]
pub struct FixedScorePolicy {
    decision: ScoreDecision,
    index: Option<FixedIndexSpec>,
}

fn score_invalid<E>(_: E) -> PolicyError {
    PolicyError::permanent("score_output_invalid").expect("static label")
}

fn settle_invalid<E>(_: E) -> PolicyError {
    PolicyError::permanent("settle_output_invalid").expect("static label")
}

#[async_trait]
impl ScorePolicy for FixedScorePolicy {
    /// The embedder and the index reader are synchronous, so the evaluation
    /// runs on the blocking pool, never on a runtime worker (Zaki review 1,
    /// round 2, N-6), as the compatibility Score's does, and under the
    /// pipeline's bound on Score evaluations (#1140,
    /// `run_score_evaluation`).
    async fn execute(&self, input: &ScoreInput) -> Result<ScoreOutput, PolicyError> {
        let policy = self.clone();
        let input = input.clone();
        run_score_evaluation(move || policy.evaluate(&input))
            .await
            .map_err(|_| PolicyError::permanent("score_task_failed").expect("static label"))?
    }
}

impl FixedScorePolicy {
    /// The evaluation `execute` runs on the blocking pool.
    fn evaluate(&self, input: &ScoreInput) -> Result<ScoreOutput, PolicyError> {
        let mut evidence = ScoreEvidence::fixed(self.decision.awards().clone());
        let command = match &self.index {
            None => None,
            Some(spec) => {
                let snapshot = spec
                    .index_reader
                    .snapshot(&input.tenant_storage_ref, MINIMAL_INDEX_ID)
                    .map_err(|_| {
                        PolicyError::transient("index_unavailable").expect("static label")
                    })?;
                let mut entries = Vec::new();
                for (chunk, bytes) in input
                    .reviewed_artifact
                    .chunks(MINIMAL_INDEX_CHUNK_BYTES)
                    .enumerate()
                {
                    let embedding = spec.embedder.embed(bytes).map_err(|_| {
                        PolicyError::transient("embedder_unavailable").expect("static label")
                    })?;
                    entries.push(SealedIndexEntry {
                        chunk: u32::try_from(chunk).map_err(score_invalid)?,
                        content_hash: dependency_content_hash(bytes),
                        embedding,
                    });
                }
                let count = u32::try_from(entries.len()).map_err(score_invalid)?;
                let command = SealedIndexCommand::new(
                    MINIMAL_INDEX_ID,
                    input.registry_revision_id,
                    MINIMAL_PROJECTION_ID,
                    spec.embedder.model_id(),
                    entries,
                )
                .map_err(score_invalid)?;
                evidence.embedding_artifact_hash =
                    Some(command.content_hash().map_err(score_invalid)?);
                evidence.index_id = Some(MINIMAL_INDEX_ID.to_string());
                evidence.index_snapshot_id = Some(snapshot.snapshot_id);
                evidence.index_snapshot_hash = Some(snapshot.snapshot_hash);
                evidence.index_cardinality = Some(snapshot.cardinality);
                evidence.embedder_model_id = Some(spec.embedder.model_id().to_string());
                evidence.projection_id = Some(MINIMAL_PROJECTION_ID.to_string());
                evidence.projection_input_hash = Some(input.source_content_hash.clone());
                evidence.chunk_count = Some(count);
                evidence.total_chunk_count = Some(count);
                evidence.chunks_capped = Some(false);
                evidence.include_eligible = Some(true);
                Some(command)
            }
        };
        let rule_id = if self.decision.awards().is_empty() {
            "minimal_fixed_zero_v1"
        } else {
            "minimal_fixed_positive_v1"
        };
        ScoreOutput::new(
            PhaseResult {
                decision: self.decision.clone(),
                evidence,
                evaluation: ScoreEvaluation {
                    rule_id: rule_id.to_string(),
                    awards: self.decision.awards().clone(),
                },
            },
            command,
            None,
        )
        .map_err(score_invalid)
    }
}

pub struct FixedSettlePolicy;

#[async_trait]
impl SettlePolicy for FixedSettlePolicy {
    async fn execute(
        &self,
        input: &SettleInput,
    ) -> Result<PhaseResult<SettleDecision, SettleEvidence, SettleEvaluation>, PolicyError> {
        let (membership, rule_id) = match &input.index_command {
            Some(command) => (
                IndexMembershipDecision::Include {
                    command_hash: command.content_hash().map_err(settle_invalid)?,
                    entry_count: u32::try_from(command.entries().len()).map_err(settle_invalid)?,
                },
                "minimal_settle_include_v1",
            ),
            None => (
                IndexMembershipDecision::Exclude {
                    reason: ReasonCode::new("minimal_bundle_exclusion").expect("static label"),
                },
                "minimal_settle_exclude_v1",
            ),
        };
        let operations =
            settlement_operations(input.run_id, input.score.awards()).map_err(settle_invalid)?;
        let decision =
            SettleDecision::new(membership, &input.score, operations).map_err(settle_invalid)?;
        let evidence = SettleEvidence::operations(
            input.index_command.is_some(),
            u32::try_from(input.score.awards().iter().len()).map_err(settle_invalid)?,
        );
        Ok(PhaseResult {
            decision,
            evidence,
            evaluation: SettleEvaluation {
                rule_id: rule_id.to_string(),
            },
        })
    }
}

/// The minimal implementation ids `minimal_package` and
/// `from_package_with_runtime` agree on. A package naming anything else is
/// not runnable by this bundle.
const MINIMAL_ADMISSION_IMPLEMENTATION: &str = "trace_commons.admission.minimal.v1";
const MINIMAL_REVIEW_IMPLEMENTATION: &str = "trace_commons.review.minimal.v1";
const MINIMAL_SCORE_IMPLEMENTATION: &str = "trace_commons.score.minimal.v1";
const MINIMAL_SETTLE_IMPLEMENTATION: &str = "trace_commons.settle.minimal.v1";

/// Which event type the Trace Credit leg of PR 2's FR2 settlement
/// transaction (`settle_internal_credit` in `versioned_pipeline.rs`) writes
/// to the ledger for this bundle's family, and whether that leg composes and
/// finalizes a batch (Ruling S10). `PipelineScore` is the minimal family's
/// unchanged PR 2 behavior: the ledger's `Accepted` event type, batched and
/// finalized like every other credit event. `NoveltyUtility` is the
/// compatibility family's leg: the ledger's `NoveltyUtility` event type,
/// which `main`'s `trace_credit_event_type_is_settlement_eligible` excludes
/// from settlement -- that leg completes in the same transaction but is
/// never composed into a batch or paid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineTraceCreditEvent {
    PipelineScore,
    NoveltyUtility,
}

pub struct MinimalPolicyBundle {
    pub package: BundlePackage,
    pub admission: Arc<dyn AdmissionPolicy>,
    pub review: Arc<dyn ReviewPolicy>,
    pub score: Arc<dyn ScorePolicy>,
    pub settle: Arc<dyn SettlePolicy>,
    pub trace_credit_event: PipelineTraceCreditEvent,
}

impl MinimalPolicyBundle {
    /// Builds an unsigned package naming `config`, `scorer`, and `embedder`
    /// by content hash. The package does not hold runnable policies; pass it
    /// to [`Self::from_package_with_runtime`] with matching dependencies to
    /// build a bundle.
    pub fn minimal_package(
        config: &PipelineBundleConfig,
        scorer: &dyn IdentifiedPerplexityScorer,
        embedder: &dyn IdentifiedEmbedder,
    ) -> anyhow::Result<BundlePackage> {
        let config_bytes = serde_json::to_vec(config)?;
        let config_hash = dependency_content_hash(&config_bytes);
        let scorer_descriptor = scorer.content_descriptor();
        let embedder_descriptor = embedder.content_descriptor();
        let scorer_hash = dependency_content_hash(&scorer_descriptor);
        let embedder_hash = dependency_content_hash(&embedder_descriptor);

        let policy_ref = |phase: &str, implementation_id: &str| PolicyRef {
            policy_id: format!("trace_commons.{phase}.minimal"),
            implementation_id: implementation_id.to_string(),
            configuration_hash: config_hash.clone(),
            data_artifact_hashes: Vec::new(),
            projection_ids: Vec::new(),
        };

        // Pin each configured award's descriptor. A repeated instrument id
        // would silently keep the last descriptor in a plain map insert, so
        // this fails the build instead.
        let mut instruments = BTreeMap::new();
        for award in &config.instrument_awards {
            let instrument_id = InstrumentId::new(award.instrument_id.clone())
                .map_err(|_| anyhow::anyhow!(PIPELINE_BUNDLE_INVALID_LABEL))?;
            anyhow::ensure!(
                instruments
                    .insert(instrument_id, award.descriptor.clone())
                    .is_none(),
                PIPELINE_BUNDLE_INVALID_LABEL
            );
        }

        let manifest = BundleManifest {
            format_version: BUNDLE_MANIFEST_FORMAT_VERSION,
            admission: policy_ref("admission", MINIMAL_ADMISSION_IMPLEMENTATION),
            review: policy_ref("review", MINIMAL_REVIEW_IMPLEMENTATION),
            score: PolicyRef {
                policy_id: "trace_commons.score.minimal".to_string(),
                implementation_id: MINIMAL_SCORE_IMPLEMENTATION.to_string(),
                configuration_hash: config_hash.clone(),
                data_artifact_hashes: vec![scorer_hash.clone(), embedder_hash.clone()],
                projection_ids: if config.include_index {
                    vec![MINIMAL_PROJECTION_ID.to_string()]
                } else {
                    Vec::new()
                },
            },
            settle: policy_ref("settle", MINIMAL_SETTLE_IMPLEMENTATION),
            instruments,
        };

        let mut artifacts = BTreeMap::new();
        artifacts.insert(config_hash, config_bytes);
        artifacts.insert(scorer_hash, scorer_descriptor);
        artifacts.insert(embedder_hash, embedder_descriptor);

        let package = BundlePackage {
            bundle_id: manifest.bundle_id()?,
            manifest,
            artifacts,
        };
        package.validate()?;
        Ok(package)
    }

    /// Builds an unsigned package for the compatibility family, naming
    /// `config`, `scorer`, and `embedder` by content hash. As
    /// [`Self::minimal_package`], but under the compatibility implementation
    /// ids, with the compatibility Score policy's own dependency and
    /// projection names, and `trace_credit` pinned to
    /// `config.instrument.descriptor` so a `CompatibilityScorePolicy`'s
    /// awards are accepted at Score time (`ScoreDecision::for_bundle`,
    /// Ruling S3).
    pub fn compatibility_package(
        config: &CompatibilityBundleConfig,
        scorer: &dyn IdentifiedPerplexityScorer,
        embedder: &dyn IdentifiedEmbedder,
    ) -> anyhow::Result<BundlePackage> {
        config.validate()?;
        let config_bytes = serde_json::to_vec(config)?;
        let config_hash = dependency_content_hash(&config_bytes);
        let scorer_descriptor = scorer.content_descriptor();
        let embedder_descriptor = embedder.content_descriptor();
        let scorer_hash = dependency_content_hash(&scorer_descriptor);
        let embedder_hash = dependency_content_hash(&embedder_descriptor);

        let policy_ref = |phase: &str, implementation_id: &str| PolicyRef {
            policy_id: format!("trace_commons.{phase}.compatibility"),
            implementation_id: implementation_id.to_string(),
            configuration_hash: config_hash.clone(),
            data_artifact_hashes: Vec::new(),
            projection_ids: Vec::new(),
        };

        let mut instruments = BTreeMap::new();
        instruments.insert(
            InstrumentId::trace_credit(),
            config.instrument.descriptor.clone(),
        );

        let manifest = BundleManifest {
            format_version: BUNDLE_MANIFEST_FORMAT_VERSION,
            admission: policy_ref("admission", COMPATIBILITY_ADMISSION_IMPLEMENTATION),
            review: policy_ref("review", COMPATIBILITY_REVIEW_IMPLEMENTATION),
            score: PolicyRef {
                policy_id: "trace_commons.score.compatibility".to_string(),
                implementation_id: COMPATIBILITY_SCORE_IMPLEMENTATION.to_string(),
                configuration_hash: config_hash.clone(),
                data_artifact_hashes: vec![scorer_hash.clone(), embedder_hash.clone()],
                projection_ids: vec![config.projection_id.clone()],
            },
            settle: policy_ref("settle", COMPATIBILITY_SETTLE_IMPLEMENTATION),
            instruments,
        };

        let mut artifacts = BTreeMap::new();
        artifacts.insert(config_hash, config_bytes);
        artifacts.insert(scorer_hash, scorer_descriptor);
        artifacts.insert(embedder_hash, embedder_descriptor);

        let package = BundlePackage {
            bundle_id: manifest.bundle_id()?,
            manifest,
            artifacts,
        };
        package.validate()?;
        Ok(package)
    }

    /// Builds a runnable bundle from `package` and the runtime dependencies
    /// it names. Every dependency's `content_descriptor()` must hash to a
    /// value the package's Score policy ref names and stores; a substituted
    /// or changed dependency is refused rather than silently accepted.
    /// `package`'s four implementation ids must be entirely one known
    /// family -- all minimal or all compatibility -- or the package is not
    /// runnable by this bundle at all.
    pub fn from_package_with_runtime(
        package: BundlePackage,
        scorer: Arc<dyn IdentifiedPerplexityScorer>,
        embedder: Arc<dyn IdentifiedEmbedder>,
        index_reader: Arc<dyn IdentifiedIndexReader>,
    ) -> anyhow::Result<Self> {
        package
            .validate()
            .map_err(|_| anyhow::anyhow!(PIPELINE_BUNDLE_INVALID_LABEL))?;
        require_named_dependency(&package, &scorer.content_descriptor())?;
        require_named_dependency(&package, &embedder.content_descriptor())?;

        let implementation_ids = [
            package.manifest.admission.implementation_id.as_str(),
            package.manifest.review.implementation_id.as_str(),
            package.manifest.score.implementation_id.as_str(),
            package.manifest.settle.implementation_id.as_str(),
        ];

        if implementation_ids
            == [
                MINIMAL_ADMISSION_IMPLEMENTATION,
                MINIMAL_REVIEW_IMPLEMENTATION,
                MINIMAL_SCORE_IMPLEMENTATION,
                MINIMAL_SETTLE_IMPLEMENTATION,
            ]
        {
            let config_bytes = package
                .artifacts
                .get(&package.manifest.score.configuration_hash)
                .ok_or_else(|| anyhow::anyhow!(PIPELINE_BUNDLE_INVALID_LABEL))?;
            let config: PipelineBundleConfig = serde_json::from_slice(config_bytes)
                .map_err(|_| anyhow::anyhow!(PIPELINE_BUNDLE_INVALID_LABEL))?;

            let awards = InstrumentAwards::new(
                config
                    .instrument_awards
                    .iter()
                    .map(|award| {
                        InstrumentAward::new(
                            InstrumentId::new(award.instrument_id.clone())?,
                            award.atomic_units,
                        )
                    })
                    .collect::<Result<Vec<_>, ContractError>>()?,
            )?;
            let decision = ScoreDecision::for_bundle(&package.manifest, awards)
                .map_err(|_| anyhow::anyhow!(PIPELINE_BUNDLE_INVALID_LABEL))?;

            // The scorer is checked above to prove it matches the named
            // dependency; the minimal Score policy does not call it, so it
            // is not held by the bundle.
            Ok(Self {
                package,
                admission: Arc::new(MinimalAdmissionPolicy),
                review: Arc::new(MinimalReviewPolicy),
                score: Arc::new(FixedScorePolicy {
                    decision,
                    index: config.include_index.then(|| FixedIndexSpec {
                        embedder,
                        index_reader,
                    }),
                }),
                settle: Arc::new(FixedSettlePolicy),
                trace_credit_event: PipelineTraceCreditEvent::PipelineScore,
            })
        } else if implementation_ids
            == [
                COMPATIBILITY_ADMISSION_IMPLEMENTATION,
                COMPATIBILITY_REVIEW_IMPLEMENTATION,
                COMPATIBILITY_SCORE_IMPLEMENTATION,
                COMPATIBILITY_SETTLE_IMPLEMENTATION,
            ]
        {
            let config_bytes = package
                .artifacts
                .get(&package.manifest.score.configuration_hash)
                .ok_or_else(|| anyhow::anyhow!(PIPELINE_BUNDLE_INVALID_LABEL))?;
            let config: CompatibilityBundleConfig = serde_json::from_slice(config_bytes)
                .map_err(|_| anyhow::anyhow!(PIPELINE_BUNDLE_INVALID_LABEL))?;
            // Ruling S4: refuse at bind time, not at the first positive
            // award, a package whose manifest does not pin `trace_credit` to
            // this config's own instrument descriptor.
            anyhow::ensure!(
                package
                    .manifest
                    .instruments
                    .get(&InstrumentId::trace_credit())
                    == Some(&config.instrument.descriptor),
                PIPELINE_BUNDLE_INVALID_LABEL
            );
            let score = CompatibilityScorePolicy::new(
                package.manifest.clone(),
                config,
                scorer,
                embedder,
                index_reader,
            )
            .map_err(|_| anyhow::anyhow!(PIPELINE_BUNDLE_INVALID_LABEL))?;

            Ok(Self {
                package,
                admission: Arc::new(MinimalAdmissionPolicy),
                review: Arc::new(MinimalReviewPolicy),
                score: Arc::new(score),
                settle: Arc::new(CompatibilitySettlePolicy),
                trace_credit_event: PipelineTraceCreditEvent::NoveltyUtility,
            })
        } else {
            anyhow::bail!("bundle_policy_not_runnable")
        }
    }
}

/// Whether `package`'s four policies are the compatibility bundle's.
fn is_compatibility_family(package: &BundlePackage) -> bool {
    [
        package.manifest.admission.implementation_id.as_str(),
        package.manifest.review.implementation_id.as_str(),
        package.manifest.score.implementation_id.as_str(),
        package.manifest.settle.implementation_id.as_str(),
    ] == [
        COMPATIBILITY_ADMISSION_IMPLEMENTATION,
        COMPATIBILITY_REVIEW_IMPLEMENTATION,
        COMPATIBILITY_SCORE_IMPLEMENTATION,
        COMPATIBILITY_SETTLE_IMPLEMENTATION,
    ]
}

/// The compatibility configuration `package` binds, when its four policies
/// are the compatibility bundle's; `None` for any other bundle, or when the
/// configuration does not decode (construction refuses that package).
pub fn package_compatibility_config(package: &BundlePackage) -> Option<CompatibilityBundleConfig> {
    if !is_compatibility_family(package) {
        return None;
    }
    let config_bytes = package
        .artifacts
        .get(&package.manifest.score.configuration_hash)?;
    serde_json::from_slice(config_bytes).ok()
}

/// Whether `package`'s own configuration may be qualified for production:
/// a compatibility package's configuration must be qualifiable
/// (`CompatibilityBundleConfig::is_qualifiable`); a package of another
/// family carries no such configuration. A compatibility package whose
/// configuration is missing or does not decode is not qualifiable: it fails
/// closed (wave 2, fix round 1; review M1), since construction refuses it
/// too. Both the per-bundle qualification
/// (`PipelineService::bundle_qualification`, which startup reads) and
/// `PipelineQualificationStore::qualify_bundle` read it from the package
/// itself.
pub fn package_configuration_is_qualifiable(package: &BundlePackage) -> bool {
    if !is_compatibility_family(package) {
        return true;
    }
    package_compatibility_config(package).is_some_and(|config| config.is_qualifiable())
}

/// Requires that `package`'s Score policy ref names `descriptor` by content
/// hash and stores exactly those bytes as an artifact.
fn require_named_dependency(package: &BundlePackage, descriptor: &[u8]) -> anyhow::Result<()> {
    let hash = dependency_content_hash(descriptor);
    let named = package.manifest.score.data_artifact_hashes.contains(&hash)
        && package.artifacts.get(&hash).map(Vec::as_slice) == Some(descriptor);
    if named {
        Ok(())
    } else {
        anyhow::bail!(PIPELINE_DEPENDENCY_MISSING_LABEL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versioned_pipeline::pipeline_tenant_storage_ref;
    use crate::versioned_pipeline_index::IsolatedPipelineIndex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use trace_commons_gate_api::pipeline::{InstrumentKind, TRACE_CREDIT_DECIMALS};
    use trace_commons_gate_api::{Embedder, ReferenceEmbedder, ReferencePerplexityScorer};

    /// An embedder whose descriptor is chosen by the test, and which counts calls.
    struct CountingEmbedder {
        descriptor: Vec<u8>,
        calls: AtomicUsize,
    }
    impl Embedder for CountingEmbedder {
        fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            ReferenceEmbedder::new().embed(plaintext)
        }
    }
    impl IdentifiedEmbedder for CountingEmbedder {
        fn dependency_identity(&self) -> &str {
            "counting_embedder_test_only"
        }
        fn model_id(&self) -> &str {
            "counting-embedder-v1"
        }
        fn content_descriptor(&self) -> Vec<u8> {
            self.descriptor.clone()
        }
    }

    /// Pinned per A4: `nep141` on `testnet`, six decimals, so one atomic
    /// unit is one microcredit.
    fn trace_credit_descriptor() -> InstrumentDescriptor {
        InstrumentDescriptor {
            kind: InstrumentKind::Nep141,
            network: "testnet".to_string(),
            contract: "trace-credit.testnet".to_string(),
            decimals: TRACE_CREDIT_DECIMALS,
        }
    }

    /// Pinned per A4: an off-chain credit account, whole units only.
    fn storage_rebate_descriptor() -> InstrumentDescriptor {
        InstrumentDescriptor {
            kind: InstrumentKind::CreditAccount,
            network: "pipeline-test".to_string(),
            contract: "storage-rebate".to_string(),
            decimals: 0,
        }
    }

    fn config(include_index: bool) -> PipelineBundleConfig {
        PipelineBundleConfig {
            instrument_awards: vec![
                PipelineInstrumentAwardConfig {
                    instrument_id: "storage_rebate".into(),
                    atomic_units: AtomicUnits::from_raw(5),
                    descriptor: storage_rebate_descriptor(),
                },
                PipelineInstrumentAwardConfig {
                    instrument_id: "trace_credit".into(),
                    atomic_units: AtomicUnits::from_raw(1_000_000),
                    descriptor: trace_credit_descriptor(),
                },
            ],
            include_index,
            variant: None,
        }
    }

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

    #[tokio::test]
    async fn score_uses_the_embedder_the_constructor_was_given() {
        let scorer = Arc::new(ReferencePerplexityScorer::new());
        let named = Arc::new(CountingEmbedder {
            descriptor: b"counting-v1".to_vec(),
            calls: AtomicUsize::new(0),
        });
        let unrelated = Arc::new(CountingEmbedder {
            descriptor: b"unrelated-v1".to_vec(),
            calls: AtomicUsize::new(0),
        });
        let package =
            MinimalPolicyBundle::minimal_package(&config(true), scorer.as_ref(), named.as_ref())
                .unwrap();
        let bundle = MinimalPolicyBundle::from_package_with_runtime(
            package,
            scorer,
            named.clone(),
            IsolatedPipelineIndex::new(),
        )
        .unwrap();
        let bytes = vec![b'x'; 600]; // three 256-byte chunks
        let output = bundle.score.execute(&score_input(&bytes)).await.unwrap();
        let command = output
            .index_command()
            .expect("include_index proposes a command");
        assert_eq!(
            command
                .entries()
                .iter()
                .map(|e| e.chunk)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(command.model_id(), "counting-embedder-v1");
        assert_eq!(named.calls.load(Ordering::SeqCst), 3);
        assert_eq!(
            unrelated.calls.load(Ordering::SeqCst),
            0,
            "unrelated dependencies are ignored"
        );
        let evidence = &output.result().evidence;
        assert_eq!(
            (
                evidence.chunk_count,
                evidence.total_chunk_count,
                evidence.chunks_capped
            ),
            (Some(3), Some(3), Some(false))
        );
        assert_eq!(
            evidence.embedding_artifact_hash.as_deref(),
            Some(command.content_hash().unwrap().as_str())
        );
    }

    /// An embedder that sleeps on every call, as a CPU-bound local model
    /// holds its thread, and records how many calls ran at once.
    struct SleepingEmbedder {
        sleep: std::time::Duration,
        entered: AtomicUsize,
        calls: AtomicUsize,
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
    }
    impl SleepingEmbedder {
        fn new(sleep: std::time::Duration) -> Self {
            Self {
                sleep,
                entered: AtomicUsize::new(0),
                calls: AtomicUsize::new(0),
                in_flight: AtomicUsize::new(0),
                max_in_flight: AtomicUsize::new(0),
            }
        }
    }
    impl Embedder for SleepingEmbedder {
        fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
            self.entered.fetch_add(1, Ordering::SeqCst);
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(self.sleep);
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            self.calls.fetch_add(1, Ordering::SeqCst);
            ReferenceEmbedder::new().embed(plaintext)
        }
    }
    impl IdentifiedEmbedder for SleepingEmbedder {
        fn dependency_identity(&self) -> &str {
            "sleeping_embedder_test_only"
        }
        fn model_id(&self) -> &str {
            "sleeping-embedder-v1"
        }
        fn content_descriptor(&self) -> Vec<u8> {
            b"sleeping-v1".to_vec()
        }
    }

    fn sleeping_bundle(embedder: Arc<SleepingEmbedder>) -> MinimalPolicyBundle {
        let scorer = Arc::new(ReferencePerplexityScorer::new());
        let package =
            MinimalPolicyBundle::minimal_package(&config(true), scorer.as_ref(), embedder.as_ref())
                .unwrap();
        MinimalPolicyBundle::from_package_with_runtime(
            package,
            scorer,
            embedder,
            IsolatedPipelineIndex::new(),
        )
        .unwrap()
    }

    /// #1140: the pipeline worker shares ingest's runtime with HTTP, so a
    /// Score whose embedder holds its thread must not hold a runtime
    /// thread. On a current-thread runtime -- one thread, as a saturated
    /// pilot runtime effectively is -- an HTTP request served by that
    /// runtime completes while the embedder is still sleeping. Were the
    /// embed run inline in `execute`, the first await below would hand the
    /// only thread to the Score, and nothing would run until every chunk
    /// was embedded.
    ///
    /// A regression guard, not evidence of the #1140 change: the raw
    /// `spawn_blocking` from N-6 already kept the embed off the runtime, so
    /// this test passes with or without `run_score_evaluation`. What #1140
    /// adds, the bound, is pinned by the two burst tests (this module's and
    /// `versioned_pipeline_compat`'s).
    #[tokio::test(flavor = "current_thread")]
    async fn a_slow_embed_leaves_the_runtime_free_to_serve_http() {
        use tower::ServiceExt;
        let embedder = Arc::new(SleepingEmbedder::new(std::time::Duration::from_millis(750)));
        let bundle = sleeping_bundle(embedder.clone());
        let health = axum::Router::new().route("/health", axum::routing::get(|| async { "ok" }));

        let started = std::time::Instant::now();
        let bytes = vec![b'x'; 600]; // three 256-byte chunks: 2.25 s of embedding
        let score = tokio::spawn(async move { bundle.score.execute(&score_input(&bytes)).await });
        // Wait, on this runtime, until the first embed is under way.
        while embedder.entered.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            assert!(started.elapsed() < std::time::Duration::from_secs(5));
        }
        let response = health
            .oneshot(
                axum::http::Request::get("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        // Counted, not timed: no embed has returned yet, so the request was
        // served while the first one still held its thread.
        assert_eq!(
            embedder.calls.load(Ordering::SeqCst),
            0,
            "the health request was served while the first embed still ran"
        );
        let output = score.await.unwrap().unwrap();
        assert_eq!(output.index_command().unwrap().entries().len(), 3);
        assert_eq!(embedder.calls.load(Ordering::SeqCst), 3);
    }

    /// #1140: a burst of Score evaluations takes at most
    /// `PIPELINE_SCORE_EVALUATION_CONCURRENCY` blocking threads at once;
    /// the rest wait for a permit on the runtime, where waiting costs
    /// nothing, instead of each taking a thread of the shared blocking
    /// pool. Every evaluation still completes.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_burst_of_scores_is_bounded_on_the_blocking_pool() {
        use crate::versioned_pipeline_blocking::PIPELINE_SCORE_EVALUATION_CONCURRENCY;
        let embedder = Arc::new(SleepingEmbedder::new(std::time::Duration::from_millis(50)));
        let bundle = sleeping_bundle(embedder.clone());
        let burst = PIPELINE_SCORE_EVALUATION_CONCURRENCY * 3;
        let mut scores = tokio::task::JoinSet::new();
        for _ in 0..burst {
            let score = bundle.score.clone();
            scores.spawn(async move { score.execute(&score_input(&[b'x'; 100])).await });
        }
        while let Some(joined) = scores.join_next().await {
            joined.unwrap().unwrap();
        }
        assert_eq!(embedder.calls.load(Ordering::SeqCst), burst);
        let max = embedder.max_in_flight.load(Ordering::SeqCst);
        assert!(
            (1..=PIPELINE_SCORE_EVALUATION_CONCURRENCY).contains(&max),
            "{max} Score evaluations held blocking threads at once"
        );
    }

    #[test]
    fn a_substituted_or_changed_dependency_is_refused() {
        let scorer = Arc::new(ReferencePerplexityScorer::new());
        let named = Arc::new(CountingEmbedder {
            descriptor: b"counting-v1".to_vec(),
            calls: AtomicUsize::new(0),
        });
        let package =
            MinimalPolicyBundle::minimal_package(&config(true), scorer.as_ref(), named.as_ref())
                .unwrap();
        // Same label, different content: the hash the package names does not match.
        let changed = Arc::new(CountingEmbedder {
            descriptor: b"counting-v2".to_vec(),
            calls: AtomicUsize::new(0),
        });
        let error = MinimalPolicyBundle::from_package_with_runtime(
            package.clone(),
            scorer.clone(),
            changed,
            IsolatedPipelineIndex::new(),
        )
        .err()
        .unwrap();
        assert_eq!(error.to_string(), PIPELINE_DEPENDENCY_MISSING_LABEL);
        // Tampered package bytes fail package validation.
        let mut tampered = package;
        let config_hash = tampered.manifest.score.configuration_hash.clone();
        tampered.artifacts.insert(config_hash, b"{}".to_vec());
        let error = MinimalPolicyBundle::from_package_with_runtime(
            tampered,
            scorer,
            named,
            IsolatedPipelineIndex::new(),
        )
        .err()
        .unwrap();
        assert_eq!(error.to_string(), PIPELINE_BUNDLE_INVALID_LABEL);
    }

    #[test]
    fn construction_names_no_code_hash_and_changes_bundle_id_with_config() {
        let scorer = ReferencePerplexityScorer::new();
        let embedder = ReferenceEmbedder::new();
        let first =
            MinimalPolicyBundle::minimal_package(&config(false), &scorer, &embedder).unwrap();
        let mut other = config(false);
        other.variant = Some("second".into());
        let second = MinimalPolicyBundle::minimal_package(&other, &scorer, &embedder).unwrap();
        assert_ne!(first.bundle_id, second.bundle_id);
        let named = &first.manifest.score.data_artifact_hashes;
        assert!(named.contains(&dependency_content_hash(&scorer.content_descriptor())));
        assert!(named.contains(&dependency_content_hash(&embedder.content_descriptor())));
    }

    /// A package is built at the current manifest format version, and a
    /// package whose manifest names an earlier one is refused as
    /// `bundle_package_invalid` before any policy is built from it.
    #[test]
    fn packages_build_at_the_current_manifest_format_version() {
        let scorer = Arc::new(ReferencePerplexityScorer::new());
        let embedder = Arc::new(ReferenceEmbedder::new());
        let package =
            MinimalPolicyBundle::minimal_package(&config(true), scorer.as_ref(), embedder.as_ref())
                .unwrap();
        assert_eq!(BUNDLE_MANIFEST_FORMAT_VERSION, 2);
        assert_eq!(
            package.manifest.format_version,
            BUNDLE_MANIFEST_FORMAT_VERSION
        );
        assert_eq!(package.bundle_id, package.manifest.bundle_id().unwrap());

        let mut earlier = package;
        earlier.manifest.format_version = BUNDLE_MANIFEST_FORMAT_VERSION - 1;
        let error = MinimalPolicyBundle::from_package_with_runtime(
            earlier,
            scorer,
            embedder,
            IsolatedPipelineIndex::new(),
        )
        .err()
        .unwrap();
        assert_eq!(error.to_string(), PIPELINE_BUNDLE_INVALID_LABEL);
    }

    #[tokio::test]
    async fn settle_operations_match_score_awards_and_use_shared_references() {
        let awards = InstrumentAwards::new(vec![
            InstrumentAward::new(
                InstrumentId::new("storage_rebate").unwrap(),
                AtomicUnits::from_raw(5),
            )
            .unwrap(),
        ])
        .unwrap();
        let operations = settlement_operations(Uuid::from_u128(1), &awards).unwrap();
        let award = awards.iter().next().unwrap();
        assert_eq!(
            operations[0].operation_ref_hash(),
            pipeline_operation_ref(Uuid::from_u128(1), award)
        );
        assert_eq!(
            operations[0].result_ref_hash(),
            Some(pipeline_result_ref(Uuid::from_u128(1), award).as_str())
        );
    }

    #[tokio::test]
    async fn review_approves_passthrough_content_with_provenance() {
        let bytes = b"approved bytes".to_vec();
        let input = ReviewInput {
            run_id: Uuid::from_u128(1),
            tenant_storage_ref: pipeline_tenant_storage_ref("tenant-a"),
            trace_id: Uuid::from_u128(2),
            source_content_hash: dependency_content_hash(&bytes),
            source_artifact: bytes.clone(),
            admission: AdmissionDecision::Admit,
            human_assessment: None,
        };
        let output = MinimalReviewPolicy.execute(&input).await.unwrap();
        let content = output
            .approved_content()
            .expect("pass-through approval carries content");
        assert_eq!(content.bytes(), bytes.as_slice());
        assert_eq!(content.worker_identity(), "minimal_review_passthrough");
        assert!(!output.result().evidence.content_changed);
    }

    /// Wave 2, fix round 1 (review M1): the configuration term fails closed
    /// for a compatibility package. A production-compatible configuration
    /// is qualifiable; the local reference (all floors zero) is not; nor is
    /// a compatibility package whose configuration artifact is missing or
    /// does not decode. A package of another family carries no such
    /// configuration and stays qualifiable.
    #[test]
    fn the_configuration_term_fails_closed_for_a_compatibility_package() {
        let scorer = ReferencePerplexityScorer::new();
        let embedder = ReferenceEmbedder::new();
        let reference = CompatibilityBundleConfig::local_reference();
        let production = CompatibilityBundleConfig::production_compatible(
            reference.scorer_model_id.clone(),
            reference.projection_id.clone(),
            reference.index_id.clone(),
            &crate::versioned_pipeline_compat::MainGateConfig {
                perplexity_floor_micros: Some(2_000_000),
                tail_fraction_floor_micros: Some(0),
                novelty_floor_micros: Some(500_000),
                embed_insert_novelty_micros: reference.embed_insert_novelty_micros,
                top_k: reference.top_k,
                chunk_target_tokens: reference.chunk_target_tokens,
                chunk_max_tokens: reference.chunk_max_tokens,
                chunk_cap: reference.chunk_cap,
                chunk_min_tokens: reference.chunk_min_tokens,
                novelty_utility_microcredits: reference.novelty_utility_microcredits,
            },
        )
        .expect("a production-compatible configuration validates");
        let qualifiable =
            MinimalPolicyBundle::compatibility_package(&production, &scorer, &embedder).unwrap();
        assert!(package_configuration_is_qualifiable(&qualifiable));
        let local =
            MinimalPolicyBundle::compatibility_package(&reference, &scorer, &embedder).unwrap();
        assert!(!package_configuration_is_qualifiable(&local));

        let config_hash = qualifiable.manifest.score.configuration_hash.clone();
        let mut missing = qualifiable.clone();
        missing.artifacts.remove(&config_hash);
        let mut undecodable = qualifiable.clone();
        undecodable
            .artifacts
            .insert(config_hash, br#"{"unknown_field":1}"#.to_vec());
        for package in [&missing, &undecodable] {
            assert_eq!(package_compatibility_config(package), None);
            assert!(!package_configuration_is_qualifiable(package));
        }

        let minimal = MinimalPolicyBundle::minimal_package(&config(false), &scorer, &embedder)
            .expect("build minimal bundle package");
        assert_eq!(package_compatibility_config(&minimal), None);
        assert!(package_configuration_is_qualifiable(&minimal));
    }

    #[test]
    fn compatibility_package_names_its_dependencies_and_pins_trace_credit() {
        let scorer = ReferencePerplexityScorer::new();
        let embedder = ReferenceEmbedder::new();
        let config = CompatibilityBundleConfig::local_reference();
        let package = MinimalPolicyBundle::compatibility_package(&config, &scorer, &embedder)
            .expect("build compatibility bundle package");

        assert_eq!(
            package.manifest.admission.implementation_id,
            COMPATIBILITY_ADMISSION_IMPLEMENTATION
        );
        assert_eq!(
            package.manifest.review.implementation_id,
            COMPATIBILITY_REVIEW_IMPLEMENTATION
        );
        assert_eq!(
            package.manifest.score.implementation_id,
            COMPATIBILITY_SCORE_IMPLEMENTATION
        );
        assert_eq!(
            package.manifest.settle.implementation_id,
            COMPATIBILITY_SETTLE_IMPLEMENTATION
        );
        assert_eq!(
            package.manifest.score.projection_ids,
            vec![config.projection_id.clone()]
        );
        let named = &package.manifest.score.data_artifact_hashes;
        assert!(named.contains(&dependency_content_hash(&scorer.content_descriptor())));
        assert!(named.contains(&dependency_content_hash(&embedder.content_descriptor())));
        assert_eq!(
            package
                .manifest
                .instruments
                .get(&InstrumentId::trace_credit()),
            Some(&config.instrument.descriptor)
        );

        let bundle = MinimalPolicyBundle::from_package_with_runtime(
            package,
            Arc::new(scorer),
            Arc::new(embedder),
            IsolatedPipelineIndex::new(),
        )
        .expect("build compatibility bundle from package");
        assert_eq!(
            bundle.trace_credit_event,
            PipelineTraceCreditEvent::NoveltyUtility
        );
    }

    /// A package cannot mix families: a compatibility Score paired with a
    /// minimal Settle (or any other combination that is not entirely one
    /// known family) is refused before either policy is ever constructed.
    #[test]
    fn mixed_family_ids_are_not_runnable() {
        let scorer = ReferencePerplexityScorer::new();
        let embedder = ReferenceEmbedder::new();
        let config = CompatibilityBundleConfig::local_reference();
        let mut package = MinimalPolicyBundle::compatibility_package(&config, &scorer, &embedder)
            .expect("build compatibility bundle package");
        package.manifest.settle.implementation_id = MINIMAL_SETTLE_IMPLEMENTATION.to_string();
        package.bundle_id = package
            .manifest
            .bundle_id()
            .expect("recompute bundle id after mutating settle's implementation id");

        let error = MinimalPolicyBundle::from_package_with_runtime(
            package,
            Arc::new(scorer),
            Arc::new(embedder),
            IsolatedPipelineIndex::new(),
        )
        .err()
        .expect("mixed family implementation ids must not build a bundle");
        assert_eq!(error.to_string(), "bundle_policy_not_runnable");
    }

    /// Ruling: the compatibility arm parses `CompatibilityBundleConfig`
    /// strictly and refuses a package whose config artifact is not one, with
    /// `bundle_package_invalid` -- exercised here by a config artifact whose
    /// bytes are still hash-consistent with the manifest (so
    /// `BundlePackage::validate` itself passes, unlike
    /// `a_substituted_or_changed_dependency_is_refused`'s tamper case) but do
    /// not deserialize into a `CompatibilityBundleConfig`.
    #[test]
    fn tampered_compatibility_config_is_invalid() {
        let scorer = ReferencePerplexityScorer::new();
        let embedder = ReferenceEmbedder::new();
        let config = CompatibilityBundleConfig::local_reference();
        let mut package = MinimalPolicyBundle::compatibility_package(&config, &scorer, &embedder)
            .expect("build compatibility bundle package");

        let old_config_hash = package.manifest.score.configuration_hash.clone();
        let invalid_config_bytes = b"{}".to_vec();
        let new_config_hash = dependency_content_hash(&invalid_config_bytes);
        package.artifacts.remove(&old_config_hash);
        package
            .artifacts
            .insert(new_config_hash.clone(), invalid_config_bytes);
        package.manifest.admission.configuration_hash = new_config_hash.clone();
        package.manifest.review.configuration_hash = new_config_hash.clone();
        package.manifest.score.configuration_hash = new_config_hash.clone();
        package.manifest.settle.configuration_hash = new_config_hash;
        package.bundle_id = package
            .manifest
            .bundle_id()
            .expect("recompute bundle id after swapping the config artifact");

        let error = MinimalPolicyBundle::from_package_with_runtime(
            package,
            Arc::new(scorer),
            Arc::new(embedder),
            IsolatedPipelineIndex::new(),
        )
        .err()
        .expect("an invalid config artifact must not build a bundle");
        assert_eq!(error.to_string(), PIPELINE_BUNDLE_INVALID_LABEL);
    }

    /// Ruling S4 / Finding I1: the compatibility arm refuses at bind time, not
    /// at the first positive award, a package whose manifest does not pin
    /// `trace_credit` to this config's own instrument descriptor. Covers both
    /// ways a manifest can fail to pin it correctly: a different (but still
    /// individually valid) `nep141` descriptor, and no `trace_credit` pin at
    /// all. Neither mutation is caught earlier -- `BundlePackage::validate`
    /// never reads `instruments` at all (only the four policy refs'
    /// configuration/data-artifact hashes), and `BundleManifest::canonical_bytes`
    /// accepts an empty or a differently-valued `instruments` map so long as
    /// each present descriptor is individually well-formed -- so both
    /// mutations reach the compatibility arm and are refused there.
    #[test]
    fn compatibility_manifest_must_pin_trace_credit_to_the_config_descriptor() {
        let scorer = ReferencePerplexityScorer::new();
        let embedder = ReferenceEmbedder::new();
        let config = CompatibilityBundleConfig::local_reference();

        // Case 1: a different, but still individually valid, nep141
        // descriptor for trace_credit (same kind and decimals, different
        // contract).
        let mismatched_descriptor = InstrumentDescriptor {
            kind: InstrumentKind::Nep141,
            network: "testnet".to_string(),
            contract: "different-trace-credit.testnet".to_string(),
            decimals: TRACE_CREDIT_DECIMALS,
        };
        assert_ne!(mismatched_descriptor, config.instrument.descriptor);
        let mut mismatched_package =
            MinimalPolicyBundle::compatibility_package(&config, &scorer, &embedder)
                .expect("build compatibility bundle package");
        mismatched_package
            .manifest
            .instruments
            .insert(InstrumentId::trace_credit(), mismatched_descriptor);
        mismatched_package.bundle_id = mismatched_package
            .manifest
            .bundle_id()
            .expect("recompute bundle id after mutating the pinned descriptor");
        let error = MinimalPolicyBundle::from_package_with_runtime(
            mismatched_package,
            Arc::new(ReferencePerplexityScorer::new()),
            Arc::new(ReferenceEmbedder::new()),
            IsolatedPipelineIndex::new(),
        )
        .err()
        .expect("a mismatched trace_credit descriptor must not build a bundle");
        assert_eq!(error.to_string(), PIPELINE_BUNDLE_INVALID_LABEL);

        // Case 2: no trace_credit pin at all.
        let mut unpinned_package =
            MinimalPolicyBundle::compatibility_package(&config, &scorer, &embedder)
                .expect("build compatibility bundle package");
        unpinned_package
            .manifest
            .instruments
            .remove(&InstrumentId::trace_credit());
        unpinned_package.bundle_id = unpinned_package
            .manifest
            .bundle_id()
            .expect("recompute bundle id after removing the pinned instrument");
        let error = MinimalPolicyBundle::from_package_with_runtime(
            unpinned_package,
            Arc::new(scorer),
            Arc::new(embedder),
            IsolatedPipelineIndex::new(),
        )
        .err()
        .expect("a package with no trace_credit pin must not build a bundle");
        assert_eq!(error.to_string(), PIPELINE_BUNDLE_INVALID_LABEL);
    }
}
