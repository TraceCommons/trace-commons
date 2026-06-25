//! TraceCommons server-owned database facade.

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::DatabaseConfig;
use crate::error::DatabaseError;
use crate::trace_corpus_storage::TraceCorpusStore;

pub mod postgres;

mod trace_corpus_common;
mod trace_corpus_pg;

/// Safe structural diagnostics for PostgreSQL TraceCommons RLS readiness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceCorpusRlsDiagnostics {
    pub expected_table_count: usize,
    pub rls_enabled_count: usize,
    pub force_rls_enabled_count: usize,
    pub policy_installed_count: usize,
    pub missing_policy_tables: Vec<String>,
    pub rls_disabled_tables: Vec<String>,
    pub force_rls_disabled_tables: Vec<String>,
    pub policy_expression_mismatch_tables: Vec<String>,
    pub current_role_hash: String,
    pub current_role_bypasses_rls: bool,
    pub current_role_owns_trace_tables: bool,
    pub tenant_context_transaction_local: bool,
}

impl TraceCorpusRlsDiagnostics {
    pub fn rls_ready(&self) -> bool {
        self.missing_policy_tables.is_empty()
            && self.rls_disabled_tables.is_empty()
            && self.policy_expression_mismatch_tables.is_empty()
            && self.policy_installed_count == self.expected_table_count
            && self.rls_enabled_count == self.expected_table_count
            && !self.current_role_bypasses_rls
            && !self.current_role_owns_trace_tables
            && self.tenant_context_transaction_local
    }

    pub fn force_rls_ready(&self) -> bool {
        self.force_rls_disabled_tables.is_empty()
            && self.force_rls_enabled_count == self.expected_table_count
    }

    pub fn production_ready(&self) -> bool {
        self.rls_ready() && self.force_rls_ready()
    }

    pub fn runtime_role_matches_expected_hash(&self, expected_hash: Option<&str>) -> bool {
        expected_hash.is_none_or(|expected_hash| self.current_role_hash == expected_hash)
    }

    pub fn production_ready_with_expected_runtime_role(&self, expected_hash: Option<&str>) -> bool {
        self.production_ready() && self.runtime_role_matches_expected_hash(expected_hash)
    }
}

#[async_trait]
pub trait Database: TraceCorpusStore + Send + Sync {
    async fn run_migrations(&self) -> Result<(), DatabaseError>;

    async fn trace_corpus_rls_diagnostics(
        &self,
    ) -> Result<Option<TraceCorpusRlsDiagnostics>, DatabaseError> {
        Ok(None)
    }

    /// Upsert a contributor's public profile (display handle + optional bio).
    /// Creates a row if none exists for `(tenant_id, principal_ref)`; otherwise
    /// updates handle/bio and bumps `update_count` + `last_updated_at`. Always
    /// clears `withdrawn_at` so a withdrawn contributor can re-opt-in.
    ///
    /// Caller is responsible for handle/bio validation. The unique
    /// `(tenant_id, handle_normalized)` constraint may surface as
    /// [`DatabaseError`] when a different principal already claimed the
    /// handle; callers should map that to a 409 conflict.
    async fn upsert_contributor_profile(
        &self,
        _tenant_id: &str,
        _principal_ref: &str,
        _display_handle: &str,
        _handle_normalized: &str,
        _bio: Option<&str>,
    ) -> Result<ContributorProfileRow, DatabaseError> {
        Err(DatabaseError::Pool(
            "upsert_contributor_profile not implemented".to_string(),
        ))
    }

    /// Soft-delete by stamping `withdrawn_at = NOW()`. Idempotent. Returns
    /// `Ok(false)` if no row exists for `(tenant_id, principal_ref)`.
    async fn withdraw_contributor_profile(
        &self,
        _tenant_id: &str,
        _principal_ref: &str,
    ) -> Result<bool, DatabaseError> {
        Err(DatabaseError::Pool(
            "withdraw_contributor_profile not implemented".to_string(),
        ))
    }

    /// Append an audit row for any profile mutation (opt_in / update /
    /// withdraw / rejected). Caller passes the action verb and any
    /// human-readable reason that is safe to surface back to the
    /// contributor.
    async fn append_contributor_profile_audit(
        &self,
        _tenant_id: &str,
        _principal_ref: &str,
        _action: &str,
        _handle_normalized: Option<&str>,
        _reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool(
            "append_contributor_profile_audit not implemented".to_string(),
        ))
    }

    /// Compute the per-contributor leaderboard inputs for the given
    /// community tenant set and window. Implementations set the RLS tenant
    /// GUC for each tenant, so aggregation still respects per-table RLS.
    /// Applies the `min_cell_count` threshold at the SQL level —
    /// contributors with fewer than `min_cell_count` accepted submissions
    /// in-window are not returned. Noise / privacy-budget integration is
    /// deferred to a follow-up slice.
    async fn compute_leaderboard_inputs(
        &self,
        _window_days: i32,
        _min_cell_count: i64,
        _tenant_ids: &[String],
    ) -> Result<Vec<LeaderboardContributorRow>, DatabaseError> {
        Err(DatabaseError::Pool(
            "compute_leaderboard_inputs not implemented".to_string(),
        ))
    }

    /// Compute the corpus-wide aggregate summary for the given community
    /// tenant set and window. Crosses tenants the same way
    /// `compute_leaderboard_inputs` does.
    async fn compute_corpus_analytics_summary(
        &self,
        _window_days: i32,
        _tenant_ids: &[String],
    ) -> Result<CorpusAnalyticsSummary, DatabaseError> {
        Err(DatabaseError::Pool(
            "compute_corpus_analytics_summary not implemented".to_string(),
        ))
    }

    /// Insert a pre-rendered leaderboard snapshot. `contents_jsonb` is
    /// the wire-shape payload the read endpoints will serve verbatim.
    async fn insert_leaderboard_snapshot(
        &self,
        _snapshot: LeaderboardSnapshotWrite,
    ) -> Result<LeaderboardSnapshotRow, DatabaseError> {
        Err(DatabaseError::Pool(
            "insert_leaderboard_snapshot not implemented".to_string(),
        ))
    }

    /// Fetch the most recent snapshot matching `(window_label, metric)`.
    /// Returns `Ok(None)` if no snapshot has ever been computed.
    async fn latest_leaderboard_snapshot(
        &self,
        _window_label: &str,
        _metric: &str,
    ) -> Result<Option<LeaderboardSnapshotRow>, DatabaseError> {
        Err(DatabaseError::Pool(
            "latest_leaderboard_snapshot not implemented".to_string(),
        ))
    }

    async fn insert_device_key(
        &self,
        _device_key: DeviceKeyWrite,
    ) -> Result<DeviceKeyRecord, DatabaseError> {
        Err(DatabaseError::Pool(
            "insert_device_key not implemented".to_string(),
        ))
    }

    async fn get_device_key(
        &self,
        _tenant_id: &str,
        _device_key_id: &str,
    ) -> Result<Option<DeviceKeyRecord>, DatabaseError> {
        Err(DatabaseError::Pool(
            "get_device_key not implemented".to_string(),
        ))
    }

    async fn list_device_keys(
        &self,
        _tenant_id: &str,
    ) -> Result<Vec<DeviceKeyRecord>, DatabaseError> {
        Err(DatabaseError::Pool(
            "list_device_keys not implemented".to_string(),
        ))
    }

    async fn revoke_device_key(
        &self,
        _tenant_id: &str,
        _device_key_id: &str,
    ) -> Result<Option<DeviceKeyRecord>, DatabaseError> {
        Err(DatabaseError::Pool(
            "revoke_device_key not implemented".to_string(),
        ))
    }

    async fn onboard_device_key(
        &self,
        _device_key: DeviceKeyWrite,
        _max_uses: i32,
    ) -> Result<OnboardDeviceKeyRecord, OnboardDeviceKeyError> {
        Err(OnboardDeviceKeyError::Database(DatabaseError::Pool(
            "onboard_device_key not implemented".to_string(),
        )))
    }

    /// Provision a tenant for an instance user (no-account fallback path).
    ///
    /// In a single tenant-scoped transaction, this op:
    /// 1. Ensures the `trace_tenants` row exists.
    /// 2. Stamps the contribution policy from the supplied template
    ///    (`ON CONFLICT (tenant_id) DO NOTHING` — never overwrites an existing policy).
    /// 3. Registers the device key (`ON CONFLICT (device_key_id) DO NOTHING`).
    ///
    /// The operation is fully idempotent: re-running with the same inputs is safe.
    async fn enroll_instance_user(
        &self,
        p: InstanceUserProvision,
    ) -> Result<(), DatabaseError>;

    /// Atomically deduplicate a user enrollment against `trace_instance_enrollments`
    /// and enforce a per-instance cap.
    ///
    /// The op runs in one instance-scoped transaction (setting
    /// `trace_commons.instance_subject` transaction-locally). It first checks
    /// for an existing `(instance_subject_hash, user_subject_hash)` row — if
    /// found, it returns `ExistingUser` without consuming cap. Otherwise it
    /// count-checks the cap before inserting with `ON CONFLICT DO NOTHING`.
    ///
    /// **Race note:** a concurrent burst of DISTINCT new users could each read
    /// `count < cap` and all insert, overshooting the cap by the concurrency
    /// width. For the pilot's per-instance rate limit this is acceptable. If
    /// strict capping is later required, take an advisory lock on
    /// `hashtext(instance_subject_hash)` at the top of the transaction.
    ///
    async fn reserve_instance_enrollment(
        &self,
        instance_subject_hash: &str,
        user_subject_hash: &str,
        tenant_id: &str,
        max_enrollments: i64,
    ) -> Result<InstanceEnrollmentOutcome, DatabaseError>;
}

/// Per-contributor row returned by [`Database::compute_leaderboard_inputs`].
/// The `*_in_window` columns count over the window the caller passed; the
/// `total_*` columns are all-time.
#[derive(Debug, Clone, PartialEq)]
pub struct LeaderboardContributorRow {
    pub tenant_id: String,
    pub principal_ref: String,
    pub display_handle: String,
    pub handle_normalized: String,
    pub bio: Option<String>,
    pub public_since: chrono::DateTime<chrono::Utc>,
    pub accepted_in_window: i64,
    pub credit_in_window: f64,
    pub total_accepted: i64,
    pub total_credit: f64,
}

/// Corpus-wide aggregates for [`Database::compute_corpus_analytics_summary`].
/// All counts are pre-noise; the snapshot worker is the right place to
/// apply Laplace noise / privacy-budget accounting in a follow-up slice.
#[derive(Debug, Clone, PartialEq)]
pub struct CorpusAnalyticsSummary {
    pub total_submissions: i64,
    pub total_accepted: i64,
    pub total_rejected: i64,
    /// Decimal in [0, 1]. Zero when there are no submissions.
    pub accept_rate: f64,
    /// `(bucket_micros, count)` sorted ascending by bucket. Bucket
    /// width is 100_000 micros (10 buckets across [0, 1_000_000]).
    pub novelty_histogram: Vec<(i64, i64)>,
    /// `(outcome_label, count)` sorted descending by count.
    /// Labels: `both_passed`, `novelty_failed`, `perplexity_failed`,
    /// `both_failed`.
    pub gate_outcomes: Vec<(String, i64)>,
}

/// Write-shape for [`Database::insert_leaderboard_snapshot`]. The caller
/// computes `contents_sha256` and `noise_seed_hash`; the DB owns
/// `computed_at`.
#[derive(Debug, Clone)]
pub struct LeaderboardSnapshotWrite {
    pub snapshot_id: uuid::Uuid,
    pub window_label: String,
    pub metric: String,
    pub contents: serde_json::Value,
    pub contents_sha256: String,
    pub min_cell_count: i32,
    pub noise_seed_hash: String,
}

/// Read-shape returned by [`Database::insert_leaderboard_snapshot`] and
/// [`Database::latest_leaderboard_snapshot`].
#[derive(Debug, Clone)]
pub struct LeaderboardSnapshotRow {
    pub snapshot_id: uuid::Uuid,
    pub computed_at: chrono::DateTime<chrono::Utc>,
    pub window_label: String,
    pub metric: String,
    pub contents: serde_json::Value,
    pub contents_sha256: String,
    pub min_cell_count: i32,
    pub noise_seed_hash: String,
}

/// Row returned by [`Database::upsert_contributor_profile`]. Mirrors the
/// columns of `trace_contributor_profiles` that the API surfaces back
/// to the contributor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributorProfileRow {
    pub tenant_id: String,
    pub principal_ref: String,
    pub display_handle: String,
    pub handle_normalized: String,
    pub bio: Option<String>,
    pub public_since: chrono::DateTime<chrono::Utc>,
    pub last_updated_at: chrono::DateTime<chrono::Utc>,
    pub update_count: i32,
}

#[derive(Debug, Clone)]
pub struct DeviceKeyWrite {
    pub device_key_id: String,
    pub tenant_id: String,
    pub public_key: String,
    pub invite_subject_hash: String,
    pub client_info: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceKeyRecord {
    pub device_key_id: String,
    pub tenant_id: String,
    pub public_key: String,
    pub invite_subject_hash: String,
    pub client_info: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardDeviceKeyStatus {
    Registered,
    Idempotent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OnboardDeviceKeyRecord {
    pub device_key: DeviceKeyRecord,
    pub status: OnboardDeviceKeyStatus,
}

#[derive(Debug, thiserror::Error)]
pub enum OnboardDeviceKeyError {
    #[error("invite not valid")]
    InviteNotValid,
    #[error("invite already consumed")]
    InviteAlreadyConsumed,
    #[error("database error: {0}")]
    Database(#[from] DatabaseError),
}

/// Input to [`Database::enroll_instance_user`].
#[derive(Debug, Clone)]
pub struct InstanceUserProvision {
    pub device_key_id: String,
    pub tenant_id: String,
    pub public_key: String,
    /// The `sha256:…` hash identifying the instance subject; used as
    /// `invite_subject_hash` on the device key row and embedded in the
    /// `updated_by_principal_ref` audit column of the policy row.
    pub instance_subject_hash: String,
    pub client_info: serde_json::Value,
    pub policy_version: String,
    pub allowed_consent_scopes: serde_json::Value,
    pub allowed_uses: serde_json::Value,
}

pub async fn connect_from_config(
    config: &DatabaseConfig,
) -> Result<Arc<dyn Database>, DatabaseError> {
    let backend = postgres::PgBackend::new(config).await?;
    backend.run_migrations().await?;
    Ok(Arc::new(backend) as Arc<dyn Database>)
}

/// Outcome of [`Database::reserve_instance_enrollment`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceEnrollmentOutcome {
    /// The user was not previously enrolled and has been added to the ledger.
    NewlyEnrolled,
    /// The user was already enrolled; no cap was consumed.
    ExistingUser,
    /// The per-instance cap is reached; the user was NOT enrolled.
    CapExceeded,
}
