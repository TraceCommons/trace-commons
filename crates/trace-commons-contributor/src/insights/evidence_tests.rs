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

fn codex_source(path: &Path) {
    fs::write(
        path,
        concat!(
            "{\"type\":\"session_meta\",\"timestamp\":\"2026-09-11T00:00:00Z\",\"payload\":{\"id\":\"fixture\",\"model_provider\":\"openai\"}}\n",
            "{\"type\":\"turn_context\",\"timestamp\":\"2026-09-11T00:00:01Z\",\"payload\":{\"model\":\"fixture-model\"}}\n",
            "{\"type\":\"response_item\",\"timestamp\":\"2026-09-11T00:00:02Z\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[]}}\n"
        ),
    )
    .unwrap();
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
    old.remove("time_evidence");
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
        assert_quarantined(&store, &first.id, field);
    }
    fs::write(&index_path, serde_json::to_vec(&valid).unwrap()).unwrap();
    store.delete(&first.id).unwrap();
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn legacy_v4_mutation_preserves_existing_evidence_and_episodes_without_inventing_time() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("source.jsonl");
    source(&file, "PRIVATE");
    let store = LocalInsightStore::open(&temp.path().join("store")).unwrap();
    let saved = store.import(SourceFormat::Trajectory, &file).unwrap();
    let annotated = store
        .annotate(&saved.id, TaskCategory::Debugging, TaskOutcome::Partial)
        .unwrap();
    let episode = store
        .episode_create(std::slice::from_ref(&saved.id))
        .unwrap();
    let index_path = store.dir.join("index.json");
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    legacy["version"] = 4.into();
    legacy["reports"][&saved.id]
        .as_object_mut()
        .unwrap()
        .remove("time_evidence");
    fs::write(&index_path, serde_json::to_vec(&legacy).unwrap()).unwrap();

    let before = fs::read(&index_path).unwrap();
    let read = store.explain(&saved.id).unwrap();
    assert!(read.time_evidence.is_none());
    assert_eq!(read.manual_annotation, annotated.manual_annotation);
    assert!(read.model_observations.is_some());
    assert_eq!(
        fs::read(&index_path).unwrap(),
        before,
        "reads stay read-only"
    );

    let linked = store.link_outcome(&saved.id, report()).unwrap();
    assert!(
        linked.time_evidence.is_none(),
        "mutation does not reread source"
    );
    assert_eq!(linked.manual_annotation, annotated.manual_annotation);
    assert!(linked.model_observations.is_some());
    assert_eq!(linked.outcome_links.len(), 1);
    assert_eq!(store.episode_explain(&episode.id).unwrap().members.len(), 1);
    let migrated: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    assert_eq!(migrated["version"], super::STORE_VERSION);
    assert!(migrated["reports"][&saved.id]["time_evidence"].is_null());
}

/// A nested model-observation schema above the legacy shape is only readable
/// in a store at the floor version. Without that coupling an older client
/// accepts the store and then rejects individual snapshots one at a time.
#[test]
fn a_post_legacy_model_schema_requires_the_store_version_floor() {
    const {
        assert!(
            super::STORE_VERSION >= models::MODEL_OBSERVATIONS_STORE_VERSION_FLOOR,
            "STORE_VERSION must advance with the nested model-observation schema"
        );
    }
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("source.jsonl");
    codex_source(&file);
    let store = LocalInsightStore::open(&temp.path().join("store")).unwrap();
    let saved = store.import(SourceFormat::Codex, &file).unwrap();
    assert_eq!(saved.model_observations.as_ref().unwrap().schema_version, 2);

    let index_path = store.dir.join("index.json");
    let current: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    assert_eq!(current["version"], super::STORE_VERSION);

    let mut below = current.clone();
    below["version"] = (models::MODEL_OBSERVATIONS_STORE_VERSION_FLOOR - 1).into();
    fs::write(&index_path, serde_json::to_vec(&below).unwrap()).unwrap();
    super::assert_quarantined(
        &store,
        &saved.id,
        "a schema 2 model observation below the floor",
    );

    let mut legacy = below.clone();
    legacy["reports"][&saved.id]["model_observations"]["schema_version"] = 1.into();
    fs::write(&index_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(
        store.list().unwrap()[0]
            .model_observations
            .as_ref()
            .unwrap()
            .schema_version,
        1,
        "the legacy shape stays readable below the floor"
    );

    let mut future = current;
    future["version"] = (super::STORE_VERSION + 1).into();
    fs::write(&index_path, serde_json::to_vec(&future).unwrap()).unwrap();
    assert_eq!(
        store.list().unwrap_err().to_string(),
        "insights_store_version_unsupported",
        "one diagnosable refusal of the whole store, not a per-snapshot reject"
    );
}

#[test]
fn legacy_model_schema_is_preserved_until_explicit_identical_content_reimport() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("source.jsonl");
    codex_source(&file);
    let store = LocalInsightStore::open(&temp.path().join("store")).unwrap();
    let saved = store.import(SourceFormat::Codex, &file).unwrap();
    let observations = saved.model_observations.as_ref().unwrap();
    assert_eq!(observations.schema_version, 2);
    assert_eq!(observations.candidate_records, 1);
    assert_eq!(observations.valid_declarations, 1);
    assert_eq!(observations.missing_declarations, 0);

    let index_path = store.dir.join("index.json");
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    legacy["reports"][&saved.id]["model_observations"]["schema_version"] = 1.into();
    fs::write(&index_path, serde_json::to_vec(&legacy).unwrap()).unwrap();

    let before = fs::read(&index_path).unwrap();
    assert_eq!(
        store.list().unwrap()[0]
            .model_observations
            .as_ref()
            .unwrap()
            .schema_version,
        1
    );
    let annotated = store
        .annotate(&saved.id, TaskCategory::Debugging, TaskOutcome::Partial)
        .unwrap();
    assert_eq!(
        annotated
            .model_observations
            .as_ref()
            .unwrap()
            .schema_version,
        1,
        "ordinary mutation must not reinterpret recorded evidence"
    );
    assert_ne!(fs::read(&index_path).unwrap(), before);

    let reimported = store.import(SourceFormat::Codex, &file).unwrap();
    assert_eq!(reimported.id, saved.id);
    assert_eq!(
        reimported
            .model_observations
            .as_ref()
            .unwrap()
            .schema_version,
        2,
        "explicit reimport must replace even identical-digest legacy evidence"
    );
    assert_eq!(
        reimported.manual_annotation, annotated.manual_annotation,
        "reimport retains the user's independent annotation"
    );
}

#[test]
fn time_evidence_follows_exact_content_alias_and_deletion_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let first_path = temp.path().join("first.jsonl");
    let alias_path = temp.path().join("alias.jsonl");
    source(&first_path, "PRIVATE-ONE");
    fs::copy(&first_path, &alias_path).unwrap();
    let store = LocalInsightStore::open(&temp.path().join("store")).unwrap();
    let index_path = store.dir.join("index.json");
    let first = store.import(SourceFormat::Trajectory, &first_path).unwrap();
    let alias = store.import(SourceFormat::Trajectory, &alias_path).unwrap();
    assert_eq!(alias.id, first.id);
    assert_eq!(alias.time_evidence, first.time_evidence);
    assert_eq!(
        first.time_evidence.as_ref().unwrap().source_digest,
        first.report.evidence[0].source_digest
    );

    source(&first_path, "PRIVATE-TWO");
    let replacement = store.import(SourceFormat::Trajectory, &first_path).unwrap();
    assert_ne!(replacement.id, first.id);
    assert_eq!(
        store.explain(&first.id).unwrap().time_evidence,
        first.time_evidence
    );
    assert!(store.delete(&replacement.id).unwrap());
    let before_noop = fs::read(&index_path).unwrap();
    assert!(
        !store.delete(&replacement.id).unwrap(),
        "delete no-op stays false"
    );
    assert_eq!(fs::read(&index_path).unwrap(), before_noop);
    assert_eq!(
        store.explain(&first.id).unwrap().time_evidence,
        first.time_evidence
    );
    assert!(store.delete(&first.id).unwrap());
    assert!(store.list().unwrap().is_empty());
    assert!(first_path.exists() && alias_path.exists());
}

/// The plan requires that RFC 3339 subsecond precision is not silently
/// reduced. Nothing else in the suite persists a fractional second, so a
/// serializer change that truncated to whole seconds would otherwise pass.
#[test]
fn subsecond_recorded_timestamps_survive_the_store_round_trip() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("subsecond.jsonl");
    fs::write(
        &path,
        "{\"role\":\"meta\",\"source\":\"fixture\"}\n         {\"role\":\"user\",\"content\":\"PRIVATE\",\"timestamp\":\"2026-09-11T00:00:00.123456789Z\"}\n",
    )
    .unwrap();
    let store = LocalInsightStore::open(&temp.path().join("store")).unwrap();
    let imported = store.import(SourceFormat::Trajectory, &path).unwrap();
    let extremum = imported
        .time_evidence
        .as_ref()
        .unwrap()
        .earliest
        .clone()
        .unwrap();
    assert_eq!(
        extremum.recorded_at.timestamp_subsec_nanos(),
        123_456_789,
        "extraction keeps every fractional digit"
    );
    let reloaded = LocalInsightStore::open(&temp.path().join("store"))
        .unwrap()
        .explain(&imported.id)
        .unwrap();
    assert_eq!(
        reloaded.time_evidence.as_ref().unwrap().earliest,
        Some(extremum),
        "persistence keeps every fractional digit"
    );
}

#[test]
fn malformed_persisted_time_evidence_fails_closed_without_panicking() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("source.jsonl");
    source(&file, "PRIVATE");
    let store = LocalInsightStore::open(&temp.path().join("store")).unwrap();
    let saved = store.import(SourceFormat::Trajectory, &file).unwrap();
    let index_path = store.dir.join("index.json");
    let valid: serde_json::Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    let corruptions = [
        ("source_digest", serde_json::Value::String("0".repeat(64))),
        ("source_format", serde_json::Value::String("codex".into())),
        ("valid_timestamps", serde_json::json!(u64::MAX)),
        ("schema_version", serde_json::json!(2)),
    ];
    for (field, value) in corruptions {
        let mut corrupt = valid.clone();
        corrupt["reports"][&saved.id]["time_evidence"][field] = value;
        fs::write(&index_path, serde_json::to_vec(&corrupt).unwrap()).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_quarantined(&store, &saved.id, field)
        }));
        assert!(result.is_ok(), "persisted {field} caused a panic");
    }
}

fn report_with(passed: u64) -> outcomes::OutcomeEvidence {
    outcomes::OutcomeEvidence::TestReport(
        outcomes::parse_test_report(
            format!(
                r#"{{"schema_version":1,"runner":"cargo-test","passed":{passed},"failed":0,
                "skipped":0,"observed_at":"2026-09-11T00:00:00Z","commit_id":null}}"#
            )
            .as_bytes(),
        )
        .unwrap(),
    )
}

#[test]
fn unlinking_an_unknown_evidence_id_is_refused_and_leaves_the_index_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("source.jsonl");
    source(&file, "PRIVATE-UNLINK");
    let store = LocalInsightStore::open(&temp.path().join("store")).unwrap();
    let saved = store.import(SourceFormat::Trajectory, &file).unwrap();
    store.link_outcome(&saved.id, report()).unwrap();
    let before = fs::read(store.dir.join("index.json")).unwrap();
    let error = store
        .unlink_outcome(&saved.id, &"f".repeat(64))
        .unwrap_err()
        .to_string();
    assert_eq!(error, "insights_evidence_link_not_found");
    assert_eq!(fs::read(store.dir.join("index.json")).unwrap(), before);
    // The link that does exist still removes, exactly once.
    let linked_id = store.explain(&saved.id).unwrap().outcome_links[0]
        .id
        .clone();
    assert!(
        store
            .unlink_outcome(&saved.id, &linked_id)
            .unwrap()
            .outcome_links
            .is_empty()
    );
    assert_eq!(
        store
            .unlink_outcome(&saved.id, &linked_id)
            .unwrap_err()
            .to_string(),
        "insights_evidence_link_not_found"
    );
}

#[test]
fn the_outcome_link_bound_is_exact_and_refuses_the_next_association() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("source.jsonl");
    source(&file, "PRIVATE-BOUND");
    let store = LocalInsightStore::open(&temp.path().join("store")).unwrap();
    let saved = store.import(SourceFormat::Trajectory, &file).unwrap();
    for passed in 0..MAX_OUTCOME_LINKS as u64 {
        store.link_outcome(&saved.id, report_with(passed)).unwrap();
    }
    assert_eq!(
        store.explain(&saved.id).unwrap().outcome_links.len(),
        MAX_OUTCOME_LINKS
    );
    assert_eq!(
        store
            .link_outcome(&saved.id, report_with(MAX_OUTCOME_LINKS as u64))
            .unwrap_err()
            .to_string(),
        "insights_outcome_links_full"
    );
    // A repeated association inside the bound is still idempotent, not refused.
    assert_eq!(
        store
            .link_outcome(&saved.id, report_with(0))
            .unwrap()
            .outcome_links
            .len(),
        MAX_OUTCOME_LINKS
    );
    assert_eq!(store.list().unwrap().len(), 1);
}

#[test]
fn a_future_dated_link_stays_readable_once_the_clock_is_corrected() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("source.jsonl");
    source(&file, "PRIVATE-CLOCK");
    let store = LocalInsightStore::open(&temp.path().join("store")).unwrap();
    let saved = store.import(SourceFormat::Trajectory, &file).unwrap();
    store.link_outcome(&saved.id, report()).unwrap();
    let index_path = store.dir.join("index.json");
    let mut index: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    // Written while the machine's clock was a year fast. The entry itself is
    // structurally intact; only the wall clock disagrees with it, and that
    // disagreement must not make a saved snapshot unreadable.
    let ahead = (chrono::Utc::now() + chrono::Duration::days(365)).to_rfc3339();
    let link = &mut index["reports"][&saved.id]["outcome_links"][0];
    link["linked_at"] = ahead.clone().into();
    link["evidence"]["evidence"]["observed_at"] = ahead.clone().into();
    link["evidence"]["evidence"]["imported_at"] = ahead.into();
    fs::write(&index_path, serde_json::to_vec(&index).unwrap()).unwrap();
    assert_eq!(store.list().unwrap().len(), 1);
    assert!(store.quarantine().unwrap().is_empty());
    assert_eq!(store.explain(&saved.id).unwrap().outcome_links.len(), 1);
    // The bound still holds where it belongs: on a value being recorded now.
    let mut ahead = match report() {
        outcomes::OutcomeEvidence::TestReport(evidence) => evidence,
        _ => unreachable!(),
    };
    ahead.observed_at = chrono::Utc::now() + chrono::Duration::days(365);
    ahead.imported_at = ahead.observed_at;
    assert!(
        store
            .link_outcome(&saved.id, outcomes::OutcomeEvidence::TestReport(ahead))
            .is_err()
    );
}
