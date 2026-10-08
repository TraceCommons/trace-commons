// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fit and evaluate a local credit estimate table from label-only rows.
//!
//! The contributor app estimates a waiting session's credit on the device,
//! from content-free features (`lef1`, in
//! [`trace_commons_protocol::local_credit_estimate`]) looked up in a published
//! table. This module is the server half: given one label-only row per scored
//! decision, it fits the table's weights and tiers on the earlier part of the
//! corpus, measures them on the later part against the go/no-go bar
//! (OWNER DECISION E9), and emits a candidate table. A candidate that misses
//! the bar is a one-tier table whose band is the held-out p10-p90: still
//! true, and with no ranking effect.
//!
//! Pure: no storage, no network, no clock, no randomness. Rows carry features
//! and labels only, never an id or content. Scores are computed with the
//! protocol's own [`estimate_score`] so the fit and the device cannot differ.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::Serialize;
use trace_commons_protocol::local_credit_estimate::{
    BUILT_IN_ESTIMATE_BYTES_PER_TOKEN, BUILT_IN_ESTIMATE_CHUNK_CAP,
    BUILT_IN_ESTIMATE_CHUNK_TARGET_TOKENS, ESTIMATE_MAX_ABS_WEIGHT, ESTIMATE_MAX_TIERS,
    ESTIMATE_TABLE_SCHEMA_VERSION, EstimateBand, EstimateTerm, EstimateWeight,
    LOCAL_ESTIMATE_FEATURES_VERSION, LocalEstimateFeatures, LocalEstimateTable, estimate_score,
};

use crate::rescore_distribution::MIN_ROWS_FOR_PERCENTILES;

/// Share of the corpus, oldest first by `decided_at`, the table is fit on;
/// the rest is held out. A time split, because the deployment question is
/// whether last month's table predicts this month.
/// OWNER DECISION E9.
pub const ESTIMATE_FIT_TRAIN_SHARE: f64 = 0.70;

/// Minimum held-out Spearman correlation between the score and displayed
/// credit. OWNER DECISION E9.
pub const ESTIMATE_MIN_SPEARMAN: f64 = 0.30;

/// Minimum Spearman gain over ranking by `user_messages` alone, the
/// server-computable stand-in for the incumbent order. OWNER DECISION E9.
pub const ESTIMATE_MIN_SPEARMAN_GAIN: f64 = 0.10;

/// Minimum held-out tier agreement above the majority-class baseline.
/// OWNER DECISION E9.
pub const ESTIMATE_MIN_TIER_GAIN: f64 = 0.10;

/// Minimum share of held-out displayed credit inside its predicted tier's
/// band (nominal 0.80 for a p10-p90 band). OWNER DECISION E9.
pub const ESTIMATE_MIN_BAND_COVERAGE: f64 = 0.75;

/// A published tier must rest on at least this many decisions; a tier under
/// it merges into its neighbour. OWNER DECISION E10.
pub const ESTIMATE_MIN_TIER_ROWS: usize = 50;

/// A published tier must rest on decisions from at least this many tenants;
/// a tier under it merges into its neighbour. OWNER DECISION E10.
pub const ESTIMATE_MIN_TIER_TENANTS: usize = 5;

/// Band ends: p10 and p90 of displayed credit. OWNER DECISION E3.
pub const ESTIMATE_BAND_LOW_QUANTILE: f64 = 0.10;
/// See [`ESTIMATE_BAND_LOW_QUANTILE`]. OWNER DECISION E3.
pub const ESTIMATE_BAND_HIGH_QUANTILE: f64 = 0.90;

/// Only decisions scored under this credit-quality calibration train a
/// table, and the table is labelled `cq<version>`. Spec section 4.3, item 1:
/// "Only calibration-version-3 rows train a V3 table". OWNER DECISION E9.
pub const ESTIMATE_FIT_CALIBRATION_VERSION: i32 = 3;

/// The terms a fit may weight, in a fixed order.
pub const ESTIMATE_FIT_TERMS: [EstimateTerm; 7] = [
    EstimateTerm::LnContentBytes,
    EstimateTerm::Capped,
    EstimateTerm::ToolResultShare,
    EstimateTerm::AgentProseShare,
    EstimateTerm::ByteEntropy,
    EstimateTerm::UserMessagesCapped,
    EstimateTerm::LnDistinctTools,
];

/// Ridge term added to the standardized normal equations so a collinear
/// pair of features still solves. Small enough not to move a well-posed fit.
const RIDGE_LAMBDA: f64 = 1.0e-6;

/// The `credit_withheld_reason` labels the gate stamps on a decision it
/// recorded without scoring because the content already had one. Mirrors
/// the ingest binary's list, which the contributor status read uses.
pub const ESTIMATE_DUPLICATE_WITHHELD_REASONS: [&str; 2] = ["skipped_duplicate", "cached"];

/// Why a decision displays as 0 rather than as a scored figure. Bands
/// exclude these (OWNER DECISION E6); the table publishes their share.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EstimateWithheldLabel {
    /// `skipped_duplicate` or `cached`.
    Duplicate,
    /// Scored, with credit quality 0 (the anomaly hard-withhold).
    ZeroQuality,
    /// Any other withheld reason the gate recorded.
    Other,
}

/// Label a decision for the eval, or `None` when it carries no label yet
/// (no credit quality and no withheld reason: still being scored).
#[must_use]
pub fn eval_label(
    credit_quality_micros: Option<i64>,
    credit_withheld_reason: Option<&str>,
) -> Option<Option<EstimateWithheldLabel>> {
    match (credit_quality_micros, credit_withheld_reason) {
        (Some(q), _) if q > 0 => Some(None),
        (Some(_), _) => Some(Some(EstimateWithheldLabel::ZeroQuality)),
        (None, Some(reason)) if ESTIMATE_DUPLICATE_WITHHELD_REASONS.contains(&reason) => {
            Some(Some(EstimateWithheldLabel::Duplicate))
        }
        (None, Some(_)) => Some(Some(EstimateWithheldLabel::Other)),
        (None, None) => None,
    }
}

/// One label-only row, as the eval route returns it. No submission id, no
/// trace id, no decision time, no content.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EstimateEvalRow {
    pub features: LocalEstimateFeatures,
    pub credit_quality_micros: Option<i64>,
    pub credit_quality_calibration_version: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub withheld: Option<EstimateWithheldLabel>,
    /// `sha256:` of the tenant id, for the per-tier tenant floor.
    pub tenant_hash: String,
}

/// A row plus the decision time the split needs. The time never leaves the
/// server.
#[derive(Debug, Clone, PartialEq)]
pub struct EstimateFitInput {
    pub row: EstimateEvalRow,
    pub decided_at: DateTime<Utc>,
}

/// What a fit reports. Aggregates only.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EstimateFitReport {
    /// Rows in the fit window / held out, scored under the trained
    /// calibration with every fitted feature known.
    pub fit_rows: usize,
    pub held_out_rows: usize,
    /// Withheld rows in the fit window, and their share of the window's
    /// labelled rows.
    pub fit_withheld_rows: usize,
    pub withheld_share: Option<f64>,
    /// Tiers placed before and after the E10 merge.
    pub tiers_before_merge: usize,
    pub tiers: usize,
    pub spearman: Option<f64>,
    pub baseline_spearman: Option<f64>,
    pub tier_agreement: Option<f64>,
    pub majority_baseline: Option<f64>,
    pub band_coverage: Option<f64>,
    /// Every E9 check passed and the candidate has more than one tier.
    pub passed: bool,
    /// Names of the checks that did not pass.
    pub failed_checks: Vec<&'static str>,
    /// The candidate table, validated. Absent with `no_table_reason`.
    pub table: Option<LocalEstimateTable>,
    pub no_table_reason: Option<&'static str>,
}

struct Scored<'a> {
    features: &'a LocalEstimateFeatures,
    displayed: f64,
    tenant: &'a str,
}

/// Displayed credit for a credit quality: `round(10 * q, 2)`, as the
/// contributor status read presents it.
#[must_use]
pub fn displayed_credit(credit_quality_micros: i64) -> f64 {
    let q = credit_quality_micros.clamp(0, 1_000_000) as f64 / 1_000_000.0;
    (10.0 * q * 100.0).round() / 100.0
}

fn base_table(version: String) -> LocalEstimateTable {
    LocalEstimateTable {
        schema_version: ESTIMATE_TABLE_SCHEMA_VERSION,
        features_version: LOCAL_ESTIMATE_FEATURES_VERSION.to_string(),
        version,
        credit_quality_calibration: format!("cq{ESTIMATE_FIT_CALIBRATION_VERSION}"),
        bytes_per_token: BUILT_IN_ESTIMATE_BYTES_PER_TOKEN,
        chunk_target_tokens: BUILT_IN_ESTIMATE_CHUNK_TARGET_TOKENS,
        chunk_cap: BUILT_IN_ESTIMATE_CHUNK_CAP,
        weights: Vec::new(),
        cut_offs: Vec::new(),
        bands: Vec::new(),
        withheld_share: None,
    }
}

/// Fit, evaluate and emit a candidate table. See the module docs.
#[must_use]
pub fn fit_estimate_table(inputs: &[EstimateFitInput]) -> EstimateFitReport {
    let mut report = EstimateFitReport {
        fit_rows: 0,
        held_out_rows: 0,
        fit_withheld_rows: 0,
        withheld_share: None,
        tiers_before_merge: 0,
        tiers: 0,
        spearman: None,
        baseline_spearman: None,
        tier_agreement: None,
        majority_baseline: None,
        band_coverage: None,
        passed: false,
        failed_checks: Vec::new(),
        table: None,
        no_table_reason: None,
    };

    // Labelled rows only: scored under the trained calibration, or withheld.
    let mut labelled: Vec<&EstimateFitInput> = inputs
        .iter()
        .filter(|input| {
            input.row.withheld.is_some()
                || input.row.credit_quality_calibration_version
                    == Some(ESTIMATE_FIT_CALIBRATION_VERSION)
        })
        .collect();
    labelled.sort_by_key(|input| input.decided_at);
    let Some(last) = labelled.last() else {
        report.no_table_reason = Some("no_labelled_rows");
        return report;
    };
    let version = format!("f{}", last.decided_at.format("%Y%m%d"));
    let split = ((labelled.len() as f64) * ESTIMATE_FIT_TRAIN_SHARE).floor() as usize;
    let (fit_window, held_window) = labelled.split_at(split);

    let reference = base_table(version.clone());
    let fit: Vec<Scored<'_>> = fit_window
        .iter()
        .filter_map(|input| usable(input, &reference))
        .collect();
    let held: Vec<Scored<'_>> = held_window
        .iter()
        .filter_map(|input| usable(input, &reference))
        .collect();
    report.fit_rows = fit.len();
    report.held_out_rows = held.len();
    report.fit_withheld_rows = fit_window
        .iter()
        .filter(|input| input.row.withheld.is_some())
        .count();
    let fit_labelled = report.fit_withheld_rows + fit.len();
    report.withheld_share =
        (fit_labelled > 0).then(|| report.fit_withheld_rows as f64 / fit_labelled as f64);

    if held.len() < MIN_ROWS_FOR_PERCENTILES {
        report.no_table_reason = Some("insufficient_held_out_rows");
        return report;
    }

    let candidate = fitted_candidate(&fit, &reference, &mut report);
    if let Some(table) = candidate {
        evaluate(&table, &fit, &held, &mut report);
        if report.passed {
            report.table = finish(table, report.withheld_share, &mut report);
            return report;
        }
    } else if report.failed_checks.is_empty() {
        report.failed_checks.push("fit");
    }

    // The bar was missed: one tier, the held-out band, no weights.
    report.passed = false;
    let tenants: BTreeSet<&str> = held.iter().map(|s| s.tenant).collect();
    if held.len() < ESTIMATE_MIN_TIER_ROWS || tenants.len() < ESTIMATE_MIN_TIER_TENANTS {
        report.no_table_reason = Some("below_non_personal_floor");
        return report;
    }
    let mut ys: Vec<f64> = held.iter().map(|s| s.displayed).collect();
    ys.sort_by(f64::total_cmp);
    let mut table = base_table(version);
    table.bands = vec![EstimateBand {
        low: quantile(&ys, ESTIMATE_BAND_LOW_QUANTILE),
        high: quantile(&ys, ESTIMATE_BAND_HIGH_QUANTILE),
    }];
    report.table = finish(table, report.withheld_share, &mut report);
    report
}

/// A scored row the fit can use: trained calibration, not withheld, known
/// features for every fitted term (or the device could not score it).
fn usable<'a>(input: &'a EstimateFitInput, reference: &LocalEstimateTable) -> Option<Scored<'a>> {
    if input.row.withheld.is_some() {
        return None;
    }
    let q = input.row.credit_quality_micros?;
    let features = &input.row.features;
    if features.version != LOCAL_ESTIMATE_FEATURES_VERSION || features.content_bytes == 0 {
        return None;
    }
    ESTIMATE_FIT_TERMS
        .iter()
        .all(|term| features.term_value(*term, reference).is_some())
        .then(|| Scored {
            features,
            displayed: displayed_credit(q),
            tenant: &input.row.tenant_hash,
        })
}

fn finish(
    mut table: LocalEstimateTable,
    withheld_share: Option<f64>,
    report: &mut EstimateFitReport,
) -> Option<LocalEstimateTable> {
    table.withheld_share = withheld_share;
    match table.validate() {
        Ok(()) => Some(table),
        Err(_) => {
            report.no_table_reason = Some("candidate_refused_by_validation");
            None
        }
    }
}

/// Weights, cut-offs and bands from the fit window, or `None` when no
/// multi-tier table can be built.
fn fitted_candidate(
    fit: &[Scored<'_>],
    reference: &LocalEstimateTable,
    report: &mut EstimateFitReport,
) -> Option<LocalEstimateTable> {
    if fit.len() < ESTIMATE_MIN_TIER_ROWS * 2 {
        report.failed_checks.push("fit_rows");
        return None;
    }
    let ys: Vec<f64> = fit.iter().map(|s| s.displayed).collect();
    let xs: Vec<Vec<f64>> = fit
        .iter()
        .map(|s| {
            ESTIMATE_FIT_TERMS
                .iter()
                .map(|term| s.features.term_value(*term, reference).unwrap_or(0.0))
                .collect()
        })
        .collect();
    let weights = least_squares(&xs, &ys)?;
    if weights
        .iter()
        .any(|w| !w.is_finite() || w.abs() > ESTIMATE_MAX_ABS_WEIGHT)
    {
        report.failed_checks.push("weights_out_of_range");
        return None;
    }
    let mut table = reference.clone();
    table.weights = ESTIMATE_FIT_TERMS
        .iter()
        .zip(&weights)
        .filter(|(_, w)| **w != 0.0)
        .map(|(term, w)| EstimateWeight {
            term: *term,
            weight: *w,
        })
        .collect();

    let scores: Vec<f64> = fit
        .iter()
        .map(|s| estimate_score(s.features, &table).unwrap_or(f64::NAN))
        .collect();
    if scores.iter().any(|s| !s.is_finite()) {
        report.failed_checks.push("fit");
        return None;
    }
    let mut sorted = scores.clone();
    sorted.sort_by(f64::total_cmp);
    let mut cut_offs: Vec<f64> = (1..ESTIMATE_MAX_TIERS)
        .map(|k| quantile(&sorted, k as f64 / ESTIMATE_MAX_TIERS as f64))
        .collect();
    cut_offs.dedup_by(|a, b| a <= b);
    cut_offs.retain(|cut| *cut > sorted[0]);
    report.tiers_before_merge = cut_offs.len() + 1;
    merge_under_floor(&mut cut_offs, &scores, fit);
    report.tiers = cut_offs.len() + 1;
    if cut_offs.is_empty() {
        report.failed_checks.push("one_tier_after_floor");
        return None;
    }
    let mut bands = Vec::with_capacity(cut_offs.len() + 1);
    for tier in 0..=cut_offs.len() {
        let mut in_tier: Vec<f64> = scores
            .iter()
            .zip(&ys)
            .filter(|(score, _)| tier_of(**score, &cut_offs) == tier)
            .map(|(_, y)| *y)
            .collect();
        in_tier.sort_by(f64::total_cmp);
        bands.push(EstimateBand {
            low: quantile(&in_tier, ESTIMATE_BAND_LOW_QUANTILE),
            high: quantile(&in_tier, ESTIMATE_BAND_HIGH_QUANTILE),
        });
    }
    table.cut_offs = cut_offs;
    table.bands = bands;
    Some(table)
}

/// Remove cut-offs until every tier has [`ESTIMATE_MIN_TIER_ROWS`] rows from
/// [`ESTIMATE_MIN_TIER_TENANTS`] tenants. The smallest failing tier merges
/// into its smaller neighbour.
fn merge_under_floor(cut_offs: &mut Vec<f64>, scores: &[f64], fit: &[Scored<'_>]) {
    while !cut_offs.is_empty() {
        let tiers = cut_offs.len() + 1;
        let mut rows = vec![0usize; tiers];
        let mut tenants: Vec<BTreeSet<&str>> = vec![BTreeSet::new(); tiers];
        for (score, scored) in scores.iter().zip(fit) {
            let tier = tier_of(*score, cut_offs);
            rows[tier] += 1;
            tenants[tier].insert(scored.tenant);
        }
        let failing = (0..tiers)
            .filter(|t| {
                rows[*t] < ESTIMATE_MIN_TIER_ROWS || tenants[*t].len() < ESTIMATE_MIN_TIER_TENANTS
            })
            .min_by_key(|t| rows[*t]);
        let Some(tier) = failing else {
            return;
        };
        // Cut-off `i` separates tier `i` from tier `i + 1`.
        let remove = if tier == 0 {
            0
        } else if tier == tiers - 1 || rows[tier - 1] <= rows[tier + 1] {
            tier - 1
        } else {
            tier
        };
        cut_offs.remove(remove);
    }
}

fn tier_of(score: f64, cut_offs: &[f64]) -> usize {
    cut_offs.iter().filter(|cut| score >= **cut).count()
}

fn evaluate(
    table: &LocalEstimateTable,
    fit: &[Scored<'_>],
    held: &[Scored<'_>],
    report: &mut EstimateFitReport,
) {
    let held_scores: Vec<f64> = held
        .iter()
        .map(|s| estimate_score(s.features, table).unwrap_or(f64::NAN))
        .collect();
    let ys: Vec<f64> = held.iter().map(|s| s.displayed).collect();
    let users: Vec<f64> = held
        .iter()
        .map(|s| f64::from(s.features.user_messages))
        .collect();
    report.spearman = spearman(&held_scores, &ys);
    report.baseline_spearman = spearman(&users, &ys);

    // The actual tier of a held-out row: its displayed credit cut at the fit
    // window's displayed-credit quantiles matching each score cut-off.
    let fit_scores: Vec<f64> = fit
        .iter()
        .map(|s| estimate_score(s.features, table).unwrap_or(f64::NAN))
        .collect();
    let mut fit_ys: Vec<f64> = fit.iter().map(|s| s.displayed).collect();
    fit_ys.sort_by(f64::total_cmp);
    let y_cuts: Vec<f64> = table
        .cut_offs
        .iter()
        .map(|cut| {
            let below = fit_scores.iter().filter(|s| **s < *cut).count();
            quantile(&fit_ys, below as f64 / fit_scores.len() as f64)
        })
        .collect();
    let tiers = table.cut_offs.len() + 1;
    let mut actual_counts = vec![0usize; tiers];
    let mut agree = 0usize;
    let mut covered = 0usize;
    for (score, y) in held_scores.iter().zip(&ys) {
        let predicted = tier_of(*score, &table.cut_offs);
        let actual = tier_of(*y, &y_cuts);
        actual_counts[actual] += 1;
        agree += usize::from(predicted == actual);
        let band = &table.bands[predicted];
        covered += usize::from(*y >= band.low && *y <= band.high);
    }
    let n = held.len() as f64;
    let agreement = agree as f64 / n;
    let majority = actual_counts.iter().copied().max().unwrap_or(0) as f64 / n;
    let coverage = covered as f64 / n;
    report.tier_agreement = Some(agreement);
    report.majority_baseline = Some(majority);
    report.band_coverage = Some(coverage);

    let mut failed = Vec::new();
    match (report.spearman, report.baseline_spearman) {
        (Some(rho), baseline) => {
            if rho < ESTIMATE_MIN_SPEARMAN {
                failed.push("spearman");
            }
            // An undefined baseline (every row the same user count) ranks
            // nothing, so it counts as 0 for the gain.
            if rho - baseline.unwrap_or(0.0) < ESTIMATE_MIN_SPEARMAN_GAIN {
                failed.push("spearman_gain");
            }
        }
        (None, _) => failed.push("spearman"),
    }
    if agreement - majority < ESTIMATE_MIN_TIER_GAIN {
        failed.push("tier_gain");
    }
    if coverage < ESTIMATE_MIN_BAND_COVERAGE {
        failed.push("band_coverage");
    }
    report.passed = failed.is_empty();
    report.failed_checks.extend(failed);
}

/// Linear-interpolated quantile of an ascending, non-empty slice.
fn quantile(sorted: &[f64], p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    let position = p * (sorted.len() - 1) as f64;
    let below = position.floor() as usize;
    let above = position.ceil() as usize;
    let fraction = position - below as f64;
    sorted[below] + (sorted[above] - sorted[below]) * fraction
}

/// Average ranks, ties sharing the mean of their positions.
fn ranks(values: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|a, b| values[*a].total_cmp(&values[*b]));
    let mut ranks = vec![0.0; values.len()];
    let mut start = 0;
    while start < order.len() {
        let mut end = start;
        while end + 1 < order.len() && values[order[end + 1]] == values[order[start]] {
            end += 1;
        }
        let rank = (start + end) as f64 / 2.0;
        for index in &order[start..=end] {
            ranks[*index] = rank;
        }
        start = end + 1;
    }
    ranks
}

/// Spearman's rho, or `None` when either side has no variance or a value is
/// not finite.
fn spearman(a: &[f64], b: &[f64]) -> Option<f64> {
    if a.len() != b.len() || a.len() < 2 || a.iter().chain(b).any(|v| !v.is_finite()) {
        return None;
    }
    let (ra, rb) = (ranks(a), ranks(b));
    let n = ra.len() as f64;
    let (ma, mb) = (ra.iter().sum::<f64>() / n, rb.iter().sum::<f64>() / n);
    let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
    for (x, y) in ra.iter().zip(&rb) {
        cov += (x - ma) * (y - mb);
        va += (x - ma) * (x - ma);
        vb += (y - mb) * (y - mb);
    }
    (va > 0.0 && vb > 0.0).then(|| cov / (va * vb).sqrt())
}

/// Ordinary least squares with an intercept, on standardized columns, with
/// a tiny ridge. Returns one weight per column on the original scale; a
/// constant column gets weight 0. The intercept is dropped: tiers are cut on
/// the score, so a constant shift moves nothing.
fn least_squares(xs: &[Vec<f64>], ys: &[f64]) -> Option<Vec<f64>> {
    let n = xs.len();
    let k = xs.first()?.len();
    let mean = |j: usize| xs.iter().map(|row| row[j]).sum::<f64>() / n as f64;
    let means: Vec<f64> = (0..k).map(mean).collect();
    let sds: Vec<f64> = (0..k)
        .map(|j| {
            let var = xs
                .iter()
                .map(|row| (row[j] - means[j]).powi(2))
                .sum::<f64>()
                / n as f64;
            var.sqrt()
        })
        .collect();
    let live: Vec<usize> = (0..k).filter(|j| sds[*j] > 1.0e-12).collect();
    let y_mean = ys.iter().sum::<f64>() / n as f64;
    let m = live.len();
    // Normal equations on centred, standardized columns: (Z'Z + lambda I) b = Z'y.
    let mut a = vec![vec![0.0; m + 1]; m];
    for (row, y) in xs.iter().zip(ys) {
        let z: Vec<f64> = live
            .iter()
            .map(|j| (row[*j] - means[*j]) / sds[*j])
            .collect();
        for i in 0..m {
            for jj in 0..m {
                a[i][jj] += z[i] * z[jj];
            }
            a[i][m] += z[i] * (y - y_mean);
        }
    }
    for (i, row) in a.iter_mut().enumerate() {
        row[i] += RIDGE_LAMBDA * n as f64;
    }
    let beta = solve(a)?;
    let mut weights = vec![0.0; k];
    for (slot, j) in live.iter().enumerate() {
        weights[*j] = beta[slot] / sds[*j];
    }
    Some(weights)
}

/// Gaussian elimination with partial pivoting on an augmented matrix.
fn solve(mut a: Vec<Vec<f64>>) -> Option<Vec<f64>> {
    let m = a.len();
    for col in 0..m {
        let pivot = (col..m).max_by(|x, y| a[*x][col].abs().total_cmp(&a[*y][col].abs()))?;
        if a[pivot][col].abs() < 1.0e-12 {
            return None;
        }
        a.swap(col, pivot);
        for row in (col + 1)..m {
            let factor = a[row][col] / a[col][col];
            for c in col..=m {
                a[row][c] -= factor * a[col][c];
            }
        }
    }
    let mut x = vec![0.0; m];
    for row in (0..m).rev() {
        let tail: f64 = ((row + 1)..m).map(|c| a[row][c] * x[c]).sum();
        x[row] = (a[row][m] - tail) / a[row][row];
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use trace_commons_protocol::local_credit_estimate::{
        EstimateRole, LocalEstimateAccumulator, estimate,
    };

    /// A deterministic pseudo-random stream, so every outcome is fixed.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    fn features(content_bytes: usize, user_messages: usize, tools: usize) -> LocalEstimateFeatures {
        let user = "u".repeat(8);
        let body = "abcdefghij".repeat(content_bytes / 10 + 1);
        let names: Vec<String> = (0..tools).map(|i| format!("tool{i}")).collect();
        let mut acc = LocalEstimateAccumulator::new();
        for _ in 0..user_messages {
            acc.accumulate(EstimateRole::User, Some(&user), None);
        }
        acc.accumulate(
            EstimateRole::Assistant,
            Some(&body[..content_bytes / 2]),
            None,
        );
        for name in &names {
            acc.accumulate(EstimateRole::Other, Some("call"), Some(name));
        }
        acc.accumulate(
            EstimateRole::ToolResult,
            Some(&body[..content_bytes / 2]),
            None,
        );
        acc.finish()
    }

    fn at(i: usize) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap() + chrono::Duration::minutes(i as i64)
    }

    /// `n` scored rows over `tenants` tenants. With `signal`, displayed
    /// credit rises with content size; without, it ignores the features.
    fn rows(n: usize, tenants: usize, signal: bool, seed: u64) -> Vec<EstimateFitInput> {
        rows_shaped(n, tenants, signal, seed, false)
    }

    /// With `fixed_shape`, every row has the same user and tool counts, so
    /// the score orders rows by content size alone.
    fn rows_shaped(
        n: usize,
        tenants: usize,
        signal: bool,
        seed: u64,
        fixed_shape: bool,
    ) -> Vec<EstimateFitInput> {
        let mut rng = Lcg(seed);
        (0..n)
            .map(|i| {
                let size = (200.0 * (8.0f64).powf(rng.next() * 3.0)) as usize;
                let users = 1 + (rng.next() * 15.0) as usize;
                let noise = rng.next();
                let displayed = if signal {
                    0.35 * (size as f64).ln() - 0.6 + 0.3 * noise
                } else {
                    1.3 + 1.6 * noise
                };
                EstimateFitInput {
                    row: EstimateEvalRow {
                        features: if fixed_shape {
                            features(size, 3, 1)
                        } else {
                            features(size, users, 1 + i % 3)
                        },
                        credit_quality_micros: Some((displayed * 100_000.0).round() as i64),
                        credit_quality_calibration_version: Some(3),
                        withheld: None,
                        tenant_hash: format!("sha256:tenant-{}", i % tenants),
                    },
                    decided_at: at(i),
                }
            })
            .collect()
    }

    #[test]
    fn eval_label_tells_scored_withheld_and_unlabelled_apart() {
        assert_eq!(eval_label(Some(150_000), None), Some(None));
        assert_eq!(
            eval_label(Some(0), None),
            Some(Some(EstimateWithheldLabel::ZeroQuality))
        );
        assert_eq!(
            eval_label(None, Some("skipped_duplicate")),
            Some(Some(EstimateWithheldLabel::Duplicate))
        );
        assert_eq!(
            eval_label(None, Some("cached")),
            Some(Some(EstimateWithheldLabel::Duplicate))
        );
        assert_eq!(
            eval_label(None, Some("policy_mismatch")),
            Some(Some(EstimateWithheldLabel::Other))
        );
        assert_eq!(eval_label(None, None), None, "unscored is not labelled 0");
    }

    #[test]
    fn a_row_serializes_labels_only() {
        let row = rows(1, 1, true, 7).remove(0).row;
        let value = serde_json::to_value(&row).unwrap();
        let keys: BTreeSet<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            BTreeSet::from([
                "features",
                "credit_quality_micros",
                "credit_quality_calibration_version",
                "tenant_hash",
            ])
        );
    }

    #[test]
    fn a_predictive_corpus_passes_and_emits_three_valid_tiers() {
        let inputs = rows(600, 12, true, 1);
        let report = fit_estimate_table(&inputs);
        assert!(report.passed, "{report:#?}");
        assert!(report.failed_checks.is_empty(), "{report:#?}");
        assert_eq!(report.fit_rows, 420);
        assert_eq!(report.held_out_rows, 180);
        assert!(report.spearman.unwrap() >= ESTIMATE_MIN_SPEARMAN);
        assert!(
            report.spearman.unwrap() - report.baseline_spearman.unwrap()
                >= ESTIMATE_MIN_SPEARMAN_GAIN
        );
        let table = report.table.expect("a table");
        table.validate().expect("emitted tables validate");
        LocalEstimateTable::from_value(&serde_json::to_value(&table).unwrap())
            .expect("and round-trip through the client's parser");
        assert_eq!(table.tier_count(), 3);
        assert_eq!(table.credit_quality_calibration, "cq3");
        assert_eq!(table.version, format!("f{}", at(599).format("%Y%m%d")));
        // Higher tiers carry higher bands.
        assert!(table.bands[0].high <= table.bands[2].high);
        // The device places a large session above a small one.
        let small = estimate(&features(300, 2, 1), &table).unwrap();
        let large = estimate(&features(90_000, 2, 1), &table).unwrap();
        assert!(small.tier < large.tier, "{small:?} {large:?}");
    }

    #[test]
    fn an_unpredictive_corpus_fails_the_bar_and_emits_one_tier_on_the_held_out_band() {
        let inputs = rows(600, 12, false, 2);
        let report = fit_estimate_table(&inputs);
        assert!(!report.passed);
        assert!(report.failed_checks.contains(&"spearman"), "{report:#?}");
        let table = report.table.expect("a one-tier table");
        table.validate().unwrap();
        assert_eq!(table.tier_count(), 1);
        assert!(table.weights.is_empty() && table.cut_offs.is_empty());
        let mut held: Vec<f64> = inputs[420..]
            .iter()
            .map(|i| displayed_credit(i.row.credit_quality_micros.unwrap()))
            .collect();
        held.sort_by(f64::total_cmp);
        assert_eq!(table.bands[0].low, quantile(&held, 0.10));
        assert_eq!(table.bands[0].high, quantile(&held, 0.90));
        // One tier, so the device reports no tier at all.
        assert_eq!(estimate(&features(5_000, 3, 1), &table).unwrap().tier, None);
    }

    #[test]
    fn a_tier_under_the_tenant_floor_merges_into_its_neighbour() {
        let mut inputs = rows_shaped(600, 12, true, 3, true);
        // The top third of the fit window by size, and everything above it,
        // comes from two tenants: that tier fails the tenant floor.
        let mut sizes: Vec<u64> = inputs[..420]
            .iter()
            .map(|i| i.row.features.content_bytes)
            .collect();
        sizes.sort_unstable();
        let top = sizes[280];
        for (i, input) in inputs.iter_mut().enumerate() {
            if input.row.features.content_bytes >= top {
                input.row.tenant_hash = format!("sha256:big-{}", i % 2);
            }
        }
        let report = fit_estimate_table(&inputs);
        assert_eq!(report.tiers_before_merge, 3, "{report:#?}");
        assert_eq!(report.tiers, 2, "{report:#?}");
        if let Some(table) = &report.table {
            table.validate().unwrap();
            assert!(table.tier_count() <= 2);
        }
    }

    #[test]
    fn rows_under_another_calibration_do_not_train() {
        let mut inputs = rows(600, 12, true, 4);
        for input in &mut inputs {
            input.row.credit_quality_calibration_version = Some(2);
        }
        let report = fit_estimate_table(&inputs);
        assert_eq!(report.fit_rows, 0);
        assert_eq!(report.no_table_reason, Some("no_labelled_rows"));
        assert!(report.table.is_none());
    }

    #[test]
    fn withheld_rows_feed_the_share_and_never_the_band() {
        let mut inputs = rows(600, 12, true, 5);
        // Every fifth row is a duplicate with no credit quality.
        for (i, input) in inputs.iter_mut().enumerate() {
            if i % 5 == 0 {
                input.row.credit_quality_micros = None;
                input.row.credit_quality_calibration_version = None;
                input.row.withheld = Some(EstimateWithheldLabel::Duplicate);
            }
        }
        let report = fit_estimate_table(&inputs);
        assert_eq!(report.fit_rows + report.fit_withheld_rows, 420);
        assert_eq!(report.fit_withheld_rows, 84);
        let share = report.withheld_share.unwrap();
        assert!((share - 0.2).abs() < 1e-12, "{share}");
        let table = report.table.expect("a table");
        assert_eq!(table.withheld_share, Some(share));
        assert!(table.bands.iter().all(|band| band.low > 0.0));
    }

    #[test]
    fn too_few_held_out_rows_gives_no_table_rather_than_a_guess() {
        let report = fit_estimate_table(&rows(40, 12, true, 6));
        assert_eq!(report.no_table_reason, Some("insufficient_held_out_rows"));
        assert!(report.table.is_none());
        assert!(!report.passed);
    }

    #[test]
    fn a_fallback_below_the_tenant_floor_gives_no_table() {
        let report = fit_estimate_table(&rows(600, 3, false, 8));
        assert!(!report.passed);
        assert_eq!(report.no_table_reason, Some("below_non_personal_floor"));
        assert!(report.table.is_none());
    }

    #[test]
    fn the_fit_is_deterministic() {
        let inputs = rows(600, 12, true, 9);
        assert_eq!(fit_estimate_table(&inputs), fit_estimate_table(&inputs));
    }

    #[test]
    fn spearman_handles_ties_and_constant_input() {
        assert_eq!(spearman(&[1.0, 1.0, 1.0], &[1.0, 2.0, 3.0]), None);
        let rho = spearman(&[1.0, 2.0, 2.0, 4.0], &[10.0, 20.0, 20.0, 40.0]).unwrap();
        assert!((rho - 1.0).abs() < 1e-12);
        let rho = spearman(&[1.0, 2.0, 3.0], &[3.0, 2.0, 1.0]).unwrap();
        assert!((rho + 1.0).abs() < 1e-12);
    }
}
