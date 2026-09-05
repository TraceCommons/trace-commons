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
    pub local_development: bool,
    pub command_pending: bool,
    pub worker_stopped: bool,
    pub admission: Option<super::worker_protocol::Admission>,
    pub telemetry_age_ms: Option<u64>,
    pub drain_outcome: Option<super::worker_protocol::DrainOutcome>,
    pub stop_outcome: Option<super::StopOutcome>,
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
    pub loading: &'static str,
    pub unavailable: &'static str,
    pub quit_detail: &'static str,
    pub quit_refused: &'static str,
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
            loading: "Loading compute settings…",
            unavailable: "Compute settings could not be loaded. Restart the app to try again.",
            quit_detail: "Enabled compute must stop before the app quits. Worker termination and coordinator drain acknowledgement are separate; a forced exit does not confirm a completed handoff.",
            quit_refused: "Quit was cancelled because worker termination was not confirmed in time. Keep the app open until stopping finishes, then try quitting again.",
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
    live: Option<super::live::LiveController>,
}

impl ComputeController {
    /// Begin a single-use platform read before consulting native APIs. Production
    /// unavailable handles have no resource adapter and return None.
    pub fn resource_begin(&self) -> Option<super::ResourceTicket> {
        self.live.as_ref()?.resource_begin()
    }

    /// Short safety ingress, independent of the actor's bounded command queue.
    pub fn resource_event(&self, event: super::ResourceEvent) -> bool {
        self.live
            .as_ref()
            .is_some_and(|live| live.resource_event(event))
    }

    pub fn open(root: &std::path::Path) -> Result<Self, ComputeSettingsError> {
        let store = ComputeSettingsStore::open(root)?;
        let settings = store.load()?;
        let state = if settings.consent_granted() {
            ComputeState::Paused
        } else {
            ComputeState::Disabled
        };
        Ok(Self {
            live: None,
            inner: Mutex::new(Inner {
                store,
                settings,
                state,
                reason: "worker-unavailable",
            }),
        })
    }

    pub fn snapshot(&self) -> ComputeSnapshot {
        if let Some(live) = &self.live {
            return live.snapshot();
        }
        match self.inner.lock() {
            Ok(inner) => inner.snapshot(),
            Err(_) => error_snapshot(),
        }
    }

    pub fn command(&self, command: ComputeCommand) -> ComputeSnapshot {
        if let Some(live) = &self.live {
            return live.command(command);
        }
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

    /// Explicit local-only development entrypoint. Release builds refuse it;
    /// the production constructor never consults environment overrides.
    pub fn open_local(
        root: &std::path::Path,
        config: super::LocalWorkerConfig,
    ) -> anyhow::Result<Self> {
        let mut controller = Self::open(root)?;
        controller.live = Some(super::live::LiveController::open(
            root,
            config,
            controller.snapshot(),
        )?);
        Ok(controller)
    }

    /// Bounded wait for owned-worker termination, independent of drain evidence.
    /// A false worker_stopped result means the caller must retain the handle.
    pub fn shutdown(&self, timeout: std::time::Duration) -> ComputeSnapshot {
        match &self.live {
            Some(live) => live.shutdown(timeout),
            None => self.command(ComputeCommand::Pause {}),
        }
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
            local_development: false,
            command_pending: false,
            worker_stopped: true,
            admission: None,
            telemetry_age_ms: None,
            drain_outcome: None,
            stop_outcome: None,
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
        local_development: false,
        command_pending: false,
        worker_stopped: false,
        admission: None,
        telemetry_age_ms: None,
        drain_outcome: None,
        stop_outcome: None,
        copy: ComputeCopy::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn local_start_flood_cannot_lose_revocation_or_restart_after_shutdown() {
        use sha2::{Digest, Sha256};
        use std::{
            os::unix::fs::PermissionsExt,
            time::{Duration, Instant},
        };
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("sleepy-worker");
        let script = b"#!/bin/sh\nexec /bin/sleep 60\n";
        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let controller = ComputeController::open_local(
            root.path(),
            super::super::LocalWorkerConfig {
                binary,
                expected_sha256: hex::encode(Sha256::digest(script)),
                coordinator: "ws://127.0.0.1:9999".into(),
                startup_timeout_secs: 20,
            },
        )
        .unwrap();
        crate::compute::resource::tests::healthy(&controller);
        controller.command(ComputeCommand::Enable {
            ram_allowance_gib: 1,
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while controller.snapshot().state != ComputeState::Starting && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(controller.snapshot().state, ComputeState::Starting);
        assert!(controller.snapshot().can_pause);
        // A previous stop must not leave a cancellation permit that consumes
        // the next explicit Resume or Enable without attempting startup.
        for (stop, stopped_state, start) in [
            (
                ComputeCommand::Pause {},
                ComputeState::Paused,
                ComputeCommand::Resume {},
            ),
            (
                ComputeCommand::Disable {},
                ComputeState::Disabled,
                ComputeCommand::Enable {
                    ram_allowance_gib: 1,
                },
            ),
        ] {
            controller.command(stop);
            let deadline = Instant::now() + Duration::from_secs(8);
            while (controller.snapshot().state != stopped_state
                || controller.snapshot().command_pending)
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(controller.snapshot().state, stopped_state);
            assert!(!controller.snapshot().command_pending);
            crate::compute::resource::tests::healthy(&controller);
            controller.command(start);
            let deadline = Instant::now() + Duration::from_secs(3);
            while controller.snapshot().state != ComputeState::Starting && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(controller.snapshot().state, ComputeState::Starting);
        }
        for _ in 0..30 {
            controller.command(ComputeCommand::Resume {});
        }
        assert_eq!(controller.snapshot().reason, "command-busy");
        controller.command(ComputeCommand::Disable {});
        controller.command(ComputeCommand::Pause {});
        let stopped = controller.shutdown(Duration::from_secs(10));
        assert!(stopped.worker_stopped);
        assert!(!stopped.consent_granted);
        assert!(!stopped.command_pending);
        let after = controller.command(ComputeCommand::Enable {
            ram_allowance_gib: 1,
        });
        assert!(after.worker_stopped);
        assert!(!after.consent_granted);
        assert_eq!(after.reason, "controller-shutting-down");
        assert!(
            !ComputeSettingsStore::open(root.path())
                .unwrap()
                .load()
                .unwrap()
                .consent_granted()
        );
    }

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
