use std::process::Command;

#[test]
fn local_draft_intake_is_account_free_and_never_authorizes_publication() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let draft = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../trace-commons-protocol/tests/fixtures/mission-draft.json");
    let output = Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"))
        .arg("--config-dir")
        .arg(&config)
        .args(["--json", "mission-draft", "--file"])
        .arg(draft)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "needs_curator_review");
    assert_eq!(result["publication_authorized"], false);
    assert_eq!(result["external_sources_verified"], false);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("example.com"));
    assert!(!config.exists());
}

#[test]
fn source_instructions_cannot_grant_publication_authority() {
    let dir = tempfile::tempdir().unwrap();
    let draft = dir.path().join("draft.json");
    std::fs::write(&draft, r#"{"publish_now":true,"secret":"DO_NOT_ECHO"}"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"))
        .args(["--json", "mission-draft", "--file"])
        .arg(&draft)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("DO_NOT_ECHO"));
}
