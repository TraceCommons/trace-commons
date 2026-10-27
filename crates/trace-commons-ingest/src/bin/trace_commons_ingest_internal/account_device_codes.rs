// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Linking a headless device with a one-time code, and that device's
//! key-signed session renewal (owner decisions 2026-10-27; wire contract in
//! `trace_commons_protocol::account_device_code`).
//!
//! - `POST /v1/account/device-codes` (account session, native only) mints a
//!   code. The body carries a proof signed by the minting device's key, so a
//!   stolen session token alone cannot mint; the key must belong to an
//!   active device of the session's account, and the proof's nonce is never
//!   accepted twice.
//! - `GET /v1/account/device-codes/{code_id}` reports whether the code was
//!   used, and by which device key.
//! - `POST /v1/device-codes/redeem` (unauthenticated) spends a code and
//!   links the redeeming device's key to the code's account.
//! - `POST /v1/device-sessions/renew` (unauthenticated) mints a fresh
//!   session for a device that joined by code, from a nonce signed with its
//!   key. A device that joined any other way is refused.
//!
//! The two unauthenticated routes answer every refusal they can with one
//! generic body after the same minimum latency, as the provisioning finish
//! does. Audit rows carry no code, nonce, key or label.

use super::*;
use trace_commons_protocol::account_device_code::{
    DEVICE_CODE_LABEL_MAX_CHARS, DEVICE_CODE_MINT_SKEW_SECS, DEVICE_CODE_PREFIX,
    DEVICE_CODE_TTL_SECS, DEVICE_SESSION_RENEW_SKEW_SECS, DeviceCodeMintRequest,
    DeviceCodeMintResponse, DeviceCodeRedeemRequest, DeviceCodeState, DeviceCodeStatusResponse,
    DeviceSessionRenewRequest, DeviceSignInResponse, device_code_mint_bytes,
    device_code_redeem_bytes, device_session_renew_bytes,
};
use trace_commons_server::db::{DeviceSignInOutcome, MintAccountDeviceCodeOutcome};

/// Unexpired, unredeemed codes an account may hold at once.
const DEVICE_CODE_OUTSTANDING_CAP: i64 = 3;
/// Mints per account per rate window.
const DEVICE_CODE_MINT_PER_ACCOUNT_LIMIT: u32 = 10;
/// Redeems and renewals per source address per rate window.
const DEVICE_SIGN_IN_PER_IP_LIMIT: u32 = 30;
/// Redeems and renewals across the deployment per rate window.
const DEVICE_SIGN_IN_GLOBAL_LIMIT: u32 = 600;
/// A device that joined by code renews at most this often. Its session
/// lives `NATIVE_SESSION_TTL_HOURS`, so this only stops a loop.
const DEVICE_SESSION_RENEW_MIN_INTERVAL_MINUTES: i64 = 10;

/// Refusal labels.
const MINT_NATIVE_SESSION_REQUIRED: &str = "native_session_required";
const MINT_INVALID: &str = "device_code_mint_invalid";
const MINT_PROOF_STALE: &str = "device_code_proof_stale";
const MINT_PROOF_REUSED: &str = "device_code_proof_reused";
const MINT_NOT_A_DEVICE: &str = "device_code_minter_not_a_device";
const MINT_TOO_MANY: &str = "device_code_too_many_outstanding";
const DEVICE_KEY_REGISTERED_ELSEWHERE: &str = "device_key_registered_elsewhere";
const DEVICE_RENEW_TOO_SOON: &str = "device_session_renew_too_soon";

pub(super) fn account_routes() -> crate::ingest::app::account_routes::AccountRoutes {
    crate::ingest::app::account_routes::AccountRoutes::new()
        .post("/v1/account/device-codes", mint_handler)
        .get("/v1/account/device-codes/{code_id}", status_handler)
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

fn key32(value: &str) -> Option<[u8; 32]> {
    if value.len() > 48 {
        return None;
    }
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .ok()?
        .try_into()
        .ok()
}

fn verify(device: &[u8; 32], message: &[u8], signature_b64: &str) -> bool {
    if signature_b64.len() > 100 {
        return false;
    }
    let Some(signature) = base64::engine::general_purpose::STANDARD
        .decode(signature_b64)
        .ok()
        .and_then(|bytes| <[u8; 64]>::try_from(bytes).ok())
    else {
        return false;
    };
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, device)
        .verify(message, &signature)
        .is_ok()
}

fn within_skew(issued_at: i64, skew: i64) -> bool {
    (Utc::now().timestamp() - issued_at).abs() <= skew
}

/// The label to store on the code, `None` when blank, or `Err` for one that
/// is too long or carries control characters.
fn code_label(raw: Option<&str>) -> Result<Option<&str>, ()> {
    let Some(label) = raw.map(str::trim).filter(|label| !label.is_empty()) else {
        return Ok(None);
    };
    if label.chars().count() > DEVICE_CODE_LABEL_MAX_CHARS || label.chars().any(char::is_control) {
        return Err(());
    }
    Ok(Some(label))
}

/// `tcd1_{b64url(tenant_id)}.{secret}`.
fn device_code_value(tenant_id: &str, secret: &str) -> String {
    format!(
        "{DEVICE_CODE_PREFIX}{}.{secret}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(tenant_id.as_bytes()),
    )
}

/// Inverse of [`device_code_value`]: `(tenant_id, sha256(secret))`. The
/// tenant only scopes the lookup; a forged one finds no row, because the
/// secret's hash is globally unique.
fn device_code_parts(code: &str) -> Option<(String, String)> {
    if code.len() > 256 {
        return None;
    }
    let body = code.strip_prefix(DEVICE_CODE_PREFIX)?;
    let (b64_tenant, secret) = body.split_once('.')?;
    if secret.is_empty() {
        return None;
    }
    let tenant = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(b64_tenant)
        .ok()?;
    Some((String::from_utf8(tenant).ok()?, hash_secret(secret)))
}

fn nonce_hash(nonce: &[u8; 32]) -> String {
    hash_secret(&hex::encode(nonce))
}

async fn mint_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    Json(body): Json<DeviceCodeMintRequest>,
) -> ApiResult<Json<DeviceCodeMintResponse>> {
    // The proof is a device key's, and only the daemon holds one: a browser
    // session has no device to prove.
    if !matches!(ctx.auth_method, AccountAuthMethod::NativeToken) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            MINT_NATIVE_SESSION_REQUIRED,
        ));
    }
    let account = ctx.account_id.as_uuid();
    if !ACCOUNT_RATE_LIMITER.check_principal(
        &format!("device-code-mint:{account}"),
        DEVICE_CODE_MINT_PER_ACCOUNT_LIMIT,
    ) {
        return Err(api_error(StatusCode::TOO_MANY_REQUESTS, MINT_TOO_MANY));
    }
    let invalid = || api_error(StatusCode::BAD_REQUEST, MINT_INVALID);
    let minter = key32(&body.minter_public_key).ok_or_else(invalid)?;
    let nonce = key32(&body.nonce).ok_or_else(invalid)?;
    let label = code_label(body.label.as_deref()).map_err(|_| invalid())?;
    if !within_skew(body.issued_at, DEVICE_CODE_MINT_SKEW_SECS) {
        return Err(api_error(StatusCode::BAD_REQUEST, MINT_PROOF_STALE));
    }
    let message = device_code_mint_bytes(
        &ctx.tenant_id,
        &account.to_string(),
        &minter,
        &nonce,
        body.issued_at,
    );
    if !verify(&minter, &message, &body.signature) {
        return Err(invalid());
    }
    let minter_device_key_id =
        trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(&minter);
    let secret = generate_login_code();
    let code_hash = hash_secret(&secret);
    let mint_nonce_hash = nonce_hash(&nonce);
    let expires_at = Utc::now() + Duration::seconds(DEVICE_CODE_TTL_SECS);
    let db = account_db(state.as_ref())?;
    let outcome = db
        .mint_account_device_code(trace_commons_server::db::NewAccountDeviceCode {
            tenant_id: &ctx.tenant_id,
            account_id: account,
            minter_device_key_id: &minter_device_key_id,
            mint_nonce_hash: &mint_nonce_hash,
            code_hash: &code_hash,
            label,
            expires_at,
            outstanding_cap: DEVICE_CODE_OUTSTANDING_CAP,
        })
        .await
        .map_err(internal_error)?;
    let code_id = match outcome {
        MintAccountDeviceCodeOutcome::Minted { code_id } => code_id,
        MintAccountDeviceCodeOutcome::MinterNotADevice => {
            return Err(api_error(StatusCode::FORBIDDEN, MINT_NOT_A_DEVICE));
        }
        MintAccountDeviceCodeOutcome::TooManyOutstanding => {
            return Err(api_error(StatusCode::TOO_MANY_REQUESTS, MINT_TOO_MANY));
        }
        MintAccountDeviceCodeOutcome::NonceReused => {
            return Err(api_error(StatusCode::BAD_REQUEST, MINT_PROOF_REUSED));
        }
    };
    db.append_account_audit(
        &ctx.tenant_id,
        "account_device_code_minted",
        &ctx.actor_ref,
        "success",
        serde_json::json!({ "labeled": label.is_some() }),
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(DeviceCodeMintResponse {
        code_id: code_id.to_string(),
        code: device_code_value(&ctx.tenant_id, &secret),
        expires_at,
    }))
}

async fn status_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    AxumPath(code_id): AxumPath<String>,
) -> ApiResult<Json<DeviceCodeStatusResponse>> {
    let not_found = || api_error(StatusCode::NOT_FOUND, "device code not found");
    let code_id = uuid::Uuid::parse_str(&code_id).map_err(|_| not_found())?;
    let db = account_db(state.as_ref())?;
    let status = db
        .account_device_code_status(&ctx.tenant_id, ctx.account_id.as_uuid(), code_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    let state = match (&status.redeemed_device_key_id, status.expired) {
        (Some(_), _) => DeviceCodeState::Redeemed,
        (None, true) => DeviceCodeState::Expired,
        (None, false) => DeviceCodeState::Pending,
    };
    Ok(Json(DeviceCodeStatusResponse {
        state,
        device_key_id: status.redeemed_device_key_id,
    }))
}

fn device_sign_in_limited(headers: &HeaderMap) -> bool {
    !ACCOUNT_RATE_LIMITER.check(
        &format!("device-sign-in:{}", client_ip_for_rate_limit(headers)),
        DEVICE_SIGN_IN_PER_IP_LIMIT,
    ) || !ACCOUNT_RATE_LIMITER.check_global("device-sign-in:global", DEVICE_SIGN_IN_GLOBAL_LIMIT)
}

/// The sign-in answer both unauthenticated routes give, marked `no-store`.
fn signed_in(
    provisioned: &trace_commons_server::account_onboarding::ProvisionedNearAccount,
    secret: &str,
) -> axum::response::Response {
    let mut response = Json(DeviceSignInResponse {
        access_token: native_token_value(&provisioned.tenant_id, secret),
        token_type: "Bearer".to_string(),
        expires_in_secs: NATIVE_SESSION_TTL_HOURS * 3600,
        account_id: provisioned.account_id.to_string(),
        tenant_id: provisioned.tenant_id.clone(),
        device_key_id: provisioned.device_key_id.clone(),
        anchor_hash: provisioned.anchor_hash.clone(),
    })
    .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response
}

/// A labelled refusal from an unauthenticated route, marked `no-store`.
fn labelled(status: StatusCode, label: &str) -> axum::response::Response {
    let mut response = api_error(status, label).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response
}

pub(super) async fn redeem_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<DeviceCodeRedeemRequest>, JsonRejection>,
) -> axum::response::Response {
    let began = std::time::Instant::now();
    let result = redeem(state, headers, body).await;
    sleep_to_redeem_floor(began).await;
    result.unwrap_or_else(native_generic_deny)
}

async fn redeem(
    state: Arc<AppState>,
    headers: HeaderMap,
    body: Result<Json<DeviceCodeRedeemRequest>, JsonRejection>,
) -> Option<axum::response::Response> {
    if device_sign_in_limited(&headers) {
        return None;
    }
    let Json(body) = body.ok()?;
    let (tenant_id, code_hash) = device_code_parts(&body.code)?;
    let device = key32(&body.device_public_key)?;
    if !verify(
        &device,
        &device_code_redeem_bytes(&body.code, &device),
        &body.signature,
    ) {
        return None;
    }
    let db = account_db(&state).ok()?;
    let secret = generate_session_secret();
    let token_hash = hash_secret(&secret);
    let outcome = db
        .redeem_account_device_code(
            &tenant_id,
            &code_hash,
            &device,
            trace_commons_server::db::NewSession {
                token_hash: &token_hash,
                client_kind: NATIVE_SESSION_CLIENT_KIND,
                expires_at: Utc::now() + Duration::hours(NATIVE_SESSION_TTL_HOURS),
            },
            state.near_ai_signup_policy_template.as_ref(),
        )
        .await
        .ok()?;
    match outcome {
        DeviceSignInOutcome::SignedIn(provisioned) => Some(signed_in(&provisioned, &secret)),
        // The caller proved it holds this key, and the fact is about its
        // own device, so it is told rather than given the generic deny.
        DeviceSignInOutcome::DeviceKeyElsewhere => Some(labelled(
            StatusCode::CONFLICT,
            DEVICE_KEY_REGISTERED_ELSEWHERE,
        )),
        DeviceSignInOutcome::NotFound
        | DeviceSignInOutcome::NonceReused
        | DeviceSignInOutcome::TooSoon => None,
    }
}

pub(super) async fn renew_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<DeviceSessionRenewRequest>, JsonRejection>,
) -> axum::response::Response {
    let began = std::time::Instant::now();
    let result = renew(state, headers, body).await;
    sleep_to_redeem_floor(began).await;
    result.unwrap_or_else(native_generic_deny)
}

async fn renew(
    state: Arc<AppState>,
    headers: HeaderMap,
    body: Result<Json<DeviceSessionRenewRequest>, JsonRejection>,
) -> Option<axum::response::Response> {
    if device_sign_in_limited(&headers) {
        return None;
    }
    let Json(body) = body.ok()?;
    if body.tenant_id.is_empty() || body.tenant_id.len() > 128 {
        return None;
    }
    let device = key32(&body.device_public_key)?;
    let nonce = key32(&body.nonce)?;
    if !within_skew(body.issued_at, DEVICE_SESSION_RENEW_SKEW_SECS) {
        return None;
    }
    if !verify(
        &device,
        &device_session_renew_bytes(&body.tenant_id, &device, &nonce, body.issued_at),
        &body.signature,
    ) {
        return None;
    }
    let db = account_db(&state).ok()?;
    let secret = generate_session_secret();
    let token_hash = hash_secret(&secret);
    let outcome = db
        .renew_device_session(
            &body.tenant_id,
            &device,
            &nonce_hash(&nonce),
            trace_commons_server::db::NewSession {
                token_hash: &token_hash,
                client_kind: NATIVE_SESSION_CLIENT_KIND,
                expires_at: Utc::now() + Duration::hours(NATIVE_SESSION_TTL_HOURS),
            },
            Duration::minutes(DEVICE_SESSION_RENEW_MIN_INTERVAL_MINUTES),
        )
        .await
        .ok()?;
    match outcome {
        DeviceSignInOutcome::SignedIn(provisioned) => Some(signed_in(&provisioned, &secret)),
        DeviceSignInOutcome::TooSoon => Some(labelled(
            StatusCode::TOO_MANY_REQUESTS,
            DEVICE_RENEW_TOO_SOON,
        )),
        DeviceSignInOutcome::NotFound
        | DeviceSignInOutcome::NonceReused
        | DeviceSignInOutcome::DeviceKeyElsewhere => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_round_trips_its_tenant_and_hashes_only_the_secret() {
        let code = device_code_value("nearai-abc", "s3cret");
        assert!(code.starts_with(DEVICE_CODE_PREFIX));
        let (tenant, hash) = device_code_parts(&code).unwrap();
        assert_eq!(tenant, "nearai-abc");
        assert_eq!(hash, hash_secret("s3cret"));
        assert!(device_code_parts("tcd1_bm9wZQ.").is_none());
        assert!(device_code_parts("tcn1_bm9wZQ.x").is_none());
        assert!(device_code_parts(&format!("{code}{}", "x".repeat(256))).is_none());
    }

    #[test]
    fn a_label_is_trimmed_and_bounded() {
        assert_eq!(code_label(Some("  ")), Ok(None));
        assert_eq!(code_label(Some(" box ")), Ok(Some("box")));
        assert!(code_label(Some(&"x".repeat(DEVICE_CODE_LABEL_MAX_CHARS + 1))).is_err());
        assert!(code_label(Some("a\tb")).is_err());
    }

    #[test]
    fn a_signature_verifies_only_for_its_key_and_message() {
        use ring::signature::{Ed25519KeyPair, KeyPair};
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let key: [u8; 32] = pair.public_key().as_ref().try_into().unwrap();
        let message = device_code_redeem_bytes("tcd1_x.y", &key);
        let signature =
            base64::engine::general_purpose::STANDARD.encode(pair.sign(&message).as_ref());
        assert!(verify(&key, &message, &signature));
        assert!(!verify(
            &key,
            &device_code_redeem_bytes("tcd1_x.z", &key),
            &signature
        ));
        assert!(!verify(&[7; 32], &message, &signature));
        assert!(!verify(&key, &message, "not base64"));
    }

    #[test]
    fn a_proof_outside_the_skew_is_stale() {
        let now = Utc::now().timestamp();
        assert!(within_skew(now, 120));
        assert!(within_skew(now - 100, 120));
        assert!(!within_skew(now - 200, 120));
        assert!(!within_skew(now + 200, 120));
    }
}
