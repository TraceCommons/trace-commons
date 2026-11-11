// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Organizations on ingest (docs/superpowers/specs/2026-11-11-enterprise-gateway-design.md).
//!
//! - [`check_submission`]: an upload into an `org-` tenant is refused unless
//!   the org's record exists, the org is active with sharing on, the
//!   uploading device's seat is still seated (and, for a member, still a
//!   member), and the envelope's scopes and uses are inside the org's current
//!   policy. These are re-checks of the issuer's decision, made when the
//!   upload lands, so nothing is credited after a change made while the
//!   upload was in flight (spec 5.3). It is also the org's admission arm: an
//!   org tenant never falls through to the ungated legacy path.
//! - [`gate_account_request`]: a removed member seat keeps a withdraw-only
//!   account session for thirty days (spec 5.6, decision 6); every other
//!   account route refuses it.
//! - The `/v1/org/*` console routes (spec 8): an account session whose seat
//!   holds the `org_admin` role, resolved on every request. They read counts,
//!   states, the policy and the org's own audit rows. **No route here
//!   serves a trace, a preview, a redacted body, a review queue or an
//!   export.**
//! - `GET /v1/account/org-policy`: the policy an enrolled seat acts on.
//!
//! Refusals are fixed labels. Logs carry labels only.

use super::*;
use trace_commons_protocol::org_policy::{
    ACCOUNT_ORG_POLICY_PATH, ORG_AUDIT_PATH, ORG_POLICY_PATH, ORG_SEATS_PATH, ORG_SUMMARY_PATH,
    OrgAuditResponse, OrgPolicyResponse, OrgPolicyUpdateRequest, OrgPolicyUpdateResponse,
    OrgSeatKind, OrgSeatSummary, OrgSeatsResponse, OrgSummaryResponse, uses_org_namespace,
};
use trace_commons_server::db::{OrgPolicyUpdateOutcome, OrgSeatRemoveOutcome};
use trace_commons_server::org::{OrgRecord, OrgRefusal, OrgSeatRecord};

/// The routes this module adds to the authenticated account surface.
pub(super) fn routes() -> account_routes::AccountRoutes {
    account_routes::AccountRoutes::new()
        .get(ORG_SUMMARY_PATH, summary_handler)
        .get(ORG_POLICY_PATH, policy_handler)
        .put(ORG_POLICY_PATH, policy_update_handler)
        .get(ORG_SEATS_PATH, seats_handler)
        .delete("/v1/org/seats/{account_id}", seat_remove_handler)
        .get(ORG_AUDIT_PATH, audit_handler)
        .get(ACCOUNT_ORG_POLICY_PATH, account_policy_handler)
}

fn org_refused(refusal: OrgRefusal) -> (StatusCode, Json<ApiError>) {
    api_error(StatusCode::FORBIDDEN, refusal.label())
}

/// The org-tenant re-checks for an upload, before admission reserves
/// anything. A tenant outside the org namespace returns at once, with no
/// database read.
pub(super) async fn check_submission(
    state: &AppState,
    tenant: &TenantCtx,
    envelope: &TraceContributionEnvelope,
) -> ApiResult<()> {
    if !uses_org_namespace(tenant.tenant_id()) {
        return Ok(());
    }
    // An org tenant with no database cannot show its record: refused, never
    // waved through.
    let Some(db) = state.db_mirror.as_ref() else {
        return Err(org_refused(OrgRefusal::RecordMissing));
    };
    let org = db
        .get_org(tenant.tenant_id())
        .await
        .map_err(internal_error)?;
    let seat = match org.as_ref() {
        Some(_) => db
            .get_org_seat_for_principal(tenant.tenant_id(), &tenant.account_principal_ref())
            .await
            .map_err(internal_error)?,
        None => None,
    };
    let mut scopes = envelope.consent.scopes.clone();
    scopes.push(envelope.trace_card.consent_scope);
    trace_commons_server::org::check_org_landing(
        org.as_ref(),
        seat.as_ref(),
        &scopes,
        &envelope.trace_card.allowed_uses,
        Utc::now(),
    )
    .map_err(|refusal| {
        tracing::info!(
            refusal = refusal.label(),
            "Trace Commons upload refused for an org tenant"
        );
        org_refused(refusal)
    })
}

/// Whether `path` (with `method`) is one a removed member seat may still
/// reach: its traces, reading one, withdrawing one, and signing out.
fn withdraw_window_route(method: &Method, path: &str) -> bool {
    let segments: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    matches!(
        (method.as_str(), segments.as_slice()),
        ("GET", ["", "v1", "account", "traces"])
            | ("GET", ["", "v1", "account", "traces", _])
            | ("POST", ["", "v1", "account", "traces", _, "withdraw"])
            | (
                "POST",
                [
                    "",
                    "v1",
                    "contributors",
                    "me",
                    "pipeline-submissions",
                    _,
                    "withdraw",
                ],
            )
            | ("POST", ["", "v1", "account", "logout"])
    )
}

/// Refuse an account request from an org seat that may no longer make it.
/// Outside the org namespace this returns at once.
///
/// - An active member or server seat passes.
/// - A removed member seat passes only [`withdraw_window_route`]s, and only
///   inside its window.
/// - A session whose account has no seat (the holder account has no
///   principal and no session, so this is not a reachable state) is refused.
pub(super) async fn gate_account_request(
    state: &AppState,
    ctx: &AccountCtx,
    method: &Method,
    path: &str,
) -> ApiResult<()> {
    if !uses_org_namespace(&ctx.tenant_id) {
        return Ok(());
    }
    let db = account_db(state)?;
    let seat = db
        .get_org_seat(&ctx.tenant_id, ctx.account_id.as_uuid())
        .await
        .map_err(internal_error)?;
    let Some(seat) = seat else {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            OrgRefusal::SeatMissing.label(),
        ));
    };
    if seat.is_active() && seat.kind != OrgSeatKind::Holder {
        return Ok(());
    }
    if trace_commons_server::org::removed_seat_may_withdraw(&seat, Utc::now())
        && withdraw_window_route(method, path)
    {
        return Ok(());
    }
    Err(api_error(
        StatusCode::UNAUTHORIZED,
        OrgRefusal::SeatRemoved.label(),
    ))
}

/// The caller's org and seat, when the seat is an active `org_admin`. Every
/// other caller, including one in another tenant, is answered `404` as if no
/// org console existed, so the console is not an oracle for which tenants
/// are orgs.
async fn org_admin(
    state: &AppState,
    ctx: &AccountCtx,
) -> ApiResult<(Arc<dyn Database>, OrgRecord, OrgSeatRecord)> {
    let not_found = || api_error(StatusCode::NOT_FOUND, "org_not_found");
    if !uses_org_namespace(&ctx.tenant_id) {
        return Err(not_found());
    }
    let db = account_db(state)?;
    let seat = db
        .get_org_seat(&ctx.tenant_id, ctx.account_id.as_uuid())
        .await
        .map_err(internal_error)?
        .filter(OrgSeatRecord::is_org_admin)
        .ok_or_else(not_found)?;
    let org = db
        .get_org(&ctx.tenant_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    Ok((db, org, seat))
}

fn refuse_cross_origin_cookie(ctx: &AccountCtx, headers: &HeaderMap) -> ApiResult<()> {
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

fn no_store<T>(body: T) -> (HeaderMap, Json<T>) {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    (headers, Json(body))
}

async fn summary_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
) -> ApiResult<(HeaderMap, Json<OrgSummaryResponse>)> {
    let (db, org, _) = org_admin(state.as_ref(), &ctx).await?;
    let seats = db
        .list_org_seats(&ctx.tenant_id)
        .await
        .map_err(internal_error)?;
    let count = |kind: OrgSeatKind| {
        seats
            .iter()
            .filter(|listing| listing.seat.kind == kind && listing.seat.is_active())
            .count() as i64
    };
    Ok(no_store(OrgSummaryResponse {
        status: org.status,
        policy_version: org.policy_version,
        policy_digest: org.policy_digest,
        seat_limit: org.seat_limit,
        server_limit: org.server_limit,
        member_seats: count(OrgSeatKind::Member),
        server_seats: count(OrgSeatKind::Server),
        near_ai_org_bound: org.near_ai_org_bound,
    }))
}

async fn policy_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
) -> ApiResult<(HeaderMap, Json<OrgPolicyResponse>)> {
    let (_, org, _) = org_admin(state.as_ref(), &ctx).await?;
    Ok(no_store(OrgPolicyResponse {
        policy: org.policy,
        policy_version: org.policy_version,
        policy_digest: org.policy_digest,
    }))
}

async fn policy_update_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    Json(body): Json<OrgPolicyUpdateRequest>,
) -> ApiResult<(HeaderMap, Json<OrgPolicyUpdateResponse>)> {
    refuse_cross_origin_cookie(&ctx, &headers)?;
    let (db, _, _) = org_admin(state.as_ref(), &ctx).await?;
    body.policy
        .validate()
        .map_err(|invalid| api_error(StatusCode::BAD_REQUEST, invalid.label()))?;
    let outcome = db
        .update_org_policy(
            &ctx.tenant_id,
            body.expected_version,
            &body.policy,
            &ctx.actor_ref,
        )
        .await
        .map_err(internal_error)?;
    match outcome {
        OrgPolicyUpdateOutcome::Updated {
            policy_version,
            policy_digest,
            widened,
        } => Ok(no_store(OrgPolicyUpdateResponse {
            policy_version,
            policy_digest,
            widened,
        })),
        OrgPolicyUpdateOutcome::VersionConflict { .. } => Err(api_error(
            StatusCode::CONFLICT,
            "org_policy_version_conflict",
        )),
        OrgPolicyUpdateOutcome::OrgMissing => {
            Err(api_error(StatusCode::NOT_FOUND, "org_not_found"))
        }
    }
}

async fn seats_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
) -> ApiResult<(HeaderMap, Json<OrgSeatsResponse>)> {
    let (db, _, _) = org_admin(state.as_ref(), &ctx).await?;
    let listings = db
        .list_org_seats(&ctx.tenant_id)
        .await
        .map_err(internal_error)?;
    // A read about people is audited (spec 8).
    db.append_org_audit_row(
        &ctx.tenant_id,
        "org_report_read",
        &ctx.actor_ref,
        "success",
        serde_json::json!({ "report": "seats", "rows": listings.len() }),
    )
    .await
    .map_err(internal_error)?;
    let seats = listings
        .into_iter()
        .map(|listing| OrgSeatSummary {
            account_id: listing.seat.account_id,
            kind: listing.seat.kind,
            org_role: listing.seat.org_role,
            active: listing.seat.is_active(),
            joined_at: listing.seat.joined_at,
            removed_at: listing.seat.removed_at,
            active_devices: listing.active_devices,
        })
        .collect();
    Ok(no_store(OrgSeatsResponse { seats }))
}

async fn seat_remove_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    AxumPath(account_id): AxumPath<String>,
) -> ApiResult<(HeaderMap, Json<serde_json::Value>)> {
    refuse_cross_origin_cookie(&ctx, &headers)?;
    let (db, _, _) = org_admin(state.as_ref(), &ctx).await?;
    // A malformed id names no seat: the same 404 as an unknown one.
    let seat_not_found = || api_error(StatusCode::NOT_FOUND, "org_seat_not_found");
    let account_id = Uuid::parse_str(&account_id).map_err(|_| seat_not_found())?;
    let outcome = db
        .remove_org_seat(&ctx.tenant_id, account_id, &ctx.actor_ref)
        .await
        .map_err(internal_error)?;
    match outcome {
        OrgSeatRemoveOutcome::Removed {
            devices_revoked,
            grants_revoked,
            sessions_revoked,
        } => Ok(no_store(serde_json::json!({
            "removed": true,
            "devices_revoked": devices_revoked,
            "grants_revoked": grants_revoked,
            "sessions_revoked": sessions_revoked,
        }))),
        OrgSeatRemoveOutcome::NotFound | OrgSeatRemoveOutcome::HolderSeat => Err(seat_not_found()),
        OrgSeatRemoveOutcome::LastAdmin => Err(api_error(StatusCode::CONFLICT, "org_last_admin")),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditQuery {
    #[serde(default)]
    limit: Option<i64>,
}

async fn audit_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    Query(query): Query<AuditQuery>,
) -> ApiResult<(HeaderMap, Json<OrgAuditResponse>)> {
    let (db, _, _) = org_admin(state.as_ref(), &ctx).await?;
    let entries = db
        .list_org_audit(&ctx.tenant_id, query.limit.unwrap_or(100))
        .await
        .map_err(internal_error)?;
    Ok(no_store(OrgAuditResponse { entries }))
}

/// The policy an enrolled seat's device acts on, with its digest. Any
/// active seat of the org may read it; a session outside an org is `404`.
async fn account_policy_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
) -> ApiResult<(HeaderMap, Json<OrgPolicyResponse>)> {
    let not_found = || api_error(StatusCode::NOT_FOUND, "org_not_found");
    if !uses_org_namespace(&ctx.tenant_id) {
        return Err(not_found());
    }
    let db = account_db(state.as_ref())?;
    let org = db
        .get_org(&ctx.tenant_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    Ok(no_store(OrgPolicyResponse {
        policy: org.policy,
        policy_version: org.policy_version,
        policy_digest: org.policy_digest,
    }))
}

#[cfg(test)]
mod route_tests {
    use super::*;

    #[test]
    fn withdraw_window_allows_only_reading_and_withdrawing() {
        let id = "00000000-0000-0000-0000-000000000001";
        for (method, path) in [
            (Method::GET, "/v1/account/traces".to_string()),
            (Method::GET, format!("/v1/account/traces/{id}")),
            (Method::POST, format!("/v1/account/traces/{id}/withdraw")),
            (
                Method::POST,
                format!("/v1/contributors/me/pipeline-submissions/{id}/withdraw"),
            ),
            (Method::POST, "/v1/account/logout".to_string()),
        ] {
            assert!(withdraw_window_route(&method, &path), "{method} {path}");
        }
        for (method, path) in [
            (Method::POST, "/v1/account/traces".to_string()),
            (Method::GET, format!("/v1/account/traces/{id}/content")),
            (Method::GET, "/v1/account/credit-summary".to_string()),
            (Method::GET, "/v1/account/org-policy".to_string()),
            (Method::GET, "/v1/org".to_string()),
            (Method::DELETE, format!("/v1/account/devices/{id}")),
            (Method::PUT, format!("/v1/account/traces/{id}/publication")),
        ] {
            assert!(!withdraw_window_route(&method, &path), "{method} {path}");
        }
    }
}
