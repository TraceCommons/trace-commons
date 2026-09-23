use tauri::State;

use super::insights_call;
use crate::state::{AppState, state_directory};

#[tauri::command]
pub(crate) async fn comparison_task_create(
    state: State<'_, AppState>,
    episode_ids: Vec<String>,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "comparison_task_create", "episode_ids": episode_ids }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn comparison_task_list(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    match insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "comparison_task_list" }),
        None,
    )
    .await
    {
        Err(error) if error == "insights_comparison_task_not_found" => {
            Ok(serde_json::json!({ "type": "comparison_task_list", "tasks": [] }))
        }
        result => result,
    }
}

#[tauri::command]
pub(crate) async fn comparison_task_explain(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "comparison_task_explain", "id": id }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn comparison_task_replace_episodes(
    state: State<'_, AppState>,
    id: String,
    expected_revision: u64,
    episode_ids: Vec<String>,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({
            "type": "comparison_task_replace_episodes",
            "id": id,
            "expected_revision": expected_revision,
            "episode_ids": episode_ids,
        }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn comparison_task_set_context(
    state: State<'_, AppState>,
    id: String,
    expected_revision: u64,
    context: serde_json::Value,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({
            "type": "comparison_task_set_context",
            "id": id,
            "expected_revision": expected_revision,
            "context": context,
        }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn comparison_task_set_outcome(
    state: State<'_, AppState>,
    id: String,
    expected_revision: u64,
    value: String,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({
            "type": "comparison_task_set_outcome",
            "id": id,
            "expected_revision": expected_revision,
            "outcome": value,
        }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn comparison_task_clear_outcome(
    state: State<'_, AppState>,
    id: String,
    expected_revision: u64,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({
            "type": "comparison_task_clear_outcome",
            "id": id,
            "expected_revision": expected_revision,
        }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn comparison_task_reconfirm(
    state: State<'_, AppState>,
    id: String,
    expected_revision: u64,
    displayed_material_digest: String,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({
            "type": "comparison_task_reconfirm",
            "id": id,
            "expected_revision": expected_revision,
            "displayed_material_digest": displayed_material_digest,
        }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn comparison_specification_list(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    match insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "comparison_list_specs" }),
        None,
    )
    .await
    {
        Err(error) if error == "insights-comparison-no-saved-tasks" => {
            Ok(serde_json::json!({ "type": "comparison_specification_list", "specifications": [] }))
        }
        result => result,
    }
}

#[tauri::command]
pub(crate) async fn comparison_specification_preview(
    state: State<'_, AppState>,
    input: serde_json::Value,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "comparison_preview_spec", "input": input }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn comparison_specification_save(
    state: State<'_, AppState>,
    input: serde_json::Value,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "comparison_save_spec", "input": input }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn comparison_specification_evaluate(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "comparison_evaluate", "id": id }),
        None,
    )
    .await
}
