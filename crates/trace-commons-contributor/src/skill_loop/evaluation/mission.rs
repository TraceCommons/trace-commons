//! Mission-package preparation for the existing held-out skill evaluator.

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use trace_commons_protocol::mission_evaluation::{
    MISSION_EVALUATION_TOTAL_REQUESTS, MissionEvaluationPackage, render_skill,
};
use uuid::Uuid;

use crate::mission_attempt::MissionTrialKey;
use crate::skill_loop::evaluation::fixtures::{
    APPLICABILITY_FIXTURES, EvaluationFixture, select_held_out_fixtures,
};
use crate::skill_loop::evaluation::{
    FrozenEvaluatorInput, NearAiEvaluationClient, SkillEvaluationError, SkillEvaluationReport,
    SkillTrialResult, evaluation_contract, run_evaluation, scheduled_arms,
};
use crate::skill_loop::source_task_fingerprint;

/// Receives each valid completed trial before it is accepted into a report.
///
/// Returning any error stops the run and is reported as
/// [`SkillEvaluationError::PersistenceUnavailable`].
#[async_trait]
pub trait SkillEvaluationObserver: Send + Sync {
    async fn trial_completed(&self, trial: &SkillTrialResult) -> Result<(), SkillEvaluationError>;
}

/// A validated mission evaluation whose model is selected but whose inference
/// requests have not started.
///
/// Preparation is structural only. It does not authorize model spend,
/// publication, reward, execution, or skill installation.
pub struct PreparedMissionEvaluation {
    client: NearAiEvaluationClient,
    input: FrozenEvaluatorInput,
    expected_trials: Vec<MissionTrialKey>,
}

impl PreparedMissionEvaluation {
    /// Validates and freezes a package before consulting the NEAR AI catalog.
    ///
    /// This may issue catalog requests. It never dispatches inference.
    pub async fn prepare(
        api_key: String,
        package: &MissionEvaluationPackage,
        approval_id: Uuid,
    ) -> Result<Self, SkillEvaluationError> {
        let (input, expected_trials) = Self::freeze(package, approval_id)?;
        let client = NearAiEvaluationClient::discover(api_key).await?;
        Ok(Self {
            client,
            input,
            expected_trials,
        })
    }

    fn freeze(
        package: &MissionEvaluationPackage,
        approval_id: Uuid,
    ) -> Result<(FrozenEvaluatorInput, Vec<MissionTrialKey>), SkillEvaluationError> {
        package
            .validate()
            .map_err(|_| SkillEvaluationError::InvalidPackage)?;
        if package.evaluation_contract_hash != evaluation_contract_hash() {
            return Err(SkillEvaluationError::StaleContract);
        }
        if approval_id.is_nil() {
            return Err(SkillEvaluationError::InvalidApproval);
        }

        let fingerprint = source_task_fingerprint(&package.task)
            .ok_or(SkillEvaluationError::HeldOutSetUnavailable)?;
        let selection = select_held_out_fixtures(&fingerprint)
            .ok_or(SkillEvaluationError::HeldOutSetUnavailable)?;
        let expected_trials = expected_trials_for(&selection.selected);
        if expected_trials.len() != MISSION_EVALUATION_TOTAL_REQUESTS as usize {
            return Err(SkillEvaluationError::Internal);
        }

        let input = FrozenEvaluatorInput {
            approval_id,
            draft: package.skill.clone(),
            skill_md: render_skill(&package.skill),
            skill_sha256: package.skill_sha256.clone(),
            source_task_fingerprint: fingerprint,
        };
        if !input.is_frozen() {
            return Err(SkillEvaluationError::Internal);
        }
        Ok((input, expected_trials))
    }

    #[cfg(test)]
    fn prepare_with_client(
        client: NearAiEvaluationClient,
        package: &MissionEvaluationPackage,
        approval_id: Uuid,
    ) -> Result<Self, SkillEvaluationError> {
        let (input, expected_trials) = Self::freeze(package, approval_id)?;
        Ok(Self {
            client,
            input,
            expected_trials,
        })
    }

    /// Returns the catalog-selected model pinned for every trial.
    #[must_use]
    pub fn requested_model(&self) -> &str {
        self.client.model()
    }

    /// Returns the exact fixture/arm keys this run must complete.
    #[must_use]
    pub fn expected_trials(&self) -> Vec<MissionTrialKey> {
        self.expected_trials.clone()
    }

    /// Runs the existing evaluator with durable progress and cancellation.
    ///
    /// The resulting quality report remains evidence only; mission status and
    /// publication authority are controlled separately by the caller.
    pub async fn run(
        self,
        observer: &dyn SkillEvaluationObserver,
        cancellation: watch::Receiver<bool>,
    ) -> Result<SkillEvaluationReport, SkillEvaluationError> {
        run_evaluation(self.client, self.input, observer, cancellation).await
    }
}

fn expected_trials_for(selected: &[EvaluationFixture]) -> Vec<MissionTrialKey> {
    let contract = evaluation_contract();
    let mut trials = Vec::with_capacity(contract.total_requests);
    for (index, fixture) in APPLICABILITY_FIXTURES.iter().enumerate() {
        for arm in scheduled_arms(index) {
            trials.push(MissionTrialKey {
                task_id: fixture.id.to_string(),
                arm,
            });
        }
    }
    for (index, fixture) in selected.iter().enumerate() {
        for arm in scheduled_arms(index) {
            trials.push(MissionTrialKey {
                task_id: fixture.id.to_string(),
                arm,
            });
        }
    }
    trials
}

// Production packages conservatively pin the exact evaluator source surface:
// any production policy, fixture, prompt, transport, renderer, or adapter edit
// requires republishing. Test modules are deliberately separate and excluded,
// so test-only maintenance cannot invalidate immutable production packages.
const EVALUATION_CONTRACT_SOURCES: &[(&str, &str)] = &[
    ("mission-adapter", include_str!("mission.rs")),
    ("evaluator", include_str!("../evaluation.rs")),
    ("scoring-and-prompts", include_str!("scoring.rs")),
    ("fixture-selection", include_str!("fixtures.rs")),
    (
        "fixture-catalog-and-oracles",
        include_str!("fixtures/catalog.rs"),
    ),
    ("near-ai-transport", include_str!("transport.rs")),
    ("source-task-fingerprint", include_str!("../mod.rs")),
    (
        "shared-mission-package-and-renderer",
        include_str!("../../../../trace-commons-protocol/src/mission_evaluation.rs"),
    ),
];

/// Returns the conservative exact-production-source evaluator contract hash.
///
/// Any change to the declared production source surface intentionally requires
/// newly published packages; sibling test-file changes do not.
#[must_use]
pub fn evaluation_contract_hash() -> String {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HASH.get_or_init(|| compute_evaluation_contract_hash(EVALUATION_CONTRACT_SOURCES))
        .clone()
}

fn compute_evaluation_contract_hash(sources: &[(&str, &str)]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"trace-commons:mission-evaluation-contract:v1\0");
    for &(label, source) in sources {
        let normalized = source.replace("\r\n", "\n");
        digest.update((label.len() as u64).to_be_bytes());
        digest.update(label.as_bytes());
        digest.update((normalized.len() as u64).to_be_bytes());
        digest.update(normalized.as_bytes());
    }
    hex::encode(digest.finalize())
}

#[cfg(test)]
#[path = "mission_tests.rs"]
mod tests;
