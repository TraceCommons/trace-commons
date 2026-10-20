//! Conformance: the privacy filter across a compaction boundary, per harness.
//!
//! `docs/PRIVACY.md` §5 calls this the hardest case in the product, and the
//! reason is worth restating: a compaction summary is not a disposable
//! response. The harness writes it into its permanent history and resends it
//! every turn afterwards. An unreversed placeholder there is *self-
//! perpetuating* corruption — next turn's map is derived from plaintext and
//! will not contain that token, so it can never be reversed again.
//!
//! Every harness compacts, and none of them document how. So these tests do not
//! assert that IronWire *recognises* a compaction request: correctness must not
//! depend on a fingerprint that breaks on a client update. They assert the
//! thing that has to hold whether we recognised it or not.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::{Config, PrivacyConfig};
use ironwire_core::protocol::Protocol;
use ironwire_creds::ConsentLedger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use ironwire_upstream::openai_chat::ChatCompletionsBackend;
use ironwire_upstream::openai_responses::ResponsesBackend;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const SECRET: &str = "alice@internal-corp.acme-holdings-real.com";

/// Which façade and dialect a harness speaks.
#[derive(Clone, Copy)]
struct Harness {
    name: &'static str,
    uri: &'static str,
    protocol: Protocol,
}

const HARNESSES: &[Harness] = &[
    Harness {
        name: "Claude Code",
        uri: "/anthropic/v1/messages",
        protocol: Protocol::AnthropicMessages,
    },
    Harness {
        name: "Codex",
        uri: "/openai/v1/responses",
        protocol: Protocol::OpenAiResponses,
    },
    Harness {
        name: "Aider",
        uri: "/openai/v1/chat/completions",
        protocol: Protocol::OpenAiChat,
    },
    // Cline / Roo speak whichever façade they are pointed at; the two above
    // cover both wires, so a third fixture would test the same code twice.
];

type Seen = Arc<Mutex<String>>;
type Reply = Arc<Mutex<String>>;

/// An upstream that records the request and echoes a settable body, in the SSE
/// dialect the façade expects.
async fn spawn(protocol: Protocol, initial: &str) -> (String, Seen, Reply) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let seen: Seen = Arc::new(Mutex::new(String::new()));
    let reply: Reply = Arc::new(Mutex::new(initial.to_string()));
    let sink = Arc::clone(&seen);
    let source = Arc::clone(&reply);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let sink = Arc::clone(&sink);
            let source = Arc::clone(&source);
            tokio::spawn(async move {
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
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(split) = text.find("\r\n\r\n") {
                        let length: usize = text[..split]
                            .lines()
                            .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                            .and_then(|l| l.split(':').nth(1))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        if buf.len() >= split + 4 + length {
                            *sink.lock().expect("lock") =
                                text[split + 4..split + 4 + length].to_string();
                            break;
                        }
                    }
                }

                let text = source.lock().expect("lock").clone();
                let encoded = serde_json::to_string(&text).expect("encodes");
                let sse = match protocol {
                    Protocol::AnthropicMessages => format!(
                        "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"m\",\"usage\":{{\"input_tokens\":1}}}}}}\n\n\
                         event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":{encoded}}}}}\n\n\
                         event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
                    ),
                    Protocol::OpenAiResponses => format!(
                        "event: response.created\ndata: {{\"type\":\"response.created\",\"response\":{{\"id\":\"r\"}}}}\n\n\
                         event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":{encoded}}}\n\n\
                         event: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"r\",\"usage\":{{\"input_tokens\":1}}}}}}\n\n"
                    ),
                    Protocol::OpenAiChat => format!(
                        "data: {{\"choices\":[{{\"delta\":{{\"content\":{encoded}}}}}]}}\n\n\
                         data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\n\
                         data: [DONE]\n\n"
                    ),
                };
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

    (format!("http://{addr}"), seen, reply)
}

/// A long session with a trailing compaction instruction — the shape every
/// harness produces when it hits the context limit.
fn compaction_request(harness: Harness) -> String {
    let mut turns: Vec<serde_json::Value> = (0..12)
        .map(|i| {
            serde_json::json!({
                "role": "user",
                "content": format!("turn {i}: mail {SECRET} about the outage"),
            })
        })
        .collect();
    turns.push(serde_json::json!({
        "role": "user",
        "content": "Provide a detailed summary of the conversation so far.",
    }));

    match harness.protocol {
        Protocol::AnthropicMessages => serde_json::json!({
            "model": "claude-opus-4-6",
            "stream": true,
            "system": "You are Claude Code",
            "messages": turns,
        }),
        Protocol::OpenAiResponses => serde_json::json!({
            "model": "gpt-5.6",
            "stream": true,
            "instructions": "You are Codex, a coding agent.",
            "input": turns,
        }),
        Protocol::OpenAiChat => serde_json::json!({
            "model": "gpt-5.6",
            "stream": true,
            "messages": turns,
        }),
    }
    .to_string()
}

fn state_for(harness: Harness, base_url: &str) -> AppState {
    let mut registry = BackendRegistry::new();
    match harness.protocol {
        Protocol::AnthropicMessages => registry.push(Arc::new(
            AnthropicBackend::api_key(
                SecretString::from("sk-ant-test"),
                Some(base_url.to_string()),
                10,
            )
            .expect("client builds"),
        )),
        Protocol::OpenAiResponses => registry.push(Arc::new(
            ResponsesBackend::openai_api_key(
                SecretString::from("sk-test"),
                Some(base_url.to_string()),
                10,
            )
            .expect("client builds"),
        )),
        // A Chat Completions client needs a Chat Completions backend. Handing
        // it a Responses one used to "work" only because the routing policy
        // treated the two as interchangeable — they share a family and are
        // different wires, and the body would have arrived unreadable.
        Protocol::OpenAiChat => registry.push(Arc::new(
            ChatCompletionsBackend::new(
                ironwire_core::protocol::BackendId::from("openai-chat"),
                "OpenAI-compatible",
                ironwire_core::protocol::BackendKind::ApiKey,
                Some(SecretString::from("sk-test")),
                base_url.to_string(),
                Vec::new(),
                10,
            )
            .expect("client builds"),
        )),
    }
    let config = Config {
        privacy: PrivacyConfig {
            enabled: true,
            secrets: true,
            named_values: vec![SECRET.to_string()],
            ..PrivacyConfig::default()
        },
        ..Config::default()
    };
    AppState::new(
        registry,
        config,
        ConsentLedger::default(),
        "test-token".to_string(),
    )
}

async fn send(state: AppState, harness: Harness, body: String) -> (StatusCode, String) {
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(harness.uri)
                .header("content-type", "application/json")
                .body(Body::from(body))
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

fn minted_token(seen: &Seen) -> String {
    let received = seen.lock().expect("lock").clone();
    received
        .split(['"', ' ', '\\'])
        .find(|piece| piece.starts_with('\u{27e6}'))
        .unwrap_or_else(|| panic!("no placeholder was sent:\n{received}"))
        .to_string()
}

#[tokio::test]
async fn a_summary_quoting_a_placeholder_is_fully_reversed() {
    // The good path, per harness. The summary is about to become permanent
    // history, so it must contain the real value and no token.
    for harness in HARNESSES {
        let (base, seen, reply) = spawn(harness.protocol, "").await;
        let state = state_for(*harness, &base);

        let (status, _) = send(state.clone(), *harness, compaction_request(*harness)).await;
        assert_eq!(status, StatusCode::OK, "{}", harness.name);
        let token = minted_token(&seen);

        // The model quotes the token in its summary, verbatim.
        *reply.lock().expect("lock") = format!("The user repeatedly emailed {token}.");
        let (_, response) = send(state, *harness, compaction_request(*harness)).await;

        assert!(
            response.contains(SECRET),
            "{}: the summary did not get the real value back:\n{response}",
            harness.name
        );
        assert!(
            !response.contains('\u{27e6}'),
            "{}: a placeholder would have been written into permanent history:\n{response}",
            harness.name
        );
    }
}

#[tokio::test]
async fn a_mangled_placeholder_in_a_summary_fails_rather_than_being_written() {
    // Summarization is exactly the operation that paraphrases and truncates, so
    // this is the likeliest place for a token to be damaged — and the worst
    // place for it to be forwarded.
    for harness in HARNESSES {
        let (base, seen, reply) = spawn(harness.protocol, "").await;
        let state = state_for(*harness, &base);

        let _ = send(state.clone(), *harness, compaction_request(*harness)).await;
        let token = minted_token(&seen);
        let truncated: String = token.chars().take(token.chars().count() - 3).collect();

        *reply.lock().expect("lock") = format!("The user emailed {truncated} several times.");
        let (_, response) = send(state, *harness, compaction_request(*harness)).await;

        assert!(
            !response.contains(SECRET),
            "{}: a half-reversed summary was forwarded:\n{response}",
            harness.name
        );
        // The client must not receive a summary it would store.
        assert!(
            !response.contains("several times"),
            "{}: the damaged summary reached the client and would become \
             permanent history:\n{response}",
            harness.name
        );
    }
}

#[tokio::test]
async fn a_stale_token_from_a_previous_salt_is_never_mis_reversed() {
    // After a daemon restart the salt changes. A token minted before it can
    // still arrive in replayed history — mapping it to a real value would be
    // the worst bug this filter could have.
    let harness = HARNESSES[0];
    let (base, seen, reply) = spawn(harness.protocol, "").await;

    let first = state_for(harness, &base);
    let _ = send(first, harness, compaction_request(harness)).await;
    let stale = minted_token(&seen);

    // A brand-new daemon: new salt, so `stale` is not in any map it builds.
    let restarted = state_for(harness, &base);
    *reply.lock().expect("lock") = format!("Earlier the user mentioned {stale}.");
    let (status, response) = send(restarted, harness, compaction_request(harness)).await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        !response.contains(SECRET),
        "a stale token was reversed to a real value:\n{response}"
    );
    assert!(
        response.contains(&stale),
        "a token we did not mint should pass through untouched:\n{response}"
    );
}

#[tokio::test]
async fn compaction_does_not_change_the_conversation_key() {
    // Affinity is derived from the system preamble and tool names, neither of
    // which compaction changes (`docs/PROTOCOL.md` §8). If it did change, every
    // compaction would silently re-roll the route *and* the salt, and the
    // provider's cache would be thrown away at the worst possible moment.
    let harness = HARNESSES[0];
    let (base, seen, _reply) = spawn(harness.protocol, "ok").await;
    let state = state_for(harness, &base);

    // An ordinary turn, then a compaction turn, in the same session.
    let ordinary = serde_json::json!({
        "model": "claude-opus-4-6",
        "stream": true,
        "system": "You are Claude Code",
        "messages": [{"role": "user", "content": format!("mail {SECRET}")}],
    })
    .to_string();

    let _ = send(state.clone(), harness, ordinary).await;
    let before = minted_token(&seen);

    let _ = send(state, harness, compaction_request(harness)).await;
    let after = minted_token(&seen);

    assert_eq!(
        before, after,
        "the placeholder changed across a compaction boundary, which means the \
         salt changed, which means the conversation key changed"
    );
}
