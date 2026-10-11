#![cfg(feature = "near-ai-privacy-filter")]

use std::time::Duration;

use serde_json::json;
use trace_commons_protocol::privacy_filter_near_ai::{
    MAX_CLASSIFY_ATTEMPTS, MAX_CLASSIFY_INPUT_TOKENS, MAX_CLASSIFY_REQUEST_BODY_BYTES,
    MAX_CONCURRENT_CLASSIFY_WINDOWS as CLASSIFY_CONCURRENCY, NearAiPrivacyFilterAdapter,
};
use trace_commons_protocol::trace_contribution::TraceContributionError;

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

/// The adapter with packing turned on at the window budget, the largest
/// setting `TRACE_NEAR_AI_PRIVACY_PACK_MAX_TOKENS` accepts.
fn packing_adapter(base_url: String) -> NearAiPrivacyFilterAdapter {
    adapter(base_url).with_pack_max_tokens(Some(MAX_CLASSIFY_INPUT_TOKENS))
}

/// A padded table with escaped cells: runs of spaces and `=` carry ~64-128
/// body bytes per token, so one token-budget window is megabytes of body.
fn padded_table(rows: usize) -> String {
    (0..rows)
        .map(|i| format!("| \\\"{i}\\\" |{}|{}|\n", " ".repeat(300), "=".repeat(300)))
        .collect()
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

/// A 429 (rate limited) or 408 (request timeout) is the classifier's load,
/// not the trace: it is retried in-client like a 5xx, and if it outlives the
/// retries it is typed transient so the daemon keeps the approval and paces a
/// retry instead of refusing an unchanged session for good.
#[tokio::test]
async fn rate_limiting_and_request_timeout_are_retried_and_typed_transient() {
    for status in [429, 408] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/privacy/classify"))
            .respond_with(ResponseTemplate::new(status).set_body_string("slow down"))
            // Retried like a 5xx, not failed fast like a 4xx.
            .expect(MAX_CLASSIFY_ATTEMPTS as u64)
            .mount(&server)
            .await;

        let err = adapter(server.uri())
            .redact_text("hello world")
            .await
            .expect_err("a refused classification must error");
        assert!(
            err.is_transient(),
            "{status} is about the classifier's load, not the trace: {err}"
        );
        server.verify().await;
    }
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

/// The same window text is classified ONCE, however many times it appears.
///
/// Measured across 40 real contributor sessions: 42,441 windows, 18,039
/// distinct -- 57.5% of classify round trips re-send text already sent,
/// because agent traces re-read the same files and echo the same tool output
/// across events. This is the exact half of that saving: reusing a real
/// classification of identical bytes, never asserting anything about text the
/// model has not seen.
#[tokio::test]
async fn identical_windows_are_classified_once() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "spans": [
                {"category": "private_email", "start": 12, "end": 29, "score": 0.99, "text": "alice@example.com"}
            ]}]
        })))
        // One request, not four.
        .expect(1)
        .mount(&server)
        .await;

    let adapter = adapter(server.uri());
    let text = "email me at alice@example.com please";
    for _ in 0..4 {
        let result = adapter
            .redact_text(text)
            .await
            .expect("call succeeds")
            .expect("non-empty redaction");
        assert_eq!(
            result.redacted_text, "email me at [REDACTED:private_email] please",
            "a cached classification must redact identically to a fresh one"
        );
    }
    // MockServer::verify runs on drop and asserts expect(1).
}

/// A failed classification is never cached. Caching a failure would turn one
/// transient upstream error into a permanent wrong answer for every later
/// occurrence of that window -- and since an empty span list means "clean",
/// a cached failure could pass unexamined text off as scrubbed.
#[tokio::test]
async fn a_failed_classification_is_not_cached() {
    let server = MockServer::start().await;
    // First attempt group fails; a later one succeeds. If the failure were
    // cached, the retry could never reach the success.
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(400).set_body_string("nope"))
        .up_to_n_times(1)
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

    let adapter = adapter(server.uri());
    let text = "email me at alice@example.com please";
    adapter
        .redact_text(text)
        .await
        .expect_err("first call fails");
    let result = adapter
        .redact_text(text)
        .await
        .expect("second call must re-classify, not replay a cached failure")
        .expect("non-empty redaction");
    assert_eq!(
        result.redacted_text,
        "email me at [REDACTED:private_email] please"
    );
}

/// Different windows must not collide. The cache is keyed on a content hash,
/// so a bug here would silently apply one window's spans to another's text --
/// wrong offsets, wrong redaction, no error.
#[tokio::test]
async fn different_windows_get_their_own_classification() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(email_span_at_actual_offset)
        .mount(&server)
        .await;

    let adapter = adapter(server.uri());
    let with_email = adapter
        .redact_text("please contact bob@example.com now")
        .await
        .expect("call succeeds")
        .expect("non-empty redaction");
    assert_eq!(
        with_email.redacted_text,
        "please contact [REDACTED:private_email] now"
    );

    // Different text, no needle: must be classified separately and come back
    // clean rather than inheriting the previous window's span.
    let without = adapter
        .redact_text("a completely different line with no address in it")
        .await
        .expect("call succeeds");
    assert!(
        without.is_none() || !without.unwrap().redacted_text.contains("[REDACTED"),
        "a distinct window must not inherit another window's spans"
    );
}

/// Responder that spans every occurrence of "Zoë" in the request input, at
/// CODEPOINT offsets, as the endpoint reports them.
fn name_spans_at_codepoints(req: &wiremock::Request) -> ResponseTemplate {
    const NAME: &str = "Zoë";
    let body: serde_json::Value = serde_json::from_slice(&req.body).expect("request body is JSON");
    let input = body
        .get("input")
        .and_then(|v| v.as_str())
        .expect("input must be a string");
    let spans: Vec<serde_json::Value> = input
        .match_indices(NAME)
        .map(|(byte_start, _)| {
            let start = input[..byte_start].chars().count();
            json!({
                "category": "private_person",
                "start": start,
                "end": start + NAME.chars().count(),
                "score": 0.9,
                "text": NAME,
            })
        })
        .collect();
    ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "spans": spans }] }))
}

/// Many short texts go up in a handful of requests, not one each, and every
/// span lands on the text it was found in -- including behind multibyte
/// characters in earlier texts of the same request.
#[tokio::test]
async fn many_short_texts_are_packed_into_few_requests() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(name_spans_at_codepoints)
        .mount(&server)
        .await;

    let adapter = packing_adapter(server.uri());
    let owned: Vec<String> = (0..500)
        .map(|i| match i % 3 {
            0 => format!("事件 {i} 😀: Zoë reviewed it"),
            1 => format!("event {i}: nothing to report"),
            _ => format!("event {i}: asked Zoë"),
        })
        .collect();
    let texts: Vec<&str> = owned.iter().map(String::as_str).collect();
    let results = adapter.redact_texts(&texts).await.expect("call succeeds");

    let requests = server.received_requests().await.expect("recording on");
    assert!(
        requests.len() <= 2,
        "500 short texts took {} requests",
        requests.len()
    );
    assert_eq!(results.len(), 500);
    for (i, result) in results.iter().enumerate() {
        let redacted = &result.as_ref().expect("non-blank text").redacted_text;
        let expected = match i % 3 {
            0 => format!("事件 {i} 😀: [REDACTED:private_person] reviewed it"),
            1 => format!("event {i}: nothing to report"),
            _ => format!("event {i}: asked [REDACTED:private_person]"),
        };
        assert_eq!(redacted, &expected, "text {i}");
    }

    // Every packed text was remembered under its own key, so asking again
    // sends nothing.
    let again = adapter.redact_texts(&texts).await.expect("call succeeds");
    assert_eq!(again, results);
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("recording on")
            .len(),
        requests.len(),
        "texts classified in a pack are served from the window cache"
    );
}

/// A text over `max_input_bytes` fails the whole call with the error
/// `redact_text` gives it. As when each text was its own request, the texts
/// before it are classified first and nothing after it is sent.
#[tokio::test]
async fn an_oversized_text_fails_the_batch_with_the_redact_text_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(name_spans_at_codepoints)
        .mount(&server)
        .await;
    let adapter = NearAiPrivacyFilterAdapter::new(
        server.uri(),
        "openai/privacy-filter",
        "test-api-key-do-not-leak",
        Duration::from_secs(5),
        64,
    )
    .expect("adapter builds");
    let big = "x".repeat(65);
    let alone = adapter.redact_text(&big).await.unwrap_err();
    let batched = adapter
        .redact_texts(&["fine", &big, "also fine"])
        .await
        .unwrap_err();
    assert_eq!(batched.to_string(), alone.to_string());
    let requests = server.received_requests().await.expect("recording on");
    assert_eq!(requests.len(), 1, "only the text before the oversized one");
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["input"], "fine");
}

/// The endpoint's binding limit is the request BODY (~256 KiB, a permanent
/// 413), not tokens. Escape-heavy and whitespace-heavy content carries many
/// body bytes per token, so packs and windows are capped by serialized body
/// size too.
#[tokio::test]
async fn packed_and_windowed_request_bodies_stay_under_the_body_cap() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "spans": [] }] })),
        )
        .mount(&server)
        .await;

    // Many small whitespace-heavy fields (padded tables, logs) and one large
    // padded, escaped table.
    let mut owned: Vec<String> = (0..300)
        .map(|i| format!("| row {i} |{}|{}|", " ".repeat(120), "\t\n".repeat(60)))
        .collect();
    owned.push(padded_table(1_200));
    let texts: Vec<&str> = owned.iter().map(String::as_str).collect();

    // With and without packing: the cap holds in both modes.
    for packing in [false, true] {
        server.reset().await;
        Mock::given(method("POST"))
            .and(path("/privacy/classify"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "spans": [] }] })),
            )
            .mount(&server)
            .await;
        let adapter = if packing {
            packing_adapter(server.uri())
        } else {
            adapter(server.uri())
        };
        let results = adapter.redact_texts(&texts).await.expect("call succeeds");
        for (text, result) in texts.iter().zip(&results) {
            assert_eq!(&result.as_ref().unwrap().redacted_text, text);
        }

        let requests = server.received_requests().await.expect("recording on");
        let largest = requests.iter().map(|r| r.body.len()).max().unwrap();
        assert!(
            largest <= MAX_CLASSIFY_REQUEST_BODY_BYTES,
            "packing={packing}: a request body was {largest} bytes, over the \
             {MAX_CLASSIFY_REQUEST_BODY_BYTES} cap"
        );
        if packing {
            assert!(requests.len() < 100, "still packed: {}", requests.len());
        } else {
            assert!(
                requests.len() > 300,
                "one text per request: {}",
                requests.len()
            );
        }
    }
}

/// The single-field path is capped too: `redact_text` windows a field whose
/// token windows would each be megabytes of body.
#[tokio::test]
async fn a_single_field_is_windowed_under_the_body_cap() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "spans": [] }] })),
        )
        .mount(&server)
        .await;
    let text = padded_table(1_200);
    let result = adapter(server.uri())
        .redact_text(&text)
        .await
        .expect("call succeeds")
        .expect("non-blank");
    assert_eq!(result.redacted_text, text);
    let requests = server.received_requests().await.expect("recording on");
    let largest = requests.iter().map(|r| r.body.len()).max().unwrap();
    assert!(
        largest <= MAX_CLASSIFY_REQUEST_BODY_BYTES,
        "a request body was {largest} bytes, over the {MAX_CLASSIFY_REQUEST_BODY_BYTES} cap"
    );
    let inputs: String = requests
        .iter()
        .map(|r| {
            let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
            body["input"].as_str().unwrap().to_string()
        })
        .collect();
    assert_eq!(inputs, text, "the windows cover the field exactly");
}

/// Packing is off unless an operator sets it: every text is its own
/// request, the request pattern before packing existed.
#[tokio::test]
async fn by_default_each_text_is_its_own_request() {
    let server = server_with(shifted_name_spans(0, Some("Zoë"))).await;
    let owned: Vec<String> = (0..50).map(|i| format!("event {i}: Zoë")).collect();
    let texts: Vec<&str> = owned.iter().map(String::as_str).collect();
    let results = adapter(server.uri()).redact_texts(&texts).await.unwrap();
    let requests = server.received_requests().await.expect("recording on");
    assert_eq!(requests.len(), 50, "one request per text");
    for (i, request) in requests.iter().enumerate() {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["input"], format!("event {i}: Zoë"));
    }
    for (i, result) in results.iter().enumerate() {
        assert_eq!(
            result.as_ref().unwrap().redacted_text,
            format!("event {i}: [REDACTED:private_person]")
        );
    }
}

/// Responder that reports every "Zoë" with the given codepoint shift and the
/// given `text` field (None omits it).
fn shifted_name_spans(
    shift: usize,
    text: Option<&'static str>,
) -> impl Fn(&wiremock::Request) -> ResponseTemplate + Send + Sync + 'static {
    move |req: &wiremock::Request| {
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        let input = body["input"].as_str().unwrap();
        let spans: Vec<serde_json::Value> = input
            .match_indices("Zoë")
            .map(|(byte, _)| {
                let start = input[..byte].chars().count() + shift;
                let mut span = json!({
                    "category": "private_person",
                    "start": start,
                    "end": start + 3,
                    "score": 0.9,
                });
                if let Some(text) = text {
                    span["text"] = json!(text);
                }
                span
            })
            .collect();
        ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "spans": spans }] }))
    }
}

async fn server_with(
    responder: impl Fn(&wiremock::Request) -> ResponseTemplate + Send + Sync + 'static,
) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(responder)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn a_span_whose_text_matches_its_offsets_is_applied() {
    let server = server_with(shifted_name_spans(0, Some("Zoë"))).await;
    let adapter = adapter(server.uri());
    let single = adapter.redact_text("😀 hi Zoë").await.unwrap().unwrap();
    assert_eq!(single.redacted_text, "😀 hi [REDACTED:private_person]");
    let packed = adapter
        .redact_texts(&["李 Zoë", "nothing", "Zoë 👨‍👩‍👧"])
        .await
        .unwrap();
    assert_eq!(
        packed[0].as_ref().unwrap().redacted_text,
        "李 [REDACTED:private_person]"
    );
    assert_eq!(
        packed[2].as_ref().unwrap().redacted_text,
        "[REDACTED:private_person] 👨‍👩‍👧"
    );
}

/// An offset slip -- the right entity reported one codepoint late -- passes
/// every range check and would redact the wrong characters, leaving the name
/// in place. The span's own `text` exposes it, so the call fails closed.
#[tokio::test]
async fn a_span_whose_text_differs_from_its_offsets_fails_closed() {
    let server = server_with(shifted_name_spans(1, Some("Zoë"))).await;
    let adapter = adapter(server.uri());
    let packing = packing_adapter(server.uri());
    for error in [
        // A single text, alone in its request: the unpacked path.
        adapter.redact_text("say hi to Zoë now").await.unwrap_err(),
        adapter
            .redact_texts(&["first text", "say hi to Zoë now", "third"])
            .await
            .unwrap_err(),
        packing
            .redact_texts(&["first text", "say hi to Zoë now", "third"])
            .await
            .unwrap_err(),
    ] {
        assert!(
            matches!(error, TraceContributionError::RedactionFailed { .. }),
            "permanent, not transient: {error}"
        );
        let message = error.to_string();
        assert!(
            message.contains("privacy_filter_span_text_mismatch"),
            "{message}"
        );
        assert!(
            !message.contains("Zoë"),
            "the text is never echoed: {message}"
        );
    }
}

#[tokio::test]
async fn a_span_without_text_fails_closed() {
    let server = server_with(shifted_name_spans(0, None)).await;
    let adapter = adapter(server.uri());
    let error = adapter.redact_text("say hi to Zoë now").await.unwrap_err();
    assert!(matches!(
        error,
        TraceContributionError::RedactionFailed { .. }
    ));
    assert!(
        error
            .to_string()
            .contains("privacy_filter_span_text_mismatch")
    );
}

/// The oversized-input check must not jump ahead of an earlier field's
/// failure: classified one at a time, the first field's transient 503 is the
/// error, and transient vs permanent decides whether the trace is charged.
#[tokio::test]
async fn an_earlier_transient_failure_wins_over_a_later_oversized_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let adapter = NearAiPrivacyFilterAdapter::new(
        server.uri(),
        "openai/privacy-filter",
        "test-api-key-do-not-leak",
        Duration::from_secs(5),
        64,
    )
    .expect("adapter builds");
    let big = "x".repeat(65);
    let error = adapter.redact_texts(&["fine", &big]).await.unwrap_err();
    assert!(
        matches!(
            error,
            TraceContributionError::TransientRedactionFailed { .. }
        ),
        "expected the first field's transient error, got: {error}"
    );
}
