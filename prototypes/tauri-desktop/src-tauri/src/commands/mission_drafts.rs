use std::path::PathBuf;

use tauri::State;

use crate::state::{AppState, state_directory};

const MAX_MISSION_INPUT_BYTES: usize = 64 * 1024;

fn execute_mission_draft(
    state_dir: PathBuf,
    operation: serde_json::Value,
    file_bytes: Option<Vec<u8>>,
) -> Result<serde_json::Value, String> {
    let mission_dir = state_dir.join("mission-drafts");
    let mut request = serde_json::json!({
        "store_dir": mission_dir,
        "operation": operation,
    });
    let staged_path = if let Some(bytes) = file_bytes {
        if request["operation"]["type"] != "import" {
            return Err("file bytes are only accepted for import".to_owned());
        }
        if bytes.len() > MAX_MISSION_INPUT_BYTES {
            return Err("mission-input-too-large".to_owned());
        }
        let input_dir = state_dir.join("mission-inputs");
        std::fs::create_dir_all(&input_dir)
            .map_err(|_| "mission-draft-file-unreadable".to_owned())?;
        let input_path = input_dir.join(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "mission-draft-file-unreadable".to_owned())?
                .as_nanos()
        ));
        std::fs::write(&input_path, bytes)
            .map_err(|_| "mission-draft-file-unreadable".to_owned())?;
        request["operation"]["file"] =
            serde_json::Value::String(input_path.to_string_lossy().into_owned());
        Some(input_path)
    } else {
        None
    };
    let result = serde_json::to_vec(&request)
        .map_err(|_| "mission-draft-request-invalid".to_owned())
        .and_then(|bytes| {
            trace_commons_contributor::mission_draft_service::dispatch_json(&bytes)
                .map_err(|error| error.to_string())
        })
        .and_then(|json| {
            serde_json::from_str(&json).map_err(|_| "mission-draft-response-invalid".to_owned())
        });
    if let Some(path) = staged_path {
        let _ = std::fs::remove_file(path);
    }
    result
}

async fn mission_draft_call(
    state_dir: PathBuf,
    operation: serde_json::Value,
    file_bytes: Option<Vec<u8>>,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        execute_mission_draft(state_dir, operation, file_bytes)
    })
    .await
    .map_err(|_| "mission-draft-operation-failed".to_owned())?
}

#[tauri::command]
pub(crate) async fn mission_draft_list(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    mission_draft_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "list" }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn mission_draft_show(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    mission_draft_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "show", "id": id }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn mission_draft_import(
    state: State<'_, AppState>,
    file_bytes: Vec<u8>,
) -> Result<serde_json::Value, String> {
    mission_draft_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "import" }),
        Some(file_bytes),
    )
    .await
}

#[tauri::command]
pub(crate) async fn mission_draft_delete(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    mission_draft_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "delete", "id": id }),
        None,
    )
    .await
}
