use std::path::PathBuf;

use tauri::{
    State,
    ipc::{InvokeBody, Request},
};

use crate::{
    commands::platform::git_repository,
    state::{AppState, state_directory},
};

pub(crate) mod comparisons;

const MAX_INSIGHTS_INPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_TEST_REPORT_INPUT_BYTES: usize = 64 * 1024;

fn upload_header(request: &Request<'_>, name: &str) -> Result<String, String> {
    request
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .map(str::to_owned)
        .ok_or_else(|| "insights-upload-header-invalid".to_owned())
}

fn raw_upload_bytes(request: &Request<'_>, max: usize) -> Result<Vec<u8>, String> {
    match request.body() {
        InvokeBody::Raw(bytes) if bytes.len() <= max => Ok(bytes.clone()),
        InvokeBody::Raw(_) => Err("insights-input-too-large".to_owned()),
        _ => Err("insights-input-must-be-raw-bytes".to_owned()),
    }
}

fn execute_insights(
    state_dir: PathBuf,
    operation: serde_json::Value,
    file_bytes: Option<Vec<u8>>,
) -> Result<serde_json::Value, String> {
    let insights_dir = state_dir.join("insights");
    let mut request = serde_json::json!({
        "store_dir": insights_dir,
        "operation": operation,
    });
    let staged_path = if let Some(bytes) = file_bytes {
        let operation_type = request["operation"]["type"].as_str();
        let max_bytes = match operation_type {
            Some("analyze") => MAX_INSIGHTS_INPUT_BYTES,
            Some("link_test_report") => MAX_TEST_REPORT_INPUT_BYTES,
            _ => return Err("file bytes are only accepted for file operations".to_owned()),
        };
        if bytes.len() > max_bytes {
            return Err("insights-input-too-large".to_owned());
        }
        let input_dir = state_dir.join("insights-inputs");
        std::fs::create_dir_all(&input_dir).map_err(|_| "insights-input-unavailable".to_owned())?;
        let input_path = input_dir.join(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "insights-input-unavailable".to_owned())?
                .as_nanos()
        ));
        std::fs::write(&input_path, bytes).map_err(|_| "insights-input-unavailable".to_owned())?;
        request["operation"]["file"] =
            serde_json::Value::String(input_path.to_string_lossy().into_owned());
        Some(input_path)
    } else {
        None
    };

    let result = serde_json::to_vec(&request)
        .map_err(|_| "insights-request-invalid".to_owned())
        .and_then(|bytes| {
            trace_commons_contributor::insights::service::dispatch_json(&bytes)
                .map_err(|error| error.to_string())
        })
        .and_then(|json| {
            serde_json::from_str(&json).map_err(|_| "insights-response-invalid".to_owned())
        });
    if let Some(path) = staged_path {
        let _ = std::fs::remove_file(path);
    }
    result
}

pub(super) async fn insights_call(
    state_dir: PathBuf,
    operation: serde_json::Value,
    file_bytes: Option<Vec<u8>>,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || execute_insights(state_dir, operation, file_bytes))
        .await
        .map_err(|_| "insights-operation-failed".to_owned())?
}

#[tauri::command]
pub(crate) async fn insights_list(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "list" }),
        None,
    )
    .await
}
#[tauri::command]
pub(crate) async fn insights_summary(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "summary" }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn analyze_insight(
    state: State<'_, AppState>,
    request: Request<'_>,
) -> Result<serde_json::Value, String> {
    let source = upload_header(&request, "x-tc-source")?;
    let save = match upload_header(&request, "x-tc-save")?.as_str() {
        "true" => true,
        "false" => false,
        _ => return Err("insights-save-invalid".to_owned()),
    };
    let file_bytes = raw_upload_bytes(&request, MAX_INSIGHTS_INPUT_BYTES)?;
    insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "analyze", "source": source, "save": save }),
        Some(file_bytes),
    )
    .await
}

#[tauri::command]
pub(crate) async fn explain_insight(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "explain", "id": id }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn delete_insight(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "delete", "id": id }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn annotate_insight(
    state: State<'_, AppState>,
    id: String,
    category: String,
    outcome: String,
) -> Result<serde_json::Value, String> {
    insights_call(state_directory(&state)?, serde_json::json!({ "type": "annotate", "id": id, "category": category, "outcome": outcome }), None).await
}

#[tauri::command]
pub(crate) async fn clear_insight_annotation(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "clear_annotation", "id": id }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn link_test_report(
    state: State<'_, AppState>,
    request: Request<'_>,
) -> Result<serde_json::Value, String> {
    let id = upload_header(&request, "x-tc-insight-id")?;
    let file_bytes = raw_upload_bytes(&request, MAX_TEST_REPORT_INPUT_BYTES)?;
    insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "link_test_report", "id": id }),
        Some(file_bytes),
    )
    .await
}

#[tauri::command]
pub(crate) async fn link_git_insight(
    state: State<'_, AppState>,
    id: String,
    repository: String,
    commit: String,
) -> Result<serde_json::Value, String> {
    let repository = git_repository(&repository)?;
    insights_call(
        state_directory(&state)?,
        serde_json::json!({
            "type": "link_git",
            "id": id,
            "repository": repository,
            "commit": commit,
        }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn unlink_insight_evidence(
    state: State<'_, AppState>,
    id: String,
    evidence_id: String,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({
            "type": "unlink_evidence",
            "id": id,
            "evidence_id": evidence_id,
        }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn insights_question_cards(
    state: State<'_, AppState>,
    questions: Vec<String>,
    snapshot_ids: Vec<String>,
    episode_ids: Vec<String>,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({
            "type": "question_cards",
            "questions": questions,
            "snapshot_ids": snapshot_ids,
            "episode_ids": episode_ids,
        }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn insights_episode_list(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "episode_list" }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn insights_episode_create(
    state: State<'_, AppState>,
    snapshot_ids: Vec<String>,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "episode_create", "snapshot_ids": snapshot_ids }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn insights_episode_explain(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({ "type": "episode_explain", "id": id }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn insights_episode_annotate(
    state: State<'_, AppState>,
    id: String,
    expected_revision: u64,
    category: String,
    outcome: String,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({
            "type": "episode_annotate",
            "id": id,
            "expected_revision": expected_revision,
            "category": category,
            "outcome": outcome,
        }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn insights_episode_clear_assessment(
    state: State<'_, AppState>,
    id: String,
    expected_revision: u64,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({
            "type": "episode_clear_assessment",
            "id": id,
            "expected_revision": expected_revision,
        }),
        None,
    )
    .await
}

#[tauri::command]
pub(crate) async fn insights_episode_delete(
    state: State<'_, AppState>,
    id: String,
    expected_revision: u64,
) -> Result<serde_json::Value, String> {
    insights_call(
        state_directory(&state)?,
        serde_json::json!({
            "type": "episode_delete",
            "id": id,
            "expected_revision": expected_revision,
        }),
        None,
    )
    .await
}
