// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The first run's estimate requests (V153) against a real PostgreSQL.
//!
//! The store runs as a real `LOGIN` role whose only privilege sources are
//! `trace_ingest_runtime` (inherited) and, with `INHERIT FALSE`, the writer
//! and reader roles it may `SET ROLE` to. Never the migration owner and
//! never a superuser: either one ignores a missing grant, and a test of a
//! refusal would then pass without testing anything.
//!
//! Needs PostgreSQL 16 or later (`GRANT ... WITH INHERIT FALSE`), a database
//! whose name starts with `admission_test_`, on the literal host
//! `127.0.0.1`, and `TRACE_COMMONS_PG_TEST_DATABASE_URL`. Without the
//! variable every test prints a skip line and returns. Run with
//! `--test-threads=1`: the statistics are deployment-wide.

#[path = "support/role_ddl_lock.rs"]
mod role_ddl_lock;

use chrono::{Duration, Utc};
use role_ddl_lock::with_role_ddl_lock;
use secrecy::SecretString;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::Database;
use trace_commons_server::db::first_run_stats::{
    FirstRunEstimateOutcome, FirstRunEstimateRecord, FirstRunStatsStore,
};
use trace_commons_server::db::postgres::PgBackend;

const RUNTIME_LOGIN: &str = "tc_first_run_stats_runtime_test";
const MIB: u64 = 1 << 20;

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
    let sixteen: bool = client
        .query_one(
            "SELECT current_setting('server_version_num')::int >= 160000",
            &[],
        )
        .await
        .expect("server version")
        .get(0);
    assert!(sixteen, "first_run_stats_pg needs PostgreSQL 16 or later");
    client
        .batch_execute(&with_role_ddl_lock(&format!(
            "DO $$ BEGIN
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{RUNTIME_LOGIN}') THEN
                     CREATE ROLE {RUNTIME_LOGIN} LOGIN;
                 END IF;
             END $$;
             ALTER ROLE {RUNTIME_LOGIN} LOGIN INHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
             GRANT trace_ingest_runtime TO {RUNTIME_LOGIN} WITH INHERIT TRUE;
             GRANT trace_first_run_stats_writer TO {RUNTIME_LOGIN} WITH INHERIT FALSE, SET TRUE;
             GRANT trace_first_run_stats_reader TO {RUNTIME_LOGIN} WITH INHERIT FALSE, SET TRUE;"
        )))
        .await
        .expect("provision the runtime login");
    drop(client);
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

fn is_denied(error: &tokio_postgres::Error) -> bool {
    error.code() == Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
}

fn served(variant: &'static str, trace_count: u64, total_mib: u64) -> FirstRunEstimateRecord {
    FirstRunEstimateRecord {
        outcome: FirstRunEstimateOutcome::Served {
            variant,
            lines_key: if variant == "usd" {
                "usd.0123456789abcdef".into()
            } else {
                "default.2026-11-04".into()
            },
            trace_count,
            total_bytes: total_mib * MIB,
        },
        app_version: Some("0.14.2".into()),
        platform: Some("macos".into()),
    }
}

/// The runtime login records served and refused requests through the
/// writer role and reads the aggregates through the reader role. Each row
/// is dated to the hour by the database.
#[tokio::test]
async fn requests_are_recorded_and_aggregated() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let store: &dyn FirstRunStatsStore = &fixture.runtime;
    let since = Utc::now() - Duration::days(1);
    let before = store.first_run_estimate_stats(since).await.unwrap();

    store
        .record_first_run_estimate(&served("default", 4100, 940))
        .await
        .unwrap();
    store
        .record_first_run_estimate(&served("default", 4100, 1200))
        .await
        .unwrap();
    store
        .record_first_run_estimate(&served("usd", 12, 3))
        .await
        .unwrap();
    store
        .record_first_run_estimate(&FirstRunEstimateRecord {
            outcome: FirstRunEstimateOutcome::Refused {
                label: "first_run_copy_request_invalid",
            },
            app_version: None,
            platform: None,
        })
        .await
        .unwrap();

    let after = store.first_run_estimate_stats(since).await.unwrap();
    assert_eq!(after.requests - before.requests, 4);
    let count = |stats: &trace_commons_server::db::first_run_stats::FirstRunEstimateStats,
                 value: i64| {
        stats
            .trace_counts
            .iter()
            .find(|v| v.value == value)
            .map_or(0, |v| v.requests)
    };
    assert_eq!(count(&after, 4100) - count(&before, 4100), 2);
    assert_eq!(count(&after, 12) - count(&before, 12), 1);
    let mib = |stats: &trace_commons_server::db::first_run_stats::FirstRunEstimateStats,
               value: i64| {
        stats
            .total_mib
            .iter()
            .find(|v| v.value == value)
            .map_or(0, |v| v.requests)
    };
    assert_eq!(mib(&after, 940) - mib(&before, 940), 1);
    let outcome = |stats: &trace_commons_server::db::first_run_stats::FirstRunEstimateStats,
                   name: &str| {
        stats
            .outcomes
            .iter()
            .filter(|o| o.outcome == name)
            .map(|o| o.requests)
            .sum::<i64>()
    };
    assert_eq!(outcome(&after, "default") - outcome(&before, "default"), 2);
    assert_eq!(outcome(&after, "usd") - outcome(&before, "usd"), 1);
    assert_eq!(outcome(&after, "refused") - outcome(&before, "refused"), 1);
    let today = Utc::now().date_naive();
    assert!(after.per_day.iter().any(|d| d.day == today));

    // Dated to the hour, by the database.
    let owner = fixture.owner.trace_pool_for_test().get().await.unwrap();
    let off_hour: i64 = owner
        .query_one(
            "SELECT count(*) FROM trace_first_run_estimate_requests
              WHERE received_hour <> date_trunc('hour', received_hour)",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(off_hour, 0);
}

/// The stance: forced RLS, exactly two policies, an insert-only writer and
/// a read-only reader, and nothing at all for the runtime group itself.
#[tokio::test]
async fn the_writer_only_inserts_the_reader_only_reads_and_the_runtime_neither() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let insert = "INSERT INTO trace_first_run_estimate_requests
                      (outcome, lines_key, trace_count, total_bytes)
                  VALUES ('default', 'default.2026-11-04', 10, 0)";
    let attempts: [(&str, &str); 9] = [
        ("", insert),
        ("", "SELECT count(*) FROM trace_first_run_estimate_requests"),
        (
            "trace_first_run_stats_writer",
            "SELECT count(*) FROM trace_first_run_estimate_requests",
        ),
        (
            "trace_first_run_stats_writer",
            "UPDATE trace_first_run_estimate_requests SET trace_count = 0",
        ),
        (
            "trace_first_run_stats_writer",
            "DELETE FROM trace_first_run_estimate_requests",
        ),
        (
            "trace_first_run_stats_writer",
            "INSERT INTO trace_first_run_estimate_requests
                 (received_hour, outcome, lines_key, trace_count, total_bytes)
             VALUES (date_trunc('hour', now()) - interval '1 day', 'default', 'k', 10, 0)",
        ),
        ("trace_first_run_stats_reader", insert),
        (
            "trace_first_run_stats_reader",
            "UPDATE trace_first_run_estimate_requests SET trace_count = 0",
        ),
        (
            "trace_first_run_stats_reader",
            "DELETE FROM trace_first_run_estimate_requests",
        ),
    ];
    for (role, sql) in attempts {
        let mut client = fixture.runtime.trace_pool_for_test().get().await.unwrap();
        let tx = client.transaction().await.unwrap();
        if !role.is_empty() {
            tx.batch_execute(&format!("SET LOCAL ROLE {role}"))
                .await
                .unwrap();
        }
        let error = tx
            .execute(sql, &[])
            .await
            .expect_err(&format!("{role:?} ran: {sql}"));
        assert!(is_denied(&error), "{role:?} {sql}: {error}");
    }

    let owner = fixture.owner.trace_pool_for_test().get().await.unwrap();
    let forced: (bool, bool) = owner
        .query_one(
            "SELECT relrowsecurity, relforcerowsecurity FROM pg_class
              WHERE oid = 'trace_first_run_estimate_requests'::regclass",
            &[],
        )
        .await
        .map(|row| (row.get(0), row.get(1)))
        .unwrap();
    assert_eq!(forced, (true, true));
    let policies: Vec<String> = owner
        .query(
            "SELECT policyname || ':' || cmd || ':' || array_to_string(roles, ',')
               FROM pg_policies WHERE tablename = 'trace_first_run_estimate_requests'
              ORDER BY 1",
            &[],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(
        policies,
        vec![
            "trace_first_run_estimate_requests_insert:INSERT:trace_first_run_stats_writer",
            "trace_first_run_estimate_requests_read:SELECT:trace_first_run_stats_reader",
        ]
    );
    let roles: Vec<(String, bool, bool, bool)> = owner
        .query(
            "SELECT rolname::text, rolcanlogin, rolsuper, rolbypassrls FROM pg_roles
              WHERE rolname IN ('trace_first_run_stats_writer', 'trace_first_run_stats_reader')
              ORDER BY 1",
            &[],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
        .collect();
    assert_eq!(roles.len(), 2);
    for (name, login, superuser, bypass) in roles {
        assert!(!login && !superuser && !bypass, "{name}");
    }
}

/// The table's shape refuses a refusal with figures, a served row without
/// them, a size that is not whole MiB, and an unknown platform.
#[tokio::test]
async fn the_row_shape_is_enforced() {
    let Some(fixture) = fixture().await else {
        return;
    };
    for sql in [
        "INSERT INTO trace_first_run_estimate_requests
             (outcome, refusal_label, trace_count, total_bytes)
         VALUES ('refused', 'first_run_copy_request_invalid', 4123, 0)",
        "INSERT INTO trace_first_run_estimate_requests (outcome, lines_key)
         VALUES ('default', 'default.2026-11-04')",
        "INSERT INTO trace_first_run_estimate_requests
             (outcome, lines_key, trace_count, total_bytes)
         VALUES ('default', 'default.2026-11-04', 10, 4123)",
        "INSERT INTO trace_first_run_estimate_requests
             (outcome, lines_key, trace_count, total_bytes, platform)
         VALUES ('default', 'default.2026-11-04', 10, 0, 'beos')",
        "INSERT INTO trace_first_run_estimate_requests
             (outcome, lines_key, trace_count, total_bytes)
         VALUES ('free text', 'default.2026-11-04', 10, 0)",
    ] {
        let mut client = fixture.runtime.trace_pool_for_test().get().await.unwrap();
        let tx = client.transaction().await.unwrap();
        tx.batch_execute("SET LOCAL ROLE trace_first_run_stats_writer")
            .await
            .unwrap();
        let error = tx.execute(sql, &[]).await.expect_err(sql);
        assert_eq!(
            error.code(),
            Some(&tokio_postgres::error::SqlState::CHECK_VIOLATION),
            "{sql}: {error}"
        );
    }
}
