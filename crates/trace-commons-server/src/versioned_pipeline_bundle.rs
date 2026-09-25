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
    Embedder, PerplexityScorer, ReferenceEmbedder, ReferencePerplexityScorer, VectorIndexReader,
    VectorIndexWriter,
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

/// A perplexity scorer identified by a content-hashable descriptor, so a
/// bundle package can name it and a runtime can prove it matched.
pub trait IdentifiedPerplexityScorer: PerplexityScorer {
    fn dependency_identity(&self) -> &str;
    fn content_descriptor(&self) -> Vec<u8>;
    fn production_qualified(&self) -> bool {
        false
    }
}

/// An embedder identified by a content-hashable descriptor and a model id.
pub trait IdentifiedEmbedder: Embedder {
    fn dependency_identity(&self) -> &str;
    fn model_id(&self) -> &str;
    fn content_descriptor(&self) -> Vec<u8>;
    fn production_qualified(&self) -> bool {
        false
    }
}

/// A vector index reader identified for bundle-dependency reporting.
pub trait IdentifiedIndexReader: VectorIndexReader {
    fn dependency_identity(&self) -> &str;
    fn production_qualified(&self) -> bool {
        false
    }
}

/// A vector index writer identified for bundle-dependency reporting.
pub trait IdentifiedIndexWriter: VectorIndexWriter {
    fn dependency_identity(&self) -> &str;
    fn production_qualified(&self) -> bool {
        false
    }
}

impl IdentifiedPerplexityScorer for ReferencePerplexityScorer {
    fn dependency_identity(&self) -> &str {
        "reference_perplexity_test_only"
    }

    fn content_descriptor(&self) -> Vec<u8> {
        b"trace-commons-reference-perplexity-scorer.v1".to_vec()
    }
}

impl IdentifiedEmbedder for ReferenceEmbedder {
    fn dependency_identity(&self) -> &str {
        "reference_embedder_test_only"
    }

    fn model_id(&self) -> &str {
        "reference-embedder-v1"
    }

    fn content_descriptor(&self) -> Vec<u8> {
        b"trace-commons-reference-embedder.v1".to_vec()
    }
}

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
                PrivacyRisk::High => (
                    AdmissionDecision::Reject {
                        reason: ReasonCode::new("privacy_risk_rejected").expect("static label"),
                    },
                    true,
                    Some("privacy_risk_rejected"),
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
                    PolicyError::transient("review_assessment_required").expect("static label")
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

struct FixedIndexSpec {
    embedder: Arc<dyn IdentifiedEmbedder>,
    index_reader: Arc<dyn IdentifiedIndexReader>,
}

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
    async fn execute(&self, input: &ScoreInput) -> Result<ScoreOutput, PolicyError> {
        let mut evidence = ScoreEvidence::fixed(self.decision.awards().clone());
        let command = match &self.index {
            None => None,
            Some(spec) => {
                let tenant = input.tenant_storage_ref.as_str();
                let snapshot = spec
                    .index_reader
                    .snapshot(tenant, MINIMAL_INDEX_ID)
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

pub struct MinimalPolicyBundle {
    pub package: BundlePackage,
    pub admission: Arc<dyn AdmissionPolicy>,
    pub review: Arc<dyn ReviewPolicy>,
    pub score: Arc<dyn ScorePolicy>,
    pub settle: Arc<dyn SettlePolicy>,
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

    /// Builds a runnable bundle from `package` and the runtime dependencies
    /// it names. Every dependency's `content_descriptor()` must hash to a
    /// value the package's Score policy ref names and stores; a substituted
    /// or changed dependency is refused rather than silently accepted.
    pub fn from_package_with_runtime(
        package: BundlePackage,
        scorer: Arc<dyn IdentifiedPerplexityScorer>,
        embedder: Arc<dyn IdentifiedEmbedder>,
        index_reader: Arc<dyn IdentifiedIndexReader>,
    ) -> anyhow::Result<Self> {
        package
            .validate()
            .map_err(|_| anyhow::anyhow!(PIPELINE_BUNDLE_INVALID_LABEL))?;

        let implementation_ids = [
            package.manifest.admission.implementation_id.as_str(),
            package.manifest.review.implementation_id.as_str(),
            package.manifest.score.implementation_id.as_str(),
            package.manifest.settle.implementation_id.as_str(),
        ];
        anyhow::ensure!(
            implementation_ids
                == [
                    MINIMAL_ADMISSION_IMPLEMENTATION,
                    MINIMAL_REVIEW_IMPLEMENTATION,
                    MINIMAL_SCORE_IMPLEMENTATION,
                    MINIMAL_SETTLE_IMPLEMENTATION,
                ],
            "bundle_policy_not_runnable"
        );

        require_named_dependency(&package, &scorer.content_descriptor())?;
        require_named_dependency(&package, &embedder.content_descriptor())?;

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
        // dependency; the minimal Score policy does not call it, so it is
        // not held by the bundle.
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
        })
    }
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
    use trace_commons_gate_api::{ReferenceEmbedder, ReferencePerplexityScorer};

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
}
