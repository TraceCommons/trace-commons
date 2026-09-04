//! Serialized consent controller. Worker availability stays closed until a
//! pinned, authenticated supervisor adapter is installed. No transport or trace
//! discovery is performed by this module.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::{ComputeSettings, ComputeSettingsError, ComputeSettingsStore};

#[derive(Debug, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputeCommand {
    Enable { ram_allowance_gib: u64 },
    Resume {},
    Pause {},
    Disable {},
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeState {
    Disabled,
    Unavailable,
    Starting,
    Waiting,
    Training,
    Serving,
    Draining,
    Paused,
    Stale,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComputeSnapshot {
    pub schema: &'static str,
    pub state: ComputeState,
    pub reason: &'static str,
    pub title: &'static str,
    pub detail: &'static str,
    pub consent_granted: bool,
    pub ram_allowance_gib: Option<u64>,
    pub available: bool,
    pub can_enable: bool,
    pub can_resume: bool,
    pub can_pause: bool,
    pub copy: ComputeCopy,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComputeCopy {
    pub destination: &'static str,
    pub introduction: &'static str,
    pub allowance_label: &'static str,
    pub allowance_detail: &'static str,
    pub enable: &'static str,
    pub resume: &'static str,
    pub pause: &'static str,
    pub disable: &'static str,
}

impl Default for ComputeCopy {
    fn default() -> Self {
        Self {
            destination: "Compute",
            introduction: "Contribute compute to Holonear independently of trace contribution. Enabling compute does not authorize access to your local traces. The test pilot does not promise paid earnings.",
            allowance_label: "RAM scheduling allowance (GiB)",
            allowance_detail: "Capacity advertised to the pool, not a hard memory limit. Actual memory use may differ.",
            enable: "Enable compute",
            resume: "Resume compute",
            pause: "Pause compute",
            disable: "Disable compute",
        }
    }
}

struct Inner {
    store: ComputeSettingsStore,
    settings: ComputeSettings,
    state: ComputeState,
    reason: &'static str,
}

/// One app-owned controller, independent of daemon and view lifetimes. Call
/// commands on a background thread: settings writes are synchronous. The mutex
/// serializes the whole read/validate/persist/transition transaction. This is
/// not a cross-process worker ownership lock.
pub struct ComputeController {
    inner: Mutex<Inner>,
}

impl ComputeController {
    pub fn open(root: &std::path::Path) -> Result<Self, ComputeSettingsError> {
        let store = ComputeSettingsStore::open(root)?;
        let settings = store.load()?;
        let state = if settings.consent_granted() {
            ComputeState::Paused
        } else {
            ComputeState::Disabled
        };
        Ok(Self {
            inner: Mutex::new(Inner {
                store,
                settings,
                state,
                reason: "worker-unavailable",
            }),
        })
    }

    pub fn snapshot(&self) -> ComputeSnapshot {
        match self.inner.lock() {
            Ok(inner) => inner.snapshot(),
            Err(_) => error_snapshot(),
        }
    }

    pub fn command(&self, command: ComputeCommand) -> ComputeSnapshot {
        let Ok(mut inner) = self.inner.lock() else {
            return error_snapshot();
        };
        // Availability is a build capability, never supplied by the shell or
        // a status frame. Consent cannot bypass missing authenticated transport.
        match command {
            ComputeCommand::Enable { ram_allowance_gib } => {
                if ComputeSettings::grant(ram_allowance_gib).is_err() {
                    inner.reason = "invalid-allowance";
                } else {
                    inner.state = ComputeState::Unavailable;
                    inner.reason = "worker-unavailable";
                }
            }
            ComputeCommand::Resume {} => {
                if inner.settings.consent_granted() {
                    inner.state = ComputeState::Unavailable;
                    inner.reason = "worker-unavailable";
                } else {
                    inner.state = ComputeState::Disabled;
                    inner.reason = "consent-required";
                }
            }
            ComputeCommand::Pause {} => {
                inner.state = if inner.settings.consent_granted() {
                    ComputeState::Paused
                } else {
                    ComputeState::Disabled
                };
                inner.reason = "worker-unavailable";
            }
            ComputeCommand::Disable {} => {
                let mut settings = inner.settings.clone();
                settings.revoke();
                match inner.store.save(&settings) {
                    Ok(()) => {
                        inner.settings = settings;
                        inner.state = ComputeState::Disabled;
                        inner.reason = "worker-unavailable";
                    }
                    Err(_) => {
                        inner.state = ComputeState::Error;
                        inner.reason = "settings-write-failed";
                    }
                }
            }
        }
        inner.snapshot()
    }
}

impl Inner {
    fn snapshot(&self) -> ComputeSnapshot {
        let (title, detail) = match self.reason {
            "invalid-allowance" => (
                "Check RAM allowance",
                "Choose a positive RAM scheduling allowance.",
            ),
            "consent-required" => (
                "Compute is disabled",
                "Compute requires your separate consent before it can start.",
            ),
            "settings-write-failed" => (
                "Compute settings could not be saved",
                "Your previous consent setting is unchanged. Try again.",
            ),
            _ => match self.state {
                ComputeState::Paused => (
                    "Compute is paused",
                    "Resume will require a compatible packaged worker. Compute never resumes automatically after restarting the app.",
                ),
                ComputeState::Disabled => (
                    "Compute is disabled",
                    "A compatible packaged Holonear worker is not available in this build.",
                ),
                _ => (
                    "Compute is unavailable",
                    "A compatible packaged Holonear worker is not available in this build.",
                ),
            },
        };
        ComputeSnapshot {
            schema: "trace_commons.compute_status.v1",
            state: self.state,
            reason: self.reason,
            title,
            detail,
            consent_granted: self.settings.consent_granted(),
            ram_allowance_gib: self.settings.ram_allowance_gib(),
            available: false,
            can_enable: false,
            can_resume: false,
            can_pause: false,
            copy: ComputeCopy::default(),
        }
    }
}

fn error_snapshot() -> ComputeSnapshot {
    ComputeSnapshot {
        schema: "trace_commons.compute_status.v1",
        state: ComputeState::Error,
        reason: "controller-unavailable",
        title: "Compute status is unavailable",
        detail: "Restart the app to read compute settings again.",
        consent_granted: false,
        ram_allowance_gib: None,
        available: false,
        can_enable: false,
        can_resume: false,
        can_pause: false,
        copy: ComputeCopy::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_cannot_grant_consent_or_launch() {
        let root = tempfile::tempdir().unwrap();
        let controller = ComputeController::open(root.path()).unwrap();
        assert_eq!(controller.snapshot().state, ComputeState::Disabled);
        let result = controller.command(ComputeCommand::Enable {
            ram_allowance_gib: 8,
        });
        assert_eq!(result.state, ComputeState::Unavailable);
        assert!(!result.consent_granted);
        assert!(!root.path().join("compute/worker").exists());
        assert_eq!(
            controller.command(ComputeCommand::Resume {}).reason,
            "consent-required"
        );
    }

    #[test]
    fn restore_pause_revoke_and_write_failure() {
        let root = tempfile::tempdir().unwrap();
        let store = ComputeSettingsStore::open(root.path()).unwrap();
        store.save(&ComputeSettings::grant(8).unwrap()).unwrap();
        let controller = ComputeController::open(root.path()).unwrap();
        assert_eq!(controller.snapshot().state, ComputeState::Paused);
        assert_eq!(
            controller.command(ComputeCommand::Resume {}).state,
            ComputeState::Unavailable
        );
        for _ in 0..2 {
            assert_eq!(
                controller.command(ComputeCommand::Pause {}).state,
                ComputeState::Paused
            );
        }
        std::fs::remove_file(root.path().join("compute/settings.json")).unwrap();
        std::fs::create_dir(root.path().join("compute/settings.json")).unwrap();
        let result = controller.command(ComputeCommand::Disable {});
        assert_eq!(result.reason, "settings-write-failed");
        assert!(result.consent_granted);
        std::fs::remove_dir(root.path().join("compute/settings.json")).unwrap();
        assert!(
            !controller
                .command(ComputeCommand::Disable {})
                .consent_granted
        );
        assert_eq!(
            ComputeController::open(root.path())
                .unwrap()
                .snapshot()
                .state,
            ComputeState::Disabled
        );
    }

    #[test]
    fn commands_are_strict_and_allowance_validation_preserves_settings() {
        for invalid in [
            r#"{"command":"resume","token":"secret"}"#,
            r#"{"command":"enable"}"#,
            r#"{"command":"start"}"#,
        ] {
            assert!(serde_json::from_str::<ComputeCommand>(invalid).is_err());
        }
        let root = tempfile::tempdir().unwrap();
        let controller = ComputeController::open(root.path()).unwrap();
        assert_eq!(
            controller
                .command(ComputeCommand::Enable {
                    ram_allowance_gib: 0
                })
                .reason,
            "invalid-allowance"
        );
        assert!(!root.path().join("compute/settings.json").exists());
    }
}
