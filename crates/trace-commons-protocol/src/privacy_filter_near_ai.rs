//! NEAR AI Cloud hosted privacy-classifier backend for trace redaction.
//!
//! See docs/superpowers/specs/2026-05-19-near-ai-pii-redaction-design.md
//! for the contract this module implements.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
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
/// returns 502 for oversized requests (observed to fail above ~30 KiB), so
/// large field text is split into windows no bigger than this before it is
/// sent. Kept well under the observed ceiling to leave room for request
/// overhead and multibyte expansion.
pub const CLASSIFY_CHUNK_BYTES: usize = 20_000;

/// How many times a single `privacy/classify` request is attempted before
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

impl NearAiPrivacyFilterAdapter {
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
        timeout: Duration,
        max_input_bytes: usize,
    ) -> Result<Self, PrivacyFilterConfigError> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                var: "<reqwest client>",
                reason: err.to_string(),
            })?;
        Ok(Self {
            client,
            base_url: base_url.into(),
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
        let mut windows: Vec<(usize, Vec<ClassifySpan>)> = Vec::with_capacity(ranges.len());
        for range in ranges {
            let window = &text[range.clone()];
            let spans = self.classify_window(window).await?;
            let codepoint_start = text[..range.start].chars().count();
            windows.push((codepoint_start, spans));
        }

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
                    return Err(TraceContributionError::RedactionFailed {
                        reason: format!("near-ai privacy classifier transport error: {}", err),
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
                return Err(TraceContributionError::RedactionFailed {
                    reason: format!(
                        "near-ai privacy classifier returned non-2xx: status={} body_hash={} body_len={}",
                        status.as_u16(),
                        body_hash,
                        body_bytes.len()
                    ),
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

    fn span(category: &str, start: usize, end: usize, score: f64) -> ClassifySpan {
        ClassifySpan {
            category: category.into(),
            start,
            end,
            score,
        }
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
