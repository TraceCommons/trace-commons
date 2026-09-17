use tauri::State;

use crate::{
    ipc::{
        READ_ONLY_METHODS, bootstrap_profile, bootstrap_settings, call_daemon,
        optional_shared_state, shared_state,
    },
    state::{AppState, core_status as state_core_status},
};

#[tauri::command]
pub(crate) fn core_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    state_core_status(&state)
}

#[tauri::command]
pub(crate) fn queue_outcome_line(label: String) -> String {
    trace_commons_contributor::private_inference_copy::queue_outcome_line(&label).to_owned()
}

#[tauri::command]
pub(crate) async fn daemon_call(
    state: State<'_, AppState>,
    method: String,
    params: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    if !READ_ONLY_METHODS.contains(&method.as_str()) {
        return Err("IPC method is not enabled in this read-only prototype".to_owned());
    }
    let Some(shared) = optional_shared_state(&state)? else {
        return match method.as_str() {
            "get_settings" => bootstrap_settings(&state),
            "get_public_profile" => Ok(bootstrap_profile()),
            _ => Err("Rust core daemon is not started; choose source roots".to_owned()),
        };
    };
    call_daemon(shared, &method, params.unwrap_or_default()).await
}

#[tauri::command]
pub(crate) async fn preview_entry(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "preview",
        serde_json::json!({ "entry_id": entry_id }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn dismiss_entry(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "dismiss",
        serde_json::json!({ "entry_id": entry_id }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn approve_entry(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "approve",
        serde_json::json!({ "entry_id": entry_id }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn approve_project(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "approve",
        serde_json::json!({ "project_id": project_id }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn pause_daemon(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    call_daemon(shared_state(&state)?, "pause", serde_json::json!({})).await
}

#[tauri::command]
pub(crate) async fn resume_daemon(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    call_daemon(shared_state(&state)?, "resume", serde_json::json!({})).await
}

#[tauri::command]
pub(crate) async fn arming_suggestion(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "arming_suggestion",
        serde_json::json!({}),
    )
    .await
}

#[tauri::command]
pub(crate) async fn accept_arming(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "set_project_mode",
        serde_json::json!({ "project_id": project_id, "mode": "auto_upload" }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn decline_arming(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "decline_arming",
        serde_json::json!({ "project_id": project_id }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn cancel_entry(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "cancel",
        serde_json::json!({ "entry_id": entry_id }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn cancel_project(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "cancel",
        serde_json::json!({ "project_id": project_id }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn preview_body(
    state: State<'_, AppState>,
    entry_id: String,
    offset: Option<u64>,
    limit: Option<u64>,
    body_digest: Option<String>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "preview_body",
        serde_json::json!({
            "entry_id": entry_id,
            "offset": offset.unwrap_or(0),
            "limit": limit.unwrap_or(256 * 1024),
            "body_digest": body_digest,
        }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn preview_turns(
    state: State<'_, AppState>,
    entry_id: String,
    body_digest: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "preview_turns",
        serde_json::json!({ "entry_id": entry_id, "body_digest": body_digest }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn search_original(
    state: State<'_, AppState>,
    entry_id: String,
    needle: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "search_original",
        serde_json::json!({ "entry_id": entry_id, "needle": needle }),
    )
    .await
}
