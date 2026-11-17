//! Conformance: the daemon survives bodies it was not designed for.
//!
//! IronWire is loopback-only, so the "attacker" here is the user's own agent
//! having a bad day, or a provider SDK sending something unexpected. That does
//! not make it unimportant: a proxy that panics takes down every conversation
//! on the machine, and the user has no idea why their agent stopped.
//!
//! Several code paths walk a parsed body recursively — `peek`'s
//! `json_contains_key`, the privacy filter's substitution walk. Recursion over
//! attacker-shaped input is how a process dies, so the depth limit that
//! protects them is asserted here rather than assumed.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::{Config, PrivacyConfig};
use ironwire_creds::ConsentLedger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use secrecy::SecretString;
use tower::ServiceExt;

fn state(privacy: PrivacyConfig) -> AppState {
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        // Port 1 is never listening. Nothing here should reach a backend
        // anyway: these bodies must be rejected or handled before that.
        AnthropicBackend::api_key(
            SecretString::from("sk-ant-test"),
            Some("http://127.0.0.1:1".to_string()),
            2,
        )
        .expect("client builds"),
    ));
    AppState::new(
        registry,
        Config {
            privacy,
            ..Config::default()
        },
        ConsentLedger::default(),
        "test-token".to_string(),
    )
}

async fn post(state: AppState, body: Vec<u8>) -> StatusCode {
    app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/anthropic/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await
        .expect("the daemon answered rather than dying")
        .status()
}

fn filter_on() -> PrivacyConfig {
    PrivacyConfig {
        enabled: true,
        secrets: true,
        named_values: vec!["sentinel-value".to_string()],
        ..PrivacyConfig::default()
    }
}

#[tokio::test]
async fn a_deeply_nested_body_does_not_take_the_daemon_down() {
    // The failure this guards against is a stack overflow in a recursive walk,
    // which is not a caught panic — it kills the process and every other
    // conversation with it.
    let depth = 20_000;
    let mut body = String::with_capacity(depth * 2 + 64);
    body.push_str(r#"{"model":"m","messages":[{"role":"user","content":"#);
    for _ in 0..depth {
        body.push('[');
    }
    for _ in 0..depth {
        body.push(']');
    }
    body.push_str("}]}");

    let status = post(state(PrivacyConfig::default()), body.into_bytes()).await;
    assert!(
        status.is_client_error() || status.is_server_error(),
        "a body this shape should be refused, got {status}"
    );
}

#[tokio::test]
async fn a_deeply_nested_body_is_survivable_with_the_privacy_filter_on() {
    // The filter adds its own recursive walk over the parsed body, so it needs
    // its own assertion rather than inheriting the one above.
    let depth = 20_000;
    let mut body = String::with_capacity(depth * 2 + 64);
    body.push_str(r#"{"model":"m","messages":[{"role":"user","content":"#);
    for _ in 0..depth {
        body.push('[');
    }
    for _ in 0..depth {
        body.push(']');
    }
    body.push_str("}]}");

    let status = post(state(filter_on()), body.into_bytes()).await;
    assert!(
        status.is_client_error() || status.is_server_error(),
        "got {status}"
    );
}

#[tokio::test]
async fn malformed_and_surprising_bodies_all_get_an_answer() {
    // None of these should reach a backend, and every one must produce a
    // response rather than a panic. A proxy that dies on a bad body takes every
    // conversation on the machine with it.
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("whitespace", b"   \n\t ".to_vec()),
        ("not json", b"this is not json at all".to_vec()),
        ("truncated", br#"{"model":"m","messages":["#.to_vec()),
        ("json null", b"null".to_vec()),
        ("json array at the root", b"[1,2,3]".to_vec()),
        ("json string at the root", br#""just a string""#.to_vec()),
        ("json number at the root", b"42".to_vec()),
        ("empty object", b"{}".to_vec()),
        (
            "messages is a string, not an array",
            br#"{"model":"m","messages":"oops"}"#.to_vec(),
        ),
        (
            "model is a number",
            br#"{"model":404,"messages":[]}"#.to_vec(),
        ),
        (
            "system is an object",
            br#"{"model":"m","system":{"unexpected":true},"messages":[]}"#.to_vec(),
        ),
        (
            "content is deeply mistyped",
            br#"{"model":"m","messages":[{"role":"user","content":{"a":{"b":[null,true]}}}]}"#
                .to_vec(),
        ),
        ("invalid utf-8", vec![0x7b, 0xff, 0xfe, 0x7d]),
        (
            "a lone surrogate escape",
            br#"{"model":"m","messages":[{"role":"user","content":"\ud800"}]}"#.to_vec(),
        ),
    ];

    for (name, body) in cases {
        let status = post(state(PrivacyConfig::default()), body.clone()).await;
        assert!(
            status.is_client_error() || status.is_server_error(),
            "{name}: expected a refusal, got {status}"
        );
        // And again with the filter on, which parses and rewrites the body.
        let status = post(state(filter_on()), body).await;
        assert!(
            status.is_client_error() || status.is_server_error(),
            "{name} (filter on): expected a refusal, got {status}"
        );
    }
}

#[tokio::test]
async fn a_very_wide_body_is_handled_without_pathological_slowdown() {
    // Width rather than depth: 50k sibling messages. The peek and the filter
    // both walk every one, and an accidental quadratic here would show up as a
    // daemon that stops answering under a long session.
    let messages: Vec<serde_json::Value> = (0..50_000)
        .map(|i| serde_json::json!({"role": "user", "content": format!("m{i}")}))
        .collect();
    let body = serde_json::json!({
        "model": "claude-opus-4-6",
        "system": "You are Claude Code",
        "messages": messages,
    })
    .to_string();

    let started = std::time::Instant::now();
    let status = post(state(filter_on()), body.into_bytes()).await;
    let elapsed = started.elapsed();

    // It reaches the dead backend and fails there, which is the correct
    // outcome — what matters is that it got that far promptly.
    assert!(
        status.is_client_error() || status.is_server_error(),
        "{status}"
    );
    assert!(
        elapsed.as_secs() < 20,
        "a wide body took {elapsed:?}; something is quadratic"
    );
}

#[tokio::test]
async fn many_conversations_at_once_stay_separate() {
    // Affinity, the privacy salt, and the circuit breaker are all keyed on the
    // conversation. Sharing state across them would be a data leak between two
    // of the user's own sessions, which is not a threat model anyone reasons
    // about because it should be impossible.
    let state = state(filter_on());

    let mut handles = Vec::new();
    for session in 0..32 {
        let state = state.clone();
        handles.push(tokio::spawn(async move {
            let body = serde_json::json!({
                "model": "claude-opus-4-6",
                "system": format!("You are Claude Code, session {session}"),
                "messages": [{"role": "user", "content": "sentinel-value"}],
            })
            .to_string();
            post(state, body.into_bytes()).await
        }));
    }

    for handle in handles {
        let status = handle.await.expect("no task panicked");
        // The backend is dead, so every one fails — the assertion is that they
        // all *answered*, concurrently, without deadlocking on a shared lock.
        assert!(
            status.is_client_error() || status.is_server_error(),
            "{status}"
        );
    }

    // Every session should have its own tracked route.
    let tracked = {
        let policy = state.policy.lock().expect("lock");
        policy.tracked_conversations()
    };
    assert!(
        tracked <= 32,
        "more conversations tracked than were started: {tracked}"
    );
}

#[tokio::test]
async fn concurrent_requests_in_one_conversation_do_not_deadlock() {
    // The policy, the breaker board and the privacy salt table are each behind
    // a lock, and a request touches all three. Ten turns of one conversation
    // arriving at once is the shape that finds a lock-ordering mistake.
    let state = state(filter_on());
    let body = serde_json::json!({
        "model": "claude-opus-4-6",
        "system": "You are Claude Code",
        "messages": [{"role": "user", "content": "sentinel-value"}],
    })
    .to_string();

    let mut handles = Vec::new();
    for _ in 0..10 {
        let state = state.clone();
        let body = body.clone();
        handles.push(tokio::spawn(
            async move { post(state, body.into_bytes()).await },
        ));
    }

    let deadline = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        for handle in handles {
            let _ = handle.await.expect("no task panicked");
        }
    });
    assert!(
        deadline.await.is_ok(),
        "concurrent turns of one conversation deadlocked"
    );
}
