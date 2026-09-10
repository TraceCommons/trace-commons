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
/// The controls `published_witness` walks, as stable labels.
///
/// Label-only by rule: these name a *control*, never a value. Nothing here may
/// come to carry an environment value, a URL, a signing address or a
/// measurement string -- see the repo's hash-only/label-only convention and
/// [`crate::redaction_witness::config`], whose errors are `&'static str`
/// control names for the same reason. `every_control_label_is_a_bare_name`
/// holds the line.
mod control {
    pub(super) const PROVISIONING_ENABLED: &str = "near_provisioning_enabled";
    pub(super) const ADMISSION_READY: &str = "near_provisioning_admission_ready";
    pub(super) const PUBLIC_ORIGIN: &str = "witness_public_origin";
    pub(super) const NEAR_SIGN_IN: &str = "near_sign_in";
    pub(super) const ACCOUNT_REGISTRY_DB: &str = "account_registry_db";
    pub(super) const WITNESS_JSON_ABSENT: &str = "witness_json_absent";
    pub(super) const WITNESS_JSON_MALFORMED: &str = "witness_json_malformed";
    pub(super) const WITNESS_URL: &str = "witness_url";
    pub(super) const WITNESS_SIGNING_ADDRESS: &str = "witness_signing_address";
    pub(super) const WITNESS_MEASUREMENTS_ABSENT: &str = "witness_measurements_absent";
    pub(super) const WITNESS_MEASUREMENT_SYNTAX: &str = "witness_measurement_syntax";
    pub(super) const ISSUER: &str = "issuer";
    /// The index pepper and account-name KEK required by both enrollment paths.
    pub(super) const NEAR_ACCOUNT_IDENTITY: &str = "near_account_identity";

    /// Every label, for the tests that hold the label-only rule.
    #[cfg(test)]
    pub(super) const ALL: [&str; 13] = [
        PROVISIONING_ENABLED,
        ADMISSION_READY,
        PUBLIC_ORIGIN,
        NEAR_SIGN_IN,
        ACCOUNT_REGISTRY_DB,
        WITNESS_JSON_ABSENT,
        WITNESS_JSON_MALFORMED,
        WITNESS_URL,
        WITNESS_SIGNING_ADDRESS,
        WITNESS_MEASUREMENTS_ABSENT,
        WITNESS_MEASUREMENT_SYNTAX,
        ISSUER,
        NEAR_ACCOUNT_IDENTITY,
    ];
}

/// Controls already reported, so a polled endpoint does not repeat itself.
///
/// Per distinct control rather than once overall: an operator who fixes one
/// gate and trips the next needs the next one named too, and the set is
/// bounded by [`control::ALL`], so this cannot grow without bound however
/// often the endpoint is polled.
static REPORTED_CONTROLS: std::sync::Mutex<Option<BTreeSet<&'static str>>> =
    std::sync::Mutex::new(None);

/// Report `control` unless it has been reported already.
///
/// Returns whether it was newly reported, which is what makes the dedup
/// testable without capturing a log.
fn report_declined_control(control: &'static str) -> bool {
    let mut reported = match REPORTED_CONTROLS.lock() {
        Ok(reported) => reported,
        // A poisoned lock must not take the endpoint down: the whole point of
        // this path is that it is diagnostic and cannot change behaviour.
        Err(poisoned) => poisoned.into_inner(),
    };
    let first = reported.get_or_insert_with(BTreeSet::new).insert(control);
    if first {
        // The control name and nothing else. No env value, no URL, no
        // measurement string, no signing address.
        tracing::warn!(
            control = control,
            "native provisioning is not ready: this control declined"
        );
    }
    first
}

fn published_witness(state: &AppState) -> Option<PublishedWitness> {
    match published_witness_named(state) {
        Ok(witness) => Some(witness),
        Err(control) => {
            report_declined_control(control);
            None
        }
    }
}

/// `published_witness`, naming the control that declined.
///
/// Includes the identity controls used by wallet completion, so a client is
/// not invited to sign a ceremony this commons cannot finish.
fn published_witness_named(state: &AppState) -> Result<PublishedWitness, &'static str> {
    if !state.near_provisioning_enabled {
        return Err(control::PROVISIONING_ENABLED);
    }
    if !state.near_provisioning_admission_ready {
        return Err(control::ADMISSION_READY);
    }
    if state.near_account_identity.is_none() {
        return Err(control::NEAR_ACCOUNT_IDENTITY);
    }
    let origin = state
        .near_provisioning_public_origin
        .as_ref()
        .ok_or(control::PUBLIC_ORIGIN)?;
    let origin = reqwest::Url::parse(origin).map_err(|_| control::PUBLIC_ORIGIN)?;
    if origin.scheme() != "https"
        || origin.host_str().is_none()
        || origin.path() != "/"
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(control::PUBLIC_ORIGIN);
    }
    account_near_config(state).map_err(|_| control::NEAR_SIGN_IN)?;
    account_db(state).map_err(|_| control::ACCOUNT_REGISTRY_DB)?;
    let witness = published_witness_material_named()?;
    published_issuer().ok_or(control::ISSUER)?;
    Ok(witness)
}

#[cfg(test)]
pub(super) fn wallet_readiness_refusal(state: &AppState) -> Option<&'static str> {
    published_witness_named(state).err()
}

/// The published witness, with **no ceremony-specific preconditions**.
///
/// Factored out of [`published_witness_named`] rather than duplicated: the
/// witness is a property of the commons, and both enrolment ceremonies need
/// the same one. What differs is what *else* each requires, which is why the
/// preconditions stayed with the callers.
///
/// Names the same controls #831 gave these gates, because they are the same
/// gates -- an operator whose witness JSON is malformed should read that fact
/// once, however they reached it.
fn published_witness_material_named() -> Result<PublishedWitness, &'static str> {
    let raw = std::env::var("TRACE_COMMONS_NEAR_PROVISIONING_WITNESS_JSON")
        .map_err(|_| control::WITNESS_JSON_ABSENT)?;
    let witness: PublishedWitness =
        serde_json::from_str(&raw).map_err(|_| control::WITNESS_JSON_MALFORMED)?;
    validate_witness_named(&witness)?;
    Ok(witness)
}

/// Whether a NEAR AI login enrolment can actually complete here (#836).
///
/// **Deliberately not `published_witness`'s predicate.** That one refuses on
/// two grounds belonging to the wallet mechanism: `account_near_config`, which
/// is NEP-413 sign-in configuration, and `near_provisioning_public_origin`,
/// which exists so the wallet ceremony can redirect a browser to its wallet
/// page. The login ceremony has no browser redirect and never reads the
/// sign-in config, so a commons offering only this path would have had its
/// enrolments refused for reasons that are not about it -- which is the shape
/// #836 exists to remove, appearing one layer above admission.
///
/// What it does require is what this path needs to leave a contributor able to
/// upload, which is the failure worth preventing: enrolled, and then refused at
/// their first submission.
fn near_ai_login_ready(state: &AppState) -> bool {
    match near_ai_login_ready_named(state) {
        Ok(()) => true,
        Err(control) => {
            report_declined_control(control);
            false
        }
    }
}

/// `near_ai_login_ready`, naming the control that declined.
///
/// **Shared controls keep their existing labels; only what is genuinely new
/// gets a new one.** The provisioning switch, the admission gate, the account
/// registry, the witness JSON and the issuer are properties of the commons,
/// not of either ceremony, so reusing #831's labels for them is not a wallet
/// label describing a login refusal -- it is one control with one name. The
/// two wallet-specific labels, `NEAR_SIGN_IN` and `PUBLIC_ORIGIN`, are
/// deliberately unreachable from here.
///
/// Both paths require [`control::NEAR_ACCOUNT_IDENTITY`], while wallet-only
/// configuration remains outside this predicate.
fn near_ai_login_ready_named(state: &AppState) -> Result<(), &'static str> {
    if !state.near_provisioning_enabled {
        return Err(control::PROVISIONING_ENABLED);
    }
    if !state.near_provisioning_admission_ready {
        return Err(control::ADMISSION_READY);
    }
    account_db(state).map_err(|_| control::ACCOUNT_REGISTRY_DB)?;
    if state.near_account_identity.is_none() {
        return Err(control::NEAR_ACCOUNT_IDENTITY);
    }
    published_witness_material_named()?;
    published_issuer().ok_or(control::ISSUER)?;
    Ok(())
}

/// The original `Option` shape, kept as the reference the equivalence test
/// compares against. Production goes through [`validate_witness_named`].
#[cfg(test)]
fn validate_witness(witness: &PublishedWitness) -> Option<()> {
    validate_witness_named(witness).ok()
}

/// `validate_witness`, naming which part of the witness declined.
///
/// The measurement syntax gets its own label, separate from
/// `witness_json_malformed`, and that distinction is the point of the whole
/// change. This repo has **two** measurement-pin spellings for the same
/// witness -- `field=<96 hex>` joined by commas, which is what
/// `ExpectedMeasurements` parses and what this field takes, and
/// `mrtd:<hex>+mrconfigid:<hex>`, which is a `WitnessPin` and what
/// `TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS` takes. The wrong one is
/// still perfectly good JSON, so it declines here while looking like a
/// well-formed document, and folding it into a JSON error would send an
/// operator to inspect the one thing that is not wrong.
fn validate_witness_named(witness: &PublishedWitness) -> Result<(), &'static str> {
    let url = reqwest::Url::parse(&witness.url).map_err(|_| control::WITNESS_URL)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(control::WITNESS_URL);
    }
    if witness.expected_measurements.is_empty() {
        return Err(control::WITNESS_MEASUREMENTS_ABSENT);
    }
    let address = witness
        .signing_address
        .strip_prefix("0x")
        .ok_or(control::WITNESS_SIGNING_ADDRESS)?;
    if address.len() != 40 || !address.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(control::WITNESS_SIGNING_ADDRESS);
    }
    for entry in &witness.expected_measurements {
        // `Ok(None)` -- an empty or whitespace entry -- is a refusal here too,
        // exactly as the `??` it replaces made it.
        match trace_commons_attestation::measurements::ExpectedMeasurements::from_env_value(Some(
            entry,
        )) {
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return Err(control::WITNESS_MEASUREMENT_SYNTAX),
        }
    }
    Ok(())
}
/// A receipt-service base URL fit to publish to clients.
///
/// Clients append `/signature/{chat_id}` and a served-model query to this, so
/// a value carrying a query, a fragment or credentials of its own would be
/// silently mangled or would leak. Refused here rather than left for every
/// client to rediscover. The same shape `published_issuer` requires.
fn validate_receipt_endpoint(endpoint: &str) -> Option<()> {
    let url = reqwest::Url::parse(endpoint).ok()?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(())
}

/// The receipt endpoint an operator has chosen to publish, if any.
///
/// Optional, unlike the issuer: a commons that serves no attested inference
/// publishes nothing here and its clients stay unattested, which is the
/// behaviour they already have.
fn published_receipt_endpoint() -> Option<String> {
    let endpoint = std::env::var("TRACE_COMMONS_NEAR_PROVISIONING_RECEIPT_ENDPOINT").ok()?;
    validate_receipt_endpoint(&endpoint)?;
    Some(endpoint)
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
/// What this commons can enrol, and the properties both ceremonies need.
///
/// **Two readiness flags, one route.** `ready` is the wallet ceremony and its
/// meaning is unchanged -- redefining it would silently alter behaviour for
/// every deployed wallet client with no test failing. `near_ai_login_ready` is
/// the login ceremony (#836). A commons may offer either, both or neither, and
/// an older client simply ignores the field it does not know.
///
/// The witness, issuer, audience and receipt endpoint are properties of the
/// commons rather than of either ceremony, so they are published whenever
/// **either** path can use them. Withholding them from a login-only commons is
/// what leaves a contributor enrolled and unable to upload -- the config is
/// written with an empty issuer and fails at their first submission rather
/// than at enrolment.
pub(super) async fn capabilities(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let wallet = published_witness(&state);
    let login_ready = near_ai_login_ready(&state);
    // The wallet's own answer, preserved exactly: witness AND issuer.
    let wallet_ready = wallet.is_some() && published_issuer().is_some();
    let witness = wallet.or_else(|| published_witness_material_named().ok());
    match (witness, published_issuer()) {
        (Some(witness), Some((issuer_url, audience))) if wallet_ready || login_ready => {
            // Absent on a login-only commons, which is correct: it is the
            // wallet sign-in network and there is no wallet here. Already
            // `Option`, already serialised as null, and no consumer requires it.
            let network = account_near_config(&state).ok().map(|c| c.network.clone());
            response(
                serde_json::json!({"ready":wallet_ready,"near_ai_login_ready":login_ready,"network":network,"witness":witness,"issuer_url":issuer_url,"audience":audience,"inference_receipt_endpoint":published_receipt_endpoint(),"funding_available":false}),
            )
        }
        _ => response(
            serde_json::json!({"ready":false,"near_ai_login_ready":false,"funding_available":false}),
        ),
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
    // Absent identity controls collapse to the same uniform deny as every other
    // failure on this path. Boot already refuses when provisioning is enabled
    // without them, so reaching here means provisioning is off.
    let identity = state.near_account_identity.as_ref()?;
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
            identity,
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
    /// The receipt endpoint is published to clients that will call it with a
    /// `chat_id` on the path and a model on the query, so a value carrying a
    /// query, a fragment or credentials of its own is refused here rather
    /// than left for every client to discover separately.
    #[test]
    fn a_published_receipt_endpoint_must_be_a_bare_https_origin() {
        assert!(validate_receipt_endpoint("https://cloud-api.near.ai/v1").is_some());
        for refused in [
            "http://cloud-api.near.ai/v1",
            "https://user@cloud-api.near.ai/v1",
            "https://user:secret@cloud-api.near.ai/v1",
            "https://cloud-api.near.ai/v1?model=x",
            "https://cloud-api.near.ai/v1#fragment",
            "not a url",
            "",
        ] {
            assert!(
                validate_receipt_endpoint(refused).is_none(),
                "{refused} was published"
            );
        }
    }
    #[test]
    fn requests_reject_extra_fields_and_pkce_is_device_bound() {
        assert!(serde_json::from_value::<StartRequest>(serde_json::json!({"account_id":"alice.near","device_public_key":"key","code_challenge":"challenge","code_challenge_method":"S256","auto_admit":true})).is_err());
        assert_ne!(binding("first"), binding("second"));
        assert!(device_key(&"x".repeat(49)).is_none());
        assert!(device_key("bad").is_none());
    }

    /// A witness whose measurements use the *other* spelling this repo has for
    /// the same pins.
    ///
    /// `mrtd:<hex>+mrconfigid:<hex>` is a `WitnessPin`, correct for
    /// `TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS`. The provisioning JSON
    /// takes `ExpectedMeasurements`, which is `field=<96 hex>` joined by
    /// commas. Same witness, same signing address, same URL.
    fn witness_with_the_certificate_pin_spelling() -> PublishedWitness {
        PublishedWitness {
            url: "https://witness.example".into(),
            signing_address: format!("0x{}", "ab".repeat(20)),
            expected_measurements: vec![format!(
                "mrtd:{}+mrconfigid:{}",
                "ab".repeat(48),
                "cd".repeat(48)
            )],
        }
    }

    fn well_formed_witness() -> PublishedWitness {
        PublishedWitness {
            url: "https://witness.example".into(),
            signing_address: format!("0x{}", "ab".repeat(20)),
            expected_measurements: vec![format!("mrtd={}", "ab".repeat(48))],
        }
    }

    /// **The motivating case.** The wrong pin spelling is perfectly good JSON,
    /// so it must not be reported as a JSON problem -- that sends an operator
    /// to inspect the one thing that is not wrong, which is what cost a
    /// restart cycle on the pilot.
    #[test]
    fn the_wrong_measurement_spelling_is_named_as_syntax_not_as_malformed_json() {
        let witness = witness_with_the_certificate_pin_spelling();
        // Precondition: it really is valid JSON, so a JSON-level label would
        // be actively misleading rather than merely vague.
        let raw = serde_json::to_string(&witness).expect("it serializes");
        assert!(
            serde_json::from_str::<PublishedWitness>(&raw).is_ok(),
            "precondition: the document parses; only the pin spelling is wrong"
        );

        assert_eq!(
            validate_witness_named(&witness),
            Err(control::WITNESS_MEASUREMENT_SYNTAX)
        );
        assert_ne!(
            validate_witness_named(&witness),
            Err(control::WITNESS_JSON_MALFORMED)
        );
    }

    /// And the spelling that *is* correct for this field is accepted, so the
    /// test above is about the spelling and not about the parser refusing
    /// everything.
    #[test]
    fn the_expected_measurements_spelling_is_the_one_this_field_accepts() {
        assert_eq!(validate_witness_named(&well_formed_witness()), Ok(()));
    }

    /// Each refusal names its own control. A single label for the whole
    /// witness would be no better than the `None` it replaces.
    #[test]
    fn each_part_of_the_witness_declines_under_its_own_name() {
        let mut witness = well_formed_witness();
        witness.url = "http://witness.example".into();
        assert_eq!(validate_witness_named(&witness), Err(control::WITNESS_URL));

        let mut witness = well_formed_witness();
        witness.url = "https://user@witness.example".into();
        assert_eq!(validate_witness_named(&witness), Err(control::WITNESS_URL));

        let mut witness = well_formed_witness();
        witness.signing_address = "0xinvalid".into();
        assert_eq!(
            validate_witness_named(&witness),
            Err(control::WITNESS_SIGNING_ADDRESS)
        );

        let mut witness = well_formed_witness();
        witness.signing_address = "ab".repeat(20);
        assert_eq!(
            validate_witness_named(&witness),
            Err(control::WITNESS_SIGNING_ADDRESS),
            "a missing 0x prefix is an address problem, not a syntax one"
        );

        let mut witness = well_formed_witness();
        witness.expected_measurements = Vec::new();
        assert_eq!(
            validate_witness_named(&witness),
            Err(control::WITNESS_MEASUREMENTS_ABSENT)
        );

        let mut witness = well_formed_witness();
        witness.expected_measurements = vec![String::new()];
        assert_eq!(
            validate_witness_named(&witness),
            Err(control::WITNESS_MEASUREMENT_SYNTAX),
            "an empty entry is a syntax refusal, as the `??` it replaced made it"
        );

        // Every one of those is a different label. A refactor that collapsed
        // them would satisfy every assertion above taken singly.
        let labels: BTreeSet<&str> = [
            control::WITNESS_URL,
            control::WITNESS_SIGNING_ADDRESS,
            control::WITNESS_MEASUREMENTS_ABSENT,
            control::WITNESS_MEASUREMENT_SYNTAX,
        ]
        .into_iter()
        .collect();
        assert_eq!(labels.len(), 4);
    }

    /// The safety property for this change: naming a control must not move
    /// any gate. Checked against the original predicates, re-stated here by
    /// hand, over a battery that exercises every arm.
    #[test]
    fn naming_a_control_does_not_change_which_configurations_are_accepted() {
        let hex96 = "ab".repeat(48);
        let mut cases = Vec::new();
        for url in [
            "https://witness.example",
            "http://witness.example",
            "https://user@witness.example",
            "https://witness.example/?q=1",
            "https://witness.example/#f",
            "not a url",
        ] {
            for address in [
                format!("0x{}", "ab".repeat(20)),
                "0xinvalid".into(),
                "ab".repeat(20),
            ] {
                for measurements in [
                    vec![format!("mrtd={hex96}")],
                    vec![format!("mrtd={hex96}"), format!("rtmr0={hex96}")],
                    vec![format!("mrtd:{hex96}+mrconfigid:{hex96}")],
                    vec![String::new()],
                    vec![],
                ] {
                    cases.push(PublishedWitness {
                        url: url.into(),
                        signing_address: address.clone(),
                        expected_measurements: measurements,
                    });
                }
            }
        }
        assert_eq!(cases.len(), 90, "the battery covers every combination");

        // Deliberately no `Debug` on `PublishedWitness`, and none added for
        // this test: a derived `Debug` is a path for a URL and a signing
        // address to reach a log by accident. The case index is enough to
        // locate a failure.
        for (index, witness) in cases.iter().enumerate() {
            // The original predicate, written out independently of the code
            // under test.
            let accepted_before = (|| {
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
                    trace_commons_attestation::measurements::ExpectedMeasurements::from_env_value(
                        Some(entry),
                    )
                    .ok()??;
                }
                Some(())
            })()
            .is_some();

            assert_eq!(
                validate_witness_named(witness).is_ok(),
                accepted_before,
                "verdict moved for case {index}"
            );
            assert_eq!(validate_witness(witness).is_some(), accepted_before);
        }
        // And the battery is not vacuous in either direction.
        assert!(cases.iter().any(|w| validate_witness_named(w).is_ok()));
        assert!(cases.iter().any(|w| validate_witness_named(w).is_err()));
    }

    /// The hash-only/label-only rule, enforced rather than described: a
    /// control name is a bare identifier and can never carry an operator
    /// value.
    #[test]
    fn every_control_label_is_a_bare_name() {
        for label in control::ALL {
            assert!(
                label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b == b'_' || b.is_ascii_digit()),
                "{label} is not a bare snake_case name"
            );
            assert!(!label.contains("://") && !label.contains('.') && !label.contains('='));
        }
        // Distinct labels, or two controls send an operator to one place.
        let unique: BTreeSet<&str> = control::ALL.into_iter().collect();
        assert_eq!(unique.len(), control::ALL.len());

        // And a refusal really returns one of them, rather than a message
        // built from the input.
        let leaky = PublishedWitness {
            url: "https://secret-host.example/path?token=SHOULD-NOT-APPEAR".into(),
            signing_address: "0xSHOULD-NOT-APPEAR".into(),
            expected_measurements: vec!["SHOULD-NOT-APPEAR".into()],
        };
        let refusal = validate_witness_named(&leaky).expect_err("it declines");
        assert!(control::ALL.contains(&refusal));
        assert!(!refusal.contains("SHOULD-NOT-APPEAR"));
    }

    /// A polled endpoint must not repeat itself, and must still name a second
    /// control when the operator trips one after fixing the first.
    #[test]
    fn a_control_is_reported_once_and_a_later_one_is_still_reported() {
        // Unique names so this test cannot collide with another that also
        // touches the process-wide set.
        let first: &'static str = "test_control_alpha";
        let second: &'static str = "test_control_beta";

        assert!(report_declined_control(first), "first sighting reports");
        assert!(!report_declined_control(first), "a repeat stays quiet");
        assert!(!report_declined_control(first));
        assert!(
            report_declined_control(second),
            "a different control must still be named"
        );
        assert!(!report_declined_control(second));
    }
}

// --- NEAR AI login enrolment (#836) ----------------------------------------
//
// The sibling ceremony to the wallet one above, and deliberately separate
// handlers rather than a mode flag: the two prove different things, and a flag
// would put the branch inside a path that currently has none.
//
// What the two share is the commons -- the same witness, issuer and audience,
// published by the same `capabilities` -- and the same ceremony table. What
// differs is what is proved: the wallet proves control of a NEAR account name
// by signing NEP-413; this proves possession of a NEAR AI session by presenting
// a token the server introspects. Neither proves the other.

/// How long a login ceremony lives. The same bound the wallet ceremony uses:
/// long enough for a person to complete a browser sign-in, short enough that a
/// captured ceremony handle is worth little.
const NEAR_AI_LOGIN_CEREMONY_TTL_SECONDS: i64 =
    trace_commons_server::account_onboarding::PROVISIONING_TTL_SECONDS;

/// Bound on the introspection call.
///
/// A hung dependency must not hang a request that is holding a pooled
/// connection, and this path is not on the submission hot path -- it runs once
/// per contributor per device -- so the bound can be short.
const NEAR_AI_INTROSPECTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Where `GET /me` lives. A compiled-in constant with no environment override:
/// there is nothing configurable for a host allowlist to protect, and making it
/// configurable would create the very thing the allowlist exists for.
const NEAR_AI_API_BASE_URL: &str = "https://cloud-api.near.ai/v1";

/// The nonce as it goes on the wire: **standard base64**, matching the wallet
/// ceremony on this same surface and matching what the client decodes.
///
/// Not hex, and the difference is not cosmetic. A 32-byte nonce in hex is 64
/// characters, every one of which is in the base64 alphabet, and 64 is a
/// multiple of four -- so a base64 decode of it **succeeds** and yields 48
/// bytes. The failure surfaces one step later as a length mismatch, under a
/// label that names nothing about encodings, and both halves' test suites stay
/// green because each tests its own convention.
///
/// The stored `nonce_hex` is hex and stays hex: that is internal, never
/// compared against anything the client sends, and hex is the readable choice
/// for a value that appears in a stored payload.
fn near_ai_nonce_wire(nonce: &[u8; 32]) -> String {
    base64::engine::general_purpose::STANDARD.encode(nonce)
}

/// The base URL introspection actually uses.
///
/// In a shipped build this is [`NEAR_AI_API_BASE_URL`] and nothing else: the
/// `cfg(not(test))` arm takes no state and has no branch, so there is no value
/// anyone could set. That is the point, and it is why the constant's own note
/// above still holds -- making the endpoint *configurable* would create the
/// thing the host allowlist exists to prevent, and this does not.
///
/// The `cfg(test)` arm exists because introspection is the last step of
/// `finish`, so before this nothing could drive the handler far enough to
/// reach `provision_near_ai_login` and the anchor row it writes.
#[cfg(not(test))]
fn introspection_base_url(_state: &AppState) -> &str {
    NEAR_AI_API_BASE_URL
}

#[cfg(test)]
fn introspection_base_url(state: &AppState) -> &str {
    state
        .near_ai_introspection_base_url
        .as_deref()
        .unwrap_or(NEAR_AI_API_BASE_URL)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NearAiStartRequest {
    device_public_key: String,
    code_challenge: String,
    code_challenge_method: String,
}

/// **No account identifier of any kind, and `deny_unknown_fields` is what
/// enforces that.**
///
/// The account is whatever the introspected token's subject resolves to. A body
/// carrying `account_id` is refused at the parse boundary rather than by a
/// check inside the handler, so the guarantee cannot be removed by deleting a
/// conditional -- accepting a client-asserted account is the sybil problem #836
/// exists to prevent.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NearAiFinishRequest {
    ceremony_id: String,
    code_verifier: String,
    device_public_key: String,
    device_signature: String,
    /// The NEAR AI access token, introspected and then dropped. Never logged,
    /// never stored, never returned.
    access_token: String,
}

pub(super) async fn near_ai_provision_start_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<NearAiStartRequest>, JsonRejection>,
) -> axum::response::Response {
    let began = std::time::Instant::now();
    let result = near_ai_start(state, headers, body).await;
    sleep_to_redeem_floor(began).await;
    result.unwrap_or_else(native_generic_deny)
}

async fn near_ai_start(
    state: Arc<AppState>,
    headers: HeaderMap,
    body: Result<Json<NearAiStartRequest>, JsonRejection>,
) -> Option<axum::response::Response> {
    // The login path's own readiness, not the wallet's: `published_witness`
    // refuses without the NEP-413 sign-in config and the wallet redirect
    // origin, neither of which this ceremony has or needs.
    if !near_ai_login_ready(&state) || limited(&headers, "near-ai-start") {
        return None;
    }
    let Json(body) = body.ok()?;
    if body.code_challenge_method != "S256" || !challenge_is_wellformed(&body.code_challenge) {
        return None;
    }
    let db = account_db(&state).ok()?;
    let device = device_key(&body.device_public_key)?;
    use rand::RngCore as _;
    let mut nonce = [0u8; 32];
    rand::rngs::OsRng.try_fill_bytes(&mut nonce).ok()?;
    let ceremony_id = generate_login_code();
    let expires_at = Utc::now()
        .timestamp()
        .checked_add(NEAR_AI_LOGIN_CEREMONY_TTL_SECONDS)?;
    let pending = trace_commons_server::account_onboarding::NearAiLoginPending {
        nonce_hex: hex::encode(nonce),
        code_challenge: body.code_challenge.clone(),
        device_public_key: base64::engine::general_purpose::STANDARD.encode(device),
        expires_at,
    };
    // The bytes the device must sign, from the shared protocol function. The
    // client computes the same preimage from the same function; nothing here
    // reconstructs the layout, because two implementations of one byte layout
    // agree in every test each side writes against itself and diverge in
    // production.
    let signing_bytes = trace_commons_protocol::onboarding::near_ai_provisioning_device_bytes(
        &nonce,
        &ceremony_id,
        &device,
        &body.code_challenge,
        expires_at,
    );
    db.store_near_ai_login_ceremony(&hash_secret(&ceremony_id), &pending, expires_at)
        .await
        .ok()?;
    Some(response(serde_json::json!({
        "ceremony_id": ceremony_id,
        "nonce": near_ai_nonce_wire(&nonce),
        "expires_at": expires_at,
        "device_signing_bytes": base64::engine::general_purpose::STANDARD.encode(&signing_bytes),
    })))
}

pub(super) async fn near_ai_provision_finish_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<NearAiFinishRequest>, JsonRejection>,
) -> axum::response::Response {
    let began = std::time::Instant::now();
    let result = near_ai_finish(state, headers, body).await;
    sleep_to_redeem_floor(began).await;
    result.unwrap_or_else(native_generic_deny)
}

async fn near_ai_finish(
    state: Arc<AppState>,
    headers: HeaderMap,
    body: Result<Json<NearAiFinishRequest>, JsonRejection>,
) -> Option<axum::response::Response> {
    if !near_ai_login_ready(&state) || limited(&headers, "near-ai-finish") {
        return None;
    }
    let Json(body) = body.ok()?;
    if body.ceremony_id.len() > 64
        || !verifier_is_wellformed(&body.code_verifier)
        || body.device_signature.len() > 128
        // Bounded before anything is done with it. A token past this length is
        // not one NEAR AI issued, and refusing early keeps an unbounded value
        // out of an outbound request.
        || body.access_token.is_empty()
        || body.access_token.len() > 8192
    {
        return None;
    }
    let db = account_db(&state).ok()?;
    let device = device_key(&body.device_public_key)?;
    let hash = hash_secret(&body.ceremony_id);
    if !ACCOUNT_RATE_LIMITER.check(&format!("near-ai-provision-ceremony:{hash}"), 5) {
        return None;
    }
    // Single use: the take deletes the row, so a replayed finish finds nothing.
    let pending = db.take_near_ai_login_ceremony(&hash).await.ok()??;
    // Everything below is checked against the ceremony, never against the
    // request. A finish naming a different device or a different challenge is
    // refused rather than believed.
    if !secret_eq(
        &pending.code_challenge,
        &challenge_for_verifier(&body.code_verifier),
    ) {
        return None;
    }
    if !secret_eq(
        &pending.device_public_key,
        &base64::engine::general_purpose::STANDARD.encode(device),
    ) {
        return None;
    }
    let nonce: [u8; 32] = hex::decode(&pending.nonce_hex).ok()?.try_into().ok()?;
    let signing_bytes = trace_commons_protocol::onboarding::near_ai_provisioning_device_bytes(
        &nonce,
        &body.ceremony_id,
        &device,
        &pending.code_challenge,
        pending.expires_at,
    );
    let signature: [u8; 64] = base64::engine::general_purpose::STANDARD
        .decode(&body.device_signature)
        .ok()?
        .try_into()
        .ok()?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &device)
        .verify(&signing_bytes, &signature)
        .ok()?;
    // Only now, with the device proven and the ceremony consumed, is the token
    // spent. Introspection is the last step because it is the only one that
    // leaves this machine, and a request that was going to be refused anyway
    // should not reach NEAR AI.
    let login = trace_commons_server::near_ai_login::introspect_login(
        introspection_base_url(&state),
        &secrecy::SecretString::from(body.access_token),
        NEAR_AI_INTROSPECTION_TIMEOUT,
    )
    .await
    .ok()?;
    let identity = state.near_account_identity.as_ref()?;
    let secret = generate_session_secret();
    let token_hash = hash_secret(&secret);
    let provisioned = db
        .provision_near_ai_login(
            &login,
            &device,
            trace_commons_server::db::NewSession {
                token_hash: &token_hash,
                client_kind: NATIVE_SESSION_CLIENT_KIND,
                expires_at: Utc::now() + Duration::hours(NATIVE_SESSION_TTL_HOURS),
            },
            identity,
        )
        .await
        .ok()?;
    Some(response(serde_json::json!({
        "access_token":native_token_value(&provisioned.tenant_id,&secret),"token_type":"Bearer",
        "expires_in_secs":NATIVE_SESSION_TTL_HOURS*3600,"account_id":provisioned.account_id,
        "tenant_id":provisioned.tenant_id,"device_key_id":provisioned.device_key_id,"anchor_hash":provisioned.anchor_hash
    })))
}

#[cfg(test)]
mod near_ai_login_tests {
    use super::*;

    /// **A client-asserted account is refused before the handler sees it.**
    ///
    /// The account on this path is whatever the introspected token's subject
    /// resolves to. A request that names one is the sybil problem #836 exists
    /// to prevent -- an attacker runs a modified client and asserts a fresh
    /// account per submission -- so the refusal must not be a conditional
    /// someone can delete. `deny_unknown_fields` makes it a parse failure: the
    /// body never becomes a `NearAiFinishRequest` at all.
    #[test]
    fn a_finish_body_naming_an_account_is_refused_at_the_parse_boundary() {
        let ok = serde_json::json!({
            "ceremony_id": "c",
            "code_verifier": "v",
            "device_public_key": "d",
            "device_signature": "s",
            "access_token": "t",
        });
        serde_json::from_value::<NearAiFinishRequest>(ok.clone())
            .expect("the shape without an account parses");

        for smuggled in ["account_id", "near_account_id", "subject", "tenant_id"] {
            let mut body = ok.clone();
            body[smuggled] = serde_json::json!("attacker-chosen");
            assert!(
                serde_json::from_value::<NearAiFinishRequest>(body).is_err(),
                "{smuggled} reached the handler instead of being refused"
            );
        }
    }

    /// The same guarantee on the other verb, and the reason it matters there
    /// too: start decides what the ceremony commits to, so an extra field it
    /// silently ignored would be one a client believed had been agreed.
    #[test]
    fn a_start_body_with_anything_extra_is_refused() {
        let ok = serde_json::json!({
            "device_public_key": "d",
            "code_challenge": "c",
            "code_challenge_method": "S256",
        });
        serde_json::from_value::<NearAiStartRequest>(ok.clone()).expect("the agreed shape parses");
        for smuggled in ["account_id", "nonce", "expires_at"] {
            let mut body = ok.clone();
            body[smuggled] = serde_json::json!("attacker-chosen");
            assert!(
                serde_json::from_value::<NearAiStartRequest>(body).is_err(),
                "{smuggled} was accepted at start"
            );
        }
    }

    /// The wire nonce decodes, **the way the client decodes it**, to the 32
    /// bytes the preimage needs.
    ///
    /// This is the test that was missing. Both halves were green while the
    /// wire disagreed, because each tested its own convention: the server
    /// checked that it encoded a nonce and the client checked that it decoded
    /// one, and nothing checked that the two were the same encoding.
    ///
    /// It exercises the production encoder rather than a copy of it, so a
    /// change there fails here.
    #[test]
    fn the_wire_nonce_decodes_to_thirty_two_bytes_as_the_client_reads_it() {
        let nonce: [u8; 32] = std::array::from_fn(|i| i as u8);
        let wire = near_ai_nonce_wire(&nonce);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&wire)
            .expect("the client's decode succeeds");
        assert_eq!(decoded, nonce, "the client reconstructs the nonce we sent");

        // Why this test exists, kept as an assertion rather than a comment:
        // hex is not merely a different encoding here, it is one that decodes
        // *successfully* as base64 and gives the wrong length. A regression to
        // hex would not fail at the decode, which is what made it survive both
        // suites.
        let as_hex = hex::encode(nonce);
        let hex_through_base64 = base64::engine::general_purpose::STANDARD
            .decode(&as_hex)
            .expect("hex is valid base64, which is the whole trap");
        assert_eq!(
            hex_through_base64.len(),
            48,
            "the trap has changed shape; re-derive what a hex nonce decodes to"
        );
        assert_ne!(wire, as_hex, "the wire field regressed to hex");
    }

    /// One surface, one convention. The wallet ceremony on this same handler
    /// file sends its nonce as standard base64, and a contributor's client
    /// should not have to know which ceremony it is talking to in order to
    /// read a field of the same name.
    #[test]
    fn both_ceremonies_encode_a_nonce_the_same_way() {
        let nonce = [7u8; 32];
        assert_eq!(
            near_ai_nonce_wire(&nonce),
            base64::engine::general_purpose::STANDARD.encode(nonce),
            "the login ceremony diverged from the wallet ceremony's encoding"
        );
    }

    /// The two ceremonies must not share a device-proof preimage, or a
    /// signature captured from one could be presented as the other. Asserted
    /// through the shared protocol function rather than against a description
    /// of it -- this is the function both halves call.
    #[test]
    fn the_login_device_proof_is_not_the_wallet_one() {
        let nonce = [3u8; 32];
        let device = [4u8; 32];
        let login = trace_commons_protocol::onboarding::near_ai_provisioning_device_bytes(
            &nonce,
            "ceremony",
            &device,
            "challenge",
            99,
        );
        let wallet = trace_commons_protocol::onboarding::near_provisioning_device_bytes(
            &nonce,
            "ceremony",
            "challenge",
            &device,
            None,
        );
        assert_ne!(login, wallet);
        // And the ceremony's own fields are covered: change any one and the
        // bytes move, so a signature is not transferable between ceremonies.
        for (n, c, d, ch, e) in [
            ([9u8; 32], "ceremony", device, "challenge", 99),
            (nonce, "other", device, "challenge", 99),
            (nonce, "ceremony", [9u8; 32], "challenge", 99),
            (nonce, "ceremony", device, "other", 99),
            (nonce, "ceremony", device, "challenge", 100),
        ] {
            assert_ne!(
                login,
                trace_commons_protocol::onboarding::near_ai_provisioning_device_bytes(
                    &n, c, &d, ch, e
                ),
            );
        }
    }
}
