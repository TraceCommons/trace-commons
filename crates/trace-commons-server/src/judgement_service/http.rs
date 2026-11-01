// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The judgement service's HTTP surface (spec section 3): two routes, and
//! nothing else.
//!
//! - `GET /v1/attestation?nonce=<64 hex>` -- `{"quote_hex",
//!   "seal_public_key", "signing_address"}`, the quote binding
//!   `tcjudge1 | SHA-256(label || seal_pk || signing_address)[0..24] | nonce`.
//! - `POST /v1/judgement` -- `application/octet-stream`, a request sealed
//!   to this boot's sealing key. The claim and the request nonce travel
//!   inside the seal; there is no `Authorization` and no nonce header. A
//!   `200` is the signed response sealed under that request's response key.
//!   Refusals are plaintext `{"error": "<label>"}`: they carry no data.
//!
//! No health or metrics route that reports state. A liveness probe is
//! `GET /v1/attestation` with a fresh nonce, which proves more.
//!
//! # What bounds the work
//!
//! `POST /v1/judgement` runs in the order of spec section 3, "The service's
//! check order", so an unauthenticated caller cannot hold a judgement slot
//! (#1531 review point 4):
//!
//! 1. read the body under its own bound ([`JudgementLoadBound`]: 32 reads
//!    at once, each under a 10-second timeout and the
//!    [`JUDGEMENT_SEALED_REQUEST_MAX_BYTES`] cap). A full read bound is
//!    `judgement_saturated`; a read past its timeout is answered `408` with
//!    `Connection: close` and no label, because no request arrived;
//! 2. open the seal (`judgement_seal_stale` if it does not open);
//! 3. parse, verify the claim and record the request nonce
//!    ([`JudgementService::judge`]);
//! 4. only then `try_acquire` a judgement permit (8 by default), or
//!    `judgement_saturated` with `Retry-After: 30`. Not a queue: a queue in
//!    front of a bounded worker turns load into memory and tells the caller
//!    nothing;
//! 5. sign, then seal the response. Spend is already settled by then, so a
//!    failure here is `judgement_service_fault` (500, never `Retry-After`):
//!    a retry would be charged again (#1531 review point 6).
//!
//! The whole request runs under one deadline, `judgement_timeout` past it.
//!
//! The attestation route has a bound of its own ([`AttestationBound`]: 4
//! quote calls at once, each under a 10-second timeout; either limit is
//! `judgement_quote_unavailable`, #1531 review point 7). The two bounds are
//! separate, so load on the judgement route cannot make the service
//! unpinnable, and a flood of quote requests cannot starve judgements.
//!
//! # Refusals
//!
//! Every refusal is from the closed set in
//! `trace_commons_protocol::insights_judgement`, with the status that label
//! is always sent with, `reset_at` on exactly the two budget refusals, and
//! `Retry-After` on `judgement_saturated` and `judgement_quote_unavailable`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use tokio::sync::Semaphore;
use trace_commons_attestation::judgement_seal::{ResponseSealer, SealError};
use trace_commons_attestation::report_data::SEAL_PUBLIC_KEY_LEN;
use trace_commons_protocol::insights_judgement::{
    JUDGEMENT_SATURATED_RETRY_AFTER_SECS, JUDGEMENT_SEALED_REQUEST_MAX_BYTES,
    JUDGEMENT_WIRE_VERSION, JudgementAttestationResponse, JudgementNonce, JudgementRefusal,
    JudgementRefusalBody, JudgementResponse, judgement_budget_reset_at, judgement_signing_bytes,
};

use super::enclave::JudgementEnclave;
use super::service::JudgementService;
use crate::witness_service::enclave::EnclaveError;

/// How many request bodies may be read at once.
pub const DEFAULT_MAX_CONCURRENT_BODY_READS: usize = 32;
/// How long one request body may take to arrive.
pub const DEFAULT_BODY_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// How many quote calls may run at once.
pub const DEFAULT_MAX_CONCURRENT_QUOTES: usize = 4;
/// How long one quote call may take.
pub const DEFAULT_QUOTE_TIMEOUT: Duration = Duration::from_secs(10);

/// What the routes need from the enclave. A trait so the router is tested
/// without a dstack agent; production holds a [`JudgementEnclave`].
#[async_trait]
pub trait JudgementIdentity: Send + Sync {
    /// The per-boot response-signing address, `0x` + 40 lowercase hex.
    fn signing_address(&self) -> &str;
    /// The per-boot sealing public key, SEC1 compressed.
    fn seal_public_key(&self) -> [u8; SEAL_PUBLIC_KEY_LEN];
    /// A quote binding both per-boot keys and `nonce` under `tcjudge1`.
    async fn attest(&self, nonce: &JudgementNonce) -> Result<Vec<u8>, EnclaveError>;
    /// Open a sealed request; the sealer answers only its sender.
    fn open_request(&self, sealed: &[u8]) -> Result<(Vec<u8>, ResponseSealer), SealError>;
    /// EIP-191 over `message` by the per-boot key, `0x` + 130 lowercase hex.
    fn sign_eip191(&self, message: &[u8]) -> Result<String, EnclaveError>;
}

#[async_trait]
impl JudgementIdentity for JudgementEnclave {
    fn signing_address(&self) -> &str {
        JudgementEnclave::signing_address(self)
    }

    fn seal_public_key(&self) -> [u8; SEAL_PUBLIC_KEY_LEN] {
        JudgementEnclave::seal_public_key(self)
    }

    async fn attest(&self, nonce: &JudgementNonce) -> Result<Vec<u8>, EnclaveError> {
        JudgementEnclave::attest(self, nonce).await
    }

    fn open_request(&self, sealed: &[u8]) -> Result<(Vec<u8>, ResponseSealer), SealError> {
        JudgementEnclave::open_request(self, sealed)
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
    load: JudgementLoadBound,
    attestation: AttestationBound,
}

impl JudgementHttpState {
    pub fn new(
        service: Arc<JudgementService>,
        identity: Arc<dyn JudgementIdentity>,
        clock: Clock,
        load: JudgementLoadBound,
        attestation: AttestationBound,
    ) -> Self {
        Self {
            service,
            identity,
            clock,
            load,
            attestation,
        }
    }
}

/// What bounds `POST /v1/judgement`: the body reads, the judgement permits
/// and the whole-request deadline.
#[derive(Clone)]
pub struct JudgementLoadBound {
    permits: Arc<Semaphore>,
    request_timeout: Duration,
    reads: Arc<Semaphore>,
    body_read_timeout: Duration,
}

impl JudgementLoadBound {
    /// `max_concurrent_requests` judgement permits under `request_timeout`,
    /// and the default body-read bound. Zero slots or a zero deadline is
    /// refused rather than guessed at.
    pub fn new(
        max_concurrent_requests: usize,
        request_timeout: Duration,
    ) -> Result<Self, &'static str> {
        Self::with_body_reads(
            max_concurrent_requests,
            request_timeout,
            DEFAULT_MAX_CONCURRENT_BODY_READS,
            DEFAULT_BODY_READ_TIMEOUT,
        )
    }

    /// As [`JudgementLoadBound::new`], with the body-read bound given.
    pub fn with_body_reads(
        max_concurrent_requests: usize,
        request_timeout: Duration,
        max_concurrent_body_reads: usize,
        body_read_timeout: Duration,
    ) -> Result<Self, &'static str> {
        if max_concurrent_requests == 0 {
            return Err("judgement concurrency limit must be positive");
        }
        if request_timeout.is_zero() {
            return Err("judgement request timeout must be positive");
        }
        if max_concurrent_body_reads == 0 {
            return Err("judgement body read limit must be positive");
        }
        if body_read_timeout.is_zero() || body_read_timeout >= request_timeout {
            return Err(
                "judgement body read timeout must be positive and inside the request timeout",
            );
        }
        Ok(Self {
            permits: Arc::new(Semaphore::new(max_concurrent_requests)),
            request_timeout,
            reads: Arc::new(Semaphore::new(max_concurrent_body_reads)),
            body_read_timeout,
        })
    }
}

/// What bounds `GET /v1/attestation`: quote calls at once, and each one's
/// timeout.
#[derive(Clone)]
pub struct AttestationBound {
    permits: Arc<Semaphore>,
    timeout: Duration,
}

impl AttestationBound {
    pub fn new(max_concurrent_quotes: usize, timeout: Duration) -> Result<Self, &'static str> {
        if max_concurrent_quotes == 0 {
            return Err("attestation concurrency limit must be positive");
        }
        if timeout.is_zero() {
            return Err("attestation timeout must be positive");
        }
        Ok(Self {
            permits: Arc::new(Semaphore::new(max_concurrent_quotes)),
            timeout,
        })
    }
}

impl Default for AttestationBound {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CONCURRENT_QUOTES, DEFAULT_QUOTE_TIMEOUT)
            .expect("the default bound is positive")
    }
}

/// The router: two routes. Built here so the route wiring is under test.
pub fn judgement_router(state: JudgementHttpState) -> Router {
    let load = state.load.clone();
    Router::new()
        .route("/v1/judgement", post(judgement_handler))
        // `Router::layer` applies to the routes added above it: the
        // deadline covers the judgement route and not the attestation
        // route, which has its own bound.
        .layer(middleware::from_fn_with_state(load, judgement_deadline))
        .route("/v1/attestation", get(attestation_handler))
        .with_state(state)
}

/// A refusal from the closed set, with its status, `reset_at` where the
/// label carries one, and `Retry-After` where the label is retried on a
/// timer.
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
    if matches!(
        refusal,
        JudgementRefusal::Saturated | JudgementRefusal::QuoteUnavailable
    ) {
        response.headers_mut().insert(
            header::RETRY_AFTER,
            HeaderValue::from(JUDGEMENT_SATURATED_RETRY_AFTER_SECS),
        );
    }
    response
}

/// Run the judgement route under the whole-request deadline.
async fn judgement_deadline(
    State(load): State<JudgementLoadBound>,
    request: Request,
    next: Next,
) -> Response {
    match tokio::time::timeout(load.request_timeout, next.run(request)).await {
        Ok(response) => response,
        // Dropping the future cancels the model call under it, and with it
        // the service's own log line; a reservation already taken stands as
        // the charge (spec section 6).
        Err(_elapsed) => {
            log_route_outcome(JudgementRefusal::Timeout.label());
            refusal_response(JudgementRefusal::Timeout, chrono::Utc::now().timestamp())
        }
    }
}

/// The request log line for an outcome the route decides before or after
/// the service's own line: the label and a random request id. No digest
/// hash: either the body was never opened, or the service's line already
/// carried it.
fn log_route_outcome(outcome: &'static str) {
    tracing::info!(
        target: super::service::LOG_TARGET,
        request_id = %uuid::Uuid::new_v4().simple(),
        outcome,
        "judgement_request"
    );
}

/// The answer to a body that did not arrive within its timeout: no label,
/// because no request arrived, and the connection is closed after it.
fn body_read_abandoned() -> Response {
    (
        StatusCode::REQUEST_TIMEOUT,
        [(header::CONNECTION, HeaderValue::from_static("close"))],
    )
        .into_response()
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
    // 1. Read, under the read bound and never a judgement permit.
    let sealed = {
        let Ok(_read) = Arc::clone(&state.load.reads).try_acquire_owned() else {
            log_route_outcome(JudgementRefusal::Saturated.label());
            return Err(JudgementRefusal::Saturated);
        };
        let (_parts, body) = request.into_parts();
        // Stops reading at the cap rather than buffering what was sent.
        let read = axum::body::to_bytes(body, JUDGEMENT_SEALED_REQUEST_MAX_BYTES);
        match tokio::time::timeout(state.load.body_read_timeout, read).await {
            Ok(Ok(sealed)) => sealed,
            Ok(Err(_)) => {
                log_route_outcome(JudgementRefusal::DigestTooLarge.label());
                return Err(JudgementRefusal::DigestTooLarge);
            }
            Err(_elapsed) => {
                log_route_outcome("body_read_timeout");
                return Ok(body_read_abandoned());
            }
        }
    };

    // 2. Open the seal. Nothing else is read from a body that does not open.
    let (plaintext, sealer) = state.identity.open_request(&sealed).map_err(|_| {
        log_route_outcome(JudgementRefusal::SealStale.label());
        JudgementRefusal::SealStale
    })?;
    let plaintext = zeroize::Zeroizing::new(plaintext);

    // 3 and 4. Parse, verify the claim, record the nonce; only then a
    // judgement permit, held until the last model call returns.
    let permits = Arc::clone(&state.load.permits);
    let judged = state
        .service
        .judge(&plaintext, now, move || {
            permits
                .try_acquire_owned()
                .map_err(|_| JudgementRefusal::Saturated)
        })
        .await?;
    drop(plaintext);

    // 5. Spend is settled. From here a failure is a fault, never capacity.
    let fault = || {
        log_route_outcome(JudgementRefusal::ServiceFault.label());
        JudgementRefusal::ServiceFault
    };
    let signing_bytes = judgement_signing_bytes(
        JUDGEMENT_WIRE_VERSION,
        &judged.request_nonce,
        &judged.digest_sha256,
        &judged.results,
        judged.spend,
        now,
    );
    let signature = state
        .identity
        .sign_eip191(&signing_bytes)
        .map_err(|_| fault())?;
    let response = JudgementResponse::new(
        judged.request_nonce,
        judged.digest_sha256,
        judged.results,
        judged.spend,
        now,
        signature,
    )
    .map_err(|_| fault())?;
    let sealed_response = sealer
        .seal(response.to_json().as_bytes())
        .map_err(|_| fault())?;
    Ok((
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        )],
        sealed_response,
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
    let unavailable = || refusal_response(JudgementRefusal::QuoteUnavailable, now);
    let Ok(_permit) = Arc::clone(&state.attestation.permits).try_acquire_owned() else {
        return unavailable();
    };
    let quote = match tokio::time::timeout(state.attestation.timeout, state.identity.attest(&nonce))
        .await
    {
        Ok(Ok(quote)) => quote,
        Ok(Err(_)) | Err(_) => return unavailable(),
    };
    let body = JudgementAttestationResponse::new(
        hex::encode(quote),
        hex::encode(state.identity.seal_public_key()),
        state.identity.signing_address().to_string(),
    );
    match body {
        Ok(body) => (
            StatusCode::OK,
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            body.to_json(),
        )
            .into_response(),
        Err(_) => unavailable(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::judgement_service::catalog::{CatalogEntry, TokenPrices};
    use crate::judgement_service::claim::tests::{ACCOUNT, NOW, TestIssuer};
    use crate::judgement_service::enclave::judgement_report_data;
    use crate::judgement_service::ledger::LedgerState;
    use crate::judgement_service::limiter::tests::MemoryStore;
    use crate::judgement_service::limiter::{Limiter, LimiterCaps, LimiterKeys};
    use crate::judgement_service::mock::{
        MOCK_MODEL_ID, MockFault, MockJudgementModel, MockReceiptSigner,
    };
    use crate::judgement_service::production::ProductionCheck;
    use crate::judgement_service::receipt_check::JudgementCaller;
    use crate::judgement_service::service::LOG_TARGET;
    use crate::judgement_service::service::tests::{Capture, LOG_LINE};
    use crate::witness_service::enclave::{address_of, sign_eip191_with};
    use axum::body::Body;
    use axum::http::{HeaderMap, Method, Request as HttpRequest};
    use k256::ecdsa::SigningKey;
    use std::collections::BTreeMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tower::ServiceExt;
    use trace_commons_attestation::judgement_seal::{
        ResponseOpener, SealPublicKey, SealSecretKey, seal_request,
    };
    use trace_commons_attestation::receipt::recover_eip191_signer;
    use trace_commons_attestation::report_data::{ReportDataLayout, check_report_data};
    use trace_commons_protocol::insights_judgement::{
        JudgementKind, JudgementRequest, JudgementRequestNonce, JudgementRoute,
        parse_judgement_attestation, parse_judgement_refusal, parse_judgement_response,
    };

    const NONCE_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const LIMITER_ROOT: [u8; 32] = [3u8; 32];

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Quote {
        Answers,
        Fails,
        Hangs,
    }

    /// A per-boot identity without a dstack agent: a real sealing key and a
    /// real signing key, and report data in place of a quote.
    struct StubIdentity {
        key: SigningKey,
        address: String,
        seal: SealSecretKey,
        quote: Quote,
        sign_fails: bool,
        attests: std::sync::atomic::AtomicUsize,
    }

    impl StubIdentity {
        fn new() -> Self {
            let key = SigningKey::from_slice(&[7u8; 32]).expect("scalar");
            let address = address_of(&key);
            Self {
                key,
                address,
                seal: SealSecretKey::generate().expect("a sealing key"),
                quote: Quote::Answers,
                sign_fails: false,
                attests: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn address_bytes(&self) -> [u8; 20] {
            hex::decode(&self.address[2..])
                .expect("hex")
                .try_into()
                .expect("20")
        }

        fn seal_key(&self) -> SealPublicKey {
            self.seal.public_key()
        }
    }

    #[async_trait]
    impl JudgementIdentity for StubIdentity {
        fn signing_address(&self) -> &str {
            &self.address
        }
        fn seal_public_key(&self) -> [u8; SEAL_PUBLIC_KEY_LEN] {
            self.seal.public_key().to_bytes()
        }
        async fn attest(&self, nonce: &JudgementNonce) -> Result<Vec<u8>, EnclaveError> {
            self.attests
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match self.quote {
                Quote::Answers => {}
                Quote::Fails => return Err(EnclaveError::Transport),
                Quote::Hangs => std::future::pending::<()>().await,
            }
            // Not a quote: the report data alone, which is what the route
            // must carry through untouched.
            Ok(judgement_report_data(
                &self.seal_public_key(),
                &self.address_bytes(),
                nonce.as_bytes(),
            )
            .to_vec())
        }
        fn open_request(&self, sealed: &[u8]) -> Result<(Vec<u8>, ResponseSealer), SealError> {
            trace_commons_attestation::judgement_seal::open_request(&self.seal, sealed)
        }
        fn sign_eip191(&self, message: &[u8]) -> Result<String, EnclaveError> {
            if self.sign_fails {
                return Err(EnclaveError::SigningFailed);
            }
            sign_eip191_with(&self.key, message).map_err(|_| EnclaveError::SigningFailed)
        }
    }

    struct Harness {
        issuer: TestIssuer,
        model: Arc<MockJudgementModel>,
        identity: Arc<StubIdentity>,
        limiter: Arc<Limiter>,
        router: Router,
        load: JudgementLoadBound,
        attestation: AttestationBound,
    }

    struct Setup {
        caps: LimiterCaps,
        identity: StubIdentity,
        load: JudgementLoadBound,
        attestation: AttestationBound,
    }

    impl Default for Setup {
        fn default() -> Self {
            Self {
                caps: LimiterCaps::from_usd(5, 100).expect("caps"),
                identity: StubIdentity::new(),
                load: JudgementLoadBound::new(4, Duration::from_secs(30)).expect("bound"),
                attestation: AttestationBound::default(),
            }
        }
    }

    fn harness_with(setup: Setup) -> Harness {
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
            setup.caps,
            LedgerState::default(),
            Arc::new(MemoryStore::default()),
        ));
        let caller = JudgementCaller::new(model.clone(), Arc::new(check), limiter.clone());
        let service = Arc::new(JudgementService::new(
            issuer.keys(),
            LimiterKeys::from_root(&LIMITER_ROOT),
            caller,
        ));
        let identity = Arc::new(setup.identity);
        let state = JudgementHttpState::new(
            service,
            identity.clone(),
            Arc::new(|| NOW),
            setup.load.clone(),
            setup.attestation.clone(),
        );
        Harness {
            issuer,
            model,
            identity,
            limiter,
            router: judgement_router(state),
            load: setup.load,
            attestation: setup.attestation,
        }
    }

    fn harness() -> Harness {
        harness_with(Setup::default())
    }

    fn digest() -> Vec<u8> {
        serde_json::json!({
            "schema_version": 1,
            "events": [{"prompt_ix": 0, "kind": "interrupt"}],
            "excerpts": [],
        })
        .to_string()
        .into_bytes()
    }

    fn judgement_request(claim: &str, nonce: u8) -> JudgementRequest {
        JudgementRequest::new(
            JudgementRequestNonce::from_bytes([nonce; 32]),
            claim.to_string(),
            digest(),
            vec![
                JudgementKind::Task,
                JudgementKind::EventKind,
                JudgementKind::Finished,
            ],
        )
        .expect("request")
    }

    fn post(body: Vec<u8>) -> HttpRequest<Body> {
        HttpRequest::builder()
            .method(Method::POST)
            .uri("/v1/judgement")
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(body))
            .expect("request")
    }

    /// `plaintext` sealed to `key`, as a request, and the one opener for
    /// its answer.
    fn sealed(key: &SealPublicKey, plaintext: &[u8]) -> (HttpRequest<Body>, ResponseOpener) {
        let (body, opener) = seal_request(key, plaintext).expect("sealed");
        (post(body), opener)
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

    fn refusal_on(route: JudgementRoute, status: StatusCode, body: &[u8]) -> JudgementRefusalBody {
        parse_judgement_refusal(route, status.as_u16(), body)
            .unwrap_or_else(|| panic!("not a closed-set refusal: {status} {body:?}"))
    }

    /// A refusal from `POST /v1/judgement`.
    fn refusal_of(status: StatusCode, body: &[u8]) -> JudgementRefusalBody {
        refusal_on(JudgementRoute::Judgement, status, body)
    }

    /// A refusal from `GET /v1/attestation`.
    fn attestation_refusal_of(status: StatusCode, body: &[u8]) -> JudgementRefusalBody {
        refusal_on(JudgementRoute::Attestation, status, body)
    }

    /// Send a valid request for `h`'s issuer, return the refusal label.
    async fn refused(h: &Harness, nonce: u8) -> (JudgementRefusal, HeaderMap) {
        let request = judgement_request(&h.issuer.valid_claim(), nonce);
        let (post, _) = sealed(&h.identity.seal_key(), request.to_json().as_bytes());
        let (status, headers, body) = send(&h.router, post).await;
        (refusal_of(status, &body).refusal(), headers)
    }

    #[tokio::test]
    async fn a_judgement_is_sealed_signed_and_bound_to_its_request() {
        // The request log line's callsite; see `service::tests::LOG_LINE`.
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let request = judgement_request(&h.issuer.valid_claim(), 0x11);
        let (post, opener) = sealed(&h.identity.seal_key(), request.to_json().as_bytes());
        let (status, headers, body) = send(&h.router, post).await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        assert_eq!(headers[header::CONTENT_TYPE], "application/octet-stream");
        // The answer is not readable on the path: it is not JSON until the
        // sender's opener opens it.
        assert!(parse_judgement_response(&body).is_err());
        let plaintext = opener.open(&body).expect("the sender opens its answer");
        let response = parse_judgement_response(&plaintext).expect("a closed-shape response");
        assert_eq!(response.signed_at(), NOW);
        assert_eq!(response.results().len(), 3);
        assert_eq!(h.model.calls(), 3);
        assert_eq!(response.check_binding(&request, 1), Ok(()));

        // Signed by the per-boot key over the request nonce: over another
        // nonce the same signature names someone else.
        let signer = recover_eip191_signer(&response.signing_bytes(), response.signature())
            .expect("recovers");
        assert_eq!(signer, h.identity.address_bytes());
        let elsewhere = judgement_signing_bytes(
            JUDGEMENT_WIRE_VERSION,
            &JudgementRequestNonce::from_bytes([0xee; 32]),
            response.digest_sha256(),
            response.results(),
            response.spend(),
            response.signed_at(),
        );
        assert_ne!(
            recover_eip191_signer(&elsewhere, response.signature()).ok(),
            Some(h.identity.address_bytes())
        );
    }

    /// The claim and digest travel only inside the seal: an unsealed body,
    /// with or without a Bearer header, is not opened, judged or charged.
    #[tokio::test]
    async fn an_unsealed_request_is_seal_stale_whatever_its_headers_say() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let claim = h.issuer.valid_claim();
        let plaintext = judgement_request(&claim, 0x12).to_json().into_bytes();
        let mut request = post(plaintext);
        request.headers_mut().insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {claim}")).expect("header"),
        );
        let (status, _, body) = send(&h.router, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            refusal_of(status, &body).refusal(),
            JudgementRefusal::SealStale
        );
        assert_eq!(h.model.calls(), 0);
    }

    /// Design item 11, the misrouting scenario at the service seam: a
    /// request sealed to instance P's key, routed to instance M, does not
    /// open at M, so M learns neither the claim nor the digest and calls
    /// nothing.
    #[tokio::test]
    async fn a_request_sealed_to_one_instance_does_not_open_at_another() {
        let _serial = LOG_LINE.lock().await;
        let pinned = harness();
        let misrouted = harness();
        let request = judgement_request(&pinned.issuer.valid_claim(), 0x13);
        let (post, _) = sealed(&pinned.identity.seal_key(), request.to_json().as_bytes());
        let (status, _, body) = send(&misrouted.router, post).await;
        assert_eq!(
            refusal_of(status, &body).refusal(),
            JudgementRefusal::SealStale
        );
        assert_eq!(misrouted.model.calls(), 0);
        // Positive control: the same request opens at the instance it was
        // sealed to.
        let (post, _) = sealed(&pinned.identity.seal_key(), request.to_json().as_bytes());
        let (status, _, _) = send(&pinned.router, post).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_replayed_request_is_refused_before_any_call() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let request = judgement_request(&h.issuer.valid_claim(), 0x14);
        let (body, _) =
            seal_request(&h.identity.seal_key(), request.to_json().as_bytes()).expect("sealed");
        let (status, _, _) = send(&h.router, post(body.clone())).await;
        assert_eq!(status, StatusCode::OK);
        let calls = h.model.calls();
        let (status, _, reply) = send(&h.router, post(body)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            refusal_of(status, &reply).refusal(),
            JudgementRefusal::RequestReplayed
        );
        assert_eq!(h.model.calls(), calls);
    }

    #[tokio::test]
    async fn a_foreign_credential_is_refused() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let stranger = TestIssuer::new().valid_claim();
        let request = judgement_request(&stranger, 0x15);
        let (post, _) = sealed(&h.identity.seal_key(), request.to_json().as_bytes());
        let (status, _, body) = send(&h.router, post).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            refusal_of(status, &body).refusal(),
            JudgementRefusal::CredentialRefused
        );
        assert_eq!(h.model.calls(), 0);
    }

    #[tokio::test]
    async fn an_oversized_body_is_refused_by_name() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let (status, _, body) = send(
            &h.router,
            post(vec![0x01; JUDGEMENT_SEALED_REQUEST_MAX_BYTES + 1]),
        )
        .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            refusal_of(status, &body).refusal(),
            JudgementRefusal::DigestTooLarge
        );
    }

    #[tokio::test]
    async fn another_wire_version_is_refused_by_name() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let v2 = serde_json::json!({
            "wire_version": 2,
            "request_nonce": NONCE_HEX,
            "claim": h.issuer.valid_claim(),
            "digest_b64": "e30=",
            "judgements": ["task"],
        })
        .to_string();
        let (post, _) = sealed(&h.identity.seal_key(), v2.as_bytes());
        let (status, _, body) = send(&h.router, post).await;
        assert_eq!(
            refusal_of(status, &body).refusal(),
            JudgementRefusal::WireVersionUnsupported
        );
    }

    #[tokio::test]
    async fn a_budget_refusal_carries_the_next_month() {
        let _serial = LOG_LINE.lock().await;
        let h = harness_with(Setup {
            caps: LimiterCaps::from_usd(0, 100).expect("caps"),
            ..Setup::default()
        });
        let request = judgement_request(&h.issuer.valid_claim(), 0x16);
        let (post, _) = sealed(&h.identity.seal_key(), request.to_json().as_bytes());
        let (status, _, body) = send(&h.router, post).await;
        let refusal = refusal_of(status, &body);
        assert_eq!(refusal.refusal(), JudgementRefusal::AccountBudgetExhausted);
        assert_eq!(refusal.reset_at(), judgement_budget_reset_at(NOW));
        assert_eq!(h.model.calls(), 0);
    }

    #[tokio::test]
    async fn a_model_timeout_is_504_and_not_capacity() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        h.model.set_fault(Some(MockFault::Timeout));
        let (refusal, headers) = refused(&h, 0x17).await;
        assert_eq!(refusal, JudgementRefusal::Timeout);
        assert!(headers.get(header::RETRY_AFTER).is_none());
    }

    /// #1531 review point 6: a signing failure after the spend was settled
    /// is a fault, never capacity, and carries no `Retry-After`, so a
    /// daemon does not retry and pay again.
    #[tokio::test]
    async fn a_signing_failure_after_settle_is_a_service_fault() {
        let _serial = LOG_LINE.lock().await;
        let mut identity = StubIdentity::new();
        identity.sign_fails = true;
        let h = harness_with(Setup {
            identity,
            ..Setup::default()
        });
        let request = judgement_request(&h.issuer.valid_claim(), 0x18);
        let (post, _) = sealed(&h.identity.seal_key(), request.to_json().as_bytes());
        let (status, headers, body) = send(&h.router, post).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let refusal = refusal_of(status, &body).refusal();
        assert_eq!(refusal, JudgementRefusal::ServiceFault);
        assert!(!refusal.is_capacity());
        assert!(headers.get(header::RETRY_AFTER).is_none());
        // The fault came after the calls were made and charged.
        assert_eq!(h.model.calls(), 3);
        let account = LimiterKeys::from_root(&LIMITER_ROOT).account_key(ACCOUNT);
        assert!(h.limiter.account_spent_nanousd(&account, NOW) > 0);
    }

    #[tokio::test]
    async fn a_full_service_refuses_with_retry_after_and_still_attests() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let _held = Arc::clone(&h.load.permits)
            .try_acquire_many_owned(4)
            .expect("all four slots");
        let (refusal, headers) = refused(&h, 0x19).await;
        assert_eq!(refusal, JudgementRefusal::Saturated);
        assert_eq!(
            headers[header::RETRY_AFTER],
            JUDGEMENT_SATURATED_RETRY_AFTER_SECS.to_string()
        );
        assert_eq!(h.model.calls(), 0);
        // Load on the judgement route cannot make the service unpinnable.
        let (status, _, _) = send(&h.router, get_attestation(Some(NONCE_HEX))).await;
        assert_eq!(status, StatusCode::OK);
    }

    /// A full read bound refuses before reading, and takes no judgement
    /// permit to do it.
    #[tokio::test]
    async fn a_full_read_bound_is_saturated_and_holds_no_judgement_permit() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let _reads = Arc::clone(&h.load.reads)
            .try_acquire_many_owned(DEFAULT_MAX_CONCURRENT_BODY_READS as u32)
            .expect("every read slot");
        let (refusal, headers) = refused(&h, 0x1a).await;
        assert_eq!(refusal, JudgementRefusal::Saturated);
        assert!(headers.get(header::RETRY_AFTER).is_some());
        assert_eq!(h.load.permits.available_permits(), 4);
        assert_eq!(h.model.calls(), 0);
    }

    /// #1531 review point 4, over a real socket: a caller with no claim
    /// opens a connection, sends headers and a few bytes of a body, and
    /// stops. While it trickles it holds one read slot and no judgement
    /// permit, a real request still gets through, and the read is cut off
    /// at its own timeout rather than the request deadline.
    #[tokio::test]
    async fn a_slow_unauthenticated_body_holds_no_judgement_permit() {
        let _serial = LOG_LINE.lock().await;
        let h = harness_with(Setup {
            load: JudgementLoadBound::with_body_reads(
                1,
                Duration::from_secs(30),
                4,
                Duration::from_millis(300),
            )
            .expect("bound"),
            ..Setup::default()
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let router = h.router.clone();
        let server = tokio::spawn(async move { axum::serve(listener, router).await });

        let mut slow = tokio::net::TcpStream::connect(address)
            .await
            .expect("connect");
        slow.write_all(
            b"POST /v1/judgement HTTP/1.1\r\nHost: judgement\r\n\
              Content-Type: application/octet-stream\r\nContent-Length: 4096\r\n\r\n\x01\x02",
        )
        .await
        .expect("write");
        // Let the server start reading the body.
        let started = std::time::Instant::now();
        while h.load.reads.available_permits() == 4 {
            assert!(started.elapsed() < Duration::from_secs(5), "never read");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // The one judgement permit is free while the body trickles, so a
        // real request is served in the meantime.
        assert_eq!(h.load.permits.available_permits(), 1);
        let request = judgement_request(&h.issuer.valid_claim(), 0x1b);
        let (post, _) = sealed(&h.identity.seal_key(), request.to_json().as_bytes());
        let (status, _, _) = send(&h.router, post).await;
        assert_eq!(status, StatusCode::OK);

        // The slow read ends at its own timeout with a 408 and a close.
        let mut answer = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), slow.read_to_end(&mut answer))
            .await
            .expect("the read bound closes the connection")
            .expect("read");
        let answer = String::from_utf8_lossy(&answer).to_lowercase();
        assert!(answer.starts_with("http/1.1 408"), "{answer}");
        assert!(answer.contains("connection: close"), "{answer}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(h.load.reads.available_permits(), 4);
        assert_eq!(h.load.permits.available_permits(), 1);
        server.abort();
    }

    #[tokio::test]
    async fn a_request_past_its_deadline_is_refused() {
        // The deadline's refusal log line; see `service::tests::LOG_LINE`.
        let _serial = LOG_LINE.lock().await;
        let load = JudgementLoadBound::with_body_reads(
            1,
            Duration::from_millis(20),
            1,
            Duration::from_millis(10),
        )
        .expect("bound");
        let router = slow_router(load);
        for _ in 0..2 {
            let (status, headers, body) = send(&router, slow_request()).await;
            assert_eq!(
                refusal_of(status, &body).refusal(),
                JudgementRefusal::Timeout
            );
            assert!(headers.get(header::RETRY_AFTER).is_none());
        }
    }

    fn slow_router(load: JudgementLoadBound) -> Router {
        Router::new()
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    "late"
                }),
            )
            .layer(middleware::from_fn_with_state(load, judgement_deadline))
    }

    fn slow_request() -> HttpRequest<Body> {
        HttpRequest::builder()
            .uri("/slow")
            .body(Body::empty())
            .expect("request")
    }

    /// The refusals the route makes outside the service are still logged:
    /// the label and a random request id, and no digest hash.
    #[test]
    fn the_route_logs_its_own_refusals_by_label_only() {
        let _serial = LOG_LINE.blocking_lock();
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .finish();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let refusals = tracing::subscriber::with_default(subscriber, || {
            // See `service::tests::no_log_line_carries_the_account_a_label_or_digest_text`.
            tracing::callsite::rebuild_interest_cache();
            runtime.block_on(async {
                let load = JudgementLoadBound::with_body_reads(
                    1,
                    Duration::from_millis(20),
                    1,
                    Duration::from_millis(10),
                )
                .expect("bound");
                let (status, _, body) = send(&slow_router(load), slow_request()).await;
                let timed_out = refusal_of(status, &body).refusal();
                let h = harness();
                let (status, _, body) = send(&h.router, post(vec![0x01; 64])).await;
                let stale = refusal_of(status, &body).refusal();
                (timed_out, stale)
            })
        });
        assert_eq!(
            refusals,
            (JudgementRefusal::Timeout, JudgementRefusal::SealStale)
        );
        let logs = String::from_utf8(capture.0.lock().expect("lock").clone()).expect("utf8");
        assert_eq!(logs.matches(LOG_TARGET).count(), 2, "captured: {logs:?}");
        assert_eq!(logs.matches("request_id=").count(), 2, "captured: {logs:?}");
        assert!(logs.contains(JudgementRefusal::Timeout.label()));
        assert!(logs.contains(JudgementRefusal::SealStale.label()));
        assert!(!logs.contains("digest_sha256"), "captured: {logs:?}");
    }

    #[test]
    fn a_bound_of_zero_is_refused() {
        assert!(JudgementLoadBound::new(0, Duration::from_secs(1)).is_err());
        assert!(JudgementLoadBound::new(1, Duration::ZERO).is_err());
        let second = Duration::from_secs(1);
        assert!(JudgementLoadBound::with_body_reads(1, second * 30, 0, second).is_err());
        assert!(JudgementLoadBound::with_body_reads(1, second * 30, 1, Duration::ZERO).is_err());
        // A body read that may outlast the request deadline is no bound.
        assert!(JudgementLoadBound::with_body_reads(1, second, 1, second).is_err());
        assert!(AttestationBound::new(0, second).is_err());
        assert!(AttestationBound::new(1, Duration::ZERO).is_err());
    }

    #[test]
    fn the_default_bounds_are_the_designed_ones() {
        assert_eq!(DEFAULT_MAX_CONCURRENT_BODY_READS, 32);
        assert_eq!(DEFAULT_BODY_READ_TIMEOUT, Duration::from_secs(10));
        assert_eq!(DEFAULT_MAX_CONCURRENT_QUOTES, 4);
        assert_eq!(DEFAULT_QUOTE_TIMEOUT, Duration::from_secs(10));
        assert_eq!(JUDGEMENT_SEALED_REQUEST_MAX_BYTES, 16_434);
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
    async fn the_attestation_route_serves_the_quote_and_both_per_boot_keys() {
        let h = harness();
        let (status, _, body) = send(&h.router, get_attestation(Some(NONCE_HEX))).await;
        assert_eq!(status, StatusCode::OK);
        let attestation = parse_judgement_attestation(&body).expect("the closed shape");
        assert_eq!(attestation.signing_address(), h.identity.address);
        assert_eq!(
            attestation.seal_public_key_bytes(),
            h.identity.seal_public_key()
        );
        let nonce = JudgementNonce::from_hex(NONCE_HEX).expect("nonce");
        assert_eq!(
            check_report_data(
                &attestation.quote_bytes(),
                ReportDataLayout::Judgement {
                    seal_public_key: &attestation.seal_public_key_bytes(),
                    signing_address: &attestation.signing_address_bytes(),
                },
                nonce.as_bytes()
            ),
            Ok(())
        );
    }

    #[tokio::test]
    async fn the_attestation_route_refuses_a_bad_nonce_and_a_missing_quote() {
        let h = harness();
        for nonce in [None, Some("00"), Some("zz")] {
            let (status, _, body) = send(&h.router, get_attestation(nonce)).await;
            assert_eq!(
                attestation_refusal_of(status, &body).refusal(),
                JudgementRefusal::RequestMalformed
            );
        }
        let mut identity = StubIdentity::new();
        identity.quote = Quote::Fails;
        let h = harness_with(Setup {
            identity,
            ..Setup::default()
        });
        let (status, headers, body) = send(&h.router, get_attestation(Some(NONCE_HEX))).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        let refusal = attestation_refusal_of(status, &body).refusal();
        assert_eq!(refusal, JudgementRefusal::QuoteUnavailable);
        assert!(!refusal.is_capacity());
        assert_eq!(
            headers[header::RETRY_AFTER],
            JUDGEMENT_SATURATED_RETRY_AFTER_SECS.to_string()
        );
    }

    /// #1531 review point 7: a wedged agent costs one bounded wait, and a
    /// flood is refused at the bound rather than queued.
    #[tokio::test]
    async fn a_hung_or_flooded_quote_is_unavailable_within_its_bound() {
        let mut identity = StubIdentity::new();
        identity.quote = Quote::Hangs;
        let h = harness_with(Setup {
            identity,
            attestation: AttestationBound::new(1, Duration::from_millis(50)).expect("bound"),
            ..Setup::default()
        });
        let started = std::time::Instant::now();
        let (status, _, body) = send(&h.router, get_attestation(Some(NONCE_HEX))).await;
        assert_eq!(
            attestation_refusal_of(status, &body).refusal(),
            JudgementRefusal::QuoteUnavailable
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(h.attestation.permits.available_permits(), 1);

        assert_eq!(
            h.identity.attests.load(std::sync::atomic::Ordering::SeqCst),
            1
        );

        // A full bound refuses without asking the agent at all.
        let _held = Arc::clone(&h.attestation.permits)
            .try_acquire_owned()
            .expect("the only slot");
        let (status, _, body) = send(&h.router, get_attestation(Some(NONCE_HEX))).await;
        assert_eq!(
            attestation_refusal_of(status, &body).refusal(),
            JudgementRefusal::QuoteUnavailable
        );
        assert_eq!(
            h.identity.attests.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        // The judgement permits are a different bound.
        assert_eq!(h.load.permits.available_permits(), 4);
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
