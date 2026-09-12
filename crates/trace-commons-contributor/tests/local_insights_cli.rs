//! Local analysis must stay independent of enrollment and preserve unknown evidence.
use std::path::Path;
use std::process::{Command, Output};

#[test]
fn question_cards_use_saved_evidence_and_invalidate_deleted_selections() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("enrollment");
    let store = dir.path().join("insights");
    let empty = value(invoke(&config, &store, &["cards"]));
    assert_eq!(empty["type"], "question_cards");
    assert_eq!(empty["result"]["cards"].as_array().unwrap().len(), 4);
    assert!(!store.exists());
    assert!(!config.exists());

    let file = dir.path().join("PRIVATE_SOURCE.jsonl");
    fixture(&file, false);
    let saved = value(invoke(
        &config,
        &store,
        &[
            "analyze",
            "--source",
            "trajectory",
            "--file",
            file.to_str().unwrap(),
            "--save",
        ],
    ));
    let id = saved["id"].as_str().unwrap();
    std::fs::remove_file(&file).unwrap();
    let cards = value(invoke(&config, &store, &["cards", "--snapshot", id]));
    assert_eq!(
        cards,
        value(invoke(&config, &store, &["cards", "--snapshot", id]))
    );
    let rows = cards["result"]["cards"][0]["rows"].as_array().unwrap();
    let row = |name| rows.iter().find(|row| row["id"] == name).unwrap();
    assert_eq!(
        row("saved_snapshots")["value"],
        serde_json::json!({"type":"count","value":1})
    );
    assert_eq!(
        row("record_span")["value"],
        serde_json::json!({"type":"milliseconds","value":60000})
    );
    assert!(cards["result"]["cards"][3]["rows"][0]["value"].is_null());
    let text = cards["text"].as_str().unwrap();
    assert!(text.contains("Span between recorded events: 60000 ms"));
    assert!(text.contains("Applicable versioned pricing evidence is not available."));
    for private in [
        "SECRET_FIXTURE_BODY",
        "PRIVATE_SOURCE",
        "/private/fixture-project",
    ] {
        assert!(!cards.to_string().contains(private));
    }
    let plain = Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"))
        .arg("--config-dir")
        .arg(&config)
        .args(["insights", "--store-dir"])
        .arg(&store)
        .args(["cards", "--snapshot", id])
        .output()
        .unwrap();
    assert!(plain.status.success());
    assert_eq!(
        String::from_utf8(plain.stdout).unwrap(),
        format!("{text}\n")
    );

    value(invoke(&config, &store, &["delete", id]));
    let missing = invoke(&config, &store, &["cards", "--snapshot", id]);
    assert!(!missing.status.success());
    let error: serde_json::Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(error["error"], "insights_card_snapshot_not_found");
    assert!(!config.exists());
}

#[test]
fn empty_history_does_not_initialize_insights_or_enrollment() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("enrollment");
    let store = dir.path().join("insights");
    assert_eq!(
        value(invoke(&config, &store, &["list"])),
        serde_json::json!([])
    );
    assert!(!store.exists());
    assert!(!config.exists());
}

fn invoke(config: &Path, store: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"))
        .arg("--config-dir")
        .arg(config)
        .args(["--json", "insights", "--store-dir"])
        .arg(store)
        .args(args)
        .output()
        .unwrap()
}

fn value(output: Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn fixture(path: &Path, extra: bool) {
    let mut text = concat!(
        "{\"role\":\"meta\",\"source\":\"claude-code\",\"model\":\"fixture-model\",\"cwd\":\"/private/fixture-project\"}\n",
        "{\"role\":\"user\",\"content\":\"SECRET_FIXTURE_BODY\",\"timestamp\":\"2026-09-11T10:00:00Z\"}\n",
        "{\"role\":\"assistant\",\"content\":\"Tests passed, task complete\",\"timestamp\":\"2026-09-11T10:01:00Z\"}\n"
    ).to_owned();
    if extra {
        text.push_str("{\"role\":\"user\",\"content\":\"Please revise\",\"timestamp\":\"2026-09-11T10:02:00Z\"}\n");
    }
    std::fs::write(path, text).unwrap();
}

#[test]
fn analyze_without_save_never_creates_insights_or_enrollment_state() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("enrollment");
    let store = dir.path().join("insights");
    let file = dir.path().join("session.jsonl");
    fixture(&file, false);
    let result = value(invoke(
        &config,
        &store,
        &[
            "analyze",
            "--source",
            "trajectory",
            "--file",
            file.to_str().unwrap(),
        ],
    ));
    assert!(!config.exists());
    assert!(!store.exists());
    assert!(result["estimated_cost_usd"].is_null());
    let serialized = result.to_string();
    assert!(!serialized.contains("SECRET_FIXTURE_BODY"));
    assert!(!serialized.contains("/private/fixture-project"));
    assert!(!serialized.contains("Tests passed, task complete"));
}

#[test]
fn saved_insights_deduplicate_replace_explain_and_delete_without_enrollment() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("enrollment");
    let store = dir.path().join("insights");
    let file = dir.path().join("session.jsonl");
    fixture(&file, false);
    let args = [
        "analyze",
        "--source",
        "trajectory",
        "--file",
        file.to_str().unwrap(),
        "--save",
    ];
    let first = value(invoke(&config, &store, &args));
    let again = value(invoke(&config, &store, &args));
    assert_eq!(first["id"], again["id"]);
    let listed = value(invoke(&config, &store, &["list"]));
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let id = first["id"].as_str().unwrap();
    let explained = value(invoke(&config, &store, &["explain", id]));
    let mut saved_fields = again.clone();
    assert_eq!(
        saved_fields
            .as_object_mut()
            .unwrap()
            .remove("mutation_effects"),
        Some(serde_json::json!({
            "invalidated_episode_ids": [],
            "stale_comparison_task_ids": [],
            "stale_comparison_tasks": []
        }))
    );
    assert_eq!(explained, saved_fields);

    fixture(&file, true);
    let changed = value(invoke(&config, &store, &args));
    assert_ne!(changed["id"], first["id"]);
    assert_eq!(
        value(invoke(&config, &store, &["list"]))
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(!invoke(&config, &store, &["explain", id]).status.success());

    let id = changed["id"].as_str().unwrap();
    assert_eq!(
        value(invoke(&config, &store, &["delete", id]))["deleted"],
        true
    );
    assert_eq!(
        value(invoke(&config, &store, &["delete", id]))["deleted"],
        false
    );
    assert_eq!(
        value(invoke(&config, &store, &["list"])),
        serde_json::json!([])
    );
    assert!(
        file.exists(),
        "deleting a derived insight must preserve the source file"
    );
    assert!(!config.exists());
}

#[test]
fn malformed_input_returns_a_safe_error_and_no_report() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("session.jsonl");
    std::fs::write(&file, "SECRET_MALFORMED_INPUT").unwrap();
    let output = invoke(
        &dir.path().join("enrollment"),
        &dir.path().join("insights"),
        &[
            "analyze",
            "--source",
            "trajectory",
            "--file",
            file.to_str().unwrap(),
        ],
    );
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(error["error"].is_string());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("SECRET_MALFORMED_INPUT"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("SECRET_MALFORMED_INPUT"));
}

#[test]
fn user_assessment_is_separate_from_measured_outcomes_and_clears() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("enrollment");
    let store = dir.path().join("insights");
    let file = dir.path().join("session.jsonl");
    fixture(&file, false);
    let first = value(invoke(
        &config,
        &store,
        &[
            "analyze",
            "--source",
            "trajectory",
            "--file",
            file.to_str().unwrap(),
            "--save",
        ],
    ));
    let id = first["id"].as_str().unwrap();
    let annotated = value(invoke(
        &config,
        &store,
        &[
            "annotate",
            id,
            "--category",
            "refactor",
            "--outcome",
            "accepted",
        ],
    ));
    assert_eq!(annotated["manual_annotation"]["category"], "refactor");
    assert_eq!(annotated["manual_annotation"]["outcome"], "accepted");
    assert_eq!(
        annotated["manual_annotation"]["provenance"],
        "user_reported"
    );
    let outcome = annotated["report"]["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "known_outcomes")
        .unwrap();
    assert!(outcome["value"].is_null());
    let cleared = value(invoke(&config, &store, &["clear-annotation", id]));
    assert!(cleared["manual_annotation"].is_null());
    assert!(!config.exists());
}

#[test]
fn native_usage_preserves_cumulative_accounting_without_writing_state() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("enrollment");
    let store = dir.path().join("insights");
    let file = dir.path().join("usage.jsonl");
    let mut rows = Vec::new();
    for input in [100, 200] {
        rows.push(serde_json::json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":input,"cached_input_tokens":50,"output_tokens":10,"reasoning_output_tokens":5,"total_tokens":input+10}}}}).to_string());
    }
    std::fs::write(&file, rows.join("\n")).unwrap();
    let result = value(invoke(
        &config,
        &store,
        &[
            "usage",
            "--source",
            "codex",
            "--file",
            file.to_str().unwrap(),
        ],
    ));
    assert_eq!(result["counts"]["input"], 200);
    assert_eq!(result["counts"]["total"], 210);
    assert_eq!(result["complete_records"], 2);
    assert!(result["unavailable_reason"].is_null());
    assert!(!store.exists());
    assert!(!config.exists());
}

#[test]
fn codex_counts_and_missingness_match_the_human_view() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("enrollment");
    let store = dir.path().join("insights");
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "fixtures/codex/2026/07/01/rollout-2026-07-01T10-00-00-22222222-2222-2222-2222-222222222222.jsonl",
    );
    let args = [
        "analyze",
        "--source",
        "codex",
        "--file",
        file.to_str().unwrap(),
    ];
    let result = value(invoke(&config, &store, &args));
    let metrics = result["report"]["metrics"].as_array().unwrap();
    let metric = |id| metrics.iter().find(|m| m["id"] == id).unwrap();
    assert_eq!(metric("tool_calls")["value"], 1);
    assert!(metric("tool_failures")["value"].is_null());
    assert!(metric("input_tokens")["value"].is_null());
    assert!(metric("known_outcomes")["value"].is_null());

    let output = Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"))
        .arg("--config-dir")
        .arg(&config)
        .args(["insights", "--store-dir"])
        .arg(&store)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Tool calls: 1"));
    assert!(text.contains("Input tokens: unknown"));
    assert!(text.contains("Known task outcomes: unknown"));
    assert!(text.contains("Analysis by trace-commons-local"));
    assert!(!text.contains("Add a healthcheck endpoint"));
    assert!(!config.exists());
    assert!(!store.exists());
}

#[cfg(unix)]
#[test]
fn a_selected_fifo_is_refused_without_waiting_for_a_writer() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("source.fifo");
    assert!(
        Command::new("mkfifo")
            .arg(&file)
            .status()
            .unwrap()
            .success()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"))
        .args([
            "--json", "insights", "analyze", "--source", "codex", "--file",
        ])
        .arg(&file)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(!status.success());
            let output = child.wait_with_output().unwrap();
            let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(error["error"], "insights_source_not_file");
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("analysis blocked on a FIFO");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn summary_counts_only_saved_snapshots_and_separates_manual_assessments() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("enrollment");
    let store = dir.path().join("insights");
    let empty = value(invoke(&config, &store, &["summary"]));
    assert_eq!(empty["saved_snapshots"], 0);
    assert!(!store.exists() && !config.exists());
    let file = dir.path().join("source.jsonl");
    fixture(&file, false);
    let result = value(invoke(
        &config,
        &store,
        &[
            "analyze",
            "--source",
            "trajectory",
            "--file",
            file.to_str().unwrap(),
            "--save",
        ],
    ));
    let id = result["id"].as_str().unwrap();
    value(invoke(
        &config,
        &store,
        &[
            "annotate",
            id,
            "--category",
            "tests",
            "--outcome",
            "partial",
        ],
    ));
    let summary = value(invoke(&config, &store, &["summary"]));
    let service_json = trace_commons_contributor::insights::service::dispatch_json(
        &serde_json::to_vec(&serde_json::json!({"store_dir":store,"operation":{"type":"summary"}}))
            .unwrap(),
    )
    .unwrap();
    let service: serde_json::Value = serde_json::from_str(&service_json).unwrap();
    assert_eq!(
        service["summary"], summary,
        "CLI and native service share the exact reducer"
    );
    assert_eq!(summary["scope"], "all_saved_selected_session_snapshots");
    assert_eq!(summary["saved_snapshots"], 1);
    assert_eq!(summary["user_reported"]["assessed_snapshots"], 1);
    assert_eq!(summary["user_reported"]["unassessed_snapshots"], 0);
    assert_eq!(summary["snapshots"][0]["id"], id);
    let outcomes = summary["user_reported"]["outcomes"].as_array().unwrap();
    assert_eq!(
        outcomes.iter().find(|o| o["outcome"] == "partial").unwrap()["snapshots"],
        1
    );
    assert!(!summary.to_string().contains("SECRET_FIXTURE_BODY"));
    assert!(!config.exists());
    value(invoke(&config, &store, &["delete", id]));
    assert_eq!(
        value(invoke(&config, &store, &["summary"]))["saved_snapshots"],
        0
    );
}

#[test]
fn explicit_test_evidence_links_are_local_removable_and_not_verified_outcomes() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("enrollment");
    let store = dir.path().join("insights");
    let file = dir.path().join("source.jsonl");
    fixture(&file, false);
    let saved = value(invoke(
        &config,
        &store,
        &[
            "analyze",
            "--source",
            "trajectory",
            "--file",
            file.to_str().unwrap(),
            "--save",
        ],
    ));
    let id = saved["id"].as_str().unwrap();
    assert!(saved["model_observations"].is_object());
    let report = dir.path().join("report.json");
    std::fs::write(&report, br#"{"schema_version":1,"runner":"cargo-test","passed":3,"failed":1,"skipped":0,"observed_at":"2026-09-11T00:00:00Z","commit_id":null}"#).unwrap();
    let linked = value(invoke(
        &config,
        &store,
        &["link-test-report", id, "--file", report.to_str().unwrap()],
    ));
    let evidence_id = linked["outcome_links"][0]["id"].as_str().unwrap();
    assert_eq!(linked["outcome_links"][0]["provenance"], "user_linked");
    assert!(linked["manual_annotation"].is_null());
    let summary = value(invoke(&config, &store, &["summary"]));
    assert_eq!(summary["user_reported"]["assessed_snapshots"], 0);
    let metrics = summary["metrics"].as_array().unwrap();
    assert!(
        metrics
            .iter()
            .find(|m| m["id"] == "known_outcomes")
            .unwrap()["observed_value_sum"]
            .is_null()
    );
    let cleared = value(invoke(
        &config,
        &store,
        &["unlink-evidence", id, evidence_id],
    ));
    assert_eq!(cleared["outcome_links"].as_array().unwrap().len(), 0);
    assert!(report.exists() && file.exists() && !config.exists());
}

#[test]
fn episode_commands_keep_revisions_assessments_overlap_and_replacement_effects_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("enrollment");
    let store = temp.path().join("insights");
    assert_eq!(
        value(invoke(&config, &store, &["episode-list"]))["episodes"],
        serde_json::json!([])
    );
    assert!(!store.exists() && !config.exists());
    let file_a = temp.path().join("a.jsonl");
    let file_b = temp.path().join("b.jsonl");
    fixture(&file_a, false);
    fixture(&file_b, true);
    let save = |file: &Path| {
        value(invoke(
            &config,
            &store,
            &[
                "analyze",
                "--source",
                "trajectory",
                "--file",
                file.to_str().unwrap(),
                "--save",
            ],
        ))
    };
    let a = save(&file_a);
    let b = save(&file_b);
    let a_id = a["id"].as_str().unwrap();
    let b_id = b["id"].as_str().unwrap();
    let first = value(invoke(
        &config,
        &store,
        &["episode-create", "--snapshot", a_id, "--snapshot", b_id],
    ));
    let other = value(invoke(
        &config,
        &store,
        &["episode-create", "--snapshot", a_id],
    ));
    let id = first["episode"]["id"].as_str().unwrap();
    let other_id = other["episode"]["id"].as_str().unwrap();
    assert_eq!(
        first["episode"]["provenance"],
        "user_selected_whole_snapshots"
    );
    let detail = value(invoke(&config, &store, &["episode-explain", id]));
    assert_eq!(detail["detail"]["members"].as_array().unwrap().len(), 2);
    assert!(
        detail["detail"]["overlap"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["snapshot_id"] == a_id
                && entry["episode_ids"] == serde_json::json!([other_id]))
    );
    let annotated = value(invoke(
        &config,
        &store,
        &[
            "episode-annotate",
            id,
            "--expected-revision",
            "1",
            "--category",
            "tests",
            "--outcome",
            "accepted",
        ],
    ));
    assert_eq!(annotated["episode"]["revision"], 2);
    assert_eq!(annotated["episode"]["membership_revision"], 1);
    assert_eq!(
        annotated["episode"]["manual_assessment"]["provenance"],
        "user_reported"
    );
    let stale = invoke(
        &config,
        &store,
        &["episode-delete", id, "--expected-revision", "1"],
    );
    assert!(!stale.status.success());
    let error: serde_json::Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(error["error"], "insights_episode_revision_conflict");
    let replaced = value(invoke(
        &config,
        &store,
        &[
            "episode-replace-members",
            id,
            "--expected-revision",
            "2",
            "--snapshot",
            b_id,
        ],
    ));
    assert_eq!(replaced["episode"]["revision"], 3);
    assert_eq!(replaced["episode"]["membership_revision"], 2);
    assert!(replaced["episode"]["manual_assessment"].is_null());
    value(invoke(
        &config,
        &store,
        &[
            "episode-annotate",
            id,
            "--expected-revision",
            "3",
            "--category",
            "unknown",
            "--outcome",
            "unknown",
        ],
    ));
    let cleared = value(invoke(
        &config,
        &store,
        &["episode-clear-assessment", id, "--expected-revision", "4"],
    ));
    assert_eq!(cleared["episode"]["revision"], 5);
    assert!(cleared["episode"]["manual_assessment"].is_null());
    value(invoke(
        &config,
        &store,
        &["episode-delete", id, "--expected-revision", "5"],
    ));
    assert_eq!(
        value(invoke(&config, &store, &["list"]))
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let summary = value(invoke(&config, &store, &["summary"]));
    assert_eq!(summary["saved_snapshots"], 2);
    assert_eq!(summary["user_reported"]["assessed_snapshots"], 0);
    fixture(&file_a, true);
    let changed = save(&file_a);
    assert_eq!(
        changed["id"], b_id,
        "saved insight JSON keeps its existing top-level fields"
    );
    assert_eq!(
        changed["mutation_effects"]["invalidated_episode_ids"],
        serde_json::json!([other_id])
    );
    assert_eq!(
        value(invoke(&config, &store, &["episode-list"]))["episodes"],
        serde_json::json!([])
    );
    let last = value(invoke(
        &config,
        &store,
        &["episode-create", "--snapshot", b_id],
    ));
    let deleted = value(invoke(&config, &store, &["delete", b_id]));
    assert_eq!(deleted["deleted"], true);
    assert_eq!(
        deleted["mutation_effects"]["invalidated_episode_ids"],
        serde_json::json!([last["episode"]["id"]])
    );
    assert!(file_a.exists() && file_b.exists() && !config.exists());
}

#[test]
fn human_snapshot_delete_reports_lost_episode_groups() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("enrollment");
    let store = temp.path().join("insights");
    let file = temp.path().join("source.jsonl");
    fixture(&file, false);
    let snapshot = value(invoke(
        &config,
        &store,
        &[
            "analyze",
            "--source",
            "trajectory",
            "--file",
            file.to_str().unwrap(),
            "--save",
        ],
    ));
    let id = snapshot["id"].as_str().unwrap();
    let episode = value(invoke(
        &config,
        &store,
        &["episode-create", "--snapshot", id],
    ));
    let human_list = || {
        let output = Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"))
            .arg("--config-dir")
            .arg(&config)
            .args(["insights", "--store-dir"])
            .arg(&store)
            .arg("episode-list")
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    };
    let copy = trace_commons_contributor::insights::service::ui_copy();
    let text = human_list();
    assert!(text.contains(&copy["episode_unassessed"]));
    assert!(text.contains(&copy["episode_no_overlap"]));
    assert!(text.contains(id));
    assert!(
        !text.contains("\"schema_version\""),
        "human list must not dump raw JSON"
    );
    value(invoke(
        &config,
        &store,
        &[
            "episode-annotate",
            episode["episode"]["id"].as_str().unwrap(),
            "--expected-revision",
            "1",
            "--category",
            "tests",
            "--outcome",
            "accepted",
        ],
    ));
    let text = human_list();
    assert!(text.contains(&format!(
        "{}: {} / {}",
        copy["episode_assessment"], copy["category_tests"], copy["outcome_accepted"]
    )));
    let output = Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"))
        .arg("--config-dir")
        .arg(&config)
        .args(["insights", "--store-dir"])
        .arg(&store)
        .args(["delete", id])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let copy = trace_commons_contributor::insights::service::ui_copy();
    assert!(text.contains(&copy["episode_invalidated_notice"]));
    assert!(text.contains(episode["episode"]["id"].as_str().unwrap()));
    assert!(file.exists() && !config.exists());
}
