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
    PIPELINE_ADMIN_LOCK_TIMEOUT_SQL, PIPELINE_ROUTING_LOCK_SEED, lock_timeout_as,
    pipeline_routing_lock, sha256_prefixed, validate_actor,
};
use crate::versioned_pipeline_product::PipelineOperationalSummary;
use crate::versioned_pipeline_qualification::{
    PipelineQualificationStore, ProductionDependencyProfile, PromotionDecision,
};

pub const PIPELINE_RECEIPT_INTAKE_CONTAINED_LABEL: &str = "pipeline_receipt_intake_contained";
pub const PIPELINE_TENANT_NOT_SERVED_LABEL: &str = "pipeline_tenant_not_served";
pub const PIPELINE_ROUTING_UNAVAILABLE_LABEL: &str = "pipeline_routing_unavailable";
/// A new upload of a tenant whose routing row says `pipeline`, on a process
/// whose build has a code revision, when the tenant's active bundle has no
/// qualification row for that revision (review round 1, point 3): the upload
/// is refused before any work for it. Intake returns when an operator records
/// the qualification on that revision; no routing change is needed.
pub const PIPELINE_BUNDLE_NOT_QUALIFIED_LABEL: &str = "pipeline_bundle_not_qualified";
pub const ACTIVATION_ACTOR_INVALID_LABEL: &str = "activation_actor_invalid";
pub const ACTIVATION_STATE_INVALID_LABEL: &str = "activation_state_invalid";
pub const ACTIVATION_READINESS_FAILED_LABEL: &str = "activation_readiness_failed";
pub const EARLIER_QUALIFIED_BUNDLE_REQUIRED_LABEL: &str = "earlier_qualified_bundle_required";
/// A routing change that names the routing its caller expected (the record
/// id, `ExpectedRecord`, or the state, `ExpectedRouting`) and found another
/// under the routing lock: nothing is written.
pub const PIPELINE_ROUTING_STATE_CHANGED_LABEL: &str = "pipeline_routing_state_changed";
/// A `deactivate` of a contained tenant that names no expectation
/// (`RoutingExpectation`): nothing is written. A request prepared before an
/// incident's `contain` must not send the tenant back to the legacy path.
pub const PIPELINE_ROUTING_EXPECTATION_REQUIRED_LABEL: &str =
    "pipeline_routing_expectation_required";
/// A routing change (`activate`, `rollback`, `contain`, `deactivate`) that
/// waited its lock timeout (5 s, `PIPELINE_ADMIN_LOCK_TIMEOUT_SQL`) for the
/// tenant's routing lock, or for a row it locks after it, and did not get
/// it: nothing is written, and the operator repeats the request.
pub const PIPELINE_ROUTING_BUSY_LABEL: &str = "pipeline_routing_busy";
/// A statement of the legacy drain report that ran longer than
/// `LEGACY_DRAIN_STATEMENT_TIMEOUT_SQL` allows: no report is answered.
pub const LEGACY_DRAIN_REPORT_TIMEOUT_LABEL: &str = "legacy_drain_report_timeout";
/// The statement timeout of the drain report's transaction: 30 seconds for
/// each of its statements (final fix wave G27). The report's cost grows with
/// the tenant's history (a count visits every submission or batch of the
/// tenant), on a pooled connection that other tenants share.
const LEGACY_DRAIN_STATEMENT_TIMEOUT_SQL: &str = "SET LOCAL statement_timeout = '30s'";
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

/// The routing state a caller read before it asked for a `contain` or a
/// `deactivate` (final fix wave A23, the final review's G35): one of the
/// three states, or `none` for a tenant with no routing row. A change that
/// carries one is a compare-and-set: the store compares it with the state it
/// reads under the exclusive routing lock and refuses with
/// `pipeline_routing_state_changed` when they differ. The state alone does
/// not tell two containments apart; `ExpectedRecord` does, and `activate` and
/// `rollback` take only that one.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedRouting {
    Legacy,
    Pipeline,
    Contained,
    /// The tenant has no routing row.
    #[serde(rename = "none")]
    NoRow,
}

impl ExpectedRouting {
    fn is(self, current: Option<RoutingState>) -> bool {
        match self {
            Self::Legacy => current == Some(RoutingState::Legacy),
            Self::Pipeline => current == Some(RoutingState::Pipeline),
            Self::Contained => current == Some(RoutingState::Contained),
            Self::NoRow => current.is_none(),
        }
    }
}

/// The routing row a caller read before it asked for a routing change
/// (review round 1, point 2; amendment A6): the row's `activation_record_id`,
/// or `none` for a tenant with no routing row. Every change gives the row a
/// new id (V110 refuses an update that keeps it), so the id names one version
/// of the row. A change that carries it is a compare-and-set: the store
/// compares it with the row it reads under the exclusive routing lock and
/// refuses with `pipeline_routing_state_changed` when they differ. So an
/// action prepared before another operator's change cannot take effect after
/// that change returned: an `activate` prepared before a `contain` (the same
/// state after contain, reopen, contain too), or a `rollback` prepared before
/// another rollback.
///
/// In JSON it is a string: a UUID, or `none`. A `null` is neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedRecord {
    /// The tenant has no routing row.
    NoRow,
    /// The row's `activation_record_id`.
    Record(Uuid),
}

impl ExpectedRecord {
    /// The JSON string for "no routing row".
    const NO_ROW: &'static str = "none";

    /// What a caller that read `routing` names: its record id, or `NoRow`.
    pub fn of(routing: Option<&TenantRouting>) -> Self {
        routing.map_or(Self::NoRow, |routing| {
            Self::Record(routing.activation_record_id)
        })
    }

    fn is(self, current: Option<CurrentRouting>) -> bool {
        match self {
            Self::NoRow => current.is_none(),
            Self::Record(id) => current.is_some_and(|row| row.activation_record_id == id),
        }
    }
}

impl Serialize for ExpectedRecord {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::NoRow => serializer.serialize_str(Self::NO_ROW),
            Self::Record(id) => id.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for ExpectedRecord {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        // A fixed message: the parser's text never holds the value.
        let value = String::deserialize(deserializer)?;
        if value == Self::NO_ROW {
            return Ok(Self::NoRow);
        }
        Uuid::parse_str(&value)
            .map(Self::Record)
            .map_err(|_| D::Error::custom("expected_record_id_invalid"))
    }
}

/// What a `contain` or a `deactivate` expects of the routing row, each part
/// optional: the record id, the state, or both. Every part that is named must
/// hold under the routing lock, else `pipeline_routing_state_changed`. A
/// `contain` may name none (an emergency stop needs no read first). A
/// `deactivate` of a contained tenant must name one
/// (`pipeline_routing_expectation_required`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoutingExpectation {
    pub record: Option<ExpectedRecord>,
    pub state: Option<ExpectedRouting>,
}

impl RoutingExpectation {
    /// No expectation: the change applies to whatever routing the tenant has.
    pub const NONE: Self = Self {
        record: None,
        state: None,
    };

    /// An expectation of the record id alone.
    pub fn record(record: ExpectedRecord) -> Self {
        Self {
            record: Some(record),
            state: None,
        }
    }

    /// An expectation of the state alone.
    pub fn state(state: ExpectedRouting) -> Self {
        Self {
            record: None,
            state: Some(state),
        }
    }

    fn is_named(self) -> bool {
        self.record.is_some() || self.state.is_some()
    }
}

/// The routing row as a routing change reads it under the routing lock: its
/// state, and the record id that names this version of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CurrentRouting {
    state: RoutingState,
    activation_record_id: Uuid,
}

/// Refuses with `pipeline_routing_state_changed` when the caller expected a
/// record id and `current`, read under the exclusive routing lock, has
/// another, or is absent.
fn require_expected_record(
    expected: ExpectedRecord,
    current: Option<CurrentRouting>,
) -> Result<(), DatabaseError> {
    if expected.is(current) {
        Ok(())
    } else {
        Err(DatabaseError::Constraint(
            PIPELINE_ROUTING_STATE_CHANGED_LABEL.to_string(),
        ))
    }
}

/// Refuses with `pipeline_routing_state_changed` when a part of `expected`
/// that the caller named does not hold for `current`, read under the
/// exclusive routing lock. An expectation that names nothing passes.
fn require_expected_routing(
    expected: RoutingExpectation,
    current: Option<CurrentRouting>,
) -> Result<(), DatabaseError> {
    if let Some(record) = expected.record {
        require_expected_record(record, current)?;
    }
    match expected.state {
        Some(state) if !state.is(current.map(|row| row.state)) => Err(DatabaseError::Constraint(
            PIPELINE_ROUTING_STATE_CHANGED_LABEL.to_string(),
        )),
        _ => Ok(()),
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
/// stays in `pipeline_active_bundles`. `activation_record_id` is the
/// `event_id` of the event that wrote this version of the row.
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

/// A tenant's routing at one instant (`PipelineActivationStore::routing_view`):
/// its routing row (`None` for a tenant with none), its active bundle (`None`
/// for a tenant with none), whether that bundle has a qualification row for
/// the revision the caller named, and its newest routing changes, newest
/// first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TenantRoutingView {
    pub routing: Option<TenantRouting>,
    pub active_bundle_id: Option<String>,
    /// Whether `pipeline_bundle_qualifications` holds a row for the tenant,
    /// its active bundle, and the caller's revision. `None` for a tenant with
    /// no active bundle, and for a caller with no revision. It says that the
    /// row exists and nothing more: the activation gate's other terms are not
    /// read here.
    pub active_bundle_qualified_on_revision: Option<bool>,
    pub events: Vec<ActivationEvent>,
}

/// What the upload path reads to decide where a new receipt goes
/// (`PipelineActivationStore::routing_for_new_receipt`), all from one
/// statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReceiptRouting {
    /// The tenant's committed routing row; `None` for a tenant with none.
    pub routing: Option<TenantRouting>,
    /// Whether the Admission policy of the tenant's active bundle is
    /// runnable; `true` for a tenant with no active bundle.
    pub admission_runnable: bool,
    /// Whether the tenant has an active bundle and
    /// `pipeline_bundle_qualifications` holds a row for the tenant, that
    /// bundle, and the caller's revision. `false` for a caller with no
    /// revision. It proves that the row exists and nothing more: the
    /// dependency identity and the settings that the activation gate compares
    /// are checked by `activate` and `rollback` only.
    pub active_bundle_qualified: bool,
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
/// lists, a zero too, so a missing key never reads as "nothing owed". It holds
/// them in both modes of the gate driver. `not_blocking` holds the one label
/// `gate_decision_absent`, a zero too: the legacy submissions with an active
/// submitted envelope that have no gate decision, counted only when the caller
/// says that the gate driver is off (`gate_driver_enabled` false), because then
/// no code makes the decision and it is not work that the legacy path owes. `evidence_hash` is the canonical
/// hash (`evidence_hash`) of `{"schema": "trace_commons.pipeline_legacy_drain.v1",
/// "gate_driver_enabled": gate_driver_enabled, "pending": pending,
/// "not_blocking": not_blocking}`: the same counts in the same mode give the
/// same hash, and it names no time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacyDrainReport {
    pub generated_at: DateTime<Utc>,
    /// The tenant's routing row at the same snapshot; `None` for a tenant
    /// that has none.
    pub routing_state: Option<RoutingState>,
    /// The mode of this report: true when the caller said that the
    /// deployment runs the gate driver (`Some(ceiling)`), false when it said
    /// that the driver is off (`None`). The two modes of the same rows always
    /// give a different `evidence_hash`; they give different `pending` and
    /// `not_blocking` only when a legacy submission with an active submitted
    /// envelope has no gate decision.
    pub gate_driver_enabled: bool,
    /// What the legacy path owes. Every count in it blocks `drained`.
    pub pending: BTreeMap<String, u64>,
    /// What the report shows and does not count as owed. It never changes
    /// `drained`.
    pub not_blocking: BTreeMap<String, u64>,
    /// True only when every count in `pending` is zero. `not_blocking` is not
    /// read.
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

/// Where a count's number goes in the report, and in which mode of the gate
/// driver the statement runs. A statement that does not run for the caller's
/// mode leaves its label at 0 in the map it belongs to.
#[derive(Clone, Copy)]
enum DrainCounted {
    /// Into `pending`, in both modes.
    Blocking,
    /// Into `pending`, only when the caller says that the deployment runs the
    /// gate driver (`Some(ceiling)`): the statement takes that ceiling.
    BlockingWhenGateDriverRuns,
    /// Into `not_blocking`, only when the caller says that the gate driver is
    /// off (`None`).
    NotBlockingWhenGateDriverIsOff,
}

/// One count of the drain report: its label, its statement, its parameters,
/// and where its number goes.
struct LegacyDrainCount {
    label: &'static str,
    sql: &'static str,
    params: DrainParams,
    counted: DrainCounted,
}

/// The ten counts that block `drained`, each the predicate of the legacy
/// worker that does the work, and the one that does not, `gate_decision_absent`
/// (the two gate counts without their attempts predicate: it runs only when
/// the gate driver is off, so no worker does that work). Each has the tenant
/// filter and "no pipeline run owns the submission"
/// (`NOT EXISTS ... pipeline_runs`, the exclusion
/// `list_submissions_needing_gate_decision` applies too: a pipeline receipt
/// also writes a `trace_submissions` row, in the status `received`,
/// `quarantined`, `rejected`, or, after a review approval, `accepted`). Every
/// statement is one `SELECT COUNT(*)` over a tenant predicate: no row is
/// locked and nothing is written. The mode of the gate driver decides which
/// statements about a missing gate decision run (`DrainCounted`): with the
/// driver on, `gate_decision_pending` and `gate_decision_exhausted`; with it
/// off, `gate_decision_absent` alone. A statement that does not run leaves its
/// label at 0.
///
/// Where a count cannot select exactly what its worker selects for the
/// legacy-owned submissions, it selects more, never less: a count that reads
/// zero while the legacy path still owes work is the failure the report exists
/// to prevent. The comments say where. (The legacy workers do not themselves
/// leave out a submission with a pipeline run, except the gate driver; the
/// report does, because that work belongs to the pipeline.)
const LEGACY_DRAIN_COUNTS: [LegacyDrainCount; 11] = [
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
        counted: DrainCounted::Blocking,
    },
    // `list_submissions_needing_gate_decision` (db/postgres.rs), without its
    // backoff clause: a submission that waits out a backoff is still owed its
    // decision. Attempts below the ceiling the gate driver runs with
    // (`run_perplexity_score_driver_tick`'s `max_attempts`). The envelope ref
    // is an `EXISTS`, not the worker's join: the join fans out to one row for
    // each active ref and the worker deduplicates it with `DISTINCT`. Taken
    // only when the caller says that the deployment runs the gate driver.
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
        counted: DrainCounted::BlockingWhenGateDriverRuns,
    },
    // The same query with the attempts at or above the ceiling: the driver
    // no longer selects the submission, and no decision exists. Without an
    // operator resetting its attempt row it never gets one. Taken only when the
    // caller says that the deployment runs the gate driver.
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
        counted: DrainCounted::BlockingWhenGateDriverRuns,
    },
    // The same query with no attempts predicate: the submissions with an
    // active submitted envelope and no gate decision, whatever their attempts,
    // which is the sum of the two statements above under any ceiling. It runs
    // instead of them when the deployment does not run the gate driver: no
    // code then makes the decision, so it is not work that the legacy path
    // owes, and the number goes into `not_blocking`. It is its own statement,
    // not one of the two above with a ceiling that stands for "none": no
    // sentinel ceiling is right for every attempt count.
    LegacyDrainCount {
        label: "gate_decision_absent",
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
                 AND NOT EXISTS (
                     SELECT 1 FROM pipeline_runs r
                      WHERE r.tenant_id = s.tenant_id AND r.submission_id = s.submission_id
                 )",
        params: DrainParams::Tenant,
        counted: DrainCounted::NotBlockingWhenGateDriverIsOff,
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
        counted: DrainCounted::Blocking,
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
        counted: DrainCounted::Blocking,
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
        counted: DrainCounted::Blocking,
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
        counted: DrainCounted::Blocking,
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
        counted: DrainCounted::Blocking,
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
        counted: DrainCounted::Blocking,
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
        counted: DrainCounted::Blocking,
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

/// An operator's request to select a qualified bundle for one tenant's new
/// runs (`PipelineActivationStore::activate_tenant`, `rollback_bundle`).
/// `tenant_id` is the credential's tenant, never a second tenant; the actor
/// is the credential's `principal_ref`. `promotion` is the decision over the
/// bundle's verified check results, evaluated just before the call;
/// `runtime_code_revision_hash` is the revision the deployed code was built
/// from (`DEPLOYED_CODE_REVISION_HASH`); `dependencies` is the bundle's
/// profile on the running service. `expected_record_id` is the routing row
/// the caller read before the request (`ExpectedRecord`): the change is
/// refused with `pipeline_routing_state_changed` unless the row under the
/// routing lock is that one. It is not optional: an `activate` or a
/// `rollback` with no expectation could reopen a contained tenant's intake.
#[derive(Debug, Clone, Copy)]
pub struct ActivationRequest<'a> {
    pub tenant_id: &'a str,
    pub bundle_id: &'a str,
    pub actor_principal_ref: &'a str,
    pub reason_code: &'a str,
    pub promotion: &'a PromotionDecision,
    pub runtime_code_revision_hash: &'a str,
    pub dependencies: &'a ProductionDependencyProfile,
    pub expected_record_id: ExpectedRecord,
}

/// The evidence hash that a routing change which selects a bundle records, in
/// its routing row and in its event (final fix wave G12). One rule for
/// `activate` and `rollback`: the canonical hash (`evidence_hash`,
/// `to_canonical_vec`) of the evidence the change rests on and the bundle it
/// selects, so the record names the activation and not only a tenant's
/// quiet state. An activation: `{"readiness": readiness.evidence_hash,
/// "promotion": promotion.evidence_hash, "bundle_id": bundle_id}`. A rollback
/// reads no readiness, so it has no `readiness` key: `{"promotion":
/// promotion.evidence_hash, "bundle_id": bundle_id}`.
fn bundle_selection_evidence_hash(
    readiness: Option<&ActivationReadiness>,
    promotion: &PromotionDecision,
    bundle_id: &str,
) -> Result<String, DatabaseError> {
    let mut evidence = serde_json::json!({
        "promotion": promotion.evidence_hash,
        "bundle_id": bundle_id,
    });
    if let Some(readiness) = readiness {
        evidence["readiness"] = serde_json::json!(readiness.evidence_hash);
    }
    crate::versioned_pipeline_qualification::evidence_hash(&evidence)
        .map_err(DatabaseError::Serialization)
}

/// Takes the tenant's routing lock exclusively for the rest of `tx`. A
/// receipt's staging and commit transactions take it shared
/// (`PipelineService`), so a routing change waits for the receipts in those
/// transactions and no later one misses it.
///
/// Lock order. A receipt takes its receipt lock, its quota lock or its
/// source session row, then this lock shared, then its attempt row and its
/// Admission policy row `FOR SHARE`. A routing change takes this lock first,
/// before any read, and after it only the tenant's routing row and events,
/// the activated bundle's four policy rows `FOR SHARE`, and the active
/// bundle row `FOR UPDATE` (`activate_qualified_bundle_in`): never a receipt
/// lock, a quota lock, or a session row. A policy intervention takes only its
/// policy row (`FOR UPDATE`). So no two of these wait for each other in
/// opposite orders.
///
/// The wait is bounded (final fix wave G15): this sets
/// `PIPELINE_ADMIN_LOCK_TIMEOUT_SQL` for the rest of `tx`, so the routing
/// lock, and each row a routing change locks after it, is waited for at most
/// 5 seconds. A wait that runs out fails the statement with
/// `lock_not_available`, which each routing change answers as
/// `pipeline_routing_busy` (`routing_busy`). A queued exclusive request holds
/// back every later receipt of the tenant, each on a pooled connection, so an
/// operator's action must not queue without a bound. A receipt sets no
/// timeout: it never fails because a routing change waits.
async fn lock_routing(tx: &Transaction<'_>, tenant_id: &str) -> Result<(), DatabaseError> {
    tx.batch_execute(PIPELINE_ADMIN_LOCK_TIMEOUT_SQL).await?;
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

/// A routing change's lock wait that ran out, as `pipeline_routing_busy`.
fn routing_busy(error: DatabaseError) -> DatabaseError {
    lock_timeout_as(error, PIPELINE_ROUTING_BUSY_LABEL)
}

/// The tenant's routing state and record id in `tx`; `None` for a tenant with
/// no row. A stored state that does not decode is an error, never `None`.
async fn routing_state_in(
    tx: &Transaction<'_>,
    tenant_id: &str,
) -> Result<Option<CurrentRouting>, DatabaseError> {
    tx.query_opt(
        "SELECT routing_state, activation_record_id
           FROM pipeline_tenant_routing WHERE tenant_id = $1",
        &[&tenant_id],
    )
    .await?
    .map(|row| {
        Ok(CurrentRouting {
            state: RoutingState::from_db(row.get::<_, String>("routing_state").as_str())?,
            activation_record_id: row.get("activation_record_id"),
        })
    })
    .transpose()
}

/// Writes the routing row and its event in the caller's transaction. The
/// caller holds the routing lock (`lock_routing`) and passes `previous`, the
/// state it read under that lock (`routing_state_in`; `None` for a tenant
/// with no row), so the event's previous state is the state the caller
/// checked and the row is not read a second time. The row has no bundle
/// column; the two bundle ids belong to the event only.
///
/// The row and its event share one id (`activation_record_id` is the event's
/// `event_id`) and one generation: V110's row trigger sets the row's
/// `routing_generation` (1 on the insert, the old value plus 1 on an update),
/// the upsert returns it, and the event insert passes it. The database
/// refuses a row without that event at the commit (a deferred foreign key,
/// and a deferred trigger that compares the state and the generation), so
/// the order here (the row, then its event) holds inside one transaction.
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
    previous: Option<RoutingState>,
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
    let previous_state = previous.map_or("unselected", RoutingState::as_db);
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
                       reason_code, evidence_hash, recorded_at, routing_generation",
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
    let routing_generation: i64 = row.get("routing_generation");
    tx.execute(
        "INSERT INTO pipeline_activation_events (
            tenant_id, event_id, action, previous_state, resulting_state,
            previous_bundle_id, resulting_bundle_id, actor_principal_ref,
            reason_code, evidence_hash, recorded_at, routing_generation
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        &[
            &tenant_id,
            &routing.activation_record_id,
            &action.as_db(),
            &previous_state,
            &resulting.as_db(),
            &previous_bundle_id,
            &resulting_bundle_id,
            &actor_principal_ref,
            &reason_code,
            &evidence_hash,
            &routing.recorded_at,
            &routing_generation,
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

    /// A tenant transaction that states its isolation level, READ COMMITTED,
    /// when it begins (`START TRANSACTION ISOLATION LEVEL READ COMMITTED`),
    /// whatever `default_transaction_isolation` the database or the role
    /// sets (final fix wave G14). A routing change reads the routing row
    /// after it waited for the routing lock, and that read must see what
    /// committed during the wait; under REPEATABLE READ it would read a
    /// snapshot taken before the wait, with no error.
    async fn tenant_transaction<'a>(
        client: &'a mut deadpool_postgres::Client,
        tenant_id: &str,
    ) -> Result<Transaction<'a>, DatabaseError> {
        let tx = client
            .build_transaction()
            .isolation_level(IsolationLevel::ReadCommitted)
            .start()
            .await?;
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

    /// What the upload path reads to decide where a new receipt goes (final
    /// fix wave G1; review round 1, points 3 and 4): the tenant's committed
    /// routing row, as `routing`; whether the Admission policy of the
    /// tenant's active bundle is runnable (Task 5's definition,
    /// `PgPipelineStore::policy_is_runnable`: the status row's `runnable`,
    /// and not runnable without a row); and whether the active bundle has a
    /// qualification row for `deployed_revision`, the revision the calling
    /// process was built from (`NewReceiptRouting`). One statement, by
    /// primary key, with no lock: it always returns one row, whose routing
    /// columns are null for a tenant with no routing row. A tenant with no
    /// active bundle reads `admission_runnable` true (there is no policy to
    /// refuse for, and the receipt's own transaction answers for the missing
    /// bundle) and `active_bundle_qualified` false.
    ///
    /// The caller refuses a pipeline receipt under a suspended Admission
    /// policy, or for a bundle with no qualification on its revision, before
    /// it does any work for it. This read decides nothing for good: the
    /// receipt's staging and commit transactions check the routing and the
    /// policy again, the commit under the row's lock. They read no
    /// qualification: that fact is checked here only.
    pub async fn routing_for_new_receipt(
        &self,
        tenant_id: &str,
        deployed_revision: Option<&str>,
    ) -> Result<NewReceiptRouting, DatabaseError> {
        self.upload_routing(tenant_id, None, deployed_revision)
            .await
            .map(|(_, routing)| routing)
    }

    /// `routing_for_new_receipt`, and whether a pipeline run owns
    /// `submission_id`, in the same statement (review round 1, amendment
    /// A11). A process that may not replay the tenant's pipeline receipts
    /// already asks the database, for each new upload, whether a run owns
    /// the id (`PgPipelineStore::submission_has_pipeline_run`). This read
    /// takes that read's place there and brings the routing with it, so the
    /// routing costs such a process no transaction and no statement of its
    /// own.
    pub async fn run_and_routing_for_upload(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        deployed_revision: Option<&str>,
    ) -> Result<(bool, NewReceiptRouting), DatabaseError> {
        self.upload_routing(tenant_id, Some(submission_id), deployed_revision)
            .await
    }

    /// The one statement of `routing_for_new_receipt` and
    /// `run_and_routing_for_upload`. A null `submission_id` finds no run and a
    /// null `deployed_revision` finds no qualification: a comparison with
    /// null holds for no row.
    async fn upload_routing(
        &self,
        tenant_id: &str,
        submission_id: Option<Uuid>,
        deployed_revision: Option<&str>,
    ) -> Result<(bool, NewReceiptRouting), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_one(
                "SELECT routing.routing_state, routing.activation_record_id,
                        routing.actor_principal_ref, routing.reason_code,
                        routing.evidence_hash, routing.recorded_at,
                        NOT EXISTS (
                            SELECT 1 FROM pipeline_active_bundles active
                             WHERE active.tenant_id = $1
                               AND NOT COALESCE((
                                       SELECT status.runnable
                                         FROM pipeline_bundle_policy_status status
                                        WHERE status.tenant_id = active.tenant_id
                                          AND status.bundle_id = active.bundle_id
                                          AND status.phase = 'admission'
                                   ), FALSE)
                        ) AS admission_runnable,
                        EXISTS (
                            SELECT 1 FROM pipeline_active_bundles active
                              JOIN pipeline_bundle_qualifications qualification
                                ON qualification.tenant_id = active.tenant_id
                               AND qualification.bundle_id = active.bundle_id
                               AND qualification.code_revision_hash = $3
                             WHERE active.tenant_id = $1
                        ) AS active_bundle_qualified,
                        EXISTS (
                            SELECT 1 FROM pipeline_runs run
                             WHERE run.tenant_id = $1 AND run.submission_id = $2
                        ) AS has_pipeline_run
                   FROM (SELECT 1) AS one
                   LEFT JOIN pipeline_tenant_routing routing ON routing.tenant_id = $1",
                &[&tenant_id, &submission_id, &deployed_revision],
            )
            .await?;
        tx.commit().await?;
        let routing = row
            .get::<_, Option<String>>("routing_state")
            .is_some()
            .then(|| routing_from_row(&row))
            .transpose()?;
        Ok((
            row.get("has_pipeline_run"),
            NewReceiptRouting {
                routing,
                admission_runnable: row.get("admission_runnable"),
                active_bundle_qualified: row.get("active_bundle_qualified"),
            },
        ))
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

    /// The tenant's routing row, its active bundle, and its newest `limit`
    /// routing changes (newest first), all read at one instant: one
    /// read-only, single-snapshot transaction (`REPEATABLE READ`), as
    /// `legacy_drain_report` reads, so a change that commits meanwhile is in
    /// all three or in none. Three statements, no row lock.
    ///
    /// `deployed_revision` is the revision the calling process was built
    /// from. The active bundle's statement also reads whether the bundle has
    /// a qualification row for it (`active_bundle_qualified_on_revision`),
    /// by primary key: what a new upload of a `pipeline` tenant is checked
    /// against on that process.
    pub async fn routing_view(
        &self,
        tenant_id: &str,
        limit: usize,
        deployed_revision: Option<&str>,
    ) -> Result<TenantRoutingView, DatabaseError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::read_only_tenant_transaction(&mut client, tenant_id).await?;
        let routing = tx
            .query_opt(
                "SELECT routing_state, activation_record_id, actor_principal_ref,
                        reason_code, evidence_hash, recorded_at
                   FROM pipeline_tenant_routing
                  WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?;
        let active = tx
            .query_opt(
                "SELECT active.bundle_id,
                        EXISTS (
                            SELECT 1 FROM pipeline_bundle_qualifications qualification
                             WHERE qualification.tenant_id = active.tenant_id
                               AND qualification.bundle_id = active.bundle_id
                               AND qualification.code_revision_hash = $2
                        ) AS qualified_on_revision
                   FROM pipeline_active_bundles active
                  WHERE active.tenant_id = $1",
                &[&tenant_id, &deployed_revision],
            )
            .await?;
        let active_bundle_id: Option<String> = active.as_ref().map(|row| row.get("bundle_id"));
        // No answer without a revision: `false` would say that a
        // qualification is missing, and nothing was compared.
        let active_bundle_qualified_on_revision = deployed_revision
            .and(active.as_ref())
            .map(|row| row.get("qualified_on_revision"));
        let events = tx
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
        Ok(TenantRoutingView {
            routing: routing.as_ref().map(routing_from_row).transpose()?,
            active_bundle_id,
            active_bundle_qualified_on_revision,
            events: events
                .iter()
                .map(event_from_row)
                .collect::<Result<_, _>>()?,
        })
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
    /// no bundle and no receipt that already has an owner. From any state,
    /// with no expectation: `contain_expecting` with
    /// `RoutingExpectation::NONE`.
    pub async fn contain(
        &self,
        tenant_id: &str,
        actor_principal_ref: &str,
        reason_code: &str,
    ) -> Result<TenantRouting, DatabaseError> {
        self.contain_expecting(
            tenant_id,
            actor_principal_ref,
            reason_code,
            RoutingExpectation::NONE,
        )
        .await
    }

    /// `contain`, refused with `pipeline_routing_state_changed` when
    /// `expected` names a record id or a state that is not the one the
    /// tenant has under the routing lock (`RoutingExpectation`).
    pub async fn contain_expecting(
        &self,
        tenant_id: &str,
        actor_principal_ref: &str,
        reason_code: &str,
        expected: RoutingExpectation,
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
            expected,
        )
        .await
    }

    /// Returns a tenant that has a routing row to the legacy path. A tenant
    /// that has no row, or is already `Legacy`, has nothing to deactivate.
    /// With no expectation: `deactivate_expecting` with
    /// `RoutingExpectation::NONE`, which a contained tenant refuses.
    pub async fn deactivate(
        &self,
        tenant_id: &str,
        actor_principal_ref: &str,
        reason_code: &str,
    ) -> Result<TenantRouting, DatabaseError> {
        self.deactivate_expecting(
            tenant_id,
            actor_principal_ref,
            reason_code,
            RoutingExpectation::NONE,
        )
        .await
    }

    /// `deactivate`, refused with `pipeline_routing_state_changed` when
    /// `expected` names a record id or a state that is not the one the
    /// tenant has under the routing lock (`RoutingExpectation`). That
    /// comparison comes first: a routing the caller did not expect is
    /// reported as changed, not as `activation_state_invalid`.
    ///
    /// A contained tenant needs an expectation (plan review G11): with none,
    /// the deactivation is refused with
    /// `pipeline_routing_expectation_required` and writes nothing. A
    /// deactivation prepared for a `pipeline` tenant would otherwise send a
    /// tenant that an incident contained back to the legacy path, with no
    /// gate. From `pipeline` the expectation stays optional.
    pub async fn deactivate_expecting(
        &self,
        tenant_id: &str,
        actor_principal_ref: &str,
        reason_code: &str,
        expected: RoutingExpectation,
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
            expected,
        )
        .await
    }

    /// Routes the tenant's new receipts to the pipeline with
    /// `request.bundle_id` (P5-D9), from any routing state, when the tenant's
    /// readiness passes (`evaluate_activation_readiness`,
    /// `activation_readiness_failed`) and every term of the qualified
    /// activation gate holds (`activate_qualified_bundle_in`, which names
    /// each term and its label). Runs that already exist keep their bundle.
    /// One transaction: the exclusive routing lock first, before any read;
    /// the routing row read (an unreadable row refuses); the readiness; the
    /// gate, which locks the bundle's four policy rows `FOR SHARE` and the
    /// active bundle row `FOR UPDATE` and switches the active bundle; then
    /// the routing row and an `activate` event whose evidence hash names
    /// the activation: the canonical hash of the readiness hash, the
    /// promotion hash, and the bundle (`bundle_selection_evidence_hash`). A
    /// refusal writes nothing. An actor or a reason that is not a label is
    /// `activation_actor_invalid`, before any read. A routing row under the
    /// lock that is not the one `request.expected_record_id` names is
    /// `pipeline_routing_state_changed`, before the readiness and the gate.
    pub async fn activate_tenant(
        &self,
        request: ActivationRequest<'_>,
        readiness: &ActivationReadiness,
    ) -> Result<TenantRouting, DatabaseError> {
        validate_actor(
            request.actor_principal_ref,
            request.reason_code,
            ACTIVATION_ACTOR_INVALID_LABEL,
        )?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, request.tenant_id).await?;
        let routing = Self::activate_in(&tx, request, readiness)
            .await
            .map_err(routing_busy)?;
        tx.commit().await?;
        Ok(routing)
    }

    /// `activate_tenant` inside its transaction, from the routing lock to the
    /// routing row and the event. A lock wait that runs out returns the
    /// database's error, which the caller answers as `pipeline_routing_busy`.
    async fn activate_in(
        tx: &Transaction<'_>,
        request: ActivationRequest<'_>,
        readiness: &ActivationReadiness,
    ) -> Result<TenantRouting, DatabaseError> {
        lock_routing(tx, request.tenant_id).await?;
        // Any state may be activated, by a caller that names the row it
        // read; the read refuses a row it cannot decode, before the gate,
        // and gives the event its previous state.
        let current = routing_state_in(tx, request.tenant_id).await?;
        require_expected_record(request.expected_record_id, current)?;
        let current = current.map(|row| row.state);
        let now = Utc::now();
        evaluate_activation_readiness(readiness, now).map_err(DatabaseError::Constraint)?;
        let previous_bundle_id = PipelineQualificationStore::activate_qualified_bundle_in(
            tx,
            request.tenant_id,
            request.bundle_id,
            request.promotion,
            request.runtime_code_revision_hash,
            request.dependencies,
            now,
        )
        .await?;
        let evidence_hash =
            bundle_selection_evidence_hash(Some(readiness), request.promotion, request.bundle_id)?;
        write_routing_in(
            tx,
            request.tenant_id,
            current,
            RoutingState::Pipeline,
            ActivationAction::Activate,
            previous_bundle_id.as_deref(),
            Some(request.bundle_id),
            request.actor_principal_ref,
            request.reason_code,
            &evidence_hash,
        )
        .await
    }

    /// Selects an earlier bundle for a `pipeline` or `contained` tenant's
    /// later runs (P5-D6): one that an `activate` or a `rollback` event of
    /// this tenant selected before, and that is not the active bundle now
    /// (else `earlier_qualified_bundle_required`). A tenant routed to
    /// `legacy`, or with no routing row, is `activation_state_invalid`.
    ///
    /// The routing state does not change (final fix wave A22, the owner's
    /// answer to question 8). A `pipeline` tenant stays `pipeline`: its new
    /// receipts bind the earlier bundle. A `contained` tenant stays
    /// `contained`: the active bundle switches and its new receipts stay
    /// stopped, so a rollback never reopens intake that an operator stopped.
    /// Uploads reopen only through `activate_tenant` of the bundle that is
    /// then active, which reads the tenant's readiness; the gate accepts the
    /// active bundle, so that activation changes the state alone. The bundle passes the same gate as an
    /// activation (`activate_qualified_bundle_in`); the tenant's readiness is
    /// not read, so a rollback is open while the readiness fails. Runs that
    /// already exist keep their bundle. The same transaction and lock order
    /// as `activate_tenant`; the event is `rollback`, and its evidence hash
    /// is the canonical hash of the promotion hash and the bundle (the rule
    /// of `bundle_selection_evidence_hash`, with no readiness). A refusal
    /// writes nothing. A routing row under the lock that is not the one
    /// `request.expected_record_id` names is
    /// `pipeline_routing_state_changed`, before any other check.
    pub async fn rollback_bundle(
        &self,
        request: ActivationRequest<'_>,
    ) -> Result<TenantRouting, DatabaseError> {
        validate_actor(
            request.actor_principal_ref,
            request.reason_code,
            ACTIVATION_ACTOR_INVALID_LABEL,
        )?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, request.tenant_id).await?;
        let routing = Self::rollback_in(&tx, request)
            .await
            .map_err(routing_busy)?;
        tx.commit().await?;
        Ok(routing)
    }

    /// `rollback_bundle` inside its transaction, as `activate_in`.
    async fn rollback_in(
        tx: &Transaction<'_>,
        request: ActivationRequest<'_>,
    ) -> Result<TenantRouting, DatabaseError> {
        let tenant_id = request.tenant_id;
        lock_routing(tx, tenant_id).await?;
        let current = routing_state_in(tx, tenant_id).await?;
        require_expected_record(request.expected_record_id, current)?;
        let current = current.map(|row| row.state);
        if !matches!(
            current,
            Some(RoutingState::Pipeline | RoutingState::Contained)
        ) {
            return Err(DatabaseError::Constraint(
                ACTIVATION_STATE_INVALID_LABEL.to_string(),
            ));
        }
        // The active bundle changes only under the routing lock, which this
        // transaction holds; the gate locks the row before it switches it.
        let active_bundle_id: Option<String> = tx
            .query_opt(
                "SELECT bundle_id FROM pipeline_active_bundles WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?
            .map(|row| row.get("bundle_id"));
        let selected_before: bool = tx
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM pipeline_activation_events
                      WHERE tenant_id = $1 AND resulting_bundle_id = $2
                        AND action IN ('activate', 'rollback')
                 )",
                &[&tenant_id, &request.bundle_id],
            )
            .await?
            .get(0);
        if active_bundle_id.as_deref() == Some(request.bundle_id) || !selected_before {
            return Err(DatabaseError::Constraint(
                EARLIER_QUALIFIED_BUNDLE_REQUIRED_LABEL.to_string(),
            ));
        }
        let previous_bundle_id = PipelineQualificationStore::activate_qualified_bundle_in(
            tx,
            tenant_id,
            request.bundle_id,
            request.promotion,
            request.runtime_code_revision_hash,
            request.dependencies,
            Utc::now(),
        )
        .await?;
        let evidence_hash =
            bundle_selection_evidence_hash(None, request.promotion, request.bundle_id)?;
        // A contained tenant stays contained (A22): the state the read above
        // found is the state the row keeps.
        let resulting = match current {
            Some(RoutingState::Contained) => RoutingState::Contained,
            _ => RoutingState::Pipeline,
        };
        write_routing_in(
            tx,
            tenant_id,
            current,
            resulting,
            ActivationAction::Rollback,
            previous_bundle_id.as_deref(),
            Some(request.bundle_id),
            request.actor_principal_ref,
            request.reason_code,
            &evidence_hash,
        )
        .await
    }

    /// What the legacy path still owes `tenant_id`, as ten counts in `pending`
    /// and one verdict: `drained` is true only when every count in `pending`
    /// is zero. Before a tenant's legacy writer can be retired the legacy code
    /// must have finished this work. A second map, `not_blocking`, holds a
    /// number that the report shows and does not count as owed
    /// (`gate_decision_absent`, below); it never changes `drained`. Each count
    /// is the selection of the legacy worker that does the work, left to what
    /// the pipeline does not own: by submission (no pipeline run) for the first
    /// seven counts, by payout instrument (none) for the two NEAR counts, and
    /// tenant-wide for the withdrawals. `LEGACY_DRAIN_COUNTS` names each worker
    /// function and every place a count is wider than the worker's own
    /// selection:
    ///
    /// - `awaiting_pii_backstop`: submissions held for the PII backstop's
    ///   verdict.
    /// - `gate_decision_pending`, only with the gate driver on: submissions
    ///   with an active submitted envelope and no gate decision whose attempts
    ///   are below `gate_driver_max_attempts`
    ///   (`list_submissions_needing_gate_decision`, with no backoff and no
    ///   status filter).
    /// - `gate_decision_exhausted`, only with the gate driver on: the same with
    ///   attempts at or above the ceiling, which the gate driver no longer
    ///   selects.
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
    /// The gate driver (owner decision, 2026-10-02). A missing gate decision
    /// is owed work only in a deployment that runs the gate driver. With the
    /// driver off, which is the default
    /// (`TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED`), no code ever makes the
    /// decision, so counting it as owed would keep `drained` false for good,
    /// and turning the driver on only to drain would have a side effect: a
    /// gate run also awards `NoveltyUtility` credit. The caller says which
    /// mode applies with `gate_driver_max_attempts`:
    ///
    /// - `Some(ceiling)`: the deployment runs the gate driver with that
    ///   attempt ceiling. `gate_decision_pending` and `gate_decision_exhausted`
    ///   are counted as above, `not_blocking["gate_decision_absent"]` is 0, and
    ///   `gate_driver_enabled` is true. `gate_decision_pending` is every legacy
    ///   submission below the ceiling with an envelope and no decision,
    ///   accepted or not, and legacy code clears it: the in-process gate
    ///   driver, or a `/v1/workers/gate/evaluate` call that names the
    ///   submission, scores it. It is therefore not listed below among the
    ///   counts that wait for a person; `gate_decision_exhausted` is.
    /// - `None`: the gate driver is off. `gate_decision_pending` and
    ///   `gate_decision_exhausted` are 0. `not_blocking["gate_decision_absent"]`
    ///   is the number of legacy submissions with an active submitted envelope
    ///   and no gate decision, whatever their attempts (the sum of what the two
    ///   labels count under any ceiling), and it does not block `drained`.
    ///   `gate_driver_enabled` is false. The count leaves out a submission that
    ///   a pipeline run owns, as every count does.
    ///
    /// Both maps hold their labels in both modes, a zero too. The other eight
    /// counts in `pending` do not depend on the mode.
    ///
    /// What the driver-off report does not claim: it does not say that the
    /// submissions have a gate decision, because they have none. It says only
    /// that no legacy code owes one. Nothing in the driver-off report makes the
    /// decision, and `/v1/workers/gate/evaluate` can still score a submission
    /// that its caller names, in either mode. An operator who wants the
    /// decisions turns the gate driver on and reads the report with `Some`.
    ///
    /// Counts that can stay above zero without legacy code ever clearing
    /// them, so that an operator reads them as work for a person:
    ///
    /// - `gate_decision_exhausted` (only with the gate driver on) and
    ///   `awaiting_pii_backstop` after the attempts ran out: the driver stops,
    ///   and an operator resets the attempt row or accepts the loss.
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
    /// owed work and the report reads zero with `drained` true. The caller
    /// passes `Some` only when the deployment's gate driver is on, and `None`
    /// otherwise: the store cannot see the deployment's configuration, and a
    /// `None` for a deployment whose driver runs would leave work that the
    /// driver is about to do out of `pending`.
    ///
    /// Parameters. `tenant_id` is the tenant of the credential, never a
    /// second tenant. `gate_driver_max_attempts` is `Some(ceiling)` when the
    /// deployment runs the gate driver, with the attempt ceiling it runs with
    /// (`PerplexityDriverKnobs::max_attempts`); it splits the gate work into
    /// pending (below the ceiling) and exhausted (at it). It is `None` when the
    /// gate driver is off. `held_retention_policy_ids` is the retention
    /// policies a legal hold keeps (`legal_hold_retention_policy_ids`), as the
    /// withdrawal worker receives them: a withdrawn submission's token
    /// attachment under one of them is not owed deletion, so it is not
    /// counted. An empty list holds nothing.
    ///
    /// The report runs as the ingest runtime role in one tenant transaction:
    /// a read-only, single-snapshot transaction (`REPEATABLE READ`), so all
    /// the counts and the routing state describe one instant, with one
    /// statement for each count that the mode takes (ten with the gate driver
    /// on: the two gate statements; nine with it off: the absent statement
    /// instead of them) and no row lock.
    ///
    /// Cost and its bound (final fix wave G27). Each count visits every
    /// submission, batch, or outbox row of the tenant, so the report's cost
    /// grows with the tenant's whole history, on a pooled connection. The
    /// transaction sets `statement_timeout` to 30 seconds
    /// (`LEGACY_DRAIN_STATEMENT_TIMEOUT_SQL`): a statement that runs longer
    /// is cancelled, and the report fails with `legacy_drain_report_timeout`
    /// and answers nothing, never a partial report.
    pub async fn legacy_drain_report(
        &self,
        tenant_id: &str,
        gate_driver_max_attempts: Option<i32>,
        held_retention_policy_ids: &[String],
    ) -> Result<LegacyDrainReport, DatabaseError> {
        let gate_driver_enabled = gate_driver_max_attempts.is_some();
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::read_only_tenant_transaction(&mut client, tenant_id).await?;
        tx.batch_execute(LEGACY_DRAIN_STATEMENT_TIMEOUT_SQL).await?;
        // A statement cancelled by the timeout (`query_canceled`, 57014).
        let timed_out = |error: tokio_postgres::Error| {
            if error.code() == Some(&tokio_postgres::error::SqlState::QUERY_CANCELED) {
                DatabaseError::Constraint(LEGACY_DRAIN_REPORT_TIMEOUT_LABEL.to_string())
            } else {
                DatabaseError::from(error)
            }
        };
        let mut pending = BTreeMap::new();
        let mut not_blocking = BTreeMap::new();
        for count in &LEGACY_DRAIN_COUNTS {
            let (runs, blocks) = match count.counted {
                DrainCounted::Blocking => (true, true),
                DrainCounted::BlockingWhenGateDriverRuns => (gate_driver_enabled, true),
                DrainCounted::NotBlockingWhenGateDriverIsOff => (!gate_driver_enabled, false),
            };
            let counted = if runs {
                let params: &[&(dyn ToSql + Sync)] = match (count.params, &gate_driver_max_attempts)
                {
                    (DrainParams::Tenant, _) => &[&tenant_id],
                    (DrainParams::TenantAndGateCeiling, Some(ceiling)) => &[&tenant_id, ceiling],
                    // The table runs a statement that takes the ceiling
                    // only for a caller that gave one.
                    (DrainParams::TenantAndGateCeiling, None) => {
                        return Err(DatabaseError::Serialization(
                            "legacy_drain_gate_ceiling_missing".to_string(),
                        ));
                    }
                    (DrainParams::TenantAndHeldRetentionPolicies, _) => {
                        &[&tenant_id, &held_retention_policy_ids]
                    }
                };
                let counted: i64 = tx
                    .query_one(count.sql, params)
                    .await
                    .map_err(timed_out)?
                    .get(0);
                u64::try_from(counted).map_err(|_| {
                    DatabaseError::Serialization("legacy_drain_count_invalid".to_string())
                })?
            } else {
                0
            };
            let shown = if blocks {
                &mut pending
            } else {
                &mut not_blocking
            };
            shown.insert(count.label.to_string(), counted);
        }
        let routing_state = tx
            .query_opt(
                "SELECT routing_state FROM pipeline_tenant_routing WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await
            .map_err(timed_out)?
            .map(|row| RoutingState::from_db(row.get::<_, String>("routing_state").as_str()))
            .transpose()?;
        tx.commit().await?;
        let drained = pending.values().all(|count| *count == 0);
        let evidence_hash =
            crate::versioned_pipeline_qualification::evidence_hash(&serde_json::json!({
                "schema": LEGACY_DRAIN_EVIDENCE_SCHEMA,
                "gate_driver_enabled": gate_driver_enabled,
                "pending": pending,
                "not_blocking": not_blocking,
            }))
            .map_err(DatabaseError::Serialization)?;
        Ok(LegacyDrainReport {
            generated_at: Utc::now(),
            routing_state,
            gate_driver_enabled,
            pending,
            not_blocking,
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
        expected: RoutingExpectation,
    ) -> Result<TenantRouting, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let routing = Self::change_routing_in(
            &tx,
            tenant_id,
            actor_principal_ref,
            reason_code,
            action,
            resulting,
            expected,
        )
        .await
        .map_err(routing_busy)?;
        tx.commit().await?;
        Ok(routing)
    }

    /// `change_routing` inside its transaction, as `activate_in`.
    async fn change_routing_in(
        tx: &Transaction<'_>,
        tenant_id: &str,
        actor_principal_ref: &str,
        reason_code: &str,
        action: ActivationAction,
        resulting: RoutingState,
        expected: RoutingExpectation,
    ) -> Result<TenantRouting, DatabaseError> {
        lock_routing(tx, tenant_id).await?;
        let current = routing_state_in(tx, tenant_id).await?;
        require_expected_routing(expected, current)?;
        let current = current.map(|row| row.state);
        // A contained tenant leaves containment for the legacy path only at
        // the word of a caller that names what it read (G11).
        if action == ActivationAction::Deactivate
            && current == Some(RoutingState::Contained)
            && !expected.is_named()
        {
            return Err(DatabaseError::Constraint(
                PIPELINE_ROUTING_EXPECTATION_REQUIRED_LABEL.to_string(),
            ));
        }
        let active_bundle_id: Option<String> = tx
            .query_opt(
                "SELECT bundle_id FROM pipeline_active_bundles WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?
            .map(|row| row.get("bundle_id"));
        if action == ActivationAction::Deactivate
            && matches!(current, None | Some(RoutingState::Legacy))
        {
            return Err(DatabaseError::Constraint(
                ACTIVATION_STATE_INVALID_LABEL.to_string(),
            ));
        }
        let evidence_hash = sha256_prefixed(
            format!(
                "trace_commons.pipeline_{}.v1\0{tenant_id}\0{reason_code}",
                action.as_db()
            )
            .as_bytes(),
        );
        write_routing_in(
            tx,
            tenant_id,
            current,
            resulting,
            action,
            active_bundle_id.as_deref(),
            active_bundle_id.as_deref(),
            actor_principal_ref,
            reason_code,
            &evidence_hash,
        )
        .await
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

    /// `expected_record_id` in JSON: a UUID string or `none`. A `null`, a
    /// state name, another word, and a number are refused, and the refusal's
    /// text for a string never holds the value.
    #[test]
    fn an_expected_record_is_a_uuid_or_none() {
        let id = Uuid::new_v4();
        assert_eq!(
            serde_json::from_value::<ExpectedRecord>(serde_json::json!("none")).unwrap(),
            ExpectedRecord::NoRow
        );
        assert_eq!(
            serde_json::from_value::<ExpectedRecord>(serde_json::json!(id)).unwrap(),
            ExpectedRecord::Record(id)
        );
        for expected in [ExpectedRecord::NoRow, ExpectedRecord::Record(id)] {
            let json = serde_json::to_value(expected).unwrap();
            assert!(json.is_string(), "{json}");
            assert_eq!(
                serde_json::from_value::<ExpectedRecord>(json).unwrap(),
                expected
            );
        }
        for refused in [
            serde_json::Value::Null,
            serde_json::json!("contained"),
            serde_json::json!("None"),
            serde_json::json!(""),
            serde_json::json!(17),
        ] {
            let error = serde_json::from_value::<ExpectedRecord>(refused.clone())
                .expect_err("neither a UUID nor none");
            if let Some(text) = refused.as_str().filter(|text| !text.is_empty()) {
                assert!(!error.to_string().contains(text), "{error}");
            }
        }
    }

    /// The comparison of an expectation with the row under the lock: the
    /// record id names one version of the row, `NoRow` only a tenant with no
    /// row, and each named part of a `RoutingExpectation` must hold.
    #[test]
    fn an_expectation_holds_only_for_the_row_it_names() {
        let row = CurrentRouting {
            state: RoutingState::Contained,
            activation_record_id: Uuid::new_v4(),
        };
        let this = ExpectedRecord::Record(row.activation_record_id);
        let other = ExpectedRecord::Record(Uuid::new_v4());
        let changed = |result: Result<(), DatabaseError>| {
            matches!(result, Err(DatabaseError::Constraint(label))
                if label == PIPELINE_ROUTING_STATE_CHANGED_LABEL)
        };
        assert!(require_expected_record(this, Some(row)).is_ok());
        assert!(require_expected_record(ExpectedRecord::NoRow, None).is_ok());
        assert!(changed(require_expected_record(other, Some(row))));
        assert!(changed(require_expected_record(this, None)));
        assert!(changed(require_expected_record(
            ExpectedRecord::NoRow,
            Some(row)
        )));

        assert!(require_expected_routing(RoutingExpectation::NONE, Some(row)).is_ok());
        assert!(require_expected_routing(RoutingExpectation::NONE, None).is_ok());
        let both = |record, state| RoutingExpectation {
            record: Some(record),
            state: Some(state),
        };
        assert!(
            require_expected_routing(both(this, ExpectedRouting::Contained), Some(row)).is_ok()
        );
        assert!(changed(require_expected_routing(
            both(other, ExpectedRouting::Contained),
            Some(row)
        )));
        assert!(changed(require_expected_routing(
            both(this, ExpectedRouting::Pipeline),
            Some(row)
        )));
        assert!(changed(require_expected_routing(
            RoutingExpectation::state(ExpectedRouting::NoRow),
            Some(row)
        )));
        assert!(changed(require_expected_routing(
            RoutingExpectation::record(other),
            Some(row)
        )));
        assert!(RoutingExpectation::record(this).is_named());
        assert!(RoutingExpectation::state(ExpectedRouting::Contained).is_named());
        assert!(!RoutingExpectation::NONE.is_named());
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

    /// The table pairs each statement with the mode that supplies its
    /// parameters: a statement that takes the gate ceiling (`$2`) is taken
    /// only when the caller gave one, and no other statement is; and exactly
    /// one entry goes into `not_blocking`.
    #[test]
    fn the_drain_table_takes_the_gate_ceiling_only_with_the_gate_driver() {
        let mut not_blocking = Vec::new();
        for count in &LEGACY_DRAIN_COUNTS {
            assert_eq!(
                matches!(count.params, DrainParams::TenantAndGateCeiling),
                matches!(count.counted, DrainCounted::BlockingWhenGateDriverRuns),
                "{}: a statement takes the ceiling if and only if it is counted with the gate driver",
                count.label
            );
            if matches!(count.counted, DrainCounted::NotBlockingWhenGateDriverIsOff) {
                not_blocking.push(count.label);
            }
        }
        assert_eq!(not_blocking, vec!["gate_decision_absent"]);
    }
}
