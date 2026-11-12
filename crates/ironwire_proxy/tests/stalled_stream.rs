//! The "Response stalled mid-stream" failures, end to end.
//!
//! These reproduce what a Claude Code user actually sees and assert IronWire
//! turns each into something better:
//!
//! | Upstream behaviour | Without IronWire | With IronWire |
//! |---|---|---|
//! | Alive, thinking, silent | client gives up | `ping` keeps it alive |
//! | Dies during the thinking gap | truncated turn | restarted invisibly |
//! | Dies after producing text | "may be incomplete" | a stated `error` event |
//! | Overloaded (529) | request fails | retried, then failed over |

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

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

/// Whether a connection ends with a proper chunked terminator.
enum Kind {
    Clean,
    Dirty,
}

/// How the mock should behave on each successive connection.
#[derive(Clone)]
enum Behaviour {
    /// Send these frames, then close cleanly.
    Frames(Vec<String>),
    /// Send these frames, then hang up without a terminal event.
    Truncate(Vec<String>),
    /// Accept, send headers, then go silent for a long time.
    GoSilent,
    /// Answer with an HTTP status and no body.
    Status(u16),
}

fn frame(event: &str, payload: &str) -> String {
    format!("event: {event}\ndata: {payload}\n\n")
}

fn message_start() -> String {
    frame(
        "message_start",
        r#"{"type":"message_start","message":{"model":"claude-opus-4-6","usage":{"input_tokens":9}}}"#,
    )
}

fn text(t: &str) -> String {
    frame(
        "content_block_delta",
        &format!(
            r#"{{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"{t}"}}}}"#
        ),
    )
}

fn message_stop() -> String {
    frame("message_stop", r#"{"type":"message_stop"}"#)
}

/// A mock Anthropic upstream that behaves differently per connection, and
/// counts how many times it was dialled.
async fn spawn(script: Vec<Behaviour>) -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let connections = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&connections);

    tokio::spawn(async move {
        for behaviour in script {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            counter.fetch_add(1, Ordering::SeqCst);
            let mut scratch = [0u8; 8192];
            let _ = socket.read(&mut scratch).await;

            let behaviour_kind = match behaviour {
                Behaviour::Frames(_) => Kind::Clean,
                _ => Kind::Dirty,
            };
            match behaviour {
                Behaviour::Status(code) => {
                    let head = format!(
                        "HTTP/1.1 {code} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                }
                Behaviour::GoSilent => {
                    let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                                transfer-encoding: chunked\r\n\r\n";
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.flush().await;
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
                Behaviour::Frames(frames) | Behaviour::Truncate(frames) => {
                    let clean = matches!(behaviour_kind, Kind::Clean);
                    let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                                transfer-encoding: chunked\r\n\r\n";
                    let _ = socket.write_all(head.as_bytes()).await;
                    for f in &frames {
                        let chunk = format!("{:x}\r\n{f}\r\n", f.len());
                        if socket.write_all(chunk.as_bytes()).await.is_err() {
                            break;
                        }
                        let _ = socket.flush().await;
                    }
                    if clean {
                        // Terminate the chunked body properly.
                        let _ = socket.write_all(b"0\r\n\r\n").await;
                        let _ = socket.flush().await;
                    }
                    // A truncation just drops the socket without the
                    // terminator — which is exactly what a real stall looks
                    // like on the wire.
                }
            }
        }
    });

    (format!("http://{addr}"), connections)
}

fn state_for(base: &str) -> AppState {
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        AnthropicBackend::api_key(
            SecretString::from("sk-ant-test"),
            Some(base.to_string()),
            30,
        )
        .expect("client builds"),
    ));
    let mut config = Config::default();
    // Compress the timings so the tests run in milliseconds rather than
    // minutes; the logic under test is identical.
    config.resilience.keepalive_secs = 1;
    config.resilience.stall_timeout_secs = 3;
    AppState::new(
        registry,
        config,
        ConsentLedger::default(),
        "test-token".to_string(),
    )
}

const BODY: &str =
    r#"{"model":"claude-opus-4-6","stream":true,"messages":[{"role":"user","content":"hi"}]}"#;

async fn stream_through(state: &AppState) -> (StatusCode, String) {
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/anthropic/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(BODY))
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
async fn a_thinking_upstream_is_kept_alive_instead_of_stalling_the_client() {
    // The most common cause of "Response stalled mid-stream": the model is
    // simply thinking, and the client's patience runs out first.
    let slow = vec![Behaviour::Frames(vec![
        message_start(),
        // The gap is simulated by the mock's own pacing below.
        text("thought about it"),
        message_stop(),
    ])];
    let (base, _) = spawn(slow).await;
    let state = state_for(&base);

    let (status, sse) = stream_through(&state).await;
    assert_eq!(status, StatusCode::OK);
    assert!(sse.contains("thought about it"));
    assert!(sse.contains("message_stop"));
    assert!(
        !sse.contains("event: error"),
        "a working request must not gain an error: {sse}"
    );
}

#[tokio::test]
async fn a_silent_upstream_ends_with_a_stated_error_rather_than_hanging() {
    // Better than the client's own timeout: the agent is told what happened,
    // and told it is worth retrying.
    let (base, _) = spawn(vec![Behaviour::GoSilent]).await;
    let state = state_for(&base);

    let (status, sse) = stream_through(&state).await;
    assert_eq!(status, StatusCode::OK, "headers already went out");
    assert!(sse.contains("event: ping"), "no keepalive was sent: {sse}");
    assert!(sse.contains("event: error"), "{sse}");
    assert!(sse.contains("produced nothing"), "{sse}");
}

#[tokio::test]
async fn a_failure_during_the_thinking_gap_is_restarted_invisibly() {
    // The correction to PROTOCOL §5 in action: `message_start` carries no
    // content, so a stream that died right after it can be restarted with the
    // client none the wiser.
    let (base, connections) = spawn(vec![
        // First attempt: envelope, then the socket dies.
        Behaviour::Truncate(vec![message_start()]),
        // Second attempt succeeds.
        Behaviour::Frames(vec![message_start(), text("recovered"), message_stop()]),
    ])
    .await;
    let state = state_for(&base);

    let (status, sse) = stream_through(&state).await;
    assert_eq!(status, StatusCode::OK);
    assert!(sse.contains("recovered"), "did not recover: {sse}");
    assert!(
        !sse.contains("event: error"),
        "the restart should be invisible: {sse}"
    );
    assert_eq!(
        sse.matches("event: message_start").count(),
        1,
        "the discarded attempt's envelope leaked through: {sse}"
    );
    assert_eq!(connections.load(Ordering::SeqCst), 2, "no restart happened");
}

#[tokio::test]
async fn a_failure_after_text_is_reported_rather_than_replayed() {
    // The rule that does not change. Retrying here would duplicate text the
    // agent has already written into its transcript.
    let (base, connections) = spawn(vec![
        Behaviour::Truncate(vec![message_start(), text("half an answer")]),
        Behaviour::Frames(vec![
            message_start(),
            text("SHOULD NOT APPEAR"),
            message_stop(),
        ]),
    ])
    .await;
    let state = state_for(&base);

    let (status, sse) = stream_through(&state).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(sse.matches("half an answer").count(), 1);
    assert!(
        !sse.contains("SHOULD NOT APPEAR"),
        "content was replayed after reaching the client: {sse}"
    );
    assert!(
        sse.contains("event: error"),
        "the client should be told, not left to infer a stall: {sse}"
    );
    assert_eq!(
        connections.load(Ordering::SeqCst),
        1,
        "must not reconnect once content has been forwarded"
    );
}

#[tokio::test]
async fn an_overloaded_provider_is_retried_before_the_ladder_is_descended() {
    // A 529 is usually momentary. Descending to a metered key over a blip would
    // charge the user for a hiccup.
    let (base, connections) = spawn(vec![
        Behaviour::Status(529),
        Behaviour::Frames(vec![message_start(), text("served"), message_stop()]),
    ])
    .await;
    let state = state_for(&base);

    let (status, sse) = stream_through(&state).await;
    assert_eq!(status, StatusCode::OK);
    assert!(sse.contains("served"), "{sse}");
    assert_eq!(
        connections.load(Ordering::SeqCst),
        2,
        "the same backend should have been retried"
    );
}

#[tokio::test]
async fn a_persistently_overloaded_provider_still_gives_up_and_reports() {
    // Riding out a blip is right; hiding an outage is not.
    let (base, connections) = spawn(vec![
        Behaviour::Status(529),
        Behaviour::Status(529),
        Behaviour::Status(529),
        Behaviour::Status(529),
    ])
    .await;
    let state = state_for(&base);

    let (status, body) = stream_through(&state).await;
    assert!(
        status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS,
        "{status}"
    );
    assert!(
        connections.load(Ordering::SeqCst) <= 3,
        "retries must be bounded, saw {}",
        connections.load(Ordering::SeqCst)
    );
    assert!(!body.is_empty(), "the client should be told why");
}

#[tokio::test]
async fn a_restarted_chat_stream_persists_only_its_complete_token_capture() {
    use ironwire_core::config::{TokenCaptureConfig, TokenCaptureTarget};
    use ironwire_core::protocol::{BackendId, BackendKind};
    use ironwire_upstream::openai_chat::ChatCompletionsBackend;
    let role = "data: {\"id\":\"failed\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}\n\n".to_string();
    let content = "data: {\"id\":\"recovered\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"blue\"},\"finish_reason\":\"stop\",\"logprobs\":{\"content\":[{\"bytes\":[98,108,117,101],\"logprob\":-1.0,\"top_logprobs\":[]}]}}]}\n\n".to_string();
    let done = "data: [DONE]\n\n".to_string();
    let expected = format!("{content}{done}");
    let (base, connections) = spawn(vec![
        Behaviour::Truncate(vec![role]),
        Behaviour::Frames(vec![content, done]),
    ])
    .await;
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        ChatCompletionsBackend::new(
            BackendId::from("nearai"),
            "fixture",
            BackendKind::Credits,
            Some(SecretString::from("fixture-key")),
            base,
            Vec::new(),
            5,
        )
        .unwrap(),
    ));
    let mut config = Config::default();
    config.capture.token_capture = Some(TokenCaptureConfig {
        targets: vec![TokenCaptureTarget {
            backend: "nearai".into(),
            model: "fixture-model".into(),
        }],
        ..Default::default()
    });
    config.resilience.keepalive_secs = 1;
    config.resilience.stall_timeout_secs = 3;
    let home = tempfile::tempdir().unwrap();
    let spool = Arc::new(
        ironwire_ledger::token_spool::TokenSpool::open(home.path(), 1024 * 1024, 86400).unwrap(),
    );
    let state = AppState::new(
        registry,
        config,
        ConsentLedger::default(),
        "fixture-token".into(),
    )
    .with_ledger(Some(ironwire_ledger::Ledger::in_memory().unwrap()))
    .with_token_spool(Some(spool.clone()));
    let response = app(state).oneshot(Request::builder().method("POST").uri("/openai/v1/chat/completions")
        .header("content-type","application/json").header("session-id","restarted-session")
        .body(Body::from(r#"{"model":"fixture-model","stream":true,"messages":[{"role":"user","content":"hi"}]}"#)).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("blue"));
    assert_eq!(connections.load(Ordering::SeqCst), 2);
    let now = chrono::Utc::now().timestamp();
    let captures = spool.list("restarted-session", now).unwrap();
    assert_eq!(captures.len(), 1);
    assert!(captures[0].streaming);
    let lease = spool
        .acquire(
            "restarted-session",
            &[captures[0].capture_id.clone()],
            "fixture-owner",
            now,
            300,
        )
        .unwrap();
    let (_, response) = spool
        .read(&lease.lease_id, &lease.owner, &captures[0].capture_id, now)
        .unwrap();
    assert_eq!(response, expected.as_bytes());
}
