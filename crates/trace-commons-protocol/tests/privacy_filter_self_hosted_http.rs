#![cfg(feature = "self-hosted-privacy-filter")]

//! HTTP contract for the loopback privacy-classifier backend.
//!
//! The point of these tests is the difference from the hosted backend: the
//! window budget is a duration bound we choose rather than an upstream context
//! limit, so a large budget means one request for a whole field; there is no
//! bearer token; and the transient/permanent split on failure is typed.

use std::time::Duration;

use serde_json::json;
use trace_commons_protocol::privacy_filter_self_hosted::SelfHostedPrivacyFilterAdapter;
use trace_commons_protocol::trace_contribution::PrivacyFilterAdapter;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn adapter(base_url: String) -> SelfHostedPrivacyFilterAdapter {
    // A token budget large enough that the small fixtures below stay in one
    // window; the windowing tests set their own.
    adapter_with_token_budget(base_url, 1_000_000)
}

fn adapter_with_token_budget(
    base_url: String,
    max_input_tokens: usize,
) -> SelfHostedPrivacyFilterAdapter {
    SelfHostedPrivacyFilterAdapter::new(
        base_url,
        "openai/privacy-filter",
        Duration::from_secs(5),
        10_000_000,
        max_input_tokens,
    )
    .expect("adapter builds")
}

/// A field far larger than the hosted backend's 2,000-token budget goes up in
/// ONE request when the budget allows it.
///
/// The local model has a real 128k context, so windowing here is a duration
/// bound we choose, not a limit imposed on us. With a large budget a big field
/// is still a single request.
#[tokio::test]
async fn a_large_field_is_classified_in_a_single_request() {
    let server = MockServer::start().await;
    let filler = "The quick brown fox jumps over the lazy dog. ".repeat(4_000);
    let text = format!("{filler}contact bob@example.com today");

    // Codepoint offset of the email within the full field.
    let start = text.chars().count() - "bob@example.com today".chars().count();
    let end = start + "bob@example.com".chars().count();

    Mock::given(method("POST"))
        .and(path("/v1/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{
                "spans": [{
                    "category": "private_email",
                    "start": start,
                    "end": end,
                    "score": 0.99
                }]
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let out = adapter(format!("{}/v1", server.uri()))
        .redact_text(&text)
        .await
        .expect("classification succeeds")
        .expect("a redaction was produced");

    assert!(
        out.redacted_text.contains("[REDACTED:private_email]"),
        "the span should have been redacted"
    );
    assert!(
        !out.redacted_text.contains("bob@example.com"),
        "the email must not survive redaction"
    );
}

/// Loopback carries no credential. Sending one would be a secret written to a
/// socket that does not need it.
#[tokio::test]
async fn no_authorization_header_is_sent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"spans": []}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    adapter(format!("{}/v1", server.uri()))
        .redact_text("nothing sensitive here")
        .await
        .expect("classification succeeds");

    let requests = server
        .received_requests()
        .await
        .expect("the mock server recorded requests");
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].headers.get("authorization").is_none(),
        "a loopback classifier must not be sent a bearer token"
    );
}

/// Fail closed. A 5xx must never be reported as "no PII found", and it must be
/// typed transient so the driver does not charge it to the trace.
#[tokio::test]
async fn a_server_error_fails_closed_and_is_transient() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/privacy/classify"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let err = adapter(format!("{}/v1", server.uri()))
        .redact_text("contact bob@example.com")
        .await
        .expect_err("a 5xx must not be reported as a clean field");

    assert!(
        err.is_transient(),
        "an upstream 5xx is the shim's problem, not the trace's"
    );
}

/// A 4xx is our misconfiguration, not a passing outage, so it must NOT be
/// typed transient -- retrying it forever would hide the bug.
#[tokio::test]
async fn a_client_error_is_permanent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/privacy/classify"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&server)
        .await;

    let err = adapter(format!("{}/v1", server.uri()))
        .redact_text("contact bob@example.com")
        .await
        .expect_err("a 4xx must not be reported as a clean field");

    assert!(
        !err.is_transient(),
        "a 4xx is our misconfiguration and must not be retried as transient"
    );
}

/// An empty `data` array is a shape we do not understand. Treating it as "no
/// PII found" would silently pass unredacted text through the control.
#[tokio::test]
async fn an_empty_data_array_fails_closed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
        .mount(&server)
        .await;

    adapter(format!("{}/v1", server.uri()))
        .redact_text("contact bob@example.com")
        .await
        .expect_err("an empty data array must fail closed, not pass the text through");
}

/// The error text must name the backend that failed. The shared span decoder
/// is used by both backends, so an unqualified message would misattribute a
/// self-hosted failure to NEAR AI.
#[tokio::test]
async fn errors_name_the_self_hosted_backend() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"spans": [{
                "category": "private_email",
                "start": 0,
                "end": 9999,
                "score": 0.9
            }]}]
        })))
        .mount(&server)
        .await;

    let err = adapter(format!("{}/v1", server.uri()))
        .redact_text("short text")
        .await
        .expect_err("an out-of-range span must be refused");

    let message = err.to_string();
    assert!(
        message.contains("self-hosted"),
        "error should name the self-hosted backend, got: {message}"
    );
    assert!(
        !message.contains("near-ai"),
        "a self-hosted failure must not be attributed to near-ai, got: {message}"
    );
}

/// A field larger than the token budget is split, and the spans come back in
/// FULL-TEXT coordinates, not per-window ones.
///
/// This is the part that silently corrupts if it is wrong: a span reported at
/// window-local offsets would redact the right length at the wrong place, in
/// text the model already told us contains PII.
#[tokio::test]
async fn a_field_over_the_budget_is_split_and_spans_are_shifted_to_full_text() {
    let server = MockServer::start().await;

    // Two windows' worth at a deliberately tiny budget. The email sits in the
    // SECOND window, so a failure to shift offsets cannot accidentally pass.
    let head = "All quiet here with nothing of interest to see. ".repeat(60);
    let text = format!("{head}please contact bob@example.com now");

    // Respond per window: find the email in whatever window carries it and
    // report it at that window's own codepoint offsets.
    Mock::given(method("POST"))
        .and(path("/v1/privacy/classify"))
        .respond_with(|req: &wiremock::Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).expect("request body is JSON");
            let input = body["input"].as_str().expect("input is a string");
            match input.find("bob@example.com") {
                Some(byte_idx) => {
                    let start = input[..byte_idx].chars().count();
                    ResponseTemplate::new(200).set_body_json(json!({
                        "data": [{"spans": [{
                            "category": "private_email",
                            "start": start,
                            "end": start + "bob@example.com".chars().count(),
                            "score": 0.99
                        }]}]
                    }))
                }
                None => ResponseTemplate::new(200).set_body_json(json!({
                    "data": [{"spans": []}]
                })),
            }
        })
        .mount(&server)
        .await;

    let out = adapter_with_token_budget(format!("{}/v1", server.uri()), 64)
        .redact_text(&text)
        .await
        .expect("classification succeeds")
        .expect("a redaction was produced");

    let requests = server
        .received_requests()
        .await
        .expect("the mock server recorded requests");
    assert!(
        requests.len() > 1,
        "a field over the budget should be split, got {} request(s)",
        requests.len()
    );
    assert!(
        out.redacted_text.contains("[REDACTED:private_email]"),
        "the span should have been redacted"
    );
    assert!(
        !out.redacted_text.contains("bob@example.com"),
        "the email must not survive redaction"
    );
    // The untouched prose either side must be preserved exactly: a mis-shifted
    // span would eat some of it.
    assert!(
        out.redacted_text.starts_with("All quiet here"),
        "text before the span must be untouched"
    );
    assert!(
        out.redacted_text.ends_with(" now"),
        "text after the span must be untouched"
    );
}

/// Windowing must cover the whole field. A gap would be text the classifier
/// never saw, reported as clean.
#[tokio::test]
async fn every_window_is_sent_and_together_they_cover_the_field() {
    let server = MockServer::start().await;
    let text = "Lorem ipsum dolor sit amet consectetur adipiscing elit. ".repeat(80);

    Mock::given(method("POST"))
        .and(path("/v1/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"spans": []}]
        })))
        .mount(&server)
        .await;

    let out = adapter_with_token_budget(format!("{}/v1", server.uri()), 64)
        .redact_text(&text)
        .await
        .expect("classification succeeds")
        .expect("a redaction was produced");

    let requests = server
        .received_requests()
        .await
        .expect("the mock server recorded requests");
    assert!(requests.len() > 1, "expected the field to be split");

    let reassembled: String = requests
        .iter()
        .map(|r| {
            let body: serde_json::Value =
                serde_json::from_slice(&r.body).expect("request body is JSON");
            body["input"]
                .as_str()
                .expect("input is a string")
                .to_string()
        })
        .collect();
    assert_eq!(
        reassembled, text,
        "the windows must concatenate back to the original field with no gaps or overlap"
    );
    assert_eq!(
        out.redacted_text, text,
        "clean text must round-trip unchanged"
    );
}
