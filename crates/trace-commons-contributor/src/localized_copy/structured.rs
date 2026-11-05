//! Explicit prose fields in structured payloads. Facts and detector identifiers
//! are never searched for English or recursively treated as translatable copy.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::text;
use crate::{
    compute::{ComputeCopy, presentation},
    redaction_summary,
};

/// Count objects are the legacy FFI shape. Pair arrays also accept callers
/// carrying raw entries, without converting labels into prose or losing counts.
fn count_map(value: &Value) -> Option<BTreeMap<String, u32>> {
    if value.is_object() {
        return serde_json::from_value(value.clone()).ok();
    }
    let pairs: Vec<(String, u32)> = serde_json::from_value(value.clone()).ok()?;
    let mut counts = BTreeMap::new();
    for (label, count) in pairs {
        if counts.insert(label, count).is_some() {
            return None;
        }
    }
    Some(counts)
}

const FAMILIES: &[&str] = &[
    "local_path",
    "secret",
    "privacy_filter",
    "sensitive_field",
    "tool_sensitive_field",
    "residual_secret_at",
];

fn redaction_row(row: redaction_summary::Row) -> Value {
    let known = FAMILIES.contains(&row.family.as_str());
    let variant = if known {
        row.family.as_str()
    } else {
        "unknown"
    };
    let description = text(
        &format!("message.redaction_summary_json.{variant}.description"),
        row.description,
        json!({}),
    );
    let display = if known {
        text(
            &format!("message.redaction_summary_json.{variant}.display"),
            &row.display,
            json!({}),
        )
    } else {
        // Open-vocabulary detector labels are technical identifiers, not copy.
        json!(row.display)
    };
    json!({
        "family": row.family, "display": display, "description": description,
        "occurrences": row.occurrences, "distinct": row.distinct,
        // Keep the canonical legacy renderer's technical detail unchanged.
        "detail": row.detail,
    })
}

fn redaction_summary(input: &Value) -> Option<Value> {
    let occurrences = count_map(&input["occurrences"])?;
    // Like tc_redaction_summary_json, unreadable distinct counts mean absent,
    // never a reason to drop an occurrence or report that nothing was removed.
    let distinct = count_map(&input["distinct"]).unwrap_or_default();
    let summary = redaction_summary::summary_copy(&occurrences, &distinct);
    Some(json!({
        "removed": summary.removed.into_iter().map(redaction_row).collect::<Vec<_>>(),
        "still_present": summary.still_present.into_iter().map(redaction_row).collect::<Vec<_>>(),
    }))
}

fn compute_key(identity: &str) -> String {
    format!("message.compute_snapshot_json.{identity}")
}

fn compute_snapshot(input: &Value) -> Option<Value> {
    let mut snapshot = input.as_object()?.clone();
    if input["schema"].as_str() != Some("trace_commons.compute_status.v1") {
        return Some(Value::Object(snapshot));
    }
    for (field, identity_field) in [("title", "title_key"), ("detail", "detail_key")] {
        // Source-owned identity only: neither reason/state nor incoming English
        // selects a key. Missing/future metadata leaves ordinary text alone.
        if let Some(message) = input[identity_field].as_str().and_then(presentation::find)
            && let Some(english) = input[field].as_str()
        {
            snapshot.insert(
                field.into(),
                text(&compute_key(message.key), english, json!({})),
            );
        }
    }
    if let Some(copy) = input["copy"].as_object() {
        let canonical =
            serde_json::to_value(ComputeCopy::default()).expect("compute copy serializes");
        let mut localized = copy.clone();
        for field in canonical
            .as_object()
            .expect("compute copy is an object")
            .keys()
        {
            if let Some(english) = copy.get(field).and_then(Value::as_str) {
                localized.insert(
                    field.clone(),
                    text(&format!("compute_copy.{field}"), english, json!({})),
                );
            }
        }
        snapshot.insert("copy".into(), Value::Object(localized));
    }
    Some(Value::Object(snapshot))
}

pub fn message(operation: &str, input: &Value) -> Option<Value> {
    match operation {
        "redaction_summary_json" => redaction_summary(input),
        "compute_snapshot_json" => compute_snapshot(input),
        _ => None,
    }
}

pub fn templates() -> Vec<Value> {
    let mut templates = Vec::new();
    let mut occurrences: BTreeMap<String, u32> = FAMILIES
        .iter()
        .map(|family| ((*family).into(), 1))
        .collect();
    occurrences.insert("unknown_detector".into(), 1);
    templates.push(
        redaction_summary(&json!({"occurrences": occurrences, "distinct": {}}))
            .expect("canonical count map"),
    );
    for message in presentation::messages() {
        templates.push(text(&compute_key(message.key), message.english, json!({})));
    }
    let copy = serde_json::to_value(ComputeCopy::default()).expect("compute copy serializes");
    for (field, english) in copy.as_object().expect("compute copy is an object") {
        templates.push(text(
            &format!("compute_copy.{field}"),
            english.as_str().expect("compute copy field is text"),
            json!({}),
        ));
    }
    templates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redactions_use_canonical_grouping_and_leave_identifiers_alone() {
        let input = json!({
            "occurrences": {
                "local_path": 8, "secret:openai_api_key": 2,
                "residual_secret_at:events.3.tool_result": 1,
                "future_detector:{count}": 3
            },
            "distinct": {"local_path": 2}
        });
        let output = message("redaction_summary_json", &input).unwrap();
        assert_eq!(output["removed"][0]["family"], "local_path");
        assert_eq!(output["removed"][0]["occurrences"], 8);
        assert_eq!(output["removed"][0]["distinct"], 2);
        assert_eq!(
            output["removed"][0]["description"]["english"],
            crate::preview_copy::REDACTION_CATEGORY_LOCAL_PATH
        );
        assert_eq!(output["removed"][1]["family"], "future_detector");
        assert!(output["removed"][1]["display"].is_string());
        assert_eq!(output["removed"][1]["detail"], json!(["{count}"]));
        assert_eq!(
            output["still_present"][0]["detail"],
            json!(["events.3.tool_result"])
        );
    }

    #[test]
    fn raw_count_entries_and_legacy_maps_agree_and_invalid_counts_are_refused() {
        let object = message(
            "redaction_summary_json",
            &json!({"occurrences":{"secret":2},"distinct":{}}),
        );
        let array = message(
            "redaction_summary_json",
            &json!({"occurrences":[["secret",2]],"distinct":[]}),
        );
        assert_eq!(object, array);
        assert_eq!(
            object,
            message(
                "redaction_summary_json",
                &json!({"occurrences":{"secret":2},"distinct":"unreadable"})
            )
        );
        for occurrences in [
            json!({"secret": -1}),
            json!({"secret": 4_294_967_296_u64}),
            json!([["secret", 1], ["secret", 2]]),
            json!(null),
        ] {
            assert!(
                message(
                    "redaction_summary_json",
                    &json!({"occurrences":occurrences})
                )
                .is_none()
            );
        }
    }

    #[test]
    fn compute_only_wraps_source_identified_prose_and_preserves_all_facts() {
        let copy = serde_json::to_value(ComputeCopy::default()).unwrap();
        let input = json!({
            "schema":"trace_commons.compute_status.v1", "state":"paused",
            "reason":"command-busy", "local_development":true,
            "title":"Compute is stopped", "detail":"The worker has exited.",
            "title_key":presentation::STOPPED.key, "detail_key":presentation::WORKER_EXITED.key,
            "copy":copy, "worker_stopped":true, "can_resume":false,
            "admission":"assigned", "stop_outcome":"not_running",
            "unknown_identifier":"{count}", "path":"/raw/{detail}"
        });
        let mut output = message("compute_snapshot_json", &input).unwrap();
        for field in ["title", "detail"] {
            assert_eq!(output[field]["english"], input[field]);
            assert_eq!(
                output[field]["key"],
                compute_key(input[format!("{field}_key")].as_str().unwrap())
            );
            output[field] = output[field]["english"].clone();
        }
        for (field, value) in output["copy"].as_object_mut().unwrap() {
            assert_eq!(value["key"], format!("compute_copy.{field}"));
            *value = value["english"].clone();
        }
        assert_eq!(output, input);
    }

    #[test]
    fn controller_snapshots_use_registered_templates_through_observable_transitions() {
        use crate::compute::{ComputeCommand, ComputeController};
        let root = tempfile::tempdir().unwrap();
        let controller = ComputeController::open(root.path()).unwrap();
        let inventory = templates();
        let mut snapshots = vec![controller.snapshot()];
        for command in [
            ComputeCommand::Enable {
                ram_allowance_gib: 0,
            },
            ComputeCommand::Resume {},
            ComputeCommand::Enable {
                ram_allowance_gib: 8,
            },
            ComputeCommand::Pause {},
            ComputeCommand::Disable {},
        ] {
            snapshots.push(controller.command(command));
        }
        for snapshot in snapshots {
            presentation::assert_snapshot(&snapshot);
            let wire = serde_json::to_value(snapshot).unwrap();
            let output = message("compute_snapshot_json", &wire).unwrap();
            for field in ["title", "detail"] {
                assert_eq!(output[field]["english"], wire[field]);
                assert!(
                    inventory
                        .iter()
                        .any(|entry| entry["key"] == output[field]["key"]
                            && entry["english"] == output[field]["english"])
                );
            }
            assert_eq!(output["state"], wire["state"]);
            assert_eq!(output["reason"], wire["reason"]);
            assert_eq!(output["worker_stopped"], wire["worker_stopped"]);
        }
    }

    #[test]
    fn missing_future_or_unknown_metadata_never_infers_keys_from_english() {
        for input in [
            json!({"schema":"future","title_key":presentation::DISABLED.key,"title":"Compute is disabled"}),
            json!({"schema":"trace_commons.compute_status.v1","state":"disabled","reason":"worker-unavailable","title":"Compute is disabled"}),
            json!({"schema":"trace_commons.compute_status.v1","title_key":"user {text}","title":"Compute is disabled"}),
        ] {
            assert_eq!(message("compute_snapshot_json", &input), Some(input));
        }
    }

    #[test]
    fn explicit_identity_does_not_replace_or_search_incoming_text() {
        let input = json!({"schema":"trace_commons.compute_status.v1","title_key":presentation::DISABLED.key,"title":"user {title} /private/path"});
        let output = message("compute_snapshot_json", &input).unwrap();
        // Runtime source agreement rejects a translation for this mismatch.
        assert_eq!(
            output["title"]["key"],
            compute_key(presentation::DISABLED.key)
        );
        assert_eq!(output["title"]["english"], input["title"]);
    }
}
