// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The witness certificate has three independent implementations of one wire
//! format, and until this file existed nothing compared any two of them.
//!
//! - The **producer**: `witness_service::http`, which renders the certificate
//!   into two response headers and signs
//!   `WitnessCertificate::signing_bytes()`.
//! - The **server consumer**: `redaction_witness::request::witness_headers`
//!   plus `verify_witness_certificate`, which reads those headers off
//!   `POST /v1/traces`.
//! - The **client consumer**: `trace_commons_contributor`'s
//!   `verify_certificate`, which rebuilds the signing preimage from the wire
//!   fields with its own encoder, because the server's encoder is AGPL and
//!   unreachable from a permissive crate.
//!
//! Every one of them had a unit suite, every suite was green, and the flow was
//! nonetheless completely non-functional: the client rebuilt the preimage
//! big-endian where the server writes little-endian, and the server consumer
//! read a header name nothing sends, in an encoding nothing produces. Each
//! suite was written against its own side's spelling, so agreement between
//! sides was the one property none of them could observe.
//!
//! # Why this file is in the AGPL crate
//!
//! `trace-commons-contributor` is `MIT OR Apache-2.0` and ships inside
//! proprietary harnesses; `trace-commons-server` is `AGPL-3.0-or-later`.
//! Permissive code may flow into the AGPL crates and never the reverse, so a
//! test that needs both sides can only live on this one. The contributor crate
//! is a `[dev-dependencies]` entry here -- the direction `license_boundary.rs`
//! permits and pins -- and nothing shipped links across.
//!
//! # What each test would catch
//!
//! Nothing here re-spells the wire format by hand. Certificates come out of
//! the real router or the real `certificate_json`, signed by a real key over
//! real `signing_bytes`, and are handed to the real consumers. A fixture
//! spelled by the same author as the code under test agrees with whatever that
//! author believed, which is how all four drifts survived.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use k256::ecdsa::SigningKey;
use sha2::{Digest as _, Sha256};
use sha3::Keccak256;
use tower::ServiceExt as _;

use trace_commons_protocol::trace_contribution::ResidualPiiRisk;
use trace_commons_server::near_attestation::receipt::{ReceiptAlgo, ReceiptPayload};
use trace_commons_server::redaction_witness::certificate::{
    CertificateDetails, WitnessCertificate,
};
use trace_commons_server::redaction_witness::correspondence::check_correspondence;
use trace_commons_server::redaction_witness::request::{
    CERTIFICATE_HEADER, SIGNATURE_HEADER, witness_headers,
};
use trace_commons_server::redaction_witness::verification::{
    WitnessPin, verify_witness_certificate,
};
use trace_commons_server::witness_service::http::{
    WITNESS_CERTIFICATE_HEADER, WITNESS_SIGNATURE_HEADER, WitnessLoadBound, certificate_json,
    verdict_label, witness_router,
};
use trace_commons_server::witness_service::inference::InferenceAttestationPolicy;
use trace_commons_server::witness_service::surface::WitnessService;
use trace_commons_server::witness_service::{
    DeterministicRedaction, Enclave, PipelineContributionRedaction, SeamUnavailable, Signer,
};

use trace_commons_contributor::witness::transport::{
    GrantedConsent, WitnessedEnvelope, verify_certificate, witness_request_body,
};

/// The measurement the test enclave reports and the pin admits.
const MEASUREMENT: &str = "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1";

const TEST_LIMIT: usize = 1024 * 1024;

/// The witness's signing seam, over a key derived from a fixed seed.
///
/// Signs the way the dstack enclave does -- EIP-191 with a 27/28 recovery
/// byte -- because the client recovers a signer address from it and a
/// different framing would recover a different address.
struct TestSigner(SigningKey);

impl TestSigner {
    fn new(seed: &str) -> Self {
        let bytes = Keccak256::digest(seed.as_bytes());
        Self(SigningKey::from_slice(&bytes).expect("the seed is a valid scalar"))
    }

    fn address(&self) -> String {
        let point = self.0.verifying_key().to_encoded_point(false);
        let digest = Keccak256::digest(&point.as_bytes()[1..]);
        format!("0x{}", hex::encode(&digest[12..]))
    }
}

impl Signer for TestSigner {
    fn sign_eip191(&self, message: &[u8]) -> Result<String, SeamUnavailable> {
        let mut hasher = Keccak256::new();
        hasher.update(b"\x19Ethereum Signed Message:\n");
        hasher.update(message.len().to_string().as_bytes());
        hasher.update(message);
        let digest: [u8; 32] = hasher.finalize().into();
        let (signature, recovery) = self
            .0
            .sign_prehash_recoverable(&digest)
            .expect("the digest is 32 bytes");
        let mut raw = signature.to_bytes().to_vec();
        raw.push(recovery.to_byte() + 27);
        Ok(format!("0x{}", hex::encode(raw)))
    }
}

/// The enclave seam, reporting the address the signer will actually recover
/// to.
///
/// Production unites the two seams in one `DstackEnclave`. Uniting them here
/// too is load-bearing: the whole check is that the address a client recovers
/// from the signature is the address it pinned, and a double that reported
/// some other constant would make that comparison pass for the wrong reason.
struct TestEnclave(String);

#[async_trait::async_trait]
impl Enclave for TestEnclave {
    fn signing_address(&self) -> &str {
        &self.0
    }

    async fn measurement(&self) -> Result<String, SeamUnavailable> {
        Ok(MEASUREMENT.to_string())
    }

    async fn attestation_quote(&self, _report_data: &[u8]) -> Result<Vec<u8>, SeamUnavailable> {
        Ok(vec![0xab; 8])
    }
}

/// A witness with the structured seam attached, and the address it signs
/// under.
fn structured_service() -> (Arc<WitnessService>, String) {
    let signer = TestSigner::new("cross-implementation");
    let address = signer.address();
    let service = WitnessService::new(
        Arc::new(DeterministicRedaction::new(Vec::new())),
        Arc::new(signer),
        Arc::new(TestEnclave(address.clone())),
        TEST_LIMIT,
    )
    .with_contribution_redactor(Arc::new(PipelineContributionRedaction::deterministic_only(
        Vec::new(),
    )));
    (Arc::new(service), address)
}

fn contribution_body(text: &str) -> String {
    use trace_commons_protocol::trace_contribution::{
        RawTraceCaptureTurn, RawTraceContribution, RecordedTraceContributionOptions,
    };
    let started = chrono::Utc::now();
    let raw = RawTraceContribution::from_capture_turns(
        &[RawTraceCaptureTurn {
            user_input: text.to_string(),
            response: None,
            tool_calls: Vec::new(),
            started_at: started,
            completed_at: Some(started + chrono::Duration::milliseconds(10)),
            state: Some("Completed".to_string()),
        }],
        RecordedTraceContributionOptions {
            include_message_text: true,
            ..RecordedTraceContributionOptions::default()
        },
    );
    serde_json::json!({
        "raw_contribution": serde_json::to_value(&raw).expect("a raw contribution serialises"),
        "granted_scopes": ["debugging_evaluation"],
        "granted_uses": ["debugging"],
    })
    .to_string()
}

/// Everything a contributor holds after `POST /v1/witness`: the envelope
/// bytes, and the two header values, read off the real response.
struct FromTheWire {
    envelope_bytes: Vec<u8>,
    certificate_header: String,
    signature_header: String,
    /// The response's headers, unmodified.
    ///
    /// Kept whole rather than reduced to the two values above, because a
    /// contributor forwards the header NAMES it received as well as their
    /// values. Rebuilding a map keyed by the ingest reader's own constants
    /// would key both sides off the same symbol and make a name drift
    /// invisible -- which is how the reader came to look up a header nothing
    /// sends.
    headers: axum::http::HeaderMap,
}

/// Drive the witness's own router and keep what it put on the wire.
async fn witness_over_the_wire(service: Arc<WitnessService>, text: &str) -> FromTheWire {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/witness")
        .header("content-type", "application/json")
        .body(Body::from(contribution_body(text)))
        .expect("a well formed request");

    let response = witness_router(
        service,
        // Not what this test is about: wide enough that the bound never fires.
        WitnessLoadBound::new(8, std::time::Duration::from_secs(30)),
    )
    .oneshot(request)
    .await
    .expect("the router is infallible");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the witness refused the fixture"
    );

    let headers = response.headers().clone();
    let read = |name: &str| {
        headers
            .get(name)
            .unwrap_or_else(|| panic!("the response carried no {name} header"))
            .to_str()
            .expect("the header is ASCII")
            .to_string()
    };
    let certificate_header = read(WITNESS_CERTIFICATE_HEADER);
    let signature_header = read(WITNESS_SIGNATURE_HEADER);

    let envelope_bytes = axum::body::to_bytes(response.into_body(), TEST_LIMIT)
        .await
        .expect("the fixture body is small")
        .to_vec();

    FromTheWire {
        envelope_bytes,
        certificate_header,
        signature_header,
        headers,
    }
}

/// The headline: a certificate this client accepts is one the server issued.
///
/// The client's `certificate_signing_bytes` is a second implementation of an
/// encoding whose first implementation is AGPL and unreachable from the
/// permissive crate. This is the only thing that requires the two to agree,
/// and it fails on any difference in either -- the domain string, the field
/// order, the length-prefix endianness, the verdict tags, the timestamp
/// endianness, or the JSON field names the client reads them out of.
#[tokio::test]
async fn a_certificate_this_client_accepts_is_one_the_server_issued() {
    let (service, address) = structured_service();
    let wire = witness_over_the_wire(service, "ran the build and read the log").await;

    let envelope = WitnessedEnvelope {
        envelope_bytes: wire.envelope_bytes,
        certificate_json: wire.certificate_header,
        signature_hex: wire.signature_header,
    };

    verify_certificate(&envelope, &address)
        .expect("the client must accept a certificate this witness issued");
}

/// And the ingest reader accepts the same two header values, unchanged.
///
/// A contributor forwards what it received byte for byte, so this drives the
/// exact strings the witness put on its response through the header names and
/// the encoding `POST /v1/traces` reads, and then through the full three-check
/// verification against a real pin.
#[tokio::test]
async fn the_headers_this_witness_serves_are_the_headers_ingest_reads() {
    let (service, address) = structured_service();
    let wire = witness_over_the_wire(service, "ran the build and read the log").await;

    // The witness's own response headers, forwarded whole. `witness_headers`
    // looks up ITS constants in this map, so a name it does not share with the
    // witness surfaces here as `Ok(None)` -- the silent
    // "ordinary unwitnessed submission" this seam shipped with.
    let (certificate, signature) = witness_headers(&wire.headers)
        .expect("ingest must read the headers the witness serves")
        .expect("ingest found neither header on the witness's own response");

    let pin = WitnessPin::new(&address, [MEASUREMENT.to_string()]).expect("the pin is well formed");
    verify_witness_certificate(certificate, &signature, Some(&pin), &wire.envelope_bytes)
        .expect("ingest must verify a certificate this witness issued");
}

/// A certificate, rendered by the real producer and signed over the real
/// preimage, for one verdict.
///
/// `check_correspondence` over identical bytes is the only way to obtain the
/// `CorrespondenceProof` that `from_proof` requires, so the digest is of these
/// bytes and no others -- the same path `witness_contribution` takes.
fn issued(signer: &TestSigner, artifact: &str, verdict: ResidualPiiRisk) -> (String, String) {
    let proof = check_correspondence(artifact, artifact, &[]).expect("identical bytes correspond");
    let certificate = WitnessCertificate::from_proof(
        proof,
        CertificateDetails {
            residual_risk_verdict: verdict,
            redaction_policy_version: "deterministic-only-v1".to_string(),
            witness_measurement: MEASUREMENT.to_string(),
            timestamp: chrono::Utc::now().timestamp(),
        },
    );
    let signature = signer
        .sign_eip191(&certificate.signing_bytes())
        .expect("the test signer is available");
    let json = serde_json::to_string(&certificate_json(&certificate, verdict))
        .expect("the certificate renders");
    (json, signature)
}

/// Every verdict the witness can certify survives both consumers.
///
/// The verdict is the one field that is a fixed-width tag rather than its
/// label: the server maps Low/Medium/High to 1/2/3 in `residual_risk_tag`,
/// and the client maps the wire LABELS back to the same tags in an inline
/// match. Two hand-written mappings of a closed set, in two crates. A swap or
/// a renumber on either side would let a Medium certificate re-verify as Low,
/// and nothing compared them.
///
/// Driven over all three variants rather than whichever one the redaction
/// pipeline happens to produce for a fixture, because a mapping is only pinned
/// where it is exercised.
#[tokio::test]
async fn every_verdict_survives_both_consumers() {
    const ARTIFACT: &str = "{\"schema_version\":1,\"turns\":[]}";

    let signer = TestSigner::new("cross-implementation");
    let address = signer.address();
    let pin = WitnessPin::new(&address, [MEASUREMENT.to_string()]).expect("the pin is well formed");

    for verdict in [
        ResidualPiiRisk::Low,
        ResidualPiiRisk::Medium,
        ResidualPiiRisk::High,
    ] {
        let label = verdict_label(verdict);
        let (json, signature) = issued(&signer, ARTIFACT, verdict);

        // The client.
        let envelope = WitnessedEnvelope {
            envelope_bytes: ARTIFACT.as_bytes().to_vec(),
            certificate_json: json.clone(),
            signature_hex: signature.clone(),
        };
        verify_certificate(&envelope, &address)
            .unwrap_or_else(|err| panic!("the client refused a {label} certificate: {err:?}"));

        // And ingest, over the same two header values.
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(CERTIFICATE_HEADER, json.parse().expect("a header value"));
        headers.insert(SIGNATURE_HEADER, signature.parse().expect("a header value"));
        let (certificate, signature) = witness_headers(&headers)
            .unwrap_or_else(|err| panic!("ingest could not read a {label} certificate: {err:?}"))
            .expect("both headers are present");
        verify_witness_certificate(certificate, &signature, Some(&pin), ARTIFACT.as_bytes())
            .unwrap_or_else(|err| panic!("ingest refused a {label} certificate: {err:?}"));
    }
}

/// The digest the certificate binds is the digest of the body that came with
/// it.
///
/// Anchors the one property both consumers check independently, so a change
/// that made either of them compare against a re-serialisation rather than the
/// bytes on the wire is visible here rather than in production.
#[tokio::test]
async fn the_certificate_binds_the_envelope_bytes_as_served() {
    let (service, _) = structured_service();
    let wire = witness_over_the_wire(service, "ran the build").await;

    let certificate: serde_json::Value =
        serde_json::from_str(&wire.certificate_header).expect("the header is JSON");
    assert_eq!(
        certificate["redacted_sha256"]
            .as_str()
            .expect("the digest is a string"),
        hex::encode(Sha256::digest(&wire.envelope_bytes)),
        "the certificate names a digest that is not the body's"
    );
}

// ---------------------------------------------------------------------------
// The offered inference receipt: a fourth implementation of one wire format.
// ---------------------------------------------------------------------------
//
// `WitnessRequestBody` is `#[serde(deny_unknown_fields)]`. So the contributor's
// spelling of `inference_receipt` is not a field that degrades to "no receipt"
// when it is wrong -- a misspelling makes EVERY witnessed submission a 400,
// which the client reports as an unreachable witness. Nothing but a test that
// hands the client's own bytes to the server's own deserialiser can observe
// that, and the client cannot host such a test: this crate is AGPL and
// unreachable from the permissive one.
//
// The client-side unit tests assert the shape of the document it builds. They
// would pass unchanged against a field the server has never heard of.

/// A receipt over these two bodies, signed the way the inference enclave signs.
///
/// The **two**-part text -- `<requestHash>:<responseHash>` -- because that is
/// what NEAR AI signs today. It binds no model, and nothing here or downstream
/// may read one out of it.
fn receipt_over(signer: &TestSigner, request_body: &str, response_body: &str) -> ReceiptPayload {
    let text = format!(
        "{}:{}",
        hex::encode(Sha256::digest(request_body.as_bytes())),
        hex::encode(Sha256::digest(response_body.as_bytes()))
    );
    let signature = signer
        .sign_eip191(text.as_bytes())
        .expect("the test signer is available");
    ReceiptPayload {
        text,
        signature,
        signing_address: signer.address(),
        signing_algo: ReceiptAlgo::Ecdsa,
    }
}

/// One attestable call, built through the client's own reader.
///
/// `attested_final_call` rather than a hand-built `AttestedCall`: the digests
/// it checks are the ones the receipt above is taken over, so a fixture that
/// is not actually attestable fails here instead of proving nothing.
fn attestable_call() -> (
    trace_commons_contributor::routing::attested::AttestedCall,
    tempfile::TempDir,
) {
    const REQUEST: &str = "{\"model\":\"Qwen/Qwen3.6-27B-FP8\",\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}";
    const RESPONSE: &str =
        "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n";

    let dir = tempfile::tempdir().expect("a temporary body store");
    let reference = "00000000000000000007-000000";
    std::fs::write(dir.path().join(format!("{reference}.req")), REQUEST).expect("the request body");
    std::fs::write(dir.path().join(format!("{reference}.res")), RESPONSE)
        .expect("the response body");

    let row = trace_commons_contributor::routing::RoutedExchange {
        id: Some(7),
        started_at: chrono::Utc::now(),
        client_session_id: Some("session".to_string()),
        total_ms: Some(10),
        facade: "openai".to_string(),
        backend: "nearai".to_string(),
        requested_model: Some("Qwen/Qwen3.6-27B-FP8".to_string()),
        served_model: Some("Qwen/Qwen3.6-27B-FP8".to_string()),
        upstream_id: Some("chatcmpl-abc123".to_string()),
        request_sha256: Some(hex::encode(Sha256::digest(REQUEST.as_bytes()))),
        response_sha256: Some(hex::encode(Sha256::digest(RESPONSE.as_bytes()))),
        body_ref: Some(reference.to_string()),
        rung: "full".to_string(),
        attempts: 1,
        input_tokens: Some(1),
        cache_read_tokens: None,
        cache_write_tokens: None,
        output_tokens: Some(1),
        cost_usd: Some(0.0),
        status: 200,
    };
    let call =
        trace_commons_contributor::routing::attested::attested_final_call(&[row], dir.path())
            .expect("the fixture must be attestable, or these tests prove nothing");
    (call, dir)
}

/// The client's `POST /v1/witness` document, with the attested exchange
/// appended exactly as `witness_contribution` appends it, and the receipt
/// passed through the client's own builder.
fn client_request_body(
    call: &trace_commons_contributor::routing::attested::AttestedCall,
    receipt: Option<ReceiptPayload>,
) -> Vec<u8> {
    use trace_commons_protocol::trace_contribution::{
        RawTraceCaptureTurn, RawTraceContribution, RecordedTraceContributionOptions,
    };
    let started = chrono::Utc::now();
    let mut raw = RawTraceContribution::from_capture_turns(
        &[RawTraceCaptureTurn {
            user_input: "ran the build and read the log".to_string(),
            response: None,
            tool_calls: Vec::new(),
            started_at: started,
            completed_at: Some(started + chrono::Duration::milliseconds(10)),
            state: Some("Completed".to_string()),
        }],
        RecordedTraceContributionOptions {
            include_message_text: true,
            ..RecordedTraceContributionOptions::default()
        },
    );
    raw.events
        .push(trace_commons_contributor::routing::attested::attested_exchange_event(call));

    // The receipt is handed over as a value, never as JSON this file spelled:
    // the whole point is that the CLIENT chooses the field names on the wire.
    // `ReceiptPayload` is the same type on both sides -- the server re-exports
    // the attestation crate's -- so nothing is transcribed.
    witness_request_body(
        &raw,
        &GrantedConsent {
            scopes: vec![
                trace_commons_protocol::trace_contribution::ConsentScope::DebuggingEvaluation,
            ],
            uses: vec![trace_commons_protocol::trace_contribution::TraceAllowedUse::Debugging],
        },
        receipt.as_ref(),
    )
    .expect("the client builds its own request document")
}

/// A witness that refuses anything unattested, and the address it signs under.
fn requiring_service() -> (Arc<WitnessService>, TestSigner) {
    let signer = TestSigner::new("cross-implementation");
    let address = signer.address();
    let service = WitnessService::new(
        Arc::new(DeterministicRedaction::new(Vec::new())),
        Arc::new(TestSigner::new("cross-implementation")),
        Arc::new(TestEnclave(address)),
        TEST_LIMIT,
    )
    .with_contribution_redactor(Arc::new(PipelineContributionRedaction::deterministic_only(
        Vec::new(),
    )))
    .requiring_attested_inference(
        InferenceAttestationPolicy::required(TEST_LIMIT).expect("the policy requires something"),
    );
    (Arc::new(service), signer)
}

/// Drive the real router with a raw body and keep the status and the label.
async fn post_witness(service: Arc<WitnessService>, body: Vec<u8>) -> (StatusCode, String) {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/witness")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("a well formed request");
    let response = witness_router(
        service,
        WitnessLoadBound::new(8, std::time::Duration::from_secs(30)),
    )
    .oneshot(request)
    .await
    .expect("the router is infallible");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), TEST_LIMIT)
        .await
        .expect("the response body is small");
    let label = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|document| {
            document
                .get("error")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    (status, label)
}

/// The headline: a receipt the client offers is one the witness reads.
///
/// The witness here **requires** attested inference, so the only way to reach
/// `200` is for the offered receipt to have been found, parsed, and verified
/// against the bodies the client put in the same document. A field named
/// anything other than `inference_receipt` is a `400
/// witness_request_malformed` (`deny_unknown_fields`); a field found but
/// shaped wrong is the same; a receipt read but not matched to the bodies is
/// `403 witness_inference_receipt_unverified`. Each failure is distinct from
/// the success, and none of them is reachable from the client's own tests.
#[tokio::test]
async fn a_receipt_this_client_offers_is_one_the_witness_verifies() {
    let (service, signer) = requiring_service();
    let (call, _dir) = attestable_call();
    let receipt = receipt_over(&signer, call.request_body(), call.response_body());

    let (status, label) = post_witness(service, client_request_body(&call, Some(receipt))).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "a witness requiring attestation refused a receipt this client offered: {label}"
    );
}

/// And the same submission with no receipt is refused, by name.
///
/// This is what makes the test above non-vacuous: without it, a witness that
/// silently ignored the receipt field entirely would also return `200`. The
/// two together say the field is read and that reading it is what decided the
/// outcome.
#[tokio::test]
async fn the_same_submission_without_a_receipt_is_refused_by_name() {
    let (service, _) = requiring_service();
    let (call, _dir) = attestable_call();

    let (status, label) = post_witness(service, client_request_body(&call, None)).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        label, "witness_inference_attestation_missing",
        "an absent receipt must be refused as an absent receipt"
    );
}

/// A receipt over other bytes does not pass, even though it is a real
/// signature by the right key.
///
/// The client forwards what the provider gave it and checks nothing. So the
/// binding between a receipt and this trace is entirely the witness's, and
/// this pins that the bodies the client carried are the bodies the witness
/// hashed -- not merely that a well-formed receipt was present.
#[tokio::test]
async fn a_receipt_over_other_bytes_is_refused() {
    let (service, signer) = requiring_service();
    let (call, _dir) = attestable_call();
    let elsewhere = receipt_over(&signer, "some other request", "some other response");

    let (status, label) = post_witness(service, client_request_body(&call, Some(elsewhere))).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(label, "witness_inference_receipt_unverified");
}

// ---------------------------------------------------------------------------
// `signing_algo` on the wire.
// ---------------------------------------------------------------------------
//
// The client's own serialiser does not emit this field yet (that lands with
// the contributor change), so these tests craft the `inference_receipt`
// object directly and splice it into a client-built document -- the same
// document `client_request_body` produces for every other field, with only
// `inference_receipt` replaced. That is what proves the *server's*
// deserialiser, not a hand-written fixture that might only agree with itself.

/// A client-built document with `inference_receipt` replaced by a raw JSON
/// object this file controls, so a wire shape no serialiser here emits yet
/// can still be sent to the real router.
fn body_with_raw_receipt(
    call: &trace_commons_contributor::routing::attested::AttestedCall,
    receipt: serde_json::Value,
) -> Vec<u8> {
    let base = client_request_body(call, None);
    let mut document: serde_json::Value =
        serde_json::from_slice(&base).expect("the client's own document is valid JSON");
    document["inference_receipt"] = receipt;
    serde_json::to_vec(&document).expect("a JSON value serialises")
}

/// A client may omit `signing_algo`, and the field's absence is accepted by
/// the deserialiser -- every receipt issued before this field existed is
/// ECDSA, and a witness that refused an absent field would refuse every
/// existing client. The receipt below is nonsense, so it fails verification;
/// the point is that it fails as *unverified*, not as *malformed*, which is
/// what proves the omission reached the verifier rather than being rejected
/// as an unrecognised shape.
#[tokio::test]
async fn a_receipt_without_signing_algo_is_read_as_ecdsa() {
    let (service, _signer) = requiring_service();
    let (call, _dir) = attestable_call();
    let body = body_with_raw_receipt(
        &call,
        serde_json::json!({
            "text": "aaaa1111:bbbb2222",
            "signature": "0xcccc3333",
            "signing_address": "0xdddd444444444444444444444444444444444444"
        }),
    );

    let (status, label) = post_witness(service, body).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(label, "witness_inference_receipt_unverified");
}

/// An unrecognised `signing_algo` is a malformed request, never a guess.
#[tokio::test]
async fn an_unknown_signing_algo_is_refused_as_malformed() {
    let (service, _signer) = requiring_service();
    let (call, _dir) = attestable_call();
    let body = body_with_raw_receipt(
        &call,
        serde_json::json!({
            "text": "aaaa1111:bbbb2222",
            "signature": "0xcccc3333",
            "signing_address": "0xdddd444444444444444444444444444444444444",
            "signing_algo": "rsa"
        }),
    );

    let (status, label) = post_witness(service, body).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(label, "witness_request_malformed");
}

/// A real, live-captured ed25519 receipt (from NEAR AI; Task 1 verifies its
/// signature) is accepted on the wire and reaches the verifier, rather than
/// being refused as an unrecognised `signing_algo` value.
///
/// This is a narrower claim than "the witness verified it as ed25519" --
/// this route cannot prove that, and must not be made to: every way a
/// receipt fails to verify is deliberately folded into one label
/// (`witness_inference_receipt_unverified`, see `inference.rs`), on purpose,
/// so an unauthenticated caller cannot learn which part of a forged receipt
/// was closest. So a receipt misread as ECDSA (failing early as a malformed
/// 20-byte address) and one correctly read as ed25519 (failing at the
/// digest, because this fixture's bodies are not the ones this test's
/// contribution carries) produce the exact same 403 here, by design. What
/// this test proves is only that the field parses and the request reaches
/// the verifier at all, rather than being rejected as malformed input.
/// That the value actually becomes `ReceiptAlgo::Ed25519` is proven at the
/// seam that dispatches on it: `http::tests::
/// the_wire_signing_algo_becomes_the_payload_discriminator`, in-crate,
/// where the wire is not the only thing that can be inspected.
#[tokio::test]
async fn an_ed25519_receipt_is_accepted_on_the_wire_and_reaches_the_verifier() {
    let (service, _signer) = requiring_service();
    let (call, _dir) = attestable_call();
    let body = body_with_raw_receipt(
        &call,
        serde_json::json!({
            "text": "81e9887990592366b55ef758cad3b3a097e890871bedc023a51b2828ed237cc3:6f7091a0fbe5917a631c70805833760fe63ceea3493466e3230bd830816a3f2e",
            "signature": "838765bd299514ec80084d50b7cef9357172ce2923dd35aa837beed0c6af04e684673e61db6c0d3ae8d69476b680d94c8e1e36e05277a1b103c27a12f563eb0c",
            "signing_address": "cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6",
            "signing_algo": "ed25519"
        }),
    );

    let (status, label) = post_witness(service, body).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(label, "witness_inference_receipt_unverified");
}

/// The client's own serialiser now emits `signing_algo` (this is the change
/// that lands it), so this is the true cross-implementation check that the
/// client's `"signing_algo"` key and the server's field agree byte-for-byte
/// -- built with `witness_request_body` from the contributor crate, the same
/// way `a_receipt_this_client_offers_is_one_the_witness_verifies` is, rather
/// than spliced raw JSON as the tests above still are.
///
/// The witness deliberately folds every receipt failure into one wire label
/// (`witness_inference_receipt_unverified`), so this proves only that the
/// FIELD crossed correctly, not which verification step failed: the receipt
/// below is over bytes that are not this call's, so it is refused at the
/// digest check (signature checked as ed25519) rather than being rejected as
/// malformed input.
#[tokio::test]
async fn an_ed25519_receipt_this_client_serialises_is_read_as_ed25519_by_the_witness() {
    let (service, _signer) = requiring_service();
    let (call, _dir) = attestable_call();
    let receipt = ReceiptPayload {
        text: "81e9887990592366b55ef758cad3b3a097e890871bedc023a51b2828ed237cc3:\
               6f7091a0fbe5917a631c70805833760fe63ceea3493466e3230bd830816a3f2e"
            .to_string(),
        signature: "838765bd299514ec80084d50b7cef9357172ce2923dd35aa837beed0c6af04e\
                    684673e61db6c0d3ae8d69476b680d94c8e1e36e05277a1b103c27a12f563eb0c"
            .to_string(),
        signing_address: "cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6"
            .to_string(),
        signing_algo: ReceiptAlgo::Ed25519,
    };

    let (status, label) = post_witness(service, client_request_body(&call, Some(receipt))).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(label, "witness_inference_receipt_unverified");
}
