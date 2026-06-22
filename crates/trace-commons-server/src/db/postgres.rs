//! PostgreSQL backend for TraceCommons server storage.

use std::collections::HashSet;

use async_trait::async_trait;
use deadpool_postgres::Pool;
use sha2::{Digest, Sha256};
use tokio_postgres::Row;
use uuid::Uuid;

use crate::config::DatabaseConfig;
use crate::db::{Database, TraceCorpusRlsDiagnostics};
use crate::error::DatabaseError;

pub struct PgBackend {
    pool: Pool,
    /// Narrow, SEPARATE pool for the unauthenticated login-link redeem path.
    /// Built only when `login_resolver_url` is configured; its DB user is the
    /// operator-provisioned `trace_login_resolver` role (no BYPASSRLS,
    /// column-scoped SELECT on `trace_login_links` only). `None` keeps the
    /// account-redeem path fail-closed. NEVER aliased to `pool`.
    login_resolver_pool: Option<Pool>,
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
    "trace_near_credit_account_outbox",
    "trace_benchmark_registry_outbox",
    "trace_ranking_model_versions",
    "trace_ranking_calibration_datasets",
    "trace_ranking_features",
    "trace_ranking_predictions",
    "trace_ranking_labels",
    "trace_ranking_preference_labels",
    "trace_ranking_calibration_runs",
    "trace_ranking_worker_runs",
    "trace_contributor_profiles",
    "trace_contributor_profile_audit",
    "device_keys",
    "onboarding_invites",
    "trace_accounts",
    "trace_account_principals",
    "trace_login_links",
    "trace_sessions",
    "trace_account_audit",
];

const TRACE_COMMONS_RLS_POLICY_EXPRESSION_VARIANTS: &[&str] = &[
    "(tenant_id = trace_current_tenant_id())",
    "(tenant_id = public.trace_current_tenant_id())",
];

const ONBOARDING_DEVICE_GRANT_REASON: &str = "onboarding device-key default pilot access";

const LEADERBOARD_INPUTS_SQL: &str = "SELECT
                        cp.tenant_id,
                        cp.principal_ref,
                        cp.display_handle,
                        cp.handle_normalized,
                        cp.bio,
                        cp.public_since,
                        COUNT(*) FILTER (
                            WHERE cl.event_type = 'accepted'
                              AND COALESCE(ts.received_at, cl.occurred_at)
                                  >= NOW() - ($1 || ' days')::interval
                        ) AS accepted_in_window,
                        COALESCE(SUM(cl.points_delta::float8) FILTER (
                            WHERE cl.event_type = 'accepted'
                              AND COALESCE(ts.received_at, cl.occurred_at)
                                  >= NOW() - ($1 || ' days')::interval
                        ), 0.0) AS credit_in_window,
                        COUNT(*) FILTER (WHERE cl.event_type = 'accepted') AS total_accepted,
                        COALESCE(SUM(cl.points_delta::float8) FILTER (
                            WHERE cl.event_type = 'accepted'
                        ), 0.0) AS total_credit
                     FROM trace_contributor_profiles cp
                     LEFT JOIN trace_credit_ledger cl
                            ON cl.tenant_id = cp.tenant_id
                           AND (
                                cl.credit_account_ref = cp.principal_ref
                                OR EXISTS (
                                    SELECT 1
                                    FROM trace_submissions ts_match
                                    WHERE ts_match.tenant_id = cl.tenant_id
                                      AND ts_match.submission_id = cl.submission_id
                                      AND (
                                           ts_match.auth_principal_ref = cp.principal_ref
                                           OR COALESCE(
                                                ts_match.contributor_pseudonym,
                                                ts_match.auth_principal_ref
                                           ) = cp.principal_ref
                                      )
                                )
                           )
                     LEFT JOIN trace_submissions ts
                            ON ts.tenant_id = cl.tenant_id
                           AND ts.submission_id = cl.submission_id
                     WHERE cp.withdrawn_at IS NULL
                     GROUP BY cp.tenant_id, cp.principal_ref, cp.display_handle,
                              cp.handle_normalized, cp.bio, cp.public_since
                     HAVING COUNT(*) FILTER (
                        WHERE cl.event_type = 'accepted'
                          AND COALESCE(ts.received_at, cl.occurred_at)
                              >= NOW() - ($1 || ' days')::interval
                     ) >= $2";

impl PgBackend {
    pub async fn new(config: &DatabaseConfig) -> Result<Self, DatabaseError> {
        let pg_config = config
            .url()
            .parse::<tokio_postgres::Config>()
            .map_err(|e| DatabaseError::Pool(format!("invalid PostgreSQL URL: {e}")))?;
        let manager = deadpool_postgres::Manager::new(pg_config, tokio_postgres::NoTls);
        let pool = Pool::builder(manager).max_size(config.pool_size).build()?;

        // Build a SEPARATE, small resolver pool only when a distinct resolver
        // connection string is configured. This pool runs as the narrow
        // `trace_login_resolver` role and is never aliased to the runtime pool.
        let login_resolver_pool = match config.login_resolver_url() {
            Some(resolver_url) => {
                let resolver_config = resolver_url
                    .parse::<tokio_postgres::Config>()
                    .map_err(|e| {
                        DatabaseError::Pool(format!("invalid login-resolver PostgreSQL URL: {e}"))
                    })?;
                let resolver_manager =
                    deadpool_postgres::Manager::new(resolver_config, tokio_postgres::NoTls);
                let resolver_pool = Pool::builder(resolver_manager).max_size(2).build()?;
                Some(resolver_pool)
            }
            None => None,
        };

        Ok(Self {
            pool,
            login_resolver_pool,
        })
    }

    pub(crate) fn trace_pool(&self) -> Pool {
        self.pool.clone()
    }

    #[doc(hidden)]
    pub fn raw_pool_for_tests_and_diagnostics(&self) -> Pool {
        self.pool.clone()
    }

    /// Resolve the tenant for a login code via the NARROW resolver pool (separate
    /// role, column-scoped SELECT, no BYPASSRLS). Returns the tenant only; the
    /// caller MUST re-confirm tenant inside an RLS-scoped transaction before any
    /// write. Fail-closed: if the resolver pool is not configured, this errors
    /// with a safe missing-control name rather than falling back to the runtime
    /// pool.
    pub async fn resolve_login_link_tenant(
        &self,
        code_hash: &str,
    ) -> anyhow::Result<Option<String>> {
        let pool = self
            .login_resolver_pool
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing-control: login-resolver-pool-unconfigured"))?;
        let client = pool.get().await?;
        // Safe without a tenant predicate: code_hash is globally UNIQUE (CHECK-shaped sha256) so
        // this returns at most one row across all tenants; the redeem handler re-confirms tenant
        // inside an RLS-scoped tx before any write. Do NOT add a non-unique lookup column to this
        // role's grant.
        let row = client
            .query_opt(
                "SELECT tenant_id FROM trace_login_links WHERE code_hash = $1",
                &[&code_hash],
            )
            .await?;
        Ok(row.map(|r| r.get::<_, String>(0)))
    }
}

#[async_trait]
impl Database for PgBackend {
    async fn run_migrations(&self) -> Result<(), DatabaseError> {
        let client = self
            .trace_pool()
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
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&19_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V19__trace_ranking_calibration_label_actor_count.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&19_i32, &"trace_ranking_calibration_label_actor_count"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&20_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V20__trace_credit_settlement_issuer_approval_hash.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&20_i32, &"trace_credit_settlement_issuer_approval_hash"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&21_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V21__trace_near_credit_account_outbox.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&21_i32, &"trace_near_credit_account_outbox"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&22_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V22__trace_revocation_worker_queue_invalidation.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&22_i32, &"trace_revocation_worker_queue_invalidation"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&23_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V23__novelty_utility_credit_and_gate_decisions.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&23_i32, &"novelty_utility_credit_and_gate_decisions"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&24_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V24__gate_decision_vector_entry_id.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&24_i32, &"gate_decision_vector_entry_id"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&25_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V25__gate_decision_credit_withheld_reason.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&25_i32, &"gate_decision_credit_withheld_reason"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&26_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V26__trace_contributor_profiles.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&26_i32, &"trace_contributor_profiles"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&27_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V27__trace_leaderboard_snapshots.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&27_i32, &"trace_leaderboard_snapshots"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&28_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!("../../../../migrations/V28__device_keys.sql"))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&28_i32, &"device_keys"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&29_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V29__onboarding_invites.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&29_i32, &"onboarding_invites"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&30_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V30__trace_accounts.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&30_i32, &"trace_accounts"],
                )
                .await?;
        }
        Ok(())
    }

    async fn trace_corpus_rls_diagnostics(
        &self,
    ) -> Result<Option<TraceCorpusRlsDiagnostics>, DatabaseError> {
        let mut client = self
            .trace_pool()
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
                    current_user AS current_role_name,
                    EXISTS (
                        SELECT 1
                        FROM pg_class c
                        JOIN pg_namespace n ON n.oid = c.relnamespace
                        JOIN pg_roles r ON r.oid = c.relowner
                        WHERE n.nspname = current_schema()
                          AND c.relkind = 'r'
                          AND c.relname = ANY($1)
                          AND r.rolname = current_user
                    ) AS owns_trace_tables,
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

        let current_role_name: String = current_role.get("current_role_name");
        let owns_unforced_trace_tables: bool = current_role.get("owns_unforced_trace_tables");
        let owns_trace_tables: bool = current_role.get("owns_trace_tables");
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
            current_role_hash: sha256_prefixed(&current_role_name),
            current_role_bypasses_rls: owns_unforced_trace_tables || bypass_role,
            current_role_owns_trace_tables: owns_trace_tables,
            tenant_context_transaction_local,
        }))
    }

    async fn upsert_contributor_profile(
        &self,
        tenant_id: &str,
        principal_ref: &str,
        display_handle: &str,
        handle_normalized: &str,
        bio: Option<&str>,
    ) -> Result<crate::db::ContributorProfileRow, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let bio_opt: Option<&str> = bio;
        let row = tx
            .query_one(
                "INSERT INTO trace_contributor_profiles (
                    tenant_id, principal_ref, display_handle, handle_normalized, bio
                 ) VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (tenant_id, principal_ref) DO UPDATE SET
                    display_handle = excluded.display_handle,
                    handle_normalized = excluded.handle_normalized,
                    bio = excluded.bio,
                    last_updated_at = NOW(),
                    update_count = trace_contributor_profiles.update_count + 1,
                    withdrawn_at = NULL
                 RETURNING tenant_id, principal_ref, display_handle, handle_normalized,
                           bio, public_since, last_updated_at, update_count",
                &[
                    &tenant_id,
                    &principal_ref,
                    &display_handle,
                    &handle_normalized,
                    &bio_opt,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(crate::db::ContributorProfileRow {
            tenant_id: row.get("tenant_id"),
            principal_ref: row.get("principal_ref"),
            display_handle: row.get("display_handle"),
            handle_normalized: row.get("handle_normalized"),
            bio: row.get("bio"),
            public_since: row.get("public_since"),
            last_updated_at: row.get("last_updated_at"),
            update_count: row.get("update_count"),
        })
    }

    async fn withdraw_contributor_profile(
        &self,
        tenant_id: &str,
        principal_ref: &str,
    ) -> Result<bool, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let affected = tx
            .execute(
                "UPDATE trace_contributor_profiles
                    SET withdrawn_at = NOW(), last_updated_at = NOW()
                  WHERE tenant_id = $1 AND principal_ref = $2 AND withdrawn_at IS NULL",
                &[&tenant_id, &principal_ref],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(affected > 0)
    }

    async fn append_contributor_profile_audit(
        &self,
        tenant_id: &str,
        principal_ref: &str,
        action: &str,
        handle_normalized: Option<&str>,
        reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "INSERT INTO trace_contributor_profile_audit (
                tenant_id, principal_ref, action, handle_normalized, reason
             ) VALUES ($1, $2, $3, $4, $5)",
            &[
                &tenant_id,
                &principal_ref,
                &action,
                &handle_normalized,
                &reason,
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn compute_leaderboard_inputs(
        &self,
        window_days: i32,
        min_cell_count: i64,
        configured_tenant_ids: &[String],
    ) -> Result<Vec<crate::db::LeaderboardContributorRow>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tenant_ids: Vec<String> = if configured_tenant_ids.is_empty() {
            client
                .query("SELECT tenant_id FROM trace_tenants", &[])
                .await
                .map_err(DatabaseError::Postgres)?
                .into_iter()
                .map(|row| row.get::<_, String>("tenant_id"))
                .collect()
        } else {
            configured_tenant_ids.to_vec()
        };
        let mut rows = Vec::new();
        for tenant_id in &tenant_ids {
            let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
            // We bind `window_days` via interval arithmetic in SQL so the
            // window is consistent across rows even if the transaction
            // straddles a clock tick.
            let pg_rows = tx
                .query(
                    LEADERBOARD_INPUTS_SQL,
                    &[&window_days.to_string(), &min_cell_count],
                )
                .await
                .map_err(DatabaseError::Postgres)?;
            tx.commit().await.map_err(DatabaseError::Postgres)?;
            for pg_row in pg_rows {
                rows.push(crate::db::LeaderboardContributorRow {
                    tenant_id: pg_row.get("tenant_id"),
                    principal_ref: pg_row.get("principal_ref"),
                    display_handle: pg_row.get("display_handle"),
                    handle_normalized: pg_row.get("handle_normalized"),
                    bio: pg_row.get("bio"),
                    public_since: pg_row.get("public_since"),
                    accepted_in_window: pg_row.get("accepted_in_window"),
                    credit_in_window: pg_row.get("credit_in_window"),
                    total_accepted: pg_row.get("total_accepted"),
                    total_credit: pg_row.get("total_credit"),
                });
            }
        }
        Ok(rows)
    }

    async fn compute_corpus_analytics_summary(
        &self,
        window_days: i32,
        configured_tenant_ids: &[String],
    ) -> Result<crate::db::CorpusAnalyticsSummary, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tenant_ids: Vec<String> = if configured_tenant_ids.is_empty() {
            client
                .query("SELECT tenant_id FROM trace_tenants", &[])
                .await
                .map_err(DatabaseError::Postgres)?
                .into_iter()
                .map(|row| row.get::<_, String>("tenant_id"))
                .collect()
        } else {
            configured_tenant_ids.to_vec()
        };

        let mut total_submissions = 0_i64;
        let mut total_accepted = 0_i64;
        let mut total_rejected = 0_i64;
        // Histogram buckets: 0, 100k, ..., 900k (the 10th absorbs >=1M).
        let mut histogram: [(i64, i64); 11] = [
            (0, 0),
            (100_000, 0),
            (200_000, 0),
            (300_000, 0),
            (400_000, 0),
            (500_000, 0),
            (600_000, 0),
            (700_000, 0),
            (800_000, 0),
            (900_000, 0),
            (1_000_000, 0),
        ];
        let mut both_passed = 0_i64;
        let mut novelty_failed = 0_i64;
        let mut perplexity_failed = 0_i64;
        let mut both_failed = 0_i64;

        for tenant_id in &tenant_ids {
            let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
            // Submission counts.
            let counts = tx
                .query_one(
                    "SELECT
                        COUNT(*) AS total,
                        COUNT(*) FILTER (WHERE status = 'accepted') AS accepted,
                        COUNT(*) FILTER (WHERE status IN ('rejected', 'quarantined', 'revoked')) AS rejected
                     FROM trace_submissions
                     WHERE received_at >= NOW() - ($1 || ' days')::interval",
                    &[&window_days.to_string()],
                )
                .await
                .map_err(DatabaseError::Postgres)?;
            total_submissions += counts.get::<_, i64>("total");
            total_accepted += counts.get::<_, i64>("accepted");
            total_rejected += counts.get::<_, i64>("rejected");
            // Novelty score histogram + gate outcomes.
            let buckets = tx
                .query(
                    "SELECT
                        LEAST(novelty_score_micros / 100000, 10) AS bucket_idx,
                        COUNT(*) AS bucket_count
                     FROM trace_gate_decisions
                     WHERE decided_at >= NOW() - ($1 || ' days')::interval
                     GROUP BY 1",
                    &[&window_days.to_string()],
                )
                .await
                .map_err(DatabaseError::Postgres)?;
            for row in buckets {
                let idx: i64 = row.get("bucket_idx");
                let count: i64 = row.get("bucket_count");
                let idx = idx.clamp(0, 10) as usize;
                histogram[idx].1 += count;
            }
            let outcomes = tx
                .query(
                    "SELECT
                        COUNT(*) FILTER (WHERE perplexity_passed AND novelty_passed) AS both_passed,
                        COUNT(*) FILTER (WHERE perplexity_passed AND NOT novelty_passed) AS novelty_failed,
                        COUNT(*) FILTER (WHERE NOT perplexity_passed AND novelty_passed) AS perplexity_failed,
                        COUNT(*) FILTER (WHERE NOT perplexity_passed AND NOT novelty_passed) AS both_failed
                     FROM trace_gate_decisions
                     WHERE decided_at >= NOW() - ($1 || ' days')::interval",
                    &[&window_days.to_string()],
                )
                .await
                .map_err(DatabaseError::Postgres)?;
            let outcome = outcomes
                .first()
                .ok_or_else(|| DatabaseError::Pool("expected 1 row".to_string()))?;
            both_passed += outcome.get::<_, i64>("both_passed");
            novelty_failed += outcome.get::<_, i64>("novelty_failed");
            perplexity_failed += outcome.get::<_, i64>("perplexity_failed");
            both_failed += outcome.get::<_, i64>("both_failed");
            tx.commit().await.map_err(DatabaseError::Postgres)?;
        }

        let accept_rate = if total_submissions > 0 {
            total_accepted as f64 / total_submissions as f64
        } else {
            0.0
        };
        let novelty_histogram = histogram.into_iter().collect();
        let mut gate_outcomes = vec![
            ("both_passed".to_string(), both_passed),
            ("novelty_failed".to_string(), novelty_failed),
            ("perplexity_failed".to_string(), perplexity_failed),
            ("both_failed".to_string(), both_failed),
        ];
        gate_outcomes.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        Ok(crate::db::CorpusAnalyticsSummary {
            total_submissions,
            total_accepted,
            total_rejected,
            accept_rate,
            novelty_histogram,
            gate_outcomes,
        })
    }

    async fn insert_leaderboard_snapshot(
        &self,
        write: crate::db::LeaderboardSnapshotWrite,
    ) -> Result<crate::db::LeaderboardSnapshotRow, DatabaseError> {
        let client = self.trace_pool().get().await?;
        let row = client
            .query_one(
                "INSERT INTO trace_leaderboard_snapshots (
                    snapshot_id, window_label, metric, contents_jsonb,
                    contents_sha256, min_cell_count, noise_seed_hash
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7)
                 RETURNING snapshot_id, computed_at, window_label, metric,
                           contents_jsonb, contents_sha256, min_cell_count,
                           noise_seed_hash",
                &[
                    &write.snapshot_id,
                    &write.window_label,
                    &write.metric,
                    &write.contents,
                    &write.contents_sha256,
                    &write.min_cell_count,
                    &write.noise_seed_hash,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        Ok(crate::db::LeaderboardSnapshotRow {
            snapshot_id: row.get("snapshot_id"),
            computed_at: row.get("computed_at"),
            window_label: row.get("window_label"),
            metric: row.get("metric"),
            contents: row.get("contents_jsonb"),
            contents_sha256: row.get("contents_sha256"),
            min_cell_count: row.get("min_cell_count"),
            noise_seed_hash: row.get("noise_seed_hash"),
        })
    }

    async fn latest_leaderboard_snapshot(
        &self,
        window_label: &str,
        metric: &str,
    ) -> Result<Option<crate::db::LeaderboardSnapshotRow>, DatabaseError> {
        let client = self.trace_pool().get().await?;
        let row = client
            .query_opt(
                "SELECT snapshot_id, computed_at, window_label, metric,
                        contents_jsonb, contents_sha256, min_cell_count,
                        noise_seed_hash
                 FROM trace_leaderboard_snapshots
                 WHERE window_label = $1 AND metric = $2
                 ORDER BY computed_at DESC
                 LIMIT 1",
                &[&window_label, &metric],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        Ok(row.map(|row| crate::db::LeaderboardSnapshotRow {
            snapshot_id: row.get("snapshot_id"),
            computed_at: row.get("computed_at"),
            window_label: row.get("window_label"),
            metric: row.get("metric"),
            contents: row.get("contents_jsonb"),
            contents_sha256: row.get("contents_sha256"),
            min_cell_count: row.get("min_cell_count"),
            noise_seed_hash: row.get("noise_seed_hash"),
        }))
    }

    async fn insert_device_key(
        &self,
        device_key: crate::db::DeviceKeyWrite,
    ) -> Result<crate::db::DeviceKeyRecord, DatabaseError> {
        self.ensure_trace_tenant(&device_key.tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &device_key.tenant_id).await?;
        let row = tx
            .query_one(
                "INSERT INTO device_keys (
                    device_key_id, tenant_id, public_key, invite_subject_hash, client_info
                 ) VALUES ($1, $2, $3, $4, $5)
                 RETURNING device_key_id, tenant_id, public_key, invite_subject_hash,
                           client_info, created_at, revoked_at",
                &[
                    &device_key.device_key_id,
                    &device_key.tenant_id,
                    &device_key.public_key,
                    &device_key.invite_subject_hash,
                    &device_key.client_info,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(device_key_record_from_row(row))
    }

    async fn get_device_key(
        &self,
        tenant_id: &str,
        device_key_id: &str,
    ) -> Result<Option<crate::db::DeviceKeyRecord>, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT device_key_id, tenant_id, public_key, invite_subject_hash,
                        client_info, created_at, revoked_at
                   FROM device_keys
                  WHERE tenant_id = $1 AND device_key_id = $2",
                &[&tenant_id, &device_key_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(row.map(device_key_record_from_row))
    }

    async fn list_device_keys(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<crate::db::DeviceKeyRecord>, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT device_key_id, tenant_id, public_key, invite_subject_hash,
                        client_info, created_at, revoked_at
                   FROM device_keys
                  WHERE tenant_id = $1
                  ORDER BY created_at ASC, device_key_id ASC",
                &[&tenant_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(rows.into_iter().map(device_key_record_from_row).collect())
    }

    async fn revoke_device_key(
        &self,
        tenant_id: &str,
        device_key_id: &str,
    ) -> Result<Option<crate::db::DeviceKeyRecord>, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "UPDATE device_keys
                    SET revoked_at = COALESCE(revoked_at, NOW())
                  WHERE tenant_id = $1 AND device_key_id = $2
                  RETURNING device_key_id, tenant_id, public_key, invite_subject_hash,
                            client_info, created_at, revoked_at",
                &[&tenant_id, &device_key_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(row.map(device_key_record_from_row))
    }

    async fn onboard_device_key(
        &self,
        device_key: crate::db::DeviceKeyWrite,
        max_uses: i32,
    ) -> Result<crate::db::OnboardDeviceKeyRecord, crate::db::OnboardDeviceKeyError> {
        if max_uses <= 0 {
            return Err(crate::db::OnboardDeviceKeyError::InviteNotValid);
        }
        self.ensure_trace_tenant(&device_key.tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &device_key.tenant_id).await?;

        if let Some(existing) = tx
            .query_opt(
                "SELECT device_key_id, tenant_id, public_key, invite_subject_hash,
                        client_info, created_at, revoked_at
                   FROM device_keys
                  WHERE tenant_id = $1 AND device_key_id = $2",
                &[&device_key.tenant_id, &device_key.device_key_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?
        {
            let record = device_key_record_from_row(existing);
            if record.public_key == device_key.public_key
                && record.invite_subject_hash == device_key.invite_subject_hash
                && record.revoked_at.is_none()
            {
                upsert_onboarding_device_tenant_access_grant(
                    &tx,
                    &device_key.tenant_id,
                    &device_key.device_key_id,
                )
                .await?;
                tx.commit().await.map_err(DatabaseError::Postgres)?;
                return Ok(crate::db::OnboardDeviceKeyRecord {
                    device_key: record,
                    status: crate::db::OnboardDeviceKeyStatus::Idempotent,
                });
            }
            return Err(crate::db::OnboardDeviceKeyError::InviteNotValid);
        }

        let invite_upsert = tx
            .query_opt(
                "INSERT INTO onboarding_invites (
                    tenant_id, invite_subject_hash, max_uses
                 ) VALUES ($1, $2, $3)
                 ON CONFLICT (tenant_id, invite_subject_hash) DO UPDATE SET
                    max_uses = GREATEST(onboarding_invites.consumed_uses, excluded.max_uses),
                    updated_at = NOW()
                 WHERE onboarding_invites.revoked_at IS NULL
                 RETURNING invite_subject_hash",
                &[
                    &device_key.tenant_id,
                    &device_key.invite_subject_hash,
                    &max_uses,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        if invite_upsert.is_none() {
            return Err(crate::db::OnboardDeviceKeyError::InviteNotValid);
        }

        let inserted = tx
            .query_opt(
                "INSERT INTO device_keys (
                    device_key_id, tenant_id, public_key, invite_subject_hash, client_info
                 ) VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (device_key_id) DO NOTHING
                 RETURNING device_key_id, tenant_id, public_key, invite_subject_hash,
                           client_info, created_at, revoked_at",
                &[
                    &device_key.device_key_id,
                    &device_key.tenant_id,
                    &device_key.public_key,
                    &device_key.invite_subject_hash,
                    &device_key.client_info,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;

        let Some(inserted) = inserted else {
            let existing = tx
                .query_opt(
                    "SELECT device_key_id, tenant_id, public_key, invite_subject_hash,
                            client_info, created_at, revoked_at
                       FROM device_keys
                      WHERE tenant_id = $1 AND device_key_id = $2",
                    &[&device_key.tenant_id, &device_key.device_key_id],
                )
                .await
                .map_err(DatabaseError::Postgres)?;
            if let Some(existing) = existing {
                let record = device_key_record_from_row(existing);
                if record.public_key == device_key.public_key
                    && record.invite_subject_hash == device_key.invite_subject_hash
                    && record.revoked_at.is_none()
                {
                    upsert_onboarding_device_tenant_access_grant(
                        &tx,
                        &device_key.tenant_id,
                        &device_key.device_key_id,
                    )
                    .await?;
                    tx.commit().await.map_err(DatabaseError::Postgres)?;
                    return Ok(crate::db::OnboardDeviceKeyRecord {
                        device_key: record,
                        status: crate::db::OnboardDeviceKeyStatus::Idempotent,
                    });
                }
            }
            return Err(crate::db::OnboardDeviceKeyError::InviteNotValid);
        };

        let consumed = tx
            .query_opt(
                "UPDATE onboarding_invites
                    SET consumed_uses = consumed_uses + 1,
                        updated_at = NOW()
                  WHERE tenant_id = $1
                    AND invite_subject_hash = $2
                    AND revoked_at IS NULL
                    AND consumed_uses < max_uses
                  RETURNING consumed_uses",
                &[&device_key.tenant_id, &device_key.invite_subject_hash],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        if consumed.is_none() {
            return Err(crate::db::OnboardDeviceKeyError::InviteAlreadyConsumed);
        }

        upsert_onboarding_device_tenant_access_grant(
            &tx,
            &device_key.tenant_id,
            &device_key.device_key_id,
        )
        .await?;

        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(crate::db::OnboardDeviceKeyRecord {
            device_key: device_key_record_from_row(inserted),
            status: crate::db::OnboardDeviceKeyStatus::Registered,
        })
    }

    async fn create_or_reuse_account(
        &self,
        tenant_id: &str,
        principal_ref: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;

        // Fast path: an active link for this principal already maps to an account.
        if let Some(row) = tx
            .query_opt(
                "SELECT account_id FROM trace_account_principals
                  WHERE tenant_id = trace_current_tenant_id()
                    AND principal_ref = $1
                    AND unlinked_at IS NULL",
                &[&principal_ref],
            )
            .await
            .map_err(DatabaseError::Postgres)?
        {
            let account_id: Uuid = row.get("account_id");
            tx.commit().await.map_err(DatabaseError::Postgres)?;
            return Ok(account_id);
        }

        // No active link: mint a fresh account and link the principal. The link
        // insert is ON CONFLICT DO NOTHING against the UNIQUE (tenant_id,
        // principal_ref) constraint so a concurrent mint that won the race does
        // not error us out; we re-select the authoritative account below.
        let new_account_id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO trace_accounts (tenant_id, account_id)
             VALUES (trace_current_tenant_id(), $1)",
            &[&new_account_id],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.execute(
            "INSERT INTO trace_account_principals (tenant_id, account_id, principal_ref)
             VALUES (trace_current_tenant_id(), $1, $2)
             ON CONFLICT (tenant_id, principal_ref) DO NOTHING",
            &[&new_account_id, &principal_ref],
        )
        .await
        .map_err(DatabaseError::Postgres)?;

        // Re-select the authoritative account for this principal. If a
        // concurrent mint inserted the link first, this returns ITS account_id
        // and our freshly-inserted (now orphaned) trace_accounts row is harmless
        // (no principal links to it). If we won, it returns new_account_id.
        let row = tx
            .query_one(
                "SELECT account_id FROM trace_account_principals
                  WHERE tenant_id = trace_current_tenant_id()
                    AND principal_ref = $1
                    AND unlinked_at IS NULL",
                &[&principal_ref],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let account_id: Uuid = row.get("account_id");
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(account_id)
    }

    async fn count_outstanding_login_links(
        &self,
        tenant_id: &str,
        created_principal_ref: &str,
    ) -> Result<i64, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_one(
                "SELECT count(*) AS outstanding
                   FROM trace_login_links
                  WHERE tenant_id = trace_current_tenant_id()
                    AND created_principal_ref = $1
                    AND consumed_at IS NULL
                    AND expires_at > now()",
                &[&created_principal_ref],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let outstanding: i64 = row.get("outstanding");
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(outstanding)
    }

    async fn insert_login_link(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        code_hash: &str,
        created_principal_ref: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let link_id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO trace_login_links (
                tenant_id, link_id, account_id, code_hash,
                created_principal_ref, created_at, expires_at
             ) VALUES (
                trace_current_tenant_id(), $1, $2, $3, $4, now(), $5
             )",
            &[
                &link_id,
                &account_id,
                &code_hash,
                &created_principal_ref,
                &expires_at,
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn append_account_audit(
        &self,
        tenant_id: &str,
        action: &str,
        actor_ref: &str,
        outcome: &str,
        safe_metadata: serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "INSERT INTO trace_account_audit (
                tenant_id, action, actor_ref, outcome, safe_metadata
             ) VALUES (trace_current_tenant_id(), $1, $2, $3, $4)",
            &[&action, &actor_ref, &outcome, &safe_metadata],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn resolve_login_link_tenant(
        &self,
        code_hash: &str,
    ) -> Result<Option<String>, DatabaseError> {
        // Delegate to the inherent implementation (narrow resolver pool); map its
        // anyhow error onto the trait's DatabaseError. The fail-closed
        // unconfigured-resolver path surfaces here as a Pool error.
        PgBackend::resolve_login_link_tenant(self, code_hash)
            .await
            .map_err(|error| DatabaseError::Pool(error.to_string()))
    }

    async fn consume_login_link(
        &self,
        tenant_id: &str,
        code_hash: &str,
    ) -> Result<Option<crate::db::ConsumedLoginLink>, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;

        // Single atomic conditional consume (Hardening D/G): ALWAYS executed, never
        // SELECT-then-branch. Unknown / expired / already-consumed / wrong-tenant
        // codes all affect zero rows. The explicit tenant predicate is
        // belt-and-suspenders on top of RLS; `code_hash` is globally UNIQUE.
        let row = tx
            .query_opt(
                "UPDATE trace_login_links SET consumed_at = now()
                  WHERE code_hash = $1
                    AND tenant_id = trace_current_tenant_id()
                    AND consumed_at IS NULL
                    AND expires_at > now()
                  RETURNING account_id, created_principal_ref",
                &[&code_hash],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;

        // `query_opt` returns at most one row; the WHERE clause guarantees
        // rows_affected is 0 or 1. None => generic deny upstream.
        Ok(row.map(|row| crate::db::ConsumedLoginLink {
            account_id: row.get("account_id"),
            created_principal_ref: row.get("created_principal_ref"),
        }))
    }

    async fn insert_session(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        token_hash: &str,
        client_kind: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Uuid, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // Server-assigned session id; never client-supplied.
        let session_id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO trace_sessions (
                tenant_id, session_id, account_id, token_hash,
                client_kind, created_at, last_seen_at, expires_at
             ) VALUES (
                trace_current_tenant_id(), $1, $2, $3, $4, now(), now(), $5
             )",
            &[
                &session_id,
                &account_id,
                &token_hash,
                &client_kind,
                &expires_at,
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(session_id)
    }
}

fn device_key_record_from_row(row: Row) -> crate::db::DeviceKeyRecord {
    crate::db::DeviceKeyRecord {
        device_key_id: row.get("device_key_id"),
        tenant_id: row.get("tenant_id"),
        public_key: row.get("public_key"),
        invite_subject_hash: row.get("invite_subject_hash"),
        client_info: row.get("client_info"),
        created_at: row.get("created_at"),
        revoked_at: row.get("revoked_at"),
    }
}

async fn upsert_onboarding_device_tenant_access_grant(
    tx: &tokio_postgres::Transaction<'_>,
    tenant_id: &str,
    device_key_id: &str,
) -> Result<(), DatabaseError> {
    let grant_id = onboarding_device_tenant_access_grant_id(tenant_id, device_key_id);
    let principal_ref = onboarding_device_principal_ref(tenant_id, device_key_id);
    let allowed_consent_scopes = serde_json::json!(["debugging_evaluation", "public_attribution"]);
    let allowed_uses = serde_json::json!(["debugging", "evaluation", "aggregate_analytics"]);
    let metadata_json =
        serde_json::json!({"source": "onboarding_device_key", "capability": "pilot_default"});

    tx.execute(
        "INSERT INTO trace_tenant_access_grants (
            tenant_id, grant_id, principal_ref, role, status,
            allowed_consent_scopes, allowed_uses, issuer, audience, subject,
            issued_at, expires_at, revoked_at, created_by_principal_ref,
            revoked_by_principal_ref, reason, metadata_json
         ) VALUES ($1, $2, $3, 'contributor', 'active',
            $4, $5, NULL, NULL, NULL, NOW(), NULL, NULL,
            'system:onboard_device_key', NULL, $6, $7)
         ON CONFLICT (tenant_id, grant_id) DO UPDATE SET
            principal_ref = excluded.principal_ref,
            role = excluded.role,
            status = excluded.status,
            allowed_consent_scopes = excluded.allowed_consent_scopes,
            allowed_uses = excluded.allowed_uses,
            issuer = excluded.issuer,
            audience = excluded.audience,
            subject = excluded.subject,
            expires_at = excluded.expires_at,
            revoked_at = excluded.revoked_at,
            created_by_principal_ref = excluded.created_by_principal_ref,
            reason = excluded.reason,
            metadata_json = excluded.metadata_json,
            updated_at = NOW()
          WHERE trace_tenant_access_grants.status <> 'revoked'",
        &[
            &tenant_id,
            &grant_id,
            &principal_ref,
            &allowed_consent_scopes,
            &allowed_uses,
            &ONBOARDING_DEVICE_GRANT_REASON,
            &metadata_json,
        ],
    )
    .await
    .map_err(DatabaseError::Postgres)?;
    Ok(())
}

fn onboarding_device_tenant_access_grant_id(tenant_id: &str, device_key_id: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!(
            "tracecommons:onboarding-device-access-grant:{}:{}",
            tenant_id.trim(),
            device_key_id.trim()
        )
        .as_bytes(),
    )
}

fn onboarding_device_principal_ref(tenant_id: &str, device_key_id: &str) -> String {
    let digest = Sha256::digest(format!(
        "device:{}:{}",
        tenant_id.trim(),
        device_key_id.trim()
    ));
    format!("principal_sha256:{}", hex::encode(digest))
}

fn sha256_prefixed(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    format!("sha256:{}", hex::encode(digest))
}

async fn trace_tenant_context_is_transaction_local(
    client: &mut deadpool_postgres::Client,
) -> Result<bool, DatabaseError> {
    let tx = client.transaction().await?;
    let probe_tenant = "__trace_rls_probe_tenant__";
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&probe_tenant],
    )
    .await?;
    let inside = tx
        .query_one(
            "SELECT current_setting('trace_commons.trace_tenant_id', true) AS tenant_context",
            &[],
        )
        .await?
        .get::<_, Option<String>>("tenant_context");
    tx.commit().await?;
    let after = client
        .query_one(
            "SELECT current_setting('trace_commons.trace_tenant_id', true) AS tenant_context",
            &[],
        )
        .await?
        .get::<_, Option<String>>("tenant_context");
    Ok(inside.as_deref() == Some(probe_tenant) && after.as_deref().is_none_or(str::is_empty))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboarding_device_principal_ref_matches_issuer_hash_input() {
        assert_eq!(
            onboarding_device_principal_ref("tenant-1", "sha256:device-key"),
            "principal_sha256:2bf5c8fb2e00d4b044f1e3d24aaf864d21197baaaca9de3da5de631a951caf9d"
        );
    }

    #[test]
    fn onboarding_device_grant_id_is_stable_and_device_scoped() {
        let first = onboarding_device_tenant_access_grant_id("tenant-1", "sha256:device-key");
        let second = onboarding_device_tenant_access_grant_id("tenant-1", "sha256:device-key");
        let other_device = onboarding_device_tenant_access_grant_id("tenant-1", "sha256:other");
        let other_tenant =
            onboarding_device_tenant_access_grant_id("tenant-2", "sha256:device-key");

        assert_eq!(first, second);
        assert_ne!(first, other_device);
        assert_ne!(first, other_tenant);
    }

    #[test]
    fn leaderboard_inputs_credit_device_key_profiles_by_submission_principal() {
        assert!(
            LEADERBOARD_INPUTS_SQL.contains("FROM trace_submissions ts_match"),
            "leaderboard rows must bridge accepted credit events through the source submission"
        );
        assert!(
            LEADERBOARD_INPUTS_SQL.contains("ts_match.auth_principal_ref = cp.principal_ref"),
            "device-key public profiles are keyed by auth principal, not trace pseudonym"
        );
        assert!(
            LEADERBOARD_INPUTS_SQL.contains("ts_match.contributor_pseudonym"),
            "leaderboard rows must also support profiles keyed by trace credit pseudonym"
        );
        assert!(
            LEADERBOARD_INPUTS_SQL.contains("COALESCE(ts.received_at, cl.occurred_at)"),
            "leaderboard recency should prefer trace receive time over ledger backfill time"
        );
        assert!(
            LEADERBOARD_INPUTS_SQL.contains("cl.credit_account_ref = cp.principal_ref"),
            "legacy credit-account joins must remain supported"
        );
    }

    #[test]
    fn trace_commons_rls_registry_matches_migration_policy_coverage() {
        let central_policy_migrations = [
            include_str!("../../../../migrations/V18__trace_central_rls_tenant_predicate.sql"),
            include_str!("../../../../migrations/V21__trace_near_credit_account_outbox.sql"),
            include_str!("../../../../migrations/V26__trace_contributor_profiles.sql"),
            include_str!("../../../../migrations/V28__device_keys.sql"),
            include_str!("../../../../migrations/V29__onboarding_invites.sql"),
            include_str!("../../../../migrations/V30__trace_accounts.sql"),
        ];
        let force_rls_migrations = [
            include_str!("../../../../migrations/V6__trace_force_rls.sql"),
            include_str!("../../../../migrations/V11__trace_ranking_worker_runs.sql"),
            include_str!("../../../../migrations/V14__trace_ranking_preference_labels.sql"),
            include_str!("../../../../migrations/V15__trace_benchmark_registry_outbox.sql"),
            include_str!("../../../../migrations/V16__trace_ranking_calibration_datasets.sql"),
            include_str!("../../../../migrations/V21__trace_near_credit_account_outbox.sql"),
            include_str!("../../../../migrations/V26__trace_contributor_profiles.sql"),
            include_str!("../../../../migrations/V28__device_keys.sql"),
            include_str!("../../../../migrations/V29__onboarding_invites.sql"),
            include_str!("../../../../migrations/V30__trace_accounts.sql"),
        ];

        for table in TRACE_COMMONS_RLS_TABLES {
            assert!(
                central_policy_migrations.iter().any(|migration| {
                    migration.contains(&format!(
                        "DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON {table};"
                    ))
                }),
                "{table} is missing from the central RLS policy migration cleanup"
            );
            assert!(
                central_policy_migrations.iter().any(|migration| {
                    migration.contains(&format!(
                        "CREATE POLICY trace_corpus_tenant_isolation ON {table}"
                    ))
                }),
                "{table} is missing from the central RLS policy migration install"
            );
            assert!(
                force_rls_migrations.iter().any(|migration| {
                    migration.contains(&format!("ALTER TABLE {table} FORCE ROW LEVEL SECURITY;"))
                }),
                "{table} is missing FORCE ROW LEVEL SECURITY migration coverage"
            );
        }

        let central_policy_count = TRACE_COMMONS_RLS_TABLES
            .iter()
            .filter(|table| {
                central_policy_migrations.iter().any(|migration| {
                    migration.contains(&format!(
                        "CREATE POLICY trace_corpus_tenant_isolation ON {table}"
                    ))
                })
            })
            .count();
        assert_eq!(
            central_policy_count,
            TRACE_COMMONS_RLS_TABLES.len(),
            "central RLS policy migration and diagnostics registry drifted"
        );
    }

    #[test]
    fn trace_corpus_pg_client_access_enters_tenant_context_transactions() {
        let source = include_str!("trace_corpus_pg.rs");
        let client_marker = concat!("self.", "trace_pool().get().await?");
        let tenant_context_marker = "Self::begin_trace_tenant_transaction";
        let mut checked_client_accesses = 0;

        for (line_number, line) in source.lines().enumerate() {
            if !line.contains(client_marker) {
                continue;
            }
            checked_client_accesses += 1;

            let tenant_context_window = source
                .lines()
                .skip(line_number + 1)
                .take(8)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                tenant_context_window.contains(tenant_context_marker),
                "trace_corpus_pg.rs:{} gets a PostgreSQL client without immediately entering \
                 transaction-local trace tenant context",
                line_number + 1
            );
        }

        assert!(
            checked_client_accesses >= TRACE_COMMONS_RLS_TABLES.len(),
            "trace corpus tenant-context guard did not inspect the expected store surface"
        );
    }

    #[test]
    fn pg_backend_does_not_expose_raw_pool_as_application_api() {
        let source = include_str!("postgres.rs");
        let public_raw_pool_marker = concat!("pub fn ", "pool(&self) -> Pool");
        assert!(
            !source.contains(public_raw_pool_marker),
            "PgBackend must not expose its raw pool as a normal public API; use the \
             tenant-context helpers for application paths and an explicit test hook for \
             raw RLS probes"
        );
    }
}
