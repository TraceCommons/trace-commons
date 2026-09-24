// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Versioned pipeline runtime against PostgreSQL, as a role that cannot bypass RLS.

use std::collections::BTreeMap;
use std::sync::Arc;

use secrecy::SecretString;
use trace_commons_gate_api::pipeline::{
    AtomicUnits, InstrumentDescriptor, InstrumentId, InstrumentKind, Phase, PhaseResult,
    ReasonCode, ReviewDecision, ReviewEvaluation, ReviewEvidence, ReviewOutput,
};
use trace_commons_gate_api::{ReferenceEmbedder, ReferencePerplexityScorer};
use trace_commons_protocol::trace_contribution::{
    DeterministicTraceRedactor, RawTraceCaptureTurn, RawTraceContribution,
    RecordedTraceContributionOptions, ResidualPiiRisk, TraceContributionEnvelope, TraceRedactor,
};
use trace_commons_server::config::DatabaseConfig;
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::secrets::SecretsCrypto;
use trace_commons_server::trace_artifact_store::{
    LocalEncryptedTraceArtifactStore, TraceArtifactStore,
};
use trace_commons_server::versioned_pipeline::*;
use trace_commons_server::versioned_pipeline_bundle::{
    MinimalPolicyBundle, PipelineBundleConfig, PipelineInstrumentAwardConfig,
    dependency_content_hash,
};
use trace_commons_server::versioned_pipeline_credit::{
    RecordingSettlementAdapter, SettlementAdapter, SettlementAdapterRegistry,
};
use trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex;

const RUNTIME_ROLE: &str = "trace_pipeline_runtime_test";
static SETUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// `None` only when the variable is unset. Every failure after that panics.
async fn runtime_backend(pool_size: usize) -> Option<Arc<PgBackend>> {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL").ok()?;
    let _guard = SETUP_LOCK.lock().await;
    let owner = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 2))
        .await
        .expect("connect as migration owner");
    owner.run_migrations().await.expect("apply migrations");
    let client = owner
        .trace_pool_for_test()
        .get()
        .await
        .expect("owner client");
    client
        .batch_execute(&format!(
            "DO $$ BEGIN
            IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{RUNTIME_ROLE}')
            THEN CREATE ROLE {RUNTIME_ROLE} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS NOINHERIT;
            END IF;
         END $$;
         GRANT USAGE ON SCHEMA public TO {RUNTIME_ROLE};
         GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO {RUNTIME_ROLE};
         GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO {RUNTIME_ROLE};"
        ))
        .await
        .expect("provision runtime role");
    let mut runtime_url = reqwest::Url::parse(&url).expect("parse test URL");
    runtime_url
        .set_username(RUNTIME_ROLE)
        .expect("set runtime user");
    let backend = PgBackend::new(&DatabaseConfig::from_postgres_url(
        runtime_url.as_str(),
        pool_size,
    ))
    .await
    .expect("connect as runtime role");
    let row = backend
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await
        .unwrap();
    assert!(
        !row.get::<_, bool>(0) && !row.get::<_, bool>(1),
        "runtime role must not bypass RLS"
    );
    Some(Arc::new(backend))
}

/// Pinned per A4: an off-chain credit account, whole units only. Shared by
/// `seed_run` and the bundle-registry tests.
fn storage_rebate_descriptor() -> InstrumentDescriptor {
    InstrumentDescriptor {
        kind: InstrumentKind::CreditAccount,
        network: "pipeline-test".to_string(),
        contract: "storage-rebate".to_string(),
        decimals: 0,
    }
}

/// Seeds one admitted, review-pending run: a `trace_tenants` row, a minimal
/// `trace_submissions` row, its `trace_object_refs` row, and the
/// `pipeline_runs` row itself, plus the reference minimal bundle registered
/// and activated for `tenant_id`.
async fn seed_run(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    run_id: uuid::Uuid,
) -> PipelineRunRecord {
    let store = PgPipelineStore::new(backend.clone());
    let package = MinimalPolicyBundle::minimal_package(
        &PipelineBundleConfig {
            instrument_awards: vec![PipelineInstrumentAwardConfig {
                instrument_id: "storage_rebate".into(),
                atomic_units: AtomicUnits::from_raw(5),
                descriptor: storage_rebate_descriptor(),
            }],
            include_index: false,
            variant: None,
        },
        &ReferencePerplexityScorer::new(),
        &ReferenceEmbedder::new(),
    )
    .expect("build minimal bundle package");
    store
        .register_bundle(tenant_id, &package)
        .await
        .expect("register minimal bundle");
    store
        .activate_bundle_if_none(tenant_id, &package.bundle_id)
        .await
        .expect("activate minimal bundle");

    let submission_id = uuid::Uuid::new_v4();
    let trace_id = uuid::Uuid::new_v4();
    let object_ref_id = uuid::Uuid::new_v4();
    let request_content_hash = dependency_content_hash(format!("seed-request:{run_id}").as_bytes());
    let request_idempotency_key =
        dependency_content_hash(format!("seed-idempotency:{run_id}").as_bytes());

    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for seed_run");
    let tx = client.transaction().await.expect("tx for seed_run");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .expect("set tenant for seed_run");
    tx.execute(
        "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT (tenant_id) DO NOTHING",
        &[&tenant_id],
    )
    .await
    .expect("seed trace_tenants");
    tx.execute(
        "INSERT INTO trace_submissions (
            tenant_id, submission_id, trace_id, auth_principal_ref, schema_version,
            consent_policy_version, consent_scopes, allowed_uses, retention_policy_id,
            status, privacy_risk, redaction_pipeline_version, redaction_hash, redaction_counts
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
        &[
            &tenant_id,
            &submission_id,
            &trace_id,
            &"seed-principal",
            &"ironclaw.trace_contribution.v1",
            &"v1",
            &serde_json::json!([]),
            &serde_json::json!([]),
            &"retention-default",
            &"received",
            &"low",
            &"v1",
            &request_content_hash,
            &serde_json::json!({}),
        ],
    )
    .await
    .expect("seed trace_submissions");
    tx.execute(
        "INSERT INTO trace_object_refs (
            tenant_id, submission_id, object_ref_id, artifact_kind, object_store,
            object_key, content_sha256, encryption_key_ref, size_bytes
         ) VALUES ($1,$2,$3,'submitted_envelope',$4,$5,$6,$7,$8)",
        &[
            &tenant_id,
            &submission_id,
            &object_ref_id,
            &"seed-store",
            &format!("seed/{object_ref_id}"),
            &request_content_hash,
            &"seed-key-ref",
            &0i64,
        ],
    )
    .await
    .expect("seed trace_object_refs");
    tx.execute(
        "INSERT INTO pipeline_runs (
            tenant_id, run_id, submission_id, trace_id, bundle_id,
            request_idempotency_key, request_content_hash, source_object_ref_id,
            next_phase, state, admission_decision
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'review','pending','admit')",
        &[
            &tenant_id,
            &run_id,
            &submission_id,
            &trace_id,
            &package.bundle_id,
            &request_idempotency_key,
            &request_content_hash,
            &object_ref_id,
        ],
    )
    .await
    .expect("seed pipeline_runs");
    tx.commit().await.expect("commit seed_run");

    store
        .get_run(tenant_id, run_id)
        .await
        .expect("load seeded run")
        .expect("seeded run exists")
}

/// Sets a run's `next_attempt_at` to `NOW()` in a tenant-scoped transaction,
/// so a subsequent `claim_run` does not have to wait out a retry backoff.
async fn force_due(backend: &PgBackend, tenant_id: &str, run_id: uuid::Uuid) {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for force_due");
    let tx = client.transaction().await.expect("tx for force_due");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .expect("set tenant for force_due");
    tx.execute(
        "UPDATE pipeline_runs SET next_attempt_at = NOW() WHERE tenant_id = $1 AND run_id = $2",
        &[&tenant_id, &run_id],
    )
    .await
    .expect("force run due");
    tx.commit().await.expect("commit force_due");
}

#[tokio::test]
async fn stale_lease_cannot_commit_after_reclaim() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let store = PgPipelineStore::new(backend.clone());
    let tenant = format!("stale-{}", uuid::Uuid::new_v4());
    let run = seed_run(&backend, &tenant, uuid::Uuid::new_v4()).await;
    let first = store
        .claim_run(&tenant, run.run_id, chrono::Duration::seconds(1))
        .await
        .unwrap()
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    let second = store
        .claim_run(&tenant, run.run_id, chrono::Duration::seconds(30))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(first.lease_token, second.lease_token);
    let stale = store
        .mark_retry(&first, "minimal_policy_failed")
        .await
        .err()
        .unwrap();
    assert!(stale.to_string().contains("pipeline lease is stale"));
    assert_eq!(
        store
            .get_run(&tenant, run.run_id)
            .await
            .unwrap()
            .unwrap()
            .lease_token,
        second.lease_token
    );
}

#[tokio::test]
async fn register_bundle_refuses_a_changed_descriptor_for_a_registered_instrument() {
    let Some(backend) = runtime_backend(2).await else {
        return;
    };
    let store = PgPipelineStore::new(backend.clone());
    let tenant = format!("bundle-registry-{}", uuid::Uuid::new_v4());
    let scorer = ReferencePerplexityScorer::new();
    let embedder = ReferenceEmbedder::new();

    let package_for = |variant: &str, descriptor: InstrumentDescriptor| {
        MinimalPolicyBundle::minimal_package(
            &PipelineBundleConfig {
                instrument_awards: vec![PipelineInstrumentAwardConfig {
                    instrument_id: "storage_rebate".into(),
                    atomic_units: AtomicUnits::from_raw(5),
                    descriptor,
                }],
                include_index: false,
                variant: Some(variant.to_string()),
            },
            &scorer,
            &embedder,
        )
        .expect("build package")
    };

    let first = package_for("first", storage_rebate_descriptor());
    store
        .register_bundle(&tenant, &first)
        .await
        .expect("register first package");

    // A different package that pins the same instrument id to the same
    // descriptor is accepted.
    let equal_descriptor = package_for("second", storage_rebate_descriptor());
    assert_ne!(first.bundle_id, equal_descriptor.bundle_id);
    store
        .register_bundle(&tenant, &equal_descriptor)
        .await
        .expect("equal descriptor is accepted");

    // A changed `decimals` for the same instrument id is refused.
    let mut changed = storage_rebate_descriptor();
    changed.decimals = 3;
    let conflicting = package_for("third", changed);
    let error = store
        .register_bundle(&tenant, &conflicting)
        .await
        .expect_err("changed descriptor is refused");
    assert!(error.to_string().contains("bundle_instrument_conflict"));

    // Registering the same package again stays idempotent.
    store
        .register_bundle(&tenant, &first)
        .await
        .expect("re-registering the same package is idempotent");
}

/// `tokio_postgres::Error`'s `Display` only prints the error kind (`"db
/// error"`) for a `DbError`, not the server's message text, so assertions on
/// the trigger's text go through `DbError::message` instead.
fn db_error_message(error: &tokio_postgres::Error) -> String {
    error
        .as_db_error()
        .map(|db| db.message().to_string())
        .unwrap_or_else(|| error.to_string())
}

#[tokio::test]
async fn outcomes_are_immutable_and_tenant_scoped() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let store = PgPipelineStore::new(backend.clone());
    let tenant = format!("outcome-{}", uuid::Uuid::new_v4());
    let run = seed_run(&backend, &tenant, uuid::Uuid::new_v4()).await;

    let claimed = store
        .claim_run(&tenant, run.run_id, chrono::Duration::seconds(30))
        .await
        .unwrap()
        .unwrap();
    let source_hash = dependency_content_hash(b"outcomes-are-immutable-source");
    let result = PhaseResult {
        decision: ReviewDecision::Rejected {
            reason: ReasonCode::new("policy_rejected").unwrap(),
        },
        evidence: ReviewEvidence {
            source_content_hash: source_hash.clone(),
            result_content_hash: source_hash,
            content_changed: false,
            worker_identity: None,
            transformation_metadata_hash: None,
            human_assessment_hash: None,
            resolved_quarantine_reasons: Vec::new(),
        },
        evaluation: ReviewEvaluation {
            rule_id: "test_rejection_v1".to_string(),
        },
    };
    let output = ReviewOutput::rejected(result).unwrap();
    let stored = StoredPhaseResult::from_result(Phase::Review, output.result()).unwrap();
    // Review commits through `commit_review`, not the generic `commit_phase`
    // -- `commit_phase` no longer knows how to set the approved-content
    // columns together with `approved_revision_id`, which is the whole
    // point of decision D7 (see `pipeline_runs_approved_content_shape`).
    store
        .commit_review(&claimed, stored, None)
        .await
        .expect("commit the rejected Review outcome");

    let outcomes = store.list_outcomes(&tenant, run.run_id).await.unwrap();
    assert_eq!(outcomes.len(), 1);
    let outcome_id = outcomes[0].outcome_id;

    // An UPDATE inside a tenant-scoped transaction, as `PgPipelineStore`
    // itself would open one, is rejected by the immutability trigger.
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for update attempt");
    let tx = client.transaction().await.expect("tx for update attempt");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .expect("set tenant for update attempt");
    let update_err = tx
        .execute(
            "UPDATE phase_outcomes SET decision = decision
             WHERE tenant_id = $1 AND outcome_id = $2",
            &[&tenant, &outcome_id],
        )
        .await
        .expect_err("update must be rejected");
    assert!(
        db_error_message(&update_err).contains("phase outcomes are immutable"),
        "unexpected update error: {update_err:?}"
    );
    drop(tx);

    // A DELETE, in a fresh transaction, is rejected the same way.
    let tx = client.transaction().await.expect("tx for delete attempt");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .expect("set tenant for delete attempt");
    let delete_err = tx
        .execute(
            "DELETE FROM phase_outcomes WHERE tenant_id = $1 AND outcome_id = $2",
            &[&tenant, &outcome_id],
        )
        .await
        .expect_err("delete must be rejected");
    assert!(
        db_error_message(&delete_err).contains("phase outcomes are immutable"),
        "unexpected delete error: {delete_err:?}"
    );
    drop(tx);

    let other_tenant = format!("outcome-other-{}", uuid::Uuid::new_v4());
    let other_outcomes = store
        .list_outcomes(&other_tenant, run.run_id)
        .await
        .unwrap();
    assert!(other_outcomes.is_empty());
}

#[tokio::test]
async fn attempts_exhaust_to_failed_but_transient_retries_do_not_charge() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let store = PgPipelineStore::new(backend.clone());
    let tenant = format!("budget-{}", uuid::Uuid::new_v4());
    let run = seed_run(&backend, &tenant, uuid::Uuid::new_v4()).await;
    for _ in 0..3 {
        let claimed = store
            .claim_run(&tenant, run.run_id, chrono::Duration::seconds(30))
            .await
            .unwrap()
            .unwrap();
        let released = store
            .mark_transient_retry(&claimed, "embedder_unavailable")
            .await
            .unwrap();
        assert_eq!(released.attempt_count, 0);
        assert_eq!(
            released.last_error_label.as_deref(),
            Some("embedder_unavailable")
        );
        force_due(&backend, &tenant, run.run_id).await;
    }
    for attempt in 1..=5 {
        let claimed = store
            .claim_run(&tenant, run.run_id, chrono::Duration::seconds(30))
            .await
            .unwrap()
            .unwrap();
        let after = store
            .mark_retry(&claimed, "minimal_policy_failed")
            .await
            .unwrap();
        assert_eq!(after.attempt_count, attempt);
        force_due(&backend, &tenant, run.run_id).await;
    }
    let failed = store.get_run(&tenant, run.run_id).await.unwrap().unwrap();
    assert_eq!(failed.state, PipelineRunState::Failed);
    assert_eq!(
        failed.last_error_label.as_deref(),
        Some("attempts_exhausted")
    );
}

fn artifact_store(root: &tempfile::TempDir) -> Arc<dyn TraceArtifactStore> {
    Arc::new(LocalEncryptedTraceArtifactStore::new(
        root.path(),
        SecretsCrypto::new(SecretString::from(
            "pipeline-runtime-test-master-key-32-bytes".to_string(),
        ))
        .unwrap(),
    ))
}

/// A redacted, low-risk envelope, as port lines 109 to 133 build one --
/// changed to return the envelope itself rather than its serialized bytes,
/// so callers can both submit it and read its fields (submission_id,
/// trace_id, privacy.redaction_hash) without a round trip through JSON.
async fn envelope(submission_id: uuid::Uuid) -> TraceContributionEnvelope {
    let now = chrono::Utc::now();
    let raw = RawTraceContribution::from_capture_turns(
        &[RawTraceCaptureTurn {
            user_input: "Inspect the bounded runtime fixture.".to_string(),
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
    envelope.privacy.residual_pii_risk = ResidualPiiRisk::Low;
    envelope
}

fn minimal_config(include_index: bool) -> PipelineBundleConfig {
    PipelineBundleConfig {
        instrument_awards: vec![],
        include_index,
        variant: None,
    }
}

/// Builds a service over an isolated index (as both reader and writer), the
/// reference scorer and embedder, and `storage_rebate`/`trace_credit`
/// recording settlement adapters with an uncapped (`u64::MAX`) cap for each,
/// on payout rail `none`. `crash_point`, when given, is wired through
/// `PipelineServiceBuilder::with_crash_point` -- for tests that must observe
/// a mid-transaction crash and prove the retry resumes from durable state
/// alone, rather than from in-memory continuation.
async fn test_service(
    backend: Arc<PgBackend>,
    artifact_store: Arc<dyn TraceArtifactStore>,
    config: PipelineBundleConfig,
    crash_point: Option<PipelineCrashPoint>,
) -> (
    Arc<PipelineService>,
    Arc<IsolatedPipelineIndex>,
    Vec<Arc<RecordingSettlementAdapter>>,
) {
    let scorer = Arc::new(ReferencePerplexityScorer::new());
    let embedder = Arc::new(ReferenceEmbedder::new());
    let package = MinimalPolicyBundle::minimal_package(&config, scorer.as_ref(), embedder.as_ref())
        .expect("build minimal bundle package");
    let index = IsolatedPipelineIndex::new();
    let storage_rebate = RecordingSettlementAdapter::new(
        InstrumentId::new("storage_rebate").unwrap(),
        "recording_storage_rebate_test_only",
        "none",
    );
    let trace_credit = RecordingSettlementAdapter::new(
        InstrumentId::trace_credit(),
        "recording_trace_credit_test_only",
        "none",
    );
    let adapters = vec![storage_rebate.clone(), trace_credit.clone()];
    let registry = SettlementAdapterRegistry::new(
        adapters
            .iter()
            .cloned()
            .map(|adapter| adapter as Arc<dyn SettlementAdapter>)
            .collect(),
    )
    .expect("build settlement adapter registry");
    let caps = PipelineCaps {
        per_instrument_atomic_units: BTreeMap::from([
            (
                "storage_rebate".to_string(),
                AtomicUnits::from_raw(u128::MAX),
            ),
            (
                InstrumentId::trace_credit().as_str().to_string(),
                AtomicUnits::from_raw(u128::MAX),
            ),
        ]),
    };
    let mut builder = PipelineServiceBuilder::new(
        backend,
        artifact_store,
        package,
        index.clone(),
        index.clone(),
        registry,
        caps,
    )
    .with_scorer(scorer)
    .with_embedder(embedder);
    if let Some(crash_point) = crash_point {
        builder = builder.with_crash_point(crash_point);
    }
    let service = builder.build().expect("build pipeline service");
    (Arc::new(service), index, adapters)
}

fn receipt<'a>(
    tenant: &'a str,
    key: &'a str,
    raw: &'a [u8],
    envelope: &'a TraceContributionEnvelope,
    limits: PipelineAdmissionLimits,
) -> PipelineReceiptRequest<'a> {
    PipelineReceiptRequest {
        tenant_id: tenant,
        actor_principal_ref: "principal_sha256:test",
        counts_toward_quota: true,
        request_idempotency_key: key,
        request_bytes: raw,
        server_envelope: envelope,
        residual_risk_basis: &[],
        limits,
    }
}

const NO_LIMITS: PipelineAdmissionLimits = PipelineAdmissionLimits {
    max_per_tenant_per_hour: 0,
    max_per_principal_per_hour: 0,
};

/// `SELECT COUNT(*)` over `pipeline_runs` for `tenant_id`, in its own
/// tenant-scoped transaction.
async fn count_runs(backend: &Arc<PgBackend>, tenant_id: &str) -> i64 {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for count_runs");
    let tx = client.transaction().await.expect("tx for count_runs");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .expect("set tenant for count_runs");
    let count: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM pipeline_runs WHERE tenant_id = $1",
            &[&tenant_id],
        )
        .await
        .expect("count runs")
        .get(0);
    tx.commit().await.expect("commit count_runs");
    count
}

/// `SELECT COUNT(*)` over `pipeline_receipt_artifacts` for `tenant_id`, in
/// its own tenant-scoped transaction. A refused receipt (tombstoned or
/// quota-exceeded) never reaches the staging insert, so this is also the
/// count of receipts that got as far as storing an artifact.
async fn count_staged_artifacts(backend: &Arc<PgBackend>, tenant_id: &str) -> i64 {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for count_staged_artifacts");
    let tx = client
        .transaction()
        .await
        .expect("tx for count_staged_artifacts");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .expect("set tenant for count_staged_artifacts");
    let count: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM pipeline_receipt_artifacts WHERE tenant_id = $1",
            &[&tenant_id],
        )
        .await
        .expect("count staged artifacts")
        .get(0);
    tx.commit().await.expect("commit count_staged_artifacts");
    count
}

/// Recursively counts regular files under `path`. Used to confirm a refused
/// receipt left no ciphertext on disk.
fn count_files_under(path: &std::path::Path) -> usize {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let entry_path = entry.path();
        if entry_path.is_dir() {
            count += count_files_under(&entry_path);
        } else {
            count += 1;
        }
    }
    count
}

#[tokio::test]
async fn receipt_replay_and_conflict_are_exact() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(false),
        None,
    )
    .await;
    let tenant = format!("replay-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) = service
        .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .unwrap()
    else {
        panic!("first receipt creates a run")
    };
    let PipelineReceiptResult::Replayed(replayed) = service
        .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .unwrap()
    else {
        panic!("identical bytes replay")
    };
    assert_eq!(created.run_id, replayed.run_id);
    let mut changed = raw.clone();
    changed.push(b' ');
    assert!(matches!(
        service
            .submit(receipt(&tenant, &key, &changed, &env, NO_LIMITS))
            .await
            .unwrap(),
        PipelineReceiptResult::ContentConflict
    ));
    assert_eq!(count_runs(&backend, &tenant).await, 1);
}

/// A tombstone can only reference a submission that already exists (the
/// table's foreign key). In production a tombstone is always created for a
/// PRIOR submission -- the one that was later withdrawn or redacted -- and
/// matches a fresh resubmission by `trace_id`/`redaction_hash`, not by
/// `submission_id`. This test reproduces that shape: it seeds an unrelated
/// prior submission, tombstones it by the redaction hash the new envelope
/// carries, and submits the new envelope under a different submission id.
#[tokio::test]
async fn tombstoned_content_is_refused_before_the_store() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(false),
        None,
    )
    .await;
    let tenant = format!("tombstone-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();

    let prior_submission_id = uuid::Uuid::new_v4();
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for tombstone seed");
    let tx = client.transaction().await.expect("tx for tombstone seed");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .expect("set tenant for tombstone seed");
    tx.execute(
        "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT (tenant_id) DO NOTHING",
        &[&tenant],
    )
    .await
    .expect("seed trace_tenants");
    tx.execute(
        "INSERT INTO trace_submissions (
            tenant_id, submission_id, trace_id, auth_principal_ref, schema_version,
            consent_policy_version, consent_scopes, allowed_uses, retention_policy_id,
            status, privacy_risk, redaction_pipeline_version, redaction_hash, redaction_counts
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
        &[
            &tenant,
            &prior_submission_id,
            &uuid::Uuid::new_v4(),
            &"seed-principal",
            &"ironclaw.trace_contribution.v1",
            &"v1",
            &serde_json::json!([]),
            &serde_json::json!([]),
            &"retention-default",
            &"revoked",
            &"low",
            &"v1",
            &dependency_content_hash(b"tombstone-test-prior-submission"),
            &serde_json::json!({}),
        ],
    )
    .await
    .expect("seed prior trace_submissions");
    tx.execute(
        "INSERT INTO trace_tombstones (
            tenant_id, tombstone_id, submission_id, trace_id, redaction_hash, reason,
            effective_at, created_by_principal_ref
         ) VALUES ($1,$2,$3,$4,$5,$6,NOW(),$7)",
        &[
            &tenant,
            &uuid::Uuid::new_v4(),
            &prior_submission_id,
            &Option::<uuid::Uuid>::None,
            &Some(env.privacy.redaction_hash.clone()),
            &"withdrawn",
            &"seed-principal",
        ],
    )
    .await
    .expect("seed trace_tombstones");
    tx.commit().await.expect("commit tombstone seed");

    let result = service
        .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .unwrap();
    assert!(matches!(result, PipelineReceiptResult::Tombstoned));
    assert_eq!(count_runs(&backend, &tenant).await, 0);
    assert_eq!(
        count_staged_artifacts(&backend, &tenant).await,
        0,
        "a tombstoned receipt stores no pipeline_receipt_artifacts row"
    );
    assert_eq!(
        count_files_under(dir.path()),
        0,
        "a tombstoned receipt writes no artifact file"
    );
}

#[tokio::test]
async fn quota_is_counted_before_the_store_under_concurrency() {
    let Some(backend) = runtime_backend(8).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(false),
        None,
    )
    .await;
    let tenant = format!("quota-{}", uuid::Uuid::new_v4());
    let limits = PipelineAdmissionLimits {
        max_per_tenant_per_hour: 3,
        max_per_principal_per_hour: 0,
    };
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let service = service.clone();
        let tenant = tenant.clone();
        tasks.push(tokio::spawn(async move {
            let env = envelope(uuid::Uuid::new_v4()).await;
            let raw = serde_json::to_vec(&env).unwrap();
            let key = env.submission_id.to_string();
            service
                .submit(receipt(&tenant, &key, &raw, &env, limits))
                .await
                .unwrap()
        }));
    }
    let mut created = 0;
    let mut refused = 0;
    for task in tasks {
        match task.await.unwrap() {
            PipelineReceiptResult::Created(_) => created += 1,
            PipelineReceiptResult::QuotaExceeded(PipelineQuotaScope::Tenant) => refused += 1,
            other => panic!("unexpected receipt result {other:?}"),
        }
    }
    assert_eq!((created, refused), (3, 3));
    assert_eq!(count_runs(&backend, &tenant).await, 3);
    assert_eq!(
        count_staged_artifacts(&backend, &tenant).await,
        3,
        "refused receipts store nothing"
    );
}

#[tokio::test]
async fn pool_size_one_receipt_does_not_nest_checkouts() {
    let Some(backend) = runtime_backend(1).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _, _) =
        test_service(backend, artifact_store(&dir), minimal_config(false), None).await;
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        service.submit(receipt("pool-one", &key, &raw, &env, NO_LIMITS)),
    )
    .await
    .expect("a receipt with one pool connection must not wait on itself");
    assert!(matches!(
        result.unwrap(),
        PipelineReceiptResult::Created(_) | PipelineReceiptResult::Replayed(_)
    ));
}

/// Sets a tenant on the given client and opens a transaction for it. A tiny
/// helper shared by the two tests below, which each need to read rows the
/// runner wrote in a separate tenant-scoped transaction of their own.
async fn tenant_tx<'a>(
    client: &'a mut deadpool_postgres::Client,
    tenant_id: &str,
) -> deadpool_postgres::Transaction<'a> {
    let tx = client.transaction().await.expect("open tenant tx");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .expect("set tenant for tx");
    tx
}

/// Review focus item 2: the approved content commits as its own object, and
/// its revision, object reference, derived record, and phase transition all
/// land together with the outcome, in one transaction.
#[tokio::test]
async fn review_commits_approved_revision_provenance_and_transition_together() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(false),
        None,
    )
    .await;
    let tenant = format!("review-commit-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) = service
        .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .unwrap()
    else {
        panic!("receipt creates a run")
    };
    let request_content_hash = created.request_content_hash.clone();

    let processed = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("the seeded run is claimable");

    assert_eq!(processed.next_phase, Some(Phase::Score));
    assert_eq!(processed.request_content_hash, request_content_hash);
    let approved_revision_id = processed
        .approved_revision_id
        .expect("Review approval records a revision id");
    let approved_object_ref_id = processed
        .approved_object_ref_id
        .expect("Review approval records an object ref id");
    let approved_content_hash = processed
        .approved_content_hash
        .clone()
        .expect("Review approval records a content hash");

    let outcomes = service
        .store()
        .list_outcomes(&tenant, processed.run_id)
        .await
        .unwrap();
    let review_outcome = outcomes
        .into_iter()
        .find(|outcome| outcome.phase == Phase::Review)
        .expect("Review outcome recorded");
    let evidence: ReviewEvidence = serde_json::from_value(review_outcome.evidence).unwrap();
    assert_eq!(approved_content_hash, evidence.result_content_hash);

    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &tenant).await;
    let row = tx
        .query_one(
            "SELECT worker_version, output_object_ref_id, input_hash
               FROM trace_derived_records
              WHERE tenant_id = $1 AND derived_id = $2",
            &[&tenant, &approved_revision_id],
        )
        .await
        .expect("derived record for the approved revision exists");
    tx.commit().await.unwrap();
    let worker_version: String = row.get("worker_version");
    let output_object_ref_id: uuid::Uuid = row.get("output_object_ref_id");
    let input_hash: String = row.get("input_hash");
    assert_eq!(worker_version, "minimal_review_passthrough");
    assert_eq!(output_object_ref_id, approved_object_ref_id);
    assert_eq!(input_hash, dependency_content_hash(&raw));

    let approved_bytes = service.load_approved_bytes(&processed).await.unwrap();
    assert_eq!(
        dependency_content_hash(&approved_bytes),
        approved_content_hash
    );
}

/// Review focus item 2's crash test: a crash between the approved-content
/// artifact write and the database commit must leave exactly one revision
/// once the run retries, and the retry reuses the deterministic object key
/// the crashed attempt already wrote to.
#[tokio::test]
async fn review_crash_after_artifact_storage_reuses_one_revision() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(false),
        Some(PipelineCrashPoint::AfterReviewArtifactStorage),
    )
    .await;
    let tenant = format!("review-crash-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) = service
        .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .unwrap()
    else {
        panic!("receipt creates a run")
    };

    let crashed = service.process_run(&tenant, created.run_id).await;
    let error = crashed.expect_err("the injected crash must propagate as an error");
    assert_eq!(error.to_string(), INJECTED_PIPELINE_CRASH);

    // Expire the lease: a direct UPDATE, a time shortcut in the test, not a
    // processor call. The crashed attempt never called `mark_retry` or
    // `mark_failed` (the injected crash propagates unchanged, as a real
    // process crash would), so the run is otherwise stuck `leased` until its
    // lease naturally expires.
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &tenant).await;
    tx.execute(
        "UPDATE pipeline_runs SET lease_expires_at = NOW() - INTERVAL '1 second'
         WHERE tenant_id = $1 AND run_id = $2",
        &[&tenant, &created.run_id],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let processed = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("the retry claims and completes Review");
    assert_eq!(processed.next_phase, Some(Phase::Score));
    let approved_object_ref_id = processed
        .approved_object_ref_id
        .expect("the retry records an approval");

    // The object id -- and so the object ref id -- is derived from the run
    // id alone, so both the crashed attempt and the retry compute the same
    // one; the retry's `put_serialized_json` overwrote the same encrypted
    // object the crashed attempt already wrote, rather than orphaning one.
    let expected_object_ref_id = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("tracecommons:pipeline-approved-object:{}", created.run_id).as_bytes(),
    );
    assert_eq!(approved_object_ref_id, expected_object_ref_id);

    let outcomes = service
        .store()
        .list_outcomes(&tenant, processed.run_id)
        .await
        .unwrap();
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome.phase == Phase::Review)
            .count(),
        1,
        "exactly one Review outcome after the crash and its retry"
    );

    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &tenant).await;
    let derived_rows = tx
        .query(
            "SELECT output_object_ref_id FROM trace_derived_records
             WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant, &created.submission_id],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(derived_rows.len(), 1, "exactly one derived record");
    let output_object_ref_id: uuid::Uuid = derived_rows[0].get("output_object_ref_id");
    assert_eq!(output_object_ref_id, approved_object_ref_id);

    let bytes = service.load_approved_bytes(&processed).await.unwrap();
    assert_eq!(
        dependency_content_hash(&bytes),
        processed.approved_content_hash.unwrap()
    );
}
