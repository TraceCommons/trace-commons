//! Journaled Cloud credential replacement. Blocking callers own OS access;
//! the commit lock covers only local publication, never an OS prompt.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::config::ConfigStore;
use crate::daemon::credential_store::{
    CredentialError, CredentialReference, CredentialStore, SecretBackend,
};
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

/// Can this process hold a Cloud credential at all?
///
/// A read of a reference that does not exist answers `NoEntry` when the store
/// is reachable and `Unentitled` when it is not, so one read distinguishes
/// them without writing anything.
pub(crate) fn store_is_reachable(store: &ConfigStore) -> Result<()> {
    match native(store)?.probe() {
        Err(CredentialError::Unentitled) => Err(anyhow!("near_ai_credential_storage_unentitled")),
        _ => Ok(()),
    }
}

pub(crate) fn cleanup_native(store: &ConfigStore) -> Result<()> {
    if Journal::read(store)?.references.is_empty() {
        return Ok(());
    }
    native(store)?.cleanup()
}

/// Delete Cloud entries left in the legacy keychain by builds before the
/// data-protection move. Best effort by construction.
///
/// Never called at startup, and never for the active reference. Both of those
/// are load-bearing rather than tidiness:
///
/// The active reference is not an orphan. `cleanup_locked` skips it for
/// deletion and leaves it in the journal, so the journal permanently names the
/// live credential. Before a contributor's first post-upgrade ceremony that
/// reference still points at their working legacy entry -- an `sk-` inference
/// key that is valid and does not expire -- and sweeping it would destroy a
/// credential they still hold, with a downgrade afterwards finding nothing.
///
/// And macOS may want authorization to delete a legacy item. Swallowing that
/// error does not prevent the prompt, it only hides the failure after the
/// contributor has already been interrupted; a denial leaves the item in place
/// so the next attempt asks again. So this runs from the ceremony tail and the
/// forget handlers -- moments the contributor initiated -- and not from
/// `DaemonShared::load`, where an unexplained dialog at launch is the exact
/// interruption this work exists to remove.
///
/// Those two together mean nothing is swept before a new ceremony: the only
/// journal reference is the active one, so there is nothing to delete and
/// nothing that can prompt. After a ceremony the previous reference is no
/// longer active and is swept right there, in a flow where the contributor has
/// just accepted a re-sign-in.
///
/// It returns nothing and swallows every failure on purpose. An entry we
/// cannot delete is left alone.
pub(crate) fn sweep_legacy_cloud_entries(store: &ConfigStore) {
    #[cfg(target_os = "macos")]
    {
        let Ok(journal) = Journal::read(store) else {
            return;
        };
        // Read the same way `cleanup_locked` does.
        let Ok(settings) = DaemonSettings::load(store) else {
            return;
        };
        let active = settings
            .cloud_credentials
            .as_ref()
            .map(StoredCloudCredentials::reference);
        let Some(backend) = legacy_secret_backend(store) else {
            return;
        };
        for reference in journal.references {
            if Some(reference) == active {
                continue;
            }
            let _ = backend.delete(&reference);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = store;
}

/// Delete exactly one legacy Cloud entry, named by the caller rather than
/// discovered by walking the journal. Same guarantees as
/// [`sweep_legacy_cloud_entries`]: best effort, swallows every failure, never
/// prompts anywhere the contributor did not just act.
///
/// This exists because the journal cannot be trusted to still name a
/// reference the caller knows is superseded: `replace`'s trailing
/// `cleanup_locked` deletes every non-active journal reference from the
/// *current* (data-protection) backend before a caller-supplied cleanup runs,
/// and `CredentialStore::delete` maps a missing entry to success. A reference
/// that only ever lived in the legacy keychain reads as cleaned up under that
/// mapping and is pruned from the journal without anything legacy-side being
/// swept. Callers that already hold the superseded reference by value --
/// captured before a replace -- must hand it here directly instead of relying
/// on the journal to still contain it.
pub(crate) fn sweep_legacy_cloud_reference(store: &ConfigStore, reference: CredentialReference) {
    #[cfg(target_os = "macos")]
    {
        let Some(backend) = legacy_secret_backend(store) else {
            return;
        };
        let _ = backend.delete(&reference);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = store;
        let _ = reference;
    }
}

#[cfg(target_os = "macos")]
fn legacy_secret_backend(store: &ConfigStore) -> Option<std::sync::Arc<dyn SecretBackend>> {
    #[cfg(test)]
    {
        crate::daemon::cloud_credential_test_support::legacy_backend(store.dir())
    }
    #[cfg(not(test))]
    {
        let _ = store;
        crate::daemon::os_secret_store::OsSecretBackend::legacy_cloud()
            .ok()
            .map(|backend| std::sync::Arc::new(backend) as std::sync::Arc<dyn SecretBackend>)
    }
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
pub(crate) struct Journal {
    pub(crate) references: Vec<CredentialReference>,
}

impl Journal {
    fn read(store: &ConfigStore) -> Result<Self> {
        Self::read_named(store, JOURNAL)
    }

    pub(crate) fn read_named(store: &ConfigStore, name: &str) -> Result<Self> {
        use std::io::Read;
        let file = match std::fs::File::open(store.daemon_path(name)) {
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

    pub(crate) fn retain(&mut self, reference: CredentialReference) -> Result<()> {
        if !self.references.contains(&reference) {
            if self.references.len() == MAX_REFERENCES {
                return Err(anyhow!("near_ai_credential_cleanup_required"));
            }
            self.references.push(reference);
        }
        Ok(())
    }

    fn save(&self, store: &ConfigStore) -> Result<()> {
        self.save_named(store, JOURNAL)
    }

    pub(crate) fn save_named(&self, store: &ConfigStore, name: &str) -> Result<()> {
        let bytes =
            serde_json::to_vec(self).map_err(|_| anyhow!("near_ai_credential_cleanup_invalid"))?;
        store.write_daemon_file(name, &bytes)?;
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

    /// One read of a reference that was never stored. `NoEntry` means the
    /// store answered; `Unentitled` means this process cannot reach it at all.
    ///
    /// [`crate::daemon::credential_store_self_check`] is the same probe with a
    /// deliberately different error mapping, and the divergence is the point.
    /// This one guards a sign-in: everything that is not `Unentitled` is go,
    /// because a transient store error must not stop a contributor signing in.
    /// That one gates a release, where the same leniency would report PASS
    /// against a store failing for any reason other than entitlement, so
    /// anything but `NoEntry` is a failure there. Change one and decide about
    /// the other; do not unify them.
    pub(crate) fn probe(&self) -> Result<(), CredentialError> {
        match self
            .credentials
            .load_bytes(&CredentialReference::allocate())
        {
            Err(CredentialError::Unentitled) => Err(CredentialError::Unentitled),
            _ => Ok(()),
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

pub(crate) fn sync_directory(store: &ConfigStore) -> Result<()> {
    #[cfg(unix)]
    std::fs::File::open(store.dir())
        .and_then(|dir| dir.sync_all())
        .map_err(|_| unavailable())?;
    #[cfg(not(unix))]
    let _ = store;
    Ok(())
}
