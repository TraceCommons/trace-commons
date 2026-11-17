//! Conformance: one client request to a forced route is a bounded number of
//! upstream requests, however many backends are configured.
//!
//! A pin or `X-IronWire-Route` names one backend. When it fails, the pipeline
//! excludes it, and the next decision must not hand the same backend straight
//! back: that turned the attempt cap, which scales with the number of
//! backends, into the number of times the one named backend was dialled. With
//! six backends and a forced route answering 429, one client request made 18
//! upstream requests.
//!
//! These count requests at the mock upstream. They never time anything.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::Config;
use ironwire_core::protocol::{BackendId, BackendKind, ModelTier};
use ironwire_creds::ConsentLedger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use ironwire_upstream::openai_chat::ChatCompletionsBackend;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// An upstream that answers every request with `429` and the given extra
/// headers, counting the requests it receives.
async fn spawn_limited_upstream(extra_headers: &'static str) -> (String, Arc<Mutex<usize>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let requests = Arc::new(Mutex::new(0usize));
    let counter = Arc::clone(&requests);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let counter = Arc::clone(&counter);
            tokio::spawn(async move {
                let mut chunk = [0u8; 16 * 1024];
                let _ = socket.read(&mut chunk).await;
                *counter.lock().expect("lock") += 1;
                let body =
                    r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#;
                let head = format!(
                    "HTTP/1.1 429 Too Many Requests\r\ncontent-type: application/json\r\n\
                     {extra_headers}content-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(body.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });

    (format!("http://{addr}"), requests)
}

/// An upstream that answers anything with a short stream, counting requests.
async fn spawn_healthy_upstream() -> (String, Arc<Mutex<usize>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let requests = Arc::new(Mutex::new(0usize));
    let counter = Arc::clone(&requests);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let counter = Arc::clone(&counter);
            tokio::spawn(async move {
                let mut chunk = [0u8; 16 * 1024];
                let _ = socket.read(&mut chunk).await;
                *counter.lock().expect("lock") += 1;
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

    (format!("http://{addr}"), requests)
}

const BODY: &str = concat!(
    r#"{"model":"claude-opus-4-6","stream":true,"#,
    r#""system":"You are Claude Code","#,
    r#""messages":[{"role":"user","content":"hi"}]}"#,
);

/// Six backends: `anthropic-key` at `limited`, and five metered
/// OpenAI-compatible spares at `spare`, any of which could take the request.
fn six_backends(limited: &str, spare: &str) -> AppState {
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        AnthropicBackend::api_key(
            SecretString::from("sk-ant-test"),
            Some(limited.to_string()),
            5,
        )
        .expect("client builds"),
    ));
    for n in 1..=5 {
        registry.push(Arc::new(
            ChatCompletionsBackend::new(
                BackendId(format!("spare-{n}")),
                &format!("Spare {n}"),
                BackendKind::ApiKey,
                Some(SecretString::from("spare-key")),
                spare.to_string(),
                vec![("spare-model".to_string(), ModelTier::Frontier)],
                5,
            )
            .expect("client builds"),
        ));
    }
    AppState::new(
        registry,
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    )
}

async fn send(state: &AppState, route: Option<&str>) -> StatusCode {
    let mut request = Request::builder()
        .method("POST")
        .uri("/anthropic/v1/messages")
        .header("content-type", "application/json");
    if let Some(route) = route {
        request = request.header("x-ironwire-route", route);
    }
    app(state.clone())
        .oneshot(request.body(Body::from(BODY)).expect("request builds"))
        .await
        .expect("served")
        .status()
}

fn count(requests: &Arc<Mutex<usize>>) -> usize {
    *requests.lock().expect("lock")
}

/// A 429 that names a real wait is not retried on the same backend, and a
/// forced route has nowhere else to go: one client request, one upstream
/// request. `main` before multi-login made 8 here; the PR as first written
/// made 18.
#[tokio::test]
async fn a_route_header_answered_with_a_long_429_is_dialled_once() {
    let (limited, limited_requests) = spawn_limited_upstream("retry-after: 60\r\n").await;
    let (spare, spare_requests) = spawn_healthy_upstream().await;
    let state = six_backends(&limited, &spare);

    let status = send(&state, Some("anthropic-key")).await;

    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        count(&limited_requests),
        1,
        "the forced route was dialled again after the pipeline excluded it"
    );
    assert_eq!(
        count(&spare_requests),
        0,
        "a forced route was served by a backend it did not name"
    );
}

/// The same through the daemon-wide pin rather than the per-request header.
#[tokio::test]
async fn a_pinned_backend_answered_with_a_long_429_is_dialled_once() {
    let (limited, limited_requests) = spawn_limited_upstream("retry-after: 60\r\n").await;
    let (spare, spare_requests) = spawn_healthy_upstream().await;
    let state = six_backends(&limited, &spare);
    state
        .policy
        .lock()
        .expect("policy lock")
        .set_pin(Some(BackendId::from("anthropic-key")), None);

    let status = send(&state, None).await;

    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(count(&limited_requests), 1);
    assert_eq!(count(&spare_requests), 0);
}

/// A 429 with no wait attached is worth the same-backend retries, and no more:
/// one request plus `MAX_SAME_BACKEND_RETRIES`, not that times the number of
/// backends.
#[tokio::test]
async fn a_route_header_answered_with_a_bare_429_gets_only_the_same_backend_retries() {
    let (limited, limited_requests) = spawn_limited_upstream("").await;
    let (spare, spare_requests) = spawn_healthy_upstream().await;
    let state = six_backends(&limited, &spare);

    let status = send(&state, Some("anthropic-key")).await;

    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        count(&limited_requests),
        1 + ironwire_proxy::pipeline::MAX_SAME_BACKEND_RETRIES
    );
    assert_eq!(count(&spare_requests), 0);
}

/// The unforced path is unchanged: the same 429 sends the request on to a
/// spare, and the limited backend is dialled once.
#[tokio::test]
async fn without_a_forced_route_a_long_429_descends_to_another_backend() {
    let (limited, limited_requests) = spawn_limited_upstream("retry-after: 60\r\n").await;
    let (spare, spare_requests) = spawn_healthy_upstream().await;
    let state = six_backends(&limited, &spare);

    let status = send(&state, None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(count(&limited_requests), 1);
    assert_eq!(count(&spare_requests), 1);
}
