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

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use trace_commons_gate_api::pipeline::BundlePackage;
use trace_commons_protocol::canonical_json::to_canonical_vec;

use crate::versioned_pipeline::sha256_prefixed;
use crate::versioned_pipeline_qualification::package_digests;

pub const COMPARISON_REPORT_SCHEMA: &str = "trace_commons.pipeline_comparison_report.v1";
pub const COMPARE_CORPUS_SCHEMA: &str = "trace_commons.pipeline_compare_corpus.v1";
pub const UNEXPLAINED_LIST_LIMIT: usize = 1_000;
pub const COMPARISON_BLOCKERS: [&str; 10] = [
    "local_test_only",
    "local_reference_scorer",
    "local_reference_embedder",
    "synthetic_index",
    "synthetic_settlement",
    "static_bearer_authentication",
    "deterministic_privacy_only",
    // baseline-old-path:
    "baseline_derived_scan_removed",
    // PC-D24: `main`'s duplicate short-circuits (`skipped_duplicate`,
    // `cached`) run on the two production paths since #1325, and on neither
    // side of the run.
    "duplicate_short_circuits_not_compared",
    // PC-D25: the harness reads the candidate's privacy fields at the
    // receipt, before the Review-start privacy pass of #1324.
    "review_start_privacy_pass_not_compared",
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
/// compared field (PC-D12). [`PermittedDifference`] holds those.
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

/// A difference in a compared field that an owner ruling permits (spec
/// section 8.3). A rule permits a difference only when its exact condition
/// holds. Any other pair stays unexplained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PermittedDifference {
    /// PC-D22. The server-side risk is `medium` for a cause other than the
    /// consent flag alone. The old path admits the trace and the pipeline
    /// quarantines it for review. The basis clause is "not exactly
    /// `["consent_content_flag"]`", so an empty basis also meets it: a
    /// declared risk has no basis label (the self-test's permitted pair).
    MediumRiskPrivacyReview,
}

impl PermittedDifference {
    pub const ALL: [PermittedDifference; 1] = [PermittedDifference::MediumRiskPrivacyReview];

    pub fn id(self) -> &'static str {
        match self {
            PermittedDifference::MediumRiskPrivacyReview => "medium_risk_privacy_review",
        }
    }

    pub fn source(self) -> &'static str {
        match self {
            PermittedDifference::MediumRiskPrivacyReview => "ruling.PC-D22",
        }
    }

    pub fn fields(self) -> &'static [&'static str] {
        match self {
            PermittedDifference::MediumRiskPrivacyReview => &["admission"],
        }
    }

    pub fn permits(
        self,
        field: &str,
        baseline: &ComparisonRecord,
        candidate: &ComparisonRecord,
    ) -> bool {
        if !self.fields().contains(&field) {
            return false;
        }
        match self {
            PermittedDifference::MediumRiskPrivacyReview => {
                let medium =
                    |record: &ComparisonRecord| record.privacy_risk.as_deref() == Some("medium");
                medium(baseline)
                    && medium(candidate)
                    && baseline.privacy_basis == candidate.privacy_basis
                    && baseline.privacy_basis != ["consent_content_flag"]
                    && baseline.admission == AdmissionLabel::Admit
                    && candidate.admission == AdmissionLabel::Quarantine
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

/// The production comparison: only a [`PermittedDifference`] permits a
/// difference. The first rule that permits the field gives its id.
pub fn compare_records(
    baseline: &ComparisonRecord,
    candidate: &ComparisonRecord,
) -> TraceComparison {
    compare_records_with(baseline, candidate, &|field, baseline, candidate| {
        PermittedDifference::ALL
            .into_iter()
            .find(|rule| rule.permits(field, baseline, candidate))
            .map(PermittedDifference::id)
    })
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

/// What the harness does for one pair of admissions, so that both indexes
/// stay equal (spec section 8.2).
// baseline-old-path: `SkipBaselineGate`, `ApproveBaseline`, `RejectBaseline`, and their rows below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignmentAction {
    None,
    ApproveCandidate,
    SkipBaselineGate,
    ApproveBaseline,
    HashRuleBoth(ReviewLabel),
    RejectBaseline,
    /// A side is `Refused` or `Other`: no review and no gate call.
    Stop,
}

/// The review decision for a trace that both sides quarantine. The first byte
/// of the hash decides: an even byte approves, an odd byte rejects. Any text
/// that is not `sha256:` and two hexadecimal characters rejects (fail closed).
pub fn hash_rule(trace_hash: &str) -> ReviewLabel {
    let byte = trace_hash
        .strip_prefix("sha256:")
        .and_then(|hex| hex.get(..2))
        .filter(|pair| pair.bytes().all(|b| b.is_ascii_hexdigit()))
        .and_then(|pair| u8::from_str_radix(pair, 16).ok());
    match byte {
        Some(byte) if byte % 2 == 0 => ReviewLabel::Approve,
        _ => ReviewLabel::Reject,
    }
}

pub fn alignment_action(
    baseline: AdmissionLabel,
    candidate: AdmissionLabel,
    trace_hash: &str,
) -> AlignmentAction {
    use AdmissionLabel::*;
    match (baseline, candidate) {
        (Admit, Admit) | (Reject, Reject) => AlignmentAction::None,
        (Admit, Quarantine) => AlignmentAction::ApproveCandidate,
        (Admit, Reject) => AlignmentAction::SkipBaselineGate,
        (Quarantine, Admit) => AlignmentAction::ApproveBaseline,
        (Quarantine, Quarantine) => AlignmentAction::HashRuleBoth(hash_rule(trace_hash)),
        (Quarantine, Reject) => AlignmentAction::RejectBaseline,
        // The old path has no direct rejection, so `(Reject, Admit)` and
        // `(Reject, Quarantine)` are not a pair it can give.
        _ => AlignmentAction::Stop,
    }
}

/// PC-D20. True when, after this pair, the two indexes can differ:
/// 1. `action` is `Stop` and `candidate.admission` is `Admit` (the worker adds
///    the trace with no call from the harness, and the baseline adds nothing);
/// 2. `action` is not `Stop` and a side has `terminal: false` (a call failed,
///    or the candidate can still complete later);
/// 3. the two sides are terminal and `member` or `member_chunks` differ.
///
/// False for every other pair, also for a pair that differs in other fields.
pub fn alignment_lost(
    action: AlignmentAction,
    baseline: &ComparisonRecord,
    candidate: &ComparisonRecord,
) -> bool {
    if action == AlignmentAction::Stop && candidate.admission == AdmissionLabel::Admit {
        return true;
    }
    if action != AlignmentAction::Stop && !(baseline.terminal && candidate.terminal) {
        return true;
    }
    baseline.terminal
        && candidate.terminal
        && (baseline.member != candidate.member
            || baseline.member_chunks != candidate.member_chunks)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct DerivedFloors {
    pub perplexity_floor_micros: u64,
    pub tail_fraction_floor_micros: u64,
    pub novelty_floor_micros: u64,
}

/// The element at index `(len - 1) / 2` of the sorted values.
pub fn lower_median(values: &[u64]) -> Option<u64> {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    (!sorted.is_empty()).then(|| sorted[(sorted.len() - 1) / 2])
}

/// `Err("comparison_calibration_invalid")` for slices of different lengths,
/// `Err("comparison_calibration_empty")` for an empty input,
/// `Err("comparison_floors_all_zero")` when every floor is zero.
pub fn derive_floors(
    perplexity: &[u64],
    tail_fraction: &[u64],
    novelty: &[u64],
) -> Result<DerivedFloors, &'static str> {
    if perplexity.len() != tail_fraction.len() || perplexity.len() != novelty.len() {
        return Err("comparison_calibration_invalid");
    }
    let (Some(perplexity), Some(tail_fraction), Some(novelty)) = (
        lower_median(perplexity),
        lower_median(tail_fraction),
        lower_median(novelty),
    ) else {
        return Err("comparison_calibration_empty");
    };
    if perplexity == 0 && tail_fraction == 0 && novelty == 0 {
        return Err("comparison_floors_all_zero");
    }
    Ok(DerivedFloors {
        perplexity_floor_micros: perplexity,
        tail_fraction_floor_micros: tail_fraction,
        novelty_floor_micros: novelty,
    })
}

/// The counts of one side. `member` and `not_member` count scored traces only.
#[derive(Debug, Default, Serialize)]
struct SideDistribution {
    admit: u64,
    quarantine: u64,
    reject: u64,
    refused: u64,
    other: u64,
    scored: u64,
    quality_passed: u64,
    quality_failed: u64,
    novelty_passed: u64,
    novelty_failed: u64,
    member: u64,
    not_member: u64,
    chunks_capped: u64,
}

impl SideDistribution {
    fn observe(&mut self, record: &ComparisonRecord) {
        match record.admission {
            AdmissionLabel::Admit => self.admit += 1,
            AdmissionLabel::Quarantine => self.quarantine += 1,
            AdmissionLabel::Reject => self.reject += 1,
            AdmissionLabel::Refused => self.refused += 1,
            AdmissionLabel::Other => self.other += 1,
        }
        let Some(gate) = record.gate.as_ref().filter(|_| record.scored) else {
            return;
        };
        self.scored += 1;
        if gate.quality_passed {
            self.quality_passed += 1;
        } else {
            self.quality_failed += 1;
        }
        if gate.novelty_passed {
            self.novelty_passed += 1;
        } else {
            self.novelty_failed += 1;
        }
        if record.member {
            self.member += 1;
        } else {
            self.not_member += 1;
        }
        if gate.chunks_capped {
            self.chunks_capped += 1;
        }
    }
}

#[derive(Debug, Serialize)]
struct UnexplainedEntry {
    position: u64,
    trace_hash: String,
    fields: Vec<&'static str>,
}

/// Counts only. No record is kept except the bounded unexplained list.
#[derive(Debug, Default)]
pub struct ComparisonSummary {
    compared: u64,
    equal: u64,
    permitted: BTreeMap<&'static str, u64>,
    /// The pairs with the result `permitted`. A pair that several rules
    /// permit adds 1 here and 1 to each rule.
    permitted_total: u64,
    unexplained_fields: BTreeMap<&'static str, u64>,
    unexplained_total: u64,
    unexplained: Vec<UnexplainedEntry>,
    first_unexplained_position: Option<u64>,
    baseline: SideDistribution,
    candidate: SideDistribution,
}

impl ComparisonSummary {
    pub fn observe(
        &mut self,
        baseline: &ComparisonRecord,
        candidate: &ComparisonRecord,
        result: &TraceComparison,
    ) {
        self.compared += 1;
        self.baseline.observe(baseline);
        self.candidate.observe(candidate);
        match result {
            TraceComparison::Equal => self.equal += 1,
            TraceComparison::Permitted { rules } => {
                self.permitted_total += 1;
                for rule in rules {
                    *self.permitted.entry(rule).or_default() += 1;
                }
            }
            TraceComparison::Unexplained { fields } => {
                self.unexplained_total += 1;
                self.first_unexplained_position
                    .get_or_insert(baseline.position);
                for field in fields {
                    *self.unexplained_fields.entry(field).or_default() += 1;
                }
                if self.unexplained.len() < UNEXPLAINED_LIST_LIMIT {
                    self.unexplained.push(UnexplainedEntry {
                        position: baseline.position,
                        trace_hash: baseline.trace_hash.clone(),
                        fields: fields.clone(),
                    });
                }
            }
        }
    }

    pub fn unexplained_total(&self) -> u64 {
        self.unexplained_total
    }

    /// The sum of the `refused` counts of the two sides (PC-D19).
    pub fn refused_total(&self) -> u64 {
        self.baseline.refused + self.candidate.refused
    }

    /// The labels of the branches with no evidence, read from the baseline side.
    pub fn branch_gaps(&self) -> Vec<&'static str> {
        let side = &self.baseline;
        [
            ("quality_passed_true", side.quality_passed),
            ("quality_passed_false", side.quality_failed),
            ("novelty_passed_true", side.novelty_passed),
            ("novelty_passed_false", side.novelty_failed),
            ("member_true", side.member),
            ("member_false", side.not_member),
        ]
        .into_iter()
        .filter(|(_, count)| *count == 0)
        .map(|(label, _)| label)
        .collect()
    }
}

pub struct ComparisonReportInput<'a> {
    pub check_id: &'a str,
    /// The number of traces in the pin. The run is partial when the
    /// summary observed fewer pairs.
    pub trace_count: u64,
    pub partial: bool,
    /// Keys: `source`, `order`, `configuration`, `bootstrap_corpus`,
    /// `holdout_corpus`.
    pub pin_digests: &'a BTreeMap<String, String>,
    pub package: &'a BundlePackage,
    pub floors: DerivedFloors,
    pub skew: Option<&'a str>,
    /// The position of the pair at which the run stopped (PC-D20), or `None`.
    pub alignment_lost_position: Option<u64>,
    pub records_digest: &'a str,
}

/// The report of `trace_commons.pipeline_comparison_report.v1`. It holds no
/// time, so two runs of one pin give one `report_digest`.
pub fn comparison_report(
    input: &ComparisonReportInput<'_>,
    summary: &ComparisonSummary,
) -> Result<serde_json::Value, String> {
    let pin_digest = |key: &str| {
        input
            .pin_digests
            .get(key)
            .cloned()
            .ok_or_else(|| "comparison_pin_digest_missing".to_string())
    };
    let digests = package_digests(input.package)?;
    let excluded_rules: Vec<serde_json::Value> = ComparisonRule::ALL
        .iter()
        .map(|rule| {
            serde_json::json!({
                "rule": rule.id(),
                "source": rule.source(),
                "fields": rule.excluded_fields(),
            })
        })
        .collect();
    let permitted_rules: Vec<serde_json::Value> = PermittedDifference::ALL
        .iter()
        .map(|rule| {
            serde_json::json!({
                "rule": rule.id(),
                "source": rule.source(),
                "fields": rule.fields(),
            })
        })
        .collect();
    let branch_gaps = if input.partial {
        Vec::new()
    } else {
        summary.branch_gaps()
    };
    let invalid = |_| "comparison_report_invalid".to_string();
    let mut report = serde_json::json!({
        "schema": COMPARISON_REPORT_SCHEMA,
        "check_id": input.check_id,
        "scope": "local_test",
        "production_ready": false,
        "external_payout_enabled": false,
        "partial": input.partial,
        "skew": input.skew,
        "safe_blockers": COMPARISON_BLOCKERS,
        "pin": {
            "source_digest": pin_digest("source")?,
            "order_digest": pin_digest("order")?,
            "configuration_digest": pin_digest("configuration")?,
            "bootstrap_corpus_digest": pin_digest("bootstrap_corpus")?,
            "holdout_corpus_digest": pin_digest("holdout_corpus")?,
        },
        "bundle_id": input.package.bundle_id,
        "package_hash": digests.package_hash,
        "configuration_digest": digests.configuration_digest,
        "dependency_digest": digests.dependency_digest,
        "floors": serde_json::to_value(input.floors).map_err(invalid)?,
        "trace_count": input.trace_count,
        "compared_count": summary.compared,
        "equal_count": summary.equal,
        "permitted_counts": summary.permitted,
        "permitted_total": summary.permitted_total,
        "unexplained_counts": summary.unexplained_fields,
        "unexplained_total": summary.unexplained_total,
        "unexplained": serde_json::to_value(&summary.unexplained).map_err(invalid)?,
        "first_unexplained_position": summary.first_unexplained_position,
        "alignment_lost_position": input.alignment_lost_position,
        "excluded_rules": excluded_rules,
        "permitted_rules": permitted_rules,
        "distribution": {
            "baseline": serde_json::to_value(&summary.baseline).map_err(invalid)?,
            "candidate": serde_json::to_value(&summary.candidate).map_err(invalid)?,
        },
        "branch_gaps": branch_gaps,
        "records_digest": input.records_digest,
    });
    let digest = sha256_prefixed(
        &to_canonical_vec(&report).map_err(|_| "comparison_report_invalid".to_string())?,
    );
    report["report_digest"] = serde_json::Value::String(digest);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versioned_pipeline_compat::CompatibilityBundleConfig;
    use crate::versioned_pipeline_qualification::is_safe_label;
    use trace_commons_gate_api::{ReferenceEmbedder, ReferencePerplexityScorer};
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
        let mut with_gate_extra =
            serde_json::to_value(record(ComparisonSide::Baseline)).expect("value");
        with_gate_extra["gate"]["unknown"] = serde_json::json!(1);
        assert!(serde_json::from_value::<ComparisonRecord>(with_gate_extra).is_err());
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

    fn package() -> BundlePackage {
        crate::versioned_pipeline_bundle::MinimalPolicyBundle::compatibility_package(
            &CompatibilityBundleConfig::local_reference(),
            &ReferencePerplexityScorer::new(),
            &ReferenceEmbedder::new(),
        )
        .expect("the compatibility package builds")
    }

    fn pin_digests() -> BTreeMap<String, String> {
        [
            ("source", '1'),
            ("order", '2'),
            ("configuration", '3'),
            ("bootstrap_corpus", '4'),
            ("holdout_corpus", '5'),
        ]
        .into_iter()
        .map(|(key, digit)| {
            (
                key.to_string(),
                format!("sha256:{}", digit.to_string().repeat(64)),
            )
        })
        .collect()
    }

    fn floors() -> DerivedFloors {
        DerivedFloors {
            perplexity_floor_micros: 20,
            tail_fraction_floor_micros: 0,
            novelty_floor_micros: 8,
        }
    }

    fn build_report(
        summary: &ComparisonSummary,
        partial: bool,
        lost: Option<u64>,
    ) -> serde_json::Value {
        let package = package();
        let digests = pin_digests();
        comparison_report(
            &ComparisonReportInput {
                check_id: "pipeline_comparison_local",
                trace_count: 3,
                partial,
                pin_digests: &digests,
                package: &package,
                floors: floors(),
                skew: None,
                alignment_lost_position: lost,
                records_digest: &format!("sha256:{}", "6".repeat(64)),
            },
            summary,
        )
        .expect("the report builds")
    }

    fn pair_at(position: u64) -> (ComparisonRecord, ComparisonRecord) {
        let (mut b, mut c) = pair();
        b.position = position;
        c.position = position;
        (b, c)
    }

    fn unscored(r: &mut ComparisonRecord) {
        r.scored = false;
        r.gate = None;
    }

    fn contains_time_key(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Array(items) => items.iter().any(contains_time_key),
            serde_json::Value::Object(map) => map.iter().any(|(key, inner)| {
                key.ends_with("_at")
                    || key.starts_with("duration")
                    || key.starts_with("elapsed")
                    || contains_time_key(inner)
            }),
            _ => false,
        }
    }

    #[test]
    fn the_hash_rule_reads_the_first_byte() {
        let with = |byte: &str| format!("sha256:{byte}{}", "0".repeat(62));
        assert_eq!(hash_rule(&with("00")), ReviewLabel::Approve);
        assert_eq!(hash_rule(&with("fe")), ReviewLabel::Approve);
        assert_eq!(hash_rule(&with("01")), ReviewLabel::Reject);
        assert_eq!(hash_rule(&with("ff")), ReviewLabel::Reject);
        assert_eq!(hash_rule(&"00".repeat(32)), ReviewLabel::Reject);
        assert_eq!(hash_rule(&with("zz")), ReviewLabel::Reject);
        assert_eq!(hash_rule(&with("+e")), ReviewLabel::Reject);
        assert_eq!(hash_rule("sha256:"), ReviewLabel::Reject);
        assert_eq!(hash_rule(""), ReviewLabel::Reject);
    }

    #[test]
    fn the_alignment_table_covers_every_pair() {
        use AdmissionLabel::*;
        let hash = format!("sha256:00{}", "0".repeat(62));
        let odd = format!("sha256:01{}", "0".repeat(62));
        assert_eq!(hash_rule(&hash), ReviewLabel::Approve);
        assert_eq!(hash_rule(&odd), ReviewLabel::Reject);
        let all = [Admit, Quarantine, Reject, Refused, Other];
        let mut seen = 0;
        for baseline in all {
            for candidate in all {
                let expected = match (baseline, candidate) {
                    (Admit, Admit) => AlignmentAction::None,
                    (Admit, Quarantine) => AlignmentAction::ApproveCandidate,
                    (Admit, Reject) => AlignmentAction::SkipBaselineGate,
                    (Quarantine, Admit) => AlignmentAction::ApproveBaseline,
                    (Quarantine, Quarantine) => AlignmentAction::HashRuleBoth(hash_rule(&hash)),
                    (Quarantine, Reject) => AlignmentAction::RejectBaseline,
                    (Reject, Reject) => AlignmentAction::None,
                    _ => AlignmentAction::Stop,
                };
                assert_eq!(
                    alignment_action(baseline, candidate, &hash),
                    expected,
                    "{baseline:?} {candidate:?}"
                );
                seen += 1;
            }
        }
        assert_eq!(seen, 25);
        assert_eq!(
            alignment_action(Quarantine, Quarantine, &odd),
            AlignmentAction::HashRuleBoth(ReviewLabel::Reject)
        );
        assert_eq!(
            alignment_action(Quarantine, Quarantine, &hash),
            AlignmentAction::HashRuleBoth(ReviewLabel::Approve)
        );
    }

    #[test]
    fn the_lower_median_is_exact() {
        assert_eq!(lower_median(&[]), None);
        assert_eq!(lower_median(&[5]), Some(5));
        assert_eq!(lower_median(&[3, 1]), Some(1));
        assert_eq!(lower_median(&[9, 1, 5]), Some(5));
        assert_eq!(lower_median(&[4, 1, 3, 2]), Some(2));
        assert_eq!(lower_median(&[2, 3, 1, 4]), Some(2));
    }

    #[test]
    fn floors_are_the_three_medians() {
        assert_eq!(
            derive_floors(&[10, 30, 20], &[0, 0, 0], &[7, 9, 8]),
            Ok(DerivedFloors {
                perplexity_floor_micros: 20,
                tail_fraction_floor_micros: 0,
                novelty_floor_micros: 8,
            })
        );
        assert_eq!(
            derive_floors(&[], &[], &[]),
            Err("comparison_calibration_empty")
        );
        assert_eq!(
            derive_floors(&[0, 0], &[0, 0], &[0, 0]),
            Err("comparison_floors_all_zero")
        );
        assert_eq!(
            derive_floors(&[1, 2], &[1], &[1, 2]),
            Err("comparison_calibration_invalid")
        );
        assert_eq!(
            derive_floors(&[1], &[], &[1]),
            Err("comparison_calibration_invalid")
        );
    }

    #[test]
    fn the_summary_counts_each_result() {
        let mut summary = ComparisonSummary::default();
        let (b, c) = pair_at(1);
        summary.observe(&b, &c, &TraceComparison::Equal);
        let (b, c) = pair_at(2);
        summary.observe(
            &b,
            &c,
            &TraceComparison::Permitted {
                rules: vec!["test_rule"],
            },
        );
        let (b, c) = pair_at(3);
        summary.observe(&b, &c, &unexplained(&["admission", "member"]));
        let report = build_report(&summary, false, None);
        assert_eq!(report["compared_count"], 3);
        assert_eq!(report["equal_count"], 1);
        assert_eq!(
            report["permitted_counts"],
            serde_json::json!({"test_rule": 1})
        );
        assert_eq!(
            report["unexplained_counts"],
            serde_json::json!({"admission": 1, "member": 1})
        );
        assert_eq!(report["unexplained_total"], 1);
        assert_eq!(summary.unexplained_total(), 1);
        assert_eq!(
            report["unexplained"],
            serde_json::json!([{
                "position": 3,
                "trace_hash": b.trace_hash,
                "fields": ["admission", "member"],
            }])
        );
        assert_eq!(report["first_unexplained_position"], 3);
    }

    #[test]
    fn the_unexplained_list_is_bounded() {
        let mut summary = ComparisonSummary::default();
        for position in 0..1_005 {
            let (b, c) = pair_at(position);
            summary.observe(&b, &c, &unexplained(&["admission"]));
        }
        let report = build_report(&summary, false, None);
        let list = report["unexplained"].as_array().expect("a list");
        assert_eq!(list.len(), UNEXPLAINED_LIST_LIMIT);
        assert_eq!(list[0]["position"], 0);
        assert_eq!(list[999]["position"], 999);
        assert_eq!(report["unexplained_total"], 1_005);
        assert_eq!(report["unexplained_counts"]["admission"], 1_005);
        assert_eq!(report["first_unexplained_position"], 0);
    }

    #[test]
    fn the_branch_check_reports_each_gap() {
        let mut summary = ComparisonSummary::default();
        let (mut b, mut c) = pair();
        unscored(&mut b);
        unscored(&mut c);
        summary.observe(&b, &c, &TraceComparison::Equal);
        let all = vec![
            "quality_passed_true",
            "quality_passed_false",
            "novelty_passed_true",
            "novelty_passed_false",
            "member_true",
            "member_false",
        ];
        assert_eq!(summary.branch_gaps(), all);
        assert_eq!(
            build_report(&summary, false, None)["branch_gaps"],
            serde_json::json!(all)
        );
        assert_eq!(
            build_report(&summary, true, None)["branch_gaps"],
            serde_json::json!([])
        );

        let (b, c) = pair();
        summary.observe(&b, &c, &TraceComparison::Equal);
        assert_eq!(
            summary.branch_gaps(),
            vec![
                "quality_passed_false",
                "novelty_passed_false",
                "member_false"
            ]
        );
    }

    #[test]
    fn the_distribution_counts_each_side() {
        let mut summary = ComparisonSummary::default();
        let (b, mut c) = pair();
        c.admission = AdmissionLabel::Reject;
        unscored(&mut c);
        c.member = false;
        summary.observe(&b, &c, &unexplained(&["admission"]));
        let report = build_report(&summary, false, None);
        let base = &report["distribution"]["baseline"];
        let cand = &report["distribution"]["candidate"];
        assert_eq!(base["admit"], 1);
        assert_eq!(base["quality_passed"], 1);
        assert_eq!(base["novelty_passed"], 1);
        assert_eq!(base["member"], 1);
        assert_eq!(base["not_member"], 0);
        assert_eq!(base["scored"], 1);
        assert_eq!(cand["reject"], 1);
        assert_eq!(cand["admit"], 0);
        assert_eq!(cand["scored"], 0);
        assert_eq!(cand["not_member"], 0);
        assert_eq!(summary.refused_total(), 0);

        let (mut b, mut c) = pair_at(9);
        refuse(&mut b);
        refuse(&mut c);
        summary.observe(&b, &c, &unexplained(&["terminal"]));
        assert_eq!(summary.refused_total(), 2);
    }

    #[test]
    fn the_report_digest_is_stable() {
        let observe = || {
            let mut summary = ComparisonSummary::default();
            let (b, c) = pair_at(1);
            summary.observe(&b, &c, &TraceComparison::Equal);
            let (b, c) = pair_at(2);
            summary.observe(&b, &c, &unexplained(&["member_chunks"]));
            summary
        };
        let first = build_report(&observe(), false, None);
        let second = build_report(&observe(), false, None);
        assert_eq!(first, second);
        let mut without = first.clone();
        let digest = without
            .as_object_mut()
            .expect("an object")
            .remove("report_digest")
            .expect("a digest");
        let bytes = to_canonical_vec(&without).expect("canonical bytes");
        assert_eq!(
            digest,
            serde_json::json!(crate::versioned_pipeline::sha256_prefixed(&bytes))
        );
        assert!(!contains_time_key(&first));
    }

    #[test]
    fn the_report_holds_its_fixed_fields() {
        let mut summary = ComparisonSummary::default();
        let (b, c) = pair();
        summary.observe(&b, &c, &TraceComparison::Equal);
        let report = build_report(&summary, false, None);
        let package = package();
        let digests = package_digests(&package).expect("digests");
        assert_eq!(report["schema"], COMPARISON_REPORT_SCHEMA);
        assert_eq!(report["check_id"], "pipeline_comparison_local");
        assert_eq!(report["scope"], "local_test");
        assert_eq!(report["production_ready"], false);
        assert_eq!(report["external_payout_enabled"], false);
        assert_eq!(report["partial"], false);
        assert_eq!(
            report["safe_blockers"],
            serde_json::json!(COMPARISON_BLOCKERS)
        );
        // PC-D24 and PC-D25: two paths of `main` that the run does not
        // compare. Each report names them.
        for blocker in [
            "duplicate_short_circuits_not_compared",
            "review_start_privacy_pass_not_compared",
        ] {
            assert!(
                report["safe_blockers"]
                    .as_array()
                    .expect("a list")
                    .contains(&serde_json::json!(blocker)),
                "{blocker}"
            );
            assert!(is_safe_label(blocker), "{blocker}");
        }
        assert_eq!(
            report["excluded_rules"].as_array().expect("a list").len(),
            3
        );
        for (item, rule) in report["excluded_rules"]
            .as_array()
            .expect("a list")
            .iter()
            .zip(ComparisonRule::ALL)
        {
            assert_eq!(item["rule"], rule.id());
            assert_eq!(item["source"], rule.source());
            assert_eq!(item["fields"], serde_json::json!(rule.excluded_fields()));
        }
        assert_eq!(
            report["pin"],
            serde_json::json!({
                "source_digest": format!("sha256:{}", "1".repeat(64)),
                "order_digest": format!("sha256:{}", "2".repeat(64)),
                "configuration_digest": format!("sha256:{}", "3".repeat(64)),
                "bootstrap_corpus_digest": format!("sha256:{}", "4".repeat(64)),
                "holdout_corpus_digest": format!("sha256:{}", "5".repeat(64)),
            })
        );
        assert_eq!(
            report["floors"],
            serde_json::json!({
                "perplexity_floor_micros": 20,
                "tail_fraction_floor_micros": 0,
                "novelty_floor_micros": 8,
            })
        );
        assert!(report["skew"].is_null());
        assert!(report["alignment_lost_position"].is_null());
        assert_eq!(report["trace_count"], 3);
        assert_eq!(report["bundle_id"], serde_json::json!(package.bundle_id));
        assert_eq!(report["package_hash"], digests.package_hash);
        assert_eq!(report["configuration_digest"], digests.configuration_digest);
        assert_eq!(report["dependency_digest"], digests.dependency_digest);
        assert_eq!(
            report["records_digest"],
            format!("sha256:{}", "6".repeat(64))
        );
        assert!(report["report_digest"].is_string());
        assert!(report.get("digest").is_none());

        let mut missing = pin_digests();
        missing.remove("order");
        let result = comparison_report(
            &ComparisonReportInput {
                check_id: "pipeline_comparison_local",
                trace_count: 3,
                partial: false,
                pin_digests: &missing,
                package: &package,
                floors: floors(),
                skew: Some("baseline_quality_floor"),
                alignment_lost_position: None,
                records_digest: "sha256:x",
            },
            &summary,
        );
        assert_eq!(result, Err("comparison_pin_digest_missing".to_string()));
    }

    #[test]
    fn stop_with_a_candidate_admit_loses_the_alignment() {
        let (mut b, c) = pair();
        for label in [AdmissionLabel::Refused, AdmissionLabel::Other] {
            b.admission = label;
            assert!(alignment_lost(AlignmentAction::Stop, &b, &c), "{label:?}");
        }
        let (b, mut c) = pair();
        for label in [
            AdmissionLabel::Quarantine,
            AdmissionLabel::Reject,
            AdmissionLabel::Refused,
        ] {
            c.admission = label;
            assert!(!alignment_lost(AlignmentAction::Stop, &b, &c), "{label:?}");
        }
        let (mut b, mut c) = pair();
        for r in [&mut b, &mut c] {
            refuse(r);
            r.member = false;
            r.member_chunks = vec![];
            r.credit_events = vec![];
        }
        assert!(!alignment_lost(AlignmentAction::Stop, &b, &c));
    }

    #[test]
    fn the_alignment_is_lost_only_when_the_indexes_can_differ() {
        let none = AlignmentAction::None;
        let (b, c) = pair();
        assert!(!alignment_lost(none, &b, &c));
        let (b, mut c) = pair();
        c.terminal = false;
        assert!(alignment_lost(none, &b, &c));
        let (mut b, c) = pair();
        b.terminal = false;
        assert!(alignment_lost(none, &b, &c));
        let (b, mut c) = pair();
        c.member = false;
        assert!(alignment_lost(none, &b, &c));
        let (b, mut c) = pair();
        c.member_chunks = vec![0];
        let mut b = b;
        b.member_chunks = vec![0, 1];
        assert!(alignment_lost(none, &b, &c));
        let (b, mut c) = pair();
        gate(&mut c).perplexity_micros += 1;
        assert!(!alignment_lost(none, &b, &c));
        let (b, mut c) = pair();
        c.admission = AdmissionLabel::Quarantine;
        assert!(!alignment_lost(AlignmentAction::ApproveCandidate, &b, &c));
    }

    #[test]
    fn the_report_names_the_alignment_loss() {
        let mut summary = ComparisonSummary::default();
        let (b, c) = pair_at(4);
        summary.observe(&b, &c, &TraceComparison::Equal);
        let lost = build_report(&summary, false, Some(4));
        let kept = build_report(&summary, false, None);
        assert_eq!(lost["alignment_lost_position"], 4);
        assert_ne!(lost["report_digest"], kept["report_digest"]);
    }

    /// A pair that holds the exact condition of `MediumRiskPrivacyReview`.
    fn medium_pair(basis: &[&str]) -> (ComparisonRecord, ComparisonRecord) {
        let (mut b, mut c) = pair();
        for r in [&mut b, &mut c] {
            r.privacy_risk = Some("medium".into());
            r.privacy_basis = basis.iter().map(|label| label.to_string()).collect();
        }
        b.admission = AdmissionLabel::Admit;
        c.admission = AdmissionLabel::Quarantine;
        (b, c)
    }

    fn permitted_medium() -> TraceComparison {
        TraceComparison::Permitted {
            rules: vec!["medium_risk_privacy_review"],
        }
    }

    #[test]
    fn the_medium_risk_rule_permits_its_exact_condition() {
        for basis in [
            &["consent_content_flag", "found_and_removed"][..],
            &["found_and_removed"][..],
            &[][..],
        ] {
            let (b, c) = medium_pair(basis);
            assert_eq!(compare_records(&b, &c), permitted_medium(), "{basis:?}");
        }
    }

    #[test]
    fn the_medium_risk_rule_permits_nothing_near_its_condition() {
        let only_admission = unexplained(&["admission"]);
        let (b, c) = medium_pair(&["consent_content_flag"]);
        assert_eq!(compare_records(&b, &c), only_admission);
        for risk in ["low", "high"] {
            let (mut b, mut c) = medium_pair(&["found_and_removed"]);
            b.privacy_risk = Some(risk.into());
            c.privacy_risk = Some(risk.into());
            assert_eq!(compare_records(&b, &c), only_admission, "{risk}");
        }
        let (mut b, mut c) = medium_pair(&["found_and_removed"]);
        b.privacy_risk = None;
        c.privacy_risk = None;
        assert_eq!(compare_records(&b, &c), only_admission);
        let (b, mut c) = medium_pair(&["found_and_removed"]);
        c.privacy_basis = vec!["entropy_flag".into()];
        assert_eq!(
            compare_records(&b, &c),
            unexplained(&["privacy_basis", "admission"])
        );
        let directions = [
            (AdmissionLabel::Quarantine, AdmissionLabel::Admit),
            (AdmissionLabel::Admit, AdmissionLabel::Reject),
            (AdmissionLabel::Quarantine, AdmissionLabel::Reject),
        ];
        for (baseline, candidate) in directions {
            let (mut b, mut c) = medium_pair(&["found_and_removed"]);
            b.admission = baseline;
            c.admission = candidate;
            assert_eq!(compare_records(&b, &c), only_admission, "{baseline:?}");
        }
    }

    #[test]
    fn the_medium_risk_rule_does_not_hide_another_field() {
        let (b, mut c) = medium_pair(&["found_and_removed"]);
        gate(&mut c).chunk_count += 1;
        assert_eq!(compare_records(&b, &c), unexplained(&["chunk_count"]));
    }

    #[test]
    fn permitted_difference_ids_and_sources_are_labels() {
        let exclusions: Vec<&str> = ComparisonRule::ALL.iter().map(|r| r.id()).collect();
        for rule in PermittedDifference::ALL {
            assert!(is_safe_label(rule.id()), "{}", rule.id());
            assert!(is_wide_label(rule.source()), "{}", rule.source());
            assert!(!exclusions.contains(&rule.id()), "{}", rule.id());
            assert!(!rule.fields().is_empty());
            for field in rule.fields() {
                assert!(COMPARED.contains(field), "{field}");
            }
        }
        assert_eq!(
            PermittedDifference::MediumRiskPrivacyReview.id(),
            "medium_risk_privacy_review"
        );
        assert_eq!(
            PermittedDifference::MediumRiskPrivacyReview.source(),
            "ruling.PC-D22"
        );
        assert_eq!(
            PermittedDifference::MediumRiskPrivacyReview.fields(),
            &["admission"]
        );
    }

    #[test]
    fn each_permitted_pair_is_counted_one_time() {
        let mut summary = ComparisonSummary::default();
        let mut position = 0;
        let mut observe = |b: ComparisonRecord, c: ComparisonRecord| {
            let result = compare_records(&b, &c);
            summary.observe(&b, &c, &result);
        };
        for _ in 0..2 {
            let (mut b, mut c) = medium_pair(&["found_and_removed"]);
            b.position = position;
            c.position = position;
            position += 1;
            observe(b, c);
        }
        for _ in 0..3 {
            let (mut b, mut c) = pair();
            b.position = position;
            c.position = position;
            position += 1;
            observe(b, c);
        }
        let (mut b, mut c) = medium_pair(&["consent_content_flag"]);
        b.position = position;
        c.position = position;
        observe(b, c);
        let report = build_report(&summary, false, None);
        assert_eq!(
            report["permitted_counts"],
            serde_json::json!({"medium_risk_privacy_review": 2})
        );
        let count = |key: &str| report[key].as_u64().expect("a number");
        let permitted: u64 = report["permitted_counts"]
            .as_object()
            .expect("a map")
            .values()
            .map(|n| n.as_u64().expect("a number"))
            .sum();
        assert_eq!(count("equal_count"), 3);
        assert_eq!(count("unexplained_total"), 1);
        assert_eq!(
            count("equal_count") + permitted + count("unexplained_total"),
            count("compared_count")
        );
    }

    #[test]
    fn the_report_lists_the_permitted_rules() {
        let summary = ComparisonSummary::default();
        let report = build_report(&summary, false, None);
        assert_eq!(
            report["permitted_rules"],
            serde_json::json!([{
                "rule": "medium_risk_privacy_review",
                "source": "ruling.PC-D22",
                "fields": ["admission"],
            }])
        );
    }

    #[test]
    fn a_pair_with_two_rules_is_counted_one_time() {
        let permit = |field: &'static str, _: &ComparisonRecord, _: &ComparisonRecord| match field {
            "admission" => Some("rule_one"),
            "member" => Some("rule_two"),
            _ => None,
        };
        let mut summary = ComparisonSummary::default();
        let (b, mut c) = pair_at(0);
        c.admission = AdmissionLabel::Quarantine;
        c.member = false;
        let result = compare_records_with(&b, &c, &permit);
        assert_eq!(
            result,
            TraceComparison::Permitted {
                rules: vec!["rule_one", "rule_two"]
            }
        );
        summary.observe(&b, &c, &result);
        let (b, c) = pair_at(1);
        summary.observe(&b, &c, &TraceComparison::Equal);
        let (b, c) = pair_at(2);
        summary.observe(&b, &c, &unexplained(&["member"]));
        let report = build_report(&summary, false, None);
        assert_eq!(report["permitted_total"], 1);
        assert_eq!(
            report["permitted_counts"],
            serde_json::json!({"rule_one": 1, "rule_two": 1})
        );
        let count = |key: &str| report[key].as_u64().expect("a number");
        assert_eq!(
            count("equal_count") + count("permitted_total") + count("unexplained_total"),
            count("compared_count")
        );
    }

    #[test]
    fn the_medium_risk_rule_needs_medium_on_each_side() {
        // The risk differs, and the admission pair is not permitted.
        let both = unexplained(&["privacy_risk", "admission"]);
        let (b, mut c) = medium_pair(&["found_and_removed"]);
        c.privacy_risk = Some("low".into());
        assert_eq!(compare_records(&b, &c), both);
        let (mut b, c) = medium_pair(&["found_and_removed"]);
        b.privacy_risk = Some("low".into());
        assert_eq!(compare_records(&b, &c), both);
        // A second difference is reported alone: the rule still permits the
        // admission.
        type Change = fn(&mut ComparisonRecord);
        let changes: [(Change, &[&'static str]); 3] = [
            (|c| c.terminal = false, &["terminal"]),
            (
                |c| {
                    c.scored = false;
                    c.gate = None;
                },
                &["scored"],
            ),
            (|c| c.receipt_code = 202, &["receipt_code"]),
        ];
        for (change, fields) in changes {
            let (b, mut c) = medium_pair(&["found_and_removed"]);
            change(&mut c);
            assert_eq!(compare_records(&b, &c), unexplained(fields), "{fields:?}");
        }
    }
}
