//! File-backed stand-in for the OS credential store, compiled only under the
//! `test-credential-store` feature.
//!
//! Integration tests, and the binaries they spawn, link this crate without
//! `cfg(test)`, so the in-memory registry in `cloud_credential_test_support`
//! never reaches them. Before this existed, every one of those tests that
//! generated a device identity wrote a real login-keychain item under the
//! production service name and left it behind.
//!
//! Entries live in `<config dir>/test-credential-store/<storage key>`, so they
//! are scoped to the test's own temporary directory, survive across the
//! processes a test spawns against that directory, and are deleted with it.
//! The bytes are stored in the clear: this is a test double, and the feature
//! that compiles it is enabled only through this crate's dev-dependency on
//! itself, which `cargo build` of a release binary never activates.

use std::path::{Path, PathBuf};

use crate::config::ConfigStore;
use crate::daemon::credential_store::{
    CredentialError, CredentialReference, MAX_SECRET_BYTES, SecretBackend,
};

/// The directory name inside a config dir. `tests/credential_store_isolation.rs`
/// reads it back, so a rename has to move both.
pub(crate) const DIR: &str = "test-credential-store";

#[derive(Debug, Clone)]
pub(crate) struct TestFileBackend {
    dir: PathBuf,
}

impl TestFileBackend {
    // Unit tests (`cfg(test)`) keep the in-memory registry instead.
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn for_store(store: &ConfigStore) -> Self {
        Self::in_dir(store.dir())
    }

    fn in_dir(config_dir: &Path) -> Self {
        Self {
            dir: config_dir.join(DIR),
        }
    }

    fn path(&self, reference: &CredentialReference) -> Result<PathBuf, CredentialError> {
        Ok(self.dir.join(reference.storage_key()?))
    }
}

impl SecretBackend for TestFileBackend {
    fn read(&self, reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
        match std::fs::read(self.path(reference)?) {
            Ok(bytes) if bytes.len() <= MAX_SECRET_BYTES => Ok(bytes),
            Ok(_) => Err(CredentialError::TooLarge),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(CredentialError::NoEntry)
            }
            Err(_) => Err(CredentialError::Unavailable),
        }
    }

    fn write(&self, reference: &CredentialReference, bytes: &[u8]) -> Result<(), CredentialError> {
        if bytes.len() > MAX_SECRET_BYTES {
            return Err(CredentialError::TooLarge);
        }
        let path = self.path(reference)?;
        std::fs::create_dir_all(&self.dir).map_err(|_| CredentialError::Unavailable)?;
        crate::config::write_atomic_0600(&self.dir, &path, bytes)
            .map_err(|_| CredentialError::Unavailable)
    }

    fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
        match std::fs::remove_file(self.path(reference)?) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(CredentialError::NoEntry)
            }
            Err(_) => Err(CredentialError::Unavailable),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_reports_absence_like_the_os_store() {
        let dir = tempfile::tempdir().unwrap();
        let backend = TestFileBackend::in_dir(dir.path());
        let reference = CredentialReference::allocate();
        assert_eq!(backend.read(&reference), Err(CredentialError::NoEntry));
        assert_eq!(backend.delete(&reference), Err(CredentialError::NoEntry));
        backend.write(&reference, b"{\"synthetic\":1}").unwrap();
        assert_eq!(backend.read(&reference).unwrap(), b"{\"synthetic\":1}");
        assert!(
            dir.path()
                .join(DIR)
                .join(reference.storage_key().unwrap())
                .is_file()
        );
        backend.delete(&reference).unwrap();
        assert_eq!(backend.read(&reference), Err(CredentialError::NoEntry));
    }

    #[test]
    fn refuses_what_the_os_store_refuses_for_size() {
        let dir = tempfile::tempdir().unwrap();
        let backend = TestFileBackend::in_dir(dir.path());
        let reference = CredentialReference::allocate();
        assert_eq!(
            backend.write(&reference, &vec![b'x'; MAX_SECRET_BYTES + 1]),
            Err(CredentialError::TooLarge)
        );
    }
}
