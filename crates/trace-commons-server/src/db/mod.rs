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

    /// Create-or-reuse the durable account for `principal_ref` within `tenant_id`.
    /// Returns the stable `account_id`. Idempotent: repeated calls for the same
    /// active principal return the same account. Tolerates a concurrent racing
    /// mint by re-selecting after an `ON CONFLICT DO NOTHING` link insert.
    async fn create_or_reuse_account(
        &self,
        _tenant_id: &str,
        _principal_ref: &str,
    ) -> Result<uuid::Uuid, DatabaseError> {
        Err(DatabaseError::Pool(
            "create_or_reuse_account not implemented".to_string(),
        ))
    }

    /// Count this principal's outstanding (unconsumed, unexpired) login links.
    /// Used by the mint endpoint to cap per-principal outstanding links.
    async fn count_outstanding_login_links(
        &self,
        _tenant_id: &str,
        _created_principal_ref: &str,
    ) -> Result<i64, DatabaseError> {
        Err(DatabaseError::Pool(
            "count_outstanding_login_links not implemented".to_string(),
        ))
    }

    /// Insert a single-use login link. Stores ONLY the `code_hash`
    /// (sha256:-shaped); the raw code never reaches the database.
    async fn insert_login_link(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _code_hash: &str,
        _created_principal_ref: &str,
        _expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool(
            "insert_login_link not implemented".to_string(),
        ))
    }

    /// Append a hash-only / label-only row to `trace_account_audit`. Actor and
    /// metadata MUST be reserved-prefix or hash-shaped; never raw codes/URLs.
    async fn append_account_audit(
        &self,
        _tenant_id: &str,
        _action: &str,
        _actor_ref: &str,
        _outcome: &str,
        _safe_metadata: serde_json::Value,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool(
            "append_account_audit not implemented".to_string(),
        ))
    }

    /// Resolve the tenant for a login `code_hash` via the NARROW restricted-role
    /// resolver pool (separate role, column-scoped SELECT, no BYPASSRLS).
    /// Returns the tenant only. Fail-closed: an unconfigured resolver pool MUST
    /// error with a safe missing-control name, never fall back to the runtime
    /// pool. The caller re-confirms tenant inside an RLS-scoped tx before any
    /// write.
    async fn resolve_login_link_tenant(
        &self,
        _code_hash: &str,
    ) -> Result<Option<String>, DatabaseError> {
        Err(DatabaseError::Pool(
            "resolve_login_link_tenant not implemented".to_string(),
        ))
    }

    /// Atomically redeem a single-use login link: consume + session insert +
    /// audit insert in ONE RLS-scoped transaction, so redeem is all-or-nothing.
    ///
    /// The conditional consume UPDATE is ALWAYS executed (never a
    /// SELECT-then-branch): unknown / expired / already-consumed / wrong-tenant
    /// codes all affect zero rows → `Ok(None)`, and the transaction commits with
    /// nothing changed, leaving the link UNconsumed and retryable. On a winning
    /// consume, the session row (hash-only `session_token_hash`) and the hash-only
    /// audit row are inserted and the whole transaction commits together; any
    /// failure rolls everything back (link stays reusable, no orphaned session, no
    /// un-audited state change). The `tenant_id = trace_current_tenant_id()`
    /// predicate is belt-and-suspenders on top of RLS. The raw code and the raw
    /// session secret never reach the database. `session_id` is server-assigned.
    async fn redeem_login_link(
        &self,
        _tenant_id: &str,
        _code_hash: &str,
        _session: NewSession<'_>,
        _audit: RedeemAudit,
    ) -> Result<Option<RedeemedSession>, DatabaseError> {
        Err(DatabaseError::Pool(
            "redeem_login_link not implemented".to_string(),
        ))
    }

    /// Validate a browser session by its `token_hash` (sha256 of the secret part
    /// of the cookie, NOT the whole cookie value) under the caller-asserted
    /// `tenant_id`. Inside an RLS-scoped tx: select the account for a session
    /// that is unexpired, not revoked, and seen within the idle cap. On a hit,
    /// bump `last_seen_at` and return the account id. On a miss (including an
    /// idle-capped row, which is auto-revoked) return `None`. Any store/DB error
    /// surfaces as `Err`; callers MUST treat both `None` and `Err` as a denial.
    ///
    /// Safety of the client-supplied tenant: `token_hash` is globally UNIQUE and
    /// bound to exactly one real `(tenant, account, session)`. A forged or
    /// mismatched tenant simply scopes the RLS lookup to a tenant where this
    /// hash does not exist, so it finds no row and fails closed.
    async fn validate_session(
        &self,
        _tenant_id: &str,
        _token_hash: &str,
    ) -> Result<Option<uuid::Uuid>, DatabaseError> {
        Err(DatabaseError::Pool(
            "validate_session not implemented".to_string(),
        ))
    }

    /// Revoke the CURRENT browser session identified by its `token_hash` (sha256
    /// of the secret part of the cookie). Idempotent: an already-revoked or
    /// unknown hash affects zero rows. Returns the number of rows revoked (0 or 1).
    /// The `tenant_id = trace_current_tenant_id()` predicate is belt-and-suspenders
    /// on top of forced RLS; `token_hash` is globally UNIQUE.
    async fn revoke_current_session(
        &self,
        _tenant_id: &str,
        _token_hash: &str,
    ) -> Result<u64, DatabaseError> {
        Err(DatabaseError::Pool(
            "revoke_current_session not implemented".to_string(),
        ))
    }

    /// Revoke ALL live sessions belonging to `account_id` (sign-out-everywhere).
    /// Only the caller's own account is ever affected: the caller passes an
    /// auth-derived `account_id` and the UPDATE is tenant- + account-scoped under
    /// forced RLS. Returns the number of sessions revoked.
    async fn revoke_all_account_sessions(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
    ) -> Result<u64, DatabaseError> {
        Err(DatabaseError::Pool(
            "revoke_all_account_sessions not implemented".to_string(),
        ))
    }

    /// Resolve the active account a device `principal_ref` is linked to (bearer
    /// path). Active-membership only (`unlinked_at IS NULL`, Hardening A). `None`
    /// when the principal is unlinked or links to no account.
    async fn resolve_account_for_principal(
        &self,
        _tenant_id: &str,
        _principal_ref: &str,
    ) -> Result<Option<uuid::Uuid>, DatabaseError> {
        Err(DatabaseError::Pool(
            "resolve_account_for_principal not implemented".to_string(),
        ))
    }

    /// Expand an account's ACTIVE principal memberships — the ONLY sanctioned
    /// ownership-bearing expansion (Hardening A). MUST filter `unlinked_at IS
    /// NULL`; an unlinked principal is absent from the returned set. Returns the
    /// `AccountPrincipalSet` newtype directly so the only mint site for an
    /// ownership-bearing set stays inside this crate (Hardening C); callers in
    /// the bins cannot construct one themselves.
    async fn expand_account_principals(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
    ) -> Result<crate::account_session::AccountPrincipalSet, DatabaseError> {
        Err(DatabaseError::Pool(
            "expand_account_principals not implemented".to_string(),
        ))
    }
}

/// The session row to create on a winning redeem. `token_hash` is sha256-shaped;
/// the raw secret never reaches the database. `session_id` is server-assigned by
/// the implementation, never carried here.
#[derive(Debug, Clone, Copy)]
pub struct NewSession<'a> {
    pub token_hash: &'a str,
    pub client_kind: &'a str,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// The hash-only / label-only audit row written inside the redeem transaction.
/// The actor is derived by the implementation from the consumed account id;
/// callers supply only the action label, outcome label, and safe metadata.
#[derive(Debug, Clone)]
pub struct RedeemAudit {
    pub action: String,
    pub outcome: String,
    pub metadata: serde_json::Value,
}

/// Result of a successful atomic login-link redeem. Carries only the durable
/// account id and the server-assigned session id; never the raw code or any
/// secret material.
#[derive(Debug, Clone)]
pub struct RedeemedSession {
    pub account_id: uuid::Uuid,
    pub session_id: uuid::Uuid,
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

pub async fn connect_from_config(
    config: &DatabaseConfig,
) -> Result<Arc<dyn Database>, DatabaseError> {
    let backend = postgres::PgBackend::new(config).await?;
    backend.run_migrations().await?;
    Ok(Arc::new(backend) as Arc<dyn Database>)
}
