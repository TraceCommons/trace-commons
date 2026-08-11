// Decision rule + report emission for the A2.1 perplexity-scorer bake-off.
//
// This module is intentionally committed before the bake-off runs: the winner
// must be chosen by formula rather than by inspection, and reviewers need to
// audit the rule independently of the numbers it will eventually see. Keep
// behavior strictly pure — no I/O outside `write_report_atomic`.

use std::cmp::Ordering;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::bakeoff_metrics::discrimination_auc;

/// Maximum acceptable stddev across determinism replays before a candidate is
/// disqualified outright. 1e-5 is tighter than any production-relevant drift
/// we have observed; bump only if a hardware change forces it and document
/// the reason alongside the decision-rule version bump.
pub const DETERMINISM_GATE: f64 = 1e-5;

/// Minimum discrimination AUC a candidate must clear to be eligible at all.
///
/// AUC is the probability the metric ranks a novel trace above a duplicate
/// one. 0.5 is chance. At or below it the candidate carries no usable signal
/// — at 0.34 it is reliably *anti*-correlated — and no amount of throughput
/// makes an unusable metric usable, which is why this gate runs before the
/// throughput floor rather than being folded into the weighted score.
///
/// `weighted_score` still contributes `0.6 * auc`, so without this gate a
/// sub-chance candidate keeps a positive score and can out-rank a
/// discriminating but slower one. Bump only with a decision-rule-version
/// increment.
pub const DISCRIMINATION_FLOOR: f64 = 0.5;

/// Minimum AUC improvement over the strongest preregistered no-model
/// baseline required for a candidate to remain eligible.
///
/// This is the repository's existing materiality threshold for replacing
/// one scorer with another: `docs/operator/a5a-rarity-preflight.md:130-148`
/// requires `(rarity AUC - A2.6 perplexity AUC) >= 0.05`.
#[allow(dead_code)] // consumed by baseline computation in the gate-calibrate binary; test targets re-import modules independently
pub const BASELINE_DOMINANCE_MARGIN: f64 = 0.05;

/// Maximum distance between independently computed candidate and baseline AUCs
/// at an inclusive boundary. Four adjacent representable values cover the
/// observed division/addition rounding without scaling epsilon by both operands
/// or adding the entire scaled tolerance to one side. The committed 300x300
/// corpus has an AUC half-step near 5.6e-6, far above this allowance; arbitrary
/// direct callers are the practical exposure this bound controls.
const BASELINE_COMPARISON_ULPS: u64 = 4;

fn ordered_f64_bits(value: f64) -> u64 {
    const SIGN_BIT: u64 = 1 << 63;
    let bits = value.to_bits();
    if bits & SIGN_BIT == 0 {
        bits | SIGN_BIT
    } else {
        !bits
    }
}

/// Version of the decision rule below. Stamped onto every report so a
/// recorded winner can be traced to the rule that chose it.
///
/// v3 adds the baseline-dominance floor after [`DISCRIMINATION_FLOOR`] and
/// ahead of throughput. A candidate must beat the strongest preregistered
/// no-model structural baseline by [`BASELINE_DOMINANCE_MARGIN`].
///
/// v2 added [`DISCRIMINATION_FLOOR`] ahead of the throughput floor. Under v1
/// the A2.6 run recorded `llama-3.1-8b-instruct` (AUC 0.3425) as winner while
/// `qwen3.6-27b-dense` (AUC 0.9363) was dropped for running at 119.49 tps
/// against a floor of 145.88; the operator applied the runbook's
/// worst-of-passing-AUC rule by hand, so production was unaffected.
#[allow(dead_code)] // stamped by the gate-calibrate binary; test target re-imports module for other unit tests
pub const DECISION_RULE_VERSION: u32 = 3;

/// Candidates with throughput below this fraction of the fastest in-gate
/// candidate are dropped before scoring. 0.5 is the spec's compromise between
/// "ignore tiny throughput differences" and "reject obviously unviable runs."
/// Bump only with an accompanying decision-rule-version increment.
#[allow(dead_code)] // consumed by `pick_winner` in the gate-calibrate binary; test target re-imports module for other unit tests
pub const THROUGHPUT_FLOOR_RATIO: f64 = 0.5;

/// Two scores within this fractional band of the top score are considered a
/// tie and decided by license / size / recency tiebreakers. 2 % matches the
/// spec; it absorbs noise without engulfing meaningful score gaps.
#[allow(dead_code)] // consumed by `pick_winner` in the gate-calibrate binary; test target re-imports module for other unit tests
pub const TIE_TOLERANCE: f64 = 0.02;

/// A preregistered structural measure evaluated on the same novel and
/// duplicate slices as every model candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoModelBaseline {
    pub name: String,
    pub discrimination_auc: f64,
}

/// Corpus-level no-model baseline evidence used by decision-rule v3.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BaselineResults {
    pub measures: Vec<NoModelBaseline>,
    pub strongest_name: Option<String>,
    pub strongest_auc: f64,
    pub required_discrimination_auc: f64,
}

impl BaselineResults {
    /// Compute the four structural measures fixed before model evaluation.
    ///
    /// Keep this list closed: adding distinct-word count, mean word length,
    /// or any other measure after observing its result would turn the
    /// baseline control into the same post-hoc selection error it prevents.
    #[allow(dead_code)] // called by the gate-calibrate binary; integration tests also exercise the committed corpus
    pub fn from_corpus(novel: &[String], duplicate: &[String]) -> Self {
        let measures = [
            (
                "utf8_byte_count",
                structural_auc(novel, duplicate, |s| s.len()),
            ),
            (
                "whitespace_word_count",
                structural_auc(novel, duplicate, |s| s.split_whitespace().count()),
            ),
            (
                "line_count",
                structural_auc(novel, duplicate, |s| s.lines().count()),
            ),
            (
                "paragraph_count",
                structural_auc(novel, duplicate, paragraph_count),
            ),
        ]
        .into_iter()
        .map(|(name, discrimination_auc)| NoModelBaseline {
            name: name.to_string(),
            discrimination_auc,
        })
        .collect::<Vec<_>>();

        let strongest = measures.iter().max_by(|a, b| {
            a.discrimination_auc
                .partial_cmp(&b.discrimination_auc)
                .unwrap_or(Ordering::Equal)
        });
        let strongest_auc = strongest.map_or(0.0, |baseline| baseline.discrimination_auc);
        Self {
            strongest_name: strongest.map(|baseline| baseline.name.clone()),
            strongest_auc,
            // Deliberately not clamped to 1.0. A floor above 1.0 records that
            // the corpus offers no remaining discrimination for a model.
            required_discrimination_auc: strongest_auc + BASELINE_DOMINANCE_MARGIN,
            measures,
        }
    }

    #[allow(dead_code)] // called by the decision rule and report assembly in the gate-calibrate binary
    pub fn clears(&self, candidate_auc: f64) -> bool {
        if !candidate_auc.is_finite() || !self.required_discrimination_auc.is_finite() {
            return false;
        }
        candidate_auc >= self.required_discrimination_auc
            || (candidate_auc < self.required_discrimination_auc
                && ordered_f64_bits(candidate_auc)
                    .abs_diff(ordered_f64_bits(self.required_discrimination_auc))
                    <= BASELINE_COMPARISON_ULPS)
    }
}

fn structural_auc(novel: &[String], duplicate: &[String], measure: impl Fn(&str) -> usize) -> f64 {
    let novel_scores = novel
        .iter()
        .map(|text| measure(text) as f64)
        .collect::<Vec<_>>();
    let duplicate_scores = duplicate
        .iter()
        .map(|text| measure(text) as f64)
        .collect::<Vec<_>>();
    discrimination_auc(&novel_scores, &duplicate_scores)
}

fn paragraph_count(text: &str) -> usize {
    let mut paragraphs = 0;
    let mut inside_paragraph = false;
    for line in text.lines() {
        if line.trim().is_empty() {
            inside_paragraph = false;
        } else if !inside_paragraph {
            paragraphs += 1;
            inside_paragraph = true;
        }
    }
    paragraphs
}

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
    #[allow(dead_code)] // called by `pick_winner` in the gate-calibrate binary; not reached from the test target
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
    /// Whether this candidate passed every candidate-local decision-rule-v3
    /// eligibility check before relative throughput and tie-breaking:
    /// determinism, discrimination, complete novel/duplicate/paraphrase
    /// support, and dominance over the strongest no-model baseline. Persisted
    /// as audit evidence; consumers must recompute eligibility from the
    /// counters, AUC, and report baseline rather than trust this claim alone.
    /// The historic field name is retained for report-schema compatibility.
    #[serde(default)]
    pub passed_baseline_dominance: bool,
    /// Novel rows omitted from this candidate's discrimination AUC after a
    /// scorer error. Any non-zero count disqualifies the candidate from the
    /// baseline-dominance stage because the structural baseline uses every
    /// corpus row. `None` means the report did not evidence this counter and
    /// is also ineligible under decision-rule v3.
    #[serde(default)]
    pub dropped_novel_rows: Option<u64>,
    /// Duplicate rows omitted from this candidate's discrimination AUC after
    /// a scorer error. See [`CandidateResult::dropped_novel_rows`].
    #[serde(default)]
    pub dropped_duplicate_rows: Option<u64>,
    /// Paraphrase pairs omitted from `paraphrase_delta` because either half
    /// failed to score. Any non-zero count makes the candidate ineligible for
    /// weighted winner selection so selective failures cannot improve the
    /// metric. `None` is missing evidence and fails the same stage closed.
    #[serde(default)]
    pub dropped_paraphrase_rows: Option<u64>,
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
    /// Per-trace perplexity scores for every corpus slice. Added to feed the
    /// A2.7 perplexity-floor calibration math (Youden's-J optimum + 10th-
    /// percentile of novel slice), which needs the underlying per-trace
    /// arrays rather than the collapsed summary statistics on this row.
    ///
    /// Each vector aligns 1:1 with the corresponding `LoadedCorpus` slice:
    /// index `i` is the score for the `i`-th entry of that slice. Entries
    /// where the scorer raised an error (and a `score_failed` warn was
    /// emitted) serialize as JSON `null` so consumers can distinguish "this
    /// entry was scored at 0" from "this entry was not scored at all".
    ///
    /// The block is perplexity-derived and is therefore absent when the
    /// bake-off was invoked in `--scorer token-rarity` mode (no perplexity
    /// column to record). Token-rarity per-trace scores remain in the
    /// existing `metrics.token_rarity` sub-block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_trace_scores: Option<PerTraceScores>,
}

/// Per-trace perplexity scores aligned 1:1 with the corpus slices. `None`
/// entries mark scorer failures so a downstream calibration consumer can
/// drop them rather than treating them as legitimate zeros.
///
/// The paraphrase slice is split into the two halves of each pair so the
/// calibration consumer can correlate them positionally; index `i` of
/// `paraphrase_original` and `paraphrase_back_translation` are the two
/// scores for pair `i`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerTraceScores {
    pub novel: Vec<Option<f64>>,
    pub duplicate: Vec<Option<f64>>,
    pub paraphrase_original: Vec<Option<f64>>,
    pub paraphrase_back_translation: Vec<Option<f64>>,
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
        Self::failed_with_dropped_rows(
            id,
            license,
            params_b,
            release_date_unix,
            error_class,
            0,
            0,
            0,
        )
    }

    /// Construct a failed result while preserving the support lost before an
    /// evaluation abort. Load failures use [`CandidateResult::failed`], where
    /// no corpus row was attempted.
    #[allow(clippy::too_many_arguments)]
    pub fn failed_with_dropped_rows(
        id: String,
        license: License,
        params_b: u32,
        release_date_unix: i64,
        error_class: &str,
        dropped_novel_rows: u64,
        dropped_duplicate_rows: u64,
        dropped_paraphrase_rows: u64,
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
            passed_baseline_dominance: false,
            dropped_novel_rows: Some(dropped_novel_rows),
            dropped_duplicate_rows: Some(dropped_duplicate_rows),
            dropped_paraphrase_rows: Some(dropped_paraphrase_rows),
            release_date_unix,
            load_or_eval_error: Some(error_class.to_string()),
            metrics: None,
            per_trace_scores: None,
        }
    }
}

/// Evaluate every candidate-local decision-rule-v3 eligibility check once for
/// both winner selection and persisted report evidence. Relative throughput
/// and tie-breaking remain in [`pick_winner`] because they depend on peers.
#[allow(dead_code)] // called by the gate-calibrate binary; integration targets import this module independently
pub fn is_v3_candidate_eligible(candidate: &CandidateResult, baselines: &BaselineResults) -> bool {
    candidate.passed_determinism_gate
        && candidate.discrimination_auc > DISCRIMINATION_FLOOR
        && candidate.dropped_novel_rows == Some(0)
        && candidate.dropped_duplicate_rows == Some(0)
        && candidate.dropped_paraphrase_rows == Some(0)
        && baselines.clears(candidate.discrimination_auc)
}

/// Weighted score per the spec:
///   0.6 * AUC + 0.3 * (1 - clamp(paraphrase_delta, 0, 1)) + 0.1 * (tail / tail_norm_max)
/// When `tail_norm_max` is 0 the tail term collapses to 0 rather than dividing.
#[allow(dead_code)] // called by `pick_winner` in the gate-calibrate binary; not reached from the test target
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
/// 1. Drop candidates that fail the candidate-local v3 predicate also used to
///    assemble `passed_baseline_dominance`.
/// 2. Drop candidates slower than `THROUGHPUT_FLOOR_RATIO * fastest_throughput`,
///    measured over the discriminating set.
/// 3. Compute weighted scores using `max(tail_fraction_range)` over the
///    in-budget set as the normalizer.
/// 4. Anyone within `(1 - TIE_TOLERANCE)` of the top score is a contender.
/// 5. Break ties by: license permissiveness DESC, params_b ASC, release_date DESC.
#[allow(dead_code)] // called by the gate-calibrate binary; not reached from the test target
pub fn pick_winner<'a>(
    results: &'a [CandidateResult],
    baselines: &BaselineResults,
) -> Option<&'a CandidateResult> {
    // Step 1: the shared predicate covers every candidate-local v3 gate.
    // It precedes throughput so speed, licensing, model size, and recency
    // cannot rescue incomplete or non-discriminating evidence.
    let eligible: Vec<&CandidateResult> = results
        .iter()
        .filter(|r| is_v3_candidate_eligible(r, baselines))
        .collect();
    if eligible.is_empty() {
        return None;
    }

    // Step 2: throughput floor.
    let fastest = eligible
        .iter()
        .map(|r| r.throughput_tps)
        .fold(f64::NEG_INFINITY, f64::max);
    let floor = THROUGHPUT_FLOOR_RATIO * fastest;
    let in_budget: Vec<&CandidateResult> = eligible
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
    /// Structural no-model baselines and the resulting decision-rule-v3
    /// requirement. Empty on archived v1/v2 reports.
    #[serde(default)]
    pub baselines: BaselineResults,
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

    if report.decision_rule_version >= 3 {
        out.push_str("## No-model structural baselines\n\n");
        out.push_str("| baseline | auc | strongest |\n");
        out.push_str("| --- | --- | --- |\n");
        for baseline in &report.baselines.measures {
            let strongest = report.baselines.strongest_name.as_deref() == Some(&baseline.name);
            out.push_str(&format!(
                "| {} | {:.6} | {} |\n",
                baseline.name, baseline.discrimination_auc, strongest
            ));
        }
        let strongest = report.baselines.strongest_name.as_deref().unwrap_or("none");
        out.push_str(&format!(
            "\nStrongest baseline: {} ({:.6})\n\nRequired discrimination AUC: {:.6}\n\n",
            strongest, report.baselines.strongest_auc, report.baselines.required_discrimination_auc
        ));
    }

    let winner = report.winner_id.as_deref().unwrap_or("none");
    out.push_str(&format!("Winner: {}\n\n", winner));

    out.push_str("| candidate | auc | paraphrase_delta | tail_range | throughput_tps | determinism_stddev | license | params_b | passed_determinism | dropped_novel_rows | dropped_duplicate_rows | dropped_paraphrase_rows | passed_baseline_dominance |\n");
    out.push_str(
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for c in &report.candidates {
        let dropped_novel_rows = c
            .dropped_novel_rows
            .map_or_else(|| "not_evidenced".to_string(), |count| count.to_string());
        let dropped_duplicate_rows = c
            .dropped_duplicate_rows
            .map_or_else(|| "not_evidenced".to_string(), |count| count.to_string());
        let dropped_paraphrase_rows = c
            .dropped_paraphrase_rows
            .map_or_else(|| "not_evidenced".to_string(), |count| count.to_string());
        out.push_str(&format!(
            "| {} | {:.6} | {:.6} | {:.6} | {:.3} | {:.3e} | {:?} | {} | {} | {} | {} | {} | {} |\n",
            c.id,
            c.discrimination_auc,
            c.paraphrase_delta,
            c.tail_fraction_range,
            c.throughput_tps,
            c.determinism_stddev,
            c.license,
            c.params_b,
            c.passed_determinism_gate,
            dropped_novel_rows,
            dropped_duplicate_rows,
            dropped_paraphrase_rows,
            c.passed_baseline_dominance,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A candidate with everything the decision rule ignores zeroed out.
    /// Callers set only the fields their case is about.
    fn candidate(id: &str, discrimination_auc: f64, throughput_tps: f64) -> CandidateResult {
        CandidateResult {
            id: id.to_string(),
            discrimination_auc,
            paraphrase_delta: 0.0,
            tail_fraction_range: 0.0,
            determinism_stddev: 0.0,
            throughput_tps,
            peak_vram_mib: 0,
            license: License::Apache2,
            params_b: 8,
            passed_determinism_gate: true,
            passed_baseline_dominance: false,
            dropped_novel_rows: Some(0),
            dropped_duplicate_rows: Some(0),
            dropped_paraphrase_rows: Some(0),
            release_date_unix: 0,
            load_or_eval_error: None,
            metrics: None,
            per_trace_scores: None,
        }
    }

    /// The four candidates of the A2.6 run, verbatim from
    /// `docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.json`.
    fn a26_candidates() -> Vec<CandidateResult> {
        vec![
            CandidateResult {
                paraphrase_delta: 0.8291336223450885,
                tail_fraction_range: 0.015259500000000002,
                license: License::LlamaCommunity,
                release_date_unix: 1721692800,
                ..candidate(
                    "llama-3.1-8b-instruct",
                    0.34253333333333336,
                    291.7555607226632,
                )
            },
            CandidateResult {
                paraphrase_delta: 0.8233920956679962,
                tail_fraction_range: 0.007072000000000002,
                release_date_unix: 1745884800,
                ..candidate("qwen3-8b-base", 0.2431111111111111, 248.4598989824905)
            },
            CandidateResult {
                paraphrase_delta: 0.5980157409848676,
                tail_fraction_range: 0.09649600000000001,
                params_b: 27,
                release_date_unix: 1776470400,
                ..candidate("qwen3.6-27b-dense", 0.9362666666666667, 119.49181844340372)
            },
            CandidateResult {
                paraphrase_delta: 1.5228894106767636,
                tail_fraction_range: 0.011243999999999997,
                params_b: 31,
                passed_determinism_gate: false,
                release_date_unix: 1774742400,
                ..candidate("gemma-4-31b", 0.09660649819494585, 208.67786910267276)
            },
        ]
    }

    fn baselines(strongest_auc: f64) -> BaselineResults {
        BaselineResults {
            measures: vec![NoModelBaseline {
                name: "test_baseline".to_string(),
                discrimination_auc: strongest_auc,
            }],
            strongest_name: Some("test_baseline".to_string()),
            strongest_auc,
            required_discrimination_auc: strongest_auc + BASELINE_DOMINANCE_MARGIN,
        }
    }

    /// Under v2, qwen3.6 was the only candidate that reached the throughput
    /// stage and therefore won. Decision-rule v3 rejects the same archived
    /// candidate set because paragraph count already has AUC 1.0.
    #[test]
    fn a26_v3_has_no_winner_while_v2_selected_qwen36() {
        let results = a26_candidates();
        let v2_eligible = results
            .iter()
            .filter(|r| r.passed_determinism_gate)
            .filter(|r| r.discrimination_auc > DISCRIMINATION_FLOOR)
            .collect::<Vec<_>>();
        assert_eq!(v2_eligible.len(), 1);
        assert_eq!(v2_eligible[0].id, "qwen3.6-27b-dense");
        let v2_winner = pick_winner(&results, &baselines(0.5))
            .expect("a nonbinding baseline reproduces the v2 result");
        assert_eq!(v2_winner.id, "qwen3.6-27b-dense");
        assert!(pick_winner(&results, &baselines(1.0)).is_none());
    }

    #[test]
    fn sub_chance_candidates_are_dropped_before_the_throughput_floor() {
        let results = a26_candidates();
        let survivors: Vec<&str> = results
            .iter()
            .filter(|r| r.passed_determinism_gate)
            .filter(|r| r.discrimination_auc > DISCRIMINATION_FLOOR)
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(survivors, vec!["qwen3.6-27b-dense"]);
    }

    #[test]
    fn no_winner_when_nothing_discriminates() {
        let results: Vec<CandidateResult> = a26_candidates()
            .into_iter()
            .filter(|r| r.id != "qwen3.6-27b-dense")
            .collect();
        assert!(
            pick_winner(&results, &baselines(0.0)).is_none(),
            "a run where nothing beats chance has no winner, not a fast loser"
        );
    }

    /// A candidate exactly at chance carries no signal and must not win.
    #[test]
    fn exactly_chance_is_not_discriminating() {
        let results = vec![candidate("coin-flip", 0.5, 1000.0)];
        assert!(pick_winner(&results, &baselines(0.0)).is_none());
    }

    /// Throughput still decides among candidates that all discriminate.
    #[test]
    fn throughput_floor_still_applies_within_the_discriminating_set() {
        let results = vec![
            CandidateResult {
                tail_fraction_range: 0.05,
                ..candidate("fast-good", 0.90, 300.0)
            },
            CandidateResult {
                tail_fraction_range: 0.05,
                ..candidate("slow-better", 0.95, 100.0)
            },
        ];
        let winner = pick_winner(&results, &baselines(0.6)).expect("both discriminate");
        assert_eq!(
            winner.id, "fast-good",
            "within the discriminating set the throughput floor is unchanged"
        );
    }

    #[test]
    fn candidate_at_the_baseline_margin_wins_normally() {
        let baselines = baselines(0.75);
        let results = vec![candidate(
            "boundary",
            baselines.required_discrimination_auc,
            100.0,
        )];
        let winner = pick_winner(&results, &baselines).expect("margin is inclusive");
        assert_eq!(winner.id, "boundary");
    }

    #[test]
    fn ulp_adjusted_comparison_boundary_is_inclusive() {
        let baselines = baselines(0.75);
        let candidate_auc = f64::from_bits(
            baselines.required_discrimination_auc.to_bits() - BASELINE_COMPARISON_ULPS,
        );
        assert_eq!(
            baselines.required_discrimination_auc.to_bits() - candidate_auc.to_bits(),
            BASELINE_COMPARISON_ULPS
        );
        assert!(baselines.clears(candidate_auc));
    }

    #[test]
    fn later_stage_advantages_cannot_rescue_a_baseline_failure() {
        let baselines = baselines(0.8);
        let mut failing = candidate("fast-permissive-small-new", 0.84, 1000.0);
        failing.license = License::Apache2;
        failing.params_b = 1;
        failing.release_date_unix = i64::MAX;
        failing.paraphrase_delta = 0.0;
        failing.tail_fraction_range = 1.0;

        let mut passing = candidate("slow-passing", baselines.required_discrimination_auc, 100.0);
        passing.license = License::LlamaCommunity;
        passing.params_b = u32::MAX;
        passing.release_date_unix = 0;
        passing.paraphrase_delta = 1.0;
        passing.tail_fraction_range = 0.0;

        let results = [failing, passing];
        let winner =
            pick_winner(&results, &baselines).expect("baseline filter must run before throughput");
        assert_eq!(winner.id, "slow-passing");
    }

    #[test]
    fn required_floor_above_one_is_not_clamped_and_has_no_winner() {
        let baselines = baselines(1.0);
        assert_eq!(baselines.required_discrimination_auc, 1.05);
        let results = vec![candidate("perfect-model", 1.0, 100.0)];
        assert!(pick_winner(&results, &baselines).is_none());
    }
}
