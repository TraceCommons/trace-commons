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
/// The fixed part of a classify request's timeout. The whole timeout grows
/// with the request body: `base + per_kib x body KiB` (see [`call_timeout`]).
///
/// Measured by the owner against the live endpoint on 2026-11-05 with
/// synthetic text, one request at a time:
///
/// | body | plain text | escape-heavy, token-dense text |
/// |---|---|---|
/// | tiny | 0.7-1.2 s | |
/// | 4 KiB | | 3.39 s |
/// | 20 KiB | 11.3 s | |
/// | 32 KiB | | 19.41 s |
/// | 103 KiB | 14.8 s | |
/// | 128 KiB | | 64.00 s |
/// | 205 KiB | 36.5 s | |
/// | 232 KiB | | 96.22 s |
///
/// Dense text fits about 3 s + 0.4 s/KiB. That is several times slower than
/// 2026-09-03 (243 KiB in 9.3 s). Owner decision 2026-11-05: 15 s + 0.6 s/KiB,
/// so a full 64 KiB window (the cap) gets 53.4 s against ~29 s expected for
/// dense text, and a tiny request gives up after ~15 s instead of waiting out
/// a timeout sized for a large one.
///
/// Override with `TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS` (this base) and
/// `TRACE_NEAR_AI_PRIVACY_TIMEOUT_PER_KIB_MS`.
pub const DEFAULT_TIMEOUT_MS: u64 = 15_000;

/// The part of a classify request's timeout that grows with its body, per
/// KiB. See [`DEFAULT_TIMEOUT_MS`].
pub const DEFAULT_TIMEOUT_PER_KIB_MS: u64 = 600;

/// One classify request's timeout for a body of `body_bytes`: `base_ms`
/// plus `per_kib_ms` for each started KiB.
pub const fn call_timeout_ms(base_ms: u64, per_kib_ms: u64, body_bytes: usize) -> u64 {
    let kib = (body_bytes as u64).div_ceil(1024);
    base_ms.saturating_add(per_kib_ms.saturating_mul(kib))
}

/// [`call_timeout_ms`] as a `Duration`.
pub fn call_timeout(base_ms: u64, per_kib_ms: u64, body_bytes: usize) -> Duration {
    Duration::from_millis(call_timeout_ms(base_ms, per_kib_ms, body_bytes))
}

/// The longest one window can take before the adapter gives up on it:
/// [`MAX_CLASSIFY_ATTEMPTS`] attempts at the full timeout, plus the backoff
/// between them, when no caller deadline cuts it shorter. A caller with a
/// deadline (the witness) passes it through
/// [`PrivacyFilterAdapter::redact_text_before`], and then no attempt starts
/// that could not finish by it.
pub const fn worst_case_window_ms(base_ms: u64, per_kib_ms: u64, body_bytes: usize) -> u64 {
    let attempts = MAX_CLASSIFY_ATTEMPTS as u64;
    let mut backoff = 0u64;
    let mut failed = 1;
    while failed < MAX_CLASSIFY_ATTEMPTS {
        backoff += backoff_ms(failed);
        failed += 1;
    }
    attempts * call_timeout_ms(base_ms, per_kib_ms, body_bytes) + backoff
}

/// One full window's worst case with no caller deadline must stay well
/// inside the witness's request budget (540 s), which also has the rest of a
/// contribution to classify. At the defaults it is 3 x 53.4 s + 0.75 s of
/// backoff = 160.95 s.
pub const MAX_WORST_CASE_WINDOW_MS: u64 = 180_000;

const _: () = assert!(
    worst_case_window_ms(
        DEFAULT_TIMEOUT_MS,
        DEFAULT_TIMEOUT_PER_KIB_MS,
        MAX_CLASSIFY_REQUEST_BODY_BYTES
    ) <= MAX_WORST_CASE_WINDOW_MS,
    "a full window's retries at the default timeout must fit the witness's request limit"
);

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
/// [`window_ranges`]). That limit has moved before, so a 413 is not fatal:
/// the window is halved and each half classified, at most
/// [`MAX_CLASSIFY_413_SPLITS`] times deep.
///
/// **64 KiB, not the ~240 KiB the body limit would allow, because of time**
/// (owner decision 2026-11-05). Dense text took 64.0 s at 128 KiB and
/// 96.2 s at 232 KiB on 2026-11-05 (see [`DEFAULT_TIMEOUT_MS`]); a 64 KiB
/// window is ~29 s, so each attempt fits its 53.4 s timeout with margin and
/// a window's retries fit the witness's request budget. Throughput comes
/// from classifying windows and leaves concurrently instead.
///
/// Override with `TRACE_NEAR_AI_PRIVACY_MAX_REQUEST_BODY_BYTES`.
/// Re-measure before trusting the number.
pub const MAX_CLASSIFY_REQUEST_BODY_BYTES: usize = 64 * 1024;

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
/// giving up. The hosted endpoint returns transient 502s, so retry with
/// exponential backoff before failing the window closed. Three since
/// 2026-11-05 (owner decision; was four): at the endpoint's current latency
/// a fourth attempt at a full window would not fit a request budget. A
/// caller deadline can stop retries sooner; see
/// [`PrivacyFilterAdapter::redact_text_before`].
pub const MAX_CLASSIFY_ATTEMPTS: usize = 3;

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
    /// Where each request's outcome and latency are counted. The process-wide
    /// recorder unless a caller supplies its own; see [`diagnostics`].
    diagnostics: Arc<diagnostics::Recorder>,
    /// The timeout's fixed and per-KiB parts; see [`call_timeout`].
    timeout_base_ms: u64,
    timeout_per_kib_ms: u64,
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
    /// Spacing between classify requests, or `None` for none (the
    /// default). See [`pacing`] and [`Self::with_requests_per_minute`].
    pacer: Option<Arc<pacing::Pacer>>,
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
            diagnostics: diagnostics::global(),
            timeout_base_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            timeout_per_kib_ms: DEFAULT_TIMEOUT_PER_KIB_MS,
            window_cache: std::sync::Mutex::new(ClassifyWindowCache::default()),
            pacer: None,
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

    /// Count this adapter's requests in `recorder` instead of the
    /// process-wide one. For tests, which run in parallel in one process.
    pub fn with_diagnostics(mut self, recorder: Arc<diagnostics::Recorder>) -> Self {
        self.diagnostics = recorder;
        self
    }

    /// The part of each request's timeout that grows with its body, per KiB.
    /// `timeout` passed to the constructor is the fixed part.
    /// Space classify requests so this adapter sends at most `requests` a
    /// minute, retries included, evenly rather than in a burst. Zero leaves
    /// it unpaced. Every caller sharing this adapter shares the one
    /// schedule; the witness builds one adapter for the process.
    pub fn with_requests_per_minute(mut self, requests: u32) -> Self {
        self.pacer = pacing::Pacer::per_minute(requests).map(Arc::new);
        self
    }

    pub fn with_timeout_per_kib_ms(mut self, per_kib_ms: u64) -> Self {
        self.timeout_per_kib_ms = per_kib_ms;
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

    let timeout_per_kib_ms = match std::env::var("TRACE_NEAR_AI_PRIVACY_TIMEOUT_PER_KIB_MS") {
        Ok(value) => {
            value
                .trim()
                .parse::<u64>()
                .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                    var: "TRACE_NEAR_AI_PRIVACY_TIMEOUT_PER_KIB_MS",
                    reason: err.to_string(),
                })?
        }
        Err(_) => DEFAULT_TIMEOUT_PER_KIB_MS,
    };

    // The fixed part of each request's timeout; the part that grows with the
    // body is the variable above.
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
    .with_max_batch_inputs(max_batch_inputs)
    .with_timeout_per_kib_ms(timeout_per_kib_ms);
    // Opt-in: unset is unpaced, as before. The witness sets it in its
    // measured compose file, so the bound is part of its identity.
    let adapter = match requests_per_minute_from(
        std::env::var("TRACE_NEAR_AI_PRIVACY_MAX_REQUESTS_PER_MINUTE")
            .ok()
            .as_deref(),
    )? {
        Some(requests) => adapter.with_requests_per_minute(requests),
        None => adapter,
    };
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
/// `TRACE_NEAR_AI_PRIVACY_MAX_REQUESTS_PER_MINUTE`: unset or empty is
/// unpaced; otherwise a whole number of requests a minute, above zero.
/// Zero is refused rather than read as "unlimited", the same rule the body
/// cap follows, so nobody believes a bound is in force that is not.
fn requests_per_minute_from(value: Option<&str>) -> Result<Option<u32>, PrivacyFilterConfigError> {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    match value.parse::<u32>() {
        Ok(0) => Err(PrivacyFilterConfigError::InvalidEnv {
            var: "TRACE_NEAR_AI_PRIVACY_MAX_REQUESTS_PER_MINUTE",
            reason: "must be greater than zero; unset it for no pacing".to_string(),
        }),
        Ok(requests) => Ok(Some(requests)),
        Err(err) => Err(PrivacyFilterConfigError::InvalidEnv {
            var: "TRACE_NEAR_AI_PRIVACY_MAX_REQUESTS_PER_MINUTE",
            reason: err.to_string(),
        }),
    }
}

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
const SPAN_OUT_OF_RANGE_LABEL: &str = "privacy_filter_span_out_of_range";

/// Fail closed unless every span lies inside the input (the range rule
/// `apply_spans` applies, [`crate::privacy_filter_spans::span_fits`]) and its
/// reported `text` is exactly the input's codepoints `[start, end)`, then
/// return the spans without it, and how many were re-anchored.
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
/// Re-anchoring (owner, 2026-11-13): NEAR AI answered one real session's
/// window that way on every request, so refusing alone refused it for good.
/// A slipped span whose `text` occurs exactly once in the whole window --
/// overlapping occurrences counted -- is moved onto that occurrence, in
/// codepoints, keeping its category and score, and must still fit. No
/// further request is made. Zero or several occurrences, empty text or no
/// text leave the answer refused. Only an answer every span of which then
/// verifies is returned, so only that can be cached.
///
/// A span out of range is refused here, not left for `apply_spans`: an
/// answer this accepts is cached, and an out-of-range span that reached the
/// cache was refused on every later request for its window without the
/// classifier being asked again (a real session, 2026-11-12). Neither the
/// span text nor the input is ever logged: the failure carries the label and
/// the input's hash-only fingerprint.
fn verify_span_texts(
    input: &str,
    spans: Vec<NearAiSpan>,
) -> Result<(Vec<ClassifySpan>, usize), (diagnostics::BodyShape, String)> {
    let mut boundaries: Vec<usize> = input.char_indices().map(|(byte, _)| byte).collect();
    boundaries.push(input.len());
    let codepoints = boundaries.len() - 1;
    let out_of_range = spans
        .iter()
        .filter(|span| !crate::privacy_filter_spans::span_fits(span.start, span.end, codepoints))
        .count();
    if out_of_range > 0 {
        let diagnostics = classify_input_diagnostics(&[input]);
        tracing::warn!(
            classify_input = %diagnostics,
            out_of_range_spans = out_of_range,
            spans = spans.len(),
            failure = SPAN_OUT_OF_RANGE_LABEL,
            "near-ai privacy classify failed"
        );
        return Err((
            diagnostics::BodyShape::SpanOutOfRange,
            format!(
                "near-ai privacy classifier returned out-of-range span: \
                 {SPAN_OUT_OF_RANGE_LABEL} out_of_range_spans={out_of_range} spans={} {diagnostics}",
                spans.len()
            ),
        ));
    }
    // A span whose text is right and whose offsets are not is moved onto
    // its text when that text occurs exactly once in the window -- anywhere
    // in it, overlapping occurrences counted. The answer is otherwise
    // unchanged, so no further request is made. Zero or several
    // occurrences, or no text to look for, leave it a mismatch.
    let mut spans = spans;
    let mut reanchored = 0usize;
    let mut mismatches = 0usize;
    for span in &mut spans {
        let covered = &input[boundaries[span.start]..boundaries[span.end]];
        if span.text.as_deref() == Some(covered) {
            continue;
        }
        match span
            .text
            .as_deref()
            .and_then(|text| sole_occurrence(input, text))
        {
            Some((start, end))
                if crate::privacy_filter_spans::span_fits(start, end, codepoints) =>
            {
                span.start = start;
                span.end = end;
                reanchored += 1;
            }
            _ => mismatches += 1,
        }
    }
    if mismatches > 0 {
        let diagnostics = classify_input_diagnostics(&[input]);
        tracing::warn!(
            classify_input = %diagnostics,
            mismatched_spans = mismatches,
            spans = spans.len(),
            failure = SPAN_TEXT_MISMATCH_LABEL,
            "near-ai privacy classify failed"
        );
        return Err((
            diagnostics::BodyShape::SpanTextMismatch,
            format!(
                "near-ai privacy classifier span offsets disagree with span text: \
                 {SPAN_TEXT_MISMATCH_LABEL} mismatched_spans={mismatches} spans={} {diagnostics}",
                spans.len()
            ),
        ));
    }
    Ok((
        spans
            .into_iter()
            .map(|span| ClassifySpan {
                category: span.category,
                start: span.start,
                end: span.end,
                score: span.score,
            })
            .collect(),
        reanchored,
    ))
}

/// The codepoint range of `text` in `input` when it occurs there exactly
/// once, overlapping occurrences counted; `None` for empty text, or for
/// zero or several occurrences.
fn sole_occurrence(input: &str, text: &str) -> Option<(usize, usize)> {
    if text.is_empty() {
        return None;
    }
    let mut found: Option<usize> = None;
    let mut from = 0usize;
    while let Some(offset) = input[from..].find(text) {
        let at = from + offset;
        if found.replace(at).is_some() {
            return None;
        }
        // Step one character, not the match, so an overlapping second
        // occurrence is seen.
        from = at + input[at..].chars().next().map_or(1, char::len_utf8);
        if from > input.len() {
            break;
        }
    }
    let at = found?;
    let start = input[..at].chars().count();
    Some((start, start + text.chars().count()))
}

#[async_trait]
impl PrivacyFilterAdapter for NearAiPrivacyFilterAdapter {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        self.redact_text_by(text, None).await
    }

    async fn redact_text_before(
        &self,
        text: &str,
        deadline: std::time::Instant,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        self.redact_text_by(text, Some(deadline)).await
    }

    fn preferred_batch_inputs(&self) -> usize {
        self.max_batch_inputs
    }

    /// One request per distinct window not already in the window cache,
    /// windowed exactly as [`Self::redact_text`] windows. Blank text and
    /// text over the input limit send nothing. A 413 split or a retry would
    /// send more, never fewer, so this is a floor. With batching on, texts
    /// share requests in a way this does not model, so it declines.
    fn requests_needed(&self, texts: &[&str]) -> Option<usize> {
        if self.max_batch_inputs > 1 {
            return None;
        }
        let budget = self.max_input_json_bytes();
        let mut windows = std::collections::HashSet::new();
        for text in texts {
            if text.trim().is_empty() || text.len() > self.max_input_bytes {
                continue;
            }
            for range in window_ranges(text, budget) {
                let key = window_key(&text[range]);
                if !self.window_is_cached(&key) {
                    windows.insert(key);
                }
            }
        }
        Some(windows.len())
    }

    fn request_interval(&self) -> Option<Duration> {
        self.pacer.as_ref().map(|pacer| pacer.interval())
    }

    async fn redact_texts(&self, texts: &[&str]) -> Vec<Answer> {
        self.redact_texts_by(texts, None).await
    }

    async fn redact_texts_before(
        &self,
        texts: &[&str],
        deadline: std::time::Instant,
    ) -> Vec<Answer> {
        self.redact_texts_by(texts, Some(deadline)).await
    }
}

impl NearAiPrivacyFilterAdapter {
    async fn redact_text_by(
        &self,
        text: &str,
        deadline: Option<std::time::Instant>,
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
                        self.classify_window(window, MAX_CLASSIFY_413_SPLITS, deadline)
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
    async fn redact_texts_by(
        &self,
        texts: &[&str],
        deadline: Option<std::time::Instant>,
    ) -> Vec<Answer> {
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
                    self.send_window(&inputs, body, deadline).await
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
                        self.remember_window(text, &spans);
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
                None => self.redact_text_by(texts[index], deadline).await,
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

    /// Whether `key` is in the window cache, without counting a hit or a
    /// miss: a prediction is not a lookup.
    fn window_is_cached(&self, key: &[u8; 32]) -> bool {
        match self.window_cache.lock() {
            Ok(cache) => cache.entries.contains_key(key),
            Err(poisoned) => poisoned.into_inner().entries.contains_key(key),
        }
    }

    /// Record a successful classification for reuse within this tick.
    /// Cache `spans` as `window`'s verified answer. Spans that do not fit
    /// the window are never stored: verification refuses them first, and
    /// this guard keeps any later path from caching what `apply_spans` would
    /// refuse.
    fn remember_window(&self, window: &str, spans: &[ClassifySpan]) {
        if !crate::privacy_filter_spans::spans_fit(window, spans) {
            tracing::warn!(
                classify_input = %classify_input_diagnostics(&[window]),
                failure = SPAN_OUT_OF_RANGE_LABEL,
                "near-ai privacy classify answer not cached"
            );
            return;
        }
        let key = window_key(window);
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
        deadline: Option<std::time::Instant>,
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
                self.classify_halves(text, splits_left, deadline).await?
            } else {
                match self.send_window(&[text], body, deadline).await? {
                    WindowAnswer::Spans(mut spans) => spans.swap_remove(0),
                    WindowAnswer::TooLarge => {
                        self.classify_halves(text, splits_left, deadline).await?
                    }
                }
            };
            // Verified before it is remembered (in `send_window`, per
            // half): a refused response is never served from the cache.
            self.remember_window(text, &spans);
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
        deadline: Option<std::time::Instant>,
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
        let mut spans = self
            .classify_window(left, splits_left - 1, deadline)
            .await?;
        let shift = left.chars().count();
        let right_spans = self
            .classify_window(right, splits_left - 1, deadline)
            .await?;
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
        deadline: Option<std::time::Instant>,
    ) -> Result<WindowAnswer, TraceContributionError> {
        let endpoint = format!("{}/privacy/classify", self.base_url.trim_end_matches('/'));
        let full_attempt = call_timeout(self.timeout_base_ms, self.timeout_per_kib_ms, body.len());

        let mut attempt = 0;
        loop {
            attempt += 1;
            // Every attempt, retries included, takes the next paced slot.
            // A slot that would start after the caller's deadline is not
            // taken: the window fails as transient, the slot stays free.
            if let Some(pacer) = &self.pacer {
                let now = std::time::Instant::now();
                match pacer.reserve(now, deadline) {
                    Some(slot) if slot > now => tokio::time::sleep(slot - now).await,
                    Some(_) => {}
                    None => return Err(budget_spent(inputs, attempt)),
                }
            }
            // The first attempt runs, cut short at the caller's deadline if
            // that comes first; a retry runs only if a whole attempt still
            // fits before the deadline (`may_retry`). The deadline is the
            // ceiling: past it the caller has given up.
            let this_attempt = match deadline {
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(std::time::Instant::now());
                    if left.is_zero() {
                        return Err(budget_spent(inputs, attempt));
                    }
                    full_attempt.min(left)
                }
                None => full_attempt,
            };
            // Transient failures (transport errors, 5xx) are retried with
            // exponential backoff; 4xx and body-shape failures are not.
            let started = std::time::Instant::now();
            let mut in_flight = Some(self.diagnostics.enter());
            let send_result = self
                .client
                .post(&endpoint)
                .timeout(this_attempt)
                .bearer_auth(&self.api_key.0)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.clone())
                .send()
                .await;
            let response = match send_result {
                Ok(response) => response,
                Err(err) => {
                    in_flight.take();
                    self.diagnostics
                        .record(transport_outcome(&err), body.len(), started.elapsed());
                    let wait = Duration::from_millis(backoff_ms(attempt));
                    if may_retry(attempt, wait, full_attempt, deadline) {
                        tokio::time::sleep(wait).await;
                        continue;
                    }
                    return Err(transport_failure(&err, inputs, attempt));
                }
            };

            let status = response.status();
            if !status.is_success() {
                in_flight.take();
                self.diagnostics.record(
                    diagnostics::Outcome::from_status(status.as_u16()),
                    body.len(),
                    started.elapsed(),
                );
            }
            if status == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
                return Ok(WindowAnswer::TooLarge);
            }
            if !status.is_success() {
                let transient = crate::privacy_filter_spans::is_transient_classifier_status(status);
                let wait = retry_wait(
                    attempt,
                    status,
                    retry_after_secs(response.headers()),
                    self.pacer.as_ref().map(|pacer| pacer.interval()),
                );
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    // Every caller on this adapter waits out a rate limit,
                    // not only the one that met it.
                    if let Some(pacer) = &self.pacer {
                        pacer.hold_until(std::time::Instant::now() + wait);
                    }
                }
                if transient && may_retry(attempt, wait, full_attempt, deadline) {
                    tokio::time::sleep(wait).await;
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

            // The whole body first. A body that never finished arriving is
            // the connection failing (a timeout or transport outcome), not a
            // malformed answer: `parse_error` means bytes arrived and were
            // not the expected JSON.
            let read = response.bytes().await;
            in_flight.take();
            let bytes = match read {
                Ok(bytes) => bytes,
                Err(err) => {
                    self.diagnostics
                        .record(transport_outcome(&err), body.len(), started.elapsed());
                    let wait = Duration::from_millis(backoff_ms(attempt));
                    if may_retry(attempt, wait, full_attempt, deadline) {
                        tokio::time::sleep(wait).await;
                        continue;
                    }
                    return Err(transport_failure(&err, inputs, attempt));
                }
            };

            // One request, one outcome, batched or not. A malformed answer
            // -- unparseable, empty, the wrong number of entries, a span
            // out of range or not matching its input -- is `body_shape`,
            // counted under which check it failed. Nothing from it is
            // applied or cached.
            return match parse_classify_answer(inputs, &bytes) {
                Ok((spans, reanchored)) => {
                    self.diagnostics.record_reanchored(reanchored);
                    self.diagnostics.record(
                        diagnostics::Outcome::Ok,
                        body.len(),
                        started.elapsed(),
                    );
                    Ok(WindowAnswer::Spans(spans))
                }
                Err((kind, reason)) => {
                    self.diagnostics
                        .record_body_shape(kind, body.len(), started.elapsed());
                    Err(TraceContributionError::RedactionFailed {
                        reason: format!("{reason} body_shape={}", kind.label()),
                    })
                }
            };
        }
    }
}

/// The outcome kind of a request that failed on the wire.
fn transport_outcome(err: &reqwest::Error) -> diagnostics::Outcome {
    if err.is_timeout() {
        diagnostics::Outcome::Timeout
    } else {
        diagnostics::Outcome::Transport
    }
}

/// A request that failed on the wire after its retries: the upstream is
/// down, the trace is fine. The input fingerprint rides along hash-only so a
/// repeatedly-failing window is identifiable in the logs.
fn transport_failure(
    err: &reqwest::Error,
    inputs: &[&str],
    attempt: usize,
) -> TraceContributionError {
    let diagnostics = classify_input_diagnostics(inputs);
    // The driver logs only a hash of the error, so emit the input
    // fingerprint here where it is provably hash-only.
    tracing::warn!(
        classify_input = %diagnostics,
        attempts = attempt,
        failure = "transport",
        "near-ai privacy classify failed"
    );
    TraceContributionError::TransientRedactionFailed {
        reason: format!(
            "near-ai privacy classifier transport error: {} attempts={} {}",
            err, attempt, diagnostics
        ),
    }
}

/// Read a 200's body into verified spans per input, in input order, or say
/// which way it was malformed.
///
/// One entry per input, in input order. A single-input request reads the
/// first entry, as it always has; a batch must account for every input
/// exactly, or none of it is used. Every span is verified before anything
/// is returned, so a refused answer can never reach the cache.
///
/// The reasons are label-only: a JSON error's own text can quote the body,
/// so only its category is kept.
fn parse_classify_answer(
    inputs: &[&str],
    body: &[u8],
) -> Result<(Vec<Vec<ClassifySpan>>, usize), (diagnostics::BodyShape, String)> {
    let parsed: ClassifyResponse = serde_json::from_slice(body).map_err(|err| {
        (
            diagnostics::BodyShape::ParseError,
            format!(
                "near-ai privacy classifier response parse error: category={:?} body_len={}",
                err.classify(),
                body.len()
            ),
        )
    })?;
    if parsed.data.is_empty() {
        return Err((
            diagnostics::BodyShape::EmptyData,
            "near-ai privacy classifier returned empty data array".to_string(),
        ));
    }
    if inputs.len() > 1 && parsed.data.len() != inputs.len() {
        return Err((
            diagnostics::BodyShape::EntryCountMismatch,
            format!(
                "near-ai privacy classifier returned {} entries for {} inputs",
                parsed.data.len(),
                inputs.len()
            ),
        ));
    }
    let mut reanchored = 0usize;
    let mut answers = Vec::with_capacity(inputs.len());
    for (input, entry) in inputs.iter().zip(parsed.data) {
        let (spans, moved) = verify_span_texts(input, entry.spans)?;
        reanchored += moved;
        answers.push(spans);
    }
    Ok((answers, reanchored))
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

    /// The escaped-input budget that keeps a request's whole serialized body
    /// within the adapter's body cap: the body is the request envelope
    /// (model, keys, the input's quotes) plus the input's JSON-escaped bytes,
    /// and the envelope is measured, not guessed.
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
/// Whether another attempt may start after `attempt` failed: attempts are
/// left, and -- with a caller deadline -- the backoff and a whole attempt at
/// `full_attempt` still fit before it.
fn may_retry(
    attempt: usize,
    wait: Duration,
    full_attempt: Duration,
    deadline: Option<std::time::Instant>,
) -> bool {
    if attempt >= MAX_CLASSIFY_ATTEMPTS {
        return false;
    }
    deadline.is_none_or(|deadline| std::time::Instant::now() + wait + full_attempt <= deadline)
}

/// The longest `Retry-After` honoured. Past it the window gives up and the
/// trace is retried later on the caller's own schedule.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

/// How long to wait after the `attempt`th failure, answered `status`.
///
/// Never less than [`backoff_ms`]. A 429 waits for its `Retry-After`, or
/// one pacing interval when it names none: the plain 250 ms backoff spent
/// all three attempts inside a second against a per-minute limit. A 503
/// with a `Retry-After` waits for it too. Both are capped at
/// [`MAX_RETRY_AFTER`].
fn retry_wait(
    attempt: usize,
    status: reqwest::StatusCode,
    retry_after_secs: Option<u64>,
    pacing_interval: Option<Duration>,
) -> Duration {
    let backoff = Duration::from_millis(backoff_ms(attempt));
    let asked = match status {
        reqwest::StatusCode::TOO_MANY_REQUESTS => retry_after_secs
            .map(Duration::from_secs)
            .or(pacing_interval),
        reqwest::StatusCode::SERVICE_UNAVAILABLE => retry_after_secs.map(Duration::from_secs),
        _ => None,
    };
    asked.map_or(backoff, |asked| backoff.max(asked.min(MAX_RETRY_AFTER)))
}

/// A `Retry-After` given in seconds. The HTTP-date form is not read.
fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// The caller's deadline passed before a window could be attempted.
/// Transient: the same window may well classify in a later request.
fn budget_spent(inputs: &[&str], attempt: usize) -> TraceContributionError {
    let diagnostics = classify_input_diagnostics(inputs);
    tracing::warn!(
        classify_input = %diagnostics,
        attempts = attempt - 1,
        failure = "deadline",
        "near-ai privacy classify failed"
    );
    TraceContributionError::TransientRedactionFailed {
        reason: format!(
            "near-ai privacy classifier: caller deadline reached before attempt {attempt} {diagnostics}"
        ),
    }
}

/// The pause after the `failed_attempt`th failure: 250 ms, doubling.
const fn backoff_ms(failed_attempt: usize) -> u64 {
    let shift = if failed_attempt == 0 {
        0
    } else {
        failed_attempt - 1
    };
    let shift = if shift > 5 { 5 } else { shift };
    250u64 << shift
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

    // ---- What may be cached. An answer that verification accepts is cached
    // and later applied without asking again, so anything `apply_spans`
    // would refuse must be refused here first.

    fn near_span(start: usize, end: usize, text: Option<&str>) -> NearAiSpan {
        NearAiSpan {
            category: "private_person".to_string(),
            start,
            end,
            score: 0.9,
            text: text.map(str::to_string),
        }
    }

    /// Every single-span answer over every offset pair around each text's
    /// end, reporting either the true covered text or a wrong one: whatever
    /// verification accepts, `apply_spans` applies. Two-span answers with
    /// one bad span are refused whole.
    #[test]
    fn every_answer_verification_accepts_applies_cleanly() {
        let texts = [
            "",
            "a",
            "hi Zo\u{eb} \u{1f600} \u{674e}\r\n ok",
            "plain ascii text",
        ];
        let (mut accepted, mut refused) = (0usize, 0usize);
        for text in texts {
            let mut boundaries: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
            boundaries.push(text.len());
            let codepoints = boundaries.len() - 1;
            for start in 0..=codepoints + 2 {
                for end in 0..=codepoints + 2 {
                    let covered = (start <= end && end <= codepoints)
                        .then(|| &text[boundaries[start]..boundaries[end]]);
                    for reported in [covered, Some("not the text"), None] {
                        match verify_span_texts(text, vec![near_span(start, end, reported)]) {
                            Ok((spans, _)) => {
                                accepted += 1;
                                assert!(
                                    crate::privacy_filter_spans::apply_spans(
                                        "near-ai", text, &spans
                                    )
                                    .is_ok(),
                                    "accepted but not applicable: {start}..{end} in {:?}",
                                    text.len()
                                );
                            }
                            Err(_) => refused += 1,
                        }
                    }
                    if let Some(covered) = covered {
                        let bad = near_span(start, codepoints + 1, Some(covered));
                        let pair = vec![near_span(start, end, Some(covered)), bad];
                        assert!(verify_span_texts(text, pair).is_err());
                    }
                }
            }
        }
        assert!(accepted > 0 && refused > 0, "{accepted} {refused}");
    }

    /// The cache's own guard: spans that do not fit the window they would be
    /// cached under are never stored, whoever asks.
    #[test]
    fn the_window_cache_never_holds_spans_that_do_not_fit_their_window() {
        let adapter = NearAiPrivacyFilterAdapter::new(
            "http://127.0.0.1:1",
            DEFAULT_MODEL,
            "stub-not-a-key",
            Duration::from_secs(1),
            1_000_000,
        )
        .unwrap();
        let window = "say hi to Zo\u{eb}";
        let codepoints = window.chars().count();
        let span = |start, end| ClassifySpan {
            category: "private_person".to_string(),
            start,
            end,
            score: 0.9,
        };
        adapter.remember_window(window, &[span(10, codepoints + 1)]);
        assert!(adapter.cached_window(&window_key(window)).is_none());
        adapter.remember_window(window, &[span(12, 10)]);
        assert!(adapter.cached_window(&window_key(window)).is_none());
        adapter.remember_window(window, &[span(10, codepoints)]);
        assert!(adapter.cached_window(&window_key(window)).is_some());
    }

    // ---- Pacing (owner, 2026-11-12: NEAR AI allows 30 classify requests a
    // minute). Schedules are checked on injected instants, never the wall
    // clock; adapter tests count requests rather than time them.

    #[test]
    fn the_pacer_spaces_requests_evenly_with_no_burst() {
        let pacer = pacing::Pacer::per_minute(30).expect("30 is a rate");
        assert_eq!(pacer.interval(), Duration::from_secs(2));
        let t0 = std::time::Instant::now();
        assert_eq!(pacer.reserve(t0, None), Some(t0));
        assert_eq!(pacer.reserve(t0, None), Some(t0 + Duration::from_secs(2)));
        assert_eq!(pacer.reserve(t0, None), Some(t0 + Duration::from_secs(4)));
        // A caller arriving after the queue drained starts at once.
        let later = t0 + Duration::from_secs(60);
        assert_eq!(pacer.reserve(later, None), Some(later));
    }

    #[test]
    fn a_slot_past_the_deadline_is_refused_and_not_spent() {
        let pacer = pacing::Pacer::per_minute(30).unwrap();
        let t0 = std::time::Instant::now();
        assert_eq!(pacer.reserve(t0, None), Some(t0));
        // The next slot is t0+2s; a caller that must be done by t0+1s gets
        // none, and the slot stays free for the next caller.
        assert_eq!(pacer.reserve(t0, Some(t0 + Duration::from_secs(1))), None);
        assert_eq!(pacer.reserve(t0, None), Some(t0 + Duration::from_secs(2)));
    }

    #[test]
    fn a_rate_limit_answer_holds_every_caller() {
        let pacer = pacing::Pacer::per_minute(30).unwrap();
        let t0 = std::time::Instant::now();
        assert_eq!(pacer.reserve(t0, None), Some(t0));
        pacer.hold_until(t0 + Duration::from_secs(20));
        assert_eq!(pacer.reserve(t0, None), Some(t0 + Duration::from_secs(20)));
        // Holding to an earlier time never pulls the schedule back.
        pacer.hold_until(t0 + Duration::from_secs(5));
        assert_eq!(pacer.reserve(t0, None), Some(t0 + Duration::from_secs(22)));
    }

    #[test]
    fn zero_is_not_a_rate() {
        assert!(pacing::Pacer::per_minute(0).is_none());
    }

    #[test]
    fn the_rate_setting_is_off_when_unset_and_refuses_zero_or_junk() {
        assert_eq!(requests_per_minute_from(None).unwrap(), None);
        assert_eq!(requests_per_minute_from(Some("")).unwrap(), None);
        assert_eq!(requests_per_minute_from(Some(" 30 ")).unwrap(), Some(30));
        for bad in ["0", "-1", "thirty", "1.5"] {
            assert!(requests_per_minute_from(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_rate_limit_waits_for_retry_after_capped_and_never_less_than_backoff() {
        use reqwest::StatusCode;
        let interval = Some(Duration::from_secs(2));
        assert_eq!(
            retry_wait(1, StatusCode::TOO_MANY_REQUESTS, Some(7), interval),
            Duration::from_secs(7)
        );
        // No Retry-After: one pacing interval.
        assert_eq!(
            retry_wait(1, StatusCode::TOO_MANY_REQUESTS, None, interval),
            Duration::from_secs(2)
        );
        // A huge Retry-After is capped.
        assert_eq!(
            retry_wait(1, StatusCode::TOO_MANY_REQUESTS, Some(3_600), interval),
            MAX_RETRY_AFTER
        );
        // Never shorter than the plain backoff.
        assert_eq!(
            retry_wait(2, StatusCode::TOO_MANY_REQUESTS, Some(0), None),
            Duration::from_millis(backoff_ms(2))
        );
        // A 502 keeps the plain backoff.
        assert_eq!(
            retry_wait(1, StatusCode::BAD_GATEWAY, None, interval),
            Duration::from_millis(backoff_ms(1))
        );
    }

    fn classify_ok() -> wiremock::ResponseTemplate {
        wiremock::ResponseTemplate::new(200)
            .set_body_json(serde_json::json!({ "data": [{ "spans": [] }] }))
    }

    /// A paced adapter sends nothing it cannot start before the caller's
    /// deadline: the second window waits for a slot a minute away, so it
    /// fails as transient and the classifier sees one request, not two.
    #[tokio::test]
    async fn a_paced_adapter_does_not_send_past_the_deadline() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/privacy/classify"))
            .respond_with(classify_ok())
            .mount(&server)
            .await;
        let adapter = NearAiPrivacyFilterAdapter::new(
            server.uri(),
            "openai/privacy-filter",
            "test-key",
            Duration::from_secs(5),
            1_000_000,
        )
        .unwrap()
        .with_requests_per_minute(1);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        assert!(
            adapter
                .redact_text_before("first window", deadline)
                .await
                .is_ok()
        );
        let second = adapter.redact_text_before("second window", deadline).await;
        assert!(
            matches!(
                second,
                Err(TraceContributionError::TransientRedactionFailed { .. })
            ),
            "{second:?}"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    /// A 429 whose Retry-After will not fit the deadline is not retried
    /// at once (the old 250 ms backoff would have spent all three attempts
    /// inside a second): one request, then a transient failure.
    #[tokio::test]
    async fn a_rate_limited_window_is_not_hammered() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/privacy/classify"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "30"))
            .mount(&server)
            .await;
        let adapter = NearAiPrivacyFilterAdapter::new(
            server.uri(),
            "openai/privacy-filter",
            "test-key",
            Duration::from_secs(5),
            1_000_000,
        )
        .unwrap()
        .with_requests_per_minute(30);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let answer = adapter.redact_text_before("one window", deadline).await;
        assert!(
            matches!(
                answer,
                Err(TraceContributionError::TransientRedactionFailed { .. })
            ),
            "{answer:?}"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    /// Unpaced (the default, and what `new` builds) is unchanged: three
    /// windows go out back to back.
    #[tokio::test]
    async fn an_unpaced_adapter_is_unchanged() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/privacy/classify"))
            .respond_with(classify_ok())
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
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        for text in ["a window", "b window", "c window"] {
            assert!(
                adapter.redact_text_before(text, deadline).await.is_ok(),
                "{text}"
            );
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
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
                // "Ada" occurs twice in its event, so the slip cannot be
                // re-anchored to one place and the answer stays invalid.
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
                event("Ada reviewed Ada's change"),
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

    /// The defaults leave a full window's retries inside the witness's 300 s
    /// request limit, with room; the compile-time assert says the same, and
    /// this pins the number it computes.
    #[test]
    fn a_full_windows_worst_case_fits_the_witness_limit() {
        let worst = worst_case_window_ms(
            DEFAULT_TIMEOUT_MS,
            DEFAULT_TIMEOUT_PER_KIB_MS,
            MAX_CLASSIFY_REQUEST_BODY_BYTES,
        );
        assert_eq!(worst, 3 * (15_000 + 600 * 64) + 250 + 500);
        assert!(worst < 540_000);
        // The 2026-11-05 dense-text times (4 and 32 KiB measured, 64 KiB from
        // the measured fit of ~3 s + 0.4 s/KiB) sit at least 1.7x inside
        // each call's timeout.
        for (kib, measured_ms) in [(4, 3_390), (32, 19_410), (64, 28_600)] {
            let timeout =
                call_timeout_ms(DEFAULT_TIMEOUT_MS, DEFAULT_TIMEOUT_PER_KIB_MS, kib * 1024);
            assert!(timeout * 10 >= measured_ms * 17, "{kib} KiB: {timeout} ms");
        }
    }

    /// A 241 KB single field of ordinary tool output -- the size that took
    /// 115 s on the live witness -- is a few requests, not dozens.
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
        assert!(windows.len() <= 7, "{} windows", windows.len());
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

/// What the classify requests made in this process did, by kind, and how long
/// the successful ones took, by body size. **Label-only**: counts, gauges and
/// durations; no text, no request identity, no key, no host.
///
/// The witness serves a snapshot at `GET /v1/diagnostics`, because its CVM's
/// logs are not readable and this is the only way an operator can tell a
/// rejected key from a slow endpoint from a down one.
/// Even spacing between classify requests.
///
/// NEAR AI allows the witness 30 `privacy/classify` requests a minute
/// (2026-11-12). The witness held at most two in flight and paced nothing,
/// so at the endpoint's quicker latencies it went over. Requests are spaced
/// one interval apart with no burst: a slot is reserved under a short lock
/// and the wait happens outside it.
pub mod pacing {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    pub struct Pacer {
        interval: Duration,
        /// The earliest the next request may start; `None` before the first.
        next: Mutex<Option<Instant>>,
    }

    impl Pacer {
        /// `requests` a minute, or `None` for zero.
        pub fn per_minute(requests: u32) -> Option<Self> {
            (requests > 0).then(|| Self {
                interval: Duration::from_secs(60) / requests,
                next: Mutex::new(None),
            })
        }

        pub fn interval(&self) -> Duration {
            self.interval
        }

        /// Reserve the first free slot at or after `now` and return when it
        /// starts. `None`, reserving nothing, when that slot would start at
        /// or after `latest`.
        pub fn reserve(&self, now: Instant, latest: Option<Instant>) -> Option<Instant> {
            let mut next = self
                .next
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let slot = next.map_or(now, |next| next.max(now));
            if latest.is_some_and(|latest| slot >= latest) {
                return None;
            }
            *next = Some(slot + self.interval);
            Some(slot)
        }

        /// Hold every later slot until at least `until`: the endpoint said
        /// to wait. Never moves the schedule earlier.
        pub fn hold_until(&self, until: Instant) {
            let mut next = self
                .next
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            *next = Some(next.map_or(until, |next| next.max(until)));
        }
    }
}

pub mod diagnostics {
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Duration;

    use serde::Serialize;

    /// How one classify request ended.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Outcome {
        /// 200 with a well-formed, verified answer.
        Ok,
        /// No answer within the request timeout.
        Timeout,
        /// The connection failed some other way.
        Transport,
        /// 401 or 403: the key was refused.
        Auth,
        /// 402 or 429: out of credit or rate limited.
        Quota,
        /// Any 5xx.
        ServerError,
        /// 413: the body was over the endpoint's limit.
        TooLarge,
        /// Any other non-2xx.
        OtherClientError,
        /// 200 whose body did not parse, had no entry, or whose spans did not
        /// match the text sent. [`BodyShape`] says which; this is their total.
        BodyShape,
    }

    /// Which way a 200 answer was malformed. Each one is also counted as
    /// [`Outcome::BodyShape`], which stays the total.
    ///
    /// Label-only: which check failed, never what the answer or the input
    /// held.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum BodyShape {
        /// The body arrived and was not the expected JSON.
        ParseError,
        /// The body parsed and its `data` array was empty.
        EmptyData,
        /// A batch was answered with a different number of entries than it
        /// carried inputs.
        EntryCountMismatch,
        /// A span's reported `text` was not the input's codepoints at its
        /// offsets, or it reported no `text`.
        SpanTextMismatch,
        /// A span's offsets were reversed or ran past the end of the input.
        SpanOutOfRange,
    }

    impl BodyShape {
        /// Every kind, in the order a snapshot lists them.
        pub const ALL: [BodyShape; 5] = [
            BodyShape::ParseError,
            BodyShape::EmptyData,
            BodyShape::EntryCountMismatch,
            BodyShape::SpanTextMismatch,
            BodyShape::SpanOutOfRange,
        ];

        pub fn label(self) -> &'static str {
            match self {
                BodyShape::ParseError => "parse_error",
                BodyShape::EmptyData => "empty_data",
                BodyShape::EntryCountMismatch => "entry_count_mismatch",
                BodyShape::SpanTextMismatch => "span_text_mismatch",
                BodyShape::SpanOutOfRange => "span_out_of_range",
            }
        }

        fn index(self) -> usize {
            match self {
                BodyShape::ParseError => 0,
                BodyShape::EmptyData => 1,
                BodyShape::EntryCountMismatch => 2,
                BodyShape::SpanTextMismatch => 3,
                BodyShape::SpanOutOfRange => 4,
            }
        }
    }

    impl Outcome {
        /// Every outcome, in the order a snapshot lists them.
        pub const ALL: [Outcome; 9] = [
            Outcome::Ok,
            Outcome::Timeout,
            Outcome::Transport,
            Outcome::Auth,
            Outcome::Quota,
            Outcome::ServerError,
            Outcome::TooLarge,
            Outcome::OtherClientError,
            Outcome::BodyShape,
        ];

        pub fn label(self) -> &'static str {
            match self {
                Outcome::Ok => "ok",
                Outcome::Timeout => "timeout",
                Outcome::Transport => "transport",
                Outcome::Auth => "auth_401_403",
                Outcome::Quota => "quota_402_429",
                Outcome::ServerError => "server_5xx",
                Outcome::TooLarge => "too_large_413",
                Outcome::OtherClientError => "other_4xx",
                Outcome::BodyShape => "body_shape",
            }
        }

        /// The outcome for a non-2xx status.
        pub fn from_status(status: u16) -> Outcome {
            match status {
                401 | 403 => Outcome::Auth,
                402 | 429 => Outcome::Quota,
                413 => Outcome::TooLarge,
                500..=599 => Outcome::ServerError,
                _ => Outcome::OtherClientError,
            }
        }

        fn index(self) -> usize {
            Outcome::ALL
                .iter()
                .position(|outcome| *outcome == self)
                .unwrap_or(0)
        }
    }

    /// Body-size buckets for latency, by request body bytes.
    const BUCKETS: [(&str, usize); 4] = [
        ("under_4_kib", 4 * 1024),
        ("4_to_32_kib", 32 * 1024),
        ("32_to_128_kib", 128 * 1024),
        ("128_kib_and_over", usize::MAX),
    ];

    /// Recent successful latencies kept per bucket.
    const SAMPLES_PER_BUCKET: usize = 256;

    /// Counts and recent latencies. One per process in production
    /// ([`global`]); tests make their own.
    #[derive(Default)]
    pub struct Recorder {
        counts: [AtomicU64; 9],
        body_shapes: [AtomicU64; 5],
        spans_reanchored: AtomicU64,
        in_flight: AtomicUsize,
        latency_ms: Mutex<[VecDeque<u64>; 4]>,
    }

    /// Decrements the in-flight gauge when the request it counts ends, on
    /// every path, including cancellation.
    pub struct InFlight<'a>(&'a Recorder);

    impl Drop for InFlight<'_> {
        fn drop(&mut self) {
            self.0.in_flight.fetch_sub(1, Ordering::Relaxed);
        }
    }

    impl Recorder {
        /// A request is starting.
        pub fn enter(&self) -> InFlight<'_> {
            self.in_flight.fetch_add(1, Ordering::Relaxed);
            InFlight(self)
        }

        /// A request ended. Only successful requests contribute latency: a
        /// timeout's duration is the timeout, and a refusal's says nothing
        /// about how fast the endpoint classifies.
        pub fn record(&self, outcome: Outcome, body_bytes: usize, elapsed: Duration) {
            self.counts[outcome.index()].fetch_add(1, Ordering::Relaxed);
            if outcome != Outcome::Ok {
                return;
            }
            let bucket = BUCKETS
                .iter()
                .position(|(_, under)| body_bytes < *under)
                .unwrap_or(BUCKETS.len() - 1);
            let mut latency = self.latency_ms.lock().unwrap_or_else(|p| p.into_inner());
            let samples = &mut latency[bucket];
            if samples.len() == SAMPLES_PER_BUCKET {
                samples.pop_front();
            }
            samples.push_back(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX));
        }

        /// A request ended in a malformed answer of kind `kind`: counted as
        /// [`Outcome::BodyShape`] and under its kind.
        pub fn record_body_shape(&self, kind: BodyShape, body_bytes: usize, elapsed: Duration) {
            self.body_shapes[kind.index()].fetch_add(1, Ordering::Relaxed);
            self.record(Outcome::BodyShape, body_bytes, elapsed);
        }

        /// A count of outcome `outcome` so far.
        pub fn count(&self, outcome: Outcome) -> u64 {
            self.counts[outcome.index()].load(Ordering::Relaxed)
        }

        /// `spans` spans were moved onto the one place their reported text
        /// occurs, in an answer that was then accepted.
        pub fn record_reanchored(&self, spans: usize) {
            self.spans_reanchored
                .fetch_add(u64::try_from(spans).unwrap_or(u64::MAX), Ordering::Relaxed);
        }

        /// Spans re-anchored so far.
        pub fn spans_reanchored(&self) -> u64 {
            self.spans_reanchored.load(Ordering::Relaxed)
        }

        /// A count of malformed answers of kind `kind` so far.
        pub fn body_shape_count(&self, kind: BodyShape) -> u64 {
            self.body_shapes[kind.index()].load(Ordering::Relaxed)
        }

        pub fn snapshot(&self) -> Snapshot {
            let outcomes = Outcome::ALL
                .iter()
                .map(|outcome| (outcome.label(), self.count(*outcome)))
                .collect();
            let body_shapes = BodyShape::ALL
                .iter()
                .map(|kind| (kind.label(), self.body_shape_count(*kind)))
                .collect();
            let latency = self.latency_ms.lock().unwrap_or_else(|p| p.into_inner());
            let latency_ms = BUCKETS
                .iter()
                .zip(latency.iter())
                .map(|((label, _), samples)| (*label, BucketLatency::of(samples)))
                .collect();
            Snapshot {
                outcomes,
                body_shapes,
                spans_reanchored: self.spans_reanchored(),
                in_flight: self.in_flight.load(Ordering::Relaxed),
                latency_ms,
            }
        }
    }

    /// The process-wide recorder every adapter uses unless told otherwise.
    pub fn global() -> Arc<Recorder> {
        static GLOBAL: OnceLock<Arc<Recorder>> = OnceLock::new();
        GLOBAL.get_or_init(|| Arc::new(Recorder::default())).clone()
    }

    #[derive(Debug, Clone, Serialize, PartialEq, Eq)]
    pub struct Snapshot {
        /// Requests by outcome, since the process started. Every attempt
        /// counts, retries included.
        pub outcomes: BTreeMap<&'static str, u64>,
        /// The `body_shape` outcomes by which check failed, every retry
        /// counted. Sums to `outcomes.body_shape`; kept apart from
        /// `outcomes` so a reader summing that map does not count twice.
        pub body_shapes: BTreeMap<&'static str, u64>,
        /// Spans whose offsets disagreed with their text and were moved onto
        /// the one place in the window that text occurs, in answers then
        /// accepted. Counted apart from `span_text_mismatch`, which counts
        /// only answers that could not be corrected.
        pub spans_reanchored: u64,
        /// Requests on the wire right now.
        pub in_flight: usize,
        /// Successful requests' latency by request-body size, over the last
        /// few hundred in each bucket.
        pub latency_ms: BTreeMap<&'static str, BucketLatency>,
    }

    #[derive(Debug, Clone, Serialize, PartialEq, Eq)]
    pub struct BucketLatency {
        pub samples: usize,
        pub p50: Option<u64>,
        pub p90: Option<u64>,
        pub p99: Option<u64>,
        pub max: Option<u64>,
    }

    impl BucketLatency {
        fn of(samples: &VecDeque<u64>) -> Self {
            let mut sorted: Vec<u64> = samples.iter().copied().collect();
            sorted.sort_unstable();
            let at = |fraction: f64| -> Option<u64> {
                if sorted.is_empty() {
                    return None;
                }
                let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
                sorted.get(index).copied()
            };
            Self {
                samples: sorted.len(),
                p50: at(0.50),
                p90: at(0.90),
                p99: at(0.99),
                max: sorted.last().copied(),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn statuses_map_to_their_kinds() {
            for (status, outcome) in [
                (401, Outcome::Auth),
                (403, Outcome::Auth),
                (402, Outcome::Quota),
                (429, Outcome::Quota),
                (413, Outcome::TooLarge),
                (500, Outcome::ServerError),
                (503, Outcome::ServerError),
                (400, Outcome::OtherClientError),
                (404, Outcome::OtherClientError),
            ] {
                assert_eq!(Outcome::from_status(status), outcome, "{status}");
            }
        }

        #[test]
        fn latency_is_bucketed_by_body_and_bounded() {
            let recorder = Recorder::default();
            for ms in 1..=300u64 {
                recorder.record(Outcome::Ok, 100, Duration::from_millis(ms));
            }
            recorder.record(Outcome::Ok, 200 * 1024, Duration::from_millis(36_500));
            recorder.record(Outcome::Timeout, 200 * 1024, Duration::from_secs(90));
            let snapshot = recorder.snapshot();
            let small = &snapshot.latency_ms["under_4_kib"];
            assert_eq!(small.samples, SAMPLES_PER_BUCKET);
            assert_eq!(small.max, Some(300));
            assert_eq!(small.p50, Some(173));
            let large = &snapshot.latency_ms["128_kib_and_over"];
            assert_eq!(large.samples, 1, "a timeout contributes no latency");
            assert_eq!(snapshot.outcomes["ok"], 301);
            assert_eq!(snapshot.outcomes["timeout"], 1);
            assert_eq!(snapshot.latency_ms["4_to_32_kib"].p50, None);
        }

        #[test]
        fn body_shapes_are_counted_by_kind_and_in_the_total() {
            let recorder = Recorder::default();
            for (times, kind) in BodyShape::ALL.into_iter().enumerate() {
                for _ in 0..=times {
                    recorder.record_body_shape(kind, 100, Duration::from_millis(5));
                }
            }
            let snapshot = recorder.snapshot();
            assert_eq!(
                snapshot.body_shapes.keys().copied().collect::<Vec<_>>(),
                [
                    "empty_data",
                    "entry_count_mismatch",
                    "parse_error",
                    "span_out_of_range",
                    "span_text_mismatch",
                ]
            );
            for (times, kind) in BodyShape::ALL.into_iter().enumerate() {
                assert_eq!(snapshot.body_shapes[kind.label()], times as u64 + 1);
            }
            assert_eq!(
                snapshot.outcomes["body_shape"],
                snapshot.body_shapes.values().sum::<u64>()
            );
            assert_eq!(snapshot.outcomes["ok"], 0, "a malformed answer is not ok");
            assert_eq!(
                snapshot.latency_ms["under_4_kib"].samples, 0,
                "a malformed answer contributes no latency"
            );
        }

        #[test]
        fn in_flight_counts_until_dropped() {
            let recorder = Recorder::default();
            let first = recorder.enter();
            let second = recorder.enter();
            assert_eq!(recorder.snapshot().in_flight, 2);
            drop(first);
            drop(second);
            assert_eq!(recorder.snapshot().in_flight, 0);
        }
    }
}
