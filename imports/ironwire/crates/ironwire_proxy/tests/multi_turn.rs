//! Agent-level conformance: a whole tool-use conversation, turn by turn.
//!
//! `docs/PROTOCOL.md` §7.5 wants a scripted Claude Code task run end to end.
//! That needs a live account and a real agent, so it stays a manual check
//! (`scripts/acceptance.sh`). This is the automatable half, and it catches the
//! same class of bug: a field mis-mapped on turn 3 of a tool loop, a tool_use
//! id mangled on replay, a signed thinking block corrupted when the
//! conversation grows.
//!
//! The shape being exercised is exactly what Claude Code does:
//!
//! ```text
//! turn 1  user "fix the test"      → assistant: thinking + tool_use(Bash)
//! turn 2  + tool_result (failure)  → assistant: thinking + tool_use(Edit)
//! turn 3  + tool_result (success)  → assistant: text "done"
//! ```
//!
//! Each turn replays the entire prior transcript, which is where fidelity bugs
//! actually surface: a proxy that survives one request can still corrupt the
//! fourth.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::Config;
use ironwire_creds::ConsentLedger;
use ironwire_ledger::Ledger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// An upstream that records every request body it receives and replies with a
/// canned non-streaming response, one per turn.
async fn spawn_recorder(turns: usize) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&bodies);

    tokio::spawn(async move {
        for _ in 0..turns {
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
                    sink.lock()
                        .expect("lock")
                        .push(String::from_utf8_lossy(&buf[split..split + length]).to_string());
                    break;
                }
            }

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
        }
    });

    (format!("http://{addr}"), bodies)
}

fn state_for(base_url: &str) -> AppState {
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
    .with_ledger(Some(Ledger::in_memory().expect("ledger opens")))
}

/// The stable head of the conversation: Claude Code's system prompt with a
/// cache breakpoint, and its tool schemas.
fn preamble() -> (Value, Value) {
    (
        json!([{
            "type": "text",
            "text": "You are Claude Code, Anthropic's official CLI for Claude.",
            "cache_control": {"type": "ephemeral"}
        }]),
        json!([
            {"name": "Bash", "description": "run a command",
             "input_schema": {"type": "object", "properties": {"command": {"type": "string"}}}},
            {"name": "Edit", "description": "edit a file",
             "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}}}
        ]),
    )
}

/// The three turns, each replaying everything before it — as a real agent does.
fn turns() -> Vec<Value> {
    let (system, tools) = preamble();

    let assistant_1 = json!({
        "role": "assistant",
        "content": [
            {"type": "thinking", "thinking": "Let me run the test.", "signature": "sig-turn-1"},
            {"type": "tool_use", "id": "toolu_01ABC", "name": "Bash",
             "input": {"command": "cargo test"}}
        ]
    });
    let tool_result_1 = json!({
        "role": "user",
        "content": [{"type": "tool_result", "tool_use_id": "toolu_01ABC",
                     "content": "assertion failed at src/lib.rs:42", "is_error": true}]
    });
    let assistant_2 = json!({
        "role": "assistant",
        "content": [
            {"type": "thinking", "thinking": "Off-by-one in the loop bound.", "signature": "sig-turn-2"},
            {"type": "tool_use", "id": "toolu_02DEF", "name": "Edit",
             "input": {"path": "src/lib.rs"}}
        ]
    });
    let tool_result_2 = json!({
        "role": "user",
        "content": [{"type": "tool_result", "tool_use_id": "toolu_02DEF", "content": "edited"}]
    });

    let user = json!({"role": "user", "content": "fix the failing test"});
    let base = |messages: Value| {
        json!({
            "model": "claude-opus-4-6",
            "max_tokens": 8192,
            "system": system,
            "tools": tools,
            "thinking": {"type": "enabled", "budget_tokens": 4096},
            "messages": messages,
        })
    };

    vec![
        base(json!([user])),
        base(json!([user, assistant_1, tool_result_1])),
        base(json!([
            user,
            assistant_1,
            tool_result_1,
            assistant_2,
            tool_result_2
        ])),
    ]
}

#[tokio::test]
async fn a_three_turn_tool_loop_survives_intact() {
    let scripted = turns();
    let (base_url, recorded) = spawn_recorder(scripted.len()).await;
    let state = state_for(&base_url);

    let mut sent = Vec::new();
    for turn in &scripted {
        // Compact, exactly as an SDK would serialise it.
        let bytes = serde_json::to_vec(turn).expect("serialises");
        sent.push(String::from_utf8(bytes.clone()).expect("utf8"));

        let response = app(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/anthropic/v1/messages")
                    .header("content-type", "application/json")
                    .header("anthropic-version", "2023-06-01")
                    .body(Body::from(bytes))
                    .expect("request builds"),
            )
            .await
            .expect("served");
        assert_eq!(response.status(), StatusCode::OK);
        let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;
    }

    let received = recorded.lock().expect("lock").clone();
    assert_eq!(
        received.len(),
        scripted.len(),
        "every turn reached upstream"
    );
    for (index, (got, want)) in received.iter().zip(&sent).enumerate() {
        assert_eq!(
            got,
            want,
            "turn {} was altered in flight — a tool loop that survives one \
             request can still be corrupted on the fourth",
            index + 1
        );
    }
}

#[tokio::test]
async fn tool_use_ids_and_thinking_signatures_replay_verbatim() {
    let scripted = turns();
    let (base_url, recorded) = spawn_recorder(scripted.len()).await;
    let state = state_for(&base_url);

    for turn in &scripted {
        let response = app(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/anthropic/v1/messages")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(turn).expect("serialises")))
                    .expect("request builds"),
            )
            .await
            .expect("served");
        let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;
    }

    let last: Value =
        serde_json::from_str(recorded.lock().expect("lock").last().expect("a final turn"))
            .expect("valid json");

    let messages = last["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 5);

    // The ids the client minted must come back to the provider byte for byte:
    // the client replays what we returned, forever.
    assert_eq!(messages[1]["content"][1]["id"], "toolu_01ABC");
    assert_eq!(messages[2]["content"][0]["tool_use_id"], "toolu_01ABC");
    assert_eq!(messages[3]["content"][1]["id"], "toolu_02DEF");
    assert_eq!(messages[4]["content"][0]["tool_use_id"], "toolu_02DEF");

    // Signed thinking blocks are validated by Anthropic on replay; a mangled
    // signature is a 400 the user cannot diagnose.
    assert_eq!(messages[1]["content"][0]["signature"], "sig-turn-1");
    assert_eq!(messages[3]["content"][0]["signature"], "sig-turn-2");

    // The cache breakpoint has to survive too, or the user silently pays full
    // price for a prefix they thought was cached.
    assert_eq!(last["system"][0]["cache_control"]["type"], "ephemeral");
}

#[tokio::test]
async fn every_turn_of_one_conversation_lands_on_the_same_backend() {
    // Affinity is the whole point of per-conversation routing: a session that
    // hops backends between turns throws away its prompt cache each time.
    let scripted = turns();
    let (base_url, _recorded) = spawn_recorder(scripted.len()).await;
    let state = state_for(&base_url);

    let mut backends = Vec::new();
    for turn in &scripted {
        let response = app(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/anthropic/v1/messages")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(turn).expect("serialises")))
                    .expect("request builds"),
            )
            .await
            .expect("served");
        backends.push(
            response
                .headers()
                .get("x-ironwire-backend")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string(),
        );
        let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;
    }

    assert_eq!(backends, vec!["anthropic-key"; scripted.len()]);
    assert_eq!(
        state.policy.lock().expect("lock").tracked_conversations(),
        1,
        "a growing conversation must stay one conversation, not become three"
    );
}
