// INTEGRATION: register under daemon/mod.rs as
// #[cfg(test)] mod cloud_credential_lifecycle_tests;
// Uses pub(crate) CloudCredentialLifecycle::{new,migrate,resolve,replace,cleanup},
// forget_locked and the per-directory coordination lock. Only synthetic secrets
// enter a fault-injectable memory backend; no real OS credential store is opened.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::config::{ConfigStore, DAEMON_SETTINGS_FILE, tests_support::temp_store};
use crate::daemon::cloud_credential_lifecycle::{CloudCredentialLifecycle, forget_locked};
use crate::daemon::credential_store::{CredentialError, CredentialReference, SecretBackend};
use crate::daemon::nearai_credential::session::coordination;
use crate::daemon::settings::{DaemonSettings, NearAiInferenceCredential, NearAiSession};

const JOURNAL: &str = "cloud-credential-cleanup.json";
const DEADLINE: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Default)]
enum Fault {
    #[default]
    None,
    Read,
    WriteBefore,
    WriteAfter,
    Readback,
    CorruptReadback,
}

struct Barrier {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

impl Barrier {
    fn wait(self) -> Result<(), CredentialError> {
        self.entered
            .send(())
            .map_err(|_| CredentialError::Unavailable)?;
        self.release
            .recv_timeout(DEADLINE)
            .map_err(|_| CredentialError::Unavailable)
    }
}

#[derive(Default)]
struct BackendState {
    entries: Mutex<BTreeMap<String, Vec<u8>>>,
    fault: Mutex<Fault>,
    delete_fails: AtomicBool,
    writes: AtomicUsize,
    pause_write: Mutex<Option<Barrier>>,
    pause_read: Mutex<Option<Barrier>>,
}

#[derive(Clone, Default)]
struct Memory(Arc<BackendState>);

impl Memory {
    fn keys(&self) -> Vec<String> {
        self.0.entries.lock().unwrap().keys().cloned().collect()
    }

    fn hold(&self, reading: bool) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (entered, waiting) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let slot = if reading {
            &self.0.pause_read
        } else {
            &self.0.pause_write
        };
        *slot.lock().unwrap() = Some(Barrier {
            entered,
            release: released,
        });
        (waiting, release)
    }
}

impl SecretBackend for Memory {
    fn read(&self, reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
        let fault = *self.0.fault.lock().unwrap();
        if matches!(fault, Fault::Read) {
            return Err(CredentialError::Unavailable);
        }
        let bytes = self
            .0
            .entries
            .lock()
            .unwrap()
            .get(&reference.storage_key()?)
            .cloned()
            .ok_or(CredentialError::NoEntry)?;
        // Capture bytes before the pause: deletion while the OS read is pending
        // cannot make this test pass by merely making the backend return missing.
        let barrier = self.0.pause_read.lock().unwrap().take();
        if let Some(barrier) = barrier {
            barrier.wait()?;
        }
        match fault {
            Fault::Readback => Err(CredentialError::Unavailable),
            Fault::CorruptReadback => Ok(b"synthetic-corruption".to_vec()),
            _ => Ok(bytes),
        }
    }

    fn write(&self, reference: &CredentialReference, bytes: &[u8]) -> Result<(), CredentialError> {
        self.0.writes.fetch_add(1, Ordering::SeqCst);
        let fault = *self.0.fault.lock().unwrap();
        if matches!(fault, Fault::WriteBefore) {
            return Err(CredentialError::Unavailable);
        }
        self.0
            .entries
            .lock()
            .unwrap()
            .insert(reference.storage_key()?, bytes.to_vec());
        let barrier = self.0.pause_write.lock().unwrap().take();
        if let Some(barrier) = barrier {
            barrier.wait()?;
        }
        if matches!(fault, Fault::WriteAfter) {
            return Err(CredentialError::Unavailable);
        }
        Ok(())
    }

    fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
        if self.0.delete_fails.load(Ordering::SeqCst) {
            return Err(CredentialError::Unavailable);
        }
        self.0
            .entries
            .lock()
            .unwrap()
            .remove(&reference.storage_key()?)
            .map(|_| ())
            .ok_or(CredentialError::NoEntry)
    }
}

struct Fixture {
    store: ConfigStore,
    backend: Memory,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let (directory, store) = temp_store();
        Self {
            store,
            backend: Memory::default(),
            _directory: directory,
        }
    }

    fn lifecycle(&self) -> CloudCredentialLifecycle<Memory> {
        CloudCredentialLifecycle::new(self.store.clone(), self.backend.clone())
    }

    fn legacy(&self, key: bool, session: bool) -> DaemonSettings {
        let (inference, retained) = credentials("old");
        let settings = DaemonSettings {
            near_ai_inference: key.then_some(inference),
            near_ai_session: session.then_some(retained),
            max_uploads_per_day: 7,
            private_inference: true,
            ..DaemonSettings::default()
        };
        // Deliberately reproduce the old on-disk format. The new save boundary
        // must refuse plaintext; bypassing that boundary is fixture setup only.
        self.store
            .write_daemon_file(
                DAEMON_SETTINGS_FILE,
                &serde_json::to_vec(&settings).unwrap(),
            )
            .unwrap();
        settings
    }

    fn bytes(&self) -> Vec<u8> {
        self.store
            .read_daemon_file(DAEMON_SETTINGS_FILE)
            .unwrap()
            .unwrap()
    }

    fn disk(&self) -> DaemonSettings {
        DaemonSettings::load(&self.store).unwrap()
    }

    fn forget(&self) -> bool {
        let locks = coordination(self.store.dir()).unwrap();
        let _commit = locks.commit.lock().unwrap();
        forget_locked(&self.store).unwrap()
    }

    fn assert_recoverable_entries(&self) {
        let refs = journal_keys(&self.store);
        for key in self.backend.keys() {
            assert!(
                refs.contains(&key),
                "an OS entry lost its recovery reference"
            );
        }
    }
}

fn credentials(account: &str) -> (NearAiInferenceCredential, NearAiSession) {
    let time = chrono::DateTime::from_timestamp(1_800_000_000, 123_456_789).unwrap();
    (
        NearAiInferenceCredential {
            key: format!("synthetic-inference-{account}"),
            key_id: format!("key-{account}"),
            key_prefix: "synthetic".into(),
            organization_id: format!("organization-{account}"),
            workspace_id: format!("workspace-{account}"),
            minted_at: time,
        },
        NearAiSession {
            refresh_token: format!("synthetic-refresh-{account}"),
            refresh_token_expires_at: Some(time + chrono::Duration::hours(1)),
            stored_at: time,
        },
    )
}

fn journal_keys(store: &ConfigStore) -> Vec<String> {
    let Some(bytes) = store.read_daemon_file(JOURNAL).unwrap() else {
        return Vec::new();
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let references: Vec<CredentialReference> =
        serde_json::from_value(value["references"].clone()).unwrap();
    references
        .iter()
        .map(|reference| reference.storage_key().unwrap())
        .collect()
}

fn active_key(settings: &DaemonSettings) -> String {
    settings
        .cloud_credentials
        .as_ref()
        .unwrap()
        .reference()
        .storage_key()
        .unwrap()
}

fn replace(
    lifecycle: &CloudCredentialLifecycle<Memory>,
    expected: &DaemonSettings,
    account: &str,
) -> anyhow::Result<DaemonSettings> {
    let (key, session) = credentials(account);
    lifecycle.replace(expected, Some(key), Some(session), || Ok(()), |_| Ok(()))
}

#[test]
fn migration_round_trips_each_credential_shape_without_plaintext_settings() {
    for (key, session) in [(true, false), (false, true), (true, true)] {
        let fixture = Fixture::new();
        let legacy = fixture.legacy(key, session);
        let runtime = fixture.lifecycle().migrate(legacy.clone()).unwrap();
        assert_eq!(runtime.near_ai_inference, legacy.near_ai_inference);
        assert_eq!(runtime.near_ai_session, legacy.near_ai_session);
        assert_eq!(runtime.max_uploads_per_day, 7);
        assert!(runtime.private_inference);
        let disk = fixture.disk();
        assert!(disk.near_ai_inference.is_none() && disk.near_ai_session.is_none());
        assert_eq!(disk.cloud_credentials, runtime.cloud_credentials);
        let wire = String::from_utf8(fixture.bytes()).unwrap();
        assert!(!wire.contains("synthetic-inference-old"));
        assert!(!wire.contains("synthetic-refresh-old"));
        assert_eq!(fixture.lifecycle().resolve(disk).unwrap(), runtime);
        assert_eq!(fixture.backend.keys(), vec![active_key(&runtime)]);
        fixture.assert_recoverable_entries();
        let writes = fixture.backend.0.writes.load(Ordering::SeqCst);
        assert_eq!(
            fixture.lifecycle().migrate(fixture.disk()).unwrap(),
            runtime
        );
        assert_eq!(fixture.backend.0.writes.load(Ordering::SeqCst), writes);
        runtime.save(&fixture.store).unwrap();
        assert_eq!(
            fixture.lifecycle().resolve(fixture.disk()).unwrap(),
            runtime
        );
    }
}

#[test]
fn plaintext_save_is_refused_before_the_legacy_document_is_changed() {
    let fixture = Fixture::new();
    let legacy = fixture.legacy(true, true);
    let before = fixture.bytes();
    assert!(legacy.save(&fixture.store).is_err());
    assert_eq!(fixture.bytes(), before);
    assert!(fixture.backend.keys().is_empty());
}

#[test]
fn os_failures_preserve_legacy_bytes_and_leave_a_retriable_migration() {
    for fault in [
        Fault::Read,
        Fault::WriteBefore,
        Fault::WriteAfter,
        Fault::Readback,
        Fault::CorruptReadback,
    ] {
        let fixture = Fixture::new();
        let legacy = fixture.legacy(true, true);
        let before = fixture.bytes();
        *fixture.backend.0.fault.lock().unwrap() = fault;
        assert!(fixture.lifecycle().migrate(legacy.clone()).is_err());
        assert_eq!(fixture.bytes(), before);
        assert!(fixture.backend.keys().is_empty());
        *fixture.backend.0.fault.lock().unwrap() = Fault::None;
        fixture.lifecycle().cleanup().unwrap();
        assert!(journal_keys(&fixture.store).is_empty());
        let runtime = fixture.lifecycle().migrate(fixture.disk()).unwrap();
        assert_eq!(runtime.near_ai_inference, legacy.near_ai_inference);
        assert_eq!(runtime.near_ai_session, legacy.near_ai_session);
    }
}

#[test]
fn failed_readback_and_failed_cleanup_retain_only_opaque_recovery_references() {
    let fixture = Fixture::new();
    let legacy = fixture.legacy(true, true);
    let before = fixture.bytes();
    *fixture.backend.0.fault.lock().unwrap() = Fault::CorruptReadback;
    fixture.backend.0.delete_fails.store(true, Ordering::SeqCst);
    assert!(fixture.lifecycle().migrate(legacy).is_err());
    assert_eq!(fixture.bytes(), before);
    assert_eq!(fixture.backend.keys().len(), 1);
    fixture.assert_recoverable_entries();
    let journal =
        String::from_utf8(fixture.store.read_daemon_file(JOURNAL).unwrap().unwrap()).unwrap();
    for forbidden in [
        "synthetic",
        "organization-old",
        "workspace-old",
        "key-old",
        "refresh_token",
        "inference_key",
    ] {
        assert!(!journal.contains(forbidden));
    }
    assert!(fixture.lifecycle().cleanup().is_err());
    fixture.assert_recoverable_entries();
    fixture
        .backend
        .0
        .delete_fails
        .store(false, Ordering::SeqCst);
    *fixture.backend.0.fault.lock().unwrap() = Fault::None;
    fixture.lifecycle().cleanup().unwrap();
    assert!(fixture.backend.keys().is_empty());
    assert!(journal_keys(&fixture.store).is_empty());
    fixture.lifecycle().migrate(fixture.disk()).unwrap();
}

#[test]
fn failed_old_entry_cleanup_keeps_the_new_active_connection_and_retries() {
    let fixture = Fixture::new();
    let initial = fixture
        .lifecycle()
        .migrate(fixture.legacy(true, true))
        .unwrap();
    let old_key = active_key(&initial);
    fixture.backend.0.delete_fails.store(true, Ordering::SeqCst);
    let current = replace(&fixture.lifecycle(), &initial, "new").unwrap();
    let current_key = active_key(&current);
    assert_ne!(old_key, current_key);
    assert_eq!(fixture.backend.keys().len(), 2);
    fixture.assert_recoverable_entries();
    assert_eq!(
        fixture.lifecycle().resolve(fixture.disk()).unwrap(),
        current
    );
    assert!(fixture.lifecycle().cleanup().is_err());
    fixture
        .backend
        .0
        .delete_fails
        .store(false, Ordering::SeqCst);
    fixture.lifecycle().cleanup().unwrap();
    assert_eq!(fixture.backend.keys(), vec![current_key.clone()]);
    assert_eq!(journal_keys(&fixture.store), vec![current_key]);
    assert_eq!(
        fixture.lifecycle().resolve(fixture.disk()).unwrap(),
        current
    );
}

#[test]
fn forgetting_removes_authority_before_retrying_a_failed_os_delete() {
    let fixture = Fixture::new();
    let initial = fixture
        .lifecycle()
        .migrate(fixture.legacy(true, true))
        .unwrap();
    fixture.backend.0.delete_fails.store(true, Ordering::SeqCst);
    assert!(fixture.forget());
    let forgotten = fixture.disk();
    assert!(forgotten.cloud_credentials.is_none());
    assert!(forgotten.near_ai_inference.is_none() && forgotten.near_ai_session.is_none());
    assert_eq!(forgotten.max_uploads_per_day, 7);
    assert_eq!(
        fixture.lifecycle().resolve(forgotten.clone()).unwrap(),
        forgotten
    );
    assert_eq!(journal_keys(&fixture.store), vec![active_key(&initial)]);
    assert!(fixture.lifecycle().cleanup().is_err());
    let shared = crate::daemon::ipc::DaemonShared::load(fixture.store.clone()).unwrap();
    let request = crate::daemon::ipc::Request {
        id: 1,
        method: "near_ai_credential_status".into(),
        params: serde_json::json!({}),
    };
    let status = crate::daemon::nearai_credential::handle_status(&shared, &request)
        .result
        .unwrap();
    assert_eq!(status["state"], "cleanup_required");
    assert_eq!(
        crate::daemon::nearai_credential::credential_state(&shared),
        "cleanup_required"
    );
    fixture
        .backend
        .0
        .delete_fails
        .store(false, Ordering::SeqCst);
    fixture.lifecycle().cleanup().unwrap();
    assert!(fixture.backend.keys().is_empty());
    assert!(journal_keys(&fixture.store).is_empty());
    assert!(!fixture.forget());
}

#[test]
fn stale_replacement_cannot_overwrite_the_new_account_or_publish_memory() {
    let fixture = Fixture::new();
    let initial = fixture
        .lifecycle()
        .migrate(fixture.legacy(true, true))
        .unwrap();
    let current = replace(&fixture.lifecycle(), &initial, "new").unwrap();
    let before = fixture.bytes();
    let writes = fixture.backend.0.writes.load(Ordering::SeqCst);
    let published = AtomicBool::new(false);
    let (key, session) = credentials("late");
    let result = fixture.lifecycle().replace(
        &initial,
        Some(key),
        Some(session),
        || Ok(()),
        |_| {
            published.store(true, Ordering::SeqCst);
            Ok(())
        },
    );
    assert_eq!(
        result.unwrap_err().to_string(),
        "near_ai_credential_session_changed"
    );
    assert!(!published.load(Ordering::SeqCst));
    assert_eq!(fixture.bytes(), before);
    assert_eq!(fixture.backend.0.writes.load(Ordering::SeqCst), writes);
    assert_eq!(
        fixture.lifecycle().resolve(fixture.disk()).unwrap(),
        current
    );
}

#[test]
fn forget_finishes_while_an_os_write_is_held_and_late_publication_is_refused() {
    let fixture = Fixture::new();
    let initial = fixture
        .lifecycle()
        .migrate(fixture.legacy(true, true))
        .unwrap();
    let (entered, release) = fixture.backend.hold(false);
    let accepted = AtomicBool::new(false);
    let published = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            let (key, session) = credentials("late");
            fixture.lifecycle().replace(
                &initial,
                Some(key),
                Some(session),
                || {
                    accepted.store(true, Ordering::SeqCst);
                    Ok(())
                },
                |_| {
                    published.store(true, Ordering::SeqCst);
                    Ok(())
                },
            )
        });
        entered
            .recv_timeout(DEADLINE)
            .expect("OS write reached barrier");
        fixture.assert_recoverable_entries();
        assert_eq!(fixture.disk().cloud_credentials, initial.cloud_credentials);
        let (removed, removal) = mpsc::channel();
        let remover = &fixture;
        scope.spawn(move || removed.send(remover.forget()).unwrap());
        assert!(
            removal
                .recv_timeout(DEADLINE / 2)
                .expect("forget waited on OS storage")
        );
        assert!(fixture.disk().cloud_credentials.is_none());
        release.send(()).expect("release pending OS write");
        worker.join().unwrap()
    });
    assert_eq!(
        result.unwrap_err().to_string(),
        "near_ai_credential_session_changed"
    );
    assert!(!accepted.load(Ordering::SeqCst) && !published.load(Ordering::SeqCst));
    fixture.lifecycle().cleanup().unwrap();
    assert!(fixture.backend.keys().is_empty());
    assert!(journal_keys(&fixture.store).is_empty());
    assert!(fixture.disk().cloud_credentials.is_none());
}

#[test]
fn preferences_changed_during_os_preparation_survive_verified_publication() {
    let fixture = Fixture::new();
    let initial = fixture
        .lifecycle()
        .migrate(fixture.legacy(true, true))
        .unwrap();
    let (entered, release) = fixture.backend.hold(false);
    let current = std::thread::scope(|scope| {
        let worker = scope.spawn(|| replace(&fixture.lifecycle(), &initial, "new"));
        entered
            .recv_timeout(DEADLINE)
            .expect("OS write reached barrier");
        {
            let locks = coordination(fixture.store.dir()).unwrap();
            let _commit = locks.commit.lock().unwrap();
            let mut disk = fixture.disk();
            disk.max_uploads_per_day = 19;
            disk.private_inference = false;
            disk.save_locked(&fixture.store).unwrap();
        }
        release.send(()).unwrap();
        worker.join().unwrap().unwrap()
    });
    assert_eq!(current.max_uploads_per_day, 19);
    assert!(!current.private_inference);
    assert_eq!(
        fixture.lifecycle().resolve(fixture.disk()).unwrap(),
        current
    );
}

#[test]
fn a_delayed_os_read_cannot_resolve_a_forgotten_connection() {
    let fixture = Fixture::new();
    fixture
        .lifecycle()
        .migrate(fixture.legacy(true, true))
        .unwrap();
    let snapshot = fixture.disk();
    let (entered, release) = fixture.backend.hold(true);
    let result = std::thread::scope(|scope| {
        let worker = scope.spawn(|| fixture.lifecycle().resolve(snapshot));
        entered
            .recv_timeout(DEADLINE)
            .expect("OS read captured the old bundle");
        let (removed, removal) = mpsc::channel();
        let remover = &fixture;
        scope.spawn(move || removed.send(remover.forget()).unwrap());
        assert!(
            removal
                .recv_timeout(DEADLINE / 2)
                .expect("forget waited on OS read")
        );
        fixture.lifecycle().cleanup().unwrap();
        assert!(fixture.backend.keys().is_empty());
        release.send(()).unwrap();
        worker.join().unwrap()
    });
    assert_eq!(
        result.unwrap_err().to_string(),
        "near_ai_credential_session_changed"
    );
    assert!(fixture.disk().cloud_credentials.is_none());
}

#[test]
fn an_attempt_refusal_withholds_publication_and_preserves_recovery_references() {
    let fixture = Fixture::new();
    let initial = fixture
        .lifecycle()
        .migrate(fixture.legacy(true, true))
        .unwrap();
    let before = fixture.bytes();
    let published = AtomicBool::new(false);
    let (key, session) = credentials("cancelled");
    let result = fixture.lifecycle().replace(
        &initial,
        Some(key),
        Some(session),
        || Err(anyhow::anyhow!("near_ai_credential_cancelled")),
        |_| {
            published.store(true, Ordering::SeqCst);
            Ok(())
        },
    );
    assert_eq!(
        result.unwrap_err().to_string(),
        "near_ai_credential_cancelled"
    );
    assert!(!published.load(Ordering::SeqCst));
    assert_eq!(fixture.bytes(), before);
    fixture.assert_recoverable_entries();
    fixture.lifecycle().cleanup().unwrap();
    assert_eq!(fixture.backend.keys(), vec![active_key(&initial)]);
    assert_eq!(
        fixture.lifecycle().resolve(fixture.disk()).unwrap(),
        initial
    );
}

#[test]
fn tampered_metadata_and_mixed_plaintext_never_resolve_a_partial_connection() {
    let fixture = Fixture::new();
    let initial = fixture
        .lifecycle()
        .migrate(fixture.legacy(true, true))
        .unwrap();
    let saved = fixture.bytes();
    for (section, field, value) in [
        (
            "inference",
            "organization_id",
            serde_json::json!("another-organization"),
        ),
        (
            "inference",
            "workspace_id",
            serde_json::json!("another-workspace"),
        ),
        ("inference", "key_id", serde_json::json!("another-key")),
        (
            "session",
            "stored_at",
            serde_json::json!("2030-01-01T00:00:00Z"),
        ),
    ] {
        let mut changed: serde_json::Value = serde_json::from_slice(&saved).unwrap();
        changed["cloud_credentials"][section][field] = value;
        fixture
            .store
            .write_daemon_file(DAEMON_SETTINGS_FILE, &serde_json::to_vec(&changed).unwrap())
            .unwrap();
        assert_eq!(
            fixture
                .lifecycle()
                .resolve(fixture.disk())
                .unwrap_err()
                .to_string(),
            "near_ai_credential_storage_binding_mismatch"
        );
    }
    let mut mixed: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    mixed["near_ai_session"] =
        serde_json::to_value(initial.near_ai_session.as_ref().unwrap()).unwrap();
    fixture
        .store
        .write_daemon_file(DAEMON_SETTINGS_FILE, &serde_json::to_vec(&mixed).unwrap())
        .unwrap();
    assert_eq!(
        fixture
            .lifecycle()
            .resolve(fixture.disk())
            .unwrap_err()
            .to_string(),
        "near_ai_credential_storage_mixed_record"
    );
    fixture
        .store
        .write_daemon_file(DAEMON_SETTINGS_FILE, &saved)
        .unwrap();
    assert_eq!(
        fixture.lifecycle().resolve(fixture.disk()).unwrap(),
        initial
    );
}

#[test]
fn an_error_after_settings_publication_keeps_both_entries_until_explicit_recovery() {
    let fixture = Fixture::new();
    let initial = fixture
        .lifecycle()
        .migrate(fixture.legacy(true, true))
        .unwrap();
    let old_key = active_key(&initial);
    let (key, session) = credentials("new");
    let result = fixture.lifecycle().replace(
        &initial,
        Some(key.clone()),
        Some(session.clone()),
        || Ok(()),
        |_| Err(anyhow::anyhow!("synthetic-runtime-publication-failed")),
    );
    assert!(result.is_err());
    let disk = fixture.disk();
    let current_key = active_key(&disk);
    assert_ne!(current_key, old_key);
    assert_eq!(fixture.backend.keys().len(), 2);
    fixture.assert_recoverable_entries();
    let runtime = fixture.lifecycle().resolve(disk).unwrap();
    assert_eq!(runtime.near_ai_inference, Some(key));
    assert_eq!(runtime.near_ai_session, Some(session));
    fixture.lifecycle().cleanup().unwrap();
    assert_eq!(fixture.backend.keys(), vec![current_key.clone()]);
    assert_eq!(journal_keys(&fixture.store), vec![current_key]);
}

#[test]
fn missing_os_entry_starts_without_authority_and_offers_explicit_removal() {
    let (_directory, store) = temp_store();
    let (inference, session) = credentials("missing");
    let (metadata, _) =
        crate::daemon::stored_cloud_credentials::StoredCloudCredentials::from_secrets(
            CredentialReference::allocate(),
            Some(&inference),
            Some(&session),
        )
        .unwrap();
    let settings = DaemonSettings {
        cloud_credentials: Some(metadata),
        ..Default::default()
    };
    // Install an intentionally missing OS reference as an upgrade fixture.
    store
        .write_daemon_file(
            DAEMON_SETTINGS_FILE,
            &serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
    let shared = crate::daemon::ipc::DaemonShared::load(store).unwrap();
    let memory = shared.settings.lock().unwrap();
    assert!(memory.cloud_storage_unavailable);
    assert!(memory.near_ai_inference.is_none() && memory.near_ai_session.is_none());
    drop(memory);
    let request = crate::daemon::ipc::Request {
        id: 1,
        method: "near_ai_credential_status".into(),
        params: serde_json::json!({}),
    };
    let status = crate::daemon::nearai_credential::handle_status(&shared, &request)
        .result
        .unwrap();
    assert_eq!(status["state"], "storage_unavailable");
    assert_eq!(status["session_state"], "storage_unavailable");
    assert_eq!(
        crate::private_inference_copy::credential_action("storage_unavailable"),
        crate::private_inference_copy::CredentialAction::Forget
    );
    assert!(
        crate::private_inference_copy::credential_state_line("storage_unavailable")
            .contains("Unlock")
    );
    let request = crate::daemon::ipc::Request {
        method: "get_settings".into(),
        ..request
    };
    let visible = crate::daemon::ipc::handle_request(&shared, &request)
        .result
        .unwrap();
    assert!(visible.get("cloud_credentials").is_none());
    assert!(visible.get("near_ai_session").is_none());
    assert!(visible.get("near_ai_inference").is_none());
    assert!(
        !serde_json::to_string(&visible)
            .unwrap()
            .contains("binding_digest")
    );
}

#[test]
fn stale_preferences_cannot_restore_a_forgotten_cloud_connection() {
    let fixture = Fixture::new();
    let mut stale = fixture
        .lifecycle()
        .migrate(fixture.legacy(true, true))
        .unwrap();
    assert!(fixture.forget());
    stale.max_uploads_per_day = 23;
    assert!(stale.save(&fixture.store).is_err());
    assert!(fixture.disk().cloud_credentials.is_none());
    assert_eq!(fixture.disk().max_uploads_per_day, 7);
}

#[test]
fn runtime_snapshot_requires_available_storage_and_the_current_binding() {
    let (_directory, store) = temp_store();
    let (key, session) = credentials("snapshot");
    DaemonSettings {
        near_ai_inference: Some(key.clone()),
        near_ai_session: Some(session.clone()),
        ..Default::default()
    }
    .save_for_test(&store)
    .unwrap();
    let shared = crate::daemon::ipc::DaemonShared::load(store).unwrap();
    let locks = coordination(shared.store.dir()).unwrap();
    let commit = locks.commit.lock().unwrap();
    let snapshot =
        crate::daemon::nearai_credential::session::runtime_snapshot(&shared, &commit).unwrap();
    assert_eq!(snapshot.near_ai_inference, Some(key));
    assert_eq!(snapshot.near_ai_session, Some(session));
    shared.settings.lock().unwrap().cloud_storage_unavailable = true;
    assert!(crate::daemon::nearai_credential::session::runtime_snapshot(&shared, &commit).is_err());
    {
        let mut memory = shared.settings.lock().unwrap();
        memory.cloud_storage_unavailable = false;
        memory.cloud_credentials = None;
    }
    assert!(crate::daemon::nearai_credential::session::runtime_snapshot(&shared, &commit).is_err());
}

#[test]
fn credential_lock_probe_child() {
    let Some(path) = std::env::var_os("TC_SYNTHETIC_CREDENTIAL_LOCK_PROBE") else {
        return;
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    assert!(matches!(
        file.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
}

#[test]
fn credential_locks_exclude_a_second_process() {
    let (_directory, store) = temp_store();
    let locks = coordination(store.dir()).unwrap();
    for (lock, name) in [
        (&locks.commit, ".cloud-credential-commit.lock"),
        (&locks.storage, ".cloud-credential-storage.lock"),
    ] {
        let guard = lock.lock().unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "daemon::cloud_credential_lifecycle_tests::credential_lock_probe_child",
            ])
            .env(
                "TC_SYNTHETIC_CREDENTIAL_LOCK_PROBE",
                store.daemon_path(name),
            )
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{}",
            String::from_utf8_lossy(&child.stdout)
        );
        drop(guard);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(store.daemon_path(name))
            .unwrap();
        file.try_lock().unwrap();
    }
}
