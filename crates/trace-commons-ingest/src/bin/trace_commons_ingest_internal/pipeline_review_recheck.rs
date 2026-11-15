// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reviewer-scoped preparation and readback of immutable corrected versions.

use super::*;
use trace_commons_server::versioned_pipeline::{
    PipelineReviewRecheck, PipelineReviewRecheckBinding,
};

#[derive(Debug, Serialize)]
pub(super) struct CandidateSummary {
    candidate_id: Uuid,
    run_id: Uuid,
    submission_id: Uuid,
    ordinal: i64,
    parent_candidate_id: Option<Uuid>,
    source_hash: String,
    content_hash: String,
    residual_pii_risk: ResidualPiiRisk,
    coverage_complete: bool,
    evidence: trace_commons_protocol::trace_contribution::ReviewPrivacyRecheckEvidence,
    created_at: DateTime<Utc>,
}

impl From<&PipelineReviewRecheck> for CandidateSummary {
    fn from(value: &PipelineReviewRecheck) -> Self {
        Self {
            candidate_id: value.recheck_id,
            run_id: value.run_id,
            submission_id: value.submission_id,
            ordinal: value.ordinal,
            parent_candidate_id: value.parent_recheck_id,
            source_hash: value.source_content_hash.clone(),
            content_hash: value.content_hash.clone(),
            residual_pii_risk: value.residual_pii_risk,
            coverage_complete: value.evidence.coverage_complete,
            evidence: value.evidence.clone(),
            created_at: value.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecheckRequest {
    lease_token: Uuid,
    recheck_id: Uuid,
    expected_source_hash: String,
}

fn valid_content_hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

pub(super) fn assessment_binding(
    candidate_id: Option<Uuid>,
    candidate_content_hash: Option<String>,
) -> ApiResult<Option<PipelineReviewRecheckBinding>> {
    match (candidate_id, candidate_content_hash) {
        (None, None) => Ok(None),
        (Some(recheck_id), Some(content_hash)) if valid_content_hash(&content_hash) => {
            Ok(Some(PipelineReviewRecheckBinding {
                recheck_id,
                content_hash,
            }))
        }
        _ => Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "candidate id and valid content hash are required together",
        )),
    }
}

pub(super) fn constraint_status(label: &str) -> Option<StatusCode> {
    match label {
        "pipeline_review_recheck_stale"
        | "pipeline_review_recheck_required"
        | "pipeline_review_recheck_source_changed" => Some(StatusCode::CONFLICT),
        "pipeline_review_recheck_unresolved" | "pipeline_review_recheck_unsupported_fields" => {
            Some(StatusCode::UNPROCESSABLE_ENTITY)
        }
        "pipeline_review_recheck_not_found" => Some(StatusCode::NOT_FOUND),
        "pipeline_review_recheck_busy" | "pipeline_review_recheck_unavailable" => {
            Some(StatusCode::SERVICE_UNAVAILABLE)
        }
        _ => None,
    }
}

fn recheck_error(error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    if let Some(DatabaseError::Constraint(label)) = error.downcast_ref::<DatabaseError>() {
        if label == PIPELINE_REVIEW_CLAIM_STALE_OR_INOPERABLE {
            return api_error(StatusCode::CONFLICT, label.clone());
        }
        if let Some(status) = constraint_status(label) {
            return api_error(status, label.clone());
        }
    }
    // Only return fixed labels. Never expose a classifier/DB response body.
    for label in [
        "pipeline_review_recheck_stale",
        "pipeline_review_recheck_required",
        "pipeline_review_recheck_source_changed",
        "pipeline_review_recheck_unresolved",
        "pipeline_review_recheck_not_found",
        "pipeline_review_recheck_unsupported_fields",
        "pipeline_review_recheck_busy",
        "pipeline_review_recheck_unavailable",
        "privacy_control_missing",
        "privacy_classification_failed",
    ] {
        if error.chain().any(|cause| cause.to_string() == label) {
            return api_error(
                constraint_status(label).unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
                label,
            );
        }
    }
    internal_error(error)
}

fn reviewer_claim(tenant: &TenantAuth, run_id: Uuid, lease_token: Uuid) -> PipelineReviewClaim {
    PipelineReviewClaim {
        tenant_id: tenant.tenant_id.clone(),
        run_id,
        reviewer_principal_ref: pipeline_reviewer_principal_ref(&tenant.principal_ref),
        lease_token,
        // The store checks the authoritative live lease, not this placeholder.
        lease_expires_at: Utc::now(),
    }
}

async fn authorize_record(
    state: &AppState,
    tenant: &TenantAuth,
    service: &PipelineService,
    run_id: Uuid,
) -> ApiResult<TraceCommonsSubmissionRecord> {
    let record = pipeline_run_submission_record(state, service, &tenant.tenant_id, run_id)
        .await?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "pipeline run not found"))?;
    let policy =
        tenant_privileged_action_policy_for_request(state, tenant, "privacy recheck").await?;
    ensure_record_matches_privileged_action_policy_abac(
        &record,
        tenant,
        policy.as_ref(),
        "privacy recheck",
    )?;
    Ok(record)
}

// Long-running classification and object reads must not reuse a grant that
// expired or was revoked while external I/O was in flight.
async fn authorize_after_io(
    state: &AppState,
    headers: &HeaderMap,
    original: &TenantAuth,
    service: &PipelineService,
    run_id: Uuid,
) -> ApiResult<TenantAuth> {
    let current = authenticate_with_tenant_access_grant(state, headers).await?;
    require_reviewer(&current)?;
    if current.tenant_id != original.tenant_id || current.principal_ref != original.principal_ref {
        return Err(api_error(StatusCode::FORBIDDEN, "reviewer access changed"));
    }
    authorize_record(state, &current, service, run_id).await?;
    Ok(current)
}

pub(super) async fn recheck_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(run_id): AxumPath<Uuid>,
    Json(body): Json<RecheckRequest>,
) -> ApiResult<Json<CandidateSummary>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_reviewer(&tenant)?;
    if !valid_content_hash(&body.expected_source_hash) {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid source content hash",
        ));
    }
    let service = require_pipeline_service(state.as_ref())?;
    authorize_record(state.as_ref(), &tenant, service, run_id).await?;
    let claim = reviewer_claim(&tenant, run_id, body.lease_token);
    let prepared = service
        .prepare_review_recheck(&claim, body.recheck_id, &body.expected_source_hash)
        .await
        .map_err(recheck_error)?;
    let tenant = match authorize_after_io(state.as_ref(), &headers, &tenant, service, run_id).await
    {
        Ok(current) => current,
        Err(error) => {
            if let Err(cleanup_error) = service.discard_review_recheck(&prepared).await {
                tracing::warn!(
                    error_class = "pipeline_review_recheck_cleanup_failed",
                    run_ref_hash = %sha256_prefixed(&run_id.to_string()),
                    error_hash = %safe_runtime_error_hash(&cleanup_error),
                    "pipeline review recheck cleanup failed"
                );
            }
            return Err(error);
        }
    };
    let candidate = service
        .finish_review_recheck(prepared)
        .await
        .map_err(recheck_error)?;
    // The candidate and its creator/evidence are already durable. As with
    // assessments, an audit append failure must not obscure a committed result.
    if let Err(error) = append_recheck_audit(state.as_ref(), &tenant, &candidate).await {
        log_pipeline_review_audit_append_failure(&tenant.tenant_id, run_id, "recheck", &error);
    }
    Ok(Json(CandidateSummary::from(&candidate)))
}

async fn append_recheck_audit(
    state: &AppState,
    tenant: &TenantAuth,
    candidate: &PipelineReviewRecheck,
) -> anyhow::Result<()> {
    let event_id = deterministic_trace_uuid_for_external_ref(
        "pipeline-review-recheck",
        &tenant.tenant_id,
        candidate.submission_id,
        &candidate.recheck_id.to_string(),
    );
    if let Some(db) = state.db_mirror.as_ref() {
        if let Some(existing) = db
            .get_trace_audit_event_by_id(&tenant.tenant_id, event_id)
            .await?
        {
            anyhow::ensure!(
                existing.submission_id == Some(candidate.submission_id)
                    && existing.object_ref_id == Some(candidate.object_ref_id)
                    && existing.decision_inputs_hash.as_deref()
                        == Some(candidate.content_hash.as_str()),
                "pipeline_review_recheck_audit_conflict"
            );
            return Ok(());
        }
    }
    let mut event = TraceCommonsAuditEvent::lifecycle_status_change(
        tenant,
        LifecycleAuditActor::Tenant,
        candidate.submission_id,
        TraceCorpusStatus::Quarantined,
        "privacy_recheck_prepared",
    );
    event.kind = "pipeline_review_recheck".into();
    event.event_id = event_id;
    event.created_at = candidate.created_at;
    event.decision_inputs_hash = Some(candidate.content_hash.clone());
    let mut row = review_decision_audit_row(
        TraceCorpusStatus::Quarantined,
        "privacy_recheck_prepared",
        None,
    )?;
    row.object_ref_id = Some(candidate.object_ref_id);
    append_audit_event_mirrored(state, tenant, event, row, "privacy recheck audit").await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListQuery {
    limit: Option<usize>,
}

pub(super) async fn list_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(run_id): AxumPath<Uuid>,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<Vec<CandidateSummary>>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_reviewer(&tenant)?;
    let service = require_pipeline_service(state.as_ref())?;
    authorize_record(state.as_ref(), &tenant, service, run_id).await?;
    let limit = query.limit.unwrap_or(25).clamp(1, 500);
    let versions = service
        .store()
        .list_review_rechecks(&tenant.tenant_id, run_id, limit)
        .await
        .map_err(pipeline_review_db_error)?;
    Ok(Json(versions.iter().map(CandidateSummary::from).collect()))
}

pub(super) async fn content_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath((run_id, recheck_id)): AxumPath<(Uuid, Uuid)>,
) -> ApiResult<axum::response::Response> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_reviewer(&tenant)?;
    let service = require_pipeline_service(state.as_ref())?;
    authorize_record(state.as_ref(), &tenant, service, run_id).await?;
    let token = headers
        .get("x-review-lease-token")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "review lease token is required"))?;
    let claim = reviewer_claim(&tenant, run_id, token);
    let candidate = service
        .store()
        .get_review_recheck(&tenant.tenant_id, run_id, recheck_id)
        .await
        .map_err(pipeline_review_db_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "reviewed version not found"))?;
    let bytes = service
        .load_review_recheck_content(&claim, recheck_id)
        .await
        .map_err(recheck_error)?;
    let tenant = authorize_after_io(state.as_ref(), &headers, &tenant, service, run_id).await?;
    append_trace_content_read_audit_per_source(
        state.as_ref(),
        &tenant,
        candidate.submission_id,
        &[candidate.object_ref_id],
        "pipeline_review_recheck",
        Some("privacy_review"),
    )
    .await
    .map_err(internal_error)?;
    axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .header(axum::http::header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from(bytes))
        .map_err(internal_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_binding_is_an_exact_validated_pair() {
        let id = Uuid::new_v4();
        let digest = format!("sha256:{}", "a".repeat(64));
        assert!(assessment_binding(None, None).unwrap().is_none());
        assert!(assessment_binding(Some(id), None).is_err());
        assert!(assessment_binding(None, Some(digest.clone())).is_err());
        assert!(assessment_binding(Some(id), Some("sha256:bad".into())).is_err());
        let binding = assessment_binding(Some(id), Some(digest.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(binding.recheck_id, id);
        assert_eq!(binding.content_hash, digest);
    }

    #[test]
    fn metadata_request_rejects_content_and_unknown_fields() {
        let value = serde_json::json!({
            "lease_token":Uuid::new_v4(),"recheck_id":Uuid::new_v4(),
            "expected_source_hash":format!("sha256:{}", "a".repeat(64)),
            "content":"caller cannot upload a replacement",
        });
        assert!(serde_json::from_value::<RecheckRequest>(value).is_err());
        assert_eq!(
            constraint_status("pipeline_review_recheck_unresolved"),
            Some(StatusCode::UNPROCESSABLE_ENTITY)
        );
        assert_eq!(constraint_status("unrecognized_private_message"), None);
        assert_eq!(
            recheck_error(anyhow::anyhow!("pipeline_review_recheck_stale")).0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            recheck_error(anyhow::anyhow!(
                "pipeline_review_recheck_unsupported_fields"
            ))
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}
