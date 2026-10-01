// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Product views over authoritative versioned-pipeline records.
//!
//! This module does not persist contributor status projections. Each status
//! response is derived from the run, immutable outcomes, credit ledger,
//! settlement batch, and payout state at read time: there is no mutable
//! status table to fall out of sync with them.
//!
//! Withdrawal (`withdraw_submission`) is not read through this module --
//! `PgPipelineStore` owns it, and it is also what invalidates an export
//! snapshot and its items when a submission they carry is withdrawn.
//!
//! Approved exports live here: `create_export_snapshot` selects the
//! tenant's approved revisions into an immutable snapshot, and
//! `complete_export_snapshot` records its delivery as `main`'s export
//! manifest.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use deadpool_postgres::Transaction;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_postgres::Row;
use trace_commons_gate_api::pipeline::{AtomicUnits, Phase};
use trace_commons_protocol::trace_contribution::{ConsentScope, ResidualPiiRisk, TraceAllowedUse};
use uuid::Uuid;

use crate::db::postgres::{PgBackend, TRACE_COMMONS_RLS_TABLES};
use crate::error::DatabaseError;
use crate::trace_corpus_storage::TraceObjectArtifactKind;
use crate::versioned_pipeline::{
    PIPELINE_SUBMISSION_INOPERABLE_LABEL, PipelineRunState, accepted_submission_sql,
    live_submission_sql, phase_from_db, sha256_prefixed,
};
use crate::versioned_pipeline_compat::COMPATIBILITY_SCORE_IMPLEMENTATION;

pub const PIPELINE_STATUS_BATCH_MAX: usize = 500;
/// The most items one export snapshot holds. V106 bounds its items'
/// `ordinal` to the same number.
pub const PIPELINE_EXPORT_ITEM_MAX: usize = 500;
pub const PIPELINE_EXPORT_SELECTION_POLICY_ID: &str = "trace_commons.pipeline_export_selection.v1";
pub const PIPELINE_AUTHORIZED_VIEW_SCHEMA_ID: &str = "trace_commons.authorized_trace_view.v1";
/// The purpose-code family of a delivered snapshot's export manifest:
/// `pipeline_export:<allowed use>`. `main`'s replay-dataset manifest list and
/// its replay manifest count leave this family out, as they leave out the
/// ranker-training families, so a replay worker never takes a pipeline
/// export for a replay dataset. The manifest's kind stays `export_artifact`.
pub const PIPELINE_EXPORT_MANIFEST_PURPOSE_PREFIX: &str = "pipeline_export";

/// The purpose code of the export manifest a snapshot for `allowed_use` is
/// delivered as: `pipeline_export:<allowed use>`.
pub fn pipeline_export_manifest_purpose_code(
    allowed_use: TraceAllowedUse,
) -> Result<String, DatabaseError> {
    Ok(format!(
        "{PIPELINE_EXPORT_MANIFEST_PURPOSE_PREFIX}:{}",
        storage_label(allowed_use)?
    ))
}

/// Whether a manifest's purpose code is in the
/// `PIPELINE_EXPORT_MANIFEST_PURPOSE_PREFIX` family.
pub fn is_pipeline_export_manifest_purpose_code(purpose_code: &str) -> bool {
    purpose_code
        .strip_prefix(PIPELINE_EXPORT_MANIFEST_PURPOSE_PREFIX)
        .is_some_and(|suffix| suffix.starts_with(':'))
}

/// `create_export_snapshot`'s refusal of a request key that an earlier,
/// different request used.
pub const PIPELINE_EXPORT_IDEMPOTENCY_CONFLICT: &str = "export idempotency content conflict";
/// `complete_export_snapshot`'s refusal of a snapshot a withdrawal
/// invalidated.
pub const PIPELINE_EXPORT_SNAPSHOT_INVALIDATED: &str = "export snapshot is invalidated";
/// `complete_export_snapshot`'s refusal of a snapshot with an item a
/// withdrawal invalidated.
pub const PIPELINE_EXPORT_SOURCE_INVALIDATED: &str =
    "export snapshot contains an invalidated source";

/// Amounts cross the API as decimal strings, because a JavaScript client
/// (the Tauri app) cannot hold an integer above `2^53` exactly. `AtomicUnits`
/// already serializes this way on its own; these two
/// helpers give the same wire shape to the plain-`u64` amount fields in this
/// module's product record types (`score_microcredits`, `scored_microcredits`,
/// and `PipelineContributorCredit`'s totals) via `#[serde(with = "...")]`.
mod decimal_amount {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        String::deserialize(deserializer)?
            .parse::<u64>()
            .map_err(serde::de::Error::custom)
    }

    /// The `Option<u64>` counterpart: `None` stays JSON `null`; `Some` is
    /// the same decimal string as `decimal_amount`.
    pub mod option {
        use serde::{Deserialize, Deserializer, Serialize, Serializer};

        pub fn serialize<S: Serializer>(
            value: &Option<u64>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            value.map(|value| value.to_string()).serialize(serializer)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<u64>, D::Error> {
            Option::<String>::deserialize(deserializer)?
                .map(|raw| raw.parse::<u64>().map_err(serde::de::Error::custom))
                .transpose()
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PipelineProcessingStatus {
    Pending,
    Retry,
    /// Review quarantined the run and it waits, parked, for a human
    /// assessment (`PipelineRunState::AwaitingReview`). It is neither a
    /// retry nor a failure.
    AwaitingReview,
    Blocked,
    Complete,
    Rejected,
    Withdrawn,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PipelineCreditStatus {
    Unscored,
    Zero,
    Pending,
    Held,
    Finalized,
    Failed,
    /// Ruling S11: a compatibility bundle's `NoveltyUtility` Trace Credit
    /// leg. It completes like any other leg, but the ledger event type it
    /// wrote is one `main` never batches or pays
    /// (`trace_credit_event_type_is_settlement_eligible` excludes it), so it
    /// would otherwise sit at `Pending` forever.
    NotSettlementEligible,
    /// Ruling RB-6: a `trace_credit` leg a failed Settle run
    /// (`resolve_open_settlement_legs_on_tx`) or a withdrawal after Score
    /// forfeits. It never settles and is never paid, so it must not sit at
    /// `Pending` forever either.
    Forfeited,
    /// Ruling T15-12: a compatibility `NoveltyUtility` leg one of `main`'s
    /// credit checks refused. It completed with no ledger event, and its
    /// `reason_label` is `main`'s withheld reason.
    Withheld,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineInstrumentStatus {
    pub instrument_id: String,
    pub atomic_units: AtomicUnits,
    pub operation_state: String,
    pub internal_settlement_state: String,
    pub credit_event_id: Option<Uuid>,
    pub settlement_batch_id: Option<Uuid>,
    pub payout_rail: String,
    pub payout_state: String,
    pub reason_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineContributorStatus {
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub run_id: Uuid,
    pub bundle_id: String,
    pub processing: PipelineProcessingStatus,
    pub current_phase: Option<Phase>,
    pub responsible_phase: Option<Phase>,
    pub reason_label: Option<String>,
    pub credit: PipelineCreditStatus,
    #[serde(with = "decimal_amount::option")]
    pub score_microcredits: Option<u64>,
    pub score_outcome_id: Option<Uuid>,
    /// Compatibility projection for clients that only understand Trace Credit.
    pub settlement_batch_id: Option<Uuid>,
    /// Compatibility projection for clients that only understand one payout.
    pub payout: Option<String>,
    pub instruments: Vec<PipelineInstrumentStatus>,
    /// Ruling T15-7: `Some` when the run is bound to a compatibility-family
    /// bundle, whose contributor status reads as `main`'s document for the
    /// same credit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<PipelineCompatibilityStatus>,
    /// The submission's `trace_submissions.status`, in `main`'s storage
    /// vocabulary (`received`, `accepted`, `rejected`, `revoked`, ...), and
    /// the run's Admission decision (`admit`, `quarantine`, `reject`): what
    /// `main`'s status route needs to report a pipeline-only submission in
    /// its own vocabulary (Zaki review 1, minor item M-c). Not part of the
    /// pipeline's own JSON.
    #[serde(skip)]
    pub submission_status: String,
    #[serde(skip)]
    pub admission_decision: String,
}

/// What a compatibility run's contributor status needs beyond the run: the
/// credit-quality figure `main` shows from its gate decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineCompatibilityStatus {
    /// The shadow credit quality the committed Score outcome recorded;
    /// `None` until Score commits.
    pub credit_quality: Option<PipelineShadowCreditQuality>,
}

/// A compatibility Score's shadow credit quality and the coverage it was
/// measured over: the fields of `main`'s gate decision its status reads.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineShadowCreditQuality {
    pub credit_quality_micros: u64,
    pub credit_quality_version: i32,
    pub chunk_count: Option<u32>,
    pub total_chunk_count: Option<u32>,
    pub chunks_capped: Option<bool>,
}

/// One run's committed Score outcome, as the versioned score attestation
/// signs it: the outcome's provenance, the Trace Credit award Score
/// computed, and the decision itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineScoreAttestationEntry {
    pub submission_id: Uuid,
    pub run_id: Uuid,
    pub score_outcome_id: Uuid,
    pub bundle_id: String,
    pub outcome_schema_id: String,
    pub outcome_schema_version: u32,
    /// The Trace Credit award Score computed, in microcredits. It is what
    /// Score decided, not credit that was issued: Settle can still withhold
    /// the award (one of `main`'s credit checks refused it) or forfeit it (a
    /// withdrawal came first), and the contributor status says which
    /// (Ruling F-M7). The same name as `PipelineContributorCredit`'s total.
    #[serde(with = "decimal_amount")]
    pub scored_microcredits: u64,
    pub decision: serde_json::Value,
}

/// One approved revision in an export snapshot. `source_object_ref_id` is
/// the run's approved object and `source_content_hash` its
/// `approved_content_hash`: the bytes Review approved, after the privacy
/// boundary transformed the request. The raw request's object is never an
/// item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineExportSnapshotItem {
    pub ordinal: u32,
    pub run_id: Uuid,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub registry_revision_id: Uuid,
    pub source_object_ref_id: Uuid,
    pub source_content_hash: String,
    pub bundle_id: String,
    pub outcome_schema_id: String,
    pub outcome_schema_version: u32,
    pub authorized_view_schema_id: String,
    pub consent_scopes: serde_json::Value,
    pub allowed_uses: serde_json::Value,
    pub invalidation_reason: Option<String>,
}

/// The consent scopes an export may carry, decided for each submission as
/// `main`'s export path decides them for each record: the caller's scoped
/// token and the tenant policy each hold an allowlist, and a submission is
/// selected only when its consent scopes meet every non-empty allowlist,
/// each through any one of its scopes. An empty allowlist allows every
/// scope. The two are checked one by one, not intersected: a submission
/// that consents to scopes A and B meets a token allowlist of A and a
/// policy allowlist of B.
///
/// `requested` is the consent scope the export request names, as `main`'s
/// replay export filter `consent_scope`: when set, a submission must also
/// hold that scope. It only narrows the two allowlists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipelineExportConsentScopes {
    pub token: BTreeSet<ConsentScope>,
    pub policy: BTreeSet<ConsentScope>,
    pub requested: Option<ConsentScope>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineExportSnapshot {
    #[serde(skip_serializing, default)]
    pub tenant_id: String,
    pub snapshot_id: Uuid,
    pub request_idempotency_key: String,
    pub requester_principal_ref: String,
    pub allowed_use: TraceAllowedUse,
    pub purpose_hash: String,
    pub selection_policy_id: String,
    pub source_list_hash: String,
    pub state: String,
    pub export_manifest_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub invalidated_at: Option<DateTime<Utc>>,
    pub items: Vec<PipelineExportSnapshotItem>,
}

// Every `u64` field below is checked against the decimal-string amount rule.
// None of them is an amount -- `PipelineWorkSummary` and
// `PipelineOperationalSummary`'s fields are all
// counts or durations (of invalidations, snapshots, errors, commands, credit
// events, outbox rows -- never a credit or token amount), and
// `PipelinePhaseTrace`'s are hashes and an outcome version. None gets
// `decimal_amount`. `PipelineForensicTrace`'s only amount-bearing field is
// `instruments: Vec<PipelineInstrumentStatus>`, whose own `atomic_units` is
// already `AtomicUnits` (decimal-string on its own), so it needs no
// additional attribute either.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineWorkSummary {
    pub phase: String,
    pub state: String,
    /// The run's `last_error_label` for this bucket, matching
    /// `^[a-z0-9_]{1,64}$` -- a retry or a suspension always carries a safe
    /// reason label here, so the bucket stays label-only, never a raw
    /// message.
    pub reason_label: Option<String>,
    pub count: u64,
    pub oldest_age_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineOperationalSummary {
    pub generated_at: DateTime<Utc>,
    pub work: Vec<PipelineWorkSummary>,
    pub retryable_error_count: u64,
    pub terminal_error_count: u64,
    pub pending_index_command_count: u64,
    pub failed_index_command_count: u64,
    pub held_credit_count: u64,
    pub delayed_credit_count: u64,
    pub near_outbox_by_state: BTreeMap<String, u64>,
    pub pending_invalidation_count: u64,
    pub failed_invalidation_count: u64,
    pub incomplete_export_count: u64,
    pub tenant_isolation_control_passed: bool,
    pub audit_immutability_control_passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelinePhaseTrace {
    pub phase: String,
    pub outcome_id: Uuid,
    pub outcome_schema_id: String,
    pub outcome_schema_version: u32,
    pub decision_hash: String,
    pub evidence_hash: String,
    pub evaluation_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineForensicTrace {
    pub run_id: Uuid,
    pub submission_id: Uuid,
    pub bundle_id: String,
    pub phases: Vec<PipelinePhaseTrace>,
    pub index_command_hash: Option<String>,
    pub index_write_state: String,
    pub score_outcome_id: Option<Uuid>,
    pub credit_event_id: Option<Uuid>,
    pub settlement_batch_id: Option<Uuid>,
    pub payout_state: String,
    pub instruments: Vec<PipelineInstrumentStatus>,
    pub index_invalidation_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineContributorCredit {
    #[serde(with = "decimal_amount")]
    pub scored_microcredits: u64,
    #[serde(with = "decimal_amount")]
    pub finalized_microcredits: u64,
    #[serde(with = "decimal_amount")]
    pub pending_microcredits: u64,
    #[serde(with = "decimal_amount")]
    pub held_microcredits: u64,
    pub submission_count: usize,
}

/// Shared by `contributor_statuses_for_principals` and `forensic_trace`
/// (Ruling T5-2, following S11): a `pipeline_run_settlements` row's internal
/// lifecycle state, given its own row aliased `settlement`, its optional
/// carrying batch aliased `batch`, and its optional ledger event aliased
/// `ledger`. A `trace_credit` row whose ledger event is `NoveltyUtility`
/// (the stored string `TraceCreditEventType::NoveltyUtility`'s own serde
/// produces, `trace_corpus_storage.rs`, `rename_all = "snake_case"`; the
/// same string `versioned_pipeline.rs`'s `settle_internal_credit` writes,
/// Ruling S10) reports `not_settlement_eligible` rather than `pending`
/// forever, since `main` never batches or pays that event type. A
/// `trace_credit` row that completed with no ledger event is one of `main`'s
/// credit checks refused (Ruling T15-12): `withheld`. Every query that
/// embeds this must alias its rows exactly as this text assumes, and must
/// also embed `TRACE_CREDIT_LEDGER_JOIN`.
const INTERNAL_SETTLEMENT_STATE_CASE: &str = "
                                    CASE
                                        WHEN settlement.instrument_id <> 'trace_credit'
                                            THEN 'not_applicable'
                                        WHEN settlement.operation_state = 'complete'
                                         AND settlement.credit_event_id IS NULL
                                            THEN 'withheld'
                                        WHEN ledger.event_type = 'novelty_utility'
                                            THEN 'not_settlement_eligible'
                                        WHEN batch.status IS NOT NULL THEN batch.status
                                        WHEN settlement.credit_event_id IS NOT NULL THEN 'pending'
                                        ELSE settlement.operation_state
                                    END";

/// The join `INTERNAL_SETTLEMENT_STATE_CASE` depends on for its
/// `ledger.event_type` read. Must appear in a query that already joins
/// `pipeline_run_settlements` as `settlement`.
const TRACE_CREDIT_LEDGER_JOIN: &str = "
                          LEFT JOIN trace_credit_ledger ledger
                            ON ledger.tenant_id = settlement.tenant_id
                           AND ledger.credit_event_id = settlement.credit_event_id
                           AND ledger.instrument_id = settlement.instrument_id";

/// Whether the submission aliased `s` may be exported for the allowed use
/// bound as `$2`.
///
/// The operability rule of `PgPipelineStore::submission_guard_on_tx`, the
/// guard Score and Settle commit under, from its one definition
/// (`accepted_submission_sql!`), then `main`'s record-level export rule
/// (`record_matches_export_policy_abac`): the submission's own
/// `allowed_uses` holds the requested use.
const EXPORTABLE_SUBMISSION_PREDICATE: &str =
    concat!(accepted_submission_sql!(), " AND s.allowed_uses ? $2");

/// The export selection: complete runs of tenant `$1` with a committed
/// Review approved revision whose approved object is live, and whose
/// submission may be exported for `$2` (`EXPORTABLE_SUBMISSION_PREDICATE`),
/// oldest run first, at most `$4`. `$3`, when not NULL, keeps only those
/// run ids. `$5` and `$6` are the token's and the policy's consent-scope
/// allowlists (`PipelineExportConsentScopes`), NULL when empty: a
/// submission must hold at least one scope of each that is not NULL. `$7`,
/// when not NULL, is a scope the submission must hold, and `$8`, when not
/// NULL, the privacy risk it must have.
fn export_selection_sql() -> String {
    format!(
        "SELECT r.run_id, r.submission_id, r.trace_id, r.approved_revision_id,
                r.approved_object_ref_id, r.approved_content_hash, r.bundle_id,
                review.outcome_schema_id, review.outcome_schema_version,
                s.consent_scopes, s.allowed_uses
           FROM pipeline_runs r
           JOIN trace_submissions s
             ON s.tenant_id = r.tenant_id
            AND s.submission_id = r.submission_id
           JOIN trace_object_refs approved_object
             ON approved_object.tenant_id = r.tenant_id
            AND approved_object.submission_id = r.submission_id
            AND approved_object.object_ref_id = r.approved_object_ref_id
            AND approved_object.invalidated_at IS NULL
            AND approved_object.deleted_at IS NULL
           JOIN phase_outcomes review
             ON review.tenant_id = r.tenant_id
            AND review.run_id = r.run_id
            AND review.phase = 'review'
          WHERE r.tenant_id = $1
            AND r.state = 'complete'
            AND r.approved_revision_id IS NOT NULL
            AND r.approved_object_ref_id IS NOT NULL
            AND r.approved_content_hash IS NOT NULL
            AND ($3::uuid[] IS NULL OR r.run_id = ANY($3))
            AND ($5::text[] IS NULL OR s.consent_scopes ?| $5)
            AND ($6::text[] IS NULL OR s.consent_scopes ?| $6)
            AND ($7::text IS NULL OR s.consent_scopes ? $7)
            AND ($8::text IS NULL OR s.privacy_risk = $8)
            AND {EXPORTABLE_SUBMISSION_PREDICATE}
          ORDER BY r.created_at ASC, r.run_id ASC
          LIMIT $4"
    )
}

#[derive(Clone)]
pub struct PipelineProductStore {
    backend: Arc<PgBackend>,
}

impl PipelineProductStore {
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

    pub async fn contributor_statuses(
        &self,
        tenant_id: &str,
        principal_ref: &str,
        submission_ids: &[Uuid],
    ) -> Result<Vec<PipelineContributorStatus>, DatabaseError> {
        self.contributor_statuses_for_principals(
            tenant_id,
            &[principal_ref.to_string()],
            submission_ids,
        )
        .await
    }

    pub async fn contributor_statuses_for_principals(
        &self,
        tenant_id: &str,
        principal_refs: &[String],
        submission_ids: &[Uuid],
    ) -> Result<Vec<PipelineContributorStatus>, DatabaseError> {
        if submission_ids.len() > PIPELINE_STATUS_BATCH_MAX {
            return Err(DatabaseError::Constraint(
                "submission status batch limit exceeded".to_string(),
            ));
        }
        if submission_ids.is_empty() {
            return Ok(Vec::new());
        }
        if principal_refs.is_empty() {
            return Ok(Vec::new());
        }
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let sql = format!(
            "SELECT r.*, s.status AS submission_status,
                        score.outcome_id AS score_outcome_id,
                        score.decision AS score_decision,
                        score.outcome_schema_id AS score_schema_id,
                        score.outcome_schema_version AS score_schema_version,
                        package.package->'manifest'->'score'->>'implementation_id'
                            AS score_implementation_id,
                        (score.evidence->>'credit_quality_micros')::BIGINT
                            AS score_credit_quality_micros,
                        (score.evidence->>'credit_quality_version')::INTEGER
                            AS score_credit_quality_version,
                        (score.evidence->>'chunk_count')::BIGINT AS score_chunk_count,
                        (score.evidence->>'total_chunk_count')::BIGINT
                            AS score_total_chunk_count,
                        (score.evidence->>'chunks_capped')::BOOLEAN AS score_chunks_capped,
                        latest.phase AS latest_outcome_phase,
                        latest.decision AS latest_outcome_decision,
                        COALESCE(settlements.items, '[]'::jsonb) AS instrument_statuses
                   FROM unnest($3::uuid[]) WITH ORDINALITY requested(submission_id, ordinal)
                   JOIN trace_submissions s
                     ON s.tenant_id = $1
                    AND s.submission_id = requested.submission_id
                    AND s.auth_principal_ref = ANY($2)
                   JOIN LATERAL (
                        SELECT pr.*
                          FROM pipeline_runs pr
                         WHERE pr.tenant_id = s.tenant_id
                           AND pr.submission_id = s.submission_id
                         ORDER BY pr.created_at DESC
                         LIMIT 1
                   ) r ON TRUE
                   LEFT JOIN pipeline_bundle_packages package
                     ON package.tenant_id = r.tenant_id
                    AND package.bundle_id = r.bundle_id
                   LEFT JOIN phase_outcomes score
                     ON score.tenant_id = r.tenant_id
                    AND score.run_id = r.run_id
                    AND score.phase = 'score'
                   LEFT JOIN LATERAL (
                        SELECT po.phase, po.decision
                          FROM phase_outcomes po
                         WHERE po.tenant_id = r.tenant_id
                           AND po.run_id = r.run_id
                         ORDER BY CASE po.phase
                            WHEN 'admission' THEN 1
                            WHEN 'review' THEN 2
                            WHEN 'score' THEN 3
                            WHEN 'settle' THEN 4
                         END DESC
                         LIMIT 1
                   ) latest ON TRUE
                   LEFT JOIN LATERAL (
                        SELECT jsonb_agg(
                            jsonb_build_object(
                                'instrument_id', settlement.instrument_id,
                                'atomic_units', settlement.atomic_units::text,
                                'operation_state', settlement.operation_state,
                                'internal_settlement_state', {INTERNAL_SETTLEMENT_STATE_CASE},
                                'credit_event_id', settlement.credit_event_id,
                                'settlement_batch_id', settlement.settlement_batch_id,
                                'payout_rail', settlement.payout_rail,
                                'payout_state', settlement.payout_state,
                                'reason_label', settlement.last_error_label
                            )
                            ORDER BY settlement.instrument_id
                        ) AS items
                          FROM pipeline_run_settlements settlement
                          LEFT JOIN trace_credit_settlement_batches batch
                            ON batch.tenant_id = settlement.tenant_id
                           AND batch.settlement_batch_id = settlement.settlement_batch_id
                           AND batch.instrument_id = settlement.instrument_id
                          {TRACE_CREDIT_LEDGER_JOIN}
                         WHERE settlement.tenant_id = r.tenant_id
                           AND settlement.run_id = r.run_id
                   ) settlements ON TRUE
                  ORDER BY requested.ordinal"
        );
        let rows = tx
            .query(&sql, &[&tenant_id, &principal_refs, &submission_ids])
            .await?;
        tx.commit().await?;
        rows.iter().map(status_from_row).collect()
    }

    pub async fn own_contributor_statuses(
        &self,
        tenant_id: &str,
        principal_ref: &str,
    ) -> Result<Vec<PipelineContributorStatus>, DatabaseError> {
        self.own_contributor_statuses_page(
            tenant_id,
            principal_ref,
            None,
            PIPELINE_STATUS_BATCH_MAX,
        )
        .await
    }

    pub async fn own_contributor_statuses_page(
        &self,
        tenant_id: &str,
        principal_ref: &str,
        after: Option<(DateTime<Utc>, Uuid)>,
        limit: usize,
    ) -> Result<Vec<PipelineContributorStatus>, DatabaseError> {
        if limit == 0 || limit > PIPELINE_STATUS_BATCH_MAX {
            return Err(DatabaseError::Constraint(
                "submission status page limit is invalid".to_string(),
            ));
        }
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let (after_received_at, after_submission_id) = after
            .map(|(received_at, submission_id)| (Some(received_at), Some(submission_id)))
            .unwrap_or((None, None));
        let limit = i64::try_from(limit).map_err(|_| {
            DatabaseError::Constraint("submission status page limit is invalid".to_string())
        })?;
        let rows = tx
            .query(
                "SELECT submission_id
                   FROM trace_submissions
                  WHERE tenant_id = $1 AND auth_principal_ref = $2
                    AND (
                        $3::timestamptz IS NULL
                        OR (received_at, submission_id) < ($3, $4)
                    )
                    AND EXISTS (
                        SELECT 1 FROM pipeline_runs r
                         WHERE r.tenant_id = trace_submissions.tenant_id
                           AND r.submission_id = trace_submissions.submission_id
                    )
                  ORDER BY received_at DESC, submission_id DESC
                  LIMIT $5",
                &[
                    &tenant_id,
                    &principal_ref,
                    &after_received_at,
                    &after_submission_id,
                    &limit,
                ],
            )
            .await?;
        tx.commit().await?;
        let submission_ids = rows
            .iter()
            .map(|row| row.get::<_, Uuid>("submission_id"))
            .collect::<Vec<_>>();
        self.contributor_statuses(tenant_id, principal_ref, &submission_ids)
            .await
    }

    pub async fn contributor_credit(
        &self,
        tenant_id: &str,
        principal_ref: &str,
    ) -> Result<PipelineContributorCredit, DatabaseError> {
        let statuses = self
            .own_contributor_statuses(tenant_id, principal_ref)
            .await?;
        let scored_microcredits = sum_trace_credit_atomic_units(&statuses, |_| true)?;
        let finalized_microcredits = sum_trace_credit_atomic_units(&statuses, |instrument| {
            instrument.internal_settlement_state == "finalized"
        })?;
        let pending_microcredits = sum_trace_credit_atomic_units(&statuses, |instrument| {
            matches!(
                instrument.internal_settlement_state.as_str(),
                "pending" | "approved"
            )
        })?;
        let held_microcredits = sum_trace_credit_atomic_units(&statuses, |instrument| {
            instrument.operation_state == "held"
        })?;
        Ok(PipelineContributorCredit {
            scored_microcredits,
            finalized_microcredits,
            pending_microcredits,
            held_microcredits,
            submission_count: statuses.len(),
        })
    }

    pub async fn score_attestation_entries(
        &self,
        tenant_id: &str,
        principal_ref: &str,
        submission_ids: &[Uuid],
    ) -> Result<Vec<PipelineScoreAttestationEntry>, DatabaseError> {
        if submission_ids.len() > PIPELINE_STATUS_BATCH_MAX {
            return Err(DatabaseError::Constraint(
                "score attestation batch limit exceeded".to_string(),
            ));
        }
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT r.run_id, r.submission_id, r.bundle_id,
                        score.outcome_id, score.outcome_schema_id,
                        score.outcome_schema_version, score.decision
                   FROM unnest($3::uuid[]) WITH ORDINALITY requested(submission_id, ordinal)
                   JOIN trace_submissions s
                     ON s.tenant_id = $1
                    AND s.submission_id = requested.submission_id
                    AND s.auth_principal_ref = $2
                   JOIN LATERAL (
                        SELECT pr.run_id, pr.submission_id, pr.bundle_id, pr.created_at
                          FROM pipeline_runs pr
                         WHERE pr.tenant_id = s.tenant_id
                           AND pr.submission_id = s.submission_id
                         ORDER BY pr.created_at DESC
                         LIMIT 1
                   ) r ON TRUE
                   JOIN phase_outcomes score
                     ON score.tenant_id = $1
                    AND score.run_id = r.run_id
                    AND score.phase = 'score'
                  ORDER BY requested.ordinal",
                &[&tenant_id, &principal_ref, &submission_ids],
            )
            .await?;
        tx.commit().await?;
        rows.iter().map(attestation_from_row).collect()
    }

    pub async fn own_score_attestation_entries(
        &self,
        tenant_id: &str,
        principal_ref: &str,
    ) -> Result<Vec<PipelineScoreAttestationEntry>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT r.run_id, r.submission_id, r.bundle_id,
                        score.outcome_id, score.outcome_schema_id,
                        score.outcome_schema_version, score.decision
                   FROM trace_submissions s
                   JOIN LATERAL (
                        SELECT pr.run_id, pr.submission_id, pr.bundle_id, pr.created_at
                          FROM pipeline_runs pr
                         WHERE pr.tenant_id = s.tenant_id
                           AND pr.submission_id = s.submission_id
                         ORDER BY pr.created_at DESC
                         LIMIT 1
                   ) r ON TRUE
                   JOIN phase_outcomes score
                     ON score.tenant_id = s.tenant_id
                    AND score.run_id = r.run_id
                    AND score.phase = 'score'
                  WHERE s.tenant_id = $1 AND s.auth_principal_ref = $2
                  ORDER BY r.created_at DESC, r.run_id DESC
                  LIMIT $3",
                &[
                    &tenant_id,
                    &principal_ref,
                    &(PIPELINE_STATUS_BATCH_MAX as i64),
                ],
            )
            .await?;
        tx.commit().await?;
        rows.iter().map(attestation_from_row).collect()
    }

    /// Creates an immutable snapshot of the tenant's approved revisions for
    /// `allowed_use`, at most `max_items` of them, oldest run first. A
    /// request whose `request_idempotency_key` an earlier one used gets that
    /// snapshot back unchanged, or a conflict when the request differs.
    ///
    /// An item is a complete run with a committed Review approved revision
    /// whose submission is exportable (`EXPORTABLE_SUBMISSION_PREDICATE`),
    /// meets `consent_scopes`, and, when `privacy_risk` is set, has that
    /// privacy risk. The item names the approved object and its
    /// `approved_content_hash`, never the raw request.
    ///
    /// The selection runs twice. The first run picks the candidates without
    /// a lock. Their run rows are then locked `FOR KEY SHARE` in `run_id`
    /// order (the lock the items' run foreign key takes at commit, taken
    /// first) and their submission rows `FOR SHARE` in `submission_id`
    /// order: the order `withdraw_submission` locks them in (run rows
    /// `FOR UPDATE`, then submission rows), so the two never wait for each
    /// other in a cycle. The second run, restricted to the candidates, is a
    /// new statement, so under READ COMMITTED it sees a withdrawal or a
    /// revocation that committed while this waited, and leaves that
    /// submission out. A withdrawal that starts after the locks waits for
    /// this transaction, and then invalidates the new snapshot and items
    /// itself.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_export_snapshot(
        &self,
        tenant_id: &str,
        requester_principal_ref: &str,
        request_idempotency_key: &str,
        allowed_use: TraceAllowedUse,
        consent_scopes: &PipelineExportConsentScopes,
        privacy_risk: Option<ResidualPiiRisk>,
        purpose_hash: &str,
        max_items: usize,
    ) -> Result<PipelineExportSnapshot, DatabaseError> {
        if max_items == 0 || max_items > PIPELINE_EXPORT_ITEM_MAX {
            return Err(DatabaseError::Constraint(
                "export item limit is invalid".to_string(),
            ));
        }
        if !is_sha256(request_idempotency_key) || !is_sha256(purpose_hash) {
            return Err(DatabaseError::Constraint(
                "export request metadata is invalid".to_string(),
            ));
        }
        let allowed_use_label = storage_label(allowed_use)?;
        let token_scopes = consent_allowlist_labels(&consent_scopes.token)?;
        let policy_scopes = consent_allowlist_labels(&consent_scopes.policy)?;
        let requested_scope = consent_scopes.requested.map(storage_label).transpose()?;
        let privacy_risk = privacy_risk.map(storage_label).transpose()?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&format!("{tenant_id}:{request_idempotency_key}")],
        )
        .await?;
        if let Some(existing) =
            load_snapshot_by_request_key(&tx, tenant_id, request_idempotency_key).await?
        {
            if existing.requester_principal_ref != requester_principal_ref
                || existing.allowed_use != allowed_use
                || existing.purpose_hash != purpose_hash
            {
                return Err(DatabaseError::Constraint(
                    PIPELINE_EXPORT_IDEMPOTENCY_CONFLICT.to_string(),
                ));
            }
            tx.commit().await?;
            return Ok(existing);
        }
        let limit = i64::try_from(max_items)
            .map_err(|_| DatabaseError::Constraint("export item limit is invalid".to_string()))?;
        let selection = export_selection_sql();
        let candidates = tx
            .query(
                &selection,
                &[
                    &tenant_id,
                    &allowed_use_label,
                    &None::<Vec<Uuid>>,
                    &limit,
                    &token_scopes,
                    &policy_scopes,
                    &requested_scope,
                    &privacy_risk,
                ],
            )
            .await?;
        let mut run_ids = candidates
            .iter()
            .map(|row| row.get::<_, Uuid>("run_id"))
            .collect::<Vec<_>>();
        run_ids.sort();
        tx.execute(
            "SELECT 1 FROM pipeline_runs
              WHERE tenant_id = $1 AND run_id = ANY($2)
              ORDER BY run_id
              FOR KEY SHARE",
            &[&tenant_id, &run_ids],
        )
        .await?;
        lock_submissions_for_share(
            &tx,
            tenant_id,
            candidates
                .iter()
                .map(|row| row.get::<_, Uuid>("submission_id")),
        )
        .await?;
        let rows = tx
            .query(
                &selection,
                &[
                    &tenant_id,
                    &allowed_use_label,
                    &Some(run_ids),
                    &limit,
                    &token_scopes,
                    &policy_scopes,
                    &requested_scope,
                    &privacy_risk,
                ],
            )
            .await?;
        let snapshot_id = Uuid::new_v4();
        let source_list_hash = source_list_hash(&rows);
        let snapshot_row = tx
            .query_one(
                "INSERT INTO pipeline_export_snapshots (
                    tenant_id, snapshot_id, request_idempotency_key,
                    requester_principal_ref, allowed_use, purpose_hash,
                    selection_policy_id, source_list_hash
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
                 RETURNING *",
                &[
                    &tenant_id,
                    &snapshot_id,
                    &request_idempotency_key,
                    &requester_principal_ref,
                    &allowed_use_label,
                    &purpose_hash,
                    &PIPELINE_EXPORT_SELECTION_POLICY_ID,
                    &source_list_hash,
                ],
            )
            .await?;
        // Zaki review 1, round 2, item 3: every item in one statement, from
        // column arrays in selection order (`WITH ORDINALITY` numbers them
        // from 1; the item ordinal is 0-based), and the snapshot built from
        // the rows written rather than read back.
        let column = |name: &str| {
            rows.iter()
                .map(|row| row.get::<_, Uuid>(name))
                .collect::<Vec<_>>()
        };
        let text = |name: &str| {
            rows.iter()
                .map(|row| row.get::<_, String>(name))
                .collect::<Vec<_>>()
        };
        let json = |name: &str| {
            rows.iter()
                .map(|row| row.get::<_, serde_json::Value>(name))
                .collect::<Vec<_>>()
        };
        let schema_versions = rows
            .iter()
            .map(|row| row.get::<_, i32>("outcome_schema_version"))
            .collect::<Vec<_>>();
        let mut item_rows = tx
            .query(
                "INSERT INTO pipeline_export_snapshot_items (
                    tenant_id, snapshot_id, ordinal, run_id, submission_id, trace_id,
                    registry_revision_id, source_object_ref_id, source_content_hash,
                    bundle_id, outcome_schema_id, outcome_schema_version,
                    authorized_view_schema_id, consent_scopes, allowed_uses
                 )
                 SELECT $1, $2, (item.position - 1)::INTEGER, item.run_id,
                        item.submission_id, item.trace_id, item.revision_id,
                        item.object_ref_id, item.content_hash, item.bundle_id,
                        item.schema_id, item.schema_version, $3, item.consent_scopes,
                        item.allowed_uses
                   FROM unnest(
                        $4::UUID[], $5::UUID[], $6::UUID[], $7::UUID[], $8::UUID[],
                        $9::TEXT[], $10::TEXT[], $11::TEXT[], $12::INTEGER[],
                        $13::JSONB[], $14::JSONB[]
                   ) WITH ORDINALITY AS item(
                        run_id, submission_id, trace_id, revision_id, object_ref_id,
                        content_hash, bundle_id, schema_id, schema_version,
                        consent_scopes, allowed_uses, position
                   )
                 RETURNING *",
                &[
                    &tenant_id,
                    &snapshot_id,
                    &PIPELINE_AUTHORIZED_VIEW_SCHEMA_ID,
                    &column("run_id"),
                    &column("submission_id"),
                    &column("trace_id"),
                    &column("approved_revision_id"),
                    &column("approved_object_ref_id"),
                    &text("approved_content_hash"),
                    &text("bundle_id"),
                    &text("outcome_schema_id"),
                    &schema_versions,
                    &json("consent_scopes"),
                    &json("allowed_uses"),
                ],
            )
            .await?;
        item_rows.sort_by_key(|row| row.get::<_, i32>("ordinal"));
        let items = item_rows
            .iter()
            .map(snapshot_item_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let snapshot = snapshot_from_row(&snapshot_row, items)?;
        tx.commit().await?;
        Ok(snapshot)
    }

    /// Delivers a `ready` snapshot: records `main`'s export manifest and its
    /// items for it and moves the snapshot to `complete`. A `complete`
    /// snapshot is returned unchanged. An `invalidated` snapshot, or one
    /// with an invalidated item, is refused. Only the requester that created
    /// the snapshot finds it.
    ///
    /// Delivery checks the items' submissions again: it locks their rows
    /// `FOR SHARE` in `submission_id` order and then, in a new statement,
    /// requires each still to be exportable for the snapshot's use
    /// (`EXPORTABLE_SUBMISSION_PREDICATE`), or refuses with
    /// `PIPELINE_SUBMISSION_INOPERABLE_LABEL` and records nothing. A
    /// withdrawal marks the items it reaches, but an expiry, or a
    /// revocation outside the pipeline, marks none. A withdrawal that
    /// arrives during delivery waits at the submission row, and then
    /// invalidates the delivered snapshot and the manifest's items.
    pub async fn complete_export_snapshot(
        &self,
        tenant_id: &str,
        requester_principal_ref: &str,
        snapshot_id: Uuid,
    ) -> Result<PipelineExportSnapshot, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let snapshot = load_snapshot(&tx, tenant_id, snapshot_id, requester_principal_ref)
            .await?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "pipeline_export_snapshot".to_string(),
                id: snapshot_id.to_string(),
            })?;
        if snapshot.state == "invalidated" {
            return Err(DatabaseError::Constraint(
                PIPELINE_EXPORT_SNAPSHOT_INVALIDATED.to_string(),
            ));
        }
        if snapshot.state == "complete" {
            tx.commit().await?;
            return Ok(snapshot);
        }
        if snapshot
            .items
            .iter()
            .any(|item| item.invalidation_reason.is_some())
        {
            return Err(DatabaseError::Constraint(
                PIPELINE_EXPORT_SOURCE_INVALIDATED.to_string(),
            ));
        }
        let submission_ids = snapshot
            .items
            .iter()
            .map(|item| item.submission_id)
            .collect::<Vec<_>>();
        let locked =
            lock_submissions_for_share(&tx, tenant_id, submission_ids.iter().copied()).await?;
        let allowed_use_label = storage_label(snapshot.allowed_use)?;
        let exportable = tx
            .query(
                &format!(
                    "SELECT s.submission_id
                       FROM trace_submissions s
                      WHERE s.tenant_id = $1 AND s.submission_id = ANY($3)
                        AND {EXPORTABLE_SUBMISSION_PREDICATE}"
                ),
                &[&tenant_id, &allowed_use_label, &locked],
            )
            .await?;
        if exportable.len() != locked.len() {
            return Err(DatabaseError::Constraint(
                PIPELINE_SUBMISSION_INOPERABLE_LABEL.to_string(),
            ));
        }
        let item_count = i32::try_from(snapshot.items.len())
            .map_err(|_| DatabaseError::Constraint("export item count overflow".to_string()))?;
        // `main` reads every manifest's kind back as a
        // `TraceObjectArtifactKind`, so the manifest carries one of those. Its
        // purpose code marks it as a pipeline export, which `main`'s
        // replay-dataset list and count leave out.
        let artifact_kind = storage_label(TraceObjectArtifactKind::ExportArtifact)?;
        let purpose_code = pipeline_export_manifest_purpose_code(snapshot.allowed_use)?;
        tx.execute(
            "INSERT INTO trace_export_manifests (
                tenant_id, export_manifest_id, artifact_kind, purpose_code,
                source_submission_ids, source_submission_ids_hash, item_count, generated_at
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,NOW())
             ON CONFLICT (tenant_id, export_manifest_id) DO NOTHING",
            &[
                &tenant_id,
                &snapshot_id,
                &artifact_kind,
                &purpose_code,
                &submission_ids,
                &snapshot.source_list_hash,
                &item_count,
            ],
        )
        .await?;
        // Zaki review 1, round 2, item 3: every manifest item in one
        // statement, from column arrays.
        let items = &snapshot.items;
        tx.execute(
            "INSERT INTO trace_export_manifest_items (
                tenant_id, export_manifest_id, submission_id, trace_id,
                derived_id, object_ref_id, source_status_at_export,
                source_hash_at_export
             )
             SELECT $1, $2, item.submission_id, item.trace_id, item.derived_id,
                    item.object_ref_id, 'accepted', item.content_hash
               FROM unnest($3::UUID[], $4::UUID[], $5::UUID[], $6::UUID[], $7::TEXT[])
                    AS item(submission_id, trace_id, derived_id, object_ref_id, content_hash)
             ON CONFLICT (tenant_id, export_manifest_id, submission_id) DO NOTHING",
            &[
                &tenant_id,
                &snapshot_id,
                &items
                    .iter()
                    .map(|item| item.submission_id)
                    .collect::<Vec<_>>(),
                &items.iter().map(|item| item.trace_id).collect::<Vec<_>>(),
                &items
                    .iter()
                    .map(|item| item.registry_revision_id)
                    .collect::<Vec<_>>(),
                &items
                    .iter()
                    .map(|item| item.source_object_ref_id)
                    .collect::<Vec<_>>(),
                &items
                    .iter()
                    .map(|item| item.source_content_hash.clone())
                    .collect::<Vec<_>>(),
            ],
        )
        .await?;
        let updated = tx
            .query_opt(
                "UPDATE pipeline_export_snapshots
                    SET state = 'complete', export_manifest_id = $3,
                        completed_at = COALESCE(completed_at, NOW())
                  WHERE tenant_id = $1 AND snapshot_id = $2 AND state = 'ready'
                  RETURNING *",
                &[&tenant_id, &snapshot_id, &snapshot_id],
            )
            .await?;
        // The items did not change: the submissions are locked, and the
        // check above found none invalidated. Another delivery that won the
        // race leaves no `ready` row to update; read that one back.
        let completed = match updated {
            Some(row) => snapshot_from_row(&row, snapshot.items)?,
            None => load_snapshot(&tx, tenant_id, snapshot_id, requester_principal_ref)
                .await?
                .ok_or_else(|| DatabaseError::NotFound {
                    entity: "pipeline_export_snapshot".to_string(),
                    id: snapshot_id.to_string(),
                })?,
        };
        tx.commit().await?;
        Ok(completed)
    }

    /// The rows a tenant's pipeline runs wrote into `main`'s tables, each
    /// found through its run, never by its shape (Ruling F-M10): the runs'
    /// submissions; the credit events whose `pipeline_run_id` is one of the
    /// runs; the settlement batches the runs' settlement legs carry; and the
    /// NEAR outbox lines of those batches. The pipeline writes these rows to
    /// the database only, so `main`'s DB/file reconciliation leaves them out
    /// of its file-versus-database comparisons.
    pub async fn reconciliation_rows(
        &self,
        tenant_id: &str,
    ) -> Result<PipelineReconciliationRows, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let ids = |rows: Vec<Row>| {
            rows.iter()
                .map(|row| row.get::<_, Uuid>(0))
                .collect::<BTreeSet<_>>()
        };
        let submission_ids = ids(tx
            .query(
                "SELECT DISTINCT submission_id FROM pipeline_runs WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?);
        let credit_event_ids = ids(tx
            .query(
                "SELECT l.credit_event_id
                   FROM trace_credit_ledger l
                   JOIN pipeline_runs r
                     ON r.tenant_id = l.tenant_id AND r.run_id = l.pipeline_run_id
                  WHERE l.tenant_id = $1",
                &[&tenant_id],
            )
            .await?);
        let settlement_batch_ids = ids(tx
            .query(
                "SELECT DISTINCT settlement_batch_id FROM pipeline_run_settlements
                  WHERE tenant_id = $1 AND settlement_batch_id IS NOT NULL",
                &[&tenant_id],
            )
            .await?);
        let near_outbox_ids = ids(tx
            .query(
                "SELECT o.near_outbox_id
                   FROM trace_near_credit_outbox o
                  WHERE o.tenant_id = $1
                    AND o.settlement_batch_id IN (
                        SELECT s.settlement_batch_id FROM pipeline_run_settlements s
                         WHERE s.tenant_id = $1 AND s.settlement_batch_id IS NOT NULL
                    )",
                &[&tenant_id],
            )
            .await?);
        tx.commit().await?;
        Ok(PipelineReconciliationRows {
            submission_ids,
            credit_event_ids,
            settlement_batch_ids,
            near_outbox_ids,
        })
    }

    /// The tenant-isolation and audit-immutability booleans are a live health
    /// signal, not a cache (`pipeline_control_health`).
    pub async fn operational_summary(
        &self,
        tenant_id: &str,
    ) -> Result<PipelineOperationalSummary, DatabaseError> {
        let generated_at = Utc::now();
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let work_rows = tx
            .query(
                "SELECT next_phase, state, last_error_label, COUNT(*) AS item_count,
                        GREATEST(
                            0,
                            EXTRACT(EPOCH FROM (NOW() - MIN(phase_started_at)))::bigint
                        ) AS oldest_age_seconds
                   FROM pipeline_runs
                  WHERE tenant_id = $1
                  GROUP BY next_phase, state, last_error_label
                  ORDER BY next_phase, state, last_error_label",
                &[&tenant_id],
            )
            .await?;
        let summary = tx
            .query_one(
                "SELECT
                    (SELECT COUNT(*) FROM pipeline_runs
                      WHERE tenant_id = $1 AND state = 'retry') AS retryable_errors,
                    (SELECT COUNT(*) FROM pipeline_runs
                      WHERE tenant_id = $1 AND state = 'failed') AS terminal_errors,
                    (SELECT COUNT(*) FROM pipeline_runs
                      WHERE tenant_id = $1 AND index_write_state = 'pending')
                        AS pending_index,
                    (SELECT COUNT(*) FROM pipeline_runs
                      WHERE tenant_id = $1 AND index_write_state = 'failed')
                        AS failed_index,
                    (SELECT COUNT(*) FROM pipeline_run_settlements
                      WHERE tenant_id = $1 AND operation_state = 'held')
                        AS held_credit,
                    (SELECT COUNT(*) FROM pipeline_run_settlements
                      WHERE tenant_id = $1
                        AND operation_state IN ('pending', 'retry', 'leased'))
                        AS delayed_credit,
                    (SELECT COUNT(*) FROM pipeline_index_invalidations
                      WHERE tenant_id = $1 AND state = 'pending')
                        AS pending_invalidation,
                    (SELECT COUNT(*) FROM pipeline_index_invalidations
                      WHERE tenant_id = $1 AND state = 'failed')
                        AS failed_invalidation,
                    (SELECT COUNT(*) FROM pipeline_export_snapshots
                      WHERE tenant_id = $1 AND state = 'ready')
                        AS incomplete_exports",
                &[&tenant_id],
            )
            .await?;
        let controls = pipeline_control_health(&tx, &pipeline_rls_tables()).await?;
        let near_rows = tx
            .query(
                "SELECT status, COUNT(*) AS item_count
                   FROM trace_near_credit_outbox
                  WHERE tenant_id = $1
                  GROUP BY status
                  ORDER BY status",
                &[&tenant_id],
            )
            .await?;
        tx.commit().await?;
        let work = work_rows
            .iter()
            .map(|row| {
                Ok(PipelineWorkSummary {
                    phase: row.get("next_phase"),
                    state: row.get("state"),
                    reason_label: row.get("last_error_label"),
                    count: count_from_row(row, "item_count")?,
                    oldest_age_seconds: u64::try_from(row.get::<_, i64>("oldest_age_seconds"))
                        .map_err(|_| {
                            DatabaseError::Serialization(
                                "pipeline work age is outside the supported range".to_string(),
                            )
                        })?,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        let near_outbox_by_state = near_rows
            .iter()
            .map(|row| {
                Ok((
                    row.get::<_, String>("status"),
                    count_from_row(row, "item_count")?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, DatabaseError>>()?;
        Ok(PipelineOperationalSummary {
            generated_at,
            work,
            retryable_error_count: count_from_row(&summary, "retryable_errors")?,
            terminal_error_count: count_from_row(&summary, "terminal_errors")?,
            pending_index_command_count: count_from_row(&summary, "pending_index")?,
            failed_index_command_count: count_from_row(&summary, "failed_index")?,
            held_credit_count: count_from_row(&summary, "held_credit")?,
            delayed_credit_count: count_from_row(&summary, "delayed_credit")?,
            near_outbox_by_state,
            pending_invalidation_count: count_from_row(&summary, "pending_invalidation")?,
            failed_invalidation_count: count_from_row(&summary, "failed_invalidation")?,
            incomplete_export_count: count_from_row(&summary, "incomplete_exports")?,
            tenant_isolation_control_passed: controls.tenant_isolation_passed,
            audit_immutability_control_passed: controls.audit_immutability_passed,
        })
    }

    pub async fn forensic_trace(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> Result<Option<PipelineForensicTrace>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let Some(run) = tx
            .query_opt(
                "SELECT r.run_id, r.submission_id, r.bundle_id, r.index_command_hash,
                        r.index_write_state,
                        COALESCE(i.state, 'none') AS index_invalidation_state
                   FROM pipeline_runs r
                   LEFT JOIN pipeline_index_invalidations i
                     ON i.tenant_id = r.tenant_id AND i.run_id = r.run_id
                  WHERE r.tenant_id = $1 AND r.run_id = $2",
                &[&tenant_id, &run_id],
            )
            .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let phase_rows = tx
            .query(
                "SELECT phase, outcome_id, outcome_schema_id,
                        outcome_schema_version, decision, evidence, evaluation
                   FROM phase_outcomes
                  WHERE tenant_id = $1 AND run_id = $2
                  ORDER BY CASE phase
                    WHEN 'admission' THEN 1 WHEN 'review' THEN 2
                    WHEN 'score' THEN 3 WHEN 'settle' THEN 4 END",
                &[&tenant_id, &run_id],
            )
            .await?;
        let settlement_sql = format!(
            "SELECT settlement.instrument_id,
                        settlement.atomic_units::TEXT AS atomic_units_text,
                        settlement.operation_state,
                        {INTERNAL_SETTLEMENT_STATE_CASE} AS internal_settlement_state,
                        settlement.credit_event_id, settlement.settlement_batch_id,
                        settlement.payout_rail, settlement.payout_state,
                        settlement.last_error_label
                   FROM pipeline_run_settlements settlement
                   LEFT JOIN trace_credit_settlement_batches batch
                     ON batch.tenant_id = settlement.tenant_id
                    AND batch.settlement_batch_id = settlement.settlement_batch_id
                    AND batch.instrument_id = settlement.instrument_id
                   {TRACE_CREDIT_LEDGER_JOIN}
                  WHERE settlement.tenant_id = $1 AND settlement.run_id = $2
                  ORDER BY settlement.instrument_id"
        );
        let settlement_rows = tx.query(&settlement_sql, &[&tenant_id, &run_id]).await?;
        tx.commit().await?;
        let phases = phase_rows
            .iter()
            .map(|row| {
                let version =
                    u32::try_from(row.get::<_, i32>("outcome_schema_version")).map_err(|_| {
                        DatabaseError::Serialization(
                            "outcome schema version is outside the supported range".to_string(),
                        )
                    })?;
                Ok(PipelinePhaseTrace {
                    phase: row.get("phase"),
                    outcome_id: row.get("outcome_id"),
                    outcome_schema_id: row.get("outcome_schema_id"),
                    outcome_schema_version: version,
                    decision_hash: json_hash(row.get("decision"))?,
                    evidence_hash: json_hash(row.get("evidence"))?,
                    evaluation_hash: json_hash(row.get("evaluation"))?,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        let score_outcome_id = phases
            .iter()
            .find(|phase| phase.phase == "score")
            .map(|phase| phase.outcome_id);
        let instruments = settlement_rows
            .iter()
            .map(instrument_status_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let trace_credit = instruments
            .iter()
            .find(|instrument| instrument.instrument_id == "trace_credit");
        Ok(Some(PipelineForensicTrace {
            run_id: run.get("run_id"),
            submission_id: run.get("submission_id"),
            bundle_id: run.get("bundle_id"),
            phases,
            index_command_hash: run.get("index_command_hash"),
            index_write_state: run.get("index_write_state"),
            score_outcome_id,
            credit_event_id: trace_credit.and_then(|instrument| instrument.credit_event_id),
            settlement_batch_id: trace_credit.and_then(|instrument| instrument.settlement_batch_id),
            payout_state: trace_credit
                .map(|instrument| instrument.payout_state.clone())
                .unwrap_or_else(|| "none".to_string()),
            instruments,
            index_invalidation_state: run.get("index_invalidation_state"),
        }))
    }
}

/// Sums `trace_credit` amounts across `statuses`' instruments matching
/// `predicate`. `PipelineContributorCredit`'s totals stay `u64` (they mirror
/// the Trace Credit ledger's own signed 64-bit column, bounded by
/// `pipeline_run_settlements_trace_credit_bound`), while
/// `PipelineInstrumentStatus.atomic_units` is `AtomicUnits` (a `u128`, #971)
/// -- this helper is the boundary between the two, and fails closed rather
/// than truncating if a total somehow will not fit.
fn sum_trace_credit_atomic_units(
    statuses: &[PipelineContributorStatus],
    predicate: impl Fn(&PipelineInstrumentStatus) -> bool,
) -> Result<u64, DatabaseError> {
    statuses
        .iter()
        .flat_map(|status| status.instruments.iter())
        .filter(|instrument| instrument.instrument_id == "trace_credit")
        .filter(|instrument| predicate(instrument))
        .try_fold(0u64, |total, instrument| {
            let units = u64::try_from(instrument.atomic_units.get()).map_err(|_| {
                DatabaseError::Serialization(
                    "trace credit settlement amount exceeds the ledger's range".to_string(),
                )
            })?;
            total.checked_add(units).ok_or_else(|| {
                DatabaseError::Serialization(
                    "trace credit total overflowed the ledger's range".to_string(),
                )
            })
        })
}

/// What `PipelineProductStore::reconciliation_rows` found: the identifiers
/// of the rows a tenant's pipeline runs wrote into `main`'s tables. Empty
/// for a tenant with no pipeline run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipelineReconciliationRows {
    pub submission_ids: BTreeSet<Uuid>,
    pub credit_event_ids: BTreeSet<Uuid>,
    pub settlement_batch_ids: BTreeSet<Uuid>,
    pub near_outbox_ids: BTreeSet<Uuid>,
}

/// The versioned-pipeline tables the operational summary's tenant-isolation
/// control covers: every `pipeline_*` table in `TRACE_COMMONS_RLS_TABLES`,
/// and `phase_outcomes`. Read from that list rather than named again here,
/// so a migration that adds a pipeline table to it is covered too.
fn pipeline_rls_tables() -> Vec<&'static str> {
    TRACE_COMMONS_RLS_TABLES
        .iter()
        .copied()
        .filter(|table| table.starts_with("pipeline_") || *table == "phase_outcomes")
        .collect()
}

/// `phase_outcomes`' two immutability triggers.
const PHASE_OUTCOME_IMMUTABILITY_TRIGGERS: [&str; 2] = [
    "phase_outcomes_reject_update",
    "phase_outcomes_reject_delete",
];

/// The function both immutability triggers call (V92).
const PHASE_OUTCOME_IMMUTABILITY_FUNCTION: &str = "reject_phase_outcome_mutation";

/// The operational summary's two control booleans.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineControlHealth {
    pub tenant_isolation_passed: bool,
    pub audit_immutability_passed: bool,
}

/// Reads the operational summary's two controls from the catalog, in `tx`,
/// failing closed (Ruling F-I4), and checking what makes each control work,
/// not only its flags (Zaki review 1, round 2, finding 10):
///
/// - Tenant isolation is `main`'s own RLS check
///   (`trace_corpus_rls_catalog_diagnostics`, the catalog half of
///   `trace_corpus_rls_diagnostics`) over `rls_tables`: each is a table in
///   the current schema with row-level security enabled and forced and the
///   tenant policy installed with the expected expression, and the current
///   role cannot bypass it. A name the catalog does not hold there, a
///   `USING (true)` policy, a superuser or `BYPASSRLS` role, and an empty
///   list each fail it.
/// - Audit immutability passes only when both of `phase_outcomes`'
///   immutability triggers exist on that table in the current schema, fire
///   for ordinary sessions (`tgenabled` `O` or `A`: not disabled, not
///   replica-only), and call `reject_phase_outcome_mutation`.
///
/// Public only so the runtime suite can check it against a catalog it
/// changed in a transaction it rolls back; `operational_summary` is its one
/// production caller.
#[doc(hidden)]
pub async fn pipeline_control_health(
    tx: &tokio_postgres::Transaction<'_>,
    rls_tables: &[&str],
) -> Result<PipelineControlHealth, DatabaseError> {
    let isolation =
        crate::db::postgres::trace_corpus_rls_catalog_diagnostics(tx, rls_tables).await?;
    let row = tx
        .query_one(
            "SELECT COUNT(*) = 2 AS audit_immutability_passed
               FROM pg_trigger t
               JOIN pg_class c ON c.oid = t.tgrelid
               JOIN pg_proc f ON f.oid = t.tgfoid
              WHERE c.relname = 'phase_outcomes'
                AND c.relnamespace = to_regnamespace(current_schema())
                AND NOT t.tgisinternal
                AND t.tgenabled IN ('O', 'A')
                AND t.tgname = ANY($1)
                AND f.proname = $2
                AND f.pronamespace = to_regnamespace(current_schema())",
            &[
                &PHASE_OUTCOME_IMMUTABILITY_TRIGGERS.as_slice(),
                &PHASE_OUTCOME_IMMUTABILITY_FUNCTION,
            ],
        )
        .await?;
    Ok(PipelineControlHealth {
        tenant_isolation_passed: isolation.tables_isolated(),
        audit_immutability_passed: row.get("audit_immutability_passed"),
    })
}

/// Hashes a JSON value for a hash-only operational surface: the forensic
/// trace stores this digest instead of the phase outcome's raw decision,
/// evidence, or evaluation payload.
fn json_hash(value: serde_json::Value) -> Result<String, DatabaseError> {
    serde_json::to_vec(&value)
        .map(|bytes| sha256_prefixed(&bytes))
        .map_err(|_| DatabaseError::Serialization("operational record is malformed".to_string()))
}

fn status_from_row(row: &Row) -> Result<PipelineContributorStatus, DatabaseError> {
    let run_state = PipelineRunState::from_db(row.get("state"))?;
    let current_phase = phase_from_db(row.get("next_phase"))?;
    let latest_outcome_phase = row
        .get::<_, Option<String>>("latest_outcome_phase")
        .map(|phase| phase_from_db(&phase))
        .transpose()?
        .flatten();
    let submission_status: String = row.get("submission_status");
    // Zaki review 1, round 2, finding 6: the reason a contributor sees is
    // the label of the attempt that set the run's state, else Review's own
    // rejection reason, read from `ReviewDecision`'s serialized shape
    // (`{"kind": "rejected", "reason": ...}`), else Admission's quarantine or
    // rejection reason -- but only while Admission's is the latest decision:
    // once Review has decided, a quarantine it resolved is no longer the
    // reason for anything, and a rejection is Review's.
    let latest_outcome_decision =
        row.get::<_, Option<serde_json::Value>>("latest_outcome_decision");
    let review_rejection_reason = latest_outcome_decision
        .as_ref()
        .filter(|_| latest_outcome_phase == Some(Phase::Review))
        .filter(|decision| {
            decision.get("kind").and_then(serde_json::Value::as_str) == Some("rejected")
        })
        .and_then(|decision| decision.get("reason"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let admission_is_latest = matches!(latest_outcome_phase, None | Some(Phase::Admission));
    let reason_label = row
        .get::<_, Option<String>>("last_error_label")
        .or(review_rejection_reason)
        .or_else(|| {
            (admission_is_latest && row.get::<_, String>("admission_decision") != "admit")
                .then(|| row.get::<_, Option<String>>("admission_reason"))
                .flatten()
        });
    let processing = if matches!(submission_status.as_str(), "revoked" | "purged") {
        PipelineProcessingStatus::Withdrawn
    } else if submission_status == "rejected" {
        PipelineProcessingStatus::Rejected
    } else {
        match run_state {
            PipelineRunState::Pending | PipelineRunState::Leased => {
                if reason_label.as_deref() == Some("bound_policy_suspended") {
                    PipelineProcessingStatus::Blocked
                } else {
                    PipelineProcessingStatus::Pending
                }
            }
            PipelineRunState::Retry => PipelineProcessingStatus::Retry,
            PipelineRunState::AwaitingReview => PipelineProcessingStatus::AwaitingReview,
            PipelineRunState::Complete => PipelineProcessingStatus::Complete,
            PipelineRunState::Failed => PipelineProcessingStatus::Failed,
        }
    };
    let score_outcome_id = row.get("score_outcome_id");
    let score_microcredits = row
        .get::<_, Option<serde_json::Value>>("score_decision")
        .as_ref()
        .and_then(score_microcredits);
    let instruments =
        serde_json::from_value::<Vec<PipelineInstrumentStatus>>(row.get("instrument_statuses"))
            .map_err(|_| {
                DatabaseError::Serialization("pipeline instrument status is malformed".to_string())
            })?;
    let trace_credit = instruments
        .iter()
        .find(|instrument| instrument.instrument_id == "trace_credit");
    let credit = match (score_outcome_id, score_microcredits) {
        (None, _) => PipelineCreditStatus::Unscored,
        (Some(_), Some(0)) => PipelineCreditStatus::Zero,
        (Some(_), Some(_))
            if trace_credit.is_some_and(|instrument| instrument.operation_state == "held") =>
        {
            PipelineCreditStatus::Held
        }
        (Some(_), Some(_))
            if trace_credit.is_some_and(|instrument| instrument.operation_state == "failed") =>
        {
            PipelineCreditStatus::Failed
        }
        (Some(_), Some(_))
            if trace_credit.is_some_and(|instrument| instrument.operation_state == "forfeited") =>
        {
            PipelineCreditStatus::Forfeited
        }
        (Some(_), Some(_))
            if trace_credit
                .is_some_and(|instrument| instrument.internal_settlement_state == "finalized") =>
        {
            PipelineCreditStatus::Finalized
        }
        (Some(_), Some(_))
            if trace_credit.is_some_and(|instrument| {
                instrument.internal_settlement_state == "not_settlement_eligible"
            }) =>
        {
            PipelineCreditStatus::NotSettlementEligible
        }
        (Some(_), Some(_))
            if trace_credit
                .is_some_and(|instrument| instrument.internal_settlement_state == "withheld") =>
        {
            PipelineCreditStatus::Withheld
        }
        _ => PipelineCreditStatus::Pending,
    };
    let payout = match trace_credit.map(|instrument| instrument.payout_state.as_str()) {
        None | Some("none") => None,
        Some(value) => Some(value.to_string()),
    };
    let settlement_batch_id = trace_credit.and_then(|instrument| instrument.settlement_batch_id);
    let compatibility = compatibility_status_from_row(row)?;
    let admission_decision: String = row.get("admission_decision");
    Ok(PipelineContributorStatus {
        submission_id: row.get("submission_id"),
        trace_id: row.get("trace_id"),
        run_id: row.get("run_id"),
        bundle_id: row.get("bundle_id"),
        processing,
        current_phase,
        // The phase "responsible" for the run's current state is the
        // one whose own attempt set `last_error_label` (e.g. Review's
        // `review_assessment_required` on a quarantine wait) -- not
        // unconditionally the latest phase to have committed an outcome,
        // which pairs a Review label with an Admission phase and misleads a
        // contributor into thinking Admission is still holding the run. A
        // Reject keeps its own rule (Admission or Review, from
        // `admission_decision`); with no label set, the latest committed
        // outcome's phase still applies.
        responsible_phase: if processing == PipelineProcessingStatus::Rejected {
            Some(if row.get::<_, String>("admission_decision") == "reject" {
                Phase::Admission
            } else {
                Phase::Review
            })
        } else if row.get::<_, Option<String>>("last_error_label").is_some() {
            current_phase
        } else {
            latest_outcome_phase
        },
        reason_label,
        credit,
        score_microcredits,
        score_outcome_id,
        settlement_batch_id,
        payout,
        instruments,
        compatibility,
        submission_status,
        admission_decision,
    })
}

/// Ruling T15-7: `Some` for a run whose bound bundle's Score policy is the
/// compatibility one, with the shadow credit quality its committed Score
/// recorded, once it has one.
fn compatibility_status_from_row(
    row: &Row,
) -> Result<Option<PipelineCompatibilityStatus>, DatabaseError> {
    if row
        .get::<_, Option<String>>("score_implementation_id")
        .as_deref()
        != Some(COMPATIBILITY_SCORE_IMPLEMENTATION)
    {
        return Ok(None);
    }
    let malformed =
        || DatabaseError::Serialization("pipeline score evidence is malformed".to_string());
    let count = |column: &str| {
        row.get::<_, Option<i64>>(column)
            .map(|value| u32::try_from(value).map_err(|_| malformed()))
            .transpose()
    };
    let credit_quality = match (
        row.get::<_, Option<i64>>("score_credit_quality_micros"),
        row.get::<_, Option<i32>>("score_credit_quality_version"),
    ) {
        (Some(micros), Some(version)) => Some(PipelineShadowCreditQuality {
            credit_quality_micros: u64::try_from(micros).map_err(|_| malformed())?,
            credit_quality_version: version,
            chunk_count: count("score_chunk_count")?,
            total_chunk_count: count("score_total_chunk_count")?,
            chunks_capped: row.get("score_chunks_capped"),
        }),
        _ => None,
    };
    Ok(Some(PipelineCompatibilityStatus { credit_quality }))
}

/// Reads one settlement row as an instrument status. The caller selects
/// `atomic_units::TEXT AS atomic_units_text` so this never parses a raw
/// numeric column directly (`AtomicUnits` is a `u128`, and Postgres `NUMERIC`
/// does not fit any integer type `tokio_postgres` decodes natively).
fn instrument_status_from_row(row: &Row) -> Result<PipelineInstrumentStatus, DatabaseError> {
    let atomic_units = row
        .get::<_, String>("atomic_units_text")
        .parse::<AtomicUnits>()
        .map_err(|_| {
            DatabaseError::Serialization("pipeline instrument amount is invalid".to_string())
        })?;
    Ok(PipelineInstrumentStatus {
        instrument_id: row.get("instrument_id"),
        atomic_units,
        operation_state: row.get("operation_state"),
        internal_settlement_state: row.get("internal_settlement_state"),
        credit_event_id: row.get("credit_event_id"),
        settlement_batch_id: row.get("settlement_batch_id"),
        payout_rail: row.get("payout_rail"),
        payout_state: row.get("payout_state"),
        reason_label: row.get("last_error_label"),
    })
}

fn attestation_from_row(row: &Row) -> Result<PipelineScoreAttestationEntry, DatabaseError> {
    let decision = row.get::<_, serde_json::Value>("decision");
    let scored_microcredits = score_microcredits(&decision).ok_or_else(|| {
        DatabaseError::Serialization("score outcome has no microcredit amount".to_string())
    })?;
    let version: i32 = row.get("outcome_schema_version");
    Ok(PipelineScoreAttestationEntry {
        submission_id: row.get("submission_id"),
        run_id: row.get("run_id"),
        score_outcome_id: row.get("outcome_id"),
        bundle_id: row.get("bundle_id"),
        outcome_schema_id: row.get("outcome_schema_id"),
        outcome_schema_version: u32::try_from(version).map_err(|_| {
            DatabaseError::Serialization("score outcome schema version is invalid".to_string())
        })?,
        scored_microcredits,
        decision,
    })
}

/// Locks the tenant's `submission_ids` rows `FOR SHARE`, in `submission_id`
/// order, and returns those ids sorted and without repeats.
async fn lock_submissions_for_share(
    tx: &Transaction<'_>,
    tenant_id: &str,
    submission_ids: impl Iterator<Item = Uuid>,
) -> Result<Vec<Uuid>, DatabaseError> {
    let mut submission_ids = submission_ids.collect::<Vec<_>>();
    submission_ids.sort();
    submission_ids.dedup();
    tx.execute(
        "SELECT 1 FROM trace_submissions
          WHERE tenant_id = $1 AND submission_id = ANY($2)
          ORDER BY submission_id
          FOR SHARE",
        &[&tenant_id, &submission_ids],
    )
    .await?;
    Ok(submission_ids)
}

/// The snapshot an earlier request with `request_idempotency_key` created,
/// loaded for the requester that created it.
async fn load_snapshot_by_request_key(
    tx: &Transaction<'_>,
    tenant_id: &str,
    request_idempotency_key: &str,
) -> Result<Option<PipelineExportSnapshot>, DatabaseError> {
    let row = tx
        .query_opt(
            "SELECT snapshot_id, requester_principal_ref
               FROM pipeline_export_snapshots
              WHERE tenant_id = $1 AND request_idempotency_key = $2",
            &[&tenant_id, &request_idempotency_key],
        )
        .await?;
    match row {
        Some(row) => {
            let snapshot_id = row.get("snapshot_id");
            let principal: String = row.get("requester_principal_ref");
            load_snapshot(tx, tenant_id, snapshot_id, &principal).await
        }
        None => Ok(None),
    }
}

/// The snapshot and its items, in ordinal order; `None` when the tenant has
/// no such snapshot or another requester created it.
async fn load_snapshot(
    tx: &Transaction<'_>,
    tenant_id: &str,
    snapshot_id: Uuid,
    requester_principal_ref: &str,
) -> Result<Option<PipelineExportSnapshot>, DatabaseError> {
    let Some(row) = tx
        .query_opt(
            "SELECT *
               FROM pipeline_export_snapshots
              WHERE tenant_id = $1 AND snapshot_id = $2
                AND requester_principal_ref = $3",
            &[&tenant_id, &snapshot_id, &requester_principal_ref],
        )
        .await?
    else {
        return Ok(None);
    };
    let item_rows = tx
        .query(
            "SELECT *
               FROM pipeline_export_snapshot_items
              WHERE tenant_id = $1 AND snapshot_id = $2
              ORDER BY ordinal",
            &[&tenant_id, &snapshot_id],
        )
        .await?;
    let items = item_rows
        .iter()
        .map(snapshot_item_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(snapshot_from_row(&row, items)?))
}

/// A snapshot from its `pipeline_export_snapshots` row and its items, in
/// ordinal order.
fn snapshot_from_row(
    row: &Row,
    items: Vec<PipelineExportSnapshotItem>,
) -> Result<PipelineExportSnapshot, DatabaseError> {
    Ok(PipelineExportSnapshot {
        tenant_id: row.get("tenant_id"),
        snapshot_id: row.get("snapshot_id"),
        request_idempotency_key: row.get("request_idempotency_key"),
        requester_principal_ref: row.get("requester_principal_ref"),
        allowed_use: from_storage_label(row.get("allowed_use"), "export allowed use")?,
        purpose_hash: row.get("purpose_hash"),
        selection_policy_id: row.get("selection_policy_id"),
        source_list_hash: row.get("source_list_hash"),
        state: row.get("state"),
        export_manifest_id: row.get("export_manifest_id"),
        created_at: row.get("created_at"),
        completed_at: row.get("completed_at"),
        invalidated_at: row.get("invalidated_at"),
        items,
    })
}

fn snapshot_item_from_row(row: &Row) -> Result<PipelineExportSnapshotItem, DatabaseError> {
    let ordinal: i32 = row.get("ordinal");
    let version: i32 = row.get("outcome_schema_version");
    Ok(PipelineExportSnapshotItem {
        ordinal: u32::try_from(ordinal)
            .map_err(|_| DatabaseError::Serialization("invalid export ordinal".to_string()))?,
        run_id: row.get("run_id"),
        submission_id: row.get("submission_id"),
        trace_id: row.get("trace_id"),
        registry_revision_id: row.get("registry_revision_id"),
        source_object_ref_id: row.get("source_object_ref_id"),
        source_content_hash: row.get("source_content_hash"),
        bundle_id: row.get("bundle_id"),
        outcome_schema_id: row.get("outcome_schema_id"),
        outcome_schema_version: u32::try_from(version).map_err(|_| {
            DatabaseError::Serialization("invalid export outcome schema version".to_string())
        })?,
        authorized_view_schema_id: row.get("authorized_view_schema_id"),
        consent_scopes: row.get("consent_scopes"),
        allowed_uses: row.get("allowed_uses"),
        invalidation_reason: row.get("invalidation_reason"),
    })
}

/// A hash of the selected approved revisions, in selection order.
fn source_list_hash(rows: &[Row]) -> String {
    let mut hasher = Sha256::new();
    for row in rows {
        hasher.update(row.get::<_, Uuid>("approved_revision_id").as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

/// The stored text of a snake_case enum value, as `main` stores its enums.
fn storage_label<T: Serialize>(value: T) -> Result<String, DatabaseError> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .ok_or_else(|| DatabaseError::Serialization("enum value is not a label".to_string()))
}

/// A consent-scope allowlist as the stored scope labels the selection
/// compares with, or `None` for an empty allowlist, which allows every
/// scope.
fn consent_allowlist_labels(
    allowlist: &BTreeSet<ConsentScope>,
) -> Result<Option<Vec<String>>, DatabaseError> {
    if allowlist.is_empty() {
        return Ok(None);
    }
    allowlist
        .iter()
        .map(|scope| storage_label(*scope))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// `storage_label`'s inverse; `what` names the value in the error.
fn from_storage_label<T: DeserializeOwned>(value: String, what: &str) -> Result<T, DatabaseError> {
    serde_json::from_value(serde_json::Value::String(value))
        .map_err(|_| DatabaseError::Serialization(format!("{what} is invalid")))
}

/// Reads the committed Score decision's `trace_credit` award. Under the
/// #971 settlement shape an award's `atomic_units` serializes as a decimal
/// string, not a JSON number (`AtomicUnits` holds a `u128`), so this parses
/// it as `AtomicUnits` and converts to `u64`: `trace_credit` settlement rows
/// are bounded at `i64::MAX` by `pipeline_run_settlements_trace_credit_bound`,
/// so a stored award that does not fit `u64` cannot exist.
///
/// Finding I1: `InstrumentAward::new` refuses a zero amount
/// (`ZeroInstrumentAward`), so a scored-but-zero `trace_credit` leg is
/// represented as *no* `trace_credit` entry in `awards`, not a zero-valued
/// one. A valid `awards` array with no `trace_credit` entry therefore reads
/// as `Some(0)`, not `None` -- `None` is reserved for `awards` missing or
/// malformed, or a `trace_credit` entry whose amount does not parse.
fn score_microcredits(decision: &serde_json::Value) -> Option<u64> {
    let awards = decision.get("awards")?.as_array()?;
    let Some(award) = awards.iter().find(|award| {
        award
            .get("instrument_id")
            .and_then(serde_json::Value::as_str)
            == Some("trace_credit")
    }) else {
        return Some(0);
    };
    award
        .get("atomic_units")?
        .as_str()?
        .parse::<AtomicUnits>()
        .ok()
        .and_then(|units| u64::try_from(units.get()).ok())
}

/// Reads a `COUNT(*)` column (always `bigint`/`i64` in Postgres) as the
/// unsigned count it actually represents.
fn count_from_row(row: &Row, column: &str) -> Result<u64, DatabaseError> {
    let count: i64 = row.get(column);
    u64::try_from(count)
        .map_err(|_| DatabaseError::Serialization("negative aggregate count".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_export_request_metadata_is_bounded() {
        assert!(is_sha256(&sha256_prefixed(b"request")));
        assert!(!is_sha256("request"));
        assert!(!is_sha256(&format!("sha256:{}", "A".repeat(64))));
        assert_eq!(
            storage_label(TraceAllowedUse::RankingModelTraining).unwrap(),
            "ranking_model_training"
        );
        assert_eq!(
            from_storage_label::<TraceAllowedUse>("ranking_model_training".to_string(), "use")
                .unwrap(),
            TraceAllowedUse::RankingModelTraining
        );
        assert!(from_storage_label::<TraceAllowedUse>("research".to_string(), "use").is_err());
    }

    /// The tenant-isolation control covers every versioned-pipeline table:
    /// the list read from `TRACE_COMMONS_RLS_TABLES` is the thirteen tables
    /// the migrations define.
    #[test]
    fn the_isolation_control_covers_every_pipeline_table() {
        let tables = pipeline_rls_tables().into_iter().collect::<BTreeSet<_>>();
        assert_eq!(
            tables,
            BTreeSet::from([
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
            ])
        );
    }

    #[test]
    fn pipeline_export_manifest_purpose_codes_are_one_family() {
        assert!(is_pipeline_export_manifest_purpose_code(
            "pipeline_export:evaluation"
        ));
        for other in [
            "evaluation",
            "pipeline_export",
            "pipeline_exporter:evaluation",
            "trace_commons_replay_dataset",
        ] {
            assert!(!is_pipeline_export_manifest_purpose_code(other), "{other}");
        }
    }

    #[test]
    fn score_amount_requires_the_versioned_decision_field() {
        assert_eq!(
            score_microcredits(&serde_json::json!({
                "awards": [
                    {"instrument_id": "trace_credit", "atomic_units": "7"}
                ]
            })),
            Some(7)
        );
        assert_eq!(score_microcredits(&serde_json::json!({"credit": 7})), None);
    }

    /// Finding I1: a valid, empty `awards` array, and a valid `awards`
    /// array with only a non-`trace_credit` entry, both mean "scored zero
    /// trace credit" (`Some(0)`) -- `InstrumentAward::new` refuses a zero
    /// amount, so "no `trace_credit` award" is the only way a policy can
    /// express a zero `trace_credit` leg. `None` is reserved for a missing
    /// or malformed `awards` field.
    #[test]
    fn a_missing_trace_credit_award_reads_as_zero_not_unscored() {
        assert_eq!(
            score_microcredits(&serde_json::json!({"awards": []})),
            Some(0)
        );
        assert_eq!(
            score_microcredits(&serde_json::json!({
                "awards": [
                    {"instrument_id": "storage_rebate", "atomic_units": "5"}
                ]
            })),
            Some(0)
        );
    }

    #[test]
    fn amount_fields_serialize_as_decimal_strings() {
        // 2^53 + 1: the smallest integer a JavaScript `Number` cannot hold
        // exactly, so this is the value that would actually expose a
        // regression back to a raw JSON number.
        const UNSAFE_FOR_JS_NUMBER: u64 = 9_007_199_254_740_993;

        let status = PipelineContributorStatus {
            submission_id: Uuid::nil(),
            trace_id: Uuid::nil(),
            run_id: Uuid::nil(),
            bundle_id: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            processing: PipelineProcessingStatus::Complete,
            current_phase: None,
            responsible_phase: None,
            reason_label: None,
            credit: PipelineCreditStatus::Finalized,
            score_microcredits: Some(UNSAFE_FOR_JS_NUMBER),
            score_outcome_id: Some(Uuid::nil()),
            settlement_batch_id: None,
            payout: None,
            instruments: Vec::new(),
            compatibility: None,
            submission_status: String::new(),
            admission_decision: String::new(),
        };
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(
            value["score_microcredits"],
            serde_json::json!(UNSAFE_FOR_JS_NUMBER.to_string())
        );
        let round_tripped: PipelineContributorStatus = serde_json::from_value(value).unwrap();
        assert_eq!(round_tripped, status);

        // `None` stays JSON `null`, not the string `"null"` or a number.
        let unscored = PipelineContributorStatus {
            score_microcredits: None,
            ..status.clone()
        };
        let unscored_value = serde_json::to_value(&unscored).unwrap();
        assert_eq!(
            unscored_value["score_microcredits"],
            serde_json::Value::Null
        );
        let unscored_round_tripped: PipelineContributorStatus =
            serde_json::from_value(unscored_value).unwrap();
        assert_eq!(unscored_round_tripped, unscored);

        let entry = PipelineScoreAttestationEntry {
            submission_id: Uuid::nil(),
            run_id: Uuid::nil(),
            score_outcome_id: Uuid::nil(),
            bundle_id: status.bundle_id.clone(),
            outcome_schema_id: "trace_commons.pipeline_score_outcome.v1".to_string(),
            outcome_schema_version: 1,
            scored_microcredits: UNSAFE_FOR_JS_NUMBER,
            decision: serde_json::json!({"awards": []}),
        };
        let entry_value = serde_json::to_value(&entry).unwrap();
        assert!(entry_value.get("credit_microcredits").is_none());
        assert_eq!(
            entry_value["scored_microcredits"],
            serde_json::json!(UNSAFE_FOR_JS_NUMBER.to_string())
        );
        let entry_round_tripped: PipelineScoreAttestationEntry =
            serde_json::from_value(entry_value).unwrap();
        assert_eq!(entry_round_tripped, entry);

        let credit = PipelineContributorCredit {
            scored_microcredits: UNSAFE_FOR_JS_NUMBER,
            finalized_microcredits: UNSAFE_FOR_JS_NUMBER,
            pending_microcredits: 0,
            held_microcredits: 0,
            submission_count: 1,
        };
        let credit_value = serde_json::to_value(&credit).unwrap();
        assert_eq!(
            credit_value["scored_microcredits"],
            serde_json::json!(UNSAFE_FOR_JS_NUMBER.to_string())
        );
        assert_eq!(credit_value["pending_microcredits"], serde_json::json!("0"));
        assert_eq!(
            credit_value["submission_count"],
            serde_json::json!(1),
            "submission_count is a count, not an amount, and stays a JSON number"
        );
        let credit_round_tripped: PipelineContributorCredit =
            serde_json::from_value(credit_value).unwrap();
        assert_eq!(credit_round_tripped, credit);
    }
}
