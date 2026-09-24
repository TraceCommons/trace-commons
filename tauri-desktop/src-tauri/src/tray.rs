use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use tauri::{
    AppHandle, Emitter, Manager, Runtime as TauriRuntime,
    menu::{Menu, MenuBuilder, SubmenuBuilder},
    tray::TrayIconBuilder,
};

use crate::{ipc::call_daemon_blocking, state::AppState};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ProjectSummary {
    label: String,
    count: usize,
    size_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct WeeklySummary {
    contributed: usize,
    held: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TraySnapshot {
    decisions_owed: usize,
    paused: bool,
    projects: Vec<ProjectSummary>,
    private_inference_state: Option<String>,
    weekly: Option<WeeklySummary>,
}

/// The tray's Private AI row and, while it is answering calls, its stop
/// action. Both names come from the contributor core: the destination is
/// "Private AI", never the setting's internal name.
fn private_ai_tray_items(state: &str) -> (String, Option<&'static str>) {
    use trace_commons_contributor::private_inference_copy::{DESTINATION, TRAY_TURN_OFF};
    let label = format!("{DESTINATION} · {}", state.replace('_', " "));
    let stop = matches!(state, "running" | "running_without_backends").then_some(TRAY_TURN_OFF);
    (label, stop)
}

fn safe_label(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(|value| {
            value
                .chars()
                .filter(|character| !character.is_control())
                .take(64)
                .collect::<String>()
        })
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Unknown project".to_owned())
}

fn format_size(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{} KiB", bytes / 1024),
        _ => format!("{} MiB", bytes / 1_048_576),
    }
}

fn read_snapshot(state: &AppState) -> Option<TraySnapshot> {
    let daemon = state.optional_daemon().ok().flatten()?;
    let status = call_daemon_blocking(daemon.clone(), "status", serde_json::json!({})).ok()?;
    let pending =
        call_daemon_blocking(daemon.clone(), "list_pending", serde_json::json!({})).ok()?;
    let entries = pending.get("pending")?.as_array()?;
    let weekly = call_daemon_blocking(daemon, "history_rollup", serde_json::json!({}))
        .ok()
        .and_then(|rollup| {
            let week = rollup.get("week")?;
            Some(WeeklySummary {
                // submitted is the macOS menu's "contributed" count: it
                // records bytes sent while a final acceptance verdict may
                // still be pending.
                contributed: week
                    .get("submitted")
                    .and_then(Value::as_u64)
                    .and_then(|count| usize::try_from(count).ok())
                    .unwrap_or_default(),
                held: week
                    .get("quarantined")
                    .and_then(Value::as_u64)
                    .and_then(|count| usize::try_from(count).ok())
                    .unwrap_or_default(),
            })
        });
    let mut grouped = BTreeMap::<String, (usize, u64)>::new();
    for entry in entries {
        let label = safe_label(entry.get("project_label"));
        let size = entry
            .get("size_bytes")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let summary = grouped.entry(label).or_default();
        summary.0 += 1;
        summary.1 = summary.1.saturating_add(size);
    }
    let projects = grouped
        .into_iter()
        .map(|(label, (count, size_bytes))| ProjectSummary {
            label,
            count,
            size_bytes,
        })
        .collect();
    Some(TraySnapshot {
        decisions_owed: entries.len(),
        paused: status
            .get("paused")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        projects,
        private_inference_state: status
            .get("private_inference_state")
            .and_then(|value| value.get("state"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        weekly,
    })
}

fn tray_menu<R: TauriRuntime, M: Manager<R>>(
    manager: &M,
    snapshot: &TraySnapshot,
) -> tauri::Result<Menu<R>> {
    let queue_label = if snapshot.paused {
        format!("Watcher paused · {} waiting", snapshot.decisions_owed)
    } else if snapshot.decisions_owed == 0 {
        "Nothing waiting".to_owned()
    } else {
        format!("{} decisions waiting", snapshot.decisions_owed)
    };
    let mut builder = MenuBuilder::new(manager).text("review", queue_label);
    for (index, project) in snapshot.projects.iter().take(8).enumerate() {
        builder = builder.text(
            format!("project-{index}"),
            format!(
                "{} · {} · {}",
                project.label,
                project.count,
                format_size(project.size_bytes)
            ),
        );
    }
    if snapshot.projects.len() > 8 {
        builder = builder.text("project-more", "More waiting sessions in Trace Commons");
    };

    if let Some(state) = snapshot.private_inference_state.as_deref() {
        let (label, stop) = private_ai_tray_items(state);
        builder = builder.text("private-inference", label);
        if let Some(stop) = stop {
            builder = builder.text("private-inference-stop", stop);
        }
    }

    if let Some(weekly) = &snapshot.weekly {
        builder = builder.text(
            "weekly",
            format!(
                "This week · {} contributed · {} held for privacy review",
                weekly.contributed, weekly.held
            ),
        );
    }

    let pause_label = if snapshot.paused {
        "Resume watcher"
    } else {
        "Pause watcher"
    };
    let pause_menu = if snapshot.paused {
        SubmenuBuilder::new(manager, pause_label)
            .text("resume", "Resume now")
            .build()?
    } else {
        SubmenuBuilder::new(manager, pause_label)
            .text("pause-60", "For 1 hour")
            .text("pause-until-resume", "Until I turn it back on")
            .build()?
    };
    builder
        .separator()
        .item(&pause_menu)
        .separator()
        .text("open", "Open Trace Commons")
        .text("settings", "Settings")
        .separator()
        .text("quit", "Quit Trace Commons")
        .build()
}

fn show_main_window<R: TauriRuntime, M: Manager<R>>(manager: &M) {
    if let Some(window) = manager.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn open_route<R: TauriRuntime>(app: &AppHandle<R>, path: &str) {
    show_main_window(app);
    let _ = app.emit("navigate", path);
}

fn pause_until(minutes: u64) -> Option<String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs()
        .checked_add(minutes.checked_mul(60)?)?;
    let days = seconds / 86_400;
    let day_seconds = seconds % 86_400;
    let z = days as i64 + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month_part = (5 * doy + 2) / 153;
    let day = doy - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    let hour = day_seconds / 3_600;
    let minute = day_seconds % 3_600 / 60;
    let second = day_seconds % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

fn handle_menu_event<R: TauriRuntime>(app: &AppHandle<R>, id: &str) {
    match id {
        "review" | "open" | "project-more" => show_main_window(app),
        "settings" => open_route(app, "/settings"),
        "private-inference" => open_route(app, "/private-ai"),
        "private-inference-stop" => {
            if let Some(state) = app.try_state::<AppState>()
                && let Ok(Some(daemon)) = state.optional_daemon()
            {
                let _ = call_daemon_blocking(
                    daemon,
                    "set_settings",
                    serde_json::json!({
                        "private_inference": false,
                        "private_inference_offer_seen": true,
                    }),
                );
            }
            open_route(app, "/private-ai");
        }
        "resume" => {
            if let Some(state) = app.try_state::<AppState>()
                && let Ok(Some(daemon)) = state.optional_daemon()
            {
                let _ = call_daemon_blocking(daemon, "resume", serde_json::json!({}));
            }
        }
        "pause-until-resume" => {
            if let Some(state) = app.try_state::<AppState>()
                && let Ok(Some(daemon)) = state.optional_daemon()
            {
                let _ = call_daemon_blocking(daemon, "pause", serde_json::json!({}));
            }
        }
        id if id.starts_with("pause-") => {
            let minutes = id
                .strip_prefix("pause-")
                .and_then(|value| value.parse::<u64>().ok());
            let Some(minutes) = minutes else { return };
            let Some(until) = pause_until(minutes) else {
                return;
            };
            if let Some(state) = app.try_state::<AppState>()
                && let Ok(Some(daemon)) = state.optional_daemon()
            {
                let _ =
                    call_daemon_blocking(daemon, "pause", serde_json::json!({ "until": until }));
            }
        }
        "quit" => {
            let _ = app.emit("quit-requested", ());
        }
        _ => {}
    }
}

pub(crate) fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let menu = tray_menu(app, &TraySnapshot::default())?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".to_owned()))?;

    TrayIconBuilder::with_id("main")
        .menu(&menu)
        .icon(icon)
        .tooltip("Trace Commons")
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
        .build(app)?;
    Ok(())
}

pub(crate) fn start_tray_refresh<R: TauriRuntime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn_blocking(move || {
        let mut previous: Option<TraySnapshot> = None;
        while let Some(state) = app.try_state::<AppState>() {
            if state.event_stopped() {
                break;
            }
            let Some(snapshot) = read_snapshot(&state) else {
                std::thread::sleep(Duration::from_secs(1));
                continue;
            };
            if previous.as_ref() != Some(&snapshot) {
                if let (Some(tray), Ok(menu)) = (app.tray_by_id("main"), tray_menu(&app, &snapshot))
                {
                    let _ = tray.set_menu(Some(menu));
                    let tooltip = if snapshot.decisions_owed == 0 {
                        "Trace Commons · nothing waiting"
                    } else {
                        "Trace Commons · review waiting sessions"
                    };
                    let _ = tray.set_tooltip(Some(tooltip));
                }
                previous = Some(snapshot);
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

#[cfg(test)]
mod tests {
    use trace_commons_contributor::private_inference_copy::{DESTINATION, TRAY_TURN_OFF};

    use super::private_ai_tray_items;

    #[test]
    fn tray_names_private_ai_with_the_shared_destination_and_stop_action() {
        let (label, stop) = private_ai_tray_items("running_without_backends");
        assert_eq!(label, format!("{DESTINATION} · running without backends"));
        assert_eq!(stop, Some(TRAY_TURN_OFF));
        assert_eq!(private_ai_tray_items("running").1, Some(TRAY_TURN_OFF));

        let (label, stop) = private_ai_tray_items("off");
        assert_eq!(label, format!("{DESTINATION} · off"));
        assert_eq!(stop, None);
    }
}
