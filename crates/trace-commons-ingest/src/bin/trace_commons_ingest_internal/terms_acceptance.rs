// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `POST /v1/account/terms-acceptance`: the contributor app reports which
//! version of the Terms of Service the signed-in person accepted, and the
//! SHA-256 of the exact document bytes it showed them.
//!
//! The server only records. It never refuses an upload over terms, and it
//! does not check the hash against the published document. Tenant and
//! account come from the authenticated context alone; the body carries the
//! version id, the hash and the client's claimed time. Labels and the
//! version id are all that reach a log line.

use super::*;
use trace_commons_protocol::terms::{TermsAcceptanceRequest, TermsAcceptanceResponse};

pub(super) async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    Json(body): Json<TermsAcceptanceRequest>,
) -> ApiResult<(HeaderMap, Json<TermsAcceptanceResponse>)> {
    if matches!(ctx.auth_method, AccountAuthMethod::SessionCookie)
        && !confirm_is_same_origin(&headers)
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "cross-origin account mutation",
        ));
    }
    body.validate(chrono::Utc::now())
        .map_err(|label| api_error(StatusCode::BAD_REQUEST, label))?;
    let recorded_at = account_db(state.as_ref())?
        .record_terms_acceptance(
            &ctx.tenant_id,
            ctx.account_id.as_uuid(),
            ctx.auth_method.as_str(),
            &ctx.client_kind,
            &body.terms_version,
            &body.content_sha256,
            body.accepted_at,
        )
        .await
        .map_err(internal_error)?;
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    Ok((
        response_headers,
        Json(TermsAcceptanceResponse { recorded_at }),
    ))
}
