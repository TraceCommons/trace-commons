// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Daily totals of submissions the witness blocked (V154) against a real
//! PostgreSQL.
//!
//! The store runs as a real `LOGIN` role whose only privilege source is
//! `trace_ingest_runtime`. Never the migration owner and never a superuser:
//! either one ignores forced row security and a missing grant, and a test of
//! isolation or of a refusal would then pass without testing anything.
//!
//! Needs a database whose name starts with `admission_test_`, on the literal
//! host `127.0.0.1`, and `TRACE_COMMONS_PG_TEST_DATABASE_URL`. Without the
//! variable every test prints a skip line and returns. Run with
//! `--test-threads=1`: the totals are deployment-wide and each test starts
//! from an empty table.

#[path = "support/role_ddl_lock.rs"]
mod role_ddl_lock;

use chrono::{Days, Utc};
use role_ddl_lock::with_role_ddl_lock;
use secrecy::SecretString;
use trace_commons_protocol::witness_blocked::WitnessBlockedCounts;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::Database;
use trace_commons_server::db::postgres::PgBackend;
use trace_commons_server::db::witness_blocked::{WITNESS_BLOCKED_DAY_CAP, WitnessBlockedStore};

const RUNTIME_LOGIN: &str = "tc_witness_blocked_runtime_test";
const TENANT_A: &str = "witness-blocked-tenant-a";
const TENANT_B: &str = "witness-blocked-tenant-b";

struct Fixture {
    owner: PgBackend,
    runtime: PgBackend,
}

fn config(url: &str) -> DatabaseConfig {
    DatabaseConfig {
        url: SecretString::from(url.to_owned()),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

async fn fixture() -> Option<Fixture> {
    let Ok(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIP: TRACE_COMMONS_PG_TEST_DATABASE_URL not configured");
        return None;
    };
    let parsed = reqwest::Url::parse(&url).expect("database url");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().starts_with("/admission_test_"));
    let owner = PgBackend::new(&config(&url)).await.expect("owner backend");
    owner.run_migrations().await.expect("apply migrations");
    let client = owner
        .trace_pool_for_test()
        .get()
        .await
        .expect("owner client");
    client
        .batch_execute(&with_role_ddl_lock(&format!(
            "DO $$ BEGIN
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{RUNTIME_LOGIN}') THEN
                     CREATE ROLE {RUNTIME_LOGIN} LOGIN;
                 END IF;
             END $$;
             ALTER ROLE {RUNTIME_LOGIN} LOGIN INHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
             GRANT trace_ingest_runtime TO {RUNTIME_LOGIN};"
        )))
        .await
        .expect("provision the runtime login");
    drop(client);
    // Seeded and emptied inside each tenant's context, so the setup holds
    // whether or not the owner is a superuser.
    for tenant in [TENANT_A, TENANT_B] {
        owner_in_tenant(
            &owner,
            tenant,
            "INSERT INTO trace_tenants (tenant_id) VALUES (trace_current_tenant_id())
                 ON CONFLICT (tenant_id) DO NOTHING",
        )
        .await;
        owner_in_tenant(&owner, tenant, "DELETE FROM trace_witness_blocked_days").await;
    }
    let mut runtime_url = parsed.clone();
    runtime_url
        .set_username(RUNTIME_LOGIN)
        .expect("runtime username");
    runtime_url.set_password(None).expect("no password");
    let runtime = PgBackend::new(&config(runtime_url.as_str()))
        .await
        .expect("runtime backend");
    let check = runtime
        .trace_pool_for_test()
        .get()
        .await
        .expect("runtime client");
    let row = check
        .query_one(
            "SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await
        .expect("runtime identity");
    assert!(
        !row.get::<_, bool>(0),
        "the runtime login must not be superuser"
    );
    assert!(
        !row.get::<_, bool>(1),
        "the runtime login must not bypass RLS"
    );
    Some(Fixture { owner, runtime })
}

/// Run one statement as the owner inside `tenant`'s context.
async fn owner_in_tenant(owner: &PgBackend, tenant: &str, sql: &str) {
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.execute(sql, &[]).await.unwrap();
    tx.commit().await.unwrap();
}

/// `(label, count)` rows of `tenant`, read as the owner in its context.
async fn rows_of(owner: &PgBackend, tenant: &str) -> Vec<(String, i64)> {
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let rows = tx
        .query(
            "SELECT label, blocked_count FROM trace_witness_blocked_days
              WHERE tenant_id = $1 ORDER BY label",
            &[&tenant],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    tx.commit().await.unwrap();
    rows
}

fn counts(busy: u32, timed_out: u32, unreachable: u32) -> WitnessBlockedCounts {
    WitnessBlockedCounts {
        busy,
        timed_out,
        unreachable,
    }
}

fn today() -> chrono::NaiveDate {
    Utc::now().date_naive()
}

/// Reports add up per tenant, day and label; a zero label writes no row;
/// the operator's totals sum across tenants and count them.
#[tokio::test]
async fn reports_add_up_per_tenant_and_the_totals_cross_tenants() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let store: &dyn WitnessBlockedStore = &fixture.runtime;
    store
        .record_witness_blocked(TENANT_A, &counts(2, 0, 1))
        .await
        .unwrap();
    store
        .record_witness_blocked(TENANT_A, &counts(3, 0, 0))
        .await
        .unwrap();
    store
        .record_witness_blocked(TENANT_B, &counts(1, 4, 0))
        .await
        .unwrap();

    assert_eq!(
        rows_of(&fixture.owner, TENANT_A).await,
        vec![("busy".to_string(), 5), ("unreachable".to_string(), 1)]
    );
    assert_eq!(
        rows_of(&fixture.owner, TENANT_B).await,
        vec![("busy".to_string(), 1), ("timed_out".to_string(), 4)]
    );

    let stats = store
        .witness_blocked_stats(today() - Days::new(1))
        .await
        .unwrap();
    let day = |label: &str| {
        stats
            .per_day
            .iter()
            .find(|d| d.day == today() && d.label == label)
            .map(|d| (d.total.blocked, d.total.tenants))
    };
    assert_eq!(day("busy"), Some((6, 2)));
    assert_eq!(day("timed_out"), Some((4, 1)));
    assert_eq!(day("unreachable"), Some((1, 1)));
    let label = |name: &str| {
        stats
            .per_label
            .iter()
            .find(|l| l.label == name)
            .map(|l| (l.total.blocked, l.total.tenants))
    };
    assert_eq!(label("busy"), Some((6, 2)));
    assert_eq!(stats.all.blocked, 11);
    assert_eq!(stats.all.tenants, 2);

    // A window that starts tomorrow holds nothing, and reads as zero.
    let empty = store
        .witness_blocked_stats(today() + Days::new(1))
        .await
        .unwrap();
    assert!(empty.per_day.is_empty());
    assert!(empty.per_label.is_empty());
    assert_eq!((empty.all.blocked, empty.all.tenants), (0, 0));
}

/// The runtime sees only the tenant whose context it opened, cannot date a
/// row, and cannot write into another tenant.
#[tokio::test]
async fn the_runtime_is_held_to_its_tenant_and_cannot_date_a_row() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let store: &dyn WitnessBlockedStore = &fixture.runtime;
    store
        .record_witness_blocked(TENANT_A, &counts(5, 0, 0))
        .await
        .unwrap();
    store
        .record_witness_blocked(TENANT_B, &counts(0, 0, 7))
        .await
        .unwrap();

    let mut client = fixture.runtime.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&TENANT_B],
    )
    .await
    .unwrap();
    let visible: Vec<String> = tx
        .query("SELECT tenant_id FROM trace_witness_blocked_days", &[])
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(visible, vec![TENANT_B.to_string()]);

    let other_tenant = tx
        .execute(
            "INSERT INTO trace_witness_blocked_days (tenant_id, label, blocked_count)
             VALUES ($1, 'busy', 1)",
            &[&TENANT_A],
        )
        .await
        .expect_err("a row for another tenant is refused");
    assert_eq!(
        other_tenant.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE),
        "{other_tenant}"
    );
    tx.rollback().await.unwrap();

    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&TENANT_B],
    )
    .await
    .unwrap();
    let dated = tx
        .execute(
            "INSERT INTO trace_witness_blocked_days (tenant_id, day, label, blocked_count)
             VALUES ($1, DATE '2020-01-01', 'busy', 1)",
            &[&TENANT_B],
        )
        .await
        .expect_err("the runtime cannot choose the day");
    assert_eq!(
        dated.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE),
        "{dated}"
    );
    tx.rollback().await.unwrap();

    // Without a tenant context nothing is visible at all.
    let none: i64 = client
        .query_one("SELECT count(*) FROM trace_witness_blocked_days", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(none, 0);
}

/// A report over the per-label cap writes nothing; a day's total stops at
/// the daily cap.
#[tokio::test]
async fn reports_and_daily_totals_are_bounded() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let store: &dyn WitnessBlockedStore = &fixture.runtime;
    assert!(
        store
            .record_witness_blocked(TENANT_A, &counts(1_001, 0, 0))
            .await
            .is_err()
    );
    assert!(rows_of(&fixture.owner, TENANT_A).await.is_empty());

    owner_in_tenant(
        &fixture.owner,
        TENANT_A,
        &format!(
            "INSERT INTO trace_witness_blocked_days (tenant_id, label, blocked_count)
             VALUES (trace_current_tenant_id(), 'busy', {})",
            WITNESS_BLOCKED_DAY_CAP - 10
        ),
    )
    .await;
    store
        .record_witness_blocked(TENANT_A, &counts(1_000, 0, 0))
        .await
        .unwrap();
    assert_eq!(
        rows_of(&fixture.owner, TENANT_A).await,
        vec![("busy".to_string(), WITNESS_BLOCKED_DAY_CAP)]
    );
}

/// The table is forced under the central tenant predicate, so the
/// deployment's RLS diagnostics count it.
#[tokio::test]
async fn the_table_is_in_the_central_rls_list() {
    assert!(
        trace_commons_server::db::postgres::TRACE_COMMONS_RLS_TABLES
            .contains(&"trace_witness_blocked_days")
    );
    let Some(fixture) = fixture().await else {
        return;
    };
    let owner = fixture.owner.trace_pool_for_test().get().await.unwrap();
    let row = owner
        .query_one(
            "SELECT relrowsecurity, relforcerowsecurity FROM pg_class
              WHERE relname = 'trace_witness_blocked_days'",
            &[],
        )
        .await
        .unwrap();
    assert!(row.get::<_, bool>(0));
    assert!(row.get::<_, bool>(1));
}
