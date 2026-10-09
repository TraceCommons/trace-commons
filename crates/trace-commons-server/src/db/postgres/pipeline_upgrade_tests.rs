// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
// INTEGRATION: upgrades a real V91 database and qualifies pipeline RLS.

use tokio_postgres::{Client, error::SqlState};

use super::{MIGRATIONS, PgBackend, TRACE_COMMONS_RLS_TABLES, apply_and_record_migration};
use crate::{
    config::{DatabaseConfig, SslMode},
    db::Database,
    versioned_pipeline_qualification::PipelineCheckEmitter,
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

async fn apply_real_migrations_through(client: &mut Client, last: i32) {
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
    for (version, name, sql) in MIGRATIONS.iter().filter(|(version, _, _)| *version <= last) {
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

const PIPELINE_TABLES: [&str; 20] = [
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
    "pipeline_bundle_qualifications",
    "pipeline_attempt_artifacts",
    "pipeline_tenant_routing",
    "pipeline_activation_events",
    "pipeline_receipt_ownership",
    "pipeline_policy_interventions",
    "pipeline_index_rebuild_fences",
];

/// Every privilege the ingest runtime group, `trace_ingest_runtime`, holds on
/// the pipeline tables once V92 to V95, V105 to V108, V110 to V113, and V117 have run, as
/// `(table, privilege, columns)`; no columns means the whole table. It holds
/// what the pipeline code reads and writes and nothing broader. The only
/// other grantee is `trace_gate_driver` (`GATE_DRIVER_PIPELINE_GRANTS`). A
/// privilege the code comes to need goes into its migration and into this
/// list in the same change.
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
            // V117: the Review-start privacy pass records its result, and
            // the Review commit links an escalated run's approval to it.
            "privacy_pass_object_ref_id",
            "privacy_pass_content_hash",
            "privacy_pass_source_hash",
            "privacy_pass_residual_risk_basis",
            "privacy_pass_outcome",
            "privacy_pass_recorded_at",
            "privacy_pass_approval_assessment_hash",
            "privacy_pass_approval_resolved_reasons",
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
            // V105
            "credit_audited_at",
        ],
    ),
    ("pipeline_admission_usage", "SELECT", &[]),
    ("pipeline_admission_usage", "INSERT", &[]),
    // V105
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
    // V106
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
    // V107: append-only, no UPDATE or DELETE.
    ("pipeline_bundle_qualifications", "SELECT", &[]),
    ("pipeline_bundle_qualifications", "INSERT", &[]),
    // V108
    ("pipeline_attempt_artifacts", "SELECT", &[]),
    ("pipeline_attempt_artifacts", "INSERT", &[]),
    ("pipeline_attempt_artifacts", "DELETE", &[]),
    // Rebase 10, option D: the commit sets the hash a compatibility Score's
    // row was staged without.
    (
        "pipeline_attempt_artifacts",
        "UPDATE",
        &["state", "committed_at", "ciphertext_sha256"],
    ),
    // V110: the routing row is read by every upload and written by an
    // operator action (every column but the key and `routing_generation`,
    // which the row trigger sets on an update, so it needs no grant); the
    // event and ownership tables are append-only, no UPDATE or DELETE.
    ("pipeline_tenant_routing", "SELECT", &[]),
    ("pipeline_tenant_routing", "INSERT", &[]),
    (
        "pipeline_tenant_routing",
        "UPDATE",
        &[
            "routing_state",
            "activation_record_id",
            "actor_principal_ref",
            "reason_code",
            "evidence_hash",
            "recorded_at",
        ],
    ),
    ("pipeline_activation_events", "SELECT", &[]),
    ("pipeline_activation_events", "INSERT", &[]),
    ("pipeline_receipt_ownership", "SELECT", &[]),
    ("pipeline_receipt_ownership", "INSERT", &[]),
    // V111: an intervention updates the status row's four columns (a phase
    // commit locks the row FOR SHARE, which needs a column UPDATE) and
    // appends its record, which is never updated or deleted.
    (
        "pipeline_bundle_policy_status",
        "UPDATE",
        &[
            "runnable",
            "operational_status",
            "error_label",
            "updated_at",
        ],
    ),
    ("pipeline_policy_interventions", "SELECT", &[]),
    ("pipeline_policy_interventions", "INSERT", &[]),
    // V112: the qualified activation gate switches the tenant's active
    // bundle (the two columns it sets; `SELECT ... FOR UPDATE` needs one).
    (
        "pipeline_active_bundles",
        "UPDATE",
        &["bundle_id", "selected_at"],
    ),
    // V113: a rebuild inserts its fence row, extends it (only
    // `fenced_until` changes), and deletes it; the invalidation claim reads
    // the rows.
    ("pipeline_index_rebuild_fences", "SELECT", &[]),
    ("pipeline_index_rebuild_fences", "INSERT", &[]),
    ("pipeline_index_rebuild_fences", "DELETE", &[]),
    ("pipeline_index_rebuild_fences", "UPDATE", &["fenced_until"]),
];

/// What `main`'s gate driver role, `trace_gate_driver`, holds on the pipeline
/// tables (V105): the two `pipeline_runs` columns its enumeration joins on to
/// leave pipeline submissions out (Zaki review 1, round 2, finding 1).
const GATE_DRIVER_PIPELINE_GRANTS: &[(&str, &str, &[&str])] =
    &[("pipeline_runs", "SELECT", &["tenant_id", "submission_id"])];

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

/// Multi-lens review C13: the tests of this module each apply every
/// migration, in a database of their own on one server. The migrations hold
/// `ALTER ROLE` statements, roles are shared by the whole server, and two
/// such statements on one role at the same time fail (`tuple concurrently
/// updated`); the migration lock is per database and does not order them.
/// Each test holds this lock from its first line, so they run one after the
/// other however the test binary is started.
static UPGRADE_CLUSTER_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL"]
async fn pipeline_upgrade_from_v91_installs_forced_rls_storage() {
    let _serial = UPGRADE_CLUSTER_LOCK.lock().await;
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
    for pipeline_version in [
        92, 93, 94, 95, 105, 106, 107, 108, 109, 110, 111, 112, 113, 117,
    ] {
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

    let mut forced_rls_tables = 0usize;
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
        forced_rls_tables += 1;
    }

    for (table, column) in [
        ("pipeline_runs", "admission_reason"),
        ("pipeline_run_settlements", "payout_eligible"),
        ("pipeline_run_settlements", "credit_audited_at"),
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

    // The pipeline's row-guard triggers, as the upgrade leaves them: present,
    // on their tables, and enabled. V108's guard is what keeps a committed
    // attempt artifact from going back to `staged` (final review M6). Four
    // of V110's triggers keep an activation event and a receipt owner
    // immutable, and its other two bind the routing row to its event (the
    // generation and the commit check). V111's two keep a policy
    // intervention immutable.
    for (table, trigger) in [
        ("phase_outcomes", "phase_outcomes_reject_update"),
        ("phase_outcomes", "phase_outcomes_reject_delete"),
        (
            "pipeline_review_assessments",
            "pipeline_review_assessments_reject_update",
        ),
        (
            "pipeline_review_assessments",
            "pipeline_review_assessments_reject_delete",
        ),
        (
            "pipeline_bundle_qualifications",
            "pipeline_bundle_qualifications_reject_update",
        ),
        (
            "pipeline_bundle_qualifications",
            "pipeline_bundle_qualifications_reject_delete",
        ),
        (
            "pipeline_attempt_artifacts",
            "pipeline_attempt_artifacts_guard_update",
        ),
        (
            "pipeline_activation_events",
            "pipeline_activation_events_reject_update",
        ),
        (
            "pipeline_activation_events",
            "pipeline_activation_events_reject_delete",
        ),
        (
            "pipeline_tenant_routing",
            "pipeline_tenant_routing_assign_generation",
        ),
        (
            "pipeline_tenant_routing",
            "pipeline_tenant_routing_event_match",
        ),
        (
            "pipeline_receipt_ownership",
            "pipeline_receipt_ownership_reject_update",
        ),
        (
            "pipeline_receipt_ownership",
            "pipeline_receipt_ownership_reject_delete",
        ),
        (
            "pipeline_policy_interventions",
            "pipeline_policy_interventions_reject_update",
        ),
        (
            "pipeline_policy_interventions",
            "pipeline_policy_interventions_reject_delete",
        ),
    ] {
        let enabled: bool = admin
            .query_one(
                "SELECT EXISTS (
                    SELECT 1 FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid
                     WHERE c.relname = $1 AND t.tgname = $2
                       AND NOT t.tgisinternal AND t.tgenabled <> 'D')",
                &[&table, &trigger],
            )
            .await
            .unwrap()
            .get(0);
        assert!(enabled, "{table} must carry the enabled trigger {trigger}");
    }

    // Rebase 10, option D: the upgrade installs V108's rule for a row staged
    // with no hash. The hash column is nullable, a committed row must have
    // one, and the guard lets only the commit set a missing hash.
    let attempt_hash = admin
        .query_one(
            "SELECT
                (SELECT is_nullable FROM information_schema.columns
                  WHERE table_name = 'pipeline_attempt_artifacts'
                    AND column_name = 'ciphertext_sha256'),
                pg_get_functiondef('guard_pipeline_attempt_artifact_update()'::regprocedure),
                (SELECT string_agg(pg_get_constraintdef(oid), ' ')
                   FROM pg_constraint
                  WHERE conrelid = 'pipeline_attempt_artifacts'::regclass AND contype = 'c')",
            &[],
        )
        .await
        .unwrap();
    let (nullable, guard, checks): (String, String, String) = (
        attempt_hash.get(0),
        attempt_hash.get(1),
        attempt_hash.get(2),
    );
    assert_eq!(nullable, "YES", "a staged attempt row may carry no hash");
    assert!(
        guard.contains("OLD.ciphertext_sha256 IS NULL")
            && guard.contains("NEW.ciphertext_sha256 ~ '^[0-9a-f]{64}$'"),
        "V108's guard lets the commit set a missing hash, and only to a hex value"
    );
    assert!(
        checks.contains("ciphertext_sha256 IS NOT NULL"),
        "a committed attempt row must have its hash: {checks}"
    );
    // Rebase 10 review, M8: an `approved` row always has its hash, so only a
    // compatibility Score's two artifacts can be staged without one. V117
    // adds the privacy pass's `privacy-pass` row to the rows that need one.
    let approved_hash: Option<String> = admin
        .query_opt(
            "SELECT pg_get_constraintdef(oid) FROM pg_constraint
              WHERE conrelid = 'pipeline_attempt_artifacts'::regclass
                AND conname = 'pipeline_attempt_artifacts_approved_hash'",
            &[],
        )
        .await
        .unwrap()
        .map(|row| row.get(0));
    assert!(
        approved_hash.as_deref().is_some_and(|definition| {
            definition.contains("ciphertext_sha256 IS NOT NULL")
                && definition
                    .contains("artifact <> ALL (ARRAY['approved'::text, 'privacy-pass'::text])")
        }),
        "an approved attempt row must have its hash: {approved_hash:?}"
    );

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
    let mut expected: Vec<(String, String, String, String, bool)> = [
        ("trace_ingest_runtime", RUNTIME_PIPELINE_GRANTS),
        ("trace_gate_driver", GATE_DRIVER_PIPELINE_GRANTS),
    ]
    .into_iter()
    .flat_map(|(grantee, grants)| {
        grants.iter().flat_map(move |(table, privilege, columns)| {
            let columns: Vec<&str> = if columns.is_empty() {
                vec![""]
            } else {
                columns.to_vec()
            };
            columns.into_iter().map(move |column| {
                (
                    grantee.to_string(),
                    table.to_string(),
                    column.to_string(),
                    privilege.to_string(),
                    false,
                )
            })
        })
    })
    .collect();
    expected.sort();
    assert_eq!(
        pipeline_table_grants(&admin).await,
        expected,
        "the pipeline tables must grant trace_ingest_runtime what the pipeline code uses, \
         trace_gate_driver its two pipeline_runs columns, nothing to anyone else, and none \
         of it WITH GRANT OPTION"
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

    // V110: the routing row is bound to its event. The key from the row to
    // the event exists and is deferred, the row's generation and the commit
    // check are triggers of the two functions, and neither function has
    // definer rights or a setting of its own (#974).
    let routing_key = admin
        .query_one(
            "SELECT condeferrable, condeferred, confrelid::regclass::text,
                    pg_get_constraintdef(oid)
               FROM pg_constraint
              WHERE conrelid = 'pipeline_tenant_routing'::regclass
                AND conname = 'pipeline_tenant_routing_activation_event_fkey'
                AND contype = 'f'",
            &[],
        )
        .await
        .expect("the routing row has its foreign key to the events");
    assert!(
        routing_key.get::<_, bool>(0) && routing_key.get::<_, bool>(1),
        "the routing row's key to its event is DEFERRABLE INITIALLY DEFERRED"
    );
    assert_eq!(
        routing_key.get::<_, String>(2),
        "pipeline_activation_events"
    );
    assert!(
        routing_key.get::<_, String>(3).starts_with(
            "FOREIGN KEY (tenant_id, activation_record_id) \
             REFERENCES pipeline_activation_events(tenant_id, event_id)"
        ),
        "{}",
        routing_key.get::<_, String>(3)
    );
    for (trigger, function, deferred) in [
        (
            "pipeline_tenant_routing_assign_generation",
            "assign_pipeline_routing_generation",
            false,
        ),
        (
            "pipeline_tenant_routing_event_match",
            "check_pipeline_routing_event",
            true,
        ),
    ] {
        let row = admin
            .query_one(
                "SELECT p.proname, t.tgdeferrable AND t.tginitdeferred,
                        p.prosecdef, p.proconfig IS NULL
                   FROM pg_trigger t JOIN pg_proc p ON p.oid = t.tgfoid
                  WHERE t.tgrelid = 'pipeline_tenant_routing'::regclass
                    AND t.tgname = $1 AND NOT t.tgisinternal",
                &[&trigger],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, String>(0), function, "{trigger}");
        assert_eq!(row.get::<_, bool>(1), deferred, "{trigger}");
        assert!(
            !row.get::<_, bool>(2) && row.get::<_, bool>(3),
            "{function} has invoker rights and no settings"
        );
    }
    for table in ["pipeline_tenant_routing", "pipeline_activation_events"] {
        let column = admin
            .query_one(
                "SELECT data_type, is_nullable FROM information_schema.columns
                  WHERE table_name = $1 AND column_name = 'routing_generation'",
                &[&table],
            )
            .await
            .unwrap();
        assert_eq!(
            (column.get::<_, String>(0), column.get::<_, String>(1)),
            ("bigint".to_string(), "NO".to_string()),
            "{table}.routing_generation"
        );
    }

    // V110: an ownership row is unique for each submission id, an event and
    // an ownership row are never updated or deleted directly, and deleting
    // the tenant removes its routing, event, and ownership rows (the cascade
    // is let through by the triggers' `pg_trigger_depth() > 1` rule, and the
    // routing row's deferred key to its event holds when both rows go). The
    // routing row and its event share one id, and the event has the
    // generation that an inserted row gets when it names none (1): without
    // that event the batch does not commit.
    let owner_tenant = "upgrade-v110";
    let owner_hash = format!("sha256:{}", "c".repeat(64));
    let owner_event = "00000000-0000-0000-0000-0000000000e1";
    admin
        .batch_execute(&format!(
            "BEGIN;
             INSERT INTO trace_tenants (tenant_id) VALUES ('{owner_tenant}')
                 ON CONFLICT DO NOTHING;
             INSERT INTO pipeline_tenant_routing
                 (tenant_id, routing_state, activation_record_id, actor_principal_ref,
                  reason_code, evidence_hash)
                 VALUES ('{owner_tenant}', 'pipeline', '{owner_event}',
                         'principal:operator', 'activation_recorded', '{owner_hash}');
             INSERT INTO pipeline_activation_events
                 (tenant_id, event_id, action, previous_state, resulting_state,
                  previous_bundle_id, resulting_bundle_id, actor_principal_ref,
                  reason_code, evidence_hash, routing_generation)
                 VALUES ('{owner_tenant}', '{owner_event}', 'activate', 'unselected',
                         'pipeline', NULL, '{owner_hash}', 'principal:operator',
                         'activation_recorded', '{owner_hash}', 1);
             INSERT INTO pipeline_receipt_ownership (tenant_id, submission_id, owner)
                 VALUES ('{owner_tenant}', '00000000-0000-0000-0000-0000000000a1', 'legacy');
             COMMIT;"
        ))
        .await
        .expect("insert one routing, event, and ownership row");
    set_tenant(&admin, owner_tenant).await;
    // A change of the row that keeps its event is refused at the statement,
    // and a row that names no event is refused at the commit.
    let kept = admin
        .batch_execute(&format!(
            "UPDATE pipeline_tenant_routing SET routing_state = 'legacy'
              WHERE tenant_id = '{owner_tenant}';"
        ))
        .await
        .expect_err("a routing update that keeps its event must fail");
    assert_eq!(
        kept.as_db_error().map(|error| error.message()),
        Some("pipeline routing change needs a new activation event"),
        "{kept}"
    );
    let unnamed = admin
        .batch_execute(&format!(
            "UPDATE pipeline_tenant_routing
                SET routing_state = 'legacy', activation_record_id = gen_random_uuid()
              WHERE tenant_id = '{owner_tenant}';"
        ))
        .await
        .expect_err("a routing row that names no event must fail at the commit");
    assert_eq!(
        unnamed.code(),
        Some(&SqlState::FOREIGN_KEY_VIOLATION),
        "{unnamed}"
    );
    let duplicate = admin
        .batch_execute(&format!(
            "INSERT INTO pipeline_receipt_ownership (tenant_id, submission_id, owner)
                 VALUES ('{owner_tenant}', '00000000-0000-0000-0000-0000000000a1', 'legacy');"
        ))
        .await
        .expect_err("a second owner row for one submission id must fail");
    assert_eq!(
        duplicate.code(),
        Some(&SqlState::UNIQUE_VIOLATION),
        "a second ownership row for one (tenant_id, submission_id) is a unique violation: \
         {duplicate}"
    );
    for statement in [
        "UPDATE pipeline_receipt_ownership SET owner = 'legacy'",
        "DELETE FROM pipeline_receipt_ownership",
        "UPDATE pipeline_activation_events SET reason_code = 'rewritten'",
        "DELETE FROM pipeline_activation_events",
    ] {
        let refused = admin
            .batch_execute(&format!("{statement} WHERE tenant_id = '{owner_tenant}';"))
            .await
            .expect_err("an activation record is immutable");
        assert_eq!(
            refused.as_db_error().map(|error| error.message()),
            Some("pipeline activation records are immutable"),
            "`{statement}` must fail with the trigger's message: {refused}"
        );
    }
    let owner_counts = |table: &'static str| {
        let admin = &admin;
        async move {
            admin
                .query_one(
                    &format!("SELECT COUNT(*) FROM {table} WHERE tenant_id = $1"),
                    &[&owner_tenant],
                )
                .await
                .unwrap()
                .get::<_, i64>(0)
        }
    };
    for table in [
        "pipeline_tenant_routing",
        "pipeline_activation_events",
        "pipeline_receipt_ownership",
    ] {
        assert_eq!(
            owner_counts(table).await,
            1,
            "{table} keeps its row through every refused change"
        );
    }
    admin
        .execute(
            "DELETE FROM trace_tenants WHERE tenant_id = $1",
            &[&owner_tenant],
        )
        .await
        .expect("deleting the tenant cascades through the immutable tables");
    for table in [
        "pipeline_tenant_routing",
        "pipeline_activation_events",
        "pipeline_receipt_ownership",
    ] {
        assert_eq!(
            owner_counts(table).await,
            0,
            "deleting the tenant removes its {table} rows"
        );
    }

    // V111: a status row starts runnable with the status `runnable`, and a
    // check keeps `runnable` equal to `operational_status = 'runnable'`, so
    // no change can leave the flag and the status apart. An intervention
    // record is never updated or deleted directly, needs its status row, and
    // goes with its tenant (the cascade is let through by the trigger's
    // `pg_trigger_depth() > 1` rule).
    let policy_tenant = "upgrade-v111";
    let policy_bundle = format!("sha256:{}", "d".repeat(64));
    let policy_evidence = format!("sha256:{}", "e".repeat(64));
    admin
        .batch_execute(&format!(
            "INSERT INTO trace_tenants (tenant_id) VALUES ('{policy_tenant}')
                 ON CONFLICT DO NOTHING;
             INSERT INTO pipeline_bundle_packages
                 (tenant_id, bundle_id, manifest_format_version, package)
                 VALUES ('{policy_tenant}', '{policy_bundle}', 1, '{{}}'::jsonb);
             INSERT INTO pipeline_bundle_policy_status (tenant_id, bundle_id, phase)
                 VALUES ('{policy_tenant}', '{policy_bundle}', 'score');"
        ))
        .await
        .expect("insert a package and one policy status row");
    set_tenant(&admin, policy_tenant).await;
    let status_row = admin
        .query_one(
            "SELECT runnable, operational_status, error_label
               FROM pipeline_bundle_policy_status WHERE tenant_id = $1",
            &[&policy_tenant],
        )
        .await
        .unwrap();
    assert_eq!(
        (
            status_row.get::<_, bool>("runnable"),
            status_row.get::<_, String>("operational_status"),
            status_row.get::<_, Option<String>>("error_label"),
        ),
        (true, "runnable".to_string(), None),
        "a status row starts runnable"
    );
    for statement in [
        "UPDATE pipeline_bundle_policy_status SET runnable = FALSE",
        "UPDATE pipeline_bundle_policy_status SET operational_status = 'suspended'",
        "UPDATE pipeline_bundle_policy_status
            SET runnable = TRUE, operational_status = 'terminated'",
    ] {
        let refused = admin
            .batch_execute(&format!("{statement} WHERE tenant_id = '{policy_tenant}';"))
            .await
            .expect_err("a status row that disagrees with itself is refused");
        assert_eq!(
            refused.code(),
            Some(&SqlState::CHECK_VIOLATION),
            "`{statement}` is a check violation: {refused}"
        );
        assert!(
            refused
                .as_db_error()
                .is_some_and(|error| error.constraint()
                    == Some("pipeline_bundle_policy_status_runnable_shape")),
            "`{statement}` names the shape check: {refused}"
        );
    }
    let unknown_status = admin
        .batch_execute(&format!(
            "UPDATE pipeline_bundle_policy_status
                SET runnable = FALSE, operational_status = 'paused'
              WHERE tenant_id = '{policy_tenant}';"
        ))
        .await
        .expect_err("an unknown status is refused");
    assert_eq!(unknown_status.code(), Some(&SqlState::CHECK_VIOLATION));
    admin
        .batch_execute(&format!(
            "UPDATE pipeline_bundle_policy_status
                SET runnable = FALSE, operational_status = 'suspended',
                    error_label = 'unsafe_bound_policy'
              WHERE tenant_id = '{policy_tenant}';
             INSERT INTO pipeline_policy_interventions
                 (tenant_id, intervention_id, bundle_id, phase, action,
                  actor_principal_ref, reason_code, previous_status,
                  resulting_status, evidence_hash)
                 VALUES ('{policy_tenant}', gen_random_uuid(), '{policy_bundle}', 'score',
                         'suspend', 'operator_sha256:test', 'unsafe_bound_policy',
                         'runnable', 'suspended', '{policy_evidence}');"
        ))
        .await
        .expect("a suspended row and its intervention record");
    let orphan = admin
        .batch_execute(&format!(
            "INSERT INTO pipeline_policy_interventions
                 (tenant_id, intervention_id, bundle_id, phase, action,
                  actor_principal_ref, reason_code, previous_status,
                  resulting_status, evidence_hash)
                 VALUES ('{policy_tenant}', gen_random_uuid(), '{policy_bundle}', 'review',
                         'suspend', 'operator_sha256:test', 'unsafe_bound_policy',
                         'runnable', 'suspended', '{policy_evidence}');"
        ))
        .await
        .expect_err("an intervention needs the status row of its phase");
    assert_eq!(orphan.code(), Some(&SqlState::FOREIGN_KEY_VIOLATION));
    for statement in [
        "UPDATE pipeline_policy_interventions SET reason_code = 'rewritten'",
        "DELETE FROM pipeline_policy_interventions",
    ] {
        let refused = admin
            .batch_execute(&format!("{statement} WHERE tenant_id = '{policy_tenant}';"))
            .await
            .expect_err("an intervention record is immutable");
        assert_eq!(
            refused.as_db_error().map(|error| error.message()),
            Some("pipeline policy interventions are immutable"),
            "`{statement}` must fail with the trigger's message: {refused}"
        );
    }
    let policy_counts = |table: &'static str| {
        let admin = &admin;
        async move {
            admin
                .query_one(
                    &format!("SELECT COUNT(*) FROM {table} WHERE tenant_id = $1"),
                    &[&policy_tenant],
                )
                .await
                .unwrap()
                .get::<_, i64>(0)
        }
    };
    assert_eq!(
        policy_counts("pipeline_policy_interventions").await,
        1,
        "the record keeps its row through every refused change"
    );
    admin
        .execute(
            "DELETE FROM trace_tenants WHERE tenant_id = $1",
            &[&policy_tenant],
        )
        .await
        .expect("deleting the tenant cascades through the immutable record");
    for table in [
        "pipeline_bundle_policy_status",
        "pipeline_policy_interventions",
    ] {
        assert_eq!(
            policy_counts(table).await,
            0,
            "deleting the tenant removes its {table} rows"
        );
    }

    // V112: a bundle is qualified once for each code revision. The key is
    // (tenant_id, bundle_id, code_revision_hash): two rows for one bundle on
    // two revisions are accepted, and a second row for one revision is a
    // unique violation.
    let primary_key: String = admin
        .query_one(
            "SELECT pg_get_constraintdef(oid) FROM pg_constraint
              WHERE conrelid = 'pipeline_bundle_qualifications'::regclass AND contype = 'p'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        primary_key,
        "PRIMARY KEY (tenant_id, bundle_id, code_revision_hash)"
    );
    let qualified_tenant = "upgrade-v112";
    let qualified_bundle = format!("sha256:{}", "f".repeat(64));
    admin
        .batch_execute(&format!(
            "INSERT INTO trace_tenants (tenant_id) VALUES ('{qualified_tenant}')
                 ON CONFLICT DO NOTHING;
             INSERT INTO pipeline_bundle_packages
                 (tenant_id, bundle_id, manifest_format_version, package)
                 VALUES ('{qualified_tenant}', '{qualified_bundle}', 1, '{{}}'::jsonb);"
        ))
        .await
        .expect("insert a package to qualify");
    set_tenant(&admin, qualified_tenant).await;
    let qualify_on = |revision_digit: &'static str| {
        let admin = &admin;
        let qualified_bundle = &qualified_bundle;
        async move {
            let digest = format!("sha256:{}", "1".repeat(64));
            let revision = format!("sha256:{}", revision_digit.repeat(64));
            admin
                .batch_execute(&format!(
                    "INSERT INTO pipeline_bundle_qualifications
                         (tenant_id, bundle_id, package_hash, signing_key_id,
                          signature_hash, corpus_digest, input_digest,
                          configuration_digest, code_revision_hash,
                          runtime_dependency_digest, evidence_hash)
                         VALUES ('{qualified_tenant}', '{qualified_bundle}', '{digest}',
                                 'upgrade-key', '{digest}', '{digest}', '{digest}',
                                 '{digest}', '{revision}', '{digest}', '{digest}');"
                ))
                .await
        }
    };
    qualify_on("2")
        .await
        .expect("a qualification on the first revision");
    qualify_on("3")
        .await
        .expect("a qualification of the same bundle on a second revision");
    let duplicate = qualify_on("3")
        .await
        .expect_err("a second qualification on one revision must fail");
    assert_eq!(
        duplicate.code(),
        Some(&SqlState::UNIQUE_VIOLATION),
        "a second row for one (tenant_id, bundle_id, code_revision_hash) is a unique \
         violation: {duplicate}"
    );
    let qualified_rows: i64 = admin
        .query_one(
            "SELECT COUNT(*) FROM pipeline_bundle_qualifications WHERE tenant_id = $1",
            &[&qualified_tenant],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(qualified_rows, 2, "one row for each revision");
    admin
        .execute(
            "DELETE FROM trace_tenants WHERE tenant_id = $1",
            &[&qualified_tenant],
        )
        .await
        .expect("deleting the tenant cascades through its qualifications");

    // V113: one fence row per rebuild. Two rebuilds of one tenant each hold
    // a row, a second row with one rebuild's fence id is a unique violation,
    // and the rows go with their tenant.
    let fence_tenant = "upgrade-v113";
    admin
        .batch_execute(&format!(
            "INSERT INTO trace_tenants (tenant_id) VALUES ('{fence_tenant}')
                 ON CONFLICT DO NOTHING;
             INSERT INTO pipeline_index_rebuild_fences (tenant_id, fence_id, fenced_until)
                 VALUES ('{fence_tenant}', '00000000-0000-0000-0000-000000000001',
                         clock_timestamp() + INTERVAL '1 minute'),
                        ('{fence_tenant}', '00000000-0000-0000-0000-000000000002',
                         clock_timestamp() + INTERVAL '1 minute');"
        ))
        .await
        .expect("two rebuilds of one tenant each hold a fence row");
    let same_fence = admin
        .batch_execute(&format!(
            "INSERT INTO pipeline_index_rebuild_fences (tenant_id, fence_id, fenced_until)
                 VALUES ('{fence_tenant}', '00000000-0000-0000-0000-000000000001',
                         clock_timestamp());"
        ))
        .await
        .expect_err("one rebuild has one fence row");
    assert_eq!(same_fence.code(), Some(&SqlState::UNIQUE_VIOLATION));
    set_tenant(&admin, fence_tenant).await;
    admin
        .execute(
            "DELETE FROM trace_tenants WHERE tenant_id = $1",
            &[&fence_tenant],
        )
        .await
        .expect("deleting the tenant cascades through its fences");
    let fence_rows: i64 = admin
        .query_one(
            "SELECT COUNT(*) FROM pipeline_index_rebuild_fences WHERE tenant_id = $1",
            &[&fence_tenant],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(fence_rows, 0, "deleting the tenant removes its fence rows");

    PipelineCheckEmitter::emit_pass_from_env(
        "pipeline_storage_upgrade_rls",
        None,
        serde_json::json!({
            "tables": PIPELINE_TABLES.len(),
            "forced_rls": forced_rls_tables,
            "maximum_version": version,
        }),
    );
}

/// Zaki review 3, Z3-M3: a leg the V94-era code seeded with payout `pending`
/// has no batch line under the account's settlement key, so the payout never
/// pays it. V109 marks it `disabled` (a payout nothing will make) instead of
/// leaving it `pending` for good, for each instrument (multi-lens review C8:
/// that code seeded `pending` for each instrument on a payout rail, and the
/// payout reads only payout-eligible Trace Credit legs). A payout-eligible
/// leg keeps its state. V109 is applied here as a migrator that owns the
/// tables and is not a superuser, so the test fails when V109 does not lift
/// forced row security for its update.
#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL"]
async fn v109_disables_the_payout_of_pending_legs_the_v94_code_seeded() {
    v109_disables_v94_era_pending_legs(V109Order::Ascending).await;
}

/// `main` took V110 to V114 while 109 was free, so a database that runs
/// `main` records every other version before V109, and the runner applies
/// V109 last there (it applies each version that is not recorded, in the
/// order of the list). Here the runner itself (`run_migrations`) applies
/// V109, on a history that lacks only that version, in a session whose role
/// is the non-superuser owner. The same seeded legs and the same results as
/// the ascending test above; it fails when a later migration changes
/// something a V109 statement names, and when the runner leaves out a
/// version below the newest one it finds recorded.
#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL"]
async fn v109_applies_after_every_later_migration_on_a_database_that_runs_main() {
    v109_disables_v94_era_pending_legs(V109Order::AfterEveryLaterMigration).await;
}

/// When V109 reaches the database of `v109_disables_v94_era_pending_legs`.
#[derive(Clone, Copy, PartialEq)]
enum V109Order {
    /// After V108 and before V110: a database that goes from #1143 to a
    /// build with V109.
    Ascending,
    /// After every other migration of the list, by the migration runner.
    AfterEveryLaterMigration,
}

async fn v109_disables_v94_era_pending_legs(order: V109Order) {
    let _serial = UPGRADE_CLUSTER_LOCK.lock().await;
    // A database of its own beside the isolated one, so this upgrade starts
    // from nothing whatever else ran on that one. It is dropped at the end.
    let base = isolated_upgrade_database_url();
    let (prefix, base_name) = base.rsplit_once('/').expect("a database name");
    let (base_name, query) = base_name
        .split_once('?')
        .map_or((base_name, String::new()), |(name, query)| {
            (name, format!("?{query}"))
        });
    let name = match order {
        V109Order::Ascending => format!("{base_name}_v94_legs"),
        V109Order::AfterEveryLaterMigration => format!("{base_name}_v109_last"),
    };
    let (setup, connection) = tokio_postgres::connect(&base, tokio_postgres::NoTls)
        .await
        .expect("connect to the isolated database");
    tokio::spawn(async move { connection.await.expect("setup connection") });
    setup
        .batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
        .await
        .unwrap();
    setup
        .batch_execute(&format!("CREATE DATABASE {name}"))
        .await
        .unwrap();
    let url = format!("{prefix}/{name}{query}");
    let (mut admin, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("connect upgrade admin");
    let admin_connection =
        tokio::spawn(async move { connection.await.expect("upgrade connection") });
    apply_real_migrations_through(&mut admin, 104).await;

    let tenant = "upgrade-v94-legs";
    set_tenant(&admin, tenant).await;
    let submission_id = uuid::Uuid::new_v4();
    let run_id = uuid::Uuid::new_v4();
    let object_ref_id = uuid::Uuid::new_v4();
    let hash = |byte: &str| format!("sha256:{}", byte.repeat(64));
    let bundle_id = hash("c");
    admin
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
            &[&tenant],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO trace_submissions (
                tenant_id, submission_id, trace_id, auth_principal_ref, schema_version,
                consent_policy_version, consent_scopes, allowed_uses, retention_policy_id,
                status, privacy_risk, redaction_pipeline_version, redaction_hash,
                redaction_counts
             ) VALUES ($1, $2, $3, 'principal', 'ironclaw.trace_contribution.v1', 'v1',
                       '[]'::jsonb, '[]'::jsonb, 'retention-default', 'accepted', 'low', 'v1',
                       $4, '{}'::jsonb)",
            &[&tenant, &submission_id, &uuid::Uuid::new_v4(), &hash("d")],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO trace_object_refs (
                tenant_id, submission_id, object_ref_id, artifact_kind, object_store,
                object_key, content_sha256, encryption_key_ref, size_bytes
             ) VALUES ($1, $2, $3, 'submitted_envelope', 'store', 'key', $4, 'key-ref', 0)",
            &[&tenant, &submission_id, &object_ref_id, &hash("d")],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO pipeline_bundle_packages (tenant_id, bundle_id, manifest_format_version, package)
             VALUES ($1, $2, 1, '{}'::jsonb)",
            &[&tenant, &bundle_id],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO pipeline_runs (
                tenant_id, run_id, submission_id, trace_id, bundle_id,
                request_idempotency_key, request_content_hash, source_object_ref_id,
                next_phase, state, admission_decision
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'settle', 'pending', 'admit')",
            &[
                &tenant,
                &run_id,
                &submission_id,
                &uuid::Uuid::new_v4(),
                &bundle_id,
                &hash("e"),
                &hash("f"),
                &object_ref_id,
            ],
        )
        .await
        .unwrap();
    for (instrument, payout_rail, payout_state, operation_ref) in [
        ("trace_credit", "near", "pending", hash("1")),
        ("storage_rebate", "near", "pending", hash("2")),
        ("eligible_credit", "near", "pending", hash("3")),
    ] {
        admin
            .execute(
                "INSERT INTO pipeline_run_settlements (
                    tenant_id, run_id, instrument_id, atomic_units, operation_ref_hash,
                    payout_rail, payout_state
                 ) VALUES ($1, $2, $3, 1000, $4, $5, $6)",
                &[
                    &tenant,
                    &run_id,
                    &instrument,
                    &operation_ref,
                    &payout_rail,
                    &payout_state,
                ],
            )
            .await
            .unwrap();
    }

    // Multi-lens review C16: V109 is applied as a migrator that owns the
    // tables and is not a superuser, with no tenant set. Row security does
    // not apply to a superuser, so as the URL's role the update would reach
    // the legs with or without V109's lift of forced row security. V105 to
    // V108 are applied first, as the admin (and each later migration too,
    // when V109 goes last); the five tables V109 changes are then given to
    // the owner role.
    for (version, migration, sql) in MIGRATIONS.iter().filter(|(version, _, _)| match order {
        V109Order::Ascending => (105..=108).contains(version),
        V109Order::AfterEveryLaterMigration => *version >= 105 && *version != 109,
    }) {
        apply_and_record_migration(&mut admin, *version, migration, sql)
            .await
            .unwrap_or_else(|error| panic!("apply real V{version} ({migration}): {error}"));
    }
    if order == V109Order::AfterEveryLaterMigration {
        // The history a database of `main` has: the newest version of the
        // list is recorded, and V109 is the one version that is not.
        let missing: Vec<i32> = admin
            .query(
                "SELECT version FROM unnest($1::INTEGER[]) AS listed(version)
                  WHERE NOT EXISTS (
                        SELECT 1 FROM _trace_commons_migrations m
                         WHERE m.version = listed.version
                  )",
                &[&MIGRATIONS.iter().map(|(v, _, _)| *v).collect::<Vec<_>>()],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect();
        assert_eq!(missing, vec![109]);
        assert!(MIGRATIONS.iter().any(|(version, _, _)| *version > 109));
    }
    admin
        .batch_execute(
            "DO $$ BEGIN
                IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pipeline_upgrade_owner')
                THEN CREATE ROLE pipeline_upgrade_owner NOLOGIN NOSUPERUSER NOBYPASSRLS; END IF;
             END $$;
             GRANT USAGE, CREATE ON SCHEMA public TO pipeline_upgrade_owner;
             GRANT SELECT, INSERT ON _trace_commons_migrations TO pipeline_upgrade_owner;
             ALTER TABLE pipeline_run_settlements OWNER TO pipeline_upgrade_owner;
             ALTER TABLE pipeline_export_snapshots OWNER TO pipeline_upgrade_owner;
             ALTER TABLE pipeline_export_snapshot_items OWNER TO pipeline_upgrade_owner;
             ALTER TABLE pipeline_review_assessments OWNER TO pipeline_upgrade_owner;
             ALTER TABLE pipeline_index_invalidations OWNER TO pipeline_upgrade_owner;",
        )
        .await
        .expect("give V109's tables to a non-superuser owner");
    // A leg today's code seeds: `pending` and payout-eligible (V105).
    assert_eq!(
        admin
            .execute(
                "UPDATE pipeline_run_settlements SET payout_eligible = TRUE
                  WHERE tenant_id = $1 AND run_id = $2 AND instrument_id = 'eligible_credit'",
                &[&tenant, &run_id],
            )
            .await
            .unwrap(),
        1
    );
    set_tenant(&admin, "").await;
    let migrator = match order {
        V109Order::Ascending => {
            admin
                .batch_execute("SET ROLE pipeline_upgrade_owner")
                .await
                .unwrap();
            let (version, migration, sql) = MIGRATIONS
                .iter()
                .find(|(version, _, _)| *version == 109)
                .expect("V109 is in the list");
            apply_and_record_migration(&mut admin, *version, migration, sql)
                .await
                .expect("apply V109 as the non-superuser owner");
            admin.batch_execute("RESET ROLE").await.unwrap();
            // The runner applies the later migrations now.
            let migrator = PgBackend::new(&database_config(url.clone())).await.unwrap();
            migrator.run_migrations().await.expect("upgrade to current");
            migrator
        }
        V109Order::AfterEveryLaterMigration => {
            // Merge review M6: the runner applies V109, not the test. Each
            // new session of the URL's role in this database starts in the
            // owner role, so the runner's own connection is that of a
            // migrator that owns the tables and is not a superuser. The
            // setting goes with the database.
            admin
                .batch_execute(&format!(
                    "ALTER ROLE CURRENT_USER IN DATABASE {name} SET role = 'pipeline_upgrade_owner'"
                ))
                .await
                .expect("start the runner's sessions in the owner role");
            let migrator = PgBackend::new(&database_config(url.clone())).await.unwrap();
            let role = migrator
                .trace_pool()
                .get()
                .await
                .unwrap()
                .query_one(
                    "SELECT current_user::TEXT, rolsuper OR rolbypassrls
                       FROM pg_roles WHERE rolname = current_user",
                    &[],
                )
                .await
                .unwrap();
            assert_eq!(
                (role.get::<_, String>(0).as_str(), role.get::<_, bool>(1)),
                ("pipeline_upgrade_owner", false),
                "the runner's session is the non-superuser owner"
            );
            migrator
                .run_migrations()
                .await
                .expect("the runner applies V109 after every later migration");
            admin
                .batch_execute(&format!(
                    "ALTER ROLE CURRENT_USER IN DATABASE {name} RESET role"
                ))
                .await
                .unwrap();
            migrator
        }
    };
    let recorded: i64 = admin
        .query_one("SELECT COUNT(*) FROM _trace_commons_migrations", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(recorded, MIGRATIONS.len() as i64);
    set_tenant(&admin, tenant).await;
    let legs: Vec<(String, String, bool)> = admin
        .query(
            "SELECT instrument_id, payout_state, payout_eligible FROM pipeline_run_settlements
              WHERE tenant_id = $1 AND run_id = $2 ORDER BY instrument_id",
            &[&tenant, &run_id],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();
    assert_eq!(
        legs,
        vec![
            ("eligible_credit".to_string(), "pending".to_string(), true),
            ("storage_rebate".to_string(), "disabled".to_string(), false),
            ("trace_credit".to_string(), "disabled".to_string(), false),
        ]
    );
    let forced: bool = admin
        .query_one(
            "SELECT relforcerowsecurity FROM pg_class WHERE relname = 'pipeline_run_settlements'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(forced, "V109 forces row security again after its update");
    // The rest of V109 is there in either order: its four checks and its
    // two indexes.
    let objects: i64 = admin
        .query_one(
            "SELECT (SELECT COUNT(*) FROM pg_constraint
                      WHERE contype = 'c' AND conname::TEXT = ANY($1::TEXT[]))
                  + (SELECT COUNT(*) FROM pg_indexes
                      WHERE schemaname = 'public' AND indexname::TEXT = ANY($2::TEXT[]))",
            &[
                &[
                    "pipeline_export_snapshots_requester_principal_ref_check",
                    "pipeline_review_assessments_resolved_reasons_array",
                    "pipeline_export_snapshot_items_outcome_schema_id_shape",
                    "pipeline_export_snapshot_items_view_schema_id_shape",
                ]
                .as_slice(),
                &[
                    "idx_pipeline_index_invalidations_submission",
                    "idx_pipeline_export_snapshot_items_run",
                ]
                .as_slice(),
            ],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(objects, 6);

    // Multi-lens review C13: no run leaves this test's database on the
    // server. Its own connections are closed first.
    drop(migrator);
    drop(admin);
    admin_connection.await.expect("the admin connection closes");
    setup
        .batch_execute(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .await
        .expect("drop the test's own database");
    // PR #1283 review, finding 8: the owner role is cluster-wide, so it is
    // dropped too. Its objects and its per-database setting went with the
    // database. A role a crashed earlier run left with objects in another
    // database cannot be dropped; that is reported, not failed, since the
    // test's own work is done and the next run reuses the role.
    if let Err(error) = setup
        .batch_execute("DROP ROLE IF EXISTS pipeline_upgrade_owner")
        .await
    {
        eprintln!("pipeline_upgrade_owner was kept: {error}");
    }
}

/// V116 (spec 2026-10-08, Slice C, C-D4): `trace_gate_decisions` gains
/// `source` and `pipeline_run_id`, both defaulted so `main`'s writers are
/// unchanged; the two must agree (a pipeline row names its run, a legacy row
/// names none); one pipeline row at most per submission; and the gate
/// driver may read `source`, which the credit-quality sweep filters on.
#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL"]
async fn v116_adds_source_and_pipeline_run_id_with_defaults() {
    let _serial = UPGRADE_CLUSTER_LOCK.lock().await;
    let base = isolated_upgrade_database_url();
    let (prefix, base_name) = base.rsplit_once('/').expect("a database name");
    let (base_name, query) = base_name
        .split_once('?')
        .map_or((base_name, String::new()), |(name, query)| {
            (name, format!("?{query}"))
        });
    let name = format!("{base_name}_v116");
    let (setup, connection) = tokio_postgres::connect(&base, tokio_postgres::NoTls)
        .await
        .expect("connect to the isolated database");
    tokio::spawn(async move { connection.await.expect("setup connection") });
    setup
        .batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
        .await
        .unwrap();
    setup
        .batch_execute(&format!("CREATE DATABASE {name}"))
        .await
        .unwrap();
    let url = format!("{prefix}/{name}{query}");
    let (mut admin, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("connect upgrade admin");
    let admin_connection =
        tokio::spawn(async move { connection.await.expect("upgrade connection") });
    let last = MIGRATIONS
        .iter()
        .map(|(version, _, _)| *version)
        .max()
        .expect("a migration list");
    assert!(last >= 116, "V116 is registered in the migration list");
    apply_real_migrations_through(&mut admin, last).await;

    let tenant = "upgrade-v116";
    set_tenant(&admin, tenant).await;
    let hash = |byte: &str| format!("sha256:{}", byte.repeat(64));
    admin
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
            &[&tenant],
        )
        .await
        .unwrap();
    let submission_id = uuid::Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_submissions (
                tenant_id, submission_id, trace_id, auth_principal_ref, schema_version,
                consent_policy_version, consent_scopes, allowed_uses, retention_policy_id,
                status, privacy_risk, redaction_pipeline_version, redaction_hash,
                redaction_counts
             ) VALUES ($1, $2, $3, 'principal', 'ironclaw.trace_contribution.v1', 'v1',
                       '[]'::jsonb, '[]'::jsonb, 'retention-default', 'accepted', 'low', 'v1',
                       $4, '{}'::jsonb)",
            &[&tenant, &submission_id, &uuid::Uuid::new_v4(), &hash("d")],
        )
        .await
        .unwrap();
    // `main`'s column list, without the two new columns.
    let insert = |source: Option<&'static str>, run: Option<uuid::Uuid>| {
        let admin = &admin;
        async move {
            let decision_id = uuid::Uuid::new_v4();
            let base_columns = "tenant_id, decision_id, submission_id, gate_policy_version,
                 gate_version_hash, perplexity_micros, tail_fraction_micros,
                 perplexity_passed, novelty_score_micros, nearest_neighbor_hash,
                 novelty_passed, embedding_evidence_hash, attestation_chain_hash";
            let result = match source {
                None => {
                    admin
                        .execute(
                            &format!(
                                "INSERT INTO trace_gate_decisions ({base_columns})
                                 VALUES ($1,$2,$3,'v1','gv',1,0,true,1,'nn',true,'ee','aa')"
                            ),
                            &[&tenant, &decision_id, &submission_id],
                        )
                        .await
                }
                Some(source) => {
                    admin
                        .execute(
                            &format!(
                                "INSERT INTO trace_gate_decisions ({base_columns},
                                     source, pipeline_run_id)
                                 VALUES ($1,$2,$3,'v1','gv',1,0,true,1,'nn',true,'ee','aa',$4,$5)"
                            ),
                            &[&tenant, &decision_id, &submission_id, &source, &run],
                        )
                        .await
                }
            };
            result.map(|_| decision_id)
        }
    };

    // A legacy insert is unchanged and reads back as `legacy_gate` with no run.
    let legacy = insert(None, None)
        .await
        .expect("main's insert is unchanged");
    let row = admin
        .query_one(
            "SELECT source, pipeline_run_id FROM trace_gate_decisions
              WHERE tenant_id = $1 AND decision_id = $2",
            &[&tenant, &legacy],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "legacy_gate");
    assert_eq!(row.get::<_, Option<uuid::Uuid>>(1), None);
    // A second legacy row for the same submission is still legal (`Cached`).
    insert(None, None)
        .await
        .expect("main may write more than one row per submission");

    let check_violation = |error: tokio_postgres::Error| {
        assert_eq!(error.code(), Some(&SqlState::CHECK_VIOLATION), "{error}");
    };
    check_violation(
        insert(Some("pipeline_settle"), None)
            .await
            .expect_err("a pipeline row names its run"),
    );
    check_violation(
        insert(Some("legacy_gate"), Some(uuid::Uuid::new_v4()))
            .await
            .expect_err("a legacy row names no run"),
    );
    check_violation(
        insert(Some("other"), Some(uuid::Uuid::new_v4()))
            .await
            .expect_err("only the two sources exist"),
    );

    insert(Some("pipeline_settle"), Some(uuid::Uuid::new_v4()))
        .await
        .expect("one pipeline row for the submission");
    let error = insert(Some("pipeline_settle"), Some(uuid::Uuid::new_v4()))
        .await
        .expect_err("a second pipeline row for the submission is refused");
    assert_eq!(error.code(), Some(&SqlState::UNIQUE_VIOLATION), "{error}");

    let index: String = admin
        .query_one(
            "SELECT indexdef FROM pg_indexes
              WHERE tablename = 'trace_gate_decisions'
                AND indexname = 'trace_gate_decisions_one_pipeline_row'",
            &[],
        )
        .await
        .expect("the partial unique index exists")
        .get(0);
    assert!(index.starts_with("CREATE UNIQUE INDEX"), "{index}");
    assert!(index.contains("(tenant_id, submission_id)"), "{index}");
    assert!(index.contains("'pipeline_settle'"), "{index}");

    let readable: bool = admin
        .query_one(
            "SELECT has_column_privilege('trace_gate_driver', 'trace_gate_decisions',
                                         'source', 'SELECT')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(readable, "the gate driver reads `source`");

    drop(admin);
    admin_connection.await.expect("the admin connection closes");
    setup
        .batch_execute(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .await
        .expect("drop the test's own database");
}

/// V117 (spec 2026-10-09, async privacy rescrub): `pipeline_runs` gains the
/// Review-start privacy pass's record. The six pass columns are NULL on a run
/// that existed before V117 and are set together or not at all; the outcome
/// is `cleared` or `escalated`; the hashes are `sha256:` hex; the two approval
/// columns are set together and only on an escalated pass; the pass object's
/// key to `trace_object_refs` is deferred to commit; the ingest runtime may
/// update the eight columns; and `pipeline_attempt_artifacts` accepts a
/// `privacy-pass` row, which, like an `approved` row, always has its hash.
#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL"]
async fn v117_adds_the_privacy_pass_record() {
    let _serial = UPGRADE_CLUSTER_LOCK.lock().await;
    let base = isolated_upgrade_database_url();
    let (prefix, base_name) = base.rsplit_once('/').expect("a database name");
    let (base_name, query) = base_name
        .split_once('?')
        .map_or((base_name, String::new()), |(name, query)| {
            (name, format!("?{query}"))
        });
    let name = format!("{base_name}_v117");
    let (setup, connection) = tokio_postgres::connect(&base, tokio_postgres::NoTls)
        .await
        .expect("connect to the isolated database");
    tokio::spawn(async move { connection.await.expect("setup connection") });
    setup
        .batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
        .await
        .unwrap();
    setup
        .batch_execute(&format!("CREATE DATABASE {name}"))
        .await
        .unwrap();
    let url = format!("{prefix}/{name}{query}");
    let (mut admin, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("connect upgrade admin");
    let admin_connection =
        tokio::spawn(async move { connection.await.expect("upgrade connection") });
    apply_real_migrations_through(&mut admin, 116).await;

    // A run received before V117.
    let tenant = "upgrade-v117";
    set_tenant(&admin, tenant).await;
    let hash = |byte: &str| format!("sha256:{}", byte.repeat(64));
    let submission_id = uuid::Uuid::new_v4();
    let run_id = uuid::Uuid::new_v4();
    let source_ref_id = uuid::Uuid::new_v4();
    let bundle_id = hash("c");
    admin
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
            &[&tenant],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO trace_submissions (
                tenant_id, submission_id, trace_id, auth_principal_ref, schema_version,
                consent_policy_version, consent_scopes, allowed_uses, retention_policy_id,
                status, privacy_risk, redaction_pipeline_version, redaction_hash,
                redaction_counts
             ) VALUES ($1, $2, $3, 'principal', 'ironclaw.trace_contribution.v1', 'v1',
                       '[]'::jsonb, '[]'::jsonb, 'retention-default', 'accepted', 'low', 'v1',
                       $4, '{}'::jsonb)",
            &[&tenant, &submission_id, &uuid::Uuid::new_v4(), &hash("d")],
        )
        .await
        .unwrap();
    let insert_object_ref = |object_ref_id: uuid::Uuid, kind: &'static str, key: String| {
        let admin = &admin;
        let content = hash("d");
        async move {
            admin
                .execute(
                    "INSERT INTO trace_object_refs (
                        tenant_id, submission_id, object_ref_id, artifact_kind, object_store,
                        object_key, content_sha256, encryption_key_ref, size_bytes
                     ) VALUES ($1, $2, $3, $4, 'store', $5, $6, 'key-ref', 0)",
                    &[
                        &tenant,
                        &submission_id,
                        &object_ref_id,
                        &kind,
                        &key,
                        &content,
                    ],
                )
                .await
        }
    };
    insert_object_ref(source_ref_id, "submitted_envelope", "source-key".into())
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO pipeline_bundle_packages (tenant_id, bundle_id, manifest_format_version, package)
             VALUES ($1, $2, 1, '{}'::jsonb)",
            &[&tenant, &bundle_id],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO pipeline_runs (
                tenant_id, run_id, submission_id, trace_id, bundle_id,
                request_idempotency_key, request_content_hash, source_object_ref_id,
                next_phase, state, admission_decision
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'review', 'pending', 'admit')",
            &[
                &tenant,
                &run_id,
                &submission_id,
                &uuid::Uuid::new_v4(),
                &bundle_id,
                &hash("e"),
                &hash("f"),
                &source_ref_id,
            ],
        )
        .await
        .unwrap();

    for (version, migration, sql) in MIGRATIONS.iter().filter(|(version, _, _)| *version > 116) {
        apply_and_record_migration(&mut admin, *version, migration, sql)
            .await
            .unwrap_or_else(|error| panic!("apply real V{version} ({migration}): {error}"));
    }

    const PASS_COLUMNS: [&str; 6] = [
        "privacy_pass_object_ref_id",
        "privacy_pass_content_hash",
        "privacy_pass_source_hash",
        "privacy_pass_residual_risk_basis",
        "privacy_pass_outcome",
        "privacy_pass_recorded_at",
    ];
    const APPROVAL_COLUMNS: [&str; 2] = [
        "privacy_pass_approval_assessment_hash",
        "privacy_pass_approval_resolved_reasons",
    ];
    for column in PASS_COLUMNS.iter().chain(APPROVAL_COLUMNS.iter()) {
        let present: bool = admin
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM information_schema.columns
                                 WHERE table_name = 'pipeline_runs' AND column_name = $1)",
                &[column],
            )
            .await
            .unwrap()
            .get(0);
        assert!(present, "pipeline_runs.{column} must exist after V117");
        let null: bool = admin
            .query_one(
                &format!(
                    "SELECT {column} IS NULL FROM pipeline_runs
                      WHERE tenant_id = $1 AND run_id = $2"
                ),
                &[&tenant, &run_id],
            )
            .await
            .unwrap()
            .get(0);
        assert!(null, "{column} is NULL on a run received before V117");
    }

    let check_violation = |error: tokio_postgres::Error, constraint: &str| {
        assert_eq!(error.code(), Some(&SqlState::CHECK_VIOLATION), "{error}");
        let db_error = error.as_db_error().expect("a database error");
        assert_eq!(db_error.constraint(), Some(constraint), "{error}");
    };
    // Sets the six pass columns (each `None` is left NULL) and the two
    // approval columns, in one statement.
    let pass_ref_id = uuid::Uuid::new_v4();
    let update = |pass: [Option<String>; 6], approval: [Option<String>; 2]| {
        let admin = &admin;
        async move {
            let value = |index: usize, cast: &str| match &pass[index] {
                Some(value) => format!("{value}::{cast}"),
                None => "NULL".to_string(),
            };
            let approval_value = |index: usize, cast: &str| match &approval[index] {
                Some(value) => format!("{value}::{cast}"),
                None => "NULL".to_string(),
            };
            admin
                .execute(
                    &format!(
                        "UPDATE pipeline_runs SET
                            privacy_pass_object_ref_id = {},
                            privacy_pass_content_hash = {},
                            privacy_pass_source_hash = {},
                            privacy_pass_residual_risk_basis = {},
                            privacy_pass_outcome = {},
                            privacy_pass_recorded_at = {},
                            privacy_pass_approval_assessment_hash = {},
                            privacy_pass_approval_resolved_reasons = {}
                          WHERE tenant_id = $1 AND run_id = $2",
                        value(0, "uuid"),
                        value(1, "text"),
                        value(2, "text"),
                        value(3, "jsonb"),
                        value(4, "text"),
                        value(5, "timestamptz"),
                        approval_value(0, "text"),
                        approval_value(1, "jsonb"),
                    ),
                    &[&tenant, &run_id],
                )
                .await
        }
    };
    let quoted = |value: &str| format!("'{value}'");
    let full_pass = |outcome: &str| -> [Option<String>; 6] {
        [
            Some(quoted(&pass_ref_id.to_string())),
            Some(quoted(&hash("1"))),
            Some(quoted(&hash("2"))),
            Some(quoted(r#"["found_and_removed"]"#)),
            Some(quoted(outcome)),
            Some("NOW()".to_string()),
        ]
    };
    let no_approval = || -> [Option<String>; 2] { [None, None] };

    // The six pass columns are set together or not at all.
    for missing in 0..PASS_COLUMNS.len() {
        let mut pass = full_pass("cleared");
        pass[missing] = None;
        check_violation(
            update(pass, no_approval())
                .await
                .expect_err("a pass record with a column missing is refused"),
            "pipeline_runs_privacy_pass_shape",
        );
    }
    check_violation(
        update(full_pass("other"), no_approval())
            .await
            .expect_err("only `cleared` and `escalated` are outcomes"),
        "pipeline_runs_privacy_pass_outcome_check",
    );
    let mut malformed = full_pass("cleared");
    malformed[1] = Some(quoted("sha256:XYZ"));
    check_violation(
        update(malformed, no_approval())
            .await
            .expect_err("a malformed content hash is refused"),
        "pipeline_runs_privacy_pass_content_hash_check",
    );
    let mut malformed = full_pass("cleared");
    malformed[2] = Some(quoted(&"a".repeat(64)));
    check_violation(
        update(malformed, no_approval())
            .await
            .expect_err("a source hash without its prefix is refused"),
        "pipeline_runs_privacy_pass_source_hash_check",
    );
    let mut not_an_array = full_pass("cleared");
    not_an_array[3] = Some(quoted(r#"{"label": 1}"#));
    check_violation(
        update(not_an_array, no_approval())
            .await
            .expect_err("the basis is a JSON array"),
        "pipeline_runs_privacy_pass_residual_risk_basis_check",
    );

    // The two approval columns go together, and only on an escalated pass.
    let approval = || -> [Option<String>; 2] {
        [
            Some(quoted(&hash("3"))),
            Some(quoted(r#"["privacy_pass_review_required"]"#)),
        ]
    };
    for missing in 0..APPROVAL_COLUMNS.len() {
        let mut half = approval();
        half[missing] = None;
        check_violation(
            update(full_pass("escalated"), half)
                .await
                .expect_err("half an approval link is refused"),
            "pipeline_runs_privacy_pass_approval_shape",
        );
    }
    check_violation(
        update(full_pass("cleared"), approval())
            .await
            .expect_err("a cleared pass has no approval link"),
        "pipeline_runs_privacy_pass_approval_shape",
    );
    check_violation(
        update([None, None, None, None, None, None], approval())
            .await
            .expect_err("no approval link without a pass"),
        "pipeline_runs_privacy_pass_approval_shape",
    );
    let mut malformed_approval = approval();
    malformed_approval[0] = Some(quoted("sha256:short"));
    check_violation(
        update(full_pass("escalated"), malformed_approval)
            .await
            .expect_err("a malformed approval hash is refused"),
        "pipeline_runs_privacy_pass_approval_assessment_hash_check",
    );

    // The key to the pass object is checked at commit: the run may name the
    // ref before the same transaction inserts it.
    // (Explicit BEGIN/COMMIT, so the closures above keep their shared
    // borrow of the client.)
    {
        admin.batch_execute("BEGIN").await.unwrap();
        admin
            .execute(
                "UPDATE pipeline_runs SET
                privacy_pass_object_ref_id = $3, privacy_pass_content_hash = $4,
                privacy_pass_source_hash = $5,
                privacy_pass_residual_risk_basis = '[\"found_and_removed\"]'::jsonb,
                privacy_pass_outcome = 'escalated', privacy_pass_recorded_at = NOW()
              WHERE tenant_id = $1 AND run_id = $2",
                &[&tenant, &run_id, &pass_ref_id, &hash("1"), &hash("2")],
            )
            .await
            .expect("the run names the pass object before its ref exists");
        admin
            .execute(
                "INSERT INTO trace_object_refs (
                tenant_id, submission_id, object_ref_id, artifact_kind, object_store,
                object_key, content_sha256, encryption_key_ref, size_bytes
             ) VALUES ($1, $2, $3, 'review_snapshot', 'store', 'pass-key', $4, 'key-ref', 0)",
                &[&tenant, &submission_id, &pass_ref_id, &hash("1")],
            )
            .await
            .unwrap();
        admin
            .batch_execute("COMMIT")
            .await
            .expect("the deferred key holds at commit");
    }
    // A pass object no ref names fails at commit, not at the UPDATE.
    {
        admin.batch_execute("BEGIN").await.unwrap();
        admin
            .execute(
                "UPDATE pipeline_runs SET privacy_pass_object_ref_id = $3
              WHERE tenant_id = $1 AND run_id = $2",
                &[&tenant, &run_id, &uuid::Uuid::new_v4()],
            )
            .await
            .expect("the key is deferred, so the UPDATE itself succeeds");
        let error = admin
            .batch_execute("COMMIT")
            .await
            .expect_err("a pass object with no ref is refused at commit");
        assert_eq!(
            error.code(),
            Some(&SqlState::FOREIGN_KEY_VIOLATION),
            "{error}"
        );
    }
    let fk = admin
        .query_one(
            "SELECT condeferrable, condeferred, confdeltype::text
               FROM pg_constraint
              WHERE conrelid = 'pipeline_runs'::regclass
                AND conname = 'pipeline_runs_privacy_pass_object_ref_fk'
                AND contype = 'f'",
            &[],
        )
        .await
        .expect("the pass object's key exists");
    assert!(
        fk.get::<_, bool>(0) && fk.get::<_, bool>(1),
        "the pass object's key is DEFERRABLE INITIALLY DEFERRED"
    );
    assert_eq!(fk.get::<_, String>(2), "a", "ON DELETE NO ACTION");

    // An escalated pass takes the approval link.
    update(full_pass("escalated"), approval())
        .await
        .expect("an escalated pass with both approval columns");

    // The ingest runtime updates the eight columns.
    for column in PASS_COLUMNS.iter().chain(APPROVAL_COLUMNS.iter()) {
        let granted: bool = admin
            .query_one(
                "SELECT has_column_privilege('trace_ingest_runtime', 'pipeline_runs', $1, 'UPDATE')",
                &[column],
            )
            .await
            .unwrap()
            .get(0);
        assert!(granted, "trace_ingest_runtime may update {column}");
    }

    // pipeline_attempt_artifacts: a `privacy-pass` row is a known artifact
    // and, like `approved`, never lacks its hash.
    let stage = |artifact: &'static str, ciphertext: Option<String>| {
        let admin = &admin;
        async move {
            admin
                .execute(
                    "INSERT INTO pipeline_attempt_artifacts (
                        tenant_id, run_id, lease_token, artifact, object_key,
                        ciphertext_sha256, cleanup_after
                     ) VALUES ($1, $2, $3, $4, $5, $6, NOW())",
                    &[
                        &tenant,
                        &run_id,
                        &uuid::Uuid::new_v4(),
                        &artifact,
                        &format!("attempt-{}", uuid::Uuid::new_v4()),
                        &ciphertext,
                    ],
                )
                .await
        }
    };
    stage("privacy-pass", Some("b".repeat(64)))
        .await
        .expect("a privacy-pass row with its hash");
    check_violation(
        stage("privacy-pass", None)
            .await
            .expect_err("a privacy-pass row without its hash is refused"),
        "pipeline_attempt_artifacts_approved_hash",
    );
    check_violation(
        stage("approved", None)
            .await
            .expect_err("an approved row still needs its hash"),
        "pipeline_attempt_artifacts_approved_hash",
    );
    stage("score-neighbors", None)
        .await
        .expect("a compatibility Score row may still be staged without a hash");
    check_violation(
        stage("other", Some("b".repeat(64)))
            .await
            .expect_err("an unknown artifact is refused"),
        "pipeline_attempt_artifacts_artifact_check",
    );

    drop(admin);
    admin_connection.await.expect("the admin connection closes");
    setup
        .batch_execute(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .await
        .expect("drop the test's own database");
}
