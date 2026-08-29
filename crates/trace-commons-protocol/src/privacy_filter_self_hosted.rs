//! Loopback privacy-classifier backend serving `openai/privacy-filter`.
//!
//! Wire-compatible with the NEAR AI hosted backend by design: both speak
//! `POST {base}/privacy/classify` with `{model, input}` and receive
//! `{data:[{spans:[...]}]}`. That is what lets the two share `apply_spans`,
//! and what makes a shadow comparison between them a direct diff rather than
//! a translation.
//!
//! Everything the hosted adapter carries to cope with a WAN upstream is
//! deliberately absent here:
//!
//! - **No windowing.** The hosted endpoint serves a model reporting
//!   `context_length: 512` behind an internal splitter, and fails above
//!   ~3,000 input tokens, so that adapter budgets windows at 2,000 tokens and
//!   stitches the spans back together. Run locally, the model has its real
//!   128k context: one field, one request, no stitching.
//! - **No tokenizer.** Nothing needs to be measured before sending.
//! - **No window cache.** It exists to amortise a ~4.5 s round trip that
//!   loopback does not have.
//! - **No retry loop.** Retries paper over a flaky vendor; a local process
//!   that is down should surface as down.
//! - **No bearer token.** The transport never leaves the machine.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::privacy_filter_spans::{ClassifySpan, apply_spans};
use crate::trace_contribution::{
    PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES, PrivacyFilterAdapter, PrivacyFilterConfigError,
    SafePrivacyFilterRedaction, TraceContributionError,
};

/// Label used in error text and operational surfaces. Distinct from the
/// hosted backend's so a failure is never misattributed.
const BACKEND: &str = "self-hosted";

pub const DEFAULT_MODEL: &str = "openai/privacy-filter";

/// Generous compared with the hosted backend's 10 s.
///
/// One request now carries a whole field rather than a 2,000-token window, and
/// CPU inference is slower per call while needing far fewer calls. A timeout
/// sized for the hosted per-window round trip would abort legitimate work on a
/// large field.
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub struct SelfHostedPrivacyFilterAdapter {
    client: reqwest::Client,
    base_url: String,
    model: String,
    max_input_bytes: usize,
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

impl SelfHostedPrivacyFilterAdapter {
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
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
            max_input_bytes,
        })
    }
}

#[async_trait]
impl PrivacyFilterAdapter for SelfHostedPrivacyFilterAdapter {
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
                    "{BACKEND} privacy classifier input exceeded limit: input_len={} max_input_bytes={}",
                    text.len(),
                    self.max_input_bytes
                ),
            });
        }

        let endpoint = format!("{}/privacy/classify", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .post(&endpoint)
            .json(&ClassifyRequest {
                model: &self.model,
                input: text,
            })
            .send()
            .await
            .map_err(|err| TraceContributionError::TransientRedactionFailed {
                reason: format!("{BACKEND} privacy classifier transport error: {err}"),
            })?;

        let status = response.status();
        if !status.is_success() {
            // Same split the hosted adapter makes: a 5xx is the shim's
            // problem and must not be charged to the trace; a 4xx is our
            // misconfiguration and retrying it forever would hide the bug.
            let reason = format!("{BACKEND} privacy classifier returned {status}");
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
                    reason: format!("{BACKEND} privacy classifier response parse error: {err}"),
                })?;

        // Fail closed on a shape we do not understand. Returning Ok(None)
        // here would pass unredacted text through the control while
        // reporting success.
        let entry = parsed.data.into_iter().next().ok_or_else(|| {
            TraceContributionError::RedactionFailed {
                reason: format!("{BACKEND} privacy classifier returned an empty data array"),
            }
        })?;

        apply_spans(BACKEND, text, &entry.spans)
    }
}

pub fn build_from_env() -> Result<Arc<dyn PrivacyFilterAdapter>, PrivacyFilterConfigError> {
    let base_url = std::env::var("TRACE_PRIVACY_FILTER_SELF_HOSTED_BASE_URL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or(PrivacyFilterConfigError::MissingEnv {
            backend: "self-hosted",
            var: "TRACE_PRIVACY_FILTER_SELF_HOSTED_BASE_URL",
        })?;

    let model = std::env::var("TRACE_PRIVACY_FILTER_SELF_HOSTED_MODEL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());

    let timeout_ms = match std::env::var("TRACE_PRIVACY_FILTER_SELF_HOSTED_TIMEOUT_MS") {
        Ok(value) => {
            value
                .trim()
                .parse::<u64>()
                .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                    var: "TRACE_PRIVACY_FILTER_SELF_HOSTED_TIMEOUT_MS",
                    reason: err.to_string(),
                })?
        }
        Err(_) => DEFAULT_TIMEOUT_MS,
    };

    let max_input_bytes =
        match std::env::var("TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_INPUT_BYTES") {
            Ok(value) => value.trim().parse::<usize>().map_err(|err| {
                PrivacyFilterConfigError::InvalidEnv {
                    var: "TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_INPUT_BYTES",
                    reason: err.to_string(),
                }
            })?,
            Err(_) => PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES,
        };

    Ok(Arc::new(SelfHostedPrivacyFilterAdapter::new(
        base_url,
        model,
        Duration::from_millis(timeout_ms),
        max_input_bytes,
    )?))
}
