//! Loopback privacy-classifier backend serving `openai/privacy-filter`.
//!
//! Wire-compatible with the NEAR AI hosted backend by design: both speak
//! `POST {base}/privacy/classify` with `{model, input}` and receive
//! `{data:[{spans:[...]}]}`. That is what lets the two share `apply_spans`,
//! and what makes a shadow comparison between them a direct diff rather than
//! a translation.
//!
//! Windowing here is bounded by TIME, not by an upstream context limit -- the
//! difference that matters:
//!
//! - The hosted endpoint serves a model reporting `context_length: 512` behind
//!   an internal splitter and fails above ~3,000 input tokens, so that adapter
//!   MUST window at 2,000 tokens or the request errors.
//! - The local model has its real 128k context, so a whole field would fit in
//!   one request. But CPU inference is linear in input length and slow --
//!   measured at ~58 characters/second on a c3-standard-4 -- so a large field
//!   in one request runs for many minutes and trips the client timeout.
//!
//! So we still window, at a much larger budget, chosen so one window completes
//! comfortably inside the configured timeout. Splitting does not reduce total
//! work; it bounds per-request duration and keeps large fields covered instead
//! of failing them closed.
//!
//! Windows are issued with bounded concurrency. The hosted backend pins this
//! at 1 because raising it collapsed throughput against the vendor; that
//! finding is about NEAR AI's service and does not transfer to a local
//! process. Measured on the pilot, against the running backstop: 2 concurrent
//! windows gave 1.32x and 3 gave 1.58x. The headroom exists because this
//! model's `hidden_size` is 640, so individual matmuls are too small to scale
//! across threads -- one request saturates at ~1.75 of 4 cores, and the rest
//! is only reachable by overlapping requests.
//!
//! Still absent, because they exist only to cope with a WAN upstream:
//!
//! - **No window cache.** It amortises a ~4.5 s round trip loopback lacks.
//! - **No retry loop.** Retries paper over a flaky vendor; a local process
//!   that is down should surface as down.
//! - **No bearer token.** The transport never leaves the machine.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};

use crate::privacy_filter_spans::{ClassifySpan, apply_windowed_spans, chunk_token_ranges};
use crate::trace_contribution::{
    PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES, PrivacyFilterAdapter, PrivacyFilterConfigError,
    SafePrivacyFilterRedaction, TraceContributionError,
};

/// Label used in error text and operational surfaces. Distinct from the
/// hosted backend's so a failure is never misattributed.
const BACKEND: &str = "self-hosted";

pub const DEFAULT_MODEL: &str = "openai/privacy-filter";

/// Input tokens per request.
///
/// Not an upstream limit -- the local model's context is 128k and it would
/// accept the whole field. This is a DURATION budget: CPU inference runs at
/// roughly 58 characters/second, so ~4,000 tokens (~16 KB of prose) is about
/// five minutes, comfortably inside the default timeout while keeping the
/// number of windows low.
///
/// Raise it only alongside the timeout, and only with a measurement: the two
/// are a pair, and a budget that outruns the timeout fails every large field.
pub const DEFAULT_MAX_INPUT_TOKENS: usize = 4_000;

/// How many windows for a single field may be in flight at once.
///
/// **Deliberately not 1**, unlike the hosted backend. That backend's limit is
/// a scar from #456, where concurrency against NEAR AI collapsed pilot
/// throughput to zero; the cause was never established but it was the vendor's
/// service, not the client. A loopback process has none of that.
///
/// 3 rather than 2 or 4: measured on the pilot under load, 2 concurrent
/// windows gave 1.32x and 3 gave 1.58x, with the gain already flattening. The
/// ceiling is the box's 4 cores, and the backstop shares them with ingest.
pub const DEFAULT_MAX_CONCURRENT_WINDOWS: usize = 3;

/// Per-window timeout. Generous compared with the hosted backend's 10 s.
///
/// A window is up to `DEFAULT_MAX_INPUT_TOKENS`, which at measured CPU speed
/// is about five minutes. Ten minutes leaves headroom for a slower host or a
/// token-dense window without aborting legitimate work.
pub const DEFAULT_TIMEOUT_MS: u64 = 600_000;

pub struct SelfHostedPrivacyFilterAdapter {
    client: reqwest::Client,
    base_url: String,
    model: String,
    max_input_bytes: usize,
    max_input_tokens: usize,
    max_concurrent_windows: usize,
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
        max_input_tokens: usize,
        max_concurrent_windows: usize,
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
            max_input_tokens: max_input_tokens.max(1),
            max_concurrent_windows: max_concurrent_windows.max(1),
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

        // Split into windows sized by DURATION, not by any upstream limit.
        // Sequential on purpose: the shim is one process on shared cores, and
        // issuing windows concurrently would contend with itself (and with
        // ingest and the embedder) rather than finishing sooner.
        let ranges = chunk_token_ranges(text, self.max_input_tokens);

        // Each window's spans come back in that window's own codepoint
        // coordinates, so record where each window starts in codepoints and
        // let apply_windowed_spans shift them into full-text coordinates.
        let mut codepoint_starts = Vec::with_capacity(ranges.len());
        let mut seen = 0usize;
        for range in &ranges {
            codepoint_starts.push(seen);
            seen += text[range.clone()].chars().count();
        }

        // `buffered` preserves input order, so the (codepoint_start, spans)
        // pairs stay aligned with their windows regardless of completion
        // order. Getting that wrong would shift every span in the field.
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
            .buffered(self.max_concurrent_windows)
            .try_collect()
            .await?;

        apply_windowed_spans(BACKEND, text, &windows)
    }
}

impl SelfHostedPrivacyFilterAdapter {
    /// POST one window and return its spans, in that window's own codepoint
    /// coordinates. Fail-closed on any transport error, non-2xx, malformed
    /// body, or empty data array.
    async fn classify_window(
        &self,
        text: &str,
    ) -> Result<Vec<ClassifySpan>, TraceContributionError> {
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

        // Fail closed on a shape we do not understand. Returning no spans
        // here would pass unredacted text through the control while
        // reporting success.
        let entry = parsed.data.into_iter().next().ok_or_else(|| {
            TraceContributionError::RedactionFailed {
                reason: format!("{BACKEND} privacy classifier returned an empty data array"),
            }
        })?;

        Ok(entry.spans)
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

    let max_input_tokens =
        match std::env::var("TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_INPUT_TOKENS") {
            Ok(value) => value.trim().parse::<usize>().map_err(|err| {
                PrivacyFilterConfigError::InvalidEnv {
                    var: "TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_INPUT_TOKENS",
                    reason: err.to_string(),
                }
            })?,
            Err(_) => DEFAULT_MAX_INPUT_TOKENS,
        };

    let max_concurrent_windows =
        match std::env::var("TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_CONCURRENT_WINDOWS") {
            Ok(value) => value.trim().parse::<usize>().map_err(|err| {
                PrivacyFilterConfigError::InvalidEnv {
                    var: "TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_CONCURRENT_WINDOWS",
                    reason: err.to_string(),
                }
            })?,
            Err(_) => DEFAULT_MAX_CONCURRENT_WINDOWS,
        };

    Ok(Arc::new(SelfHostedPrivacyFilterAdapter::new(
        base_url,
        model,
        Duration::from_millis(timeout_ms),
        max_input_bytes,
        max_input_tokens,
        max_concurrent_windows,
    )?))
}
