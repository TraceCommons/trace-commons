// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! INTEGRATION: participant routes are covered by the existing account middleware.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use trace_commons_server::account_session::AccountCtx;
use trace_commons_server::mission_rewards::RewardError;
use trace_commons_server::reward_participant::{RewardHistoryQuery, RewardReservationRequest};
use uuid::Uuid;

use crate::{
    ACCOUNT_RATE_LIMITER, AppState, ConcurrencyGuard, account_auth_middleware, api_error,
    client_ip_for_rate_limit, confirm_is_same_origin,
};

type RewardHttpResult = Result<Response, RewardHttpError>;

pub(crate) struct RewardHttpError {
    status: StatusCode,
    label: &'static str,
}

impl RewardHttpError {
    fn new(status: StatusCode, label: &'static str) -> Self {
        Self { status, label }
    }
}

impl IntoResponse for RewardHttpError {
    fn into_response(self) -> Response {
        protected_response(api_error(self.status, self.label))
    }
}

pub(crate) fn account_routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/account/rewards", get(history))
        .route(
            "/v1/account/reward-reservations/{reservation_id}",
            get(reservation),
        )
        .route(
            "/v1/account/reward-offers/{program_id}/reservations",
            post(reserve).layer(DefaultBodyLimit::max(1024)),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            account_auth_middleware,
        ))
        // Covers early authentication refusals as well as handler responses.
        .layer(axum::middleware::map_response(
            |response: Response| async move { protected_response(response) },
        ))
}

fn protected_response(response: impl IntoResponse) -> Response {
    let mut response = response.into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn refusal(error: RewardError) -> RewardHttpError {
    let status = match error {
        RewardError::Unauthorized => StatusCode::UNAUTHORIZED,
        RewardError::RequestInvalid => StatusCode::BAD_REQUEST,
        RewardError::NotFound => StatusCode::NOT_FOUND,
        RewardError::StoreUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::CONFLICT,
    };
    RewardHttpError::new(status, error.label())
}

fn rate_refusal() -> RewardHttpError {
    RewardHttpError::new(StatusCode::TOO_MANY_REQUESTS, "rate limited")
}

fn database_slot() -> Option<ConcurrencyGuard<'static>> {
    // Public and account reward queries share this budget, leaving room in the
    // default five-connection pool for authentication and other account work.
    ACCOUNT_RATE_LIMITER.acquire("reward-database", 2)
}

fn account_slot(ctx: &AccountCtx) -> Option<[ConcurrencyGuard<'static>; 2]> {
    let key = format!(
        "reward-account:{}:{}",
        ctx.tenant_id,
        ctx.account_id.as_uuid()
    );
    if !ACCOUNT_RATE_LIMITER.check("reward-account-global", 1_200)
        || !ACCOUNT_RATE_LIMITER.check(&key, 120)
    {
        return None;
    }
    // Acquire before checking out a database connection. Same-tenant reward
    // operations serialize in PostgreSQL, so excess requests must not queue
    // there while holding the shared pool's connections.
    let tenant = ACCOUNT_RATE_LIMITER.acquire(&format!("reward-tenant:{}", ctx.tenant_id), 1)?;
    let database = database_slot()?;
    Some([tenant, database])
}

fn database(state: &AppState) -> Result<&dyn trace_commons_server::db::Database, RewardError> {
    state
        .db_mirror
        .as_deref()
        .ok_or(RewardError::StoreUnavailable)
}

pub(crate) async fn offer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    path: Result<Path<Uuid>, PathRejection>,
) -> RewardHttpResult {
    if !ACCOUNT_RATE_LIMITER.check("reward-offer-global", 2_000)
        || !ACCOUNT_RATE_LIMITER.check(
            &format!("reward-offer-ip:{}", client_ip_for_rate_limit(&headers)),
            120,
        )
    {
        return Err(rate_refusal());
    }
    let _slot = database_slot().ok_or_else(rate_refusal)?;
    let Path(program) = path.map_err(|_| refusal(RewardError::RequestInvalid))?;
    let offer = database(&state)
        .map_err(refusal)?
        .get_reward_offer(program)
        .await
        .map_err(refusal)?;
    Ok(protected_response(Json(offer)))
}

pub(crate) async fn reserve(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<RewardReservationRequest>, JsonRejection>,
) -> RewardHttpResult {
    if !confirm_is_same_origin(&headers) {
        return Err(RewardHttpError::new(
            StatusCode::FORBIDDEN,
            "cross-origin request denied",
        ));
    }
    // R1 permits each existing account credential to acknowledge its own offer.
    // This holds capacity only; stronger authenticator/payout gates are unchanged.
    let _slot = account_slot(&ctx).ok_or_else(rate_refusal)?;
    let Path(program) = path.map_err(|_| refusal(RewardError::RequestInvalid))?;
    let Json(request) = body.map_err(|error| {
        if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
            RewardHttpError::new(StatusCode::PAYLOAD_TOO_LARGE, "reward_request_too_large")
        } else {
            refusal(RewardError::RequestInvalid)
        }
    })?;
    request.validate().map_err(refusal)?;
    let reservation = database(&state)
        .map_err(refusal)?
        .reserve_reward_offer(&ctx.tenant_id, ctx.account_id.as_uuid(), program, &request)
        .await
        .map_err(refusal)?;
    Ok(protected_response(Json(reservation)))
}

pub(crate) async fn reservation(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    path: Result<Path<Uuid>, PathRejection>,
) -> RewardHttpResult {
    let _slot = account_slot(&ctx).ok_or_else(rate_refusal)?;
    let Path(reservation) = path.map_err(|_| refusal(RewardError::RequestInvalid))?;
    let reservation = database(&state)
        .map_err(refusal)?
        .get_reward_reservation(&ctx.tenant_id, ctx.account_id.as_uuid(), reservation)
        .await
        .map_err(refusal)?;
    Ok(protected_response(Json(reservation)))
}

pub(crate) async fn history(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    query: Result<Query<RewardHistoryQuery>, QueryRejection>,
) -> RewardHttpResult {
    let _slot = account_slot(&ctx).ok_or_else(rate_refusal)?;
    let Query(query) = query.map_err(|_| refusal(RewardError::RequestInvalid))?;
    query.validate().map_err(refusal)?;
    let history = database(&state)
        .map_err(refusal)?
        .get_reward_history(&ctx.tenant_id, ctx.account_id.as_uuid(), &query)
        .await
        .map_err(refusal)?;
    Ok(protected_response(Json(history)))
}
