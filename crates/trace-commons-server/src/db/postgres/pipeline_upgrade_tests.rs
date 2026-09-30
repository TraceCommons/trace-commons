// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
// INTEGRATION: upgrades a real V91 database and qualifies pipeline RLS.

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

async fn apply_real_migrations_through_v91(client: &mut Client) {
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
    for (version, name, sql) in MIGRATIONS.iter().filter(|(version, _, _)| *version <= 91) {
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

const PIPELINE_TABLES: [&str; 13] = [
    "pipeline_runs",
    "phase_outcomes",
    "pipeline_bundle_packages",
    "pipeline_active_bundles",
    "pipeline_bundle_policy_status",
    "pipeline_receipt_artifacts",
    "pipeline_run_settlements",
    "pipeline_admission_usage",
    "pipeline_review_claims",
    "pipeline_review_assessments",
    "pipeline_index_invalidations",
    "pipeline_export_snapshots",
    "pipeline_export_snapshot_items",
];

/// Every privilege any role but the owner holds on the pipeline tables once
/// V92 to V95, V103 and V104 have run, as `(table, privilege, columns)`; no columns means
/// the whole table. The ingest runtime group, `trace_ingest_runtime`, is the
/// only grantee, and it holds what the pipeline code reads and writes and
/// nothing broader. A privilege the code comes to need goes into its
/// migration and into this list in the same change.
const RUNTIME_PIPELINE_GRANTS: &[(&str, &str, &[&str])] = &[
    ("pipeline_runs", "SELECT", &[]),
    ("pipeline_runs", "INSERT", &[]),
    (
        "pipeline_runs",
        "UPDATE",
        &[
            // V92
            "next_phase",
            "state",
            "approved_revision_id",
            "last_error_label",
            "index_membership",
            "updated_at",
            // V93
            "lease_token",
            "lease_expires_at",
            "attempt_count",
            "next_attempt_at",
            "phase_started_at",
            // V94
            "index_command_ref",
            "index_command_hash",
            "index_write_state",
            "score_neighbor_ref",
            "score_neighbor_hash",
            "settle_selection",
            "settle_selection_hash",
            // V95
            "approved_object_ref_id",
            "approved_content_hash",
            // V103
            "index_invalidation_state",
        ],
    ),
    ("phase_outcomes", "SELECT", &[]),
    ("phase_outcomes", "INSERT", &[]),
    ("pipeline_bundle_packages", "SELECT", &[]),
    ("pipeline_bundle_packages", "INSERT", &[]),
    ("pipeline_active_bundles", "SELECT", &[]),
    ("pipeline_active_bundles", "INSERT", &[]),
    ("pipeline_bundle_policy_status", "SELECT", &[]),
    ("pipeline_bundle_policy_status", "INSERT", &[]),
    ("pipeline_receipt_artifacts", "SELECT", &[]),
    ("pipeline_receipt_artifacts", "INSERT", &[]),
    ("pipeline_receipt_artifacts", "DELETE", &[]),
    (
        "pipeline_receipt_artifacts",
        "UPDATE",
        &["state", "committed_at", "cleanup_after"],
    ),
    ("pipeline_run_settlements", "SELECT", &[]),
    ("pipeline_run_settlements", "INSERT", &[]),
    (
        "pipeline_run_settlements",
        "UPDATE",
        &[
            "operation_state",
            "result_ref_hash",
            "external_receipt_hash",
            "credit_event_id",
            "settlement_batch_id",
            "payout_state",
            "lease_token",
            "lease_expires_at",
            "dispatched_at",
            "attempt_count",
            "last_error_label",
            "updated_at",
        ],
    ),
    ("pipeline_admission_usage", "SELECT", &[]),
    ("pipeline_admission_usage", "INSERT", &[]),
    // V103
    ("pipeline_index_invalidations", "SELECT", &[]),
    ("pipeline_index_invalidations", "INSERT", &[]),
    (
        "pipeline_index_invalidations",
        "UPDATE",
        &[
            "state",
            "completed_at",
            "attempt_count",
            "next_attempt_at",
            "last_error_label",
        ],
    ),
    ("pipeline_review_claims", "SELECT", &[]),
    ("pipeline_review_claims", "INSERT", &[]),
    ("pipeline_review_claims", "DELETE", &[]),
    (
        "pipeline_review_claims",
        "UPDATE",
        &[
            "reviewer_principal_ref",
            "lease_token",
            "lease_expires_at",
            "claimed_at",
        ],
    ),
    ("pipeline_review_assessments", "SELECT", &[]),
    ("pipeline_review_assessments", "INSERT", &[]),
    // V104
    ("pipeline_export_snapshots", "SELECT", &[]),
    ("pipeline_export_snapshots", "INSERT", &[]),
    (
        "pipeline_export_snapshots",
        "UPDATE",
        &[
            "state",
            "export_manifest_id",
            "completed_at",
            "invalidated_at",
        ],
    ),
    ("pipeline_export_snapshot_items", "SELECT", &[]),
    ("pipeline_export_snapshot_items", "INSERT", &[]),
    (
        "pipeline_export_snapshot_items",
        "UPDATE",
        &["invalidated_at", "invalidation_reason"],
    ),
];

/// The privileges non-owner roles hold on the pipeline tables, table-wide and
/// per column, as sorted `(grantee, table, column, privilege, is_grantable)`
/// rows; the column is empty for a table-wide privilege. `is_grantable` is
/// PostgreSQL's own name for `WITH GRANT OPTION`: a role holding it can grant
/// the privilege on to others, which is broader than holding the privilege
/// itself and is never something a plain `GRANT ... TO trace_ingest_runtime`
/// (no `WITH GRANT OPTION`) produces.
async fn pipeline_table_grants(client: &Client) -> Vec<(String, String, String, String, bool)> {
    let tables: Vec<&str> = PIPELINE_TABLES.to_vec();
    let rows = client
        .query(
            "SELECT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE a.grantee::regrole::TEXT END,
                    c.relname::TEXT, '', a.privilege_type, a.is_grantable
               FROM pg_class c, aclexplode(c.relacl) a
              WHERE c.relnamespace = 'public'::regnamespace
                AND c.relname = ANY($1) AND a.grantee <> c.relowner
             UNION ALL
             SELECT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE a.grantee::regrole::TEXT END,
                    c.relname::TEXT, att.attname::TEXT, a.privilege_type, a.is_grantable
               FROM pg_class c
               JOIN pg_attribute att ON att.attrelid = c.oid
               , aclexplode(att.attacl) a
              WHERE c.relnamespace = 'public'::regnamespace
                AND c.relname = ANY($1) AND a.grantee <> c.relowner",
            &[&tables],
        )
        .await
        .expect("read the pipeline tables' privileges");
    let mut grants: Vec<(String, String, String, String, bool)> = rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4)))
        .collect();
    grants.sort();
    grants
}

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL"]
async fn pipeline_upgrade_from_v91_installs_forced_rls_storage() {
    let url = isolated_upgrade_database_url();
    let (mut admin, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("connect upgrade admin");
    tokio::spawn(async move { connection.await.expect("upgrade connection") });
    apply_real_migrations_through_v91(&mut admin).await;
    assert!(
        admin
            .query_one("SELECT to_regclass('public.pipeline_runs') IS NULL", &[])
            .await
            .unwrap()
            .get::<_, bool>(0),
        "the V91 predecessor must not contain pipeline storage"
    );

    let migrator = PgBackend::new(&database_config(url.clone())).await.unwrap();
    migrator.run_migrations().await.expect("upgrade to current");
    let version: Option<i32> = admin
        .query_one("SELECT MAX(version) FROM _trace_commons_migrations", &[])
        .await
        .unwrap()
        .get(0);
    // The upgrade runs every migration, so the recorded maximum is the
    // newest one in the list; the pipeline versions themselves must be there.
    let latest = super::MIGRATIONS.iter().map(|(v, _, _)| *v).max();
    assert_eq!(version, latest);
    for pipeline_version in [92, 93, 94, 95, 103, 104] {
        let recorded: bool = admin
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM _trace_commons_migrations WHERE version = $1)",
                &[&pipeline_version],
            )
            .await
            .unwrap()
            .get(0);
        assert!(
            recorded,
            "V{pipeline_version} was not recorded by the upgrade"
        );
    }

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

    for (table, column) in [
        ("pipeline_runs", "admission_reason"),
        ("pipeline_runs", "index_invalidation_state"),
        ("pipeline_index_invalidations", "next_attempt_at"),
    ] {
        let present: bool = admin
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM information_schema.columns
                                 WHERE table_name = $1 AND column_name = $2)",
                &[&table, &column],
            )
            .await
            .unwrap()
            .get(0);
        assert!(present, "{table}.{column} must exist after the upgrade");
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

    // Least privilege for the ingest runtime, pinned: exactly these grants on
    // the pipeline tables, to trace_ingest_runtime only, and none of them
    // `WITH GRANT OPTION`.
    let mut expected: Vec<(String, String, String, String, bool)> = RUNTIME_PIPELINE_GRANTS
        .iter()
        .flat_map(|(table, privilege, columns)| {
            let columns: Vec<&str> = if columns.is_empty() {
                vec![""]
            } else {
                columns.to_vec()
            };
            columns.into_iter().map(move |column| {
                (
                    "trace_ingest_runtime".to_string(),
                    table.to_string(),
                    column.to_string(),
                    privilege.to_string(),
                    false,
                )
            })
        })
        .collect();
    expected.sort();
    assert_eq!(
        pipeline_table_grants(&admin).await,
        expected,
        "the pipeline tables must grant trace_ingest_runtime what the pipeline code uses, \
         nothing to anyone else, and none of it WITH GRANT OPTION"
    );

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
