use std::path::PathBuf;

use tauri::State;

use crate::{
    ipc::{call_daemon, shared_state},
    state::AppState,
};

#[tauri::command]
pub(crate) async fn discover_routing(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "discover_routing",
        serde_json::json!({}),
    )
    .await
}

fn routing_token_dir(token_dir: Option<String>) -> Result<Option<String>, String> {
    let Some(token_dir) = token_dir else {
        return Ok(None);
    };
    let token_dir = token_dir.trim();
    if token_dir.is_empty() {
        return Ok(None);
    }
    if !PathBuf::from(token_dir).is_absolute() {
        return Err("routing-token-dir-must-be-absolute".to_owned());
    }
    Ok(Some(token_dir.to_owned()))
}

#[tauri::command]
pub(crate) async fn configure_routing(
    state: State<'_, AppState>,
    enabled: bool,
    port: u16,
    token_dir: Option<String>,
) -> Result<serde_json::Value, String> {
    let declaration = if !enabled {
        serde_json::Value::Null
    } else {
        if port == 0 {
            return Err("routing-port-invalid".to_owned());
        }
        let mut value = serde_json::json!({ "mode": "watch", "port": port });
        if let Some(token_dir) = routing_token_dir(token_dir)? {
            value["token_dir"] = serde_json::Value::String(token_dir);
        }
        value
    };
    call_daemon(
        shared_state(&state)?,
        "set_settings",
        serde_json::json!({ "ironwire": declaration }),
    )
    .await
}

async fn probe_routing_call(
    state: State<'_, AppState>,
    method: &'static str,
    port: u16,
    token_dir: Option<String>,
) -> Result<serde_json::Value, String> {
    if port == 0 {
        return Err("routing-port-invalid".to_owned());
    }
    let token_dir = routing_token_dir(token_dir)?;
    call_daemon(
        shared_state(&state)?,
        method,
        serde_json::json!({ "port": port, "token_dir": token_dir }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn probe_routing(
    state: State<'_, AppState>,
    port: u16,
    token_dir: Option<String>,
) -> Result<serde_json::Value, String> {
    probe_routing_call(state, "probe_routing", port, token_dir).await
}

#[tauri::command]
pub(crate) async fn probe_routed_tools(
    state: State<'_, AppState>,
    port: u16,
    token_dir: Option<String>,
) -> Result<serde_json::Value, String> {
    probe_routing_call(state, "probe_routed_tools", port, token_dir).await
}

#[cfg(test)]
mod tests {
    use super::routing_token_dir;

    #[test]
    fn routing_token_directory_is_optional_but_never_relative() {
        assert_eq!(routing_token_dir(None).unwrap(), None);
        assert_eq!(routing_token_dir(Some("  ".to_owned())).unwrap(), None);
        assert_eq!(
            routing_token_dir(Some("/tmp/ironwire".to_owned())).unwrap(),
            Some("/tmp/ironwire".to_owned())
        );
        assert_eq!(
            routing_token_dir(Some(".ironwire".to_owned())).unwrap_err(),
            "routing-token-dir-must-be-absolute"
        );
    }
}
