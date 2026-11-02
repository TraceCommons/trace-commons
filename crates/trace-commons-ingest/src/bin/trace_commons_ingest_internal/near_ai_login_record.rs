// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Recording the NEAR AI login on an existing account (owner decision
//! 2026-11-02, option (a) on #1841).
//!
//! `POST /v1/account/near-ai-login` with a fresh NEAR AI access token. The
//! account session says which account; the token, spent once on NEAR AI's
//! `/users/me`, says which login. The login's blind index becomes the
//! account's `near_ai_login` anchor, so a payout link can be proved against
//! it and a later NEAR AI sign-in with that login lands on this account.
//!
//! Trust on first use: an account takes one login and a login anchors one
//! account. Recording the same login again answers `already_recorded`;
//! another login, or a login another account holds, answers
//! `near_ai_login_unavailable` either way, so the route says nothing about
//! which. Only accounts in a NEAR account tenant (`near-` / `nearai-`) take
//! one: anywhere else a later NEAR AI sign-in would put a `near_ai` device
//! in a legacy tenant, which account-admission readiness refuses
//! deployment-wide. Those, and closed accounts, answer
//! `near_ai_login_account_ineligible`, before the token is spent.
//!
//! Hash-only: the response, logs and the audit row carry labels, never the
//! token, the subject, the provider or an account id.

use super::*;
use trace_commons_protocol::payout_link::{
    NEAR_AI_LOGIN_RECORD_PATH, NearAiLoginRecordOutcome as Outcome, NearAiLoginRecordRequest,
    NearAiLoginRecorded, PAYOUT_LINK_CONTRACT_VERSION,
};
use trace_commons_server::account_onboarding::NearAiLoginRecord;

pub(super) fn routes() -> crate::ingest::app::account_routes::AccountRoutes {
    crate::ingest::app::account_routes::AccountRoutes::new()
        .post(NEAR_AI_LOGIN_RECORD_PATH, handler)
}

fn refuse(status: StatusCode, outcome: Outcome) -> (StatusCode, Json<ApiError>) {
    tracing::info!(outcome = outcome.as_str(), "near ai login record refused");
    api_error(status, outcome.as_str())
}

fn unavailable() -> (StatusCode, Json<ApiError>) {
    refuse(
        StatusCode::SERVICE_UNAVAILABLE,
        Outcome::NearAiLoginRecordUnavailable,
    )
}

/// `near-` or `nearai-` and 64 lowercase hex: the tenants made for a NEAR
/// account. The store checks the same; this answers before the token is
/// spent.
fn is_near_account_tenant(tenant: &str) -> bool {
    tenant
        .strip_prefix("nearai-")
        .or_else(|| tenant.strip_prefix("near-"))
        .is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

/// The account's hash on the audit row, as the payout links write it. Never
/// the account id.
fn actor_hash(ctx: &AccountCtx) -> String {
    sha256_prefixed(&format!(
        "near-ai-login-record-actor-v1:{}",
        ctx.account_id.as_uuid()
    ))
}

pub(super) async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    body: Result<Json<NearAiLoginRecordRequest>, JsonRejection>,
) -> ApiResult<(HeaderMap, Json<NearAiLoginRecorded>)> {
    // An account session, as for a payout link: never a device bearer, and
    // a cookie session only from the same origin.
    if matches!(ctx.auth_method, AccountAuthMethod::DeviceBearer) {
        return Err(api_error(StatusCode::FORBIDDEN, "account session required"));
    }
    if matches!(ctx.auth_method, AccountAuthMethod::SessionCookie)
        && !confirm_is_same_origin(&headers)
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "cross-origin account mutation",
        ));
    }
    let db = account_db(&state).map_err(|_| unavailable())?;
    let identity = state
        .near_account_identity
        .clone()
        .ok_or_else(unavailable)?;
    let Ok(Json(body)) = body else {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            Outcome::InvalidNearAiLoginRequest,
        ));
    };
    if body.validate().is_err() {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            Outcome::InvalidNearAiLoginRequest,
        ));
    }
    if !is_near_account_tenant(&ctx.tenant_id) {
        return Err(refuse(
            StatusCode::FORBIDDEN,
            Outcome::NearAiLoginAccountIneligible,
        ));
    }
    // The token becomes a secret here and is dropped when this returns.
    let token = secrecy::SecretString::from(body.near_ai_access_token);
    let login = trace_commons_server::near_ai_login::introspect_login(
        near_provisioning::introspection_base_url(&state),
        &token,
        near_provisioning::NEAR_AI_INTROSPECTION_TIMEOUT,
    )
    .await
    .map_err(|_| refuse(StatusCode::FORBIDDEN, Outcome::NearAiLoginUnverified))?;
    drop(token);
    let recorded = db
        .record_near_ai_login_anchor(
            &ctx.tenant_id,
            ctx.account_id.as_uuid(),
            &actor_hash(&ctx),
            &login,
            &identity,
        )
        .await
        .map_err(|_| unavailable())?;
    let outcome = match recorded {
        NearAiLoginRecord::Recorded => Outcome::Recorded,
        NearAiLoginRecord::AlreadyRecorded => Outcome::AlreadyRecorded,
        NearAiLoginRecord::Unavailable => {
            return Err(refuse(
                StatusCode::CONFLICT,
                Outcome::NearAiLoginUnavailable,
            ));
        }
        NearAiLoginRecord::AccountIneligible => {
            return Err(refuse(
                StatusCode::FORBIDDEN,
                Outcome::NearAiLoginAccountIneligible,
            ));
        }
    };
    tracing::info!(outcome = outcome.as_str(), "near ai login recorded");
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    Ok((
        response_headers,
        Json(NearAiLoginRecorded {
            contract_version: PAYOUT_LINK_CONTRACT_VERSION.to_owned(),
            outcome,
        }),
    ))
}
