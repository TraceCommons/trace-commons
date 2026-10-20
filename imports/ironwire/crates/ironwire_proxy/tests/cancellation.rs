//! Cancellation: an abandoned request must stop generating upstream.
//!
//! `docs/PROTOCOL.md` §4. Coding agents abandon requests constantly — the user
//! hits Esc, the agent decides it has enough, the editor closes. If IronWire
//! keeps the upstream request alive after the client is gone, it burns exactly
//! the scarce subscription quota it exists to protect, silently, at scale.
//!
//! This is also the test that proves the ledger and quota accounting survive a
//! cancelled request: both hang off the observation tee's `Drop`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use futures_util::StreamExt;
use ironwire_core::config::Config;
use ironwire_creds::ConsentLedger;
use ironwire_ledger::Ledger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// What the upstream observed about its own connection's fate.
#[derive(Debug, Default)]
struct UpstreamOutcome {
    /// Frames written before the connection went away.
    frames_written: usize,
    /// Whether the write side failed, i.e. the client hung up.
    disconnected: bool,
}

/// An upstream that streams a frame every 50ms, forever, and records when its
/// connection dies. A real provider behaves the same way: it keeps generating
/// until someone stops listening.
async fn spawn_endless_upstream() -> (String, Arc<Mutex<UpstreamOutcome>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let outcome = Arc::new(Mutex::new(UpstreamOutcome::default()));
    let sink = Arc::clone(&outcome);

    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        // Drain the request head; we do not care what it says.
        let mut scratch = [0u8; 8192];
        let _ = socket.read(&mut scratch).await;

        let head = "HTTP/1.1 200 OK\r\n\
                    content-type: text/event-stream\r\n\
                    transfer-encoding: chunked\r\n\r\n";
        if socket.write_all(head.as_bytes()).await.is_err() {
            return;
        }

        // Usage arrives first, exactly as Anthropic sends it, so the test can
        // also assert that a cancelled request still records what it consumed.
        let first = concat!(
            "event: message_start\n",
            r#"data: {"type":"message_start","message":{"model":"claude-opus-4-6","usage":{"input_tokens":9}}}"#,
            "\n\n",
        );
        let mut frames = 0usize;
        for body in std::iter::once(first.to_string())
            .chain((0..).map(|i| format!("event: ping\ndata: {{\"n\":{i}}}\n\n")))
        {
            let chunk = format!("{:x}\r\n{body}\r\n", body.len());
            if socket.write_all(chunk.as_bytes()).await.is_err() || socket.flush().await.is_err() {
                let mut outcome = sink.lock().expect("lock");
                outcome.frames_written = frames;
                outcome.disconnected = true;
                return;
            }
            frames += 1;
            // A runaway guard: if cancellation never propagates, stop rather
            // than hanging the suite, and leave `disconnected` false so the
            // assertion below fails with a clear message.
            if frames > 400 {
                let mut outcome = sink.lock().expect("lock");
                outcome.frames_written = frames;
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    });

    (format!("http://{addr}"), outcome)
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

const BODY: &str =
    r#"{"model":"claude-opus-4-6","stream":true,"messages":[{"role":"user","content":"hi"}]}"#;

#[tokio::test]
async fn dropping_the_client_stream_aborts_the_upstream_request() {
    let (base_url, outcome) = spawn_endless_upstream().await;
    let ledger = Ledger::in_memory().expect("ledger opens");

    let response = app(state_for(&base_url, ledger.clone()))
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

    // Read a couple of frames, then walk away — exactly what a coding agent
    // does when the user interrupts it.
    let mut stream = response.into_body().into_data_stream();
    for _ in 0..2 {
        let _ = stream.next().await;
    }
    drop(stream);

    // The upstream should notice within a couple of write attempts.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if outcome.lock().expect("lock").disconnected {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "upstream kept generating after the client left — abandoned \
             requests are burning subscription quota"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let written = outcome.lock().expect("lock").frames_written;
    assert!(
        written < 100,
        "upstream wrote {written} frames after a 2-frame read; cancellation \
         is not propagating promptly"
    );
}

#[tokio::test]
async fn a_cancelled_request_still_records_what_it_consumed() {
    let (base_url, _outcome) = spawn_endless_upstream().await;
    let ledger = Ledger::in_memory().expect("ledger opens");

    let response = app(state_for(&base_url, ledger.clone()))
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

    let mut stream = response.into_body().into_data_stream();
    for _ in 0..2 {
        let _ = stream.next().await;
    }
    drop(stream);

    // The ledger write happens on the tee's Drop, which may land a beat after
    // the stream is dropped.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let recorded = loop {
        let rows = ledger.recent(10).expect("reads");
        if let Some(row) = rows.first() {
            break row.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "an abandoned request left no ledger entry; its token spend would \
             be invisible to the user"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    };

    assert_eq!(recorded.backend, "anthropic-key");
    assert_eq!(
        recorded.input_tokens,
        Some(9),
        "usage the provider already reported must survive cancellation"
    );
    assert_eq!(recorded.served_model.as_deref(), Some("claude-opus-4-6"));
}
