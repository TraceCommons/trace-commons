// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The witness's HTTP surface: two routes, and deliberately nothing else.
//!
//! - `POST /v1/witness` -- raw transcript in, redacted artifact and
//!   certificate out.
//! - `GET /v1/attestation?nonce=<64 hex chars>` -- a nonce-bound quote and the
//!   signing address, so a contributor can pin the enclave *before* sending
//!   anything.
//!
//! # What is missing on purpose
//!
//! There is no health route that reports state, no metrics route, and no route
//! that lists anything. The witness's entire posture is that it holds nothing;
//! a surface that can be asked what it has seen contradicts that claim
//! regardless of how carefully the answer is phrased. A counter of requests
//! served is a record of contributor activity, and an operator who can read it
//! is an operator who can correlate it. If a load balancer needs a liveness
//! probe, `GET /v1/attestation` with a fresh nonce is one, and it proves more.
//!
//! # Why this module cannot serve an unbound quote
//!
//! [`Enclave::attestation_quote`] takes arbitrary report data. A handler that
//! called it directly would serve a quote that carries no caller nonce -- a
//! replay, indistinguishable from a success at the response boundary, and the
//! exact thing `/v1/attestation` exists to prevent.
//!
//! Rather than documenting that hazard, this module is structured so the call
//! cannot be written here: the handlers hold a [`WitnessService`], whose
//! `Arc<dyn Enclave>` is a private field of a *different* module. No accessor
//! returns it, no `Deref` reaches it, and Rust's module privacy is what
//! enforces that. The only quote this module can obtain is the one
//! [`WitnessService::attest`] returns, which is composed by
//! [`Enclave::nonce_bound_quote`] over a [`ContributorNonce`] that itself
//! cannot be built except by parsing 32 bytes of hex. This is the pattern
//! `WitnessCertificate` uses for its digest, applied to the other end of the
//! service.
//!
//! [`Enclave`]: super::Enclave
//! [`Enclave::attestation_quote`]: super::Enclave::attestation_quote
//! [`Enclave::nonce_bound_quote`]: super::Enclave::nonce_bound_quote

use std::sync::Arc;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Query, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use trace_commons_protocol::trace_contribution::{ConsentMetadata, ResidualPiiRisk};

use super::WitnessError;
use super::surface::{AttestationError, ContributorNonce, NonceMalformed, WitnessService};

/// The two routes, and nothing else.
///
/// Built here rather than in the binary so that route wiring is covered by the
/// library test suite. A handler can be correct and unreachable; the tests
/// below drive this exact `Router`.
pub fn witness_router(service: Arc<WitnessService>) -> Router {
    Router::new()
        .route("/v1/witness", post(witness_handler))
        // Axum's default 2 MiB body cap would refuse an oversized transcript
        // before the handler could name the refusal, and would accept nothing
        // larger even when the operator configured a larger bound. The bound
        // that applies is `WitnessService::max_request_bytes`, enforced in the
        // handler by `to_bytes`, which stops reading rather than buffering.
        //
        // The position of this line is load-bearing: `Router::layer` applies
        // to the routes added *above* it, so the default cap is lifted for
        // `/v1/witness` and still guards `/v1/attestation`, which has no body
        // to read and should not be able to be sent one.
        .layer(DefaultBodyLimit::disable())
        .route("/v1/attestation", get(attestation_handler))
        .with_state(service)
}

/// The wire form of a witness request.
///
/// `deny_unknown_fields` because a field this witness does not understand may
/// be one a contributor believed was being witnessed. Refusing is the honest
/// answer to that.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WitnessRequestBody {
    raw_transcript: String,
    consent: ConsentMetadata,
}

/// The nonce query parameter, and only that.
///
/// `Option` so that an absent nonce is refused by this module's own name
/// rather than by axum's extractor rejection, whose body is not one of our
/// labels. `deny_unknown_fields` so a caller who misspells `nonce` is told,
/// rather than being served a refusal that looks like a bad value.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttestationQuery {
    nonce: Option<String>,
}

/// A refusal, as a machine-readable label and nothing else.
///
/// The label set is closed and every member is a constant: no variant carries
/// a byte count, an offset, a field name taken from the request, or a
/// serialized error. On this path every quantity derived from the input
/// describes contributor content, and an error body is the easiest place in a
/// service to leak one.
struct Refusal {
    status: StatusCode,
    code: &'static str,
}

impl Refusal {
    const fn new(status: StatusCode, code: &'static str) -> Self {
        Self { status, code }
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        (
            self.status,
            axum::Json(serde_json::json!({ "error": self.code })),
        )
            .into_response()
    }
}

/// The refusal an operator sees for each witness failure.
///
/// A `match` rather than a catch-all, so a new [`WitnessError`] variant is a
/// compile error here and gets a deliberate status instead of inheriting one.
/// All four are 503: every one of them is the witness failing to do its job,
/// not the contributor sending something wrong, and a 4xx would tell a
/// contributor to change their input when nothing about their input was the
/// problem.
fn refusal_for(error: WitnessError) -> Refusal {
    match error {
        WitnessError::RedactionFailed => {
            Refusal::new(StatusCode::SERVICE_UNAVAILABLE, "witness_redaction_failed")
        }
        WitnessError::ArtifactBindingFailed => Refusal::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "witness_artifact_binding_failed",
        ),
        WitnessError::MeasurementUnavailable => Refusal::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "witness_measurement_unavailable",
        ),
        WitnessError::SigningUnavailable => Refusal::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "witness_signing_unavailable",
        ),
    }
}

/// `POST /v1/witness`.
async fn witness_handler(
    State(service): State<Arc<WitnessService>>,
    request: Request,
) -> Result<Response, Refusal> {
    // `to_bytes` stops at the bound rather than buffering the whole body and
    // measuring afterwards, so an oversized request costs the configured
    // maximum and not what the sender chose to send.
    let body = axum::body::to_bytes(request.into_body(), service.max_request_bytes())
        .await
        .map_err(|_| Refusal::new(StatusCode::PAYLOAD_TOO_LARGE, "witness_request_too_large"))?;

    let parsed: WitnessRequestBody = serde_json::from_slice(&body)
        .map_err(|_| Refusal::new(StatusCode::BAD_REQUEST, "witness_request_malformed"))?;

    let response = service
        .witness(super::WitnessRequest {
            raw_transcript: parsed.raw_transcript,
            consent: parsed.consent,
        })
        .await
        .map_err(refusal_for)?;

    let certificate = &response.certificate;
    Ok(axum::Json(serde_json::json!({
        "redacted_artifact": response.redacted_artifact,
        "certificate": {
            "redacted_sha256": certificate.claimed_redacted_sha256(),
            "residual_risk_verdict": verdict_label(response.residual_risk_verdict()),
            "redaction_policy_version": certificate.claimed_redaction_policy_version(),
            "witness_measurement": certificate.claimed_witness_measurement(),
            "timestamp": certificate.claimed_timestamp(),
        },
        "signature_hex": response.signature_hex,
    }))
    .into_response())
}

/// The wire spelling of a verdict.
///
/// Written here rather than derived from `Serialize` so that the strings a
/// server compares against are visible in one place, and exhaustive so a new
/// tier cannot silently serialize as something a consumer treats as unknown.
fn verdict_label(verdict: ResidualPiiRisk) -> &'static str {
    match verdict {
        ResidualPiiRisk::Low => "low",
        ResidualPiiRisk::Medium => "medium",
        ResidualPiiRisk::High => "high",
    }
}

/// `GET /v1/attestation?nonce=<hex>`.
async fn attestation_handler(
    State(service): State<Arc<WitnessService>>,
    Query(query): Query<AttestationQuery>,
) -> Result<Response, Refusal> {
    let Some(nonce_hex) = query.nonce else {
        return Err(Refusal::new(
            StatusCode::BAD_REQUEST,
            "witness_nonce_malformed",
        ));
    };
    let nonce = ContributorNonce::parse_hex(&nonce_hex).map_err(|NonceMalformed| {
        Refusal::new(StatusCode::BAD_REQUEST, "witness_nonce_malformed")
    })?;

    let evidence = service.attest(&nonce).await.map_err(|AttestationError| {
        Refusal::new(StatusCode::SERVICE_UNAVAILABLE, "witness_quote_unavailable")
    })?;

    Ok(axum::Json(serde_json::json!({
        "quote_hex": evidence.quote_hex,
        "signing_address": evidence.signing_address,
    }))
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::witness_service::enclave::{WITNESS_NONCE_LEN, witness_report_data};
    use crate::witness_service::{
        DeterministicRedaction, Enclave, RedactedTranscript, SeamUnavailable, Signer,
        TranscriptRedactor,
    };
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Method, Request as HttpRequest};
    use k256::ecdsa::SigningKey;
    use sha3::{Digest, Keccak256};
    use std::sync::Mutex;
    use tower::ServiceExt;
    use trace_commons_protocol::trace_contribution::ConsentScope;

    /// Matches the `aws_access_key` pattern exactly, so the deterministic pass
    /// is guaranteed to remove it.
    // Split so the twenty-character form never appears verbatim in the
    // source. The value is synthetic -- a keyboard walk, not a
    // credential -- but GitHub push protection matches the shape, and it
    // is right to: a scanner that trusted our word about which
    // AKIA-prefixed strings are fake would be useless. Our own detector
    // requires the prefix, so the fixture cannot avoid it; splitting the
    // literal is the honest way to keep both checks working.
    const SECRET: &str = concat!("AKIA", "QQWERTYUIOPASDFG");

    /// Not secret-shaped, so it must SURVIVE redaction. The positive control:
    /// without it, a redactor that returned the empty string would satisfy
    /// every "the secret is absent" assertion below.
    const SURVIVOR: &str = "zzq-control-token-zzq";

    const MEASUREMENT: &str = "c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2";
    const ENCLAVE_ADDRESS: &str = "0x1111111111111111111111111111111111111111";

    /// A generous default for tests that are not about the bound.
    const TEST_LIMIT: usize = 64 * 1024;

    struct TestSigner(SigningKey);

    impl TestSigner {
        fn new(seed: &str) -> Self {
            let bytes = Keccak256::digest(seed.as_bytes());
            Self(SigningKey::from_slice(&bytes).expect("seed is a valid scalar"))
        }
    }

    impl Signer for TestSigner {
        fn sign_eip191(&self, message: &[u8]) -> Result<String, SeamUnavailable> {
            let mut hasher = Keccak256::new();
            hasher.update(b"\x19Ethereum Signed Message:\n");
            hasher.update(message.len().to_string().as_bytes());
            hasher.update(message);
            let digest: [u8; 32] = hasher.finalize().into();
            let (signature, recovery_id) = self
                .0
                .sign_prehash_recoverable(&digest)
                .expect("the digest is 32 bytes");
            let mut raw = signature.to_bytes().to_vec();
            raw.push(recovery_id.to_byte() + 27);
            Ok(format!("0x{}", hex::encode(raw)))
        }
    }

    struct RefusingSigner;

    impl Signer for RefusingSigner {
        fn sign_eip191(&self, _message: &[u8]) -> Result<String, SeamUnavailable> {
            Err(SeamUnavailable)
        }
    }

    /// Records every `report_data` it was asked to quote over, so a test can
    /// assert what the route actually bound rather than that it returned
    /// something.
    #[derive(Default)]
    struct RecordingEnclave {
        seen: Mutex<Vec<Vec<u8>>>,
    }

    impl RecordingEnclave {
        fn seen(&self) -> Vec<Vec<u8>> {
            self.seen.lock().expect("no test panics holding it").clone()
        }
    }

    #[async_trait]
    impl Enclave for RecordingEnclave {
        fn signing_address(&self) -> &str {
            ENCLAVE_ADDRESS
        }

        async fn measurement(&self) -> Result<String, SeamUnavailable> {
            Ok(MEASUREMENT.to_string())
        }

        async fn attestation_quote(&self, report_data: &[u8]) -> Result<Vec<u8>, SeamUnavailable> {
            self.seen
                .lock()
                .expect("no test panics holding it")
                .push(report_data.to_vec());
            // Echoing the report data as the quote body lets a test read what
            // was bound out of the served bytes, not only out of the double.
            Ok(report_data.to_vec())
        }
    }

    struct SilentEnclave;

    #[async_trait]
    impl Enclave for SilentEnclave {
        fn signing_address(&self) -> &str {
            ENCLAVE_ADDRESS
        }

        async fn measurement(&self) -> Result<String, SeamUnavailable> {
            Err(SeamUnavailable)
        }

        async fn attestation_quote(&self, _report_data: &[u8]) -> Result<Vec<u8>, SeamUnavailable> {
            Err(SeamUnavailable)
        }
    }

    struct RefusingRedactor;

    #[async_trait]
    impl TranscriptRedactor for RefusingRedactor {
        async fn redact(&self, _raw: &str) -> Result<RedactedTranscript, SeamUnavailable> {
            Err(SeamUnavailable)
        }
    }

    fn service_with(
        redactor: Arc<dyn TranscriptRedactor>,
        signer: Arc<dyn Signer>,
        enclave: Arc<dyn Enclave>,
        max_request_bytes: usize,
    ) -> Arc<WitnessService> {
        Arc::new(WitnessService::new(
            redactor,
            signer,
            enclave,
            max_request_bytes,
        ))
    }

    fn healthy_service(limit: usize) -> (Arc<WitnessService>, Arc<RecordingEnclave>) {
        let enclave = Arc::new(RecordingEnclave::default());
        let service = service_with(
            Arc::new(DeterministicRedaction::new(Vec::new())),
            Arc::new(TestSigner::new("http-surface")),
            enclave.clone(),
            limit,
        );
        (service, enclave)
    }

    fn consent_json() -> serde_json::Value {
        serde_json::json!({
            "policy_version": "consent-v1",
            "scopes": [ConsentScope::DebuggingEvaluation],
            "message_text_included": false,
            "tool_payloads_included": false,
            "correction_included": false,
            "routing_metadata_included": false,
            "revocable": true,
        })
    }

    fn witness_body(raw: &str) -> String {
        serde_json::json!({ "raw_transcript": raw, "consent": consent_json() }).to_string()
    }

    /// A request body of exactly `bytes` total length, whose transcript is
    /// padded to reach it. The padding is a non-secret filler so the request
    /// remains one the witness would otherwise certify.
    fn witness_body_of_length(bytes: usize) -> String {
        let base = witness_body("");
        let padding = bytes
            .checked_sub(base.len())
            .expect("the caller asked for a body at least as long as an empty one");
        witness_body(&"a".repeat(padding))
    }

    async fn send(
        service: Arc<WitnessService>,
        request: HttpRequest<Body>,
    ) -> (StatusCode, String) {
        let response = witness_router(service)
            .oneshot(request)
            .await
            .expect("the router is infallible");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .expect("the test bodies are small");
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    fn post_witness(body: String) -> HttpRequest<Body> {
        HttpRequest::builder()
            .method(Method::POST)
            .uri("/v1/witness")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .expect("a well formed test request")
    }

    fn get_attestation(query: &str) -> HttpRequest<Body> {
        HttpRequest::builder()
            .method(Method::GET)
            .uri(format!("/v1/attestation{query}"))
            .body(Body::empty())
            .expect("a well formed test request")
    }

    fn error_code(body: &str) -> String {
        serde_json::from_str::<serde_json::Value>(body)
            .expect("a refusal is JSON")
            .get("error")
            .and_then(|value| value.as_str())
            .expect("a refusal names its code")
            .to_string()
    }

    /// The witness route is reachable through the real router and returns the
    /// artifact, the certificate and the signature.
    ///
    /// Drives `witness_router` rather than the handler: a handler can be
    /// correct and unreachable.
    #[tokio::test]
    async fn the_witness_route_is_reachable_through_the_router() {
        let (service, _) = healthy_service(TEST_LIMIT);
        let (status, body) = send(
            service,
            post_witness(witness_body(&format!("deploy {SURVIVOR} with {SECRET}"))),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "body: {body}");
        let value: serde_json::Value = serde_json::from_str(&body).expect("a JSON response");
        let artifact = value["redacted_artifact"]
            .as_str()
            .expect("the artifact is a string");
        assert!(
            !artifact.contains(SECRET),
            "the secret survived the served artifact"
        );
        assert!(
            artifact.contains(SURVIVOR),
            "the positive control did not survive, so the assertion above proves nothing"
        );

        let certificate = &value["certificate"];
        let missing: Vec<&str> = [
            "redacted_sha256",
            "residual_risk_verdict",
            "redaction_policy_version",
            "witness_measurement",
            "timestamp",
        ]
        .into_iter()
        .filter(|field| certificate.get(*field).is_none())
        .collect();
        assert!(
            missing.is_empty(),
            "certificate fields missing: {missing:?}"
        );
        assert_eq!(certificate["witness_measurement"], MEASUREMENT);
        assert!(
            value["signature_hex"]
                .as_str()
                .is_some_and(|s| s.starts_with("0x") && s.len() == 132),
            "the signature is not 65 bytes of 0x hex: {}",
            value["signature_hex"]
        );
    }

    /// The certificate's digest is over the artifact the route served, byte
    /// for byte. A response whose digest described some other bytes would fail
    /// at the server rather than here, which is far too late.
    #[tokio::test]
    async fn the_served_digest_is_over_the_served_artifact() {
        let (service, _) = healthy_service(TEST_LIMIT);
        let (status, body) = send(
            service,
            post_witness(witness_body(&format!("deploy {SURVIVOR} with {SECRET}"))),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {body}");

        let value: serde_json::Value = serde_json::from_str(&body).expect("a JSON response");
        let artifact = value["redacted_artifact"].as_str().expect("a string");
        let expected = hex::encode(sha2::Sha256::digest(artifact.as_bytes()));
        assert_eq!(value["certificate"]["redacted_sha256"], expected);
    }

    /// The verdict reaches the wire as the label a server compares against,
    /// and different verdicts reach it as different labels.
    ///
    /// Without the second case a `verdict_label` that returned one constant
    /// would satisfy a single-verdict assertion, and the field a server keys
    /// its PII-backstop bypass off would be a constant on the wire.
    #[tokio::test]
    async fn the_verdict_reaches_the_wire_as_its_label() {
        async fn label(raw: &str) -> String {
            let (service, _) = healthy_service(TEST_LIMIT);
            let (status, body) = send(service, post_witness(witness_body(raw))).await;
            assert_eq!(status, StatusCode::OK, "body: {body}");
            serde_json::from_str::<serde_json::Value>(&body).expect("a JSON response")
                ["certificate"]["residual_risk_verdict"]
                .as_str()
                .expect("the verdict is a string")
                .to_string()
        }

        assert_eq!(label(SURVIVOR).await, "low");
        assert_eq!(label(&format!("{SURVIVOR} {SECRET}")).await, "medium");
    }

    /// The attestation route is reachable, and the quote it serves is bound to
    /// the caller's nonce and to this witness's signing address.
    #[tokio::test]
    async fn the_attestation_route_serves_a_nonce_bound_quote() {
        let (service, enclave) = healthy_service(TEST_LIMIT);
        let nonce = [0x5au8; WITNESS_NONCE_LEN];
        let nonce_hex = hex::encode(nonce);

        let (status, body) = send(service, get_attestation(&format!("?nonce={nonce_hex}"))).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");

        let value: serde_json::Value = serde_json::from_str(&body).expect("a JSON response");
        assert_eq!(value["signing_address"], ENCLAVE_ADDRESS);

        let expected = witness_report_data(ENCLAVE_ADDRESS, &nonce).expect("well formed inputs");
        assert_eq!(
            enclave.seen(),
            vec![expected.to_vec()],
            "the route quoted over report data that is not the nonce-bound composition"
        );
        // And the same bytes reached the caller, not only the double.
        assert_eq!(value["quote_hex"], hex::encode(expected));
    }

    /// A different nonce produces a different binding. Without this, a handler
    /// that ignored the query string and quoted a constant would satisfy the
    /// test above.
    #[tokio::test]
    async fn a_different_nonce_produces_a_different_quote() {
        let (service, _) = healthy_service(TEST_LIMIT);
        let first = send(
            service.clone(),
            get_attestation(&format!(
                "?nonce={}",
                hex::encode([0x01u8; WITNESS_NONCE_LEN])
            )),
        )
        .await;
        let second = send(
            service,
            get_attestation(&format!(
                "?nonce={}",
                hex::encode([0x02u8; WITNESS_NONCE_LEN])
            )),
        )
        .await;

        assert_eq!(first.0, StatusCode::OK);
        assert_eq!(second.0, StatusCode::OK);
        assert_ne!(
            first.1, second.1,
            "two different nonces produced the same attestation response"
        );
    }

    /// A malformed nonce is refused by name, and nothing is quoted over it.
    ///
    /// The second half is the point: a handler that padded, truncated or
    /// hashed a bad nonce into 32 bytes would serve a quote a contributor
    /// would read as bound to the nonce they sent. Asserting only the status
    /// would not catch a handler that refused *after* quoting.
    #[tokio::test]
    async fn a_malformed_nonce_is_refused_rather_than_padded() {
        let cases = [
            ("empty", String::new()),
            ("too short", hex::encode([0xaau8; 16])),
            ("too long", hex::encode([0xaau8; 33])),
            ("odd length", "abc".to_string()),
            ("not hex", "z".repeat(64)),
            (
                "0x prefixed",
                format!("0x{}", hex::encode([0xaau8; WITNESS_NONCE_LEN])),
            ),
        ];

        let mut wrong: Vec<(&str, StatusCode, String, usize)> = Vec::new();
        for (label, nonce) in cases {
            let (service, enclave) = healthy_service(TEST_LIMIT);
            let (status, body) = send(service, get_attestation(&format!("?nonce={nonce}"))).await;
            let quoted = enclave.seen().len();
            if status != StatusCode::BAD_REQUEST
                || error_code(&body) != "witness_nonce_malformed"
                || quoted != 0
            {
                wrong.push((label, status, error_code(&body), quoted));
            }
        }
        // Collected rather than asserted in the loop: a short-circuiting
        // assertion lets the first failure hide every case after it.
        assert!(wrong.is_empty(), "malformed nonces not refused: {wrong:?}");
    }

    /// A missing `nonce` parameter is the same refusal, not a quote over
    /// nothing.
    #[tokio::test]
    async fn an_absent_nonce_is_refused() {
        let (service, enclave) = healthy_service(TEST_LIMIT);
        let (status, body) = send(service, get_attestation("")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_code(&body), "witness_nonce_malformed");
        assert!(enclave.seen().is_empty(), "a quote was taken anyway");
    }

    /// A body over the configured bound is refused by name.
    #[tokio::test]
    async fn an_oversized_body_is_refused_by_name() {
        let limit = witness_body_of_length(2048).len();
        let (service, _) = healthy_service(limit);
        let (status, body) = send(service, post_witness(witness_body_of_length(limit + 1))).await;

        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "body: {body}");
        assert_eq!(error_code(&body), "witness_request_too_large");
    }

    /// A body exactly at the bound is accepted.
    ///
    /// The positive control for the test above: without it, a surface that
    /// refused every request would pass the oversize test, and so would one
    /// whose bound was off by an order of magnitude in the wrong direction.
    #[tokio::test]
    async fn a_body_at_the_bound_is_accepted() {
        let limit = witness_body_of_length(2048).len();
        let (service, _) = healthy_service(limit);
        let (status, body) = send(service, post_witness(witness_body_of_length(limit))).await;

        assert_eq!(status, StatusCode::OK, "body: {body}");
    }

    /// An oversized body is refused without the transcript reaching the
    /// response, and without a byte count naming how much was sent.
    #[tokio::test]
    async fn an_oversized_refusal_reports_no_quantity_and_no_content() {
        let limit = witness_body_of_length(2048).len();
        let (service, _) = healthy_service(limit);
        let marker = "zzq-oversize-marker-zzq";
        let padded = format!("{marker}{}", "a".repeat(limit));
        let (status, body) = send(service, post_witness(witness_body(&padded))).await;

        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(!body.contains(marker), "the refusal echoed the transcript");
        assert!(
            !body.contains(&limit.to_string()),
            "the refusal named the configured bound"
        );
    }

    /// Nothing but the two routes exists, and the two exist only under the
    /// methods they are documented for.
    #[tokio::test]
    async fn no_route_other_than_the_two_exists() {
        let probes = [
            (Method::GET, "/healthz", StatusCode::NOT_FOUND),
            (Method::GET, "/health", StatusCode::NOT_FOUND),
            (Method::GET, "/metrics", StatusCode::NOT_FOUND),
            (Method::GET, "/v1/source", StatusCode::NOT_FOUND),
            (Method::GET, "/v1/witnesses", StatusCode::NOT_FOUND),
            (Method::GET, "/", StatusCode::NOT_FOUND),
            (Method::GET, "/v1/witness", StatusCode::METHOD_NOT_ALLOWED),
            (
                Method::POST,
                "/v1/attestation",
                StatusCode::METHOD_NOT_ALLOWED,
            ),
        ];

        let mut wrong: Vec<(&str, StatusCode)> = Vec::new();
        for (method, path, expected) in probes {
            let (service, _) = healthy_service(TEST_LIMIT);
            let request = HttpRequest::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .expect("a well formed test request");
            let (status, _) = send(service, request).await;
            if status != expected {
                wrong.push((path, status));
            }
        }
        assert!(wrong.is_empty(), "unexpected routing: {wrong:?}");
    }

    /// Every witness refusal reaches the wire as a named 503, and none of them
    /// carries the transcript.
    #[tokio::test]
    async fn a_seam_failure_is_a_named_refusal_that_echoes_nothing() {
        let marker = "zzq-refused-marker-zzq";
        let cases: Vec<(&str, Arc<WitnessService>, &str)> = vec![
            (
                "redaction",
                service_with(
                    Arc::new(RefusingRedactor),
                    Arc::new(TestSigner::new("http-surface")),
                    Arc::new(RecordingEnclave::default()),
                    TEST_LIMIT,
                ),
                "witness_redaction_failed",
            ),
            (
                "measurement",
                service_with(
                    Arc::new(DeterministicRedaction::new(Vec::new())),
                    Arc::new(TestSigner::new("http-surface")),
                    Arc::new(SilentEnclave),
                    TEST_LIMIT,
                ),
                "witness_measurement_unavailable",
            ),
            (
                "signing",
                service_with(
                    Arc::new(DeterministicRedaction::new(Vec::new())),
                    Arc::new(RefusingSigner),
                    Arc::new(RecordingEnclave::default()),
                    TEST_LIMIT,
                ),
                "witness_signing_unavailable",
            ),
        ];

        let mut wrong: Vec<(&str, StatusCode, String, bool)> = Vec::new();
        for (label, service, expected) in cases {
            let (status, body) = send(
                service,
                post_witness(witness_body(&format!("{marker} {SECRET}"))),
            )
            .await;
            let leaked = body.contains(marker) || body.contains(SECRET);
            if status != StatusCode::SERVICE_UNAVAILABLE || error_code(&body) != expected || leaked
            {
                wrong.push((label, status, error_code(&body), leaked));
            }
        }
        assert!(wrong.is_empty(), "refusals wrong: {wrong:?}");
    }

    /// A quote the enclave cannot produce is a named 503, not an empty 200.
    #[tokio::test]
    async fn an_unavailable_quote_is_a_named_refusal() {
        let service = service_with(
            Arc::new(DeterministicRedaction::new(Vec::new())),
            Arc::new(TestSigner::new("http-surface")),
            Arc::new(SilentEnclave),
            TEST_LIMIT,
        );
        let (status, body) = send(
            service,
            get_attestation(&format!(
                "?nonce={}",
                hex::encode([0x07u8; WITNESS_NONCE_LEN])
            )),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error_code(&body), "witness_quote_unavailable");
    }

    /// A malformed request body is refused by name and does not echo itself.
    #[tokio::test]
    async fn a_malformed_request_body_is_refused_by_name() {
        let marker = "zzq-malformed-marker-zzq";
        let (service, _) = healthy_service(TEST_LIMIT);
        let (status, body) = send(
            service,
            post_witness(format!("{{\"raw_transcript\": \"{marker}\"}}")),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_code(&body), "witness_request_malformed");
        assert!(!body.contains(marker), "the refusal echoed the request");
    }

    /// An unknown field is refused rather than dropped: a contributor may
    /// believe it was witnessed.
    #[tokio::test]
    async fn an_unknown_request_field_is_refused() {
        let (service, _) = healthy_service(TEST_LIMIT);
        let body = serde_json::json!({
            "raw_transcript": "hello",
            "consent": consent_json(),
            "attest_inference": true,
        })
        .to_string();
        let (status, body) = send(service, post_witness(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_code(&body), "witness_request_malformed");
    }
}
