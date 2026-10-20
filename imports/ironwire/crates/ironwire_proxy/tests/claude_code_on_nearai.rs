//! Claude Code running on NEAR AI inference, end to end.
//!
//! This is the test the translated lane exists for. A Claude Code session hits
//! its Anthropic rate limit; IronWire falls back across the API family boundary
//! to NEAR AI; the agent keeps working and never learns it changed providers.
//!
//! It also pins the rule that makes the switch safe (`docs/PROTOCOL.md` §6):
//! **families change at a turn boundary, never mid tool loop.** A conversation
//! caught mid-loop waits for the next clean turn rather than being refused for
//! its lifetime — which is the correction that motivated this whole lane.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration, Utc};
use ironwire_core::config::Config;
use ironwire_core::protocol::ModelTier;
use ironwire_core::quota::Headroom;
use ironwire_creds::ConsentLedger;
use ironwire_ledger::Ledger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::Backend;
use ironwire_upstream::anthropic::AnthropicBackend;
use ironwire_upstream::observe::Observation;
use ironwire_upstream::openai_chat::ChatCompletionsBackend;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// What the NEAR AI stand-in received, so the test can assert the translation.
type Received = Arc<Mutex<Vec<Value>>>;

/// A NEAR-AI-shaped endpoint: OpenAI Chat Completions, streaming SSE.
///
/// It records every request body and replies with the canned turn it is given,
/// so the assertions are about *our* translation rather than a model's whims.
async fn spawn_nearai(turns: Vec<String>) -> (String, Received) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);

    tokio::spawn(async move {
        for turn in turns {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
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
                let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
                else {
                    continue;
                };
                let head = String::from_utf8_lossy(&buf[..split]).to_string();
                let length = head
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                    .and_then(|l| l.split(':').nth(1))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if buf.len() - split >= length {
                    let body = String::from_utf8_lossy(&buf[split..split + length]).to_string();
                    sink.lock()
                        .expect("lock")
                        .push(serde_json::from_str(&body).unwrap_or(Value::Null));
                    break;
                }
            }

            let head = "HTTP/1.1 200 OK\r\n\
                        content-type: text/event-stream\r\n\
                        transfer-encoding: chunked\r\n\r\n";
            let _ = socket.write_all(head.as_bytes()).await;
            let framed = format!("{:x}\r\n{turn}\r\n0\r\n\r\n", turn.len());
            let _ = socket.write_all(framed.as_bytes()).await;
            let _ = socket.flush().await;
        }
    });

    (format!("http://{addr}"), received)
}

/// Chat Completions SSE for a plain text answer.
fn text_turn(text: &str) -> String {
    format!(
        concat!(
            "data: {}\n\n",
            "data: {}\n\n",
            "data: {}\n\n",
            "data: [DONE]\n\n",
        ),
        json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": text}}]}),
        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
        json!({"choices": [], "usage": {"prompt_tokens": 480, "completion_tokens": 12}}),
    )
}

/// Chat Completions SSE for a turn that calls a tool, arguments fragmented the
/// way a real provider streams them.
fn tool_call_turn(name: &str, arguments: &str) -> String {
    let (head, tail) = arguments.split_at(arguments.len() / 2);
    format!(
        concat!(
            "data: {}\n\n",
            "data: {}\n\n",
            "data: {}\n\n",
            "data: {}\n\n",
            "data: [DONE]\n\n",
        ),
        json!({"choices": [{"index": 0, "delta": {"tool_calls": [
            {"index": 0, "id": "call_near_1", "type": "function",
             "function": {"name": name, "arguments": ""}}
        ]}}]}),
        json!({"choices": [{"index": 0, "delta": {"tool_calls": [
            {"index": 0, "function": {"arguments": head}}
        ]}}]}),
        json!({"choices": [{"index": 0, "delta": {"tool_calls": [
            {"index": 0, "function": {"arguments": tail}}
        ]}}]}),
        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
    )
}

/// A minimal Anthropic-shaped upstream, so "the subscription recovered" can be
/// tested as a real request rather than only as a routing decision.
async fn spawn_anthropic() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut scratch = [0u8; 8192];
            let _ = socket.read(&mut scratch).await;
            let sse = concat!(
                "event: message_start\n",
                r#"data: {"type":"message_start","message":{"model":"claude-opus-4-6","usage":{"input_tokens":5}}}"#,
                "\n\n",
                "event: message_stop\n",
                r#"data: {"type":"message_stop"}"#,
                "\n\n",
            );
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n",
                sse.len()
            );
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(sse.as_bytes()).await;
            let _ = socket.flush().await;
        }
    });
    format!("http://{addr}")
}

/// Claude Max exhausted, NEAR AI available — the situation IronWire exists for.
fn state_with_exhausted_claude(nearai_base: &str) -> AppState {
    state_with(nearai_base, "http://127.0.0.1:1")
}

fn state_with(nearai_base: &str, anthropic_base: &str) -> AppState {
    let mut registry = BackendRegistry::new();

    // The subscription is present and healthy, but rate limited. It stays in
    // the registry so the router has to *choose* NEAR AI rather than being
    // handed it by construction.
    let claude = Arc::new(
        AnthropicBackend::api_key(
            SecretString::from("sk-ant-test"),
            Some(anthropic_base.to_string()),
            30,
        )
        .expect("client builds"),
    );
    claude.record(&Observation {
        retry_after_secs: Some(3600),
        ..Observation::default()
    });
    registry.push(claude);

    registry.push(Arc::new(
        ChatCompletionsBackend::nearai(
            Some(SecretString::from("near-key")),
            Some(nearai_base.to_string()),
            vec![("near-x".to_string(), ModelTier::Frontier)],
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
    .with_ledger(Some(Ledger::in_memory().expect("ledger opens")))
}

/// A Claude Code request: its system preamble, tools, thinking enabled, and a
/// cache breakpoint — everything the translation has to cope with.
fn claude_code_request(messages: Value) -> Value {
    json!({
        "model": "claude-opus-4-6",
        "max_tokens": 8192,
        "stream": true,
        "system": [{
            "type": "text",
            "text": "You are Claude Code, Anthropic's official CLI for Claude.",
            "cache_control": {"type": "ephemeral"}
        }],
        "thinking": {"type": "enabled", "budget_tokens": 4096},
        "tools": [{
            "name": "Bash",
            "description": "run a shell command",
            "input_schema": {
                "type": "object",
                "properties": {"command": {"type": "string"}},
                "required": ["command"]
            }
        }],
        "messages": messages,
    })
}

async fn post(state: &AppState, body: &Value) -> (StatusCode, Option<String>, String) {
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/anthropic/v1/messages")
                .header("content-type", "application/json")
                .header("anthropic-version", "2023-06-01")
                .body(Body::from(serde_json::to_vec(body).expect("serialises")))
                .expect("request builds"),
        )
        .await
        .expect("served");
    let status = response.status();
    let backend = response
        .headers()
        .get("x-ironwire-backend")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    (status, backend, String::from_utf8_lossy(&bytes).to_string())
}

/// Parse an Anthropic SSE stream into (event, payload) pairs.
fn events(sse: &str) -> Vec<(String, Value)> {
    sse.split("\n\n")
        .filter(|f| !f.trim().is_empty())
        .map(|frame| {
            let mut name = String::new();
            let mut data = String::new();
            for line in frame.lines() {
                if let Some(rest) = line.strip_prefix("event: ") {
                    name = rest.to_string();
                } else if let Some(rest) = line.strip_prefix("data: ") {
                    data.push_str(rest);
                }
            }
            (name, serde_json::from_str(&data).unwrap_or(Value::Null))
        })
        .collect()
}

#[tokio::test]
async fn claude_code_keeps_working_on_near_ai_when_the_subscription_is_exhausted() {
    let (base, received) = spawn_nearai(vec![text_turn("The off-by-one is in sum_to.")]).await;
    let state = state_with_exhausted_claude(&base);

    let (status, backend, sse) = post(
        &state,
        &claude_code_request(json!([{"role": "user", "content": "why does the test fail?"}])),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(backend.as_deref(), Some("nearai"), "did not fall back");

    // The client gets a well-formed Anthropic stream — it cannot tell.
    let events = events(&sse);
    let names: Vec<&str> = events.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop"
        ],
        "stream shape Claude Code cannot parse: {sse}"
    );
    assert_eq!(events[0].1["message"]["role"], "assistant");
    // The model the client asked for — not NEAR AI's slug, which would make the
    // client's own bookkeeping incoherent.
    assert_eq!(events[0].1["message"]["model"], "claude-opus-4-6");
    assert_eq!(events[2].1["delta"]["text"], "The off-by-one is in sum_to.");
    assert_eq!(events[4].1["delta"]["stop_reason"], "end_turn");
    assert_eq!(events[4].1["usage"]["output_tokens"], 12);

    // And NEAR AI received a valid Chat Completions request.
    let sent = received.lock().expect("lock");
    let request = sent.first().expect("NEAR AI was called");
    assert_eq!(request["model"], "near-x");
    assert_eq!(request["stream"], true);
    assert_eq!(request["stream_options"]["include_usage"], true);
    assert_eq!(request["messages"][0]["role"], "system");
    assert_eq!(request["messages"][1]["content"], "why does the test fail?");
    assert_eq!(request["tools"][0]["function"]["name"], "Bash");
}

#[tokio::test]
async fn a_tool_call_from_near_ai_reaches_claude_code_as_a_valid_tool_use_block() {
    // The load-bearing case: an agent is only useful if it can call tools, and
    // the two providers disagree about how a call is shaped on the wire.
    let (base, _received) = spawn_nearai(vec![tool_call_turn(
        "Bash",
        r#"{"command":"cargo test --quiet"}"#,
    )])
    .await;
    let state = state_with_exhausted_claude(&base);

    let (status, backend, sse) = post(
        &state,
        &claude_code_request(json!([{"role": "user", "content": "run the tests"}])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(backend.as_deref(), Some("nearai"));

    let events = events(&sse);
    let block = events
        .iter()
        .find(|(n, p)| n == "content_block_start" && p["content_block"]["type"] == "tool_use")
        .map(|(_, p)| p["content_block"].clone())
        .unwrap_or_else(|| panic!("no tool_use block in: {sse}"));

    assert_eq!(block["name"], "Bash");
    // An object, not the JSON *string* Chat Completions streams.
    assert_eq!(block["input"]["command"], "cargo test --quiet");
    // An id valid in the client's namespace — it will replay this forever.
    assert!(
        block["id"].as_str().expect("id").starts_with("toolu_"),
        "tool id is not valid for an Anthropic client: {block}"
    );

    let stop = events
        .iter()
        .find(|(n, _)| n == "message_delta")
        .expect("message_delta");
    // Get this wrong and the agent stops instead of running the command.
    assert_eq!(stop.1["delta"]["stop_reason"], "tool_use");
}

#[tokio::test]
async fn the_tool_id_near_ai_minted_survives_the_round_trip_back() {
    // Claude Code replays whatever id we returned. If the reverse mapping is
    // wrong, the second turn is rejected by the far side.
    let (base, received) = spawn_nearai(vec![
        tool_call_turn("Bash", r#"{"command":"cargo test"}"#),
        text_turn("Fixed."),
    ])
    .await;
    let state = state_with_exhausted_claude(&base);

    let (_, _, first) = post(
        &state,
        &claude_code_request(json!([{"role": "user", "content": "run the tests"}])),
    )
    .await;
    let minted = events(&first)
        .iter()
        .find(|(n, p)| n == "content_block_start" && p["content_block"]["type"] == "tool_use")
        .map(|(_, p)| p["content_block"]["id"].as_str().expect("id").to_string())
        .expect("a tool_use id");

    // Turn two: the agent replays our id and appends the tool result, then
    // closes the loop with a fresh user turn (a boundary, so the family stays).
    let (status, _, _) = post(
        &state,
        &claude_code_request(json!([
            {"role": "user", "content": "run the tests"},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": minted, "name": "Bash",
                 "input": {"command": "cargo test"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": minted, "content": "1 failed"}
            ]},
            {"role": "assistant", "content": [{"type": "text", "text": "I see it."}]},
            {"role": "user", "content": "fix it"}
        ])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let sent = received.lock().expect("lock");
    let second = sent.get(1).expect("a second NEAR AI request");
    let messages = second["messages"].as_array().expect("messages");
    let tool_message = messages
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap_or_else(|| panic!("no tool message in {second}"));
    // Back in NEAR AI's own namespace, exactly as it minted it.
    assert_eq!(tool_message["tool_call_id"], "call_near_1");
    assert_eq!(tool_message["content"], "1 failed");

    let assistant_call = messages
        .iter()
        .find(|m| m.get("tool_calls").is_some())
        .expect("the assistant turn with the call");
    assert_eq!(assistant_call["tool_calls"][0]["id"], "call_near_1");
}

#[tokio::test]
async fn a_conversation_mid_tool_loop_does_not_change_families() {
    // The safety rule. Switching here would hand the agent an assistant turn
    // whose tool_use carries no reasoning state, and replaying that history to
    // Anthropic later risks a hard rejection.
    let (base, received) = spawn_nearai(vec![text_turn("unused")]).await;
    let state = state_with_exhausted_claude(&base);

    let (status, backend, body) = post(
        &state,
        &claude_code_request(json!([
            {"role": "user", "content": "run the tests"},
            {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "...", "signature": "sig-abc"},
                {"type": "tool_use", "id": "toolu_01ABC", "name": "Bash",
                 "input": {"command": "cargo test"}}
            ]},
            // The last message replays a tool result: the loop is in flight.
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_01ABC", "content": "1 failed"}
            ]}
        ])),
    )
    .await;

    assert_ne!(backend.as_deref(), Some("nearai"), "switched mid tool loop");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        body.contains("MidToolLoop"),
        "the refusal should name its reason: {body}"
    );
    assert!(
        received.lock().expect("lock").is_empty(),
        "NEAR AI was called mid tool loop"
    );
}

#[tokio::test]
async fn the_same_conversation_switches_at_the_next_turn_boundary() {
    // The other half of the rule, and the reason it is a deferral rather than a
    // refusal: the identical session becomes eligible one turn later.
    let (base, received) = spawn_nearai(vec![text_turn("On it.")]).await;
    let state = state_with_exhausted_claude(&base);

    let history = json!([
        {"role": "user", "content": "run the tests"},
        {"role": "assistant", "content": [
            {"type": "thinking", "thinking": "...", "signature": "sig-abc"},
            {"type": "tool_use", "id": "toolu_01ABC", "name": "Bash",
             "input": {"command": "cargo test"}}
        ]},
        {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_01ABC", "content": "1 failed"}
        ]},
        {"role": "assistant", "content": [{"type": "text", "text": "Off-by-one."}]},
        // A fresh user turn: nothing is in flight.
        {"role": "user", "content": "fix it"}
    ]);

    let (status, backend, _) = post(&state, &claude_code_request(history)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        backend.as_deref(),
        Some("nearai"),
        "a turn boundary is a clean switch point"
    );

    // Signed thinking blocks reached the boundary and were dropped, not
    // refused — the correction this lane is built on.
    let sent = received.lock().expect("lock");
    let request = sent.first().expect("NEAR AI was called");
    assert!(
        !request.to_string().contains("sig-abc"),
        "a signature leaked to a foreign provider: {request}"
    );
    assert!(
        request["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .any(|m| m["role"] == "tool"),
        "the tool history did not survive: {request}"
    );
}

#[tokio::test]
async fn the_cross_family_hop_is_recorded_as_such() {
    // Rung 3 is the one the user is told about, so it has to be visible.
    let (base, _received) = spawn_nearai(vec![text_turn("done")]).await;
    let state = state_with_exhausted_claude(&base);
    let ledger = state.ledger.clone().expect("ledger");

    let (status, _, _) = post(
        &state,
        &claude_code_request(json!([{"role": "user", "content": "hi"}])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let entry = loop {
        if let Some(row) = ledger.recent(1).expect("reads").first() {
            break row.clone();
        }
        assert!(tokio::time::Instant::now() < deadline, "no ledger entry");
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    };
    assert_eq!(entry.backend, "nearai");
    assert_eq!(entry.rung, "crossfamily");
    assert_eq!(entry.requested_model.as_deref(), Some("claude-opus-4-6"));
    assert_eq!(entry.output_tokens, Some(12));
}

#[tokio::test]
async fn the_subscription_is_used_again_once_its_window_resets() {
    // Cross-family is a fallback, not a destination.
    let (near_base, received) = spawn_nearai(vec![]).await;
    let anthropic_base = spawn_anthropic().await;
    let state = state_with(&near_base, &anthropic_base);
    let claude = state
        .backends
        .all()
        .iter()
        .find(|b| b.id().as_str() == "anthropic-key")
        .cloned()
        .expect("the Anthropic backend");

    // Its retry-after window elapses and the provider reports plenty of room.
    claude.record(&Observation {
        primary: Some(ironwire_upstream::observe::RateLimitReading {
            used_pct: 5.0,
            resets_at: Some(Utc::now() + Duration::hours(1)),
        }),
        ..Observation::default()
    });
    assert!(matches!(claude.quota().primary, Headroom::Observed { .. }));

    let (status, backend, _) = post(
        &state,
        &claude_code_request(json!([{"role": "user", "content": "hi"}])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        backend.as_deref(),
        Some("anthropic-key"),
        "should return to the same-family backend once it recovers"
    );
    assert!(
        received.lock().expect("lock").is_empty(),
        "NEAR AI was used while same-family capacity was available"
    );
}

#[tokio::test]
async fn an_unrecognised_content_block_refuses_the_cross_family_route() {
    // `docs/PROTOCOL.md` §6: what would genuinely break the request is refused
    // rather than silently degraded. An unmodelled block — a `document` a user
    // asked a question about, or whatever Anthropic ships next — used to fall
    // through the translator's catch-all and vanish, so the model answered
    // about content it never received.
    //
    // Refusing means the conversation waits for same-family capacity. That is
    // the correct trade: an error the user can see beats an answer built on
    // something they sent and the model never got.
    let (base, received) = spawn_nearai(vec![text_turn("should never be reached")]).await;
    let state = state_with_exhausted_claude(&base);

    let mut body = claude_code_request(json!([
        {"role": "user", "content": [
            {"type": "text", "text": "what does this say?"},
            {"type": "document", "source": {"type": "base64", "data": "JVBERi0="}},
        ]}
    ]));
    body["stream"] = json!(true);

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/anthropic/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("served");

    assert_ne!(
        response.status(),
        StatusCode::OK,
        "the request was served by translating away content the user sent"
    );
    assert!(
        received.lock().expect("lock").is_empty(),
        "a request with an unmodelled block reached the foreign provider"
    );
}

#[tokio::test]
async fn a_request_with_only_modelled_blocks_still_crosses_families() {
    // The other side of the refusal: it must not have disabled the fallback
    // lane for ordinary traffic, which is the whole feature.
    let (base, received) = spawn_nearai(vec![text_turn("done")]).await;
    let state = state_with_exhausted_claude(&base);

    let mut body = claude_code_request(json!([
        {"role": "user", "content": [{"type": "text", "text": "fix the test"}]}
    ]));
    body["stream"] = json!(true);

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/anthropic/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("served");

    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;
    assert!(
        !received.lock().expect("lock").is_empty(),
        "ordinary traffic no longer reaches the fallback lane"
    );
}
