//! Local analysis must stay independent of enrollment and preserve unknown evidence.
use std::path::Path;
use std::process::{Command, Output};

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
    assert_eq!(value(invoke(&config, &store, &["explain", id])), again);

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
