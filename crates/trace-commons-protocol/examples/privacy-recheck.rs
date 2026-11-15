// Read-only operator diagnostic. MIT OR Apache-2.0.

use std::collections::BTreeMap;
use std::io::{self, BufRead, Read, Write};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use trace_commons_protocol::trace_contribution::{
    DeterministicTraceRedactor, MAX_TRACE_ENVELOPE_BYTES, PiiClassifyPolicy, PrivacyFilterAdapter,
    ResidualScanError, residual_secret_labels_json,
};
#[cfg(feature = "near-ai-privacy-filter")]
use trace_commons_protocol::trace_contribution::{
    PrivacyFilterBackendTag, privacy_filter_adapter_from_env, rescrub_envelope_prose_pii_with,
    rescrub_trace_envelope_with, residual_secret_labels,
};

#[cfg(any(test, feature = "near-ai-privacy-filter"))]
use trace_commons_protocol::trace_contribution::TraceContributionEnvelope;

const MAX_RECORD_BYTES: usize = MAX_TRACE_ENVELOPE_BYTES * 6 + 4096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    run_ref: String,
    expected_sha256: String,
    envelope_json: String,
}

fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn failure(label: &'static str, run_ref: Option<&str>) -> Value {
    json!({"run_ref_hash":run_ref.map(|r| hash(r.as_bytes())), "verdict":"error", "error":label,"approval_eligible":false})
}

fn decode_record(line: &[u8]) -> Result<(Record, Value), Value> {
    let record: Record =
        serde_json::from_slice(line).map_err(|_| failure("invalid_record", None))?;
    let fail = |label| failure(label, Some(&record.run_ref));
    if record.run_ref.len() > 256 {
        return Err(fail("run_ref_too_large"));
    }
    if record.envelope_json.len() > MAX_TRACE_ENVELOPE_BYTES {
        return Err(fail("envelope_too_large"));
    }
    if hash(record.envelope_json.as_bytes()) != record.expected_sha256 {
        return Err(fail("input_hash_mismatch"));
    }
    let original: Value =
        serde_json::from_str(&record.envelope_json).map_err(|_| fail("invalid_envelope"))?;
    Ok((record, original))
}

// Serde ordinarily discards unknown struct fields. Keep refusing them, but
// expose bounded hashed diagnostics so an operator can identify schema drift
// without printing contributor field names, values or paths.
#[cfg(any(test, feature = "near-ai-privacy-filter"))]
fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(any(test, feature = "near-ai-privacy-filter"))]
#[derive(Default)]
struct CoverageReport {
    count: usize,
    samples: Vec<Value>,
}

#[cfg(any(test, feature = "near-ai-privacy-filter"))]
impl CoverageReport {
    fn note(
        &mut self,
        kind: &'static str,
        path: &str,
        key: Option<&str>,
        original: &Value,
        supported: Option<&Value>,
    ) {
        self.count += 1;
        if self.samples.len() < 8 {
            self.samples.push(json!({
                "kind":kind,
                "path_sha256":hash(path.as_bytes()),
                "key_sha256":key.map(|key| hash(key.as_bytes())),
                "original_kind":value_kind(original),
                "supported_kind":supported.map(value_kind).unwrap_or("missing"),
            }));
        }
    }

    fn walk(&mut self, original: &Value, supported: &Value, path: &str, key: Option<&str>) {
        match (original, supported) {
            (Value::Object(left), Value::Object(right)) => {
                for (name, value) in left {
                    // RFC 6901 JSON Pointer, hashed before it reaches output.
                    let child = format!("{path}/{}", name.replace('~', "~0").replace('/', "~1"));
                    match right.get(name) {
                        Some(other) => self.walk(value, other, &child, Some(name)),
                        None => self.note("missing_field", &child, Some(name), value, None),
                    }
                }
            }
            (Value::Array(left), Value::Array(right)) => {
                if left.len() != right.len() {
                    self.note("array_length", path, key, original, Some(supported));
                }
                for (index, (a, b)) in left.iter().zip(right).enumerate() {
                    self.walk(a, b, &format!("{path}/{index}"), None);
                }
            }
            (Value::Null, Value::Null)
            | (Value::Bool(_), Value::Bool(_))
            | (Value::Number(_), Value::Number(_))
            | (Value::String(_), Value::String(_)) => {
                if original != supported {
                    self.note("scalar_changed", path, key, original, Some(supported));
                }
            }
            _ => self.note("type_changed", path, key, original, Some(supported)),
        }
    }
}

#[cfg(any(test, feature = "near-ai-privacy-filter"))]
fn coverage_diagnostics(original: &Value, supported: &Value) -> Value {
    let mut report = CoverageReport::default();
    report.walk(original, supported, "", None);
    json!({"mismatch_count":report.count,"mismatches":report.samples})
}

fn scan_json(value: &Value) -> Value {
    scan_result(residual_secret_labels_json(
        &DeterministicTraceRedactor::deterministic_only(Vec::new()),
        value,
    ))
}

#[cfg(feature = "near-ai-privacy-filter")]
fn scan(envelope: &TraceContributionEnvelope) -> Value {
    scan_result(residual_secret_labels(
        &DeterministicTraceRedactor::deterministic_only(Vec::new()),
        envelope,
    ))
}

fn scan_result(result: Result<BTreeMap<String, u32>, ResidualScanError>) -> Value {
    match result {
        Ok(labels) => {
            // Residual locations may contain contributor-controlled object
            // keys even when the path looks schema-shaped. Return only hashes.
            let locations: BTreeMap<String, u32> = labels
                .into_iter()
                .map(|(label, count)| (format!("location_{}", hash(label.as_bytes())), count))
                .collect();
            json!({"status":if locations.is_empty() {"clean"} else {"findings"},"labels":locations})
        }
        Err(error) => json!({"status":"error","error":match error {
            ResidualScanError::SerializationFailed => "serialization_failed",
            ResidualScanError::BudgetExceeded => "budget_exceeded",
        }}),
    }
}

// Stored pass artifacts use the wire serializer's shortest f32 decimals.
// `to_value` promotes f32 to f64 and would falsely report changed scalars.
#[cfg(any(test, feature = "near-ai-privacy-filter"))]
fn wire_value(envelope: &TraceContributionEnvelope) -> Result<Value, serde_json::Error> {
    serde_json::from_slice(&serde_json::to_vec(envelope)?)
}

// Compare every content field, including fields outside events. Only scrub
// annotations are excluded; new schema fields remain covered automatically.
#[cfg(any(test, feature = "near-ai-privacy-filter"))]
fn content_view(envelope: &TraceContributionEnvelope) -> Result<Value, serde_json::Error> {
    let mut value = serde_json::to_value(envelope)?;
    if let Some(object) = value.as_object_mut() {
        object.remove("privacy");
    }
    if let Some(card) = value.get_mut("trace_card").and_then(Value::as_object_mut) {
        card.remove("redaction_pipeline_version");
    }
    Ok(value)
}

#[cfg(feature = "near-ai-privacy-filter")]
fn deltas(before: &BTreeMap<String, u32>, after: &BTreeMap<String, u32>) -> BTreeMap<String, u32> {
    after
        .iter()
        .filter_map(|(label, count)| {
            let delta = count.saturating_sub(before.get(label).copied().unwrap_or(0));
            (delta > 0).then(|| (format!("label_{}", hash(label.as_bytes())), delta))
        })
        .collect()
}

async fn inspect(
    line: &[u8],
    residual_only: bool,
    adapter: Option<&Arc<dyn PrivacyFilterAdapter>>,
    policy: PiiClassifyPolicy,
) -> Value {
    let (record, original_json) = match decode_record(line) {
        Ok(decoded) => decoded,
        Err(error) => return error,
    };
    let original_scan = scan_json(&original_json);
    let original_risk = match original_json
        .pointer("/privacy/residual_pii_risk")
        .and_then(Value::as_str)
    {
        Some("low") => "low",
        Some("medium") => "medium",
        Some("high") => "high",
        _ => "unknown",
    };
    let mut result = json!({
        "run_ref_hash":hash(record.run_ref.as_bytes()),
        "input_sha256":record.expected_sha256,
        "original_risk":original_risk,
        "original_residual_scan":original_scan,
        "approval_eligible":false,
    });
    if residual_only {
        result["mode"] = json!("residual_only");
        result["verdict"] = json!("diagnostic_only");
        result["current_risk"] = json!(original_risk);
        result["content_changed"] = json!(false);
        result["output_sha256"] = json!(record.expected_sha256);
        return result;
    }
    #[cfg(feature = "near-ai-privacy-filter")]
    {
        let original: TraceContributionEnvelope = match serde_json::from_str(&record.envelope_json)
        {
            Ok(envelope) => envelope,
            Err(_) => return failure("invalid_envelope", Some(&record.run_ref)),
        };
        let supported = match wire_value(&original) {
            Ok(value) => value,
            Err(_) => return failure("serialization_failed", Some(&record.run_ref)),
        };
        let coverage = coverage_diagnostics(&original_json, &supported);
        if coverage["mismatch_count"] != 0 {
            let mut error = failure("unsupported_envelope_fields", Some(&record.run_ref));
            error["coverage"] = coverage;
            return error;
        }
        let Some(adapter) = adapter else {
            return failure("classifier_unavailable", Some(&record.run_ref));
        };
        let mut checked = original.clone();
        let deterministic_basis = rescrub_trace_envelope_with(
            &DeterministicTraceRedactor::deterministic_only(Vec::new()),
            &mut checked,
        );
        let classifier_basis =
            match rescrub_envelope_prose_pii_with(adapter.as_ref(), &mut checked, policy).await {
                Ok(basis) => basis,
                Err(_) => return failure("classifier_failed", Some(&record.run_ref)),
            };
        let (Ok(before), Ok(after), Ok(output)) = (
            content_view(&original),
            content_view(&checked),
            serde_json::to_vec(&checked),
        ) else {
            return failure("serialization_failed", Some(&record.run_ref));
        };
        result["mode"] = json!("classifier");
        result["classify_policy"] = json!(policy.as_label());
        result["verdict"] = json!("diagnostic_only");
        result["current_risk"] = json!(checked.privacy.residual_pii_risk);
        result["content_changed"] = json!(before != after);
        result["output_sha256"] = json!(hash(&output));
        result["deterministic_basis"] = json!(deterministic_basis);
        result["classifier_basis"] = json!(classifier_basis);
        result["new_redaction_counts"] = json!(deltas(
            &original.privacy.redaction_counts,
            &checked.privacy.redaction_counts
        ));
        result["output_residual_scan"] = scan(&checked);
        result
    }
    #[cfg(not(feature = "near-ai-privacy-filter"))]
    {
        let _ = (adapter, policy);
        failure("classifier_feature_required", Some(&record.run_ref))
    }
}

#[tokio::main]
async fn main() {
    if let Err(label) = run().await {
        eprintln!("{label}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), &'static str> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help") {
        println!(
            "privacy-recheck [--residual-only] [--all-events]\nReads JSONL records {{run_ref, expected_sha256, envelope_json}} from stdin.\nDiagnostic only. Never modifies stored artifacts or approves traces.\nDefault classifier policy: prose-only. Requires a real configured privacy backend."
        );
        return Ok(());
    }
    if args
        .iter()
        .any(|arg| !matches!(arg.as_str(), "--residual-only" | "--all-events"))
    {
        return Err("invalid_arguments");
    }
    let residual_only = args.iter().any(|arg| arg == "--residual-only");
    let policy = if args.iter().any(|arg| arg == "--all-events") {
        PiiClassifyPolicy::AllEvents
    } else {
        PiiClassifyPolicy::ProseOnly
    };
    let adapter: Option<Arc<dyn PrivacyFilterAdapter>> = if residual_only {
        None
    } else {
        #[cfg(not(feature = "near-ai-privacy-filter"))]
        return Err("classifier_feature_required");
        #[cfg(feature = "near-ai-privacy-filter")]
        {
            let (adapter, backend) = privacy_filter_adapter_from_env()
                .map_err(|_| "classifier_configuration_invalid")?
                .ok_or("classifier_unavailable")?;
            if matches!(backend, PrivacyFilterBackendTag::None) {
                return Err("classifier_unavailable");
            }
            Some(adapter)
        }
    };
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut number = 0u64;
    loop {
        let mut line = Vec::new();
        let size = (&mut input)
            .take((MAX_RECORD_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|_| "input_read_failed")?;
        if size == 0 {
            break;
        }
        if size > MAX_RECORD_BYTES {
            return Err("record_too_large");
        }
        number += 1;
        eprintln!("recheck_started record={number}");
        let result = inspect(&line, residual_only, adapter.as_ref(), policy).await;
        serde_json::to_writer(&mut output, &result).map_err(|_| "output_write_failed")?;
        output.write_all(b"\n").map_err(|_| "output_write_failed")?;
        output.flush().map_err(|_| "output_write_failed")?;
        eprintln!("recheck_finished record={number}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use trace_commons_protocol::trace_contribution::{
        RawTraceCaptureTurn, RawTraceContribution, RecordedTraceContributionOptions,
        ResidualPiiRisk, TraceRedactor,
    };

    async fn envelope(text: &str) -> TraceContributionEnvelope {
        let now = Utc::now();
        let raw = RawTraceContribution::from_capture_turns(
            &[RawTraceCaptureTurn {
                user_input: text.to_owned(),
                response: None,
                tool_calls: Vec::new(),
                started_at: now,
                completed_at: Some(now),
                state: Some("complete".into()),
            }],
            RecordedTraceContributionOptions {
                include_message_text: true,
                ..Default::default()
            },
        );
        DeterministicTraceRedactor::deterministic_only(Vec::new())
            .redact_trace(raw)
            .await
            .unwrap()
    }
    fn record(envelope: &TraceContributionEnvelope) -> Vec<u8> {
        let bytes = serde_json::to_string(envelope).unwrap();
        serde_json::to_vec(&json!({"run_ref":"test-run","expected_sha256":hash(bytes.as_bytes()),"envelope_json":bytes})).unwrap()
    }
    #[tokio::test]
    async fn hash_mismatch_is_refused() {
        let mut value: Value =
            serde_json::from_slice(&record(&envelope("list files").await)).unwrap();
        value["expected_sha256"] = json!("sha256:wrong");
        let result = inspect(
            &serde_json::to_vec(&value).unwrap(),
            true,
            None,
            PiiClassifyPolicy::AllEvents,
        )
        .await;
        assert_eq!(result["error"], "input_hash_mismatch");
        assert_eq!(result["approval_eligible"], false);
    }
    #[tokio::test]
    async fn original_survivor_is_reported_without_mutation() {
        let mut env = envelope("list files").await;
        env.outcome.human_correction =
            Some(format!("sk-{}", "aB3cD4eF5gH6iJ7kL8mN9pQ0rS1tU2vW3xY4zA5b"));
        let input = record(&env);
        let result = inspect(&input, true, None, PiiClassifyPolicy::AllEvents).await;
        assert_eq!(result["original_residual_scan"]["status"], "findings");
        assert_eq!(result["content_changed"], false);
        assert_eq!(result["approval_eligible"], false);
        assert_eq!(result["output_sha256"], result["input_sha256"]);
        assert!(!result.to_string().contains("aB3cD4"));
    }
    #[tokio::test]
    async fn residual_locations_never_expose_contributor_keys() {
        let mut env = envelope("list files").await;
        env.events[0].structured_payload = json!({
            "alice": format!("sk-{}", "aB3cD4eF5gH6iJ7kL8mN9pQ0rS1tU2vW3xY4zA5b")
        });
        let result = inspect(&record(&env), true, None, PiiClassifyPolicy::AllEvents).await;
        let scan = &result["original_residual_scan"];
        assert_eq!(scan["status"], "findings");
        let labels = scan["labels"].as_object().unwrap();
        assert!(!labels.is_empty());
        assert!(
            labels
                .keys()
                .all(|key| key.starts_with("location_sha256:") && key.len() == 80)
        );
        assert!(labels.values().all(|count| count.as_u64().unwrap() > 0));
        assert!(!result.to_string().contains("alice"));
        assert!(!result.to_string().contains("aB3cD4"));
    }
    #[cfg(feature = "near-ai-privacy-filter")]
    #[test]
    fn redaction_count_labels_are_also_hashed() {
        let after = BTreeMap::from([("residual_secret_at:envelope.events[].alice".into(), 2)]);
        let output = serde_json::to_string(&deltas(&BTreeMap::new(), &after)).unwrap();
        assert!(!output.contains("alice"));
        assert!(output.contains("label_sha256:"));
        assert!(output.contains(":2"));
    }
    #[tokio::test]
    async fn clean_residual_does_not_approve_high() {
        let mut env = envelope("list files").await;
        env.privacy.residual_pii_risk = ResidualPiiRisk::High;
        let result = inspect(&record(&env), true, None, PiiClassifyPolicy::AllEvents).await;
        assert_eq!(result["original_residual_scan"]["status"], "clean");
        assert_eq!(result["current_risk"], "high");
        assert_eq!(result["approval_eligible"], false);
    }
    #[tokio::test]
    async fn content_comparison_includes_sidechannels() {
        let env = envelope("list files").await;
        let mut changed = env.clone();
        changed.replay.replay_notes.push("a new note".into());
        assert_ne!(content_view(&env).unwrap(), content_view(&changed).unwrap());
        changed = env.clone();
        changed.privacy.warnings.push("annotation".into());
        changed
            .trace_card
            .redaction_pipeline_version
            .push_str("+annotation");
        assert_eq!(content_view(&env).unwrap(), content_view(&changed).unwrap());
    }
    #[cfg(feature = "near-ai-privacy-filter")]
    #[tokio::test]
    async fn unsupported_fields_are_not_silently_unchecked() {
        let env = envelope("list files").await;
        let mut value = serde_json::to_value(&env).unwrap();
        value["events"][0]["future_content"] = json!("sensitive_marker");
        let raw = serde_json::to_string(&value).unwrap();
        let input=serde_json::to_vec(&json!({"run_ref":"test-run","expected_sha256":hash(raw.as_bytes()),"envelope_json":raw})).unwrap();
        let result = inspect(&input, false, None, PiiClassifyPolicy::AllEvents).await;
        assert_eq!(result["error"], "unsupported_envelope_fields");
        assert!(!result.to_string().contains("sensitive_marker"));
    }
    #[tokio::test]
    async fn raw_scan_covers_unknown_fields_and_does_not_echo_risk() {
        let mut value = serde_json::to_value(envelope("list files").await).unwrap();
        value["future_field"] =
            json!({"alice":format!("sk-{}", "aB3cD4eF5gH6iJ7kL8mN9pQ0rS1tU2vW3xY4zA5b")});
        value["privacy"]["residual_pii_risk"] = json!("private_sensitive_risk");
        let raw = serde_json::to_string(&value).unwrap();
        let input=serde_json::to_vec(&json!({"run_ref":"test-run","expected_sha256":hash(raw.as_bytes()),"envelope_json":raw})).unwrap();
        let result = inspect(&input, true, None, PiiClassifyPolicy::ProseOnly).await;
        assert_eq!(result["original_residual_scan"]["status"], "findings");
        assert_eq!(result["original_risk"], "unknown");
        assert_eq!(result["approval_eligible"], false);
        let output = result.to_string();
        for private in ["future_field", "alice", "aB3cD4", "private_sensitive_risk"] {
            assert!(!output.contains(private));
        }
    }
    #[tokio::test]
    async fn raw_and_typed_scans_have_identical_results() {
        let env = envelope("list files").await;
        let redactor = DeterministicTraceRedactor::deterministic_only(Vec::new());
        let raw = serde_json::to_value(&env).unwrap();
        assert_eq!(
            residual_secret_labels_json(&redactor, &raw),
            trace_commons_protocol::trace_contribution::residual_secret_labels(&redactor, &env)
        );
    }
    #[test]
    fn raw_scan_depth_limit_fails_closed() {
        let mut value = json!("leaf");
        for _ in 0..80 {
            value = json!([value]);
        }
        let result = residual_secret_labels_json(
            &DeterministicTraceRedactor::deterministic_only(Vec::new()),
            &value,
        );
        assert_eq!(result, Err(ResidualScanError::BudgetExceeded));
    }
    #[tokio::test]
    async fn wire_roundtrip_preserves_real_f32_decimals_and_rejects_changes() {
        use trace_commons_protocol::trace_contribution::EmbeddingAnalysisMetadata;
        let mut env = envelope("list files").await;
        env.embedding_analysis = Some(EmbeddingAnalysisMetadata {
            embedding_model: None,
            canonical_summary_hash: String::new(),
            trace_vector_id: None,
            nearest_trace_ids: Vec::new(),
            cluster_id: None,
            nearest_cluster_id: None,
            novelty_score: Some(0.1),
            duplicate_score: None,
            coverage_tags: Vec::new(),
        });
        env.value_card.scorecard.quality = 0.2;
        env.value_card.scorecard.novelty = 0.3;
        env.value_card.scorecard.coverage_bonus = 0.4;
        env.value_card.scorecard.difficulty = 0.6;
        let raw = serde_json::to_vec(&env).unwrap();
        let stored: Value = serde_json::from_slice(&raw).unwrap();
        let typed: TraceContributionEnvelope = serde_json::from_slice(&raw).unwrap();
        // Reproduce the failure: to_value promotes these real f32 decimals.
        let promoted = serde_json::to_value(&typed).unwrap();
        assert!(
            coverage_diagnostics(&stored, &promoted)["mismatch_count"]
                .as_u64()
                .unwrap()
                >= 5
        );
        let supported = wire_value(&typed).unwrap();
        assert_eq!(
            coverage_diagnostics(&stored, &supported)["mismatch_count"],
            0
        );
        let mut changed = supported.clone();
        changed["value_card"]["scorecard"]["quality"] = json!(0.9);
        let diagnosis = coverage_diagnostics(&stored, &changed);
        assert_eq!(diagnosis["mismatch_count"], 1);
        assert_eq!(diagnosis["mismatches"][0]["kind"], "scalar_changed");
        assert_eq!(diagnosis["mismatches"][0]["key_sha256"], hash(b"quality"));
    }
    #[cfg(feature = "near-ai-privacy-filter")]
    #[tokio::test]
    async fn classifier_preflight_accepts_stored_f32_wire_values() {
        let mut env = envelope("list files").await;
        env.value_card.scorecard.quality = 0.2;
        env.privacy.residual_pii_risk = ResidualPiiRisk::High;
        let adapter: Arc<dyn PrivacyFilterAdapter> = Arc::new(Classifier { find_person: false });
        let result = inspect(
            &record(&env),
            false,
            Some(&adapter),
            PiiClassifyPolicy::ProseOnly,
        )
        .await;
        assert_eq!(result["mode"], "classifier");
        assert_eq!(result["current_risk"], "high");
        assert_eq!(result["approval_eligible"], false);
    }
    #[test]
    fn coverage_diagnostics_hash_keys_paths_and_cap_samples() {
        let mut original = json!({"privatepersonname":"sensitive_value_marker"});
        for index in 0..12 {
            original[format!("privatekey{index}")] = json!(null);
        }
        let report = coverage_diagnostics(&original, &json!({}));
        assert_eq!(report["mismatch_count"], 13);
        let samples = report["mismatches"].as_array().unwrap();
        assert_eq!(samples.len(), 8);
        assert!(samples.iter().all(|entry| entry["kind"] == "missing_field"));
        assert!(samples.iter().all(|entry| {
            entry["path_sha256"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        }));
        assert!(
            samples
                .iter()
                .all(|entry| entry["key_sha256"].as_str().unwrap().starts_with("sha256:"))
        );
        let output = report.to_string();
        assert!(!output.contains("privatepersonname"));
        assert!(!output.contains("privatekey"));
        assert!(!output.contains("sensitive_value_marker"));
    }
    #[test]
    fn coverage_diagnostics_distinguish_changed_shapes() {
        let original = json!({"x":"secret_marker","y":true,"z":[1,2]});
        let supported = json!({"x":"different_marker","y":null,"z":[1]});
        let report = coverage_diagnostics(&original, &supported);
        assert_eq!(report["mismatch_count"], 3);
        let kinds: Vec<_> = report["mismatches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["kind"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            vec!["scalar_changed", "type_changed", "array_length"]
        );
        assert!(!report.to_string().contains("secret_marker"));
        assert!(!report.to_string().contains("different_marker"));
    }
    #[tokio::test]
    async fn malformed_input_never_echoes_sensitive_bytes() {
        let marker = "private_sensitive_marker";
        let result = inspect(
            format!("{{\"{marker}").as_bytes(),
            true,
            None,
            PiiClassifyPolicy::AllEvents,
        )
        .await;
        assert_eq!(result["error"], "invalid_record");
        assert!(!result.to_string().contains(marker));
    }
    #[cfg(feature = "near-ai-privacy-filter")]
    struct Classifier {
        find_person: bool,
    }
    #[cfg(feature = "near-ai-privacy-filter")]
    #[async_trait::async_trait]
    impl PrivacyFilterAdapter for Classifier {
        async fn redact_text(
            &self,
            text: &str,
        ) -> Result<
            Option<trace_commons_protocol::trace_contribution::SafePrivacyFilterRedaction>,
            trace_commons_protocol::trace_contribution::TraceContributionError,
        > {
            use trace_commons_protocol::trace_contribution::{
                RedactionReport, SafePrivacyFilterRedaction, SafePrivacyFilterSummary,
            };
            if !self.find_person || !text.contains("Jane Doe") {
                return Ok(None);
            }
            let mut report = RedactionReport::default();
            report
                .counts
                .insert("privacy_filter:private_person".into(), 1);
            report.pii_labels_present.push("private_person".into());
            Ok(Some(SafePrivacyFilterRedaction {
                private_edits: None,
                redacted_text: text.replace("Jane Doe", "[REDACTED:private_person]"),
                report,
                summary: SafePrivacyFilterSummary {
                    schema_version: 1,
                    output_mode: "redacted_text_only".into(),
                    span_count: 1,
                    by_label: BTreeMap::from([("private_person".into(), 1)]),
                    decoded_mismatch: false,
                    classify_policy: None,
                    events_examined: 0,
                    events_skipped_by_policy: 0,
                },
            }))
        }
    }
    #[cfg(feature = "near-ai-privacy-filter")]
    #[tokio::test]
    async fn empty_classifier_does_not_lower_high_or_approve() {
        let mut env = envelope("list files").await;
        env.privacy.residual_pii_risk = ResidualPiiRisk::High;
        let adapter: Arc<dyn PrivacyFilterAdapter> = Arc::new(Classifier { find_person: false });
        let result = inspect(
            &record(&env),
            false,
            Some(&adapter),
            PiiClassifyPolicy::AllEvents,
        )
        .await;
        assert_eq!(result["current_risk"], "high");
        assert_eq!(result["approval_eligible"], false);
        assert_eq!(result["output_residual_scan"]["status"], "clean");
    }
    #[cfg(feature = "near-ai-privacy-filter")]
    #[tokio::test]
    async fn additional_redaction_is_reported_as_changed_content() {
        let mut env = envelope("list files").await;
        env.privacy.residual_pii_risk = ResidualPiiRisk::High;
        env.events[0].redacted_content = Some("Ask Jane Doe about the task.".into());
        let input = record(&env);
        let original = input.clone();
        let adapter: Arc<dyn PrivacyFilterAdapter> = Arc::new(Classifier { find_person: true });
        let result = inspect(&input, false, Some(&adapter), PiiClassifyPolicy::AllEvents).await;
        assert_eq!(result["content_changed"], true);
        assert!(
            result["new_redaction_counts"]
                [format!("label_{}", hash(b"privacy_filter:private_person"))]
            .as_u64()
            .unwrap()
                > 0
        );
        assert_eq!(result["approval_eligible"], false);
        assert_eq!(input, original);
        assert!(!result.to_string().contains("Jane Doe"));
    }
}
