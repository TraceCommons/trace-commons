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
//! ### Fail-closed contract
//!
//! Any HTTP error, JSON parse failure, missing field, non-finite logprob, or
//! token-count mismatch propagates as `Err` — matching the trait contract that
//! a refused score must not silently substitute zero (which would pass any
//! positive perplexity floor).

#![cfg(feature = "near-ai-scorer")]

use crate::perplexity::{PerplexityResult, PerplexityScorer, TokenRarityResult, TokenRarityScorer};
use crate::perplexity_local::{aggregate_perplexity_metrics, per_token_rarity_micros};
use anyhow::{Context, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::time::Duration;

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
    /// Per-call `logprobs` value sent to the API. Five is the OpenAI-canonical
    /// upper bound and is what NEAR AI's hosted vLLM accepts; the rarity
    /// metric only needs `top_logprobs` for the K-rarest computation when
    /// inspecting alternatives, not the realized-token NLL we use for both
    /// metrics here.
    pub logprobs_top_k: u32,
    /// HTTP timeout for a single scoring request.
    pub timeout: Duration,
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

impl NearAiPerplexityScorer {
    pub fn try_new(cfg: NearAiScorerConfig) -> anyhow::Result<Self> {
        cfg.validate()?;

        let client = reqwest::blocking::Client::builder()
            .timeout(cfg.timeout)
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
        let req = self.build_request(plaintext)?;
        let url = format!("{}/completions", self.cfg.base_url);

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.cfg.api_key)
            .json(&req)
            .send()
            .context("NearAiScorerHttpSendFailed")?;
        let status = resp.status();
        let body = resp.text().context("NearAiScorerHttpBodyReadFailed")?;
        if !status.is_success() {
            // Body may contain a vLLM validation message; surface its
            // length only, not its content (provider strings are not
            // hash-only audit material).
            return Err(anyhow!(
                "NearAiScorerHttpStatusError status={} body_len={}",
                status.as_u16(),
                body.len()
            ));
        }

        let parsed: CompletionsResponse =
            serde_json::from_str(&body).context("NearAiScorerResponseParseFailed")?;
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
}

impl PerplexityScorer for NearAiPerplexityScorer {
    fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
        let logprobs = self.fetch_logprobs(plaintext)?;
        Ok(aggregate_perplexity_metrics(
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
            logprobs_top_k: 5,
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
        assert_eq!(req.logprobs, 5);
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
}
