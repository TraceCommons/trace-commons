// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `POST /v1/account/invites/legacy-link[/challenge]`: link a legacy
//! `tenant-…` invite tenant to the signed-in NEAR account (V81).
//!
//! Account-session only, like `/v1/account/invites/redeem`: a device bearer
//! is refused, and a cookie session must be same-origin. Neither the account
//! tenant nor the account id is accepted from the body. Off unless
//! `TRACE_COMMONS_LEGACY_INVITE_LINK_ENABLED` is set; see
//! docs/operator/legacy-invite-migration.md.

use super::*;
use trace_commons_protocol::legacy_invite_link::{
    LegacyInviteLinkChallenge, LegacyInviteLinkRequest, LegacyInviteLinkResponse,
};
use trace_commons_server::legacy_invite_link::{
    LegacyInviteLinkSigner, LinkRefusal, issue_challenge, link_legacy_invite,
};

pub(super) const NOT_ENABLED: &str = "legacy_invite_link_not_enabled";

pub(super) fn routes() -> crate::account_routes::AccountRoutes {
    crate::account_routes::AccountRoutes::new()
        .post(
            "/v1/account/invites/legacy-link/challenge",
            challenge_handler,
        )
        .post("/v1/account/invites/legacy-link", link_handler)
}

fn no_store() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    headers
}

fn require_account_session(ctx: &AccountCtx, headers: &HeaderMap) -> ApiResult<()> {
    if matches!(ctx.auth_method, AccountAuthMethod::DeviceBearer) {
        return Err(api_error(StatusCode::FORBIDDEN, "account session required"));
    }
    if matches!(ctx.auth_method, AccountAuthMethod::SessionCookie)
        && !confirm_is_same_origin(headers)
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "cross-origin account mutation",
        ));
    }
    Ok(())
}

fn signer(state: &AppState) -> ApiResult<&LegacyInviteLinkSigner> {
    state
        .legacy_invite_link
        .as_deref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, NOT_ENABLED))
}

fn refusal_status(refusal: LinkRefusal) -> StatusCode {
    match refusal {
        LinkRefusal::InvalidRequest => StatusCode::BAD_REQUEST,
        LinkRefusal::TooManyChallenges => StatusCode::TOO_MANY_REQUESTS,
        LinkRefusal::TenantPooled | LinkRefusal::TenantClaimed => StatusCode::CONFLICT,
        LinkRefusal::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        LinkRefusal::SignatureInvalid
        | LinkRefusal::ChallengeInvalid
        | LinkRefusal::AccountIneligible
        | LinkRefusal::DeviceNotEligible
        | LinkRefusal::InviteRevoked => StatusCode::FORBIDDEN,
    }
}

fn refused(refusal: LinkRefusal) -> (StatusCode, Json<ApiError>) {
    api_error(refusal_status(refusal), refusal.label())
}

pub(super) async fn challenge_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
) -> ApiResult<(HeaderMap, Json<LegacyInviteLinkChallenge>)> {
    require_account_session(&ctx, &headers)?;
    signer(state.as_ref())?;
    let db = account_db(state.as_ref())?;
    let challenge = issue_challenge(db.as_ref(), &ctx.tenant_id, ctx.account_id.as_uuid())
        .await
        .map_err(refused)?;
    Ok((no_store(), Json(challenge)))
}

pub(super) async fn link_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    Json(body): Json<LegacyInviteLinkRequest>,
) -> ApiResult<(HeaderMap, Json<LegacyInviteLinkResponse>)> {
    require_account_session(&ctx, &headers)?;
    let signer = signer(state.as_ref())?;
    let db = account_db(state.as_ref())?;
    let response = link_legacy_invite(
        db.as_ref(),
        signer,
        &ctx.tenant_id,
        ctx.account_id.as_uuid(),
        &body,
    )
    .await
    .map_err(refused)?;
    Ok((no_store(), Json(response)))
}
