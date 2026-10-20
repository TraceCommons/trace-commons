//! Proof status, end to end, against a fake NEAR AI.
//!
//! The fake plays the enclave honestly: it hashes the request body it
//! received and the response body it sent, and signs `<model>:<req>:<resp>`
//! with its own ed25519 key -- exactly what NEAR AI's `GET /v1/signature/{id}`
//! serves. So a row reaching `verified` here means the digests the proxy took
//! on the wire agree with the ones the upstream took, the signature checks,
//! and the key is one the attestor vouched for. The attestor is a fake too:
//! quote verification is the part this crate does not have yet, and the tests
//! that pin its absence live beside `proof::NoQuoteVerification`.

use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use ed25519_dalek::{Signer, SigningKey};
use ironwire_core::config::Config;
use ironwire_core::protocol::ModelTier;
use ironwire_creds::ConsentLedger;
use ironwire_ledger::bodies::{BodyStore, sha256_hex};
use ironwire_ledger::{Exchange, Ledger, ProofStatus};
use ironwire_proxy::proof::{Attestation, ProofSettings, SignerAttestor, run_once};
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::openai_chat::ChatCompletionsBackend;
use secrecy::SecretString;
use tower::ServiceExt;

const MODEL: &str = "Qwen/Qwen3.6-35B-A3B-FP8";
const HOSTED_ID: &str = "c54961ab1d594cf591e5566caa21196b";
const BROKERED_ID: &str = "chatcmpl-brokered";
const FLAKY_ID: &str = "0123456789abcdef0123456789abcdef";

fn enclave_key() -> SigningKey {
    SigningKey::from_bytes(&[42; 32])
}

fn enclave_public() -> String {
    hex::encode(enclave_key().verifying_key().as_bytes())
}

fn response_body() -> String {
    format!(
        r#"{{"id":"{HOSTED_ID}","object":"chat.completion","model":"{MODEL}","choices":[{{"index":0,"message":{{"role":"assistant","content":"hi"}},"finish_reason":"stop"}}],"usage":{{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}}}"#
    )
}

#[derive(Default)]
struct Seen {
    /// Digests the "enclave" took of what it received and sent.
    digests: Option<(String, String)>,
    /// Every signature request: (id, query, authorization header).
    lookups: Vec<(String, std::collections::HashMap<String, String>, String)>,
    /// Answer 404 for this many hosted lookups first, as a provider that has
    /// not written its record yet would.
    lag: usize,
}

type Shared = Arc<Mutex<Seen>>;

async fn completions(State(seen): State<Shared>, body: Bytes) -> impl IntoResponse {
    let response = response_body();
    seen.lock().expect("lock").digests = Some((sha256_hex(&body), sha256_hex(response.as_bytes())));
    ([("content-type", "application/json")], response)
}

async fn signature(
    State(seen): State<Shared>,
    Path(id): Path<String>,
    Query(query): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let (digests, lagging) = {
        let mut seen = seen.lock().expect("lock");
        seen.lookups.push((id.clone(), query, auth));
        let lagging = seen.lag > 0;
        seen.lag = seen.lag.saturating_sub(1);
        (seen.digests.clone(), lagging)
    };
    match id.as_str() {
        HOSTED_ID if lagging => StatusCode::NOT_FOUND.into_response(),
        HOSTED_ID => {
            let (request, response) = digests.expect("a completion was served first");
            let text = format!("{MODEL}:{request}:{response}");
            axum::Json(serde_json::json!({
                "text": text,
                "signature": hex::encode(enclave_key().sign(text.as_bytes()).to_bytes()),
                "signing_address": enclave_public(),
                "signing_algo": "ed25519",
                "signature_kind": "provider_tee",
            }))
            .into_response()
        }
        BROKERED_ID => StatusCode::NOT_FOUND.into_response(),
        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

async fn fake_near() -> (String, Shared) {
    let seen: Shared = Arc::default();
    let router = axum::Router::new()
        .route("/v1/chat/completions", post(completions))
        .route("/v1/signature/{id}", get(signature))
        .with_state(Arc::clone(&seen));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{addr}/v1"), seen)
}

struct Vouches(Vec<String>);

#[async_trait::async_trait]
impl SignerAttestor for Vouches {
    async fn model_keys(&self, _backend: &str, _model: &str) -> Attestation {
        Attestation::Keys(self.0.clone())
    }
}

fn registry(base: &str) -> BackendRegistry {
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        ChatCompletionsBackend::nearai(
            Some(SecretString::from("near-key")),
            Some(base.to_string()),
            vec![(MODEL.to_string(), ModelTier::Frontier)],
            30,
        )
        .expect("client builds"),
    ));
    registry
}

fn chat_request() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/openai/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(format!(
            r#"{{"model":"{MODEL}","messages":[{{"role":"user","content":"hi"}}]}}"#
        )))
        .expect("request builds")
}

async fn first_row(ledger: &Ledger) -> Exchange {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(row) = ledger.recent(1).expect("reads").into_iter().next() {
            return row;
        }
        assert!(tokio::time::Instant::now() < deadline, "no ledger entry");
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

fn row(backend: &str, upstream_id: &str, proof: ProofStatus) -> Exchange {
    Exchange {
        id: None,
        started_at: chrono::Utc::now(),
        ttfb_ms: None,
        total_ms: Some(10),
        facade: "openai".into(),
        path: "/v1/chat/completions".into(),
        conversation: "c-1".into(),
        client_session_id: None,
        backend: backend.into(),
        requested_model: Some(MODEL.into()),
        served_model: Some(MODEL.into()),
        upstream_id: Some(upstream_id.into()),
        model_alias_resolved: None,
        request_sha256: Some("aa".repeat(32)),
        response_sha256: Some("bb".repeat(32)),
        body_ref: None,
        rung: "same_model".into(),
        attempts: 1,
        input_tokens: Some(3),
        cache_read_tokens: None,
        cache_write_tokens: None,
        output_tokens: Some(1),
        cost_usd: Some(0.01),
        substitutions: None,
        status: 200,
        error: None,
        confidence: None,
        proof: Some(proof),
    }
}

/// The whole path: a real request through the façade, recorded `pending`
/// without the answer changing, then settled `verified` by the background
/// check against the upstream's own digests.
#[tokio::test]
async fn a_routed_answer_is_recorded_pending_and_settles_verified() {
    let (base, seen) = fake_near().await;
    let ledger = Ledger::in_memory().expect("ledger");
    let bodies = tempfile::tempdir().expect("tempdir");
    let registry = registry(&base);
    let state = AppState::new(
        registry.clone(),
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    )
    .with_ledger(Some(ledger.clone()))
    .with_bodies(Some(Arc::new(
        BodyStore::open(bodies.path()).expect("body store"),
    )));

    let response = app(state).oneshot(chat_request()).await.expect("answers");
    assert_eq!(response.status(), StatusCode::OK);
    let delivered = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    assert_eq!(
        delivered,
        response_body().as_bytes(),
        "the answer is forwarded untouched"
    );

    let recorded = first_row(&ledger).await;
    assert_eq!(recorded.proof, Some(ProofStatus::Pending));
    assert_eq!(recorded.upstream_id.as_deref(), Some(HOSTED_ID));
    assert!(
        seen.lock().expect("lock").lookups.is_empty(),
        "no receipt is fetched on the response path"
    );

    let round = run_once(
        &ledger,
        &registry,
        &Vouches(vec![enclave_public()]),
        &ProofSettings::default(),
    )
    .await;
    assert_eq!(
        round.settled,
        vec![(recorded.id.expect("id"), ProofStatus::Verified)]
    );
    assert_eq!(
        ledger.recent(1).expect("reads")[0].proof,
        Some(ProofStatus::Verified)
    );

    let lookups = &seen.lock().expect("lock").lookups;
    assert_eq!(lookups.len(), 1);
    let (id, query, auth) = &lookups[0];
    assert_eq!(id, HOSTED_ID);
    assert_eq!(query.get("model").map(String::as_str), Some(MODEL));
    assert_eq!(
        query.get("signing_algo").map(String::as_str),
        Some("ed25519")
    );
    assert_eq!(auth, "Bearer near-key", "the key goes to its own host");
}

/// The same honest receipt, with a key nobody vouched for, is not proof.
#[tokio::test]
async fn an_unvouched_key_is_not_proof() {
    let (base, _seen) = fake_near().await;
    let ledger = Ledger::in_memory().expect("ledger");
    let bodies = tempfile::tempdir().expect("tempdir");
    let registry = registry(&base);
    let state = AppState::new(
        registry.clone(),
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    )
    .with_ledger(Some(ledger.clone()))
    .with_bodies(Some(Arc::new(
        BodyStore::open(bodies.path()).expect("body store"),
    )));
    let response = app(state).oneshot(chat_request()).await.expect("answers");
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;
    first_row(&ledger).await;

    run_once(
        &ledger,
        &registry,
        &Vouches(vec!["cd".repeat(32)]),
        &ProofSettings::default(),
    )
    .await;
    assert_eq!(
        ledger.recent(1).expect("reads")[0].proof,
        Some(ProofStatus::Failed)
    );
}

/// Without body capture there is no digest to check a receipt against, so
/// nothing is fetched and the row says so.
#[tokio::test]
async fn without_captured_digests_a_routed_answer_is_unavailable() {
    let (base, seen) = fake_near().await;
    let ledger = Ledger::in_memory().expect("ledger");
    let registry = registry(&base);
    let state = AppState::new(
        registry.clone(),
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    )
    .with_ledger(Some(ledger.clone()));
    let response = app(state).oneshot(chat_request()).await.expect("answers");
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;
    first_row(&ledger).await;

    run_once(
        &ledger,
        &registry,
        &Vouches(vec![enclave_public()]),
        &ProofSettings::default(),
    )
    .await;
    assert_eq!(
        ledger.recent(1).expect("reads")[0].proof,
        Some(ProofStatus::Unavailable)
    );
    assert!(seen.lock().expect("lock").lookups.is_empty());
}

/// A brokered model 404s permanently: unavailable at once. A flaky endpoint is
/// retried, a bounded number of times, and never promoted to proof.
#[tokio::test]
async fn retries_are_bounded_and_no_receipt_is_not_a_failure() {
    let (base, seen) = fake_near().await;
    let ledger = Ledger::in_memory().expect("ledger");
    let registry = registry(&base);
    let brokered = ledger
        .record(&row("nearai", BROKERED_ID, ProofStatus::Pending))
        .expect("records");
    let flaky = ledger
        .record(&row("nearai", FLAKY_ID, ProofStatus::Pending))
        .expect("records");
    let settings = ProofSettings::default().with_max_attempts(3);
    let vouches = Vouches(vec![enclave_public()]);

    // A 404 is looked at once more, a round later, before it is believed.
    let mut first = run_once(&ledger, &registry, &vouches, &settings).await;
    first.deferred.sort_unstable();
    assert!(first.settled.is_empty());
    assert_eq!(first.deferred, vec![brokered, flaky]);

    let second = run_once(&ledger, &registry, &vouches, &settings).await;
    assert_eq!(second.settled, vec![(brokered, ProofStatus::Unavailable)]);
    assert_eq!(second.deferred, vec![flaky]);
    let third = run_once(&ledger, &registry, &vouches, &settings).await;
    assert_eq!(third.settled, vec![(flaky, ProofStatus::Unavailable)]);

    // Settled rows are not read again.
    let fourth = run_once(&ledger, &registry, &vouches, &settings).await;
    assert_eq!(fourth, ironwire_proxy::proof::Round::default());
    let flaky_lookups = seen
        .lock()
        .expect("lock")
        .lookups
        .iter()
        .filter(|(id, ..)| id == FLAKY_ID)
        .count();
    assert_eq!(flaky_lookups, 3, "exactly the retry budget, no more");
    let brokered_lookups = seen
        .lock()
        .expect("lock")
        .lookups
        .iter()
        .filter(|(id, ..)| id == BROKERED_ID)
        .count();
    assert_eq!(brokered_lookups, 2, "one grace look at a 404, no more");
}

/// A backend that signs nothing is never asked, whatever its rows say.
#[tokio::test]
async fn an_outside_row_is_never_checked() {
    let (base, seen) = fake_near().await;
    let ledger = Ledger::in_memory().expect("ledger");
    ledger
        .record(&row("claude-sub", HOSTED_ID, ProofStatus::Outside))
        .expect("records");
    let round = run_once(
        &ledger,
        &registry(&base),
        &Vouches(vec![enclave_public()]),
        &ProofSettings::default(),
    )
    .await;
    assert_eq!(round, ironwire_proxy::proof::Round::default());
    assert!(seen.lock().expect("lock").lookups.is_empty());
}

async fn control(
    ledger: Ledger,
    uri: &str,
    token: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let state = AppState::new(
        BackendRegistry::new(),
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    )
    .with_ledger(Some(ledger));
    let mut request = Request::builder().uri(uri);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = app(state)
        .oneshot(request.body(Body::empty()).expect("builds"))
        .await
        .expect("served");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

fn seeded() -> Ledger {
    let ledger = Ledger::in_memory().expect("ledger");
    let verified = ledger
        .record(&row("nearai", HOSTED_ID, ProofStatus::Pending))
        .expect("records");
    ledger
        .settle_proof(verified, ProofStatus::Verified)
        .expect("settles");
    ledger
        .record(&row("nearai", FLAKY_ID, ProofStatus::Pending))
        .expect("records");
    let mut outside = row("claude-sub", "msg_1", ProofStatus::Outside);
    outside.served_model = Some("claude-opus-4-6".into());
    outside.cost_usd = Some(0.5);
    ledger.record(&outside).expect("records");
    ledger
}

/// Additive only: the Trace Commons daemon already reads `/log`, and a row
/// gains one field.
#[tokio::test]
async fn log_rows_carry_their_proof_label() {
    let (status, view) = control(seeded(), "/_ironwire/log?limit=10", Some("test-token")).await;
    assert_eq!(status, StatusCode::OK);
    let labels: Vec<&str> = view["exchanges"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["proof"].as_str().expect("a label"))
        .collect();
    assert_eq!(labels, vec!["outside", "pending", "verified"]);
}

#[tokio::test]
async fn the_summary_splits_routed_from_outside_per_model() {
    let (status, view) = control(seeded(), "/_ironwire/summary", Some("test-token")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["enabled"], true);
    assert_eq!(view["receipts"], false, "off unless configured");
    assert_eq!(view["routed"]["calls"], 2);
    assert_eq!(view["routed"]["proof"]["verified"], 1);
    assert_eq!(view["routed"]["proof"]["pending"], 1);
    assert_eq!(view["outside"]["calls"], 1);
    assert_eq!(view["outside"]["proof"]["outside"], 1);
    assert!((view["outside"]["cost_usd"].as_f64().expect("cost") - 0.5).abs() < 1e-9);

    let groups = view["groups"].as_array().expect("groups");
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0]["backend"], "nearai");
    assert_eq!(groups[0]["model"], MODEL);
    assert_eq!(groups[0]["route"], "routed");
    assert_eq!(groups[0]["calls"], 2);
    assert!(
        groups[0]["work_kind"].is_null(),
        "nothing classifies work, so nothing is claimed"
    );
    assert_eq!(groups[1]["route"], "outside");
}

#[tokio::test]
async fn the_summary_window_excludes_older_calls() {
    let (_, view) = control(
        seeded(),
        "/_ironwire/summary?since=2999-01-01T00:00:00Z",
        Some("test-token"),
    )
    .await;
    assert_eq!(view["routed"]["calls"], 0);
    assert_eq!(view["groups"].as_array().expect("groups").len(), 0);
}

#[tokio::test]
async fn the_summary_needs_the_control_token() {
    let (status, _) = control(seeded(), "/_ironwire/summary", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = control(seeded(), "/_ironwire/summary", Some("wrong")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---- digests without bodies ---------------------------------------------

/// The request and response bodies of a real NEAR AI exchange, captured on
/// 2026-09-01 from `qwen3-6-27b.completions.near.ai`, with the `text` of the
/// receipt the enclave returned for them. The newer ed25519 captures carry a
/// receipt but not the bodies, so this is the one capture that pins what the
/// two hashes are *of*: the raw request bytes as sent and the raw response
/// bytes as received -- not `message.content`, which here is `null`.
const LIVE_REQUEST_B64: &str = "eyJtb2RlbCI6IlF3ZW4vUXdlbjMuNi0yN0ItRlA4IiwibWVzc2FnZXMiOlt7InJvbGUiOiJ1c2VyIiwiY29udGVudCI6ImhpIn1dLCJtYXhfdG9rZW5zIjoxNiwic3RyZWFtIjpmYWxzZX0=";
const LIVE_RESPONSE_B64: &str = "eyJjaG9pY2VzIjpbeyJmaW5pc2hfcmVhc29uIjoibGVuZ3RoIiwiaW5kZXgiOjAsImxvZ3Byb2JzIjpudWxsLCJtZXNzYWdlIjp7ImNvbnRlbnQiOm51bGwsInJlYXNvbmluZ19jb250ZW50IjoiSGVyZSdzIGEgdGhpbmtpbmcgcHJvY2VzczpcblxuMS4gICoqQW5hbHl6ZSBVc2VyIElucHV0OioqIiwicm9sZSI6ImFzc2lzdGFudCJ9fV0sImNyZWF0ZWQiOjE3ODgyOTI2MTcsImlkIjoiYzU0OTYxYWIxZDU5NGNmNTkxZTU1NjZjYWEyMTE5NmIiLCJtb2RlbCI6IlF3ZW4vUXdlbjMuNi0yN0ItRlA4Iiwib2JqZWN0IjoiY2hhdC5jb21wbGV0aW9uIiwidXNhZ2UiOnsiY29tcGxldGlvbl90b2tlbnMiOjE2LCJwcm9tcHRfdG9rZW5zIjoxMSwicHJvbXB0X3Rva2Vuc19kZXRhaWxzIjpudWxsLCJyZWFzb25pbmdfdG9rZW5zIjoxNiwidG90YWxfdG9rZW5zIjoyN319";
const LIVE_RECEIPT_TEXT: &str = "Qwen/Qwen3.6-27B-FP8:92a1a2fd9a1b5e9138e7a99f741f0b7ec9e457a7874b429df9b5c64b2f49eafd:52d2225b7a0004e880b701419b28a89e7df956a32918c76fb53a89e0c39de675";

/// Both capture modes, fed the live bytes in awkward chunks, produce exactly
/// the digests the enclave signed.
#[tokio::test]
async fn the_digests_are_the_ones_a_live_receipt_signs() {
    use base64::Engine as _;
    use futures_util::StreamExt as _;
    use ironwire_proxy::pipeline::{Capture, capture_stream};

    let engine = base64::engine::general_purpose::STANDARD;
    let request = engine.decode(LIVE_REQUEST_B64).expect("request");
    let response = engine.decode(LIVE_RESPONSE_B64).expect("response");
    let mut parts = LIVE_RECEIPT_TEXT.split(':').skip(1);
    let (signed_request, signed_response) = (
        parts.next().expect("request hash"),
        parts.next().expect("response hash"),
    );

    for capture in [
        Capture::digest_only(&request),
        Capture::of_request(bytes::Bytes::from(request.clone())),
    ] {
        let chunks: Vec<Result<bytes::Bytes, ironwire_upstream::backend::UpstreamError>> = response
            .chunks(7)
            .map(|c| Ok(bytes::Bytes::copy_from_slice(c)))
            .collect();
        let mut streamed = Box::pin(capture_stream(futures_util::stream::iter(chunks), &capture));
        let mut forwarded = Vec::new();
        while let Some(chunk) = streamed.next().await {
            forwarded.extend_from_slice(&chunk.expect("chunk"));
        }
        assert_eq!(forwarded, response, "the tee forwards every byte untouched");
        assert_eq!(capture.request_sha256(), signed_request);
        assert_eq!(capture.response_sha256().as_deref(), Some(signed_response));
    }
    // Digest-only keeps nothing.
    let capture = Capture::digest_only(&request);
    assert!(capture.request().is_none());
}

/// `capture.receipts` with bodies off: the row gets both digests, no body
/// reference, and still settles `verified`.
#[tokio::test]
async fn receipts_without_body_capture_still_reach_verified() {
    let (base, _seen) = fake_near().await;
    let ledger = Ledger::in_memory().expect("ledger");
    let registry = registry(&base);
    let mut config = Config::default();
    config.capture.receipts = true;
    let state = AppState::new(
        registry.clone(),
        config,
        ConsentLedger::default(),
        "test-token".to_string(),
    )
    .with_ledger(Some(ledger.clone()));
    assert!(state.bodies.is_none());

    let response = app(state).oneshot(chat_request()).await.expect("answers");
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;
    let recorded = first_row(&ledger).await;
    assert!(recorded.request_sha256.is_some() && recorded.response_sha256.is_some());
    assert_eq!(recorded.body_ref, None, "no body was kept");

    run_once(
        &ledger,
        &registry,
        &Vouches(vec![enclave_public()]),
        &ProofSettings::default(),
    )
    .await;
    assert_eq!(
        ledger.recent(1).expect("reads")[0].proof,
        Some(ProofStatus::Verified)
    );
}

/// A receipt that is not there yet on the first look is found on the second.
#[tokio::test]
async fn a_receipt_that_lags_the_answer_is_still_found() {
    let (base, seen) = fake_near().await;
    seen.lock().expect("lock").lag = 1;
    let ledger = Ledger::in_memory().expect("ledger");
    let registry = registry(&base);
    let mut config = Config::default();
    config.capture.receipts = true;
    let state = AppState::new(
        registry.clone(),
        config,
        ConsentLedger::default(),
        "test-token".to_string(),
    )
    .with_ledger(Some(ledger.clone()));
    let response = app(state).oneshot(chat_request()).await.expect("answers");
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;
    first_row(&ledger).await;

    let vouches = Vouches(vec![enclave_public()]);
    let first = run_once(&ledger, &registry, &vouches, &ProofSettings::default()).await;
    assert!(first.settled.is_empty(), "a first 404 is not believed");
    run_once(&ledger, &registry, &vouches, &ProofSettings::default()).await;
    assert_eq!(
        ledger.recent(1).expect("reads")[0].proof,
        Some(ProofStatus::Verified)
    );
}

/// The embedding path end to end: a host supplies the attestor and nothing
/// else, bodies stay off, and the row reaches `verified` while no body file is
/// ever written under the home.
#[tokio::test]
async fn an_embedding_host_supplies_the_attestor_and_no_body_is_written() {
    use ironwire_proxy::embed::{EmbedOptions, StartupProbes, UpdateChecks, start_with_options};

    let (base, _seen) = fake_near().await;
    let home = tempfile::tempdir().expect("home");
    std::fs::write(
        home.path().join("config.toml"),
        format!(
            "[updates]\ncheck = false\n[[backends]]\nid = 'nearai'\nkind = 'nearai'\n\
             base_url = '{base}'\nmodels = ['{MODEL}']\n"
        ),
    )
    .expect("config");
    let options = EmbedOptions::default()
        .with_startup_probes(StartupProbes::Off)
        .with_update_checks(UpdateChecks::Off)
        .with_credentials(|name: &str| {
            (name == "NEARAI_API_KEY").then(|| SecretString::from("near-key".to_string()))
        })
        .with_signer_attestor(Arc::new(Vouches(vec![enclave_public()])))
        .with_proof_settings(
            ProofSettings::default().with_period(std::time::Duration::from_millis(50)),
        );
    let proxy = start_with_options(home.path(), Some(0), options, |_, _| {})
        .await
        .expect("starts");
    let port = proxy.port();
    let token = std::fs::read_to_string(home.path().join("control.token")).expect("token");
    let client = reqwest::Client::new();

    let answer = client
        .post(format!(
            "http://127.0.0.1:{port}/openai/v1/chat/completions"
        ))
        .header("content-type", "application/json")
        .body(format!(
            r#"{{"model":"{MODEL}","messages":[{{"role":"user","content":"hi"}}]}}"#
        ))
        .send()
        .await
        .expect("answers");
    assert_eq!(answer.status(), 200);
    assert_eq!(answer.text().await.expect("body"), response_body());

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let label = loop {
        let view: serde_json::Value = client
            .get(format!("http://127.0.0.1:{port}/_ironwire/log?limit=1"))
            .bearer_auth(token.trim())
            .send()
            .await
            .expect("log")
            .json()
            .await
            .expect("json");
        let label = view["exchanges"][0]["proof"].as_str().map(str::to_string);
        if label.as_deref() == Some("verified") || tokio::time::Instant::now() > deadline {
            break label;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    proxy.shutdown().await;
    assert_eq!(label.as_deref(), Some("verified"));

    let bodies = home.path().join("bodies");
    let written = std::fs::read_dir(&bodies)
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(written, 0, "bodies are off, so nothing is written");
}
