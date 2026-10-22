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

/// A **synthetic** ed25519 report over the ECDSA capture's quote, which
/// verifies. The key is the quote's own `report_data[0..32]` and the nonce
/// its `[32..64]`, so the report is consistent by construction. Same recipe
/// as `near_attestation::key_drift_tests`.
fn synthetic_ed25519_report() -> (String, String) {
    let quote_hex = json(ECDSA_REPORT)["intel_quote"]
        .as_str()
        .unwrap()
        .to_string();
    let quote = hex::decode(&quote_hex).unwrap();
    let report_data = &quote[568..632];
    let key = hex::encode(&report_data[0..32]);
    let nonce = hex::encode(&report_data[32..64]);
    let entry = serde_json::json!({
        "model_name": MODEL,
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

#[tokio::test]
async fn observe_reads_the_registers_of_a_verified_quote() {
    // The control for the two refusals below: the same path, over a quote
    // that verifies and binds the nonce asked for, prints its registers.
    let (report, nonce) = synthetic_ed25519_report();
    let (_server, gateway) = gateway_serving(&report, &nonce).await;

    let sets = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &verify_quote,
    )
    .await
    .expect("a verified, bound quote is observed");

    assert_eq!(sets.len(), 1);
    let set = ExpectedMeasurements::from_env_value(Some(&sets[0]))
        .expect("an observed set parses")
        .expect("and is not empty");
    let names: Vec<&str> = set.pinned_fields().iter().map(|f| f.as_str()).collect();
    assert_eq!(names, ["mrtd", "rtmr0", "rtmr1", "rtmr2", "rtmr3"]);
    assert_eq!(set.to_pin_string(), sets[0], "printed canonically");
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

    let refusal = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(ours.to_string()),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &verify_quote,
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

    let refusal = observe_measurement_sets(
        &[&good, &bad],
        &nonces,
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &verify_quote,
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

    let refusal = observe_measurement_sets(
        &[&gateway],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &verify_then(|quote| quote.report_data[32..64].fill(0x5a)),
    )
    .await
    .expect_err("a quote bound to another nonce yields no set");

    assert_eq!(refusal.model_position, 1);
    assert_eq!(refusal.label, "quote_nonce_mismatch");
}

#[tokio::test]
async fn observe_keeps_the_sets_in_model_order() {
    // The staging ingest is pinned to the first set, so the first set must
    // be the first model's, not whichever sorts lowest. The second model's
    // set is altered to sort first.
    let (report, nonce) = synthetic_ed25519_report();
    let (_first_server, first) = gateway_serving(&report, &nonce).await;
    let (_second_server, second) = gateway_serving(&report, &nonce).await;

    let calls = std::sync::atomic::AtomicUsize::new(0);
    let verify = |quote: &[u8], collateral: &Collateral, now: u64| {
        let mut verified = verify_quote(quote, collateral, now)?;
        if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            verified.mrtd = "00".repeat(48);
        }
        Ok(verified)
    };

    let sets = observe_measurement_sets(
        &[&first, &second],
        &FixedNonce(nonce),
        FIXTURE_CAPTURED_AT,
        &DEFAULT_REGISTERS,
        &verify,
    )
    .await
    .expect("both models observed");

    assert_eq!(sets.len(), 2);
    assert!(sets[1] < sets[0], "the second model's set sorts first");
    assert!(
        sets[1].starts_with(&format!("mrtd={}", "00".repeat(48))),
        "and is listed second, after the first model's"
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

/// Two synthetic sets: registers no enclave has, 96 hex characters each.
fn synthetic_sets() -> [String; 2] {
    let set = |byte: &str| {
        ["mrtd", "rtmr0", "rtmr1", "rtmr2", "rtmr3"]
            .iter()
            .map(|field| format!("{field}={}", byte.repeat(48)))
            .collect::<Vec<_>>()
            .join(",")
    };
    [set("a1"), set("b2")]
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
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn write_sets(dir: &Path) -> std::path::PathBuf {
    let sets = dir.join("observed.txt");
    std::fs::write(&sets, synthetic_sets().join("\n") + "\n").unwrap();
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
    let out = run(&["observe", "--model", "m"], "", dir.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_eq!(text(&out.stderr).trim(), "refused: api_key_missing");
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
    for out in &outputs[1..] {
        let stdout = text(&out.stdout);
        assert!(!stdout.contains(&seed));
        for value in &measurement_values {
            assert!(!stdout.contains(value.as_str()), "only sign prints a set");
        }
    }

    // sign's stdout is the one line the staging ingest's drop-in takes, for
    // the first set, and nothing else.
    let stdout = text(&signed.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        [format!(
            "TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS={}",
            sets[0]
        )]
    );
}
