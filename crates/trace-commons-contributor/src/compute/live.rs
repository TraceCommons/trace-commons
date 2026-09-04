//! One background actor serializes settings, launch, telemetry and stop. UI
//! snapshots never acquire a process-operation lock or perform network I/O.

use super::{
    ComputeCommand, ComputeSettings, ComputeSettingsStore, ComputeSnapshot, ComputeState,
    LocalWorkerConfig,
    process::{StopReport, WorkerProcess},
    worker_protocol as wire,
};
use std::{
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Notify, mpsc};

struct View {
    snapshot: ComputeSnapshot,
    last_seen: Option<Instant>,
    shutdown_requested: bool,
    shutdown_complete: bool,
}
pub(super) struct LiveController {
    sender: mpsc::Sender<ComputeCommand>,
    view: Arc<Mutex<View>>,
    interrupt: Arc<Notify>,
    cancel_requested: Arc<AtomicBool>,
    requested_stop: Arc<AtomicU8>,
    wake: Arc<Notify>,
    shutdown_done: Arc<Condvar>,
}

impl LiveController {
    pub fn open(
        root: &Path,
        config: LocalWorkerConfig,
        mut snapshot: ComputeSnapshot,
    ) -> anyhow::Result<Self> {
        let worker = WorkerProcess::new(root, config)?;
        let store = ComputeSettingsStore::open(root)?;
        let settings = store.load()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        snapshot.available = true;
        snapshot.local_development = true;
        snapshot.reason = "local-development";
        snapshot.title = "Local compute development";
        snapshot.detail =
            "Explicit local test worker only. No production pool or earnings are enabled.";
        snapshot.can_enable = !settings.consent_granted();
        snapshot.can_resume = settings.consent_granted();
        let view = Arc::new(Mutex::new(View {
            snapshot,
            last_seen: None,
            shutdown_requested: false,
            shutdown_complete: false,
        }));
        let interrupt = Arc::new(Notify::new());
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let requested_stop = Arc::new(AtomicU8::new(0));
        let wake = Arc::new(Notify::new());
        let shutdown_done = Arc::new(Condvar::new());
        let (sender, receiver) = mpsc::channel(1);
        let actor = Actor {
            worker,
            store,
            settings,
            view: view.clone(),
            interrupt: interrupt.clone(),
            cancel_requested: cancel_requested.clone(),
            requested_stop: requested_stop.clone(),
            wake: wake.clone(),
            shutdown_done: shutdown_done.clone(),
            running: false,
        };
        std::thread::Builder::new()
            .name("compute-controller".into())
            .spawn(move || runtime.block_on(actor.run(receiver)))?;
        Ok(Self {
            sender,
            view,
            interrupt,
            cancel_requested,
            requested_stop,
            wake,
            shutdown_done,
        })
    }
    pub fn snapshot(&self) -> ComputeSnapshot {
        let view = self.view.lock().unwrap_or_else(|e| e.into_inner());
        let mut snapshot = view.snapshot.clone();
        snapshot.telemetry_age_ms = view
            .last_seen
            .map(|seen| seen.elapsed().as_millis().min(u64::MAX as u128) as u64);
        if !snapshot.worker_stopped
            && !matches!(
                snapshot.state,
                ComputeState::Starting | ComputeState::Draining | ComputeState::Error
            )
            && snapshot.telemetry_age_ms.is_none_or(|age| age > 6000)
        {
            snapshot.state = ComputeState::Stale;
            snapshot.title = "Compute status is stale";
            snapshot.detail = "The worker has not provided a fresh authenticated status. Work progress is unknown.";
        }
        snapshot
    }
    pub fn command(&self, command: ComputeCommand) -> ComputeSnapshot {
        let interrupts = matches!(
            command,
            ComputeCommand::Pause {} | ComputeCommand::Disable {}
        );
        let mut view = self.view.lock().unwrap_or_else(|e| e.into_inner());
        if view.shutdown_requested {
            view.snapshot.reason = "controller-shutting-down";
        } else if interrupts {
            // Bits coalesce without losing revocation when a later Pause or
            // Shutdown arrives: bit 1 stop, bit 2 revoke, bit 4 shutdown.
            self.requested_stop.fetch_or(
                if matches!(command, ComputeCommand::Disable {}) {
                    3
                } else {
                    1
                },
                Ordering::AcqRel,
            );
            view.snapshot.command_pending = true;
            view.snapshot.can_resume = false;
            view.snapshot.can_enable = false;
            self.cancel_requested.store(true, Ordering::Release);
            self.interrupt.notify_waiters();
            self.wake.notify_one();
        } else if view.snapshot.command_pending {
            view.snapshot.reason = "command-busy";
        } else {
            view.snapshot.command_pending = true;
            if self.sender.try_send(command).is_err() {
                view.snapshot.command_pending = false;
                view.snapshot.reason = "command-queue-unavailable";
            } else {
                view.snapshot.can_enable = false;
                view.snapshot.can_resume = false;
            }
        }
        drop(view);
        self.snapshot()
    }
    pub fn shutdown(&self, timeout: Duration) -> ComputeSnapshot {
        let mut view = self.view.lock().unwrap_or_else(|e| e.into_inner());
        view.shutdown_requested = true;
        view.shutdown_complete = false;
        view.snapshot.can_enable = false;
        view.snapshot.can_resume = false;
        view.snapshot.command_pending = true;
        self.cancel_requested.store(true, Ordering::Release);
        self.requested_stop.fetch_or(5, Ordering::AcqRel);
        self.interrupt.notify_waiters();
        self.wake.notify_one();
        let (view, _) = self
            .shutdown_done
            .wait_timeout_while(view, timeout.min(Duration::from_secs(30)), |view| {
                !view.shutdown_complete
            })
            .unwrap_or_else(|e| e.into_inner());
        let completed = view.shutdown_complete;
        drop(view);
        let mut snapshot = self.snapshot();
        if !completed {
            snapshot.worker_stopped = false;
            snapshot.command_pending = true;
            snapshot.reason = "shutdown-pending";
        }
        snapshot
    }
}

struct Actor {
    worker: WorkerProcess,
    store: ComputeSettingsStore,
    settings: ComputeSettings,
    view: Arc<Mutex<View>>,
    interrupt: Arc<Notify>,
    cancel_requested: Arc<AtomicBool>,
    requested_stop: Arc<AtomicU8>,
    wake: Arc<Notify>,
    shutdown_done: Arc<Condvar>,
    running: bool,
}

impl Drop for LiveController {
    fn drop(&mut self) {
        self.requested_stop.fetch_or(5, Ordering::AcqRel);
        self.cancel_requested.store(true, Ordering::Release);
        self.interrupt.notify_waiters();
        self.wake.notify_one();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn aged_telemetry_becomes_stale_without_claiming_progress() {
        let root = tempfile::tempdir().unwrap();
        let initial = super::super::ComputeController::open(root.path())
            .unwrap()
            .snapshot();
        let live = LiveController::open(
            root.path(),
            LocalWorkerConfig {
                binary: "/usr/bin/true".into(),
                expected_sha256: "00".repeat(32),
                coordinator: "ws://127.0.0.1:9999".into(),
                startup_timeout_secs: 1,
            },
            initial,
        )
        .unwrap();
        {
            let mut view = live.view.lock().unwrap();
            view.snapshot.state = ComputeState::Serving;
            view.snapshot.worker_stopped = false;
            view.last_seen = Some(Instant::now() - Duration::from_secs(7));
        }
        let stale = live.snapshot();
        assert_eq!(stale.state, ComputeState::Stale);
        assert!(stale.telemetry_age_ms.unwrap() >= 7000);
        assert!(stale.detail.contains("progress is unknown"));
    }
}
impl Actor {
    fn update(&self, f: impl FnOnce(&mut View)) {
        f(&mut self.view.lock().unwrap_or_else(|e| e.into_inner()));
    }
    fn message(
        &self,
        state: ComputeState,
        reason: &'static str,
        title: &'static str,
        detail: &'static str,
    ) {
        self.update(|view| {
            view.snapshot.state = state;
            view.snapshot.reason = reason;
            view.snapshot.title = title;
            view.snapshot.detail = detail;
        });
    }
    fn controls(&self) {
        self.update(|view| {
            view.snapshot.consent_granted = self.settings.consent_granted();
            view.snapshot.ram_allowance_gib = self.settings.ram_allowance_gib();
            view.snapshot.can_enable =
                !view.shutdown_requested && !self.settings.consent_granted() && !self.running;
            view.snapshot.can_resume =
                !view.shutdown_requested && self.settings.consent_granted() && !self.running;
            view.snapshot.can_pause = self.running;
            view.snapshot.command_pending = self.requested_stop.load(Ordering::Acquire) != 0;
        });
    }
    fn observe(&self, status: wire::Status) {
        let state = match (status.state, status.admission) {
            (wire::State::Training, wire::Admission::Assigned) => ComputeState::Training,
            (wire::State::Serving, wire::Admission::Assigned) => ComputeState::Serving,
            (wire::State::Draining, _) => ComputeState::Draining,
            _ => ComputeState::Waiting,
        };
        self.message(state, "authenticated-status", match state {
            ComputeState::Training => "Worker reports training", ComputeState::Serving => "Worker reports serving",
            ComputeState::Draining => "Compute is draining", _ => "Waiting for compute work",
        }, "A fresh signed worker response confirms endpoint liveness and reported assignment, not workload progress or a hard memory limit.");
        self.update(|view| {
            view.last_seen = Some(Instant::now());
            view.snapshot.admission = Some(status.admission);
            view.snapshot.drain_outcome = Some(status.drain);
            view.snapshot.worker_stopped = false;
        });
    }
    async fn stop(&mut self) -> StopReport {
        self.message(
            ComputeState::Draining,
            "draining",
            "Stopping compute",
            "Waiting for worker termination. Coordinator acknowledgement is tracked separately.",
        );
        let report = self.worker.stop().await;
        self.running = !report.stopped;
        self.update(|view| {
            view.last_seen = None;
            view.snapshot.worker_stopped = report.stopped;
            if report.process != super::StopOutcome::NotRunning
                || view.snapshot.stop_outcome.is_none()
            {
                view.snapshot.stop_outcome = Some(report.process);
                view.snapshot.drain_outcome = report.drain;
            }
            view.snapshot.admission = None;
        });
        if report.stopped {
            self.message(if self.settings.consent_granted() { ComputeState::Paused } else { ComputeState::Disabled },
                "worker-stopped", "Compute is stopped", "The owned worker has exited. An acknowledged drain is shown only when received from the authenticated worker.");
        } else {
            self.message(
                ComputeState::Error,
                "worker-stop-failed",
                "Compute could not be stopped",
                "The app still owns the worker. Keep the app open and retry stopping compute.",
            );
        }
        report
    }
    async fn execute(&mut self, command: ComputeCommand) {
        match command {
            ComputeCommand::Enable { ram_allowance_gib } => {
                if self.running {
                    return;
                }
                match ComputeSettings::grant(ram_allowance_gib) {
                    Ok(settings) if self.store.save(&settings).is_ok() => self.settings = settings,
                    _ => {
                        self.message(
                            ComputeState::Error,
                            "settings-invalid-or-unsaved",
                            "Compute settings could not be saved",
                            "Check the RAM scheduling allowance and try again.",
                        );
                        return;
                    }
                }
                self.start().await;
            }
            ComputeCommand::Resume {} => {
                if !self.settings.consent_granted() {
                    self.message(
                        ComputeState::Disabled,
                        "consent-required",
                        "Compute is disabled",
                        "Enable compute with separate consent before resuming.",
                    );
                } else if !self.running {
                    self.start().await;
                }
            }
            ComputeCommand::Pause {} => {
                self.stop().await;
                self.cancel_requested.store(false, Ordering::Release);
            }
            ComputeCommand::Disable {} => {
                let report = self.stop().await;
                self.cancel_requested.store(false, Ordering::Release);
                let mut revoked = self.settings.clone();
                revoked.revoke();
                if self.store.save(&revoked).is_ok() {
                    self.settings = revoked;
                    if report.stopped {
                        self.message(
                            ComputeState::Disabled,
                            "consent-revoked",
                            "Compute is disabled",
                            "Compute consent has been revoked.",
                        );
                    }
                } else {
                    self.message(
                        ComputeState::Error,
                        "settings-write-failed",
                        "Compute settings could not be saved",
                        "The previous consent setting remains. Retry disabling compute.",
                    );
                }
            }
        }
    }
    async fn start(&mut self) {
        let interrupt = self.interrupt.clone();
        let notified = interrupt.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.cancel_requested.swap(false, Ordering::AcqRel)
            || self
                .view
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .shutdown_requested
        {
            return;
        }
        self.message(
            ComputeState::Starting,
            "starting",
            "Starting local compute",
            "Verifying the worker and waiting for authenticated readiness.",
        );
        self.update(|view| {
            view.last_seen = None;
            view.snapshot.worker_stopped = false;
            view.snapshot.drain_outcome = None;
            view.snapshot.stop_outcome = None;
            view.snapshot.can_pause = true;
            view.snapshot.consent_granted = self.settings.consent_granted();
            view.snapshot.ram_allowance_gib = self.settings.ram_allowance_gib();
        });
        // A pause/shutdown can cancel readiness, but the worker object retains
        // its child and lock until stop() confirms it has been reaped.
        let allowance = self.settings.ram_allowance_gib().unwrap_or(0);
        let result = tokio::select! {
            result = self.worker.start(allowance) => result,
            _ = &mut notified => Err(anyhow::anyhow!("start-interrupted")),
        };
        match result {
            Ok(status) => {
                self.running = true;
                self.observe(status);
            }
            Err(_) => {
                let report = self.stop().await;
                if report.stopped {
                    self.message(ComputeState::Error, "worker-start-failed", "Compute could not start", "The local worker failed verification or authenticated readiness. It has been stopped; check the local test setup before resuming.");
                }
            }
        }
    }
    async fn run(mut self, mut receiver: mpsc::Receiver<ComputeCommand>) {
        let mut interval = tokio::time::interval(Duration::from_secs(3));
        let mut failures = 0;
        loop {
            let stop = self.requested_stop.swap(0, Ordering::AcqRel);
            if stop != 0 {
                while receiver.try_recv().is_ok() {}
                self.execute(if stop & 2 != 0 {
                    ComputeCommand::Disable {}
                } else {
                    ComputeCommand::Pause {}
                })
                .await;
                self.controls();
                if stop & 4 != 0 {
                    self.update(|view| view.shutdown_complete = true);
                    self.shutdown_done.notify_all();
                }
                continue;
            }
            tokio::select! {
                biased;
                _ = self.wake.notified() => { continue; }
                message = receiver.recv() => {
                    match message {
                        Some(command) => {
                            if self.requested_stop.load(Ordering::Acquire) == 0 { self.execute(command).await; }
                        }
                        None => { self.stop().await; break; }
                    }
                    failures = 0;
                    self.controls();
                }
                _ = interval.tick(), if self.running => {
                    match self.worker.status().await {
                        Ok(status) => { failures = 0; self.observe(status); }
                        Err(_) => {
                            failures += 1;
                            self.message(ComputeState::Stale, "worker-status-unavailable", "Compute status is stale", "A fresh authenticated status is unavailable. Work progress is unknown.");
                            if failures >= 3 {
                                let report = self.stop().await;
                                if report.stopped { self.message(ComputeState::Error, "worker-status-lost", "Compute stopped after losing status", "Resume explicitly after checking the local worker setup."); }
                                self.controls();
                            }
                        }
                    }
                }
            }
        }
    }
}
