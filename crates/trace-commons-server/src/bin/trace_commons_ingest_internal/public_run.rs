// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

// INTEGRATION: route handlers are registered by trace-commons-ingest; storage
// stays behind Database and public DTOs stay in trace-commons-protocol.

use super::*;

const PUBLIC_RUN_PER_IP_LIMIT: u32 = 120;
const PUBLIC_RUN_GLOBAL_LIMIT: u32 = 2_000;
const PUBLIC_RUN_GLOBAL_CONCURRENCY: u32 = 32;

fn public_run_contribution_status(
    status: StorageTraceCorpusStatus,
) -> trace_commons_protocol::public_run::PublicRunContributionStatus {
    use trace_commons_protocol::public_run::PublicRunContributionStatus;

    match status {
        StorageTraceCorpusStatus::Received => PublicRunContributionStatus::Received,
        StorageTraceCorpusStatus::Accepted => PublicRunContributionStatus::Accepted,
        StorageTraceCorpusStatus::Quarantined => PublicRunContributionStatus::Quarantined,
        StorageTraceCorpusStatus::AwaitingPiiBackstop => {
            PublicRunContributionStatus::AwaitingPiiBackstop
        }
        StorageTraceCorpusStatus::Rejected => PublicRunContributionStatus::Rejected,
        StorageTraceCorpusStatus::Revoked => PublicRunContributionStatus::Revoked,
        StorageTraceCorpusStatus::Expired => PublicRunContributionStatus::Expired,
        StorageTraceCorpusStatus::Purged => PublicRunContributionStatus::Purged,
    }
}

fn public_run_permitted_uses(values: &[String]) -> anyhow::Result<Vec<TraceAllowedUse>> {
    values
        .iter()
        .map(|value| {
            serde_json::from_value(serde_json::Value::String(value.clone()))
                .context("failed to parse session-detail allowed use")
        })
        .collect()
}

fn validation_error(
    error: trace_commons_protocol::public_run::PublicRunValidationError,
) -> (StatusCode, Json<ApiError>) {
    use trace_commons_protocol::public_run::PublicRunValidationError;
    match error {
        PublicRunValidationError::ApprovalDigest => {
            api_error(StatusCode::CONFLICT, "public run approval changed")
        }
        PublicRunValidationError::SensitiveText => api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "public run contains sensitive text",
        ),
        _ => api_error(StatusCode::BAD_REQUEST, "public run invalid"),
    }
}

fn account_response<T: serde::Serialize>(value: T) -> axum::response::Response {
    let mut response = Json(value).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        axum::http::header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn audit_source_read_failure(
    state: &AppState,
    ctx: &AccountCtx,
    submission_id: Uuid,
    purpose: &'static str,
) -> (StatusCode, Json<ApiError>) {
    if let Err(error) = append_trace_content_read_audit_per_source(
        state,
        &account_audit_tenant(ctx),
        submission_id,
        &[],
        purpose,
        Some("read_failed"),
    )
    .await
    {
        tracing::warn!(
            error_hash = %safe_display_error_hash(&error),
            "Trace Commons public run source-read audit append failed"
        );
    }
    api_error(StatusCode::INTERNAL_SERVER_ERROR, "public run unavailable")
}

pub(super) async fn account_public_run_session_detail_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    AxumPath(submission_id): AxumPath<Uuid>,
) -> ApiResult<axum::response::Response> {
    let not_found = || api_error(StatusCode::NOT_FOUND, "trace not found");
    let db = account_db(state.as_ref())?;
    let stored = db
        .get_trace_submission(&ctx.tenant_id, submission_id)
        .await
        .map_err(internal_error)?;
    let stored = match stored {
        Some(record) if ctx.principal_set.contains(&record.auth_principal_ref) => record,
        _ => return Err(not_found()),
    };
    let contribution_status = public_run_contribution_status(stored.status);
    let permitted_uses = public_run_permitted_uses(&stored.allowed_uses).map_err(internal_error)?;
    let account_key = ctx.account_id.as_uuid().to_string();
    if !ACCOUNT_RATE_LIMITER.check(
        &format!("content-account:{account_key}"),
        CONTENT_PER_ACCOUNT_LIMIT,
    ) {
        return Err(api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited"));
    }
    let _account_slot = ACCOUNT_RATE_LIMITER
        .acquire(
            &format!("content-account:{account_key}"),
            CONTENT_PER_ACCOUNT_CONCURRENCY,
        )
        .ok_or_else(|| api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited"))?;
    let _global_slot = ACCOUNT_RATE_LIMITER
        .acquire("public-run-detail-global", PUBLIC_RUN_GLOBAL_CONCURRENCY)
        .ok_or_else(|| api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited"))?;
    let owner_state = db
        .get_owned_public_run_state(&ctx.tenant_id, ctx.account_id.as_uuid(), submission_id)
        .await
        .map_err(internal_error)?;
    let owner_state = owner_state_from_data(owner_state).map_err(internal_error)?;
    if let Some(status_only) =
        trace_commons_protocol::public_run::PublicRunSessionRecord::status_only(
            contribution_status,
            permitted_uses.clone(),
            stored.schema_version.clone(),
            stored.consent_policy_version.clone(),
            stored.redaction_pipeline_version.clone(),
            owner_state.clone(),
        )
    {
        return Ok(account_response(status_only));
    }
    let record = trace_commons_record_from_storage_submission(stored)
        .ok_or_else(not_found)?
        .map_err(internal_error)?;
    let read_state = Arc::clone(&state);
    let projected = tokio::task::spawn_blocking(move || {
        let envelope = read_envelope_by_record(read_state.as_ref(), &record)?;
        if envelope.submission_id != submission_id {
            anyhow::bail!("session-detail source submission mismatch");
        }
        Ok::<_, anyhow::Error>(
            trace_commons_protocol::public_run::PublicRunSessionRecord::from_envelope(
                envelope,
                owner_state,
            )
            .with_contribution_state(contribution_status, permitted_uses),
        )
    })
    .await
    .map_err(anyhow::Error::from)
    .and_then(|result| result)
    .inspect_err(|error| {
        tracing::warn!(
            error_hash = %safe_display_error_hash(error),
            "Trace Commons session-detail source read failed; failing closed"
        );
    });
    let projected = match projected {
        Ok(projected) => projected,
        Err(_) => {
            return Err(audit_source_read_failure(
                state.as_ref(),
                &ctx,
                submission_id,
                "account_public_run_detail",
            )
            .await);
        }
    };
    append_trace_content_read_audit_per_source(
        state.as_ref(),
        &account_audit_tenant(&ctx),
        submission_id,
        &[],
        "account_public_run_detail",
        None,
    )
    .await
    .map_err(internal_error)?;
    Ok(account_response(projected))
}

pub(super) fn validate_public_run_provenance(
    draft: &trace_commons_protocol::public_run::PublicRunDraft,
    envelope: &trace_commons_protocol::trace_contribution::TraceContributionEnvelope,
) -> Result<(), &'static str> {
    let redacted_events = envelope
        .events
        .iter()
        .filter_map(|event| {
            event
                .redacted_content
                .as_deref()
                .map(|content| (event.event_id, content))
        })
        .collect::<HashMap<_, _>>();
    for evidence in &draft.evidence {
        let Some(content) = redacted_events.get(&evidence.event_id) else {
            return Err("public run evidence changed");
        };
        if !content.contains(&evidence.excerpt) {
            return Err("public run evidence changed");
        }
    }
    if let Some(correction) = draft.correction_excerpt.as_deref() {
        if !envelope
            .outcome
            .human_correction
            .as_deref()
            .is_some_and(|source| source.contains(correction))
        {
            return Err("public run correction changed");
        }
    }
    Ok(())
}

async fn validate_public_run_prose(
    draft: &trace_commons_protocol::public_run::PublicRunDraft,
) -> ApiResult<()> {
    use trace_commons_protocol::trace_contribution::privacy_filter_adapter_from_env;

    let adapter = privacy_filter_adapter_from_env()
        .map_err(|error| {
            tracing::warn!(
                error_hash = %safe_display_error_hash(&error),
                "Trace Commons public run privacy filter is misconfigured"
            );
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "public run privacy filter unavailable",
            )
        })?
        .map(|(adapter, _)| adapter)
        .ok_or_else(|| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "public run privacy filter unavailable",
            )
        })?;
    validate_public_run_prose_with(adapter.as_ref(), draft).await
}

async fn validate_public_run_prose_with(
    adapter: &dyn trace_commons_protocol::trace_contribution::PrivacyFilterAdapter,
    draft: &trace_commons_protocol::public_run::PublicRunDraft,
) -> ApiResult<()> {
    let public_text = draft.public_text().collect::<Vec<_>>().join("\n\n");
    let filtered = adapter.redact_text(&public_text).await.map_err(|error| {
        tracing::warn!(
            error_hash = %safe_display_error_hash(&error),
            "Trace Commons public run privacy filter failed"
        );
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "public run privacy filter unavailable",
        )
    })?;
    if filtered.is_some_and(|result| {
        result.redacted_text != public_text
            || result.report.blocked_secret_detected
            || result.report.key_finding_detected
            || result.report.coverage_incomplete
            || !result.report.pii_labels_present.is_empty()
    }) {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "public run contains sensitive text",
        ));
    }
    Ok(())
}

fn page_from_data(
    data: trace_commons_server::db::PublicRunPageData,
) -> trace_commons_protocol::public_run::PublicRunPage {
    use trace_commons_protocol::public_run::PublicRunPage;
    let public_url = community_cors_origins().into_iter().find_map(|origin| {
        let origin = origin.to_str().ok()?;
        if origin.starts_with("https://") || origin.starts_with("http://localhost") {
            Some(format!("{origin}/runs/{}", data.slug))
        } else {
            None
        }
    });
    PublicRunPage {
        slug: data.slug,
        title: data.title,
        outcome_summary: data.outcome_summary,
        correction_excerpt: data.correction_excerpt,
        workflow: data.workflow,
        reuse_permission: data.reuse_permission,
        evidence: data.evidence,
        task_success: data.task_success,
        contributed_version: data.contributed_version,
        version: data.version,
        published_at: data.published_at,
        public_url,
        source: data.source,
        source_unavailable: data.source_unavailable,
        variations: data.variations,
    }
}

fn owner_state_from_data(
    data: trace_commons_server::db::PublicRunOwnerData,
) -> anyhow::Result<trace_commons_protocol::public_run::PublicRunOwnerState> {
    let expected_publication_version = data
        .row
        .as_ref()
        .map(|row| u32::try_from(row.version))
        .transpose()?
        .unwrap_or(0);
    if data
        .page
        .as_ref()
        .is_some_and(|page| page.version != expected_publication_version)
    {
        anyhow::bail!("public run owner state is inconsistent");
    }
    Ok(trace_commons_protocol::public_run::PublicRunOwnerState {
        publication: data.page.map(page_from_data),
        expected_publication_version,
        retained_source_slug: data.retained_source_slug,
    })
}

pub(super) async fn community_public_run_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(slug): AxumPath<String>,
) -> ApiResult<axum::response::Response> {
    let client_ip = client_ip_for_rate_limit(&headers);
    if !ACCOUNT_RATE_LIMITER.check("public-run-global", PUBLIC_RUN_GLOBAL_LIMIT)
        || !ACCOUNT_RATE_LIMITER.check(
            &format!("public-run-ip:{client_ip}"),
            PUBLIC_RUN_PER_IP_LIMIT,
        )
    {
        return Err(api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited"));
    }
    let _public_run_slot = ACCOUNT_RATE_LIMITER
        .acquire("public-run-global", PUBLIC_RUN_GLOBAL_CONCURRENCY)
        .ok_or_else(|| api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited"))?;
    trace_commons_protocol::public_run::validate_slug(&slug).map_err(validation_error)?;
    let db = account_db(state.as_ref())?;
    let page = db
        .get_public_run_page_by_slug(&slug, 12)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "public run not found"))?;
    let mut response = Json(page_from_data(page)).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=30"),
    );
    response.headers_mut().insert(
        axum::http::header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

pub(super) async fn account_public_run_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    AxumPath(submission_id): AxumPath<Uuid>,
) -> ApiResult<axum::response::Response> {
    let db = account_db(state.as_ref())?;
    let state = db
        .get_owned_public_run_state(&ctx.tenant_id, ctx.account_id.as_uuid(), submission_id)
        .await
        .map_err(internal_error)?;
    Ok(account_response(
        owner_state_from_data(state).map_err(internal_error)?,
    ))
}

pub(crate) async fn resolve_public_run_source_for_write(
    db: &dyn trace_commons_server::db::Database,
    requested_slug: Option<&str>,
    existing: &trace_commons_server::db::PublicRunOwnerData,
) -> ApiResult<Option<Uuid>> {
    if let Some(slug) = requested_slug {
        if existing.retained_source_slug.as_deref() == Some(slug)
            && let Some(source_publication_id) = existing
                .row
                .as_ref()
                .and_then(|row| row.source_publication_id)
        {
            return Ok(Some(source_publication_id));
        }
        let source = db
            .resolve_public_run_source(slug)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "source public run not found"))?;
        if existing
            .row
            .as_ref()
            .is_some_and(|current| current.publication_id == source.publication_id)
        {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "public run cannot source itself",
            ));
        }
        return Ok(Some(source.publication_id));
    }
    Ok(None)
}

pub(super) async fn account_public_run_publish_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    AxumPath(submission_id): AxumPath<Uuid>,
    Json(request): Json<trace_commons_protocol::public_run::PublicRunPublishRequest>,
) -> ApiResult<axum::response::Response> {
    if !confirm_is_same_origin(&headers) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "cross-origin request denied",
        ));
    }
    request.validate().map_err(validation_error)?;
    let expected_publication_version = i32::try_from(request.expected_publication_version)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "public run invalid"))?;
    let db = account_db(state.as_ref())?;
    let not_found = || api_error(StatusCode::NOT_FOUND, "trace not found");
    let stored = db
        .get_trace_submission(&ctx.tenant_id, submission_id)
        .await
        .map_err(internal_error)?;
    let stored = match stored {
        Some(record) if ctx.principal_set.contains(&record.auth_principal_ref) => record,
        _ => return Err(not_found()),
    };
    if stored.status != StorageTraceCorpusStatus::Accepted {
        return Err(api_error(StatusCode::CONFLICT, "trace is not accepted"));
    }
    let existing = db
        .get_owned_public_run_state(&ctx.tenant_id, ctx.account_id.as_uuid(), submission_id)
        .await
        .map_err(internal_error)?;
    let current_version = existing.row.as_ref().map_or(0, |row| row.version);
    if current_version != expected_publication_version {
        return Err(api_error(StatusCode::CONFLICT, "public run changed"));
    }
    let record = trace_commons_record_from_storage_submission(stored)
        .ok_or_else(not_found)?
        .map_err(internal_error)?;

    let account_key = ctx.account_id.as_uuid().to_string();
    if !ACCOUNT_RATE_LIMITER.check(
        &format!("content-account:{account_key}"),
        CONTENT_PER_ACCOUNT_LIMIT,
    ) {
        return Err(api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited"));
    }
    let _content_slot = ACCOUNT_RATE_LIMITER
        .acquire(
            &format!("content-account:{account_key}"),
            CONTENT_PER_ACCOUNT_CONCURRENCY,
        )
        .ok_or_else(|| api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited"))?;
    validate_public_run_prose(&request.draft).await?;

    let _source_read_slot = ACCOUNT_RATE_LIMITER
        .acquire(
            "public-run-source-read-global",
            PUBLIC_RUN_GLOBAL_CONCURRENCY,
        )
        .ok_or_else(|| api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited"))?;
    let read_state = Arc::clone(&state);
    let envelope_result =
        tokio::task::spawn_blocking(move || read_envelope_by_record(read_state.as_ref(), &record))
            .await
            .map_err(anyhow::Error::from)
            .and_then(|result| result);
    let envelope = match envelope_result {
        Ok(envelope) => envelope,
        Err(error) => {
            tracing::warn!(
                error_hash = %safe_display_error_hash(&error),
                "Trace Commons public run source read failed; failing closed"
            );
            return Err(audit_source_read_failure(
                state.as_ref(),
                &ctx,
                submission_id,
                "account_public_run_publish",
            )
            .await);
        }
    };
    if envelope.submission_id != submission_id {
        return Err(audit_source_read_failure(
            state.as_ref(),
            &ctx,
            submission_id,
            "account_public_run_publish",
        )
        .await);
    }
    if request.task_success != envelope.outcome.task_success
        || request.contributed_version != envelope.schema_version
    {
        return Err(api_error(
            StatusCode::CONFLICT,
            "public run approval changed",
        ));
    }

    append_trace_content_read_audit_per_source(
        state.as_ref(),
        &account_audit_tenant(&ctx),
        submission_id,
        &[],
        "account_public_run_publish",
        None,
    )
    .await
    .map_err(internal_error)?;

    validate_public_run_provenance(&request.draft, &envelope)
        .map_err(|message| api_error(StatusCode::UNPROCESSABLE_ENTITY, message))?;

    let source_publication_id = resolve_public_run_source_for_write(
        db.as_ref(),
        request.draft.source_slug.as_deref(),
        &existing,
    )
    .await?;
    let publication_id = Uuid::new_v4();
    let random_slug = Uuid::new_v4().simple().to_string();
    let mutation = match db
        .upsert_public_run(trace_commons_server::db::PublicRunWrite {
            tenant_id: ctx.tenant_id.clone(),
            publication_id,
            account_id: ctx.account_id.as_uuid(),
            submission_id,
            slug: format!("run-{}", &random_slug[..16]),
            title: request.draft.title,
            outcome_summary: request.draft.outcome_summary,
            correction_excerpt: request.draft.correction_excerpt,
            workflow: request.draft.workflow,
            reuse_permission: request.draft.reuse_permission,
            evidence: request.draft.evidence,
            task_success: request.task_success,
            contributed_version: request.contributed_version,
            approval_sha256: request.approval_sha256,
            source_publication_id,
            expected_publication_version,
        })
        .await
    {
        Ok(row) => row,
        Err(DatabaseError::Constraint(message))
            if message == trace_commons_server::db::PUBLIC_RUN_SOURCE_NOT_ACCEPTED =>
        {
            return Err(api_error(StatusCode::CONFLICT, "trace is not accepted"));
        }
        Err(DatabaseError::Constraint(message))
            if message == trace_commons_server::db::PUBLIC_RUN_PROVENANCE_CYCLE =>
        {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "public run source creates a cycle",
            ));
        }
        Err(DatabaseError::Constraint(message))
            if message == trace_commons_server::db::PUBLIC_RUN_ACCOUNT_CLOSED =>
        {
            return Err(api_error(StatusCode::CONFLICT, "account changed"));
        }
        Err(DatabaseError::Constraint(message))
            if message == trace_commons_server::db::PUBLIC_RUN_VERSION_CONFLICT =>
        {
            return Err(api_error(StatusCode::CONFLICT, "public run changed"));
        }
        Err(error) => return Err(internal_error(error)),
    };
    Ok(account_response(page_from_data(mutation.page)))
}

pub(super) async fn account_public_run_unpublish_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    AxumPath(submission_id): AxumPath<Uuid>,
) -> ApiResult<axum::response::Response> {
    if !confirm_is_same_origin(&headers) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "cross-origin request denied",
        ));
    }
    let db = account_db(state.as_ref())?;
    let mutation = db
        .unpublish_public_run(&ctx.tenant_id, ctx.account_id.as_uuid(), submission_id)
        .await
        .map_err(internal_error)?;
    Ok(account_response(
        trace_commons_protocol::public_run::PublicRunUnpublishResult {
            unpublished: mutation.unpublished,
            expected_publication_version: mutation.expected_publication_version,
        },
    ))
}

#[cfg(test)]
mod privacy_tests {
    use async_trait::async_trait;
    use trace_commons_protocol::public_run::{
        PublicRunDraft, PublicRunEvidenceDraft, PublicRunReusePermission,
    };
    use trace_commons_protocol::trace_contribution::{
        NoopPrivacyFilterAdapter, PrivacyFilterAdapter, SafePrivacyFilterRedaction,
        TraceContributionError, safe_privacy_filter_redaction_from_output,
    };

    use super::*;

    struct FindingAdapter;

    struct ErrorAdapter;

    #[async_trait]
    impl PrivacyFilterAdapter for FindingAdapter {
        async fn redact_text(
            &self,
            _text: &str,
        ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
            safe_privacy_filter_redaction_from_output(&serde_json::json!({
                "redacted_text": "[REDACTED]",
                "detected_spans": [{"label": "person", "start": 0, "end": 4}]
            }))
            .map(Some)
        }
    }

    #[async_trait]
    impl PrivacyFilterAdapter for ErrorAdapter {
        async fn redact_text(
            &self,
            _text: &str,
        ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
            Err(TraceContributionError::TransientRedactionFailed {
                reason: "synthetic-filter-outage".to_string(),
            })
        }
    }

    fn draft() -> PublicRunDraft {
        PublicRunDraft {
            title: "Synthetic workflow".to_string(),
            outcome_summary: "The synthetic task completed.".to_string(),
            correction_excerpt: None,
            workflow: "Apply the bounded synthetic steps.".to_string(),
            reuse_permission: PublicRunReusePermission::CcBy40,
            evidence: vec![PublicRunEvidenceDraft {
                event_id: Uuid::nil(),
                excerpt: "The synthetic result was observed.".to_string(),
            }],
            source_slug: None,
        }
    }

    #[test]
    fn session_detail_projects_every_stored_corpus_status() {
        use trace_commons_protocol::public_run::PublicRunContributionStatus;

        let cases = [
            (
                StorageTraceCorpusStatus::Received,
                PublicRunContributionStatus::Received,
            ),
            (
                StorageTraceCorpusStatus::Accepted,
                PublicRunContributionStatus::Accepted,
            ),
            (
                StorageTraceCorpusStatus::Quarantined,
                PublicRunContributionStatus::Quarantined,
            ),
            (
                StorageTraceCorpusStatus::AwaitingPiiBackstop,
                PublicRunContributionStatus::AwaitingPiiBackstop,
            ),
            (
                StorageTraceCorpusStatus::Rejected,
                PublicRunContributionStatus::Rejected,
            ),
            (
                StorageTraceCorpusStatus::Revoked,
                PublicRunContributionStatus::Revoked,
            ),
            (
                StorageTraceCorpusStatus::Expired,
                PublicRunContributionStatus::Expired,
            ),
            (
                StorageTraceCorpusStatus::Purged,
                PublicRunContributionStatus::Purged,
            ),
        ];

        for (stored, expected) in cases {
            assert_eq!(public_run_contribution_status(stored), expected);
        }
    }

    #[test]
    fn session_detail_permitted_uses_are_typed_and_ordered() {
        let uses =
            public_run_permitted_uses(&["evaluation".to_string(), "model_training".to_string()])
                .expect("stored allowed uses");

        assert_eq!(
            uses,
            vec![TraceAllowedUse::Evaluation, TraceAllowedUse::ModelTraining]
        );
        assert!(public_run_permitted_uses(&["unsupported".to_string()]).is_err());
    }

    #[tokio::test]
    async fn prose_privacy_filter_accepts_clean_text_and_rejects_a_finding() {
        assert!(
            validate_public_run_prose_with(&NoopPrivacyFilterAdapter, &draft())
                .await
                .is_ok()
        );
        let (status, Json(error)) = validate_public_run_prose_with(&FindingAdapter, &draft())
            .await
            .expect_err("classifier finding must fail closed");
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.error, "public run contains sensitive text");
    }

    #[tokio::test]
    async fn prose_privacy_filter_failure_is_unavailable_and_fail_closed() {
        let (status, Json(error)) = validate_public_run_prose_with(&ErrorAdapter, &draft())
            .await
            .expect_err("classifier outage must fail closed");
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.error, "public run privacy filter unavailable");
    }
}
