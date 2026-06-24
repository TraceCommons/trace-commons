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

    /// Resolve the tenant for a WebAuthn `credential_id` via the SAME NARROW
    /// restricted-role resolver pool used by `resolve_login_link_tenant` (separate
    /// role, column-scoped SELECT, no BYPASSRLS). Returns the tenant ONLY. The
    /// discoverable-login path (Task 6) calls this with the credential id parsed
    /// from an UNAUTHENTICATED assertion, BEFORE any tenant context exists, then
    /// re-confirms tenant inside an RLS-scoped tx via
    /// `load_webauthn_credential_for_login`. Fail-closed: an unconfigured resolver
    /// pool MUST error with a safe missing-control name, never fall back to the
    /// runtime pool. NO `ensure_trace_tenant` here: this is a pure read on a
    /// globally-UNIQUE column and MUST NOT write any tenant row for a forged id.
    async fn resolve_credential_tenant(
        &self,
        _credential_id: &str,
    ) -> Result<Option<String>, DatabaseError> {
        Err(DatabaseError::Pool(
            "resolve_credential_tenant not implemented".to_string(),
        ))
    }

    /// Resolve a NEAR access public_key to its tenant via the column-scoped,
    /// restricted-role resolver pool, exactly as `resolve_credential_tenant` does
    /// for webauthn credentials. Returns the tenant ONLY. The NEAR-login path
    /// (Task 7) calls this with the public_key parsed from an UNAUTHENTICATED,
    /// wallet-signed assertion, BEFORE any tenant context exists, then re-confirms
    /// tenant inside an RLS-scoped tx. Fail-closed: an unconfigured resolver pool
    /// MUST error with a safe missing-control name, never fall back to the runtime
    /// pool. NO `ensure_trace_tenant` here: this is a pure read on a
    /// globally-UNIQUE column and MUST NOT write any tenant row for a forged key.
    async fn resolve_near_public_key_tenant(
        &self,
        _public_key: &str,
    ) -> Result<Option<String>, DatabaseError> {
        Err(DatabaseError::Pool(
            "resolve_near_public_key_tenant not implemented".to_string(),
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
    /// that is unexpired, not revoked, and seen within the idle cap, matching
    /// either the CURRENT `token_hash` or a within-grace previous token. On a hit,
    /// bump `last_seen_at` and return the account id together with the session's
    /// `auth_credential_id`. When the matched row aged past the rotation interval
    /// AND the request matched the current token, mint a new secret in the same tx
    /// (rotate `token_hash`, set `prev_token_hash`/`prev_token_valid_until`,
    /// reset `token_issued_at`) and return it as `rotated_secret`. A within-grace
    /// previous-token match does NOT re-rotate. On a miss (including an
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
    ) -> Result<Option<ValidatedSession>, DatabaseError> {
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

    /// Insert a registered passkey for `account_id`. The serialized webauthn-rs
    /// `Passkey` (public key, sign counter, AAGUID, transports) is stored as
    /// JSONB; `credential_id` is the globally-UNIQUE base64url WebAuthn id and
    /// `label` is an optional user-supplied display name. Tenant- + account-
    /// scoped under forced RLS.
    async fn insert_webauthn_credential(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _credential_id: &str,
        _passkey: &serde_json::Value,
        _label: Option<&str>,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool(
            "insert_webauthn_credential not implemented".to_string(),
        ))
    }

    /// Load an ACTIVE credential for the LOGIN (assertion) path by its
    /// `credential_id`, under the already-resolved `tenant_id`. Returns the
    /// account id and serialized `Passkey` JSON, or `None` for an unknown /
    /// revoked / other-tenant credential. Runs inside an RLS-scoped tx; the
    /// `tenant_id = trace_current_tenant_id()` predicate is belt-and-suspenders
    /// on top of forced RLS, and `credential_id` is globally UNIQUE.
    async fn load_webauthn_credential_for_login(
        &self,
        _tenant_id: &str,
        _credential_id: &str,
    ) -> Result<Option<WebauthnCredentialRow>, DatabaseError> {
        Err(DatabaseError::Pool(
            "load_webauthn_credential_for_login not implemented".to_string(),
        ))
    }

    /// Persist the post-finish `Passkey` after a successful assertion so the
    /// updated sign counter (clone-detection state) is durable, and stamp
    /// `last_used_at`. Tenant-scoped under forced RLS; only an active
    /// (unrevoked) credential is updated.
    async fn update_webauthn_credential_after_login(
        &self,
        _tenant_id: &str,
        _credential_id: &str,
        _passkey: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool(
            "update_webauthn_credential_after_login not implemented".to_string(),
        ))
    }

    /// Issue a browser session for a passkey login that has ALREADY been verified
    /// (Task 6). Inserts a `trace_sessions` row (hash-only `token_hash`,
    /// `client_kind = 'passkey'`, `auth_credential_id` = the base64url credential
    /// id STRING that authenticated the session, `token_issued_at` defaulted) plus
    /// a hash-only audit row, in ONE RLS-scoped tx under the already-resolved
    /// tenant. The caller MUST have verified the assertion (and thereby the
    /// credential's FK-backed tenant) before calling this; there is therefore NO
    /// `ensure_trace_tenant` here — the tenant provably exists via the credential
    /// row, and an UPSERT before verification would let a forged assertion spray
    /// tenant rows. The raw session secret never reaches the database; only its
    /// sha256 `token_hash` is stored.
    async fn issue_passkey_session(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _session: NewSession<'_>,
        _auth_credential_id: &str,
        _audit: RedeemAudit,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool(
            "issue_passkey_session not implemented".to_string(),
        ))
    }

    /// Issue a browser session for a NEAR wallet login that has ALREADY been
    /// verified (Slice 3a Task 7). Mirrors [`Database::issue_passkey_session`]
    /// exactly except `client_kind = 'near'` and `auth_credential_id` carries the
    /// NEAR access key (`ed25519:...` public key) that authenticated the session.
    /// Inserts a `trace_sessions` row (hash-only `token_hash`, `token_issued_at`
    /// defaulted) plus a hash-only audit row in ONE RLS-scoped tx under the
    /// already-resolved tenant. The caller MUST have verified the NEP-413 assertion
    /// AND loaded the identity under that tenant before calling this; there is
    /// therefore NO `ensure_trace_tenant` here — the tenant provably exists via the
    /// identity row, and an UPSERT before verification would let a forged assertion
    /// spray tenant rows. The raw session secret never reaches the database; only
    /// its sha256 `token_hash` is stored, and the public key is never audited.
    async fn issue_near_session(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _session: NewSession<'_>,
        _auth_credential_id: &str,
        _audit: RedeemAudit,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool(
            "issue_near_session not implemented".to_string(),
        ))
    }

    /// List the ACTIVE (unrevoked) credentials for `account_id`, oldest first.
    /// Label- and timestamp-only; no key material. Tenant- + account-scoped
    /// under forced RLS so another account's credentials are never returned.
    async fn list_account_credentials(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
    ) -> Result<Vec<AccountCredentialSummary>, DatabaseError> {
        Err(DatabaseError::Pool(
            "list_account_credentials not implemented".to_string(),
        ))
    }

    /// Rename one of `account_id`'s ACTIVE credentials. Returns whether a row was
    /// affected (`false` for an unknown / revoked / other-account credential).
    /// Tenant- + account-scoped under forced RLS so a caller can never rename a
    /// credential they do not own.
    async fn rename_account_credential(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _credential_id: &str,
        _label: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        Err(DatabaseError::Pool(
            "rename_account_credential not implemented".to_string(),
        ))
    }

    /// Soft-revoke one of `account_id`'s ACTIVE credentials and report how many of
    /// the account's credentials remain active afterwards. `removed` is `false`
    /// for an unknown / already-revoked / other-account credential. Tenant- +
    /// account-scoped under forced RLS so a caller can never revoke a credential
    /// they do not own; the remaining-count lets the caller refuse to remove the
    /// last passkey.
    async fn revoke_account_credential(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _credential_id: &str,
    ) -> Result<RevokeCredentialResult, DatabaseError> {
        Err(DatabaseError::Pool(
            "revoke_account_credential not implemented".to_string(),
        ))
    }

    /// Insert a registered NEAR access key for `account_id`. `public_key` is the
    /// globally-UNIQUE NEAR access key (`ed25519:...`) used by the unauthenticated
    /// login resolver to map key -> tenant; `near_account_id` is the human-readable
    /// NEAR account label (attribution only); `label` is an optional user-supplied
    /// display name. Tenant- + account-scoped under forced RLS.
    async fn insert_near_identity(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _public_key: &str,
        _near_account_id: &str,
        _label: Option<&str>,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool(
            "insert_near_identity not implemented".to_string(),
        ))
    }

    /// Load an ACTIVE NEAR identity for the LOGIN (wallet-assertion) path by its
    /// `public_key`, under the already-resolved `tenant_id`. Returns the account id
    /// and the human-readable NEAR account id, or `None` for an unknown / revoked /
    /// other-tenant key. Runs inside an RLS-scoped tx; the `tenant_id =
    /// trace_current_tenant_id()` predicate is belt-and-suspenders on top of forced
    /// RLS, and `public_key` is globally UNIQUE. NO `ensure_trace_tenant` here: this
    /// mirrors `load_webauthn_credential_for_login` — the tenant already exists for
    /// any registered identity, and the resolver mapped public_key -> tenant first.
    async fn load_near_identity_for_login(
        &self,
        _tenant_id: &str,
        _public_key: &str,
    ) -> Result<Option<NearIdentityRow>, DatabaseError> {
        Err(DatabaseError::Pool(
            "load_near_identity_for_login not implemented".to_string(),
        ))
    }

    /// Stamp `last_used_at` on an ACTIVE NEAR identity after a successful
    /// wallet-assertion login. Tenant-scoped under forced RLS; only an active
    /// (unrevoked) identity is updated. `public_key` is globally UNIQUE.
    async fn touch_near_identity_last_used(
        &self,
        _tenant_id: &str,
        _public_key: &str,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool(
            "touch_near_identity_last_used not implemented".to_string(),
        ))
    }

    /// List the ACTIVE (unrevoked) NEAR identities for `account_id`, oldest first.
    /// Label- and timestamp-only plus the public attribution fields (`public_key`,
    /// `near_account_id`); no secret material. Tenant- + account-scoped under forced
    /// RLS so another account's identities are never returned.
    async fn list_account_near_identities(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
    ) -> Result<Vec<NearIdentitySummary>, DatabaseError> {
        Err(DatabaseError::Pool(
            "list_account_near_identities not implemented".to_string(),
        ))
    }

    /// Rename one of `account_id`'s ACTIVE NEAR identities. Returns whether a row was
    /// affected (`false` for an unknown / revoked / other-account key). Tenant- +
    /// account-scoped under forced RLS so a caller can never rename an identity they
    /// do not own.
    async fn rename_account_near_identity(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _public_key: &str,
        _label: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        Err(DatabaseError::Pool(
            "rename_account_near_identity not implemented".to_string(),
        ))
    }

    /// Soft-revoke one of `account_id`'s ACTIVE NEAR identities and report how many
    /// strong authenticators (webauthn credentials + NEAR identities) remain active
    /// for the account afterwards, computed in the SAME tx as the revoke. `removed`
    /// is `false` for an unknown / already-revoked / other-account key. Tenant- +
    /// account-scoped under forced RLS so a caller can never revoke an identity they
    /// do not own; the remaining-strong count lets the caller refuse to remove the
    /// last strong authenticator.
    async fn revoke_account_near_identity(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _public_key: &str,
    ) -> Result<RevokeNearResult, DatabaseError> {
        Err(DatabaseError::Pool(
            "revoke_account_near_identity not implemented".to_string(),
        ))
    }

    /// Count `account_id`'s ACTIVE strong authenticators: webauthn credentials plus
    /// NEAR identities, summed in ONE RLS-scoped tx. Lets a caller enforce
    /// "keep at least one strong authenticator" before revoking the last passkey or
    /// NEAR key. Tenant- + account-scoped under forced RLS.
    async fn count_active_strong_authenticators(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
    ) -> Result<i64, DatabaseError> {
        Err(DatabaseError::Pool(
            "count_active_strong_authenticators not implemented".to_string(),
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

/// Outcome of a successful [`Database::validate_session`] lookup. Carries the
/// durable account id and the `auth_credential_id` recorded on the session row
/// (the base64url WebAuthn credential id that authenticated a passkey session, or
/// `None` for a device-link session). The credential id is a public identifier,
/// never key material.
#[derive(Debug, Clone)]
pub struct ValidatedSession {
    pub account_id: uuid::Uuid,
    pub auth_credential_id: Option<String>,
    /// The `client_kind` recorded on the session row (`'web'` for a device-link
    /// session, `'passkey'` for a passkey login, `'near'` for a NEAR login). Used
    /// by the strong-authenticator gate to classify session strength: a `'web'`
    /// session is WEAK, a `'passkey'`/`'near'` session is STRONG. A public label,
    /// never key material.
    pub client_kind: String,
    /// Set to the NEW raw session secret when rotation-on-use fired for this
    /// request (the matched row's `token_issued_at` aged past the rotation
    /// interval and the request matched the CURRENT `token_hash`), and `None`
    /// otherwise. When `Some`, the caller MUST emit a fresh `Set-Cookie` so the
    /// browser swaps to the new secret before the short prev-token grace lapses;
    /// the auth middleware is the single guaranteed attach point. A request that
    /// matched the previous token within grace (multi-tab) does NOT re-rotate and
    /// leaves this `None`.
    pub rotated_secret: Option<String>,
}

/// A registered passkey resolved for the LOGIN (assertion) path. Carries only
/// the durable account id and the serialized webauthn-rs `Passkey` JSON (public
/// key, sign counter, AAGUID, transports); never PII. The loader runs under the
/// already-resolved tenant's RLS, so the row is tenant-scoped by construction.
#[derive(Debug, Clone)]
pub struct WebauthnCredentialRow {
    pub account_id: uuid::Uuid,
    pub passkey: serde_json::Value,
}

/// A single ACTIVE credential as surfaced in the account's credential list.
/// Label-only and timestamp-only: no public-key material or counter state. The
/// `credential_id` is the base64url WebAuthn id (a label, not a secret).
#[derive(Debug, Clone)]
pub struct AccountCredentialSummary {
    pub credential_id: String,
    pub label: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Outcome of revoking one of an account's credentials. `removed` is whether the
/// soft-delete affected a row (false for an unknown / already-revoked / other
/// account's credential); `remaining` is the count of the account's credentials
/// still active after the revoke, so the caller can refuse to remove the last
/// passkey if policy requires it.
#[derive(Debug, Clone, Copy)]
pub struct RevokeCredentialResult {
    pub removed: bool,
    pub remaining: i64,
}

/// A registered NEAR identity resolved for the LOGIN (wallet-assertion) path.
/// Carries only the durable account id and the human-readable `near_account_id`
/// (attribution only); never key material or PII. The loader runs under the
/// already-resolved tenant's RLS, so the row is tenant-scoped by construction.
#[derive(Debug, Clone)]
pub struct NearIdentityRow {
    pub account_id: uuid::Uuid,
    pub near_account_id: String,
}

/// A single ACTIVE NEAR identity as surfaced in the account's identity list.
/// Label-, timestamp-, and public-attribution-only: `public_key` is the NEAR
/// access key (a public identifier, not a secret) and `near_account_id` is the
/// human-readable NEAR account label.
#[derive(Debug, Clone)]
pub struct NearIdentitySummary {
    pub public_key: String,
    pub near_account_id: String,
    pub label: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Outcome of revoking one of an account's NEAR identities. `removed` is whether
/// the soft-delete affected a row (false for an unknown / already-revoked / other
/// account's key); `remaining_strong` is the count of the account's strong
/// authenticators (webauthn credentials + NEAR identities) still active after the
/// revoke, so the caller can refuse to remove the last strong authenticator.
#[derive(Debug, Clone, Copy)]
pub struct RevokeNearResult {
    pub removed: bool,
    pub remaining_strong: i64,
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
