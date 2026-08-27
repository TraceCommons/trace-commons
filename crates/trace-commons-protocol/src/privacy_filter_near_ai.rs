//! NEAR AI Cloud hosted privacy-classifier backend for trace redaction.
//!
//! See docs/superpowers/specs/2026-05-19-near-ai-pii-redaction-design.md
//! for the contract this module implements.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};

use crate::trace_contribution::{
    PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES, PrivacyFilterAdapter, PrivacyFilterConfigError,
    RedactionReport, SafePrivacyFilterRedaction, SafePrivacyFilterSummary, TraceContributionError,
    safe_privacy_filter_label,
};

pub const DEFAULT_BASE_URL: &str = "https://cloud-api.near.ai/v1";
pub const DEFAULT_MODEL: &str = "openai/privacy-filter";
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;

/// Maximum input bytes per `privacy/classify` request. The hosted endpoint
/// returns 502 for oversized requests, so large field text is split into
/// windows no bigger than this before it is sent.
///
/// The ceiling is set by the vendor and has moved down over time. Measured
/// against the live endpoint on 2026-08-27, same key and model as production:
///
/// | input bytes | 200 OK |
/// |-------------|--------|
/// | 2,000       | 14/15  |
/// | 8,000       | 7/8    |
/// | 12,000      | 6/8    |
/// | 16,000      | 0/8    |
/// | 20,000      | 0/15   |
///
/// There is a hard cliff between 12 KiB and 16 KiB, and the band below it is
/// itself flaky. The value 20_000 sat *above* the cliff, so every request the
/// adapter made failed 100% of the time and the PII-backstop backlog could not
/// drain at all.
///
/// The first fix went to 4_000, which drained but only just: on the pilot a
/// single held trace took roughly eleven minutes, projecting to ~45 hours for
/// a 247-trace backlog, because chunk count scales inversely with this value
/// and each window is a separate round-trip. 8_000 measured 7/8 -- as reliable
/// as anything below the cliff -- and halves the request count, while still
/// leaving 2x headroom under the measured cliff for another vendor-side
/// tightening. That headroom is enforced below.
pub const CLASSIFY_CHUNK_BYTES: usize = 8_000;

/// The lowest input size measured to fail outright against the hosted
/// endpoint (0/8 successes on 2026-08-27). `CLASSIFY_CHUNK_BYTES` must stay
/// well below this; see the table above.
pub const MEASURED_CLASSIFY_FAILURE_CLIFF_BYTES: usize = 16_000;

// 2026-08-27 outage regression, enforced at compile time. The vendor's payload
// ceiling dropped below the configured chunk size, so every `privacy/classify`
// request the adapter made returned 502 and the PII-backstop backlog wedged at
// 248 held traces with 233 never attempted once. Raising CLASSIFY_CHUNK_BYTES
// back over the cliff must not compile.
const _: () = assert!(
    CLASSIFY_CHUNK_BYTES < MEASURED_CLASSIFY_FAILURE_CLIFF_BYTES,
    "CLASSIFY_CHUNK_BYTES must stay below the measured hard-failure cliff; \
     above it every classify request fails and the backstop cannot drain"
);
// Leave real headroom, not a single byte of it: the band just under the cliff
// was itself only 6/8 reliable when measured.
const _: () = assert!(
    CLASSIFY_CHUNK_BYTES * 2 <= MEASURED_CLASSIFY_FAILURE_CLIFF_BYTES,
    "CLASSIFY_CHUNK_BYTES leaves too little headroom under the measured cliff"
);

/// How many `privacy/classify` requests for a single field may be in flight
/// at once.
///
/// **Set to 1 deliberately.** #456 raised this to 8 and throughput collapsed:
/// on the pilot every PII-backstop tick then returned
/// `done=0 transient=3 breaker_tripped=true` and the queue drained nothing,
/// so the host was rolled back to the sequential build. Why concurrency hurt
/// is still not established -- a rate-limit theory did not survive testing
/// (20 rapid 8 KB requests all returned 200) -- so this stays at 1 until the
/// diagnostics below explain the failures. One window per request is also
/// what makes a failure attributable to specific content.
pub const MAX_CONCURRENT_CLASSIFY_WINDOWS: usize = 1;

/// How many times a single `privacy/classify` request is attempted before/// How many times a single `privacy/classify` request is attempted before
/// giving up. The hosted endpoint returns transient 502s, so retry a few
/// times with exponential backoff before failing the window closed.
pub const MAX_CLASSIFY_ATTEMPTS: usize = 4;

#[derive(Clone)]
struct SecretApiKey(String);

impl std::fmt::Debug for SecretApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretApiKey(***)")
    }
}

pub struct NearAiPrivacyFilterAdapter {
    client: reqwest::Client,
    base_url: String,
    model: String,
    api_key: SecretApiKey,
    max_input_bytes: usize,
}

/// True when `base_url` uses TLS, or is plain HTTP against a loopback host.
///
/// Loopback is exempt because such a request never leaves the machine: local
/// sidecars and the mock servers used in tests are the intended cases. Any
/// other plaintext endpoint would put the bearer API key on the wire.
fn base_url_is_tls_or_loopback(base_url: &str) -> bool {
    if base_url.starts_with("https://") {
        return true;
    }
    let Some(rest) = base_url.strip_prefix("http://") else {
        return false;
    };
    // Authority runs to the first path/query/fragment delimiter; any
    // userinfo before an `@` is not part of the host.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = match authority.strip_prefix('[') {
        // IPv6 literal: the host is what sits inside the brackets.
        Some(inner) => inner.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

impl NearAiPrivacyFilterAdapter {
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
        timeout: Duration,
        max_input_bytes: usize,
    ) -> Result<Self, PrivacyFilterConfigError> {
        let base_url = base_url.into();
        // The classify request carries the API key as a bearer token, so the
        // configured endpoint decides who receives a credential. Require TLS
        // unless the endpoint is loopback (local sidecars, test mock
        // servers), and refuse rather than shipping the key in plaintext.
        if !base_url_is_tls_or_loopback(&base_url) {
            return Err(PrivacyFilterConfigError::InvalidEnv {
                var: "TRACE_NEAR_AI_PRIVACY_BASE_URL",
                reason: "base URL must use https (or loopback http)".to_string(),
            });
        }
        let client = reqwest::Client::builder()
            .timeout(timeout)
            // No redirect following: a redirect would hand the bearer key to
            // a host that never passed the check above.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                var: "<reqwest client>",
                reason: err.to_string(),
            })?;
        Ok(Self {
            client,
            base_url,
            model: model.into(),
            api_key: SecretApiKey(api_key.into()),
            max_input_bytes,
        })
    }
}

pub fn build_from_env() -> Result<Arc<dyn PrivacyFilterAdapter>, PrivacyFilterConfigError> {
    let api_key = std::env::var("TRACE_NEAR_AI_PRIVACY_API_KEY")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or(PrivacyFilterConfigError::MissingEnv {
            backend: "near-ai",
            var: "TRACE_NEAR_AI_PRIVACY_API_KEY",
        })?;

    let base_url = std::env::var("TRACE_NEAR_AI_PRIVACY_BASE_URL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

    let model = std::env::var("TRACE_NEAR_AI_PRIVACY_MODEL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());

    let timeout_ms = match std::env::var("TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS") {
        Ok(value) => {
            value
                .trim()
                .parse::<u64>()
                .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                    var: "TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS",
                    reason: err.to_string(),
                })?
        }
        Err(_) => DEFAULT_TIMEOUT_MS,
    };

    let max_input_bytes =
        match std::env::var("TRACE_NEAR_AI_PRIVACY_MAX_INPUT_BYTES") {
            Ok(value) => value.trim().parse::<usize>().map_err(|err| {
                PrivacyFilterConfigError::InvalidEnv {
                    var: "TRACE_NEAR_AI_PRIVACY_MAX_INPUT_BYTES",
                    reason: err.to_string(),
                }
            })?,
            Err(_) => PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES,
        };

    let adapter = NearAiPrivacyFilterAdapter::new(
        base_url,
        model,
        api_key,
        Duration::from_millis(timeout_ms),
        max_input_bytes,
    )?;
    Ok(Arc::new(adapter))
}

#[derive(Serialize)]
struct ClassifyRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Deserialize)]
struct ClassifyResponse {
    data: Vec<ClassifyEntry>,
}

#[derive(Deserialize)]
struct ClassifyEntry {
    #[serde(default)]
    spans: Vec<ClassifySpan>,
}

#[derive(Deserialize, Clone)]
struct ClassifySpan {
    category: String,
    start: usize,
    end: usize,
    #[serde(default)]
    score: f64,
}

#[async_trait]
impl PrivacyFilterAdapter for NearAiPrivacyFilterAdapter {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        if text.len() > self.max_input_bytes {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!(
                    "near-ai privacy classifier input exceeded limit: input_len={} max_input_bytes={}",
                    text.len(),
                    self.max_input_bytes
                ),
            });
        }

        // The hosted endpoint rejects oversized requests, so split large
        // field text into windows and classify each. Every window's spans
        // are reported in that window's own codepoint coordinates; shift
        // them into full-text codepoints before merging so the single
        // apply_spans pass validates and redacts against the whole field.
        let ranges = chunk_byte_ranges(text, CLASSIFY_CHUNK_BYTES);

        // Accumulate each window's starting codepoint in ONE pass over the
        // field. This used to be `text[..range.start].chars().count()` inside
        // the classification loop, which rescans the whole prefix per window
        // and is quadratic in field length -- ~60 MB of redundant scanning on
        // a 971 kB field, far worse on the 16 MB envelopes the pilot holds.
        // `chunk_byte_ranges` returns contiguous ranges covering the text from
        // 0, so a running total is exact.
        let mut codepoint_starts = Vec::with_capacity(ranges.len());
        let mut codepoints_so_far = 0usize;
        for range in &ranges {
            codepoint_starts.push(codepoints_so_far);
            codepoints_so_far += text[range.clone()].chars().count();
        }

        // Classify the windows concurrently. They are independent -- each is
        // classified in its own coordinates and merged afterwards -- so the
        // sequential loop this replaces made a field cost
        // (windows x round-trip) for no reason. With pilot envelopes at a
        // median 421 kB that was 50+ serialized requests per field, and the
        // PII-backstop backlog drained at roughly nine minutes per trace.
        //
        // `buffered` preserves stream order, so `windows` is still ordered by
        // range and `try_collect` surfaces the FIRST window's error in field
        // order -- the same error the sequential loop would have returned.
        // That matters: transient-vs-permanent classification drives whether
        // the trace's attempt budget is charged.
        let windows: Vec<(usize, Vec<ClassifySpan>)> =
            futures::stream::iter(ranges.into_iter().zip(codepoint_starts).map(
                |(range, codepoint_start)| {
                    let window = &text[range];
                    async move {
                        self.classify_window(window)
                            .await
                            .map(|spans| (codepoint_start, spans))
                    }
                },
            ))
            .buffered(MAX_CONCURRENT_CLASSIFY_WINDOWS)
            .try_collect()
            .await?;

        apply_windowed_spans(text, &windows)
    }
}

impl NearAiPrivacyFilterAdapter {
    /// POST one window of text to the classifier and return its raw spans
    /// (in that window's codepoint coordinates). Fail-closed on any
    /// transport error, non-2xx status, malformed body, or empty data
    /// array.
    async fn classify_window(
        &self,
        text: &str,
    ) -> Result<Vec<ClassifySpan>, TraceContributionError> {
        let endpoint = format!("{}/privacy/classify", self.base_url.trim_end_matches('/'));
        let request_body = ClassifyRequest {
            model: &self.model,
            input: text,
        };

        let mut attempt = 0;
        loop {
            attempt += 1;
            // Transient failures (transport errors, 5xx) are retried with
            // exponential backoff; 4xx and body-shape failures are not.
            let send_result = self
                .client
                .post(&endpoint)
                .bearer_auth(&self.api_key.0)
                .json(&request_body)
                .send()
                .await;
            let response = match send_result {
                Ok(response) => response,
                Err(err) => {
                    if attempt < MAX_CLASSIFY_ATTEMPTS {
                        backoff(attempt).await;
                        continue;
                    }
                    // Retries are spent and the request never reached the
                    // classifier: the upstream is down, the trace is fine.
                    // The input fingerprint rides along hash-only so a
                    // repeatedly-failing window is identifiable in the logs.
                    let diagnostics = classify_input_diagnostics(&[text]);
                    // The driver logs only a hash of the error, so emit the
                    // input fingerprint here where it is provably hash-only.
                    tracing::warn!(
                        classify_input = %diagnostics,
                        attempts = attempt,
                        failure = "transport",
                        "near-ai privacy classify failed"
                    );
                    return Err(TraceContributionError::TransientRedactionFailed {
                        reason: format!(
                            "near-ai privacy classifier transport error: {} attempts={} {}",
                            err, attempt, diagnostics
                        ),
                    });
                }
            };

            let status = response.status();
            if !status.is_success() {
                if status.is_server_error() && attempt < MAX_CLASSIFY_ATTEMPTS {
                    backoff(attempt).await;
                    continue;
                }
                // Hash the body for audit; do not include it verbatim.
                let body_bytes = response.bytes().await.unwrap_or_default();
                let body_hash = format!(
                    "sha256:{}",
                    hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&body_bytes))
                );
                // Same split the retry decision above makes, carried out to
                // the caller: a 5xx that outlived our retries is the vendor's
                // problem, anything else (4xx) is ours or the trace's.
                let diagnostics = classify_input_diagnostics(&[text]);
                tracing::warn!(
                    classify_input = %diagnostics,
                    attempts = attempt,
                    status = status.as_u16(),
                    body_hash = %body_hash,
                    body_len = body_bytes.len(),
                    failure = "status",
                    "near-ai privacy classify failed"
                );
                let reason = format!(
                    "near-ai privacy classifier returned non-2xx: status={} body_hash={} \
                     body_len={} attempts={} {}",
                    status.as_u16(),
                    body_hash,
                    body_bytes.len(),
                    attempt,
                    diagnostics
                );
                return Err(if status.is_server_error() {
                    TraceContributionError::TransientRedactionFailed { reason }
                } else {
                    TraceContributionError::RedactionFailed { reason }
                });
            }

            let parsed: ClassifyResponse =
                response
                    .json()
                    .await
                    .map_err(|err| TraceContributionError::RedactionFailed {
                        reason: format!("near-ai privacy classifier response parse error: {}", err),
                    })?;
            let entry =
                parsed
                    .data
                    .into_iter()
                    .next()
                    .ok_or(TraceContributionError::RedactionFailed {
                        reason: "near-ai privacy classifier returned empty data array".to_string(),
                    })?;
            return Ok(entry.spans);
        }
    }
}

/// Hash-only description of the input a failing classify request carried.
///
/// Every synthetic probe of this endpoint succeeds while the driver's real
/// traffic fails, so the failures depend on something about the actual window
/// content that cannot be reproduced from outside. This records enough to
/// identify the offending window -- its size and a stable fingerprint -- while
/// disclosing none of it.
///
/// Content-derived values are SHA-256 prefixes, never the text: these strings
/// reach operational logs, where the repo's rule is hash-only or label-only.
fn classify_input_diagnostics(windows: &[&str]) -> String {
    let parts: Vec<String> = windows
        .iter()
        .enumerate()
        .map(|(index, window)| {
            let digest = <sha2::Sha256 as sha2::Digest>::digest(window.as_bytes());
            format!(
                "{index}:bytes={},chars={},sha256={}",
                window.len(),
                window.chars().count(),
                &hex::encode(digest)[..16]
            )
        })
        .collect();
    format!("inputs={} [{}]", windows.len(), parts.join(" "))
}

/// Exponential backoff before retrying a classify attempt: 250ms, 500ms,
/// 1s, ... keyed on the just-failed attempt number (1-based).
async fn backoff(failed_attempt: usize) {
    let millis = 250u64.saturating_mul(1u64 << (failed_attempt.saturating_sub(1)).min(5));
    tokio::time::sleep(Duration::from_millis(millis)).await;
}

/// Split `text` into contiguous byte ranges each no larger than `max_bytes`,
/// always on char boundaries and covering the whole input. Windows prefer to
/// end at a newline within the limit (PII rarely spans lines); a run with no
/// newline under the limit is hard-split at the nearest lower char boundary.
fn chunk_byte_ranges(text: &str, max_bytes: usize) -> Vec<std::ops::Range<usize>> {
    let max_bytes = max_bytes.max(1);
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < text.len() {
        if text.len() - start <= max_bytes {
            ranges.push(start..text.len());
            break;
        }
        // Provisional hard cap, walked back to a char boundary.
        let mut end = start + max_bytes;
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        // Prefer to break just after the last newline inside the window.
        if let Some(nl) = text[start..end].rfind('\n') {
            end = start + nl + 1;
        }
        // Guard against no progress (e.g. a single multibyte char wider
        // than the char-boundary walk left us): force at least one char.
        if end <= start {
            end = start + max_bytes;
            while end < text.len() && !text.is_char_boundary(end) {
                end += 1;
            }
        }
        ranges.push(start..end);
        start = end;
    }
    if ranges.is_empty() {
        ranges.push(0..text.len());
    }
    ranges
}

/// Merge per-window spans into a single redaction over `text`. Each window
/// carries its starting codepoint index; its spans are reported relative to
/// that window, so shift them into full-text codepoint coordinates before
/// the shared `apply_spans` validation/redaction pass.
fn apply_windowed_spans(
    text: &str,
    windows: &[(usize, Vec<ClassifySpan>)],
) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
    let mut all_spans = Vec::new();
    for (codepoint_start, spans) in windows {
        for span in spans {
            all_spans.push(ClassifySpan {
                category: span.category.clone(),
                start: codepoint_start + span.start,
                end: codepoint_start + span.end,
                score: span.score,
            });
        }
    }
    apply_spans(text, &all_spans)
}

fn apply_spans(
    text: &str,
    spans: &[ClassifySpan],
) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
    let mut report = RedactionReport::default();
    let mut by_label = std::collections::BTreeMap::new();
    let span_count = spans.len() as u32;

    // NEAR AI reports span offsets as Unicode codepoint indices, not byte
    // offsets. Build a codepoint -> byte-offset table once so we can both
    // validate the offsets and translate them before any byte slicing.
    // `boundaries[i]` is the byte offset of codepoint `i`; the final entry
    // is `text.len()`, so a valid end index is `<= boundaries.len() - 1`.
    let mut boundaries: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    boundaries.push(text.len());

    // Validate offsets and labels; populate summary book-keeping per raw
    // span (matches sidecar accounting). Offsets are converted from
    // codepoint indices to byte offsets here.
    let mut byte_spans: Vec<ClassifySpan> = Vec::with_capacity(spans.len());
    for span in spans {
        if span.start > span.end || span.end >= boundaries.len() {
            return Err(TraceContributionError::RedactionFailed {
                reason: "near-ai privacy classifier returned out-of-range span".to_string(),
            });
        }
        byte_spans.push(ClassifySpan {
            category: span.category.clone(),
            start: boundaries[span.start],
            end: boundaries[span.end],
            score: span.score,
        });
        let label = safe_privacy_filter_label(Some(&span.category), &mut report);
        *by_label.entry(label.clone()).or_insert(0u32) += 1;
        report.increment(format!("privacy_filter:{label}"));
        if label.eq_ignore_ascii_case("secret") {
            report.blocked_secret_detected = true;
        }
        if !report.pii_labels_present.contains(&label) {
            report.pii_labels_present.push(label);
        }
    }

    // Build redacted text. Collapse overlapping spans: sort by start,
    // pick widest end; on overlap pick the highest-score category. Offsets
    // below are byte offsets (already translated from codepoint indices).
    let mut sorted: Vec<ClassifySpan> = byte_spans;
    sorted.sort_by_key(|s| (s.start, std::cmp::Reverse(s.end)));

    let mut collapsed: Vec<ClassifySpan> = Vec::new();
    for span in sorted {
        match collapsed.last_mut() {
            Some(prev) if span.start < prev.end => {
                if span.end > prev.end {
                    prev.end = span.end;
                }
                if span.score > prev.score {
                    prev.category = span.category;
                    prev.score = span.score;
                }
            }
            _ => collapsed.push(span),
        }
    }

    let mut redacted_text = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let mut dummy_report = RedactionReport::default();
    for span in &collapsed {
        redacted_text.push_str(&text[cursor..span.start]);
        let label = safe_privacy_filter_label(Some(&span.category), &mut dummy_report);
        redacted_text.push_str(&format!("[REDACTED:{label}]"));
        cursor = span.end;
    }
    redacted_text.push_str(&text[cursor..]);

    Ok(Some(SafePrivacyFilterRedaction {
        redacted_text,
        summary: SafePrivacyFilterSummary {
            schema_version: 1,
            output_mode: "redacted_text_only".to_string(),
            span_count,
            by_label,
            decoded_mismatch: false,
        },
        report,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The classify call sends the API key as a bearer token, so a plaintext
    /// non-loopback endpoint would put it on the wire. Loopback stays
    /// allowed — that is what the wiremock-backed tests use.
    #[test]
    fn adapter_refuses_plaintext_non_loopback_base_url() {
        let build = |base_url: &str| {
            NearAiPrivacyFilterAdapter::new(
                base_url,
                "openai/privacy-filter",
                "test-api-key-do-not-leak",
                Duration::from_secs(5),
                1_000_000,
            )
        };
        for rejected in [
            "http://near-ai.example.com",
            "http://127.0.0.1.evil.example.com",
            "ftp://near-ai.example.com",
        ] {
            assert!(
                build(rejected).is_err(),
                "plaintext or non-http base URL must be refused: {rejected}"
            );
        }
        for allowed in [
            "https://near-ai.example.com",
            "http://127.0.0.1:8080",
            "http://localhost:8080",
        ] {
            assert!(
                build(allowed).is_ok(),
                "tls or loopback base URL must be accepted: {allowed}"
            );
        }
    }

    fn span(category: &str, start: usize, end: usize, score: f64) -> ClassifySpan {
        ClassifySpan {
            category: category.into(),
            start,
            end,
            score,
        }
    }

    /// The diagnostics exist to identify a failing window in operational
    /// logs, so they must carry size and a stable fingerprint -- and none of
    /// the text. This is the hash-only rule at the one place content could
    /// leak into a log line.
    #[test]
    fn classify_diagnostics_fingerprint_without_disclosing_content() {
        let secret = "contact alice@example.com about sk-live-000111222333";
        let diagnostics = classify_input_diagnostics(&[secret]);

        assert!(
            diagnostics.contains(&format!("bytes={}", secret.len())),
            "diagnostics must record the window size: {diagnostics}"
        );
        assert!(
            diagnostics.contains("sha256="),
            "diagnostics must record a fingerprint: {diagnostics}"
        );
        for leaked in [
            secret,
            "alice@example.com",
            "sk-live-000111222333",
            "contact",
        ] {
            assert!(
                !diagnostics.contains(leaked),
                "diagnostics leaked {leaked:?}: {diagnostics}"
            );
        }
    }

    /// The fingerprint has to be stable for the same window and different for
    /// different ones, or it cannot answer the question it exists for: is one
    /// window failing repeatedly, or many different windows failing once?
    #[test]
    fn classify_diagnostics_are_stable_per_window_and_distinct_across_windows() {
        let a = classify_input_diagnostics(&["window content one"]);
        let a_again = classify_input_diagnostics(&["window content one"]);
        let b = classify_input_diagnostics(&["window content two"]);

        assert_eq!(a, a_again, "same window must fingerprint identically");
        assert_ne!(a, b, "different windows must fingerprint differently");
    }

    /// Multibyte text: the byte length and the character count are both
    /// recorded because a window can be under the byte cap while carrying far
    /// fewer characters, and the endpoint's limits are not obviously in either
    /// unit.
    #[test]
    fn classify_diagnostics_record_bytes_and_chars_separately() {
        let text = "héllo wörld";
        let diagnostics = classify_input_diagnostics(&[text]);
        assert!(diagnostics.contains(&format!("bytes={}", text.len())));
        assert!(diagnostics.contains(&format!("chars={}", text.chars().count())));
        assert_ne!(
            text.len(),
            text.chars().count(),
            "test text must actually be multibyte"
        );
    }

    #[test]
    fn empty_input_short_circuits() {
        // Cannot call redact_text without a client; test apply_spans
        // covers the inner behavior. Empty-text short-circuit is in
        // redact_text proper, exercised by integration tests in Task 8.
    }

    #[test]
    fn replaces_single_span() {
        let text = "email me at alice@example.com please";
        let spans = vec![span("private_email", 12, 29, 0.99)];
        let result = apply_spans(text, &spans).unwrap().unwrap();
        assert_eq!(
            result.redacted_text,
            "email me at [REDACTED:private_email] please"
        );
        assert_eq!(result.summary.span_count, 1);
        assert_eq!(result.summary.by_label.get("private_email"), Some(&1));
    }

    #[test]
    fn chunk_byte_ranges_cover_text_and_respect_limit() {
        let text = "line one\nline two\nline three\n";
        let ranges = chunk_byte_ranges(text, 12);
        assert_eq!(ranges.first().unwrap().start, 0);
        assert_eq!(ranges.last().unwrap().end, text.len());
        let mut prev = 0;
        for r in &ranges {
            assert_eq!(r.start, prev, "ranges must be contiguous");
            assert!(r.end - r.start <= 12, "range {r:?} exceeds limit");
            assert!(text.is_char_boundary(r.start) && text.is_char_boundary(r.end));
            prev = r.end;
        }
        let joined: String = ranges.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(joined, text, "windows must reconstruct the input");
        // Newline-preferring: no window splits mid-line here.
        for r in &ranges {
            assert!(text[r.clone()].ends_with('\n') || r.end == text.len());
        }
    }

    #[test]
    fn chunk_byte_ranges_hard_splits_a_long_unbroken_run() {
        // A single line longer than the limit must still be split, on a
        // char boundary, into covering windows.
        let text = "café".repeat(10); // 50 bytes, no newline, multibyte
        let ranges = chunk_byte_ranges(&text, 8);
        let mut prev = 0;
        for r in &ranges {
            assert_eq!(r.start, prev);
            assert!(r.end - r.start <= 8);
            assert!(text.is_char_boundary(r.start) && text.is_char_boundary(r.end));
            prev = r.end;
        }
        assert_eq!(prev, text.len());
    }

    #[test]
    fn windowed_spans_shift_into_full_text_codepoints() {
        // Two windows over multibyte text. Window 2 starts at codepoint 4
        // ("café") and reports an email at window-local codepoints 1..16.
        let text = "café bob@example.com!";
        let windows = vec![
            (0usize, vec![]),
            (4usize, vec![span("private_email", 1, 16, 0.99)]),
        ];
        let result = apply_windowed_spans(text, &windows).unwrap().unwrap();
        assert_eq!(result.redacted_text, "café [REDACTED:private_email]!");
        assert_eq!(result.summary.span_count, 1);
    }

    #[test]
    fn maps_codepoint_offsets_over_multibyte_text() {
        // NEAR AI returns Unicode codepoint offsets, not byte offsets. When
        // multibyte characters precede the span, treating the offsets as
        // byte indices slices the wrong region. Offsets below are codepoint
        // indices exactly as the API emits them ('cafe' with an accented e
        // plus an emoji before the email).
        let text = "café 😀 reach me jane@example.com now";
        let spans = vec![span("private_email", 15, 32, 0.99)];
        let result = apply_spans(text, &spans).unwrap().unwrap();
        assert_eq!(
            result.redacted_text,
            "café 😀 reach me[REDACTED:private_email] now"
        );
    }

    #[test]
    fn collapses_overlapping_spans_keeps_highest_score() {
        let text = "abcdefghij";
        let spans = vec![
            span("private_email", 1, 5, 0.4),
            span("private_phone", 3, 7, 0.9),
        ];
        let result = apply_spans(text, &spans).unwrap().unwrap();
        assert_eq!(result.redacted_text, "a[REDACTED:private_phone]hij");
        // span_count is raw, even though only one collapsed redaction.
        assert_eq!(result.summary.span_count, 2);
    }

    #[test]
    fn redacts_multibyte_codepoint_span_without_splitting() {
        // Codepoint indices always land on character boundaries, so a span
        // over a multibyte character redacts the whole character. 'héllo':
        // 'é' is codepoint index 1.
        let text = "héllo";
        let spans = vec![span("private_name", 1, 2, 0.9)];
        let result = apply_spans(text, &spans).unwrap().unwrap();
        assert_eq!(result.redacted_text, "h[REDACTED:private_name]llo");
    }

    #[test]
    fn rejects_out_of_range_span() {
        // Codepoint index 9999 is far beyond the 5-codepoint string.
        let text = "short";
        let spans = vec![span("private_name", 0, 9999, 0.9)];
        let err = apply_spans(text, &spans).unwrap_err();
        assert!(err.to_string().contains("out-of-range"));
    }

    #[test]
    fn rejects_out_of_range_span_over_multibyte_text() {
        // 'café' is 4 codepoints; index 5 is out of range even though the
        // byte length is 5 (the trailing accented byte must not be treated
        // as a valid codepoint index).
        let text = "café";
        let spans = vec![span("private_name", 0, 5, 0.9)];
        let err = apply_spans(text, &spans).unwrap_err();
        assert!(err.to_string().contains("out-of-range"));
    }

    #[test]
    fn unknown_category_maps_to_unknown_with_warning() {
        let text = "secret-text";
        let spans = vec![span("brand_new_category", 0, 6, 0.5)];
        let result = apply_spans(text, &spans).unwrap().unwrap();
        assert_eq!(result.redacted_text, "[REDACTED:unknown]-text");
        assert!(
            result
                .report
                .warnings
                .iter()
                .any(|w| w.to_lowercase().contains("unsupported"))
        );
    }

    #[test]
    fn known_categories_land_in_allowlist() {
        let table = [
            "private_email",
            "private_phone",
            "account_number",
            "private_address",
            "private_name",
            "secret",
        ];
        for raw in table {
            let mut r = RedactionReport::default();
            let label = safe_privacy_filter_label(Some(raw), &mut r);
            assert_eq!(label, raw, "{raw} should pass through allow-list");
        }
    }

    #[test]
    fn debug_does_not_leak_api_key() {
        let secret = SecretApiKey("super-secret-token".into());
        let debug = format!("{secret:?}");
        assert!(!debug.contains("super-secret-token"));
        assert!(debug.contains("***"));
    }
}
