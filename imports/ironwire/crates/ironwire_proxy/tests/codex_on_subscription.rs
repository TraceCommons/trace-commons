//! Conformance: Codex → IronWire → ChatGPT subscription.
//!
//! The counterpart to `passthrough.rs` for the second native lane. Same claim,
//! different wire: a Codex request reaches `chatgpt.com` byte-identical, carries
//! the account header the subscription requires, and never carries the client's
//! own credential.
//!
//! It also pins the rule that makes the subscription lane defensible at all —
//! `docs/TRUST.md` §3, a subscription backend serves only requests that already
//! arrive as its own product.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::Config;
use ironwire_core::protocol::BackendId;
use ironwire_creds::ConsentLedger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::openai_responses::ResponsesBackend;
use ironwire_upstream::{Backend, Observation};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// What the mock upstream saw.
#[derive(Debug, Default, Clone)]
struct Received {
    request_line: String,
    headers: Vec<(String, String)>,
    body: String,
}

/// A Responses-API stream, in the framing ChatGPT actually sends.
const UPSTREAM_SSE: &str = concat!(
    "event: response.created\n",
    r#"data: {"type":"response.created","response":{"id":"resp_1","model":"gpt-5.6"}}"#,
    "\n\n",
    "event: response.output_text.delta\n",
    r#"data: {"type":"response.output_text.delta","delta":"hi"}"#,
    "\n\n",
    "event: response.completed\n",
    r#"data: {"type":"response.completed","response":{"id":"resp_1","usage":{"input_tokens":11,"output_tokens":3}}}"#,
    "\n\n",
);

/// A ChatGPT access token carrying the account claim IronWire must echo back.
///
/// Structurally a JWT, cryptographically meaningless — nothing in IronWire
/// verifies the signature, and nothing should: the issuer does that.
const ACCOUNT_ID: &str = "36afe797-0000-4444-8888-aaaaaaaaaaaa";

fn access_token() -> String {
    access_token_for(ACCOUNT_ID)
}

fn access_token_for(account_id: &str) -> String {
    use base64::Engine as _;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload = serde_json::json!({
        "https://api.openai.com/auth": {"chatgpt_account_id": account_id},
        "exp": 4_000_000_000u64,
    });
    format!(
        "{}.{}.{}",
        engine.encode(br#"{"alg":"RS256","typ":"JWT"}"#),
        engine.encode(payload.to_string()),
        engine.encode(b"inert"),
    )
}

/// A fixture `auth.json` in the shape Codex writes.
fn codex_auth_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    codex_auth_fixture_for(ACCOUNT_ID)
}

/// A fixture Codex home logged in to `account_id`.
fn codex_auth_fixture_for(account_id: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("auth.json");
    let contents = serde_json::json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": access_token_for(account_id),
            "access_token": access_token_for(account_id),
            "refresh_token": "rt.1.EXAMPLE",
            "account_id": account_id,
        },
        "last_refresh": "2026-07-30T04:20:00Z",
    });
    std::fs::write(&path, contents.to_string()).expect("write fixture");
    (dir, path)
}

/// Minimal HTTP/1.1 upstream that records one request and replies with SSE.
async fn spawn_mock() -> (String, Arc<Mutex<Option<Received>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let received = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&received);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let Ok(n) = socket.read(&mut chunk).await else {
                    return;
                };
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(split) = find_head_end(&buf) {
                    let head = String::from_utf8_lossy(&buf[..split]).to_string();
                    let length = content_length(&head).unwrap_or(0);
                    if buf.len() - split >= length {
                        let mut lines = head.lines();
                        let request_line = lines.next().unwrap_or_default().to_string();
                        let headers = lines
                            .filter_map(|line| line.split_once(": "))
                            .map(|(k, v)| (k.to_ascii_lowercase(), v.to_string()))
                            .collect();
                        *sink.lock().expect("lock") = Some(Received {
                            request_line,
                            headers,
                            body: String::from_utf8_lossy(&buf[split..split + length]).to_string(),
                        });
                        break;
                    }
                }
            }

            let head = format!(
                "HTTP/1.1 200 OK\r\n\
             content-type: text/event-stream\r\n\
             connection: close\r\n\
             x-codex-primary-used-percent: 41\r\n\
             x-codex-primary-reset-after-seconds: 1800\r\n\
             content-length: {}\r\n\r\n",
                UPSTREAM_SSE.len()
            );
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(UPSTREAM_SSE.as_bytes()).await;
            let _ = socket.flush().await;
        }
    });

    (format!("http://{addr}"), received)
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

fn content_length(head: &str) -> Option<usize> {
    head.lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.trim().parse().ok())
}

/// State with the ChatGPT subscription backend pointed at the mock, reading a
/// fixture credential rather than whatever login is on this machine.
fn state_for(base_url: &str, auth_path: &std::path::Path) -> AppState {
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        ResponsesBackend::codex_subscription_at(
            Some(auth_path.to_path_buf()),
            Some(base_url.to_string()),
            30,
        )
        .expect("client builds"),
    ));
    // A subscription backend is off until consent is recorded (TRUST.md §2);
    // these tests are about what happens *after* the user said yes.
    let mut consent = ConsentLedger::default();
    consent.grant("codex-sub", chrono::Utc::now());
    AppState::new(
        registry,
        Config::default(),
        consent,
        "test-token".to_string(),
    )
}

/// A Codex-shaped Responses request: its own instructions block, a tool, and
/// encrypted reasoning state carried forward from an earlier turn.
const CODEX_BODY: &str = concat!(
    r#"{"model":"gpt-5.6","stream":true,"#,
    r#""instructions":"You are Codex, based on GPT-5.","#,
    r#""tools":[{"type":"function","name":"shell","parameters":{"type":"object"}}],"#,
    r#""input":[{"type":"reasoning","id":"rs_1","encrypted_content":"gAAAAA"},"#,
    r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}],"#,
    r#""a_field_from_the_future":{"nested":[1,2,3]},"#,
    r#""reasoning":{"effort":"high","summary":"auto"}}"#,
);

/// The same request without Codex's identity — a third-party client pointed at
/// the OpenAI façade.
const THIRD_PARTY_BODY: &str = concat!(
    r#"{"model":"gpt-5.6","stream":true,"#,
    r#""instructions":"You are a helpful assistant.","#,
    r#""input":[{"type":"message","role":"user","content":"hi"}]}"#,
);

fn codex_request(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("originator", "codex_cli_rs")
        .header("session_id", "01998f6e-0000-7000-8000-000000000000")
        .header("openai-beta", "responses=experimental")
        .body(Body::from(body.to_string()))
        .expect("request builds")
}

async fn prime_native_session(state: &AppState) {
    let mut first: serde_json::Value = serde_json::from_str(CODEX_BODY).expect("fixture JSON");
    first["input"].as_array_mut().expect("input").remove(0);
    let response = app(state.clone())
        .oneshot(codex_request("/openai/v1/responses", &first.to_string()))
        .await
        .expect("first turn");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "plain native first turn establishes its account scope"
    );
    axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("first stream");
}

/// The same request as a client that is not Codex would send it: no
/// `originator`, and a body with none of Codex's own instructions.
///
/// Both halves matter. Codex 0.145 dropped the `instructions` field entirely,
/// so the body is no longer a reliable signal on its own and the `originator`
/// header carries the identity — which means a fixture that sends Codex's
/// header while claiming to be a third party is testing a spoofer, not a third
/// party. Neither signal survives a client that deliberately copies it; the
/// invariant is that IronWire never *synthesizes* one (`docs/TRUST.md` §3).
fn third_party_request(uri: &str, body: &'static str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("request builds")
}

#[tokio::test]
async fn a_codex_request_reaches_chatgpt_byte_identical() {
    let (base_url, received) = spawn_mock().await;
    let (_dir, auth) = codex_auth_fixture();

    let state = state_for(&base_url, &auth);
    prime_native_session(&state).await;
    let response = app(state)
        .oneshot(codex_request("/openai/v1/responses", CODEX_BODY))
        .await
        .expect("served");
    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;

    let got = received.lock().expect("lock").clone().expect("mock saw it");
    assert_eq!(
        got.body, CODEX_BODY,
        "the native lane must not re-encode the body — encrypted reasoning \
         state in particular does not survive a round trip"
    );
    // The ChatGPT base URL is `…/backend-api/codex`, which already carries its
    // own root — so the endpoint is `<base>/responses`, and re-appending the
    // client's `/v1` would 404 against the real provider. The mock is mounted
    // at the server root, so here that is a bare `/responses`.
    assert!(
        got.request_line.starts_with("POST /responses "),
        "unexpected request line: {}",
        got.request_line
    );
}

#[tokio::test]
async fn the_account_header_is_restored_from_our_own_credential() {
    // Codex sends `chatgpt-account-id` on its built-in provider path but not
    // through a custom one, and the subscription refuses a request without it.
    // The value comes out of the token we are presenting, so it cannot name any
    // account but the one that already owns the credential (TRUST.md §3).
    let (base_url, received) = spawn_mock().await;
    let (_dir, auth) = codex_auth_fixture();

    let state = state_for(&base_url, &auth);
    prime_native_session(&state).await;
    let response = app(state)
        .oneshot(codex_request("/openai/v1/responses", CODEX_BODY))
        .await
        .expect("served");
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;

    let got = received.lock().expect("lock").clone().expect("mock saw it");
    let header = |name: &str| {
        got.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(header("chatgpt-account-id"), Some(ACCOUNT_ID));

    // Codex's own identifying headers ride along untouched — we read them, we
    // do not write them.
    assert_eq!(header("originator"), Some("codex_cli_rs"));
    assert_eq!(
        header("session_id"),
        Some("01998f6e-0000-7000-8000-000000000000")
    );
    assert_eq!(header("openai-beta"), Some("responses=experimental"));

    // And the credential is ours, not the client's.
    assert_eq!(
        header("authorization"),
        Some(format!("Bearer {}", access_token()).as_str())
    );
}

/// Every value the mock saw for one header, in order. A header sent twice
/// shows up twice, which is the point.
fn all_values<'a>(got: &'a Received, name: &str) -> Vec<&'a str> {
    got.headers
        .iter()
        .filter(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
        .collect()
}

#[tokio::test]
async fn an_account_id_the_client_sends_never_reaches_chatgpt() {
    // `docs/PROTOCOL.md` §2 item 2. The account header names the workspace the
    // bearer belongs to, so it is the backend's to set, from the token it is
    // presenting — a client's copy is dropped, not preferred and not doubled.
    let (base_url, received) = spawn_mock().await;
    let (_dir, auth) = codex_auth_fixture();

    let mut request = codex_request("/openai/v1/responses", CODEX_BODY);
    let headers = request.headers_mut();
    headers.insert(
        "chatgpt-account-id",
        "someone-elses-account".parse().expect("header value"),
    );
    headers.insert("x-openai-fedramp", "true".parse().expect("header value"));

    let state = state_for(&base_url, &auth);
    prime_native_session(&state).await;
    let response = app(state).oneshot(request).await.expect("served");
    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;

    let got = received.lock().expect("lock").clone().expect("mock saw it");
    assert_eq!(
        all_values(&got, "chatgpt-account-id"),
        vec![ACCOUNT_ID],
        "exactly one account id, and it is the presented token's own"
    );
    assert!(
        all_values(&got, "x-openai-fedramp").is_empty(),
        "an account-bound header the client sent must not reach the provider"
    );
    assert_eq!(got.body, CODEX_BODY, "only headers changed, never the body");
}

#[tokio::test]
async fn a_request_served_by_another_login_carries_that_logins_bearer_and_account_id() {
    // The drift this rule exists for: the client's own login is spent, the user
    // turned on cross-account fallback, and the request goes out on a second
    // ChatGPT login. Its bearer and its account id must travel together — the
    // id the client named (its own login's) must not ride along with the other
    // login's token.
    const OWN_ACCOUNT: &str = "own-account-0000";
    const WORK_ACCOUNT: &str = "work-account-1111";
    let (own_url, own_received) = spawn_mock().await;
    let (work_url, work_received) = spawn_mock().await;
    let (_own_dir, own_auth) = codex_auth_fixture_for(OWN_ACCOUNT);
    let (work_home, _work_auth) = codex_auth_fixture_for(WORK_ACCOUNT);

    let own = ResponsesBackend::codex_subscription_at(Some(own_auth), Some(own_url), 30)
        .expect("client builds");
    // Spent, so the only way to serve the request is the other login.
    own.record(&Observation {
        retry_after_secs: Some(3600),
        ..Observation::default()
    });
    let work = ResponsesBackend::codex_subscription_for(
        BackendId::from("codex-work"),
        "ChatGPT subscription (work)".to_string(),
        Some(work_home.path().to_path_buf()),
        Some(work_url),
        30,
    )
    .expect("client builds");

    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(own));
    registry.push(Arc::new(work));
    let mut consent = ConsentLedger::default();
    consent.grant("codex-sub", chrono::Utc::now());
    // Consent answers a login by its directory, not by its id.
    let work_home_dir = work_home.path().display().to_string();
    consent.grant_for("codex-work", Some(&work_home_dir), chrono::Utc::now());
    let mut config = Config::default();
    config.subscriptions.cross_account_fallback = true;
    let state = AppState::new(registry, config, consent, "test-token".to_string());
    state
        .policy
        .lock()
        .expect("policy lock")
        .set_cross_account_fallback(true);

    // No encrypted reasoning in this body: a conversation carrying some may
    // never change account at all (`docs/PROTOCOL.md` §6).
    let mut request = codex_request("/openai/v1/responses", THIRD_PARTY_BODY);
    request.headers_mut().insert(
        "chatgpt-account-id",
        OWN_ACCOUNT.parse().expect("header value"),
    );
    let response = app(state).oneshot(request).await.expect("served");
    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;

    assert!(
        own_received.lock().expect("lock").is_none(),
        "the spent login must not have been dialled"
    );
    let got = work_received
        .lock()
        .expect("lock")
        .clone()
        .expect("the other login served it");
    assert_eq!(all_values(&got, "chatgpt-account-id"), vec![WORK_ACCOUNT]);
    assert_eq!(
        all_values(&got, "authorization"),
        vec![format!("Bearer {}", access_token_for(WORK_ACCOUNT)).as_str()]
    );
    assert_eq!(got.body, THIRD_PARTY_BODY);
}

#[tokio::test]
async fn a_client_that_is_not_codex_is_refused_the_subscription() {
    // TRUST.md §3, the invariant the whole subscription lane rests on. With the
    // subscription as the only backend, a third-party request must fail rather
    // than be served — being served would mean IronWire had forged an identity.
    let (base_url, received) = spawn_mock().await;
    let (_dir, auth) = codex_auth_fixture();

    let response = app(state_for(&base_url, &auth))
        .oneshot(third_party_request(
            "/openai/v1/responses",
            THIRD_PARTY_BODY,
        ))
        .await
        .expect("served");
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "a non-Codex client must not reach the ChatGPT subscription"
    );
    // The status alone would also be produced by a backend that failed for some
    // unrelated reason. What matters is that nothing was sent at all.
    assert!(
        received.lock().expect("lock").is_none(),
        "the request reached chatgpt.com despite carrying no Codex identity"
    );
}

/// The regression that made the subscription unreachable from Codex itself.
///
/// Codex 0.145 sends no `instructions` field — its system prompt moved into
/// `input` — so an identity check that reads only the body stops recognising
/// the client the subscription belongs to, and the request quietly falls off
/// onto some other backend. The `originator` header is what still names it.
#[tokio::test]
async fn codex_is_recognised_from_its_header_when_the_body_carries_no_marker() {
    let (base_url, received) = spawn_mock().await;
    let (_dir, auth) = codex_auth_fixture();

    let response = app(state_for(&base_url, &auth))
        .oneshot(codex_request("/openai/v1/responses", THIRD_PARTY_BODY))
        .await
        .expect("served");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Codex must reach its own subscription without an `instructions` field"
    );
    assert!(
        received.lock().expect("lock").is_some(),
        "nothing was dialled"
    );
}

#[tokio::test]
async fn the_response_reaches_the_client_byte_identical() {
    let (base_url, _received) = spawn_mock().await;
    let (_dir, auth) = codex_auth_fixture();

    let state = state_for(&base_url, &auth);
    prime_native_session(&state).await;
    let response = app(state)
        .oneshot(codex_request("/openai/v1/responses", CODEX_BODY))
        .await
        .expect("served");
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    assert_eq!(
        String::from_utf8_lossy(&body),
        UPSTREAM_SSE,
        "the client must see exactly what the provider sent"
    );
}

#[tokio::test]
async fn chat_completions_is_served_on_the_same_facade() {
    // Aider and friends speak this wire, not Responses. It routes to whatever
    // backend can serve it — here, nothing can, and the answer must be an
    // OpenAI-shaped error rather than a 404 that reads as "wrong URL".
    let (base_url, _received) = spawn_mock().await;
    let (_dir, auth) = codex_auth_fixture();

    let response = app(state_for(&base_url, &auth))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/openai/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"model":"gpt-5.6","messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .expect("request builds"),
        )
        .await
        .expect("served");
    assert_ne!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_model_list_offers_what_the_subscription_can_serve() {
    let (base_url, _received) = spawn_mock().await;
    let (_dir, auth) = codex_auth_fixture();

    let response = app(state_for(&base_url, &auth))
        .oneshot(
            Request::builder()
                .uri("/openai/v1/models")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("served");
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let ids: Vec<&str> = value["data"]
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    assert!(ids.contains(&"gpt-5.6"), "got {ids:?}");
    assert_eq!(value["object"], "list", "OpenAI clients key off this");
}

#[tokio::test]
async fn encrypted_reasoning_with_no_observed_native_origin_never_reaches_a_subscription() {
    let (base_url, received) = spawn_mock().await;
    let (_dir, auth) = codex_auth_fixture();
    let response = app(state_for(&base_url, &auth))
        .oneshot(codex_request("/openai/v1/responses", CODEX_BODY))
        .await
        .expect("served refusal");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(received.lock().expect("lock").is_none());
}
