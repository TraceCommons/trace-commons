// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Linking an account to a payout recipient, rail by rail
//! (docs/superpowers/specs/2026-10-21-nearai-credit-payout-design.md,
//! section 5; wire types in `trace_commons_protocol::payout_link`).
//!
//! - `POST /v1/account/payout-rails/{rail_id}/link-requests` starts a link
//!   or an unlink. Local refusals come first (session kind, rail, body,
//!   account standing, the "changed since shown" version); only then does a
//!   `link` spend the person's NEAR AI token on the rail's recipient-proof
//!   verifier (`/users/me`, then a zero-target probe). The verified org is
//!   sealed and stored on a request that changes nothing yet, with a 5
//!   minute expiry and a handle.
//! - `POST /v1/account/payout-rails/{rail_id}/link-requests/{handle}/confirm`
//!   takes the request from the same account and, in one transaction, writes
//!   the `waiting` link (or the unlink mark) and its audit rows, refusing if
//!   the link changed since the request, a newer request superseded it, or
//!   it expired. A confirmed change wakes the rail.
//! - `GET /v1/account/payout-rails` lists each rail that is not `disabled`:
//!   link state, version and recipient ref, never a recipient or a name.
//!
//! **Authority (owner decision 2026-10-23, superseding D5).** "We aren't
//! using passkeys at all in the app right now." The contributor's signed-in
//! account session is enough to link, change and unlink: a native token or a
//! same-origin cookie session, weak or strong. There is no passkey gate, no
//! step-up page section, and nothing requires the account to have a passkey.
//! A device bearer is not an account session and is refused. The owner
//! accepted the trade-off that anyone holding the app session could redirect
//! future payouts; what still stands behind a link is the server's own
//! ownership check with NEAR AI, the version check, and the audit trail.
//!
//! Hash-only: responses and logs carry the rail id, labels, versions and
//! `sha256:` refs, never an org id or name, a token, an account id or a
//! subject.

use super::*;
use trace_commons_protocol::payout_link::{
    PAYOUT_LINK_CONTRACT_VERSION, PayoutLinkAction as WireAction, PayoutLinkConfirmed,
    PayoutLinkOutcome, PayoutLinkProof, PayoutLinkRequest as WireRequest, PayoutLinkStarted,
    PayoutRailLinkStatus, PayoutRailsStatus, PayoutStreamKind, PayoutStreamStatus,
    near_ai_recipient_ref,
};
use trace_commons_server::db::{
    PayoutLinkAction, PayoutLinkConfirm, PayoutLinkConfirmWrite, PayoutLinkRequestInsert,
    PayoutLinkRequestWrite, PayoutLinkState, PayoutStore, PayoutStream,
};
use trace_commons_server::payout_pass::{stream_status_hold_label, stream_waiting};
use trace_commons_server::payout_rate::PayoutMode;
use trace_commons_server::payout_recipient_proof::{
    NEAR_AI_USERS_ME_TIMEOUT, NearAiOrgVerifier, RecipientProof, RecipientProofRefusal,
    RecipientProofVerifier,
};
use trace_commons_server::payout_recipient_seal::{payout_link_handle, seal_payout_recipient};
use trace_commons_server::payout_worker::PayoutRailEntry;

/// How long a started request waits for its confirm.
const PAYOUT_LINK_REQUEST_TTL: chrono::Duration = chrono::Duration::minutes(5);

pub(super) fn routes() -> crate::ingest::app::account_routes::AccountRoutes {
    crate::ingest::app::account_routes::AccountRoutes::new()
        .get("/v1/account/payout-rails", status_handler)
        .post(
            "/v1/account/payout-rails/{rail_id}/link-requests",
            start_handler,
        )
        .post(
            "/v1/account/payout-rails/{rail_id}/link-requests/{handle}/confirm",
            confirm_handler,
        )
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

/// The base `/users/me` is read at. In a shipped build the compiled-in
/// NEAR AI host and nothing else, the same host the rail pays; the
/// `cfg(test)` arm reads the same seam login introspection uses.
#[cfg(not(test))]
fn users_me_base_url(_state: &AppState) -> &str {
    trace_commons_server::near_ai_api::NEAR_AI_API_BASE_URL
}

#[cfg(test)]
fn users_me_base_url(state: &AppState) -> &str {
    state
        .near_ai_introspection_base_url
        .as_deref()
        .unwrap_or(trace_commons_server::near_ai_api::NEAR_AI_API_BASE_URL)
}

fn refuse(status: StatusCode, outcome: PayoutLinkOutcome) -> (StatusCode, Json<ApiError>) {
    api_error(status, outcome.as_str())
}

/// Any signed-in account session may change a payout link; a device bearer
/// may not, and a cookie session only from the same origin.
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

/// The account's hash on audit rows. Never the account id.
fn account_actor_hash(ctx: &AccountCtx) -> String {
    sha256_prefixed(&format!(
        "payout-link-actor-v1:{}",
        ctx.account_id.as_uuid()
    ))
}

/// The rail named in the path, if it is enabled; an unknown id is refused
/// by name and a disabled rail answers `payout_unavailable`.
fn enabled_rail<'a>(state: &'a AppState, rail_id: &str) -> ApiResult<&'a Arc<PayoutRailEntry>> {
    let entry = state
        .payout_rails
        .get(rail_id)
        .map_err(|_| refuse(StatusCode::NOT_FOUND, PayoutLinkOutcome::PayoutRailUnknown))?;
    if entry.mode() == PayoutMode::Disabled || entry.rail().is_none() {
        return Err(refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            PayoutLinkOutcome::PayoutUnavailable,
        ));
    }
    Ok(entry)
}

fn payout_store(state: &AppState) -> ApiResult<&Arc<dyn PayoutStore>> {
    state.payout_store.as_ref().ok_or_else(|| {
        refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            PayoutLinkOutcome::PayoutUnavailable,
        )
    })
}

fn store_failed(_: trace_commons_server::error::DatabaseError) -> (StatusCode, Json<ApiError>) {
    refuse(
        StatusCode::SERVICE_UNAVAILABLE,
        PayoutLinkOutcome::PayoutUnavailable,
    )
}

fn proof_refusal(refusal: RecipientProofRefusal) -> (StatusCode, Json<ApiError>) {
    let status = match refusal {
        RecipientProofRefusal::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::FORBIDDEN,
    };
    api_error(status, refusal.label())
}

/// What a replay must match: the action, the org and the version, never
/// the token.
fn request_digest(rail: &str, body: &WireRequest) -> String {
    let org = match &body.proof {
        Some(PayoutLinkProof::NearAi {
            organization_id, ..
        }) => organization_id.to_string(),
        None => String::new(),
    };
    sha256_prefixed(&format!(
        "payout-link-request-v1:{rail}:{}:{org}:{}",
        match body.action {
            WireAction::Link => "link",
            WireAction::Unlink => "unlink",
        },
        body.expected_version().unwrap_or(0)
    ))
}

pub(super) async fn start_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    AxumPath(rail_id): AxumPath<String>,
    body: Result<Json<WireRequest>, JsonRejection>,
) -> ApiResult<(HeaderMap, Json<PayoutLinkStarted>)> {
    require_account_session(&ctx, &headers)?;
    let entry = enabled_rail(&state, &rail_id)?;
    let rail = entry.rail().cloned().ok_or_else(|| {
        refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            PayoutLinkOutcome::PayoutUnavailable,
        )
    })?;
    let store = payout_store(&state)?;
    // The recipient key: without it nothing could be sealed, and nothing
    // sealed could be paid.
    let identity = state.near_account_identity.clone().ok_or_else(|| {
        refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            PayoutLinkOutcome::PayoutUnavailable,
        )
    })?;
    let Ok(Json(body)) = body else {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            PayoutLinkOutcome::InvalidPayoutLinkRequest,
        ));
    };
    if body.validate(entry.rail_id().as_str()).is_err() {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            PayoutLinkOutcome::InvalidPayoutLinkRequest,
        ));
    }
    let tenant = ctx.tenant_id.as_str();
    let account = ctx.account_id.as_uuid();
    let rail_id = entry.rail_id().clone();

    // Local refusals before anything leaves this machine.
    let standing = store
        .payout_account_standing(tenant, account)
        .await
        .map_err(store_failed)?;
    if !standing.exists || standing.closed {
        return Err(refuse(
            StatusCode::FORBIDDEN,
            PayoutLinkOutcome::PayoutAccountIneligible,
        ));
    }
    store
        .purge_expired_payout_link_requests(tenant)
        .await
        .map_err(store_failed)?;
    let view = store
        .payout_account_link_view(tenant, account, &rail_id)
        .await
        .map_err(store_failed)?;
    if body.expected_version().unwrap_or(0) != view.version {
        return Err(refuse(
            StatusCode::CONFLICT,
            PayoutLinkOutcome::VersionConflict,
        ));
    }
    if body.action == WireAction::Unlink
        && !view
            .newest
            .as_ref()
            .is_some_and(|link| link.state != PayoutLinkState::Closing)
    {
        return Err(refuse(
            StatusCode::CONFLICT,
            PayoutLinkOutcome::PayoutNotLinked,
        ));
    }

    let digest = request_digest(rail_id.as_str(), &body);
    let handle = payout_link_handle(
        &identity,
        tenant,
        account,
        rail_id.as_str(),
        body.idempotency_key,
    );
    let expected_version = body.expected_version();
    let idempotency_key = body.idempotency_key;
    let wire_action = body.action;
    let (action, recipient, recipient_ref) = match body.proof {
        None => (PayoutLinkAction::Unlink, None, None),
        Some(proof) => {
            let organization_id = match &proof {
                PayoutLinkProof::NearAi {
                    organization_id, ..
                } => *organization_id,
            };
            let anchors = store
                .payout_account_login_anchors(tenant, account)
                .await
                .map_err(store_failed)?;
            // The token becomes a secret here and is dropped inside verify.
            let proof = RecipientProof::from(proof);
            let verifier = NearAiOrgVerifier::new(
                users_me_base_url(&state),
                rail.clone(),
                identity.clone(),
                NEAR_AI_USERS_ME_TIMEOUT,
            );
            let verified = match verifier.verify(proof, &anchors).await {
                Ok(verified) => verified,
                Err(refusal) => {
                    tracing::info!(
                        rail = %rail_id,
                        outcome = refusal.label(),
                        "payout link refused"
                    );
                    return Err(proof_refusal(refusal));
                }
            };
            let recipient_ref = near_ai_recipient_ref(idempotency_key, organization_id);
            let sealed = seal_payout_recipient(
                &identity,
                rail_id.as_str(),
                verified.recipient.expose(),
                &verified.subject_id,
                Some(recipient_ref.clone()),
            )
            .map_err(|_| {
                refuse(
                    StatusCode::SERVICE_UNAVAILABLE,
                    PayoutLinkOutcome::PayoutUnavailable,
                )
            })?;
            (
                PayoutLinkAction::Link,
                Some((rail.descriptor().recipient_kind.clone(), sealed)),
                Some(recipient_ref),
            )
        }
    };
    let expires_at = Utc::now() + PAYOUT_LINK_REQUEST_TTL;
    let inserted = store
        .insert_payout_link_request(
            tenant,
            &PayoutLinkRequestWrite {
                account_id: account,
                rail_id: rail_id.clone(),
                idempotency_key,
                handle_hash: sha256_prefixed(&handle),
                request_digest: digest,
                action,
                expected_current_version: expected_version,
                recipient,
                expires_at,
                actor_ref_hash: account_actor_hash(&ctx),
            },
        )
        .await
        .map_err(store_failed)?;
    if inserted == PayoutLinkRequestInsert::IdempotencyConflict {
        return Err(refuse(
            StatusCode::CONFLICT,
            PayoutLinkOutcome::IdempotencyConflict,
        ));
    }
    tracing::info!(
        rail = %rail_id,
        action = action.as_str(),
        replayed = inserted == PayoutLinkRequestInsert::Replayed,
        "payout link requested"
    );
    Ok((
        no_store(),
        Json(PayoutLinkStarted {
            contract_version: PAYOUT_LINK_CONTRACT_VERSION.to_owned(),
            rail_id: rail_id.as_str().to_owned(),
            action: wire_action,
            outcome: PayoutLinkOutcome::PendingConfirmation,
            handle,
            expires_at,
            recipient_ref,
        }),
    ))
}

fn is_handle(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) async fn confirm_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    AxumPath((rail_id, handle)): AxumPath<(String, String)>,
) -> ApiResult<(HeaderMap, Json<PayoutLinkConfirmed>)> {
    require_account_session(&ctx, &headers)?;
    let entry = enabled_rail(&state, &rail_id)?;
    let rail = entry.rail().cloned().ok_or_else(|| {
        refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            PayoutLinkOutcome::PayoutUnavailable,
        )
    })?;
    let store = payout_store(&state)?;
    if !is_handle(&handle) {
        return Err(refuse(StatusCode::NOT_FOUND, PayoutLinkOutcome::NotFound));
    }
    let link_id = Uuid::new_v4();
    let outcome = store
        .confirm_payout_link_request(
            &ctx.tenant_id,
            ctx.account_id.as_uuid(),
            entry.rail_id(),
            &sha256_prefixed(&handle),
            &PayoutLinkConfirmWrite {
                link_id,
                program: rail.program_for_link(link_id),
                actor_ref_hash: account_actor_hash(&ctx),
            },
        )
        .await
        .map_err(store_failed)?;
    let refusal = match outcome {
        PayoutLinkConfirm::Confirmed { action, version } => {
            tracing::info!(
                rail = %entry.rail_id(),
                action = action.as_str(),
                version,
                "payout link confirmed"
            );
            // A new link or an unlink is the pass's to act on: wake it.
            state.payout_rails.notifier().notify_all();
            return Ok((
                no_store(),
                Json(PayoutLinkConfirmed {
                    contract_version: PAYOUT_LINK_CONTRACT_VERSION.to_owned(),
                    rail_id: entry.rail_id().as_str().to_owned(),
                    action: match action {
                        PayoutLinkAction::Link => WireAction::Link,
                        PayoutLinkAction::Unlink => WireAction::Unlink,
                    },
                    outcome: PayoutLinkOutcome::Confirmed,
                    version,
                }),
            ));
        }
        PayoutLinkConfirm::NotFound => (StatusCode::NOT_FOUND, PayoutLinkOutcome::NotFound),
        PayoutLinkConfirm::Expired => (StatusCode::GONE, PayoutLinkOutcome::Expired),
        PayoutLinkConfirm::VersionConflict => {
            (StatusCode::CONFLICT, PayoutLinkOutcome::VersionConflict)
        }
        PayoutLinkConfirm::ChangePending => (
            StatusCode::CONFLICT,
            PayoutLinkOutcome::PayoutLinkChangePending,
        ),
        PayoutLinkConfirm::NotLinked => (StatusCode::CONFLICT, PayoutLinkOutcome::PayoutNotLinked),
    };
    tracing::info!(
        rail = %entry.rail_id(),
        outcome = refusal.1.as_str(),
        "payout link confirm refused"
    );
    Err(refuse(refusal.0, refusal.1))
}

pub(super) async fn status_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
) -> ApiResult<(HeaderMap, Json<PayoutRailsStatus>)> {
    let mut rails = Vec::new();
    for entry in state.payout_rails.entries() {
        if entry.mode() == PayoutMode::Disabled || entry.rail().is_none() {
            continue;
        }
        let store = payout_store(&state)?;
        let view = store
            .payout_account_link_view(&ctx.tenant_id, ctx.account_id.as_uuid(), entry.rail_id())
            .await
            .map_err(store_failed)?;
        let newest = view.newest.as_ref();
        rails.push(PayoutRailLinkStatus {
            rail_id: entry.rail_id().as_str().to_owned(),
            mode: entry.mode().as_label().to_owned(),
            linked: newest.is_some(),
            recipient_ref: newest.and_then(|link| link.recipient.recipient_ref.clone()),
            state: newest.map(|link| link.state.as_str().to_owned()),
            version: view.version,
            request_state: view
                .pending_request
                .then(|| PayoutLinkOutcome::PendingConfirmation.as_str().to_owned()),
            unlink_requested: newest.is_some_and(|link| link.unlink_requested_at.is_some()),
            recipient_unavailable: newest
                .is_some_and(|link| link.recipient_unavailable_at.is_some()),
            // Own and referral credit apart (referral rewards spec, R7),
            // only once the rail runs a referral stream: until then the
            // answer keeps the shape apps already read.
            streams: if entry.referral_mode() == PayoutMode::Disabled {
                Vec::new()
            } else {
                stream_statuses(store.as_ref(), &ctx, entry.rail_id()).await?
            },
        });
    }
    Ok((
        no_store(),
        Json(PayoutRailsStatus {
            contract_version: PAYOUT_LINK_CONTRACT_VERSION.to_owned(),
            rails,
        }),
    ))
}

/// The account's earned and referral streams on `rail`, earned first.
/// Amounts only; a hold shows as its label.
async fn stream_statuses(
    store: &dyn PayoutStore,
    ctx: &AccountCtx,
    rail: &trace_commons_gate_api::RailId,
) -> ApiResult<Vec<PayoutStreamStatus>> {
    let tenant = ctx.tenant_id.as_str();
    let account = ctx.account_id.as_uuid();
    let standing = store
        .payout_account_standing(tenant, account)
        .await
        .map_err(store_failed)?
        .hold_label();
    let open = store
        .open_payout_hold_labels(tenant, account, rail)
        .await
        .map_err(store_failed)?;
    let mut streams = Vec::with_capacity(PayoutStream::ALL.len());
    for stream in PayoutStream::ALL {
        // An owed sum the pass refuses (reversals above credits) shows as
        // nothing owed; the pass holds that account rather than send.
        let owed = match stream {
            PayoutStream::Earned => store
                .payout_owed(tenant, account, rail)
                .await
                .map_err(store_failed)?
                .net(),
            PayoutStream::Referral => {
                store
                    .payout_referral_owed(tenant, account, rail)
                    .await
                    .map_err(store_failed)?
                    .net
            }
        }
        .unwrap_or(0);
        let sent = store
            .payout_account_stream_sent(tenant, account, rail, stream)
            .await
            .map_err(store_failed)?;
        streams.push(PayoutStreamStatus {
            stream: match stream {
                PayoutStream::Earned => PayoutStreamKind::Earned,
                PayoutStream::Referral => PayoutStreamKind::Referral,
            },
            owed: owed.to_string(),
            sent: sent.to_string(),
            waiting: stream_waiting(owed, sent).to_string(),
            hold_label: stream_status_hold_label(stream, standing, &open),
        });
    }
    Ok(streams)
}

fn no_store() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    headers
}
