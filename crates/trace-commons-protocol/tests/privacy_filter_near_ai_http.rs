#![cfg(feature = "near-ai-privacy-filter")]

use std::time::Duration;

use serde_json::json;
use trace_commons_protocol::privacy_filter_near_ai::{
    MAX_CLASSIFY_INPUT_TOKENS, MAX_CONCURRENT_CLASSIFY_WINDOWS as CLASSIFY_CONCURRENCY,
    NearAiPrivacyFilterAdapter,
};

/// Roughly the byte span of one full window for this filler, at the sparse
/// (prose-like) end of token density. Windows are budgeted in TOKENS now, so
/// tests size their filler from the token budget rather than a byte constant.
const APPROX_BYTES_PER_WINDOW: usize = MAX_CLASSIFY_INPUT_TOKENS * 4;
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

/// Responder that finds the test email inside whichever window carries it and
/// spans it at that window's own codepoint offsets.
///
/// Windows are budgeted in tokens, so which window holds the email -- and
/// where inside it -- depends on the content's density. Computing the offset
/// instead of hardcoding it keeps these tests about span merging rather than
/// about where a boundary happens to fall.
fn email_span_at_actual_offset(req: &wiremock::Request) -> ResponseTemplate {
    const NEEDLE: &str = "bob@example.com";
    let body: serde_json::Value = serde_json::from_slice(&req.body).expect("request body is JSON");
    let input = body
        .get("input")
        .and_then(|v| v.as_str())
        .expect("input must be a string");
    let spans = match input.find(NEEDLE) {
        Some(byte_start) => {
            let start = input[..byte_start].chars().count();
            vec![json!({
                "category": "private_email",
                "start": start,
                "end": start + NEEDLE.chars().count(),
                "score": 0.99,
                "text": NEEDLE,
            })]
        }
        None => Vec::new(),
    };
    ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "spans": spans }] }))
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
    // Field text larger than one window is split and
    // classified per window. The email lands in a later window; its
    // window-local offsets must be shifted into full-text coordinates so the
    // right region is redacted. Sized off the constant rather than a literal
    // so the test keeps its meaning when the vendor's ceiling moves.
    // Windows break at the last newline inside the limit, so a whole number
    // of lines fills a window exactly. Two full windows of filler leave
    // "contact ..." alone in the third, putting the email at window-local
    // codepoint offset 8 regardless of how the window budget is set.
    const LINE: &str = "clean line\n"; // 11 bytes, ends on a newline
    let lines_per_window = APPROX_BYTES_PER_WINDOW / LINE.len();
    let filler = LINE.repeat(lines_per_window * 2);
    assert!(
        filler.len() > APPROX_BYTES_PER_WINDOW,
        "filler must overflow at least one window"
    );
    let text = format!("{filler}contact bob@example.com now");

    let server = MockServer::start().await;
    // Window carrying the email: return a span at the email's window-local
    // codepoint offsets ("contact " = 8, "bob@example.com" = 15 -> 8..23).
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(email_span_at_actual_offset)
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

/// Windows are classified ONE AT A TIME, on purpose.
///
/// #456 raised `MAX_CONCURRENT_CLASSIFY_WINDOWS` to 8 to cut the per-field
/// cost. Throughput collapsed instead: on the pilot every PII-backstop tick
/// then returned `done=0 transient=3 breaker_tripped=true` and the queue
/// drained nothing, so the host was rolled back to the sequential build.
///
/// Why concurrency hurt is still not established -- the obvious rate-limit
/// explanation did not survive testing, since 20 rapid 8 KB requests all
/// returned 200. Until the classify diagnostics explain the real failures,
/// this stays at 1, and raising it should be a deliberate change made with
/// evidence rather than an optimisation someone reaches for again.
#[test]
fn classify_windows_are_not_overlapped() {
    assert_eq!(
        CLASSIFY_CONCURRENCY, 1,
        "raising classify concurrency regressed the pilot to zero throughput \
         once already; see this test's comment before changing it"
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
    let lines_per_window = APPROX_BYTES_PER_WINDOW / LINE.len();
    let filler = LINE.repeat(lines_per_window * 2);
    let text = format!("{filler}contact bob@example.com now");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(email_span_at_actual_offset)
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
