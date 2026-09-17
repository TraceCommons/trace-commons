use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tauri::State;
use trace_commons_contributor::{
    compute::{ComputeCommand, ComputeController},
    daemon::{
        EmbeddedDaemon,
        attached::AttachedDaemon,
        ipc::{DaemonShared, Response},
    },
};

#[derive(Clone)]
pub(crate) enum DaemonConnection {
    Embedded(Arc<DaemonShared>),
    #[cfg(unix)]
    Attached(Arc<AttachedDaemon>),
}

pub(crate) struct Runtime {
    state_dir: PathBuf,
    connection: Option<DaemonConnection>,
    compute: Arc<ComputeController>,
    daemon: Option<EmbeddedDaemon>,
}

impl Runtime {
    pub(crate) fn new(
        state_dir: PathBuf,
        connection: Option<DaemonConnection>,
        compute: Arc<ComputeController>,
        daemon: Option<EmbeddedDaemon>,
    ) -> Self {
        Self {
            state_dir,
            connection,
            compute,
            daemon,
        }
    }

    fn shutdown(&mut self) {
        self.compute.shutdown(Duration::from_secs(1));
        if let Some(connection) = self.connection.take() {
            match connection {
                DaemonConnection::Embedded(shared) => {
                    shared.shutdown.store(true, Ordering::Release);
                    shared.shutdown_signal.notify_one();
                }
                #[cfg(unix)]
                DaemonConnection::Attached(daemon) => daemon.close(),
            }
        }
        if let Some(daemon) = self.daemon.take() {
            daemon.close();
        }
    }
}

pub(crate) struct AppState {
    runtime: Mutex<Option<Runtime>>,
    pending_deep_link: Mutex<Option<String>>,
    event_stop: Arc<AtomicBool>,
    exit_authorized: AtomicBool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            runtime: Mutex::new(None),
            pending_deep_link: Mutex::new(None),
            event_stop: Arc::new(AtomicBool::new(false)),
            exit_authorized: AtomicBool::new(false),
        }
    }
}

impl AppState {
    pub(crate) fn install(&self, runtime: Runtime) -> Result<(), ()> {
        let mut slot = self.runtime.lock().map_err(|_| ())?;
        *slot = Some(runtime);
        Ok(())
    }

    pub(crate) fn daemon(&self) -> Result<DaemonConnection, String> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "application state lock poisoned".to_owned())?;
        Ok(runtime
            .as_ref()
            .and_then(|runtime| runtime.connection.clone())
            .ok_or_else(|| "Rust core daemon is not started; choose source roots".to_owned())?)
    }

    pub(crate) fn optional_daemon(&self) -> Result<Option<DaemonConnection>, String> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "application state lock poisoned".to_owned())?;
        Ok(runtime
            .as_ref()
            .and_then(|runtime| runtime.connection.clone()))
    }

    pub(crate) fn state_directory(&self) -> Result<PathBuf, String> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "application state lock poisoned".to_owned())?;
        runtime
            .as_ref()
            .map(|runtime| runtime.state_dir.clone())
            .ok_or_else(|| "Rust core is not started".to_owned())
    }

    pub(crate) fn attach_embedded_daemon(
        &self,
        shared: Arc<DaemonShared>,
        daemon: EmbeddedDaemon,
    ) -> Result<(), String> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "application state lock poisoned".to_owned())?;
        let runtime = runtime
            .as_mut()
            .ok_or_else(|| "Rust core is not started".to_owned())?;
        if runtime.connection.is_none() {
            runtime.connection = Some(DaemonConnection::Embedded(shared));
            runtime.daemon = Some(daemon);
        }
        Ok(())
    }

    #[cfg(unix)]
    pub(crate) fn attach_external_daemon(&self, daemon: Arc<AttachedDaemon>) -> Result<(), String> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "application state lock poisoned".to_owned())?;
        let runtime = runtime
            .as_mut()
            .ok_or_else(|| "Rust core is not started".to_owned())?;
        if runtime.connection.is_none() {
            runtime.connection = Some(DaemonConnection::Attached(daemon));
        }
        Ok(())
    }

    pub(crate) fn event_stop(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.event_stop)
    }

    pub(crate) fn reset_event_stop(&self) {
        self.event_stop.store(false, Ordering::Release);
    }

    fn response_value(response: Response) -> Result<serde_json::Value, String> {
        match (response.result, response.error) {
            (Some(result), None) => Ok(result),
            (None, Some(error)) => Err(format!("{}: {}", error.code, error.message)),
            _ => Err("Rust core returned an invalid IPC response".to_owned()),
        }
    }

    fn daemon_value(
        connection: Option<DaemonConnection>,
    ) -> Result<Option<serde_json::Value>, String> {
        let Some(connection) = connection else {
            return Ok(None);
        };
        match connection {
            DaemonConnection::Embedded(shared) => Ok(Some(shared.status_value())),
            #[cfg(unix)]
            DaemonConnection::Attached(daemon) => Ok(Some(Self::response_value(
                daemon
                    .call("status", &serde_json::json!({}))
                    .map_err(|error| error.to_string())?,
            )?)),
        }
    }

    fn core_status(&self) -> Result<serde_json::Value, String> {
        let guard = self
            .runtime
            .lock()
            .map_err(|_| "application state lock poisoned".to_owned())?;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| "Rust core is not started".to_owned())?;
        let state_dir = runtime.state_dir.clone();
        let connection = runtime.connection.clone();
        drop(guard);
        let daemon = Self::daemon_value(connection.clone())?.unwrap_or_else(|| {
            serde_json::json!({
                "schema_version": "not-started",
                "logged_in": false,
                "tenant_id": null,
                "consent_scopes": [],
                "paused": false,
                "queue_depth": 0,
                "health": { "last_error_label": null, "since": null },
            })
        });
        let startup = if connection.is_some() {
            "running"
        } else {
            "needs_roots"
        };

        Ok(serde_json::json!({
            "prototype": true,
            "state_dir": state_dir,
            "startup": startup,
            "daemon": daemon,
        }))
    }

    pub(crate) fn compute_status(&self) -> Result<serde_json::Value, String> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "application state lock poisoned".to_owned())?;
        let runtime = runtime
            .as_ref()
            .ok_or_else(|| "Rust core is not started".to_owned())?;
        serde_json::to_value(runtime.compute.snapshot())
            .map_err(|_| "compute-status-invalid".to_owned())
    }

    pub(crate) fn compute_command(
        &self,
        command: ComputeCommand,
    ) -> Result<serde_json::Value, String> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "application state lock poisoned".to_owned())?;
        let runtime = runtime
            .as_ref()
            .ok_or_else(|| "Rust core is not started".to_owned())?;
        serde_json::to_value(runtime.compute.command(command))
            .map_err(|_| "compute-status-invalid".to_owned())
    }

    pub(crate) fn event_stopped(&self) -> bool {
        self.event_stop.load(Ordering::Acquire)
    }

    pub(crate) fn authorize_exit(&self) {
        self.exit_authorized.store(true, Ordering::Release);
    }

    pub(crate) fn consume_authorized_exit(&self) -> bool {
        self.exit_authorized.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn shutdown(&self) {
        self.event_stop.store(true, Ordering::Release);
        let runtime = self
            .runtime
            .lock()
            .ok()
            .and_then(|mut runtime| runtime.take());
        if let Some(mut runtime) = runtime {
            runtime.shutdown();
        }
    }

    pub(crate) fn set_pending_deep_link(&self, url: String) {
        if let Ok(mut pending) = self.pending_deep_link.lock() {
            *pending = Some(url);
        }
    }

    pub(crate) fn take_pending_deep_link(&self) -> Option<String> {
        self.pending_deep_link
            .lock()
            .ok()
            .and_then(|mut pending| pending.take())
    }
}

pub(crate) fn core_status(state: &State<'_, AppState>) -> Result<serde_json::Value, String> {
    state.inner().core_status()
}

pub(crate) fn state_directory(state: &State<'_, AppState>) -> Result<PathBuf, String> {
    state.inner().state_directory()
}

pub(crate) fn compute_status(state: &State<'_, AppState>) -> Result<serde_json::Value, String> {
    state.inner().compute_status()
}

pub(crate) fn compute_command(
    state: &State<'_, AppState>,
    command: ComputeCommand,
) -> Result<serde_json::Value, String> {
    state.inner().compute_command(command)
}
