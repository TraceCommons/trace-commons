//! Source-bound Codex usage evidence extracted from exact selected bytes.
//!
//! Cumulative counters before the first observed snapshot remain explicitly
//! unpriced. This module records no path, source body, or native session ID.

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::SourceFormat;
use super::usage::{NativeTokenCounts, UsageSource};

pub const USAGE_EVIDENCE_SCHEMA_VERSION: u32 = 1;
pub const MAX_USAGE_RECORD_REFS: usize = 64;
const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
const MAX_MODEL_BYTES: usize = 96;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageEvidenceScope {
    ExactSourceFile,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageRecordCoordinates {
    JsonlPhysicalLinesOneBased,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UsageRecordRef {
    pub record_index: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UsageCounterSnapshot {
    pub record_index: u64,
    pub recorded_at: DateTime<Utc>,
    pub counts: NativeTokenCounts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ObservedUsageInterval {
    pub baseline: UsageCounterSnapshot,
    pub final_snapshot: UsageCounterSnapshot,
    /// Checked final minus baseline counters. This is the only price-eligible
    /// quantity; it does not represent the full session.
    pub observed_delta: NativeTokenCounts,
    /// Counters already present at the baseline. Their time/model history is
    /// not inferred and they remain unpriced.
    pub unpriced_prior: NativeTokenCounts,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AggregateUnavailableReason {
    NoUsage,
    IncompleteOrInvalidUsage,
    CumulativeReset,
    MissingOrInvalidSession,
    MultipleSessions,
    Overflow,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntervalUnavailableReason {
    IncompleteAggregate,
    InsufficientBaseline,
    MissingOrInvalidTimestamp,
    Overflow,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelAttributionUnavailableReason {
    NoDeclarationAtBaseline,
    MissingOrInvalidDeclaration,
    ModelChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum UsageModelAttribution {
    SingleDeclaredModel {
        model: String,
    },
    Unavailable {
        reason: ModelAttributionUnavailableReason,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PersistedUsageEvidence {
    pub schema_version: u32,
    pub source: UsageSource,
    pub scope: UsageEvidenceScope,
    pub source_format: SourceFormat,
    pub source_digest: String,
    pub coordinates: UsageRecordCoordinates,
    /// Every native token-count candidate, including malformed and repeated
    /// cumulative snapshots.
    pub candidate_records: u64,
    /// Candidates with a complete, internally valid accounting object.
    pub complete_records: u64,
    pub incomplete_records: u64,
    /// Complete records whose counters repeat the prior complete snapshot.
    pub duplicate_records: u64,
    pub record_refs: Vec<UsageRecordRef>,
    /// Complete supporting records beyond the reference cap. This does not
    /// imply omitted counters or other semantic facts.
    pub omitted_record_refs: u64,
    pub aggregate_counts: Option<NativeTokenCounts>,
    pub aggregate_unavailable_reason: Option<AggregateUnavailableReason>,
    pub interval: Option<ObservedUsageInterval>,
    pub interval_unavailable_reason: Option<IntervalUnavailableReason>,
    pub attribution: UsageModelAttribution,
}

fn invalid() -> anyhow::Error {
    anyhow!("insights_usage_evidence_invalid")
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn safe_model(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_MODEL_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
}

impl PersistedUsageEvidence {
    pub fn has_attributed_interval(&self) -> bool {
        self.validate().is_ok()
            && self.aggregate_counts.is_some()
            && self.interval.is_some()
            && matches!(
                self.attribution,
                UsageModelAttribution::SingleDeclaredModel { .. }
            )
    }

    pub fn validate_binding(&self, format: SourceFormat, source_digest: &str) -> Result<()> {
        self.validate()?;
        if self.source_format != format || self.source_digest != source_digest {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        let complete_plus_incomplete = self.complete_records.checked_add(self.incomplete_records);
        let retained = u64::try_from(self.record_refs.len()).map_err(|_| invalid())?;
        let refs_complete = retained.checked_add(self.omitted_record_refs);
        let expected_retained = self.complete_records.min(MAX_USAGE_RECORD_REFS as u64);
        if self.schema_version != USAGE_EVIDENCE_SCHEMA_VERSION
            || self.source != UsageSource::Codex
            || self.source_format != SourceFormat::Codex
            || !digest(&self.source_digest)
            || complete_plus_incomplete != Some(self.candidate_records)
            || self.candidate_records > MAX_SOURCE_BYTES as u64
            || self.duplicate_records > self.complete_records
            || self.record_refs.len() > MAX_USAGE_RECORD_REFS
            || retained != expected_retained
            || refs_complete != Some(self.complete_records)
            || !self
                .record_refs
                .windows(2)
                .all(|pair| pair[0].record_index < pair[1].record_index)
            || self
                .record_refs
                .iter()
                .any(|reference| reference.record_index == 0)
            || (self.aggregate_counts.is_some() == self.aggregate_unavailable_reason.is_some())
            || (self.interval.is_some() == self.interval_unavailable_reason.is_some())
        {
            return Err(invalid());
        }
        if let Some(counts) = &self.aggregate_counts {
            counts.validate_accounting().map_err(|_| invalid())?;
            if fields(counts).is_none()
                || self.candidate_records == 0
                || self.complete_records != self.candidate_records
            {
                return Err(invalid());
            }
        }
        match &self.attribution {
            UsageModelAttribution::SingleDeclaredModel { model } if safe_model(model) => {}
            UsageModelAttribution::Unavailable { .. } => {}
            _ => return Err(invalid()),
        }
        if let Some(interval) = &self.interval {
            interval.validate()?;
            let first_ref = self.record_refs.first().ok_or_else(invalid)?.record_index;
            let last_ref = self.record_refs.last().ok_or_else(invalid)?.record_index;
            let endpoint_shape_valid = if self.complete_records == 1 {
                interval.baseline == interval.final_snapshot
            } else {
                self.complete_records > 1
                    && interval.baseline.record_index < interval.final_snapshot.record_index
            };
            let final_ref_valid = if self.omitted_record_refs == 0 {
                interval.final_snapshot.record_index == last_ref
            } else {
                interval
                    .final_snapshot
                    .record_index
                    .checked_sub(last_ref)
                    .is_some_and(|gap| gap >= self.omitted_record_refs)
            };
            if self.aggregate_counts.as_ref() != Some(&interval.final_snapshot.counts)
                || interval.baseline.record_index != first_ref
                || !endpoint_shape_valid
                || !final_ref_valid
            {
                return Err(invalid());
            }
        }
        let no_usage =
            self.aggregate_unavailable_reason == Some(AggregateUnavailableReason::NoUsage);
        if no_usage != (self.candidate_records == 0)
            || (no_usage
                && (self.complete_records != 0
                    || self.incomplete_records != 0
                    || self.duplicate_records != 0
                    || !self.record_refs.is_empty()
                    || self.omitted_record_refs != 0))
        {
            return Err(invalid());
        }
        Ok(())
    }
}

impl ObservedUsageInterval {
    fn validate(&self) -> Result<()> {
        self.baseline
            .counts
            .validate_accounting()
            .map_err(|_| invalid())?;
        self.final_snapshot
            .counts
            .validate_accounting()
            .map_err(|_| invalid())?;
        self.observed_delta
            .validate_accounting()
            .map_err(|_| invalid())?;
        self.unpriced_prior
            .validate_accounting()
            .map_err(|_| invalid())?;
        if self.baseline.record_index == 0
            || self.baseline.record_index > self.final_snapshot.record_index
            || (self.baseline.record_index == self.final_snapshot.record_index
                && (fields(&self.baseline.counts) != Some([0; 5])
                    || fields(&self.final_snapshot.counts) != Some([0; 5])))
            || self.baseline.recorded_at > self.final_snapshot.recorded_at
            || self.unpriced_prior != self.baseline.counts
            || subtract(&self.final_snapshot.counts, &self.baseline.counts).as_ref()
                != Some(&self.observed_delta)
        {
            return Err(invalid());
        }
        Ok(())
    }
}

fn codex_counts(value: &Value) -> Option<NativeTokenCounts> {
    let counts = NativeTokenCounts::Codex {
        input: value.get("input_tokens")?.as_u64()?,
        cached_input: value.get("cached_input_tokens")?.as_u64()?,
        output: value.get("output_tokens")?.as_u64()?,
        reasoning_output: value.get("reasoning_output_tokens")?.as_u64()?,
        total: value.get("total_tokens")?.as_u64()?,
    };
    counts.validate_accounting().ok()?;
    Some(counts)
}

fn fields(value: &NativeTokenCounts) -> Option<[u64; 5]> {
    match *value {
        NativeTokenCounts::Codex {
            input,
            cached_input,
            output,
            reasoning_output,
            total,
        } => Some([input, cached_input, output, reasoning_output, total]),
        NativeTokenCounts::ClaudeCode { .. } => None,
    }
}

fn subtract(
    final_counts: &NativeTokenCounts,
    baseline: &NativeTokenCounts,
) -> Option<NativeTokenCounts> {
    let final_fields = fields(final_counts)?;
    let baseline_fields = fields(baseline)?;
    let mut delta = [0; 5];
    for ((result, final_value), baseline_value) in
        delta.iter_mut().zip(final_fields).zip(baseline_fields)
    {
        *result = final_value.checked_sub(baseline_value)?;
    }
    let counts = NativeTokenCounts::Codex {
        input: delta[0],
        cached_input: delta[1],
        output: delta[2],
        reasoning_output: delta[3],
        total: delta[4],
    };
    counts.validate_accounting().ok()?;
    Some(counts)
}

#[derive(Clone)]
struct Candidate {
    index: u64,
    timestamp: Option<DateTime<Utc>>,
    counts: NativeTokenCounts,
}

enum Declaration {
    Valid(u64, String),
    Invalid(u64),
}

/// Extract persisted evidence from one already-selected Codex JSONL file.
/// Session IDs are compared for continuity and discarded before return.
pub fn extract_codex_usage_evidence(bytes: &[u8]) -> Result<PersistedUsageEvidence> {
    if bytes.len() > MAX_SOURCE_BYTES {
        bail!("insights_usage_evidence_source_too_large");
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    let mut candidate_records = 0u64;
    let mut complete_records = 0u64;
    let mut duplicate_records = 0u64;
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut declarations = Vec::new();
    let mut session: Option<String> = None;
    let mut session_invalid = false;
    let mut multiple_sessions = false;
    let mut aggregate_problem = None;

    for (offset, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let index = u64::try_from(offset + 1).map_err(|_| invalid())?;
        let row: Value = serde_json::from_str(line).map_err(|_| invalid())?;
        let object = row.as_object().ok_or_else(invalid)?;
        let kind = object.get("type").and_then(Value::as_str);
        let payload = object.get("payload").filter(|value| value.is_object());
        if kind == Some("session_meta") {
            match payload
                .and_then(|payload| payload.get("id").and_then(Value::as_str))
                .filter(|id| !id.is_empty() && id.len() <= 256)
            {
                Some(id) if session.as_deref().is_none_or(|previous| previous == id) => {
                    session = Some(id.to_owned())
                }
                Some(_) => multiple_sessions = true,
                None => session_invalid = true,
            }
        }
        // Codex session metadata identifies the provider but does not declare a
        // model. The persisted turn context is the authoritative model scope.
        if kind == Some("turn_context") {
            declarations.push(
                match payload
                    .and_then(|payload| payload.get("model"))
                    .and_then(Value::as_str)
                {
                    Some(model) if safe_model(model) => Declaration::Valid(index, model.to_owned()),
                    _ => Declaration::Invalid(index),
                },
            );
        }
        if kind != Some("event_msg") {
            continue;
        }
        let Some(payload) = payload else {
            continue;
        };
        if payload.get("type").and_then(Value::as_str) != Some("token_count") {
            continue;
        }
        candidate_records = candidate_records.checked_add(1).ok_or_else(invalid)?;
        let Some(counts) = payload
            .pointer("/info/total_token_usage")
            .and_then(codex_counts)
        else {
            aggregate_problem.get_or_insert(AggregateUnavailableReason::IncompleteOrInvalidUsage);
            continue;
        };
        complete_records = complete_records.checked_add(1).ok_or_else(invalid)?;
        if let Some(previous) = candidates.last() {
            if subtract(&counts, &previous.counts).is_none() {
                aggregate_problem = Some(AggregateUnavailableReason::CumulativeReset);
            } else if counts == previous.counts {
                duplicate_records = duplicate_records.checked_add(1).ok_or_else(invalid)?;
            }
        }
        let timestamp = object
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        candidates.push(Candidate {
            index,
            timestamp,
            counts,
        });
    }

    if multiple_sessions {
        aggregate_problem = Some(AggregateUnavailableReason::MultipleSessions);
    } else if session_invalid || session.is_none() {
        aggregate_problem = Some(AggregateUnavailableReason::MissingOrInvalidSession);
    }
    if candidate_records == 0 {
        aggregate_problem = Some(AggregateUnavailableReason::NoUsage);
    } else if complete_records != candidate_records && aggregate_problem.is_none() {
        aggregate_problem = Some(AggregateUnavailableReason::IncompleteOrInvalidUsage);
    }
    let aggregate_counts = aggregate_problem
        .is_none()
        .then(|| candidates.last().map(|value| value.counts.clone()))
        .flatten();

    let (mut interval, mut interval_unavailable_reason) =
        interval(&candidates, aggregate_counts.as_ref());
    let mut model_attribution = attribution(&declarations, interval.as_ref());
    if interval
        .as_ref()
        .is_some_and(|value| value.baseline.record_index == value.final_snapshot.record_index)
        && !matches!(
            model_attribution,
            UsageModelAttribution::SingleDeclaredModel { .. }
        )
    {
        interval = None;
        interval_unavailable_reason = Some(IntervalUnavailableReason::InsufficientBaseline);
        model_attribution = attribution(&declarations, None);
    }
    let refs = candidates
        .iter()
        .take(MAX_USAGE_RECORD_REFS)
        .map(|value| UsageRecordRef {
            record_index: value.index,
        })
        .collect::<Vec<_>>();
    let omitted_record_refs = complete_records
        .checked_sub(u64::try_from(refs.len()).map_err(|_| invalid())?)
        .ok_or_else(invalid)?;
    let evidence = PersistedUsageEvidence {
        schema_version: USAGE_EVIDENCE_SCHEMA_VERSION,
        source: UsageSource::Codex,
        scope: UsageEvidenceScope::ExactSourceFile,
        source_format: SourceFormat::Codex,
        source_digest: format!("{:x}", Sha256::digest(bytes)),
        coordinates: UsageRecordCoordinates::JsonlPhysicalLinesOneBased,
        candidate_records,
        complete_records,
        incomplete_records: candidate_records
            .checked_sub(complete_records)
            .ok_or_else(invalid)?,
        duplicate_records,
        record_refs: refs,
        omitted_record_refs,
        aggregate_counts,
        aggregate_unavailable_reason: aggregate_problem,
        interval,
        interval_unavailable_reason,
        attribution: model_attribution,
    };
    evidence.validate()?;
    Ok(evidence)
}

fn interval(
    candidates: &[Candidate],
    aggregate: Option<&NativeTokenCounts>,
) -> (
    Option<ObservedUsageInterval>,
    Option<IntervalUnavailableReason>,
) {
    if aggregate.is_none() {
        return (None, Some(IntervalUnavailableReason::IncompleteAggregate));
    }
    let Some(baseline) = candidates.first() else {
        return (None, Some(IntervalUnavailableReason::InsufficientBaseline));
    };
    let final_snapshot = candidates.last().expect("baseline exists");
    if candidates.len() == 1 && fields(&baseline.counts) != Some([0; 5]) {
        return (None, Some(IntervalUnavailableReason::InsufficientBaseline));
    }
    let (Some(baseline_time), Some(final_time)) = (baseline.timestamp, final_snapshot.timestamp)
    else {
        return (
            None,
            Some(IntervalUnavailableReason::MissingOrInvalidTimestamp),
        );
    };
    if baseline_time > final_time {
        return (
            None,
            Some(IntervalUnavailableReason::MissingOrInvalidTimestamp),
        );
    }
    let Some(observed_delta) = subtract(&final_snapshot.counts, &baseline.counts) else {
        return (None, Some(IntervalUnavailableReason::Overflow));
    };
    (
        Some(ObservedUsageInterval {
            baseline: UsageCounterSnapshot {
                record_index: baseline.index,
                recorded_at: baseline_time,
                counts: baseline.counts.clone(),
            },
            final_snapshot: UsageCounterSnapshot {
                record_index: final_snapshot.index,
                recorded_at: final_time,
                counts: final_snapshot.counts.clone(),
            },
            observed_delta,
            unpriced_prior: baseline.counts.clone(),
        }),
        None,
    )
}

fn attribution(
    declarations: &[Declaration],
    interval: Option<&ObservedUsageInterval>,
) -> UsageModelAttribution {
    let Some(interval) = interval else {
        return UsageModelAttribution::Unavailable {
            reason: ModelAttributionUnavailableReason::NoDeclarationAtBaseline,
        };
    };
    // Counters already present at the baseline are explicitly unpriced. Use
    // the latest durable context at that boundary rather than treating older
    // model scopes as part of the observed interval.
    let baseline_declaration = declarations
        .iter()
        .rev()
        .find(|declaration| match declaration {
            Declaration::Valid(index, _) | Declaration::Invalid(index) => {
                *index <= interval.baseline.record_index
            }
        });
    let (baseline_index, model) = match baseline_declaration {
        Some(Declaration::Valid(index, model)) => (*index, model.as_str()),
        Some(Declaration::Invalid(_)) => {
            return UsageModelAttribution::Unavailable {
                reason: ModelAttributionUnavailableReason::MissingOrInvalidDeclaration,
            };
        }
        None => {
            return UsageModelAttribution::Unavailable {
                reason: ModelAttributionUnavailableReason::NoDeclarationAtBaseline,
            };
        }
    };
    for declaration in declarations.iter().filter(|declaration| match declaration {
        Declaration::Valid(index, _) | Declaration::Invalid(index) => {
            *index > baseline_index && *index <= interval.final_snapshot.record_index
        }
    }) {
        match declaration {
            Declaration::Invalid(_) => {
                return UsageModelAttribution::Unavailable {
                    reason: ModelAttributionUnavailableReason::MissingOrInvalidDeclaration,
                };
            }
            Declaration::Valid(_, value) => {
                if model != value {
                    return UsageModelAttribution::Unavailable {
                        reason: ModelAttributionUnavailableReason::ModelChanged,
                    };
                }
            }
        }
    }
    UsageModelAttribution::SingleDeclaredModel {
        model: model.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(value: Value) -> String {
        value.to_string()
    }

    fn source(records: Vec<Value>) -> Vec<u8> {
        records
            .into_iter()
            .map(line)
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes()
    }

    fn meta() -> Value {
        json!({"type":"session_meta","payload":{"id":"discard-me","model_provider":"openai"}})
    }

    fn context(model: Option<&str>) -> Value {
        let mut value = json!({"type":"turn_context","payload":{}});
        if let Some(model) = model {
            value["payload"]["model"] = json!(model);
        }
        value
    }

    fn assistant_message() -> Value {
        json!({
            "type":"response_item",
            "payload":{"type":"message","role":"assistant","content":[]}
        })
    }

    fn usage(at: Option<&str>, input: u64, cached: u64, output: u64, reasoning: u64) -> Value {
        let mut value = json!({
            "type":"event_msg",
            "payload":{"type":"token_count","info":{"total_token_usage":{
                "input_tokens":input,"cached_input_tokens":cached,"output_tokens":output,
                "reasoning_output_tokens":reasoning,"total_tokens":input + output
            }}}
        });
        if let Some(at) = at {
            value["timestamp"] = json!(at);
        }
        value
    }

    #[test]
    fn cumulative_total_and_observed_delta_are_separate() {
        let bytes = source(vec![
            meta(),
            meta(),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:00:00Z"), 100, 60, 20, 10),
            assistant_message(),
            usage(Some("2026-01-01T00:01:00Z"), 180, 100, 50, 20),
        ]);
        let evidence = extract_codex_usage_evidence(&bytes).unwrap();
        assert_eq!(evidence.candidate_records, 2);
        assert_eq!(evidence.complete_records, 2);
        assert_eq!(evidence.incomplete_records, 0);
        assert_eq!(evidence.duplicate_records, 0);
        assert_eq!(evidence.aggregate_counts, Some(codex(180, 100, 50, 20)));
        let interval = evidence.interval.as_ref().unwrap();
        assert_eq!(interval.unpriced_prior, codex(100, 60, 20, 10));
        assert_eq!(interval.observed_delta, codex(80, 40, 30, 10));
        assert!(evidence.has_attributed_interval());
        evidence
            .validate_binding(
                SourceFormat::Codex,
                &format!("{:x}", Sha256::digest(&bytes)),
            )
            .unwrap();
    }

    #[test]
    fn sole_nonzero_snapshot_never_infers_prior_history() {
        let evidence = extract_codex_usage_evidence(&source(vec![
            meta(),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:00:00Z"), 10, 2, 4, 1),
        ]))
        .unwrap();
        assert!(evidence.aggregate_counts.is_some());
        assert!(evidence.interval.is_none());
        assert_eq!(
            evidence.interval_unavailable_reason,
            Some(IntervalUnavailableReason::InsufficientBaseline)
        );
        assert!(!evidence.has_attributed_interval());
    }

    #[test]
    fn sole_zero_requires_valid_time_session_and_model() {
        let ready = extract_codex_usage_evidence(&source(vec![
            meta(),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:00:00Z"), 0, 0, 0, 0),
        ]))
        .unwrap();
        assert!(ready.has_attributed_interval());

        let missing_model = extract_codex_usage_evidence(&source(vec![
            meta(),
            usage(Some("2026-01-01T00:00:00Z"), 0, 0, 0, 0),
        ]))
        .unwrap();
        assert!(missing_model.interval.is_none());
        assert!(!missing_model.has_attributed_interval());
    }

    #[test]
    fn missing_metadata_or_counters_are_typed_missingness() {
        let no_usage = extract_codex_usage_evidence(&source(vec![
            json!({"type":"session_meta","payload":{}}),
            json!({"type":"other"}),
        ]))
        .unwrap();
        assert_eq!(
            no_usage.aggregate_unavailable_reason,
            Some(AggregateUnavailableReason::NoUsage)
        );

        let incomplete = extract_codex_usage_evidence(&source(vec![
            meta(),
            json!({"type":"event_msg","payload":{"type":"token_count","info":{}}}),
        ]))
        .unwrap();
        assert_eq!(incomplete.candidate_records, 1);
        assert_eq!(incomplete.incomplete_records, 1);
        assert_eq!(
            incomplete.aggregate_unavailable_reason,
            Some(AggregateUnavailableReason::IncompleteOrInvalidUsage)
        );

        assert!(extract_codex_usage_evidence(b"[]\n").is_err());
        assert!(extract_codex_usage_evidence(b"not-json\n").is_err());
    }

    #[test]
    fn timestamp_or_model_ambiguity_preserves_usable_aggregate() {
        let missing_time = extract_codex_usage_evidence(&source(vec![
            meta(),
            context(Some("gpt-6")),
            usage(None, 0, 0, 0, 0),
            usage(Some("2026-01-01T00:01:00Z"), 10, 2, 4, 1),
        ]))
        .unwrap();
        assert_eq!(missing_time.aggregate_counts, Some(codex(10, 2, 4, 1)));
        assert_eq!(
            missing_time.interval_unavailable_reason,
            Some(IntervalUnavailableReason::MissingOrInvalidTimestamp)
        );

        let switched = extract_codex_usage_evidence(&source(vec![
            meta(),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:00:00Z"), 0, 0, 0, 0),
            json!({"type":"turn_context","payload":{"model":"gpt-6-mini"}}),
            usage(Some("2026-01-01T00:01:00Z"), 10, 2, 4, 1),
        ]))
        .unwrap();
        assert!(switched.aggregate_counts.is_some());
        assert_eq!(
            switched.attribution,
            UsageModelAttribution::Unavailable {
                reason: ModelAttributionUnavailableReason::ModelChanged
            }
        );
    }

    #[test]
    fn latest_context_at_baseline_owns_only_the_observed_delta() {
        let evidence = extract_codex_usage_evidence(&source(vec![
            meta(),
            context(Some("gpt-5")),
            context(None),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:00:00Z"), 100, 60, 20, 10),
            assistant_message(),
            usage(Some("2026-01-01T00:01:00Z"), 180, 100, 50, 20),
        ]))
        .unwrap();
        assert_eq!(
            evidence.attribution,
            UsageModelAttribution::SingleDeclaredModel {
                model: "gpt-6".to_owned()
            }
        );
        assert_eq!(
            evidence.interval.unwrap().unpriced_prior,
            codex(100, 60, 20, 10)
        );
    }

    #[test]
    fn latest_invalid_or_late_context_cannot_establish_baseline() {
        let invalid_baseline = extract_codex_usage_evidence(&source(vec![
            meta(),
            context(Some("gpt-6")),
            context(None),
            usage(Some("2026-01-01T00:00:00Z"), 0, 0, 0, 0),
            usage(Some("2026-01-01T00:01:00Z"), 10, 2, 4, 1),
        ]))
        .unwrap();
        assert_eq!(
            invalid_baseline.attribution,
            UsageModelAttribution::Unavailable {
                reason: ModelAttributionUnavailableReason::MissingOrInvalidDeclaration
            }
        );

        let late = extract_codex_usage_evidence(&source(vec![
            meta(),
            usage(Some("2026-01-01T00:00:00Z"), 0, 0, 0, 0),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:01:00Z"), 10, 2, 4, 1),
        ]))
        .unwrap();
        assert_eq!(
            late.attribution,
            UsageModelAttribution::Unavailable {
                reason: ModelAttributionUnavailableReason::NoDeclarationAtBaseline
            }
        );
    }

    #[test]
    fn later_context_must_be_valid_and_unchanged() {
        let malformed = extract_codex_usage_evidence(&source(vec![
            meta(),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:00:00Z"), 0, 0, 0, 0),
            context(None),
            usage(Some("2026-01-01T00:01:00Z"), 10, 2, 4, 1),
        ]))
        .unwrap();
        assert_eq!(
            malformed.attribution,
            UsageModelAttribution::Unavailable {
                reason: ModelAttributionUnavailableReason::MissingOrInvalidDeclaration
            }
        );

        let unchanged = extract_codex_usage_evidence(&source(vec![
            meta(),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:00:00Z"), 0, 0, 0, 0),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:01:00Z"), 10, 2, 4, 1),
        ]))
        .unwrap();
        assert!(unchanged.has_attributed_interval());
    }

    #[test]
    fn duplicate_and_reference_omission_are_coverage_not_semantic_loss() {
        let mut records = vec![
            meta(),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:00:00Z"), 0, 0, 0, 0),
        ];
        for second in 1..=70 {
            records.push(usage(
                Some(&format!("2026-01-01T00:{:02}:00Z", second.min(59))),
                second,
                0,
                second,
                0,
            ));
        }
        let evidence = extract_codex_usage_evidence(&source(records)).unwrap();
        assert_eq!(evidence.record_refs.len(), MAX_USAGE_RECORD_REFS);
        assert_eq!(evidence.omitted_record_refs, 7);
        assert!(evidence.has_attributed_interval());

        let duplicate = extract_codex_usage_evidence(&source(vec![
            meta(),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:00:00Z"), 0, 0, 0, 0),
            usage(Some("2026-01-01T00:01:00Z"), 0, 0, 0, 0),
        ]))
        .unwrap();
        assert_eq!(duplicate.duplicate_records, 1);
    }

    #[test]
    fn invalid_delta_subset_and_reset_fail_closed() {
        // Endpoints are each valid, but cached input grows faster than input.
        let evidence = extract_codex_usage_evidence(&source(vec![
            meta(),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:00:00Z"), 10, 0, 2, 0),
            usage(Some("2026-01-01T00:01:00Z"), 11, 2, 3, 0),
        ]))
        .unwrap();
        assert_eq!(
            evidence.aggregate_unavailable_reason,
            Some(AggregateUnavailableReason::CumulativeReset)
        );
        assert!(evidence.aggregate_counts.is_none());

        let reset = extract_codex_usage_evidence(&source(vec![
            meta(),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:00:00Z"), 10, 2, 4, 1),
            usage(Some("2026-01-01T00:01:00Z"), 9, 2, 4, 1),
        ]))
        .unwrap();
        assert_eq!(
            reset.aggregate_unavailable_reason,
            Some(AggregateUnavailableReason::CumulativeReset)
        );

        // Codex may synthesize a full context-window token count with only the
        // total populated. It is not complete accounting for pricing.
        let synthetic = extract_codex_usage_evidence(&source(vec![
            meta(),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:00:00Z"), 0, 0, 0, 0),
            json!({
                "type":"event_msg",
                "timestamp":"2026-01-01T00:01:00Z",
                "payload":{"type":"token_count","info":{"total_token_usage":{
                    "input_tokens":0,"cached_input_tokens":0,"output_tokens":0,
                    "reasoning_output_tokens":0,"total_tokens":128000
                }}}
            }),
        ]))
        .unwrap();
        assert_eq!(
            synthetic.aggregate_unavailable_reason,
            Some(AggregateUnavailableReason::IncompleteOrInvalidUsage)
        );
        assert!(synthetic.interval.is_none());
    }

    #[test]
    fn malformed_persisted_relationships_are_rejected_without_panicking() {
        let valid = extract_codex_usage_evidence(&source(vec![
            meta(),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:00:00Z"), 0, 0, 0, 0),
        ]))
        .unwrap();

        let mut evidence = valid.clone();
        evidence.incomplete_records = 1;
        assert_eq!(
            evidence.validate().unwrap_err().to_string(),
            "insights_usage_evidence_invalid"
        );

        let mut evidence = valid.clone();
        evidence.interval.as_mut().unwrap().observed_delta = codex(1, 0, 0, 0);
        assert!(evidence.validate().is_err());

        let mut evidence = valid.clone();
        evidence.candidate_records = 1;
        evidence.complete_records = 0;
        evidence.incomplete_records = 1;
        evidence.record_refs.clear();
        assert!(evidence.validate().is_err());

        let mut evidence = valid.clone();
        let interval = evidence.interval.as_mut().unwrap();
        interval.baseline.counts = codex(1, 0, 0, 0);
        interval.final_snapshot.counts = codex(1, 0, 0, 0);
        interval.unpriced_prior = codex(1, 0, 0, 0);
        evidence.aggregate_counts = Some(codex(1, 0, 0, 0));
        assert!(evidence.validate().is_err());

        let mut fabricated_endpoint = valid.clone();
        fabricated_endpoint.aggregate_counts = Some(codex(10, 0, 0, 0));
        let interval = fabricated_endpoint.interval.as_mut().unwrap();
        interval.final_snapshot.record_index += 1;
        interval.final_snapshot.counts = codex(10, 0, 0, 0);
        interval.observed_delta = codex(10, 0, 0, 0);
        assert!(fabricated_endpoint.validate().is_err());

        let mut wrong_accounting = extract_codex_usage_evidence(&source(vec![
            meta(),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:00:00Z"), 10, 2, 4, 1),
        ]))
        .unwrap();
        wrong_accounting.aggregate_counts = Some(NativeTokenCounts::ClaudeCode {
            input: 10,
            cache_read_input: 2,
            cache_creation_input: 0,
            output: 4,
        });
        assert!(wrong_accounting.validate().is_err());
    }

    #[test]
    fn serialized_evidence_contains_no_native_id_path_or_body() {
        let evidence = extract_codex_usage_evidence(&source(vec![
            meta(),
            context(Some("gpt-6")),
            usage(Some("2026-01-01T00:00:00Z"), 0, 0, 0, 0),
        ]))
        .unwrap();
        let json = serde_json::to_string(&evidence).unwrap();
        assert!(!json.contains("discard-me"));
        assert!(!json.contains("session_id"));
        assert!(!json.contains("path"));
        assert!(!json.contains("payload"));
    }

    fn codex(
        input: u64,
        cached_input: u64,
        output: u64,
        reasoning_output: u64,
    ) -> NativeTokenCounts {
        NativeTokenCounts::Codex {
            input,
            cached_input,
            output,
            reasoning_output,
            total: input + output,
        }
    }
}
