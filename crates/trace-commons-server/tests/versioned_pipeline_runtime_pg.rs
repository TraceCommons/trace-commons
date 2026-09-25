// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Versioned pipeline runtime against PostgreSQL, as a role that cannot bypass RLS.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use secrecy::SecretString;
use sha2::{Digest, Sha256};
use tokio_postgres::NoTls;
use trace_commons_gate_api::pipeline::{
    AdmissionDecision, AtomicUnits, IndexMembershipDecision, InstrumentAward, InstrumentDescriptor,
    InstrumentId, InstrumentKind, InstrumentSettlementOutcome, Phase, PhaseResult, ReasonCode,
    ReviewDecision, ReviewEvaluation, ReviewEvidence, ReviewOutput, ScoreEvidence, SettleDecision,
    SettleEvidence, TRACE_CREDIT_DECIMALS,
};
use trace_commons_gate_api::{
    Embedder, IndexEntryKey, IndexUpsertResult, ReferenceEmbedder, ReferencePerplexityScorer,
    VectorIndexWriter,
};
use trace_commons_protocol::trace_contribution::{
    DeterministicTraceRedactor, RawTraceCaptureTurn, RawTraceContribution,
    RecordedTraceContributionOptions, ResidualPiiRisk, TraceContributionEnvelope, TraceRedactor,
    retention_policy_for_trace,
};
use trace_commons_server::config::DatabaseConfig;
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::secrets::SecretsCrypto;
use trace_commons_server::trace_artifact_store::{
    LocalEncryptedTraceArtifactStore, TraceArtifactKind, TraceArtifactStore,
};
use trace_commons_server::trace_corpus_storage::{
    TraceCorpusStore, TraceCreditHoldReason, TraceCreditHoldWrite,
};
use trace_commons_server::versioned_pipeline::*;
use trace_commons_server::versioned_pipeline_bundle::{
    IdentifiedEmbedder, MINIMAL_INDEX_ID, MINIMAL_PROJECTION_ID, MinimalPolicyBundle,
    PipelineBundleConfig, PipelineInstrumentAwardConfig, dependency_content_hash,
    pipeline_operation_ref, pipeline_result_ref,
};
use trace_commons_server::versioned_pipeline_credit::{
    RecordingSettlementAdapter, SettlementAdapter, SettlementAdapterRegistry, SettlementRequest,
    credit_account_hash,
};
use trace_commons_server::versioned_pipeline_index::{IndexFault, IsolatedPipelineIndex};

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

/// Pinned per A4: `nep141` on `testnet`, six decimals. Shared by the Score
/// tests below.
fn trace_credit_descriptor() -> InstrumentDescriptor {
    InstrumentDescriptor {
        kind: InstrumentKind::Nep141,
        network: "testnet".to_string(),
        contract: "trace-credit.testnet".to_string(),
        decimals: TRACE_CREDIT_DECIMALS,
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

/// Corrupts the stored `package` for `(tenant_id, bundle_id)` in place, as
/// an owner connection rather than through `PgPipelineStore` -- proving a
/// tampered row, not a tampering API `PgPipelineStore` would ever offer.
/// `pipeline_bundle_packages` carries its own immutability trigger
/// (`pipeline_bundle_packages_reject_update`, migration V76), so this drops
/// it and recreates it -- exactly as the migration defines it -- inside the
/// same transaction that performs the `UPDATE`. Flips one hex digit of the
/// first stored artifact so its bytes no longer hash to the key they are
/// stored under (`BundlePackage::validate`'s `ArtifactHashMismatch`).
async fn tamper_stored_bundle_package(tenant_id: &str, bundle_id: &str) {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .expect("TRACE_COMMONS_PG_TEST_DATABASE_URL must be set for this test");
    let (mut client, connection) = tokio_postgres::connect(&url, NoTls)
        .await
        .expect("connect as the migration owner");
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let tx = client
        .transaction()
        .await
        .expect("open owner transaction for tampering");
    tx.batch_execute(
        "DROP TRIGGER pipeline_bundle_packages_reject_update ON pipeline_bundle_packages;",
    )
    .await
    .expect("drop the immutability trigger");

    let row = tx
        .query_one(
            "SELECT package FROM pipeline_bundle_packages
             WHERE tenant_id = $1 AND bundle_id = $2",
            &[&tenant_id, &bundle_id],
        )
        .await
        .expect("load the stored package");
    let mut package: serde_json::Value = row.get("package");
    let artifacts = package
        .get_mut("artifacts")
        .and_then(serde_json::Value::as_object_mut)
        .expect("the package carries an artifacts object");
    let (_, value) = artifacts
        .iter_mut()
        .next()
        .expect("the package carries at least one artifact");
    let hex = value
        .as_str()
        .expect("artifact bytes are hex-encoded")
        .to_string();
    let mut corrupted = hex.chars().collect::<Vec<_>>();
    corrupted[0] = if corrupted[0] == '0' { '1' } else { '0' };
    *value = serde_json::Value::String(corrupted.into_iter().collect());

    tx.execute(
        "UPDATE pipeline_bundle_packages SET package = $3
         WHERE tenant_id = $1 AND bundle_id = $2",
        &[&tenant_id, &bundle_id, &package],
    )
    .await
    .expect("tamper the stored package");

    tx.batch_execute(
        "CREATE TRIGGER pipeline_bundle_packages_reject_update
             BEFORE UPDATE ON pipeline_bundle_packages
             FOR EACH ROW EXECUTE FUNCTION reject_pipeline_bundle_package_mutation();",
    )
    .await
    .expect("recreate the immutability trigger");

    tx.commit().await.expect("commit the tampering transaction");
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
    // Review commits through `commit_review`, which sets the approved-content
    // columns together with `approved_revision_id` (decision D7, see
    // `pipeline_runs_approved_content_shape`); there is no generic phase
    // commit.
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

    // M13: the isolation comes from forced RLS, not from the store's own
    // tenant predicate. A raw SELECT with no tenant predicate at all, in a
    // transaction scoped to the other tenant, sees no row for the outcome;
    // the same statement scoped to the owning tenant sees it.
    for (scope, expected) in [(&other_tenant, 0_i64), (&tenant, 1_i64)] {
        let tx = client.transaction().await.expect("tx for raw select");
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[scope],
        )
        .await
        .expect("set tenant for raw select");
        let visible: i64 = tx
            .query_one(
                "SELECT COUNT(*) FROM phase_outcomes WHERE outcome_id = $1",
                &[&outcome_id],
            )
            .await
            .expect("raw select without a tenant predicate")
            .get(0);
        tx.commit().await.expect("commit raw select");
        assert_eq!(
            visible,
            expected,
            "rows visible to a raw SELECT scoped to the {} tenant",
            if expected == 0 { "other" } else { "owning" }
        );
    }
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

/// Moves a run's clock back by `by`: `phase_started_at` and
/// `next_attempt_at` both move `by` into the past, as if that much time had
/// passed since the last transient retry was scheduled. A time shortcut in
/// the test, never a processor call.
async fn age_run(backend: &PgBackend, tenant_id: &str, run_id: uuid::Uuid, by: chrono::Duration) {
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .unwrap();
    let milliseconds = by.num_milliseconds();
    tx.execute(
        "UPDATE pipeline_runs
            SET phase_started_at = phase_started_at - ($3::bigint * INTERVAL '1 millisecond'),
                next_attempt_at = next_attempt_at - ($3::bigint * INTERVAL '1 millisecond')
          WHERE tenant_id = $1 AND run_id = $2",
        &[&tenant_id, &run_id, &milliseconds],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// FR3 (I2): `mark_transient_retry` schedules the next attempt after the
/// time the run has spent in its current phase, clamped to [1 s, 1 h]. With
/// no counter, the delay doubles on each retry (the next retry happens that
/// much later, so the phase is twice as old) and caps at one retry per hour.
#[tokio::test]
async fn transient_retry_backoff_doubles_from_the_phase_start_and_caps_at_one_hour() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let store = PgPipelineStore::new(backend.clone());
    let tenant = format!("backoff-{}", uuid::Uuid::new_v4());
    let run = seed_run(&backend, &tenant, uuid::Uuid::new_v4()).await;

    // A phase that started just now waits the one-second floor.
    let claimed = store
        .claim_run(&tenant, run.run_id, chrono::Duration::seconds(30))
        .await
        .unwrap()
        .unwrap();
    let first = store
        .mark_transient_retry(&claimed, "embedder_unavailable")
        .await
        .unwrap();
    assert_eq!(
        first.next_attempt_at - first.updated_at,
        chrono::Duration::seconds(1)
    );

    // The phase is 10 s old at the next retry; from then on each retry
    // happens when the previous delay has passed.
    age_run(&backend, &tenant, run.run_id, chrono::Duration::seconds(10)).await;
    let mut previous = chrono::Duration::zero();
    let one_hour = chrono::Duration::hours(1);
    for retry in 0..12 {
        let claimed = store
            .claim_run(&tenant, run.run_id, chrono::Duration::seconds(30))
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("retry {retry}: the run is due"));
        let released = store
            .mark_transient_retry(&claimed, "embedder_unavailable")
            .await
            .unwrap();
        assert_eq!(released.state, PipelineRunState::Retry);
        assert_eq!(released.attempt_count, 0, "retry {retry} is uncharged");
        let delay = released.next_attempt_at - released.updated_at;
        assert!(
            delay <= one_hour,
            "retry {retry}: delay {delay} exceeds one hour"
        );
        if previous < one_hour {
            assert!(
                delay > previous,
                "retry {retry}: delay {delay} did not grow past {previous}"
            );
        } else {
            assert_eq!(delay, one_hour, "retry {retry}: the delay stays capped");
        }
        previous = delay;
        age_run(&backend, &tenant, run.run_id, delay).await;
    }
    assert_eq!(previous, one_hour, "the delay reached the one-hour cap");
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

/// Like `envelope`, but with a long capture turn so the serialized envelope
/// is well over 768 bytes -- the minimal Score policy chunks the approved
/// bytes at 256 bytes each, and the Score tests below need at least three
/// chunks.
async fn large_envelope(submission_id: uuid::Uuid) -> TraceContributionEnvelope {
    let now = chrono::Utc::now();
    let long_input = "Inspect the bounded runtime fixture in detail. ".repeat(30);
    let long_response = "Every field of the fixture was reviewed. ".repeat(20);
    let raw = RawTraceContribution::from_capture_turns(
        &[RawTraceCaptureTurn {
            user_input: long_input,
            response: Some(long_response),
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

/// A config that awards both pinned instruments (`storage_rebate` 5 atomic
/// units, `trace_credit` 1,000,000 atomic units), per the resolution note on
/// test configs that award both descriptors.
fn scored_config(include_index: bool) -> PipelineBundleConfig {
    PipelineBundleConfig {
        instrument_awards: vec![
            PipelineInstrumentAwardConfig {
                instrument_id: "storage_rebate".into(),
                atomic_units: AtomicUnits::from_raw(5),
                descriptor: storage_rebate_descriptor(),
            },
            PipelineInstrumentAwardConfig {
                instrument_id: InstrumentId::trace_credit().as_str().to_string(),
                atomic_units: AtomicUnits::from_raw(1_000_000),
                descriptor: trace_credit_descriptor(),
            },
        ],
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

/// P5: like `test_service`, but takes the settlement adapters directly
/// rather than building the two recording ones itself -- for a test whose
/// adapter shape `RecordingSettlementAdapter` cannot produce (a double that
/// always returns a mismatching result).
async fn test_service_with_adapters(
    backend: Arc<PgBackend>,
    artifact_store: Arc<dyn TraceArtifactStore>,
    config: PipelineBundleConfig,
    adapters: Vec<Arc<dyn SettlementAdapter>>,
) -> Arc<PipelineService> {
    let scorer = Arc::new(ReferencePerplexityScorer::new());
    let embedder = Arc::new(ReferenceEmbedder::new());
    let package = MinimalPolicyBundle::minimal_package(&config, scorer.as_ref(), embedder.as_ref())
        .expect("build minimal bundle package");
    let index = IsolatedPipelineIndex::new();
    let registry =
        SettlementAdapterRegistry::new(adapters).expect("build settlement adapter registry");
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
    let service = PipelineServiceBuilder::new(
        backend,
        artifact_store,
        package,
        index.clone(),
        index.clone(),
        registry,
        caps,
    )
    .with_scorer(scorer)
    .with_embedder(embedder)
    .build()
    .expect("build pipeline service");
    Arc::new(service)
}

/// P5, Task 16: like `test_service_with_adapters`, but also takes the index
/// directly rather than building its own -- the only variant that takes the
/// adapters, the index, and an optional crash point together, per the
/// ruling that a crash-matrix test needing two services to share both the
/// recording adapters *and* the in-memory index (which has no other way for
/// a second service to see the first one's writes) gets one helper, not a
/// separate combination for each.
async fn test_service_with_adapters_and_index(
    backend: Arc<PgBackend>,
    artifact_store: Arc<dyn TraceArtifactStore>,
    config: PipelineBundleConfig,
    adapters: Vec<Arc<dyn SettlementAdapter>>,
    index: Arc<IsolatedPipelineIndex>,
    crash_point: Option<PipelineCrashPoint>,
) -> Arc<PipelineService> {
    let scorer = Arc::new(ReferencePerplexityScorer::new());
    let embedder = Arc::new(ReferenceEmbedder::new());
    let package = MinimalPolicyBundle::minimal_package(&config, scorer.as_ref(), embedder.as_ref())
        .expect("build minimal bundle package");
    let registry =
        SettlementAdapterRegistry::new(adapters).expect("build settlement adapter registry");
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
    Arc::new(builder.build().expect("build pipeline service"))
}

/// P5: like `test_service`, but takes the embedder directly instead of
/// building a `ReferenceEmbedder` itself -- for a service that must hold a
/// dependency other than the one an existing run's bound bundle names. Its
/// own default package names `embedder`, so `build()` still succeeds.
async fn test_service_with_embedder(
    backend: Arc<PgBackend>,
    artifact_store: Arc<dyn TraceArtifactStore>,
    config: PipelineBundleConfig,
    embedder: Arc<dyn IdentifiedEmbedder>,
) -> Arc<PipelineService> {
    let scorer = Arc::new(ReferencePerplexityScorer::new());
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
    let registry = SettlementAdapterRegistry::new(vec![
        storage_rebate as Arc<dyn SettlementAdapter>,
        trace_credit as Arc<dyn SettlementAdapter>,
    ])
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
    let service = PipelineServiceBuilder::new(
        backend,
        artifact_store,
        package,
        index.clone(),
        index.clone(),
        registry,
        caps,
    )
    .with_scorer(scorer)
    .with_embedder(embedder)
    .build()
    .expect("build pipeline service");
    Arc::new(service)
}

/// An embedder whose descriptor is chosen by the test and which counts
/// calls, mirroring the unit-test double in `versioned_pipeline_bundle.rs`
/// (P5: integration tests define their own copy of these doubles).
struct CountingEmbedder {
    descriptor: Vec<u8>,
    calls: AtomicUsize,
}

impl Embedder for CountingEmbedder {
    fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ReferenceEmbedder::new().embed(plaintext)
    }
}

impl IdentifiedEmbedder for CountingEmbedder {
    fn dependency_identity(&self) -> &str {
        "counting_embedder_test_only"
    }

    fn model_id(&self) -> &str {
        "counting-embedder-v1"
    }

    fn content_descriptor(&self) -> Vec<u8> {
        self.descriptor.clone()
    }
}

/// P5: an embedder that fails its first 7 `embed` calls and then delegates
/// to the reference embedder, for `transient_policy_errors_do_not_exhaust_the_trace`.
/// `FixedScorePolicy` chunks the reviewed artifact and aborts a Score
/// attempt on the first `embed` error, so a failing attempt never reaches a
/// second chunk -- every one of the first 7 failing calls is therefore the
/// sole call of its own attempt, and counting raw `embed` calls counts
/// attempts.
struct FlakyEmbedder {
    calls: AtomicUsize,
}

impl Embedder for FlakyEmbedder {
    fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call < 7 {
            anyhow::bail!("embedder dependency outage (test double)");
        }
        ReferenceEmbedder::new().embed(plaintext)
    }
}

impl IdentifiedEmbedder for FlakyEmbedder {
    fn dependency_identity(&self) -> &str {
        "flaky_embedder_test_only"
    }

    fn model_id(&self) -> &str {
        "flaky-embedder-v1"
    }

    fn content_descriptor(&self) -> Vec<u8> {
        b"flaky-embedder-test-descriptor-v1".to_vec()
    }
}

/// P5's mismatching adapter double: always returns a well-formed but wrong
/// result reference, regardless of what the request expects. Proves Settle
/// fails the row closed on D4's binding check -- the expected result comes
/// from the persisted selection, never trusted from whatever the adapter
/// hands back -- rather than accepting a plausible-looking but different
/// result.
struct MismatchingSettlementAdapter {
    instrument_id: InstrumentId,
}

impl SettlementAdapter for MismatchingSettlementAdapter {
    fn instrument_id(&self) -> &InstrumentId {
        &self.instrument_id
    }

    fn adapter_identity(&self) -> &str {
        "mismatching_test_only"
    }

    fn payout_rail(&self) -> &str {
        "none"
    }

    fn settle(&self, _request: &SettlementRequest) -> anyhow::Result<String> {
        Ok(format!("sha256:{}", "f".repeat(64)))
    }
}

/// What `InterruptingCreditAdapter` does to the database during its first
/// `settle` call, after the delegated call returns.
enum CreditInterruption {
    /// Expires the calling run's lease: "the adapter call outlived the
    /// lease" (FR2, Failure 1).
    ExpireLease,
    /// Places an unreleased hold on the account: a hold that lands between
    /// the pre-dispatch hold check and the credit transaction's re-check.
    PlaceHold(TraceCreditHoldWrite),
}

/// A Trace Credit adapter that delegates to a recording adapter (so a
/// repeated operation is still one logical effect) and, on its first call
/// only, applies one `CreditInterruption` in the database before it returns.
/// `settle` is synchronous, so the database write runs through
/// `block_in_place`; a test using it runs on a multi-thread runtime.
struct InterruptingCreditAdapter {
    inner: Arc<RecordingSettlementAdapter>,
    backend: Arc<PgBackend>,
    tenant_id: String,
    interruption: CreditInterruption,
    armed: std::sync::atomic::AtomicBool,
}

impl SettlementAdapter for InterruptingCreditAdapter {
    fn instrument_id(&self) -> &InstrumentId {
        self.inner.instrument_id()
    }

    fn adapter_identity(&self) -> &str {
        "interrupting_credit_test_only"
    }

    fn payout_rail(&self) -> &str {
        "none"
    }

    fn settle(&self, request: &SettlementRequest) -> anyhow::Result<String> {
        let result = self.inner.settle(request)?;
        if self.armed.swap(false, Ordering::SeqCst) {
            let backend = self.backend.clone();
            let tenant_id = self.tenant_id.clone();
            let run_id = request.run_id;
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    match &self.interruption {
                        CreditInterruption::ExpireLease => {
                            let mut client = backend.trace_pool_for_test().get().await.unwrap();
                            let tx = tenant_tx(&mut client, &tenant_id).await;
                            tx.execute(
                                "UPDATE pipeline_runs
                                    SET lease_expires_at = NOW() - INTERVAL '1 second'
                                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'",
                                &[&tenant_id, &run_id],
                            )
                            .await
                            .unwrap();
                            tx.commit().await.unwrap();
                        }
                        CreditInterruption::PlaceHold(hold) => {
                            backend
                                .upsert_trace_credit_hold(hold.clone())
                                .await
                                .unwrap();
                        }
                    }
                })
            });
        }
        Ok(result)
    }
}

/// The hold the tests place on the receipt helper's fixed principal
/// (`receipt`'s `actor_principal_ref`, which becomes the submission's
/// `auth_principal_ref` and so the credit account), released or not.
fn credit_hold(
    tenant_id: &str,
    hold_id: uuid::Uuid,
    released_at: Option<chrono::DateTime<chrono::Utc>>,
) -> TraceCreditHoldWrite {
    let account_ref = "principal_sha256:test".to_string();
    TraceCreditHoldWrite {
        tenant_id: tenant_id.to_string(),
        hold_id,
        credit_account_ref: account_ref.clone(),
        credit_account_hash: credit_account_hash(&account_ref),
        reason: TraceCreditHoldReason::PolicyMigration,
        reason_hash: credit_account_hash("pipeline-hold"),
        actor_principal_ref: account_ref,
        released_at,
    }
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

/// `SELECT COUNT(*)` over `trace_credit_ledger` for one pipeline run, in its
/// own tenant-scoped transaction.
async fn count_credit_ledger_rows_for_run(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    run_id: uuid::Uuid,
) -> i64 {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for count_credit_ledger_rows_for_run");
    let tx = tenant_tx(&mut client, tenant_id).await;
    let count: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM trace_credit_ledger
              WHERE tenant_id = $1 AND pipeline_run_id = $2",
            &[&tenant_id, &run_id],
        )
        .await
        .expect("count credit ledger rows")
        .get(0);
    tx.commit()
        .await
        .expect("commit count_credit_ledger_rows_for_run");
    count
}

/// The `status` column of `trace_credit_settlement_batches` for
/// `settlement_batch_id`, or `None` if no row exists.
async fn settlement_batch_status(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    settlement_batch_id: uuid::Uuid,
) -> Option<String> {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for settlement_batch_status");
    let tx = tenant_tx(&mut client, tenant_id).await;
    let row = tx
        .query_opt(
            "SELECT status FROM trace_credit_settlement_batches
              WHERE tenant_id = $1 AND settlement_batch_id = $2",
            &[&tenant_id, &settlement_batch_id],
        )
        .await
        .expect("query settlement batch status");
    tx.commit().await.expect("commit settlement_batch_status");
    row.map(|row| row.get::<_, String>("status"))
}

/// The `settlement_state` of one `trace_credit_ledger` event, or `None` if
/// no row exists.
async fn credit_event_state(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    credit_event_id: uuid::Uuid,
) -> Option<String> {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for credit_event_state");
    let tx = tenant_tx(&mut client, tenant_id).await;
    let row = tx
        .query_opt(
            "SELECT settlement_state FROM trace_credit_ledger
              WHERE tenant_id = $1 AND credit_event_id = $2",
            &[&tenant_id, &credit_event_id],
        )
        .await
        .expect("query credit event state");
    tx.commit().await.expect("commit credit_event_state");
    row.map(|row| row.get::<_, String>("settlement_state"))
}

/// Every finalized `trace_credit_settlement_batches` row whose source list
/// carries `credit_event_id`, as `(settlement_batch_id, instrument_id)`.
async fn finalized_batches_carrying(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    credit_event_id: uuid::Uuid,
) -> Vec<(uuid::Uuid, Option<String>)> {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for finalized_batches_carrying");
    let tx = tenant_tx(&mut client, tenant_id).await;
    let rows = tx
        .query(
            "SELECT settlement_batch_id, instrument_id
               FROM trace_credit_settlement_batches
              WHERE tenant_id = $1 AND status = 'finalized'
                AND $2 = ANY(source_credit_event_ids)",
            &[&tenant_id, &credit_event_id],
        )
        .await
        .expect("query batches carrying the event");
    tx.commit()
        .await
        .expect("commit finalized_batches_carrying");
    rows.iter()
        .map(|row| (row.get("settlement_batch_id"), row.get("instrument_id")))
        .collect()
}

/// Ruling FR2's per-run credit invariant: the run's Trace Credit leg is
/// complete, it wrote exactly one ledger row, that event is final, exactly
/// one finalized batch carries it, the batch's `instrument_id` is set, and
/// the settlement row points at that batch.
async fn assert_credit_settled_once(
    backend: &Arc<PgBackend>,
    service: &PipelineService,
    tenant_id: &str,
    run_id: uuid::Uuid,
    context: &str,
) {
    assert_eq!(
        count_credit_ledger_rows_for_run(backend, tenant_id, run_id).await,
        1,
        "exactly one credit ledger row for the run ({context})"
    );
    let credit_row = service
        .store()
        .list_settlements(tenant_id, run_id)
        .await
        .unwrap()
        .into_iter()
        .find(|settlement| settlement.instrument_id == InstrumentId::trace_credit().as_str())
        .unwrap_or_else(|| panic!("trace_credit row present ({context})"));
    assert_eq!(credit_row.operation_state, "complete", "{context}");
    let event_id = credit_row
        .credit_event_id
        .unwrap_or_else(|| panic!("the completed leg records its credit event ({context})"));
    assert_eq!(
        credit_event_state(backend, tenant_id, event_id)
            .await
            .as_deref(),
        Some("final"),
        "the run's credit event is final ({context})"
    );
    let batches = finalized_batches_carrying(backend, tenant_id, event_id).await;
    assert_eq!(
        batches.len(),
        1,
        "exactly one finalized batch carries the event ({context})"
    );
    assert_eq!(
        batches[0].1.as_deref(),
        Some(InstrumentId::trace_credit().as_str()),
        "the batch's instrument_id is set ({context})"
    );
    assert_eq!(
        credit_row.settlement_batch_id,
        Some(batches[0].0),
        "the settlement row points at the batch that carries its event ({context})"
    );
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
/// I6: the receipt derives the submission's retention policy and expiry on
/// the server, as the legacy path does (`retention_policy_for_trace` over
/// the envelope's allowed uses and consent, and `received_at +
/// max_age_days`), and never stores the retention policy the envelope
/// itself asserts.
#[tokio::test]
async fn receipt_derives_retention_and_expiry_on_the_server() {
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
    let tenant = format!("retention-{}", uuid::Uuid::new_v4());
    let mut env = envelope(uuid::Uuid::new_v4()).await;
    env.trace_card.retention_policy = "client_asserted_keep_forever".to_string();
    let expected = retention_policy_for_trace(&env);
    assert_ne!(expected.name, env.trace_card.retention_policy);
    let max_age_days = expected
        .max_age_days
        .expect("the fixture's allowed uses carry a bounded retention policy");
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(_) = service
        .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .unwrap()
    else {
        panic!("receipt creates a run")
    };

    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &tenant).await;
    let row = tx
        .query_one(
            "SELECT retention_policy_id, received_at, expires_at
               FROM trace_submissions
              WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant, &env.submission_id],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let retention_policy_id: String = row.get("retention_policy_id");
    let received_at: chrono::DateTime<chrono::Utc> = row.get("received_at");
    let expires_at: Option<chrono::DateTime<chrono::Utc>> = row.get("expires_at");
    assert_eq!(retention_policy_id, expected.name);
    assert_eq!(
        expires_at,
        Some(received_at + chrono::Duration::days(i64::from(max_age_days)))
    );
}

/// Reads a submission's stored `status` in a tenant-scoped transaction.
async fn submission_status(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    submission_id: uuid::Uuid,
) -> String {
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant_id).await;
    let status: String = tx
        .query_one(
            "SELECT status FROM trace_submissions WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant_id, &submission_id],
        )
        .await
        .unwrap()
        .get(0);
    tx.commit().await.unwrap();
    status
}

/// Admission Reject (a high-risk envelope): the receipt stores the
/// submission as `rejected`, records the Reject decision, and completes the
/// run at Admission -- there is no Review work to claim.
#[tokio::test]
async fn a_rejected_receipt_records_the_decision_and_creates_no_review_work() {
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
    let tenant = format!("admission-reject-{}", uuid::Uuid::new_v4());
    let mut env = envelope(uuid::Uuid::new_v4()).await;
    env.privacy.residual_pii_risk = ResidualPiiRisk::High;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) = service
        .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .unwrap()
    else {
        panic!("a rejected receipt still creates its run record")
    };
    assert_eq!(created.admission_decision, "reject");
    assert_eq!(created.state, PipelineRunState::Complete);
    assert_eq!(created.next_phase, None);
    assert_eq!(
        submission_status(&backend, &tenant, env.submission_id).await,
        "rejected"
    );

    let outcomes = service
        .store()
        .list_outcomes(&tenant, created.run_id)
        .await
        .unwrap();
    assert_eq!(outcomes.len(), 1, "only the Admission outcome");
    assert_eq!(outcomes[0].phase, Phase::Admission);
    let decision: AdmissionDecision = serde_json::from_value(outcomes[0].decision.clone()).unwrap();
    match decision {
        AdmissionDecision::Reject { reason } => {
            assert_eq!(reason.as_str(), "privacy_risk_rejected");
        }
        other => panic!("expected an Admission Reject, got {other:?}"),
    }

    // No Review work: nothing is claimable, for this run or the tenant.
    assert!(
        service
            .process_run(&tenant, created.run_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(service.process_one(&tenant).await.unwrap().is_none());
}

/// Admission Quarantine (a Medium-risk envelope): the receipt stores the
/// submission as `quarantined` and records the Quarantine decision; Review
/// cannot resolve it without a human assessment (PR 3), so each Review
/// attempt is the uncharged `review_assessment_required` suspension, with
/// the phase-age backoff -- never a charged retry and never a hot loop.
#[tokio::test]
async fn a_quarantined_receipt_waits_for_review_with_backoff() {
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
    let tenant = format!("admission-quarantine-{}", uuid::Uuid::new_v4());
    let mut env = envelope(uuid::Uuid::new_v4()).await;
    env.privacy.residual_pii_risk = ResidualPiiRisk::Medium;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) = service
        .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .unwrap()
    else {
        panic!("receipt creates a run")
    };
    assert_eq!(created.admission_decision, "quarantine");
    assert_eq!(created.state, PipelineRunState::Pending);
    assert_eq!(created.next_phase, Some(Phase::Review));
    assert_eq!(
        submission_status(&backend, &tenant, env.submission_id).await,
        "quarantined"
    );
    let outcomes = service
        .store()
        .list_outcomes(&tenant, created.run_id)
        .await
        .unwrap();
    let decision: AdmissionDecision = serde_json::from_value(outcomes[0].decision.clone()).unwrap();
    match decision {
        AdmissionDecision::Quarantine { reason } => {
            assert_eq!(reason.as_str(), "privacy_review_required");
        }
        other => panic!("expected an Admission Quarantine, got {other:?}"),
    }

    // The first Review attempt waits the one-second floor, uncharged.
    let waited = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Review runs and waits for a human assessment");
    assert_eq!(waited.state, PipelineRunState::Retry);
    assert_eq!(waited.next_phase, Some(Phase::Review));
    assert_eq!(
        waited.last_error_label.as_deref(),
        Some("review_assessment_required")
    );
    assert_eq!(waited.attempt_count, 0);
    let first_delay = waited.next_attempt_at - waited.updated_at;
    assert!(first_delay >= chrono::Duration::seconds(1));
    assert!(
        service
            .process_run(&tenant, created.run_id)
            .await
            .unwrap()
            .is_none(),
        "the run is not claimable again before its backoff elapses"
    );

    // A minute later the phase is older, so the next wait is longer; the
    // run is still uncharged and has no Review outcome.
    age_run(
        &backend,
        &tenant,
        created.run_id,
        chrono::Duration::seconds(60),
    )
    .await;
    let waited_again = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Review runs again once the backoff elapses");
    assert_eq!(waited_again.state, PipelineRunState::Retry);
    assert_eq!(waited_again.attempt_count, 0);
    let second_delay = waited_again.next_attempt_at - waited_again.updated_at;
    assert!(second_delay > first_delay);
    assert!(second_delay <= chrono::Duration::hours(1));
    assert!(
        !service
            .store()
            .list_outcomes(&tenant, created.run_id)
            .await
            .unwrap()
            .iter()
            .any(|outcome| outcome.phase == Phase::Review),
        "no Review outcome while the quarantine is unresolved"
    );
}

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
/// once the run retries, and the retry records the same deterministic
/// object ref id. The object key is the retry's own (ruling FR1: it carries
/// the claim's lease token), so the crashed attempt's object is left
/// unreferenced rather than overwritten.
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

    // The object ref id is derived from the run id alone, so both the
    // crashed attempt and the retry compute the same one. The object key is
    // per claim (FR1): the retry wrote its own object and never touched the
    // crashed attempt's.
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

/// FR1 (C1): a worker whose lease expired during policy work may still
/// write its artifacts after another worker committed the phase. Every
/// phase artifact is keyed by the claim's own lease token, so the stale
/// write lands under a key no committed row refers to: the committed
/// approved content and index command still read and verify, and the run
/// completes.
#[tokio::test]
async fn a_stale_worker_cannot_overwrite_committed_artifacts() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(true),
        None,
    )
    .await;
    let tenant = format!("stale-writer-{}", uuid::Uuid::new_v4());
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

    // Worker A claims the run and stalls in policy work until its lease
    // expires (a direct UPDATE: a time shortcut, not a processor call).
    let stale_claim = service
        .store()
        .claim_run(&tenant, created.run_id, chrono::Duration::seconds(30))
        .await
        .unwrap()
        .expect("worker A claims the run");
    let stale_token = stale_claim.lease_token.expect("a claim carries a lease");
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &tenant).await;
    tx.execute(
        "UPDATE pipeline_runs SET lease_expires_at = NOW() - INTERVAL '1 second'
         WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'",
        &[&tenant, &created.run_id],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // Worker B reclaims the run and commits Review and Score.
    let reviewed = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("worker B commits Review");
    assert_eq!(reviewed.next_phase, Some(Phase::Score));
    let scored = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("worker B commits Score");
    assert_eq!(scored.next_phase, Some(Phase::Settle));

    // Worker A wakes up and writes every phase artifact under the object id
    // its own claim uses. The ciphertext differs from the committed one
    // whatever the plaintext is (a fresh salt and nonce per write).
    let stale_store = artifact_store(&dir);
    let tenant_ref = pipeline_tenant_storage_ref(&tenant);
    let stale_bytes = serde_json::to_vec(&serde_json::json!({"stale_worker": true})).unwrap();
    for (artifact, kind) in [
        ("approved", TraceArtifactKind::ContributionEnvelope),
        ("index-command", TraceArtifactKind::VectorPayload),
        ("score-neighbors", TraceArtifactKind::VectorPayload),
    ] {
        stale_store
            .put_serialized_json(
                tenant_ref.as_str(),
                kind,
                &pipeline_attempt_object_id(artifact, created.run_id, stale_token),
                &stale_bytes,
            )
            .expect("the stale write itself succeeds");
    }

    // The committed objects still read and verify.
    let approved = service
        .load_approved_bytes(&scored)
        .await
        .expect("the committed approved content still verifies");
    assert_eq!(
        dependency_content_hash(&approved),
        scored.approved_content_hash.clone().unwrap()
    );
    let score_outcome = service
        .store()
        .list_outcomes(&tenant, scored.run_id)
        .await
        .unwrap()
        .into_iter()
        .find(|outcome| outcome.phase == Phase::Score)
        .expect("Score outcome recorded");
    let evidence: ScoreEvidence = serde_json::from_value(score_outcome.evidence).unwrap();
    service
        .load_index_command(&scored, &evidence)
        .await
        .expect("the committed index command still verifies")
        .expect("Score proposed a command");

    // And the run completes.
    let settled = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Settle runs");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(settled.index_write_state, "complete");
}

/// Score stores the exact index command it proposed, and seeds one pending
/// settlement operation per award, in the Score commit (decision D5). The
/// stored command survives a restart bit-for-bit, and keeps every chunk of a
/// multi-chunk envelope.
#[tokio::test]
async fn score_commit_seeds_one_operation_per_award_and_keeps_every_chunk() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        scored_config(true),
        None,
    )
    .await;
    let tenant = format!("score-commit-{}", uuid::Uuid::new_v4());
    let env = large_envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) = service
        .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .unwrap()
    else {
        panic!("receipt creates a run")
    };

    let reviewed = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Review runs");
    assert_eq!(reviewed.next_phase, Some(Phase::Score));

    let scored = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Score runs");

    // One state, all its facts together: next_phase advanced, the command
    // reference and hash are set, and Settle has not yet decided membership.
    assert_eq!(scored.next_phase, Some(Phase::Settle));
    assert!(scored.index_command_ref.is_some());
    assert!(scored.index_command_hash.is_some());
    assert_eq!(scored.index_membership, "undecided");

    let settlements = service
        .store()
        .list_settlements(&tenant, scored.run_id)
        .await
        .unwrap();
    assert_eq!(settlements.len(), 2);
    let storage_award = InstrumentAward::new(
        InstrumentId::new("storage_rebate").unwrap(),
        AtomicUnits::from_raw(5),
    )
    .unwrap();
    let credit_award = InstrumentAward::new(
        InstrumentId::trace_credit(),
        AtomicUnits::from_raw(1_000_000),
    )
    .unwrap();
    for settlement in &settlements {
        assert_eq!(settlement.operation_state, "pending");
        assert!(settlement.result_ref_hash.is_none());
        // The test adapters (`test_service`) use payout rail "none".
        assert_eq!(settlement.payout_state, "disabled");
        let expected_award = if settlement.instrument_id == "storage_rebate" {
            &storage_award
        } else {
            &credit_award
        };
        assert_eq!(
            settlement.operation_ref_hash,
            pipeline_operation_ref(scored.run_id, expected_award)
        );
    }

    let outcomes = service
        .store()
        .list_outcomes(&tenant, scored.run_id)
        .await
        .unwrap();
    let score_outcome = outcomes
        .into_iter()
        .find(|outcome| outcome.phase == Phase::Score)
        .expect("Score outcome recorded");
    let evidence: ScoreEvidence = serde_json::from_value(score_outcome.evidence).unwrap();

    let command = service
        .load_index_command(&scored, &evidence)
        .await
        .unwrap()
        .expect("Score proposed a command");
    assert_eq!(
        command.content_hash().unwrap(),
        evidence.embedding_artifact_hash.clone().unwrap()
    );
    let chunks: Vec<u32> = command.entries().iter().map(|entry| entry.chunk).collect();
    assert!(
        chunks.len() >= 3,
        "expected at least three chunks, got {}",
        chunks.len()
    );
    assert_eq!(chunks, (0..chunks.len() as u32).collect::<Vec<_>>());

    // Build a NEW service over the same database and artifact root (a
    // restart), then load the command again from durable state alone.
    let (restarted, _, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        scored_config(true),
        None,
    )
    .await;
    let reloaded_run = restarted
        .store()
        .get_run(&tenant, scored.run_id)
        .await
        .unwrap()
        .expect("run still exists after restart");
    let reloaded_command = restarted
        .load_index_command(&reloaded_run, &evidence)
        .await
        .unwrap()
        .expect("the retained command reloads");
    assert_eq!(
        command, reloaded_command,
        "reloaded command must be bit-for-bit equal"
    );
}

/// A run with no pinned awards, and indexing disabled, still advances to
/// Settle: zero settlement rows and no command reference.
#[tokio::test]
async fn empty_awards_still_continue_to_settle() {
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
    let tenant = format!("score-empty-{}", uuid::Uuid::new_v4());
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

    service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Review runs");
    let scored = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Score runs");

    assert_eq!(scored.next_phase, Some(Phase::Settle));
    assert!(scored.index_command_ref.is_none());
    assert!(scored.index_command_hash.is_none());
    let settlements = service
        .store()
        .list_settlements(&tenant, scored.run_id)
        .await
        .unwrap();
    assert!(settlements.is_empty());
}

/// Review focus item 2's crash test, applied to Score: a crash between the
/// index command's artifact write and the database commit must leave
/// exactly one Score outcome, one command reference, and one settlement row
/// per award once the run retries -- the retry stores the command under its
/// own claim's key (FR1) and commits once, never twice.
#[tokio::test]
async fn score_crash_after_command_storage_keeps_one_command() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        scored_config(true),
        Some(PipelineCrashPoint::AfterScoreArtifactStorage),
    )
    .await;
    let tenant = format!("score-crash-{}", uuid::Uuid::new_v4());
    let env = large_envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) = service
        .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .unwrap()
    else {
        panic!("receipt creates a run")
    };

    service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Review runs");

    let crashed = service.process_run(&tenant, created.run_id).await;
    let error = crashed.expect_err("the injected crash must propagate as an error");
    assert_eq!(error.to_string(), INJECTED_PIPELINE_CRASH);

    // Expire the lease: a direct UPDATE, a time shortcut in the test, not a
    // processor call -- the crashed attempt never called `mark_retry` or
    // `mark_failed` (the injected crash propagates unchanged), so the run is
    // otherwise stuck `leased` until its lease naturally expires.
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

    let scored = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("the retry claims and completes Score");
    assert_eq!(scored.next_phase, Some(Phase::Settle));
    assert!(
        scored.index_command_ref.is_some(),
        "the retry stores one command reference"
    );

    let outcomes = service
        .store()
        .list_outcomes(&tenant, scored.run_id)
        .await
        .unwrap();
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome.phase == Phase::Score)
            .count(),
        1,
        "exactly one Score outcome after the crash and its retry"
    );

    let settlements = service
        .store()
        .list_settlements(&tenant, scored.run_id)
        .await
        .unwrap();
    assert_eq!(settlements.len(), 2, "two settlement rows, one per award");
}

/// Drives a fresh submission through Review and Score under `service`,
/// returning the Score-completed run (`next_phase = Settle`) and its Score
/// evidence. Shared by the Settle tests below, all of which need a run that
/// has already reached Settle with a real, stored index command.
async fn run_to_settle_ready(
    service: &PipelineService,
    tenant: &str,
) -> (PipelineRunRecord, ScoreEvidence) {
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) = service
        .submit(receipt(tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .unwrap()
    else {
        panic!("receipt creates a run")
    };
    service
        .process_run(tenant, created.run_id)
        .await
        .unwrap()
        .expect("Review runs");
    let scored = service
        .process_run(tenant, created.run_id)
        .await
        .unwrap()
        .expect("Score runs");
    assert_eq!(scored.next_phase, Some(Phase::Settle));
    let outcomes = service
        .store()
        .list_outcomes(tenant, scored.run_id)
        .await
        .unwrap();
    let score_outcome = outcomes
        .into_iter()
        .find(|outcome| outcome.phase == Phase::Score)
        .expect("Score outcome recorded");
    let evidence: ScoreEvidence = serde_json::from_value(score_outcome.evidence).unwrap();
    (scored, evidence)
}

/// The on-disk path `LocalEncryptedTraceArtifactStore` writes an object key
/// under (its private layout: `root/tenants/{sha256(tenant_storage_ref)}/
/// artifacts/{object_key}.json`). Used only to delete or corrupt a stored
/// artifact directly, to exercise Settle's fail-closed path around a
/// binding failure (review focus item 3).
fn artifact_file_path(
    root: &std::path::Path,
    tenant_storage_ref: &str,
    object_key: &str,
) -> std::path::PathBuf {
    let tenant_hash = hex::encode(Sha256::digest(tenant_storage_ref.as_bytes()));
    root.join("tenants")
        .join(tenant_hash)
        .join("artifacts")
        .join(format!("{object_key}.json"))
}

/// Review focus item 3 (part 1): the live index changed after Score (an
/// unrelated entry for the same tenant), and Settle must still apply
/// exactly the stored command -- never re-query the reader -- so the extra
/// entry cannot perturb its outcome.
#[tokio::test]
async fn settle_writes_the_stored_command_without_requerying_the_live_index() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, index, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(true),
        None,
    )
    .await;
    let tenant = format!("settle-included-{}", uuid::Uuid::new_v4());
    let (run, evidence) = run_to_settle_ready(&service, &tenant).await;

    // Before Settle: the live index changes for the same tenant. Settle
    // reads the stored command bytes only (ruling P1); it never calls
    // `index_reader.nearest`/`snapshot` again, so this unrelated entry must
    // not affect the result.
    let tenant_ref = pipeline_tenant_storage_ref(&tenant);
    let unrelated_key = IndexEntryKey {
        tenant_storage_ref: tenant_ref.as_str().to_string(),
        index_id: MINIMAL_INDEX_ID.to_string(),
        revision_id: uuid::Uuid::new_v4(),
        projection_id: MINIMAL_PROJECTION_ID.to_string(),
        model_id: "unrelated-test-model".to_string(),
        chunk: 0,
    };
    index
        .upsert(
            &unrelated_key,
            &[0.25_f32; 4],
            &dependency_content_hash(b"unrelated-entry"),
        )
        .unwrap();

    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle runs");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(settled.next_phase, None);
    assert_eq!(settled.index_membership, "included");
    assert_eq!(settled.index_write_state, "complete");
    assert_eq!(service.settle_evaluations(), 1);

    let command = service
        .load_index_command(&settled, &evidence)
        .await
        .unwrap()
        .expect("the stored command is retained");
    for entry in command.entries() {
        let key = command.entry_key(&tenant_ref, entry);
        assert_eq!(
            index.upsert(&key, &entry.embedding, &entry.content_hash),
            Ok(IndexUpsertResult::Unchanged),
            "the index already holds this exact chunk with its stored embedding"
        );
    }

    let outcomes = service
        .store()
        .list_outcomes(&tenant, settled.run_id)
        .await
        .unwrap();
    let settle_outcome = outcomes
        .into_iter()
        .find(|outcome| outcome.phase == Phase::Settle)
        .expect("Settle outcome recorded");
    let decision: SettleDecision = serde_json::from_value(settle_outcome.decision).unwrap();
    match decision.index_membership {
        IndexMembershipDecision::Include {
            command_hash,
            entry_count,
        } => {
            assert_eq!(command_hash, command.content_hash().unwrap());
            assert_eq!(entry_count as usize, command.entries().len());
        }
        IndexMembershipDecision::Exclude { .. } => panic!("expected an Include decision"),
    }
}

/// Review focus item 3 (part 2): an index write that cannot complete
/// (`IndexWriteError::Failed`/`Uncertain`) puts the run in retry under the
/// safe label `index_unavailable` without recording a Settle outcome, and
/// the retry reuses the selection already persisted -- the policy does not
/// run a second time.
#[tokio::test]
async fn settle_retry_reuses_the_persisted_selection() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, index, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(true),
        None,
    )
    .await;
    let tenant = format!("settle-retry-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;

    index.set_fault(IndexFault::FailBeforeApply);
    let retried = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("the Settle attempt retries rather than erroring out");
    assert_eq!(retried.state, PipelineRunState::Retry);
    assert_eq!(
        retried.last_error_label.as_deref(),
        Some(PIPELINE_INDEX_UNAVAILABLE_LABEL)
    );
    assert!(
        retried.settle_selection_hash.is_some(),
        "the selection was persisted before dispatch was attempted"
    );
    let outcomes_after_retry = service
        .store()
        .list_outcomes(&tenant, run.run_id)
        .await
        .unwrap();
    assert!(
        !outcomes_after_retry
            .iter()
            .any(|outcome| outcome.phase == Phase::Settle),
        "no Settle outcome exists after a retry"
    );
    let persisted_selection = service
        .store()
        .load_settle_selection(&retried)
        .await
        .unwrap()
        .expect("the selection is durable across the retry");

    // The fault is one-shot (it clears itself after the one failed call),
    // but clear it explicitly so this test does not depend on that detail.
    index.set_fault(IndexFault::None);
    force_due(&backend, &tenant, run.run_id).await;

    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("the retry completes Settle");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(
        service.settle_evaluations(),
        1,
        "the policy did not run again on retry"
    );

    let outcomes = service
        .store()
        .list_outcomes(&tenant, run.run_id)
        .await
        .unwrap();
    let settle_outcome = outcomes
        .into_iter()
        .find(|outcome| outcome.phase == Phase::Settle)
        .expect("Settle outcome recorded after the retry completes");
    let persisted_decision: SettleDecision =
        serde_json::from_value(persisted_selection.decision).unwrap();
    let committed_decision: SettleDecision =
        serde_json::from_value(settle_outcome.decision).unwrap();
    assert_eq!(committed_decision, persisted_decision);
}

/// FR3 (I1): a Settle index outage (`IndexWriteError::Failed`/`Uncertain`)
/// is a dependency failure like Score's `index_unavailable`, not the
/// trace's fault. An outage that lasts more retries than the run's whole
/// attempt budget leaves the run waiting in retry, uncharged, and the run
/// completes once the index is back.
#[tokio::test]
async fn a_settle_index_outage_longer_than_the_attempt_budget_does_not_fail_the_run() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, index, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(true),
        None,
    )
    .await;
    let tenant = format!("settle-index-outage-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;

    for attempt in 1..=run.max_attempts + 2 {
        index.set_fault(IndexFault::FailBeforeApply);
        force_due(&backend, &tenant, run.run_id).await;
        let retried = service
            .process_run(&tenant, run.run_id)
            .await
            .unwrap()
            .expect("the Settle attempt waits for the index");
        assert_eq!(retried.state, PipelineRunState::Retry, "attempt {attempt}");
        assert_eq!(
            retried.last_error_label.as_deref(),
            Some(PIPELINE_INDEX_UNAVAILABLE_LABEL),
            "attempt {attempt}"
        );
        assert_eq!(
            retried.attempt_count, run.attempt_count,
            "attempt {attempt}: an index outage never charges the run"
        );
    }

    index.set_fault(IndexFault::None);
    force_due(&backend, &tenant, run.run_id).await;
    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle completes once the index is back");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(settled.index_write_state, "complete");
    assert_eq!(service.settle_evaluations(), 1);
}

/// FR3: a service that holds no settlement adapter for an instrument a run
/// awards cannot seed or dispatch that leg. That is a deployment gap, not
/// the trace's fault: at Score and at Settle the run waits in retry under
/// `settlement_adapter_missing` without being charged, and a service that
/// holds the adapter completes it.
#[tokio::test]
async fn a_missing_settlement_adapter_waits_without_charging() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let shared_artifacts = artifact_store(&dir);
    let (full, _, _) = test_service(
        backend.clone(),
        shared_artifacts.clone(),
        scored_config(false),
        None,
    )
    .await;
    // Same package (same config), but no storage_rebate adapter.
    let missing = test_service_with_adapters(
        backend.clone(),
        shared_artifacts,
        scored_config(false),
        vec![RecordingSettlementAdapter::new(
            InstrumentId::trace_credit(),
            "recording_trace_credit_test_only",
            "none",
        ) as Arc<dyn SettlementAdapter>],
    )
    .await;
    assert_eq!(full.bundle_id(), missing.bundle_id());
    let tenant = format!("settle-adapter-missing-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) = full
        .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .unwrap()
    else {
        panic!("receipt creates a run")
    };
    let reviewed = full
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Review runs");

    // Score under the service without the adapter: no settlement row can be
    // seeded, so the run waits, uncharged.
    let waited = missing
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Score waits for the adapter");
    assert_eq!(waited.state, PipelineRunState::Retry);
    assert_eq!(waited.next_phase, Some(Phase::Score));
    assert_eq!(
        waited.last_error_label.as_deref(),
        Some("settlement_adapter_missing")
    );
    assert_eq!(waited.attempt_count, reviewed.attempt_count);

    force_due(&backend, &tenant, created.run_id).await;
    let scored = full
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Score completes under the full service");
    assert_eq!(scored.next_phase, Some(Phase::Settle));

    // Settle under the service without the adapter: the leg cannot be
    // dispatched, so the run waits, uncharged.
    let waited = missing
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Settle waits for the adapter");
    assert_eq!(waited.state, PipelineRunState::Retry);
    assert_eq!(
        waited.last_error_label.as_deref(),
        Some("settlement_adapter_missing")
    );
    assert_eq!(waited.attempt_count, scored.attempt_count);

    force_due(&backend, &tenant, created.run_id).await;
    let settled = full
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Settle completes under the full service");
    assert_eq!(settled.state, PipelineRunState::Complete);
}

/// Review focus item 3 (part 3): a stored command that is missing, corrupt,
/// bound to another tenant, or bound to another run of the same tenant
/// makes Settle fail closed with the safe label `index_command_invalid`,
/// without completing the run or writing a Settle outcome.
#[tokio::test]
async fn stored_command_binding_failures_fail_closed() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };

    async fn assert_fails_closed(service: &PipelineService, tenant: &str, run_id: uuid::Uuid) {
        let processed = service
            .process_run(tenant, run_id)
            .await
            .unwrap()
            .expect("the Settle attempt runs and fails closed rather than erroring out");
        assert_ne!(processed.state, PipelineRunState::Complete);
        assert_eq!(
            processed.last_error_label.as_deref(),
            Some("index_command_invalid")
        );
        assert!(
            !service
                .store()
                .list_outcomes(tenant, run_id)
                .await
                .unwrap()
                .iter()
                .any(|outcome| outcome.phase == Phase::Settle),
            "no Settle outcome is written"
        );
    }

    // (a) delete the command file from the artifact root.
    {
        let dir = tempfile::tempdir().unwrap();
        let (service, _, _) = test_service(
            backend.clone(),
            artifact_store(&dir),
            minimal_config(true),
            None,
        )
        .await;
        let tenant = format!("settle-binding-a-{}", uuid::Uuid::new_v4());
        let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;
        let (object_key, _) = run
            .index_command_ref
            .as_deref()
            .unwrap()
            .rsplit_once('#')
            .unwrap();
        let path = artifact_file_path(
            dir.path(),
            pipeline_tenant_storage_ref(&tenant).as_str(),
            object_key,
        );
        assert!(
            path.exists(),
            "the command's ciphertext file must exist before deletion"
        );
        std::fs::remove_file(&path).unwrap();

        assert_fails_closed(&service, &tenant, run.run_id).await;
    }

    // (b) overwrite the file with other bytes.
    {
        let dir = tempfile::tempdir().unwrap();
        let (service, _, _) = test_service(
            backend.clone(),
            artifact_store(&dir),
            minimal_config(true),
            None,
        )
        .await;
        let tenant = format!("settle-binding-b-{}", uuid::Uuid::new_v4());
        let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;
        let (object_key, _) = run
            .index_command_ref
            .as_deref()
            .unwrap()
            .rsplit_once('#')
            .unwrap();
        let path = artifact_file_path(
            dir.path(),
            pipeline_tenant_storage_ref(&tenant).as_str(),
            object_key,
        );
        std::fs::write(&path, b"not a valid encrypted trace artifact").unwrap();

        assert_fails_closed(&service, &tenant, run.run_id).await;
    }

    // (c) point index_command_ref at another tenant's stored command.
    {
        let dir = tempfile::tempdir().unwrap();
        let (service, _, _) = test_service(
            backend.clone(),
            artifact_store(&dir),
            minimal_config(true),
            None,
        )
        .await;
        let tenant_a = format!("settle-binding-c-a-{}", uuid::Uuid::new_v4());
        let tenant_b = format!("settle-binding-c-b-{}", uuid::Uuid::new_v4());
        let (run_a, _) = run_to_settle_ready(&service, &tenant_a).await;
        let (run_b, _) = run_to_settle_ready(&service, &tenant_b).await;
        let foreign_ref = run_b.index_command_ref.clone().unwrap();

        // The runtime role's own tenant-scoped UPDATE is sufficient here:
        // `reject_pipeline_run_identity_mutation` does not protect
        // `index_command_ref`, and RLS's `WITH CHECK` only constrains
        // `tenant_id`, which this UPDATE does not touch.
        let mut client = backend.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, &tenant_a).await;
        tx.execute(
            "UPDATE pipeline_runs SET index_command_ref = $3
             WHERE tenant_id = $1 AND run_id = $2",
            &[&tenant_a, &run_a.run_id, &foreign_ref],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        assert_fails_closed(&service, &tenant_a, run_a.run_id).await;
    }

    // (d) point it at another run's command of the same tenant.
    {
        let dir = tempfile::tempdir().unwrap();
        let (service, _, _) = test_service(
            backend.clone(),
            artifact_store(&dir),
            minimal_config(true),
            None,
        )
        .await;
        let tenant = format!("settle-binding-d-{}", uuid::Uuid::new_v4());
        let (run_1, _) = run_to_settle_ready(&service, &tenant).await;
        let (run_2, _) = run_to_settle_ready(&service, &tenant).await;
        let other_ref = run_2.index_command_ref.clone().unwrap();

        let mut client = backend.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, &tenant).await;
        tx.execute(
            "UPDATE pipeline_runs SET index_command_ref = $3
             WHERE tenant_id = $1 AND run_id = $2",
            &[&tenant, &run_1.run_id, &other_ref],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        assert_fails_closed(&service, &tenant, run_1.run_id).await;
    }
}

/// Review focus item 3 (part 4): a stored command entry whose key already
/// exists in the index under a different embedding is a genuine content
/// conflict (`IndexWriteError::ContentConflict`), not a silent overwrite --
/// the run fails (not retries) under the safe label `index_key_conflict`,
/// and no Settle outcome is written.
#[tokio::test]
async fn equal_key_with_different_content_fails_closed() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, index, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(true),
        None,
    )
    .await;
    let tenant = format!("settle-conflict-{}", uuid::Uuid::new_v4());
    let (run, evidence) = run_to_settle_ready(&service, &tenant).await;
    let command = service
        .load_index_command(&run, &evidence)
        .await
        .unwrap()
        .expect("Score proposed a command");
    let first_entry = command.entries().first().expect("at least one chunk");
    let tenant_ref = pipeline_tenant_storage_ref(&tenant);
    let key = command.entry_key(&tenant_ref, first_entry);

    // Pre-insert the stored command's first entry key with a different
    // embedding, before Settle ever dispatches to the index.
    index
        .upsert(
            &key,
            &vec![9.9_f32; first_entry.embedding.len()],
            &dependency_content_hash(b"conflicting-content"),
        )
        .unwrap();

    let processed = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("the Settle attempt runs and fails rather than erroring out");
    assert_eq!(processed.state, PipelineRunState::Failed);
    assert_eq!(
        processed.last_error_label.as_deref(),
        Some(PIPELINE_INDEX_CONFLICT_LABEL)
    );
    assert!(
        !service
            .store()
            .list_outcomes(&tenant, run.run_id)
            .await
            .unwrap()
            .iter()
            .any(|outcome| outcome.phase == Phase::Settle),
        "no Settle outcome is written"
    );
}

/// Inserts a `trace_withdrawals` tombstone for `submission_id` (columns
/// from `migrations/V43__trace_withdrawal.sql`), in a tenant-scoped
/// transaction on the runtime backend. This alone flips
/// `PipelineService::submission_guard`'s `operable` to `false` -- the guard
/// checks for a withdrawal row directly, regardless of
/// `trace_submissions.status`.
async fn withdraw_submission(backend: &PgBackend, tenant_id: &str, submission_id: uuid::Uuid) {
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant_id).await;
    tx.execute(
        "INSERT INTO trace_withdrawals (
            tenant_id, submission_id, withdrawn_at, prior_status, distribution_reach
         ) VALUES ($1, $2, NOW(), 'accepted', 'not_distributed')",
        &[&tenant_id, &submission_id],
    )
    .await
    .expect("insert trace_withdrawals row");
    tx.commit().await.expect("commit withdrawal insert");
}

/// Fix round 1 (review finding on Task 12): the committed Settle decision
/// and the `index_membership` column must reflect the submission-
/// operability guard, not the Settle policy's raw selection. A run whose
/// submission was withdrawn between Score and Settle must commit `Exclude
/// { reason: submission_inoperable }` and touch the index writer zero
/// times, even though the policy itself selected `Include` (guard.operable
/// was still true when the policy ran, inside `commit_settle_from_progress`
/// -- not inside the policy call itself).
#[tokio::test]
async fn withdrawal_between_score_and_settle_excludes_the_index() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, index, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(true),
        None,
    )
    .await;
    let tenant = format!("settle-withdrawn-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;

    withdraw_submission(&backend, &tenant, run.submission_id).await;

    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle runs and completes despite the withdrawal");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(settled.next_phase, None);
    assert_eq!(settled.index_membership, "excluded");
    assert_eq!(settled.index_write_state, "none");
    assert_eq!(service.settle_evaluations(), 1);

    let outcomes = service
        .store()
        .list_outcomes(&tenant, settled.run_id)
        .await
        .unwrap();
    let settle_outcome = outcomes
        .into_iter()
        .find(|outcome| outcome.phase == Phase::Settle)
        .expect("Settle outcome recorded");
    let decision: SettleDecision = serde_json::from_value(settle_outcome.decision).unwrap();
    match decision.index_membership {
        IndexMembershipDecision::Exclude { reason } => {
            assert_eq!(reason.as_str(), PIPELINE_SUBMISSION_INOPERABLE_LABEL);
        }
        IndexMembershipDecision::Include { .. } => {
            panic!("a withdrawn submission must not commit an Include decision")
        }
    }
    let evidence: SettleEvidence = serde_json::from_value(settle_outcome.evidence).unwrap();
    assert_eq!(evidence.submission_operable, Some(false));

    let tenant_ref = pipeline_tenant_storage_ref(&tenant);
    assert_eq!(
        index.entry_count(tenant_ref.as_str(), MINIMAL_INDEX_ID),
        0,
        "no entry was applied for a withdrawn submission"
    );
    assert_eq!(
        index.writer_calls(),
        0,
        "dispatch never started for a submission already inoperable at persist time"
    );
}

/// Fix round 1, second case: the guard can newly fail *between*
/// `persist_settle_selection` and dispatch -- a crash right after the
/// selection persists (leaving `index_membership = "included"`,
/// `index_write_state = "pending"`) gives real wall-clock room for a
/// withdrawal to land before the retry reaches Step 5's dispatch. The
/// retry must cancel the index write and still commit `Exclude`, not the
/// stale `Include` the persisted selection holds.
#[tokio::test]
async fn withdrawal_during_dispatch_cancels_the_index_write() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, index, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(true),
        Some(PipelineCrashPoint::AfterSettleSelection),
    )
    .await;
    let tenant = format!("settle-withdrawn-mid-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;

    let crashed = service.process_run(&tenant, run.run_id).await;
    let error = crashed.expect_err("the injected crash must propagate as an error");
    assert_eq!(error.to_string(), INJECTED_PIPELINE_CRASH);

    let after_crash = service
        .store()
        .get_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("run still exists after the crash");
    assert_eq!(
        after_crash.index_membership, "included",
        "the selection persisted an include before the crash"
    );
    assert_eq!(after_crash.index_write_state, "pending");

    // Expire the lease directly (a time shortcut, not a processor call --
    // the crashed attempt never called `mark_retry`/`mark_failed`) and
    // insert the withdrawal before the retry claims the run.
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &tenant).await;
    tx.execute(
        "UPDATE pipeline_runs SET lease_expires_at = NOW() - INTERVAL '1 second'
         WHERE tenant_id = $1 AND run_id = $2",
        &[&tenant, &run.run_id],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    withdraw_submission(&backend, &tenant, run.submission_id).await;

    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("the retry cancels dispatch and completes Settle");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(settled.index_write_state, "cancelled");
    assert_eq!(settled.index_membership, "excluded");
    assert_eq!(
        service.settle_evaluations(),
        1,
        "the policy ran once, before the crash; the retry reuses the persisted selection"
    );

    let outcomes = service
        .store()
        .list_outcomes(&tenant, run.run_id)
        .await
        .unwrap();
    let settle_outcome = outcomes
        .into_iter()
        .find(|outcome| outcome.phase == Phase::Settle)
        .expect("Settle outcome recorded");
    let decision: SettleDecision = serde_json::from_value(settle_outcome.decision).unwrap();
    match decision.index_membership {
        IndexMembershipDecision::Exclude { reason } => {
            assert_eq!(reason.as_str(), PIPELINE_SUBMISSION_INOPERABLE_LABEL);
        }
        IndexMembershipDecision::Include { .. } => {
            panic!("a submission withdrawn before dispatch must not commit an Include decision")
        }
    }
    let evidence: SettleEvidence = serde_json::from_value(settle_outcome.evidence).unwrap();
    assert_eq!(evidence.submission_operable, Some(false));

    let tenant_ref = pipeline_tenant_storage_ref(&tenant);
    assert_eq!(
        index.entry_count(tenant_ref.as_str(), MINIMAL_INDEX_ID),
        0,
        "no entry was applied before dispatch was cancelled"
    );
    assert_eq!(
        index.writer_calls(),
        0,
        "dispatch was cancelled before any upsert call"
    );
}

/// Amendments-971 A9: each instrument settles as an independent leg with no
/// atomicity across instruments. One leg's adapter failure retries only
/// that leg; the other, already `complete`, is never dispatched again, and
/// the run records its Settle outcome only once both legs are terminal.
#[tokio::test]
async fn independent_instruments_retry_without_repeating_a_completed_one() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _index, adapters) = test_service(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        None,
    )
    .await;
    let rebate = adapters[0].clone();
    let trace_credit = adapters[1].clone();
    let tenant = format!("settle-instruments-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;

    // FR3 (I1): an adapter call error is a dependency failure, not the
    // trace's fault -- an uncharged retry, however many times it repeats,
    // including more times than the run's whole attempt budget.
    let mut retried = None;
    for attempt in 1..=run.max_attempts + 2 {
        rebate.fail_next();
        force_due(&backend, &tenant, run.run_id).await;
        let current = service
            .process_run(&tenant, run.run_id)
            .await
            .unwrap()
            .expect("Settle retries while one leg is blocked");
        assert_eq!(current.state, PipelineRunState::Retry, "attempt {attempt}");
        assert_eq!(
            current.last_error_label.as_deref(),
            Some("settlement_operation_retry"),
            "attempt {attempt}"
        );
        assert_eq!(
            current.attempt_count, run.attempt_count,
            "attempt {attempt}: an adapter call error never charges the run"
        );
        retried = Some(current);
    }
    assert_eq!(
        retried.expect("at least one retry").state,
        PipelineRunState::Retry
    );

    let settlements = service
        .store()
        .list_settlements(&tenant, run.run_id)
        .await
        .unwrap();
    let rebate_row = settlements
        .iter()
        .find(|settlement| settlement.instrument_id == "storage_rebate")
        .expect("storage_rebate row seeded");
    let credit_row = settlements
        .iter()
        .find(|settlement| settlement.instrument_id == InstrumentId::trace_credit().as_str())
        .expect("trace_credit row seeded");
    assert_eq!(rebate_row.operation_state, "retry");
    assert_eq!(credit_row.operation_state, "complete");
    let expected_credit_result = pipeline_result_ref(
        run.run_id,
        &InstrumentAward::new(
            InstrumentId::trace_credit(),
            AtomicUnits::from_raw(1_000_000),
        )
        .unwrap(),
    );
    assert_eq!(
        credit_row.result_ref_hash.as_deref(),
        Some(expected_credit_result.as_str())
    );

    let outcomes = service
        .store()
        .list_outcomes(&tenant, run.run_id)
        .await
        .unwrap();
    assert!(
        !outcomes
            .iter()
            .any(|outcome| outcome.phase == Phase::Settle),
        "no Settle outcome while a leg is still blocked"
    );
    assert_eq!(
        trace_credit.requests().len(),
        1,
        "the completed leg was dispatched exactly once so far"
    );

    force_due(&backend, &tenant, run.run_id).await;
    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("the retry completes the remaining leg");
    assert_eq!(settled.state, PipelineRunState::Complete);

    assert_eq!(
        trace_credit.requests().len(),
        1,
        "the already-complete leg was never dispatched again"
    );
    assert_eq!(
        count_credit_ledger_rows_for_run(&backend, &tenant, run.run_id).await,
        1,
        "exactly one credit ledger row for the run"
    );

    let settlements = service
        .store()
        .list_settlements(&tenant, run.run_id)
        .await
        .unwrap();
    let credit_row = settlements
        .iter()
        .find(|settlement| settlement.instrument_id == InstrumentId::trace_credit().as_str())
        .expect("trace_credit row present");
    let batch_id = credit_row
        .settlement_batch_id
        .expect("trace_credit settled into a batch");
    assert_eq!(
        settlement_batch_status(&backend, &tenant, batch_id).await,
        Some("finalized".to_string())
    );

    let outcomes = service
        .store()
        .list_outcomes(&tenant, run.run_id)
        .await
        .unwrap();
    let settle_outcomes: Vec<_> = outcomes
        .into_iter()
        .filter(|outcome| outcome.phase == Phase::Settle)
        .collect();
    assert_eq!(settle_outcomes.len(), 1, "exactly one Settle outcome");
    let settle_outcome = settle_outcomes.into_iter().next().unwrap();
    let decision: SettleDecision = serde_json::from_value(settle_outcome.decision).unwrap();
    let operations = decision.settlement_operations();
    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0].instrument_id().as_str(), "storage_rebate");
    assert_eq!(operations[1].instrument_id().as_str(), "trace_credit");
    for operation in operations {
        match operation.outcome() {
            InstrumentSettlementOutcome::Completed { result_ref_hash } => {
                assert!(!result_ref_hash.is_empty());
            }
            InstrumentSettlementOutcome::Forfeited { .. } => {
                panic!("both legs completed; neither should be forfeited")
            }
        }
    }
    let evidence: SettleEvidence = serde_json::from_value(settle_outcome.evidence).unwrap();
    assert_eq!(evidence.settlement_progress.len(), 2);
}

/// Brief 3C / amendments-971: a withdrawal recorded after Score forfeits
/// every settlement leg that has not already completed -- no adapter is
/// called for either instrument -- and Settle still completes the run with
/// a committed `Forfeited` operation per leg.
#[tokio::test]
async fn withdrawal_after_score_forfeits_pending_operations_and_settle_completes() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _index, adapters) = test_service(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        None,
    )
    .await;
    let rebate = adapters[0].clone();
    let trace_credit = adapters[1].clone();
    let tenant = format!("settle-forfeit-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;

    withdraw_submission(&backend, &tenant, run.submission_id).await;

    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle completes despite the withdrawal");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(settled.next_phase, None);

    assert_eq!(
        rebate.requests().len(),
        0,
        "no adapter call for a forfeited leg"
    );
    assert_eq!(
        trace_credit.requests().len(),
        0,
        "no adapter call for a forfeited leg"
    );
    assert_eq!(
        count_credit_ledger_rows_for_run(&backend, &tenant, run.run_id).await,
        0,
        "no credit ledger row for a forfeited trace_credit leg"
    );

    let settlements = service
        .store()
        .list_settlements(&tenant, run.run_id)
        .await
        .unwrap();
    assert_eq!(settlements.len(), 2);
    for settlement in &settlements {
        assert_eq!(settlement.operation_state, "forfeited");
        assert_eq!(settlement.result_ref_hash, None);
    }

    let outcomes = service
        .store()
        .list_outcomes(&tenant, run.run_id)
        .await
        .unwrap();
    let settle_outcome = outcomes
        .into_iter()
        .find(|outcome| outcome.phase == Phase::Settle)
        .expect("Settle outcome recorded");
    let decision: SettleDecision = serde_json::from_value(settle_outcome.decision).unwrap();
    let operations = decision.settlement_operations();
    assert_eq!(operations.len(), 2);
    for operation in operations {
        match operation.outcome() {
            InstrumentSettlementOutcome::Forfeited { reason } => {
                assert_eq!(reason.as_str(), PIPELINE_SUBMISSION_INOPERABLE_LABEL);
            }
            InstrumentSettlementOutcome::Completed { .. } => {
                panic!("expected every operation to be forfeited")
            }
        }
    }
    match decision.index_membership {
        IndexMembershipDecision::Exclude { reason } => {
            assert_eq!(reason.as_str(), PIPELINE_SUBMISSION_INOPERABLE_LABEL);
        }
        IndexMembershipDecision::Include { .. } => {
            panic!("a withdrawn submission must not commit an Include decision")
        }
    }
    let evidence: SettleEvidence = serde_json::from_value(settle_outcome.evidence).unwrap();
    assert_eq!(evidence.submission_operable, Some(false));
}

/// A withdrawal recorded after a run has already completed leaves the
/// settled leg, its finalized batch, and the committed Settle outcome
/// untouched -- there is no reprocessing path that could revisit them, and
/// this locks that in.
#[tokio::test]
async fn settled_credit_stays_when_withdrawal_follows_settlement() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _index, _adapters) = test_service(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        None,
    )
    .await;
    let tenant = format!("settle-post-withdraw-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;

    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle completes");
    assert_eq!(settled.state, PipelineRunState::Complete);

    let settlements_before = service
        .store()
        .list_settlements(&tenant, run.run_id)
        .await
        .unwrap();
    let credit_before = settlements_before
        .iter()
        .find(|settlement| settlement.instrument_id == InstrumentId::trace_credit().as_str())
        .expect("trace_credit row present")
        .clone();
    let outcomes_before = service
        .store()
        .list_outcomes(&tenant, run.run_id)
        .await
        .unwrap();
    let settle_outcome_before = outcomes_before
        .into_iter()
        .find(|outcome| outcome.phase == Phase::Settle)
        .expect("Settle outcome recorded");
    let batch_id = credit_before
        .settlement_batch_id
        .expect("trace_credit settled into a batch");
    let batch_status_before = settlement_batch_status(&backend, &tenant, batch_id).await;
    let ledger_count_before = count_credit_ledger_rows_for_run(&backend, &tenant, run.run_id).await;

    withdraw_submission(&backend, &tenant, run.submission_id).await;

    let settlements_after = service
        .store()
        .list_settlements(&tenant, run.run_id)
        .await
        .unwrap();
    let credit_after = settlements_after
        .iter()
        .find(|settlement| settlement.instrument_id == InstrumentId::trace_credit().as_str())
        .expect("trace_credit row present")
        .clone();
    assert_eq!(
        credit_after, credit_before,
        "the settled leg is unchanged by a later withdrawal"
    );

    let outcomes_after = service
        .store()
        .list_outcomes(&tenant, run.run_id)
        .await
        .unwrap();
    let settle_outcome_after = outcomes_after
        .into_iter()
        .find(|outcome| outcome.phase == Phase::Settle)
        .expect("Settle outcome still recorded");
    assert_eq!(
        settle_outcome_after.decision,
        settle_outcome_before.decision
    );
    assert_eq!(
        settle_outcome_after.evidence,
        settle_outcome_before.evidence
    );

    assert_eq!(
        settlement_batch_status(&backend, &tenant, batch_id).await,
        batch_status_before
    );
    assert_eq!(
        count_credit_ledger_rows_for_run(&backend, &tenant, run.run_id).await,
        ledger_count_before
    );
}

/// A hold on the Trace Credit account (the same `TraceCorpusStore` API the
/// port's `place_credit_hold` uses) keeps that leg pending while every other
/// instrument still completes: the run retries under `credit_held`, the
/// held leg's adapter is never called (FR2, I3), and only once the hold is
/// released does the leg settle and the Settle outcome commit.
#[tokio::test]
async fn a_held_account_keeps_trace_credit_pending_and_other_instruments_complete() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _index, adapters) = test_service(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        None,
    )
    .await;
    let trace_credit = adapters[1].clone();
    let tenant = format!("settle-held-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;

    // The receipt helper always submits as this principal (`receipt`'s
    // fixed `actor_principal_ref`), which becomes the submission's
    // `auth_principal_ref` and so the credit account the hold must name.
    let account_ref = "principal_sha256:test".to_string();
    let hold_id = uuid::Uuid::new_v4();
    backend
        .upsert_trace_credit_hold(TraceCreditHoldWrite {
            tenant_id: tenant.clone(),
            hold_id,
            credit_account_ref: account_ref.clone(),
            credit_account_hash: credit_account_hash(&account_ref),
            reason: TraceCreditHoldReason::PolicyMigration,
            reason_hash: credit_account_hash("pipeline-hold"),
            actor_principal_ref: account_ref.clone(),
            released_at: None,
        })
        .await
        .expect("place a credit hold through the runtime role");

    let held = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle retries while the account is held");
    assert_eq!(held.state, PipelineRunState::Retry);
    assert_eq!(
        held.last_error_label.as_deref(),
        Some(PIPELINE_CREDIT_HELD_LABEL)
    );
    // FR3 (C3): a hold is a suspension, not a charged retry. The run stays
    // in retry with its attempt count unchanged for more retries than its
    // whole attempt budget, so the credit is never lost to exhaustion.
    assert_eq!(held.attempt_count, run.attempt_count);
    for attempt in 1..=run.max_attempts + 2 {
        force_due(&backend, &tenant, run.run_id).await;
        let still_held = service
            .process_run(&tenant, run.run_id)
            .await
            .unwrap()
            .expect("Settle keeps waiting while the account is held");
        assert_eq!(
            still_held.state,
            PipelineRunState::Retry,
            "held retry {attempt}"
        );
        assert_eq!(
            still_held.last_error_label.as_deref(),
            Some(PIPELINE_CREDIT_HELD_LABEL),
            "held retry {attempt}"
        );
        assert_eq!(
            still_held.attempt_count, run.attempt_count,
            "held retry {attempt}: a hold never charges the run"
        );
    }

    let settlements = service
        .store()
        .list_settlements(&tenant, run.run_id)
        .await
        .unwrap();
    let rebate_row = settlements
        .iter()
        .find(|settlement| settlement.instrument_id == "storage_rebate")
        .expect("storage_rebate row seeded");
    let credit_row = settlements
        .iter()
        .find(|settlement| settlement.instrument_id == InstrumentId::trace_credit().as_str())
        .expect("trace_credit row seeded");
    assert_eq!(rebate_row.operation_state, "complete");
    assert_eq!(credit_row.operation_state, "held");
    assert_eq!(credit_row.result_ref_hash, None);
    assert_eq!(
        credit_row.last_error_label.as_deref(),
        Some(PIPELINE_CREDIT_HELD_LABEL)
    );
    assert_eq!(
        trace_credit.requests().len(),
        0,
        "a held account never reaches its adapter (FR2: the hold is checked first)"
    );
    assert_eq!(
        count_credit_ledger_rows_for_run(&backend, &tenant, run.run_id).await,
        0,
        "no ledger row while the account is held"
    );

    let outcomes = service
        .store()
        .list_outcomes(&tenant, run.run_id)
        .await
        .unwrap();
    assert!(
        !outcomes
            .iter()
            .any(|outcome| outcome.phase == Phase::Settle),
        "no Settle outcome while the trace_credit leg is held"
    );

    backend
        .upsert_trace_credit_hold(TraceCreditHoldWrite {
            tenant_id: tenant.clone(),
            hold_id,
            credit_account_ref: account_ref.clone(),
            credit_account_hash: credit_account_hash(&account_ref),
            reason: TraceCreditHoldReason::PolicyMigration,
            reason_hash: credit_account_hash("pipeline-hold"),
            actor_principal_ref: account_ref.clone(),
            released_at: Some(chrono::Utc::now()),
        })
        .await
        .expect("release the credit hold");

    force_due(&backend, &tenant, run.run_id).await;
    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("the retry completes once the hold is released");
    assert_eq!(settled.state, PipelineRunState::Complete);

    let settlements = service
        .store()
        .list_settlements(&tenant, run.run_id)
        .await
        .unwrap();
    let credit_row = settlements
        .iter()
        .find(|settlement| settlement.instrument_id == InstrumentId::trace_credit().as_str())
        .expect("trace_credit row present");
    assert_eq!(credit_row.operation_state, "complete");
    assert_eq!(
        trace_credit.requests().len(),
        1,
        "the adapter is dispatched once the hold is released"
    );
    assert_credit_settled_once(&backend, &service, &tenant, run.run_id, "after release").await;

    let outcomes = service
        .store()
        .list_outcomes(&tenant, run.run_id)
        .await
        .unwrap();
    let settle_outcomes: Vec<_> = outcomes
        .into_iter()
        .filter(|outcome| outcome.phase == Phase::Settle)
        .collect();
    assert_eq!(settle_outcomes.len(), 1, "exactly one Settle outcome");
}

/// FR2, Failure 1: the Trace Credit adapter settles, and the lease expires
/// before the ledger work commits (a slow adapter call). The credit
/// transaction re-checks the lease on the run row and rolls back, so nothing
/// is half-written; the next claim repeats the idempotent adapter call and
/// commits the ledger row, the finalized batch, and the settlement row
/// together. Before the fix the ledger committed in its own transactions,
/// the settlement row could not be written under the stale lease, and every
/// retry then failed on the already-final event until the run was
/// exhausted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credit_ledger_work_survives_a_lease_that_expires_during_the_adapter_call() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let tenant = format!("settle-credit-lease-{}", uuid::Uuid::new_v4());
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
    let expiring = Arc::new(InterruptingCreditAdapter {
        inner: trace_credit.clone(),
        backend: backend.clone(),
        tenant_id: tenant.clone(),
        interruption: CreditInterruption::ExpireLease,
        armed: std::sync::atomic::AtomicBool::new(true),
    });
    let service = test_service_with_adapters(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        vec![
            storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            expiring as Arc<dyn SettlementAdapter>,
        ],
    )
    .await;
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;

    // The first Settle attempt loses its lease inside the adapter call; it
    // may surface as an error (its own retry bookkeeping is fenced by the
    // same lease) or as a retry, but it must not complete the run.
    let _ = service.process_run(&tenant, run.run_id).await;
    let after_first = service
        .store()
        .get_run(&tenant, run.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(after_first.state, PipelineRunState::Complete);

    for attempt in 0..6 {
        let current = service
            .store()
            .get_run(&tenant, run.run_id)
            .await
            .unwrap()
            .unwrap();
        if current.state == PipelineRunState::Complete {
            break;
        }
        assert!(
            attempt < 5 && current.state != PipelineRunState::Failed,
            "the run must complete after the stale attempt (state {:?}, label {:?})",
            current.state,
            current.last_error_label
        );
        force_due(&backend, &tenant, run.run_id).await;
        let _ = service.process_run(&tenant, run.run_id).await;
    }

    assert_credit_settled_once(
        &backend,
        &service,
        &tenant,
        run.run_id,
        "after a lease expired during the adapter call",
    )
    .await;
    assert_eq!(
        trace_credit.requests().len(),
        1,
        "the repeated adapter call is one logical request"
    );
    assert_eq!(storage_rebate.requests().len(), 1);
}

/// FR2: a hold placed after the pre-dispatch hold check but before the
/// credit transaction is caught by the transaction's re-check under the
/// account lock. The transaction rolls back with nothing written, the leg
/// waits as `held`, and once the hold is released the retry repeats the
/// idempotent adapter call and settles the leg exactly once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hold_placed_during_the_adapter_call_is_caught_by_the_credit_transaction() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let tenant = format!("settle-credit-hold-race-{}", uuid::Uuid::new_v4());
    let hold_id = uuid::Uuid::new_v4();
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
    let holding = Arc::new(InterruptingCreditAdapter {
        inner: trace_credit.clone(),
        backend: backend.clone(),
        tenant_id: tenant.clone(),
        interruption: CreditInterruption::PlaceHold(credit_hold(&tenant, hold_id, None)),
        armed: std::sync::atomic::AtomicBool::new(true),
    });
    let service = test_service_with_adapters(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        vec![
            storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            holding as Arc<dyn SettlementAdapter>,
        ],
    )
    .await;
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;

    let held = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle waits while the account is held");
    assert_eq!(held.state, PipelineRunState::Retry);
    assert_eq!(
        held.last_error_label.as_deref(),
        Some(PIPELINE_CREDIT_HELD_LABEL)
    );
    let credit_row = service
        .store()
        .list_settlements(&tenant, run.run_id)
        .await
        .unwrap()
        .into_iter()
        .find(|settlement| settlement.instrument_id == InstrumentId::trace_credit().as_str())
        .expect("trace_credit row seeded");
    assert_eq!(credit_row.operation_state, "held");
    assert_eq!(credit_row.credit_event_id, None);
    assert_eq!(credit_row.settlement_batch_id, None);
    assert_eq!(
        count_credit_ledger_rows_for_run(&backend, &tenant, run.run_id).await,
        0,
        "the credit transaction rolled back: no ledger row"
    );

    backend
        .upsert_trace_credit_hold(credit_hold(&tenant, hold_id, Some(chrono::Utc::now())))
        .await
        .expect("release the credit hold");
    force_due(&backend, &tenant, run.run_id).await;
    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("the retry settles the leg once the hold is released");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_credit_settled_once(&backend, &service, &tenant, run.run_id, "after release").await;
    assert_eq!(
        trace_credit.requests().len(),
        1,
        "the adapter call repeated after the rollback is one logical request"
    );
}

/// FR2: two runs for one credit account that settle at the same time never
/// place one credit event in two finalized batches. The per-account advisory
/// lock in the credit transaction serializes the pending-event selection and
/// the final mark, so each event lands in exactly one batch. Repeated over
/// several tenants to give the interleaving a chance to occur.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_runs_for_one_account_never_share_a_credit_event_across_batches() {
    let Some(backend) = runtime_backend(8).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _index, _adapters) = test_service(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        None,
    )
    .await;
    for round in 0..4 {
        let tenant = format!("settle-two-runs-{round}-{}", uuid::Uuid::new_v4());
        // The receipt helper submits every run as the same principal, so
        // both runs credit one account.
        let (first, _) = run_to_settle_ready(&service, &tenant).await;
        let (second, _) = run_to_settle_ready(&service, &tenant).await;
        let (left, right) = tokio::join!(
            service.process_run(&tenant, first.run_id),
            service.process_run(&tenant, second.run_id)
        );
        left.expect("first Settle attempt");
        right.expect("second Settle attempt");
        for run_id in [first.run_id, second.run_id] {
            for _ in 0..5 {
                let current = service
                    .store()
                    .get_run(&tenant, run_id)
                    .await
                    .unwrap()
                    .unwrap();
                if current.state == PipelineRunState::Complete {
                    break;
                }
                force_due(&backend, &tenant, run_id).await;
                service.process_run(&tenant, run_id).await.unwrap();
            }
            assert_credit_settled_once(
                &backend,
                &service,
                &tenant,
                run_id,
                &format!("round {round}"),
            )
            .await;
        }
    }
}

/// D4: the expected result comes from the persisted selection, not from
/// whatever the adapter hands back. An adapter that returns a well-formed
/// but different result reference fails the row closed rather than being
/// trusted.
#[tokio::test]
async fn adapter_result_that_differs_from_the_selection_fails_closed() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mismatching = Arc::new(MismatchingSettlementAdapter {
        instrument_id: InstrumentId::new("storage_rebate").unwrap(),
    });
    let trace_credit_adapter = RecordingSettlementAdapter::new(
        InstrumentId::trace_credit(),
        "recording_trace_credit_test_only",
        "none",
    );
    let service = test_service_with_adapters(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        vec![
            mismatching as Arc<dyn SettlementAdapter>,
            trace_credit_adapter as Arc<dyn SettlementAdapter>,
        ],
    )
    .await;
    let tenant = format!("settle-mismatch-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;

    let result = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle retries after the mismatch");
    assert_eq!(result.state, PipelineRunState::Retry);
    assert_eq!(
        result.last_error_label.as_deref(),
        Some("settlement_operation_retry")
    );
    // FR3: unlike an adapter call error, a result that differs from the
    // selection fails closed and stays charged.
    assert_eq!(result.attempt_count, run.attempt_count + 1);

    let settlements = service
        .store()
        .list_settlements(&tenant, run.run_id)
        .await
        .unwrap();
    let rebate_row = settlements
        .iter()
        .find(|settlement| settlement.instrument_id == "storage_rebate")
        .expect("storage_rebate row seeded");
    assert_eq!(rebate_row.operation_state, "failed");
    assert_eq!(
        rebate_row.last_error_label.as_deref(),
        Some("settlement_result_mismatch")
    );
    assert_eq!(rebate_row.result_ref_hash, None);

    let outcomes = service
        .store()
        .list_outcomes(&tenant, run.run_id)
        .await
        .unwrap();
    assert!(
        !outcomes
            .iter()
            .any(|outcome| outcome.phase == Phase::Settle),
        "no Settle outcome after a mismatch"
    );
}

/// 3A acceptance: a run keeps the bundle it was bound to at receipt even
/// after another bundle is activated for the tenant. Activating bundle B
/// changes what a *new* receipt binds to; it never rebinds a run already in
/// flight.
#[tokio::test]
async fn activation_does_not_rebind_an_existing_run() {
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
    let tenant = format!("activation-{}", uuid::Uuid::new_v4());

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
    let bundle_a = created.bundle_id.clone();

    // Register and activate a second bundle B for the tenant, with a
    // storage_rebate award A does not have -- so if a phase ever ran under
    // the wrong bundle, its settlement rows would visibly differ, not just
    // its `bundle_id` label (`pipeline_runs.bundle_id` is immutable and
    // `phase_outcomes.bundle_id` is stamped from the run row either way, so
    // neither alone would catch the runner loading the wrong package). The
    // run above must stay bound to A; only a receipt submitted from here on
    // should see B.
    let scorer = ReferencePerplexityScorer::new();
    let embedder = ReferenceEmbedder::new();
    let config_b = PipelineBundleConfig {
        instrument_awards: vec![PipelineInstrumentAwardConfig {
            instrument_id: "storage_rebate".into(),
            atomic_units: AtomicUnits::from_raw(9),
            descriptor: storage_rebate_descriptor(),
        }],
        include_index: false,
        variant: Some("activation-b".to_string()),
    };
    let package_b = MinimalPolicyBundle::minimal_package(&config_b, &scorer, &embedder)
        .expect("build bundle B package");
    assert_ne!(bundle_a, package_b.bundle_id);
    service
        .register_bundle(&tenant, &package_b)
        .await
        .expect("register bundle B");
    service
        .activate_bundle(&tenant, &package_b.bundle_id)
        .await
        .expect("activate bundle B");

    // Process the run to completion: every outcome stays bound to A.
    service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Review runs");
    service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Score runs");
    let settled = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Settle runs");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(settled.bundle_id, bundle_a);

    // A's Score policy awards nothing: if Score had run under B instead, a
    // storage_rebate settlement row would exist.
    let settlements = service
        .store()
        .list_settlements(&tenant, created.run_id)
        .await
        .unwrap();
    assert!(
        settlements.is_empty(),
        "the run settled under A's award-free policy, not B's"
    );

    let outcomes = service
        .store()
        .list_outcomes(&tenant, created.run_id)
        .await
        .unwrap();
    assert!(!outcomes.is_empty(), "the run recorded outcomes");
    for outcome in &outcomes {
        assert_eq!(
            outcome.bundle_id, bundle_a,
            "every outcome stays bound to the bundle the run was bound to at receipt"
        );
    }

    // A new receipt binds to B.
    let env_second = envelope(uuid::Uuid::new_v4()).await;
    let raw_second = serde_json::to_vec(&env_second).unwrap();
    let key_second = env_second.submission_id.to_string();
    let PipelineReceiptResult::Created(created_second) = service
        .submit(receipt(
            &tenant,
            &key_second,
            &raw_second,
            &env_second,
            NO_LIMITS,
        ))
        .await
        .unwrap()
    else {
        panic!("second receipt creates a run")
    };
    assert_eq!(created_second.bundle_id, package_b.bundle_id);
}

/// 3A acceptance (decision D9): a run whose named dependency (by content
/// hash) is not held by the service processing it waits in retry without
/// charging the attempt the claim took, rather than failing. The moment a
/// service that does hold the named dependency processes the same run, it
/// proceeds.
#[tokio::test]
async fn a_run_whose_dependency_is_not_held_waits_without_charging() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let shared_artifact_store = artifact_store(&dir);
    let (service_one, _, _) = test_service(
        backend.clone(),
        shared_artifact_store.clone(),
        minimal_config(false),
        None,
    )
    .await;
    let counting_embedder = Arc::new(CountingEmbedder {
        descriptor: b"dependency-missing-test-embedder-v1".to_vec(),
        calls: AtomicUsize::new(0),
    });
    let service_two = test_service_with_embedder(
        backend.clone(),
        shared_artifact_store,
        minimal_config(false),
        counting_embedder,
    )
    .await;

    let tenant = format!("dep-missing-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) = service_one
        .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .unwrap()
    else {
        panic!("receipt creates a run")
    };

    let reviewed = service_one
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("service 1 holds every dependency Review needs");
    assert_eq!(reviewed.next_phase, Some(Phase::Score));
    let attempt_count_before = reviewed.attempt_count;

    // Service 2 does not hold the embedder bundle A names: the run waits in
    // retry without the claim's attempt being charged.
    let waited = service_two
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("service 2 releases the run into retry rather than failing it");
    assert_eq!(waited.state, PipelineRunState::Retry);
    assert_eq!(
        waited.last_error_label.as_deref(),
        Some("bundle_dependency_missing")
    );
    assert_eq!(
        waited.attempt_count, attempt_count_before,
        "a missing named dependency never charges the attempt the claim took"
    );

    force_due(&backend, &tenant, created.run_id).await;

    // Service 1 holds the named embedder: the same run now proceeds.
    let scored = service_one
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("service 1 holds the named embedder and completes Score");
    assert_eq!(scored.next_phase, Some(Phase::Settle));
}

/// R4 (decision D9): a `PolicyError::Transient` raised while a phase runs --
/// not only a missing bound dependency at the bundle load, which
/// `a_run_whose_dependency_is_not_held_waits_without_charging` above already
/// covers -- releases the run without charging the claim's attempt, so a
/// dependency outage cannot exhaust the trace's attempt budget.
/// `FixedScorePolicy` (`versioned_pipeline_bundle.rs`) maps an embedder
/// failure to `PolicyError::transient("embedder_unavailable")` when the
/// bundle carries an index (P5). `FlakyEmbedder` fails its first 7 calls;
/// `max_attempts` defaults to 5 (migration V76/V77), so 7 failures is more
/// than the run's whole attempt budget, and the run still reaches Score.
#[tokio::test]
async fn transient_policy_errors_do_not_exhaust_the_trace() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let flaky_embedder = Arc::new(FlakyEmbedder {
        calls: AtomicUsize::new(0),
    });
    let service = test_service_with_embedder(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(true),
        flaky_embedder,
    )
    .await;

    let tenant = format!("transient-score-retry-{}", uuid::Uuid::new_v4());
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

    let reviewed = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Review runs");
    assert_eq!(reviewed.next_phase, Some(Phase::Score));

    // Read `max_attempts` from the run row itself, and prove the test's
    // premise: 7 failures is more than the whole budget, not merely more
    // than what is left of it.
    assert!(
        7 > reviewed.max_attempts,
        "the test proves the trace survives more failures than its attempt \
         budget (max_attempts = {}), not merely a lucky few",
        reviewed.max_attempts
    );
    let attempt_count_before_score = reviewed.attempt_count;

    for attempt in 1..=7 {
        force_due(&backend, &tenant, created.run_id).await;
        let retried = service
            .process_run(&tenant, created.run_id)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("attempt {attempt} releases the run into retry"));
        assert_eq!(
            retried.state,
            PipelineRunState::Retry,
            "attempt {attempt} stays in retry, never failed, although attempt \
             {attempt} > max_attempts once attempt > 5"
        );
        assert_eq!(
            retried.last_error_label.as_deref(),
            Some("embedder_unavailable"),
            "attempt {attempt} carries the policy's own transient label"
        );
        assert_eq!(
            retried.attempt_count, attempt_count_before_score,
            "attempt {attempt}: a transient policy failure never charges the \
             attempt the claim took"
        );
    }

    // The 8th call: the embedder's 8th `embed` call succeeds, and Score
    // completes.
    force_due(&backend, &tenant, created.run_id).await;
    let scored = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("the 8th call completes Score");
    assert_eq!(scored.state, PipelineRunState::Pending);
    assert_eq!(scored.next_phase, Some(Phase::Settle));
    assert_eq!(
        scored.attempt_count,
        attempt_count_before_score + 1,
        "the attempt that finally succeeds is the only one that charges the trace"
    );
}

/// 3A acceptance: a stored bundle package that has been tampered with
/// underneath the service fails closed -- the run is marked failed under
/// the safe label, and no new phase outcome is recorded for it.
#[tokio::test]
async fn a_tampered_stored_package_fails_closed() {
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
    let tenant = format!("tampered-package-{}", uuid::Uuid::new_v4());

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

    let outcomes_before = service
        .store()
        .list_outcomes(&tenant, created.run_id)
        .await
        .unwrap();

    tamper_stored_bundle_package(&tenant, &created.bundle_id).await;

    let failed = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("the run fails closed on a tampered package");
    assert_eq!(failed.state, PipelineRunState::Failed);
    assert_eq!(
        failed.last_error_label.as_deref(),
        Some("bundle_package_invalid")
    );

    let outcomes_after = service
        .store()
        .list_outcomes(&tenant, created.run_id)
        .await
        .unwrap();
    assert_eq!(
        outcomes_after.len(),
        outcomes_before.len(),
        "no new outcome is recorded when the stored package fails closed"
    );
}

/// Review focus item 2, generalized to every crash point (Task 16): a crash
/// between an artifact write and its database commit -- and at every other
/// injected point -- must leave exactly one logical effect per phase once a
/// second service resumes the run to completion.
///
/// `AfterArtifactStorage` crashes inside `submit` itself, before the run's
/// insert ever commits (the whole receipt is one transaction that has not
/// reached its final commit yet), so recovery there is a resubmission of
/// the same bytes, not a lease-expiry resume. Every other point crashes
/// mid-phase, after the run was claimed (`leased`) -- `commit_review`,
/// `commit_score`, and `commit_settle` each clear the lease as part of
/// their own transaction before the crash point that follows them fires, so
/// for those three the lease is already gone by the time service A's error
/// propagates; for the rest the run is genuinely stuck `leased`. Expiring
/// the lease unconditionally (the earlier crash tests' pattern: a direct
/// UPDATE, a time shortcut, never a processor call) is harmless either way,
/// since `claim_run` only consults it for a row still in state `leased`.
#[tokio::test]
async fn crash_matrix_produces_one_logical_effect_per_point() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    for point in [
        PipelineCrashPoint::AfterArtifactStorage,
        PipelineCrashPoint::AfterReviewArtifactStorage,
        PipelineCrashPoint::AfterReviewCommit,
        PipelineCrashPoint::AfterScoreArtifactStorage,
        PipelineCrashPoint::AfterScoreCommit,
        PipelineCrashPoint::AfterSettleSelection,
        PipelineCrashPoint::AfterIndexApply,
        PipelineCrashPoint::AfterInstrumentOperation,
        PipelineCrashPoint::AfterCreditLedgerInsert,
        PipelineCrashPoint::AfterCreditBatchFinalize,
        PipelineCrashPoint::AfterSettleCommit,
    ] {
        // Fresh tenant, artifact directory, and index per point (ruling:
        // "if that keeps the points independent") -- the index is an
        // in-memory double whose `writer_calls()` counter is global, not
        // tenant-scoped, so a fresh one per point keeps that counter (and
        // the on-disk artifact tree) free of carryover from earlier points.
        let dir = tempfile::tempdir().unwrap();
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
        let adapters: Vec<Arc<dyn SettlementAdapter>> = vec![
            storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            trace_credit.clone() as Arc<dyn SettlementAdapter>,
        ];

        // P5 (ruling): A and B share the same two recording adapters, the
        // same `IsolatedPipelineIndex`, the same database (one `backend`
        // Arc), and the same artifact root -- a second `artifact_store(&dir)`
        // over the same directory, as a real restart would reopen it.
        let service_a = test_service_with_adapters_and_index(
            backend.clone(),
            artifact_store(&dir),
            scored_config(true),
            adapters.clone(),
            index.clone(),
            Some(point),
        )
        .await;
        let service_b = test_service_with_adapters_and_index(
            backend.clone(),
            artifact_store(&dir),
            scored_config(true),
            adapters.clone(),
            index.clone(),
            None,
        )
        .await;

        let tenant = format!("crash-matrix-{point:?}-{}", uuid::Uuid::new_v4());
        let env = envelope(uuid::Uuid::new_v4()).await;
        let raw = serde_json::to_vec(&env).unwrap();
        let key = env.submission_id.to_string();

        let run_id = if point == PipelineCrashPoint::AfterArtifactStorage {
            let crashed = service_a
                .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
                .await;
            let error =
                crashed.expect_err("service A's receipt must crash at AfterArtifactStorage");
            assert_eq!(error.to_string(), INJECTED_PIPELINE_CRASH);

            // Nothing committed (the receipt transaction never reached its
            // final commit), so this is a fresh submission from the store's
            // point of view, not a claim resume -- resubmit the same bytes.
            let PipelineReceiptResult::Created(created) = service_b
                .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
                .await
                .unwrap()
            else {
                panic!("resubmission after the crash must create the run (point {point:?})")
            };
            created.run_id
        } else {
            let PipelineReceiptResult::Created(created) = service_a
                .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
                .await
                .unwrap()
            else {
                panic!("receipt creates a run (point {point:?})")
            };

            // Drive service A phase by phase until its injected crash
            // fires -- bounded, with a clear failure message if it never
            // does (Review, then Score, then Settle is at most 3 calls).
            let mut crashed_error = None;
            for _ in 0..5 {
                match service_a.process_run(&tenant, created.run_id).await {
                    Ok(_) => continue,
                    Err(error) => {
                        crashed_error = Some(error);
                        break;
                    }
                }
            }
            let error = crashed_error.unwrap_or_else(|| {
                panic!(
                    "service A never hit its crash point within 5 process_run calls (point {point:?})"
                )
            });
            assert_eq!(error.to_string(), INJECTED_PIPELINE_CRASH);

            // `pipeline_runs_lease_shape` requires `lease_token`/
            // `lease_expires_at` to be both set or both null, so this only
            // touches a row still genuinely `leased` -- for
            // `AfterReviewCommit`/`AfterScoreCommit`, whose commit already
            // cleared the lease before the crash fired, the row is not
            // `leased` and this matches zero rows (a harmless no-op).
            let mut client = backend.trace_pool_for_test().get().await.unwrap();
            let tx = tenant_tx(&mut client, &tenant).await;
            tx.execute(
                "UPDATE pipeline_runs SET lease_expires_at = NOW() - INTERVAL '1 second'
                 WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'",
                &[&tenant, &created.run_id],
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            created.run_id
        };

        // Resume with service B until the run completes -- bounded, with a
        // clear failure message if it never does. The first iteration's
        // check also covers `AfterSettleCommit`, whose crash fires only
        // after `commit_settle` already committed `state = complete`: the
        // loop finds the run already `Complete` and calls `process_run`
        // zero times.
        for attempt in 0..6 {
            let current = service_b
                .store()
                .get_run(&tenant, run_id)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("run must exist while resuming (point {point:?})"));
            if current.state == PipelineRunState::Complete {
                break;
            }
            assert!(
                attempt < 5,
                "run at crash point {point:?} did not reach Complete within 6 resume attempts \
                 (last state {:?})",
                current.state
            );
            service_b
                .process_run(&tenant, run_id)
                .await
                .unwrap_or_else(|error| {
                    panic!("service B failed to resume at point {point:?}: {error}")
                });
        }

        // Exactly four phase outcomes, one per phase.
        let outcomes = service_b
            .store()
            .list_outcomes(&tenant, run_id)
            .await
            .unwrap();
        for phase in [Phase::Admission, Phase::Review, Phase::Score, Phase::Settle] {
            assert_eq!(
                outcomes
                    .iter()
                    .filter(|outcome| outcome.phase == phase)
                    .count(),
                1,
                "expected exactly one {phase:?} outcome at crash point {point:?}"
            );
        }

        // A9: legs are independent -- each adapter was dispatched exactly
        // once across A and B together (the adapters are the same shared
        // instances for both services).
        assert_eq!(
            storage_rebate.requests().len(),
            1,
            "storage_rebate must be dispatched exactly once across A and B at point {point:?}"
        );
        assert_eq!(
            trace_credit.requests().len(),
            1,
            "trace_credit must be dispatched exactly once across A and B at point {point:?}"
        );

        // Ruling FR2: exactly one trace_credit_ledger row for the run, its
        // event final, exactly one finalized batch carrying it, and that
        // batch's instrument_id set.
        assert_credit_settled_once(
            &backend,
            &service_b,
            &tenant,
            run_id,
            &format!("crash point {point:?}"),
        )
        .await;

        // The index holds each command chunk once. The double's map key is
        // (tenant, index_id, entry_id), and `entry_id` is derived
        // deterministically from the chunk's own `IndexEntryKey` bytes, so
        // a chunk applied twice (a crashed attempt's apply, then the
        // retry's re-apply of the same still-`pending` command) can only
        // ever occupy one map slot -- `entry_count` equal to the command's
        // own chunk count is exactly the proof that no chunk was inserted
        // twice, independent of how many times `upsert` was actually
        // called for it.
        let score_outcome = outcomes
            .iter()
            .find(|outcome| outcome.phase == Phase::Score)
            .expect("Score outcome recorded");
        let score_evidence: ScoreEvidence =
            serde_json::from_value(score_outcome.evidence.clone()).unwrap();
        let final_run = service_b
            .store()
            .get_run(&tenant, run_id)
            .await
            .unwrap()
            .expect("run exists after completion");
        let command = service_b
            .load_index_command(&final_run, &score_evidence)
            .await
            .unwrap()
            .expect("Score proposed a command");
        let tenant_ref = pipeline_tenant_storage_ref(&tenant);
        assert_eq!(
            index.entry_count(tenant_ref.as_str(), MINIMAL_INDEX_ID),
            command.entries().len(),
            "the index must hold each command chunk exactly once at point {point:?}"
        );

        // The Settle decision's operations are Completed, with the helper
        // refs -- both legs actually settled (no withdrawal was injected),
        // so neither is Forfeited.
        let settle_outcome = outcomes
            .iter()
            .find(|outcome| outcome.phase == Phase::Settle)
            .expect("Settle outcome recorded");
        let decision: SettleDecision =
            serde_json::from_value(settle_outcome.decision.clone()).unwrap();
        let operations = decision.settlement_operations();
        assert_eq!(
            operations.len(),
            2,
            "one settlement operation per award at point {point:?}"
        );
        let storage_award = InstrumentAward::new(
            InstrumentId::new("storage_rebate").unwrap(),
            AtomicUnits::from_raw(5),
        )
        .unwrap();
        let credit_award = InstrumentAward::new(
            InstrumentId::trace_credit(),
            AtomicUnits::from_raw(1_000_000),
        )
        .unwrap();
        for operation in operations {
            let expected_award = if operation.instrument_id().as_str() == "storage_rebate" {
                &storage_award
            } else {
                &credit_award
            };
            match operation.outcome() {
                InstrumentSettlementOutcome::Completed { result_ref_hash } => {
                    assert_eq!(
                        result_ref_hash.as_str(),
                        pipeline_result_ref(run_id, expected_award).as_str(),
                        "unexpected result ref for {} at point {point:?}",
                        expected_award.instrument_id().as_str()
                    );
                }
                InstrumentSettlementOutcome::Forfeited { .. } => {
                    panic!("expected every leg Completed (no withdrawal) at point {point:?}")
                }
            }
        }
    }
}
