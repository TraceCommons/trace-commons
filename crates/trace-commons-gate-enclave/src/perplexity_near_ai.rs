//! NEAR AI Cloud-backed perplexity + token-rarity scorer.
//!
//! Posts a single `/v1/completions` request with `echo=true` + `logprobs=N`
//! against a direct model endpoint (e.g. `https://qwen3-30b.completions.near.ai/v1`)
//! and reads `logprobs.token_logprobs` over the prompt tokens. One round-trip
//! satisfies both [`PerplexityScorer`] and [`TokenRarityScorer`] — the
//! returned `top_logprobs` slice carries the K-rarest data the rarity metric
//! consumes, so callers asking for both metrics on the same trace pay one HTTP
//! call, not two.
//!
//! ### Why a remote scorer alongside the local one
//!
//! The local mistralrs path (`perplexity_local.rs`) pins a single-GPU host to
//! the deployment. The NEAR AI path moves inference into a TEE-hosted vLLM
//! (Intel TDX + NVIDIA GPU TEE), so the gate-service binary no longer needs
//! `local-gpu-models-cuda` for production scoring. Smoke-validated 2026-05-17
//! against `qwen3-6-35b.completions.near.ai` and `qwen3-30b.completions.near.ai`.
//!
//! ### Endpoint
//!
//! Use the per-model direct endpoint, not the gateway. The gateway
//! (`cloud-api.near.ai`) does not route `/v1/completions` — only
//! `/v1/chat/completions` — and the chat path injects a chat template that
//! pollutes the prompt with role tokens we'd then have to score.
//!
//! ### Hash-only logging
//!
//! Plaintext never logs, never serializes outside the request body, never
//! leaves process memory. Error paths carry the configured model label only.
//! The API key is never logged.
//!
//! ### Retry policy
//!
//! A single request is attempted up to `MAX_SCORE_ATTEMPTS` times with
//! 250ms-doubling backoff, matching the privacy-filter adapter that talks to
//! the same vendor and sees the same intermittent 502s. Two things make this
//! narrower than the classifier's policy: a scoring retry is a billed
//! inference over a whole chunk rather than a cheap classify call, and a
//! chunked trace issues one of these per chunk. So only failures that
//! produced no completion are re-issued — transport, 5xx, 429 — while a
//! timeout, the one failure that may already have run (and billed) the
//! inference, is never retried.
//!
//! ### Fail-closed contract
//!
//! Any HTTP error, JSON parse failure, missing field, non-finite logprob, or
//! token-count mismatch propagates as `Err` — matching the trait contract that
//! a refused score must not silently substitute zero (which would pass any
//! positive perplexity floor).

#![cfg(feature = "near-ai-scorer")]

use crate::perplexity::{
    ChunkPerplexity, PerplexityResult, PerplexityScorer, TokenRarityResult, TokenRarityScorer,
};
use crate::perplexity_local::{aggregate_perplexity_metrics, per_token_rarity_micros};
use anyhow::{Context, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use trace_commons_gate_api::{ScorerFailure, scorer_status_is_transient};

/// Static configuration for the NEAR AI scorer.
///
/// All fields are required and validated at construction. The model label is
/// the only one of these that surfaces in logs — it is intentionally treated
/// as non-secret. `api_key` is never logged.
#[derive(Debug, Clone)]
pub struct NearAiScorerConfig {
    /// Base URL of a NEAR AI direct-completions endpoint, including the `/v1`
    /// suffix and no trailing slash. Example:
    /// `https://qwen3-30b.completions.near.ai/v1`.
    pub base_url: String,
    /// `model` field sent in the request body. Must match the hosted model ID,
    /// e.g. `Qwen/Qwen3-30B-A3B-Instruct-2507`.
    pub model: String,
    /// Bearer token issued by cloud.near.ai.
    pub api_key: String,
    /// Tail-fraction logprob cutoff in nats, passed through to
    /// `aggregate_perplexity_metrics`. Mirrors the local scorer's
    /// `tail_logprob_cutoff` argument.
    pub tail_logprob_cutoff: f32,
    /// Per-call `logprobs` value sent to the API. Production default is 1:
    /// both metrics here consume only the realized-token NLL, and
    /// `echo + prompt_logprobs` memory on the TEE backend scales with this
    /// value. NEAR AI's hosted vLLM accepts 1..=5.
    pub logprobs_top_k: u32,
    /// HTTP timeout for a single scoring request.
    pub timeout: Duration,
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
        // Neither http nor https: not a URL shape this scorer will post to.
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

impl NearAiScorerConfig {
    /// Validate the config. Called at construction; cheap.
    fn validate(&self) -> anyhow::Result<()> {
        if self.base_url.is_empty() {
            bail!("NearAiScorerConfigBaseUrlMissing");
        }
        if self.base_url.ends_with('/') {
            bail!("NearAiScorerConfigBaseUrlTrailingSlash");
        }
        // The API key rides on every request as a bearer token, so this URL
        // decides who receives a credential -- it is a disclosure surface,
        // not just a routing detail. Require TLS, exempting loopback (which
        // never reaches a network). Fail-closed rather than silently
        // shipping the key in the clear.
        if !base_url_is_tls_or_loopback(&self.base_url) {
            bail!("NearAiScorerConfigBaseUrlNotTls");
        }
        if self.model.is_empty() {
            bail!("NearAiScorerConfigModelMissing");
        }
        if self.api_key.is_empty() {
            bail!("NearAiScorerConfigApiKeyMissing");
        }
        if !self.tail_logprob_cutoff.is_finite() {
            bail!("NearAiScorerConfigTailCutoffNonFinite");
        }
        if self.logprobs_top_k == 0 || self.logprobs_top_k > 5 {
            // NEAR AI's hosted vLLM accepts logprobs up to 5; values outside
            // that band would be silently rejected upstream, fail-closed.
            bail!("NearAiScorerConfigLogprobsTopKOutOfRange");
        }
        Ok(())
    }
}

/// HTTP-backed scorer using `reqwest::blocking`.
///
/// `reqwest::blocking::Client` runs its own tokio current-thread runtime on a
/// dedicated worker thread internally — same dedicated-thread pattern that
/// `LocalPerplexityScorer` uses to avoid the "Cannot start a runtime from
/// within a runtime" panic when the sync trait method is called inside the
/// bake-off binary's `#[tokio::main]` context. Keeping the bridging inside
/// reqwest means we don't have to hand-roll the mpsc job loop.
pub struct NearAiPerplexityScorer {
    cfg: NearAiScorerConfig,
    client: reqwest::blocking::Client,
}

/// How many times a single `/v1/completions` scoring request is attempted
/// before the scorer gives up.
///
/// Lower than the privacy classifier's `MAX_CLASSIFY_ATTEMPTS` (4) on
/// purpose. A classify retry is a small, cheap call; a scoring retry is a
/// billed inference over a whole chunk, and a chunked trace multiplies the
/// budget by its chunk count. Three attempts absorb the single-blip case the
/// vendor's 502s actually produce without turning one flaky second into a
/// tripled inference bill for a fifteen-chunk trace.
pub const MAX_SCORE_ATTEMPTS: usize = 3;

/// Whether a failed attempt is worth issuing again.
///
/// Deliberately NOT the same question as `scorer_status_is_transient`, which
/// decides whether a failure may be charged against the trace's attempt
/// budget. A 402 is transient for accounting -- the trace did nothing wrong,
/// so it must not lose a budgeted attempt -- and is still pointless to retry:
/// the correct response to an exhausted credit balance is to stall the queue,
/// not to re-ask three times a second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryClass {
    Retry,
    Fatal,
}

/// A failed attempt, carrying both the error the caller will see and whether
/// the loop should spend another attempt on it.
struct AttemptError {
    class: RetryClass,
    err: anyhow::Error,
}

/// Retry classification for a response that arrived with a non-2xx status.
///
/// 5xx and 429 are the vendor telling us to come back -- its own 502 body
/// text reads "Please try again later". Everything else in 4xx was rejected
/// on the request's own terms and will be rejected identically next time.
fn status_retry_class(status: u16) -> RetryClass {
    if status == 429 || (500..600).contains(&status) {
        RetryClass::Retry
    } else {
        RetryClass::Fatal
    }
}

/// Retry classification for a request that never produced a usable response.
///
/// A timeout is the one transport failure that may already have cost money:
/// the deadline can expire while the backend is midway through an inference
/// it will bill for, and the chunk that blew a 30s deadline once is likely to
/// blow it again. Retrying that doubles a large bill for a request that
/// probably fails the same way. Connect failures, TLS failures, and bodies
/// that died mid-stream produced no completion at all, so retrying them is
/// close to free -- that asymmetry is what keeps a per-request retry budget
/// from multiplying cost across a chunked trace.
fn transport_retry_class(is_timeout: bool) -> RetryClass {
    if is_timeout {
        RetryClass::Fatal
    } else {
        RetryClass::Retry
    }
}

/// Backoff before re-issuing `failed_attempt`. Same 250ms-doubling shape the
/// privacy classifier uses, so the two NEAR AI callers behave alike under the
/// same outage.
fn score_backoff_delay(failed_attempt: usize) -> Duration {
    let millis = 250u64.saturating_mul(1u64 << (failed_attempt.saturating_sub(1)).min(5));
    Duration::from_millis(millis)
}

/// Run `attempt_fn` up to [`MAX_SCORE_ATTEMPTS`] times, sleeping via `sleep`
/// between retryable failures. Returns the last attempt's error when the
/// budget is spent, classification intact, so the scoring driver's
/// transient/permanent accounting is unchanged by the retries happening at
/// all.
///
/// `sleep` is injected so the loop's attempt count and backoff sequence are
/// unit-testable without either HTTP or wall-clock time; production passes
/// `std::thread::sleep`.
fn fetch_with_retry<T>(
    mut attempt_fn: impl FnMut(usize) -> Result<T, AttemptError>,
    mut sleep: impl FnMut(Duration),
) -> anyhow::Result<T> {
    let mut attempt = 0usize;
    loop {
        attempt += 1;
        match attempt_fn(attempt) {
            Ok(value) => return Ok(value),
            Err(AttemptError { class, err }) => {
                if class == RetryClass::Fatal || attempt >= MAX_SCORE_ATTEMPTS {
                    return Err(err);
                }
                sleep(score_backoff_delay(attempt));
            }
        }
    }
}

impl NearAiPerplexityScorer {
    pub fn try_new(cfg: NearAiScorerConfig) -> anyhow::Result<Self> {
        cfg.validate()?;

        let client = reqwest::blocking::Client::builder()
            .timeout(cfg.timeout)
            // Do not follow redirects: the request carries a bearer API key,
            // and a redirect would hand the validated endpoint's authority to
            // a host that was never validated. Matching the ingest binary,
            // which builds every outbound client with this policy.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("NearAiScorerHttpClientBuildFailed")?;

        Ok(Self { cfg, client })
    }

    /// Build a scoring request body. Kept factored out so the request shape is
    /// asserted on in unit tests without exercising HTTP.
    fn build_request(&self, plaintext: &[u8]) -> anyhow::Result<CompletionsRequest> {
        // The API only accepts UTF-8 prompts. Non-UTF-8 traces fail-closed
        // here rather than silently lossy-converting (which would change the
        // tokenization and produce a misleading score).
        let prompt = std::str::from_utf8(plaintext)
            .map_err(|_| anyhow!("NearAiScorerPromptNotUtf8"))?
            .to_owned();
        Ok(CompletionsRequest {
            model: self.cfg.model.clone(),
            prompt,
            // `max_tokens: 0` would be cheaper but NEAR AI still bills one
            // completion token; using 1 keeps the response shape consistent
            // (the realized next token's logprob is appended to the prompt
            // logprobs, and aggregate_perplexity_metrics already handles a
            // trailing prediction position).
            max_tokens: 1,
            logprobs: self.cfg.logprobs_top_k,
            echo: true,
            temperature: 0.0,
            // Streaming would split the response across SSE chunks; vLLM
            // returns logprobs in the same shape either way, but the
            // non-streaming branch is what the existing inference-proxy
            // logprobs encryption test path covers, so it's the safer choice
            // operationally.
            stream: false,
        })
    }

    /// Issue the HTTP call and return the parsed `logprobs` slice. Shared
    /// between [`PerplexityScorer::score`] and
    /// [`TokenRarityScorer::score_rarity`] so a caller computing both metrics
    /// on the same trace pays a single round-trip per scorer-pair invocation
    /// (the orchestrator must wire that sharing; this method does not cache).
    fn fetch_logprobs(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
        // Built once, outside the retry loop: a non-UTF-8 prompt is a
        // property of the trace, not of the upstream, and must never consume
        // an attempt.
        let req = self.build_request(plaintext)?;
        let url = format!("{}/completions", self.cfg.base_url);

        // Backoff sleeps on the calling thread. The production caller reaches
        // this inside `tokio::task::spawn_blocking`, so the sleep does hold a
        // blocking-pool thread -- but that thread is already held for up to
        // `cfg.timeout` (30s in production) per attempt, and the entire retry
        // budget adds at most 750ms of sleep on top. Moving the retries above
        // the blocking boundary would mean making `PerplexityScorer` async
        // across every implementation and both orchestrator call sites, for a
        // sub-3% change in how long the thread is held.
        fetch_with_retry(
            |_attempt| self.fetch_logprobs_once(&url, &req),
            std::thread::sleep,
        )
    }

    /// One `/v1/completions` round-trip. Every failure is classified for both
    /// questions the callers ask independently: whether the trace may be
    /// charged for it ([`ScorerFailure`]) and whether it is worth issuing
    /// again ([`RetryClass`]).
    fn fetch_logprobs_once(
        &self,
        url: &str,
        req: &CompletionsRequest,
    ) -> Result<Vec<f32>, AttemptError> {
        // Strip the URL from any reqwest transport error before it enters an
        // error chain: reqwest's Display embeds the request URL, and error
        // labels are recorded/logged under the hash-only convention (no raw
        // URLs). `without_url` keeps the error kind (timeout/connect/decode)
        // while dropping the endpoint.
        let resp = self
            .client
            .post(url)
            .bearer_auth(&self.cfg.api_key)
            .json(req)
            .send()
            .map_err(|e| {
                // The request never reached the model: a connect failure, a
                // TLS failure, or the client timeout expiring. Nothing about
                // the trace produced this, so it must not cost the trace its
                // budget -- but only the non-timeout cases are worth
                // re-issuing.
                AttemptError {
                    class: transport_retry_class(e.is_timeout()),
                    err: ScorerFailure::TransientScorerFailed {
                        reason: format!("NearAiScorerHttpSendFailed: {}", e.without_url()),
                    }
                    .into(),
                }
            })?;
        let status = resp.status();
        let body = resp.text().map_err(|e| {
            // A response that started and then died mid-body is the same
            // class of upstream failure as never connecting at all.
            AttemptError {
                class: transport_retry_class(e.is_timeout()),
                err: ScorerFailure::TransientScorerFailed {
                    reason: format!("NearAiScorerHttpBodyReadFailed: {}", e.without_url()),
                }
                .into(),
            }
        })?;
        if !status.is_success() {
            // Body may contain a vLLM validation message; surface its
            // length only, not its content (provider strings are not
            // hash-only audit material).
            //
            // `scorer_status_is_transient` decides whether the caller may
            // charge this to the trace. The label text is identical either
            // way -- the classification travels in the variant, never in the
            // string -- so attempt labels already recorded in production stay
            // byte-comparable with new ones.
            let reason = format!(
                "NearAiScorerHttpStatusError status={} body_len={}",
                status.as_u16(),
                body.len()
            );
            return Err(AttemptError {
                class: status_retry_class(status.as_u16()),
                err: if scorer_status_is_transient(status.as_u16()) {
                    ScorerFailure::TransientScorerFailed { reason }.into()
                } else {
                    ScorerFailure::ScorerFailed { reason }.into()
                },
            });
        }

        // Body-shape failures are Fatal: a 2xx whose body we cannot read is
        // the backend speaking a schema we do not understand, and asking the
        // same question again produces the same answer at full inference
        // cost. They stay permanent for attempt accounting too -- unlike a
        // transport blip, this one is reproducible.
        parse_logprobs_body(&body).map_err(|err| AttemptError {
            class: RetryClass::Fatal,
            err,
        })
    }
}

/// Parse a 2xx `/v1/completions` body into its realized-token logprob slice.
/// Split out of the HTTP path so the wire-shape contract unit-tests without
/// a request, and so the retry loop has one place to classify body-shape
/// failures.
fn parse_logprobs_body(body: &str) -> anyhow::Result<Vec<f32>> {
    let parsed: CompletionsResponse =
        serde_json::from_str(body).context("NearAiScorerResponseParseFailed")?;
    let choice = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("NearAiScorerResponseEmptyChoices"))?;
    let lp = choice
        .logprobs
        .ok_or_else(|| anyhow!("NearAiScorerResponseLogprobsMissing"))?;

    if lp.token_logprobs.is_empty() {
        bail!("NearAiScorerResponseTokenLogprobsEmpty");
    }
    // NEAR AI returns `null` for token 0 (no prior context). Map to 0.0;
    // `aggregate_perplexity_metrics` drops the first element regardless.
    // Positions 1..N must be finite — non-finite there is a degenerate
    // model output and fails closed via the helper.
    let mut out = Vec::with_capacity(lp.token_logprobs.len());
    for (i, v) in lp.token_logprobs.into_iter().enumerate() {
        match v {
            Some(f) => out.push(f as f32),
            None if i == 0 => out.push(0.0),
            None => bail!("NearAiScorerResponseNullLogprobInBody"),
        }
    }
    Ok(out)
}

/// Convert a raw logprob slice (element 0 = BOS placeholder, dropped) into
/// [`ChunkPerplexity`]. Factored out of `score_chunk` so it unit-tests
/// without HTTP. Fail-closed parallel to `aggregate_perplexity_metrics`:
/// short or non-finite input collapses to a zero-token chunk.
fn chunk_perplexity_from_logprobs(logprobs: &[f32], tail_logprob_cutoff: f32) -> ChunkPerplexity {
    if logprobs.len() < 2 || logprobs[1..].iter().any(|lp| !lp.is_finite()) {
        return ChunkPerplexity {
            sum_nll: 0.0,
            tokens: 0,
            tail_tokens: 0,
            logprobs: Vec::new(),
        };
    }
    let usable = &logprobs[1..];
    let sum_nll: f64 = usable.iter().map(|lp| -(*lp as f64)).sum();
    let tail_tokens = usable
        .iter()
        .filter(|&&lp| lp < tail_logprob_cutoff)
        .count() as u64;
    ChunkPerplexity {
        sum_nll,
        tokens: usable.len() as u64,
        tail_tokens,
        logprobs: usable.to_vec(),
    }
}

impl PerplexityScorer for NearAiPerplexityScorer {
    fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
        let logprobs = self.fetch_logprobs(plaintext)?;
        Ok(aggregate_perplexity_metrics(
            &logprobs,
            self.cfg.tail_logprob_cutoff,
        ))
    }

    fn score_chunk(&self, chunk: &[u8]) -> anyhow::Result<ChunkPerplexity> {
        let logprobs = self.fetch_logprobs(chunk)?;
        Ok(chunk_perplexity_from_logprobs(
            &logprobs,
            self.cfg.tail_logprob_cutoff,
        ))
    }
}

impl TokenRarityScorer for NearAiPerplexityScorer {
    fn score_rarity(&self, plaintext: &[u8], k: usize) -> anyhow::Result<TokenRarityResult> {
        let logprobs = self.fetch_logprobs(plaintext)?;
        // Mirrors the local scorer: token 0 is dropped, K is capped at the
        // count of usable tokens, K=0 collapses to zero.
        let token_rarity_micros = per_token_rarity_micros(&logprobs, k);
        let tokens_scored = logprobs.len().saturating_sub(1) as u64;
        let k_eff = (k.min(tokens_scored as usize)).min(u32::MAX as usize) as u32;
        Ok(TokenRarityResult {
            token_rarity_micros,
            tokens_scored,
            k: k_eff,
        })
    }
}

// ---------------------------------------------------------------------------
// Wire shapes — minimal subset of the OpenAI /v1/completions schema we depend
// on. Kept private; the public surface is the scorer types above.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct CompletionsRequest {
    model: String,
    prompt: String,
    max_tokens: u32,
    logprobs: u32,
    echo: bool,
    temperature: f32,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct CompletionsResponse {
    choices: Vec<CompletionsChoice>,
}

#[derive(Debug, Deserialize)]
struct CompletionsChoice {
    logprobs: Option<LogprobsBlock>,
}

#[derive(Debug, Deserialize)]
struct LogprobsBlock {
    /// One entry per token in the echoed prompt + each generated token. The
    /// first entry is `null` (no prior context for token 0). vLLM emits f64
    /// values; we narrow to f32 to match the local scorer's arithmetic.
    token_logprobs: Vec<Option<f64>>,
}

// ---------------------------------------------------------------------------
// Tests — request-shape only; the HTTP path is not exercised in CI (it would
// burn budget and require a live API key). A `#[ignore]`d integration test
// gated on `TRACE_COMMONS_NEAR_AI_INTEGRATION=1` + `TRACE_COMMONS_NEAR_AI_API_KEY` is
// the appropriate place to validate the wire round-trip.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_cfg() -> NearAiScorerConfig {
        NearAiScorerConfig {
            base_url: "https://qwen3-30b.completions.near.ai/v1".to_string(),
            model: "Qwen/Qwen3-30B-A3B-Instruct-2507".to_string(),
            api_key: "sk-test".to_string(),
            tail_logprob_cutoff: -8.0,
            // Production default (mirrors
            // TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K = 1): perplexity
            // needs only the realized token's logprob; k=1 cuts backend
            // memory + response size ~5x vs the old 5.
            logprobs_top_k: 1,
            timeout: Duration::from_secs(30),
        }
    }

    #[test]
    fn config_validates() {
        ok_cfg().validate().unwrap();
    }

    #[test]
    fn config_rejects_trailing_slash() {
        let mut c = ok_cfg();
        c.base_url.push('/');
        assert!(c.validate().is_err());
    }

    /// The bearer API key goes to whatever host `base_url` names, so a
    /// plaintext endpoint would put it on the wire. Loopback stays allowed:
    /// the request never reaches a network.
    #[test]
    fn config_rejects_plaintext_non_loopback_base_url() {
        for rejected in [
            "http://qwen3-30b.completions.near.ai/v1",
            "http://evil.example.com/v1",
            "http://127.0.0.1.evil.example.com/v1",
            "http://user@evil.example.com/v1",
            "ftp://qwen3-30b.completions.near.ai/v1",
            "qwen3-30b.completions.near.ai/v1",
        ] {
            let mut c = ok_cfg();
            c.base_url = rejected.to_string();
            assert!(
                c.validate().is_err(),
                "plaintext or non-http base URL must be refused: {rejected}"
            );
        }

        for allowed in [
            "https://qwen3-30b.completions.near.ai/v1",
            "http://localhost:8000/v1",
            "http://127.0.0.1:8000/v1",
            "http://[::1]:8000/v1",
        ] {
            let mut c = ok_cfg();
            c.base_url = allowed.to_string();
            assert!(
                c.validate().is_ok(),
                "tls or loopback base URL must be accepted: {allowed}"
            );
        }
    }

    #[test]
    fn config_rejects_empty_api_key() {
        let mut c = ok_cfg();
        c.api_key.clear();
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_rejects_logprobs_out_of_range() {
        let mut c = ok_cfg();
        c.logprobs_top_k = 0;
        assert!(c.validate().is_err());
        c.logprobs_top_k = 6;
        assert!(c.validate().is_err());
    }

    #[test]
    fn build_request_sets_echo_and_logprobs() {
        let s = NearAiPerplexityScorer::try_new(ok_cfg()).unwrap();
        let req = s.build_request(b"The capital of France is Paris.").unwrap();
        assert!(req.echo);
        assert_eq!(req.logprobs, 1);
        assert!(!req.stream);
        assert_eq!(req.temperature, 0.0);
        assert_eq!(req.model, "Qwen/Qwen3-30B-A3B-Instruct-2507");
    }

    #[test]
    fn build_request_rejects_non_utf8_prompt() {
        let s = NearAiPerplexityScorer::try_new(ok_cfg()).unwrap();
        // Invalid UTF-8: lone continuation byte.
        let err = s.build_request(&[0x80]).unwrap_err();
        assert!(err.to_string().contains("NotUtf8"));
    }

    /// Wire-shape parse check: the fixture matches the body Test 4 returned
    /// during the smoke validation (prompt "The capital of France is Paris.",
    /// max_tokens=0). Locks the deserializer against vLLM-side field renames.
    #[test]
    fn parses_smoke_fixture() {
        let body = r#"{
          "choices": [{
            "logprobs": {
              "token_logprobs": [
                null, -13.17, -13.44, -12.34, -16.02, -9.77, -14.01
              ]
            }
          }]
        }"#;
        let parsed: CompletionsResponse = serde_json::from_str(body).unwrap();
        let lp = parsed.choices[0].logprobs.as_ref().unwrap();
        assert_eq!(lp.token_logprobs.len(), 7);
        assert!(lp.token_logprobs[0].is_none());
        assert_eq!(lp.token_logprobs[1], Some(-13.17));
    }

    // -- retry policy -----------------------------------------------------

    /// A 502 is what the vendor actually returns when it wants us to come
    /// back; a 429 says so explicitly. Both must be retried.
    #[test]
    fn status_retry_class_retries_server_errors_and_throttling() {
        for retried in [500u16, 502, 503, 504, 429] {
            assert_eq!(
                status_retry_class(retried),
                RetryClass::Retry,
                "status {retried} must be retried"
            );
        }
    }

    /// Retrying these changes nothing: the request was rejected on its own
    /// terms, and a 402 in particular wants the queue stalled, not hammered.
    #[test]
    fn status_retry_class_gives_up_on_client_errors() {
        for fatal in [400u16, 401, 402, 403, 404, 413, 422] {
            assert_eq!(
                status_retry_class(fatal),
                RetryClass::Fatal,
                "status {fatal} must not be retried"
            );
        }
    }

    /// The retry split is not the attempt-accounting split. A 402 is
    /// transient for accounting (it must not cost the trace its budget) and
    /// still pointless to retry; a 400 is permanent for both. Locking this
    /// down so a later edit does not collapse the two questions into one.
    #[test]
    fn retry_class_is_independent_of_attempt_accounting() {
        assert!(scorer_status_is_transient(402));
        assert_eq!(status_retry_class(402), RetryClass::Fatal);
        assert!(!scorer_status_is_transient(400));
        assert_eq!(status_retry_class(400), RetryClass::Fatal);
        assert!(scorer_status_is_transient(502));
        assert_eq!(status_retry_class(502), RetryClass::Retry);
    }

    /// A timeout is the one failure where the backend may already have run
    /// the inference we are about to pay for again, on a chunk large enough
    /// to have blown the deadline once. Retrying doubles a large bill for a
    /// request that is likely to blow the same deadline. Connect/TLS/body
    /// failures produced no completion at all, so retrying them is close to
    /// free -- that asymmetry is the whole reason the split exists.
    #[test]
    fn transport_retry_class_gives_up_on_timeout_only() {
        assert_eq!(transport_retry_class(true), RetryClass::Fatal);
        assert_eq!(transport_retry_class(false), RetryClass::Retry);
    }

    #[test]
    fn score_backoff_is_exponential_and_bounded() {
        assert_eq!(score_backoff_delay(1), Duration::from_millis(250));
        assert_eq!(score_backoff_delay(2), Duration::from_millis(500));
        // Total sleep held across a fully-spent budget, which is what the
        // blocking-pool thread pays on top of the requests themselves.
        let total: Duration = (1..MAX_SCORE_ATTEMPTS).map(score_backoff_delay).sum();
        assert!(
            total <= Duration::from_millis(1_000),
            "retry budget holds a blocking thread for {total:?}"
        );
    }

    #[test]
    fn retry_loop_returns_first_success_without_sleeping() {
        let mut calls = 0usize;
        let mut slept: Vec<Duration> = Vec::new();
        let out = fetch_with_retry(
            |_| {
                calls += 1;
                Ok(7u32)
            },
            |d| slept.push(d),
        )
        .unwrap();
        assert_eq!(out, 7);
        assert_eq!(calls, 1);
        assert!(slept.is_empty());
    }

    #[test]
    fn retry_loop_recovers_from_a_transient_blip() {
        let mut calls = 0usize;
        let mut slept: Vec<Duration> = Vec::new();
        let out = fetch_with_retry(
            |attempt| {
                calls += 1;
                if attempt == 1 {
                    Err(AttemptError {
                        class: RetryClass::Retry,
                        err: anyhow!("NearAiScorerHttpStatusError status=502 body_len=0"),
                    })
                } else {
                    Ok(9u32)
                }
            },
            |d| slept.push(d),
        )
        .unwrap();
        assert_eq!(out, 9);
        assert_eq!(calls, 2);
        assert_eq!(slept, vec![Duration::from_millis(250)]);
    }

    /// The budget is spent, not exceeded, and the error the caller sees is
    /// the last attempt's -- classification included, so the scoring driver
    /// still reads it as transient and does not charge the trace.
    #[test]
    fn retry_loop_spends_the_budget_then_surfaces_the_last_error() {
        let mut calls = 0usize;
        let mut slept: Vec<Duration> = Vec::new();
        let err = fetch_with_retry(
            |attempt| {
                calls += 1;
                Err::<u32, _>(AttemptError {
                    class: RetryClass::Retry,
                    err: anyhow::Error::new(ScorerFailure::TransientScorerFailed {
                        reason: format!("attempt {attempt}"),
                    }),
                })
            },
            |d| slept.push(d),
        )
        .unwrap_err();
        assert_eq!(calls, MAX_SCORE_ATTEMPTS);
        assert_eq!(slept.len(), MAX_SCORE_ATTEMPTS - 1);
        assert!(
            err.downcast_ref::<ScorerFailure>()
                .is_some_and(|f| matches!(f, ScorerFailure::TransientScorerFailed { .. })),
            "the surfaced error must keep its transient classification"
        );
        assert!(
            err.to_string()
                .contains(&format!("attempt {MAX_SCORE_ATTEMPTS}")),
            "the last attempt's error is the one that surfaces: {err}"
        );
    }

    /// A fatal attempt short-circuits: no sleep, no second billed call.
    #[test]
    fn retry_loop_does_not_retry_a_fatal_attempt() {
        let mut calls = 0usize;
        let mut slept: Vec<Duration> = Vec::new();
        let err = fetch_with_retry(
            |_| {
                calls += 1;
                Err::<u32, _>(AttemptError {
                    class: RetryClass::Fatal,
                    err: anyhow::Error::new(ScorerFailure::ScorerFailed {
                        reason: "NearAiScorerHttpStatusError status=400 body_len=0".to_string(),
                    }),
                })
            },
            |d| slept.push(d),
        )
        .unwrap_err();
        assert_eq!(calls, 1);
        assert!(slept.is_empty());
        assert!(
            err.downcast_ref::<ScorerFailure>()
                .is_some_and(|f| matches!(f, ScorerFailure::ScorerFailed { .. }))
        );
    }

    #[test]
    fn chunk_perplexity_from_logprobs_matches_helpers() {
        // Unit-test the pure conversion (no HTTP): the same fixture logprob
        // slice must produce sum_nll / tokens / tail_tokens consistent with
        // aggregate_perplexity_metrics.
        let logprobs = vec![0.0_f32, -1.0, -2.0, -20.0, -1.0];
        let chunk = chunk_perplexity_from_logprobs(&logprobs, -8.0);
        assert_eq!(chunk.tokens, 4);
        assert!((chunk.sum_nll - 24.0).abs() < 1e-6);
        assert_eq!(chunk.tail_tokens, 1); // only -20.0 < -8.0
        assert_eq!(chunk.logprobs, vec![-1.0, -2.0, -20.0, -1.0]);
        let rebuilt = ((chunk.sum_nll / chunk.tokens as f64).exp() * 1_000_000.0) as u64;
        let reference = aggregate_perplexity_metrics(&logprobs, -8.0);
        let diff = rebuilt.abs_diff(reference.aggregate_perplexity_micros);
        // At this fixture's magnitude (perplexity ~403, ~4e8 micros), the
        // reference path's f32 mean/exp/scale arithmetic loses close to a
        // full f32 ULP (~48 at this scale) versus this function's f64 path.
        // That dwarfs the <=2 tolerance that holds for the small-magnitude
        // mock-scorer round trip in perplexity.rs (values there stay under
        // 1e7, where f32 ULP is comparably tiny). Observed drift here is 7;
        // 64 leaves headroom for the same fixture on other toolchains/targets
        // while still catching a real algorithmic divergence (which would be
        // orders of magnitude larger).
        assert!(diff <= 64, "chunk-form perplexity drifted by {diff} micros");
    }
}
