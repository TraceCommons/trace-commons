// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Referral codes and applying one
//! (docs/superpowers/specs/2026-10-29-referral-rewards-design.md, sections
//! 2, 3 and 6; wire types in `trace_commons_protocol::referral`).
//!
//! - `POST /v1/account/referral-codes` makes a code: ten random Crockford
//!   symbols, sealed and indexed under the NEAR account identity keys, at
//!   most five live per account. The answer is the only time the code is
//!   sent unasked.
//! - `GET /v1/account/referral-codes` lists the account's codes, opened
//!   from their seals, with referee counts and bonus owed and sent.
//! - `POST /v1/account/referral-codes/{code_id}/disable` turns one off.
//! - `POST /v1/account/referral` applies a code to the signed-in account
//!   (`applied_via = 'later'`).
//! - `GET /v1/account/referral` says whether one is applied, never whose.
//!
//! [`signup_referral`] applies a code sent with a NEAR AI or wallet
//! provisioning finish, after that transaction committed
//! (`applied_via = 'signup'`). It never fails provisioning.
//!
//! **Authority**, as the payout link routes (`payout_link_routes`): any
//! signed-in account session, a native token or a same-origin cookie
//! session. A device bearer is not an account session and is refused.
//!
//! **One outward refusal.** Every refused apply answers
//! `referral_not_applied`, whatever the reason; the precise label goes only
//! to the referee's `trace_referral_audit` row, so codes cannot be probed.
//!
//! Hash-only: responses and logs carry labels, counts and code ids, never a
//! code's text in a log, a code index, a tenant or an account id.

use super::*;
use trace_commons_protocol::referral::{
    ApplyReferralRequest, INVALID_REFERRAL_REQUEST, MAX_LIVE_REFERRAL_CODES,
    MAX_REFERRAL_CODE_INPUT_LEN, REFERRAL_ACCOUNT_INELIGIBLE, REFERRAL_CODE_LIMIT_REACHED,
    REFERRAL_CODE_NOT_FOUND, REFERRAL_CONTRACT_VERSION, REFERRAL_NOT_APPLIED, REFERRAL_UNAVAILABLE,
    ReferralApplied, ReferralAppliedVia, ReferralCodeCreated, ReferralCodeDisabled,
    ReferralCodeState, ReferralCodeSummary, ReferralCodesList, ReferralOutcome, ReferralStatus,
    format_referral_code, normalize_referral_code,
};
use trace_commons_server::db::{
    ReferralApply, ReferralCodeCreate, ReferralCodeDisable, ReferralCodeRow, ReferralStore,
};
use trace_commons_server::near_account_identity::NearAccountIdentity;
use trace_commons_server::referral_code::{
    generate_referral_code, open_referral_code, referral_code_index, seal_referral_code,
};

/// Fresh codes drawn before a create gives up on index collisions. With 50
/// random bits a second draw is already vanishingly rare.
const CODE_DRAWS: usize = 3;

pub(super) fn routes() -> crate::ingest::app::account_routes::AccountRoutes {
    crate::ingest::app::account_routes::AccountRoutes::new()
        .get("/v1/account/referral-codes", list_handler)
        .post("/v1/account/referral-codes", create_handler)
        .post(
            "/v1/account/referral-codes/{code_id}/disable",
            disable_handler,
        )
        .get("/v1/account/referral", status_handler)
        .post("/v1/account/referral", apply_handler)
        .route_layer_fn(no_store_middleware)
}

async fn no_store_middleware(request: Request, next: Next) -> axum::response::Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response
}

/// Any signed-in account session; a device bearer may not, and a cookie
/// session only from the same origin for a change.
fn require_account_session(ctx: &AccountCtx, headers: &HeaderMap, mutation: bool) -> ApiResult<()> {
    if matches!(ctx.auth_method, AccountAuthMethod::DeviceBearer) {
        return Err(api_error(StatusCode::FORBIDDEN, "account session required"));
    }
    if mutation
        && matches!(ctx.auth_method, AccountAuthMethod::SessionCookie)
        && !confirm_is_same_origin(headers)
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "cross-origin account mutation",
        ));
    }
    Ok(())
}

fn unavailable() -> (StatusCode, Json<ApiError>) {
    api_error(StatusCode::SERVICE_UNAVAILABLE, REFERRAL_UNAVAILABLE)
}

fn store_failed(_: trace_commons_server::error::DatabaseError) -> (StatusCode, Json<ApiError>) {
    unavailable()
}

fn referral_store(state: &AppState) -> ApiResult<&Arc<dyn ReferralStore>> {
    state.referral_store.as_ref().ok_or_else(unavailable)
}

/// The keys every code is sealed and indexed under: without them nothing
/// could be found, so nothing is made or applied.
fn referral_identity(state: &AppState) -> ApiResult<&Arc<NearAccountIdentity>> {
    state.near_account_identity.as_ref().ok_or_else(unavailable)
}

fn code_state(row: &ReferralCodeRow) -> ReferralCodeState {
    if row.voided_at.is_some() {
        ReferralCodeState::Voided
    } else if row.suspended_at.is_some() {
        ReferralCodeState::Suspended
    } else if row.disabled_at.is_some() {
        ReferralCodeState::Disabled
    } else {
        ReferralCodeState::Live
    }
}

/// An amount for display: never below zero, clamped to `u64`.
fn nano(amount: i128) -> u64 {
    u64::try_from(amount.max(0)).unwrap_or(u64::MAX)
}

pub(super) async fn create_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
) -> ApiResult<(HeaderMap, Json<ReferralCodeCreated>)> {
    require_account_session(&ctx, &headers, true)?;
    let store = referral_store(&state)?;
    let identity = referral_identity(&state)?;
    // A code is useless before the program opens, so none is made.
    if !state.referral_config.is_open_at(Utc::now()) {
        return Err(unavailable());
    }
    let tenant = ctx.tenant_id.as_str();
    let account = ctx.account_id.as_uuid();
    for _ in 0..CODE_DRAWS {
        let code = generate_referral_code();
        let code_id = Uuid::new_v4();
        let write = seal_referral_code(identity, code_id, &code).map_err(|_| unavailable())?;
        match store
            .create_referral_code(tenant, account, &write)
            .await
            .map_err(store_failed)?
        {
            ReferralCodeCreate::Created { created_at } => {
                tracing::info!(outcome = "code_created", "referral code created");
                let shown = format_referral_code(&code).ok_or_else(unavailable)?;
                return Ok((
                    no_store(),
                    Json(ReferralCodeCreated {
                        contract_version: REFERRAL_CONTRACT_VERSION.to_owned(),
                        code_id,
                        code: shown,
                        created_at,
                    }),
                ));
            }
            ReferralCodeCreate::IndexTaken => continue,
            ReferralCodeCreate::LimitReached => {
                return Err(api_error(StatusCode::CONFLICT, REFERRAL_CODE_LIMIT_REACHED));
            }
            ReferralCodeCreate::AccountIneligible => {
                return Err(api_error(
                    StatusCode::FORBIDDEN,
                    REFERRAL_ACCOUNT_INELIGIBLE,
                ));
            }
        }
    }
    tracing::warn!(
        outcome = "referral_code_index_collisions",
        draws = CODE_DRAWS,
        "referral code create gave up"
    );
    Err(unavailable())
}

pub(super) async fn list_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
) -> ApiResult<(HeaderMap, Json<ReferralCodesList>)> {
    require_account_session(&ctx, &headers, false)?;
    let store = referral_store(&state)?;
    let identity = referral_identity(&state)?;
    let view = store
        .referral_codes(&ctx.tenant_id, ctx.account_id.as_uuid())
        .await
        .map_err(store_failed)?;
    let codes = view
        .codes
        .iter()
        .map(|row| ReferralCodeSummary {
            code_id: row.code_id,
            code: open_referral_code(
                identity,
                &row.code_index,
                &row.sealed_code,
                &row.code_key_ref,
                &row.index_pepper_ref,
            )
            .as_deref()
            .and_then(format_referral_code),
            state: code_state(row),
            created_at: row.created_at,
            referee_count: row.referee_count,
            bonus_owed_nano_usd: nano(row.bonus_owed),
        })
        .collect();
    Ok((
        no_store(),
        Json(ReferralCodesList {
            contract_version: REFERRAL_CONTRACT_VERSION.to_owned(),
            codes,
            max_live_codes: MAX_LIVE_REFERRAL_CODES,
            bonus_owed_nano_usd: nano(view.bonus_owed),
            bonus_sent_nano_usd: u64::try_from(view.bonus_sent).unwrap_or(u64::MAX),
        }),
    ))
}

pub(super) async fn disable_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    AxumPath(code_id): AxumPath<String>,
) -> ApiResult<(HeaderMap, Json<ReferralCodeDisabled>)> {
    require_account_session(&ctx, &headers, true)?;
    let store = referral_store(&state)?;
    let not_found = || api_error(StatusCode::NOT_FOUND, REFERRAL_CODE_NOT_FOUND);
    let code_id = Uuid::parse_str(&code_id).map_err(|_| not_found())?;
    let tenant = ctx.tenant_id.as_str();
    let account = ctx.account_id.as_uuid();
    let outcome = store
        .disable_referral_code(tenant, account, code_id)
        .await
        .map_err(store_failed)?;
    if outcome == ReferralCodeDisable::NotFound {
        return Err(not_found());
    }
    // Read back the state it is in now: disabled, or what it already was.
    let view = store
        .referral_codes(tenant, account)
        .await
        .map_err(store_failed)?;
    let row = view
        .codes
        .iter()
        .find(|row| row.code_id == code_id)
        .ok_or_else(not_found)?;
    tracing::info!(
        outcome = match outcome {
            ReferralCodeDisable::Disabled => "code_disabled",
            _ => "code_already_disabled",
        },
        "referral code disabled"
    );
    Ok((
        no_store(),
        Json(ReferralCodeDisabled {
            contract_version: REFERRAL_CONTRACT_VERSION.to_owned(),
            code_id,
            state: code_state(row),
        }),
    ))
}

/// Applies `code` as typed to the account, returning the store's outcome.
/// `Err` only when the store could not be reached; every refusal is an
/// `Ok(Refused)` already recorded in the audit table.
async fn apply(
    state: &AppState,
    store: &Arc<dyn ReferralStore>,
    identity: &NearAccountIdentity,
    tenant: &str,
    account: Uuid,
    code: &str,
    applied_via: ReferralAppliedVia,
) -> Result<ReferralApply, trace_commons_server::error::DatabaseError> {
    // Text that is not a code still goes to the store, which records it as
    // an unknown code (and counts it toward the rate limit) after the
    // program and rate checks, in the spec's order.
    let index = (code.len() <= MAX_REFERRAL_CODE_INPUT_LEN)
        .then(|| normalize_referral_code(code))
        .flatten()
        .map(|normalized| referral_code_index(identity, &normalized));
    let outcome = store
        .apply_referral_code(
            tenant,
            account,
            index.as_deref(),
            applied_via,
            &state.referral_config.apply_policy(),
        )
        .await?;
    match outcome {
        ReferralApply::Applied { .. } => tracing::info!(
            applied_via = applied_via.as_str(),
            outcome = "referral_applied",
            "referral applied"
        ),
        ReferralApply::Refused(refusal) => tracing::info!(
            applied_via = applied_via.as_str(),
            outcome = refusal.label(),
            "referral not applied"
        ),
    }
    Ok(outcome)
}

pub(super) async fn apply_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    body: Result<Json<ApplyReferralRequest>, JsonRejection>,
) -> ApiResult<(HeaderMap, Json<ReferralApplied>)> {
    require_account_session(&ctx, &headers, true)?;
    let store = referral_store(&state)?;
    let identity = referral_identity(&state)?;
    let Ok(Json(body)) = body else {
        return Err(api_error(StatusCode::BAD_REQUEST, INVALID_REFERRAL_REQUEST));
    };
    match apply(
        &state,
        store,
        identity,
        &ctx.tenant_id,
        ctx.account_id.as_uuid(),
        &body.code,
        ReferralAppliedVia::Later,
    )
    .await
    .map_err(store_failed)?
    {
        ReferralApply::Applied {
            applied_at,
            referee_account_created_at,
        } => Ok((
            no_store(),
            Json(ReferralApplied {
                contract_version: REFERRAL_CONTRACT_VERSION.to_owned(),
                outcome: ReferralOutcome::Applied,
                applied_at,
                earn_until: state.referral_config.earn_until(referee_account_created_at),
            }),
        )),
        ReferralApply::Refused(_) => Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            REFERRAL_NOT_APPLIED,
        )),
    }
}

pub(super) async fn status_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
) -> ApiResult<(HeaderMap, Json<ReferralStatus>)> {
    require_account_session(&ctx, &headers, false)?;
    let store = referral_store(&state)?;
    let status = store
        .referral_status(&ctx.tenant_id, ctx.account_id.as_uuid())
        .await
        .map_err(store_failed)?;
    let now = Utc::now();
    let body = match status.attribution {
        Some(attribution) => ReferralStatus {
            contract_version: REFERRAL_CONTRACT_VERSION.to_owned(),
            applied: true,
            applied_at: Some(attribution.applied_at),
            applied_via: Some(attribution.applied_via),
            earn_until: (!attribution.voided).then(|| {
                state
                    .referral_config
                    .earn_until(attribution.referee_account_created_at)
            }),
            apply_until: None,
        },
        None => ReferralStatus {
            contract_version: REFERRAL_CONTRACT_VERSION.to_owned(),
            applied: false,
            applied_at: None,
            applied_via: None,
            earn_until: None,
            apply_until: status
                .account_created_at
                .filter(|_| state.referral_config.is_open_at(now))
                .and_then(|created| state.referral_config.apply_until(created))
                .filter(|until| *until > now),
        },
    };
    Ok((no_store(), Json(body)))
}

/// Applies a code sent with a provisioning finish, after the provisioning
/// transaction committed. Never fails: a missing store or key, a store
/// error and every refusal are all `not_applied`, and the account stands.
pub(super) async fn signup_referral(
    state: &AppState,
    tenant: &str,
    account: Uuid,
    code: &str,
) -> ReferralOutcome {
    let (Some(store), Some(identity)) = (
        state.referral_store.as_ref(),
        state.near_account_identity.as_ref(),
    ) else {
        tracing::info!(
            applied_via = "signup",
            outcome = REFERRAL_UNAVAILABLE,
            "referral not applied"
        );
        return ReferralOutcome::NotApplied;
    };
    match apply(
        state,
        store,
        identity,
        tenant,
        account,
        code,
        ReferralAppliedVia::Signup,
    )
    .await
    {
        Ok(ReferralApply::Applied { .. }) => ReferralOutcome::Applied,
        Ok(ReferralApply::Refused(_)) => ReferralOutcome::NotApplied,
        Err(error) => {
            tracing::warn!(
                applied_via = "signup",
                error_class = "ReferralStoreFailed",
                error_hash = %safe_display_error_hash(&error),
                "referral not applied"
            );
            ReferralOutcome::NotApplied
        }
    }
}

fn no_store() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    headers
}
