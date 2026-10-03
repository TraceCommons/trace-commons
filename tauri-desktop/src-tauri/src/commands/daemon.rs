use tauri::{AppHandle, Manager, Runtime, State};

use crate::{
    app::start_event_bridge,
    ipc::{
        READ_ONLY_METHODS, bootstrap_profile, bootstrap_settings, call_daemon,
        optional_shared_state, shared_state,
    },
    state::{AppState, core_status as state_core_status},
};

#[tauri::command]
pub(crate) async fn retry_daemon_startup<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    crate::runtime::ensure_daemon_started(&state).await?;
    start_event_bridge(app);
    Ok(())
}

#[tauri::command]
pub(crate) async fn core_status<R: Runtime>(
    app: AppHandle<R>,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state_core_status(&state)
    })
    .await
    .map_err(|_| "core-status-unavailable".to_owned())?
}

#[tauri::command]
pub(crate) fn queue_outcome_line(label: String) -> String {
    trace_commons_contributor::private_inference_copy::queue_outcome_line(&label).to_owned()
}

#[tauri::command]
pub(crate) fn residual_secret_line(count: u32, sites: Vec<String>) -> String {
    trace_commons_contributor::preview_copy::residual_secret_line(count, &sites)
}

#[tauri::command]
pub(crate) fn redaction_summary_copy(
    redactions: std::collections::BTreeMap<String, u32>,
    distinct: Option<std::collections::BTreeMap<String, u32>>,
) -> serde_json::Value {
    serde_json::json!(trace_commons_contributor::redaction_summary::summary_copy(
        &redactions,
        &distinct.unwrap_or_default(),
    ))
}

/// The ignore-project words, assembled in the contributor core
/// (`project_copy::ignore_project_copy`).
#[tauri::command]
pub(crate) fn project_ignore_copy(project_label: String, pending: usize) -> serde_json::Value {
    serde_json::json!(
        trace_commons_contributor::project_copy::ignore_project_copy(&project_label, pending)
    )
}

/// The arming offer and confirmation, assembled in the contributor core
/// (`project_copy::arming_offer_copy`): the evidence and question, both
/// confirmation bodies (K5's `body_with_backlog` for `include_backlog`) and
/// Customize's table. DRAFT, NEEDS APPROVAL where that module says so.
#[tauri::command]
pub(crate) fn arming_offer_copy(project_label: String, count: u32) -> serde_json::Value {
    serde_json::json!(trace_commons_contributor::project_copy::arming_offer_copy(
        &project_label,
        count
    ))
}

#[tauri::command]
pub(crate) fn attestation_copy(label: String, reason: Option<String>) -> serde_json::Value {
    use trace_commons_contributor::private_inference_copy::{
        PrivateInferenceTone, attestation_reason_line, attestation_state_line,
        attestation_state_tone,
    };

    let reason_line = reason
        .as_deref()
        .map(attestation_reason_line)
        .filter(|line| !line.is_empty());
    let tone = match attestation_state_tone(&label) {
        PrivateInferenceTone::Clear => "clear",
        PrivateInferenceTone::Attention => "attention",
        PrivateInferenceTone::Held => "held",
        PrivateInferenceTone::Refused => "refused",
        PrivateInferenceTone::Neutral => "neutral",
    };
    serde_json::json!({
        "state_line": attestation_state_line(&label),
        "reason_line": reason_line,
        "tone": tone,
    })
}

#[tauri::command]
pub(crate) fn certificate_copy(evidence_admitted: bool) -> serde_json::Value {
    let copy = trace_commons_contributor::private_inference_copy::private_inference_copy();
    serde_json::json!({
        "list_title": trace_commons_contributor::private_inference_copy::certificate_list_title(evidence_admitted),
        "row_line": trace_commons_contributor::private_inference_copy::certificate_row_line(evidence_admitted),
        "list_empty": copy.certificate_list_empty,
    })
}

#[tauri::command]
pub(crate) fn eligibility_copy(label: String, reason: Option<String>) -> serde_json::Value {
    use trace_commons_contributor::private_inference_copy::{
        ContributionControl, eligibility_control, eligibility_reason_line, eligibility_state_line,
    };

    serde_json::json!({
        "state_line": eligibility_state_line(&label),
        "reason_line": reason.as_deref().map(eligibility_reason_line).unwrap_or_default(),
        "can_contribute": eligibility_control(&label) == ContributionControl::Contribute,
    })
}

#[tauri::command]
pub(crate) fn eligibility_group_copy(
    pending: u64,
    contributable: Option<u64>,
) -> serde_json::Value {
    use trace_commons_contributor::private_inference_copy::{
        group_eligibility, group_withheld_line,
    };

    // The eligible/withheld arithmetic itself -- `min`-clamping `contributable`
    // to `pending`, then the non-negative remainder -- used to live here.
    // `group_eligibility` is now the one place it is computed; this file only
    // chooses the words for its answer.
    let eligibility = group_eligibility(pending, contributable);
    serde_json::json!({
        "can_contribute": eligibility.can_contribute,
        "eligible_count": eligibility.eligible_count,
        "withheld_line": group_withheld_line(eligibility.withheld_count),
    })
}

#[tauri::command]
pub(crate) async fn daemon_call(
    state: State<'_, AppState>,
    method: String,
    params: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    if !READ_ONLY_METHODS.contains(&method.as_str()) {
        return Err("IPC method is not enabled in this read-only Tauri command".to_owned());
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
    let mut preview = call_daemon(
        shared_state(&state)?,
        "preview",
        serde_json::json!({ "entry_id": entry_id }),
    )
    .await?;
    let preview_object = preview
        .as_object_mut()
        .ok_or_else(|| "preview-response-invalid".to_owned())?;
    preview_object.insert(
        "gate_statement".to_owned(),
        serde_json::json!(trace_commons_contributor::consent_copy::consent_copy().gate_statement),
    );
    Ok(preview)
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
    outcome: Option<String>,
    correction: Option<String>,
) -> Result<serde_json::Value, String> {
    let mut params = serde_json::json!({ "entry_id": entry_id });
    if let Some(outcome) = outcome {
        params["outcome"] = serde_json::json!(outcome);
    }
    if let Some(correction) = correction {
        params["correction"] = serde_json::json!(correction);
    }
    call_daemon(shared_state(&state)?, "approve", params).await
}

#[tauri::command]
pub(crate) async fn approve_project(
    state: State<'_, AppState>,
    project_id: String,
    outcome: Option<String>,
) -> Result<serde_json::Value, String> {
    let mut params = serde_json::json!({ "project_id": project_id });
    if let Some(outcome) = outcome {
        params["outcome"] = serde_json::json!(outcome);
    }
    call_daemon(shared_state(&state)?, "approve", params).await
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

#[cfg(test)]
mod tests {
    use super::eligibility_group_copy;

    /// The eligible/withheld arithmetic itself now lives in the core's
    /// `private_inference_copy::group_eligibility`; this pins that this
    /// command's JSON shape (and the clamp/remainder values it reports)
    /// did not change when the arithmetic moved out of this file.
    #[test]
    fn eligibility_group_copy_clamps_and_reports_the_withheld_line() {
        let value = eligibility_group_copy(7, Some(3));
        assert_eq!(value["can_contribute"], true);
        assert_eq!(value["eligible_count"], 3);
        assert_eq!(
            value["withheld_line"],
            "4 sessions here cannot be sent, so they are not included."
        );

        // An absent `contributable_count`: every pending session is
        // sendable, and nothing is withheld.
        let value = eligibility_group_copy(7, None);
        assert_eq!(value["can_contribute"], true);
        assert_eq!(value["eligible_count"], 7);
        assert_eq!(value["withheld_line"], "");

        // `contributable` above `pending` is clamped rather than producing
        // a negative withheld count.
        let value = eligibility_group_copy(3, Some(9));
        assert_eq!(value["eligible_count"], 3);
        assert_eq!(value["withheld_line"], "");
    }
}
