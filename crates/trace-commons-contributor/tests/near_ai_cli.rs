use std::process::Command;

#[test]
fn near_ai_and_legacy_entry_points_share_existing_json_errors() {
    let home = tempfile::tempdir().unwrap();
    let mut values = Vec::new();
    for binary in [
        env!("CARGO_BIN_EXE_near-ai"),
        env!("CARGO_BIN_EXE_trace-commons-contributor"),
    ] {
        let output = Command::new(binary)
            .arg("--config-dir")
            .arg(home.path())
            .args(["--json", "import-preview", "--file"])
            .arg(home.path().join("missing.json"))
            .arg("--cwd")
            .arg(home.path())
            .output()
            .unwrap();
        assert!(!output.status.success());
        values.push(serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap());
    }
    assert_eq!(values[0], values[1]);
}

#[test]
fn canonical_cli_identifies_near_ai_and_preserves_existing_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_near-ai"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("NEAR AI"));
    assert!(help.contains("near-ai"));
    assert!(help.contains("daemon"));
    assert!(help.contains("login"));
}
