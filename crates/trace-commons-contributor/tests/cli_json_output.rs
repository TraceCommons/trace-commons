use std::path::Path;
use std::process::{Command, Output};

use trace_commons_contributor::config::{
    CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig,
};

fn save_config(dir: &Path) {
    let store = ConfigStore::open(dir.to_path_buf()).expect("config store opens");
    store
        .save_config(&ContributorConfig {
            schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
            issuer_url: "https://issuer.example.invalid".to_string(),
            ingest_url: "https://ingest.example.invalid/v1/traces".to_string(),
            audience: "trace-commons".to_string(),
            tenant_id: "tenant-test".to_string(),
            instance_id: "instance-test".to_string(),
            user_subject: "test-user".to_string(),
            device_key_id: "device-test".to_string(),
            consent_scopes: vec!["debugging_evaluation".to_string()],
            pii_filter: None,
            allowed_hosts: Some("issuer.example.invalid,ingest.example.invalid".to_string()),
        })
        .expect("config saves");
}

fn parse_single_document(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout must contain exactly one JSON document: {error}; stdout={}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn contributor_command(config_dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"));
    command.arg("--json").arg("--config-dir").arg(config_dir);
    command
}

#[test]
fn json_empty_discovery_emits_one_empty_results_document() {
    let state = tempfile::tempdir().expect("state dir");
    let trajectories = tempfile::tempdir().expect("trajectory dir");
    save_config(state.path());

    let output = contributor_command(state.path())
        .args([
            "submit",
            "--dry-run",
            "--all",
            "--source",
            "trajectory",
            "--trajectory",
        ])
        .arg(trajectories.path())
        .output()
        .expect("CLI runs");

    assert!(output.status.success(), "unexpected status: {output:?}");
    let document = parse_single_document(&output);
    assert_eq!(document["schema_version"], "trace_commons.submit_result.v1");
    assert_eq!(document["results"], serde_json::json!([]));
}

#[test]
fn json_refusal_emits_one_results_document_and_exits_nonzero() {
    let state = tempfile::tempdir().expect("state dir");
    save_config(state.path());
    let trajectory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/letta-conformance/ok__minimal-turn.jsonl");

    let output = contributor_command(state.path())
        .args([
            "submit",
            "--dry-run",
            "--all",
            "--source",
            "trajectory",
            "--trajectory",
        ])
        .arg(trajectory)
        .args(["--pii-filter", "near-ai"])
        .env_remove("TRACE_NEAR_AI_PRIVACY_API_KEY")
        .output()
        .expect("CLI runs");

    assert!(!output.status.success(), "refusal must exit nonzero");
    let document = parse_single_document(&output);
    assert_eq!(document["schema_version"], "trace_commons.submit_result.v1");
    assert_eq!(document["results"][0]["outcome"], "refused");
    assert_eq!(document["results"][0]["reason"], "pii-filter-unavailable");
    assert!(output.stderr.is_empty(), "JSON errors stay on stdout only");
}

#[test]
fn json_error_before_results_keeps_cli_error_document() {
    let state = tempfile::tempdir().expect("state dir");
    let trajectories = tempfile::tempdir().expect("trajectory dir");

    let output = contributor_command(state.path())
        .args([
            "submit",
            "--dry-run",
            "--all",
            "--source",
            "trajectory",
            "--trajectory",
        ])
        .arg(trajectories.path())
        .output()
        .expect("CLI runs");

    assert!(!output.status.success(), "missing config must exit nonzero");
    let document = parse_single_document(&output);
    assert_eq!(document["schema_version"], "trace_commons.cli_error.v1");
}

#[test]
fn json_manifest_stdout_destinations_are_refused_before_submission() {
    for destination in ["/dev/stdout", "-"] {
        let state = tempfile::tempdir().expect("state dir");
        let working_dir = tempfile::tempdir().expect("working dir");
        let trajectory = working_dir.path().join("bad.jsonl");
        std::fs::write(&trajectory, "not json\n").expect("trajectory fixture writes");
        save_config(state.path());

        let output = contributor_command(state.path())
            .current_dir(working_dir.path())
            .args(["submit", "--all", "--source", "trajectory", "--trajectory"])
            .arg(&trajectory)
            .args(["--manifest", destination])
            .output()
            .expect("CLI runs");

        assert!(
            !output.status.success(),
            "manifest destination {destination:?} must be refused"
        );
        let document = parse_single_document(&output);
        assert_eq!(document["schema_version"], "trace_commons.cli_error.v1");
        assert!(
            document["error"]
                .as_str()
                .is_some_and(|error| error.contains("standard output")),
            "unexpected error for {destination:?}: {document}"
        );
        assert!(
            !working_dir.path().join("-").exists(),
            "the conventional stdout spelling must not become a file"
        );
    }
}
