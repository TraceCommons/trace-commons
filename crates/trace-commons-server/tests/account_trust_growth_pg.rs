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
    trace_commons_server::account_trust_rule::parse_shadow_growth_policy(
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
        account_trust_ref, account_trust_reproduction_drill, evaluate_account_trust,
        explain_account_trust, find_account_by_ref, record_account_trust_facts,
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
    let summary = evaluate_account_trust(db, &policy, as_of, None)
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
    evaluate_account_trust(db, &policy, as_of, None)
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
    let report = explain_account_trust(db, &policy, &trust).await.unwrap();
    assert!(report.reproduced);
    assert_eq!(report.stored.as_ref().unwrap().tier, 1);
    let rendered = serde_json::to_string(&report).unwrap();
    assert!(!rendered.contains(&first.to_string()));
    assert!(!rendered.contains(&account.to_string()));
    assert!(!rendered.contains(&tenant));
    assert!(
        account_trust_reproduction_drill(db, &policy, None)
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
    let drifted = explain_account_trust(db, &policy, &trust).await.unwrap();
    assert!(!drifted.reproduced);
    assert_ne!(
        drifted.stored.as_ref().unwrap().facts_digest,
        drifted.recomputed.as_ref().unwrap().facts_digest
    );
    let drill = account_trust_reproduction_drill(db, &policy, None)
        .await
        .unwrap();
    assert!(drill.not_reproduced >= 1);
    assert!(!drill.passed());
    // Re-evaluating records the drop (1 -> 0) and reproduces again.
    evaluate_account_trust(db, &policy, at(1_001), None)
        .await
        .unwrap();
    let changes = audit(admin, &tenant).await;
    assert_eq!(changes.len(), 2);
    assert!(
        explain_account_trust(db, &policy, &trust)
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
            "SELECT count(*) FROM trace_account_trust_evaluations WHERE mode='applied'",
            &[],
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
