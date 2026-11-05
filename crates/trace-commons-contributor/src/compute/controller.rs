//! Serialized consent controller. Worker availability stays closed until a
//! pinned, authenticated supervisor adapter is installed. No transport or trace
//! discovery is performed by this module.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::presentation::{self as prose, Message};
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
    /// Additive presentation identities, independent of transient state/reason.
    pub title_key: &'static str,
    pub detail_key: &'static str,
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

impl ComputeSnapshot {
    pub(super) fn set_title(&mut self, message: Message) {
        self.title = message.english;
        self.title_key = message.key;
    }

    pub(super) fn set_detail(&mut self, message: Message) {
        self.detail = message.english;
        self.detail_key = message.key;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ComputeCopy {
    pub destination: &'static str,
    pub subtitle: &'static str,
    pub retry: &'static str,
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
            subtitle: "Contribute compute independently of your traces.",
            retry: "Try again",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P311).
            introduction: "Lend this computer's compute to Holonear. It gets no access to your traces, and the pilot promises no pay.",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P338).
            allowance_label: "RAM allowance (GiB)",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P317).
            allowance_detail: "Not a hard limit; actual memory use may differ.",
            enable: "Enable compute",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P339).
            resume: "Resume",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P340).
            pause: "Pause",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P341).
            disable: "Disable",
            // #1146 (`settings-modal.tsx`, `use-compute-status.ts`).
            loading: "Reading compute settings…",
            unavailable: "Compute settings unavailable.",
            quit_detail: "Enabled compute must stop before the app quits. Worker termination and coordinator drain acknowledgement are separate; a forced exit does not confirm a completed handoff.",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P305).
            quit_refused: "Quit cancelled: compute hasn't stopped yet. Quit again once it has.",
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

    /// Explicit local-only development entrypoint. Builds without debug assertions refuse it;
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
                prose::ALLOWANCE,
                // Approved by the owner 2026-11-04 (Settings copy pass 2, P345).
                prose::INVALID_ALLOWANCE,
            ),
            "consent-required" => (
                prose::DISABLED,
                // Approved by the owner 2026-11-04 (Settings copy pass 2, P309).
                // Cut: the shells draw this detail unconditionally, so it renders as an empty line.
                prose::EMPTY,
            ),
            "settings-write-failed" => (
                // Approved by the owner 2026-11-04 (Settings copy pass 2, P330).
                prose::SAVE_FAILED,
                // Approved by the owner 2026-11-04 (Settings copy pass 2, P314).
                prose::NOTHING_CHANGED,
            ),
            _ => match self.state {
                ComputeState::Paused => (
                    prose::PAUSED,
                    // Approved by the owner 2026-11-04 (Settings copy pass 2, P315).
                    prose::NO_AUTO_RESUME,
                ),
                ComputeState::Disabled => (
                    prose::DISABLED,
                    // Approved by the owner 2026-11-04 (Settings copy pass 2, P316).
                    prose::NOT_AVAILABLE,
                ),
                _ => (
                    prose::UNAVAILABLE,
                    // Approved by the owner 2026-11-04 (Settings copy pass 2, P316).
                    prose::NOT_AVAILABLE,
                ),
            },
        };
        ComputeSnapshot {
            schema: "trace_commons.compute_status.v1",
            state: self.state,
            reason: self.reason,
            title: title.english,
            detail: detail.english,
            title_key: title.key,
            detail_key: detail.key,
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
        title: prose::STATUS_UNAVAILABLE.english,
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P331).
        detail: prose::RESTART.english,
        title_key: prose::STATUS_UNAVAILABLE.key,
        detail_key: prose::RESTART.key,
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
        prose::assert_snapshot(&controller.snapshot());
        assert_eq!(controller.snapshot().title_key, prose::STARTING.key);
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
            prose::assert_snapshot(&controller.snapshot());
            assert!(!controller.snapshot().command_pending);
            crate::compute::resource::tests::healthy(&controller);
            controller.command(start);
            let deadline = Instant::now() + Duration::from_secs(3);
            while controller.snapshot().state != ComputeState::Starting && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(controller.snapshot().state, ComputeState::Starting);
            prose::assert_snapshot(&controller.snapshot());
        }
        let starting = controller.snapshot();
        for _ in 0..30 {
            controller.command(ComputeCommand::Resume {});
        }
        let busy = controller.snapshot();
        assert_eq!(busy.reason, "command-busy");
        prose::assert_snapshot(&busy);
        assert_eq!(
            (busy.title_key, busy.detail_key),
            (starting.title_key, starting.detail_key)
        );
        assert_eq!((busy.title, busy.detail), (starting.title, starting.detail));
        controller.command(ComputeCommand::Disable {});
        controller.command(ComputeCommand::Pause {});
        let stopped = controller.shutdown(Duration::from_secs(10));
        prose::assert_snapshot(&stopped);
        assert!(stopped.worker_stopped);
        assert!(!stopped.consent_granted);
        assert!(!stopped.command_pending);
        let after = controller.command(ComputeCommand::Enable {
            ram_allowance_gib: 1,
        });
        assert!(after.worker_stopped);
        assert!(!after.consent_granted);
        assert_eq!(after.reason, "controller-shutting-down");
        prose::assert_snapshot(&after);
        assert_eq!(after.title_key, stopped.title_key);
        // The shutdown snapshot overlays resource-policy detail. A refused
        // command changes the reason and exposes the actor's retained detail;
        // each identity must describe its own snapshot, not the earlier one.
        assert_eq!(
            stopped.detail_key,
            prose::policy(super::super::policy::PolicyReason::Shutdown).key
        );
        assert!(
            !ComputeSettingsStore::open(root.path())
                .unwrap()
                .load()
                .unwrap()
                .consent_granted()
        );
    }

    #[test]
    fn presentation_identities_follow_controller_messages_not_reason_spelling() {
        let root = tempfile::tempdir().unwrap();
        let controller = ComputeController::open(root.path()).unwrap();
        let initial = controller.snapshot();
        prose::assert_snapshot(&initial);
        assert_eq!(initial.title_key, prose::DISABLED.key);
        for (command, title, detail) in [
            (
                ComputeCommand::Enable {
                    ram_allowance_gib: 0,
                },
                prose::ALLOWANCE,
                prose::INVALID_ALLOWANCE,
            ),
            (ComputeCommand::Resume {}, prose::DISABLED, prose::EMPTY),
            (
                ComputeCommand::Enable {
                    ram_allowance_gib: 8,
                },
                prose::UNAVAILABLE,
                prose::NOT_AVAILABLE,
            ),
            (
                ComputeCommand::Pause {},
                prose::DISABLED,
                prose::NOT_AVAILABLE,
            ),
            (
                ComputeCommand::Disable {},
                prose::DISABLED,
                prose::NOT_AVAILABLE,
            ),
        ] {
            let snapshot = controller.command(command);
            prose::assert_snapshot(&snapshot);
            assert_eq!(
                (snapshot.title_key, snapshot.detail_key),
                (title.key, detail.key)
            );
            assert_eq!(
                (snapshot.title, snapshot.detail),
                (title.english, detail.english)
            );
            let wire = serde_json::to_value(snapshot).unwrap();
            assert_eq!(wire["title_key"], title.key);
            assert_eq!(wire["detail_key"], detail.key);
        }
        prose::assert_snapshot(&error_snapshot());
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
        prose::assert_snapshot(&result);
        assert_eq!(result.title_key, prose::SAVE_FAILED.key);
        assert_eq!(result.detail_key, prose::NOTHING_CHANGED.key);
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
