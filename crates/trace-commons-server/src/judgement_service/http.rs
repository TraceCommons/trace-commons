// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The judgement service's HTTP surface (spec section 3): two routes, and
//! nothing else.
//!
//! - `GET /v1/attestation?nonce=<64 hex>` -- `{"quote_hex",
//!   "signing_address"}`, the quote binding `tcjudge1 | address | nonce`.
//! - `POST /v1/judgement` -- a judgement claim in `Authorization`, the nonce
//!   the caller verified the quote with in `x-trace-judgement-nonce`, and a
//!   digest; labels out, signed over that nonce.
//!
//! No health or metrics route that reports state. A liveness probe is
//! `GET /v1/attestation` with a fresh nonce, which proves more.
//!
//! # What bounds the work
//!
//! `POST /v1/judgement` runs under [`JudgementLoadBound`]: a fixed number in
//! flight, each under a deadline, and an immediate `judgement_saturated`
//! (with `Retry-After: 30`) past it. Not a queue: a queue in front of a
//! bounded worker turns load into memory and tells the caller nothing. The
//! attestation route is outside the bound, so load on the judgement route
//! cannot make the service unpinnable.
//!
//! # Refusals
//!
//! `POST /v1/judgement` answers every refusal from the closed set in
//! `trace_commons_protocol::insights_judgement`, with the status that label
//! is always sent with and `reset_at` on exactly the two budget refusals.
//! The attestation route has two labels of its own: a malformed nonce is
//! `judgement_request_malformed`, and a quote the enclave could not produce
//! is [`JUDGEMENT_QUOTE_UNAVAILABLE`]. That one is outside the closed set
//! because the closed set is the judgement route's; a daemon that cannot
//! verify a quote sends nothing, whatever the label.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use tokio::sync::Semaphore;
use trace_commons_protocol::insights_judgement::{
    JUDGEMENT_NONCE_HEADER, JUDGEMENT_REQUEST_MAX_BYTES, JUDGEMENT_SATURATED_RETRY_AFTER_SECS,
    JudgementNonce, JudgementRefusal, JudgementRefusalBody, JudgementResponse,
    judgement_budget_reset_at, judgement_signing_bytes, parse_judgement_request,
};

use super::enclave::JudgementEnclave;
use super::service::JudgementService;
use crate::witness_service::enclave::EnclaveError;

/// The attestation route's refusal when the enclave produced no bound
/// quote. 503, not retryable on a timer by a daemon: see the module docs.
pub const JUDGEMENT_QUOTE_UNAVAILABLE: &str = "judgement_quote_unavailable";

/// What the routes need from the enclave. A trait so the router is tested
/// without a dstack agent; production holds a [`JudgementEnclave`].
#[async_trait]
pub trait JudgementIdentity: Send + Sync {
    /// `0x` + 40 lowercase hex.
    fn signing_address(&self) -> &str;
    /// A quote binding this service's address and `nonce` under `tcjudge1`.
    async fn attest(&self, nonce: &JudgementNonce) -> Result<Vec<u8>, EnclaveError>;
    /// EIP-191 over `message`, `0x` + 130 lowercase hex.
    fn sign_eip191(&self, message: &[u8]) -> Result<String, EnclaveError>;
}

#[async_trait]
impl JudgementIdentity for JudgementEnclave {
    fn signing_address(&self) -> &str {
        JudgementEnclave::signing_address(self)
    }

    async fn attest(&self, nonce: &JudgementNonce) -> Result<Vec<u8>, EnclaveError> {
        JudgementEnclave::attest(self, nonce).await
    }

    fn sign_eip191(&self, message: &[u8]) -> Result<String, EnclaveError> {
        JudgementEnclave::sign_eip191(self, message)
    }
}

/// Unix seconds now. A parameter of the router so tests run at a fixed
/// instant (claims and months are time-dependent).
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The wall clock.
pub fn system_clock() -> Clock {
    Arc::new(|| chrono::Utc::now().timestamp())
}

/// What the handlers share.
#[derive(Clone)]
pub struct JudgementHttpState {
    service: Arc<JudgementService>,
    identity: Arc<dyn JudgementIdentity>,
    clock: Clock,
}

impl JudgementHttpState {
    pub fn new(
        service: Arc<JudgementService>,
        identity: Arc<dyn JudgementIdentity>,
        clock: Clock,
    ) -> Self {
        Self {
            service,
            identity,
            clock,
        }
    }
}

/// How many `POST /v1/judgement` requests may run at once, and for how
/// long each may run.
#[derive(Clone)]
pub struct JudgementLoadBound {
    permits: Arc<Semaphore>,
    request_timeout: Duration,
}

impl JudgementLoadBound {
    /// Zero slots or a zero deadline is refused rather than guessed at.
    pub fn new(
        max_concurrent_requests: usize,
        request_timeout: Duration,
    ) -> Result<Self, &'static str> {
        if max_concurrent_requests == 0 {
            return Err("judgement concurrency limit must be positive");
        }
        if request_timeout.is_zero() {
            return Err("judgement request timeout must be positive");
        }
        Ok(Self {
            permits: Arc::new(Semaphore::new(max_concurrent_requests)),
            request_timeout,
        })
    }
}

/// The router: two routes. Built here so the route wiring is under test.
pub fn judgement_router(state: JudgementHttpState, load: JudgementLoadBound) -> Router {
    Router::new()
        .route("/v1/judgement", post(judgement_handler))
        // `Router::layer` applies to the routes added above it: the bound
        // covers the judgement route and not the attestation route.
        .layer(middleware::from_fn_with_state(load, bound_judgement_load))
        .route("/v1/attestation", get(attestation_handler))
        .with_state(state)
}

/// A refusal from the closed set, with its status, `reset_at` where the
/// label carries one, and `Retry-After` on `judgement_saturated`.
fn refusal_response(refusal: JudgementRefusal, now: i64) -> Response {
    let reset_at = if refusal.carries_reset_at() {
        judgement_budget_reset_at(now)
    } else {
        None
    };
    let body = match JudgementRefusalBody::new(refusal, reset_at) {
        Ok(body) => body.to_json(),
        // `reset_at` out of chrono's range: still the label, never a guess.
        Err(_) => serde_json::json!({ "error": refusal.label() }).to_string(),
    };
    let status =
        StatusCode::from_u16(refusal.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = (
        status,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        body,
    )
        .into_response();
    if refusal == JudgementRefusal::Saturated {
        response.headers_mut().insert(
            header::RETRY_AFTER,
            HeaderValue::from(JUDGEMENT_SATURATED_RETRY_AFTER_SECS),
        );
    }
    response
}

/// Acquire or refuse, then run under the deadline.
async fn bound_judgement_load(
    State(load): State<JudgementLoadBound>,
    request: Request,
    next: Next,
) -> Response {
    let now = || chrono::Utc::now().timestamp();
    let Ok(_permit) = Arc::clone(&load.permits).try_acquire_owned() else {
        return refusal_response(JudgementRefusal::Saturated, now());
    };
    match tokio::time::timeout(load.request_timeout, next.run(request)).await {
        Ok(response) => response,
        // Dropping the future cancels the model call under it; a
        // reservation already taken stands as the charge (spec section 6).
        Err(_elapsed) => refusal_response(JudgementRefusal::Timeout, now()),
    }
}

/// The single value of `name`, or `None` when absent, repeated, or not
/// visible ASCII.
fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    value.to_str().ok()
}

/// `POST /v1/judgement`.
async fn judgement_handler(State(state): State<JudgementHttpState>, request: Request) -> Response {
    let now = (state.clock)();
    match judge(&state, request, now).await {
        Ok(response) => response,
        Err(refusal) => refusal_response(refusal, now),
    }
}

async fn judge(
    state: &JudgementHttpState,
    request: Request,
    now: i64,
) -> Result<Response, JudgementRefusal> {
    let (parts, body) = request.into_parts();
    // Stops reading at the cap rather than buffering what was sent.
    let body = axum::body::to_bytes(body, JUDGEMENT_REQUEST_MAX_BYTES)
        .await
        .map_err(|_| JudgementRefusal::DigestTooLarge)?;
    let nonce = single_header(&parts.headers, JUDGEMENT_NONCE_HEADER)
        .and_then(|value| JudgementNonce::from_hex(value).ok())
        .ok_or(JudgementRefusal::RequestMalformed)?;
    let bearer = single_header(&parts.headers, header::AUTHORIZATION.as_str())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| !token.is_empty())
        .ok_or(JudgementRefusal::CredentialRefused)?;
    let request = parse_judgement_request(&body)?;

    let judged = state.service.judge(bearer, &request, now).await?;

    let signing_bytes =
        judgement_signing_bytes(&nonce, &judged.digest_sha256, &judged.results, now);
    // A service that cannot sign does not answer. In-process secp256k1
    // signing does not fail in practice; if it did, this is not a refusal
    // the daemon should treat as anything but "try later".
    let signature = state
        .identity
        .sign_eip191(&signing_bytes)
        .map_err(|_| JudgementRefusal::Saturated)?;
    let response = JudgementResponse::new(
        judged.digest_sha256,
        judged.results,
        judged.spend,
        now,
        signature,
    )
    .map_err(|_| JudgementRefusal::Saturated)?;
    Ok((
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        response.to_json(),
    )
        .into_response())
}

#[derive(Deserialize)]
struct AttestationQuery {
    nonce: Option<String>,
}

/// `GET /v1/attestation?nonce=<64 hex>`.
async fn attestation_handler(
    State(state): State<JudgementHttpState>,
    Query(query): Query<AttestationQuery>,
) -> Response {
    let now = (state.clock)();
    let Some(nonce) = query
        .nonce
        .as_deref()
        .and_then(|value| JudgementNonce::from_hex(value).ok())
    else {
        return refusal_response(JudgementRefusal::RequestMalformed, now);
    };
    match state.identity.attest(&nonce).await {
        Ok(quote) => axum::Json(serde_json::json!({
            "quote_hex": hex::encode(quote),
            "signing_address": state.identity.signing_address(),
        }))
        .into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            serde_json::json!({ "error": JUDGEMENT_QUOTE_UNAVAILABLE }).to_string(),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::judgement_service::catalog::{CatalogEntry, TokenPrices};
    use crate::judgement_service::claim::tests::{NOW, TestIssuer};
    use crate::judgement_service::enclave::judgement_report_data;
    use crate::judgement_service::ledger::LedgerState;
    use crate::judgement_service::limiter::tests::MemoryStore;
    use crate::judgement_service::limiter::{Limiter, LimiterCaps, LimiterKeys};
    use crate::judgement_service::mock::{
        MOCK_MODEL_ID, MockFault, MockJudgementModel, MockReceiptSigner,
    };
    use crate::judgement_service::production::ProductionCheck;
    use crate::judgement_service::receipt_check::JudgementCaller;
    use crate::judgement_service::service::tests::LOG_LINE;
    use crate::witness_service::enclave::{address_of, sign_eip191_with};
    use axum::body::Body;
    use axum::http::{Method, Request as HttpRequest};
    use k256::ecdsa::SigningKey;
    use std::collections::BTreeMap;
    use tower::ServiceExt;
    use trace_commons_attestation::receipt::recover_eip191_signer;
    use trace_commons_protocol::insights_judgement::{
        JudgementKind, JudgementRequest, parse_judgement_refusal, parse_judgement_response,
    };

    const NONCE_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    struct StubIdentity {
        key: SigningKey,
        address: String,
        quote_fails: bool,
    }

    impl StubIdentity {
        fn new() -> Self {
            let key = SigningKey::from_slice(&[7u8; 32]).expect("scalar");
            let address = address_of(&key);
            Self {
                key,
                address,
                quote_fails: false,
            }
        }

        fn address_bytes(&self) -> [u8; 20] {
            hex::decode(&self.address[2..])
                .expect("hex")
                .try_into()
                .expect("20")
        }
    }

    #[async_trait]
    impl JudgementIdentity for StubIdentity {
        fn signing_address(&self) -> &str {
            &self.address
        }
        async fn attest(&self, nonce: &JudgementNonce) -> Result<Vec<u8>, EnclaveError> {
            if self.quote_fails {
                return Err(EnclaveError::Transport);
            }
            // Not a quote: the report data alone, which is what the route
            // must carry through untouched.
            Ok(judgement_report_data(&self.address_bytes(), nonce.as_bytes()).to_vec())
        }
        fn sign_eip191(&self, message: &[u8]) -> Result<String, EnclaveError> {
            sign_eip191_with(&self.key, message).map_err(|_| EnclaveError::SigningFailed)
        }
    }

    struct Harness {
        issuer: TestIssuer,
        model: Arc<MockJudgementModel>,
        identity: Arc<StubIdentity>,
        router: Router,
        load: JudgementLoadBound,
    }

    fn harness_with(caps: LimiterCaps, identity: StubIdentity) -> Harness {
        let issuer = TestIssuer::new();
        let model = Arc::new(MockJudgementModel::new(MockReceiptSigner::generate()));
        let mut pins = BTreeMap::new();
        pins.insert(MOCK_MODEL_ID.to_string(), vec![model.signer_public_hex()]);
        let check = ProductionCheck::for_tests(&[MOCK_MODEL_ID], pins);
        let mut entries = BTreeMap::new();
        entries.insert(
            MOCK_MODEL_ID.to_string(),
            CatalogEntry {
                model_id: MOCK_MODEL_ID.to_string(),
                verifiable: true,
                attestation_supported: true,
                prices: Some(TokenPrices {
                    input_nanousd: 40,
                    output_nanousd: 1350,
                }),
            },
        );
        check
            .catalog()
            .write()
            .expect("lock")
            .record_refresh(NOW, entries);
        let limiter = Arc::new(Limiter::new(
            caps,
            LedgerState::default(),
            Arc::new(MemoryStore::default()),
        ));
        let caller = JudgementCaller::new(model.clone(), Arc::new(check), limiter);
        let service = Arc::new(JudgementService::new(
            issuer.keys(),
            LimiterKeys::from_root(&[3u8; 32]),
            caller,
        ));
        let identity = Arc::new(identity);
        let load = JudgementLoadBound::new(4, Duration::from_secs(30)).expect("bound");
        let state = JudgementHttpState::new(service, identity.clone(), Arc::new(|| NOW));
        let router = judgement_router(state, load.clone());
        Harness {
            issuer,
            model,
            identity,
            router,
            load,
        }
    }

    fn harness() -> Harness {
        harness_with(
            LimiterCaps::from_usd(5, 100).expect("caps"),
            StubIdentity::new(),
        )
    }

    fn digest() -> String {
        serde_json::json!({
            "schema_version": 1,
            "events": [{"prompt_ix": 0, "kind": "interrupt"}],
            "excerpts": [],
        })
        .to_string()
    }

    fn request_body() -> String {
        JudgementRequest::new(
            digest(),
            vec![
                JudgementKind::Task,
                JudgementKind::EventKind,
                JudgementKind::Finished,
            ],
        )
        .expect("request")
        .to_json()
    }

    fn post(body: String, claim: Option<&str>, nonce: Option<&str>) -> HttpRequest<Body> {
        let mut request = HttpRequest::builder()
            .method(Method::POST)
            .uri("/v1/judgement")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(claim) = claim {
            request = request.header(header::AUTHORIZATION, format!("Bearer {claim}"));
        }
        if let Some(nonce) = nonce {
            request = request.header(JUDGEMENT_NONCE_HEADER, nonce);
        }
        request.body(Body::from(body)).expect("request")
    }

    async fn send(router: &Router, request: HttpRequest<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let response = router.clone().oneshot(request).await.expect("served");
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body")
            .to_vec();
        (status, headers, body)
    }

    fn refusal_of(status: StatusCode, body: &[u8]) -> JudgementRefusalBody {
        parse_judgement_refusal(status.as_u16(), body)
            .unwrap_or_else(|| panic!("not a closed-set refusal: {status} {body:?}"))
    }

    #[tokio::test]
    async fn a_judgement_is_signed_over_the_callers_nonce() {
        // The request log line's callsite; see `service::tests::LOG_LINE`.
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let claim = h.issuer.valid_claim();
        let (status, headers, body) = send(
            &h.router,
            post(request_body(), Some(&claim), Some(NONCE_HEX)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        assert_eq!(headers[header::CONTENT_TYPE], "application/json");
        let response = parse_judgement_response(&body).expect("a closed-shape response");
        assert_eq!(response.signed_at(), NOW);
        assert_eq!(response.results().len(), 3);
        assert_eq!(h.model.calls(), 3);

        let nonce = JudgementNonce::from_hex(NONCE_HEX).expect("nonce");
        let signer = recover_eip191_signer(&response.signing_bytes(&nonce), response.signature())
            .expect("recovers");
        assert_eq!(signer, h.identity.address_bytes());

        // Over another nonce the same signature names someone else: the
        // answer is bound to the exchange the caller verified.
        let other = JudgementNonce::from_bytes([0xee; 32]);
        let elsewhere =
            recover_eip191_signer(&response.signing_bytes(&other), response.signature());
        assert_ne!(elsewhere.ok(), Some(h.identity.address_bytes()));
    }

    #[tokio::test]
    async fn a_missing_or_malformed_nonce_is_refused_before_any_call() {
        // The request log line's callsite; see `service::tests::LOG_LINE`.
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let claim = h.issuer.valid_claim();
        for nonce in [None, Some("abc"), Some(&NONCE_HEX.to_uppercase()[..])] {
            let (status, _, body) =
                send(&h.router, post(request_body(), Some(&claim), nonce)).await;
            assert_eq!(
                refusal_of(status, &body).refusal(),
                JudgementRefusal::RequestMalformed
            );
        }
        // Two nonce headers are not one nonce.
        let mut twice = post(request_body(), Some(&claim), Some(NONCE_HEX));
        twice
            .headers_mut()
            .append(JUDGEMENT_NONCE_HEADER, HeaderValue::from_static(NONCE_HEX));
        let (status, _, body) = send(&h.router, twice).await;
        assert_eq!(
            refusal_of(status, &body).refusal(),
            JudgementRefusal::RequestMalformed
        );
        assert_eq!(h.model.calls(), 0);
    }

    #[tokio::test]
    async fn a_missing_or_foreign_credential_is_refused() {
        // The request log line's callsite; see `service::tests::LOG_LINE`.
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let (status, _, body) = send(&h.router, post(request_body(), None, Some(NONCE_HEX))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            refusal_of(status, &body).refusal(),
            JudgementRefusal::CredentialRefused
        );
        let stranger = TestIssuer::new().valid_claim();
        let (status, _, body) = send(
            &h.router,
            post(request_body(), Some(&stranger), Some(NONCE_HEX)),
        )
        .await;
        assert_eq!(
            refusal_of(status, &body).refusal(),
            JudgementRefusal::CredentialRefused
        );
        assert_eq!(h.model.calls(), 0);
    }

    #[tokio::test]
    async fn an_oversized_body_is_refused_by_name() {
        // The request log line's callsite; see `service::tests::LOG_LINE`.
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let claim = h.issuer.valid_claim();
        let oversized = "x".repeat(JUDGEMENT_REQUEST_MAX_BYTES + 1);
        let (status, _, body) =
            send(&h.router, post(oversized, Some(&claim), Some(NONCE_HEX))).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            refusal_of(status, &body).refusal(),
            JudgementRefusal::DigestTooLarge
        );
    }

    #[tokio::test]
    async fn another_wire_version_is_refused_by_name() {
        // The request log line's callsite; see `service::tests::LOG_LINE`.
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let claim = h.issuer.valid_claim();
        let v2 = serde_json::json!({"wire_version": 2, "digest": digest(), "judgements": ["task"]})
            .to_string();
        let (status, _, body) = send(&h.router, post(v2, Some(&claim), Some(NONCE_HEX))).await;
        assert_eq!(
            refusal_of(status, &body).refusal(),
            JudgementRefusal::WireVersionUnsupported
        );
    }

    #[tokio::test]
    async fn a_budget_refusal_carries_the_next_month() {
        // The request log line's callsite; see `service::tests::LOG_LINE`.
        let _serial = LOG_LINE.lock().await;
        let h = harness_with(
            LimiterCaps::from_usd(0, 100).expect("caps"),
            StubIdentity::new(),
        );
        let claim = h.issuer.valid_claim();
        let (status, _, body) = send(
            &h.router,
            post(request_body(), Some(&claim), Some(NONCE_HEX)),
        )
        .await;
        let refusal = refusal_of(status, &body);
        assert_eq!(refusal.refusal(), JudgementRefusal::AccountBudgetExhausted);
        assert_eq!(refusal.reset_at(), judgement_budget_reset_at(NOW));
        assert_eq!(h.model.calls(), 0);
    }

    #[tokio::test]
    async fn a_model_timeout_is_504_and_not_capacity() {
        // The request log line's callsite; see `service::tests::LOG_LINE`.
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        h.model.set_fault(Some(MockFault::Timeout));
        let claim = h.issuer.valid_claim();
        let (status, headers, body) = send(
            &h.router,
            post(request_body(), Some(&claim), Some(NONCE_HEX)),
        )
        .await;
        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(
            refusal_of(status, &body).refusal(),
            JudgementRefusal::Timeout
        );
        assert!(headers.get(header::RETRY_AFTER).is_none());
    }

    #[tokio::test]
    async fn a_full_service_refuses_with_retry_after_and_still_attests() {
        // The request log line's callsite; see `service::tests::LOG_LINE`.
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let _held = Arc::clone(&h.load.permits)
            .try_acquire_many_owned(4)
            .expect("all four slots");
        let claim = h.issuer.valid_claim();
        let (status, headers, body) = send(
            &h.router,
            post(request_body(), Some(&claim), Some(NONCE_HEX)),
        )
        .await;
        assert_eq!(
            refusal_of(status, &body).refusal(),
            JudgementRefusal::Saturated
        );
        assert_eq!(
            headers[header::RETRY_AFTER],
            JUDGEMENT_SATURATED_RETRY_AFTER_SECS.to_string()
        );
        assert_eq!(h.model.calls(), 0);
        // Load on the judgement route cannot make the service unpinnable.
        let (status, _, _) = send(&h.router, get_attestation(Some(NONCE_HEX))).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_request_past_its_deadline_is_refused_and_releases_its_slot() {
        let load = JudgementLoadBound::new(1, Duration::from_millis(20)).expect("bound");
        let router = Router::new()
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    "late"
                }),
            )
            .layer(middleware::from_fn_with_state(
                load.clone(),
                bound_judgement_load,
            ));
        let request = || {
            HttpRequest::builder()
                .uri("/slow")
                .body(Body::empty())
                .expect("request")
        };
        for _ in 0..2 {
            let (status, _, body) = send(&router, request()).await;
            assert_eq!(
                refusal_of(status, &body).refusal(),
                JudgementRefusal::Timeout
            );
        }
        assert_eq!(load.permits.available_permits(), 1);
    }

    #[test]
    fn a_bound_of_zero_is_refused() {
        assert!(JudgementLoadBound::new(0, Duration::from_secs(1)).is_err());
        assert!(JudgementLoadBound::new(1, Duration::ZERO).is_err());
    }

    fn get_attestation(nonce: Option<&str>) -> HttpRequest<Body> {
        let uri = match nonce {
            Some(nonce) => format!("/v1/attestation?nonce={nonce}"),
            None => "/v1/attestation".to_string(),
        };
        HttpRequest::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("request")
    }

    #[tokio::test]
    async fn the_attestation_route_serves_a_quote_bound_to_the_nonce() {
        let h = harness();
        let (status, _, body) = send(&h.router, get_attestation(Some(NONCE_HEX))).await;
        assert_eq!(status, StatusCode::OK);
        let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(value["signing_address"], h.identity.address);
        let nonce = JudgementNonce::from_hex(NONCE_HEX).expect("nonce");
        assert_eq!(
            value["quote_hex"],
            hex::encode(judgement_report_data(
                &h.identity.address_bytes(),
                nonce.as_bytes()
            ))
        );
        assert_eq!(value.as_object().expect("object").len(), 2);
    }

    #[tokio::test]
    async fn the_attestation_route_refuses_a_bad_nonce_and_a_missing_quote() {
        let h = harness();
        for nonce in [None, Some("00"), Some("zz")] {
            let (status, _, body) = send(&h.router, get_attestation(nonce)).await;
            assert_eq!(
                refusal_of(status, &body).refusal(),
                JudgementRefusal::RequestMalformed
            );
        }
        let mut identity = StubIdentity::new();
        identity.quote_fails = true;
        let h = harness_with(LimiterCaps::from_usd(5, 100).expect("caps"), identity);
        let (status, _, body) = send(&h.router, get_attestation(Some(NONCE_HEX))).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).expect("json"),
            serde_json::json!({"error": JUDGEMENT_QUOTE_UNAVAILABLE})
        );
    }

    #[tokio::test]
    async fn no_route_other_than_the_two_exists() {
        let h = harness();
        for (method, uri) in [
            (Method::GET, "/"),
            (Method::GET, "/health"),
            (Method::GET, "/metrics"),
            (Method::GET, "/v1/judgement"),
            (Method::POST, "/v1/attestation"),
            (Method::POST, "/v1/witness"),
        ] {
            let request = HttpRequest::builder()
                .method(method.clone())
                .uri(uri)
                .body(Body::empty())
                .expect("request");
            let (status, _, _) = send(&h.router, request).await;
            assert!(
                status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED,
                "{method} {uri} answered {status}"
            );
        }
    }
}
