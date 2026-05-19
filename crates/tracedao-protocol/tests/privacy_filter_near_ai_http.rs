#![cfg(feature = "near-ai-privacy-filter")]

use std::time::Duration;

use serde_json::json;
use tracedao_protocol::privacy_filter_near_ai::NearAiPrivacyFilterAdapter;
use tracedao_protocol::trace_contribution::{run_privacy_filter_canary, PrivacyFilterAdapter};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn adapter(base_url: String) -> NearAiPrivacyFilterAdapter {
    NearAiPrivacyFilterAdapter::new(
        base_url,
        "openai/privacy-filter",
        "test-api-key-do-not-leak",
        Duration::from_secs(5),
        1_000_000,
    )
    .expect("adapter builds")
}

#[tokio::test]
async fn classifies_and_redacts_single_span() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .and(header("authorization", "Bearer test-api-key-do-not-leak"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{
                "spans": [
                    {"category": "private_email", "start": 12, "end": 29, "score": 0.99, "text": "alice@example.com"}
                ]
            }]
        })))
        .mount(&server)
        .await;

    let adapter = adapter(server.uri());
    let result = adapter
        .redact_text("email me at alice@example.com please")
        .await
        .expect("call succeeds")
        .expect("non-empty redaction");
    assert_eq!(result.redacted_text, "email me at [REDACTED:private_email] please");
    assert_eq!(result.summary.span_count, 1);
}

#[tokio::test]
async fn surfaces_http_5xx_as_redaction_failed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(500).set_body_string("oh no"))
        .mount(&server)
        .await;

    let err = adapter(server.uri())
        .redact_text("hello world")
        .await
        .expect_err("5xx must error");
    let msg = err.to_string();
    assert!(msg.contains("status=500"));
    assert!(msg.contains("body_hash=sha256:"));
    assert!(!msg.contains("oh no"));
}

#[tokio::test]
async fn timeout_surfaces_as_redaction_failed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&server)
        .await;

    let adapter = NearAiPrivacyFilterAdapter::new(
        server.uri(),
        "openai/privacy-filter",
        "test-api-key-do-not-leak",
        Duration::from_millis(200),
        1_000_000,
    )
    .expect("adapter builds");

    let err = adapter
        .redact_text("hello world")
        .await
        .expect_err("timeout must error");
    let msg = err.to_string();
    assert!(msg.to_lowercase().contains("transport"));
    assert!(!msg.contains("test-api-key-do-not-leak"));
}

#[tokio::test]
async fn error_strings_do_not_leak_api_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let err = adapter(server.uri())
        .redact_text("x")
        .await
        .expect_err("401 errors");
    assert!(!err.to_string().contains("test-api-key-do-not-leak"));
}

#[tokio::test]
async fn canary_run_against_mock_returns_healthy() {
    let server = MockServer::start().await;
    // Canary text is three synthetic values joined by spaces; mock
    // returns three spans covering each.
    // Canary text is the three synthetic values joined by single
    // spaces: "<a> <b> <c>". Verified byte offsets:
    //   a = "trace-canary.person@example.invalid"  (35 bytes) → 0..35
    //   space at 35..36
    //   b = "tc_canary_secret_0123456789abcdef"     (33 bytes) → 36..69
    //   space at 69..70
    //   c = "/tmp/trace_canary_private/path.txt"   (34 bytes) → 70..104
    // Total text length: 104 bytes.
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{
                "spans": [
                    {"category": "private_email",   "start":  0, "end":  35, "score": 0.99, "text": "trace-canary.person@example.invalid"},
                    {"category": "secret",          "start": 36, "end":  69, "score": 0.99, "text": "tc_canary_secret_0123456789abcdef"},
                    {"category": "private_address", "start": 70, "end": 104, "score": 0.99, "text": "/tmp/trace_canary_private/path.txt"}
                ]
            }]
        })))
        .mount(&server)
        .await;

    let report = run_privacy_filter_canary(&adapter(server.uri()))
        .await
        .expect("canary runs");
    assert!(report.healthy, "canary should be healthy: {:?}", report.failures);
}
