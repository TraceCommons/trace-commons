//! Explicit synthetic OS-store injection for temporary daemon fixtures.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::config::ConfigStore;
use crate::daemon::credential_store::{CredentialError, CredentialReference, SecretBackend};
use crate::daemon::settings::DaemonSettings;

#[derive(Default)]
struct MemoryBackend(Mutex<HashMap<String, Vec<u8>>>);

impl SecretBackend for MemoryBackend {
    fn read(&self, reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
        self.0
            .lock()
            .unwrap()
            .get(&reference.storage_key()?)
            .cloned()
            .ok_or(CredentialError::NoEntry)
    }
    fn write(&self, reference: &CredentialReference, bytes: &[u8]) -> Result<(), CredentialError> {
        self.0
            .lock()
            .unwrap()
            .insert(reference.storage_key()?, bytes.to_vec());
        Ok(())
    }
    fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
        self.0.lock().unwrap().remove(&reference.storage_key()?);
        Ok(())
    }
}

fn registry() -> &'static Mutex<HashMap<PathBuf, Arc<dyn SecretBackend>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<dyn SecretBackend>>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

pub(crate) fn install(store: &ConfigStore) {
    registry()
        .lock()
        .unwrap()
        .entry(store.dir().to_path_buf())
        .or_insert_with(|| Arc::new(MemoryBackend::default()));
}

pub(crate) fn backend(dir: &Path) -> Option<Arc<dyn SecretBackend>> {
    registry().lock().unwrap().get(dir).cloned()
}

pub(crate) fn install_backend(store: &ConfigStore, backend: Arc<dyn SecretBackend>) {
    registry()
        .lock()
        .unwrap()
        .insert(store.dir().to_path_buf(), backend);
}

impl DaemonSettings {
    /// Exercise production OS publication with synthetic storage. Legacy
    /// migration tests deliberately write their old-format fixture as bytes.
    pub(crate) fn save_for_test(&mut self, store: &ConfigStore) -> anyhow::Result<()> {
        install(store);
        if self.near_ai_inference.is_some() || self.near_ai_session.is_some() {
            let expected = Self::load(store)?;
            let mut preferences = self.clone();
            preferences.cloud_credentials = expected.cloud_credentials.clone();
            preferences.near_ai_inference = None;
            preferences.near_ai_session = None;
            preferences.save(store)?;
            *self = crate::daemon::cloud_credential_lifecycle::native(store)?.replace(
                &expected,
                self.near_ai_inference.clone(),
                self.near_ai_session.clone(),
                || Ok(()),
                |_| Ok(()),
            )?;
            Ok(())
        } else {
            self.cloud_credentials = None;
            self.save(store)
        }
    }
}
