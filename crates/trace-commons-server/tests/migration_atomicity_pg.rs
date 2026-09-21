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
//! `DATABASE_URL`, and these skip without one. CI's PostgreSQL job runs this
//! suite as its own step; the plain `cargo test` job has no server, so
//! `applying_a_migration_and_recording_it_share_one_transaction` and the
//! `no_migration_*` tests in `src/db/postgres.rs` gate the shape there.
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

/// Held by every test here that migrates a database of its own from nothing.
///
/// The runner's advisory lock is per database, and the roles migrations create
/// are per server: two virgin databases migrating side by side both reach
/// `CREATE ROLE` and `ALTER ROLE` on the same catalog rows, and one of them
/// loses with `tuple concurrently updated`. libtest runs this file's tests in
/// parallel, so they take turns instead.
static VIRGIN_DATABASE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Puts a different login in a connection URL, keeping host, database and query.
fn with_user(url: &str, user: &str, password: &str) -> String {
    let (scheme, rest) = url
        .split_once("://")
        .expect("a connection URL with a scheme");
    let host_onwards = match rest.rsplit_once('@') {
        Some((_, host_onwards)) => host_onwards,
        None => rest,
    };
    format!("{scheme}://{user}:{password}@{host_onwards}")
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

    let _turn = VIRGIN_DATABASE.lock().await;

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

/// Every suite migrates as a superuser, and a superuser may do things the role
/// an operator is told to migrate with may not. The pilot found one the hard
/// way: V64, V71 and V72 attached `SET trace_commons.trace_tenant_id = ''` to
/// six functions, which PostgreSQL permits only to a true superuser, so the
/// database's own non-superuser owner could not apply them at all --
/// `permission denied to set parameter "trace_commons.trace_tenant_id"`.
///
/// So: a login that owns its database and may create roles, and nothing more,
/// applies every migration through the real runner.
///
/// The second half is what the fix had to keep. Those clauses cleared the
/// caller's tenant for the length of the call, and that is load-bearing: row
/// policies combine with OR, so a definer function entered with a tenant still
/// set also sees that tenant's rows -- for `trace_public_run_page`, its
/// withdrawn pages. The replacement clears the setting in the body, so it also
/// has to hand the caller's value back: the server calls these from inside a
/// tenant-scoped write transaction and keeps going afterwards.
#[tokio::test]
async fn a_non_superuser_owner_can_apply_every_migration() {
    let Some(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
    else {
        eprintln!("skipping: TRACE_COMMONS_PG_TEST_DATABASE_URL or DATABASE_URL not configured");
        return;
    };
    let _turn = VIRGIN_DATABASE.lock().await;

    const OWNER: &str = "trace_migration_owner_probe";
    const OWNER_DB: &str = "trace_migration_owner_probe_db";
    let admin = connect(&with_database(&url, "postgres")).await;
    admin
        .execute(
            &format!("DROP DATABASE IF EXISTS {OWNER_DB} WITH (FORCE)"),
            &[],
        )
        .await
        .expect("drop any owner-probe database left by an earlier run");
    admin
        .batch_execute(&format!(
            "DO $$ BEGIN
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{OWNER}') THEN
                     CREATE ROLE {OWNER};
                 END IF;
             END $$;
             ALTER ROLE {OWNER} LOGIN CREATEROLE NOSUPERUSER NOBYPASSRLS PASSWORD 'probe';"
        ))
        .await
        .expect("create the non-superuser owner");
    // Roles are per server, so on a shared one the roles the migrations create
    // already exist, made by whoever migrated first. Since PostgreSQL 16
    // CREATEROLE may only administer roles it holds ADMIN on, which a real
    // operator's migrator has by having created them. Give the probe the same
    // standing; on 15 and earlier CREATEROLE already covers it. Login roles
    // too: V30 runs `ALTER ROLE trace_login_resolver SET statement_timeout`,
    // and 16 asks for ADMIN there as well -- leaving them out is how this
    // test's first CI run failed.
    let sixteen_or_later: bool = admin
        .query_one(
            "SELECT current_setting('server_version_num')::int >= 160000",
            &[],
        )
        .await
        .expect("server version")
        .get(0);
    // Listed on every version so the statement is exercised wherever this runs;
    // only the grants are version-specific.
    let existing = admin
        .query(
            "SELECT rolname FROM pg_roles
              WHERE rolname LIKE 'trace\\_%' AND NOT rolsuper AND rolname <> $1",
            &[&OWNER],
        )
        .await
        .expect("list the roles already on this server");
    if sixteen_or_later {
        for row in existing {
            let role: String = row.get(0);
            admin
                .execute(
                    &format!("GRANT \"{role}\" TO {OWNER} WITH ADMIN OPTION"),
                    &[],
                )
                .await
                .unwrap_or_else(|error| panic!("grant admin on {role}: {error}"));
        }
    }
    admin
        .execute(&format!("CREATE DATABASE {OWNER_DB} OWNER {OWNER}"), &[])
        .await
        .expect("create the owner-probe database");

    // PostgreSQL 15 stopped giving PUBLIC the CREATE privilege on `public` and
    // handed the schema to the database owner. Put every version in that
    // shape: on 14 an `ALTER FUNCTION ... OWNER TO` a role that was never
    // granted CREATE passes by way of PUBLIC, and the migration that forgot the
    // grant fails only on the servers people actually deploy.
    connect(&with_database(&url, OWNER_DB))
        .await
        .batch_execute(&format!(
            "REVOKE CREATE ON SCHEMA public FROM PUBLIC;
             ALTER SCHEMA public OWNER TO {OWNER};"
        ))
        .await
        .expect("put the public schema in its PostgreSQL 15 shape");

    let owner_url = with_user(&with_database(&url, OWNER_DB), OWNER, "probe");
    let config = DatabaseConfig {
        url: SecretString::from(owner_url),
        pool_size: 2,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: DatabaseConfig::login_resolver_url_from_env(),
        gate_driver_url: DatabaseConfig::gate_driver_url_from_env(),
        pii_backstop_driver_url: DatabaseConfig::pii_backstop_driver_url_from_env(),
        invite_registry_url: DatabaseConfig::invite_registry_url_from_env(),
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    // Debug, not Display: Display stops at "db error" and drops the server's message,
    // which is the only part that says which migration and why.
    let migrated = backend
        .run_migrations()
        .await
        .map_err(|error| format!("{error:?}"));
    drop(backend);

    let behaviour = match &migrated {
        Ok(()) => Some(tenant_is_cleared_inside_and_restored_after(&url, OWNER_DB).await),
        Err(_) => None,
    };

    let dropped = admin
        .execute(
            &format!("DROP DATABASE IF EXISTS {OWNER_DB} WITH (FORCE)"),
            &[],
        )
        .await;

    assert_eq!(
        migrated,
        Ok(()),
        "a database owner with CREATEROLE and no superuser must be able to apply every migration"
    );
    let problems = behaviour.expect("behaviour is checked whenever the migrations applied");
    assert!(
        problems.is_empty(),
        "the definer functions must run with the tenant cleared and return it untouched:\n  {}",
        problems.join("\n  ")
    );
    dropped.expect("drop the owner-probe database");
}

/// One withdrawn page and one live page for a tenant, then every call made the
/// way the server makes it: inside a transaction that already has that tenant.
/// Returns what went wrong rather than panicking, so the caller can still drop
/// its database.
async fn tenant_is_cleared_inside_and_restored_after(url: &str, database: &str) -> Vec<String> {
    const TENANT: &str = "tenant-owner-probe";
    const SETTING: &str = "trace_commons.trace_tenant_id";
    let client = connect(&with_database(url, database)).await;
    // The fixture skips the tenant, account and submission rows the foreign keys
    // want: this is about row visibility, and a superuser session in replica
    // mode is the shortest honest way to two rows.
    client
        .batch_execute(&format!(
            "SET session_replication_role = replica;
             INSERT INTO trace_public_runs (
                 tenant_id, publication_id, account_id, submission_id, slug, title,
                 outcome_summary, workflow, reuse_permission, evidence_jsonb, task_success,
                 contributed_version, approval_sha256, unpublished_at)
             SELECT '{TENANT}', gen_random_uuid(), gen_random_uuid(), gen_random_uuid(), slug,
                    'A run', 'It worked.', 'Steps.', 'cc0_1_0', '[{{\"excerpt\": \"x\"}}]'::jsonb,
                    'success', '1', 'sha256:' || repeat('0', 64), withdrawn
               FROM (VALUES ('withdrawn-run', NOW()), ('live-run', NULL)) AS run(slug, withdrawn);
             SET session_replication_role = origin;"
        ))
        .await
        .expect("insert the two fixture pages");

    let mut problems = Vec::new();
    // (what it is, a query returning one bigint, the count it must return)
    let calls: [(&str, &str, i64); 5] = [
        (
            "trace_public_run_page on a withdrawn page of the caller's own tenant",
            "SELECT count(*) FROM trace_public_run_page('withdrawn-run', 5)",
            0,
        ),
        (
            "trace_public_run_page on a live page",
            "SELECT count(*) FROM trace_public_run_page('live-run', 5)",
            1,
        ),
        (
            "trace_resolve_public_run_source on a withdrawn page of the caller's own tenant",
            "SELECT count(*) FROM trace_resolve_public_run_source('withdrawn-run')",
            0,
        ),
        (
            "trace_public_run_retained_source for an owner with no page",
            "SELECT count(*) FROM trace_public_run_retained_source(
                 'tenant-owner-probe', gen_random_uuid(), gen_random_uuid())",
            0,
        ),
        (
            "trace_reward_mission_list on an empty catalogue",
            "SELECT count(*) FROM (SELECT public.trace_reward_mission_list(NULL, 10)) AS listed",
            1,
        ),
    ];
    for (what, query, expected) in calls {
        client
            .batch_execute(&format!(
                "BEGIN; SELECT set_config('{SETTING}', '{TENANT}', true);"
            ))
            .await
            .expect("open a tenant-scoped transaction");
        match client.query_one(query, &[]).await {
            Ok(row) => {
                let got: i64 = row.get(0);
                if got != expected {
                    problems.push(format!("{what}: returned {got} rows, expected {expected}"));
                }
            }
            Err(error) => problems.push(format!("{what}: {error}")),
        }
        match client
            .query_one(&format!("SELECT current_setting('{SETTING}', true)"), &[])
            .await
        {
            Ok(row) => {
                let after: Option<String> = row.get(0);
                if after.as_deref() != Some(TENANT) {
                    problems.push(format!(
                        "{what}: the caller's tenant came back as {after:?}, not {TENANT:?}"
                    ));
                }
            }
            Err(error) => problems.push(format!("{what}: reading the tenant back: {error}")),
        }
        let _ = client.batch_execute("ROLLBACK").await;
    }
    problems
}
