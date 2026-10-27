// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The staging pin re-sign tool, end to end.
//!
//! Two halves. `observe` runs in process against a mock NEAR AI gateway
//! serving checked-in report captures, with Intel collateral from the
//! checked-in fixture, so a quote that does not verify and a report bound to
//! another nonce are both refused for real. `sign`, `verify` and
//! `public-key` run as the built binary, so what is asserted about argv,
//! stdin, stdout and stderr is what an operator's shell sees.
//!
//! Every key here is generated in the test and every measurement is
//! synthetic or read out of a checked-in capture.

#![cfg(feature = "near-ai-staging")]

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use async_trait::async_trait;
use rand::RngCore as _;
use ring::signature::{Ed25519KeyPair, KeyPair as _};
use secrecy::SecretString;
use trace_commons_attestation::measurements::ExpectedMeasurements;
use trace_commons_attestation::quote::{
    Collateral, QuoteVerifyError, VerifiedQuote, parse_collateral, verify_quote,
};
use trace_commons_attestation::staging_pins;
use trace_commons_attestation::staging_replay::{
    MARKER_EVENTS, RTMR3_EVENT_TYPE, RTMR3_EVENTS, compose_actions_hash, event_digest,
    replay_digests,
};
use trace_commons_server::near_attestation::client::{
    AttestationClientError, AttestedKeyReportClient, HttpAttestationClient, INTEL_PCS_URL,
};
use trace_commons_server::near_attestation::key_drift::DrillNonceSource;
use trace_commons_server::near_attestation::pin_observe::{
    DEFAULT_REGISTERS, observe_measurement_sets,
};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const BIN: &str = env!("CARGO_BIN_EXE_trace-commons-staging-pins");

const COLLATERAL: &str = include_str!(
    "../../trace-commons-attestation/tests/fixtures/near_ai_attestation_collateral.json"
);
/// The single-enclave ECDSA capture. Its quote is the only checked-in quote
/// the checked-in collateral verifies.
const ECDSA_REPORT: &str =
    include_str!("../../trace-commons-attestation/tests/fixtures/near_ai_attestation_report.json");
/// A real ed25519 gateway capture. Its model quote is genuine but does not
/// verify against the checked-in collateral, which is for another platform.
const ED25519_REPORT: &str = include_str!(
    "../../trace-commons-attestation/tests/fixtures/near_ai_model_attestation_report_ed25519.json"
);

const MODEL: &str = "Qwen/Qwen3.6-35B-A3B-FP8";

/// 2026-09-01T12:00:00Z, when the fixtures were captured. `verify_quote`
/// consults no clock but this one, so these tests fail on a code change and
/// never on a date.
const FIXTURE_CAPTURED_AT: u64 = 1_788_264_000;

/// The staging gateway host the app compares a payload's `gateway_host`
/// with: the host of the server's compiled-in staging NEAR AI API base.
const STAGING_GATEWAY_HOST: &str = "cloud-stg-api.near.ai";

// ---------------------------------------------------------------------------
// observe, against a mock gateway
// ---------------------------------------------------------------------------

struct FixedNonce(String);

impl DrillNonceSource for FixedNonce {
    fn nonce(&self) -> String {
        self.0.clone()
    }
}

/// The live HTTP client for the report, the checked-in fixture for the
/// collateral (the collateral client needs Intel's service).
struct MockGateway {
    http: HttpAttestationClient,
    collateral: String,
}

#[async_trait]
impl AttestedKeyReportClient for MockGateway {
    fn model(&self) -> &str {
        AttestedKeyReportClient::model(&self.http)
    }

    async fn fetch_ed25519_report_json(
        &self,
        nonce: &str,
    ) -> Result<String, AttestationClientError> {
        self.http.fetch_ed25519_report_json(nonce).await
    }

    async fn fetch_collateral_for(
        &self,
        _quote: &[u8],
    ) -> Result<Collateral, AttestationClientError> {
        Ok(parse_collateral(&self.collateral).expect("collateral fixture parses"))
    }
}

async fn gateway_serving(report: &str, nonce: &str) -> (MockServer, MockGateway) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/attestation/report"))
        .and(query_param("model", MODEL))
        .and(query_param("nonce", nonce))
        .and(query_param("signing_algo", "ed25519"))
        .respond_with(ResponseTemplate::new(200).set_body_string(report.to_string()))
        .expect(1)
        .mount(&server)
        .await;
    let http = HttpAttestationClient::new(
        format!("{}/v1", server.uri()),
        MODEL,
        SecretString::from("synthetic-test-key".to_string()),
        INTEL_PCS_URL,
        Duration::from_secs(10),
        Some(format!("{}/v1", server.uri())),
    )
    .expect("client builds");
    (
        server,
        MockGateway {
            http,
            collateral: COLLATERAL.to_string(),
        },
    )
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).expect("fixture parses")
}

// Synthetic RTMR3 payloads and compose actions: none is a real
// measurement.
const APP_ID: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
const COMPOSE_HASH: &str = "c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";
/// The per-boot value observe must never print, in any case.
const INSTANCE_ID: &str = "1d5ec2e71d5ec2e71d5ec2e71d5ec2e71d5ec2e7";
const MR_KMS: &str = "3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b";
const OS_IMAGE_HASH: &str = "0505050505050505050505050505050505050505050505050505050505050505";
const KEY_PROVIDER: &str = "7b226e616d65223a226b6d73222c226964223a2273796e746865746963227d";

fn marker_payload(name: &str) -> String {
    hex::encode(format!("synthetic {name}"))
}

/// The ten RTMR3 events, in order, with synthetic payloads.
fn ten_events() -> Vec<(String, String)> {
    RTMR3_EVENTS
        .iter()
        .map(|name| {
            let payload = match *name {
                "app-id" => APP_ID.to_string(),
                "compose-hash" => COMPOSE_HASH.to_string(),
                "instance-id" => INSTANCE_ID.to_string(),
                "mr-kms" => MR_KMS.to_string(),
                "os-image-hash" => OS_IMAGE_HASH.to_string(),
                "key-provider" => KEY_PROVIDER.to_string(),
                marker => marker_payload(marker),
            };
            (name.to_string(), payload)
        })
        .collect()
}

fn event_log(events: &[(String, String)]) -> serde_json::Value {
    let mut log = vec![serde_json::json!({
        "imr": 0, "event_type": 1, "digest": "ee".repeat(48), "event": "", "event_payload": ""
    })];
    for (name, payload) in events {
        log.push(serde_json::json!({
            "imr": 3, "event_type": RTMR3_EVENT_TYPE, "digest": "",
            "event": name, "event_payload": payload,
        }));
    }
    serde_json::Value::Array(log)
}

fn replayed(events: &[(String, String)]) -> String {
    let digests: Vec<[u8; 48]> = events
        .iter()
        .map(|(n, p)| event_digest(RTMR3_EVENT_TYPE, n, &hex::decode(p).unwrap()))
        .collect();
    hex::encode(replay_digests(&digests))
}

/// Synthetic compose-action values, in the live record's formats.
const COMPOSE_FILE: &str = "example/models.yaml";
const FILE_SHA256: &str = "f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1";
const COMMIT: &str = "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1";

fn manager_image() -> String {
    format!(
        "registry.example/compose-manager@sha256:{}",
        "ab".repeat(32)
    )
}

/// One action of each kind, structurally exact to the live record (string
/// timestamps, a digest-pinned compose-manager image, compose files named by
/// path, commit and SHA-256), all values synthetic. The second `compose_up`
/// repeats the first file, so the review lists it once.
fn actions() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({"action": "compose_manager_started", "container": "compose-manager", "image": manager_image(), "timestamp": "2026-01-01T00:00:00Z"}),
        serde_json::json!({"action": "compose_up", "tag": "v0.0.1", "commit": COMMIT, "file": COMPOSE_FILE, "file_sha256": FILE_SHA256, "services": ["model-a", "proxy"], "timestamp": "2026-01-01T00:00:01Z"}),
        serde_json::json!({"action": "compose_down", "tag": "v0.0.1", "commit": COMMIT, "file": COMPOSE_FILE, "services": ["proxy"], "timestamp": "2026-01-01T00:00:02Z"}),
        serde_json::json!({"action": "compose_up", "tag": "v0.0.1", "commit": COMMIT, "file": COMPOSE_FILE, "file_sha256": FILE_SHA256, "services": ["proxy"], "timestamp": "2026-01-01T00:00:03Z"}),
    ]
}

/// The capture's quote bytes, and the compose-manager quote's: the same
/// quote with one byte appended, so the staged verifier below can tell the
/// two apart by their bytes and verify the real quote for both.
fn fixture_quote() -> Vec<u8> {
    hex::decode(json(ECDSA_REPORT)["intel_quote"].as_str().unwrap()).unwrap()
}

fn compose_quote() -> Vec<u8> {
    let mut quote = fixture_quote();
    quote.push(0);
    quote
}

/// A gateway quote: the capture's with two bytes appended.
fn gateway_quote() -> Vec<u8> {
    let mut quote = fixture_quote();
    quote.extend([0, 1]);
    quote
}

/// A **synthetic** ed25519 report over the ECDSA capture's quote, which
/// verifies. The key is the quote's own `report_data[0..32]` and the nonce
/// its `[32..64]`, so the report is consistent by construction. Same recipe
/// as `near_attestation::key_drift_tests`. The model entry carries its own
/// RTMR3 log and compose-manager record, as NEAR AI staging's do.
fn synthetic_ed25519_report() -> (String, String) {
    report_with(&ten_events(), &actions())
}

fn report_with(events: &[(String, String)], actions: &[serde_json::Value]) -> (String, String) {
    let quote = fixture_quote();
    let report_data = &quote[568..632];
    let key = hex::encode(&report_data[0..32]);
    let nonce = hex::encode(&report_data[32..64]);
    let entry = serde_json::json!({
        "model_name": MODEL,
        "signing_algo": "ed25519",
        "signing_address": key,
        "request_nonce": nonce,
        "intel_quote": hex::encode(&quote),
        "event_log": event_log(events),
        "compose_manager_attestation": {
            "actions": actions,
            "actions_hash": hex::encode(compose_actions_hash(actions).unwrap()),
            // A JSON-encoded string, as the live record carries it.
            "event_log": event_log(events).to_string(),
            "nonce": nonce,
            "nonce_source": "client",
            "quote": hex::encode(compose_quote()),
            "report_data": "00",
            "vm_config": "{}",
        },
    });
    let gateway = serde_json::json!({
        "signing_algo": "ed25519",
        "signing_address": key,
        "request_nonce": nonce,
        "intel_quote": hex::encode(gateway_quote()),
        "event_log": event_log(events),
    });
    let document = serde_json::json!({
        "gateway_attestation": gateway,
        "model_attestations": [entry],
    });
    (document.to_string(), nonce)
}

type Alter = Box<dyn Fn(usize, &mut VerifiedQuote) + Send + Sync>;

/// The quote verifier a staged report needs. Every quote is verified for
/// real against the capture's collateral; then the registers the synthetic
/// log and record describe are staged on the verified result:
///
/// - the model quote (the capture's bytes) gets RTMR3 replayed from
///   `events` and a config-id v1 MRCONFIGID over the synthetic compose hash;
/// - the compose-manager quote gets the model quote's registers (the same
///   TD, the same boot), our nonce and the actions hash in `report_data`;
/// - the gateway quote gets registers of its own.
///
/// `alter(n, quote)` runs on the `n`th model quote's staged result (from 0)
/// and the compose-manager quote that follows it takes the altered
/// registers, as one TD's would.
struct Staged {
    events: Vec<(String, String)>,
    actions: Vec<serde_json::Value>,
    alter: Alter,
    alter_compose: Alter,
    models: std::sync::atomic::AtomicUsize,
    last: std::sync::Mutex<Option<VerifiedQuote>>,
}

impl Staged {
    fn new() -> Self {
        Self::over(ten_events(), actions())
    }

    fn over(events: Vec<(String, String)>, actions: Vec<serde_json::Value>) -> Self {
        Staged {
            events,
            actions,
            alter: Box::new(|_, _| {}),
            alter_compose: Box::new(|_, _| {}),
            models: std::sync::atomic::AtomicUsize::new(0),
            last: std::sync::Mutex::new(None),
        }
    }

    fn altering(
        mut self,
        alter: impl Fn(usize, &mut VerifiedQuote) + Send + Sync + 'static,
    ) -> Self {
        self.alter = Box::new(alter);
        self
    }

    fn altering_compose(
        mut self,
        alter: impl Fn(usize, &mut VerifiedQuote) + Send + Sync + 'static,
    ) -> Self {
        self.alter_compose = Box::new(alter);
        self
    }

    fn verify(
        &self,
        quote: &[u8],
        collateral: &Collateral,
        now: u64,
    ) -> Result<VerifiedQuote, QuoteVerifyError> {
        let real = fixture_quote();
        let mut verified = verify_quote(&real, collateral, now)?;
        if quote == real.as_slice() {
            verified.rtmr[3] = replayed(&self.events);
            verified.mr_config_id = format!("01{COMPOSE_HASH}{}", "00".repeat(15));
            let n = self
                .models
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (self.alter)(n, &mut verified);
            *self.last.lock().unwrap() = Some(verified.clone());
            Ok(verified)
        } else if quote == compose_quote().as_slice() {
            let model = self
                .last
                .lock()
                .unwrap()
                .clone()
                .expect("model quote first");
            let mut staged = model.clone();
            staged.report_data[0..32]
                .copy_from_slice(&compose_actions_hash(&self.actions).unwrap());
            staged.report_data[32..64].copy_from_slice(&real[600..632]);
            let n = self.models.load(std::sync::atomic::Ordering::SeqCst) - 1;
            (self.alter_compose)(n, &mut staged);
            Ok(staged)
        } else if quote == gateway_quote().as_slice() {
            verified.mrtd = "6a".repeat(48);
            verified.rtmr = [
                "6b".repeat(48),
                "6c".repeat(48),
                "6d".repeat(48),
                "6e".repeat(48),
            ];
            verified.mr_config_id = "6f".repeat(48);
            Ok(verified)
        } else {
            verify_quote(quote, collateral, now)
        }
    }
}

fn staged_verify(
    staged: &Staged,
) -> impl Fn(&[u8], &Collateral, u64) -> Result<VerifiedQuote, QuoteVerifyError> + Sync + '_ {
    move |quote, collateral, now| staged.verify(quote, collateral, now)
}

#[tokio::test]
async fn observe_reads_the_registers_of_a_verified_quote() {
    // The control for the refusals below: the same path, over a quote that
    // verifies and binds the nonce asked for, with a log that replays to it
    // and a compose-manager record bound to it, prints its registers.
    let (report, nonce) = synthetic_ed25519_report();
    let (_server, gateway) = gateway_serving(&report, &nonce).await;
    let staged = Staged::new();

    let observed = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &staged_verify(&staged),
    )
    .await
    .expect("a verified, bound, replayed quote is observed");

    assert_eq!(observed.sets.len(), 1);
    let set = ExpectedMeasurements::from_env_value(Some(&observed.sets[0]))
        .expect("an observed set parses")
        .expect("and is not empty");
    let names: Vec<&str> = set.pinned_fields().iter().map(|f| f.as_str()).collect();
    assert_eq!(names, ["mrtd", "mrconfigid", "rtmr0", "rtmr1", "rtmr2"]);
    assert_eq!(set.to_pin_string(), observed.sets[0], "printed canonically");
    assert!(
        observed.sets[0].contains(&format!("mrconfigid=01{COMPOSE_HASH}")),
        "MRCONFIGID read out of the verified quote"
    );
    // The staging ingest's own pin keeps production's form, from the same
    // observation (spec 5.9): rtmr3 included, as the ingest does not replay.
    let ingest = ExpectedMeasurements::from_env_value(Some(&observed.ingest_pin))
        .unwrap()
        .unwrap();
    let names: Vec<&str> = ingest.pinned_fields().iter().map(|f| f.as_str()).collect();
    assert_eq!(names, ["mrtd", "rtmr0", "rtmr1", "rtmr2", "rtmr3"]);
    assert!(observed.ingest_pin.contains(&replayed(&ten_events())));
}

#[tokio::test]
async fn observe_refuses_a_set_from_a_quote_that_does_not_verify() {
    // A real capture whose JSON binds its own nonce, over a quote the
    // collateral does not verify. Nothing is printed for it.
    let nonce = json(ED25519_REPORT)["model_attestations"][0]["request_nonce"]
        .as_str()
        .unwrap()
        .to_string();
    let (_server, gateway) = gateway_serving(ED25519_REPORT, &nonce).await;

    let refusal = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &verify_quote,
    )
    .await
    .expect_err("an unverified quote yields no set");

    assert_eq!(refusal.model_position, 1);
    assert_eq!(refusal.label, "quote_verification_failed");
}

#[tokio::test]
async fn observe_refuses_a_report_not_bound_to_its_nonce() {
    // The report a replaying gateway would serve: genuine and verifiable,
    // but issued for a nonce we did not send.
    let (report, _its_own_nonce) = synthetic_ed25519_report();
    let ours = "9f3c1d2e4b5a60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9";
    let (_server, gateway) = gateway_serving(&report, ours).await;
    let staged = Staged::new();

    let refusal = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(ours.to_string()),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &staged_verify(&staged),
    )
    .await
    .expect_err("a report for another nonce yields no set");

    assert_eq!(refusal.label, "report_nonce_mismatch");
}

#[tokio::test]
async fn one_refused_model_prints_nothing_for_any() {
    let (good_report, good_nonce) = synthetic_ed25519_report();
    let (_good_server, good) = gateway_serving(&good_report, &good_nonce).await;
    let (_bad_server, bad) = gateway_serving(&good_report, &"0".repeat(64)).await;

    struct Sequence(std::sync::Mutex<Vec<String>>);
    impl DrillNonceSource for Sequence {
        fn nonce(&self) -> String {
            self.0.lock().unwrap().remove(0)
        }
    }
    let nonces = Sequence(std::sync::Mutex::new(vec![good_nonce, "0".repeat(64)]));
    let staged = Staged::new();

    let refusal = observe_measurement_sets(
        &[&good, &bad],
        &nonces,
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &staged_verify(&staged),
    )
    .await
    .expect_err("the second model's refusal refuses the run");
    assert_eq!(refusal.model_position, 2);
}

/// `verify_quote` over the real fixture, then `alter` applied to what it
/// verified: how a test reaches the checks after verification, which no
/// checked-in quote fails on its own. Same shape as the contributor's
/// `proof_attestor` tests.
fn verify_then(
    alter: impl Fn(&mut VerifiedQuote),
) -> impl Fn(&[u8], &Collateral, u64) -> Result<VerifiedQuote, QuoteVerifyError> {
    move |quote, collateral, now| {
        let mut verified = verify_quote(quote, collateral, now)?;
        alter(&mut verified);
        Ok(verified)
    }
}

#[tokio::test]
async fn observe_refuses_a_quote_whose_tcb_is_not_up_to_date() {
    let (report, nonce) = synthetic_ed25519_report();
    let (_server, gateway) = gateway_serving(&report, &nonce).await;

    let refusal = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &verify_then(|quote| quote.tcb_status = "OutOfDate".to_string()),
    )
    .await
    .expect_err("an out-of-date platform yields no set");

    assert_eq!(refusal.model_position, 1);
    assert_eq!(refusal.label, "quote_tcb_not_up_to_date");
}

#[tokio::test]
async fn observe_refuses_a_verified_quote_bound_to_another_nonce() {
    // The JSON binds our nonce, but the verified quote does not: what a
    // gateway relabelling an old quote would serve. Refused at the check
    // against the verified report data, not the JSON one.
    let (report, nonce) = synthetic_ed25519_report();
    let (_server, gateway) = gateway_serving(&report, &nonce).await;
    let staged = Staged::new().altering(|_, quote| quote.report_data[32..64].fill(0x5a));

    let refusal = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &staged_verify(&staged),
    )
    .await
    .expect_err("a quote bound to another nonce yields no set");

    assert_eq!(refusal.model_position, 1);
    assert_eq!(refusal.label, "quote_nonce_mismatch");
}

#[tokio::test]
async fn observe_keeps_the_sets_in_model_order() {
    // The staging ingest is pinned to the first model's quote, so the first
    // set must be the first model's, not whichever sorts lowest. The second
    // model's set is altered to sort first.
    let (report, nonce) = synthetic_ed25519_report();
    let (_first_server, first) = gateway_serving(&report, &nonce).await;
    let (_second_server, second) = gateway_serving(&report, &nonce).await;
    let staged = Staged::new().altering(|n, quote| {
        if n == 1 {
            quote.mrtd = "00".repeat(48);
        }
    });

    let observed = observe_measurement_sets(
        &[&first, &second],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &staged_verify(&staged),
    )
    .await
    .expect("both models observed");

    let sets = observed.sets;
    assert_eq!(sets.len(), 2);
    assert!(sets[1] < sets[0], "the second model's set sorts first");
    assert!(
        sets[1].starts_with(&format!("mrtd={}", "00".repeat(48))),
        "and is listed second, after the first model's"
    );
    assert!(
        !observed
            .ingest_pin
            .starts_with(&format!("mrtd={}", "00".repeat(48))),
        "the ingest pin is the first model's"
    );
}

#[tokio::test]
async fn observe_refuses_a_run_whose_replay_does_not_match() {
    let (report, nonce) = synthetic_ed25519_report();
    let (_server, gateway) = gateway_serving(&report, &nonce).await;
    // The quote measured another log: its RTMR3 is not what the report's
    // log replays to.
    let staged = Staged::new().altering(|_, quote| quote.rtmr[3] = "77".repeat(48));

    let refusal = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce.clone()),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &staged_verify(&staged),
    )
    .await
    .expect_err("a log that does not replay to the quote yields no set");
    assert_eq!(refusal.model_position, 1);
    assert_eq!(refusal.label, "rtmr3_replay_mismatch");

    // A log that replays but is not the ten events in order is refused too.
    let mut nine = ten_events();
    nine.remove(7);
    let (report, nonce) = report_with(&nine, &actions());
    let (_server, gateway) = gateway_serving(&report, &nonce).await;
    let staged = Staged::over(nine, actions());
    let refusal = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &staged_verify(&staged),
    )
    .await
    .expect_err("a log missing its key-provider event yields no set");
    assert_eq!(refusal.label, "rtmr3_event_sequence");

    // All ten, each once, but out of order: refused, though the quote
    // really measured this order. `observe` checks no allowlist, so this
    // is the replay's own order check.
    let mut reordered = ten_events();
    reordered.swap(5, 6);
    let (report, nonce) = report_with(&reordered, &actions());
    let (_server, gateway) = gateway_serving(&report, &nonce).await;
    let staged = Staged::over(reordered, actions());
    let refusal = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &staged_verify(&staged),
    )
    .await
    .expect_err("a reordered log yields no set");
    assert_eq!(refusal.model_position, 1);
    assert_eq!(refusal.label, "rtmr3_event_sequence");
}

#[tokio::test]
async fn observe_refuses_entries_whose_values_disagree() {
    // Two entries for the model, each with a log that replays to its own
    // quote. The allowlist holds one value per event, so a run whose
    // entries carry different ones must say so rather than keep the first.
    let (report, nonce) = synthetic_ed25519_report();
    let mut document = json(&report);
    let first = document["model_attestations"][0].clone();
    document["model_attestations"]
        .as_array_mut()
        .unwrap()
        .push(first);
    let two = document.to_string();

    // The control: two entries that agree are observed.
    let (_server, gateway) = gateway_serving(&two, &nonce).await;
    let staged = Staged::new();
    let observed = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce.clone()),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &staged_verify(&staged),
    )
    .await
    .expect("two agreeing entries are observed");
    assert_eq!(observed.sets.len(), 1);

    // The second entry under another KMS, then another progress marker.
    for (name, payload) in [
        ("mr-kms", "4c".repeat(48)),
        ("storage-fs", hex::encode("another fs")),
    ] {
        let other: Vec<(String, String)> = ten_events()
            .into_iter()
            .map(|(n, p)| {
                if n == name {
                    (n, payload.clone())
                } else {
                    (n, p)
                }
            })
            .collect();
        let mut document = json(&two);
        document["model_attestations"][1]["event_log"] = event_log(&other);
        let (_server, gateway) = gateway_serving(&document.to_string(), &nonce).await;
        let other_rtmr3 = replayed(&other);
        let staged = Staged::new().altering(move |n, quote| {
            if n == 1 {
                quote.rtmr[3] = other_rtmr3.clone();
            }
        });
        let refusal = observe_measurement_sets(
            &[&gateway],
            &FixedNonce(nonce.clone()),
            FIXTURE_CAPTURED_AT,
            &DEFAULT_REGISTERS,
            &staged_verify(&staged),
        )
        .await
        .expect_err("entries that disagree yield no allowlist");
        assert_eq!(refusal.model_position, 1, "{name}");
        assert_eq!(refusal.label, "review_entries_disagree", "{name}");
    }
}

#[tokio::test]
async fn observe_refuses_a_run_whose_compose_manager_check_fails() {
    let cases: [(&str, Staged); 4] = [
        (
            "compose_manager_other_td",
            Staged::new().altering_compose(|_, quote| quote.rtmr[2] = "44".repeat(48)),
        ),
        (
            "compose_manager_nonce_mismatch",
            Staged::new().altering_compose(|_, quote| quote.report_data[32..64].fill(0x11)),
        ),
        (
            "compose_actions_hash_mismatch",
            Staged::new().altering_compose(|_, quote| quote.report_data[0..32].fill(0x22)),
        ),
        (
            "compose_manager_tcb_not_up_to_date",
            Staged::new().altering_compose(|_, quote| quote.tcb_status = "OutOfDate".to_string()),
        ),
    ];
    for (label, staged) in cases {
        let (report, nonce) = synthetic_ed25519_report();
        let (_server, gateway) = gateway_serving(&report, &nonce).await;
        let refusal = observe_measurement_sets(
            &[&gateway],
            &FixedNonce(nonce),
            FIXTURE_CAPTURED_AT,
            &DEFAULT_REGISTERS,
            &staged_verify(&staged),
        )
        .await
        .expect_err("a compose-manager record that does not check yields no set");
        assert_eq!(refusal.label, label);
    }

    // A record whose quote does not verify: the record's quote is the
    // ed25519 capture's, which the checked-in collateral refuses for real.
    let (report, nonce) = synthetic_ed25519_report();
    let mut document = json(&report);
    document["model_attestations"][0]["compose_manager_attestation"]["quote"] =
        json(ED25519_REPORT)["model_attestations"][0]["intel_quote"].clone();
    let (_server, gateway) = gateway_serving(&document.to_string(), &nonce).await;
    let staged = Staged::new();
    let refusal = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce.clone()),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &staged_verify(&staged),
    )
    .await
    .expect_err("an unverified compose-manager quote yields no set");
    assert_eq!(refusal.label, "compose_manager_quote_unverified");

    // And a model entry with no record at all.
    let mut document = json(&report);
    document["model_attestations"][0]
        .as_object_mut()
        .unwrap()
        .remove("compose_manager_attestation");
    let (_server, gateway) = gateway_serving(&document.to_string(), &nonce).await;
    let staged = Staged::new();
    let refusal = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &staged_verify(&staged),
    )
    .await
    .expect_err("no compose-manager record, no set");
    assert_eq!(refusal.label, "compose_manager_missing");
}

#[tokio::test]
async fn observe_prints_the_compose_actions_for_review() {
    let (report, nonce) = synthetic_ed25519_report();
    let (_server, gateway) = gateway_serving(&report, &nonce).await;
    let staged = Staged::new();

    let observed = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &staged_verify(&staged),
    )
    .await
    .expect("observed");

    // The review is a candidate allowlist in the exact shape `sign` reads:
    // the replayed events' values and every compose action seen.
    let review = observed.review.to_json();
    assert_eq!(review["rtmr3"]["app_id"], APP_ID);
    assert_eq!(review["rtmr3"]["mr_kms"], MR_KMS);
    assert_eq!(review["rtmr3"]["os_image_hash"], OS_IMAGE_HASH);
    assert_eq!(review["rtmr3"]["key_provider"], KEY_PROVIDER);
    let markers: Vec<&str> = review["rtmr3"]["markers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(markers, MARKER_EVENTS);
    // Owner decision S17: the distinct compose-manager images and the
    // distinct compose files brought up, never services, tags or times.
    assert_eq!(
        review["compose_actions"],
        serde_json::json!({
            "compose_manager_images": [manager_image()],
            "compose_files": [
                {"file": COMPOSE_FILE, "file_sha256": FILE_SHA256, "commit": COMMIT},
            ],
        })
    );
    assert_eq!(
        trace_commons_attestation::staging_replay::Allowlist::from_json(&review).unwrap(),
        observed.review,
        "the review reads back as an allowlist"
    );
}

#[tokio::test]
async fn observe_refuses_a_compose_down_no_compose_up_in_the_run_brought_up() {
    // A compose_down carries no file hash, so the review cannot list it
    // unless a compose_up of the same file and commit did. Rather than write
    // an allowlist that refuses the live record, observe refuses the run.
    let mut downed = actions();
    downed[2]["commit"] = serde_json::json!("c2".repeat(20));
    let (report, nonce) = report_with(&ten_events(), &downed);
    let (_server, gateway) = gateway_serving(&report, &nonce).await;
    let staged = Staged::over(ten_events(), downed);
    let refusal = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &staged_verify(&staged),
    )
    .await
    .expect_err("a compose_down the review cannot list");
    assert_eq!(refusal.model_position, 0);
    assert_eq!(refusal.label, "review_compose_down_unlisted");
}

#[tokio::test]
async fn observe_refuses_an_unknown_or_malformed_compose_action() {
    let mut unknown = actions();
    unknown[1]["action"] = serde_json::json!("compose_restart");
    let mut extra = actions();
    extra[1]["container"] = serde_json::json!("model");
    let mut numeric_time = actions();
    numeric_time[0]["timestamp"] = serde_json::json!(1);
    let mut tagged_image = actions();
    tagged_image[0]["image"] = serde_json::json!("registry.example/compose-manager:latest");
    for (actions, label) in [
        (unknown, "compose_action_unknown"),
        (extra, "compose_manager_malformed"),
        (numeric_time, "compose_manager_malformed"),
        // Readable, but not a value the allowlist can list.
        (tagged_image, "review_compose_actions_unlistable"),
    ] {
        let (report, nonce) = report_with(&ten_events(), &actions);
        let (_server, gateway) = gateway_serving(&report, &nonce).await;
        let staged = Staged::over(ten_events(), actions);
        let refusal = observe_measurement_sets(
            &[&gateway],
            &FixedNonce(nonce),
            FIXTURE_CAPTURED_AT,
            &DEFAULT_REGISTERS,
            &staged_verify(&staged),
        )
        .await
        .expect_err("refused");
        assert_eq!(refusal.label, label);
    }
}

#[tokio::test]
async fn observe_never_emits_a_gateway_or_compose_manager_quote_as_a_pin_set() {
    // The report's gateway quote verifies and carries registers of its own
    // (the staged verifier gives it 6a.. 6f..); the compose-manager quote
    // verifies with the actions hash in its report data. Neither is a set.
    let (report, nonce) = synthetic_ed25519_report();
    let (_server, gateway) = gateway_serving(&report, &nonce).await;
    let staged = Staged::new();

    let observed = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &staged_verify(&staged),
    )
    .await
    .expect("observed");
    assert_eq!(observed.sets.len(), 1, "one set: the model entry's");
    let printed = format!("{}\n{}", observed.sets.join("\n"), observed.ingest_pin);
    for register in ["6a", "6b", "6c", "6d", "6e", "6f"] {
        assert!(
            !printed.contains(&register.repeat(48)),
            "a gateway register reached the output"
        );
    }
    let actions_hash = hex::encode(compose_actions_hash(&actions()).unwrap());
    assert!(!printed.contains(&actions_hash));
}

#[tokio::test]
async fn observe_never_prints_the_instance_id_value() {
    let (report, nonce) = synthetic_ed25519_report();
    let (_server, gateway) = gateway_serving(&report, &nonce).await;
    let staged = Staged::new();

    let observed = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &staged_verify(&staged),
    )
    .await
    .expect("observed");
    let everything = format!(
        "{}\n{}\n{}\n{:?}",
        observed.sets.join("\n"),
        observed.ingest_pin,
        serde_json::to_string_pretty(&observed.review.to_json()).unwrap(),
        observed,
    );
    for form in [INSTANCE_ID.to_string(), INSTANCE_ID.to_ascii_uppercase()] {
        assert!(!everything.contains(&form), "the instance id was printed");
    }
    assert!(
        !everything.contains("instance"),
        "not even its name has a field"
    );
}

/// The live client, collateral and all, in a build without the collateral
/// client: `observe` refuses with that control's name rather than printing
/// a set it could not verify.
#[cfg(not(feature = "near-attestation-collateral"))]
#[tokio::test]
async fn observe_without_the_collateral_client_names_the_missing_control() {
    let (report, nonce) = synthetic_ed25519_report();
    let (_server, gateway) = gateway_serving(&report, &nonce).await;

    let refusal = observe_measurement_sets(
        &[&gateway.http],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &verify_quote,
    )
    .await
    .expect_err("no collateral, no set");
    assert_eq!(
        refusal.label,
        "collateral_missing_control:near_ai_attestation_collateral_client"
    );
}

// ---------------------------------------------------------------------------
// sign, verify and public-key, as the built binary
// ---------------------------------------------------------------------------

fn synthetic_seed_hex() -> String {
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    hex::encode(seed)
}

fn public_key_hex(seed_hex: &str) -> String {
    let seed = hex::decode(seed_hex).unwrap();
    let pair = Ed25519KeyPair::from_seed_unchecked(&seed).unwrap();
    hex::encode(pair.public_key().as_ref())
}

/// Two synthetic sets in the S16 form: registers no enclave has, 96 hex
/// characters each.
fn synthetic_sets() -> [String; 2] {
    let set = |byte: &str| {
        ["mrtd", "mrconfigid", "rtmr0", "rtmr1", "rtmr2"]
            .iter()
            .map(|field| format!("{field}={}", byte.repeat(48)))
            .collect::<Vec<_>>()
            .join(",")
    };
    [set("a1"), set("b2")]
}

/// The staging ingest's own pin, in production's form, as `observe` prints
/// it.
fn synthetic_ingest_pin() -> String {
    ["mrtd", "rtmr0", "rtmr1", "rtmr2", "rtmr3"]
        .iter()
        .map(|field| format!("{field}={}", "c3".repeat(48)))
        .collect::<Vec<_>>()
        .join(",")
}

/// A synthetic allowlist file, in the shape `sign --allowlist` reads.
fn synthetic_allowlist() -> serde_json::Value {
    serde_json::json!({
        "rtmr3": {
            "app_id": "a1".repeat(20),
            "mr_kms": "3b".repeat(48),
            "os_image_hash": "05".repeat(32),
            "key_provider": "6b6d73",
            "markers": [
                {"name": "system-preparing", "payload": ""},
                {"name": "boot-mr-done", "payload": ""},
                {"name": "storage-fs", "payload": "00"},
                {"name": "system-ready", "payload": "00"},
            ],
        },
        "compose_actions": {
            "compose_manager_images": [manager_image()],
            "compose_files": [
                {"file": COMPOSE_FILE, "file_sha256": FILE_SHA256, "commit": COMMIT},
            ],
        },
    })
}

fn run(args: &[&str], stdin: &str, dir: &Path) -> Output {
    let mut child = Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the tool starts");
    // A tool that refuses before reading its input may exit before this
    // write: a broken pipe is that refusal, not a failure, and what it
    // printed and how it exited are what the test asserts.
    match child.stdin.take().unwrap().write_all(stdin.as_bytes()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
        Err(error) => panic!("writing the tool's stdin: {error}"),
    }
    child.wait_with_output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn write_sets(dir: &Path) -> std::path::PathBuf {
    let sets = dir.join("observed.txt");
    std::fs::write(
        &sets,
        format!(
            "{}\ningest={}\n",
            synthetic_sets().join("\n"),
            synthetic_ingest_pin()
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("allowlist.json"),
        serde_json::to_vec_pretty(&synthetic_allowlist()).unwrap(),
    )
    .unwrap();
    sets
}

#[test]
fn sign_then_verify_round_trips_through_the_binary() {
    let dir = tempfile::tempdir().unwrap();
    let seed = synthetic_seed_hex();
    write_sets(dir.path());

    let signed = run(
        &[
            "sign",
            "--allowlist",
            "allowlist.json",
            "--sets",
            "observed.txt",
            "--sequence",
            "7",
            "--ttl-days",
            "7",
            "--first",
        ],
        &seed,
        dir.path(),
    );
    assert!(signed.status.success(), "stderr: {}", text(&signed.stderr));

    let document = std::fs::read(dir.path().join("current.json")).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let public_key = hex::decode(public_key_hex(&seed)).unwrap();
    let verified = staging_pins::verify(&document, &public_key, now, STAGING_GATEWAY_HOST)
        .expect("the app's verifier accepts what the tool signed");
    assert_eq!(verified.sequence, 7);
    assert_eq!(verified.sets.len(), 2);
    assert_eq!(verified.expires_at - verified.issued_at, 7 * 86_400);
    assert_eq!(
        verified.allowlist,
        trace_commons_attestation::staging_replay::Allowlist::from_json(&synthetic_allowlist())
            .unwrap(),
        "the allowlist file is inside the signed payload"
    );

    let checked = run(
        &[
            "verify",
            "current.json",
            "--public-key",
            &public_key_hex(&seed),
        ],
        "",
        dir.path(),
    );
    assert!(
        checked.status.success(),
        "stderr: {}",
        text(&checked.stderr)
    );

    let shown = run(&["public-key"], &seed, dir.path());
    assert!(shown.status.success());
    assert!(text(&shown.stdout).contains(&public_key_hex(&seed)));
    assert!(text(&shown.stdout).contains(&staging_pins::key_id(&public_key)));

    let another = synthetic_seed_hex();
    let refused = run(
        &[
            "verify",
            "current.json",
            "--public-key",
            &public_key_hex(&another),
        ],
        "",
        dir.path(),
    );
    assert!(!refused.status.success(), "another key's verify refuses");
    assert!(text(&refused.stderr).contains("pins_unknown_key"));
}

#[test]
fn sign_never_reads_a_key_from_argv() {
    let dir = tempfile::tempdir().unwrap();
    let seed = synthetic_seed_hex();
    write_sets(dir.path());
    let base = [
        "sign",
        "--allowlist",
        "allowlist.json",
        "--sets",
        "observed.txt",
        "--sequence",
        "1",
        "--ttl-days",
        "7",
        "--first",
    ];

    for flag in [
        "--key",
        "--private-key",
        "--seed",
        "--signing-key",
        "--key-file",
    ] {
        let mut args = base.to_vec();
        args.extend([flag, seed.as_str()]);
        let out = run(&args, "", dir.path());
        assert_eq!(out.status.code(), Some(2), "{flag} is not an argument");
        assert!(!dir.path().join("current.json").exists());
    }

    // A key in argv where a positional could take it is refused too, and
    // with nothing on stdin nothing is signed.
    let mut args = base.to_vec();
    args.push(seed.as_str());
    assert_eq!(run(&args, "", dir.path()).status.code(), Some(2));

    let out = run(&base, "", dir.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("signing_key_missing"));
    assert!(!dir.path().join("current.json").exists());
}

#[test]
fn sign_refuses_an_existing_output_before_reading_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let seed = synthetic_seed_hex();
    write_sets(dir.path());
    let args = [
        "sign",
        "--allowlist",
        "allowlist.json",
        "--sets",
        "observed.txt",
        "--sequence",
        "1",
        "--ttl-days",
        "7",
        "--first",
    ];

    let first = run(&args, &seed, dir.path());
    assert!(first.status.success(), "stderr: {}", text(&first.stderr));
    let written = std::fs::read(dir.path().join("current.json")).unwrap();

    let again = run(&args, &seed, dir.path());
    assert_eq!(again.status.code(), Some(1));
    assert!(text(&again.stderr).contains("refused: output_exists"));
    assert!(again.stdout.is_empty());
    assert_eq!(
        std::fs::read(dir.path().join("current.json")).unwrap(),
        written,
        "the existing document is untouched"
    );

    // With no key at all, the existing output is still what is reported:
    // the check comes before the key is read.
    let keyless = run(&args, "", dir.path());
    assert!(
        text(&keyless.stderr).contains("refused: output_exists"),
        "stderr: {}",
        text(&keyless.stderr)
    );
}

#[test]
fn observe_without_a_key_prints_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(
        &["observe", "--model", "m", "--review", "review.json"],
        "",
        dir.path(),
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_eq!(text(&out.stderr).trim(), "refused: api_key_missing");
    assert!(
        !dir.path().join("review.json").exists(),
        "no review written"
    );

    // An existing review file is refused before the key is read.
    std::fs::write(dir.path().join("review.json"), b"kept").unwrap();
    let out = run(
        &["observe", "--model", "m", "--review", "review.json"],
        "",
        dir.path(),
    );
    assert_eq!(text(&out.stderr).trim(), "refused: review_exists");
    assert_eq!(
        std::fs::read(dir.path().join("review.json")).unwrap(),
        b"kept"
    );
}

#[test]
fn nothing_logs_a_key_or_a_measurement() {
    let dir = tempfile::tempdir().unwrap();
    let seed = synthetic_seed_hex();
    let public_key = public_key_hex(&seed);
    write_sets(dir.path());
    let sets = synthetic_sets();

    let mut outputs = Vec::new();
    // Success.
    outputs.push(run(
        &[
            "sign",
            "--allowlist",
            "allowlist.json",
            "--sets",
            "observed.txt",
            "--sequence",
            "3",
            "--ttl-days",
            "7",
            "--first",
        ],
        &seed,
        dir.path(),
    ));
    let signed = outputs[0].clone();
    assert!(signed.status.success(), "stderr: {}", text(&signed.stderr));
    outputs.push(run(
        &["verify", "current.json", "--public-key", &public_key],
        "",
        dir.path(),
    ));
    // Refusals: a sequence not above the served one, a TTL too long, a
    // malformed key.
    outputs.push(run(
        &[
            "sign",
            "--allowlist",
            "allowlist.json",
            "--sets",
            "observed.txt",
            "--sequence",
            "3",
            "--ttl-days",
            "7",
            "--current",
            "current.json",
            "--out",
            "next.json",
        ],
        &seed,
        dir.path(),
    ));
    outputs.push(run(
        &[
            "sign",
            "--allowlist",
            "allowlist.json",
            "--sets",
            "observed.txt",
            "--sequence",
            "4",
            "--ttl-days",
            "15",
            "--current",
            "current.json",
            "--out",
            "next.json",
        ],
        &seed,
        dir.path(),
    ));
    outputs.push(run(
        &[
            "sign",
            "--allowlist",
            "allowlist.json",
            "--sets",
            "observed.txt",
            "--sequence",
            "4",
            "--ttl-days",
            "7",
            "--current",
            "current.json",
            "--out",
            "next.json",
        ],
        &format!("{seed}zz"),
        dir.path(),
    ));
    assert!(!dir.path().join("next.json").exists());

    let measurement_values: Vec<String> = sets
        .iter()
        .chain(std::iter::once(&synthetic_ingest_pin()))
        .flat_map(|set| {
            set.split(',')
                .map(|kv| kv.split_once('=').unwrap().1.to_string())
        })
        .collect();
    for out in &outputs {
        let stderr = text(&out.stderr);
        assert!(!stderr.contains(&seed), "stderr carries the seed: {stderr}");
        assert!(
            !stderr.contains(&public_key),
            "stderr carries the key: {stderr}"
        );
        for value in &measurement_values {
            assert!(
                !stderr.contains(value.as_str()),
                "stderr carries a measurement"
            );
        }
    }
    // The document's own bytes: the measurements sit base64url-encoded in
    // `payload`, so the raw-hex search above cannot see a leaked document.
    // Neither the payload nor the signature, nor any 16-character piece of
    // either, reaches stderr or a stdout.
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("current.json")).unwrap()).unwrap();
    let encoded: Vec<String> = ["payload", "signature"]
        .iter()
        .map(|field| document[field].as_str().unwrap().to_string())
        .collect();
    for out in &outputs {
        for stream in [text(&out.stderr), text(&out.stdout)] {
            for value in &encoded {
                for start in 0..=value.len() - 16 {
                    let piece = &value[start..start + 16];
                    assert!(
                        !stream.contains(piece),
                        "a document byte reaches the output: {piece}"
                    );
                }
            }
        }
    }
    for out in &outputs[1..] {
        let stdout = text(&out.stdout);
        assert!(!stdout.contains(&seed));
        for value in &measurement_values {
            assert!(!stdout.contains(value.as_str()), "only sign prints a set");
        }
    }

    // sign's stdout is the one line the staging ingest's drop-in takes, in
    // production's form from the `ingest=` line, and nothing else: never an
    // S16 set, which the ingest cannot use without the replay.
    let stdout = text(&signed.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        [format!(
            "TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS={}",
            synthetic_ingest_pin()
        )]
    );
}

#[test]
fn sign_embeds_the_allowlist_file() {
    let dir = tempfile::tempdir().unwrap();
    let seed = synthetic_seed_hex();
    write_sets(dir.path());
    let args = |out: &'static str| {
        [
            "sign",
            "--allowlist",
            "allowlist.json",
            "--sets",
            "observed.txt",
            "--sequence",
            "5",
            "--ttl-days",
            "7",
            "--first",
            "--out",
            out,
        ]
    };

    let signed = run(&args("current.json"), &seed, dir.path());
    assert!(signed.status.success(), "stderr: {}", text(&signed.stderr));
    let document = std::fs::read(dir.path().join("current.json")).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let public_key = hex::decode(public_key_hex(&seed)).unwrap();
    let verified =
        staging_pins::verify(&document, &public_key, now, STAGING_GATEWAY_HOST).expect("verifies");
    assert_eq!(verified.allowlist.to_json(), synthetic_allowlist());
    // The summary names the allowlist by hash, never its values.
    let stderr = text(&signed.stderr);
    assert!(stderr.contains("allowlist_sha256=sha256:"), "{stderr}");
    assert!(!stderr.contains(&"3b".repeat(48)), "{stderr}");
    assert!(!stderr.contains("registry.example"), "{stderr}");

    // The refuse-everything placeholder signs, and refuses every entry in
    // the app (its values match no event, its lists are empty).
    let placeholder = r#"{
  "rtmr3": {"app_id": "", "mr_kms": "", "os_image_hash": "", "key_provider": "", "markers": []},
  "compose_actions": {"compose_manager_images": [], "compose_files": []}
}"#;
    std::fs::write(dir.path().join("allowlist.json"), placeholder).unwrap();
    let signed = run(&args("placeholder.json"), &seed, dir.path());
    assert!(signed.status.success(), "stderr: {}", text(&signed.stderr));
    let document = std::fs::read(dir.path().join("placeholder.json")).unwrap();
    let verified = staging_pins::verify(&document, &public_key, now, STAGING_GATEWAY_HOST)
        .expect("the placeholder verifies");
    assert!(verified.allowlist.compose_actions.is_empty());
    assert!(verified.allowlist.rtmr3.markers.is_empty());
    assert!(verified.allowlist.rtmr3.app_id.is_empty());

    // A malformed or missing allowlist file signs nothing.
    std::fs::write(dir.path().join("allowlist.json"), b"{\"rtmr3\": {}}").unwrap();
    let refused = run(&args("bad.json"), &seed, dir.path());
    assert_eq!(refused.status.code(), Some(1));
    assert!(text(&refused.stderr).contains("refused: allowlist_malformed"));
    assert!(!dir.path().join("bad.json").exists());
    std::fs::remove_file(dir.path().join("allowlist.json")).unwrap();
    let refused = run(&args("bad.json"), &seed, dir.path());
    assert!(text(&refused.stderr).contains("refused: allowlist_unreadable"));
    assert!(!dir.path().join("bad.json").exists());
}

/// The helper itself: a tool that refuses before reading stdin can exit
/// before the test has written it, and the write then fails with a broken
/// pipe. That is the tool behaving as specified, not a failure, so `run`
/// returns the tool's output either way. A megabyte of stdin, past any
/// pipe buffer, makes the race certain instead of occasional.
#[test]
fn run_survives_a_tool_that_exits_before_reading_its_input() {
    let dir = tempfile::tempdir().unwrap();
    let seed = synthetic_seed_hex();
    write_sets(dir.path());
    let args = [
        "sign",
        "--allowlist",
        "allowlist.json",
        "--sets",
        "observed.txt",
        "--sequence",
        "1",
        "--ttl-days",
        "7",
        "--first",
    ];
    assert!(run(&args, &seed, dir.path()).status.success());
    let flood = "0".repeat(1 << 20);
    let again = run(&args, &flood, dir.path());
    assert_eq!(again.status.code(), Some(1));
    assert!(text(&again.stderr).contains("refused: output_exists"));
}
