use tauri::State;

use crate::{
    ipc::{call_daemon, shared_state},
    state::AppState,
};

#[tauri::command]
pub(crate) async fn history_detail(
    state: State<'_, AppState>,
    submission_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "history_detail",
        serde_json::json!({ "submission_id": submission_id }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn withdraw_history(
    state: State<'_, AppState>,
    submission_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "withdraw",
        serde_json::json!({ "submission_id": submission_id }),
    )
    .await
}

#[tauri::command]
pub(crate) fn validate_public_run_editor(
    input: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let input = serde_json::from_value(input).map_err(|_| "public-run-invalid".to_owned())?;
    serde_json::to_value(trace_commons_contributor::public_run::validate_public_run_editor(input))
        .map_err(|_| "public-run-invalid".to_owned())
}

#[tauri::command]
pub(crate) async fn publish_public_run(
    state: State<'_, AppState>,
    submission_id: String,
    draft: serde_json::Value,
    task_success: String,
    contributed_version: String,
    expected_publication_version: u32,
) -> Result<serde_json::Value, String> {
    let task_success = serde_json::Value::String(task_success);
    call_daemon(
        shared_state(&state)?,
        "publish_public_run",
        serde_json::json!({
            "submission_id": submission_id,
            "draft": draft,
            "task_success": task_success,
            "contributed_version": contributed_version,
            "expected_publication_version": expected_publication_version,
        }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn unpublish_public_run(
    state: State<'_, AppState>,
    submission_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "unpublish_public_run",
        serde_json::json!({ "submission_id": submission_id }),
    )
    .await
}

#[tauri::command]
pub(crate) fn skill_learning_copy() -> Result<serde_json::Value, String> {
    serde_json::to_value(trace_commons_contributor::skill_loop::skill_learning_copy())
        .map_err(|_| "skill-copy-unavailable".to_owned())
}

#[tauri::command]
pub(crate) async fn skill_candidate(
    state: State<'_, AppState>,
    submission_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "skill_candidate",
        serde_json::json!({ "submission_id": submission_id }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn skill_review(
    state: State<'_, AppState>,
    candidate_id: String,
    draft: serde_json::Value,
    replaces_review_id: Option<String>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "skill_review",
        serde_json::json!({
            "candidate_id": candidate_id,
            "draft": draft,
            "replaces_review_id": replaces_review_id,
        }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn skill_evaluate(
    state: State<'_, AppState>,
    review_id: String,
    skill_sha256: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "skill_evaluate",
        serde_json::json!({ "review_id": review_id, "skill_sha256": skill_sha256 }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn skill_install_plan(
    state: State<'_, AppState>,
    evaluation_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "skill_install_plan",
        serde_json::json!({ "evaluation_id": evaluation_id }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn skill_install_commit(
    state: State<'_, AppState>,
    plan_id: String,
    as_previewed_sha256: String,
    as_previewed_marker_sha256: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "skill_install_commit",
        serde_json::json!({
            "plan_id": plan_id,
            "as_previewed_sha256": as_previewed_sha256,
            "as_previewed_marker_sha256": as_previewed_marker_sha256,
        }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn skill_install_status(
    state: State<'_, AppState>,
    source_submission_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "skill_install_status",
        serde_json::json!({ "source_submission_id": source_submission_id }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn skill_install_rollback(
    state: State<'_, AppState>,
    install_id: String,
    source_submission_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "skill_install_rollback",
        serde_json::json!({
            "install_id": install_id,
            "source_submission_id": source_submission_id,
        }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::validate_public_run_editor;

    #[test]
    fn public_run_validation_stays_in_rust() {
        let value = validate_public_run_editor(serde_json::json!({
            "title": "Repair a stalled upload",
            "outcome_summary": "The upload completed.",
            "correction_excerpt": null,
            "workflow": "Renew the session, then retry once.",
            "reuse_permission": "cc_by_4_0",
            "evidence": [{
                "event_id": "00000000-0000-0000-0000-000000000001",
                "excerpt": "The retry returned a successful status."
            }],
            "source": ""
        }))
        .expect("valid editor input");
        assert_eq!(value["error"], serde_json::Value::Null);
        assert_eq!(value["draft"]["reuse_permission"], "cc_by_4_0");

        let missing_permission = validate_public_run_editor(serde_json::json!({
            "title": "Repair a stalled upload",
            "outcome_summary": "The upload completed.",
            "correction_excerpt": null,
            "workflow": "Renew the session, then retry once.",
            "reuse_permission": null,
            "evidence": [{
                "event_id": "00000000-0000-0000-0000-000000000001",
                "excerpt": "The retry returned a successful status."
            }],
            "source": ""
        }))
        .expect("validation response");
        assert!(missing_permission["draft"].is_null());
        assert!(missing_permission["error"].is_string());
    }
}
