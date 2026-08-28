// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The trivial-measure battery: a preregistered set of no-model scores that a
//! valid bake-off corpus must *fail* to be separated by (#204, #205).
//!
//! Background. The A2.6 corpus built its novel slice from agent traces and
//! reused its duplicate slice verbatim from a Wikipedia corpus, so class label
//! and source format were entangled by construction. Paragraph count scored
//! AUC 1.000 on it — every one of the 300 duplicate files had exactly one
//! paragraph — while the selected model scored 0.936. Six trivial measures
//! beat the model that the corpus was used to select, and nothing in the
//! repository would have said so; the defect was found by hand months later.
//!
//! What this module is for. A corpus is admissible only if every measure below
//! lands near AUC 0.5. Any structural measure that classifies the corpus is a
//! defect in the corpus, not a finding about the data. The AUC is the
//! repository's own [`discrimination_auc`] — ties 0.5 each under the
//! Mann–Whitney U convention, empty inputs 0.5 — so the battery's numbers are
//! directly comparable to the archived bake-off AUCs they are meant to police.
//!
//! What it is not. Passing the battery is necessary, not sufficient. It says
//! that six named structural properties do not separate the classes; it says
//! nothing about the seventh that nobody wrote down. Treat it as a floor.

use serde::{Deserialize, Serialize};

use super::bakeoff_metrics::discrimination_auc;

/// Maximum tolerated distance from AUC 0.5 for any single measure.
///
/// A measure at 0.65 already classifies two thirds of random pairs correctly
/// on structure alone, which is more separation than the deployed floor gets
/// out of the model on a source-controlled slice (#205 puts that at 0.757 AUC
/// before any threshold is applied). The value is a ceiling on an obvious
/// defect, not a claim that 0.64 is fine.
pub const DEFAULT_CEILING: f64 = 0.15;

/// One no-model score over a single trace body.
pub struct TrivialMeasure {
    pub name: &'static str,
    pub f: fn(&str) -> f64,
}

/// The preregistered battery, in the order #204 tabulates it. Preregistered
/// means fixed before a corpus is built: adding a measure after seeing a
/// corpus's numbers, or dropping one that fails, defeats the point.
pub const TRIVIAL_MEASURES: [TrivialMeasure; 6] = [
    TrivialMeasure {
        name: "paragraph_count",
        f: paragraph_count,
    },
    TrivialMeasure {
        name: "line_count",
        f: line_count,
    },
    TrivialMeasure {
        name: "distinct_word_count",
        f: distinct_word_count,
    },
    TrivialMeasure {
        name: "utf8_byte_count",
        f: utf8_byte_count,
    },
    TrivialMeasure {
        name: "whitespace_word_count",
        f: whitespace_word_count,
    },
    TrivialMeasure {
        name: "mean_word_length",
        f: mean_word_length,
    },
];

/// The measure used as the length covariate in every report. #199's guardrail:
/// if a score's AUC matches this one's to two decimals, the score is measuring
/// length.
pub const LENGTH_COVARIATE: &str = "utf8_byte_count";

/// Blank-line-separated blocks with any content. This is the measure that
/// scored 1.000 on the A2.6 corpus.
pub fn paragraph_count(text: &str) -> f64 {
    text.split("\n\n").filter(|b| !b.trim().is_empty()).count() as f64
}

/// Newline-separated lines. A trailing newline does not add a line.
pub fn line_count(text: &str) -> f64 {
    text.lines().count() as f64
}

/// Distinct whitespace-separated tokens, case-sensitive and unnormalised.
pub fn distinct_word_count(text: &str) -> f64 {
    text.split_whitespace()
        .collect::<std::collections::BTreeSet<_>>()
        .len() as f64
}

/// Encoded size in bytes.
pub fn utf8_byte_count(text: &str) -> f64 {
    text.len() as f64
}

/// Whitespace-separated tokens.
pub fn whitespace_word_count(text: &str) -> f64 {
    text.split_whitespace().count() as f64
}

/// Mean token length in characters. Zero when there are no tokens — a body
/// with nothing in it has no mean word length, and 0.0 keeps it at the bottom
/// of the ordering rather than inventing a value.
pub fn mean_word_length(text: &str) -> f64 {
    let mut tokens = 0usize;
    let mut chars = 0usize;
    for w in text.split_whitespace() {
        tokens += 1;
        chars += w.chars().count();
    }
    if tokens == 0 {
        return 0.0;
    }
    chars as f64 / tokens as f64
}

/// One measure's verdict on one slice pair. Ranges are carried because a
/// disjoint support — as with the A2.6 paragraph counts, novel 7..163 against
/// duplicate 1..1 — is the thing that makes the confound unadjustable, and an
/// AUC alone does not show it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasureAuc {
    pub measure: String,
    pub auc: f64,
    /// Distance from 0.5. Direction-free: a measure that separates the classes
    /// backwards separates them just as well.
    pub abs_deviation: f64,
    pub novel_min: f64,
    pub novel_max: f64,
    pub duplicate_min: f64,
    pub duplicate_max: f64,
    /// True when the two slices' observed ranges do not overlap at all. There
    /// is then no stratum in which to hold the measure constant, so the
    /// confound cannot be adjusted away by any later analysis.
    pub support_disjoint: bool,
}

/// The battery's verdict on one slice pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryOutcome {
    pub pair: String,
    pub novel_count: usize,
    pub duplicate_count: usize,
    pub ceiling: f64,
    pub measures: Vec<MeasureAuc>,
    /// AUC of [`LENGTH_COVARIATE`], repeated at the top level so every report
    /// carries it beside whatever else it reports.
    pub length_covariate: String,
    pub length_covariate_auc: f64,
    pub worst_measure: Option<String>,
    pub worst_abs_deviation: f64,
    pub admissible: bool,
}

/// Run the battery over one labelled slice pair.
///
/// `ceiling` is the maximum tolerated `|auc - 0.5|`. Empty slices are never
/// admissible: `discrimination_auc` returns 0.5 for them, which is the right
/// answer to "which class is higher" and the wrong answer to "is this a
/// corpus".
pub fn run_battery(
    pair: &str,
    novel: &[String],
    duplicate: &[String],
    ceiling: f64,
) -> BatteryOutcome {
    let mut measures = Vec::with_capacity(TRIVIAL_MEASURES.len());
    for m in TRIVIAL_MEASURES.iter() {
        let n: Vec<f64> = novel.iter().map(|t| (m.f)(t)).collect();
        let d: Vec<f64> = duplicate.iter().map(|t| (m.f)(t)).collect();
        let auc = discrimination_auc(&n, &d);
        let (novel_min, novel_max) = min_max(&n);
        let (duplicate_min, duplicate_max) = min_max(&d);
        let support_disjoint = !n.is_empty()
            && !d.is_empty()
            && (novel_min > duplicate_max || duplicate_min > novel_max);
        measures.push(MeasureAuc {
            measure: m.name.to_string(),
            auc,
            abs_deviation: (auc - 0.5).abs(),
            novel_min,
            novel_max,
            duplicate_min,
            duplicate_max,
            support_disjoint,
        });
    }

    let length_covariate_auc = measures
        .iter()
        .find(|m| m.measure == LENGTH_COVARIATE)
        .map(|m| m.auc)
        .unwrap_or(0.5);

    let worst = measures
        .iter()
        .fold(None::<&MeasureAuc>, |acc, m| match acc {
            Some(prev) if prev.abs_deviation >= m.abs_deviation => Some(prev),
            _ => Some(m),
        });
    let worst_abs_deviation = worst.map(|m| m.abs_deviation).unwrap_or(0.0);
    let worst_measure = worst.map(|m| m.measure.clone());

    let populated = !novel.is_empty() && !duplicate.is_empty();
    let admissible = populated && worst_abs_deviation <= ceiling;

    BatteryOutcome {
        pair: pair.to_string(),
        novel_count: novel.len(),
        duplicate_count: duplicate.len(),
        ceiling,
        measures,
        length_covariate: LENGTH_COVARIATE.to_string(),
        length_covariate_auc,
        worst_measure,
        worst_abs_deviation,
        admissible,
    }
}

fn min_max(values: &[f64]) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0);
    }
    let mut lo = values[0];
    let mut hi = values[0];
    for &v in &values[1..] {
        if v < lo {
            lo = v;
        }
        if v > hi {
            hi = v;
        }
    }
    (lo, hi)
}

/// Render one outcome as a fixed-width table for operator eyes. Body text is
/// never echoed — only counts, ranges and AUCs.
pub fn render_outcome(outcome: &BatteryOutcome) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "pair={} novel={} duplicate={} ceiling=|auc-0.5|<={:.3}",
        outcome.pair, outcome.novel_count, outcome.duplicate_count, outcome.ceiling
    );
    let _ = writeln!(
        s,
        "{:<22} {:>9} {:>9}  {:<22} {:<22} support",
        "measure", "auc", "|dev|", "novel range", "duplicate range"
    );
    for m in &outcome.measures {
        let _ = writeln!(
            s,
            "{:<22} {:>9.6} {:>9.6}  {:<22} {:<22} {}",
            m.measure,
            m.auc,
            m.abs_deviation,
            format!("{:.4}..{:.4}", m.novel_min, m.novel_max),
            format!("{:.4}..{:.4}", m.duplicate_min, m.duplicate_max),
            if m.support_disjoint {
                "DISJOINT"
            } else {
                "overlapping"
            }
        );
    }
    let _ = writeln!(
        s,
        "length covariate ({}) auc={:.6}",
        outcome.length_covariate, outcome.length_covariate_auc
    );
    let _ = writeln!(
        s,
        "verdict={} worst={} |dev|={:.6}",
        if outcome.admissible {
            "ADMISSIBLE"
        } else {
            "INADMISSIBLE"
        },
        outcome.worst_measure.as_deref().unwrap_or("none"),
        outcome.worst_abs_deviation
    );
    s
}
