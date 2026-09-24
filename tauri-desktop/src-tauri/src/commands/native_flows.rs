use serde_json::{Value, json};
use tauri::State;

use crate::{
    ipc::{call_daemon, call_daemon_response, shared_state},
    state::{AppState, DaemonConnection},
};

const WALLET_ACTIONS: &[&str] = &["open", "check", "start", "wait", "cancel"];

fn required<'a>(value: &'a str, label: &'static str) -> Result<&'a str, String> {
    let value = value.trim();
    (!value.is_empty())
        .then_some(value)
        .ok_or_else(|| label.to_owned())
}

fn wallet_action(value: &str) -> Result<&str, String> {
    let value = required(value, "native-wallet-action-required")?;
    WALLET_ACTIONS
        .contains(&value)
        .then_some(value)
        .ok_or_else(|| "native-wallet-action-invalid".to_owned())
}

async fn call_result_or_view(
    daemon: DaemonConnection,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let response = call_daemon_response(daemon, method, params).await?;
    if let Some(result) = response.result {
        return Ok(result);
    }
    match response.error {
        Some(error) => Err(format!("{}: {}", error.code, error.message)),
        None => Err("Rust core returned an invalid IPC response".to_owned()),
    }
}

fn near_ai_error_view(label: &str) -> Value {
    json!({
        "enrolled": false,
        "message": trace_commons_contributor::private_inference_copy::near_ai_enroll_line(label),
    })
}

#[tauri::command]
pub(crate) async fn native_wallet_flow(
    state: State<'_, AppState>,
    action: String,
    flow_id: String,
    commons: String,
    account: String,
) -> Result<Value, String> {
    let action = wallet_action(&action)?;
    if matches!(action, "wait" | "cancel") {
        required(&flow_id, "native-wallet-flow-required")?;
    }
    let params = json!({
        "action": action,
        "flow_id": flow_id.trim(),
        "ingest_url": commons.trim(),
        "account_id": account.trim(),
    });
    let result = call_daemon(shared_state(&state)?, "native_wallet_flow", params).await?;
    if action == "start" {
        state
            .inner()
            .authorize_wallet_url(result.get("browser_url").and_then(Value::as_str))?;
    } else if action == "cancel" {
        state.inner().authorize_wallet_url(None)?;
    }
    Ok(result)
}

#[tauri::command]
pub(crate) fn contributor_disclosure_copy() -> Value {
    let witness = trace_commons_contributor::witness_copy::witness_copy();
    let inference = trace_commons_contributor::private_inference_copy::private_inference_copy();
    // The contributor core names this hold once, in its shared status table;
    // History reads that label rather than keeping a second spelling here.
    let awaiting_pii_backstop = trace_commons_contributor::public_run::public_run_copy()
        .contribution_status_choices
        .into_iter()
        .find(|choice| choice.value == "awaiting_pii_backstop")
        .map(|choice| choice.label);
    let source_checks = ["claude", "codex", "gemini", "cline", "opencode"]
        .into_iter()
        .filter_map(|key| {
            trace_commons_contributor::source_copy::SourceTool::from_key(key).map(|tool| {
                (key.to_owned(), json!({
                    "watch": trace_commons_contributor::source_copy::source_check_line(tool, "watch"),
                    "unset": trace_commons_contributor::source_copy::source_check_line(tool, "unset"),
                    "off": trace_commons_contributor::source_copy::source_check_line(tool, "off"),
                }))
            })
        })
        .collect::<serde_json::Map<String, Value>>();
    json!({
        "witness_review": witness.review,
        "wallet": witness.wallet,
        "admission": witness.admission,
        "onboarding": witness.onboarding,
        "onboarding_shell": trace_commons_contributor::onboarding_copy::onboarding_copy(),
        "privacy_scan": trace_commons_contributor::privacy_scan_copy::privacy_scan_copy(),
        "source_settings": trace_commons_contributor::source_copy::source_settings_copy(),
        "source_check_lines": source_checks,
        "insights_ui": trace_commons_contributor::insights::service::ui_copy(),
        "mission_drafts_ui": trace_commons_contributor::mission_draft_service::ui_copy(),
        "history_ui": {
            "held_row_body": trace_commons_contributor::history_copy::HELD_ROW_BODY,
            "status_awaiting_pii_backstop": awaiting_pii_backstop,
        },
        "outcome": trace_commons_contributor::outcome_copy::outcome_copy(),
        "private_inference": {
            "destination": inference.destination,
            "subtitle": inference.subtitle,
            "settings_title": inference.settings_title,
            "write_unconfirmed": inference.write_unconfirmed,
            "offer_title": inference.offer_title,
            "offer_what": inference.offer_what,
            "offer_exposure": inference.offer_exposure,
            "offer_no_repoint": inference.offer_no_repoint,
            "offer_accept": inference.offer_accept,
            "offer_decline": inference.offer_decline,
            "offer_asked_once": inference.offer_asked_once,
        },
        "credential_cost": trace_commons_contributor::private_inference_copy::CREDENTIAL_COST,
        "credential_wallet_notice": trace_commons_contributor::private_inference_copy::CREDENTIAL_WALLET_NOTICE,
        "near_ai_enroll_title": inference.near_ai_enroll_title,
        "near_ai_enroll_what": inference.near_ai_enroll_what,
        "near_ai_enroll_action": inference.near_ai_enroll_action,
        "near_ai_enroll_needs_login": inference.near_ai_enroll_needs_login,
    })
}

#[tauri::command]
pub(crate) fn witness_review_copy() -> Value {
    json!(trace_commons_contributor::witness_copy::witness_copy().review)
}

#[tauri::command]
pub(crate) async fn near_ai_account_enroll(
    state: State<'_, AppState>,
    commons: String,
) -> Result<Value, String> {
    required(&commons, "near-ai-commons-required")?;
    let response = call_daemon_response(
        shared_state(&state)?,
        "near_ai_account_enroll",
        json!({ "ingest_url": commons.trim() }),
    )
    .await?;
    if let Some(result) = response.result {
        return Ok(result);
    }
    if let Some(error) = response.error {
        return Ok(near_ai_error_view(&error.message));
    }
    Err("Rust core returned an invalid IPC response".to_owned())
}

#[tauri::command]
pub(crate) async fn prepare_admission_session(
    state: State<'_, AppState>,
    entry_id: String,
    backend: String,
    confirmed: bool,
) -> Result<Value, String> {
    required(&entry_id, "admission-entry-required")?;
    required(&backend, "admission-backend-required")?;
    if !confirmed {
        return Err("admission-confirmation-required".to_owned());
    }
    call_result_or_view(
        shared_state(&state)?,
        "prepare_admission_session",
        json!({
            "entry_id": entry_id.trim(),
            "backend": backend.trim(),
            "confirmed": confirmed,
        }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn witness_preview_support(state: State<'_, AppState>) -> Result<bool, String> {
    let value = call_daemon(shared_state(&state)?, "hello", json!({})).await?;
    Ok(value
        .get("methods")
        .and_then(Value::as_array)
        .is_some_and(|methods| {
            methods
                .iter()
                .any(|method| method.as_str() == Some("witness_preview_request"))
        }))
}

#[tauri::command]
pub(crate) async fn witness_preview_request(
    state: State<'_, AppState>,
    entry_id: String,
    raw_session_confirmed: bool,
) -> Result<Value, String> {
    required(&entry_id, "witness-entry-required")?;
    if !raw_session_confirmed {
        return Err("witness-raw-session-confirmation-required".to_owned());
    }
    call_result_or_view(
        shared_state(&state)?,
        "witness_preview_request",
        json!({
            "entry_id": entry_id.trim(),
            "raw_session_confirmed": raw_session_confirmed,
        }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{contributor_disclosure_copy, near_ai_error_view, wallet_action};

    #[test]
    fn private_ai_copy_carries_the_shared_destination_and_its_surrounding_lines() {
        use trace_commons_contributor::private_inference_copy::{
            DESTINATION, SETTINGS_TITLE, SUBTITLE, WRITE_UNCONFIRMED,
        };
        let copy = contributor_disclosure_copy();
        for (key, expected) in [
            ("destination", DESTINATION),
            ("subtitle", SUBTITLE),
            ("settings_title", SETTINGS_TITLE),
            ("write_unconfirmed", WRITE_UNCONFIRMED),
        ] {
            assert_eq!(
                copy.pointer(&format!("/private_inference/{key}"))
                    .and_then(Value::as_str),
                Some(expected),
                "{key}"
            );
        }
    }

    #[test]
    fn history_copy_names_the_privacy_backstop_hold_from_the_shared_status_table() {
        let copy = contributor_disclosure_copy();
        let shared = trace_commons_contributor::public_run::public_run_copy()
            .contribution_status_choices
            .into_iter()
            .find(|choice| choice.value == "awaiting_pii_backstop")
            .expect("the shared status table names awaiting_pii_backstop")
            .label;
        assert_eq!(
            copy.pointer("/history_ui/status_awaiting_pii_backstop")
                .and_then(Value::as_str),
            Some(shared)
        );
    }

    #[test]
    fn wallet_action_boundary_accepts_only_daemon_lifecycle_actions() {
        for action in ["open", "check", "start", "wait", "cancel"] {
            assert_eq!(wallet_action(action).unwrap(), action);
        }
        for action in ["", "retry", "start;shutdown"] {
            assert!(wallet_action(action).is_err());
        }
    }

    #[test]
    fn near_ai_refusals_cross_as_fixed_copy_without_control_labels() {
        let response = near_ai_error_view("near_ai_enroll_no_session");
        let message = response.get("message").and_then(Value::as_str).unwrap();
        assert!(message.contains("not signed in"));
        assert!(!message.contains("near_ai_enroll_no_session"));

        let unknown = near_ai_error_view("future-label");
        assert_eq!(
            unknown.get("message").and_then(Value::as_str),
            Some("Joining with a NEAR AI login is not available right now. Nothing was joined.")
        );
    }
}
