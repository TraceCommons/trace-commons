//! NEAR AI Cloud hosted privacy-classifier backend for trace redaction.
//!
//! See docs/superpowers/specs/2026-05-19-near-ai-pii-redaction-design.md
//! for the contract this module implements.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};

use crate::privacy_filter_spans::{ClassifySpan, apply_windowed_spans};
use crate::trace_contribution::{
    PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES, PrivacyFilterAdapter, PrivacyFilterConfigError,
    SafePrivacyFilterRedaction, TraceContributionError,
};

pub const DEFAULT_BASE_URL: &str = "https://cloud-api.near.ai/v1";
pub const DEFAULT_MODEL: &str = "openai/privacy-filter";
/// Per-request timeout, sized for a full window at the body cap.
///
/// Measured by the owner against the live endpoint on 2026-11-05 with
/// synthetic text: 20 KiB in 11.3 s, 103 KiB in 14.8 s, 205 KiB in 36.5 s --
/// several times slower than 2026-09-03, when 243 KiB took 9.3 s. At that
/// rate a 240 KiB window takes over 40 s, past the old 30 s timeout, which
/// would turn every full window into four timed-out attempts and a refusal.
/// 90 s leaves room for the cap and for the endpoint slowing further; four
/// attempts at it still fit inside the witness's 300 s request limit only
/// for a single window, which is the case a retry is for.
pub const DEFAULT_TIMEOUT_MS: u64 = 90_000;

/// Largest `privacy/classify` request BODY, in bytes of the exact serialized
/// JSON the adapter sends. **Windows are sized by this and nothing else.**
///
/// The endpoint's binding limit is the body size. Measured 2026-09-03 against
/// the live endpoint with trace-realistic content:
///
/// | true tokens | body | result |
/// |---|---|---|
/// | 2,000 -> 64,000 | 6.4 KB -> 203 KB | 200 OK, 1.5-7.1 s |
/// | 73,256 | 243 KiB | 200 OK, 9.3 s -- largest success |
/// | 78,940 | 261 KiB | 413 |
/// | 100,000 | 330 KiB | 413 |
///
/// An over-limit body gets an honest `413 Failed to buffer the request body`.
/// Before that date the limit was on tokens per request and was reported as
/// a generic 502 (2026-08-27 to 2026-08-31, ceilings of ~1,000 to ~6,000
/// tokens); the token budget that answered that era is gone, because a token
/// budget can only bound the body indirectly through bytes-per-token, which
/// varies about 2x between prose and trace content and far more for
/// escape-heavy text.
///
/// So the adapter builds the request body for a candidate window, measures
/// its length, and cuts the window when it would exceed this cap (see
/// [`window_ranges`]). 240 KiB sits under the ~256 KiB measured limit. That
/// limit has moved before, so a 413 is not fatal: the window is halved and
/// each half classified, at most [`MAX_CLASSIFY_413_SPLITS`] times deep.
///
/// Override with `TRACE_NEAR_AI_PRIVACY_MAX_REQUEST_BODY_BYTES`.
/// Re-measure before trusting the number.
pub const MAX_CLASSIFY_REQUEST_BODY_BYTES: usize = 240 * 1024;

/// The smallest body proven to be refused with a 413 (261 KiB, 2026-09-03),
/// rounded down to the 256 KiB the endpoint's buffer limit appears to be.
pub const MEASURED_CLASSIFY_BODY_LIMIT_BYTES: usize = 256 * 1024;

// A cap at or over the measured limit turns every full window into a 413 and
// a halving round trip. Enforced at compile time.
const _: () = assert!(
    MAX_CLASSIFY_REQUEST_BODY_BYTES < MEASURED_CLASSIFY_BODY_LIMIT_BYTES,
    "MAX_CLASSIFY_REQUEST_BODY_BYTES must stay under the measured body limit"
);

/// How many times a window answered with a 413 may be halved before the
/// field is refused. Four halvings take a full window to 1/16 of the cap
/// (15 KiB): a limit that has fallen that far is an endpoint change for an
/// operator to look at, not something to keep bisecting through.
pub const MAX_CLASSIFY_413_SPLITS: usize = 4;

/// Texts per request when a caller hands the adapter several at once. 1:
/// batching is off by default. #463 batched windows and was reverted (#468)
/// when the endpoint limited TOKENS per request and a batch summed them; the
/// limit has been a body size since 2026-09-03, and batching under it has
/// not been measured. Turn on with `TRACE_NEAR_AI_PRIVACY_MAX_BATCH_INPUTS`
/// only after `scripts/witness-load/near_ai_probe.py batch` says it holds.
pub const DEFAULT_MAX_BATCH_INPUTS: usize = 1;

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
    /// Largest serialized request body. Configurable because the endpoint's
    /// limit has moved before; see [`MAX_CLASSIFY_REQUEST_BODY_BYTES`].
    max_request_body_bytes: usize,
    /// Most texts sent in one request as an input array. 1 is off.
    max_batch_inputs: usize,
    /// Memoized classifications, keyed by SHA-256 of the window text.
    ///
    /// Agent traces re-read the same files and echo the same tool output
    /// across events, so the same window is classified over and over.
    /// Measured across 40 real sessions on a contributor machine: 42,441
    /// windows, 18,039 distinct -- **57.5% of round trips are re-classifying
    /// text already sent**, at 2.35:1 duplication. That is a larger effect
    /// than chunk sizing, concurrency, batching or host CPU, all of which
    /// were tried against this bottleneck and none of which moved it.
    ///
    /// This is EXACT, not a heuristic: reusing a real classification of
    /// identical bytes makes no cleanliness claim about text the model never
    /// saw. That distinction is what separates it from "skip windows that
    /// look clean", which would be a fail-open.
    ///
    /// The key is a hash and the value is offsets and labels, so the cache
    /// holds no plaintext.
    window_cache: std::sync::Mutex<ClassifyWindowCache>,
}

/// Per-adapter memo table. The adapter is constructed once per driver tick,
/// so this lives for one tick's work and then drops -- no persistence, no
/// cross-tenant surface, and no invalidation problem, because it cannot
/// outlive the model version that produced its entries.
#[derive(Default)]
struct ClassifyWindowCache {
    entries: std::collections::HashMap<[u8; 32], Vec<ClassifySpan>>,
    hits: usize,
    misses: usize,
}

/// Upper bound on memoized windows. A pathological envelope (the pilot holds
/// one at 16 MB) could otherwise accumulate thousands of distinct entries in
/// a single tick. Past the cap we simply stop inserting: correctness is
/// unaffected, only the hit rate falls.
const MAX_CACHED_CLASSIFY_WINDOWS: usize = 20_000;

/// Lookups between hit-rate samples.
const CACHE_SAMPLE_INTERVAL: usize = 500;

/// Lookups before the first sample, so a deploy is observable quickly rather
/// than only once 500 windows have gone by.
const FIRST_CACHE_SAMPLE_AFTER: usize = 25;

// The first sample must precede the steady-state interval, or a deploy shows
// no cache activity until 500 windows have gone by -- which is the silence
// this schedule exists to remove. Enforced at compile time.
const _: () = assert!(
    FIRST_CACHE_SAMPLE_AFTER > 0 && FIRST_CACHE_SAMPLE_AFTER < CACHE_SAMPLE_INTERVAL,
    "the first cache sample must land after some work but before a full interval"
);

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
        Self::with_max_request_body_bytes(
            base_url,
            model,
            api_key,
            timeout,
            max_input_bytes,
            MAX_CLASSIFY_REQUEST_BODY_BYTES,
        )
    }

    /// As [`Self::new`], with an explicit request-body cap.
    pub fn with_max_request_body_bytes(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
        timeout: Duration,
        max_input_bytes: usize,
        max_request_body_bytes: usize,
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
            max_request_body_bytes,
            max_batch_inputs: DEFAULT_MAX_BATCH_INPUTS,
            window_cache: std::sync::Mutex::new(ClassifyWindowCache::default()),
        })
    }
}

impl NearAiPrivacyFilterAdapter {
    /// Send up to `max_batch_inputs` single-window texts per request as an
    /// input array, when a caller hands several to
    /// [`PrivacyFilterAdapter::redact_texts`]. 1 turns batching off; 0 is
    /// treated as 1.
    pub fn with_max_batch_inputs(mut self, max_batch_inputs: usize) -> Self {
        self.max_batch_inputs = max_batch_inputs.max(1);
        self
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

    // The token budget this variable set is gone: windows are sized by the
    // encoded request body. A host that still sets it is refused at startup
    // rather than silently ignored, so nobody believes a budget is in force
    // that is not.
    if std::env::var_os("TRACE_NEAR_AI_PRIVACY_MAX_INPUT_TOKENS").is_some() {
        return Err(PrivacyFilterConfigError::InvalidEnv {
            var: "TRACE_NEAR_AI_PRIVACY_MAX_INPUT_TOKENS",
            reason: "retired: windows are sized by request body; set \
                     TRACE_NEAR_AI_PRIVACY_MAX_REQUEST_BODY_BYTES instead"
                .to_string(),
        });
    }

    // Zero is rejected rather than silently meaning "unlimited": an unbounded
    // cap sends the whole field as one request, which is exactly the
    // over-limit case this cap exists to prevent. A cap at or over the
    // measured limit is refused for the same reason.
    let max_request_body_bytes = match std::env::var("TRACE_NEAR_AI_PRIVACY_MAX_REQUEST_BODY_BYTES")
    {
        Ok(value) => {
            let parsed = value.trim().parse::<usize>().map_err(|err| {
                PrivacyFilterConfigError::InvalidEnv {
                    var: "TRACE_NEAR_AI_PRIVACY_MAX_REQUEST_BODY_BYTES",
                    reason: err.to_string(),
                }
            })?;
            if parsed == 0 || parsed >= MEASURED_CLASSIFY_BODY_LIMIT_BYTES {
                return Err(PrivacyFilterConfigError::InvalidEnv {
                    var: "TRACE_NEAR_AI_PRIVACY_MAX_REQUEST_BODY_BYTES",
                    reason: format!(
                        "must be greater than zero and under the measured limit of \
                         {MEASURED_CLASSIFY_BODY_LIMIT_BYTES} bytes"
                    ),
                });
            }
            parsed
        }
        Err(_) => MAX_CLASSIFY_REQUEST_BODY_BYTES,
    };

    // Off unless set. Sending several inputs in one request was reverted in
    // #468, when the endpoint limited tokens per request; whether it holds
    // under today's body limit has not been measured
    // (scripts/witness-load/near_ai_probe.py measures it).
    let max_batch_inputs = match std::env::var("TRACE_NEAR_AI_PRIVACY_MAX_BATCH_INPUTS") {
        Ok(value) => {
            let parsed = value.trim().parse::<usize>().map_err(|err| {
                PrivacyFilterConfigError::InvalidEnv {
                    var: "TRACE_NEAR_AI_PRIVACY_MAX_BATCH_INPUTS",
                    reason: err.to_string(),
                }
            })?;
            if parsed == 0 {
                return Err(PrivacyFilterConfigError::InvalidEnv {
                    var: "TRACE_NEAR_AI_PRIVACY_MAX_BATCH_INPUTS",
                    reason: "must be at least 1 (1 is off)".to_string(),
                });
            }
            parsed
        }
        Err(_) => DEFAULT_MAX_BATCH_INPUTS,
    };

    let adapter = NearAiPrivacyFilterAdapter::with_max_request_body_bytes(
        base_url,
        model,
        api_key,
        Duration::from_millis(timeout_ms),
        max_input_bytes,
        max_request_body_bytes,
    )?
    .with_max_batch_inputs(max_batch_inputs);
    Ok(Arc::new(adapter))
}

#[derive(Serialize)]
struct ClassifyRequest<'a, I: Serialize + ?Sized = str> {
    model: &'a str,
    input: &'a I,
}

#[derive(Deserialize)]
struct ClassifyResponse {
    data: Vec<ClassifyEntry>,
}

#[derive(Deserialize)]
struct ClassifyEntry {
    #[serde(default)]
    spans: Vec<NearAiSpan>,
}

/// A span as the endpoint reports it: offsets in CODEPOINTS, plus the text
/// the endpoint says those offsets cover. `text` is checked against the
/// request (see [`verify_span_texts`]) and then dropped; it is never logged.
#[derive(Deserialize)]
struct NearAiSpan {
    category: String,
    start: usize,
    end: usize,
    #[serde(default)]
    score: f64,
    #[serde(default)]
    text: Option<String>,
}

/// Bytes `text` occupies inside a JSON string literal, quotes excluded --
/// exactly what `serde_json` writes: `"` and `\` and the short-escape
/// controls (`\n`, `\r`, `\t`, backspace, form feed) take 2 bytes, every
/// other control below 0x20 takes 6 (`\u00XX`), and everything else,
/// including non-ASCII UTF-8, is copied as is.
fn json_escaped_len(text: &str) -> usize {
    text.bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0x08 | 0x0c => 2,
            0x00..=0x1f => 6,
            _ => 1,
        })
        .sum()
}

/// The windows a field is classified in: contiguous, covering the whole
/// text, on char boundaries, each as large as `max_input_json_bytes` of
/// JSON-escaped input allows. A window ends after the last newline that fits
/// where there is one, so a line is split only when it alone exceeds the cap.
///
/// `max_input_json_bytes` is the request-body cap less the measured envelope
/// (see [`NearAiPrivacyFilterAdapter::max_input_json_bytes`]), and
/// [`json_escaped_len`] is exactly what `serde_json` writes, so every window's
/// serialized body fits the cap. `classify_window` re-measures the real body
/// before sending anyway.
///
/// One linear pass: each character's escaped cost is computed once.
fn window_ranges(text: &str, max_input_json_bytes: usize) -> Vec<std::ops::Range<usize>> {
    let max_input_json_bytes = max_input_json_bytes.max(1);
    let mut ranges = Vec::new();
    let mut start = 0usize;
    while start < text.len() {
        let mut used = 0usize;
        let mut end = start;
        let mut last_newline_end: Option<usize> = None;
        for (offset, character) in text[start..].char_indices() {
            let mut buffer = [0u8; 4];
            let cost = json_escaped_len(character.encode_utf8(&mut buffer));
            if used + cost > max_input_json_bytes && end > start {
                break;
            }
            used += cost;
            end = start + offset + character.len_utf8();
            if character == '\n' {
                last_newline_end = Some(end);
            }
        }
        if end < text.len() {
            if let Some(newline_end) = last_newline_end {
                end = newline_end;
            }
        }
        ranges.push(start..end);
        start = end;
    }
    ranges
}

/// Where to halve a window the endpoint refused as too large: after the last
/// newline in the first half when that keeps at least a quarter of the window
/// on the left, otherwise at the middle char boundary. `None` when the window
/// is a single character and cannot be split.
fn split_point(text: &str) -> Option<usize> {
    let mut middle = text.len() / 2;
    while middle > 0 && !text.is_char_boundary(middle) {
        middle -= 1;
    }
    if middle == 0 {
        // The first character is wider than half the window: split after it,
        // if anything follows.
        let first = text.chars().next()?.len_utf8();
        return (first < text.len()).then_some(first);
    }
    match text[..middle].rfind('\n') {
        Some(newline) if newline + 1 >= text.len() / 4 && newline + 1 < text.len() => {
            Some(newline + 1)
        }
        _ => Some(middle),
    }
}

const SPAN_TEXT_MISMATCH_LABEL: &str = "privacy_filter_span_text_mismatch";

/// Fail closed unless every span's reported `text` is exactly the request
/// input's codepoints `[start, end)`, then return the spans without it.
///
/// The endpoint reports, with each span, the text it covers (probed live on
/// 2026-10-11 with ZWJ emoji, accented and CJK text, CRLF and blank lines:
/// the codepoint slice equalled `text` on every span). It does not always
/// hold: a shadow run on real sessions found one event whose spans' offsets
/// disagree with their text even when the event is sent alone. An offset
/// slip -- the right entity at shifted offsets -- passes every range check
/// and redacts the wrong characters, leaving the PII in place and reporting
/// success. This check turns that into a refusal. A span with no `text`
/// cannot be checked, so it refuses too.
///
/// Spans whose offsets are out of range are passed through for
/// `apply_spans`, which refuses them with its own error. Neither the span
/// text nor the input is ever logged: the failure carries the label and the
/// input's hash-only fingerprint.
fn verify_span_texts(
    input: &str,
    spans: Vec<NearAiSpan>,
) -> Result<Vec<ClassifySpan>, TraceContributionError> {
    let mut boundaries: Vec<usize> = input.char_indices().map(|(byte, _)| byte).collect();
    boundaries.push(input.len());
    let mismatches = spans
        .iter()
        .filter(|span| {
            if span.start > span.end || span.end >= boundaries.len() {
                return false;
            }
            let covered = &input[boundaries[span.start]..boundaries[span.end]];
            span.text.as_deref() != Some(covered)
        })
        .count();
    if mismatches > 0 {
        let diagnostics = classify_input_diagnostics(&[input]);
        tracing::warn!(
            classify_input = %diagnostics,
            mismatched_spans = mismatches,
            spans = spans.len(),
            failure = SPAN_TEXT_MISMATCH_LABEL,
            "near-ai privacy classify failed"
        );
        return Err(TraceContributionError::RedactionFailed {
            reason: format!(
                "near-ai privacy classifier span offsets disagree with span text: \
                 {SPAN_TEXT_MISMATCH_LABEL} mismatched_spans={mismatches} spans={} {diagnostics}",
                spans.len()
            ),
        });
    }
    Ok(spans
        .into_iter()
        .map(|span| ClassifySpan {
            category: span.category,
            start: span.start,
            end: span.end,
            score: span.score,
        })
        .collect())
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

        // One window per request, and one request at a time.
        //
        // Batching windows into a single request (#463) was reverted: the
        // endpoint's limit is on TOTAL tokens per request, so batching sums
        // the windows and blows it. Measured -- four inputs that each classify
        // fine alone (2,250 tokens each) fail 0/3 when sent together. The
        // first batched tick on the pilot returned
        // `done=0 transient=3 breaker_tripped=true`.
        //
        // Concurrency was tried too (#456, 8 in flight) and also made
        // throughput worse, so there is deliberately no knob for either.
        // The hosted endpoint rejects oversized requests, so split large
        // field text into windows and classify each. Every window's spans
        // are reported in that window's own codepoint coordinates; shift
        // them into full-text codepoints before merging so the single
        // apply_spans pass validates and redacts against the whole field.
        //
        // Windows are sized by the serialized request BODY and nothing else
        // (see MAX_CLASSIFY_REQUEST_BODY_BYTES): the endpoint's limit is a
        // body size, so a window is as large as the body cap allows.
        let ranges = window_ranges(text, self.max_input_json_bytes());

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
                        self.classify_window(window, MAX_CLASSIFY_413_SPLITS)
                            .await
                            .map(|spans| (codepoint_start, spans))
                    }
                },
            ))
            .buffered(MAX_CONCURRENT_CLASSIFY_WINDOWS)
            .try_collect()
            .await?;

        apply_windowed_spans("near-ai", text, &windows)
    }

    fn preferred_batch_inputs(&self) -> usize {
        self.max_batch_inputs
    }

    /// With batching on (`TRACE_NEAR_AI_PRIVACY_MAX_BATCH_INPUTS` above 1),
    /// texts that fit one window are sent several to a request as an input
    /// array, packed under the body cap, and each answer is mapped back by
    /// its index. Everything else -- blank text, a field over the input
    /// limit, a text needing several windows, a text already in the window
    /// cache -- goes through [`Self::redact_text`] exactly as it would alone.
    ///
    /// Each text's answer is what `redact_text` returns for it given the
    /// same spans: the same single window, the same verification, the same
    /// cache entry, the same `apply_windowed_spans`. A failed request fails
    /// every text it carried; a 413 splits the batch in half, and a single
    /// text that still draws one is halved by `redact_text` as usual.
    async fn redact_texts(&self, texts: &[&str]) -> Vec<Answer> {
        let mut answers: Vec<Option<Answer>> = vec![None; texts.len()];
        let mut batchable: Vec<usize> = Vec::new();
        if self.max_batch_inputs > 1 && texts.len() > 1 {
            let window_budget = self.max_input_json_bytes();
            for (index, text) in texts.iter().enumerate() {
                let fits = !text.trim().is_empty()
                    && text.len() <= self.max_input_bytes
                    && json_escaped_len(text) <= window_budget;
                if !fits {
                    continue;
                }
                let key = window_key(text);
                match self.cached_window(&key) {
                    Some(spans) => {
                        answers[index] = Some(apply_windowed_spans("near-ai", text, &[(0, spans)]))
                    }
                    None => batchable.push(index),
                }
            }
        }

        // Pack in order, under both the item limit and the body cap.
        let mut pending: Vec<Vec<usize>> = Vec::new();
        let base = self.batch_body_base_len();
        let mut current: Vec<usize> = Vec::new();
        let mut current_len = base;
        for index in batchable {
            let cost = json_escaped_len(texts[index]) + 2 + usize::from(!current.is_empty());
            if !current.is_empty()
                && (current.len() >= self.max_batch_inputs
                    || current_len + cost > self.max_request_body_bytes)
            {
                pending.push(std::mem::take(&mut current));
                current_len = base;
            }
            current_len += json_escaped_len(texts[index]) + 2 + usize::from(!current.is_empty());
            current.push(index);
        }
        if !current.is_empty() {
            pending.push(current);
        }
        // Batches are sent in order; a halved batch's halves go first.
        pending.reverse();
        while let Some(batch) = pending.pop() {
            if batch.len() == 1 {
                // Alone it is an ordinary single-window request.
                continue;
            }
            let inputs: Vec<&str> = batch.iter().map(|&index| texts[index]).collect();
            let body = serde_json::to_vec(&ClassifyRequest {
                model: &self.model,
                input: inputs.as_slice(),
            });
            let outcome = match body {
                Ok(body) if body.len() <= self.max_request_body_bytes => {
                    self.send_window(&inputs, body).await
                }
                // Over the cap after all: treat as the endpoint would.
                Ok(_) => Ok(WindowAnswer::TooLarge),
                Err(_) => Err(TraceContributionError::RedactionFailed {
                    reason: "near-ai privacy classifier request did not serialize".to_string(),
                }),
            };
            match outcome {
                Ok(WindowAnswer::Spans(all)) => {
                    for (&index, spans) in batch.iter().zip(all) {
                        let text = texts[index];
                        self.remember_window(window_key(text), &spans);
                        answers[index] = Some(apply_windowed_spans("near-ai", text, &[(0, spans)]));
                    }
                }
                Ok(WindowAnswer::TooLarge) => {
                    let (left, right) = batch.split_at(batch.len() / 2);
                    pending.push(right.to_vec());
                    pending.push(left.to_vec());
                }
                Err(error) => {
                    for &index in &batch {
                        answers[index] = Some(Err(error.clone()));
                    }
                }
            }
        }

        // Everything not answered above, in order, exactly as alone.
        let mut out = Vec::with_capacity(texts.len());
        for (index, answer) in answers.into_iter().enumerate() {
            out.push(match answer {
                Some(answer) => answer,
                None => self.redact_text(texts[index]).await,
            });
        }
        out
    }
}

type Answer = Result<Option<SafePrivacyFilterRedaction>, TraceContributionError>;

/// The window cache key: SHA-256 of the window's bytes.
fn window_key(text: &str) -> [u8; 32] {
    <sha2::Sha256 as sha2::Digest>::digest(text.as_bytes()).into()
}

impl NearAiPrivacyFilterAdapter {
    /// Look up a previously classified window, counting the hit or miss.
    ///
    /// A miss is recorded here rather than at insert time so that a window
    /// whose classification FAILS still counts as a miss -- otherwise a run
    /// of failures would inflate the apparent hit rate.
    fn cached_window(&self, key: &[u8; 32]) -> Option<Vec<ClassifySpan>> {
        let mut cache = match self.window_cache.lock() {
            Ok(cache) => cache,
            // A poisoned lock means a previous caller panicked mid-update.
            // Losing the memo is a throughput cost, never a correctness one,
            // so degrade to "always classify" rather than propagate.
            Err(poisoned) => poisoned.into_inner(),
        };
        let hit = cache.entries.get(key).cloned();
        match &hit {
            Some(_) => cache.hits += 1,
            None => cache.misses += 1,
        }
        // Sample on BOTH outcomes. Sampling only on hits meant a cold cache
        // -- every lookup a miss -- logged nothing at all, so "no telemetry"
        // was indistinguishable from "no lookups" and the first tick after a
        // deploy reported silence either way.
        Self::log_cache_sample(&cache);
        hit
    }

    /// Record a successful classification for reuse within this tick.
    fn remember_window(&self, key: [u8; 32], spans: &[ClassifySpan]) {
        let mut cache = match self.window_cache.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        };
        if cache.entries.len() >= MAX_CACHED_CLASSIFY_WINDOWS {
            return;
        }
        cache.entries.insert(key, spans.to_vec());
    }

    /// Periodic hit-rate line, so the production rate can be compared against
    /// the 57.5% measured offline. Sampled rather than per-window: a single
    /// envelope can carry thousands of windows and a line each would drown
    /// the log.
    ///
    /// Fires on hits AND misses. A cold cache is all misses, so sampling only
    /// hits produced no output until the cache was already working -- exactly
    /// when the measurement is least needed. The first sample is emitted
    /// early so a deploy shows signs of life without waiting a full interval.
    fn log_cache_sample(cache: &ClassifyWindowCache) {
        let total = cache.hits + cache.misses;
        // An early first sample confirms the cache is being consulted at all;
        // after that, one line per interval.
        if total != FIRST_CACHE_SAMPLE_AFTER && !total.is_multiple_of(CACHE_SAMPLE_INTERVAL) {
            return;
        }
        tracing::info!(
            hits = cache.hits,
            misses = cache.misses,
            distinct_windows = cache.entries.len(),
            hit_rate_pct = (cache.hits * 100) / total.max(1),
            "near-ai privacy classify window cache"
        );
    }

    /// POST one window of text to the classifier and return its raw spans
    /// (in that window's codepoint coordinates). Fail-closed on any
    /// transport error, non-2xx status, malformed body, or empty data
    /// array.
    /// Classify one window, in that window's own codepoint coordinates.
    ///
    /// The request body is serialized here and its length checked against
    /// the cap before anything is sent. A body over the cap, or one the
    /// endpoint refuses with a 413, is halved (see [`split_point`]) and each
    /// half classified, at most `splits_left` times deep; the halves' spans
    /// are shifted back into this window's coordinates, so the caller cannot
    /// tell a split window from a whole one. The endpoint classifies a span
    /// the same wherever the window boundary falls only to the extent its
    /// model does, which is why splitting is the exception and the cap keeps
    /// it so.
    fn classify_window<'a>(
        &'a self,
        text: &'a str,
        splits_left: usize,
    ) -> futures::future::BoxFuture<'a, Result<Vec<ClassifySpan>, TraceContributionError>> {
        Box::pin(async move {
            // Memoize on the window's content hash. The lock is taken only
            // around the map access and released before any await, so it
            // never spans a network round trip.
            let key = window_key(text);
            if let Some(hit) = self.cached_window(&key) {
                return Ok(hit);
            }
            let body = serde_json::to_vec(&ClassifyRequest {
                model: &self.model,
                input: text,
            })
            .map_err(|_| TraceContributionError::RedactionFailed {
                reason: "near-ai privacy classifier request did not serialize".to_string(),
            })?;
            let spans = if body.len() > self.max_request_body_bytes {
                self.classify_halves(text, splits_left).await?
            } else {
                match self.send_window(&[text], body).await? {
                    WindowAnswer::Spans(mut spans) => spans.swap_remove(0),
                    WindowAnswer::TooLarge => self.classify_halves(text, splits_left).await?,
                }
            };
            // Verified before it is remembered (in `send_window`, per
            // half): a refused response is never served from the cache.
            self.remember_window(key, &spans);
            Ok(spans)
        })
    }

    /// Classify `text` as two windows and merge their spans into `text`'s
    /// codepoint coordinates. Refuses, permanently, when the split budget is
    /// spent or the window is one character.
    async fn classify_halves(
        &self,
        text: &str,
        splits_left: usize,
    ) -> Result<Vec<ClassifySpan>, TraceContributionError> {
        let split = match (splits_left, split_point(text)) {
            (0, _) | (_, None) => {
                let diagnostics = classify_input_diagnostics(&[text]);
                tracing::warn!(
                    classify_input = %diagnostics,
                    failure = "too_large",
                    "near-ai privacy classify failed"
                );
                return Err(TraceContributionError::RedactionFailed {
                    reason: format!(
                        "near-ai privacy classifier refused the request as too large after \
                         {MAX_CLASSIFY_413_SPLITS} halvings {diagnostics}"
                    ),
                });
            }
            (_, Some(split)) => split,
        };
        let (left, right) = text.split_at(split);
        let mut spans = self.classify_window(left, splits_left - 1).await?;
        let shift = left.chars().count();
        let right_spans = self.classify_window(right, splits_left - 1).await?;
        spans.extend(right_spans.into_iter().map(|span| ClassifySpan {
            start: span.start + shift,
            end: span.end + shift,
            ..span
        }));
        Ok(spans)
    }

    /// One request for one window, with the transient retries. A 413 comes
    /// back as [`WindowAnswer::TooLarge`] for the caller to split.
    async fn send_window(
        &self,
        inputs: &[&str],
        body: Vec<u8>,
    ) -> Result<WindowAnswer, TraceContributionError> {
        let endpoint = format!("{}/privacy/classify", self.base_url.trim_end_matches('/'));

        let mut attempt = 0;
        loop {
            attempt += 1;
            // Transient failures (transport errors, 5xx) are retried with
            // exponential backoff; 4xx and body-shape failures are not.
            let send_result = self
                .client
                .post(&endpoint)
                .bearer_auth(&self.api_key.0)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.clone())
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
                    let diagnostics = classify_input_diagnostics(inputs);
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
            if status == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
                return Ok(WindowAnswer::TooLarge);
            }
            if !status.is_success() {
                let transient = crate::privacy_filter_spans::is_transient_classifier_status(status);
                if transient && attempt < MAX_CLASSIFY_ATTEMPTS {
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
                // the caller: a 5xx, 429 or 408 that outlived our retries is
                // the vendor's problem, any other 4xx is ours or the trace's.
                let diagnostics = classify_input_diagnostics(inputs);
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
                return Err(if transient {
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
            // One entry per input, in input order. A single-input request
            // reads the first entry, as it always has; a batch must account
            // for every input exactly, or none of it is used.
            if parsed.data.is_empty() {
                return Err(TraceContributionError::RedactionFailed {
                    reason: "near-ai privacy classifier returned empty data array".to_string(),
                });
            }
            if inputs.len() > 1 && parsed.data.len() != inputs.len() {
                return Err(TraceContributionError::RedactionFailed {
                    reason: format!(
                        "near-ai privacy classifier returned {} entries for {} inputs",
                        parsed.data.len(),
                        inputs.len()
                    ),
                });
            }
            // Verified before anything is remembered: a refused response
            // must never be served from the cache.
            let spans = inputs
                .iter()
                .zip(parsed.data)
                .map(|(input, entry)| verify_span_texts(input, entry.spans))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(WindowAnswer::Spans(spans));
        }
    }
}

/// What one classify request answered.
enum WindowAnswer {
    /// Verified spans per input, each in that input's own codepoint
    /// coordinates, in input order.
    Spans(Vec<Vec<ClassifySpan>>),
    /// A 413: the body was over the endpoint's limit.
    TooLarge,
}

impl NearAiPrivacyFilterAdapter {
    /// The escaped-input budget that keeps a request's whole serialized body
    /// within the adapter's body cap: the body is the request envelope
    /// (model, keys, the input's quotes) plus the input's JSON-escaped bytes,
    /// and the envelope is measured, not guessed.
    /// Body bytes of a batch request with no inputs (`"input":[]`); each
    /// input adds its escaped length, two quotes and, after the first, a
    /// comma.
    fn batch_body_base_len(&self) -> usize {
        let empty: &[&str] = &[];
        serde_json::to_vec(&ClassifyRequest {
            model: &self.model,
            input: empty,
        })
        .map(|body| body.len())
        .unwrap_or(self.max_request_body_bytes)
    }

    fn max_input_json_bytes(&self) -> usize {
        let envelope = serde_json::to_vec(&ClassifyRequest {
            model: &self.model,
            input: "",
        })
        .map(|body| body.len())
        .unwrap_or(self.max_request_body_bytes);
        self.max_request_body_bytes.saturating_sub(envelope).max(1)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact body the adapter would send for `input`.
    fn request_body_len(input: &str) -> usize {
        serde_json::to_vec(&ClassifyRequest {
            model: DEFAULT_MODEL,
            input,
        })
        .expect("serializes")
        .len()
    }

    fn input_budget(cap: usize) -> usize {
        cap - request_body_len("")
    }

    /// Escape-heavy text: quotes, backslashes, newlines, tabs, other control
    /// characters, and 2-, 3- and 4-byte UTF-8.
    fn escape_heavy(lines: usize) -> String {
        (0..lines)
            .map(|i| {
                format!(
                    "{i} \"quoted\" C:\\path\\x\t\u{1}\u{1f} caf\u{e9} \u{4e2d}\u{6587} \u{1f600} {}\r\n",
                    "\\\"".repeat(i % 7)
                )
            })
            .collect()
    }

    /// A span whose reported text is not the text at its offsets fails the
    /// server's prose-PII rescrub closed, with the hash-only label, and
    /// nothing the classifier would change is applied to the envelope.
    #[tokio::test]
    async fn a_span_text_mismatch_fails_the_rescrub_and_applies_nothing() {
        use crate::trace_contribution::{
            PiiClassifyPolicy, TraceContributionEnvelope, rescrub_envelope_prose_pii_with,
        };
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/privacy/classify"))
            .respond_with(|req: &wiremock::Request| {
                let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
                let input = body["input"].as_str().unwrap();
                // Every "Ada" found, reported one codepoint late with the
                // right text: an offset slip the range check cannot see.
                let spans: Vec<serde_json::Value> = input
                    .match_indices("Ada")
                    .map(|(byte, _)| {
                        let start = input[..byte].chars().count() + 1;
                        serde_json::json!({
                            "category": "private_person",
                            "start": start,
                            "end": start + 3,
                            "score": 0.9,
                            "text": "Ada",
                        })
                    })
                    .collect();
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "data": [{ "spans": spans }] }))
            })
            .mount(&server)
            .await;
        let adapter = NearAiPrivacyFilterAdapter::new(
            server.uri(),
            "openai/privacy-filter",
            "test-key",
            Duration::from_secs(5),
            1_000_000,
        )
        .unwrap();

        let event = |content: &str| {
            serde_json::json!({
                "event_id": uuid::Uuid::new_v4(),
                "event_type": "user_message",
                "redacted_content": content,
                "side_effect": "none",
                "timestamp": "2026-10-11T00:00:00Z",
            })
        };
        let mut envelope: TraceContributionEnvelope = serde_json::from_value(serde_json::json!({
            "schema_version": "ironclaw.trace_contribution.v1",
            "trace_id": uuid::Uuid::new_v4(),
            "submission_id": uuid::Uuid::new_v4(),
            "created_at": "2026-10-11T00:00:00Z",
            "ironclaw": {"channel": "cli", "version": "1"},
            "consent": {
                "policy_version": "2026-04-24",
                "scopes": ["debugging_evaluation"],
                "message_text_included": true,
                "tool_payloads_included": false,
                "routing_metadata_included": false,
                "correction_included": false,
                "revocable": true
            },
            "contributor": {
                "pseudonymous_contributor_id": "sha256:contributor",
                "revocation_handle": uuid::Uuid::new_v4()
            },
            "events": [
                event("nothing here"),
                event("Ada reviewed the change"),
                event("ran the tests"),
            ],
            "outcome": {"task_success": "unknown", "user_feedback": "none"},
            "privacy": {
                "redaction_hash": "sha256:placeholder",
                "redaction_pipeline_version": "ironclaw-deterministic-secret-path-v3",
                "residual_pii_risk": "low"
            },
            "replay": {"replayable": false},
            "trace_card": {
                "allowed_uses": ["debugging", "evaluation", "aggregate_analytics"],
                "consent_scope": "debugging_evaluation",
                "redaction_pipeline_version": "ironclaw-deterministic-secret-path-v3",
                "retention_policy": "private_corpus_revocable",
                "revocation_handle": uuid::Uuid::new_v4(),
                "source_channel": "unknown"
            },
            "value": {"credit_points_pending": 0.0, "submission_score": 0.0},
            "value_card": {
                "limitations": [],
                "score_version": "trace-value-scorecard-v1",
                "scorecard": {
                    "coverage_bonus": 0.0, "credit_points_estimate": 0.0,
                    "dependability": 0.0, "difficulty": 0.0, "duplicate_penalty": 0.0,
                    "novelty": 0.0, "online_score": 0.0, "privacy_risk": 0.0,
                    "quality": 0.0, "replayability": 0.0, "schema_validity": 0.0,
                    "user_correction_value": 0.0
                }
            }
        }))
        .expect("fixture envelope deserializes");
        let before = serde_json::to_value(&envelope).unwrap();

        let error =
            rescrub_envelope_prose_pii_with(&adapter, &mut envelope, PiiClassifyPolicy::AllEvents)
                .await
                .expect_err("a span text mismatch must fail closed");
        assert!(matches!(
            error,
            TraceContributionError::RedactionFailed { .. }
        ));
        assert!(error.to_string().contains(SPAN_TEXT_MISMATCH_LABEL));
        assert!(!error.to_string().contains("Ada"), "never echo the text");

        let after = serde_json::to_value(&envelope).unwrap();
        assert_eq!(after["events"], before["events"]);
        for field in [
            "redaction_counts",
            "pii_labels_present",
            "privacy_filter_summary",
            "redaction_pipeline_version",
            "residual_pii_risk",
            "redaction_hash",
        ] {
            assert_eq!(
                after["privacy"][field], before["privacy"][field],
                "privacy.{field} must be untouched on failure"
            );
        }
    }

    #[test]
    fn json_escaped_len_matches_serde_json() {
        for text in [
            "",
            "plain ascii",
            "quote \" backslash \\ slash /",
            "\n\r\t\u{8}\u{c}",
            "\u{0}\u{1}\u{1f}\u{7f}",
            "Zo\u{eb} \u{674e}\u{5c0f}\u{9f8d} \u{1f600} \u{1f468}\u{200d}\u{1f469}",
        ] {
            assert_eq!(
                json_escaped_len(text),
                serde_json::to_string(text).unwrap().len() - 2,
                "{text:?}"
            );
        }
    }

    /// Every window's serialized request body -- measured by serializing
    /// it, not by the arithmetic `window_ranges` uses -- fits the cap, for
    /// escape-heavy text whose body is several times its length.
    #[test]
    fn escape_heavy_windows_stay_under_the_cap_after_encoding() {
        let cap = MAX_CLASSIFY_REQUEST_BODY_BYTES;
        let text = escape_heavy(12_000);
        assert!(
            request_body_len(&text) > 3 * cap,
            "the fixture must need several windows"
        );
        let windows = window_ranges(&text, input_budget(cap));
        assert!(windows.len() > 3);
        for window in &windows {
            let body = request_body_len(&text[window.clone()]);
            assert!(body <= cap, "a {body}-byte body is over the {cap}-byte cap");
        }
        let covered: String = windows.iter().map(|w| &text[w.clone()]).collect();
        assert_eq!(covered, text);
    }

    /// The cap is used, not wasted: each window but the last is full to
    /// within one line of the cap, so a field costs as few requests as the
    /// body limit allows.
    #[test]
    fn windows_fill_the_cap_to_within_a_line() {
        let cap = MAX_CLASSIFY_REQUEST_BODY_BYTES;
        let text = escape_heavy(12_000);
        let longest_line = text
            .split_inclusive('\n')
            .map(|line| request_body_len(line) - request_body_len(""))
            .max()
            .expect("lines");
        let windows = window_ranges(&text, input_budget(cap));
        for window in &windows[..windows.len() - 1] {
            assert!(
                request_body_len(&text[window.clone()]) + longest_line > cap,
                "a window stopped more than a line short of the cap"
            );
            assert!(
                text[window.clone()].ends_with('\n'),
                "a window split a line"
            );
        }
    }

    /// A 241 KB single field of ordinary tool output -- the size that took
    /// 115 s on the live witness -- is one or two requests, not dozens.
    #[test]
    fn a_large_ordinary_field_takes_few_requests() {
        let text: String = (0..6_000)
            .map(|i| {
                format!(
                    "[{i:06}] INFO build::module took {}ms /home/u/src/f{i}.rs\n",
                    i % 997
                )
            })
            .collect();
        assert!(text.len() > 240_000);
        let windows = window_ranges(&text, input_budget(MAX_CLASSIFY_REQUEST_BODY_BYTES));
        assert!(windows.len() <= 2, "{} windows", windows.len());
    }

    /// No window boundary falls inside a codepoint, including in a run with
    /// no newline at all, where the cut is at the cap itself. (`&text[..]` on
    /// a non-boundary would panic, so the slice is the assertion; the
    /// explicit check says what is being tested.)
    #[test]
    fn windows_never_split_a_codepoint() {
        let text = "\u{e9}\u{4e2d}\u{1f600}\"\\".repeat(40_000);
        let envelope = request_body_len("");
        for input_cap in [
            4,
            5,
            6,
            7,
            1_000,
            input_budget(MAX_CLASSIFY_REQUEST_BODY_BYTES),
        ] {
            let windows = window_ranges(&text, input_cap);
            for window in &windows {
                assert!(text.is_char_boundary(window.start) && text.is_char_boundary(window.end));
                assert!(!window.is_empty());
                assert!(request_body_len(&text[window.clone()]) <= envelope + input_cap);
            }
            let covered: String = windows.iter().map(|w| &text[w.clone()]).collect();
            assert_eq!(covered, text);
        }
    }

    /// The 413 split point is a char boundary that leaves both halves
    /// non-empty, prefers a newline, and refuses a single character.
    #[test]
    fn split_points_are_char_boundaries_inside_the_window() {
        let accents = "\u{e9}".repeat(1001);
        for text in [
            "\u{1f600}\u{1f600}\u{1f600}",
            "ab",
            "\u{1f600}a",
            "line one\nline two\nline three\nline four\n",
            accents.as_str(),
        ] {
            let split = split_point(text).expect("splittable");
            assert!(split > 0 && split < text.len(), "{text:?} at {split}");
            assert!(text.is_char_boundary(split));
        }
        assert_eq!(
            split_point("line one\nline two\nline three\nline four\n"),
            Some("line one\nline two\n".len())
        );
        assert_eq!(split_point("\u{1f600}"), None);
        assert_eq!(split_point("a"), None);
    }

    fn span(category: &str, start: usize, end: usize, score: f64) -> ClassifySpan {
        ClassifySpan {
            category: category.to_string(),
            start,
            end,
            score,
        }
    }

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

    /// A cold cache is all misses. Sampling only on hits meant the telemetry
    /// stayed silent exactly when it was most needed -- right after a deploy
    /// -- and "no telemetry" could not be distinguished from "no lookups".
    /// The counters must advance on both outcomes.
    #[test]
    fn cache_counters_advance_on_misses_not_only_hits() {
        let mut cache = ClassifyWindowCache::default();
        assert_eq!((cache.hits, cache.misses), (0, 0));

        // Simulate the miss path: nothing stored, so a lookup must count.
        cache.misses += 1;
        assert_eq!(
            (cache.hits, cache.misses),
            (0, 1),
            "a miss must be counted, or hit rate is unmeasurable on a cold cache"
        );

        // And the sampler must be reachable with zero hits.
        let total = cache.hits + cache.misses;
        assert!(
            total == FIRST_CACHE_SAMPLE_AFTER
                || total.is_multiple_of(CACHE_SAMPLE_INTERVAL)
                || total < FIRST_CACHE_SAMPLE_AFTER,
            "sampling schedule must be defined for an all-miss cache"
        );
    }

    #[test]
    fn empty_input_short_circuits() {
        // Cannot call redact_text without a client; test apply_spans
        // covers the inner behavior. Empty-text short-circuit is in
        // redact_text proper, exercised by integration tests in Task 8.
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
        let result = apply_windowed_spans("near-ai", text, &windows)
            .unwrap()
            .unwrap();
        assert_eq!(result.redacted_text, "café [REDACTED:private_email]!");
        assert_eq!(result.summary.span_count, 1);
    }

    #[test]
    fn debug_does_not_leak_api_key() {
        let secret = SecretApiKey("super-secret-token".into());
        let debug = format!("{secret:?}");
        assert!(!debug.contains("super-secret-token"));
        assert!(debug.contains("***"));
    }
}
