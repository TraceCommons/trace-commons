#![cfg(feature = "near-ai-privacy-filter")]

use std::time::Duration;

use serde_json::json;
use trace_commons_protocol::privacy_filter_near_ai::NearAiPrivacyFilterAdapter;
use trace_commons_protocol::trace_contribution::{PrivacyFilterAdapter, run_privacy_filter_canary};
use wiremock::matchers::{body_string_contains, header, method, path};
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
    assert_eq!(
        result.redacted_text,
        "email me at [REDACTED:private_email] please"
    );
    assert_eq!(result.summary.span_count, 1);
}

#[tokio::test]
async fn large_field_is_chunked_and_span_offsets_are_merged() {
    // Field text larger than CLASSIFY_CHUNK_BYTES (20_000) is split into
    // windows and classified per window. The email lands in the second
    // window; its window-local offsets must be shifted into full-text
    // coordinates so the right region is redacted.
    let filler = "clean line\n".repeat(1818); // 19_998 bytes, ends on a newline
    let text = format!("{filler}contact bob@example.com now");

    let server = MockServer::start().await;
    // Window carrying the email: return a span at the email's window-local
    // codepoint offsets ("contact " = 8, "bob@example.com" = 15 -> 8..23).
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .and(body_string_contains("bob@example.com"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "spans": [
                {"category": "private_email", "start": 8, "end": 23, "score": 0.99, "text": "bob@example.com"}
            ]}]
        })))
        .with_priority(1)
        .mount(&server)
        .await;
    // Every other window (the filler): no PII.
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "spans": [] }]
        })))
        .with_priority(5)
        .mount(&server)
        .await;

    let result = adapter(server.uri())
        .redact_text(&text)
        .await
        .expect("call succeeds")
        .expect("non-empty redaction");
    let expected = format!("{filler}contact [REDACTED:private_email] now");
    assert_eq!(result.redacted_text, expected);
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
async fn empty_data_array_surfaces_as_redaction_failed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .mount(&server)
        .await;

    let err = adapter(server.uri())
        .redact_text("hello world")
        .await
        .expect_err("empty data array must error fail-closed");
    let msg = err.to_string();
    assert!(
        msg.contains("near-ai privacy classifier returned empty data array"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn malformed_json_surfaces_as_redaction_failed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("{not valid json")
                .insert_header("content-type", "application/json"),
        )
        .mount(&server)
        .await;

    let err = adapter(server.uri())
        .redact_text("hello world")
        .await
        .expect_err("malformed JSON must error fail-closed");
    let msg = err.to_string();
    assert!(
        msg.contains("parse error"),
        "expected 'parse error' in message; got: {msg}"
    );
}

#[tokio::test]
async fn empty_spans_passes_text_through_unchanged() {
    // Documented contract: a 200 with `data: [{spans: []}]` means the
    // classifier looked at the text and found no PII. We return
    // Ok(Some(redaction)) with span_count = 0 and redacted_text equal
    // to the original — same effective behavior as the sidecar on a
    // clean trace.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"spans": []}]
        })))
        .mount(&server)
        .await;

    let original = "totally clean text without PII";
    let result = adapter(server.uri())
        .redact_text(original)
        .await
        .expect("call succeeds")
        .expect("entry present even with no spans");
    assert_eq!(result.redacted_text, original);
    assert_eq!(result.summary.span_count, 0);
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
    assert!(
        report.healthy,
        "canary should be healthy: {:?}",
        report.failures
    );
}
