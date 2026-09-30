use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use trace_commons_protocol::mission_evaluation::{
    MISSION_EVALUATION_MAX_CONCURRENCY, MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT,
    MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS, MISSION_EVALUATION_REQUIRED_MODEL_OWNER,
    MISSION_EVALUATION_SCHEMA_VERSION, MISSION_EVALUATION_TOTAL_REQUESTS, MISSION_EVALUATOR_ID,
    MissionEvaluationPackage, MissionEvaluationPolicy, SkillDraft, render_skill,
};
use uuid::Uuid;

use crate::skill_loop::evaluation::fixtures::FIXTURES;
use crate::skill_loop::evaluation::mission::{
    EVALUATION_CONTRACT_SOURCES, PreparedMissionEvaluation, SkillEvaluationObserver,
    compute_evaluation_contract_hash, evaluation_contract_hash,
};
use crate::skill_loop::evaluation::transport::NearAiEvaluationClient;
use crate::skill_loop::evaluation::{
    EVALUATION_CONCURRENCY, EvaluationArm, EvaluationUsage, SkillEvaluationError,
    SkillEvaluationReport, SkillTrialResult, abort_drain_error,
};
use crate::skill_loop::sha256;

const MODEL: &str = "deepseek-ai/DeepSeek-V4-Flash";

#[derive(Clone, Default)]
struct TestServerState {
    requests: Arc<AtomicUsize>,
    fail_at: Option<usize>,
    mismatched_model_at: Option<usize>,
    delay: Option<Duration>,
    slow_non_mismatch: bool,
}

async fn completion(State(state): State<TestServerState>, Json(_request): Json<Value>) -> Response {
    let index = state.requests.fetch_add(1, Ordering::SeqCst);
    let mismatched = state.mismatched_model_at == Some(index);
    if state.delay.is_some() || (state.slow_non_mismatch && !mismatched) {
        tokio::time::sleep(state.delay.unwrap_or(Duration::from_millis(200))).await;
    }
    if state.fail_at == Some(index) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let served_model = if mismatched {
        "nearai/unexpected-model"
    } else {
        MODEL
    };
    Json(serde_json::json!({
        "model": served_model,
        "choices": [{
            "message": {"role": "assistant", "content": "{}"},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 1,
            "reasoning_tokens": 0,
            "total_tokens": 2
        }
    }))
    .into_response()
}

async fn test_client(
    state: TestServerState,
) -> (
    NearAiEvaluationClient,
    TestServerState,
    tokio::task::JoinHandle<()>,
) {
    let app = Router::new()
        .route("/v1/chat/completions", post(completion))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback evaluator");
    let address = listener.local_addr().expect("loopback address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve loopback evaluator");
    });
    let client = NearAiEvaluationClient::at(
        format!("http://{address}/v1"),
        "synthetic-key".to_string(),
        MODEL.to_string(),
    )
    .expect("loopback client");
    (client, state, server)
}

fn package(task: &str) -> MissionEvaluationPackage {
    let skill = SkillDraft {
        name: "safe-candidate".to_string(),
        description: "Use for generated repository outputs.".to_string(),
        procedure: "# Repair\n\nChange the source and regenerate outputs.".to_string(),
    };
    MissionEvaluationPackage {
        schema_version: MISSION_EVALUATION_SCHEMA_VERSION,
        mission_id: Uuid::from_u128(1),
        program_id: Uuid::from_u128(2),
        offer_version_hash: format!("sha256:{}", "a".repeat(64)),
        task: task.to_string(),
        skill_sha256: sha256(render_skill(&skill).as_bytes()),
        skill,
        evaluator_id: MISSION_EVALUATOR_ID.to_string(),
        evaluation_contract_hash: evaluation_contract_hash(),
        execution: MissionEvaluationPolicy {
            required_model_owner: MISSION_EVALUATION_REQUIRED_MODEL_OWNER.to_string(),
            total_requests: MISSION_EVALUATION_TOTAL_REQUESTS,
            output_token_limit: MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT,
            request_timeout_seconds: MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS,
            max_concurrency: MISSION_EVALUATION_MAX_CONCURRENCY,
        },
    }
}

fn ordinary_package() -> MissionEvaluationPackage {
    package("Repair a generated GraphQL client from its source schema and run its drift check.")
}

fn closed_cancellation() -> watch::Receiver<bool> {
    let (sender, receiver) = watch::channel(false);
    drop(sender);
    receiver
}

#[derive(Default)]
struct RecordingObserver {
    trials: Mutex<Vec<SkillTrialResult>>,
    calls: AtomicUsize,
    fail_from: Option<usize>,
    cancel_on: Option<usize>,
    cancellation: Option<watch::Sender<bool>>,
}

impl RecordingObserver {
    fn recorded(&self) -> Vec<SkillTrialResult> {
        self.trials.lock().expect("observer lock").clone()
    }
}

#[async_trait]
impl SkillEvaluationObserver for RecordingObserver {
    async fn trial_completed(&self, trial: &SkillTrialResult) -> Result<(), SkillEvaluationError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_from.is_some_and(|first| call >= first) {
            return Err(SkillEvaluationError::RequestUnavailable);
        }
        self.trials
            .lock()
            .expect("observer lock")
            .push(trial.clone());
        if self.cancel_on == Some(call) {
            if let Some(sender) = &self.cancellation {
                sender.send(true).expect("runner still holds cancellation");
            }
        }
        Ok(())
    }
}

fn prepare_with_client(
    client: NearAiEvaluationClient,
    package: &MissionEvaluationPackage,
) -> PreparedMissionEvaluation {
    PreparedMissionEvaluation::prepare_with_client(client, package, Uuid::from_u128(9))
        .expect("valid prepared mission evaluation")
}

async fn run_negative(
    state: TestServerState,
    observer: &RecordingObserver,
    cancellation: watch::Receiver<bool>,
) -> (
    Result<SkillEvaluationReport, SkillEvaluationError>,
    TestServerState,
    tokio::task::JoinHandle<()>,
) {
    let (client, state, server) = test_client(state).await;
    let prepared = prepare_with_client(client, &ordinary_package());
    let result = prepared.run(observer, cancellation).await;
    (result, state, server)
}

#[tokio::test]
async fn stale_package_and_contract_precede_invalid_credential_catalog_access() {
    let mut stale_package = ordinary_package();
    stale_package.skill_sha256 = "c".repeat(64);
    assert!(matches!(
        PreparedMissionEvaluation::prepare(String::new(), &stale_package, Uuid::from_u128(9)).await,
        Err(SkillEvaluationError::InvalidPackage)
    ));

    let mut stale_contract = ordinary_package();
    stale_contract.evaluation_contract_hash = "d".repeat(64);
    assert!(matches!(
        PreparedMissionEvaluation::prepare(String::new(), &stale_contract, Uuid::from_u128(9))
            .await,
        Err(SkillEvaluationError::StaleContract)
    ));
    assert!(matches!(
        PreparedMissionEvaluation::prepare(String::new(), &ordinary_package(), Uuid::nil()).await,
        Err(SkillEvaluationError::InvalidApproval)
    ));
    assert_eq!(
        SkillEvaluationError::InvalidPackage.label(),
        "skill-evaluation-package-invalid"
    );
    assert_eq!(
        SkillEvaluationError::StaleContract.label(),
        "skill-evaluation-contract-stale"
    );
    assert_eq!(
        SkillEvaluationError::InvalidApproval.label(),
        "skill-evaluation-approval-invalid"
    );
}

#[test]
fn contract_hash_excludes_sibling_tests_and_binds_production_sources() {
    let baseline = compute_evaluation_contract_hash(EVALUATION_CONTRACT_SOURCES);
    assert_eq!(baseline, evaluation_contract_hash());
    for &(label, source) in EVALUATION_CONTRACT_SOURCES {
        let lines = source.lines().map(str::trim).collect::<Vec<_>>();
        assert!(
            !lines.contains(&"mod tests {"),
            "{label} contains an inline test module body"
        );
        assert!(
            !lines.windows(2).any(|lines| {
                (lines[0] == "#[test]" && lines[1].starts_with("fn "))
                    || (lines[0] == "#[tokio::test]" && lines[1].starts_with("async fn "))
            }),
            "{label} contains an inline test function"
        );
    }

    let mut changed_fixture = EVALUATION_CONTRACT_SOURCES.to_vec();
    let fixture_index = changed_fixture
        .iter()
        .position(|(label, _)| *label == "fixture-selection")
        .expect("fixture source is declared");
    changed_fixture[fixture_index] = ("fixture-selection", "mutated production fixture");
    assert_ne!(
        compute_evaluation_contract_hash(&changed_fixture),
        baseline,
        "production fixture changes must invalidate the exact-source pin"
    );

    let mut changed_policy = EVALUATION_CONTRACT_SOURCES.to_vec();
    let evaluator_index = changed_policy
        .iter()
        .position(|(label, _)| *label == "evaluator")
        .expect("evaluator source is declared");
    changed_policy[evaluator_index] = ("evaluator", "mutated production policy");
    assert_ne!(compute_evaluation_contract_hash(&changed_policy), baseline);
}

#[test]
fn contract_hash_is_crlf_normalized_domain_separated_and_length_framed() {
    assert_eq!(
        compute_evaluation_contract_hash(&[("source", "one\r\ntwo\r\n")]),
        compute_evaluation_contract_hash(&[("source", "one\ntwo\n")])
    );
    assert_ne!(
        compute_evaluation_contract_hash(&[("ab", "c")]),
        compute_evaluation_contract_hash(&[("a", "bc")])
    );
    assert_ne!(
        compute_evaluation_contract_hash(&[("source", "one")]),
        hex::encode(Sha256::digest(b"one"))
    );
}

#[tokio::test]
async fn mission_task_selects_source_excluded_trials_without_contribution_lineage() {
    let (client, state, server) = test_client(TestServerState::default()).await;
    let source = FIXTURES[0];
    let prepared = prepare_with_client(client, &package(source.task));
    let expected = prepared.expected_trials();

    assert_eq!(prepared.requested_model(), MODEL);
    assert_eq!(expected.len(), MISSION_EVALUATION_TOTAL_REQUESTS as usize);
    assert!(expected.iter().all(|trial| trial.task_id != source.id));
    assert_eq!(state.requests.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn full_negative_mission_evaluation_completes_a_bounded_report() {
    let observer = RecordingObserver::default();
    let (result, state, server) =
        run_negative(TestServerState::default(), &observer, closed_cancellation()).await;
    let report = result.expect("negative outcomes are valid evidence");

    assert_eq!(
        report.trials.len(),
        MISSION_EVALUATION_TOTAL_REQUESTS as usize
    );
    assert!(report.trials.iter().all(|trial| !trial.passed));
    assert!(!report.install_allowed);
    assert_eq!(
        observer.recorded().len(),
        MISSION_EVALUATION_TOTAL_REQUESTS as usize
    );
    assert_eq!(
        state.requests.load(Ordering::SeqCst),
        MISSION_EVALUATION_TOTAL_REQUESTS as usize
    );
    server.abort();
}

#[tokio::test]
async fn completed_partial_results_are_observed_before_http_failure_returns() {
    let observer = RecordingObserver::default();
    let state = TestServerState {
        fail_at: Some(4),
        ..TestServerState::default()
    };
    let (result, state, server) = run_negative(state, &observer, closed_cancellation()).await;

    assert_eq!(result, Err(SkillEvaluationError::RequestUnavailable));
    assert!(!observer.recorded().is_empty());
    assert!(observer.recorded().len() < MISSION_EVALUATION_TOTAL_REQUESTS as usize);
    assert!(state.requests.load(Ordering::SeqCst) < MISSION_EVALUATION_TOTAL_REQUESTS as usize);
    server.abort();
}

#[tokio::test]
async fn observer_failure_aborts_and_has_persistence_error_priority() {
    let observer = RecordingObserver {
        fail_from: Some(1),
        ..RecordingObserver::default()
    };
    let (result, state, server) =
        run_negative(TestServerState::default(), &observer, closed_cancellation()).await;

    assert_eq!(result, Err(SkillEvaluationError::PersistenceUnavailable));
    assert!(observer.recorded().is_empty());
    assert!(state.requests.load(Ordering::SeqCst) < MISSION_EVALUATION_TOTAL_REQUESTS as usize);
    server.abort();
}

#[tokio::test]
async fn drained_persistence_failure_overrides_a_distinct_provider_error() {
    let observer = RecordingObserver {
        fail_from: Some(1),
        ..RecordingObserver::default()
    };
    let trial = SkillTrialResult {
        task_id: "controlled-drain".into(),
        cluster: "controlled".into(),
        task: "Public controlled drain task.".into(),
        source_url: "https://example.invalid/controlled".into(),
        arm: EvaluationArm::Baseline,
        passed: false,
        failure_reasons: vec!["negative-result".into()],
        answer: None,
        raw_output: "{}".into(),
        request_id: Some("controlled-request".into()),
        served_model: MODEL.into(),
        finish_reason: "stop".into(),
        usage: EvaluationUsage {
            prompt_tokens: Some(1),
            completion_tokens: Some(1),
            reasoning_tokens: None,
            total_tokens: Some(2),
        },
    };
    let mut jobs = tokio::task::JoinSet::new();
    let (completed, completion_observed) = tokio::sync::oneshot::channel();
    jobs.spawn(async move {
        let _ = completed.send(());
        (None, Ok::<SkillTrialResult, SkillEvaluationError>(trial))
    });
    completion_observed
        .await
        .expect("controlled job reached its ready return");

    let mut trials = Vec::new();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(EVALUATION_CONCURRENCY));
    let result = abort_drain_error(
        &mut jobs,
        &semaphore,
        MODEL,
        &observer,
        &mut trials,
        SkillEvaluationError::RequestUnavailable,
    )
    .await;

    assert_eq!(result, SkillEvaluationError::PersistenceUnavailable);
    assert!(trials.is_empty());
    // The gate closes before the abort, so nothing parked on a permit can slip
    // one more paid request in while the drain awaits.
    assert!(semaphore.is_closed());
    assert!(
        semaphore.clone().acquire_owned().await.is_err(),
        "no queued sibling can take a permit once the run is aborted"
    );
}

#[tokio::test]
async fn already_cancelled_mission_issues_zero_inference_requests() {
    let observer = RecordingObserver::default();
    let (sender, receiver) = watch::channel(true);
    let (result, state, server) =
        run_negative(TestServerState::default(), &observer, receiver).await;

    assert_eq!(result, Err(SkillEvaluationError::Cancelled));
    assert_eq!(state.requests.load(Ordering::SeqCst), 0);
    assert!(observer.recorded().is_empty());
    drop(sender);
    server.abort();
}

#[tokio::test]
async fn midflight_cancellation_drains_and_never_starts_queued_requests() {
    let state = TestServerState {
        delay: Some(Duration::from_millis(500)),
        ..TestServerState::default()
    };
    let (client, state, server) = test_client(state).await;
    let prepared = prepare_with_client(client, &ordinary_package());
    let observer = Arc::new(RecordingObserver::default());
    let run_observer = Arc::clone(&observer);
    let (sender, receiver) = watch::channel(false);
    let run = tokio::spawn(async move { prepared.run(run_observer.as_ref(), receiver).await });

    tokio::time::timeout(Duration::from_secs(1), async {
        while state.requests.load(Ordering::SeqCst) < MISSION_EVALUATION_MAX_CONCURRENCY as usize {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("initial requests start");
    sender.send(true).expect("runner holds cancellation");
    let result = tokio::time::timeout(Duration::from_secs(1), run)
        .await
        .expect("cancelled runner drains")
        .expect("runner task");
    tokio::time::sleep(Duration::from_millis(550)).await;

    assert_eq!(result, Err(SkillEvaluationError::Cancelled));
    assert_eq!(
        state.requests.load(Ordering::SeqCst),
        MISSION_EVALUATION_MAX_CONCURRENCY as usize
    );
    assert!(observer.recorded().is_empty());
    server.abort();
}

#[tokio::test]
async fn closed_false_cancellation_receiver_completes_without_spin() {
    let observer = RecordingObserver::default();
    let (client, state, server) = test_client(TestServerState::default()).await;
    let prepared = prepare_with_client(client, &ordinary_package());
    let report = tokio::time::timeout(
        Duration::from_secs(3),
        prepared.run(&observer, closed_cancellation()),
    )
    .await
    .expect("closed receiver does not spin")
    .expect("evaluation completes");

    assert_eq!(
        report.trials.len(),
        MISSION_EVALUATION_TOTAL_REQUESTS as usize
    );
    assert_eq!(
        state.requests.load(Ordering::SeqCst),
        MISSION_EVALUATION_TOTAL_REQUESTS as usize
    );
    server.abort();
}

#[tokio::test]
async fn served_model_mismatch_is_never_sent_to_observer() {
    let observer = RecordingObserver::default();
    let state = TestServerState {
        mismatched_model_at: Some(0),
        slow_non_mismatch: true,
        ..TestServerState::default()
    };
    let (result, state, server) = run_negative(state, &observer, closed_cancellation()).await;

    assert_eq!(result, Err(SkillEvaluationError::ModelChanged));
    assert!(
        observer
            .recorded()
            .iter()
            .all(|trial| trial.served_model == MODEL)
    );
    assert!(state.requests.load(Ordering::SeqCst) < MISSION_EVALUATION_TOTAL_REQUESTS as usize);
    server.abort();
}

#[tokio::test]
async fn cancellation_after_last_completion_does_not_replace_the_report() {
    let (sender, receiver) = watch::channel(false);
    let observer = RecordingObserver {
        cancel_on: Some(MISSION_EVALUATION_TOTAL_REQUESTS as usize),
        cancellation: Some(sender),
        ..RecordingObserver::default()
    };
    let (result, state, server) =
        run_negative(TestServerState::default(), &observer, receiver).await;
    let report = result.expect("completed report wins the cancellation race");

    assert_eq!(
        report.trials.len(),
        MISSION_EVALUATION_TOTAL_REQUESTS as usize
    );
    assert_eq!(
        state.requests.load(Ordering::SeqCst),
        MISSION_EVALUATION_TOTAL_REQUESTS as usize
    );
    server.abort();
}
