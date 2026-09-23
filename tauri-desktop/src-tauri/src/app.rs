use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;
use tauri::{Emitter, Manager, Runtime};
use trace_commons_contributor::daemon::ipc::{
    EVENT_DIGEST_DUE, EVENT_PREVIEW_READY, EVENT_QUEUE_CHANGED, EVENT_RESYNC_REQUIRED,
    EVENT_SNAPSHOT, EVENT_STATUS_CHANGED, Event,
};

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

fn pending_digest_labels<R: Runtime>(app: &tauri::AppHandle<R>) -> Vec<String> {
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

fn schedule_digest_notification<R: Runtime>(app: &tauri::AppHandle<R>, event: Event) {
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

fn handle_daemon_event<R: Runtime>(app: &tauri::AppHandle<R>, event: Event) {
    // Event payloads can contain queue snapshots. The webview receives only
    // allowlisted invalidation names; never forward daemon data, paths, or
    // digest text through the frontend event channel.
    if matches!(
        event.event.as_str(),
        EVENT_SNAPSHOT
            | EVENT_QUEUE_CHANGED
            | EVENT_STATUS_CHANGED
            | EVENT_RESYNC_REQUIRED
            | EVENT_PREVIEW_READY
    ) {
        let _ = app.emit("daemon-event", serde_json::json!({ "event": event.event }));
    }
    if event.event != EVENT_DIGEST_DUE {
        return;
    }
    schedule_digest_notification(app, event);
}

fn wait_for_event_retry(stop: &AtomicBool, delay: Duration) -> bool {
    for _ in 0..delay.as_secs() {
        if stop.load(Ordering::Acquire) {
            return false;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    !stop.load(Ordering::Acquire)
}

fn start_daemon_recovery<R: Runtime>(app: tauri::AppHandle<R>) {
    tauri::async_runtime::spawn_blocking(move || {
        let stop = app.state::<AppState>().event_stop();
        let mut delay = Duration::from_secs(1);
        loop {
            if !wait_for_event_retry(&stop, delay) {
                return;
            }
            let state = app.state::<AppState>();
            if tauri::async_runtime::block_on(runtime::ensure_daemon_started(&state)).is_ok() {
                start_event_bridge(app.clone());
                return;
            }
            delay = delay.saturating_mul(2).min(Duration::from_secs(30));
        }
    });
}

fn supervise_attached_events<R: Runtime>(
    app: tauri::AppHandle<R>,
    daemon: Arc<trace_commons_contributor::daemon::attached::AttachedDaemon>,
    stop: Arc<AtomicBool>,
) {
    tauri::async_runtime::spawn_blocking(move || {
        let mut retry_delay = Duration::from_secs(1);
        loop {
            if stop.load(Ordering::Acquire) {
                break;
            }

            let event_app = app.clone();
            let event_stop = Arc::clone(&stop);
            match daemon.subscribe(move |event| {
                if !event_stop.load(Ordering::Acquire) {
                    handle_daemon_event(&event_app, event);
                }
            }) {
                Ok(response) if response.error.is_none() => {
                    while !stop.load(Ordering::Acquire) && !daemon.is_closed() {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                Ok(_) if !daemon.is_closed() => {
                    daemon.clear_sink();
                    if !wait_for_event_retry(&stop, retry_delay) {
                        break;
                    }
                    continue;
                }
                _ => daemon.close(),
            }

            if stop.load(Ordering::Acquire) {
                break;
            }

            loop {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let state = app.state::<AppState>();
                if tauri::async_runtime::block_on(runtime::ensure_daemon_started(&state)).is_ok() {
                    state.release_event_bridge();
                    start_event_bridge(app.clone());
                    return;
                }
                if !wait_for_event_retry(&stop, retry_delay) {
                    break;
                }
                retry_delay = retry_delay.saturating_mul(2).min(Duration::from_secs(30));
            }
            break;
        }
        app.state::<AppState>().release_event_bridge();
    });
}

pub(crate) fn start_event_bridge<R: Runtime>(app: tauri::AppHandle<R>) {
    let state = app.state::<AppState>();
    let daemon = state.optional_daemon().ok().flatten();
    let Some(daemon) = daemon else {
        return;
    };
    if !state.claim_event_bridge() {
        return;
    }
    let stop = state.event_stop();
    let event_app = app.clone();
    let started = match daemon {
        DaemonConnection::Embedded(shared) => {
            tauri::async_runtime::spawn(async move {
                let mut events = shared.events.subscribe();
                loop {
                    if stop.load(std::sync::atomic::Ordering::Acquire) {
                        break;
                    }
                    match events.recv().await {
                        Ok(event) => handle_daemon_event(&event_app, event),
                        Err(_) if events.is_closed() => break,
                        Err(_) => {
                            let _ = event_app.emit(
                                "daemon-event",
                                serde_json::json!({ "event": EVENT_RESYNC_REQUIRED }),
                            );
                        }
                    }
                }
                event_app.state::<AppState>().release_event_bridge();
            });
            true
        }
        DaemonConnection::Attached(daemon) => {
            supervise_attached_events(app.clone(), daemon, stop);
            true
        }
    };
    if !started {
        state.release_event_bridge();
    }
}

fn remember_deep_link(app: &tauri::AppHandle, value: &str) {
    if platform::is_tracecommons_deep_link(value) {
        app.state::<AppState>()
            .set_pending_deep_link(value.to_owned());
        let _ = app.emit("deep-link-received", ());
    }
}

pub(crate) fn run() {
    let builder = tauri::Builder::default().manage(AppState::default());

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    let builder = builder
        // Windows and Linux deliver protocol activations to a new process.
        // Register this before deep-link handling so the first process owns
        // all subsequent activations.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            for argument in argv {
                remember_deep_link(app, &argument);
            }
        }))
        .plugin(tauri_plugin_deep_link::init());

    #[cfg(target_os = "macos")]
    let builder = builder
        .menu(|app| {
            use tauri::menu::{Menu, MenuItem, MenuItemKind};

            let menu = Menu::default(app)?;
            if let Some(MenuItemKind::Submenu(app_menu)) = menu.items()?.into_iter().next() {
                let items = app_menu.items()?;
                if !items.is_empty() {
                    app_menu.remove_at(items.len() - 1)?;
                }
                app_menu.append(&MenuItem::with_id(
                    app,
                    "request_quit",
                    format!("Quit {}", app.package_info().name),
                    true,
                    Some("CmdOrCtrl+Q"),
                )?)?;
            }
            Ok(menu)
        })
        .on_menu_event(|app, event| {
            if event.id() == "request_quit" {
                let _ = app.emit("quit-requested", ());
            }
        });

    builder
        .setup(|app| {
            let runtime = runtime::start_runtime()?;
            let recover_daemon = runtime.daemon_recovery_needed();
            let state = app.state::<AppState>();
            state
                .install(runtime)
                .map_err(|_| "application state lock poisoned")?;
            if let Some(window) = app.get_webview_window("main") {
                let app_handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = app_handle.emit("quit-requested", ());
                    }
                });
            }
            for argument in std::env::args().skip(1) {
                remember_deep_link(app.handle(), &argument);
            }
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;

                // Runtime registration makes debug and unpackaged builds
                // usable. Packaged macOS registration comes from the bundle
                // configuration above.
                let state = if app.deep_link().register_all().is_ok() {
                    "configured"
                } else {
                    "unavailable"
                };
                app.state::<AppState>().set_deep_link_state(state);
            }
            #[cfg(target_os = "macos")]
            app.state::<AppState>()
                .set_deep_link_state(if app.config().bundle.active {
                    "configured"
                } else {
                    "unknown"
                });
            #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
            app.state::<AppState>().set_deep_link_state("unavailable");
            native::configure_notifications();
            tray::setup_tray(app)?;
            start_event_bridge(app.handle().clone());
            if recover_daemon {
                start_daemon_recovery(app.handle().clone());
            }
            tray::start_tray_refresh(app.handle().clone());
            Ok(())
        })
        .invoke_handler(commands::handler())
        .build(tauri::generate_context!())
        .expect("failed to build Tauri desktop app")
        .run(|app_handle, event| match event {
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Opened { urls } => {
                for url in urls {
                    remember_deep_link(app_handle, url.as_str());
                }
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            tauri::RunEvent::Exit => {
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
