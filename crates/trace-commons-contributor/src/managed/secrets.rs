//! OS-backed, per-installation API-key storage. No plaintext fallback.

use super::{AccountId, ManagedError};
use crate::config::ConfigStore;
use sha2::{Digest, Sha256};

pub struct ApiKey(String);

impl ApiKey {
    pub fn new(value: String) -> Result<Self, ManagedError> {
        if value.is_empty() || value.len() > 8192 || value.chars().any(char::is_control) {
            return Err(ManagedError::InvalidRequest);
        }
        Ok(Self(value))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey([redacted])")
    }
}

pub struct SecretStore {
    service: String,
}

impl SecretStore {
    pub fn new(store: &ConfigStore) -> Self {
        let path = std::fs::canonicalize(store.dir()).unwrap_or_else(|_| store.dir().to_path_buf());
        let scope = format!("{:x}", Sha256::digest(path.as_os_str().as_encoded_bytes()));
        Self {
            service: format!("org.tracecommons.near-ai.{}", &scope[..32]),
        }
    }

    /// The managed API-key store. On macOS this is the legacy file keychain,
    /// as for `OsSecretBackend::commons`: the `near-ai` CLI reads these keys
    /// and a command-line binary cannot carry a keychain access group.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn entry(&self, id: AccountId) -> Result<keyring_core::Entry, ManagedError> {
        // A test build never touches the user's real credential store; see
        // the `test-credential-store` feature in this crate's manifest.
        if cfg!(feature = "test-credential-store") {
            return Err(ManagedError::StorageUnavailable);
        }
        #[cfg(target_os = "macos")]
        let store: std::sync::Arc<keyring_core::api::CredentialStore> =
            apple_native_keyring_store::keychain::Store::new()
                .map_err(|_| ManagedError::StorageUnavailable)?;
        #[cfg(windows)]
        let store: std::sync::Arc<keyring_core::api::CredentialStore> =
            windows_native_keyring_store::Store::new()
                .map_err(|_| ManagedError::StorageUnavailable)?;
        #[cfg(target_os = "linux")]
        let store: std::sync::Arc<keyring_core::api::CredentialStore> =
            zbus_secret_service_keyring_store::Store::new()
                .map_err(|_| ManagedError::StorageUnavailable)?;
        // The Windows provider otherwise defaults to Enterprise persistence,
        // which permits roaming the key to other machines.
        #[cfg(windows)]
        let modifiers = std::collections::HashMap::from([("persistence", "Local")]);
        #[cfg(windows)]
        let modifiers = Some(&modifiers);
        #[cfg(not(windows))]
        let modifiers = None;
        // Platform errors can carry secret bytes or identifying metadata, so
        // every one maps to a fixed label.
        store
            .build(&self.service, &id.to_string(), modifiers)
            .map_err(|_| ManagedError::StorageUnavailable)
    }

    pub fn put(&self, id: AccountId, key: &ApiKey) -> Result<(), ManagedError> {
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        {
            self.entry(id)?
                .set_password(key.expose())
                .map_err(|_| ManagedError::StorageUnavailable)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            let _ = (&self.service, id, key);
            Err(ManagedError::StorageUnavailable)
        }
    }

    pub fn get(&self, id: AccountId) -> Result<ApiKey, ManagedError> {
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        {
            ApiKey::new(
                self.entry(id)?
                    .get_password()
                    .map_err(|_| ManagedError::StorageUnavailable)?,
            )
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            let _ = (&self.service, id);
            Err(ManagedError::StorageUnavailable)
        }
    }

    pub fn delete(&self, id: AccountId) -> Result<(), ManagedError> {
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        {
            match self.entry(id)?.delete_credential() {
                Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
                Err(_) => Err(ManagedError::StorageUnavailable),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            let _ = (&self.service, id);
            Err(ManagedError::StorageUnavailable)
        }
    }
}
