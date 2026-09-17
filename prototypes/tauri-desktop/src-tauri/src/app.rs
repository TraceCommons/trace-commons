use std::collections::BTreeSet;

use anyhow::Context;
use serde_json::Value;
use tauri::{Emitter, Manager};
use trace_commons_contributor::daemon::ipc::{EVENT_DIGEST_DUE, Event};

use crate::{
    commands,
    commands::platform,
    native, runtime,
    state::{AppState, DaemonConnection},
    tray,
};

fn sanitize_digest_label(value: &str) -> Option<String> {
    let label: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(64)
        .collect();
    (!label.trim().is_empty()).then_some(label)
}

fn labels_from_value(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|values| values.iter())
        .filter_map(Value::as_str)
        .filter_map(sanitize_digest_label)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn summarize_digest_labels(labels: &[String]) -> Option<String> {
    if labels.is_empty() {
        return None;
    }
    let named: Vec<&str> = labels.iter().take(3).map(String::as_str).collect();
    let more = labels.len().saturating_sub(named.len());
    if more > 0 {
        return Some(format!("{} and {more} more", named.join(", ")));
    }
    Some(match named.as_slice() {
        [one] => (*one).to_owned(),
        [first, second] => format!("{first} and {second}"),
        _ => format!(
            "{} and {}",
            named[..named.len() - 1].join(", "),
            named[named.len() - 1]
        ),
    })
}

fn digest_body(
    pending_count: usize,
    pending_projects: &[String],
    contributed_count: usize,
    contributed_projects: &[String],
    credit_pending: f64,
) -> Option<String> {
    if pending_count == 0 && contributed_count == 0 {
        return None;
    }
    let mut lines = Vec::new();
    if pending_count > 0 {
        let noun = if pending_count == 1 {
            "session"
        } else {
            "sessions"
        };
        let from = summarize_digest_labels(pending_projects)
            .map(|labels| format!(" from {labels}"))
            .unwrap_or_default();
        lines.push(format!("{pending_count} {noun} ready{from}."));
        lines.push("Nothing is sent until you review them.".to_owned());
    }
    if contributed_count > 0 {
        let noun = if contributed_count == 1 {
            "session"
        } else {
            "sessions"
        };
        let from = summarize_digest_labels(contributed_projects)
            .map(|labels| format!(" from {labels}"))
            .unwrap_or_default();
        let mut line = format!("{contributed_count} {noun} contributed{from}.");
        if credit_pending.is_finite() && credit_pending > 0.0 {
            let rounded = (credit_pending * 10.0).round() / 10.0;
            line.push_str(&format!(" {rounded:.1} credit pending."));
        }
        lines.push(line);
    }
    Some(lines.join("\n"))
}

fn pending_digest_labels(app: &tauri::AppHandle) -> Vec<String> {
    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return Vec::new();
    };
    let Ok(Some(daemon)) = state.optional_daemon() else {
        return Vec::new();
    };
    let Ok(value) = crate::ipc::call_daemon_blocking(daemon, "list_pending", serde_json::json!({}))
    else {
        return Vec::new();
    };
    value
        .get("pending")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|entries| entries.iter())
        .filter_map(|entry| entry.get("project_label"))
        .filter_map(Value::as_str)
        .filter_map(sanitize_digest_label)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn schedule_digest_notification(app: &tauri::AppHandle, event: Event) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(state) = app.try_state::<crate::state::AppState>() else {
            return;
        };
        if state.event_stopped() {
            return;
        }
        let pending_count = event
            .data
            .get("pending")
            .and_then(Value::as_u64)
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or_default();
        let contributed_count = event
            .data
            .get("contributed")
            .and_then(Value::as_u64)
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or_default();
        let pending_projects = if pending_count > 0 {
            pending_digest_labels(&app)
        } else {
            Vec::new()
        };
        let contributed_projects = labels_from_value(event.data.get("contributed_projects"));
        let credit_pending = event
            .data
            .get("credit_pending")
            .and_then(Value::as_f64)
            .unwrap_or_default();
        if let Some(body) = digest_body(
            pending_count,
            &pending_projects,
            contributed_count,
            &contributed_projects,
            credit_pending,
        ) {
            if state.event_stopped() {
                return;
            }
            let _ = crate::native::post_digest(&body);
        }
    });
}

fn handle_daemon_event(app: &tauri::AppHandle, event: Event) {
    // Event payloads can contain queue snapshots. The webview only needs an
    // invalidation signal; never forward daemon data, paths, or digest text
    // through the frontend event channel.
    let _ = app.emit("daemon-event", serde_json::json!({ "event": event.event }));
    if event.event != EVENT_DIGEST_DUE {
        return;
    }
    schedule_digest_notification(app, event);
}

fn start_event_bridge(app: tauri::AppHandle) {
    let state = app.state::<AppState>();
    state.reset_event_stop();
    let stop = state.event_stop();
    let daemon = state.optional_daemon().ok().flatten();
    match daemon {
        Some(DaemonConnection::Embedded(shared)) => {
            let _ = std::thread::Builder::new()
                .name("tc-embedded-events".to_owned())
                .spawn(move || {
                    let mut events = shared.events.subscribe();
                    loop {
                        if stop.load(std::sync::atomic::Ordering::Acquire) {
                            break;
                        }
                        if let Ok(event) = events.try_recv() {
                            handle_daemon_event(&app, event);
                        } else {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                    }
                });
        }
        #[cfg(unix)]
        Some(DaemonConnection::Attached(daemon)) => {
            let stop = std::sync::Arc::clone(&stop);
            tauri::async_runtime::spawn_blocking(move || {
                let _ = daemon.subscribe(move |event| {
                    if !stop.load(std::sync::atomic::Ordering::Acquire) {
                        handle_daemon_event(&app, event);
                    }
                });
            });
        }
        None => {}
    }
}

pub(crate) fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
            let state_dir = app
                .path()
                .app_data_dir()
                .context("resolving Trace Commons application data directory")?;
            let runtime = runtime::start_runtime(state_dir)?;
            let state = app.state::<AppState>();
            state
                .install(runtime)
                .map_err(|_| "application state lock poisoned")?;
            for argument in std::env::args().skip(1) {
                if platform::is_tracecommons_deep_link(&argument) {
                    state.set_pending_deep_link(argument);
                }
            }
            native::configure_notifications();
            tray::setup_tray(app)?;
            start_event_bridge(app.handle().clone());
            tray::start_tray_refresh(app.handle().clone());
            Ok(())
        })
        .invoke_handler(commands::handler())
        .build(tauri::generate_context!())
        .expect("error while building Tauri prototype")
        .run(|app_handle, event| match event {
            tauri::RunEvent::Opened { urls } => {
                for url in urls {
                    let value = url.as_str();
                    if platform::is_tracecommons_deep_link(value) {
                        app_handle
                            .state::<AppState>()
                            .set_pending_deep_link(value.to_owned());
                    }
                }
            }
            tauri::RunEvent::ExitRequested { api, code, .. } => {
                let state = app_handle.state::<AppState>();
                let authorized = code.is_some() && state.consume_authorized_exit();
                if !authorized {
                    api.prevent_exit();
                    let _ = app_handle.emit("quit-requested", ());
                }
            }
            tauri::RunEvent::Exit { .. } => {
                runtime::stop_runtime(&app_handle.state::<AppState>());
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::{digest_body, sanitize_digest_label, summarize_digest_labels};

    #[test]
    fn digest_copy_matches_review_safe_mac_surface() {
        let projects = vec!["alpha".to_owned(), "beta".to_owned()];
        let contributed = vec!["alpha".to_owned()];
        assert_eq!(
            digest_body(2, &projects, 1, &contributed, 4.25).as_deref(),
            Some(
                "2 sessions ready from alpha and beta.\nNothing is sent until you review them.\n1 session contributed from alpha. 4.3 credit pending."
            )
        );
    }

    #[test]
    fn digest_labels_are_bounded_and_controls_are_removed() {
        let labels = vec![
            "one".to_owned(),
            "two".to_owned(),
            "three".to_owned(),
            "four".to_owned(),
        ];
        assert_eq!(
            summarize_digest_labels(&labels).as_deref(),
            Some("one, two, three and 1 more")
        );
        assert_eq!(sanitize_digest_label("ok\npath"), Some("okpath".to_owned()));
        assert_eq!(sanitize_digest_label("  \t"), None);
    }
}
