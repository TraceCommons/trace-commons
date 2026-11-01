// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The model-training grant routes against PostgreSQL: a trace whose
//! scoring award was withheld as `policy_mismatch` takes the use and earns
//! the award once, without being scored again.

use super::*;
use trace_commons_protocol::model_training_consent::{
    MODEL_TRAINING_NOT_PERMITTED_LABEL, ModelTrainingAwardState, ModelTrainingConsentGrantRequest,
    ModelTrainingConsentOutcome,
};

/// Gives `token` a claim-style ceiling, as an issuer-signed upload claim
/// carries one.
fn scope_token(state: &mut Arc<AppState>, token: &str, model_training: bool) {
    let tokens = Arc::make_mut(&mut Arc::make_mut(state).tokens);
    let auth = tokens.get_mut(token).expect("test token exists");
    auth.allowed_consent_scopes = BTreeSet::from([ConsentScope::DebuggingEvaluation]);
    auth.allowed_uses = BTreeSet::from([TraceAllowedUse::Debugging, TraceAllowedUse::Evaluation]);
    if model_training {
        auth.allowed_consent_scopes
            .insert(ConsentScope::ModelTraining);
        auth.allowed_uses.insert(TraceAllowedUse::ModelTraining);
    }
}

/// Moves the fixture submission to `token`'s principal: the gate-worker
/// fixture seeds it under the worker's.
async fn give_submission_to(
    backend: &PgBackend,
    tenant_id: &str,
    submission_id: Uuid,
    token: &str,
) {
    let mut client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("connection");
    let tx = client.transaction().await.expect("transaction");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .expect("tenant context");
    let principal = static_token_principal_refs(token).0;
    tx.execute(
        "UPDATE trace_submissions SET auth_principal_ref = $3
          WHERE tenant_id = $1 AND submission_id = $2",
        &[&tenant_id, &submission_id, &principal],
    )
    .await
    .expect("owner moves");
    tx.commit().await.expect("commit");
}

async fn stored_uses(backend: &PgBackend, tenant_id: &str, submission_id: Uuid) -> Vec<String> {
    let record = backend
        .get_trace_submission(tenant_id, submission_id)
        .await
        .expect("submission reads")
        .expect("submission exists");
    record.allowed_uses
}

async fn withheld_fixture(
    backend: Arc<PgBackend>,
    temp: &Path,
    artifact_temp: &Path,
) -> (Arc<AppState>, Uuid) {
    let (mut state, submission_id) = novelty_credit_test_fixture(
        backend.clone(),
        temp,
        artifact_temp,
        "tenant-a",
        vec!["evaluation".to_string()],
    )
    .await;
    let Json(scored) = gate_evaluate_worker_handler(
        State(state.clone()),
        auth_headers("vector-worker-token-a"),
        Json(TraceGateEvaluateWorkerRequest { submission_id }),
    )
    .await
    .expect("gate evaluation succeeds");
    assert!(scored.perplexity_passed && scored.novelty_passed);
    assert_eq!(
        scored.credit_withheld_reason.as_deref(),
        Some("policy_mismatch")
    );
    give_submission_to(backend.as_ref(), "tenant-a", submission_id, "token-a").await;
    scope_token(&mut state, "token-a", true);
    (state, submission_id)
}

#[tokio::test]
async fn a_grant_reruns_the_withheld_award_once_without_rescoring() {
    let Some(backend) = postgres_backend_for_ingest_test().await else {
        return;
    };
    cleanup_pg_trace_tenant(backend.as_ref(), "tenant-a").await;
    let temp = tempfile::tempdir().expect("temp dir");
    let artifact_temp = tempfile::tempdir().expect("artifact temp dir");
    let (state, submission_id) =
        withheld_fixture(backend.clone(), temp.path(), artifact_temp.path()).await;
    let decisions_before = backend
        .list_latest_gate_credit_decisions("tenant-a", &[submission_id])
        .await
        .expect("decisions read");
    assert_eq!(decisions_before.len(), 1);
    assert!(read_novelty_credit_events_for_test(temp.path(), "tenant-a").is_empty());

    let Json(eligible) =
        model_training_consent::eligibility_handler(State(state.clone()), auth_headers("token-a"))
            .await
            .expect("eligibility reads");
    assert_eq!(eligible.eligible_count, 1);
    assert_eq!(eligible.submission_ids, vec![submission_id]);

    let Json(granted) = model_training_consent::grant_handler(
        State(state.clone()),
        auth_headers("token-a"),
        Json(ModelTrainingConsentGrantRequest::AllEligible),
    )
    .await
    .expect("grant succeeds");
    assert_eq!(granted.results.len(), 1);
    let item = &granted.results[0];
    assert_eq!(item.submission_id, submission_id);
    assert_eq!(item.outcome, ModelTrainingConsentOutcome::Granted);
    assert!(item.granted_at.is_some());
    assert_eq!(item.award, ModelTrainingAwardState::Credited);
    assert_eq!(granted.remaining_eligible, 0);

    // Add-only: what the trace carried stays, plus model_training.
    assert_eq!(
        stored_uses(backend.as_ref(), "tenant-a", submission_id).await,
        vec!["evaluation".to_string(), "model_training".to_string()]
    );
    // One award, from the stored decision: no new decision row, and the
    // stored one no longer says withheld.
    assert_eq!(
        read_novelty_credit_events_for_test(temp.path(), "tenant-a").len(),
        1
    );
    let decisions_after = backend
        .list_latest_gate_credit_decisions("tenant-a", &[submission_id])
        .await
        .expect("decisions read");
    assert_eq!(decisions_after.len(), 1);
    assert_eq!(decisions_after[0].credit_withheld_reason, None);

    // Never twice: a repeat answers already granted, keeps the first date,
    // and writes no second award.
    let Json(repeat) = model_training_consent::grant_handler(
        State(state.clone()),
        auth_headers("token-a"),
        Json(ModelTrainingConsentGrantRequest::SubmissionIds {
            submission_ids: vec![submission_id],
        }),
    )
    .await
    .expect("repeat grant succeeds");
    assert_eq!(
        repeat.results[0].outcome,
        ModelTrainingConsentOutcome::AlreadyGranted
    );
    assert_eq!(repeat.results[0].granted_at, item.granted_at);
    assert_eq!(
        read_novelty_credit_events_for_test(temp.path(), "tenant-a").len(),
        1
    );

    // The grant is audited, hash-only: the kind and the trace, no consent
    // text and no contributor identity beyond the principal ref.
    let audit = read_all_audit_events(temp.path(), "tenant-a").expect("audit reads");
    let grants = audit
        .iter()
        .filter(|event| event.kind == model_training_consent::MODEL_TRAINING_CONSENT_AUDIT_KIND)
        .collect::<Vec<_>>();
    assert_eq!(grants.len(), 1, "one audit event per grant, none on repeat");
    assert_eq!(grants[0].submission_id, submission_id);
    let reason = grants[0].reason.as_deref().unwrap_or_default();
    assert!(reason.contains("purpose_hash=sha256:"), "{reason}");
    assert!(!reason.contains("model_training;"), "{reason}");

    let Json(after) =
        model_training_consent::eligibility_handler(State(state.clone()), auth_headers("token-a"))
            .await
            .expect("eligibility reads");
    assert_eq!(after.eligible_count, 0);

    cleanup_pg_trace_tenant(backend.as_ref(), "tenant-a").await;
}

#[tokio::test]
async fn a_grant_outside_the_ceiling_is_refused_and_changes_nothing() {
    let Some(backend) = postgres_backend_for_ingest_test().await else {
        return;
    };
    cleanup_pg_trace_tenant(backend.as_ref(), "tenant-a").await;
    let temp = tempfile::tempdir().expect("temp dir");
    let artifact_temp = tempfile::tempdir().expect("artifact temp dir");
    let (mut state, submission_id) =
        withheld_fixture(backend.clone(), temp.path(), artifact_temp.path()).await;
    scope_token(&mut state, "token-a", false);

    let refused = model_training_consent::grant_handler(
        State(state.clone()),
        auth_headers("token-a"),
        Json(ModelTrainingConsentGrantRequest::AllEligible),
    )
    .await
    .expect_err("outside the ceiling");
    assert_eq!(refused.0, StatusCode::FORBIDDEN);
    assert_eq!(refused.1.0.error, MODEL_TRAINING_NOT_PERMITTED_LABEL);
    let listed =
        model_training_consent::eligibility_handler(State(state.clone()), auth_headers("token-a"))
            .await
            .expect_err("the list is refused too");
    assert_eq!(listed.0, StatusCode::FORBIDDEN);
    assert_eq!(
        stored_uses(backend.as_ref(), "tenant-a", submission_id).await,
        vec!["evaluation".to_string()]
    );
    assert!(read_novelty_credit_events_for_test(temp.path(), "tenant-a").is_empty());

    cleanup_pg_trace_tenant(backend.as_ref(), "tenant-a").await;
}

#[tokio::test]
async fn another_contributor_cannot_grant_on_my_trace() {
    let Some(backend) = postgres_backend_for_ingest_test().await else {
        return;
    };
    cleanup_pg_trace_tenant(backend.as_ref(), "tenant-a").await;
    let temp = tempfile::tempdir().expect("temp dir");
    let artifact_temp = tempfile::tempdir().expect("artifact temp dir");
    let (mut state, submission_id) =
        withheld_fixture(backend.clone(), temp.path(), artifact_temp.path()).await;
    scope_token(&mut state, "token-a-2", true);

    let Json(eligible) = model_training_consent::eligibility_handler(
        State(state.clone()),
        auth_headers("token-a-2"),
    )
    .await
    .expect("eligibility reads");
    assert_eq!(eligible.eligible_count, 0);
    let Json(answer) = model_training_consent::grant_handler(
        State(state.clone()),
        auth_headers("token-a-2"),
        Json(ModelTrainingConsentGrantRequest::SubmissionIds {
            submission_ids: vec![submission_id],
        }),
    )
    .await
    .expect("the request is answered");
    assert_eq!(
        answer.results[0].outcome,
        ModelTrainingConsentOutcome::NotEligible
    );
    assert_eq!(answer.results[0].award, ModelTrainingAwardState::None);
    assert_eq!(
        stored_uses(backend.as_ref(), "tenant-a", submission_id).await,
        vec!["evaluation".to_string()]
    );
    assert!(read_novelty_credit_events_for_test(temp.path(), "tenant-a").is_empty());

    cleanup_pg_trace_tenant(backend.as_ref(), "tenant-a").await;
}

#[tokio::test]
async fn an_unscored_trace_is_left_pending_for_scoring() {
    let Some(backend) = postgres_backend_for_ingest_test().await else {
        return;
    };
    cleanup_pg_trace_tenant(backend.as_ref(), "tenant-a").await;
    let temp = tempfile::tempdir().expect("temp dir");
    let artifact_temp = tempfile::tempdir().expect("artifact temp dir");
    let (mut state, submission_id) = novelty_credit_test_fixture(
        backend.clone(),
        temp.path(),
        artifact_temp.path(),
        "tenant-a",
        vec!["evaluation".to_string()],
    )
    .await;
    give_submission_to(backend.as_ref(), "tenant-a", submission_id, "token-a").await;
    scope_token(&mut state, "token-a", true);

    let Json(granted) = model_training_consent::grant_handler(
        State(state.clone()),
        auth_headers("token-a"),
        Json(ModelTrainingConsentGrantRequest::AllEligible),
    )
    .await
    .expect("grant succeeds");
    assert_eq!(
        granted.results[0].outcome,
        ModelTrainingConsentOutcome::Granted
    );
    assert_eq!(granted.results[0].award, ModelTrainingAwardState::Pending);

    // Scoring now reads the stored use and pays the award first time.
    let Json(scored) = gate_evaluate_worker_handler(
        State(state.clone()),
        auth_headers("vector-worker-token-a"),
        Json(TraceGateEvaluateWorkerRequest { submission_id }),
    )
    .await
    .expect("gate evaluation succeeds");
    assert_eq!(scored.credit_withheld_reason, None);
    assert!(scored.credit_emitted);

    cleanup_pg_trace_tenant(backend.as_ref(), "tenant-a").await;
}
