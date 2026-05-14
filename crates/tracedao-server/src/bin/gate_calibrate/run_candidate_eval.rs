//! Per-candidate evaluation for the A2.1 perplexity-scorer bake-off.
//!
//! Takes a loaded corpus and a single candidate, runs it through a
//! `PerplexityScorer` impl, and returns the `CandidateResult` row that
//! feeds the decision rule. The function is async because real scorers
//! may want to interleave I/O during their warmup; the mock scorer is
//! synchronous and returns immediately.
//!
//! Failure handling: per-entry scoring errors are tolerated up to a 5%
//! rate per candidate. Above that, we abort the candidate outright —
//! a partial result with > 5% missing samples is not trustworthy enough
//! to feed into the decision rule.
//!
//! All operational logging is label-only: candidate id (operator-set,
//! safe), entry indices (numbers), and hashed error fingerprints. Trace
//! plaintexts never log.

use std::time::Instant;

use tracedao_gate_enclave::perplexity::{
    PerplexityResult, PerplexityScorer, TokenRarityResult, TokenRarityScorer,
};

use super::bakeoff_corpus::LoadedCorpus;
use super::bakeoff_manifest::{Candidate, CandidateArch, CandidateLicense};
use super::bakeoff_metrics::{
    determinism_stddev, discrimination_auc, paraphrase_delta, tail_fraction_range,
};
use super::bakeoff_report::{
    CandidateMetrics, CandidateResult, DETERMINISM_GATE, License, MetricBlock,
    TokenRarityMetricBlock,
};

/// Scorer selection for a bake-off pass. The bake-off binary's `--scorer`
/// flag maps to one of these; the eval loop reads `perplexity` /
/// `token_rarity` `Option`s to decide which metric columns to populate.
///
/// At least one scorer must be present; the eval-loop entry function
/// asserts that as a fail-closed guard.
pub struct EvalScorers<'a> {
    pub perplexity: Option<&'a dyn PerplexityScorer>,
    pub token_rarity: Option<&'a dyn TokenRarityScorer>,
    /// K parameter passed to every `score_rarity` call. Ignored when
    /// `token_rarity` is `None`.
    pub token_rarity_k: usize,
}

impl<'a> EvalScorers<'a> {
    /// Back-compat helper: perplexity-only selection. Matches the pre-A.5
    /// behavior so existing call sites keep working.
    ///
    /// Used by the real-scorer path in the bake-off binary and by the
    /// integration test suite; the mock-scorer path in the binary builds
    /// `EvalScorers` directly so it can mix in token-rarity. The
    /// integration-test usage is in a separate compilation unit (via
    /// `#[path = ...]`), so `dead_code` analysis on the binary doesn't see
    /// those callers — hence the allow.
    #[allow(dead_code)]
    pub fn perplexity_only(s: &'a dyn PerplexityScorer) -> Self {
        Self {
            perplexity: Some(s),
            token_rarity: None,
            token_rarity_k: 0,
        }
    }

    fn assert_at_least_one(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.perplexity.is_some() || self.token_rarity.is_some(),
            "EvalScorersEmpty: at least one scorer (perplexity or token_rarity) is required"
        );
        if self.token_rarity.is_some() {
            anyhow::ensure!(
                self.token_rarity_k > 0,
                "EvalScorersBadK: token_rarity_k must be greater than zero"
            );
        }
        Ok(())
    }
}

/// Conversion of `u64` micros (scorer output) to floating-point. Centralized
/// so the metric-feeding code reads as one boundary cross rather than a
/// scatter of inline `as f64 / 1_000_000.0` divisions.
fn micros_to_f64(v: u64) -> f64 {
    v as f64 / 1_000_000.0
}

/// Hardware-class hint passed from the CLI. CUDA hosts get their peak VRAM
/// measured via `nvidia-smi`; everything else reports zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    #[allow(dead_code)]
    // constructed by `From<HardwareTier>` in the gate-calibrate binary; test target only constructs `NonCuda`
    Cuda,
    NonCuda,
}

/// Per-arch context window for the bake-off. Spec: all 4096 for predictable
/// VRAM and comparable throughput across architectures. Bump only if a
/// candidate requires it and document the reason with a decision-rule
/// version bump.
pub fn ctx_for(arch: &CandidateArch) -> usize {
    match arch {
        CandidateArch::Llama
        | CandidateArch::Qwen3
        | CandidateArch::Qwen3_5
        | CandidateArch::Qwen2
        | CandidateArch::Gemma3
        | CandidateArch::Gemma4 => 4096,
    }
}

/// Peak VRAM in MiB. CUDA: shell out to `nvidia-smi`. Non-CUDA: 0.
///
/// Best-effort: a parse failure or missing binary returns 0 and emits a
/// `tracing::warn`. We don't propagate the error because a missing VRAM
/// figure is not a reason to fail the whole eval.
pub fn peak_vram_mib(device: DeviceKind) -> anyhow::Result<u64> {
    if matches!(device, DeviceKind::NonCuda) {
        return Ok(0);
    }
    let out = match std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used", "--format=csv,noheader,nounits"])
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            tracing::warn!(
                error_class = "PeakVramQueryFailed",
                "nvidia-smi unavailable; reporting zero VRAM"
            );
            return Ok(0);
        }
    };
    if !out.status.success() {
        tracing::warn!(
            error_class = "PeakVramQueryFailed",
            "nvidia-smi returned non-zero; reporting zero VRAM"
        );
        return Ok(0);
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    let first = raw.lines().next().unwrap_or("").trim();
    match first.parse::<u64>() {
        Ok(v) => Ok(v),
        Err(_) => {
            tracing::warn!(
                error_class = "PeakVramParseFailed",
                "nvidia-smi output did not parse; reporting zero VRAM"
            );
            Ok(0)
        }
    }
}

/// Map manifest license enum to report license enum. Distinct types because
/// the manifest is `serde(rename_all="...")` deserialize-only and the report
/// is the public JSON-stable shape.
pub fn map_license(lic: &CandidateLicense) -> License {
    match lic {
        CandidateLicense::Apache2 => License::Apache2,
        CandidateLicense::Mit => License::Mit,
        CandidateLicense::LlamaCommunity => License::LlamaCommunity,
        CandidateLicense::GemmaCustom => License::GemmaCustom,
    }
}

/// Maximum fraction of per-entry scoring failures we tolerate before aborting
/// the candidate. Above this, the result wouldn't be trustworthy enough to
/// rank against peers.
const FAILURE_RATE_ABORT: f64 = 0.05;

/// Determinism replay sample size. The minimum of this and the available
/// novel slice; small enough that 3-5 replays don't dominate wallclock.
const DETERMINISM_SAMPLE_SIZE: usize = 16;

/// Inferred params count from a HuggingFace-style `config.json` at
/// `<candidate.path>/config.json`. Approximate — `12 * hidden_size *
/// num_hidden_layers` is the standard textbook estimate for a Llama-family
/// transformer (it ignores embedding and lm_head parameters but the figure
/// is only used as a tiebreaker, so a few percent off doesn't matter).
fn infer_params_b(candidate_path: &std::path::Path) -> Option<u32> {
    let cfg_path = candidate_path.join("config.json");
    let raw = std::fs::read(&cfg_path).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let hidden = v.get("hidden_size")?.as_u64()?;
    let layers = v.get("num_hidden_layers")?.as_u64()?;
    let params = 12u64.saturating_mul(hidden).saturating_mul(layers);
    let billions = (params as f64 / 1e9).round() as u32;
    Some(billions.max(1))
}

/// Score one entry and convert micros to (perplexity_f64, tail_f64,
/// tokens_scored). Returns `Err` on scorer failure so the caller can count
/// it toward the failure-rate budget.
fn score_one(scorer: &dyn PerplexityScorer, text: &str) -> anyhow::Result<(f64, f64, u64)> {
    let r: PerplexityResult = scorer.score(text.as_bytes())?;
    Ok((
        micros_to_f64(r.aggregate_perplexity_micros),
        micros_to_f64(r.tail_fraction_micros),
        r.tokens_scored,
    ))
}

/// Score one entry for per-token rarity and convert micros to
/// (rarity_f64, tokens_scored). Returns `Err` on scorer failure.
fn score_one_rarity(
    scorer: &dyn TokenRarityScorer,
    text: &str,
    k: usize,
) -> anyhow::Result<(f64, u64)> {
    let r: TokenRarityResult = scorer.score_rarity(text.as_bytes(), k)?;
    Ok((micros_to_f64(r.token_rarity_micros), r.tokens_scored))
}

/// Run a single candidate over the full corpus and produce its
/// `CandidateResult` row.
///
/// Algorithm:
///
/// 1. Score every entry across novel / duplicate / paraphrase. Collect
///    `(perplexity, tail, tokens_scored)` per entry. Wall-clock the loop.
/// 2. Compute AUC (novel vs duplicate), paraphrase delta, tail-fraction
///    range from the collected `f64` vectors.
/// 3. Replay the first `min(16, novel.len())` novel entries
///    `repeat_runs` times to compute determinism stddev. Set
///    `passed_determinism_gate = stddev < DETERMINISM_GATE`.
/// 4. Sample peak VRAM via `nvidia-smi` on CUDA hosts; zero elsewhere.
///
/// Failures: per-entry scorer errors are counted; if the cumulative rate
/// exceeds 5 %, the candidate is aborted with an error so the caller can
/// decide whether to surface it or move on.
pub async fn run_candidate_eval(
    scorers: EvalScorers<'_>,
    candidate: &Candidate,
    corpus: &LoadedCorpus,
    repeat_runs: u32,
    device_kind: DeviceKind,
) -> anyhow::Result<CandidateResult> {
    scorers.assert_at_least_one()?;
    anyhow::ensure!(
        repeat_runs >= 2,
        "RunCandidateEval: repeat_runs must be at least 2 to compute a stddev"
    );

    tracing::info!(
        candidate_id = %candidate.id,
        ctx_max_tokens = ctx_for(&candidate.arch),
        novel_count = corpus.novel.len(),
        duplicate_count = corpus.duplicate.len(),
        paraphrase_count = corpus.paraphrase.len(),
        "candidate_eval_start"
    );

    // ---- 1. Score every slice. The eval loop interleaves scorer calls so
    // both metrics share the same loop overhead. Per-trace failures from
    // either scorer count toward one combined failure budget; the slice is
    // dropped from BOTH metrics if either scorer errors on it, which keeps
    // novel/duplicate vectors aligned across columns.
    //
    // `total_tokens` is sourced from whichever scorer ran first (perplexity
    // when both are active, else rarity). Token counts from the two scorers
    // are expected to match for any real implementation since they tokenize
    // the same input — but only one is added to the budget to avoid double-
    // counting work that produced two metrics from one forward pass.
    let mut novel_perp: Vec<f64> = Vec::with_capacity(corpus.novel.len());
    let mut dup_perp: Vec<f64> = Vec::with_capacity(corpus.duplicate.len());
    let mut novel_tail: Vec<f64> = Vec::with_capacity(corpus.novel.len());
    let mut dup_tail: Vec<f64> = Vec::with_capacity(corpus.duplicate.len());
    let mut para_pairs: Vec<(f64, f64)> = Vec::with_capacity(corpus.paraphrase.len());

    let mut novel_rarity: Vec<f64> = Vec::with_capacity(corpus.novel.len());
    let mut dup_rarity: Vec<f64> = Vec::with_capacity(corpus.duplicate.len());

    let mut total_tokens: u64 = 0;
    let mut attempts: u64 = 0;
    let mut failures: u64 = 0;

    let start = Instant::now();

    // Per-entry scorer dispatch. Returns Ok((perplexity_opt, rarity_opt,
    // tokens_scored)) on success; Err on any active-scorer failure.
    let score_entry = |text: &str| -> anyhow::Result<(Option<(f64, f64)>, Option<f64>, u64)> {
        let mut tokens = 0u64;
        let perp = if let Some(p) = scorers.perplexity {
            let (a, t, tok) = score_one(p, text)?;
            tokens = tok;
            Some((a, t))
        } else {
            None
        };
        let rarity = if let Some(r) = scorers.token_rarity {
            let (a, tok) = score_one_rarity(r, text, scorers.token_rarity_k)?;
            if perp.is_none() {
                // Rarity-only mode: throughput accounting reads from the
                // rarity scorer.
                tokens = tok;
            }
            Some(a)
        } else {
            None
        };
        Ok((perp, rarity, tokens))
    };

    let novel_slice_start = Instant::now();
    let novel_scored_before = novel_perp.len().max(novel_rarity.len());
    let novel_failures_before = failures;
    for (i, text) in corpus.novel.iter().enumerate() {
        attempts += 1;
        match score_entry(text) {
            Ok((perp, rarity, tok)) => {
                if let Some((a, t)) = perp {
                    novel_perp.push(a);
                    novel_tail.push(t);
                }
                if let Some(a) = rarity {
                    novel_rarity.push(a);
                }
                total_tokens = total_tokens.saturating_add(tok);
            }
            Err(e) => {
                failures += 1;
                tracing::warn!(
                    candidate_id = %candidate.id,
                    slice = "novel",
                    entry_index = i,
                    err = %hash_err(&e),
                    "score_failed"
                );
            }
        }
    }
    tracing::info!(
        candidate_id = %candidate.id,
        slice = "novel",
        scored = novel_perp.len().max(novel_rarity.len()) - novel_scored_before,
        failed = failures - novel_failures_before,
        elapsed_seconds = novel_slice_start.elapsed().as_secs_f64(),
        "candidate_slice_done"
    );

    let duplicate_slice_start = Instant::now();
    let dup_scored_before = dup_perp.len().max(dup_rarity.len());
    let dup_failures_before = failures;
    for (i, text) in corpus.duplicate.iter().enumerate() {
        attempts += 1;
        match score_entry(text) {
            Ok((perp, rarity, tok)) => {
                if let Some((a, t)) = perp {
                    dup_perp.push(a);
                    dup_tail.push(t);
                }
                if let Some(a) = rarity {
                    dup_rarity.push(a);
                }
                total_tokens = total_tokens.saturating_add(tok);
            }
            Err(e) => {
                failures += 1;
                tracing::warn!(
                    candidate_id = %candidate.id,
                    slice = "duplicate",
                    entry_index = i,
                    err = %hash_err(&e),
                    "score_failed"
                );
            }
        }
    }
    tracing::info!(
        candidate_id = %candidate.id,
        slice = "duplicate",
        scored = dup_perp.len().max(dup_rarity.len()) - dup_scored_before,
        failed = failures - dup_failures_before,
        elapsed_seconds = duplicate_slice_start.elapsed().as_secs_f64(),
        "candidate_slice_done"
    );

    // Paraphrase pairs are only used by the legacy paraphrase-delta metric,
    // which is perplexity-derived. Skip the slice entirely when the
    // perplexity scorer is absent (rarity-only mode) — paraphrase_delta
    // collapses to 0.0 for that path, mirroring its empty-input contract.
    let paraphrase_slice_start = Instant::now();
    let para_scored_before = para_pairs.len();
    let para_failures_before = failures;
    if let Some(scorer) = scorers.perplexity {
        for (i, pair) in corpus.paraphrase.iter().enumerate() {
            attempts += 2;
            let orig = score_one(scorer, &pair.original);
            let para = score_one(scorer, &pair.paraphrase);
            match (orig, para) {
                (Ok((op, _, otok)), Ok((pp, _, ptok))) => {
                    total_tokens = total_tokens.saturating_add(otok);
                    total_tokens = total_tokens.saturating_add(ptok);
                    para_pairs.push((op, pp));
                }
                (orig_res, para_res) => {
                    if let Err(e) = &orig_res {
                        failures += 1;
                        tracing::warn!(
                            candidate_id = %candidate.id,
                            slice = "paraphrase_original",
                            entry_index = i,
                            err = %hash_err(e),
                            "score_failed"
                        );
                    }
                    if let Err(e) = &para_res {
                        failures += 1;
                        tracing::warn!(
                            candidate_id = %candidate.id,
                            slice = "paraphrase",
                            entry_index = i,
                            err = %hash_err(e),
                            "score_failed"
                        );
                    }
                    // Skip the pair entirely; partial pairs aren't meaningful
                    // for the paraphrase-delta metric.
                }
            }
        }
    }

    tracing::info!(
        candidate_id = %candidate.id,
        slice = "paraphrase",
        scored = para_pairs.len() - para_scored_before,
        failed = failures - para_failures_before,
        elapsed_seconds = paraphrase_slice_start.elapsed().as_secs_f64(),
        "candidate_slice_done"
    );

    let elapsed = start.elapsed();
    let elapsed_seconds = elapsed.as_secs_f64().max(1e-9); // avoid /0 in synthetic tests
    let tokens_per_second = total_tokens as f64 / elapsed_seconds;

    if attempts > 0 {
        let failure_rate = failures as f64 / attempts as f64;
        if failure_rate > FAILURE_RATE_ABORT {
            anyhow::bail!(
                "RunCandidateEval: candidate {} aborted; failure rate {:.3} exceeds {:.3} threshold",
                candidate.id,
                failure_rate,
                FAILURE_RATE_ABORT
            );
        }
    }

    // ---- 2. Compute primary metrics. ------------------------------------
    // Perplexity-derived metrics populate the legacy `CandidateResult`
    // fields. When perplexity is absent (rarity-only mode), AUC is 0.5 (the
    // empty-input convention) and tail / paraphrase collapse to 0. The
    // decision rule already requires `passed_determinism_gate = true` to
    // pick a winner, so a rarity-only run cannot accidentally elect one
    // through these zeroed fields.
    let auc = discrimination_auc(&novel_perp, &dup_perp);
    let para_delta = paraphrase_delta(&para_pairs);
    let tail_range = tail_fraction_range(&novel_tail, &dup_tail);

    let rarity_auc = if scorers.token_rarity.is_some() {
        Some(discrimination_auc(&novel_rarity, &dup_rarity))
    } else {
        None
    };

    // Build the per-metric report sub-block when at least one scorer's
    // metrics need to surface beyond the legacy fields. Perplexity-only
    // runs skip this block entirely so existing report consumers see the
    // same JSON shape they saw before A.5.
    let metrics_block = if scorers.token_rarity.is_some() {
        let perplexity_block = scorers.perplexity.map(|_| MetricBlock {
            discrimination_auc: auc,
            novel_scores: novel_perp.clone(),
            duplicate_scores: dup_perp.clone(),
        });
        let rarity_block = TokenRarityMetricBlock {
            discrimination_auc: rarity_auc.unwrap_or(0.5),
            novel_scores: novel_rarity.clone(),
            duplicate_scores: dup_rarity.clone(),
            k: scorers.token_rarity_k.min(u32::MAX as usize) as u32,
        };
        Some(CandidateMetrics {
            perplexity: perplexity_block,
            token_rarity: Some(rarity_block),
        })
    } else {
        None
    };

    // ---- 3. Determinism replay. -----------------------------------------
    // Use perplexity when available (the production-decision metric); fall
    // back to rarity only in rarity-only mode. Mixing both in one stddev
    // would conflate two metric scales; the bake-off decision rule reads
    // exactly one column so we replay exactly one.
    let sample_n = DETERMINISM_SAMPLE_SIZE.min(corpus.novel.len());
    tracing::info!(
        candidate_id = %candidate.id,
        entries = sample_n,
        repeat_runs,
        det_scorer = if scorers.perplexity.is_some() {
            "perplexity"
        } else {
            "token_rarity"
        },
        "determinism_replay_start"
    );
    let mut runs: Vec<Vec<f64>> = Vec::with_capacity(repeat_runs as usize);
    for _ in 0..repeat_runs {
        let mut row: Vec<f64> = Vec::with_capacity(sample_n);
        for text in corpus.novel.iter().take(sample_n) {
            let sample: Result<f64, ()> = if let Some(p) = scorers.perplexity {
                score_one(p, text).map(|(a, _, _)| a).map_err(|_| ())
            } else if let Some(r) = scorers.token_rarity {
                score_one_rarity(r, text, scorers.token_rarity_k)
                    .map(|(a, _)| a)
                    .map_err(|_| ())
            } else {
                // assert_at_least_one above forbids this branch.
                unreachable!("EvalScorers empty after assert_at_least_one passed");
            };
            match sample {
                Ok(v) => row.push(v),
                Err(()) => {
                    // Determinism samples that fail leave an NaN-equivalent
                    // hole; we substitute zero so the population stddev
                    // computation doesn't trip over missing entries. A
                    // candidate that fails determinism replays will also
                    // have been flagged by the main failure-rate gate.
                    row.push(0.0);
                }
            }
        }
        runs.push(row);
    }
    let det_stddev = determinism_stddev(&runs);
    let passed_det_gate = det_stddev < DETERMINISM_GATE;
    tracing::info!(
        candidate_id = %candidate.id,
        stddev = det_stddev,
        passed = passed_det_gate,
        "determinism_replay_done"
    );

    // ---- 4. VRAM. -------------------------------------------------------
    let peak_vram = peak_vram_mib(device_kind)?;

    // ---- 5. params_b resolution. ----------------------------------------
    let params_b = match candidate.params_b {
        Some(n) => n,
        None => match infer_params_b(&candidate.path) {
            Some(n) => n,
            None => {
                tracing::warn!(
                    candidate_id = %candidate.id,
                    "params_b_unknown: manifest omitted params_b and config.json inference failed; defaulting to 0"
                );
                0
            }
        },
    };

    // ---- 6. Release date. -----------------------------------------------
    let release_date_unix = match candidate.release_date_unix {
        Some(t) => t,
        None => {
            tracing::warn!(
                candidate_id = %candidate.id,
                "release_date_unknown: manifest omitted release_date_unix; defaulting to 0"
            );
            0
        }
    };

    tracing::info!(
        candidate_id = %candidate.id,
        auc, para_delta, tail_range, det_stddev, tokens_per_second, peak_vram,
        passed_det_gate,
        token_rarity_auc = ?rarity_auc,
        "run_candidate_eval complete"
    );

    Ok(CandidateResult {
        id: candidate.id.clone(),
        discrimination_auc: auc,
        paraphrase_delta: para_delta,
        tail_fraction_range: tail_range,
        determinism_stddev: det_stddev,
        throughput_tps: tokens_per_second,
        peak_vram_mib: peak_vram,
        license: map_license(&candidate.license),
        params_b,
        passed_determinism_gate: passed_det_gate,
        release_date_unix,
        load_or_eval_error: None,
        metrics: metrics_block,
    })
}

/// Stable hash of an `anyhow::Error` so warn lines don't carry raw error
/// text that might include operator-secret material on a real scorer.
fn hash_err(e: &anyhow::Error) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(format!("{e}").as_bytes());
    format!("sha256:{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_vram_non_cuda_returns_zero() {
        assert_eq!(peak_vram_mib(DeviceKind::NonCuda).unwrap(), 0);
    }

    #[test]
    fn ctx_for_returns_4096_for_all_arches() {
        assert_eq!(ctx_for(&CandidateArch::Llama), 4096);
        assert_eq!(ctx_for(&CandidateArch::Qwen3), 4096);
        assert_eq!(ctx_for(&CandidateArch::Qwen3_5), 4096);
        assert_eq!(ctx_for(&CandidateArch::Qwen2), 4096);
        assert_eq!(ctx_for(&CandidateArch::Gemma3), 4096);
        assert_eq!(ctx_for(&CandidateArch::Gemma4), 4096);
    }

    #[test]
    fn infer_params_b_returns_none_when_path_missing() {
        let p = std::path::Path::new("/nonexistent/definitely-not-there");
        assert!(infer_params_b(p).is_none());
    }

    #[test]
    fn map_license_round_trips_each_variant() {
        assert!(matches!(
            map_license(&CandidateLicense::Apache2),
            License::Apache2
        ));
        assert!(matches!(map_license(&CandidateLicense::Mit), License::Mit));
        assert!(matches!(
            map_license(&CandidateLicense::LlamaCommunity),
            License::LlamaCommunity
        ));
        assert!(matches!(
            map_license(&CandidateLicense::GemmaCustom),
            License::GemmaCustom
        ));
    }
}
