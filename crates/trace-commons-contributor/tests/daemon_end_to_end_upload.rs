//! The whole daemon loop, against stub issuer and ingest servers.
//!
//! The unit tests cover each decision in isolation; this covers the thing a
//! contributor actually cares about, which is that a finished session in an
//! opted-in project reaches the server without anyone touching it, and that a
//! session in a project they have *not* opted in does not.

#![cfg(unix)]
// Drives the daemon over its unix-socket IPC (`ipc::bind`/`ipc::serve`, both
// `#[cfg(unix)]`). Ungated, this target fails to COMPILE on Windows rather
// than skipping, which is why the suite had never run there.

use std::sync::{Arc, Mutex};

use axum::{Json, Router, routing::post};

use trace_commons_contributor::config::{
    CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig,
};
use trace_commons_contributor::daemon::ipc::DaemonShared;
use trace_commons_contributor::daemon::policy::ProjectMode;
use trace_commons_contributor::daemon::queue::QueueState;
use trace_commons_contributor::identity::DeviceIdentity;

async fn spawn(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

fn stub_issuer() -> Router {
    Router::new().route(
        "/v1/trace-upload-claim",
        post(|| async {
            Json(serde_json::json!({
                "access_token": "stub-claim-jwt",
                "token_type": "Bearer",
                "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                "expires_in": 300,
                "consent_scopes": ["debugging_evaluation"],
                "allowed_uses": ["debugging", "evaluation"],
            }))
        }),
    )
}

fn stub_ingest(received: Arc<Mutex<Vec<serde_json::Value>>>) -> Router {
    Router::new().route(
        "/v1/traces",
        post(move |Json(body): Json<serde_json::Value>| {
            let received = received.clone();
            async move {
                received.lock().unwrap().push(body);
                Json(serde_json::json!({
                    "status": "accepted",
                    "credit_points_pending": 1.0,
                    "explanation": []
                }))
            }
        }),
    )
}

struct Harness {
    _dir: tempfile::TempDir,
    shared: Arc<DaemonShared>,
    claude_root: std::path::PathBuf,
    received: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl Harness {
    async fn new() -> Self {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;

        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().join("state")).unwrap();
        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        store
            .save_config(&ContributorConfig {
                schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
                issuer_url: issuer,
                ingest_url: ingest,
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
            })
            .unwrap();

        let claude_root = dir.path().join("projects");
        std::fs::create_dir_all(&claude_root).unwrap();
        let shared = Arc::new(DaemonShared::load(store).unwrap());
        {
            let mut s = shared.settings.lock().unwrap();
            s.claude_source = Some(
                trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
                    path: claude_root.clone(),
                },
            );
            s.codex_source = Some(
                trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
                    path: dir.path().join("codex"),
                },
            );
        }
        Self {
            _dir: dir,
            shared,
            claude_root,
            received,
        }
    }

    fn write_session(&self, project: &str, id: &str) {
        let project_dir = self
            .claude_root
            .join(format!("-Users-testuser-code-{project}"));
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join(format!("{id}.jsonl")),
            format!(
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"fix the parser\"}},\
                 \"cwd\":\"/Users/testuser/code/{project}\",\
                 \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                 \"sessionId\":\"{id}\",\"uuid\":\"a1\"}}\n"
            ),
        )
        .unwrap();
    }

    fn opt_in(&self, project: &str) {
        self.shared
            .policy
            .lock()
            .unwrap()
            .set_mode(
                &format!("/Users/testuser/code/{project}"),
                ProjectMode::AutoUpload,
                chrono::Utc::now(),
            )
            .unwrap();
    }

    /// Two watch passes (eligibility needs a stable size), then an upload pass.
    async fn run_cycle(&self) {
        // Far-future clock so the fixture's real mtime reads as quiescent.
        let now: chrono::DateTime<chrono::Utc> = "2030-01-01T00:00:00Z".parse().unwrap();
        trace_commons_contributor::daemon::watcher::tick(&self.shared, now)
            .await
            .unwrap();
        trace_commons_contributor::daemon::watcher::tick(&self.shared, now)
            .await
            .unwrap();
        trace_commons_contributor::daemon::drain_approved_for_test(&self.shared, now)
            .await
            .unwrap();
    }

    fn states(&self) -> Vec<QueueState> {
        self.shared
            .queue
            .lock()
            .unwrap()
            .all()
            .iter()
            .map(|e| e.state)
            .collect()
    }
}

#[tokio::test]
async fn a_finished_session_in_an_opted_in_project_uploads_unattended() {
    let h = Harness::new().await;
    h.opt_in("myproj");
    h.write_session("myproj", "11111111-1111-1111-1111-111111111111");

    h.run_cycle().await;

    assert_eq!(
        h.received.lock().unwrap().len(),
        1,
        "an opted-in project should have uploaded without anyone acting"
    );
    assert_eq!(h.states(), vec![QueueState::Uploaded]);

    // And the receipt is recorded locally, so a re-run does not re-send it.
    let receipts = h.shared.store.load_receipts().unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].status, "accepted");
}

#[tokio::test]
async fn a_session_in_a_project_that_was_never_opted_in_waits_for_a_decision() {
    let h = Harness::new().await;
    h.write_session("otherproj", "22222222-2222-2222-2222-222222222222");

    h.run_cycle().await;

    assert_eq!(
        h.received.lock().unwrap().len(),
        0,
        "nothing may leave the machine without an opt-in or an approval"
    );
    assert_eq!(h.states(), vec![QueueState::Pending]);
}

#[tokio::test]
async fn approving_a_pending_session_uploads_it_on_the_next_pass() {
    let h = Harness::new().await;
    h.write_session("otherproj", "33333333-3333-3333-3333-333333333333");
    h.run_cycle().await;
    assert_eq!(h.states(), vec![QueueState::Pending]);

    let entry_id = h.shared.queue.lock().unwrap().all()[0].entry_id;
    let resp = trace_commons_contributor::daemon::ipc::handle_local(
        &h.shared,
        "approve",
        serde_json::json!({ "entry_id": entry_id.to_string() }),
    );
    assert!(resp.error.is_none(), "{:?}", resp.error);

    h.run_cycle().await;
    assert_eq!(h.received.lock().unwrap().len(), 1);
    assert_eq!(h.states(), vec![QueueState::Uploaded]);
}

#[tokio::test]
async fn a_paused_daemon_uploads_nothing_even_for_an_opted_in_project() {
    // Pausing *before* anything is queued only proves discovery stops --
    // the old version of this test asserted `queue.all().is_empty()`, which
    // gave that away, and it passed while `drain_approved` had no pause
    // check at all and uploaded everything already approved. So: discover
    // and auto-approve first, pause second, and assert on the upload.
    let h = Harness::new().await;
    h.opt_in("myproj");
    h.write_session("myproj", "44444444-4444-4444-4444-444444444444");

    let now: chrono::DateTime<chrono::Utc> = "2030-01-01T00:00:00Z".parse().unwrap();
    trace_commons_contributor::daemon::watcher::tick(&h.shared, now)
        .await
        .unwrap();
    trace_commons_contributor::daemon::watcher::tick(&h.shared, now)
        .await
        .unwrap();
    assert_eq!(
        h.states(),
        vec![QueueState::Approved],
        "the armed project must have auto-approved before the pause"
    );

    // A real `pause` call sets both together; `DaemonShared::is_paused`
    // trusts `state.paused` once it holds the state lock, so the harness
    // must keep both in sync too.
    h.shared
        .paused
        .store(true, std::sync::atomic::Ordering::Relaxed);
    h.shared.state.lock().unwrap().paused = true;

    trace_commons_contributor::daemon::drain_approved_for_test(&h.shared, now)
        .await
        .unwrap();

    assert_eq!(
        h.received.lock().unwrap().len(),
        0,
        "a pause must stop uploads that were already approved, not only \
         stop discovering new ones"
    );
    assert_eq!(h.states(), vec![QueueState::Approved]);
}

#[tokio::test]
async fn a_second_cycle_does_not_re_upload_the_same_session() {
    let h = Harness::new().await;
    h.opt_in("myproj");
    h.write_session("myproj", "55555555-5555-5555-5555-555555555555");

    h.run_cycle().await;
    h.run_cycle().await;

    assert_eq!(
        h.received.lock().unwrap().len(),
        1,
        "an unchanged session must not be sent twice"
    );
}

// --- "Sent, waiting to hear back" ---------------------------------------
//
// After the daily cap was raised on a real machine, fourteen approved traces
// uploaded inside thirty seconds and the History screen kept showing the old
// counts -- correctly, because nothing had asked the server for verdicts and
// nothing had recorded the uploads locally either. `history_poll_secs` is
// 1800 there and the last poll had been 24 minutes earlier, so the outcome of
// a dozen uploads was up to half an hour from being visible. That is
// indistinguishable from the upload having failed.

/// `POST_UPLOAD_HISTORY_DELAY_SECS`, restated here so a change to it has to
/// be a deliberate change to the contract these tests describe.
const EXPECTED_DELAY_SECS: i64 = 90;

fn now() -> chrono::DateTime<chrono::Utc> {
    "2030-01-01T00:00:00Z".parse().unwrap()
}

#[tokio::test]
async fn an_upload_makes_history_due_far_sooner_than_the_poll_interval_would() {
    let h = Harness::new().await;
    h.opt_in("myproj");
    h.write_session("myproj", "11111111-1111-1111-1111-111111111111");

    h.run_cycle().await;
    assert_eq!(h.received.lock().unwrap().len(), 1);

    let due = h
        .shared
        .state
        .lock()
        .unwrap()
        .history_refresh_due_at
        .expect("an upload must schedule a read-back");
    assert_eq!(due, now() + chrono::Duration::seconds(EXPECTED_DELAY_SECS));
    // And that is emphatically sooner than the interval it replaces.
    let interval = h.shared.settings.lock().unwrap().history_poll_secs as i64;
    assert!(
        EXPECTED_DELAY_SECS < interval,
        "{EXPECTED_DELAY_SECS} < {interval}"
    );
}

#[tokio::test]
async fn a_tick_that_uploaded_nothing_schedules_no_read_back() {
    // No opted-in project, so the watcher offers rather than uploads.
    let h = Harness::new().await;
    h.write_session("myproj", "11111111-1111-1111-1111-111111111111");

    h.run_cycle().await;
    assert_eq!(h.received.lock().unwrap().len(), 0);
    assert_eq!(h.shared.state.lock().unwrap().history_refresh_due_at, None);
}

#[tokio::test]
async fn a_burst_schedules_one_read_back_rather_than_one_per_upload() {
    let h = Harness::new().await;
    h.opt_in("myproj");
    for n in 1..=4 {
        h.write_session("myproj", &format!("{n}{n}{n}{n}{n}{n}{n}{n}-{n}{n}{n}{n}-{n}{n}{n}{n}-{n}{n}{n}{n}-{n}{n}{n}{n}{n}{n}{n}{n}{n}{n}{n}{n}"));
    }

    h.run_cycle().await;
    assert_eq!(
        h.received.lock().unwrap().len(),
        4,
        "all four should upload"
    );

    // One deadline for the whole burst, not four.
    let due = h.shared.state.lock().unwrap().history_refresh_due_at;
    assert_eq!(
        due,
        Some(now() + chrono::Duration::seconds(EXPECTED_DELAY_SECS))
    );

    // A later pass that uploads again keeps the EARLIEST deadline rather
    // than pushing it back, so a burst spread over several passes still
    // resolves to one read-back instead of receding forever.
    h.write_session("myproj", "99999999-9999-9999-9999-999999999999");
    let later = now() + chrono::Duration::seconds(30);
    trace_commons_contributor::daemon::watcher::tick(&h.shared, later)
        .await
        .unwrap();
    trace_commons_contributor::daemon::watcher::tick(&h.shared, later)
        .await
        .unwrap();
    trace_commons_contributor::daemon::drain_approved_for_test(&h.shared, later)
        .await
        .unwrap();
    assert_eq!(h.received.lock().unwrap().len(), 5);
    assert_eq!(
        h.shared.state.lock().unwrap().history_refresh_due_at,
        due,
        "a second burst must not push the read-back further out"
    );
}

#[tokio::test]
async fn an_upload_is_visible_as_sent_immediately_rather_than_at_the_next_poll() {
    // No server was asked anything here: the row comes from the local
    // receipt the upload just wrote. That is what "sent, waiting to hear
    // back" means, and it is available the moment the upload lands.
    let h = Harness::new().await;
    h.opt_in("myproj");
    h.write_session("myproj", "11111111-1111-1111-1111-111111111111");

    h.run_cycle().await;

    let records =
        trace_commons_contributor::daemon::history::HistoryCache::load(&h.shared.store).unwrap();
    assert_eq!(records.len(), 1, "the upload should be in history at once");
    let receipts = h.shared.store.load_receipts().unwrap();
    assert_eq!(records[0].submission_id, receipts[0].submission_id);
    assert_eq!(records[0].status, receipts[0].status);
    assert_eq!(records[0].project_label, "myproj");
    // Nothing has been read back from the server for this row, and it does
    // not pretend otherwise.
    assert_eq!(records[0].last_refreshed_at, None);
    // And no local path rode along into the history file.
    let body = serde_json::to_string(&records[0]).unwrap();
    assert!(!body.contains(".jsonl"), "path leaked into history: {body}");
}
