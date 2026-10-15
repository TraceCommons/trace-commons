//! The device-side check of the local credit estimate (nudge value
//! addendum, 4.3 step 5). Dev-only, behind a hidden CLI subcommand.
//!
//! It computes `lef1` features from local sessions that have a scored
//! history row, estimates each under a table, and reports how often the
//! credit the server actually gave fell inside the estimated band. It
//! measures what the server's own eval cannot see: the gap between the raw
//! transcript this machine estimates from and the redacted envelope the
//! server scored.
//!
//! It sends nothing. This module makes no network request and holds no
//! client; its inputs are the local history cache, the local sessions and,
//! optionally, a table file. Its output is aggregate counts only: no
//! session, path, project or per-row figure.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Serialize;
use trace_commons_protocol::local_credit_estimate::{
    LocalEstimateFeatures, LocalEstimateTable, estimate, estimate_score,
};

use crate::config::ConfigStore;
use crate::daemon::history::{HistoryCache, HistoryRecord};

/// DRAFT, NEEDS APPROVAL. Heading of the check's human-readable output.
pub const CHECK_HEADING: &str = "Local credit estimate check (nothing was sent)";
/// DRAFT, NEEDS APPROVAL. `{scored}` scored history rows, `{joined}` found
/// on this machine with the same content.
pub const CHECK_JOINED: &str =
    "Scored sessions in history: {scored}; found unchanged on this machine: {joined}";
/// DRAFT, NEEDS APPROVAL. `{withheld}` rows displayed as 0 (repeats and
/// holds), which the estimate's band leaves out.
pub const CHECK_WITHHELD: &str = "Given 0 credit (left out of the band): {withheld}";
/// DRAFT, NEEDS APPROVAL. `{inside}` of `{estimated}` inside the estimate.
pub const CHECK_COVERAGE: &str =
    "Credit inside the estimate's range: {inside} of {estimated} ({share})";
/// DRAFT, NEEDS APPROVAL. Stands in for a share with nothing to divide.
pub const CHECK_NOT_MEASURED: &str = "not measured";
/// DRAFT, NEEDS APPROVAL. Rank agreement between the estimate's score and
/// the credit given.
pub const CHECK_SPEARMAN: &str = "Rank agreement with credit given (Spearman): {rho}";
/// DRAFT, NEEDS APPROVAL. Shown when rank agreement cannot be measured.
pub const CHECK_SPEARMAN_UNAVAILABLE: &str =
    "Rank agreement: not measured (the table has one tier, or too few sessions)";
/// DRAFT, NEEDS APPROVAL. Names the table the check used.
pub const CHECK_TABLE: &str = "Estimate table: {calibration}";

/// Fewest estimated rows a rank correlation is reported over.
const MIN_SPEARMAN_ROWS: usize = 3;

/// The check's whole output: counts and two shares, nothing per session.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeviceCheckReport {
    /// The table's calibration label, such as `lef1.t1/cq3`.
    pub calibration: String,
    /// History rows with a final credit.
    pub scored: usize,
    /// Of those, rows whose session was found on this machine unchanged.
    pub joined: usize,
    /// Joined rows given 0, which the band excludes (OWNER DECISION E6).
    pub withheld: usize,
    /// Joined, non-withheld rows the table gave an estimate for.
    pub estimated: usize,
    /// Of those, rows whose credit fell inside the estimated band.
    pub inside_band: usize,
    /// `inside_band / estimated`; `None` with nothing estimated.
    pub band_coverage: Option<f64>,
    /// Spearman correlation between the table's score and the credit,
    /// over estimated rows; `None` for a one-tier table or too few rows.
    pub spearman: Option<f64>,
}

/// The credit a history row was finally given, when it has one.
#[must_use]
pub fn final_credit(record: &HistoryRecord) -> Option<f64> {
    record
        .credit_points_final
        .map(f64::from)
        .filter(|credit| credit.is_finite())
}

/// Aggregate agreement over `(features, final credit)` pairs. `scored` is
/// the number of scored history rows the pairs were joined from.
#[must_use]
pub fn agreement(
    scored: usize,
    rows: &[(LocalEstimateFeatures, f64)],
    table: &LocalEstimateTable,
) -> DeviceCheckReport {
    let mut withheld = 0;
    let mut estimated = 0;
    let mut inside_band = 0;
    let mut ranked: Vec<(f64, f64)> = Vec::new();
    for (features, credit) in rows {
        // A 0 is the duplicate and hold paths, which the band leaves out
        // (OWNER DECISION E6); counted, never scored as a miss.
        if *credit <= 0.0 {
            withheld += 1;
            continue;
        }
        let Some(band) = estimate(features, table) else {
            continue;
        };
        estimated += 1;
        if (band.low..=band.high).contains(credit) {
            inside_band += 1;
        }
        if table.tier_count() > 1 {
            if let Some(score) = estimate_score(features, table) {
                ranked.push((score, *credit));
            }
        }
    }
    DeviceCheckReport {
        calibration: table.calibration_label(),
        scored,
        joined: rows.len(),
        withheld,
        estimated,
        inside_band,
        band_coverage: (estimated > 0).then(|| inside_band as f64 / estimated as f64),
        spearman: (ranked.len() >= MIN_SPEARMAN_ROWS)
            .then(|| spearman(&ranked))
            .flatten(),
    }
}

/// Spearman's rank correlation, ties given their average rank; `None` when
/// either side has no spread.
fn spearman(pairs: &[(f64, f64)]) -> Option<f64> {
    let xs = ranks(&pairs.iter().map(|p| p.0).collect::<Vec<_>>());
    let ys = ranks(&pairs.iter().map(|p| p.1).collect::<Vec<_>>());
    let n = xs.len() as f64;
    let mean = (n + 1.0) / 2.0;
    let (mut cov, mut vx, mut vy) = (0.0, 0.0, 0.0);
    for (x, y) in xs.iter().zip(&ys) {
        cov += (x - mean) * (y - mean);
        vx += (x - mean).powi(2);
        vy += (y - mean).powi(2);
    }
    (vx > 0.0 && vy > 0.0).then(|| cov / (vx * vy).sqrt())
}

fn ranks(values: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut out = vec![0.0; values.len()];
    let mut i = 0;
    while i < order.len() {
        let mut j = i;
        while j + 1 < order.len() && values[order[j + 1]] == values[order[i]] {
            j += 1;
        }
        let rank = (i + j) as f64 / 2.0 + 1.0;
        for &index in &order[i..=j] {
            out[index] = rank;
        }
        i = j + 1;
    }
    out
}

/// The table at `path`, accepted only as the daemon would accept a fetched
/// one, or the built-in table.
pub fn table_from(path: Option<&Path>) -> Result<LocalEstimateTable> {
    let Some(path) = path else {
        return Ok(LocalEstimateTable::built_in());
    };
    let raw = std::fs::read(path).context("reading the estimate table file")?;
    let value: serde_json::Value =
        serde_json::from_slice(&raw).context("the estimate table file is not JSON")?;
    LocalEstimateTable::from_value(&value).context("the estimate table was refused")
}

/// Run the check over this machine's history cache and sessions.
pub fn run(store: &ConfigStore, table: &LocalEstimateTable) -> Result<DeviceCheckReport> {
    let history = HistoryCache::load(store)?;
    let mut credit_by_hash: HashMap<String, f64> = HashMap::new();
    for record in &history {
        if let Some(credit) = final_credit(record) {
            credit_by_hash.insert(record.session_hash.clone(), credit);
        }
    }
    let scored = credit_by_hash.len();
    let mut rows = Vec::new();
    if !credit_by_hash.is_empty() {
        let roots = crate::source::cli_source_roots(None);
        for source in crate::source::all_sources(&roots) {
            let Ok(refs) = source.discover() else {
                continue;
            };
            for session_ref in refs {
                let Ok(transcript) = source.load(&session_ref) else {
                    continue;
                };
                // Joined by content hash, so a session that changed since
                // it was scored is not compared against a score of other
                // bytes.
                if let Some(credit) = credit_by_hash.remove(&transcript.session_hash) {
                    rows.push((
                        crate::daemon::queue::estimate_features_of(&transcript),
                        credit,
                    ));
                }
            }
        }
    }
    Ok(agreement(scored, &rows, table))
}

/// The human-readable lines for `report`.
#[must_use]
pub fn render(report: &DeviceCheckReport) -> Vec<String> {
    let mut lines = vec![
        CHECK_HEADING.to_string(),
        CHECK_TABLE.replace("{calibration}", &report.calibration),
        CHECK_JOINED
            .replace("{scored}", &report.scored.to_string())
            .replace("{joined}", &report.joined.to_string()),
        CHECK_WITHHELD.replace("{withheld}", &report.withheld.to_string()),
    ];
    let share = report.band_coverage.map_or_else(
        || CHECK_NOT_MEASURED.to_string(),
        |s| format!("{:.0}%", s * 100.0),
    );
    lines.push(
        CHECK_COVERAGE
            .replace("{inside}", &report.inside_band.to_string())
            .replace("{estimated}", &report.estimated.to_string())
            .replace("{share}", &share),
    );
    lines.push(match report.spearman {
        Some(rho) => CHECK_SPEARMAN.replace("{rho}", &format!("{rho:.2}")),
        None => CHECK_SPEARMAN_UNAVAILABLE.to_string(),
    });
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn features(text: &str, user_messages: usize) -> LocalEstimateFeatures {
        crate::daemon::queue::estimate_features_of(&crate::source::SessionTranscript {
            events: (0..user_messages.max(1))
                .map(|_| crate::source::SessionEvent {
                    kind: crate::source::SessionEventKind::User,
                    content: Some(text.to_string()),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
    }

    fn three_tier() -> LocalEstimateTable {
        LocalEstimateTable::from_value(&json!({
            "schema_version": 1,
            "features_version": "lef1",
            "version": "t7",
            "credit_quality_calibration": "cq3",
            "bytes_per_token": 4,
            "chunk_target_tokens": 2048,
            "chunk_cap": 16,
            "weights": [{"term": "ln_content_bytes", "weight": 1.0}],
            "cut_offs": [4.0, 6.0],
            "bands": [
                {"low": 0.9, "high": 1.9},
                {"low": 1.3, "high": 2.4},
                {"low": 1.8, "high": 3.1}
            ]
        }))
        .unwrap()
    }

    /// Under the built-in table (band shown as 1.0 to 3.0), credit inside
    /// the band counts, credit outside does not, a 0 is withheld and left
    /// out, and rank agreement is not measured for one tier.
    #[test]
    fn coverage_counts_credit_inside_the_band_and_leaves_zeros_out() {
        let table = LocalEstimateTable::built_in();
        let rows = vec![
            (features("fix the parser", 1), 1.0),
            (features("fix the parser", 1), 2.9),
            (features("fix the parser", 1), 3.0),
            (features("fix the parser", 1), 4.0),
            (features("fix the parser", 1), 0.0),
        ];
        let report = agreement(7, &rows, &table);
        assert_eq!(report.calibration, "lef1.t1/cq3");
        assert_eq!(report.scored, 7);
        assert_eq!(report.joined, 5);
        assert_eq!(report.withheld, 1);
        assert_eq!(report.estimated, 4);
        assert_eq!(report.inside_band, 3);
        assert_eq!(report.band_coverage, Some(0.75));
        assert_eq!(report.spearman, None);
    }

    /// A session with no content has no estimate; it is not counted as a
    /// miss, and nothing estimated gives no coverage rather than 0.
    #[test]
    fn nothing_estimated_reports_no_coverage_not_zero() {
        let report = agreement(
            1,
            &[(features("", 1), 2.0)],
            &LocalEstimateTable::built_in(),
        );
        assert_eq!(report.joined, 1);
        assert_eq!(report.estimated, 0);
        assert_eq!(report.band_coverage, None);
    }

    /// With a weighted table, rank agreement is measured: credit rising
    /// with content length is a perfect +1, falling a perfect -1.
    #[test]
    fn rank_agreement_is_measured_for_a_weighted_table() {
        let table = three_tier();
        let rising: Vec<_> = [("a", 1.0), ("bbbbbbbbbb", 2.0), (&"c".repeat(500)[..], 3.0)]
            .iter()
            .map(|(text, credit)| (features(text, 1), *credit))
            .collect();
        let report = agreement(3, &rising, &table);
        assert!((report.spearman.unwrap() - 1.0).abs() < 1e-9, "{report:?}");
        let falling: Vec<_> = rising.iter().map(|(f, c)| (f.clone(), 4.0 - c)).collect();
        let report = agreement(3, &falling, &table);
        assert!((report.spearman.unwrap() + 1.0).abs() < 1e-9, "{report:?}");
        // Too few rows: not measured.
        let report = agreement(2, &rising[..2], &table);
        assert_eq!(report.spearman, None);
    }

    #[test]
    fn a_refused_table_file_is_an_error_and_none_is_the_built_in_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("table.json");
        std::fs::write(&path, br#"{"schema_version": 2}"#).unwrap();
        assert!(table_from(Some(&path)).is_err());
        assert_eq!(table_from(None).unwrap(), LocalEstimateTable::built_in());
    }

    /// The output is counts and shares only.
    #[test]
    fn the_rendered_lines_carry_counts_only() {
        let report = agreement(
            2,
            &[(features("fix the parser at /Users/alice/code", 1), 2.0)],
            &LocalEstimateTable::built_in(),
        );
        let lines = render(&report);
        let text = lines.join("\n");
        assert!(!text.contains("alice"), "{text}");
        assert!(text.contains("1 of 1 (100%)"), "{text}");
        let empty = render(&agreement(0, &[], &LocalEstimateTable::built_in())).join("\n");
        assert!(empty.contains("0 of 0 (not measured)"), "{empty}");
        for line in [
            CHECK_HEADING,
            CHECK_JOINED,
            CHECK_WITHHELD,
            CHECK_COVERAGE,
            CHECK_SPEARMAN,
            CHECK_SPEARMAN_UNAVAILABLE,
            CHECK_TABLE,
            CHECK_NOT_MEASURED,
        ] {
            assert!(line.len() <= 100, "{line}");
        }
    }

    /// It sends nothing: this module names no HTTP client.
    #[test]
    fn the_check_sends_nothing() {
        let source = include_str!("credit_estimate_check.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        for word in [
            concat!("req", "west"),
            concat!("Http", "Client"),
            concat!("EstimateTable", "Client"),
            concat!("operator", "_client"),
            concat!("Tcp", "Stream"),
        ] {
            assert!(!production.contains(word), "names {word}");
        }
    }
}
