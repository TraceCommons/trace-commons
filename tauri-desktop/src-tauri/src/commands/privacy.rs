use tauri::State;

use crate::{
    ipc::{call_daemon, shared_state},
    state::AppState,
};

#[tauri::command]
pub(crate) async fn set_private_inference(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "set_settings",
        serde_json::json!({
            "private_inference": enabled,
            "private_inference_offer_seen": true,
        }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn set_inference_evidence(
    state: State<'_, AppState>,
    enabled: bool,
    confirmed: bool,
) -> Result<serde_json::Value, String> {
    if enabled && !confirmed {
        return Err("disclosure-required".to_owned());
    }
    call_daemon(
        shared_state(&state)?,
        "set_settings",
        serde_json::json!({ "ironwire_attested_bodies": enabled }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn set_token_contribution(
    state: State<'_, AppState>,
    enabled: bool,
    confirmed: bool,
) -> Result<serde_json::Value, String> {
    if enabled && !confirmed {
        return Err("disclosure-required".to_owned());
    }
    call_daemon(
        shared_state(&state)?,
        "set_settings",
        serde_json::json!({ "token_distributions_contribution": enabled }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn set_token_capture(
    state: State<'_, AppState>,
    enabled: bool,
    confirmed: bool,
) -> Result<serde_json::Value, String> {
    if enabled && !confirmed {
        return Err("disclosure-required".to_owned());
    }
    call_daemon(
        shared_state(&state)?,
        "set_settings",
        serde_json::json!({ "token_capture_enabled": enabled }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn token_storage_status(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "token_storage_status",
        serde_json::json!({}),
    )
    .await
}

#[tauri::command]
pub(crate) async fn clean_token_storage(
    state: State<'_, AppState>,
    discard: bool,
    confirmed: bool,
) -> Result<serde_json::Value, String> {
    if discard && !confirmed {
        return Err("discard-confirmation-required".to_owned());
    }
    call_daemon(
        shared_state(&state)?,
        if discard {
            "discard_token_reviews"
        } else {
            "remove_token_local_copies"
        },
        serde_json::json!({ "confirmed": discard && confirmed }),
    )
    .await
}
