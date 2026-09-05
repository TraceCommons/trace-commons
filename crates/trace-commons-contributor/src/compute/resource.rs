//! Bounded observation ingress. Native code obtains a single-use ticket BEFORE
//! reading every field; Rust owns the capture timestamp and wake generation.
//! This trusted in-process adapter seam is not remote platform attestation.

use super::policy::{
    Decision, MemoryPressure, Observation, PowerSource, ResourcePolicy, StopUrgency, ThermalState,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Instant,
};
use tokio::sync::{Notify, watch};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceTicket {
    pub epoch: u64,
    pub sequence: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceReading {
    pub power: PowerSource,
    pub low_power_mode: Option<bool>,
    pub thermal: ThermalState,
    pub memory: MemoryPressure,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::compute::{ComputeCommand, ComputeController, ComputeState, LocalWorkerConfig};
    use std::time::Duration;

    fn reading() -> ResourceReading {
        ResourceReading {
            power: PowerSource::Ac,
            low_power_mode: Some(false),
            thermal: ThermalState::Nominal,
            memory: MemoryPressure::Normal,
        }
    }

    #[cfg(unix)]
    pub fn healthy(controller: &ComputeController) {
        let ticket = controller.resource_begin().unwrap();
        assert!(controller.resource_event(ResourceEvent::Sample {
            ticket,
            reading: reading()
        }));
    }

    fn channel() -> ResourceChannel {
        ResourceChannel::new(
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Notify::new()),
            Arc::new(Notify::new()),
        )
    }

    #[test]
    fn controller_preserves_ffi_unwind_safety_with_unified_features() {
        fn check<T: std::panic::RefUnwindSafe + std::panic::UnwindSafe>() {}
        check::<crate::compute::ComputeController>();
    }

    #[test]
    fn tickets_are_single_use_replaced_and_invalidated_on_wake() {
        let channel = channel();
        let old = channel.begin().unwrap();
        let next = channel.begin().unwrap();
        assert!(!channel.event(ResourceEvent::Sample {
            ticket: old,
            reading: reading()
        }));
        assert!(channel.event(ResourceEvent::Sample {
            ticket: next,
            reading: reading()
        }));
        assert!(!channel.event(ResourceEvent::Sample {
            ticket: next,
            reading: reading()
        }));
        let old = channel.begin().unwrap();
        channel.event(ResourceEvent::Wake {});
        assert!(!channel.event(ResourceEvent::Sample {
            ticket: old,
            reading: reading()
        }));
        assert!(!channel.ready());
    }

    #[test]
    fn delayed_ticket_cannot_create_a_fresh_lease() {
        let channel = channel();
        let ticket = channel.begin().unwrap();
        channel.state.lock().unwrap().pending.as_mut().unwrap().1 =
            Instant::now() - Duration::from_secs(7);
        assert!(channel.event(ResourceEvent::Sample {
            ticket,
            reading: reading()
        }));
        assert!(!channel.request_run(true).may_run);
    }

    #[tokio::test]
    async fn pressure_between_enqueue_and_spawn_refuses_spawn_and_latches_urgency() {
        let channel = channel();
        let ticket = channel.begin().unwrap();
        channel.event(ResourceEvent::Sample {
            ticket,
            reading: reading(),
        });
        assert!(channel.request_run(true).may_run);
        let ticket = channel.begin().unwrap();
        channel.event(ResourceEvent::Sample {
            ticket,
            reading: ResourceReading {
                memory: MemoryPressure::Critical,
                ..reading()
            },
        });
        let urgent_at = *channel.urgency().borrow();
        assert!(urgent_at.is_some());
        let ticket = channel.begin().unwrap();
        channel.event(ResourceEvent::Sample {
            ticket,
            reading: reading(),
        });
        assert!(
            channel
                .spawn(&mut tokio::process::Command::new("/usr/bin/true"))
                .is_err()
        );
        assert_eq!(*channel.urgency().borrow(), urgent_at);
        assert!(!channel.request_run(false).may_run);
        channel.stopped();
        assert!(!channel.cancel.load(Ordering::Acquire));
        assert!(!channel.decision().may_run);
        assert!(channel.request_run(false).may_run);
    }

    #[test]
    fn strict_resource_json_rejects_unknown_fields_and_labels() {
        for json in [
            r#"{"event":"wake","allow_run":true}"#,
            r#"{"event":"sample","ticket":{"epoch":0,"sequence":1},"reading":{"power":"solar","thermal":"nominal","memory":"normal"}}"#,
        ] {
            assert!(serde_json::from_str::<ResourceEvent>(json).is_err());
        }
    }

    #[cfg(unix)]
    fn local(root: &std::path::Path) -> ComputeController {
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::PermissionsExt;
        let binary = root.join("sleepy-worker");
        let script = b"#!/bin/sh\nexec /bin/sleep 60\n";
        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        ComputeController::open_local(
            root,
            LocalWorkerConfig {
                binary,
                expected_sha256: hex::encode(Sha256::digest(script)),
                coordinator: "ws://127.0.0.1:9999".into(),
                startup_timeout_secs: 30,
            },
        )
        .unwrap()
    }

    #[cfg(unix)]
    fn wait_for(
        controller: &ComputeController,
        timeout: u64,
        predicate: impl Fn(&crate::compute::ComputeSnapshot) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(timeout);
        while Instant::now() < deadline {
            if predicate(&controller.snapshot()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "compute lifecycle deadline exceeded: {:?}",
            controller.snapshot().state
        );
    }

    #[cfg(unix)]
    #[test]
    fn native_pressure_interrupts_readiness_and_recovery_needs_resume() {
        let root = tempfile::tempdir().unwrap();
        let controller = local(root.path());
        controller.command(ComputeCommand::Enable {
            ram_allowance_gib: 1,
        });
        assert!(controller.snapshot().worker_stopped);
        assert!(!controller.snapshot().consent_granted);
        healthy(&controller);
        controller.command(ComputeCommand::Enable {
            ram_allowance_gib: 1,
        });
        wait_for(&controller, 3, |s| s.state == ComputeState::Starting);
        let ticket = controller.resource_begin().unwrap();
        controller.resource_event(ResourceEvent::Sample {
            ticket,
            reading: ResourceReading {
                memory: MemoryPressure::Critical,
                ..reading()
            },
        });
        // Recovery cannot remove the stop, even during the readiness operation.
        healthy(&controller);
        wait_for(&controller, 5, |s| s.worker_stopped && !s.command_pending);
        assert!(controller.snapshot().consent_granted);
        assert_ne!(controller.snapshot().state, ComputeState::Starting);
        controller.command(ComputeCommand::Resume {});
        wait_for(&controller, 3, |s| s.state == ComputeState::Starting);
        assert!(controller.shutdown(Duration::from_secs(8)).worker_stopped);
    }

    #[cfg(unix)]
    #[test]
    fn watchdog_stops_readiness_without_any_native_callbacks() {
        let root = tempfile::tempdir().unwrap();
        let controller = local(root.path());
        healthy(&controller);
        controller.command(ComputeCommand::Enable {
            ram_allowance_gib: 1,
        });
        wait_for(&controller, 3, |s| s.state == ComputeState::Starting);
        // No status()/snapshot() call while lease expires: watchdog must act.
        std::thread::sleep(Duration::from_secs(10));
        assert!(controller.snapshot().worker_stopped);
        assert!(controller.snapshot().consent_granted);
        assert!(!controller.snapshot().can_resume);
        assert!(controller.shutdown(Duration::from_secs(3)).worker_stopped);
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceEvent {
    Sample {
        ticket: ResourceTicket,
        reading: ResourceReading,
    },
    Sleep {},
    Wake {},
}

struct State {
    policy: ResourcePolicy,
    sequence: u64,
    pending: Option<(ResourceTicket, Instant)>,
    emitted: bool,
    // Keep the watch sender behind the same poison-aware mutex. Tokio's
    // parking_lot feature must not change the FFI handle's unwind-safety bound.
    urgency: watch::Sender<Option<Instant>>,
}

pub(super) struct ResourceChannel {
    state: Mutex<State>,
    cancel: Arc<AtomicBool>,
    stop: Arc<AtomicU8>,
    interrupt: Arc<Notify>,
    wake: Arc<Notify>,
}

impl ResourceChannel {
    pub fn new(
        cancel: Arc<AtomicBool>,
        stop: Arc<AtomicU8>,
        interrupt: Arc<Notify>,
        wake: Arc<Notify>,
    ) -> Self {
        Self {
            state: Mutex::new(State {
                policy: ResourcePolicy::default(),
                sequence: 0,
                pending: None,
                emitted: false,
                urgency: watch::channel(None).0,
            }),
            cancel,
            stop,
            interrupt,
            wake,
        }
    }

    fn update<T>(&self, f: impl FnOnce(&mut State, Instant) -> T) -> T {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let result = f(&mut state, now);
        self.publish(&mut state, now);
        result
    }

    fn publish(&self, state: &mut State, now: Instant) {
        let decision = state.policy.evaluate(now);
        if let Some(request) = decision.stop {
            if request.urgency == StopUrgency::Urgent && state.urgency.borrow().is_none() {
                state.urgency.send_replace(Some(now));
            }
            if !state.emitted {
                state.emitted = true;
                self.stop.fetch_or(8, Ordering::AcqRel);
                self.cancel.store(true, Ordering::Release);
                self.interrupt.notify_waiters();
                self.wake.notify_one();
            }
        }
    }

    pub fn begin(&self) -> Option<ResourceTicket> {
        self.update(|state, now| {
            state.sequence = state.sequence.checked_add(1)?;
            let ticket = ResourceTicket {
                epoch: state.policy.epoch(),
                sequence: state.sequence,
            };
            state.pending = Some((ticket, now));
            Some(ticket)
        })
    }

    pub fn urgency(&self) -> watch::Receiver<Option<Instant>> {
        self.update(|state, _| state.urgency.subscribe())
    }

    pub fn event(&self, event: ResourceEvent) -> bool {
        self.update(|state, now| match event {
            ResourceEvent::Sample { ticket, reading } => {
                let Some((expected, observed_at)) = state.pending else {
                    return false;
                };
                if ticket != expected {
                    return false;
                }
                state.pending = None;
                state.policy.observe(
                    Observation {
                        epoch: ticket.epoch,
                        sequence: ticket.sequence,
                        observed_at,
                        power: reading.power,
                        low_power_mode: reading.low_power_mode,
                        thermal: reading.thermal,
                        memory: reading.memory,
                    },
                    now,
                );
                true
            }
            ResourceEvent::Sleep {} => {
                state.pending = None;
                state.policy.sleep(now);
                true
            }
            ResourceEvent::Wake {} => {
                state.pending = None;
                state.policy.wake(now);
                true
            }
        })
    }

    pub fn decision(&self) -> Decision {
        self.update(|state, now| state.policy.evaluate(now))
    }
    pub fn epoch(&self) -> u64 {
        self.update(|state, _| state.policy.epoch())
    }
    pub fn ready(&self) -> bool {
        self.update(|state, now| state.policy.ready(now))
    }
    pub fn request_run(&self, enable: bool) -> Decision {
        self.update(|state, now| state.policy.request_run(enable, now))
    }
    pub fn pause(&self) {
        self.update(|state, now| {
            state.policy.pause(now);
        });
    }
    pub fn disable(&self) {
        self.update(|state, now| {
            state.policy.disable(now);
        });
    }
    pub fn shutdown(&self) {
        self.update(|state, now| {
            state.policy.shutdown(now);
        });
    }

    pub fn stopped(&self) {
        self.update(|state, now| {
            state.policy.confirm_stopped(now);
            state.emitted = false;
            self.stop.fetch_and(!8, Ordering::AcqRel);
            self.cancel
                .store(self.stop.load(Ordering::Acquire) != 0, Ordering::Release);
            state.urgency.send_replace(None);
        });
    }

    /// Linearizes native updates/manual stops and the final check with spawn.
    /// Hashing and filesystem preparation happen before entering this lock.
    pub fn spawn(
        &self,
        command: &mut tokio::process::Command,
    ) -> std::io::Result<tokio::process::Child> {
        self.update(|state, now| {
            if !state.policy.evaluate(now).may_run || self.stop.load(Ordering::Acquire) != 0 {
                return Err(std::io::Error::other("resource-launch-refused"));
            }
            command.spawn()
        })
    }
}
