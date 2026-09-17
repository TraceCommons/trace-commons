use tauri::State;

use crate::{
    ipc::{call_daemon, shared_state},
    state::AppState,
};

#[tauri::command]
pub(crate) async fn publish_profile(
    state: State<'_, AppState>,
    handle: String,
    bio: Option<String>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "set_public_profile",
        serde_json::json!({ "handle": handle, "bio": bio }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn withdraw_profile(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "clear_public_profile",
        serde_json::json!({}),
    )
    .await
}
