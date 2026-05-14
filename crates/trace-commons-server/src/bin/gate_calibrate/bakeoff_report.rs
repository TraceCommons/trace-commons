// Decision rule + report emission for the A2.1 perplexity-scorer bake-off.
//
// This module is intentionally committed before the bake-off runs: the winner
// must be chosen by formula rather than by inspection, and reviewers need to
// audit the rule independently of the numbers it will eventually see. Keep
// behavior strictly pure — no I/O outside `write_report_atomic`.

use std::cmp::Ordering;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Maximum acceptable stddev across determinism replays before a candidate is
/// disqualified outright. 1e-5 is tighter than any production-relevant drift
/// we have observed; bump only if a hardware change forces it and document
/// the reason alongside the decision-rule version bump.
pub const DETERMINISM_GATE: f64 = 1e-5;

/// Candidates with throughput below this fraction of the fastest in-gate
/// candidate are dropped before scoring. 0.5 is the spec's compromise between
/// "ignore tiny throughput differences" and "reject obviously unviable runs."
/// Bump only with an accompanying decision-rule-version increment.
pub const THROUGHPUT_FLOOR_RATIO: f64 = 0.5;

/// Two scores within this fractional band of the top score are considered a
/// tie and decided by license / size / recency tiebreakers. 2 % matches the
/// spec; it absorbs noise without engulfing meaningful score gaps.
pub const TIE_TOLERANCE: f64 = 0.02;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum License {
    #[serde(rename = "Apache-2.0")]
    Apache2,
    #[serde(rename = "MIT")]
    Mit,
    #[serde(rename = "llama-community")]
    LlamaCommunity,
    #[serde(rename = "gemma-custom")]
    GemmaCustom,
}

impl License {
    /// Higher = more permissive. Used as the first tiebreaker.
    pub fn permissiveness(&self) -> u8 {
        match self {
            License::Apache2 => 4,
            License::Mit => 3,
            License::GemmaCustom => 2,
            License::LlamaCommunity => 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateResult {
    pub id: String,
    pub discrimination_auc: f64,
    pub paraphrase_delta: f64,
    pub tail_fraction_range: f64,
    pub determinism_stddev: f64,
    pub throughput_tps: f64,
    pub peak_vram_mib: u64,
    pub license: License,
    pub params_b: u32,
    pub passed_determinism_gate: bool,
    /// Release date of the underlying model weights, unix seconds.
    /// Sourced from the manifest. Third tiebreaker (newer wins).
    pub release_date_unix: i64,
    /// Class name of the error that aborted load / eval, if any. Hash-only
    /// / label-only by construction — never carries raw error message text,
    /// because real candle errors can include filesystem paths or other
    /// operator-secret material. Failed candidates appear in the report with
    /// zeroed numeric fields and `passed_determinism_gate = false`, which
    /// already disqualifies them from `pick_winner`; this field exists so
    /// the operator can see *which* candidates fell over without having to
    /// re-derive it from the logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_or_eval_error: Option<String>,
    /// Per-metric scoring block, populated when the bake-off was invoked with
    /// `--scorer token-rarity` or `--scorer both`. Absent for `--scorer
    /// perplexity` (the default and back-compat path for A2.3c / A2.4 / A2.6
    /// reports). When present, each sub-field carries the per-trace scores
    /// and the AUC of that metric on this candidate; the decision rule still
    /// reads `discrimination_auc` (perplexity-derived when available) so
    /// promoting per-token rarity is a deliberate, separate change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<CandidateMetrics>,
}

/// Per-metric scoring detail for the Phase A.5 dual-scorer bake-off. Each
/// sub-block carries the discrimination AUC plus the per-trace scores that
/// AUC was computed from, so downstream tooling can re-compute / re-plot
/// without re-running the bake-off.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CandidateMetrics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perplexity: Option<MetricBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_rarity: Option<TokenRarityMetricBlock>,
}

/// Per-trace perplexity scores + AUC for a single candidate. The
/// per-trace numbers are the same `f64` micros-divided values that fed
/// `discrimination_auc`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricBlock {
    pub discrimination_auc: f64,
    pub novel_scores: Vec<f64>,
    pub duplicate_scores: Vec<f64>,
}

/// Per-trace token-rarity scores + AUC + the K the scorer used. Same shape
/// as `MetricBlock` plus the K so report consumers can spot mismatched-K
/// comparisons.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRarityMetricBlock {
    pub discrimination_auc: f64,
    pub novel_scores: Vec<f64>,
    pub duplicate_scores: Vec<f64>,
    pub k: u32,
}

impl CandidateResult {
    /// Construct a placeholder result for a candidate that failed to load
    /// or evaluate. All numeric fields zero and `passed_determinism_gate =
    /// false`, so the row never wins. `error_class` is a stable label such
    /// as `"LocalPerplexityScorerLoadFailed"` — never raw error text.
    // Reserved for the rarity-real-scorer failure path; #63 added the
    // constructor but deferred wiring it into a test target.
    #[allow(dead_code)]
    pub fn failed(
        id: String,
        license: License,
        params_b: u32,
        release_date_unix: i64,
        error_class: &str,
    ) -> Self {
        Self {
            id,
            discrimination_auc: 0.0,
            paraphrase_delta: 0.0,
            tail_fraction_range: 0.0,
            determinism_stddev: 0.0,
            throughput_tps: 0.0,
            peak_vram_mib: 0,
            license,
            params_b,
            passed_determinism_gate: false,
            release_date_unix,
            load_or_eval_error: Some(error_class.to_string()),
            metrics: None,
        }
    }
}

/// Weighted score per the spec:
///   0.6 * AUC + 0.3 * (1 - clamp(paraphrase_delta, 0, 1)) + 0.1 * (tail / tail_norm_max)
/// When `tail_norm_max` is 0 the tail term collapses to 0 rather than dividing.
pub fn weighted_score(r: &CandidateResult, tail_norm_max: f64) -> f64 {
    let para_clamped = r.paraphrase_delta.clamp(0.0, 1.0);
    let tail_term = if tail_norm_max == 0.0 {
        0.0
    } else {
        r.tail_fraction_range / tail_norm_max
    };
    0.6 * r.discrimination_auc + 0.3 * (1.0 - para_clamped) + 0.1 * tail_term
}

/// Apply the full decision rule and return the winner, if any.
///
/// 1. Drop candidates that failed the determinism gate.
/// 2. Drop candidates slower than `THROUGHPUT_FLOOR_RATIO * fastest_throughput`.
/// 3. Compute weighted scores using `max(tail_fraction_range)` over the
///    in-budget set as the normalizer.
/// 4. Anyone within `(1 - TIE_TOLERANCE)` of the top score is a contender.
/// 5. Break ties by: license permissiveness DESC, params_b ASC, release_date DESC.
pub fn pick_winner(results: &[CandidateResult]) -> Option<&CandidateResult> {
    // Step 1: determinism gate.
    let gated: Vec<&CandidateResult> = results
        .iter()
        .filter(|r| r.passed_determinism_gate)
        .collect();
    if gated.is_empty() {
        return None;
    }

    // Step 2: throughput floor.
    let fastest = gated
        .iter()
        .map(|r| r.throughput_tps)
        .fold(f64::NEG_INFINITY, f64::max);
    let floor = THROUGHPUT_FLOOR_RATIO * fastest;
    let in_budget: Vec<&CandidateResult> = gated
        .into_iter()
        .filter(|r| r.throughput_tps >= floor)
        .collect();
    if in_budget.is_empty() {
        return None;
    }

    // Step 3: normalize tail term using in-budget max.
    let tail_norm_max = in_budget
        .iter()
        .map(|r| r.tail_fraction_range)
        .fold(0.0_f64, f64::max);

    let scored: Vec<(&CandidateResult, f64)> = in_budget
        .iter()
        .map(|r| (*r, weighted_score(r, tail_norm_max)))
        .collect();

    // Step 4: contenders within tolerance band of top score.
    let top_score = scored
        .iter()
        .map(|(_, s)| *s)
        .fold(f64::NEG_INFINITY, f64::max);
    let threshold = top_score * (1.0 - TIE_TOLERANCE);
    let mut contenders: Vec<&(&CandidateResult, f64)> =
        scored.iter().filter(|(_, s)| *s >= threshold).collect();

    // Step 5: sort by license DESC, params_b ASC, release_date DESC.
    contenders.sort_by(|a, b| {
        let lp_a = a.0.license.permissiveness();
        let lp_b = b.0.license.permissiveness();
        match lp_b.cmp(&lp_a) {
            Ordering::Equal => {}
            other => return other,
        }
        match a.0.params_b.cmp(&b.0.params_b) {
            Ordering::Equal => {}
            other => return other,
        }
        b.0.release_date_unix.cmp(&a.0.release_date_unix)
    });

    contenders.first().map(|(c, _)| *c)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub generated_at: String,
    pub corpus_sha256: String,
    pub manifest_sha256: String,
    pub candidates: Vec<CandidateResult>,
    pub winner_id: Option<String>,
    pub decision_rule_version: u32,
    pub mock_scorer: bool,
    pub ctx_max_tokens: u32,
    pub determinism_gate_value: f64,
    /// Mid-run incremental snapshot marker. `true` when the report was
    /// written between candidates while the loop was still running (no
    /// `winner_id` is computed yet). The final write at the end of the loop
    /// flips this to `false`. Consumers can use it to tell "this is the
    /// authoritative report" from "this is a partial mid-run dump that may
    /// not have a winner yet."
    #[serde(default)]
    pub partial: bool,
}

/// Render the report as a markdown document. The output is intentionally
/// stable: review tooling greps for `"Winner: "` and the table header
/// `"| candidate | auc |"`. When `mock_scorer` is set, the banner is loud and
/// bracketed (no emojis — repo convention).
pub fn render_markdown(report: &Report) -> String {
    let mut out = String::new();
    if report.mock_scorer {
        out.push_str("> [MOCK SCORER - NOT VALID FOR PRODUCTION DECISIONS]\n\n");
    }
    out.push_str(&format!("# Bake-off report ({})\n\n", report.generated_at));
    out.push_str(&format!("- corpus: {}\n", report.corpus_sha256));
    out.push_str(&format!("- manifest: {}\n", report.manifest_sha256));
    out.push_str(&format!(
        "- decision-rule version: {}\n",
        report.decision_rule_version
    ));
    out.push_str(&format!("- ctx_max_tokens: {}\n", report.ctx_max_tokens));
    out.push_str(&format!(
        "- determinism gate: {}\n\n",
        report.determinism_gate_value
    ));

    let winner = report.winner_id.as_deref().unwrap_or("none");
    out.push_str(&format!("Winner: {}\n\n", winner));

    out.push_str("| candidate | auc | paraphrase_delta | tail_range | throughput_tps | determinism_stddev | license | params_b | passed_gate |\n");
    out.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for c in &report.candidates {
        out.push_str(&format!(
            "| {} | {:.6} | {:.6} | {:.6} | {:.3} | {:.3e} | {:?} | {} | {} |\n",
            c.id,
            c.discrimination_auc,
            c.paraphrase_delta,
            c.tail_fraction_range,
            c.throughput_tps,
            c.determinism_stddev,
            c.license,
            c.params_b,
            c.passed_determinism_gate,
        ));
    }

    // Phase A.5: emit a per-token-rarity summary table when any candidate
    // carries a `metrics.token_rarity` block. The legacy table above stays
    // perplexity-only so existing review tooling (which greps for the
    // "| candidate | auc |" header) is unaffected.
    let any_rarity = report.candidates.iter().any(|c| {
        c.metrics
            .as_ref()
            .and_then(|m| m.token_rarity.as_ref())
            .is_some()
    });
    if any_rarity {
        out.push_str("\n## Per-token rarity (Phase A.5)\n\n");
        out.push_str("| candidate | token_rarity_auc | k |\n");
        out.push_str("| --- | --- | --- |\n");
        for c in &report.candidates {
            let Some(rarity) = c.metrics.as_ref().and_then(|m| m.token_rarity.as_ref()) else {
                continue;
            };
            out.push_str(&format!(
                "| {} | {:.6} | {} |\n",
                c.id, rarity.discrimination_auc, rarity.k,
            ));
        }
    }

    let failed: Vec<&CandidateResult> = report
        .candidates
        .iter()
        .filter(|c| c.load_or_eval_error.is_some())
        .collect();
    if !failed.is_empty() {
        out.push_str("\n## Failed candidates\n\n");
        out.push_str("| candidate | error_class |\n");
        out.push_str("| --- | --- |\n");
        for c in failed {
            // load_or_eval_error is Some by construction (filter above);
            // unwrap_or for paranoia.
            let cls = c.load_or_eval_error.as_deref().unwrap_or("Unknown");
            out.push_str(&format!("| {} | {} |\n", c.id, cls));
        }
    }
    out
}

/// Write the report as JSON to `json_out`. Best-effort writes the markdown
/// alongside (same stem, `.md` suffix) when the path has a `.json` extension;
/// failures to write the companion file are propagated so partial state is
/// visible. SHA companion is intentionally deferred — not on the critical
/// path for this slice.
/// Atomic-rename report writer used by the bake-off loop. Writes JSON to
/// `<dest>.tmp`, fsyncs, then renames over `dest`, so a process kill mid-
/// write cannot leave a half-written `report.json` on disk. The markdown
/// companion is written non-atomically (best-effort) because consumers grep
/// the JSON file for "is the run still going" — the markdown is for humans
/// and a transient half-write is acceptable.
// Used by the bake-off binary loop; the `bakeoff_report` integration test
// target imports the module via `#[path = ...]` and exercises the pure
// scoring functions, not the on-disk writer.
#[allow(dead_code)]
pub fn write_report_atomic(report: &Report, dest: &Path) -> anyhow::Result<()> {
    use std::io::Write;
    let tmp = dest.with_extension("json.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        let json = serde_json::to_vec_pretty(report)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, dest)?;
    if dest.extension().and_then(|s| s.to_str()) == Some("json") {
        let md_path = dest.with_extension("md");
        let md = render_markdown(report);
        // Best-effort: a markdown write failure mid-loop should not abort
        // the next candidate. Surface as a warn.
        if let Err(e) = std::fs::write(&md_path, md) {
            tracing::warn!(
                error_class = "BakeoffMarkdownWriteFailed",
                err = %e,
                "incremental markdown write failed; continuing"
            );
        }
    }
    Ok(())
}
