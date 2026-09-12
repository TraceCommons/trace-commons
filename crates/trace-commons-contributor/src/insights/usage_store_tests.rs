use super::*;
use serde_json::{Value, json};

fn source(path: &Path, final_input: u64) {
    let token = |time: &str, input: u64, cached: u64, output: u64, reasoning: u64| {
        json!({
            "type":"event_msg", "timestamp":time,
            "payload":{"type":"token_count","info":{"total_token_usage":{
                "input_tokens":input,"cached_input_tokens":cached,"output_tokens":output,
                "reasoning_output_tokens":reasoning,"total_tokens":input + output
            }}}
        })
    };
    let rows = [
        json!({"type":"session_meta","timestamp":"2026-09-11T00:00:00Z","payload":{"id":"PRIVATE_SESSION_ID","model_provider":"openai"}}),
        json!({"type":"turn_context","timestamp":"2026-09-11T00:00:00Z","payload":{"model":"fixture-model"}}),
        token("2026-09-11T00:00:01Z", 100, 20, 20, 5),
        json!({"type":"response_item","timestamp":"2026-09-11T00:00:02Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"PRIVATE_BODY"}]}}),
        token("2026-09-11T00:00:03Z", final_input, 40, 30, 8),
    ];
    fs::write(
        path,
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
}

#[test]
fn usage_is_bound_to_same_bytes_and_follows_alias_reimport_and_deletion() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.jsonl");
    let alias = root.path().join("alias.jsonl");
    source(&path, 150);
    fs::copy(&path, &alias).unwrap();
    let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
    let first = store.import(SourceFormat::Codex, &path).unwrap();
    let expected = usage_evidence::extract_codex_usage_evidence(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(first.usage_evidence.as_ref(), Some(&expected));
    expected
        .validate_binding(first.source_format, &first.report.evidence[0].source_digest)
        .unwrap();
    assert!(expected.has_attributed_interval());
    assert!(first.estimated_cost_usd.is_none());
    let second = store.import(SourceFormat::Codex, &alias).unwrap();
    assert_eq!(second.id, first.id);
    assert_eq!(second.usage_evidence, first.usage_evidence);
    assert_eq!(
        store
            .import(SourceFormat::Codex, &path)
            .unwrap()
            .usage_evidence,
        first.usage_evidence
    );
    source(&path, 160);
    let changed = store.import(SourceFormat::Codex, &path).unwrap();
    assert_ne!(changed.id, first.id);
    assert_ne!(changed.usage_evidence, first.usage_evidence);
    fs::remove_file(&alias).unwrap();
    assert_eq!(
        store.explain(&first.id).unwrap().usage_evidence,
        first.usage_evidence
    );
    store.delete(&first.id).unwrap();
    assert!(store.explain(&first.id).is_err());
    assert_eq!(
        store.explain(&changed.id).unwrap().usage_evidence,
        changed.usage_evidence
    );
    let serialized = fs::read_to_string(store.dir.join("index.json")).unwrap();
    assert!(!serialized.contains("PRIVATE_BODY"));
    assert!(!serialized.contains("PRIVATE_SESSION_ID"));
}

#[test]
fn versions_one_through_five_remain_read_only_and_do_not_invent_usage() {
    for version in 1..=5 {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.jsonl");
        source(&path, 150);
        let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
        let saved = store.import(SourceFormat::Codex, &path).unwrap();
        if version >= 2 {
            store
                .annotate(&saved.id, TaskCategory::Docs, TaskOutcome::Partial)
                .unwrap();
        }
        if version >= 3 {
            let evidence = outcomes::OutcomeEvidence::TestReport(outcomes::parse_test_report(
                br#"{"schema_version":1,"runner":"fixture","passed":2,"failed":0,"skipped":0,"observed_at":"2026-09-11T00:00:04Z","commit_id":null}"#
            ).unwrap());
            store.link_outcome(&saved.id, evidence).unwrap();
        }
        if version >= 4 {
            store
                .episode_create(std::slice::from_ref(&saved.id))
                .unwrap();
        }
        let index_path = store.dir.join("index.json");
        let mut legacy: Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
        legacy["version"] = version.into();
        let report = legacy["reports"][&saved.id].as_object_mut().unwrap();
        report.remove("usage_evidence");
        report.remove("task_attribution");
        if version < 5 {
            report.remove("time_evidence");
        }
        if version < 3 {
            report.remove("model_observations");
            report.remove("outcome_links");
        }
        if version == 1 {
            report.remove("manual_annotation");
        }
        fs::write(&index_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        fs::remove_file(&path).unwrap();
        let before = fs::read(&index_path).unwrap();
        let read = store.explain(&saved.id).unwrap();
        assert!(read.usage_evidence.is_none());
        assert_eq!(fs::read(&index_path).unwrap(), before);
        let annotated = store
            .annotate(&saved.id, TaskCategory::Tests, TaskOutcome::Unknown)
            .unwrap();
        assert!(annotated.usage_evidence.is_none());
        assert_eq!(annotated.time_evidence, read.time_evidence);
        assert_eq!(annotated.model_observations, read.model_observations);
        assert_eq!(
            serde_json::to_value(&annotated.outcome_links).unwrap(),
            serde_json::to_value(&read.outcome_links).unwrap()
        );
        assert_eq!(annotated.report, read.report);
        let upgraded: Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
        assert_eq!(upgraded["version"], super::STORE_VERSION);
        assert_eq!(upgraded["episodes"], legacy["episodes"]);
        source(&path, 150);
        let reimported = store.import(SourceFormat::Codex, &path).unwrap();
        assert_eq!(reimported.id, saved.id);
        assert!(reimported.usage_evidence.is_some());
        assert_eq!(reimported.manual_annotation, annotated.manual_annotation);
    }
}

#[test]
fn corrupted_usage_or_usage_in_legacy_versions_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.jsonl");
    source(&path, 150);
    let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
    let saved = store.import(SourceFormat::Codex, &path).unwrap();
    let index_path = store.dir.join("index.json");
    let valid: Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    for (field, value) in [
        ("source_digest", json!("f".repeat(64))),
        ("source_format", json!("trajectory")),
        ("schema_version", json!(2)),
        ("complete_records", json!(u64::MAX)),
    ] {
        let mut corrupted = valid.clone();
        corrupted["reports"][&saved.id]["usage_evidence"][field] = value;
        fs::write(&index_path, serde_json::to_vec(&corrupted).unwrap()).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.list()));
        assert!(result.is_ok());
        assert!(result.unwrap().is_err(), "accepted invalid {field}");
    }
    for version in 1..=5 {
        let mut corrupted = valid.clone();
        corrupted["version"] = version.into();
        let report = corrupted["reports"][&saved.id].as_object_mut().unwrap();
        if version < 5 {
            report.remove("time_evidence");
        }
        if version < 3 {
            report.remove("model_observations");
            report.remove("outcome_links");
        }
        fs::write(&index_path, serde_json::to_vec(&corrupted).unwrap()).unwrap();
        assert!(
            store.list().is_err(),
            "legacy version {version} accepted usage evidence"
        );
    }
}
