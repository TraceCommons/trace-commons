use crate::skill_loop::evaluation::EvaluationUsage;
use crate::skill_loop::evaluation::fixtures::{APPLICABILITY_FIXTURES, FIXTURES};
use crate::skill_loop::evaluation::scoring::*;

fn completion(raw: String, finish_reason: &str) -> ModelCompletion {
    ModelCompletion {
        raw_output: raw,
        request_id: Some("request-1".to_string()),
        served_model: "deepseek-ai/DeepSeek-V4-Flash".to_string(),
        finish_reason: finish_reason.to_string(),
        usage: EvaluationUsage {
            prompt_tokens: Some(50),
            completion_tokens: Some(30),
            reasoning_tokens: Some(0),
            total_tokens: Some(80),
        },
    }
}

fn valid_plan(fixture: EvaluationFixture) -> String {
    serde_json::json!({
        "diagnosis": "The generated files derive from one source.",
        "edit_paths": fixture.required_edit_paths,
        "commands": fixture.regeneration_steps.iter().map(|step| step[0]).collect::<Vec<_>>(),
        "verification": fixture.verification_steps.iter().map(|step| step[0]).collect::<Vec<_>>(),
    })
    .to_string()
}

#[test]
fn fixture_prompt_contains_raw_evidence_without_serializing_oracle_arrays() {
    for fixture in FIXTURES {
        let prompt = fixture_prompt(*fixture, EvaluationArm::Baseline, "ignored", "ignored");
        assert!(prompt.contains(fixture.snapshot_commit));
        for evidence in fixture.evidence {
            assert!(prompt.contains(evidence.path));
            assert!(prompt.contains(evidence.contents));
        }
        assert!(!prompt.contains("required_edit_paths"));
        assert!(!prompt.contains("generated_output_paths"));
        assert!(!prompt.contains("regeneration_steps"));
        assert!(!prompt.contains("verification_steps"));
        assert!(!prompt.contains(fixture.source_url));
    }
}

#[test]
fn manual_and_candidate_use_the_same_neutral_guidance_wrapper() {
    let fixture = FIXTURES[0];
    let manual = fixture_prompt(
        fixture,
        EvaluationArm::ManualInstruction,
        "candidate-name",
        "candidate-body",
    );
    let candidate = fixture_prompt(
        fixture,
        EvaluationArm::CandidateSkill,
        "candidate-name",
        "candidate-body",
    );
    assert!(manual.contains("Guidance (JSON; null means none): {"));
    assert!(manual.contains("\"name\":\"generated-source-manual-control\""));
    assert!(manual.contains("\"content\":"));
    assert!(candidate.contains("Guidance (JSON; null means none): {"));
    assert!(candidate.contains("\"name\":\"candidate-name\""));
    assert!(candidate.contains("\"content\":\"candidate-body\""));
    assert!(!manual.contains("approved"));
    assert!(!candidate.contains("approved"));
    assert!(plan_system_prompt().contains("paths changed directly"));
    assert!(plan_system_prompt().contains("omit generated outputs"));
}

#[test]
fn discovery_uses_metadata_only_and_the_same_wrapper() {
    let fixture = APPLICABILITY_FIXTURES[0];
    let manual = applicability_prompt(
        fixture,
        EvaluationArm::ManualInstruction,
        "candidate-name",
        "candidate-description",
    );
    let candidate = applicability_prompt(
        fixture,
        EvaluationArm::CandidateSkill,
        "candidate-name",
        "candidate-description",
    );
    assert!(manual.contains("Guidance metadata (JSON; null means no additional guidance): {"));
    assert!(manual.contains("\"name\":\"generated-source-manual-control\""));
    assert!(manual.contains("\"description\":"));
    assert!(candidate.contains("Guidance metadata (JSON; null means no additional guidance): {"));
    assert!(candidate.contains("\"name\":\"candidate-name\""));
    assert!(candidate.contains("\"description\":\"candidate-description\""));
    assert!(!candidate.contains("candidate-body"));
}

#[test]
fn plan_and_applicability_schemas_deny_unknown_fields() {
    let fixture = FIXTURES[0];
    let mut plan: serde_json::Value =
        serde_json::from_str(&valid_plan(fixture)).expect("valid plan JSON");
    plan.as_object_mut()
        .expect("plan object")
        .insert("answer_key".to_string(), serde_json::Value::Bool(true));
    assert!(parse_plan(&plan.to_string()).is_none());
    assert!(
        parse_json::<ApplicabilityDecision>(r#"{"applicable":true,"reason":"extra"}"#).is_none()
    );
}

#[test]
fn every_incident_is_scored_once_and_requires_all_command_steps() {
    let ids = FIXTURES
        .iter()
        .map(|fixture| fixture.id)
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), FIXTURES.len());
    for fixture in FIXTURES {
        let result = score_completion(
            *fixture,
            EvaluationArm::CandidateSkill,
            completion(valid_plan(*fixture), "stop"),
        );
        assert!(
            result.passed,
            "{}: {:?}",
            fixture.id, result.failure_reasons
        );
    }
    let flatpak = FIXTURES[2];
    let incomplete = serde_json::json!({
        "diagnosis": "Update generated vendor inputs.",
        "edit_paths": flatpak.required_edit_paths,
        "commands": [flatpak.regeneration_steps[2][0]],
        "verification": [flatpak.verification_steps[0][0]],
    })
    .to_string();
    assert!(
        !score_completion(
            flatpak,
            EvaluationArm::CandidateSkill,
            completion(incomplete, "stop")
        )
        .passed
    );
}

#[test]
fn candidate_discovery_failure_is_always_an_install_regression() {
    let fixture = APPLICABILITY_FIXTURES[0];
    let trial = score_applicability(
        fixture,
        EvaluationArm::CandidateSkill,
        completion(r#"{"applicable":false}"#.to_string(), "stop"),
    );
    assert_eq!(
        candidate_regressions(&[trial]),
        vec![fixture.id.to_string()]
    );
}

#[test]
fn fenced_strict_json_is_consumed_whole() {
    let raw = "```json\n{\"diagnosis\":\"generated\",\"edit_paths\":[\"deploy/witness/docker-compose.yml\"],\"commands\":[\"deploy/witness/build-app-compose.sh\"],\"verification\":[\"deploy/witness/build-app-compose.sh --check\"]}\n```";
    let plan = parse_plan(raw).expect("fenced JSON");
    assert_eq!(plan.commands.len(), 1);
}
