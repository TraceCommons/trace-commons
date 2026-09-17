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
    call_daemon(shared_state(&state)?, "native_wallet_flow", params).await
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
) -> Result<Value, String> {
    required(&entry_id, "admission-entry-required")?;
    required(&backend, "admission-backend-required")?;
    call_result_or_view(
        shared_state(&state)?,
        "prepare_admission_session",
        json!({
            "entry_id": entry_id.trim(),
            "backend": backend.trim(),
            "confirmed": true,
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
) -> Result<Value, String> {
    required(&entry_id, "witness-entry-required")?;
    call_result_or_view(
        shared_state(&state)?,
        "witness_preview_request",
        json!({
            "entry_id": entry_id.trim(),
            "raw_session_confirmed": true,
        }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{near_ai_error_view, wallet_action};

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
