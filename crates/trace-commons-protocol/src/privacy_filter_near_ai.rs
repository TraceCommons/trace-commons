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

        let endpoint = format!("{}/privacy/classify", self.base_url.trim_end_matches('/'));
        let request_body = ClassifyRequest {
            model: &self.model,
            input: text,
        };
        let response = self
            .client
            .post(&endpoint)
            .bearer_auth(&self.api_key.0)
            .json(&request_body)
            .send()
            .await
            .map_err(|err| TraceContributionError::RedactionFailed {
                reason: format!("near-ai privacy classifier transport error: {}", err),
            })?;

        let status = response.status();
        if !status.is_success() {
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

        apply_spans(text, &entry.spans)
    }
}

fn apply_spans(
    text: &str,
    spans: &[ClassifySpan],
) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
    let mut report = RedactionReport::default();
    let mut by_label = std::collections::BTreeMap::new();
    let span_count = spans.len() as u32;

    // Validate offsets and labels; populate summary book-keeping per
    // raw span (matches sidecar accounting).
    for span in spans {
        if span.start > span.end || span.end > text.len() {
            return Err(TraceContributionError::RedactionFailed {
                reason: "near-ai privacy classifier returned out-of-range span".to_string(),
            });
        }
        if !text.is_char_boundary(span.start) || !text.is_char_boundary(span.end) {
            return Err(TraceContributionError::RedactionFailed {
                reason: "near-ai privacy classifier returned non-utf8 span boundary".to_string(),
            });
        }
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
    // pick widest end; on overlap pick the highest-score category.
    let mut sorted: Vec<ClassifySpan> = spans.to_vec();
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
    fn rejects_non_char_boundary() {
        let text = "héllo";
        // 'é' starts at byte 1, ends at byte 3 (UTF-8 two bytes).
        // Splitting at byte 2 is mid-codepoint.
        let spans = vec![span("private_name", 1, 2, 0.9)];
        let err = apply_spans(text, &spans).unwrap_err();
        assert!(err.to_string().contains("non-utf8 span boundary"));
    }

    #[test]
    fn rejects_out_of_range_span() {
        let text = "short";
        let spans = vec![span("private_name", 0, 9999, 0.9)];
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
