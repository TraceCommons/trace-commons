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

/// The ranges below are a pure unit conversion of
/// `trace_commons_contributor::daemon::settings::settings_ranges()` into the
/// minute/hour/megabyte granularity each control here is scaled in -- a
/// shell UI concern the core does not have an opinion on. The *range* itself
/// is enforced exactly once, by the core's `set_settings` (via
/// `apply_settings_object`), which every command below now reaches with no
/// range check of its own: a value this conversion lets through but the core
/// still refuses is rejected there, with the shared, label-only
/// `settings-invalid-value`, identically for every caller of `set_settings`,
/// not only this one.
#[tauri::command]
pub(crate) async fn set_quiescence_minutes(
    state: State<'_, AppState>,
    minutes: u64,
) -> Result<serde_json::Value, String> {
    set_numeric_setting(state, "quiescence_secs", (minutes * 60).into()).await
}

#[tauri::command]
pub(crate) async fn set_approval_hold_seconds(
    state: State<'_, AppState>,
    seconds: u64,
) -> Result<serde_json::Value, String> {
    set_numeric_setting(state, "approval_hold_secs", seconds.into()).await
}

#[tauri::command]
pub(crate) async fn set_digest_hours(
    state: State<'_, AppState>,
    hours: u64,
) -> Result<serde_json::Value, String> {
    set_numeric_setting(state, "digest_interval_secs", (hours * 3600).into()).await
}

#[tauri::command]
pub(crate) async fn set_max_uploads_per_day(
    state: State<'_, AppState>,
    uploads: u64,
) -> Result<serde_json::Value, String> {
    set_numeric_setting(state, "max_uploads_per_day", uploads.into()).await
}

#[tauri::command]
pub(crate) async fn set_max_bytes_per_day_mb(
    state: State<'_, AppState>,
    megabytes: u64,
) -> Result<serde_json::Value, String> {
    set_numeric_setting(state, "max_bytes_per_day", (megabytes * 1024 * 1024).into()).await
}

/// The ranges a shell draws its quiescence/approval-hold/digest/daily-cap
/// controls from, converted from the core's seconds/bytes into the
/// minute/hour/megabyte units those controls use. One conversion, so the
/// frontend never hand-maintains a second copy of the core's numbers.
#[tauri::command]
pub(crate) fn settings_ranges() -> serde_json::Value {
    let ranges = trace_commons_contributor::daemon::settings::settings_ranges();
    serde_json::json!({
        "quiescence_minutes": {
            "min": ranges.quiescence_secs.min.div_ceil(60),
            "max": ranges.quiescence_secs.max / 60,
        },
        "approval_hold_seconds": {
            "min": ranges.approval_hold_secs.min,
            "max": ranges.approval_hold_secs.max,
        },
        "digest_hours": {
            "min": ranges.digest_interval_secs.min.div_ceil(3600),
            "max": ranges.digest_interval_secs.max / 3600,
        },
        "max_uploads_per_day": {
            "min": ranges.max_uploads_per_day.min,
            "max": ranges.max_uploads_per_day.max,
        },
        "max_bytes_per_day_mb": {
            "min": ranges.max_bytes_per_day.min.div_ceil(1024 * 1024),
            "max": ranges.max_bytes_per_day.max / (1024 * 1024),
        },
    })
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
    // The declaration is saved at this point. A start failure is reported as
    // `daemon_started: false`, not as a failed save; onboarding retries the
    // start and says truthfully that the watcher did not start.
    let daemon_started = roots_ready && ensure_daemon_started(&state).await.is_ok();
    if daemon_started {
        start_event_bridge(app);
    }
    Ok(serde_json::json!({
        "saved": true,
        "daemon_started": daemon_started,
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

#[cfg(test)]
mod tests {
    use super::settings_ranges;

    /// The minute/hour/megabyte ranges this shell draws its controls from
    /// are a unit conversion of the core's own seconds/bytes ranges, not a
    /// second, hand-typed copy of the numbers this file used to clamp to
    /// locally (0..=300 seconds, 1..=24 hours, 1..=1000 uploads, 1..=5120
    /// MB) before that clamp moved into `apply_settings_object`. A drift
    /// here would mean the control's own bounds no longer match what
    /// `set_settings` actually accepts.
    ///
    /// `quiescence_minutes.min` is 0, not the old control's 1: the core's
    /// floor is 0 seconds (instant quiescence is meaningful, not an
    /// accident -- see `daemon::settings::QUIESCENCE_SECS_MIN`'s doc), and a
    /// one-minute floor was this shell's own, narrower, UI choice. The
    /// exported range is the core's real floor, which a shell's control is
    /// free to round up from but must not read as wider than.
    #[test]
    fn the_exported_ranges_match_the_controls_this_shell_used_to_hard_code() {
        let ranges = settings_ranges();
        assert_eq!(ranges["quiescence_minutes"]["min"], 0);
        assert_eq!(ranges["quiescence_minutes"]["max"], 240);
        assert_eq!(ranges["approval_hold_seconds"]["min"], 0);
        assert_eq!(ranges["approval_hold_seconds"]["max"], 300);
        assert_eq!(ranges["digest_hours"]["min"], 1);
        assert_eq!(ranges["digest_hours"]["max"], 24);
        assert_eq!(ranges["max_uploads_per_day"]["min"], 1);
        assert_eq!(ranges["max_uploads_per_day"]["max"], 1_000);
        assert_eq!(ranges["max_bytes_per_day_mb"]["min"], 1);
        assert_eq!(ranges["max_bytes_per_day_mb"]["max"], 5_120);
    }
}
