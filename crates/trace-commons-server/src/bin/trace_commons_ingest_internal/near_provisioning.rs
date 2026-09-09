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

    /// Every label, for the tests that hold the label-only rule.
    #[cfg(test)]
    pub(super) const ALL: [&str; 12] = [
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
/// Deliberately the same gates in the same order as before, decision for
/// decision: this exists to make an existing refusal legible, and a
/// configuration that was accepted before must still be accepted.
/// `naming_a_control_does_not_change_which_configurations_are_accepted` pins
/// that against the original predicates.
fn published_witness_named(state: &AppState) -> Result<PublishedWitness, &'static str> {
    if !state.near_provisioning_enabled {
        return Err(control::PROVISIONING_ENABLED);
    }
    if !state.near_provisioning_admission_ready {
        return Err(control::ADMISSION_READY);
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
    let raw = std::env::var("TRACE_COMMONS_NEAR_PROVISIONING_WITNESS_JSON")
        .map_err(|_| control::WITNESS_JSON_ABSENT)?;
    let witness: PublishedWitness =
        serde_json::from_str(&raw).map_err(|_| control::WITNESS_JSON_MALFORMED)?;
    validate_witness_named(&witness)?;
    published_issuer().ok_or(control::ISSUER)?;
    Ok(witness)
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
pub(super) async fn capabilities(State(state): State<Arc<AppState>>) -> axum::response::Response {
    match published_witness(&state) {
        Some(witness) => {
            let Some((issuer_url, audience)) = published_issuer() else {
                return response(serde_json::json!({"ready":false,"funding_available":false}));
            };
            let network = account_near_config(&state).ok().map(|c| c.network.clone());
            response(
                serde_json::json!({"ready":true,"network":network,"witness":witness,"issuer_url":issuer_url,"audience":audience,"inference_receipt_endpoint":published_receipt_endpoint(),"funding_available":false}),
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
