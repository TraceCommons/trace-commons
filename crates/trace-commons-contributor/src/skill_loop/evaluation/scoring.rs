//! INTEGRATION: constructs fair arm prompts and scores one binary outcome for
//! each public incident or metadata-applicability probe.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::skill_loop::MANUAL_CONTROL_INSTRUCTION;
use crate::skill_loop::evaluation::fixtures::{
    APPLICABILITY_SOURCE_URL, ApplicabilityFixture, EvaluationFixture,
};
use crate::skill_loop::evaluation::transport::ModelCompletion;
use crate::skill_loop::evaluation::{EvaluationArm, ProposedTaskPlan, SkillTrialResult};

const MANUAL_GUIDANCE_NAME: &str = "generated-source-manual-control";
pub(super) const METADATA_CLUSTER: &str = "metadata-applicability";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplicabilityDecision {
    applicable: bool,
}

pub(super) fn plan_system_prompt() -> &'static str {
    "You are a coding agent planning one repository change. Use only the supplied public repository snapshot. Additional guidance may be supplied in a neutral JSON wrapper; apply it only when relevant. Return one JSON object with exactly these keys: diagnosis (string), edit_paths (array of repository-relative paths changed directly before running any generator), commands (array of complete regeneration commands), verification (array of complete independent check commands). In edit_paths, omit generated outputs that the listed commands will update. Do not invent paths. Keep every array at or below 16 items and every string at or below 1024 characters."
}

pub(super) fn fixture_prompt(
    fixture: EvaluationFixture,
    arm: EvaluationArm,
    skill_name: &str,
    skill_md: &str,
) -> String {
    let guidance = match arm {
        EvaluationArm::Baseline => serde_json::Value::Null,
        EvaluationArm::ManualInstruction => serde_json::json!({
            "name": MANUAL_GUIDANCE_NAME,
            "content": MANUAL_CONTROL_INSTRUCTION,
        }),
        EvaluationArm::CandidateSkill => serde_json::json!({
            "name": skill_name,
            "content": skill_md,
        }),
    };
    let mut prompt = format!(
        "Task: {}\n\nPublic repository snapshot commit: {}\n\nGuidance (JSON; null means none): {}\n\nPublic repository evidence:\n",
        fixture.task, fixture.snapshot_commit, guidance
    );
    for item in fixture.evidence {
        prompt.push_str("\n--- file excerpt: ");
        prompt.push_str(item.path);
        prompt.push_str(" ---\n");
        prompt.push_str(item.contents);
        prompt.push_str("\n--- end excerpt ---\n");
    }
    prompt.push_str("\nGive the complete change plan now.");
    prompt
}

pub(super) fn applicability_system_prompt() -> &'static str {
    "Make an applicability decision from the supplied guidance metadata: should this guidance be loaded for the task? You receive metadata only, never the guidance body. Return one JSON object with exactly one key: applicable (boolean)."
}

pub(super) fn applicability_prompt(
    fixture: ApplicabilityFixture,
    arm: EvaluationArm,
    skill_name: &str,
    skill_description: &str,
) -> String {
    let metadata = match arm {
        EvaluationArm::Baseline => serde_json::Value::Null,
        EvaluationArm::ManualInstruction => serde_json::json!({
            "name": MANUAL_GUIDANCE_NAME,
            "description": MANUAL_CONTROL_INSTRUCTION,
        }),
        EvaluationArm::CandidateSkill => serde_json::json!({
            "name": skill_name,
            "description": skill_description,
        }),
    };
    format!(
        "Task: {}\n\nGuidance metadata (JSON; null means no additional guidance): {}\n\nReturn the applicability decision now.",
        fixture.task, metadata
    )
}

pub(super) fn score_completion(
    fixture: EvaluationFixture,
    arm: EvaluationArm,
    completion: ModelCompletion,
) -> SkillTrialResult {
    let parsed = parse_plan(&completion.raw_output);
    let (answer, mut failure_reasons) = match parsed {
        Some(answer) => {
            let reasons = score_plan(fixture, &answer);
            (Some(answer), reasons)
        }
        None => (
            None,
            vec!["The model did not return the required complete JSON plan.".to_string()],
        ),
    };
    add_finish_failure(&completion, &mut failure_reasons);
    SkillTrialResult {
        task_id: fixture.id.to_string(),
        cluster: fixture.cluster.to_string(),
        task: fixture.task.to_string(),
        source_url: fixture.source_url.to_string(),
        arm,
        passed: failure_reasons.is_empty(),
        failure_reasons,
        answer,
        raw_output: completion.raw_output,
        request_id: completion.request_id,
        served_model: completion.served_model,
        finish_reason: completion.finish_reason,
        usage: completion.usage,
    }
}

pub(super) fn score_applicability(
    fixture: ApplicabilityFixture,
    arm: EvaluationArm,
    completion: ModelCompletion,
) -> SkillTrialResult {
    let decision = parse_json::<ApplicabilityDecision>(&completion.raw_output);
    let expected = fixture.guidance_should_apply && arm != EvaluationArm::Baseline;
    let mut failure_reasons = match decision {
        Some(decision) if decision.applicable == expected => Vec::new(),
        Some(decision) => vec![format!(
            "The metadata applicability decision was `{}`; `{expected}` was required for this arm.",
            decision.applicable
        )],
        None => vec![
            "The model did not return the required strict JSON applicability decision.".to_string(),
        ],
    };
    add_finish_failure(&completion, &mut failure_reasons);
    SkillTrialResult {
        task_id: fixture.id.to_string(),
        cluster: METADATA_CLUSTER.to_string(),
        task: format!("Agent Skill metadata applicability: {}", fixture.task),
        source_url: APPLICABILITY_SOURCE_URL.to_string(),
        arm,
        passed: failure_reasons.is_empty(),
        failure_reasons,
        answer: None,
        raw_output: completion.raw_output,
        request_id: completion.request_id,
        served_model: completion.served_model,
        finish_reason: completion.finish_reason,
        usage: completion.usage,
    }
}

fn add_finish_failure(completion: &ModelCompletion, failures: &mut Vec<String>) {
    if completion.finish_reason != "stop" {
        failures.push(format!(
            "The model response did not complete normally (finish_reason `{}`).",
            completion.finish_reason
        ));
    }
}

fn parse_plan(raw: &str) -> Option<ProposedTaskPlan> {
    let plan = parse_json(raw)?;
    valid_plan_shape(&plan).then_some(plan)
}

fn parse_json<T: for<'de> Deserialize<'de>>(raw: &str) -> Option<T> {
    let trimmed = raw.trim();
    let candidate = if trimmed.starts_with("```") {
        let first_newline = trimmed.find('\n')?;
        let without_open = &trimmed[first_newline + 1..];
        let close = without_open.rfind("```")?;
        if !without_open[close + 3..].trim().is_empty() {
            return None;
        }
        without_open[..close].trim()
    } else {
        trimmed
    };
    serde_json::from_str(candidate).ok()
}

fn valid_plan_shape(plan: &ProposedTaskPlan) -> bool {
    if plan.diagnosis.trim().is_empty()
        || plan.diagnosis.chars().count() > 1_024
        || plan.edit_paths.len() > 16
        || plan.commands.len() > 16
        || plan.verification.len() > 16
    {
        return false;
    }
    plan.edit_paths
        .iter()
        .all(|path| valid_relative_path(path) && path.chars().count() <= 1_024)
        && plan
            .commands
            .iter()
            .chain(&plan.verification)
            .all(|value| !value.trim().is_empty() && value.chars().count() <= 1_024)
}

fn valid_relative_path(path: &str) -> bool {
    let trimmed = path.trim();
    !trimmed.is_empty()
        && trimmed == path
        && !trimmed.starts_with('/')
        && !trimmed.contains('\\')
        && !trimmed.split('/').any(|component| component == "..")
}

fn normalize_command(command: &str) -> Option<String> {
    if command
        .chars()
        .any(|character| matches!(character, '\n' | '\r' | '\0'))
    {
        return None;
    }
    let tokens = command
        .split_ascii_whitespace()
        .map(|token| token.strip_prefix("./").unwrap_or(token))
        .collect::<Vec<_>>();
    (!tokens.is_empty()).then(|| tokens.join(" "))
}

fn commands_match_required_steps(commands: &[String], steps: &[&[&str]]) -> bool {
    let Some(commands) = commands
        .iter()
        .map(|command| normalize_command(command))
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    let command_count = commands.len();
    let supplied = commands.into_iter().collect::<BTreeSet<_>>();
    if supplied.len() != command_count || supplied.len() != steps.len() {
        return false;
    }
    let allowed = steps
        .iter()
        .flat_map(|alternatives| alternatives.iter())
        .filter_map(|command| normalize_command(command))
        .collect::<BTreeSet<_>>();
    supplied.iter().all(|command| allowed.contains(command))
        && steps.iter().all(|alternatives| {
            alternatives.iter().any(|alternative| {
                normalize_command(alternative).is_some_and(|value| supplied.contains(&value))
            })
        })
}

fn score_plan(fixture: EvaluationFixture, plan: &ProposedTaskPlan) -> Vec<String> {
    let edits = plan
        .edit_paths
        .iter()
        .map(|path| path.trim_start_matches("./"))
        .collect::<BTreeSet<_>>();
    let expected_edits = fixture
        .required_edit_paths
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut failures = Vec::new();
    if edits.len() != plan.edit_paths.len() || edits != expected_edits {
        failures.push(
            "The plan's intentional edits did not exactly match the authoritative source set."
                .to_string(),
        );
    }
    if edits
        .iter()
        .any(|path| fixture.generated_output_paths.contains(path))
    {
        failures.push("The plan directly edited a generated output.".to_string());
    }
    if !commands_match_required_steps(&plan.commands, fixture.regeneration_steps) {
        failures.push("The plan did not include exactly one approved command for every required regeneration step.".to_string());
    }
    if !commands_match_required_steps(&plan.verification, fixture.verification_steps) {
        failures.push("The plan did not include exactly one approved command for every independent verification step.".to_string());
    }
    failures
}

pub(super) fn candidate_regressions(trials: &[SkillTrialResult]) -> Vec<String> {
    let mut regressions = trials
        .iter()
        .filter(|trial| {
            trial.cluster == METADATA_CLUSTER
                && trial.arm == EvaluationArm::CandidateSkill
                && !trial.passed
        })
        .map(|trial| trial.task_id.clone())
        .collect::<BTreeSet<_>>();
    let mut by_task: BTreeMap<&str, BTreeMap<EvaluationArm, bool>> = BTreeMap::new();
    for trial in trials {
        by_task
            .entry(&trial.task_id)
            .or_default()
            .insert(trial.arm, trial.passed);
    }
    for (task, arms) in by_task {
        let candidate = arms
            .get(&EvaluationArm::CandidateSkill)
            .copied()
            .unwrap_or(false);
        let control_passed = arms.get(&EvaluationArm::Baseline).copied().unwrap_or(false)
            || arms
                .get(&EvaluationArm::ManualInstruction)
                .copied()
                .unwrap_or(false);
        if !candidate && control_passed {
            regressions.insert(task.to_string());
        }
    }
    regressions.into_iter().collect()
}

#[cfg(test)]
#[path = "scoring_tests.rs"]
mod tests;
