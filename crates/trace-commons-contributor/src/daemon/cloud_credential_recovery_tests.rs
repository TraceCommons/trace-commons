//! Real settings-write failures and running-proxy credential withdrawal.
//! Nested under IPC so tests can inspect the proxy without a production API.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::config::{ConfigStore, DAEMON_SETTINGS_FILE, tests_support::temp_store};
use crate::daemon::cloud_credential_lifecycle::native;
#[cfg(target_os = "macos")]
use crate::daemon::cloud_credential_lifecycle::{Journal, sweep_legacy_cloud_entries};
#[cfg(target_os = "macos")]
use crate::daemon::cloud_credential_test_support::install_legacy_backend;
use crate::daemon::cloud_credential_test_support::{backend, install_backend};
use crate::daemon::credential_store::{CredentialError, CredentialReference, SecretBackend};
use crate::daemon::ipc::{DaemonShared, Request, handle_request_async};
use crate::daemon::nearai_credential::{api::MintedKey, ceremony};
use crate::daemon::private_inference::PrivateInference;
use crate::daemon::settings::{DaemonSettings, NearAiInferenceCredential, NearAiSession};

const DEADLINE: Duration = Duration::from_secs(10);
/// Converts a deadlock into a failure. Only a broken build ever reaches it;
/// see `pending_os_read_cannot_delay_forget_withdrawing_proxy_authority`.
const HANG_GUARD: Duration = Duration::from_secs(120);
const JOURNAL: &str = "cloud-credential-cleanup.json";

/// Parks one OS read until the test says so. There is deliberately no timeout
/// on the release: the read stays pending until the test sends on, or drops,
/// the sender, so "the read was still pending" is a fact the test controls
/// rather than a race it measures.
struct ReadBarrier {
    entered: tokio::sync::oneshot::Sender<()>,
    release: mpsc::Receiver<()>,
}

struct FaultBackend {
    inner: Arc<dyn SecretBackend>,
    fail_delete: AtomicBool,
    reads: AtomicUsize,
    pause_read: Mutex<Option<ReadBarrier>>,
    /// Paused reads that have come back out of their barrier.
    paused_reads_returned: AtomicUsize,
}

impl SecretBackend for FaultBackend {
    fn read(&self, reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let bytes = self.inner.read(reference)?;
        let pause = self.pause_read.lock().unwrap().take();
        if let Some(pause) = pause {
            let _ = pause.entered.send(());
            // A dropped sender (a panicking test) also releases the read, so
            // a failure cannot leave this thread parked.
            let released = pause.release.recv();
            self.paused_reads_returned.fetch_add(1, Ordering::SeqCst);
            released.map_err(|_| CredentialError::Unavailable)?;
        }
        Ok(bytes)
    }

    fn write(&self, reference: &CredentialReference, bytes: &[u8]) -> Result<(), CredentialError> {
        self.inner.write(reference, bytes)
    }

    fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
        if self.fail_delete.load(Ordering::SeqCst) {
            Err(CredentialError::Unavailable)
        } else {
            self.inner.delete(reference)
        }
    }
}

fn fault_backend(store: &ConfigStore) -> Arc<FaultBackend> {
    let backend = Arc::new(FaultBackend {
        inner: backend(store.dir()).unwrap(),
        fail_delete: AtomicBool::new(false),
        reads: AtomicUsize::new(0),
        pause_read: Mutex::new(None),
        paused_reads_returned: AtomicUsize::new(0),
    });
    install_backend(store, backend.clone());
    backend
}

fn credentials(name: &str) -> DaemonSettings {
    let now = chrono::Utc::now();
    DaemonSettings {
        private_inference: true,
        near_ai_inference: Some(NearAiInferenceCredential {
            key: format!("sk-synthetic-recovery-{name}"),
            key_id: name.into(),
            key_prefix: "sk-synthetic".into(),
            organization_id: "synthetic-org".into(),
            workspace_id: "synthetic-workspace".into(),
            minted_at: now,
        }),
        near_ai_session: Some(NearAiSession {
            refresh_token: format!("rt-synthetic-recovery-{name}"),
            refresh_token_expires_at: None,
            stored_at: now,
            user_agent: "Mozilla/5.0 Test".into(),
        }),
        ..Default::default()
    }
}

fn journal(store: &ConfigStore) -> Vec<CredentialReference> {
    let bytes = store.read_daemon_file(JOURNAL).unwrap().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    serde_json::from_value(value["references"].clone()).unwrap()
}

#[test]
fn source_declarations_migrate_legacy_credentials_before_saving() {
    let (_directory, store) = temp_store();
    let legacy = credentials("legacy-roots");
    store
        .write_daemon_file(DAEMON_SETTINGS_FILE, &serde_json::to_vec(&legacy).unwrap())
        .unwrap();
    let preferences = serde_json::json!({
        "claude_source": {"mode": "off"},
        "codex_source": {"mode": "off"}
    });
    // Reproduce the old GTK path: raw load -> declare roots -> save cannot
    // advance to the startup migration because this save refuses plaintext.
    let mut old_path = DaemonSettings::load(&store).unwrap();
    crate::daemon::settings::apply_settings_object(&mut old_path, &preferences).unwrap();
    assert_eq!(
        old_path.save(&store).unwrap_err().to_string(),
        "near_ai_credential_storage_migration_required"
    );
    let mut current = DaemonSettings::load_for_preferences(&store).unwrap();
    crate::daemon::settings::apply_settings_object(&mut current, &preferences).unwrap();
    current.save(&store).unwrap();
    let runtime = DaemonSettings::load_with_cloud_credentials(&store).unwrap();
    assert!(crate::daemon::settings::roots_declared(&runtime));
    assert_eq!(runtime.near_ai_inference, legacy.near_ai_inference);
    assert_eq!(runtime.near_ai_session, legacy.near_ai_session);
    let persisted = DaemonSettings::load(&store).unwrap();
    assert!(persisted.near_ai_inference.is_none());
    assert!(persisted.near_ai_session.is_none());
    assert!(persisted.cloud_credentials.is_some());
}

#[test]
fn metadata_only_preferences_do_not_read_the_os_entry() {
    let (_directory, store) = temp_store();
    let mut original = credentials("metadata-roots");
    original.save_for_test(&store).unwrap();
    let backend = fault_backend(&store);
    let reference = original.cloud_credentials.as_ref().unwrap().reference();
    backend.delete(&reference).unwrap();
    let mut preferences = DaemonSettings::load_for_preferences(&store).unwrap();
    preferences.private_inference_offer_seen = true;
    preferences.save(&store).unwrap();
    assert_eq!(backend.reads.load(Ordering::SeqCst), 0);
    assert_eq!(preferences.cloud_credentials, original.cloud_credentials);
    assert!(preferences.near_ai_inference.is_none());
    assert!(preferences.near_ai_session.is_none());
    let shared = DaemonShared::load(store).unwrap();
    // A store that answers "nothing under this reference" is, on macOS, the
    // shape an upgrade from the legacy keychain leaves, and is offered the
    // move; elsewhere there is no other store it could be in.
    let expected = if cfg!(target_os = "macos") {
        "migration_available"
    } else {
        "storage_unavailable"
    };
    assert_eq!(
        crate::daemon::nearai_credential::credential_state(&shared),
        expected
    );
}

struct PermissionsGuard {
    path: PathBuf,
    original: std::fs::Permissions,
}

impl PermissionsGuard {
    fn new(store: &ConfigStore) -> Self {
        // Unix rename needs a writable directory; Windows MoveFileEx cannot
        // replace a read-only destination. Both fail the real publication.
        #[cfg(unix)]
        let path = store.dir().to_path_buf();
        #[cfg(not(unix))]
        let path = store.daemon_path(DAEMON_SETTINGS_FILE);
        let original = std::fs::metadata(&path).unwrap().permissions();
        Self { path, original }
    }

    fn refuse_publication(&self) {
        let mut denied = self.original.clone();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            denied.set_mode(0o500);
        }
        #[cfg(not(unix))]
        denied.set_readonly(true);
        std::fs::set_permissions(&self.path, denied).unwrap();
    }
}

impl Drop for PermissionsGuard {
    fn drop(&mut self) {
        std::fs::set_permissions(&self.path, self.original.clone()).unwrap();
    }
}

#[test]
fn settings_publish_failure_preserves_old_authority_and_cleanup_reference() {
    let (_directory, store) = temp_store();
    let backend = fault_backend(&store);
    let mut old = credentials("old");
    old.save_for_test(&store).unwrap();
    let before = store
        .read_daemon_file(DAEMON_SETTINGS_FILE)
        .unwrap()
        .unwrap();
    let reads = backend.reads.load(Ordering::SeqCst);
    let published = AtomicBool::new(false);
    let permissions = PermissionsGuard::new(&store);
    let new = credentials("new");
    let result = native(&store).unwrap().replace(
        &old,
        new.near_ai_inference,
        new.near_ai_session,
        || {
            // accept runs after OS write/readback and the final settings read.
            assert!(backend.reads.load(Ordering::SeqCst) > reads);
            permissions.refuse_publication();
            Ok(())
        },
        |_| {
            published.store(true, Ordering::SeqCst);
            Ok(())
        },
    );
    drop(permissions);
    assert!(result.is_err(), "the actual settings publication must fail");
    assert!(!published.load(Ordering::SeqCst));
    assert_eq!(
        store
            .read_daemon_file(DAEMON_SETTINGS_FILE)
            .unwrap()
            .unwrap(),
        before
    );
    let references = journal(&store);
    let active = old.cloud_credentials.as_ref().unwrap().reference();
    assert_eq!(references.len(), 2);
    assert!(references.contains(&active));
    for reference in &references {
        assert!(
            backend.read(reference).is_ok(),
            "both prepared entries remain recoverable"
        );
    }
    native(&store).unwrap().cleanup().unwrap();
    for reference in references {
        assert_eq!(backend.read(&reference).is_ok(), reference == active);
    }
    assert_eq!(
        DaemonSettings::load_with_cloud_credentials(&store).unwrap(),
        old
    );
}

async fn running_proxy(store: ConfigStore, home: &tempfile::TempDir) -> Arc<DaemonShared> {
    let shared = Arc::new(DaemonShared::load(store).unwrap());
    *shared.private_inference.lock().await =
        Some(PrivateInference::with_port(home.path().into(), 0));
    shared.reconcile_private_inference().await;
    {
        let proxy = shared.private_inference.lock().await;
        let proxy = proxy.as_ref().unwrap();
        assert!(proxy.holds_credential());
        assert_eq!(proxy.starts(), 1, "{:?}", proxy.state());
    }
    shared
}

fn forget_request() -> Request {
    Request {
        id: 1,
        method: "near_ai_credential_forget".into(),
        params: serde_json::json!({}),
    }
}

async fn assert_withdrawn(shared: &DaemonShared) {
    let disk = DaemonSettings::load(&shared.store).unwrap();
    assert!(disk.cloud_credentials.is_none());
    {
        let memory = shared.settings.lock().unwrap();
        assert!(memory.near_ai_inference.is_none() && memory.near_ai_session.is_none());
    }
    let proxy = shared.private_inference.lock().await;
    let proxy = proxy.as_ref().unwrap();
    assert!(!proxy.holds_credential());
    assert_eq!(
        proxy.starts(),
        2,
        "the registry must drop the withdrawn key"
    );
}

#[tokio::test]
async fn forget_cleanup_failure_withdraws_proxy_authority_and_allows_retry() {
    let (_directory, store) = temp_store();
    let backend = fault_backend(&store);
    credentials("forget").save_for_test(&store).unwrap();
    let home = tempfile::tempdir().unwrap();
    let shared = running_proxy(store, &home).await;
    backend.fail_delete.store(true, Ordering::SeqCst);
    let response = handle_request_async(&shared, &forget_request()).await;
    assert_eq!(
        response.error.unwrap().message,
        "near_ai_credential_cleanup_required"
    );
    assert_withdrawn(&shared).await;
    assert_eq!(journal(&shared.store).len(), 1);
    let status = crate::daemon::nearai_credential::handle_status(&shared, &forget_request())
        .result
        .unwrap();
    assert_eq!(status["state"], "cleanup_required");
    backend.fail_delete.store(false, Ordering::SeqCst);
    let retry = handle_request_async(&shared, &forget_request()).await;
    assert_eq!(retry.result.unwrap()["removed"], false);
    assert!(journal(&shared.store).is_empty());
    assert_withdrawn(&shared).await;
    shared.stop_private_inference().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_os_read_cannot_delay_forget_withdrawing_proxy_authority() {
    let (_directory, store) = temp_store();
    let backend = fault_backend(&store);
    credentials("initial").save_for_test(&store).unwrap();
    let home = tempfile::tempdir().unwrap();
    let shared = running_proxy(store, &home).await;
    ceremony::persist(
        shared.store.dir(),
        MintedKey {
            key: "sk-synthetic-new-ceremony".into(),
            key_id: "new".into(),
            key_prefix: "sk-synthetic".into(),
            organization_id: "synthetic-org".into(),
            workspace_id: "synthetic-workspace".into(),
        },
        "rt-synthetic-new-ceremony".into(),
        "Mozilla/5.0 Test".into(),
    )
    .unwrap();
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, released) = mpsc::channel();
    *backend.pause_read.lock().unwrap() = Some(ReadBarrier {
        entered,
        release: released,
    });
    let reconciling = shared.clone();
    let reconcile = tokio::spawn(async move {
        reconciling.reconcile_private_inference().await;
    });
    tokio::time::timeout(DEADLINE, waiting)
        .await
        .unwrap()
        .unwrap();
    // The read is now parked and stays parked until `release` is sent, so
    // everything below happens while it is pending. No step races a clock.
    //
    // The mechanism: an OS read that may sit behind an unlock prompt must not
    // hold proxy ownership, which Forget needs to withdraw the live key.
    assert!(
        shared.private_inference.try_lock().is_ok(),
        "the pending OS read holds proxy ownership, so Forget would wait for it"
    );
    // The behaviour: Forget runs to completion while the read is pending.
    // Nothing releases the read until Forget returns, so if Forget depended
    // on it, it would never return. HANG_GUARD only turns that deadlock into a
    // failure rather than a hung test. It is not a speed claim, and a correct
    // Forget under any load stays far inside it.
    let forgetting = shared.clone();
    let forget =
        tokio::spawn(async move { handle_request_async(&forgetting, &forget_request()).await });
    let response = tokio::time::timeout(HANG_GUARD, forget)
        .await
        .expect("Forget waited for the unrelated OS read prompt")
        .unwrap();
    assert_eq!(
        backend.paused_reads_returned.load(Ordering::SeqCst),
        0,
        "the OS read must still be pending when Forget completes"
    );
    assert!(!reconcile.is_finished());
    assert!(response.error.is_none());
    assert_withdrawn(&shared).await;
    // Now let the stale reconciliation finish: it must not restore the key
    // that Forget withdrew while it was parked.
    release.send(()).unwrap();
    tokio::time::timeout(DEADLINE, reconcile)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(backend.paused_reads_returned.load(Ordering::SeqCst), 1);
    assert_withdrawn(&shared).await;
    shared.stop_private_inference().await;
}

// Never installed at all against `backend()`/`install_backend`: the sweep
// must reach only the legacy registry, never the current Cloud backend.
#[cfg(target_os = "macos")]
struct RefusingLegacyBackend;

#[cfg(target_os = "macos")]
impl SecretBackend for RefusingLegacyBackend {
    fn read(&self, _reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
        Err(CredentialError::NoEntry)
    }
    fn write(
        &self,
        _reference: &CredentialReference,
        _bytes: &[u8],
    ) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
    fn delete(&self, _reference: &CredentialReference) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
}

/// A legacy backend that deletes nothing and records what it was asked for.
///
/// The sweep's body is `#[cfg(target_os = "macos")]`, so every assertion
/// about what it deletes is macOS-only -- elsewhere it is a no-op and a test
/// of it would pass without exercising anything.
#[cfg(target_os = "macos")]
#[derive(Default)]
struct RecordingLegacyBackend(Mutex<Vec<CredentialReference>>);

#[cfg(target_os = "macos")]
impl SecretBackend for RecordingLegacyBackend {
    fn read(&self, _reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
        Err(CredentialError::NoEntry)
    }
    fn write(
        &self,
        _reference: &CredentialReference,
        _bytes: &[u8],
    ) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
    fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
        self.0.lock().unwrap().push(*reference);
        Ok(())
    }
}

/// The reference settings currently point at is not an orphan. On the first
/// launch after the upgrade the journal names exactly one reference -- the
/// active one -- and the legacy entry behind it is the contributor's working
/// `sk-` key. A sweep that deletes it destroys a credential they still hold.
#[test]
#[cfg(target_os = "macos")]
fn the_sweep_never_deletes_the_active_reference() {
    let (_directory, store) = temp_store();
    let mut settings = credentials("sweep-active-roots");
    settings.save_for_test(&store).unwrap();
    let active = settings.cloud_credentials.as_ref().unwrap().reference();
    // What `cleanup_locked` leaves behind: the active reference, still in the
    // journal because it is skipped for deletion and never pruned.
    assert_eq!(
        journal(&store),
        vec![active],
        "the post-replace journal must name the active reference"
    );
    let legacy = Arc::new(RecordingLegacyBackend::default());
    install_legacy_backend(&store, legacy.clone());

    sweep_legacy_cloud_entries(&store);

    assert_eq!(
        *legacy.0.lock().unwrap(),
        Vec::new(),
        "the active reference must never be handed to the legacy delete"
    );
}

/// An orphan -- a reference the settings no longer point at -- is what the
/// sweep exists for, so the active-reference skip must not have disabled it.
#[test]
#[cfg(target_os = "macos")]
fn the_sweep_still_deletes_an_orphaned_reference() {
    let (_directory, store) = temp_store();
    let mut settings = credentials("sweep-orphan-roots");
    settings.save_for_test(&store).unwrap();
    let active = settings.cloud_credentials.as_ref().unwrap().reference();
    let orphan = CredentialReference::allocate();
    let mut legacy_journal = Journal::default();
    legacy_journal.retain(active).unwrap();
    legacy_journal.retain(orphan).unwrap();
    legacy_journal.save_named(&store, JOURNAL).unwrap();
    let legacy = Arc::new(RecordingLegacyBackend::default());
    install_legacy_backend(&store, legacy.clone());

    sweep_legacy_cloud_entries(&store);

    assert_eq!(
        *legacy.0.lock().unwrap(),
        vec![orphan],
        "the orphan and only the orphan is swept"
    );
}

/// The sweep is housekeeping, not a control. A legacy entry that refuses to
/// delete -- because macOS wants authorization for it, and the contributor
/// declined -- must leave the caller's own operation untouched.
///
/// This no longer asserts anything about startup: the sweep was moved off
/// `DaemonShared::load` precisely because a prompt there is unexplained. What
/// remains true is that a refusal is swallowed and the reference is left
/// alone, so the next contributor-initiated attempt can retry it.
#[test]
#[cfg(target_os = "macos")]
fn a_refused_legacy_delete_is_swallowed() {
    let (_directory, store) = temp_store();
    install_legacy_backend(&store, Arc::new(RefusingLegacyBackend));
    let mut legacy_journal = Journal::default();
    legacy_journal
        .retain(CredentialReference::allocate())
        .unwrap();
    legacy_journal.save_named(&store, JOURNAL).unwrap();

    sweep_legacy_cloud_entries(&store);

    assert_eq!(
        journal(&store).len(),
        1,
        "a refused delete must leave the orphaned reference alone"
    );
}

fn status_label(shared: &DaemonShared) -> String {
    crate::daemon::nearai_credential::handle_status(shared, &forget_request())
        .result
        .unwrap()["state"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn migrate_request() -> Request {
    Request {
        id: 1,
        method: "near_ai_credential_migrate".into(),
        params: serde_json::json!({}),
    }
}

/// A current Cloud backend that answers every read as a process without the
/// keychain entitlement does. Writes and deletes refuse the same way.
struct UnentitledBackend;

impl SecretBackend for UnentitledBackend {
    fn read(&self, _reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
        Err(CredentialError::Unentitled)
    }
    fn write(
        &self,
        _reference: &CredentialReference,
        _bytes: &[u8],
    ) -> Result<(), CredentialError> {
        Err(CredentialError::Unentitled)
    }
    fn delete(&self, _reference: &CredentialReference) -> Result<(), CredentialError> {
        Err(CredentialError::Unentitled)
    }
}

/// A build that cannot reach the store at all (a local ad-hoc build, or any
/// binary without the access group) is not a locked store, and restarting
/// it changes nothing. It gets its own state rather than
/// `storage_unavailable`'s "unlock and restart", and no action: Forget from
/// a build that cannot reach the store would drop the signed app's pointer
/// to a sign-in this build cannot even see.
#[test]
fn an_unentitled_build_reports_its_own_state_at_startup() {
    let (_directory, store) = temp_store();
    credentials("unentitled").save_for_test(&store).unwrap();
    install_backend(&store, Arc::new(UnentitledBackend));

    let shared = DaemonShared::load(store).unwrap();

    let label = status_label(&shared);
    assert_eq!(label, "storage_unentitled");
    let line = crate::private_inference_copy::credential_state_line(&label);
    assert_ne!(
        line,
        crate::private_inference_copy::credential_state_line("storage_unavailable")
    );
    assert!(!line.contains("restart"), "{line}");
    assert_eq!(
        crate::private_inference_copy::credential_action(&label),
        crate::private_inference_copy::CredentialAction::None
    );
}

/// A legacy-keychain entry, counted. Startup must not read it: on an
/// upgraded build that read is exactly the password prompt this work
/// removes.
#[cfg(target_os = "macos")]
#[derive(Default)]
struct CountingLegacyBackend {
    inner: crate::daemon::cloud_credential_test_support::MemoryBackend,
    reads: AtomicUsize,
    deletes: AtomicUsize,
}

#[cfg(target_os = "macos")]
impl SecretBackend for CountingLegacyBackend {
    fn read(&self, reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.read(reference)
    }
    fn write(&self, reference: &CredentialReference, bytes: &[u8]) -> Result<(), CredentialError> {
        self.inner.write(reference, bytes)
    }
    fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        self.inner.delete(reference)
    }
}

/// What an upgrade from a legacy-keychain build leaves: settings name a
/// reference, the entry behind it is in the legacy keychain, and the
/// data-protection store has nothing under it.
#[cfg(target_os = "macos")]
fn upgraded_from_legacy(
    store: &ConfigStore,
    name: &str,
    keep_legacy_entry: bool,
) -> (CredentialReference, Arc<CountingLegacyBackend>) {
    let mut settings = credentials(name);
    settings.save_for_test(store).unwrap();
    let reference = settings.cloud_credentials.as_ref().unwrap().reference();
    let current = backend(store.dir()).unwrap();
    let bytes = current.read(&reference).unwrap();
    let legacy = Arc::new(CountingLegacyBackend::default());
    if keep_legacy_entry {
        legacy.inner.write(&reference, &bytes).unwrap();
    }
    current.delete(&reference).unwrap();
    install_legacy_backend(store, legacy.clone());
    (reference, legacy)
}

/// The upgrade itself. Before this state existed the contributor saw "could
/// not be read, unlock and restart", and no number of restarts helped.
#[test]
#[cfg(target_os = "macos")]
fn an_upgraded_contributor_is_offered_the_move_not_told_to_restart() {
    let (_directory, store) = temp_store();
    let (_reference, legacy) = upgraded_from_legacy(&store, "upgrade-state", true);

    let shared = DaemonShared::load(store).unwrap();

    let label = status_label(&shared);
    assert_eq!(label, "migration_available");
    let line = crate::private_inference_copy::credential_state_line(&label);
    assert!(!line.contains("restart"), "{line}");
    assert!(!line.contains("Unlock"), "{line}");
    assert_eq!(
        crate::private_inference_copy::credential_action(&label),
        crate::private_inference_copy::CredentialAction::Migrate
    );
    assert_eq!(
        legacy.reads.load(Ordering::SeqCst),
        0,
        "startup must not read the legacy keychain"
    );
    // The refresh gate still holds: nothing may use a credential this
    // process has not read.
    assert!(shared.settings.lock().unwrap().cloud_storage_unavailable);
}

/// The contributor chose to move it. One legacy read, a copy into the
/// data-protection store under the same reference, and the running daemon
/// holds the key again -- with no sign-in. The legacy entry is copied, never
/// deleted: it is the contributor's live `sk-` key, and an older build
/// installed later still finds it there.
#[tokio::test]
#[cfg(target_os = "macos")]
async fn the_move_copies_the_legacy_entry_and_keeps_it() {
    let (_directory, store) = temp_store();
    let (reference, legacy) = upgraded_from_legacy(&store, "upgrade-move", true);
    let shared = DaemonShared::load(store).unwrap();

    let response = handle_request_async(&shared, &migrate_request()).await;

    let result = response
        .result
        .unwrap_or_else(|| panic!("migrate failed: {:?}", response.error.map(|e| e.message)));
    assert_eq!(result["migrated"], true);
    assert_eq!(result["state"], "present");
    assert_eq!(status_label(&shared), "present");
    assert_eq!(legacy.reads.load(Ordering::SeqCst), 1);
    assert_eq!(legacy.deletes.load(Ordering::SeqCst), 0);
    assert!(legacy.inner.read(&reference).is_ok(), "legacy entry kept");
    assert!(
        backend(shared.store.dir())
            .unwrap()
            .read(&reference)
            .is_ok()
    );
    {
        let memory = shared.settings.lock().unwrap();
        assert!(!memory.cloud_storage_unavailable);
        assert_eq!(
            memory.near_ai_inference.as_ref().unwrap().key,
            "sk-synthetic-recovery-upgrade-move"
        );
        assert!(memory.near_ai_session.is_some());
    }
    // And a restart reads it from the data-protection store, no legacy read.
    let restarted = DaemonShared::load(shared.store.clone()).unwrap();
    assert_eq!(status_label(&restarted), "present");
    assert_eq!(legacy.reads.load(Ordering::SeqCst), 1);
}

/// Pressing it twice is not an error, and does not read the legacy
/// keychain a second time.
#[tokio::test]
#[cfg(target_os = "macos")]
async fn a_second_move_is_a_no_op() {
    let (_directory, store) = temp_store();
    let (_reference, legacy) = upgraded_from_legacy(&store, "upgrade-twice", true);
    let shared = DaemonShared::load(store).unwrap();

    handle_request_async(&shared, &migrate_request())
        .await
        .result
        .unwrap();
    let again = handle_request_async(&shared, &migrate_request())
        .await
        .result
        .unwrap();

    assert_eq!(again["migrated"], true);
    assert_eq!(again["state"], "present");
    assert_eq!(legacy.reads.load(Ordering::SeqCst), 1);
}

/// Neither store holds the entry -- deleted by hand, or swept by a newer
/// ceremony and then downgraded and upgraded again. There is nothing to
/// keep, so the dangling reference is dropped and the contributor is
/// offered a sign-in rather than a button that can never work.
#[tokio::test]
#[cfg(target_os = "macos")]
async fn a_move_with_nothing_to_move_leaves_the_contributor_able_to_sign_in() {
    let (_directory, store) = temp_store();
    let (_reference, _legacy) = upgraded_from_legacy(&store, "upgrade-empty", false);
    let shared = DaemonShared::load(store).unwrap();
    assert_eq!(status_label(&shared), "migration_available");

    let result = handle_request_async(&shared, &migrate_request())
        .await
        .result
        .unwrap();

    assert_eq!(result["migrated"], false);
    assert_eq!(result["state"], "absent");
    assert_eq!(status_label(&shared), "absent");
    assert!(
        DaemonSettings::load(&shared.store)
            .unwrap()
            .cloud_credentials
            .is_none()
    );
}

/// A legacy read the contributor refused (they denied the keychain prompt)
/// is an error, and changes nothing: the state still offers the move.
#[tokio::test]
#[cfg(target_os = "macos")]
async fn a_refused_legacy_read_changes_nothing() {
    let (_directory, store) = temp_store();
    let (_reference, _legacy) = upgraded_from_legacy(&store, "upgrade-refused", true);
    install_legacy_backend(&store, Arc::new(RefusingLegacyReadBackend));
    let shared = DaemonShared::load(store).unwrap();

    let response = handle_request_async(&shared, &migrate_request()).await;

    assert_eq!(
        response.error.unwrap().message,
        "near_ai_credential_storage_unavailable"
    );
    assert_eq!(status_label(&shared), "migration_available");
    assert!(
        DaemonSettings::load(&shared.store)
            .unwrap()
            .cloud_credentials
            .is_some()
    );
}

#[cfg(target_os = "macos")]
struct RefusingLegacyReadBackend;

#[cfg(target_os = "macos")]
impl SecretBackend for RefusingLegacyReadBackend {
    fn read(&self, _reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
        Err(CredentialError::Unavailable)
    }
    fn write(
        &self,
        _reference: &CredentialReference,
        _bytes: &[u8],
    ) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
    fn delete(&self, _reference: &CredentialReference) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
}

/// Only the legacy store can hold a reference the data-protection store has
/// never seen, and that store exists only on macOS. Elsewhere a missing
/// entry is still a store that could not be read.
#[test]
#[cfg(not(target_os = "macos"))]
fn a_missing_entry_off_macos_is_not_offered_a_move() {
    let (_directory, store) = temp_store();
    let mut settings = credentials("missing-elsewhere");
    settings.save_for_test(&store).unwrap();
    let reference = settings.cloud_credentials.as_ref().unwrap().reference();
    backend(store.dir()).unwrap().delete(&reference).unwrap();

    let shared = DaemonShared::load(store).unwrap();

    assert_eq!(status_label(&shared), "storage_unavailable");
}
