//! Phase 8 production switch, tenant activation, rollback, and retirement.
//!
//! The switch assigns each receipt to one implementation. Retries look up that
//! owner first. Activation records select the bundle explicitly. Timestamps are
//! audit metadata and do not choose the active bundle.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::error::DatabaseError;
use crate::versioned_pipeline::{
    LEGACY_RECEIPT_OWNED_LABEL, PIPELINE_CONTAINED_LABEL, PIPELINE_NOT_ACTIVE_LABEL,
    PipelineReceiptResult, PipelineService, sha256_prefixed,
};
use crate::versioned_pipeline_credit::{PIPELINE_CREDIT_REASON, pipeline_ledger_source_key};
use crate::versioned_pipeline_product::PipelineOperationalSummary;
use crate::versioned_pipeline_qualification::{
    PipelineQualificationStore, ProductionDependencyProfile, PromotionDecision,
};
use trace_commons_gate_api::pipeline::Phase;
use trace_commons_protocol::trace_contribution::TraceContributionEnvelope;

pub const LEGACY_WRITER_DISABLED_LABEL: &str = "legacy_writer_disabled";
pub const LEGACY_WRITER_PENDING_LABEL: &str = "legacy_writer_has_pending_work";
pub const ACTIVATION_READINESS_FAILED_LABEL: &str = "activation_readiness_failed";
pub const EARLIER_QUALIFIED_BUNDLE_REQUIRED_LABEL: &str = "earlier_qualified_bundle_required";
pub const BOUND_POLICY_MUST_BE_SUSPENDED_LABEL: &str = "bound_policy_must_be_suspended";
pub const LEDGER_SOURCE_CONFLICT_LABEL: &str = "ledger_source_conflict";
pub const ACTIVATION_MAX_ERROR_COUNT: u64 = 0;
pub const ACTIVATION_MAX_WORK_AGE_SECONDS: u64 = 300;

fn is_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn is_operator_actor(value: &str) -> bool {
    (value.starts_with("operator_sha256:") || value.starts_with("admin_sha256:"))
        && value.len() <= 160
}

fn safe_reason(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

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

    fn from_db(value: &str) -> Result<Self, DatabaseError> {
        match value {
            "legacy" => Ok(Self::Legacy),
            "pipeline" => Ok(Self::Pipeline),
            "contained" => Ok(Self::Contained),
            _ => Err(DatabaseError::Serialization(
                "pipeline routing state invalid".to_string(),
            )),
        }
    }
}

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
                "pipeline receipt owner invalid".to_string(),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LegacyWriterState {
    Enabled,
    Draining,
    Disabled,
}

impl LegacyWriterState {
    fn as_db(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Draining => "draining",
            Self::Disabled => "disabled",
        }
    }

    fn from_db(value: &str) -> Result<Self, DatabaseError> {
        match value {
            "enabled" => Ok(Self::Enabled),
            "draining" => Ok(Self::Draining),
            "disabled" => Ok(Self::Disabled),
            _ => Err(DatabaseError::Serialization(
                "legacy writer state invalid".to_string(),
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TenantRouting {
    pub routing_state: RoutingState,
    pub selected_bundle_id: Option<String>,
    pub activation_record_id: Uuid,
    pub actor_principal_ref: String,
    pub reason_code: String,
    pub evidence_hash: String,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceiptOwnership {
    pub owner: ReceiptOwner,
    pub request_idempotency_key: String,
    pub request_content_hash: String,
    pub submission_id: Uuid,
    pub run_id: Option<Uuid>,
    pub ledger_source_key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SwitchedReceipt {
    Pipeline(Box<PipelineReceiptResult>),
    Legacy {
        submission_id: Uuid,
        request_content_hash: String,
        replayed: bool,
        pending_work: bool,
    },
    Contained,
    ContentConflict,
    WriterDisabled,
}

impl SwitchedReceipt {
    fn pipeline(result: PipelineReceiptResult) -> Self {
        Self::Pipeline(Box::new(result))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivationReadiness {
    pub drills_ready: bool,
    pub readiness_ok: bool,
    pub corpus_evidence_current: bool,
    pub error_count: u64,
    pub max_work_age_seconds: u64,
    pub credit_reconciled: bool,
    pub index_consistent: bool,
    pub invalidation_clear: bool,
    pub evaluated_at: DateTime<Utc>,
    pub evidence_hash: String,
}

impl ActivationReadiness {
    pub fn passing(now: DateTime<Utc>) -> Self {
        let mut ready = Self {
            drills_ready: true,
            readiness_ok: true,
            corpus_evidence_current: true,
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

    pub fn from_operational_summary(summary: &PipelineOperationalSummary) -> Self {
        let max_work_age_seconds = summary
            .work
            .iter()
            .map(|item| item.oldest_age_seconds)
            .max()
            .unwrap_or(0);
        let error_count = summary.terminal_error_count + summary.retryable_error_count;
        let mut ready = Self {
            drills_ready: true,
            readiness_ok: summary.tenant_isolation_control_passed
                && summary.audit_immutability_control_passed,
            corpus_evidence_current: true,
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
            "trace_commons.pipeline_activation_readiness.v1\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            ready.drills_ready,
            ready.readiness_ok,
            ready.corpus_evidence_current,
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
        || now - ready.evaluated_at > Duration::minutes(15)
    {
        return Err(ACTIVATION_READINESS_FAILED_LABEL.to_string());
    }
    if ready.drills_ready
        && ready.readiness_ok
        && ready.corpus_evidence_current
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

pub fn decide_new_receipt_owner(routing: Option<&TenantRouting>) -> Result<ReceiptOwner, String> {
    match routing.map(|item| item.routing_state) {
        None | Some(RoutingState::Legacy) => Ok(ReceiptOwner::Legacy),
        Some(RoutingState::Pipeline) => Ok(ReceiptOwner::Pipeline),
        Some(RoutingState::Contained) => Err(PIPELINE_CONTAINED_LABEL.to_string()),
    }
}

pub struct PipelineActivationStore {
    backend: Arc<PgBackend>,
    qualification: PipelineQualificationStore,
}

impl PipelineActivationStore {
    pub fn new(backend: Arc<PgBackend>) -> Self {
        Self {
            qualification: PipelineQualificationStore::new(backend.clone()),
            backend,
        }
    }

    async fn tenant_transaction<'a>(
        client: &'a mut deadpool_postgres::Client,
        tenant_id: &str,
    ) -> Result<deadpool_postgres::Transaction<'a>, DatabaseError> {
        let tx = client.transaction().await?;
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant_id],
        )
        .await?;
        Ok(tx)
    }

    pub async fn routing(&self, tenant_id: &str) -> Result<Option<TenantRouting>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT routing_state, selected_bundle_id, activation_record_id,
                        actor_principal_ref, reason_code, evidence_hash, recorded_at
                   FROM pipeline_tenant_routing
                  WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?;
        tx.commit().await?;
        row.map(routing_from_row).transpose()
    }

    pub async fn ownership(
        &self,
        tenant_id: &str,
        request_idempotency_key: &str,
    ) -> Result<Option<ReceiptOwnership>, DatabaseError> {
        let key_hash = sha256_prefixed(request_idempotency_key.as_bytes());
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT owner, request_idempotency_key, request_content_hash,
                        submission_id, run_id, ledger_source_key
                   FROM pipeline_receipt_ownership
                  WHERE tenant_id = $1 AND request_idempotency_key = $2",
                &[&tenant_id, &key_hash],
            )
            .await?;
        tx.commit().await?;
        row.map(ownership_from_row).transpose()
    }

    pub async fn pending_legacy_work_count(&self, tenant_id: &str) -> Result<i64, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let count: i64 = tx
            .query_one(
                "SELECT COUNT(*)::BIGINT
                   FROM pipeline_legacy_owned_work
                  WHERE tenant_id = $1 AND work_state = 'pending'",
                &[&tenant_id],
            )
            .await?
            .get(0);
        tx.commit().await?;
        Ok(count)
    }

    pub async fn phase_outcome_count_for_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<i64, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let count: i64 = tx
            .query_one(
                "SELECT COUNT(*)::BIGINT
                   FROM phase_outcomes o
                   JOIN pipeline_runs r
                     ON r.tenant_id = o.tenant_id AND r.run_id = o.run_id
                  WHERE o.tenant_id = $1 AND r.submission_id = $2",
                &[&tenant_id, &submission_id],
            )
            .await?
            .get(0);
        tx.commit().await?;
        Ok(count)
    }

    pub async fn record_legacy_receipt(
        &self,
        tenant_id: &str,
        actor_principal_ref: &str,
        request_idempotency_key: &str,
        request_bytes: &[u8],
        pending_work: bool,
        award_microcredits: u64,
    ) -> Result<SwitchedReceipt, DatabaseError> {
        if !request_idempotency_key.trim().is_empty() && request_idempotency_key.len() <= 200 {
        } else {
            return Err(DatabaseError::Constraint(
                "invalid idempotency key".to_string(),
            ));
        }
        let writer = self.legacy_writer_state(tenant_id).await?;
        if writer == LegacyWriterState::Disabled {
            return Ok(SwitchedReceipt::WriterDisabled);
        }
        if writer == LegacyWriterState::Draining && award_microcredits == 0 && !pending_work {
            // Replays of existing work continue below after the ownership lookup.
        } else if writer == LegacyWriterState::Draining {
            let existing = self.ownership(tenant_id, request_idempotency_key).await?;
            if existing.is_none() {
                return Ok(SwitchedReceipt::WriterDisabled);
            }
        }
        let request_content_hash = sha256_prefixed(request_bytes);
        let key_hash = sha256_prefixed(request_idempotency_key.as_bytes());
        let envelope: TraceContributionEnvelope = serde_json::from_slice(request_bytes)
            .map_err(|_| DatabaseError::Serialization("invalid envelope".to_string()))?;
        let ledger_source_key = pipeline_ledger_source_key(tenant_id, &key_hash);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let receipt_lock = format!("pipeline-receipt:{tenant_id}:{key_hash}");
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&receipt_lock],
        )
        .await?;
        tx.execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)
             ON CONFLICT (tenant_id) DO NOTHING",
            &[&tenant_id],
        )
        .await?;
        if let Some(existing) = tx
            .query_opt(
                "SELECT owner, request_content_hash, submission_id, run_id, ledger_source_key,
                        request_idempotency_key
                   FROM pipeline_receipt_ownership
                  WHERE tenant_id = $1 AND request_idempotency_key = $2",
                &[&tenant_id, &key_hash],
            )
            .await?
        {
            let owned = ownership_from_row(existing)?;
            if owned.request_content_hash != request_content_hash {
                tx.commit().await?;
                return Ok(SwitchedReceipt::ContentConflict);
            }
            if owned.owner != ReceiptOwner::Legacy {
                tx.commit().await?;
                return Err(DatabaseError::Constraint(
                    LEGACY_RECEIPT_OWNED_LABEL.to_string(),
                ));
            }
            let pending = tx
                .query_one(
                    "SELECT EXISTS(
                        SELECT 1 FROM pipeline_legacy_owned_work
                         WHERE tenant_id = $1 AND request_idempotency_key = $2
                           AND work_state = 'pending'
                     )",
                    &[&tenant_id, &key_hash],
                )
                .await?
                .get::<_, bool>(0);
            tx.commit().await?;
            return Ok(SwitchedReceipt::Legacy {
                submission_id: owned.submission_id,
                request_content_hash: owned.request_content_hash,
                replayed: true,
                pending_work: pending,
            });
        }
        if writer == LegacyWriterState::Disabled || writer == LegacyWriterState::Draining {
            tx.commit().await?;
            return Ok(SwitchedReceipt::WriterDisabled);
        }
        let consent_scopes = serde_json::to_value(&envelope.consent.scopes).map_err(|_| {
            DatabaseError::Serialization("trace consent scopes encode failed".to_string())
        })?;
        let allowed_uses =
            serde_json::to_value(&envelope.trace_card.allowed_uses).map_err(|_| {
                DatabaseError::Serialization("trace allowed uses encode failed".to_string())
            })?;
        let redaction_counts =
            serde_json::to_value(&envelope.privacy.redaction_counts).map_err(|_| {
                DatabaseError::Serialization("trace redaction counts encode failed".to_string())
            })?;
        tx.execute(
            "INSERT INTO trace_submissions (
                tenant_id, submission_id, trace_id, auth_principal_ref,
                schema_version, consent_policy_version, consent_scopes, allowed_uses,
                retention_policy_id, status, privacy_risk, redaction_pipeline_version,
                redaction_hash, redaction_counts, canonical_summary_hash
             ) VALUES (
                $1,$2,$3,$4,$5,$6,$7,$8,$9,'received',$10,$11,$12,$13,$14
             )",
            &[
                &tenant_id,
                &envelope.submission_id,
                &envelope.trace_id,
                &actor_principal_ref,
                &envelope.schema_version,
                &envelope.consent.policy_version,
                &consent_scopes,
                &allowed_uses,
                &envelope.trace_card.retention_policy,
                &format!("{:?}", envelope.privacy.residual_pii_risk).to_ascii_lowercase(),
                &envelope.privacy.redaction_pipeline_version,
                &envelope.privacy.redaction_hash,
                &redaction_counts,
                &envelope.privacy.redaction_hash,
            ],
        )
        .await?;
        tx.execute(
            "INSERT INTO pipeline_receipt_ownership (
                tenant_id, request_idempotency_key, request_content_hash, owner,
                submission_id, run_id, ledger_source_key
             ) VALUES ($1,$2,$3,$4,$5,NULL,$6)",
            &[
                &tenant_id,
                &key_hash,
                &request_content_hash,
                &ReceiptOwner::Legacy.as_db(),
                &envelope.submission_id,
                &ledger_source_key,
            ],
        )
        .await?;
        if pending_work {
            tx.execute(
                "INSERT INTO pipeline_legacy_owned_work (
                    tenant_id, work_id, request_idempotency_key, submission_id,
                    work_state, executor
                 ) VALUES ($1,$2,$3,$4,'pending','legacy')",
                &[
                    &tenant_id,
                    &Uuid::new_v5(
                        &Uuid::NAMESPACE_URL,
                        format!("tracecommons:legacy-work:{tenant_id}:{key_hash}").as_bytes(),
                    ),
                    &key_hash,
                    &envelope.submission_id,
                ],
            )
            .await?;
        }
        if award_microcredits > 0 {
            let event_id = Uuid::new_v5(
                &Uuid::NAMESPACE_URL,
                format!("tracecommons:legacy-credit:{tenant_id}:{key_hash}").as_bytes(),
            );
            let points_delta = format!("{}.000000", award_microcredits / 1_000_000);
            tx.execute(
                "INSERT INTO trace_credit_ledger (
                    tenant_id, credit_event_id, submission_id, trace_id, credit_account_ref,
                    event_type, points_delta, reason, external_ref, actor_principal_ref,
                    actor_role, settlement_state, ledger_source_key
                 ) VALUES (
                    $1,$2,$3,$4,$5,'accepted',$6,$7,$8,$9,'legacy_executor','pending',$10
                 )",
                &[
                    &tenant_id,
                    &event_id,
                    &envelope.submission_id,
                    &envelope.trace_id,
                    &actor_principal_ref,
                    &points_delta,
                    &PIPELINE_CREDIT_REASON,
                    &format!("legacy:{key_hash}"),
                    &actor_principal_ref,
                    &ledger_source_key,
                ],
            )
            .await
            .map_err(map_ledger_conflict)?;
        }
        tx.commit().await?;
        Ok(SwitchedReceipt::Legacy {
            submission_id: envelope.submission_id,
            request_content_hash,
            replayed: false,
            pending_work,
        })
    }

    pub async fn complete_legacy_work(
        &self,
        tenant_id: &str,
        request_idempotency_key: &str,
    ) -> Result<(), DatabaseError> {
        let key_hash = sha256_prefixed(request_idempotency_key.as_bytes());
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let updated = tx
            .execute(
                "UPDATE pipeline_legacy_owned_work
                    SET work_state = 'complete', completed_at = NOW()
                  WHERE tenant_id = $1 AND request_idempotency_key = $2
                    AND work_state = 'pending'",
                &[&tenant_id, &key_hash],
            )
            .await?;
        if updated != 1 {
            return Err(DatabaseError::NotFound {
                entity: "pipeline_legacy_owned_work".to_string(),
                id: key_hash,
            });
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn submit_switched(
        &self,
        pipeline: &PipelineService,
        tenant_id: &str,
        actor_principal_ref: &str,
        request_idempotency_key: &str,
        request_bytes: &[u8],
    ) -> anyhow::Result<SwitchedReceipt> {
        if let Some(owned) = self.ownership(tenant_id, request_idempotency_key).await? {
            let request_content_hash = sha256_prefixed(request_bytes);
            if owned.request_content_hash != request_content_hash {
                return Ok(SwitchedReceipt::ContentConflict);
            }
            return match owned.owner {
                ReceiptOwner::Legacy => {
                    let pending = self
                        .pending_legacy_for_key(tenant_id, &owned.request_idempotency_key)
                        .await?;
                    Ok(SwitchedReceipt::Legacy {
                        submission_id: owned.submission_id,
                        request_content_hash: owned.request_content_hash,
                        replayed: true,
                        pending_work: pending,
                    })
                }
                ReceiptOwner::Pipeline => Ok(SwitchedReceipt::pipeline(
                    pipeline
                        .submit(
                            tenant_id,
                            actor_principal_ref,
                            request_idempotency_key,
                            request_bytes,
                        )
                        .await?,
                )),
            };
        }
        let routing = self.routing(tenant_id).await?;
        match decide_new_receipt_owner(routing.as_ref()) {
            Err(label) if label == PIPELINE_CONTAINED_LABEL => Ok(SwitchedReceipt::Contained),
            Err(label) => anyhow::bail!(label),
            Ok(ReceiptOwner::Legacy) => Ok(self
                .record_legacy_receipt(
                    tenant_id,
                    actor_principal_ref,
                    request_idempotency_key,
                    request_bytes,
                    true,
                    0,
                )
                .await?),
            Ok(ReceiptOwner::Pipeline) => Ok(SwitchedReceipt::pipeline(
                pipeline
                    .submit(
                        tenant_id,
                        actor_principal_ref,
                        request_idempotency_key,
                        request_bytes,
                    )
                    .await?,
            )),
        }
    }

    async fn pending_legacy_for_key(
        &self,
        tenant_id: &str,
        key_hash: &str,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let pending = tx
            .query_one(
                "SELECT EXISTS(
                    SELECT 1 FROM pipeline_legacy_owned_work
                     WHERE tenant_id = $1 AND request_idempotency_key = $2
                       AND work_state = 'pending'
                 )",
                &[&tenant_id, &key_hash],
            )
            .await?
            .get::<_, bool>(0);
        tx.commit().await?;
        Ok(pending)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn activate_tenant(
        &self,
        tenant_id: &str,
        bundle_id: &str,
        actor_principal_ref: &str,
        reason_code: &str,
        promotion: &PromotionDecision,
        readiness: &ActivationReadiness,
        runtime_code_revision_hash: &str,
        dependencies: &ProductionDependencyProfile,
    ) -> Result<TenantRouting, DatabaseError> {
        self.activate_or_expand(
            tenant_id,
            bundle_id,
            actor_principal_ref,
            reason_code,
            promotion,
            readiness,
            runtime_code_revision_hash,
            dependencies,
            "activate",
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn expand_activation(
        &self,
        source_tenant_id: &str,
        tenant_id: &str,
        bundle_id: &str,
        actor_principal_ref: &str,
        reason_code: &str,
        promotion: &PromotionDecision,
        source_readiness: &ActivationReadiness,
        runtime_code_revision_hash: &str,
        dependencies: &ProductionDependencyProfile,
    ) -> Result<TenantRouting, DatabaseError> {
        let source = self.routing(source_tenant_id).await?.ok_or_else(|| {
            DatabaseError::Constraint(ACTIVATION_READINESS_FAILED_LABEL.to_string())
        })?;
        if source.routing_state != RoutingState::Pipeline {
            return Err(DatabaseError::Constraint(
                ACTIVATION_READINESS_FAILED_LABEL.to_string(),
            ));
        }
        evaluate_activation_readiness(source_readiness, Utc::now())
            .map_err(DatabaseError::Constraint)?;
        self.activate_or_expand(
            tenant_id,
            bundle_id,
            actor_principal_ref,
            reason_code,
            promotion,
            source_readiness,
            runtime_code_revision_hash,
            dependencies,
            "expand",
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn activate_or_expand(
        &self,
        tenant_id: &str,
        bundle_id: &str,
        actor_principal_ref: &str,
        reason_code: &str,
        promotion: &PromotionDecision,
        readiness: &ActivationReadiness,
        runtime_code_revision_hash: &str,
        dependencies: &ProductionDependencyProfile,
        action: &str,
    ) -> Result<TenantRouting, DatabaseError> {
        validate_actor(actor_principal_ref, reason_code)?;
        evaluate_activation_readiness(readiness, Utc::now()).map_err(DatabaseError::Constraint)?;
        self.qualification
            .activate_qualified_bundle(
                tenant_id,
                bundle_id,
                promotion,
                runtime_code_revision_hash,
                dependencies,
            )
            .await?;
        self.write_routing(
            tenant_id,
            RoutingState::Pipeline,
            Some(bundle_id),
            actor_principal_ref,
            reason_code,
            &readiness.evidence_hash,
            action,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn rollback_bundle(
        &self,
        tenant_id: &str,
        earlier_bundle_id: &str,
        actor_principal_ref: &str,
        reason_code: &str,
        promotion: &PromotionDecision,
        runtime_code_revision_hash: &str,
        dependencies: &ProductionDependencyProfile,
    ) -> Result<TenantRouting, DatabaseError> {
        validate_actor(actor_principal_ref, reason_code)?;
        let current = self.routing(tenant_id).await?;
        if current
            .as_ref()
            .and_then(|item| item.selected_bundle_id.as_deref())
            == Some(earlier_bundle_id)
        {
            return Err(DatabaseError::Constraint(
                EARLIER_QUALIFIED_BUNDLE_REQUIRED_LABEL.to_string(),
            ));
        }
        if current.as_ref().map(|item| item.routing_state) != Some(RoutingState::Pipeline)
            && current.as_ref().map(|item| item.routing_state) != Some(RoutingState::Contained)
        {
            return Err(DatabaseError::Constraint(
                EARLIER_QUALIFIED_BUNDLE_REQUIRED_LABEL.to_string(),
            ));
        }
        self.qualification
            .activate_qualified_bundle(
                tenant_id,
                earlier_bundle_id,
                promotion,
                runtime_code_revision_hash,
                dependencies,
            )
            .await?;
        self.write_routing(
            tenant_id,
            RoutingState::Pipeline,
            Some(earlier_bundle_id),
            actor_principal_ref,
            reason_code,
            &promotion.evidence_hash,
            "rollback",
        )
        .await
    }

    pub async fn contain_pipeline(
        &self,
        tenant_id: &str,
        actor_principal_ref: &str,
        reason_code: &str,
    ) -> Result<TenantRouting, DatabaseError> {
        validate_actor(actor_principal_ref, reason_code)?;
        let evidence_hash = sha256_prefixed(
            format!("trace_commons.pipeline_contain.v1\0{tenant_id}\0{reason_code}").as_bytes(),
        );
        self.write_routing(
            tenant_id,
            RoutingState::Contained,
            None,
            actor_principal_ref,
            reason_code,
            &evidence_hash,
            "contain",
        )
        .await
    }

    pub async fn switch_bound_run_bundle(
        &self,
        _tenant_id: &str,
        _run_id: Uuid,
        _bundle_id: &str,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Constraint(
            BOUND_POLICY_MUST_BE_SUSPENDED_LABEL.to_string(),
        ))
    }

    pub async fn retire_legacy_writer(
        &self,
        tenant_id: &str,
        actor_principal_ref: &str,
        reason_code: &str,
    ) -> Result<LegacyWriterState, DatabaseError> {
        validate_actor(actor_principal_ref, reason_code)?;
        let pending = self.pending_legacy_work_count(tenant_id).await?;
        if pending > 0 {
            return Err(DatabaseError::Constraint(
                LEGACY_WRITER_PENDING_LABEL.to_string(),
            ));
        }
        let routing = self.routing(tenant_id).await?;
        if routing
            .as_ref()
            .map(|item| item.routing_state)
            .is_none_or(|state| state == RoutingState::Legacy)
        {
            return Err(DatabaseError::Constraint(
                PIPELINE_NOT_ACTIVE_LABEL.to_string(),
            ));
        }
        let evidence_hash = sha256_prefixed(
            format!("trace_commons.legacy_writer_retire.v1\0{tenant_id}\0{reason_code}").as_bytes(),
        );
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)
             ON CONFLICT (tenant_id) DO NOTHING",
            &[&tenant_id],
        )
        .await?;
        tx.execute(
            "INSERT INTO pipeline_legacy_writer_status (
                tenant_id, writer_state, actor_principal_ref, reason_code, evidence_hash
             ) VALUES ($1,$2,$3,$4,$5)
             ON CONFLICT (tenant_id) DO UPDATE
             SET writer_state = EXCLUDED.writer_state,
                 actor_principal_ref = EXCLUDED.actor_principal_ref,
                 reason_code = EXCLUDED.reason_code,
                 evidence_hash = EXCLUDED.evidence_hash,
                 recorded_at = NOW()",
            &[
                &tenant_id,
                &LegacyWriterState::Disabled.as_db(),
                &actor_principal_ref,
                &reason_code,
                &evidence_hash,
            ],
        )
        .await?;
        let previous = routing
            .as_ref()
            .map(|item| item.routing_state.as_db())
            .unwrap_or("unselected");
        let resulting = routing
            .as_ref()
            .map(|item| item.routing_state.as_db())
            .unwrap_or("pipeline");
        tx.execute(
            "INSERT INTO pipeline_activation_events (
                tenant_id, event_id, action, previous_state, resulting_state,
                previous_bundle_id, resulting_bundle_id, actor_principal_ref,
                reason_code, evidence_hash
             ) VALUES ($1,$2,'retire_legacy_writer',$3,$4,$5,$6,$7,$8,$9)",
            &[
                &tenant_id,
                &Uuid::new_v4(),
                &previous,
                &resulting,
                &routing
                    .as_ref()
                    .and_then(|item| item.selected_bundle_id.clone()),
                &routing
                    .as_ref()
                    .and_then(|item| item.selected_bundle_id.clone()),
                &actor_principal_ref,
                &reason_code,
                &evidence_hash,
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(LegacyWriterState::Disabled)
    }

    pub async fn legacy_writer_state(
        &self,
        tenant_id: &str,
    ) -> Result<LegacyWriterState, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT writer_state FROM pipeline_legacy_writer_status WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?;
        tx.commit().await?;
        Ok(row
            .map(|item| LegacyWriterState::from_db(item.get::<_, String>("writer_state").as_str()))
            .transpose()?
            .unwrap_or(LegacyWriterState::Enabled))
    }

    pub async fn insert_conflicting_ledger_award(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        trace_id: Uuid,
        ledger_source_key: &str,
        actor_principal_ref: &str,
    ) -> Result<(), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let result = tx
            .execute(
                "INSERT INTO trace_credit_ledger (
                    tenant_id, credit_event_id, submission_id, trace_id, credit_account_ref,
                    event_type, points_delta, reason, external_ref, actor_principal_ref,
                    actor_role, settlement_state, ledger_source_key
                 ) VALUES (
                    $1,$2,$3,$4,$5,'accepted','1.000000',$6,$7,$8,'pipeline_worker','pending',$9
                 )",
                &[
                    &tenant_id,
                    &Uuid::new_v4(),
                    &submission_id,
                    &trace_id,
                    &actor_principal_ref,
                    &PIPELINE_CREDIT_REASON,
                    &format!(
                        "conflict:{}",
                        Sha256::digest(ledger_source_key.as_bytes())[0]
                    ),
                    &actor_principal_ref,
                    &ledger_source_key,
                ],
            )
            .await
            .map_err(map_ledger_conflict);
        match result {
            Ok(_) => {
                tx.commit().await?;
                Ok(())
            }
            Err(error) => {
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_routing(
        &self,
        tenant_id: &str,
        resulting: RoutingState,
        selected_bundle_id: Option<&str>,
        actor_principal_ref: &str,
        reason_code: &str,
        evidence_hash: &str,
        action: &str,
    ) -> Result<TenantRouting, DatabaseError> {
        let keep_bundle = resulting != RoutingState::Pipeline;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)
             ON CONFLICT (tenant_id) DO NOTHING",
            &[&tenant_id],
        )
        .await?;
        let previous = tx
            .query_opt(
                "SELECT routing_state, selected_bundle_id
                   FROM pipeline_tenant_routing
                  WHERE tenant_id = $1
                  FOR UPDATE",
                &[&tenant_id],
            )
            .await?;
        let previous_state = previous
            .as_ref()
            .map(|row| row.get::<_, String>("routing_state"))
            .unwrap_or_else(|| "unselected".to_string());
        let previous_bundle = previous
            .as_ref()
            .and_then(|row| row.get::<_, Option<String>>("selected_bundle_id"));
        let resulting_bundle = if keep_bundle {
            selected_bundle_id
                .map(ToOwned::to_owned)
                .or(previous_bundle.clone())
        } else {
            selected_bundle_id.map(ToOwned::to_owned)
        };
        if resulting == RoutingState::Pipeline && resulting_bundle.is_none() {
            return Err(DatabaseError::Constraint(
                EARLIER_QUALIFIED_BUNDLE_REQUIRED_LABEL.to_string(),
            ));
        }
        let activation_record_id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO pipeline_tenant_routing (
                tenant_id, routing_state, selected_bundle_id, activation_record_id,
                actor_principal_ref, reason_code, evidence_hash
             ) VALUES ($1,$2,$3,$4,$5,$6,$7)
             ON CONFLICT (tenant_id) DO UPDATE
             SET routing_state = EXCLUDED.routing_state,
                 selected_bundle_id = COALESCE(EXCLUDED.selected_bundle_id, pipeline_tenant_routing.selected_bundle_id),
                 activation_record_id = EXCLUDED.activation_record_id,
                 actor_principal_ref = EXCLUDED.actor_principal_ref,
                 reason_code = EXCLUDED.reason_code,
                 evidence_hash = EXCLUDED.evidence_hash,
                 recorded_at = NOW()",
            &[
                &tenant_id,
                &resulting.as_db(),
                &resulting_bundle,
                &activation_record_id,
                &actor_principal_ref,
                &reason_code,
                &evidence_hash,
            ],
        )
        .await?;
        tx.execute(
            "INSERT INTO pipeline_activation_events (
                tenant_id, event_id, action, previous_state, resulting_state,
                previous_bundle_id, resulting_bundle_id, actor_principal_ref,
                reason_code, evidence_hash
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
            &[
                &tenant_id,
                &Uuid::new_v4(),
                &action,
                &previous_state.as_str(),
                &resulting.as_db(),
                &previous_bundle,
                &resulting_bundle,
                &actor_principal_ref,
                &reason_code,
                &evidence_hash,
            ],
        )
        .await?;
        let row = tx
            .query_one(
                "SELECT routing_state, selected_bundle_id, activation_record_id,
                        actor_principal_ref, reason_code, evidence_hash, recorded_at
                   FROM pipeline_tenant_routing
                  WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?;
        let routing = routing_from_row(row)?;
        tx.commit().await?;
        Ok(routing)
    }
}

fn validate_actor(actor_principal_ref: &str, reason_code: &str) -> Result<(), DatabaseError> {
    if !is_operator_actor(actor_principal_ref) || !safe_reason(reason_code) {
        return Err(DatabaseError::Constraint(
            "invalid activation actor".to_string(),
        ));
    }
    Ok(())
}

fn map_ledger_conflict(error: tokio_postgres::Error) -> DatabaseError {
    if error
        .code()
        .is_some_and(|code| code == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
    {
        DatabaseError::Constraint(LEDGER_SOURCE_CONFLICT_LABEL.to_string())
    } else {
        DatabaseError::from(error)
    }
}

fn routing_from_row(row: tokio_postgres::Row) -> Result<TenantRouting, DatabaseError> {
    Ok(TenantRouting {
        routing_state: RoutingState::from_db(row.get::<_, String>("routing_state").as_str())?,
        selected_bundle_id: row.get("selected_bundle_id"),
        activation_record_id: row.get("activation_record_id"),
        actor_principal_ref: row.get("actor_principal_ref"),
        reason_code: row.get("reason_code"),
        evidence_hash: row.get("evidence_hash"),
        recorded_at: row.get("recorded_at"),
    })
}

fn ownership_from_row(row: tokio_postgres::Row) -> Result<ReceiptOwnership, DatabaseError> {
    Ok(ReceiptOwnership {
        owner: ReceiptOwner::from_db(row.get::<_, String>("owner").as_str())?,
        request_idempotency_key: row.get("request_idempotency_key"),
        request_content_hash: row.get("request_content_hash"),
        submission_id: row.get("submission_id"),
        run_id: row.get("run_id"),
        ledger_source_key: row.get("ledger_source_key"),
    })
}

pub fn suspend_instead_of_rebundle_label() -> &'static str {
    BOUND_POLICY_MUST_BE_SUSPENDED_LABEL
}

pub fn contained_phase_is_skipped() -> Option<Phase> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_routing_assigns_new_receipts_to_legacy() {
        assert_eq!(decide_new_receipt_owner(None), Ok(ReceiptOwner::Legacy));
    }

    #[test]
    fn contained_routing_refuses_new_pipeline_receipts() {
        let routing = TenantRouting {
            routing_state: RoutingState::Contained,
            selected_bundle_id: Some(
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            ),
            activation_record_id: Uuid::new_v4(),
            actor_principal_ref: "operator_sha256:test".to_string(),
            reason_code: "contain_first_rollout".to_string(),
            evidence_hash: sha256_prefixed(b"contain"),
            recorded_at: Utc::now(),
        };
        assert_eq!(
            decide_new_receipt_owner(Some(&routing)),
            Err(PIPELINE_CONTAINED_LABEL.to_string())
        );
    }

    #[test]
    fn stale_or_failed_readiness_blocks_activation() {
        let now = Utc::now();
        let passing = ActivationReadiness::passing(now);
        assert!(evaluate_activation_readiness(&passing, now).is_ok());
        let mut failed = passing.clone();
        failed.error_count = 1;
        failed.evidence_hash = activation_readiness_hash(&failed);
        assert_eq!(
            evaluate_activation_readiness(&failed, now),
            Err(ACTIVATION_READINESS_FAILED_LABEL.to_string())
        );
        assert_eq!(
            evaluate_activation_readiness(&passing, now + Duration::minutes(16)),
            Err(ACTIVATION_READINESS_FAILED_LABEL.to_string())
        );
        let _ = BOUND_POLICY_MUST_BE_SUSPENDED_LABEL;
        assert_eq!(
            suspend_instead_of_rebundle_label(),
            BOUND_POLICY_MUST_BE_SUSPENDED_LABEL
        );
        assert!(contained_phase_is_skipped().is_none());
    }
}
