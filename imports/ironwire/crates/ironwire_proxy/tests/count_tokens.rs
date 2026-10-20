//! Conformance: `count_tokens` and the privacy filter agree with each other.
//!
//! Claude Code drives its context budget and its compaction trigger off
//! `POST /v1/messages/count_tokens` (`docs/PROTOCOL.md` §1). The privacy filter
//! changes the text that is sent. Put those together and there is an obvious
//! failure waiting: a count that describes different bytes than the request it
//! is predicting, so the client compacts at the wrong moment — too late, and
//! the real request overflows the context.
//!
//! It does not happen, and the reason is worth pinning rather than leaving to
//! be rediscovered: `count_tokens` goes through the **same pipeline** as
//! `messages`, so both are substituted identically and the count describes
//! exactly the bytes that will be sent.
//!
//! That is a property of the routing, not of the filter, and it is the kind of
//! thing a later change could break while looking like a cleanup — "why are we
//! running the privacy filter on a token count?" has an answer, and this is it.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::{Config, PrivacyConfig};
use ironwire_creds::ConsentLedger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// Every body the upstream received, in order.
type Seen = Arc<Mutex<Vec<String>>>;

/// A mock that answers both `count_tokens` (JSON) and `messages` (SSE), and
/// records what it was asked.
async fn spawn() -> (String, Seen) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let sink = Arc::clone(&sink);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                let (path, body) = loop {
                    let Ok(n) = socket.read(&mut chunk).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(split) = text.find("\r\n\r\n") {
                        let head = &text[..split];
                        let length: usize = head
                            .lines()
                            .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                            .and_then(|l| l.split(':').nth(1))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        if buf.len() >= split + 4 + length {
                            let path = head
                                .lines()
                                .next()
                                .and_then(|line| line.split_whitespace().nth(1))
                                .unwrap_or_default()
                                .to_string();
                            break (path, text[split + 4..split + 4 + length].to_string());
                        }
                    }
                };
                sink.lock().expect("lock").push(body.clone());

                // Count in a way the test can predict: bytes, not tokens. What
                // matters here is *which bytes* the provider was given.
                let (content_type, payload) = if path.ends_with("count_tokens") {
                    (
                        "application/json",
                        format!("{{\"input_tokens\":{}}}", body.len()),
                    )
                } else {
                    (
                        "text/event-stream",
                        concat!(
                            "event: message_start\n",
                            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"usage\":{\"input_tokens\":1}}}\n\n",
                            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
                        )
                        .to_string(),
                    )
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n\r\n",
                    payload.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(payload.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });

    (format!("http://{addr}"), seen)
}

const SECRET: &str = "alice@a-very-long-internal-domain-name-indeed.example-real.com";

fn state_for(base: &str, privacy: PrivacyConfig) -> AppState {
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        AnthropicBackend::api_key(
            SecretString::from("sk-ant-test"),
            Some(base.to_string()),
            10,
        )
        .expect("client builds"),
    ));
    AppState::new(
        registry,
        Config {
            privacy,
            ..Config::default()
        },
        ConsentLedger::default(),
        "test-token".to_string(),
    )
}

fn body(stream: bool) -> String {
    serde_json::json!({
        "model": "claude-opus-4-6",
        "stream": stream,
        "system": "You are Claude Code",
        "messages": [{"role": "user", "content": format!("mail {SECRET} about it")}],
    })
    .to_string()
}

async fn post(state: AppState, path: &str, body: String) -> (StatusCode, String) {
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await
        .expect("served");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

fn filter_on() -> PrivacyConfig {
    PrivacyConfig {
        enabled: true,
        secrets: true,
        named_values: vec![SECRET.to_string()],
        ..PrivacyConfig::default()
    }
}

#[tokio::test]
async fn a_token_count_describes_the_bytes_that_will_actually_be_sent() {
    // The property the client's compaction trigger depends on. If the count
    // described the *unfiltered* body while the request sent the filtered one,
    // Claude Code would compact at the wrong moment — and being wrong in the
    // "there is more room than you think" direction overflows the context.
    let (base, seen) = spawn().await;
    let state = state_for(&base, filter_on());

    let (status, _) = post(
        state.clone(),
        "/anthropic/v1/messages/count_tokens",
        body(false),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = post(state, "/anthropic/v1/messages", body(true)).await;
    assert_eq!(status, StatusCode::OK);

    let bodies = seen.lock().expect("lock").clone();
    assert_eq!(bodies.len(), 2, "both requests reached the provider");
    // Byte-for-byte, apart from the one field that legitimately differs.
    let counted = bodies[0].replace("\"stream\":false", "\"stream\":true");
    assert_eq!(
        counted, bodies[1],
        "the token count described different bytes than the request it predicts"
    );
    assert!(
        !bodies[0].contains(SECRET),
        "count_tokens leaked the value the filter exists to withhold"
    );
}

#[tokio::test]
async fn count_tokens_is_filtered_at_all() {
    // Stated separately, because exempting it would be a tempting "cleanup" —
    // it is only a count, after all — and would both leak the value and break
    // the agreement above.
    let (base, seen) = spawn().await;
    let state = state_for(&base, filter_on());

    let (status, _) = post(state, "/anthropic/v1/messages/count_tokens", body(false)).await;
    assert_eq!(status, StatusCode::OK);

    let received = seen.lock().expect("lock").first().cloned().expect("saw it");
    assert!(!received.contains(SECRET), "leaked:\n{received}");
}

#[tokio::test]
async fn a_non_streaming_request_round_trips() {
    // Most of the suite streams, because coding agents do. `count_tokens` does
    // not, and neither do plenty of third-party clients, so the non-streaming
    // path needs its own assertion rather than inheriting confidence.
    let (base, _seen) = spawn().await;
    let state = state_for(&base, PrivacyConfig::default());

    let (status, response) = post(state, "/anthropic/v1/messages/count_tokens", body(false)).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&response).expect("json came back");
    assert!(
        parsed["input_tokens"].as_u64().is_some_and(|n| n > 0),
        "got: {response}"
    );
}

#[tokio::test]
async fn with_the_filter_off_count_tokens_is_byte_identical() {
    let (base, seen) = spawn().await;
    let state = state_for(&base, PrivacyConfig::default());
    let sent = body(false);

    let (status, _) = post(state, "/anthropic/v1/messages/count_tokens", sent.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(seen.lock().expect("lock")[0], sent);
}
