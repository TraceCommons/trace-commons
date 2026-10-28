// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! TraceCommons server-owned database facade.

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::DatabaseConfig;
use crate::error::DatabaseError;
use crate::trace_corpus_storage::{
    TraceCorpusStore, TraceCreditSettlementBatchWrite, TraceNearCreditOutboxItemWrite,
};
use crate::trace_invite_registry::InviteTenantMode;

pub mod postgres;
pub mod postgres_inference_connection;
mod postgres_payout;

mod trace_corpus_common;
mod trace_corpus_pg;

pub(crate) use trace_corpus_pg::{
    insert_credit_settlement_batch_on_tx, record_source_submission_withdrawal_on_tx,
    withdraw_source_session_on_tx,
};

pub use postgres::InviteRedemption;

/// Result of redeeming a durable invite into an authenticated account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountInviteRedemption {
    Invited { trust_version: i64 },
    InvalidInvite,
    AccountIneligible,
    IdempotencyConflict,
}

/// Insert payload for an invite grant. Mirrors `InviteEntry` minus
/// `revoked_at`, which is only ever set by `revoke_invite_grant`.
#[derive(Debug, Clone)]
pub struct InviteGrantWrite {
    pub invite_subject_hash: String,
    pub policy_label: String,
    pub tenant_mode: InviteTenantMode,
    pub fixed_tenant_id: Option<String>,
    pub tenant_template_id: Option<String>,
    pub policy_version: String,
    pub allowed_consent_scopes: Vec<String>,
    pub allowed_uses: Vec<String>,
    pub max_uses: u32,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub issuance_source: String,
    pub issued_by_label: Option<String>,
    pub credential_binding_hash: Option<String>,
    pub note_label: Option<String>,
    /// Public issuer name shown by the non-redeeming lookup (V103).
    pub issuer_display_name: Option<String>,
    /// Operator-set `(min, max)` credit points per accepted trace (V103).
    pub credit_range: Option<(i64, i64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteGrantInsertOutcome {
    Inserted,
    /// The partial unique index refused the write: this credential already has
    /// a live invite in this pool.
    CredentialAlreadyBound,
    /// An invite with this hash already exists. Makes the file import
    /// idempotent.
    AlreadyExists,
}

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

    /// `rls_ready` and `force_rls_ready` for the tables checked, without the
    /// transaction-local tenant-context probe: every table has row-level
    /// security enabled and forced and the tenant policy with the expected
    /// expression, and the current role cannot bypass it.
    pub fn tables_isolated(&self) -> bool {
        self.expected_table_count > 0
            && self.missing_policy_tables.is_empty()
            && self.rls_disabled_tables.is_empty()
            && self.policy_expression_mismatch_tables.is_empty()
            && self.policy_installed_count == self.expected_table_count
            && self.rls_enabled_count == self.expected_table_count
            && self.force_rls_ready()
            && !self.current_role_bypasses_rls
            && !self.current_role_owns_trace_tables
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

/// Session-level Postgres advisory lock guarding the NEAR settlement SUBMIT
/// pass for a single tenant. The lock is held on a dedicated pooled connection
/// for the full duration of a submit run — across the external submitter call
/// and the `pending -> submitted/failed` status writes — so two overlapping
/// submit runs (e.g. a scheduler tick racing a manual worker POST) can never
/// both invoke the external submitter for the same outbox row (money-path
/// double-submit). The lock is independent of RLS / tenant transactions.
///
/// MUST be released by calling [`NearCreditSubmitAdvisoryLock::release`] when the
/// pass completes; the connection is then returned to the pool. `Drop` is a
/// best-effort safety net only (it cannot run async unlock), so callers are
/// responsible for the explicit release on every path.
pub struct NearCreditSubmitAdvisoryLock {
    inner: Option<crate::db::postgres::NearCreditSubmitAdvisoryLockInner>,
}

impl NearCreditSubmitAdvisoryLock {
    pub(crate) fn new(inner: crate::db::postgres::NearCreditSubmitAdvisoryLockInner) -> Self {
        Self { inner: Some(inner) }
    }

    /// Release the advisory lock and return the connection to the pool. Idempotent.
    pub async fn release(mut self) -> Result<(), DatabaseError> {
        if let Some(inner) = self.inner.take() {
            inner.release().await
        } else {
            Ok(())
        }
    }

    /// The pooled connection that holds the lock, for a pass that does its
    /// own database work on it rather than take a second pooled connection
    /// while it holds this one (the versioned pipeline's NEAR payout, which
    /// shares this lock with the submit worker). `None` only after release.
    pub(crate) fn client_mut(&mut self) -> Option<&mut deadpool_postgres::Client> {
        self.inner.as_mut().map(|inner| inner.client_mut())
    }
}

impl Drop for NearCreditSubmitAdvisoryLock {
    fn drop(&mut self) {
        if self.inner.is_some() {
            // Safety net: the lock guard was dropped without an explicit
            // `release().await`. The session-level lock will linger on the
            // recycled connection until the backend session ends or the
            // connection is reset. Log hash-free so operators can spot the leak.
            tracing::warn!(
                "NEAR credit submit advisory lock dropped without explicit release; \
                 connection-scoped lock may linger until session reset"
            );
        }
    }
}

/// Session-level Postgres advisory lock guarding a live credit-settlement
/// finalize for a single tenant. Prevents two overlapping settlement runs from
/// both passing the source-event conflict check and writing divergent batches
/// (and partial outbox rows) for the same credits. Independent of the submit
/// lock classid / namespace.
///
/// MUST be released via [`CreditSettlementAdvisoryLock::release`].
pub struct CreditSettlementAdvisoryLock {
    inner: Option<crate::db::postgres::CreditSettlementAdvisoryLockInner>,
}

impl CreditSettlementAdvisoryLock {
    pub(crate) fn new(inner: crate::db::postgres::CreditSettlementAdvisoryLockInner) -> Self {
        Self { inner: Some(inner) }
    }

    /// Release the advisory lock and return the connection to the pool. Idempotent.
    pub async fn release(mut self) -> Result<(), DatabaseError> {
        if let Some(inner) = self.inner.take() {
            inner.release().await
        } else {
            Ok(())
        }
    }
}

impl Drop for CreditSettlementAdvisoryLock {
    fn drop(&mut self) {
        if self.inner.is_some() {
            tracing::warn!(
                "credit settlement advisory lock dropped without explicit release; \
                 connection-scoped lock may linger until session reset"
            );
        }
    }
}

#[async_trait]
pub trait Database: TraceCorpusStore + Send + Sync {
    async fn select_inference_connection(
        &self,
        _tenant: &str,
        _account: uuid::Uuid,
        _request: &trace_commons_protocol::inference_connection::SelectInferenceConnection,
        _catalog: &crate::inference_connection::OperatorInferenceConnection,
    ) -> Result<postgres_inference_connection::InferenceSelectionOutcome, DatabaseError> {
        Err(DatabaseError::Pool(
            "inference_connection_unavailable".into(),
        ))
    }

    async fn current_inference_connection(
        &self,
        _tenant: &str,
        _account: uuid::Uuid,
        _catalog: &[crate::inference_connection::OperatorInferenceConnection],
    ) -> Result<Option<postgres_inference_connection::InferenceConnectionStatus>, DatabaseError>
    {
        Err(DatabaseError::Pool(
            "inference_connection_unavailable".into(),
        ))
    }

    async fn disconnect_inference_connection(
        &self,
        _tenant: &str,
        _account: uuid::Uuid,
        _connection_id: uuid::Uuid,
    ) -> Result<postgres_inference_connection::InferenceDisconnectOutcome, DatabaseError> {
        Err(DatabaseError::Pool(
            "inference_connection_unavailable".into(),
        ))
    }

    async fn redeem_account_invite(
        &self,
        _tenant: &str,
        _account: uuid::Uuid,
        _invite_hash: &str,
        _idempotency_key: uuid::Uuid,
    ) -> Result<AccountInviteRedemption, DatabaseError> {
        Err(DatabaseError::Pool("account_invite_unavailable".into()))
    }
    /// Store a legacy-link challenge for the authenticated account. `false`
    /// when the account already holds the maximum of open challenges.
    async fn store_legacy_invite_link_challenge(
        &self,
        _challenge: &crate::legacy_invite_link::ChallengeWrite,
    ) -> Result<bool, DatabaseError> {
        Err(DatabaseError::Pool("legacy_invite_link_unavailable".into()))
    }
    /// Record a verified, countersigned legacy invite link (V81).
    async fn link_legacy_invite(
        &self,
        _attempt: &crate::legacy_invite_link::LinkDbAttempt,
    ) -> Result<crate::legacy_invite_link::LinkDbOutcome, DatabaseError> {
        Err(DatabaseError::Pool("legacy_invite_link_unavailable".into()))
    }
    async fn get_reward_offer(
        &self,
        _program: uuid::Uuid,
    ) -> Result<crate::reward_participant::RewardOffer, crate::mission_rewards::RewardError> {
        Err(crate::mission_rewards::RewardError::StoreUnavailable)
    }

    async fn get_mission_publication(
        &self,
        _mission: uuid::Uuid,
    ) -> Result<
        trace_commons_protocol::mission_catalog::MissionPublication,
        crate::mission_rewards::RewardError,
    > {
        Err(crate::mission_rewards::RewardError::StoreUnavailable)
    }

    async fn list_mission_catalog(
        &self,
        _query: &trace_commons_protocol::mission_catalog::MissionCatalogQuery,
    ) -> Result<
        trace_commons_protocol::mission_catalog::MissionCatalogPage,
        crate::mission_rewards::RewardError,
    > {
        Err(crate::mission_rewards::RewardError::StoreUnavailable)
    }

    async fn reserve_reward_offer(
        &self,
        _tenant: &str,
        _account: uuid::Uuid,
        _program: uuid::Uuid,
        _request: &crate::reward_participant::RewardReservationRequest,
    ) -> Result<crate::reward_participant::RewardReservation, crate::mission_rewards::RewardError>
    {
        Err(crate::mission_rewards::RewardError::StoreUnavailable)
    }

    async fn get_reward_reservation(
        &self,
        _tenant: &str,
        _account: uuid::Uuid,
        _reservation: uuid::Uuid,
    ) -> Result<crate::reward_participant::RewardReservation, crate::mission_rewards::RewardError>
    {
        Err(crate::mission_rewards::RewardError::StoreUnavailable)
    }

    async fn get_reward_history(
        &self,
        _tenant: &str,
        _account: uuid::Uuid,
        _query: &crate::reward_participant::RewardHistoryQuery,
    ) -> Result<
        crate::reward_participant::RewardParticipantHistory,
        crate::mission_rewards::RewardError,
    > {
        Err(crate::mission_rewards::RewardError::StoreUnavailable)
    }

    async fn get_near_provisioned_anchor(
        &self,
        _tenant_id: &str,
        _principal_ref: &str,
    ) -> Result<Option<String>, DatabaseError> {
        Err(DatabaseError::Pool("near_provisioning_unconfigured".into()))
    }
    async fn get_near_provisioned_account(
        &self,
        _tenant_id: &str,
        _principal_ref: &str,
    ) -> Result<Option<uuid::Uuid>, DatabaseError> {
        Err(DatabaseError::Pool("near_provisioning_unconfigured".into()))
    }
    async fn external_account_trust_runtime_ready(&self) -> Result<bool, DatabaseError> {
        Ok(false)
    }
    /// Advisory client caps use a narrower worker/status role than admission.
    /// No direct table or column writes, admission mutations, or ledger role.
    async fn client_upload_trust_runtime_ready(&self) -> Result<bool, DatabaseError> {
        Ok(false)
    }
    /// The extra grants a `tiered-v1` admission policy's in-process
    /// evaluator needs (V126). See `db/postgres_account_trust_growth.rs`.
    async fn tiered_account_trust_runtime_ready(&self) -> Result<bool, DatabaseError> {
        Ok(false)
    }
    async fn enable_external_account_trust_growth(&self) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool(
            "external_account_trust_unavailable".into(),
        ))
    }
    async fn prune_account_trust_dependency_locks(
        &self,
        _limit: i32,
        _dry_run: bool,
    ) -> Result<u64, DatabaseError> {
        Err(DatabaseError::Pool(
            "account_trust_retention_unavailable".into(),
        ))
    }
    async fn account_admission_runtime_ready(&self) -> Result<bool, DatabaseError> {
        Ok(false)
    }
    async fn admission_runtime_ready(&self) -> Result<bool, DatabaseError> {
        Err(DatabaseError::Pool("admission_database_unavailable".into()))
    }
    async fn lookup_completed_submission_admission(
        &self,
        _tenant: &str,
        _anchor: &str,
        _submission: uuid::Uuid,
        _body_hash: &str,
    ) -> Result<bool, DatabaseError> {
        Err(DatabaseError::Pool("admission_database_unavailable".into()))
    }
    async fn legacy_admission_record(
        &self,
        _tenant: &str,
        _submission: uuid::Uuid,
    ) -> Result<Option<crate::admission_ledger::LegacyAdmissionRecord>, DatabaseError> {
        Err(DatabaseError::Pool("admission_database_unavailable".into()))
    }
    async fn resume_legacy_admission(
        &self,
        _tenant: &str,
        _anchor: &str,
        _submission: uuid::Uuid,
        _body_hash: &str,
        _lease: uuid::Uuid,
        _lease_seconds: i64,
    ) -> Result<crate::admission_ledger::AdmissionDecision, DatabaseError> {
        Err(DatabaseError::Pool("admission_database_unavailable".into()))
    }
    async fn acquire_admission_processing_lock(
        &self,
        _tenant: &str,
        _submission: uuid::Uuid,
    ) -> Result<Option<crate::admission_ledger::AdmissionProcessingGuard>, DatabaseError> {
        Err(DatabaseError::Pool("admission_database_unavailable".into()))
    }
    async fn prune_onboarding_expiry(
        &self,
        _tenant: &str,
        _limit: i32,
        _dry_run: bool,
    ) -> Result<u64, DatabaseError> {
        Err(DatabaseError::Pool(
            "onboarding_retention_unavailable".into(),
        ))
    }
    async fn issue_admission_challenge(
        &self,
        _tenant: &str,
        _anchor: &str,
        _challenge: &str,
        _expires: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool("admission_database_unavailable".into()))
    }
    async fn reserve_submission_admission(
        &self,
        _request: &crate::admission_ledger::AdmissionReservation,
    ) -> Result<crate::admission_ledger::AdmissionDecision, DatabaseError> {
        Err(DatabaseError::Pool("admission_database_unavailable".into()))
    }
    async fn account_admission_record(
        &self,
        _tenant: &str,
        _submission: uuid::Uuid,
    ) -> Result<Option<crate::admission_ledger::AccountAdmissionRecord>, DatabaseError> {
        Err(DatabaseError::Pool(
            "account_admission_database_unavailable".into(),
        ))
    }
    async fn reserve_account_admission(
        &self,
        _request: &crate::admission_ledger::AccountAdmissionReservation,
    ) -> Result<crate::admission_ledger::AccountAdmissionResult, DatabaseError> {
        Err(DatabaseError::Pool(
            "account_admission_database_unavailable".into(),
        ))
    }
    async fn record_account_trust_fact(
        &self,
        _account: &crate::account_trust::TrustAccount,
        _source: crate::account_trust::TrustFactSource,
    ) -> Result<Option<crate::account_trust::TrustFactOutcome>, DatabaseError> {
        Err(DatabaseError::Pool("account_trust_fact_unavailable".into()))
    }
    /// Earned-trust worker enumeration: open accounts in anchored tenants,
    /// keyed and paged. See `db/postgres_account_trust_growth.rs`.
    async fn list_account_trust_worker_accounts(
        &self,
        _after: Option<&crate::account_trust::TrustAccount>,
        _limit: i64,
    ) -> Result<Vec<crate::account_trust::TrustAccount>, DatabaseError> {
        Err(DatabaseError::Pool(
            "account_trust_worker_unavailable".into(),
        ))
    }
    /// Records one accepted submission's starter-advance plausibility
    /// features (V131) once. Returns whether a row was written. See
    /// `crate::starter_plausibility`.
    async fn record_starter_plausibility_features(
        &self,
        _tenant_id: &str,
        _submission_id: uuid::Uuid,
        _features: &crate::starter_plausibility::StarterEnvelopeFeatures,
    ) -> Result<bool, DatabaseError> {
        Err(DatabaseError::Pool("starter_features_unavailable".into()))
    }
    /// The starter-advance features of one account's own accepted,
    /// unwithdrawn submissions, oldest first. Tenant-scoped.
    async fn starter_plausibility_features_for_account(
        &self,
        _account: &crate::account_trust::TrustAccount,
    ) -> Result<Vec<crate::starter_plausibility::StarterEnvelopeFeatures>, DatabaseError> {
        Err(DatabaseError::Pool("starter_features_unavailable".into()))
    }
    /// Sources naming this account that have no trust fact yet.
    async fn list_account_trust_fact_candidates(
        &self,
        _account: &crate::account_trust::TrustAccount,
        _limit: i64,
    ) -> Result<Vec<crate::account_trust::TrustFactSource>, DatabaseError> {
        Err(DatabaseError::Pool(
            "account_trust_worker_unavailable".into(),
        ))
    }
    /// Every fact of one account with the gate fields the rule reads.
    async fn account_trust_evaluation_inputs(
        &self,
        _account: &crate::account_trust::TrustAccount,
    ) -> Result<Vec<crate::account_trust_rule::EvaluationFact>, DatabaseError> {
        Err(DatabaseError::Pool(
            "account_trust_worker_unavailable".into(),
        ))
    }
    /// Evaluates one account under the admission policy's growth rule and
    /// stores an `applied` row bound to its input generation, in one
    /// snapshot. `None` when the account has no facts.
    async fn evaluate_and_record_applied_account_trust(
        &self,
        _account: &crate::account_trust::TrustAccount,
        _policy: &crate::account_trust_rule::GrowthPolicy,
    ) -> Result<Option<(crate::account_trust_rule::Evaluation, bool)>, DatabaseError> {
        Err(DatabaseError::Pool(
            "account_trust_worker_unavailable".into(),
        ))
    }
    /// Stores one shadow evaluation; returns whether the tier changed.
    async fn record_account_trust_evaluation(
        &self,
        _account: &crate::account_trust::TrustAccount,
        _mode: &str,
        _evaluation: &crate::account_trust_rule::Evaluation,
    ) -> Result<bool, DatabaseError> {
        Err(DatabaseError::Pool(
            "account_trust_worker_unavailable".into(),
        ))
    }
    /// The newest stored evaluation under one policy version and mode.
    async fn latest_account_trust_evaluation(
        &self,
        _account: &crate::account_trust::TrustAccount,
        _policy_version: &str,
        _mode: &str,
    ) -> Result<Option<crate::account_trust_rule::Evaluation>, DatabaseError> {
        Err(DatabaseError::Pool(
            "account_trust_worker_unavailable".into(),
        ))
    }
    async fn account_admission_status(
        &self,
        _account: &crate::account_trust::TrustAccount,
        _principal: &str,
        _policy: &crate::account_trust::BoundedPolicy,
    ) -> Result<Option<crate::admission_ledger::AccountAdmissionStatus>, DatabaseError> {
        Err(DatabaseError::Pool(
            "account_admission_database_unavailable".into(),
        ))
    }
    async fn transition_account_admission(
        &self,
        _tenant: &str,
        _principal: &str,
        _account: uuid::Uuid,
        _submission: uuid::Uuid,
        _lease: uuid::Uuid,
        _next: &str,
    ) -> Result<bool, DatabaseError> {
        Err(DatabaseError::Pool(
            "account_admission_database_unavailable".into(),
        ))
    }
    async fn transition_submission_admission(
        &self,
        _tenant: &str,
        _submission: uuid::Uuid,
        _lease: uuid::Uuid,
        _next: &str,
    ) -> Result<bool, DatabaseError> {
        Err(DatabaseError::Pool("admission_database_unavailable".into()))
    }

    /// Bounded, tenant-ID-only discovery of potentially unsettled utility credit.
    /// Eligibility and holds are rechecked inside each tenant's settlement pass.
    async fn pending_credit_settlement_tenants(
        &self,
        _after: Option<&str>,
        _limit: usize,
        _novelty_settle_from: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<String>, DatabaseError> {
        Err(DatabaseError::Pool(
            "credit_settlement_enumerator_unavailable".into(),
        ))
    }

    async fn run_migrations(&self) -> Result<(), DatabaseError>;

    /// Try to acquire the per-tenant NEAR settlement submit advisory lock without
    /// blocking. Returns `Ok(Some(guard))` if acquired (caller may proceed and
    /// MUST `release()` it), or `Ok(None)` if another submit run already holds it
    /// (caller must no-op). Default impl returns `Ok(None)` (no serialization
    /// available), which is fail-closed for the submit worker.
    async fn try_acquire_near_credit_submit_lock(
        &self,
        _tenant_id: &str,
    ) -> Result<Option<NearCreditSubmitAdvisoryLock>, DatabaseError> {
        Ok(None)
    }

    /// Try to acquire the per-tenant live credit-settlement advisory lock without
    /// blocking. Returns `Ok(Some(guard))` if acquired (caller MUST `release()`),
    /// or `Ok(None)` if another settlement run already holds it. Default returns
    /// `Ok(None)` so callers without Postgres serialization refuse to race.
    async fn try_acquire_credit_settlement_lock(
        &self,
        _tenant_id: &str,
    ) -> Result<Option<CreditSettlementAdvisoryLock>, DatabaseError> {
        Ok(None)
    }

    /// Atomically persist a finalized settlement batch together with every NEAR
    /// outbox row it expects. Default falls back to sequential upserts (not
    /// crash-atomic across statements); Postgres overrides this with one
    /// tenant-scoped transaction so a process death cannot leave a finalized
    /// batch without its payout work.
    async fn upsert_credit_settlement_finalize(
        &self,
        batch: TraceCreditSettlementBatchWrite,
        outbox_items: Vec<TraceNearCreditOutboxItemWrite>,
    ) -> Result<(), DatabaseError> {
        self.upsert_trace_credit_settlement_batch(batch).await?;
        for item in outbox_items {
            self.upsert_trace_near_credit_outbox_item(item).await?;
        }
        Ok(())
    }

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

    /// Soft-delete by stamping `withdrawn_at = NOW()`, enqueue a coalesced
    /// community-snapshot invalidation for `(window_label, metric)`, and
    /// record an eviction receipt — all in one transaction.
    ///
    /// Idempotent: returns `Ok(None)` when no active profile row exists for
    /// `(tenant_id, principal_ref)`. A successful withdrawal returns the
    /// eviction receipt so callers can audit that the published surface was
    /// asked to drop the contributor.
    async fn withdraw_contributor_profile(
        &self,
        _tenant_id: &str,
        _principal_ref: &str,
        _window_label: &str,
        _metric: &str,
    ) -> Result<Option<CommunityWithdrawalEvictionRow>, DatabaseError> {
        Err(DatabaseError::Pool(
            "withdraw_contributor_profile not implemented".to_string(),
        ))
    }

    /// Latest undrained community-snapshot invalidation watermark, if any.
    /// Public reads refuse snapshots whose `computed_at` is strictly before
    /// this instant.
    async fn pending_community_snapshot_invalidation(
        &self,
        _window_label: &str,
        _metric: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, DatabaseError> {
        Err(DatabaseError::Pool(
            "pending_community_snapshot_invalidation not implemented".to_string(),
        ))
    }

    /// Clear a pending invalidation after a snapshot whose `computed_at` is
    /// at or after the pending watermark has been written. Returns `true`
    /// when a pending row was drained. Concurrent withdrawals that land
    /// after `snapshot_computed_at` leave the pending flag set.
    async fn drain_community_snapshot_invalidation(
        &self,
        _window_label: &str,
        _metric: &str,
        _snapshot_id: uuid::Uuid,
        _snapshot_computed_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, DatabaseError> {
        Err(DatabaseError::Pool(
            "drain_community_snapshot_invalidation not implemented".to_string(),
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

    /// Create or replace the one public page attached to an owned submission.
    /// Implementations must keep the mutation and its account-audit row in one
    /// tenant-scoped transaction.
    async fn upsert_public_run(
        &self,
        _write: PublicRunWrite,
    ) -> Result<PublicRunMutation, DatabaseError> {
        Err(DatabaseError::Pool(
            "upsert_public_run not implemented".to_string(),
        ))
    }

    /// Read the owner row, active public projection, and retained source slug
    /// from one locked database snapshot.
    async fn get_owned_public_run_state(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _submission_id: uuid::Uuid,
    ) -> Result<PublicRunOwnerData, DatabaseError> {
        Err(DatabaseError::Pool(
            "get_owned_public_run_state not implemented".to_string(),
        ))
    }

    /// Withdraw the page owned by this account and submission. Idempotent.
    async fn unpublish_public_run(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _submission_id: uuid::Uuid,
    ) -> Result<PublicRunUnpublishMutation, DatabaseError> {
        Err(DatabaseError::Pool(
            "unpublish_public_run not implemented".to_string(),
        ))
    }

    /// Public, cross-tenant read through V64's SECURITY DEFINER projection.
    /// The database function returns public fields only and its owner cannot be
    /// assumed by the runtime role.
    async fn get_public_run_page_by_slug(
        &self,
        _slug: &str,
        _variation_limit: i64,
    ) -> Result<Option<PublicRunPageData>, DatabaseError> {
        Err(DatabaseError::Pool(
            "get_public_run_page_by_slug not implemented".to_string(),
        ))
    }

    /// Resolve one active source page through V64's bounded internal function.
    async fn resolve_public_run_source(
        &self,
        _slug: &str,
    ) -> Result<Option<PublicRunSourceRow>, DatabaseError> {
        Err(DatabaseError::Pool(
            "resolve_public_run_source not implemented".to_string(),
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

    /// All-time totals for the public register-stats aggregate. Crosses
    /// tenants the same way `compute_leaderboard_inputs` and
    /// `compute_corpus_analytics_summary` do: per-tenant transactions under
    /// the RLS tenant GUC, never a raw cross-tenant query and never
    /// `BYPASSRLS`.
    ///
    /// `configured_tenant_ids` is the same escape hatch
    /// `compute_leaderboard_inputs` / `compute_corpus_analytics_summary`
    /// take: when non-empty, enumerate exactly those tenants; when empty,
    /// fall back to `SELECT tenant_id FROM trace_tenants`. That fallback
    /// query carries no tenant GUC, and `trace_tenants` is itself
    /// FORCE-RLS'd on `tenant_id = trace_current_tenant_id()` -- so under a
    /// NOBYPASSRLS runtime role with no tenant context, it silently returns
    /// zero rows rather than erroring. Implementations MUST refuse
    /// (`Err`) rather than compute totals when the resolved tenant list is
    /// empty: a zero here is indistinguishable from "the register really
    /// has nothing" and from "RLS ate the enumeration," and only the first
    /// is safe to publish. (This is why a superuser-connected pg test
    /// cannot exercise the failure mode this guards against -- the owning
    /// test principal bypasses the RLS that would otherwise produce it.)
    async fn compute_register_stats_totals(
        &self,
        _configured_tenant_ids: &[String],
    ) -> Result<RegisterStatsTotals, DatabaseError> {
        Err(DatabaseError::Pool(
            "compute_register_stats_totals not implemented".to_string(),
        ))
    }

    /// Read the `trace_register_stats` singleton row as it stands.
    async fn fetch_register_stats_row(&self) -> Result<RegisterStatsRow, DatabaseError> {
        Err(DatabaseError::Pool(
            "fetch_register_stats_row not implemented".to_string(),
        ))
    }

    /// Read the same singleton row, but under the dedicated
    /// `trace_commons_public_read` role, for the unauthenticated public
    /// endpoint.
    ///
    /// Separate from [`Database::fetch_register_stats_row`] on purpose. That
    /// one runs as the ordinary runtime role, which reaches this row through
    /// the unscoped `trace_register_stats_runtime_read` policy and could
    /// reach every other table besides. The unauthenticated path takes the
    /// least-privileged route V55 built for it: a NOBYPASSRLS role holding
    /// one column-scoped SELECT grant and nothing else, so a mistake in this
    /// handler cannot become a read of anything but six aggregate columns.
    async fn fetch_register_stats_row_as_public_read(
        &self,
    ) -> Result<RegisterStatsRow, DatabaseError> {
        Err(DatabaseError::Pool(
            "fetch_register_stats_row_as_public_read not implemented".to_string(),
        ))
    }

    /// Write freshly computed totals into the `trace_register_stats`
    /// singleton row, stamping both `as_of` and `refreshed_at` to now, and
    /// clearing `withheld`. Returns the row as written.
    async fn write_register_stats_row(
        &self,
        _totals: RegisterStatsTotals,
    ) -> Result<RegisterStatsRow, DatabaseError> {
        Err(DatabaseError::Pool(
            "write_register_stats_row not implemented".to_string(),
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

    /// Drop all but the `keep` most recent snapshots for a window/metric,
    /// returning how many rows were removed.
    ///
    /// Only the newest snapshot is ever served, but older ones are the
    /// record of what was published and under which privacy controls, so
    /// this trims rather than truncates. It exists because recompute went
    /// from an occasional manual action to a scheduled one: at a 60s
    /// interval an unpruned table gains on the order of half a million
    /// rows a year, each carrying a full JSON payload.
    async fn prune_leaderboard_snapshots(
        &self,
        _window_label: &str,
        _metric: &str,
        _keep: i64,
    ) -> Result<u64, DatabaseError> {
        Err(DatabaseError::Pool(
            "prune_leaderboard_snapshots not implemented".to_string(),
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
    async fn enroll_instance_user(&self, p: InstanceUserProvision) -> Result<(), DatabaseError>;

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

    /// Verify the V35 instance-enrollment ledger has forced row-level security
    /// and its `trace_instance_isolation` policy installed. The ledger isolates
    /// on the INSTANCE predicate (`trace_current_instance_subject()`), not the
    /// tenant predicate, so it is intentionally absent from the tenant RLS
    /// diagnostics; this check lets the production-readiness gate catch RLS drift
    /// on the ledger. Returns `false` (fail-closed) if either control is missing.
    async fn instance_ledger_rls_ready(&self) -> Result<bool, DatabaseError>;

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

    /// Batched principal->account resolution. Returns a map from each
    /// `principal_ref` that has an ACTIVE link (`unlinked_at IS NULL`) within
    /// `tenant_id` to its `account_id`. Principals with no active link (never
    /// linked, unlinked, or absent) are omitted from the map. An empty input
    /// slice yields an empty map. Used by the credit-view and settlement re-key
    /// paths to fold per-principal rows up to their durable account.
    async fn resolve_principals_to_accounts(
        &self,
        _tenant_id: &str,
        _principal_refs: &[String],
    ) -> Result<std::collections::HashMap<String, uuid::Uuid>, DatabaseError> {
        Err(DatabaseError::Pool(
            "resolve_principals_to_accounts not implemented".to_string(),
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
    async fn store_near_provisioning_ceremony(
        &self,
        _ceremony_hash: &str,
        _pending: crate::account_onboarding::NativeProvisioningPending,
        _expires_at: i64,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool("near_provisioning_unconfigured".into()))
    }

    async fn take_near_provisioning_ceremony(
        &self,
        _ceremony_hash: &str,
    ) -> Result<Option<crate::account_onboarding::NativeProvisioningPending>, DatabaseError> {
        Err(DatabaseError::Pool("near_provisioning_unconfigured".into()))
    }

    /// `identity` is not optional and has no default: a backend that cannot be
    /// handed a pepper and an account-name key must not provision, because the
    /// alternative is an anchor computed from public inputs alone -- the exact
    /// defect the salted scheme exists to remove.
    async fn provision_verified_near_account(
        &self,
        _proof: crate::account_onboarding::VerifiedNearProvisioning,
        _session: NewSession<'_>,
        _identity: &crate::near_account_identity::NearAccountIdentity,
    ) -> Result<crate::account_onboarding::ProvisionedNearAccount, DatabaseError> {
        Err(DatabaseError::Pool("near_provisioning_unconfigured".into()))
    }

    /// Store a NEAR AI login ceremony (#836). Fail-closed by default, like
    /// its wallet sibling.
    async fn store_near_ai_login_ceremony(
        &self,
        _ceremony_hash: &str,
        _pending: &crate::account_onboarding::NearAiLoginPending,
        _expires_at: i64,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool(
            "near_ai_login_provisioning_unconfigured".into(),
        ))
    }

    /// Consume a NEAR AI login ceremony. Single use: a second take of the same
    /// handle answers `None`.
    async fn take_near_ai_login_ceremony(
        &self,
        _ceremony_hash: &str,
    ) -> Result<Option<crate::account_onboarding::NearAiLoginPending>, DatabaseError> {
        Err(DatabaseError::Pool(
            "near_ai_login_provisioning_unconfigured".into(),
        ))
    }

    /// Provision from a verified NEAR AI login (#836).
    ///
    /// With `grant_template`, the device also gets the contributor grant
    /// invite enrollment writes, from that template, insert-only (owner
    /// decision 2026-10-24). Bind and enrollment below take the same
    /// argument with the same meaning. `None` writes no grant.
    ///
    /// The default refuses, like its wallet sibling: a backend that has not
    /// implemented this has not configured login provisioning, and the
    /// fail-closed answer is the only safe one for a path that mints an
    /// admission anchor.
    async fn provision_near_ai_login(
        &self,
        _login: &crate::near_ai_login::VerifiedNearAiLogin,
        _device_public_key: &[u8; 32],
        _session: NewSession<'_>,
        _identity: &crate::near_account_identity::NearAccountIdentity,
        _grant_template: Option<&crate::near_ai_signup_policy::NearAiSignupPolicyTemplate>,
    ) -> Result<crate::account_onboarding::ProvisionedNearAccount, DatabaseError> {
        Err(DatabaseError::Pool(
            "near_ai_login_provisioning_unconfigured".into(),
        ))
    }

    /// Store a NEAR AI bind ceremony (Z2 S3) in the provisioning ceremony
    /// table. Fail-closed by default.
    async fn store_near_ai_bind_ceremony(
        &self,
        _ceremony_hash: &str,
        _pending: &crate::account_onboarding::NearAiBindPending,
        _expires_at: i64,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool("near_ai_bind_unconfigured".into()))
    }

    /// Consume a NEAR AI bind ceremony. Single use; a row that is not a bind
    /// ceremony (a provisioning or wallet row) is consumed and refused.
    async fn take_near_ai_bind_ceremony(
        &self,
        _ceremony_hash: &str,
    ) -> Result<Option<crate::account_onboarding::NearAiBindPending>, DatabaseError> {
        Err(DatabaseError::Pool("near_ai_bind_unconfigured".into()))
    }

    /// Bind a verified NEAR AI login to the unbound passkey account
    /// `(tenant_id, account_id)` (Z2 S3), or, when the login's anchor already
    /// belongs to another account, provision that account exactly as
    /// [`Self::provision_near_ai_login`] would and close the passkey account.
    /// The default refuses.
    #[allow(clippy::too_many_arguments)]
    async fn bind_near_ai_login(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _login: &crate::near_ai_login::VerifiedNearAiLogin,
        _device_public_key: &[u8; 32],
        _session: NewSession<'_>,
        _identity: &crate::near_account_identity::NearAccountIdentity,
        _grant_template: Option<&crate::near_ai_signup_policy::NearAiSignupPolicyTemplate>,
    ) -> Result<crate::account_onboarding::NearAiBindOutcome, DatabaseError> {
        Err(DatabaseError::Pool("near_ai_bind_unconfigured".into()))
    }

    /// Enroll a further device into the `bound` account `(tenant_id,
    /// account_id)` from a verified NEAR AI login: a second Mac signed in with
    /// the account's passkey. Attaches the device only if the login's anchor
    /// is that account's own, decided in the same transaction that would
    /// write the device; otherwise refuses with
    /// [`crate::account_onboarding::NEAR_AI_ENROL_ACCOUNT_MISMATCH`] having
    /// written nothing. Never claims an anchor or mints a tenant or account.
    /// The default refuses.
    #[allow(clippy::too_many_arguments)]
    async fn enrol_near_ai_login(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _login: &crate::near_ai_login::VerifiedNearAiLogin,
        _device_public_key: &[u8; 32],
        _session: NewSession<'_>,
        _identity: &crate::near_account_identity::NearAccountIdentity,
        _grant_template: Option<&crate::near_ai_signup_policy::NearAiSignupPolicyTemplate>,
    ) -> Result<crate::account_onboarding::ProvisionedNearAccount, DatabaseError> {
        Err(DatabaseError::Pool("near_ai_bind_unconfigured".into()))
    }

    /// Write the NEAR AI sign-up contributor grant for a device NEAR AI
    /// provisioned before sign-up wrote one (owner decision 2026-10-24).
    ///
    /// The issuer calls this at claim time when a device has no active
    /// contributor grant. In one transaction in `tenant_id`, it checks that
    /// the device is a live `near_ai`-origin key there, and if so inserts the
    /// grant sign-up would have written from `template`, never replacing a
    /// grant row that exists (so a revoked grant stays revoked and a later
    /// template does not rewrite an earlier one). Returns whether the device
    /// was eligible; `false` writes nothing. A written grant is issued at
    /// `issued_at`, the instant the caller lists grants at. The default
    /// refuses.
    async fn backfill_near_ai_signup_device_grant(
        &self,
        _tenant_id: &str,
        _device_key_id: &str,
        _template: &crate::near_ai_signup_policy::NearAiSignupPolicyTemplate,
        _issued_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, DatabaseError> {
        Err(DatabaseError::Pool(
            "near_ai_signup_grant_backfill_unconfigured".into(),
        ))
    }

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

    /// Record the ownership alias of an authenticated device upload claim.
    /// Call only after verifying the device signature and claim scope. This
    /// binds provenance to the device, never grants an account authenticator.
    /// Registered legacy devices without server-side NEAR provisioning are a
    /// no-op: their signed principals retain the independent login model.
    async fn bind_device_upload_principal(
        &self,
        _tenant_id: &str,
        _device_key_id: &str,
        _actor_ref: &str,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool(
            "device_upload_principal_binding_unavailable".to_string(),
        ))
    }

    /// Explicit operator repair for the native client's deterministic subject.
    /// Returns the number of existing submissions whose exact provenance hash
    /// matches that device. Dry-run never writes; apply records only the alias.
    async fn repair_native_device_upload_principal(
        &self,
        _tenant_id: &str,
        _device_key_id: &str,
        _dry_run: bool,
    ) -> Result<u64, DatabaseError> {
        Err(DatabaseError::Pool(
            "device_upload_principal_repair_unavailable".to_string(),
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

    /// The label `account_id` gave its ACTIVE credential `credential_id`, for
    /// the native sign-in answer (`passkey_label`). `None` for no label, and
    /// for an unknown, revoked, other-account or other-tenant credential: the
    /// row must match the tenant, the account AND the credential, so a caller
    /// can only ever read the label of a credential its own account holds.
    /// Tenant-scoped under forced RLS.
    async fn credential_label_for_account(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _credential_id: &str,
    ) -> Result<Option<String>, DatabaseError> {
        Err(DatabaseError::Pool(
            "credential_label_for_account not implemented".to_string(),
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

    /// Issue a session for a native-app loopback sign-in whose one-time code and
    /// PKCE verifier have ALREADY been checked. Mirrors
    /// [`Database::issue_passkey_session`] except `client_kind = 'native'` and
    /// `auth_credential_id` is NULL: no passkey or wallet key authenticated this
    /// session, the human's browser login did.
    ///
    /// As with the passkey/NEAR paths there is NO `ensure_trace_tenant` here.
    /// The tenant came from a login link that a human already redeemed in a
    /// browser, so the account row provably exists; the insert is FK-bound to
    /// `(tenant_id, account_id)` and a bogus pair simply fails rather than
    /// writing anything. The raw session secret never reaches the database.
    async fn issue_native_session(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _session: NewSession<'_>,
        _audit: RedeemAudit,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool(
            "issue_native_session not implemented".to_string(),
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
    /// last passkey. When a credential is removed, every live session it
    /// minted (`auth_credential_id` = that credential, browser or native) is
    /// revoked in the same transaction. See
    /// [`Database::revoke_account_credential_sparing_session`], which the
    /// removal route calls to keep the caller's own session.
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

    /// [`Database::revoke_account_credential`], sparing one session: the one
    /// whose current or within-grace previous `token_hash` is
    /// `caller_token_hash` (the session making the removal request). `None`
    /// spares nothing. Sessions with no recorded credential are never touched.
    async fn revoke_account_credential_sparing_session(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _credential_id: &str,
        _caller_token_hash: Option<&str>,
    ) -> Result<RevokeCredentialResult, DatabaseError> {
        Err(DatabaseError::Pool(
            "revoke_account_credential_sparing_session not implemented".to_string(),
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

    /// List the ACTIVE devices linked to `account_id` through NEAR sign-in
    /// (`trace_near_provisioned_devices` joined to unrevoked `device_keys`),
    /// oldest first. `session_token_hash` is the hash of the session the caller
    /// presented, used only to flag `this_device`. Tenant- and account-scoped
    /// under forced RLS, so another account's devices are never returned.
    async fn list_account_devices(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _session_token_hash: Option<&str>,
    ) -> Result<Vec<AccountDeviceSummary>, DatabaseError> {
        Err(DatabaseError::Pool(
            "list_account_devices not implemented".to_string(),
        ))
    }

    /// Rename one of `account_id`'s ACTIVE devices. `None` clears the label.
    /// Returns whether a row was affected (`false` for an unknown, revoked or
    /// other-account device).
    async fn rename_account_device(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _device_key_id: &str,
        _label: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        Err(DatabaseError::Pool(
            "rename_account_device not implemented".to_string(),
        ))
    }

    /// Revoke one of `account_id`'s ACTIVE devices in one transaction: set
    /// `device_keys.revoked_at` (the upload-claim issuer then refuses the key),
    /// revoke the device principal's active tenant access grants, and revoke
    /// the native sessions minted for the device. The account's native sessions
    /// that name neither a device nor a passkey (issued before V139, or by the
    /// login-link native sign-in) cannot be attributed, so they are revoked
    /// too, except the caller's own (`caller_session_token_hash`). The device's principal
    /// stays linked, so traces it already contributed remain in the account's
    /// history. `actor_ref` is recorded as the grant revoker.
    async fn revoke_account_device(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _device_key_id: &str,
        _actor_ref: &str,
        _caller_session_token_hash: Option<&str>,
    ) -> Result<RevokeAccountDeviceResult, DatabaseError> {
        Err(DatabaseError::Pool(
            "revoke_account_device not implemented".to_string(),
        ))
    }

    /// The caller's traces that could take the `model_training` use (V141):
    /// owned by one of `owner_principal_refs`, `accepted`, not revoked,
    /// purged, expired or withdrawn (either withdrawal ledger), and without
    /// `model_training` in their allowed uses. The count is all of them; the
    /// ids are the oldest `limit`. No owners lists nothing.
    async fn list_model_training_consent_eligible(
        &self,
        _tenant_id: &str,
        _owner_principal_refs: &[String],
        _limit: i64,
    ) -> Result<ModelTrainingConsentEligible, DatabaseError> {
        Err(DatabaseError::Pool(
            MODEL_TRAINING_CONSENT_BACKEND_MISSING.to_string(),
        ))
    }

    /// Add the `model_training` consent scope and allowed use to each listed
    /// trace that is eligible (as above), in one transaction, and record one
    /// dated grant row per trace. Add-only and idempotent: a trace that
    /// already carries the use is answered `AlreadyGranted` and left as it
    /// is; one that is not eligible is answered `NotEligible`.
    async fn grant_model_training_consent(
        &self,
        _grant: ModelTrainingConsentGrant<'_>,
    ) -> Result<Vec<ModelTrainingConsentGrantRow>, DatabaseError> {
        Err(DatabaseError::Pool(
            MODEL_TRAINING_CONSENT_BACKEND_MISSING.to_string(),
        ))
    }

    /// The submission's latest gate decision when it is a legacy-gate
    /// decision that passed both floors and whose scoring award was withheld
    /// as `policy_mismatch` (or left at `credit_check_error` by a re-run whose
    /// checks failed): the award a model-training grant re-runs.
    /// `None` otherwise (never scored, a floor failed, withheld for another
    /// reason, credited, or a pipeline Settle row).
    async fn withheld_policy_mismatch_gate_award(
        &self,
        _tenant_id: &str,
        _submission_id: uuid::Uuid,
    ) -> Result<Option<WithheldGateAward>, DatabaseError> {
        Err(DatabaseError::Pool(
            MODEL_TRAINING_CONSENT_BACKEND_MISSING.to_string(),
        ))
    }

    /// Mint a one-time device code for `code.account_id`, after checking
    /// that `code.minter_device_key_id` is an active device of that account.
    async fn mint_account_device_code(
        &self,
        _code: NewAccountDeviceCode<'_>,
    ) -> Result<MintAccountDeviceCodeOutcome, DatabaseError> {
        Err(DatabaseError::Pool(
            "mint_account_device_code not implemented".to_string(),
        ))
    }

    /// One of `account_id`'s minted codes, or `None` for an unknown or
    /// foreign code.
    async fn account_device_code_status(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _code_id: uuid::Uuid,
    ) -> Result<Option<AccountDeviceCodeStatus>, DatabaseError> {
        Err(DatabaseError::Pool(
            "account_device_code_status not implemented".to_string(),
        ))
    }

    /// Spend a device code and link `device_public_key` to the code's
    /// account in one transaction, as a NEAR AI sign-in links a device: key,
    /// principal, grant from `grant_template`, provisioned-device row, and
    /// a native session. The code's label becomes the device's label.
    async fn redeem_account_device_code(
        &self,
        _tenant_id: &str,
        _code_hash: &str,
        _device_public_key: &[u8; 32],
        _session: NewSession<'_>,
        _grant_template: Option<&crate::near_ai_signup_policy::NearAiSignupPolicyTemplate>,
    ) -> Result<DeviceSignInOutcome, DatabaseError> {
        Err(DatabaseError::Pool(
            "redeem_account_device_code not implemented".to_string(),
        ))
    }

    /// Mint a fresh native session for a device that joined by code and is
    /// still active, recording `nonce_hash` so the proof cannot be replayed.
    /// Refuses a device that renewed within `min_interval`.
    async fn renew_device_session(
        &self,
        _tenant_id: &str,
        _device_public_key: &[u8; 32],
        _nonce_hash: &str,
        _session: NewSession<'_>,
        _min_interval: chrono::Duration,
    ) -> Result<DeviceSignInOutcome, DatabaseError> {
        Err(DatabaseError::Pool(
            "renew_device_session not implemented".to_string(),
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

    /// Count passkey-origin accounts that hold a slot under the unbound-account
    /// ceiling, in EVERY tenant (Z2 S2): `unbound` ones, and since V102
    /// `closed` ones the reaper has not yet removed, so create-then-close
    /// cycles cannot escape the ceiling. A cross-tenant read, so the
    /// PostgreSQL backend answers through the V98 SECURITY DEFINER function,
    /// never through a runtime-pool query. The default refuses: a backend that
    /// cannot count cannot enforce the ceiling, so creation stays closed.
    async fn count_unbound_passkey_accounts(&self) -> Result<i64, DatabaseError> {
        Err(DatabaseError::Pool(
            "unbound_passkey_account_count_unconfigured".to_string(),
        ))
    }

    /// Create a passkey-origin account from a verified native `create/finish`
    /// (Z2 S2): in ONE transaction under the freshly minted tenant, re-check
    /// the unbound-account ceiling, then write the tenant, the account and its
    /// `unbound` binding row, the credential, a native session and the audit
    /// row. Either every row is written or none is. The default refuses.
    async fn create_passkey_origin_account(
        &self,
        _account: NewPasskeyOriginAccount<'_>,
    ) -> Result<PasskeyOriginAccountOutcome, DatabaseError> {
        Err(DatabaseError::Pool(
            "create_passkey_origin_account not implemented".to_string(),
        ))
    }

    /// The binding state of one account, read under its tenant's RLS. No
    /// binding row is `Legacy`. Used by native passkey sign-in to report
    /// `binding_state`; the unbound gate reads it inside `validate_session`.
    async fn account_binding_state(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
    ) -> Result<crate::account_binding::AccountBindingState, DatabaseError> {
        Err(DatabaseError::Pool(
            "account_binding_state not implemented".to_string(),
        ))
    }

    /// Designate one of `account_id`'s ACTIVE NEAR identities as its payout target.
    /// In ONE tx: clear any existing active designation for the account (so at most
    /// one holds, never violating the partial-unique index), then stamp
    /// `payout_designated_at = now()` on the named active key. Returns whether the
    /// stamp affected a row (`false` for an unknown / revoked / other-account key,
    /// which the caller surfaces as a 404). Tenant- + account-scoped under forced RLS.
    async fn designate_payout_near_identity(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _public_key: &str,
    ) -> Result<bool, DatabaseError> {
        Err(DatabaseError::Pool(
            "designate_payout_near_identity not implemented".to_string(),
        ))
    }

    /// Clear the payout designation from one of `account_id`'s ACTIVE NEAR
    /// identities. Returns whether a row changed (`false` when the key was not an
    /// active, currently-designated identity). Tenant- + account-scoped under forced
    /// RLS so a caller can never clear an identity they do not own.
    async fn clear_payout_near_identity(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
        _public_key: &str,
    ) -> Result<bool, DatabaseError> {
        Err(DatabaseError::Pool(
            "clear_payout_near_identity not implemented".to_string(),
        ))
    }

    /// Resolve the NEAR account id to pay for `account_id`, fail-closed. Reading the
    /// account's ACTIVE identities in one RLS-scoped tx: a designated one wins
    /// (`Designated`); else a single active identity is the unambiguous target
    /// (`SoleActive`); else zero active holds `NoneEnrolled` and two-or-more active
    /// with none designated holds `AmbiguousNoDesignation`. Settlement must withhold
    /// on any `Hold` rather than guess. Tenant- + account-scoped under forced RLS.
    async fn resolve_payout_near_account_id(
        &self,
        _tenant_id: &str,
        _account_id: uuid::Uuid,
    ) -> Result<PayoutResolution, DatabaseError> {
        Err(DatabaseError::Pool(
            "resolve_payout_near_account_id not implemented".to_string(),
        ))
    }

    /// Stage a device-principal account merge by consuming device B's login-link
    /// as proof-of-control, in ONE RLS-scoped transaction. The conditional consume
    /// UPDATE on `trace_login_links` is ALWAYS executed (never a SELECT-then-branch):
    /// an unknown / expired / already-consumed code affects zero rows -> `Ok(None)`,
    /// committing the no-op tx so the link stays retryable. On a winning consume,
    /// `account_id` is the absorbed account B. Fail-closed guards (each -> `Ok(None)`,
    /// no proposal written): B equals `surviving_account_id` (cannot merge into self);
    /// B already `closed_at`. Otherwise B's ACTIVE principal count is taken and a
    /// single-use, 10-minute proposal row is inserted naming `surviving_account_id`
    /// (A) as surviving and B as absorbed. Returns the staged proposal. The raw code
    /// never reaches the database; `proposal_id` is server-assigned. Tenant-scoped
    /// under forced RLS throughout.
    async fn stage_merge_proposal(
        &self,
        _tenant_id: &str,
        _surviving_account_id: uuid::Uuid,
        _merge_code_hash: &str,
    ) -> Result<Option<StagedMergeProposal>, DatabaseError> {
        Err(DatabaseError::Pool(
            "stage_merge_proposal not implemented".to_string(),
        ))
    }

    /// Execute a previously-staged merge atomically, in ONE RLS-scoped transaction:
    /// a partial merge is never observable. The proposal is loaded + consumed with a
    /// single conditional UPDATE (`consumed_at = now()` WHERE not-yet-consumed AND
    /// unexpired AND `surviving_account_id` matches the auth-derived A): an expired /
    /// not-owned / already-consumed / unknown proposal affects zero rows -> `Ok(None)`.
    /// The consume re-validates ownership (only A can execute its own proposal). B
    /// (absorbed) is re-checked still-open; if B closed mid-flight, return `Ok(None)`
    /// WITHOUT committing so the consume rolls back (the proposal stays usable). On the
    /// happy path, in the SAME tx: active principal links, authenticators, and owned
    /// public runs move from B to A; ALL of B's live sessions are revoked; B is closed;
    /// and a count-only `account_merged` audit row is appended. The whole tx commits
    /// together; any failure rolls everything back. Returns the principal and
    /// authenticator move counts. Tenant-scoped under forced RLS throughout.
    async fn execute_merge(
        &self,
        _tenant_id: &str,
        _surviving_account_id: uuid::Uuid,
        _proposal_id: uuid::Uuid,
    ) -> Result<Option<ExecutedMerge>, DatabaseError> {
        Err(DatabaseError::Pool(
            "execute_merge not implemented".to_string(),
        ))
    }

    /// Enumerate submissions that still need a perplexity-scoring gate
    /// decision, across ALL tenants. Runs on the narrow, cross-tenant
    /// `trace_gate_driver` pool (never the tenant-scoped runtime pool) — the
    /// permissive `USING (true)` SELECT policies installed by migration V36
    /// authorize cross-tenant row visibility, and V42 narrows that grant to
    /// the columns the driver's queries actually use (mirroring V38).
    /// authorize the read.
    ///
    /// A submission qualifies when: it has a `submitted_envelope` object ref
    /// that is neither invalidated nor deleted; it has no `trace_gate_decisions`
    /// row yet; its recorded attempt count in `trace_gate_evaluation_attempts`
    /// is below `max_attempts`; and (if it has been attempted before) enough
    /// time has passed since `last_attempt_at` per an exponential backoff of
    /// `backoff_base_seconds * 2^attempts`. Results are ordered oldest-received
    /// first and capped at `limit`.
    ///
    /// The default implementation returns a "not configured" error, which is
    /// the correct behavior for any [`Database`] impl that never wires a
    /// gate-driver pool (e.g. test doubles). [`postgres::PgBackend`] overrides
    /// this with the real query and its own "pool not configured" check when
    /// `TRACE_COMMONS_GATE_DRIVER_DATABASE_URL` is unset.
    /// How many submissions the enumeration above would eventually return,
    /// ignoring its `LIMIT`. The driver's backlog depth.
    ///
    /// Deliberately the SAME predicate as `list_submissions_needing_gate_decision`,
    /// so the number answers "work this driver can actually pick up" rather
    /// than "rows with no decision". Two exclusions follow from that and are
    /// worth knowing before reading a zero as "nothing outstanding":
    ///
    /// - Submissions in exponential backoff are excluded until their next
    ///   attempt is due. They come back on their own.
    /// - Submissions at or past `max_attempts` are excluded permanently. They
    ///   do not come back without an operator resetting their attempt row, so
    ///   a backlog of zero can coexist with traces that will never be scored.
    async fn count_submissions_needing_gate_decision(
        &self,
        _now: chrono::DateTime<chrono::Utc>,
        _max_attempts: i32,
        _backoff_base_seconds: i64,
    ) -> Result<i64, DatabaseError> {
        Err(DatabaseError::Pool(
            "gate-driver pool not configured".to_string(),
        ))
    }

    async fn list_submissions_needing_gate_decision(
        &self,
        _now: chrono::DateTime<chrono::Utc>,
        _max_attempts: i32,
        _backoff_base_seconds: i64,
        _limit: i64,
    ) -> Result<Vec<crate::trace_corpus_storage::GateWorkItem>, DatabaseError> {
        Err(DatabaseError::Pool(
            "gate-driver pool not configured".to_string(),
        ))
    }

    /// Enumerate submissions awaiting the server-side NEAR AI PII backstop,
    /// across ALL tenants. Runs on the narrow, cross-tenant
    /// `trace_pii_backstop_driver` pool (never the tenant-scoped runtime
    /// pool) — the permissive `USING (true)` SELECT policies installed by
    /// migration V38 authorize the read.
    ///
    /// A submission qualifies when: its status is `awaiting_pii_backstop`; it
    /// has a `submitted_envelope` object ref that is neither invalidated nor
    /// deleted; its recorded attempt count in `trace_pii_backstop` is below
    /// `max_attempts`; and (if it has been attempted before) enough time has
    /// passed since `last_attempt_at` per an exponential backoff of
    /// `backoff_base_seconds * 2^attempts`. Results are ordered
    /// oldest-received first and capped at `limit`.
    ///
    /// The default implementation returns a "not configured" error, which is
    /// the correct behavior for any [`Database`] impl that never wires a
    /// pii-backstop-driver pool (e.g. test doubles). [`postgres::PgBackend`]
    /// overrides this with the real query and its own "pool not configured"
    /// check when `TRACE_COMMONS_PII_BACKSTOP_DRIVER_DATABASE_URL` is unset.
    async fn list_submissions_awaiting_pii_backstop(
        &self,
        _now: chrono::DateTime<chrono::Utc>,
        _max_attempts: i32,
        _backoff_base_seconds: i64,
        _limit: i64,
    ) -> Result<Vec<crate::trace_corpus_storage::GateWorkItem>, DatabaseError> {
        Err(DatabaseError::Pool(
            "pii-backstop-driver pool not configured".to_string(),
        ))
    }

    /// Enumerate submissions that ALREADY have a `trace_gate_decisions` row,
    /// cross-tenant, ordered oldest-received first, capped at `limit`. This is
    /// the sibling of [`Self::list_submissions_needing_gate_decision`] used by
    /// the perplexity re-score maintenance task: it targets the historical
    /// decisions that need their perplexity recomputed. Like the enumeration it
    /// mirrors, it reads through the gate-driver reader pool with NO tenant GUC
    /// set — the `trace_gate_driver` role's permissive cross-tenant SELECT
    /// policies authorize the read.
    ///
    /// The default returns an empty list, which is correct for any [`Database`]
    /// impl that never wires a gate-driver pool (e.g. test doubles).
    /// [`postgres::PgBackend`] overrides this with the real query.
    async fn list_submissions_with_gate_decision(
        &self,
        _limit: i64,
    ) -> Result<Vec<crate::trace_corpus_storage::GateWorkItem>, DatabaseError> {
        Ok(Vec::new())
    }

    /// Enumerate decision rows for shadow credit-quality scoring, cross-tenant,
    /// oldest-decided first, capped at `limit`. Reads through the gate-driver
    /// reader pool with NO tenant GUC (the trace_gate_driver role's permissive
    /// cross-tenant SELECT policies authorize it). Default: empty (test doubles
    /// / backends without a gate-driver pool).
    async fn list_gate_decisions_for_credit_scoring(
        &self,
        _limit: i64,
    ) -> Result<Vec<crate::trace_corpus_storage::GateCreditInput>, DatabaseError> {
        Ok(Vec::new())
    }

    /// Enumerate dedup signal rows (migration V40), cross-tenant, oldest-decided
    /// first, capped at `limit`. Reads through the gate-driver reader pool with
    /// NO tenant GUC (the trace_gate_driver role's permissive cross-tenant
    /// SELECT policies authorize it). Default: empty (test doubles / backends
    /// without a gate-driver pool).
    async fn list_dedup_signals(
        &self,
        _limit: i64,
    ) -> Result<Vec<crate::trace_corpus_storage::DedupSignalRow>, DatabaseError> {
        Ok(Vec::new())
    }

    /// Enumerate every decision row for the dedup re-derivation pass,
    /// cross-tenant, ordered `decided_at ASC, decision_id ASC`, capped at
    /// `limit`. Not filtered by stamp: the pass reuses rows already on its
    /// target stamp and derives the rest, which is what makes it resumable.
    /// Reads through the gate-driver reader pool with NO tenant GUC. Every
    /// column is granted to `trace_gate_driver` by V45 and V57. Default:
    /// empty (test doubles / backends without a gate-driver pool).
    async fn list_dedup_rederive_rows(
        &self,
        _limit: i64,
    ) -> Result<Vec<crate::trace_corpus_storage::DedupRederiveRow>, DatabaseError> {
        Ok(Vec::new())
    }

    /// Enumerate decision identifiers and times, cross-tenant, NEWEST first
    /// (`decided_at DESC, decision_id DESC`), capped at `limit`: the credit
    /// estimate eval's enumeration. Newest first so a capped run reads the
    /// most recent decisions, and so every submission it reaches has its
    /// latest decision among the rows -- the decision its label comes from.
    /// Reads through the gate-driver reader pool with NO tenant GUC; every
    /// column is granted to `trace_gate_driver` by V45. Default: empty (test
    /// doubles / backends without a gate-driver pool).
    async fn list_recent_gate_decision_keys(
        &self,
        _limit: i64,
    ) -> Result<Vec<crate::trace_corpus_storage::GateDecisionKeyRow>, DatabaseError> {
        Ok(Vec::new())
    }

    /// The settlement label of each asked-about submission, for the credit
    /// estimate eval under `target=settlement`: one row per submission with
    /// a pipeline gate decision, labelled from the run that decision names
    /// (see [`crate::trace_corpus_storage::SettlementLabel`]). A submission
    /// with no pipeline decision has no row. Tenant-scoped through the
    /// forced-RLS trace pool; the id list narrows, it never widens. Default:
    /// empty (test doubles / backends without the pipeline tables).
    async fn list_settlement_labels(
        &self,
        _tenant_id: &str,
        _submission_ids: &[uuid::Uuid],
    ) -> Result<Vec<crate::trace_corpus_storage::SettlementLabelRow>, DatabaseError> {
        Ok(Vec::new())
    }

    /// Enumerate correction-value signal rows (migration V48), cross-tenant,
    /// oldest-decided first, capped at `limit`. Reads through the gate-driver
    /// reader pool with NO tenant GUC (the trace_gate_driver role's permissive
    /// cross-tenant SELECT policies authorize it). Corrections cluster
    /// cross-tenant for the same reason traces do: the same correction pasted
    /// into two tenants is one correction. Default: empty (test doubles /
    /// backends without a gate-driver pool).
    async fn list_correction_signals(
        &self,
        _limit: i64,
    ) -> Result<Vec<crate::trace_corpus_storage::CorrectionSignalRow>, DatabaseError> {
        Ok(Vec::new())
    }

    /// Enumerate per-contributor cap signal rows (migration V41), cross-tenant,
    /// joining each decision to its submission for the contributor identity
    /// (`auth_principal_ref`), ordered `(auth_principal_ref, decided_at ASC)`
    /// so the recompute pass can group per contributor and forward-accumulate
    /// in time order. Reads through the gate-driver reader pool with NO tenant
    /// GUC (the trace_gate_driver role's permissive cross-tenant SELECT
    /// policies authorize it). Default: empty (test doubles / backends
    /// without a gate-driver pool).
    async fn list_contributor_cap_signals(
        &self,
        _limit: i64,
    ) -> Result<Vec<crate::trace_corpus_storage::ContributorCapSignalRow>, DatabaseError> {
        Ok(Vec::new())
    }

    /// Look up the LATEST gate-decision score for each of `submission_ids`,
    /// cross-tenant. Reads through the gate-driver reader pool with NO tenant
    /// GUC (the trace_gate_driver role's permissive cross-tenant SELECT
    /// policies authorize it). Ids with no decision row are simply absent
    /// from the result — this is not an error. Default: empty (test doubles
    /// / backends without a gate-driver pool).
    async fn list_scores_by_submission_ids(
        &self,
        _submission_ids: &[uuid::Uuid],
    ) -> Result<Vec<crate::trace_corpus_storage::TraceScoreBySubmissionRow>, DatabaseError> {
        Ok(Vec::new())
    }

    /// Enumerate the LATEST gate-decision score for every submission owned
    /// by `(tenant_id, auth_principal_ref)` — the read behind server-signed
    /// score attestations (see `trace_score_attestation`). Unlike
    /// `list_scores_by_submission_ids`, both identity fields are supplied by
    /// the CALLER'S SERVER-SIDE code, resolved from an authenticated
    /// request context, never from a client-supplied id list; this method
    /// exists so that resolution never has to widen into a
    /// caller-suppliable filter. Reads through the gate-driver reader pool
    /// with NO tenant GUC (the trace_gate_driver role's permissive
    /// cross-tenant SELECT policies authorize it); the `tenant_id`/
    /// `auth_principal_ref` equality checks are enforced in the query text
    /// itself as defense in depth. Default: empty (test doubles / backends
    /// without a gate-driver pool).
    async fn list_own_gate_decision_scores(
        &self,
        _tenant_id: &str,
        _auth_principal_ref: &str,
        _limit: i64,
    ) -> Result<Vec<crate::trace_corpus_storage::TraceScoreBySubmissionRow>, DatabaseError> {
        Ok(Vec::new())
    }

    /// The scoped counterpart of `list_own_gate_decision_scores`: for each of
    /// `submission_ids` OWNED by `(tenant_id, auth_principal_ref)`, return
    /// its latest gate-decision score, or `None` when it has not been scored
    /// yet. Ids the principal does not own are simply absent from the
    /// result, whether they belong to another principal or to nobody — the
    /// caller must not be able to tell those apart, or this becomes a probe
    /// for other contributors' submission ids.
    ///
    /// `submission_ids` is the ONLY caller-supplied input; the identity pair
    /// still comes from the authenticated request context, never from the
    /// request body. The id list narrows a set the principal already owns,
    /// so it cannot widen the read. Default: empty (test doubles / backends
    /// without a gate-driver pool).
    async fn list_own_gate_decision_scores_for_submissions(
        &self,
        _tenant_id: &str,
        _auth_principal_ref: &str,
        _submission_ids: &[uuid::Uuid],
    ) -> Result<Vec<crate::trace_corpus_storage::OwnSubmissionScoreRow>, DatabaseError> {
        Ok(Vec::new())
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
    /// The account's binding state (Z2 S1), read in the SAME query that
    /// validated the session, so the unbound gate costs no second round trip
    /// and cannot see a different account than the session did. No binding row
    /// is [`AccountBindingState::Legacy`](crate::account_binding::AccountBindingState::Legacy),
    /// which is never gated.
    pub binding: crate::account_binding::AccountBindingState,
    /// The session's expiry, as stored once this validation is done. A
    /// browser session's rotation never moves it; the auth middleware caps a
    /// rotated cookie's Max-Age at what is left of it, so a rotated cookie never
    /// outlives its row. A native session's rotation extends it (see
    /// `account_native_auth::NATIVE_SESSION_MAX_LIFETIME_DAYS`), and this is the
    /// extended value, which the middleware hands back beside the new token.
    pub expires_at: chrono::DateTime<chrono::Utc>,
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

/// Everything native passkey `create/finish` writes for a new passkey-origin
/// account (Z2 S2). Every value is server-minted or came out of a verified
/// attestation: the tenant from the OS RNG, the account id from `start`, the
/// credential from `finish_passkey_registration`. Nothing here is client
/// supplied except the optional label, which the start handler bounded.
#[derive(Debug)]
pub struct NewPasskeyOriginAccount<'a> {
    pub tenant_id: &'a str,
    pub account_id: uuid::Uuid,
    /// Canonical base64url credential id; globally UNIQUE (V32), so a replayed
    /// attestation fails the insert and rolls the whole account back.
    pub credential_id: &'a str,
    /// The serialized webauthn-rs `Passkey`.
    pub passkey: &'a serde_json::Value,
    pub label: Option<&'a str>,
    /// The weak native session minted with the account. Its `client_kind`
    /// must be `native`; the backend refuses anything else.
    pub session: NewSession<'a>,
    /// The unbound-account ceiling, re-checked inside the transaction.
    pub ceiling: i64,
}

/// What `create_passkey_origin_account` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasskeyOriginAccountOutcome {
    /// Every row was written and committed.
    Created,
    /// The ceiling was reached when re-checked inside the transaction; nothing
    /// was written.
    CeilingReached,
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
    /// True when this identity is the account's designated payout target
    /// (`payout_designated_at IS NOT NULL` on an ACTIVE row). At most one active
    /// identity per account carries this flag (enforced by a partial-unique index).
    pub is_payout: bool,
}

/// Why [`Database::resolve_payout_near_account_id`] withheld a payout target. A
/// fail-closed signal: settlement must hold (not guess) when no unambiguous
/// designation exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayoutHoldReason {
    /// The account has no ACTIVE NEAR identities at all.
    NoneEnrolled,
    /// The account has two or more ACTIVE NEAR identities and none is designated
    /// as the payout target, so there is no unambiguous choice.
    AmbiguousNoDesignation,
}

impl PayoutHoldReason {
    /// The coarse label a held settlement line records
    /// (`near_payout_hold_reason`). A label only; it carries no account
    /// identity.
    pub fn label(&self) -> &'static str {
        match self {
            Self::NoneEnrolled => "none_enrolled",
            Self::AmbiguousNoDesignation => "ambiguous_no_designation",
        }
    }
}

/// The prefix of a credit settlement key that names an account:
/// `account:{account_id}`.
pub const ACCOUNT_SETTLEMENT_KEY_PREFIX: &str = "account:";

/// The key credit settlement groups a principal's credit under: its account,
/// `account:{account_id}`, when `principal_to_account` links it to one
/// ([`Database::resolve_principals_to_accounts`]), so every principal of an
/// account settles into one account line and one payout; otherwise the
/// principal itself.
pub fn settlement_group_key(
    auth_principal_ref: &str,
    principal_to_account: &std::collections::HashMap<String, uuid::Uuid>,
) -> String {
    principal_to_account
        .get(auth_principal_ref)
        .map(|account_id| format!("{ACCOUNT_SETTLEMENT_KEY_PREFIX}{account_id}"))
        .unwrap_or_else(|| auth_principal_ref.to_string())
}

/// The account a settlement key names ([`settlement_group_key`]), or `None`
/// for a principal key.
pub fn settlement_key_account_id(settlement_key: &str) -> Option<uuid::Uuid> {
    settlement_key
        .strip_prefix(ACCOUNT_SETTLEMENT_KEY_PREFIX)
        .and_then(|raw| uuid::Uuid::parse_str(raw).ok())
}

/// Outcome of resolving an account's payout NEAR account id. The carried `String`
/// is the human-readable `near_account_id` to pay; a [`PayoutResolution::Hold`]
/// means settlement must withhold until the account designates a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayoutResolution {
    /// The account explicitly designated this `near_account_id` as its payout target.
    Designated(String),
    /// The account has exactly one ACTIVE NEAR identity and none is explicitly
    /// designated; pay it as the sole unambiguous choice.
    SoleActive(String),
    /// No unambiguous payout target; hold with the given reason.
    Hold(PayoutHoldReason),
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

/// One device linked to an account through NEAR sign-in, as
/// [`Database::list_account_devices`] reports it. `device_key_id` and
/// `public_key` are public identifiers, not secrets. `last_seen_at` is the
/// newest `last_seen_at` of the device's native sessions, or `None` when no
/// session names the device (one minted before sessions carried it).
#[derive(Debug, Clone)]
pub struct AccountDeviceSummary {
    pub device_key_id: String,
    pub public_key: String,
    pub label: Option<String>,
    /// `near` (wallet sign-in) or `near_ai` (NEAR AI sign-in).
    pub onboarding_origin: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: Option<chrono::DateTime<chrono::Utc>>,
    /// True when the session the caller presented belongs to this device.
    pub this_device: bool,
    /// How many of the device's traces still stand: submissions under its
    /// own principal, or an upload principal bound to it (V135), that are
    /// received, accepted or quarantined and not withdrawn, revoked or
    /// purged. Counted only while the device's principal is linked to this
    /// account. Zero is a count, not "unknown".
    pub contributed_traces: i64,
    /// When the newest of those was received; `None` with none.
    pub last_contributed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Safe label a backend without the V141 methods refuses with.
pub const MODEL_TRAINING_CONSENT_BACKEND_MISSING: &str = "model_training_consent_backend_missing";

/// What [`Database::list_model_training_consent_eligible`] found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelTrainingConsentEligible {
    pub eligible_count: u64,
    pub submission_ids: Vec<uuid::Uuid>,
}

/// A grant, as [`Database::grant_model_training_consent`] takes it.
#[derive(Debug, Clone, Copy)]
pub struct ModelTrainingConsentGrant<'a> {
    pub tenant_id: &'a str,
    /// The principals whose traces the caller may grant on: its own, and
    /// its account's when it is linked to one.
    pub owner_principal_refs: &'a [String],
    pub submission_ids: &'a [uuid::Uuid],
    /// The authenticated principal storage ref, recorded on the grant row.
    pub actor_principal_ref: &'a str,
    /// When the contributor gave the consent; stored as given.
    pub granted_at: chrono::DateTime<chrono::Utc>,
}

/// One trace's answer from [`Database::grant_model_training_consent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelTrainingConsentGrantRow {
    pub submission_id: uuid::Uuid,
    pub outcome: trace_commons_protocol::model_training_consent::ModelTrainingConsentOutcome,
    /// The recorded grant time; `None` when nothing was recorded (not
    /// eligible, or the trace was sent with the use).
    pub granted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The stored gate decision whose withheld scoring award a model-training
/// grant re-runs, as [`Database::withheld_policy_mismatch_gate_award`]
/// returns it. Both floors passed, by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithheldGateAward {
    pub decision_id: uuid::Uuid,
    pub gate_policy_version: String,
    pub gate_version_hash: String,
    pub vector_entry_id: Option<uuid::Uuid>,
    pub chunks_capped: bool,
}

/// A code to mint, as [`Database::mint_account_device_code`] takes it. Every
/// value is a hash, an id or a label; the code's secret never reaches the
/// database.
#[derive(Debug, Clone)]
pub struct NewAccountDeviceCode<'a> {
    pub tenant_id: &'a str,
    pub account_id: uuid::Uuid,
    pub minter_device_key_id: &'a str,
    pub mint_nonce_hash: &'a str,
    pub code_hash: &'a str,
    pub label: Option<&'a str>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// Refuse when the account already holds this many unexpired, unredeemed
    /// codes.
    pub outstanding_cap: i64,
}

/// What [`Database::mint_account_device_code`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MintAccountDeviceCodeOutcome {
    Minted {
        code_id: uuid::Uuid,
    },
    /// The proving key is not an active device of the session's account, or
    /// it joined the account by code (a code-joined device never mints).
    MinterNotADevice,
    TooManyOutstanding,
    /// The proof's nonce was already used.
    NonceReused,
}

/// A minted code as its account sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDeviceCodeStatus {
    pub expired: bool,
    pub redeemed_device_key_id: Option<String>,
}

/// What [`Database::redeem_account_device_code`] or
/// [`Database::renew_device_session`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceSignInOutcome {
    SignedIn(crate::account_onboarding::ProvisionedNearAccount),
    /// Redeem: no unexpired, unredeemed code with that hash. Renew: the key
    /// is not an active device that joined by code.
    NotFound,
    /// Redeem: the device key is registered to another tenant or account.
    DeviceKeyElsewhere,
    /// Renew: the proof's nonce was already used.
    NonceReused,
    /// Renew: the device renewed too recently.
    TooSoon,
}

/// Outcome of [`Database::revoke_account_device`]. `removed` is `false` for an
/// unknown, already-revoked or other-account device; the counts say what the
/// revoke ended, for a hash-free audit row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RevokeAccountDeviceResult {
    pub removed: bool,
    pub sessions_revoked: u64,
    pub grants_revoked: u64,
    /// The account's live native sessions that named neither a device key nor
    /// a passkey (issued before V139, or by the login-link native sign-in),
    /// revoked with the
    /// device because they cannot be told apart from its own. The caller's
    /// session is never among them.
    pub unattributed_sessions_revoked: u64,
}

/// Outcome of [`Database::stage_merge_proposal`]: device B's login-link was
/// consumed as proof-of-control and a single-use, time-bounded merge proposal
/// was staged. Carries only durable account/proposal ids and an attribution-only
/// active-principal count for `absorbed_account_id`; never any code or secret.
#[derive(Debug, Clone, Copy)]
pub struct StagedMergeProposal {
    pub proposal_id: uuid::Uuid,
    pub absorbed_account_id: uuid::Uuid,
    pub absorbed_principal_count: i64,
}

/// Outcome of a successful [`Database::execute_merge`]: how many active principal
/// links and strong authenticators (webauthn credentials + NEAR identities) were
/// re-keyed from the absorbed account onto the surviving account. Counts only;
/// never identifiers.
#[derive(Debug, Clone, Copy)]
pub struct ExecutedMerge {
    pub principals_moved: i64,
    pub authenticators_moved: i64,
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

/// All-time, all-tenant totals for [`Database::compute_register_stats_totals`].
/// Feeds the `trace_register_stats` singleton row that the public register
/// endpoint reads; computed from the same tables the tenant-scoped admin
/// summaries already read, crossed the same way `compute_corpus_analytics_summary`
/// crosses tenants -- per-tenant transactions under the RLS tenant GUC,
/// never `BYPASSRLS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterStatsTotals {
    pub traces_accepted: i64,
    /// Distinct `credit_account_ref` values across every tenant's credit
    /// ledger.
    pub contributors: i64,
    /// Sum of positive `points_delta` across every tenant's credit ledger.
    pub points_issued: i64,
}

/// The `trace_register_stats` singleton row, as read or as just written by
/// [`Database::fetch_register_stats_row`] / [`Database::write_register_stats_row`].
/// `refreshed_at` is `None` until a refresh has actually run; the public
/// endpoint (a later slice) must refuse to publish while it is `None`,
/// because a zero would be a claim about the register nobody made.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegisterStatsRow {
    pub traces_accepted: i64,
    pub contributors: i64,
    pub points_issued: i64,
    /// The computed/never-computed marker: TRUE until a refresh has run, and
    /// cleared by every refresh. Not an operator control -- see `suppressed`.
    pub withheld: bool,
    /// The operator's lever. No refresh ever writes it, so suppression set
    /// during an incident survives the next scheduled run.
    pub suppressed: bool,
    pub as_of: chrono::DateTime<chrono::Utc>,
    pub refreshed_at: Option<chrono::DateTime<chrono::Utc>>,
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

/// Exact reviewed publication input for an account-owned accepted submission.
/// `expected_publication_version` is the compare-and-swap version shown during review.
#[derive(Debug, Clone)]
pub struct PublicRunWrite {
    pub tenant_id: String,
    pub publication_id: uuid::Uuid,
    pub account_id: uuid::Uuid,
    pub submission_id: uuid::Uuid,
    pub slug: String,
    pub title: String,
    pub outcome_summary: String,
    pub correction_excerpt: Option<String>,
    pub workflow: String,
    pub reuse_permission: trace_commons_protocol::public_run::PublicRunReusePermission,
    pub evidence: Vec<trace_commons_protocol::public_run::PublicRunEvidenceDraft>,
    pub task_success: trace_commons_protocol::trace_contribution::TaskSuccess,
    pub contributed_version: String,
    pub approval_sha256: String,
    pub source_publication_id: Option<uuid::Uuid>,
    pub expected_publication_version: i32,
}

/// Private owner row, including approval and provenance fields plus an optional
/// unpublication tombstone. This type must never cross the public run API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRunRow {
    pub tenant_id: String,
    pub publication_id: uuid::Uuid,
    pub account_id: uuid::Uuid,
    pub submission_id: uuid::Uuid,
    pub slug: String,
    pub title: String,
    pub outcome_summary: String,
    pub correction_excerpt: Option<String>,
    pub workflow: String,
    pub reuse_permission: trace_commons_protocol::public_run::PublicRunReusePermission,
    pub evidence: Vec<trace_commons_protocol::public_run::PublicRunEvidenceDraft>,
    pub task_success: trace_commons_protocol::trace_contribution::TaskSuccess,
    pub contributed_version: String,
    pub approval_sha256: String,
    pub source_publication_id: Option<uuid::Uuid>,
    pub version: i32,
    pub published_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub unpublished_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Active source identity resolved for a reviewed workflow variation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRunSourceRow {
    pub publication_id: uuid::Uuid,
    pub slug: String,
    pub title: String,
}

/// Identifier-free projection safe for the public community route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRunPageData {
    pub slug: String,
    pub title: String,
    pub outcome_summary: String,
    pub correction_excerpt: Option<String>,
    pub workflow: String,
    pub reuse_permission: trace_commons_protocol::public_run::PublicRunReusePermission,
    pub evidence: Vec<trace_commons_protocol::public_run::PublicRunEvidence>,
    pub task_success: trace_commons_protocol::trace_contribution::TaskSuccess,
    pub contributed_version: String,
    pub version: u32,
    pub published_at: chrono::DateTime<chrono::Utc>,
    pub source: Option<trace_commons_protocol::public_run::PublicRunLink>,
    pub source_unavailable: bool,
    pub variations: Vec<trace_commons_protocol::public_run::PublicRunLink>,
}

/// Atomic publication result containing the private owner row and its matching
/// public projection from the same transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRunMutation {
    pub row: PublicRunRow,
    pub page: PublicRunPageData,
}

/// Account-only publication snapshot. The row may be an unpublished tombstone;
/// `page` is present only while the workflow remains public.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRunOwnerData {
    pub row: Option<PublicRunRow>,
    pub page: Option<PublicRunPageData>,
    pub retained_source_slug: Option<String>,
}

/// Result of an idempotent unpublish, including the owner version to bind the
/// next reviewed publication attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicRunUnpublishMutation {
    pub unpublished: bool,
    pub expected_publication_version: u32,
}

pub const PUBLIC_RUN_SOURCE_NOT_ACCEPTED: &str = "public_run_source_not_accepted";
pub const PUBLIC_RUN_PROVENANCE_CYCLE: &str = "public_run_provenance_cycle";
pub const PUBLIC_RUN_ACCOUNT_CLOSED: &str = "public_run_account_closed";
pub const PUBLIC_RUN_VERSION_CONFLICT: &str = "public_run_version_conflict";

/// Eviction receipt written atomically with a community-profile
/// withdrawal. Proves the published snapshot surface was asked to drop
/// the contributor rather than relying on an assumed cache TTL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunityWithdrawalEvictionRow {
    pub eviction_id: uuid::Uuid,
    pub tenant_id: String,
    pub principal_ref: String,
    pub display_handle: Option<String>,
    pub handle_normalized: Option<String>,
    pub withdrawn_at: chrono::DateTime<chrono::Utc>,
    pub invalidation_requested_at: chrono::DateTime<chrono::Utc>,
    pub window_label: String,
    pub metric: String,
}

#[derive(Debug, Clone)]
pub struct DeviceKeyWrite {
    pub device_key_id: String,
    pub tenant_id: String,
    pub public_key: String,
    pub invite_subject_hash: String,
    pub client_info: serde_json::Value,
    /// Grant-scope override from the invite, when redemption goes through
    /// the DB-authoritative registry. `None` (every pre-existing call site)
    /// preserves today's behavior byte-for-byte: the hardcoded
    /// `DEFAULT_ONBOARDING_CONSENT_SCOPES`. An empty `Some(vec![])` -- as
    /// carried by invites imported from the file allowlist, which never had
    /// a scopes column -- is treated the same as `None`, not as "grant
    /// nothing": provisioning an imported invite with zero consent scopes
    /// would silently strip permissions the pilot already granted it.
    pub allowed_consent_scopes: Option<Vec<String>>,
    /// Same override/empty-fallback behavior as `allowed_consent_scopes`,
    /// against `DEFAULT_ONBOARDING_ALLOWED_USES`.
    pub allowed_uses: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceKeyRecord {
    pub device_key_id: String,
    pub tenant_id: String,
    pub public_key: String,
    pub invite_subject_hash: Option<String>,
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

mod token_bundles;

// ---------------------------------------------------------------------------
// Payout rails (V132; docs/superpowers/specs/2026-10-21-nearai-credit-payout-
// design.md, sections 4 and 5).
//
// One set of tables serves every rail, each row keyed by `rail_id`. Every
// tenant-scoped call takes the auth-derived tenant and runs under
// `trace_current_tenant_id()`; the two deployment-wide reads and writes, and
// the cross-tenant enumeration, run under their narrow roles with
// `SET LOCAL ROLE`, so the role ends with the transaction on every path.
// Amounts are a rail's atomic units, `u128`, stored as NUMERIC(39,0).
// ---------------------------------------------------------------------------

use trace_commons_gate_api::pipeline::{AtomicUnits, InstrumentId};
use trace_commons_gate_api::{RailId, RailRate, RecipientKind};
use uuid::Uuid;

/// A link's lifecycle on one rail. Per account and rail, at most one
/// `Waiting` link and at most one link that is `Active` or `Closing`: a link
/// goes active only once its predecessor is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PayoutLinkState {
    Waiting,
    Active,
    Closing,
    Closed,
}

impl PayoutLinkState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Active => "active",
            Self::Closing => "closing",
            Self::Closed => "closed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "waiting" => Self::Waiting,
            "active" => Self::Active,
            "closing" => Self::Closing,
            "closed" => Self::Closed,
            _ => return None,
        })
    }
}

/// A payout line's state. `Pending` until its outcome is recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PayoutLineState {
    Pending,
    Confirmed,
    NoOp,
    Held,
    Failed,
}

impl PayoutLineState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::NoOp => "no_op",
            Self::Held => "held",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "confirmed" => Self::Confirmed,
            "no_op" => Self::NoOp,
            "held" => Self::Held,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

/// A credit accrual adds to what an account is owed on a rail; a reversal
/// takes back exactly what its credit added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PayoutAccrualKind {
    Credit,
    Reversal,
}

impl PayoutAccrualKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Credit => "credit",
            Self::Reversal => "reversal",
        }
    }
}

/// What a `trace_payout_audit` row records. Hash-only: the row carries an
/// actor ref hash, a link id hash, states and a label, never an account,
/// recipient, program, amount or rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PayoutAuditAction {
    LinkRequested,
    UnlinkRequested,
    LinkCreated,
    LinkConfirmed,
    LinkActivated,
    LinkClosing,
    LinkClosed,
    LinkReleased,
    LinkRecipientUnavailable,
    HoldSet,
    HoldCleared,
    AccrualClaimed,
}

impl PayoutAuditAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LinkRequested => "link_requested",
            Self::UnlinkRequested => "unlink_requested",
            Self::LinkCreated => "link_created",
            Self::LinkConfirmed => "link_confirmed",
            Self::LinkActivated => "link_activated",
            Self::LinkClosing => "link_closing",
            Self::LinkClosed => "link_closed",
            Self::LinkReleased => "link_released",
            Self::LinkRecipientUnavailable => "link_recipient_unavailable",
            Self::HoldSet => "hold_set",
            Self::HoldCleared => "hold_cleared",
            Self::AccrualClaimed => "accrual_claimed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "link_requested" => Self::LinkRequested,
            "unlink_requested" => Self::UnlinkRequested,
            "link_created" => Self::LinkCreated,
            "link_confirmed" => Self::LinkConfirmed,
            "link_activated" => Self::LinkActivated,
            "link_closing" => Self::LinkClosing,
            "link_closed" => Self::LinkClosed,
            "link_released" => Self::LinkReleased,
            "link_recipient_unavailable" => Self::LinkRecipientUnavailable,
            "hold_set" => Self::HoldSet,
            "hold_cleared" => Self::HoldCleared,
            "accrual_claimed" => Self::AccrualClaimed,
            _ => return None,
        })
    }

    /// Whether this action may record a `from` to `to` link transition made
    /// by [`PayoutStore::transition_payout_link`]: `waiting` to `active` is
    /// `LinkActivated`; `active` to `closing` is `LinkClosing` (a change or
    /// an unlink) or `LinkReleased` (an operator release).
    pub fn fits_link_transition(self, from: PayoutLinkState, to: PayoutLinkState) -> bool {
        use PayoutLinkState::{Active, Closing, Waiting};
        matches!(
            (from, to, self),
            (Waiting, Active, Self::LinkActivated)
                | (Active, Closing, Self::LinkClosing | Self::LinkReleased)
        )
    }
}

/// A recipient as stored: sealed, with a keyed blind index, the key and
/// pepper refs it was sealed and indexed under, and a keyed index of what
/// proved it. Neither key is in the database. `Debug` prints none of it.
#[derive(Clone, PartialEq, Eq)]
pub struct SealedRecipient {
    pub sealed: serde_json::Value,
    pub recipient_index: String,
    pub recipient_key_ref: String,
    pub index_pepper_ref: String,
    pub subject_index: String,
    /// What the status route reports so the app can check the server holds
    /// the recipient it chose (for `near_ai`,
    /// `trace_commons_protocol::payout_link::near_ai_recipient_ref`). A hash;
    /// `None` on a link written without one.
    pub recipient_ref: Option<String>,
}

impl std::fmt::Debug for SealedRecipient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SealedRecipient([redacted])")
    }
}

/// One accrual to insert. `award_units` and `rail_units` are magnitudes; a
/// reversal is stored negated. `account_id` is `None` for a line keyed to a
/// bare principal, owed to nobody until claimed. `Debug` prints neither the
/// account nor its hash.
#[derive(Clone, PartialEq, Eq)]
pub struct PayoutAccrualWrite {
    pub rail_id: RailId,
    pub kind: PayoutAccrualKind,
    pub settlement_batch_id: Uuid,
    pub credit_account_hash: String,
    pub reversal_event_id: Option<Uuid>,
    pub account_id: Option<Uuid>,
    pub award_units: AtomicUnits,
    pub rail_units: AtomicUnits,
    pub rate_version: String,
    pub instrument_id: InstrumentId,
}

/// One stored accrual on a rail, as accrual reads it back to skip what is
/// already priced and to price a reversal at its credit's version.
/// `award_units` and `rail_units` are magnitudes; `kind` gives the sign.
/// `Debug` prints neither the account nor its hash.
#[derive(Clone, PartialEq, Eq)]
pub struct PayoutAccrual {
    pub rail_id: RailId,
    pub kind: PayoutAccrualKind,
    pub settlement_batch_id: Uuid,
    pub credit_account_hash: String,
    pub reversal_event_id: Option<Uuid>,
    pub account_id: Option<Uuid>,
    pub award_units: AtomicUnits,
    pub rail_units: AtomicUnits,
    pub rate_version: String,
    pub instrument_id: InstrumentId,
}

impl std::fmt::Debug for PayoutAccrual {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PayoutAccrual")
            .field("rail_id", &self.rail_id)
            .field("kind", &self.kind)
            .field("owned", &self.account_id.is_some())
            .field("rate_version", &self.rate_version)
            .finish_non_exhaustive()
    }
}

/// What an account is owed on a rail, from its accruals alone: the credit
/// and reversal sums, each a `u128` of the rail's atomic units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayoutOwed {
    pub credited: u128,
    pub reversed: u128,
    pub accrual_count: i64,
}

impl PayoutOwed {
    /// Credited minus reversed; `None` if reversals exceed credits, which a
    /// reversal that takes back exactly its own credit cannot produce.
    pub fn net(&self) -> Option<u128> {
        self.credited.checked_sub(self.reversed)
    }
}

/// A new link generation. Only `Waiting` or `Active` may be inserted.
/// `Debug` redacts the account, the recipient and the program.
#[derive(Clone, PartialEq, Eq)]
pub struct PayoutLinkWrite {
    pub account_id: Uuid,
    pub rail_id: RailId,
    pub link_id: Uuid,
    pub recipient_kind: RecipientKind,
    pub recipient: SealedRecipient,
    pub program: String,
    pub state: PayoutLinkState,
    pub actor_ref_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayoutLinkInsert {
    Inserted,
    /// The account already has a waiting link on that rail or, for an
    /// active insert, an active or closing one.
    StateTaken,
}

/// The outcome of [`PayoutStore::transition_payout_link`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayoutLinkTransition {
    Moved,
    /// The link is not in `from` in this tenant; nothing changed.
    NotInState,
    /// Activating would give the account a second active-or-closing link on
    /// the rail; nothing changed. A waiting successor is activated by
    /// [`PayoutStore::close_payout_link`] instead.
    LiveLinkTaken,
}

/// The outcome of [`PayoutStore::close_payout_link`]. Every variant but
/// `Closed` changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayoutLinkClose {
    Closed,
    /// The link is not `closing` in this tenant.
    NotClosing,
    /// The link still has a `pending` line, which recovery resolves first.
    PendingLineOpen,
    /// The named successor is not a `waiting` link of the same account and
    /// rail.
    SuccessorNotWaiting,
}

/// A stored link. `Debug` redacts the recipient and the program.
#[derive(Clone, PartialEq, Eq)]
pub struct PayoutLink {
    pub account_id: Uuid,
    pub rail_id: RailId,
    pub link_id: Uuid,
    pub recipient_kind: RecipientKind,
    pub state: PayoutLinkState,
    pub recipient: SealedRecipient,
    pub program: String,
    pub final_paid: Option<AtomicUnits>,
    pub unlink_requested_at: Option<chrono::DateTime<chrono::Utc>>,
    pub recipient_unavailable_at: Option<chrono::DateTime<chrono::Utc>>,
    pub version: i64,
}

impl std::fmt::Debug for PayoutLink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PayoutLink")
            .field("rail_id", &self.rail_id)
            .field("state", &self.state)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// A line to write `pending`, committed before any network call. `Debug`
/// redacts the account.
#[derive(Clone, PartialEq, Eq)]
pub struct PayoutLineWrite {
    pub line_id: Uuid,
    pub link_id: Uuid,
    pub account_id: Uuid,
    pub rail_id: RailId,
    pub target_total: AtomicUnits,
    pub delta: AtomicUnits,
    pub owed_through_accrual_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingPayoutLineInsert {
    Inserted,
    /// The link already has an unresolved line, which recovery handles first.
    LinkHasPendingLine,
    /// The link is not `active` (or is not this account's on this rail): a
    /// line is never written to a waiting, closing or closed link.
    LinkNotActive,
}

/// A stored line. `Debug` redacts the account.
#[derive(Clone, PartialEq, Eq)]
pub struct PayoutLine {
    pub line_id: Uuid,
    pub link_id: Uuid,
    pub account_id: Uuid,
    pub rail_id: RailId,
    pub target_total: AtomicUnits,
    pub delta: AtomicUnits,
    pub state: PayoutLineState,
    pub receipt_hash: Option<String>,
    pub attempts: i32,
    pub last_error_label: Option<String>,
}

/// How a pending line is resolved. `Retry` keeps it pending and counts the
/// attempt; every other variant ends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayoutLineResolution {
    Confirmed {
        receipt_hash: String,
        delta: AtomicUnits,
    },
    NoOp,
    Held {
        label: String,
    },
    Failed {
        label: String,
    },
    Retry {
        label: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayoutLineResolve {
    Resolved,
    /// No pending line with that id in this tenant.
    NotPending,
    /// Another line on this rail, in any tenant, already holds that
    /// receipt; nothing changed. The authoritative answer: a caller skips the
    /// record and keeps looking, and the line stays pending.
    ReceiptAlreadyClaimed,
}

/// What a link request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PayoutLinkAction {
    Link,
    Unlink,
}

impl PayoutLinkAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Link => "link",
            Self::Unlink => "unlink",
        }
    }
}

/// A link or unlink request awaiting a passkey confirmation. A link request
/// carries its verified, sealed recipient; an unlink request none. `Debug`
/// redacts the account, the handle and the recipient.
#[derive(Clone, PartialEq, Eq)]
pub struct PayoutLinkRequestWrite {
    pub account_id: Uuid,
    pub rail_id: RailId,
    pub idempotency_key: Uuid,
    pub handle_hash: String,
    pub request_digest: String,
    pub action: PayoutLinkAction,
    pub expected_current_version: Option<i64>,
    pub recipient: Option<(RecipientKind, SealedRecipient)>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub actor_ref_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayoutLinkRequestInsert {
    Inserted,
    /// The same idempotency key with the same request digest: nothing new.
    Replayed,
    /// The same idempotency key with a different request.
    IdempotencyConflict,
}

/// A taken link request. The caller checks `expires_at`. `Debug` redacts
/// the account and the recipient.
#[derive(Clone, PartialEq, Eq)]
pub struct PayoutLinkRequest {
    pub account_id: Uuid,
    pub rail_id: RailId,
    pub idempotency_key: Uuid,
    pub action: PayoutLinkAction,
    pub expected_current_version: Option<i64>,
    pub recipient: Option<(RecipientKind, SealedRecipient)>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// What a confirm writes beside the request it takes: the new link's id and
/// program (for a `link`) and the confirming account's actor hash. The
/// recipient comes from the request row, never from the confirm call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayoutLinkConfirmWrite {
    pub link_id: Uuid,
    pub program: String,
    pub actor_ref_hash: String,
}

/// The outcome of [`PayoutStore::confirm_payout_link_request`]. Every
/// variant but `Confirmed` changed nothing but consuming the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayoutLinkConfirm {
    Confirmed {
        action: PayoutLinkAction,
        /// The account's link version on the rail after the change.
        version: i64,
    },
    /// No request with that handle on this account and rail, or a newer
    /// request on the same account and rail superseded it.
    NotFound,
    Expired,
    /// The account's link version on the rail is not the one the request
    /// was made against: the link changed since the app showed it.
    VersionConflict,
    /// A link already waits for the pass, or an unlink is already recorded.
    ChangePending,
    /// An unlink with no active link.
    NotLinked,
}

/// One account's link state on one rail, for the link routes.
#[derive(Clone, PartialEq, Eq)]
pub struct PayoutAccountLinkView {
    /// Counts every link the account was given on the rail and every unlink
    /// it confirmed: it moves on each change the person makes and on none
    /// the pass makes, so it says whether the recipient changed since an app
    /// read it. 0 with no history.
    pub version: i64,
    /// The newest link that is not closed: the waiting one if there is one,
    /// else the active or closing one.
    pub newest: Option<PayoutLink>,
    /// An unexpired request waits for a confirm.
    pub pending_request: bool,
}

impl std::fmt::Debug for PayoutAccountLinkView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PayoutAccountLinkView")
            .field("version", &self.version)
            .field("newest", &self.newest)
            .field("pending_request", &self.pending_request)
            .finish()
    }
}

// Hash-only logging (spec, "Hash-only logging"): these `Debug` impls print
// the rail, kinds and states, never an account id, its hash, a program, a
// handle or a recipient. `payout_debug_tests` pins it.
impl std::fmt::Debug for PayoutAccrualWrite {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PayoutAccrualWrite")
            .field("rail_id", &self.rail_id)
            .field("kind", &self.kind)
            .field("rate_version", &self.rate_version)
            .field("instrument_id", &self.instrument_id)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PayoutLinkWrite {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PayoutLinkWrite")
            .field("rail_id", &self.rail_id)
            .field("recipient_kind", &self.recipient_kind)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PayoutLineWrite {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PayoutLineWrite")
            .field("rail_id", &self.rail_id)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PayoutLine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PayoutLine")
            .field("rail_id", &self.rail_id)
            .field("state", &self.state)
            .field("attempts", &self.attempts)
            .field("last_error_label", &self.last_error_label)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PayoutLinkRequestWrite {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PayoutLinkRequestWrite")
            .field("rail_id", &self.rail_id)
            .field("action", &self.action)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PayoutLinkRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PayoutLinkRequest")
            .field("rail_id", &self.rail_id)
            .field("action", &self.action)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Whether a rate epoch was recorded at boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayoutRateEpochRecord {
    Recorded,
    /// The same `(rail, version)` with the same rate is already there.
    AlreadyRecorded,
    /// The `(rail, version)` is there with a different rate: a version names
    /// one rate forever, so the caller refuses to start.
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayoutRateEpoch {
    pub rate: RailRate,
    pub pricing_label: Option<String>,
    pub observed_at: chrono::DateTime<chrono::Utc>,
}

/// A rail's kill switch: the latest halt or resume row, or not halted when
/// there is none.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PayoutControl {
    pub halted: bool,
    pub label: Option<String>,
    pub changed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The account state every rail shares (payout spec section 4, "Holds", and
/// section 6): whether the account is open, and whether a credit hold is
/// open on it or on any principal linked to it. Each rail checks it itself,
/// before every send, and holds only its own sends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayoutAccountStanding {
    /// The account row exists in this tenant.
    pub exists: bool,
    /// `trace_accounts.closed_at` is set.
    pub closed: bool,
    /// An open `trace_credit_holds` row names the account's `account:` key
    /// or a principal actively linked to it.
    pub credit_held: bool,
}

/// An account whose row is gone or closed.
pub const PAYOUT_ACCOUNT_CLOSED: &str = "payout_account_closed";
/// An open credit hold on the account or a principal linked to it.
pub const PAYOUT_ACCOUNT_CREDIT_HOLD: &str = "payout_account_credit_hold";

impl PayoutAccountStanding {
    /// The shared hold label, if any. A missing account row holds like a
    /// closed one: nothing is sent to an account this tenant does not have.
    pub fn hold_label(&self) -> Option<&'static str> {
        if !self.exists || self.closed {
            Some(PAYOUT_ACCOUNT_CLOSED)
        } else if self.credit_held {
            Some(PAYOUT_ACCOUNT_CREDIT_HOLD)
        } else {
            None
        }
    }
}

pub use postgres_payout::{
    PayoutPassLock, account_id_hash as payout_account_id_hash, link_id_hash as payout_link_id_hash,
};

/// One open hold on a rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayoutHold {
    pub account_id: Uuid,
    pub label: String,
}

/// The payout rail store. Implemented for PostgreSQL in `postgres_payout`.
#[async_trait]
pub trait PayoutStore: Send + Sync {
    // -- tenant-scoped ------------------------------------------------------

    /// Inserts each accrual once (`ON CONFLICT DO NOTHING` on its credit or
    /// reversal key); returns how many were new. Never re-prices a row.
    async fn insert_payout_accruals(
        &self,
        tenant: &str,
        accruals: &[PayoutAccrualWrite],
    ) -> Result<u64, DatabaseError>;

    /// Sets the owner of every unowned accrual on `rail` keyed to
    /// `credit_account_hash`, once, with one audit row when any was claimed.
    /// Per rail: one rail's accrual never writes another rail's rows.
    async fn claim_payout_accruals(
        &self,
        tenant: &str,
        rail: &RailId,
        credit_account_hash: &str,
        account: Uuid,
        actor_ref_hash: &str,
    ) -> Result<u64, DatabaseError>;

    /// The account each of `principals` is actively linked to, for
    /// accrual. Reads the shared ownership view only: unlike
    /// [`Database::resolve_principals_to_accounts`] it never upserts the
    /// tenant row, so accrual writes nothing outside the payout tables.
    async fn payout_principal_accounts(
        &self,
        tenant: &str,
        principals: &[String],
    ) -> Result<std::collections::HashMap<String, Uuid>, DatabaseError>;

    /// The database clock (`clock_timestamp()`), the clock rate epochs'
    /// `observed_at` is taken from.
    async fn payout_database_now(&self) -> Result<chrono::DateTime<chrono::Utc>, DatabaseError>;

    /// Every accrual on `rail` in this tenant, oldest first.
    async fn payout_accruals(
        &self,
        tenant: &str,
        rail: &RailId,
    ) -> Result<Vec<PayoutAccrual>, DatabaseError>;

    /// What `account` is owed on `rail`, from its accruals. Refuses a sum
    /// above `u128::MAX` rather than wrap.
    async fn payout_owed(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
    ) -> Result<PayoutOwed, DatabaseError>;

    /// The sum of `final_paid` over the account's closed links on `rail`.
    async fn payout_closed_links_final_paid(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
    ) -> Result<u128, DatabaseError>;

    /// The account's `active` link on `rail`.
    async fn payout_live_link(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
    ) -> Result<Option<PayoutLink>, DatabaseError>;

    /// The account-agnostic read the pass needs: every link on `rail` in
    /// this tenant whose state is one of `states`, by account then age.
    async fn payout_links(
        &self,
        tenant: &str,
        rail: &RailId,
        states: &[PayoutLinkState],
    ) -> Result<Vec<PayoutLink>, DatabaseError>;

    /// Records a confirmed unlink on an `active` link (spec section 5,
    /// "Unlink"), with an audit row; the pass then closes it. False, with
    /// nothing changed, when the link is not active or is already marked.
    async fn mark_payout_link_unlink_requested(
        &self,
        tenant: &str,
        link_id: Uuid,
        actor_ref_hash: &str,
    ) -> Result<bool, DatabaseError>;

    /// Marks an `active` or `closing` link's recipient unavailable (the
    /// rail answered that the recipient is gone), with an audit row. A mark,
    /// not a state: the link keeps its slot until it is closed. False, with
    /// nothing changed, when the link is in neither state or already marked.
    async fn mark_payout_link_recipient_unavailable(
        &self,
        tenant: &str,
        link_id: Uuid,
        actor_ref_hash: &str,
        label: Option<&str>,
    ) -> Result<bool, DatabaseError>;

    /// Inserts a `waiting` or `active` link and its audit row together.
    async fn insert_payout_link(
        &self,
        tenant: &str,
        link: &PayoutLinkWrite,
    ) -> Result<PayoutLinkInsert, DatabaseError>;

    /// Moves a link `waiting` to `active`, or `active` to `closing`, and
    /// writes `action`'s audit row, in one transaction. `action` must fit
    /// the move ([`PayoutAuditAction::fits_link_transition`]); any other
    /// pair is an error. A waiting link cannot go active while the account
    /// has an active or closing link on the rail (`LiveLinkTaken`); closing
    /// to closed is [`Self::close_payout_link`].
    #[allow(clippy::too_many_arguments)]
    async fn transition_payout_link(
        &self,
        tenant: &str,
        link_id: Uuid,
        from: PayoutLinkState,
        to: PayoutLinkState,
        action: PayoutAuditAction,
        actor_ref_hash: &str,
        label: Option<&str>,
    ) -> Result<PayoutLinkTransition, DatabaseError>;

    /// Closes a `closing` link with its program total `final_paid` and, when
    /// `successor` names the same account's `waiting` link on the same rail,
    /// activates it, in one transaction with an audit row for each (spec
    /// section 5, "Closing"). Refuses, changing nothing, while the closing
    /// link has a `pending` line. Concurrent writers are excluded by the
    /// rail pass's single-flight lock; the V132 link trigger refuses a close
    /// over a pending line as a backstop.
    #[allow(clippy::too_many_arguments)]
    async fn close_payout_link(
        &self,
        tenant: &str,
        link_id: Uuid,
        final_paid: AtomicUnits,
        successor: Option<Uuid>,
        actor_ref_hash: &str,
        label: Option<&str>,
    ) -> Result<PayoutLinkClose, DatabaseError>;

    /// Writes a `pending` line, only onto the account's `active` link on the
    /// line's rail.
    async fn insert_pending_payout_line(
        &self,
        tenant: &str,
        line: &PayoutLineWrite,
    ) -> Result<PendingPayoutLineInsert, DatabaseError>;

    async fn pending_payout_line_for_link(
        &self,
        tenant: &str,
        link_id: Uuid,
    ) -> Result<Option<PayoutLine>, DatabaseError>;

    async fn resolve_payout_line(
        &self,
        tenant: &str,
        line_id: Uuid,
        resolution: &PayoutLineResolution,
    ) -> Result<PayoutLineResolve, DatabaseError>;

    /// Whether a line on `rail` in this tenant already holds `receipt_hash`,
    /// so recovery can skip a record that answers another line.
    ///
    /// This is a hint, not the authority. It sees only this tenant (forced
    /// RLS), while `trace_payout_lines_one_receipt` is unique on `(rail_id,
    /// receipt_hash)` across every tenant. A record already claimed by a line
    /// in another tenant reads `false` here and surfaces only when the
    /// resolve is refused with [`PayoutLineResolve::ReceiptAlreadyClaimed`],
    /// which a caller treats the same way: skip that record and keep paging,
    /// leaving the line pending. (On `near_ai` each link has its own program,
    /// so a record from another tenant's program cannot match.)
    async fn payout_receipt_is_claimed(
        &self,
        tenant: &str,
        rail: &RailId,
        receipt_hash: &str,
    ) -> Result<bool, DatabaseError>;

    /// Opens a hold with an audit row; false if that hold is already open.
    async fn open_payout_hold(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
        label: &str,
        actor_ref_hash: &str,
    ) -> Result<bool, DatabaseError>;

    /// Records, once per `(rail, reversal_event_id)`, accrual's decision to
    /// hold `account` for a reversal it cannot price, and opens that hold
    /// (with its audit row) only when the decision is new. A later run
    /// finds the decision and leaves the hold as it is, so an operator's
    /// clear stands. True if a hold was opened.
    async fn open_payout_reversal_hold(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
        reversal_event_id: Uuid,
        label: &str,
        actor_ref_hash: &str,
    ) -> Result<bool, DatabaseError>;

    /// Clears an open hold with an audit row; false if none was open.
    async fn clear_payout_hold(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
        label: &str,
        actor_ref_hash: &str,
    ) -> Result<bool, DatabaseError>;

    async fn open_payout_hold_labels(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
    ) -> Result<Vec<String>, DatabaseError>;

    /// Every open hold in `tenant` on `rail`, by account and label: the
    /// operator's view before clearing one.
    async fn open_payout_holds(
        &self,
        tenant: &str,
        rail: &RailId,
    ) -> Result<Vec<PayoutHold>, DatabaseError>;

    /// Records a request with its audit row.
    async fn insert_payout_link_request(
        &self,
        tenant: &str,
        request: &PayoutLinkRequestWrite,
    ) -> Result<PayoutLinkRequestInsert, DatabaseError>;

    /// Takes (deletes and returns) the account's request on `rail` with that
    /// handle hash, so it can be confirmed at most once.
    async fn take_payout_link_request(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
        handle_hash: &str,
    ) -> Result<Option<PayoutLinkRequest>, DatabaseError>;

    /// Deletes this tenant's requests whose `expires_at` has passed by the
    /// database clock; returns how many. Requests are audited when made, so
    /// a discard writes no audit row.
    async fn purge_expired_payout_link_requests(&self, tenant: &str) -> Result<u64, DatabaseError>;

    /// The account's version, newest live link and pending-request flag on
    /// `rail` ([`PayoutAccountLinkView`]).
    async fn payout_account_link_view(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
    ) -> Result<PayoutAccountLinkView, DatabaseError>;

    /// The account's NEAR AI login anchors (`trace_near_account_anchors`
    /// rows with `identity_source = 'near_ai_login'`), which a `near_ai`
    /// proof's subject must match. Empty for an account with none.
    async fn payout_account_login_anchors(
        &self,
        tenant: &str,
        account: Uuid,
    ) -> Result<Vec<String>, DatabaseError>;

    /// Takes the account's request on `rail` with that handle hash and, in
    /// the same transaction, writes what it asks for: a `waiting` link
    /// carrying the request's sealed recipient, or the unlink mark on the
    /// active link, each with its audit rows. Serialized per account and
    /// rail. Refuses (consuming the request) when it expired, a newer
    /// request superseded it, or the account's link version is no longer
    /// the request's `expected_current_version`.
    async fn confirm_payout_link_request(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
        handle_hash: &str,
        confirm: &PayoutLinkConfirmWrite,
    ) -> Result<PayoutLinkConfirm, DatabaseError>;

    /// The audit actions on `rail` in this tenant, oldest first.
    async fn payout_audit_actions(
        &self,
        tenant: &str,
        rail: &RailId,
    ) -> Result<Vec<PayoutAuditAction>, DatabaseError>;

    // -- deployment-wide ----------------------------------------------------

    /// Records `(rail, rate.version)` once, under `trace_payout_config_writer`,
    /// with `observed_at` from the database clock.
    async fn record_payout_rate_epoch(
        &self,
        rail: &RailId,
        rate: &RailRate,
        pricing_label: Option<&str>,
    ) -> Result<PayoutRateEpochRecord, DatabaseError>;

    /// The rail's epochs, oldest observation first.
    async fn payout_rate_epochs(
        &self,
        rail: &RailId,
    ) -> Result<Vec<PayoutRateEpoch>, DatabaseError>;

    async fn payout_control(&self, rail: &RailId) -> Result<PayoutControl, DatabaseError>;

    /// Appends a halt or resume row under `trace_payout_config_writer`.
    async fn set_payout_control(
        &self,
        rail: &RailId,
        halted: bool,
        label: Option<&str>,
        actor_ref_hash: &str,
    ) -> Result<(), DatabaseError>;

    /// The tenants with a `waiting`, `active` or `closing` link on `rail`,
    /// read under `trace_payout_enumerator`, which sees only tenant, rail and
    /// state. `waiting` is included because every confirmed link is written
    /// waiting and only the pass activates it: a tenant whose only link is
    /// waiting (every first link, or a successor left after a crash) must
    /// still get a pass.
    async fn tenants_with_live_payout_links(
        &self,
        rail: &RailId,
    ) -> Result<Vec<String>, DatabaseError>;

    /// The shared account state a rail checks before every send (spec
    /// section 4, "Holds"). Reads only.
    async fn payout_account_standing(
        &self,
        tenant: &str,
        account: Uuid,
    ) -> Result<PayoutAccountStanding, DatabaseError>;

    /// The sum of `delta` over the link's `confirmed` lines: what the
    /// commons has confirmed landing on the link's program.
    async fn payout_link_confirmed_total(
        &self,
        tenant: &str,
        link_id: Uuid,
    ) -> Result<u128, DatabaseError>;

    /// Takes the rail's single-flight pass lock (spec section 4, "One pass,
    /// per rail", step 0): a session-level
    /// `pg_try_advisory_lock(hashtextextended('payout-pass-v1:' || rail, 0))`
    /// on a connection pinned until [`PayoutPassLock::release`]. `None` when
    /// another pass of the same rail holds it. Each rail has its own key, so
    /// passes on two rails never wait on each other.
    async fn try_acquire_payout_pass_lock(
        &self,
        rail: &RailId,
    ) -> Result<Option<PayoutPassLock>, DatabaseError>;
}

#[cfg(test)]
mod payout_debug_tests {
    //! Hash-only logging: a `{:?}` of any payout row type must print neither
    //! the account id nor the program, so a `tracing::warn!(?line)` cannot
    //! leak either.

    use super::*;

    const ACCOUNT: &str = "6f1c2d3e-4b5a-4c6d-8e7f-0a1b2c3d4e5f";
    const PROGRAM: &str = "tc1-programsecretvalue";
    const ACCOUNT_HASH: &str =
        "sha256:abababababababababababababababababababababababababababababababab";
    const HANDLE_HASH: &str =
        "sha256:cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";

    fn account() -> Uuid {
        Uuid::parse_str(ACCOUNT).unwrap()
    }

    fn rail() -> RailId {
        RailId::new("near_ai").unwrap()
    }

    fn sealed() -> SealedRecipient {
        SealedRecipient {
            sealed: serde_json::json!({"ciphertext": "sealedvalue"}),
            recipient_index: "sha256:00".into(),
            recipient_key_ref: "key".into(),
            index_pepper_ref: "pepper".into(),
            subject_index: "sha256:11".into(),
            recipient_ref: None,
        }
    }

    fn assert_redacted(rendered: String) {
        for secret in [ACCOUNT, PROGRAM, ACCOUNT_HASH, HANDLE_HASH, "sealedvalue"] {
            assert!(
                !rendered.contains(secret),
                "Debug printed {secret}: {rendered}"
            );
        }
    }

    #[test]
    fn payout_row_types_redact_account_and_program_in_debug() {
        let kind = RecipientKind::new("test_recipient").unwrap();
        let expires = chrono::Utc::now();
        assert_redacted(format!(
            "{:?}",
            PayoutAccrualWrite {
                rail_id: rail(),
                kind: PayoutAccrualKind::Credit,
                settlement_batch_id: Uuid::nil(),
                credit_account_hash: ACCOUNT_HASH.into(),
                reversal_event_id: None,
                account_id: Some(account()),
                award_units: AtomicUnits::from_raw(1),
                rail_units: AtomicUnits::from_raw(1),
                rate_version: "v1".into(),
                instrument_id: InstrumentId::new("test_units").unwrap(),
            }
        ));
        assert_redacted(format!(
            "{:?}",
            PayoutLinkWrite {
                account_id: account(),
                rail_id: rail(),
                link_id: Uuid::nil(),
                recipient_kind: kind.clone(),
                recipient: sealed(),
                program: PROGRAM.into(),
                state: PayoutLinkState::Waiting,
                actor_ref_hash: ACCOUNT_HASH.into(),
            }
        ));
        assert_redacted(format!(
            "{:?}",
            PayoutLineWrite {
                line_id: Uuid::nil(),
                link_id: Uuid::nil(),
                account_id: account(),
                rail_id: rail(),
                target_total: AtomicUnits::from_raw(1),
                delta: AtomicUnits::from_raw(1),
                owed_through_accrual_count: 1,
            }
        ));
        assert_redacted(format!(
            "{:?}",
            PayoutLine {
                line_id: Uuid::nil(),
                link_id: Uuid::nil(),
                account_id: account(),
                rail_id: rail(),
                target_total: AtomicUnits::from_raw(1),
                delta: AtomicUnits::from_raw(1),
                state: PayoutLineState::Pending,
                receipt_hash: None,
                attempts: 0,
                last_error_label: None,
            }
        ));
        assert_redacted(format!(
            "{:?}",
            PayoutLinkRequestWrite {
                account_id: account(),
                rail_id: rail(),
                idempotency_key: Uuid::nil(),
                handle_hash: HANDLE_HASH.into(),
                request_digest: HANDLE_HASH.into(),
                action: PayoutLinkAction::Link,
                expected_current_version: None,
                recipient: Some((kind.clone(), sealed())),
                expires_at: expires,
                actor_ref_hash: ACCOUNT_HASH.into(),
            }
        ));
        assert_redacted(format!(
            "{:?}",
            PayoutLinkRequest {
                account_id: account(),
                rail_id: rail(),
                idempotency_key: Uuid::nil(),
                action: PayoutLinkAction::Link,
                expected_current_version: None,
                recipient: Some((kind, sealed())),
                expires_at: expires,
            }
        ));
    }
}
