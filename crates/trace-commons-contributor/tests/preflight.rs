use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const UNENROLLED_NOTICE: &str = "unenrolled preview: deterministic-only redaction";

fn write_trajectory(dir: &Path, content: &str) -> std::path::PathBuf {
    write_trajectory_with_source(dir, content, "preflight-test")
}

fn write_trajectory_with_source(dir: &Path, content: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join("trajectory.json");
    let body = serde_json::json!([
        {"role": "meta", "source": source},
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
        .env_remove("TRACE_NEAR_AI_PRIVACY_API_KEY")
        .env_remove("TRACE_NEAR_AI_PRIVACY_BASE_URL")
        .env_remove("TRACE_NEAR_AI_PRIVACY_MODEL")
        .env_remove("TRACE_PRIVACY_FILTER_BACKEND");
    command.output().unwrap()
}

fn write_enrolled_config(config_dir: &Path) {
    let tenant_id =
        trace_commons_protocol::onboarding::derive_user_tenant_id("instance-test", "user-test");
    let config = serde_json::json!({
        "schema_version": "trace_commons.contributor_config.v1",
        "issuer_url": "https://issuer.example",
        "ingest_url": "https://ingest.example",
        "audience": "trace-commons-upload",
        "tenant_id": tenant_id,
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

fn json_submit_command(config_dir: &Path, trajectory: &Path, dry_run: bool) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"));
    command
        .arg("--config-dir")
        .arg(config_dir)
        .arg("--json")
        .arg("submit");
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
        .env_remove("TRACE_NEAR_AI_PRIVACY_API_KEY")
        .env_remove("TRACE_NEAR_AI_PRIVACY_BASE_URL")
        .env_remove("TRACE_NEAR_AI_PRIVACY_MODEL")
        .env_remove("TRACE_PRIVACY_FILTER_BACKEND");
    command
}

fn spawn_http_counter() -> (
    String,
    Arc<AtomicUsize>,
    Arc<AtomicBool>,
    std::thread::JoinHandle<()>,
) {
    use std::io::{Read as _, Write as _};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let thread_requests = Arc::clone(&requests);
    let thread_stop = Arc::clone(&stop);
    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !thread_stop.load(Ordering::SeqCst) && Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    thread_requests.fetch_add(1, Ordering::SeqCst);
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut request = [0_u8; 16 * 1024];
                    let _ = stream.read(&mut request);
                    let body = r#"{"data":[{"spans":[]}]}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("HTTP counter failed: {error}"),
            }
        }
    });
    (base_url, requests, stop, handle)
}

fn run_preview_against_http_counter(
    trajectory: &Path,
    use_flag: bool,
    use_backend_env: bool,
) -> usize {
    let config_dir = tempfile::tempdir().unwrap();
    let (base_url, requests, stop, handle) = spawn_http_counter();
    let mut command = json_submit_command(config_dir.path(), trajectory, true);
    if use_flag {
        command.arg("--pii-filter").arg("near-ai");
    }
    if use_backend_env {
        command.env("TRACE_PRIVACY_FILTER_BACKEND", "near-ai");
    }
    command
        .env("TRACE_NEAR_AI_PRIVACY_API_KEY", "test-key")
        .env("TRACE_NEAR_AI_PRIVACY_BASE_URL", base_url);
    let output = command.output().unwrap();
    stop.store(true, Ordering::SeqCst);
    handle.join().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "stdout={stdout}\nstderr={stderr}");
    requests.load(Ordering::SeqCst)
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
    let final_line = stdout.lines().last().unwrap_or_default();
    assert!(final_line.contains("unenrolled-preview"), "stdout={stdout}");
    assert!(final_line.contains("previewed"), "stdout={stdout}");
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
    assert_eq!(
        document["results"][0]["unenrolled_preview"], true,
        "stdout={stdout}"
    );
    assert_eq!(document["results"][0]["outcome"], "previewed");
    assert!(document["results"][0]["preview_id"].is_string());
    assert!(document["results"][0]["submission_id"].is_null());
}

#[test]
fn unenrolled_preview_ignores_flagged_and_inherited_network_filters() {
    let fixture_dir = tempfile::tempdir().unwrap();
    let trajectory = write_trajectory(fixture_dir.path(), "inspect this session");

    assert_eq!(
        run_preview_against_http_counter(&trajectory, true, false),
        0
    );
    assert_eq!(
        run_preview_against_http_counter(&trajectory, false, true),
        0
    );
}

#[test]
fn preview_id_is_disjoint_from_real_submission_id() {
    let fixture_dir = tempfile::tempdir().unwrap();
    let trajectory = write_trajectory(fixture_dir.path(), "inspect this session");

    let preview_config = tempfile::tempdir().unwrap();
    let preview = run_submit(preview_config.path(), &trajectory, true, true);
    let preview_doc: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();
    let preview_id = preview_doc["results"][0]["preview_id"]
        .as_str()
        .expect("preview id");

    let enrolled_config = tempfile::tempdir().unwrap();
    write_enrolled_config(enrolled_config.path());
    let enrolled = run_submit(enrolled_config.path(), &trajectory, true, true);
    let enrolled_doc: serde_json::Value = serde_json::from_slice(&enrolled.stdout).unwrap();
    let submission_id = enrolled_doc["results"][0]["submission_id"]
        .as_str()
        .expect("submission id");

    assert_ne!(preview_id, submission_id);
    assert_eq!(
        uuid::Uuid::parse_str(preview_id).unwrap().get_version_num(),
        8
    );
    assert_eq!(
        uuid::Uuid::parse_str(submission_id)
            .unwrap()
            .get_version_num(),
        5
    );
}

#[test]
fn unenrolled_preview_leaves_config_directory_empty() {
    let fixture_dir = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    let trajectory = write_trajectory(fixture_dir.path(), "inspect this session");

    let output = run_submit(config_dir.path(), &trajectory, false, true);
    assert!(output.status.success());
    let entries: Vec<_> = std::fs::read_dir(config_dir.path())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(entries.is_empty(), "unexpected state: {entries:?}");
}

#[test]
fn canonical_size_boundary_agrees_before_and_after_enrollment() {
    let fixture_dir = tempfile::tempdir().unwrap();
    let trajectory =
        write_trajectory_with_source(fixture_dir.path(), &"x".repeat(1_497_756), "boundary-test");

    let preview_config = tempfile::tempdir().unwrap();
    let preview = run_submit(preview_config.path(), &trajectory, true, true);
    let preview_doc: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();

    let enrolled_config = tempfile::tempdir().unwrap();
    write_enrolled_config(enrolled_config.path());
    let enrolled = run_submit(enrolled_config.path(), &trajectory, true, true);
    let enrolled_doc: serde_json::Value = serde_json::from_slice(&enrolled.stdout).unwrap();

    assert!(preview.status.success());
    assert!(enrolled.status.success());
    assert_eq!(preview_doc["results"][0]["outcome"], "refused");
    assert_eq!(enrolled_doc["results"][0]["outcome"], "refused");
    assert_eq!(preview_doc["results"][0]["reason"], "session-too-large");
    assert_eq!(preview_doc["results"][0]["size_bytes"], 1_500_020);
    assert_eq!(
        preview_doc["results"][0]["size_bytes"],
        enrolled_doc["results"][0]["size_bytes"]
    );
}

#[test]
fn near_ai_notice_is_inside_single_json_document() {
    let fixture_dir = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    write_enrolled_config(config_dir.path());
    let trajectory = write_trajectory(fixture_dir.path(), "inspect this session");
    let output = json_submit_command(config_dir.path(), &trajectory, true)
        .arg("--pii-filter")
        .arg("near-ai")
        .output()
        .unwrap();
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert!(!output.status.success());
    assert_eq!(document["results"][0]["reason"], "pii-filter-unavailable");
    assert!(
        document["notices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|notice| notice.as_str().unwrap_or_default().contains("NEAR AI"))
    );
}

#[test]
fn pii_filter_refusal_fails_enrolled_dry_run_and_real_submit() {
    let fixture_dir = tempfile::tempdir().unwrap();
    let trajectory = write_trajectory(fixture_dir.path(), "inspect this session");

    for dry_run in [true, false] {
        let config_dir = tempfile::tempdir().unwrap();
        write_enrolled_config(config_dir.path());
        let output = json_submit_command(config_dir.path(), &trajectory, dry_run)
            .arg("--pii-filter")
            .arg("near-ai")
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "dry_run={dry_run} stdout={}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
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
