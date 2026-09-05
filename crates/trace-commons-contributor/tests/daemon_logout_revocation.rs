//! Logging out has to actually revoke.
//!
//! A minted upload claim stays valid for minutes after the enrollment behind
//! it is gone, and the daemon holds one in memory. Without these behaviours a
//! logout would leave a background process still uploading the contributor's
//! coding sessions against an enrollment they just revoked, appending to a
//! receipts file that no longer exists, and leaving the next person to enroll
//! on this machine holding the previous contributor's auto-upload opt-ins.

#![cfg(unix)]
// Drives the daemon over its unix-socket IPC (`ipc::bind`/`ipc::serve`, both
// `#[cfg(unix)]`). Ungated, this target fails to COMPILE on Windows rather
// than skipping, which is why the suite had never run there.

use std::sync::Arc;
use std::time::Duration;

use trace_commons_contributor::commands;
use trace_commons_contributor::config::{
    CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig, DAEMON_HISTORY_FILE,
    DAEMON_PROJECTS_FILE, DAEMON_QUEUE_FILE, DAEMON_SETTINGS_FILE, DAEMON_SOCK_FILE,
    DAEMON_STATE_FILE,
};
use trace_commons_contributor::daemon::ipc::{DaemonShared, bind, serve};
use trace_commons_contributor::daemon::policy::ProjectMode;
use trace_commons_contributor::identity::DeviceIdentity;

fn enrolled_store(dir: &std::path::Path) -> ConfigStore {
    let store = ConfigStore::open(dir.to_path_buf()).unwrap();
    let device = DeviceIdentity::load_or_generate(&store).unwrap();
    store
        .save_config(&ContributorConfig {
            inference_receipt_endpoint: None,
            inference_receipt_check_attestation: false,
            schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "https://issuer.example.ai".into(),
            ingest_url: "https://ingest.example.ai".into(),
            audience: "trace-commons-ingest".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device.device_key_id,
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
            witness: None,
        })
        .unwrap();
    store
}

#[tokio::test]
async fn logout_stops_a_running_daemon_and_removes_all_of_its_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = enrolled_store(&dir.path().join("state"));

    // Populate every daemon state file, including an auto-upload opt-in.
    let shared = Arc::new(
        DaemonShared::load(ConfigStore::open(store.dir().to_path_buf()).unwrap()).unwrap(),
    );
    shared
        .policy
        .lock()
        .unwrap()
        .set_mode(
            "/Users/z/code/proj",
            ProjectMode::AutoUpload,
            chrono::Utc::now(),
        )
        .unwrap();
    shared.policy.lock().unwrap().save(&shared.store).unwrap();
    shared.state.lock().unwrap().save(&shared.store).unwrap();
    shared.settings.lock().unwrap().save(&shared.store).unwrap();
    shared.queue.lock().unwrap().save(&shared.store).unwrap();
    store.write_daemon_file(DAEMON_HISTORY_FILE, b"").unwrap();

    let listener = bind(&store).await.unwrap();
    let serve_shared = Arc::clone(&shared);
    let server = tokio::spawn(async move { serve(listener, serve_shared).await });
    assert!(store.daemon_path(DAEMON_SOCK_FILE).exists());

    // Logout runs blocking socket I/O, so keep it off the async worker.
    let logout_store = ConfigStore::open(store.dir().to_path_buf()).unwrap();
    tokio::task::spawn_blocking(move || commands::logout(&logout_store).unwrap())
        .await
        .unwrap();

    for name in [
        DAEMON_PROJECTS_FILE,
        DAEMON_QUEUE_FILE,
        DAEMON_HISTORY_FILE,
        DAEMON_STATE_FILE,
        DAEMON_SETTINGS_FILE,
    ] {
        assert!(
            store.read_daemon_file(name).unwrap().is_none(),
            "{name} survived logout"
        );
    }
    assert!(store.load_config().unwrap().is_none());
    assert!(store.load_device_key().unwrap().is_none());
    assert!(
        !store.daemon_path(DAEMON_SOCK_FILE).exists(),
        "the socket must not outlive the logout"
    );

    // The daemon was asked to stop.
    assert!(shared.shutdown.load(std::sync::atomic::Ordering::Relaxed));
    server.abort();
}

#[tokio::test]
async fn logout_succeeds_when_no_daemon_is_running() {
    let dir = tempfile::tempdir().unwrap();
    let store = enrolled_store(&dir.path().join("state"));
    let logout_store = ConfigStore::open(store.dir().to_path_buf()).unwrap();
    tokio::task::spawn_blocking(move || commands::logout(&logout_store).unwrap())
        .await
        .unwrap();
    assert!(store.load_config().unwrap().is_none());
}

#[tokio::test]
async fn logout_is_not_blocked_by_a_stale_socket_from_a_crashed_daemon() {
    // A socket file with nothing listening must not wedge a logout: the
    // contributor asked to revoke, and revoking is what has to happen.
    let dir = tempfile::tempdir().unwrap();
    let store = enrolled_store(&dir.path().join("state"));
    std::fs::write(store.daemon_path(DAEMON_SOCK_FILE), b"").unwrap();

    let logout_store = ConfigStore::open(store.dir().to_path_buf()).unwrap();
    let done = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || commands::logout(&logout_store)),
    )
    .await
    .expect("logout must not hang on a stale socket")
    .unwrap();
    assert!(done.is_ok(), "{done:?}");
    assert!(store.load_config().unwrap().is_none());
}

#[tokio::test]
async fn logout_makes_a_real_running_daemon_exit() {
    // The weaker version of this test asserted only that the shutdown flag
    // flipped, and passed while the actual daemon kept running: the
    // supervisor did not wake until its next poll, a minute away, so logout
    // gave up waiting. Drive the real `run` loop and require it to return.
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("s");
    let store = enrolled_store(&state_dir);
    {
        // Poll rarely, so a daemon that only notices shutdown on its next
        // tick cannot pass this by accident.
        // Point the watcher at empty tempdirs. Left unset it would scan the
        // developer's real session store, which is both slow and none of a
        // test's business.
        let settings = trace_commons_contributor::daemon::settings::DaemonSettings {
            poll_interval_secs: 3600,
            claude_source: Some(
                trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
                    path: dir.path().join("empty-claude"),
                },
            ),
            codex_source: Some(
                trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
                    path: dir.path().join("empty-codex"),
                },
            ),
            ..Default::default()
        };
        settings.save(&store).unwrap();
    }

    let run_store = ConfigStore::open(state_dir.clone()).unwrap();
    let daemon =
        tokio::spawn(async move { trace_commons_contributor::daemon::run(run_store, true).await });

    // Wait for it to come up.
    for _ in 0..50 {
        if store.daemon_path(DAEMON_SOCK_FILE).exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        store.daemon_path(DAEMON_SOCK_FILE).exists(),
        "daemon never bound"
    );

    let logout_store = ConfigStore::open(state_dir).unwrap();
    tokio::task::spawn_blocking(move || commands::logout(&logout_store).unwrap())
        .await
        .unwrap();

    let exited = tokio::time::timeout(Duration::from_secs(10), daemon).await;
    assert!(
        exited.is_ok(),
        "the daemon must exit on logout, not merely acknowledge it"
    );
    exited.unwrap().unwrap().unwrap();
}
