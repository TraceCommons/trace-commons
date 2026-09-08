// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Applying a migration and recording it must commit or roll back together.
//!
//! The migration runner used to `batch_execute` a migration file and then, as a
//! separate statement on the same connection, insert its row into
//! `_trace_commons_migrations`. Both statements succeed in every normal run, so
//! no test in the suite could see the difference: the gap only shows when the
//! second one does not happen. This test forces that case -- a recording table
//! that rejects the row -- and requires the migration's own DDL to be gone
//! afterwards.
//!
//! Requires PostgreSQL: set `TRACE_COMMONS_PG_TEST_DATABASE_URL` or
//! `DATABASE_URL`. CI runs no PostgreSQL, so these skip there;
//! `applying_a_migration_and_recording_it_share_one_transaction` in
//! `src/db/postgres.rs` is what gates the shape on every run.
//!
//! Everything here lives in a scratch schema of its own and is dropped again,
//! so the shared test database keeps whatever migration state it already had.

use secrecy::SecretString;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{
    Database, postgres::PgBackend, postgres::apply_and_record_migration,
};

/// Connects and puts the session in a private scratch schema holding its own
/// `_trace_commons_migrations`. Returns `None` when no database is configured.
///
/// `search_path` is what makes this work: the runner writes to an unqualified
/// `_trace_commons_migrations`, so the scratch copy in front of the path is the
/// one it finds, and the real recording table is never touched.
async fn scratch_client(schema: &str, reject_version: i32) -> Option<tokio_postgres::Client> {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;

    let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("connect to the configured test database");
    tokio::spawn(async move {
        let _ = connection.await;
    });

    client
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {schema} CASCADE;
             CREATE SCHEMA {schema};
             SET search_path = {schema}, public;
             CREATE TABLE _trace_commons_migrations (
                 version INTEGER PRIMARY KEY,
                 name TEXT NOT NULL,
                 applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                 CONSTRAINT probe_rejects_one_version CHECK (version <> {reject_version})
             );"
        ))
        .await
        .expect("set up the scratch schema");

    Some(client)
}

async fn drop_scratch(client: &tokio_postgres::Client, schema: &str) {
    client
        .batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE;"))
        .await
        .expect("drop the scratch schema");
}

async fn probe_table_exists(client: &tokio_postgres::Client, schema: &str) -> bool {
    let exists: Option<String> = client
        .query_one(
            &format!("SELECT to_regclass('{schema}.atomicity_probe')::TEXT"),
            &[],
        )
        .await
        .expect("look the probe table up")
        .get(0);
    exists.is_some()
}

/// The failure this exists for: the record cannot be written, so the migration
/// must not stay applied. Before the transaction, `atomicity_probe` survived
/// and the next boot re-ran V999 into `relation already exists`.
#[tokio::test]
async fn a_migration_whose_recording_fails_leaves_nothing_applied() {
    let schema = "trace_migration_atomicity_rollback";
    let Some(client) = scratch_client(schema, 999).await else {
        eprintln!("skipping: TRACE_COMMONS_PG_TEST_DATABASE_URL or DATABASE_URL not configured");
        return;
    };
    let mut client = client;

    let result = apply_and_record_migration(
        &mut client,
        999,
        "atomicity_probe",
        "CREATE TABLE atomicity_probe (id INTEGER PRIMARY KEY);",
    )
    .await;

    assert!(
        result.is_err(),
        "the CHECK constraint must reject version 999, or this test proves nothing"
    );
    assert!(
        !probe_table_exists(&client, schema).await,
        "the migration's table outlived the failed recording: the apply and the \
         record are not in one transaction, and a crash between them leaves an \
         applied-but-unrecorded migration that fails every later boot"
    );

    let recorded: i64 = client
        .query_one("SELECT COUNT(*) FROM _trace_commons_migrations", &[])
        .await
        .expect("count recordings")
        .get(0);
    assert_eq!(
        recorded, 0,
        "nothing may be recorded when the insert failed"
    );

    drop_scratch(&client, schema).await;
}

/// The rollback is not bought by refusing to apply anything: a migration whose
/// recording succeeds must leave both its DDL and its row behind.
#[tokio::test]
async fn a_migration_that_records_cleanly_keeps_both_halves() {
    let schema = "trace_migration_atomicity_commit";
    let Some(client) = scratch_client(schema, -1).await else {
        eprintln!("skipping: TRACE_COMMONS_PG_TEST_DATABASE_URL or DATABASE_URL not configured");
        return;
    };
    let mut client = client;

    apply_and_record_migration(
        &mut client,
        999,
        "atomicity_probe",
        "CREATE TABLE atomicity_probe (id INTEGER PRIMARY KEY);",
    )
    .await
    .expect("a migration with a writable recording must apply");

    assert!(
        probe_table_exists(&client, schema).await,
        "the migration's own DDL must be durable after the commit"
    );

    let row = client
        .query_one("SELECT version, name FROM _trace_commons_migrations", &[])
        .await
        .expect("read the recording");
    let version: i32 = row.get(0);
    let name: String = row.get(1);
    assert_eq!(
        (version, name.as_str()),
        (999, "atomicity_probe"),
        "the recorded (version, name) must be exactly what was applied"
    );

    drop_scratch(&client, schema).await;
}

/// Splits the database name off a connection URL and puts a different one back,
/// so a test can reach a second database on the server it was pointed at.
fn with_database(url: &str, database: &str) -> String {
    let (head, query) = match url.split_once('?') {
        Some((head, query)) => (head, Some(query)),
        None => (url, None),
    };
    let (base, _) = head
        .rsplit_once('/')
        .expect("a connection URL ending in /<database>");
    match query {
        Some(query) => format!("{base}/{database}?{query}"),
        None => format!("{base}/{database}"),
    }
}

async fn connect(url: &str) -> tokio_postgres::Client {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .unwrap_or_else(|error| panic!("connect to {url}: {error}"));
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

/// Nothing serialised the migration runner. `CREATE TABLE IF NOT EXISTS` is not
/// atomic against a concurrent `CREATE TABLE` in PostgreSQL, and neither is the
/// applied/not-applied check the runner makes per migration: two callers both
/// read a migration as unapplied and both run its DDL, and the loser dies on a
/// system-catalog unique index with `duplicate key value violates unique
/// constraint "pg_type_typname_nsp_index"`.
///
/// That is reachable in production -- a rolling restart, or an operator running
/// `trace-commons-upload-claim-issuer import-invites` (which migrates at boot)
/// beside a starting service. It is also why `trace_invite_registry_pg` failed
/// twenty of its twenty-one tests: every one of them migrates, and libtest runs
/// them in parallel.
///
/// A virgin database is essential. Against one that is already migrated the
/// runner applies nothing, races over nothing, and this test passes without
/// having exercised the path at all.
#[tokio::test]
async fn concurrent_migration_runs_do_not_race_on_a_virgin_database() {
    let Some(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
    else {
        eprintln!("skipping: TRACE_COMMONS_PG_TEST_DATABASE_URL or DATABASE_URL not configured");
        return;
    };

    // A database of its own, created and dropped here: the point is to start
    // from nothing, which the shared test database cannot offer twice.
    const PROBE_DB: &str = "trace_migration_concurrency_probe";
    let admin = connect(&with_database(&url, "postgres")).await;
    // Separate statements on purpose. `batch_execute` wraps a multi-statement
    // string in an implicit transaction, and DROP/CREATE DATABASE cannot run
    // inside one.
    admin
        .execute(&format!("DROP DATABASE IF EXISTS {PROBE_DB}"), &[])
        .await
        .expect("drop any probe database left by an earlier run");
    admin
        .execute(&format!("CREATE DATABASE {PROBE_DB}"), &[])
        .await
        .expect("create the probe database");

    let probe_url = with_database(&url, PROBE_DB);
    let mut runs = Vec::new();
    for _ in 0..4 {
        let probe_url = probe_url.clone();
        runs.push(tokio::spawn(async move {
            let config = DatabaseConfig {
                url: SecretString::from(probe_url),
                pool_size: 4,
                ssl_mode: SslMode::Prefer,
                login_resolver_url: DatabaseConfig::login_resolver_url_from_env(),
                gate_driver_url: DatabaseConfig::gate_driver_url_from_env(),
                pii_backstop_driver_url: DatabaseConfig::pii_backstop_driver_url_from_env(),
                invite_registry_url: DatabaseConfig::invite_registry_url_from_env(),
            };
            let backend = PgBackend::new(&config).await.expect("backend");
            backend.run_migrations().await
        }));
    }

    let mut failures = Vec::new();
    for run in runs {
        if let Err(error) = run.await.expect("migration task must not panic") {
            failures.push(error.to_string());
        }
    }

    let dropped = admin
        .execute(
            &format!("DROP DATABASE IF EXISTS {PROBE_DB} WITH (FORCE)"),
            &[],
        )
        .await;

    assert!(
        failures.is_empty(),
        "every concurrent migration run must succeed; {} of 4 failed: {failures:#?}",
        failures.len()
    );
    dropped.expect("drop the probe database");
}
