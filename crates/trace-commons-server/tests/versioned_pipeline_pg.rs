use std::sync::Arc;

use chrono::Utc;
use secrecy::SecretString;
use trace_commons_gate_api::pipeline::Phase;
use trace_commons_protocol::trace_contribution::{
    DeterministicTraceRedactor, RawTraceCaptureTurn, RawTraceContribution,
    RecordedTraceContributionOptions, TraceRedactor,
};
use trace_commons_server::config::DatabaseConfig;
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::secrets::SecretsCrypto;
use trace_commons_server::trace_artifact_store::{
    LocalEncryptedTraceArtifactStore, TraceArtifactStore,
};
use trace_commons_server::trace_corpus_storage::{TraceCorpusStatus, TraceCorpusStore};
use trace_commons_server::versioned_pipeline::{
    PipelineReceiptResult, PipelineRunState, PipelineService,
};
use uuid::Uuid;

async fn backend() -> Option<Arc<PgBackend>> {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;
    let backend = match PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 4)).await {
        Ok(backend) => Arc::new(backend),
        Err(error) => {
            eprintln!("skipping: database unavailable ({error})");
            return None;
        }
    };
    if let Err(error) = backend.run_migrations().await {
        eprintln!("skipping: migrations failed ({error})");
        return None;
    }
    Some(backend)
}

async fn envelope_bytes(submission_id: Uuid, secret: &str) -> Vec<u8> {
    let now = Utc::now();
    let raw = RawTraceContribution::from_capture_turns(
        &[RawTraceCaptureTurn {
            user_input: format!("Inspect this fixture safely. Credential: {secret}"),
            response: Some("Done.".to_string()),
            tool_calls: Vec::new(),
            started_at: now,
            completed_at: Some(now + chrono::Duration::seconds(1)),
            state: Some("complete".to_string()),
        }],
        RecordedTraceContributionOptions {
            include_message_text: true,
            ..RecordedTraceContributionOptions::default()
        },
    );
    let mut envelope = DeterministicTraceRedactor::try_default()
        .unwrap()
        .redact_trace(raw)
        .await
        .unwrap();
    envelope.submission_id = submission_id;
    serde_json::to_vec(&envelope).unwrap()
}

fn service(
    backend: Arc<PgBackend>,
    fail_phase: Option<Phase>,
) -> (tempfile::TempDir, PipelineService) {
    let root = tempfile::tempdir().unwrap();
    let artifact_store: Arc<dyn TraceArtifactStore> =
        Arc::new(LocalEncryptedTraceArtifactStore::new(
            root.path(),
            SecretsCrypto::new(SecretString::from(
                "phase-1-test-master-key-material-32-bytes".to_string(),
            ))
            .unwrap(),
        ));
    let service = PipelineService::new(backend, artifact_store, fail_phase).unwrap();
    (root, service)
}

#[tokio::test]
async fn phase_one_pipeline_is_idempotent_tenant_scoped_and_complete() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant_a = format!("pipeline-a-{}", Uuid::new_v4());
    let tenant_b = format!("pipeline-b-{}", Uuid::new_v4());
    let secret = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890";
    let bytes = envelope_bytes(Uuid::new_v4(), secret).await;
    assert!(!String::from_utf8_lossy(&bytes).contains(secret));
    let (_root, service) = service(backend.clone(), None);

    let created = service
        .submit(&tenant_a, "principal_sha256:test", "same-key", &bytes)
        .await
        .unwrap();
    let PipelineReceiptResult::Created(created) = created else {
        panic!("first receipt must create a run");
    };
    let replayed = service
        .submit(&tenant_a, "principal_sha256:test", "same-key", &bytes)
        .await
        .unwrap();
    let PipelineReceiptResult::Replayed(replayed) = replayed else {
        panic!("exact replay must return the run");
    };
    assert_eq!(created.run_id, replayed.run_id);

    let mut changed = bytes.clone();
    changed.push(b' ');
    assert!(matches!(
        service
            .submit(&tenant_a, "principal_sha256:test", "same-key", &changed)
            .await
            .unwrap(),
        PipelineReceiptResult::ContentConflict
    ));
    assert!(
        service
            .inspect(&tenant_b, created.run_id)
            .await
            .unwrap()
            .is_none()
    );
    let client = backend.trace_pool_for_test().get().await.unwrap();
    let role = client
        .query_one(
            "SELECT r.rolbypassrls,
                    EXISTS (
                        SELECT 1
                        FROM pg_class c
                        WHERE c.relname IN ('pipeline_runs', 'phase_outcomes')
                          AND c.relowner = r.oid
                    ) AS owns_pipeline_tables
             FROM pg_roles r
             WHERE r.rolname = current_user",
            &[],
        )
        .await
        .unwrap();
    let bypasses_rls: bool = role.get("rolbypassrls");
    let owns_pipeline_tables: bool = role.get("owns_pipeline_tables");
    if !bypasses_rls && !owns_pipeline_tables {
        let mut client = backend.trace_pool_for_test().get().await.unwrap();
        let tx = client.transaction().await.unwrap();
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant_b],
        )
        .await
        .unwrap();
        let visible: i64 = tx
            .query_one("SELECT COUNT(*) FROM phase_outcomes", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(visible, 0, "RLS exposed another tenant's outcome");
        tx.commit().await.unwrap();
    }

    service
        .process_one(&tenant_a, Some(Phase::Review))
        .await
        .unwrap();
    let paused_review = service
        .inspect(&tenant_a, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(paused_review.run.state, PipelineRunState::Pending);
    assert_eq!(paused_review.run.next_phase, Some(Phase::Review));
    assert_eq!(paused_review.outcomes.len(), 1);

    service.process_one(&tenant_a, None).await.unwrap();
    service
        .process_one(&tenant_a, Some(Phase::Score))
        .await
        .unwrap();
    let paused_score = service
        .inspect(&tenant_a, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(paused_score.run.next_phase, Some(Phase::Score));
    assert_eq!(paused_score.outcomes.len(), 2);

    service.process_one(&tenant_a, None).await.unwrap();
    service.process_one(&tenant_a, None).await.unwrap();
    let complete = service
        .inspect(&tenant_a, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(complete.run.state, PipelineRunState::Complete);
    assert_eq!(complete.run.index_membership, "excluded");
    assert!(complete.run.approved_revision_id.is_some());
    assert_eq!(
        backend
            .get_trace_submission(&tenant_a, created.submission_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        TraceCorpusStatus::Accepted
    );
    assert_eq!(
        complete
            .outcomes
            .iter()
            .map(|outcome| outcome.phase)
            .collect::<Vec<_>>(),
        vec![Phase::Admission, Phase::Review, Phase::Score, Phase::Settle]
    );
    let score = complete
        .outcomes
        .iter()
        .find(|outcome| outcome.phase == Phase::Score)
        .unwrap();
    assert_eq!(score.decision["credit_microcredits"], 0);
    let settle = complete
        .outcomes
        .iter()
        .find(|outcome| outcome.phase == Phase::Settle)
        .unwrap();
    assert_eq!(settle.decision["credit_microcredits_finalized"], 0);
    assert!(settle.decision["settlement_batch_ref_hash"].is_null());
    assert!(!serde_json::to_string(&complete).unwrap().contains(secret));

    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_a],
    )
    .await
    .unwrap();
    let error = tx
        .execute(
            "UPDATE phase_outcomes SET decision = '{}'::jsonb
             WHERE tenant_id = $1 AND run_id = $2",
            &[&tenant_a, &created.run_id],
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.as_db_error().map(|error| error.message()),
        Some("phase outcomes are immutable")
    );
}

#[tokio::test]
async fn policy_failure_records_an_error_without_a_decision() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-failure-{}", Uuid::new_v4());
    let bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
    let (_root, service) = service(backend, Some(Phase::Score));
    let PipelineReceiptResult::Created(created) = service
        .submit(&tenant, "principal_sha256:test", "failure-key", &bytes)
        .await
        .unwrap()
    else {
        panic!("first receipt must create a run");
    };
    service.process_one(&tenant, None).await.unwrap();
    service.process_one(&tenant, None).await.unwrap();
    let failed = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.run.state, PipelineRunState::Failed);
    assert_eq!(
        failed.run.last_error_label.as_deref(),
        Some("minimal_policy_failed")
    );
    assert_eq!(failed.outcomes.len(), 2);
    assert!(
        failed
            .outcomes
            .iter()
            .all(|outcome| outcome.phase != Phase::Score)
    );
}
