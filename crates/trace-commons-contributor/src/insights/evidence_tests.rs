use super::*;

fn source(path: &Path, body: &str) {
    fs::write(path, format!(
        "{{\"role\":\"meta\",\"source\":\"fixture\",\"model\":\"model-a\"}}\n{{\"role\":\"user\",\"content\":\"{body}\",\"timestamp\":\"2026-09-11T00:00:00Z\"}}\n"
    )).unwrap();
}

fn report() -> outcomes::OutcomeEvidence {
    outcomes::OutcomeEvidence::TestReport(
        outcomes::parse_test_report(
            br#"{
        "schema_version":1,"runner":"cargo-test","passed":3,"failed":1,"skipped":0,
        "observed_at":"2026-09-11T00:00:00Z","commit_id":null
    }"#,
        )
        .unwrap(),
    )
}

#[test]
fn explicit_links_deduplicate_follow_exact_content_and_do_not_upgrade_outcomes() {
    let temp = tempfile::tempdir().unwrap();
    let first_path = temp.path().join("first.jsonl");
    let alias_path = temp.path().join("alias.jsonl");
    source(&first_path, "PRIVATE-ONE");
    fs::copy(&first_path, &alias_path).unwrap();
    let store = LocalInsightStore::open(&temp.path().join("store")).unwrap();
    let first = store.import(SourceFormat::Trajectory, &first_path).unwrap();
    assert!(first.model_observations.is_some());
    store.import(SourceFormat::Trajectory, &alias_path).unwrap();
    let linked = store.link_outcome(&first.id, report()).unwrap();
    assert_eq!(linked.outcome_links.len(), 1);
    let again = store.link_outcome(&first.id, report()).unwrap();
    assert_eq!(again.outcome_links.len(), 1);
    assert_eq!(
        linked.outcome_links[0].linked_at,
        again.outcome_links[0].linked_at
    );
    assert_eq!(
        again.outcome_links[0].source_digest,
        first.report.evidence[0].source_digest
    );
    assert!(
        again
            .report
            .metrics
            .iter()
            .find(|m| m.id == MetricId::KnownOutcomes)
            .unwrap()
            .value
            .is_none()
    );
    assert!(again.manual_annotation.is_none());
    assert_eq!(
        store
            .import(SourceFormat::Trajectory, &first_path)
            .unwrap()
            .outcome_links
            .len(),
        1
    );
    source(&first_path, "PRIVATE-TWO");
    let second = store.import(SourceFormat::Trajectory, &first_path).unwrap();
    assert!(second.outcome_links.is_empty());
    assert_eq!(
        store.explain(&first.id).unwrap().outcome_links.len(),
        1,
        "alias retains old snapshot"
    );
    // One report may be explicitly associated with multiple snapshots; no exclusive attribution.
    let second = store.link_outcome(&second.id, report()).unwrap();
    assert_eq!(second.outcome_links[0].id, linked.outcome_links[0].id);
    store.delete(&first.id).unwrap();
    assert_eq!(store.explain(&second.id).unwrap().outcome_links.len(), 1);
    assert!(
        store
            .unlink_outcome(&second.id, &linked.outcome_links[0].id)
            .unwrap()
            .outcome_links
            .is_empty()
    );
    assert!(first_path.exists() && alias_path.exists());
    let saved = fs::read_to_string(store.dir.join("index.json")).unwrap();
    assert!(!saved.contains("PRIVATE-"));
}

#[test]
fn legacy_v2_stays_unknown_until_reimport_and_new_fields_are_digest_validated() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("source.jsonl");
    source(&file, "PRIVATE");
    let store = LocalInsightStore::open(&temp.path().join("store")).unwrap();
    let first = store.import(SourceFormat::Trajectory, &file).unwrap();
    let index_path = store.dir.join("index.json");
    let mut index: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    index["version"] = 2.into();
    let old = index["reports"][&first.id].as_object_mut().unwrap();
    old.remove("model_observations");
    old.remove("outcome_links");
    fs::write(&index_path, serde_json::to_vec(&index).unwrap()).unwrap();
    assert!(
        store
            .explain(&first.id)
            .unwrap()
            .model_observations
            .is_none()
    );
    let changed = store.link_outcome(&first.id, report()).unwrap();
    assert!(
        changed.model_observations.is_none(),
        "linking does not reread the source"
    );
    assert!(
        store
            .import(SourceFormat::Trajectory, &file)
            .unwrap()
            .model_observations
            .is_some()
    );
    let valid: serde_json::Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    assert_eq!(valid["version"], super::STORE_VERSION);
    for field in ["model_observations", "outcome_links"] {
        let mut corrupt = valid.clone();
        if field == "model_observations" {
            corrupt["reports"][&first.id][field]["source_digest"] = "0".repeat(64).into();
        } else {
            corrupt["reports"][&first.id][field][0]["source_digest"] = "0".repeat(64).into();
        }
        fs::write(&index_path, serde_json::to_vec(&corrupt).unwrap()).unwrap();
        assert!(store.list().is_err());
    }
    fs::write(&index_path, serde_json::to_vec(&valid).unwrap()).unwrap();
    store.delete(&first.id).unwrap();
    assert!(store.list().unwrap().is_empty());
}
