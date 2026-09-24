use tauri::State;

use crate::{
    ipc::{call_daemon, optional_shared_state, shared_state},
    state::AppState,
};

#[tauri::command]
pub(crate) async fn consent_options(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let Some(shared) = optional_shared_state(&state)? else {
        return Ok(trace_commons_contributor::daemon::enroll::consent_options());
    };
    call_daemon(shared, "consent_options", serde_json::json!({})).await
}

#[tauri::command]
pub(crate) fn scrubber_pattern_names() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "names": trace_commons_contributor::secret_leak_pattern_names(),
    }))
}

#[tauri::command]
pub(crate) async fn enroll_with_invite(
    state: State<'_, AppState>,
    invite: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "enroll",
        serde_json::json!({ "invite": invite }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn set_consent_scopes(
    state: State<'_, AppState>,
    scopes: Vec<String>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "set_consent_scopes",
        serde_json::json!({ "scopes": scopes }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn acknowledge_near_ai_notice(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "acknowledge_near_ai_notice",
        serde_json::json!({}),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::scrubber_pattern_names;

    #[test]
    fn scrubber_disclosure_exposes_names_without_patterns() {
        let value = scrubber_pattern_names().expect("scrubber names response");
        let names = value["names"].as_array().expect("names array");
        assert!(!names.is_empty());
        assert!(
            names
                .iter()
                .all(|name| name.as_str().is_some_and(|name| !name.contains("regex")))
        );
    }
}
