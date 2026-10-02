// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tenant routing, receipt ownership, containment, activation, and rollback.
//!
//! A committed row assigns each tenant's new receipts to the legacy path or
//! the pipeline, or stops them (containment). Each receipt has one permanent
//! owner; a retry goes to that owner whatever the routing row says. An
//! activation or a rollback selects a qualified bundle for later runs only.
//! Timestamps are audit metadata and select nothing.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use deadpool_postgres::Transaction;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::error::DatabaseError;
use crate::versioned_pipeline::{
    PIPELINE_ROUTING_LOCK_SEED, pipeline_routing_lock, sha256_prefixed,
};
use crate::versioned_pipeline_product::PipelineOperationalSummary;
use crate::versioned_pipeline_qualification::is_safe_label;

pub const PIPELINE_RECEIPT_INTAKE_CONTAINED_LABEL: &str = "pipeline_receipt_intake_contained";
pub const PIPELINE_TENANT_NOT_SERVED_LABEL: &str = "pipeline_tenant_not_served";
pub const PIPELINE_ROUTING_UNAVAILABLE_LABEL: &str = "pipeline_routing_unavailable";
pub const ACTIVATION_ACTOR_INVALID_LABEL: &str = "activation_actor_invalid";
pub const ACTIVATION_STATE_INVALID_LABEL: &str = "activation_state_invalid";
pub const ACTIVATION_READINESS_FAILED_LABEL: &str = "activation_readiness_failed";
pub const EARLIER_QUALIFIED_BUNDLE_REQUIRED_LABEL: &str = "earlier_qualified_bundle_required";
pub const ACTIVATION_MAX_ERROR_COUNT: u64 = 0;
pub const ACTIVATION_MAX_WORK_AGE_SECONDS: u64 = 300;

/// How long a readiness evaluation stays usable for an activation.
const ACTIVATION_READINESS_MAX_AGE_MINUTES: i64 = 15;

fn is_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// The tenant's routing: where its new receipts go.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingState {
    Legacy,
    Pipeline,
    Contained,
}

impl RoutingState {
    fn as_db(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Pipeline => "pipeline",
            Self::Contained => "contained",
        }
    }

    pub(crate) fn from_db(value: &str) -> Result<Self, DatabaseError> {
        match value {
            "legacy" => Ok(Self::Legacy),
            "pipeline" => Ok(Self::Pipeline),
            "contained" => Ok(Self::Contained),
            _ => Err(DatabaseError::Serialization(
                "pipeline_routing_state_invalid".to_string(),
            )),
        }
    }
}

/// The implementation that owns a receipt for good.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptOwner {
    Legacy,
    Pipeline,
}

impl ReceiptOwner {
    fn as_db(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Pipeline => "pipeline",
        }
    }

    fn from_db(value: &str) -> Result<Self, DatabaseError> {
        match value {
            "legacy" => Ok(Self::Legacy),
            "pipeline" => Ok(Self::Pipeline),
            _ => Err(DatabaseError::Serialization(
                "pipeline_receipt_owner_invalid".to_string(),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivationAction {
    Activate,
    Rollback,
    Contain,
    Deactivate,
}

impl ActivationAction {
    fn as_db(self) -> &'static str {
        match self {
            Self::Activate => "activate",
            Self::Rollback => "rollback",
            Self::Contain => "contain",
            Self::Deactivate => "deactivate",
        }
    }

    fn from_db(value: &str) -> Result<Self, DatabaseError> {
        match value {
            "activate" => Ok(Self::Activate),
            "rollback" => Ok(Self::Rollback),
            "contain" => Ok(Self::Contain),
            "deactivate" => Ok(Self::Deactivate),
            _ => Err(DatabaseError::Serialization(
                "pipeline_activation_action_invalid".to_string(),
            )),
        }
    }
}

/// Where a tenant's new receipt goes, from its routing row and its scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewReceiptRoute {
    Legacy,
    Pipeline,
    Contained,
    NotServed,
}

/// The route of a new receipt. A committed row decides first: `Pipeline`
/// serves only a tenant on the receipts list, `Contained` stops the receipt,
/// `Legacy` keeps it on the legacy path. Only a tenant with no row falls to
/// the scope, and only when the deployment allows unqualified routing.
pub fn decide_new_receipt_route(
    routing: Option<RoutingState>,
    on_receipts_list: bool,
    unqualified_routing_allowed: bool,
) -> NewReceiptRoute {
    match routing {
        Some(RoutingState::Pipeline) if on_receipts_list => NewReceiptRoute::Pipeline,
        Some(RoutingState::Pipeline) => NewReceiptRoute::NotServed,
        Some(RoutingState::Contained) => NewReceiptRoute::Contained,
        Some(RoutingState::Legacy) => NewReceiptRoute::Legacy,
        None if on_receipts_list && unqualified_routing_allowed => NewReceiptRoute::Pipeline,
        None => NewReceiptRoute::Legacy,
    }
}

/// A tenant's committed routing row. The selected bundle is not here: it
/// stays in `pipeline_active_bundles`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TenantRouting {
    pub routing_state: RoutingState,
    pub activation_record_id: Uuid,
    pub actor_principal_ref: String,
    pub reason_code: String,
    pub evidence_hash: String,
    pub recorded_at: DateTime<Utc>,
}

/// One routing change. `previous_state` is `None` for a tenant that had no
/// routing row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivationEvent {
    pub event_id: Uuid,
    pub action: ActivationAction,
    pub previous_state: Option<RoutingState>,
    pub resulting_state: RoutingState,
    pub previous_bundle_id: Option<String>,
    pub resulting_bundle_id: Option<String>,
    pub actor_principal_ref: String,
    pub reason_code: String,
    pub evidence_hash: String,
    pub recorded_at: DateTime<Utc>,
}

/// A receipt's permanent owner. A pipeline-owned receipt carries its run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceiptOwnership {
    pub owner: ReceiptOwner,
    pub submission_id: Uuid,
    pub run_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivationReadiness {
    pub readiness_ok: bool,
    pub error_count: u64,
    pub max_work_age_seconds: u64,
    pub credit_reconciled: bool,
    pub index_consistent: bool,
    pub invalidation_clear: bool,
    pub evaluated_at: DateTime<Utc>,
    pub evidence_hash: String,
}

impl ActivationReadiness {
    pub fn from_operational_summary(summary: &PipelineOperationalSummary) -> Self {
        // Only work that is still due counts for its age. A completed run, a
        // failed one, and a run that waits for a human review say nothing
        // about whether the pipeline keeps up.
        let max_work_age_seconds = summary
            .work
            .iter()
            .filter(|item| matches!(item.state.as_str(), "pending" | "retry" | "leased"))
            .map(|item| item.oldest_age_seconds)
            .max()
            .unwrap_or(0);
        let error_count = summary.retryable_error_count;
        let mut ready = Self {
            readiness_ok: summary.tenant_isolation_control_passed
                && summary.audit_immutability_control_passed,
            error_count,
            max_work_age_seconds,
            credit_reconciled: summary.held_credit_count == 0 && summary.delayed_credit_count == 0,
            index_consistent: summary.pending_index_command_count == 0
                && summary.failed_index_command_count == 0,
            invalidation_clear: summary.pending_invalidation_count == 0
                && summary.failed_invalidation_count == 0,
            evaluated_at: summary.generated_at,
            evidence_hash: String::new(),
        };
        ready.evidence_hash = activation_readiness_hash(&ready);
        ready
    }
}

fn activation_readiness_hash(ready: &ActivationReadiness) -> String {
    sha256_prefixed(
        format!(
            "trace_commons.pipeline_activation_readiness.v1\0{}\0{}\0{}\0{}\0{}\0{}",
            ready.readiness_ok,
            ready.error_count,
            ready.max_work_age_seconds,
            ready.credit_reconciled,
            ready.index_consistent,
            ready.invalidation_clear
        )
        .as_bytes(),
    )
}

pub fn evaluate_activation_readiness(
    ready: &ActivationReadiness,
    now: DateTime<Utc>,
) -> Result<(), String> {
    if !is_sha256(&ready.evidence_hash)
        || ready.evidence_hash != activation_readiness_hash(ready)
        || ready.evaluated_at > now
        || now - ready.evaluated_at > Duration::minutes(ACTIVATION_READINESS_MAX_AGE_MINUTES)
    {
        return Err(ACTIVATION_READINESS_FAILED_LABEL.to_string());
    }
    if ready.readiness_ok
        && ready.error_count == ACTIVATION_MAX_ERROR_COUNT
        && ready.max_work_age_seconds <= ACTIVATION_MAX_WORK_AGE_SECONDS
        && ready.credit_reconciled
        && ready.index_consistent
        && ready.invalidation_clear
    {
        Ok(())
    } else {
        Err(ACTIVATION_READINESS_FAILED_LABEL.to_string())
    }
}

fn validate_actor(actor_principal_ref: &str, reason_code: &str) -> Result<(), DatabaseError> {
    let actor_ok = !actor_principal_ref.is_empty()
        && actor_principal_ref.len() <= 160
        && actor_principal_ref
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'.' | b'-'));
    if !actor_ok || !is_safe_label(reason_code) {
        return Err(DatabaseError::Constraint(
            ACTIVATION_ACTOR_INVALID_LABEL.to_string(),
        ));
    }
    Ok(())
}

/// Takes the tenant's routing lock exclusively for the rest of `tx`. A
/// receipt's staging and commit transactions take it shared
/// (`PipelineService`), so a routing change waits for the receipts in those
/// transactions and no later one misses it.
async fn lock_routing(tx: &Transaction<'_>, tenant_id: &str) -> Result<(), DatabaseError> {
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended($1, $2))",
        &[
            &pipeline_routing_lock(tenant_id),
            &PIPELINE_ROUTING_LOCK_SEED,
        ],
    )
    .await?;
    Ok(())
}

/// Writes the routing row and its event in the caller's transaction. The
/// caller holds the routing lock (`lock_routing`) and has read whatever it
/// checks before it writes, so the previous state read here is the state the
/// caller checked. The row has no bundle column; the two bundle ids belong to
/// the event only.
///
/// The row's and the event's `recorded_at` are the database clock at the
/// write (`clock_timestamp()`), not the transaction's start. The lock orders
/// writers by when they get it, and a transaction that waited for it began
/// earlier than the one it waited for, so the start time would put a later
/// change before an earlier one in the newest-first list.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn write_routing_in(
    tx: &Transaction<'_>,
    tenant_id: &str,
    resulting: RoutingState,
    action: ActivationAction,
    previous_bundle_id: Option<&str>,
    resulting_bundle_id: Option<&str>,
    actor_principal_ref: &str,
    reason_code: &str,
    evidence_hash: &str,
) -> Result<TenantRouting, DatabaseError> {
    tx.execute(
        "INSERT INTO trace_tenants (tenant_id) VALUES ($1)
         ON CONFLICT (tenant_id) DO NOTHING",
        &[&tenant_id],
    )
    .await?;
    let previous_state = match tx
        .query_opt(
            "SELECT routing_state FROM pipeline_tenant_routing WHERE tenant_id = $1",
            &[&tenant_id],
        )
        .await?
    {
        Some(row) => RoutingState::from_db(row.get::<_, String>("routing_state").as_str())?.as_db(),
        None => "unselected",
    };
    let activation_record_id = Uuid::new_v4();
    let row = tx
        .query_one(
            "INSERT INTO pipeline_tenant_routing (
                tenant_id, routing_state, activation_record_id,
                actor_principal_ref, reason_code, evidence_hash, recorded_at
             ) VALUES ($1, $2, $3, $4, $5, $6, clock_timestamp())
             ON CONFLICT (tenant_id) DO UPDATE
             SET routing_state = EXCLUDED.routing_state,
                 activation_record_id = EXCLUDED.activation_record_id,
                 actor_principal_ref = EXCLUDED.actor_principal_ref,
                 reason_code = EXCLUDED.reason_code,
                 evidence_hash = EXCLUDED.evidence_hash,
                 recorded_at = EXCLUDED.recorded_at
             RETURNING routing_state, activation_record_id, actor_principal_ref,
                       reason_code, evidence_hash, recorded_at",
            &[
                &tenant_id,
                &resulting.as_db(),
                &activation_record_id,
                &actor_principal_ref,
                &reason_code,
                &evidence_hash,
            ],
        )
        .await?;
    let routing = routing_from_row(&row)?;
    tx.execute(
        "INSERT INTO pipeline_activation_events (
            tenant_id, event_id, action, previous_state, resulting_state,
            previous_bundle_id, resulting_bundle_id, actor_principal_ref,
            reason_code, evidence_hash, recorded_at
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        &[
            &tenant_id,
            &Uuid::new_v4(),
            &action.as_db(),
            &previous_state,
            &resulting.as_db(),
            &previous_bundle_id,
            &resulting_bundle_id,
            &actor_principal_ref,
            &reason_code,
            &evidence_hash,
            &routing.recorded_at,
        ],
    )
    .await?;
    Ok(routing)
}

/// The tenant's routing, its history, and each receipt's owner.
#[derive(Clone)]
pub struct PipelineActivationStore {
    backend: Arc<PgBackend>,
}

impl PipelineActivationStore {
    pub fn new(backend: Arc<PgBackend>) -> Self {
        Self { backend }
    }

    async fn tenant_transaction<'a>(
        client: &'a mut deadpool_postgres::Client,
        tenant_id: &str,
    ) -> Result<Transaction<'a>, DatabaseError> {
        let tx = client.transaction().await?;
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant_id],
        )
        .await?;
        Ok(tx)
    }

    /// The tenant's committed routing row; `None` for a tenant that has none.
    pub async fn routing(&self, tenant_id: &str) -> Result<Option<TenantRouting>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT routing_state, activation_record_id, actor_principal_ref,
                        reason_code, evidence_hash, recorded_at
                   FROM pipeline_tenant_routing
                  WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(routing_from_row).transpose()
    }

    /// The tenant's newest `limit` routing changes, newest first.
    pub async fn events(
        &self,
        tenant_id: &str,
        limit: usize,
    ) -> Result<Vec<ActivationEvent>, DatabaseError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT event_id, action, previous_state, resulting_state,
                        previous_bundle_id, resulting_bundle_id, actor_principal_ref,
                        reason_code, evidence_hash, recorded_at
                   FROM pipeline_activation_events
                  WHERE tenant_id = $1
                  ORDER BY recorded_at DESC, event_id
                  LIMIT $2",
                &[&tenant_id, &limit],
            )
            .await?;
        tx.commit().await?;
        rows.iter().map(event_from_row).collect()
    }

    /// The permanent owner of a receipt; `None` for a submission id no path
    /// has claimed.
    pub async fn ownership(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<Option<ReceiptOwnership>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT owner, submission_id, run_id
                   FROM pipeline_receipt_ownership
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &submission_id],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(ownership_from_row).transpose()
    }

    /// Claims a receipt for the legacy path before the legacy path's first
    /// write, and returns the owner the row records. A receipt that has an
    /// owner keeps it: the claim neither changes the row nor fails, so a
    /// retry of a legacy receipt claims it again, and a receipt the pipeline
    /// owns comes back as `Pipeline`.
    pub async fn claim_legacy_receipt(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<ReceiptOwner, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        // The ownership row references the tenant. The legacy path has not
        // written for a tenant's first receipt yet, so the claim makes sure
        // the tenant exists, as `write_routing_in` does.
        tx.execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)
             ON CONFLICT (tenant_id) DO NOTHING",
            &[&tenant_id],
        )
        .await?;
        tx.execute(
            "INSERT INTO pipeline_receipt_ownership (tenant_id, submission_id, owner)
             VALUES ($1, $2, $3)
             ON CONFLICT (tenant_id, submission_id) DO NOTHING",
            &[&tenant_id, &submission_id, &ReceiptOwner::Legacy.as_db()],
        )
        .await?;
        let row = tx
            .query_one(
                "SELECT owner FROM pipeline_receipt_ownership
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &submission_id],
            )
            .await?;
        tx.commit().await?;
        ReceiptOwner::from_db(row.get::<_, String>("owner").as_str())
    }

    /// Stops the tenant's new receipts. Contained is the state of a tenant
    /// whose intake is held, for a first rollout or an incident; it changes
    /// no bundle and no receipt that already has an owner.
    pub async fn contain(
        &self,
        tenant_id: &str,
        actor_principal_ref: &str,
        reason_code: &str,
    ) -> Result<TenantRouting, DatabaseError> {
        validate_actor(actor_principal_ref, reason_code)?;
        self.change_routing(
            tenant_id,
            actor_principal_ref,
            reason_code,
            ActivationAction::Contain,
            RoutingState::Contained,
        )
        .await
    }

    /// Returns a tenant that has a routing row to the legacy path. A tenant
    /// that has no row, or is already `Legacy`, has nothing to deactivate.
    pub async fn deactivate(
        &self,
        tenant_id: &str,
        actor_principal_ref: &str,
        reason_code: &str,
    ) -> Result<TenantRouting, DatabaseError> {
        validate_actor(actor_principal_ref, reason_code)?;
        self.change_routing(
            tenant_id,
            actor_principal_ref,
            reason_code,
            ActivationAction::Deactivate,
            RoutingState::Legacy,
        )
        .await
    }

    /// `contain` and `deactivate`: neither changes the bundle, so the event's
    /// two bundle columns hold the one active bundle.
    async fn change_routing(
        &self,
        tenant_id: &str,
        actor_principal_ref: &str,
        reason_code: &str,
        action: ActivationAction,
        resulting: RoutingState,
    ) -> Result<TenantRouting, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        lock_routing(&tx, tenant_id).await?;
        let active_bundle_id: Option<String> = tx
            .query_opt(
                "SELECT bundle_id FROM pipeline_active_bundles WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?
            .map(|row| row.get("bundle_id"));
        if action == ActivationAction::Deactivate {
            let current = tx
                .query_opt(
                    "SELECT routing_state FROM pipeline_tenant_routing WHERE tenant_id = $1",
                    &[&tenant_id],
                )
                .await?
                .map(|row| RoutingState::from_db(row.get::<_, String>("routing_state").as_str()))
                .transpose()?;
            if matches!(current, None | Some(RoutingState::Legacy)) {
                return Err(DatabaseError::Constraint(
                    ACTIVATION_STATE_INVALID_LABEL.to_string(),
                ));
            }
        }
        let evidence_hash = sha256_prefixed(
            format!(
                "trace_commons.pipeline_{}.v1\0{tenant_id}\0{reason_code}",
                action.as_db()
            )
            .as_bytes(),
        );
        let routing = write_routing_in(
            &tx,
            tenant_id,
            resulting,
            action,
            active_bundle_id.as_deref(),
            active_bundle_id.as_deref(),
            actor_principal_ref,
            reason_code,
            &evidence_hash,
        )
        .await?;
        tx.commit().await?;
        Ok(routing)
    }
}

fn routing_from_row(row: &tokio_postgres::Row) -> Result<TenantRouting, DatabaseError> {
    Ok(TenantRouting {
        routing_state: RoutingState::from_db(row.get::<_, String>("routing_state").as_str())?,
        activation_record_id: row.get("activation_record_id"),
        actor_principal_ref: row.get("actor_principal_ref"),
        reason_code: row.get("reason_code"),
        evidence_hash: row.get("evidence_hash"),
        recorded_at: row.get("recorded_at"),
    })
}

fn event_from_row(row: &tokio_postgres::Row) -> Result<ActivationEvent, DatabaseError> {
    // `unselected` is the stored form of "the tenant had no routing row".
    let previous_state = match row.get::<_, String>("previous_state").as_str() {
        "unselected" => None,
        state => Some(RoutingState::from_db(state)?),
    };
    Ok(ActivationEvent {
        event_id: row.get("event_id"),
        action: ActivationAction::from_db(row.get::<_, String>("action").as_str())?,
        previous_state,
        resulting_state: RoutingState::from_db(row.get::<_, String>("resulting_state").as_str())?,
        previous_bundle_id: row.get("previous_bundle_id"),
        resulting_bundle_id: row.get("resulting_bundle_id"),
        actor_principal_ref: row.get("actor_principal_ref"),
        reason_code: row.get("reason_code"),
        evidence_hash: row.get("evidence_hash"),
        recorded_at: row.get("recorded_at"),
    })
}

fn ownership_from_row(row: &tokio_postgres::Row) -> Result<ReceiptOwnership, DatabaseError> {
    Ok(ReceiptOwnership {
        owner: ReceiptOwner::from_db(row.get::<_, String>("owner").as_str())?,
        submission_id: row.get("submission_id"),
        run_id: row.get("run_id"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versioned_pipeline_product::PipelineWorkSummary;

    /// A readiness that passes every check, with its hash computed.
    fn passing_readiness(now: DateTime<Utc>) -> ActivationReadiness {
        let mut ready = ActivationReadiness {
            readiness_ok: true,
            error_count: 0,
            max_work_age_seconds: 0,
            credit_reconciled: true,
            index_consistent: true,
            invalidation_clear: true,
            evaluated_at: now,
            evidence_hash: String::new(),
        };
        ready.evidence_hash = activation_readiness_hash(&ready);
        ready
    }

    #[test]
    fn the_route_of_a_new_receipt_follows_the_row_then_the_scope() {
        for on_list in [true, false] {
            for unqualified in [true, false] {
                assert_eq!(
                    decide_new_receipt_route(Some(RoutingState::Contained), on_list, unqualified),
                    NewReceiptRoute::Contained
                );
                assert_eq!(
                    decide_new_receipt_route(Some(RoutingState::Legacy), on_list, unqualified),
                    NewReceiptRoute::Legacy
                );
                assert_eq!(
                    decide_new_receipt_route(Some(RoutingState::Pipeline), on_list, unqualified),
                    if on_list {
                        NewReceiptRoute::Pipeline
                    } else {
                        NewReceiptRoute::NotServed
                    }
                );
            }
        }
        assert_eq!(
            decide_new_receipt_route(None, true, true),
            NewReceiptRoute::Pipeline
        );
        assert_eq!(
            decide_new_receipt_route(None, true, false),
            NewReceiptRoute::Legacy
        );
        assert_eq!(
            decide_new_receipt_route(None, false, true),
            NewReceiptRoute::Legacy
        );
        assert_eq!(
            decide_new_receipt_route(None, false, false),
            NewReceiptRoute::Legacy
        );
    }

    #[test]
    fn stale_or_failed_readiness_blocks_activation() {
        let now = Utc::now();
        let passing = passing_readiness(now);
        assert!(evaluate_activation_readiness(&passing, now).is_ok());

        // The thresholds are inclusive: a value exactly at a limit passes.
        let mut at_limit = passing.clone();
        at_limit.max_work_age_seconds = ACTIVATION_MAX_WORK_AGE_SECONDS;
        at_limit.evidence_hash = activation_readiness_hash(&at_limit);
        assert!(evaluate_activation_readiness(&at_limit, now).is_ok());
        assert!(evaluate_activation_readiness(&passing, now + Duration::minutes(15)).is_ok());

        let refused = Err(ACTIVATION_READINESS_FAILED_LABEL.to_string());

        // An evidence hash that was changed, or is not a hash, fails.
        let mut tampered = passing.clone();
        tampered.evidence_hash = sha256_prefixed(b"another readiness");
        assert_eq!(evaluate_activation_readiness(&tampered, now), refused);
        let mut not_a_hash = passing.clone();
        not_a_hash.evidence_hash = "sha256:not-a-hash".to_string();
        assert_eq!(evaluate_activation_readiness(&not_a_hash, now), refused);

        // A readiness evaluated 16 minutes ago fails; so does one evaluated
        // in the future.
        assert_eq!(
            evaluate_activation_readiness(&passing, now + Duration::minutes(16)),
            refused
        );
        let future = passing_readiness(now + Duration::minutes(1));
        assert_eq!(evaluate_activation_readiness(&future, now), refused);

        // Each failed check fails, with the hash recomputed so that only the
        // check itself can be the reason.
        for (name, break_check) in [
            (
                "readiness_ok",
                (|ready: &mut ActivationReadiness| ready.readiness_ok = false)
                    as fn(&mut ActivationReadiness),
            ),
            ("error_count", |ready| ready.error_count = 1),
            ("max_work_age_seconds", |ready| {
                ready.max_work_age_seconds = ACTIVATION_MAX_WORK_AGE_SECONDS + 1
            }),
            ("credit_reconciled", |ready| ready.credit_reconciled = false),
            ("index_consistent", |ready| ready.index_consistent = false),
            ("invalidation_clear", |ready| {
                ready.invalidation_clear = false
            }),
        ] {
            let mut incomplete = passing.clone();
            break_check(&mut incomplete);
            incomplete.evidence_hash = activation_readiness_hash(&incomplete);
            assert_eq!(
                evaluate_activation_readiness(&incomplete, now),
                refused,
                "{name} must block activation"
            );
        }
    }

    #[test]
    fn readiness_ignores_completed_and_terminal_history_age() {
        let now = Utc::now();
        let summary = PipelineOperationalSummary {
            generated_at: now,
            work: vec![
                PipelineWorkSummary {
                    phase: "complete".to_string(),
                    state: "complete".to_string(),
                    reason_label: None,
                    count: 20,
                    oldest_age_seconds: 86_400,
                },
                PipelineWorkSummary {
                    phase: "settle".to_string(),
                    state: "failed".to_string(),
                    reason_label: Some("settlement_failed".to_string()),
                    count: 3,
                    oldest_age_seconds: 43_200,
                },
                PipelineWorkSummary {
                    phase: "score".to_string(),
                    state: "pending".to_string(),
                    reason_label: None,
                    count: 1,
                    oldest_age_seconds: 12,
                },
            ],
            retryable_error_count: 0,
            terminal_error_count: 7,
            pending_index_command_count: 0,
            failed_index_command_count: 0,
            held_credit_count: 0,
            delayed_credit_count: 0,
            near_outbox_by_state: Default::default(),
            pending_invalidation_count: 0,
            failed_invalidation_count: 0,
            incomplete_export_count: 0,
            tenant_isolation_control_passed: true,
            audit_immutability_control_passed: true,
        };
        let ready = ActivationReadiness::from_operational_summary(&summary);
        assert_eq!(ready.max_work_age_seconds, 12);
        assert_eq!(ready.error_count, 0);
        assert!(evaluate_activation_readiness(&ready, now).is_ok());
    }

    #[test]
    fn an_actor_and_a_reason_are_validated() {
        let actor = format!("principal_sha256:{}", "a".repeat(64));
        assert!(validate_actor(&actor, "contain_first_rollout").is_ok());
        for (actor, reason) in [
            (actor.as_str(), "Bad Reason"),
            ("", "contain_first_rollout"),
            ("an actor with spaces", "contain_first_rollout"),
            (actor.as_str(), ""),
        ] {
            assert!(
                matches!(
                    validate_actor(actor, reason),
                    Err(DatabaseError::Constraint(label)) if label == ACTIVATION_ACTOR_INVALID_LABEL
                ),
                "actor {actor:?} with reason {reason:?} must be refused"
            );
        }
        let too_long = "a".repeat(161);
        assert!(matches!(
            validate_actor(&too_long, "contain_first_rollout"),
            Err(DatabaseError::Constraint(label)) if label == ACTIVATION_ACTOR_INVALID_LABEL
        ));
    }
}
