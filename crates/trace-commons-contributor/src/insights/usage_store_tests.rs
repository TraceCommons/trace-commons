use super::*;
use serde_json::{Value, json};
use trace_commons_protocol::insights_usage_series::{DigestKey, DigestKeyError, DigestKeyStore};

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
        report.remove("session_identity");
        if version < 5 {
            report.remove("time_evidence");
        }
        if version < 3 {
            report.remove("model_observations");
            report.remove("outcome_links");
        } else {
            // A store this old never carried a post-legacy nested model schema;
            // the schema and the store version advance together.
            report["model_observations"]["schema_version"] = 1.into();
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
        ("schema_version", json!(3)),
        ("complete_records", json!(u64::MAX)),
    ] {
        let mut corrupted = valid.clone();
        corrupted["reports"][&saved.id]["usage_evidence"][field] = value;
        fs::write(&index_path, serde_json::to_vec(&corrupted).unwrap()).unwrap();
        // The snapshot is withheld, not the store: one unreadable entry must
        // not cost the caller every other entry they saved.
        super::assert_quarantined(&store, &saved.id, field);
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
        super::assert_quarantined(
            &store,
            &saved.id,
            &format!("usage evidence in a version {version} store"),
        );
    }
}

fn claude_fixture() -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/insights/claude-turn-series/session.jsonl"),
    )
    .unwrap()
}

#[test]
fn saved_claude_analysis_persists_usage_timestamps_and_the_turn_series() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("PRIVATE_SELECTED_NAME.jsonl");
    fs::write(&path, claude_fixture()).unwrap();
    let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
    let saved = store.import(SourceFormat::ClaudeCode, &path).unwrap();
    let digest = &saved.report.evidence[0].source_digest;

    let usage = saved
        .usage_evidence
        .as_ref()
        .expect("claude usage evidence");
    usage
        .validate_binding(SourceFormat::ClaudeCode, digest)
        .unwrap();
    assert_eq!(usage.source, usage::UsageSource::ClaudeCode);
    assert_eq!(usage.candidate_records, 7);
    assert_eq!(usage.complete_records, 5);
    // Two records are unstated, so the file's total is unknown, not partial.
    assert!(usage.aggregate_counts.is_none());

    let time = saved.time_evidence.as_ref().expect("claude time evidence");
    assert_eq!(time.source_format, SourceFormat::ClaudeCode);
    assert_eq!(time.total_eligible_records, 11);
    assert_eq!(time.missing_timestamps, 1);

    let series = saved.turn_series.as_ref().expect("claude turn series");
    series.validate_binding(digest).unwrap();
    assert_eq!(series.series.turns.len(), 6);
    assert_eq!(series.series.tool_calls.len(), 5);
    assert!(saved.estimated_cost_usd.is_none());

    // It survives a reload, and the store holds nothing readable from the
    // source: no path, body, command, or message, tool or session ID.
    assert_eq!(
        store.explain(&saved.id).unwrap().turn_series,
        saved.turn_series
    );
    let index = fs::read_to_string(store.dir.join("index.json")).unwrap();
    for private in [
        "PRIVATE",
        "/Users",
        "cargo test",
        "toolu_",
        "11111111-2222",
        root.path().to_str().unwrap(),
    ] {
        assert!(!index.contains(private), "{private} reached the store");
    }
    let saved_json: Value = serde_json::from_str(&index).unwrap();
    assert_eq!(saved_json["version"], super::STORE_VERSION);
}

/// Unsaved analysis computes the evidence that needs no key; the per-turn
/// rows are made only for a saved snapshot, under the store's key.
#[test]
fn unsaved_claude_analysis_has_usage_and_time_but_no_series() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.jsonl");
    fs::write(&path, claude_fixture()).unwrap();
    let analyzed = analyze_file(SourceFormat::ClaudeCode, &path).unwrap();
    assert!(analyzed.usage_evidence.is_some());
    assert!(analyzed.time_evidence.is_some());
    assert!(analyzed.turn_series.is_none());
}

/// A store saved before version 13 keeps its Claude snapshots without usage
/// (unknown until reimported) and its schema-1 Codex usage, and upgrades.
#[test]
fn a_version_12_store_keeps_old_snapshots_and_upgrades() {
    let root = tempfile::tempdir().unwrap();
    let claude = root.path().join("claude.jsonl");
    let codex = root.path().join("codex.jsonl");
    fs::write(&claude, claude_fixture()).unwrap();
    source(&codex, 150);
    let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
    let claude_saved = store.import(SourceFormat::ClaudeCode, &claude).unwrap();
    let codex_saved = store.import(SourceFormat::Codex, &codex).unwrap();
    let index_path = store.dir.join("index.json");
    let mut legacy: Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    legacy["version"] = 12.into();
    let report = legacy["reports"][&claude_saved.id].as_object_mut().unwrap();
    report.remove("turn_series");
    report.remove("usage_evidence");
    report.remove("time_evidence");
    report.remove("session_identity");
    let codex_report = legacy["reports"][&codex_saved.id].as_object_mut().unwrap();
    codex_report.remove("session_identity");
    legacy["reports"][&codex_saved.id]["usage_evidence"]["schema_version"] = 1.into();
    fs::write(&index_path, serde_json::to_vec(&legacy).unwrap()).unwrap();

    assert!(store.quarantine().unwrap().is_empty());
    let old_claude = store.explain(&claude_saved.id).unwrap();
    assert!(old_claude.turn_series.is_none());
    assert!(old_claude.usage_evidence.is_none());
    let old_codex = store.explain(&codex_saved.id).unwrap();
    assert_eq!(old_codex.usage_evidence.unwrap().schema_version, 1);

    store
        .annotate(&claude_saved.id, TaskCategory::Docs, TaskOutcome::Partial)
        .unwrap();
    let upgraded: Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    assert_eq!(upgraded["version"], 13);
    assert!(store.quarantine().unwrap().is_empty());

    let reimported = store.import(SourceFormat::ClaudeCode, &claude).unwrap();
    assert_eq!(reimported.id, claude_saved.id);
    assert!(reimported.turn_series.is_some());
}

#[test]
fn claude_series_or_usage_in_an_older_or_mismatched_snapshot_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let claude = root.path().join("claude.jsonl");
    let codex = root.path().join("codex.jsonl");
    fs::write(&claude, claude_fixture()).unwrap();
    source(&codex, 150);
    let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
    let saved = store.import(SourceFormat::ClaudeCode, &claude).unwrap();
    let codex_saved = store.import(SourceFormat::Codex, &codex).unwrap();
    let index_path = store.dir.join("index.json");
    let valid: Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();

    for field in ["turn_series", "usage_evidence", "time_evidence"] {
        let mut older = valid.clone();
        older["version"] = 12.into();
        for other in [
            "turn_series",
            "usage_evidence",
            "time_evidence",
            "session_identity",
        ] {
            if other != field {
                older["reports"][&saved.id]
                    .as_object_mut()
                    .unwrap()
                    .remove(other);
            }
        }
        fs::write(&index_path, serde_json::to_vec(&older).unwrap()).unwrap();
        super::assert_quarantined(&store, &saved.id, &format!("Claude {field} in a v12 store"));
    }

    let mut rebound = valid.clone();
    rebound["reports"][&saved.id]["turn_series"]["source_digest"] = json!("f".repeat(64));
    fs::write(&index_path, serde_json::to_vec(&rebound).unwrap()).unwrap();
    super::assert_quarantined(&store, &saved.id, "series bound to other bytes");

    let mut misplaced = valid.clone();
    misplaced["reports"][&codex_saved.id]["turn_series"] =
        valid["reports"][&saved.id]["turn_series"].clone();
    fs::write(&index_path, serde_json::to_vec(&misplaced).unwrap()).unwrap();
    super::assert_quarantined(&store, &codex_saved.id, "a Claude series on Codex");
}

/// Without a key the snapshot is still saved, with usage and time evidence,
/// and no series: never one made under a substitute key.
#[test]
fn a_store_without_a_digest_key_saves_no_series() {
    struct Refusing;
    impl DigestKeyStore for Refusing {
        fn load(&self) -> Result<Option<DigestKey>, DigestKeyError> {
            Err(DigestKeyError::Unavailable("refusing"))
        }
        fn load_or_create(&self) -> Result<DigestKey, DigestKeyError> {
            Err(DigestKeyError::Unavailable("refusing"))
        }
        fn clear(&self) -> Result<(), DigestKeyError> {
            Ok(())
        }
    }
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("claude.jsonl");
    fs::write(&path, claude_fixture()).unwrap();
    let store =
        LocalInsightStore::open_with_digest_keys(&root.path().join("store"), Box::new(Refusing))
            .unwrap();
    let saved = store.import(SourceFormat::ClaudeCode, &path).unwrap();
    assert!(saved.turn_series.is_none());
    assert!(saved.usage_evidence.is_some());
}

/// A saved snapshot names the harness session it records by a keyed digest,
/// so the overlap rule can tell a reimport from a concurrent session. Never
/// readable, never on unsaved analysis.
#[test]
fn saved_snapshots_carry_a_keyed_session_identity() {
    let root = tempfile::tempdir().unwrap();
    let claude = root.path().join("claude.jsonl");
    let codex = root.path().join("codex.jsonl");
    fs::write(&claude, claude_fixture()).unwrap();
    source(&codex, 150);
    let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
    let codex_saved = store.import(SourceFormat::Codex, &codex).unwrap();
    let claude_saved = store.import(SourceFormat::ClaudeCode, &claude).unwrap();
    let codex_identity = codex_saved
        .session_identity
        .clone()
        .expect("codex identity");
    let claude_identity = claude_saved
        .session_identity
        .clone()
        .expect("claude identity");
    assert_ne!(codex_identity.session, claude_identity.session);
    assert_eq!(
        Some(codex_identity.key_fingerprint),
        claude_saved
            .turn_series
            .as_ref()
            .map(|series| series.key_fingerprint),
        "one store key"
    );
    assert_eq!(
        store.explain(&codex_saved.id).unwrap().session_identity,
        Some(codex_identity)
    );
    let index = fs::read_to_string(store.dir.join("index.json")).unwrap();
    assert!(
        !index.contains("PRIVATE_SESSION_ID"),
        "session ID reached the store"
    );
    assert!(
        analyze_file(SourceFormat::Codex, &codex)
            .unwrap()
            .session_identity
            .is_none()
    );
}

#[test]
fn a_session_identity_in_an_older_or_malformed_snapshot_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let codex = root.path().join("codex.jsonl");
    source(&codex, 150);
    let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
    let saved = store.import(SourceFormat::Codex, &codex).unwrap();
    let index_path = store.dir.join("index.json");
    let valid: Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();

    let mut older = valid.clone();
    older["version"] = 12.into();
    older["reports"][&saved.id]["usage_evidence"]["schema_version"] = 1.into();
    fs::write(&index_path, serde_json::to_vec(&older).unwrap()).unwrap();
    super::assert_quarantined(&store, &saved.id, "session identity in a v12 store");

    let mut malformed = valid.clone();
    malformed["reports"][&saved.id]["session_identity"]["schema_version"] = 99.into();
    fs::write(&index_path, serde_json::to_vec(&malformed).unwrap()).unwrap();
    super::assert_quarantined(&store, &saved.id, "unknown session identity schema");
}

/// Two Codex sessions that ran at the same time, saved from a store holding
/// no Claude snapshot, are both counted; the same session saved twice is
/// counted once.
#[test]
fn concurrent_codex_sessions_saved_to_the_store_both_count() {
    use super::week_rollup::{CoverageReason, Feed, week_rollup};
    let overlapped = |rollup: &super::week_rollup::WeekRollup| {
        rollup
            .sessions
            .iter()
            .filter(|session| session.reasons.contains(&CoverageReason::ReimportOverlap))
            .count()
    };
    let root = tempfile::tempdir().unwrap();
    let rollout = |name: &str, id: &str, final_input: u64| {
        let path = root.path().join(name);
        source(&path, final_input);
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("PRIVATE_SESSION_ID", id);
        fs::write(&path, text).unwrap();
        path
    };
    let one = rollout("one.jsonl", "session-one", 150);
    let two = rollout("two.jsonl", "session-two", 160);
    let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
    store.import(SourceFormat::Codex, &one).unwrap();
    store.import(SourceFormat::Codex, &two).unwrap();
    let monday = chrono::NaiveDate::from_ymd_opt(2026, 9, 7).unwrap();
    let rollup = |store: &LocalInsightStore| {
        week_rollup(
            Feed::Saved,
            &week_glance::saved_sessions(&store.list().unwrap()),
            monday,
            &chrono::Utc,
        )
    };
    let both = rollup(&store);
    assert_eq!(both.sessions.len(), 2);
    assert_eq!(overlapped(&both), 0, "{:?}", both.sessions);

    // The same session, grown and saved again from another file.
    let again = rollout("again.jsonl", "session-one", 170);
    store.import(SourceFormat::Codex, &again).unwrap();
    let reimported = rollup(&store);
    assert_eq!(reimported.sessions.len(), 3);
    assert_eq!(overlapped(&reimported), 1, "{:?}", reimported.sessions);
}
