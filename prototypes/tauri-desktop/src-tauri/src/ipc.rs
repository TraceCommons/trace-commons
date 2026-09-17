use tauri::State;
use trace_commons_contributor::{
    config::ConfigStore,
    daemon::{
        self,
        ipc::{Request, Response},
        settings::{DaemonSettings, SourceDeclaration},
    },
};

use crate::state::{AppState, DaemonConnection, state_directory};

pub(crate) const READ_ONLY_METHODS: &[&str] = &[
    "get_public_profile",
    "get_settings",
    "harness_list",
    "history_rollup",
    "list_audit",
    "list_history",
    "list_pending",
    "list_projects",
    "queue_outcome_counts",
    "token_storage_status",
];

pub(crate) fn shared_state(state: &State<'_, AppState>) -> Result<DaemonConnection, String> {
    state.inner().daemon()
}

pub(crate) fn optional_shared_state(
    state: &State<'_, AppState>,
) -> Result<Option<DaemonConnection>, String> {
    state.inner().optional_daemon()
}

fn source_mode(source: Option<&SourceDeclaration>) -> &'static str {
    match source {
        Some(SourceDeclaration::Watch { .. }) => "watch",
        Some(SourceDeclaration::Off) => "off",
        None => "unset",
    }
}

pub(crate) fn bootstrap_settings(state: &State<'_, AppState>) -> Result<serde_json::Value, String> {
    let store = ConfigStore::open(state_directory(state)?).map_err(|_| "settings-read-failed")?;
    let settings = DaemonSettings::load(&store).map_err(|_| "settings-read-failed")?;
    Ok(serde_json::json!({
        "schema_version": settings.schema_version,
        "claude_source_mode": source_mode(settings.claude_source.as_ref()),
        "codex_source_mode": source_mode(settings.codex_source.as_ref()),
        "gemini_source_mode": source_mode(settings.gemini_source.as_ref()),
        "cline_source_mode": source_mode(settings.cline_source.as_ref()),
        "opencode_source_mode": source_mode(settings.opencode_source.as_ref()),
        "local_notifications": settings.local_notifications,
        "near_ai_configured": settings.near_ai.is_some(),
        "near_ai_inference_configured": settings.near_ai_inference.is_some(),
        "near_ai_session_retained": settings.near_ai_session.is_some(),
    }))
}

pub(crate) fn bootstrap_profile() -> serde_json::Value {
    serde_json::json!({
        "on_roster": false,
        "handle": null,
        "bio": null,
        "public_since": null,
        "public_url": null,
    })
}

pub(crate) async fn call_daemon(
    daemon: DaemonConnection,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    response_value(call_daemon_response(daemon, method, params).await?)
}

pub(crate) async fn call_daemon_response(
    daemon: DaemonConnection,
    method: &str,
    params: serde_json::Value,
) -> Result<Response, String> {
    let request = Request {
        id: 0,
        method: method.to_owned(),
        params,
    };
    let response = match daemon {
        DaemonConnection::Embedded(shared) => {
            daemon::ipc::handle_request_async(&shared, &request).await
        }
        #[cfg(unix)]
        DaemonConnection::Attached(attached) => tauri::async_runtime::spawn_blocking(move || {
            attached.call(&request.method, &request.params)
        })
        .await
        .map_err(|_| "attached-daemon-call-panicked".to_owned())?
        .map_err(|error| error.to_string())?,
    };
    Ok(response)
}

pub(crate) fn call_daemon_blocking(
    daemon: DaemonConnection,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let response = match daemon {
        DaemonConnection::Embedded(shared) => daemon::ipc::handle_local(&shared, method, params),
        #[cfg(unix)]
        DaemonConnection::Attached(attached) => attached
            .call(method, &params)
            .map_err(|error| error.to_string())?,
    };
    response_value(response)
}

fn response_value(response: Response) -> Result<serde_json::Value, String> {
    match (response.result, response.error) {
        (Some(result), None) => Ok(result),
        (None, Some(error)) => Err(format!("{}: {}", error.code, error.message)),
        _ => Err("Rust core returned an invalid IPC response".to_owned()),
    }
}
