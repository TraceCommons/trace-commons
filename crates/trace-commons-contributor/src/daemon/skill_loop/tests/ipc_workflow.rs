//! INTEGRATION: drives the reviewed skill lifecycle through the daemon dispatcher.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::account_auth::AccountSession;
use crate::config::ACCOUNT_SESSION_FILE;
use crate::daemon::ipc::{
    DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response, handle_request_async,
};

use super::*;

fn save_account_session(shared: &DaemonShared, account_id: &str) {
    let session = AccountSession {
        access_token: "synthetic-test-session".to_string(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        account_id: account_id.to_string(),
    };
    shared
        .store
        .write_daemon_file(
            ACCOUNT_SESSION_FILE,
            &serde_json::to_vec(&session).expect("serialize account session fixture"),
        )
        .expect("save account session fixture");
}

fn configured_shared() -> (tempfile::TempDir, DaemonShared) {
    let (directory, store) = crate::config::tests_support::temp_store();
    let identity =
        crate::identity::DeviceIdentity::load_or_generate(&store).expect("test device identity");
    let mut config = crate::commands::unenrolled_preview_config();
    config.issuer_url = "https://issuer.test.invalid".to_string();
    config.ingest_url = "https://ingest.test.invalid".to_string();
    config.audience = "synthetic-audience".to_string();
    config.tenant_id = "synthetic-tenant".to_string();
    config.instance_id = "synthetic-instance".to_string();
    config.user_subject = "synthetic-subject".to_string();
    config.device_key_id = identity.device_key_id;
    store
        .save_config(&config)
        .expect("save contributor config fixture");
    let shared = DaemonShared::load(store).expect("load daemon fixture");
    save_account_session(&shared, "owner-a.test");
    (directory, shared)
}

async fn dispatch_wire(shared: &DaemonShared, id: u64, method: &str, params: Value) -> Response {
    let request: Request = serde_json::from_value(json!({
        "id": id,
        "method": method,
        "params": params,
    }))
    .expect("decode request wire fixture");
    let response = handle_request_async(shared, &request).await;
    serde_json::from_slice(&serde_json::to_vec(&response).expect("encode response wire fixture"))
        .expect("decode response wire fixture")
}

fn successful_result(response: Response, expected_id: u64) -> Value {
    assert_eq!(response.id, expected_id);
    assert!(
        response.error.is_none(),
        "unexpected error: {:?}",
        response.error
    );
    response.result.expect("successful result")
}

fn assert_error(response: Response, expected_id: u64, code: &str, message: &str) {
    assert_eq!(response.id, expected_id);
    assert!(response.result.is_none());
    let error = response.error.expect("error response");
    assert_eq!(error.code, code);
    assert_eq!(error.message, message);
}

fn assert_workflow_phase(shared: &DaemonShared, source_submission_id: Uuid, expected: &str) {
    let state = shared.skill_loop.lock().expect("skill-loop state");
    let workflow = state
        .workflows
        .iter()
        .find(|workflow| workflow.candidate.source_submission_id == source_submission_id)
        .expect("held workflow");
    assert_eq!(phase_name(&workflow.phase), expected);
}

#[tokio::test]
async fn account_switch_after_install_admission_prevents_filesystem_publication() {
    let (_state_directory, shared) = configured_shared();
    let skills_directory = tempfile::tempdir().expect("temporary Codex home");
    let skills_root = skills_directory.path().join("skills");
    let admitted = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let hook_admitted = Arc::clone(&admitted);
    let hook_release = Arc::clone(&release);
    let _runtime = install_test_skill_loop_runtime(
        &shared,
        TestSkillLoopRuntime {
            evaluate: Arc::new(report),
            codex_skills_root: skills_root.clone(),
            before_install_mutation: Some(Arc::new(move || {
                hook_admitted.wait();
                hook_release.wait();
            })),
        },
    );

    let source_submission_id = Uuid::new_v4();
    let candidate = candidate(source_submission_id);
    let review = reviewed(&candidate);
    let evaluation = report(&review);
    let owner_a =
        crate::daemon::public_run::current_account_binding(&shared, 70).expect("owner A binding");
    {
        let mut state = shared.skill_loop.lock().expect("skill-loop state");
        state.bind_owner(&owner_a);
        state.hold_candidate(candidate).expect("hold candidate");
        state.hold_review(review, None).expect("hold review");
        assert!(matches!(
            state.begin_evaluation(evaluation.review_id),
            EvaluationStart::Started
        ));
        state.insert_evaluation(evaluation.clone());
    }
    let plan = successful_result(
        dispatch_wire(
            &shared,
            71,
            "skill_install_plan",
            json!({"evaluation_id": evaluation.evaluation_id}),
        )
        .await,
        71,
    );
    let request: Request = serde_json::from_value(json!({
        "id": 72,
        "method": "skill_install_commit",
        "params": {
            "plan_id": plan["plan_id"],
            "as_previewed_sha256": plan["skill_sha256"],
            "as_previewed_marker_sha256": plan["marker_file_sha256"],
        },
    }))
    .expect("commit request");

    let response = std::thread::scope(|scope| {
        let worker = scope.spawn(|| handle_install_commit(&shared, &request));
        admitted.wait();
        save_account_session(&shared, "owner-b.test");
        let owner_b = crate::daemon::public_run::current_account_binding(&shared, 73)
            .expect("owner B binding");
        shared
            .skill_loop
            .lock()
            .expect("skill-loop state")
            .bind_owner(&owner_b);
        release.wait();
        worker.join().expect("commit worker")
    });

    assert_error(response, 72, ERR_UNAVAILABLE, "skill-owner-changed");
    assert!(!skills_root.join("repair-generated-sources").exists());
    let current_owner = crate::daemon::public_run::current_account_binding(&shared, 74)
        .expect("current owner binding");
    let state = shared.skill_loop.lock().expect("skill-loop state");
    assert!(state.install_receipt(source_submission_id).is_none());
    assert_eq!(state.owner_scope.as_deref(), Some(current_owner.as_str()));
}

#[tokio::test]
async fn ipc_review_evaluate_install_status_owner_binding_and_rollback_round_trip() {
    let (_state_directory, shared) = configured_shared();
    let skills_directory = tempfile::tempdir().expect("temporary Codex home");
    let skills_root = skills_directory.path().join("skills");
    let evaluation_calls = Arc::new(AtomicUsize::new(0));
    let observed_calls = Arc::clone(&evaluation_calls);
    let _runtime = install_test_skill_loop_runtime(
        &shared,
        TestSkillLoopRuntime {
            evaluate: Arc::new(move |review| {
                observed_calls.fetch_add(1, Ordering::SeqCst);
                report(review)
            }),
            codex_skills_root: skills_root.clone(),
            before_install_mutation: None,
        },
    );

    let source_submission_id = Uuid::new_v4();
    let held_candidate = candidate(source_submission_id);
    let owner_a = crate::daemon::public_run::current_account_binding(&shared, 1)
        .expect("authenticated owner binding");
    {
        let mut state = shared.skill_loop.lock().expect("skill-loop state");
        state.bind_owner(&owner_a);
        state
            .hold_candidate(held_candidate.clone())
            .expect("hold source candidate");
    }

    let malformed_review = dispatch_wire(
        &shared,
        10,
        "skill_review",
        json!({
            "candidate_id": held_candidate.candidate_id,
            "draft": {"name": 7, "description": "fixture", "procedure": "fixture"},
        }),
    )
    .await;
    assert_error(malformed_review, 10, ERR_BAD_PARAMS, "skill-review-invalid");
    assert_workflow_phase(&shared, source_submission_id, "candidate");

    let review_value = successful_result(
        dispatch_wire(
            &shared,
            11,
            "skill_review",
            json!({
                "candidate_id": held_candidate.candidate_id,
                "draft": held_candidate.draft,
            }),
        )
        .await,
        11,
    );
    assert!(review_value.get("source_correction").is_none());
    assert!(review_value.get("source_evidence").is_none());
    let review: crate::skill_loop::SkillReview =
        serde_json::from_value(review_value).expect("decode reviewed skill response");
    assert_eq!(review.candidate_id, held_candidate.candidate_id);
    assert_workflow_phase(&shared, source_submission_id, "reviewed");

    let stale_evaluation = dispatch_wire(
        &shared,
        12,
        "skill_evaluate",
        json!({
            "review_id": review.review_id,
            "skill_sha256": "0".repeat(64),
        }),
    )
    .await;
    assert_error(stale_evaluation, 12, ERR_BAD_PARAMS, "skill-review-changed");
    assert_eq!(evaluation_calls.load(Ordering::SeqCst), 0);
    assert_workflow_phase(&shared, source_submission_id, "reviewed");

    let evaluation_value = successful_result(
        dispatch_wire(
            &shared,
            13,
            "skill_evaluate",
            json!({
                "review_id": review.review_id,
                "skill_sha256": review.skill_sha256,
            }),
        )
        .await,
        13,
    );
    assert_eq!(evaluation_value["install_allowed"], true);
    assert_eq!(evaluation_value["review_id"], review.review_id.to_string());
    assert_eq!(evaluation_value["skill_sha256"], review.skill_sha256);
    let evaluation_id = evaluation_value["evaluation_id"]
        .as_str()
        .expect("evaluation id")
        .parse::<Uuid>()
        .expect("valid evaluation id");
    assert_eq!(evaluation_calls.load(Ordering::SeqCst), 1);
    assert_workflow_phase(&shared, source_submission_id, "evaluated");

    let first_plan = successful_result(
        dispatch_wire(
            &shared,
            14,
            "skill_install_plan",
            json!({"evaluation_id": evaluation_id}),
        )
        .await,
        14,
    );
    assert_eq!(
        first_plan["target_location"],
        "$CODEX_HOME/skills/repair-generated-sources"
    );
    assert_eq!(
        first_plan["skill_location"],
        "$CODEX_HOME/skills/repair-generated-sources/SKILL.md"
    );
    assert_eq!(first_plan["can_install"], true);
    assert!(
        !serde_json::to_string(&first_plan)
            .expect("serialize plan assertion")
            .contains(skills_directory.path().to_string_lossy().as_ref())
    );
    assert_workflow_phase(&shared, source_submission_id, "planned");

    let first_plan_id = first_plan["plan_id"]
        .as_str()
        .expect("plan id")
        .parse::<Uuid>()
        .expect("valid plan id");
    let first_marker_digest = first_plan["marker_file_sha256"]
        .as_str()
        .expect("marker digest");
    let stale_commit = dispatch_wire(
        &shared,
        15,
        "skill_install_commit",
        json!({
            "plan_id": first_plan_id,
            "as_previewed_sha256": "f".repeat(64),
            "as_previewed_marker_sha256": first_marker_digest,
        }),
    )
    .await;
    assert_error(
        stale_commit,
        15,
        ERR_UNAVAILABLE,
        "skill-install-plan-changed",
    );
    assert!(!skills_root.join("repair-generated-sources").exists());
    assert_workflow_phase(&shared, source_submission_id, "evaluated");

    let plan = successful_result(
        dispatch_wire(
            &shared,
            16,
            "skill_install_plan",
            json!({"evaluation_id": evaluation_id}),
        )
        .await,
        16,
    );
    let plan_id = plan["plan_id"]
        .as_str()
        .expect("replacement plan id")
        .parse::<Uuid>()
        .expect("valid replacement plan id");
    assert_ne!(plan_id, first_plan_id);
    let skill_digest = plan["skill_sha256"]
        .as_str()
        .expect("skill digest")
        .to_string();
    let marker_digest = plan["marker_file_sha256"]
        .as_str()
        .expect("marker digest")
        .to_string();
    let installed = successful_result(
        dispatch_wire(
            &shared,
            17,
            "skill_install_commit",
            json!({
                "plan_id": plan_id,
                "as_previewed_sha256": skill_digest,
                "as_previewed_marker_sha256": marker_digest,
            }),
        )
        .await,
        17,
    );
    let install_id = installed["install_id"]
        .as_str()
        .expect("install id")
        .parse::<Uuid>()
        .expect("valid install id");
    assert_eq!(
        installed["source_submission_id"],
        source_submission_id.to_string()
    );
    assert_eq!(installed["skill_sha256"], review.skill_sha256);
    assert!(installed.get("local_target_path").is_none());
    let installed_marker_digest = installed["marker_sha256"]
        .as_str()
        .expect("installed marker digest")
        .to_string();
    assert_workflow_phase(&shared, source_submission_id, "evaluated");
    assert!(
        shared
            .skill_loop
            .lock()
            .expect("skill-loop state")
            .install_receipt(source_submission_id)
            .is_some()
    );

    let target = skills_root.join("repair-generated-sources");
    assert_eq!(
        std::fs::read_to_string(target.join("SKILL.md")).expect("installed skill file"),
        review.skill_md
    );
    let marker_path = target.join(".trace-commons-install.json");
    assert!(marker_path.is_file());
    assert_eq!(
        crate::skill_loop::sha256(&std::fs::read(&marker_path).expect("installed marker file")),
        marker_digest
    );

    let status = successful_result(
        dispatch_wire(
            &shared,
            18,
            "skill_install_status",
            json!({"source_submission_id": source_submission_id}),
        )
        .await,
        18,
    );
    assert_eq!(status["installed"], true);
    assert_eq!(status["skill"]["install_id"], install_id.to_string());
    assert_eq!(status["skill"]["marker_sha256"], installed_marker_digest);

    save_account_session(&shared, "owner-b.test");
    let owner_b = crate::daemon::public_run::current_account_binding(&shared, 19)
        .expect("replacement owner binding");
    assert_ne!(owner_a, owner_b);
    let wrong_owner_status = dispatch_wire(
        &shared,
        19,
        "skill_install_status",
        json!({"source_submission_id": source_submission_id}),
    )
    .await;
    assert_error(
        wrong_owner_status,
        19,
        ERR_UNAVAILABLE,
        "skill-install-not-owned",
    );
    {
        let state = shared.skill_loop.lock().expect("skill-loop state");
        assert_eq!(state.owner_scope.as_deref(), Some(owner_b.as_str()));
        assert!(state.workflows.is_empty());
        assert!(state.install_receipts.is_empty());
    }

    save_account_session(&shared, "owner-a.test");
    let recovered = successful_result(
        dispatch_wire(
            &shared,
            20,
            "skill_install_status",
            json!({"source_submission_id": source_submission_id}),
        )
        .await,
        20,
    );
    assert_eq!(recovered["installed"], true);
    assert_eq!(recovered["skill"]["install_id"], install_id.to_string());

    let rollback = successful_result(
        dispatch_wire(
            &shared,
            21,
            "skill_install_rollback",
            json!({
                "install_id": install_id,
                "source_submission_id": source_submission_id,
            }),
        )
        .await,
        21,
    );
    assert_eq!(
        rollback,
        json!({"removed": true, "retained_directory": true})
    );
    assert!(!target.exists());

    let absent = successful_result(
        dispatch_wire(
            &shared,
            22,
            "skill_install_status",
            json!({"source_submission_id": source_submission_id}),
        )
        .await,
        22,
    );
    assert_eq!(absent, json!({"installed": false, "skill": null}));
    assert_eq!(evaluation_calls.load(Ordering::SeqCst), 1);
}
