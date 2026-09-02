// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The public read path, exercised as the role that actually serves it.
//!
//! Every assertion here runs under `SET ROLE trace_commons_public_read`. Run
//! as the owner instead and all of them pass whether or not the policy exists
//! -- which is exactly how a missing policy reaches production.

use secrecy::SecretString;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::postgres::REGISTER_STATS_SELECT_SQL;
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::register_stats::{fetch_register_stats_row, run_register_stats_refresh};

/// Connects to the shared PostgreSQL test database and applies migrations.
///
/// Deliberately does not skip when unconfigured: these tests are `#[ignore]`
/// and only ever run deliberately (`-- --ignored`), so a missing
/// `TRACE_COMMONS_PG_TEST_DATABASE_URL` / `DATABASE_URL` -- or a database
/// that refuses the connection -- must fail loudly here, not be silently
/// skipped.
async fn test_pool() -> PgBackend {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect(
            "TRACE_COMMONS_PG_TEST_DATABASE_URL or DATABASE_URL must be set to run \
             register_stats_rls tests",
        );
    let config = DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: DatabaseConfig::login_resolver_url_from_env(),
        gate_driver_url: DatabaseConfig::gate_driver_url_from_env(),
        pii_backstop_driver_url: DatabaseConfig::pii_backstop_driver_url_from_env(),
        invite_registry_url: None,
    };
    let backend = PgBackend::new(&config)
        .await
        .expect("connect to the PostgreSQL test database");
    backend.run_migrations().await.expect("run migrations");
    backend
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_public_role_can_read_the_aggregate() {
    let backend = test_pool().await;
    let client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("connection");
    client
        .execute("SET ROLE trace_commons_public_read", &[])
        .await
        .expect("set role");

    // The statement the server actually issues, not a hand-copied projection.
    // The hand-copied one used to pass here while the shipped query was denied
    // on every request: it dropped the real query's `WHERE singleton = TRUE`,
    // and PostgreSQL column privileges cover every column a query references,
    // `WHERE` included -- so it exercised a privilege the endpoint never
    // needed and skipped the one it did. Referencing the constant is what
    // stops this test and the server drifting apart again.
    let row = client
        .query_one(REGISTER_STATS_SELECT_SQL, &[])
        .await
        .expect("the public role reads the aggregate");
    let _: i64 = row.get("traces_accepted");
    let _: i64 = row.get("contributors");
    let _: i64 = row.get("points_issued");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_public_role_can_read_nothing_else() {
    // The role exists to serve one view. If it can reach a table with rows in
    // it, the endpoint is one query change away from a leak.
    let backend = test_pool().await;
    let client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("connection");
    client
        .execute("SET ROLE trace_commons_public_read", &[])
        .await
        .expect("set role");

    // `is_err()`, with no `is_none()` escape. Every table here is FORCE-RLS'd
    // on a tenant predicate, so under this role -- which sets no tenant GUC --
    // they return zero rows WHETHER OR NOT the role holds SELECT on them. An
    // `|| is_none()` arm therefore passed for the wrong reason: it asserted
    // "RLS hid the rows", which is true even of a role that has been granted
    // the table. The property that actually bounds this role is that
    // permission is denied BEFORE RLS is ever consulted.
    for table in ["trace_submissions", "trace_credit_ledger", "trace_accounts"] {
        let result = client
            .query_opt(&format!("SELECT * FROM {table} LIMIT 1"), &[])
            .await;
        assert!(
            result.is_err(),
            "the public role must be DENIED {table}, not merely find it empty"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_public_role_cannot_write_the_aggregate() {
    // The runtime-write policy is deliberately unscoped by role (no `TO`
    // clause), because RLS policies gate rows, not operations. What
    // actually stops trace_commons_public_read writing this row is that it
    // holds no UPDATE grant on the table -- one line in the migration
    // (the GRANT SELECT is column-scoped and column-scoped to SELECT only).
    // This test is what notices if that line is ever loosened to also grant
    // UPDATE, or widened to GRANT UPDATE ... TO trace_commons_public_read.
    let backend = test_pool().await;
    let client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("connection");
    client
        .execute("SET ROLE trace_commons_public_read", &[])
        .await
        .expect("set role");

    let result = client
        .execute(
            "UPDATE trace_register_stats SET traces_accepted = traces_accepted + 1",
            &[],
        )
        .await;
    match result {
        Err(_) => {
            // Expected: PostgreSQL refuses with a permission error before
            // RLS is even consulted, because the role holds no UPDATE grant.
        }
        Ok(rows_affected) => {
            assert_eq!(
                rows_affected, 0,
                "the public role must never modify the register stats row"
            );
        }
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_public_role_does_not_bypass_rls() {
    let backend = test_pool().await;
    let client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("connection");
    let row = client
        .query_one(
            "SELECT rolbypassrls FROM pg_roles WHERE rolname = 'trace_commons_public_read'",
            &[],
        )
        .await
        .expect("the role exists");
    let bypass: bool = row.get("rolbypassrls");
    assert!(!bypass, "the public role must never bypass RLS");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_refresh_stamps_the_row_that_was_never_computed() {
    let backend = test_pool().await;

    // Deterministic setup rather than an `if before.refreshed_at.is_none()`
    // guard: on a shared test database a prior run of this same test
    // already refreshed the row, so that guard would silently stop
    // asserting anything on every run after the first. Reset the singleton
    // row to the never-computed state ourselves so the "before" half is
    // load-bearing every time, not just on a freshly migrated database.
    let client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("connection");
    client
        .execute(
            "UPDATE trace_register_stats
             SET traces_accepted = 0, contributors = 0, points_issued = 0,
                 withheld = TRUE, refreshed_at = NULL
             WHERE singleton = TRUE",
            &[],
        )
        .await
        .expect("reset the row to never-computed");
    drop(client);

    let before = fetch_register_stats_row(&backend)
        .await
        .expect("fetch before refresh");
    assert!(before.refreshed_at.is_none(), "reset to never-computed");
    assert!(before.withheld, "an uncomputed row must stay withheld");

    let after = run_register_stats_refresh(&backend, &[])
        .await
        .expect("refresh");
    assert!(after.refreshed_at.is_some(), "a refresh stamps the row");
    assert!(!after.withheld, "a refreshed row is no longer withheld");
}
