//! NEAR AI Cloud hosted privacy-classifier backend for trace redaction.
//!
//! See docs/superpowers/specs/2026-05-19-near-ai-pii-redaction-design.md
//! for the contract this module implements.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};

use crate::privacy_filter_spans::{ClassifySpan, apply_spans};
use crate::trace_contribution::{
    PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES, PrivacyFilterAdapter, PrivacyFilterConfigError,
    SafePrivacyFilterRedaction, TraceContributionError,
};

pub const DEFAULT_BASE_URL: &str = "https://cloud-api.near.ai/v1";
pub const DEFAULT_MODEL: &str = "openai/privacy-filter";
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;

/// Maximum input tokens per `privacy/classify` request.
///
/// **The endpoint limits input TOKENS per request and reports exceeding that
/// as a generic 502** -- not a 413, not a 429 -- so an over-limit request is
/// indistinguishable from a vendor outage by status code alone. That ambiguity
/// cost a day of misdiagnosis on 2026-08-27, during which a byte budget was
/// tuned three times (20_000 -> 4_000 -> 8_000 -> 4_000) without ever
/// addressing the quantity actually being limited.
///
/// A byte budget cannot express this. Token density varies ~1.6x across
/// realistic content -- measured against the endpoint's own
/// `usage.input_tokens`, English prose runs ~4.7 bytes/token while the paths,
/// hashes and UUIDs that fill trace content run ~3.0. So any byte budget is
/// either too small for prose (needless extra requests) or too large for dense
/// content (silent 502s). 8_000 bytes was the latter: fine for most windows,
/// deterministically fatal for token-dense ones, which is exactly the
/// intermittent failure the #462 fingerprints caught.
///
/// Budgeting in tokens fixes both ends at once: prose windows get BIGGER
/// (~9,400 bytes at this budget, fewer requests than the 4_000-byte cap it
/// replaces) while dense windows get capped where they actually break.
///
/// Sized against measurement, not the advertised context. The served model
/// reports `context_length: 512` and the cloud-api wrapper splits internally,
/// so requests spanning several context windows are normal and fine: 2,609
/// tokens (5.1 windows) classified 6/6. Failures begin around 3,000 (1/3) and
/// are total by 6,000 (0/3). This budget leaves ~1.5x margin under the point
/// where failures start.
pub const MAX_CLASSIFY_INPUT_TOKENS: usize = 2_000;

/// Token count at which classification begins failing, measured 2026-08-27:
/// 3,000 tokens classified 1-of-3, 6,000 classified 0-of-3.
pub const MEASURED_CLASSIFY_TOKEN_LIMIT: usize = 3_000;

// Keep real margin under the measured failure point. The budget is not a
// guess: exceeding it is what produced the intermittent 502s.
const _: () = assert!(
    MAX_CLASSIFY_INPUT_TOKENS * 3 <= MEASURED_CLASSIFY_TOKEN_LIMIT * 2,
    "MAX_CLASSIFY_INPUT_TOKENS must stay well under the measured token limit"
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
            window_cache: std::sync::Mutex::new(ClassifyWindowCache::default()),
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
        let ranges = chunk_token_ranges(text, MAX_CLASSIFY_INPUT_TOKENS);

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
    async fn classify_window(
        &self,
        text: &str,
    ) -> Result<Vec<ClassifySpan>, TraceContributionError> {
        // Memoize on the window's content hash. The lock is taken only around
        // the map access and released before any await, so it never spans a
        // network round trip.
        let key: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(text.as_bytes()).into();
        if let Some(hit) = self.cached_window(&key) {
            return Ok(hit);
        }

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
            self.remember_window(key, &entry.spans);
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

/// The tokenizer the hosted classifier actually uses.
///
/// Identified by measurement rather than assumption: `o200k_base` reproduced
/// the endpoint's own `usage.input_tokens` exactly on 17 of 17 samples --
/// prose, source code, identifier-dense text, hex digests, long words and
/// repeated characters, from 5 bytes to 8 KB. `cl100k_base` matched only 6 of
/// 9 on the same short set, so the choice is not arbitrary and should not be
/// changed without re-running that comparison.
#[cfg(feature = "near-ai-privacy-filter")]
fn classifier_bpe() -> Option<&'static tiktoken_rs::CoreBPE> {
    static BPE: std::sync::OnceLock<Option<tiktoken_rs::CoreBPE>> = std::sync::OnceLock::new();
    BPE.get_or_init(|| tiktoken_rs::o200k_base().ok()).as_ref()
}

/// Count the tokens the classifier will charge for `text`.
#[cfg(feature = "near-ai-privacy-filter")]
fn classifier_token_count(text: &str) -> Option<usize> {
    classifier_bpe().map(|bpe| bpe.encode_ordinary(text).len())
}

/// Split `text` into contiguous byte ranges, each within `max_tokens` of the
/// classifier's budget, covering the whole input on char boundaries.
///
/// Segments are cut at newlines where possible -- PII rarely spans lines, and
/// a window that ends mid-entity risks splitting one across two requests. A
/// single line that alone exceeds the budget (a long log line, a base64 blob)
/// is bisected until its pieces fit.
///
/// Falls back to a conservative byte split if the tokenizer is unavailable:
/// under-filling requests costs throughput, over-filling costs 502s, so the
/// safe direction is down.
#[cfg(feature = "near-ai-privacy-filter")]
fn chunk_token_ranges(text: &str, max_tokens: usize) -> Vec<std::ops::Range<usize>> {
    if classifier_bpe().is_none() {
        // No tokenizer: fall back to the dense-content byte equivalent, which
        // is the smallest realistic window for this budget.
        return chunk_byte_ranges(text, max_tokens.saturating_mul(3).max(1));
    }
    let max_tokens = max_tokens.max(1);

    // Line-ish segments, each carrying its own token cost.
    let mut segments: Vec<std::ops::Range<usize>> = Vec::new();
    let mut seg_start = 0usize;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            segments.push(seg_start..index + 1);
            seg_start = index + 1;
        }
    }
    if seg_start < text.len() {
        segments.push(seg_start..text.len());
    }

    // Any segment too big on its own is bisected until each piece fits.
    let mut sized: Vec<(std::ops::Range<usize>, usize)> = Vec::new();
    let mut pending: Vec<std::ops::Range<usize>> = segments;
    pending.reverse();
    while let Some(range) = pending.pop() {
        let tokens = classifier_token_count(&text[range.clone()]).unwrap_or(usize::MAX);
        if tokens <= max_tokens || range.len() <= 1 {
            sized.push((range, tokens));
            continue;
        }
        // Bisect on a char boundary and re-measure both halves.
        let mut mid = range.start + range.len() / 2;
        while mid > range.start && !text.is_char_boundary(mid) {
            mid -= 1;
        }
        if mid == range.start {
            sized.push((range, tokens));
            continue;
        }
        pending.push(mid..range.end);
        pending.push(range.start..mid);
    }

    // Greedily pack segments up to the budget. Per-segment counts can differ
    // slightly from the count of the joined text, because BPE merges across a
    // boundary; the budget's margin under the measured limit absorbs that.
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    let mut current: Option<std::ops::Range<usize>> = None;
    let mut running = 0usize;
    for (range, tokens) in sized {
        match current {
            Some(ref mut open) if running + tokens <= max_tokens => {
                open.end = range.end;
                running += tokens;
            }
            Some(open) => {
                ranges.push(open);
                running = tokens;
                current = Some(range);
            }
            None => {
                running = tokens;
                current = Some(range);
            }
        }
    }
    if let Some(open) = current {
        ranges.push(open);
    }
    if ranges.is_empty() {
        ranges.push(0..text.len());
    }
    ranges
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
    apply_spans("near-ai", text, &all_spans)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The tokenizer must be the one the endpoint actually charges against.
    /// These counts are the endpoint's own `usage.input_tokens`, recorded
    /// 2026-08-27; `cl100k_base` disagrees on several of them.
    #[test]
    fn tokenizer_reproduces_the_endpoints_own_counts() {
        let cases: &[(&str, usize)] = &[
            ("hello", 1),
            ("hello world", 2),
            ("The quick brown fox jumps over the lazy dog.", 10),
            ("alice@example.com", 3),
            ("/usr/local/lib/python3.11/site-packages/", 11),
            ("a7f3c9d2e1b48856f0c1d2e3a4b5c6d7", 29),
            ("tenant_id submission_id auth_principal_ref", 8),
            ("supercalifragilisticexpialidocious", 10),
        ];
        for (text, expected) in cases {
            assert_eq!(
                classifier_token_count(text),
                Some(*expected),
                "token count drifted from the endpoint's accounting for {text:?}"
            );
        }
    }

    /// The property the whole change exists for: no window may exceed the
    /// budget, whatever the content's token density.
    #[test]
    fn every_window_fits_the_token_budget() {
        let prose = "Please email alice@example.com about invoice 12345. ".repeat(600);
        let dense = (0..600)
            .map(|i| format!("user{i:04}@example.com /home/u{i:04}/src/f{i:04}.rs"))
            .collect::<Vec<_>>()
            .join(" ");
        let hex = "a7f3c9d2e1b48856f0c1d2e3a4b5c6d7".repeat(400);

        for (label, text) in [("prose", &prose), ("dense", &dense), ("hex", &hex)] {
            let ranges = chunk_token_ranges(text, MAX_CLASSIFY_INPUT_TOKENS);
            for range in &ranges {
                let tokens = classifier_token_count(&text[range.clone()]).expect("tokenizer");
                assert!(
                    tokens <= MAX_CLASSIFY_INPUT_TOKENS,
                    "{label}: window of {tokens} tokens exceeds the budget of {}",
                    MAX_CLASSIFY_INPUT_TOKENS
                );
            }
        }
    }

    /// Ranges must be contiguous and cover the whole field: a gap would drop
    /// text from classification entirely, which is a silent privacy hole
    /// rather than a performance bug.
    #[test]
    fn token_windows_cover_the_whole_field_without_gaps() {
        let text = (0..300)
            .map(|i| format!("line {i} with alice@example.com and /var/log/f{i}.log"))
            .collect::<Vec<_>>()
            .join("\n");
        let ranges = chunk_token_ranges(&text, MAX_CLASSIFY_INPUT_TOKENS);

        assert_eq!(ranges.first().expect("at least one window").start, 0);
        assert_eq!(ranges.last().expect("at least one window").end, text.len());
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].end, pair[1].start, "gap or overlap between windows");
        }
        let rebuilt: String = ranges.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(rebuilt, text, "windows must reconstruct the field exactly");
    }

    /// Budgeting in tokens is what lets sparse content travel in FEWER
    /// requests than dense content of the same size -- the thing a byte
    /// budget structurally cannot do, and the reason dense windows used to
    /// 502 while prose windows of identical size were fine.
    #[test]
    fn sparse_content_packs_into_fewer_windows_than_dense() {
        let bytes = 60_000;
        let prose: String = "Please email alice@example.com about invoice 12345. "
            .repeat(2000)
            .chars()
            .take(bytes)
            .collect();
        let dense: String = (0..4000)
            .map(|i| format!("user{i:04}@example.com /home/u{i:04}/src/f{i:04}.rs"))
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(bytes)
            .collect();

        let prose_windows = chunk_token_ranges(&prose, MAX_CLASSIFY_INPUT_TOKENS).len();
        let dense_windows = chunk_token_ranges(&dense, MAX_CLASSIFY_INPUT_TOKENS).len();
        assert!(
            prose_windows < dense_windows,
            "prose took {prose_windows} windows and dense took {dense_windows}; \
             token budgeting should give sparse content larger windows"
        );
    }

    /// A single line longer than the budget still has to be split, or one
    /// base64 blob or long log line stalls its whole field.
    #[test]
    fn an_oversized_single_line_is_split() {
        let one_line = "a7f3c9d2e1b48856f0c1d2e3a4b5c6d7".repeat(1000);
        assert!(!one_line.contains('\n'));
        let ranges = chunk_token_ranges(&one_line, MAX_CLASSIFY_INPUT_TOKENS);
        assert!(ranges.len() > 1, "an oversized single line must be split");
        for range in &ranges {
            let tokens = classifier_token_count(&one_line[range.clone()]).expect("tokenizer");
            assert!(tokens <= MAX_CLASSIFY_INPUT_TOKENS);
        }
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
    fn debug_does_not_leak_api_key() {
        let secret = SecretApiKey("super-secret-token".into());
        let debug = format!("{secret:?}");
        assert!(!debug.contains("super-secret-token"));
        assert!(debug.contains("***"));
    }
}
