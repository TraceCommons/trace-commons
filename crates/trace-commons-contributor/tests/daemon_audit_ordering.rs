//! The audited socket actions must record first and act second.
//!
//! Arming auto-upload, bulk-approving a queue, and widening consent scopes
//! are all fail-closed on their audit entry: if the record cannot be
//! written, the action does not happen. That was implemented by acting
//! first and rolling back on an append failure -- and the rollback was a
//! second write to the same disk that had just refused the first one.
//! Disk-full and permissions failures do not politely apply to one write
//! and not the next, so the rollback would fail too, leaving the change
//! standing on disk with no record of it. The daemon reads all three back
//! from disk on restart, so the guarantee did not survive a reboot.
//!
//! `acknowledge_near_ai_notice` already had the right shape: record, then
//! act, so there is nothing to roll back. These tests hold the other three
//! to it.
//!
//! The audit log is broken here by making it invalid UTF-8, which
//! `audit::load` refuses -- an append is a whole-file read-modify-write, so
//! a log it cannot read is a log it cannot append to. That fails the append
//! specifically, leaving every other write on the store working, which is
//! what makes "the change was never written at all" an observable claim
//! rather than a side effect of a dead directory.

use std::sync::Arc;

use trace_commons_contributor::config::{
    CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig, DAEMON_AUDIT_FILE,
    DAEMON_PROJECTS_FILE, DAEMON_QUEUE_FILE,
};
use trace_commons_contributor::daemon::ipc::{self, DaemonShared};
use trace_commons_contributor::daemon::policy::{ProjectMode, ProjectPolicy};
use trace_commons_contributor::daemon::queue::{Queue, QueueEntry, QueueState, entry_id_for};
use trace_commons_contributor::daemon::settings::SourceDeclaration;
use trace_commons_contributor::identity::DeviceIdentity;
use trace_commons_contributor::source::TraceSource;
use trace_commons_contributor::source::claude_code::ClaudeCodeSource;

struct Harness {
    _dir: tempfile::TempDir,
    shared: Arc<DaemonShared>,
}

fn cfg(device_key_id: String) -> ContributorConfig {
    ContributorConfig {
        inference_receipt_endpoint: None,
        schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
        issuer_url: "http://issuer.invalid".into(),
        ingest_url: "http://ingest.invalid".into(),
        audience: "trace-commons-upload".into(),
        tenant_id: "tenant-abc".into(),
        instance_id: "instance-1".into(),
        user_subject: "alice".into(),
        device_key_id,
        consent_scopes: vec!["debugging_evaluation".into()],
        pii_filter: None,
        allowed_hosts: None,
        display_handle: None,
        public_bio: None,
        public_since: None,
        witness: None,
    }
}

impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().join("state")).unwrap();
        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        store.save_config(&cfg(device.device_key_id)).unwrap();
        let shared = Arc::new(DaemonShared::load(store).unwrap());
        Self { _dir: dir, shared }
    }

    /// Make `audit::append` fail, and only `audit::append`.
    fn break_the_audit_log(&self) {
        self.shared
            .store
            .write_daemon_file(DAEMON_AUDIT_FILE, &[0xff, 0xfe, 0xfd])
            .unwrap();
    }

    /// A real, canonical, existing directory, which is what
    /// `project_key_is_admissible` accepts as a new key.
    fn project_key(&self) -> String {
        let path = self.shared.store.dir().join("a-project");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::canonicalize(&path)
            .unwrap()
            .to_string_lossy()
            .to_string()
    }

    /// A pending entry over a session that really exists, with the source
    /// declared so the daemon can find it. `approve` builds and pins the
    /// envelope for anything unpreviewed, and an entry whose session file
    /// is not there is skipped rather than approved -- so a fixture with a
    /// made-up path would answer a question about missing files instead of
    /// the one these tests ask about the audit log.
    fn queue_one_pending(&self) -> QueueEntry {
        let sessions_root = self.shared.store.dir().join("sessions/projects");
        let project = sessions_root.join("-Users-testuser-code-proj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("44444444-4444-4444-4444-444444444444.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\
             \"content\":\"list the files\"},\
             \"cwd\":\"/Users/testuser/code/proj\",\
             \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
             \"sessionId\":\"44444444-4444-4444-4444-444444444444\",\
             \"uuid\":\"a1\"}\n",
        )
        .unwrap();
        let src = ClaudeCodeSource::new(sessions_root.clone());
        let session_ref = TraceSource::discover(&src).unwrap().remove(0);
        {
            let mut settings = self.shared.settings.lock().unwrap();
            settings.claude_source = Some(SourceDeclaration::Watch {
                path: sessions_root.clone(),
            });
            settings.save(&self.shared.store).unwrap();
        }
        let entry = QueueEntry {
            entry_id: entry_id_for("sha256:aa"),
            session_hash: "sha256:aa".into(),
            source: "claude-code".into(),
            declared_source: None,
            project_key: "/Users/testuser/code/proj".into(),
            project_path: None,
            session_cwd: None,
            project_label: "proj".into(),
            path: session_ref.path.clone(),
            size_bytes: session_ref.size_bytes,
            discovered_at: chrono::Utc::now(),
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
        };
        let mut q = self.shared.queue.lock().unwrap();
        q.upsert(entry.clone(), 100).unwrap();
        q.save(&self.shared.store).unwrap();
        entry
    }
}

#[test]
fn an_audit_write_failure_leaves_no_armed_policy_on_disk() {
    let h = Harness::new();
    h.break_the_audit_log();
    let key = h.project_key();

    let resp = ipc::handle_local(
        &h.shared,
        "set_project_mode",
        serde_json::json!({ "project_key": key, "mode": "auto_upload" }),
    );
    assert_eq!(
        resp.error.as_ref().map(|e| e.message.as_str()),
        Some("audit-write-failed"),
        "the call must fail when its record cannot be written"
    );

    // The strong claim: the policy file was never written at all. The old
    // ordering wrote it, appended, and then wrote it back -- so on a disk
    // where the write-back also failed, an armed policy survived the
    // restart with no record of it. Recording first means there is nothing
    // that has to succeed twice.
    assert!(
        h.shared
            .store
            .read_daemon_file(DAEMON_PROJECTS_FILE)
            .unwrap()
            .is_none(),
        "nothing may be persisted before the audit entry is"
    );
    assert_eq!(
        ProjectPolicy::load(&h.shared.store).unwrap().resolve(&key),
        ProjectMode::NotifyOnly,
        "a daemon restarting from disk must not find this project armed"
    );
    assert_eq!(
        h.shared.policy.lock().unwrap().resolve(&key),
        ProjectMode::NotifyOnly,
        "and the running process must not think it is armed either"
    );
}

#[test]
fn a_notify_only_mode_change_is_not_blocked_by_a_broken_audit_log() {
    // Only arming is audited. A contributor silencing a project must not be
    // stopped by an unrelated log they cannot repair.
    let h = Harness::new();
    h.break_the_audit_log();
    let key = h.project_key();

    let resp = ipc::handle_local(
        &h.shared,
        "set_project_mode",
        serde_json::json!({ "project_key": key, "mode": "ignore" }),
    );
    assert!(resp.error.is_none(), "{:?}", resp.error);
    assert_eq!(
        ProjectPolicy::load(&h.shared.store).unwrap().resolve(&key),
        ProjectMode::Ignore
    );
}

#[test]
fn an_audit_write_failure_leaves_no_bulk_approval_on_disk() {
    let h = Harness::new();
    let entry = h.queue_one_pending();
    h.break_the_audit_log();

    let resp = ipc::handle_local(&h.shared, "approve", serde_json::json!({ "all": true }));
    assert_eq!(
        resp.error.as_ref().map(|e| e.message.as_str()),
        Some("audit-write-failed")
    );

    assert_eq!(
        h.shared
            .queue
            .lock()
            .unwrap()
            .get(entry.entry_id)
            .unwrap()
            .state,
        QueueState::Pending,
        "nothing may be approved before the record of it is written"
    );
    let on_disk = Queue::load(&h.shared.store).unwrap();
    assert_eq!(
        on_disk.get(entry.entry_id).unwrap().state,
        QueueState::Pending,
        "and a daemon restarting from disk must not find it approved"
    );
    // The queue file is untouched, not rewritten-and-restored.
    assert!(
        h.shared
            .store
            .read_daemon_file(DAEMON_QUEUE_FILE)
            .unwrap()
            .is_some()
    );
}

#[test]
fn an_audit_write_failure_leaves_the_old_consent_scopes_on_disk() {
    let h = Harness::new();
    h.break_the_audit_log();

    let resp = ipc::handle_local(
        &h.shared,
        "set_consent_scopes",
        serde_json::json!({ "scopes": ["debugging_evaluation", "model_training"] }),
    );
    assert_eq!(
        resp.error.as_ref().map(|e| e.message.as_str()),
        Some("audit-write-failed")
    );
    assert_eq!(
        h.shared
            .store
            .load_config()
            .unwrap()
            .unwrap()
            .consent_scopes,
        vec!["debugging_evaluation".to_string()],
        "consent must not be widened without a record of the widening"
    );
}

#[test]
fn a_single_entry_approval_is_not_blocked_by_a_broken_audit_log() {
    // Only the bulk form is audited: approving one entry the contributor is
    // looking at is not the action the terminal-only restriction covered.
    let h = Harness::new();
    let entry = h.queue_one_pending();
    h.break_the_audit_log();

    let resp = ipc::handle_local(
        &h.shared,
        "approve",
        serde_json::json!({ "entry_id": entry.entry_id.to_string() }),
    );
    assert!(resp.error.is_none(), "{:?}", resp.error);
    assert_eq!(
        Queue::load(&h.shared.store)
            .unwrap()
            .get(entry.entry_id)
            .unwrap()
            .state,
        QueueState::Approved
    );
}
