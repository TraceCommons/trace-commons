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

/// Rotation-on-use cadence for browser sessions. A cookie session whose CURRENT
/// token was issued more than this many seconds ago is rotated on its next live
/// request: a fresh secret is minted and the old hash is parked in
/// `prev_token_hash` for a short grace window. Default ~12h. Overridable via
/// `TRACE_COMMONS_SESSION_ROTATION_INTERVAL_SECS` so tests can force rotation
/// without waiting (the env var is read per call; production never sets it).
const SESSION_ROTATION_INTERVAL_SECS_DEFAULT: i64 = 12 * 60 * 60;
/// Grace window during which the PREVIOUS token still validates after a rotation,
/// so an in-flight / multi-tab request holding the old cookie is not logged out
/// before it observes the new `Set-Cookie`. Default ~2 min. Overridable via
/// `TRACE_COMMONS_SESSION_ROTATION_GRACE_SECS` for tests.
const SESSION_ROTATION_GRACE_SECS_DEFAULT: i64 = 2 * 60;

/// Read the rotation interval (seconds), honoring the test-only env override.
/// A malformed or non-positive override falls back to the default so a bad value
/// can never disable rotation cadence entirely.
fn session_rotation_interval_secs() -> i64 {
    std::env::var("TRACE_COMMONS_SESSION_ROTATION_INTERVAL_SECS")
        .ok()
        .and_then(|raw| raw.parse::<i64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(SESSION_ROTATION_INTERVAL_SECS_DEFAULT)
}

/// Read the prev-token grace (seconds), honoring the test-only env override.
fn session_rotation_grace_secs() -> i64 {
    std::env::var("TRACE_COMMONS_SESSION_ROTATION_GRACE_SECS")
        .ok()
        .and_then(|raw| raw.parse::<i64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(SESSION_ROTATION_GRACE_SECS_DEFAULT)
}

/// Fixed advisory-lock namespace (classid) for the NEAR settlement submit serializer.
/// Paired with a per-tenant objid (`hashtext('near-credit-submit:'||tenant)`), the
/// two-int `pg_try_advisory_lock(classid, objid)` form keeps this lock space
/// disjoint from the one-arg `pg_advisory_xact_lock(hashtext(tenant))` used by the
/// audit-chain append, so the two can never alias.
const NEAR_CREDIT_SUBMIT_ADVISORY_LOCK_CLASSID: i32 = 0x7472_6163u32 as i32; // "trac"

/// Owns the pooled connection that holds a session-level advisory lock for the
/// duration of a NEAR settlement submit pass. Released explicitly via
/// [`NearCreditSubmitAdvisoryLockInner::release`]; see the public
/// `NearCreditSubmitAdvisoryLock` wrapper in `db::mod` for lifecycle docs.
pub struct NearCreditSubmitAdvisoryLockInner {
    client: deadpool_postgres::Object,
    objid: i32,
}

impl NearCreditSubmitAdvisoryLockInner {
    pub(crate) async fn release(self) -> Result<(), DatabaseError> {
        // Best-effort unlock on the SAME connection that took the lock; session
        // advisory locks are connection-scoped, so this must run here before the
        // connection returns to the pool.
        self.client
            .execute(
                "SELECT pg_advisory_unlock($1, $2)",
                &[&NEAR_CREDIT_SUBMIT_ADVISORY_LOCK_CLASSID, &self.objid],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        Ok(())
    }
}

pub struct PgBackend {
    pool: Pool,
    /// Narrow, SEPARATE pool for the unauthenticated login-link redeem path.
    /// Built only when `login_resolver_url` is configured; its DB user is the
    /// operator-provisioned `trace_login_resolver` role (no BYPASSRLS,
    /// column-scoped SELECT on `trace_login_links` only). `None` keeps the
    /// account-redeem path fail-closed. NEVER aliased to `pool`.
    login_resolver_pool: Option<Pool>,
    /// Narrow, SEPARATE pool for the cross-tenant gate-driver enumeration
    /// query. Built only when `gate_driver_url` is configured; its DB user is
    /// the operator-provisioned `trace_gate_driver` role (NOLOGIN base,
    /// NOBYPASSRLS, permissive cross-tenant SELECT policies from migration
    /// V36). `None` keeps the gate driver's enumeration path fail-closed.
    /// NEVER aliased to `pool`.
    gate_driver_pool: Option<Pool>,
    /// Narrow, SEPARATE pool for the cross-tenant PII-backstop driver
    /// enumeration query (server-side NEAR AI PII backstop). Built only when
    /// `pii_backstop_driver_url` is configured; its DB user is the
    /// operator-provisioned `trace_pii_backstop_driver` role (NOLOGIN base,
    /// NOBYPASSRLS, permissive cross-tenant SELECT policies from migration
    /// V38). `None` keeps the backstop driver's enumeration path fail-closed.
    /// NEVER aliased to `pool`. Mirrors `gate_driver_pool`. Query methods
    /// against this pool land in a follow-up task; this field is wired but
    /// unused until then.
    #[allow(dead_code)]
    pii_backstop_driver_pool: Option<Pool>,
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
    "trace_webauthn_credentials",
    "trace_near_identities",
    "trace_account_merge_proposals",
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
                let resolver_config =
                    resolver_url
                        .parse::<tokio_postgres::Config>()
                        .map_err(|e| {
                            DatabaseError::Pool(format!(
                                "invalid login-resolver PostgreSQL URL: {e}"
                            ))
                        })?;
                let resolver_manager =
                    deadpool_postgres::Manager::new(resolver_config, tokio_postgres::NoTls);
                let resolver_pool = Pool::builder(resolver_manager).max_size(2).build()?;
                Some(resolver_pool)
            }
            None => None,
        };

        // Build a SEPARATE, small gate-driver pool only when a distinct
        // gate-driver connection string is configured. This pool runs as the
        // narrow `trace_gate_driver` role and is never aliased to the runtime
        // pool. Mirrors the login-resolver pool above exactly.
        let gate_driver_pool = match config.gate_driver_url() {
            Some(gate_driver_url) => {
                let gate_driver_config = gate_driver_url
                    .parse::<tokio_postgres::Config>()
                    .map_err(|e| {
                        DatabaseError::Pool(format!("invalid gate-driver PostgreSQL URL: {e}"))
                    })?;
                let gate_driver_manager =
                    deadpool_postgres::Manager::new(gate_driver_config, tokio_postgres::NoTls);
                let gate_driver_pool = Pool::builder(gate_driver_manager).max_size(2).build()?;
                Some(gate_driver_pool)
            }
            None => None,
        };

        // Build a SEPARATE, small PII-backstop driver pool only when a
        // distinct PII-backstop driver connection string is configured. This
        // pool runs as the narrow `trace_pii_backstop_driver` role and is
        // never aliased to the runtime pool. Mirrors the gate-driver pool
        // above exactly.
        let pii_backstop_driver_pool = match config.pii_backstop_driver_url() {
            Some(pii_backstop_driver_url) => {
                let pii_backstop_driver_config = pii_backstop_driver_url
                    .parse::<tokio_postgres::Config>()
                    .map_err(|e| {
                        DatabaseError::Pool(format!(
                            "invalid pii-backstop-driver PostgreSQL URL: {e}"
                        ))
                    })?;
                let pii_backstop_driver_manager = deadpool_postgres::Manager::new(
                    pii_backstop_driver_config,
                    tokio_postgres::NoTls,
                );
                let pii_backstop_driver_pool = Pool::builder(pii_backstop_driver_manager)
                    .max_size(2)
                    .build()?;
                Some(pii_backstop_driver_pool)
            }
            None => None,
        };

        Ok(Self {
            pool,
            login_resolver_pool,
            gate_driver_pool,
            pii_backstop_driver_pool,
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

    /// Resolve the tenant for a WebAuthn credential via the NARROW resolver pool
    /// (separate role, column-scoped SELECT, no BYPASSRLS). Returns the tenant
    /// only; the caller MUST re-confirm tenant inside an RLS-scoped transaction
    /// before any write. Fail-closed: if the resolver pool is not configured, this
    /// errors with a safe missing-control name rather than falling back to the
    /// runtime pool.
    ///
    /// Wired into the login handler in Task 6; until then its only non-test caller
    /// is pending. It is `pub` (part of the crate API, like
    /// `resolve_login_link_tenant`), so it does not trip dead-code under
    /// `-D warnings` despite having no internal caller yet.
    pub async fn resolve_credential_tenant(
        &self,
        credential_id: &str,
    ) -> anyhow::Result<Option<String>> {
        let pool = self
            .login_resolver_pool
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing-control: login-resolver-pool-unconfigured"))?;
        let client = pool.get().await?;
        // Safe without a tenant predicate: credential_id is globally UNIQUE so this returns at most
        // one row across all tenants; the login handler re-confirms tenant inside an RLS-scoped tx
        // before any write. Do NOT add a non-unique lookup column to this role's grant.
        let row = client
            .query_opt(
                "SELECT tenant_id FROM trace_webauthn_credentials WHERE credential_id = $1",
                &[&credential_id],
            )
            .await?;
        Ok(row.map(|r| r.get::<_, String>(0)))
    }

    /// Resolve a NEAR access public_key to its tenant via the narrow,
    /// restricted-role resolver pool (separate role, column-scoped SELECT, no
    /// BYPASSRLS), exactly like `resolve_credential_tenant`. Returns the tenant
    /// ONLY. The NEAR login path (Task 7) calls this with the public_key parsed
    /// from an UNAUTHENTICATED, wallet-signed assertion, BEFORE any tenant context
    /// exists, then re-confirms tenant inside an RLS-scoped tx before any write.
    /// Fail-closed: if the resolver pool is not configured, this errors with a
    /// safe missing-control name rather than falling back to the runtime pool.
    ///
    /// Wired into the login handler in Task 7; until then its only non-test caller
    /// is pending. It is `pub` (part of the crate API, like
    /// `resolve_credential_tenant`), so it does not trip dead-code under
    /// `-D warnings` despite having no internal caller yet.
    pub async fn resolve_near_public_key_tenant(
        &self,
        public_key: &str,
    ) -> anyhow::Result<Option<String>> {
        let pool = self
            .login_resolver_pool
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing-control: login-resolver-pool-unconfigured"))?;
        let client = pool.get().await?;
        // Safe without a tenant predicate: public_key is globally UNIQUE so this returns at most
        // one row across all tenants; the login handler re-confirms tenant inside an RLS-scoped tx
        // before any write. Do NOT add a non-unique lookup column to this role's grant.
        let row = client
            .query_opt(
                "SELECT tenant_id FROM trace_near_identities WHERE public_key = $1",
                &[&public_key],
            )
            .await?;
        Ok(row.map(|r| r.get::<_, String>(0)))
    }
}

#[async_trait]
impl Database for PgBackend {
    async fn try_acquire_near_credit_submit_lock(
        &self,
        tenant_id: &str,
    ) -> Result<Option<crate::db::NearCreditSubmitAdvisoryLock>, DatabaseError> {
        let client = self
            .trace_pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e.to_string()))?;
        // Derive the per-tenant objid in SQL so it matches the unlock key exactly.
        let objid: i32 = client
            .query_one(
                "SELECT hashtext('near-credit-submit:' || $1)",
                &[&tenant_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?
            .get(0);
        let acquired: bool = client
            .query_one(
                "SELECT pg_try_advisory_lock($1, $2)",
                &[&NEAR_CREDIT_SUBMIT_ADVISORY_LOCK_CLASSID, &objid],
            )
            .await
            .map_err(DatabaseError::Postgres)?
            .get(0);
        if !acquired {
            // Another submit run already holds the lock; drop the connection back
            // to the pool without taking ownership.
            return Ok(None);
        }
        Ok(Some(crate::db::NearCreditSubmitAdvisoryLock::new(
            NearCreditSubmitAdvisoryLockInner { client, objid },
        )))
    }

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
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&31_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V31__account_traces_index.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&31_i32, &"account_traces_index"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&32_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V32__webauthn_credentials.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&32_i32, &"webauthn_credentials"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&33_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V33__near_identities.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&33_i32, &"near_identities"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&34_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V34__account_consolidation.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&34_i32, &"account_consolidation"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&35_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V35__trace_instance_enrollments.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&35_i32, &"trace_instance_enrollments"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&36_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V36__trace_gate_driver.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&36_i32, &"trace_gate_driver"],
                )
                .await?;
        }
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&37_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V37__large_trace_chunked_scoring.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&37_i32, &"large_trace_chunked_scoring"],
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
        let default_allowed_consent_scopes: Vec<String> = DEFAULT_ONBOARDING_CONSENT_SCOPES
            .iter()
            .map(|s| s.to_string())
            .collect();
        let default_allowed_uses: Vec<String> = DEFAULT_ONBOARDING_ALLOWED_USES
            .iter()
            .map(|s| s.to_string())
            .collect();
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
                    &default_allowed_consent_scopes,
                    &default_allowed_uses,
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
                        &default_allowed_consent_scopes,
                        &default_allowed_uses,
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
            &default_allowed_consent_scopes,
            &default_allowed_uses,
        )
        .await?;

        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(crate::db::OnboardDeviceKeyRecord {
            device_key: device_key_record_from_row(inserted),
            status: crate::db::OnboardDeviceKeyStatus::Registered,
        })
    }

    async fn enroll_instance_user(
        &self,
        p: crate::db::InstanceUserProvision,
    ) -> Result<(), DatabaseError> {
        // ensure_trace_tenant runs in its own transaction and is idempotent.
        self.ensure_trace_tenant(&p.tenant_id).await?;

        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &p.tenant_id).await?;

        // Stamp the contribution policy once; never overwrite an existing row.
        tx.execute(
            "INSERT INTO trace_tenant_policies
                 (tenant_id, policy_version, allowed_consent_scopes, allowed_uses,
                  updated_by_principal_ref)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (tenant_id) DO NOTHING",
            &[
                &p.tenant_id,
                &p.policy_version,
                &p.allowed_consent_scopes,
                &p.allowed_uses,
                &format!("instance-enroll:{}", p.instance_subject_hash),
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;

        // Register the device key (the principal). `device_key_id` is a GLOBAL
        // primary key, so an `ON CONFLICT DO NOTHING` no-op can mean the key
        // already exists under a DIFFERENT tenant or is revoked here — in which
        // case the device's bearer would not authenticate to the derived tenant.
        // Mirror `onboard_device_key`: insert-or-reselect and accept only an
        // identical, non-revoked row under THIS tenant; otherwise fail closed so
        // enrollment never reports success for an unusable device key.
        let inserted = tx
            .query_opt(
                "INSERT INTO device_keys
                     (device_key_id, tenant_id, public_key, invite_subject_hash, client_info)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (device_key_id) DO NOTHING
                 RETURNING device_key_id",
                &[
                    &p.device_key_id,
                    &p.tenant_id,
                    &p.public_key,
                    &p.instance_subject_hash,
                    &p.client_info,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;

        if inserted.is_none() {
            let existing = tx
                .query_opt(
                    "SELECT public_key, revoked_at
                       FROM device_keys
                      WHERE tenant_id = $1 AND device_key_id = $2",
                    &[&p.tenant_id, &p.device_key_id],
                )
                .await
                .map_err(DatabaseError::Postgres)?;
            let usable = match existing {
                Some(row) => {
                    let public_key: String = row.get("public_key");
                    let revoked_at: Option<chrono::DateTime<chrono::Utc>> = row.get("revoked_at");
                    public_key == p.public_key && revoked_at.is_none()
                }
                None => false,
            };
            if !usable {
                return Err(DatabaseError::Pool(
                    "instance enroll device key conflict: not usable under derived tenant"
                        .to_string(),
                ));
            }
        }

        // Grant the default contributor tenant-access grant so the device can mint
        // upload claims under the `require_tenant_access_grants` gate, exactly like
        // an invite-onboarded device. Same helper, same principal_ref derivation.
        // Scopes come from the instance's policy template, falling back to the
        // pilot defaults when the template is missing, empty, or malformed.
        let allowed_consent_scopes = normalize_provision_scope_values(
            &p.allowed_consent_scopes,
            &DEFAULT_ONBOARDING_CONSENT_SCOPES,
        );
        let allowed_uses =
            normalize_provision_scope_values(&p.allowed_uses, &DEFAULT_ONBOARDING_ALLOWED_USES);
        upsert_onboarding_device_tenant_access_grant(
            &tx,
            &p.tenant_id,
            &p.device_key_id,
            &allowed_consent_scopes,
            &allowed_uses,
        )
        .await?;

        tx.commit().await.map_err(DatabaseError::Postgres)?;

        // Create-or-reuse the contributor account and bind the device principal so
        // instance-enrolled users get account-scoped trace read-back immediately,
        // without waiting for a first login-link mint. The principal_ref is the one
        // the device authenticates as (and the login-link mint path passes), so
        // this converges idempotently with that path. `create_or_reuse_account`
        // runs its own self-contained tenant transaction.
        let principal_ref = onboarding_device_principal_ref(&p.tenant_id, &p.device_key_id);
        self.create_or_reuse_account(&p.tenant_id, &principal_ref)
            .await?;

        Ok(())
    }

    async fn reserve_instance_enrollment(
        &self,
        instance_subject_hash: &str,
        user_subject_hash: &str,
        tenant_id: &str,
        max_enrollments: i64,
    ) -> Result<crate::db::InstanceEnrollmentOutcome, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = client
            .transaction()
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.execute(
            "SELECT set_config('trace_commons.instance_subject', $1, true)",
            &[&instance_subject_hash],
        )
        .await
        .map_err(DatabaseError::Postgres)?;

        // Already enrolled? Idempotent — no cap consumption.
        let existing = tx
            .query_opt(
                "SELECT 1 FROM trace_instance_enrollments
                  WHERE instance_subject_hash = $1 AND user_subject_hash = $2",
                &[&instance_subject_hash, &user_subject_hash],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        if existing.is_some() {
            tx.commit().await.map_err(DatabaseError::Postgres)?;
            return Ok(crate::db::InstanceEnrollmentOutcome::ExistingUser);
        }

        // Lock-free cap check: count, then insert ON CONFLICT DO NOTHING.
        // A concurrent burst of DISTINCT new users could each read count < cap
        // and all insert, overshooting the cap by the concurrency width. For
        // the pilot's per-instance rate limit this is acceptable; if strict
        // capping is later required, take an advisory lock on
        // hashtext(instance_subject_hash) at the top of the tx.
        let count: i64 = tx
            .query_one(
                "SELECT COUNT(*)::BIGINT FROM trace_instance_enrollments
                  WHERE instance_subject_hash = $1",
                &[&instance_subject_hash],
            )
            .await
            .map_err(DatabaseError::Postgres)?
            .get(0);
        if count >= max_enrollments {
            tx.commit().await.map_err(DatabaseError::Postgres)?;
            return Ok(crate::db::InstanceEnrollmentOutcome::CapExceeded);
        }

        let inserted = tx
            .execute(
                "INSERT INTO trace_instance_enrollments
                     (instance_subject_hash, user_subject_hash, tenant_id)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (instance_subject_hash, user_subject_hash) DO NOTHING",
                &[&instance_subject_hash, &user_subject_hash, &tenant_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;

        // A racing insert of the SAME user resolves to ExistingUser.
        Ok(if inserted == 1 {
            crate::db::InstanceEnrollmentOutcome::NewlyEnrolled
        } else {
            crate::db::InstanceEnrollmentOutcome::ExistingUser
        })
    }

    async fn instance_ledger_rls_ready(&self) -> Result<bool, DatabaseError> {
        let client = self.trace_pool().get().await?;
        let row = client
            .query_one(
                "SELECT
                    EXISTS (
                        SELECT 1
                          FROM pg_class c
                          JOIN pg_namespace n ON n.oid = c.relnamespace
                         WHERE n.nspname = current_schema()
                           AND c.relname = 'trace_instance_enrollments'
                           AND c.relrowsecurity
                           AND c.relforcerowsecurity
                    ) AS rls_forced,
                    EXISTS (
                        SELECT 1
                          FROM pg_policies
                         WHERE schemaname = current_schema()
                           AND tablename = 'trace_instance_enrollments'
                           AND policyname = 'trace_instance_isolation'
                    ) AS policy_present",
                &[],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let rls_forced: bool = row.get("rls_forced");
        let policy_present: bool = row.get("policy_present");
        Ok(rls_forced && policy_present)
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

    /// Batched principal->account resolution. One query under the tenant tx maps
    /// every active-linked principal in `principal_refs` to its account; principals
    /// with no active link are simply absent from the result. Mirrors the
    /// tenant-tx shape of `create_or_reuse_account`.
    async fn resolve_principals_to_accounts(
        &self,
        tenant_id: &str,
        principal_refs: &[String],
    ) -> Result<std::collections::HashMap<String, Uuid>, DatabaseError> {
        // Empty input short-circuits without a query.
        if principal_refs.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT principal_ref, account_id FROM trace_account_principals
                  WHERE tenant_id = trace_current_tenant_id()
                    AND unlinked_at IS NULL
                    AND principal_ref = ANY($1)",
                &[&principal_refs],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        let mut map = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            let principal_ref: String = row.get("principal_ref");
            let account_id: Uuid = row.get("account_id");
            map.insert(principal_ref, account_id);
        }
        Ok(map)
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

    async fn resolve_credential_tenant(
        &self,
        credential_id: &str,
    ) -> Result<Option<String>, DatabaseError> {
        // Delegate to the inherent implementation (narrow resolver pool); map its
        // anyhow error onto the trait's DatabaseError. The fail-closed
        // unconfigured-resolver path surfaces here as a Pool error, which the
        // login handler collapses to the uniform deny.
        PgBackend::resolve_credential_tenant(self, credential_id)
            .await
            .map_err(|error| DatabaseError::Pool(error.to_string()))
    }

    async fn resolve_near_public_key_tenant(
        &self,
        public_key: &str,
    ) -> Result<Option<String>, DatabaseError> {
        // Delegate to the inherent implementation (narrow resolver pool); map its
        // anyhow error onto the trait's DatabaseError. The fail-closed
        // unconfigured-resolver path surfaces here as a Pool error, which the
        // login handler collapses to the uniform deny.
        PgBackend::resolve_near_public_key_tenant(self, public_key)
            .await
            .map_err(|error| DatabaseError::Pool(error.to_string()))
    }

    async fn redeem_login_link(
        &self,
        tenant_id: &str,
        code_hash: &str,
        session: crate::db::NewSession<'_>,
        audit: crate::db::RedeemAudit,
    ) -> Result<Option<crate::db::RedeemedSession>, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;

        // Single atomic conditional consume (Hardening D/G): ALWAYS executed, never
        // SELECT-then-branch. Unknown / expired / already-consumed / wrong-tenant
        // codes all affect zero rows. The explicit tenant predicate is
        // belt-and-suspenders on top of RLS; `code_hash` is globally UNIQUE.
        let consumed = tx
            .query_opt(
                "UPDATE trace_login_links SET consumed_at = now()
                  WHERE code_hash = $1
                    AND tenant_id = trace_current_tenant_id()
                    AND consumed_at IS NULL
                    AND expires_at > now()
                  RETURNING account_id",
                &[&code_hash],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let Some(consumed) = consumed else {
            // No row consumed: commit the no-op tx so the link stays UNconsumed and
            // retryable, and return None for the uniform deny upstream.
            tx.commit().await.map_err(DatabaseError::Postgres)?;
            return Ok(None);
        };
        let account_id: Uuid = consumed.get("account_id");

        // Same RLS-scoped tx: insert the session (hash-only token) ...
        let session_id = Uuid::new_v4(); // server-assigned; never client-supplied.
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
                &session.token_hash,
                &session.client_kind,
                &session.expires_at,
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;

        // ... and the hash-only / label-only audit row. Actor is derived from the
        // consumed account_id (reserved-prefix, never sha-shaped). If any of these
        // fail the whole redeem rolls back: link stays reusable, no orphaned
        // session, no un-audited state change.
        let actor_ref = crate::account_session::account_actor_ref(
            &crate::account_session::AccountId::from_uuid(account_id),
        );
        tx.execute(
            "INSERT INTO trace_account_audit (
                tenant_id, action, actor_ref, outcome, safe_metadata
             ) VALUES (trace_current_tenant_id(), $1, $2, $3, $4)",
            &[&audit.action, &actor_ref, &audit.outcome, &audit.metadata],
        )
        .await
        .map_err(DatabaseError::Postgres)?;

        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(Some(crate::db::RedeemedSession {
            account_id,
            session_id,
        }))
    }

    async fn validate_session(
        &self,
        tenant_id: &str,
        token_hash: &str,
    ) -> Result<Option<crate::db::ValidatedSession>, DatabaseError> {
        // SECURITY: do NOT ensure_trace_tenant here. `tenant_id` is the
        // client-supplied, pre-auth value decoded from the session cookie; an
        // UPSERT into trace_tenants would let an unauthenticated forged cookie
        // spray arbitrary tenant rows before the token is validated. The tenant
        // already exists for any legitimate session (created at mint), and
        // begin_trace_tenant_transaction only sets the RLS config var (no row
        // dependency), so a forged/nonexistent tenant scopes the lookup to a
        // tenant where this hash cannot exist -> zero rows -> deny, no write.
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;

        // Every-request liveness gate (Hardening I): unexpired AND not revoked AND
        // seen within the idle cap. The idle cap (3d) is intentionally shorter than
        // the 7d absolute `expires_at` so an abandoned session dies sooner than its
        // hard expiry. The `tenant_id = trace_current_tenant_id()` predicate is
        // belt-and-suspenders on top of forced RLS; `token_hash` is globally
        // UNIQUE so a client-supplied (forged/mismatched) tenant simply scopes the
        // lookup to a tenant where this hash does not exist -> no row -> deny.
        //
        // Rotation-on-use: the SELECT also accepts a within-grace PREVIOUS token
        // (`prev_token_hash = $1 AND prev_token_valid_until > now()`) so a request
        // still carrying the old cookie during the brief rotation grace continues
        // to validate (multi-tab / in-flight). `matched_current` records whether
        // the request matched the CURRENT token: only a current match is eligible
        // to (re-)rotate, so a prev-token request never re-rotates. `needs_rotate`
        // pre-computes the age check in SQL against the (possibly env-overridden)
        // interval. We always read back the row's CURRENT `token_hash` so the
        // last_seen / rotate UPDATE keys on the canonical hash, not the presented
        // one (which may be the prev token).
        let interval_secs = session_rotation_interval_secs();
        let grace_secs = session_rotation_grace_secs();
        let row = tx
            .query_opt(
                "SELECT s.account_id, s.auth_credential_id, s.token_hash, s.client_kind,
                        (s.token_hash = $1) AS matched_current,
                        (s.token_issued_at < now() - make_interval(secs => $2)) AS needs_rotate
                   FROM trace_sessions s
                   JOIN trace_accounts a
                     ON a.tenant_id = s.tenant_id
                    AND a.account_id = s.account_id
                  WHERE s.tenant_id = trace_current_tenant_id()
                    AND a.closed_at IS NULL
                    AND (s.token_hash = $1
                         OR (s.prev_token_hash = $1 AND s.prev_token_valid_until > now()))
                    AND s.expires_at > now()
                    AND s.revoked_at IS NULL
                    AND s.last_seen_at > now() - INTERVAL '3 days'",
                &[&token_hash, &(interval_secs as f64)],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let Some(row) = row else {
            // Miss OR idle-capped. Auto-revoke any matching row that is still live
            // (unexpired, unrevoked) but fell past the idle cap, so a leaked secret
            // cannot be reused after the idle window even before hard expiry. The
            // predicate is the inverse idle-cap on an otherwise-valid row; a true
            // unknown hash affects zero rows. It keys ONLY on the CURRENT
            // `token_hash`, so a within-grace prev-token presentation (whose own
            // row, if any, is still live and was simply not matched here) is never
            // revoked by this branch.
            tx.execute(
                "UPDATE trace_sessions SET revoked_at = now()
                  WHERE tenant_id = trace_current_tenant_id()
                    AND token_hash = $1
                    AND revoked_at IS NULL
                    AND expires_at > now()
                    AND last_seen_at <= now() - INTERVAL '3 days'",
                &[&token_hash],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
            tx.commit().await.map_err(DatabaseError::Postgres)?;
            return Ok(None);
        };
        let account_id: Uuid = row.get("account_id");
        let auth_credential_id: Option<String> = row.get("auth_credential_id");
        let client_kind: String = row.get("client_kind");
        let current_token_hash: String = row.get("token_hash");
        let matched_current: bool = row.get("matched_current");
        let needs_rotate: bool = row.get("needs_rotate");

        // Rotate only on a CURRENT-token match that has aged past the interval. A
        // prev-token (within-grace) request slides the idle window forward but must
        // NOT re-rotate (that would churn the secret every multi-tab request and
        // could orphan the cookie the other tab still holds).
        let rotated_secret = if matched_current && needs_rotate {
            let new_secret = crate::account_session::generate_session_secret();
            let new_hash = crate::account_session::hash_secret(&new_secret);
            // Same-tx rotation: park the old hash as the prev token for `grace`,
            // swap in the new hash, reset the issue clock, slide last_seen. Keyed on
            // the canonical current hash.
            tx.execute(
                "UPDATE trace_sessions
                    SET prev_token_hash = token_hash,
                        prev_token_valid_until = now() + make_interval(secs => $2),
                        token_hash = $3,
                        token_issued_at = now(),
                        last_seen_at = now()
                  WHERE tenant_id = trace_current_tenant_id()
                    AND token_hash = $1",
                &[&current_token_hash, &(grace_secs as f64), &new_hash],
            )
            .await
            .map_err(DatabaseError::Postgres)?;

            // Hash-only audit. Records only the action and actor (reserved-prefix
            // account actor); never a token, secret, or hash.
            let actor_ref = crate::account_session::account_actor_ref(
                &crate::account_session::AccountId::from_uuid(account_id),
            );
            tx.execute(
                "INSERT INTO trace_account_audit (
                    tenant_id, action, actor_ref, outcome, safe_metadata
                 ) VALUES (trace_current_tenant_id(), $1, $2, $3, $4)",
                &[
                    &"account_session_rotated",
                    &actor_ref,
                    &"success",
                    &serde_json::json!({}),
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
            Some(new_secret)
        } else {
            // Live hit (no rotation): bump last_seen_at to slide the idle window
            // forward. Keyed on the canonical current hash so a prev-token match
            // still refreshes the right row.
            tx.execute(
                "UPDATE trace_sessions SET last_seen_at = now()
                  WHERE tenant_id = trace_current_tenant_id()
                    AND token_hash = $1",
                &[&current_token_hash],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
            None
        };
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(Some(crate::db::ValidatedSession {
            account_id,
            auth_credential_id,
            client_kind,
            rotated_secret,
        }))
    }

    async fn revoke_current_session(
        &self,
        tenant_id: &str,
        token_hash: &str,
    ) -> Result<u64, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // Idempotent single-session revoke. Match the presented hash as EITHER the
        // current `token_hash` OR the just-rotated-away `prev_token_hash`: if the
        // same request rotated the session (rotation runs in the auth middleware
        // BEFORE the logout handler re-derives the hash from the presented OLD
        // cookie), the row's current hash is now the freshly minted one and the
        // presented hash lives in `prev_token_hash`. Keying on the current hash
        // alone would miss that row and leave the rotated session live. Revoking on
        // either match kills the WHOLE row (one row per session: both hashes belong
        // to the same row), so the rotated cookie the middleware appends lands on an
        // already-revoked row and the next request 401s. `token_hash` is globally
        // UNIQUE and `prev_token_hash` is its short-lived predecessor, so at most one
        // row matches; an already-revoked or unknown hash affects zero rows.
        let revoked = tx
            .execute(
                "UPDATE trace_sessions SET revoked_at = now()
                  WHERE tenant_id = trace_current_tenant_id()
                    AND (token_hash = $1 OR prev_token_hash = $1)
                    AND revoked_at IS NULL",
                &[&token_hash],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(revoked)
    }

    async fn revoke_all_account_sessions(
        &self,
        tenant_id: &str,
        account_id: Uuid,
    ) -> Result<u64, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // Sign-out-everywhere: revoke every live session for the auth-derived
        // account. Tenant- + account-scoped under forced RLS, so only the caller's
        // own sessions can ever be touched.
        let revoked = tx
            .execute(
                "UPDATE trace_sessions SET revoked_at = now()
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND revoked_at IS NULL",
                &[&account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(revoked)
    }

    async fn resolve_account_for_principal(
        &self,
        tenant_id: &str,
        principal_ref: &str,
    ) -> Result<Option<Uuid>, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // Active-membership only (Hardening A): an unlinked principal must not
        // resolve to its former account.
        let row = tx
            .query_opt(
                "SELECT account_id FROM trace_account_principals
                  WHERE tenant_id = trace_current_tenant_id()
                    AND principal_ref = $1
                    AND unlinked_at IS NULL",
                &[&principal_ref],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let account_id = row.map(|row| row.get::<_, Uuid>("account_id"));
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(account_id)
    }

    async fn expand_account_principals(
        &self,
        tenant_id: &str,
        account_id: Uuid,
    ) -> Result<crate::account_session::AccountPrincipalSet, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // The ONLY sanctioned ownership-bearing expansion (Hardening A): active
        // memberships only. An `unlinked_at`-set principal is absent from the set.
        let rows = tx
            .query(
                "SELECT principal_ref FROM trace_account_principals
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND unlinked_at IS NULL",
                &[&account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let principals = crate::account_session::AccountPrincipalSet::from_iter(
            rows.into_iter()
                .map(|row| row.get::<_, String>("principal_ref")),
        );
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(principals)
    }

    async fn insert_webauthn_credential(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        credential_id: &str,
        passkey: &serde_json::Value,
        label: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "INSERT INTO trace_webauthn_credentials (
                tenant_id, credential_id, account_id, passkey, label, created_at
             ) VALUES (
                trace_current_tenant_id(), $1, $2, $3, $4, now()
             )",
            &[&credential_id, &account_id, &passkey, &label],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn load_webauthn_credential_for_login(
        &self,
        tenant_id: &str,
        credential_id: &str,
    ) -> Result<Option<crate::db::WebauthnCredentialRow>, DatabaseError> {
        // The resolver has already mapped credential_id -> tenant_id; this load
        // runs under that resolved tenant's RLS. We do NOT ensure_trace_tenant
        // here: the tenant already exists for any registered credential, and
        // begin_trace_tenant_transaction only sets the RLS config var (no row
        // dependency). The `tenant_id = trace_current_tenant_id()` predicate is
        // belt-and-suspenders on top of forced RLS; `credential_id` is globally
        // UNIQUE.
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT account_id, passkey FROM trace_webauthn_credentials
                  WHERE tenant_id = trace_current_tenant_id()
                    AND credential_id = $1
                    AND revoked_at IS NULL",
                &[&credential_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(row.map(|row| crate::db::WebauthnCredentialRow {
            account_id: row.get("account_id"),
            passkey: row.get("passkey"),
        }))
    }

    async fn update_webauthn_credential_after_login(
        &self,
        tenant_id: &str,
        credential_id: &str,
        passkey: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "UPDATE trace_webauthn_credentials
                SET passkey = $1, last_used_at = now()
              WHERE tenant_id = trace_current_tenant_id()
                AND credential_id = $2
                AND revoked_at IS NULL",
            &[&passkey, &credential_id],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn issue_passkey_session(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        session: crate::db::NewSession<'_>,
        auth_credential_id: &str,
        audit: crate::db::RedeemAudit,
    ) -> Result<(), DatabaseError> {
        // SECURITY: do NOT ensure_trace_tenant here. The credential (and thus its
        // tenant) was verified by the login handler before this call: the tenant
        // provably exists via the credential row's FK, and an UPSERT here would
        // let a forged assertion spray tenant rows. begin_trace_tenant_transaction
        // only sets the RLS config var (no row dependency), and the session insert
        // is FK-bound to (tenant_id, account_id) in trace_accounts, so a bogus
        // tenant/account simply fails the insert rather than writing anything.
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;

        // Session row: hash-only token, client_kind='passkey', the base64url
        // credential id STRING in auth_credential_id (V32 TEXT column), and
        // token_issued_at left to its DEFAULT now(). session_id is server-assigned.
        let session_id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO trace_sessions (
                tenant_id, session_id, account_id, token_hash,
                client_kind, auth_credential_id, created_at, last_seen_at, expires_at
             ) VALUES (
                trace_current_tenant_id(), $1, $2, $3, $4, $5, now(), now(), $6
             )",
            &[
                &session_id,
                &account_id,
                &session.token_hash,
                &session.client_kind,
                &auth_credential_id,
                &session.expires_at,
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;

        // Hash-only / label-only audit row in the SAME tx so an audit failure rolls
        // back the session (no un-audited session, no orphaned audit).
        let actor_ref = crate::account_session::account_actor_ref(
            &crate::account_session::AccountId::from_uuid(account_id),
        );
        tx.execute(
            "INSERT INTO trace_account_audit (
                tenant_id, action, actor_ref, outcome, safe_metadata
             ) VALUES (trace_current_tenant_id(), $1, $2, $3, $4)",
            &[&audit.action, &actor_ref, &audit.outcome, &audit.metadata],
        )
        .await
        .map_err(DatabaseError::Postgres)?;

        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn issue_near_session(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        session: crate::db::NewSession<'_>,
        auth_credential_id: &str,
        audit: crate::db::RedeemAudit,
    ) -> Result<(), DatabaseError> {
        // SECURITY: do NOT ensure_trace_tenant here. The NEAR identity (and thus
        // its tenant) was verified by the login handler before this call: the
        // tenant provably exists via the identity row's FK, and an UPSERT here
        // would let a forged assertion spray tenant rows. begin_trace_tenant_
        // transaction only sets the RLS config var (no row dependency), and the
        // session insert is FK-bound to (tenant_id, account_id) in trace_accounts,
        // so a bogus tenant/account simply fails the insert rather than writing
        // anything. Mirrors issue_passkey_session except client_kind='near' and
        // auth_credential_id carries the NEAR access key (a public identifier).
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;

        let session_id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO trace_sessions (
                tenant_id, session_id, account_id, token_hash,
                client_kind, auth_credential_id, created_at, last_seen_at, expires_at
             ) VALUES (
                trace_current_tenant_id(), $1, $2, $3, $4, $5, now(), now(), $6
             )",
            &[
                &session_id,
                &account_id,
                &session.token_hash,
                &session.client_kind,
                &auth_credential_id,
                &session.expires_at,
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;

        // Hash-only / label-only audit row in the SAME tx so an audit failure rolls
        // back the session (no un-audited session, no orphaned audit). Never the
        // NEAR public key, account id, or any signature material.
        let actor_ref = crate::account_session::account_actor_ref(
            &crate::account_session::AccountId::from_uuid(account_id),
        );
        tx.execute(
            "INSERT INTO trace_account_audit (
                tenant_id, action, actor_ref, outcome, safe_metadata
             ) VALUES (trace_current_tenant_id(), $1, $2, $3, $4)",
            &[&audit.action, &actor_ref, &audit.outcome, &audit.metadata],
        )
        .await
        .map_err(DatabaseError::Postgres)?;

        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn list_account_credentials(
        &self,
        tenant_id: &str,
        account_id: Uuid,
    ) -> Result<Vec<crate::db::AccountCredentialSummary>, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT credential_id, label, created_at, last_used_at
                   FROM trace_webauthn_credentials
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND revoked_at IS NULL
                  ORDER BY created_at",
                &[&account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(rows
            .into_iter()
            .map(|row| crate::db::AccountCredentialSummary {
                credential_id: row.get("credential_id"),
                label: row.get("label"),
                created_at: row.get("created_at"),
                last_used_at: row.get("last_used_at"),
            })
            .collect())
    }

    async fn rename_account_credential(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        credential_id: &str,
        label: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // account-scoped so a caller can never rename a credential they do not own.
        let affected = tx
            .execute(
                "UPDATE trace_webauthn_credentials
                    SET label = $1
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $2
                    AND credential_id = $3
                    AND revoked_at IS NULL",
                &[&label, &account_id, &credential_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(affected > 0)
    }

    async fn revoke_account_credential(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        credential_id: &str,
    ) -> Result<crate::db::RevokeCredentialResult, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // account-scoped soft-delete: an unknown / already-revoked / other-account
        // credential affects zero rows.
        let removed = tx
            .execute(
                "UPDATE trace_webauthn_credentials
                    SET revoked_at = now()
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND credential_id = $2
                    AND revoked_at IS NULL",
                &[&account_id, &credential_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let remaining_row = tx
            .query_one(
                "SELECT count(*) AS remaining
                   FROM trace_webauthn_credentials
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND revoked_at IS NULL",
                &[&account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let remaining: i64 = remaining_row.get("remaining");
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(crate::db::RevokeCredentialResult {
            removed: removed > 0,
            remaining,
        })
    }

    async fn insert_near_identity(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        public_key: &str,
        near_account_id: &str,
        label: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "INSERT INTO trace_near_identities (
                tenant_id, public_key, near_account_id, account_id, label, created_at
             ) VALUES (
                trace_current_tenant_id(), $1, $2, $3, $4, now()
             )",
            &[&public_key, &near_account_id, &account_id, &label],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn load_near_identity_for_login(
        &self,
        tenant_id: &str,
        public_key: &str,
    ) -> Result<Option<crate::db::NearIdentityRow>, DatabaseError> {
        // The resolver has already mapped public_key -> tenant_id; this load runs
        // under that resolved tenant's RLS. We do NOT ensure_trace_tenant here: the
        // tenant already exists for any registered identity, and
        // begin_trace_tenant_transaction only sets the RLS config var (no row
        // dependency). The `tenant_id = trace_current_tenant_id()` predicate is
        // belt-and-suspenders on top of forced RLS; `public_key` is globally UNIQUE.
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT account_id, near_account_id FROM trace_near_identities
                  WHERE tenant_id = trace_current_tenant_id()
                    AND public_key = $1
                    AND revoked_at IS NULL",
                &[&public_key],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(row.map(|row| crate::db::NearIdentityRow {
            account_id: row.get("account_id"),
            near_account_id: row.get("near_account_id"),
        }))
    }

    async fn touch_near_identity_last_used(
        &self,
        tenant_id: &str,
        public_key: &str,
    ) -> Result<(), DatabaseError> {
        // SECURITY: do NOT ensure_trace_tenant here — login path. The identity row
        // (loaded above) already guarantees the tenant via its FK;
        // begin_trace_tenant_transaction only sets the RLS var (no write).
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "UPDATE trace_near_identities
                SET last_used_at = now()
              WHERE tenant_id = trace_current_tenant_id()
                AND public_key = $1
                AND revoked_at IS NULL",
            &[&public_key],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn list_account_near_identities(
        &self,
        tenant_id: &str,
        account_id: Uuid,
    ) -> Result<Vec<crate::db::NearIdentitySummary>, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT public_key, near_account_id, label, created_at, last_used_at,
                        payout_designated_at
                   FROM trace_near_identities
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND revoked_at IS NULL
                  ORDER BY created_at",
                &[&account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let payout_designated_at: Option<chrono::DateTime<chrono::Utc>> =
                    row.get("payout_designated_at");
                crate::db::NearIdentitySummary {
                    public_key: row.get("public_key"),
                    near_account_id: row.get("near_account_id"),
                    label: row.get("label"),
                    created_at: row.get("created_at"),
                    last_used_at: row.get("last_used_at"),
                    is_payout: payout_designated_at.is_some(),
                }
            })
            .collect())
    }

    async fn rename_account_near_identity(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        public_key: &str,
        label: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // account-scoped so a caller can never rename an identity they do not own.
        let affected = tx
            .execute(
                "UPDATE trace_near_identities
                    SET label = $1
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $2
                    AND public_key = $3
                    AND revoked_at IS NULL",
                &[&label, &account_id, &public_key],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(affected > 0)
    }

    async fn revoke_account_near_identity(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        public_key: &str,
    ) -> Result<crate::db::RevokeNearResult, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // account-scoped soft-delete: an unknown / already-revoked / other-account
        // key affects zero rows.
        let removed = tx
            .execute(
                "UPDATE trace_near_identities
                    SET revoked_at = now()
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND public_key = $2
                    AND revoked_at IS NULL",
                &[&account_id, &public_key],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        // remaining strong authenticators = webauthn credentials + NEAR identities,
        // computed in the SAME tx after the revoke.
        let remaining_row = tx
            .query_one(
                "SELECT (
                    SELECT count(*) FROM trace_webauthn_credentials
                      WHERE tenant_id = trace_current_tenant_id()
                        AND account_id = $1
                        AND revoked_at IS NULL
                  ) + (
                    SELECT count(*) FROM trace_near_identities
                      WHERE tenant_id = trace_current_tenant_id()
                        AND account_id = $1
                        AND revoked_at IS NULL
                  ) AS remaining_strong",
                &[&account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let remaining_strong: i64 = remaining_row.get("remaining_strong");
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(crate::db::RevokeNearResult {
            removed: removed > 0,
            remaining_strong,
        })
    }

    async fn count_active_strong_authenticators(
        &self,
        tenant_id: &str,
        account_id: Uuid,
    ) -> Result<i64, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_one(
                "SELECT (
                    SELECT count(*) FROM trace_webauthn_credentials
                      WHERE tenant_id = trace_current_tenant_id()
                        AND account_id = $1
                        AND revoked_at IS NULL
                  ) + (
                    SELECT count(*) FROM trace_near_identities
                      WHERE tenant_id = trace_current_tenant_id()
                        AND account_id = $1
                        AND revoked_at IS NULL
                  ) AS strong_count",
                &[&account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(row.get("strong_count"))
    }

    async fn designate_payout_near_identity(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        public_key: &str,
    ) -> Result<bool, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // Clear any existing active designation first so at most one active row ever
        // carries payout_designated_at -> the partial-unique index can never trip.
        tx.execute(
            "UPDATE trace_near_identities
                SET payout_designated_at = NULL
              WHERE tenant_id = trace_current_tenant_id()
                AND account_id = $1
                AND payout_designated_at IS NOT NULL",
            &[&account_id],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        // Stamp the named active key. account-scoped + revoked_at IS NULL so an
        // unknown / revoked / other-account key affects zero rows.
        let affected = tx
            .execute(
                "UPDATE trace_near_identities
                    SET payout_designated_at = now()
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND public_key = $2
                    AND revoked_at IS NULL",
                &[&account_id, &public_key],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(affected > 0)
    }

    async fn clear_payout_near_identity(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        public_key: &str,
    ) -> Result<bool, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let affected = tx
            .execute(
                "UPDATE trace_near_identities
                    SET payout_designated_at = NULL
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND public_key = $2
                    AND revoked_at IS NULL
                    AND payout_designated_at IS NOT NULL",
                &[&account_id, &public_key],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(affected > 0)
    }

    async fn resolve_payout_near_account_id(
        &self,
        tenant_id: &str,
        account_id: Uuid,
    ) -> Result<crate::db::PayoutResolution, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT near_account_id, payout_designated_at
                   FROM trace_near_identities
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND revoked_at IS NULL",
                &[&account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;

        // A designated active identity wins outright.
        if let Some(row) = rows.iter().find(|row| {
            row.get::<_, Option<chrono::DateTime<chrono::Utc>>>("payout_designated_at")
                .is_some()
        }) {
            return Ok(crate::db::PayoutResolution::Designated(
                row.get("near_account_id"),
            ));
        }
        // No designation: a single active identity is unambiguous; otherwise hold.
        match rows.len() {
            0 => Ok(crate::db::PayoutResolution::Hold(
                crate::db::PayoutHoldReason::NoneEnrolled,
            )),
            1 => Ok(crate::db::PayoutResolution::SoleActive(
                rows[0].get("near_account_id"),
            )),
            _ => Ok(crate::db::PayoutResolution::Hold(
                crate::db::PayoutHoldReason::AmbiguousNoDesignation,
            )),
        }
    }

    async fn stage_merge_proposal(
        &self,
        tenant_id: &str,
        surviving_account_id: Uuid,
        merge_code_hash: &str,
    ) -> Result<Option<crate::db::StagedMergeProposal>, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;

        // Proof-of-control: consume device B's login-link with the SAME atomic
        // conditional consume as redeem_login_link (ALWAYS executed, never a
        // SELECT-then-branch). Unknown / expired / already-consumed / wrong-tenant
        // codes all affect zero rows -> commit the no-op tx and deny. The
        // `tenant_id = trace_current_tenant_id()` predicate is belt-and-suspenders
        // on top of forced RLS; `code_hash` is globally UNIQUE.
        let consumed = tx
            .query_opt(
                "UPDATE trace_login_links SET consumed_at = now()
                  WHERE code_hash = $1
                    AND tenant_id = trace_current_tenant_id()
                    AND consumed_at IS NULL
                    AND expires_at > now()
                  RETURNING account_id",
                &[&merge_code_hash],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let Some(consumed) = consumed else {
            tx.commit().await.map_err(DatabaseError::Postgres)?;
            return Ok(None);
        };
        let absorbed_account_id: Uuid = consumed.get("account_id");

        // Guard: cannot merge an account into itself. The consume already fired,
        // so commit it (the link is single-use spent) and deny.
        if absorbed_account_id == surviving_account_id {
            tx.commit().await.map_err(DatabaseError::Postgres)?;
            return Ok(None);
        }

        // Guard: the absorbed account B must still be open. A closed account has
        // nothing live to fold in.
        let closed_at: Option<chrono::DateTime<chrono::Utc>> = tx
            .query_one(
                "SELECT closed_at FROM trace_accounts
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1",
                &[&absorbed_account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?
            .get("closed_at");
        if closed_at.is_some() {
            tx.commit().await.map_err(DatabaseError::Postgres)?;
            return Ok(None);
        }

        // Attribution-only count of B's ACTIVE principals for operator review.
        let absorbed_principal_count: i64 = tx
            .query_one(
                "SELECT count(*) FROM trace_account_principals
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND unlinked_at IS NULL",
                &[&absorbed_account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?
            .get(0);

        // Stage the single-use, time-bounded proposal. proposal_id is
        // server-assigned; expires in 10 minutes.
        let proposal_id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO trace_account_merge_proposals (
                tenant_id, proposal_id, surviving_account_id, absorbed_account_id,
                absorbed_principal_count, created_at, expires_at
             ) VALUES (
                trace_current_tenant_id(), $1, $2, $3, $4, now(),
                now() + interval '10 minutes'
             )",
            &[
                &proposal_id,
                &surviving_account_id,
                &absorbed_account_id,
                &absorbed_principal_count,
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;

        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(Some(crate::db::StagedMergeProposal {
            proposal_id,
            absorbed_account_id,
            absorbed_principal_count,
        }))
    }

    async fn execute_merge(
        &self,
        tenant_id: &str,
        surviving_account_id: Uuid,
        proposal_id: Uuid,
    ) -> Result<Option<crate::db::ExecutedMerge>, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;

        // Load + consume the proposal atomically. The conditional UPDATE
        // re-validates OWNERSHIP (surviving_account_id = A, the auth-derived
        // caller), single-use (consumed_at IS NULL), and freshness (expires_at >
        // now()) AND that the surviving account A is still OPEN in one shot: an
        // expired / not-owned / already-consumed / unknown proposal, or a
        // soft-closed A, affects zero rows -> deny. The A-open EXISTS guard is
        // load-bearing: the proposal FK only guarantees A exists, not that it is
        // open, and a soft-close (closed_at) does not trigger ON DELETE CASCADE,
        // so without it B's principals + authenticators could be folded onto a
        // tombstoned account (irreversible identity corruption). Returning Ok(None)
        // before any mutation drops the tx (no commit), so the consume itself rolls
        // back and the proposal stays usable on the benign-not-found paths.
        let consumed = tx
            .query_opt(
                "UPDATE trace_account_merge_proposals SET consumed_at = now()
                  WHERE tenant_id = trace_current_tenant_id()
                    AND proposal_id = $1
                    AND surviving_account_id = $2
                    AND consumed_at IS NULL
                    AND expires_at > now()
                    AND EXISTS (
                        SELECT 1 FROM trace_accounts a
                         WHERE a.tenant_id = trace_current_tenant_id()
                           AND a.account_id = $2
                           AND a.closed_at IS NULL
                    )
                  RETURNING absorbed_account_id",
                &[&proposal_id, &surviving_account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let Some(consumed) = consumed else {
            return Ok(None);
        };
        let absorbed_account_id: Uuid = consumed.get("absorbed_account_id");

        // Re-check B is still open. If B closed between stage and execute, abandon
        // the whole merge: return Ok(None) WITHOUT committing so the consume above
        // (and any reads) roll back and the proposal remains usable.
        let closed_at: Option<chrono::DateTime<chrono::Utc>> = tx
            .query_one(
                "SELECT closed_at FROM trace_accounts
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1",
                &[&absorbed_account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?
            .get("closed_at");
        if closed_at.is_some() {
            return Ok(None);
        }

        // Move B's ACTIVE principal links onto A. PK-column UPDATE; collision-free
        // because (tenant_id, principal_ref) is UNIQUE and a principal has at most
        // one active link.
        let principals_moved = tx
            .execute(
                "UPDATE trace_account_principals
                    SET account_id = $1
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $2
                    AND unlinked_at IS NULL",
                &[&surviving_account_id, &absorbed_account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)? as i64;

        // Re-key B's ACTIVE webauthn credentials onto A (account_id is a non-key
        // column, so a plain UPDATE is safe).
        let webauthn_moved = tx
            .execute(
                "UPDATE trace_webauthn_credentials
                    SET account_id = $1
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $2
                    AND revoked_at IS NULL",
                &[&surviving_account_id, &absorbed_account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)? as i64;

        // Re-key B's ACTIVE NEAR identities onto A, CLEARING payout_designated_at:
        // A may already have its own designated payout, and the partial-unique
        // index forbids two active designations per account. The contributor can
        // re-designate afterward.
        let near_moved = tx
            .execute(
                "UPDATE trace_near_identities
                    SET account_id = $1, payout_designated_at = NULL
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $2
                    AND revoked_at IS NULL",
                &[&surviving_account_id, &absorbed_account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)? as i64;
        let authenticators_moved = webauthn_moved + near_moved;

        // Revoke ALL of B's live sessions (mirror revoke_all_account_sessions):
        // B's credentials now belong to A, so its old sessions must die.
        tx.execute(
            "UPDATE trace_sessions SET revoked_at = now()
              WHERE tenant_id = trace_current_tenant_id()
                AND account_id = $1
                AND revoked_at IS NULL",
            &[&absorbed_account_id],
        )
        .await
        .map_err(DatabaseError::Postgres)?;

        // Close B.
        tx.execute(
            "UPDATE trace_accounts SET closed_at = now()
              WHERE tenant_id = trace_current_tenant_id()
                AND account_id = $1
                AND closed_at IS NULL",
            &[&absorbed_account_id],
        )
        .await
        .map_err(DatabaseError::Postgres)?;

        // Hash-only / label-only audit. Actor is the surviving account A
        // (reserved-prefix). Metadata is COUNTS ONLY: no principal_refs, public
        // keys, or account uuids.
        let actor_ref = crate::account_session::account_actor_ref(
            &crate::account_session::AccountId::from_uuid(surviving_account_id),
        );
        let safe_metadata = serde_json::json!({
            "principals_moved": principals_moved,
            "authenticators_moved": authenticators_moved,
        });
        tx.execute(
            "INSERT INTO trace_account_audit (
                tenant_id, action, actor_ref, outcome, safe_metadata
             ) VALUES (trace_current_tenant_id(), $1, $2, $3, $4)",
            &[&"account_merged", &actor_ref, &"success", &safe_metadata],
        )
        .await
        .map_err(DatabaseError::Postgres)?;

        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(Some(crate::db::ExecutedMerge {
            principals_moved,
            authenticators_moved,
        }))
    }

    async fn list_submissions_needing_gate_decision(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        max_attempts: i32,
        backoff_base_seconds: i64,
        limit: i64,
    ) -> Result<Vec<crate::trace_corpus_storage::GateWorkItem>, DatabaseError> {
        let pool = self
            .gate_driver_pool
            .as_ref()
            .ok_or_else(|| DatabaseError::Pool("gate-driver pool not configured".to_string()))?;
        let client = pool.get().await.map_err(DatabaseError::from)?;
        // No tenant context is set on this connection: the trace_gate_driver
        // role's permissive cross-tenant SELECT policies (migration V36) are
        // what authorize this read across every tenant's submissions.
        let rows = client
            .query(
                // DISTINCT: a submission can carry more than one active
                // submitted_envelope object ref, so the INNER JOIN can fan out
                // to multiple rows per submission. Deduplicate to one work item
                // per (tenant, submission) — otherwise a multi-ref submission
                // wastes LIMIT slots and gets scored/attempted concurrently
                // more than once. `received_at` is included in the projection
                // only so it is a legal DISTINCT + ORDER BY target; it is a
                // per-submission constant, so it does not change dedup
                // cardinality, and it is dropped when mapping to GateWorkItem.
                "SELECT DISTINCT s.tenant_id, s.submission_id, s.received_at
                 FROM trace_submissions s
                 JOIN trace_object_refs o
                   ON o.tenant_id = s.tenant_id
                  AND o.submission_id = s.submission_id
                  AND o.artifact_kind = 'submitted_envelope'
                  AND o.invalidated_at IS NULL
                  AND o.deleted_at IS NULL
                 LEFT JOIN trace_gate_decisions d
                   ON d.tenant_id = s.tenant_id AND d.submission_id = s.submission_id
                 LEFT JOIN trace_gate_evaluation_attempts a
                   ON a.tenant_id = s.tenant_id AND a.submission_id = s.submission_id
                 WHERE d.decision_id IS NULL
                   AND COALESCE(a.attempts, 0) < $1
                   AND (a.last_attempt_at IS NULL
                        OR a.last_attempt_at + make_interval(secs => ($2::bigint)::double precision * POWER(2, COALESCE(a.attempts,0))) <= $3)
                 ORDER BY s.received_at ASC
                 LIMIT $4",
                &[&max_attempts, &backoff_base_seconds, &now, &limit],
            )
            .await
            .map_err(DatabaseError::Postgres)?;

        Ok(rows
            .into_iter()
            .map(|row| crate::trace_corpus_storage::GateWorkItem {
                tenant_id: row.get("tenant_id"),
                submission_id: row.get("submission_id"),
            })
            .collect())
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

/// Pilot-default consent scopes for onboarding device-key grants, used when a
/// device is onboarded via invite (no per-tenant policy template) and as the
/// fail-closed fallback for instance-enrolled devices.
const DEFAULT_ONBOARDING_CONSENT_SCOPES: [&str; 2] = ["debugging_evaluation", "public_attribution"];
/// Pilot-default allowed uses, mirroring `DEFAULT_ONBOARDING_CONSENT_SCOPES`.
const DEFAULT_ONBOARDING_ALLOWED_USES: [&str; 3] =
    ["debugging", "evaluation", "aggregate_analytics"];

/// Normalize a policy-template scope array (serde_json::Value from
/// InstanceUserProvision) into storage strings. Non-array, empty, or
/// non-string-element values fall back to `defaults` (fail closed).
fn normalize_provision_scope_values(value: &serde_json::Value, defaults: &[&str]) -> Vec<String> {
    let fallback = || defaults.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let Some(items) = value.as_array() else {
        return fallback();
    };
    if items.is_empty() {
        return fallback();
    }
    let mut normalized = Vec::with_capacity(items.len());
    for item in items {
        match item.as_str() {
            Some(s) => normalized.push(s.to_string()),
            None => return fallback(),
        }
    }
    normalized
}

async fn upsert_onboarding_device_tenant_access_grant(
    tx: &tokio_postgres::Transaction<'_>,
    tenant_id: &str,
    device_key_id: &str,
    allowed_consent_scopes: &[String],
    allowed_uses: &[String],
) -> Result<(), DatabaseError> {
    let grant_id = onboarding_device_tenant_access_grant_id(tenant_id, device_key_id);
    let principal_ref = onboarding_device_principal_ref(tenant_id, device_key_id);
    let allowed_consent_scopes = serde_json::json!(allowed_consent_scopes);
    let allowed_uses = serde_json::json!(allowed_uses);
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
    fn provision_scopes_normalize_or_fall_back() {
        use serde_json::json;
        let d = ["debugging_evaluation", "public_attribution"];
        assert_eq!(
            normalize_provision_scope_values(
                &json!(["model_training", "debugging_evaluation"]),
                &d
            ),
            vec![
                "model_training".to_string(),
                "debugging_evaluation".to_string()
            ]
        );
        // Empty array, non-array, and mixed-type arrays all fall back.
        assert_eq!(
            normalize_provision_scope_values(&json!([]), &d),
            d.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            normalize_provision_scope_values(&json!("nope"), &d),
            d.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            normalize_provision_scope_values(&json!([1, "x"]), &d),
            d.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
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
            include_str!("../../../../migrations/V32__webauthn_credentials.sql"),
            include_str!("../../../../migrations/V33__near_identities.sql"),
            include_str!("../../../../migrations/V34__account_consolidation.sql"),
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
            include_str!("../../../../migrations/V32__webauthn_credentials.sql"),
            include_str!("../../../../migrations/V33__near_identities.sql"),
            include_str!("../../../../migrations/V34__account_consolidation.sql"),
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
