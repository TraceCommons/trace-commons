//! Commons secrets use the same immutable OS entries, verified writes, cleanup
//! journal and storage/commit locks as Cloud credentials, with distinct bindings.
//! These synchronous functions belong on blocking workers when called from async code.
use anyhow::{Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;

use super::cloud_credential_lifecycle::{Journal, sync_directory};
use super::credential_store::{CredentialReference, CredentialStore, SecretBackend};
use super::nearai_credential::session::coordination;
use crate::config::{ACCOUNT_SESSION_FILE, ConfigStore, ContributorConfig, write_atomic_0600};

const JOURNAL: &str = "commons-credential-cleanup.json";
const EPOCH: &str = "commons-credential-generation.json";

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Device,
    Account,
}
impl Kind {
    fn file(self, store: &ConfigStore) -> std::path::PathBuf {
        match self {
            Self::Device => store.device_key_path(),
            Self::Account => store.daemon_path(ACCOUNT_SESSION_FILE),
        }
    }
    fn domain(self) -> &'static str {
        match self {
            Self::Device => "trace-commons/device-key/v1",
            Self::Account => "trace-commons/account-session/v1",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    commons_credential_version: u8,
    kind: Kind,
    reference: CredentialReference,
    authority: Option<String>,
}

// Separate from Cloud's SecretBundle: a Commons key cannot become an inference key.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bundle {
    version: u8,
    binding: String,
    payload: String,
}

// Secret Service's synchronous API drives its own runtime. Keep the entire
// native handle lifetime outside a caller's Tokio runtime, including callers
// of the pre-existing synchronous ConfigStore and DeviceIdentity APIs.
fn needs_native_thread() -> bool {
    cfg!(target_os = "linux") && tokio::runtime::Handle::try_current().is_ok()
}

fn unavailable() -> anyhow::Error {
    anyhow!("commons_credential_storage_unavailable")
}
fn changed() -> anyhow::Error {
    anyhow!("commons_credential_state_changed")
}
fn read(path: &std::path::Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) if bytes.len() <= 16_384 => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        _ => Err(unavailable()),
    }
}
fn write(store: &ConfigStore, path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    write_atomic_0600(store.dir(), path, bytes).map_err(|_| unavailable())?;
    sync_directory(store).map_err(|_| unavailable())
}
fn record(bytes: &[u8]) -> Result<Option<Record>> {
    // DER cannot start with a JSON object. A malformed reference must never be
    // treated as a missing key, which would silently generate a new identity.
    if !bytes.starts_with(b"{") {
        return Ok(None);
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| unavailable())?;
    if value.get("commons_credential_version").is_none() {
        return Ok(None);
    }
    let record: Record = serde_json::from_value(value).map_err(|_| unavailable())?;
    if record.commons_credential_version != 1 {
        return Err(unavailable());
    }
    Ok(Some(record))
}
fn authority(cfg: &ContributorConfig) -> String {
    let bytes = serde_json::to_vec(&(
        &cfg.ingest_url,
        &cfg.issuer_url,
        &cfg.tenant_id,
        &cfg.instance_id,
        &cfg.user_subject,
        &cfg.audience,
        &cfg.device_key_id,
    ))
    .expect("strings serialize");
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}
fn current_authority(store: &ConfigStore) -> Result<Option<String>> {
    Ok(store.load_config()?.as_ref().map(authority))
}
fn binding(store: &ConfigStore, record: &Record) -> Result<String> {
    let dir = std::fs::canonicalize(store.dir()).map_err(|_| unavailable())?;
    let bytes = serde_json::to_vec(&(
        record.kind.domain(),
        dir,
        &record.authority,
        record.reference,
    ))
    .map_err(|_| unavailable())?;
    Ok(format!("sha256:{}", hex::encode(Sha256::digest(bytes))))
}
fn native(_store: &ConfigStore) -> Result<CredentialStore<Arc<dyn SecretBackend>>> {
    #[cfg(test)]
    let backend = test_backend(_store);
    #[cfg(not(test))]
    let backend: Arc<dyn SecretBackend> =
        Arc::new(super::os_secret_store::OsSecretBackend::commons().map_err(|_| unavailable())?);
    Ok(CredentialStore::new(backend))
}

/// Capture before a browser/network operation. Both sign-out and wipe advance
/// this durable generation, so even an absent -> absent change invalidates it.
#[derive(Clone)]
pub(crate) struct Snapshot {
    generation: Option<Vec<u8>>,
    config: Option<Vec<u8>>,
    previous: Option<Vec<u8>>,
    kind: Kind,
}
pub(crate) fn snapshot(store: &ConfigStore, kind: Kind) -> Result<Snapshot> {
    let locks = coordination(store.dir())?;
    let _commit = locks.commit.lock()?;
    snapshot_locked(store, kind)
}
pub(crate) fn account_snapshot(store: &ConfigStore, cfg: &ContributorConfig) -> Result<Snapshot> {
    let locks = coordination(store.dir())?;
    let _commit = locks.commit.lock()?;
    let expected = snapshot_locked(store, Kind::Account)?;
    if current_authority(store)? != Some(authority(cfg)) {
        return Err(changed());
    }
    Ok(expected)
}

fn snapshot_locked(store: &ConfigStore, kind: Kind) -> Result<Snapshot> {
    Ok(Snapshot {
        generation: read(&store.daemon_path(EPOCH))?,
        config: read(&store.daemon_path("contributor.json"))?,
        previous: read(&kind.file(store))?,
        kind,
    })
}
fn ensure_current(store: &ConfigStore, expected: &Snapshot) -> Result<()> {
    let current = snapshot_locked(store, expected.kind)?;
    if current.generation != expected.generation
        || current.config != expected.config
        || current.previous != expected.previous
    {
        return Err(changed());
    }
    Ok(())
}

pub(crate) fn load(store: &ConfigStore, kind: Kind) -> Result<Option<Vec<u8>>> {
    Ok(load_with_snapshot(store, kind)?.map(|(payload, _)| payload))
}

pub(crate) fn load_with_snapshot(
    store: &ConfigStore,
    kind: Kind,
) -> Result<Option<(Vec<u8>, Snapshot)>> {
    if needs_native_thread() {
        return std::thread::scope(|scope| {
            scope
                .spawn(|| load_with_snapshot(store, kind))
                .join()
                .map_err(|_| unavailable())?
        });
    }
    let expected = snapshot(store, kind)?;
    let Some(bytes) = &expected.previous else {
        return Ok(None);
    };
    let Some(record) = record(bytes)? else {
        // Migration keeps the original file intact until an immutable OS entry
        // has passed readback. The same filename then becomes a rollback guard.
        replace(store, &expected, bytes, None)?;
        return load_with_snapshot(store, kind);
    };
    if record.kind != kind
        || (kind == Kind::Account && record.authority != current_authority(store)?)
    {
        return Err(changed());
    }
    let credentials = native(store)?;
    let stored = credentials
        .load_bytes(&record.reference)
        .map_err(|_| unavailable())?;
    let bundle: Bundle = serde_json::from_slice(&stored).map_err(|_| unavailable())?;
    if bundle.version != 1 || bundle.binding != binding(store, &record)? {
        return Err(unavailable());
    }
    let payload = STANDARD.decode(bundle.payload).map_err(|_| unavailable())?;
    let locks = coordination(store.dir())?;
    let _commit = locks.commit.lock()?;
    ensure_current(store, &expected)?;
    Ok(Some((payload, expected)))
}

/// Publish a secret, optionally publishing its first enrollment config in the
/// same commit section. No config is installed before the OS write verifies.
pub(crate) fn replace(
    store: &ConfigStore,
    expected: &Snapshot,
    payload: &[u8],
    enrollment: Option<&ContributorConfig>,
) -> Result<()> {
    if needs_native_thread() {
        return std::thread::scope(|scope| {
            scope
                .spawn(|| replace(store, expected, payload, enrollment))
                .join()
                .map_err(|_| unavailable())?
        });
    }
    let locks = coordination(store.dir())?;
    let _storage = locks.storage.lock()?;
    let credentials = native(store)?;
    cleanup_locked(store, &credentials)?;
    let record = Record {
        commons_credential_version: 1,
        kind: expected.kind,
        reference: CredentialReference::allocate(),
        authority: if expected.kind == Kind::Account {
            enrollment.map(authority).or(current_authority(store)?)
        } else {
            None
        },
    };
    let bundle = Bundle {
        version: 1,
        binding: binding(store, &record)?,
        payload: STANDARD.encode(payload),
    };
    let encoded = serde_json::to_vec(&bundle).map_err(|_| unavailable())?;
    {
        let _commit = locks.commit.lock()?;
        ensure_current(store, expected)?;
        if enrollment.is_some() && expected.config.is_some() {
            return Err(changed());
        }
        let mut journal = Journal::read_named(store, JOURNAL)?;
        journal.retain(record.reference)?;
        if let Some(previous) = &expected.previous {
            if let Some(old) = self::record(previous)? {
                journal.retain(old.reference)?;
            }
        }
        journal.save_named(store, JOURNAL)?;
    }
    credentials
        .prepare_bytes_at(&record.reference, &encoded)
        .map_err(|_| unavailable())?;
    {
        let _commit = locks.commit.lock()?;
        ensure_current(store, expected)?;
        write(
            store,
            &expected.kind.file(store),
            &serde_json::to_vec(&record).map_err(|_| unavailable())?,
        )?;
        if let Some(config) = enrollment {
            // Use a create-only link to preserve concurrent invite enrollment.
            let temporary =
                store.daemon_path(&format!(".commons-enrollment-{}", uuid::Uuid::new_v4()));
            write(store, &temporary, &serde_json::to_vec(config)?)?;
            let published = std::fs::hard_link(&temporary, store.daemon_path("contributor.json"));
            let _ = std::fs::remove_file(temporary);
            if published.is_err() {
                let _ = std::fs::remove_file(expected.kind.file(store));
                return Err(changed());
            }
        }
    }
    sync_directory(store).map_err(|_| unavailable())?;
    // Publication succeeded. A retired entry that cannot yet be deleted stays
    // journaled; do not misreport the newly installed session as a failed login.
    let _ = cleanup_locked(store, &credentials);
    Ok(())
}

/// Persist a server-issued account-token rotation without overwriting a
/// sign-out or configuration change. A concurrent token rotation wins; its
/// immutable credential entry is already newer than this request's snapshot.
pub(crate) fn replace_account_rotation(
    store: &ConfigStore,
    expected: &Snapshot,
    payload: &[u8],
) -> Result<()> {
    match replace(store, expected, payload, None) {
        Ok(()) => Ok(()),
        Err(_) => {
            let current = snapshot(store, Kind::Account)?;
            if current.generation == expected.generation
                && current.config == expected.config
                && current.previous != expected.previous
            {
                Ok(())
            } else {
                Err(changed())
            }
        }
    }
}

/// Local invalidation never waits for the OS store. Cleanup is journaled for
/// a later blocking operation, including when the store is locked or offline.
pub(crate) fn clear(store: &ConfigStore, kinds: &[Kind]) -> Result<()> {
    clear_with(store, kinds, || Ok(()))
}
pub(crate) fn clear_with<T>(
    store: &ConfigStore,
    kinds: &[Kind],
    after: impl FnOnce() -> Result<T>,
) -> Result<T> {
    clear_checked(store, kinds, None, after)
}
pub(crate) fn clear_expected(store: &ConfigStore, expected: &Snapshot) -> Result<()> {
    clear_checked(store, &[expected.kind], Some(expected), || Ok(()))
}
fn clear_checked<T>(
    store: &ConfigStore,
    kinds: &[Kind],
    expected: Option<&Snapshot>,
    after: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let locks = coordination(store.dir())?;
    let _commit = locks.commit.lock()?;
    if let Some(expected) = expected {
        ensure_current(store, expected)?;
    }
    let mut journal = Journal::read_named(store, JOURNAL)?;
    for kind in kinds {
        if let Some(bytes) = read(&kind.file(store))? {
            if let Some(record) = record(&bytes)? {
                journal.retain(record.reference)?;
            }
        }
    }
    journal.save_named(store, JOURNAL)?;
    write(
        store,
        &store.daemon_path(EPOCH),
        uuid::Uuid::new_v4().to_string().as_bytes(),
    )?;
    for kind in kinds {
        match std::fs::remove_file(kind.file(store)) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(unavailable()),
        }
    }
    let result = after()?;
    sync_directory(store).map_err(|_| unavailable())?;
    Ok(result)
}
fn cleanup_locked<B: SecretBackend>(
    store: &ConfigStore,
    credentials: &CredentialStore<B>,
) -> Result<()> {
    let locks = coordination(store.dir())?;
    let active = {
        let _commit = locks.commit.lock()?;
        let mut active = Vec::new();
        for kind in [Kind::Device, Kind::Account] {
            if let Some(bytes) = read(&kind.file(store))? {
                if let Some(record) = record(&bytes)? {
                    active.push(record.reference);
                }
            }
        }
        active
    };
    let pending = Journal::read_named(store, JOURNAL)?.references;
    for reference in pending {
        if active.contains(&reference) {
            continue;
        }
        credentials.delete(&reference).map_err(|_| unavailable())?;
        let _commit = locks.commit.lock()?;
        let mut journal = Journal::read_named(store, JOURNAL)?;
        journal.references.retain(|value| *value != reference);
        journal.save_named(store, JOURNAL)?;
    }
    Ok(())
}
pub(crate) fn cleanup(store: &ConfigStore) -> Result<()> {
    if needs_native_thread() {
        return std::thread::scope(|scope| {
            scope
                .spawn(|| cleanup(store))
                .join()
                .map_err(|_| unavailable())?
        });
    }
    if Journal::read_named(store, JOURNAL)?.references.is_empty() {
        return Ok(());
    }
    let locks = coordination(store.dir())?;
    let _storage = locks.storage.lock()?;
    cleanup_locked(store, &native(store)?)
}

#[cfg(test)]
fn test_registry()
-> &'static std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, Arc<dyn SecretBackend>>>
{
    static REGISTRY: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, Arc<dyn SecretBackend>>>,
    > = std::sync::OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}
#[cfg(test)]
fn test_backend(store: &ConfigStore) -> Arc<dyn SecretBackend> {
    test_registry()
        .lock()
        .unwrap()
        .entry(store.dir().to_path_buf())
        .or_insert_with(|| Arc::new(super::cloud_credential_test_support::MemoryBackend::default()))
        .clone()
}

#[cfg(test)]
pub(crate) fn install_test_backend(store: &ConfigStore, backend: Arc<dyn SecretBackend>) {
    test_registry()
        .lock()
        .unwrap()
        .insert(store.dir().to_path_buf(), backend);
}

/// Resolve only the token removed by sign-out, for best-effort server revocation.
/// It is never published or returned as a currently authorized account session.
pub(crate) fn removed_payload(store: &ConfigStore, expected: &Snapshot) -> Result<Option<Vec<u8>>> {
    let Some(bytes) = &expected.previous else {
        return Ok(None);
    };
    let Some(record) = record(bytes)? else {
        return Ok(Some(bytes.clone()));
    };
    if record.kind != Kind::Account {
        return Err(unavailable());
    }
    let cfg = expected
        .config
        .as_ref()
        .map(|bytes| serde_json::from_slice::<ContributorConfig>(bytes))
        .transpose()
        .map_err(|_| unavailable())?;
    if record.authority != cfg.as_ref().map(authority) {
        return Err(changed());
    }
    let bytes = native(store)?
        .load_bytes(&record.reference)
        .map_err(|_| unavailable())?;
    let bundle: Bundle = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
    if bundle.version != 1 || bundle.binding != binding(store, &record)? {
        return Err(unavailable());
    }
    Ok(Some(
        STANDARD.decode(bundle.payload).map_err(|_| unavailable())?,
    ))
}

/// Enrollment changes revoke the previous account session, including an A -> B
/// -> A switch. Preference-only edits preserve the current session.
pub(crate) fn save_config(store: &ConfigStore, cfg: &ContributorConfig, body: &[u8]) -> Result<()> {
    let expected = snapshot(store, Kind::Account)?;
    let before = expected
        .config
        .as_ref()
        .map(|bytes| serde_json::from_slice::<ContributorConfig>(bytes))
        .transpose()
        .map_err(|_| unavailable())?;
    if before.as_ref().map(authority) != Some(authority(cfg)) {
        clear_checked(store, &[Kind::Account], Some(&expected), || {
            write(store, &store.daemon_path("contributor.json"), body)
        })
    } else {
        let locks = coordination(store.dir())?;
        let _commit = locks.commit.lock()?;
        ensure_current(store, &expected)?;
        write(store, &store.daemon_path("contributor.json"), body)
    }
}

/// Cancel an in-flight enrollment without touching any existing credential.
pub(crate) fn invalidate(store: &ConfigStore) -> Result<()> {
    let locks = coordination(store.dir())?;
    let _commit = locks.commit.lock()?;
    write(
        store,
        &store.daemon_path(EPOCH),
        uuid::Uuid::new_v4().to_string().as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::super::credential_store::CredentialError;
    use super::*;
    use crate::identity::DeviceIdentity;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    #[derive(Default)]
    struct Backend {
        values: Mutex<std::collections::HashMap<String, Vec<u8>>>,
        unavailable: AtomicBool,
        corrupt: AtomicBool,
        refuse_delete: AtomicBool,
        pause_read: Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
        pause_write: Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
    }
    impl SecretBackend for Backend {
        fn read(
            &self,
            reference: &CredentialReference,
        ) -> std::result::Result<Vec<u8>, CredentialError> {
            if let Some((started, resume)) = self.pause_read.lock().unwrap().take() {
                started.send(()).unwrap();
                resume.recv().unwrap();
            }
            if self.unavailable.load(Ordering::SeqCst) {
                return Err(CredentialError::Unavailable);
            }
            self.values
                .lock()
                .unwrap()
                .get(&reference.storage_key()?)
                .cloned()
                .ok_or(CredentialError::NoEntry)
        }
        fn write(
            &self,
            reference: &CredentialReference,
            bytes: &[u8],
        ) -> std::result::Result<(), CredentialError> {
            if let Some((started, resume)) = self.pause_write.lock().unwrap().take() {
                started.send(()).unwrap();
                resume.recv().unwrap();
            }
            let bytes = if self.corrupt.load(Ordering::SeqCst) {
                b"corrupt".to_vec()
            } else {
                bytes.to_vec()
            };
            self.values
                .lock()
                .unwrap()
                .insert(reference.storage_key()?, bytes);
            Ok(())
        }
        fn delete(
            &self,
            reference: &CredentialReference,
        ) -> std::result::Result<(), CredentialError> {
            if self.refuse_delete.load(Ordering::SeqCst) {
                return Err(CredentialError::Unavailable);
            }
            self.values
                .lock()
                .unwrap()
                .remove(&reference.storage_key()?);
            Ok(())
        }
    }
    fn fixture() -> (tempfile::TempDir, ConfigStore, Arc<Backend>) {
        let (dir, store) = crate::config::tests_support::temp_store();
        let backend = Arc::new(Backend::default());
        test_registry()
            .lock()
            .unwrap()
            .insert(store.dir().to_path_buf(), backend.clone());
        (dir, store, backend)
    }
    #[tokio::test]
    async fn synchronous_credential_api_is_safe_inside_an_async_runtime() {
        let (_dir, store, _) = fixture();
        let identity = DeviceIdentity::load_or_generate(&store).unwrap();
        assert_eq!(
            DeviceIdentity::load(&store).unwrap().unwrap().device_key_id,
            identity.device_key_id
        );
        store
            .write_daemon_file(ACCOUNT_SESSION_FILE, &session())
            .unwrap();
        assert!(
            crate::account_auth::try_load_token(&store)
                .unwrap()
                .is_some()
        );
        clear(&store, &[Kind::Account, Kind::Device]).unwrap();
        cleanup(&store).unwrap();
    }

    fn config() -> ContributorConfig {
        serde_json::from_value(serde_json::json!({
            "schema_version": crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,
            "issuer_url":"https://issuer.example", "ingest_url":"https://ingest.example",
            "audience":"trace-commons-upload", "tenant_id":"tenant-one", "instance_id":"instance-one",
            "user_subject":"subject-one", "device_key_id":"sha256:00", "consent_scopes":[]
        })).unwrap()
    }
    #[test]
    fn account_switch_rejects_old_sessions_and_stale_signin_parameters() {
        let (_dir, store, _) = fixture();
        let first = config();
        store.save_config(&first).unwrap();
        store
            .write_daemon_file(ACCOUNT_SESSION_FILE, &session())
            .unwrap();
        let pending = account_snapshot(&store, &first).unwrap();
        let mut second = first.clone();
        second.tenant_id = "tenant-two".into();
        store.save_config(&second).unwrap();
        assert!(
            crate::account_auth::try_load_token(&store)
                .unwrap()
                .is_none()
        );
        store.save_config(&first).unwrap();
        assert!(
            crate::account_auth::try_load_token(&store)
                .unwrap()
                .is_none()
        );
        store.save_config(&second).unwrap();
        assert!(account_snapshot(&store, &first).is_err());
        assert!(replace(&store, &pending, &session(), None).is_err());
    }
    #[test]
    fn failed_retired_entry_cleanup_does_not_turn_publication_into_failure() {
        let (_dir, store, backend) = fixture();
        store
            .write_daemon_file(ACCOUNT_SESSION_FILE, &session())
            .unwrap();
        backend.refuse_delete.store(true, Ordering::SeqCst);
        store
            .write_daemon_file(ACCOUNT_SESSION_FILE, &session())
            .unwrap();
        assert!(
            crate::account_auth::try_load_token(&store)
                .unwrap()
                .is_some()
        );
        assert_eq!(backend.values.lock().unwrap().len(), 2);
        backend.refuse_delete.store(false, Ordering::SeqCst);
        cleanup(&store).unwrap();
        assert_eq!(backend.values.lock().unwrap().len(), 1);
    }

    #[test]
    fn stale_signout_cannot_remove_a_newer_session() {
        let (_dir, store, _) = fixture();
        store
            .write_daemon_file(ACCOUNT_SESSION_FILE, &session())
            .unwrap();
        let stale = snapshot(&store, Kind::Account).unwrap();
        store
            .write_daemon_file(ACCOUNT_SESSION_FILE, &session())
            .unwrap();
        assert!(clear_expected(&store, &stale).is_err());
        assert!(
            crate::account_auth::try_load_token(&store)
                .unwrap()
                .is_some()
        );
    }

    fn session() -> Vec<u8> {
        serde_json::to_vec(&crate::account_auth::AccountSession {
            access_token: "tcn1_sensitive_fixture".into(),
            account_id: "fixture-account".into(),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        })
        .unwrap()
    }
    #[test]
    fn legacy_migration_preserves_device_identity_and_removes_plaintext() {
        let (_dir, store, _) = fixture();
        let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap();
        write(&store, &store.device_key_path(), doc.as_ref()).unwrap();
        write(&store, &Kind::Account.file(&store), &session()).unwrap();
        let identity = DeviceIdentity::load_or_generate(&store).unwrap();
        assert_eq!(store.load_device_key().unwrap().unwrap(), doc.as_ref());
        assert_eq!(
            DeviceIdentity::load_or_generate(&store)
                .unwrap()
                .device_key_id,
            identity.device_key_id
        );
        assert_eq!(
            crate::account_auth::load_token(&store).as_deref(),
            Some("tcn1_sensitive_fixture")
        );
        let device = std::fs::read(store.device_key_path()).unwrap();
        let account = std::fs::read(Kind::Account.file(&store)).unwrap();
        assert!(record(&device).unwrap().is_some());
        assert!(
            ring::signature::Ed25519KeyPair::from_pkcs8(&device).is_err(),
            "older clients must refuse, never regenerate"
        );
        assert!(serde_json::from_slice::<crate::account_auth::AccountSession>(&account).is_err());
        for path in std::fs::read_dir(store.dir()).unwrap() {
            let bytes = std::fs::read(path.unwrap().path()).unwrap();
            assert!(
                !bytes
                    .windows(doc.as_ref().len())
                    .any(|window| window == doc.as_ref())
            );
            assert!(!String::from_utf8_lossy(&bytes).contains("tcn1_sensitive_fixture"));
        }
    }
    #[test]
    fn failed_readback_leaves_legacy_bytes_and_cleanup_is_recoverable() {
        let (_dir, store, backend) = fixture();
        let original = session();
        write(&store, &Kind::Account.file(&store), &original).unwrap();
        backend.corrupt.store(true, Ordering::SeqCst);
        backend.refuse_delete.store(true, Ordering::SeqCst);
        assert!(load(&store, Kind::Account).is_err());
        assert_eq!(std::fs::read(Kind::Account.file(&store)).unwrap(), original);
        assert_eq!(
            Journal::read_named(&store, JOURNAL)
                .unwrap()
                .references
                .len(),
            1
        );
        backend.corrupt.store(false, Ordering::SeqCst);
        backend.refuse_delete.store(false, Ordering::SeqCst);
        cleanup(&store).unwrap();
        assert!(backend.values.lock().unwrap().is_empty());
        assert!(load(&store, Kind::Account).unwrap().is_some());
    }
    #[test]
    fn unavailable_or_missing_native_key_never_generates_a_replacement_identity() {
        let (_dir, store, backend) = fixture();
        let identity = DeviceIdentity::load_or_generate(&store).unwrap();
        let before = std::fs::read(store.device_key_path()).unwrap();
        backend.unavailable.store(true, Ordering::SeqCst);
        assert!(DeviceIdentity::load_or_generate(&store).is_err());
        assert_eq!(std::fs::read(store.device_key_path()).unwrap(), before);
        backend.unavailable.store(false, Ordering::SeqCst);
        assert_eq!(
            DeviceIdentity::load_or_generate(&store)
                .unwrap()
                .device_key_id,
            identity.device_key_id
        );
        backend.values.lock().unwrap().clear();
        assert!(DeviceIdentity::load_or_generate(&store).is_err());
        assert_eq!(std::fs::read(store.device_key_path()).unwrap(), before);
    }
    #[test]
    fn copied_directory_and_swapped_credential_kinds_fail_closed() {
        let (_dir, store, backend) = fixture();
        DeviceIdentity::load_or_generate(&store).unwrap();
        store
            .write_daemon_file(ACCOUNT_SESSION_FILE, &session())
            .unwrap();
        let account = std::fs::read(Kind::Account.file(&store)).unwrap();
        let (_other_dir, other) = crate::config::tests_support::temp_store();
        test_registry()
            .lock()
            .unwrap()
            .insert(other.dir().to_path_buf(), backend);
        write(&other, &Kind::Account.file(&other), &account).unwrap();
        assert!(load(&other, Kind::Account).is_err());
        write(&store, &store.device_key_path(), &account).unwrap();
        assert!(DeviceIdentity::load_or_generate(&store).is_err());
    }
    #[test]
    fn signout_invalidates_a_paused_os_write_without_waiting_for_it() {
        let (_dir, store, backend) = fixture();
        let expected = snapshot(&store, Kind::Account).unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        *backend.pause_write.lock().unwrap() = Some((started_tx, resume_rx));
        let worker_store = store.clone();
        let worker =
            std::thread::spawn(move || replace(&worker_store, &expected, &session(), None));
        started_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        clear(&store, &[Kind::Account]).unwrap();
        assert!(load(&store, Kind::Account).unwrap().is_none());
        resume_tx.send(()).unwrap();
        assert!(worker.join().unwrap().is_err());
        assert!(load(&store, Kind::Account).unwrap().is_none());
        cleanup(&store).unwrap();
        assert!(backend.values.lock().unwrap().is_empty());
    }
    #[test]
    fn withdrawal_read_cannot_return_a_token_after_signout() {
        let (_dir, store, backend) = fixture();
        store
            .write_daemon_file(ACCOUNT_SESSION_FILE, &session())
            .unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        *backend.pause_read.lock().unwrap() = Some((started_tx, resume_rx));
        let reader_store = store.clone();
        let reader = std::thread::spawn(move || crate::account_auth::try_load_token(&reader_store));
        started_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        clear(&store, &[Kind::Account]).unwrap();
        resume_tx.send(()).unwrap();
        assert!(reader.join().unwrap().is_err());
    }
    #[test]
    fn restart_recovers_a_journaled_write_before_metadata_publication() {
        for written in [false, true] {
            let (_dir, store, backend) = fixture();
            write(&store, &Kind::Account.file(&store), &session()).unwrap();
            let reference = CredentialReference::allocate();
            let mut journal = Journal::default();
            journal.retain(reference).unwrap();
            journal.save_named(&store, JOURNAL).unwrap();
            if written {
                backend.write(&reference, b"unpublished-synthetic").unwrap();
            }
            let restarted = ConfigStore::open(store.dir().to_path_buf()).unwrap();
            assert!(
                crate::account_auth::try_load_token(&restarted)
                    .unwrap()
                    .is_some()
            );
            assert!(
                !backend
                    .values
                    .lock()
                    .unwrap()
                    .contains_key(&reference.storage_key().unwrap())
            );
            assert_eq!(backend.values.lock().unwrap().len(), 1);
        }
    }

    #[test]
    fn stale_browser_completion_cannot_restore_a_cleared_session() {
        let (_dir, store, _) = fixture();
        let expected = snapshot(&store, Kind::Account).unwrap();
        clear(&store, &[Kind::Account]).unwrap();
        assert!(replace(&store, &expected, &session(), None).is_err());
        assert!(load(&store, Kind::Account).unwrap().is_none());
    }
    #[test]
    fn wipe_invalidates_credentials_and_keeps_failed_deletion_journal() {
        let (_dir, store, backend) = fixture();
        DeviceIdentity::load_or_generate(&store).unwrap();
        store
            .write_daemon_file(ACCOUNT_SESSION_FILE, &session())
            .unwrap();
        backend.refuse_delete.store(true, Ordering::SeqCst);
        store.wipe().unwrap();
        assert!(DeviceIdentity::load(&store).unwrap().is_none());
        assert!(crate::account_auth::load_token(&store).is_none());
        assert_eq!(
            Journal::read_named(&store, JOURNAL)
                .unwrap()
                .references
                .len(),
            2
        );
        backend.refuse_delete.store(false, Ordering::SeqCst);
        cleanup(&store).unwrap();
        assert!(backend.values.lock().unwrap().is_empty());
    }
}
