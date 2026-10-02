// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tenant routing, receipt ownership, containment, activation, and rollback.
//!
//! A committed row assigns each tenant's new receipts to the legacy path or
//! the pipeline, or stops them (containment). Each receipt has one permanent
//! owner; a retry goes to that owner whatever the routing row says. An
//! activation or a rollback selects a qualified bundle for later runs only.
//! Timestamps are audit metadata and select nothing.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use deadpool_postgres::Transaction;
use serde::{Deserialize, Serialize};
use tokio_postgres::IsolationLevel;
use tokio_postgres::types::ToSql;
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::error::DatabaseError;
use crate::versioned_pipeline::{
    PIPELINE_ROUTING_LOCK_SEED, pipeline_routing_lock, sha256_prefixed, validate_actor,
};
use crate::versioned_pipeline_product::PipelineOperationalSummary;

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

/// What the legacy path still owes one tenant for the receipts it already
/// took: a count for each kind of pending legacy work, and whether all of them
/// are zero. Labels, counts, a routing state, a time, and a hash: no tenant,
/// submission, principal, or trace text.
///
/// `pending` holds every one of the ten labels that `legacy_drain_report`
/// lists, a zero too, so a missing key never reads as "nothing owed". `evidence_hash` is the canonical hash
/// (`evidence_hash`) of `{"schema": "trace_commons.pipeline_legacy_drain.v1",
/// "pending": pending}`: the same counts give the same hash, and it names no
/// time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacyDrainReport {
    pub generated_at: DateTime<Utc>,
    /// The tenant's routing row at the same snapshot; `None` for a tenant
    /// that has none.
    pub routing_state: Option<RoutingState>,
    pub pending: BTreeMap<String, u64>,
    /// True only when every count is zero.
    pub drained: bool,
    pub evidence_hash: String,
}

const LEGACY_DRAIN_EVIDENCE_SCHEMA: &str = "trace_commons.pipeline_legacy_drain.v1";

/// What a count's statement takes after the tenant (`$1`): nothing, the gate
/// driver's attempt ceiling (`$2`), or the retention policies a legal hold
/// keeps (`$2`, a `text[]`).
#[derive(Clone, Copy)]
enum DrainParams {
    Tenant,
    TenantAndGateCeiling,
    TenantAndHeldRetentionPolicies,
}

/// One count of the drain report: its label, its statement, and its
/// parameters.
struct LegacyDrainCount {
    label: &'static str,
    sql: &'static str,
    params: DrainParams,
}

/// The ten counts, each the predicate of the legacy worker that does the
/// work, with the tenant filter and "no pipeline run owns the submission"
/// (`NOT EXISTS ... pipeline_runs`, the exclusion
/// `list_submissions_needing_gate_decision` applies too: a pipeline receipt
/// also writes a `trace_submissions` row, in the status `received`,
/// `quarantined`, `rejected`, or, after a review approval, `accepted`). Every
/// statement is one `SELECT COUNT(*)` over a tenant predicate: no row is
/// locked and nothing is written.
///
/// Where a count cannot select exactly what its worker selects for the
/// legacy-owned submissions, it selects more, never less: a count that reads
/// zero while the legacy path still owes work is the failure the report exists
/// to prevent. The comments say where. (The legacy workers do not themselves
/// leave out a submission with a pipeline run, except the gate driver; the
/// report does, because that work belongs to the pipeline.)
const LEGACY_DRAIN_COUNTS: [LegacyDrainCount; 10] = [
    // `list_submissions_awaiting_pii_backstop` (db/postgres.rs) selects a
    // submission in this status that also has an active envelope ref, fewer
    // attempts than its ceiling, and an elapsed backoff. This count is the
    // status alone: a submission the driver gave up on, or one in backoff, is
    // still owed its verdict.
    LegacyDrainCount {
        label: "awaiting_pii_backstop",
        sql: "SELECT COUNT(*) FROM trace_submissions s
               WHERE s.tenant_id = $1
                 AND s.status = 'awaiting_pii_backstop'
                 AND NOT EXISTS (
                     SELECT 1 FROM pipeline_runs r
                      WHERE r.tenant_id = s.tenant_id AND r.submission_id = s.submission_id
                 )",
        params: DrainParams::Tenant,
    },
    // `list_submissions_needing_gate_decision` (db/postgres.rs), without its
    // backoff clause: a submission that waits out a backoff is still owed its
    // decision. Attempts below the ceiling the gate driver runs with
    // (`run_perplexity_score_driver_tick`'s `max_attempts`). The envelope ref
    // is an `EXISTS`, not the worker's join: the join fans out to one row for
    // each active ref and the worker deduplicates it with `DISTINCT`.
    LegacyDrainCount {
        label: "gate_decision_pending",
        sql: "SELECT COUNT(*) FROM trace_submissions s
               WHERE s.tenant_id = $1
                 AND EXISTS (
                     SELECT 1 FROM trace_object_refs o
                      WHERE o.tenant_id = s.tenant_id
                        AND o.submission_id = s.submission_id
                        AND o.artifact_kind = 'submitted_envelope'
                        AND o.invalidated_at IS NULL
                        AND o.deleted_at IS NULL
                 )
                 AND NOT EXISTS (
                     SELECT 1 FROM trace_gate_decisions d
                      WHERE d.tenant_id = s.tenant_id AND d.submission_id = s.submission_id
                 )
                 AND COALESCE((
                     SELECT a.attempts FROM trace_gate_evaluation_attempts a
                      WHERE a.tenant_id = s.tenant_id AND a.submission_id = s.submission_id
                 ), 0) < $2
                 AND NOT EXISTS (
                     SELECT 1 FROM pipeline_runs r
                      WHERE r.tenant_id = s.tenant_id AND r.submission_id = s.submission_id
                 )",
        params: DrainParams::TenantAndGateCeiling,
    },
    // The same query with the attempts at or above the ceiling: the driver
    // no longer selects the submission, and no decision exists. Without an
    // operator resetting its attempt row it never gets one.
    LegacyDrainCount {
        label: "gate_decision_exhausted",
        sql: "SELECT COUNT(*) FROM trace_submissions s
               WHERE s.tenant_id = $1
                 AND EXISTS (
                     SELECT 1 FROM trace_object_refs o
                      WHERE o.tenant_id = s.tenant_id
                        AND o.submission_id = s.submission_id
                        AND o.artifact_kind = 'submitted_envelope'
                        AND o.invalidated_at IS NULL
                        AND o.deleted_at IS NULL
                 )
                 AND NOT EXISTS (
                     SELECT 1 FROM trace_gate_decisions d
                      WHERE d.tenant_id = s.tenant_id AND d.submission_id = s.submission_id
                 )
                 AND COALESCE((
                     SELECT a.attempts FROM trace_gate_evaluation_attempts a
                      WHERE a.tenant_id = s.tenant_id AND a.submission_id = s.submission_id
                 ), 0) >= $2
                 AND NOT EXISTS (
                     SELECT 1 FROM pipeline_runs r
                      WHERE r.tenant_id = s.tenant_id AND r.submission_id = s.submission_id
                 )",
        params: DrainParams::TenantAndGateCeiling,
    },
    // `claim_trace_review_lease` (db/trace_corpus_pg.rs) claims a submission
    // in this status. A submission whose lease another reviewer holds is
    // still waiting for a decision, so the lease is not part of the count.
    LegacyDrainCount {
        label: "quarantine_review_pending",
        sql: "SELECT COUNT(*) FROM trace_submissions s
               WHERE s.tenant_id = $1
                 AND s.status = 'quarantined'
                 AND NOT EXISTS (
                     SELECT 1 FROM pipeline_runs r
                      WHERE r.tenant_id = s.tenant_id AND r.submission_id = s.submission_id
                 )",
        params: DrainParams::Tenant,
    },
    // `index_vector_metadata_from_db` (trace-commons-ingest.rs) builds this
    // set in memory: an accepted submission that is not revoked or purged,
    // whose allowed uses include one of `VECTOR_INDEX_ALLOWED_USES`, and
    // each of its current `duplicate_precheck` derived records that has a
    // summary hash, minus the records whose deterministic vector entry id
    // has an active entry. That id is a version 5 UUID over the tenant, the
    // submission, the derived record, and the summary hash, and SQL has no
    // SHA-1: an entry is matched by the three values the id is made of (the
    // entry stores them, and the worker is the only writer of entries), which
    // selects the same records as the id does. What the worker also asks of
    // its caller (the credential's and the tenant policy's consent scopes
    // and uses) is not applied: a record that fails it stays counted.
    LegacyDrainCount {
        label: "vector_index_pending",
        sql: "SELECT COUNT(*) FROM trace_derived_records d
               JOIN trace_submissions s
                 ON s.tenant_id = d.tenant_id AND s.submission_id = d.submission_id
               WHERE d.tenant_id = $1
                 AND d.status = 'current'
                 AND d.worker_kind = 'duplicate_precheck'
                 AND d.canonical_summary_hash IS NOT NULL
                 AND s.status = 'accepted'
                 AND s.revoked_at IS NULL
                 AND s.purged_at IS NULL
                 AND s.allowed_uses ?| ARRAY[
                     'debugging', 'evaluation', 'benchmark_generation',
                     'ranking_model_training', 'model_training'
                 ]
                 AND NOT EXISTS (
                     SELECT 1 FROM trace_vector_entries v
                      WHERE v.tenant_id = d.tenant_id
                        AND v.submission_id = d.submission_id
                        AND v.derived_id = d.derived_id
                        AND v.source_hash = d.canonical_summary_hash
                        AND v.status = 'active'
                        AND v.invalidated_at IS NULL
                        AND v.deleted_at IS NULL
                 )
                 AND NOT EXISTS (
                     SELECT 1 FROM pipeline_runs r
                      WHERE r.tenant_id = s.tenant_id AND r.submission_id = s.submission_id
                 )",
        params: DrainParams::Tenant,
    },
    // `run_credit_settlement_unlocked` (trace-commons-ingest.rs) settles the
    // events of the four types `trace_credit_event_type_is_settlement_eligible`
    // names (`NoveltyUtility` and the immediate events never settle) with a
    // positive delta, on an accepted submission, that no finalized batch
    // names in `source_credit_event_ids`. The settled ids are unnested once
    // and anti-joined, not searched for each event. The worker also skips an
    // event of an account with an active credit hold, a ranking event its
    // calibration gate excludes, and what the per-account cap holds back;
    // those events are still owed, so they stay counted.
    LegacyDrainCount {
        label: "delayed_credit_unsettled",
        sql: "WITH settled AS (
                  SELECT DISTINCT unnest(b.source_credit_event_ids) AS credit_event_id
                    FROM trace_credit_settlement_batches b
                   WHERE b.tenant_id = $1 AND b.status = 'finalized'
              )
              SELECT COUNT(*) FROM trace_credit_ledger l
               JOIN trace_submissions s
                 ON s.tenant_id = l.tenant_id AND s.submission_id = l.submission_id
               WHERE l.tenant_id = $1
                 AND l.event_type IN (
                     'benchmark_conversion', 'regression_catch',
                     'training_utility', 'ranking_utility'
                 )
                 AND l.points_delta::numeric > 0
                 AND s.status = 'accepted'
                 AND NOT EXISTS (
                     SELECT 1 FROM settled x WHERE x.credit_event_id = l.credit_event_id
                 )
                 AND NOT EXISTS (
                     SELECT 1 FROM pipeline_runs r
                      WHERE r.tenant_id = s.tenant_id AND r.submission_id = s.submission_id
                 )",
        params: DrainParams::Tenant,
    },
    // `list_due_trace_revocation_propagation_items` (db/trace_corpus_pg.rs)
    // lists the `pending` and `failed` items whose `next_attempt_at` is due.
    // This count takes the `failed` items whatever their `next_attempt_at`
    // (an item in backoff, or one that reached its attempt limit, is still
    // owed) and the `in_progress` items, which the worker never lists again:
    // an item a crashed run left claimed is work that started and did not
    // finish. Items whose source submission has a pipeline run belong to the
    // pipeline's own follow-up.
    LegacyDrainCount {
        label: "revocation_propagation_pending",
        sql: "SELECT COUNT(*) FROM trace_revocation_propagation_items i
               WHERE i.tenant_id = $1
                 AND i.status IN ('pending', 'in_progress', 'failed')
                 AND NOT EXISTS (
                     SELECT 1 FROM pipeline_runs r
                      WHERE r.tenant_id = i.tenant_id
                        AND r.submission_id = i.source_submission_id
                 )",
        params: DrainParams::Tenant,
    },
    // `read_mains_near_credit_outbox_items` (trace-commons-ingest.rs): the
    // outbox rows `main`'s NEAR workers read, which leave out the pipeline's
    // payout rows (`instrument_id` set), in the states those workers still
    // act on: `pending` and `failed` are submitted, `submitted` is confirmed.
    // Tenant-wide: an outbox row names a settlement batch, not a submission.
    LegacyDrainCount {
        label: "near_outbox_pending",
        sql: "SELECT COUNT(*) FROM trace_near_credit_outbox o
               WHERE o.tenant_id = $1
                 AND o.instrument_id IS NULL
                 AND o.status IN ('pending', 'failed', 'submitted')",
        params: DrainParams::Tenant,
    },
    // `repair_missing_near_credit_outbox_items_for_finalized_batches`
    // (trace-commons-ingest.rs), which every live settlement run calls first.
    // It acts on the line items (`line_items_json`, an array) of a finalized
    // batch that has a NEAR contract (`near_contract_id`, stored on the batch:
    // no ingest setting is needed, and with NEAR payouts not configured no
    // batch has one): an item that names an outbox row (`near_outbox_id`)
    // the table lacks gets the row written again, and an item with a recorded
    // payout hold (`near_payout_hold_reason`, no `near_outbox_id`) and no
    // outbox row for its (batch, account) gets one once the account's payout
    // target resolves. The count takes both kinds of item, and takes a held
    // item whether or not its payout target resolves now: until an outbox row
    // exists the payout is not queued, though the batch is finalized and
    // `delayed_credit_unsettled` reads its events as settled. The pipeline's
    // batches (an `instrument_id`) are not `main`'s to repair.
    LegacyDrainCount {
        label: "near_payout_unqueued",
        sql: "SELECT COUNT(*) FROM trace_credit_settlement_batches b
               CROSS JOIN LATERAL jsonb_array_elements(b.line_items_json) AS item
               WHERE b.tenant_id = $1
                 AND b.status = 'finalized'
                 AND b.instrument_id IS NULL
                 AND b.near_contract_id IS NOT NULL
                 AND (
                     (item ->> 'near_outbox_id' IS NOT NULL
                      AND NOT EXISTS (
                          SELECT 1 FROM trace_near_credit_outbox o
                           WHERE o.tenant_id = b.tenant_id
                             AND o.near_outbox_id = (item ->> 'near_outbox_id')::uuid
                      ))
                     OR
                     (item ->> 'near_outbox_id' IS NULL
                      AND item ->> 'near_payout_hold_reason' IS NOT NULL
                      AND NOT EXISTS (
                          SELECT 1 FROM trace_near_credit_outbox o
                           WHERE o.tenant_id = b.tenant_id
                             AND o.settlement_batch_id = b.settlement_batch_id
                             AND o.credit_account_hash = item ->> 'credit_account_hash'
                      ))
                 )",
        params: DrainParams::Tenant,
    },
    // `list_incomplete_source_session_withdrawals` (db/trace_corpus_pg.rs),
    // which the revocation propagation worker runs for the whole tenant
    // (`reconcile_source_session_withdrawals`): one row for each submission
    // mapped to a withdrawn source session that has no `trace_withdrawals`
    // row, or still has an object ref not marked deleted, a vector entry not
    // invalidated, a dedup assignment, a derived record not revoked, or a
    // token attachment not deleted outside the retention policies a legal hold
    // keeps (`$2`, the worker's `held_retention_policy_ids`). The selection is
    // by session mapping and by actual state, not by an item the legacy path
    // wrote, and the worker completes a version that has a pipeline run too
    // (it runs the pipeline's follow-up first), so the count is tenant-wide:
    // it does not leave out a submission with a run.
    LegacyDrainCount {
        label: "withdrawal_completion_pending",
        sql: "SELECT COUNT(*) FROM trace_submission_sessions mapping
               JOIN trace_source_sessions source
                 ON source.tenant_id = mapping.tenant_id
                AND source.account_id = mapping.account_id
                AND source.session_digest = mapping.session_digest
               WHERE mapping.tenant_id = $1
                 AND source.withdrawn_at IS NOT NULL
                 AND (
                     NOT EXISTS (SELECT 1 FROM trace_withdrawals withdrawal
                                  WHERE withdrawal.tenant_id = mapping.tenant_id
                                    AND withdrawal.submission_id = mapping.submission_id)
                  OR EXISTS (SELECT 1 FROM trace_object_refs object_ref
                              WHERE object_ref.tenant_id = mapping.tenant_id
                                AND object_ref.submission_id = mapping.submission_id
                                AND object_ref.deleted_at IS NULL)
                  OR EXISTS (SELECT 1 FROM trace_vector_entries vector_entry
                              WHERE vector_entry.tenant_id = mapping.tenant_id
                                AND vector_entry.submission_id = mapping.submission_id
                                AND vector_entry.status <> 'invalidated'
                                AND vector_entry.deleted_at IS NULL)
                  OR EXISTS (SELECT 1 FROM trace_gate_decisions decision
                              WHERE decision.tenant_id = mapping.tenant_id
                                AND decision.submission_id = mapping.submission_id
                                AND (decision.dedup_cluster_id IS NOT NULL
                                     OR decision.dedup_simhash IS NOT NULL))
                  OR EXISTS (SELECT 1 FROM trace_derived_records derived
                              WHERE derived.tenant_id = mapping.tenant_id
                                AND derived.submission_id = mapping.submission_id
                                AND derived.status <> 'revoked')
                  OR EXISTS (SELECT 1 FROM trace_token_attachments attachment
                               JOIN trace_submissions submission
                                 ON submission.tenant_id = attachment.tenant_id
                                AND submission.submission_id = attachment.submission_id
                              WHERE attachment.tenant_id = mapping.tenant_id
                                AND attachment.submission_id = mapping.submission_id
                                AND attachment.deleted = FALSE
                                AND NOT (submission.retention_policy_id = ANY($2)))
                 )",
        params: DrainParams::TenantAndHeldRetentionPolicies,
    },
];

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
    ///
    /// A pipeline run that was created before the ownership table existed
    /// has no ownership row, and the pipeline owns its submission id all the
    /// same: the claim finds the run, answers `Pipeline`, and writes no
    /// legacy row, so the legacy path never takes such an id.
    pub async fn claim_legacy_receipt(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<ReceiptOwner, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        if tx
            .query_opt(
                "SELECT 1 FROM pipeline_runs WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &submission_id],
            )
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(ReceiptOwner::Pipeline);
        }
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
        validate_actor(
            actor_principal_ref,
            reason_code,
            ACTIVATION_ACTOR_INVALID_LABEL,
        )?;
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
        validate_actor(
            actor_principal_ref,
            reason_code,
            ACTIVATION_ACTOR_INVALID_LABEL,
        )?;
        self.change_routing(
            tenant_id,
            actor_principal_ref,
            reason_code,
            ActivationAction::Deactivate,
            RoutingState::Legacy,
        )
        .await
    }

    /// What the legacy path still owes `tenant_id`, as ten counts and one
    /// verdict: `drained` is true only when every count is zero. Before a
    /// tenant's legacy writer can be retired the legacy code must have
    /// finished this work. Each count is the selection of the legacy worker
    /// that does the work, left to what the pipeline does not own: by
    /// submission (no pipeline run) for the first seven counts, by payout
    /// instrument (none) for the two NEAR counts, and tenant-wide for the
    /// withdrawals. `LEGACY_DRAIN_COUNTS` names each worker function and every
    /// place a count is wider than the worker's own selection:
    ///
    /// - `awaiting_pii_backstop`: submissions held for the PII backstop's
    ///   verdict.
    /// - `gate_decision_pending`: submissions with an active submitted
    ///   envelope and no gate decision whose attempts are below
    ///   `gate_max_attempts` (`list_submissions_needing_gate_decision`, with
    ///   no backoff and no status filter).
    /// - `gate_decision_exhausted`: the same with attempts at or above the
    ///   ceiling, which the gate driver no longer selects.
    /// - `quarantine_review_pending`: quarantined submissions waiting for a
    ///   reviewer's decision (`claim_trace_review_lease`).
    /// - `vector_index_pending`: the current precheck records of accepted
    ///   submissions that have no active vector entry
    ///   (`index_vector_metadata_from_db`).
    /// - `delayed_credit_unsettled`: positive credit events of the four
    ///   settlement-eligible types on accepted submissions that no finalized
    ///   settlement batch names (`run_credit_settlement`).
    /// - `revocation_propagation_pending`: revocation propagation items
    ///   `pending`, `in_progress`, or `failed`
    ///   (`list_due_trace_revocation_propagation_items`).
    /// - `near_outbox_pending`, tenant-wide: `main`'s NEAR outbox rows
    ///   `pending`, `failed`, or `submitted` (`read_mains_near_credit_outbox_items`).
    /// - `near_payout_unqueued`: line items of finalized `main` settlement
    ///   batches that have a NEAR contract and whose payout has no outbox row:
    ///   a payout held for want of a payout target, or an outbox row that is
    ///   missing (`repair_missing_near_credit_outbox_items_for_finalized_batches`,
    ///   which each live settlement run calls first). The batch is settled,
    ///   so `delayed_credit_unsettled` and `near_outbox_pending` read zero
    ///   while the payout is not queued. A batch has a contract only when NEAR
    ///   payouts were configured when it was settled: with none, this count is
    ///   zero.
    /// - `withdrawal_completion_pending`, tenant-wide: submissions of a
    ///   withdrawn source session whose withdrawal is not complete
    ///   (`list_incomplete_source_session_withdrawals`, which the revocation
    ///   propagation worker runs for the whole tenant). The worker completes a
    ///   version that has a pipeline run too, so this count does not leave
    ///   out a submission with a run.
    ///
    /// Counts that can stay above zero without legacy code ever clearing
    /// them, so that an operator reads them as work for a person:
    ///
    /// - `gate_decision_pending` is every legacy submission with an envelope
    ///   and no decision, accepted or not, and nothing scores one by itself
    ///   unless the in-process gate driver runs, which it does not unless an
    ///   operator turns it on (`TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED`);
    ///   `/v1/workers/gate/evaluate` scores a submission its caller names.
    /// - `gate_decision_exhausted` and `awaiting_pii_backstop` after the
    ///   attempts ran out: the driver stops, and an operator resets the
    ///   attempt row or accepts the loss.
    /// - `quarantine_review_pending` waits for a reviewer, not for code.
    /// - `revocation_propagation_pending` for an item left `in_progress` by a
    ///   run that crashed: the worker never lists it again.
    /// - `delayed_credit_unsettled` for credit of an account with an active
    ///   hold, a ranking event that its calibration gate excludes, or what the
    ///   per-account cap holds back: the settlement worker skips them until
    ///   the hold is released, the gate passes, or the cap allows them.
    /// - `vector_index_pending` for a record that the credential's or the
    ///   tenant policy's consent check refuses: the report cannot apply that
    ///   check, so the worker will never index the record.
    /// - `near_outbox_pending` for a line `submitted` with no transaction
    ///   hash: the confirm worker acts only on a hash.
    /// - `near_payout_unqueued` for an account that stays without a payout
    ///   target (none enrolled, or several and none designated): enrolling or
    ///   designating one, then a live settlement run, queues the payout.
    /// - `withdrawal_completion_pending` for a withdrawal whose tail keeps
    ///   failing; the held retention policies below keep a legal hold's
    ///   attachments out of the count.
    ///
    /// Not counted, by the owner's decision (P5-D8), because none of it is
    /// follow-up that the legacy path owes for a receipt it took. This list
    /// is the owner's; nothing here proves it complete.
    ///
    /// - Retention and purge are time-driven maintenance over every
    ///   submission's `expires_at`; the pipeline's submissions go through the
    ///   same maintenance.
    /// - Export jobs are snapshots an operator requests and completes by
    ///   calling a route; they have their own completion state.
    /// - Benchmark and process-evaluation work runs when an operator calls
    ///   its route, with the inputs of the request.
    /// - The database mirror backfill copies file records into the tables, an
    ///   operator's maintenance over the file store.
    /// - Work that the legacy path does not start by itself:
    ///   `/v1/workers/utility-credit` takes its submission ids from its
    ///   caller, so no table lists what it still has to do.
    ///
    /// The counts `near_outbox_pending`, `near_payout_unqueued`, and
    /// `withdrawal_completion_pending` block `drained` (owner decisions,
    /// 2026-10-02): retiring the legacy path before a payout is confirmed or
    /// queued, or a withdrawal is complete, would strand money or privacy
    /// work.
    ///
    /// Precondition on the caller. The report reads the database tables only,
    /// and the store cannot see how the deployment fills and reads them. The
    /// caller must not present this report as a drain signal unless the
    /// tenant's legacy records in the database are authoritative: (1) the
    /// legacy path's database writes are required, not best effort
    /// (`TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES`, or account admission, is
    /// on), so a failed database write fails the legacy operation instead of
    /// leaving a record in the file store alone; and (2) the
    /// tenant's review, credit, settlement, and outbox reads come from the
    /// database (`db_reviewer_reads_for_tenant`), so the legacy workers act
    /// on the rows this report counts. Otherwise a quarantined submission, a
    /// credit event, or an outbox row that exists only in the file store is
    /// owed work and the report reads zero with `drained` true.
    ///
    /// Parameters. `tenant_id` is the tenant of the credential, never a
    /// second tenant. `gate_max_attempts` is the attempt ceiling the gate
    /// driver runs with (`PerplexityDriverKnobs::max_attempts`); it splits
    /// the gate work into pending (below the ceiling) and exhausted (at it).
    /// `held_retention_policy_ids` is the retention policies a legal hold
    /// keeps (`legal_hold_retention_policy_ids`), as the withdrawal worker
    /// receives them: a withdrawn submission's token attachment under one of
    /// them is not owed deletion, so it is not counted. An empty list holds
    /// nothing.
    ///
    /// The report runs as the ingest runtime role in one tenant transaction:
    /// a read-only, single-snapshot transaction (`REPEATABLE READ`), so all
    /// ten counts and the routing state describe one instant, with one
    /// statement for each count and no row lock.
    pub async fn legacy_drain_report(
        &self,
        tenant_id: &str,
        gate_max_attempts: i32,
        held_retention_policy_ids: &[String],
    ) -> Result<LegacyDrainReport, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::read_only_tenant_transaction(&mut client, tenant_id).await?;
        let mut pending = BTreeMap::new();
        for count in &LEGACY_DRAIN_COUNTS {
            let params: &[&(dyn ToSql + Sync)] = match count.params {
                DrainParams::Tenant => &[&tenant_id],
                DrainParams::TenantAndGateCeiling => &[&tenant_id, &gate_max_attempts],
                DrainParams::TenantAndHeldRetentionPolicies => {
                    &[&tenant_id, &held_retention_policy_ids]
                }
            };
            let counted: i64 = tx.query_one(count.sql, params).await?.get(0);
            let counted = u64::try_from(counted).map_err(|_| {
                DatabaseError::Serialization("legacy_drain_count_invalid".to_string())
            })?;
            pending.insert(count.label.to_string(), counted);
        }
        let routing_state = tx
            .query_opt(
                "SELECT routing_state FROM pipeline_tenant_routing WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?
            .map(|row| RoutingState::from_db(row.get::<_, String>("routing_state").as_str()))
            .transpose()?;
        tx.commit().await?;
        let drained = pending.values().all(|count| *count == 0);
        let evidence_hash =
            crate::versioned_pipeline_qualification::evidence_hash(&serde_json::json!({
                "schema": LEGACY_DRAIN_EVIDENCE_SCHEMA,
                "pending": pending,
            }))
            .map_err(DatabaseError::Serialization)?;
        Ok(LegacyDrainReport {
            generated_at: Utc::now(),
            routing_state,
            pending,
            drained,
            evidence_hash,
        })
    }

    /// `tenant_transaction` for a read: the transaction cannot write, and
    /// every statement in it sees one snapshot.
    async fn read_only_tenant_transaction<'a>(
        client: &'a mut deadpool_postgres::Client,
        tenant_id: &str,
    ) -> Result<Transaction<'a>, DatabaseError> {
        let tx = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .await?;
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant_id],
        )
        .await?;
        Ok(tx)
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
            suspended_policy_count: 0,
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
        assert!(
            validate_actor(
                &actor,
                "contain_first_rollout",
                ACTIVATION_ACTOR_INVALID_LABEL
            )
            .is_ok()
        );
        for (actor, reason) in [
            (actor.as_str(), "Bad Reason"),
            ("", "contain_first_rollout"),
            ("an actor with spaces", "contain_first_rollout"),
            (actor.as_str(), ""),
        ] {
            assert!(
                matches!(
                    validate_actor(actor, reason, ACTIVATION_ACTOR_INVALID_LABEL),
                    Err(DatabaseError::Constraint(label)) if label == ACTIVATION_ACTOR_INVALID_LABEL
                ),
                "actor {actor:?} with reason {reason:?} must be refused"
            );
        }
        let too_long = "a".repeat(161);
        assert!(matches!(
            validate_actor(
                &too_long,
                "contain_first_rollout",
                ACTIVATION_ACTOR_INVALID_LABEL
            ),
            Err(DatabaseError::Constraint(label)) if label == ACTIVATION_ACTOR_INVALID_LABEL
        ));
    }
}
