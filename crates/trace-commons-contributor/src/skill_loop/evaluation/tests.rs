use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{Json, Router, extract::State, routing::post};
use serde_json::Value;

use super::*;
use crate::skill_loop::{SkillDraft, render_skill, sha256, source_task_fingerprint};

const MODEL: &str = "deepseek-ai/DeepSeek-V4-Flash";

#[derive(Clone, Default)]
struct RecordingState {
    requests: Arc<Mutex<Vec<Value>>>,
    request_sequence: Arc<AtomicUsize>,
    active_requests: Arc<AtomicUsize>,
    peak_requests: Arc<AtomicUsize>,
    fail_positive_discovery: bool,
    metadata_candidate_only: bool,
    equal_plan_performance: bool,
    delay_requests: bool,
    mismatched_model_on_request: Option<usize>,
    slow_matching_requests: bool,
}

struct ActiveRequest(Arc<AtomicUsize>);

impl Drop for ActiveRequest {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn arm_from_prompt(prompt: &str) -> EvaluationArm {
    if prompt.contains("\"name\":\"safe-candidate\"") {
        EvaluationArm::CandidateSkill
    } else if prompt.contains(crate::skill_loop::MANUAL_CONTROL_INSTRUCTION) {
        EvaluationArm::ManualInstruction
    } else {
        EvaluationArm::Baseline
    }
}

fn fixture_from_prompt(prompt: &str) -> fixtures::EvaluationFixture {
    FIXTURES
        .iter()
        .copied()
        .find(|fixture| prompt.contains(fixture.task))
        .expect("known fixture prompt")
}

fn successful_plan(fixture: fixtures::EvaluationFixture) -> Value {
    serde_json::json!({
        "diagnosis": "The source owns generated outputs.",
        "edit_paths": fixture.required_edit_paths,
        "commands": fixture.regeneration_steps.iter().map(|step| step[0]).collect::<Vec<_>>(),
        "verification": fixture.verification_steps.iter().map(|step| step[0]).collect::<Vec<_>>(),
    })
}

async fn completion(
    State(state): State<RecordingState>,
    Json(request): Json<Value>,
) -> Json<Value> {
    let request_index = state.request_sequence.fetch_add(1, Ordering::SeqCst);
    let active = state.active_requests.fetch_add(1, Ordering::SeqCst) + 1;
    state.peak_requests.fetch_max(active, Ordering::SeqCst);
    let _active_request = ActiveRequest(Arc::clone(&state.active_requests));
    state
        .requests
        .lock()
        .expect("recording lock")
        .push(request.clone());
    if state.delay_requests
        || (state.slow_matching_requests
            && state.mismatched_model_on_request != Some(request_index))
    {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let messages = request["messages"].as_array().expect("messages");
    let system = messages[0]["content"].as_str().expect("system prompt");
    let user = messages[1]["content"].as_str().expect("fixture prompt");
    let arm = arm_from_prompt(user);
    let output = if system.contains("applicability") {
        let fixture = APPLICABILITY_FIXTURES
            .iter()
            .copied()
            .find(|fixture| user.contains(fixture.task))
            .expect("known applicability prompt");
        let applicable = if state.metadata_candidate_only {
            if arm == EvaluationArm::CandidateSkill {
                fixture.guidance_should_apply
            } else if arm == EvaluationArm::Baseline {
                true
            } else {
                !fixture.guidance_should_apply
            }
        } else if state.fail_positive_discovery && fixture.guidance_should_apply {
            arm == EvaluationArm::Baseline
        } else {
            fixture.guidance_should_apply && arm != EvaluationArm::Baseline
        };
        serde_json::json!({"applicable": applicable})
    } else {
        let fixture = fixture_from_prompt(user);
        let fixture_index = FIXTURES
            .iter()
            .position(|candidate| candidate.id == fixture.id)
            .expect("fixture index");
        let should_pass = state.equal_plan_performance
            || match arm {
                EvaluationArm::Baseline => fixture_index < 1,
                EvaluationArm::ManualInstruction => fixture_index < 2,
                EvaluationArm::CandidateSkill => true,
            };
        if should_pass {
            successful_plan(fixture)
        } else {
            serde_json::json!({
                "diagnosis": "Edit the visible result.",
                "edit_paths": [],
                "commands": [],
                "verification": [],
            })
        }
    };
    let served_model = if state.mismatched_model_on_request == Some(request_index) {
        "nearai/unexpected-model"
    } else {
        MODEL
    };
    Json(serde_json::json!({
        "model": served_model,
        "choices": [{
            "message": {"role": "assistant", "content": output.to_string()},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 40,
            "completion_tokens": 30,
            "reasoning_tokens": 0,
            "total_tokens": 70
        }
    }))
}

fn review() -> SkillReview {
    let draft = SkillDraft {
        name: "safe-candidate".to_string(),
        description: "PUBLIC-CANDIDATE-DESCRIPTION for generated repository outputs".to_string(),
        procedure: "# SAFE-CANDIDATE-SKILL\n\nPRIVATE-PROCEDURE-BODY".to_string(),
    };
    let skill_md = render_skill(&draft);
    SkillReview {
        review_id: Uuid::new_v4(),
        candidate_id: Uuid::new_v4(),
        family: "generated-source-repair".to_string(),
        source_submission_id: Uuid::new_v4(),
        source_evidence_ids: vec![Uuid::new_v4()],
        source_task_fingerprint: source_task_fingerprint(
            "Repair a generated GraphQL client from its source schema and run its drift check.",
        )
        .expect("source fingerprint"),
        draft,
        skill_sha256: sha256(skill_md.as_bytes()),
        skill_md,
    }
}

fn default_generated_source_review() -> SkillReview {
    let draft = SkillDraft {
        name: crate::skill_loop::DEFAULT_SKILL_NAME.to_string(),
        description: crate::skill_loop::DEFAULT_DESCRIPTION.to_string(),
        procedure: crate::skill_loop::DEFAULT_PROCEDURE.to_string(),
    };
    let skill_md = render_skill(&draft);
    SkillReview {
        review_id: Uuid::new_v4(),
        candidate_id: Uuid::new_v4(),
        family: crate::skill_loop::GENERATED_SOURCE_FAMILY.to_string(),
        source_submission_id: Uuid::new_v4(),
        source_evidence_ids: vec![Uuid::new_v4()],
        source_task_fingerprint: source_task_fingerprint(
            "Repair a generated GraphQL client from its source schema and run its drift check.",
        )
        .expect("source fingerprint"),
        draft,
        skill_sha256: sha256(skill_md.as_bytes()),
        skill_md,
    }
}

#[tokio::test]
async fn three_arm_evaluation_uses_24_equal_budget_public_requests_and_gates() {
    let state = RecordingState::default();
    let app = Router::new()
        .route("/v1/chat/completions", post(completion))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let client = NearAiEvaluationClient::at(
        format!("http://{address}/v1"),
        "synthetic-key".to_string(),
        MODEL.to_string(),
    )
    .expect("client");
    let private_review = review();

    let report = evaluate_with_client(client, &private_review)
        .await
        .expect("evaluation");
    server.abort();

    assert_eq!(report.trials.len(), 24);
    assert_eq!(report.contract.task_count, 8);
    assert_eq!(report.contract.plan_task_count, PLAN_TASK_COUNT);
    assert_eq!(report.contract.fixture_pool_count, 7);
    assert_eq!(report.contract.cluster_count, 7);
    assert_eq!(report.execution.requested_model, MODEL);
    assert_eq!(report.execution.served_model, MODEL);
    assert_eq!(report.contract.output_token_limit, OUTPUT_TOKEN_LIMIT);
    assert_eq!(
        report.contract.request_timeout_seconds,
        REQUEST_TIMEOUT_SECS
    );
    let summaries = report
        .summaries
        .iter()
        .map(|summary| (summary.arm, summary.passed))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(summaries[&EvaluationArm::Baseline], 3);
    assert_eq!(summaries[&EvaluationArm::ManualInstruction], 4);
    assert_eq!(summaries[&EvaluationArm::CandidateSkill], 8);
    let plan_summaries = report
        .plan_summaries
        .iter()
        .map(|summary| (summary.arm, summary.passed))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(plan_summaries[&EvaluationArm::Baseline], 1);
    assert_eq!(plan_summaries[&EvaluationArm::ManualInstruction], 2);
    assert_eq!(plan_summaries[&EvaluationArm::CandidateSkill], 6);
    assert!(
        report
            .applicability_summaries
            .iter()
            .all(|summary| summary.passed == 2 && summary.total == 2)
    );
    assert!(report.regressions.is_empty());
    assert!(report.install_allowed);
    assert_eq!(report.gate_reason, "skill-improved-over-both-controls");
    assert_eq!(
        report.execution.selected_plan_task_ids.len(),
        PLAN_TASK_COUNT
    );
    assert!(report.execution.excluded_source_overlap_task_ids.is_empty());
    assert_eq!(
        report.execution.reserve_plan_task_ids,
        vec![FIXTURES[PLAN_TASK_COUNT].id]
    );
    assert_eq!(
        report.execution.source_task_fingerprint_sha256,
        private_review.source_task_fingerprint.sha256
    );
    let wire = serde_json::to_value(&report).expect("serialize evaluation report");
    assert_eq!(wire["arm_count"], EvaluationArm::ALL.len());
    assert_eq!(wire["total_requests"], 24);
    assert_eq!(wire["required_model_owner"], "nearai");
    assert!(wire.get("requests_per_task").is_none());

    let requests = state.requests.lock().expect("recording lock");
    assert_eq!(requests.len(), 24);
    let mut seen = BTreeMap::new();
    for (index, request) in requests.iter().enumerate() {
        assert_eq!(request["model"], MODEL);
        assert_eq!(request["max_tokens"], OUTPUT_TOKEN_LIMIT);
        assert_eq!(request["temperature"], 0);
        assert_eq!(request["response_format"]["type"], "json_object");
        assert_eq!(request["reasoning"]["enabled"], false);
        let wire = request.to_string();
        assert!(!wire.contains(&private_review.source_submission_id.to_string()));
        assert!(!wire.contains(&private_review.source_evidence_ids[0].to_string()));
        assert!(!wire.contains(&private_review.source_task_fingerprint.sha256));

        let messages = request["messages"].as_array().expect("messages");
        let system = messages[0]["content"].as_str().expect("system");
        let user = messages[1]["content"].as_str().expect("user");
        let arm = arm_from_prompt(user);
        let task_id = if system.contains("applicability") {
            assert!(index < 6, "body request ran before discovery completed");
            assert!(!user.contains("PRIVATE-PROCEDURE-BODY"));
            APPLICABILITY_FIXTURES
                .iter()
                .find(|fixture| user.contains(fixture.task))
                .expect("applicability fixture")
                .id
        } else {
            fixture_from_prompt(user).id
        };
        let key = (task_id.to_string(), arm);
        *seen.entry(key).or_insert(0_usize) += 1;
    }
    assert_eq!(seen.len(), 24);
    assert!(seen.values().all(|count| *count == 1));
}

#[tokio::test]
async fn digest_mismatch_is_rejected_before_any_model_call() {
    let state = RecordingState::default();
    let app = Router::new()
        .route("/v1/chat/completions", post(completion))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let client = NearAiEvaluationClient::at(
        format!("http://{address}/v1"),
        "synthetic-key".to_string(),
        MODEL.to_string(),
    )
    .expect("client");
    let mut changed = review();
    changed.skill_md.push_str("\nchanged after review");
    assert_eq!(
        evaluate_with_client(client, &changed).await,
        Err(SkillEvaluationError::Internal)
    );
    assert!(state.requests.lock().expect("recording lock").is_empty());
    server.abort();
}

#[tokio::test]
async fn metadata_failure_blocks_install_when_candidate_still_beats_both_controls() {
    let state = RecordingState {
        fail_positive_discovery: true,
        ..RecordingState::default()
    };
    let app = Router::new()
        .route("/v1/chat/completions", post(completion))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let client = NearAiEvaluationClient::at(
        format!("http://{address}/v1"),
        "synthetic-key".to_string(),
        MODEL.to_string(),
    )
    .expect("client");

    let report = evaluate_with_client(client, &review())
        .await
        .expect("evaluation");
    server.abort();

    let summaries = report
        .summaries
        .iter()
        .map(|summary| (summary.arm, summary.passed))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(summaries[&EvaluationArm::Baseline], 2);
    assert_eq!(summaries[&EvaluationArm::ManualInstruction], 3);
    assert_eq!(summaries[&EvaluationArm::CandidateSkill], 7);
    assert!(!report.install_allowed);
    assert_eq!(report.gate_reason, "skill-failed-metadata-applicability");
}

#[tokio::test]
async fn applicability_advantage_cannot_mask_zero_plan_improvement() {
    let state = RecordingState {
        metadata_candidate_only: true,
        equal_plan_performance: true,
        ..RecordingState::default()
    };
    let app = Router::new()
        .route("/v1/chat/completions", post(completion))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let client = NearAiEvaluationClient::at(
        format!("http://{address}/v1"),
        "synthetic-key".to_string(),
        MODEL.to_string(),
    )
    .expect("client");

    let report = evaluate_with_client(client, &review())
        .await
        .expect("evaluation");
    server.abort();

    assert_eq!(report.summaries[0].passed, 6);
    assert_eq!(report.summaries[1].passed, 6);
    assert_eq!(report.summaries[2].passed, 8);
    assert!(
        report
            .plan_summaries
            .iter()
            .all(|summary| summary.passed == PLAN_TASK_COUNT)
    );
    assert!(report.regressions.is_empty());
    assert!(!report.install_allowed);
    assert_eq!(report.gate_reason, "skill-did-not-beat-both-controls");
}

#[test]
fn reviewed_digest_freezes_metadata_and_body_as_one_candidate() {
    let reviewed = review();
    assert!(review_candidate_is_frozen(&reviewed));

    let mut changed_metadata = reviewed.clone();
    changed_metadata.draft.description.push_str(" changed");
    assert!(!review_candidate_is_frozen(&changed_metadata));

    let mut changed_body = reviewed;
    changed_body.skill_md.push_str("\nchanged");
    assert!(!review_candidate_is_frozen(&changed_body));
}

#[test]
fn plan_schedule_independently_counterbalances_every_arm_position() {
    let schedules = (0..evaluation_contract().plan_task_count)
        .map(scheduled_arms)
        .collect::<Vec<_>>();
    for position in 0..EvaluationArm::ALL.len() {
        let counts = EvaluationArm::ALL
            .into_iter()
            .map(|arm| {
                schedules
                    .iter()
                    .filter(|schedule| schedule[position] == arm)
                    .count()
            })
            .collect::<Vec<_>>();
        assert_eq!(counts, vec![2, 2, 2]);
    }
}

#[tokio::test]
async fn model_change_aborts_queued_requests_immediately() {
    let state = RecordingState {
        mismatched_model_on_request: Some(0),
        slow_matching_requests: true,
        ..RecordingState::default()
    };
    let app = Router::new()
        .route("/v1/chat/completions", post(completion))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let client = NearAiEvaluationClient::at(
        format!("http://{address}/v1"),
        "synthetic-key".to_string(),
        MODEL.to_string(),
    )
    .expect("client");

    assert_eq!(
        evaluate_with_client(client, &review()).await,
        Err(SkillEvaluationError::ModelChanged)
    );
    let issued = state.request_sequence.load(Ordering::SeqCst);
    assert!((1..=EVALUATION_CONCURRENCY).contains(&issued));
    server.abort();
}

#[tokio::test]
async fn evaluation_never_exceeds_the_request_concurrency_ceiling() {
    let state = RecordingState {
        delay_requests: true,
        ..RecordingState::default()
    };
    let app = Router::new()
        .route("/v1/chat/completions", post(completion))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let client = NearAiEvaluationClient::at(
        format!("http://{address}/v1"),
        "synthetic-key".to_string(),
        MODEL.to_string(),
    )
    .expect("client");

    evaluate_with_client(client, &review())
        .await
        .expect("evaluation");
    assert_eq!(
        state.peak_requests.load(Ordering::SeqCst),
        EVALUATION_CONCURRENCY
    );
    assert_eq!(state.active_requests.load(Ordering::SeqCst), 0);
    server.abort();
}

#[test]
fn funding_error_has_a_distinct_wire_label() {
    assert_eq!(
        SkillEvaluationError::FundingRequired.label(),
        "skill-evaluation-funding-required"
    );
}

#[tokio::test]
#[ignore = "requires a NEAR AI inference key and makes the full 24-request comparison"]
async fn live_default_skill_beats_both_controls_before_installation() {
    let api_key = std::env::var("NEARAI_API_KEY")
        .expect("NEARAI_API_KEY must be set when the ignored live gate is requested");
    assert!(
        !api_key.trim().is_empty(),
        "NEARAI_API_KEY must be nonempty when the ignored live gate is requested"
    );

    let report = evaluate_skill(api_key, &default_generated_source_review())
        .await
        .expect("live full comparison");
    assert!(report.install_allowed, "{}", report.gate_reason);
    assert!(report.regressions.is_empty());
    let passed = report
        .plan_summaries
        .iter()
        .map(|summary| (summary.arm, summary.passed))
        .collect::<BTreeMap<_, _>>();
    eprintln!(
        "live skill gate: model={} plans baseline={}/{} manual={}/{} candidate={}/{} applicability={}/{} regressions={}",
        report.execution.served_model,
        passed[&EvaluationArm::Baseline],
        PLAN_TASK_COUNT,
        passed[&EvaluationArm::ManualInstruction],
        PLAN_TASK_COUNT,
        passed[&EvaluationArm::CandidateSkill],
        PLAN_TASK_COUNT,
        report
            .applicability_summaries
            .iter()
            .find(|summary| summary.arm == EvaluationArm::CandidateSkill)
            .map_or(0, |summary| summary.passed),
        APPLICABILITY_FIXTURES.len(),
        report.regressions.len()
    );
    assert!(passed[&EvaluationArm::CandidateSkill] > passed[&EvaluationArm::Baseline]);
    assert!(passed[&EvaluationArm::CandidateSkill] > passed[&EvaluationArm::ManualInstruction]);
}
