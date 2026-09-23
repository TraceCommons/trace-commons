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
        ipc::{DaemonShared, EVENT_RESYNC_REQUIRED, Event, Response},
    },
};

#[derive(Clone)]
pub(crate) enum DaemonConnection {
    Embedded(Arc<DaemonShared>),
    Attached(Arc<AttachedDaemon>),
}

impl DaemonConnection {
    pub(crate) fn is_alive(&self) -> bool {
        match self {
            Self::Embedded(_) => true,
            Self::Attached(daemon) => !daemon.is_closed(),
        }
    }
}

pub(crate) struct Runtime {
    state_dir: PathBuf,
    connection: Option<DaemonConnection>,
    compute: Arc<ComputeController>,
    daemon: Option<EmbeddedDaemon>,
    daemon_unavailable: bool,
}

impl Runtime {
    pub(crate) fn daemon_recovery_needed(&self) -> bool {
        self.daemon_unavailable
    }

    pub(crate) fn new(
        state_dir: PathBuf,
        connection: Option<DaemonConnection>,
        compute: Arc<ComputeController>,
        daemon: Option<EmbeddedDaemon>,
        daemon_unavailable: bool,
    ) -> Self {
        Self {
            state_dir,
            connection,
            compute,
            daemon,
            daemon_unavailable,
        }
    }

    fn shutdown(&mut self) {
        self.compute.shutdown(Duration::from_secs(1));
        if let Some(connection) = self.connection.take() {
            match connection {
                DaemonConnection::Embedded(shared) => {
                    shared.shutdown.store(true, Ordering::Release);
                    shared.shutdown_signal.notify_one();
                    let _ = shared.events.send(Event {
                        event: EVENT_RESYNC_REQUIRED.to_owned(),
                        data: serde_json::json!({}),
                    });
                }
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
    allowed_wallet_url: Mutex<Option<String>>,
    allowed_account_sign_in_url: Mutex<Option<String>>,
    deep_link_state: Mutex<String>,
    event_stop: Arc<AtomicBool>,
    event_bridge_started: AtomicBool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            runtime: Mutex::new(None),
            pending_deep_link: Mutex::new(None),
            allowed_wallet_url: Mutex::new(None),
            allowed_account_sign_in_url: Mutex::new(None),
            deep_link_state: Mutex::new("unknown".to_owned()),
            event_stop: Arc::new(AtomicBool::new(false)),
            event_bridge_started: AtomicBool::new(false),
        }
    }
}

impl AppState {
    pub(crate) fn authorize_wallet_url(&self, url: Option<&str>) -> Result<(), String> {
        let mut slot = self
            .allowed_wallet_url
            .lock()
            .map_err(|_| "application state lock poisoned".to_owned())?;
        *slot = url.map(str::to_owned);
        Ok(())
    }

    pub(crate) fn consume_wallet_url(&self, url: &str) -> Result<bool, String> {
        let mut slot = self
            .allowed_wallet_url
            .lock()
            .map_err(|_| "application state lock poisoned".to_owned())?;
        if slot.as_deref() == Some(url) {
            slot.take();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(crate) fn authorize_account_sign_in_url(&self, url: Option<&str>) -> Result<(), String> {
        let mut slot = self
            .allowed_account_sign_in_url
            .lock()
            .map_err(|_| "application state lock poisoned".to_owned())?;
        *slot = url.map(str::to_owned);
        Ok(())
    }

    pub(crate) fn consume_account_sign_in_url(&self, url: &str) -> Result<bool, String> {
        let mut slot = self
            .allowed_account_sign_in_url
            .lock()
            .map_err(|_| "application state lock poisoned".to_owned())?;
        if slot.as_deref() == Some(url) {
            slot.take();
            Ok(true)
        } else {
            Ok(false)
        }
    }

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
        runtime
            .as_ref()
            .and_then(|runtime| runtime.connection.clone())
            .filter(|connection| connection.is_alive())
            .ok_or_else(|| "Rust core daemon is not started; choose source roots".to_owned())
    }

    pub(crate) fn optional_daemon(&self) -> Result<Option<DaemonConnection>, String> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "application state lock poisoned".to_owned())?;
        Ok(runtime
            .as_ref()
            .and_then(|runtime| runtime.connection.clone())
            .filter(DaemonConnection::is_alive))
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
        let mut slot = self
            .runtime
            .lock()
            .map_err(|_| "application state lock poisoned".to_owned())?;
        let runtime = slot
            .as_mut()
            .ok_or_else(|| "Rust core is not started".to_owned())?;
        if matches!(runtime.connection, Some(DaemonConnection::Embedded(_))) {
            drop(slot);
            daemon.close();
            return Ok(());
        }
        let previous = runtime
            .connection
            .replace(DaemonConnection::Embedded(shared));
        runtime.daemon = Some(daemon);
        runtime.daemon_unavailable = false;
        drop(slot);
        if let Some(DaemonConnection::Attached(attached)) = previous {
            attached.close();
        }
        Ok(())
    }

    pub(crate) fn attach_external_daemon(&self, daemon: Arc<AttachedDaemon>) -> Result<(), String> {
        let mut slot = self
            .runtime
            .lock()
            .map_err(|_| "application state lock poisoned".to_owned())?;
        let runtime = slot
            .as_mut()
            .ok_or_else(|| "Rust core is not started".to_owned())?;
        let replace_closed_attachment = match runtime.connection.as_ref() {
            None => true,
            Some(DaemonConnection::Attached(attached)) => attached.is_closed(),
            Some(DaemonConnection::Embedded(_)) => false,
        };
        let previous = if replace_closed_attachment {
            runtime.daemon_unavailable = false;
            runtime
                .connection
                .replace(DaemonConnection::Attached(Arc::clone(&daemon)))
        } else {
            None
        };
        drop(slot);
        if !replace_closed_attachment {
            daemon.close();
        }
        if let Some(DaemonConnection::Attached(attached)) = previous {
            attached.close();
        }
        Ok(())
    }

    pub(crate) fn event_stop(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.event_stop)
    }

    pub(crate) fn claim_event_bridge(&self) -> bool {
        !self.event_bridge_started.swap(true, Ordering::AcqRel)
    }

    pub(crate) fn release_event_bridge(&self) {
        self.event_bridge_started.store(false, Ordering::Release);
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
            DaemonConnection::Attached(daemon) => Ok(Some(Self::response_value(
                daemon
                    .call_with_timeout("status", &serde_json::json!({}), Duration::from_secs(3))
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
        let daemon_unavailable = runtime.daemon_unavailable;
        drop(guard);
        let daemon = Self::daemon_value(connection.clone().filter(DaemonConnection::is_alive))?
            .unwrap_or_else(|| {
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
        let startup = if connection.as_ref().is_some_and(DaemonConnection::is_alive) {
            "running"
        } else if daemon_unavailable || connection.is_some() {
            "daemon_unavailable"
        } else {
            "needs_roots"
        };

        Ok(serde_json::json!({
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

    pub(crate) fn set_deep_link_state(&self, state: &str) {
        if let Ok(mut current) = self.deep_link_state.lock() {
            *current = state.to_owned();
        }
    }

    pub(crate) fn deep_link_state(&self) -> String {
        self.deep_link_state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_else(|_| "unknown".to_owned())
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

#[cfg(test)]
mod tests {
    use super::AppState;

    #[test]
    fn wallet_url_authorization_is_exact_and_single_use() {
        let state = AppState::default();
        state
            .authorize_wallet_url(Some("https://commons.example/wallet/start?state=1"))
            .unwrap();
        assert!(
            !state
                .consume_wallet_url("https://attacker.example/wallet/start?state=1")
                .unwrap()
        );
        assert!(
            state
                .consume_wallet_url("https://commons.example/wallet/start?state=1")
                .unwrap()
        );
        assert!(
            !state
                .consume_wallet_url("https://commons.example/wallet/start?state=1")
                .unwrap()
        );
    }

    #[test]
    fn account_sign_in_url_authorization_expires_after_use() {
        let state = AppState::default();
        state
            .authorize_account_sign_in_url(Some("https://commons.example/account/start?once=1"))
            .unwrap();
        assert!(
            !state
                .consume_account_sign_in_url("https://other.example/account/start?once=1")
                .unwrap()
        );
        assert!(
            state
                .consume_account_sign_in_url("https://commons.example/account/start?once=1")
                .unwrap()
        );
        assert!(
            !state
                .consume_account_sign_in_url("https://commons.example/account/start?once=1")
                .unwrap()
        );
    }
}
