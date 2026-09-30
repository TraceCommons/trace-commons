// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A database migrated the way the pilot's was, and a runtime login that
//! connects to it the way the pilot's ingest does: through the ingest runtime
//! group, `trace_ingest_runtime`, and nothing else. Included with `#[path]`
//! beside `pilot_runtime_grants.rs`, which the includer declares as a sibling
//! module.

use tokio_postgres::NoTls;
use trace_commons_server::config::DatabaseConfig;
use trace_commons_server::db::Database;
use trace_commons_server::db::postgres::{
    PgBackend, apply_and_record_migration, registered_migrations,
};

use super::pilot_runtime_grants::{PILOT_V62_RUNTIME_GRANTS, PILOT_V74_RUNTIME_GRANT};

/// The last migration the pilot had applied when its runtime group got its
/// hand-made grants.
pub const PILOT_GRANTS_VERSION: i32 = 62;

/// The first migration that grants to `trace_ingest_runtime` itself (V90;
/// matches the constant of the same name in `migration_atomicity_pg.rs`).
/// The pilot took the V74 grant by hand well before this, so this harness
/// applies it at that point in the migration order too, not after the
/// database reaches the current head.
const RUNTIME_GRANTS_VERSION: i32 = 90;

/// Migrates the database at `url` the way the pilot's was migrated: every
/// migration through V62, then the pilot's hand-made grants to
/// `trace_ingest_runtime`, then every migration up to V90 with the pilot's
/// V74 grant taken partway through, at its own version -- the way the pilot
/// took it and `migration_atomicity_pg` reproduces it, not after the
/// database reaches the current head -- then the rest through the real
/// runner. Every table created after V62 is then visible to the group only
/// through the grants its own migration makes.
///
/// Needs a fresh database. A database already past V62 is accepted only when
/// it carries the pilot's grants, which only this function gives: past that
/// point there is no way to grant them where the pilot did.
pub async fn migrate_like_the_pilot(url: &str) {
    let (mut client, connection) = tokio_postgres::connect(url, NoTls)
        .await
        .expect("connect as the migration owner");
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });
    // The group exists before any migration, made by hand, as on the pilot.
    client
        .batch_execute(
            "DO $$ BEGIN
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
                     CREATE ROLE trace_ingest_runtime NOLOGIN NOSUPERUSER NOBYPASSRLS;
                 END IF;
             END $$;
             CREATE TABLE IF NOT EXISTS _trace_commons_migrations (
                 version INTEGER PRIMARY KEY,
                 name TEXT NOT NULL,
                 applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
             );",
        )
        .await
        .expect("create the runtime group and the migration history table");
    let recorded: std::collections::BTreeSet<i32> = client
        .query("SELECT version FROM _trace_commons_migrations", &[])
        .await
        .expect("read the migration history")
        .iter()
        .map(|row| row.get(0))
        .collect();
    if recorded
        .iter()
        .any(|version| *version > PILOT_GRANTS_VERSION)
    {
        let staged: bool = client
            .query_one(
                "SELECT has_table_privilege(
                     'trace_ingest_runtime', 'public.trace_submissions', 'INSERT')",
                &[],
            )
            .await
            .expect("check for the pilot's grants")
            .get(0);
        assert!(
            staged,
            "this suite needs a fresh database: this one is past V{PILOT_GRANTS_VERSION} \
             without the pilot's runtime grants"
        );
    } else {
        for (version, name, sql) in registered_migrations()
            .iter()
            .filter(|(version, ..)| *version <= PILOT_GRANTS_VERSION && !recorded.contains(version))
        {
            apply_and_record_migration(&mut client, *version, name, sql)
                .await
                .unwrap_or_else(|error| panic!("apply V{version}: {error:?}"));
        }
        client
            .batch_execute(PILOT_V62_RUNTIME_GRANTS)
            .await
            .expect("the pilot's V62 runtime grants");
    }

    // The rest up to V90, the first migration that grants to
    // trace_ingest_runtime itself, applied one at a time so the V74 grant
    // can land right after V74 -- at its own version, as the pilot took it
    // and migration_atomicity_pg reproduces it -- rather than after the
    // database reaches the current head.
    let recorded: std::collections::BTreeSet<i32> = client
        .query("SELECT version FROM _trace_commons_migrations", &[])
        .await
        .expect("read the migration history")
        .iter()
        .map(|row| row.get(0))
        .collect();
    for (version, name, sql) in registered_migrations().iter().filter(|(version, ..)| {
        *version > PILOT_GRANTS_VERSION
            && *version < RUNTIME_GRANTS_VERSION
            && !recorded.contains(version)
    }) {
        apply_and_record_migration(&mut client, *version, name, sql)
            .await
            .unwrap_or_else(|error| panic!("apply V{version}: {error:?}"));
    }
    client
        .batch_execute(PILOT_V74_RUNTIME_GRANT)
        .await
        .expect("the pilot's V74 runtime grant");
    drop(client);
    let _ = connection.await;

    let owner = PgBackend::new(&DatabaseConfig::from_postgres_url(url, 2))
        .await
        .expect("connect as the migration owner");
    owner.run_migrations().await.expect("apply migrations");
}

/// Makes `login` a `LOGIN`, `NOSUPERUSER`, `NOBYPASSRLS` role whose only
/// privilege source is membership in `trace_ingest_runtime`, and checks that
/// it is: no other membership, a group that cannot bypass RLS, and no
/// privilege of its own on any schema, table, column, sequence, or function
/// in the database at `url`.
pub async fn provision_member_only_login(url: &str, login: &str) {
    let (client, connection) = tokio_postgres::connect(url, NoTls)
        .await
        .expect("connect as the migration owner");
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });
    let sixteen_or_later: bool = client
        .query_one(
            "SELECT current_setting('server_version_num')::int >= 160000",
            &[],
        )
        .await
        .expect("server version")
        .get(0);
    // Since PostgreSQL 16 a membership carries its own inherit option, taken
    // from the member's INHERIT attribute when it is granted. Say it outright,
    // so a membership granted while the login was NOINHERIT is repaired too.
    let membership = if sixteen_or_later {
        format!("GRANT trace_ingest_runtime TO {login} WITH INHERIT TRUE;")
    } else {
        format!("GRANT trace_ingest_runtime TO {login};")
    };
    client
        .batch_execute(&format!(
            "DO $$ BEGIN
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{login}') THEN
                     CREATE ROLE {login} LOGIN;
                 END IF;
             END $$;
             ALTER ROLE {login} LOGIN INHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
             {membership}
             -- Roles are server-wide, so a prior run's grant of
             -- trace_account_admission_runtime (V77) survives a database
             -- drop; revoke it here before the exclusivity check below.
             DO $$ BEGIN
                 IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_account_admission_runtime') THEN
                     REVOKE trace_account_admission_runtime FROM {login};
                 END IF;
             END $$;"
        ))
        .await
        .expect("provision the runtime login");
    let row = client
        .query_one(
            "SELECT
                 (SELECT array_agg(g.rolname::TEXT ORDER BY g.rolname)
                    FROM pg_auth_members m JOIN pg_roles g ON g.oid = m.roleid
                   WHERE m.member = to_regrole($1)),
                 (SELECT rolsuper OR rolbypassrls FROM pg_roles
                   WHERE rolname = 'trace_ingest_runtime'),
                 (SELECT COUNT(*) FROM (
                      SELECT 1 FROM pg_class c, aclexplode(c.relacl) a
                       WHERE a.grantee = to_regrole($1)
                      UNION ALL
                      SELECT 1 FROM pg_attribute att, aclexplode(att.attacl) a
                       WHERE a.grantee = to_regrole($1)
                      UNION ALL
                      SELECT 1 FROM pg_namespace n, aclexplode(n.nspacl) a
                       WHERE a.grantee = to_regrole($1)
                      UNION ALL
                      SELECT 1 FROM pg_proc p, aclexplode(p.proacl) a
                       WHERE a.grantee = to_regrole($1)
                  ) own),
                 -- Role memberships are per server, not per database: a role
                 -- granted trace_ingest_runtime by one grantor in one test
                 -- run and by a different grantor in another (both on this
                 -- same PostgreSQL server) leaves two pg_auth_members rows
                 -- for the same (roleid, member) pair, one per grantor.
                 -- DISTINCT compares the set of role names, which is what
                 -- this assertion means, and still catches a genuinely new
                 -- role name.
                 (SELECT array_agg(DISTINCT g.rolname::TEXT ORDER BY g.rolname::TEXT)
                    FROM pg_auth_members m JOIN pg_roles g ON g.oid = m.roleid
                   WHERE m.member = 'trace_ingest_runtime'::regrole)",
            &[&login],
        )
        .await
        .expect("read the runtime login's privileges");
    let memberships: Vec<String> = row.get::<_, Option<Vec<String>>>(0).unwrap_or_default();
    let group_privileged: bool = row.get(1);
    let own_privileges: i64 = row.get(2);
    let group_memberships: Vec<String> = row.get::<_, Option<Vec<String>>>(3).unwrap_or_default();
    assert_eq!(
        memberships,
        vec!["trace_ingest_runtime".to_string()],
        "the runtime login must be a member of trace_ingest_runtime and of nothing else"
    );
    assert!(
        !group_privileged,
        "trace_ingest_runtime must be neither SUPERUSER nor BYPASSRLS"
    );
    assert_eq!(
        own_privileges, 0,
        "the runtime login must hold no privilege of its own; use a fresh database"
    );
    // trace_ingest_runtime is itself a member of only the two roles a
    // migration grants it: trace_witness_evidence_runtime (V90) and
    // trace_public_run_runtime (the V74 grant, applied above). A broader
    // membership widens every login the group carries, so catch it here
    // rather than in whatever test happens to exercise it.
    assert_eq!(
        group_memberships,
        vec![
            "trace_public_run_runtime".to_string(),
            "trace_witness_evidence_runtime".to_string(),
        ],
        "trace_ingest_runtime must be a member of only the V74 and V90 roles it is granted"
    );
    drop(client);
    let _ = connection.await;
}

/// Grants `login` membership in `main`'s `trace_account_admission_runtime`
/// (V77), on top of whatever `provision_member_only_login` already gave it.
///
/// `main`'s own legacy session withdrawal reads `trace_account_admission_submissions`
/// under that role, not under `trace_ingest_runtime` (V102 does not grant the
/// table: owner ruling RB-11), and the pipeline withdrawal reaches the same
/// table through `main`'s helper. A test harness whose runtime login
/// exercises the pipeline withdrawal with a mapped source session needs this
/// membership too; call it after `provision_member_only_login`, whose own
/// exclusivity check runs before this grant lands.
pub async fn grant_admission_runtime_membership(url: &str, login: &str) {
    let (client, connection) = tokio_postgres::connect(url, NoTls)
        .await
        .expect("connect as the migration owner");
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .batch_execute(&format!(
            "GRANT trace_account_admission_runtime TO {login};"
        ))
        .await
        .expect("grant trace_account_admission_runtime to the runtime login");
    drop(client);
    let _ = connection.await;
}
