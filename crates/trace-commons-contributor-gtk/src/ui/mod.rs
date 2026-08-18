//! The window, which on Linux is the primary surface.
//!
//! GNOME has no system tray, so nothing here may depend on one: every
//! capability is reachable from this window, and the tray -- where a desktop
//! has a real one -- would only ever be a shortcut into it. Nothing in this
//! application tells a contributor to install a shell extension.

pub mod community_brand;
pub mod history;
pub mod mark;
pub mod onboarding;
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
use crate::model::{HistoryRollup, PreviewSummary, QueueEntry, Status};
use crate::worker::{Outcome, Worker};

pub const APP_ID: &str = "ai.tracecommons.Contributor";

/// The header bar's height, from §5.1's Linux column. Set as a size request
/// rather than in CSS because `AdwHeaderBar` derives its height from the
/// tallest thing packed into it, and the mark and the switcher are both
/// shorter than the bar the design draws.
const HEADER_HEIGHT: i32 = 46;

/// The reading column, shared by every screen in this window so the header's
/// switcher, the health banner and the queue's cards all sit on the same two
/// vertical lines.
const COLUMN_MAX: i32 = 840;
const COLUMN_TIGHTEN: i32 = 680;

/// The three screens, in the order the switcher shows them, with the icon
/// each one carries. One list, so the stack pages and the switcher items
/// cannot drift apart.
///
/// **Every icon name here must stay symbolic.** GTK recolours a symbolic
/// icon from its node's `color`, which is what lets style.css mute an
/// unselected item's icon and turn the selected one green without this code
/// setting anything. A full-colour icon ignores `color` and renders as-is,
/// so swapping one in would silently leave that item's icon un-recoloured
/// while the other two kept working -- a difference that shows up in a
/// screenshot months later and in no diff at all.
const SCREENS: [(&str, &str, &str); 3] = [
    ("queue", "Queue", "view-list-symbolic"),
    ("history", "History", "document-open-recent-symbolic"),
    ("settings", "Settings", "emblem-system-symbolic"),
];

/// The switcher's icon, §5.1 item 1. Set in pixels rather than by an icon
/// size so it matches the label it sits beside at every text scale.
const SWITCHER_ICON: i32 = 13;

type Callback = Box<dyn FnOnce(&Rc<App>, Outcome)>;

/// An approval that has been made but has not left the machine yet.
///
/// `hold_until` is the daemon's own instant, read from the `approve`
/// response. Nothing here is computed from a duration this process picked --
/// see `docs/contributor-daemon-ipc-v1_1.md` on the approval hold.
pub struct PendingUndo {
    pub entry_id: String,
    pub project_label: String,
    /// When this window offered the undo, which is what the bar's "held 41s"
    /// counts up from. It counts *up* on purpose: the app cannot see the
    /// watcher's sweep, so a countdown would be a promise it cannot keep.
    pub approved_at: chrono::DateTime<chrono::Utc>,
    pub hold_until: chrono::DateTime<chrono::Utc>,
}

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
    ///
    /// This is the clamped column, not the banner box inside it: showing and
    /// hiding the column is what also removes its top margin, so a window
    /// with nothing wrong has no gap under the header.
    health_banner: adw::Clamp,
    health_label: gtk::Label,
    health_button: gtk::Button,
    /// The count on the switcher's Queue item. Hidden at zero rather than
    /// drawn as a "0": an empty badge is a decoration, and the queue's own
    /// empty state already says the true thing.
    queue_badge: gtk::Label,

    callbacks: RefCell<HashMap<u64, Callback>>,
    pub entries: RefCell<Vec<QueueEntry>>,
    pub status: RefCell<Option<Status>>,
    /// The week band's three figures. Read here rather than passed down from
    /// `history` so the queue can draw the band without depending on which
    /// screen the contributor happens to be looking at.
    pub rollup: RefCell<HistoryRollup>,
    /// The approval the undo bar is currently offering to take back, if any.
    pub undo: RefCell<Option<PendingUndo>>,
    /// The handle for the once-a-second tick that moves the undo bar's
    /// elapsed figure, held so a second approval can stop the first one's
    /// timer instead of leaving it running. Approving is a loop -- the sheet
    /// approves and advances to the next entry -- so back-to-back approvals
    /// inside one hold window are the normal path, not an edge case.
    undo_tick: RefCell<Option<glib::SourceId>>,
    /// Preview summaries, keyed by entry id, so a row can show what would be
    /// sent without re-running the pipeline on every redraw.
    pub previews: RefCell<HashMap<String, PreviewSummary>>,
    /// What each withdrawal attempt did, keyed by submission id.
    ///
    /// Kept here rather than in a screen-level banner because a failure
    /// next to a row that still reads "In the commons" leaves it genuinely
    /// ambiguous whether the trace was withdrawn. History rebuilds its rows
    /// wholesale on every refresh, so the outcome has to outlive the widget
    /// that reported it.
    pub withdrawals: RefCell<HashMap<String, history::Withdrawal>>,
    /// `queue_outcome_counts`: how many queued sessions ended each way.
    ///
    /// A `BTreeMap` so the disclosure lists them in a stable order rather
    /// than in whatever order a hash map happened to produce this second.
    pub outcome_counts: RefCell<std::collections::BTreeMap<String, u64>>,
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

        // Pages and switcher items are built from the same list, in the same
        // order, so a screen cannot be renamed in one place and not the
        // other.
        let pages: [&gtk::Box; 3] = [&queue.root, &history.root, &settings.root];
        for ((name, label, icon_name), page) in SCREENS.into_iter().zip(pages) {
            stack
                .add_titled(page, Some(name), label)
                .set_icon_name(Some(icon_name));
        }

        let queue_badge = gtk::Label::builder().visible(false).build();
        queue_badge.add_css_class("tc-count-badge");
        queue_badge.set_valign(gtk::Align::Center);

        // The bar's own close button is left to `AdwHeaderBar`'s window
        // controls rather than hand-built. The design draws a 24px round
        // `x` on a faint wash, which is exactly what GNOME's own decoration
        // already is -- and a hand-built one would lose the window menu, the
        // keyboard path to it, and whatever button layout the contributor
        // has chosen in their desktop settings.
        let header = adw::HeaderBar::builder()
            .title_widget(&view_switcher(&stack, &queue_badge))
            .build();
        header.add_css_class("tc-header");
        header.set_size_request(-1, HEADER_HEIGHT);
        // The mark, drawn from its own geometry rather than shipped as an
        // asset. 20px is the header-bar size the design spec names.
        header.pack_start(&mark::framed(20));

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
            .build();
        health_banner.append(&health_glyph);
        health_banner.append(&health_label);
        health_banner.append(&health_button);
        health_banner.add_css_class("tc-banner");

        // The banner sits above the stack rather than inside the queue, so a
        // contributor reading History or Settings still learns that
        // contributions are held up. The design draws it on the queue
        // because the queue is the only screen it drew; putting it here says
        // the same thing on every screen and never says it twice.
        //
        // It is clamped to the same column as the screens below it, which is
        // the only reason this is a clamp and not a bare margin: an
        // unclamped banner runs the full width of a maximised window while
        // the cards under it stop at 840, and the two stop looking like one
        // document.
        let banner_column = adw::Clamp::builder()
            .maximum_size(COLUMN_MAX)
            .tightening_threshold(COLUMN_TIGHTEN)
            .child(&health_banner)
            .visible(false)
            .margin_top(style::space::L)
            .margin_start(style::space::XL)
            .margin_end(style::space::XL)
            .build();

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("tc-root");
        content.append(&header);
        content.append(&banner_column);
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
            health_banner: banner_column,
            health_label,
            health_button,
            queue_badge,
            callbacks: RefCell::new(HashMap::new()),
            entries: RefCell::new(Vec::new()),
            status: RefCell::new(None),
            rollup: RefCell::new(HistoryRollup::default()),
            undo: RefCell::new(None),
            undo_tick: RefCell::new(None),
            previews: RefCell::new(HashMap::new()),
            withdrawals: RefCell::new(HashMap::new()),
            outcome_counts: RefCell::new(Default::default()),
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
        // run on their own threads. The portal request also classifies
        // whether any backend answered at all, so a desktop with none does
        // not silently no-op -- see `settings::wire_background_probe` for
        // where that classification is actually shown to a contributor.
        let portal_probe = crate::portal::spawn_request();
        settings::wire_background_probe(&app, portal_probe);

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
                // Both halves of "does this device need onboarding" are the
                // daemon's to answer, so the question is asked here rather
                // than at window construction, where neither is known yet.
                onboarding::present_if_needed(app, status.logged_in, status.tenant_id.as_deref());
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
        // Why sessions are no longer waiting, as counts.
        //
        // `list_pending` above returns pending entries and nothing else --
        // `queue.pending()` filters on exactly that state -- so the queue
        // cannot answer this from the entries it already holds, however
        // many resolved ones it looks for among them. This is the method
        // that answers it, and it is the only one that does.
        self.call(
            "queue_outcome_counts",
            serde_json::json!({}),
            |app, result| {
                let counts = result
                    .ok()
                    .and_then(|v| serde_json::from_value(v.get("reasons").cloned()?).ok())
                    .unwrap_or_default();
                *app.outcome_counts.borrow_mut() = counts;
                queue::render(app);
            },
        );
        // The queue's week band needs the same rollup History does. It is
        // read here rather than handed over by `history::refresh` so the
        // band is filled in whether or not History has ever been opened;
        // `history_rollup` is a read of counters the daemon already holds.
        self.call("history_rollup", serde_json::json!({}), |app, result| {
            if let Ok(Ok(rollup)) = result.map(serde_json::from_value::<HistoryRollup>) {
                *app.rollup.borrow_mut() = rollup;
                queue::render(app);
            }
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

    /// Put the number of decisions owed on the switcher's Queue item.
    ///
    /// Called by `queue::render` rather than computed here, so there is
    /// exactly one place that decides which entries count as waiting.
    pub fn set_queue_count(self: &Rc<Self>, waiting: usize) {
        self.queue_badge.set_label(&waiting.to_string());
        self.queue_badge.set_visible(waiting > 0);
    }

    /// Offer to take back an approval, on the queue rather than in a toast.
    ///
    /// Recovery belongs on the surface a contributor is already looking at:
    /// a toast is gone in seconds and takes the only path back with it,
    /// whereas the bar stays for the whole of the daemon's hold and says in
    /// words what it can and cannot promise.
    ///
    /// `hold_until` is the daemon's instant from the `approve` response.
    /// `None` means no undo may be offered, so none is -- the contributor is
    /// told plainly instead.
    pub fn offer_undo(
        self: &Rc<Self>,
        entry_id: &str,
        project_label: &str,
        hold_until: Option<chrono::DateTime<chrono::Utc>>,
    ) {
        let Some(hold_until) = hold_until else {
            self.toast(copy::APPROVED_NO_UNDO);
            return;
        };
        *self.undo.borrow_mut() = Some(PendingUndo {
            entry_id: entry_id.to_string(),
            project_label: project_label.to_string(),
            approved_at: chrono::Utc::now(),
            hold_until,
        });
        queue::render_undo(self);

        // One tick per second, and it only moves the elapsed figure -- the
        // bar is not rebuilt, so a contributor whose pointer is on `Undo`
        // does not have it pulled out from under them.
        //
        // Any tick left over from a previous approval is stopped first. The
        // old one would not have stopped itself: it breaks on the pending
        // undo being absent or expired, and this call has just replaced it
        // with a later one, so it would keep ticking alongside the new timer
        // until the last hold ran out.
        self.stop_undo_tick();
        let app = Rc::clone(self);
        let source = glib::timeout_add_seconds_local(1, move || {
            let expired = app
                .undo
                .borrow()
                .as_ref()
                .is_none_or(|undo| chrono::Utc::now() >= undo.hold_until);
            if expired {
                app.undo.borrow_mut().take();
                // Forget our own handle before breaking, so a later
                // `stop_undo_tick` does not try to remove a source that has
                // already finished.
                app.undo_tick.borrow_mut().take();
                queue::render_undo(&app);
                return glib::ControlFlow::Break;
            }
            queue::render_undo(&app);
            glib::ControlFlow::Continue
        });
        *self.undo_tick.borrow_mut() = Some(source);
    }

    /// Stop the undo bar's tick if one is running. Idempotent.
    fn stop_undo_tick(&self) {
        if let Some(source) = self.undo_tick.borrow_mut().take() {
            source.remove();
        }
    }

    /// Withdraw the undo bar without cancelling: the hold simply runs out.
    pub fn dismiss_undo(self: &Rc<Self>) {
        self.stop_undo_tick();
        self.undo.borrow_mut().take();
        queue::render_undo(self);
    }
}

/// The segmented view switcher, §5.1's Linux column.
///
/// Hand-built rather than an `AdwViewSwitcher` because the design puts a
/// count badge inside one item, and `AdwViewSwitcher` builds its own buttons
/// from the stack pages' titles and icons with nowhere to put one.
///
/// The stack, not this widget, is the source of truth for which screen is
/// showing: the tray and every notification action set the stack directly,
/// so the items follow it rather than the other way round.
fn view_switcher(stack: &adw::ViewStack, queue_badge: &gtk::Label) -> gtk::Box {
    let track = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(0)
        .valign(gtk::Align::Center)
        .build();
    track.add_css_class("tc-switcher");

    let mut items: Vec<(&'static str, gtk::ToggleButton)> = Vec::new();
    for (name, label, icon_name) in SCREENS {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, style::space::XXS);
        // A real `GtkImage`, not a glyph: §6.6 turns the selected item's icon
        // green while its label stays ink, and style.css matches that on the
        // `image` node rather than on a class this code would have to
        // remember to set.
        let icon = gtk::Image::from_icon_name(icon_name);
        icon.set_pixel_size(SWITCHER_ICON);
        content.append(&icon);
        content.append(&gtk::Label::new(Some(label)));
        if name == "queue" {
            content.append(queue_badge);
        }
        let item = gtk::ToggleButton::builder().child(&content).build();
        item.add_css_class("tc-tab");
        // `flat` drops Adwaita's own button face, so the track underneath is
        // what the unselected items sit on.
        item.add_css_class("flat");
        if let Some((_, first)) = items.first() {
            item.set_group(Some(first));
        }
        let stack = stack.clone();
        item.connect_toggled(move |item| {
            if item.is_active() {
                stack.set_visible_child_name(name);
            }
        });
        track.append(&item);
        items.push((name, item));
    }

    let followers = items.clone();
    stack.connect_notify_local(Some("visible-child-name"), move |stack, _| {
        let Some(showing) = stack.visible_child_name() else {
            return;
        };
        for (name, item) in &followers {
            if *name == showing.as_str() {
                item.set_active(true);
            }
        }
    });
    if let Some((_, first)) = items.first() {
        first.set_active(true);
    }

    track
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
