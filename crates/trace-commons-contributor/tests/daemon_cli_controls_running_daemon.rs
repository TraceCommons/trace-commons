//! The CLI control surface must control the daemon that is actually
//! running.
//!
//! Every `daemon <verb>` command used to build a private `DaemonShared`
//! from the on-disk files and write them back. A running daemon loads those
//! files exactly once (`DaemonShared::load`) and never re-reads them, and
//! rewrites them from its own copy on its next pass -- so the CLI's edit
//! reached the disk and nothing else, and was then silently overwritten.
//!
//! Every test here asserts against the *running* daemon's in-memory state,
//! never against the files. That is the only assertion that can tell the
//! two designs apart: the old code passed a file-based assertion and failed
//! every one of these.

use std::sync::Arc;

use chrono::Utc;
use trace_commons_contributor::commands;
use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::daemon::health::{HealthState, LABEL_INGEST_UNREACHABLE};
use trace_commons_contributor::daemon::policy::ProjectMode;
use trace_commons_contributor::daemon::queue::{QueueEntry, QueueState, entry_id_for};
use trace_commons_contributor::daemon::settings::{DaemonSettings, SourceDeclaration};
use trace_commons_contributor::daemon::{EmbeddedDaemon, ipc, start_embedded};
use trace_commons_contributor::identity::DeviceIdentity;
use trace_commons_contributor::source::TraceSource;
use trace_commons_contributor::source::claude_code::ClaudeCodeSource;

/// A daemon that is genuinely running: locked, socket bound, server task
/// serving. Deliberately does *not* run the supervise loop -- these tests
/// are about the control surface reaching the running process, not about
/// what its poll pass does.
struct Running {
    _dir: tempfile::TempDir,
    store: ConfigStore,
    shared: Arc<ipc::DaemonShared>,
    embedded: Option<EmbeddedDaemon>,
    /// The one real session every seeded entry points at. `approve` builds
    /// and pins an envelope for anything unpreviewed, so an entry over a
    /// path that does not exist is skipped rather than approved -- these
    /// tests are about the control surface reaching the running daemon, so
    /// the fixture gives it something it can actually build.
    session_path: std::path::PathBuf,
}

impl Running {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();

        let sessions_root = dir.path().join("sessions/projects");
        let project = sessions_root.join("-Users-testuser-code-proj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("55555555-5555-5555-5555-555555555555.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\
             \"content\":\"list the files\"},\
             \"cwd\":\"/Users/testuser/code/proj\",\
             \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
             \"sessionId\":\"55555555-5555-5555-5555-555555555555\",\
             \"uuid\":\"a1\"}\n",
        )
        .unwrap();
        let src = ClaudeCodeSource::new(sessions_root.clone());
        let session_path = TraceSource::discover(&src).unwrap().remove(0).path;

        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = trace_commons_contributor::config::ContributorConfig {
            inference_receipt_endpoint: None,
            schema_version: trace_commons_contributor::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION
                .into(),
            issuer_url: "http://issuer.invalid".into(),
            ingest_url: "http://ingest.invalid".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device.device_key_id.clone(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
            witness: None,
        };
        store.save_config(&cfg).unwrap();
        let mut settings = DaemonSettings::load(&store).unwrap();
        settings.claude_source = Some(SourceDeclaration::Watch {
            path: sessions_root.clone(),
        });
        settings.save(&store).unwrap();

        let embedded = start_embedded(ConfigStore::open(dir.path().to_path_buf()).unwrap())
            .await
            .unwrap();
        let shared = Arc::clone(&embedded.shared);
        Self {
            _dir: dir,
            store,
            shared,
            embedded: Some(embedded),
            session_path,
        }
    }

    /// A separate `ConfigStore` over the same directory: exactly what the
    /// CLI binary constructs, with no access to the daemon's memory.
    fn cli_store(&self) -> ConfigStore {
        ConfigStore::open(self.store.dir().to_path_buf()).unwrap()
    }

    fn seed_pending(&self, project_key: &str, hash: &str) -> uuid::Uuid {
        let entry_id = entry_id_for(hash);
        let mut queue = self.shared.queue.lock().unwrap();
        queue
            .upsert(
                QueueEntry {
                    entry_id,
                    session_hash: hash.to_string(),
                    source: "claude-code".to_string(),
                    declared_source: None,
                    project_key: project_key.to_string(),
                    project_path: None,
                    session_cwd: None,
                    project_label: "proj".to_string(),
                    path: self.session_path.clone(),
                    size_bytes: 1,
                    discovered_at: Utc::now(),
                    state: QueueState::Pending,
                    reason_label: None,
                    attempts: 0,
                    retry_after: None,
                    submission_id: None,
                    approved_scopes: None,
                    approved_verdict: None,
                    approved_correction: None,
                    approved_inputs: None,
                    previewed_envelope_digest: None,
                    approved_at: None,
                    subagent_count: 0,
                    subagents_dropped: 0,
                    observed_modified_at: None,
                },
                500,
            )
            .unwrap();
        entry_id
    }

    fn state_of(&self, entry_id: uuid::Uuid) -> QueueState {
        self.shared
            .queue
            .lock()
            .unwrap()
            .get(entry_id)
            .unwrap()
            .state
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(e) = self.embedded.take() {
            e.close();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn daemon_approve_reaches_the_running_daemon_not_just_the_file() {
    let h = Running::new().await;
    let entry_id = h.seed_pending("/tmp/p", "sha256:aa");
    assert_eq!(h.state_of(entry_id), QueueState::Pending);

    let store = h.cli_store();
    let entry_id_str = entry_id.to_string();
    tokio::task::spawn_blocking(move || {
        commands::daemon_approve(&store, Some(&entry_id_str), false, None, true).unwrap()
    })
    .await
    .unwrap();

    assert_eq!(
        h.state_of(entry_id),
        QueueState::Approved,
        "the running daemon's own queue must reflect the approval; writing \
         the file behind its back does nothing, because it never re-reads it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn daemon_dismiss_reaches_the_running_daemon() {
    let h = Running::new().await;
    let entry_id = h.seed_pending("/tmp/p", "sha256:bb");

    let store = h.cli_store();
    let entry_id_str = entry_id.to_string();
    tokio::task::spawn_blocking(move || {
        commands::daemon_dismiss(&store, &entry_id_str, true).unwrap()
    })
    .await
    .unwrap();

    assert_eq!(h.state_of(entry_id), QueueState::Refused);
}

#[tokio::test(flavor = "multi_thread")]
async fn daemon_pause_actually_pauses_the_running_daemon() {
    let h = Running::new().await;
    assert!(!h.shared.is_paused(Utc::now()));

    let store = h.cli_store();
    tokio::task::spawn_blocking(move || commands::daemon_pause(&store, true, true).unwrap())
        .await
        .unwrap();

    assert!(
        h.shared.is_paused(Utc::now()),
        "pause must stop the daemon that is running; the old path wrote \
         daemon-state.json, which watcher::tick overwrites every poll"
    );

    let store = h.cli_store();
    tokio::task::spawn_blocking(move || commands::daemon_pause(&store, false, true).unwrap())
        .await
        .unwrap();
    assert!(!h.shared.is_paused(Utc::now()));
}

#[tokio::test(flavor = "multi_thread")]
async fn disarming_auto_upload_from_the_cli_takes_effect_immediately() {
    // The consent property this whole control surface exists to protect:
    // you must be able to turn auto-upload OFF on a headless machine
    // without restarting the daemon.
    let h = Running::new().await;
    let project = tempfile::tempdir().unwrap();
    let key = std::fs::canonicalize(project.path())
        .unwrap()
        .to_string_lossy()
        .to_string();
    h.shared
        .policy
        .lock()
        .unwrap()
        .set_mode(&key, ProjectMode::AutoUpload, Utc::now())
        .unwrap();
    assert_eq!(
        h.shared.policy.lock().unwrap().resolve(&key),
        ProjectMode::AutoUpload
    );

    let store = h.cli_store();
    let path = project.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        commands::daemon_set_project(&store, &path, "ignore", true).unwrap()
    })
    .await
    .unwrap();

    assert_eq!(
        h.shared.policy.lock().unwrap().resolve(&key),
        ProjectMode::Ignore,
        "you must be able to disarm auto-upload from the CLI while the \
         daemon runs -- this is the control we ship as the answer for \
         headless machines"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn daemon_status_reports_the_running_daemons_real_health() {
    // `HealthState` is in-memory only and never persisted, so a CLI that
    // built its own `DaemonShared` always started from
    // `HealthState::default()` and always printed "health: ok" -- even
    // while the running daemon was refusing every upload.
    let h = Running::new().await;
    {
        let mut health = h.shared.health.lock().unwrap();
        *health = HealthState::default();
        health.fail(LABEL_INGEST_UNREACHABLE, Utc::now());
    }

    let store = h.cli_store();
    let resp = tokio::task::spawn_blocking(move || {
        trace_commons_contributor::daemon::client::try_call(
            &store,
            "status",
            &serde_json::json!({}),
        )
        .unwrap()
        .expect("a daemon is running, so the CLI must reach it")
    })
    .await
    .unwrap();

    let v = resp.result.unwrap();
    assert_eq!(
        v["health"]["last_error_label"], LABEL_INGEST_UNREACHABLE,
        "status must report the running daemon's real health, not a fresh default"
    );

    // And the thing the old code did instead, spelled out: a private
    // `DaemonShared` loaded from the same directory reports healthy, because
    // health lives only in the running process's memory. Any `daemon status`
    // answered that way is a lie whenever it matters most.
    let private = ipc::DaemonShared::load(h.cli_store()).unwrap();
    assert!(
        private.health.lock().unwrap().ok(),
        "a freshly-loaded DaemonShared cannot know the daemon is unhealthy -- \
         which is exactly why status must go over the socket"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn with_no_daemon_running_commands_still_work_against_the_files() {
    // The fallback must stay: a one-shot command against a stopped daemon
    // writes the files, which primes the next start. That is correct there
    // and only there.
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
    let project = tempfile::tempdir().unwrap();
    let key = std::fs::canonicalize(project.path())
        .unwrap()
        .to_string_lossy()
        .to_string();

    commands::daemon_set_project(&store, project.path(), "auto", true).unwrap();

    let policy = trace_commons_contributor::daemon::policy::ProjectPolicy::load(&store).unwrap();
    assert_eq!(policy.resolve(&key), ProjectMode::AutoUpload);
}

/// `approve --project <id>` must reach the running daemon *and* stop at the
/// project boundary. The second entry is the whole point: a selector that
/// approves everything passes any assertion that only checks the intended
/// entry became `Approved`, and approving a queue when one project was meant
/// is the exact failure the CLI's refusal of ambiguous selectors exists to
/// prevent. Both halves are asserted here.
#[tokio::test(flavor = "multi_thread")]
async fn daemon_approve_by_project_approves_only_that_project() {
    let h = Running::new().await;
    let mine = h.seed_pending("/tmp/mine", "sha256:c1");
    let other = h.seed_pending("/tmp/other", "sha256:c2");
    assert_eq!(h.state_of(mine), QueueState::Pending);
    assert_eq!(h.state_of(other), QueueState::Pending);

    // The opaque handle the CLI is given, derived the same way the daemon
    // derives it for `list_pending` -- never the raw project key.
    let project_id = trace_commons_contributor::daemon::policy::project_id_for("/tmp/mine");
    let store = h.cli_store();
    tokio::task::spawn_blocking(move || {
        commands::daemon_approve(&store, None, false, Some(&project_id), true).unwrap()
    })
    .await
    .unwrap();

    assert_eq!(
        h.state_of(mine),
        QueueState::Approved,
        "the selected project's pending entry must be approved in the \
         running daemon's own queue"
    );
    assert_eq!(
        h.state_of(other),
        QueueState::Pending,
        "an entry in a different project must be untouched; a project \
         selector that approves the whole queue is the failure mode"
    );
}
