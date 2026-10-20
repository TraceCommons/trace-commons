//! Conformance: telling the user where their request went, and letting them
//! say where it should go.
//!
//! Two features that answer the same question from opposite ends. IronWire has
//! no channel into a coding agent's UI, so `docs/CRITIQUE.md` left open how a
//! user learns their model family changed. The answer is a side channel —
//! `/_ironwire/events` — plus a per-request override for anyone who wants to
//! decide for themselves.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::Config;
use ironwire_core::protocol::ModelTier;
use ironwire_creds::ConsentLedger;
use ironwire_proxy::events::Event;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use ironwire_upstream::openai_chat::ChatCompletionsBackend;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// An upstream that answers any Chat Completions request.
async fn spawn_chat_upstream() -> (String, Arc<Mutex<usize>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let hits = Arc::new(Mutex::new(0usize));
    let counter = Arc::clone(&hits);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            *counter.lock().expect("lock") += 1;
            tokio::spawn(async move {
                let mut chunk = [0u8; 8192];
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

    (format!("http://{addr}"), hits)
}

const BODY: &str = concat!(
    r#"{"model":"claude-opus-4-6","stream":true,"#,
    r#""system":"You are Claude Code","#,
    r#""messages":[{"role":"user","content":"hi"}]}"#,
);

/// A dead Anthropic backend (preferred) and a healthy NEAR AI one behind it.
fn state_for(nearai: &str) -> AppState {
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        AnthropicBackend::api_key(
            SecretString::from("sk-ant-test"),
            // Port 1 is never listening; the Anthropic lane is simply absent.
            Some("http://127.0.0.1:1".to_string()),
            5,
        )
        .expect("client builds"),
    ));
    registry.push(Arc::new(
        ChatCompletionsBackend::nearai(
            Some(SecretString::from("near-key")),
            Some(nearai.to_string()),
            vec![("near-x".to_string(), ModelTier::Frontier)],
            5,
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

fn request(headers: &[(&str, &str)]) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/anthropic/v1/messages")
        .header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    builder.body(Body::from(BODY)).expect("request builds")
}

#[tokio::test]
async fn a_family_change_is_announced_on_the_event_channel() {
    // The whole point: the user's agent just started talking to a different
    // model family, and nothing in their terminal would otherwise say so.
    let (nearai, _hits) = spawn_chat_upstream().await;
    let state = state_for(&nearai);
    let mut events = state.events.subscribe();

    let response = app(state.clone())
        .oneshot(request(&[]))
        .await
        .expect("served");
    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;

    let event = events.try_recv().expect("an event was published");
    match event {
        Event::Routed {
            to,
            translated,
            rung,
            ..
        } => {
            assert_eq!(to, "nearai");
            assert!(translated, "a cross-family route must say it is translated");
            assert!(
                rung.is_user_visible(),
                "a family change is the one descent worth interrupting someone for"
            );
        }
        other => panic!("expected a Routed event, got {other:?}"),
    }
}

#[tokio::test]
async fn a_conversation_that_stays_put_is_not_announced_every_turn() {
    // A "routed" line per request would bury the one line that means
    // something, and then the announcement stops working.
    let (nearai, _hits) = spawn_chat_upstream().await;
    let state = state_for(&nearai);
    let mut events = state.events.subscribe();

    for _ in 0..3 {
        let response = app(state.clone())
            .oneshot(request(&[]))
            .await
            .expect("served");
        let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;
    }

    let mut routed = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(event, Event::Routed { .. }) {
            routed += 1;
        }
    }
    assert_eq!(routed, 1, "only the first turn changed anything");
}

#[tokio::test]
async fn the_route_header_forces_a_backend_for_one_request() {
    let (nearai, hits) = spawn_chat_upstream().await;
    let state = state_for(&nearai);

    let response = app(state)
        .oneshot(request(&[("x-ironwire-route", "nearai")]))
        .await
        .expect("served");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ironwire-backend")
            .and_then(|v| v.to_str().ok()),
        Some("nearai")
    );
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;
    assert_eq!(*hits.lock().expect("lock"), 1);
}

#[tokio::test]
async fn the_route_header_is_not_forwarded_to_the_provider() {
    // It is IronWire's own vocabulary. Leaking it into someone else's API is
    // at best noise and at worst a 400 from a strict provider.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let seen = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&seen);
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut buf = vec![0u8; 8192];
        if let Ok(n) = socket.read(&mut buf).await {
            *sink.lock().expect("lock") = String::from_utf8_lossy(&buf[..n]).to_string();
        }
        let sse = "data: [DONE]\n\n";
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n",
            sse.len()
        );
        let _ = socket.write_all(head.as_bytes()).await;
        let _ = socket.write_all(sse.as_bytes()).await;
        let _ = socket.flush().await;
    });

    let state = state_for(&format!("http://{addr}"));
    let response = app(state)
        .oneshot(request(&[("x-ironwire-route", "nearai")]))
        .await
        .expect("served");
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;

    let received = seen.lock().expect("lock").to_ascii_lowercase();
    assert!(!received.is_empty(), "the upstream saw nothing");
    assert!(
        !received.contains("x-ironwire-route"),
        "IronWire's own header leaked upstream:\n{received}"
    );
}

#[tokio::test]
async fn an_unknown_route_is_refused_rather_than_quietly_ignored() {
    // The caller asked for something specific. Serving them from somewhere
    // else and saying nothing is worse than saying we could not.
    let (nearai, hits) = spawn_chat_upstream().await;
    let state = state_for(&nearai);

    let response = app(state)
        .oneshot(request(&[("x-ironwire-route", "does-not-exist")]))
        .await
        .expect("served");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let message = value["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("does-not-exist"), "got: {message}");
    assert!(
        message.contains("nearai"),
        "the error must list what is available: {message}"
    );
    assert_eq!(*hits.lock().expect("lock"), 0, "nothing should be sent");
}

#[tokio::test]
async fn the_event_endpoint_needs_the_control_token() {
    // It reports where a user's traffic is going, which is not something
    // another local user gets to watch.
    let (nearai, _hits) = spawn_chat_upstream().await;
    let response = app(state_for(&nearai))
        .oneshot(
            Request::builder()
                .uri("/_ironwire/events")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("served");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn pinning_to_a_backend_that_does_not_exist_is_refused() {
    // It used to succeed. Every subsequent request then silently ignored the
    // pin and routed normally, while `ironwire status` reported "Pinned to
    // <whatever>" — so the user believed all their traffic was on one backend
    // and it was not. Worse than the `X-IronWire-Route` version of this bug,
    // because a pin persists rather than affecting one request.
    let (nearai, _hits) = spawn_chat_upstream().await;
    let response = app(state_for(&nearai))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_ironwire/pin")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"backend":"not-a-real-backend"}"#))
                .expect("request builds"),
        )
        .await
        .expect("served");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert!(
        value["error"]
            .as_str()
            .is_some_and(|e| e.contains("not-a-real-backend")),
        "the error must name what was asked for: {value}"
    );
    // Rejected here rather than per-request precisely so the answer can list
    // what exists; without that the user has to go and find out separately.
    let available = value["available"].as_array().expect("a list of backends");
    assert!(
        available.iter().any(|id| id == "nearai"),
        "the error must name what is available: {value}"
    );
}

#[tokio::test]
async fn pinning_to_a_real_backend_still_works() {
    // The regression that would make the validation useless.
    let (nearai, _hits) = spawn_chat_upstream().await;
    let response = app(state_for(&nearai))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_ironwire/pin")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"backend":"nearai"}"#))
                .expect("request builds"),
        )
        .await
        .expect("served");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn clearing_the_pin_needs_no_backend_to_exist() {
    // `{"backend": null}` is how the pin is cleared, and it must not be
    // validated against the registry — a user clearing a stale pin should not
    // have to satisfy a check about a backend they are removing.
    let (nearai, _hits) = spawn_chat_upstream().await;
    let response = app(state_for(&nearai))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_ironwire/pin")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"backend":null}"#))
                .expect("request builds"),
        )
        .await
        .expect("served");
    assert_eq!(response.status(), StatusCode::OK);
}
