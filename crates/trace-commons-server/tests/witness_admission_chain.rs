// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The invite-free admission chain, end to end, with no hand-built headers on
//! either side.
//!
//! Both halves of this chain had tests and neither had a test that crossed it.
//! The client-side seam test signs a witness response with a fixture key and
//! never stands the route up; `admission_pg_tests` hand-builds the evidence
//! headers it feeds to ingest. So the client was tested against what its author
//! believed the witness returns, the server against what its author believed a
//! client sends, and agreement between them was the one property neither could
//! observe -- the same shape as the four drifts that
//! `witness_certificate_cross_implementation` exists to catch, on the next
//! route along.
//!
//! What crosses here:
//!
//! 1. the **client's** `witness_request_body`, the only function in the
//!    contributor crate permitted to spell that document,
//! 2. the **real** `/v1/witness/admission` route from `witness_router`, driven
//!    over `tower::oneshot` into the real `witness_admission_contribution`,
//!    which runs `verify_admission_call` and certifies,
//! 3. the **client's** `verify_certificate` over the response it gets back,
//!    including that function's admission-header checks,
//! 4. **ingest's** `verify_witness_certificate` and `verify_admission_evidence`
//!    over the headers the witness actually emitted.
//!
//! This file is in the AGPL crate for the reason
//! `witness_certificate_cross_implementation` gives: a test needing both sides
//! can only live here, because permissive code may flow into the AGPL crates
//! and never the reverse.
//!
//! # What is real and what stands in
//!
//! Real: the client encoder, the router, the handler, redaction, receipt
//! verification (genuine Ed25519 over the genuine two- and three-part receipt
//! texts), the certificate and its signature, and both server-side verifiers.
//!
//! Stood in for, and named rather than disguised:
//!
//! - **The enclave.** `Enclave::attestation_quote` returns bytes, not a DCAP
//!   quote; CI has no TDX. Nothing here verifies a quote, so nothing here
//!   depends on it being real -- and note the consequence, which is that the
//!   client's `witness_session` (which *does* verify one) cannot be driven from
//!   a test. That is why step 1 enters at `witness_request_body` rather than at
//!   the top of the client's transport.
//! - **The provider keys.** Ed25519 keypairs generated here, not NEAR AI's
//!   attested keys. The signatures are real and verified by the real verifier;
//!   what is absent is the attestation binding those keys to a TEE. The
//!   verified live keys are recorded in `attested_keys_are_the_shape_this_
//!   chain_verifies` so the shape this file exercises is pinned against them.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use ring::signature::{Ed25519KeyPair, KeyPair as _};
use sha2::{Digest as _, Sha256};
use sha3::Keccak256;
use tower::ServiceExt as _;

use trace_commons_protocol::admission::{
    AdmissionBinding, AdmissionEvidence, EVIDENCE_HEADER, REQUEST_METADATA_KEY, SIGNATURE_HEADER,
    is_hash,
};
use trace_commons_protocol::trace_contribution::{
    ConsentScope, RawTraceCaptureTurn, RawTraceContribution, RecordedTraceContributionOptions,
    ResidualPiiRisk, TraceAllowedUse, TraceContributionEventType,
};

use trace_commons_server::admission_evidence::{AdmissionProviderTrust, verify_admission_evidence};
use trace_commons_server::near_attestation::receipt::{
    ReceiptAlgo, ReceiptPayload, ReceiptSignatureKind,
};
use trace_commons_server::redaction_witness::certificate::{
    CertificateDetails, WitnessCertificate,
};
use trace_commons_server::redaction_witness::verification::{
    WitnessPin, verify_witness_certificate,
};
use trace_commons_server::witness_service::http::{
    WITNESS_CERTIFICATE_HEADER, WITNESS_SIGNATURE_HEADER, WitnessLoadBound, witness_router,
};
use trace_commons_server::witness_service::surface::WitnessService;
use trace_commons_server::witness_service::{
    DeterministicRedaction, Enclave, PipelineContributionRedaction, SeamUnavailable, Signer,
};

// The client's own types, deliberately. `GrantedConsent` exists on both sides
// of this wire and they are distinct Rust types; `witness_request_body` takes
// the contributor's, which is the point -- this test drives the client's
// encoder with the client's types, not a server-side lookalike.
use trace_commons_contributor::witness::transport::{
    AdmissionHeaders, GrantedConsent, WitnessedEnvelope, verify_certificate, witness_request_body,
};

// ---------------------------------------------------------------------------
// Witness fixtures: a real secp256k1 signer, a stand-in enclave.
// ---------------------------------------------------------------------------

struct FixtureSigner(k256::ecdsa::SigningKey);

impl FixtureSigner {
    fn new(seed: &str) -> Self {
        Self(
            k256::ecdsa::SigningKey::from_slice(&Keccak256::digest(seed.as_bytes()))
                .expect("seed is a valid scalar"),
        )
    }
    fn address(&self) -> String {
        use k256::elliptic_curve::sec1::ToEncodedPoint as _;
        let point = self.0.verifying_key().to_encoded_point(false);
        format!(
            "0x{}",
            hex::encode(&Keccak256::digest(&point.as_bytes()[1..])[12..])
        )
    }
}

impl Signer for FixtureSigner {
    fn sign_eip191(&self, message: &[u8]) -> Result<String, SeamUnavailable> {
        let mut prefixed =
            format!("\x19Ethereum Signed Message:\n{}", message.len()).into_bytes();
        prefixed.extend_from_slice(message);
        let (signature, recovery) = self
            .0
            .sign_prehash_recoverable(&Keccak256::digest(&prefixed))
            .map_err(|_| SeamUnavailable)?;
        let mut out = signature.to_bytes().to_vec();
        out.push(27 + recovery.to_byte());
        Ok(format!("0x{}", hex::encode(out)))
    }
}

const MEASUREMENT: &str = "mrtd=fixture-witness-measurement";

/// Returns bytes where a real deployment returns a DCAP quote. Nothing in this
/// file verifies a quote, so nothing here rests on it.
struct FixtureEnclave(String);

#[async_trait]
impl Enclave for FixtureEnclave {
    fn signing_address(&self) -> &str {
        &self.0
    }
    async fn measurement(&self) -> Result<String, SeamUnavailable> {
        Ok(MEASUREMENT.into())
    }
    async fn attestation_quote(&self, _nonce: &[u8]) -> Result<Vec<u8>, SeamUnavailable> {
        Ok(vec![0xab; 8])
    }
}

/// A witness wired as the binary wires it, with `trust` deciding whether the
/// admission policy is installed at all -- which is the same `Option` the
/// binary's `TRACE_COMMONS_WITNESS_ADMISSION_PROVIDER_SIGNERS` check produces.
fn witness(trust: Option<AdmissionProviderTrust>) -> (axum::Router, String) {
    let signer = Arc::new(FixtureSigner::new("witness-admission-chain"));
    let address = signer.address();
    let mut service = WitnessService::new(
        Arc::new(DeterministicRedaction::new(Vec::new())),
        signer.clone() as Arc<dyn Signer>,
        Arc::new(FixtureEnclave(address.clone())),
        1024 * 1024,
    )
    .with_contribution_redactor(Arc::new(PipelineContributionRedaction::deterministic_only(
        Vec::new(),
    )));
    if let Some(trust) = trust {
        service = service.with_admission_provider_trust(trust);
    }
    (
        witness_router(
            Arc::new(service),
            WitnessLoadBound::new(4, std::time::Duration::from_secs(30)),
        ),
        address,
    )
}

// ---------------------------------------------------------------------------
// Provider fixtures: real Ed25519 over the real receipt texts.
// ---------------------------------------------------------------------------

struct Provider {
    keypair: Ed25519KeyPair,
    public_hex: String,
}

impl Provider {
    fn new(seed: [u8; 32]) -> Self {
        let keypair = Ed25519KeyPair::from_seed_unchecked(&seed).expect("fixture seed");
        let public_hex = hex::encode(keypair.public_key().as_ref());
        Self { keypair, public_hex }
    }

    /// A receipt over these exact bodies. `bound_model` present gives the
    /// three-part provider-TEE text that commits to a model; absent gives the
    /// two-part gateway text that commits to no model at all.
    fn receipt(
        &self,
        bound_model: Option<&str>,
        kind: ReceiptSignatureKind,
        request: &str,
        response: &str,
    ) -> ReceiptPayload {
        let digests = format!(
            "{}:{}",
            hex::encode(Sha256::digest(request.as_bytes())),
            hex::encode(Sha256::digest(response.as_bytes()))
        );
        let text = match bound_model {
            Some(model) => format!("{model}:{digests}"),
            None => digests,
        };
        ReceiptPayload {
            signature: hex::encode(self.keypair.sign(text.as_bytes()).as_ref()),
            signing_address: self.public_hex.clone(),
            signing_algo: ReceiptAlgo::Ed25519,
            signature_kind: kind,
            text,
        }
    }
}

const MODEL: &str = "Qwen/Qwen3.8-27B";

/// An account anchor drawn from the OS RNG, so it is a function of nothing in
/// this file. The tenant id is deliberately absent: the witness chain never
/// sees one, and V61 made it independent of the anchor anyway.
fn anchor() -> String {
    let mut bytes = [0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes).expect("os rng");
    let anchor = hex::encode(bytes);
    assert!(is_hash(&anchor));
    anchor
}

/// The client's side of one attested exchange: request body carrying the
/// binding, response body, and the raw contribution holding both.
struct Exchange {
    binding: AdmissionBinding,
    request_body: String,
    response_body: String,
    raw: RawTraceContribution,
}

fn exchange(anchor: &str) -> Exchange {
    let binding = AdmissionBinding {
        account_anchor_sha256: anchor.to_string(),
        nonce_hex: hex::encode([0x5au8; 32]),
        expires_at: chrono::Utc::now().timestamp() + 600,
    };
    let request_body = serde_json::json!({
        "model": MODEL,
        "metadata": {REQUEST_METADATA_KEY: binding.encode().expect("binding encodes")},
        "messages": [{"role": "user", "content": "explain the admission chain"}],
    })
    .to_string();
    let response_body =
        r#"{"choices":[{"message":{"role":"assistant","content":"it closes end to end"}}]}"#
            .to_string();

    let mut raw = RawTraceContribution::from_capture_turns(
        &[RawTraceCaptureTurn {
            user_input: "explain the admission chain".into(),
            response: None,
            tool_calls: Vec::new(),
            started_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
            state: Some("Completed".into()),
        }],
        RecordedTraceContributionOptions {
            include_message_text: true,
            consent_scopes: vec![ConsentScope::DebuggingEvaluation],
            ..Default::default()
        },
    );
    // The receipt profile binds exactly one exchange; `verify_admission_call`
    // refuses anything else, so the contribution is that one event.
    let mut event = raw.events.pop().expect("a capture turn yields an event");
    event.event_type = TraceContributionEventType::HttpExchange;
    event.structured_payload = serde_json::json!({
        "request": {"method": "POST", "body": request_body},
        "response": {"status": 200},
    });
    event.content = Some(response_body.clone());
    raw.events = vec![event];

    Exchange {
        binding,
        request_body,
        response_body,
        raw,
    }
}

fn granted() -> GrantedConsent {
    GrantedConsent {
        scopes: vec![ConsentScope::DebuggingEvaluation],
        uses: vec![TraceAllowedUse::Debugging],
    }
}

/// What the witness returned: status, and on success the exact bytes and
/// headers a client would forward to ingest untouched.
struct Witnessed {
    status: StatusCode,
    envelope_bytes: Vec<u8>,
    certificate_json: String,
    certificate_signature: String,
    evidence_json: String,
    evidence_signature: String,
}

/// Drive the real route with bytes the real client encoder produced.
async fn post_admission(router: &axum::Router, body: Vec<u8>) -> Witnessed {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/witness/admission")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    let status = response.status();
    let header = |name: &str| -> String {
        response
            .headers()
            .get(name)
            .map(|v| v.to_str().expect("header is ascii").to_string())
            .unwrap_or_default()
    };
    let certificate_json = header(WITNESS_CERTIFICATE_HEADER);
    let certificate_signature = header(WITNESS_SIGNATURE_HEADER);
    let evidence_json = header(EVIDENCE_HEADER);
    let evidence_signature = header(SIGNATURE_HEADER);
    let envelope_bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("body reads")
        .to_vec();
    Witnessed {
        status,
        envelope_bytes,
        certificate_json,
        certificate_signature,
        evidence_json,
        evidence_signature,
    }
}

/// Rebuild the server-side certificate from the wire header, as ingest's
/// `witness_headers` does, so `verify_witness_certificate` can be run over it.
fn certificate_from_wire(certificate_json: &str) -> WitnessCertificate {
    let value: serde_json::Value =
        serde_json::from_str(certificate_json).expect("certificate header is JSON");
    let verdict = match value["residual_risk_verdict"]
        .as_str()
        .expect("verdict present")
    {
        "low" => ResidualPiiRisk::Low,
        "medium" => ResidualPiiRisk::Medium,
        "high" => ResidualPiiRisk::High,
        other => panic!("unknown verdict {other}"),
    };
    WitnessCertificate::from_wire(
        value["redacted_sha256"]
            .as_str()
            .expect("digest present")
            .to_string(),
        CertificateDetails {
            residual_risk_verdict: verdict,
            redaction_policy_version: value["redaction_policy_version"]
                .as_str()
                .expect("policy present")
                .to_string(),
            witness_measurement: value["witness_measurement"]
                .as_str()
                .expect("measurement present")
                .to_string(),
            timestamp: value["timestamp"].as_i64().expect("timestamp present"),
        },
    )
}

fn trust_for(provider_tee: Vec<String>, gateway: Vec<String>) -> AdmissionProviderTrust {
    AdmissionProviderTrust::new(provider_tee, gateway, [MODEL.to_string()], 1)
        .expect("fixture provider policy")
}

// ---------------------------------------------------------------------------
// 1. The happy path closes.
// ---------------------------------------------------------------------------

/// A client-encoded request, admitted by the real witness route, produces
/// evidence the real ingest verifier accepts -- over the same bytes and the
/// same binding at both ends.
#[tokio::test]
async fn a_client_request_admitted_by_the_witness_is_admitted_by_ingest() {
    let provider = Provider::new([7u8; 32]);
    let anchor = anchor();
    let exchange = exchange(&anchor);
    let receipt = provider.receipt(
        Some(MODEL),
        ReceiptSignatureKind::ProviderTee,
        &exchange.request_body,
        &exchange.response_body,
    );
    let (router, witness_address) = witness(Some(trust_for(
        vec![provider.public_hex.clone()],
        Vec::new(),
    )));

    // Step 1: the client's own encoder, not a document spelled here.
    let body = witness_request_body(&exchange.raw, &granted(), Some(&receipt))
        .expect("the client encodes its request");

    // Step 2: the real route.
    let witnessed = post_admission(&router, body).await;
    assert_eq!(
        witnessed.status,
        StatusCode::OK,
        "the witness refused a request it should admit: {}",
        String::from_utf8_lossy(&witnessed.envelope_bytes)
    );

    // Step 3: the client's own response verification, including its
    // admission-header checks, over what the route actually returned.
    let returned = WitnessedEnvelope {
        envelope_bytes: witnessed.envelope_bytes.clone(),
        certificate_json: witnessed.certificate_json.clone(),
        signature_hex: witnessed.certificate_signature.clone(),
        admission: Some(AdmissionHeaders {
            evidence_json: witnessed.evidence_json.clone(),
            signature_hex: witnessed.evidence_signature.clone(),
        }),
    };
    verify_certificate(&returned, &witness_address)
        .expect("the client must accept the certificate its own witness issued");

    // Step 4: ingest's verifiers, over the headers the witness emitted.
    let evidence: AdmissionEvidence =
        serde_json::from_str(&witnessed.evidence_json).expect("ingest parses the evidence header");
    let pin = WitnessPin::new(&witness_address, [MEASUREMENT.to_string()]).expect("pin");
    let verified = verify_witness_certificate(
        certificate_from_wire(&witnessed.certificate_json),
        &witnessed.certificate_signature,
        Some(&pin),
        &witnessed.envelope_bytes,
    )
    .expect("ingest must verify the certificate the witness issued");
    verify_admission_evidence(
        &evidence,
        &witnessed.evidence_signature,
        &verified,
        &pin,
        &trust_for(vec![provider.public_hex.clone()], Vec::new()),
        &anchor,
        chrono::Utc::now().timestamp(),
    )
    .expect("ingest must admit evidence its witness certified");

    // The same bytes and the same binding at both ends, checked against the
    // client's inputs rather than against anything the witness reported.
    assert_eq!(evidence.account_anchor_sha256, anchor);
    assert_eq!(
        evidence.challenge_sha256,
        exchange.binding.digest().expect("binding digests")
    );
    assert_eq!(evidence.expires_at, exchange.binding.expires_at);
    assert_eq!(
        evidence.request_sha256,
        hex::encode(Sha256::digest(exchange.request_body.as_bytes()))
    );
    assert_eq!(
        evidence.response_sha256,
        hex::encode(Sha256::digest(exchange.response_body.as_bytes()))
    );
    assert_eq!(evidence.provider_signer, provider.public_hex);
    assert_eq!(
        evidence.artifact_sha256,
        hex::encode(Sha256::digest(&witnessed.envelope_bytes)),
        "the evidence must bind the bytes the response actually returned"
    );
    assert_eq!(evidence.witness_measurement, MEASUREMENT);
}

// ---------------------------------------------------------------------------
// 2. A signer the witness trusts and ingest does not.
// ---------------------------------------------------------------------------

/// Ingest's provider check is defence in depth over the witness's, so it has
/// to be shown refusing something the witness admitted -- not merely shown
/// agreeing with it.
#[tokio::test]
async fn ingest_refuses_a_provider_its_own_policy_does_not_name() {
    let provider = Provider::new([11u8; 32]);
    let anchor = anchor();
    let exchange = exchange(&anchor);
    let receipt = provider.receipt(
        Some(MODEL),
        ReceiptSignatureKind::ProviderTee,
        &exchange.request_body,
        &exchange.response_body,
    );
    let (router, witness_address) = witness(Some(trust_for(
        vec![provider.public_hex.clone()],
        Vec::new(),
    )));
    let witnessed = post_admission(
        &router,
        witness_request_body(&exchange.raw, &granted(), Some(&receipt)).expect("client encodes"),
    )
    .await;
    assert_eq!(
        witnessed.status,
        StatusCode::OK,
        "the witness admitted this one; that is the premise of the test"
    );

    let evidence: AdmissionEvidence =
        serde_json::from_str(&witnessed.evidence_json).expect("evidence parses");
    let pin = WitnessPin::new(&witness_address, [MEASUREMENT.to_string()]).expect("pin");
    let verified = verify_witness_certificate(
        certificate_from_wire(&witnessed.certificate_json),
        &witnessed.certificate_signature,
        Some(&pin),
        &witnessed.envelope_bytes,
    )
    .expect("the certificate itself is sound");

    // Ingest pinned a different provider. The witness signature over the
    // evidence still verifies; the provider membership is what refuses.
    let other = Provider::new([12u8; 32]);
    assert_ne!(other.public_hex, provider.public_hex);
    assert!(
        verify_admission_evidence(
            &evidence,
            &witnessed.evidence_signature,
            &verified,
            &pin,
            &trust_for(vec![other.public_hex], Vec::new()),
            &anchor,
            chrono::Utc::now().timestamp(),
        )
        .is_err(),
        "ingest admitted a provider its own policy never named, so its \
         re-check is not independent of the witness's"
    );
}

// ---------------------------------------------------------------------------
// 3. The signature_kind split.
// ---------------------------------------------------------------------------

/// Neither kind of receipt may be admitted against the other kind's key set.
///
/// The gateway case is the common one, not an edge: the Responses API returns
/// a `gateway` receipt and `routing/receipt.rs` says Codex speaks only that
/// API, so most real sessions arrive in the form that binds no model.
///
/// Both refusals are the **witness's**. Ingest cannot make this distinction
/// and does not claim to: `AdmissionEvidence` carries no signature kind, and
/// `AdmissionProviderTrust::accepts` is documented as kind-agnostic precisely
/// because the kind was already decided inside the witness. So this is one
/// control at one place, and a test asserting ingest re-checks it would be
/// asserting something the design deliberately does not do.
#[tokio::test]
async fn neither_receipt_kind_is_admitted_against_the_other_kinds_keys() {
    let provider = Provider::new([13u8; 32]);
    let anchor = anchor();
    let exchange = exchange(&anchor);

    // A provider-TEE receipt offered where only gateway keys are pinned.
    let (gateway_only, _) = witness(Some(
        AdmissionProviderTrust::new(
            [Provider::new([14u8; 32]).public_hex],
            [provider.public_hex.clone()],
            [MODEL.to_string()],
            1,
        )
        .expect("policy"),
    ));
    let provider_tee = provider.receipt(
        Some(MODEL),
        ReceiptSignatureKind::ProviderTee,
        &exchange.request_body,
        &exchange.response_body,
    );
    assert_eq!(
        post_admission(
            &gateway_only,
            witness_request_body(&exchange.raw, &granted(), Some(&provider_tee))
                .expect("client encodes"),
        )
        .await
        .status,
        StatusCode::FORBIDDEN,
        "a provider-TEE receipt was admitted against a gateway key"
    );

    // A gateway receipt offered where only provider-TEE keys are pinned. This
    // is the shape every Codex session produces.
    let (provider_only, _) = witness(Some(trust_for(
        vec![provider.public_hex.clone()],
        Vec::new(),
    )));
    let gateway = provider.receipt(
        None,
        ReceiptSignatureKind::Gateway,
        &exchange.request_body,
        &exchange.response_body,
    );
    assert_eq!(
        post_admission(
            &provider_only,
            witness_request_body(&exchange.raw, &granted(), Some(&gateway))
                .expect("client encodes"),
        )
        .await
        .status,
        StatusCode::FORBIDDEN,
        "a gateway receipt was admitted against a provider-TEE key"
    );

    // Positive control: the same gateway receipt, against a gateway key, is
    // admitted -- so the two refusals above are about the key set and not
    // about the gateway form being unusable.
    let (gateway_pinned, _) = witness(Some(
        AdmissionProviderTrust::new(
            [Provider::new([15u8; 32]).public_hex],
            [provider.public_hex.clone()],
            [MODEL.to_string()],
            1,
        )
        .expect("policy"),
    ));
    assert_eq!(
        post_admission(
            &gateway_pinned,
            witness_request_body(&exchange.raw, &granted(), Some(&gateway))
                .expect("client encodes"),
        )
        .await
        .status,
        StatusCode::OK,
        "a gateway receipt against a gateway key must be admitted, or the \
         refusals above prove nothing about the split"
    );
}

// ---------------------------------------------------------------------------
// 4. The deployed state: no admission policy configured.
// ---------------------------------------------------------------------------

/// The pilot sets no `TRACE_COMMONS_WITNESS_ADMISSION_PROVIDER_SIGNERS`, so
/// `bin/trace-commons-witness.rs` installs no `AdmissionProviderTrust` and the
/// witness runs with `admission_provider_trust: None`. That is the deployed
/// configuration right now, and this pins that it refuses rather than falling
/// open to the ordinary contribution path.
#[tokio::test]
async fn a_witness_with_no_admission_policy_refuses_rather_than_falling_open() {
    let provider = Provider::new([17u8; 32]);
    let anchor = anchor();
    let exchange = exchange(&anchor);
    let receipt = provider.receipt(
        Some(MODEL),
        ReceiptSignatureKind::ProviderTee,
        &exchange.request_body,
        &exchange.response_body,
    );
    let (router, _) = witness(None);
    let witnessed = post_admission(
        &router,
        witness_request_body(&exchange.raw, &granted(), Some(&receipt)).expect("client encodes"),
    )
    .await;

    assert_eq!(witnessed.status, StatusCode::FORBIDDEN);
    assert!(
        witnessed.evidence_json.is_empty() && witnessed.evidence_signature.is_empty(),
        "an unconfigured witness emitted admission headers"
    );
    // And it is a refusal, not a silent downgrade to the ordinary route: no
    // certificate is issued either.
    assert!(
        witnessed.certificate_json.is_empty(),
        "an unconfigured witness certified a contribution on the admission route"
    );
}

/// The live NEAR AI keys this chain's shape was checked against.
///
/// Both are 64 lowercase hex characters -- the form `is_hash` requires and the
/// form `AdmissionProviderTrust::new` accepts -- and they are distinct, which
/// is what lets the two sets stay disjoint. Recorded as a shape check rather
/// than used as a fixture: this test verifies no signature, because the
/// matching private keys live in a TEE.
#[test]
fn attested_keys_are_the_shape_this_chain_verifies() {
    let provider_tee = "73cf225ab4f09154ad8b299d4ac89425c7f25468a42ba9a87d09fcd4e87b8bf5";
    let gateway = "cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6";
    assert!(is_hash(provider_tee) && is_hash(gateway));
    assert_ne!(provider_tee, gateway);
    AdmissionProviderTrust::new(
        [provider_tee.to_string()],
        [gateway.to_string()],
        [MODEL.to_string()],
        1,
    )
    .expect("the live keys are a policy this deployment could actually run");
}
