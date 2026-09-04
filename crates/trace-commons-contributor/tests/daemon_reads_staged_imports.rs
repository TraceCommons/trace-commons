//! A conversation staged by `import-antigravity` reaches the daemon's queue.
//!
//! This failed before the daemon had a staging scope, and failed silently:
//! no error, no empty state, the conversation simply did not exist as far as
//! any desktop app was concerned. The CLI could see it the whole time,
//! because the CLI builds its own source roots.

#![cfg(unix)]
// Drives the daemon the way `daemon_end_to_end_upload.rs` does. Ungated, this
// target would fail to COMPILE on Windows rather than skipping.

use std::sync::Arc;

use trace_commons_contributor::config::{
    CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig,
};
use trace_commons_contributor::daemon::ipc::DaemonShared;
use trace_commons_contributor::daemon::policy::ProjectMode;
use trace_commons_contributor::daemon::queue::{QueueEntry, QueueState};
use trace_commons_contributor::identity::DeviceIdentity;

/// The project the staged conversation says it came from.
const PROJECT: &str = "/Users/testuser/code/demo";

/// One staged conversation, in the shape `import-antigravity` writes: a JSON
/// array whose first record is the meta, carrying `source: "antigravity"`.
///
/// Taken from what `antigravity::convert` produces rather than invented, so
/// this breaks if the converter's output drifts away from what the trajectory
/// reader accepts.
fn staged_conversation() -> String {
    serde_json::json!([
        {"role": "meta", "source": "antigravity", "cwd": PROJECT},
        {"role": "user", "content": "Tell me about this repo",
         "timestamp": "2026-08-30T10:00:00Z"},
        {"role": "assistant", "content": "It is a contributor client.",
         "timestamp": "2026-08-30T10:00:05Z"}
    ])
    .to_string()
}

struct Harness {
    _dir: tempfile::TempDir,
    shared: Arc<DaemonShared>,
}

impl Harness {
    /// An enrolled daemon whose staging directory holds one conversation.
    ///
    /// Nothing declares the staging directory: that is the point. The
    /// contributor ran the import, and the files are in their own state
    /// directory.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().join("state")).unwrap();
        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        store
            .save_config(&ContributorConfig {
                inference_receipt_endpoint: None,
                schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
                issuer_url: "http://issuer.invalid".into(),
                ingest_url: "http://ingest.invalid".into(),
                audience: "trace-commons-upload".into(),
                tenant_id: "tenant-abc".into(),
                instance_id: "instance-1".into(),
                user_subject: "alice".into(),
                device_key_id: device.device_key_id,
                consent_scopes: vec!["debugging_evaluation".into()],
                pii_filter: None,
                allowed_hosts: Some("127.0.0.1".into()),
                display_handle: None,
                public_bio: None,
                public_since: None,
                witness: None,
            })
            .unwrap();

        let staging = store
            .dir()
            .join(trace_commons_contributor::source::TRAJECTORY_STAGING_SUBDIR);
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("conversation.json"), staged_conversation()).unwrap();

        let shared = Arc::new(DaemonShared::load(store).unwrap());

        // Declare the native roots INSIDE the temp directory before any
        // tick. An undeclared source is "never asked", which still falls
        // back to the conventional per-user location -- so a daemon started
        // on defaults scans the developer's real ~/.claude and ~/.codex. A
        // test suite that reads a person's actual transcripts is not one
        // anybody should run, and this harness did exactly that before the
        // declarations below were added.
        {
            let mut s = shared.settings.lock().unwrap();
            s.claude_source = Some(
                trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
                    path: dir.path().join("claude"),
                },
            );
            s.codex_source = Some(
                trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
                    path: dir.path().join("codex"),
                },
            );
        }

        Self { _dir: dir, shared }
    }

    /// Arm the project the staged conversation names.
    fn arm(&self) {
        self.shared
            .policy
            .lock()
            .unwrap()
            .set_mode(PROJECT, ProjectMode::AutoUpload, chrono::Utc::now())
            .expect("arming the project");
    }

    /// Two ticks: a first sighting is deliberately unstable, so one tick
    /// never queues anything.
    async fn settle(&self) {
        let now: chrono::DateTime<chrono::Utc> = "2030-01-01T00:00:00Z".parse().unwrap();
        trace_commons_contributor::daemon::watcher::tick(&self.shared, now)
            .await
            .unwrap();
        trace_commons_contributor::daemon::watcher::tick(&self.shared, now)
            .await
            .unwrap();
    }

    fn entries(&self) -> Vec<QueueEntry> {
        self.shared.queue.lock().unwrap().all().to_vec()
    }
}

#[tokio::test]
async fn a_staged_conversation_reaches_the_queue_as_antigravity() {
    let h = Harness::new();
    h.settle().await;

    let entries = h.entries();
    assert_eq!(
        entries.len(),
        1,
        "the staged conversation must be queued; got {entries:?}"
    );
    let entry = &entries[0];

    assert_eq!(
        entry.source, "trajectory",
        "the adapter that loads it is still what `source` names"
    );
    assert_eq!(
        entry.declared_source.as_deref(),
        Some("antigravity"),
        "but the queue must carry what the conversation declares itself to be"
    );
}

/// A conversation found in the staging directory is offered, never
/// auto-uploaded, even for a project armed for auto-upload.
///
/// This is a consent judgement, not a mechanical one. A contributor who
/// armed auto-upload did so for a watched source they had declared. An
/// imported conversation was invisible to this daemon until it upgraded;
/// taking it straight to Approved would send, with no further prompt,
/// something they may not remember importing.
#[tokio::test]
async fn a_staged_conversation_is_offered_even_when_the_project_is_armed() {
    let h = Harness::new();
    h.arm();
    h.settle().await;

    let entries = h.entries();
    assert_eq!(entries.len(), 1, "got {entries:?}");
    assert_eq!(
        entries[0].state,
        QueueState::Pending,
        "an armed project must not arm an imported conversation"
    );
}
