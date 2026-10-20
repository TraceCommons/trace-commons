//! Smoke tests against the **real** providers, with the machine's own
//! credentials.
//!
//! Everything else in this directory proves IronWire's side of a contract
//! against a mock. That is most of the value and none of the risk — but the
//! contract itself is with two undocumented, private surfaces
//! (`docs/TRUST.md` §2) that can change without telling anyone. These are the
//! tests that notice.
//!
//! ```bash
//! IRONWIRE_LIVE=1 cargo test -p ironwire_proxy --test live_subscriptions -- --nocapture
//! ```
//!
//! **Opt-in, and deliberately hard to run by accident.** They spend real
//! subscription quota, they need credentials this repository does not have, and
//! nothing in CI runs them. Without `IRONWIRE_LIVE=1` every one of them returns
//! immediately.
//!
//! Two rules they follow, because the alternative would make the suite itself a
//! trust problem:
//!
//! - **Consent is read, never granted.** A subscription is off until the user
//!   enables it (`docs/TRUST.md` §2), and a test that flipped that switch to
//!   make itself runnable would be doing exactly what the gate exists to
//!   prevent. `IRONWIRE_LIVE=1` is permission to *spend* consented capacity, not
//!   permission to consent.
//! - **Nothing is forged.** The Claude Code and Codex identities in these
//!   requests are the real products' own, because that is what the providers
//!   are being asked to serve (`docs/TRUST.md` §3, I5). A test that synthesized
//!   one would be demonstrating the impersonation the invariant forbids.
//!
//! Each request is one short turn with a small output cap: enough to prove the
//! path works, cheap enough to run often.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::{Config, PathsConfig};
use ironwire_creds::codex::CodexMode;
use ironwire_creds::{ClaudeCodeCredentials, CodexCredentials, ConsentLedger};
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use ironwire_upstream::openai_responses::ResponsesBackend;
use serde_json::{Value, json};
use tower::ServiceExt;

/// Whether the user asked for these to run at all.
fn live() -> bool {
    std::env::var("IRONWIRE_LIVE").as_deref() == Ok("1")
}

/// Print why a test did nothing, so a silent pass is never mistaken for a real
/// one. `--nocapture` is in the command above for exactly this.
fn skip(reason: &str) {
    eprintln!("SKIPPED: {reason}");
}

/// The machine's recorded consents. Read from `$IRONWIRE_HOME`, exactly as the
/// daemon reads them, and never written.
fn consent() -> ConsentLedger {
    PathsConfig::resolve()
        .map(|paths| ConsentLedger::load(&paths.consent_file()))
        .unwrap_or_default()
}

/// State holding one real backend.
fn state_with(backend: Arc<dyn ironwire_upstream::backend::Backend>) -> AppState {
    let mut registry = BackendRegistry::new();
    registry.push(backend);
    AppState::new(
        registry,
        Config::default(),
        consent(),
        "live-test-token".to_string(),
    )
}

/// A Claude Code turn, carrying Claude Code's own identifying system block —
/// which is what the Anthropic subscription is entitled to require.
fn claude_code_body() -> Value {
    json!({
        "model": "claude-haiku-4-5-20251001",
        "max_tokens": 16,
        "stream": true,
        "system": [{
            "type": "text",
            "text": "You are Claude Code, Anthropic's official CLI for Claude."
        }],
        "messages": [{"role": "user", "content": "Reply with the single word: pong"}]
    })
}

/// The same turn from something that is not Claude Code.
fn third_party_body() -> Value {
    json!({
        "model": "claude-haiku-4-5-20251001",
        "max_tokens": 16,
        "stream": true,
        "system": "You are a helpful coding assistant.",
        "messages": [{"role": "user", "content": "Reply with the single word: pong"}]
    })
}

/// A Codex turn, carrying Codex's own instructions block.
fn codex_body() -> Value {
    json!({
        "model": "gpt-5.6",
        "stream": true,
        "instructions": "You are Codex, based on GPT-5.",
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Reply with the single word: pong"}]
        }]
    })
}

fn anthropic_request(body: &Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/anthropic/v1/messages")
        .header("content-type", "application/json")
        .header("anthropic-version", "2023-06-01")
        .body(Body::from(serde_json::to_vec(body).expect("serialises")))
        .expect("request builds")
}

fn codex_request(body: &Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/openai/v1/responses")
        .header("content-type", "application/json")
        .header("originator", "codex_cli_rs")
        .body(Body::from(serde_json::to_vec(body).expect("serialises")))
        .expect("request builds")
}

async fn send(state: AppState, request: Request<Body>) -> (StatusCode, String) {
    let response = app(state)
        .oneshot(request)
        .await
        .expect("the proxy answers");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// Text deltas out of an SSE stream, whichever wire it is.
fn streamed_text(sse: &str) -> String {
    let mut text = String::new();
    for line in sse.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(data.trim()) else {
            continue;
        };
        // Anthropic content_block_delta, and Responses output_text.delta.
        if let Some(chunk) = value
            .pointer("/delta/text")
            .and_then(Value::as_str)
            .or_else(|| {
                value
                    .get("delta")
                    .filter(|_| {
                        value.get("type").and_then(Value::as_str)
                            == Some("response.output_text.delta")
                    })
                    .and_then(Value::as_str)
            })
        {
            text.push_str(chunk);
        }
    }
    text
}

// ---------------------------------------------------------------------------
// Claude subscription
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_claude_code_turn_reaches_the_live_claude_subscription() {
    if !live() {
        return skip("set IRONWIRE_LIVE=1 to spend real subscription quota");
    }
    if ClaudeCodeCredentials::discover().is_err() {
        return skip("no Claude Code login on this machine — run `claude login`");
    }
    if !consent().is_granted("claude-sub") {
        return skip("the Claude subscription is not enabled — `ironwire connect claude`");
    }

    let backend = Arc::new(AnthropicBackend::subscription(None, 60).expect("client builds"));
    let (status, sse) = send(state_with(backend), anthropic_request(&claude_code_body())).await;

    assert_eq!(status, StatusCode::OK, "{sse}");
    assert!(sse.contains("message_start"), "{sse}");
    assert!(sse.contains("message_stop"), "{sse}");
    assert!(
        !streamed_text(&sse).trim().is_empty(),
        "the subscription answered with no text at all: {sse}"
    );
}

/// `docs/TRUST.md` §3, against the real registry: a client that is not Claude
/// Code does not get Claude Code's subscription, and IronWire does not dress it
/// up as one to change that.
#[tokio::test]
async fn a_third_party_client_is_refused_the_live_claude_subscription() {
    if !live() {
        return skip("set IRONWIRE_LIVE=1 to spend real subscription quota");
    }
    if ClaudeCodeCredentials::discover().is_err() {
        return skip("no Claude Code login on this machine — run `claude login`");
    }
    if !consent().is_granted("claude-sub") {
        return skip("the Claude subscription is not enabled — `ironwire connect claude`");
    }

    let backend = Arc::new(AnthropicBackend::subscription(None, 60).expect("client builds"));
    let (status, body) = send(state_with(backend), anthropic_request(&third_party_body())).await;

    assert_ne!(
        status,
        StatusCode::OK,
        "a third-party client was served from the Claude subscription: {body}"
    );
    // Refused by us, before anything reached Anthropic.
    assert!(
        body.contains("Claude Code") || body.contains("identity"),
        "refused for the wrong reason: {body}"
    );
}

// ---------------------------------------------------------------------------
// ChatGPT / Codex subscription
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_codex_turn_reaches_the_live_chatgpt_subscription() {
    if !live() {
        return skip("set IRONWIRE_LIVE=1 to spend real subscription quota");
    }
    if !CodexCredentials::discover().is_ok_and(|c| c.mode == CodexMode::ChatGpt) {
        return skip("no ChatGPT login in ~/.codex/auth.json — run `codex login`");
    }
    if !consent().is_granted("codex-sub") {
        return skip("the ChatGPT subscription is not enabled — `ironwire connect codex`");
    }

    let backend = Arc::new(ResponsesBackend::codex_subscription(None, 60).expect("client builds"));
    let (status, sse) = send(state_with(backend), codex_request(&codex_body())).await;

    assert_eq!(status, StatusCode::OK, "{sse}");
    assert!(sse.contains("response.created"), "{sse}");
    assert!(
        sse.contains("response.completed") || sse.contains("response.output_text"),
        "{sse}"
    );
    assert!(
        !streamed_text(&sse).trim().is_empty(),
        "the subscription answered with no text at all: {sse}"
    );
}

/// The other half of I5, and the direction that only became reachable once every
/// wire could translate to every other.
///
/// Claude Code carries a real client identity — its own. Before the identity
/// check named the product, that was enough to satisfy the ChatGPT
/// subscription's requirement, and a Claude Code turn falling back cross-wire
/// would have arrived at `chatgpt.com` wearing Claude Code's name.
#[tokio::test]
async fn claude_code_is_refused_the_live_chatgpt_subscription() {
    if !live() {
        return skip("set IRONWIRE_LIVE=1 to spend real subscription quota");
    }
    if !CodexCredentials::discover().is_ok_and(|c| c.mode == CodexMode::ChatGpt) {
        return skip("no ChatGPT login in ~/.codex/auth.json — run `codex login`");
    }
    if !consent().is_granted("codex-sub") {
        return skip("the ChatGPT subscription is not enabled — `ironwire connect codex`");
    }

    // The ChatGPT subscription is the only backend, so the only way to serve
    // this is the one TRUST.md forbids.
    let backend = Arc::new(ResponsesBackend::codex_subscription(None, 60).expect("client builds"));
    let (status, body) = send(state_with(backend), anthropic_request(&claude_code_body())).await;

    assert_ne!(
        status,
        StatusCode::OK,
        "Claude Code was served from the ChatGPT subscription: {body}"
    );
    assert!(
        !body.contains("pong"),
        "a request reached ChatGPT wearing another product's name: {body}"
    );
}

// ---------------------------------------------------------------------------
// Both at once
// ---------------------------------------------------------------------------

/// Both subscriptions registered together, each serving its own client. The
/// arrangement a machine with both logins actually has, and the one where an
/// identity mix-up would show up as "it works" until a provider noticed.
#[tokio::test]
async fn each_subscription_serves_only_its_own_client() {
    if !live() {
        return skip("set IRONWIRE_LIVE=1 to spend real subscription quota");
    }
    let has_claude =
        ClaudeCodeCredentials::discover().is_ok() && consent().is_granted("claude-sub");
    let has_codex = CodexCredentials::discover().is_ok_and(|c| c.mode == CodexMode::ChatGpt)
        && consent().is_granted("codex-sub");
    if !(has_claude && has_codex) {
        return skip("needs both subscriptions logged in and enabled");
    }

    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        AnthropicBackend::subscription(None, 60).expect("client builds"),
    ));
    registry.push(Arc::new(
        ResponsesBackend::codex_subscription(None, 60).expect("client builds"),
    ));
    let build = || {
        AppState::new(
            registry.clone(),
            Config::default(),
            consent(),
            "live-test-token".to_string(),
        )
    };

    let (claude_status, claude_sse) = send(build(), anthropic_request(&claude_code_body())).await;
    assert_eq!(claude_status, StatusCode::OK, "{claude_sse}");
    assert!(!streamed_text(&claude_sse).trim().is_empty());

    let (codex_status, codex_sse) = send(build(), codex_request(&codex_body())).await;
    assert_eq!(codex_status, StatusCode::OK, "{codex_sse}");
    assert!(!streamed_text(&codex_sse).trim().is_empty());

    // Neither answer came from the other's provider: an Anthropic stream frames
    // its events as `message_*`, a Responses one as `response.*`.
    assert!(claude_sse.contains("message_start"), "{claude_sse}");
    assert!(codex_sse.contains("response.created"), "{codex_sse}");
}
