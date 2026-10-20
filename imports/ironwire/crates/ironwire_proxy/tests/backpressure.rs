//! Conformance: a slow client does not make IronWire buffer a fast upstream.
//!
//! `docs/PROTOCOL.md` §2 states the intent — "backpressure is the client's; a
//! slow reader slows the upstream read" — and that is a claim about memory, not
//! about politeness. A proxy that reads as fast as the provider sends, while
//! the client reads slowly, holds the difference in RAM. On a coding agent
//! streaming a large file edit that difference is megabytes per conversation,
//! and it is unbounded.
//!
//! It holds because nothing between the upstream body and the client's socket
//! is a queue. The tee that observes usage is the one place that could become
//! one, and it drops rather than blocking.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::StreamExt;
use ironwire_core::config::Config;
use ironwire_creds::ConsentLedger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// How many bytes the upstream has managed to push.
type Written = Arc<AtomicUsize>;

/// An upstream that streams as fast as the socket will take it, and reports how
/// far it got.
async fn spawn_firehose(frames: usize) -> (String, Written) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let written: Written = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&written);

    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut buf = [0u8; 8192];
        let _ = socket.read(&mut buf).await;

        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                    transfer-encoding: chunked\r\n\r\n";
        if socket.write_all(head.as_bytes()).await.is_err() {
            return;
        }

        // A 4 KB text delta per frame: roughly what a file edit streams.
        let payload = "x".repeat(4096);
        let frame = format!(
            "event: content_block_delta\n\
             data: {{\"type\":\"content_block_delta\",\"index\":0,\
             \"delta\":{{\"type\":\"text_delta\",\"text\":\"{payload}\"}}}}\n\n"
        );
        let chunked = format!("{:x}\r\n{frame}\r\n", frame.len());

        for _ in 0..frames {
            if socket.write_all(chunked.as_bytes()).await.is_err() {
                return;
            }
            counter.fetch_add(chunked.len(), Ordering::Relaxed);
        }
        let _ = socket.write_all(b"0\r\n\r\n").await;
        let _ = socket.flush().await;
    });

    (format!("http://{addr}"), written)
}

fn state(base: &str) -> AppState {
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        AnthropicBackend::api_key(
            SecretString::from("sk-ant-test"),
            Some(base.to_string()),
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

const BODY: &str = concat!(
    r#"{"model":"claude-opus-4-6","stream":true,"#,
    r#""system":"You are Claude Code","#,
    r#""messages":[{"role":"user","content":"hi"}]}"#,
);

#[tokio::test]
async fn a_client_that_stops_reading_stops_the_upstream() {
    // The property `docs/PROTOCOL.md` §2 claims. If it did not hold, the
    // upstream would run to completion into IronWire's memory while the client
    // read nothing.
    let (base, written) = spawn_firehose(4_000).await;

    let response = app(state(&base))
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
    assert_eq!(response.status(), StatusCode::OK);

    // Read a little, then stall — a client whose terminal is blocked, or an
    // agent busy running a tool.
    let mut stream = response.into_body().into_data_stream();
    let mut read = 0usize;
    for _ in 0..3 {
        if let Some(Ok(chunk)) = stream.next().await {
            read += chunk.len();
        }
    }
    assert!(read > 0, "the client read nothing at all");

    // Give the upstream a generous window to run away if it can.
    tokio::time::sleep(std::time::Duration::from_millis(750)).await;
    let pushed = written.load(Ordering::Relaxed);

    // Socket buffers mean the upstream gets somewhat ahead; what matters is
    // that it is *bounded* rather than racing to completion.
    let total = 4_000 * 4_200;
    assert!(
        pushed < total / 2,
        "the upstream pushed {pushed} of ~{total} bytes while the client had \
         read {read}; nothing is applying backpressure and the difference is \
         sitting in IronWire's memory"
    );

    drop(stream);
}

#[tokio::test]
async fn a_client_that_reads_everything_gets_everything() {
    // The other half: backpressure must not lose or truncate anything.
    let (base, _written) = spawn_firehose(200).await;

    let response = app(state(&base))
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

    let bytes = axum::body::to_bytes(response.into_body(), 16 << 20)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(
        text.matches("content_block_delta").count(),
        // Each frame names the event twice: the `event:` line and the payload.
        400,
        "frames were lost or duplicated in transit"
    );
}

#[tokio::test]
async fn the_observation_tee_never_becomes_the_bottleneck() {
    // The one component that could turn the forward path into a queue. It reads
    // a copy for usage accounting and, under pressure, must drop rather than
    // block bytes the client is waiting for (`docs/PROTOCOL.md` §2).
    let (base, _written) = spawn_firehose(500).await;
    let state = state(&base);

    let started = std::time::Instant::now();
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
    let bytes = axum::body::to_bytes(response.into_body(), 16 << 20)
        .await
        .expect("body");
    let elapsed = started.elapsed();

    assert!(bytes.len() > 500 * 4_000, "got {} bytes", bytes.len());
    assert!(
        elapsed.as_secs() < 10,
        "2 MB through the proxy took {elapsed:?}; the tee is throttling the \
         forward path"
    );
}

#[tokio::test]
async fn many_slow_clients_at_once_do_not_accumulate() {
    // The shape that turns a bounded per-request cost into an unbounded one:
    // several conversations, all with stalled readers.
    let mut streams = Vec::new();
    let mut writers = Vec::new();

    for _ in 0..8 {
        let (base, written) = spawn_firehose(2_000).await;
        writers.push(written);
        let response = app(state(&base))
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
        // One chunk each, then stall.
        let _ = stream.next().await;
        streams.push(stream);
    }

    tokio::time::sleep(std::time::Duration::from_millis(750)).await;

    let total_pushed: usize = writers.iter().map(|w| w.load(Ordering::Relaxed)).sum();
    let would_be = 8 * 2_000 * 4_200;
    assert!(
        total_pushed < would_be / 2,
        "{total_pushed} bytes across 8 stalled clients out of ~{would_be}; \
         concurrent slow readers accumulate in memory"
    );

    drop(streams);
}
