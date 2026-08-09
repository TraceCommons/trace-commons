//! Settings: pause, projects, permissions, and the local record of what was
//! armed.
//!
//! Everything here has a CLI equivalent, which on Linux is the point: a
//! capability reachable only through this window is a capability a headless
//! contributor does not have. Nothing in this view is the only way to do
//! anything.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use super::App;
use super::style::{self, Tone, space};
use crate::copy;
use crate::model::{Project, Settings, Status};

pub struct SettingsView {
    pub root: gtk::Box,
    connection: gtk::Label,
    pause_button: gtk::Button,
    projects: gtk::Box,
    knobs: gtk::Box,
    autostart_body: gtk::Label,
    autostart_row: gtk::Box,
    autostart_switch: gtk::Switch,
    /// The background-app-registration row, filled in once
    /// `portal::spawn_request`'s classification lands -- see
    /// `render_background`. `None` until then, which is why it starts on
    /// `copy::PORTAL_STATUS_CHECKING` rather than a guess.
    background_state: RefCell<Option<crate::portal::BackendState>>,
    background_body: gtk::Label,
    audit: gtk::Box,
}

impl Default for SettingsView {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsView {
    pub fn new() -> Self {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(space::L)
            .margin_top(space::XL)
            .margin_bottom(space::XL)
            .margin_start(space::L)
            .margin_end(space::L)
            .build();

        // What is running, and the one control that changes it, in one
        // card. These two facts belong together: reading "a background
        // watcher is running" and then hunting for Pause somewhere else is
        // the state and its control being separated for no reason.
        let state_card = style::card(gtk::Orientation::Vertical, space::M);
        let connection = gtk::Label::builder().xalign(0.0).wrap(true).build();
        connection.add_css_class("tc-body");
        state_card.append(&connection);
        let pause_button = gtk::Button::with_label("Pause");
        pause_button.add_css_class("tc-quiet");
        pause_button.set_halign(gtk::Align::Start);
        state_card.append(&pause_button);
        content.append(&state_card);

        content.append(&style::section("Projects"));
        let projects = style::card(gtk::Orientation::Vertical, space::M);
        content.append(&projects);

        content.append(&style::section("How it behaves"));
        let knobs = style::card(gtk::Orientation::Vertical, space::M);
        content.append(&knobs);

        content.append(&style::section(copy::AUTOSTART_HEADING));
        let autostart_card = style::card(gtk::Orientation::Vertical, space::M);
        let autostart_body = gtk::Label::builder().xalign(0.0).wrap(true).build();
        autostart_body.add_css_class("tc-body");
        autostart_card.append(&autostart_body);
        let autostart_row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        let autostart_switch_label = gtk::Label::builder()
            .label(copy::AUTOSTART_XDG_LABEL)
            .xalign(0.0)
            .hexpand(true)
            .build();
        autostart_switch_label.add_css_class("tc-body");
        let autostart_switch = gtk::Switch::builder().halign(gtk::Align::End).build();
        // The switch is reachable by keyboard; the label beside it is not a
        // control, so it is pointed at the switch for a screen reader
        // rather than left as loose text.
        autostart_switch
            .update_property(&[gtk::accessible::Property::Label(copy::AUTOSTART_XDG_LABEL)]);
        autostart_row.append(&autostart_switch_label);
        autostart_row.append(&autostart_switch);
        autostart_card.append(&autostart_row);
        // Same card, not a new section: whether this desktop can list
        // Trace Commons as a background app is the same topic as whether
        // it starts automatically, and the two facts read better together
        // than split across headings.
        let background_body = gtk::Label::builder().xalign(0.0).wrap(true).build();
        background_body.add_css_class("tc-meta");
        background_body.set_text(copy::PORTAL_STATUS_CHECKING);
        autostart_card.append(&background_body);
        content.append(&autostart_card);

        content.append(&style::section("What has been changed on this machine"));
        let audit = style::card(gtk::Orientation::Vertical, space::XS);
        content.append(&audit);

        let clamp = adw::Clamp::builder()
            .maximum_size(840)
            .tightening_threshold(680)
            .child(&content)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("tc-root");
        root.append(&scroller);

        Self {
            root,
            connection,
            pause_button,
            projects,
            knobs,
            autostart_body,
            autostart_row,
            autostart_switch,
            background_state: RefCell::new(None),
            background_body,
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

    render_autostart(app);
    let a = Rc::clone(app);
    app.settings
        .autostart_switch
        .connect_state_set(move |_, wanted| {
            // Only reachable when detection already put the switch in play --
            // see `render_autostart`, which hides this row entirely rather
            // than disabling it when a systemd unit is doing the job. So an
            // event here always means "write or remove the XDG entry",
            // never "fight the systemd unit for control".
            let result = if wanted {
                crate::autostart::enable_xdg_entry()
            } else {
                crate::autostart::disable_xdg_entry()
            };
            if result.is_err() {
                a.toast("That couldn't be changed just now. Nothing else changed either.");
            }
            render_autostart(&a);
            gtk::glib::Propagation::Proceed
        });
}

/// Drain the background-portal probe's answer, once, and render it. The
/// probe is spawned once at app startup (see `App::build`), so this is
/// wired once here too rather than re-run on every settings refresh.
pub fn wire_background_probe(
    app: &Rc<App>,
    rx: async_channel::Receiver<crate::portal::BackendState>,
) {
    render_background(app);
    let a = Rc::clone(app);
    gtk::glib::spawn_future_local(async move {
        if let Ok(state) = rx.recv().await {
            *a.settings.background_state.borrow_mut() = Some(state);
            render_background(&a);
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

/// Which of the two autostart mechanisms is in force, and say so plainly.
/// Local filesystem state, not the daemon's, so this needs no round trip
/// and can be called eagerly at wire time.
fn render_autostart(app: &Rc<App>) {
    match crate::autostart::detect() {
        crate::autostart::Mechanism::SystemdUnit => {
            app.settings
                .autostart_body
                .set_text(copy::AUTOSTART_SYSTEMD_BODY);
            app.settings.autostart_row.set_visible(false);
        }
        crate::autostart::Mechanism::XdgEntry { enabled } => {
            app.settings
                .autostart_body
                .set_text(copy::AUTOSTART_XDG_BODY);
            app.settings.autostart_row.set_visible(true);
            // Programmatic sync from disk, not a click. GTK's
            // `state-set` signal fires only on user interaction with the
            // switch, not from `set_active`, so this cannot loop back
            // into the handler above and re-write the file it just read.
            app.settings.autostart_switch.set_active(enabled);
        }
    }
}

/// The background-app-registration row. Reads live filesystem detection for
/// the systemd signal (cheap, and the source of truth `render_autostart`
/// already uses) and the stored probe answer for the portal signal, so the
/// two are never rendered from stale copies of each other.
fn render_background(app: &Rc<App>) {
    let systemd_unit_installed = matches!(
        crate::autostart::detect(),
        crate::autostart::Mechanism::SystemdUnit
    );
    let text = match *app.settings.background_state.borrow() {
        Some(state) => copy::portal_status_line(state, systemd_unit_installed),
        None => copy::PORTAL_STATUS_CHECKING,
    };
    app.settings.background_body.set_text(text);
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
                empty.add_css_class("tc-meta");
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
                // The instant is a figure, so it is set as one, and the
                // column of them lines up down the card.
                let line = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
                let when = gtk::Label::builder().label(at).xalign(0.0).build();
                when.add_css_class("tc-ledger");
                when.add_css_class("tc-neutral");
                line.append(&when);
                let what = gtk::Label::builder()
                    .label(format!("{}  {project}", audit_sentence(action)))
                    .xalign(0.0)
                    .wrap(true)
                    .build();
                what.add_css_class("tc-meta");
                line.append(&what);
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
    // Armed means "contributes without asking", which is the strongest
    // thing this window can be set to do. It gets the attention tone when
    // anything is armed and the clear tone when nothing is, plus a glyph
    // and words, because it is the state a person most needs to be able to
    // check at a glance.
    let armed_summary = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    armed_summary.append(&if armed.is_empty() {
        style::tag("Nothing is armed", Tone::Clear)
    } else {
        style::tag(&format!("{} armed", armed.len()), Tone::Attention)
    });
    let armed_line = gtk::Label::builder()
        .label(if armed.is_empty() {
            "Every session is offered to you first.".to_string()
        } else {
            armed
                .iter()
                .map(|p| p.project_label.clone())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .xalign(0.0)
        .wrap(true)
        .build();
    armed_line.add_css_class("tc-body");
    armed_summary.append(&armed_line);
    view.append(&armed_summary);

    for project in projects {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        let label = gtk::Label::builder()
            .label(&project.project_label)
            .xalign(0.0)
            .hexpand(true)
            .build();
        label.add_css_class("tc-body");
        row.append(&label);

        let modes = gtk::DropDown::from_strings(&[
            "Ask me first",
            "Contribute automatically",
            "Never offer this one",
        ]);
        // The project name sits in a separate label, so the control has to
        // say what it controls on its own.
        modes.update_property(&[gtk::accessible::Property::Label(&format!(
            "How to treat {}",
            project.project_label
        ))]);
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
