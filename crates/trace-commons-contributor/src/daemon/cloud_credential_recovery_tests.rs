//! Real settings-write failures and running-proxy credential withdrawal.
//! Nested under IPC so tests can inspect the proxy without a production API.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::config::{ConfigStore, DAEMON_SETTINGS_FILE, tests_support::temp_store};
use crate::daemon::cloud_credential_lifecycle::native;
use crate::daemon::cloud_credential_test_support::{backend, install_backend};
use crate::daemon::credential_store::{CredentialError, CredentialReference, SecretBackend};
use crate::daemon::ipc::{DaemonShared, Request, handle_request_async};
use crate::daemon::nearai_credential::{api::MintedKey, ceremony};
use crate::daemon::private_inference::PrivateInference;
use crate::daemon::settings::{DaemonSettings, NearAiInferenceCredential, NearAiSession};

const DEADLINE: Duration = Duration::from_secs(10);
const JOURNAL: &str = "cloud-credential-cleanup.json";

struct ReadBarrier {
    entered: tokio::sync::oneshot::Sender<()>,
    release: mpsc::Receiver<()>,
}

struct FaultBackend {
    inner: Arc<dyn SecretBackend>,
    fail_delete: AtomicBool,
    reads: AtomicUsize,
    pause_read: Mutex<Option<ReadBarrier>>,
}

impl SecretBackend for FaultBackend {
    fn read(&self, reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let bytes = self.inner.read(reference)?;
        let pause = self.pause_read.lock().unwrap().take();
        if let Some(pause) = pause {
            let _ = pause.entered.send(());
            pause
                .release
                .recv_timeout(DEADLINE)
                .map_err(|_| CredentialError::Unavailable)?;
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
    assert_eq!(
        crate::daemon::nearai_credential::credential_state(&shared),
        "storage_unavailable"
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
    let forgetting = shared.clone();
    let mut forget =
        tokio::spawn(async move { handle_request_async(&forgetting, &forget_request()).await });
    let early = tokio::time::timeout(Duration::from_secs(3), &mut forget).await;
    let completed_before_release = early.is_ok();
    // Always release before assertions, including in the failing-before run.
    release.send(()).unwrap();
    let response = match early {
        Ok(response) => response.unwrap(),
        Err(_) => tokio::time::timeout(DEADLINE, forget)
            .await
            .unwrap()
            .unwrap(),
    };
    tokio::time::timeout(DEADLINE, reconcile)
        .await
        .unwrap()
        .unwrap();
    assert!(response.error.is_none());
    assert_withdrawn(&shared).await;
    shared.stop_private_inference().await;
    assert!(
        completed_before_release,
        "Forget waited for the unrelated OS read prompt"
    );
}
