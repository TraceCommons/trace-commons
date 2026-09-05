// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use trace_commons_server::account_onboarding::{
    NativeProvisioningPending, PendingNearProvisioning, ProvisioningAssertion,
};

/// Operator-published trust data, served only after the admission gate is ready.
#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct PublishedWitness {
    url: String,
    signing_address: String,
    expected_measurements: Vec<String>,
}
fn published_witness(state: &AppState) -> Option<PublishedWitness> {
    if !state.near_provisioning_enabled || !state.near_provisioning_admission_ready {
        return None;
    }
    let origin = reqwest::Url::parse(state.near_provisioning_public_origin.as_ref()?).ok()?;
    if origin.scheme() != "https"
        || origin.host_str().is_none()
        || origin.path() != "/"
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return None;
    }
    account_near_config(state).ok()?;
    account_db(state).ok()?;
    let raw = std::env::var("TRACE_COMMONS_NEAR_PROVISIONING_WITNESS_JSON").ok()?;
    let witness: PublishedWitness = serde_json::from_str(&raw).ok()?;
    validate_witness(&witness)?;
    published_issuer()?;
    Some(witness)
}
fn validate_witness(witness: &PublishedWitness) -> Option<()> {
    let url = reqwest::Url::parse(&witness.url).ok()?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || witness.expected_measurements.is_empty()
    {
        return None;
    }
    let address = witness.signing_address.strip_prefix("0x")?;
    if address.len() != 40 || !address.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    for entry in &witness.expected_measurements {
        trace_commons_attestation::measurements::ExpectedMeasurements::from_env_value(Some(entry))
            .ok()??;
    }
    Some(())
}
fn published_issuer() -> Option<(String, String)> {
    let issuer = std::env::var("TRACE_COMMONS_NEAR_PROVISIONING_ISSUER_URL").ok()?;
    let audience = std::env::var("TRACE_COMMONS_NEAR_PROVISIONING_AUDIENCE").ok()?;
    let url = reqwest::Url::parse(&issuer).ok()?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || audience.trim().is_empty()
        || audience.len() > 256
    {
        return None;
    }
    Some((issuer, audience))
}
pub(super) async fn capabilities(State(state): State<Arc<AppState>>) -> axum::response::Response {
    match published_witness(&state) {
        Some(witness) => {
            let Some((issuer_url, audience)) = published_issuer() else {
                return response(serde_json::json!({"ready":false,"funding_available":false}));
            };
            let network = account_near_config(&state).ok().map(|c| c.network.clone());
            response(
                serde_json::json!({"ready":true,"network":network,"witness":witness,"issuer_url":issuer_url,"audience":audience,"funding_available":false}),
            )
        }
        None => response(serde_json::json!({"ready":false,"funding_available":false})),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartRequest {
    account_id: String,
    device_public_key: String,
    code_challenge: String,
    code_challenge_method: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FinishRequest {
    ceremony_id: String,
    account_id: String,
    device_public_key: String,
    wallet_public_key: String,
    wallet_signature: String,
    device_signature: String,
    code_verifier: String,
}

fn device_key(value: &str) -> Option<[u8; 32]> {
    if value.len() > 48 {
        return None;
    }
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .ok()?
        .try_into()
        .ok()
}
fn binding(challenge: &str) -> [u8; 32] {
    Sha256::digest(format!("trace_commons.near_native_pkce.v1\n{challenge}")).into()
}
fn limited(headers: &HeaderMap, action: &str) -> bool {
    !ACCOUNT_RATE_LIMITER.check(
        &format!(
            "near-provision-{action}:{}",
            client_ip_for_rate_limit(headers)
        ),
        30,
    ) || !ACCOUNT_RATE_LIMITER.check(&format!("near-provision-{action}:global"), 600)
}
fn response(value: serde_json::Value) -> axum::response::Response {
    let mut response = Json(value).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        axum::http::header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

pub(super) async fn near_provision_start_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<StartRequest>, JsonRejection>,
) -> axum::response::Response {
    let began = std::time::Instant::now();
    let result = start(state, headers, body).await;
    sleep_to_redeem_floor(began).await;
    result.unwrap_or_else(native_generic_deny)
}
async fn start(
    state: Arc<AppState>,
    headers: HeaderMap,
    body: Result<Json<StartRequest>, JsonRejection>,
) -> Option<axum::response::Response> {
    if published_witness(&state).is_none() || limited(&headers, "start") {
        return None;
    }
    let Json(body) = body.ok()?;
    if body.code_challenge_method != "S256" || !challenge_is_wellformed(&body.code_challenge) {
        return None;
    }
    let cfg = account_near_config(&state).ok()?;
    let db = account_db(&state).ok()?;
    let device = device_key(&body.device_public_key)?;
    let wallet_url = format!(
        "{}/account/near/provision/wallet",
        state
            .near_provisioning_public_origin
            .as_ref()?
            .trim_end_matches('/')
    );
    let pending = PendingNearProvisioning::issue(
        &cfg,
        &body.account_id,
        device,
        binding(&body.code_challenge),
        Utc::now().timestamp(),
    )
    .ok()?
    .with_wallet_callback(&wallet_url)
    .ok()?;
    let challenge = pending.challenge();
    let ceremony_id = generate_login_code();
    let expires_at = challenge.expires_at;
    let result = serde_json::json!({
        "ceremony_id":ceremony_id,"message":challenge.message,
        "nonce":base64::engine::general_purpose::STANDARD.encode(challenge.nonce),
        "recipient":challenge.recipient,"expires_at":expires_at,"wallet_url":wallet_url,"network":cfg.network,
        "device_signing_bytes":base64::engine::general_purpose::STANDARD.encode(pending.device_signing_bytes())
    });
    db.store_near_provisioning_ceremony(
        &hash_secret(&ceremony_id),
        NativeProvisioningPending {
            ceremony: pending.into_stored(),
            code_challenge: body.code_challenge,
        },
        expires_at,
    )
    .await
    .ok()?;
    Some(response(result))
}

pub(super) async fn near_provision_finish_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<FinishRequest>, JsonRejection>,
) -> axum::response::Response {
    let began = std::time::Instant::now();
    let result = finish(state, headers, body).await;
    sleep_to_redeem_floor(began).await;
    result.unwrap_or_else(native_generic_deny)
}
async fn finish(
    state: Arc<AppState>,
    headers: HeaderMap,
    body: Result<Json<FinishRequest>, JsonRejection>,
) -> Option<axum::response::Response> {
    if published_witness(&state).is_none() || limited(&headers, "finish") {
        return None;
    }
    let Json(body) = body.ok()?;
    if body.ceremony_id.len() > 64
        || !verifier_is_wellformed(&body.code_verifier)
        || body.wallet_public_key.len() > 120
        || body.wallet_signature.len() > 128
        || body.device_signature.len() > 128
    {
        return None;
    }
    let cfg = account_near_config(&state).ok()?;
    let db = account_db(&state).ok()?;
    let device = device_key(&body.device_public_key)?;
    let hash = hash_secret(&body.ceremony_id);
    if !ACCOUNT_RATE_LIMITER.check(&format!("near-provision-ceremony:{hash}"), 5) {
        return None;
    }
    let record = db.take_near_provisioning_ceremony(&hash).await.ok()??;
    if !secret_eq(
        &record.code_challenge,
        &challenge_for_verifier(&body.code_verifier),
    ) {
        return None;
    }
    let browser_binding = binding(&record.code_challenge);
    let wallet_url = format!(
        "{}/account/near/provision/wallet",
        state
            .near_provisioning_public_origin
            .as_ref()?
            .trim_end_matches('/')
    );
    let pending = PendingNearProvisioning::restore(
        record.ceremony,
        &cfg,
        &body.account_id,
        device,
        Some(&wallet_url),
    )
    .ok()?;
    let verified = pending
        .verify(
            &cfg,
            ProvisioningAssertion {
                wallet_public_key: &body.wallet_public_key,
                wallet_signature: &body.wallet_signature,
                device_signature: &body.device_signature,
            },
            &browser_binding,
            Utc::now().timestamp(),
        )
        .await
        .ok()?;
    let secret = generate_session_secret();
    let token_hash = hash_secret(&secret);
    let provisioned = db
        .provision_verified_near_account(
            verified,
            trace_commons_server::db::NewSession {
                token_hash: &token_hash,
                client_kind: NATIVE_SESSION_CLIENT_KIND,
                expires_at: Utc::now() + Duration::hours(NATIVE_SESSION_TTL_HOURS),
            },
        )
        .await
        .ok()?;
    Some(response(serde_json::json!({
        "access_token":native_token_value(&provisioned.tenant_id,&secret),"token_type":"Bearer",
        "expires_in_secs":NATIVE_SESSION_TTL_HOURS*3600,"account_id":provisioned.account_id,
        "tenant_id":provisioned.tenant_id,"device_key_id":provisioned.device_key_id,"anchor_hash":provisioned.anchor_hash
    })))
}

pub(super) async fn wallet_page(State(state): State<Arc<AppState>>) -> axum::response::Response {
    if !state.near_provisioning_enabled {
        return native_generic_deny();
    }
    let mut response =
        axum::response::Html(include_str!("near_provisioning_wallet.html")).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        axum::http::header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(axum::http::header::CONTENT_SECURITY_POLICY,HeaderValue::from_static("default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn published_trust_requires_https_address_and_nonempty_parsed_pins() {
        let mut value = PublishedWitness {
            url: "https://witness.example".into(),
            signing_address: format!("0x{}", "ab".repeat(20)),
            expected_measurements: vec![format!("mrtd={}", "ab".repeat(48))],
        };
        assert!(validate_witness(&value).is_some());
        value.expected_measurements = vec![String::new()];
        assert!(validate_witness(&value).is_none());
        value.expected_measurements = vec![format!("mrtd={}", "ab".repeat(48))];
        value.url = "http://witness.example".into();
        assert!(validate_witness(&value).is_none());
        value.url = "https://user@witness.example".into();
        assert!(validate_witness(&value).is_none());
        value.url = "https://witness.example".into();
        value.signing_address = "0xinvalid".into();
        assert!(validate_witness(&value).is_none());
    }
    #[test]
    fn requests_reject_extra_fields_and_pkce_is_device_bound() {
        assert!(serde_json::from_value::<StartRequest>(serde_json::json!({"account_id":"alice.near","device_public_key":"key","code_challenge":"challenge","code_challenge_method":"S256","auto_admit":true})).is_err());
        assert_ne!(binding("first"), binding("second"));
        assert!(device_key(&"x".repeat(49)).is_none());
        assert!(device_key("bad").is_none());
    }
}
