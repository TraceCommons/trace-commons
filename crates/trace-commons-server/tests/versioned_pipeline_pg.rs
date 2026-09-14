use std::sync::Arc;

use chrono::{Duration, Utc};
use secrecy::SecretString;
use trace_commons_gate_api::pipeline::{Phase, ReviewDecision, ReviewInput};
use trace_commons_protocol::trace_contribution::{
    DeterministicTraceRedactor, RawTraceCaptureTurn, RawTraceContribution,
    RecordedTraceContributionOptions, TraceRedactor,
};
use trace_commons_server::config::DatabaseConfig;
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::secrets::SecretsCrypto;
use trace_commons_server::trace_artifact_store::{
    EncryptedTraceArtifact, EncryptedTraceArtifactReceipt, LocalEncryptedTraceArtifactStore,
    TraceArtifactKind, TraceArtifactStore,
};
use trace_commons_server::trace_corpus_storage::{TraceCorpusStatus, TraceCorpusStore};
use trace_commons_server::versioned_pipeline::{
    MinimalPolicyBundle, PgPipelineStore, PipelineCrashPoint, PipelineReceiptResult,
    PipelineRunState, PipelineService, StoredPhaseResult,
};
use uuid::Uuid;

static MIGRATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn backend() -> Option<Arc<PgBackend>> {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;
    let backend = match PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 8)).await {
        Ok(backend) => Arc::new(backend),
        Err(error) => {
            eprintln!("skipping: database unavailable ({error})");
            return None;
        }
    };
    let _migration_guard = MIGRATION_LOCK.lock().await;
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
    let artifact_store = artifact_store(&root);
    let service = PipelineService::new(backend, artifact_store, fail_phase).unwrap();
    (root, service)
}

fn artifact_store(root: &tempfile::TempDir) -> Arc<dyn TraceArtifactStore> {
    Arc::new(LocalEncryptedTraceArtifactStore::new(
        root.path(),
        SecretsCrypto::new(SecretString::from(
            "phase-2-test-master-key-material-32-bytes".to_string(),
        ))
        .unwrap(),
    ))
}

struct BlockingArtifactStore {
    inner: Arc<dyn TraceArtifactStore>,
    entered: Arc<std::sync::Barrier>,
    release: Arc<std::sync::Barrier>,
}

impl TraceArtifactStore for BlockingArtifactStore {
    fn put_serialized_json(
        &self,
        tenant_storage_ref: &str,
        artifact_kind: TraceArtifactKind,
        object_id: &str,
        serialized_json: &[u8],
    ) -> anyhow::Result<EncryptedTraceArtifactReceipt> {
        self.entered.wait();
        self.release.wait();
        self.inner.put_serialized_json(
            tenant_storage_ref,
            artifact_kind,
            object_id,
            serialized_json,
        )
    }

    fn read_artifact(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<EncryptedTraceArtifact> {
        self.inner
            .read_artifact(expected_tenant_storage_ref, receipt)
    }

    fn read_json(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<serde_json::Value> {
        self.inner.read_json(expected_tenant_storage_ref, receipt)
    }

    fn read_json_by_object_key(
        &self,
        expected_tenant_storage_ref: &str,
        expected_artifact_kind: TraceArtifactKind,
        object_key: &str,
        expected_ciphertext_sha256: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.inner.read_json_by_object_key(
            expected_tenant_storage_ref,
            expected_artifact_kind,
            object_key,
            expected_ciphertext_sha256,
        )
    }

    fn delete_artifact(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<bool> {
        self.inner
            .delete_artifact(expected_tenant_storage_ref, receipt)
    }
}

async fn expire_leases(backend: &PgBackend, tenant_id: &str) {
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .unwrap();
    tx.execute(
        "UPDATE pipeline_runs
         SET lease_expires_at = NOW() - INTERVAL '1 second'
         WHERE tenant_id = $1 AND state = 'leased'",
        &[&tenant_id],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

async fn finish_run(service: &PipelineService, tenant_id: &str, run_id: Uuid) {
    for _ in 0..6 {
        let inspection = service.inspect(tenant_id, run_id).await.unwrap().unwrap();
        if inspection.run.state == PipelineRunState::Complete {
            return;
        }
        service.process_one(tenant_id, None).await.unwrap();
    }
    panic!("pipeline run did not complete");
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
    let PipelineReceiptResult::Created(overlapping) = service
        .submit(&tenant_b, "principal_sha256:test", "same-key", &bytes)
        .await
        .unwrap()
    else {
        panic!("the other tenant must get an independent run");
    };
    assert_eq!(created.submission_id, overlapping.submission_id);
    assert_ne!(created.run_id, overlapping.run_id);

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
            .query_one(
                "SELECT COUNT(*) FROM phase_outcomes WHERE run_id = $1",
                &[&created.run_id],
            )
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
async fn concurrent_receipts_and_workers_commit_one_logical_result() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-concurrent-{}", Uuid::new_v4());
    let bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
    let (_root, service) = service(backend, None);
    let service = Arc::new(service);
    let (left, right) = tokio::join!(
        service.submit(&tenant, "principal_sha256:test", "concurrent-key", &bytes),
        service.submit(&tenant, "principal_sha256:test", "concurrent-key", &bytes)
    );
    let receipts = [left.unwrap(), right.unwrap()];
    assert_eq!(
        receipts
            .iter()
            .filter(|receipt| matches!(receipt, PipelineReceiptResult::Created(_)))
            .count(),
        1
    );
    let run_ids = receipts
        .iter()
        .map(|receipt| match receipt {
            PipelineReceiptResult::Created(run) | PipelineReceiptResult::Replayed(run) => {
                run.run_id
            }
            PipelineReceiptResult::ContentConflict => panic!("equal receipts cannot conflict"),
        })
        .collect::<Vec<_>>();
    assert_eq!(run_ids[0], run_ids[1]);

    let workers = (0..4)
        .map(|_| {
            let service = service.clone();
            let tenant = tenant.clone();
            tokio::spawn(async move { service.process_one(&tenant, None).await })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.await.unwrap().unwrap();
    }
    finish_run(&service, &tenant, run_ids[0]).await;
    let complete = service.inspect(&tenant, run_ids[0]).await.unwrap().unwrap();
    assert_eq!(complete.outcomes.len(), 4);
}

#[tokio::test]
async fn stale_lease_cannot_commit_and_retry_exhaustion_is_visible() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-lease-{}", Uuid::new_v4());
    let bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
    let (_root, service) = service(backend.clone(), None);
    let PipelineReceiptResult::Created(created) = service
        .submit(&tenant, "principal_sha256:test", "lease-key", &bytes)
        .await
        .unwrap()
    else {
        panic!("first receipt must create a run");
    };
    let store = PgPipelineStore::new(backend.clone());
    let stale = store
        .claim_next_with_lease(&tenant, Duration::milliseconds(10))
        .await
        .unwrap()
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    let current = store
        .claim_next_with_lease(&tenant, Duration::seconds(30))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(stale.lease_token, current.lease_token);
    let bundle = MinimalPolicyBundle::build().unwrap();
    let review = bundle
        .review
        .execute(&ReviewInput {
            run_id: current.run_id,
            trace_id: current.trace_id,
            source_content_hash: current.request_content_hash.clone(),
            source_artifact: bytes,
        })
        .await
        .unwrap();
    let ReviewDecision::Approved {
        registry_revision_id,
    } = review.decision
    else {
        panic!("minimal review must approve");
    };
    let outcome = StoredPhaseResult::from_result(Phase::Review, &review).unwrap();
    store
        .commit_phase(
            &current,
            outcome.clone(),
            Some(Phase::Score),
            Some(registry_revision_id),
        )
        .await
        .unwrap();
    assert!(
        store
            .commit_phase(
                &stale,
                outcome,
                Some(Phase::Score),
                Some(registry_revision_id)
            )
            .await
            .is_err()
    );
    let inspection = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        inspection
            .outcomes
            .iter()
            .filter(|outcome| outcome.phase == Phase::Review)
            .count(),
        1
    );

    let retry_tenant = format!("pipeline-retry-{}", Uuid::new_v4());
    let retry_bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
    let PipelineReceiptResult::Created(retry_run) = service
        .submit(
            &retry_tenant,
            "principal_sha256:test",
            "retry-key",
            &retry_bytes,
        )
        .await
        .unwrap()
    else {
        panic!("first receipt must create a run");
    };
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&retry_tenant],
    )
    .await
    .unwrap();
    tx.execute(
        "UPDATE pipeline_runs SET max_attempts = 2
         WHERE tenant_id = $1 AND run_id = $2",
        &[&retry_tenant, &retry_run.run_id],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let first = store.claim_next(&retry_tenant).await.unwrap().unwrap();
    let retry = store
        .mark_retry(&first, "dependency_unavailable")
        .await
        .unwrap();
    assert_eq!(retry.state, PipelineRunState::Retry);
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&retry_tenant],
    )
    .await
    .unwrap();
    tx.execute(
        "UPDATE pipeline_runs SET next_attempt_at = NOW()
         WHERE tenant_id = $1 AND run_id = $2",
        &[&retry_tenant, &retry_run.run_id],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let second = store.claim_next(&retry_tenant).await.unwrap().unwrap();
    let failed = store
        .mark_retry(&second, "dependency_unavailable")
        .await
        .unwrap();
    assert_eq!(failed.state, PipelineRunState::Failed);
    assert_eq!(
        failed.last_error_label.as_deref(),
        Some("attempts_exhausted")
    );
    assert_eq!(
        service
            .inspect(&retry_tenant, retry_run.run_id)
            .await
            .unwrap()
            .unwrap()
            .outcomes
            .len(),
        1
    );
}

#[tokio::test]
async fn bound_bundle_survives_activation_and_rollback() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-bundle-{}", Uuid::new_v4());
    let (root, service) = service(backend.clone(), None);
    let bundle_a = service.bundle_id().to_string();
    let first_bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
    let PipelineReceiptResult::Created(first) = service
        .submit(
            &tenant,
            "principal_sha256:test",
            "bundle-a-key",
            &first_bytes,
        )
        .await
        .unwrap()
    else {
        panic!("first receipt must create a run");
    };
    let bundle_b = MinimalPolicyBundle::build_variant("minimal-local-v2").unwrap();
    service
        .register_bundle(&tenant, &bundle_b.package)
        .await
        .unwrap();
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let runnable_count: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM pipeline_bundle_policy_status
             WHERE tenant_id = $1 AND bundle_id = $2 AND runnable",
            &[&tenant, &bundle_b.package.bundle_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(runnable_count, 4);
    assert!(
        tx.execute(
            "UPDATE pipeline_bundle_packages SET package = '{}'::jsonb
             WHERE tenant_id = $1 AND bundle_id = $2",
            &[&tenant, &bundle_b.package.bundle_id],
        )
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
    service
        .activate_bundle(&tenant, &bundle_b.package.bundle_id)
        .await
        .unwrap();
    let second_bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
    let PipelineReceiptResult::Created(second) = service
        .submit(
            &tenant,
            "principal_sha256:test",
            "bundle-b-key",
            &second_bytes,
        )
        .await
        .unwrap()
    else {
        panic!("second receipt must create a run");
    };
    assert_eq!(first.bundle_id, bundle_a);
    assert_eq!(second.bundle_id, bundle_b.package.bundle_id);
    let restarted = PipelineService::new(backend.clone(), artifact_store(&root), None).unwrap();
    finish_run(&restarted, &tenant, first.run_id).await;
    finish_run(&restarted, &tenant, second.run_id).await;
    let first_outcomes = restarted
        .inspect(&tenant, first.run_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        first_outcomes
            .outcomes
            .iter()
            .all(|outcome| outcome.bundle_id == bundle_a)
    );
    let first_outcome_bytes = serde_json::to_vec(&first_outcomes.outcomes).unwrap();
    let second_outcomes = restarted
        .inspect(&tenant, second.run_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        second_outcomes
            .outcomes
            .iter()
            .all(|outcome| outcome.bundle_id == bundle_b.package.bundle_id)
    );
    restarted.activate_bundle(&tenant, &bundle_a).await.unwrap();
    let third_bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
    let PipelineReceiptResult::Created(third) = restarted
        .submit(
            &tenant,
            "principal_sha256:test",
            "bundle-a-rollback-key",
            &third_bytes,
        )
        .await
        .unwrap()
    else {
        panic!("rollback receipt must create a run");
    };
    assert_eq!(third.bundle_id, bundle_a);
    assert_eq!(
        serde_json::to_vec(
            &restarted
                .inspect(&tenant, first.run_id)
                .await
                .unwrap()
                .unwrap()
                .outcomes
        )
        .unwrap(),
        first_outcome_bytes
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activation_after_resolution_does_not_rebind_the_receipt() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-resolution-race-{}", Uuid::new_v4());
    let root = tempfile::tempdir().unwrap();
    let inner = artifact_store(&root);
    let entered = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    let blocking_store: Arc<dyn TraceArtifactStore> = Arc::new(BlockingArtifactStore {
        inner,
        entered: entered.clone(),
        release: release.clone(),
    });
    let service = Arc::new(PipelineService::new(backend.clone(), blocking_store, None).unwrap());
    let bundle_a = MinimalPolicyBundle::build().unwrap();
    let bundle_b = MinimalPolicyBundle::build_variant("activation-race-v2").unwrap();
    service
        .register_bundle(&tenant, &bundle_a.package)
        .await
        .unwrap();
    service
        .register_bundle(&tenant, &bundle_b.package)
        .await
        .unwrap();
    service
        .activate_bundle(&tenant, &bundle_a.package.bundle_id)
        .await
        .unwrap();

    let request = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
    let submit_service = service.clone();
    let submit_tenant = tenant.clone();
    let submit_request = request.clone();
    let submit = tokio::spawn(async move {
        submit_service
            .submit(
                &submit_tenant,
                "principal_sha256:test",
                "resolution-race-key",
                &submit_request,
            )
            .await
    });
    tokio::task::spawn_blocking(move || entered.wait())
        .await
        .unwrap();
    service
        .activate_bundle(&tenant, &bundle_b.package.bundle_id)
        .await
        .unwrap();
    tokio::task::spawn_blocking(move || release.wait())
        .await
        .unwrap();
    let PipelineReceiptResult::Created(first) = submit.await.unwrap().unwrap() else {
        panic!("first receipt must create a run");
    };
    assert_eq!(first.bundle_id, bundle_a.package.bundle_id);

    let restarted = PipelineService::new(backend, artifact_store(&root), None).unwrap();
    let later_request = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
    let PipelineReceiptResult::Created(later) = restarted
        .submit(
            &tenant,
            "principal_sha256:test",
            "after-resolution-race-key",
            &later_request,
        )
        .await
        .unwrap()
    else {
        panic!("later receipt must create a run");
    };
    assert_eq!(later.bundle_id, bundle_b.package.bundle_id);
}

#[tokio::test]
async fn crash_boundaries_one_through_five_converge_after_restart() {
    let Some(backend) = backend().await else {
        return;
    };
    for crash_point in [
        PipelineCrashPoint::AfterArtifactStorage,
        PipelineCrashPoint::AfterAdmissionWork,
        PipelineCrashPoint::AfterReviewWork,
        PipelineCrashPoint::AfterReviewCommit,
        PipelineCrashPoint::AfterScoreWork,
    ] {
        let tenant = format!("pipeline-crash-{:?}-{}", crash_point, Uuid::new_v4());
        let bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
        let root = tempfile::tempdir().unwrap();
        let artifacts = artifact_store(&root);
        let crashing = PipelineService::new_with_crash_point(
            backend.clone(),
            artifacts.clone(),
            None,
            Some(crash_point),
        )
        .unwrap();
        let key = "crash-recovery-key";
        let run_id = if matches!(
            crash_point,
            PipelineCrashPoint::AfterArtifactStorage | PipelineCrashPoint::AfterAdmissionWork
        ) {
            assert!(
                crashing
                    .submit(&tenant, "principal_sha256:test", key, &bytes)
                    .await
                    .is_err()
            );
            let mut client = backend.trace_pool_for_test().get().await.unwrap();
            let tx = client.transaction().await.unwrap();
            tx.execute(
                "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
                &[&tenant],
            )
            .await
            .unwrap();
            tx.execute(
                "UPDATE pipeline_receipt_artifacts
                 SET cleanup_after = NOW() - INTERVAL '1 second'
                 WHERE tenant_id = $1",
                &[&tenant],
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            assert_eq!(
                crashing
                    .list_cleanup_orphans(&tenant, Utc::now())
                    .await
                    .unwrap()
                    .len(),
                1
            );
            let mut changed = bytes.clone();
            changed.push(b' ');
            assert!(matches!(
                crashing
                    .submit(&tenant, "principal_sha256:test", key, &changed)
                    .await
                    .unwrap(),
                PipelineReceiptResult::ContentConflict
            ));
            let restarted = PipelineService::new(backend.clone(), artifacts.clone(), None).unwrap();
            let PipelineReceiptResult::Created(created) = restarted
                .submit(&tenant, "principal_sha256:test", key, &bytes)
                .await
                .unwrap()
            else {
                panic!("restart must commit the receipt");
            };
            finish_run(&restarted, &tenant, created.run_id).await;
            assert!(
                restarted
                    .list_cleanup_orphans(&tenant, Utc::now())
                    .await
                    .unwrap()
                    .is_empty()
            );
            created.run_id
        } else {
            let PipelineReceiptResult::Created(created) = crashing
                .submit(&tenant, "principal_sha256:test", key, &bytes)
                .await
                .unwrap()
            else {
                panic!("receipt must commit before worker crash");
            };
            if crash_point == PipelineCrashPoint::AfterScoreWork {
                crashing.process_one(&tenant, None).await.unwrap();
            }
            assert!(crashing.process_one(&tenant, None).await.is_err());
            expire_leases(&backend, &tenant).await;
            let restarted = PipelineService::new(backend.clone(), artifacts.clone(), None).unwrap();
            finish_run(&restarted, &tenant, created.run_id).await;
            created.run_id
        };
        let restarted = PipelineService::new(backend.clone(), artifacts, None).unwrap();
        let before = serde_json::to_vec(
            &restarted
                .inspect(&tenant, run_id)
                .await
                .unwrap()
                .unwrap()
                .outcomes,
        )
        .unwrap();
        let replayed = restarted
            .submit(&tenant, "principal_sha256:test", key, &bytes)
            .await
            .unwrap();
        assert!(matches!(replayed, PipelineReceiptResult::Replayed(_)));
        let after = serde_json::to_vec(
            &restarted
                .inspect(&tenant, run_id)
                .await
                .unwrap()
                .unwrap()
                .outcomes,
        )
        .unwrap();
        assert_eq!(before, after);
    }
}

#[tokio::test]
async fn claimer_privileges_and_outcome_reader_fail_closed() {
    let Some(backend) = backend().await else {
        return;
    };
    let client = backend.trace_pool_for_test().get().await.unwrap();
    let privileges = client
        .query_one(
            "SELECT
                has_function_privilege(
                    'pipeline_claimer',
                    'claim_pipeline_run(uuid,integer)',
                    'EXECUTE'
                ) AS can_claim,
                has_table_privilege(
                    'pipeline_claimer',
                    'pipeline_runs',
                    'SELECT'
                ) AS can_read_runs,
                has_table_privilege(
                    'pipeline_claimer',
                    'phase_outcomes',
                    'SELECT'
                ) AS can_read_outcomes",
            &[],
        )
        .await
        .unwrap();
    assert!(privileges.get::<_, bool>("can_claim"));
    assert!(!privileges.get::<_, bool>("can_read_runs"));
    assert!(!privileges.get::<_, bool>("can_read_outcomes"));

    let tenant = format!("pipeline-schema-{}", Uuid::new_v4());
    let bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
    let (_root, service) = service(backend.clone(), None);
    let PipelineReceiptResult::Created(created) = service
        .submit(&tenant, "principal_sha256:test", "schema-key", &bytes)
        .await
        .unwrap()
    else {
        panic!("first receipt must create a run");
    };
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.execute(
        "INSERT INTO phase_outcomes (
            tenant_id, outcome_id, run_id, trace_id, phase, bundle_id,
            outcome_schema_id, outcome_schema_version, decision, evidence, evaluation
         ) VALUES ($1,$2,$3,$4,'review',$5,'trace_commons.pipeline_outcome',999,
                   '{}'::jsonb,'{}'::jsonb,'{}'::jsonb)",
        &[
            &tenant,
            &Uuid::new_v4(),
            &created.run_id,
            &created.trace_id,
            &created.bundle_id,
        ],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert!(service.inspect(&tenant, created.run_id).await.is_err());
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
