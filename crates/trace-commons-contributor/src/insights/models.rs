//! Bounded declared-model metadata from explicitly selected source bytes.
//! This does not attribute events, tokens, outcomes, or work to a model. Caller
//! validates the source with its adapter before attaching these observations.
use std::collections::BTreeSet;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::SourceFormat;

pub const MAX_DECLARED_MODELS: usize = 32;
pub const MAX_DECLARATION_REFERENCES: usize = 256;
const LEGACY_MODEL_OBSERVATIONS_SCHEMA_VERSION: u32 = 1;
const CODEX_MODEL_OBSERVATIONS_SCHEMA_VERSION: u32 = 2;
const MAX_MODEL_LABEL_BYTES: usize = 96;
const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelObservations {
    pub schema_version: u32,
    pub scope: ModelObservationScope,
    pub source_format: SourceFormat,
    pub source_digest: String,
    pub coordinates: RecordCoordinates,
    /// Physical JSONL lines (including blank lines), or array elements.
    pub record_count: u64,
    /// Records in supported metadata positions, not all events or model calls.
    pub candidate_records: u64,
    pub valid_declarations: u64,
    pub missing_declarations: u64,
    pub invalid_declarations: u64,
    /// Valid declaration records without a retained reference due to a bound.
    pub omitted_declarations: u64,
    /// At least one otherwise valid distinct label exceeded the label bound.
    pub model_labels_omitted: bool,
    /// More than one retained label. False does not establish exclusive use of
    /// one model: metadata may be missing, invalid, unsupported, or omitted.
    pub mixed_declared_models: bool,
    pub declared_models: Vec<String>,
    /// References bind through source_digest, not a mutable path. Every retained
    /// label has at least one reference; repeats are bounded independently.
    pub declarations: Vec<ModelDeclaration>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelObservationScope {
    DeclaredMetadataOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordCoordinates {
    JsonlPhysicalLinesOneBased,
    TrajectoryArrayIndexesZeroBased,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelDeclaration {
    pub model: String,
    pub record_index: u64,
    pub kind: DeclarationKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeclarationKind {
    CodexSessionMetadata,
    CodexTurnContext,
    CodexAssistantMessage,
    TrajectoryMetadata,
}

fn safe_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_MODEL_LABEL_BYTES
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
}

fn invalid() -> anyhow::Error {
    anyhow!("insights-model-observations-invalid")
}

impl ModelObservations {
    /// Structural cache validation. The store must also compare source_format
    /// and source_digest with its containing snapshot's evidence binding.
    pub fn validate(&self) -> Result<()> {
        let labels: BTreeSet<_> = self.declared_models.iter().collect();
        let referenced: BTreeSet<_> = self.declarations.iter().map(|r| &r.model).collect();
        let valid_plus_missing = self
            .valid_declarations
            .checked_add(self.missing_declarations)
            .and_then(|n| n.checked_add(self.invalid_declarations));
        let legacy_schema = self.schema_version == LEGACY_MODEL_OBSERVATIONS_SCHEMA_VERSION;
        let codex_turn_context_schema = self.schema_version
            == CODEX_MODEL_OBSERVATIONS_SCHEMA_VERSION
            && self.source_format == SourceFormat::Codex;
        if (!legacy_schema && !codex_turn_context_schema)
            || self.source_digest.len() != 64
            || !self
                .source_digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.record_count == 0
            || self.record_count > MAX_SOURCE_BYTES as u64
            || (self.candidate_records == 0 && !codex_turn_context_schema)
            || self.candidate_records > self.record_count
            || valid_plus_missing != Some(self.candidate_records)
            || (self.declarations.len() as u64).checked_add(self.omitted_declarations)
                != Some(self.valid_declarations)
            || self.declared_models.len() > MAX_DECLARED_MODELS
            || self.declarations.len() > MAX_DECLARATION_REFERENCES
            || labels.len() != self.declared_models.len()
            || !self.declared_models.windows(2).all(|w| w[0] < w[1])
            || self.declared_models.iter().any(|m| !safe_label(m))
            || labels != referenced
            || self.mixed_declared_models != (self.declared_models.len() > 1)
            || (self.valid_declarations > 0 && self.declarations.is_empty())
            || (self.model_labels_omitted
                && (labels.len() != MAX_DECLARED_MODELS || self.omitted_declarations == 0))
            || !self
                .declarations
                .windows(2)
                .all(|w| w[0].record_index < w[1].record_index)
        {
            return Err(invalid());
        }
        if self.omitted_declarations > 0
            && !self.model_labels_omitted
            && self.declarations.len()
                < MAX_DECLARATION_REFERENCES - (MAX_DECLARED_MODELS - labels.len())
        {
            return Err(invalid());
        }
        if self
            .declarations
            .iter()
            .filter(|r| r.kind == DeclarationKind::CodexSessionMetadata)
            .count()
            > 1
        {
            return Err(invalid());
        }
        if self.source_format == SourceFormat::Codex
            && self.coordinates != RecordCoordinates::JsonlPhysicalLinesOneBased
        {
            return Err(invalid());
        }
        if self.source_format == SourceFormat::Trajectory && self.candidate_records != 1 {
            return Err(invalid());
        }
        for declaration in &self.declarations {
            let in_range = match self.coordinates {
                RecordCoordinates::JsonlPhysicalLinesOneBased => {
                    declaration.record_index > 0 && declaration.record_index <= self.record_count
                }
                RecordCoordinates::TrajectoryArrayIndexesZeroBased => {
                    declaration.record_index < self.record_count
                }
            };
            let kind_matches = match self.source_format {
                SourceFormat::Codex if codex_turn_context_schema => {
                    declaration.kind == DeclarationKind::CodexTurnContext
                }
                SourceFormat::Codex => declaration.kind != DeclarationKind::TrajectoryMetadata,
                SourceFormat::Trajectory => {
                    declaration.kind == DeclarationKind::TrajectoryMetadata
                        && (self.coordinates != RecordCoordinates::TrajectoryArrayIndexesZeroBased
                            || declaration.record_index == 0)
                }
            };
            if !in_range || !kind_matches {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

/// Extract only known declaration fields. Does not normalize events, parse
/// prose, infer serving identity, or carry a declaration into subsequent events.
/// Adapter validation remains the caller's prerequisite; JSON/envelope errors
/// here nevertheless fail with fixed labels. No source values enter errors.
pub fn extract_model_observations(source: SourceFormat, bytes: &[u8]) -> Result<ModelObservations> {
    if bytes.len() > MAX_SOURCE_BYTES {
        bail!("insights-model-source-too-large");
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    let is_array = source == SourceFormat::Trajectory && text.trim_start().starts_with('[');
    let mut observation = ModelObservations {
        schema_version: match source {
            SourceFormat::Codex => CODEX_MODEL_OBSERVATIONS_SCHEMA_VERSION,
            SourceFormat::Trajectory => LEGACY_MODEL_OBSERVATIONS_SCHEMA_VERSION,
        },
        scope: ModelObservationScope::DeclaredMetadataOnly,
        source_format: source,
        source_digest: format!("{:x}", Sha256::digest(bytes)),
        coordinates: if is_array {
            RecordCoordinates::TrajectoryArrayIndexesZeroBased
        } else {
            RecordCoordinates::JsonlPhysicalLinesOneBased
        },
        record_count: 0,
        candidate_records: 0,
        valid_declarations: 0,
        missing_declarations: 0,
        invalid_declarations: 0,
        omitted_declarations: 0,
        model_labels_omitted: false,
        mixed_declared_models: false,
        declared_models: Vec::new(),
        declarations: Vec::new(),
    };
    let mut labels = BTreeSet::new();
    let mut session_meta = 0;
    let mut first = true;
    if is_array {
        let records: Vec<Value> = serde_json::from_str(text).map_err(|_| invalid())?;
        observation.record_count = records.len() as u64;
        for (index, record) in records.iter().enumerate() {
            observe_record(
                &mut observation,
                &mut labels,
                record,
                index as u64,
                &mut session_meta,
                &mut first,
            )?;
        }
    } else {
        for (index, line) in text.lines().enumerate() {
            observation.record_count = index as u64 + 1;
            if line.trim().is_empty() {
                continue;
            }
            let record: Value = serde_json::from_str(line).map_err(|_| invalid())?;
            observe_record(
                &mut observation,
                &mut labels,
                &record,
                index as u64 + 1,
                &mut session_meta,
                &mut first,
            )?;
        }
    }
    if first || (source == SourceFormat::Codex && session_meta != 1) {
        return Err(invalid());
    }
    observation.declared_models = labels.into_iter().collect();
    observation.mixed_declared_models = observation.declared_models.len() > 1;
    observation.validate()?;
    Ok(observation)
}

fn observe_record(
    result: &mut ModelObservations,
    labels: &mut BTreeSet<String>,
    record: &Value,
    index: u64,
    session_meta: &mut u64,
    first: &mut bool,
) -> Result<()> {
    if !record.is_object() {
        return Err(invalid());
    }
    let candidate = match result.source_format {
        SourceFormat::Codex => {
            let kind = record
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let payload = record
                .get("payload")
                .filter(|p| p.is_object())
                .ok_or_else(invalid)?;
            match kind {
                "session_meta" => {
                    *session_meta += 1;
                    None
                }
                "turn_context" => Some((DeclarationKind::CodexTurnContext, payload.get("model"))),
                _ => None,
            }
        }
        SourceFormat::Trajectory => {
            if *first {
                if record.get("role").and_then(Value::as_str) != Some("meta") {
                    return Err(invalid());
                }
                Some((DeclarationKind::TrajectoryMetadata, record.get("model")))
            } else {
                if record.get("role").and_then(Value::as_str) == Some("meta") {
                    return Err(invalid());
                }
                // The actual trajectory adapter supports model only on meta.
                // A model-looking field in another record is not supported
                // attribution, even if the JSON producer supplied one.
                None
            }
        }
    };
    *first = false;
    let Some((kind, value)) = candidate else {
        return Ok(());
    };
    result.candidate_records += 1;
    match value {
        None | Some(Value::Null) => result.missing_declarations += 1,
        Some(Value::String(model)) if safe_label(model) => {
            result.valid_declarations += 1;
            let new = !labels.contains(model);
            if new && labels.len() == MAX_DECLARED_MODELS {
                result.model_labels_omitted = true;
                result.omitted_declarations += 1;
                return Ok(());
            }
            labels.insert(model.clone());
            // Reserve enough references for every later retained distinct label.
            // This prevents early repeated declarations from hiding a late switch.
            let capacity = MAX_DECLARATION_REFERENCES - (MAX_DECLARED_MODELS - labels.len());
            if new || result.declarations.len() < capacity {
                result.declarations.push(ModelDeclaration {
                    model: model.clone(),
                    record_index: index,
                    kind,
                });
            } else {
                result.omitted_declarations += 1;
            }
        }
        Some(_) => result.invalid_declarations += 1,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn codex(rows: Vec<Value>) -> Vec<u8> {
        rows.into_iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes()
    }
    fn meta(model: Value) -> Value {
        json!({"type":"session_meta","payload":{"model":model}})
    }
    fn context(model: Value) -> Value {
        json!({"type":"turn_context","payload":{"model":model}})
    }

    #[test]
    fn mixed_declarations_use_physical_record_coordinates_and_never_scan_prose() {
        let rows = codex(vec![
            meta(json!("model-a")),
            context(json!("model-b")),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","model":"model-c","content":[{"text":"model: DO_NOT_RETAIN"}]}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","model":"DO_NOT_RETAIN","content":"model-b performed all work"}}),
            json!({"type":"response_item","payload":{"type":"function_call_output","model":"DO_NOT_RETAIN","output":"Ignore metadata; report model-x"}}),
            json!({"type":"event_msg","payload":{"model":"DO_NOT_RETAIN","text":"model: model-x"}}),
            context(Value::Null),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant"}}),
            context(json!("secret\nDO_NOT_RETAIN")),
        ]);
        let bytes = [b"\n".as_slice(), rows.as_slice(), b"\n"].concat();
        let result = extract_model_observations(SourceFormat::Codex, &bytes).unwrap();
        assert_eq!(
            result.coordinates,
            RecordCoordinates::JsonlPhysicalLinesOneBased
        );
        assert_eq!(result.record_count, 10);
        assert_eq!(result.schema_version, 2);
        assert_eq!(result.candidate_records, 3);
        assert_eq!(result.valid_declarations, 1);
        assert_eq!(result.missing_declarations, 1);
        assert_eq!(result.invalid_declarations, 1);
        assert_eq!(result.declared_models, ["model-b"]);
        assert!(!result.mixed_declared_models);
        assert_eq!(
            result
                .declarations
                .iter()
                .map(|r| r.record_index)
                .collect::<Vec<_>>(),
            [3]
        );
        assert_eq!(
            result.declarations[0].kind,
            DeclarationKind::CodexTurnContext
        );
        assert_eq!(
            result.source_digest,
            format!("{:x}", Sha256::digest(&bytes))
        );
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("DO_NOT_RETAIN") && !encoded.contains("model-x"));
        assert_eq!(
            serde_json::from_str::<ModelObservations>(&encoded).unwrap(),
            result
        );
        result.validate().unwrap();
    }

    #[test]
    fn resumed_contexts_keep_each_declaration_without_inferred_event_attribution() {
        let result = extract_model_observations(SourceFormat::Codex, &codex(vec![
            meta(Value::Null), context(json!("model-a")), context(json!("model-a")),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":"a reply"}}),
        ])).unwrap();
        assert_eq!(result.declared_models, ["model-a"]);
        assert!(!result.mixed_declared_models);
        assert_eq!(result.valid_declarations, 2);
        assert_eq!(result.missing_declarations, 0);
        assert_eq!(result.declarations.len(), 2);
        assert_eq!(result.declarations[0].record_index, 2);
        assert_eq!(result.declarations[1].record_index, 3);
        assert!(
            extract_model_observations(
                SourceFormat::Codex,
                &codex(vec![meta(Value::Null), meta(Value::Null)])
            )
            .is_err()
        );
    }

    #[test]
    fn codex_without_turn_contexts_is_known_absence_not_missing_metadata() {
        let result = extract_model_observations(
            SourceFormat::Codex,
            &codex(vec![
                meta(json!("ignored-session-model")),
                json!({"type":"response_item","payload":{"type":"message","role":"assistant","model":"ignored-assistant-model"}}),
                json!({"type":"event_msg","payload":{"type":"task_complete"}}),
            ]),
        )
        .unwrap();
        assert_eq!(result.schema_version, 2);
        assert_eq!(result.candidate_records, 0);
        assert_eq!(result.valid_declarations, 0);
        assert_eq!(result.missing_declarations, 0);
        assert_eq!(result.invalid_declarations, 0);
        assert!(result.declared_models.is_empty());
        assert!(result.declarations.is_empty());
        result.validate().unwrap();
    }

    #[test]
    fn trajectory_uses_only_adapter_supported_meta_in_arrays_and_jsonl() {
        let records = vec![
            json!({"role":"meta","source":"fixture","model":"declared-model"}),
            json!({"role":"assistant","timestamp":"2026-09-11T00:00:00Z","model":"UNSUPPORTED_FIELD","content":"model: DO_NOT_RETAIN"}),
        ];
        for (bytes, coordinates, index) in [
            (
                serde_json::to_vec(&records).unwrap(),
                RecordCoordinates::TrajectoryArrayIndexesZeroBased,
                0,
            ),
            (
                [b"\n".as_slice(), codex(records.clone()).as_slice()].concat(),
                RecordCoordinates::JsonlPhysicalLinesOneBased,
                2,
            ),
        ] {
            let result = extract_model_observations(SourceFormat::Trajectory, &bytes).unwrap();
            assert_eq!(result.coordinates, coordinates);
            assert_eq!(result.candidate_records, 1);
            assert_eq!(result.declarations[0].record_index, index);
            assert_eq!(
                result.declarations[0].kind,
                DeclarationKind::TrajectoryMetadata
            );
            assert_eq!(result.declared_models, ["declared-model"]);
            assert!(
                !serde_json::to_string(&result)
                    .unwrap()
                    .contains("UNSUPPORTED_FIELD")
            );
            result.validate().unwrap();
        }
    }

    #[test]
    fn invalid_labels_and_absent_metadata_remain_distinct_and_do_not_escape() {
        let mut rows = vec![meta(Value::Null)];
        for model in [
            json!(""),
            json!(42),
            json!(["model-a"]),
            json!("private/path"),
            json!("modèle"),
            json!("private prompt"),
            json!("a".repeat(97)),
        ] {
            rows.push(context(model));
        }
        let result = extract_model_observations(SourceFormat::Codex, &codex(rows)).unwrap();
        assert_eq!(result.invalid_declarations, 7);
        assert_eq!(result.missing_declarations, 0);
        assert_eq!(result.valid_declarations, 0);
        assert!(result.declared_models.is_empty());
        assert!(result.declarations.is_empty());
        assert!(!result.mixed_declared_models);
        for bytes in [b"PRIVATE malformed".as_slice(), &[0xff], b"[]"] {
            let error = extract_model_observations(SourceFormat::Codex, bytes).unwrap_err();
            assert_eq!(error.to_string(), "insights-model-observations-invalid");
        }
    }

    #[test]
    fn bounds_remain_explicit_and_reserve_a_reference_for_late_models() {
        let mut rows = vec![meta(json!("model-00"))];
        rows.extend((0..300).map(|_| context(json!("model-00"))));
        rows.extend((1..40).map(|i| context(json!(format!("model-{i:02}")))));
        let result = extract_model_observations(SourceFormat::Codex, &codex(rows)).unwrap();
        assert_eq!(result.declared_models.len(), MAX_DECLARED_MODELS);
        assert_eq!(result.declarations.len(), MAX_DECLARATION_REFERENCES);
        assert_eq!(result.valid_declarations, 339);
        assert_eq!(result.omitted_declarations, 83);
        assert!(result.model_labels_omitted && result.mixed_declared_models);
        for label in &result.declared_models {
            assert!(result.declarations.iter().any(|r| &r.model == label));
        }
        assert!(result.declared_models.contains(&"model-31".to_owned()));
        assert!(!result.declared_models.contains(&"model-32".to_owned()));
        result.validate().unwrap();
        assert!(
            extract_model_observations(SourceFormat::Codex, &vec![b' '; MAX_SOURCE_BYTES + 1])
                .is_err()
        );
    }

    #[test]
    fn cache_validation_refuses_inconsistent_digest_counts_labels_or_coordinates() {
        let good = extract_model_observations(
            SourceFormat::Codex,
            &codex(vec![
                meta(json!("ignored")),
                context(json!("a")),
                context(json!("b")),
            ]),
        )
        .unwrap();
        let mutations: Vec<Box<dyn Fn(&mut ModelObservations)>> = vec![
            Box::new(|r| r.schema_version = 3),
            Box::new(|r| r.source_digest = "private/path".into()),
            Box::new(|r| r.candidate_records += 1),
            Box::new(|r| r.invalid_declarations = u64::MAX),
            Box::new(|r| r.mixed_declared_models = false),
            Box::new(|r| r.declared_models.push("missing-reference".into())),
            Box::new(|r| r.declared_models.reverse()),
            Box::new(|r| r.declarations[1].record_index = 1),
            Box::new(|r| r.declarations[0].record_index = 0),
            Box::new(|r| r.declarations[0].kind = DeclarationKind::TrajectoryMetadata),
            Box::new(|r| r.declarations[0].kind = DeclarationKind::CodexSessionMetadata),
            Box::new(|r| r.coordinates = RecordCoordinates::TrajectoryArrayIndexesZeroBased),
            Box::new(|r| r.model_labels_omitted = true),
        ];
        for mutate in mutations {
            let mut invalid = good.clone();
            mutate(&mut invalid);
            assert!(invalid.validate().is_err());
        }

        let mut legacy = good;
        legacy.schema_version = 1;
        legacy.declarations[0].kind = DeclarationKind::CodexSessionMetadata;
        legacy.declarations[1].kind = DeclarationKind::CodexAssistantMessage;
        legacy.validate().unwrap();

        let mut trajectory = extract_model_observations(
            SourceFormat::Trajectory,
            &serde_json::to_vec(&vec![json!({
                "role": "meta",
                "source": "fixture",
                "model": "model-a"
            })])
            .unwrap(),
        )
        .unwrap();
        trajectory.schema_version = 2;
        assert!(trajectory.validate().is_err());
    }
}
