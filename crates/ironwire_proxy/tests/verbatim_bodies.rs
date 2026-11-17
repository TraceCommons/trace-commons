//! The ledger records the bodies a receipt is actually about.
//!
//! NEAR AI signs `<sha256 of the request body as sent>:<sha256 of the response
//! body as received>` and serves it at `GET /v1/signature/{id}`, keyed by the
//! `id` the ledger already stores as `upstream_id`. So the two digests have to
//! be taken over the exact bytes that crossed the wire. Anything that parses
//! and re-emits a body -- reordering keys, re-escaping a character,
//! normalising whitespace, round-tripping a float -- produces a digest that
//! fails against a perfectly good receipt, and a receipt that fails reads as
//! tampering rather than as our bug.
//!
//! Every body here is therefore one a re-serialiser demonstrably changes, and
//! `a_reserialised_body_would_not_match` proves the fixture has that property
//! rather than assuming it.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::Config;
use ironwire_creds::ConsentLedger;
use ironwire_ledger::Ledger;
use ironwire_ledger::bodies::{BodyStore, sha256_hex};
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// A request body no re-serialiser leaves alone: `model` after `messages`,
/// two spaces after a colon, a non-ASCII character, an escape `serde_json`
/// would re-emit as the character itself, and a float whose shortest
/// representation is shorter than what is written.
const AWKWARD_REQUEST: &str = concat!(
    "{\"messages\":[{\"role\":\"user\",\"content\":\"caf\u{e9} \\u00e9 \u{2014} d\u{e9}j\u{e0}\"}],",
    "  \"max_tokens\":64,\"model\":\"claude-opus-4-6\",",
    "\"temperature\":0.1000000000000000055511151231257827,\"metadata\":{}}"
);

/// The same treatment on the way back: `usage` before `content`, an escape
/// that would be rewritten, and trailing whitespace inside the document.
const AWKWARD_RESPONSE: &str = concat!(
    "{\"usage\":{\"input_tokens\":10,\"output_tokens\":2},\"id\":\"msg_receipt_1\",",
    "\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-opus-4-6\",",
    "\"content\":[{\"type\":\"text\",\"text\":\"\\u00e9t\u{e9}\"}] ,\"stop_reason\":\"end_turn\"}"
);

/// An upstream that records the exact bytes it was sent and replies with
/// exactly `response`.
async fn upstream(response: &'static str) -> (String, Arc<Mutex<Vec<u8>>>) {
    upstream_as("application/json", response).await
}

/// The same, for a response the provider serves as an event stream.
async fn upstream_as(
    content_type: &'static str,
    response: &'static str,
) -> (String, Arc<Mutex<Vec<u8>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut raw = Vec::new();
        let mut chunk = [0u8; 4096];
        // Read until the declared body has arrived: a single `read` returns
        // whatever one segment carried, which would silently truncate the very
        // bytes this test exists to compare.
        loop {
            let Ok(read) = socket.read(&mut chunk).await else {
                return;
            };
            if read == 0 {
                break;
            }
            raw.extend_from_slice(&chunk[..read]);
            if let Some(start) = find(&raw, b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&raw[..start]).to_ascii_lowercase();
                let length: usize = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                if raw.len() >= start + 4 + length {
                    *sink.lock().expect("lock") = raw[start + 4..start + 4 + length].to_vec();
                    break;
                }
            }
        }
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n\r\n",
            response.len()
        );
        let _ = socket.write_all(head.as_bytes()).await;
        let _ = socket.write_all(response.as_bytes()).await;
        let _ = socket.flush().await;
    });
    (format!("http://{addr}"), seen)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn state_for(base_url: &str, ledger: Ledger, bodies: Option<Arc<BodyStore>>) -> AppState {
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
    .with_bodies(bodies)
}

/// One exchange through the façade. Returns the row, the bytes the upstream
/// actually received, and the store the bodies went to.
async fn exchange(
    capture_bodies: bool,
) -> (
    ironwire_ledger::Exchange,
    Vec<u8>,
    Option<(Arc<BodyStore>, tempfile::TempDir)>,
) {
    let (base, seen) = upstream(AWKWARD_RESPONSE).await;
    let ledger = Ledger::in_memory().expect("ledger opens");
    let store = capture_bodies.then(|| {
        let home = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(BodyStore::open(&home.path().join("bodies")).expect("store opens"));
        (store, home)
    });
    let state = state_for(
        &base,
        ledger.clone(),
        store.as_ref().map(|(store, _)| Arc::clone(store)),
    );

    let request = Request::builder()
        .method("POST")
        .uri("/anthropic/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(AWKWARD_REQUEST))
        .expect("request builds");
    let response = app(state).oneshot(request).await.expect("served");
    assert_eq!(response.status(), StatusCode::OK);
    // The ledger write happens when the response body is consumed.
    let _ = axum::body::to_bytes(response.into_body(), usize::MAX).await;

    let rows = ledger.recent(1).expect("read the ledger");
    assert_eq!(rows.len(), 1, "the exchange was recorded");
    let sent = seen.lock().expect("lock").clone();
    (rows[0].clone(), sent, store)
}

/// The fixture has to be one a re-serialiser changes, or every other case here
/// would pass with the bug present.
#[test]
fn a_reserialised_body_would_not_match() {
    for body in [AWKWARD_REQUEST, AWKWARD_RESPONSE] {
        let value: serde_json::Value = serde_json::from_str(body).expect("valid JSON");
        let round_tripped = serde_json::to_string(&value).expect("serialises");
        assert_ne!(round_tripped, body, "fixture survives a round trip: {body}");
        assert_ne!(
            sha256_hex(round_tripped.as_bytes()),
            sha256_hex(body.as_bytes())
        );
    }
}

#[tokio::test]
async fn the_request_digest_is_over_the_bytes_the_upstream_received() {
    let (row, sent, _store) = exchange(true).await;
    assert_eq!(
        sent,
        AWKWARD_REQUEST.as_bytes(),
        "the native lane forwards bytes, so the upstream got what the client sent"
    );
    assert_eq!(
        row.request_sha256.as_deref(),
        Some(sha256_hex(&sent).as_str()),
        "the recorded digest is of the bytes that crossed the wire"
    );
}

#[tokio::test]
async fn the_response_digest_is_over_the_bytes_the_upstream_returned() {
    let (row, _sent, _store) = exchange(true).await;
    assert_eq!(
        row.response_sha256.as_deref(),
        Some(sha256_hex(AWKWARD_RESPONSE.as_bytes()).as_str())
    );
}

#[tokio::test]
async fn the_stored_bodies_are_byte_for_byte_what_crossed_the_wire() {
    let (row, sent, store) = exchange(true).await;
    let (store, _home) = store.expect("capture was on");
    let reference = row.body_ref.expect("a body reference was recorded");
    let (request, response) = store.read(&reference).expect("bodies read back");
    assert_eq!(request, sent);
    assert_eq!(request, AWKWARD_REQUEST.as_bytes());
    assert_eq!(response, AWKWARD_RESPONSE.as_bytes());
}

/// The receipt is fetched by the provider's own id, and the row already
/// carries it. Both halves on one row is the whole point: an id with no
/// digests cannot be checked, and digests with no id cannot be looked up.
#[tokio::test]
async fn the_row_carries_the_id_the_receipt_is_fetched_by() {
    let (row, _sent, _store) = exchange(true).await;
    assert_eq!(row.upstream_id.as_deref(), Some("msg_receipt_1"));
    assert!(row.request_sha256.is_some() && row.response_sha256.is_some());
}

/// Bodies are the user's source code. Off is off -- no digests either, because
/// a digest is derived from a body we would have had to hold.
#[tokio::test]
async fn nothing_is_captured_when_the_setting_is_off() {
    let (row, _sent, store) = exchange(false).await;
    assert!(store.is_none());
    assert_eq!(row.request_sha256, None);
    assert_eq!(row.response_sha256, None);
    assert_eq!(row.body_ref, None);
    assert_eq!(
        row.upstream_id.as_deref(),
        Some("msg_receipt_1"),
        "the id is metadata and is recorded either way"
    );
}

/// A streamed response is a chunk stream, not a JSON document, and NEAR AI's
/// own verifier hashes the raw concatenated stream text -- not any reassembled
/// content. Streaming is the common case for a coding agent, so a capture that
/// only worked for whole documents would be a capture that never fired.
///
/// The frames here are deliberately not uniform: two spaces after `data:` on
/// one frame, a `\u00e9` escape, and a comment line. Reassembling and
/// re-emitting this stream changes every one of them.
const AWKWARD_STREAM: &str = concat!(
    ": keep-alive\n\n",
    "event: message_start\n",
    "data:  {\"type\":\"message_start\",\"message\":{\"id\":\"msg_stream_1\",\"model\":\"claude-opus-4-6\",\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"\\u00e9t\u{e9}\"}}\n\n",
    "event: message_delta\n",
    "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":2}}\n\n",
    "event: message_stop\n",
    "data: {\"type\":\"message_stop\"}\n\n",
);

#[tokio::test]
async fn a_streamed_response_is_digested_as_the_raw_stream_it_arrived_as() {
    let (base, _seen) = upstream_as("text/event-stream", AWKWARD_STREAM).await;
    let ledger = Ledger::in_memory().expect("ledger opens");
    let home = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(BodyStore::open(&home.path().join("bodies")).expect("store opens"));
    let state = state_for(&base, ledger.clone(), Some(Arc::clone(&store)));

    let body = AWKWARD_REQUEST.replace("\"metadata\":{}", "\"stream\":true,\"metadata\":{}");
    let request = Request::builder()
        .method("POST")
        .uri("/anthropic/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("request builds");
    let response = app(state).oneshot(request).await.expect("served");
    assert_eq!(response.status(), StatusCode::OK);
    let delivered = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");

    let rows = ledger.recent(1).expect("read the ledger");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].response_sha256.as_deref(),
        Some(sha256_hex(AWKWARD_STREAM.as_bytes()).as_str()),
        "the digest is of the raw stream, not of anything reassembled"
    );
    assert_eq!(
        delivered.as_ref(),
        AWKWARD_STREAM.as_bytes(),
        "and the client got those same bytes"
    );
    let reference = rows[0].body_ref.clone().expect("a body reference");
    let (_, stored) = store.read(&reference).expect("bodies read back");
    assert_eq!(stored, AWKWARD_STREAM.as_bytes());
    assert_eq!(
        rows[0].upstream_id.as_deref(),
        Some("msg_stream_1"),
        "the receipt id comes off the opening frame"
    );
}

/// A response the client abandoned has no honest digest: we never saw it whole.
/// Recording one would put a hash on the row that no receipt can match, which
/// is indistinguishable from tampering.
#[tokio::test]
async fn an_unfinished_response_records_no_digest() {
    use futures_util::stream;
    use ironwire_upstream::backend::UpstreamError;

    let capture = ironwire_proxy::pipeline::Capture::of_request(bytes::Bytes::from_static(b"req"));
    let failing = stream::iter(vec![
        Ok(bytes::Bytes::from_static(b"half a ")),
        Err(UpstreamError::Transport {
            backend: ironwire_core::protocol::BackendId::from("anthropic"),
            detail: "connection reset".into(),
        }),
    ]);
    let mut streamed = Box::pin(ironwire_proxy::pipeline::capture_stream(failing, &capture));
    use futures_util::StreamExt;
    while streamed.next().await.is_some() {}
    drop(streamed);
    assert_eq!(capture.response(), None);
}

/// Chat Completions SSE, as NEAR AI serves it. The `id` here is the `chat_id`
/// its `GET /v1/signature/{chat_id}` takes.
const NEARAI_STREAM: &str = concat!(
    "data: {\"id\":\"chatcmpl-receipt-1\",\"model\":\"qwen3.6-27b\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\\u00e9t\u{e9}\"}}]}\n\n",
    "data: {\"id\":\"chatcmpl-receipt-1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    "data: {\"id\":\"chatcmpl-receipt-1\",\"choices\":[],\"usage\":{\"prompt_tokens\":480,\"completion_tokens\":12}}\n\n",
    "data: [DONE]\n\n",
);

/// The case the receipts exist for, and the one where capturing the *client's*
/// bytes would be silently wrong.
///
/// A Claude Code request served by NEAR AI is translated in both directions:
/// the body the enclave hashed is the Chat Completions document IronWire
/// built, and the response it hashed is its own Chat Completions stream --
/// neither of which the client ever sees. Capturing at the façade instead of
/// at the upstream boundary would put two plausible digests on the row that no
/// receipt can ever match.
#[tokio::test]
async fn a_translated_route_captures_what_the_provider_hashed_not_what_the_client_saw() {
    use ironwire_core::protocol::ModelTier;
    use ironwire_upstream::openai_chat::ChatCompletionsBackend;

    let (base, seen) = upstream_as("text/event-stream", NEARAI_STREAM).await;
    let ledger = Ledger::in_memory().expect("ledger opens");
    let home = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(BodyStore::open(&home.path().join("bodies")).expect("store opens"));

    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        ChatCompletionsBackend::nearai(
            Some(SecretString::from("near-key")),
            Some(base),
            vec![("qwen3.6-27b".to_string(), ModelTier::Balanced)],
            30,
        )
        .expect("client builds"),
    ));
    let state = AppState::new(
        registry,
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    )
    .with_ledger(Some(ledger.clone()))
    .with_bodies(Some(Arc::clone(&store)));

    let body = AWKWARD_REQUEST.replace("\"metadata\":{}", "\"stream\":true,\"metadata\":{}");
    let request = Request::builder()
        .method("POST")
        .uri("/anthropic/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(body.clone()))
        .expect("request builds");
    let response = app(state).oneshot(request).await.expect("served");
    assert_eq!(response.status(), StatusCode::OK);
    let delivered = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");

    let sent = seen.lock().expect("lock").clone();
    assert_ne!(
        sent,
        body.as_bytes(),
        "this lane translates, so the upstream body is not the client's"
    );
    let rows = ledger.recent(1).expect("read the ledger");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].request_sha256.as_deref(),
        Some(sha256_hex(&sent).as_str()),
        "the digest is of the document the provider hashed"
    );
    assert_ne!(
        rows[0].request_sha256.as_deref(),
        Some(sha256_hex(body.as_bytes()).as_str()),
        "and not of the one the client sent"
    );

    assert_ne!(
        delivered.as_ref(),
        NEARAI_STREAM.as_bytes(),
        "the client was served Anthropic frames, not the provider's"
    );
    assert_eq!(
        rows[0].response_sha256.as_deref(),
        Some(sha256_hex(NEARAI_STREAM.as_bytes()).as_str()),
        "the response digest is of the provider's own stream"
    );
    let reference = rows[0].body_ref.clone().expect("a body reference");
    let (request, response) = store.read(&reference).expect("bodies read back");
    assert_eq!(request, sent);
    assert_eq!(response, NEARAI_STREAM.as_bytes());
    assert_eq!(
        rows[0].upstream_id.as_deref(),
        Some("chatcmpl-receipt-1"),
        "the chat_id the receipt is fetched by"
    );
}
