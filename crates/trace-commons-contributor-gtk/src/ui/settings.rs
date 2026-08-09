//! Settings: pause, projects, permissions, and the local record of what was
//! armed.
//!
//! Everything here has a CLI equivalent, which on Linux is the point: a
//! capability reachable only through this window is a capability a headless
//! contributor does not have. Nothing in this view is the only way to do
//! anything.

use std::rc::Rc;

use adw::prelude::*;

use super::App;
use crate::copy;
use crate::model::{Project, Settings, Status};

pub struct SettingsView {
    pub root: gtk::Box,
    connection: gtk::Label,
    pause_button: gtk::Button,
    projects: gtk::Box,
    knobs: gtk::Box,
    audit: gtk::Box,
}

impl SettingsView {
    pub fn new() -> Self {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(16)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(16)
            .margin_end(16)
            .build();

        let connection = gtk::Label::builder().xalign(0.0).wrap(true).build();
        content.append(&connection);

        let pause_button = gtk::Button::with_label("Pause");
        pause_button.set_halign(gtk::Align::Start);
        content.append(&pause_button);

        let projects_heading = gtk::Label::builder().label("Projects").xalign(0.0).build();
        projects_heading.add_css_class("title-4");
        content.append(&projects_heading);
        let projects = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&projects);

        let knobs_heading = gtk::Label::builder()
            .label("How it behaves")
            .xalign(0.0)
            .build();
        knobs_heading.add_css_class("title-4");
        content.append(&knobs_heading);
        let knobs = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&knobs);

        let audit_heading = gtk::Label::builder()
            .label("What has been changed on this machine")
            .xalign(0.0)
            .build();
        audit_heading.add_css_class("title-4");
        content.append(&audit_heading);
        let audit = gtk::Box::new(gtk::Orientation::Vertical, 4);
        content.append(&audit);

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&content)
            .build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&scroller);

        Self {
            root,
            connection,
            pause_button,
            projects,
            knobs,
            audit,
        }
    }
}

pub fn wire(app: &Rc<App>) {
    let a = Rc::clone(app);
    app.settings.pause_button.connect_clicked(move |_| {
        let paused = a
            .status
            .borrow()
            .as_ref()
            .map(|s| s.paused)
            .unwrap_or(false);
        if paused {
            a.call("resume", serde_json::json!({}), |app, _| app.refresh());
        } else {
            offer_pause(&a);
        }
    });
}

/// Pause offers the three durations from the shared spec, backed by
/// `pause {until}` so a timed pause survives this window quitting -- which
/// on Linux it routinely does, since the watcher is usually another
/// process.
fn offer_pause(app: &Rc<App>) {
    let dialog = adw::MessageDialog::new(
        Some(&app.window),
        Some("Pause contributing?"),
        Some("Nothing is queued or sent while paused. Anything already waiting stays waiting."),
    );
    dialog.add_responses(&[
        ("cancel", "Cancel"),
        ("hour", "For 1 hour"),
        ("tomorrow", "Until tomorrow morning"),
        ("forever", "Until I turn it back on"),
    ]);
    dialog.set_close_response("cancel");
    let app = Rc::clone(app);
    dialog.connect_response(None, move |dialog, response| {
        dialog.close();
        let params = match response {
            "hour" => serde_json::json!({
                "until": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339()
            }),
            "tomorrow" => serde_json::json!({ "until": tomorrow_morning().to_rfc3339() }),
            "forever" => serde_json::json!({}),
            _ => return,
        };
        app.call("pause", params, |app, _| app.refresh());
    });
    dialog.present();
}

/// 9am local time on the next day, expressed as an instant. The daemon
/// rejects a timestamp already in the past, so this is always at least a
/// few hours out.
fn tomorrow_morning() -> chrono::DateTime<chrono::Utc> {
    let local = chrono::Local::now();
    let tomorrow = local.date_naive().succ_opt().unwrap_or(local.date_naive());
    tomorrow
        .and_hms_opt(9, 0, 0)
        .map(|naive| naive.and_utc())
        .unwrap_or_else(|| chrono::Utc::now() + chrono::Duration::hours(12))
}

pub fn render_status(app: &Rc<App>, status: &Status) {
    let hosting = app.worker.hosts_the_loop();
    let connection = if status.paused {
        "Paused. Nothing is being queued or sent."
    } else if hosting {
        "This window is doing the watching. Closing it stops that."
    } else {
        "A background watcher is running separately. It keeps going when this window closes."
    };
    let connected = if status.logged_in {
        "Connected to Trace Commons."
    } else {
        "Not connected. Sessions are still being queued; nothing can be sent yet, and nothing \
         has been lost."
    };
    app.settings
        .connection
        .set_text(&format!("{connection}\n{connected}"));
    app.settings
        .pause_button
        .set_label(if status.paused { "Resume" } else { "Pause" });
}

pub fn refresh(app: &Rc<App>) {
    app.call("list_projects", serde_json::json!({}), |app, result| {
        let projects: Vec<Project> = result
            .ok()
            .and_then(|v| serde_json::from_value(v.get("projects").cloned()?).ok())
            .unwrap_or_default();
        render_projects(app, &projects);
    });
    app.call("get_settings", serde_json::json!({}), |app, result| {
        let Ok(value) = result else { return };
        let Ok(settings) = serde_json::from_value::<Settings>(value) else {
            return;
        };
        render_knobs(app, &settings);
    });
    app.call(
        "list_audit",
        serde_json::json!({ "limit": 20 }),
        |app, result| {
            let entries = result
                .ok()
                .and_then(|v| v.get("entries").cloned())
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            let view = &app.settings.audit;
            while let Some(child) = view.first_child() {
                view.remove(&child);
            }
            if entries.is_empty() {
                let empty = gtk::Label::builder()
                    .label("Nothing has been changed.")
                    .xalign(0.0)
                    .build();
                empty.add_css_class("dim-label");
                view.append(&empty);
            }
            for entry in entries {
                // `action`, `project_label` and `detail` are fixed labels by
                // contract, so this line can carry no path or token.
                let action = entry.get("action").and_then(|v| v.as_str()).unwrap_or("");
                let project = entry
                    .get("project_label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let at = entry.get("at").and_then(|v| v.as_str()).unwrap_or("");
                let line = gtk::Label::builder()
                    .label(format!("{at}  {}  {project}", audit_sentence(action)))
                    .xalign(0.0)
                    .wrap(true)
                    .build();
                line.add_css_class("dim-label");
                view.append(&line);
            }
        },
    );
}

fn audit_sentence(action: &str) -> &'static str {
    match action {
        "armed-auto-upload" => "Automatic contributing turned on for",
        "disarmed-auto-upload" => "Automatic contributing turned off for",
        "queue-bulk-approved" => "The whole queue was approved",
        "consent-scopes-changed" => "Permissions changed",
        "near-ai-notice-acknowledged" => "The extra privacy scan was confirmed",
        _ => "Changed",
    }
}

/// Projects, named by `project_id` on the wire and by `project_label` on
/// screen. A path never appears in either direction.
fn render_projects(app: &Rc<App>, projects: &[Project]) {
    let view = &app.settings.projects;
    while let Some(child) = view.first_child() {
        view.remove(&child);
    }

    // Armed projects are listed first and never collapsed away, so a
    // contributor always knows what is contributing without being asked.
    let armed: Vec<&Project> = projects
        .iter()
        .filter(|p| p.mode == "auto_upload")
        .collect();
    let armed_line = gtk::Label::builder()
        .label(if armed.is_empty() {
            "Armed: nothing. Every session is offered to you first.".to_string()
        } else {
            format!(
                "Armed: {} - {}",
                armed.len(),
                armed
                    .iter()
                    .map(|p| p.project_label.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .xalign(0.0)
        .wrap(true)
        .build();
    armed_line.add_css_class("heading");
    view.append(&armed_line);

    for project in projects {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::builder()
            .label(&project.project_label)
            .xalign(0.0)
            .hexpand(true)
            .build();
        row.append(&label);

        let modes = gtk::DropDown::from_strings(&[
            "Ask me first",
            "Contribute automatically",
            "Never offer this one",
        ]);
        modes.set_selected(match project.mode.as_str() {
            "auto_upload" => 1,
            "ignore" => 2,
            _ => 0,
        });
        row.append(&modes);
        view.append(&row);

        let app = Rc::clone(app);
        let project = project.clone();
        modes.connect_selected_notify(move |dropdown| {
            let wanted = match dropdown.selected() {
                1 => "auto_upload",
                2 => "ignore",
                _ => "notify_only",
            };
            if wanted == project.mode {
                return;
            }
            if wanted == "auto_upload" {
                confirm_arming(&app, &project, dropdown.clone());
            } else {
                set_mode(&app, &project.project_id, wanted);
            }
        });
    }
}

/// Arming is allowed from the app, but never silently.
fn confirm_arming(app: &Rc<App>, project: &Project, dropdown: gtk::DropDown) {
    let dialog = adw::MessageDialog::new(
        Some(&app.window),
        Some(&copy::arming_heading(&project.project_label)),
        Some(copy::ARMING_BODY),
    );
    dialog.add_responses(&[
        ("cancel", copy::ARMING_CANCEL),
        ("arm", copy::ARMING_CONFIRM),
    ]);
    dialog.set_response_appearance("arm", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");

    let app = Rc::clone(app);
    let project_id = project.project_id.clone();
    let previous = match project.mode.as_str() {
        "ignore" => 2u32,
        _ => 0u32,
    };
    dialog.connect_response(None, move |dialog, response| {
        dialog.close();
        if response == "arm" {
            set_mode(&app, &project_id, "auto_upload");
        } else {
            dropdown.set_selected(previous);
        }
    });
    dialog.present();
}

fn set_mode(app: &Rc<App>, project_id: &str, mode: &str) {
    app.call(
        "set_project_mode",
        serde_json::json!({ "project_id": project_id, "mode": mode }),
        |app, result| {
            if result.is_err() {
                app.toast("That couldn't be changed just now. Nothing else changed either.");
            }
            app.refresh();
        },
    );
}

fn render_knobs(app: &Rc<App>, settings: &Settings) {
    let view = &app.settings.knobs;
    while let Some(child) = view.first_child() {
        view.remove(&child);
    }

    view.append(&super::titled_paragraph(
        "Quiet time before a session counts as finished",
        &format!("{} minutes", settings.quiescence_secs / 60),
    ));
    view.append(&super::titled_paragraph(
        "How long you can take something back",
        &if settings.approval_hold_secs == 0 {
            "No undo window. Approving sends on the next pass.".to_string()
        } else {
            format!("{} seconds after you approve", settings.approval_hold_secs)
        },
    ));
    view.append(&super::titled_paragraph(
        "How often you can be interrupted",
        &format!(
            "At most once every {} hours",
            settings.digest_interval_secs / 3600
        ),
    ));

    // Configured-or-not facts only. The credential and the two local paths
    // never cross the socket, so there is nothing here that could render
    // one by accident.
    view.append(&super::titled_paragraph(
        "Extra privacy scan",
        if settings.near_ai_configured {
            "Set up. Message text is scanned by a third party before anything is sent."
        } else {
            "Not set up. Local scrubbing only."
        },
    ));
    view.append(&super::titled_paragraph(
        "Where sessions are read from",
        &format!(
            "Claude Code: {}   Codex: {}",
            if settings.claude_root_configured {
                "a folder you chose"
            } else {
                "the usual place"
            },
            if settings.codex_root_configured {
                "a folder you chose"
            } else {
                "the usual place"
            }
        ),
    ));
}
