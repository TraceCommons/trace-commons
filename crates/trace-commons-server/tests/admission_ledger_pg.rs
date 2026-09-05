// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Explicit isolated PostgreSQL test: never falls back to DATABASE_URL or skips failure.
use trace_commons_server::{
    admission_ledger::{AdmissionDecision as D, AdmissionLimits, AdmissionReservation},
    config::{DatabaseConfig, SslMode},
    db::{Database, postgres::PgBackend},
};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ADMISSION_PG_TEST_URL"]
async fn atomic_admission_replay_budget_recovery_and_rls() {
    let url = std::env::var("TRACE_COMMONS_ADMISSION_PG_TEST_URL")
        .expect("explicit isolated test URL required");
    let target = url.parse::<tokio_postgres::Config>().unwrap();
    assert!(
        matches!(target.get_hosts().first(), Some(tokio_postgres::config::Host::Tcp(host)) if host == "127.0.0.1" || host == "localhost")
    );
    assert!(
        target
            .get_dbname()
            .is_some_and(|name| name.starts_with("admission_test")),
        "dedicated admission_test database required before destructive fixtures"
    );
    let (admin, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        connection.await.unwrap();
    });
    admin.batch_execute("CREATE OR REPLACE FUNCTION trace_current_tenant_id() RETURNS TEXT LANGUAGE SQL STABLE AS $$ SELECT NULLIF(current_setting('trace_commons.trace_tenant_id',true),'') $$;").await.unwrap();
    admin
        .batch_execute(include_str!(
            "../../../migrations/V59__trace_admission_ledger.sql"
        ))
        .await
        .unwrap();
    admin.batch_execute("DO $$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='admission_test_runtime') THEN CREATE ROLE admission_test_runtime LOGIN NOBYPASSRLS; END IF; END $$;
        GRANT USAGE ON SCHEMA public TO admission_test_runtime;
        GRANT SELECT,INSERT,UPDATE ON trace_admission_challenges,trace_admission_accounts,trace_admission_submissions TO admission_test_runtime;
        GRANT EXECUTE ON FUNCTION trace_reserve_admission(TEXT,TEXT,UUID,TEXT,TEXT,TEXT,BIGINT,BIGINT,BIGINT,BIGINT,UUID,BIGINT),trace_transition_admission(TEXT,UUID,UUID,TEXT) TO admission_test_runtime;
        TRUNCATE trace_admission_challenges,trace_admission_accounts,trace_admission_submissions,trace_admission_receipts,trace_admission_global_budget;").await.unwrap();
    let mut parsed = url.parse::<tokio_postgres::Config>().unwrap();
    parsed.user("admission_test_runtime");
    let host = parsed.get_hosts().first().unwrap();
    let tokio_postgres::config::Host::Tcp(host) = host else {
        panic!("TCP localhost required")
    };
    assert!(
        host == "127.0.0.1" || host == "localhost",
        "isolated localhost only"
    );
    let runtime_url = format!(
        "postgresql://admission_test_runtime@{host}:{}/{}",
        parsed.get_ports()[0],
        parsed.get_dbname().unwrap()
    );
    let backend = PgBackend::new(&DatabaseConfig {
        url: runtime_url.clone().into(),
        pool_size: 1,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    })
    .await
    .unwrap();
    let limits = AdmissionLimits {
        window_attempts: 1,
        account_cost_limit: 50,
        global_cost_limit: 100,
        processing_cost_bound: 10,
        lease_seconds: 60,
        challenge_ttl_seconds: 60,
    };
    let make = |tenant: &str, anchor: &str| AdmissionReservation {
        tenant_id: tenant.into(),
        anchor_hash: anchor.repeat(64),
        submission_id: Uuid::new_v4(),
        body_hash: "b".repeat(64),
        receipt_hash: None,
        challenge_hash: None,
        lease_id: Uuid::new_v4(),
        limits: limits.clone(),
    };
    let mut first = make("tenant-a", "a");
    let (left, right) = tokio::join!(
        backend.reserve_submission_admission(&first),
        backend.reserve_submission_admission(&first)
    );
    let decisions = [left.unwrap(), right.unwrap()];
    assert!(decisions.contains(&D::Reserved) && decisions.contains(&D::Busy));
    assert_eq!(
        backend
            .reserve_submission_admission(&make("tenant-a", "a"))
            .await
            .unwrap(),
        D::Exhausted
    );
    assert!(
        backend
            .transition_submission_admission(
                "tenant-a",
                first.submission_id,
                first.lease_id,
                "released"
            )
            .await
            .unwrap()
    );
    assert!(
        !backend
            .transition_submission_admission(
                "tenant-a",
                first.submission_id,
                first.lease_id,
                "released"
            )
            .await
            .unwrap()
    );
    first.lease_id = Uuid::new_v4();
    assert_eq!(
        backend.reserve_submission_admission(&first).await.unwrap(),
        D::Reserved
    );
    assert!(
        backend
            .transition_submission_admission(
                "tenant-a",
                first.submission_id,
                first.lease_id,
                "processing"
            )
            .await
            .unwrap()
    );
    assert!(
        !backend
            .transition_submission_admission(
                "tenant-a",
                first.submission_id,
                first.lease_id,
                "released"
            )
            .await
            .unwrap()
    );
    // A crashed processing attempt retains its bound and slot. A fresh
    // lease reserves another bound; the stale holder cannot complete it.
    let old_lease = first.lease_id;
    admin.execute("UPDATE trace_admission_submissions SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE submission_id=$1",&[&first.submission_id]).await.unwrap();
    first.lease_id = Uuid::new_v4();
    assert_eq!(
        backend.reserve_submission_admission(&first).await.unwrap(),
        D::Reserved
    );
    assert!(
        !backend
            .transition_submission_admission(
                "tenant-a",
                first.submission_id,
                old_lease,
                "completed"
            )
            .await
            .unwrap()
    );
    assert!(
        backend
            .transition_submission_admission(
                "tenant-a",
                first.submission_id,
                first.lease_id,
                "processing"
            )
            .await
            .unwrap()
    );
    assert!(
        backend
            .transition_submission_admission(
                "tenant-a",
                first.submission_id,
                first.lease_id,
                "completed"
            )
            .await
            .unwrap()
    );
    assert_eq!(
        backend.reserve_submission_admission(&first).await.unwrap(),
        D::Completed
    );
    let mut changed = first.clone();
    changed.body_hash = "c".repeat(64);
    assert_eq!(
        backend
            .reserve_submission_admission(&changed)
            .await
            .unwrap(),
        D::Conflict
    );
    let lock = backend
        .acquire_admission_processing_lock("tenant-a", first.submission_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        backend
            .acquire_admission_processing_lock("tenant-a", first.submission_id)
            .await
            .unwrap()
            .is_none()
    );
    let second_guard = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        backend.acquire_admission_processing_lock("tenant-other", Uuid::new_v4()),
    )
    .await
    .expect("a held guard must not occupy the sole pool slot")
    .unwrap()
    .unwrap();
    assert_eq!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            backend.reserve_submission_admission(&first)
        )
        .await
        .expect("two guards cannot starve ledger pool")
        .unwrap(),
        D::Completed
    );
    drop(second_guard);
    drop(lock);
    let mut reacquired = None;
    for _ in 0..20 {
        reacquired = backend
            .acquire_admission_processing_lock("tenant-a", first.submission_id)
            .await
            .unwrap();
        if reacquired.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(reacquired.is_some());
    drop(reacquired);
    let mut attested = make("tenant-a", "a");
    attested.receipt_hash = Some("d".repeat(64));
    attested.challenge_hash = Some("e".repeat(64));
    backend
        .issue_admission_challenge(
            "tenant-a",
            &attested.anchor_hash,
            attested.challenge_hash.as_ref().unwrap(),
            chrono::Utc::now() + chrono::Duration::minutes(1),
        )
        .await
        .unwrap();
    assert_eq!(
        backend
            .reserve_submission_admission(&attested)
            .await
            .unwrap(),
        D::Reserved
    );
    let mut reused = make("tenant-b", "f");
    reused.receipt_hash = attested.receipt_hash.clone();
    reused.challenge_hash = Some("f".repeat(64));
    backend
        .issue_admission_challenge(
            "tenant-b",
            &reused.anchor_hash,
            reused.challenge_hash.as_ref().unwrap(),
            chrono::Utc::now() + chrono::Duration::minutes(1),
        )
        .await
        .unwrap();
    assert_eq!(
        backend.reserve_submission_admission(&reused).await.unwrap(),
        D::Refused
    );
    reused.receipt_hash = Some("1".repeat(64));
    assert_eq!(
        backend.reserve_submission_admission(&reused).await.unwrap(),
        D::Reserved,
        "refused replay must not consume challenge"
    );
    let mut expired = make("tenant-c", "2");
    expired.receipt_hash = Some("2".repeat(64));
    expired.challenge_hash = Some("3".repeat(64));
    backend
        .issue_admission_challenge(
            "tenant-c",
            &expired.anchor_hash,
            expired.challenge_hash.as_ref().unwrap(),
            chrono::Utc::now() - chrono::Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(
        backend
            .reserve_submission_admission(&expired)
            .await
            .unwrap(),
        D::Refused
    );
    let mut wrong = make("tenant-d", "4");
    wrong.receipt_hash = Some("4".repeat(64));
    wrong.challenge_hash = Some("5".repeat(64));
    backend
        .issue_admission_challenge(
            "tenant-d",
            &"5".repeat(64),
            wrong.challenge_hash.as_ref().unwrap(),
            chrono::Utc::now() + chrono::Duration::minutes(1),
        )
        .await
        .unwrap();
    assert_eq!(
        backend.reserve_submission_admission(&wrong).await.unwrap(),
        D::Refused
    );
    // Competing tenants share the same global ceiling, enforced atomically.
    let mut a = make("tenant-e", "6");
    a.limits.processing_cost_bound = 50;
    let mut b = make("tenant-f", "7");
    b.limits.processing_cost_bound = 50;
    let (a, b) = tokio::join!(
        backend.reserve_submission_admission(&a),
        backend.reserve_submission_admission(&b)
    );
    let results = [a.unwrap(), b.unwrap()];
    assert!(results.contains(&D::Reserved) && results.contains(&D::Exhausted));
    let (client, conn) = tokio_postgres::connect(&runtime_url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        conn.await.unwrap();
    });
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM trace_admission_submissions", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0,
        "missing tenant must hide all rows"
    );
    assert!(
        client
            .query("SELECT * FROM trace_admission_receipts", &[])
            .await
            .is_err(),
        "runtime cannot inspect global receipt hashes"
    );
    let row = admin
        .query_one(
            "SELECT cost_bound_used FROM trace_admission_global_budget",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), 90);
}
