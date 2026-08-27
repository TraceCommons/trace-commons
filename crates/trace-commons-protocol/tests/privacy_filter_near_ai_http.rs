#![cfg(feature = "near-ai-privacy-filter")]

use std::time::Duration;

use serde_json::json;
use trace_commons_protocol::privacy_filter_near_ai::{
    CLASSIFY_CHUNK_BYTES, MAX_CLASSIFY_BATCH_INPUTS, NearAiPrivacyFilterAdapter,
};
use trace_commons_protocol::trace_contribution::{PrivacyFilterAdapter, run_privacy_filter_canary};
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

/// Responder for the batched wire contract: one `data` entry per `input`
/// element, carrying its `index`, with a `private_email` span on whichever
/// element contains the test's email.
fn email_span_per_input(req: &wiremock::Request) -> ResponseTemplate {
    let body: serde_json::Value = serde_json::from_slice(&req.body).expect("request body is JSON");
    let inputs = body
        .get("input")
        .and_then(|v| v.as_array())
        .expect("input must be an array");
    let data: Vec<serde_json::Value> = inputs
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if value.as_str().unwrap_or("").contains("bob@example.com") {
                json!({ "index": index, "spans": [
                    {"category": "private_email", "start": 8, "end": 23,
                     "score": 0.99, "text": "bob@example.com"}
                ]})
            } else {
                json!({ "index": index, "spans": [] })
            }
        })
        .collect();
    ResponseTemplate::new(200).set_body_json(json!({ "data": data }))
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
    // Field text larger than CLASSIFY_CHUNK_BYTES is split into windows and
    // classified per window. The email lands in a later window; its
    // window-local offsets must be shifted into full-text coordinates so the
    // right region is redacted. Sized off the constant rather than a literal
    // so the test keeps its meaning when the vendor's ceiling moves.
    // Windows break at the last newline inside the limit, so a whole number
    // of lines fills a window exactly. Two full windows of filler leave
    // "contact ..." alone in the third, putting the email at window-local
    // codepoint offset 8 regardless of what CLASSIFY_CHUNK_BYTES is set to.
    const LINE: &str = "clean line\n"; // 11 bytes, ends on a newline
    let lines_per_window = CLASSIFY_CHUNK_BYTES / LINE.len();
    let filler = LINE.repeat(lines_per_window * 2);
    assert!(
        filler.len() > CLASSIFY_CHUNK_BYTES,
        "filler must overflow at least one window"
    );
    let text = format!("{filler}contact bob@example.com now");

    let server = MockServer::start().await;
    // Window carrying the email: return a span at the email's window-local
    // codepoint offsets ("contact " = 8, "bob@example.com" = 15 -> 8..23).
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(email_span_per_input)
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
async fn retries_on_5xx_then_succeeds() {
    // The hosted endpoint returns transient 502s; the adapter should retry
    // a few times before giving up. First two calls 502, third succeeds.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(502).set_body_string("upstream hiccup"))
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "spans": [
                {"category": "private_email", "start": 12, "end": 29, "score": 0.99, "text": "alice@example.com"}
            ]}]
        })))
        .with_priority(2)
        .mount(&server)
        .await;

    let result = adapter(server.uri())
        .redact_text("email me at alice@example.com please")
        .await
        .expect("should succeed after retrying transient 5xx")
        .expect("non-empty redaction");
    assert_eq!(
        result.redacted_text,
        "email me at [REDACTED:private_email] please"
    );
}

#[tokio::test]
async fn does_not_retry_4xx() {
    // A 4xx is not transient (bad auth/request); fail fast without retry.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .expect(1) // must be hit exactly once (no retries)
        .mount(&server)
        .await;

    let err = adapter(server.uri())
        .redact_text("hello")
        .await
        .expect_err("4xx must error");
    assert!(err.to_string().contains("status=401"));
    // server.verify() on drop asserts expect(1)
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
        msg.contains("missing a result for input 0"),
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

// --- typed transient/permanent classification ---------------------------
//
// The adapter already decides transient-vs-permanent when it chooses whether
// to retry. These tests pin that decision as TYPED data on the returned
// error, so a caller that keeps a per-trace attempt budget can act on it
// without parsing an error string.

#[tokio::test]
async fn http_5xx_is_typed_transient() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(502).set_body_string("Please try again later."))
        .mount(&server)
        .await;

    let err = adapter(server.uri())
        .redact_text("hello world")
        .await
        .expect_err("5xx must error");
    assert!(
        err.is_transient(),
        "an upstream 5xx is about the vendor, not the trace: {err}"
    );
}

#[tokio::test]
async fn http_4xx_is_typed_permanent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
        .mount(&server)
        .await;

    let err = adapter(server.uri())
        .redact_text("hello world")
        .await
        .expect_err("4xx must error");
    assert!(
        !err.is_transient(),
        "a 4xx is about the request we sent and must stay permanent: {err}"
    );
}

#[tokio::test]
async fn transport_timeout_is_typed_transient() {
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
    assert!(
        err.is_transient(),
        "a transport timeout is transient: {err}"
    );
}

#[tokio::test]
async fn malformed_body_is_typed_permanent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .mount(&server)
        .await;

    let err = adapter(server.uri())
        .redact_text("hello world")
        .await
        .expect_err("empty data array must error");
    assert!(
        !err.is_transient(),
        "a body-shape failure is not retryable and must stay permanent: {err}"
    );
}

#[tokio::test]
async fn oversized_input_is_typed_permanent() {
    // Nothing was sent upstream at all: the input itself is the problem.
    let adapter = NearAiPrivacyFilterAdapter::new(
        "https://near-ai.example.com",
        "openai/privacy-filter",
        "test-api-key-do-not-leak",
        Duration::from_secs(5),
        16,
    )
    .expect("adapter builds");

    let err = adapter
        .redact_text(&"x".repeat(64))
        .await
        .expect_err("oversized input must error");
    assert!(
        !err.is_transient(),
        "an oversized input is about the trace and must stay permanent: {err}"
    );
}

/// Windows go out batched, and batches go out one at a time.
///
/// #456 added a concurrency knob and set it to 8; the pilot dropped to zero
/// throughput, failures beginning 80 seconds after that deploy and decaying
/// away only ~14 minutes after the rollback. The knob is gone. Batching is the
/// replacement: it cuts the request COUNT while lowering the request RATE.
#[tokio::test]
async fn windows_are_batched_into_one_request_per_group() {
    const LINE: &str = "clean line\n";
    let lines_per_window = CLASSIFY_CHUNK_BYTES / LINE.len();
    let window_count = MAX_CLASSIFY_BATCH_INPUTS + 4;
    let text = LINE.repeat(lines_per_window * window_count);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(|req: &wiremock::Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).expect("request body is JSON");
            let inputs = body
                .get("input")
                .and_then(|v| v.as_array())
                .expect("input must be an array, not a bare string");
            assert!(
                inputs.len() <= MAX_CLASSIFY_BATCH_INPUTS,
                "batch of {} exceeds the cap",
                inputs.len()
            );
            let data: Vec<serde_json::Value> = inputs
                .iter()
                .enumerate()
                .map(|(index, _)| json!({ "index": index, "spans": [] }))
                .collect();
            ResponseTemplate::new(200).set_body_json(json!({ "data": data }))
        })
        // Twelve windows batched eight at a time is two requests, not twelve.
        .expect(2)
        .mount(&server)
        .await;

    adapter(server.uri())
        .redact_text(&text)
        .await
        .expect("call succeeds");
    // MockServer::verify runs on drop and asserts the expected request count.
}

/// Results are matched to windows by the response's `index`, including across
/// batch boundaries -- a span in the second batch must land at its full-text
/// offset.
#[tokio::test]
async fn spans_map_back_by_index_across_batch_boundaries() {
    const LINE: &str = "clean line\n";
    let lines_per_window = CLASSIFY_CHUNK_BYTES / LINE.len();
    // Enough whole windows of filler to push the email past the first batch.
    let filler = LINE.repeat(lines_per_window * (MAX_CLASSIFY_BATCH_INPUTS + 2));
    let text = format!("{filler}contact bob@example.com now");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(email_span_per_input)
        .mount(&server)
        .await;

    let result = adapter(server.uri())
        .redact_text(&text)
        .await
        .expect("call succeeds")
        .expect("non-empty redaction");
    assert_eq!(
        result.redacted_text,
        format!("{filler}contact [REDACTED:private_email] now")
    );
    assert_eq!(result.summary.span_count, 1);
}

/// Fail closed on a short response. An input with no corresponding `data`
/// entry must be an error, never an empty span list: empty spans mean "this
/// window is clean", so silently accepting a missing result would pass
/// unexamined text off as scrubbed.
#[tokio::test]
async fn a_missing_result_for_an_input_is_an_error_not_a_clean_window() {
    const LINE: &str = "clean line\n";
    let lines_per_window = CLASSIFY_CHUNK_BYTES / LINE.len();
    let text = LINE.repeat(lines_per_window * 3);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(|req: &wiremock::Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).expect("request body is JSON");
            let inputs = body
                .get("input")
                .and_then(|v| v.as_array())
                .expect("input must be an array");
            let data: Vec<serde_json::Value> = inputs
                .iter()
                .enumerate()
                .take(inputs.len().saturating_sub(1))
                .map(|(index, _)| json!({ "index": index, "spans": [] }))
                .collect();
            ResponseTemplate::new(200).set_body_json(json!({ "data": data }))
        })
        .mount(&server)
        .await;

    let err = adapter(server.uri())
        .redact_text(&text)
        .await
        .expect_err("a short response must fail closed");
    assert!(
        err.to_string().contains("missing"),
        "error should name the missing result; got {err}"
    );
}

/// The window's full-text offset is accumulated across windows rather than
/// recomputed from the start of the field each time (which was O(n^2)). That
/// accumulation counts CODEPOINTS, not bytes -- multibyte filler in every
/// window would shift every later span if it counted bytes.
#[tokio::test]
async fn window_offsets_stay_correct_across_multibyte_windows() {
    // Multibyte in the filler, and a whole number of lines per window so the
    // email lands alone in the final window at window-local offset 8.
    const LINE: &str = "clèan lïne wîth ünicode\n";
    let lines_per_window = CLASSIFY_CHUNK_BYTES / LINE.len();
    let filler = LINE.repeat(lines_per_window * 2);
    let text = format!("{filler}contact bob@example.com now");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(email_span_per_input)
        .mount(&server)
        .await;

    let result = adapter(server.uri())
        .redact_text(&text)
        .await
        .expect("call succeeds")
        .expect("non-empty redaction");
    assert_eq!(
        result.redacted_text,
        format!("{filler}contact [REDACTED:private_email] now"),
        "a byte-counted accumulator would misplace this span"
    );
    assert_eq!(result.summary.span_count, 1);
}
