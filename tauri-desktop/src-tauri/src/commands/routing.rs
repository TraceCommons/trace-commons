use tauri::State;
use trace_commons_contributor::daemon::settings::{
    validate_routing_port, validate_routing_token_dir,
};

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
        validate_routing_port(port)?;
        let mut value = serde_json::json!({ "mode": "watch", "port": port });
        if let Some(token_dir) = validate_routing_token_dir(token_dir.as_deref())? {
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
    validate_routing_port(port)?;
    let token_dir = validate_routing_token_dir(token_dir.as_deref())?;
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
    use super::validate_routing_token_dir;

    /// The port/token-dir validation this file used to implement locally is
    /// now `trace_commons_contributor::daemon::settings::validate_routing_
    /// token_dir`/`validate_routing_port`, which `set_settings` itself also
    /// enforces (see that crate's own tests for the port-zero and
    /// relative-path cases). This pins that this file reaches the moved
    /// function under the same import path a caller of this module expects.
    #[test]
    fn routing_token_directory_is_optional_but_never_relative() {
        assert_eq!(validate_routing_token_dir(None).unwrap(), None);
        assert_eq!(validate_routing_token_dir(Some("  ")).unwrap(), None);
        let absolute = std::env::temp_dir()
            .join("ironwire")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            validate_routing_token_dir(Some(&absolute)).unwrap(),
            Some(absolute)
        );
        assert_eq!(
            validate_routing_token_dir(Some(".ironwire")).unwrap_err(),
            "routing-token-dir-must-be-absolute"
        );
    }
}
