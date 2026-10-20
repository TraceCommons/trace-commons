//! Conformance: the native lane forwards bytes, not meanings.
//!
//! `docs/PROTOCOL.md` §7.2 — replay a request through the proxy against a
//! recording mock and assert the bytes the upstream received differ from the
//! original *only* in the mutations §2 enumerates, and that the bytes the
//! client received are byte-identical to what the upstream sent.
//!
//! This is the test that makes the fidelity claim real. Everything else in the
//! design rests on it.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::Config;
use ironwire_creds::ConsentLedger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// What the mock upstream saw.
#[derive(Debug, Default, Clone)]
struct Received {
    request_line: String,
    headers: Vec<(String, String)>,
    body: String,
}

/// The exact SSE bytes the mock sends back, including framing.
const UPSTREAM_SSE: &str = concat!(
    "event: message_start\n",
    r#"data: {"type":"message_start","message":{"id":"msg_1","model":"claude-opus-4-6","usage":{"input_tokens":11,"cache_read_input_tokens":98000,"output_tokens":1}}}"#,
    "\n\n",
    "event: content_block_delta\n",
    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
    "\n\n",
    "event: message_delta\n",
    r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":137}}"#,
    "\n\n",
    "event: message_stop\n",
    r#"data: {"type":"message_stop"}"#,
    "\n\n",
);

/// Minimal HTTP/1.1 upstream that records one request and replies with SSE.
///
/// Hand-rolled rather than mounted on a framework so the response bytes are
/// exactly what this test wrote — a framework could re-frame them, which would
/// make a byte-identity assertion meaningless.
async fn spawn_mock() -> (String, Arc<Mutex<Option<Received>>>) {
    spawn_mock_response(UPSTREAM_SSE).await
}

async fn spawn_mock_response(sse: &'static str) -> (String, Arc<Mutex<Option<Received>>>) {
    spawn_mock_content(sse, "text/event-stream").await
}

async fn spawn_mock_content(
    sse: &'static str,
    content_type: &'static str,
) -> (String, Arc<Mutex<Option<Received>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let received = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&received);

    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        // Read until the body is complete, using content-length from the head.
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
             content-type: {content_type}\r\n\
             anthropic-ratelimit-unified-limit: 1000\r\n\
             anthropic-ratelimit-unified-remaining: 180\r\n\
             content-length: {}\r\n\r\n",
            sse.len()
        );
        let _ = socket.write_all(head.as_bytes()).await;
        let _ = socket.write_all(sse.as_bytes()).await;
        let _ = socket.flush().await;
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

/// State with a single API-key Anthropic backend pointed at the mock.
///
/// The API-key backend rather than the subscription one, so this test does not
/// depend on a Claude Code login existing on the machine running it.
fn state_for(base_url: &str) -> AppState {
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        AnthropicBackend::api_key(
            SecretString::from("sk-ant-test-key"),
            Some(base_url.to_string()),
            30,
        )
        .expect("client builds"),
    ));
    AppState::new(
        registry,
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    )
}

/// A request body carrying every shape the native lane must not disturb:
/// cache breakpoints, a signed thinking block, tools, and a field we have
/// never heard of.
const CLIENT_BODY: &str = concat!(
    r#"{"model":"claude-opus-4-6","stream":true,"#,
    r#""system":[{"type":"text","text":"You are Claude Code","cache_control":{"type":"ephemeral"}}],"#,
    r#""tools":[{"name":"Read","input_schema":{"type":"object"}}],"#,
    r#""messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"...","signature":"sig-abc"}]},"#,
    r#"{"role":"user","content":"hi"}],"#,
    r#""a_field_from_the_future":{"nested":[1,2,3]},"#,
    r#""thinking":{"type":"enabled","budget_tokens":1024}}"#,
);

#[tokio::test]
async fn the_request_body_reaches_the_provider_byte_identical() {
    let (base_url, received) = spawn_mock().await;
    let response = app(state_for(&base_url))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/anthropic/v1/messages")
                .header("content-type", "application/json")
                .header("anthropic-version", "2023-06-01")
                .body(Body::from(CLIENT_BODY))
                .expect("request builds"),
        )
        .await
        .expect("served");
    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;

    let got = received.lock().expect("lock").clone().expect("mock saw it");
    assert_eq!(
        got.body, CLIENT_BODY,
        "the native lane must not re-encode the body"
    );
    assert!(got.request_line.starts_with("POST /v1/messages "));
}

#[tokio::test]
async fn only_the_enumerated_headers_are_mutated() {
    let (base_url, received) = spawn_mock().await;
    let response = app(state_for(&base_url))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/anthropic/v1/messages")
                .header("content-type", "application/json")
                .header("anthropic-version", "2023-06-01")
                .header("anthropic-beta", "interleaved-thinking-2025-05-14")
                .header("x-stainless-lang", "js")
                .header("x-api-key", "CLIENT-KEY-MUST-NOT-LEAK")
                .header("authorization", "Bearer CLIENT-TOKEN-MUST-NOT-LEAK")
                .header("x-ironwire-session-id", "aider-1")
                .body(Body::from(CLIENT_BODY))
                .expect("request builds"),
        )
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

    // Preserved: provider headers, including ones we do not model.
    assert_eq!(header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(header("x-stainless-lang"), Some("js"));
    assert_eq!(
        header("anthropic-beta"),
        Some("interleaved-thinking-2025-05-14"),
        "the client's own beta flags must survive"
    );
    assert_eq!(header("content-type"), Some("application/json"));

    // Replaced: the credential is ours, never the client's.
    assert_eq!(header("x-api-key"), Some("sk-ant-test-key"));
    assert!(
        !got.headers.iter().any(|(_, v)| v.contains("MUST-NOT-LEAK")),
        "a credential the client sent must never reach the provider"
    );

    // Stripped: addressed to IronWire, so the provider never sees it.
    assert_eq!(
        header("x-ironwire-session-id"),
        None,
        "a header addressed to IronWire must not announce the proxy upstream"
    );

    // Not added: §2.8's provider opt-in belongs to NEAR AI, and is off unless
    // an operator asks for it even there. Anthropic never asked for it, and the
    // enumeration is exhaustive for every other backend.
    assert_eq!(
        header("x-no-aliasing"),
        None,
        "a provider-specific header must not leak onto another provider"
    );
}

#[tokio::test]
async fn an_account_header_the_client_sends_never_reaches_any_provider() {
    // §2 item 2: `chatgpt-account-id` and `x-openai-fedramp` name the account a
    // bearer belongs to, so they are stripped with the client's credential on
    // every lane — not only on the one backend that sets its own. Forwarding a
    // client's copy to any other provider would be handing it an identifier
    // for an account it holds no credential for.
    let (anthropic_url, anthropic_received) = spawn_mock().await;
    let (chat_url, chat_received) = spawn_mock_response(ADMISSION_SSE).await;
    let lanes = [
        (
            state_for(&anthropic_url),
            "/anthropic/v1/messages",
            CLIENT_BODY,
            anthropic_received,
        ),
        (
            admission_state(&chat_url),
            "/openai/v1/chat/completions",
            ADMISSION_REQUEST,
            chat_received,
        ),
    ];

    for (state, uri, body, received) in lanes {
        let response = app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .header("chatgpt-account-id", "CLIENT-ACCOUNT-MUST-NOT-LEAK")
                    .header("x-openai-fedramp", "true")
                    .body(Body::from(body))
                    .expect("request builds"),
            )
            .await
            .expect("served");
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;

        let got = received.lock().expect("lock").clone().expect("mock saw it");
        for name in ["chatgpt-account-id", "x-openai-fedramp"] {
            assert!(
                !got.headers.iter().any(|(k, _)| k == name),
                "{name} reached the provider behind {uri}"
            );
        }
        assert_eq!(got.body, body, "{uri}: only headers changed");
    }
}

#[tokio::test]
async fn the_response_stream_reaches_the_client_byte_identical() {
    let (base_url, _received) = spawn_mock().await;
    let response = app(state_for(&base_url))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/anthropic/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(CLIENT_BODY))
                .expect("request builds"),
        )
        .await
        .expect("served");

    assert_eq!(response.status(), StatusCode::OK);
    // Rate-limit headers must survive: observation depends on them.
    assert_eq!(
        response
            .headers()
            .get("anthropic-ratelimit-unified-remaining")
            .and_then(|v| v.to_str().ok()),
        Some("180")
    );
    // Streaming responses must not be buffered anywhere in our path.
    assert_eq!(
        response
            .headers()
            .get("x-accel-buffering")
            .and_then(|v| v.to_str().ok()),
        Some("no")
    );

    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    assert_eq!(
        String::from_utf8_lossy(&body),
        UPSTREAM_SSE,
        "SSE must be forwarded frame-for-frame"
    );
}

#[tokio::test]
async fn usage_is_observed_from_the_stream_without_altering_it() {
    let (base_url, _received) = spawn_mock().await;
    let state = state_for(&base_url);
    let backend = state
        .backends
        .all()
        .first()
        .cloned()
        .expect("one backend registered");

    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/anthropic/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(CLIENT_BODY))
                .expect("request builds"),
        )
        .await
        .expect("served");
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;

    // The mock reported 180 of 1000 remaining, i.e. 82% used. We must report
    // exactly that, from the provider's own headers.
    match backend.quota().primary {
        ironwire_core::quota::Headroom::Observed { used_pct, .. } => {
            assert!(
                (used_pct - 82.0).abs() < 0.01,
                "expected 82% used, got {used_pct}"
            );
        }
        other => panic!("expected an observed headroom, got {other:?}"),
    }
}

const ADMISSION_SSE: &str = concat!(
    "data: {\"id\":\"chatcmpl-fixture\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"synthetic\"},\"finish_reason\":null}]}\n\n",
    "data: {\"id\":\"chatcmpl-fixture\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":1,\"total_tokens\":3}}\n\n",
    "data: [DONE]\n\n"
);

fn admission_state(base: &str) -> AppState {
    admission_state_with(&[("nearai", base)], Config::default())
}
fn admission_state_with(backends: &[(&str, &str)], config: Config) -> AppState {
    use ironwire_core::protocol::{BackendId, BackendKind};
    use ironwire_upstream::openai_chat::ChatCompletionsBackend;
    let mut registry = BackendRegistry::new();
    for (id, base) in backends {
        registry.push(Arc::new(
            ChatCompletionsBackend::new(
                BackendId::from(*id),
                "local fixture",
                BackendKind::Credits,
                Some(SecretString::from("test-only")),
                (*base).to_owned(),
                Vec::new(),
                5,
            )
            .unwrap(),
        ));
    }
    AppState::new(
        registry,
        config,
        ConsentLedger::default(),
        "test-token".into(),
    )
}

fn admission_value() -> String {
    format!(
        "tcad1:{}:{}:{}",
        "a".repeat(64),
        "b".repeat(64),
        chrono::Utc::now().timestamp() + 300
    )
}

async fn set_admission(
    state: &AppState,
    token: &str,
    confirmed: bool,
    binding: &str,
) -> StatusCode {
    let body = serde_json::json!({"session_id":"selected-session", "backend":"nearai", "binding":binding, "confirmed":confirmed});
    let request = Request::builder()
        .method("POST")
        .uri("/_ironwire/admission-binding")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    app(state.clone()).oneshot(request).await.unwrap().status()
}

const ADMISSION_REQUEST: &str = r#" {"model" : "fixture", "stream":true, "messages": [{"role":"user", "content":"synthetic task"}], "unknown":1.500} "#;

#[tokio::test]
async fn explicit_admission_is_the_only_added_metadata_and_capture_remains_off() {
    let (base, received) = spawn_mock_response(ADMISSION_SSE).await;
    let state = admission_state(&base);
    let binding = admission_value();
    assert_eq!(
        set_admission(&state, "wrong", true, &binding).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        set_admission(&state, "test-token", false, &binding).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        set_admission(&state, "test-token", true, &binding).await,
        StatusCode::OK
    );
    let capability = app(state.clone())
        .oneshot(
            Request::builder()
                .uri("/_ironwire/admission-binding")
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(capability.status(), StatusCode::OK);
    let capability = axum::body::to_bytes(capability.into_body(), 4096)
        .await
        .unwrap();
    let capability = String::from_utf8(capability.to_vec()).unwrap();
    assert!(capability.contains("max_lifetime_seconds"));
    assert!(capability.contains("\"body_capture_ready\":false"));
    assert!(!capability.contains(&binding));
    assert!(!capability.contains("selected-session"));
    assert!(state.bodies.is_none());
    assert!(!state.config.capture.bodies);
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/openai/v1/chat/completions")
                .header("content-type", "application/json")
                .header("x-ironwire-session-id", "selected-session")
                .body(Body::from(ADMISSION_REQUEST))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let returned = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(returned, ADMISSION_SSE);
    let recorded = received.lock().unwrap().clone().unwrap();
    let expected =
        ironwire_core::admission::insert_binding(ADMISSION_REQUEST.as_bytes(), &binding).unwrap();
    assert_eq!(recorded.body.as_bytes(), expected);
    let addition = format!(r#","metadata":{{"trace_commons_admission":"{binding}"}}"#);
    assert_eq!(recorded.body.replace(&addition, ""), ADMISSION_REQUEST);
    assert!(
        !recorded
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("x-ironwire-session-id"))
    );
}

#[tokio::test]
async fn other_sessions_and_revoked_bindings_preserve_the_native_bytes() {
    for revoke in [false, true] {
        let (base, received) = spawn_mock_response(ADMISSION_SSE).await;
        let state = admission_state(&base);
        assert_eq!(
            set_admission(&state, "test-token", true, &admission_value()).await,
            StatusCode::OK
        );
        if revoke {
            let response = app(state.clone())
                .oneshot(
                    Request::builder()
                        .method("DELETE")
                        .uri("/_ironwire/admission-binding")
                        .header("authorization", "Bearer test-token")
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"session_id":"selected-session"}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        let session = if revoke {
            "selected-session"
        } else {
            "other-session"
        };
        let response = app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/openai/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("session-id", session)
                    .body(Body::from(ADMISSION_REQUEST))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            received.lock().unwrap().as_ref().unwrap().body,
            ADMISSION_REQUEST
        );
    }
}

#[tokio::test]
async fn a_bound_session_cannot_silently_route_elsewhere_or_overwrite_client_metadata() {
    for wrong_backend in [false, true] {
        let (base, received) = spawn_mock_response(ADMISSION_SSE).await;
        let state = admission_state(&base);
        let binding = admission_value();
        state
            .admission_bindings
            .lock()
            .unwrap()
            .register(
                "selected-session",
                if wrong_backend {
                    "different-backend"
                } else {
                    "nearai"
                },
                &binding,
                true,
                chrono::Utc::now().timestamp(),
            )
            .unwrap();
        let body = if wrong_backend {
            ADMISSION_REQUEST
        } else {
            r#"{"model":"fixture","messages":[{"role":"user","content":"synthetic"}],"metadata":{"trace_commons_admission":"already-set"}}"#
        };
        let response = app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/openai/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("x-ironwire-session-id", "selected-session")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if wrong_backend {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::BAD_REQUEST
            }
        );
        assert!(received.lock().unwrap().is_none());
        let error = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let error = String::from_utf8(error.to_vec()).unwrap();
        assert!(!error.contains(&binding));
        assert!(!error.contains("selected-session"));
    }
}

#[tokio::test]
async fn separately_enabled_capture_hashes_exactly_the_bound_bytes_sent_upstream() {
    let (base, received) = spawn_mock_response(ADMISSION_SSE).await;
    let dir = tempfile::tempdir().unwrap();
    let bodies =
        Arc::new(ironwire_ledger::bodies::BodyStore::open(&dir.path().join("bodies")).unwrap());
    let ledger = ironwire_ledger::Ledger::in_memory().unwrap();
    let mut state = admission_state(&base)
        .with_ledger(Some(ledger.clone()))
        .with_bodies(Some(bodies.clone()));
    Arc::make_mut(&mut state.config).capture.bodies = true;
    let binding = admission_value();
    assert_eq!(
        set_admission(&state, "test-token", true, &binding).await,
        StatusCode::OK
    );
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/openai/v1/chat/completions")
                .header("content-type", "application/json")
                .header("session-id", "selected-session")
                .body(Body::from(ADMISSION_REQUEST))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let rows = ledger.recent(1).unwrap();
    assert_eq!(rows.len(), 1);
    let sent = received.lock().unwrap().clone().unwrap().body;
    let (captured, response) = bodies.read(rows[0].body_ref.as_deref().unwrap()).unwrap();
    assert_eq!(captured, sent.as_bytes());
    assert_eq!(
        captured,
        ironwire_core::admission::insert_binding(ADMISSION_REQUEST.as_bytes(), &binding).unwrap()
    );
    assert_eq!(response, ADMISSION_SSE.as_bytes());
    assert_eq!(
        rows[0].request_sha256.as_deref(),
        Some(ironwire_ledger::bodies::sha256_hex(&captured).as_str())
    );
}

#[tokio::test]
async fn admission_selects_its_eligible_backend_even_when_policy_prefers_another() {
    let (other, other_seen) = spawn_mock_response(ADMISSION_SSE).await;
    let (bound, bound_seen) = spawn_mock_response(ADMISSION_SSE).await;
    let state = admission_state_with(
        &[("preferred", &other), ("nearai", &bound)],
        Config::default(),
    );
    assert_eq!(
        set_admission(&state, "test-token", true, &admission_value()).await,
        StatusCode::OK
    );
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/openai/v1/chat/completions")
                .header("content-type", "application/json")
                .header("x-ironwire-session-id", "selected-session")
                .body(Body::from(ADMISSION_REQUEST))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(other_seen.lock().unwrap().is_none());
    assert!(
        bound_seen
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .body
            .contains("trace_commons_admission")
    );
}

#[tokio::test]
async fn privacy_and_translation_happen_before_binding_insertion() {
    for translated in [false, true] {
        let (base, received) = spawn_mock_response(ADMISSION_SSE).await;
        let mut config = Config::default();
        config.privacy.enabled = true;
        config.privacy.named_values = vec!["sentinel-value".into()];
        let state = admission_state_with(&[("nearai", &base)], config);
        let binding = admission_value();
        assert_eq!(
            set_admission(&state, "test-token", true, &binding).await,
            StatusCode::OK
        );
        let path = if translated {
            "/anthropic/v1/messages"
        } else {
            "/openai/v1/chat/completions"
        };
        let body = r#"{"model":"fixture","max_tokens":16,"stream":true,"messages":[{"role":"user","content":"sentinel-value"}]}"#;
        let response = app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/json")
                    .header("x-ironwire-session-id", "selected-session")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let seen = received.lock().unwrap().clone().unwrap();
        assert!(!seen.body.contains("sentinel-value"));
        let sent: serde_json::Value = serde_json::from_str(&seen.body).unwrap();
        assert_eq!(sent["metadata"]["trace_commons_admission"], binding);
        assert!(seen.request_line.contains("chat/completions"));
    }
}

#[tokio::test]
async fn poisoned_admission_state_remains_available_to_control_and_traffic() {
    for session in [None, Some("selected-session")] {
        let (base, received) = spawn_mock_response(ADMISSION_SSE).await;
        let state = admission_state(&base);
        let poisoned = state.admission_bindings.clone();
        let _ = std::panic::catch_unwind(move || {
            let _guard = poisoned.lock().unwrap();
            panic!("synthetic registry panic");
        });
        assert_eq!(
            set_admission(&state, "test-token", true, &admission_value()).await,
            StatusCode::OK
        );
        let mut request = Request::builder()
            .method("POST")
            .uri("/openai/v1/chat/completions")
            .header("content-type", "application/json");
        if let Some(session) = session {
            request = request.header("x-ironwire-session-id", session);
        }
        let response = app(state.clone())
            .oneshot(request.body(Body::from(ADMISSION_REQUEST)).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(received.lock().unwrap().is_some());
        for (token, status) in [
            ("wrong", StatusCode::UNAUTHORIZED),
            ("test-token", StatusCode::OK),
        ] {
            let response = app(state.clone())
                .oneshot(
                    Request::builder()
                        .uri("/_ironwire/admission-binding?session_id=selected-session")
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status);
            if status == StatusCode::OK {
                let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(status["status"], "active");
                assert!(status.get("binding").is_none());
            }
        }
    }
}

#[tokio::test]
async fn a_failed_bound_backend_never_fails_over_to_an_unbound_candidate() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bound = format!("http://{}", listener.local_addr().unwrap());
    let router = axum::Router::new().fallback(|| async { StatusCode::TOO_MANY_REQUESTS });
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let (other, other_seen) = spawn_mock_response(ADMISSION_SSE).await;
    let state = admission_state_with(&[("nearai", &bound), ("other", &other)], Config::default());
    assert_eq!(
        set_admission(&state, "test-token", true, &admission_value()).await,
        StatusCode::OK
    );
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/openai/v1/chat/completions")
                .header("content-type", "application/json")
                .header("x-ironwire-session-id", "selected-session")
                .body(Body::from(ADMISSION_REQUEST))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_server_error() || response.status().is_client_error());
    assert!(other_seen.lock().unwrap().is_none());
    task.abort();
}

#[tokio::test]
async fn authenticated_session_status_reports_expiry_and_explicit_revocation() {
    let (base, _received) = spawn_mock_response(ADMISSION_SSE).await;
    let state = admission_state(&base);
    let now = chrono::Utc::now().timestamp();
    let expired = format!("tcad1:{}:{}:{}", "a".repeat(64), "b".repeat(64), now - 1);
    state
        .admission_bindings
        .lock()
        .unwrap()
        .register("expired-session", "nearai", &expired, true, now - 2)
        .unwrap();
    // A fresh registration purges expired payloads while keeping their tombstones.
    assert_eq!(
        set_admission(&state, "test-token", true, &admission_value()).await,
        StatusCode::OK
    );
    let get = |id: &str| {
        Request::builder()
            .uri(format!("/_ironwire/admission-binding?session_id={id}"))
            .header("authorization", "Bearer test-token")
            .body(Body::empty())
            .unwrap()
    };
    let response = app(state.clone())
        .oneshot(get("expired-session"))
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(status["status"], "expired");
    assert_eq!(status["expires_at"], now - 1);
    assert!(
        !String::from_utf8(body.to_vec())
            .unwrap()
            .contains("expired-session")
    );
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/_ironwire/admission-binding")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"session_id":"expired-session"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app(state).oneshot(get("expired-session")).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["status"],
        "inactive"
    );
}

#[tokio::test]
async fn explicit_token_capture_preserves_wire_bytes_and_leases_exact_evidence() {
    use ironwire_core::config::{TokenCaptureConfig, TokenCaptureTarget};
    let (base, received) = spawn_mock_response(ADMISSION_SSE).await;
    let home = tempfile::tempdir().unwrap();
    let spool = Arc::new(
        ironwire_ledger::token_spool::TokenSpool::open(home.path(), 1024 * 1024, 86400).unwrap(),
    );
    let model: serde_json::Value = serde_json::from_str(ADMISSION_REQUEST).unwrap();
    let mut config = Config::default();
    config.capture.token_capture = Some(TokenCaptureConfig {
        targets: vec![TokenCaptureTarget {
            backend: "nearai".into(),
            model: model["model"].as_str().unwrap().into(),
        }],
        ..Default::default()
    });
    let state = admission_state_with(&[("nearai", &base)], config)
        .with_ledger(Some(ironwire_ledger::Ledger::in_memory().unwrap()))
        .with_token_spool(Some(spool.clone()));
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/openai/v1/chat/completions")
                .header("content-type", "application/json")
                .header("session-id", "capture-session")
                .body(Body::from(ADMISSION_REQUEST))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap(),
        ADMISSION_SSE
    );
    let sent = received.lock().unwrap().clone().unwrap().body;
    assert_eq!(
        sent.as_bytes(),
        ironwire_core::token_capture::augment_chat(ADMISSION_REQUEST.as_bytes(), 20).unwrap()
    );
    let now = chrono::Utc::now().timestamp();
    let captures = spool.list("capture-session", now).unwrap();
    assert_eq!(captures.len(), 1);
    let lease = spool
        .acquire(
            "capture-session",
            &[captures[0].capture_id.clone()],
            "commons",
            now,
            300,
        )
        .unwrap();
    let (request, response) = spool
        .read(&lease.lease_id, &lease.owner, &captures[0].capture_id, now)
        .unwrap();
    assert_eq!(request, sent.as_bytes());
    assert_eq!(response, ADMISSION_SSE.as_bytes());
    let unauthenticated = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_ironwire/token-captures")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"operation":"list","session":"capture-session"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn token_capture_uses_response_format_and_stops_augmenting_a_full_spool() {
    use ironwire_core::config::{TokenCaptureConfig, TokenCaptureTarget};
    const JSON: &str = r#"{"id":"json-fixture","choices":[{"index":0,"message":{"content":"blue"},"finish_reason":"stop","logprobs":{"content":[{"bytes":[98,108,117,101],"logprob":-1.0,"top_logprobs":[]}]}}]}"#;
    for full in [false, true] {
        let (base, received) = spawn_mock_content(JSON, "application/json").await;
        let home = tempfile::tempdir().unwrap();
        let spool = Arc::new(
            ironwire_ledger::token_spool::TokenSpool::open(home.path(), 1024, 86400).unwrap(),
        );
        let now = chrono::Utc::now().timestamp();
        if full {
            spool
                .record(
                    "other",
                    1,
                    "openai.chat",
                    false,
                    (&vec![0; 512], &vec![1; 512]),
                    now,
                )
                .unwrap();
        }
        assert_eq!(
            spool.can_accept_capture("capture-session", 100).unwrap(),
            !full
        );
        let model: serde_json::Value = serde_json::from_str(ADMISSION_REQUEST).unwrap();
        let mut config = Config::default();
        config.capture.token_capture = Some(TokenCaptureConfig {
            targets: vec![TokenCaptureTarget {
                backend: "nearai".into(),
                model: model["model"].as_str().unwrap().into(),
            }],
            ..Default::default()
        });
        let state = admission_state_with(&[("nearai", &base)], config)
            .with_ledger(Some(ironwire_ledger::Ledger::in_memory().unwrap()))
            .with_token_spool(Some(spool.clone()));
        let response = app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/openai/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("session-id", "capture-session")
                    .body(Body::from(ADMISSION_REQUEST))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 1 << 20)
                .await
                .unwrap(),
            JSON
        );
        let sent: serde_json::Value =
            serde_json::from_str(&received.lock().unwrap().as_ref().unwrap().body).unwrap();
        let captures = spool.list("capture-session", now).unwrap();
        if full {
            assert!(sent.get("logprobs").is_none());
            assert!(captures.is_empty());
        } else {
            assert_eq!(sent["logprobs"], true);
            assert_eq!(captures.len(), 1);
            assert!(!captures[0].streaming);
        }
    }
}
