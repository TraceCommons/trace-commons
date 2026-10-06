use tauri::State;
use trace_commons_contributor::{config::ConfigStore, daemon::settings::DaemonSettings};

use crate::{
    ipc::{call_daemon, shared_state},
    state::{AppState, state_directory},
};

#[tauri::command]
pub(crate) async fn start_private_ai_credential(
    state: State<'_, AppState>,
    provider: Option<String>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "near_ai_credential_start",
        serde_json::json!({ "provider": provider.unwrap_or_else(|| "github".to_owned()) }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn private_ai_credential_status(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let mut value = call_daemon(
        shared_state(&state)?,
        "near_ai_credential_status",
        serde_json::json!({}),
    )
    .await?;
    if let Some(label) = value.get("state").and_then(serde_json::Value::as_str) {
        let action =
            match trace_commons_contributor::private_inference_copy::credential_action(label) {
                trace_commons_contributor::private_inference_copy::CredentialAction::Obtain => {
                    "obtain"
                }
                trace_commons_contributor::private_inference_copy::CredentialAction::Cancel => {
                    "cancel"
                }
                trace_commons_contributor::private_inference_copy::CredentialAction::Forget => {
                    "forget"
                }
                trace_commons_contributor::private_inference_copy::CredentialAction::Migrate => {
                    "migrate"
                }
                trace_commons_contributor::private_inference_copy::CredentialAction::None => "none",
            };
        let mut view = serde_json::json!({
            "state_line": trace_commons_contributor::private_inference_copy::credential_state_line(label),
            "action": action,
        });
        // The move's button and the sentence beside it come from the core
        // copy module, like every other shell's, rather than being spelled in
        // the frontend: it is the one action that can raise a macOS password
        // prompt, and the sentence that warns about that must not drift.
        if action == "migrate" {
            view["action_label"] = serde_json::Value::String(
                trace_commons_contributor::private_inference_copy::CREDENTIAL_MIGRATE.to_owned(),
            );
            view["action_explains"] = serde_json::Value::String(
                trace_commons_contributor::private_inference_copy::CREDENTIAL_MIGRATE_EXPLAINS
                    .to_owned(),
            );
        }
        value["view"] = view;
    }
    let store = ConfigStore::open(state_directory(&state)?)
        .map_err(|_| "credential-storage-unavailable".to_owned())?;
    let loaded = tauri::async_runtime::spawn_blocking(move || {
        DaemonSettings::load_with_cloud_credentials(&store)
    })
    .await
    .map_err(|_| "credential-storage-unavailable".to_owned())?;
    // A credential this process cannot load is a state the daemon's own
    // answer above already names (`migration_available`,
    // `storage_unentitled`, `storage_unavailable`), with the action that goes
    // with it. Failing the whole call here threw that answer away, so the
    // contributor saw an error instead of the button that fixes it.
    // The "keychain" block's assembly lives in the core (K7, #1173), so
    // every shell -- this one and macOS over the C ABI -- shows the
    // identical fields the identical way.
    value["keychain"] = match loaded {
        Ok(settings) => settings.keychain_status_json(),
        Err(_) => trace_commons_contributor::daemon::settings::keychain_status_unavailable_json(),
    };
    Ok(value)
}

#[tauri::command]
pub(crate) async fn private_ai_balance(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let mut value = call_daemon(
        shared_state(&state)?,
        "near_ai_balance",
        serde_json::json!({}),
    )
    .await?;
    let Some(state) = value.get("state").and_then(serde_json::Value::as_str) else {
        return Ok(value);
    };
    let Some(scale) = value
        .get("scale")
        .and_then(serde_json::Value::as_u64)
        .and_then(|scale| u8::try_from(scale).ok())
    else {
        return Ok(value);
    };
    let optional_i64 = |key: &str| value.get(key).and_then(serde_json::Value::as_i64);
    value["view"] = serde_json::json!({
        "state_line": trace_commons_contributor::private_inference_copy::balance_state_line(state),
        "remaining_line": trace_commons_contributor::private_inference_copy::balance_remaining_line(optional_i64("remaining_nanos"), scale),
        "limit_line": trace_commons_contributor::private_inference_copy::balance_limit_line(optional_i64("spend_limit_nanos"), scale),
        "spent_line": trace_commons_contributor::private_inference_copy::balance_spent_line(optional_i64("total_spent_nanos"), scale),
    });
    Ok(value)
}

#[tauri::command]
pub(crate) async fn private_ai_funding(
    state: State<'_, AppState>,
    expected_organization_id: Option<String>,
    expected_connection_revision: Option<String>,
) -> Result<serde_json::Value, String> {
    let params = match (expected_organization_id, expected_connection_revision) {
        (Some(organization_id), Some(connection_revision)) => serde_json::json!({
            "expected_organization_id": organization_id,
            "expected_connection_revision": connection_revision,
        }),
        (None, None) => serde_json::json!({}),
        _ => serde_json::json!({ "expected_organization_id": null }),
    };
    call_daemon(shared_state(&state)?, "near_ai_funding", params).await
}

#[tauri::command]
pub(crate) async fn private_ai_harnesses(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let mut value =
        call_daemon(shared_state(&state)?, "harness_list", serde_json::json!({})).await?;
    let copy = trace_commons_contributor::private_inference_copy::private_inference_copy();
    let credentialed = value
        .get("destination_credentialed")
        .and_then(serde_json::Value::as_bool);
    let state_lines = value
        .get("harnesses")
        .and_then(serde_json::Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let id = row.get("id")?.as_str()?;
                    let state = row.get("state")?.as_str()?;
                    Some((
                        id.to_owned(),
                        trace_commons_contributor::private_inference_copy::harness_state_line(
                            state,
                        ),
                    ))
                })
                .collect::<std::collections::HashMap<_, _>>()
        })
        .unwrap_or_default();
    value["view"] = serde_json::json!({
        "title": copy.harnesses_title,
        "what": copy.harnesses_what,
        "spend_scope": copy.harnesses_spend_scope,
        "none_found": copy.harnesses_none_found,
        "credential_notice": trace_commons_contributor::private_inference_copy::harness_credential_notice(credentialed),
        "state_lines": state_lines,
        "spend_line": value
            .get("spend")
            .and_then(|spend| spend.get("micros"))
            .and_then(serde_json::Value::as_u64)
            .map(|micros| trace_commons_contributor::private_inference_copy::harness_spend_line(Some(micros)))
            .unwrap_or_default(),
    });
    Ok(value)
}

#[tauri::command]
pub(crate) async fn plan_harness(
    state: State<'_, AppState>,
    id: String,
    action: String,
) -> Result<serde_json::Value, String> {
    let mut value = call_daemon(
        shared_state(&state)?,
        "harness_plan",
        serde_json::json!({ "id": id, "action": action }),
    )
    .await?;
    let copy = trace_commons_contributor::private_inference_copy::private_inference_copy();
    let outcome = value
        .get("outcome")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let can_commit = outcome == "changes"
        && value
            .get("plan_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|plan_id| !plan_id.is_empty());
    value["view"] = serde_json::json!({
        "preview_title": copy.harness_preview_title,
        "confirm": copy.harness_preview_confirm,
        "cancel": copy.harness_preview_cancel,
        "slot_taken": copy.harness_slot_taken,
        "outcome_line": trace_commons_contributor::private_inference_copy::harness_outcome_line(outcome),
        "can_commit": can_commit,
    });
    Ok(value)
}

#[tauri::command]
pub(crate) async fn commit_harness(
    state: State<'_, AppState>,
    plan_id: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "harness_commit",
        serde_json::json!({ "plan_id": plan_id }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn cancel_private_ai_credential(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "near_ai_credential_cancel",
        serde_json::json!({}),
    )
    .await
}

/// Copy a sign-in an earlier macOS build kept in the login keychain into the
/// store this build uses. Contributor-initiated: the daemon's legacy read may
/// raise a macOS password prompt, and this button is the only way to cause it.
#[tauri::command]
pub(crate) async fn migrate_private_ai_credential(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "near_ai_credential_migrate",
        serde_json::json!({}),
    )
    .await
}

#[tauri::command]
pub(crate) async fn forget_private_ai_credential(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "near_ai_credential_forget",
        serde_json::json!({}),
    )
    .await
}
