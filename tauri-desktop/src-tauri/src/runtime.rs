use std::sync::Arc;

use anyhow::Context;
use tauri::State;
use trace_commons_contributor::{
    compute::ComputeController,
    config::ConfigStore,
    daemon::{
        self, StartFailure, attached::AttachedDaemon, ipc::DaemonShared, settings::DaemonSettings,
    },
};

use crate::{
    ipc::optional_shared_state,
    state::{AppState, DaemonConnection, Runtime, state_directory},
};

pub(crate) fn start_runtime() -> anyhow::Result<Runtime> {
    let store = ConfigStore::resolve(None)?;
    let state_dir = store.dir().to_path_buf();
    let settings = DaemonSettings::load(&store);
    let (connection, embedded, daemon_unavailable) = match settings {
        Ok(settings) if trace_commons_contributor::daemon::settings::roots_declared(&settings) => {
            match tauri::async_runtime::block_on(daemon::start_embedded(store.clone())) {
                Ok(embedded) => {
                    let shared = Arc::clone(&embedded.shared);
                    (
                        Some(DaemonConnection::Embedded(shared)),
                        Some(embedded),
                        false,
                    )
                }
                Err(error) if is_already_running(&error) => match AttachedDaemon::connect(&store) {
                    Ok(attached) => (
                        Some(DaemonConnection::Attached(Arc::new(attached))),
                        None,
                        false,
                    ),
                    Err(_) => (None, None, true),
                },
                Err(_) => (None, None, true),
            }
        }
        Ok(_) => (None, None, false),
        Err(_) => (None, None, true),
    };
    let compute =
        Arc::new(ComputeController::open(&state_dir).context("opening compute controller")?);
    if let Some(embedded) = embedded.as_ref() {
        let shared = Arc::clone(&embedded.shared);
        start_supervisor(shared);
    }

    Ok(Runtime::new(
        state_dir,
        connection,
        compute,
        embedded,
        daemon_unavailable,
    ))
}

fn is_already_running(error: &anyhow::Error) -> bool {
    error.downcast_ref::<StartFailure>() == Some(&StartFailure::AlreadyRunning)
}

fn start_supervisor(shared: Arc<DaemonShared>) {
    tauri::async_runtime::spawn(async move {
        if daemon::run_supervisor(shared, false).await.is_err() {
            eprintln!("Tauri daemon supervisor stopped");
        }
    });
}

pub(crate) async fn ensure_daemon_started(state: &State<'_, AppState>) -> Result<(), String> {
    if optional_shared_state(state)?.is_some() {
        return Ok(());
    }
    let state_dir = state_directory(state)?;
    let store = ConfigStore::open(state_dir).map_err(|_| "settings-write-failed")?;
    let embedded = match daemon::start_embedded(store.clone()).await {
        Ok(embedded) => embedded,
        Err(error) if is_already_running(&error) => {
            let attached =
                tauri::async_runtime::spawn_blocking(move || AttachedDaemon::connect(&store))
                    .await
                    .map_err(|_| "attached-daemon-call-panicked")?
                    .map_err(|error| error.to_string())?;
            state.inner().attach_external_daemon(Arc::new(attached))?;
            return Ok(());
        }
        Err(_) => return Err("starting embedded contributor daemon failed".to_owned()),
    };
    let shared = Arc::clone(&embedded.shared);
    start_supervisor(Arc::clone(&shared));
    state.inner().attach_embedded_daemon(shared, embedded)
}

pub(crate) fn stop_runtime(state: &AppState) {
    state.shutdown();
}
