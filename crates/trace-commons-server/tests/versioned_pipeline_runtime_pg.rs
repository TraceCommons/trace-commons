// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Versioned pipeline runtime against PostgreSQL, as a role that cannot bypass RLS.

use std::sync::Arc;

use trace_commons_gate_api::pipeline::{
    AtomicUnits, InstrumentDescriptor, InstrumentKind, Phase, PhaseResult, ReasonCode,
    ReviewDecision, ReviewEvaluation, ReviewEvidence, ReviewOutput,
};
use trace_commons_gate_api::{ReferenceEmbedder, ReferencePerplexityScorer};
use trace_commons_server::config::DatabaseConfig;
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::versioned_pipeline::*;
use trace_commons_server::versioned_pipeline_bundle::{
    MinimalPolicyBundle, PipelineBundleConfig, PipelineInstrumentAwardConfig,
    dependency_content_hash,
};

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
    store
        .commit_phase(&claimed, stored, None, None)
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
