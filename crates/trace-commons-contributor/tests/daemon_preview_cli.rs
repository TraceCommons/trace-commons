//! Covers the exact regression a fix-round review caught: `daemon preview
//! <entry_id>` is the CLI's entry point onto `handle_local(&shared,
//! "preview", ...)` (wired in `src/bin/trace-commons-contributor.rs`), which
//! now runs every method -- `"preview"` included -- through the real async
//! dispatcher (`handle_request_async`, via a generic `block_on_ipc`), not the
//! synchronous `handle_request`. The synchronous `handle_request` answers
//! `"preview"` with an honest `preview_requires_async` marker instead of
//! `would_send_bytes`, precisely because it *cannot* run the async redaction
//! pipeline -- but no real caller reaches it for this method. Before this
//! test existed, nothing exercised the real CLI-to-daemon path end to end, so
//! a change that accidentally routed `"preview"` to the synchronous arm would
//! silently turn `daemon preview`'s output from "a wrong number" into "no
//! number at all" -- `bytes: null` in the human-readable renderer, no
//! `would_send_bytes` key at all in `--json` output -- with no test noticing.
//!
//! This spawns the real compiled binary (matching the pattern in
//! `preflight.rs`), because the bug lives in `commands::daemon_preview`'s
//! interaction with the binary's argument wiring and JSON rendering, not
//! just in the library call graph.

use std::path::Path;
use std::process::{Command, Output};

use trace_commons_contributor::config::{ConfigStore, ContributorConfig};
use trace_commons_contributor::daemon::queue::{Queue, QueueEntry, QueueState, entry_id_for};
use trace_commons_contributor::daemon::settings::DaemonSettings;
use trace_commons_contributor::source::TraceSource;
use trace_commons_contributor::source::claude_code::ClaudeCodeSource;

/// The planted secret in the fixture session below. It must never reach the
/// CLI's stdout, redacted or not.
const PLANTED_SECRET: &str = "sk-fake-fixture-secret-1234";

/// Write a config dir with a saved config, a queue holding one entry that
/// points at a real fixture session (with a planted secret, so redaction has
/// something to do), and settings pointed at that fixture's project root.
/// Returns the config dir and the entry id to preview.
fn seed_config_dir() -> (tempfile::TempDir, uuid::Uuid) {
    seed_config_dir_with(true)
}

/// `enrolled: false` writes no `contributor.json` at all, which is the state
/// a contributor is in before they have decided whether to enrol -- and the
/// state in which "what would you actually send?" is the most useful
/// question they can ask.
fn seed_config_dir_with(enrolled: bool) -> (tempfile::TempDir, uuid::Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path().join("config");
    let store = ConfigStore::open(config_dir.clone()).unwrap();

    let sessions_root = dir.path().join("sessions/projects");
    let project = sessions_root.join("-Users-testuser-code-myproj");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("11111111-1111-1111-1111-111111111111.jsonl"),
        format!(
            "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\
             \"content\":\"deploy with key {PLANTED_SECRET}\"}},\
             \"cwd\":\"/Users/testuser/code/myproj\",\
             \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
             \"sessionId\":\"11111111-1111-1111-1111-111111111111\",\
             \"uuid\":\"a1\"}}\n"
        ),
    )
    .unwrap();
    let src = ClaudeCodeSource::new(sessions_root.clone());
    let session_ref = TraceSource::discover(&src).unwrap().remove(0);

    let cfg = ContributorConfig {
        schema_version: trace_commons_contributor::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
        issuer_url: "http://issuer.invalid".into(),
        ingest_url: "http://ingest.invalid".into(),
        audience: "trace-commons-upload".into(),
        tenant_id: "tenant-abc".into(),
        instance_id: "instance-1".into(),
        user_subject: "alice".into(),
        device_key_id: "sha256:test".into(),
        consent_scopes: vec!["debugging_evaluation".into()],
        pii_filter: None,
        allowed_hosts: None,
        display_handle: None,
        public_bio: None,
        public_since: None,
    };
    if enrolled {
        store.save_config(&cfg).unwrap();
    }

    let mut settings = DaemonSettings::load(&store).unwrap();
    settings.claude_source = Some(
        trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
            path: sessions_root,
        },
    );
    settings.save(&store).unwrap();

    let entry_id = entry_id_for("cli-preview-test-hash");
    let mut queue = Queue::new();
    queue
        .upsert(
            QueueEntry {
                entry_id,
                session_hash: "cli-preview-test-hash".into(),
                source: "claude-code".into(),
                project_key: "/Users/testuser/code/myproj".into(),
                project_label: "myproj".into(),
                path: session_ref.path.clone(),
                size_bytes: session_ref.size_bytes,
                discovered_at: chrono::Utc::now(),
                state: QueueState::Pending,
                reason_label: None,
                attempts: 0,
                retry_after: None,
                submission_id: None,
                approved_scopes: None,
                approved_inputs: None,
                previewed_envelope_digest: None,
                approved_at: None,
                subagent_count: 0,
                subagents_dropped: 0,
                observed_modified_at: None,
            },
            100,
        )
        .unwrap();
    queue.save(&store).unwrap();

    (dir, entry_id)
}

fn run_preview(config_dir: &Path, entry_id: &str, json: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"));
    command.arg("--config-dir").arg(config_dir);
    if json {
        command.arg("--json");
    }
    command
        .arg("daemon")
        .arg("preview")
        .arg(entry_id)
        .env_remove("TRACE_COMMONS_ALLOWED_HOSTS")
        .env_remove("TRACE_COMMONS_CONTRIBUTOR_DIR")
        .env_remove("TRACE_NEAR_AI_PRIVACY_API_KEY")
        .env_remove("TRACE_NEAR_AI_PRIVACY_BASE_URL")
        .env_remove("TRACE_NEAR_AI_PRIVACY_MODEL")
        .env_remove("TRACE_PRIVACY_FILTER_BACKEND");
    command.output().unwrap()
}

#[test]
fn cli_daemon_preview_prints_a_real_byte_count_not_null() {
    let (dir, entry_id) = seed_config_dir();
    let config_dir = dir.path().join("config");

    let out = run_preview(&config_dir, &entry_id.to_string(), false);
    assert!(
        out.status.success(),
        "daemon preview failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);

    // The regression this test exists to catch: after the sync/async split,
    // the CLI's render callback did `println!("bytes: {}", v["would_send_bytes"])`
    // against a response that no longer carried that key, and
    // serde_json::Value's Display for a missing field prints the bare word
    // "null". A contributor asking for a preview must see a real number.
    let bytes_line = stdout
        .lines()
        .find(|l| l.trim_start().starts_with("bytes:"))
        .unwrap_or_else(|| panic!("no 'bytes:' line in output: {stdout}"));
    assert!(
        !bytes_line.contains("null"),
        "would_send_bytes came back null: {bytes_line}"
    );
    let reported: u64 = bytes_line
        .split(':')
        .nth(1)
        .unwrap()
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("'bytes:' line was not a number ({e}): {bytes_line}"));
    assert!(reported > 0, "reported 0 bytes: {bytes_line}");

    assert!(
        !stdout.contains(PLANTED_SECRET),
        "the planted secret leaked into CLI stdout: {stdout}"
    );
}

#[test]
fn preview_works_before_enrollment() {
    // Found by an application build: `preview` refused with `not-logged-in`
    // unless a `contributor.json` existed, so the app's fixture launcher had
    // to fabricate an enrollment purely to preview a local file. Preview
    // does no network I/O and needs neither the daemon's lock nor its
    // running loop -- the requirement was incidental, and it inverted the
    // decision it exists to support: seeing what would be sent is precisely
    // what someone wants *before* enrolling.
    let (dir, entry_id) = seed_config_dir_with(false);
    let config_dir = dir.path().join("config");

    let out = run_preview(&config_dir, &entry_id.to_string(), true);
    assert!(
        out.status.success(),
        "preview must not require an enrollment: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout was not valid JSON ({e}): {stdout}"));

    assert_eq!(
        value["enrolled"],
        serde_json::json!(false),
        "an unenrolled preview must say so, so an app can label it as a \
         placeholder-identity build: {value}"
    );
    assert!(
        value["would_send_bytes"].as_u64().unwrap_or(0) > 0,
        "an unenrolled preview still reports what would be sent: {value}"
    );
    assert!(!stdout.contains(PLANTED_SECRET));
}

#[test]
fn cli_daemon_preview_json_carries_would_send_bytes() {
    let (dir, entry_id) = seed_config_dir();
    let config_dir = dir.path().join("config");

    let out = run_preview(&config_dir, &entry_id.to_string(), true);
    assert!(
        out.status.success(),
        "daemon preview --json failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout was not valid JSON ({e}): {stdout}"));

    let would_send = value["would_send_bytes"]
        .as_u64()
        .unwrap_or_else(|| panic!("would_send_bytes missing or not a number: {value}"));
    assert!(would_send > 0);
    assert!(
        !value["redactions"].is_null(),
        "redactions missing from CLI --json preview output: {value}"
    );
    assert!(!stdout.contains(PLANTED_SECRET));
}
