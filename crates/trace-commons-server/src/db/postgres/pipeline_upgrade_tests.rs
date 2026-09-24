// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
// INTEGRATION: upgrades a real V74 database and qualifies pipeline RLS.

use tokio_postgres::Client;

use super::{MIGRATIONS, PgBackend, TRACE_COMMONS_RLS_TABLES, apply_and_record_migration};
use crate::{
    config::{DatabaseConfig, SslMode},
    db::Database,
};

fn database_config(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url: url.into(),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

fn isolated_upgrade_database_url() -> String {
    let url = std::env::var("TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL")
        .expect("TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL is required; no fallback is permitted");
    let target = url
        .parse::<tokio_postgres::Config>()
        .expect("parse explicit pipeline upgrade test URL");
    let hosts = target.get_hosts();
    let hosts_are_local = hosts.iter().all(|host| match host {
        tokio_postgres::config::Host::Tcp(host) => host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback()),
        #[cfg(unix)]
        tokio_postgres::config::Host::Unix(_) => true,
    });
    assert!(
        !hosts.is_empty() && hosts_are_local,
        "every explicit PostgreSQL host must be loopback or a local Unix socket"
    );
    assert!(
        target
            .get_hostaddrs()
            .iter()
            .all(std::net::IpAddr::is_loopback),
        "every PostgreSQL hostaddr override must be loopback"
    );
    assert!(
        target
            .get_dbname()
            .is_some_and(|name| name.starts_with("pipeline_test_")),
        "database name must start pipeline_test_"
    );
    url
}

async fn apply_real_migrations_through_v74(client: &mut Client) {
    client
        .batch_execute(
            "CREATE TABLE _trace_commons_migrations (\
                version INTEGER PRIMARY KEY,\
                name TEXT NOT NULL,\
                applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
            );",
        )
        .await
        .expect("create migration history table");
    for (version, name, sql) in MIGRATIONS.iter().filter(|(version, _, _)| *version <= 74) {
        apply_and_record_migration(client, *version, name, sql)
            .await
            .unwrap_or_else(|error| panic!("apply real V{version} ({name}): {error}"));
    }
}

async fn set_tenant(client: &Client, tenant: &str) {
    client
        .execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, false)",
            &[&tenant],
        )
        .await
        .expect("set migration test tenant");
}

const PIPELINE_TABLES: [&str; 8] = [
    "pipeline_runs",
    "phase_outcomes",
    "pipeline_bundle_packages",
    "pipeline_active_bundles",
    "pipeline_bundle_policy_status",
    "pipeline_receipt_artifacts",
    "pipeline_run_settlements",
    "pipeline_admission_usage",
];

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL"]
async fn pipeline_upgrade_from_v74_installs_forced_rls_storage() {
    let url = isolated_upgrade_database_url();
    let (mut admin, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("connect upgrade admin");
    tokio::spawn(async move { connection.await.expect("upgrade connection") });
    apply_real_migrations_through_v74(&mut admin).await;
    assert!(
        admin
            .query_one("SELECT to_regclass('public.pipeline_runs') IS NULL", &[])
            .await
            .unwrap()
            .get::<_, bool>(0),
        "the V74 predecessor must not contain pipeline storage"
    );

    let migrator = PgBackend::new(&database_config(url.clone())).await.unwrap();
    migrator.run_migrations().await.expect("upgrade to current");
    let version: Option<i32> = admin
        .query_one("SELECT MAX(version) FROM _trace_commons_migrations", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(version, Some(78));

    for table in PIPELINE_TABLES {
        assert!(
            TRACE_COMMONS_RLS_TABLES.contains(&table),
            "{table} not registered"
        );
        let row = admin
            .query_one(
                "SELECT c.relrowsecurity, c.relforcerowsecurity,
                        EXISTS (SELECT 1 FROM pg_policies p
                                 WHERE p.tablename = c.relname
                                   AND p.policyname = 'trace_corpus_tenant_isolation')
                   FROM pg_class c WHERE c.relname = $1",
                &[&table],
            )
            .await
            .unwrap();
        assert!(
            row.get::<_, bool>(0) && row.get::<_, bool>(1) && row.get::<_, bool>(2),
            "{table} must enable and force RLS with the tenant policy"
        );
    }
    let claim_function: bool = admin
        .query_one(
            "SELECT to_regprocedure('claim_pipeline_run(uuid,integer)') IS NOT NULL",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!claim_function, "no cross-tenant claim function may exist");

    // Isolation as a role that cannot bypass RLS.
    admin
        .batch_execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ('upgrade-a'), ('upgrade-b')
                 ON CONFLICT DO NOTHING;
             DO $$ BEGIN
                IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pipeline_upgrade_runtime')
                THEN CREATE ROLE pipeline_upgrade_runtime NOLOGIN NOSUPERUSER NOBYPASSRLS; END IF;
             END $$;
             GRANT USAGE ON SCHEMA public TO pipeline_upgrade_runtime;
             GRANT SELECT, INSERT ON pipeline_bundle_packages TO pipeline_upgrade_runtime;",
        )
        .await
        .unwrap();
    for (tenant, hex) in [("upgrade-a", "a"), ("upgrade-b", "b")] {
        set_tenant(&admin, tenant).await;
        admin
            .execute(
                "INSERT INTO pipeline_bundle_packages (tenant_id, bundle_id, manifest_format_version, package)
                 VALUES ($1, $2, 1, '{}'::jsonb)",
                &[&tenant, &format!("sha256:{}", hex.repeat(64))],
            )
            .await
            .unwrap();
    }
    admin
        .batch_execute("SET ROLE pipeline_upgrade_runtime")
        .await
        .unwrap();
    set_tenant(&admin, "upgrade-a").await;
    let visible: i64 = admin
        .query_one("SELECT COUNT(*) FROM pipeline_bundle_packages", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(visible, 1, "a tenant sees only its own package");
    set_tenant(&admin, "").await;
    let unscoped: i64 = admin
        .query_one("SELECT COUNT(*) FROM pipeline_bundle_packages", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(unscoped, 0, "no tenant context sees nothing");
    admin.batch_execute("RESET ROLE").await.unwrap();
}
