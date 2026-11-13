#![cfg(feature = "near-ai-privacy-filter")]

use std::time::Duration;

use serde_json::json;
use trace_commons_protocol::privacy_filter_near_ai::{
    MAX_CLASSIFY_413_SPLITS, MAX_CLASSIFY_ATTEMPTS, MAX_CLASSIFY_REQUEST_BODY_BYTES,
    MAX_CONCURRENT_CLASSIFY_WINDOWS as CLASSIFY_CONCURRENCY, NearAiPrivacyFilterAdapter,
};
use trace_commons_protocol::trace_contribution::TraceContributionError;

/// Roughly the byte span of one full window. Windows are sized by request
/// body, and plain filler escapes to little more than its length, so a field
/// this long fills about one window.
const APPROX_BYTES_PER_WINDOW: usize = MAX_CLASSIFY_REQUEST_BODY_BYTES;
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

/// A padded table with escaped cells: runs of spaces and `=` carry ~64-128
/// body bytes per token, so one token-budget window is far over the body
/// limit the endpoint enforces (a permanent 413 at ~256 KiB).
fn padded_table(rows: usize) -> String {
    (0..rows)
        .map(|i| format!("| \\\"{i}\\\" |{}|{}|\n", " ".repeat(300), "=".repeat(300)))
        .collect()
}

/// A field whose token windows would each be hundreds of KiB of body is cut
/// into windows under `MAX_CLASSIFY_REQUEST_BODY_BYTES`, and together they
/// still cover it exactly.
#[tokio::test]
async fn a_field_is_windowed_under_the_body_cap() {
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
    for (input, expected) in [
        ("😀 hi Zoë", "😀 hi [REDACTED:private_person]"),
        ("李 Zoë", "李 [REDACTED:private_person]"),
        ("Zoë 👨‍👩‍👧", "[REDACTED:private_person] 👨‍👩‍👧"),
    ] {
        let result = adapter.redact_text(input).await.unwrap().unwrap();
        assert_eq!(result.redacted_text, expected);
    }
}

/// An offset slip -- the right entity reported one codepoint late -- passes
/// every range check and would redact the wrong characters, leaving the name
/// in place and reporting success. The span's own `text` exposes it, so the
/// call fails closed, non-transient, hash-only.
#[tokio::test]
async fn a_span_whose_text_differs_from_its_offsets_fails_closed() {
    let server = server_with(shifted_name_spans(1, Some("Zoë"))).await;
    let adapter = adapter(server.uri());
    let error = adapter.redact_text("say hi to Zoë now").await.unwrap_err();
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

    // A refused response is not cached: the same text is refused again.
    let again = adapter.redact_text("say hi to Zoë now").await.unwrap_err();
    assert!(
        again
            .to_string()
            .contains("privacy_filter_span_text_mismatch")
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_span_without_text_fails_closed() {
    let server = server_with(shifted_name_spans(0, None)).await;
    let error = adapter(server.uri())
        .redact_text("say hi to Zoë now")
        .await
        .unwrap_err();
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

fn adapter_with_cap(base_url: String, cap: usize) -> NearAiPrivacyFilterAdapter {
    NearAiPrivacyFilterAdapter::with_max_request_body_bytes(
        base_url,
        "openai/privacy-filter",
        "test-api-key-do-not-leak",
        Duration::from_secs(5),
        1_000_000,
        cap,
    )
    .expect("adapter builds")
}

/// A stand-in for an endpoint whose body limit has fallen to `limit`: a 413
/// above it, and below it a span on every "bob@example.com" at the window's
/// own codepoint offsets -- derived from the request, so a split window is
/// asked a different question and answered for it.
fn emails_under_body_limit(
    limit: usize,
) -> impl Fn(&wiremock::Request) -> ResponseTemplate + Send + Sync + 'static {
    move |req: &wiremock::Request| {
        if req.body.len() > limit {
            return ResponseTemplate::new(413)
                .set_body_string("Failed to buffer the request body: length limit exceeded");
        }
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        let input = body["input"].as_str().unwrap();
        let spans: Vec<serde_json::Value> = input
            .match_indices("bob@example.com")
            .map(|(byte, needle)| {
                let start = input[..byte].chars().count();
                json!({
                    "category": "private_email",
                    "start": start,
                    "end": start + needle.chars().count(),
                    "score": 0.99,
                    "text": needle,
                })
            })
            .collect();
        ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "spans": spans }] }))
    }
}

/// Escape-heavy multibyte filler with the email scattered through it.
fn escape_heavy_field_with_emails(lines: usize) -> String {
    (0..lines)
        .map(|i| {
            if i % 997 == 0 {
                format!("{i} \"caf\u{e9}\" \u{4e2d}\u{1f600} mail bob@example.com\\x\n")
            } else {
                format!("{i} \"caf\u{e9}\" \u{4e2d}\u{1f600} C:\\dir\\f{i}\t\\\"\n")
            }
        })
        .collect()
}

/// The endpoint's limit falls under the cap: every full window is answered
/// with a 413, is halved and the field still redacts to
/// exactly what an adapter whose cap was under the limit all along produces.
#[tokio::test]
async fn a_413_halves_the_window_and_redacts_identically() {
    const LIMIT: usize = 40 * 1024;
    let text = escape_heavy_field_with_emails(12_000);

    let reference_server = server_with(emails_under_body_limit(LIMIT)).await;
    let reference = adapter_with_cap(reference_server.uri(), LIMIT)
        .redact_text(&text)
        .await
        .expect("under the limit throughout")
        .expect("spans found");
    let reference_requests = reference_server.received_requests().await.unwrap();
    assert!(reference_requests.iter().all(|r| r.body.len() <= LIMIT));

    let server = server_with(emails_under_body_limit(LIMIT)).await;
    let split = adapter_with_cap(server.uri(), MAX_CLASSIFY_REQUEST_BODY_BYTES)
        .redact_text(&text)
        .await
        .expect("413s are split, not fatal")
        .expect("spans found");
    let requests = server.received_requests().await.unwrap();
    let refused = requests.iter().filter(|r| r.body.len() > LIMIT).count();
    assert!(refused > 0, "the fixture must actually draw 413s");

    assert_eq!(split.redacted_text, reference.redacted_text);
    assert_eq!(split.summary.span_count, reference.summary.span_count);
    assert_eq!(
        split.summary.span_count,
        text.matches("bob@example.com").count() as u32,
        "every email redacted, none twice"
    );
    assert!(!split.redacted_text.contains("bob@example.com"));
}

/// An endpoint that refuses everything as too large is not bisected
/// forever: after `MAX_CLASSIFY_413_SPLITS` halvings the field is refused,
/// permanently -- retrying the same text later meets the same limit.
#[tokio::test]
async fn a_413_at_every_size_refuses_after_bounded_splits() {
    let server = server_with(emails_under_body_limit(0)).await;
    let text = "line of text\n".repeat(4_000);
    let err = adapter(server.uri())
        .redact_text(&text)
        .await
        .expect_err("refused");
    assert!(
        !err.is_transient(),
        "a 413 that survives splitting is permanent: {err}"
    );
    let requests = server.received_requests().await.unwrap().len();
    assert!(
        requests <= MAX_CLASSIFY_413_SPLITS + 1,
        "{requests} requests: the first refused half must stop the field"
    );
    assert!(
        !err.to_string().contains("line of text"),
        "no content in the error"
    );
}

/// A body is never sent over the adapter's cap, whatever the content, and
/// windows cover the field exactly -- measured on the bodies the server
/// actually received.
#[tokio::test]
async fn escape_heavy_bodies_arrive_under_the_cap() {
    let server = server_with(emails_under_body_limit(usize::MAX)).await;
    let text = escape_heavy_field_with_emails(20_000);
    adapter(server.uri())
        .redact_text(&text)
        .await
        .expect("classified")
        .expect("spans found");
    let requests = server.received_requests().await.unwrap();
    assert!(requests.len() > 1);
    let inputs: String = requests
        .iter()
        .map(|r| {
            assert!(r.body.len() <= MAX_CLASSIFY_REQUEST_BODY_BYTES);
            let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
            body["input"].as_str().unwrap().to_string()
        })
        .collect();
    assert_eq!(inputs, text);
}

/// A stand-in that answers a string or an array of inputs, one entry per
/// input, spanning every "bob@example.com" in each at its own codepoint
/// offsets. `refuse_arrays_over` returns a 413 for arrays longer than it;
/// `drop_one_entry` answers an array with one entry too few.
fn per_input_emails(
    refuse_arrays_over: usize,
    drop_one_entry: bool,
) -> impl Fn(&wiremock::Request) -> ResponseTemplate + Send + Sync + 'static {
    move |req: &wiremock::Request| {
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        let (inputs, is_array): (Vec<String>, bool) = match &body["input"] {
            serde_json::Value::String(one) => (vec![one.clone()], false),
            serde_json::Value::Array(many) => (
                many.iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect(),
                true,
            ),
            other => panic!("unexpected input {other}"),
        };
        if is_array && inputs.len() > refuse_arrays_over {
            return ResponseTemplate::new(413).set_body_string("Failed to buffer the request body");
        }
        let mut data: Vec<serde_json::Value> = inputs
            .iter()
            .map(|input| {
                let spans: Vec<serde_json::Value> = input
                    .match_indices("bob@example.com")
                    .map(|(byte, needle)| {
                        let start = input[..byte].chars().count();
                        json!({"category": "private_email", "start": start,
                               "end": start + needle.chars().count(), "score": 0.99, "text": needle})
                    })
                    .collect();
                json!({"spans": spans})
            })
            .collect();
        if is_array && drop_one_entry {
            data.pop();
        }
        ResponseTemplate::new(200).set_body_json(json!({ "data": data }))
    }
}

/// Texts of every kind `redact_texts` routes differently: blank, small with
/// and without a span, multibyte, repeated, and one needing several windows.
fn mixed_texts() -> Vec<String> {
    let mut texts: Vec<String> = (0..40)
        .map(|i| match i % 5 {
            0 => format!("mail bob@example.com about item {i}"),
            1 => format!("caf\u{e9} \u{4e2d}\u{6587} \u{1f600} \"q\" \\ line {i}\n"),
            2 => format!("plain text {i}"),
            3 => "   ".to_string(),
            _ => "repeated bob@example.com text".to_string(),
        })
        .collect();
    texts.push(format!(
        "{}tail bob@example.com",
        "clean line\n".repeat(APPROX_BYTES_PER_WINDOW / 8)
    ));
    texts
}

fn batching_adapter(base_url: String, batch: usize) -> NearAiPrivacyFilterAdapter {
    adapter(base_url).with_max_batch_inputs(batch)
}

async fn answers_of(
    adapter: &NearAiPrivacyFilterAdapter,
    texts: &[String],
) -> Vec<Result<Option<String>, String>> {
    let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
    adapter
        .redact_texts(&borrowed)
        .await
        .into_iter()
        .map(|answer| {
            answer
                .map(|redaction| redaction.map(|r| r.redacted_text))
                .map_err(|e| e.to_string())
        })
        .collect()
}

/// Off by default: an adapter that was not asked to batch prefers one text
/// per call and sends no input arrays.
#[tokio::test]
async fn batching_is_off_by_default() {
    let server = server_with(per_input_emails(usize::MAX, false)).await;
    let adapter = adapter(server.uri());
    assert_eq!(adapter.preferred_batch_inputs(), 1);
    answers_of(&adapter, &mixed_texts()).await;
    for request in server.received_requests().await.unwrap() {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert!(
            body["input"].is_string(),
            "an input array was sent with batching off"
        );
    }
}

/// Batched answers are exactly the unbatched ones, text by text, in order,
/// in fewer requests -- and every body is under the cap.
#[tokio::test]
async fn batched_answers_match_unbatched_answers_by_index() {
    let texts = mixed_texts();
    let single_server = server_with(per_input_emails(usize::MAX, false)).await;
    let expected = answers_of(&adapter(single_server.uri()), &texts).await;
    let single_requests = single_server.received_requests().await.unwrap().len();

    let server = server_with(per_input_emails(usize::MAX, false)).await;
    let batched = answers_of(&batching_adapter(server.uri(), 8), &texts).await;
    assert_eq!(batched, expected);
    assert!(expected.iter().all(Result::is_ok));
    assert!(
        expected
            .iter()
            .filter_map(|a| a.as_ref().ok().cloned().flatten())
            .any(|t| t.contains("[REDACTED:private_email]")),
        "the fixture must redact something"
    );
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests.len() * 3 < single_requests,
        "{} vs {single_requests}",
        requests.len()
    );
    assert!(
        requests
            .iter()
            .all(|r| r.body.len() <= MAX_CLASSIFY_REQUEST_BODY_BYTES)
    );
    assert!(requests.iter().any(|r| {
        let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
        body["input"].as_array().is_some_and(|a| a.len() > 1)
    }));
}

/// Packing respects the body cap, not only the item limit: with a small cap
/// many texts that the item limit would put together go out in several
/// requests, each under the cap.
#[tokio::test]
async fn batches_are_packed_under_the_body_cap() {
    const CAP: usize = 2 * 1024;
    let texts: Vec<String> = (0..60)
        .map(|i| format!("line {i} \"quoted\" \\ bob@example.com {}", "x".repeat(200)))
        .collect();
    let server = server_with(per_input_emails(usize::MAX, false)).await;
    let adapter = adapter_with_cap(server.uri(), CAP).with_max_batch_inputs(50);
    let answers = answers_of(&adapter, &texts).await;
    assert!(answers.iter().all(Result::is_ok));
    let requests = server.received_requests().await.unwrap();
    assert!(requests.len() > 1);
    for request in &requests {
        assert!(request.body.len() <= CAP, "{} > {CAP}", request.body.len());
    }
}

/// A batch the endpoint refuses as too large is split until it is accepted,
/// and the answers are unchanged.
#[tokio::test]
async fn a_413_on_a_batch_splits_it() {
    let texts = mixed_texts();
    let reference_server = server_with(per_input_emails(usize::MAX, false)).await;
    let expected = answers_of(&adapter(reference_server.uri()), &texts).await;
    let server = server_with(per_input_emails(3, false)).await;
    let answers = answers_of(&batching_adapter(server.uri(), 16), &texts).await;
    assert_eq!(answers, expected);
}

/// A batch answered with the wrong number of entries is a failure for every
/// text it carried -- none of its answers is used -- while texts that went
/// alone are unaffected.
#[tokio::test]
async fn a_batch_with_a_missing_entry_fails_every_text_in_it() {
    let texts = mixed_texts();
    let server = server_with(per_input_emails(usize::MAX, true)).await;
    let answers = answers_of(&batching_adapter(server.uri(), 8), &texts).await;
    let failed = answers.iter().filter(|a| a.is_err()).count();
    assert!(failed > 0, "a short batch must fail");
    for (text, answer) in texts.iter().zip(&answers) {
        if text.trim().is_empty() {
            assert_eq!(answer, &Ok(None), "blank text never goes to the classifier");
        }
        if let Err(error) = answer {
            assert!(
                !error.contains("bob@example.com"),
                "no content in the error"
            );
        }
    }
}

/// A span in one batch entry that does not match its own input fails the
/// whole batch closed, as it fails a single request.
#[tokio::test]
async fn a_mismatched_span_in_one_entry_fails_the_batch() {
    let server = server_with(|req: &wiremock::Request| {
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        let n = body["input"].as_array().map_or(1, Vec::len);
        let mut data = vec![json!({"spans": []}); n];
        data[n - 1] = json!({"spans": [{"category": "private_person", "start": 0, "end": 2,
                                          "score": 0.9, "text": "zz"}]});
        ResponseTemplate::new(200).set_body_json(json!({ "data": data }))
    })
    .await;
    let texts: Vec<String> = (0..6).map(|i| format!("text number {i}")).collect();
    let answers = answers_of(&batching_adapter(server.uri(), 6), &texts).await;
    assert!(answers.iter().all(Result::is_err), "{answers:?}");
}

/// Every request is counted by outcome in the adapter's diagnostics
/// recorder -- retries included -- and successful ones contribute latency.
#[tokio::test]
async fn requests_are_counted_by_outcome() {
    use std::sync::Arc;
    use trace_commons_protocol::privacy_filter_near_ai::diagnostics::{Outcome, Recorder};

    let counted = |server: &MockServer, recorder: &Arc<Recorder>| {
        adapter(server.uri()).with_diagnostics(recorder.clone())
    };
    let recorder = Arc::new(Recorder::default());
    let ok = server_with(|_: &wiremock::Request| {
        ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "spans": [] }] }))
    })
    .await;
    counted(&ok, &recorder).redact_text("one").await.unwrap();
    counted(&ok, &recorder).redact_text("two").await.unwrap();
    for status in [401, 429, 400] {
        let server = server_with(move |_: &wiremock::Request| ResponseTemplate::new(status)).await;
        let _ = counted(&server, &recorder).redact_text("three").await;
    }
    let down = server_with(|_: &wiremock::Request| ResponseTemplate::new(503)).await;
    let _ = counted(&down, &recorder).redact_text("four").await;
    let garbled =
        server_with(|_: &wiremock::Request| ResponseTemplate::new(200).set_body_string("not json"))
            .await;
    let _ = counted(&garbled, &recorder).redact_text("five").await;

    assert_eq!(recorder.count(Outcome::Ok), 2);
    assert_eq!(recorder.count(Outcome::Auth), 1);
    // 429 is retried like a 5xx, so every attempt counts.
    assert_eq!(recorder.count(Outcome::Quota), MAX_CLASSIFY_ATTEMPTS as u64);
    assert_eq!(recorder.count(Outcome::OtherClientError), 1);
    assert_eq!(
        recorder.count(Outcome::ServerError),
        MAX_CLASSIFY_ATTEMPTS as u64
    );
    assert_eq!(recorder.count(Outcome::BodyShape), 1);
    let snapshot = recorder.snapshot();
    assert_eq!(snapshot.in_flight, 0);
    assert_eq!(snapshot.latency_ms["under_4_kib"].samples, 2);
}

/// The timeout grows with the body: a slow answer to a small request is
/// abandoned, the same delay on a larger request is waited for.
#[tokio::test]
async fn the_timeout_grows_with_the_request_body() {
    use trace_commons_protocol::privacy_filter_near_ai::call_timeout;
    // 200 ms + 200 ms/KiB: under 1 KiB waits 400 ms, 16 KiB 3.4 s.
    assert_eq!(call_timeout(200, 200, 100), Duration::from_millis(400));
    assert_eq!(
        call_timeout(200, 200, 16 * 1024),
        Duration::from_millis(3_400)
    );
    let server = server_with(|_: &wiremock::Request| {
        ResponseTemplate::new(200)
            .set_delay(Duration::from_millis(1_000))
            .set_body_json(json!({ "data": [{ "spans": [] }] }))
    })
    .await;
    let adapter = || {
        NearAiPrivacyFilterAdapter::new(
            server.uri(),
            "openai/privacy-filter",
            "test-api-key-do-not-leak",
            Duration::from_millis(200),
            1_000_000,
        )
        .expect("adapter builds")
        .with_timeout_per_kib_ms(200)
    };
    let small = adapter().redact_text("a short text").await;
    assert!(small.is_err(), "a 1 s answer must time out at 400 ms");
    assert!(small.unwrap_err().is_transient());
    let large = adapter().redact_text(&"x".repeat(16 * 1024)).await;
    assert!(large.is_ok(), "a 1 s answer is inside 3.4 s: {large:?}");
}

/// With a caller deadline, a retry that could not finish before it is not
/// started: a slow endpoint costs one attempt and a prompt transient
/// refusal, not three attempts run into the caller's own timeout.
#[tokio::test]
async fn retries_stop_when_the_deadline_cannot_fit_another_attempt() {
    let slow = || {
        ResponseTemplate::new(200)
            .set_delay(Duration::from_secs(10))
            .set_body_json(json!({ "data": [{ "spans": [] }] }))
    };
    let adapter = |server: &MockServer| {
        NearAiPrivacyFilterAdapter::new(
            server.uri(),
            "openai/privacy-filter",
            "test-api-key-do-not-leak",
            // Long enough that a loaded test machine still delivers each
            // request to the server before its attempt times out, so the
            // request counts below are exact.
            Duration::from_millis(1_500),
            1_000_000,
        )
        .expect("adapter builds")
        .with_timeout_per_kib_ms(0)
    };

    // Deadline 2.5 s out: the first attempt times out at 1.5 s; another
    // would need 250 ms of backoff plus 1.5 s, which does not fit.
    let server = server_with(move |_: &wiremock::Request| slow()).await;
    let started = std::time::Instant::now();
    let error = adapter(&server)
        .redact_text_before("a slow window", started + Duration::from_millis(2_500))
        .await
        .expect_err("the endpoint never answers in time");
    assert!(error.is_transient());
    // Counted, not timed: one request means no retry was started. The
    // elapsed bound only says it did not wait for the 10 s answer.
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);

    // No deadline: every attempt runs.
    let server = server_with(move |_: &wiremock::Request| slow()).await;
    let _ = adapter(&server).redact_text("a slow window").await;
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        MAX_CLASSIFY_ATTEMPTS
    );

    // A deadline that has already passed starts nothing.
    let server = server_with(move |_: &wiremock::Request| slow()).await;
    let past = std::time::Instant::now();
    assert!(
        adapter(&server)
            .redact_text_before("late", past)
            .await
            .is_err()
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 0);

    // The first attempt is cut at the deadline even when its own timeout is
    // longer.
    let server = server_with(move |_: &wiremock::Request| slow()).await;
    let long = NearAiPrivacyFilterAdapter::new(
        server.uri(),
        "openai/privacy-filter",
        "test-api-key-do-not-leak",
        Duration::from_secs(30),
        1_000_000,
    )
    .expect("adapter builds");
    let started = std::time::Instant::now();
    assert!(
        long.redact_text_before("cut short", started + Duration::from_millis(400))
            .await
            .is_err()
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

/// A batched request is one request to the diagnostics recorder, counted and
/// timed exactly as a single call is, with nothing left in flight.
#[tokio::test]
async fn a_batched_request_is_counted_in_diagnostics() {
    use std::sync::Arc;
    use trace_commons_protocol::privacy_filter_near_ai::diagnostics::{Outcome, Recorder};

    let recorder = Arc::new(Recorder::default());
    let server = server_with(per_input_emails(usize::MAX, false)).await;
    let adapter = batching_adapter(server.uri(), 8).with_diagnostics(recorder.clone());
    let texts: Vec<String> = (0..6)
        .map(|i| format!("mail bob@example.com about item {i}"))
        .collect();
    let answers = answers_of(&adapter, &texts).await;
    assert!(answers.iter().all(Result::is_ok), "{answers:?}");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "six texts in one batch");
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["input"].as_array().map(Vec::len), Some(6));
    assert_eq!(recorder.count(Outcome::Ok), 1);
    let snapshot = recorder.snapshot();
    assert_eq!(snapshot.in_flight, 0);
    let samples: usize = snapshot
        .latency_ms
        .values()
        .map(|bucket| bucket.samples)
        .sum();
    assert_eq!(samples, 1);
}

/// A batch answered with the wrong number of entries is recorded as
/// `body_shape`, the label the single path gives any malformed answer, and
/// nothing from it counts as ok.
#[tokio::test]
async fn a_batch_count_mismatch_is_recorded_as_body_shape() {
    use std::sync::Arc;
    use trace_commons_protocol::privacy_filter_near_ai::diagnostics::{
        BodyShape, Outcome, Recorder,
    };

    let recorder = Arc::new(Recorder::default());
    let server = server_with(per_input_emails(usize::MAX, true)).await;
    let adapter = batching_adapter(server.uri(), 8).with_diagnostics(recorder.clone());
    let texts: Vec<String> = (0..6).map(|i| format!("plain text {i}")).collect();
    let answers = answers_of(&adapter, &texts).await;
    assert!(answers.iter().all(Result::is_err), "{answers:?}");
    assert_eq!(recorder.count(Outcome::BodyShape), 1);
    assert_eq!(recorder.body_shape_count(BodyShape::EntryCountMismatch), 1);
    assert_eq!(recorder.count(Outcome::Ok), 0);
    assert_eq!(recorder.snapshot().in_flight, 0);
}

const ALICE_TEXT: &str = "email me at alice@example.com please";

/// A span whose offsets run past its input: NEAR AI answered one for a real
/// session on every request (2026-11-12). It used to pass verification, be
/// cached and counted `ok`, and then be refused when applied -- so every
/// later request for that window was refused from the cache with no
/// classifier call at all.
fn out_of_range_answer(inputs: usize) -> ResponseTemplate {
    let entry = json!({ "spans": [
        {"category": "private_email", "start": 12, "end": 900, "score": 0.99, "text": "alice@example.com"}
    ]});
    ResponseTemplate::new(200).set_body_json(json!({ "data": vec![entry; inputs] }))
}

#[tokio::test]
async fn an_out_of_range_span_is_refused_counted_and_never_cached() {
    use std::sync::Arc;
    use trace_commons_protocol::privacy_filter_near_ai::diagnostics::{Outcome, Recorder};

    let server = server_with(|_: &wiremock::Request| out_of_range_answer(1)).await;
    let recorder = Arc::new(Recorder::default());
    let adapter = adapter(server.uri()).with_diagnostics(recorder.clone());
    let error = adapter.redact_text(ALICE_TEXT).await.unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("privacy_filter_span_out_of_range"),
        "{message}"
    );
    assert!(!message.contains("alice"), "no content: {message}");
    assert_eq!(recorder.count(Outcome::BodyShape), 1);
    assert_eq!(recorder.count(Outcome::Ok), 0, "a refused answer is not ok");
    let requests = server.received_requests().await.unwrap().len();

    // The same window again: a fresh request, not a refusal from the cache.
    adapter.redact_text(ALICE_TEXT).await.unwrap_err();
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        2 * requests
    );

    // Batched: the entry refuses the whole batch, and nothing is cached.
    let batch_server = server_with(|req: &wiremock::Request| {
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        out_of_range_answer(body["input"].as_array().map_or(1, Vec::len))
    })
    .await;
    let batching = batching_adapter(batch_server.uri(), 8);
    let texts: Vec<String> = (0..3).map(|i| format!("{ALICE_TEXT} {i}")).collect();
    let first = answers_of(&batching, &texts).await;
    assert!(first.iter().all(Result::is_err), "{first:?}");
    let sent = batch_server.received_requests().await.unwrap().len();
    let second = answers_of(&batching, &texts).await;
    assert!(second.iter().all(Result::is_err), "{second:?}");
    assert_eq!(
        batch_server.received_requests().await.unwrap().len(),
        2 * sent,
        "the second pass asked again"
    );
}

/// A 200 that is malformed in the way `kind` names, for [`ALICE_TEXT`].
fn malformed_alice_answer(
    kind: trace_commons_protocol::privacy_filter_near_ai::diagnostics::BodyShape,
) -> ResponseTemplate {
    use trace_commons_protocol::privacy_filter_near_ai::diagnostics::BodyShape;
    match kind {
        BodyShape::ParseError => ResponseTemplate::new(200)
            .set_body_string("{not valid json")
            .insert_header("content-type", "application/json"),
        BodyShape::EmptyData => ResponseTemplate::new(200).set_body_json(json!({ "data": [] })),
        // Right offsets, wrong text: the answer disagrees with itself.
        BodyShape::SpanTextMismatch => ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "spans": [
                {"category": "private_email", "start": 12, "end": 29, "score": 0.99, "text": "alice@example.org"}
            ]}]
        })),
        BodyShape::SpanOutOfRange => out_of_range_answer(1),
        BodyShape::EntryCountMismatch => ResponseTemplate::new(200)
            .set_body_json(json!({ "data": [{"spans": []}, {"spans": []}] })),
    }
}

/// Each kind of malformed answer on a single window is counted once under
/// its own label and in the `body_shape` total, never as `ok`, and refused
/// without leaking content. (Entry-count mismatch exists only for a batch;
/// see `a_batch_count_mismatch_is_recorded_as_body_shape`.)
#[tokio::test]
async fn each_malformed_answer_is_counted_by_its_kind() {
    use std::sync::Arc;
    use trace_commons_protocol::privacy_filter_near_ai::diagnostics::{
        BodyShape, Outcome, Recorder,
    };
    let single_window = [
        BodyShape::ParseError,
        BodyShape::EmptyData,
        BodyShape::SpanTextMismatch,
        BodyShape::SpanOutOfRange,
    ];
    for kind in single_window {
        let server = server_with(move |_: &wiremock::Request| malformed_alice_answer(kind)).await;
        let recorder = Arc::new(Recorder::default());
        let adapter = adapter(server.uri()).with_diagnostics(recorder.clone());
        let error = adapter.redact_text(ALICE_TEXT).await.unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains(&format!("body_shape={}", kind.label())),
            "{kind:?}: {message}"
        );
        assert!(!message.contains("alice"), "no content: {message}");
        assert_eq!(recorder.count(Outcome::BodyShape), 1, "{kind:?}");
        assert_eq!(recorder.count(Outcome::Ok), 0, "{kind:?}");
        for other in BodyShape::ALL {
            let expected = u64::from(other == kind);
            assert_eq!(
                recorder.body_shape_count(other),
                expected,
                "{kind:?}/{other:?}"
            );
        }
        let snapshot = recorder.snapshot();
        assert_eq!(snapshot.body_shapes[kind.label()], 1);
        assert_eq!(
            snapshot.outcomes["body_shape"],
            snapshot.body_shapes.values().sum::<u64>()
        );
        // Never cached: the same window is asked about again.
        let _ = adapter.redact_text(ALICE_TEXT).await.unwrap_err();
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            2,
            "{kind:?}"
        );
    }
}
