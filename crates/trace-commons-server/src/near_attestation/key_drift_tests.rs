// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tests for the attested-key drift probe.
//!
//! Split into a sibling file the way `trace-commons-ingest.rs` splits its
//! tests: `key_drift.rs` is documentation-heavy and the two read better apart.
//!
//! Everything here runs offline against checked-in captures. Where a captured
//! report cannot exercise a step -- because the collateral committed for the
//! gateway platform does not verify a model enclave's quote, which is measured
//! below rather than assumed -- a **synthetic** entry is built over a quote
//! that does verify, and it is labelled synthetic wherever it appears.

use std::sync::Mutex;

use async_trait::async_trait;

use super::*;
use crate::near_attestation::client::{AttestationStep, AttestedKeyReportClient};
use crate::near_attestation::quote::{Collateral, parse_collateral};

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
const ED25519_REPORT_SECOND_MODEL: &str = include_str!(
    "../../../trace-commons-attestation/tests/fixtures/near_ai_model_attestation_report_ed25519_second_model.json"
);

const FIRST_MODEL: &str = "Qwen/Qwen3.6-35B-A3B-FP8";
const SECOND_MODEL: &str = "Qwen/Qwen3.8-27B";

/// See `quote::tests::FIXTURE_CAPTURED_AT`. `verify_quote` consults no clock
/// but this one, so these tests fail on a code change, never on a date.
const FIXTURE_CAPTURED_AT: u64 = 1_788_264_000;

/// A nonce this test process "generated". Deliberately not any fixture's.
const OUR_NONCE: &str = "9f3c1d2e4b5a60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9";

struct FixedNonce(&'static str);

impl DrillNonceSource for FixedNonce {
    fn nonce(&self) -> String {
        self.0.to_string()
    }
}

/// A stub endpoint. Records what it was asked for, so a test can assert on the
/// *challenge* and not only on the verdict.
struct StubEndpoint {
    model: String,
    report: Result<String, AttestationClientError>,
    collateral: String,
    seen_nonce: Mutex<Option<String>>,
    seen_quotes: Mutex<Vec<Vec<u8>>>,
}

impl StubEndpoint {
    fn serving(model: &str, report: &str) -> Self {
        Self {
            model: model.to_string(),
            report: Ok(report.to_string()),
            collateral: COLLATERAL.to_string(),
            seen_nonce: Mutex::new(None),
            seen_quotes: Mutex::new(Vec::new()),
        }
    }

    fn refusing(model: &str, error: AttestationClientError) -> Self {
        Self {
            model: model.to_string(),
            report: Err(error),
            collateral: COLLATERAL.to_string(),
            seen_nonce: Mutex::new(None),
            seen_quotes: Mutex::new(Vec::new()),
        }
    }

    fn nonce_seen(&self) -> Option<String> {
        self.seen_nonce.lock().unwrap().clone()
    }

    fn quotes_seen(&self) -> Vec<Vec<u8>> {
        self.seen_quotes.lock().unwrap().clone()
    }
}

#[async_trait]
impl AttestedKeyReportClient for StubEndpoint {
    fn model(&self) -> &str {
        &self.model
    }

    async fn fetch_ed25519_report_json(
        &self,
        nonce: &str,
    ) -> Result<String, AttestationClientError> {
        *self.seen_nonce.lock().unwrap() = Some(nonce.to_string());
        self.report.clone()
    }

    async fn fetch_collateral_for(
        &self,
        quote: &[u8],
    ) -> Result<Collateral, AttestationClientError> {
        self.seen_quotes.lock().unwrap().push(quote.to_vec());
        parse_collateral(&self.collateral).map_err(|_| AttestationClientError::MalformedResponse {
            step: AttestationStep::Collateral,
            detail_hash: "0000000000000000".to_string(),
        })
    }
}

fn json(report: &str) -> serde_json::Value {
    serde_json::from_str(report).expect("fixture parses")
}

fn fixture_nonce(report: &str) -> String {
    json(report)["gateway_attestation"]["request_nonce"]
        .as_str()
        .expect("captures carry a gateway nonce")
        .to_string()
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

fn step(
    outcome: &AttestedKeyDriftOutcome,
    step: AttestedKeyDriftStep,
) -> &AttestedKeyDriftStepResult {
    outcome
        .steps
        .iter()
        .find(|result| result.step == step)
        .expect("every step is rendered")
}

fn reason(outcome: &AttestedKeyDriftOutcome, wanted: AttestedKeyDriftStep) -> Option<&str> {
    step(outcome, wanted).reason.as_deref()
}

async fn run(stub: &StubEndpoint, nonce: &'static str) -> AttestedKeyDriftOutcome {
    run_attested_key_drift_drill(stub, None, &FixedNonce(nonce), FIXTURE_CAPTURED_AT).await
}

// ---------------------------------------------------------------------------
// The challenge is ours
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_probe_challenges_with_the_nonce_it_generated() {
    // The precondition, asserted rather than assumed: the capture is bound to
    // a nonce that is NOT the one this run generates. Without this the test
    // would pass against a probe that simply echoed whatever the report said.
    let captured = fixture_nonce(ED25519_REPORT);
    assert_ne!(captured, OUR_NONCE, "the fixture must bind a foreign nonce");

    let stub = StubEndpoint::serving(FIRST_MODEL, ED25519_REPORT);
    let outcome = run(&stub, OUR_NONCE).await;

    assert_eq!(
        stub.nonce_seen().as_deref(),
        Some(OUR_NONCE),
        "the endpoint must be challenged with the nonce the source produced"
    );
    assert_eq!(outcome.nonce, OUR_NONCE);
    assert_eq!(
        reason(&outcome, AttestedKeyDriftStep::ModelKeysBound),
        Some("nonce_mismatch"),
        "a report bound to someone else's nonce must not yield a key"
    );
    assert!(!outcome.passed);
}

#[tokio::test]
async fn rewriting_the_echoed_nonce_does_not_satisfy_the_binding() {
    // `request_nonce` is the provider's echo beside the quote. Rewrite every
    // echo in the capture to the nonce we sent; the quotes still commit to the
    // original, and the probe must still refuse. This is the check that stops
    // the whole exercise from being the endpoint marking its own homework.
    let mut document = json(ED25519_REPORT);
    document["gateway_attestation"]["request_nonce"] = serde_json::json!(OUR_NONCE);
    for entry in document["model_attestations"].as_array_mut().unwrap() {
        entry["request_nonce"] = serde_json::json!(OUR_NONCE);
    }
    let stub = StubEndpoint::serving(FIRST_MODEL, &document.to_string());
    let outcome = run(&stub, OUR_NONCE).await;

    assert_eq!(
        reason(&outcome, AttestedKeyDriftStep::ModelKeysBound),
        Some("nonce_mismatch")
    );
    assert!(!outcome.passed);
}

#[tokio::test]
async fn a_nonce_source_that_yields_a_malformed_nonce_never_reaches_the_network() {
    let stub = StubEndpoint::serving(FIRST_MODEL, ED25519_REPORT);
    let outcome =
        run_attested_key_drift_drill(&stub, None, &FixedNonce("not-a-nonce"), FIXTURE_CAPTURED_AT)
            .await;

    assert_eq!(stub.nonce_seen(), None, "no call may be made at all");
    assert_eq!(
        reason(&outcome, AttestedKeyDriftStep::ReportFetched),
        Some("nonce_source_malformed")
    );
}

// ---------------------------------------------------------------------------
// It is the ed25519 report, and the per-model key
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_ecdsa_report_carries_no_per_model_key() {
    // The ECDSA report is what `super::drill` fetches. It has no
    // `model_attestations` at all -- which is the reason this probe exists --
    // so feeding one here must be a named refusal, not an empty pass.
    assert!(
        json(ECDSA_REPORT).get("model_attestations").is_none(),
        "precondition: the ECDSA capture carries no model attestations"
    );
    let stub = StubEndpoint::serving(FIRST_MODEL, ECDSA_REPORT);
    let outcome = run(&stub, OUR_NONCE).await;

    assert_eq!(
        reason(&outcome, AttestedKeyDriftStep::ModelKeysBound),
        Some("report_shape")
    );
    assert!(outcome.model_key_refs.is_empty());
    assert!(!outcome.passed);
}

#[tokio::test]
async fn the_per_model_key_is_not_the_gateway_key() {
    // Checking a hosted-model receipt against the gateway key refuses every
    // real one. The two keys being different is the whole reason the client
    // reads `model_attestations`, and it is asserted here against a real
    // capture so a regression to "one key" is visible.
    let nonce = fixture_nonce(ED25519_REPORT);
    let stub = StubEndpoint::serving(FIRST_MODEL, ED25519_REPORT);
    let outcome = run_attested_key_drift_drill(
        &stub,
        None,
        &FixedNonce(Box::leak(nonce.into_boxed_str())),
        FIXTURE_CAPTURED_AT,
    )
    .await;

    assert_eq!(
        step(&outcome, AttestedKeyDriftStep::GatewayKeyBound).status,
        AttestedKeyDriftStatus::Passed
    );
    assert_eq!(
        step(&outcome, AttestedKeyDriftStep::ModelKeysBound).status,
        AttestedKeyDriftStatus::Passed,
        "{:?}",
        outcome.blocking_steps()
    );
    assert_eq!(outcome.model_entry_count, 1);
    assert_ne!(
        outcome.gateway_key_ref.as_deref(),
        outcome.model_key_refs.first().map(String::as_str),
        "the model key and the gateway key must not be the same key"
    );
}

#[tokio::test]
async fn two_models_have_different_keys_behind_one_gateway_key() {
    // Answers, from captures alone: per-model keys differ per model, while the
    // gateway key is shared. A probe that returned the gateway key for both
    // would look stable and be useless.
    let first = {
        let nonce = fixture_nonce(ED25519_REPORT);
        let stub = StubEndpoint::serving(FIRST_MODEL, ED25519_REPORT);
        run_attested_key_drift_drill(
            &stub,
            None,
            &FixedNonce(Box::leak(nonce.into_boxed_str())),
            FIXTURE_CAPTURED_AT,
        )
        .await
    };
    let second = {
        let nonce = fixture_nonce(ED25519_REPORT_SECOND_MODEL);
        let stub = StubEndpoint::serving(SECOND_MODEL, ED25519_REPORT_SECOND_MODEL);
        run_attested_key_drift_drill(
            &stub,
            None,
            &FixedNonce(Box::leak(nonce.into_boxed_str())),
            FIXTURE_CAPTURED_AT,
        )
        .await
    };

    assert_eq!(first.gateway_key_ref, second.gateway_key_ref);
    assert_ne!(first.model_key_refs, second.model_key_refs);
    assert_ne!(first.model_key_refs, Vec::<String>::new());
    // And a comparison across two models reports nothing, because they are not
    // comparable: keys differing per model is the design, not drift.
    assert_eq!(compare_attested_key_drift(&first, &second), Vec::new());
}

// ---------------------------------------------------------------------------
// The quote, and the collateral it needs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn collateral_is_fetched_for_the_model_entry_quote_not_the_gateways() {
    let nonce = fixture_nonce(ED25519_REPORT);
    let stub = StubEndpoint::serving(FIRST_MODEL, ED25519_REPORT);
    let _ = run_attested_key_drift_drill(
        &stub,
        None,
        &FixedNonce(Box::leak(nonce.into_boxed_str())),
        FIXTURE_CAPTURED_AT,
    )
    .await;

    let seen = stub.quotes_seen();
    assert_eq!(seen.len(), 1, "one entry names this model");
    assert_eq!(
        seen[0],
        model_quote_bytes(ED25519_REPORT),
        "the quote sent for collateral must be the model entry's"
    );
    assert_ne!(
        seen[0],
        gateway_quote_bytes(ED25519_REPORT),
        "precondition: the two quotes differ, so this test can fail"
    );
}

#[tokio::test]
async fn the_gateways_collateral_does_not_verify_a_model_quote() {
    // Measured, and the reason `fetch_collateral_for` is called per quote.
    // The checked-in collateral was fetched for the ECDSA gateway platform;
    // against a model enclave's quote `dcap-qvl` reports an invalid QE report
    // signature, i.e. the PCK chain belongs to another machine. If a future
    // capture ever makes this pass, the assertion below is the thing that says
    // so rather than a silently stronger drill.
    let nonce = fixture_nonce(ED25519_REPORT);
    let stub = StubEndpoint::serving(FIRST_MODEL, ED25519_REPORT);
    let outcome = run_attested_key_drift_drill(
        &stub,
        None,
        &FixedNonce(Box::leak(nonce.into_boxed_str())),
        FIXTURE_CAPTURED_AT,
    )
    .await;

    assert_eq!(
        reason(&outcome, AttestedKeyDriftStep::ModelQuoteVerified),
        Some("verification_failed")
    );
    assert_eq!(
        step(&outcome, AttestedKeyDriftStep::ModelTcbUpToDate).status,
        AttestedKeyDriftStatus::NotRun,
        "a step that never ran is not a pass"
    );
    assert!(outcome.quote.is_none());
}

// ---------------------------------------------------------------------------
// A green run, over a quote that really verifies
// ---------------------------------------------------------------------------

/// A **synthetic** ed25519 report built over the ECDSA capture's quote.
///
/// The quote is real and verifies against the checked-in collateral; what is
/// synthetic is the ed25519 framing around it. The signing key is taken to be
/// `report_data[0..32]` of that very quote, which is what an ed25519
/// attestation commits to, so the report is internally consistent by
/// construction rather than by a hand-written expectation.
fn synthetic_ed25519_report(model: &str) -> (String, String) {
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

fn pin_from(outcome: &AttestedKeyDriftOutcome) -> ExpectedMeasurements {
    let quote = outcome.quote.as_ref().expect("the quote verified");
    let raw = format!(
        "mrtd={},rtmr0={},rtmr1={},rtmr2={},rtmr3={}",
        quote.mrtd, quote.rtmr[0], quote.rtmr[1], quote.rtmr[2], quote.rtmr[3]
    );
    ExpectedMeasurements::from_env_value(Some(&raw))
        .expect("the quote's own values are a valid pin set")
        .expect("a non-empty value yields a set")
}

#[tokio::test]
async fn a_bound_entry_over_a_verifiable_quote_reaches_the_measurement_step() {
    let (report, nonce) = synthetic_ed25519_report(FIRST_MODEL);
    let stub = StubEndpoint::serving(FIRST_MODEL, &report);
    let nonce: &'static str = Box::leak(nonce.into_boxed_str());
    let outcome = run(&stub, nonce).await;

    for wanted in [
        AttestedKeyDriftStep::ReportFetched,
        AttestedKeyDriftStep::GatewayKeyBound,
        AttestedKeyDriftStep::ModelKeysBound,
        AttestedKeyDriftStep::ModelQuoteVerified,
        AttestedKeyDriftStep::ModelTcbUpToDate,
        AttestedKeyDriftStep::ModelKeyInVerifiedQuote,
    ] {
        assert_eq!(
            step(&outcome, wanted).status,
            AttestedKeyDriftStatus::Passed,
            "{} should pass; blocking: {:?}",
            wanted.as_str(),
            outcome.blocking_steps()
        );
    }
    assert_eq!(outcome.credential, ReportCredentialVerdict::Accepted);
    assert_eq!(
        outcome.quote.as_ref().map(|q| q.tcb_status.as_str()),
        Some(REQUIRED_TCB_STATUS)
    );
    // Nothing pinned is a failure, not a skip -- same rule as the ECDSA drill.
    assert_eq!(
        step(&outcome, AttestedKeyDriftStep::ModelMeasurementsPinned).missing_control,
        Some("near_ai_expected_measurements".to_string())
    );
    assert!(!outcome.passed, "an unpinned run has not passed");
}

#[tokio::test]
async fn with_the_measurements_pinned_every_step_passes() {
    let (report, nonce) = synthetic_ed25519_report(FIRST_MODEL);
    let stub = StubEndpoint::serving(FIRST_MODEL, &report);
    let nonce: &'static str = Box::leak(nonce.into_boxed_str());
    let unpinned = run(&stub, nonce).await;
    let pin = pin_from(&unpinned);

    let stub = StubEndpoint::serving(FIRST_MODEL, &report);
    let outcome =
        run_attested_key_drift_drill(&stub, Some(&pin), &FixedNonce(nonce), FIXTURE_CAPTURED_AT)
            .await;

    assert!(
        outcome.passed,
        "blocking steps: {:?}",
        outcome.blocking_steps()
    );
    assert!(outcome.blocking_steps().is_empty());
}

#[tokio::test]
async fn a_key_the_verified_quote_does_not_commit_to_is_refused() {
    // The step that makes DCAP verification mean something: a report whose
    // JSON is internally consistent, over a quote committing to a different
    // key. Built by swapping the quote for the ed25519 capture's gateway
    // quote, which verifies against no checked-in collateral -- so the quote
    // step is what fails first. What this asserts is that the probe never
    // reaches a pass on the strength of the JSON alone.
    let (report, nonce) = synthetic_ed25519_report(FIRST_MODEL);
    let mut document: serde_json::Value = serde_json::from_str(&report).unwrap();
    let foreign = hex::encode(gateway_quote_bytes(ED25519_REPORT));
    document["model_attestations"][0]["intel_quote"] = serde_json::json!(foreign);
    let stub = StubEndpoint::serving(FIRST_MODEL, &document.to_string());
    let nonce: &'static str = Box::leak(nonce.into_boxed_str());
    let outcome = run(&stub, nonce).await;

    assert!(!outcome.passed);
    assert_eq!(
        step(&outcome, AttestedKeyDriftStep::ModelKeyInVerifiedQuote).status,
        AttestedKeyDriftStatus::NotRun,
        "the binding step must not pass on a quote nothing verified"
    );
    // And the JSON-level reader refuses it too, since the swapped quote
    // commits to another key entirely.
    assert_eq!(
        reason(&outcome, AttestedKeyDriftStep::ModelKeysBound),
        Some("report_data_mismatch")
    );
}

#[tokio::test]
async fn an_entry_for_another_model_is_not_verified_in_its_place() {
    // A report carrying a foreign model first. The probe must derive and
    // verify only the entry naming its own model: pairing the target model's
    // key with another entry's quote is the mispairing the set comparison at
    // the binding step exists to catch.
    let (report, nonce) = synthetic_ed25519_report(FIRST_MODEL);
    let mut document: serde_json::Value = serde_json::from_str(&report).unwrap();
    let mine = document["model_attestations"][0].clone();
    let mut foreign = mine.clone();
    foreign["model_name"] = serde_json::json!(SECOND_MODEL);
    foreign["intel_quote"] = serde_json::json!(hex::encode(model_quote_bytes(ED25519_REPORT)));
    document["model_attestations"] = serde_json::json!([foreign, mine]);
    let stub = StubEndpoint::serving(FIRST_MODEL, &document.to_string());
    let nonce: &'static str = Box::leak(nonce.into_boxed_str());
    let outcome = run(&stub, nonce).await;

    let seen = stub.quotes_seen();
    assert_eq!(seen.len(), 1, "only our model's entry is verified");
    assert_eq!(
        seen[0],
        hex::decode(json(ECDSA_REPORT)["intel_quote"].as_str().unwrap()).unwrap()
    );
    assert_eq!(
        step(&outcome, AttestedKeyDriftStep::ModelKeyInVerifiedQuote).status,
        AttestedKeyDriftStatus::Passed,
        "blocking: {:?}",
        outcome.blocking_steps()
    );
    assert_eq!(outcome.model_entry_count, 1);
}

// ---------------------------------------------------------------------------
// The credential question
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_rejected_credential_is_reported_as_such() {
    for status in [401u16, 403] {
        let stub = StubEndpoint::refusing(
            FIRST_MODEL,
            AttestationClientError::HttpStatus {
                step: AttestationStep::Report,
                status,
            },
        );
        let outcome = run(&stub, OUR_NONCE).await;
        assert_eq!(
            outcome.credential,
            ReportCredentialVerdict::Unauthorized,
            "HTTP {status} is an authorization answer"
        );
        assert_eq!(
            reason(&outcome, AttestedKeyDriftStep::ReportFetched),
            Some("http_status")
        );
    }
}

#[tokio::test]
async fn an_unreachable_endpoint_says_nothing_about_the_credential() {
    let stub = StubEndpoint::refusing(
        FIRST_MODEL,
        AttestationClientError::Transport {
            step: AttestationStep::Report,
            detail_hash: "0011223344556677".to_string(),
        },
    );
    let outcome = run(&stub, OUR_NONCE).await;
    assert_eq!(outcome.credential, ReportCredentialVerdict::Inconclusive);
}

#[tokio::test]
async fn a_missing_control_is_named_rather_than_hashed() {
    let stub = StubEndpoint::refusing(
        FIRST_MODEL,
        AttestationClientError::MissingControl {
            step: AttestationStep::Report,
            control: "near_ai_api_key",
        },
    );
    let outcome = run(&stub, OUR_NONCE).await;
    assert_eq!(
        step(&outcome, AttestedKeyDriftStep::ReportFetched).missing_control,
        Some("near_ai_api_key".to_string())
    );
}

// ---------------------------------------------------------------------------
// Evidence discipline
// ---------------------------------------------------------------------------

#[tokio::test]
async fn evidence_carries_no_key_material() {
    let nonce = fixture_nonce(ED25519_REPORT);
    let stub = StubEndpoint::serving(FIRST_MODEL, ED25519_REPORT);
    let outcome = run_attested_key_drift_drill(
        &stub,
        None,
        &FixedNonce(Box::leak(nonce.clone().into_boxed_str())),
        FIXTURE_CAPTURED_AT,
    )
    .await;
    let rendered = serde_json::to_string(&outcome).unwrap();

    let gateway_key = json(ED25519_REPORT)["gateway_attestation"]["signing_address"]
        .as_str()
        .unwrap()
        .to_string();
    let model_key = json(ED25519_REPORT)["model_attestations"][0]["signing_address"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!gateway_key.is_empty() && !model_key.is_empty());
    assert!(
        !rendered.contains(&gateway_key),
        "the gateway key must appear only as a digest"
    );
    assert!(
        !rendered.contains(&model_key),
        "the model key must appear only as a digest"
    );
    assert!(!rendered.contains("http"), "no URL may reach evidence");
    // The nonce is public by construction: we generated it and put it in a
    // query string. It is the one full value here.
    assert!(rendered.contains(&nonce));
    for key_ref in outcome
        .model_key_refs
        .iter()
        .chain(outcome.gateway_key_ref.iter())
    {
        assert!(key_ref.starts_with("sha256:"), "{key_ref}");
    }
}

#[tokio::test]
async fn every_step_is_rendered_even_when_the_probe_stops_early() {
    let stub = StubEndpoint::refusing(
        FIRST_MODEL,
        AttestationClientError::Transport {
            step: AttestationStep::Report,
            detail_hash: "0011223344556677".to_string(),
        },
    );
    let outcome = run(&stub, OUR_NONCE).await;

    assert_eq!(outcome.steps.len(), AttestedKeyDriftStep::ALL.len());
    let rendered: Vec<AttestedKeyDriftStep> =
        outcome.steps.iter().map(|result| result.step).collect();
    assert_eq!(rendered, AttestedKeyDriftStep::ALL.to_vec());
    assert_eq!(
        outcome
            .steps
            .iter()
            .filter(|result| result.status == AttestedKeyDriftStatus::NotRun)
            .count(),
        AttestedKeyDriftStep::ALL.len() - 1
    );
}

#[test]
fn every_step_has_a_distinct_label() {
    let mut labels: Vec<&str> = AttestedKeyDriftStep::ALL
        .iter()
        .map(|step| step.as_str())
        .collect();
    let count = labels.len();
    labels.sort_unstable();
    labels.dedup();
    assert_eq!(labels.len(), count);
}

// ---------------------------------------------------------------------------
// Drift comparison
// ---------------------------------------------------------------------------

fn outcome_for_drift() -> AttestedKeyDriftOutcome {
    AttestedKeyDriftOutcome {
        nonce: OUR_NONCE.to_string(),
        model_label: FIRST_MODEL.to_string(),
        passed: true,
        steps: AttestedKeyDriftStep::ALL
            .iter()
            .map(|step| AttestedKeyDriftStepResult {
                step: *step,
                status: AttestedKeyDriftStatus::Passed,
                reason: None,
                missing_control: None,
            })
            .collect(),
        credential: ReportCredentialVerdict::Accepted,
        gateway_key_ref: Some("sha256:aa".to_string()),
        model_key_refs: vec!["sha256:bb".to_string()],
        model_entry_count: 1,
        quote: Some(AttestedKeyQuoteEvidence {
            tcb_status: REQUIRED_TCB_STATUS.to_string(),
            advisory_ids: Vec::new(),
            mrtd: "11".to_string(),
            mr_config_id: "22".to_string(),
            rtmr: [
                "30".to_string(),
                "31".to_string(),
                "32".to_string(),
                "33".to_string(),
            ],
        }),
        measurements: None,
    }
}

#[test]
fn two_identical_runs_report_no_drift() {
    let before = outcome_for_drift();
    let after = outcome_for_drift();
    assert_eq!(compare_attested_key_drift(&before, &after), Vec::new());
}

#[test]
fn a_key_rotation_and_a_measurement_move_are_separate_findings() {
    // The distinction is the entire value of running this on a schedule: a
    // rotation is NEAR AI re-keying, a measurement move is a redeployed image,
    // and one label for both sends an operator to the wrong place.
    let before = outcome_for_drift();
    let mut after = outcome_for_drift();
    after.model_key_refs = vec!["sha256:cc".to_string()];
    after.quote.as_mut().unwrap().mrtd = "99".to_string();

    let findings = compare_attested_key_drift(&before, &after);
    assert!(
        findings.contains(&AttestedKeyDrift::ModelKeysRotated),
        "{findings:?}"
    );
    assert!(
        findings.contains(&AttestedKeyDrift::MeasurementMoved {
            field: "mrtd".to_string()
        }),
        "{findings:?}"
    );
    assert_eq!(findings.len(), 2, "{findings:?}");
}

#[test]
fn the_gateway_key_and_the_model_keys_rotate_independently() {
    let before = outcome_for_drift();
    let mut after = outcome_for_drift();
    after.gateway_key_ref = Some("sha256:zz".to_string());
    assert_eq!(
        compare_attested_key_drift(&before, &after),
        vec![AttestedKeyDrift::GatewayKeyRotated]
    );
}

#[test]
fn a_quote_that_stops_verifying_is_not_reported_as_a_rotation() {
    let before = outcome_for_drift();
    let mut after = outcome_for_drift();
    after.passed = false;
    after.quote = None;
    for result in &mut after.steps {
        if result.step == AttestedKeyDriftStep::ModelQuoteVerified {
            result.status = AttestedKeyDriftStatus::Failed;
            result.reason = Some("verification_failed".to_string());
        }
    }
    let findings = compare_attested_key_drift(&before, &after);
    assert_eq!(findings, vec![AttestedKeyDrift::QuoteVerificationRegressed]);
}

#[test]
fn a_tcb_change_is_named_with_both_verdicts() {
    let before = outcome_for_drift();
    let mut after = outcome_for_drift();
    after.quote.as_mut().unwrap().tcb_status = "SWHardeningNeeded".to_string();
    assert_eq!(
        compare_attested_key_drift(&before, &after),
        vec![AttestedKeyDrift::TcbStatusChanged {
            before: REQUIRED_TCB_STATUS.to_string(),
            after: "SWHardeningNeeded".to_string(),
        }]
    );
}

#[test]
fn a_run_that_learned_nothing_is_not_a_rotation() {
    // The failure mode this guards: a run whose fetch failed has no keys, and
    // comparing `Some` against `None` would report a rotation on exactly the
    // run that observed the least.
    let before = outcome_for_drift();
    let mut after = outcome_for_drift();
    after.gateway_key_ref = None;
    after.model_key_refs = Vec::new();
    after.model_entry_count = 0;
    after.quote = None;
    after.credential = ReportCredentialVerdict::Unauthorized;
    for result in &mut after.steps {
        result.status = AttestedKeyDriftStatus::NotRun;
        if result.step == AttestedKeyDriftStep::ReportFetched {
            result.status = AttestedKeyDriftStatus::Failed;
            result.reason = Some("http_status".to_string());
        }
    }

    let findings = compare_attested_key_drift(&before, &after);
    assert!(findings.contains(&AttestedKeyDrift::ReportUnavailable));
    assert!(findings.contains(&AttestedKeyDrift::CredentialRejected));
    assert!(findings.contains(&AttestedKeyDrift::QuoteVerificationRegressed));
    assert!(
        !findings.contains(&AttestedKeyDrift::ModelKeysRotated),
        "{findings:?}"
    );
    assert!(
        !findings.contains(&AttestedKeyDrift::GatewayKeyRotated),
        "{findings:?}"
    );
}

#[test]
fn an_enclave_count_change_is_its_own_finding() {
    let before = outcome_for_drift();
    let mut after = outcome_for_drift();
    after.model_entry_count = 2;
    after.model_key_refs = vec!["sha256:bb".to_string(), "sha256:dd".to_string()];
    let findings = compare_attested_key_drift(&before, &after);
    assert!(
        findings.contains(&AttestedKeyDrift::ModelEntryCountChanged {
            before: 1,
            after: 2
        })
    );
    assert!(findings.contains(&AttestedKeyDrift::ModelKeysRotated));
}

// ---------------------------------------------------------------------------
// The live client's request shape
// ---------------------------------------------------------------------------

/// The only test that proves the live client asks for the ed25519 report. The
/// stub above cannot: it is handed a report and never sees a query string.
#[tokio::test]
async fn the_live_client_asks_for_the_ed25519_report_with_our_nonce() {
    use crate::near_attestation::client::HttpAttestationClient;
    use secrecy::SecretString;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/attestation/report"))
        .and(query_param("signing_algo", "ed25519"))
        .and(query_param("nonce", OUR_NONCE))
        .and(query_param("model", FIRST_MODEL))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"marker\":1}"))
        .mount(&server)
        .await;

    let client = HttpAttestationClient::new(
        format!("{}/v1", server.uri()),
        FIRST_MODEL,
        SecretString::from("unused"),
        "https://invalid.test",
        Duration::from_secs(5),
    )
    .expect("client builds");

    let body = AttestedKeyReportClient::fetch_ed25519_report_json(&client, OUR_NONCE)
        .await
        .expect("the mock matched every parameter the probe must send");
    assert_eq!(
        body, "{\"marker\":1}",
        "the raw body must be returned, not a re-serialization"
    );
}

#[tokio::test]
async fn the_live_client_reports_a_rejected_credential_by_status() {
    use crate::near_attestation::client::HttpAttestationClient;
    use secrecy::SecretString;
    use std::time::Duration;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401).set_body_string("nope"))
        .mount(&server)
        .await;

    let client = HttpAttestationClient::new(
        format!("{}/v1", server.uri()),
        FIRST_MODEL,
        SecretString::from("unused"),
        "https://invalid.test",
        Duration::from_secs(5),
    )
    .expect("client builds");

    let error = AttestedKeyReportClient::fetch_ed25519_report_json(&client, OUR_NONCE)
        .await
        .expect_err("401 is not a report");
    assert_eq!(
        credential_verdict(&error),
        ReportCredentialVerdict::Unauthorized
    );
}
