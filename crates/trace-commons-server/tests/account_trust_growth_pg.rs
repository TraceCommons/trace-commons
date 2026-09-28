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
    assert_eq!(recorded, 9, "each source recorded exactly once");
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
    assert_eq!(facts().await, 3);

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
    assert_eq!(facts().await, 3, "a re-run records nothing new");

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
