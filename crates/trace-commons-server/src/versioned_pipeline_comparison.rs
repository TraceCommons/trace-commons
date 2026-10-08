// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Decision records of the pipeline comparison and the function that
//! compares two of them.
//!
//! `pipeline.py compare` sends the same traces through the old gate path
//! (side `baseline`) and through the versioned pipeline (side `candidate`).
//! Each side gives one [`ComparisonRecord`] for a trace. [`compare_records`]
//! compares the two records field by field and names each field that differs.
//!
//! A record holds only numbers, enumerations, hashes, and labels. This module
//! has no database, no HTTP, and no clock.

use serde::{Deserialize, Serialize};

pub const COMPARISON_REPORT_SCHEMA: &str = "trace_commons.pipeline_comparison_report.v1";
pub const COMPARE_CORPUS_SCHEMA: &str = "trace_commons.pipeline_compare_corpus.v1";
pub const UNEXPLAINED_LIST_LIMIT: usize = 1_000;
pub const COMPARISON_BLOCKERS: [&str; 8] = [
    "local_test_only",
    "local_reference_scorer",
    "local_reference_embedder",
    "synthetic_index",
    "synthetic_settlement",
    "static_bearer_authentication",
    "deterministic_privacy_only",
    "baseline_derived_scan_removed",
];
pub const COMPARISON_UNEXPLAINED_LABEL: &str = "comparison_has_unexplained_differences";
pub const COMPARISON_BRANCH_LABEL: &str = "comparison_gate_branch_not_exercised";
pub const COMPARISON_REFUSED_LABEL: &str = "comparison_receipt_refused";
pub const COMPARISON_ALIGNMENT_LABEL: &str = "comparison_alignment_lost";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonSide {
    Baseline,
    Candidate,
}

/// `Refused`: the receipt was not accepted. `Other`: a state that is none of
/// the three decisions (for example `awaiting_pii_backstop`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionLabel {
    Admit,
    Quarantine,
    Reject,
    Refused,
    Other,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewLabel {
    None,
    Approve,
    Reject,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSource {
    None,
    HashRule,
    Alignment,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GateValues {
    pub quality_passed: bool,
    pub novelty_passed: bool,
    pub perplexity_micros: u64,
    pub tail_fraction_micros: u64,
    pub peak_perplexity_micros: u64,
    pub novelty_score_micros: u64,
    pub peak_novelty_micros: u64,
    pub chunk_count: u32,
    pub total_chunk_count: u32,
    pub chunks_capped: bool,
    pub index_cardinality: Option<u64>,
    pub credit_quality_micros: Option<i64>,
    /// The column is INT4 and the evidence is i32: read as i32, then widen.
    pub credit_quality_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreditEvent {
    pub event_type: String,
    pub microcredits: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonRecord {
    pub position: u64,
    /// "bootstrap" or "holdout".
    pub partition: String,
    /// `sha256:` of the trace id text.
    pub trace_hash: String,
    pub side: ComparisonSide,
    pub receipt_code: u16,
    /// The side reached a terminal state in time.
    pub terminal: bool,
    /// "low", "medium", or "high".
    pub privacy_risk: Option<String>,
    /// Sorted labels.
    pub privacy_basis: Vec<String>,
    pub admission: AdmissionLabel,
    pub review: ReviewLabel,
    pub review_source: ReviewSource,
    pub gate_skipped_by_alignment: bool,
    pub scored: bool,
    /// `Some` exactly when `scored`.
    pub gate: Option<GateValues>,
    pub member: bool,
    /// Ascending chunk numbers.
    pub member_chunks: Vec<u32>,
    pub credit_events: Vec<CreditEvent>,
}

/// The exclusions of spec section 10.2. No variant permits a difference in a
/// compared field (PC-D12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ComparisonRule {
    DeterministicIndexKeys,
    LedgerReasonText,
    ShadowValuesNotInContract,
}

impl ComparisonRule {
    pub const ALL: [ComparisonRule; 3] = [
        ComparisonRule::DeterministicIndexKeys,
        ComparisonRule::LedgerReasonText,
        ComparisonRule::ShadowValuesNotInContract,
    ];

    pub fn id(self) -> &'static str {
        match self {
            ComparisonRule::DeterministicIndexKeys => "deterministic_index_keys",
            ComparisonRule::LedgerReasonText => "ledger_reason_text",
            ComparisonRule::ShadowValuesNotInContract => "shadow_values_not_in_contract",
        }
    }

    pub fn source(self) -> &'static str {
        match self {
            ComparisonRule::DeterministicIndexKeys => {
                "compatibility_mapping.required_behavior_change_2"
            }
            ComparisonRule::LedgerReasonText => "ruling.T15-2",
            ComparisonRule::ShadowValuesNotInContract => {
                "compatibility_mapping.membership_and_credit_rules"
            }
        }
    }

    pub fn excluded_fields(self) -> &'static [&'static str] {
        match self {
            ComparisonRule::DeterministicIndexKeys => &["index_entry_id", "nearest_neighbor_hash"],
            ComparisonRule::LedgerReasonText => &["ledger_reason"],
            ComparisonRule::ShadowValuesNotInContract => {
                &["dedup_penalty", "contributor_cap", "anomaly_withheld"]
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceComparison {
    Equal,
    Permitted { rules: Vec<&'static str> },
    Unexplained { fields: Vec<&'static str> },
}

/// The production comparison: no rule permits a difference.
pub fn compare_records(
    baseline: &ComparisonRecord,
    candidate: &ComparisonRecord,
) -> TraceComparison {
    compare_records_with(baseline, candidate, &|_, _, _| None)
}

fn record_pair_refused() -> TraceComparison {
    TraceComparison::Unexplained {
        fields: vec!["record_pair"],
    }
}

/// `permit` answers the rule id that permits a difference in `field`, or `None`.
pub fn compare_records_with(
    baseline: &ComparisonRecord,
    candidate: &ComparisonRecord,
    permit: &dyn Fn(&'static str, &ComparisonRecord, &ComparisonRecord) -> Option<&'static str>,
) -> TraceComparison {
    if baseline.position != candidate.position
        || baseline.partition != candidate.partition
        || baseline.trace_hash != candidate.trace_hash
        || baseline.side != ComparisonSide::Baseline
        || candidate.side != ComparisonSide::Candidate
        || baseline.scored != baseline.gate.is_some()
        || candidate.scored != candidate.gate.is_some()
    {
        return record_pair_refused();
    }
    if baseline.receipt_code != candidate.receipt_code {
        return TraceComparison::Unexplained {
            fields: vec!["receipt_code"],
        };
    }

    // A pair in which a side did not finish is never equal, also when the
    // two values of `terminal` are equal.
    let mut differing: Vec<&'static str> = Vec::new();
    if !(baseline.terminal && candidate.terminal) {
        differing.push("terminal");
    }
    for (name, differs) in [
        (
            "privacy_risk",
            baseline.privacy_risk != candidate.privacy_risk,
        ),
        (
            "privacy_basis",
            baseline.privacy_basis != candidate.privacy_basis,
        ),
        ("admission", baseline.admission != candidate.admission),
        ("scored", baseline.scored != candidate.scored),
    ] {
        if differs {
            differing.push(name);
        }
    }
    // When `scored` differs, the gate values of one side do not exist: they
    // are not listed again.
    if let (Some(b), Some(c)) = (&baseline.gate, &candidate.gate) {
        for (name, differs) in [
            ("quality_passed", b.quality_passed != c.quality_passed),
            ("novelty_passed", b.novelty_passed != c.novelty_passed),
            (
                "perplexity_micros",
                b.perplexity_micros != c.perplexity_micros,
            ),
            (
                "tail_fraction_micros",
                b.tail_fraction_micros != c.tail_fraction_micros,
            ),
            (
                "peak_perplexity_micros",
                b.peak_perplexity_micros != c.peak_perplexity_micros,
            ),
            (
                "novelty_score_micros",
                b.novelty_score_micros != c.novelty_score_micros,
            ),
            (
                "peak_novelty_micros",
                b.peak_novelty_micros != c.peak_novelty_micros,
            ),
            ("chunk_count", b.chunk_count != c.chunk_count),
            (
                "total_chunk_count",
                b.total_chunk_count != c.total_chunk_count,
            ),
            ("chunks_capped", b.chunks_capped != c.chunks_capped),
            (
                "index_cardinality",
                b.index_cardinality != c.index_cardinality,
            ),
            (
                "credit_quality_micros",
                b.credit_quality_micros != c.credit_quality_micros,
            ),
            (
                "credit_quality_version",
                b.credit_quality_version != c.credit_quality_version,
            ),
        ] {
            if differs {
                differing.push(name);
            }
        }
    }
    for (name, differs) in [
        ("member", baseline.member != candidate.member),
        (
            "member_chunks",
            baseline.member_chunks != candidate.member_chunks,
        ),
        (
            "credit_events",
            baseline.credit_events != candidate.credit_events,
        ),
    ] {
        if differs {
            differing.push(name);
        }
    }

    let mut rules: Vec<&'static str> = Vec::new();
    let mut fields: Vec<&'static str> = Vec::new();
    for name in differing {
        match permit(name, baseline, candidate) {
            Some(rule) => rules.push(rule),
            None => fields.push(name),
        }
    }
    rules.sort_unstable();
    rules.dedup();
    if !fields.is_empty() {
        TraceComparison::Unexplained { fields }
    } else if !rules.is_empty() {
        TraceComparison::Permitted { rules }
    } else {
        TraceComparison::Equal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versioned_pipeline_qualification::is_safe_label;
    use trace_commons_protocol::canonical_json::to_canonical_vec;

    const COMPARED: [&str; 22] = [
        "receipt_code",
        "terminal",
        "privacy_risk",
        "privacy_basis",
        "admission",
        "scored",
        "quality_passed",
        "novelty_passed",
        "perplexity_micros",
        "tail_fraction_micros",
        "peak_perplexity_micros",
        "novelty_score_micros",
        "peak_novelty_micros",
        "chunk_count",
        "total_chunk_count",
        "chunks_capped",
        "index_cardinality",
        "credit_quality_micros",
        "credit_quality_version",
        "member",
        "member_chunks",
        "credit_events",
    ];

    fn record(side: ComparisonSide) -> ComparisonRecord {
        ComparisonRecord {
            position: 7,
            partition: "holdout".into(),
            trace_hash: format!("sha256:{}", "a".repeat(64)),
            side,
            receipt_code: 200,
            terminal: true,
            privacy_risk: Some("medium".into()),
            privacy_basis: vec!["consent_content_flag".into()],
            admission: AdmissionLabel::Admit,
            review: ReviewLabel::None,
            review_source: ReviewSource::None,
            gate_skipped_by_alignment: false,
            scored: true,
            gate: Some(GateValues {
                quality_passed: true,
                novelty_passed: true,
                perplexity_micros: 31_000_000,
                tail_fraction_micros: 120_000,
                peak_perplexity_micros: 40_000_000,
                novelty_score_micros: 600_000,
                peak_novelty_micros: 900_000,
                chunk_count: 3,
                total_chunk_count: 3,
                chunks_capped: false,
                index_cardinality: Some(12),
                credit_quality_micros: Some(450_000),
                credit_quality_version: Some(2),
            }),
            member: true,
            member_chunks: vec![0, 1, 2],
            credit_events: vec![CreditEvent {
                event_type: "novelty_utility".into(),
                microcredits: 2_500_000,
            }],
        }
    }

    fn pair() -> (ComparisonRecord, ComparisonRecord) {
        (
            record(ComparisonSide::Baseline),
            record(ComparisonSide::Candidate),
        )
    }

    fn gate(r: &mut ComparisonRecord) -> &mut GateValues {
        r.gate.as_mut().expect("the record is scored")
    }

    fn unexplained(fields: &[&'static str]) -> TraceComparison {
        TraceComparison::Unexplained {
            fields: fields.to_vec(),
        }
    }

    fn refuse(r: &mut ComparisonRecord) {
        r.receipt_code = 429;
        r.admission = AdmissionLabel::Refused;
        r.terminal = false;
        r.scored = false;
        r.gate = None;
    }

    fn is_wide_label(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
    }

    #[test]
    fn two_equal_records_compare_equal() {
        let (b, c) = pair();
        assert_eq!(compare_records(&b, &c), TraceComparison::Equal);
    }

    #[test]
    fn each_compared_field_is_reported_by_name() {
        type Change = fn(&mut ComparisonRecord);
        let table: Vec<(&str, Change)> = vec![
            ("receipt_code", |r| r.receipt_code = 202),
            ("terminal", |r| r.terminal = false),
            ("privacy_risk", |r| r.privacy_risk = Some("high".into())),
            ("privacy_basis", |r| {
                r.privacy_basis = vec!["other_label".into()]
            }),
            ("admission", |r| r.admission = AdmissionLabel::Quarantine),
            ("scored", |r| {
                r.scored = false;
                r.gate = None;
            }),
            ("quality_passed", |r| gate(r).quality_passed = false),
            ("novelty_passed", |r| gate(r).novelty_passed = false),
            ("perplexity_micros", |r| gate(r).perplexity_micros += 1),
            ("tail_fraction_micros", |r| {
                gate(r).tail_fraction_micros += 1
            }),
            ("peak_perplexity_micros", |r| {
                gate(r).peak_perplexity_micros += 1
            }),
            ("novelty_score_micros", |r| {
                gate(r).novelty_score_micros += 1
            }),
            ("peak_novelty_micros", |r| gate(r).peak_novelty_micros += 1),
            ("chunk_count", |r| gate(r).chunk_count += 1),
            ("total_chunk_count", |r| gate(r).total_chunk_count += 1),
            ("chunks_capped", |r| gate(r).chunks_capped = true),
            ("index_cardinality", |r| {
                gate(r).index_cardinality = Some(13)
            }),
            ("credit_quality_micros", |r| {
                gate(r).credit_quality_micros = Some(450_001)
            }),
            ("credit_quality_version", |r| {
                gate(r).credit_quality_version = Some(3)
            }),
            ("member", |r| r.member = false),
            ("member_chunks", |r| r.member_chunks = vec![0, 1]),
            ("credit_events", |r| r.credit_events.clear()),
        ];
        assert_eq!(table.len(), 22);
        let names: Vec<&str> = table.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, COMPARED.to_vec());
        for (name, change) in table {
            let (b, mut c) = pair();
            change(&mut c);
            assert_eq!(compare_records(&b, &c), unexplained(&[name]), "{name}");
        }
    }

    #[test]
    fn several_differences_are_reported_in_field_order() {
        let (b, mut c) = pair();
        gate(&mut c).novelty_score_micros += 1;
        c.admission = AdmissionLabel::Reject;
        assert_eq!(
            compare_records(&b, &c),
            unexplained(&["admission", "novelty_score_micros"])
        );
    }

    #[test]
    fn a_scored_record_against_an_unscored_record_reports_scored_only() {
        let (b, mut c) = pair();
        c.scored = false;
        c.gate = None;
        assert_eq!(compare_records(&b, &c), unexplained(&["scored"]));
    }

    #[test]
    fn two_unscored_records_are_equal() {
        let (mut b, mut c) = pair();
        for r in [&mut b, &mut c] {
            r.scored = false;
            r.gate = None;
            r.member = false;
            r.member_chunks = vec![];
            r.credit_events = vec![];
        }
        assert_eq!(compare_records(&b, &c), TraceComparison::Equal);
    }

    #[test]
    fn a_record_that_is_not_terminal_is_unexplained() {
        let (mut b, mut c) = pair();
        b.terminal = false;
        c.terminal = false;
        assert_eq!(compare_records(&b, &c), unexplained(&["terminal"]));
    }

    #[test]
    fn records_of_different_traces_are_refused() {
        let (b, mut c) = pair();
        c.position = 8;
        assert_eq!(compare_records(&b, &c), unexplained(&["record_pair"]));
        let (b, mut c) = pair();
        c.trace_hash = format!("sha256:{}", "b".repeat(64));
        assert_eq!(compare_records(&b, &c), unexplained(&["record_pair"]));
        let (b, mut c) = pair();
        c.partition = "bootstrap".into();
        assert_eq!(compare_records(&b, &c), unexplained(&["record_pair"]));
    }

    #[test]
    fn a_permit_function_gives_permitted() {
        let permit = |field: &'static str, _: &ComparisonRecord, _: &ComparisonRecord| {
            (field == "admission").then_some("test_rule")
        };
        let (b, mut c) = pair();
        c.admission = AdmissionLabel::Quarantine;
        assert_eq!(
            compare_records_with(&b, &c, &permit),
            TraceComparison::Permitted {
                rules: vec!["test_rule"]
            }
        );
        gate(&mut c).chunk_count += 1;
        assert_eq!(
            compare_records_with(&b, &c, &permit),
            unexplained(&["chunk_count"])
        );
    }

    #[test]
    fn rule_ids_and_sources_are_labels() {
        for rule in ComparisonRule::ALL {
            assert!(is_safe_label(rule.id()), "{}", rule.id());
            assert!(is_wide_label(rule.source()), "{}", rule.source());
            assert!(!rule.excluded_fields().is_empty());
            for field in rule.excluded_fields() {
                assert!(!COMPARED.contains(field), "{field}");
            }
        }
    }

    fn walk(value: &serde_json::Value) {
        const FORBIDDEN_KEYS: [&str; 8] = [
            "input",
            "text",
            "trace_text",
            "secret",
            "secret_probe",
            "token",
            "account_id",
            "email",
        ];
        match value {
            serde_json::Value::String(s) => {
                assert!(is_wide_label(s), "{s}");
                assert!(uuid::Uuid::parse_str(s).is_err(), "{s}");
            }
            serde_json::Value::Array(items) => items.iter().for_each(walk),
            serde_json::Value::Object(map) => {
                for (key, inner) in map {
                    assert!(!FORBIDDEN_KEYS.contains(&key.as_str()), "{key}");
                    walk(inner);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn a_record_holds_only_labels_numbers_and_hashes() {
        let as_value = serde_json::to_value(record(ComparisonSide::Baseline)).expect("value");
        let bytes = to_canonical_vec(&as_value).expect("canonical bytes");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        walk(&value);

        let mut with_extra = value;
        with_extra["input"] = serde_json::json!("x");
        assert!(serde_json::from_value::<ComparisonRecord>(with_extra).is_err());
        let text = r#"{"position":7,"unknown":1}"#;
        assert!(serde_json::from_str::<ComparisonRecord>(text).is_err());
    }

    #[test]
    fn a_one_side_refusal_reports_receipt_code_only() {
        let (b, mut c) = pair();
        refuse(&mut c);
        assert_eq!(compare_records(&b, &c), unexplained(&["receipt_code"]));
    }

    #[test]
    fn scored_without_gate_values_is_refused() {
        let (b, mut c) = pair();
        c.gate = None;
        assert_eq!(compare_records(&b, &c), unexplained(&["record_pair"]));
        let (mut b, c) = pair();
        b.scored = false;
        assert_eq!(compare_records(&b, &c), unexplained(&["record_pair"]));
    }

    #[test]
    fn two_refused_records_are_unexplained() {
        let (mut b, mut c) = pair();
        for r in [&mut b, &mut c] {
            refuse(r);
            r.member = false;
            r.member_chunks = vec![];
            r.credit_events = vec![];
        }
        assert_eq!(compare_records(&b, &c), unexplained(&["terminal"]));
    }
}
