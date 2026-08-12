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

fn write_trajectory_with_model(dir: &Path, content: &str, model: &str) -> std::path::PathBuf {
    let path = dir.join("trajectory.json");
    let body = serde_json::json!([
        {"role": "meta", "source": "preflight-test", "model": model},
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

fn enrolled_config() -> trace_commons_contributor::config::ContributorConfig {
    let tenant_id =
        trace_commons_protocol::onboarding::derive_user_tenant_id("instance-test", "user-test");
    trace_commons_contributor::config::ContributorConfig {
        schema_version: "trace_commons.contributor_config.v1".to_string(),
        issuer_url: "https://issuer.example".to_string(),
        ingest_url: "https://ingest.example".to_string(),
        audience: "trace-commons-upload".to_string(),
        tenant_id,
        instance_id: "instance-test".to_string(),
        user_subject: "user-test".to_string(),
        device_key_id: "sha256:test".to_string(),
        consent_scopes: vec!["debugging_evaluation".to_string()],
        pii_filter: None,
        allowed_hosts: None,
    }
}

fn write_enrolled_config(config_dir: &Path) {
    let config = enrolled_config();
    std::fs::write(
        config_dir.join("contributor.json"),
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();
}

fn unenrolled_preview_config() -> trace_commons_contributor::config::ContributorConfig {
    trace_commons_contributor::config::ContributorConfig {
        schema_version: "trace_commons.contributor_config.v1".to_string(),
        issuer_url: "https://unenrolled-preview.invalid".to_string(),
        ingest_url: "https://unenrolled-preview.invalid".to_string(),
        audience: "unenrolled-preview-placeholder".to_string(),
        tenant_id: "tenant-0000000000000000000000000000000000000000000000000000000000000000"
            .to_string(),
        instance_id: "unenrolled-preview-placeholder".to_string(),
        user_subject: "unenrolled-preview-placeholder".to_string(),
        device_key_id: "unenrolled-preview-placeholder".to_string(),
        consent_scopes: vec!["debugging_evaluation".to_string()],
        pii_filter: None,
        allowed_hosts: None,
    }
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
                    let request_len = stream.read(&mut request).unwrap_or(0);
                    let request = &request[..request_len];
                    let body_start = request
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .map(|index| index + 4)
                        .unwrap_or(request.len());
                    let input = serde_json::from_slice::<serde_json::Value>(&request[body_start..])
                        .ok()
                        .and_then(|body| body["input"].as_str().map(str::to_string))
                        .unwrap_or_default();
                    let targets: &[(&str, &str)] = &[
                        ("trace-canary.person@example.invalid", "private_email"),
                        ("tc_canary_secret_0123456789abcdef", "secret"),
                        ("/tmp/trace_canary_private/path.txt", "private_url"),
                    ];
                    let spans: Vec<_> = targets
                        .iter()
                        .filter_map(|(needle, category)| {
                            input.find(needle).map(|start| {
                                serde_json::json!({
                                    "category": category,
                                    "start": start,
                                    "end": start + needle.len(),
                                    "score": 0.99
                                })
                            })
                        })
                        .collect();
                    let body = serde_json::to_string(&serde_json::json!({
                        "data": [{"spans": spans}]
                    }))
                    .unwrap();
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

#[tokio::test]
async fn canonical_size_boundary_agrees_before_and_after_enrollment() {
    use chrono::{DateTime, Utc};
    use trace_commons_contributor::envelope::{
        build_deterministic_preview_redactor, build_preview_raw_contribution,
        build_raw_contribution, build_redactor_with, envelope_size, redact_to_envelope,
    };
    use trace_commons_contributor::source::TraceSource as _;

    let fixture_dir = tempfile::tempdir().unwrap();
    // Sized with headroom rather than tuned to the byte. The claim under test
    // is that the preview and enrolled paths agree on canonical size; sitting
    // a handful of bytes above MAX_ENVELOPE_BYTES made the precondition break
    // on any change to serialized length, including a consent boolean
    // flipping between `true` and `false`.
    let trajectory =
        write_trajectory_with_source(fixture_dir.path(), &"x".repeat(1_499_000), "boundary-test");
    let source = trace_commons_contributor::source::trajectory::TrajectorySource::new(trajectory);
    let session_ref = source.discover().unwrap().remove(0);
    let transcript = source.load(&session_ref).unwrap();
    let now = DateTime::parse_from_rfc3339("2026-07-31T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let preview_cfg = unenrolled_preview_config();
    let enrolled_cfg = enrolled_config();
    let preview_redactor = build_deterministic_preview_redactor(transcript.cwd.as_deref());
    let enrolled_redactor =
        build_redactor_with(&enrolled_cfg, transcript.cwd.as_deref(), None).unwrap();
    let preview = redact_to_envelope(
        &preview_redactor,
        build_preview_raw_contribution(&transcript, &preview_cfg, now),
    )
    .await
    .unwrap();
    let enrolled = redact_to_envelope(
        &enrolled_redactor,
        build_raw_contribution(&transcript, &enrolled_cfg, now),
    )
    .await
    .unwrap();
    let preview_size = envelope_size(&preview).unwrap();
    let enrolled_size = envelope_size(&enrolled).unwrap();

    assert!(
        preview_size > trace_commons_contributor::envelope::MAX_ENVELOPE_BYTES,
        "fixture must remain above the refusal boundary: {preview_size}"
    );
    assert_eq!(preview_size, enrolled_size);
}

#[test]
fn failed_near_ai_dry_run_keeps_notice_and_device_state_unburned() {
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
    assert!(!config_dir.path().join("near-ai-notice-shown").exists());
    assert!(!config_dir.path().join("device.pk8").exists());
}

#[test]
fn successful_near_ai_dry_run_records_notice_without_generating_device_key() {
    let fixture_dir = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    write_enrolled_config(config_dir.path());
    let trajectory = write_trajectory(fixture_dir.path(), "inspect this session");
    let (base_url, requests, stop, handle) = spawn_http_counter();
    let output = json_submit_command(config_dir.path(), &trajectory, true)
        .arg("--pii-filter")
        .arg("near-ai")
        .env("TRACE_NEAR_AI_PRIVACY_API_KEY", "test-key")
        .env("TRACE_NEAR_AI_PRIVACY_BASE_URL", base_url)
        .output()
        .unwrap();
    stop.store(true, Ordering::SeqCst);
    handle.join().unwrap();

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(requests.load(Ordering::SeqCst) > 0);
    assert!(config_dir.path().join("near-ai-notice-shown").exists());
    assert!(!config_dir.path().join("device.pk8").exists());
}

#[test]
fn residual_secret_refusal_fails_enrolled_and_unenrolled_dry_runs() {
    let fixture_dir = tempfile::tempdir().unwrap();
    let trajectory = write_trajectory_with_model(
        fixture_dir.path(),
        "inspect this session",
        "sk-ant-EXPOSEDsecret0123456789abcdefghij",
    );

    for enrolled in [false, true] {
        let config_dir = tempfile::tempdir().unwrap();
        if enrolled {
            write_enrolled_config(config_dir.path());
        }
        let output = run_submit(config_dir.path(), &trajectory, true, true);
        let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            !output.status.success(),
            "enrolled={enrolled} stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(document["results"][0]["outcome"], "refused");
        assert_eq!(document["results"][0]["reason"], "secret-leak-detected");
    }
}

#[test]
fn unenrolled_preview_ignores_stale_receipts() {
    let fixture_dir = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    let trajectory = write_trajectory(fixture_dir.path(), "inspect this session");
    let session_hash =
        trace_commons_contributor::source::session_hash(&std::fs::read(&trajectory).unwrap());
    let receipt = serde_json::json!({
        "submission_id": uuid::Uuid::new_v4(),
        "session_hash": session_hash,
        "source": "preflight-test",
        "submitted_at": "2026-07-31T12:00:00Z",
        "status": "accepted"
    });
    std::fs::write(
        config_dir.path().join("receipts.jsonl"),
        format!("{}\n", serde_json::to_string(&receipt).unwrap()),
    )
    .unwrap();

    let output = run_submit(config_dir.path(), &trajectory, true, true);
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success());
    assert_eq!(document["results"][0]["outcome"], "previewed");
    assert_eq!(document["results"][0]["unenrolled_preview"], true);
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
