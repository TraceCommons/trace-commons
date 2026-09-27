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

/// The notice for one element of `status.grant_voids`: the words, and the
/// choice between the project and the automatic-grant wording, both from the
/// contributor core, which also words an element it cannot place. `null`
/// only for a value that is not an element at all.
#[tauri::command]
pub(crate) fn grant_void_notice(void: serde_json::Value) -> serde_json::Value {
    trace_commons_contributor::consent_copy::void_notice_for_wire(&void)
        .and_then(|copy| serde_json::to_value(copy).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// Record that the notices with these ids were shown. Acknowledging re-arms
/// nothing.
#[tauri::command]
pub(crate) async fn acknowledge_grant_voids(
    state: State<'_, AppState>,
    ids: Vec<u64>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "acknowledge_grant_voids",
        serde_json::json!({ "ids": ids }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{grant_void_notice, scrubber_pattern_names};

    #[test]
    fn a_void_notice_is_the_contributor_cores_copy() {
        let wire = serde_json::json!({
            "id": 1, "kind": "project", "project_id": "p", "project_label": "api",
            "reasons": ["witness-measurement-admitted"], "voided_at": "2026-09-26T00:00:00Z",
        });
        let expected = serde_json::to_value(
            trace_commons_contributor::consent_copy::void_notice_for_wire(&wire).unwrap(),
        )
        .unwrap();
        assert_eq!(grant_void_notice(wire), expected);
    }

    #[test]
    fn only_a_value_that_is_not_an_element_gets_null() {
        assert!(grant_void_notice(serde_json::json!("project")).is_null());
        // An element the core cannot place still gets the core's words.
        assert!(grant_void_notice(serde_json::json!({ "kind": "folder" }))["title"].is_string());
    }

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
