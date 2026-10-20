//! Codex running on NEAR AI, end to end.
//!
//! The failure this test exists for, from a real daemon log:
//!
//! ```text
//! WARN no route for request error=no route: AllIneligible {
//!   reasons: [(BackendId("nearai"), NoTranslationPath)] }
//! ```
//!
//! Codex speaks the Responses API. NEAR AI answers `/v1/responses` and
//! `/v1/chat/completions` at the same base URL, but IronWire modelled it as
//! Chat Completions and nothing else — so the router asked "can a Responses
//! request be translated into Chat Completions", found no translator (there
//! isn't one, and there does not need to be), and refused a request that had a
//! native lane available the whole time.
//!
//! The claim here is the strong one: **byte-identical**. Nothing is translated,
//! because nothing needs to be. If this ever starts passing through
//! `ironwire_translate`, the body will stop matching and this test will say so.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::Config;
use ironwire_core::protocol::ModelTier;
use ironwire_creds::ConsentLedger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::openai_chat::ChatCompletionsBackend;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// What the NEAR AI stand-in was asked for.
#[derive(Debug, Default, Clone)]
struct Received {
    request_line: String,
    body: String,
}

/// A Responses-API stream, in the framing an OpenAI-compatible endpoint sends.
const UPSTREAM_SSE: &str = concat!(
    "event: response.created\n",
    r#"data: {"type":"response.created","response":{"id":"resp_1","model":"qwen3-coder"}}"#,
    "\n\n",
    "event: response.output_text.delta\n",
    r#"data: {"type":"response.output_text.delta","delta":"hi"}"#,
    "\n\n",
    "event: response.completed\n",
    r#"data: {"type":"response.completed","response":{"id":"resp_1","usage":{"input_tokens":11,"output_tokens":3}}}"#,
    "\n\n",
);

/// A Codex-shaped Responses request, including a field this build has never
/// heard of — the native lane forwards bytes, so it must survive untouched.
const CODEX_BODY: &str = concat!(
    r#"{"model":"gpt-5.6","stream":true,"#,
    r#""instructions":"You are Codex, based on GPT-5.","#,
    r#""tools":[{"type":"function","name":"shell","parameters":{"type":"object"}}],"#,
    r#""input":[{"type":"reasoning","id":"rs_1","encrypted_content":"gAAAAA"},"#,
    r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}],"#,
    r#""a_field_from_the_future":{"nested":[1,2,3]},"#,
    r#""reasoning":{"effort":"high","summary":"auto"}}"#,
);

fn plain_first_turn() -> String {
    let mut body: serde_json::Value = serde_json::from_str(CODEX_BODY).expect("fixture JSON");
    body["input"].as_array_mut().expect("input").remove(0);
    body.to_string()
}

async fn spawn_nearai() -> (String, Arc<Mutex<Option<Received>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let received = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&received);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let Ok(n) = socket.read(&mut chunk).await else {
                    return;
                };
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(split) = find_head_end(&buf) {
                    let head = String::from_utf8_lossy(&buf[..split]).to_string();
                    let length = content_length(&head).unwrap_or(0);
                    if buf.len() - split >= length {
                        *sink.lock().expect("lock") = Some(Received {
                            request_line: head.lines().next().unwrap_or_default().to_string(),
                            body: String::from_utf8_lossy(&buf[split..split + length]).to_string(),
                        });
                        break;
                    }
                }
            }

            let head = format!(
                "HTTP/1.1 200 OK\r\n\
             content-type: text/event-stream\r\n\
             connection: close\r\n\
             content-length: {}\r\n\r\n",
                UPSTREAM_SSE.len()
            );
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(UPSTREAM_SSE.as_bytes()).await;
            let _ = socket.flush().await;
        }
    });

    // `/v1` included, because that is the shape of the real base URL
    // (`NEARAI_DEFAULT_BASE_URL`) and `endpoint_url` composes against it: the
    // client's `/v1` prefix is stripped and the base's is kept. A mock rooted
    // at `/` would silently pass while production went somewhere else.
    (format!("http://{addr}/v1"), received)
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

fn content_length(head: &str) -> Option<usize> {
    head.lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.trim().parse().ok())
}

/// NEAR AI alone, which is the machine the bug was reported from: both
/// subscriptions present and unconsented, so the credits backend is all there
/// is. Registering only it makes the same point without the noise.
fn state_for(base_url: &str) -> AppState {
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        ChatCompletionsBackend::nearai(
            Some(SecretString::from("near-key")),
            Some(base_url.to_string()),
            vec![("qwen3-coder".to_string(), ModelTier::Frontier)],
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

fn codex_request(body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/openai/v1/responses")
        .header("content-type", "application/json")
        .header("authorization", "Bearer sk-the-clients-own-key")
        .header("originator", "codex_cli_rs")
        .header("session-id", "nearai-native-session")
        .body(Body::from(body.to_string()))
        .expect("request builds")
}

#[tokio::test]
async fn a_codex_request_reaches_near_ai_on_its_own_wire() {
    let (base, received) = spawn_nearai().await;
    let state = state_for(&base);
    let first_body = plain_first_turn();
    let first = app(state.clone())
        .oneshot(codex_request(&first_body))
        .await
        .expect("first turn");
    assert_eq!(first.status(), StatusCode::OK);
    axum::body::to_bytes(first.into_body(), 1 << 20)
        .await
        .expect("first stream");
    assert_eq!(
        received
            .lock()
            .expect("lock")
            .as_ref()
            .expect("first request")
            .body,
        first_body
    );
    let response = app(state)
        .oneshot(codex_request(CODEX_BODY))
        .await
        .expect("the proxy answers");

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "NEAR AI serves the Responses API; this used to be refused as untranslatable"
    );

    let seen = received
        .lock()
        .expect("lock")
        .clone()
        .expect("upstream saw a request");
    assert_eq!(
        seen.request_line, "POST /v1/responses HTTP/1.1",
        "the request went to the wrong endpoint"
    );
    // The whole point of a native lane: not re-serialised, not re-shaped, not
    // stripped of the field this build does not recognise.
    assert_eq!(seen.body, CODEX_BODY, "the body was not forwarded verbatim");
}

/// A backend that really does speak only Chat Completions is now reached by
/// translation rather than refused.
///
/// This used to be a refusal, and the refusal was correct at the time: there
/// was no Responses → Chat Completions mapping. There is one now
/// (`docs/TRANSLATION.md`), so a local server becomes real fallback capacity
/// for Codex — which is the whole point of the matrix.
///
/// The assertion is that the body actually changed shape. Forwarding a
/// Responses body to `/v1/chat/completions` and calling it translated would
/// pass a status check and fail at the provider.
#[tokio::test]
async fn a_chat_only_backend_receives_a_translated_request() {
    let (base, received) = spawn_nearai().await;
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        ChatCompletionsBackend::local(
            ironwire_core::protocol::BackendId::from("ollama"),
            "Ollama",
            base,
            None,
            vec![("qwen3".to_string(), ModelTier::Balanced)],
            30,
        )
        .expect("client builds"),
    ));
    let state = AppState::new(
        registry,
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    );

    let response = app(state)
        .oneshot(codex_request(CODEX_BODY))
        .await
        .expect("the proxy answers");
    assert_eq!(response.status(), StatusCode::OK);

    let seen = received
        .lock()
        .expect("lock")
        .clone()
        .expect("upstream saw a request");
    assert_eq!(seen.request_line, "POST /v1/chat/completions HTTP/1.1");
    assert_ne!(seen.body, CODEX_BODY, "the body was forwarded untranslated");

    let sent: serde_json::Value = serde_json::from_str(&seen.body).expect("valid JSON");
    // Chat Completions shape: messages, not `input`; the instructions became a
    // system message; the tool moved under a `function` key.
    assert!(sent.get("input").is_none(), "{sent}");
    assert_eq!(sent["messages"][0]["role"], "system");
    assert_eq!(sent["messages"][1]["content"], "hi");
    assert_eq!(sent["tools"][0]["function"]["name"], "shell");
    // The encrypted reasoning item is not replayable off its own provider.
    assert!(!seen.body.contains("gAAAAA"), "{}", seen.body);
}
