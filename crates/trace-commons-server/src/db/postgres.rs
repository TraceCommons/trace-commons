//! PostgreSQL backend for TraceCommons server storage.

use std::collections::HashSet;

use async_trait::async_trait;
use deadpool_postgres::Pool;

use crate::config::DatabaseConfig;
use crate::db::{Database, TraceCorpusRlsDiagnostics};
use crate::error::DatabaseError;

pub struct PgBackend {
    pool: Pool,
}

const TRACE_COMMONS_RLS_TABLES: &[&str] = &[
    "trace_tenants",
    "trace_tenant_policies",
    "trace_tenant_access_grants",
    "trace_submissions",
    "trace_object_refs",
    "trace_derived_records",
    "trace_audit_events",
    "trace_credit_ledger",
    "trace_tombstones",
    "trace_vector_entries",
    "trace_export_manifests",
    "trace_export_manifest_items",
    "trace_retention_jobs",
    "trace_retention_job_items",
    "trace_export_access_grants",
    "trace_export_jobs",
    "trace_revocation_propagation_items",
    "trace_utility_attestations",
    "trace_credit_settlement_batches",
    "trace_credit_holds",
    "trace_near_credit_outbox",
    "trace_benchmark_registry_outbox",
    "trace_ranking_model_versions",
    "trace_ranking_calibration_datasets",
    "trace_ranking_features",
    "trace_ranking_predictions",
    "trace_ranking_labels",
    "trace_ranking_preference_labels",
    "trace_ranking_calibration_runs",
    "trace_ranking_worker_runs",
];

const TRACE_COMMONS_RLS_POLICY_EXPRESSION_VARIANTS: &[&str] = &[
    "(tenant_id = trace_current_tenant_id())",
    "(tenant_id = public.trace_current_tenant_id())",
];

impl PgBackend {
    pub async fn new(config: &DatabaseConfig) -> Result<Self, DatabaseError> {
        let pg_config = config
            .url()
            .parse::<tokio_postgres::Config>()
            .map_err(|e| DatabaseError::Pool(format!("invalid PostgreSQL URL: {e}")))?;
        let manager = deadpool_postgres::Manager::new(pg_config, tokio_postgres::NoTls);
        let pool = Pool::builder(manager).max_size(config.pool_size).build()?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> Pool {
        self.pool.clone()
    }
}

#[async_trait]
impl Database for PgBackend {
    async fn run_migrations(&self) -> Result<(), DatabaseError> {
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e.to_string()))?;
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS _trace_commons_migrations (
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                );",
            )
            .await?;
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&1_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V1__trace_commons_schema.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&1_i32, &"trace_commons_schema"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&2_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V2__trace_credit_settlement.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&2_i32, &"trace_credit_settlement"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&3_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V3__trace_ranking_evidence.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&3_i32, &"trace_ranking_evidence"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&4_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V4__trace_ranking_calibration_runs.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&4_i32, &"trace_ranking_calibration_runs"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&5_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V5__trace_credit_settlement_ranking_gate.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&5_i32, &"trace_credit_settlement_ranking_gate"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&6_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V6__trace_force_rls.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&6_i32, &"trace_force_rls"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&7_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V7__trace_ranking_calibration_label_source_gate.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&7_i32, &"trace_ranking_calibration_label_source_gate"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&8_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V8__trace_ranking_calibration_source_error_gate.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&8_i32, &"trace_ranking_calibration_source_error_gate"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&9_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V9__trace_ranking_calibration_joined_evidence_hash.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&9_i32, &"trace_ranking_calibration_joined_evidence_hash"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&10_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V10__trace_credit_settlement_joined_evidence_hash.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&10_i32, &"trace_credit_settlement_joined_evidence_hash"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&11_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V11__trace_ranking_worker_runs.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&11_i32, &"trace_ranking_worker_runs"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&12_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V12__trace_ranking_worker_run_lifecycle.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&12_i32, &"trace_ranking_worker_run_lifecycle"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&13_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V13__trace_credit_settlement_exclusion_reasons.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&13_i32, &"trace_credit_settlement_exclusion_reasons"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&14_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V14__trace_ranking_preference_labels.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&14_i32, &"trace_ranking_preference_labels"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&15_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V15__trace_benchmark_registry_outbox.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&15_i32, &"trace_benchmark_registry_outbox"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&16_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V16__trace_ranking_calibration_datasets.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&16_i32, &"trace_ranking_calibration_datasets"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&17_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V17__trace_ranking_calibration_dataset_manifest_immutability.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[
                        &17_i32,
                        &"trace_ranking_calibration_dataset_manifest_immutability",
                    ],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&18_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V18__trace_central_rls_tenant_predicate.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&18_i32, &"trace_central_rls_tenant_predicate"],
                )
                .await?;
        }
        Ok(())
    }

    async fn trace_corpus_rls_diagnostics(
        &self,
    ) -> Result<Option<TraceCorpusRlsDiagnostics>, DatabaseError> {
        let mut client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e.to_string()))?;
        let expected_tables = TRACE_COMMONS_RLS_TABLES
            .iter()
            .map(|table| (*table).to_string())
            .collect::<Vec<_>>();
        let expected_policy_expressions = TRACE_COMMONS_RLS_POLICY_EXPRESSION_VARIANTS
            .iter()
            .map(|expression| (*expression).to_string())
            .collect::<Vec<_>>();
        let rows = client
            .query(
                "SELECT
                    c.relname,
                    c.relrowsecurity,
                    c.relforcerowsecurity,
                    p.has_policy,
                    COALESCE(p.expression_matches, false) AS expression_matches
                 FROM pg_class c
                 JOIN pg_namespace n ON n.oid = c.relnamespace
                 LEFT JOIN LATERAL (
                    SELECT
                        true AS has_policy,
                        pol.cmd = '*'
                            AND pg_get_expr(pol.qual, pol.polrelid) = ANY($2)
                            AND pg_get_expr(pol.with_check, pol.polrelid) = ANY($2)
                            AS expression_matches
                        FROM pg_policies p
                        JOIN pg_policy pol
                          ON pol.polname = p.policyname
                         AND pol.polrelid = c.oid
                        WHERE p.schemaname = n.nspname
                          AND p.tablename = c.relname
                          AND p.policyname = 'trace_corpus_tenant_isolation'
                        LIMIT 1
                 ) p ON true
                 WHERE n.nspname = current_schema()
                   AND c.relkind = 'r'
                   AND c.relname = ANY($1)",
                &[&expected_tables, &expected_policy_expressions],
            )
            .await?;
        let current_role = client
            .query_one(
                "SELECT
                    EXISTS (
                        SELECT 1
                        FROM pg_class c
                        JOIN pg_namespace n ON n.oid = c.relnamespace
                        JOIN pg_roles r ON r.oid = c.relowner
                        WHERE n.nspname = current_schema()
                          AND c.relkind = 'r'
                          AND c.relname = ANY($1)
                          AND r.rolname = current_user
                          AND NOT c.relforcerowsecurity
                    ) AS owns_unforced_trace_tables,
                    COALESCE((
                        SELECT rolsuper OR rolbypassrls
                        FROM pg_roles
                        WHERE rolname = current_user
                    ), false) AS bypass_role",
                &[&expected_tables],
            )
            .await?;

        let mut seen_tables = HashSet::new();
        let mut rls_enabled_count = 0usize;
        let mut force_rls_enabled_count = 0usize;
        let mut policy_installed_count = 0usize;
        let mut rls_disabled_tables = Vec::new();
        let mut force_rls_disabled_tables = Vec::new();
        let mut missing_policy_tables = Vec::new();
        let mut policy_expression_mismatch_tables = Vec::new();
        for row in rows {
            let table: String = row.get("relname");
            let rls_enabled: bool = row.get("relrowsecurity");
            let force_rls_enabled: bool = row.get("relforcerowsecurity");
            let has_policy: bool = row.get("has_policy");
            let expression_matches: bool = row.get("expression_matches");
            seen_tables.insert(table.clone());
            if rls_enabled {
                rls_enabled_count += 1;
            } else {
                rls_disabled_tables.push(table.clone());
            }
            if force_rls_enabled {
                force_rls_enabled_count += 1;
            } else {
                force_rls_disabled_tables.push(table.clone());
            }
            if has_policy {
                policy_installed_count += 1;
                if !expression_matches {
                    policy_expression_mismatch_tables.push(table.clone());
                }
            } else {
                missing_policy_tables.push(table.clone());
            }
        }
        for table in &expected_tables {
            if !seen_tables.contains(table) {
                missing_policy_tables.push(table.clone());
                rls_disabled_tables.push(table.clone());
                force_rls_disabled_tables.push(table.clone());
            }
        }
        missing_policy_tables.sort();
        missing_policy_tables.dedup();
        rls_disabled_tables.sort();
        rls_disabled_tables.dedup();
        force_rls_disabled_tables.sort();
        force_rls_disabled_tables.dedup();
        policy_expression_mismatch_tables.sort();
        policy_expression_mismatch_tables.dedup();

        let owns_unforced_trace_tables: bool = current_role.get("owns_unforced_trace_tables");
        let bypass_role: bool = current_role.get("bypass_role");
        let tenant_context_transaction_local =
            trace_tenant_context_is_transaction_local(&mut client).await?;
        Ok(Some(TraceCorpusRlsDiagnostics {
            expected_table_count: expected_tables.len(),
            rls_enabled_count,
            force_rls_enabled_count,
            policy_installed_count,
            missing_policy_tables,
            rls_disabled_tables,
            force_rls_disabled_tables,
            policy_expression_mismatch_tables,
            current_role_bypasses_rls: owns_unforced_trace_tables || bypass_role,
            tenant_context_transaction_local,
        }))
    }
}

async fn trace_tenant_context_is_transaction_local(
    client: &mut deadpool_postgres::Client,
) -> Result<bool, DatabaseError> {
    let tx = client.transaction().await?;
    let probe_tenant = "__trace_rls_probe_tenant__";
    tx.execute(
        "SELECT set_config('trace-commons.trace_tenant_id', $1, true)",
        &[&probe_tenant],
    )
    .await?;
    let inside = tx
        .query_one(
            "SELECT current_setting('trace-commons.trace_tenant_id', true) AS tenant_context",
            &[],
        )
        .await?
        .get::<_, Option<String>>("tenant_context");
    tx.commit().await?;
    let after = client
        .query_one(
            "SELECT current_setting('trace-commons.trace_tenant_id', true) AS tenant_context",
            &[],
        )
        .await?
        .get::<_, Option<String>>("tenant_context");
    Ok(inside.as_deref() == Some(probe_tenant) && after.as_deref().is_none_or(str::is_empty))
}
