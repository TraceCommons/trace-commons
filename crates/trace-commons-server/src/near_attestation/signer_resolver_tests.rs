// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tests for the server-side receipt-signer resolver.
//!
//! Split into a sibling file the way `trace-commons-ingest.rs` splits its
//! tests: `signer_resolver.rs` is documentation-heavy and the two read better
//! apart.
//!
//! Everything here runs offline against checked-in captures. Where a captured
//! report cannot exercise a step -- because the checked-in collateral was
//! fetched for the ECDSA gateway platform and does not verify a model
//! enclave's quote, which is **measured** below rather than assumed -- a
//! **synthetic** report is built over a quote that does verify, and it is
//! labelled synthetic wherever it appears.
//!
//! Every test here asserts a **refusal**, not merely the absence of an
//! acceptance: the accepted set must be empty *and* the refusal label must
//! name the reason. An assertion that only checked "not accepted" would pass
//! against a resolver that accepted nothing ever, which is the failure mode
//! that looks green.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use super::*;
use crate::near_attestation::client::AttestationStep;
use crate::near_attestation::quote::parse_collateral;

const COLLATERAL: &str = include_str!(
    "../../../trace-commons-attestation/tests/fixtures/near_ai_attestation_collateral.json"
);
/// The single-enclave ECDSA report. Its quote is the only checked-in quote the
/// checked-in collateral verifies.
const ECDSA_REPORT: &str = include_str!(
    "../../../trace-commons-attestation/tests/fixtures/near_ai_attestation_report.json"
);
const ED25519_REPORT: &str = include_str!(
    "../../../trace-commons-attestation/tests/fixtures/near_ai_model_attestation_report_ed25519.json"
);

const FIRST_MODEL: &str = "Qwen/Qwen3.6-35B-A3B-FP8";

/// See `quote::tests::FIXTURE_CAPTURED_AT`. `verify_quote` consults no clock
/// but the one it is handed, so these tests fail on a code change, never on a
/// calendar date.
const FIXTURE_CAPTURED_AT: u64 = 1_788_264_000;

/// A nonce this server "generated". Deliberately not any fixture's.
const OUR_NONCE: &str = "9f3c1d2e4b5a60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9";

/// A 32-byte ed25519 key that appears in no fixture and no verified quote.
const FOREIGN_KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";

struct FixedNonce(String);

impl ResolverNonceSource for FixedNonce {
    fn nonce(&self) -> String {
        self.0.clone()
    }
}

/// A stub endpoint. Counts its calls and records what it was asked for, so a
/// test can assert on the *challenge* and on the single-flight floor, not only
/// on the verdict.
struct StubEndpoint {
    model: String,
    report: Mutex<Result<String, AttestationClientError>>,
    collateral: Mutex<Option<String>>,
    seen_nonces: Mutex<Vec<String>>,
    seen_quotes: Mutex<Vec<Vec<u8>>>,
    report_calls: AtomicUsize,
}

impl StubEndpoint {
    fn serving(model: &str, report: &str) -> Self {
        Self {
            model: model.to_string(),
            report: Mutex::new(Ok(report.to_string())),
            collateral: Mutex::new(Some(COLLATERAL.to_string())),
            seen_nonces: Mutex::new(Vec::new()),
            seen_quotes: Mutex::new(Vec::new()),
            report_calls: AtomicUsize::new(0),
        }
    }

    fn refusing(model: &str) -> Self {
        let stub = Self::serving(model, "");
        stub.start_refusing();
        stub
    }

    /// The endpoint stops answering. Used to drive a *second* refresh on a
    /// resolver that already installed a set, which is the only way to observe
    /// the freeze rather than the absence of an install.
    fn start_refusing(&self) {
        *self.report.lock().unwrap() = Err(AttestationClientError::HttpStatus {
            step: AttestationStep::Report,
            status: 503,
        });
    }

    /// The endpoint starts answering with a different report.
    fn now_serving(&self, report: &str) {
        *self.report.lock().unwrap() = Ok(report.to_string());
    }

    /// The endpoint answers, but collateral cannot be had for its quotes.
    fn without_collateral(self) -> Self {
        *self.collateral.lock().unwrap() = None;
        self
    }

    fn nonces_seen(&self) -> Vec<String> {
        self.seen_nonces.lock().unwrap().clone()
    }

    fn quotes_seen(&self) -> Vec<Vec<u8>> {
        self.seen_quotes.lock().unwrap().clone()
    }

    fn report_calls(&self) -> usize {
        self.report_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl AttestedReportSource for StubEndpoint {
    fn model(&self) -> &str {
        &self.model
    }

    async fn fetch_ed25519_report_json(
        &self,
        nonce: &str,
    ) -> Result<String, AttestationClientError> {
        self.report_calls.fetch_add(1, Ordering::SeqCst);
        self.seen_nonces.lock().unwrap().push(nonce.to_string());
        self.report.lock().unwrap().clone()
    }

    async fn fetch_collateral_for(
        &self,
        quote: &[u8],
    ) -> Result<Collateral, AttestationClientError> {
        self.seen_quotes.lock().unwrap().push(quote.to_vec());
        let collateral = self.collateral.lock().unwrap().clone();
        let Some(collateral) = collateral else {
            return Err(AttestationClientError::HttpStatus {
                step: AttestationStep::Collateral,
                status: 502,
            });
        };
        parse_collateral(&collateral).map_err(|_| AttestationClientError::MalformedResponse {
            step: AttestationStep::Collateral,
            detail_hash: "0000000000000000".to_string(),
        })
    }
}

/// A stub whose endpoint is shared with the test, so a test can both drive a
/// resolver and read the stub's counters. `Arc` is not usable directly because
/// the resolver takes a `Box<dyn ..>`, so the shared half is a second handle
/// that forwards.
struct SharedEndpoint(std::sync::Arc<StubEndpoint>);

#[async_trait]
impl AttestedReportSource for SharedEndpoint {
    fn model(&self) -> &str {
        self.0.model()
    }

    async fn fetch_ed25519_report_json(
        &self,
        nonce: &str,
    ) -> Result<String, AttestationClientError> {
        self.0.fetch_ed25519_report_json(nonce).await
    }

    async fn fetch_collateral_for(
        &self,
        quote: &[u8],
    ) -> Result<Collateral, AttestationClientError> {
        self.0.fetch_collateral_for(quote).await
    }
}

fn json(report: &str) -> serde_json::Value {
    serde_json::from_str(report).expect("fixture parses")
}

fn model_quote_bytes(report: &str) -> Vec<u8> {
    hex::decode(
        json(report)["model_attestations"][0]["intel_quote"]
            .as_str()
            .unwrap(),
    )
    .unwrap()
}

fn gateway_quote_bytes(report: &str) -> Vec<u8> {
    hex::decode(
        json(report)["gateway_attestation"]["intel_quote"]
            .as_str()
            .unwrap(),
    )
    .unwrap()
}

/// A **synthetic** ed25519 report built over the ECDSA capture's quote.
///
/// The quote is real and verifies against the checked-in collateral; what is
/// synthetic is the ed25519 framing around it. The signing key is taken to be
/// `report_data[0..32]` of that very quote, which is what an ed25519
/// attestation commits to, so the report is internally consistent **by
/// construction** rather than by a hand-written expectation.
fn synthetic_report(model: &str) -> (String, String) {
    let quote_hex = json(ECDSA_REPORT)["intel_quote"]
        .as_str()
        .unwrap()
        .to_string();
    let quote = hex::decode(&quote_hex).unwrap();
    let report_data = &quote[568..632];
    let key = hex::encode(&report_data[0..32]);
    let nonce = hex::encode(&report_data[32..64]);
    let entry = serde_json::json!({
        "model_name": model,
        "signing_algo": "ed25519",
        "signing_address": key,
        "request_nonce": nonce,
        "intel_quote": quote_hex,
    });
    let document = serde_json::json!({
        "gateway_attestation": entry,
        "model_attestations": [entry],
    });
    (document.to_string(), nonce)
}

/// A pin set built from the ECDSA capture's own verified registers.
///
/// Deliberately derived by running the real verification path, not by pasting
/// hex: a hand-written expectation here would be a second implementation of
/// the register read, agreeing with the code by construction.
fn synthetic_pins() -> ExpectedMeasurements {
    let quote = hex::decode(json(ECDSA_REPORT)["intel_quote"].as_str().unwrap()).unwrap();
    let collateral = parse_collateral(COLLATERAL).unwrap();
    let verified = verify_quote(&quote, &collateral, FIXTURE_CAPTURED_AT)
        .expect("the ECDSA capture's quote verifies against the checked-in collateral");
    let raw = format!(
        "mrtd={},rtmr0={},rtmr1={},rtmr2={},rtmr3={}",
        verified.mrtd, verified.rtmr[0], verified.rtmr[1], verified.rtmr[2], verified.rtmr[3]
    );
    ExpectedMeasurements::from_env_value(Some(&raw))
        .expect("the quote's own values are a valid pin set")
        .expect("a non-empty value yields a set")
}

fn pins_for_both(expected: ExpectedMeasurements) -> ResolverMeasurementPins {
    ResolverMeasurementPins {
        model: Some(expected.clone()),
        gateway: Some(expected),
    }
}

fn resolver(
    source: StubEndpoint,
    nonce: &str,
    pins: ResolverMeasurementPins,
) -> ReportBackedResolver {
    ReportBackedResolver::new(
        Box::new(source),
        Box::new(FixedNonce(nonce.to_string())),
        pins,
        OperatorKeyPins::none(),
    )
}

/// The model half's accepted set, at the fixture's capture time.
fn model_keys(resolver: &ReportBackedResolver) -> Vec<String> {
    resolver.keys_for(
        ReceiptSignatureKind::ProviderTee,
        Some(FIRST_MODEL),
        FIXTURE_CAPTURED_AT,
    )
}

// ---------------------------------------------------------------------------
// The challenge is the server's
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_server_challenges_with_the_nonce_it_generated() {
    // The precondition, asserted rather than assumed: the capture binds a
    // nonce that is NOT the one this run generates. Without it the test would
    // pass against a resolver that echoed whatever the report said.
    let captured = json(ED25519_REPORT)["gateway_attestation"]["request_nonce"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(captured, OUR_NONCE, "the fixture must bind a foreign nonce");

    let stub = StubEndpoint::serving(FIRST_MODEL, ED25519_REPORT);
    let shared = std::sync::Arc::new(stub);
    let r = ReportBackedResolver::new(
        Box::new(SharedEndpoint(shared.clone())),
        Box::new(FixedNonce(OUR_NONCE.to_string())),
        pins_for_both(synthetic_pins()),
        OperatorKeyPins::none(),
    );
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(
        shared.nonces_seen(),
        vec![OUR_NONCE.to_string()],
        "the endpoint must be challenged with the nonce the source produced"
    );
    assert_eq!(
        outcome.model.refusal(),
        Some(RefreshRefusal::KeyBindingFailed),
        "a report bound to somebody else's nonce must yield no key"
    );
    assert!(model_keys(&r).is_empty(), "and nothing may be accepted");
}

#[tokio::test]
async fn rewriting_the_echoed_nonce_does_not_satisfy_the_binding() {
    // `request_nonce` is the provider's echo beside the quote. Rewrite every
    // echo to the nonce the server sent; the quotes still commit to the
    // original, and the resolver must still refuse. This is the check that
    // stops the exercise from being the endpoint marking its own homework.
    let mut document = json(ED25519_REPORT);
    document["gateway_attestation"]["request_nonce"] = serde_json::json!(OUR_NONCE);
    for entry in document["model_attestations"].as_array_mut().unwrap() {
        entry["request_nonce"] = serde_json::json!(OUR_NONCE);
    }
    let stub = StubEndpoint::serving(FIRST_MODEL, &document.to_string());
    let r = resolver(stub, OUR_NONCE, pins_for_both(synthetic_pins()));
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(
        outcome.model.refusal(),
        Some(RefreshRefusal::KeyBindingFailed)
    );
    assert!(model_keys(&r).is_empty());
}

#[tokio::test]
async fn a_malformed_nonce_never_reaches_the_network() {
    let stub = StubEndpoint::serving(FIRST_MODEL, ED25519_REPORT);
    let shared = std::sync::Arc::new(stub);
    let r = ReportBackedResolver::new(
        Box::new(SharedEndpoint(shared.clone())),
        Box::new(FixedNonce("not-a-nonce".to_string())),
        pins_for_both(synthetic_pins()),
        OperatorKeyPins::none(),
    );
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(shared.report_calls(), 0, "no call may be made at all");
    assert_eq!(
        outcome.model.refusal(),
        Some(RefreshRefusal::NonceSourceMalformed)
    );
    assert_eq!(
        outcome.gateway.refusal(),
        Some(RefreshRefusal::NonceSourceMalformed)
    );
    assert!(model_keys(&r).is_empty());
}

#[tokio::test]
async fn a_nonce_of_the_right_length_that_is_not_hex_never_reaches_the_network() {
    // 64 characters, so a length check alone lets it through, and `zz..` is
    // not hex. The refusal has to come from the decode.
    let stub = StubEndpoint::serving(FIRST_MODEL, ED25519_REPORT);
    let shared = std::sync::Arc::new(stub);
    let r = ReportBackedResolver::new(
        Box::new(SharedEndpoint(shared.clone())),
        Box::new(FixedNonce("z".repeat(64))),
        pins_for_both(synthetic_pins()),
        OperatorKeyPins::none(),
    );
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(shared.report_calls(), 0);
    assert_eq!(
        outcome.model.refusal(),
        Some(RefreshRefusal::NonceSourceMalformed)
    );
}

// ---------------------------------------------------------------------------
// Each failure mode refuses
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_report_refuses() {
    let stub = StubEndpoint::refusing(FIRST_MODEL);
    let r = resolver(stub, OUR_NONCE, pins_for_both(synthetic_pins()));
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(
        outcome.model.refusal(),
        Some(RefreshRefusal::ReportUnavailable)
    );
    assert_eq!(
        outcome.gateway.refusal(),
        Some(RefreshRefusal::ReportUnavailable)
    );
    assert!(model_keys(&r).is_empty());
    assert!(
        r.keys_for(ReceiptSignatureKind::Gateway, None, FIXTURE_CAPTURED_AT)
            .is_empty()
    );
}

#[tokio::test]
async fn a_malformed_report_refuses() {
    let stub = StubEndpoint::serving(FIRST_MODEL, "{not json");
    let r = resolver(stub, OUR_NONCE, pins_for_both(synthetic_pins()));
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(outcome.model.refusal(), Some(RefreshRefusal::ReportShape));
    assert_eq!(outcome.gateway.refusal(), Some(RefreshRefusal::ReportShape));
    assert!(model_keys(&r).is_empty());
}

#[tokio::test]
async fn an_ecdsa_report_carries_no_per_model_key_and_refuses() {
    // The ECDSA report is what `super::drill` fetches. It has no
    // `model_attestations` at all -- which is the reason the ed25519 fetch
    // exists -- so feeding one here must be a named refusal, not an empty pass.
    assert!(
        json(ECDSA_REPORT).get("model_attestations").is_none(),
        "precondition: the ECDSA capture carries no model attestations"
    );
    let stub = StubEndpoint::serving(FIRST_MODEL, ECDSA_REPORT);
    let r = resolver(stub, OUR_NONCE, pins_for_both(synthetic_pins()));
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(outcome.model.refusal(), Some(RefreshRefusal::ReportShape));
    assert!(model_keys(&r).is_empty());
}

#[tokio::test]
async fn an_ed25519_report_with_no_entry_for_this_model_refuses() {
    let (report, nonce) = synthetic_report("some/other-model");
    let stub = StubEndpoint::serving(FIRST_MODEL, &report);
    let r = resolver(stub, &nonce, pins_for_both(synthetic_pins()));
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(
        outcome.model.refusal(),
        Some(RefreshRefusal::ModelNotAttested),
        "a key for another model may not answer for this one"
    );
    assert!(model_keys(&r).is_empty());
}

#[tokio::test]
async fn an_entry_that_is_not_ed25519_refuses() {
    // `report_data[0..32]` of an ECDSA attestation is a 20-byte address and 12
    // zero bytes. Reading a key out of one would manufacture 32 bytes no
    // ed25519 signer holds, so the algorithm check has to be the thing that
    // refuses first.
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let mut document: serde_json::Value = serde_json::from_str(&report).unwrap();
    document["model_attestations"][0]["signing_algo"] = serde_json::json!("ecdsa");
    let stub = StubEndpoint::serving(FIRST_MODEL, &document.to_string());
    let r = resolver(stub, &nonce, pins_for_both(synthetic_pins()));
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(
        outcome.model.refusal(),
        Some(RefreshRefusal::KeyBindingFailed)
    );
    assert!(model_keys(&r).is_empty());
}

#[tokio::test]
async fn collateral_that_does_not_match_the_quote_refuses() {
    // Measured, and the reason collateral is fetched per quote. The
    // checked-in collateral was fetched for the ECDSA gateway platform;
    // against a model enclave's quote `dcap-qvl` reports an invalid QE report
    // signature, i.e. the PCK chain belongs to another machine. If a future
    // capture ever makes this verify, this assertion is what says so rather
    // than a silently weaker resolver.
    let nonce = json(ED25519_REPORT)["gateway_attestation"]["request_nonce"]
        .as_str()
        .unwrap()
        .to_string();
    let stub = StubEndpoint::serving(FIRST_MODEL, ED25519_REPORT);
    let shared = std::sync::Arc::new(stub);
    let r = ReportBackedResolver::new(
        Box::new(SharedEndpoint(shared.clone())),
        Box::new(FixedNonce(nonce)),
        pins_for_both(synthetic_pins()),
        OperatorKeyPins::none(),
    );
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(
        outcome.model.refusal(),
        Some(RefreshRefusal::QuoteUnverified),
        "a quote the fetched collateral does not verify may install no key"
    );
    assert!(model_keys(&r).is_empty());

    // And the quote sent for collateral was the model entry's, not the
    // gateway's -- the distinction the per-quote fetch exists for.
    let seen = shared.quotes_seen();
    assert_eq!(seen.first(), Some(&model_quote_bytes(ED25519_REPORT)));
    assert_ne!(
        seen.first(),
        Some(&gateway_quote_bytes(ED25519_REPORT)),
        "precondition: the two quotes differ, so this test can fail"
    );
}

#[tokio::test]
async fn collateral_that_cannot_be_fetched_refuses() {
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let stub = StubEndpoint::serving(FIRST_MODEL, &report).without_collateral();
    let r = resolver(stub, &nonce, pins_for_both(synthetic_pins()));
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(
        outcome.model.refusal(),
        Some(RefreshRefusal::CollateralUnavailable)
    );
    assert!(model_keys(&r).is_empty());
}

#[tokio::test]
async fn a_quote_that_does_not_verify_refuses() {
    // The synthetic report, whose quote does verify, with the quote swapped
    // for one that does not. Everything else about the report is unchanged,
    // so the only thing that can refuse is the DCAP verification.
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let mut document: serde_json::Value = serde_json::from_str(&report).unwrap();
    let foreign = hex::encode(gateway_quote_bytes(ED25519_REPORT));
    document["model_attestations"][0]["intel_quote"] = serde_json::json!(foreign);
    let stub = StubEndpoint::serving(FIRST_MODEL, &document.to_string());
    let r = resolver(stub, &nonce, pins_for_both(synthetic_pins()));
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert!(model_keys(&r).is_empty());
    assert!(
        matches!(
            outcome.model.refusal(),
            // The swapped quote commits to a different key, so the JSON-level
            // reader refuses it before verification is even reached. Either
            // label is a refusal; what may never happen is an install.
            Some(RefreshRefusal::KeyBindingFailed) | Some(RefreshRefusal::QuoteUnverified)
        ),
        "got {:?}",
        outcome.model
    );
    assert!(!outcome.model.installed());
}

#[tokio::test]
async fn a_measurement_outside_the_pin_set_refuses() {
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let wrong = ExpectedMeasurements::from_env_value(Some(&format!("mrtd={}", "ab".repeat(48))))
        .unwrap()
        .unwrap();
    let stub = StubEndpoint::serving(FIRST_MODEL, &report);
    let r = resolver(stub, &nonce, pins_for_both(wrong));
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(
        outcome.model.refusal(),
        Some(RefreshRefusal::MeasurementsNotPinned),
        "an enclave whose image we did not pin may install no key"
    );
    assert!(model_keys(&r).is_empty());
}

#[tokio::test]
async fn nothing_pinned_refuses_rather_than_skipping_the_check() {
    // The default. An operator who pinned no measurements has named no image,
    // and "no image named" is not "every image".
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let stub = StubEndpoint::serving(FIRST_MODEL, &report);
    let r = resolver(stub, &nonce, ResolverMeasurementPins::default());
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(
        outcome.model.refusal(),
        Some(RefreshRefusal::MeasurementsNotPinned)
    );
    assert_eq!(
        outcome.gateway.refusal(),
        Some(RefreshRefusal::MeasurementsNotPinned)
    );
    assert!(model_keys(&r).is_empty());
}

#[tokio::test]
async fn a_key_the_verified_quote_does_not_commit_to_refuses() {
    // A report whose JSON is internally consistent about a key the quote does
    // not carry. Both the claimed key and the echoed `report_data` are
    // rewritten so the JSON-level reader is satisfied; only the comparison
    // against the *verified* quote can catch it.
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let mut document: serde_json::Value = serde_json::from_str(&report).unwrap();
    let forged_report_data = format!("{FOREIGN_KEY}{nonce}");
    for path in ["gateway_attestation", "model_attestations"] {
        let target = if path == "model_attestations" {
            &mut document["model_attestations"][0]
        } else {
            &mut document["gateway_attestation"]
        };
        target["signing_address"] = serde_json::json!(FOREIGN_KEY);
        target["report_data"] = serde_json::json!(forged_report_data);
    }
    let stub = StubEndpoint::serving(FIRST_MODEL, &document.to_string());
    let r = resolver(stub, &nonce, pins_for_both(synthetic_pins()));
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert!(!outcome.model.installed(), "got {:?}", outcome.model);
    assert!(model_keys(&r).is_empty());
    assert!(
        !r.keys_for(ReceiptSignatureKind::Gateway, None, FIXTURE_CAPTURED_AT)
            .contains(&FOREIGN_KEY.to_string()),
        "a key nothing verified may never be accepted"
    );
}

// ---------------------------------------------------------------------------
// The green path, and what it installs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_verified_measurement_pinned_report_installs_the_quotes_own_key() {
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let stub = StubEndpoint::serving(FIRST_MODEL, &report);
    let r = resolver(stub, &nonce, pins_for_both(synthetic_pins()));
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;

    assert!(outcome.model.installed(), "got {:?}", outcome.model);
    assert!(outcome.gateway.installed(), "got {:?}", outcome.gateway);

    // The installed key is the one the *verified quote* commits to, read from
    // the quote here rather than copied from the report's JSON.
    let quote = hex::decode(json(ECDSA_REPORT)["intel_quote"].as_str().unwrap()).unwrap();
    let expected = hex::encode(&quote[568..600]);
    assert_eq!(model_keys(&r), vec![expected.clone()]);
    assert_eq!(
        r.keys_for(ReceiptSignatureKind::Gateway, None, FIXTURE_CAPTURED_AT),
        vec![expected]
    );
}

#[tokio::test]
async fn an_unrecognised_signature_kind_is_answered_with_nothing() {
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let stub = StubEndpoint::serving(FIRST_MODEL, &report);
    let r = resolver(stub, &nonce, pins_for_both(synthetic_pins()));
    r.refresh(FIXTURE_CAPTURED_AT).await;

    assert!(
        r.keys_for(
            ReceiptSignatureKind::Unrecognised,
            Some(FIRST_MODEL),
            FIXTURE_CAPTURED_AT
        )
        .is_empty(),
        "a kind naming no key source is not a licence to try every key"
    );
}

#[tokio::test]
async fn a_model_the_report_never_attested_is_answered_with_nothing() {
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let stub = StubEndpoint::serving(FIRST_MODEL, &report);
    let r = resolver(stub, &nonce, pins_for_both(synthetic_pins()));
    r.refresh(FIXTURE_CAPTURED_AT).await;

    assert!(
        !model_keys(&r).is_empty(),
        "precondition: the model resolves"
    );
    assert!(
        r.keys_for(
            ReceiptSignatureKind::ProviderTee,
            Some("some/other-model"),
            FIXTURE_CAPTURED_AT
        )
        .is_empty()
    );
    assert!(
        r.keys_for(ReceiptSignatureKind::ProviderTee, None, FIXTURE_CAPTURED_AT)
            .is_empty(),
        "a receipt binding no model can be placed against no set"
    );
}

// ---------------------------------------------------------------------------
// Monotonicity: a failure never widens
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_fresh_resolver_accepts_nothing() {
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let stub = StubEndpoint::serving(FIRST_MODEL, &report);
    let r = resolver(stub, &nonce, pins_for_both(synthetic_pins()));

    assert!(
        model_keys(&r).is_empty(),
        "a resolver that has fetched nothing knows nobody"
    );
    assert!(
        r.keys_for(ReceiptSignatureKind::Gateway, None, FIXTURE_CAPTURED_AT)
            .is_empty()
    );
    assert!(r.is_enforcing(), "and it enforces from the first request");
    assert!(r.needs_refresh(FIXTURE_CAPTURED_AT));
}

#[tokio::test]
async fn a_failed_refresh_leaves_the_installed_set_alone() {
    // Install, then fail on the same resolver. The set must be exactly what it
    // was: a failure may subtract or freeze, never widen, and freezing is what
    // this asserts. Driven by making the endpoint stop answering, so the
    // second refresh really runs against state the first one wrote.
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let shared = std::sync::Arc::new(StubEndpoint::serving(FIRST_MODEL, &report));
    let r = ReportBackedResolver::new(
        Box::new(SharedEndpoint(shared.clone())),
        Box::new(FixedNonce(nonce)),
        pins_for_both(synthetic_pins()),
        OperatorKeyPins::none(),
    );
    r.refresh(FIXTURE_CAPTURED_AT).await;
    let installed = model_keys(&r);
    assert_eq!(installed.len(), 1, "precondition: one key installed");

    shared.start_refusing();
    let outcome = r.refresh(FIXTURE_CAPTURED_AT + 1).await;

    assert_eq!(
        outcome.model.refusal(),
        Some(RefreshRefusal::ReportUnavailable)
    );
    assert_eq!(
        model_keys(&r),
        installed,
        "a failed refresh may neither widen the set nor discard it"
    );
}

#[tokio::test]
async fn a_refusing_endpoint_cannot_hold_a_set_open_past_the_ceiling() {
    // The other half of the freeze: frozen is not forever. A resolver that has
    // been unable to refresh for a day does not know who may sign, and the
    // fail-closed reading of that is nobody.
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let shared = std::sync::Arc::new(StubEndpoint::serving(FIRST_MODEL, &report));
    let r = ReportBackedResolver::new(
        Box::new(SharedEndpoint(shared.clone())),
        Box::new(FixedNonce(nonce)),
        pins_for_both(synthetic_pins()),
        OperatorKeyPins::none(),
    );
    r.refresh(FIXTURE_CAPTURED_AT).await;
    assert_eq!(model_keys(&r).len(), 1, "precondition");

    shared.start_refusing();
    let a_day_later = FIXTURE_CAPTURED_AT + HARD_CEILING_SECS + 1;
    r.refresh(a_day_later).await;

    assert!(
        r.keys_for(
            ReceiptSignatureKind::ProviderTee,
            Some(FIRST_MODEL),
            a_day_later
        )
        .is_empty(),
        "a set nothing has been able to renew for a day accepts nobody"
    );
}

#[tokio::test]
async fn a_rotation_replaces_the_old_key_rather_than_accumulating() {
    // A union would keep every key the endpoint has ever attested, which is
    // the one direction a refresh may not go: a rotated-out key must stop
    // being accepted.
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let shared = std::sync::Arc::new(StubEndpoint::serving(FIRST_MODEL, &report));
    let r = ReportBackedResolver::new(
        Box::new(SharedEndpoint(shared.clone())),
        Box::new(FixedNonce(nonce.clone())),
        pins_for_both(synthetic_pins()),
        OperatorKeyPins::none(),
    );
    r.refresh(FIXTURE_CAPTURED_AT).await;
    let first = model_keys(&r);
    assert_eq!(first.len(), 1, "precondition: one key installed");

    // The same report with a second, unverifiable entry appended. The whole
    // half must refuse: a report mixing a sound entry with one nothing
    // verified cannot be answered with the sound half.
    let mut document: serde_json::Value = serde_json::from_str(&report).unwrap();
    let mut forged = document["model_attestations"][0].clone();
    forged["signing_address"] = serde_json::json!(FOREIGN_KEY);
    forged["report_data"] = serde_json::json!(format!("{FOREIGN_KEY}{nonce}"));
    forged["intel_quote"] = serde_json::json!(hex::encode(gateway_quote_bytes(ED25519_REPORT)));
    document["model_attestations"]
        .as_array_mut()
        .unwrap()
        .push(forged);
    shared.now_serving(&document.to_string());

    let outcome = r.refresh(FIXTURE_CAPTURED_AT + 1).await;
    assert!(!outcome.model.installed(), "got {:?}", outcome.model);
    assert!(
        !model_keys(&r).contains(&FOREIGN_KEY.to_string()),
        "a key nothing verified may never join the accepted set"
    );
    assert_eq!(model_keys(&r), first, "and the sound set is left alone");
}

#[tokio::test]
async fn past_the_hard_ceiling_the_set_reads_empty() {
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let stub = StubEndpoint::serving(FIRST_MODEL, &report);
    let r = resolver(stub, &nonce, pins_for_both(synthetic_pins()));
    r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(
        model_keys(&r).len(),
        1,
        "precondition: something is installed"
    );
    assert_eq!(
        r.keys_for(
            ReceiptSignatureKind::ProviderTee,
            Some(FIRST_MODEL),
            FIXTURE_CAPTURED_AT + HARD_CEILING_SECS
        )
        .len(),
        1,
        "at the ceiling exactly, the set still stands"
    );
    assert!(
        r.keys_for(
            ReceiptSignatureKind::ProviderTee,
            Some(FIRST_MODEL),
            FIXTURE_CAPTURED_AT + HARD_CEILING_SECS + 1
        )
        .is_empty(),
        "one second past it, an expired set reads empty and refuses"
    );
    assert!(
        r.keys_for(
            ReceiptSignatureKind::Gateway,
            None,
            FIXTURE_CAPTURED_AT + HARD_CEILING_SECS + 1
        )
        .is_empty()
    );
}

// ---------------------------------------------------------------------------
// Freshness windows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn within_the_ttl_nothing_is_refetched() {
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let shared = std::sync::Arc::new(StubEndpoint::serving(FIRST_MODEL, &report));
    let r = ReportBackedResolver::new(
        Box::new(SharedEndpoint(shared.clone())),
        Box::new(FixedNonce(nonce)),
        pins_for_both(synthetic_pins()),
        OperatorKeyPins::none(),
    );
    r.refresh(FIXTURE_CAPTURED_AT).await;
    assert_eq!(shared.report_calls(), 1);

    assert!(
        r.refresh_if_stale(FIXTURE_CAPTURED_AT + FRESH_TTL_SECS - 1)
            .await
            .is_none(),
        "inside the TTL there is nothing to do"
    );
    assert_eq!(shared.report_calls(), 1);

    assert!(
        r.refresh_if_stale(FIXTURE_CAPTURED_AT + FRESH_TTL_SECS)
            .await
            .is_some(),
        "at the TTL it refetches"
    );
    assert_eq!(shared.report_calls(), 2);
}

#[tokio::test]
async fn a_signer_miss_refetches_at_most_once_a_minute() {
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let shared = std::sync::Arc::new(StubEndpoint::serving(FIRST_MODEL, &report));
    let r = ReportBackedResolver::new(
        Box::new(SharedEndpoint(shared.clone())),
        Box::new(FixedNonce(nonce)),
        pins_for_both(synthetic_pins()),
        OperatorKeyPins::none(),
    );

    assert!(
        r.refresh_on_signer_miss(FIXTURE_CAPTURED_AT)
            .await
            .is_some()
    );
    assert_eq!(shared.report_calls(), 1);

    // Every miss inside the floor is declined, however many arrive. An
    // unauthenticated route may not be a way to make the server hammer its
    // provider.
    for offset in [0, 1, 30, SIGNER_MISS_FLOOR_SECS - 1] {
        assert!(
            r.refresh_on_signer_miss(FIXTURE_CAPTURED_AT + offset)
                .await
                .is_none(),
            "a miss {offset}s after the last attempt must be declined"
        );
    }
    assert_eq!(shared.report_calls(), 1, "still exactly one fetch");

    assert!(
        r.refresh_on_signer_miss(FIXTURE_CAPTURED_AT + SIGNER_MISS_FLOOR_SECS)
            .await
            .is_some(),
        "at the floor a miss may refetch"
    );
    assert_eq!(shared.report_calls(), 2);
}

#[tokio::test]
async fn a_failed_attempt_still_starts_the_floor() {
    // Otherwise a permanently failing endpoint is retried per request, which
    // is the shape of a self-inflicted outage.
    let shared = std::sync::Arc::new(StubEndpoint::refusing(FIRST_MODEL));
    let r = ReportBackedResolver::new(
        Box::new(SharedEndpoint(shared.clone())),
        Box::new(FixedNonce(OUR_NONCE.to_string())),
        pins_for_both(synthetic_pins()),
        OperatorKeyPins::none(),
    );

    assert!(
        r.refresh_on_signer_miss(FIXTURE_CAPTURED_AT)
            .await
            .is_some()
    );
    assert!(
        r.refresh_on_signer_miss(FIXTURE_CAPTURED_AT + 1)
            .await
            .is_none()
    );
    assert_eq!(shared.report_calls(), 1);
}

#[tokio::test]
async fn concurrent_signer_misses_collapse_into_one_fetch() {
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let shared = std::sync::Arc::new(StubEndpoint::serving(FIRST_MODEL, &report));
    let r = std::sync::Arc::new(ReportBackedResolver::new(
        Box::new(SharedEndpoint(shared.clone())),
        Box::new(FixedNonce(nonce)),
        pins_for_both(synthetic_pins()),
        OperatorKeyPins::none(),
    ));

    let mut handles = Vec::new();
    for _ in 0..8 {
        let r = r.clone();
        handles.push(tokio::spawn(async move {
            r.refresh_on_signer_miss(FIXTURE_CAPTURED_AT)
                .await
                .is_some()
        }));
    }
    let mut ran = 0;
    for handle in handles {
        if handle.await.unwrap() {
            ran += 1;
        }
    }

    assert_eq!(ran, 1, "exactly one of eight concurrent misses may fetch");
    assert_eq!(shared.report_calls(), 1);
}

// ---------------------------------------------------------------------------
// Operator pins, as an override
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_operator_pin_overrides_the_report_derived_set() {
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let mut pins = BTreeMap::new();
    pins.insert(FIRST_MODEL.to_string(), vec![FOREIGN_KEY.to_string()]);
    let r = ReportBackedResolver::new(
        Box::new(StubEndpoint::serving(FIRST_MODEL, &report)),
        Box::new(FixedNonce(nonce)),
        pins_for_both(synthetic_pins()),
        OperatorKeyPins::none().with_model_pins(pins),
    );
    let outcome = r.refresh(FIXTURE_CAPTURED_AT).await;
    assert!(
        outcome.model.installed(),
        "precondition: the report half did resolve"
    );

    assert_eq!(
        model_keys(&r),
        vec![FOREIGN_KEY.to_string()],
        "where an operator nailed a key, that key is the answer"
    );
    // And the override narrows: the report-derived key is no longer accepted.
    let quote = hex::decode(json(ECDSA_REPORT)["intel_quote"].as_str().unwrap()).unwrap();
    assert!(!model_keys(&r).contains(&hex::encode(&quote[568..600])));
}

#[tokio::test]
async fn a_pinned_kind_with_no_entry_for_this_model_refuses_rather_than_falling_through() {
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let mut pins = BTreeMap::new();
    pins.insert(
        "some/other-model".to_string(),
        vec![FOREIGN_KEY.to_string()],
    );
    let r = ReportBackedResolver::new(
        Box::new(StubEndpoint::serving(FIRST_MODEL, &report)),
        Box::new(FixedNonce(nonce)),
        pins_for_both(synthetic_pins()),
        OperatorKeyPins::none().with_model_pins(pins),
    );
    r.refresh(FIXTURE_CAPTURED_AT).await;

    assert!(
        model_keys(&r).is_empty(),
        "an operator who listed the models they trust did not trust the rest"
    );
}

#[tokio::test]
async fn an_operator_pin_on_one_kind_leaves_the_other_kind_report_derived() {
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let r = ReportBackedResolver::new(
        Box::new(StubEndpoint::serving(FIRST_MODEL, &report)),
        Box::new(FixedNonce(nonce)),
        pins_for_both(synthetic_pins()),
        OperatorKeyPins::none().with_gateway_pins(vec![FOREIGN_KEY.to_string()]),
    );
    r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(
        r.keys_for(ReceiptSignatureKind::Gateway, None, FIXTURE_CAPTURED_AT),
        vec![FOREIGN_KEY.to_string()]
    );
    let quote = hex::decode(json(ECDSA_REPORT)["intel_quote"].as_str().unwrap()).unwrap();
    assert_eq!(model_keys(&r), vec![hex::encode(&quote[568..600])]);
}

#[tokio::test]
async fn an_operator_pin_is_not_subject_to_the_hard_ceiling() {
    // An operator's own statement does not go stale on a clock the endpoint
    // controls: it is theirs to withdraw.
    let (report, nonce) = synthetic_report(FIRST_MODEL);
    let mut pins = BTreeMap::new();
    pins.insert(FIRST_MODEL.to_string(), vec![FOREIGN_KEY.to_string()]);
    let r = ReportBackedResolver::new(
        Box::new(StubEndpoint::serving(FIRST_MODEL, &report)),
        Box::new(FixedNonce(nonce)),
        pins_for_both(synthetic_pins()),
        OperatorKeyPins::none().with_model_pins(pins),
    );
    r.refresh(FIXTURE_CAPTURED_AT).await;

    assert_eq!(
        r.keys_for(
            ReceiptSignatureKind::ProviderTee,
            Some(FIRST_MODEL),
            FIXTURE_CAPTURED_AT + HARD_CEILING_SECS * 10
        ),
        vec![FOREIGN_KEY.to_string()]
    );
}

// ---------------------------------------------------------------------------
// The static resolver, which is what the admission path still consults
// ---------------------------------------------------------------------------

#[test]
fn a_static_resolver_with_no_pins_enforces_nothing() {
    let r = StaticPinResolver::default();
    assert!(!r.is_enforcing(), "the pre-existing dormant default");
    assert!(
        r.keys_for(ReceiptSignatureKind::ProviderTee, Some(FIRST_MODEL), 0)
            .is_empty()
    );
}

#[test]
fn a_static_resolver_answers_exactly_the_pins_it_was_given() {
    let mut pins = BTreeMap::new();
    pins.insert(FIRST_MODEL.to_string(), vec![FOREIGN_KEY.to_string()]);
    let r = StaticPinResolver::new(OperatorKeyPins::none().with_model_pins(pins));

    assert!(r.is_enforcing());
    assert_eq!(
        r.keys_for(ReceiptSignatureKind::ProviderTee, Some(FIRST_MODEL), 0),
        vec![FOREIGN_KEY.to_string()]
    );
    assert!(
        r.keys_for(ReceiptSignatureKind::Gateway, None, 0)
            .is_empty(),
        "pinning one kind does not admit the other"
    );
    assert!(
        r.keys_for(ReceiptSignatureKind::Unrecognised, Some(FIRST_MODEL), 0)
            .is_empty()
    );
}

#[test]
fn a_static_resolver_ignores_the_clock() {
    // It holds no fetched state, so nothing about it may go stale. Asserted
    // because the trait carries a clock and a resolver that quietly aged its
    // operator's pins out would be a surprising outage.
    let mut pins = BTreeMap::new();
    pins.insert(FIRST_MODEL.to_string(), vec![FOREIGN_KEY.to_string()]);
    let r = StaticPinResolver::new(OperatorKeyPins::none().with_model_pins(pins));

    assert_eq!(
        r.keys_for(ReceiptSignatureKind::ProviderTee, Some(FIRST_MODEL), 0),
        r.keys_for(
            ReceiptSignatureKind::ProviderTee,
            Some(FIRST_MODEL),
            u64::MAX
        )
    );
}

// ---------------------------------------------------------------------------
// `reconcile`, exercised directly
// ---------------------------------------------------------------------------
//
// These are unit tests on the function rather than end-to-end runs, and that
// is deliberate rather than a shortcut. No report fixture can reach these
// branches: `attested_ed25519_key` derives the claimed key out of the entry's
// own `intel_quote` at the same fixed offset the verified quote exposes, so
// for any report the resolver gets this far with, the claimed set and the
// verified set are equal by construction and every comparison below is
// vacuously true. Mutating them away leaves an end-to-end suite green.
//
// That does not make the comparisons dead. They are what stands between the
// resolver and a future change to either walk -- a dropped `model_name`
// filter on one side, a JSON `report_data` echo trusted over the quote on the
// other -- and the way to keep them honest is to feed them the disagreement
// no capture can produce. The inputs here are hand-built; the *expectations*
// are not restatements of the implementation but the four properties the
// doc comment names.

/// A `VerifiedQuote` committing to `key || nonce`, with placeholder registers.
///
/// Registers are placeholders because `reconcile` reads `report_data` and
/// nothing else; a register that mattered here would be a coupling worth
/// finding out about, and this fixture would not hide it.
fn verified_quote_committing_to(key: &str, nonce: &str) -> VerifiedQuote {
    let mut report_data = hex::decode(key).expect("key is hex");
    report_data.extend(hex::decode(nonce).expect("nonce is hex"));
    assert_eq!(report_data.len(), 64, "report_data is 32 + 32 bytes");
    VerifiedQuote {
        report_data,
        mrtd: String::new(),
        mr_config_id: String::new(),
        rtmr: [String::new(), String::new(), String::new(), String::new()],
        tcb_status: "UpToDate".to_string(),
        advisory_ids: Vec::new(),
    }
}

const OTHER_KEY: &str = "2222222222222222222222222222222222222222222222222222222222222222";

#[test]
fn reconcile_accepts_a_quote_that_commits_to_the_claimed_key_and_our_nonce() {
    // The precondition every negative case below departs from by one property.
    let quote = verified_quote_committing_to(FOREIGN_KEY, OUR_NONCE);
    assert_eq!(
        reconcile(&[FOREIGN_KEY.to_string()], &[quote], OUR_NONCE),
        Ok(vec![FOREIGN_KEY.to_string()])
    );
}

#[test]
fn reconcile_refuses_a_quote_bound_to_a_nonce_we_did_not_send() {
    let quote = verified_quote_committing_to(FOREIGN_KEY, &"a".repeat(64));
    assert_eq!(
        reconcile(&[FOREIGN_KEY.to_string()], &[quote], OUR_NONCE),
        Err(RefreshRefusal::KeyNotInVerifiedQuote),
        "a quote committing to somebody else's challenge is not fresh for us"
    );
}

#[test]
fn reconcile_refuses_a_verified_key_the_report_never_claimed() {
    // The quote verifies and is bound to our nonce, but commits to a key the
    // report's JSON did not name. Installing it would mean the resolver
    // accepted a signer nothing in the report ever offered.
    let quote = verified_quote_committing_to(OTHER_KEY, OUR_NONCE);
    assert_eq!(
        reconcile(&[FOREIGN_KEY.to_string()], &[quote], OUR_NONCE),
        Err(RefreshRefusal::KeyNotInVerifiedQuote)
    );
}

#[test]
fn reconcile_refuses_a_verified_key_beyond_the_claimed_set() {
    // The claimed set is a strict *subset* of what the quotes carry, so the
    // "every claimed key was verified" direction is satisfied and only the
    // "every verified key was claimed" direction can refuse. Written this way
    // deliberately: a single-key mismatch is caught by either direction, and a
    // test that either one catches proves neither.
    let quotes = vec![
        verified_quote_committing_to(FOREIGN_KEY, OUR_NONCE),
        verified_quote_committing_to(OTHER_KEY, OUR_NONCE),
    ];
    assert_eq!(
        reconcile(&[FOREIGN_KEY.to_string()], &quotes, OUR_NONCE),
        Err(RefreshRefusal::KeyNotInVerifiedQuote)
    );
}

#[test]
fn reconcile_refuses_a_claimed_key_no_verified_quote_carries() {
    // The report claims two keys; one quote verifies. Answering with the
    // sound half would let a report carry a forged entry through on the
    // strength of a genuine one.
    let quote = verified_quote_committing_to(FOREIGN_KEY, OUR_NONCE);
    assert_eq!(
        reconcile(
            &[FOREIGN_KEY.to_string(), OTHER_KEY.to_string()],
            &[quote],
            OUR_NONCE
        ),
        Err(RefreshRefusal::KeyNotInVerifiedQuote)
    );
}

#[test]
fn reconcile_returns_the_verified_keys_in_quote_order_not_the_claimed_order() {
    // What is installed comes from the quotes, not from the JSON. Asserted
    // through the order, which is the only observable difference once the two
    // sets are equal -- and it is the assertion that survives an edit turning
    // the return into the claimed list.
    let quotes = vec![
        verified_quote_committing_to(OTHER_KEY, OUR_NONCE),
        verified_quote_committing_to(FOREIGN_KEY, OUR_NONCE),
    ];
    assert_eq!(
        reconcile(
            // Claimed in the opposite order, so the two lists are equal as
            // sets and distinguishable as sequences.
            &[FOREIGN_KEY.to_string(), OTHER_KEY.to_string()],
            &quotes,
            OUR_NONCE
        ),
        Ok(vec![OTHER_KEY.to_string(), FOREIGN_KEY.to_string()])
    );
}

#[test]
fn reconcile_refuses_report_data_that_is_too_short_to_read() {
    // `VerifiedQuote::report_data` is a `Vec<u8>` and this is a range read.
    // A short one must be a named refusal rather than a panic.
    let mut quote = verified_quote_committing_to(FOREIGN_KEY, OUR_NONCE);
    quote.report_data.truncate(40);
    assert_eq!(
        reconcile(&[FOREIGN_KEY.to_string()], &[quote], OUR_NONCE),
        Err(RefreshRefusal::KeyNotInVerifiedQuote)
    );
}

// ---------------------------------------------------------------------------
// `model_entry_quotes`, exercised directly
// ---------------------------------------------------------------------------

#[test]
fn model_entry_quotes_returns_only_the_entries_naming_this_model() {
    let document = serde_json::json!({
        "model_attestations": [
            {"model_name": "a", "intel_quote": "aabb"},
            {"model_name": "b", "intel_quote": "ccdd"},
            {"model_name": "a", "intel_quote": "eeff"},
        ]
    });
    assert_eq!(
        model_entry_quotes(&document.to_string(), "a"),
        Ok(vec![vec![0xaa, 0xbb], vec![0xee, 0xff]])
    );
}

#[test]
fn model_entry_quotes_refuses_a_model_with_no_entries() {
    // Reachable only if `model_ed25519_keys` ever stops refusing first. The
    // empty answer is the one that must never be `Ok(vec![])`: an empty quote
    // list verifies vacuously.
    let document = serde_json::json!({
        "model_attestations": [{"model_name": "b", "intel_quote": "ccdd"}]
    });
    assert_eq!(
        model_entry_quotes(&document.to_string(), "a"),
        Err(RefreshRefusal::ModelNotAttested)
    );
}

#[test]
fn model_entry_quotes_refuses_an_entry_for_this_model_with_no_readable_quote() {
    // A skip would leave the resolver verifying some other entry's quote and
    // installing a key it never checked.
    for entry in [
        serde_json::json!({"model_name": "a"}),
        serde_json::json!({"model_name": "a", "intel_quote": "not hex"}),
    ] {
        let document = serde_json::json!({
            "model_attestations": [entry, {"model_name": "a", "intel_quote": "aabb"}]
        });
        assert_eq!(
            model_entry_quotes(&document.to_string(), "a"),
            Err(RefreshRefusal::ReportShape)
        );
    }
}

#[test]
fn evidence_carries_digests_and_never_a_key() {
    let key = FOREIGN_KEY.to_string();
    let outcome = half_outcome(Ok(vec![key.clone()]));
    let rendered = format!("{outcome:?}");

    assert!(rendered.contains("sha256:"));
    assert!(
        !rendered.contains(&key),
        "a key may not reach an evidence surface"
    );
}
