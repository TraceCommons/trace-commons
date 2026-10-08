use std::sync::Arc;

use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::config::{
    ACCOUNT_SESSION_FILE, DAEMON_SETTINGS_FILE, INVITE_SUBJECT_FILE, REMEMBERED_PASSKEYS_FILE,
};
use crate::daemon::ipc::{ERR_BUSY, ERR_UNAVAILABLE, handle_request, handle_request_async};
use crate::daemon::queue::{QueueEntry, QueueState};

fn shared() -> DaemonShared {
    let (dir, store) = crate::config::tests_support::temp_store();
    std::mem::forget(dir);
    DaemonShared::load(store).unwrap()
}

fn req(method: &str, params: serde_json::Value) -> Request {
    Request {
        id: 7,
        method: method.to_string(),
        params,
    }
}

/// A whole enrollment on disk: a device key, a config whose scopes were
/// chosen, an account session, and an invite subject.
fn enroll_fixture(s: &DaemonShared) -> crate::config::ContributorConfig {
    crate::identity::DeviceIdentity::load_or_generate(&s.store).unwrap();
    let mut cfg = crate::commands::unenrolled_preview_config();
    cfg.consent_scopes_chosen = Some(true);
    s.store.save_config(&cfg).unwrap();
    let session = json!({
        "access_token": "opaque-session-token",
        "expires_at": Utc::now() + chrono::Duration::hours(1),
        "account_id": "acct-1",
    });
    s.store
        .write_daemon_file(
            ACCOUNT_SESSION_FILE,
            serde_json::to_vec(&session).unwrap().as_slice(),
        )
        .unwrap();
    s.store
        .write_daemon_file(
            INVITE_SUBJECT_FILE,
            br#"{"invite_subject_hash":"sha256:00"}"#,
        )
        .unwrap();
    assert!(crate::daemon::uploader::enrollment_is_live(&s.store));
    assert!(crate::account_auth::session_status(&s.store).is_some());
    cfg
}

// Redundant in a test-only file, and what `queue_fixture_default_guard`
// reads, since it scans this file on its own.
#[cfg(test)]
fn seed(s: &DaemonShared, state: QueueState) -> Uuid {
    let entry_id = Uuid::new_v4();
    let mut queue = s.queue.lock().unwrap();
    queue
        .upsert(
            QueueEntry {
                entry_id,
                session_hash: format!("sha256:{entry_id}"),
                source: "claude-code".to_string(),
                project_key: "/tmp/p".to_string(),
                project_label: "p".to_string(),
                path: std::path::PathBuf::from("/tmp/seed.jsonl"),
                size_bytes: 1,
                discovered_at: Utc::now(),
                ..Default::default()
            },
            500,
        )
        .unwrap();
    queue.set_state(entry_id, state, None);
    entry_id
}

/// An entry the person previewed and approved: pinned to stored bytes.
fn seed_previewed_approval(s: &DaemonShared) -> Uuid {
    let id = seed(s, QueueState::Pending);
    let name = crate::daemon::approved_envelope::file_name(id);
    s.store.write_daemon_file(&name, b"{}").unwrap();
    let mut queue = s.queue.lock().unwrap();
    assert!(queue.record_previewed_envelope(id, "sha256:pinned", None, Some(2)));
    let scopes = vec!["debugging_evaluation".to_string()];
    assert!(queue.approve(id, &scopes, Some("sha256:inputs"), None, None, None));
    assert_eq!(queue.get(id).unwrap().state, QueueState::Approved);
    id
}

async fn call(s: &DaemonShared) -> Response {
    handle_request_async(s, &req("unenroll", json!({}))).await
}

#[tokio::test]
async fn unenroll_removes_config_credentials_and_session_and_reads_not_logged_in() {
    let s = shared();
    enroll_fixture(&s);
    assert_eq!(s.status_value()["logged_in"], true);

    let r = call(&s).await;
    assert!(r.error.is_none(), "{:?}", r.error);
    let v = r.result.unwrap();
    assert_eq!(v["unenrolled"], true);
    assert_eq!(v["removed"], true);

    assert!(s.store.load_config().unwrap().is_none());
    assert!(!s.store.device_key_path().exists());
    assert!(s.store.load_device_key().unwrap().is_none());
    assert!(!s.store.daemon_path(ACCOUNT_SESSION_FILE).exists());
    assert!(crate::account_auth::session_status(&s.store).is_none());
    assert!(!s.store.daemon_path(INVITE_SUBJECT_FILE).exists());
    assert_eq!(s.status_value()["logged_in"], false);
    assert!(s.status_value()["tenant_id"].is_null());
}

#[tokio::test]
async fn unenroll_keeps_what_is_not_the_enrollment() {
    let s = shared();
    enroll_fixture(&s);
    s.store
        .write_daemon_file(REMEMBERED_PASSKEYS_FILE, b"{\"passkeys\":[]}")
        .unwrap();
    s.store
        .write_daemon_file(DAEMON_SETTINGS_FILE, b"{}")
        .unwrap();
    let waiting = seed(&s, QueueState::Pending);
    s.queue.lock().unwrap().save(&s.store).unwrap();

    assert!(call(&s).await.error.is_none());

    // A passkey is not an enrollment, and the person's choices about this
    // Mac are not an account's.
    assert!(s.store.daemon_path(REMEMBERED_PASSKEYS_FILE).exists());
    assert!(s.store.daemon_path(DAEMON_SETTINGS_FILE).exists());
    assert_eq!(
        s.queue.lock().unwrap().get(waiting).unwrap().state,
        QueueState::Pending
    );
    // The audit log is kept, and records the call.
    assert!(!crate::daemon::audit::load(&s.store).unwrap().is_empty());
}

#[tokio::test]
async fn unenroll_writes_one_label_only_audit_row() {
    let s = shared();
    let cfg = enroll_fixture(&s);
    seed_previewed_approval(&s);

    assert!(call(&s).await.error.is_none());

    let rows: Vec<_> = crate::daemon::audit::load(&s.store)
        .unwrap()
        .into_iter()
        .filter(|e| e.action == AUDIT_ACTION)
        .collect();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.project_label, None);
    assert_eq!(row.detail.as_deref(), Some("approvals_returned=1"));
    let written = serde_json::to_string(row).unwrap();
    for secret in [
        cfg.tenant_id.as_str(),
        cfg.device_key_id.as_str(),
        cfg.user_subject.as_str(),
        "opaque-session-token",
        "acct-1",
        s.store.dir().to_str().unwrap(),
    ] {
        assert!(!written.contains(secret), "audit row names {secret}");
    }
}

#[tokio::test]
async fn approved_entries_return_to_waiting_and_never_upload_afterwards() {
    let s = Arc::new(shared());
    enroll_fixture(&s);
    let id = seed_previewed_approval(&s);
    let envelope = s
        .store
        .daemon_path(&crate::daemon::approved_envelope::file_name(id));
    assert!(envelope.exists());

    let v = call(&s).await.result.unwrap();
    assert_eq!(v["approvals_returned"], 1);

    {
        let queue = s.queue.lock().unwrap();
        let entry = queue.get(id).unwrap();
        assert_eq!(entry.state, QueueState::Pending);
        assert_eq!(
            entry.reason_label.as_deref(),
            Some(crate::daemon::preview::REASON_INPUTS_CHANGED)
        );
        assert!(entry.approved_inputs.is_none());
        assert!(entry.approved_scopes.is_none());
        assert!(entry.previewed_envelope_digest.is_none());
    }
    // The bytes stamped with the old enrollment are gone with it.
    assert!(!envelope.exists());

    // An upload pass after the unenroll sends nothing, and nor does one
    // after enrolling again: the approval did not survive.
    crate::daemon::drain_approved_for_test(&s, Utc::now() + chrono::Duration::hours(1))
        .await
        .unwrap();
    enroll_fixture(&s);
    crate::daemon::drain_approved_for_test(&s, Utc::now() + chrono::Duration::hours(1))
        .await
        .unwrap();
    let queue = s.queue.lock().unwrap();
    let entry = queue.get(id).unwrap();
    assert_eq!(entry.state, QueueState::Pending);
    assert_eq!(entry.attempts, 0);
    assert!(entry.submission_id.is_none());
}

#[tokio::test]
async fn a_pending_pin_is_released_too() {
    let s = shared();
    enroll_fixture(&s);
    let id = seed(&s, QueueState::Pending);
    s.store
        .write_daemon_file(&crate::daemon::approved_envelope::file_name(id), b"{}")
        .unwrap();
    assert!(
        s.queue
            .lock()
            .unwrap()
            .record_previewed_envelope(id, "sha256:pinned", None, None)
    );

    assert!(call(&s).await.error.is_none());

    assert!(
        s.queue
            .lock()
            .unwrap()
            .get(id)
            .unwrap()
            .previewed_envelope_digest
            .is_none()
    );
}

#[tokio::test]
async fn unenroll_refuses_while_an_upload_is_in_flight_and_removes_nothing() {
    let s = shared();
    enroll_fixture(&s);
    seed(&s, QueueState::Uploading);

    let r = call(&s).await;
    let error = r.error.expect("refused");
    assert_eq!(error.code, ERR_BUSY);
    assert_eq!(error.message, ERR_UPLOAD_IN_FLIGHT);
    assert!(crate::daemon::uploader::enrollment_is_live(&s.store));
    assert!(
        crate::daemon::audit::load(&s.store)
            .unwrap()
            .iter()
            .all(|e| e.action != AUDIT_ACTION)
    );
}

#[tokio::test]
async fn unenroll_with_nothing_enrolled_succeeds_and_records_nothing() {
    let s = shared();
    let v = call(&s).await.result.unwrap();
    assert_eq!(v["unenrolled"], true);
    assert_eq!(v["removed"], false);
    assert!(crate::daemon::audit::load(&s.store).unwrap().is_empty());
}

/// The crash the order is chosen for: the credentials are gone and the
/// config is not. That state reads as not enrolled, can send nothing, and
/// a second call finishes it.
#[tokio::test]
async fn a_crash_between_credentials_and_config_reads_not_enrolled_and_is_finished_by_a_retry() {
    let s = shared();
    enroll_fixture(&s);
    commons_credentials::clear(&s.store, &Kind::ALL).unwrap();
    assert!(s.store.load_config().unwrap().is_some());
    assert_eq!(s.status_value()["logged_in"], false);
    assert!(!crate::daemon::uploader::enrollment_is_live(&s.store));

    let v = call(&s).await.result.unwrap();
    assert_eq!(v["removed"], true);
    assert!(s.store.load_config().unwrap().is_none());
    assert!(!s.store.daemon_path(INVITE_SUBJECT_FILE).exists());
}

/// The other half of the order: the credentials always go, whatever
/// happens to the files after them.
#[tokio::test]
async fn the_credentials_are_gone_even_when_a_later_file_cannot_be_removed() {
    let s = shared();
    enroll_fixture(&s);
    // A directory where an enrollment file should be: `remove_file` fails.
    let blocker = s.store.daemon_path(crate::config::LEGACY_INVITE_LINK_FILE);
    std::fs::create_dir(&blocker).unwrap();

    let r = call(&s).await;
    assert_eq!(r.error.expect("refused").message, ERR_UNENROLL_FAILED);
    assert!(!crate::daemon::uploader::enrollment_is_live(&s.store));
    assert!(s.store.load_device_key().unwrap().is_none());
    assert_eq!(s.status_value()["logged_in"], false);

    std::fs::remove_dir(&blocker).unwrap();
    assert!(call(&s).await.error.is_none());
    assert!(s.store.load_config().unwrap().is_none());
}

#[tokio::test]
async fn admission_answers_and_passkey_ceremonies_do_not_outlive_the_enrollment() {
    let s = shared();
    let cfg = enroll_fixture(&s);
    s.account_admission.record_for_test(&cfg, "bounded", true);
    assert_ne!(
        s.account_admission.current(Some(&cfg)),
        crate::daemon::automatic_gate::AccountAdmission::NotAdvertised
    );

    assert!(call(&s).await.error.is_none());

    // Even for a config that came back word for word.
    assert_eq!(
        s.account_admission.current(Some(&cfg)),
        crate::daemon::automatic_gate::AccountAdmission::NotAdvertised
    );
    assert!(s.native_identity.lock().unwrap().is_empty());
}

/// Start a real `/v1/onboard` responder on an ephemeral `127.0.0.1` port.
async fn spawn_onboard_mock() -> String {
    use axum::{Json, Router, routing::post};
    let router = Router::new().route(
        "/v1/onboard",
        post(|| async move {
            Json(json!({
                "schema_version": "trace_commons.onboard_response.v1",
                "tenant_id": "tenant-second",
                "ingest_url": "https://ingest.invalid",
                "issuer_url": "https://issuer.invalid",
                "audience": "trace-commons-upload",
                "device_key_id": "sha256:mockdevice",
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn the_daemon_can_be_enrolled_again_with_a_new_device_key() {
    let s = shared();
    enroll_fixture(&s);
    let old_key = s.store.load_device_key().unwrap().unwrap();
    assert!(call(&s).await.error.is_none());

    let base = spawn_onboard_mock().await;
    let r = handle_request_async(
        &s,
        &req(
            "enroll",
            json!({ "invite": format!("{base}/onboard#SECOND-CODE") }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "{:?}", r.error);
    assert_eq!(r.result.unwrap()["enrolled"], true);
    assert_eq!(
        s.store.load_config().unwrap().unwrap().tenant_id,
        "tenant-second"
    );
    let new_key = s.store.load_device_key().unwrap().unwrap();
    assert_ne!(
        old_key, new_key,
        "a new enrollment never reuses the old key"
    );
}

#[test]
fn unenroll_answers_only_on_the_async_dispatcher() {
    let s = shared();
    enroll_fixture(&s);
    let r = handle_request(&s, &req("unenroll", json!({})));
    let error = r.error.expect("refused synchronously");
    assert_eq!(error.code, ERR_UNAVAILABLE);
    assert_eq!(error.message, "unenroll-requires-async");
    assert!(crate::daemon::uploader::enrollment_is_live(&s.store));
}

/// Store a counter row for Insights feed T, as a watcher pass would.
fn counted(s: &DaemonShared) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    std::fs::write(
        &path,
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/insights/claude-turn-series/session.jsonl"),
        )
        .unwrap(),
    )
    .unwrap();
    let written = Utc::now() - chrono::Duration::hours(2);
    let candidate = crate::daemon::insights_week::CounterCandidate {
        origin: 0,
        source: crate::source::SOURCE_CLAUDE_CODE,
        size_bytes: std::fs::metadata(&path).unwrap().len(),
        path,
        modified_at: written,
        started_at: Some(written),
        group_member_count: 0,
    };
    let summary = s
        .insights_counter
        .run_pass(&[candidate], &[], Utc::now(), 60, &mut |_| {
            ("/proj".to_string(), false)
        })
        .unwrap();
    assert_eq!(summary.read, 1);
    assert!(rows_file(s).exists());
}

fn rows_file(s: &DaemonShared) -> std::path::PathBuf {
    s.store
        .daemon_path(crate::daemon::insights_week::COUNTER_ROWS_FILE)
}

fn stored(s: &DaemonShared) -> serde_json::Value {
    s.insights_counter.week_value(
        true,
        None,
        chrono::FixedOffset::east_opt(0).unwrap(),
        Utc::now(),
        &[],
    )["sessions_stored"]
        .clone()
}

/// Owner decision D4, open: the counter store is cleared on unenroll, and
/// its key forgotten, whether or not an enrollment was on disk.
#[tokio::test]
async fn unenroll_clears_the_insights_counter_store() {
    let s = shared();
    enroll_fixture(&s);
    counted(&s);
    assert_eq!(stored(&s), 1);
    assert!(call(&s).await.error.is_none());
    assert!(!rows_file(&s).exists());
    assert_eq!(stored(&s), 0);
    assert!(s.insights_counter.key_is_absent_for_test());

    let s = shared();
    counted(&s);
    let v = call(&s).await.result.unwrap();
    assert_eq!(v["removed"], false);
    assert!(!rows_file(&s).exists());
    assert!(s.insights_counter.key_is_absent_for_test());
}
