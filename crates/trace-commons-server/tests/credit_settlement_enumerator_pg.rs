// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use chrono::{DateTime, Duration, Utc};
use trace_commons_server::{
    config::DatabaseConfig,
    db::{Database, postgres::PgBackend},
};
use uuid::Uuid;

type EventFixture<'a> = (
    &'a str,
    &'a str,
    &'a str,
    Option<&'a str>,
    DateTime<Utc>,
    bool,
);

async fn event(db: &PgBackend, tenant: &str, fixture: EventFixture<'_>) {
    let (kind, points, status, external, at, settled) = fixture;
    let conn = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let id = Uuid::new_v4();
    conn.execute(
        "INSERT INTO trace_tenants(tenant_id) VALUES($1) ON CONFLICT DO NOTHING",
        &[&tenant],
    )
    .await
    .unwrap();
    conn.execute("INSERT INTO trace_submissions(tenant_id,submission_id,trace_id,auth_principal_ref,schema_version,consent_policy_version,retention_policy_id,status,privacy_risk,redaction_pipeline_version,redaction_hash) VALUES($1,$2,$2,'private-principal','v1','v1','default',$3,'low','v1','hash')", &[&tenant,&id,&status]).await.unwrap();
    conn.execute("INSERT INTO trace_credit_ledger(tenant_id,credit_event_id,submission_id,trace_id,credit_account_ref,event_type,points_delta,reason,external_ref,actor_principal_ref,actor_role,settlement_state,occurred_at) VALUES($1,$2,$2,$2,'private-account',$3,$4,'private-reason',$5,'private-actor','operator','pending',$6)", &[&tenant,&id,&kind,&points,&external,&at]).await.unwrap();
    if settled {
        conn.execute("INSERT INTO trace_credit_settlement_batches(tenant_id,settlement_batch_id,policy_version,status,reason_hash,source_credit_event_ids,source_list_hash,settled_credit_points,settled_credit_micros,actor_principal_ref) VALUES($1,$2,'v1','finalized','hash',ARRAY[$2::uuid],$2::text,'1',1,'private-actor')", &[&tenant,&id]).await.unwrap();
    }
}

#[tokio::test]
async fn discovers_bounded_pending_tenants_without_content_authority() {
    let Ok(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL") else {
        return;
    };
    assert!(url.contains("127.0.0.1"));
    let db = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 4))
        .await
        .unwrap();
    db.run_migrations().await.unwrap();
    let prefix = format!("enum-{}-", Uuid::new_v4());
    let a = format!("{prefix}a");
    let b = format!("{prefix}b");
    let c = format!("{prefix}c");
    let d = format!("{prefix}d");
    let now = Utc::now();
    let cutoff = now - Duration::hours(1);
    event(
        &db,
        &a,
        ("training_utility", "1", "accepted", None, now, false),
    )
    .await;
    event(
        &db,
        &b,
        (
            "novelty_utility",
            "2",
            "accepted",
            Some("novelty_utility:gate"),
            now,
            false,
        ),
    )
    .await;
    // A tenant can have multiple candidate events and must appear only once.
    event(
        &db,
        &a,
        ("ranking_utility", "1", "accepted", None, now, false),
    )
    .await;
    for (kind, points, status, external, at, settled) in [
        ("training_utility", "0", "accepted", None, now, false),
        ("training_utility", "-1", "accepted", None, now, false),
        ("training_utility", "NaN", "accepted", None, now, false),
        ("training_utility", "bad", "accepted", None, now, false),
        ("training_utility", "1", "rejected", None, now, false),
        ("upload", "1", "accepted", None, now, false),
        ("novelty_utility", "1", "accepted", None, now, false),
        (
            "novelty_utility",
            "1",
            "accepted",
            Some("noveltyXutility:gate"),
            now,
            false,
        ),
        (
            "novelty_utility",
            "1",
            "accepted",
            Some("novelty_utility:gate"),
            cutoff - Duration::seconds(1),
            false,
        ),
        ("training_utility", "1", "accepted", None, now, true),
    ] {
        event(&db, &c, (kind, points, status, external, at, settled)).await;
    }
    event(
        &db,
        &d,
        ("regression_catch", "3", "accepted", None, now, false),
    )
    .await;
    assert_eq!(
        db.pending_credit_settlement_tenants(Some(&prefix), 1, Some(cutoff))
            .await
            .unwrap(),
        vec![a.clone()]
    );
    assert_eq!(
        db.pending_credit_settlement_tenants(Some(&a), 1, Some(cutoff))
            .await
            .unwrap(),
        vec![b.clone()]
    );
    assert_eq!(
        db.pending_credit_settlement_tenants(Some(&prefix), 1000, None)
            .await
            .unwrap(),
        vec![a.clone(), d.clone()]
    );
    assert!(
        db.pending_credit_settlement_tenants(Some(&prefix), 0, Some(cutoff))
            .await
            .is_err()
    );
    assert!(
        db.pending_credit_settlement_tenants(Some(&b), 1001, Some(cutoff))
            .await
            .is_err()
    );
    assert_eq!(
        db.pending_credit_settlement_tenants(Some(&b), 1000, Some(cutoff))
            .await
            .unwrap(),
        vec![d.clone()]
    );
    let mut conn = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    conn.batch_execute("DO $$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='tc_credit_enum_test') THEN CREATE ROLE tc_credit_enum_test LOGIN NOSUPERUSER NOBYPASSRLS INHERIT; END IF; END $$; REVOKE trace_credit_settlement_enumerator FROM tc_credit_enum_test; ALTER ROLE tc_credit_enum_test INHERIT; GRANT trace_ingest_runtime TO tc_credit_enum_test; GRANT SELECT ON trace_credit_ledger,trace_submissions,trace_credit_settlement_batches TO tc_credit_enum_test;").await.unwrap();
    let mut runtime_url = reqwest::Url::parse(&url).unwrap();
    runtime_url.set_username("tc_credit_enum_test").unwrap();
    let runtime = PgBackend::new(&DatabaseConfig::from_postgres_url(runtime_url.as_str(), 1))
        .await
        .unwrap();
    assert!(
        runtime
            .pending_credit_settlement_tenants(Some(&prefix), 1, Some(cutoff))
            .await
            .is_err(),
        "missing enumerator authority must fail closed"
    );
    conn.batch_execute("GRANT trace_credit_settlement_enumerator TO tc_credit_enum_test")
        .await
        .unwrap();
    // A single-connection pool makes role restoration observable on the same
    // connection that performs enumeration, before and after its transaction.
    for stage in 0..2 {
        let mut runtime_conn = runtime
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .unwrap();
        assert_eq!(
            runtime_conn
                .query_one("SELECT current_user::text", &[])
                .await
                .unwrap()
                .get::<_, String>(0),
            "tc_credit_enum_test"
        );
        let runtime_tx = runtime_conn.transaction().await.unwrap();
        runtime_tx
            .execute(
                "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
                &[&a],
            )
            .await
            .unwrap();
        for relation in [
            "trace_credit_ledger",
            "trace_submissions",
            "trace_credit_settlement_batches",
        ] {
            let rows = runtime_tx
                .query(
                    &format!("SELECT tenant_id FROM {relation} WHERE tenant_id <> $1"),
                    &[&a],
                )
                .await
                .unwrap();
            assert!(
                rows.is_empty(),
                "inherited role bypassed tenant RLS on {relation} at stage {stage}"
            );
        }
        assert_eq!(
            runtime_tx
                .query(
                    "SELECT auth_principal_ref FROM trace_submissions WHERE tenant_id=$1",
                    &[&b]
                )
                .await
                .unwrap()
                .len(),
            0
        );
        runtime_tx.commit().await.unwrap();
        drop(runtime_conn);
        if stage == 0 {
            assert_eq!(
                runtime
                    .pending_credit_settlement_tenants(Some(&prefix), 1, Some(cutoff))
                    .await
                    .unwrap(),
                vec![a.clone()]
            );
        }
    }
    let tx = conn.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE trace_credit_settlement_enumerator")
        .await
        .unwrap();
    assert!(!tx.query_one("SELECT rolsuper OR rolbypassrls OR rolcanlogin FROM pg_roles WHERE rolname=current_user",&[]).await.unwrap().get::<_,bool>(0));
    assert_eq!(
        tx.query(
            "SELECT DISTINCT tenant_id FROM trace_credit_ledger WHERE tenant_id LIKE $1",
            &[&format!("{prefix}%")]
        )
        .await
        .unwrap()
        .len(),
        4
    );
    tx.commit().await.unwrap();
    for forbidden in [
        "SELECT auth_principal_ref FROM trace_submissions",
        "SELECT credit_account_ref FROM trace_credit_ledger",
        "SELECT actor_principal_ref FROM trace_credit_settlement_batches",
        "UPDATE trace_credit_ledger SET points_delta='100'",
    ] {
        let tx = conn.transaction().await.unwrap();
        tx.batch_execute("SET LOCAL ROLE trace_credit_settlement_enumerator")
            .await
            .unwrap();
        assert_eq!(
            tx.batch_execute(forbidden)
                .await
                .unwrap_err()
                .code()
                .map(|c| c.code()),
            Some("42501")
        );
        tx.rollback().await.unwrap();
    }
}
