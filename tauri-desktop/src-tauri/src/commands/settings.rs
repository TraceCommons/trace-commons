use tauri::{AppHandle, Runtime, State};
use trace_commons_contributor::{config::ConfigStore, daemon::settings::DaemonSettings};

use crate::{
    app::start_event_bridge,
    commands::platform::existing_directory,
    ipc::{call_daemon, optional_shared_state, shared_state},
    runtime::ensure_daemon_started,
    state::{AppState, state_directory},
};

async fn set_numeric_setting(
    state: State<'_, AppState>,
    field: &'static str,
    value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "set_settings",
        serde_json::json!({ field: value }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn set_quiescence_minutes(
    state: State<'_, AppState>,
    minutes: u64,
) -> Result<serde_json::Value, String> {
    if !(1..=240).contains(&minutes) {
        return Err("quiescence-minutes-out-of-range".to_owned());
    }
    set_numeric_setting(state, "quiescence_secs", (minutes * 60).into()).await
}

#[tauri::command]
pub(crate) async fn set_approval_hold_seconds(
    state: State<'_, AppState>,
    seconds: u64,
) -> Result<serde_json::Value, String> {
    if seconds > 300 {
        return Err("approval-hold-seconds-out-of-range".to_owned());
    }
    set_numeric_setting(state, "approval_hold_secs", seconds.into()).await
}

#[tauri::command]
pub(crate) async fn set_digest_hours(
    state: State<'_, AppState>,
    hours: u64,
) -> Result<serde_json::Value, String> {
    if !(1..=24).contains(&hours) {
        return Err("digest-hours-out-of-range".to_owned());
    }
    set_numeric_setting(state, "digest_interval_secs", (hours * 3600).into()).await
}

#[tauri::command]
pub(crate) async fn set_max_uploads_per_day(
    state: State<'_, AppState>,
    uploads: u64,
) -> Result<serde_json::Value, String> {
    if !(1..=1_000).contains(&uploads) {
        return Err("max-uploads-out-of-range".to_owned());
    }
    set_numeric_setting(state, "max_uploads_per_day", uploads.into()).await
}

#[tauri::command]
pub(crate) async fn set_max_bytes_per_day_mb(
    state: State<'_, AppState>,
    megabytes: u64,
) -> Result<serde_json::Value, String> {
    if !(1..=5_120).contains(&megabytes) {
        return Err("max-bytes-out-of-range".to_owned());
    }
    set_numeric_setting(state, "max_bytes_per_day", (megabytes * 1024 * 1024).into()).await
}
#[tauri::command]
pub(crate) async fn set_source_declaration<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    source: String,
    mode: String,
    path: Option<String>,
) -> Result<serde_json::Value, String> {
    let field = match source.as_str() {
        "claude" => "claude_source",
        "codex" => "codex_source",
        "gemini" => "gemini_source",
        "cline" => "cline_source",
        "opencode" => "opencode_source",
        _ => return Err("source-root-unknown".to_owned()),
    };
    let declaration = match mode.as_str() {
        "off" => serde_json::json!({ "mode": "off" }),
        "watch" => {
            let path = path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "source-root-path-required".to_owned())?;
            let path = existing_directory(path, "source-root-directory-required")?;
            serde_json::json!({ "mode": "watch", "path": path })
        }
        _ => return Err("source-root-mode-unknown".to_owned()),
    };
    let params = serde_json::json!({ field: declaration });
    if let Some(shared) = optional_shared_state(&state)? {
        return call_daemon(shared, "set_settings", params).await;
    }

    let store = ConfigStore::open(state_directory(&state)?).map_err(|_| "settings-write-failed")?;
    let mut settings =
        DaemonSettings::load_for_preferences(&store).map_err(|_| "settings-write-failed")?;
    trace_commons_contributor::daemon::settings::apply_settings_object(&mut settings, &params)
        .map_err(|label| label.to_owned())?;
    let roots_ready = trace_commons_contributor::daemon::settings::roots_declared(&settings);
    settings.save(&store).map_err(|_| "settings-write-failed")?;
    if roots_ready {
        ensure_daemon_started(&state).await?;
        start_event_bridge(app);
    }
    Ok(serde_json::json!({
        "saved": true,
        "daemon_started": roots_ready,
    }))
}

#[tauri::command]
pub(crate) async fn set_project_mode(
    state: State<'_, AppState>,
    project_id: String,
    mode: String,
) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "set_project_mode",
        serde_json::json!({ "project_id": project_id, "mode": mode }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn list_projects(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "list_projects",
        serde_json::json!({}),
    )
    .await
}

#[tauri::command]
pub(crate) async fn list_audit(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    call_daemon(
        shared_state(&state)?,
        "list_audit",
        serde_json::json!({ "limit": 20 }),
    )
    .await
}
