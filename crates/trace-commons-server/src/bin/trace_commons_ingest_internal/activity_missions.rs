// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Profile-independent catalogue and read-only account activity projection.
use super::*;
use axum::extract::RawQuery;
use trace_commons_protocol::activity_missions::{ActivityCatalogue, ActivityPolicy};

pub(super) fn policy_from_env() -> anyhow::Result<Option<Arc<ActivityPolicy>>> {
    policy_from_raw(std::env::var("TRACE_COMMONS_ACTIVITY_MISSIONS_POLICY_JSON"))
}

/// Absent is unconfigured; anything malformed fails startup. That includes a
/// mission predicate of a version this server cannot validate: the server
/// publishes predicates for clients to match locally, and never evaluates
/// one or accepts a matching result.
pub(super) fn policy_from_raw(
    raw: Result<String, std::env::VarError>,
) -> anyhow::Result<Option<Arc<ActivityPolicy>>> {
    match raw {
        Ok(raw) => ActivityPolicy::parse(raw.as_bytes())
            .map(|p| Some(Arc::new(p)))
            .map_err(|_| anyhow::anyhow!("activity_missions_policy_invalid")),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => anyhow::bail!("activity_missions_policy_invalid"),
    }
}
fn protected(value: impl IntoResponse) -> axum::response::Response {
    let mut response = value.into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}
fn no_query(query: Option<String>) -> ApiResult<()> {
    if query.is_some_and(|query| !query.is_empty()) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "activity_missions_parameters_refused",
        ));
    }
    Ok(())
}
pub(super) async fn catalogue(
    State(state): State<Arc<AppState>>,
    RawQuery(query): RawQuery,
) -> axum::response::Response {
    let result = no_query(query).and_then(|()| {
        ActivityCatalogue::new(state.activity_missions_policy.as_deref().cloned())
            .map(Json)
            .map_err(|_| {
                api_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "activity_missions_policy_invalid",
                )
            })
    });
    protected(result)
}
pub(super) async fn status(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    RawQuery(query): RawQuery,
) -> axum::response::Response {
    protected(status_inner(&state, &ctx, query).await)
}
async fn status_inner(
    state: &AppState,
    ctx: &AccountCtx,
    query: Option<String>,
) -> ApiResult<Json<trace_commons_protocol::activity_missions::ActivityProgress>> {
    no_query(query)?;
    if !ACCOUNT_RATE_LIMITER.check_principal(
        &format!("activity-missions-account:{}", ctx.account_id.as_uuid()),
        30,
    ) {
        return Err(api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited"));
    }
    let policy = state.activity_missions_policy.as_ref().ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "activity_missions_unconfigured",
        )
    })?;
    let now = Utc::now();
    if now.date_naive() < policy.starts_on || now.date_naive() >= policy.ends_before {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "activity_missions_policy_inactive",
        ));
    }
    let starts = policy
        .starts_on
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "activity_missions_policy_invalid",
            )
        })?
        .and_utc();
    let db = account_db(state)?;
    let days = db
        .account_activity_days(
            &ctx.tenant_id,
            &ctx.principal_set.to_vec(),
            starts,
            now,
            policy.qualification,
        )
        .await
        .map_err(|_| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "activity_missions_source_unavailable",
            )
        })?;
    let progress = policy.evaluate(&days, now).map_err(|_| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "activity_missions_source_invalid",
        )
    })?;
    append_control_plane_read_audit(
        state,
        &account_audit_tenant(ctx),
        "account_activity_missions",
        1,
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(progress))
}
