//! Journaled Cloud credential replacement. Blocking callers own OS access;
//! the commit lock covers only local publication, never an OS prompt.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::config::ConfigStore;
use crate::daemon::credential_store::{CredentialReference, CredentialStore, SecretBackend};
use crate::daemon::nearai_credential::session::coordination;
use crate::daemon::settings::{DaemonSettings, NearAiInferenceCredential, NearAiSession};
use crate::daemon::stored_cloud_credentials::StoredCloudCredentials;

const JOURNAL: &str = "cloud-credential-cleanup.json";
const MAX_REFERENCES: usize = 64;

/// Synchronous boundary for startup and the FFI pre-start settings writer.
/// Async callers must move this entire operation to `spawn_blocking`.
pub fn load_for_runtime(store: &ConfigStore) -> Result<DaemonSettings> {
    let settings = DaemonSettings::load(store)?;
    if settings.cloud_credentials.is_none()
        && settings.near_ai_inference.is_none()
        && settings.near_ai_session.is_none()
    {
        return Ok(settings);
    }
    native(store)?.migrate(settings)
}

pub(crate) fn native(
    store: &ConfigStore,
) -> Result<CloudCredentialLifecycle<std::sync::Arc<dyn SecretBackend>>> {
    #[cfg(test)]
    {
        let backend = crate::daemon::cloud_credential_test_support::backend(store.dir())
            .ok_or_else(unavailable)?;
        Ok(CloudCredentialLifecycle::new(store.clone(), backend))
    }
    #[cfg(not(test))]
    Ok(CloudCredentialLifecycle::new(
        store.clone(),
        std::sync::Arc::new(crate::daemon::os_secret_store::OsSecretBackend::new()?),
    ))
}

pub(crate) fn cleanup_native(store: &ConfigStore) -> Result<()> {
    if Journal::read(store)?.references.is_empty() {
        return Ok(());
    }
    native(store)?.cleanup()
}

pub(crate) fn cleanup_pending(store: &ConfigStore) -> Result<bool> {
    let locks = coordination(store.dir())?;
    let _commit = locks.commit.lock().map_err(|_| unavailable())?;
    let active = DaemonSettings::load(store)?
        .cloud_credentials
        .as_ref()
        .map(StoredCloudCredentials::reference);
    Ok(Journal::read(store)?
        .references
        .iter()
        .any(|reference| Some(*reference) != active))
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    references: Vec<CredentialReference>,
}

impl Journal {
    fn read(store: &ConfigStore) -> Result<Self> {
        use std::io::Read;
        let file = match std::fs::File::open(store.daemon_path(JOURNAL)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(_) => return Err(unavailable()),
        };
        let mut bytes = Vec::new();
        file.take(16_385)
            .read_to_end(&mut bytes)
            .map_err(|_| unavailable())?;
        if bytes.len() > 16_384 {
            return Err(anyhow!("near_ai_credential_cleanup_invalid"));
        }
        let journal: Self = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow!("near_ai_credential_cleanup_invalid"))?;
        if journal.references.len() > MAX_REFERENCES {
            return Err(anyhow!("near_ai_credential_cleanup_invalid"));
        }
        for reference in &journal.references {
            reference.storage_key()?;
        }
        Ok(journal)
    }

    fn retain(&mut self, reference: CredentialReference) -> Result<()> {
        if !self.references.contains(&reference) {
            if self.references.len() == MAX_REFERENCES {
                return Err(anyhow!("near_ai_credential_cleanup_required"));
            }
            self.references.push(reference);
        }
        Ok(())
    }

    fn save(&self, store: &ConfigStore) -> Result<()> {
        let bytes =
            serde_json::to_vec(self).map_err(|_| anyhow!("near_ai_credential_cleanup_invalid"))?;
        store.write_daemon_file(JOURNAL, &bytes)?;
        sync_directory(store)
    }
}

/// The OS adapter and all methods on this owner run in a blocking task.
/// Settings publication takes the short per-directory commit lock. A second
/// storage lock serializes preparation and cleanup without delaying forget.
pub(crate) struct CloudCredentialLifecycle<B: SecretBackend> {
    store: ConfigStore,
    credentials: CredentialStore<B>,
}

impl<B: SecretBackend> CloudCredentialLifecycle<B> {
    pub(crate) fn new(store: ConfigStore, backend: B) -> Self {
        Self {
            store,
            credentials: CredentialStore::new(backend),
        }
    }

    pub(crate) fn resolve(&self, mut settings: DaemonSettings) -> Result<DaemonSettings> {
        if let Some(metadata) = &settings.cloud_credentials {
            // A mixed record is ambiguous and must never fall back to its
            // plaintext fields when the OS entry is unavailable.
            if settings.near_ai_inference.is_some() || settings.near_ai_session.is_some() {
                return Err(anyhow!("near_ai_credential_storage_mixed_record"));
            }
            let bundle = self
                .credentials
                .load(&metadata.reference(), metadata.binding_digest())?;
            (settings.near_ai_inference, settings.near_ai_session) = metadata.resolve(bundle)?;
            let locks = coordination(self.store.dir())?;
            let _commit = locks.commit.lock().map_err(|_| unavailable())?;
            ensure_current(&DaemonSettings::load(&self.store)?, &settings)?;
        }
        Ok(settings)
    }

    /// A late browser result or rotated token may publish only against the
    /// connection it started with. The callback also checks attempt identity
    /// and updates runtime memory while publication remains serialized.
    pub(crate) fn replace(
        &self,
        expected: &DaemonSettings,
        inference: Option<NearAiInferenceCredential>,
        session: Option<NearAiSession>,
        accept: impl FnOnce() -> Result<()>,
        published: impl FnOnce(&DaemonSettings) -> Result<()>,
    ) -> Result<DaemonSettings> {
        let locks = coordination(self.store.dir())?;
        let _storage = locks.storage.lock().map_err(|_| unavailable())?;
        self.cleanup_locked()?;
        let reference = CredentialReference::allocate();
        let (metadata, bundle) =
            StoredCloudCredentials::from_secrets(reference, inference.as_ref(), session.as_ref())?;
        {
            let _commit = locks.commit.lock().map_err(|_| unavailable())?;
            let disk = DaemonSettings::load(&self.store)?;
            ensure_current(&disk, expected)?;
            let mut journal = Journal::read(&self.store)?;
            journal.retain(reference)?;
            if let Some(old) = &disk.cloud_credentials {
                journal.retain(old.reference())?;
            }
            journal.save(&self.store)?;
        }
        self.credentials.prepare_at(&reference, &bundle)?;
        let result = (|| {
            let _commit = locks.commit.lock().map_err(|_| unavailable())?;
            let mut disk = DaemonSettings::load(&self.store)?;
            ensure_current(&disk, expected)?;
            accept()?;
            disk.cloud_credentials = Some(metadata);
            disk.near_ai_inference = inference;
            disk.near_ai_session = session;
            disk.save_locked(&self.store)?;
            sync_directory(&self.store)?;
            published(&disk)?;
            Ok(disk)
        })();
        // Publication may have succeeded even if its directory sync failed.
        // Cleanup re-reads the active reference and will never delete it.
        // Failed cleanup remains in the journal for the next explicit retry.
        if result.is_ok() {
            let _ = self.cleanup_locked();
        }
        result
    }

    pub(crate) fn migrate(&self, settings: DaemonSettings) -> Result<DaemonSettings> {
        if settings.cloud_credentials.is_some() {
            return self.resolve(settings);
        }
        if settings.near_ai_inference.is_none() && settings.near_ai_session.is_none() {
            return Ok(settings);
        }
        self.replace(
            &settings,
            settings.near_ai_inference.clone(),
            settings.near_ai_session.clone(),
            || Ok(()),
            |_| Ok(()),
        )
    }

    pub(crate) fn cleanup(&self) -> Result<()> {
        let locks = coordination(self.store.dir())?;
        let _storage = locks.storage.lock().map_err(|_| unavailable())?;
        self.cleanup_locked()
    }

    fn cleanup_locked(&self) -> Result<()> {
        let locks = coordination(self.store.dir())?;
        let (active, references) = {
            let _commit = locks.commit.lock().map_err(|_| unavailable())?;
            let settings = DaemonSettings::load(&self.store)?;
            sync_directory(&self.store)?;
            let active = settings
                .cloud_credentials
                .as_ref()
                .map(StoredCloudCredentials::reference);
            (active, Journal::read(&self.store)?.references)
        };
        let mut failed = false;
        for reference in references {
            if Some(reference) == active {
                continue;
            }
            if self.credentials.delete(&reference).is_err() {
                failed = true;
                continue;
            }
            let _commit = locks.commit.lock().map_err(|_| unavailable())?;
            let mut journal = Journal::read(&self.store)?;
            journal.references.retain(|entry| *entry != reference);
            journal.save(&self.store)?;
        }
        if failed {
            Err(anyhow!("near_ai_credential_cleanup_required"))
        } else {
            Ok(())
        }
    }
}

/// Caller holds the commit lock and cancels pending browser attempts before
/// this operation. Removal is durable before any slow OS deletion begins.
pub(crate) fn forget_locked(store: &ConfigStore) -> Result<bool> {
    let mut settings = DaemonSettings::load(store)?;
    let had = settings.cloud_credentials.is_some()
        || settings.near_ai_inference.is_some()
        || settings.near_ai_session.is_some();
    if let Some(metadata) = settings.cloud_credentials.take() {
        let mut journal = Journal::read(store)?;
        journal.retain(metadata.reference())?;
        journal.save(store)?;
    }
    settings.near_ai_inference = None;
    settings.near_ai_session = None;
    settings.save_locked(store)?;
    sync_directory(store)?;
    Ok(had)
}

pub(crate) fn ensure_current(disk: &DaemonSettings, expected: &DaemonSettings) -> Result<()> {
    let same = disk.cloud_credentials == expected.cloud_credentials
        && (disk.cloud_credentials.is_some()
            || (disk.near_ai_inference == expected.near_ai_inference
                && disk.near_ai_session == expected.near_ai_session));
    if same {
        Ok(())
    } else {
        Err(anyhow!("near_ai_credential_session_changed"))
    }
}

fn unavailable() -> anyhow::Error {
    anyhow!("near_ai_credential_storage_unavailable")
}

fn sync_directory(store: &ConfigStore) -> Result<()> {
    #[cfg(unix)]
    std::fs::File::open(store.dir())
        .and_then(|dir| dir.sync_all())
        .map_err(|_| unavailable())?;
    #[cfg(not(unix))]
    let _ = store;
    Ok(())
}
