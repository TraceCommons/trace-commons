//! Explicit local import works without enrollment, writes, or a source whitelist.
use trace_commons_protocol::evidence_import::EvidenceImport;
use trace_commons_protocol::trace_contribution::{
    RawTraceCaptureTurn, RawTraceContribution, RecordedTraceContributionOptions,
};

#[test]
fn two_ordinary_imports_preview_without_creating_contributor_state() {
    let dir = tempfile::tempdir().unwrap();
    for source in ["opencode", "second-client"] {
        let mut trace = RawTraceContribution::from_capture_turns(
            &[RawTraceCaptureTurn {
                user_input:
                    "Review /foreign/work/src/main.rs with sk-ant-EXPOSEDsecret0123456789abcdefghij"
                        .into(),
                response: Some("Done".into()),
                tool_calls: Vec::new(),
                started_at: chrono::Utc::now(),
                completed_at: None,
                state: None,
            }],
            RecordedTraceContributionOptions {
                include_message_text: true,
                ..Default::default()
            },
        );
        trace
            .ironclaw
            .feature_flags
            .insert("agent".into(), source.into());
        let path = dir.path().join(format!("{source}.json"));
        std::fs::write(
            &path,
            serde_json::to_vec(&EvidenceImport {
                schema_version: 1,
                trace,
                inference: None,
            })
            .unwrap(),
        )
        .unwrap();
        let state = dir.path().join(format!("{source}-state"));
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"))
            .arg("--config-dir")
            .arg(&state)
            .args(["import-preview", "--cwd", "/foreign/work", "--file"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["preview_only"], true);
        assert_eq!(value["admission_verified"], false);
        assert_eq!(value["coverage"], serde_json::Value::Null);
        let output_text = String::from_utf8(output.stdout).unwrap();
        assert!(!output_text.contains("/foreign/work"));
        assert!(!output_text.contains("sk-ant-EXPOSEDsecret0123456789abcdefghij"));
        assert!(value.get("artifact_sha256").is_none());
        assert!(value["preview_sha256"].is_string());
        assert!(
            !state.exists(),
            "local preview must not create or alter enrollment state"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_fifo_import_is_refused_without_waiting_for_a_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"))
        .args(["import-preview", "--cwd", "/foreign/work", "--file"])
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(!status.success());
            let output = child.wait_with_output().unwrap();
            assert!(
                String::from_utf8(output.stderr)
                    .unwrap()
                    .contains("import-file-not-regular")
            );
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("FIFO import blocked waiting for a writer");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn import_requires_explicit_redaction_context_before_opening_state() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("unused-state");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"))
        .arg("--config-dir")
        .arg(&state)
        .args(["import-preview", "--file", "unused-import.json"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "missing --cwd unexpectedly succeeded: stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--cwd"),
        "missing --cwd error was not reported (status={}): stderr={stderr}",
        output.status
    );
    assert!(!state.exists());
}
