// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A contributor adds the `model_training` use to traces already sent, and
//! the scoring award those traces were refused for lack of it is re-run
//! (owner decisions 2026-10-28; wire contract in
//! `trace_commons_protocol::model_training_consent`).
//!
//! - `GET /v1/contributors/me/model-training-consent` lists the caller's
//!   eligible traces.
//! - `POST /v1/contributors/me/model-training-consent` adds the use to the
//!   named traces, or to every eligible one (at most
//!   `MODEL_TRAINING_CONSENT_MAX_IDS` per call), and re-runs only their
//!   withheld award.
//!
//! Both routes take the contributor's device credential (the upload claim
//! the submission-status read takes) and act only on the caller's own
//! traces: its principal's, and its account's when it is linked to one.
//! Tenant and actor come from that credential, never from the body.
//!
//! The grant is capped by the caller's access-grant ceiling, which the
//! issuer signs into the claim (`allowed_consent_scopes`, `allowed_uses`,
//! from the device's tenant access grant), and by the tenant's submission
//! policy. When either leaves out model training the routes refuse with
//! `model_training_not_permitted`. A credential that carries no scoped
//! allowlist (a static token) is refused too: the grant needs a ceiling that
//! names model training, not the absence of one.
//!
//! The grant is add-only and dated (`trace_consent_grants`, V141), and each
//! one is audited hash-only. The award is not re-scored: the stored gate
//! decision is handed to `emit_scoring_award_for_scored_decision`, the one
//! path every scoring award takes, so the production-gate, central-issuer,
//! tenant-policy and one-award-per-submission checks, the payout cut-over,
//! holds and settlement behave as for any other award.

use super::*;
use trace_commons_protocol::model_training_consent::{
    MODEL_TRAINING_CONSENT_MAX_IDS, MODEL_TRAINING_CONSENT_UNAVAILABLE_LABEL,
    MODEL_TRAINING_NOT_PERMITTED_LABEL, ModelTrainingAwardState, ModelTrainingConsentEligibility,
    ModelTrainingConsentGrantItem, ModelTrainingConsentGrantRequest,
    ModelTrainingConsentGrantResponse, ModelTrainingConsentOutcome,
};
use trace_commons_server::db::ModelTrainingConsentGrant;

/// The audit event kind of one grant. Its DB row is a `PolicyUpdate` row
/// with `Maintenance` metadata whose surface is this kind and whose one
/// count is `granted`. It names the trace and the actor's principal ref and
/// nothing else.
pub(crate) const MODEL_TRAINING_CONSENT_AUDIT_KIND: &str = "model_training_consent";

/// The read-audit surface of the eligibility list.
const MODEL_TRAINING_CONSENT_READ_SURFACE: &str = "model_training_consent";

/// `GET`: the caller's eligible traces.
pub(crate) async fn eligibility_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<ModelTrainingConsentEligibility>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    let db = consent_database(state.as_ref())?;
    require_model_training_permitted(state.as_ref(), &tenant).await?;
    let owners = owner_principal_refs(state.as_ref(), &tenant).await?;
    let eligible = db
        .list_model_training_consent_eligible(
            &tenant.tenant_id,
            &owners,
            MODEL_TRAINING_CONSENT_MAX_IDS as i64,
        )
        .await
        .map_err(internal_error)?;
    append_control_plane_read_audit(
        state.as_ref(),
        &tenant,
        MODEL_TRAINING_CONSENT_READ_SURFACE,
        eligible.submission_ids.len(),
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(ModelTrainingConsentEligibility {
        eligible_count: eligible.eligible_count,
        submission_ids: eligible.submission_ids,
    }))
}

/// `POST`: add the use, then re-run the withheld awards.
pub(crate) async fn grant_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ModelTrainingConsentGrantRequest>,
) -> ApiResult<Json<ModelTrainingConsentGrantResponse>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    let selection = body
        .selected_ids()
        .map_err(|label| api_error(StatusCode::BAD_REQUEST, label))?;
    let db = consent_database(state.as_ref())?;
    require_model_training_permitted(state.as_ref(), &tenant).await?;
    let owners = owner_principal_refs(state.as_ref(), &tenant).await?;
    let submission_ids = match selection {
        Some(ids) => ids,
        None => {
            db.list_model_training_consent_eligible(
                &tenant.tenant_id,
                &owners,
                MODEL_TRAINING_CONSENT_MAX_IDS as i64,
            )
            .await
            .map_err(internal_error)?
            .submission_ids
        }
    };
    // Dated when given: the time this request was accepted, never earlier.
    let granted_at = Utc::now();
    let rows = db
        .grant_model_training_consent(ModelTrainingConsentGrant {
            tenant_id: &tenant.tenant_id,
            owner_principal_refs: &owners,
            submission_ids: &submission_ids,
            actor_principal_ref: &tenant.principal_ref,
            granted_at,
        })
        .await
        .map_err(internal_error)?;

    let mut results = Vec::with_capacity(rows.len());
    for row in rows {
        let item = match row.outcome {
            ModelTrainingConsentOutcome::NotEligible => ModelTrainingConsentGrantItem {
                submission_id: row.submission_id,
                outcome: row.outcome,
                granted_at: None,
                award: ModelTrainingAwardState::None,
                award_withheld_reason: None,
            },
            ModelTrainingConsentOutcome::Granted | ModelTrainingConsentOutcome::AlreadyGranted => {
                // The database grant has committed for every row, so each
                // follow-up runs on its own and a failure on one trace never
                // skips the next one's audit. The audit row comes first. A
                // trace whose follow-up failed is answered `pending` and
                // logged hash-only; asking again repairs the file record
                // and re-runs the award.
                let mut follow_up_failed = false;
                if row.outcome == ModelTrainingConsentOutcome::Granted
                    && let Err(error) =
                        append_grant_audit(state.as_ref(), &tenant, row.submission_id).await
                {
                    log_follow_up_failure("audit", &tenant.tenant_id, row.submission_id, &error);
                    follow_up_failed = true;
                }
                // The file record is what the award reads when database
                // reads are off. A repeat grant mirrors it too.
                if let Err(error) = mirror_grant_to_file_record(
                    state.as_ref(),
                    &tenant.tenant_id,
                    row.submission_id,
                ) {
                    log_follow_up_failure(
                        "file_record",
                        &tenant.tenant_id,
                        row.submission_id,
                        &error,
                    );
                    follow_up_failed = true;
                }
                let (award, award_withheld_reason) = if follow_up_failed {
                    (ModelTrainingAwardState::Pending, None)
                } else {
                    rerun_withheld_award(state.as_ref(), &tenant.tenant_id, row.submission_id).await
                };
                ModelTrainingConsentGrantItem {
                    submission_id: row.submission_id,
                    outcome: row.outcome,
                    granted_at: row.granted_at,
                    award,
                    award_withheld_reason,
                }
            }
        };
        results.push(item);
    }
    let remaining_eligible = db
        .list_model_training_consent_eligible(&tenant.tenant_id, &owners, 1)
        .await
        .map_err(internal_error)?
        .eligible_count;
    Ok(Json(ModelTrainingConsentGrantResponse {
        results,
        remaining_eligible,
    }))
}

fn log_follow_up_failure(
    step: &'static str,
    tenant_id: &str,
    submission_id: Uuid,
    error: &anyhow::Error,
) {
    tracing::warn!(
        step,
        error_hash = %safe_runtime_error_hash(error),
        submission_hash = %sha256_prefixed(&submission_id.to_string()),
        tenant_hash = %sha256_prefixed(tenant_id),
        "Trace Commons model training grant recorded; a follow-up step failed"
    );
}

fn consent_database(state: &AppState) -> ApiResult<&Arc<dyn Database>> {
    state.db_mirror.as_ref().ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            MODEL_TRAINING_CONSENT_UNAVAILABLE_LABEL,
        )
    })
}

/// Contributors only, and only within the ceiling: the claim's own scoped
/// allowlists must both name model training, and the tenant policy, when
/// there is one (or must be one), must allow it.
async fn require_model_training_permitted(state: &AppState, tenant: &TenantAuth) -> ApiResult<()> {
    if tenant.role != TokenRole::Contributor {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            MODEL_TRAINING_NOT_PERMITTED_LABEL,
        ));
    }
    let policy = tenant_submission_policy_for_request(state, tenant).await?;
    if !model_training_within_ceiling(
        tenant,
        policy.as_ref(),
        state.require_tenant_submission_policy,
    ) {
        tracing::info!(
            tenant_storage_ref = %tenant_storage_ref(&tenant.tenant_id),
            "Trace Commons model training grant refused outside the access ceiling"
        );
        return Err(api_error(
            StatusCode::FORBIDDEN,
            MODEL_TRAINING_NOT_PERMITTED_LABEL,
        ));
    }
    Ok(())
}

/// The ceiling check, pure. An empty claim allowlist is "unscoped", which
/// elsewhere reads as unrestricted; here it is a refusal, because the grant
/// widens what a trace may be used for and needs a ceiling that says so.
fn model_training_within_ceiling(
    tenant: &TenantAuth,
    policy: Option<&TenantSubmissionPolicy>,
    require_policy: bool,
) -> bool {
    if !tenant
        .allowed_consent_scopes
        .contains(&ConsentScope::ModelTraining)
        || !tenant
            .allowed_uses
            .contains(&TraceAllowedUse::ModelTraining)
    {
        return false;
    }
    match policy {
        None => !require_policy,
        Some(policy) => {
            (policy.allowed_consent_scopes.is_empty()
                || policy
                    .allowed_consent_scopes
                    .contains(&ConsentScope::ModelTraining))
                && (policy.allowed_uses.is_empty()
                    || policy
                        .allowed_uses
                        .contains(&TraceAllowedUse::ModelTraining))
        }
    }
}

/// The principals whose traces the caller may grant on: its own refs, and
/// every principal of its account when it is linked to one (the same set
/// the credit and status reads show it). Never the shared legacy ref.
async fn owner_principal_refs(state: &AppState, tenant: &TenantAuth) -> ApiResult<Vec<String>> {
    let mut owners: BTreeSet<String> = tenant.principal_aliases().map(str::to_string).collect();
    if let Some(scope) = caller_credit_account_scope(state, tenant)
        .await
        .map_err(internal_error)?
    {
        owners.extend(scope.principals().to_vec());
    }
    owners.remove(&legacy_principal_ref());
    Ok(owners.into_iter().collect())
}

/// Adds the scope and the use to the trace's file record, when it has one
/// and lacks them. Add-only, like the database write.
fn mirror_grant_to_file_record(
    state: &AppState,
    tenant_id: &str,
    submission_id: Uuid,
) -> anyhow::Result<()> {
    let Some(mut record) = read_submission_record(&state.root, tenant_id, submission_id)? else {
        return Ok(());
    };
    let mut changed = false;
    if !record.consent_scopes.contains(&ConsentScope::ModelTraining) {
        record.consent_scopes.push(ConsentScope::ModelTraining);
        changed = true;
    }
    if !record
        .allowed_uses
        .contains(&TraceAllowedUse::ModelTraining)
    {
        record.allowed_uses.push(TraceAllowedUse::ModelTraining);
        changed = true;
    }
    if changed {
        write_submission_record(&state.root, &record)?;
    }
    Ok(())
}

async fn append_grant_audit(
    state: &AppState,
    tenant: &TenantAuth,
    submission_id: Uuid,
) -> anyhow::Result<()> {
    let event = TraceCommonsAuditEvent::model_training_consent(tenant, submission_id);
    let metadata = model_training_consent_audit_metadata();
    append_audit_event_with_db_mirror(
        state,
        tenant,
        event,
        StorageTraceAuditAction::PolicyUpdate,
        metadata,
    )
    .await
}

/// The DB row metadata of a grant's audit event.
pub(crate) fn model_training_consent_audit_metadata() -> StorageTraceAuditSafeMetadata {
    StorageTraceAuditSafeMetadata::Maintenance {
        surface: Some(MODEL_TRAINING_CONSENT_AUDIT_KIND.to_string()),
        purpose_hash: Some(sha256_prefixed(MODEL_TRAINING_CONSENT_AUDIT_KIND)),
        dry_run: false,
        action_counts: BTreeMap::from([("granted".to_string(), 1)]),
    }
}

/// Re-runs the trace's withheld scoring award, without re-scoring it.
///
/// - Its latest decision is a legacy-gate decision that passed both floors
///   and was withheld as `policy_mismatch`: that decision goes through
///   `emit_scoring_award_for_scored_decision` as the scoring driver issues
///   it. Written, or already paid: `credited`, and the decision row's
///   withheld reason is cleared. Withheld again: `none` with the new label
///   (the emitter records it on the row). A failed check: `pending`, so a
///   repeat grant tries again.
/// - Not scored yet: `pending`. Scoring reads the stored uses, which now
///   include model training.
/// - Anything else (a floor failed, another withheld reason, a pipeline
///   Settle row): `none`. A pipeline run's leg that already completed
///   withheld is not re-opened; a run that has not settled yet reads the
///   new uses at Settle.
async fn rerun_withheld_award(
    state: &AppState,
    tenant_id: &str,
    submission_id: Uuid,
) -> (ModelTrainingAwardState, Option<String>) {
    match rerun_withheld_award_inner(state, tenant_id, submission_id).await {
        Ok(answer) => answer,
        Err(error) => {
            tracing::warn!(
                error_hash = %safe_runtime_error_hash(&error),
                submission_hash = %sha256_prefixed(&submission_id.to_string()),
                tenant_hash = %sha256_prefixed(tenant_id),
                "Trace Commons model training grant could not re-run the scoring award; \
                 grant recorded, award left pending"
            );
            (ModelTrainingAwardState::Pending, None)
        }
    }
}

async fn rerun_withheld_award_inner(
    state: &AppState,
    tenant_id: &str,
    submission_id: Uuid,
) -> anyhow::Result<(ModelTrainingAwardState, Option<String>)> {
    let db = state
        .db_mirror
        .as_ref()
        .context(MODEL_TRAINING_CONSENT_UNAVAILABLE_LABEL)?;
    let pipeline_owned = match state.pipeline_store.as_ref() {
        Some(store) => {
            store
                .submission_has_pipeline_run(tenant_id, submission_id)
                .await?
        }
        None => false,
    };
    let withheld = if pipeline_owned {
        None
    } else {
        db.withheld_policy_mismatch_gate_award(tenant_id, submission_id)
            .await?
    };
    let Some(withheld) = withheld else {
        let latest = db
            .list_latest_gate_credit_decisions(tenant_id, &[submission_id])
            .await?;
        return Ok(match latest.into_iter().next() {
            None => (ModelTrainingAwardState::Pending, None),
            Some(row) => (ModelTrainingAwardState::None, row.credit_withheld_reason),
        });
    };
    let issuer = scoring_driver_credit_issuer(state, tenant_id);
    let award = emit_scoring_award_for_scored_decision(
        state,
        &issuer,
        submission_id,
        ScoredGateDecision {
            decision_id: withheld.decision_id,
            perplexity_passed: true,
            novelty_passed: true,
            vector_entry_id: withheld.vector_entry_id,
            gate_policy_version: &withheld.gate_policy_version,
            gate_version_hash: &withheld.gate_version_hash,
            chunks_capped: withheld.chunks_capped,
        },
    )
    .await;
    match award {
        Ok((_, None)) => {
            // Credited now, or by an earlier award (the emitter's
            // one-award check). The row no longer says withheld.
            db.update_trace_gate_decision_credit_withheld_reason(
                tenant_id,
                withheld.decision_id,
                None,
            )
            .await?;
            Ok((ModelTrainingAwardState::Credited, None))
        }
        Ok((_, Some(label))) => Ok((ModelTrainingAwardState::None, Some(label))),
        Err((status, _)) => Err(anyhow::anyhow!(
            "model_training_award_check_failed status={}",
            status.as_u16()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contributor(scopes: &[ConsentScope], uses: &[TraceAllowedUse]) -> TenantAuth {
        TenantAuth {
            tenant_id: "tenant-a".to_string(),
            role: TokenRole::Contributor,
            principal_ref: "principal".to_string(),
            legacy_principal_ref: None,
            expires_at: None,
            auth_method: TraceAuthMethod::SignedClaim,
            signed_claim_issuer: None,
            signed_claim_audiences: BTreeSet::new(),
            signed_claim_subject: None,
            allowed_consent_scopes: scopes.iter().copied().collect(),
            allowed_uses: uses.iter().copied().collect(),
        }
    }

    fn policy(scopes: &[ConsentScope], uses: &[TraceAllowedUse]) -> TenantSubmissionPolicy {
        TenantSubmissionPolicy {
            allowed_consent_scopes: scopes.iter().copied().collect(),
            allowed_uses: uses.iter().copied().collect(),
        }
    }

    #[test]
    fn the_ceiling_must_name_model_training() {
        let full = contributor(
            &[
                ConsentScope::DebuggingEvaluation,
                ConsentScope::ModelTraining,
            ],
            &[TraceAllowedUse::Debugging, TraceAllowedUse::ModelTraining],
        );
        assert!(model_training_within_ceiling(&full, None, false));
        // The floor a device without a grant gets.
        let floor = contributor(
            &[ConsentScope::DebuggingEvaluation],
            &[TraceAllowedUse::Debugging, TraceAllowedUse::Evaluation],
        );
        assert!(!model_training_within_ceiling(&floor, None, false));
        // Scope without the use, and the use without the scope.
        assert!(!model_training_within_ceiling(
            &contributor(
                &[ConsentScope::ModelTraining],
                &[TraceAllowedUse::Debugging]
            ),
            None,
            false
        ));
        assert!(!model_training_within_ceiling(
            &contributor(
                &[ConsentScope::DebuggingEvaluation],
                &[TraceAllowedUse::ModelTraining]
            ),
            None,
            false
        ));
        // An unscoped credential is not a ceiling that names it.
        assert!(!model_training_within_ceiling(
            &contributor(&[], &[]),
            None,
            false
        ));
    }

    #[test]
    fn the_tenant_policy_caps_the_grant() {
        let full = contributor(
            &[ConsentScope::ModelTraining],
            &[TraceAllowedUse::ModelTraining],
        );
        assert!(!model_training_within_ceiling(&full, None, true));
        assert!(model_training_within_ceiling(
            &full,
            Some(&policy(&[], &[])),
            true
        ));
        assert!(model_training_within_ceiling(
            &full,
            Some(&policy(
                &[ConsentScope::ModelTraining],
                &[TraceAllowedUse::ModelTraining]
            )),
            true
        ));
        assert!(!model_training_within_ceiling(
            &full,
            Some(&policy(&[ConsentScope::DebuggingEvaluation], &[])),
            false
        ));
        assert!(!model_training_within_ceiling(
            &full,
            Some(&policy(&[], &[TraceAllowedUse::Evaluation])),
            false
        ));
    }

    #[test]
    fn audit_labels_are_plain_snake_case() {
        for label in [
            MODEL_TRAINING_CONSENT_AUDIT_KIND,
            MODEL_TRAINING_NOT_PERMITTED_LABEL,
            MODEL_TRAINING_CONSENT_UNAVAILABLE_LABEL,
        ] {
            assert!(
                label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_'),
                "{label}"
            );
        }
    }
}
