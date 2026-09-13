//! INTEGRATION: runs the approved skill and both controls through the same
//! NEAR AI model and scores complete structured decisions on public fixtures.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::skill_loop::SkillReview;

mod fixtures;
mod scoring;
mod transport;

use fixtures::{APPLICABILITY_FIXTURES, FIXTURES, PLAN_TASK_COUNT, select_held_out_fixtures};
use scoring::{METADATA_CLUSTER, candidate_regressions, score_applicability, score_completion};
use transport::NearAiEvaluationClient;

/// Maximum completion tokens allowed for each arm of a held-out task.
pub(super) const OUTPUT_TOKEN_LIMIT: u32 = 900;
/// Per-request timeout applied uniformly across evaluation arms.
pub(super) const REQUEST_TIMEOUT_SECS: u64 = 90;
/// Maximum number of evaluation requests allowed in flight.
pub(super) const EVALUATION_CONCURRENCY: usize = 2;

/// Leaves room for the daemon response envelope and framing below the 1 MiB
/// IPC line limit. The transport also caps each retained model output.
const MAX_SERIALIZED_REPORT_BYTES: usize = 768 * 1024;
const HELD_OUT_SELECTION_POLICY: &str = "Select six incidents from the fixed public fixture pool after excluding normalized task SHA-256 matches and lexical 128-bit SimHash near-matches; retain the remainder as disclosed reserves and refuse evaluation unless six tasks remain.";

/// Disclosed controls that govern a candidate's held-out evaluation.
///
/// The contract fixes task selection, arm count, model ownership, token budget,
/// and timeout before the owner approves a Cloud-backed test.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillEvaluationContract {
    /// Total applicability and plan tasks evaluated for each arm.
    pub task_count: usize,
    /// Number of source-excluded repository plan tasks evaluated for each arm.
    pub plan_task_count: usize,
    /// Number of public repository incidents available before source exclusion.
    pub fixture_pool_count: usize,
    /// Number of plan clusters plus the metadata applicability cluster.
    pub cluster_count: usize,
    /// Number of comparison arms run for every selected task.
    pub arm_count: usize,
    /// Total model requests required by the contract.
    pub total_requests: usize,
    /// Maximum completion tokens allowed for each request.
    pub output_token_limit: u32,
    /// Timeout applied independently to each request.
    pub request_timeout_seconds: u64,
    /// Required model-catalog owner for every comparison arm.
    pub required_model_owner: &'static str,
    /// Human-readable disclosure of fixture content sent for evaluation.
    pub fixture_scope: &'static str,
    /// Human-readable source-overlap exclusion and reserve policy.
    pub selection_policy: &'static str,
}

/// One of the three controls evaluated under the same model and request budget.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationArm {
    /// Public fixture prompt without additional corrective guidance.
    Baseline,
    /// Public fixture prompt augmented by the fixed simple instruction.
    ManualInstruction,
    /// Public fixture prompt augmented by the exact reviewed skill package.
    CandidateSkill,
}

impl EvaluationArm {
    pub(super) const ALL: [Self; 3] = [
        Self::Baseline,
        Self::ManualInstruction,
        Self::CandidateSkill,
    ];

    #[must_use]
    /// Returns the owner-facing label for this comparison arm.
    pub fn label(self) -> &'static str {
        match self {
            Self::Baseline => "Baseline",
            Self::ManualInstruction => "Simple instruction",
            Self::CandidateSkill => "Candidate skill",
        }
    }
}

/// Provider-reported token usage for one evaluation request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvaluationUsage {
    /// Prompt-token count when reported by the provider.
    pub prompt_tokens: Option<u64>,
    /// Completion-token count when reported by the provider.
    pub completion_tokens: Option<u64>,
    /// Reasoning-token count when reported by the provider.
    pub reasoning_tokens: Option<u64>,
    /// Total-token count when reported by the provider.
    pub total_tokens: Option<u64>,
}

/// Structured repository plan required from a model completion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProposedTaskPlan {
    /// Diagnosis of the fixture's generated-source failure.
    pub diagnosis: String,
    /// Authoritative source paths the model proposes changing.
    pub edit_paths: Vec<String>,
    /// Generator or repository commands the model proposes running.
    pub commands: Vec<String>,
    /// Independent checks the model proposes for the regenerated output.
    pub verification: Vec<String>,
}

/// Scored result for one public fixture and one evaluation arm.
///
/// The task, source URL, and raw output are content-bearing evaluation evidence.
/// They describe client-shipped public fixtures and the provider response, rather
/// than the account-owned source session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillTrialResult {
    /// Stable client-shipped fixture identifier.
    pub task_id: String,
    /// Fixture cluster used to separate plan and applicability scoring.
    pub cluster: String,
    /// Public held-out task presented to the model.
    pub task: String,
    /// Public source URL supporting the fixture.
    pub source_url: String,
    /// Comparison arm used for this request.
    pub arm: EvaluationArm,
    /// Whether the completion satisfied the fixture scorer.
    pub passed: bool,
    /// Stable explanations for failed scoring requirements.
    pub failure_reasons: Vec<String>,
    /// Parsed structured plan, absent when parsing or scoring could not retain one.
    pub answer: Option<ProposedTaskPlan>,
    /// Bounded full provider output retained for owner inspection.
    pub raw_output: String,
    /// Provider request identifier when one was returned.
    pub request_id: Option<String>,
    /// Exact model identifier reported for this completion.
    pub served_model: String,
    /// Provider finish reason for this completion.
    pub finish_reason: String,
    /// Provider-reported token usage.
    pub usage: EvaluationUsage,
}

/// Aggregate pass count for one arm over a defined task subset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillArmSummary {
    /// Comparison arm summarized by this record.
    pub arm: EvaluationArm,
    /// Owner-facing arm label.
    pub label: &'static str,
    /// Number of tasks that passed in this subset.
    pub passed: usize,
    /// Number of tasks evaluated in this subset.
    pub total: usize,
}

/// Model and held-out selection actually used for one evaluation run.
///
/// These fields vary per run. Fixed counts, ownership requirements, and request
/// budgets live only in [`SkillEvaluationContract`]. Flattening preserves the
/// existing daemon wire shape while keeping the Rust domain model canonical.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SkillEvaluationExecution {
    /// Model selected before dispatching evaluation requests.
    pub requested_model: String,
    /// Model verified across every completed evaluation request.
    pub served_model: String,
    /// Catalog owner observed for the selected model.
    pub model_owned_by: &'static str,
    /// Text-free SHA-256 fingerprint of the account-owned source task.
    pub source_task_fingerprint_sha256: String,
    /// Public fixture IDs selected for plan evaluation.
    pub selected_plan_task_ids: Vec<String>,
    /// Public fixture IDs rejected because they overlapped the source task.
    pub excluded_source_overlap_task_ids: Vec<String>,
    /// Eligible public fixture IDs retained outside this evaluation.
    pub reserve_plan_task_ids: Vec<String>,
    /// Fixed context categories that were available to the evaluator.
    pub ambient_context: Vec<&'static str>,
}

/// Complete, bounded evidence report for an approved skill evaluation.
///
/// The report binds results to `review_id` and `skill_sha256`, records source-task
/// exclusion without source text, and exposes the exact gate used for installation.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SkillEvaluationReport {
    /// Unique identifier for this completed evaluation.
    pub evaluation_id: Uuid,
    /// Approved review evaluated by every arm.
    pub review_id: Uuid,
    /// SHA-256 digest of the exact reviewed `SKILL.md` bytes.
    pub skill_sha256: String,
    /// Immutable controls approved before evaluation started.
    #[serde(flatten)]
    pub contract: SkillEvaluationContract,
    /// Run-specific model identity and held-out fixture selection.
    #[serde(flatten)]
    pub execution: SkillEvaluationExecution,
    /// Pass counts over applicability and repository plan tasks together.
    pub summaries: Vec<SkillArmSummary>,
    /// Pass counts over repository plan tasks only.
    pub plan_summaries: Vec<SkillArmSummary>,
    /// Pass counts over metadata applicability tasks only.
    pub applicability_summaries: Vec<SkillArmSummary>,
    /// Per-request outcomes and bounded model output for owner inspection.
    pub trials: Vec<SkillTrialResult>,
    /// Fixture IDs where the candidate failed after a control arm passed.
    pub regressions: Vec<String>,
    /// Whether the candidate beat both controls and passed all safety gates.
    pub install_allowed: bool,
    /// Stable explanation for the installation-gate result.
    pub gate_reason: &'static str,
}

/// Stable failures returned while preparing or running a held-out evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillEvaluationError {
    /// No NEAR AI inference credential was available.
    CredentialRequired,
    /// The HTTP evaluation client could not be constructed.
    ClientUnavailable,
    /// The NEAR AI model catalog could not be read.
    CatalogUnavailable,
    /// No ready structured text model owned by `nearai` was available.
    NoPrivateModel,
    /// An evaluation request could not be completed.
    RequestUnavailable,
    /// The provider rejected the inference credential.
    ProviderRejected,
    /// The account requires additional NEAR AI Cloud credit.
    FundingRequired,
    /// A bounded response or complete report exceeded its size limit.
    ResponseTooLarge,
    /// The provider served a different model during the comparison.
    ModelChanged,
    /// Source-task exclusion left too few held-out fixtures.
    HeldOutSetUnavailable,
    /// A local invariant, task, serialization, or digest check failed.
    Internal,
}

impl SkillEvaluationError {
    #[must_use]
    /// Returns the stable daemon refusal label for this evaluation failure.
    pub fn label(self) -> &'static str {
        match self {
            Self::CredentialRequired => "skill-evaluation-credential-required",
            Self::ClientUnavailable => "skill-evaluation-client-unavailable",
            Self::CatalogUnavailable => "skill-evaluation-catalog-unavailable",
            Self::NoPrivateModel => "skill-evaluation-private-model-unavailable",
            Self::RequestUnavailable => "skill-evaluation-request-unavailable",
            Self::ProviderRejected => "skill-evaluation-provider-rejected",
            Self::FundingRequired => "skill-evaluation-funding-required",
            Self::ResponseTooLarge => "skill-evaluation-response-too-large",
            Self::ModelChanged => "skill-evaluation-model-changed",
            Self::HeldOutSetUnavailable => "skill-evaluation-held-out-set-unavailable",
            Self::Internal => "skill-evaluation-internal",
        }
    }
}

#[must_use]
/// Returns the fixed, owner-visible controls for the current evaluation fixture set.
pub fn evaluation_contract() -> SkillEvaluationContract {
    let task_count = PLAN_TASK_COUNT + APPLICABILITY_FIXTURES.len();
    SkillEvaluationContract {
        task_count,
        plan_task_count: PLAN_TASK_COUNT,
        fixture_pool_count: FIXTURES.len(),
        cluster_count: PLAN_TASK_COUNT + 1,
        arm_count: EvaluationArm::ALL.len(),
        total_requests: task_count * EvaluationArm::ALL.len(),
        output_token_limit: OUTPUT_TOKEN_LIMIT,
        request_timeout_seconds: REQUEST_TIMEOUT_SECS,
        required_model_owner: "nearai",
        fixture_scope: "Six source-excluded client-shipped public repository incidents plus two metadata-only applicability probes; no session task, correction, or evidence is sent.",
        selection_policy: HELD_OUT_SELECTION_POLICY,
    }
}

/// Evaluates an exact approved skill against both controls on held-out public tasks.
///
/// Every request uses the same ready NEAR-owned model, timeout, and output budget.
/// Prompts include public fixtures and, for the candidate arm, `review.skill_md`;
/// the source session task, correction, and evidence excerpts are not sent. Callers
/// must authorize the review for the active account before invoking this function.
pub async fn evaluate_skill(
    api_key: String,
    review: &SkillReview,
) -> Result<SkillEvaluationReport, SkillEvaluationError> {
    if !review_candidate_is_frozen(review) {
        return Err(SkillEvaluationError::Internal);
    }
    let client = NearAiEvaluationClient::discover(api_key).await?;
    evaluate_with_client(client, review).await
}

async fn evaluate_with_client(
    client: NearAiEvaluationClient,
    review: &SkillReview,
) -> Result<SkillEvaluationReport, SkillEvaluationError> {
    // Freeze the reviewed candidate before the first applicability or body
    // request. Every later clone derives from the digest-checked value.
    if !review_candidate_is_frozen(review) {
        return Err(SkillEvaluationError::Internal);
    }
    let contract = evaluation_contract();
    let selection = select_held_out_fixtures(&review.source_task_fingerprint)
        .ok_or(SkillEvaluationError::HeldOutSetUnavailable)?;
    let semaphore = Arc::new(Semaphore::new(EVALUATION_CONCURRENCY));
    let mut trials = Vec::with_capacity(contract.total_requests);

    // Agent Skills are discovered from metadata. Run one relevant and one
    // unrelated decision for every arm before any candidate body is supplied.
    let mut jobs = JoinSet::new();
    for (fixture_index, fixture) in APPLICABILITY_FIXTURES.iter().copied().enumerate() {
        for arm in scheduled_arms(fixture_index) {
            let permit = Arc::clone(&semaphore);
            let client = client.clone();
            let skill_name = review.draft.name.clone();
            let skill_description = review.draft.description.clone();
            jobs.spawn(async move {
                let _held = permit
                    .acquire_owned()
                    .await
                    .map_err(|_| SkillEvaluationError::Internal)?;
                let completion = client
                    .complete_applicability(fixture, arm, &skill_name, &skill_description)
                    .await?;
                Ok::<_, SkillEvaluationError>(score_applicability(fixture, arm, completion))
            });
        }
    }
    collect_job_results(&mut jobs, client.model(), &mut trials).await?;

    let mut jobs = JoinSet::new();
    for (fixture_index, fixture) in selection.selected.iter().copied().enumerate() {
        for arm in scheduled_arms(fixture_index) {
            let permit = Arc::clone(&semaphore);
            let client = client.clone();
            let skill_name = review.draft.name.clone();
            let skill_md = review.skill_md.clone();
            jobs.spawn(async move {
                let _held = permit
                    .acquire_owned()
                    .await
                    .map_err(|_| SkillEvaluationError::Internal)?;
                let completion = client
                    .complete(fixture, arm, &skill_name, &skill_md)
                    .await?;
                Ok::<_, SkillEvaluationError>(score_completion(fixture, arm, completion))
            });
        }
    }

    collect_job_results(&mut jobs, client.model(), &mut trials).await?;
    trials.sort_by(|left, right| {
        left.task_id
            .cmp(&right.task_id)
            .then(left.arm.cmp(&right.arm))
    });

    let served_model = client.model().to_string();

    let summaries = summarize_trials(&trials, contract.task_count, |_| true);
    let plan_summaries = summarize_trials(&trials, selection.selected.len(), |trial| {
        trial.cluster != METADATA_CLUSTER
    });
    let applicability_summaries =
        summarize_trials(&trials, APPLICABILITY_FIXTURES.len(), |trial| {
            trial.cluster == METADATA_CLUSTER
        });
    let plan_by_arm = plan_summaries
        .iter()
        .map(|summary| (summary.arm, summary.passed))
        .collect::<BTreeMap<_, _>>();
    let applicability_by_arm = applicability_summaries
        .iter()
        .map(|summary| (summary.arm, summary.passed))
        .collect::<BTreeMap<_, _>>();
    let regressions = candidate_regressions(&trials);
    let metadata_failed = applicability_by_arm
        .get(&EvaluationArm::CandidateSkill)
        .copied()
        .unwrap_or(0)
        != APPLICABILITY_FIXTURES.len();
    let baseline_plan = *plan_by_arm.get(&EvaluationArm::Baseline).unwrap_or(&0);
    let manual_plan = *plan_by_arm
        .get(&EvaluationArm::ManualInstruction)
        .unwrap_or(&0);
    let candidate_plan = *plan_by_arm
        .get(&EvaluationArm::CandidateSkill)
        .unwrap_or(&0);
    let install_allowed = candidate_plan > baseline_plan
        && candidate_plan > manual_plan
        && regressions.is_empty()
        && !metadata_failed;
    let gate_reason = if install_allowed {
        "skill-improved-over-both-controls"
    } else if metadata_failed {
        "skill-failed-metadata-applicability"
    } else if !regressions.is_empty() {
        "skill-regressed-on-held-out-task"
    } else {
        "skill-did-not-beat-both-controls"
    };
    let selected_plan_task_ids = selection
        .selected
        .iter()
        .map(|fixture| fixture.id.to_string())
        .collect::<Vec<_>>();
    let selected_cluster_count = selection
        .selected
        .iter()
        .map(|fixture| fixture.cluster)
        .collect::<BTreeSet<_>>()
        .len()
        + 1;
    if selection.selected.len() != contract.plan_task_count
        || FIXTURES.len() != contract.fixture_pool_count
        || selected_cluster_count != contract.cluster_count
        || trials.len() != contract.total_requests
    {
        return Err(SkillEvaluationError::Internal);
    }
    let report = SkillEvaluationReport {
        evaluation_id: Uuid::new_v4(),
        review_id: review.review_id,
        skill_sha256: review.skill_sha256.clone(),
        contract,
        execution: SkillEvaluationExecution {
            requested_model: client.model().to_string(),
            served_model,
            model_owned_by: "nearai",
            source_task_fingerprint_sha256: review.source_task_fingerprint.sha256.clone(),
            selected_plan_task_ids,
            excluded_source_overlap_task_ids: selection.source_overlap_ids,
            reserve_plan_task_ids: selection.reserve_ids,
            ambient_context: vec![
                "fixed evaluator system prompt",
                "bounded client-shipped public repository evidence",
                "neutral JSON guidance wrapper",
                "metadata-only applicability probes before body evaluation",
                "fixed counterbalanced arm schedule across held-out tasks",
                "no host rules, skills, repository files, or session excerpts",
            ],
        },
        summaries,
        plan_summaries,
        applicability_summaries,
        trials,
        regressions,
        install_allowed,
        gate_reason,
    };
    let serialized = serde_json::to_vec(&report).map_err(|_| SkillEvaluationError::Internal)?;
    if serialized.len() > MAX_SERIALIZED_REPORT_BYTES {
        return Err(SkillEvaluationError::ResponseTooLarge);
    }
    Ok(report)
}

async fn collect_job_results(
    jobs: &mut JoinSet<Result<SkillTrialResult, SkillEvaluationError>>,
    expected_model: &str,
    trials: &mut Vec<SkillTrialResult>,
) -> Result<(), SkillEvaluationError> {
    while let Some(joined) = jobs.join_next().await {
        let trial = match joined {
            Ok(Ok(trial)) => trial,
            Ok(Err(error)) => {
                jobs.abort_all();
                return Err(error);
            }
            Err(_) => {
                jobs.abort_all();
                return Err(SkillEvaluationError::Internal);
            }
        };
        if trial.served_model != expected_model {
            jobs.abort_all();
            return Err(SkillEvaluationError::ModelChanged);
        }
        trials.push(trial);
    }
    Ok(())
}

fn scheduled_arms(task_index: usize) -> [EvaluationArm; 3] {
    match task_index % EvaluationArm::ALL.len() {
        0 => [
            EvaluationArm::Baseline,
            EvaluationArm::ManualInstruction,
            EvaluationArm::CandidateSkill,
        ],
        1 => [
            EvaluationArm::ManualInstruction,
            EvaluationArm::CandidateSkill,
            EvaluationArm::Baseline,
        ],
        _ => [
            EvaluationArm::CandidateSkill,
            EvaluationArm::Baseline,
            EvaluationArm::ManualInstruction,
        ],
    }
}

fn summarize_trials(
    trials: &[SkillTrialResult],
    total: usize,
    include: impl Fn(&SkillTrialResult) -> bool,
) -> Vec<SkillArmSummary> {
    EvaluationArm::ALL
        .into_iter()
        .map(|arm| SkillArmSummary {
            arm,
            label: arm.label(),
            passed: trials
                .iter()
                .filter(|trial| include(trial) && trial.arm == arm && trial.passed)
                .count(),
            total,
        })
        .collect()
}

fn review_candidate_is_frozen(review: &SkillReview) -> bool {
    crate::skill_loop::render_skill(&review.draft) == review.skill_md
        && crate::skill_loop::sha256(review.skill_md.as_bytes()) == review.skill_sha256
}

#[cfg(test)]
#[path = "evaluation/tests.rs"]
mod tests;
