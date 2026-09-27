use tauri::State;
use trace_commons_contributor::config::{ConfigStore, ContributorConfig};

use crate::{
    ipc::{call_daemon, optional_shared_state, shared_state},
    state::{AppState, state_directory},
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

/// The notice for approved sessions held because the privacy witness is
/// busy, from `status.witness_capacity` passed through: the words and the
/// count, both from the contributor core. `null` when nothing is waiting.
#[tauri::command]
pub(crate) fn witness_capacity_notice(capacity: serde_json::Value) -> serde_json::Value {
    trace_commons_contributor::consent_copy::witness_capacity_notice_for_wire(&capacity)
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

/// What the Tauri shell can check before asking for the Flow 1 grant.
///
/// The daemon's `grant_automatic` refuses only when there are no terms to
/// grant under; it does not check R7. So the shell refuses first when the
/// contributor has not confirmed the grant screens, is not enrolled, or has
/// no data-use scope saved -- the scope picker has no default, so an empty
/// list means nobody chose. The labels are fixed and carry no content.
fn grant_precondition(
    confirmed: bool,
    config: Option<&ContributorConfig>,
) -> Result<(), &'static str> {
    if !confirmed {
        return Err("automatic-grant-confirmation-required");
    }
    let Some(config) = config else {
        return Err("automatic-grant-not-enrolled");
    };
    if config.consent_scopes.is_empty() {
        return Err("automatic-grant-scope-required");
    }
    Ok(())
}

pub(crate) fn load_config(
    state: &State<'_, AppState>,
) -> Result<Option<ContributorConfig>, String> {
    let store = ConfigStore::open(state_directory(state)?)
        .map_err(|_| "contributor-config-unreadable".to_owned())?;
    store
        .load_config()
        .map_err(|_| "contributor-config-unreadable".to_owned())
}

/// Whether the Flow 1 grant is in force, as the daemon reports it.
#[tauri::command]
pub(crate) async fn automatic_grant(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "automatic_grant",
        serde_json::json!({}),
    )
    .await
}

/// Give the Flow 1 grant: arm projects discovered from now on, never what is
/// on disk. `confirmed` is the grant screen's button, pressed after the
/// scope, path and disclosure steps.
#[tauri::command]
pub(crate) async fn grant_automatic(
    state: State<'_, AppState>,
    confirmed: bool,
) -> Result<serde_json::Value, String> {
    grant_precondition(confirmed, load_config(&state)?.as_ref()).map_err(str::to_owned)?;
    call_daemon(
        shared_state(&state)?,
        "grant_automatic",
        serde_json::json!({}),
    )
    .await
}

/// Withdraw the Flow 1 grant. Projects it armed keep their own modes.
#[tauri::command]
pub(crate) async fn withdraw_automatic_grant(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "withdraw_automatic_grant",
        serde_json::json!({}),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{grant_precondition, grant_void_notice, scrubber_pattern_names};

    fn config(scopes: &[&str]) -> trace_commons_contributor::config::ContributorConfig {
        serde_json::from_value(serde_json::json!({
            "schema_version": trace_commons_contributor::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,
            "issuer_url": "https://issuer.invalid",
            "ingest_url": "https://ingest.invalid",
            "audience": "aud",
            "tenant_id": "tenant-1",
            "instance_id": "instance-1",
            "user_subject": "alice",
            "device_key_id": "sha256:aa",
            "consent_scopes": scopes,
        }))
        .expect("a contributor config")
    }

    #[test]
    fn the_grant_needs_confirmation_enrollment_and_a_chosen_scope() {
        let chosen = config(&["debugging_evaluation"]);
        assert_eq!(
            grant_precondition(false, Some(&chosen)),
            Err("automatic-grant-confirmation-required")
        );
        assert_eq!(
            grant_precondition(true, None),
            Err("automatic-grant-not-enrolled")
        );
        assert_eq!(
            grant_precondition(true, Some(&config(&[]))),
            Err("automatic-grant-scope-required")
        );
        assert_eq!(grant_precondition(true, Some(&chosen)), Ok(()));
    }

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
