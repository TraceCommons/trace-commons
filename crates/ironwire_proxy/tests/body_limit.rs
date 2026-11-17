//! The façades' request body limit.
//!
//! axum's default is 2 MB, and a turn sized for a 1M-token window is several
//! times that. Over the limit the refusal has to arrive in the façade's own
//! error envelope: axum's plain-text 413 is a body no client parses.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use ironwire_core::config::Config;
use ironwire_creds::ConsentLedger;
use ironwire_proxy::facade::MAX_REQUEST_BODY_BYTES;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use tower::ServiceExt;

/// No backends: a request that gets past the body limit is refused for
/// capacity, which is a different status from the one under test.
fn state() -> AppState {
    AppState::new(
        BackendRegistry::new(),
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    )
}

/// A syntactically valid request body of about `bytes` bytes.
fn padded(path: &str, bytes: usize) -> Vec<u8> {
    let filler = "x".repeat(bytes);
    let body = if path.starts_with("/anthropic") {
        serde_json::json!({
            "model": "claude-opus-4-6",
            "max_tokens": 16,
            "messages": [{"role": "user", "content": filler}],
        })
    } else {
        serde_json::json!({
            "model": "z-ai/glm-5.3",
            "messages": [{"role": "user", "content": filler}],
        })
    };
    serde_json::to_vec(&body).expect("serialises")
}

async fn post(path: &str, body: Vec<u8>) -> (StatusCode, Vec<u8>) {
    let response = app(state())
        .oneshot(
            Request::post(path)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await
        .expect("router answers");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body reads")
        .to_bytes()
        .to_vec();
    (status, body)
}

const PATHS: [&str; 4] = [
    "/anthropic/v1/messages",
    "/anthropic/v1/messages/count_tokens",
    "/openai/v1/chat/completions",
    "/openai/v1/responses",
];

// Above axum's 2 MB default, which refuses a turn sized for a 1M-token window.
const _: () = assert!(MAX_REQUEST_BODY_BYTES > 2 * 1024 * 1024);

#[test]
fn the_limit_is_the_ledgers_ceiling_on_one_body() {
    assert_eq!(
        MAX_REQUEST_BODY_BYTES,
        ironwire_ledger::token_spool::MAX_BODY_BYTES
    );
}

#[tokio::test]
async fn a_body_past_axums_default_reaches_routing() {
    for path in PATHS {
        let (status, body) = post(path, padded(path, 3 * 1024 * 1024)).await;
        assert_ne!(
            status,
            StatusCode::PAYLOAD_TOO_LARGE,
            "{path}: {}",
            String::from_utf8_lossy(&body)
        );
    }
}

#[tokio::test]
async fn a_body_over_the_limit_is_refused_in_the_facades_envelope() {
    for path in PATHS {
        let (status, body) = post(path, padded(path, MAX_REQUEST_BODY_BYTES + 1)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{path}");
        let error: serde_json::Value = serde_json::from_slice(&body)
            .unwrap_or_else(|_| panic!("{path}: not JSON: {}", String::from_utf8_lossy(&body)));
        if path.starts_with("/anthropic") {
            assert_eq!(error["type"], "error", "{path}: {error}");
        }
        assert_eq!(
            error["error"]["type"], "invalid_request_error",
            "{path}: {error}"
        );
        assert!(
            error.to_string().contains("32 MiB"),
            "{path}: the refusal names the limit: {error}"
        );
    }
}
