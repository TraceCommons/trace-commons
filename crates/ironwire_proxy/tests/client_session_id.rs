//! The ledger records which agent session an exchange belonged to.
//!
//! `Exchange::conversation` cannot answer that. It is a routing-affinity key --
//! protocol family, the head of the preamble, the tool list -- deliberately
//! stable across a whole session, and therefore equally stable across two
//! different sessions that share a tool list. The agent's own session header is
//! the identifier that actually distinguishes them.
//!
//! These cases pin the wiring end to end: a header on the inbound request has
//! to survive routing and land on the row, and its absence has to leave the row
//! addressed by nothing rather than by a guess.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::Config;
use ironwire_creds::ConsentLedger;
use ironwire_ledger::Ledger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use secrecy::SecretString;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// An upstream that answers one request with a canned non-streaming reply.
async fn upstream() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut chunk = [0u8; 8192];
        let _ = socket.read(&mut chunk).await;
        let body = json!({
            "id": "msg_x",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-4-6",
            "content": [{"type": "text", "text": "ok"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 2}
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ = socket.write_all(response.as_bytes()).await;
        let _ = socket.flush().await;
    });
    format!("http://{addr}")
}

fn state_for(base_url: &str, ledger: Ledger) -> AppState {
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
    .with_ledger(Some(ledger))
}

fn request(header: &'static str, session: Option<&str>) -> Request<Body> {
    let body = json!({
        "model": "claude-opus-4-6",
        "max_tokens": 64,
        "messages": [{"role": "user", "content": "hello"}]
    })
    .to_string();
    let mut builder = Request::builder()
        .method("POST")
        .uri("/anthropic/v1/messages")
        .header("content-type", "application/json");
    if let Some(session) = session {
        builder = builder.header(header, session);
    }
    builder.body(Body::from(body)).expect("request builds")
}

async fn recorded_under(header: &'static str, session: Option<&str>) -> Option<String> {
    let base = upstream().await;
    let ledger = Ledger::in_memory().expect("ledger opens");
    let state = state_for(&base, ledger.clone());
    let response = app(state)
        .oneshot(request(header, session))
        .await
        .expect("served");
    assert_eq!(response.status(), StatusCode::OK);

    // The ledger write happens when the response body is consumed.
    let _ = axum::body::to_bytes(response.into_body(), usize::MAX).await;
    let rows = ledger.recent(1).expect("read the ledger");
    assert_eq!(rows.len(), 1, "the exchange was recorded");
    rows[0].client_session_id.clone()
}

/// The native header Claude Code already sends.
async fn recorded(session: Option<&str>) -> Option<String> {
    recorded_under("x-claude-code-session-id", session).await
}

/// The vendor-neutral header any client can be configured to send.
async fn recorded_neutral(session: &str) -> Option<String> {
    recorded_under("x-ironwire-session-id", Some(session)).await
}

#[tokio::test]
async fn the_session_the_agent_named_is_the_session_on_the_row() {
    assert_eq!(
        recorded(Some("5db811ed-ce4a-45a7-ab00-56890e111668")).await,
        Some("5db811ed-ce4a-45a7-ab00-56890e111668".to_string())
    );
}

#[tokio::test]
async fn a_neutral_session_header_reaches_the_row_too() {
    // A client with no native session header -- Aider, Cline, Roo -- names its
    // session with this one. Nothing about it is Anthropic-specific, so the
    // value has to survive the whole route on a façade that also has a native
    // header of its own.
    assert_eq!(
        recorded_neutral("aider-1").await,
        Some("aider-1".to_string())
    );
}

#[tokio::test]
async fn a_client_that_names_no_session_leaves_the_row_unaddressed() {
    assert_eq!(recorded(None).await, None);
}

#[tokio::test]
async fn a_session_header_that_is_not_an_identifier_is_not_recorded() {
    // Client-supplied text that `ironwire log` renders to a terminal. A row
    // addressed by nothing beats a row carrying whatever a client sent.
    assert_eq!(recorded(Some("not an id")).await, None);
}
