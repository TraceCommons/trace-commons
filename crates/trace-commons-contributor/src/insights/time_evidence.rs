//! Bounded evidence about timestamps explicitly recorded on supported events.
//!
//! This module does not infer elapsed or active time. In particular, it never
//! substitutes an import time for a missing event timestamp. Callers validate
//! the selected bytes with the source adapter before persisting this evidence.

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::SourceFormat;

pub const MAX_EXTREMUM_EVENT_REFS: usize = 16;
const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecordedTimeEvidence {
    pub schema_version: u32,
    pub scope: TimeEvidenceScope,
    pub source_format: SourceFormat,
    pub source_digest: String,
    pub coordinates: TimeRecordCoordinates,
    /// Adapter-supported event records, excluding source metadata records.
    pub total_eligible_records: u64,
    pub valid_timestamps: u64,
    pub missing_timestamps: u64,
    pub invalid_timestamps: u64,
    pub earliest: Option<TimestampExtremum>,
    pub latest: Option<TimestampExtremum>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimeEvidenceScope {
    RecordedEventTimestampsOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimeRecordCoordinates {
    JsonlPhysicalLinesOneBased,
    TrajectoryArrayIndexesZeroBased,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TimestampExtremum {
    pub recorded_at: DateTime<Utc>,
    pub event_refs: Vec<TimestampEventRef>,
    /// Additional equal-time records beyond the retained reference bound.
    pub omitted_event_refs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TimestampEventRef {
    pub record_index: u64,
}

fn invalid() -> anyhow::Error {
    anyhow!("insights-time-evidence-invalid")
}

impl RecordedTimeEvidence {
    /// Structural cache validation. A containing snapshot must additionally
    /// compare the source format and digest with its own evidence binding.
    pub fn validate(&self) -> Result<()> {
        let classified = self
            .valid_timestamps
            .checked_add(self.missing_timestamps)
            .and_then(|n| n.checked_add(self.invalid_timestamps));
        if self.schema_version != 1
            || self.source_digest.len() != 64
            || !self
                .source_digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.total_eligible_records > MAX_SOURCE_BYTES as u64
            || classified != Some(self.total_eligible_records)
            || (self.valid_timestamps == 0) != self.earliest.is_none()
            || (self.valid_timestamps == 0) != self.latest.is_none()
            || (matches!(
                self.source_format,
                SourceFormat::Codex | SourceFormat::ClaudeCode
            ) && self.coordinates != TimeRecordCoordinates::JsonlPhysicalLinesOneBased)
        {
            return Err(invalid());
        }
        let (Some(earliest), Some(latest)) = (&self.earliest, &self.latest) else {
            return Ok(());
        };
        let earliest_count = extremum_count(earliest);
        let latest_count = extremum_count(latest);
        let equal = earliest.recorded_at == latest.recorded_at;
        let distinct_count = earliest_count
            .zip(latest_count)
            .and_then(|(earliest, latest)| earliest.checked_add(latest));
        if earliest.recorded_at > latest.recorded_at
            || !valid_extremum(earliest, self.coordinates, self.total_eligible_records)
            || !valid_extremum(latest, self.coordinates, self.total_eligible_records)
            || earliest_count.is_none_or(|count| count > self.valid_timestamps)
            || latest_count.is_none_or(|count| count > self.valid_timestamps)
            || (equal && (earliest != latest || earliest_count != Some(self.valid_timestamps)))
            || (!equal
                && (distinct_count.is_none_or(|count| count > self.valid_timestamps)
                    || earliest.event_refs.iter().any(|left| {
                        latest
                            .event_refs
                            .iter()
                            .any(|right| left.record_index == right.record_index)
                    })))
        {
            return Err(invalid());
        }
        Ok(())
    }
}

fn extremum_count(extremum: &TimestampExtremum) -> Option<u64> {
    (extremum.event_refs.len() as u64).checked_add(extremum.omitted_event_refs)
}

fn valid_extremum(
    extremum: &TimestampExtremum,
    coordinates: TimeRecordCoordinates,
    total_eligible_records: u64,
) -> bool {
    !extremum.event_refs.is_empty()
        && extremum.event_refs.len() <= MAX_EXTREMUM_EVENT_REFS
        && (extremum.omitted_event_refs == 0
            || extremum.event_refs.len() == MAX_EXTREMUM_EVENT_REFS)
        && extremum
            .event_refs
            .windows(2)
            .all(|refs| refs[0].record_index < refs[1].record_index)
        && extremum
            .event_refs
            .iter()
            .all(|reference| match coordinates {
                TimeRecordCoordinates::JsonlPhysicalLinesOneBased => {
                    reference.record_index > 0 && reference.record_index <= MAX_SOURCE_BYTES as u64
                }
                TimeRecordCoordinates::TrajectoryArrayIndexesZeroBased => {
                    reference.record_index > 0 && reference.record_index <= total_eligible_records
                }
            })
}

/// Extract timestamp coverage from adapter-supported event records. Timestamp
/// defects remain distinct counts. Invalid JSON or record envelopes fail with
/// a fixed label and never include source content in the error.
pub fn extract_recorded_time_evidence(
    source: SourceFormat,
    bytes: &[u8],
) -> Result<RecordedTimeEvidence> {
    if bytes.len() > MAX_SOURCE_BYTES {
        bail!("insights-time-source-too-large");
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    let is_array = source == SourceFormat::Trajectory && text.trim_start().starts_with('[');
    let coordinates = if is_array {
        TimeRecordCoordinates::TrajectoryArrayIndexesZeroBased
    } else {
        TimeRecordCoordinates::JsonlPhysicalLinesOneBased
    };
    let mut result = RecordedTimeEvidence {
        schema_version: 1,
        scope: TimeEvidenceScope::RecordedEventTimestampsOnly,
        source_format: source,
        source_digest: format!("{:x}", Sha256::digest(bytes)),
        coordinates,
        total_eligible_records: 0,
        valid_timestamps: 0,
        missing_timestamps: 0,
        invalid_timestamps: 0,
        earliest: None,
        latest: None,
    };

    if is_array {
        let records: Vec<Value> = serde_json::from_str(text).map_err(|_| invalid())?;
        observe_trajectory_records(
            &mut result,
            records.iter().enumerate().map(|(i, v)| (i as u64, v)),
        )?;
    } else {
        let mut records = Vec::new();
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let record = serde_json::from_str(line).map_err(|_| invalid())?;
            records.push((index as u64 + 1, record));
        }
        match source {
            SourceFormat::ClaudeCode => return Err(invalid()),
            SourceFormat::Codex => {
                observe_codex_records(&mut result, records.iter().map(|(i, v)| (*i, v)))?
            }
            SourceFormat::Trajectory => {
                observe_trajectory_records(&mut result, records.iter().map(|(i, v)| (*i, v)))?
            }
        }
    }
    result.validate()?;
    Ok(result)
}

fn observe_codex_records<'a>(
    result: &mut RecordedTimeEvidence,
    records: impl Iterator<Item = (u64, &'a Value)>,
) -> Result<()> {
    let mut saw_session_meta = false;
    for (index, record) in records {
        let object = record.as_object().ok_or_else(invalid)?;
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        match kind {
            "session_meta" => {
                if saw_session_meta {
                    return Err(invalid());
                }
                saw_session_meta = true;
            }
            "turn_context" => {}
            _ => observe_timestamp(result, index, object.get("timestamp")),
        }
    }
    if !saw_session_meta {
        return Err(invalid());
    }
    Ok(())
}

fn observe_trajectory_records<'a>(
    result: &mut RecordedTimeEvidence,
    mut records: impl Iterator<Item = (u64, &'a Value)>,
) -> Result<()> {
    let Some((_, meta)) = records.next() else {
        return Err(invalid());
    };
    if meta.get("role").and_then(Value::as_str) != Some("meta") {
        return Err(invalid());
    }
    for (index, record) in records {
        let role = record
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        if !matches!(
            role,
            "user" | "assistant" | "reasoning" | "tool" | "system" | "observation"
        ) {
            return Err(invalid());
        }
        observe_timestamp(result, index, record.get("timestamp"));
    }
    Ok(())
}

fn observe_timestamp(result: &mut RecordedTimeEvidence, index: u64, value: Option<&Value>) {
    result.total_eligible_records += 1;
    let raw = match value {
        None | Some(Value::Null) => {
            result.missing_timestamps += 1;
            return;
        }
        Some(Value::String(raw)) => raw,
        Some(_) => {
            result.invalid_timestamps += 1;
            return;
        }
    };
    let Ok(parsed) = DateTime::parse_from_rfc3339(raw) else {
        result.invalid_timestamps += 1;
        return;
    };
    result.valid_timestamps += 1;
    let recorded_at = parsed.with_timezone(&Utc);
    update_extremum(&mut result.earliest, recorded_at, index, true);
    update_extremum(&mut result.latest, recorded_at, index, false);
}

fn update_extremum(
    extremum: &mut Option<TimestampExtremum>,
    recorded_at: DateTime<Utc>,
    record_index: u64,
    earliest: bool,
) {
    let replace = extremum.as_ref().is_none_or(|current| {
        if earliest {
            recorded_at < current.recorded_at
        } else {
            recorded_at > current.recorded_at
        }
    });
    if replace {
        *extremum = Some(TimestampExtremum {
            recorded_at,
            event_refs: vec![TimestampEventRef { record_index }],
            omitted_event_refs: 0,
        });
    } else if extremum
        .as_ref()
        .is_some_and(|current| current.recorded_at == recorded_at)
    {
        let current = extremum.as_mut().expect("checked above");
        if current.event_refs.len() < MAX_EXTREMUM_EVENT_REFS {
            current.event_refs.push(TimestampEventRef { record_index });
        } else {
            current.omitted_event_refs += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn jsonl(records: &[Value]) -> Vec<u8> {
        records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes()
    }

    #[test]
    fn codex_keeps_missing_invalid_and_out_of_order_extrema_distinct() {
        let bytes = jsonl(&[
            json!({"type":"session_meta","timestamp":"2099-01-01T00:00:00Z","payload":{}}),
            json!({"type":"response_item","timestamp":"2026-09-11T01:00:00+01:00","payload":{}}),
            json!({"type":"event_msg","payload":{}}),
            json!({"type":"response_item","timestamp":0,"payload":{}}),
            json!({"type":"response_item","timestamp":"malformed","payload":{}}),
            json!({"type":"response_item","timestamp":"1970-01-01T00:00:00Z","payload":{}}),
        ]);
        let evidence = extract_recorded_time_evidence(SourceFormat::Codex, &bytes).unwrap();
        assert_eq!(
            (
                evidence.total_eligible_records,
                evidence.valid_timestamps,
                evidence.missing_timestamps,
                evidence.invalid_timestamps
            ),
            (5, 2, 1, 2)
        );
        assert_eq!(
            evidence.earliest.as_ref().unwrap().recorded_at.timestamp(),
            0
        );
        assert_eq!(
            evidence.earliest.as_ref().unwrap().event_refs[0].record_index,
            6
        );
        assert_eq!(
            evidence.latest.as_ref().unwrap().recorded_at.timestamp(),
            1_789_084_800
        );
        assert_eq!(
            evidence.latest.as_ref().unwrap().event_refs[0].record_index,
            2
        );
    }

    #[test]
    fn trajectory_array_and_jsonl_preserve_their_coordinate_conventions() {
        let records = vec![
            json!({"role":"meta","source":"test"}),
            json!({"role":"user","content":"x","timestamp":"1970-01-01T00:00:00Z"}),
        ];
        let array = serde_json::to_vec(&records).unwrap();
        let array_result =
            extract_recorded_time_evidence(SourceFormat::Trajectory, &array).unwrap();
        assert_eq!(
            array_result.coordinates,
            TimeRecordCoordinates::TrajectoryArrayIndexesZeroBased
        );
        assert_eq!(array_result.earliest.unwrap().event_refs[0].record_index, 1);
        let lines_result =
            extract_recorded_time_evidence(SourceFormat::Trajectory, &jsonl(&records)).unwrap();
        assert_eq!(
            lines_result.coordinates,
            TimeRecordCoordinates::JsonlPhysicalLinesOneBased
        );
        assert_eq!(lines_result.earliest.unwrap().event_refs[0].record_index, 2);
    }

    #[test]
    fn resumed_equal_timestamps_are_bounded_without_deduplication() {
        let mut records = vec![json!({"type":"session_meta","payload":{}})];
        records.extend((0..MAX_EXTREMUM_EVENT_REFS + 3).map(
            |_| json!({"type":"response_item","timestamp":"2026-09-11T00:00:00Z","payload":{}}),
        ));
        let evidence =
            extract_recorded_time_evidence(SourceFormat::Codex, &jsonl(&records)).unwrap();
        for extremum in [evidence.earliest.unwrap(), evidence.latest.unwrap()] {
            assert_eq!(extremum.event_refs.len(), MAX_EXTREMUM_EVENT_REFS);
            assert_eq!(extremum.omitted_event_refs, 3);
        }
    }

    #[test]
    fn no_valid_timestamp_does_not_infer_one() {
        // The production trajectory adapter currently rejects these defects.
        // This fixture pins the evidence contract independently; callers must
        // still prevalidate a source and must not weaken that adapter.
        let bytes = jsonl(&[
            json!({"role":"meta","source":"test"}),
            json!({"role":"user","content":"x"}),
            json!({"role":"assistant","content":"y","timestamp":"bad"}),
        ]);
        let evidence = extract_recorded_time_evidence(SourceFormat::Trajectory, &bytes).unwrap();
        assert_eq!(
            (evidence.missing_timestamps, evidence.invalid_timestamps),
            (1, 1)
        );
        assert!(evidence.earliest.is_none());
        assert!(evidence.latest.is_none());
    }

    #[test]
    fn metadata_only_source_has_zero_eligible_records() {
        let evidence = extract_recorded_time_evidence(
            SourceFormat::Codex,
            &jsonl(&[json!({
                "type":"session_meta",
                "timestamp":"2026-09-11T00:00:00Z",
                "payload":{}
            })]),
        )
        .unwrap();
        assert_eq!(evidence.total_eligible_records, 0);
        assert!(evidence.earliest.is_none());
        assert!(evidence.latest.is_none());
    }

    #[test]
    fn malformed_envelopes_fail_with_a_fixed_label() {
        let error = extract_recorded_time_evidence(SourceFormat::Codex, b"{private-content")
            .unwrap_err()
            .to_string();
        assert_eq!(error, "insights-time-evidence-invalid");
    }

    #[test]
    fn validation_rejects_inconsistent_cached_counts_coordinates_and_refs() {
        let bytes = jsonl(&[
            json!({"type":"session_meta","payload":{}}),
            json!({"type":"response_item","timestamp":"2026-09-11T00:00:00Z","payload":{}}),
        ]);
        let evidence = extract_recorded_time_evidence(SourceFormat::Codex, &bytes).unwrap();
        let mut bad_count = evidence.clone();
        bad_count.missing_timestamps = 1;
        assert!(bad_count.validate().is_err());
        let mut bad_coordinates = evidence.clone();
        bad_coordinates.coordinates = TimeRecordCoordinates::TrajectoryArrayIndexesZeroBased;
        assert!(bad_coordinates.validate().is_err());
        let mut bad_ref = evidence;
        bad_ref.earliest.as_mut().unwrap().event_refs[0].record_index = 0;
        assert!(bad_ref.validate().is_err());
    }

    #[test]
    fn validation_rejects_overflow_incomplete_equal_extrema_and_overlapping_distinct_refs() {
        let bytes = jsonl(&[
            json!({"type":"session_meta","payload":{}}),
            json!({"type":"response_item","timestamp":"2026-09-11T00:00:00Z","payload":{}}),
            json!({"type":"response_item","timestamp":"2026-09-11T00:00:01Z","payload":{}}),
        ]);
        let evidence = extract_recorded_time_evidence(SourceFormat::Codex, &bytes).unwrap();
        let mut overflow = evidence.clone();
        overflow.earliest.as_mut().unwrap().omitted_event_refs = u64::MAX;
        assert!(overflow.validate().is_err());
        let mut overlap = evidence;
        overlap.latest.as_mut().unwrap().event_refs[0].record_index = 2;
        assert!(overlap.validate().is_err());

        let same = jsonl(&[
            json!({"type":"session_meta","payload":{}}),
            json!({"type":"response_item","timestamp":"2026-09-11T00:00:00Z","payload":{}}),
            json!({"type":"response_item","timestamp":"2026-09-11T00:00:00Z","payload":{}}),
        ]);
        let mut incomplete = extract_recorded_time_evidence(SourceFormat::Codex, &same).unwrap();
        incomplete.earliest.as_mut().unwrap().event_refs.pop();
        incomplete.latest = incomplete.earliest.clone();
        assert!(incomplete.validate().is_err());
    }
}
