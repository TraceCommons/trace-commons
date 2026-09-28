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
    Database,
    postgres::PgBackend,
    postgres::{apply_and_record_migration, registered_migrations},
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

/// The membership a migrator holds, since PostgreSQL 16, in a role that already
/// exists on the server. A superuser's grant is recorded against the bootstrap
/// superuser, whichever superuser makes it, so it writes the same
/// `pg_auth_members` row that creating the role does, and granting it again
/// resets that row's options to these.
#[derive(Clone, Copy, Debug)]
enum Standing {
    /// What `CREATE ROLE` gives the CREATEROLE role that runs it: ADMIN, and
    /// neither INHERIT nor SET, since `createrole_self_grant` is empty by
    /// default. A migrator on a fresh server holds exactly this in every role
    /// the migrations make.
    Creator,
    /// What a superuser's plain `GRANT ... WITH ADMIN OPTION` gives a migrator
    /// on a server where someone else made the roles: INHERIT and SET follow
    /// the member's defaults, which are true.
    GrantedWithInherit,
}

impl Standing {
    fn grant_options(self) -> &'static str {
        match self {
            Standing::Creator => "WITH ADMIN TRUE, INHERIT FALSE, SET FALSE",
            Standing::GrantedWithInherit => "WITH ADMIN TRUE, INHERIT TRUE, SET TRUE",
        }
    }
}

/// A database owned by `owner`, a login with CREATEROLE and nothing more, in
/// the PostgreSQL 15 shape, the way an operator's migrator holds one, and
/// holding `standing` in every `trace_` role already on the server. Drops any
/// copy an earlier run left behind. Returns a superuser connection to the
/// `postgres` database, for dropping it again afterwards.
async fn owned_database(
    url: &str,
    owner: &str,
    database: &str,
    standing: Standing,
) -> tokio_postgres::Client {
    let admin = connect(&with_database(url, "postgres")).await;
    admin
        .execute(
            &format!("DROP DATABASE IF EXISTS {database} WITH (FORCE)"),
            &[],
        )
        .await
        .expect("drop any owner-probe database left by an earlier run");
    admin
        .batch_execute(&format!(
            "DO $$ BEGIN
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{owner}') THEN
                     CREATE ROLE {owner};
                 END IF;
             END $$;
             ALTER ROLE {owner} LOGIN CREATEROLE NOSUPERUSER NOBYPASSRLS PASSWORD 'probe';"
        ))
        .await
        .expect("create the non-superuser owner");
    // Roles are per server, so on a shared one the roles the migrations create
    // already exist, made by whoever migrated first. Since PostgreSQL 16
    // CREATEROLE may only administer roles it holds ADMIN on, which a real
    // operator's migrator has by having created them. Give the probe the
    // standing asked for; on 15 and earlier CREATEROLE already covers it, and
    // creating a role grants its creator nothing. Login roles too: V30 runs
    // `ALTER ROLE trace_login_resolver SET statement_timeout`, and 16 asks for
    // ADMIN there as well -- leaving them out is how this test's first CI run
    // failed.
    //
    // The standing is not a detail. This used to grant `WITH ADMIN OPTION`
    // alone, which takes INHERIT from the member's default, so a probe on a
    // server where another test had made the roles held more than a creator
    // does -- and V60 failed only for a probe that made them itself.
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
            &[&owner],
        )
        .await
        .expect("list the roles already on this server");
    if sixteen_or_later {
        for row in existing {
            let role: String = row.get(0);
            admin
                .execute(
                    &format!("GRANT \"{role}\" TO {owner} {}", standing.grant_options()),
                    &[],
                )
                .await
                .unwrap_or_else(|error| panic!("grant admin on {role}: {error}"));
        }
    }
    admin
        .execute(&format!("CREATE DATABASE {database} OWNER {owner}"), &[])
        .await
        .expect("create the owner-probe database");

    // PostgreSQL 15 stopped giving PUBLIC the CREATE privilege on `public` and
    // handed the schema to the database owner. Put every version in that
    // shape: on 14 an `ALTER FUNCTION ... OWNER TO` a role that was never
    // granted CREATE passes by way of PUBLIC, and the migration that forgot the
    // grant fails only on the servers people actually deploy.
    connect(&with_database(url, database))
        .await
        .batch_execute(&format!(
            "REVOKE CREATE ON SCHEMA public FROM PUBLIC;
             ALTER SCHEMA public OWNER TO {owner};"
        ))
        .await
        .expect("put the public schema in its PostgreSQL 15 shape");

    admin
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
    let admin = owned_database(&url, OWNER, OWNER_DB, Standing::Creator).await;

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

/// V60 on a fresh PostgreSQL 16 server, whichever test runs first.
///
/// Since 16, `CREATE ROLE` run by a CREATEROLE role that is not a superuser
/// also grants the new role to its creator: ADMIN, and neither INHERIT nor SET.
/// V59's `REVOKE trace_admission_guard FROM CURRENT_USER` removes the grant V59
/// made, not that one. V60 tested `pg_has_role(current_user,
/// 'trace_admission_guard', 'MEMBER')`, which counts it, so on a fresh server
/// V60 took the branch meant for deployments of the pre-revocation V59 and
/// granted the migrator the EXECUTE grant option V59 had just given it. The
/// migrator does not hold the owner's privileges, so PostgreSQL made it the
/// grantor of that grant to itself, and refused it as circular: 0LP01, "grant
/// options cannot be granted back to your own grantor".
///
/// `a_non_superuser_owner_can_apply_every_migration` missed it whenever another
/// test had made the roles first, because the probe was then given them with
/// INHERIT. This sets the probe's standing in `trace_admission_guard` itself,
/// for both standings a non-superuser migrator can hold there, and checks the
/// standing took just before V60. Each must apply V1 to V60, must end with the
/// migrator holding EXECUTE WITH GRANT OPTION on both admission functions in an
/// ACL entry of its own, and must leave the standing as V60 found it.
#[tokio::test]
async fn v60_applies_whether_the_migrator_made_the_admission_guard_or_was_granted_it() {
    let Some(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
    else {
        eprintln!("skipping: TRACE_COMMONS_PG_TEST_DATABASE_URL or DATABASE_URL not configured");
        return;
    };
    let _turn = VIRGIN_DATABASE.lock().await;

    // Not `trace_`: `owned_database` gives every `trace_` role on the server to
    // the probe it sets up, and two probes given to each other are a cycle
    // PostgreSQL refuses.
    const OWNER: &str = "tc_v60_owner_probe";
    const DB: &str = "trace_v60_owner_probe_db";

    let server = connect(&with_database(&url, "postgres")).await;
    let sixteen_or_later: bool = server
        .query_one(
            "SELECT current_setting('server_version_num')::int >= 160000",
            &[],
        )
        .await
        .expect("server version")
        .get(0);
    if !sixteen_or_later {
        eprintln!(
            "skipping: before PostgreSQL 16 creating a role grants its creator nothing, \
             and a membership has no INHERIT or SET of its own to set"
        );
        return;
    }
    // Made here, with V59's attributes, when nothing on this server has made it
    // yet, so that `owned_database` sets the probe's standing in it before V1.
    // V59 creates it only when it is missing.
    server
        .batch_execute(
            "DO $$ BEGIN
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_admission_guard') THEN
                     CREATE ROLE trace_admission_guard NOLOGIN NOBYPASSRLS;
                 END IF;
             END $$;",
        )
        .await
        .expect("make trace_admission_guard");
    drop(server);

    /// (MEMBER, USAGE, SET) for the connected login in `trace_admission_guard`.
    async fn held(client: &tokio_postgres::Client) -> (bool, bool, bool) {
        let row = client
            .query_one(
                "SELECT pg_has_role(current_user, 'trace_admission_guard', 'MEMBER'),
                        pg_has_role(current_user, 'trace_admission_guard', 'USAGE'),
                        pg_has_role(current_user, 'trace_admission_guard', 'SET')",
                &[],
            )
            .await
            .expect("read the probe's standing in trace_admission_guard");
        (row.get(0), row.get(1), row.get(2))
    }

    let mut problems: Vec<String> = Vec::new();
    for (standing, expected) in [
        (Standing::Creator, (true, false, false)),
        (Standing::GrantedWithInherit, (true, true, true)),
    ] {
        let admin = owned_database(&url, OWNER, DB, standing).await;
        let mut owner = connect(&with_user(&with_database(&url, DB), OWNER, "probe")).await;
        owner
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS _trace_commons_migrations (
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                );",
            )
            .await
            .expect("create the recording table");

        let mut outcome: Result<(), String> = Err("V60 is not wired into the runner".into());
        for (version, name, sql) in registered_migrations()
            .iter()
            .filter(|(version, ..)| *version <= 60)
        {
            if *version == 60 {
                let before = held(&owner).await;
                if before != expected {
                    outcome = Err(format!(
                        "held (MEMBER, USAGE, SET) = {before:?} in trace_admission_guard \
                         before V60, not {expected:?}, so V60 was not tried with this standing"
                    ));
                    break;
                }
            }
            if let Err(error) = apply_and_record_migration(&mut owner, *version, name, sql).await {
                outcome = Err(format!("V{version} failed: {error:?}"));
                break;
            }
            if *version == 60 {
                outcome = Ok(());
            }
        }

        if outcome.is_ok() {
            let own_grant_options: i64 = owner
                .query_one(
                    "SELECT count(*) FROM unnest(ARRAY[
                         'trace_reserve_admission(TEXT,TEXT,UUID,TEXT,TEXT,TEXT,BIGINT,BIGINT,BIGINT,BIGINT,UUID,BIGINT)',
                         'trace_transition_admission(TEXT,UUID,UUID,TEXT)'
                     ]::regprocedure[]) AS f(oid)
                     WHERE EXISTS (
                         SELECT 1 FROM pg_proc p, aclexplode(p.proacl) a
                          WHERE p.oid = f.oid
                            AND a.grantee = (SELECT oid FROM pg_roles WHERE rolname = current_user)
                            AND a.privilege_type = 'EXECUTE' AND a.is_grantable)",
                    &[],
                )
                .await
                .expect("read the admission functions' ACL")
                .get(0);
            let after = held(&owner).await;
            if own_grant_options != 2 {
                outcome = Err(format!(
                    "the migrator holds EXECUTE WITH GRANT OPTION in an ACL entry of its own \
                     on {own_grant_options} of the 2 admission functions"
                ));
            } else if after != expected {
                outcome = Err(format!(
                    "V59 and V60 left (MEMBER, USAGE, SET) = {after:?} in \
                     trace_admission_guard, not the {expected:?} they found"
                ));
            }
        }

        drop(owner);
        admin
            .execute(&format!("DROP DATABASE IF EXISTS {DB} WITH (FORCE)"), &[])
            .await
            .expect("drop the probe database");
        if let Err(problem) = outcome {
            problems.push(format!("{standing:?}: {problem}"));
        }
    }

    assert!(
        problems.is_empty(),
        "a CREATEROLE migrator must apply V60 whichever way it holds trace_admission_guard:\n  {}",
        problems.join("\n  ")
    );
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

/// The pilot's hand-made runtime grants, taken once when the schema was at V62
/// (from the cutover rehearsal harness). `pg_default_acl` is empty there, so
/// every table a later migration creates is invisible to the group until
/// something grants on it.
const PILOT_V62_RUNTIME_GRANTS: &str = "
    GRANT USAGE ON SCHEMA public TO trace_ingest_runtime;
    GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO trace_ingest_runtime;
    REVOKE ALL ON trace_admission_receipts, trace_admission_global_budget FROM trace_ingest_runtime;
    GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO trace_ingest_runtime;
    GRANT EXECUTE ON FUNCTION
        trace_reserve_admission(TEXT,TEXT,UUID,TEXT,TEXT,TEXT,BIGINT,BIGINT,BIGINT,BIGINT,UUID,BIGINT),
        trace_transition_admission(TEXT,UUID,UUID,TEXT)
        TO trace_ingest_runtime;
    GRANT EXECUTE ON FUNCTION trace_prune_onboarding_expiry(TEXT,INTEGER,BOOLEAN)
        TO trace_ingest_runtime;
    GRANT CREATE ON SCHEMA public TO trace_ingest_runtime;";

/// The migration under test: the first one that grants to `trace_ingest_runtime`.
const RUNTIME_GRANTS_VERSION: i32 = 90;

fn v90_submission(
    tenant: &str,
    submission_id: uuid::Uuid,
    principal: &str,
) -> trace_commons_server::trace_corpus_storage::TraceSubmissionWrite {
    use trace_commons_server::trace_corpus_storage::{TraceCorpusStatus, TraceSubmissionWrite};
    TraceSubmissionWrite {
        tenant_id: tenant.into(),
        submission_id,
        trace_id: uuid::Uuid::new_v4(),
        auth_principal_ref: principal.into(),
        contributor_pseudonym: None,
        submitted_tenant_scope_ref: None,
        schema_version: "ironclaw.trace_contribution.v1".into(),
        consent_policy_version: "2026-04-24".into(),
        consent_scopes: vec!["debugging".into()],
        allowed_uses: vec!["debugging".into()],
        retention_policy_id: "standard".into(),
        status: TraceCorpusStatus::Accepted,
        privacy_risk: "low".into(),
        redaction_pipeline_version: "deterministic-v1".into(),
        redaction_counts: std::collections::BTreeMap::new(),
        redaction_hash: "sha256:v90-probe".into(),
        canonical_summary_hash: None,
        submission_score: None,
        credit_points_pending: None,
        credit_points_final: None,
        expires_at: None,
        residual_risk_basis: None,
    }
}

fn pg_backend(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 2,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

/// `Err` rendered with the server's message, which is what names the table.
fn denial<T: std::fmt::Debug>(
    result: Result<T, trace_commons_server::error::DatabaseError>,
) -> String {
    match result {
        Ok(value) => format!("succeeded: {value:?}"),
        Err(error) => format!("{error:?}"),
    }
}

/// V90 against the database the pilot actually has, not the one CI has.
///
/// The pilot's ingest login holds its privileges through a hand-made group,
/// `trace_ingest_runtime`, granted table-wide once at V62. Nothing migrated
/// since then granted it anything, so on the cutover build every submission
/// (`FOR UPDATE` on the V78 source-session row), every idempotent re-POST (the
/// V76 witness-evidence read) and every withdrawal (V78, plus the V65-V68
/// token-bundle trigger) failed with `permission denied`, and the table-wide
/// `UPDATE ON trace_accounts` it was given failed account admission's readiness
/// check. CI never sees any of it, because CI migrates and serves as one
/// superuser.
///
/// So: a non-superuser CREATEROLE owner migrates to V62, the pilot's grants are
/// applied, the owner migrates to just below V90, and the store methods those
/// routes call run as a login in that group -- first to show each one refused,
/// then, once the real runner has applied V90, to show each one working.
#[tokio::test]
async fn v90_gives_a_pilot_shaped_runtime_group_what_submit_repost_and_withdraw_need() {
    use trace_commons_protocol::token_distribution::{
        ContentDigest, ContributionBundleManifest, TokenUsageProfile,
    };
    use trace_commons_server::token_bundle_store::{StoredTokenBundle, StoredTokenObject};
    use trace_commons_server::trace_artifact_store::{
        TraceArtifactKind, TraceArtifactObjectRef, TraceArtifactProviderKind,
    };
    use trace_commons_server::trace_corpus_storage::{
        TraceCorpusStatus, TraceCorpusStore, TraceSourceSessionStatus,
    };

    let Some(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
    else {
        eprintln!("skipping: TRACE_COMMONS_PG_TEST_DATABASE_URL or DATABASE_URL not configured");
        return;
    };
    let _turn = VIRGIN_DATABASE.lock().await;

    const OWNER: &str = "trace_v90_owner_probe";
    const DB: &str = "trace_v90_runtime_probe_db";
    // Released clients: ingest with account admission off.
    const INGEST: &str = "trace_v90_probe_ingest";
    // The same login once account admission is switched on.
    const ADMISSION: &str = "trace_v90_probe_admission";

    // The group exists before any of this, made by hand, as on the pilot.
    // Roles are per server: strip whatever another database's V90 (or an
    // interrupted run) already gave these roles, so the "before" half sees
    // only what the pilot has.
    let server = connect(&with_database(&url, "postgres")).await;
    server
        .batch_execute(&format!(
            "DO $$ BEGIN
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
                     CREATE ROLE trace_ingest_runtime NOLOGIN NOSUPERUSER NOBYPASSRLS;
                 END IF;
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{INGEST}') THEN
                     CREATE ROLE {INGEST} LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD 'probe';
                 END IF;
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{ADMISSION}') THEN
                     CREATE ROLE {ADMISSION} LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD 'probe';
                 END IF;
             END $$;"
        ))
        .await
        .expect("create the runtime group and its logins");
    let sixteen_or_later: bool = server
        .query_one(
            "SELECT current_setting('server_version_num')::int >= 160000",
            &[],
        )
        .await
        .expect("server version")
        .get(0);
    let memberships = server
        .query(
            "SELECT g.rolname, m.rolname, gr.rolname
               FROM pg_auth_members a
               JOIN pg_roles g ON g.oid = a.roleid
               JOIN pg_roles m ON m.oid = a.member
               JOIN pg_roles gr ON gr.oid = a.grantor
              WHERE m.rolname IN ('trace_ingest_runtime', $1, $2)",
            &[&INGEST, &ADMISSION],
        )
        .await
        .expect("list the memberships left on this server");
    for row in memberships {
        let (granted, member, grantor): (String, String, String) =
            (row.get(0), row.get(1), row.get(2));
        // Before 16 a membership is one row whoever granted it; since 16 each
        // grantor's grant is its own row, and a superuser's REVOKE removes only
        // its own unless told whose.
        let revoke = if sixteen_or_later {
            format!("REVOKE \"{granted}\" FROM \"{member}\" GRANTED BY \"{grantor}\"")
        } else {
            format!("REVOKE \"{granted}\" FROM \"{member}\"")
        };
        server
            .batch_execute(&revoke)
            .await
            .unwrap_or_else(|error| panic!("{revoke}: {error}"));
    }
    server
        .batch_execute(&format!(
            "GRANT trace_ingest_runtime TO {INGEST}, {ADMISSION};"
        ))
        .await
        .expect("put both logins in the group");
    drop(server);

    let admin = owned_database(&url, OWNER, DB, Standing::Creator).await;
    let owner_url = with_user(&with_database(&url, DB), OWNER, "probe");
    let mut owner = connect(&owner_url).await;
    owner
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS _trace_commons_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );",
        )
        .await
        .expect("create the recording table");
    let migrations = registered_migrations();
    for (version, name, sql) in migrations.iter().filter(|(version, ..)| *version <= 62) {
        apply_and_record_migration(&mut owner, *version, name, sql)
            .await
            .unwrap_or_else(|error| panic!("apply V{version}: {error:?}"));
    }
    owner
        .batch_execute(PILOT_V62_RUNTIME_GRANTS)
        .await
        .expect("the pilot's V62-era runtime grants");
    for (version, name, sql) in migrations
        .iter()
        .filter(|(version, ..)| *version > 62 && *version < RUNTIME_GRANTS_VERSION)
    {
        apply_and_record_migration(&mut owner, *version, name, sql)
            .await
            .unwrap_or_else(|error| panic!("apply V{version}: {error:?}"));
    }
    // The one runtime grant the pilot took at V74, per deployment.md.
    owner
        .batch_execute("GRANT trace_public_run_runtime TO trace_ingest_runtime;")
        .await
        .expect("the pilot's V74 runtime grant");

    let superuser = PgBackend::new(&pg_backend(with_database(&url, DB)))
        .await
        .expect("superuser backend");
    let ingest = PgBackend::new(&pg_backend(with_user(
        &with_database(&url, DB),
        INGEST,
        "probe",
    )))
    .await
    .expect("ingest backend");
    let admission = PgBackend::new(&pg_backend(with_user(
        &with_database(&url, DB),
        ADMISSION,
        "probe",
    )))
    .await
    .expect("admission backend");
    let ingest_raw = connect(&with_user(&with_database(&url, DB), INGEST, "probe")).await;

    // Switching account admission on grants its role to the ingest login. The
    // readiness check is asked before any account exists, so linkage cannot
    // be what refuses it.
    owner
        .batch_execute(&format!(
            "GRANT trace_account_admission_runtime TO {ADMISSION};"
        ))
        .await
        .expect("grant the admission role");
    let readiness_before = admission.account_admission_runtime_ready().await;

    // Fixtures, as the superuser: one account with one device principal, an
    // accepted submission with a staged token bundle, and nothing else.
    let tenant = format!("tenant-v90-{}", uuid::Uuid::new_v4().simple());
    let account = uuid::Uuid::new_v4();
    let principal = format!("principal:v90:{}", uuid::Uuid::new_v4().simple());
    let existing = uuid::Uuid::new_v4();
    let fresh = uuid::Uuid::new_v4();
    let claimed = uuid::Uuid::new_v4();
    let digest = [0x90u8; 32];
    let db_client = connect(&with_database(&url, DB)).await;
    db_client
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&tenant],
        )
        .await
        .expect("tenant");
    db_client
        .execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant, &account],
        )
        .await
        .expect("account");
    db_client
        .execute(
            "INSERT INTO trace_account_principals (tenant_id, account_id, principal_ref)
             VALUES ($1, $2, $3)",
            &[&tenant, &account, &principal],
        )
        .await
        .expect("principal");
    superuser
        .upsert_trace_submission(v90_submission(&tenant, existing, &principal))
        .await
        .expect("an accepted submission");
    let revision = "v90-revision".to_string();
    superuser
        .begin_token_bundle(StoredTokenBundle {
            tenant_id: tenant.clone(),
            submission_id: existing,
            revision: revision.clone(),
            owner_ref: principal.clone(),
            manifest: ContributionBundleManifest {
                version: 1,
                usage_profile: TokenUsageProfile::RestrictedResearch,
                submission_id: existing.to_string(),
                bundle_revision: revision.clone(),
                envelope_digest: ContentDigest::of(b"envelope"),
                consent_digest: ContentDigest::of(b"consent"),
                policy_version: "policy".into(),
                attachments: Vec::new(),
            },
            witness_headers: std::collections::BTreeMap::new(),
            state: "staging".into(),
            processing_state: "pending".into(),
            processing_summary: None,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            receipt: None,
            attachments: Vec::new(),
        })
        .await
        .expect("a token bundle");
    superuser
        .stage_token_object(
            &tenant,
            existing,
            &revision,
            &principal,
            StoredTokenObject {
                artifact_id: "envelope".into(),
                object_ref: TraceArtifactObjectRef {
                    provider_kind: TraceArtifactProviderKind::LocalEncrypted,
                    object_store: "v90-probe".into(),
                    tenant_storage_ref: "tenant-ref".into(),
                    submission_storage_ref: "submission-ref".into(),
                    artifact_kind: TraceArtifactKind::TokenDistribution,
                    object_key: "v90/probe".into(),
                    ciphertext_sha256: "0".repeat(64),
                },
                deleted: false,
                ready: false,
                prepared: Some(vec![1]),
            },
        )
        .await
        .expect("a staged token object");

    // What the pilot's runtime is refused, one route at a time.
    let mut before = Vec::new();
    before.push((
        "submit: a new submission",
        "permission denied for table trace_submission_sessions",
        denial(
            ingest
                .upsert_trace_submission(v90_submission(&tenant, fresh, &principal))
                .await,
        ),
    ));
    before.push((
        "re-POST: the witness-evidence retry read",
        "permission denied for table trace_witness_certificate_evidence",
        denial(
            ingest
                .witness_retry_identity_matches(&tenant, existing, None, None, b"body")
                .await,
        ),
    ));
    before.push((
        "withdraw: the source-session mapping",
        "permission denied for table trace_submission_sessions",
        denial(
            ingest
                .withdraw_trace_source_session(&tenant, account, existing, chrono::Utc::now())
                .await,
        ),
    ));
    // The raw statement, so the token trigger is what refuses it rather than
    // the source-session lock the store method takes first.
    before.push((
        "withdraw: revoking fires the token-bundle trigger",
        "permission denied for table trace_token_bundles",
        denial(
            ingest_raw
                .batch_execute(&format!(
                    "BEGIN;
                     SELECT set_config('trace_commons.trace_tenant_id', '{tenant}', true);
                     UPDATE trace_submissions SET status = 'revoked'
                      WHERE tenant_id = '{tenant}' AND submission_id = '{existing}';
                     COMMIT;"
                ))
                .await
                .map_err(trace_commons_server::error::DatabaseError::Postgres),
        ),
    ));
    let _ = ingest_raw.batch_execute("ROLLBACK").await;
    before.push((
        "withdraw: the pending token-bundle deletions",
        "permission denied for table trace_token_bundles",
        denial(
            ingest
                .pending_token_bundle_deletions(&tenant, Some(existing))
                .await
                .map(|pending| pending.len()),
        ),
    ));
    before.push((
        "account admission: claiming a source session",
        "permission denied for table trace_source_sessions",
        denial(
            admission
                .claim_trace_source_session(&tenant, account, &digest, claimed)
                .await,
        ),
    ));
    let account_id_writable_before: bool = db_client
        .query_one(
            "SELECT has_column_privilege($1, 'trace_accounts', 'account_id', 'UPDATE')",
            &[&ADMISSION],
        )
        .await
        .expect("account_id privilege")
        .get(0);

    // The real runner applies V90, as the owner.
    let owner_backend = PgBackend::new(&pg_backend(owner_url.clone()))
        .await
        .expect("owner backend");
    let migrated = owner_backend
        .run_migrations()
        .await
        .map_err(|error| format!("{error:?}"));

    let mut after = Vec::new();
    after.push((
        "submit: a new submission",
        denial(
            ingest
                .upsert_trace_submission(v90_submission(&tenant, fresh, &principal))
                .await
                .map(|record| record.status),
        ),
    ));
    after.push((
        "re-POST: the witness-evidence retry read",
        denial(
            ingest
                .witness_retry_identity_matches(&tenant, existing, None, None, b"body")
                .await,
        ),
    ));
    after.push((
        "withdraw: the source-session mapping",
        denial(
            ingest
                .withdraw_trace_source_session(&tenant, account, existing, chrono::Utc::now())
                .await
                .map(|mapped| mapped.is_some()),
        ),
    ));
    after.push((
        "withdraw: revoking through the store, which fires the trigger",
        denial(
            ingest
                .update_trace_submission_status(
                    &tenant,
                    existing,
                    TraceCorpusStatus::Revoked,
                    &principal,
                    Some("withdrawn"),
                )
                .await,
        ),
    ));
    after.push((
        "withdraw: the pending token-bundle deletions",
        denial(
            ingest
                .pending_token_bundle_deletions(&tenant, Some(existing))
                .await
                .map(|pending| {
                    pending
                        .iter()
                        .map(|bundle| (bundle.state.clone(), bundle.attachments.len()))
                        .collect::<Vec<_>>()
                }),
        ),
    ));
    after.push((
        "withdraw: marking the token object deleted",
        denial(
            ingest
                .mark_token_object_deleted(&tenant, existing, &revision, "envelope")
                .await,
        ),
    ));
    after.push((
        "withdraw: nothing left pending once the object is deleted",
        denial(
            ingest
                .pending_token_bundle_deletions(&tenant, Some(existing))
                .await
                .map(|pending| pending.len()),
        ),
    ));
    after.push((
        "account admission: claiming a source session",
        denial(
            admission
                .claim_trace_source_session(&tenant, account, &digest, claimed)
                .await,
        ),
    ));
    after.push((
        "account admission: submitting into the claimed session",
        denial(
            ingest
                .upsert_trace_submission(v90_submission(&tenant, claimed, &principal))
                .await
                .map(|record| record.status),
        ),
    ));
    after.push((
        "account admission: withdrawing the claimed session",
        denial(
            admission
                .withdraw_trace_source_session(&tenant, account, claimed, chrono::Utc::now())
                .await
                .map(|mapped| mapped.map(|mapped| mapped.affected_submission_ids)),
        ),
    ));
    after.push((
        "account admission: the session reads back withdrawn",
        denial(
            ingest
                .get_trace_source_session_status(&tenant, account, &digest)
                .await
                .map(|status| status == TraceSourceSessionStatus::Withdrawn),
        ),
    ));
    // An account merge closes the absorbed account as the runtime; that is the
    // one trace_accounts column ingest writes. Closing it also clears the
    // linkage condition for the readiness check below.
    after.push((
        "account merge: closing an account",
        denial(
            ingest_raw
                .batch_execute(&format!(
                    "BEGIN;
                     SELECT set_config('trace_commons.trace_tenant_id', '{tenant}', true);
                     UPDATE trace_accounts SET closed_at = now()
                      WHERE tenant_id = trace_current_tenant_id() AND account_id = '{account}'
                        AND closed_at IS NULL;
                     COMMIT;"
                ))
                .await
                .map_err(trace_commons_server::error::DatabaseError::Postgres),
        ),
    ));
    let _ = ingest_raw.batch_execute("ROLLBACK").await;
    let identity_rewrite = denial(
        ingest_raw
            .batch_execute(&format!(
                "BEGIN;
                 SELECT set_config('trace_commons.trace_tenant_id', '{tenant}', true);
                 UPDATE trace_accounts SET account_id = account_id
                  WHERE tenant_id = trace_current_tenant_id();
                 COMMIT;"
            ))
            .await
            .map_err(trace_commons_server::error::DatabaseError::Postgres),
    );
    let _ = ingest_raw.batch_execute("ROLLBACK").await;
    let readiness_after = admission.account_admission_runtime_ready().await;

    // Applying V90's SQL a second time changes nothing and fails nothing.
    let reapplied = match migrations
        .iter()
        .find(|(version, ..)| *version == RUNTIME_GRANTS_VERSION)
    {
        Some((_, _, sql)) => owner
            .batch_execute(sql)
            .await
            .map_err(|error| format!("{error:?}")),
        None => Err(format!(
            "V{RUNTIME_GRANTS_VERSION} is not wired into the runner"
        )),
    };
    let readiness_after_reapply = admission.account_admission_runtime_ready().await;

    drop((
        superuser,
        ingest,
        admission,
        owner_backend,
        ingest_raw,
        db_client,
        owner,
    ));
    let dropped = admin
        .execute(&format!("DROP DATABASE IF EXISTS {DB} WITH (FORCE)"), &[])
        .await;
    // The probe logins held privileges only in the database just dropped. The
    // group stays: another database on this server may have migrated to V90.
    if dropped.is_ok() {
        admin
            .batch_execute(&format!("DROP ROLE IF EXISTS {INGEST}, {ADMISSION};"))
            .await
            .expect("drop the probe logins");
    }

    // Before V90: every route refused, and refused on the table it names.
    let wrong_before: Vec<String> = before
        .iter()
        .filter(|(_, expected, got)| !got.contains(expected))
        .map(|(route, expected, got)| format!("{route}: expected `{expected}`, got {got}"))
        .collect();
    assert!(
        wrong_before.is_empty(),
        "before V90 each route must be refused the way the pilot refuses it:\n  {}",
        wrong_before.join("\n  ")
    );
    assert!(
        account_id_writable_before,
        "before V90 the pilot's table-wide grant lets the runtime rewrite trace_accounts.account_id"
    );
    assert!(
        matches!(readiness_before, Ok(false)),
        "before V90 account admission's readiness refuses the pilot's runtime: {readiness_before:?}"
    );

    assert_eq!(migrated, Ok(()), "the owner applies V90 through the runner");
    let expected_after: Vec<(&str, String)> = vec![
        ("submit: a new submission", "Ok(Accepted)".into()),
        (
            "re-POST: the witness-evidence retry read",
            "Ok(None)".into(),
        ),
        ("withdraw: the source-session mapping", "Ok(false)".into()),
        (
            "withdraw: revoking through the store, which fires the trigger",
            "Ok(())".into(),
        ),
        (
            "withdraw: the pending token-bundle deletions",
            "Ok([(\"revoked\", 1)])".into(),
        ),
        (
            "withdraw: marking the token object deleted",
            "Ok(())".into(),
        ),
        (
            "withdraw: nothing left pending once the object is deleted",
            "Ok(0)".into(),
        ),
        (
            "account admission: claiming a source session",
            "Ok(Active)".into(),
        ),
        (
            "account admission: submitting into the claimed session",
            "Ok(Accepted)".into(),
        ),
        (
            "account admission: withdrawing the claimed session",
            format!("Ok(Some([{claimed}]))"),
        ),
        (
            "account admission: the session reads back withdrawn",
            "Ok(true)".into(),
        ),
        ("account merge: closing an account", "Ok(())".into()),
    ];
    let got_after: Vec<(&str, String)> = after
        .iter()
        .map(|(route, got)| {
            let got = got
                .strip_prefix("succeeded: ")
                .map(|value| format!("Ok({value})"))
                .unwrap_or_else(|| got.clone());
            (*route, got)
        })
        .collect();
    assert_eq!(
        got_after, expected_after,
        "after V90 the same runtime must complete every route"
    );
    assert!(
        identity_rewrite.contains("permission denied for table trace_accounts"),
        "after V90 the runtime may not rewrite account identity: {identity_rewrite}"
    );
    assert!(
        matches!(readiness_after, Ok(true)),
        "after V90 account admission's readiness accepts the runtime: {readiness_after:?}"
    );
    assert_eq!(reapplied, Ok(()), "V90 is idempotent");
    assert!(
        matches!(readiness_after_reapply, Ok(true)),
        "re-applying V90 keeps the runtime ready: {readiness_after_reapply:?}"
    );
    dropped.expect("drop the V90 probe database");
}
