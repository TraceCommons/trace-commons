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
                user_input: "Review the result".into(),
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
            .args(["import-preview", "--file"])
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
        assert!(
            !state.exists(),
            "local preview must not create or alter enrollment state"
        );
    }
}
