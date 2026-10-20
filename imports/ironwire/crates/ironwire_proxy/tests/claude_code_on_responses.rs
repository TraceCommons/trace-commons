//! Claude Code running on a Responses backend, end to end.
//!
//! The lane the pivot IR was built for. Before it, `ironwire_translate` mapped
//! Anthropic Messages onto Chat Completions and nothing else — so a Claude Code
//! session that ran out of Anthropic capacity could fall back to NEAR AI or a
//! local server, and **could not reach a ChatGPT subscription or an OpenAI key
//! at all**, which is the capacity most users of this product actually have.
//!
//! Both directions are asserted, because both are load-bearing and they fail
//! differently: a bad request is rejected by the provider and shows up at once,
//! while a bad response is replayed by the client forever.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::Config;
use ironwire_creds::ConsentLedger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::Backend;
use ironwire_upstream::anthropic::AnthropicBackend;
use ironwire_upstream::observe::Observation;
use ironwire_upstream::openai_responses::ResponsesBackend;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// What the Responses stand-in was asked for.
type Received = Arc<Mutex<Option<String>>>;

/// A Responses stream carrying prose and then a tool call, in the framing the
/// real API uses.
const UPSTREAM_SSE: &str = concat!(
    "event: response.created\n",
    r#"data: {"type":"response.created","response":{"id":"resp_1","model":"gpt-5.6"}}"#,
    "\n\n",
    "event: response.output_text.delta\n",
    r#"data: {"type":"response.output_text.delta","output_index":0,"delta":"Running it."}"#,
    "\n\n",
    "event: response.output_item.added\n",
    r#"data: {"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"call_9","name":"Bash"}}"#,
    "\n\n",
    "event: response.function_call_arguments.done\n",
    r#"data: {"type":"response.function_call_arguments.done","output_index":1,"arguments":"{\"command\":\"cargo test\"}"}"#,
    "\n\n",
    "event: response.completed\n",
    r#"data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","usage":{"input_tokens":900,"input_tokens_details":{"cached_tokens":100},"output_tokens":12}}}"#,
    "\n\n",
);

async fn spawn_responses() -> (String, Received) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let received: Received = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&received);

    tokio::spawn(async move {
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
                    let line = head.lines().next().unwrap_or_default().to_string();
                    let body = String::from_utf8_lossy(&buf[split..split + length]).to_string();
                    *sink.lock().expect("lock") = Some(format!("{line}\n{body}"));
                    break;
                }
            }
        }
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n",
            UPSTREAM_SSE.len()
        );
        let _ = socket.write_all(head.as_bytes()).await;
        let _ = socket.write_all(UPSTREAM_SSE.as_bytes()).await;
        let _ = socket.flush().await;
    });

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

/// Anthropic rate limited, an OpenAI key available. The situation IronWire
/// exists for, on the pair it could not previously serve.
fn state_with_exhausted_anthropic(responses_base: &str) -> AppState {
    let mut registry = BackendRegistry::new();

    let anthropic = Arc::new(
        AnthropicBackend::api_key(
            SecretString::from("sk-ant-test"),
            Some("http://127.0.0.1:1".to_string()),
            30,
        )
        .expect("client builds"),
    );
    // Present and healthy, but out of window. It stays registered so the router
    // has to *choose* the foreign wire rather than being handed it.
    anthropic.record(&Observation {
        retry_after_secs: Some(3600),
        ..Observation::default()
    });
    registry.push(anthropic);

    registry.push(Arc::new(
        ResponsesBackend::openai_api_key(
            SecretString::from("sk-openai-test"),
            Some(responses_base.to_string()),
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

/// A Claude Code turn: system preamble with a cache breakpoint, tools, thinking
/// on, and a signed block replayed from an earlier turn.
fn claude_code_body() -> Value {
    json!({
        "model": "claude-opus-4-6",
        "max_tokens": 8192,
        "stream": true,
        "system": [
            {"type": "text", "text": "You are Claude Code.",
             "cache_control": {"type": "ephemeral"}}
        ],
        "thinking": {"type": "enabled", "budget_tokens": 4096},
        "tools": [{
            "name": "Bash",
            "description": "run a command",
            "input_schema": {"type": "object", "properties": {"command": {"type": "string"}}}
        }],
        "messages": [
            {"role": "user", "content": "fix the failing test"},
            {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "let me look", "signature": "SIGNED-BY-ANTHROPIC"},
                {"type": "tool_use", "id": "toolu_01ABC", "name": "Bash",
                 "input": {"command": "cargo build"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_01ABC", "content": "it built"}
            ]},
            {"role": "user", "content": "now run the tests"}
        ]
    })
}

async fn post(state: AppState, body: &Value) -> (StatusCode, String) {
    let request = Request::builder()
        .method("POST")
        .uri("/anthropic/v1/messages")
        .header("content-type", "application/json")
        .header("x-api-key", "the-clients-own-key")
        .header("anthropic-version", "2023-06-01")
        .body(Body::from(serde_json::to_vec(body).expect("serialises")))
        .expect("request builds");
    let response = app(state)
        .oneshot(request)
        .await
        .expect("the proxy answers");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// Parse an SSE stream into `(event, data)` pairs.
fn events(sse: &str) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for frame in sse.split("\n\n") {
        let mut name = String::new();
        let mut data = String::new();
        for line in frame.lines() {
            if let Some(rest) = line.strip_prefix("event:") {
                name = rest.trim().to_string();
            } else if let Some(rest) = line.strip_prefix("data:") {
                data.push_str(rest.trim_start());
            }
        }
        if let Ok(value) = serde_json::from_str::<Value>(&data) {
            out.push((name, value));
        }
    }
    out
}

#[tokio::test]
async fn a_claude_code_turn_reaches_a_responses_backend_in_its_own_shape() {
    let (base, received) = spawn_responses().await;
    let (status, _) = post(state_with_exhausted_anthropic(&base), &claude_code_body()).await;
    assert_eq!(status, StatusCode::OK);

    let seen = received
        .lock()
        .expect("lock")
        .clone()
        .expect("the upstream saw a request");
    let (line, body) = seen.split_once('\n').expect("a request line and a body");
    assert_eq!(line, "POST /v1/responses HTTP/1.1");

    let sent: Value = serde_json::from_str(body).expect("valid JSON");
    // Responses shape, not Anthropic's and not Chat Completions'.
    assert!(sent.get("messages").is_none(), "{sent}");
    assert_eq!(sent["instructions"], "You are Claude Code.");
    assert_eq!(sent["max_output_tokens"], 8192);
    // Tools are flat here, not nested under `function`.
    assert_eq!(sent["tools"][0]["name"], "Bash");
    assert_eq!(sent["tools"][0]["type"], "function");

    let input = sent["input"].as_array().expect("input items");
    assert_eq!(input[0]["content"][0]["type"], "input_text");
    assert_eq!(input[0]["content"][0]["text"], "fix the failing test");
    assert_eq!(input[1]["type"], "function_call");
    assert_eq!(input[1]["name"], "Bash");
    assert_eq!(input[2]["type"], "function_call_output");
    assert_eq!(input[2]["output"], "it built");
    // The call and its result must still pair, or the provider rejects the run.
    assert_eq!(input[1]["call_id"], input[2]["call_id"]);

    // A signature Anthropic issued is meaningless to OpenAI and must not travel
    // (`docs/PROTOCOL.md` §6).
    assert!(
        !body.contains("SIGNED-BY-ANTHROPIC"),
        "an Anthropic signature reached OpenAI: {body}"
    );
}

#[tokio::test]
async fn the_answer_comes_back_as_a_stream_claude_code_can_read() {
    let (base, _) = spawn_responses().await;
    let (status, sse) = post(state_with_exhausted_anthropic(&base), &claude_code_body()).await;
    assert_eq!(status, StatusCode::OK);

    let frames = events(&sse);
    let names: Vec<&str> = frames.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names.first(), Some(&"message_start"), "{sse}");
    assert_eq!(names.last(), Some(&"message_stop"), "{sse}");

    // The model the client asked for, never the one that served it.
    let (_, start) = &frames[0];
    assert_eq!(start["message"]["model"], "claude-opus-4-6");

    let text: String = frames
        .iter()
        .filter(|(name, _)| name == "content_block_delta")
        .filter_map(|(_, value)| value.pointer("/delta/text").and_then(Value::as_str))
        .collect();
    assert_eq!(text, "Running it.");

    // The tool call arrives whole, with parsed input and an id valid in
    // Anthropic's namespace.
    let (_, call) = frames
        .iter()
        .find(|(name, value)| {
            name == "content_block_start"
                && value.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use")
        })
        .expect("a tool_use block");
    assert_eq!(call["content_block"]["name"], "Bash");
    assert_eq!(call["content_block"]["input"]["command"], "cargo test");
    let id = call["content_block"]["id"].as_str().expect("an id");
    assert!(id.starts_with("toolu_"), "{id} is not valid for Anthropic");

    // The field that decides whether the agent runs the call or stops.
    let (_, delta) = frames
        .iter()
        .find(|(name, _)| name == "message_delta")
        .expect("a message_delta");
    assert_eq!(delta["delta"]["stop_reason"], "tool_use");
    // Usage survives, with the cached portion kept separate.
    assert_eq!(delta["usage"]["input_tokens"], 800);
    assert_eq!(delta["usage"]["cache_read_input_tokens"], 100);
    assert_eq!(delta["usage"]["output_tokens"], 12);
}

/// The id IronWire minted has to survive the client replaying it: the next turn
/// carries `toolu_xw_call_9`, and the provider must see `call_9` again.
#[tokio::test]
async fn a_minted_tool_id_reverses_when_the_client_replays_it() {
    let (base, received) = spawn_responses().await;
    let mut body = claude_code_body();
    body["messages"] = json!([
        {"role": "user", "content": "go"},
        {"role": "assistant", "content": [
            {"type": "tool_use", "id": "toolu_xw_call_9", "name": "Bash", "input": {}}
        ]},
        {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_xw_call_9", "content": "done"}
        ]},
        // A fresh user turn, so this is a turn boundary rather than mid tool
        // loop — families change at a boundary and nowhere else
        // (`docs/PROTOCOL.md` §6), and without this the route is correctly
        // refused and the test would be measuring the wrong rule.
        {"role": "user", "content": "carry on"}
    ]);
    let (status, _) = post(state_with_exhausted_anthropic(&base), &body).await;
    assert_eq!(status, StatusCode::OK);

    let seen = received.lock().expect("lock").clone().expect("a request");
    let (_, sent) = seen.split_once('\n').expect("a body");
    let sent: Value = serde_json::from_str(sent).expect("valid JSON");
    let input = sent["input"].as_array().expect("input items");
    let call = input
        .iter()
        .find(|item| item["type"] == "function_call")
        .expect("the call");
    assert_eq!(
        call["call_id"], "call_9",
        "the id IronWire minted was not reversed on the way back out"
    );
}
