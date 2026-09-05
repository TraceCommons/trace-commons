//! Independent compute consent and preferences for the desktop controller.
//!
//! This foundation does not launch a worker. Persisted consent never implies an
//! automatic start: a new controller session requires an explicit Resume. Trace
//! scopes, enrollment, credentials and session discovery are not inputs here.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::ConfigStore;

pub mod artifact;
mod controller;
mod live;
mod process;
pub mod worker_protocol;
pub use controller::{
    ComputeCommand, ComputeController, ComputeCopy, ComputeSnapshot, ComputeState,
};
pub use process::{LocalWorkerConfig, StopOutcome};

const SETTINGS_FILE: &str = "settings.json";
const SETTINGS_SCHEMA: &str = "trace_commons.compute_settings.v1";
const MAX_SETTINGS_BYTES: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ComputeSettingsError {
    #[error("compute state directory must be absolute")]
    InvalidDirectory,
    #[error("compute settings could not be read")]
    Read,
    #[error("compute settings are invalid or unsupported")]
    InvalidSettings,
    #[error("compute settings could not be saved")]
    Write,
    #[error("compute memory allowance must be positive and representable in bytes")]
    InvalidAllowance,
}

/// Capacity advertised to the pool, not a hard process memory limit. Whether it
/// fits this machine is checked against current capabilities before launching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSettings {
    schema: String,
    consent_granted: bool,
    ram_allowance_gib: Option<u64>,
}

impl Default for ComputeSettings {
    fn default() -> Self {
        Self {
            schema: SETTINGS_SCHEMA.into(),
            consent_granted: false,
            ram_allowance_gib: None,
        }
    }
}

impl ComputeSettings {
    pub fn consent_granted(&self) -> bool {
        self.consent_granted
    }

    pub fn ram_allowance_gib(&self) -> Option<u64> {
        self.ram_allowance_gib
    }

    /// Explicit consent, called only after the host's compute consent journey.
    /// Saving this value does not start a worker or permit automatic restart.
    pub fn grant(ram_allowance_gib: u64) -> Result<Self, ComputeSettingsError> {
        let settings = Self {
            consent_granted: true,
            ram_allowance_gib: Some(ram_allowance_gib),
            ..Self::default()
        };
        settings.validate()?;
        Ok(settings)
    }

    pub fn revoke(&mut self) {
        self.consent_granted = false;
    }

    /// The only states restored from disk. Running/starting are deliberately not
    /// representable here: pool admission and live process state need fresh proof.
    pub fn restored_state(&self) -> RestoredComputeState {
        if self.consent_granted {
            RestoredComputeState::Paused
        } else {
            RestoredComputeState::Disabled
        }
    }

    fn validate(&self) -> Result<(), ComputeSettingsError> {
        if self.schema != SETTINGS_SCHEMA
            || (self.consent_granted && self.ram_allowance_gib.is_none())
        {
            return Err(ComputeSettingsError::InvalidSettings);
        }
        if let Some(gib) = self.ram_allowance_gib {
            if gib == 0 || gib.checked_mul(1 << 30).is_none() {
                return Err(ComputeSettingsError::InvalidAllowance);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoredComputeState {
    Disabled,
    Paused,
}

/// Stores only compute preferences beneath `<contributor-state>/compute` using
/// the existing atomic, permission-restricted writer. The future controller owns
/// command serialization; this store is not a cross-process ownership lock.
pub struct ComputeSettingsStore {
    store: ConfigStore,
}

impl ComputeSettingsStore {
    pub fn open(contributor_state: &Path) -> Result<Self, ComputeSettingsError> {
        if !contributor_state.is_absolute() {
            return Err(ComputeSettingsError::InvalidDirectory);
        }
        let store = ConfigStore::open(contributor_state.join("compute"))
            .map_err(|_| ComputeSettingsError::Write)?;
        Ok(Self { store })
    }

    /// Passed only to the child as HOLONEAR_HOME. It is separate from both the
    /// contributor credentials and a standalone Holonear installation.
    pub fn worker_home(&self) -> PathBuf {
        self.store.dir().join("worker")
    }

    pub fn load(&self) -> Result<ComputeSettings, ComputeSettingsError> {
        let file = match std::fs::File::open(self.store.dir().join(SETTINGS_FILE)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ComputeSettings::default());
            }
            Err(_) => return Err(ComputeSettingsError::Read),
        };
        let mut bytes = Vec::new();
        file.take(MAX_SETTINGS_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ComputeSettingsError::Read)?;
        if bytes.len() as u64 > MAX_SETTINGS_BYTES {
            return Err(ComputeSettingsError::InvalidSettings);
        }
        let settings: ComputeSettings =
            serde_json::from_slice(&bytes).map_err(|_| ComputeSettingsError::InvalidSettings)?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn save(&self, settings: &ComputeSettings) -> Result<(), ComputeSettingsError> {
        settings.validate()?;
        let bytes = serde_json::to_vec_pretty(settings).map_err(|_| ComputeSettingsError::Write)?;
        self.store
            .write_daemon_file(SETTINGS_FILE, &bytes)
            .map_err(|_| ComputeSettingsError::Write)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_settings_are_disabled_without_enrollment_or_session_discovery() {
        let root = tempfile::tempdir().unwrap();
        let store = ComputeSettingsStore::open(root.path()).unwrap();
        let settings = store.load().unwrap();
        assert!(!settings.consent_granted());
        assert_eq!(settings.ram_allowance_gib(), None);
        assert_eq!(settings.restored_state(), RestoredComputeState::Disabled);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        assert!(!store.worker_home().exists());
    }

    #[test]
    fn consent_restores_paused_and_revocation_preserves_trace_files() {
        let root = tempfile::tempdir().unwrap();
        let trace_config = root.path().join("config.json");
        std::fs::write(&trace_config, b"trace configuration sentinel").unwrap();
        let store = ComputeSettingsStore::open(root.path()).unwrap();
        store.save(&ComputeSettings::grant(8).unwrap()).unwrap();
        let reopened = ComputeSettingsStore::open(root.path()).unwrap();
        let mut settings = reopened.load().unwrap();
        assert!(settings.consent_granted());
        assert_eq!(settings.ram_allowance_gib(), Some(8));
        assert_eq!(settings.restored_state(), RestoredComputeState::Paused);
        settings.revoke();
        reopened.save(&settings).unwrap();
        let revoked = store.load().unwrap();
        assert_eq!(revoked.restored_state(), RestoredComputeState::Disabled);
        assert_eq!(revoked.ram_allowance_gib(), Some(8));
        assert_eq!(
            std::fs::read(trace_config).unwrap(),
            b"trace configuration sentinel"
        );
        assert_eq!(store.worker_home(), root.path().join("compute/worker"));
    }

    #[test]
    fn malformed_future_missing_and_oversized_settings_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let store = ComputeSettingsStore::open(root.path()).unwrap();
        let mut future = ComputeSettings::grant(8).unwrap();
        future.schema = "future".into();
        let mut missing_allowance = ComputeSettings::grant(8).unwrap();
        missing_allowance.ram_allowance_gib = None;
        for bytes in [
            b"{".to_vec(),
            b"{}".to_vec(),
            serde_json::to_vec(&future).unwrap(),
            serde_json::to_vec(&missing_allowance).unwrap(),
            vec![b' '; MAX_SETTINGS_BYTES as usize + 1],
        ] {
            std::fs::write(root.path().join("compute/settings.json"), &bytes).unwrap();
            assert_eq!(store.load(), Err(ComputeSettingsError::InvalidSettings));
            assert_eq!(
                std::fs::read(root.path().join("compute/settings.json")).unwrap(),
                bytes
            );
        }
    }

    #[test]
    fn rejects_invalid_allowances_and_relative_roots() {
        for allowance in [0, u64::MAX / (1 << 30) + 1] {
            assert_eq!(
                ComputeSettings::grant(allowance),
                Err(ComputeSettingsError::InvalidAllowance)
            );
        }
        assert_eq!(
            ComputeSettings::grant(1).unwrap().ram_allowance_gib(),
            Some(1)
        );
        assert!(matches!(
            ComputeSettingsStore::open(Path::new("relative")),
            Err(ComputeSettingsError::InvalidDirectory)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn settings_and_directory_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let store = ComputeSettingsStore::open(root.path()).unwrap();
        store.save(&ComputeSettings::grant(4).unwrap()).unwrap();
        for (path, mode) in [
            (root.path().join("compute"), 0o700),
            (root.path().join("compute/settings.json"), 0o600),
        ] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                mode
            );
        }
    }
}
