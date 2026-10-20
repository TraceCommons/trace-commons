//! Conformance: a backend that is down stops being tried first.
//!
//! The unit tests in `ironwire_upstream::breaker` pin the state machine. This
//! pins the thing that actually matters to a user: that the state machine is
//! *wired into routing*, so the cost of a dead backend is paid once rather than
//! on every turn for the length of the outage.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::Config;
use ironwire_core::protocol::ModelTier;
use ironwire_creds::ConsentLedger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use ironwire_upstream::breaker::{BreakerBoard, CircuitBreakerConfig, CircuitState};
use ironwire_upstream::openai_chat::ChatCompletionsBackend;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// An upstream that accepts a connection and closes it without answering — the
/// shape of a provider that is down, rather than one that says so.
async fn spawn_dead_upstream() -> (String, Arc<Mutex<usize>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let connections = Arc::new(Mutex::new(0usize));
    let counter = Arc::clone(&connections);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            *counter.lock().expect("lock") += 1;
            let _ = socket.shutdown().await;
        }
    });

    (format!("http://{addr}"), connections)
}

/// A healthy Chat Completions upstream that answers anything, forever.
async fn spawn_healthy_upstream() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut chunk = [0u8; 8192];
                // One read is enough: we do not care what was asked.
                let _ = socket.read(&mut chunk).await;
                let sse = concat!(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n\n",
                );
                let head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n",
                    sse.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(sse.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });

    format!("http://{addr}")
}

const BODY: &str = concat!(
    r#"{"model":"claude-opus-4-6","stream":true,"#,
    r#""system":"You are Claude Code","#,
    r#""messages":[{"role":"user","content":"hi"}]}"#,
);

/// A dead Anthropic backend that routing prefers, and a healthy NEAR AI one
/// behind it. Registration order decides the tie, so the dead one is tried
/// first — which is exactly the situation the breaker is for.
fn state_for(dead: &str, healthy: &str, threshold: u32) -> AppState {
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        AnthropicBackend::api_key(SecretString::from("sk-ant-test"), Some(dead.to_string()), 5)
            .expect("client builds"),
    ));
    registry.push(Arc::new(
        ChatCompletionsBackend::nearai(
            Some(SecretString::from("near-key")),
            Some(healthy.to_string()),
            vec![("near-x".to_string(), ModelTier::Frontier)],
            5,
        )
        .expect("client builds"),
    ));

    let mut state = AppState::new(
        registry,
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    );
    // A lower threshold than the default, so the test spends a couple of round
    // trips against a dead socket instead of five.
    state.breakers = Arc::new(BreakerBoard::new(CircuitBreakerConfig {
        failure_threshold: threshold,
        recovery_timeout: std::time::Duration::from_secs(300),
        half_open_successes_needed: 1,
    }));
    state
}

async fn send(state: &AppState) -> axum::http::Response<Body> {
    app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/anthropic/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(BODY))
                .expect("request builds"),
        )
        .await
        .expect("served")
}

fn served_by(response: &axum::http::Response<Body>) -> Option<String> {
    response
        .headers()
        .get("x-ironwire-backend")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

#[tokio::test]
async fn a_dead_backend_stops_being_dialled_once_its_circuit_opens() {
    let (dead, dials) = spawn_dead_upstream().await;
    let healthy = spawn_healthy_upstream().await;
    let state = state_for(&dead, &healthy, 2);

    // First requests: the dead backend is preferred, fails, and the request
    // falls through to the healthy one. The user is served — at the cost of a
    // wasted round trip every time.
    let first = send(&state).await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(served_by(&first).as_deref(), Some("nearai"));
    let _ = axum::body::to_bytes(first.into_body(), 1 << 20).await;

    let second = send(&state).await;
    assert_eq!(second.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(second.into_body(), 1 << 20).await;

    let wasted_dials = *dials.lock().expect("lock");
    assert!(
        wasted_dials > 0,
        "the dead backend was never dialled; this test would prove nothing"
    );
    let health = state.breakers.statuses();
    let anthropic = health
        .iter()
        .find(|h| h.backend.as_str() == "anthropic-key")
        .expect("tracked");
    assert_eq!(
        anthropic.state,
        CircuitState::Open,
        "repeated transport failures must open the circuit"
    );

    // The point of the exercise: from here the wasted round trip stops.
    for _ in 0..3 {
        let response = send(&state).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            served_by(&response).as_deref(),
            Some("nearai"),
            "requests must keep being served while the circuit is open"
        );
        let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;
    }
    assert_eq!(
        *dials.lock().expect("lock"),
        wasted_dials,
        "an open circuit must not dial the dead backend again"
    );
}

#[tokio::test]
async fn a_healthy_backend_never_has_its_circuit_opened() {
    let (dead, _dials) = spawn_dead_upstream().await;
    let healthy = spawn_healthy_upstream().await;
    let state = state_for(&dead, &healthy, 2);

    for _ in 0..4 {
        let response = send(&state).await;
        let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;
    }

    let health = state.breakers.statuses();
    let nearai = health
        .iter()
        .find(|h| h.backend.as_str() == "nearai")
        .expect("tracked");
    assert_eq!(nearai.state, CircuitState::Closed);
    assert_eq!(nearai.consecutive_failures, 0);
}

#[tokio::test]
async fn the_last_backend_standing_is_tried_even_with_its_circuit_open() {
    // A breaker exists to waste less time on a failing backend, not to turn a
    // degraded proxy into a dead one. With nowhere else to go, the honest move
    // is to try anyway and report the provider's real error — refusing outright
    // would keep failing for the whole recovery window even after the provider
    // came back.
    let (dead, dials) = spawn_dead_upstream().await;
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        AnthropicBackend::api_key(SecretString::from("sk-ant-test"), Some(dead.clone()), 5)
            .expect("client builds"),
    ));
    let mut state = AppState::new(
        registry,
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    );
    state.breakers = Arc::new(BreakerBoard::new(CircuitBreakerConfig {
        failure_threshold: 1,
        recovery_timeout: std::time::Duration::from_secs(300),
        half_open_successes_needed: 1,
    }));

    let first = send(&state).await;
    assert_ne!(first.status(), StatusCode::OK);
    let after_first = *dials.lock().expect("lock");
    assert!(after_first > 0);

    let second = send(&state).await;
    assert_ne!(second.status(), StatusCode::OK);
    assert!(
        *dials.lock().expect("lock") > after_first,
        "with no alternative, IronWire must still try rather than refuse on our own authority"
    );

    // And the client gets an error it already knows how to handle.
    let body = axum::body::to_bytes(second.into_body(), 64 * 1024)
        .await
        .expect("body");
    let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(value["type"], "error");
    assert!(value["error"]["type"].is_string());
}

/// An open circuit does not make a backend the user named one that does not
/// exist. Dropped from the candidates, a pinned backend with an open circuit
/// was answered "not a connected backend" about one `status` still lists. It
/// is tried, as the last backend standing is, and the provider's own error is
/// what comes back.
#[tokio::test]
async fn a_pinned_backend_with_its_circuit_open_is_tried_rather_than_called_unknown() {
    let (dead, dials) = spawn_dead_upstream().await;
    let healthy = spawn_healthy_upstream().await;
    let state = state_for(&dead, &healthy, 1);

    let opened = send(&state).await;
    assert_eq!(served_by(&opened).as_deref(), Some("nearai"));
    let before = *dials.lock().expect("lock");

    let pinned = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/anthropic/v1/messages")
                .header("content-type", "application/json")
                .header("x-ironwire-route", "anthropic-key")
                .body(Body::from(BODY))
                .expect("request builds"),
        )
        .await
        .expect("served");
    assert_ne!(pinned.status(), StatusCode::OK);
    assert!(
        *dials.lock().expect("lock") > before,
        "the backend the request named was not tried"
    );
    let body = axum::body::to_bytes(pinned.into_body(), 64 * 1024)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&body);
    assert!(!text.contains("not a connected backend"), "{text}");
}
