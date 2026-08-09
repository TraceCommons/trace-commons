//! The window, which on Linux is the primary surface.
//!
//! GNOME has no system tray, so nothing here may depend on one: every
//! capability is reachable from this window, and the tray -- where a desktop
//! has a real one -- would only ever be a shortcut into it. Nothing in this
//! application tells a contributor to install a shell extension.

pub mod history;
pub mod preview;
pub mod queue;
pub mod settings;
pub mod style;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::copy;
use crate::model::{PreviewSummary, QueueEntry, Status};
use crate::worker::{Outcome, Worker};

pub const APP_ID: &str = "ai.tracecommons.Contributor";

type Callback = Box<dyn FnOnce(&Rc<App>, Outcome)>;

pub struct App {
    pub worker: Worker,
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    pub stack: adw::ViewStack,

    pub queue: queue::QueueView,
    pub history: history::HistoryView,
    pub settings: settings::SettingsView,

    /// Health is rendered from `status.health.last_error_label` and nothing
    /// else: the daemon owns the precedence order and a client that
    /// reconstructed it would eventually disagree with the daemon about
    /// what is wrong.
    health_banner: gtk::Box,
    health_label: gtk::Label,
    health_button: gtk::Button,

    callbacks: RefCell<HashMap<u64, Callback>>,
    pub entries: RefCell<Vec<QueueEntry>>,
    pub status: RefCell<Option<Status>>,
    /// Preview summaries, keyed by entry id, so a row can show what would be
    /// sent without re-running the pipeline on every redraw.
    pub previews: RefCell<HashMap<String, PreviewSummary>>,
    /// Kept for the session rather than written to disk. The point of
    /// persisting them is that the second trace is one keystroke, and a
    /// search term is the contributor's own sensitive string -- a client
    /// name, usually. It does not need to outlive the process to do its job.
    pub recent_searches: RefCell<Vec<String>>,
    /// Guards against stacking a second preview request for a row that is
    /// already being previewed.
    prefetching: RefCell<std::collections::HashSet<String>>,
    quit_confirmed: Cell<bool>,
}

/// How many queue rows get their "would send / scrubbed" line filled in
/// automatically.
///
/// The shared spec puts that line on the row, but the only way to compute it
/// is a full preview -- which redacts the session and, under an external
/// scanner, makes a network call. Previewing 500 queued sessions to draw a
/// list would be absurd, so the first screenful is prefetched and the rest
/// fill in when opened. See the report for the contract note.
const PREVIEW_PREFETCH_LIMIT: usize = 12;

impl App {
    pub fn build(application: &adw::Application, worker: Worker) -> Rc<Self> {
        // Before any widget is built, so nothing is ever drawn in the
        // theme's palette and then repainted in this one.
        style::install();

        let window = adw::ApplicationWindow::builder()
            .application(application)
            .title(copy::APP_NAME)
            .default_width(980)
            .default_height(720)
            .build();

        let stack = adw::ViewStack::new();
        let queue = queue::QueueView::new();
        let history = history::HistoryView::new();
        let settings = settings::SettingsView::new();

        stack
            .add_titled(&queue.root, Some("queue"), "Queue")
            .set_icon_name(Some("view-list-symbolic"));
        stack
            .add_titled(&history.root, Some("history"), "History")
            .set_icon_name(Some("document-open-recent-symbolic"));
        stack
            .add_titled(&settings.root, Some("settings"), "Settings")
            .set_icon_name(Some("emblem-system-symbolic"));

        let switcher = adw::ViewSwitcherTitle::builder()
            .stack(&stack)
            .title(copy::APP_NAME)
            .build();
        let header = adw::HeaderBar::builder().title_widget(&switcher).build();
        header.add_css_class("tc-header");
        // The mark, drawn from its own geometry rather than shipped as an
        // asset. See `style::brand_mark`.
        header.pack_start(&style::brand_mark());

        let health_label = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .build();
        health_label.add_css_class("tc-body");
        let health_button = gtk::Button::builder().visible(false).build();
        health_button.add_css_class("tc-quiet");
        health_button.set_valign(gtk::Align::Center);
        // The glyph is what carries "weigh this" into greyscale; the gold
        // rule around the banner is the colour half of the same statement.
        let health_glyph = gtk::Label::new(Some(style::Tone::Attention.glyph()));
        health_glyph.add_css_class("tc-attention");
        health_glyph.add_css_class("tc-card-title");
        health_glyph.set_valign(gtk::Align::Start);
        let health_banner = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(style::space::M)
            .visible(false)
            .margin_top(style::space::M)
            .margin_start(style::space::L)
            .margin_end(style::space::L)
            .build();
        health_banner.append(&health_glyph);
        health_banner.append(&health_label);
        health_banner.append(&health_button);
        health_banner.add_css_class("tc-banner");

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("tc-root");
        content.append(&header);
        content.append(&health_banner);
        content.append(&stack);
        stack.set_vexpand(true);

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&content));
        window.set_content(Some(&toasts));

        let app = Rc::new(Self {
            worker,
            window,
            toasts,
            stack,
            queue,
            history,
            settings,
            health_banner,
            health_label,
            health_button,
            callbacks: RefCell::new(HashMap::new()),
            entries: RefCell::new(Vec::new()),
            status: RefCell::new(None),
            previews: RefCell::new(HashMap::new()),
            recent_searches: RefCell::new(Vec::new()),
            prefetching: RefCell::new(Default::default()),
            quit_confirmed: Cell::new(false),
        });

        app.wire_result_pump();
        app.wire_event_pump();
        app.wire_quit();
        app.wire_tray();
        queue::wire(&app);
        history::wire(&app);
        settings::wire(&app);
        app.refresh();

        // Best-effort and platform-optional, in the order the design spec
        // gives them: the portal registration is the one that matters most
        // (it is where a GNOME user looks for this app at all), the tray
        // is the bonus. Neither can keep the window from opening -- both
        // run on their own threads and report nothing back that would
        // block or fail startup.
        crate::portal::spawn_request();

        app
    }

    /// Drain worker results on the main loop and hand each to the closure
    /// that asked for it.
    fn wire_result_pump(self: &Rc<Self>) {
        let app = Rc::clone(self);
        glib::spawn_future_local(async move {
            while let Ok((id, outcome)) = app.worker.results.recv().await {
                let callback = app.callbacks.borrow_mut().remove(&id);
                if let Some(callback) = callback {
                    callback(&app, outcome);
                }
            }
        });
    }

    /// Daemon events are treated as "something moved, look again" rather
    /// than as deltas to apply. That is also the only correct response to
    /// `resync_required`, so there is one code path instead of two.
    fn wire_event_pump(self: &Rc<Self>) {
        let app = Rc::clone(self);
        glib::spawn_future_local(async move {
            while let Ok(event) = app.worker.events.recv().await {
                match event.as_str() {
                    "digest_due" => {
                        app.refresh();
                        app.post_digest();
                    }
                    _ => app.refresh(),
                }
            }
        });
    }

    /// Quitting must say what continues, and the true sentence depends on
    /// which process is doing the watching.
    fn wire_quit(self: &Rc<Self>) {
        let app = Rc::clone(self);
        self.window.connect_close_request(move |window| {
            if app.quit_confirmed.get() {
                return glib::Propagation::Proceed;
            }
            let dialog = if app.worker.hosts_the_loop() {
                let d = adw::MessageDialog::new(
                    Some(window),
                    Some("Quit Trace Commons?"),
                    Some(copy::QUIT_HOSTING_BODY),
                );
                d.add_responses(&[
                    ("cancel", copy::QUIT_HOSTING_CANCEL),
                    ("quit", copy::QUIT_HOSTING_CONFIRM),
                ]);
                d
            } else {
                let d = adw::MessageDialog::new(
                    Some(window),
                    Some("Quit Trace Commons?"),
                    Some(copy::QUIT_ATTACHED_BODY),
                );
                d.add_responses(&[
                    ("quit", copy::QUIT_ATTACHED_CONFIRM),
                    ("quit-and-stop", copy::QUIT_ATTACHED_ALSO_STOP),
                ]);
                d
            };
            dialog.set_close_response("cancel");
            let app = Rc::clone(&app);
            dialog.connect_response(None, move |dialog, response| {
                dialog.close();
                match response {
                    "quit" => {
                        app.quit_confirmed.set(true);
                        app.window.close();
                    }
                    "quit-and-stop" => {
                        app.quit_confirmed.set(true);
                        // Stop the separate watcher too, since that is what
                        // the contributor just asked for. The window closes
                        // either way.
                        app.call("shutdown", serde_json::json!({}), |app, _| {
                            app.window.close();
                        });
                    }
                    _ => {}
                }
            });
            dialog.present();
            glib::Propagation::Stop
        });
    }

    /// The tray icon's entire vocabulary reaches the window through here:
    /// a click of any kind raises it at the queue. See `tray.rs` for why
    /// that is the whole of it, and why absence of a tray (most Linux
    /// desktops, including plain GNOME) never reaches this at all.
    fn wire_tray(self: &Rc<Self>) {
        let rx = crate::tray::spawn();
        let app = Rc::clone(self);
        glib::spawn_future_local(async move {
            while rx.recv().await.is_ok() {
                app.stack.set_visible_child_name("queue");
                app.window.present();
            }
        });
    }

    /// One daemon call, with its answer delivered back on the main loop.
    pub fn call<F>(self: &Rc<Self>, method: &str, params: serde_json::Value, callback: F)
    where
        F: FnOnce(&Rc<App>, Result<serde_json::Value, String>) + 'static,
    {
        let id = self.worker.call(method, params);
        self.callbacks.borrow_mut().insert(
            id,
            Box::new(move |app, outcome| {
                if let Outcome::Call(result) = outcome {
                    callback(app, result)
                }
            }),
        );
    }

    pub fn preview<F>(self: &Rc<Self>, entry_id: &str, callback: F)
    where
        F: FnOnce(&Rc<App>, Result<(PreviewSummary, Option<String>), String>) + 'static,
    {
        let id = self.worker.preview(entry_id);
        self.callbacks.borrow_mut().insert(
            id,
            Box::new(move |app, outcome| {
                if let Outcome::Preview(result) = outcome {
                    callback(app, result)
                }
            }),
        );
    }

    /// Re-read everything the window renders. Cheap, and the honest response
    /// to any event.
    pub fn refresh(self: &Rc<Self>) {
        self.call("status", serde_json::json!({}), |app, result| {
            if let Ok(Ok(status)) = result.map(serde_json::from_value::<Status>) {
                app.render_health(&status);
                settings::render_status(app, &status);
                *app.status.borrow_mut() = Some(status);
            }
        });
        self.call("list_pending", serde_json::json!({}), |app, result| {
            let Ok(value) = result else { return };
            let entries: Vec<QueueEntry> =
                serde_json::from_value(value.get("pending").cloned().unwrap_or_default())
                    .unwrap_or_default();
            *app.entries.borrow_mut() = entries;
            queue::render(app);
            app.prefetch_previews();
        });
        history::refresh(self);
        settings::refresh(self);
    }

    /// Fill in the "would send / scrubbed" line for the first screenful of
    /// rows. Bounded on purpose -- see `PREVIEW_PREFETCH_LIMIT`.
    fn prefetch_previews(self: &Rc<Self>) {
        let wanted: Vec<String> = self
            .entries
            .borrow()
            .iter()
            .filter(|e| e.state == "pending")
            .take(PREVIEW_PREFETCH_LIMIT)
            .map(|e| e.entry_id.clone())
            .filter(|id| {
                !self.previews.borrow().contains_key(id) && !self.prefetching.borrow().contains(id)
            })
            .collect();
        for entry_id in wanted {
            self.prefetching.borrow_mut().insert(entry_id.clone());
            let key = entry_id.clone();
            self.preview(&entry_id, move |app, result| {
                app.prefetching.borrow_mut().remove(&key);
                if let Ok((summary, _)) = result {
                    app.previews.borrow_mut().insert(key.clone(), summary);
                    queue::render(app);
                }
            });
        }
    }

    fn render_health(self: &Rc<Self>, status: &Status) {
        match status.health.last_error_label.as_deref() {
            Some(label) => {
                self.health_label.set_text(copy::health_sentence(label));
                match copy::health_action(label) {
                    Some(action) => {
                        self.health_button.set_label(action);
                        self.health_button.set_visible(true);
                    }
                    None => self.health_button.set_visible(false),
                }
                self.health_banner.set_visible(true);
            }
            None => self.health_banner.set_visible(false),
        }
    }

    /// The 4-hour digest. Posted only when there is pending work, and its
    /// actions can only ever open the window or dismiss.
    fn post_digest(self: &Rc<Self>) {
        let entries = self.entries.borrow();
        let pending: Vec<&QueueEntry> = entries.iter().filter(|e| e.state == "pending").collect();
        if pending.is_empty() {
            return;
        }
        let mut labels: Vec<String> = pending.iter().map(|e| e.project_label.clone()).collect();
        labels.sort();
        labels.dedup();
        let body = crate::notify::digest_body(pending.len(), &labels);
        drop(entries);
        self.notify(copy::APP_NAME, &body);
    }

    /// Post a notification on a thread and, if the contributor pressed
    /// `Review`, bring the window forward at the queue.
    ///
    /// `Review` opens the window. That is the whole of what any notification
    /// action in this application can do.
    pub fn notify(self: &Rc<Self>, summary: &str, body: &str) {
        let (tx, rx) = async_channel::bounded(1);
        let summary = summary.to_string();
        let body = body.to_string();
        std::thread::spawn(move || {
            if let Some(action) = crate::notify::post(&summary, &body) {
                let _ = tx.send_blocking(action);
            }
        });
        let app = Rc::clone(self);
        glib::spawn_future_local(async move {
            if let Ok(crate::notify::Action::Review) = rx.recv().await {
                app.stack.set_visible_child_name("queue");
                app.window.present();
            }
        });
    }

    pub fn toast(self: &Rc<Self>, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }
}

/// A heading and a paragraph, the shape most of this window is made of.
///
/// The heading is set as an eyebrow rather than as a bold sentence: these
/// are field labels over values, not section titles, and setting them as
/// titles made every list of facts read like a stack of headlines.
pub fn titled_paragraph(title: &str, body: &str) -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Vertical, style::space::XXS);
    container.append(&style::eyebrow(title));
    let paragraph = gtk::Label::builder()
        .label(body)
        .xalign(0.0)
        .wrap(true)
        .build();
    paragraph.add_css_class("tc-body");
    container.append(&paragraph);
    container
}
