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
use trace_commons_protocol::mission_catalog::MissionCatalogQuery;
use trace_commons_server::account_session::AccountCtx;
use trace_commons_server::mission_rewards::RewardError;
use trace_commons_server::reward_participant::{RewardHistoryQuery, RewardReservationRequest};
use uuid::Uuid;

use crate::{
    ACCOUNT_RATE_LIMITER, AccountRateLimiter, AppState, ConcurrencyGuard, account_auth_middleware,
    api_error, client_ip_for_rate_limit, confirm_is_same_origin,
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
        // A missing control, not a client fault: the deployment never provisioned
        // the participant database login.
        RewardError::ParticipantUnprovisioned | RewardError::StoreUnavailable => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        _ => StatusCode::CONFLICT,
    };
    RewardHttpError::new(status, error.label())
}

pub(crate) fn public_mission_error(error: RewardError) -> RewardError {
    match error {
        RewardError::OfferSuspended | RewardError::ProgramClosed => RewardError::NotFound,
        error => error,
    }
}

fn rate_refusal() -> RewardHttpError {
    RewardHttpError::new(StatusCode::TOO_MANY_REQUESTS, "rate limited")
}

// Public and account reward queries share this budget, leaving room in the
// default five-connection pool for authentication and other account work.
const REWARD_DATABASE_SLOTS: u32 = 3;
// Anonymous reads get their own admission strictly below the shared budget, so
// a slot always remains for authenticated account work no matter how many
// anonymous readers arrive. A bound of one would have serialized every
// anonymous reader behind a single slow request.
const REWARD_PUBLIC_READ_SLOTS: u32 = REWARD_DATABASE_SLOTS - 1;

fn database_slot() -> Option<ConcurrencyGuard<'static>> {
    database_slot_for(&ACCOUNT_RATE_LIMITER)
}

fn database_slot_for(limiter: &AccountRateLimiter) -> Option<ConcurrencyGuard<'_>> {
    limiter.acquire("reward-database", REWARD_DATABASE_SLOTS)
}

fn public_read_slots_for(limiter: &AccountRateLimiter) -> Option<[ConcurrencyGuard<'_>; 2]> {
    // Anonymous offer, catalog, and detail reads share this bounded admission
    // inside the reward database budget.
    let public = limiter.acquire("reward-public-read", REWARD_PUBLIC_READ_SLOTS)?;
    let database = database_slot_for(limiter)?;
    Some([public, database])
}

/// Reservations take the tenant's EXCLUSIVE advisory lock, so only one may be
/// in flight per tenant.
fn account_slot(ctx: &AccountCtx) -> Option<[ConcurrencyGuard<'static>; 2]> {
    account_slot_in(ctx, "reward-tenant", 1)
}

/// Status and history take only the SHARED advisory lock, so they neither need
/// nor benefit from the write path's serialization. Sharing its slot of one
/// made a single in-flight reservation refuse every other participant's read in
/// that tenant. They stay bounded by the reward database budget instead.
fn account_read_slot(ctx: &AccountCtx) -> Option<[ConcurrencyGuard<'static>; 2]> {
    account_slot_in(ctx, "reward-tenant-read", 2)
}

fn account_slot_in(
    ctx: &AccountCtx,
    namespace: &str,
    tenant_limit: u32,
) -> Option<[ConcurrencyGuard<'static>; 2]> {
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
    let tenant =
        ACCOUNT_RATE_LIMITER.acquire(&format!("{namespace}:{}", ctx.tenant_id), tenant_limit)?;
    let database = database_slot()?;
    Some([tenant, database])
}

fn database(state: &AppState) -> Result<&dyn trace_commons_server::db::Database, RewardError> {
    state
        .db_mirror
        .as_deref()
        .ok_or(RewardError::StoreUnavailable)
}

fn public_slot(headers: &HeaderMap, resource: &str) -> Option<[ConcurrencyGuard<'static>; 2]> {
    if !ACCOUNT_RATE_LIMITER.check(&format!("reward-{resource}-global"), 2_000)
        || !ACCOUNT_RATE_LIMITER.check(
            &format!("reward-{resource}-ip:{}", client_ip_for_rate_limit(headers)),
            120,
        )
    {
        return None;
    }
    public_read_slots_for(&ACCOUNT_RATE_LIMITER)
}

pub(crate) async fn offer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    path: Result<Path<Uuid>, PathRejection>,
) -> RewardHttpResult {
    let _slot = public_slot(&headers, "offer").ok_or_else(rate_refusal)?;
    let Path(program) = path.map_err(|_| refusal(RewardError::RequestInvalid))?;
    let offer = database(&state)
        .map_err(refusal)?
        .get_reward_offer(program)
        .await
        .map_err(refusal)?;
    Ok(protected_response(Json(offer)))
}

pub(crate) async fn mission_catalog(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    query: Result<Query<MissionCatalogQuery>, QueryRejection>,
) -> RewardHttpResult {
    let _slot = public_slot(&headers, "mission-catalog").ok_or_else(rate_refusal)?;
    let Query(query) = query.map_err(|_| refusal(RewardError::RequestInvalid))?;
    query
        .validate()
        .map_err(|_| refusal(RewardError::RequestInvalid))?;
    let page = database(&state)
        .map_err(refusal)?
        .list_mission_catalog(&query)
        .await
        .map_err(refusal)?;
    Ok(protected_response(Json(page)))
}

pub(crate) async fn mission_publication(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    path: Result<Path<Uuid>, PathRejection>,
) -> RewardHttpResult {
    let _slot = public_slot(&headers, "mission-publication").ok_or_else(rate_refusal)?;
    let Path(mission) = path.map_err(|_| refusal(RewardError::RequestInvalid))?;
    if mission.is_nil() {
        return Err(refusal(RewardError::RequestInvalid));
    }
    let publication = database(&state)
        .map_err(refusal)?
        .get_mission_publication(mission)
        .await
        .map_err(|error| refusal(public_mission_error(error)))?;
    Ok(protected_response(Json(publication)))
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
    let _slot = account_read_slot(&ctx).ok_or_else(rate_refusal)?;
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
    let _slot = account_read_slot(&ctx).ok_or_else(rate_refusal)?;
    let Query(query) = query.map_err(|_| refusal(RewardError::RequestInvalid))?;
    query.validate().map_err(refusal)?;
    let history = database(&state)
        .map_err(refusal)?
        .get_reward_history(&ctx.tenant_id, ctx.account_id.as_uuid(), &query)
        .await
        .map_err(refusal)?;
    Ok(protected_response(Json(history)))
}

#[cfg(test)]
mod limiter_tests {
    use super::*;

    #[test]
    fn unprovisioned_participant_login_refuses_as_a_missing_control() {
        let missing_control = refusal(RewardError::ParticipantUnprovisioned);
        assert_eq!(missing_control.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(missing_control.label, "reward_participant_unprovisioned");
        assert_eq!(
            refusal(RewardError::Unauthorized).status,
            StatusCode::UNAUTHORIZED,
            "a credential failure stays a credential failure"
        );
    }

    #[test]
    fn concurrent_public_reads_are_admitted_up_to_their_own_bound() {
        let limiter = AccountRateLimiter::new();
        let first = public_read_slots_for(&limiter).expect("first anonymous read is admitted");
        // A bound of one made every anonymous reader wait behind one slow
        // request, which the pre-existing offer endpoint never did.
        let second = public_read_slots_for(&limiter).expect("second anonymous read is admitted");

        let refusal = public_read_slots_for(&limiter)
            .map(|_| ())
            .ok_or_else(rate_refusal)
            .expect_err("a third concurrent anonymous read is refused");
        assert_eq!(refusal.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(refusal.label, "rate limited");

        drop(second);
        assert!(public_read_slots_for(&limiter).is_some());
        drop(first);
    }

    #[test]
    fn saturated_public_reads_still_leave_a_database_slot_for_account_work() {
        let limiter = AccountRateLimiter::new();
        let _first = public_read_slots_for(&limiter).expect("first anonymous read is admitted");
        let _second = public_read_slots_for(&limiter).expect("second anonymous read is admitted");
        assert!(
            public_read_slots_for(&limiter).is_none(),
            "anonymous admission stays bounded"
        );

        let account = database_slot_for(&limiter)
            .expect("account work keeps a reward database slot while anonymous reads saturate");
        assert!(
            database_slot_for(&limiter).is_none(),
            "the shared reward database budget is not widened beyond its cap"
        );
        drop(account);
        assert!(database_slot_for(&limiter).is_some());
    }
}
