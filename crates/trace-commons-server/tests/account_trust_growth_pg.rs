// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Earned account trust against a real PostgreSQL: the facts the growth rule
//! reads (V84/V85). Explicit isolated database only; never falls back to
//! DATABASE_URL.

use chrono::{DateTime, TimeZone, Utc};
use std::sync::Arc;
use trace_commons_server::account_trust::{TrustAccount, TrustFactOutcome, TrustFactSource};
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use uuid::Uuid;

const URL_VAR: &str = "TRACE_COMMONS_ACCOUNT_TRUST_GROWTH_PG_TEST_DATABASE_URL";

fn config(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url: url.into(),
        pool_size: 8,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

struct Fixture {
    admin_db: PgBackend,
    admin: deadpool_postgres::Object,
    url: String,
    _serial: tokio::sync::MutexGuard<'static, ()>,
}

/// The workers enumerate every account in the database, so a test that runs
/// one would record or evaluate another test's fixtures mid-assertion. The
/// suite shares one database and runs one test at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn fixture() -> Option<Fixture> {
    let Ok(url) = std::env::var(URL_VAR) else {
        eprintln!("SKIPPED: isolated earned-trust PostgreSQL fixture required ({URL_VAR})");
        return None;
    };
    let serial = SERIAL.lock().await;
    let parsed = reqwest::Url::parse(&url).expect("test database URL");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(
        parsed
            .path()
            .starts_with("/admission_test_account_trust_growth")
    );
    let admin_db = PgBackend::new(&config(url.clone())).await.unwrap();
    admin_db.run_migrations().await.unwrap();
    let admin = admin_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    Some(Fixture {
        admin_db,
        admin,
        url,
        _serial: serial,
    })
}

/// A NOBYPASSRLS login that holds exactly one NOLOGIN role.
async fn login_with_role(fx: &Fixture, login: &str, role: &str) -> Arc<PgBackend> {
    fx.admin
        .batch_execute(&format!(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{login}') \
             THEN CREATE ROLE {login} LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$; \
             GRANT {role} TO {login};"
        ))
        .await
        .unwrap();
    let mut url = reqwest::Url::parse(&fx.url).unwrap();
    url.set_username(login).unwrap();
    Arc::new(PgBackend::new(&config(url.into())).await.unwrap())
}

fn anchored_tenant() -> String {
    format!("near-{}", Uuid::new_v4().simple().to_string().repeat(2))
}

fn principal() -> String {
    format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2))
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_780_000_000 + seconds, 0).unwrap()
}

async fn seed_account(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    principals: &[&str],
) -> Uuid {
    let account = Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_tenants(tenant_id) VALUES($1) ON CONFLICT DO NOTHING",
            &[&tenant],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2)",
            &[&tenant, &account],
        )
        .await
        .unwrap();
    for principal_ref in principals {
        admin
            .execute(
                "INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3)",
                &[&tenant, &account, principal_ref],
            )
            .await
            .unwrap();
    }
    account
}

async fn seed_submission(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    principal_ref: &str,
    status: &str,
    received_at: DateTime<Utc>,
) -> (Uuid, Uuid) {
    let submission = Uuid::new_v4();
    let trace = Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_submissions(tenant_id,submission_id,trace_id,auth_principal_ref,
                schema_version,consent_policy_version,retention_policy_id,status,privacy_risk,
                redaction_pipeline_version,redaction_hash,received_at)
             VALUES($1,$2,$3,$4,'v1','v1','test',$5,'low','test',$6,$7)",
            &[
                &tenant,
                &submission,
                &trace,
                &principal_ref,
                &status,
                &"a".repeat(64),
                &received_at,
            ],
        )
        .await
        .unwrap();
    (submission, trace)
}

#[allow(clippy::too_many_arguments)]
async fn seed_credit(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    submission: Uuid,
    trace: Uuid,
    event_type: &str,
    actor: &str,
    actor_role: &str,
    occurred_at: DateTime<Utc>,
) -> Uuid {
    let event = Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_credit_ledger(tenant_id,credit_event_id,submission_id,trace_id,
                credit_account_ref,event_type,points_delta,reason,actor_principal_ref,actor_role,
                settlement_state,occurred_at)
             VALUES($1,$2,$3,$4,'fixture',$5,'0','fixture',$6,$7,'pending',$8)",
            &[
                &tenant,
                &event,
                &submission,
                &trace,
                &event_type,
                &actor,
                &actor_role,
                &occurred_at,
            ],
        )
        .await
        .unwrap();
    event
}

async fn seed_gate_decision(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    submission: Uuid,
    passed: bool,
    decided_at: DateTime<Utc>,
) -> Uuid {
    let decision = Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_gate_decisions(tenant_id,decision_id,submission_id,gate_policy_version,
                gate_version_hash,perplexity_micros,tail_fraction_micros,perplexity_passed,
                novelty_score_micros,nearest_neighbor_hash,novelty_passed,embedding_evidence_hash,
                attestation_chain_hash,decided_at)
             VALUES($1,$2,$3,'gate-v1',$4,1,1,$5,1,$4,TRUE,$4,$4,$6)",
            &[
                &tenant,
                &decision,
                &submission,
                &"a".repeat(64),
                &passed,
                &decided_at,
            ],
        )
        .await
        .unwrap();
    decision
}

async fn seed_review_approval(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    submission: Uuid,
    reviewer: &str,
) {
    admin
        .execute(
            "INSERT INTO trace_audit_events(tenant_id,audit_sequence,audit_event_id,
                actor_principal_ref,actor_role,action,submission_id,metadata_json)
             VALUES($1,(SELECT COALESCE(max(audit_sequence),0)+1 FROM trace_audit_events
                         WHERE tenant_id=$1),$2,$3,'reviewer','review',$4,$5)",
            &[
                &tenant,
                &Uuid::new_v4(),
                &reviewer,
                &submission,
                &serde_json::json!({
                    "kind": "review_decision",
                    "decision": "accepted",
                    "resulting_status": "accepted",
                    "reason_code": null,
                }),
            ],
        )
        .await
        .unwrap();
}

async fn fact_occurred_at(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    account: Uuid,
    source: TrustFactSource,
) -> DateTime<Utc> {
    admin
        .query_one(
            "SELECT occurred_at FROM trace_account_trust_facts
              WHERE tenant_id=$1 AND account_id=$2 AND source_kind=$3 AND source_id=$4",
            &[&tenant, &account, &source.kind(), &source.source_id()],
        )
        .await
        .unwrap()
        .get(0)
}

async fn enumerate_all(worker: &PgBackend) -> Vec<TrustAccount> {
    let mut out = Vec::new();
    let mut after: Option<TrustAccount> = None;
    loop {
        let page = worker
            .list_account_trust_worker_accounts(after.as_ref(), 2)
            .await
            .unwrap();
        if page.is_empty() {
            return out;
        }
        after = page.last().cloned();
        out.extend(page);
    }
}

#[tokio::test]
async fn every_fact_kind_is_verified_from_its_server_row_with_its_source_time() {
    let Some(fx) = fixture().await else {
        return;
    };
    let admin = &fx.admin;
    let worker = login_with_role(
        &fx,
        "tc_account_trust_worker_login",
        "trace_account_trust_worker",
    )
    .await;
    let runtime = login_with_role(
        &fx,
        "tc_account_trust_runtime_login",
        "trace_account_admission_runtime",
    )
    .await;

    let tenant = anchored_tenant();
    let owner = principal();
    let second_device = principal();
    let reviewer = principal();
    let account = seed_account(admin, &tenant, &[&owner, &second_device]).await;
    let other_tenant = anchored_tenant();
    let other_owner = principal();
    let other_account = seed_account(admin, &other_tenant, &[&other_owner]).await;
    // Neither a legacy tenant nor a closed account is ever enumerated.
    let legacy_tenant = format!("tenant-{}", Uuid::new_v4());
    let legacy_account = seed_account(admin, &legacy_tenant, &[&principal()]).await;
    let closed_account = seed_account(admin, &tenant, &[&principal()]).await;
    admin
        .execute(
            "UPDATE trace_accounts SET closed_at=clock_timestamp() WHERE tenant_id=$1 AND account_id=$2",
            &[&tenant, &closed_account],
        )
        .await
        .unwrap();

    let listed = enumerate_all(&worker).await;
    let key = |a: &TrustAccount| (a.tenant_id().to_string(), a.account_id());
    let listed_keys: Vec<_> = listed.iter().map(key).collect();
    assert!(listed_keys.contains(&(tenant.clone(), account)));
    assert!(listed_keys.contains(&(other_tenant.clone(), other_account)));
    assert!(!listed_keys.contains(&(legacy_tenant.clone(), legacy_account)));
    assert!(!listed_keys.contains(&(tenant.clone(), closed_account)));
    let mut sorted = listed_keys.clone();
    sorted.sort();
    assert_eq!(sorted, listed_keys, "enumeration pages in key order");
    let trust = listed
        .iter()
        .find(|a| key(a) == (tenant.clone(), account))
        .unwrap()
        .clone();
    let other_trust = listed
        .iter()
        .find(|a| key(a) == (other_tenant.clone(), other_account))
        .unwrap()
        .clone();

    // accepted, automated: occurred_at is the credit event's time.
    let (accepted, accepted_trace) =
        seed_submission(admin, &tenant, &owner, "accepted", at(0)).await;
    seed_credit(
        admin,
        &tenant,
        accepted,
        accepted_trace,
        "accepted",
        &owner,
        "system",
        at(10),
    )
    .await;
    // a gate evaluation: occurred_at is decided_at.
    let decision = seed_gate_decision(admin, &tenant, accepted, true, at(20)).await;
    // withdrawn: status moves to revoked, withdrawn_at is what marks it.
    let (withdrawn, withdrawn_trace) =
        seed_submission(admin, &tenant, &owner, "revoked", at(0)).await;
    seed_credit(
        admin,
        &tenant,
        withdrawn,
        withdrawn_trace,
        "accepted",
        &owner,
        "system",
        at(5),
    )
    .await;
    admin
        .execute(
            "UPDATE trace_submissions SET withdrawn_at=$3, revoked_at=$3 WHERE tenant_id=$1 AND submission_id=$2",
            &[&tenant, &withdrawn, &at(30)],
        )
        .await
        .unwrap();
    // revoked by an operator: revoked_at, no withdrawn_at.
    let (revoked, _) = seed_submission(admin, &tenant, &second_device, "revoked", at(0)).await;
    admin
        .execute(
            "UPDATE trace_submissions SET revoked_at=$3 WHERE tenant_id=$1 AND submission_id=$2",
            &[&tenant, &revoked, &at(40)],
        )
        .await
        .unwrap();
    // quarantined at submit time: no review, so its receipt time.
    let (quarantined, _) = seed_submission(admin, &tenant, &owner, "quarantined", at(50)).await;
    // an abuse penalty on the accepted submission.
    let penalty = seed_credit(
        admin,
        &tenant,
        accepted,
        accepted_trace,
        "abuse_penalty",
        &reviewer,
        "reviewer",
        at(60),
    )
    .await;
    // Operator credit events are never facts.
    seed_credit(
        admin,
        &tenant,
        accepted,
        accepted_trace,
        "reviewer_bonus",
        &reviewer,
        "reviewer",
        at(61),
    )
    .await;
    // accepted through a review approved by the account's own second device.
    let (self_reviewed, self_trace) =
        seed_submission(admin, &tenant, &owner, "accepted", at(0)).await;
    seed_credit(
        admin,
        &tenant,
        self_reviewed,
        self_trace,
        "accepted",
        &owner,
        "system",
        at(70),
    )
    .await;
    seed_review_approval(admin, &tenant, self_reviewed, &second_device).await;
    // accepted through a review approved by someone else: qualifies.
    let (reviewed, reviewed_trace) =
        seed_submission(admin, &tenant, &owner, "accepted", at(0)).await;
    seed_credit(
        admin,
        &tenant,
        reviewed,
        reviewed_trace,
        "accepted",
        &owner,
        "system",
        at(75),
    )
    .await;
    seed_review_approval(admin, &tenant, reviewed, &reviewer).await;
    // accepted with a credit event a human issued and no approval row to
    // name who approved it: fails closed.
    let (unknown, unknown_trace) = seed_submission(admin, &tenant, &owner, "accepted", at(0)).await;
    seed_credit(
        admin,
        &tenant,
        unknown,
        unknown_trace,
        "accepted",
        &reviewer,
        "reviewer",
        at(80),
    )
    .await;

    let mut candidates = worker
        .list_account_trust_fact_candidates(&trust, 100)
        .await
        .unwrap();
    candidates.sort_by_key(|s| (s.kind(), s.source_id()));
    let mut expected = vec![
        TrustFactSource::AcceptedSubmission(accepted),
        TrustFactSource::AcceptedSubmissionStatus(accepted),
        TrustFactSource::AcceptedSubmissionStatus(self_reviewed),
        TrustFactSource::AcceptedSubmissionStatus(reviewed),
        TrustFactSource::AcceptedSubmissionStatus(unknown),
        TrustFactSource::GateEvaluation(decision),
        TrustFactSource::SubmissionWithdrawn(withdrawn),
        TrustFactSource::SubmissionRevoked(revoked),
        TrustFactSource::SubmissionQuarantined(quarantined),
        TrustFactSource::AbusePenalty(penalty),
        TrustFactSource::AcceptedSubmission(self_reviewed),
        TrustFactSource::AcceptedSubmission(reviewed),
        TrustFactSource::AcceptedSubmission(unknown),
    ];
    expected.sort_by_key(|s| (s.kind(), s.source_id()));
    assert_eq!(
        candidates, expected,
        "every unrecorded source, and nothing else"
    );
    assert_eq!(
        worker
            .list_account_trust_fact_candidates(&trust, 2)
            .await
            .unwrap()
            .len(),
        2,
        "the limit bounds one call"
    );

    let expectations = [
        (
            TrustFactSource::AcceptedSubmissionStatus(accepted),
            TrustFactOutcome::Accepted,
            at(0),
        ),
        (
            TrustFactSource::AcceptedSubmissionStatus(self_reviewed),
            TrustFactOutcome::AcceptedSelfReviewed,
            at(0),
        ),
        (
            TrustFactSource::AcceptedSubmissionStatus(reviewed),
            TrustFactOutcome::Accepted,
            at(0),
        ),
        (
            TrustFactSource::AcceptedSubmissionStatus(unknown),
            TrustFactOutcome::AcceptedApproverUnknown,
            at(0),
        ),
        (
            TrustFactSource::AcceptedSubmission(accepted),
            TrustFactOutcome::Accepted,
            at(10),
        ),
        (
            TrustFactSource::GateEvaluation(decision),
            TrustFactOutcome::EvaluatedPassed,
            at(20),
        ),
        (
            TrustFactSource::SubmissionWithdrawn(withdrawn),
            TrustFactOutcome::Withdrawn,
            at(30),
        ),
        (
            TrustFactSource::SubmissionRevoked(revoked),
            TrustFactOutcome::Revoked,
            at(40),
        ),
        (
            TrustFactSource::SubmissionQuarantined(quarantined),
            TrustFactOutcome::Quarantined,
            at(50),
        ),
        (
            TrustFactSource::AbusePenalty(penalty),
            TrustFactOutcome::Penalized,
            at(60),
        ),
        (
            TrustFactSource::AcceptedSubmission(self_reviewed),
            TrustFactOutcome::AcceptedSelfReviewed,
            at(70),
        ),
        (
            TrustFactSource::AcceptedSubmission(reviewed),
            TrustFactOutcome::Accepted,
            at(75),
        ),
        (
            TrustFactSource::AcceptedSubmission(unknown),
            TrustFactOutcome::AcceptedApproverUnknown,
            at(80),
        ),
    ];
    for (source, outcome, occurred) in expectations {
        let (left, right) = tokio::join!(
            worker.record_account_trust_fact(&trust, source),
            worker.record_account_trust_fact(&trust, source)
        );
        assert_eq!(left.unwrap(), Some(outcome), "{source:?}");
        assert_eq!(right.unwrap(), Some(outcome), "{source:?} concurrently");
        assert_eq!(
            fact_occurred_at(admin, &tenant, account, source).await,
            occurred,
            "{source:?} carries its source row's time, not the recorder's"
        );
    }
    let recorded: i64 = admin
        .query_one(
            "SELECT count(*) FROM trace_account_trust_facts WHERE tenant_id=$1 AND account_id=$2",
            &[&tenant, &account],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        recorded, 13,
        "each source, including four status acceptances, recorded exactly once"
    );
    assert!(
        worker
            .list_account_trust_fact_candidates(&trust, 100)
            .await
            .unwrap()
            .is_empty(),
        "a re-run finds nothing new"
    );

    // A source is verified against its own kind's server row.
    let (plain, _) = seed_submission(admin, &tenant, &owner, "accepted", at(90)).await;
    for wrong in [
        TrustFactSource::SubmissionWithdrawn(plain),
        TrustFactSource::SubmissionRevoked(plain),
        TrustFactSource::SubmissionQuarantined(plain),
        TrustFactSource::AbusePenalty(plain),
        TrustFactSource::SubmissionRevoked(withdrawn),
        TrustFactSource::AcceptedSubmission(plain),
    ] {
        assert_eq!(
            worker
                .record_account_trust_fact(&trust, wrong)
                .await
                .unwrap(),
            None,
            "{wrong:?} has no matching server row"
        );
    }
    // Another tenant's account cannot record this tenant's rows.
    assert_eq!(
        worker
            .record_account_trust_fact(
                &other_trust,
                TrustFactSource::SubmissionQuarantined(quarantined)
            )
            .await
            .unwrap(),
        None
    );
    assert!(
        worker
            .list_account_trust_fact_candidates(&other_trust, 100)
            .await
            .unwrap()
            .is_empty()
    );

    // An adverse fact is recorded even after the principal is unlinked; a
    // positive one needs a live link, as V77 required.
    let (late, late_trace) =
        seed_submission(admin, &tenant, &second_device, "quarantined", at(95)).await;
    seed_credit(
        admin,
        &tenant,
        late,
        late_trace,
        "abuse_penalty",
        &reviewer,
        "reviewer",
        at(96),
    )
    .await;
    admin
        .execute(
            "UPDATE trace_account_principals SET unlinked_at=clock_timestamp() WHERE tenant_id=$1 AND principal_ref=$2",
            &[&tenant, &second_device],
        )
        .await
        .unwrap();
    assert_eq!(
        worker
            .record_account_trust_fact(&trust, TrustFactSource::SubmissionQuarantined(late))
            .await
            .unwrap(),
        Some(TrustFactOutcome::Quarantined)
    );

    // Least privilege: the worker reads facts only through the definer
    // functions, and the admission runtime cannot enumerate accounts.
    let worker_raw = worker
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    assert!(
        worker_raw
            .query("SELECT 1 FROM trace_account_trust_facts", &[])
            .await
            .is_err(),
        "the worker role holds no table privilege on facts"
    );
    assert!(
        worker_raw
            .query("SELECT 1 FROM trace_accounts", &[])
            .await
            .is_err()
    );
    let runtime_raw = runtime
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    assert!(
        runtime_raw
            .query(
                "SELECT * FROM trace_account_trust_worker_accounts(NULL,NULL,10)",
                &[]
            )
            .await
            .is_err(),
        "the admission runtime role is not widened"
    );
    assert!(
        runtime_raw
            .query("SELECT 1 FROM trace_account_trust_facts", &[])
            .await
            .is_err()
    );

    // A hard delete of a submission takes its facts with it (V43 keeps that
    // delete possible; a trust fact must not be what blocks it).
    admin
        .execute(
            "DELETE FROM trace_submissions WHERE tenant_id=$1 AND submission_id=$2",
            &[&tenant, &quarantined],
        )
        .await
        .unwrap();
    let left: i64 = admin
        .query_one(
            "SELECT count(*) FROM trace_account_trust_facts WHERE tenant_id=$1 AND submission_id=$2",
            &[&tenant, &quarantined],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(left, 0);
    drop(fx.admin_db);
}

#[tokio::test]
async fn facts_table_rejects_inconsistent_kinds_and_is_tenant_isolated() {
    let Some(fx) = fixture().await else {
        return;
    };
    let admin = &fx.admin;
    let tenant = anchored_tenant();
    let owner = principal();
    let account = seed_account(admin, &tenant, &[&owner]).await;
    let (submission, _) = seed_submission(admin, &tenant, &owner, "revoked", at(0)).await;
    for (kind, source, outcome, version) in [
        ("submission_withdrawn", submission, "accepted", None::<&str>),
        ("submission_withdrawn", Uuid::new_v4(), "withdrawn", None),
        ("submission_revoked", submission, "revoked", Some("v1")),
        ("abuse_penalty", Uuid::new_v4(), "withdrawn", None),
        ("gate_evaluation", Uuid::new_v4(), "penalized", Some("v1")),
        ("something_else", submission, "accepted", None),
    ] {
        assert!(
            admin
                .execute(
                    "INSERT INTO trace_account_trust_facts(tenant_id,account_id,source_kind,source_id,
                        submission_id,outcome,evaluator_version,occurred_at)
                     VALUES($1,$2,$3,$4,$5,$6,$7,$8)",
                    &[&tenant, &account, &kind, &source, &submission, &outcome, &version, &at(1)],
                )
                .await
                .is_err(),
            "{kind}/{outcome} must be refused"
        );
    }
    assert!(
        admin
            .execute(
                "INSERT INTO trace_account_trust_facts(tenant_id,account_id,source_kind,source_id,
                    submission_id,outcome,evaluator_version)
                 VALUES($1,$2,'submission_revoked',$3,$3,'revoked',NULL)",
                &[&tenant, &account, &submission],
            )
            .await
            .is_err(),
        "occurred_at is required"
    );
    let index: bool = admin
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE tablename='trace_account_trust_facts'
                AND indexdef LIKE '%(tenant_id, account_id, occurred_at)%')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(index);
}

#[tokio::test]
async fn recorder_run_is_idempotent_bounded_and_reports_only_counts() {
    use trace_commons_server::account_trust_growth::record_account_trust_facts;
    let Some(fx) = fixture().await else {
        return;
    };
    let admin = &fx.admin;
    let worker = login_with_role(
        &fx,
        "tc_account_trust_recorder_login",
        "trace_account_trust_worker",
    )
    .await;
    let tenant = anchored_tenant();
    let owner = principal();
    let account = seed_account(admin, &tenant, &[&owner]).await;
    let (accepted, trace) = seed_submission(admin, &tenant, &owner, "accepted", at(0)).await;
    seed_credit(
        admin,
        &tenant,
        accepted,
        trace,
        "accepted",
        &owner,
        "system",
        at(1),
    )
    .await;
    seed_gate_decision(admin, &tenant, accepted, true, at(2)).await;
    let (quarantined, _) = seed_submission(admin, &tenant, &owner, "quarantined", at(3)).await;

    let facts = || async {
        admin
            .query_one(
                "SELECT count(*) FROM trace_account_trust_facts WHERE tenant_id=$1 AND account_id=$2",
                &[&tenant, &account],
            )
            .await
            .unwrap()
            .get::<_, i64>(0)
    };
    let db: &dyn Database = worker.as_ref();

    let bounded = record_account_trust_facts(db, Some(0), false)
        .await
        .unwrap();
    assert!(bounded.limit_reached, "a zero limit visits nothing");
    assert_eq!(bounded.candidates, 0);

    let dry = record_account_trust_facts(db, Some(10_000), true)
        .await
        .unwrap();
    assert!(dry.dry_run);
    assert!(dry.candidates >= 3);
    assert!(dry.recorded_by_outcome.is_empty());
    assert_eq!(facts().await, 0, "a dry run records nothing");

    let run = record_account_trust_facts(db, Some(10_000), false)
        .await
        .unwrap();
    assert_eq!(run.failed, 0);
    assert!(
        run.recorded_by_outcome
            .get("accepted")
            .copied()
            .unwrap_or(0)
            >= 1
    );
    assert!(
        run.recorded_by_outcome
            .get("quarantined")
            .copied()
            .unwrap_or(0)
            >= 1
    );
    assert_eq!(facts().await, 4);

    let trust = worker
        .list_account_trust_worker_accounts(None, 10_000)
        .await
        .unwrap()
        .into_iter()
        .find(|a| a.tenant_id() == tenant && a.account_id() == account)
        .unwrap();
    assert!(
        worker
            .list_account_trust_fact_candidates(&trust, 100)
            .await
            .unwrap()
            .is_empty()
    );
    record_account_trust_facts(db, Some(10_000), false)
        .await
        .unwrap();
    assert_eq!(facts().await, 4, "a re-run records nothing new");

    let rendered = serde_json::to_string(&run).unwrap();
    for identifier in [
        tenant.clone(),
        account.to_string(),
        accepted.to_string(),
        quarantined.to_string(),
        owner.clone(),
    ] {
        assert!(!rendered.contains(&identifier), "summary is label-only");
    }
}

// ---------------------------------------------------------------------------
// V86: evaluations, the cross-tenant first-in-cluster boolean, explain, drill.
// ---------------------------------------------------------------------------

const DEDUP_V2: &str = "events.v2+simhash.v2";

/// A gate decision stamped with a dedup cluster and signal version.
async fn seed_clustered_decision(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    submission: Uuid,
    cluster: Option<Uuid>,
    decided_at: DateTime<Utc>,
) -> Uuid {
    let decision = seed_gate_decision(admin, tenant, submission, true, decided_at).await;
    admin
        .execute(
            "UPDATE trace_gate_decisions SET dedup_cluster_id=$3, dedup_signal_version=$4,
                credit_quality_micros=900000, credit_quality_calibration_version=3
              WHERE tenant_id=$1 AND decision_id=$2",
            &[&tenant, &decision, &cluster, &DEDUP_V2],
        )
        .await
        .unwrap();
    decision
}

/// An accepted submission with its credit event and one clustered decision.
async fn seed_qualified(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    owner: &str,
    cluster: Option<Uuid>,
    at_time: DateTime<Utc>,
) -> Uuid {
    let (submission, trace) = seed_submission(admin, tenant, owner, "accepted", at_time).await;
    seed_credit(
        admin, tenant, submission, trace, "accepted", owner, "system", at_time,
    )
    .await;
    seed_clustered_decision(admin, tenant, submission, cluster, at_time).await;
    submission
}

fn shadow_policy() -> trace_commons_server::account_trust_rule::GrowthPolicy {
    let json = serde_json::json!({
        "version": "growth-shadow-test-v1",
        "processing_cost_bound": 10,
        "bounded_allowance": 100,
        "period": {"mode": "fixed", "seconds": 604800},
        "growth_rule": "tiered-v1",
        "growth": {
            "window_seconds": 3650 * 86400,
            "weekly_cap": 10,
            "q_min_micros": null,
            "penalty_cooldown_seconds": 0,
            "evaluation_max_age_seconds": 86400,
            "allowance_ceiling": 300,
            "evaluator_versions": ["gate-v1"],
            "dedup_signal_versions": [DEDUP_V2],
            "tiers": [
                {"units": 0, "active_weeks": 0, "age_seconds": 0, "multiplier": 1},
                {"units": 2, "active_weeks": 1, "age_seconds": 0, "multiplier": 2}
            ]
        }
    });
    trace_commons_server::account_trust_rule::parse_growth_policy(
        &json.to_string(),
        &["growth-shadow-test-v1"],
    )
    .unwrap()
}

async fn find(worker: &PgBackend, tenant: &str, account: Uuid) -> TrustAccount {
    enumerate_all(worker)
        .await
        .into_iter()
        .find(|a| a.tenant_id() == tenant && a.account_id() == account)
        .expect("enumerated")
}

#[tokio::test]
async fn first_in_cluster_is_answered_across_tenants_as_a_boolean_only() {
    let Some(fx) = fixture().await else {
        return;
    };
    let admin = &fx.admin;
    let worker = login_with_role(
        &fx,
        "tc_account_trust_cluster_login",
        "trace_account_trust_worker",
    )
    .await;
    let shared = Uuid::new_v4();
    let tenant_a = anchored_tenant();
    let owner_a = principal();
    let account_a = seed_account(admin, &tenant_a, &[&owner_a]).await;
    let tenant_b = anchored_tenant();
    let owner_b = principal();
    let account_b = seed_account(admin, &tenant_b, &[&owner_b]).await;
    // B's copy reached the gate first; A's copy of the same trace is second.
    let first_b = seed_qualified(admin, &tenant_b, &owner_b, Some(shared), at(0)).await;
    let copy_a = seed_qualified(admin, &tenant_a, &owner_a, Some(shared), at(10)).await;
    let unique_a = seed_qualified(admin, &tenant_a, &owner_a, Some(Uuid::new_v4()), at(5)).await;
    let unassigned_a = seed_qualified(admin, &tenant_a, &owner_a, None, at(6)).await;

    let db: &dyn Database = worker.as_ref();
    trace_commons_server::account_trust_growth::record_account_trust_facts(db, None, false)
        .await
        .unwrap();
    let trust_a = find(&worker, &tenant_a, account_a).await;
    let trust_b = find(&worker, &tenant_b, account_b).await;
    let first = |facts: &[trace_commons_server::account_trust_rule::EvaluationFact],
                 submission: Uuid| {
        facts
            .iter()
            .find(|f| f.source_kind == "gate_evaluation" && f.submission_id == submission)
            .and_then(|f| f.first_in_cluster)
    };
    let inputs_a = worker
        .account_trust_evaluation_inputs(&trust_a)
        .await
        .unwrap();
    assert_eq!(
        first(&inputs_a, copy_a),
        Some(false),
        "a later copy is not first"
    );
    assert_eq!(first(&inputs_a, unique_a), Some(true));
    assert_eq!(
        first(&inputs_a, unassigned_a),
        Some(false),
        "no cluster yet earns nothing"
    );
    assert!(
        inputs_a
            .iter()
            .filter(|f| f.source_kind != "gate_evaluation")
            .all(|f| f.first_in_cluster.is_none())
    );
    let inputs_b = worker
        .account_trust_evaluation_inputs(&trust_b)
        .await
        .unwrap();
    assert_eq!(first(&inputs_b, first_b), Some(true));
    // Nothing of B's reaches A's inputs.
    assert!(inputs_a.iter().all(|f| f.submission_id != first_b));

    // The boolean is callable only by the facts guard, never by the worker.
    let raw = worker
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    assert!(
        raw.query_one(
            "SELECT trace_account_trust_first_in_cluster($1,$2)",
            &[&tenant_a, &copy_a]
        )
        .await
        .is_err()
    );
    // An account named under another tenant's context reads nothing.
    let mut client = worker
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_b],
    )
    .await
    .unwrap();
    let crossed = tx
        .query(
            "SELECT * FROM trace_account_trust_evaluation_inputs($1,$2)",
            &[&tenant_a, &account_a],
        )
        .await
        .unwrap();
    assert!(crossed.is_empty());
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn shadow_evaluations_are_stored_audited_and_reproduced() {
    use trace_commons_server::account_trust_growth::{
        EvaluationMode, account_trust_ref, account_trust_reproduction_drill,
        evaluate_account_trust, explain_account_trust, find_account_by_ref,
        record_account_trust_facts,
    };
    let Some(fx) = fixture().await else {
        return;
    };
    let admin = &fx.admin;
    let worker = login_with_role(
        &fx,
        "tc_account_trust_evaluator_login",
        "trace_account_trust_worker",
    )
    .await;
    let runtime = login_with_role(
        &fx,
        "tc_account_trust_eval_runtime_login",
        "trace_account_admission_runtime",
    )
    .await;
    let policy = shadow_policy();
    let tenant = anchored_tenant();
    let owner = principal();
    let account = seed_account(admin, &tenant, &[&owner]).await;
    let idle_tenant = anchored_tenant();
    let idle = seed_account(admin, &idle_tenant, &[&principal()]).await;
    let first = seed_qualified(admin, &tenant, &owner, Some(Uuid::new_v4()), at(0)).await;
    seed_qualified(admin, &tenant, &owner, Some(Uuid::new_v4()), at(100)).await;
    let db: &dyn Database = worker.as_ref();
    record_account_trust_facts(db, None, false).await.unwrap();

    let as_of = at(1_000);
    let summary = evaluate_account_trust(db, &policy, EvaluationMode::Shadow, as_of, None)
        .await
        .unwrap();
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.mode, "shadow");
    assert!(summary.tier_distribution.get(&1).copied().unwrap_or(0) >= 1);
    let rows = admin
        .query(
            "SELECT tier, previous_tier, mode, effective_allowance, units, facts_digest
               FROM trace_account_trust_evaluations WHERE tenant_id=$1 AND account_id=$2",
            &[&tenant, &account],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, i32>(0), 1);
    assert_eq!(rows[0].get::<_, Option<i32>>(1), None);
    assert_eq!(rows[0].get::<_, String>(2), "shadow");
    assert_eq!(rows[0].get::<_, i64>(3), 200);
    assert_eq!(rows[0].get::<_, i32>(4), 2);
    let idle_rows: i64 = admin
        .query_one(
            "SELECT count(*) FROM trace_account_trust_evaluations WHERE tenant_id=$1 AND account_id=$2",
            &[&idle_tenant, &idle],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(idle_rows, 0, "an account with no facts gets no row");

    async fn audit(admin: &deadpool_postgres::Object, tenant: &str) -> Vec<tokio_postgres::Row> {
        admin
            .query(
                "SELECT outcome, actor_ref, safe_metadata FROM trace_account_audit
                  WHERE tenant_id=$1 AND action='account_trust_tier_changed'
                  ORDER BY audit_sequence",
                &[&tenant],
            )
            .await
            .unwrap()
    }
    let changes = audit(admin, &tenant).await;
    assert_eq!(changes.len(), 1, "0 -> 1 is a tier change");
    assert_eq!(changes[0].get::<_, String>(0), "shadow");
    assert_eq!(
        changes[0].get::<_, String>(1),
        format!("account-actor:{account}")
    );
    let metadata: serde_json::Value = changes[0].get(2);
    assert_eq!(metadata["from_tier"], 0);
    assert_eq!(metadata["to_tier"], 1);
    assert_eq!(metadata["growth_policy_version"], "growth-shadow-test-v1");
    assert!(!metadata.to_string().contains(&first.to_string()));
    assert!(!metadata.to_string().contains(&owner));

    // Re-running at the same as_of changes nothing and audits nothing.
    evaluate_account_trust(db, &policy, EvaluationMode::Shadow, as_of, None)
        .await
        .unwrap();
    assert_eq!(audit(admin, &tenant).await.len(), 1);

    // Explain, by hash-only ref.
    let trust = find(&worker, &tenant, account).await;
    let account_ref = account_trust_ref(&trust);
    assert!(account_ref.starts_with("sha256:"));
    assert!(!account_ref.contains(&account.to_string()));
    let found = find_account_by_ref(db, &account_ref)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found, trust);
    let report = explain_account_trust(db, &policy, EvaluationMode::Shadow, &trust)
        .await
        .unwrap();
    assert!(report.reproduced);
    assert_eq!(report.stored.as_ref().unwrap().tier, 1);
    let rendered = serde_json::to_string(&report).unwrap();
    assert!(!rendered.contains(&first.to_string()));
    assert!(!rendered.contains(&account.to_string()));
    assert!(!rendered.contains(&tenant));
    assert!(
        account_trust_reproduction_drill(db, &policy, EvaluationMode::Shadow, None)
            .await
            .unwrap()
            .passed()
    );

    // A later dedup rederive that makes the first submission a copy of an
    // earlier trace elsewhere changes the recomputation: not reproduced.
    let other_tenant = anchored_tenant();
    let other_owner = principal();
    seed_account(admin, &other_tenant, &[&other_owner]).await;
    let earlier_cluster = Uuid::new_v4();
    seed_qualified(
        admin,
        &other_tenant,
        &other_owner,
        Some(earlier_cluster),
        at(-50),
    )
    .await;
    admin
        .execute(
            "UPDATE trace_gate_decisions SET dedup_cluster_id=$3 WHERE tenant_id=$1 AND submission_id=$2",
            &[&tenant, &first, &earlier_cluster],
        )
        .await
        .unwrap();
    let drifted = explain_account_trust(db, &policy, EvaluationMode::Shadow, &trust)
        .await
        .unwrap();
    assert!(!drifted.reproduced);
    assert_ne!(
        drifted.stored.as_ref().unwrap().facts_digest,
        drifted.recomputed.as_ref().unwrap().facts_digest
    );
    let drill = account_trust_reproduction_drill(db, &policy, EvaluationMode::Shadow, None)
        .await
        .unwrap();
    assert!(drill.not_reproduced >= 1);
    assert!(!drill.passed());
    // Re-evaluating records the drop (1 -> 0) and reproduces again.
    evaluate_account_trust(db, &policy, EvaluationMode::Shadow, at(1_001), None)
        .await
        .unwrap();
    let changes = audit(admin, &tenant).await;
    assert_eq!(changes.len(), 2);
    assert!(
        explain_account_trust(db, &policy, EvaluationMode::Shadow, &trust)
            .await
            .unwrap()
            .reproduced
    );

    // Shadow only, enforced in the database: the write function refuses
    // 'applied', and no role but its guard can insert.
    let mut client = worker
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let applied = tx
        .query_one(
            "SELECT trace_record_account_trust_evaluation($1,$2,$3,'growth-shadow-test-v1',
                'applied',now(),5,500,9,0,9,9,0,0,0,FALSE,$4)",
            &[
                &tenant,
                &Uuid::new_v4(),
                &account,
                &format!("sha256:{}", "0".repeat(64)),
            ],
        )
        .await;
    let error = applied.expect_err("an applied evaluation is refused");
    assert_eq!(
        error.as_db_error().map(|db| db.message()),
        Some("account_trust_evaluation_shadow_only")
    );
    tx.rollback().await.unwrap();
    let applied_rows: i64 = admin
        .query_one(
            "SELECT count(*) FROM trace_account_trust_evaluations WHERE tenant_id=$1 AND account_id=$2 AND mode='applied'",
            &[&tenant, &account],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(applied_rows, 0);
    for login in [&worker, &runtime] {
        let mut client = login
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .unwrap();
        let tx = client.transaction().await.unwrap();
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await
        .unwrap();
        let visible: i64 = tx
            .query_one(
                "SELECT count(*) FROM trace_account_trust_evaluations WHERE account_id=$1",
                &[&account],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            visible, 3,
            "SELECT under the caller's tenant: one row per run"
        );
        assert!(
            tx.execute(
                "INSERT INTO trace_account_trust_evaluations(tenant_id,evaluation_id,account_id,
                    growth_policy_version,mode,as_of,tier,effective_allowance,units,units_capped,
                    active_weeks,age_weeks,netted_withdrawn,netted_revoked,netted_quarantined,
                    penalty_active,facts_digest)
                 VALUES($1,$2,$3,'v','shadow',now(),9,900,9,0,9,9,0,0,0,FALSE,$4)",
                &[
                    &tenant,
                    &Uuid::new_v4(),
                    &account,
                    &format!("sha256:{}", "0".repeat(64)),
                ],
            )
            .await
            .is_err(),
            "no direct write"
        );
        tx.rollback().await.unwrap();
        let mut client = login
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .unwrap();
        let tx = client.transaction().await.unwrap();
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&other_tenant],
        )
        .await
        .unwrap();
        let crossed: i64 = tx
            .query_one(
                "SELECT count(*) FROM trace_account_trust_evaluations WHERE account_id=$1",
                &[&account],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(crossed, 0, "RLS isolates evaluations by tenant");
        tx.rollback().await.unwrap();
    }
}

/// V88 (M5): each reservation can name the tier and evaluation it relied on.
/// Nullable: NULL is "no growth rule applied", which is every reservation
/// while growth is shadow-only. The runtime's existing table-wide grants
/// cover the columns; nothing new is granted.
#[tokio::test]
async fn reservation_rows_carry_nullable_earned_tier_columns() {
    let Some(fx) = fixture().await else {
        return;
    };
    let admin = &fx.admin;
    let tenant = anchored_tenant();
    let account = seed_account(admin, &tenant, &[&principal()]).await;
    async fn insert_reservation(
        admin: &deadpool_postgres::Object,
        tenant: &str,
        account: Uuid,
        tier: Option<i32>,
        digest: Option<String>,
    ) -> Result<u64, tokio_postgres::Error> {
        {
            admin
                .execute(
                    "INSERT INTO trace_account_admission_submissions(tenant_id,submission_id,
                        account_id,body_hash,authority_kind,trust_version,policy_version,period_id,
                        status,lease_id,lease_expires_at,last_cost_bound,last_charged,earned_tier,
                        trust_evaluation_digest)
                     VALUES($1,$2,$3,$4,'account',1,'v1','v1:lifetime','reserved',$5,now(),1,TRUE,
                        $6,$7)",
                    &[
                        &tenant,
                        &Uuid::new_v4(),
                        &account,
                        &"c".repeat(64),
                        &Uuid::new_v4(),
                        &tier,
                        &digest,
                    ],
                )
                .await
        }
    }
    let insert = |tier: Option<i32>, digest: Option<String>| {
        insert_reservation(admin, &tenant, account, tier, digest)
    };
    assert!(insert(None, None).await.is_ok(), "shadow: nothing applied");
    assert!(
        insert(Some(2), Some(format!("sha256:{}", "d".repeat(64))))
            .await
            .is_ok()
    );
    assert!(insert(Some(-1), None).await.is_err());
    assert!(
        insert(Some(1), Some("not-a-digest".into())).await.is_err(),
        "hash-only"
    );
    let runtime_can_write: bool = admin
        .query_one(
            "SELECT has_column_privilege('trace_account_admission_runtime',
                'trace_account_admission_submissions','earned_tier','INSERT')
                AND has_column_privilege('trace_account_admission_runtime',
                'trace_account_admission_submissions','trust_evaluation_digest','UPDATE')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(runtime_can_write);
}

/// V137: upload claims retain the verified device's account ownership. A
/// server-accepted trace needs no monetary event to supply status evidence;
/// only an explicitly gate-required policy can qualify that evidence.
#[tokio::test]
async fn status_facts_use_verified_upload_ownership_and_invalidate_applied_inputs() {
    let Some(fx) = fixture().await else {
        return;
    };
    let admin = &fx.admin;
    let worker = login_with_role(
        &fx,
        "tc_upload_trust_status_worker",
        "trace_account_trust_worker",
    )
    .await;
    fx.admin_db
        .enable_external_account_trust_growth()
        .await
        .unwrap();
    let tenant = anchored_tenant();
    let device = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
    let parent = trace_commons_server::account_trust::device_principal_ref(&tenant, &device);
    let account = seed_account(admin, &tenant, &[&parent]).await;
    let foreign_account = seed_account(admin, &tenant, &[&principal()]).await;
    admin.execute("INSERT INTO device_keys(tenant_id,device_key_id,public_key,onboarding_origin) VALUES($1,$2,'test','near_ai')", &[&tenant,&device]).await.unwrap();
    let alias = format!(
        "principal_sha256:{}",
        Uuid::new_v4().simple().to_string().repeat(2)
    );
    admin.execute("INSERT INTO trace_device_upload_principals(tenant_id,upload_principal_ref,device_principal_ref,device_key_id) VALUES($1,$2,$3,$4)", &[&tenant,&alias,&parent,&device]).await.unwrap();
    let trust = find(&worker, &tenant, account).await;
    let foreign = find(&worker, &tenant, foreign_account).await;
    let (submission, trace) = seed_submission(
        admin,
        &tenant,
        &alias,
        "accepted",
        Utc::now() - chrono::Duration::seconds(1),
    )
    .await;
    let gate = seed_gate_decision(admin, &tenant, submission, true, Utc::now()).await;
    admin.execute("UPDATE trace_gate_decisions SET dedup_cluster_id=$3,dedup_signal_version='events.v2+simhash.v2' WHERE tenant_id=$1 AND decision_id=$2", &[&tenant,&gate,&Uuid::new_v4()]).await.unwrap();
    let source = TrustFactSource::AcceptedSubmissionStatus(submission);
    assert_eq!(
        TrustFactSource::from_storage(source.kind(), source.source_id()),
        Some(source)
    );
    assert_eq!(
        worker
            .record_account_trust_fact(&trust, TrustFactSource::AcceptedSubmission(submission))
            .await
            .unwrap(),
        None,
        "legacy acceptance still requires a credit event"
    );
    assert_eq!(
        worker
            .record_account_trust_fact(&foreign, source)
            .await
            .unwrap(),
        None
    );
    let candidates = worker
        .list_account_trust_fact_candidates(&trust, 100)
        .await
        .unwrap();
    assert!(candidates.contains(&source));
    assert!(candidates.contains(&TrustFactSource::GateEvaluation(gate)));
    for source in candidates {
        assert!(
            worker
                .record_account_trust_fact(&trust, source)
                .await
                .unwrap()
                .is_some()
        );
    }
    let generation = || async {
        admin.query_one("SELECT generation FROM trace_account_trust_frontiers WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account]).await.unwrap().get::<_,i64>(0)
    };
    let before = generation().await;
    assert_eq!(
        worker
            .record_account_trust_fact(&trust, source)
            .await
            .unwrap(),
        Some(TrustFactOutcome::Accepted)
    );
    assert_eq!(
        generation().await,
        before,
        "idempotent recording keeps the generation"
    );
    assert!(
        worker
            .list_account_trust_fact_candidates(&trust, 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        admin
            .query_one(
                "SELECT count(*) FROM trace_credit_ledger WHERE tenant_id=$1 AND submission_id=$2",
                &[&tenant, &submission]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    let policy_json =
        include_str!("../../../deploy/staging-gcp/client-upload-trust-policy.json.template")
            .replace("${TC_UPLOAD_TRUST_GATE_POLICY_VERSION}", "gate-v1")
            .replace(
                "${TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION}",
                "events.v2+simhash.v2",
            );
    let policy = trace_commons_server::account_trust_rule::parse_growth_policy(
        &policy_json,
        &["staging-earned-upload-v1"],
    )
    .unwrap();
    let inputs = worker
        .account_trust_evaluation_inputs(&trust)
        .await
        .unwrap();
    assert_eq!(
        trace_commons_server::account_trust_rule::evaluate(&policy, &inputs, Utc::now()).units,
        1
    );
    assert_eq!(
        trace_commons_server::account_trust_rule::evaluate(&shadow_policy(), &inputs, Utc::now())
            .units,
        0,
        "legacy policy ignores status-only acceptance"
    );
    let (applied, _) = worker
        .evaluate_and_record_applied_account_trust(&trust, &policy)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(applied.units, 1);
    // The authoritative gate columns reject SQL NULL, never treating it as pass.
    for column in ["novelty_passed", "perplexity_passed"] {
        let mut check = fx
            .admin_db
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .unwrap();
        let tx = check.transaction().await.unwrap();
        let error = tx.execute(&format!("UPDATE trace_gate_decisions SET {column}=NULL WHERE tenant_id=$1 AND decision_id=$2"), &[&tenant,&gate]).await.unwrap_err();
        assert_eq!(error.code().map(|code| code.code()), Some("23502"));
        tx.rollback().await.unwrap();
    }
    admin.execute("UPDATE trace_gate_decisions SET novelty_passed=FALSE WHERE tenant_id=$1 AND decision_id=$2", &[&tenant,&gate]).await.unwrap();
    assert!(
        generation().await > before,
        "gate pass correction invalidates applied inputs"
    );
    let inputs = worker
        .account_trust_evaluation_inputs(&trust)
        .await
        .unwrap();
    assert_eq!(
        trace_commons_server::account_trust_rule::evaluate(&policy, &inputs, Utc::now()).units,
        0
    );
    let (failed_evaluation, _) = worker
        .evaluate_and_record_applied_account_trust(&trust, &policy)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        failed_evaluation.units, 0,
        "a fresh re-evaluation cannot count a corrected failed gate"
    );
    admin.execute("UPDATE trace_gate_decisions SET novelty_passed=TRUE WHERE tenant_id=$1 AND decision_id=$2", &[&tenant,&gate]).await.unwrap();
    let before_perplexity = generation().await;
    admin.execute("UPDATE trace_gate_decisions SET perplexity_passed=FALSE WHERE tenant_id=$1 AND decision_id=$2", &[&tenant,&gate]).await.unwrap();
    assert!(generation().await > before_perplexity);
    let (failed_evaluation, _) = worker
        .evaluate_and_record_applied_account_trust(&trust, &policy)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed_evaluation.units, 0);
    admin.execute("UPDATE trace_gate_decisions SET perplexity_passed=TRUE WHERE tenant_id=$1 AND decision_id=$2", &[&tenant,&gate]).await.unwrap();
    let before_revoke = generation().await;
    admin.execute("UPDATE device_keys SET revoked_at=clock_timestamp() WHERE tenant_id=$1 AND device_key_id=$2", &[&tenant,&device]).await.unwrap();
    assert!(generation().await > before_revoke);
    let inputs = worker
        .account_trust_evaluation_inputs(&trust)
        .await
        .unwrap();
    assert_eq!(
        trace_commons_server::account_trust_rule::evaluate(&policy, &inputs, Utc::now()).units,
        0
    );
    admin
        .execute(
            "UPDATE device_keys SET revoked_at=NULL WHERE tenant_id=$1 AND device_key_id=$2",
            &[&tenant, &device],
        )
        .await
        .unwrap();
    let before_alias = generation().await;
    admin.execute("DELETE FROM trace_device_upload_principals WHERE tenant_id=$1 AND upload_principal_ref=$2", &[&tenant,&alias]).await.unwrap();
    assert!(
        generation().await > before_alias,
        "removing verified alias ownership invalidates its input snapshot"
    );
    let inputs = worker
        .account_trust_evaluation_inputs(&trust)
        .await
        .unwrap();
    assert_eq!(
        trace_commons_server::account_trust_rule::evaluate(&policy, &inputs, Utc::now()).units,
        0
    );
    admin.execute("INSERT INTO trace_device_upload_principals(tenant_id,upload_principal_ref,device_principal_ref,device_key_id) VALUES($1,$2,$3,$4)", &[&tenant,&alias,&parent,&device]).await.unwrap();
    admin.execute("UPDATE trace_accounts SET closed_at=clock_timestamp() WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account]).await.unwrap();
    assert_eq!(
        worker
            .record_account_trust_fact(&trust, source)
            .await
            .unwrap(),
        None,
        "closed accounts cannot refresh positive facts"
    );
    admin
        .execute(
            "UPDATE trace_accounts SET closed_at=NULL WHERE tenant_id=$1 AND account_id=$2",
            &[&tenant, &account],
        )
        .await
        .unwrap();
    // A different verified device with no membership cannot supply evidence.
    let wrong_device = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
    let wrong_parent =
        trace_commons_server::account_trust::device_principal_ref(&tenant, &wrong_device);
    let wrong_alias = format!(
        "principal_sha256:{}",
        Uuid::new_v4().simple().to_string().repeat(2)
    );
    admin
        .execute(
            "INSERT INTO device_keys(tenant_id,device_key_id,public_key,onboarding_origin) VALUES($1,$2,'test','near_ai')",
            &[&tenant, &wrong_device],
        )
        .await
        .unwrap();
    admin.execute("INSERT INTO trace_device_upload_principals(tenant_id,upload_principal_ref,device_principal_ref,device_key_id) VALUES($1,$2,$3,$4)", &[&tenant,&wrong_alias,&wrong_parent,&wrong_device]).await.unwrap();
    let (unowned, _) = seed_submission(admin, &tenant, &wrong_alias, "accepted", Utc::now()).await;
    assert_eq!(
        worker
            .record_account_trust_fact(&trust, TrustFactSource::AcceptedSubmissionStatus(unowned))
            .await
            .unwrap(),
        None
    );
    // The function refuses a tenant argument different from the caller's GUC.
    let other_tenant = anchored_tenant();
    let other_account = seed_account(admin, &other_tenant, &[&principal()]).await;
    let mut raw = worker
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let tx = raw.transaction().await.unwrap();
    tx.query_one(
        "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
        &[&tenant],
    )
    .await
    .unwrap();
    assert!(
        tx.query_one(
            "SELECT trace_record_account_trust_fact($1,$2,'accepted_submission_status',$3)",
            &[&other_tenant, &other_account, &submission]
        )
        .await
        .unwrap()
        .get::<_, Option<String>>(0)
        .is_none()
    );
    for query in [
        "SELECT trace_record_account_trust_fact(NULL,$2,'accepted_submission_status',$3)",
        "SELECT trace_record_account_trust_fact($1,NULL,'accepted_submission_status',$3)",
        "SELECT trace_record_account_trust_fact($1,$2,'accepted_submission_status',NULL)",
    ] {
        // Keep parameter types constrained even for the deliberately unused slot.
        let query = format!("SELECT ({query}) FROM (SELECT $1::text,$2::uuid,$3::uuid) unused");
        assert!(
            tx.query_one(&query, &[&tenant, &account, &submission])
                .await
                .unwrap()
                .get::<_, Option<String>>(0)
                .is_none()
        );
    }
    tx.commit().await.unwrap();
    // A recorded source remains eligible for netting after its parent unlinks.
    admin.execute("UPDATE trace_account_principals SET unlinked_at=clock_timestamp() WHERE tenant_id=$1 AND principal_ref=$2", &[&tenant,&parent]).await.unwrap();
    admin.execute("UPDATE trace_submissions SET status='quarantined',reviewed_at=clock_timestamp() WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&submission]).await.unwrap();
    assert!(
        worker
            .list_account_trust_fact_candidates(&trust, 100)
            .await
            .unwrap()
            .contains(&TrustFactSource::SubmissionQuarantined(submission))
    );
    assert_eq!(
        worker
            .record_account_trust_fact(&trust, TrustFactSource::SubmissionQuarantined(submission))
            .await
            .unwrap(),
        Some(TrustFactOutcome::Quarantined)
    );
    let inputs = worker
        .account_trust_evaluation_inputs(&trust)
        .await
        .unwrap();
    let result = trace_commons_server::account_trust_rule::evaluate(&policy, &inputs, Utc::now());
    assert_eq!((result.units, result.netted_quarantined), (0, 1));
    // A passed gate must not turn status acceptance into a self-review bypass.
    // The former source is netted out; only an independently reviewed new
    // source may add a unit, even when neither has any monetary credit event.
    admin.execute("UPDATE trace_account_principals SET unlinked_at=NULL WHERE tenant_id=$1 AND principal_ref=$2", &[&tenant,&parent]).await.unwrap();
    let independent_reviewer = principal();
    for (reviewer, human_credit, expected_outcome, expected_units) in [
        (
            Some(alias.as_str()),
            false,
            TrustFactOutcome::AcceptedSelfReviewed,
            0,
        ),
        (None, true, TrustFactOutcome::AcceptedApproverUnknown, 0),
        (
            Some(independent_reviewer.as_str()),
            false,
            TrustFactOutcome::Accepted,
            1,
        ),
    ] {
        let (reviewed, reviewed_trace) =
            seed_submission(admin, &tenant, &alias, "accepted", Utc::now()).await;
        if let Some(reviewer) = reviewer {
            seed_review_approval(admin, &tenant, reviewed, reviewer).await;
        }
        if human_credit {
            seed_credit(
                admin,
                &tenant,
                reviewed,
                reviewed_trace,
                "accepted",
                &independent_reviewer,
                "reviewer",
                Utc::now(),
            )
            .await;
        }
        let reviewed_gate = seed_gate_decision(admin, &tenant, reviewed, true, Utc::now()).await;
        admin.execute("UPDATE trace_gate_decisions SET dedup_cluster_id=$3,dedup_signal_version='events.v2+simhash.v2' WHERE tenant_id=$1 AND decision_id=$2", &[&tenant,&reviewed_gate,&Uuid::new_v4()]).await.unwrap();
        assert_eq!(
            worker
                .record_account_trust_fact(
                    &trust,
                    TrustFactSource::AcceptedSubmissionStatus(reviewed)
                )
                .await
                .unwrap(),
            Some(expected_outcome)
        );
        assert_eq!(
            worker
                .record_account_trust_fact(&trust, TrustFactSource::GateEvaluation(reviewed_gate))
                .await
                .unwrap(),
            Some(TrustFactOutcome::EvaluatedPassed)
        );
        let inputs = worker
            .account_trust_evaluation_inputs(&trust)
            .await
            .unwrap();
        assert_eq!(
            trace_commons_server::account_trust_rule::evaluate(&policy, &inputs, Utc::now()).units,
            expected_units
        );
        if !human_credit {
            assert_eq!(admin.query_one("SELECT count(*) FROM trace_credit_ledger WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&reviewed]).await.unwrap().get::<_,i64>(0),0,
                "review classification never adds a monetary prerequisite");
        }
    }
    // Legacy reviewer classification intentionally stays unchanged. An
    // alias-owned reviewer may therefore have an old accepted credit fact;
    // under the new policy its status exclusion still governs that same UUID.
    let self_reviewed = admin
        .query_one(
            "SELECT f.submission_id,s.trace_id FROM trace_account_trust_facts f
           JOIN trace_submissions s ON s.tenant_id=f.tenant_id AND s.submission_id=f.submission_id
          WHERE f.tenant_id=$1 AND f.account_id=$2 AND f.source_kind='accepted_submission_status'
            AND f.outcome='accepted_self_reviewed'",
            &[&tenant, &account],
        )
        .await
        .unwrap();
    let self_submission: Uuid = self_reviewed.get(0);
    let self_trace: Uuid = self_reviewed.get(1);
    seed_credit(
        admin,
        &tenant,
        self_submission,
        self_trace,
        "accepted",
        &alias,
        "system",
        Utc::now(),
    )
    .await;
    assert_eq!(
        worker
            .record_account_trust_fact(&trust, TrustFactSource::AcceptedSubmission(self_submission))
            .await
            .unwrap(),
        Some(TrustFactOutcome::Accepted)
    );
    let inputs = worker
        .account_trust_evaluation_inputs(&trust)
        .await
        .unwrap();
    assert_eq!(
        inputs
            .iter()
            .filter(|fact| fact.submission_id == self_submission
                && matches!(
                    fact.source_kind.as_str(),
                    "accepted_submission" | "accepted_submission_status"
                ))
            .count(),
        2
    );
    assert_eq!(
        trace_commons_server::account_trust_rule::evaluate(&policy, &inputs, Utc::now()).units,
        1,
        "only the independently reviewed UUID qualifies; old credit cannot override status self-review exclusion"
    );
    assert_eq!(
        trace_commons_server::account_trust_rule::evaluate(&shadow_policy(), &inputs, Utc::now())
            .units,
        1,
        "legacy default-false policies retain credit-source semantics"
    );
    let _ = trace;
}

#[tokio::test]
async fn upload_trust_readiness_requires_private_ownership_and_all_invalidation_triggers() {
    let Some(fx) = fixture().await else {
        return;
    };
    let runtime = login_with_role(
        &fx,
        "tc_upload_trust_readiness_runtime",
        "trace_account_admission_runtime",
    )
    .await;
    runtime
        .enable_external_account_trust_growth()
        .await
        .unwrap();
    assert!(
        runtime
            .external_account_trust_runtime_ready()
            .await
            .unwrap()
    );
    for (table, trigger) in [
        (
            "trace_device_upload_principals",
            "account_trust_upload_alias_frontier",
        ),
        ("device_keys", "account_trust_upload_device_frontier"),
        (
            "trace_submissions",
            "account_trust_upload_submission_frontier",
        ),
    ] {
        fx.admin
            .batch_execute(&format!("ALTER TABLE {table} DISABLE TRIGGER {trigger}"))
            .await
            .unwrap();
        assert!(
            !runtime
                .external_account_trust_runtime_ready()
                .await
                .unwrap(),
            "{trigger}"
        );
        fx.admin
            .batch_execute(&format!("ALTER TABLE {table} ENABLE TRIGGER {trigger}"))
            .await
            .unwrap();
        assert!(
            runtime
                .external_account_trust_runtime_ready()
                .await
                .unwrap()
        );
    }
    let helper = "trace_account_trust_live_owned_principal(TEXT,UUID,TEXT)";
    let raw = runtime
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    assert_eq!(
        raw.query_one(
            "SELECT trace_account_trust_live_owned_principal('tenant',NULL,NULL)",
            &[]
        )
        .await
        .unwrap_err()
        .code()
        .map(|code| code.code()),
        Some("42501"),
        "the runtime cannot call the private ownership helper directly"
    );
    fx.admin
        .batch_execute(&format!("GRANT EXECUTE ON FUNCTION {helper} TO PUBLIC"))
        .await
        .unwrap();
    assert!(
        !runtime
            .external_account_trust_runtime_ready()
            .await
            .unwrap(),
        "a public ownership helper fails readiness"
    );
    fx.admin
        .batch_execute(&format!("REVOKE EXECUTE ON FUNCTION {helper} FROM PUBLIC"))
        .await
        .unwrap();
    assert!(
        runtime
            .external_account_trust_runtime_ready()
            .await
            .unwrap()
    );
    fx.admin
        .batch_execute(&format!("ALTER FUNCTION {helper} SET search_path=public"))
        .await
        .unwrap();
    assert!(
        !runtime
            .external_account_trust_runtime_ready()
            .await
            .unwrap(),
        "an unsafe definer search path fails readiness"
    );
    fx.admin
        .batch_execute(&format!(
            "ALTER FUNCTION {helper} SET search_path=pg_catalog"
        ))
        .await
        .unwrap();
    assert!(
        runtime
            .external_account_trust_runtime_ready()
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn retention_keeps_verified_trust_without_promoting_unaccepted_or_failed_sources() {
    let Some(fx) = fixture().await else {
        return;
    };
    let admin = &fx.admin;
    let worker = login_with_role(
        &fx,
        "tc_upload_trust_retention_worker",
        "trace_account_trust_worker",
    )
    .await;
    fx.admin_db
        .enable_external_account_trust_growth()
        .await
        .unwrap();
    let tenant = anchored_tenant();
    let owner = principal();
    let account = seed_account(admin, &tenant, &[&owner]).await;
    let trust = find(&worker, &tenant, account).await;
    let submission = seed_qualified(
        admin,
        &tenant,
        &owner,
        Some(Uuid::new_v4()),
        Utc::now() - chrono::Duration::seconds(1),
    )
    .await;
    let gate: Uuid = admin
        .query_one(
            "SELECT decision_id FROM trace_gate_decisions WHERE tenant_id=$1 AND submission_id=$2",
            &[&tenant, &submission],
        )
        .await
        .unwrap()
        .get(0);
    for source in worker
        .list_account_trust_fact_candidates(&trust, 100)
        .await
        .unwrap()
    {
        assert!(
            worker
                .record_account_trust_fact(&trust, source)
                .await
                .unwrap()
                .is_some()
        );
    }
    let strict_json =
        include_str!("../../../deploy/staging-gcp/client-upload-trust-policy.json.template")
            .replace("${TC_UPLOAD_TRUST_GATE_POLICY_VERSION}", "gate-v1")
            .replace(
                "${TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION}",
                "events.v2+simhash.v2",
            );
    let strict = trace_commons_server::account_trust_rule::parse_growth_policy(
        &strict_json,
        &["staging-earned-upload-v1"],
    )
    .unwrap();
    let legacy = shadow_policy();
    let units = || async {
        let inputs = worker
            .account_trust_evaluation_inputs(&trust)
            .await
            .unwrap();
        (
            trace_commons_server::account_trust_rule::evaluate(&strict, &inputs, Utc::now()).units,
            trace_commons_server::account_trust_rule::evaluate(&legacy, &inputs, Utc::now()).units,
        )
    };
    assert_eq!(units().await, (1, 1));
    // A gate recorded as failed must never be promoted merely because its
    // metadata is later corrected to passed, even on an accepted source.
    let (failed, failed_trace) =
        seed_submission(admin, &tenant, &owner, "accepted", Utc::now()).await;
    seed_credit(
        admin,
        &tenant,
        failed,
        failed_trace,
        "accepted",
        &owner,
        "system",
        Utc::now(),
    )
    .await;
    let failed_gate = seed_gate_decision(admin, &tenant, failed, false, Utc::now()).await;
    admin.execute("UPDATE trace_gate_decisions SET dedup_cluster_id=$3,dedup_signal_version=$4 WHERE tenant_id=$1 AND decision_id=$2", &[&tenant,&failed_gate,&Uuid::new_v4(),&DEDUP_V2]).await.unwrap();
    for source in worker
        .list_account_trust_fact_candidates(&trust, 100)
        .await
        .unwrap()
    {
        worker
            .record_account_trust_fact(&trust, source)
            .await
            .unwrap();
    }
    admin.execute("UPDATE trace_gate_decisions SET perplexity_passed=TRUE WHERE tenant_id=$1 AND decision_id=$2", &[&tenant,&failed_gate]).await.unwrap();
    assert_eq!(
        units().await,
        (1, 1),
        "an originally failed gate is not promoted"
    );
    // A row that was never accepted is not made into acceptance by retention.
    let (unaccepted, _) = seed_submission(admin, &tenant, &owner, "received", Utc::now()).await;
    let unaccepted_gate =
        seed_clustered_decision(admin, &tenant, unaccepted, Some(Uuid::new_v4()), Utc::now()).await;
    assert_eq!(
        worker
            .record_account_trust_fact(&trust, TrustFactSource::GateEvaluation(unaccepted_gate))
            .await
            .unwrap(),
        Some(TrustFactOutcome::EvaluatedNotAccepted)
    );
    admin.execute("UPDATE trace_submissions SET status='purged',purged_at=clock_timestamp() WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&unaccepted]).await.unwrap();
    assert_eq!(
        worker
            .record_account_trust_fact(
                &trust,
                TrustFactSource::AcceptedSubmissionStatus(unaccepted)
            )
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        worker
            .record_account_trust_fact(&trust, TrustFactSource::AcceptedSubmission(unaccepted))
            .await
            .unwrap(),
        None
    );
    assert_eq!(units().await, (1, 1));
    // Even a formerly accepted row cannot mint a new status fact after it
    // expired, when its acceptance was never verified into the trust ledger.
    let (unrecorded, _) = seed_submission(admin, &tenant, &owner, "accepted", Utc::now()).await;
    admin.execute("UPDATE trace_submissions SET status='expired',expires_at=clock_timestamp()-interval '1 second' WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&unrecorded]).await.unwrap();
    assert_eq!(
        worker
            .record_account_trust_fact(
                &trust,
                TrustFactSource::AcceptedSubmissionStatus(unrecorded)
            )
            .await
            .unwrap(),
        None
    );
    assert!(
        !worker
            .list_account_trust_fact_candidates(&trust, 100)
            .await
            .unwrap()
            .contains(&TrustFactSource::AcceptedSubmissionStatus(unrecorded))
    );
    for (status, purged, expired) in [
        ("accepted", false, true),
        ("expired", false, true),
        ("accepted", true, false),
        ("purged", true, false),
    ] {
        admin.execute("UPDATE trace_submissions SET status=$3,purged_at=CASE WHEN $4 THEN clock_timestamp() END,expires_at=CASE WHEN $5 THEN clock_timestamp()-interval '1 second' END WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&submission,&status,&purged,&expired]).await.unwrap();
        assert_eq!(
            units().await,
            (1, 1),
            "retention-only state {status}/{purged}/{expired} keeps both policies' qualified contribution"
        );
    }
    // The new live-ownership requirement cannot fall back to the legacy
    // credit fact of this same UUID when the source principal unlinks.
    admin.execute("UPDATE trace_account_principals SET unlinked_at=clock_timestamp() WHERE tenant_id=$1 AND principal_ref=$2", &[&tenant,&owner]).await.unwrap();
    assert_eq!(
        units().await,
        (0, 1),
        "strict policy excludes inactive status ownership while legacy semantics remain unchanged"
    );
    admin.execute("UPDATE trace_account_principals SET unlinked_at=NULL WHERE tenant_id=$1 AND principal_ref=$2", &[&tenant,&owner]).await.unwrap();
    assert_eq!(units().await, (1, 1));
    admin.execute("UPDATE trace_gate_decisions SET novelty_passed=FALSE WHERE tenant_id=$1 AND decision_id=$2", &[&tenant,&gate]).await.unwrap();
    assert_eq!(
        units().await,
        (0, 0),
        "current failed novelty wins over retained accepted evidence"
    );
    admin.execute("UPDATE trace_gate_decisions SET novelty_passed=TRUE WHERE tenant_id=$1 AND decision_id=$2", &[&tenant,&gate]).await.unwrap();
    assert_eq!(units().await, (1, 1));
    admin.execute("UPDATE trace_submissions SET status='quarantined',reviewed_at=clock_timestamp() WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&submission]).await.unwrap();
    assert_eq!(units().await, (0, 0));
    assert_eq!(
        worker
            .record_account_trust_fact(&trust, TrustFactSource::SubmissionQuarantined(submission))
            .await
            .unwrap(),
        Some(TrustFactOutcome::Quarantined)
    );
    admin
        .execute(
            "UPDATE trace_submissions SET status='purged' WHERE tenant_id=$1 AND submission_id=$2",
            &[&tenant, &submission],
        )
        .await
        .unwrap();
    assert_eq!(
        units().await,
        (0, 0),
        "retention cannot undo an explicit adverse trust fact"
    );
}

/// Two participating connections, with PostgreSQL's lock graph as the
/// synchronization evidence: ownership mutation must own the account frontier
/// even when there was no fact from which the old trigger could discover it.
#[tokio::test]
async fn ownership_mutation_serializes_first_facts_and_invalidates_prior_evaluations() {
    let Some(fx) = fixture().await else {
        return;
    };
    let admin = &fx.admin;
    let worker = login_with_role(
        &fx,
        "tc_upload_trust_race_worker",
        "trace_account_trust_worker",
    )
    .await;
    fx.admin_db
        .enable_external_account_trust_growth()
        .await
        .unwrap();
    let json = include_str!("../../../deploy/staging-gcp/client-upload-trust-policy.json.template")
        .replace("${TC_UPLOAD_TRUST_GATE_POLICY_VERSION}", "gate-v1")
        .replace(
            "${TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION}",
            "events.v2+simhash.v2",
        );
    let strict = trace_commons_server::account_trust_rule::parse_growth_policy(
        &json,
        &["staging-earned-upload-v1"],
    )
    .unwrap();
    for revoke_device in [false, true] {
        let tenant = anchored_tenant();
        let device = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
        let parent = trace_commons_server::account_trust::device_principal_ref(&tenant, &device);
        let alias = format!(
            "principal_sha256:{}",
            Uuid::new_v4().simple().to_string().repeat(2)
        );
        let account = seed_account(admin, &tenant, &[&parent]).await;
        admin.execute("INSERT INTO device_keys(tenant_id,device_key_id,public_key,onboarding_origin) VALUES($1,$2,'test','near_ai')", &[&tenant,&device]).await.unwrap();
        admin.execute("INSERT INTO trace_device_upload_principals(tenant_id,upload_principal_ref,device_principal_ref,device_key_id) VALUES($1,$2,$3,$4)", &[&tenant,&alias,&parent,&device]).await.unwrap();
        for _ in 0..5 {
            seed_qualified(
                admin,
                &tenant,
                &alias,
                Some(Uuid::new_v4()),
                Utc::now() - chrono::Duration::seconds(1),
            )
            .await;
        }
        let trust = find(&worker, &tenant, account).await;
        let sources = worker
            .list_account_trust_fact_candidates(&trust, 100)
            .await
            .unwrap();
        assert_eq!(sources.len(), 15);
        assert_eq!(admin.query_one("SELECT count(*) FROM trace_account_trust_facts WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account]).await.unwrap().get::<_,i64>(0),0);
        let mut a = fx
            .admin_db
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .unwrap();
        let a_pid: i32 = a
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .unwrap()
            .get(0);
        let mut b = worker
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .unwrap();
        let b_pid: i32 = b
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .unwrap()
            .get(0);
        let mutation = a.transaction().await.unwrap();
        if revoke_device {
            mutation.execute("UPDATE device_keys SET revoked_at=clock_timestamp() WHERE tenant_id=$1 AND device_key_id=$2", &[&tenant,&device]).await.unwrap();
        } else {
            mutation.execute("DELETE FROM trace_device_upload_principals WHERE tenant_id=$1 AND upload_principal_ref=$2", &[&tenant,&alias]).await.unwrap();
        }
        let b_tenant = tenant.clone();
        let recording = tokio::spawn(async move {
            let tx = b.transaction().await.unwrap();
            tx.query_one(
                "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
                &[&b_tenant],
            )
            .await
            .unwrap();
            for source in sources {
                tx.query_one(
                    "SELECT trace_record_account_trust_fact($1,$2,$3,$4)",
                    &[&b_tenant, &account, &source.kind(), &source.source_id()],
                )
                .await
                .unwrap();
            }
            tx.commit().await.unwrap();
        });
        let blocked = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let observed: bool = mutation
                    .query_one(
                        "SELECT $1::integer=ANY(pg_blocking_pids($2::integer))",
                        &[&a_pid, &b_pid],
                    )
                    .await
                    .unwrap()
                    .get(0);
                if observed {
                    break true;
                }
                if recording.is_finished() {
                    break false;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or(false);
        mutation.commit().await.unwrap();
        recording.await.unwrap();
        assert!(
            blocked,
            "first fact recording must block on the ownership transaction's account frontier (device={revoke_device})"
        );
        let (evaluation, _) = worker
            .evaluate_and_record_applied_account_trust(&trust, &strict)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (evaluation.units, evaluation.tier),
            (0, 0),
            "stale positive fact cannot publish uplift after ownership commits"
        );
        // Reverse order: establish a real uplift, then remove ownership.
        if revoke_device {
            admin.execute("UPDATE device_keys SET revoked_at=NULL WHERE tenant_id=$1 AND device_key_id=$2", &[&tenant,&device]).await.unwrap();
        } else {
            admin.execute("INSERT INTO trace_device_upload_principals(tenant_id,upload_principal_ref,device_principal_ref,device_key_id) VALUES($1,$2,$3,$4)", &[&tenant,&alias,&parent,&device]).await.unwrap();
        }
        for source in worker
            .list_account_trust_fact_candidates(&trust, 100)
            .await
            .unwrap()
        {
            worker
                .record_account_trust_fact(&trust, source)
                .await
                .unwrap();
        }
        let (raised, _) = worker
            .evaluate_and_record_applied_account_trust(&trust, &strict)
            .await
            .unwrap()
            .unwrap();
        assert_eq!((raised.units, raised.tier), (5, 1));
        let prior_generation:i64=admin.query_one("SELECT generation FROM trace_account_trust_frontiers WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account]).await.unwrap().get(0);
        if revoke_device {
            admin.execute("UPDATE device_keys SET revoked_at=clock_timestamp() WHERE tenant_id=$1 AND device_key_id=$2", &[&tenant,&device]).await.unwrap();
        } else {
            admin.execute("DELETE FROM trace_device_upload_principals WHERE tenant_id=$1 AND upload_principal_ref=$2", &[&tenant,&alias]).await.unwrap();
        }
        assert!(admin.query_one("SELECT generation FROM trace_account_trust_frontiers WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account]).await.unwrap().get::<_,i64>(0)>prior_generation);
        let inputs = worker
            .account_trust_evaluation_inputs(&trust)
            .await
            .unwrap();
        assert_eq!(
            trace_commons_server::account_trust_rule::evaluate(&strict, &inputs, Utc::now()).units,
            0
        );
        assert_eq!(
            trace_commons_server::account_trust_rule::evaluate(
                &shadow_policy(),
                &inputs,
                Utc::now()
            )
            .units,
            5,
            "legacy policy keeps its historical credited contributions"
        );
    }
}

#[tokio::test]
async fn legacy_only_retained_sources_get_strict_lifecycle_and_reviewer_exclusions() {
    let Some(fx) = fixture().await else {
        return;
    };
    let admin = &fx.admin;
    let worker = login_with_role(
        &fx,
        "tc_upload_trust_legacy_only_worker",
        "trace_account_trust_worker",
    )
    .await;
    fx.admin_db
        .enable_external_account_trust_growth()
        .await
        .unwrap();
    let json = include_str!("../../../deploy/staging-gcp/client-upload-trust-policy.json.template")
        .replace("${TC_UPLOAD_TRUST_GATE_POLICY_VERSION}", "gate-v1")
        .replace(
            "${TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION}",
            "events.v2+simhash.v2",
        );
    let strict = trace_commons_server::account_trust_rule::parse_growth_policy(
        &json,
        &["staging-earned-upload-v1"],
    )
    .unwrap();
    for exclusion in ["ownership", "alias_self_review", "unknown_approver"] {
        let tenant = anchored_tenant();
        let device = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
        let parent = trace_commons_server::account_trust::device_principal_ref(&tenant, &device);
        let alias = format!(
            "principal_sha256:{}",
            Uuid::new_v4().simple().to_string().repeat(2)
        );
        let account = seed_account(admin, &tenant, &[&parent]).await;
        admin.execute("INSERT INTO device_keys(tenant_id,device_key_id,public_key,onboarding_origin) VALUES($1,$2,'test','near_ai')", &[&tenant,&device]).await.unwrap();
        admin.execute("INSERT INTO trace_device_upload_principals(tenant_id,upload_principal_ref,device_principal_ref,device_key_id) VALUES($1,$2,$3,$4)", &[&tenant,&alias,&parent,&device]).await.unwrap();
        let submission = seed_qualified(
            admin,
            &tenant,
            &alias,
            Some(Uuid::new_v4()),
            Utc::now() - chrono::Duration::seconds(1),
        )
        .await;
        let gate:Uuid=admin.query_one("SELECT decision_id FROM trace_gate_decisions WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&submission]).await.unwrap().get(0);
        let trust = find(&worker, &tenant, account).await;
        assert_eq!(
            worker
                .record_account_trust_fact(&trust, TrustFactSource::AcceptedSubmission(submission))
                .await
                .unwrap(),
            Some(TrustFactOutcome::Accepted)
        );
        assert_eq!(
            worker
                .record_account_trust_fact(&trust, TrustFactSource::GateEvaluation(gate))
                .await
                .unwrap(),
            Some(TrustFactOutcome::EvaluatedPassed)
        );
        if exclusion == "alias_self_review" {
            // Partial recorder state: the legacy accepted fact already exists;
            // current verified audit evidence identifies its own reviewer.
            seed_review_approval(admin, &tenant, submission, &alias).await;
        } else if exclusion == "unknown_approver" {
            let trace:Uuid=admin.query_one("SELECT trace_id FROM trace_submissions WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&submission]).await.unwrap().get(0);
            seed_credit(
                admin,
                &tenant,
                submission,
                trace,
                "accepted",
                &principal(),
                "reviewer",
                Utc::now(),
            )
            .await;
        }
        admin.execute("UPDATE trace_submissions SET status='purged',purged_at=clock_timestamp() WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&submission]).await.unwrap();
        assert_eq!(
            worker
                .record_account_trust_fact(
                    &trust,
                    TrustFactSource::AcceptedSubmissionStatus(submission)
                )
                .await
                .unwrap(),
            None
        );
        let units = || async {
            let inputs = worker
                .account_trust_evaluation_inputs(&trust)
                .await
                .unwrap();
            (
                trace_commons_server::account_trust_rule::evaluate(&strict, &inputs, Utc::now())
                    .units,
                trace_commons_server::account_trust_rule::evaluate(
                    &shadow_policy(),
                    &inputs,
                    Utc::now(),
                )
                .units,
            )
        };
        assert_eq!(
            units().await,
            (u32::from(exclusion == "ownership"), 1),
            "{exclusion}"
        );
        assert_eq!(admin.query_one("SELECT count(*) FROM trace_account_trust_facts WHERE tenant_id=$1 AND account_id=$2 AND source_kind='accepted_submission_status'", &[&tenant,&account]).await.unwrap().get::<_,i64>(0),0,
            "read-only synthetic status must never backfill historical facts");
        admin.execute("UPDATE trace_account_principals SET unlinked_at=clock_timestamp() WHERE tenant_id=$1 AND principal_ref=$2", &[&tenant,&parent]).await.unwrap();
        assert_eq!(units().await, (0, 1));
        admin.execute("UPDATE trace_account_principals SET unlinked_at=NULL WHERE tenant_id=$1 AND principal_ref=$2", &[&tenant,&parent]).await.unwrap();
        admin.execute("UPDATE device_keys SET revoked_at=clock_timestamp() WHERE tenant_id=$1 AND device_key_id=$2", &[&tenant,&device]).await.unwrap();
        assert_eq!(units().await, (0, 1));
    }
}
