// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Versioned pipeline runtime against PostgreSQL, as a role that cannot bypass RLS.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use base64::Engine;
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
    Embedder, IdentifiedEmbedder, IndexEntryKey, IndexUpsertResult, ReferenceEmbedder,
    ReferencePerplexityScorer, SettlementAdapter, SettlementError, SettlementReceipt,
    SettlementRequest, VectorIndexWriter,
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
    EncryptedTraceArtifact, EncryptedTraceArtifactReceipt, LocalEncryptedTraceArtifactStore,
    PreparedSerializedJsonArtifact, TraceArtifactKind, TraceArtifactStore,
};
use trace_commons_server::trace_corpus_storage::{
    TraceCorpusStore, TraceCreditHoldReason, TraceCreditHoldWrite, TraceObjectArtifactKind,
    TraceObjectRefWrite,
};
use trace_commons_server::versioned_pipeline::*;
use trace_commons_server::versioned_pipeline_bundle::{
    MINIMAL_INDEX_ID, MINIMAL_PROJECTION_ID, MinimalPolicyBundle, PipelineBundleConfig,
    PipelineInstrumentAwardConfig, dependency_content_hash, pipeline_operation_ref,
    pipeline_result_ref,
};
use trace_commons_server::versioned_pipeline_credit::{
    RecordingSettlementAdapter, SettlementAdapterRegistry, credit_account_hash,
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

/// The token-only fence. `record_lease_expired` is fenced by the lease
/// token alone, with no expiry predicate (unlike every other lease-checked
/// write) -- but once another claim has moved the run onto a new token, the
/// old token no longer matches and this changes nothing, the same as any
/// other lease-checked write losing a race to a reclaim.
#[tokio::test]
async fn record_lease_expired_on_a_reclaimed_run_changes_nothing() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let store = PgPipelineStore::new(backend.clone());
    let tenant = format!("lease-expired-fence-{}", uuid::Uuid::new_v4());
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

    let unchanged = store.record_lease_expired(&first).await.unwrap();
    assert!(
        unchanged.is_none(),
        "the old lease token no longer matches the reclaimed run"
    );

    let current = store.get_run(&tenant, run.run_id).await.unwrap().unwrap();
    assert_eq!(current.lease_token, second.lease_token);
    assert_eq!(current.state, PipelineRunState::Leased);
}

/// A Score phase
/// slower than Review's and Settle's short lease still completes when Score
/// has its own longer configured lease -- scaled to a couple of seconds,
/// never a real 30-second sleep. Review and Settle get 1
/// second, Score gets a few seconds, and the embedder's delay sits strictly
/// between the two (1s < 2s < 4s) -- a fixed margin that does not depend on
/// how many 256-byte chunks the fixture happens to produce (`SlowEmbedder`
/// sleeps once per attempt, not once per chunk).
#[tokio::test]
async fn a_slow_score_completes_under_its_own_longer_configured_lease() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let lease_config = PipelineLeaseConfig::new(
        chrono::Duration::seconds(1),
        chrono::Duration::seconds(4),
        chrono::Duration::seconds(1),
    )
    .expect("1s review/settle, 4s score is in bounds");
    let embedder = Arc::new(SlowEmbedder::new(std::time::Duration::from_millis(2_000)));
    let service = score_lease_test_service(
        backend.clone(),
        artifact_store(&dir),
        lease_config,
        embedder,
    )
    .await;

    let tenant = format!("slow-score-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
            .await
            .unwrap()
    else {
        panic!("receipt creates a run")
    };

    let reviewed = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Review completes under its own 1-second lease");
    assert_eq!(reviewed.next_phase, Some(Phase::Score));
    assert_ne!(
        reviewed.last_error_label.as_deref(),
        Some(PIPELINE_LEASE_EXPIRED_LABEL)
    );

    let scored = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Score completes under its own longer lease despite the slow embedder");
    assert_eq!(scored.next_phase, Some(Phase::Settle));
    assert_ne!(
        scored.last_error_label.as_deref(),
        Some(PIPELINE_LEASE_EXPIRED_LABEL)
    );

    let settled = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Settle completes under its own 1-second lease");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_ne!(
        settled.last_error_label.as_deref(),
        Some(PIPELINE_LEASE_EXPIRED_LABEL)
    );
}

/// A Score phase whose lease expires records the expiry as its own
/// uncharged reason, never as a policy failure -- `record_lease_expired`
/// always gives back the attempt the claim took, so `attempt_count` never
/// climbs toward `max_attempts` no matter how many times the phase overruns,
/// and the run stays retryable rather than ever reaching
/// `failed`/`attempts_exhausted`. `SlowEmbedder` sleeps
/// once per attempt (`reset_for_next_attempt` between iterations below), so
/// each of the 8 attempts here costs about 1.5 s of real sleep, not the
/// fixture's chunk count times 1.5 s.
#[tokio::test]
async fn a_score_lease_that_always_expires_records_the_expiry_and_never_exhausts() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let lease_config = PipelineLeaseConfig::new(
        chrono::Duration::seconds(1),
        chrono::Duration::seconds(1),
        chrono::Duration::seconds(1),
    )
    .expect("1s/1s/1s is in bounds");
    let embedder = Arc::new(SlowEmbedder::new(std::time::Duration::from_millis(1_500)));
    let service = score_lease_test_service(
        backend.clone(),
        artifact_store(&dir),
        lease_config,
        embedder.clone(),
    )
    .await;

    let tenant = format!("expired-score-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
            .await
            .unwrap()
    else {
        panic!("receipt creates a run")
    };

    let reviewed = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Review completes under its own 1-second lease");
    assert_eq!(reviewed.next_phase, Some(Phase::Score));
    let attempt_count_before = reviewed.attempt_count;

    let expired = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("the stale Score lease is recorded, not propagated as an error");
    assert_eq!(expired.state, PipelineRunState::Retry);
    assert_eq!(expired.next_phase, Some(Phase::Score));
    assert_eq!(
        expired.last_error_label.as_deref(),
        Some(PIPELINE_LEASE_EXPIRED_LABEL)
    );
    assert_eq!(expired.attempt_count, attempt_count_before);
    assert!(expired.next_attempt_at > chrono::Utc::now());

    // The run's max_attempts defaults to 5 (migration V93); loop more
    // times than that and confirm the run is never failed /
    // attempts_exhausted.
    for _ in 0..7 {
        force_due(&backend, &tenant, created.run_id).await;
        embedder.reset_for_next_attempt();
        let retried = service
            .process_run(&tenant, created.run_id)
            .await
            .unwrap()
            .expect("the run stays claimable every time its lease expires again");
        assert_eq!(retried.state, PipelineRunState::Retry);
        assert_eq!(
            retried.last_error_label.as_deref(),
            Some(PIPELINE_LEASE_EXPIRED_LABEL)
        );
    }
}

/// The stale-lease-vs-charged-failure case B: the phase itself fails for
/// an ordinary reason (a transient `PolicyError`, not a lease problem) --
/// but the
/// *follow-up* `mark_transient_retry` call finds the lease already gone,
/// because the phase ran right up to (and past) its own lease's edge before
/// it failed. `process_claimed_run`'s bundle-load branch and every
/// generic-dispatch branch used to call the store directly here and let a
/// stale-lease error from that follow-up write propagate raw; this proves
/// each now records the expiry instead, uncharged, the same as when the
/// phase's own commit finds the lease gone (case A, tested above).
#[tokio::test]
async fn a_stale_lease_found_by_the_follow_up_mark_call_is_recorded_not_charged() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let lease_config = PipelineLeaseConfig::new(
        chrono::Duration::seconds(1),
        chrono::Duration::seconds(1),
        chrono::Duration::seconds(1),
    )
    .expect("1s/1s/1s is in bounds");
    let embedder = Arc::new(SlowThenFailingEmbedder {
        delay: std::time::Duration::from_millis(1_500),
    });
    let service = score_lease_test_service(
        backend.clone(),
        artifact_store(&dir),
        lease_config,
        embedder,
    )
    .await;

    let tenant = format!("case-b-lease-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
            .await
            .unwrap()
    else {
        panic!("receipt creates a run")
    };

    let reviewed = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Review completes under its own 1-second lease");
    assert_eq!(reviewed.next_phase, Some(Phase::Score));
    let attempt_count_before = reviewed.attempt_count;

    let expired = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect(
            "the stale lease the follow-up mark_transient_retry finds is recorded, not propagated",
        );
    assert_eq!(expired.state, PipelineRunState::Retry);
    assert_eq!(expired.next_phase, Some(Phase::Score));
    assert_eq!(
        expired.last_error_label.as_deref(),
        Some(PIPELINE_LEASE_EXPIRED_LABEL)
    );
    assert_ne!(
        expired.last_error_label.as_deref(),
        Some("embedder_unavailable")
    );
    assert_eq!(expired.attempt_count, attempt_count_before);
    assert!(expired.next_attempt_at > chrono::Utc::now());
}

/// `claim_next` -- the tenant-wide claim `process_one`
/// uses, as opposed to `claim_run`'s explicit-duration claim -- picks the
/// lease by the claimed row's own `next_phase` in the claiming SQL itself,
/// so a Score row never gets Review's lease or vice versa. Drives the same
/// run through all three phases with three different configured lease
/// lengths and checks each claim's granted `lease_expires_at` against the
/// phase it actually claimed.
#[tokio::test]
async fn claim_next_picks_the_lease_by_the_runs_next_phase() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let store = PgPipelineStore::new(backend.clone());
    let tenant = format!("claim-next-lease-{}", uuid::Uuid::new_v4());
    let run = seed_run(&backend, &tenant, uuid::Uuid::new_v4()).await;
    let lease_config = PipelineLeaseConfig::new(
        chrono::Duration::seconds(11),
        chrono::Duration::seconds(1_234),
        chrono::Duration::seconds(22),
    )
    .expect("11s/1234s/22s is in bounds");

    let claimed_review = store
        .claim_next(&tenant, lease_config)
        .await
        .unwrap()
        .expect("the seeded run is due for Review");
    assert_eq!(claimed_review.next_phase, Some(Phase::Review));
    let review_seconds =
        (claimed_review.lease_expires_at.unwrap() - claimed_review.updated_at).num_seconds();
    assert!(
        (10..=11).contains(&review_seconds),
        "expected ~11s, got {review_seconds}s"
    );

    // Force the run straight to Score, pending and due -- a raw update, not
    // a real Review commit: this test is only about which lease `claim_next`
    // grants a row by its `next_phase`, not about getting there through a
    // real phase transition.
    advance_run_to_phase(&backend, &tenant, run.run_id, "score").await;
    let claimed_score = store
        .claim_next(&tenant, lease_config)
        .await
        .unwrap()
        .expect("the run is now due for Score");
    assert_eq!(claimed_score.next_phase, Some(Phase::Score));
    let score_seconds =
        (claimed_score.lease_expires_at.unwrap() - claimed_score.updated_at).num_seconds();
    assert!(
        (1_233..=1_234).contains(&score_seconds),
        "expected ~1234s, got {score_seconds}s"
    );

    advance_run_to_phase(&backend, &tenant, run.run_id, "settle").await;
    let claimed_settle = store
        .claim_next(&tenant, lease_config)
        .await
        .unwrap()
        .expect("the run is now due for Settle");
    assert_eq!(claimed_settle.next_phase, Some(Phase::Settle));
    let settle_seconds =
        (claimed_settle.lease_expires_at.unwrap() - claimed_settle.updated_at).num_seconds();
    assert!(
        (21..=22).contains(&settle_seconds),
        "expected ~22s, got {settle_seconds}s"
    );
}

/// Test-only: forces `run_id` to `phase`, `pending`, and due now, with no
/// lease -- so the next `claim_next`/`claim_run` claims it fresh under
/// whatever lease its new `next_phase` earns. Used only to set up which
/// phase a claim will see; never a stand-in for a real phase commit.
async fn advance_run_to_phase(
    backend: &PgBackend,
    tenant_id: &str,
    run_id: uuid::Uuid,
    phase: &str,
) {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for advance_run_to_phase");
    let tx = tenant_tx(&mut client, tenant_id).await;
    tx.execute(
        "UPDATE pipeline_runs
            SET next_phase = $3, state = 'pending', lease_token = NULL,
                lease_expires_at = NULL, next_attempt_at = NOW()
          WHERE tenant_id = $1 AND run_id = $2",
        &[&tenant_id, &run_id, &phase],
    )
    .await
    .expect("advance run to phase");
    tx.commit().await.expect("commit advance_run_to_phase");
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

    // BND-005: one tenant's registrations do not constrain another tenant.
    // The exact package refused above for `tenant` is accepted for a second,
    // unrelated tenant that has never registered anything.
    let other_tenant = format!("bundle-registry-{}", uuid::Uuid::new_v4());
    store
        .register_bundle(&other_tenant, &conflicting)
        .await
        .expect("a different tenant may register the descriptor the first tenant refused");

    // Registering the same package again stays idempotent.
    store
        .register_bundle(&tenant, &first)
        .await
        .expect("re-registering the same package is idempotent");
}

/// PR #971 round 3: `pipeline_run_settlements` bounds every row's
/// `atomic_units` to `u128::MAX` (`pipeline_run_settlements_atomic_units_bound`),
/// and additionally bounds a `trace_credit` row to `i64::MAX`
/// (`pipeline_run_settlements_trace_credit_bound`). `AtomicUnits` already
/// refuses a value above `u128::MAX` on load, so the database CHECK is a
/// backstop a Rust caller cannot trigger through the typed API; this test
/// goes around it with raw SQL to prove the backstop itself.
#[tokio::test]
async fn settlement_amounts_are_bounded_by_the_database() {
    let Some(backend) = runtime_backend(2).await else {
        return;
    };
    let tenant = format!("settlement-bound-{}", uuid::Uuid::new_v4());
    let run = seed_run(&backend, &tenant, uuid::Uuid::new_v4()).await;

    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for settlement bound test");

    // Each assertion below runs in its own transaction: a CHECK violation
    // aborts the transaction it occurs in, so a failing insert must not
    // share a transaction with the assertion that follows it.
    async fn open_tx<'a>(
        client: &'a mut deadpool_postgres::Client,
        tenant_id: &str,
    ) -> deadpool_postgres::Transaction<'a> {
        let tx = client
            .transaction()
            .await
            .expect("tx for settlement bound test");
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant_id],
        )
        .await
        .expect("set tenant for settlement bound test");
        tx
    }

    const INSERT: &str = "INSERT INTO pipeline_run_settlements (
        tenant_id, run_id, instrument_id, atomic_units, operation_ref_hash, payout_rail
    ) VALUES ($1,$2,$3,$4::TEXT::NUMERIC,$5,'none')";
    const U128_MAX: &str = "340282366920938463463374607431768211455";
    const U128_MAX_PLUS_ONE: &str = "340282366920938463463374607431768211456";
    const I64_MAX_PLUS_ONE: &str = "9223372036854775808";

    // A non-`trace_credit` row at exactly `u128::MAX` is accepted.
    let tx = open_tx(&mut client, &tenant).await;
    tx.execute(
        INSERT,
        &[
            &tenant,
            &run.run_id,
            &"storage_rebate",
            &U128_MAX,
            &dependency_content_hash(b"settlement-bound-at-u128-max"),
        ],
    )
    .await
    .expect("a non-trace_credit row at u128::MAX is accepted");
    tx.rollback().await.expect("rollback settlement bound tx");

    // One atomic unit over `u128::MAX` is refused by the new bound.
    let tx = open_tx(&mut client, &tenant).await;
    let over_u128 = tx
        .execute(
            INSERT,
            &[
                &tenant,
                &run.run_id,
                &"storage_rebate_over",
                &U128_MAX_PLUS_ONE,
                &dependency_content_hash(b"settlement-bound-over-u128-max"),
            ],
        )
        .await
        .expect_err("a non-trace_credit row over u128::MAX is refused");
    let db = over_u128.as_db_error().expect("database refusal");
    assert_eq!(db.code(), &tokio_postgres::error::SqlState::CHECK_VIOLATION);
    assert_eq!(
        db.constraint(),
        Some("pipeline_run_settlements_atomic_units_bound"),
        "refused by the wrong constraint: {db:?}"
    );
    tx.rollback().await.expect("rollback settlement bound tx");

    // A `trace_credit` row one unit over `i64::MAX` stays within
    // `u128::MAX` but is refused by the tighter Trace Credit bound.
    let tx = open_tx(&mut client, &tenant).await;
    let over_trace_credit = tx
        .execute(
            INSERT,
            &[
                &tenant,
                &run.run_id,
                &"trace_credit",
                &I64_MAX_PLUS_ONE,
                &dependency_content_hash(b"settlement-bound-over-trace-credit"),
            ],
        )
        .await
        .expect_err("a trace_credit row over i64::MAX is refused");
    let db = over_trace_credit.as_db_error().expect("database refusal");
    assert_eq!(db.code(), &tokio_postgres::error::SqlState::CHECK_VIOLATION);
    assert_eq!(
        db.constraint(),
        Some("pipeline_run_settlements_trace_credit_bound"),
        "refused by the wrong constraint: {db:?}"
    );
    tx.rollback().await.expect("rollback settlement bound tx");
}

/// Corrupts a stored package's pinned `network` for one instrument, as an
/// owner connection with the immutability trigger dropped and recreated
/// exactly like `tamper_stored_bundle_package`. Unlike that helper, this
/// keeps the artifact bytes intact and instead breaks the manifest itself,
/// so a later load fails `BundleManifest`'s own deserialize-time validation
/// (#971 round 3) rather than `BundlePackage::validate`'s artifact-hash
/// check.
async fn tamper_stored_bundle_manifest_network(
    tenant_id: &str,
    bundle_id: &str,
    instrument_id: &str,
    network: &str,
) {
    tamper_stored_bundle_manifest(tenant_id, bundle_id, |manifest| {
        manifest["instruments"][instrument_id]["network"] =
            serde_json::Value::String(network.to_string());
    })
    .await;
}

/// Rewrites a stored package's manifest JSON with `edit`, as an owner
/// connection with the immutability trigger dropped and recreated exactly
/// like `tamper_stored_bundle_package`, keeping the artifact bytes intact.
async fn tamper_stored_bundle_manifest(
    tenant_id: &str,
    bundle_id: &str,
    edit: impl FnOnce(&mut serde_json::Value),
) {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .expect("TRACE_COMMONS_PG_TEST_DATABASE_URL must be set for this test");
    let (mut client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
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
    edit(&mut package["manifest"]);

    tx.execute(
        "UPDATE pipeline_bundle_packages SET package = $3
         WHERE tenant_id = $1 AND bundle_id = $2",
        &[&tenant_id, &bundle_id, &package],
    )
    .await
    .expect("tamper the stored manifest");

    tx.batch_execute(
        "CREATE TRIGGER pipeline_bundle_packages_reject_update
             BEFORE UPDATE ON pipeline_bundle_packages
             FOR EACH ROW EXECUTE FUNCTION reject_pipeline_bundle_package_mutation();",
    )
    .await
    .expect("recreate the immutability trigger");

    tx.commit().await.expect("commit the tampering transaction");
}

/// Packages build at the current manifest format version (2), and a stored
/// package at an earlier one no longer loads. That package fails closed the
/// way any package that no longer loads does: a registration for the tenant
/// is refused while it is on file, and a run bound to it fails as
/// `bundle_package_invalid` with no new outcome.
#[tokio::test]
async fn a_stored_package_at_an_earlier_format_version_fails_closed() {
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
    let tenant = format!("earlier-format-{}", uuid::Uuid::new_v4());

    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
            .await
            .unwrap()
    else {
        panic!("receipt creates a run")
    };
    let stored = service
        .store()
        .load_bundle(&tenant, &created.bundle_id)
        .await
        .unwrap()
        .expect("the bound package is stored");
    assert_eq!(stored.manifest.format_version, 2);
    assert_eq!(
        stored.manifest.format_version,
        trace_commons_gate_api::pipeline::BUNDLE_MANIFEST_FORMAT_VERSION
    );

    tamper_stored_bundle_manifest(&tenant, &created.bundle_id, |manifest| {
        manifest["format_version"] = serde_json::Value::from(1);
    })
    .await;
    let error = service
        .store()
        .load_bundle(&tenant, &created.bundle_id)
        .await
        .expect_err("a package at an earlier format version does not load");
    assert!(error.to_string().contains("bundle_package_invalid"));

    let fresh = MinimalPolicyBundle::minimal_package(
        &PipelineBundleConfig {
            instrument_awards: vec![PipelineInstrumentAwardConfig {
                instrument_id: "storage_rebate".into(),
                atomic_units: AtomicUnits::from_raw(5),
                descriptor: storage_rebate_descriptor(),
            }],
            include_index: false,
            variant: Some("fresh".to_string()),
        },
        &ReferencePerplexityScorer::new(),
        &ReferenceEmbedder::new(),
    )
    .expect("build package");
    let refused = service
        .store()
        .register_bundle(&tenant, &fresh)
        .await
        .expect_err("a registration is refused while a stored package no longer loads");
    assert!(refused.to_string().contains("bundle_package_invalid"));

    let outcomes_before = service
        .store()
        .list_outcomes(&tenant, created.run_id)
        .await
        .unwrap();
    let failed = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("the run fails closed on a package at an earlier format version");
    assert_eq!(failed.state, PipelineRunState::Failed);
    assert_eq!(
        failed.last_error_label.as_deref(),
        Some("bundle_package_invalid")
    );
    assert_eq!(
        service
            .store()
            .list_outcomes(&tenant, created.run_id)
            .await
            .unwrap()
            .len(),
        outcomes_before.len(),
        "no new outcome is recorded"
    );
}

/// The per-tenant descriptor-conflict check in `register_bundle` reads
/// every package already registered for the tenant. #971 round 3 tightened
/// `BundleManifest`'s deserialize-time validation, so a package registered
/// before the tightening can stop loading under the new rule. The conflict
/// check cannot compare against a package it cannot deserialize, so it fails
/// closed: the tenant's other registrations are refused rather than silently
/// proceeding past a control the call could not evaluate.
#[tokio::test]
async fn register_bundle_refuses_while_a_registered_package_no_longer_loads() {
    let Some(backend) = runtime_backend(2).await else {
        return;
    };
    let store = PgPipelineStore::new(backend.clone());
    let tenant = format!("bundle-registry-stale-{}", uuid::Uuid::new_v4());
    let scorer = ReferencePerplexityScorer::new();
    let embedder = ReferenceEmbedder::new();

    let package_for = |variant: &str, instrument_id: &str, descriptor: InstrumentDescriptor| {
        MinimalPolicyBundle::minimal_package(
            &PipelineBundleConfig {
                instrument_awards: vec![PipelineInstrumentAwardConfig {
                    instrument_id: instrument_id.into(),
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

    // A `nep141` descriptor, inlined here (not `trace_credit_descriptor`,
    // which the Score tests add later): six decimals, `testnet`, so it is
    // valid on the round-3 rule this test is about to violate.
    let nep141_trace_credit = InstrumentDescriptor {
        kind: InstrumentKind::Nep141,
        network: "testnet".to_string(),
        contract: "trace-credit.testnet".to_string(),
        decimals: 6,
    };
    let stale = package_for("stale", "trace_credit", nep141_trace_credit);
    store
        .register_bundle(&tenant, &stale)
        .await
        .expect("register the package that will be corrupted");

    // Corrupt it in place: the network no longer satisfies the round-3
    // `nep141` rule (only `mainnet` or `testnet`), so the stored package no
    // longer deserializes at all.
    tamper_stored_bundle_manifest_network(
        &tenant,
        &stale.bundle_id,
        "trace_credit",
        "near-mainnet",
    )
    .await;

    // A new package naming a different instrument is refused: the conflict
    // check cannot compare against the corrupted package, so it fails closed
    // rather than registering past a control it cannot evaluate.
    let fresh = package_for("fresh", "storage_rebate", storage_rebate_descriptor());
    let error = store
        .register_bundle(&tenant, &fresh)
        .await
        .expect_err("a registration is refused while a registered package no longer loads");
    assert!(error.to_string().contains("bundle_package_invalid"));

    let stored_fresh = store
        .load_bundle(&tenant, &fresh.bundle_id)
        .await
        .expect("load after the refused registration");
    assert!(
        stored_fresh.is_none(),
        "no row is added when registration is refused"
    );

    // Another tenant, unaffected by the first tenant's corrupted package,
    // can still register the same descriptor.
    let other_tenant = format!("bundle-registry-stale-{}", uuid::Uuid::new_v4());
    store
        .register_bundle(&other_tenant, &fresh)
        .await
        .expect("a different tenant is not blocked by another tenant's corrupted package");
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
/// (`pipeline_bundle_packages_reject_update`, migration V93), so this drops
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
///
/// Ages by whole microseconds, not milliseconds: the database stores
/// timestamps with microsecond resolution, and a backoff delay read back
/// from the database can carry a sub-millisecond remainder. Rounding `by`
/// down to the nearest millisecond would throw that remainder away and
/// leave `next_attempt_at` a fraction of a millisecond later than intended,
/// which can still read as due in the future depending on how much real
/// time passes before the next claim.
async fn age_run(backend: &PgBackend, tenant_id: &str, run_id: uuid::Uuid, by: chrono::Duration) {
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .unwrap();
    let microseconds = by
        .num_microseconds()
        .expect("age_run: duration does not fit in microseconds");
    tx.execute(
        "UPDATE pipeline_runs
            SET phase_started_at = phase_started_at - ($3::bigint * INTERVAL '1 microsecond'),
                next_attempt_at = next_attempt_at - ($3::bigint * INTERVAL '1 microsecond')
          WHERE tenant_id = $1 AND run_id = $2",
        &[&tenant_id, &run_id, &microseconds],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// `mark_transient_retry` schedules the next attempt after the
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
    test_service_with_adapters_and_caps(
        backend,
        artifact_store,
        config,
        adapters,
        uncapped_caps(&["storage_rebate", InstrumentId::trace_credit().as_str()]),
    )
    .await
}

/// A `u128::MAX` cap -- in effect no limit -- for each named instrument, and
/// no cap at all for any other.
fn uncapped_caps(instruments: &[&str]) -> PipelineCaps {
    PipelineCaps {
        per_instrument_atomic_units: instruments
            .iter()
            .map(|instrument| (instrument.to_string(), AtomicUnits::from_raw(u128::MAX)))
            .collect(),
    }
}

/// Like `test_service_with_adapters`, but with the per-instrument caps
/// given, for a test that needs a cap missing or below an award.
async fn test_service_with_adapters_and_caps(
    backend: Arc<PgBackend>,
    artifact_store: Arc<dyn TraceArtifactStore>,
    config: PipelineBundleConfig,
    adapters: Vec<Arc<dyn SettlementAdapter>>,
    caps: PipelineCaps,
) -> Arc<PipelineService> {
    let scorer = Arc::new(ReferencePerplexityScorer::new());
    let embedder = Arc::new(ReferenceEmbedder::new());
    let package = MinimalPolicyBundle::minimal_package(&config, scorer.as_ref(), embedder.as_ref())
        .expect("build minimal bundle package");
    let index = IsolatedPipelineIndex::new();
    let registry =
        SettlementAdapterRegistry::new(adapters).expect("build settlement adapter registry");
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

/// An embedder whose `embed` call blocks for a fixed wall-clock delay
/// once per simulated attempt -- not once per chunk --
/// before delegating to the reference embedder. `Embedder::embed` is
/// synchronous, so `std::thread::sleep` inside it is a real elapsed delay a
/// claimed lease's `lease_expires_at` genuinely runs past, without a real
/// 30-second sleep (this test is scaled down) and without the
/// total delay growing with the fixture's chunk count: only the first
/// `embed` call since construction or since the last
/// `reset_for_next_attempt` sleeps, so a test driving several simulated
/// attempts controls the total delay directly rather than as a function of
/// how many 256-byte chunks the reviewed artifact happens to produce.
struct SlowEmbedder {
    delay: std::time::Duration,
    slept_this_attempt: AtomicBool,
}

impl SlowEmbedder {
    fn new(delay: std::time::Duration) -> Self {
        Self {
            delay,
            slept_this_attempt: AtomicBool::new(false),
        }
    }

    /// Test-only: call before driving the next simulated attempt so its
    /// first chunk sleeps again. Without this, only the very first `embed`
    /// call across every attempt this double ever serves would sleep.
    fn reset_for_next_attempt(&self) {
        self.slept_this_attempt.store(false, Ordering::SeqCst);
    }
}

impl Embedder for SlowEmbedder {
    fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
        if !self.slept_this_attempt.swap(true, Ordering::SeqCst) {
            std::thread::sleep(self.delay);
        }
        ReferenceEmbedder::new().embed(plaintext)
    }
}

impl IdentifiedEmbedder for SlowEmbedder {
    fn dependency_identity(&self) -> &str {
        "slow_embedder_test_only"
    }

    fn model_id(&self) -> &str {
        "slow-embedder-v1"
    }

    fn content_descriptor(&self) -> Vec<u8> {
        b"slow-embedder-test-descriptor-v1".to_vec()
    }
}

/// The stale-lease-vs-charged-failure case B's embedder double: sleeps a
/// fixed wall-clock delay and then fails, every call. `FixedScorePolicy`'s
/// chunk loop aborts on the first
/// `embed` error (the same reason `FlakyEmbedder` above needs no per-attempt
/// reset), so this always sleeps exactly once per simulated attempt without
/// needing `SlowEmbedder`'s reset bookkeeping. Used to make the *phase*
/// raise an ordinary transient `PolicyError` -- not a lease problem -- after
/// its lease has already gone stale, so it is the follow-up
/// `mark_transient_retry` call, not the phase's own commit, that discovers
/// the lease is gone.
struct SlowThenFailingEmbedder {
    delay: std::time::Duration,
}

impl Embedder for SlowThenFailingEmbedder {
    fn embed(&self, _plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
        std::thread::sleep(self.delay);
        anyhow::bail!("embedder dependency outage (test double)")
    }
}

impl IdentifiedEmbedder for SlowThenFailingEmbedder {
    fn dependency_identity(&self) -> &str {
        "slow_then_failing_embedder_test_only"
    }

    fn model_id(&self) -> &str {
        "slow-then-failing-embedder-v1"
    }

    fn content_descriptor(&self) -> Vec<u8> {
        b"slow-then-failing-embedder-test-descriptor-v1".to_vec()
    }
}

/// Builds a service over `minimal_config(true)` (Score's embedder runs, and
/// there are no instrument awards, so Settle needs no adapter dispatch) with
/// `embedder` and the given lease configuration. Shared by the
/// lease-expiry tests below; the caller keeps its own `Arc` to the embedder
/// double so it can call `reset_for_next_attempt` (for `SlowEmbedder`)
/// between simulated attempts.
async fn score_lease_test_service(
    backend: Arc<PgBackend>,
    artifact_store: Arc<dyn TraceArtifactStore>,
    lease_config: PipelineLeaseConfig,
    embedder: Arc<dyn IdentifiedEmbedder>,
) -> Arc<PipelineService> {
    let scorer = Arc::new(ReferencePerplexityScorer::new());
    let package = MinimalPolicyBundle::minimal_package(
        &minimal_config(true),
        scorer.as_ref(),
        embedder.as_ref(),
    )
    .expect("build minimal bundle package");
    let index = IsolatedPipelineIndex::new();
    let registry =
        SettlementAdapterRegistry::new(Vec::new()).expect("build settlement adapter registry");
    let caps = PipelineCaps {
        per_instrument_atomic_units: BTreeMap::new(),
    };
    let service = PipelineServiceBuilder::new(
        backend,
        artifact_store,
        package,
        index.clone(),
        index,
        registry,
        caps,
    )
    .with_scorer(scorer)
    .with_embedder(embedder)
    .with_lease_config(lease_config)
    .build()
    .expect("build pipeline service");
    Arc::new(service)
}

/// P5's mismatching adapter double: always returns a well-formed but wrong
/// result reference, regardless of what the request expects. Proves Settle
/// fails the row closed on its binding check -- the expected result comes
/// from the persisted selection, never trusted from whatever the adapter
/// hands back -- rather than accepting a plausible-looking but different
/// result.
struct MismatchingSettlementAdapter {
    instrument_id: InstrumentId,
}

#[async_trait::async_trait]
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

    async fn settle(
        &self,
        _request: &SettlementRequest,
    ) -> Result<SettlementReceipt, SettlementError> {
        Ok(SettlementReceipt::internal(format!("sha256:{}", "f".repeat(64))).unwrap())
    }
}

/// An adapter that answers every call with the same `SettlementError`: a
/// `Conflict` or `Rejected` says no effect happened and the operation must
/// not be sent again.
struct RefusingSettlementAdapter {
    instrument_id: InstrumentId,
    error: SettlementError,
}

#[async_trait::async_trait]
impl SettlementAdapter for RefusingSettlementAdapter {
    fn instrument_id(&self) -> &InstrumentId {
        &self.instrument_id
    }

    fn adapter_identity(&self) -> &str {
        "refusing_test_only"
    }

    fn payout_rail(&self) -> &str {
        "none"
    }

    async fn settle(
        &self,
        _request: &SettlementRequest,
    ) -> Result<SettlementReceipt, SettlementError> {
        Err(self.error)
    }
}

/// An adapter whose effect has an external record: it delegates to a
/// recording adapter (so a repeated operation is one logical effect) and
/// answers with an external receipt whose hash is `receipt_hash`.
struct ExternalReceiptAdapter {
    inner: Arc<RecordingSettlementAdapter>,
    receipt_hash: String,
}

#[async_trait::async_trait]
impl SettlementAdapter for ExternalReceiptAdapter {
    fn instrument_id(&self) -> &InstrumentId {
        self.inner.instrument_id()
    }

    fn adapter_identity(&self) -> &str {
        "external_receipt_test_only"
    }

    fn payout_rail(&self) -> &str {
        "none"
    }

    async fn settle(
        &self,
        request: &SettlementRequest,
    ) -> Result<SettlementReceipt, SettlementError> {
        let receipt = self.inner.settle(request).await?;
        Ok(
            SettlementReceipt::external(receipt.result_ref_hash(), self.receipt_hash.clone())
                .unwrap(),
        )
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
struct InterruptingCreditAdapter {
    inner: Arc<RecordingSettlementAdapter>,
    backend: Arc<PgBackend>,
    tenant_id: String,
    interruption: CreditInterruption,
    armed: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
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

    async fn settle(
        &self,
        request: &SettlementRequest,
    ) -> Result<SettlementReceipt, SettlementError> {
        let receipt = self.inner.settle(request).await?;
        if self.armed.swap(false, Ordering::SeqCst) {
            let tenant_id = self.tenant_id.clone();
            let run_id = request.run_id();
            match &self.interruption {
                CreditInterruption::ExpireLease => {
                    let mut client = self.backend.trace_pool_for_test().get().await.unwrap();
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
                    self.backend
                        .upsert_trace_credit_hold(hold.clone())
                        .await
                        .unwrap();
                }
            }
        }
        Ok(receipt)
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

/// `submit`, but registering the request's tenant for the default bundle
/// first. Startup owns that registration now, not `submit` itself, so a
/// tenant this suite invents -- one startup never touched -- still needs an
/// active bundle before its first receipt. Every direct call this suite
/// makes to `submit` goes through this instead of calling
/// `register_default_bundle` inline at each call site.
async fn submit_registered(
    service: &PipelineService,
    request: PipelineReceiptRequest<'_>,
) -> anyhow::Result<PipelineReceiptResult> {
    service.register_default_bundle(request.tenant_id).await?;
    service.submit(request).await
}

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
/// quota-exceeded) never reaches the staging insert, which commits before
/// the object write, so this is also the count of receipt attempts that got
/// as far as storing an artifact.
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
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
            .await
            .unwrap()
    else {
        panic!("first receipt creates a run")
    };
    let PipelineReceiptResult::Replayed(replayed) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
            .await
            .unwrap()
    else {
        panic!("identical bytes replay")
    };
    assert_eq!(created.run_id, replayed.run_id);
    let mut changed = raw.clone();
    changed.push(b' ');
    assert!(matches!(
        submit_registered(&service, receipt(&tenant, &key, &changed, &env, NO_LIMITS))
            .await
            .unwrap(),
        PipelineReceiptResult::ContentConflict
    ));
    assert_eq!(count_runs(&backend, &tenant).await, 1);
}

/// `replay_receipt` is the read-only lookup `submit_trace_handler`'s
/// completed-admission branch uses for a retried upload for a
/// pipeline-routed tenant, instead of the legacy file record the pipeline
/// never writes. It must behave exactly like `submit`'s own replay check for
/// the same key/content -- `None` before any run exists, `Replayed` for
/// identical content, `ContentConflict` for different content -- while never
/// creating, staging, storing, or counting anything. It must also report the
/// principal `submit` recorded when it first created the run
/// (`trace_submissions.auth_principal_ref`), so a caller can check ownership
/// before acting on either outcome.
#[tokio::test]
async fn replay_receipt_reads_without_writing() {
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
    let tenant = format!("replay-receipt-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();

    // No run yet: a retry the pipeline has never seen for this key.
    assert!(
        service
            .replay_receipt(&tenant, &key, &raw)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(count_runs(&backend, &tenant).await, 0);

    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
            .await
            .unwrap()
    else {
        panic!("first receipt creates a run")
    };

    // Identical content: the same run, read-only, with the principal
    // `receipt()` submitted it under.
    let PipelineReplayReceipt {
        result,
        auth_principal_ref,
    } = service
        .replay_receipt(&tenant, &key, &raw)
        .await
        .unwrap()
        .expect("a run now exists for this key");
    assert_eq!(auth_principal_ref, "principal_sha256:test");
    let PipelineReceiptResult::Replayed(replayed) = result else {
        panic!("identical bytes replay")
    };
    assert_eq!(created.run_id, replayed.run_id);
    assert_eq!(count_runs(&backend, &tenant).await, 1);

    // Different content, same key: a conflict, not a second run. Still
    // reports the original principal, so a caller can distinguish a
    // conflict its own principal caused from one it did not.
    let mut changed = raw.clone();
    changed.push(b' ');
    let conflicted = service
        .replay_receipt(&tenant, &key, &changed)
        .await
        .unwrap()
        .expect("a run still exists for this key");
    assert_eq!(conflicted.auth_principal_ref, "principal_sha256:test");
    assert!(matches!(
        conflicted.result,
        PipelineReceiptResult::ContentConflict
    ));
    assert_eq!(count_runs(&backend, &tenant).await, 1);

    // A different key under the same tenant still has no run.
    assert!(
        service
            .replay_receipt(&tenant, "an-unrelated-key", &raw)
            .await
            .unwrap()
            .is_none()
    );
}

/// Seeds an unrelated prior submission for `tenant` and a tombstone on it
/// that matches `redaction_hash`. A tombstone can only reference an
/// existing submission (its foreign key), and in production it matches a
/// fresh resubmission by `trace_id` or `redaction_hash`.
async fn seed_redaction_tombstone(backend: &Arc<PgBackend>, tenant: &str, redaction_hash: &str) {
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
            &Some(redaction_hash.to_string()),
            &"withdrawn",
            &"seed-principal",
        ],
    )
    .await
    .expect("seed trace_tombstones");
    tx.commit().await.expect("commit tombstone seed");
}

/// A tombstone can only reference a submission that already exists (the
/// table's foreign key). In production a tombstone is always created for a
/// PRIOR submission -- the one that was later withdrawn or redacted -- and
/// matches a fresh resubmission by `trace_id`/`redaction_hash`, not by
/// `submission_id`. This test reproduces that shape: it seeds an unrelated
/// prior submission, tombstones it by the redaction hash the new envelope
/// carries, and submits the new envelope under a different submission id.
/// The receipt derives the submission's retention policy and expiry on
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
    let PipelineReceiptResult::Created(_) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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
/// cannot resolve it without a human assessment (PR 3), so the one Review
/// attempt it gets is the uncharged `review_assessment_required` suspension
/// -- but parked in `awaiting_review`, not retried. No claim ever selects
/// that state, so the run does no further work on its own however far the
/// clock moves; the route that moves it back to `pending` is not in this
/// code.
#[tokio::test]
async fn a_quarantined_receipt_waits_for_review_without_retrying() {
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
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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

    // The one Review attempt this run gets parks it, uncharged -- not a
    // retry with a backoff.
    let parked = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Review runs and parks the run awaiting a human assessment");
    assert_eq!(parked.state, PipelineRunState::AwaitingReview);
    assert_eq!(parked.next_phase, Some(Phase::Review));
    assert_eq!(
        parked.last_error_label.as_deref(),
        Some("review_assessment_required")
    );
    assert_eq!(parked.attempt_count, 0, "parking is not a charged attempt");
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

    // No claim selects `awaiting_review`, at any next_attempt_at and however
    // far the clock moves -- unlike a retry, this is not a matter of timing.
    force_due(&backend, &tenant, created.run_id).await;
    age_run(
        &backend,
        &tenant,
        created.run_id,
        chrono::Duration::hours(2),
    )
    .await;
    assert!(
        service
            .process_run(&tenant, created.run_id)
            .await
            .unwrap()
            .is_none(),
        "a run awaiting review is never claimed on its own"
    );
    let still_parked = service
        .store()
        .get_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("the run still exists");
    assert_eq!(still_parked.state, PipelineRunState::AwaitingReview);
    assert_eq!(still_parked.attempt_count, 0);
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
    seed_redaction_tombstone(&backend, &tenant, &env.privacy.redaction_hash).await;

    let result = submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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
            submit_registered(&service, receipt(&tenant, &key, &raw, &env, limits))
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
        submit_registered(&service, receipt("pool-one", &key, &raw, &env, NO_LIMITS)),
    )
    .await
    .expect("a receipt with one pool connection must not wait on itself");
    assert!(matches!(
        result.unwrap(),
        PipelineReceiptResult::Created(_) | PipelineReceiptResult::Replayed(_)
    ));
}

/// A tenant `register_default_bundle` has never touched has no active
/// bundle; calling it once registers the service's default package and
/// activates it, so the tenant is ready for its first receipt without
/// `submit` having to do any of that work itself.
#[tokio::test]
async fn register_default_bundle_activates_it_for_a_fresh_tenant() {
    let Some(backend) = runtime_backend(2).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _, _) =
        test_service(backend, artifact_store(&dir), minimal_config(false), None).await;
    let tenant = format!("register-default-{}", uuid::Uuid::new_v4());

    assert!(
        service
            .store()
            .active_bundle_id(&tenant)
            .await
            .unwrap()
            .is_none(),
        "a fresh tenant has no active bundle before registration"
    );

    service
        .register_default_bundle(&tenant)
        .await
        .expect("register and activate the default bundle");

    let active = service
        .store()
        .active_bundle_id(&tenant)
        .await
        .unwrap()
        .expect("the tenant has an active bundle after registration");
    assert_eq!(active, service.bundle_id());

    // Idempotent: calling it again for a tenant that already has an active
    // bundle changes nothing and still succeeds.
    service
        .register_default_bundle(&tenant)
        .await
        .expect("register_default_bundle is idempotent");
    assert_eq!(
        service.store().active_bundle_id(&tenant).await.unwrap(),
        Some(service.bundle_id().to_string())
    );
}

/// `submit` never asks the bundle registry to register anything -- that is
/// startup's job now (`register_default_bundle`), called once per rollout
/// tenant before any receipt. Proven here by putting the registry into a
/// state where registering this service's own default bundle for the
/// tenant would be refused, then showing `submit` still succeeds because it
/// never makes that call.
#[tokio::test]
async fn submit_does_not_register_the_bundle_registry() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let store = PgPipelineStore::new(backend.clone());
    let tenant = format!("submit-no-register-{}", uuid::Uuid::new_v4());
    let scorer = ReferencePerplexityScorer::new();
    let embedder = ReferenceEmbedder::new();

    // The bundle already active for this tenant -- standing in for what
    // ingest startup registers before any receipt.
    let active_package = MinimalPolicyBundle::minimal_package(
        &PipelineBundleConfig {
            instrument_awards: vec![PipelineInstrumentAwardConfig {
                instrument_id: "storage_rebate".into(),
                atomic_units: AtomicUnits::from_raw(5),
                descriptor: storage_rebate_descriptor(),
            }],
            include_index: false,
            variant: None,
        },
        &scorer,
        &embedder,
    )
    .expect("build the active package");
    store
        .register_bundle(&tenant, &active_package)
        .await
        .expect("register the active package");
    store
        .activate_bundle_if_none(&tenant, &active_package.bundle_id)
        .await
        .expect("activate the active package");

    // A service whose own default bundle names the same instrument with a
    // different descriptor: the registry refuses to register this one for a
    // tenant that already has the first descriptor on file.
    let mut conflicting_descriptor = storage_rebate_descriptor();
    conflicting_descriptor.decimals = 3;
    let conflicting_config = PipelineBundleConfig {
        instrument_awards: vec![PipelineInstrumentAwardConfig {
            instrument_id: "storage_rebate".into(),
            atomic_units: AtomicUnits::from_raw(5),
            descriptor: conflicting_descriptor,
        }],
        include_index: false,
        variant: None,
    };
    let dir = tempfile::tempdir().unwrap();
    let (conflicting_service, _, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        conflicting_config.clone(),
        None,
    )
    .await;

    // Confirm the registry really would refuse this service's own bundle
    // for the tenant right now, so the receipt below proves something.
    let conflicting_package =
        MinimalPolicyBundle::minimal_package(&conflicting_config, &scorer, &embedder)
            .expect("build the conflicting package");
    let registration_error = store
        .register_bundle(&tenant, &conflicting_package)
        .await
        .expect_err("the registry refuses a descriptor conflict for this tenant");
    assert!(
        registration_error
            .to_string()
            .contains("bundle_instrument_conflict")
    );

    // A receipt against the conflicting service still succeeds: `submit`
    // never asks the registry to register its own bundle, so a tenant whose
    // registry state would refuse it is unaffected.
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let result = conflicting_service
        .submit(receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .expect("submit succeeds without touching the bundle registry");
    assert!(matches!(result, PipelineReceiptResult::Created(_)));
}

/// A store whose `publish_serialized_json` blocks the first call it
/// receives until the test releases it; every other call, and every call
/// after the first, goes straight to the real store underneath. Lets a test
/// prove nothing is held across a receipt's object write: if the tenant
/// quota lock were still held there, a second receipt for the same tenant
/// could never pass its own staging transaction while the first one's write
/// is still blocked.
struct BlockFirstPublishArtifactStore {
    inner: Arc<dyn TraceArtifactStore>,
    should_block: AtomicBool,
    release: std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

impl TraceArtifactStore for BlockFirstPublishArtifactStore {
    fn prepare_serialized_json(
        &self,
        tenant_storage_ref: &str,
        artifact_kind: TraceArtifactKind,
        object_id: &str,
        serialized_json: &[u8],
    ) -> anyhow::Result<PreparedSerializedJsonArtifact> {
        self.inner.prepare_serialized_json(
            tenant_storage_ref,
            artifact_kind,
            object_id,
            serialized_json,
        )
    }

    fn publish_serialized_json(
        &self,
        prepared: &PreparedSerializedJsonArtifact,
    ) -> anyhow::Result<EncryptedTraceArtifactReceipt> {
        if self.should_block.swap(false, Ordering::SeqCst) {
            let receiver = self
                .release
                .lock()
                .unwrap()
                .take()
                .expect("the blocked call fires at most once");
            receiver
                .recv_timeout(std::time::Duration::from_secs(20))
                .expect("the test released the blocked write within the bound");
        }
        self.inner.publish_serialized_json(prepared)
    }

    fn put_serialized_json(
        &self,
        tenant_storage_ref: &str,
        artifact_kind: TraceArtifactKind,
        object_id: &str,
        serialized_json: &[u8],
    ) -> anyhow::Result<EncryptedTraceArtifactReceipt> {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_receipt_does_not_wait_on_the_firsts_blocked_object_write() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let store: Arc<dyn TraceArtifactStore> = Arc::new(BlockFirstPublishArtifactStore {
        inner: artifact_store(&dir),
        should_block: AtomicBool::new(true),
        release: std::sync::Mutex::new(Some(receiver)),
    });
    let (service, _, _) = test_service(backend.clone(), store, minimal_config(false), None).await;
    let tenant = format!("quota-lock-write-{}", uuid::Uuid::new_v4());
    service.register_default_bundle(&tenant).await.unwrap();

    let env_a = envelope(uuid::Uuid::new_v4()).await;
    let raw_a = serde_json::to_vec(&env_a).unwrap();
    let key_a = env_a.submission_id.to_string();
    let service_a = service.clone();
    let tenant_a = tenant.clone();
    let receipt_a = tokio::spawn(async move {
        service_a
            .submit(receipt(&tenant_a, &key_a, &raw_a, &env_a, NO_LIMITS))
            .await
    });

    // Receipt A's staging transaction always runs, and commits, before the
    // object write it is now blocked in -- wait for its `staged` row rather
    // than guessing at a sleep.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if !receipt_artifact_rows(&backend, &tenant).await.is_empty() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "receipt A's staging transaction never committed"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // Receipt B, same tenant, a different key: if the quota lock were still
    // held while A's write is blocked, B's own staging transaction (which
    // takes the same lock) would hang here too. Bound the wait so a
    // regression fails the test instead of hanging it.
    let env_b = envelope(uuid::Uuid::new_v4()).await;
    let raw_b = serde_json::to_vec(&env_b).unwrap();
    let key_b = env_b.submission_id.to_string();
    let result_b = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        service.submit(receipt(&tenant, &key_b, &raw_b, &env_b, NO_LIMITS)),
    )
    .await
    .expect("receipt B must not wait on receipt A's blocked object write");
    assert!(matches!(
        result_b.unwrap(),
        PipelineReceiptResult::Created(_)
    ));

    // Release A and confirm it still completes.
    sender.send(()).unwrap();
    let result_a = tokio::time::timeout(std::time::Duration::from_secs(10), receipt_a)
        .await
        .expect("receipt A must finish once released")
        .expect("receipt A's task did not panic");
    assert!(matches!(
        result_a.unwrap(),
        PipelineReceiptResult::Created(_)
    ));
}

// The receipt's staging row commits
// in its own transaction before the object write and names that object, so
// an attempt that fails after the write leaves a `staged` row the sweeper
// (`PipelineService::sweep_staged_receipts`) consumes.

/// One `pipeline_receipt_artifacts` row of a tenant.
#[derive(Debug)]
struct ReceiptArtifactRow {
    state: String,
    object_key: String,
    ciphertext_sha256: Option<String>,
}

/// Every `pipeline_receipt_artifacts` row of `tenant_id`, oldest first, in
/// its own tenant-scoped transaction.
async fn receipt_artifact_rows(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
) -> Vec<ReceiptArtifactRow> {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for receipt_artifact_rows");
    let tx = tenant_tx(&mut client, tenant_id).await;
    let rows = tx
        .query(
            "SELECT state, object_key, ciphertext_sha256
               FROM pipeline_receipt_artifacts
              WHERE tenant_id = $1
              ORDER BY staged_at, object_key",
            &[&tenant_id],
        )
        .await
        .expect("read receipt artifact rows");
    tx.commit().await.expect("commit receipt_artifact_rows");
    rows.iter()
        .map(|row| ReceiptArtifactRow {
            state: row.get("state"),
            object_key: row.get("object_key"),
            ciphertext_sha256: row.get("ciphertext_sha256"),
        })
        .collect()
}

/// Moves `cleanup_after` of every `state` row of `tenant_id` into the past,
/// the way an hour passing would.
async fn make_receipt_artifacts_due(backend: &Arc<PgBackend>, tenant_id: &str, state: &str) {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for make_receipt_artifacts_due");
    let tx = tenant_tx(&mut client, tenant_id).await;
    tx.execute(
        "UPDATE pipeline_receipt_artifacts
            SET cleanup_after = NOW() - INTERVAL '1 second'
          WHERE tenant_id = $1 AND state = $2",
        &[&tenant_id, &state],
    )
    .await
    .expect("make receipt artifacts due");
    tx.commit()
        .await
        .expect("commit make_receipt_artifacts_due");
}

/// `SELECT COUNT(*)` over `pipeline_admission_usage` for `tenant_id`.
async fn count_admission_usage(backend: &Arc<PgBackend>, tenant_id: &str) -> i64 {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for count_admission_usage");
    let tx = tenant_tx(&mut client, tenant_id).await;
    let count: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM pipeline_admission_usage WHERE tenant_id = $1",
            &[&tenant_id],
        )
        .await
        .expect("count admission usage")
        .get(0);
    tx.commit().await.expect("commit count_admission_usage");
    count
}

/// The SHA-256 (hex) of the ciphertext a local artifact file holds.
fn stored_ciphertext_sha256(path: &std::path::Path) -> String {
    let artifact: EncryptedTraceArtifact =
        serde_json::from_slice(&std::fs::read(path).expect("read artifact file"))
            .expect("parse artifact file");
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(artifact.ciphertext_base64.as_bytes())
        .expect("decode ciphertext");
    hex::encode(Sha256::digest(&ciphertext))
}

/// The owner's test: a receipt that fails after the object write leaves a
/// `staged` row that names the object, and the object file exists. Once
/// `cleanup_after` has passed, the sweeper removes the object and the row.
#[tokio::test]
async fn a_receipt_failing_after_the_write_leaves_a_staged_row_the_sweeper_removes() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (crashing, _, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(false),
        Some(PipelineCrashPoint::AfterArtifactStorage),
    )
    .await;
    let tenant = format!("receipt-orphan-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();

    let error = submit_registered(&crashing, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .expect_err("the receipt fails after its object write");
    assert_eq!(error.to_string(), INJECTED_PIPELINE_CRASH);
    assert_eq!(count_runs(&backend, &tenant).await, 0);

    let rows = receipt_artifact_rows(&backend, &tenant).await;
    assert_eq!(rows.len(), 1, "the failed receipt keeps its staging row");
    assert_eq!(rows[0].state, "staged");
    let tenant_ref = pipeline_tenant_storage_ref(&tenant);
    let orphan = artifact_file_path(dir.path(), tenant_ref.as_str(), &rows[0].object_key);
    assert!(orphan.exists(), "the staging row names the stored object");
    assert_eq!(
        rows[0].ciphertext_sha256.as_deref(),
        Some(stored_ciphertext_sha256(&orphan).as_str()),
        "the staging row names the exact ciphertext"
    );

    make_receipt_artifacts_due(&backend, &tenant, "staged").await;
    assert_eq!(
        crashing.sweep_staged_receipts(&tenant, 10).await.unwrap(),
        1,
        "the sweeper removes the due staged attempt"
    );
    assert!(!orphan.exists(), "the sweeper deleted the orphaned object");
    assert!(receipt_artifact_rows(&backend, &tenant).await.is_empty());
    assert_eq!(count_files_under(dir.path()), 0);
}

/// The sweeper never touches a committed receipt's object, even when its
/// row's `cleanup_after` has passed, nor a `staged` row that is not yet
/// due. A retry of a key whose first attempt failed after the write is
/// counted once: with a tenant limit of one it still gets through.
#[tokio::test]
async fn the_sweeper_keeps_committed_receipts_and_staged_rows_not_yet_due() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (crashing, _, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(false),
        Some(PipelineCrashPoint::AfterArtifactStorage),
    )
    .await;
    let (service, _, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(false),
        None,
    )
    .await;
    let tenant = format!("receipt-sweep-keep-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let one_per_hour = PipelineAdmissionLimits {
        max_per_tenant_per_hour: 1,
        max_per_principal_per_hour: 0,
    };

    submit_registered(&crashing, receipt(&tenant, &key, &raw, &env, one_per_hour))
        .await
        .expect_err("the first attempt fails after its object write");
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, one_per_hour))
            .await
            .unwrap()
    else {
        panic!("the retry of the same key is not refused by its own count")
    };
    assert_eq!(count_admission_usage(&backend, &tenant).await, 1);

    let rows = receipt_artifact_rows(&backend, &tenant).await;
    assert_eq!(rows.len(), 2, "one staged and one committed attempt");
    let staged = rows.iter().find(|row| row.state == "staged").unwrap();
    let committed = rows.iter().find(|row| row.state == "committed").unwrap();
    assert_ne!(staged.object_key, committed.object_key);
    let tenant_ref = pipeline_tenant_storage_ref(&tenant);
    let staged_path = artifact_file_path(dir.path(), tenant_ref.as_str(), &staged.object_key);
    let committed_path = artifact_file_path(dir.path(), tenant_ref.as_str(), &committed.object_key);

    // The committed row is due; the staged row is not.
    make_receipt_artifacts_due(&backend, &tenant, "committed").await;
    assert_eq!(service.sweep_staged_receipts(&tenant, 10).await.unwrap(), 0);
    assert!(staged_path.exists(), "a staged row not yet due is kept");
    assert!(committed_path.exists(), "a committed object is never swept");
    assert_eq!(receipt_artifact_rows(&backend, &tenant).await.len(), 2);

    // The committed run still reads its source object: Review runs over it.
    let reviewed = service
        .process_run(&tenant, created.run_id)
        .await
        .expect("Review reads the committed source object")
        .expect("the run is claimable");
    assert_eq!(reviewed.last_error_label, None);
    assert_eq!(reviewed.next_phase, Some(Phase::Score));
}

/// A store that runs `after_write` just after each object write it passes
/// to `inner` (`put_serialized_json` or `publish_serialized_json`), and
/// counts the objects it prepares (`prepare_serialized_json`). The receipt
/// holds no transaction and no lock between its write and its final
/// transaction, so the hook can drive another receipt, a tombstone, or a
/// failed sweep to completion in that window.
struct HookedWriteStore {
    inner: Arc<dyn TraceArtifactStore>,
    after_write: Box<dyn Fn() + Send + Sync>,
    prepares: AtomicUsize,
}

impl HookedWriteStore {
    fn new(inner: Arc<dyn TraceArtifactStore>, after_write: Box<dyn Fn() + Send + Sync>) -> Self {
        Self {
            inner,
            after_write,
            prepares: AtomicUsize::new(0),
        }
    }
}

impl TraceArtifactStore for HookedWriteStore {
    fn put_serialized_json(
        &self,
        tenant_storage_ref: &str,
        artifact_kind: TraceArtifactKind,
        object_id: &str,
        serialized_json: &[u8],
    ) -> anyhow::Result<EncryptedTraceArtifactReceipt> {
        let receipt = self.inner.put_serialized_json(
            tenant_storage_ref,
            artifact_kind,
            object_id,
            serialized_json,
        )?;
        (self.after_write)();
        Ok(receipt)
    }

    fn prepare_serialized_json(
        &self,
        tenant_storage_ref: &str,
        artifact_kind: TraceArtifactKind,
        object_id: &str,
        serialized_json: &[u8],
    ) -> anyhow::Result<PreparedSerializedJsonArtifact> {
        self.prepares.fetch_add(1, Ordering::SeqCst);
        self.inner.prepare_serialized_json(
            tenant_storage_ref,
            artifact_kind,
            object_id,
            serialized_json,
        )
    }

    fn publish_serialized_json(
        &self,
        prepared: &PreparedSerializedJsonArtifact,
    ) -> anyhow::Result<EncryptedTraceArtifactReceipt> {
        let receipt = self.inner.publish_serialized_json(prepared)?;
        (self.after_write)();
        Ok(receipt)
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

/// Two concurrent receipts with the same key and content: both are past the
/// object write together (each under its own attempt's object), one run is
/// created, the other attempt returns `Replayed` and deletes its own
/// object, and the one committed object ref's hash matches the stored
/// ciphertext. The sweeper then finds nothing more to remove.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_receipts_for_one_key_commit_one_object() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    // Hold each write until both attempts are writing (or five seconds
    // pass, which records a miss).
    let arrived = Arc::new(AtomicUsize::new(0));
    let missed = Arc::new(AtomicBool::new(false));
    let store = HookedWriteStore::new(
        artifact_store(&dir),
        Box::new({
            let arrived = arrived.clone();
            let missed = missed.clone();
            move || {
                arrived.fetch_add(1, Ordering::SeqCst);
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while arrived.load(Ordering::SeqCst) < 2 {
                    if std::time::Instant::now() >= deadline {
                        missed.store(true, Ordering::SeqCst);
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }),
    );
    let (service, _, _) = test_service(
        backend.clone(),
        Arc::new(store),
        minimal_config(false),
        None,
    )
    .await;
    let tenant = format!("receipt-race-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();

    let mut tasks = Vec::new();
    for _ in 0..2 {
        let service = service.clone();
        let tenant = tenant.clone();
        let env = env.clone();
        let raw = raw.clone();
        let key = key.clone();
        tasks.push(tokio::spawn(async move {
            submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
                .await
                .unwrap()
        }));
    }
    let mut run_ids = Vec::new();
    let (mut created, mut replayed) = (0, 0);
    for task in tasks {
        match task.await.unwrap() {
            PipelineReceiptResult::Created(run) => {
                created += 1;
                run_ids.push(run.run_id);
            }
            PipelineReceiptResult::Replayed(run) => {
                replayed += 1;
                run_ids.push(run.run_id);
            }
            other => panic!("unexpected receipt result {other:?}"),
        }
    }
    assert!(
        !missed.load(Ordering::SeqCst),
        "both attempts reached the object write together: no lock is held across it"
    );
    assert_eq!((created, replayed), (1, 1));
    assert_eq!(run_ids[0], run_ids[1]);
    assert_eq!(count_runs(&backend, &tenant).await, 1);

    let rows = receipt_artifact_rows(&backend, &tenant).await;
    assert_eq!(rows.len(), 1, "the losing attempt removed its staging row");
    assert_eq!(rows[0].state, "committed");
    assert_eq!(
        count_files_under(dir.path()),
        1,
        "the losing attempt deleted its own object"
    );

    let run = service
        .store()
        .get_run(&tenant, run_ids[0])
        .await
        .unwrap()
        .expect("run exists");
    let (object_key, content_sha256): (String, String) = {
        let mut client = backend.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, &tenant).await;
        let row = tx
            .query_one(
                "SELECT object_key, content_sha256 FROM trace_object_refs
                  WHERE tenant_id = $1 AND object_ref_id = $2",
                &[&tenant, &run.source_object_ref_id],
            )
            .await
            .expect("the run's source object ref");
        tx.commit().await.unwrap();
        (row.get("object_key"), row.get("content_sha256"))
    };
    assert_eq!(object_key, rows[0].object_key);
    let tenant_ref = pipeline_tenant_storage_ref(&tenant);
    let path = artifact_file_path(dir.path(), tenant_ref.as_str(), &object_key);
    assert_eq!(
        content_sha256,
        format!("sha256:{}", stored_ciphertext_sha256(&path)),
        "the committed object ref's hash matches the stored ciphertext"
    );

    make_receipt_artifacts_due(&backend, &tenant, "staged").await;
    make_receipt_artifacts_due(&backend, &tenant, "committed").await;
    assert_eq!(service.sweep_staged_receipts(&tenant, 10).await.unwrap(), 0);
    assert_eq!(count_files_under(dir.path()), 1);
}

/// A tombstone that arrives just after the attempt writes its object (after
/// the staging transaction's check) is caught by the final transaction's
/// re-check: the receipt is `Tombstoned`, no run is created, and the
/// attempt deletes its own object and row at once. The quota stays counted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tombstone_during_the_write_refuses_the_commit_and_deletes_the_object() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let tenant = format!("receipt-late-tombstone-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let store = HookedWriteStore::new(
        artifact_store(&dir),
        Box::new({
            let backend = backend.clone();
            let tenant = tenant.clone();
            let redaction_hash = env.privacy.redaction_hash.clone();
            move || {
                let handle = tokio::runtime::Handle::current();
                tokio::task::block_in_place(|| {
                    handle.block_on(seed_redaction_tombstone(&backend, &tenant, &redaction_hash))
                });
            }
        }),
    );
    let (service, _, _) = test_service(
        backend.clone(),
        Arc::new(store),
        minimal_config(false),
        None,
    )
    .await;

    let result = submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .unwrap();
    assert!(matches!(result, PipelineReceiptResult::Tombstoned));
    assert_eq!(count_runs(&backend, &tenant).await, 0);
    assert!(
        receipt_artifact_rows(&backend, &tenant).await.is_empty(),
        "the refused attempt removed its staging row"
    );
    assert_eq!(
        count_files_under(dir.path()),
        0,
        "the refused attempt deleted its own object"
    );
    assert_eq!(count_admission_usage(&backend, &tenant).await, 1);
}

/// A sweep that deletes a due row's object and then fails
/// before its commit leaves the row `staged`, unlocked, and its object gone.
/// An attempt that is still alive past `cleanup_after` must then refuse at
/// its final transaction (`receipt_staging_missing`) instead of committing a
/// run whose source object ref names a deleted object.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_attempt_whose_row_is_due_does_not_commit() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let tenant = format!("receipt-due-attempt-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    // Just after the write: the row is due and its object is gone, the
    // state a failed sweep leaves behind.
    let store = HookedWriteStore::new(
        artifact_store(&dir),
        Box::new({
            let backend = backend.clone();
            let tenant = tenant.clone();
            let root = dir.path().to_path_buf();
            move || {
                let handle = tokio::runtime::Handle::current();
                tokio::task::block_in_place(|| {
                    handle.block_on(async {
                        make_receipt_artifacts_due(&backend, &tenant, "staged").await;
                        let rows = receipt_artifact_rows(&backend, &tenant).await;
                        assert_eq!(rows.len(), 1, "the attempt's row is staged");
                        let tenant_ref = pipeline_tenant_storage_ref(&tenant);
                        std::fs::remove_file(artifact_file_path(
                            &root,
                            tenant_ref.as_str(),
                            &rows[0].object_key,
                        ))
                        .expect("remove the staged object");
                    })
                });
            }
        }),
    );
    let (service, _, _) = test_service(
        backend.clone(),
        Arc::new(store),
        minimal_config(false),
        None,
    )
    .await;

    let error = submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
        .await
        .expect_err("an attempt whose row is due must not commit");
    assert_eq!(error.to_string(), "receipt_staging_missing");
    assert_eq!(count_runs(&backend, &tenant).await, 0, "no run is created");
    assert!(
        receipt_artifact_rows(&backend, &tenant).await.is_empty(),
        "the refused attempt removed its row"
    );
    assert_eq!(count_files_under(dir.path()), 0);
}

/// The common refusals -- quota, replay, content conflict,
/// tombstone -- are found by a read-only check before the attempt's object
/// is encrypted, so a refused receipt costs no encryption and no key wrap.
#[tokio::test]
async fn refused_receipts_do_not_prepare_an_object() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(HookedWriteStore::new(artifact_store(&dir), Box::new(|| {})));
    let (service, _, _) = test_service(
        backend.clone(),
        store.clone() as Arc<dyn TraceArtifactStore>,
        minimal_config(false),
        None,
    )
    .await;
    let tenant = format!("receipt-early-refusal-{}", uuid::Uuid::new_v4());
    let one_per_hour = PipelineAdmissionLimits {
        max_per_tenant_per_hour: 1,
        max_per_principal_per_hour: 0,
    };

    let first = envelope(uuid::Uuid::new_v4()).await;
    let first_raw = serde_json::to_vec(&first).unwrap();
    let first_key = first.submission_id.to_string();
    assert!(matches!(
        submit_registered(
            &service,
            receipt(&tenant, &first_key, &first_raw, &first, one_per_hour)
        )
        .await
        .unwrap(),
        PipelineReceiptResult::Created(_)
    ));
    assert_eq!(store.prepares.load(Ordering::SeqCst), 1);

    let second = envelope(uuid::Uuid::new_v4()).await;
    let second_raw = serde_json::to_vec(&second).unwrap();
    let second_key = second.submission_id.to_string();
    assert!(matches!(
        submit_registered(
            &service,
            receipt(&tenant, &second_key, &second_raw, &second, one_per_hour)
        )
        .await
        .unwrap(),
        PipelineReceiptResult::QuotaExceeded(PipelineQuotaScope::Tenant)
    ));
    assert_eq!(
        store.prepares.load(Ordering::SeqCst),
        1,
        "a quota refusal prepares nothing"
    );

    assert!(matches!(
        submit_registered(
            &service,
            receipt(&tenant, &first_key, &first_raw, &first, one_per_hour)
        )
        .await
        .unwrap(),
        PipelineReceiptResult::Replayed(_)
    ));
    let mut changed = first_raw.clone();
    changed.push(b' ');
    assert!(matches!(
        submit_registered(
            &service,
            receipt(&tenant, &first_key, &changed, &first, one_per_hour)
        )
        .await
        .unwrap(),
        PipelineReceiptResult::ContentConflict
    ));
    assert_eq!(
        store.prepares.load(Ordering::SeqCst),
        1,
        "a replay and a content conflict prepare nothing"
    );

    let tombstone_tenant = format!("receipt-early-tombstone-{}", uuid::Uuid::new_v4());
    let third = envelope(uuid::Uuid::new_v4()).await;
    let third_raw = serde_json::to_vec(&third).unwrap();
    let third_key = third.submission_id.to_string();
    seed_redaction_tombstone(&backend, &tombstone_tenant, &third.privacy.redaction_hash).await;
    assert!(matches!(
        submit_registered(
            &service,
            receipt(&tombstone_tenant, &third_key, &third_raw, &third, NO_LIMITS)
        )
        .await
        .unwrap(),
        PipelineReceiptResult::Tombstoned
    ));
    assert_eq!(
        store.prepares.load(Ordering::SeqCst),
        1,
        "a tombstone refusal prepares nothing"
    );
    assert_eq!(count_files_under(dir.path()), 1);
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
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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
    let PipelineReceiptResult::Created(created) =
        submit_registered(service, receipt(tenant, &key, &raw, &env, NO_LIMITS))
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
        tenant_storage_ref: tenant_ref.clone(),
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
    for (key, entry) in command.keyed_entries(&tenant_ref) {
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

/// A Settle index outage (`IndexWriteError::Failed`/`Uncertain`)
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
    let PipelineReceiptResult::Created(created) =
        submit_registered(&full, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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
    let tenant_ref = pipeline_tenant_storage_ref(&tenant);
    let (key, first_entry) = command
        .keyed_entries(&tenant_ref)
        .next()
        .expect("at least one chunk");

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

/// A withdrawal that lands while a
/// run is claimed in Review must never be reversed by the Review commit
/// racing in behind it. Store level: the run is claimed in Review, the
/// approved revision is built exactly the way the Review arm builds it, the
/// submission is withdrawn *for real* (`record_trace_withdrawal`, which sets
/// `status = 'revoked'` with `revoked_at`/`purged_at` under the submission
/// row's lock -- not the tombstone-only `withdraw_submission` helper), and
/// then `commit_review` is called directly. The commit must refuse and
/// write nothing: no `review_snapshot` object ref, no derived record, no
/// Review outcome, and no change to the run row.
#[tokio::test]
async fn withdrawal_during_review_refuses_commit_and_stays_revoked() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let store = PgPipelineStore::new(backend.clone());
    let dir = tempfile::tempdir().unwrap();
    let artifacts = artifact_store(&dir);
    let tenant = format!("review-withdraw-store-{}", uuid::Uuid::new_v4());
    let seeded = seed_run(&backend, &tenant, uuid::Uuid::new_v4()).await;
    let claimed = store
        .claim_run(&tenant, seeded.run_id, chrono::Duration::seconds(30))
        .await
        .unwrap()
        .expect("the seeded run is claimable");
    let lease_token = claimed.lease_token.expect("a claim carries a lease");

    // Build the approved revision the way the Review arm does: write the
    // approved bytes under the claim's own attempt object id (FR1), wrapped
    // per decision P1, then the same `trace_object_refs` write
    // `approved_object_ref` computes.
    let content = b"approved content for the withdrawal-during-review test".to_vec();
    let wrapper = serde_json::to_vec(&serde_json::json!({
        "schema": "trace_commons.pipeline_artifact_bytes.v1",
        "bytes_base64": base64::engine::general_purpose::STANDARD.encode(&content),
    }))
    .unwrap();
    let object_id = pipeline_attempt_object_id("approved", claimed.run_id, lease_token);
    let receipt = artifacts
        .put_serialized_json(
            pipeline_tenant_storage_ref(&tenant).as_str(),
            TraceArtifactKind::ContributionEnvelope,
            &object_id,
            &wrapper,
        )
        .expect("write the approved object");
    let object_ref_id = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("tracecommons:pipeline-approved-object:{}", claimed.run_id).as_bytes(),
    );
    let approved = ApprovedRevision {
        revision_id: uuid::Uuid::new_v4(),
        object_ref: TraceObjectRefWrite {
            object_ref_id,
            tenant_id: tenant.clone(),
            submission_id: claimed.submission_id,
            artifact_kind: TraceObjectArtifactKind::ReviewSnapshot,
            object_store: PIPELINE_DEFAULT_OBJECT_STORE_NAME.to_string(),
            object_key: receipt.object_key.clone(),
            content_sha256: format!("sha256:{}", receipt.ciphertext_sha256),
            encryption_key_ref: format!("tenant:{}", pipeline_tenant_storage_ref(&tenant).as_str()),
            size_bytes: content.len() as i64,
            compression: None,
            created_by_job_id: None,
        },
        content_hash: dependency_content_hash(&content),
        source_content_hash: dependency_content_hash(b"source-bytes-for-the-test"),
        worker_identity: "minimal_review_passthrough".to_string(),
    };

    // The real withdrawal: revokes and purges the submission under the
    // submission row's lock, exactly as a contributor-initiated withdrawal
    // does in production.
    backend
        .record_trace_withdrawal(
            &tenant,
            claimed.submission_id,
            chrono::Utc::now(),
            "received",
            "not_distributed",
        )
        .await
        .expect("record the withdrawal");
    assert_eq!(
        submission_status(&backend, &tenant, claimed.submission_id).await,
        "revoked"
    );

    let outcome = StoredPhaseResult {
        phase: Phase::Review,
        decision: serde_json::json!({"approved": true}),
        evidence: serde_json::json!({}),
        evaluation: serde_json::json!({}),
    };

    let result = store.commit_review(&claimed, outcome, Some(approved)).await;
    let error = result.expect_err("commit_review must refuse an inoperable submission");
    assert!(
        error
            .to_string()
            .contains(PIPELINE_SUBMISSION_INOPERABLE_LABEL),
        "unexpected error: {error}"
    );

    // The withdrawal's own effect must survive untouched -- not reversed to
    // `accepted`.
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &tenant).await;
    let row = tx
        .query_one(
            "SELECT status, revoked_at, purged_at FROM trace_submissions
              WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant, &claimed.submission_id],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let status: String = row.get("status");
    let revoked_at: Option<chrono::DateTime<chrono::Utc>> = row.get("revoked_at");
    let purged_at: Option<chrono::DateTime<chrono::Utc>> = row.get("purged_at");
    assert_eq!(status, "revoked");
    assert!(revoked_at.is_some());
    assert!(purged_at.is_some());

    // No review_snapshot object ref, no derived record for the run.
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &tenant).await;
    let object_ref_count: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM trace_object_refs
              WHERE tenant_id = $1 AND submission_id = $2 AND artifact_kind = 'review_snapshot'",
            &[&tenant, &claimed.submission_id],
        )
        .await
        .unwrap()
        .get(0);
    let derived_count: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM trace_derived_records
              WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant, &claimed.submission_id],
        )
        .await
        .unwrap()
        .get(0);
    tx.commit().await.unwrap();
    assert_eq!(object_ref_count, 0, "no review_snapshot object ref");
    assert_eq!(derived_count, 0, "no derived record for the submission");

    // No Review outcome row.
    let outcomes = store.list_outcomes(&tenant, claimed.run_id).await.unwrap();
    assert!(
        outcomes
            .iter()
            .all(|outcome| outcome.phase != Phase::Review),
        "no Review outcome row"
    );

    // The run row is unchanged by the refused commit.
    let after = store
        .get_run(&tenant, claimed.run_id)
        .await
        .unwrap()
        .expect("the run still exists");
    assert_eq!(after.next_phase, Some(Phase::Review));
    assert_eq!(after.state, PipelineRunState::Leased);
    assert_eq!(after.lease_token, Some(lease_token));
    assert!(after.approved_revision_id.is_none());
    assert!(after.approved_object_ref_id.is_none());
    assert!(after.approved_content_hash.is_none());
}

/// The service-level race window: withdraws the submission
/// *after* the approved object is written and *before* `commit_review`
/// runs. `put_serialized_json` is a synchronous call the Review arm makes
/// mid-transaction-free (there is no open database transaction while it
/// runs), so a wrapper that intercepts exactly that write and drives the
/// real withdrawal to completion before returning makes the race
/// deterministic: `commit_review` must always find the
/// submission already withdrawn by the time it takes the submission row's
/// lock.
///
/// The withdrawal itself runs on its own thread with its own Tokio runtime
/// and its own single-connection `PgBackend` (never the shared pool the
/// rest of the test drives): the wrapper's `put_serialized_json` is called
/// from inside the test's own `#[tokio::test]` runtime, and nesting a
/// `block_on` inside a running runtime panics, so the withdrawal needs a
/// runtime of its own. Using a dedicated connection (rather than checking
/// the shared pool out from a foreign runtime) avoids leaving a connection
/// whose background I/O task belongs to a runtime that is about to be torn
/// down.
struct WithdrawOnApprovedWriteStore {
    inner: Arc<dyn TraceArtifactStore>,
    runtime_url: String,
    tenant_id: String,
    submission_id: uuid::Uuid,
    triggered: AtomicBool,
}

impl TraceArtifactStore for WithdrawOnApprovedWriteStore {
    fn put_serialized_json(
        &self,
        tenant_storage_ref: &str,
        artifact_kind: TraceArtifactKind,
        object_id: &str,
        serialized_json: &[u8],
    ) -> anyhow::Result<EncryptedTraceArtifactReceipt> {
        let receipt = self.inner.put_serialized_json(
            tenant_storage_ref,
            artifact_kind,
            object_id,
            serialized_json,
        )?;
        if object_id.starts_with("pipeline-approved-")
            && !self.triggered.swap(true, Ordering::SeqCst)
        {
            let runtime_url = self.runtime_url.clone();
            let tenant_id = self.tenant_id.clone();
            let submission_id = self.submission_id;
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Runtime::new().expect("build withdrawal runtime");
                runtime.block_on(async move {
                    let withdrawal_backend =
                        PgBackend::new(&DatabaseConfig::from_postgres_url(&runtime_url, 1))
                            .await
                            .expect("connect a dedicated withdrawal connection");
                    withdrawal_backend
                        .record_trace_withdrawal(
                            &tenant_id,
                            submission_id,
                            chrono::Utc::now(),
                            "received",
                            "not_distributed",
                        )
                        .await
                        .expect("record the race-window withdrawal");
                });
            })
            .join()
            .expect("withdrawal thread completes");
        }
        Ok(receipt)
    }

    fn prepare_serialized_json(
        &self,
        tenant_storage_ref: &str,
        artifact_kind: TraceArtifactKind,
        object_id: &str,
        serialized_json: &[u8],
    ) -> anyhow::Result<PreparedSerializedJsonArtifact> {
        self.inner.prepare_serialized_json(
            tenant_storage_ref,
            artifact_kind,
            object_id,
            serialized_json,
        )
    }

    fn publish_serialized_json(
        &self,
        prepared: &PreparedSerializedJsonArtifact,
    ) -> anyhow::Result<EncryptedTraceArtifactReceipt> {
        self.inner.publish_serialized_json(prepared)
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

/// Builds the runtime role's connection URL from the test database URL env
/// var, the same transform `runtime_backend` applies.
fn runtime_role_url(base_url: &str) -> String {
    let mut runtime_url = reqwest::Url::parse(base_url).expect("parse test URL");
    runtime_url
        .set_username(RUNTIME_ROLE)
        .expect("set runtime user");
    runtime_url.to_string()
}

#[tokio::test]
async fn withdrawal_during_the_review_commit_race_fails_the_run_and_deletes_the_object() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let owner_url =
        std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL").expect("guarded by runtime_backend");
    let dir = tempfile::tempdir().unwrap();
    let tenant = format!("review-withdraw-race-{}", uuid::Uuid::new_v4());
    let submission_id = uuid::Uuid::new_v4();
    let wrapper = Arc::new(WithdrawOnApprovedWriteStore {
        inner: artifact_store(&dir),
        runtime_url: runtime_role_url(&owner_url),
        tenant_id: tenant.clone(),
        submission_id,
        triggered: AtomicBool::new(false),
    });
    let (service, _, _) = test_service(
        backend.clone(),
        wrapper as Arc<dyn TraceArtifactStore>,
        minimal_config(false),
        None,
    )
    .await;
    let env = envelope(submission_id).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
            .await
            .unwrap()
    else {
        panic!("receipt creates a run")
    };

    // Before Review runs, only the source envelope has been written.
    assert_eq!(count_files_under(dir.path()), 1);

    let processed = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Review runs and fails closed on the race-window withdrawal");

    assert_eq!(processed.state, PipelineRunState::Failed);
    assert_eq!(
        processed.last_error_label.as_deref(),
        Some(PIPELINE_SUBMISSION_INOPERABLE_LABEL)
    );
    assert_eq!(
        submission_status(&backend, &tenant, created.submission_id).await,
        "revoked"
    );

    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &tenant).await;
    let object_ref_count: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM trace_object_refs
              WHERE tenant_id = $1 AND submission_id = $2 AND artifact_kind = 'review_snapshot'",
            &[&tenant, &created.submission_id],
        )
        .await
        .unwrap()
        .get(0);
    tx.commit().await.unwrap();
    assert_eq!(object_ref_count, 0, "no review_snapshot object ref");

    // The approved object this attempt wrote must be deleted after the
    // refused commit -- the source envelope is the only file left.
    assert_eq!(
        count_files_under(dir.path()),
        1,
        "the approved object this attempt wrote must be deleted, leaving only \
         the source envelope"
    );
}

/// A submission withdrawn *before* the run is
/// even claimed for Review must fail the run terminally on the very first
/// pass -- `load_object_bytes`'s existing operability refusal is permanent
/// in Review, not a charged retry to burn through `max_attempts` (the label
/// otherwise falls into P2's ordinary charged-retry allowlist, which would
/// take five attempts to exhaust it). No approved object is ever written,
/// because Review never reaches the point that writes one.
#[tokio::test]
async fn withdrawal_before_review_claim_fails_closed_on_the_first_pass() {
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
    let tenant = format!("review-withdraw-before-claim-{}", uuid::Uuid::new_v4());
    let env = envelope(uuid::Uuid::new_v4()).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
            .await
            .unwrap()
    else {
        panic!("receipt creates a run")
    };

    backend
        .record_trace_withdrawal(
            &tenant,
            created.submission_id,
            chrono::Utc::now(),
            "received",
            "not_distributed",
        )
        .await
        .expect("record the withdrawal");

    let files_before = count_files_under(dir.path());

    let processed = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Review fails closed on the first pass");

    assert_eq!(processed.state, PipelineRunState::Failed);
    assert_eq!(processed.attempt_count, 1, "failed on the first attempt");
    assert_eq!(
        processed.last_error_label.as_deref(),
        Some(PIPELINE_SUBMISSION_INOPERABLE_LABEL)
    );
    assert_eq!(
        submission_status(&backend, &tenant, created.submission_id).await,
        "revoked"
    );
    assert_eq!(
        count_files_under(dir.path()),
        files_before,
        "no approved object was written"
    );
}

/// The committed Settle decision
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
        index.entry_count(&tenant_ref, MINIMAL_INDEX_ID),
        0,
        "no entry was applied for a withdrawn submission"
    );
    assert_eq!(
        index.writer_calls(),
        0,
        "dispatch never started for a submission already inoperable at persist time"
    );
}

/// The second case: the guard can newly fail *between*
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
        index.entry_count(&tenant_ref, MINIMAL_INDEX_ID),
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

    // An adapter call error is a dependency failure, not the
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
    assert_eq!(
        rebate_row.last_error_label.as_deref(),
        Some(SettlementError::Unavailable.label()),
        "an `Unavailable` answer leaves the leg waiting under the adapter's own label"
    );
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
        rebate.requests().len(),
        1,
        "every retry after an `Unavailable` answer sent the same request: the recording \
         adapter refuses changed content for one operation as a conflict"
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
            InstrumentSettlementOutcome::Completed {
                result_ref_hash, ..
            } => {
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

/// Step 6's top-level inoperable branch: a Settle run has
/// already dispatched one external leg and left it waiting -- `storage_bonus`
/// answers `Unavailable`, so it is `retry` with `dispatched_at` set -- while
/// a second external leg, `storage_rebate`, never got a cap configured and
/// so never reached its adapter at all (an uncharged suspension raised
/// before any adapter call), and `trace_credit` was never reached either
/// (the missing cap aborts the pass before Step 6's loop gets past
/// `storage_rebate`, which sorts ahead of it). A real withdrawal
/// (`record_trace_withdrawal`) then lands. The next pass must forfeit
/// `storage_bonus` as `settlement_unreconciled` -- its adapter was called at
/// least once, so it may have taken effect -- while `storage_rebate` (never
/// dispatched) and `trace_credit` (pays only through its ledger row) both
/// keep `submission_inoperable`. No adapter is called in that pass, and
/// Settle still completes under the inoperable guard.
#[tokio::test]
async fn withdrawal_flags_a_dispatched_external_leg_for_reconciliation() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let storage_bonus = RecordingSettlementAdapter::new(
        InstrumentId::new("storage_bonus").unwrap(),
        "recording_storage_bonus_test_only",
        "none",
    );
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
    let service = test_service_with_adapters_and_caps(
        backend.clone(),
        artifact_store(&dir),
        three_leg_config(),
        vec![
            storage_bonus.clone() as Arc<dyn SettlementAdapter>,
            storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            trace_credit.clone() as Arc<dyn SettlementAdapter>,
        ],
        // `storage_rebate` gets no cap: a configuration gap that waits
        // uncharged before its adapter is ever called, so it stays
        // undispatched through the withdrawal below.
        uncapped_caps(&["storage_bonus", InstrumentId::trace_credit().as_str()]),
    )
    .await;
    let tenant = format!("settle-withdraw-unreconciled-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;

    storage_bonus.fail_next();
    let waited = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("the first Settle attempt waits, uncharged, on the missing cap");
    assert_eq!(waited.state, PipelineRunState::Retry);
    assert_eq!(
        waited.last_error_label.as_deref(),
        Some(PIPELINE_SETTLEMENT_CAP_MISSING_LABEL)
    );
    assert_eq!(
        waited.attempt_count, run.attempt_count,
        "a missing cap is a configuration gap, never charged"
    );
    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_bonus"), "retry");
    assert!(leg_dispatched(&rows, "storage_bonus"));
    assert_eq!(leg_state(&rows, "storage_rebate"), "pending");
    assert!(!leg_dispatched(&rows, "storage_rebate"));
    assert_eq!(leg_state(&rows, "trace_credit"), "pending");
    assert!(!leg_dispatched(&rows, "trace_credit"));

    backend
        .record_trace_withdrawal(
            &tenant,
            run.submission_id,
            chrono::Utc::now(),
            "accepted",
            "not_distributed",
        )
        .await
        .expect("record the withdrawal");

    let calls_before = (
        storage_bonus.requests().len(),
        storage_rebate.requests().len(),
        trace_credit.requests().len(),
    );
    force_due(&backend, &tenant, run.run_id).await;
    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle completes despite the withdrawal");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(settled.next_phase, None);
    assert_eq!(
        (
            storage_bonus.requests().len(),
            storage_rebate.requests().len(),
            trace_credit.requests().len(),
        ),
        calls_before,
        "no adapter call in the pass that forfeits every leg"
    );

    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_bonus"), "forfeited");
    assert_eq!(
        leg_label(&rows, "storage_bonus"),
        Some(PIPELINE_SETTLEMENT_UNRECONCILED_LABEL),
        "a dispatched external leg may have taken effect"
    );
    assert!(
        leg_dispatched(&rows, "storage_bonus"),
        "dispatched_at is kept"
    );
    assert_eq!(leg_state(&rows, "storage_rebate"), "forfeited");
    assert_eq!(
        leg_label(&rows, "storage_rebate"),
        Some(PIPELINE_SUBMISSION_INOPERABLE_LABEL),
        "an undispatched leg had no effect"
    );
    assert!(!leg_dispatched(&rows, "storage_rebate"));
    assert_eq!(leg_state(&rows, "trace_credit"), "forfeited");
    assert_eq!(
        leg_label(&rows, "trace_credit"),
        Some(PIPELINE_SUBMISSION_INOPERABLE_LABEL),
        "a Trace Credit leg pays only through its ledger row"
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
    let evidence: SettleEvidence = serde_json::from_value(settle_outcome.evidence).unwrap();
    assert_eq!(evidence.submission_operable, Some(false));
}

/// A leg that had no effect keeps the ordinary withdrawal label: a leg whose own label
/// already says no effect happened -- the adapter answered `Conflict` --
/// keeps `submission_inoperable` when a later withdrawal forfeits it, even
/// though it was dispatched: unlike an ambiguous `Unavailable` or
/// `settlement_result_mismatch` answer, `Conflict` (and `Rejected`) already
/// mean nothing needs reconciling.
#[tokio::test]
async fn a_conflict_leg_forfeited_on_withdrawal_keeps_submission_inoperable() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let storage_rebate = Arc::new(RefusingSettlementAdapter {
        instrument_id: InstrumentId::new("storage_rebate").unwrap(),
        error: SettlementError::Conflict,
    });
    let trace_credit = RecordingSettlementAdapter::new(
        InstrumentId::trace_credit(),
        "recording_trace_credit_test_only",
        "none",
    );
    let service = test_service_with_adapters(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        vec![
            storage_rebate as Arc<dyn SettlementAdapter>,
            trace_credit.clone() as Arc<dyn SettlementAdapter>,
        ],
    )
    .await;
    let tenant = format!("settle-withdraw-conflict-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;

    let blocked = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("the first Settle attempt retries on the charged conflict");
    assert_eq!(blocked.state, PipelineRunState::Retry);
    assert_eq!(
        blocked.last_error_label.as_deref(),
        Some("settlement_operation_retry")
    );
    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "failed");
    assert_eq!(
        leg_label(&rows, "storage_rebate"),
        Some(SettlementError::Conflict.label())
    );
    assert!(leg_dispatched(&rows, "storage_rebate"));
    assert_eq!(
        leg_state(&rows, "trace_credit"),
        "complete",
        "the leg ahead of the conflicted one still settles in the same pass"
    );

    backend
        .record_trace_withdrawal(
            &tenant,
            run.submission_id,
            chrono::Utc::now(),
            "accepted",
            "not_distributed",
        )
        .await
        .expect("record the withdrawal");

    force_due(&backend, &tenant, run.run_id).await;
    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle completes despite the withdrawal");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(settled.next_phase, None);

    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "forfeited");
    assert_eq!(
        leg_label(&rows, "storage_rebate"),
        Some(PIPELINE_SUBMISSION_INOPERABLE_LABEL),
        "a leg whose own label already says no effect happened keeps its \
         withdrawal label, even though it was dispatched"
    );
    assert!(
        leg_dispatched(&rows, "storage_rebate"),
        "dispatched_at is kept"
    );
    assert_eq!(
        leg_state(&rows, "trace_credit"),
        "complete",
        "a leg completed earlier stays complete: no leg reverses another"
    );
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
/// held leg's adapter is never called, and only once the hold is
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

/// Like `InterruptingCreditAdapter`'s
/// `PlaceHold` interruption, but the intervening effect is a real withdrawal
/// (`record_trace_withdrawal`) rather than a hold, so it must be driven the
/// way `WithdrawOnApprovedWriteStore` drives one from inside a synchronous
/// callback: on a dedicated thread with its own Tokio runtime and its own
/// single-connection `PgBackend`, never the shared pool the rest of the test
/// drives from inside the test's own runtime.
struct WithdrawOnCreditSettleAdapter {
    inner: Arc<RecordingSettlementAdapter>,
    runtime_url: String,
    tenant_id: String,
    submission_id: uuid::Uuid,
    triggered: AtomicBool,
}

#[async_trait::async_trait]
impl SettlementAdapter for WithdrawOnCreditSettleAdapter {
    fn instrument_id(&self) -> &InstrumentId {
        self.inner.instrument_id()
    }

    fn adapter_identity(&self) -> &str {
        "withdraw_on_credit_settle_test_only"
    }

    fn payout_rail(&self) -> &str {
        "none"
    }

    async fn settle(
        &self,
        request: &SettlementRequest,
    ) -> Result<SettlementReceipt, SettlementError> {
        let receipt = self.inner.settle(request).await?;
        if !self.triggered.swap(true, Ordering::SeqCst) {
            let runtime_url = self.runtime_url.clone();
            let tenant_id = self.tenant_id.clone();
            let submission_id = self.submission_id;
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Runtime::new().expect("build withdrawal runtime");
                runtime.block_on(async move {
                    let withdrawal_backend =
                        PgBackend::new(&DatabaseConfig::from_postgres_url(&runtime_url, 1))
                            .await
                            .expect("connect a dedicated withdrawal connection");
                    withdrawal_backend
                        .record_trace_withdrawal(
                            &tenant_id,
                            submission_id,
                            chrono::Utc::now(),
                            "accepted",
                            "not_distributed",
                        )
                        .await
                        .expect("record the race-window withdrawal");
                });
            })
            .join()
            .expect("withdrawal thread completes");
        }
        Ok(receipt)
    }
}

/// `submission_guard` commits and
/// releases its lock as soon as Step 6 reads it, so a withdrawal landing
/// between that read and the Trace Credit leg's own ledger transaction must
/// still be caught -- otherwise the leg is credited to an account that has
/// already been withdrawn. `settle_internal_credit` re-checks
/// `trace_withdrawals`/`revoked_at` under the submission row's own `FOR
/// SHARE` lock, inside the transaction that would otherwise insert the
/// ledger row and finalize the batch, so the withdrawal here forfeits the
/// pending award instead of paying it.
///
/// The withdrawal lands from inside the Trace Credit adapter's own `settle`
/// call -- the seam
/// `a_hold_placed_during_the_adapter_call_is_caught_by_the_credit_transaction`
/// uses, but with a real withdrawal instead of a hold. `scored_config` also
/// awards `storage_rebate`, which sorts before `trace_credit` and so
/// completes earlier in the same pass: reusing it here doubles as the "a leg
/// completed earlier in the pass stays complete" case (A9), so a second,
/// dedicated test for that is not needed.
#[tokio::test]
async fn a_withdrawal_during_the_credit_adapter_call_forfeits_the_pending_award() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let owner_url =
        std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL").expect("guarded by runtime_backend");
    let dir = tempfile::tempdir().unwrap();
    let tenant = format!("settle-credit-withdraw-race-{}", uuid::Uuid::new_v4());
    let submission_id = uuid::Uuid::new_v4();

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
    let withdrawing = Arc::new(WithdrawOnCreditSettleAdapter {
        inner: trace_credit.clone(),
        runtime_url: runtime_role_url(&owner_url),
        tenant_id: tenant.clone(),
        submission_id,
        triggered: AtomicBool::new(false),
    });
    let service = test_service_with_adapters(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        vec![
            storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            withdrawing as Arc<dyn SettlementAdapter>,
        ],
    )
    .await;

    let env = envelope(submission_id).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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

    let settled = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Settle completes despite the mid-pass withdrawal");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(settled.next_phase, None);

    assert_eq!(
        trace_credit.requests().len(),
        1,
        "the credit adapter is still called once -- the race is caught after the \
         adapter call returns, inside the ledger transaction"
    );
    assert_eq!(
        storage_rebate.requests().len(),
        1,
        "the earlier leg in the pass still completes (A9: no leg reverses another)"
    );
    assert_eq!(
        count_credit_ledger_rows_for_run(&backend, &tenant, created.run_id).await,
        0,
        "no ledger row for the withdrawn submission's pending award"
    );

    let settlements = service
        .store()
        .list_settlements(&tenant, created.run_id)
        .await
        .unwrap();
    let credit_row = settlements
        .iter()
        .find(|settlement| settlement.instrument_id == InstrumentId::trace_credit().as_str())
        .expect("trace_credit row present");
    assert_eq!(credit_row.operation_state, "forfeited");
    assert_eq!(
        credit_row.last_error_label.as_deref(),
        Some(PIPELINE_SUBMISSION_INOPERABLE_LABEL)
    );
    assert_eq!(credit_row.result_ref_hash, None);
    assert_eq!(credit_row.credit_event_id, None);
    assert_eq!(
        credit_row.settlement_batch_id, None,
        "no finalized batch carries the run's event"
    );

    let rebate_row = settlements
        .iter()
        .find(|settlement| settlement.instrument_id == "storage_rebate")
        .expect("storage_rebate row present");
    assert_eq!(
        rebate_row.operation_state, "complete",
        "a leg completed earlier in the pass stays complete"
    );

    let outcomes = service
        .store()
        .list_outcomes(&tenant, created.run_id)
        .await
        .unwrap();
    let settle_outcome = outcomes
        .into_iter()
        .find(|outcome| outcome.phase == Phase::Settle)
        .expect("Settle outcome recorded");
    let evidence: SettleEvidence = serde_json::from_value(settle_outcome.evidence).unwrap();
    assert_eq!(
        evidence.submission_operable,
        Some(false),
        "the run's Settle outcome records the inoperable guard"
    );

    let decision: SettleDecision = serde_json::from_value(settle_outcome.decision).unwrap();
    let operations = decision.settlement_operations();
    assert_eq!(operations.len(), 2);
    for operation in operations {
        match operation.instrument_id().as_str() {
            "trace_credit" => match operation.outcome() {
                InstrumentSettlementOutcome::Forfeited { reason } => {
                    assert_eq!(reason.as_str(), PIPELINE_SUBMISSION_INOPERABLE_LABEL);
                }
                InstrumentSettlementOutcome::Completed { .. } => {
                    panic!("the withdrawn Trace Credit leg must not commit as completed")
                }
            },
            "storage_rebate" => match operation.outcome() {
                InstrumentSettlementOutcome::Completed {
                    result_ref_hash, ..
                } => {
                    assert!(!result_ref_hash.is_empty());
                }
                InstrumentSettlementOutcome::Forfeited { .. } => {
                    panic!("a leg completed earlier in the pass must stay complete")
                }
            },
            other => panic!("unexpected instrument: {other}"),
        }
    }

    assert_eq!(
        submission_status(&backend, &tenant, created.submission_id).await,
        "revoked",
        "the submission stays revoked"
    );
}

/// The in-pass branch after `InternalCreditResult::Inoperable`:
/// `credit_then_vector_config` sorts `trace_credit` ahead of `vector_rebate`,
/// so a withdrawal the credit leg's own ledger transaction catches mid-pass
/// still reaches a later leg in the same pass. `vector_rebate` is dispatched
/// and left waiting (`Unavailable`) on the first Settle attempt, while
/// `trace_credit` is held by an account hold that attempt and so never
/// reaches its adapter -- "dispatched in an earlier attempt" for the later
/// leg, untouched for the credit leg. Once the hold is released, the second
/// attempt's credit leg triggers a real withdrawal from inside its own
/// adapter call (the seam
/// `a_withdrawal_during_the_credit_adapter_call_forfeits_the_pending_award`
/// uses); its ledger transaction's own re-check finds the submission
/// inoperable and forfeits the credit leg as `submission_inoperable` (Trace
/// Credit keeps that label), then treats every remaining leg of the same
/// pass the way Step 6's top-level branch would -- so `vector_rebate` is
/// forfeited as `settlement_unreconciled` without another adapter call, and
/// Settle still completes.
#[tokio::test]
async fn a_withdrawal_inside_the_credit_call_also_forfeits_a_later_dispatched_leg() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let owner_url =
        std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL").expect("guarded by runtime_backend");
    let dir = tempfile::tempdir().unwrap();
    let tenant = format!(
        "settle-credit-then-vector-withdraw-{}",
        uuid::Uuid::new_v4()
    );
    let submission_id = uuid::Uuid::new_v4();
    let hold_id = uuid::Uuid::new_v4();

    let trace_credit = RecordingSettlementAdapter::new(
        InstrumentId::trace_credit(),
        "recording_trace_credit_test_only",
        "none",
    );
    let withdrawing = Arc::new(WithdrawOnCreditSettleAdapter {
        inner: trace_credit.clone(),
        runtime_url: runtime_role_url(&owner_url),
        tenant_id: tenant.clone(),
        submission_id,
        triggered: AtomicBool::new(false),
    });
    let vector_rebate = RecordingSettlementAdapter::new(
        InstrumentId::new("vector_rebate").unwrap(),
        "recording_vector_rebate_test_only",
        "none",
    );
    let service = test_service_with_adapters_and_caps(
        backend.clone(),
        artifact_store(&dir),
        credit_then_vector_config(),
        vec![
            withdrawing as Arc<dyn SettlementAdapter>,
            vector_rebate.clone() as Arc<dyn SettlementAdapter>,
        ],
        credit_then_vector_caps(),
    )
    .await;

    let env = envelope(submission_id).await;
    let raw = serde_json::to_vec(&env).unwrap();
    let key = env.submission_id.to_string();
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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

    // First Settle attempt: the credit account is held, so `trace_credit`
    // never reaches its adapter (the withdrawing adapter is never
    // triggered), while `vector_rebate` answers `Unavailable` and so is left
    // `retry` with `dispatched_at` set -- dispatched in this earlier
    // attempt.
    backend
        .upsert_trace_credit_hold(credit_hold(&tenant, hold_id, None))
        .await
        .expect("hold the credit account");
    vector_rebate.fail_next();
    let waited = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("the first Settle attempt waits, uncharged, on the credit hold");
    assert_eq!(waited.state, PipelineRunState::Retry);
    assert_eq!(
        waited.last_error_label.as_deref(),
        Some(PIPELINE_CREDIT_HELD_LABEL)
    );
    let rows = settlement_rows(&backend, &tenant, created.run_id).await;
    assert_eq!(leg_state(&rows, "trace_credit"), "held");
    assert!(!leg_dispatched(&rows, "trace_credit"));
    assert_eq!(leg_state(&rows, "vector_rebate"), "retry");
    assert!(leg_dispatched(&rows, "vector_rebate"));
    assert_eq!(
        vector_rebate.requests().len(),
        0,
        "an `Unavailable` answer is not a logical request"
    );

    // Release the hold: the second attempt reaches the credit adapter for
    // the first time, and its own `settle` call triggers the real
    // withdrawal before the ledger transaction re-checks operability.
    backend
        .upsert_trace_credit_hold(credit_hold(&tenant, hold_id, Some(chrono::Utc::now())))
        .await
        .expect("release the credit hold");
    let vector_calls_before = vector_rebate.requests().len();
    force_due(&backend, &tenant, created.run_id).await;
    let settled = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("Settle completes despite the mid-pass withdrawal");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(settled.next_phase, None);

    assert_eq!(
        trace_credit.requests().len(),
        1,
        "the credit adapter is called exactly once, on this attempt"
    );
    assert_eq!(
        vector_rebate.requests().len(),
        vector_calls_before,
        "vector_rebate is forfeited without another adapter call"
    );
    assert_eq!(
        count_credit_ledger_rows_for_run(&backend, &tenant, created.run_id).await,
        0,
        "no ledger row for the withdrawn submission's pending award"
    );

    let rows = settlement_rows(&backend, &tenant, created.run_id).await;
    assert_eq!(leg_state(&rows, "trace_credit"), "forfeited");
    assert_eq!(
        leg_label(&rows, "trace_credit"),
        Some(PIPELINE_SUBMISSION_INOPERABLE_LABEL)
    );
    assert_eq!(leg_state(&rows, "vector_rebate"), "forfeited");
    assert_eq!(
        leg_label(&rows, "vector_rebate"),
        Some(PIPELINE_SETTLEMENT_UNRECONCILED_LABEL),
        "a leg dispatched in an earlier attempt may have taken effect"
    );
    assert!(
        leg_dispatched(&rows, "vector_rebate"),
        "dispatched_at is kept"
    );

    let outcomes = service
        .store()
        .list_outcomes(&tenant, created.run_id)
        .await
        .unwrap();
    let settle_outcome = outcomes
        .into_iter()
        .find(|outcome| outcome.phase == Phase::Settle)
        .expect("Settle outcome recorded");
    let evidence: SettleEvidence = serde_json::from_value(settle_outcome.evidence).unwrap();
    assert_eq!(
        evidence.submission_operable,
        Some(false),
        "the run's Settle outcome records the inoperable guard"
    );
    let decision: SettleDecision = serde_json::from_value(settle_outcome.decision).unwrap();
    let operations = decision.settlement_operations();
    assert_eq!(operations.len(), 2);
    for operation in operations {
        match operation.outcome() {
            InstrumentSettlementOutcome::Forfeited { reason } => {
                assert_eq!(
                    reason.as_str(),
                    PIPELINE_SUBMISSION_INOPERABLE_LABEL,
                    "the committed decision's reason never changes, whatever the \
                     stored row's own label says"
                );
            }
            InstrumentSettlementOutcome::Completed { .. } => {
                panic!("every operation must be forfeited")
            }
        }
    }

    assert_eq!(
        submission_status(&backend, &tenant, created.submission_id).await,
        "revoked",
        "the submission stays revoked"
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

/// The expected result comes from the persisted selection, not from
/// whatever the adapter hands back. An adapter that returns a well-formed
/// but different result reference fails the row closed rather than being
/// trusted, and the leg is never dispatched again: the next attempt skips
/// it and stays charged.
#[tokio::test]
async fn adapter_result_that_differs_from_the_selection_fails_closed() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mismatching = CountingSettlementAdapter::new(Arc::new(MismatchingSettlementAdapter {
        instrument_id: InstrumentId::new("storage_rebate").unwrap(),
    }));
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
            mismatching.clone() as Arc<dyn SettlementAdapter>,
            trace_credit_adapter.clone() as Arc<dyn SettlementAdapter>,
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
    // Unlike an adapter call error, a result that differs from the
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
    assert_eq!(rebate_row.external_receipt_hash, None);
    assert_eq!(mismatching.calls(), 1);

    // The next attempt does not dispatch the mismatched leg again, and it
    // is charged again while that leg exists.
    force_due(&backend, &tenant, run.run_id).await;
    let again = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle retries again");
    assert_eq!(again.state, PipelineRunState::Retry);
    assert_eq!(
        again.last_error_label.as_deref(),
        Some("settlement_operation_retry")
    );
    assert_eq!(again.attempt_count, run.attempt_count + 2);
    assert_eq!(
        mismatching.calls(),
        1,
        "a mismatched leg is never dispatched again"
    );
    assert_eq!(
        trace_credit_adapter.requests().len(),
        1,
        "the other leg completed on the first attempt and is not repeated"
    );
    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "failed");
    assert_eq!(
        leg_label(&rows, "storage_rebate"),
        Some("settlement_result_mismatch")
    );
    assert_eq!(leg_state(&rows, "trace_credit"), "complete");

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

/// Rewrites the `bundle_id` of a run's stored Score decision, as the
/// database owner, with the outcome immutability trigger disabled only
/// inside the one transaction that makes the change.
async fn tamper_stored_score_bundle_id(tenant_id: &str, run_id: uuid::Uuid, bundle_id: &str) {
    let mut client = owner_client().await;
    let tx = client
        .transaction()
        .await
        .expect("open owner transaction for tampering");
    tx.batch_execute("ALTER TABLE phase_outcomes DISABLE TRIGGER phase_outcomes_reject_update;")
        .await
        .expect("disable the immutability trigger");
    let updated = tx
        .execute(
            "UPDATE phase_outcomes
                SET decision = jsonb_set(decision, '{bundle_id}', to_jsonb($3::TEXT))
              WHERE tenant_id = $1 AND run_id = $2 AND phase = 'score'",
            &[&tenant_id, &run_id, &bundle_id],
        )
        .await
        .expect("tamper the stored Score decision");
    assert_eq!(updated, 1, "one stored Score decision");
    tx.batch_execute("ALTER TABLE phase_outcomes ENABLE TRIGGER phase_outcomes_reject_update;")
        .await
        .expect("enable the immutability trigger");
    tx.commit().await.expect("commit the tampering transaction");
}

/// A stored Score decision records the bundle identifier it was built
/// under, loads unverified, and becomes a decision Settle can use only
/// against the run's bound manifest. A stored decision whose `bundle_id`
/// names another bundle is refused at Settle as `score_outcome_invalid`: a
/// charged retry, with no adapter call and no leg dispatched. A run of the
/// same tenant whose decision is untouched settles.
#[tokio::test]
async fn a_stored_score_decision_under_another_bundle_is_refused_at_settle() {
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
    let tenant = format!("settle-score-bundle-{}", uuid::Uuid::new_v4());
    let (tampered, _evidence) = run_to_settle_ready(&service, &tenant).await;
    let (untouched, _evidence) = run_to_settle_ready(&service, &tenant).await;

    let stored = service
        .store()
        .outcome_for_phase(&tenant, tampered.run_id, Phase::Score)
        .await
        .unwrap()
        .expect("Score outcome recorded");
    assert_eq!(
        stored.decision["bundle_id"].as_str(),
        Some(tampered.bundle_id.as_str()),
        "the stored decision records the run's bound bundle"
    );

    let other_bundle = dependency_content_hash(b"another-bundle-manifest");
    tamper_stored_score_bundle_id(&tenant, tampered.run_id, &other_bundle).await;
    let reloaded = service
        .store()
        .outcome_for_phase(&tenant, tampered.run_id, Phase::Score)
        .await
        .expect("a stored decision under another bundle still loads, unverified")
        .expect("Score outcome recorded");
    assert_eq!(
        reloaded.decision["bundle_id"].as_str(),
        Some(other_bundle.as_str())
    );

    let refused = service
        .process_run(&tenant, tampered.run_id)
        .await
        .unwrap()
        .expect("Settle runs and retries");
    assert_eq!(refused.state, PipelineRunState::Retry);
    assert_eq!(
        refused.last_error_label.as_deref(),
        Some("score_outcome_invalid")
    );
    assert_eq!(
        refused.attempt_count,
        tampered.attempt_count + 1,
        "the refusal is charged"
    );
    let rows = settlement_rows(&backend, &tenant, tampered.run_id).await;
    for instrument in ["storage_rebate", "trace_credit"] {
        assert_eq!(leg_state(&rows, instrument), "pending", "{instrument}");
        assert!(!leg_dispatched(&rows, instrument), "{instrument}");
    }
    for adapter in &adapters {
        assert!(adapter.requests().is_empty(), "no adapter call");
    }
    assert!(
        !service
            .store()
            .list_outcomes(&tenant, tampered.run_id)
            .await
            .unwrap()
            .iter()
            .any(|outcome| outcome.phase == Phase::Settle),
        "no Settle outcome for a refused decision"
    );

    let settled = service
        .process_run(&tenant, untouched.run_id)
        .await
        .unwrap()
        .expect("Settle runs");
    assert_eq!(settled.state, PipelineRunState::Complete);
    for adapter in &adapters {
        assert_eq!(adapter.requests().len(), 1, "the untouched run settles");
    }
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
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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
    let PipelineReceiptResult::Created(created_second) = submit_registered(
        &service,
        receipt(&tenant, &key_second, &raw_second, &env_second, NO_LIMITS),
    )
    .await
    .unwrap() else {
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
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service_one, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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

/// A `PolicyError::Transient` raised while a phase runs (decision D9) --
/// not only a missing bound dependency at the bundle load, which
/// `a_run_whose_dependency_is_not_held_waits_without_charging` above already
/// covers -- releases the run without charging the claim's attempt, so a
/// dependency outage cannot exhaust the trace's attempt budget.
/// `FixedScorePolicy` (`versioned_pipeline_bundle.rs`) maps an embedder
/// failure to `PolicyError::transient("embedder_unavailable")` when the
/// bundle carries an index (P5). `FlakyEmbedder` fails its first 7 calls;
/// the run's `max_attempts` defaults to 5 (migration V93), so 7 failures is
/// more than the run's whole attempt budget, and the run still reaches Score.
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
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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
    let PipelineReceiptResult::Created(created) =
        submit_registered(&service, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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

    // #971 round 3: a manifest whose pinned descriptor fails the tightened
    // `nep141` network rule fails closed the same way, even though the
    // stored JSON is well formed -- only `BundleManifest`'s own load-time
    // validation refuses it, not `BundlePackage::validate`'s artifact-hash
    // check exercised above.
    let (manifest_service, _, _) = test_service(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        None,
    )
    .await;
    let manifest_tenant = format!("tampered-manifest-{}", uuid::Uuid::new_v4());

    let manifest_env = envelope(uuid::Uuid::new_v4()).await;
    let manifest_raw = serde_json::to_vec(&manifest_env).unwrap();
    let manifest_key = manifest_env.submission_id.to_string();
    let PipelineReceiptResult::Created(manifest_created) = submit_registered(
        &manifest_service,
        receipt(
            &manifest_tenant,
            &manifest_key,
            &manifest_raw,
            &manifest_env,
            NO_LIMITS,
        ),
    )
    .await
    .unwrap() else {
        panic!("receipt creates a run")
    };

    let manifest_outcomes_before = manifest_service
        .store()
        .list_outcomes(&manifest_tenant, manifest_created.run_id)
        .await
        .unwrap();

    tamper_stored_bundle_manifest_network(
        &manifest_tenant,
        &manifest_created.bundle_id,
        InstrumentId::trace_credit().as_str(),
        "near-mainnet",
    )
    .await;

    let manifest_failed = manifest_service
        .process_run(&manifest_tenant, manifest_created.run_id)
        .await
        .unwrap()
        .expect("the run fails closed on a manifest that no longer loads");
    assert_eq!(manifest_failed.state, PipelineRunState::Failed);
    assert_eq!(
        manifest_failed.last_error_label.as_deref(),
        Some("bundle_package_invalid")
    );

    let manifest_outcomes_after = manifest_service
        .store()
        .list_outcomes(&manifest_tenant, manifest_created.run_id)
        .await
        .unwrap();
    assert_eq!(
        manifest_outcomes_after.len(),
        manifest_outcomes_before.len(),
        "no new outcome is recorded when the stored manifest fails closed"
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
            let crashed =
                submit_registered(&service_a, receipt(&tenant, &key, &raw, &env, NO_LIMITS)).await;
            let error =
                crashed.expect_err("service A's receipt must crash at AfterArtifactStorage");
            assert_eq!(error.to_string(), INJECTED_PIPELINE_CRASH);

            // No run committed (the receipt's final transaction never ran;
            // only its staging row did), so this is a fresh submission from
            // the store's point of view, not a claim resume -- resubmit the
            // same bytes. The crashed attempt's object stays staged for the
            // sweeper.
            let PipelineReceiptResult::Created(created) =
                submit_registered(&service_b, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
                    .await
                    .unwrap()
            else {
                panic!("resubmission after the crash must create the run (point {point:?})")
            };
            created.run_id
        } else {
            let PipelineReceiptResult::Created(created) =
                submit_registered(&service_a, receipt(&tenant, &key, &raw, &env, NO_LIMITS))
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
            index.entry_count(&tenant_ref, MINIMAL_INDEX_ID),
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
                InstrumentSettlementOutcome::Completed {
                    result_ref_hash, ..
                } => {
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

// A Settle failure that is not the
// trace's fault is an uncharged suspension, and a Settle run that fails
// resolves every open settlement leg -- forfeited, completed on a
// reconciling adapter call, or labeled `settlement_unreconciled`.

/// A third off-chain instrument, for the tests that need two external legs.
fn storage_bonus_descriptor() -> InstrumentDescriptor {
    InstrumentDescriptor {
        kind: InstrumentKind::CreditAccount,
        network: "pipeline-test".to_string(),
        contract: "storage-bonus".to_string(),
        decimals: 0,
    }
}

/// `scored_config` plus a third, external instrument, `storage_bonus` (7
/// atomic units). Step 6 settles legs in instrument order: `storage_bonus`,
/// `storage_rebate`, then `trace_credit`.
fn three_leg_config() -> PipelineBundleConfig {
    let mut config = scored_config(false);
    config
        .instrument_awards
        .push(PipelineInstrumentAwardConfig {
            instrument_id: "storage_bonus".into(),
            atomic_units: AtomicUnits::from_raw(7),
            descriptor: storage_bonus_descriptor(),
        });
    config
}

fn three_leg_caps() -> PipelineCaps {
    uncapped_caps(&[
        "storage_bonus",
        "storage_rebate",
        InstrumentId::trace_credit().as_str(),
    ])
}

/// A fourth off-chain instrument whose id sorts *after* `trace_credit`
/// (`vector_rebate`), for the in-pass withdrawal test:
/// Step 6 settles `trace_credit` first and only reaches this leg afterward
/// in the same pass, so a withdrawal the credit leg's own transaction
/// catches can still forfeit this leg before the pass ends.
fn vector_rebate_descriptor() -> InstrumentDescriptor {
    InstrumentDescriptor {
        kind: InstrumentKind::CreditAccount,
        network: "pipeline-test".to_string(),
        contract: "vector-rebate".to_string(),
        decimals: 0,
    }
}

/// Just the two legs the in-pass withdrawal test needs: `trace_credit`
/// (1,000,000 atomic units) and `vector_rebate` (9 atomic units), an
/// external instrument that sorts after it.
fn credit_then_vector_config() -> PipelineBundleConfig {
    PipelineBundleConfig {
        instrument_awards: vec![
            PipelineInstrumentAwardConfig {
                instrument_id: InstrumentId::trace_credit().as_str().to_string(),
                atomic_units: AtomicUnits::from_raw(1_000_000),
                descriptor: trace_credit_descriptor(),
            },
            PipelineInstrumentAwardConfig {
                instrument_id: "vector_rebate".into(),
                atomic_units: AtomicUnits::from_raw(9),
                descriptor: vector_rebate_descriptor(),
            },
        ],
        include_index: false,
        variant: None,
    }
}

fn credit_then_vector_caps() -> PipelineCaps {
    uncapped_caps(&[InstrumentId::trace_credit().as_str(), "vector_rebate"])
}

/// Counts every `settle` call, a repeated call for one operation included,
/// and delegates to `inner`. `RecordingSettlementAdapter::requests` counts
/// logical operations; this counts calls.
struct CountingSettlementAdapter {
    inner: Arc<dyn SettlementAdapter>,
    calls: AtomicUsize,
}

impl CountingSettlementAdapter {
    fn new(inner: Arc<dyn SettlementAdapter>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            calls: AtomicUsize::new(0),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl SettlementAdapter for CountingSettlementAdapter {
    fn instrument_id(&self) -> &InstrumentId {
        self.inner.instrument_id()
    }

    fn adapter_identity(&self) -> &str {
        self.inner.adapter_identity()
    }

    fn payout_rail(&self) -> &str {
        self.inner.payout_rail()
    }

    async fn settle(
        &self,
        request: &SettlementRequest,
    ) -> Result<SettlementReceipt, SettlementError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.settle(request).await
    }
}

/// Fails the next `fail_next_calls(n)` calls as an adapter outage, then
/// delegates to a recording adapter (so the call after the outage is the
/// first and only logical effect).
struct OutageThenRecordingAdapter {
    inner: Arc<RecordingSettlementAdapter>,
    failures_left: AtomicUsize,
}

impl OutageThenRecordingAdapter {
    fn fail_next_calls(&self, calls: usize) {
        self.failures_left.store(calls, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl SettlementAdapter for OutageThenRecordingAdapter {
    fn instrument_id(&self) -> &InstrumentId {
        self.inner.instrument_id()
    }

    fn adapter_identity(&self) -> &str {
        "outage_then_recording_test_only"
    }

    fn payout_rail(&self) -> &str {
        "none"
    }

    async fn settle(
        &self,
        request: &SettlementRequest,
    ) -> Result<SettlementReceipt, SettlementError> {
        let failing = self
            .failures_left
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok();
        if failing {
            return Err(SettlementError::Unavailable);
        }
        self.inner.settle(request).await
    }
}

/// The settlement row and its run row, as JSON, while an adapter call for
/// that row was in flight.
#[derive(Debug, Clone)]
struct InFlightLeg {
    leg: serde_json::Value,
    run: serde_json::Value,
}

/// A recording adapter that, after each delegated call returns, snapshots
/// its own settlement row and the run row (as JSON, so it reads the lease
/// columns and `dispatched_at` without depending on the record type), and
/// on its first call only expires the run's lease: "the adapter call
/// outlived the lease".
struct ObservingLeaseExpiringAdapter {
    inner: Arc<RecordingSettlementAdapter>,
    backend: Arc<PgBackend>,
    tenant_id: String,
    calls: AtomicUsize,
    observed: std::sync::Mutex<Vec<InFlightLeg>>,
}

impl ObservingLeaseExpiringAdapter {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn observed(&self) -> Vec<InFlightLeg> {
        self.observed.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl SettlementAdapter for ObservingLeaseExpiringAdapter {
    fn instrument_id(&self) -> &InstrumentId {
        self.inner.instrument_id()
    }

    fn adapter_identity(&self) -> &str {
        "observing_lease_expiring_test_only"
    }

    fn payout_rail(&self) -> &str {
        "none"
    }

    async fn settle(
        &self,
        request: &SettlementRequest,
    ) -> Result<SettlementReceipt, SettlementError> {
        let first_call = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        let receipt = self.inner.settle(request).await?;
        let tenant_id = self.tenant_id.clone();
        let run_id = request.run_id();
        let instrument_id = request.instrument_id().as_str().to_string();
        let observed = {
            let mut client = self.backend.trace_pool_for_test().get().await.unwrap();
            let tx = tenant_tx(&mut client, &tenant_id).await;
            let row = tx
                .query_one(
                    "SELECT to_jsonb(s) AS leg, to_jsonb(p) AS run
                       FROM pipeline_run_settlements s
                       JOIN pipeline_runs p
                         ON p.tenant_id = s.tenant_id AND p.run_id = s.run_id
                      WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.instrument_id = $3",
                    &[&tenant_id, &run_id, &instrument_id],
                )
                .await
                .unwrap();
            if first_call {
                tx.execute(
                    "UPDATE pipeline_runs
                        SET lease_expires_at = NOW() - INTERVAL '1 second'
                      WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'",
                    &[&tenant_id, &run_id],
                )
                .await
                .unwrap();
            }
            tx.commit().await.unwrap();
            InFlightLeg {
                leg: row.get("leg"),
                run: row.get("run"),
            }
        };
        self.observed.lock().unwrap().push(observed);
        Ok(receipt)
    }
}

/// Every settlement row of a run as JSON (`to_jsonb`), keyed by instrument
/// -- read as JSON so it also sees the columns `PipelineSettlementRecord`
/// does not carry (the lease columns and `dispatched_at`).
async fn settlement_rows(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    run_id: uuid::Uuid,
) -> BTreeMap<String, serde_json::Value> {
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant_id).await;
    let rows = tx
        .query(
            "SELECT instrument_id, to_jsonb(s) AS leg FROM pipeline_run_settlements s
              WHERE tenant_id = $1 AND run_id = $2",
            &[&tenant_id, &run_id],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    rows.iter()
        .map(|row| (row.get("instrument_id"), row.get("leg")))
        .collect()
}

fn leg_field<'a>(
    rows: &'a BTreeMap<String, serde_json::Value>,
    instrument: &str,
    field: &str,
) -> &'a serde_json::Value {
    rows.get(instrument)
        .unwrap_or_else(|| panic!("{instrument} row present"))
        .get(field)
        .unwrap_or(&serde_json::Value::Null)
}

fn leg_state<'a>(rows: &'a BTreeMap<String, serde_json::Value>, instrument: &str) -> &'a str {
    leg_field(rows, instrument, "operation_state")
        .as_str()
        .unwrap_or("")
}

fn leg_label<'a>(
    rows: &'a BTreeMap<String, serde_json::Value>,
    instrument: &str,
) -> Option<&'a str> {
    leg_field(rows, instrument, "last_error_label").as_str()
}

fn leg_holds_no_lease(rows: &BTreeMap<String, serde_json::Value>, instrument: &str) -> bool {
    leg_field(rows, instrument, "lease_token").is_null()
        && leg_field(rows, instrument, "lease_expires_at").is_null()
}

fn leg_dispatched(rows: &BTreeMap<String, serde_json::Value>, instrument: &str) -> bool {
    !leg_field(rows, instrument, "dispatched_at").is_null()
}

/// The resolved states a leg of a failed run may end in. A Trace Credit leg
/// `failed` as `settlement_result_mismatch` is not one of them: the failure
/// path forfeits it.
fn leg_is_resolved(rows: &BTreeMap<String, serde_json::Value>, instrument: &str) -> bool {
    match leg_state(rows, instrument) {
        "complete" | "forfeited" => true,
        "failed" => match leg_label(rows, instrument) {
            Some("settlement_unreconciled") => true,
            Some("settlement_result_mismatch") => {
                instrument != InstrumentId::trace_credit().as_str()
            }
            _ => false,
        },
        _ => false,
    }
}

/// A direct connection as the test database's owner (a superuser in the
/// test container), for DDL the runtime role cannot run.
async fn owner_client() -> tokio_postgres::Client {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .expect("TRACE_COMMONS_PG_TEST_DATABASE_URL must be set for this test");
    let (client, connection) = tokio_postgres::connect(&url, NoTls)
        .await
        .expect("connect as the database owner");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

/// A test-only trigger that raises SQLSTATE 40001 (serialization failure)
/// exactly once, on the first update that moves `instrument_id`'s leg of
/// `tenant_id` to `complete` -- a transient database error after the
/// adapter call returned. "Once" is a sequence, which a rollback does not
/// undo. Created and dropped as the owner; scoped to one tenant, so tests
/// running beside it are untouched.
struct CompletionFault {
    name: String,
}

impl CompletionFault {
    async fn install(tenant_id: &str, instrument_id: &str) -> Self {
        let name = format!("m1_fault_{}", uuid::Uuid::new_v4().simple());
        owner_client()
            .await
            .batch_execute(&format!(
                "CREATE SEQUENCE {name};
                 GRANT USAGE ON SEQUENCE {name} TO {RUNTIME_ROLE};
                 CREATE FUNCTION {name}() RETURNS TRIGGER LANGUAGE plpgsql AS $$
                 BEGIN
                     IF nextval('{name}') = 1 THEN
                         RAISE EXCEPTION 'injected serialization failure'
                             USING ERRCODE = '40001';
                     END IF;
                     RETURN NEW;
                 END;
                 $$;
                 CREATE TRIGGER {name}
                     BEFORE UPDATE ON pipeline_run_settlements
                     FOR EACH ROW
                     WHEN (
                         NEW.tenant_id = '{tenant_id}'
                         AND NEW.instrument_id = '{instrument_id}'
                         AND NEW.operation_state = 'complete'
                     )
                     EXECUTE FUNCTION {name}();"
            ))
            .await
            .expect("install the completion fault");
        Self { name }
    }

    async fn remove(self) {
        let name = self.name;
        owner_client()
            .await
            .batch_execute(&format!(
                "DROP TRIGGER {name} ON pipeline_run_settlements;
                 DROP FUNCTION {name}();
                 DROP SEQUENCE {name};"
            ))
            .await
            .expect("remove the completion fault");
    }
}

/// A missing per-instrument cap is a configuration gap -- the run
/// waits in retry, uncharged, before any adapter call. An amount over a
/// configured cap is a spend limit that refuses the payment: the leg fails
/// with `credit_cap_exceeded` and the retry stays charged. A service with
/// the cap then settles the leg.
#[tokio::test]
async fn a_missing_cap_waits_uncharged_and_an_amount_over_the_cap_stays_charged() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let shared_artifacts = artifact_store(&dir);
    let storage_rebate = CountingSettlementAdapter::new(RecordingSettlementAdapter::new(
        InstrumentId::new("storage_rebate").unwrap(),
        "recording_storage_rebate_test_only",
        "none",
    ));
    let trace_credit = CountingSettlementAdapter::new(RecordingSettlementAdapter::new(
        InstrumentId::trace_credit(),
        "recording_trace_credit_test_only",
        "none",
    ));
    let adapters: Vec<Arc<dyn SettlementAdapter>> = vec![
        storage_rebate.clone() as Arc<dyn SettlementAdapter>,
        trace_credit.clone() as Arc<dyn SettlementAdapter>,
    ];
    let full = test_service_with_adapters_and_caps(
        backend.clone(),
        shared_artifacts.clone(),
        scored_config(false),
        adapters.clone(),
        uncapped_caps(&["storage_rebate", InstrumentId::trace_credit().as_str()]),
    )
    .await;
    let capless = test_service_with_adapters_and_caps(
        backend.clone(),
        shared_artifacts.clone(),
        scored_config(false),
        adapters.clone(),
        uncapped_caps(&[InstrumentId::trace_credit().as_str()]),
    )
    .await;
    let mut below_award = uncapped_caps(&[InstrumentId::trace_credit().as_str()]);
    below_award
        .per_instrument_atomic_units
        .insert("storage_rebate".to_string(), AtomicUnits::from_raw(4));
    let capped = test_service_with_adapters_and_caps(
        backend.clone(),
        shared_artifacts,
        scored_config(false),
        adapters,
        below_award,
    )
    .await;
    let tenant = format!("settle-cap-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&full, &tenant).await;

    let waited = capless
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle waits for the missing cap");
    assert_eq!(waited.state, PipelineRunState::Retry);
    assert_eq!(waited.next_phase, Some(Phase::Settle));
    assert_eq!(
        waited.last_error_label.as_deref(),
        Some("settlement_cap_missing")
    );
    assert_eq!(
        waited.attempt_count, run.attempt_count,
        "a missing cap is a configuration gap, never charged"
    );
    assert_eq!(storage_rebate.calls(), 0, "no adapter call without a cap");
    assert_eq!(trace_credit.calls(), 0, "no adapter call without a cap");
    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "pending");
    assert!(!leg_dispatched(&rows, "storage_rebate"));

    force_due(&backend, &tenant, run.run_id).await;
    let blocked = capped
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle retries after the cap refuses the amount");
    assert_eq!(blocked.state, PipelineRunState::Retry);
    assert_eq!(
        blocked.last_error_label.as_deref(),
        Some("settlement_operation_retry")
    );
    assert_eq!(
        blocked.attempt_count,
        run.attempt_count + 1,
        "an amount over a configured cap stays a charged failure"
    );
    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "failed");
    assert_eq!(
        leg_label(&rows, "storage_rebate"),
        Some(PIPELINE_CREDIT_CAP_LABEL)
    );
    assert!(
        !leg_dispatched(&rows, "storage_rebate"),
        "the cap is checked before the leg is dispatched"
    );
    assert_eq!(storage_rebate.calls(), 0);

    force_due(&backend, &tenant, run.run_id).await;
    let settled = full
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle completes under a service with the cap");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(storage_rebate.calls(), 1);
    assert_eq!(trace_credit.calls(), 1);
    assert_credit_settled_once(&backend, &full, &tenant, run.run_id, "after the cap").await;
}

/// A transient database error during Settle after `adapter.settle`
/// returned (here a serialization failure on the leg's completion) is not
/// the trace's fault: the run waits in retry as `database_unavailable`,
/// uncharged, and the next attempt repeats the idempotent adapter call and
/// completes the leg with one logical payment.
#[tokio::test]
async fn a_transient_database_error_after_the_adapter_call_is_not_charged() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let rebate_recording = RecordingSettlementAdapter::new(
        InstrumentId::new("storage_rebate").unwrap(),
        "recording_storage_rebate_test_only",
        "none",
    );
    let storage_rebate = CountingSettlementAdapter::new(rebate_recording.clone());
    let trace_credit = CountingSettlementAdapter::new(RecordingSettlementAdapter::new(
        InstrumentId::trace_credit(),
        "recording_trace_credit_test_only",
        "none",
    ));
    let service = test_service_with_adapters(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        vec![
            storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            trace_credit.clone() as Arc<dyn SettlementAdapter>,
        ],
    )
    .await;
    let tenant = format!("settle-db-transient-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;
    let fault = CompletionFault::install(&tenant, "storage_rebate").await;

    let waited = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("the attempt records the database failure");
    assert_eq!(waited.state, PipelineRunState::Retry);
    assert_eq!(
        waited.last_error_label.as_deref(),
        Some("database_unavailable")
    );
    assert_eq!(
        waited.attempt_count, run.attempt_count,
        "a transient database error is never charged"
    );
    assert_eq!(storage_rebate.calls(), 1, "the adapter call returned");
    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(
        leg_state(&rows, "storage_rebate"),
        "leased",
        "the completion rolled back; the leg stays dispatched"
    );
    assert!(leg_dispatched(&rows, "storage_rebate"));

    force_due(&backend, &tenant, run.run_id).await;
    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("the next attempt completes");
    fault.remove().await;
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(storage_rebate.calls(), 2, "the adapter call is repeated");
    assert_eq!(
        rebate_recording.requests().len(),
        1,
        "the repeated call is one logical payment"
    );
    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "complete");
    assert!(leg_holds_no_lease(&rows, "storage_rebate"));
    assert_credit_settled_once(
        &backend,
        &service,
        &tenant,
        run.run_id,
        "after a database error",
    )
    .await;
}

/// A stale lease after `adapter.settle` returns is recorded as
/// `lease_expired`, uncharged. While the call was in flight the leg was
/// `leased` under the run's own lease token and expiry, with
/// `dispatched_at` set; the next attempt dispatches the still-`leased` leg
/// again under its new lease, keeps the first `dispatched_at`, and settles
/// it once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_lease_after_the_adapter_call_is_recorded_and_the_leg_settles_once() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let tenant = format!("settle-stale-after-call-{}", uuid::Uuid::new_v4());
    let rebate_recording = RecordingSettlementAdapter::new(
        InstrumentId::new("storage_rebate").unwrap(),
        "recording_storage_rebate_test_only",
        "none",
    );
    let storage_rebate = Arc::new(ObservingLeaseExpiringAdapter {
        inner: rebate_recording.clone(),
        backend: backend.clone(),
        tenant_id: tenant.clone(),
        calls: AtomicUsize::new(0),
        observed: std::sync::Mutex::new(Vec::new()),
    });
    let trace_credit = CountingSettlementAdapter::new(RecordingSettlementAdapter::new(
        InstrumentId::trace_credit(),
        "recording_trace_credit_test_only",
        "none",
    ));
    let service = test_service_with_adapters(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        vec![
            storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            trace_credit.clone() as Arc<dyn SettlementAdapter>,
        ],
    )
    .await;
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;

    let expired = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("the stale attempt is recorded");
    assert_eq!(expired.state, PipelineRunState::Retry);
    assert_eq!(
        expired.last_error_label.as_deref(),
        Some(PIPELINE_LEASE_EXPIRED_LABEL)
    );
    assert_eq!(
        expired.attempt_count, run.attempt_count,
        "an expired lease is never charged"
    );
    let first = storage_rebate.observed()[0].clone();
    assert_eq!(first.leg["operation_state"], "leased", "{first:?}");
    assert!(!first.leg["lease_token"].is_null(), "{first:?}");
    assert_eq!(first.leg["lease_token"], first.run["lease_token"]);
    assert_eq!(first.leg["lease_expires_at"], first.run["lease_expires_at"]);
    assert!(!first.leg["dispatched_at"].is_null(), "{first:?}");
    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "leased");
    assert_eq!(trace_credit.calls(), 0);

    force_due(&backend, &tenant, run.run_id).await;
    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("the next attempt completes");
    assert_eq!(settled.state, PipelineRunState::Complete);
    let second = storage_rebate.observed()[1].clone();
    assert_eq!(second.leg["operation_state"], "leased", "{second:?}");
    assert_eq!(second.leg["lease_token"], second.run["lease_token"]);
    assert_ne!(
        second.leg["lease_token"], first.leg["lease_token"],
        "the retry dispatches under its own lease"
    );
    assert_eq!(
        second.leg["dispatched_at"], first.leg["dispatched_at"],
        "dispatched_at is set once and never cleared"
    );
    assert_eq!(storage_rebate.calls(), 2);
    assert_eq!(
        rebate_recording.requests().len(),
        1,
        "the repeated call is one logical payment"
    );
    assert_eq!(trace_credit.calls(), 1);
    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "complete");
    assert!(leg_holds_no_lease(&rows, "storage_rebate"));
    assert_credit_settled_once(
        &backend,
        &service,
        &tenant,
        run.run_id,
        "after a stale lease",
    )
    .await;
}

/// The exhausted `mark_retry` path: a Settle run that runs out of
/// attempts resolves every open leg before it fails. The leg whose adapter
/// returns a different result fails closed on the first attempt and is
/// never dispatched again, not by the reconciling call either: it ends
/// `failed` / `settlement_result_mismatch`, and every attempt while it
/// exists is charged. The leg whose adapter was `Unavailable` on every
/// attempt succeeds on the reconciling call (`complete`). The held Trace
/// Credit leg is forfeited (`run_failed`) without reaching its adapter.
#[tokio::test]
async fn a_settle_run_that_exhausts_its_attempts_resolves_every_leg() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let bonus_outage = Arc::new(OutageThenRecordingAdapter {
        inner: RecordingSettlementAdapter::new(
            InstrumentId::new("storage_bonus").unwrap(),
            "recording_storage_bonus_test_only",
            "none",
        ),
        failures_left: AtomicUsize::new(0),
    });
    let storage_bonus = CountingSettlementAdapter::new(bonus_outage.clone());
    let storage_rebate = CountingSettlementAdapter::new(Arc::new(MismatchingSettlementAdapter {
        instrument_id: InstrumentId::new("storage_rebate").unwrap(),
    }));
    let trace_credit = CountingSettlementAdapter::new(RecordingSettlementAdapter::new(
        InstrumentId::trace_credit(),
        "recording_trace_credit_test_only",
        "none",
    ));
    let service = test_service_with_adapters_and_caps(
        backend.clone(),
        artifact_store(&dir),
        three_leg_config(),
        vec![
            storage_bonus.clone() as Arc<dyn SettlementAdapter>,
            storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            trace_credit.clone() as Arc<dyn SettlementAdapter>,
        ],
        three_leg_caps(),
    )
    .await;
    let tenant = format!("settle-exhausted-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;
    backend
        .upsert_trace_credit_hold(credit_hold(&tenant, uuid::Uuid::new_v4(), None))
        .await
        .expect("hold the credit account");
    // The attempts Settle has left; the bonus adapter fails every one of
    // them and answers only the reconciling call after the last.
    let settle_attempts = run.max_attempts - run.attempt_count;
    assert!(settle_attempts >= 2, "the fixture leaves Settle a retry");
    bonus_outage.fail_next_calls(settle_attempts as usize);

    for attempt in 1..=settle_attempts {
        force_due(&backend, &tenant, run.run_id).await;
        let processed = service
            .process_run(&tenant, run.run_id)
            .await
            .unwrap()
            .expect("the Settle attempt runs");
        if attempt < settle_attempts {
            assert_eq!(
                processed.state,
                PipelineRunState::Retry,
                "attempt {attempt}"
            );
            assert_eq!(
                processed.last_error_label.as_deref(),
                Some("settlement_operation_retry"),
                "attempt {attempt}"
            );
        }
    }
    let failed = service
        .store()
        .get_run(&tenant, run.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.state, PipelineRunState::Failed);
    assert_eq!(
        failed.last_error_label.as_deref(),
        Some(PIPELINE_ATTEMPTS_EXHAUSTED_LABEL)
    );

    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    for instrument in ["storage_bonus", "storage_rebate", "trace_credit"] {
        assert!(
            leg_is_resolved(&rows, instrument),
            "{instrument} is resolved: {:?}",
            rows.get(instrument)
        );
        assert!(leg_holds_no_lease(&rows, instrument), "{instrument}");
    }
    let bonus_award = InstrumentAward::new(
        InstrumentId::new("storage_bonus").unwrap(),
        AtomicUnits::from_raw(7),
    )
    .unwrap();
    assert_eq!(leg_state(&rows, "storage_bonus"), "complete");
    assert_eq!(
        leg_field(&rows, "storage_bonus", "result_ref_hash").as_str(),
        Some(pipeline_result_ref(run.run_id, &bonus_award).as_str())
    );
    assert_eq!(leg_state(&rows, "storage_rebate"), "failed");
    assert_eq!(
        leg_label(&rows, "storage_rebate"),
        Some("settlement_result_mismatch")
    );
    assert!(leg_dispatched(&rows, "storage_rebate"));
    assert_eq!(leg_state(&rows, "trace_credit"), "forfeited");
    assert_eq!(leg_label(&rows, "trace_credit"), Some("run_failed"));
    assert!(!leg_dispatched(&rows, "trace_credit"));

    // The outage leg: one call per Settle attempt, plus exactly one
    // reconciling call. The mismatched leg: its first call only.
    assert_eq!(storage_bonus.calls(), settle_attempts as usize + 1);
    assert_eq!(
        storage_rebate.calls(),
        1,
        "a mismatched leg is never dispatched again, not by the reconciling call either"
    );
    assert_eq!(bonus_outage.inner.requests().len(), 1);
    assert_eq!(
        trace_credit.calls(),
        0,
        "a held account never reaches its adapter"
    );
    assert_eq!(
        count_credit_ledger_rows_for_run(&backend, &tenant, run.run_id).await,
        0
    );
    assert!(
        !service
            .store()
            .list_outcomes(&tenant, run.run_id)
            .await
            .unwrap()
            .iter()
            .any(|outcome| outcome.phase == Phase::Settle),
        "a failed run commits no Settle outcome"
    );
}

/// The `mark_failed` path: a Settle run failed by its bound bundle (a
/// tampered package) reconciles its dispatched external leg with one more
/// adapter call and forfeits its undispatched Trace Credit leg. When the
/// submission is no longer operable the reconciling call is not made: the
/// dispatched leg is `settlement_unreconciled`.
#[tokio::test]
async fn a_settle_run_failed_by_its_bundle_reconciles_dispatched_legs() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let rebate_recording = RecordingSettlementAdapter::new(
        InstrumentId::new("storage_rebate").unwrap(),
        "recording_storage_rebate_test_only",
        "none",
    );
    let storage_rebate = CountingSettlementAdapter::new(rebate_recording.clone());
    let trace_credit = CountingSettlementAdapter::new(RecordingSettlementAdapter::new(
        InstrumentId::trace_credit(),
        "recording_trace_credit_test_only",
        "none",
    ));
    let service = test_service_with_adapters(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        vec![
            storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            trace_credit.clone() as Arc<dyn SettlementAdapter>,
        ],
    )
    .await;

    for withdrawn in [false, true] {
        let tenant = format!("settle-failed-bundle-{withdrawn}-{}", uuid::Uuid::new_v4());
        let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;
        backend
            .upsert_trace_credit_hold(credit_hold(&tenant, uuid::Uuid::new_v4(), None))
            .await
            .expect("hold the credit account");
        let calls_before = storage_rebate.calls();
        let requests_before = rebate_recording.requests().len();

        // First attempt: the rebate adapter errors after dispatch and the
        // credit account is held -- both uncharged suspensions.
        rebate_recording.fail_next();
        let waited = service
            .process_run(&tenant, run.run_id)
            .await
            .unwrap()
            .expect("the first Settle attempt waits");
        assert_eq!(
            waited.state,
            PipelineRunState::Retry,
            "withdrawn {withdrawn}"
        );
        let rows = settlement_rows(&backend, &tenant, run.run_id).await;
        assert_eq!(leg_state(&rows, "storage_rebate"), "retry");
        assert!(leg_dispatched(&rows, "storage_rebate"));
        assert_eq!(leg_state(&rows, "trace_credit"), "held");

        if withdrawn {
            withdraw_submission(&backend, &tenant, run.submission_id).await;
        }
        tamper_stored_bundle_package(&tenant, &run.bundle_id).await;
        force_due(&backend, &tenant, run.run_id).await;
        let failed = service
            .process_run(&tenant, run.run_id)
            .await
            .unwrap()
            .expect("the run fails on its tampered package");
        assert_eq!(
            failed.state,
            PipelineRunState::Failed,
            "withdrawn {withdrawn}"
        );
        assert_eq!(
            failed.last_error_label.as_deref(),
            Some("bundle_package_invalid")
        );

        let rows = settlement_rows(&backend, &tenant, run.run_id).await;
        if withdrawn {
            assert_eq!(leg_state(&rows, "storage_rebate"), "failed");
            assert_eq!(
                leg_label(&rows, "storage_rebate"),
                Some("settlement_unreconciled")
            );
            assert_eq!(
                storage_rebate.calls(),
                calls_before + 1,
                "no reconciling call for an inoperable submission"
            );
            assert_eq!(rebate_recording.requests().len(), requests_before);
        } else {
            let rebate_award = InstrumentAward::new(
                InstrumentId::new("storage_rebate").unwrap(),
                AtomicUnits::from_raw(5),
            )
            .unwrap();
            assert_eq!(leg_state(&rows, "storage_rebate"), "complete");
            assert_eq!(
                leg_field(&rows, "storage_rebate", "result_ref_hash").as_str(),
                Some(pipeline_result_ref(run.run_id, &rebate_award).as_str())
            );
            assert_eq!(storage_rebate.calls(), calls_before + 2);
            assert_eq!(rebate_recording.requests().len(), requests_before + 1);
        }
        assert_eq!(leg_state(&rows, "trace_credit"), "forfeited");
        assert_eq!(leg_label(&rows, "trace_credit"), Some("run_failed"));
        for instrument in ["storage_rebate", "trace_credit"] {
            assert!(leg_holds_no_lease(&rows, instrument), "{instrument}");
        }
        assert_eq!(trace_credit.calls(), 0);
    }
}

/// The claim sweep: a Settle run whose attempts are exhausted and whose
/// lease expired (its worker is gone) is failed by the next claim, which
/// resolves its legs in the same transaction without an adapter call: the
/// dispatched external leg is `failed` / `settlement_unreconciled`, the
/// undispatched external leg and the Trace Credit leg (dispatched or not)
/// are `forfeited` / `run_failed`, and no leg keeps a lease.
#[tokio::test]
async fn the_claim_sweep_resolves_the_legs_of_an_exhausted_settle_run() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let adapters: Vec<Arc<CountingSettlementAdapter>> = [
        "storage_bonus",
        "storage_rebate",
        InstrumentId::trace_credit().as_str(),
    ]
    .into_iter()
    .map(|instrument| {
        CountingSettlementAdapter::new(RecordingSettlementAdapter::new(
            InstrumentId::new(instrument).unwrap(),
            "recording_test_only",
            "none",
        ))
    })
    .collect();
    let service = test_service_with_adapters_and_caps(
        backend.clone(),
        artifact_store(&dir),
        three_leg_config(),
        adapters
            .iter()
            .cloned()
            .map(|adapter| adapter as Arc<dyn SettlementAdapter>)
            .collect(),
        three_leg_caps(),
    )
    .await;
    let tenant = format!("settle-sweep-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;
    {
        let mut client = backend.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, &tenant).await;
        let lease_token = uuid::Uuid::new_v4();
        tx.execute(
            "UPDATE pipeline_runs
                SET state = 'leased', lease_token = $3,
                    lease_expires_at = NOW() - INTERVAL '1 second',
                    attempt_count = max_attempts
              WHERE tenant_id = $1 AND run_id = $2",
            &[&tenant, &run.run_id, &lease_token],
        )
        .await
        .unwrap();
        tx.execute(
            "UPDATE pipeline_run_settlements
                SET operation_state = 'leased', lease_token = $3,
                    lease_expires_at = NOW() - INTERVAL '1 second',
                    dispatched_at = NOW() - INTERVAL '2 seconds'
              WHERE tenant_id = $1 AND run_id = $2
                AND instrument_id IN ('storage_rebate', 'trace_credit')",
            &[&tenant, &run.run_id, &lease_token],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }

    let claimed = service.process_one(&tenant).await.unwrap();
    assert!(claimed.is_none(), "an exhausted run is not claimed");
    let swept = service
        .store()
        .get_run(&tenant, run.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(swept.state, PipelineRunState::Failed);
    assert_eq!(
        swept.last_error_label.as_deref(),
        Some(PIPELINE_ATTEMPTS_EXHAUSTED_LABEL)
    );

    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "failed");
    assert_eq!(
        leg_label(&rows, "storage_rebate"),
        Some("settlement_unreconciled")
    );
    assert!(leg_dispatched(&rows, "storage_rebate"));
    assert_eq!(leg_state(&rows, "storage_bonus"), "forfeited");
    assert_eq!(leg_label(&rows, "storage_bonus"), Some("run_failed"));
    assert_eq!(leg_state(&rows, "trace_credit"), "forfeited");
    assert_eq!(leg_label(&rows, "trace_credit"), Some("run_failed"));
    for instrument in ["storage_bonus", "storage_rebate", "trace_credit"] {
        assert!(leg_holds_no_lease(&rows, instrument), "{instrument}");
    }
    for adapter in &adapters {
        assert_eq!(adapter.calls(), 0, "the sweep never calls an adapter");
    }
}

/// A settlement adapter that answers every call `Unavailable`, and on its
/// `expire_on_call`-th call (1-based) expires the calling run's lease before
/// it returns.
struct LeaseExpiringOutageAdapter {
    instrument_id: InstrumentId,
    backend: Arc<PgBackend>,
    tenant_id: String,
    calls: AtomicUsize,
    expire_on_call: AtomicUsize,
}

#[async_trait::async_trait]
impl SettlementAdapter for LeaseExpiringOutageAdapter {
    fn instrument_id(&self) -> &InstrumentId {
        &self.instrument_id
    }

    fn adapter_identity(&self) -> &str {
        "lease_expiring_outage_test_only"
    }

    fn payout_rail(&self) -> &str {
        "none"
    }

    async fn settle(
        &self,
        request: &SettlementRequest,
    ) -> Result<SettlementReceipt, SettlementError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == self.expire_on_call.load(Ordering::SeqCst) {
            let run_id = request.run_id();
            let mut client = self.backend.trace_pool_for_test().get().await.unwrap();
            let tx = tenant_tx(&mut client, &self.tenant_id).await;
            tx.execute(
                "UPDATE pipeline_runs
                    SET lease_expires_at = NOW() - INTERVAL '1 second'
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'",
                &[&self.tenant_id, &run_id],
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
        }
        Err(SettlementError::Unavailable)
    }
}

/// A stale lease inside the failure path's own resolution --
/// here the lease expires during the reconciling adapter call of the
/// attempt that would exhaust the run -- ends as `lease_expired`,
/// uncharged: the run is not failed and no leg is resolved under the stale
/// lease. The next attempt repeats the charged failure and then resolves
/// the leg.
///
/// Three legs: `storage_bonus` answers with a result that differs from the
/// selection (a charged blocker that is never dispatched again),
/// `storage_rebate` is `Unavailable` on every call (the dispatched leg the
/// failure path reconciles), and `trace_credit` settles.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_lease_during_the_failure_resolution_is_recorded_not_charged() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let tenant = format!("settle-stale-resolution-{}", uuid::Uuid::new_v4());
    let storage_bonus = CountingSettlementAdapter::new(Arc::new(MismatchingSettlementAdapter {
        instrument_id: InstrumentId::new("storage_bonus").unwrap(),
    }));
    let storage_rebate = Arc::new(LeaseExpiringOutageAdapter {
        instrument_id: InstrumentId::new("storage_rebate").unwrap(),
        backend: backend.clone(),
        tenant_id: tenant.clone(),
        calls: AtomicUsize::new(0),
        expire_on_call: AtomicUsize::new(0),
    });
    let service = test_service_with_adapters_and_caps(
        backend.clone(),
        artifact_store(&dir),
        three_leg_config(),
        vec![
            storage_bonus.clone() as Arc<dyn SettlementAdapter>,
            storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            RecordingSettlementAdapter::new(
                InstrumentId::trace_credit(),
                "recording_trace_credit_test_only",
                "none",
            ) as Arc<dyn SettlementAdapter>,
        ],
        three_leg_caps(),
    )
    .await;
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;
    // One outage call per Settle attempt; the call after the last one is the
    // reconciling call, and it outlives the lease.
    let settle_attempts = run.max_attempts - run.attempt_count;
    storage_rebate
        .expire_on_call
        .store(settle_attempts as usize + 1, Ordering::SeqCst);

    let mut last = None;
    for _ in 0..settle_attempts {
        force_due(&backend, &tenant, run.run_id).await;
        last = service.process_run(&tenant, run.run_id).await.unwrap();
    }
    let interrupted = last.expect("the last attempt is recorded");
    assert_eq!(interrupted.state, PipelineRunState::Retry);
    assert_eq!(
        interrupted.last_error_label.as_deref(),
        Some(PIPELINE_LEASE_EXPIRED_LABEL)
    );
    assert_eq!(
        interrupted.attempt_count,
        run.max_attempts - 1,
        "the attempt whose resolution lost its lease is given back"
    );
    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "retry");
    assert_eq!(
        leg_label(&rows, "storage_rebate"),
        Some("settlement_adapter_unavailable"),
        "nothing is resolved under a stale lease"
    );
    assert_eq!(leg_state(&rows, "storage_bonus"), "failed");
    assert_eq!(
        leg_label(&rows, "storage_bonus"),
        Some("settlement_result_mismatch")
    );
    assert_eq!(leg_state(&rows, "trace_credit"), "complete");

    force_due(&backend, &tenant, run.run_id).await;
    let failed = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("the next attempt fails the run");
    assert_eq!(failed.state, PipelineRunState::Failed);
    assert_eq!(
        failed.last_error_label.as_deref(),
        Some(PIPELINE_ATTEMPTS_EXHAUSTED_LABEL)
    );
    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "failed");
    assert_eq!(
        leg_label(&rows, "storage_rebate"),
        Some("settlement_unreconciled")
    );
    assert_eq!(leg_state(&rows, "storage_bonus"), "failed");
    assert_eq!(
        leg_label(&rows, "storage_bonus"),
        Some("settlement_result_mismatch")
    );
    assert_eq!(
        leg_state(&rows, "trace_credit"),
        "complete",
        "a completed leg never changes"
    );
    // The outage leg: one call per Settle attempt, the interrupted
    // reconciling call, then the next attempt's call and its reconciling
    // call. The mismatched leg: its first call only.
    assert_eq!(
        storage_rebate.calls.load(Ordering::SeqCst),
        settle_attempts as usize + 3
    );
    assert_eq!(storage_bonus.calls(), 1);
    assert_credit_settled_once(&backend, &service, &tenant, run.run_id, "failed run").await;
}

/// A `Conflict` or `Rejected` answer says no effect happened and the
/// operation must not be sent again. The leg fails under the adapter's own
/// label, is never dispatched again (not by the reconciling call either),
/// and every attempt while it exists is charged, so the other legs still
/// settle and the run's attempts run out. The failure path then forfeits
/// both legs under their own labels.
#[tokio::test]
async fn conflict_and_rejected_legs_fail_under_their_labels_and_are_never_dispatched_again() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let storage_bonus = CountingSettlementAdapter::new(Arc::new(RefusingSettlementAdapter {
        instrument_id: InstrumentId::new("storage_bonus").unwrap(),
        error: SettlementError::Conflict,
    }));
    let storage_rebate = CountingSettlementAdapter::new(Arc::new(RefusingSettlementAdapter {
        instrument_id: InstrumentId::new("storage_rebate").unwrap(),
        error: SettlementError::Rejected,
    }));
    let trace_credit = CountingSettlementAdapter::new(RecordingSettlementAdapter::new(
        InstrumentId::trace_credit(),
        "recording_trace_credit_test_only",
        "none",
    ));
    let service = test_service_with_adapters_and_caps(
        backend.clone(),
        artifact_store(&dir),
        three_leg_config(),
        vec![
            storage_bonus.clone() as Arc<dyn SettlementAdapter>,
            storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            trace_credit.clone() as Arc<dyn SettlementAdapter>,
        ],
        three_leg_caps(),
    )
    .await;
    let tenant = format!("settle-refused-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;
    let settle_attempts = run.max_attempts - run.attempt_count;
    assert!(settle_attempts >= 2, "the fixture leaves Settle a retry");

    for attempt in 1..=settle_attempts {
        force_due(&backend, &tenant, run.run_id).await;
        let processed = service
            .process_run(&tenant, run.run_id)
            .await
            .unwrap()
            .expect("the Settle attempt runs");
        if attempt < settle_attempts {
            assert_eq!(
                processed.state,
                PipelineRunState::Retry,
                "attempt {attempt}"
            );
            assert_eq!(
                processed.last_error_label.as_deref(),
                Some("settlement_operation_retry"),
                "attempt {attempt}"
            );
            assert_eq!(
                processed.attempt_count,
                run.attempt_count + attempt,
                "attempt {attempt}: the retry is charged"
            );
            let rows = settlement_rows(&backend, &tenant, run.run_id).await;
            assert_eq!(leg_state(&rows, "storage_bonus"), "failed");
            assert_eq!(
                leg_label(&rows, "storage_bonus"),
                Some(SettlementError::Conflict.label())
            );
            assert_eq!(leg_state(&rows, "storage_rebate"), "failed");
            assert_eq!(
                leg_label(&rows, "storage_rebate"),
                Some(SettlementError::Rejected.label())
            );
            assert_eq!(
                leg_state(&rows, "trace_credit"),
                "complete",
                "the other leg settles"
            );
        }
        assert_eq!(storage_bonus.calls(), 1, "attempt {attempt}");
        assert_eq!(storage_rebate.calls(), 1, "attempt {attempt}");
        assert_eq!(trace_credit.calls(), 1, "attempt {attempt}");
    }

    let failed = service
        .store()
        .get_run(&tenant, run.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.state, PipelineRunState::Failed);
    assert_eq!(
        failed.last_error_label.as_deref(),
        Some(PIPELINE_ATTEMPTS_EXHAUSTED_LABEL)
    );
    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_bonus"), "forfeited");
    assert_eq!(
        leg_label(&rows, "storage_bonus"),
        Some(SettlementError::Conflict.label())
    );
    assert_eq!(leg_state(&rows, "storage_rebate"), "forfeited");
    assert_eq!(
        leg_label(&rows, "storage_rebate"),
        Some(SettlementError::Rejected.label())
    );
    assert_eq!(leg_state(&rows, "trace_credit"), "complete");
    for instrument in ["storage_bonus", "storage_rebate", "trace_credit"] {
        assert!(leg_holds_no_lease(&rows, instrument), "{instrument}");
        assert!(leg_dispatched(&rows, instrument), "{instrument}");
    }
    assert_eq!(
        storage_bonus.calls(),
        1,
        "no reconciling call for a leg that is never dispatched again"
    );
    assert_eq!(storage_rebate.calls(), 1);
    assert_credit_settled_once(&backend, &service, &tenant, run.run_id, "refused legs").await;
    assert!(
        !service
            .store()
            .list_outcomes(&tenant, run.run_id)
            .await
            .unwrap()
            .iter()
            .any(|outcome| outcome.phase == Phase::Settle),
        "a failed run commits no Settle outcome"
    );
}

/// A leg whose adapter answers with an external receipt records the
/// receipt's hash when it completes, and the committed Settle outcome
/// carries it in the leg's operation and progress. One external receipt
/// answers one leg: a second run's leg answered with the same receipt hash
/// fails closed as `settlement_result_mismatch`, pays nothing, and is never
/// dispatched again.
#[tokio::test]
async fn an_external_receipt_is_recorded_on_its_leg_and_answers_only_that_leg() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let receipt_hash = dependency_content_hash(b"external-receipt-for-one-leg");
    let storage_rebate = CountingSettlementAdapter::new(Arc::new(ExternalReceiptAdapter {
        inner: RecordingSettlementAdapter::new(
            InstrumentId::new("storage_rebate").unwrap(),
            "recording_storage_rebate_test_only",
            "none",
        ),
        receipt_hash: receipt_hash.clone(),
    }));
    let service = test_service_with_adapters(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        vec![
            storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            RecordingSettlementAdapter::new(
                InstrumentId::trace_credit(),
                "recording_trace_credit_test_only",
                "none",
            ) as Arc<dyn SettlementAdapter>,
        ],
    )
    .await;
    let tenant = format!("settle-external-receipt-{}", uuid::Uuid::new_v4());

    let (first, _evidence) = run_to_settle_ready(&service, &tenant).await;
    let settled = service
        .process_run(&tenant, first.run_id)
        .await
        .unwrap()
        .expect("Settle runs");
    assert_eq!(settled.state, PipelineRunState::Complete);
    let settlements = service
        .store()
        .list_settlements(&tenant, first.run_id)
        .await
        .unwrap();
    let rebate_row = settlements
        .iter()
        .find(|settlement| settlement.instrument_id == "storage_rebate")
        .expect("storage_rebate row");
    assert_eq!(rebate_row.operation_state, "complete");
    assert_eq!(
        rebate_row.external_receipt_hash.as_deref(),
        Some(receipt_hash.as_str())
    );
    let credit_row = settlements
        .iter()
        .find(|settlement| settlement.instrument_id == InstrumentId::trace_credit().as_str())
        .expect("trace_credit row");
    assert_eq!(credit_row.operation_state, "complete");
    assert_eq!(
        credit_row.external_receipt_hash, None,
        "an internal receipt records no external hash"
    );

    let settle_outcome = service
        .store()
        .list_outcomes(&tenant, first.run_id)
        .await
        .unwrap()
        .into_iter()
        .find(|outcome| outcome.phase == Phase::Settle)
        .expect("Settle outcome recorded");
    let decision: SettleDecision = serde_json::from_value(settle_outcome.decision).unwrap();
    for operation in decision.settlement_operations() {
        let expected = match operation.instrument_id().as_str() {
            "storage_rebate" => Some(receipt_hash.as_str()),
            _ => None,
        };
        assert_eq!(
            operation.external_receipt_hash(),
            expected,
            "{}",
            operation.instrument_id().as_str()
        );
    }
    let evidence: SettleEvidence = serde_json::from_value(settle_outcome.evidence).unwrap();
    for progress in &evidence.settlement_progress {
        let expected = match progress.instrument_id.as_str() {
            "storage_rebate" => Some(receipt_hash.as_str()),
            _ => None,
        };
        assert_eq!(
            progress.external_receipt_hash.as_deref(),
            expected,
            "{}",
            progress.instrument_id.as_str()
        );
    }

    // A second run of the same tenant: its leg is answered with the receipt
    // the first run's leg already recorded.
    let (second, _evidence) = run_to_settle_ready(&service, &tenant).await;
    let calls_before = storage_rebate.calls();
    let retried = service
        .process_run(&tenant, second.run_id)
        .await
        .unwrap()
        .expect("Settle retries after the reused receipt");
    assert_eq!(retried.state, PipelineRunState::Retry);
    assert_eq!(
        retried.last_error_label.as_deref(),
        Some("settlement_operation_retry")
    );
    assert_eq!(
        retried.attempt_count,
        second.attempt_count + 1,
        "a reused receipt fails closed and is charged"
    );
    let rows = settlement_rows(&backend, &tenant, second.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "failed");
    assert_eq!(
        leg_label(&rows, "storage_rebate"),
        Some(PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL)
    );
    assert!(leg_field(&rows, "storage_rebate", "external_receipt_hash").is_null());
    assert!(leg_field(&rows, "storage_rebate", "result_ref_hash").is_null());
    assert_eq!(leg_state(&rows, "trace_credit"), "complete");
    assert_eq!(storage_rebate.calls(), calls_before + 1);

    force_due(&backend, &tenant, second.run_id).await;
    let again = service
        .process_run(&tenant, second.run_id)
        .await
        .unwrap()
        .expect("Settle retries again");
    assert_eq!(again.state, PipelineRunState::Retry);
    assert_eq!(
        storage_rebate.calls(),
        calls_before + 1,
        "the leg is never dispatched again"
    );
    let first_rows = settlement_rows(&backend, &tenant, first.run_id).await;
    assert_eq!(
        leg_field(&first_rows, "storage_rebate", "external_receipt_hash").as_str(),
        Some(receipt_hash.as_str()),
        "the first leg keeps its receipt"
    );
}

/// The database holds the external receipt hash to its contract: only a
/// `complete` leg carries one, it is a lowercase SHA-256 reference, one hash
/// answers one leg of a tenant (another tenant may record the same hash),
/// and a recorded hash never changes.
#[tokio::test]
async fn external_receipt_hashes_are_checked_by_the_database() {
    let Some(backend) = runtime_backend(2).await else {
        return;
    };
    let tenant = format!("external-receipt-db-{}", uuid::Uuid::new_v4());
    let other_tenant = format!("external-receipt-db-other-{}", uuid::Uuid::new_v4());
    let run = seed_run(&backend, &tenant, uuid::Uuid::new_v4()).await;
    let other_run = seed_run(&backend, &other_tenant, uuid::Uuid::new_v4()).await;
    let receipt_hash = dependency_content_hash(b"external-receipt-db-check");

    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for the external receipt test");

    const INSERT: &str = "INSERT INTO pipeline_run_settlements (
        tenant_id, run_id, instrument_id, atomic_units, operation_ref_hash,
        result_ref_hash, operation_state, dispatched_at, payout_rail, external_receipt_hash
    ) VALUES ($1,$2,$3,5,$4,$5,$6,NOW(),'none',$7)";
    let insert = |instrument: &'static str, state: &'static str, external: String| {
        let result_ref = (state == "complete")
            .then(|| dependency_content_hash(format!("result:{instrument}").as_bytes()));
        (
            instrument,
            dependency_content_hash(format!("operation:{instrument}").as_bytes()),
            result_ref,
            state,
            external,
        )
    };

    // A complete leg with a well-formed hash is accepted.
    let tx = tenant_tx(&mut client, &tenant).await;
    let (instrument, operation, result, state, external) =
        insert("storage_rebate", "complete", receipt_hash.clone());
    tx.execute(
        INSERT,
        &[
            &tenant,
            &run.run_id,
            &instrument,
            &operation,
            &result,
            &state,
            &external,
        ],
    )
    .await
    .expect("a complete leg records its external receipt hash");
    tx.commit().await.unwrap();

    // Refused: a hash on a leg that is not complete, a malformed hash, and a
    // second leg of the tenant with the same hash.
    for (instrument, state, external, code, constraint) in [
        (
            "storage_bonus",
            "pending",
            dependency_content_hash(b"external-receipt-pending"),
            tokio_postgres::error::SqlState::CHECK_VIOLATION,
            "pipeline_run_settlements_external_receipt_shape",
        ),
        (
            "storage_bonus",
            "complete",
            "0xabc".to_string(),
            tokio_postgres::error::SqlState::CHECK_VIOLATION,
            "pipeline_run_settlements_external_receipt_shape",
        ),
        (
            "storage_bonus",
            "complete",
            receipt_hash.clone(),
            tokio_postgres::error::SqlState::UNIQUE_VIOLATION,
            "pipeline_run_settlements_external_receipt_unique",
        ),
    ] {
        let tx = tenant_tx(&mut client, &tenant).await;
        let (instrument, operation, result, state, external) = insert(instrument, state, external);
        let error = tx
            .execute(
                INSERT,
                &[
                    &tenant,
                    &run.run_id,
                    &instrument,
                    &operation,
                    &result,
                    &state,
                    &external,
                ],
            )
            .await
            .expect_err("the database refuses the leg");
        let db = error.as_db_error().expect("database refusal");
        assert_eq!(db.code(), &code, "{state} {external}");
        assert_eq!(db.constraint(), Some(constraint), "{db:?}");
        tx.rollback().await.unwrap();
    }

    // Another tenant may record the same hash.
    let tx = tenant_tx(&mut client, &other_tenant).await;
    let (instrument, operation, result, state, external) =
        insert("storage_rebate", "complete", receipt_hash.clone());
    tx.execute(
        INSERT,
        &[
            &other_tenant,
            &other_run.run_id,
            &instrument,
            &operation,
            &result,
            &state,
            &external,
        ],
    )
    .await
    .expect("the uniqueness is per tenant");
    tx.commit().await.unwrap();

    // A recorded hash never changes.
    let tx = tenant_tx(&mut client, &tenant).await;
    let error = tx
        .execute(
            "UPDATE pipeline_run_settlements
                SET external_receipt_hash = $3
              WHERE tenant_id = $1 AND run_id = $2 AND instrument_id = 'storage_rebate'",
            &[
                &tenant,
                &run.run_id,
                &dependency_content_hash(b"external-receipt-replacement"),
            ],
        )
        .await
        .expect_err("a recorded external receipt hash is immutable");
    assert_eq!(
        db_error_message(&error),
        "pipeline settlement identity is immutable"
    );
    tx.rollback().await.unwrap();
}

/// A Trace Credit leg pays only through the ledger row that commits with
/// its completion. When its adapter answers with a receipt that does not
/// answer the request, the leg fails closed as `settlement_result_mismatch`
/// with no ledger row, is never dispatched again, and every attempt while it
/// exists is charged. When the attempts run out, the failure path forfeits
/// it as `run_failed`: nothing was paid. No reconciling call is made for it.
#[tokio::test]
async fn a_trace_credit_leg_with_a_mismatched_receipt_is_forfeited_when_its_run_fails() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let storage_rebate = CountingSettlementAdapter::new(RecordingSettlementAdapter::new(
        InstrumentId::new("storage_rebate").unwrap(),
        "recording_storage_rebate_test_only",
        "none",
    ));
    let trace_credit = CountingSettlementAdapter::new(Arc::new(MismatchingSettlementAdapter {
        instrument_id: InstrumentId::trace_credit(),
    }));
    let service = test_service_with_adapters(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        vec![
            storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            trace_credit.clone() as Arc<dyn SettlementAdapter>,
        ],
    )
    .await;
    let tenant = format!("settle-credit-mismatch-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;
    let settle_attempts = run.max_attempts - run.attempt_count;
    assert!(settle_attempts >= 2, "the fixture leaves Settle a retry");

    for attempt in 1..=settle_attempts {
        force_due(&backend, &tenant, run.run_id).await;
        let processed = service
            .process_run(&tenant, run.run_id)
            .await
            .unwrap()
            .expect("the Settle attempt runs");
        if attempt < settle_attempts {
            assert_eq!(
                processed.state,
                PipelineRunState::Retry,
                "attempt {attempt}"
            );
            assert_eq!(
                processed.last_error_label.as_deref(),
                Some("settlement_operation_retry"),
                "attempt {attempt}"
            );
            assert_eq!(
                processed.attempt_count,
                run.attempt_count + attempt,
                "attempt {attempt}: the retry is charged"
            );
            let rows = settlement_rows(&backend, &tenant, run.run_id).await;
            assert_eq!(leg_state(&rows, "trace_credit"), "failed");
            assert_eq!(
                leg_label(&rows, "trace_credit"),
                Some(PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL)
            );
        }
        assert_eq!(trace_credit.calls(), 1, "attempt {attempt}");
    }

    let failed = service
        .store()
        .get_run(&tenant, run.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.state, PipelineRunState::Failed);
    assert_eq!(
        failed.last_error_label.as_deref(),
        Some(PIPELINE_ATTEMPTS_EXHAUSTED_LABEL)
    );
    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "trace_credit"), "forfeited");
    assert_eq!(
        leg_label(&rows, "trace_credit"),
        Some(PIPELINE_SETTLEMENT_RUN_FAILED_LABEL)
    );
    assert!(leg_dispatched(&rows, "trace_credit"));
    assert!(leg_holds_no_lease(&rows, "trace_credit"));
    assert!(leg_field(&rows, "trace_credit", "result_ref_hash").is_null());
    assert!(leg_field(&rows, "trace_credit", "credit_event_id").is_null());
    assert_eq!(leg_state(&rows, "storage_rebate"), "complete");
    assert_eq!(
        count_credit_ledger_rows_for_run(&backend, &tenant, run.run_id).await,
        0,
        "no ledger row: nothing was paid"
    );
    assert_eq!(
        trace_credit.calls(),
        1,
        "the adapter was called once, and no reconciling call was made"
    );
    assert_eq!(storage_rebate.calls(), 1);
}

/// The reconciling call a worker makes before it fails a run records a
/// receipt only when that receipt answers this leg alone. A receipt whose
/// external hash another leg of the tenant already recorded does not
/// reconcile the leg: it ends `failed` / `settlement_unreconciled`, with no
/// result and no external receipt hash, and the first leg keeps its receipt.
#[tokio::test]
async fn a_reused_external_receipt_does_not_reconcile_a_leg() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let receipt_hash = dependency_content_hash(b"external-receipt-reused-on-reconcile");
    let rebate_recording = RecordingSettlementAdapter::new(
        InstrumentId::new("storage_rebate").unwrap(),
        "recording_storage_rebate_test_only",
        "none",
    );
    let storage_rebate = CountingSettlementAdapter::new(Arc::new(ExternalReceiptAdapter {
        inner: rebate_recording.clone(),
        receipt_hash: receipt_hash.clone(),
    }));
    let service = test_service_with_adapters(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        vec![
            storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            RecordingSettlementAdapter::new(
                InstrumentId::trace_credit(),
                "recording_trace_credit_test_only",
                "none",
            ) as Arc<dyn SettlementAdapter>,
        ],
    )
    .await;
    let tenant = format!("settle-reconcile-reused-receipt-{}", uuid::Uuid::new_v4());

    let (first, _evidence) = run_to_settle_ready(&service, &tenant).await;
    let settled = service
        .process_run(&tenant, first.run_id)
        .await
        .unwrap()
        .expect("Settle runs");
    assert_eq!(settled.state, PipelineRunState::Complete);

    // The second run's leg is dispatched and left waiting by an
    // `Unavailable` answer.
    let (second, _evidence) = run_to_settle_ready(&service, &tenant).await;
    rebate_recording.fail_next();
    let waited = service
        .process_run(&tenant, second.run_id)
        .await
        .unwrap()
        .expect("the Settle attempt waits");
    assert_eq!(waited.state, PipelineRunState::Retry);
    let rows = settlement_rows(&backend, &tenant, second.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "retry");
    assert!(leg_dispatched(&rows, "storage_rebate"));
    assert_eq!(leg_state(&rows, "trace_credit"), "complete");

    // The run then fails on its tampered package. The reconciling call is
    // answered with the receipt the first run's leg already recorded.
    let calls_before = storage_rebate.calls();
    tamper_stored_bundle_package(&tenant, &second.bundle_id).await;
    force_due(&backend, &tenant, second.run_id).await;
    let failed = service
        .process_run(&tenant, second.run_id)
        .await
        .unwrap()
        .expect("the run fails on its tampered package");
    assert_eq!(failed.state, PipelineRunState::Failed);
    assert_eq!(
        failed.last_error_label.as_deref(),
        Some("bundle_package_invalid")
    );
    assert_eq!(
        storage_rebate.calls(),
        calls_before + 1,
        "the reconciling call was made"
    );
    let rows = settlement_rows(&backend, &tenant, second.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "failed");
    assert_eq!(
        leg_label(&rows, "storage_rebate"),
        Some(PIPELINE_SETTLEMENT_UNRECONCILED_LABEL)
    );
    assert!(leg_field(&rows, "storage_rebate", "result_ref_hash").is_null());
    assert!(leg_field(&rows, "storage_rebate", "external_receipt_hash").is_null());
    assert!(leg_holds_no_lease(&rows, "storage_rebate"));
    let first_rows = settlement_rows(&backend, &tenant, first.run_id).await;
    assert_eq!(
        leg_field(&first_rows, "storage_rebate", "external_receipt_hash").as_str(),
        Some(receipt_hash.as_str()),
        "the first leg keeps its receipt"
    );
}

/// A `failed` leg with no label is a state no writer produces today, but
/// the schema allows it. The SQL predicates treat it the way their Rust
/// forms do: it is open to dispatch, so Step 6 leases and settles it, and it
/// is unresolved, so a failure path resolves it (here the claim sweep: the
/// dispatched external leg is `settlement_unreconciled`, the Trace Credit leg
/// is forfeited as `run_failed`).
#[tokio::test]
async fn a_failed_leg_without_a_label_is_dispatched_and_resolved() {
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

    let tenant = format!("settle-unlabeled-dispatch-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;
    {
        let mut client = backend.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, &tenant).await;
        tx.execute(
            "UPDATE pipeline_run_settlements
                SET operation_state = 'failed', last_error_label = NULL
              WHERE tenant_id = $1 AND run_id = $2 AND instrument_id = 'storage_rebate'",
            &[&tenant, &run.run_id],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }
    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle runs");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(adapters[0].requests().len(), 1, "the leg was dispatched");
    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "complete");

    let sweep_tenant = format!("settle-unlabeled-sweep-{}", uuid::Uuid::new_v4());
    let (swept_run, _evidence) = run_to_settle_ready(&service, &sweep_tenant).await;
    {
        let mut client = backend.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, &sweep_tenant).await;
        tx.execute(
            "UPDATE pipeline_runs
                SET state = 'leased', lease_token = $3,
                    lease_expires_at = NOW() - INTERVAL '1 second',
                    attempt_count = max_attempts
              WHERE tenant_id = $1 AND run_id = $2",
            &[&sweep_tenant, &swept_run.run_id, &uuid::Uuid::new_v4()],
        )
        .await
        .unwrap();
        tx.execute(
            "UPDATE pipeline_run_settlements
                SET operation_state = 'failed', last_error_label = NULL,
                    dispatched_at = CASE
                        WHEN instrument_id = 'storage_rebate' THEN NOW() - INTERVAL '2 seconds'
                    END
              WHERE tenant_id = $1 AND run_id = $2",
            &[&sweep_tenant, &swept_run.run_id],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }
    let claimed = service.process_one(&sweep_tenant).await.unwrap();
    assert!(claimed.is_none(), "an exhausted run is not claimed");
    let rows = settlement_rows(&backend, &sweep_tenant, swept_run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "failed");
    assert_eq!(
        leg_label(&rows, "storage_rebate"),
        Some(PIPELINE_SETTLEMENT_UNRECONCILED_LABEL)
    );
    assert_eq!(leg_state(&rows, "trace_credit"), "forfeited");
    assert_eq!(
        leg_label(&rows, "trace_credit"),
        Some(PIPELINE_SETTLEMENT_RUN_FAILED_LABEL)
    );
    assert_eq!(
        adapters[0].requests().len(),
        1,
        "the sweep calls no adapter"
    );
}

/// A `settlement_unreconciled` leg on a run that is still live -- left
/// by a failure path that lost its lease before it failed the run -- is
/// dispatched again by the next Settle attempt, since the run did not
/// fail; a matching result then completes it.
#[tokio::test]
async fn an_unreconciled_leg_on_a_live_run_is_dispatched_again() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let rebate_recording = RecordingSettlementAdapter::new(
        InstrumentId::new("storage_rebate").unwrap(),
        "recording_storage_rebate_test_only",
        "none",
    );
    let storage_rebate = CountingSettlementAdapter::new(rebate_recording.clone());
    let service = test_service_with_adapters(
        backend.clone(),
        artifact_store(&dir),
        scored_config(false),
        vec![
            storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            RecordingSettlementAdapter::new(
                InstrumentId::trace_credit(),
                "recording_trace_credit_test_only",
                "none",
            ) as Arc<dyn SettlementAdapter>,
        ],
    )
    .await;
    let tenant = format!("settle-unreconciled-live-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;
    {
        let mut client = backend.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, &tenant).await;
        tx.execute(
            "UPDATE pipeline_run_settlements
                SET operation_state = 'failed',
                    last_error_label = 'settlement_unreconciled',
                    dispatched_at = NOW() - INTERVAL '1 second'
              WHERE tenant_id = $1 AND run_id = $2 AND instrument_id = 'storage_rebate'",
            &[&tenant, &run.run_id],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }

    let settled = service
        .process_run(&tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle runs");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(storage_rebate.calls(), 1);
    let rows = settlement_rows(&backend, &tenant, run.run_id).await;
    assert_eq!(leg_state(&rows, "storage_rebate"), "complete");
    assert_eq!(leg_label(&rows, "storage_rebate"), None);
}

/// Drives one run through every phase to completion with both a Trace
/// Credit leg and a second-instrument leg, so its tenant ends up with a
/// pipeline row of every kind the versioned-pipeline migrations add: a
/// `pipeline_runs` row and its four `phase_outcomes`, two settled
/// `pipeline_run_settlements` legs (and the `trace_credit_ledger` /
/// `trace_credit_settlement_batches` rows the Trace Credit leg creates),
/// the tenant's `pipeline_bundle_packages` / `pipeline_active_bundles` /
/// `pipeline_bundle_policy_status` rows (`register_default_bundle`, inside
/// `submit_registered`), its committed `pipeline_receipt_artifacts` row,
/// and its `pipeline_admission_usage` row.
async fn run_with_every_pipeline_row_kind(
    service: &PipelineService,
    tenant: &str,
) -> PipelineRunRecord {
    let (run, _evidence) = run_to_settle_ready(service, tenant).await;
    let settled = service
        .process_run(tenant, run.run_id)
        .await
        .unwrap()
        .expect("Settle runs to completion");
    assert_eq!(settled.state, PipelineRunState::Complete);
    settled
}

/// Deleting the submission that a fully-settled run belongs to must succeed
/// and take every pipeline row for it along, not stop partway at an
/// immutability trigger or a foreign key that has not yet seen its sibling
/// cascade finish.
#[tokio::test]
async fn deleting_a_submission_with_pipeline_rows_succeeds() {
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
    let tenant = format!("delete-submission-{}", uuid::Uuid::new_v4());
    let run = run_with_every_pipeline_row_kind(&service, &tenant).await;

    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &tenant).await;
    tx.execute(
        "DELETE FROM trace_submissions WHERE tenant_id = $1 AND submission_id = $2",
        &[&tenant, &run.submission_id],
    )
    .await
    .expect("a submission with pipeline rows of every kind can be deleted");
    tx.commit().await.unwrap();

    assert_eq!(
        count_runs(&backend, &tenant).await,
        0,
        "the run cascaded away with its submission"
    );
}

/// The tenant-delete half of the same requirement: the cascade chain runs
/// from `trace_tenants` instead of `trace_submissions`, and the tenant also
/// owns `pipeline_bundle_packages` / `pipeline_active_bundles` /
/// `pipeline_bundle_policy_status` / `pipeline_admission_usage` rows
/// directly (not through a submission).
#[tokio::test]
async fn deleting_a_tenant_with_pipeline_rows_succeeds() {
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
    let tenant = format!("delete-tenant-{}", uuid::Uuid::new_v4());
    let _run = run_with_every_pipeline_row_kind(&service, &tenant).await;

    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &tenant).await;
    tx.execute("DELETE FROM trace_tenants WHERE tenant_id = $1", &[&tenant])
        .await
        .expect("a tenant with pipeline rows of every kind can be deleted");
    tx.commit().await.unwrap();

    let mut check_client = backend.trace_pool_for_test().get().await.unwrap();
    let check_tx = tenant_tx(&mut check_client, &tenant).await;
    let remaining: i64 = check_tx
        .query_one(
            "SELECT COUNT(*) FROM trace_tenants WHERE tenant_id = $1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);
    check_tx.commit().await.unwrap();
    assert_eq!(remaining, 0, "the tenant row itself is gone");
}

/// The append-only property the trigger exists for stays exactly as strict
/// as before: an outcome cannot be deleted out from under a run that is
/// still live, only cascaded away with it.
#[tokio::test]
async fn a_direct_outcome_delete_is_still_refused_while_its_run_exists() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (service, _index, _adapters) = test_service(
        backend.clone(),
        artifact_store(&dir),
        minimal_config(false),
        None,
    )
    .await;
    let tenant = format!("outcome-delete-refused-{}", uuid::Uuid::new_v4());
    let (run, _evidence) = run_to_settle_ready(&service, &tenant).await;

    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &tenant).await;
    let error = tx
        .execute(
            "DELETE FROM phase_outcomes WHERE tenant_id = $1 AND run_id = $2",
            &[&tenant, &run.run_id],
        )
        .await
        .expect_err("an outcome delete is refused while its run still exists");
    assert!(
        db_error_message(&error).contains("phase outcomes are immutable"),
        "unexpected error: {error:?}"
    );
}
