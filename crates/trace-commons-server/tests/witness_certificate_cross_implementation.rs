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
use trace_commons_server::witness_service::surface::WitnessService;
use trace_commons_server::witness_service::{
    DeterministicRedaction, Enclave, PipelineContributionRedaction, SeamUnavailable, Signer,
};

use trace_commons_contributor::witness::transport::{WitnessedEnvelope, verify_certificate};

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
