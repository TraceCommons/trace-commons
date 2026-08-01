use std::path::Path;
use std::process::{Command, Output};

const UNENROLLED_NOTICE: &str =
    "unenrolled preview: identity fields are placeholders; nothing was submitted";

fn write_trajectory(dir: &Path, content: &str) -> std::path::PathBuf {
    let path = dir.join("trajectory.json");
    let body = serde_json::json!([
        {"role": "meta", "source": "preflight-test"},
        {
            "role": "user",
            "content": content,
            "timestamp": "2026-07-31T12:00:00Z"
        }
    ]);
    std::fs::write(&path, serde_json::to_vec(&body).unwrap()).unwrap();
    path
}

fn run_submit(config_dir: &Path, trajectory: &Path, json: bool, dry_run: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"));
    command.arg("--config-dir").arg(config_dir);
    if json {
        command.arg("--json");
    }
    command.arg("submit");
    if dry_run {
        command.arg("--dry-run");
    }
    command
        .arg("--all")
        .arg("--source")
        .arg("trajectory")
        .arg("--trajectory")
        .arg(trajectory)
        .env_remove("TRACE_COMMONS_ALLOWED_HOSTS")
        .env_remove("TRACE_COMMONS_CONTRIBUTOR_DIR")
        .env_remove("TRACE_NEAR_AI_PRIVACY_API_KEY");
    command.output().unwrap()
}

fn write_enrolled_config(config_dir: &Path) {
    let config = serde_json::json!({
        "schema_version": "trace_commons.contributor_config.v1",
        "issuer_url": "https://issuer.example",
        "ingest_url": "https://ingest.example",
        "audience": "trace-commons-upload",
        "tenant_id": "tenant-test",
        "instance_id": "instance-test",
        "user_subject": "user-test",
        "device_key_id": "sha256:test",
        "consent_scopes": ["debugging_evaluation"],
        "pii_filter": null,
        "allowed_hosts": null
    });
    std::fs::write(
        config_dir.join("contributor.json"),
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();
}

#[test]
fn unenrolled_dry_run_succeeds_and_marks_human_output() {
    let fixture_dir = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    let trajectory = write_trajectory(fixture_dir.path(), "inspect this session");

    let output = run_submit(config_dir.path(), &trajectory, false, true);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(output.status.success(), "stdout={stdout}\nstderr={stderr}");
    assert!(stdout.contains(UNENROLLED_NOTICE), "stdout={stdout}");
    assert!(stdout.contains("dry-run"), "stdout={stdout}");
}

#[test]
fn unenrolled_dry_run_marks_json_output() {
    let fixture_dir = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    let trajectory = write_trajectory(fixture_dir.path(), "inspect this session");

    let output = run_submit(config_dir.path(), &trajectory, true, true);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(output.status.success(), "stdout={stdout}\nstderr={stderr}");
    let document: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(document["unenrolled_preview"], true, "stdout={stdout}");
}

#[test]
fn unenrolled_real_submit_still_requires_login() {
    let fixture_dir = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    let trajectory = write_trajectory(fixture_dir.path(), "inspect this session");

    let output = run_submit(config_dir.path(), &trajectory, false, false);
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(!output.status.success());
    assert!(
        stderr.contains("not logged in; run `login` first"),
        "stderr={stderr}"
    );
}

#[test]
fn refusal_reports_session_and_size_and_only_fails_real_submit() {
    let fixture_dir = tempfile::tempdir().unwrap();
    let trajectory = write_trajectory(fixture_dir.path(), &"x".repeat(1_600_000));

    let dry_config_dir = tempfile::tempdir().unwrap();
    let dry = run_submit(dry_config_dir.path(), &trajectory, false, true);
    let dry_stdout = String::from_utf8(dry.stdout).unwrap();
    let dry_stderr = String::from_utf8(dry.stderr).unwrap();
    assert!(
        dry.status.success(),
        "stdout={dry_stdout}\nstderr={dry_stderr}"
    );
    assert!(
        dry_stdout.contains("refused (session-too-large)"),
        "stdout={dry_stdout}"
    );
    assert!(
        dry_stdout.contains("session=sha256:"),
        "stdout={dry_stdout}"
    );
    assert!(dry_stdout.contains("size="), "stdout={dry_stdout}");
    assert!(dry_stdout.contains("limit=1500000"), "stdout={dry_stdout}");

    let real_config_dir = tempfile::tempdir().unwrap();
    write_enrolled_config(real_config_dir.path());
    let real = run_submit(real_config_dir.path(), &trajectory, false, false);
    let real_stdout = String::from_utf8(real.stdout).unwrap();
    let real_stderr = String::from_utf8(real.stderr).unwrap();
    assert!(!real.status.success());
    assert!(
        real_stdout.contains("refused (session-too-large)"),
        "stdout={real_stdout}\nstderr={real_stderr}"
    );
}
