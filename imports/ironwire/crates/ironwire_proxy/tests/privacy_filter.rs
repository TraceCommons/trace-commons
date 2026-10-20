//! Conformance: the privacy filter, end to end.
//!
//! `docs/PRIVACY.md` §9 specifies these before the code existed. The two that
//! matter most are the first and the last: with the filter **off** nothing
//! changes at all, and with it **on** the provider never sees the value while
//! the client never sees a placeholder.

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

/// What the provider actually received.
type Seen = Arc<Mutex<String>>;

/// The text the upstream will put in its next response. Shared so a test can
/// change it *without* rebuilding the daemon — which matters, because a fresh
/// `AppState` means a fresh salt, and a fresh salt means the token from the
/// previous request is correctly not in the new map.
type Reply = Arc<Mutex<String>>;

/// An upstream that records the request body and echoes a settable SSE response.
async fn spawn_upstream(initial: String) -> (String, Seen, Reply) {
    let reply: Reply = Arc::new(Mutex::new(initial));
    let (base, seen) = spawn_upstream_with(Arc::clone(&reply)).await;
    (base, seen, reply)
}

async fn spawn_upstream_with(reply: Reply) -> (String, Seen) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let seen: Seen = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&seen);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let sink = Arc::clone(&sink);
            let reply = Arc::clone(&reply);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                loop {
                    let Ok(n) = socket.read(&mut chunk).await else {
                        return;
                    };
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf);
                    if let Some(split) = text.find("\r\n\r\n") {
                        let head = &text[..split];
                        let length: usize = head
                            .lines()
                            .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                            .and_then(|l| l.split(':').nth(1))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        if buf.len() >= split + 4 + length {
                            *sink.lock().expect("lock") =
                                text[split + 4..split + 4 + length].to_string();
                            break;
                        }
                    }
                }

                let sse = format!(
                    "event: message_start\n\
                     data: {{\"type\":\"message_start\",\"message\":{{\"id\":\"m\",\"usage\":{{\"input_tokens\":1}}}}}}\n\n\
                     event: content_block_delta\n\
                     data: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":{}}}}}\n\n\
                     event: message_stop\n\
                     data: {{\"type\":\"message_stop\"}}\n\n",
                    serde_json::to_string(&*reply.lock().expect("lock")).expect("encodes")
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

    (format!("http://{addr}"), seen)
}

const SECRET_EMAIL: &str = "alice@internal-corp.example-real.com";

fn body_containing(value: &str) -> String {
    serde_json::json!({
        "model": "claude-opus-4-6",
        "stream": true,
        "system": "You are Claude Code",
        "messages": [{"role": "user", "content": format!("email {value} about the outage")}],
    })
    .to_string()
}

fn state_for(base_url: &str, privacy: PrivacyConfig) -> AppState {
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        AnthropicBackend::api_key(
            SecretString::from("sk-ant-test"),
            Some(base_url.to_string()),
            10,
        )
        .expect("client builds"),
    ));
    let config = Config {
        privacy,
        ..Config::default()
    };
    AppState::new(
        registry,
        config,
        ConsentLedger::default(),
        "test-token".to_string(),
    )
}

fn filter_on() -> PrivacyConfig {
    PrivacyConfig {
        enabled: true,
        secrets: true,
        named_values: vec![SECRET_EMAIL.to_string()],
        ..PrivacyConfig::default()
    }
}

/// The placeholder the daemon actually sent upstream.
fn minted_token(seen: &Seen) -> String {
    let received = seen.lock().expect("lock").clone();
    received
        .split(['"', ' '])
        .find(|piece| piece.starts_with('\u{27e6}'))
        .unwrap_or_else(|| panic!("no placeholder was sent:\n{received}"))
        .to_string()
}

async fn send(state: AppState, body: String) -> (StatusCode, String) {
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/anthropic/v1/messages")
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

#[tokio::test]
async fn with_the_filter_off_the_body_reaches_the_provider_unchanged() {
    // The baseline the whole product rests on. `tests/passthrough.rs` asserts
    // this too; asserting it here as well means a privacy change that breaks
    // byte-identity fails in the suite that introduced it.
    let (base, seen, _reply) = spawn_upstream("ok".to_string()).await;
    let body = body_containing(SECRET_EMAIL);

    let (status, _) = send(state_for(&base, PrivacyConfig::default()), body.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(*seen.lock().expect("lock"), body);
}

#[tokio::test]
async fn with_the_filter_on_the_provider_never_sees_the_value() {
    let (base, seen, _reply) = spawn_upstream("ok".to_string()).await;
    let (status, _) = send(state_for(&base, filter_on()), body_containing(SECRET_EMAIL)).await;
    assert_eq!(status, StatusCode::OK);

    let received = seen.lock().expect("lock").clone();
    assert!(!received.is_empty(), "the upstream saw nothing");
    assert!(
        !received.contains(SECRET_EMAIL),
        "the nominated value reached the provider:\n{received}"
    );
    // ...and what it did see is still a valid request.
    let parsed: serde_json::Value = serde_json::from_str(&received).expect("still valid JSON");
    assert_eq!(parsed["model"], "claude-opus-4-6");
    assert_eq!(parsed["stream"], true);
}

#[tokio::test]
async fn the_client_gets_the_real_value_back() {
    // The half that makes substitution usable at all. A model handed a token
    // writes about the token; the client must see the value.
    let (base, seen, reply) = spawn_upstream(String::new()).await;

    // Discover what token this conversation mints, by sending once. The same
    // daemon must be reused: a fresh one has a fresh salt, and this token would
    // then correctly not be in its map.
    let state = state_for(&base, filter_on());
    let _ = send(state.clone(), body_containing(SECRET_EMAIL)).await;
    let token = minted_token(&seen);

    // Now have the provider echo that token back, as a model quoting it would.
    *reply.lock().expect("lock") = format!("I will contact {token} shortly");
    let (status, response) = send(state, body_containing(SECRET_EMAIL)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        response.contains(SECRET_EMAIL),
        "the client did not get the real value back:\n{response}"
    );
    assert!(
        !response.contains('\u{27e6}'),
        "a placeholder leaked to the client:\n{response}"
    );
}

#[tokio::test]
async fn an_api_key_in_the_prompt_is_substituted_by_tier_one() {
    // No configuration needed: this is the tier that pays for itself.
    let (base, seen, _reply) = spawn_upstream("ok".to_string()).await;
    let key = concat!("ghp_", "abcdefghijklmnopqrstuvwxyz0123456789");
    let state = state_for(
        &base,
        PrivacyConfig {
            enabled: true,
            secrets: true,
            ..PrivacyConfig::default()
        },
    );
    let (status, _) = send(state, body_containing(key)).await;
    assert_eq!(status, StatusCode::OK);

    let received = seen.lock().expect("lock").clone();
    assert!(
        !received.contains(key),
        "a GitHub token reached the provider:\n{received}"
    );
}

#[tokio::test]
async fn a_conversation_mints_the_same_token_every_turn() {
    // Otherwise the prompt prefix changes every turn and the provider's cache
    // is destroyed on every request — costing far more than the filter saves.
    let (base, seen, _reply) = spawn_upstream("ok".to_string()).await;
    let state = state_for(&base, filter_on());

    let _ = send(state.clone(), body_containing(SECRET_EMAIL)).await;
    let first = seen.lock().expect("lock").clone();
    let _ = send(state, body_containing(SECRET_EMAIL)).await;
    let second = seen.lock().expect("lock").clone();

    assert_eq!(first, second, "the placeholder changed between turns");
}

#[tokio::test]
async fn a_response_that_ends_mid_placeholder_fails_rather_than_corrupting() {
    // The compaction hazard in miniature (`docs/PRIVACY.md` §5). Forwarding a
    // fragment writes a token into the client's permanent transcript, where it
    // can never be reversed again.
    let (base, seen, reply) = spawn_upstream(String::new()).await;
    let state = state_for(&base, filter_on());
    let _ = send(state.clone(), body_containing(SECRET_EMAIL)).await;
    let token = minted_token(&seen);

    // Echo back a truncated token — what a stream cut short would produce.
    let truncated: String = token.chars().take(token.chars().count() - 3).collect();
    *reply.lock().expect("lock") = format!("contacting {truncated}");
    let (_, response) = send(state, body_containing(SECRET_EMAIL)).await;

    assert!(
        !response.contains(SECRET_EMAIL),
        "a half-reversed response was forwarded:\n{response}"
    );
    assert!(
        response.contains("error") || response.contains("mid-substitution"),
        "the failure was not surfaced:\n{response}"
    );
}

#[tokio::test]
async fn enabling_the_filter_with_nothing_to_match_leaves_it_off() {
    // `ironwire status` must not claim a filter is running when it cannot
    // possibly do anything (`docs/TRUST.md` I7).
    let (base, seen, _reply) = spawn_upstream("ok".to_string()).await;
    let body = body_containing(SECRET_EMAIL);
    let state = state_for(
        &base,
        PrivacyConfig {
            enabled: true,
            secrets: false,
            named_values: Vec::new(),
            ..PrivacyConfig::default()
        },
    );
    assert!(state.privacy().is_none());

    let (status, _) = send(state, body.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(*seen.lock().expect("lock"), body);
}
