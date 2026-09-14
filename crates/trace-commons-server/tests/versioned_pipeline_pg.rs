use std::sync::Arc;

use base64::Engine;
use chrono::{Duration, Utc};
use ring::signature::{Ed25519KeyPair, KeyPair};
use secrecy::SecretString;
use trace_commons_gate_api::pipeline::{Phase, ReasonCode, ReviewDecision, ReviewRecommendation};
use trace_commons_gate_api::{IndexEntryKey, IndexWriteError, VectorIndexWriter};
use trace_commons_protocol::trace_contribution::{
    DeterministicTraceRedactor, RawTraceCaptureTurn, RawTraceContribution,
    RecordedTraceContributionOptions, ResidualPiiRisk, TraceRedactor,
};
use trace_commons_server::config::DatabaseConfig;
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::secrets::SecretsCrypto;
use trace_commons_server::trace_artifact_store::{
    EncryptedTraceArtifact, EncryptedTraceArtifactReceipt, LocalEncryptedTraceArtifactStore,
    TraceArtifactKind, TraceArtifactStore,
};
use trace_commons_server::trace_corpus_storage::{
    TraceCorpusStatus, TraceCorpusStore, TraceCreditSettlementNearStatus,
    TraceCreditSettlementState,
};
use trace_commons_server::versioned_pipeline::{
    MinimalPolicyBundle, PIPELINE_FIXED_POSITIVE_MICROCREDITS, PIPELINE_SCORE_DEPENDENCY_LABEL,
    PgPipelineStore, PipelineCrashPoint, PipelineReceiptResult, PipelineRunState, PipelineService,
    StoredPhaseResult,
};
use trace_commons_server::versioned_pipeline_compat::{
    CompatibilityBundleConfig, CompatibilityScoreRuntime,
};
use trace_commons_server::versioned_pipeline_credit::RecordingNearAdapter;
use trace_commons_server::versioned_pipeline_index::{
    IndexFault, IsolatedPipelineIndex, PIPELINE_EMBEDDER_MODEL_ID, PIPELINE_INDEX_ID,
    PIPELINE_PROJECTION_ID,
};
use trace_commons_server::versioned_pipeline_product::{
    PipelineCreditStatus, PipelineProcessingStatus, PipelineProductStore, sha256_prefixed,
};
use trace_commons_server::versioned_pipeline_qualification::{
    BundlePackageSignature, BundlePackageTrustStore, BundleQualificationMetadata, DrillEvidence,
    DrillStatus, PACKAGE_QUALIFICATION_MISSING_LABEL, PACKAGE_SIGNATURE_ALGORITHM,
    PipelineQualificationStore, ProductionDependencyProfile, REQUIRED_PHASE_SEVEN_DRILLS,
    SignedBundlePackage, TrustedBundleKey, evaluate_promotion,
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
    envelope_bytes_with_risk(submission_id, secret, ResidualPiiRisk::Low).await
}

async fn envelope_bytes_with_risk(
    submission_id: Uuid,
    secret: &str,
    privacy_risk: ResidualPiiRisk,
) -> Vec<u8> {
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
    envelope.privacy.residual_pii_risk = privacy_risk;
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
    for _ in 0..24 {
        let inspection = service.inspect(tenant_id, run_id).await.unwrap().unwrap();
        if inspection.run.state == PipelineRunState::Complete {
            service.process_payout(tenant_id, run_id).await.unwrap();
            return;
        }
        if inspection.run.state == PipelineRunState::Failed {
            panic!("pipeline run failed: {:?}", inspection.run.last_error_label);
        }
        if inspection.run.next_attempt_at > Utc::now() {
            let wait = (inspection.run.next_attempt_at - Utc::now())
                .to_std()
                .unwrap_or_else(|_| std::time::Duration::from_millis(5));
            tokio::time::sleep(wait + std::time::Duration::from_millis(10)).await;
        }
        service.process_run(tenant_id, run_id, None).await.unwrap();
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
    assert_eq!(complete.run.credit_write_state, "none");
    assert_eq!(complete.run.index_write_state, "none");
    assert_eq!(complete.run.payout_state, "none");
    assert!(complete.run.credit_event_id.is_none());
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
    let (_root, service) = service(backend.clone(), None);
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
    let outcome = StoredPhaseResult {
        phase: Phase::Review,
        decision: serde_json::to_value(ReviewDecision::Rejected {
            reason: ReasonCode::new("lease_test_rejection").unwrap(),
        })
        .unwrap(),
        evidence: serde_json::json!({
            "source_content_hash": current.request_content_hash,
            "result_content_hash": current.request_content_hash,
            "content_changed": false,
        }),
        evaluation: serde_json::json!({
            "rule_id": "lease_test_rejection_v1",
        }),
    };
    store
        .commit_phase(&current, outcome.clone(), None, None)
        .await
        .unwrap();
    assert!(
        store
            .commit_phase(&stale, outcome, None, None)
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

async fn activate_operations(
    service: &PipelineService,
    tenant: &str,
    score_microcredits: u64,
    include_index: bool,
) {
    let bundle = MinimalPolicyBundle::build_operations(score_microcredits, include_index).unwrap();
    service
        .register_bundle(tenant, &bundle.package)
        .await
        .unwrap();
    service
        .activate_bundle(tenant, &bundle.package.bundle_id)
        .await
        .unwrap();
}

#[tokio::test]
async fn four_index_and_credit_combinations_complete() {
    let Some(backend) = backend().await else {
        return;
    };
    let (_root, service) = service(backend.clone(), None);
    let principal = "principal_sha256:test";
    for (score, include, label) in [
        (0, false, "zero-exclude"),
        (0, true, "zero-include"),
        (
            PIPELINE_FIXED_POSITIVE_MICROCREDITS,
            false,
            "positive-exclude",
        ),
        (
            PIPELINE_FIXED_POSITIVE_MICROCREDITS,
            true,
            "positive-include",
        ),
    ] {
        let tenant = format!("pipeline-combo-{label}-{}", Uuid::new_v4());
        activate_operations(&service, &tenant, score, include).await;
        let bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
        let PipelineReceiptResult::Created(created) = service
            .submit(&tenant, principal, label, &bytes)
            .await
            .unwrap()
        else {
            panic!("{label} must create a run");
        };
        finish_run(&service, &tenant, created.run_id).await;
        let complete = service
            .inspect(&tenant, created.run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(complete.run.state, PipelineRunState::Complete);
        assert_eq!(
            complete.run.index_membership,
            if include { "included" } else { "excluded" }
        );
        assert_eq!(
            complete.run.index_write_state,
            if include { "complete" } else { "none" }
        );
        let score_outcome = complete
            .outcomes
            .iter()
            .find(|outcome| outcome.phase == Phase::Score)
            .unwrap();
        assert_eq!(score_outcome.decision["credit_microcredits"], score);
        let settle = complete
            .outcomes
            .iter()
            .find(|outcome| outcome.phase == Phase::Settle)
            .unwrap();
        if score == 0 {
            assert!(complete.run.credit_event_id.is_none());
            assert_eq!(complete.run.credit_write_state, "none");
            assert_eq!(settle.decision["credit_microcredits_finalized"], 0);
            assert!(settle.decision["settlement_batch_ref_hash"].is_null());
        } else {
            assert!(complete.run.credit_event_id.is_some());
            assert_eq!(complete.run.credit_write_state, "complete");
            assert_eq!(settle.decision["credit_microcredits_finalized"], score);
            assert!(settle.decision["settlement_batch_ref_hash"].is_string());
        }
        assert_eq!(
            service.index().contains_revision(
                &tenant,
                PIPELINE_INDEX_ID,
                complete.run.approved_revision_id.unwrap()
            ),
            include
        );
        let report = serde_json::to_string(&complete).unwrap();
        assert!(!report.contains(principal));
        let events = backend.list_trace_credit_events(&tenant).await.unwrap();
        if score == 0 {
            assert!(events.is_empty());
        } else {
            assert_eq!(events.len(), 1);
            assert_eq!(
                events[0].settlement_state,
                TraceCreditSettlementState::Final
            );
        }
    }
}

#[tokio::test]
async fn compatible_runs_finalize_in_one_batch() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-batch-{}", Uuid::new_v4());
    let (_root, service) = service(backend.clone(), None);
    activate_operations(
        &service,
        &tenant,
        PIPELINE_FIXED_POSITIVE_MICROCREDITS,
        false,
    )
    .await;
    let mut run_ids = Vec::new();
    for index in 0..3 {
        let bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
        let PipelineReceiptResult::Created(created) = service
            .submit(
                &tenant,
                "principal_sha256:test",
                &format!("batch-key-{index}"),
                &bytes,
            )
            .await
            .unwrap()
        else {
            panic!("batch receipt must create a run");
        };
        run_ids.push(created.run_id);
        service
            .process_run(&tenant, created.run_id, None)
            .await
            .unwrap();
        service
            .process_run(&tenant, created.run_id, None)
            .await
            .unwrap();
    }
    for run_id in &run_ids {
        finish_run(&service, &tenant, *run_id).await;
    }
    let mut hashes = Vec::new();
    for run_id in &run_ids {
        let inspection = service.inspect(&tenant, *run_id).await.unwrap().unwrap();
        hashes.push(
            inspection
                .outcomes
                .iter()
                .find(|outcome| outcome.phase == Phase::Settle)
                .and_then(|outcome| {
                    outcome.decision["settlement_batch_ref_hash"]
                        .as_str()
                        .map(str::to_string)
                }),
        );
    }
    assert!(hashes.iter().all(|hash| hash.is_some()));
    // The first Settle call batches every pending event. Later runs reuse that batch.
    assert_eq!(hashes[0], hashes[1]);
    assert_eq!(hashes[1], hashes[2]);
}

#[tokio::test]
async fn hold_and_index_failure_recover_independently() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-hold-{}", Uuid::new_v4());
    let root = tempfile::tempdir().unwrap();
    let artifacts = artifact_store(&root);
    let index = IsolatedPipelineIndex::new();
    let service = PipelineService::new_with_ops(
        backend.clone(),
        artifacts,
        None,
        None,
        index.clone(),
        std::sync::Arc::new(RecordingNearAdapter::new()),
    )
    .unwrap();
    activate_operations(
        &service,
        &tenant,
        PIPELINE_FIXED_POSITIVE_MICROCREDITS,
        true,
    )
    .await;
    let bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
    let principal = "principal_sha256:test";
    let PipelineReceiptResult::Created(created) = service
        .submit(&tenant, principal, "hold-key", &bytes)
        .await
        .unwrap()
    else {
        panic!("receipt must create a run");
    };
    let hold_id = service.place_credit_hold(&tenant, principal).await.unwrap();
    service
        .process_run(&tenant, created.run_id, None)
        .await
        .unwrap();
    service
        .process_run(&tenant, created.run_id, None)
        .await
        .unwrap();
    service
        .process_run(&tenant, created.run_id, None)
        .await
        .unwrap();
    let paused = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(paused.run.state, PipelineRunState::Complete);
    assert!(paused.run.credit_event_id.is_some());
    assert_eq!(paused.run.credit_write_state, "held");
    assert_eq!(paused.run.index_write_state, "complete");
    assert_eq!(index.entry_count(&tenant, PIPELINE_INDEX_ID), 1);
    assert!(
        paused
            .outcomes
            .iter()
            .all(|outcome| outcome.phase != Phase::Settle)
    );
    service
        .release_credit_hold(&tenant, hold_id, principal)
        .await
        .unwrap();
    finish_run(&service, &tenant, created.run_id).await;
    let complete = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(complete.run.state, PipelineRunState::Complete);
    assert_eq!(complete.run.credit_write_state, "complete");
    assert_eq!(complete.run.index_write_state, "complete");
    assert_eq!(
        backend
            .list_trace_credit_events(&tenant)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(index.entry_count(&tenant, PIPELINE_INDEX_ID), 1);

    let fail_bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
    let PipelineReceiptResult::Created(failed_index) = service
        .submit(&tenant, principal, "index-fail-key", &fail_bytes)
        .await
        .unwrap()
    else {
        panic!("index-failure receipt must create a run");
    };
    service
        .process_run(&tenant, failed_index.run_id, None)
        .await
        .unwrap();
    service
        .process_run(&tenant, failed_index.run_id, None)
        .await
        .unwrap();
    index.set_fault(IndexFault::FailBeforeApply);
    service
        .process_run(&tenant, failed_index.run_id, None)
        .await
        .unwrap();
    let interrupted = service
        .inspect(&tenant, failed_index.run_id)
        .await
        .unwrap()
        .unwrap();
    assert!(interrupted.run.credit_event_id.is_some());
    assert_eq!(interrupted.run.credit_write_state, "complete");
    assert_eq!(interrupted.run.index_write_state, "pending");
    assert!(
        interrupted
            .outcomes
            .iter()
            .all(|outcome| outcome.phase != Phase::Settle)
    );
    assert_eq!(
        backend
            .list_trace_credit_events(&tenant)
            .await
            .unwrap()
            .len(),
        2
    );
    finish_run(&service, &tenant, failed_index.run_id).await;
    let recovered = service
        .inspect(&tenant, failed_index.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovered.run.state, PipelineRunState::Complete);
    assert_eq!(recovered.run.index_write_state, "complete");
    assert_eq!(recovered.run.credit_write_state, "complete");
    assert_eq!(index.entry_count(&tenant, PIPELINE_INDEX_ID), 2);
}

#[tokio::test]
async fn crash_boundaries_six_through_eleven_converge_after_restart() {
    let Some(backend) = backend().await else {
        return;
    };
    for crash_point in [
        PipelineCrashPoint::AfterScoreCommit,
        PipelineCrashPoint::AfterIndexCommandStorage,
        PipelineCrashPoint::AfterIndexApply,
        PipelineCrashPoint::AfterInternalSettlement,
        PipelineCrashPoint::AfterSettleCommit,
        PipelineCrashPoint::AfterNearSubmit,
    ] {
        let tenant = format!("pipeline-crash-{:?}-{}", crash_point, Uuid::new_v4());
        let bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
        let root = tempfile::tempdir().unwrap();
        let artifacts = artifact_store(&root);
        let index = IsolatedPipelineIndex::new();
        let near = std::sync::Arc::new(RecordingNearAdapter::new());
        let payout = matches!(
            crash_point,
            PipelineCrashPoint::AfterSettleCommit | PipelineCrashPoint::AfterNearSubmit
        );
        let crashing = PipelineService::new_with_ops(
            backend.clone(),
            artifacts.clone(),
            None,
            Some(crash_point),
            index.clone(),
            near.clone(),
        )
        .unwrap();
        if payout {
            crashing.set_payout_enabled(true);
        }
        activate_operations(
            &crashing,
            &tenant,
            PIPELINE_FIXED_POSITIVE_MICROCREDITS,
            true,
        )
        .await;
        let PipelineReceiptResult::Created(created) = crashing
            .submit(&tenant, "principal_sha256:test", "crash-ops-key", &bytes)
            .await
            .unwrap()
        else {
            panic!("receipt must commit before worker crash");
        };
        crashing.process_one(&tenant, None).await.unwrap();
        if crash_point == PipelineCrashPoint::AfterScoreCommit {
            assert!(crashing.process_one(&tenant, None).await.is_err());
        } else {
            crashing.process_one(&tenant, None).await.unwrap();
            assert!(crashing.process_one(&tenant, None).await.is_err());
        }
        expire_leases(&backend, &tenant).await;
        let restarted = PipelineService::new_with_ops(
            backend.clone(),
            artifacts,
            None,
            None,
            index.clone(),
            near.clone(),
        )
        .unwrap();
        if payout {
            restarted.set_payout_enabled(true);
        }
        activate_operations(
            &restarted,
            &tenant,
            PIPELINE_FIXED_POSITIVE_MICROCREDITS,
            true,
        )
        .await;
        finish_run(&restarted, &tenant, created.run_id).await;
        let complete = restarted
            .inspect(&tenant, created.run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(complete.outcomes.len(), 4);
        assert_eq!(
            backend
                .list_trace_credit_events(&tenant)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(index.entry_count(&tenant, PIPELINE_INDEX_ID), 1);
        if matches!(
            crash_point,
            PipelineCrashPoint::AfterIndexCommandStorage | PipelineCrashPoint::AfterIndexApply
        ) {
            assert_eq!(crashing.settle_evaluations(), 1);
            assert_eq!(restarted.settle_evaluations(), 0);
        }
        if crash_point == PipelineCrashPoint::AfterScoreCommit {
            assert_eq!(crashing.score_evaluations(), 1);
            assert_eq!(restarted.score_evaluations(), 0);
        }
        if payout {
            assert_eq!(near.requests().len(), 1);
        }
        let replayed = serde_json::to_vec(&complete.outcomes).unwrap();
        let again = restarted
            .inspect(&tenant, created.run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(serde_json::to_vec(&again.outcomes).unwrap(), replayed);
    }
}

#[tokio::test]
async fn lost_index_response_and_stale_lease_reuse_sealed_command() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-lost-{}", Uuid::new_v4());
    let root = tempfile::tempdir().unwrap();
    let artifacts = artifact_store(&root);
    let index = IsolatedPipelineIndex::new();
    let service = PipelineService::new_with_ops(
        backend.clone(),
        artifacts,
        None,
        None,
        index.clone(),
        std::sync::Arc::new(RecordingNearAdapter::new()),
    )
    .unwrap();
    activate_operations(
        &service,
        &tenant,
        PIPELINE_FIXED_POSITIVE_MICROCREDITS,
        true,
    )
    .await;
    let bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
    let PipelineReceiptResult::Created(created) = service
        .submit(&tenant, "principal_sha256:test", "lost-key", &bytes)
        .await
        .unwrap()
    else {
        panic!("receipt must create a run");
    };
    service
        .process_run(&tenant, created.run_id, None)
        .await
        .unwrap();
    service
        .process_run(&tenant, created.run_id, None)
        .await
        .unwrap();
    index.set_fault(IndexFault::LostAfterApply);
    service
        .process_run(&tenant, created.run_id, None)
        .await
        .unwrap();
    assert_eq!(index.entry_count(&tenant, PIPELINE_INDEX_ID), 1);
    expire_leases(&backend, &tenant).await;
    let writer_calls = index.writer_calls();
    assert_eq!(service.settle_evaluations(), 1);
    finish_run(&service, &tenant, created.run_id).await;
    assert!(index.writer_calls() > writer_calls);
    assert_eq!(index.entry_count(&tenant, PIPELINE_INDEX_ID), 1);
    assert_eq!(service.settle_evaluations(), 1);
    let complete = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(complete.run.state, PipelineRunState::Complete);
    assert!(complete.run.index_command_hash.is_some());

    let conflict_key = IndexEntryKey {
        tenant_id: tenant.clone(),
        index_id: PIPELINE_INDEX_ID.to_string(),
        revision_id: complete.run.approved_revision_id.unwrap(),
        projection_id: PIPELINE_PROJECTION_ID.to_string(),
        model_id: PIPELINE_EMBEDDER_MODEL_ID.to_string(),
        chunk: 0,
    };
    assert_eq!(
        index.upsert(
            &conflict_key,
            &[0.0; 4],
            "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
        ),
        Err(IndexWriteError::ContentConflict)
    );
    assert_eq!(index.entry_count(&tenant, PIPELINE_INDEX_ID), 1);
}

#[tokio::test]
async fn near_payout_states_do_not_change_settle_outcome() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-near-{}", Uuid::new_v4());
    let root = tempfile::tempdir().unwrap();
    let artifacts = artifact_store(&root);
    let near = std::sync::Arc::new(RecordingNearAdapter::new());
    let service = PipelineService::new_with_ops(
        backend.clone(),
        artifacts,
        None,
        None,
        IsolatedPipelineIndex::new(),
        near.clone(),
    )
    .unwrap();
    activate_operations(
        &service,
        &tenant,
        PIPELINE_FIXED_POSITIVE_MICROCREDITS,
        false,
    )
    .await;
    let bytes = envelope_bytes(Uuid::new_v4(), "fixture-secret-not-present").await;
    let PipelineReceiptResult::Created(created) = service
        .submit(&tenant, "principal_sha256:test", "near-key", &bytes)
        .await
        .unwrap()
    else {
        panic!("receipt must create a run");
    };
    finish_run(&service, &tenant, created.run_id).await;
    let before = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.run.payout_state, "disabled");
    let outcome_bytes = serde_json::to_vec(&before.outcomes).unwrap();
    let disabled_items = backend
        .list_trace_near_credit_outbox_items(&tenant)
        .await
        .unwrap();
    assert_eq!(disabled_items.len(), 1);
    assert_eq!(
        disabled_items[0].status,
        TraceCreditSettlementNearStatus::Disabled
    );
    service.set_payout_enabled(true);
    near.fail_next();
    service
        .process_payout(&tenant, created.run_id)
        .await
        .unwrap();
    let failed = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::to_vec(&failed.outcomes).unwrap(), outcome_bytes);
    assert_eq!(failed.run.payout_state, "failed");
    assert_eq!(
        backend
            .list_trace_near_credit_outbox_items(&tenant)
            .await
            .unwrap()[0]
            .status,
        TraceCreditSettlementNearStatus::Failed
    );
    service
        .process_payout(&tenant, created.run_id)
        .await
        .unwrap();
    let after = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::to_vec(&after.outcomes).unwrap(), outcome_bytes);
    assert_eq!(after.run.payout_state, "confirmed");
    let items = backend
        .list_trace_near_credit_outbox_items(&tenant)
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].status, TraceCreditSettlementNearStatus::Confirmed);
    assert_eq!(near.requests().len(), 1);
    service
        .process_payout(&tenant, created.run_id)
        .await
        .unwrap();
    let replayed = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&replayed.outcomes).unwrap(),
        outcome_bytes
    );
    assert_eq!(replayed.run.payout_state, "confirmed");
    assert_eq!(near.requests().len(), 1);
}

#[tokio::test]
async fn phase_four_admission_review_and_quarantine_are_policy_driven() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-phase4-review-{}", Uuid::new_v4());
    let principal =
        "principal_sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let reviewer_a =
        "reviewer_sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let reviewer_b =
        "reviewer_sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    let (_root, service) = service(backend.clone(), None);

    let medium =
        envelope_bytes_with_risk(Uuid::new_v4(), "medium-risk", ResidualPiiRisk::Medium).await;
    let PipelineReceiptResult::Created(quarantined) = service
        .submit(&tenant, principal, "medium", &medium)
        .await
        .unwrap()
    else {
        panic!("medium risk must create a quarantined run");
    };
    assert_eq!(quarantined.admission_decision, "quarantine");
    assert_eq!(quarantined.next_phase, Some(Phase::Review));

    let blocked = service
        .process_run(&tenant, quarantined.run_id, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        blocked.last_error_label.as_deref(),
        Some("review_assessment_required")
    );
    assert_eq!(blocked.attempt_count, quarantined.attempt_count);

    let claim = service
        .claim_review(
            &tenant,
            quarantined.run_id,
            reviewer_a,
            Duration::minutes(5),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(
        service
            .claim_review(
                &tenant,
                quarantined.run_id,
                reviewer_b,
                Duration::minutes(5),
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .record_review_assessment(
                &claim,
                ReviewRecommendation::Approve,
                ReasonCode::new("privacy_resolved").unwrap(),
                Vec::new(),
            )
            .await
            .is_err()
    );
    service
        .record_review_assessment(
            &claim,
            ReviewRecommendation::Approve,
            ReasonCode::new("privacy_resolved").unwrap(),
            vec![ReasonCode::new("privacy_review_required").unwrap()],
        )
        .await
        .unwrap();
    finish_run(&service, &tenant, quarantined.run_id).await;
    let approved = service
        .inspect(&tenant, quarantined.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approved.outcomes.len(), 4);
    assert!(approved.run.transformed_object_ref_id.is_some());
    let review = approved
        .outcomes
        .iter()
        .find(|outcome| outcome.phase == Phase::Review)
        .unwrap();
    assert!(review.evidence["human_assessment_hash"].is_string());
    assert!(review.evidence["transformed_artifact_hash"].is_string());

    let rejected_bytes =
        envelope_bytes_with_risk(Uuid::new_v4(), "review-reject", ResidualPiiRisk::Medium).await;
    let PipelineReceiptResult::Created(rejected) = service
        .submit(&tenant, principal, "review-reject", &rejected_bytes)
        .await
        .unwrap()
    else {
        panic!("review rejection fixture must create a run");
    };
    let claim = service
        .claim_review(&tenant, rejected.run_id, reviewer_a, Duration::minutes(5))
        .await
        .unwrap()
        .unwrap();
    service
        .record_review_assessment(
            &claim,
            ReviewRecommendation::Reject,
            ReasonCode::new("review_privacy_rejected").unwrap(),
            Vec::new(),
        )
        .await
        .unwrap();
    finish_run(&service, &tenant, rejected.run_id).await;
    let rejected = service
        .inspect(&tenant, rejected.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rejected.outcomes.len(), 2);
    assert!(rejected.run.approved_revision_id.is_none());

    let high = envelope_bytes_with_risk(Uuid::new_v4(), "high-risk", ResidualPiiRisk::High).await;
    let PipelineReceiptResult::Created(high) = service
        .submit(&tenant, principal, "high", &high)
        .await
        .unwrap()
    else {
        panic!("high risk must create a terminal Admission run");
    };
    assert_eq!(high.state, PipelineRunState::Complete);
    assert_eq!(high.admission_decision, "reject");
    assert_eq!(
        service
            .inspect(&tenant, high.run_id)
            .await
            .unwrap()
            .unwrap()
            .outcomes
            .len(),
        1
    );
    let content_reads = backend
        .list_trace_audit_events(&tenant)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.reason.as_deref() == Some("pipeline_review_content_read"))
        .count();
    assert_eq!(content_reads, 2);
}

#[tokio::test]
async fn phase_four_policy_suspension_preserves_attempts_bundle_and_audit() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-phase4-suspend-{}", Uuid::new_v4());
    let principal =
        "principal_sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let operator =
        "operator_sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    let bytes = envelope_bytes(Uuid::new_v4(), "suspend").await;
    let (_root, service) = service(backend, None);
    let PipelineReceiptResult::Created(created) = service
        .submit(&tenant, principal, "suspend", &bytes)
        .await
        .unwrap()
    else {
        panic!("receipt must create a run");
    };
    let after_review = service
        .process_run(&tenant, created.run_id, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_review.next_phase, Some(Phase::Score));
    let attempts_before = after_review.attempt_count;
    service
        .intervene_policy(
            &tenant,
            &created.bundle_id,
            Phase::Score,
            "suspend",
            operator,
            "score_dependency_pause",
        )
        .await
        .unwrap();
    let blocked = service
        .process_run(&tenant, created.run_id, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(blocked.attempt_count, attempts_before);
    assert_eq!(blocked.bundle_id, created.bundle_id);
    assert_eq!(
        blocked.last_error_label.as_deref(),
        Some("bound_policy_suspended")
    );
    service
        .intervene_policy(
            &tenant,
            &created.bundle_id,
            Phase::Score,
            "resume",
            operator,
            "score_dependency_restored",
        )
        .await
        .unwrap();
    let interventions = service
        .list_policy_interventions(&tenant, &created.bundle_id)
        .await
        .unwrap();
    assert_eq!(interventions.len(), 2);
    assert!(
        interventions
            .iter()
            .all(|record| record.evidence_hash.starts_with("sha256:"))
    );
    finish_run(&service, &tenant, created.run_id).await;
    let complete = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(complete.run.bundle_id, created.bundle_id);
    assert_eq!(complete.outcomes.len(), 4);
    let outcome_bytes = serde_json::to_vec(&complete.outcomes).unwrap();
    service
        .intervene_policy(
            &tenant,
            &created.bundle_id,
            Phase::Review,
            "terminate",
            operator,
            "review_policy_retired",
        )
        .await
        .unwrap();
    assert!(
        service
            .intervene_policy(
                &tenant,
                &created.bundle_id,
                Phase::Review,
                "resume",
                operator,
                "invalid_resume",
            )
            .await
            .is_err()
    );
    assert_eq!(
        serde_json::to_vec(
            &service
                .inspect(&tenant, created.run_id)
                .await
                .unwrap()
                .unwrap()
                .outcomes
        )
        .unwrap(),
        outcome_bytes
    );
}

#[tokio::test]
async fn phase_four_settle_suspension_blocks_near_dispatch() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-phase4-payout-guard-{}", Uuid::new_v4());
    let principal =
        "principal_sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let operator =
        "operator_sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    let root = tempfile::tempdir().unwrap();
    let near = Arc::new(RecordingNearAdapter::new());
    let service = PipelineService::new_with_ops(
        backend,
        artifact_store(&root),
        None,
        Some(PipelineCrashPoint::AfterSettleCommit),
        IsolatedPipelineIndex::new(),
        near.clone(),
    )
    .unwrap();
    service.set_payout_enabled(true);
    let bundle =
        MinimalPolicyBundle::build_operations(PIPELINE_FIXED_POSITIVE_MICROCREDITS, false).unwrap();
    service
        .register_bundle(&tenant, &bundle.package)
        .await
        .unwrap();
    service
        .activate_bundle(&tenant, &bundle.package.bundle_id)
        .await
        .unwrap();
    let bytes = envelope_bytes(Uuid::new_v4(), "payout-guard").await;
    let PipelineReceiptResult::Created(created) = service
        .submit(&tenant, principal, "payout-guard", &bytes)
        .await
        .unwrap()
    else {
        panic!("receipt must create a run");
    };
    service
        .process_run(&tenant, created.run_id, None)
        .await
        .unwrap();
    service
        .process_run(&tenant, created.run_id, None)
        .await
        .unwrap();
    assert!(
        service
            .process_run(&tenant, created.run_id, None)
            .await
            .is_err()
    );
    let complete = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(complete.run.state, PipelineRunState::Complete);
    assert_eq!(complete.run.payout_state, "pending");
    service
        .intervene_policy(
            &tenant,
            &created.bundle_id,
            Phase::Settle,
            "suspend",
            operator,
            "payout_pause",
        )
        .await
        .unwrap();
    let blocked = service
        .process_payout(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(blocked.payout_state, "pending");
    assert!(near.requests().is_empty());
    service
        .intervene_policy(
            &tenant,
            &created.bundle_id,
            Phase::Settle,
            "resume",
            operator,
            "payout_resume",
        )
        .await
        .unwrap();
    service
        .process_payout(&tenant, created.run_id)
        .await
        .unwrap();
    assert_eq!(near.requests().len(), 1);
}

#[tokio::test]
async fn phase_four_withdrawal_excludes_index_but_preserves_committed_credit() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-phase4-withdraw-{}", Uuid::new_v4());
    let principal =
        "principal_sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bytes = envelope_bytes(Uuid::new_v4(), "withdraw-before-settle").await;
    let root = tempfile::tempdir().unwrap();
    let service = PipelineService::new(backend.clone(), artifact_store(&root), None).unwrap();
    let bundle =
        MinimalPolicyBundle::build_operations(PIPELINE_FIXED_POSITIVE_MICROCREDITS, true).unwrap();
    service
        .register_bundle(&tenant, &bundle.package)
        .await
        .unwrap();
    service
        .activate_bundle(&tenant, &bundle.package.bundle_id)
        .await
        .unwrap();
    let PipelineReceiptResult::Created(created) = service
        .submit(&tenant, principal, "withdraw-before-settle", &bytes)
        .await
        .unwrap()
    else {
        panic!("receipt must create a run");
    };
    service
        .process_run(&tenant, created.run_id, None)
        .await
        .unwrap();
    let after_score = service
        .process_run(&tenant, created.run_id, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_score.next_phase, Some(Phase::Settle));
    assert!(after_score.credit_event_id.is_some());

    service
        .withdraw_submission(&tenant, created.submission_id, principal)
        .await
        .unwrap();
    finish_run(&service, &tenant, created.run_id).await;
    let complete = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(complete.run.index_membership, "excluded");
    let settle = complete
        .outcomes
        .iter()
        .find(|outcome| outcome.phase == Phase::Settle)
        .unwrap();
    assert_eq!(
        settle.decision["credit_microcredits_finalized"],
        PIPELINE_FIXED_POSITIVE_MICROCREDITS
    );
    assert_eq!(settle.evidence["guard_reason"], "submission_inoperable");
    let events = backend.list_trace_credit_events(&tenant).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].settlement_state,
        TraceCreditSettlementState::Final
    );
    assert!(
        service
            .submit(&tenant, principal, "withdrawn-retry", &bytes)
            .await
            .unwrap_err()
            .to_string()
            .contains("content_tombstoned")
    );
}

#[tokio::test]
async fn phase_four_admission_limit_is_global_and_replay_counts_once() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-phase4-limit-{}", Uuid::new_v4());
    let principal =
        "principal_sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let root_a = tempfile::tempdir().unwrap();
    let root_b = tempfile::tempdir().unwrap();
    let service_a = PipelineService::new(backend.clone(), artifact_store(&root_a), None).unwrap();
    let service_b = PipelineService::new(backend.clone(), artifact_store(&root_b), None).unwrap();
    let bundle = MinimalPolicyBundle::build_with_limits(0, false, 1, 1).unwrap();
    service_a
        .register_bundle(&tenant, &bundle.package)
        .await
        .unwrap();
    service_a
        .activate_bundle(&tenant, &bundle.package.bundle_id)
        .await
        .unwrap();
    let bytes_a = envelope_bytes(Uuid::new_v4(), "quota-a").await;
    let bytes_b = envelope_bytes(Uuid::new_v4(), "quota-b").await;
    let (first, second) = tokio::join!(
        service_a.submit(&tenant, principal, "quota-a", &bytes_a),
        service_b.submit(&tenant, principal, "quota-b", &bytes_b)
    );
    let PipelineReceiptResult::Created(first) = first.unwrap() else {
        panic!("first concurrent receipt must create");
    };
    let PipelineReceiptResult::Created(second) = second.unwrap() else {
        panic!("second concurrent receipt must create");
    };
    let decisions = [&first.admission_decision, &second.admission_decision];
    assert_eq!(
        decisions
            .iter()
            .filter(|decision| decision.as_str() == "admit")
            .count(),
        1
    );
    assert_eq!(
        decisions
            .iter()
            .filter(|decision| decision.as_str() == "reject")
            .count(),
        1
    );
    let (admitted, admitted_bytes, admitted_key) = if first.admission_decision == "admit" {
        (&first, &bytes_a, "quota-a")
    } else {
        (&second, &bytes_b, "quota-b")
    };
    let replay = service_a
        .submit(&tenant, principal, admitted_key, admitted_bytes)
        .await
        .unwrap();
    let PipelineReceiptResult::Replayed(replay) = replay else {
        panic!("accepted receipt must replay");
    };
    assert_eq!(replay.run_id, admitted.run_id);
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let usage: i64 = tx
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM pipeline_admission_usage WHERE tenant_id = $1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);
    tx.commit().await.unwrap();
    assert_eq!(usage, 1);
}

#[tokio::test]
async fn phase_four_withdrawal_queues_completed_index_invalidation() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-phase4-invalidate-{}", Uuid::new_v4());
    let principal =
        "principal_sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let root = tempfile::tempdir().unwrap();
    let index = IsolatedPipelineIndex::new();
    let service = PipelineService::new_with_ops(
        backend,
        artifact_store(&root),
        None,
        None,
        index.clone(),
        Arc::new(RecordingNearAdapter::new()),
    )
    .unwrap();
    let bundle =
        MinimalPolicyBundle::build_operations(PIPELINE_FIXED_POSITIVE_MICROCREDITS, true).unwrap();
    service
        .register_bundle(&tenant, &bundle.package)
        .await
        .unwrap();
    service
        .activate_bundle(&tenant, &bundle.package.bundle_id)
        .await
        .unwrap();
    let bytes = envelope_bytes(Uuid::new_v4(), "invalidate").await;
    let PipelineReceiptResult::Created(created) = service
        .submit(&tenant, principal, "invalidate", &bytes)
        .await
        .unwrap()
    else {
        panic!("receipt must create a run");
    };
    finish_run(&service, &tenant, created.run_id).await;
    let complete = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    let revision = complete.run.approved_revision_id.unwrap();
    assert!(index.contains_revision(&tenant, PIPELINE_INDEX_ID, revision));
    service
        .withdraw_submission(&tenant, created.submission_id, principal)
        .await
        .unwrap();
    let pending = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.run.index_invalidation_state, "pending");
    service
        .process_index_invalidation(&tenant, created.run_id)
        .await
        .unwrap();
    let invalidated = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(invalidated.run.index_invalidation_state, "complete");
    assert!(!index.contains_revision(&tenant, PIPELINE_INDEX_ID, revision));
}

async fn activate_compatibility(service: &PipelineService, tenant: &str) {
    let bundle = MinimalPolicyBundle::build_compatibility().unwrap();
    service
        .register_bundle(tenant, &bundle.package)
        .await
        .unwrap();
    service
        .activate_bundle(tenant, &bundle.package.bundle_id)
        .await
        .unwrap();
}

async fn process_until_phase(
    service: &PipelineService,
    tenant: &str,
    run_id: Uuid,
    stop_before: Phase,
) {
    for _ in 0..24 {
        let inspection = service.inspect(tenant, run_id).await.unwrap().unwrap();
        if inspection.run.next_phase == Some(stop_before)
            || inspection.run.state == PipelineRunState::Complete
            || inspection.run.state == PipelineRunState::Failed
        {
            return;
        }
        if inspection.run.next_attempt_at > Utc::now() {
            let wait = (inspection.run.next_attempt_at - Utc::now())
                .to_std()
                .unwrap_or_else(|_| std::time::Duration::from_millis(5));
            tokio::time::sleep(wait + std::time::Duration::from_millis(10)).await;
        }
        service
            .process_run(tenant, run_id, Some(stop_before))
            .await
            .unwrap();
    }
    panic!("pipeline run did not reach {stop_before:?}");
}

#[tokio::test]
async fn phase_five_score_does_not_write_and_settle_ignores_later_index_mutation() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-phase5-score-{}", Uuid::new_v4());
    let principal = "principal_sha256:test";
    let root = tempfile::tempdir().unwrap();
    let index = IsolatedPipelineIndex::new();
    let service = PipelineService::new_with_ops(
        backend,
        artifact_store(&root),
        None,
        None,
        index.clone(),
        Arc::new(RecordingNearAdapter::new()),
    )
    .unwrap();
    activate_compatibility(&service, &tenant).await;
    let bytes = envelope_bytes(Uuid::new_v4(), "compat-score").await;
    let PipelineReceiptResult::Created(created) = service
        .submit(&tenant, principal, "compat-score", &bytes)
        .await
        .unwrap()
    else {
        panic!("receipt must create a run");
    };
    process_until_phase(&service, &tenant, created.run_id, Phase::Settle).await;
    let after_score = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_score.run.next_phase, Some(Phase::Settle));
    assert_eq!(index.writer_calls(), 0);
    assert_eq!(index.entry_count(&tenant, PIPELINE_INDEX_ID), 0);
    let score_outcome = after_score
        .outcomes
        .iter()
        .find(|outcome| outcome.phase == Phase::Score)
        .expect("Score outcome must exist before Settle");
    let include_eligible = score_outcome.evidence["include_eligible"]
        .as_bool()
        .unwrap_or(false);
    let decoy = IndexEntryKey {
        tenant_id: tenant.clone(),
        index_id: PIPELINE_INDEX_ID.to_string(),
        revision_id: Uuid::from_u128(99),
        projection_id: PIPELINE_PROJECTION_ID.to_string(),
        model_id: PIPELINE_EMBEDDER_MODEL_ID.to_string(),
        chunk: 0,
    };
    index
        .upsert(
            &decoy,
            &[1.0, 0.0],
            "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        )
        .unwrap();
    let writes_after_mutation = index.writer_calls();
    finish_run(&service, &tenant, created.run_id).await;
    let complete = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(complete.run.state, PipelineRunState::Complete);
    assert_eq!(
        complete.run.index_membership,
        if include_eligible {
            "included"
        } else {
            "excluded"
        }
    );
    assert_eq!(service.settle_evaluations(), 1);
    assert!(index.writer_calls() >= writes_after_mutation);
}

#[tokio::test]
async fn phase_five_score_dependency_failure_retries_without_credit() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-phase5-fail-{}", Uuid::new_v4());
    let principal = "principal_sha256:test";
    let (_root, service) = service(backend, None);
    activate_compatibility(&service, &tenant).await;
    let bytes = envelope_bytes(Uuid::new_v4(), "compat-fail").await;
    let PipelineReceiptResult::Created(created) = service
        .submit(&tenant, principal, "compat-fail", &bytes)
        .await
        .unwrap()
    else {
        panic!("receipt must create a run");
    };
    process_until_phase(&service, &tenant, created.run_id, Phase::Score).await;
    let review = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    let review_outcome = review
        .outcomes
        .iter()
        .find(|outcome| outcome.phase == Phase::Review)
        .cloned()
        .expect("Review must complete before Score");
    service.set_score_dependency_failure(true);
    service
        .process_run(&tenant, created.run_id, None)
        .await
        .unwrap();
    let failed = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        failed.run.last_error_label.as_deref(),
        Some(PIPELINE_SCORE_DEPENDENCY_LABEL)
    );
    assert!(
        !failed
            .outcomes
            .iter()
            .any(|outcome| outcome.phase == Phase::Score)
    );
    assert!(failed.run.credit_event_id.is_none());
    assert_eq!(
        failed
            .outcomes
            .iter()
            .find(|outcome| outcome.phase == Phase::Review)
            .map(|outcome| outcome.outcome_id),
        Some(review_outcome.outcome_id)
    );
    service.set_score_dependency_failure(false);
    finish_run(&service, &tenant, created.run_id).await;
    let complete = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(complete.run.state, PipelineRunState::Complete);
    assert!(
        complete
            .outcomes
            .iter()
            .any(|outcome| outcome.phase == Phase::Score)
    );
    assert_eq!(complete.run.bundle_id, review.run.bundle_id);
}

#[tokio::test]
async fn phase_six_contributor_status_derives_processing_credit_and_payout() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-phase6-status-{}", Uuid::new_v4());
    let principal = "principal_sha256:status";
    let (_root, service) = service(backend.clone(), None);
    activate_operations(
        &service,
        &tenant,
        PIPELINE_FIXED_POSITIVE_MICROCREDITS,
        false,
    )
    .await;
    let bytes = envelope_bytes(Uuid::new_v4(), "phase6-status").await;
    let PipelineReceiptResult::Created(created) = service
        .submit(&tenant, principal, "phase6-status", &bytes)
        .await
        .unwrap()
    else {
        panic!("receipt must create a run");
    };
    let product = PipelineProductStore::new(backend);
    let pending = product
        .contributor_statuses(&tenant, principal, &[created.submission_id])
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].processing, PipelineProcessingStatus::Pending);
    assert_eq!(pending[0].credit, PipelineCreditStatus::Unscored);
    assert_eq!(pending[0].payout, None);
    assert!(
        product
            .contributor_statuses(
                &tenant,
                "principal_sha256:unrelated",
                &[created.submission_id]
            )
            .await
            .unwrap()
            .is_empty()
    );

    finish_run(&service, &tenant, created.run_id).await;
    let complete = product
        .contributor_statuses(&tenant, principal, &[created.submission_id])
        .await
        .unwrap();
    assert_eq!(complete[0].processing, PipelineProcessingStatus::Complete);
    assert_eq!(complete[0].credit, PipelineCreditStatus::Finalized);
    assert_eq!(
        complete[0].score_microcredits,
        Some(PIPELINE_FIXED_POSITIVE_MICROCREDITS)
    );
    assert_eq!(complete[0].payout.as_deref(), Some("disabled"));
    assert_eq!(complete[0].bundle_id, created.bundle_id);
    assert!(complete[0].score_outcome_id.is_some());

    let attestation = product
        .own_score_attestation_entries(&tenant, principal)
        .await
        .unwrap();
    assert_eq!(attestation.len(), 1);
    assert_eq!(attestation[0].bundle_id, created.bundle_id);
    assert_eq!(
        attestation[0].credit_microcredits,
        PIPELINE_FIXED_POSITIVE_MICROCREDITS
    );
}

#[tokio::test]
async fn phase_six_export_snapshot_is_immutable_and_withdrawal_invalidates_it() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-phase6-export-{}", Uuid::new_v4());
    let principal = "principal_sha256:exportsource";
    let exporter = "exporter_sha256:customer";
    let (_root, service) = service(backend.clone(), None);
    activate_operations(&service, &tenant, 0, true).await;
    let bytes = envelope_bytes(Uuid::new_v4(), "phase6-export").await;
    let PipelineReceiptResult::Created(created) = service
        .submit(&tenant, principal, "phase6-export", &bytes)
        .await
        .unwrap()
    else {
        panic!("receipt must create a run");
    };
    finish_run(&service, &tenant, created.run_id).await;
    let product = PipelineProductStore::new(backend.clone());
    let request_key = sha256_prefixed(b"phase6-export-request");
    let purpose_hash = sha256_prefixed(b"compatibility-export");
    let snapshot = product
        .create_export_snapshot(
            &tenant,
            exporter,
            &request_key,
            "evaluation",
            &purpose_hash,
            10,
        )
        .await
        .unwrap();
    assert_eq!(snapshot.items.len(), 1);
    assert_eq!(snapshot.items[0].submission_id, created.submission_id);
    assert_eq!(snapshot.items[0].bundle_id, created.bundle_id);
    assert_eq!(
        snapshot.items[0].authorized_view_schema_id,
        "trace_commons.authorized_trace_view.v1"
    );
    let replay = product
        .create_export_snapshot(
            &tenant,
            exporter,
            &request_key,
            "evaluation",
            &purpose_hash,
            10,
        )
        .await
        .unwrap();
    assert_eq!(replay.snapshot_id, snapshot.snapshot_id);
    assert!(
        product
            .create_export_snapshot(
                &tenant,
                exporter,
                &request_key,
                "evaluation",
                &sha256_prefixed(b"different-purpose"),
                10,
            )
            .await
            .is_err()
    );
    let completed = product
        .complete_export_snapshot(&tenant, exporter, snapshot.snapshot_id)
        .await
        .unwrap();
    assert_eq!(completed.state, "complete");
    assert_eq!(completed.export_manifest_id, Some(snapshot.snapshot_id));

    let withdrawal = service
        .withdraw_submission(&tenant, created.submission_id, principal)
        .await
        .unwrap();
    assert_eq!(withdrawal.distribution_reach, "commons_distributed");
    let summary = product.lifecycle_summary(&tenant).await.unwrap();
    assert_eq!(summary.invalidated_export_snapshots, 1);
    let next = product
        .create_export_snapshot(
            &tenant,
            exporter,
            &sha256_prefixed(b"phase6-export-after-withdrawal"),
            "evaluation",
            &purpose_hash,
            10,
        )
        .await
        .unwrap();
    assert!(next.items.is_empty());
    assert!(
        !service.index().contains_revision(
            &tenant,
            PIPELINE_INDEX_ID,
            completed.items[0].registry_revision_id
        ) || service
            .inspect(&tenant, created.run_id)
            .await
            .unwrap()
            .unwrap()
            .run
            .index_invalidation_state
            == "pending"
    );
    let propagation = backend
        .list_trace_revocation_propagation_items(&tenant, created.submission_id)
        .await
        .unwrap();
    assert!(
        propagation
            .iter()
            .any(|item| item.action
                == trace_commons_server::trace_corpus_storage::TraceRevocationPropagationAction::DeleteObjectPayload)
    );
}

#[tokio::test]
async fn phase_six_index_invalidation_retries_and_exposes_terminal_failure() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-phase6-invalidation-{}", Uuid::new_v4());
    let principal = "principal_sha256:invalidation";
    let (_root, service) = service(backend.clone(), None);
    activate_operations(&service, &tenant, 0, true).await;
    let bytes = envelope_bytes(Uuid::new_v4(), "phase6-invalidation").await;
    let PipelineReceiptResult::Created(created) = service
        .submit(&tenant, principal, "phase6-invalidation", &bytes)
        .await
        .unwrap()
    else {
        panic!("receipt must create a run");
    };
    finish_run(&service, &tenant, created.run_id).await;
    service
        .withdraw_submission(&tenant, created.submission_id, principal)
        .await
        .unwrap();
    for _ in 0..5 {
        service.index().set_fault(IndexFault::FailInvalidation);
        service
            .process_index_invalidation(&tenant, created.run_id)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    }
    let failed = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.run.index_invalidation_state, "failed");
    let summary = PipelineProductStore::new(backend)
        .lifecycle_summary(&tenant)
        .await
        .unwrap();
    assert_eq!(summary.terminal_index_invalidation_failures, 1);
}

#[tokio::test]
async fn phase_seven_signed_package_qualification_gates_activation() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-phase7-package-{}", Uuid::new_v4());
    let runtime = CompatibilityScoreRuntime::reference(IsolatedPipelineIndex::new());
    let mut config = CompatibilityBundleConfig::local_reference();
    config.scorer_model_id = "near-ai-qwen-qualified-v1".to_string();
    config.embedder_model_id = "production-embedder-v1".to_string();
    config.projection_id = "trace-commons-production-projection-v1".to_string();
    config.index_id = "trace-commons-production-index-v1".to_string();
    let package = MinimalPolicyBundle::build_compatibility_candidate(&runtime, config)
        .unwrap()
        .package;
    let random = ring::rand::SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&random).unwrap();
    let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let package_hash = package.package_hash().unwrap();
    let signed = SignedBundlePackage {
        package,
        signature: BundlePackageSignature {
            algorithm: PACKAGE_SIGNATURE_ALGORITHM.to_string(),
            key_id: "phase7-release-key".to_string(),
            package_hash: package_hash.clone(),
            signature_base64url: base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(key_pair.sign(package_hash.as_bytes()).as_ref()),
        },
    };
    let trust = BundlePackageTrustStore::new([TrustedBundleKey {
        key_id: signed.signature.key_id.clone(),
        public_key_base64url: base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(key_pair.public_key().as_ref()),
    }])
    .unwrap();
    let metadata = BundleQualificationMetadata {
        corpus_digest: sha256_prefixed(b"phase7-corpus"),
        input_digest: sha256_prefixed(b"phase7-input"),
        configuration_digest: sha256_prefixed(b"phase7-configuration"),
        code_revision_hash: sha256_prefixed(b"phase7-code"),
        evidence_hash: sha256_prefixed(b"phase7-evidence"),
    };
    let qualification = PipelineQualificationStore::new(backend.clone());
    qualification
        .qualify_bundle(
            &tenant,
            &signed,
            &trust,
            &metadata,
            &ProductionDependencyProfile::production(),
        )
        .await
        .unwrap();
    let now = Utc::now();
    let evidence = REQUIRED_PHASE_SEVEN_DRILLS
        .iter()
        .map(|drill_id| DrillEvidence {
            drill_id: (*drill_id).to_string(),
            status: DrillStatus::Pass,
            safe_blockers: Vec::new(),
            observed_at: now,
            maximum_age_seconds: 3_600,
            evidence_hash: sha256_prefixed(drill_id.as_bytes()),
        })
        .collect::<Vec<_>>();
    let blocked_promotion = evaluate_promotion(&evidence[..1], now).unwrap();
    let production_dependencies = ProductionDependencyProfile::production();
    assert!(
        qualification
            .activate_qualified_bundle(
                &tenant,
                &signed.package.bundle_id,
                &blocked_promotion,
                &metadata.code_revision_hash,
                &production_dependencies,
            )
            .await
            .is_err()
    );
    let promotion = evaluate_promotion(&evidence, now).unwrap();
    assert!(
        qualification
            .activate_qualified_bundle(
                &tenant,
                &signed.package.bundle_id,
                &promotion,
                &metadata.code_revision_hash,
                &ProductionDependencyProfile::local_test(),
            )
            .await
            .is_err()
    );
    let different_code_revision = sha256_prefixed(b"different-code-revision");
    assert!(
        qualification
            .activate_qualified_bundle(
                &tenant,
                &signed.package.bundle_id,
                &promotion,
                &different_code_revision,
                &production_dependencies,
            )
            .await
            .is_err()
    );
    qualification
        .activate_qualified_bundle(
            &tenant,
            &signed.package.bundle_id,
            &promotion,
            &metadata.code_revision_hash,
            &production_dependencies,
        )
        .await
        .unwrap();
    let packages = PgPipelineStore::new(backend.clone());
    assert_eq!(
        packages.active_bundle_id(&tenant).await.unwrap().as_deref(),
        Some(signed.package.bundle_id.as_str())
    );
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let replacement_evidence = sha256_prefixed(b"replacement-evidence");
    let mutation = tx
        .execute(
            "UPDATE pipeline_bundle_qualifications
                SET evidence_hash = $3
              WHERE tenant_id = $1 AND bundle_id = $2",
            &[&tenant, &signed.package.bundle_id, &replacement_evidence],
        )
        .await;
    assert!(mutation.is_err());
    tx.rollback().await.unwrap();

    let unqualified_tenant = format!("pipeline-phase7-unqualified-{}", Uuid::new_v4());
    packages
        .register_bundle(&unqualified_tenant, &signed.package)
        .await
        .unwrap();
    let error = qualification
        .activate_qualified_bundle(
            &unqualified_tenant,
            &signed.package.bundle_id,
            &promotion,
            &metadata.code_revision_hash,
            &production_dependencies,
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains(PACKAGE_QUALIFICATION_MISSING_LABEL)
    );
}

#[tokio::test]
async fn phase_seven_index_rebuild_uses_commands_without_creating_credit_or_outcomes() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-phase7-rebuild-{}", Uuid::new_v4());
    let (_root, service) = service(backend, None);
    activate_operations(
        &service,
        &tenant,
        PIPELINE_FIXED_POSITIVE_MICROCREDITS,
        true,
    )
    .await;
    let bytes = envelope_bytes(Uuid::new_v4(), "phase7-rebuild").await;
    let PipelineReceiptResult::Created(created) = service
        .submit(
            &tenant,
            "principal_sha256:rebuild",
            "phase7-rebuild",
            &bytes,
        )
        .await
        .unwrap()
    else {
        panic!("receipt must create a run");
    };
    finish_run(&service, &tenant, created.run_id).await;
    let before = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    let rebuilt = IsolatedPipelineIndex::new();
    let first = service
        .rebuild_index_from_authoritative_commands(&tenant, rebuilt.clone())
        .await
        .unwrap();
    assert_eq!(first.command_count, 1);
    assert!(first.entry_count > 0);
    assert_eq!(first.unchanged_entry_count, 0);
    let second = service
        .rebuild_index_from_authoritative_commands(&tenant, rebuilt)
        .await
        .unwrap();
    assert_eq!(second.command_set_hash, first.command_set_hash);
    assert_eq!(second.unchanged_entry_count, second.entry_count);
    let after = service
        .inspect(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.outcomes, before.outcomes);
    assert_eq!(after.run.credit_event_id, before.run.credit_event_id);
    assert_eq!(
        after.run.settlement_batch_id,
        before.run.settlement_batch_id
    );
}

#[tokio::test]
async fn phase_seven_operational_summary_and_traceability_are_hash_only() {
    let Some(backend) = backend().await else {
        return;
    };
    let tenant = format!("pipeline-phase7-operations-{}", Uuid::new_v4());
    let (_root, service) = service(backend.clone(), None);
    activate_operations(&service, &tenant, 0, true).await;
    let bytes = envelope_bytes(Uuid::new_v4(), "ghp_PHASE7_SECRET_MUST_NOT_APPEAR").await;
    let PipelineReceiptResult::Created(created) = service
        .submit(
            &tenant,
            "principal_sha256:operations",
            "phase7-operations",
            &bytes,
        )
        .await
        .unwrap()
    else {
        panic!("receipt must create a run");
    };
    finish_run(&service, &tenant, created.run_id).await;
    let product = PipelineProductStore::new(backend);
    let summary = product.operational_summary(&tenant).await.unwrap();
    assert!(summary.tenant_isolation_control_passed);
    assert!(summary.audit_immutability_control_passed);
    let trace = product
        .forensic_trace(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(trace.phases.len(), 4);
    assert!(trace.phases.iter().all(|phase| {
        phase.decision_hash.starts_with("sha256:")
            && phase.evidence_hash.starts_with("sha256:")
            && phase.evaluation_hash.starts_with("sha256:")
    }));
    let output = serde_json::to_string(&(summary, trace)).unwrap();
    assert!(!output.contains("ghp_PHASE7_SECRET_MUST_NOT_APPEAR"));
}
