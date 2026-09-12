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

#[test]
fn local_inbox_import_list_show_and_delete_preserve_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("inbox");
    let source = dir.path().join("proposal.json");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../trace-commons-protocol/tests/fixtures/mission-draft.json");
    let original = std::fs::read(fixture).unwrap();
    std::fs::write(&source, &original).unwrap();
    let binary = env!("CARGO_BIN_EXE_trace-commons-contributor");

    let empty = Command::new(binary)
        .args(["--json", "mission-drafts", "--store-dir"])
        .arg(&store)
        .arg("list")
        .output()
        .unwrap();
    assert!(empty.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&empty.stdout).unwrap(),
        serde_json::json!([])
    );
    assert!(!store.exists());

    let import = |source: &std::path::Path| {
        Command::new(binary)
            .args(["--json", "mission-drafts", "--store-dir"])
            .arg(&store)
            .args(["import", "--file"])
            .arg(source)
            .output()
            .unwrap()
    };
    let first = import(&source);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stdout)
    );
    let first: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(first["inserted"], true);
    let id = first["id"].as_str().unwrap();
    assert_eq!(id, first["review"]["proposal_sha256"]);
    assert!(first.get("proposal").is_none());
    let repeated = import(&source);
    assert!(repeated.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&repeated.stdout).unwrap()["inserted"],
        false
    );

    for action in ["list", "show"] {
        let mut command = Command::new(binary);
        command
            .args(["--json", "mission-drafts", "--store-dir"])
            .arg(&store)
            .arg(action);
        if action == "show" {
            command.arg(id);
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{action}");
    }
    let deleted = Command::new(binary)
        .args(["--json", "mission-drafts", "--store-dir"])
        .arg(&store)
        .args(["delete", id])
        .output()
        .unwrap();
    assert!(deleted.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&deleted.stdout).unwrap()["deleted"],
        true
    );
    assert_eq!(std::fs::read(source).unwrap(), original);
}

#[test]
fn missing_inbox_id_returns_a_fixed_error_without_creating_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("inbox");
    let output = Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"))
        .args(["--json", "mission-drafts", "--store-dir"])
        .arg(&store)
        .args(["show", &"0".repeat(64)])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"], "mission-draft-not-found");
    assert!(!store.exists());
}
