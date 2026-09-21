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
        Ok(()) => Some((
            tenant_is_cleared_inside_and_restored_after(&url, OWNER_DB).await,
            public_run_acl_is_closed_and_the_trigger_needs_no_table_grant(&url, OWNER_DB).await,
        )),
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
    let (tenant_problems, acl_problems) =
        behaviour.expect("behaviour is checked whenever the migrations applied");
    assert!(
        tenant_problems.is_empty(),
        "the definer functions must run with the tenant cleared and return it untouched:\n  {}",
        tenant_problems.join("\n  ")
    );
    assert!(
        acl_problems.is_empty(),
        "the public-run functions must be closed to PUBLIC and open to trace_public_run_runtime, \
         and the unpublish trigger must work for a login with no grant on trace_public_runs:\n  {}",
        acl_problems.join("\n  ")
    );
    dropped.expect("drop the owner-probe database");
}

/// The ACL a non-superuser migrator leaves behind on the public-run functions,
/// and whether the unpublish trigger works for the login the docs describe.
///
/// V64 revoked PUBLIC's EXECUTE on its four definer functions and granted its
/// own after it had left the roles that own them. A non-superuser migrator may
/// not do that, and PostgreSQL warns rather than fails, so on the pilot PUBLIC
/// kept EXECUTE and the migrator got none. V64's trigger function also ran as
/// the caller, so a runtime login with no grant on `trace_public_runs` could not
/// move a submission away from `accepted`. V74 repairs both; this is the check
/// that says whether it did, as three probe roles that hold nothing else:
///
/// - one that holds nothing at all, which must not be able to execute any of
///   the four functions (that is PUBLIC's EXECUTE, or its absence);
/// - one that is a member of `trace_public_run_runtime`, which must;
/// - an ingest-shaped login with DML on the submission tables and nothing on
///   `trace_public_runs`, which must be able to revoke an accepted submission
///   and thereby unpublish its page, and only its own tenant's page.
///
/// Returns what went wrong rather than panicking, so the caller can still drop
/// its database.
async fn public_run_acl_is_closed_and_the_trigger_needs_no_table_grant(
    url: &str,
    database: &str,
) -> Vec<String> {
    const NOBODY: &str = "trace_public_run_probe_nobody";
    const RUNTIME: &str = "trace_public_run_probe_runtime";
    const INGEST: &str = "trace_public_run_probe_ingest";
    const FUNCTIONS: [&str; 4] = [
        "trace_public_run_page(text, integer)",
        "trace_resolve_public_run_source(text)",
        "trace_public_run_would_cycle(uuid, uuid)",
        "trace_public_run_retained_source(text, uuid, uuid)",
    ];
    let mut problems = Vec::new();
    let admin = connect(&with_database(url, database)).await;

    // Roles are per server; the probe roles from an interrupted run may exist.
    for role in [NOBODY, RUNTIME, INGEST] {
        admin
            .batch_execute(&format!(
                "DO $$ BEGIN
                     IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{role}') THEN
                         DROP OWNED BY {role};
                         DROP ROLE {role};
                     END IF;
                 END $$;"
            ))
            .await
            .expect("drop a probe role left by an earlier run");
    }
    admin
        .batch_execute(&format!(
            "CREATE ROLE {NOBODY} NOLOGIN NOSUPERUSER NOBYPASSRLS NOINHERIT;
             CREATE ROLE {RUNTIME} LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD 'probe';
             CREATE ROLE {INGEST} LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD 'probe';
             GRANT USAGE ON SCHEMA public TO {RUNTIME}, {INGEST};
             GRANT SELECT, INSERT, UPDATE ON trace_tenants, trace_accounts, trace_submissions,
                 trace_token_bundles TO {INGEST};"
        ))
        .await
        .expect("create the probe roles");
    if let Err(error) = admin
        .batch_execute(&format!("GRANT trace_public_run_runtime TO {RUNTIME}"))
        .await
    {
        problems.push(format!("granting trace_public_run_runtime: {error:?}"));
    }

    for function in FUNCTIONS {
        for (role, expected) in [(NOBODY, false), (RUNTIME, true)] {
            match admin
                .query_one(
                    "SELECT has_function_privilege($1, $2, 'EXECUTE')",
                    &[&role, &function],
                )
                .await
            {
                Ok(row) => {
                    let got: bool = row.get(0);
                    if got != expected {
                        problems.push(format!(
                            "{role} {} execute {function}",
                            if got { "may" } else { "may not" }
                        ));
                    }
                }
                Err(error) => {
                    problems.push(format!("has_function_privilege for {function}: {error:?}"))
                }
            }
        }
    }

    // Two tenants, each with an accepted submission and a live page. Replica
    // mode is the shortest honest way past the foreign keys and the forced RLS
    // policy for a superuser fixture; the update under test runs without it.
    admin
        .batch_execute(
            "SET session_replication_role = replica;
             INSERT INTO trace_tenants (tenant_id) VALUES ('tenant-probe-a'), ('tenant-probe-b');
             INSERT INTO trace_accounts (tenant_id, account_id) VALUES
                 ('tenant-probe-a', '00000000-0000-4000-8000-00000000000a'),
                 ('tenant-probe-b', '00000000-0000-4000-8000-00000000000b');
             INSERT INTO trace_submissions (
                 tenant_id, submission_id, trace_id, auth_principal_ref, schema_version,
                 consent_policy_version, retention_policy_id, status, privacy_risk,
                 redaction_pipeline_version, redaction_hash)
             VALUES
                 ('tenant-probe-a', '00000000-0000-4000-8000-0000000000a1', gen_random_uuid(),
                  'sha256:' || repeat('a', 64), '1', '1', 'default', 'accepted', 'low', '1',
                  'sha256:' || repeat('1', 64)),
                 ('tenant-probe-b', '00000000-0000-4000-8000-0000000000b1', gen_random_uuid(),
                  'sha256:' || repeat('b', 64), '1', '1', 'default', 'accepted', 'low', '1',
                  'sha256:' || repeat('2', 64));
             INSERT INTO trace_public_runs (
                 tenant_id, publication_id, account_id, submission_id, slug, title,
                 outcome_summary, workflow, reuse_permission, evidence_jsonb, task_success,
                 contributed_version, approval_sha256)
             VALUES
                 ('tenant-probe-a', gen_random_uuid(), '00000000-0000-4000-8000-00000000000a',
                  '00000000-0000-4000-8000-0000000000a1', 'probe-run-a', 'A run', 'It worked.',
                  'Steps.', 'cc0_1_0', '[{\"excerpt\": \"x\"}]'::jsonb, 'success', '1',
                  'sha256:' || repeat('0', 64)),
                 ('tenant-probe-b', gen_random_uuid(), '00000000-0000-4000-8000-00000000000b',
                  '00000000-0000-4000-8000-0000000000b1', 'probe-run-b', 'B run', 'It worked.',
                  'Steps.', 'cc0_1_0', '[{\"excerpt\": \"x\"}]'::jsonb, 'success', '1',
                  'sha256:' || repeat('0', 64));
             SET session_replication_role = origin;",
        )
        .await
        .expect("insert the two tenants' fixtures");

    // The runtime member calls a page the way the server does, with a tenant
    // set: EXECUTE on paper is not the same as a call that returns.
    let runtime = connect(&with_user(&with_database(url, database), RUNTIME, "probe")).await;
    match runtime
        .query_one(
            "SELECT count(*) FROM trace_public_run_page('probe-run-b', 5)",
            &[],
        )
        .await
    {
        Ok(row) => {
            let got: i64 = row.get(0);
            if got != 1 {
                problems.push(format!(
                    "the runtime member's trace_public_run_page returned {got} rows, expected 1"
                ));
            }
        }
        Err(error) => problems.push(format!(
            "the runtime member calling trace_public_run_page: {error:?}"
        )),
    }

    let ingest = connect(&with_user(&with_database(url, database), INGEST, "probe")).await;
    if let Err(error) = ingest
        .batch_execute(
            "BEGIN;
             SELECT set_config('trace_commons.trace_tenant_id', 'tenant-probe-a', true);
             UPDATE trace_submissions SET status = 'revoked'
              WHERE submission_id = '00000000-0000-4000-8000-0000000000a1';
             COMMIT;",
        )
        .await
    {
        problems.push(format!(
            "the ingest login revoking an accepted submission with no grant on trace_public_runs: {error:?}"
        ));
        let _ = ingest.batch_execute("ROLLBACK").await;
    }
    match admin
        .query(
            "SELECT slug, unpublished_at IS NOT NULL, version FROM trace_public_runs
              WHERE slug LIKE 'probe-run-%' ORDER BY slug",
            &[],
        )
        .await
    {
        Ok(rows) => {
            let state: Vec<(String, bool, i32)> = rows
                .iter()
                .map(|row| (row.get(0), row.get(1), row.get(2)))
                .collect();
            let expected = vec![
                ("probe-run-a".to_string(), true, 2),
                ("probe-run-b".to_string(), false, 1),
            ];
            if state != expected {
                problems.push(format!(
                    "after revoking tenant-probe-a's submission the pages read {state:?}, expected {expected:?}"
                ));
            }
        }
        Err(error) => problems.push(format!("reading the pages back: {error:?}")),
    }

    drop(runtime);
    drop(ingest);
    for role in [NOBODY, RUNTIME, INGEST] {
        admin
            .batch_execute(&format!("DROP OWNED BY {role}; DROP ROLE {role};"))
            .await
            .expect("drop the probe role");
    }
    problems
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
            Err(error) => problems.push(format!("{what}: {error:?}")),
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
            Err(error) => problems.push(format!("{what}: reading the tenant back: {error:?}")),
        }
        let _ = client.batch_execute("ROLLBACK").await;
    }
    problems
}
