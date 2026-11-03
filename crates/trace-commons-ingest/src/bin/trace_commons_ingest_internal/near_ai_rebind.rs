// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! F10: change the NEAR AI login an account uses.
//!
//! Spec: `docs/superpowers/specs/2026-11-03-f10-change-account-design.md`
//! (owner decisions of 2026-11-03). Wire contract:
//! `trace_commons_protocol::account_login_change`.
//!
//! - `POST /v1/account/near-ai/rebind/start`, on the signed-in native session
//!   and from that session's own device, opens a single-use ceremony. For an
//!   account whose current identity is a NEAR wallet it also answers the
//!   NEP-413 challenge the wallet signs on the Commons wallet page (W1).
//! - `POST /v1/account/near-ai/rebind/finish` checks the device's signature
//!   over both proofs, the current identity (a NEAR AI token that anchors to
//!   the account's live login, or a FullAccess wallet signature whose name
//!   indexes to its live wallet anchor), and the new login, then moves the
//!   account in one transaction. Tenant and account never change.
//!
//! Every refusal is the uniform deny except three the person can act on,
//! each answered only after they proved what it is about:
//! `near_ai_login_in_use`, `login_change_too_soon` and
//! `wallet_account_required`. Neither token nor the wallet signature is
//! stored, logged or returned.

use super::*;
use trace_commons_protocol::account_login_change as wire;
use trace_commons_server::account_login_change::{
    NearAiRebindPending, RebindCurrent, RebindOutcome, RebindPrecheck, RebindPurpose,
    RebindSession, wallet_commitment,
};

pub(in crate::ingest::app) fn routes() -> crate::ingest::app::account_routes::AccountRoutes {
    crate::ingest::app::account_routes::AccountRoutes::new()
        .post(wire::REBIND_START_PATH, start_handler)
        .post(wire::REBIND_FINISH_PATH, finish_handler)
}

fn rebind_limited(headers: &HeaderMap, action: &str) -> bool {
    !ACCOUNT_RATE_LIMITER.check(
        &format!("{action}:{}", client_ip_for_rate_limit(headers)),
        30,
    ) || !ACCOUNT_RATE_LIMITER.check_global(&format!("{action}:global"), 600)
}

fn named(status: StatusCode, label: &'static str) -> axum::response::Response {
    no_store(api_error(status, label))
}

/// The wallet page every wallet ceremony signs on.
fn wallet_url(origin: &str) -> String {
    format!(
        "{}/account/near/provision/wallet",
        origin.trim_end_matches('/')
    )
}

/// A NEAR account name in the shape NEAR accepts, checked before it is put
/// into a message a wallet displays. The finish compares the name with the
/// account's anchor; this only keeps junk out of the challenge.
fn plausible_account_name(value: &str) -> bool {
    (2..=64).contains(&value.len())
        && value.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_')
        })
}

fn session_of(ctx: &AccountCtx, device: &[u8; 32]) -> Option<RebindSession> {
    Some(RebindSession {
        tenant_id: ctx.tenant_id.clone(),
        account_id: ctx.account_id.as_uuid(),
        session_token_hash: ctx.session_token_hash.clone()?,
        device_key_id: trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
            device,
        ),
    })
}

async fn start_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    body: Result<Json<wire::RebindStartRequest>, JsonRejection>,
) -> axum::response::Response {
    let began = std::time::Instant::now();
    let result = start(&state, &ctx, &headers, body).await;
    sleep_to_redeem_floor(began).await;
    result.unwrap_or_else(native_generic_deny)
}

async fn start(
    state: &AppState,
    ctx: &AccountCtx,
    headers: &HeaderMap,
    body: Result<Json<wire::RebindStartRequest>, JsonRejection>,
) -> Option<axum::response::Response> {
    if rebind_limited(headers, "near-ai-rebind-start") {
        return Some(near_ai_sign_in_rate_limited());
    }
    if let Err(control) = ready_or_declined(state, ProvisionContract::ExplicitSelection, false) {
        return not_ready_answer(control, CeremonyHalf::Start);
    }
    let Json(body) = body.ok()?;
    if body.code_challenge_method != "S256" || !challenge_is_wellformed(&body.code_challenge) {
        return None;
    }
    let device = device_key(&body.device_public_key)?;
    let session = session_of(ctx, &device)?;
    let Ok(db) = account_db(state) else {
        return Some(near_ai_sign_in_unavailable("database"));
    };
    let current_kind = match db.near_ai_rebind_precheck(&session).await {
        Ok(RebindPrecheck::Ready { current_kind }) => current_kind,
        Ok(RebindPrecheck::TooSoon) => {
            return Some(named(StatusCode::CONFLICT, wire::REBIND_TOO_SOON));
        }
        Ok(RebindPrecheck::Refused(label)) => {
            tracing::info!(reason = label, "NEAR AI login change refused at start");
            return None;
        }
        Err(error) => return near_ai_database_failure(&error),
    };
    use rand::RngCore as _;
    let mut nonce = [0u8; 32];
    rand::rngs::OsRng.try_fill_bytes(&mut nonce).ok()?;
    let ceremony_id = generate_login_code();
    let expires_at = Utc::now()
        .timestamp()
        .checked_add(wire::REBIND_CEREMONY_TTL_SECS)?;
    let (wallet, commitment) = match current_kind {
        wire::IdentityKind::NearAiLogin => (None, None),
        wire::IdentityKind::Wallet => {
            if let Err(control) =
                ready_or_declined(state, ProvisionContract::ExplicitSelection, true)
            {
                return not_ready_answer(control, CeremonyHalf::Start);
            }
            let Some(account) = body.wallet_account_id.as_deref() else {
                return Some(named(
                    StatusCode::CONFLICT,
                    wire::REBIND_WALLET_ACCOUNT_REQUIRED,
                ));
            };
            if !plausible_account_name(account) {
                return None;
            }
            let (Ok(cfg), Some(origin)) = (
                account_near_config(state),
                state.near_provisioning_public_origin.as_ref(),
            ) else {
                return Some(near_ai_sign_in_unavailable("not_ready"));
            };
            (
                Some(wire::RebindWalletChallenge {
                    message: wire::rebind_wallet_message(
                        &cfg.network,
                        account,
                        &device,
                        expires_at,
                    ),
                    recipient: cfg.recipient.clone(),
                    wallet_url: wallet_url(origin),
                    network: cfg.network.clone(),
                }),
                Some(wallet_commitment(&cfg.network, account)),
            )
        }
    };
    let pending = NearAiRebindPending {
        purpose: RebindPurpose::Rebind,
        tenant_id: session.tenant_id.clone(),
        account_id: session.account_id.to_string(),
        session_token_hash: session.session_token_hash.clone(),
        device_public_key: base64::engine::general_purpose::STANDARD.encode(device),
        nonce_hex: hex::encode(nonce),
        code_challenge: body.code_challenge,
        expires_at,
        current_kind,
        wallet_commitment: commitment,
    };
    if let Err(error) = db
        .store_near_ai_rebind_ceremony(&hash_secret(&ceremony_id), &pending, expires_at)
        .await
    {
        return near_ai_database_failure(&error);
    }
    let answer = wire::RebindStartResponse {
        ceremony_id,
        nonce: base64::engine::general_purpose::STANDARD.encode(nonce),
        expires_at,
        current_kind,
        wallet,
    };
    Some(response(serde_json::to_value(answer).ok()?))
}

async fn finish_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    body: Result<Json<wire::RebindFinishRequest>, JsonRejection>,
) -> axum::response::Response {
    let began = std::time::Instant::now();
    let result = finish(&state, &ctx, &headers, body).await;
    sleep_to_redeem_floor(began).await;
    result.unwrap_or_else(native_generic_deny)
}

/// The bounds a finish body must meet before anything is done with it.
fn finish_precheck(body: &wire::RebindFinishRequest) -> Option<[u8; 32]> {
    let token_ok = |token: &str| !token.is_empty() && token.len() <= 8192;
    let proof_ok = match &body.current_proof {
        wire::CurrentProof::NearAiLogin { access_token } => token_ok(access_token),
        wire::CurrentProof::Wallet {
            account_id,
            wallet_public_key,
            wallet_signature,
        } => {
            plausible_account_name(account_id)
                && wallet_public_key.len() <= 120
                && wallet_signature.len() <= 128
        }
    };
    if body.ceremony_id.len() > 64
        || !verifier_is_wellformed(&body.code_verifier)
        || body.device_signature.len() > 128
        || !token_ok(&body.new_access_token)
        || !proof_ok
    {
        return None;
    }
    device_key(&body.device_public_key)
}

async fn finish(
    state: &AppState,
    ctx: &AccountCtx,
    headers: &HeaderMap,
    body: Result<Json<wire::RebindFinishRequest>, JsonRejection>,
) -> Option<axum::response::Response> {
    if rebind_limited(headers, "near-ai-rebind-finish") {
        return Some(near_ai_sign_in_rate_limited());
    }
    if let Err(control) = ready_or_declined(state, ProvisionContract::ExplicitSelection, false) {
        return not_ready_answer(control, CeremonyHalf::Finish);
    }
    let Json(body) = body.ok()?;
    let device = finish_precheck(&body)?;
    let hash = hash_secret(&body.ceremony_id);
    if !ACCOUNT_RATE_LIMITER.check(&format!("near-ai-rebind-ceremony:{hash}"), 5) {
        return None;
    }
    let session = session_of(ctx, &device)?;
    let Ok(db) = account_db(state) else {
        return Some(near_ai_sign_in_unavailable("database"));
    };
    let Some(identity) = state.near_account_identity.clone() else {
        return Some(near_ai_sign_in_unavailable("near_ai_identity"));
    };
    let pending = match db.take_near_ai_rebind_ceremony(&hash).await {
        Ok(pending) => pending?,
        Err(error) => return near_ai_database_failure(&error),
    };
    // Bound to the session that started it, its device and its PKCE pair.
    if pending.purpose != RebindPurpose::Rebind
        || pending.tenant_id != session.tenant_id
        || pending.account_id != session.account_id.to_string()
        || !secret_eq(&pending.session_token_hash, &session.session_token_hash)
        || !secret_eq(
            &pending.device_public_key,
            &base64::engine::general_purpose::STANDARD.encode(device),
        )
        || !secret_eq(
            &pending.code_challenge,
            &challenge_for_verifier(&body.code_verifier),
        )
        || Utc::now().timestamp() >= pending.expires_at
        || body.current_proof.kind() != pending.current_kind
    {
        return None;
    }
    let nonce: [u8; 32] = hex::decode(&pending.nonce_hex).ok()?.try_into().ok()?;
    let signing = wire::rebind_device_bytes(
        &nonce,
        &body.ceremony_id,
        &device,
        &pending.code_challenge,
        pending.expires_at,
        &body.current_proof.digest(),
        &wire::new_login_digest(&body.new_access_token),
    );
    let signature: [u8; 64] = base64::engine::general_purpose::STANDARD
        .decode(&body.device_signature)
        .ok()?
        .try_into()
        .ok()?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, device)
        .verify(&signing, &signature)
        .ok()?;
    // The wallet signature is checked offline before anything leaves this
    // machine, as the sign-up finish orders it.
    if let wire::CurrentProof::Wallet {
        account_id,
        wallet_public_key,
        wallet_signature,
    } = &body.current_proof
    {
        let (Ok(cfg), Some(origin)) = (
            account_near_config(state),
            state.near_provisioning_public_origin.as_ref(),
        ) else {
            return Some(near_ai_sign_in_unavailable("not_ready"));
        };
        let committed = pending.wallet_commitment.as_deref()?;
        if !secret_eq(committed, &wallet_commitment(&cfg.network, account_id)) {
            return None;
        }
        trace_commons_server::account_near::verify_nep413(
            wallet_public_key,
            &wire::rebind_wallet_message(&cfg.network, account_id, &device, pending.expires_at),
            &nonce,
            &cfg.recipient,
            Some(&wallet_url(origin)),
            wallet_signature,
        )
        .ok()?;
    }
    let new_login = match introspect(state, body.new_access_token).await {
        Ok(login) => login,
        Err(failure) => return introspection_answer(failure),
    };
    let current = match body.current_proof {
        wire::CurrentProof::NearAiLogin { access_token } => {
            let login = match introspect(state, access_token).await {
                Ok(login) => login,
                Err(failure) => return introspection_answer(failure),
            };
            RebindCurrent::NearAiLogin {
                anchor_hash: identity.login_index_label(login.subject_id()),
            }
        }
        wire::CurrentProof::Wallet {
            account_id,
            wallet_public_key,
            ..
        } => {
            let Ok(cfg) = account_near_config(state) else {
                return Some(near_ai_sign_in_unavailable("not_ready"));
            };
            // Last, because it is the only check that leaves for the NEAR RPC.
            match trace_commons_server::account_near::near_account_has_full_access_key(
                &cfg,
                &account_id,
                &wallet_public_key,
            )
            .await
            {
                Ok(true) => {}
                Ok(false) => return None,
                Err(error) => {
                    return match trace_commons_server::account_onboarding::ownership_check_failure(
                        &error,
                    ) {
                        trace_commons_server::account_onboarding::ProvisioningFailure::Unavailable => {
                            Some(near_ai_sign_in_unavailable("near_rpc"))
                        }
                        trace_commons_server::account_onboarding::ProvisioningFailure::Refused => None,
                    };
                }
            }
            RebindCurrent::Wallet {
                anchor_hash: identity.index_label(&cfg.network, &account_id),
                near_account_id: account_id,
            }
        }
    };
    let outcome = match db
        .near_ai_rebind(&session, &current, &new_login, &identity)
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => return near_ai_database_failure(&error),
    };
    match outcome {
        RebindOutcome::Rebound {
            anchor_hash,
            auth_provider,
            switched_back,
        } => Some(response(
            serde_json::to_value(wire::RebindFinishResponse {
                tenant_id: session.tenant_id,
                account_id: session.account_id.to_string(),
                anchor_hash,
                auth_provider,
                identity: wire::IdentityKind::NearAiLogin,
                switched_back,
            })
            .ok()?,
        )),
        RebindOutcome::InUse => Some(named(StatusCode::CONFLICT, wire::REBIND_IN_USE)),
        RebindOutcome::TooSoon => Some(named(StatusCode::CONFLICT, wire::REBIND_TOO_SOON)),
        RebindOutcome::PayoutReviewRequired => {
            tracing::warn!(
                reason = wire::REBIND_PAYOUT_REVIEW_REQUIRED,
                "NEAR AI login change held: a wallet-origin account has a waiting payout link"
            );
            Some(named(
                StatusCode::CONFLICT,
                wire::REBIND_PAYOUT_REVIEW_REQUIRED,
            ))
        }
        RebindOutcome::Refused(label) => {
            tracing::info!(reason = label, "NEAR AI login change refused");
            None
        }
    }
}

/// Introspect one NEAR AI token.
async fn introspect(
    state: &AppState,
    token: String,
) -> Result<
    trace_commons_server::near_ai_login::VerifiedNearAiLogin,
    trace_commons_server::near_ai_login::LoginIntrospectionFailure,
> {
    near_ai_introspect(state, token).await
}

/// A finish's answer to a failed introspection: NEAR AI not answering is
/// `503`, a token it named no one for is the uniform deny.
fn introspection_answer(
    failure: trace_commons_server::near_ai_login::LoginIntrospectionFailure,
) -> Option<axum::response::Response> {
    use trace_commons_server::near_ai_login::LoginIntrospectionFailure;
    match failure {
        LoginIntrospectionFailure::Unavailable => {
            Some(near_ai_sign_in_unavailable("near_ai_introspection"))
        }
        LoginIntrospectionFailure::ClientUnbuildable => {
            tracing::error!(
                cause = "near_ai_client",
                "NEAR AI login change answered unavailable: no HTTP client could be built"
            );
            Some(unavailable_response())
        }
        LoginIntrospectionFailure::Refused => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_plausible_name_reaches_a_wallet_message() {
        assert!(plausible_account_name("alice.near"));
        assert!(plausible_account_name("a-b_c.testnet"));
        assert!(!plausible_account_name("A.near"));
        assert!(!plausible_account_name("x"));
        assert!(!plausible_account_name("evil\nPurpose: other"));
        assert!(!plausible_account_name(&"a".repeat(65)));
    }

    #[test]
    fn a_finish_body_out_of_bounds_is_refused_before_anything_runs() {
        let body = |token: &str| wire::RebindFinishRequest {
            ceremony_id: "c".into(),
            code_verifier: "v".repeat(64),
            device_public_key: base64::engine::general_purpose::STANDARD.encode([1u8; 32]),
            device_signature: "s".into(),
            current_proof: wire::CurrentProof::NearAiLogin {
                access_token: "t".into(),
            },
            new_access_token: token.into(),
        };
        assert!(finish_precheck(&body("t")).is_some());
        assert!(finish_precheck(&body("")).is_none());
        assert!(finish_precheck(&body(&"t".repeat(8193))).is_none());
    }
}
