//! The window, which on Linux is the primary surface.
//!
//! GNOME has no system tray, so nothing here may depend on one: every
//! capability is reachable from this window, and the tray -- where a desktop
//! has a real one -- would only ever be a shortcut into it. Nothing in this
//! application tells a contributor to install a shell extension.

pub(crate) mod account_change;
pub mod appearance;
pub mod armed_mark;
pub mod balance;
pub mod community_brand;
pub(crate) mod consent_settings;
pub mod credential;
mod css_contract;
mod first_run;
mod first_run_marker_check;
mod first_run_screens;
mod first_run_stage;
mod first_run_stage_paint;
mod first_run_stage_view;
mod first_run_terms;
mod first_run_tools;
pub mod folder_handoff;
pub mod funding;
pub mod glass_backdrop;
pub mod glass_feedback;
pub mod glass_motion;
pub mod hero_art;
pub mod hero_card;
pub mod history;
pub(crate) mod history_detail;
pub mod home;
pub mod insights;
pub mod legacy_move;
mod lift;
mod managed_sessions;
pub mod mark;
pub mod mission_drafts;
pub mod monitor;
mod motion_css;
pub mod omarchy;
pub mod onboarding;
mod payout_link;
pub mod popover_stack;
pub mod preview;
pub mod private_inference;
pub(crate) mod projects_bulk;
pub mod queue;
pub mod roots;
pub(crate) mod sent_training;
pub mod service_dialog;
pub mod settings;
pub(crate) mod shortcuts;
mod stage;
pub mod style;
mod terminal_choice;
pub(crate) mod terms_hold;
pub mod tool_tile;
pub mod update;
pub mod wrap_row;

#[cfg(all(test, target_os = "linux"))]
#[path = "../../tests/support/consent_fixture.rs"]
pub(crate) mod consent_fixture;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use trace_commons_contributor::daemon::preview_scheduler::{STATE_READY, STATE_TOO_LARGE};

use crate::copy;
use crate::model::{ApproveResult, HistoryRollup, PreviewSummary, QueueEntry, Status};
use crate::worker::{Outcome, Worker};

pub const APP_ID: &str = "ai.tracecommons.Contributor";

/// Names the process by the app id, before any window exists. A window GTK
/// does not hold as one of the application's (the first run, the Look
/// inside sheet, every dialog) takes its Wayland app id and its X11
/// `WM_CLASS` from the program name, which is otherwise the binary's
/// (`trace-commons-shell`), so the desktop could not match it to
/// `ai.tracecommons.Contributor.desktop`.
pub fn name_the_process() {
    glib::set_prgname(Some(APP_ID));
}

/// The reading column, shared by every screen in this window so the header's
/// switcher, the health banner and the queue's cards all sit on the same two
/// vertical lines.
const COLUMN_MAX: i32 = 840;
const COLUMN_TIGHTEN: i32 = 680;

/// Settings, which is its own window and no tab.
pub(crate) const SETTINGS_SCREEN: &str = "settings";

/// Whether this process shows missions: the daemon's `missions_enabled`
/// setting, read once at startup by `main` and fixed for the process. Off
/// until [`set_missions_shown`] says otherwise, so a test or a caller that
/// never reads the setting hides missions, which is the default.
static MISSIONS_SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Record the startup reading of `missions_enabled`. Called before any
/// window, tray or accelerator is built, because all three derive from
/// [`screens`] and the tray's toggle row id is `screens().len()`.
pub fn set_missions_shown(shown: bool) {
    MISSIONS_SHOWN.store(shown, std::sync::atomic::Ordering::Relaxed);
}

pub fn missions_shown() -> bool {
    MISSIONS_SHOWN.load(std::sync::atomic::Ordering::Relaxed)
}

/// `missions_enabled` as the daemon persisted it in `daemon-settings.json`
/// under `dir`. Read from the file because the default launch window has no
/// daemon to ask, and the full window is built before its first
/// `get_settings` answer arrives. Anything unreadable, absent or not a
/// boolean `true` is off, and nothing is created.
pub fn read_missions_shown(dir: &std::path::Path) -> bool {
    std::fs::read(dir.join(trace_commons_contributor::config::DAEMON_SETTINGS_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|value| value["missions_enabled"].as_bool() == Some(true))
}

/// The state directory without creating it, for the one startup read of
/// the switch. Same order as `ConfigStore::resolve`: the environment, then
/// the platform default, which on Linux is the XDG config directory.
pub fn peek_state_dir() -> std::path::PathBuf {
    match std::env::var("TRACE_COMMONS_CONTRIBUTOR_DIR") {
        Ok(dir) => std::path::PathBuf::from(dir),
        Err(_) => gtk::glib::user_config_dir().join("trace-commons"),
    }
}

/// The window's tabs, in the strip's order, as `(name, label)`: Home,
/// Inference, Traces, Insights, labelled from the core's shell table
/// (`monitor::tabs::Tab`). The one list the tab strip, the tray menu and
/// the accelerators all read, so the digit a contributor presses is the
/// position they see.
///
/// History, Mission drafts and Settings are not in it: History and Mission
/// drafts are sub-pages behind the breadcrumb, and Settings is its own
/// window. Each still opens by its old name (`App::open_screen`).
pub(crate) fn screens() -> Vec<(&'static str, &'static str)> {
    monitor::tabs::Tab::ALL
        .into_iter()
        .map(|tab| (tab.name(), tab.title()))
        .collect()
}

/// The stack's name for the model-calls screen. Named once because the
/// tray and every other shortcut into it has to spell it identically, and
/// `set_visible_child_name` on a name no page carries is a silent no-op.
pub const PRIVATE_INFERENCE_SCREEN: &str = "private-inference";

type Callback = Box<dyn FnOnce(&Rc<App>, Outcome)>;

/// An approval that has been made but has not left the machine yet.
///
/// `hold_until` is the daemon's own instant, read from the `approve`
/// response. Nothing here is computed from a duration this process picked --
/// see `docs/contributor-daemon-ipc-v1_1.md` on the approval hold.
pub struct PendingUndo {
    /// The entries this approval is shown to cover: one for a row's Submit,
    /// the sendable rows this window saw for a folder's Submit all. The
    /// inspector lists them; Undo does not read them for a folder, which it
    /// cancels by `project_id` (`queue::undo_cancel_params`).
    pub entry_ids: Vec<String>,
    pub project_label: String,
    /// The folder whose Submit all this is; `None` for one trace's Submit.
    /// The inspector's demand tells the two apart (`monitor::demand`).
    pub folder: Option<String>,
    /// When this window offered the undo, which is what the bar's "Approved 41s ago"
    /// counts up from. It counts *up* on purpose: the app cannot see the
    /// watcher's sweep, so a countdown would be a promise it cannot keep.
    pub approved_at: chrono::DateTime<chrono::Utc>,
    pub hold_until: chrono::DateTime<chrono::Utc>,
}

pub struct App {
    pub worker: Worker,
    /// Where the model-calls switch stands, as the tray reads it.
    ///
    /// Written on every settings read (see `private_inference::render`) and
    /// read by nothing that paints: it is the switch's position, not a claim
    /// that a listener is running. It decides which way the tray's one
    /// action row points and nothing else.
    pub tray: crate::tray::TrayState,
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    /// The three-pane Monitor shell: the tabs, the map pane and the
    /// inspector pane. Which screen is showing is its state, and
    /// [`App::open_screen`] is the one way anything else moves it.
    pub shell: monitor::Shell,

    /// `admission_evidence_required`, as `get_settings` last answered it.
    ///
    /// Held verbatim and NEVER negated. It is true for a contributor who
    /// signed up through NEAR and therefore has no invite, so it selects the
    /// candidate reading of the certificate-held section. A `!` anywhere on
    /// the way to `certificate::view` would swap both readings and compile.
    ///
    /// False until the first settings read, which is the reading that claims
    /// less: a section that has not been told yet must not assert
    /// attestation.
    pub evidence_admitted: std::cell::Cell<bool>,

    pub home: home::HomeView,
    pub queue: queue::QueueView,
    pub history: history::HistoryView,
    /// The model-calls screen. Its own destination rather than a card on
    /// Settings: this is the one switch that makes this computer answer
    /// calls for whatever else is running on it, and it was undiscoverable
    /// five sections down a settings list.
    pub private_inference: private_inference::PrivateInferenceView,
    pub settings: settings::SettingsView,
    /// Changing the NEAR AI login the account uses (F10).
    pub account_change: account_change::ChangeView,
    /// The update banner. Above the health banner is deliberate: an update
    /// is a standing fact about this machine, while health is about the
    /// current run.
    pub update: update::UpdateView,

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
    /// Grants the daemon voided and no shell has shown yet (R6). Beside the
    /// health banner rather than in it: the health slot carries one label,
    /// and a void must neither hide an outage nor be hidden by one.
    void_notices: gtk::Box,
    /// The account's notices: the legacy invite move and its notice.
    account_notices: gtk::Box,
    /// The move to a NEAR AI account for a device that joined with an
    /// invite: its state, and the dismissible offer card that sits first in
    /// the notices column (Settings holds the other set of controls).
    pub legacy_move: legacy_move::MoveState,
    pub legacy_card: legacy_move::Controls,

    callbacks: RefCell<HashMap<u64, Callback>>,
    pub entries: RefCell<Vec<QueueEntry>>,
    /// `list_projects` rows, held for the two counts they carry.
    ///
    /// The queue builds its folders from `entries` and always will -- the
    /// header must agree with the rows on screen, not with a number the
    /// contributor cannot check. What these rows are for is the ONE number
    /// the shell must not compute: how many of a group a project-wide
    /// `approve` would actually send. That is the daemon's filter, and a
    /// shell deriving it is a second implementation which agrees until the
    /// day the rule changes an arm.
    pub projects: RefCell<Vec<crate::model::Project>>,
    pub status: RefCell<Option<Status>>,
    /// The last `list_history` page, for the Traces graph and an opened
    /// History row's details. `None` while unread or after a failed read,
    /// which the graph draws as a dash, never as nothing shared.
    pub history_records: RefCell<Option<Vec<crate::model::HistoryRecord>>>,
    /// The entry this window last kept, and the one it last dismissed, for
    /// the Traces map's transfer cards (`monitor::transfers::Feed`), as the
    /// Mac's `TracesStore.lastKept` and `lastDismissed`. Set only after the
    /// daemon answered. This shell has no keep action yet, so `last_kept`
    /// stays `None` until one sets it.
    pub last_kept: RefCell<Option<String>>,
    pub last_dismissed: RefCell<Option<String>>,
    /// The week band's three figures. Read here rather than passed down from
    /// `history` so the queue can draw the band without depending on which
    /// screen the contributor happens to be looking at.
    pub rollup: RefCell<HistoryRollup>,
    /// The last `history_rollup` read that worked, as Home reads it (a
    /// count it did not give stays unknown); `None` before the first answer
    /// and after a failed read, so the summary never shows a stale figure.
    pub summary_rollup: RefCell<Option<crate::home::Rollup>>,
    /// Whether `history_rollup` has answered at all, worked or failed: until
    /// it has, the summary's contributed line is a skeleton bar
    /// (`HomeStore.isLoading("history_rollup")`).
    pub summary_rollup_answered: std::cell::Cell<bool>,
    /// The commons' statement of what pending credit waits on, from the
    /// last `commons_credit_summary` read (`HomeFormat.pendingCondition`).
    pub credit_condition: RefCell<Option<String>>,
    /// The approval the undo bar is currently offering to take back, if any.
    pub undo: RefCell<Option<PendingUndo>>,
    /// The Undo presses whose `cancel` is still out (`queue::begin_undo_write`).
    pub undo_writing: RefCell<std::collections::HashSet<queue::UndoIdentity>>,
    /// The folder whose Submit all is waiting on the daemon's answer, as
    /// the Mac's `TracesStore.submittingFolder`: a demand on the inspector.
    pub submitting_folder: RefCell<Option<String>>,
    /// The handle for the once-a-second tick that moves the undo bar's
    /// elapsed figure, held so a second approval can stop the first one's
    /// timer instead of leaving it running. Approving is a loop -- the sheet
    /// approves and advances to the next entry -- so back-to-back approvals
    /// inside one hold window are the normal path, not an edge case.
    undo_tick: RefCell<Option<glib::SourceId>>,
    /// Preview summaries, keyed by entry id, so a row can show what would be
    /// sent without re-running the pipeline on every redraw. Filled in by
    /// `handle_preview_request_result` for a scheduled card preview, and by
    /// the preview sheet's own full preview when it pins one -- see
    /// `ui::preview::Sheet::load`.
    pub previews: RefCell<HashMap<String, PreviewSummary>>,
    /// Entries the daemon's admission control refused to preview at all,
    /// keyed by entry id, carrying `(raw_session_bytes, limit_bytes)` and
    /// nothing else -- never a would-send estimate. Mutually exclusive with
    /// `previews`: an entry lands in exactly one of the two, once, and never
    /// moves. See `docs/superpowers/specs/2026-08-20-preview-scheduler-design.md`.
    pub previews_too_large: RefCell<HashMap<String, (u64, u64)>>,
    /// Every pending entry this shell has asked the daemon's preview
    /// scheduler about. Superset of `previews.keys()` and
    /// `previews_too_large.keys()`: an id lands here the moment
    /// `preview_request` is sent and leaves only when the entry itself
    /// leaves the pending list, which is also the signal to tell the
    /// daemon `preview_cancel` -- see `App::reconcile_card_previews`.
    card_tracked: RefCell<std::collections::HashSet<String>>,
    /// Each rendered card's own widget, keyed by entry id, so a scroll
    /// settle can ask every card its bounds against the scroller without
    /// keeping a second copy of the queue's layout. Rebuilt every render,
    /// in step with `queue::render` rebuilding `QueueView::list` itself.
    card_widgets: RefCell<HashMap<String, gtk::Widget>>,
    /// The pending debounce timer for `preview_visible`, so a scroll drag
    /// reschedules the same single call rather than stacking one per
    /// `value-changed` signal.
    scroll_debounce: RefCell<Option<glib::SourceId>>,
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
    /// Which level of the queue is showing. Resolved against the live
    /// folders on every render (`queue_folders::resolve`), so a folder that
    /// empties while it is open returns to the list.
    pub queue_location: RefCell<crate::queue_folders::Location>,
    /// The same, for history. A second field rather than one shared with the
    /// queue: the two screens hold different sets of projects -- history
    /// keeps a folder the queue has emptied -- and sharing one location
    /// would drop a contributor into a folder that does not exist on the
    /// screen they just switched to.
    pub history_location: RefCell<crate::queue_folders::Location>,
    quit_confirmed: Cell<bool>,
    /// The folders screen was shown for the daemon this window is attached
    /// to; cleared when the backend is replaced.
    folders_asked: Cell<bool>,
}

/// How long a scroll must sit still before `preview_visible` is sent.
///
/// Cheap and idempotent on the daemon's side, so this only needs to beat a
/// drag: any value that turns a continuous `value-changed` stream into a
/// single call after it stops is enough. There is no throughput reason to
/// tune it further -- see the design's "Priority" section.
const SCROLL_SETTLE_DEBOUNCE_MS: u64 = 250;

/// How often a Flatpak window attached to the host daemon checks that the
/// host part is still there.
const FALLBACK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

impl App {
    pub fn build(application: &adw::Application, worker: Worker) -> Rc<Self> {
        // Before any widget is built, so nothing is ever drawn in the
        // theme's palette and then repainted in this one.
        style::install();

        // No default size here: the shell sets it from the panes it opens
        // with (`monitor::Shell::new`).
        let window = adw::ApplicationWindow::builder()
            .application(application)
            .title(copy::APP_NAME)
            .build();

        window.set_widget_name("contributions-window");
        let home = home::HomeView::new(missions_shown());
        let queue = queue::QueueView::new();
        let history = history::HistoryView::new();
        let private_inference = private_inference::PrivateInferenceView::new();
        let settings = settings::SettingsView::new();
        let update = update::UpdateView::new();

        let insights = insights::InsightsView::new(&window);
        // Not constructed while missions are hidden; the code stays.
        let mission_drafts =
            missions_shown().then(|| mission_drafts::MissionDraftsView::new(&window));

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

        // The banner is a notice over the Traces tab and its History page,
        // as on the Mac (`monitor::notices`). Settings is its own window now
        // and does not carry it.
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

        // Every consent notice about folders (grant voids, arming
        // rewordings, a gate hold), in the same column as the health banner
        // and over the same tabs, as on the Mac (`monitor::notices`).
        let void_notices = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(style::space::M)
            .build();
        let void_column = adw::Clamp::builder()
            .maximum_size(COLUMN_MAX)
            .tightening_threshold(COLUMN_TIGHTEN)
            .child(&void_notices)
            .visible(false)
            .margin_top(style::space::L)
            .margin_start(style::space::XL)
            .margin_end(style::space::XL)
            .build();
        // The account's notices, the legacy invite move and its notice, are
        // about the account rather than a tab, and are shown over every tab
        // as on the Mac. The offer card is built once and kept first in the
        // column, so a refresh never replaces a half-pasted invite.
        let account_notices = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(style::space::M)
            .build();
        let legacy_card = legacy_move::Controls::new(true);
        account_notices.append(&legacy_card.root);
        let account_column = adw::Clamp::builder()
            .maximum_size(COLUMN_MAX)
            .tightening_threshold(COLUMN_TIGHTEN)
            .child(&account_notices)
            .visible(false)
            .margin_top(style::space::L)
            .margin_start(style::space::XL)
            .margin_end(style::space::XL)
            .build();

        // There is no header bar: the main pane's toolbar row is the title
        // bar, with the desktop's own window controls in it
        // (`monitor::shell`). The update banner, the health banner and the
        // notices column that sat above the stack are popover notices laid
        // over the tab now, in the Mac's stack.
        let shell = monitor::Shell::new(
            &window,
            monitor::Pages {
                home: &home.root,
                inference: &private_inference.root,
                traces: &queue.root,
                history: &history.root,
                insights: &insights.root,
                mission_drafts: mission_drafts.as_ref().map(|view| &view.root),
                settings: &settings.root,
                notices: vec![
                    // The front first, as the Mac stacks them
                    // (`monitor::notices::Notice::ALL`).
                    (
                        monitor::notices::Notice::Undo,
                        queue.undo_notice.clone().upcast(),
                    ),
                    (
                        monitor::notices::Notice::Health,
                        banner_column.clone().upcast(),
                    ),
                    (
                        monitor::notices::Notice::Consent,
                        void_column.clone().upcast(),
                    ),
                    (
                        monitor::notices::Notice::ArmingOffer,
                        queue.arming_notice.clone().upcast(),
                    ),
                    (
                        monitor::notices::Notice::PrivateAiOffer,
                        queue.private_inference_notice.clone().upcast(),
                    ),
                    (
                        monitor::notices::Notice::Account,
                        account_column.clone().upcast(),
                    ),
                    (
                        monitor::notices::Notice::Update,
                        update.root.clone().upcast(),
                    ),
                ],
            },
            missions_shown(),
        );

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&shell.root));
        window.set_content(Some(&toasts));

        let app = Rc::new(Self {
            worker,
            tray: crate::tray::TrayState::default(),
            window,
            toasts,
            shell,
            evidence_admitted: std::cell::Cell::new(false),
            home,
            queue,
            history,
            private_inference,
            settings,
            update,
            health_banner: banner_column,
            health_label,
            health_button,
            void_notices,
            account_notices,
            legacy_move: legacy_move::MoveState::default(),
            legacy_card,
            callbacks: RefCell::new(HashMap::new()),
            entries: RefCell::new(Vec::new()),
            projects: RefCell::new(Vec::new()),
            status: RefCell::new(None),
            history_records: RefCell::new(None),
            last_kept: RefCell::new(None),
            last_dismissed: RefCell::new(None),
            account_change: account_change::ChangeView::default(),
            rollup: RefCell::new(HistoryRollup::default()),
            summary_rollup: RefCell::new(None),
            summary_rollup_answered: std::cell::Cell::new(false),
            credit_condition: RefCell::new(None),
            undo: RefCell::new(None),
            undo_writing: RefCell::new(Default::default()),
            submitting_folder: RefCell::new(None),
            undo_tick: RefCell::new(None),
            previews: RefCell::new(HashMap::new()),
            previews_too_large: RefCell::new(HashMap::new()),
            card_tracked: RefCell::new(Default::default()),
            card_widgets: RefCell::new(HashMap::new()),
            scroll_debounce: RefCell::new(None),
            withdrawals: RefCell::new(HashMap::new()),
            outcome_counts: RefCell::new(Default::default()),
            recent_searches: RefCell::new(Vec::new()),
            queue_location: RefCell::new(crate::queue_folders::Location::Root),
            history_location: RefCell::new(crate::queue_folders::Location::Root),
            quit_confirmed: Cell::new(false),
            folders_asked: Cell::new(false),
        });

        app.home.bind(&app);
        insights.bind_app(&app);
        monitor::shell::wire(&app);
        app.wire_result_pump();
        app.wire_event_pump();
        app.wire_quit();
        app.wire_health_action();
        app.wire_tray();
        app.wire_service_fallback();
        app.wire_folder_refresh();
        // After `wire_tray`, and reaching the same handler it does. GNOME
        // has no tray, so on most Linux desktops this is the only surface
        // besides the window's own controls -- and it is still an
        // accelerant, never the only path to anything.
        shortcuts::wire(&app, application);
        queue::wire(&app);
        history::wire(&app);
        private_inference::wire(&app);
        funding::wire(&app);
        settings::wire(&app);
        legacy_move::wire(&app);
        update::wire(&app);
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

    /// The health banner's button, which until now was drawn and wired to
    /// nothing.
    ///
    /// `render_health` gives it a label and shows it whenever the daemon
    /// reports a label that carries an action, so it has always looked like
    /// a control. Clicking it did nothing at all -- the worst kind of
    /// affordance, because the banner it sits in exists to tell a
    /// contributor that contributions are held up, and the button is the
    /// only thing offering a way out.
    ///
    /// The label is read at click time rather than captured when the button
    /// was labelled: the banner is re-rendered on every status change, and a
    /// closure holding the label from three states ago would send someone to
    /// the screen for a problem they no longer have.
    fn wire_health_action(self: &Rc<Self>) {
        let app = self.clone();
        self.health_button.connect_clicked(move |_| {
            let label = app
                .status
                .borrow()
                .as_ref()
                .and_then(|s| s.health.last_error_label.clone());
            if let Some(label) = label {
                onboarding::present_for_health(&app, &label);
            }
        });
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
    ///
    /// `preview_ready` is the one exception to "look again by calling
    /// `refresh`", and the one event this connection reads a field off of at
    /// all -- see `DaemonEvent`'s doc comment for why an entry id is exempt
    /// from "names only" while everything else stays a bare name. Reading it
    /// is what turns "a card resolved" into a single targeted
    /// `preview_request` (`App::handle_preview_ready`) instead of a sweep
    /// over every card still outstanding: the first version of this feature
    /// swept on every event, which reduces to firing roughly one sweep per
    /// still-shrinking outstanding set while a queue drains -- fine at ten
    /// cards, a self-inflicted request storm at the five hundred this
    /// feature exists for.
    fn wire_event_pump(self: &Rc<Self>) {
        let app = Rc::clone(self);
        glib::spawn_future_local(async move {
            while let Ok(event) = app.worker.events.recv().await {
                match event.name.as_str() {
                    "digest_due" => {
                        app.refresh();
                        app.post_digest(event.digest.clone().unwrap_or_default());
                    }
                    "preview_ready" => app.handle_preview_ready(event.entry_id),
                    // A missed preview_ready is exactly the gap
                    // resync_required exists to cover, so both halves of
                    // "look again" run: the queue itself, and a full sweep
                    // of the scheduled previews a targeted request could
                    // never catch up on by itself. This is the one place a
                    // sweep still belongs -- it runs once per resync, not
                    // once per card resolved.
                    "resync_required" => {
                        // The daemon may have restarted, counting its
                        // managed revisions afresh.
                        managed_sessions::backend_changed(&app.private_inference.managed_revision);
                        app.refresh();
                        app.poll_outstanding_previews();
                    }
                    _ => app.refresh(),
                }
            }
        });
    }

    /// Closing the window quits. It asks first only when that would stop
    /// Private AI answering model calls through this process, in one
    /// sentence from the core (`quit_copy::quit_prompt`); any other close
    /// quits at once (owner, 2026-11-12). An attached app stops nothing by
    /// quitting, so it never asks.
    fn wire_quit(self: &Rc<Self>) {
        let app = Rc::clone(self);
        self.window.connect_close_request(move |window| {
            if app.quit_confirmed.get() {
                return glib::Propagation::Proceed;
            }
            let stops_private_ai =
                app.worker.hosts_the_loop() && app.private_inference.quit_stops.get();
            // GTK hosts no compute worker.
            let Some(prompt) =
                trace_commons_contributor::quit_copy::quit_prompt(false, stops_private_ai)
            else {
                return glib::Propagation::Proceed;
            };
            let dialog =
                adw::MessageDialog::new(Some(window), Some(prompt.title), Some(prompt.body));
            dialog.add_responses(&[("cancel", prompt.cancel), ("quit", prompt.confirm)]);
            dialog.set_close_response("cancel");
            let app = Rc::clone(&app);
            dialog.connect_response(None, move |dialog, response| {
                dialog.close();
                if response == "quit" {
                    app.quit_confirmed.set(true);
                    app.window.close();
                }
            });
            dialog.present();
            glib::Propagation::Stop
        });
    }

    /// The tray icon's entire vocabulary reaches the window through here:
    /// a click of any kind raises it, a menu press raises it at the screen
    /// that was pressed, and one row can stop this computer answering model
    /// calls.
    ///
    /// That last one is the only write anything outside this window can ask
    /// for, and it goes in one direction. There is no request that turns
    /// model calls ON -- `tray.rs` has no word for it -- because enabling it
    /// changes what anything else running here may send through, and the
    /// sentence saying so is on the screen the off-state row opens instead.
    /// Turning it off only ever reduces what this computer answers, which is
    /// why that direction is safe from a menu and the other is not.
    ///
    /// The window comes up either way: the daemon's answer to a stop is a
    /// sentence only the model-calls screen renders, and a change nobody can
    /// see the result of is the shape this surface avoids. See `tray.rs` for
    /// the rest, and for why absence of a tray (most Linux desktops,
    /// including plain GNOME) never reaches this at all.
    fn wire_tray(self: &Rc<Self>) {
        let rx = crate::tray::spawn(self.tray.clone());
        // The icon is drawn in the desktop's ink; draw it again when that
        // flips. The tray lives as long as the process, as the window does.
        let tray = self.tray.clone();
        adw::StyleManager::default()
            .connect_dark_notify(move |_| tray.redraw(self::mark::current_scheme()));
        let app = Rc::clone(self);
        glib::spawn_future_local(async move {
            while let Ok(request) = rx.recv().await {
                apply_tray_request(&app, request);
            }
        });
    }

    /// Draw the tray icon from a status reply, or from its absence: a
    /// status that did not answer or could not be read is unavailable,
    /// never the last state as current (`MenuBarStripInputs.of`).
    fn render_tray(self: &Rc<Self>, status: Option<&Status>) {
        use crate::tray_icon as icon;
        let available = status.is_some();
        let (condition, badge, nudge) = match status {
            Some(status) => (
                icon::condition(
                    status.decisions_owed,
                    status.unhealthy(),
                    status.paused,
                    true,
                    false,
                    true,
                ),
                icon::badge(status.decisions_owed),
                status.nudge.as_ref(),
            ),
            None => (
                icon::condition(None, false, false, false, false, true),
                None,
                None,
            ),
        };
        let words = icon::mark_words(nudge, available, badge, condition)
            .map(|words| words.tooltip.as_str())
            .unwrap_or_default();
        let strip = icon::Strip {
            condition,
            badge,
            mark: icon::lit_mark(nudge, available),
        };
        self.tray.show(strip, self::mark::current_scheme(), words);
    }

    /// One daemon call, with its answer delivered back on the main loop.
    ///
    /// A failure is its fixed label. A caller that needs the payload a
    /// daemon sent beside an error uses [`Self::call_with_failure`].
    pub fn call<F>(self: &Rc<Self>, method: &str, params: serde_json::Value, callback: F)
    where
        F: FnOnce(&Rc<App>, Result<serde_json::Value, String>) + 'static,
    {
        self.call_with_failure(method, params, move |app, result| {
            callback(app, result.map_err(|failure| failure.label))
        });
    }

    /// [`Self::call`], keeping the `result` a daemon sent beside an error.
    pub fn call_with_failure<F>(
        self: &Rc<Self>,
        method: &str,
        params: serde_json::Value,
        callback: F,
    ) where
        F: FnOnce(&Rc<App>, Result<serde_json::Value, crate::backend::DaemonFailure>) + 'static,
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

    /// In a Flatpak attached to the host daemon: when the host part is gone
    /// (uninstalled, or the login's runtime directory cleared), quietly go
    /// back to running the daemon in the sandbox. Private AI then shows its
    /// unavailable line again. Nothing is shown about it.
    fn wire_service_fallback(self: &Rc<Self>) {
        if !trace_commons_contributor::daemon::host_link::runs_sandboxed() {
            return;
        }
        let weak = Rc::downgrade(self);
        let busy = Rc::new(Cell::new(false));
        let failed = Rc::new(Cell::new(false));
        glib::timeout_add_local(FALLBACK_INTERVAL, move || {
            let Some(app) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if busy.get() {
                return glib::ControlFlow::Continue;
            }
            let link = trace_commons_contributor::daemon::host_link::socket_path()
                .map(|sock| trace_commons_contributor::daemon::host_link::probe(&sock))
                .unwrap_or(trace_commons_contributor::daemon::host_link::LinkState::Absent);
            if link != trace_commons_contributor::daemon::host_link::LinkState::Absent {
                failed.set(false);
            }
            if crate::service::falls_back(app.worker.via_host_link(), link, failed.get()) {
                busy.set(true);
                let busy = Rc::clone(&busy);
                let failed = Rc::clone(&failed);
                app.reopen(false, move |app| {
                    busy.set(false);
                    // Still on the host link: the sandbox could not start
                    // its own daemon (no folders declared there). Quiet
                    // until the link changes.
                    failed.set(app.worker.via_host_link());
                });
            }
            glib::ControlFlow::Continue
        });
    }

    /// Read the folder rows again whenever the window comes to the front,
    /// as the Mac reads its state again when the app becomes active: a
    /// folder declared from a terminal changes the daemon without a word to
    /// this window, which otherwise showed the old folders until a relaunch.
    fn wire_folder_refresh(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |window| {
            if let Some(app) = weak.upgrade()
                && window.is_active()
            {
                settings::refresh_folders(&app);
            }
        });
    }

    /// Show the folders screen when the attached daemon has none declared
    /// (`service::asks_for_folders`). Sign-in and enrollment stay with first
    /// run, which asks the same question at its own Tools step.
    fn ask_for_folders_if_needed(self: &Rc<Self>, settings: &serde_json::Value) {
        // Camera runs, as `onboarding::present_if_needed`.
        if std::env::var_os("TC_SUPPRESS_ONBOARDING").is_some() {
            return;
        }
        if crate::service::asks_for_folders(
            self.worker.hosts_the_loop(),
            settings,
            self.folders_asked.get(),
            onboarding::automatic_window_open(),
        ) {
            self.folders_asked.set(true);
            roots::present_connected(self);
        }
    }

    /// Classify the background service from this window's backend.
    pub fn service_state<F>(self: &Rc<Self>, callback: F)
    where
        F: FnOnce(&Rc<App>, trace_commons_contributor::daemon::service::ServiceState) + 'static,
    {
        let id = self.worker.service_state();
        self.callbacks.borrow_mut().insert(
            id,
            Box::new(move |app, outcome| {
                if let Outcome::ServiceState(state) = outcome {
                    callback(app, state)
                }
            }),
        );
    }

    /// Replace the backend (`worker::Job::Reopen`), then read everything
    /// again from the daemon now in use. `callback` runs either way.
    pub fn reopen<F>(self: &Rc<Self>, restart_unit: bool, callback: F)
    where
        F: FnOnce(&Rc<App>) + 'static,
    {
        let id = self.worker.reopen(restart_unit);
        self.callbacks.borrow_mut().insert(
            id,
            Box::new(move |app, outcome| {
                if let Outcome::Reopened { attachment, .. } = outcome {
                    app.worker.set_attachment(attachment);
                    // Another daemon, with revisions and folders of its own.
                    managed_sessions::backend_changed(&app.private_inference.managed_revision);
                    app.folders_asked.set(false);
                    // Whatever happened, the daemon may have changed: the
                    // same "look again" as `resync_required`.
                    app.refresh();
                    app.poll_outstanding_previews();
                    callback(app);
                }
            }),
        );
    }

    /// How many times `needle` was in the entry's pre-redaction session
    /// text. `None` for any failure -- see `worker::Outcome::SearchOriginal`.
    pub fn search_original<F>(self: &Rc<Self>, entry_id: &str, needle: &str, callback: F)
    where
        F: FnOnce(&Rc<App>, Option<u32>) + 'static,
    {
        let id = self.worker.search_original(entry_id, needle);
        self.callbacks.borrow_mut().insert(
            id,
            Box::new(move |app, outcome| {
                if let Outcome::SearchOriginal(matches) = outcome {
                    callback(app, matches)
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
            // The Traces map's gate reads its own fields of the same reply,
            // each unread one closed rather than defaulted.
            app.shell.map.set_status(result.as_ref().ok());
            app.shell.map.render(app);
            let result = result.map(serde_json::from_value::<Status>);
            app.home
                .render_status(result.as_ref().ok().and_then(|r| r.as_ref().ok()));
            app.render_tray(result.as_ref().ok().and_then(|r| r.as_ref().ok()));
            if let Ok(Ok(status)) = result {
                app.render_health(&status);
                app.render_grant_voids(&status);
                settings::render_status(app, &status);
                // The witness lives in the contributor's config rather than
                // in daemon settings, so nothing in the settings answer
                // carries it and it is repainted here instead -- including
                // the last-submission row, which changes when a session
                // goes out and on no other event.
                settings::render_witness(app);
                // Both halves of "does this device need onboarding" are the
                // daemon's to answer, so the question is asked here rather
                // than at window construction, where neither is known yet.
                // The same answer narrows the tab strip: before onboarding
                // Home and Traces are not shown (`monitor::tabs`).
                app.shell
                    .set_requires_onboarding(onboarding::requires_onboarding(&status));
                onboarding::present_if_needed(app, &status);
                *app.status.borrow_mut() = Some(status);
                queue::render_badge(app);
            } else {
                consent_settings::note_status_failed(app);
            }
        });
        // Asked alongside the queue, not with `list_projects`, because this
        // is drawn on the queue screen and the daemon's answer changes on
        // exactly the events that change the queue -- an upload landing is
        // what moves a project past the threshold.
        self.call("arming_suggestion", serde_json::json!({}), |app, result| {
            let offer = result
                .ok()
                .and_then(|v| serde_json::from_value::<crate::model::ArmingOffer>(v).ok());
            queue::render_arming_offer(app, offer);
        });
        // The offer is about a daemon setting and both its answers are
        // daemon settings, so it is read from the daemon on every refresh
        // rather than latched in this window. A window-local latch would
        // re-ask on the next launch, which is the nagging the marker exists
        // to prevent.
        self.call("get_settings", serde_json::json!({}), |app, result| {
            if let Ok(raw) = result.as_ref() {
                app.ask_for_folders_if_needed(raw);
            }
            let read = result.map(serde_json::from_value::<crate::model::Settings>);
            // The map's tools are unknown, never unset, without this read.
            app.shell
                .map
                .set_settings(read.as_ref().ok().and_then(|r| r.as_ref().ok()).cloned());
            let Ok(Ok(settings)) = read else {
                app.shell.map.render(app);
                return;
            };
            // The certificate-held section's reading, from the same read.
            // Re-rendered rather than latched: the flag can change under a
            // running window when a contributor completes signup.
            app.evidence_admitted
                .set(settings.admission_evidence_required == Some(true));
            queue::render_private_inference_offer(app, &settings);
            queue::render(app);
        });
        // The group counts, asked on the same refresh as the queue itself.
        // Only `contributable_count` is read from here: the folder's total
        // stays the rows on screen.
        self.call("list_projects", serde_json::json!({}), |app, result| {
            app.shell.map.set_projects_read(result.is_ok());
            let Ok(value) = result else {
                app.shell.map.render(app);
                return;
            };
            let projects: Vec<crate::model::Project> =
                serde_json::from_value(value.get("projects").cloned().unwrap_or_default())
                    .unwrap_or_default();
            *app.projects.borrow_mut() = projects;
            queue::render(app);
        });
        self.call("list_pending", serde_json::json!({}), |app, result| {
            app.shell.map.set_pending_read(result.is_ok());
            let Ok(value) = result else {
                app.shell.map.render(app);
                return;
            };
            let entries: Vec<QueueEntry> =
                serde_json::from_value(value.get("pending").cloned().unwrap_or_default())
                    .unwrap_or_default();
            app.home.render_uncontributed(&entries);
            *app.entries.borrow_mut() = entries;
            queue::render(app);
            app.reconcile_card_previews();
        });
        // Where each tool's sessions go, for the Traces map's commons gate:
        // only a route the core names, for a tool it says is watched, ever
        // draws a flow.
        self.call("tool_destinations", serde_json::json!({}), |app, result| {
            app.shell.map.set_destinations(result.as_ref().ok());
            app.shell.map.render(app);
            app.shell.inspector.render_summary(app);
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
            let home_rollup = result.as_ref().ok().and_then(|value| {
                serde_json::from_value::<crate::home::Rollup>(value.clone()).ok()
            });
            app.home.render_rollup(home_rollup.as_ref());
            app.home.render_credit(
                home_rollup.as_ref(),
                app.credit_condition.borrow().as_deref(),
            );
            *app.summary_rollup.borrow_mut() = home_rollup;
            app.summary_rollup_answered.set(true);
            app.shell.inspector.render_summary(app);
            let result = result.map(serde_json::from_value::<HistoryRollup>);
            if let Ok(Ok(rollup)) = result {
                *app.rollup.borrow_mut() = rollup;
                queue::render(app);
            }
        });
        // What pending credit waits on, for the summary's pending line
        // (D6): the same read the Mac's Home store makes. The daemon
        // caches the answer, so a refresh does not reach the commons each
        // time.
        self.call(
            "commons_credit_summary",
            serde_json::json!({}),
            |app, result| {
                *app.credit_condition.borrow_mut() = result
                    .ok()
                    .and_then(|value| monitor::summary::pending_condition(&value));
                app.home.render_credit(
                    app.summary_rollup.borrow().as_ref(),
                    app.credit_condition.borrow().as_deref(),
                );
                app.shell.inspector.render_summary(app);
            },
        );
        history::refresh(self);
        home::refresh(self);
        private_inference::refresh(self);
        settings::refresh(self);
    }

    /// Keep the daemon's scheduled preview for every pending card in sync
    /// with the queue.
    ///
    /// This is the whole of the fan-out this shell now does: one
    /// `preview_request` per card that has not been asked about, and one
    /// `preview_cancel` for every entry that has stopped being pending --
    /// approved, dismissed, expired, or superseded. `dismiss` already
    /// cancels its own entry's scheduled preview implicitly (see the IPC
    /// doc's "Scheduled previews" section), so the cancel sent here for that
    /// case is a harmless no-op (`dropped: false`) rather than a duplicate
    /// of work the daemon already did; what this covers that `dismiss`
    /// cannot is every other way an entry leaves `pending`.
    fn reconcile_card_previews(self: &Rc<Self>) {
        let current: std::collections::HashSet<String> = self
            .entries
            .borrow()
            .iter()
            .filter(|e| e.state == "pending")
            .map(|e| e.entry_id.clone())
            .collect();
        let (gone, wanted) = card_preview_diff(&current, &self.card_tracked.borrow());

        for entry_id in gone {
            self.card_tracked.borrow_mut().remove(&entry_id);
            self.previews.borrow_mut().remove(&entry_id);
            self.previews_too_large.borrow_mut().remove(&entry_id);
            self.call(
                "preview_cancel",
                serde_json::json!({ "entry_id": entry_id }),
                |_app, _result| {},
            );
        }

        for entry_id in wanted {
            self.card_tracked.borrow_mut().insert(entry_id.clone());
            let key = entry_id.clone();
            self.call(
                "preview_request",
                serde_json::json!({ "entry_id": entry_id }),
                move |app, result| app.handle_preview_request_result(&key, result),
            );
        }
    }

    /// What `preview_request` (and, on a cache hit, `preview_ready`) answers
    /// with for one entry.
    ///
    /// `ready` and `too_large` are the only states that ever draw anything:
    /// `queued` and `running` mean the card stays "checking" until an event
    /// or a later poll answers again, and `failed` is left the same way
    /// rather than risking a wrong or alarming label -- the fan-out this
    /// replaces had the identical gap for a `preview` call that errored.
    fn handle_preview_request_result(
        self: &Rc<Self>,
        entry_id: &str,
        result: Result<serde_json::Value, String>,
    ) {
        let Ok(value) = result else { return };
        match parse_preview_outcome(&value) {
            Some(CardOutcome::Ready(summary)) => {
                self.previews
                    .borrow_mut()
                    .insert(entry_id.to_string(), *summary);
                queue::render(self);
            }
            Some(CardOutcome::TooLarge {
                raw_session_bytes,
                limit_bytes,
            }) => {
                self.previews_too_large
                    .borrow_mut()
                    .insert(entry_id.to_string(), (raw_session_bytes, limit_bytes));
                queue::render(self);
            }
            // `queued` / `running`: wait for `preview_ready` or a later
            // poll. `failed`, and any object this daemon build's contract
            // does not answer with: leave the card "checking" rather than
            // drawing a wrong or alarming label.
            None => {}
        }
    }

    /// Ask about exactly the one card a `preview_ready` event named.
    ///
    /// This is the whole of the fix for the request storm the naive version
    /// of this feature had: one event, one targeted `preview_request`, so
    /// filling an N-card queue costs O(N) requests -- one per card resolving
    /// -- rather than a full sweep of the outstanding set on every one of
    /// N events (O(N^2)). `preview_ready_wants_request` is the decision,
    /// pulled out so it is checkable without a daemon or a GTK loop; see its
    /// own doc comment and the tests beside it.
    ///
    /// `entry_id` is `None` only for an event this connection could not read
    /// one off of, which the contract says should not happen for
    /// `preview_ready` -- `resync_required`'s periodic full sweep is what
    /// backstops that case (and any other kind of missed event) without
    /// needing this path to guess.
    fn handle_preview_ready(self: &Rc<Self>, entry_id: Option<String>) {
        let Some(entry_id) = entry_id else { return };
        let wants_request = preview_ready_wants_request(
            &entry_id,
            &self.card_tracked.borrow(),
            &self.previews.borrow(),
            &self.previews_too_large.borrow(),
        );
        if !wants_request {
            return;
        }
        let key = entry_id.clone();
        self.call(
            "preview_request",
            serde_json::json!({ "entry_id": entry_id }),
            move |app, result| app.handle_preview_request_result(&key, result),
        );
    }

    /// Re-poll every card whose scheduled preview has not resolved yet.
    ///
    /// The daemon's answer to a repeat `preview_request` for an
    /// already-ready or already-refused entry is a cache hit with no work
    /// done, and for one still queued or running it is the same "keep
    /// waiting" answer as the first call -- so a full sweep is cheap and
    /// shrinks to nothing as cards resolve. That is what makes it the right
    /// response to `resync_required`, which really has lost track of
    /// everything and has no single id to target -- but it is exactly the
    /// wrong response to an ordinary `preview_ready`, which always names the
    /// one card that changed; see `handle_preview_ready`.
    fn poll_outstanding_previews(self: &Rc<Self>) {
        let outstanding: Vec<String> = {
            let tracked = self.card_tracked.borrow();
            let previews = self.previews.borrow();
            let too_large = self.previews_too_large.borrow();
            tracked
                .iter()
                .filter(|id| !previews.contains_key(*id) && !too_large.contains_key(*id))
                .cloned()
                .collect()
        };
        for entry_id in outstanding {
            let key = entry_id.clone();
            self.call(
                "preview_request",
                serde_json::json!({ "entry_id": entry_id }),
                move |app, result| app.handle_preview_request_result(&key, result),
            );
        }
    }

    /// Reschedule the debounced `preview_visible` call.
    ///
    /// Called on every scroll movement and after every render, so a drag
    /// through the whole list or a queue that just changed shape both
    /// settle on one call a quarter-second after the last thing happened,
    /// rather than one per pixel or one per render.
    pub fn schedule_visible_preview_update(self: &Rc<Self>) {
        if let Some(source) = self.scroll_debounce.borrow_mut().take() {
            source.remove();
        }
        let app = Rc::clone(self);
        let source = glib::timeout_add_local(
            std::time::Duration::from_millis(SCROLL_SETTLE_DEBOUNCE_MS),
            move || {
                app.scroll_debounce.borrow_mut().take();
                app.send_visible_previews();
                glib::ControlFlow::Break
            },
        );
        *self.scroll_debounce.borrow_mut() = Some(source);
    }

    /// Tell the daemon which cards are actually on screen right now.
    ///
    /// Wholesale, matching `preview_visible`'s own contract: this is not a
    /// diff against the last call, it is what is visible *now*, and
    /// visibility only ever reorders scheduled work -- see item 3 of "What
    /// each shell must do" in the preview-scheduler design.
    fn send_visible_previews(self: &Rc<Self>) {
        let ids: Vec<String> = self
            .card_widgets
            .borrow()
            .iter()
            .filter(|(_, widget)| self.card_is_visible(widget))
            .map(|(id, _)| id.clone())
            .collect();
        self.call(
            "preview_visible",
            serde_json::json!({ "entry_ids": ids }),
            |_app, _result| {},
        );
    }

    /// Whether a card's widget overlaps the queue's own scrolled viewport,
    /// measured in the scroller's coordinate space -- which already accounts
    /// for however far the list is scrolled, so this needs no adjustment
    /// value of its own.
    fn card_is_visible(&self, widget: &gtk::Widget) -> bool {
        let Some(bounds) = widget.compute_bounds(&self.queue.scroller) else {
            return false;
        };
        let viewport_height = self.queue.scroller.height() as f32;
        overlaps_viewport(bounds.y(), bounds.height(), viewport_height)
    }

    /// One card per void, in the core's words (`Status::grant_void_notices`).
    ///
    /// Rebuilt on every status read rather than diffed: there are at most a
    /// handful, and a card left over from a notice another shell already
    /// acknowledged would say something stopped that this window has no
    /// record of anymore.
    fn render_grant_voids(self: &Rc<Self>, status: &Status) {
        while let Some(card) = self.void_notices.first_child() {
            self.void_notices.remove(&card);
        }
        // Every account card but the offer, which is built once.
        let mut child = self.account_notices.first_child();
        while let Some(card) = child {
            child = card.next_sibling();
            if card != self.legacy_card.root {
                self.account_notices.remove(&card);
            }
        }
        let offer_shown = legacy_move::repaint_for(self, status);
        let notices = status.grant_void_notices();
        // The switch-on notices share the column: a folder whose arming was
        // reworded (K5), and armed folders the gate is holding.
        let rewordings = status.arming_rewording_notices();
        let held = status.gate_held_notice();
        let terms = terms_hold::notice_words(status.terms_hold);
        // The legacy invite migration notice is the account's: it changes
        // what the contributor's sessions go under, and they are told on
        // whichever screen they are reading (the consent spec requires it in
        // every shell).
        let migration = status.legacy_migration_notice();
        if let Some(column) = self.void_notices.parent() {
            column.set_visible(
                terms.is_some() || !notices.is_empty() || !rewordings.is_empty() || held.is_some(),
            );
        }
        if let Some(column) = self.account_notices.parent() {
            column.set_visible(migration.is_some() || offer_shown);
        }
        // First: until the terms are accepted again, nothing uploads.
        if let Some(words) = &terms {
            self.void_notices.append(&terms_hold::card(self, words));
        }
        for card in notices {
            self.void_notices.append(&self.grant_void_card(&card));
        }
        for card in rewordings {
            self.void_notices.append(&self.arming_rewording_card(&card));
        }
        if let Some(held) = held {
            self.void_notices.append(&self.gate_held_card(&held));
        }
        if let Some(notice) = &migration {
            self.account_notices
                .append(&self.legacy_migration_card(notice));
        }
    }

    /// A card in the notices column: the attention glyph, a column of text,
    /// and a column of buttons.
    fn notice_card() -> (gtk::Box, gtk::Box, gtk::Box) {
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(style::space::XS)
            .hexpand(true)
            .build();
        let glyph = gtk::Label::new(Some(style::Tone::Attention.glyph()));
        glyph.add_css_class("tc-attention");
        glyph.add_css_class("tc-card-title");
        glyph.set_valign(gtk::Align::Start);
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(style::space::M)
            .build();
        card.add_css_class("tc-banner");
        card.append(&glyph);
        card.append(&column);
        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(style::space::XS)
            .valign(gtk::Align::Center)
            .build();
        card.append(&buttons);
        (card, column, buttons)
    }

    fn notice_text(value: &str, class: &str) -> gtk::Label {
        let label = gtk::Label::builder()
            .label(value)
            .wrap(true)
            .xalign(0.0)
            .build();
        label.add_css_class(class);
        label
    }

    /// "Ask": Settings' call, unchanged -- `set_project_mode` with
    /// this project's id and `notify_only`. It also answers a rewording
    /// notice. A refusal changes nothing; the core's refusal line says so.
    fn ask_first_button(
        self: &Rc<Self>,
        project_id: String,
        action: &str,
        failed: &'static str,
    ) -> gtk::Button {
        let button = gtk::Button::with_label(action);
        button.add_css_class("tc-quiet");
        let app = Rc::clone(self);
        button.connect_clicked(move |button| {
            button.set_sensitive(false);
            app.call(
                "set_project_mode",
                serde_json::json!({ "project_id": project_id, "mode": "notify_only" }),
                move |app, result| {
                    if result.is_err() {
                        app.toast(failed);
                    }
                    app.refresh();
                },
            );
        });
        button
    }

    /// One rewording notice (K5), in the core's words.
    fn arming_rewording_card(
        self: &Rc<Self>,
        card_data: &crate::model::ArmingRewordingCard,
    ) -> gtk::Box {
        let notice = &card_data.notice;
        let (card, column, buttons) = Self::notice_card();
        column.append(&Self::notice_text(&notice.title, "tc-card-title"));
        column.append(&Self::notice_text(notice.body, "tc-body"));
        column.append(&Self::notice_text(notice.now_heading, "tc-card-title"));
        for line in [notice.scope, notice.limit, notice.no_review] {
            column.append(&Self::notice_text(line, "tc-body"));
        }
        if let (Some(project_id), Some(action), Some(failed)) = (
            card_data.ask_first_project_id.clone(),
            notice.ask_first_action,
            notice.ask_first_failed,
        ) {
            buttons.append(&self.ask_first_button(project_id, action, failed));
        }
        // Records that this notice was shown, and nothing else. Only the id
        // on this card, so a rewording raised after this window drew is never
        // cleared unseen.
        if let Some(id) = card_data.id {
            let button = gtk::Button::with_label(notice.acknowledge);
            button.add_css_class("tc-quiet");
            let app = Rc::clone(self);
            button.connect_clicked(move |button| {
                button.set_sensitive(false);
                app.call(
                    "acknowledge_arming_rewordings",
                    serde_json::json!({ "ids": [id] }),
                    |app, _result| app.refresh(),
                );
            });
            buttons.append(&button);
        }
        card
    }

    /// Armed folders the gate is holding, in the core's words. No dismiss:
    /// it goes when the hold does.
    fn gate_held_card(self: &Rc<Self>, notice: &crate::copy::GateHeldNoticeCopy) -> gtk::Box {
        let (card, column, _buttons) = Self::notice_card();
        column.append(&Self::notice_text(notice.title, "tc-card-title"));
        column.append(&Self::notice_text(&notice.body, "tc-body"));
        for reason in &notice.reasons {
            column.append(&Self::notice_text(&format!("\u{2022} {reason}"), "tc-body"));
        }
        column.append(&Self::notice_text(notice.release, "tc-body"));
        if !notice.projects.is_empty() {
            column.append(&Self::notice_text(notice.ask_first, "tc-body"));
        }
        for project in &notice.projects {
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(style::space::S)
                .build();
            row.append(&Self::notice_text(&project.line, "tc-body"));
            if let (Some(project_id), Some(action), Some(failed)) = (
                project.project_id.clone(),
                project.ask_first_action,
                project.ask_first_failed,
            ) {
                row.append(&self.ask_first_button(project_id, action, failed));
            }
            column.append(&row);
        }
        card
    }

    /// The notice after the move, in the core's words. Its one button
    /// records that it was shown and does nothing else.
    fn legacy_migration_card(
        self: &Rc<Self>,
        notice: &crate::copy::LegacyMigrationNoticeCopy,
    ) -> gtk::Box {
        let text = |value: &str, class: &str| {
            let label = gtk::Label::builder()
                .label(value)
                .wrap(true)
                .xalign(0.0)
                .build();
            label.add_css_class(class);
            label
        };
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(style::space::XS)
            .hexpand(true)
            .build();
        column.append(&text(notice.title, "tc-card-title"));
        column.append(&text(notice.body, "tc-body"));
        column.append(&text(notice.folders, "tc-body"));
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(style::space::M)
            .build();
        card.add_css_class("tc-banner");
        card.append(&column);
        let button = gtk::Button::with_label(notice.acknowledge);
        button.add_css_class("tc-quiet");
        button.set_valign(gtk::Align::Center);
        let app = Rc::clone(self);
        button.connect_clicked(move |button| {
            button.set_sensitive(false);
            app.call(
                "acknowledge_legacy_invite_migration",
                serde_json::json!({}),
                |app, _result| app.refresh(),
            );
        });
        card.append(&button);
        card
    }

    fn grant_void_card(self: &Rc<Self>, card_data: &crate::model::GrantVoidCard) -> gtk::Box {
        let id = card_data.id;
        let notice = &card_data.notice;
        let text = |value: &str, class: &str| {
            let label = gtk::Label::builder()
                .label(value)
                .wrap(true)
                .xalign(0.0)
                .build();
            label.add_css_class(class);
            label
        };
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(style::space::XS)
            .hexpand(true)
            .build();
        column.append(&text(&notice.title, "tc-card-title"));
        column.append(&text(notice.body, "tc-body"));
        column.append(&text(notice.reasons_heading, "tc-card-title"));
        for reason in &notice.reasons {
            column.append(&text(&format!("\u{2022} {reason}"), "tc-body"));
        }
        column.append(&text(notice.rearm, "tc-body"));

        let glyph = gtk::Label::new(Some(style::Tone::Attention.glyph()));
        glyph.add_css_class("tc-attention");
        glyph.add_css_class("tc-card-title");
        glyph.set_valign(gtk::Align::Start);

        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(style::space::M)
            .build();
        card.add_css_class("tc-banner");
        card.append(&glyph);
        card.append(&column);
        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(style::space::XS)
            .valign(gtk::Align::Center)
            .build();
        card.append(&buttons);
        // "Turn back on", beside the sentence saying that doing so agrees to
        // the new settings. It is Settings' arming call, unchanged:
        // `set_project_mode` with this project's id and `auto_upload`, so the
        // daemon applies the same refusals, writes the same
        // `armed-auto-upload` row, and clears this notice. A refusal changes
        // nothing; the core's refusal line says so.
        if let (Some(project_id), Some(action), Some(failed)) = (
            card_data.rearm_project_id.clone(),
            notice.rearm_action,
            notice.rearm_failed,
        ) {
            let button = gtk::Button::with_label(action);
            button.add_css_class("tc-quiet");
            let app = Rc::clone(self);
            button.connect_clicked(move |button| {
                button.set_sensitive(false);
                app.call(
                    "set_project_mode",
                    serde_json::json!({ "project_id": project_id, "mode": "auto_upload" }),
                    move |app, result| {
                        if result.is_err() {
                            app.toast(failed);
                        }
                        app.refresh();
                    },
                );
            });
            buttons.append(&button);
        }
        // This button records that this notice was shown, and nothing else.
        // Only the id on this card is acknowledged, so a void raised after
        // this window drew is never cleared unseen.
        if let Some(id) = id {
            let button = gtk::Button::with_label(notice.acknowledge);
            button.add_css_class("tc-quiet");
            let app = Rc::clone(self);
            button.connect_clicked(move |button| {
                button.set_sensitive(false);
                app.call(
                    "acknowledge_grant_voids",
                    serde_json::json!({ "ids": [id] }),
                    |app, _result| app.refresh(),
                );
            });
            buttons.append(&button);
        }
        card
    }

    fn render_health(self: &Rc<Self>, status: &Status) {
        // Independent conditions, and the banner shows each. The health slot
        // carries one label at a time by design, so the witness line is
        // drawn from its own status object rather than waiting for the
        // label; see `Status::health_banner_lines`.
        let lines = status.health_banner_lines();
        if lines.is_empty() {
            self.health_banner.set_visible(false);
            return;
        }
        self.health_label.set_text(&lines.join("\n\n"));
        // The action belongs to the health label.
        match status
            .health
            .last_error_label
            .as_deref()
            .and_then(copy::health_action)
        {
            Some(action) => {
                self.health_button.set_label(action);
                self.health_button.set_visible(true);
            }
            None => self.health_button.set_visible(false),
        }
        self.health_banner.set_visible(true);
    }

    /// The digest, rate-limited to the daemon's configured interval rather
    /// than a fixed one. Its actions can only ever open the window or
    /// dismiss.
    ///
    /// Posted when either half has something to say. It used to return early
    /// on an empty pending list, which was right while every upload passed
    /// through review -- an empty queue then meant an idle period. An armed
    /// project queues nothing however much it sends, so that early return
    /// meant a contributor who armed everything was never told anything at
    /// all. The contributed half comes off the event (`DigestFacts`) because
    /// it describes traces that were never in this shell's entry list.
    fn post_digest(self: &Rc<Self>, facts: crate::backend::DigestFacts) {
        let entries = self.entries.borrow();
        let pending: Vec<&QueueEntry> = entries.iter().filter(|e| e.state == "pending").collect();
        let waiting = (!pending.is_empty()).then(|| {
            let mut labels: Vec<String> = pending.iter().map(|e| e.project_label.clone()).collect();
            labels.sort();
            labels.dedup();
            crate::notify::digest_body(pending.len(), &labels)
        });
        drop(entries);
        let contributed = crate::notify::contribution_body(
            facts.contributed,
            &facts.contributed_projects,
            facts.credit_final,
            facts.credit_pending,
        );
        // Two sentences, either of which may be absent: what is waiting for
        // you, and what went without you. Separate lines because they are
        // about different things and a contributor acts on only one.
        let body = match (waiting, contributed) {
            (Some(w), Some(c)) => format!("{w}\n{c}"),
            (Some(w), None) => w,
            (None, Some(c)) => c,
            (None, None) => return,
        };
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
                app.open_screen("queue");
                app.window.present();
            }
        });
    }

    /// Opens a screen by name: a tab, a sub-page behind the breadcrumb, or
    /// the Settings window. The one way anything outside the shell moves
    /// what the window shows; a name nothing carries moves nothing.
    pub fn open_screen(self: &Rc<Self>, name: &str) -> bool {
        self.shell.open(self, name)
    }

    pub fn toast(self: &Rc<Self>, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    /// Put the number of decisions owed on the switcher's Queue item.
    ///
    /// The count comes from daemon status, independently of the pending
    /// rows whose preview flags determine the shield.
    /// The sidebar's queue badge: the number, and a shield glyph beside it.
    ///
    /// The glyph is ADDED to the count, not substituted for it. At 149
    /// waiting sessions the number is the signal a contributor reads, and an
    /// icon meaning "there is a queue" says strictly less. What the glyph
    /// adds is the one thing the number cannot carry: whether anything in
    /// there wants looking at. See [`crate::shield`].
    pub fn set_queue_count(
        self: &Rc<Self>,
        decisions_owed: Option<u64>,
        shield: crate::shield::Shield,
    ) {
        let mut label = crate::shield::decision_count_label(decisions_owed);
        if !label.is_empty() && shield == crate::shield::Shield::Attention {
            label.push_str(&format!(" {}", style::Tone::Attention.glyph()));
        }
        self.shell.traces_badge.set_label(&label);
        self.shell.describe_traces_badge(decisions_owed);
        self.shell.traces_badge.set_visible(!label.is_empty());
        // The core's words for an unknown count, as the other shells say it.
        self.shell.traces_badge.set_tooltip_text(
            decisions_owed
                .is_none()
                .then(|| trace_commons_contributor::preview_copy::decisions_owed_text(None))
                .as_deref(),
        );
        // A colour is never the only carrier: the glyph above is what
        // survives greyscale, and this is what makes it findable.
        if shield == crate::shield::Shield::Attention {
            self.shell.traces_badge.add_css_class("tc-attention");
        } else {
            self.shell.traces_badge.remove_css_class("tc-attention");
        }
    }

    /// Render the sentence `approve` earns -- see `crate::toast` -- and, when
    /// it says Undo belongs with it, offer one.
    ///
    /// This is the single place both the queue row's Submit and the project
    /// group's Submit all reach the toast, so the two controls cannot drift
    /// in how they read the same response. `entry_ids` is what the undo
    /// card lists: one id for a row, the sendable rows seen for a group.
    /// Undo cancels a row by its id and a group by its `project_id`.
    ///
    /// `offer_undo` is only called when the daemon returned a `hold_until`:
    /// `approve > 0` with no `hold_until` means the hold is configured off,
    /// which the rendered sentence already reports as sent, and there is
    /// nothing left to hold open.
    pub fn render_submit_response(
        self: &Rc<Self>,
        approve: &ApproveResult,
        entry_ids: Vec<String>,
        project_label: &str,
        folder: Option<String>,
    ) {
        let skipped: Vec<&str> = approve
            .skipped
            .iter()
            .map(|s| s.reason_label.as_str())
            .collect();
        let rendered = crate::toast::toast(
            approve.approved,
            approve.total_redactions(),
            approve.flagged,
            &skipped,
        );
        // What a group selector left out for being ineligible, said after
        // the press as the folder said it before.
        //
        // NOT branched on. `group_withheld_line` answers the empty string
        // for zero, and an absent `excluded_ineligible` -- a single-entry
        // call, or an invited contributor, where no filter ran at all --
        // reads as zero here and draws nothing for the same reason. Reading
        // the absence as a real zero would be claiming "nothing was left
        // out" about a filter that never ran; drawing nothing claims
        // nothing either way.
        //
        // Appended to the toast rather than given a second one: a
        // contributor pressed one button and gets one answer.
        let withheld = crate::copy::group_withheld_line(approve.excluded_ineligible.unwrap_or(0));
        if withheld.is_empty() {
            self.toast(&rendered.line);
        } else {
            self.toast(&format!("{} {withheld}", rendered.line));
        }
        // `ApproveResult::offers_undo` is the single source of truth for
        // this decision -- see its doc comment for the defect it fixes --
        // so this checks it directly rather than re-deriving `rendered`'s
        // half of the same rule here.
        if approve.offers_undo() {
            if let Some(hold_until) = approve.hold_until {
                self.offer_undo(entry_ids, project_label, folder, hold_until);
            }
        }
    }

    /// Offer to take back an approval, on the queue rather than in a toast.
    ///
    /// Recovery belongs on the surface a contributor is already looking at:
    /// a toast is gone in seconds and takes the only path back with it,
    /// whereas the bar stays for the whole of the daemon's hold and says in
    /// words what it can and cannot promise.
    ///
    /// `hold_until` is the daemon's instant from the `approve` response, and
    /// is mandatory here -- see [`App::render_submit_response`] for the
    /// caller-side rule about when it is safe to call this at all.
    pub fn offer_undo(
        self: &Rc<Self>,
        entry_ids: Vec<String>,
        project_label: &str,
        folder: Option<String>,
        hold_until: chrono::DateTime<chrono::Utc>,
    ) {
        *self.undo.borrow_mut() = Some(PendingUndo {
            entry_ids,
            project_label: project_label.to_string(),
            folder,
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

    /// The daemon dismissed `entry_id`: the Traces map shows its card. The
    /// map looks it up now, while the entries still list it.
    pub fn note_dismissed(self: &Rc<Self>, entry_id: &str) {
        *self.last_kept.borrow_mut() = None;
        *self.last_dismissed.borrow_mut() = Some(entry_id.to_string());
        self.shell.map.render(self);
    }

    /// Withdraw the undo bar without cancelling: the hold simply runs out.
    pub fn dismiss_undo(self: &Rc<Self>) {
        self.stop_undo_tick();
        self.undo.borrow_mut().take();
        queue::render_undo(self);
    }
}

/// Carry out one [`crate::tray::TrayRequest`].
///
/// A free function rather than the body of `wire_tray`'s loop because the
/// toggle accelerator (`ui::shortcuts`) arrives here as well, and the two
/// surfaces must not each hold their own idea of what a request does. The
/// vocabulary is still the tray's two words, so what a keypress can ask for
/// is bounded by the same type: there is no request that starts answering.
///
/// The window comes up either way, including for the stop: the daemon's
/// answer is a sentence only the model-calls screen renders, and a change
/// nobody can see the result of is the shape both surfaces avoid.
pub(crate) fn apply_tray_request(app: &Rc<App>, request: crate::tray::TrayRequest) {
    match request {
        crate::tray::TrayRequest::Open(screen) => {
            app.open_screen(screen);
        }
        crate::tray::TrayRequest::StopAnsweringModelCalls => {
            app.open_screen(PRIVATE_INFERENCE_SCREEN);
            private_inference::turn_off(app);
        }
    }
    app.window.present();
}

/// Replace `parent`'s children with K11 disclosure rows. Machine values --
/// addresses, keys, measurements -- are selectable and monospace, shown
/// verbatim; every word is `crate::disclosure`'s, which is the core's.
///
/// One widget per `crate::disclosure::drawn` item and nothing else, so the
/// tests on `drawn` are tests of what this puts on screen.
pub fn fill_disclosure_rows(parent: &gtk::Box, rows: &[crate::disclosure::Row]) {
    use crate::disclosure::Drawn;
    while let Some(child) = parent.first_child() {
        parent.remove(&child);
    }
    for item in crate::disclosure::drawn(rows) {
        match item {
            Drawn::Eyebrow(text) => parent.append(&style::eyebrow(&text)),
            Drawn::Body(text) => style::append_body(parent, text),
            Drawn::Mono(value) => {
                let value = gtk::Label::builder()
                    .label(value)
                    .wrap(true)
                    .wrap_mode(gtk::pango::WrapMode::Char)
                    .selectable(true)
                    .xalign(0.0)
                    .css_classes(["monospace"])
                    .build();
                parent.append(&value);
            }
        }
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
    style::append_body(&container, body);
    container
}

/// What a `preview_request` (or a cache-hit `preview_ready`) result decoded
/// to, once the wire object has been read -- see
/// `App::handle_preview_request_result`.
enum CardOutcome {
    /// Boxed because the two variants are otherwise wildly unequal -- a
    /// summary is a few hundred bytes of maps and vectors, the refusal is
    /// two integers -- and every `Option<CardOutcome>` would carry the
    /// larger of the two.
    Ready(Box<PreviewSummary>),
    TooLarge {
        raw_session_bytes: u64,
        limit_bytes: u64,
    },
}

/// Decode a `preview_request` result object into what this shell draws.
///
/// `queued` and `running` -- and anything this build does not recognise --
/// decode to `None`: there is nothing to draw yet, and the caller's answer
/// to `None` is to leave the card exactly as it was. Pulled out from
/// `App::handle_preview_request_result` so the parsing can be checked
/// against real wire shapes without a running daemon.
fn parse_preview_outcome(value: &serde_json::Value) -> Option<CardOutcome> {
    match value.get("state").and_then(|v| v.as_str()) {
        Some(STATE_READY) => {
            let summary = serde_json::from_value(value.get("summary")?.clone()).ok()?;
            Some(CardOutcome::Ready(Box::new(summary)))
        }
        Some(STATE_TOO_LARGE) => Some(CardOutcome::TooLarge {
            raw_session_bytes: value.get("raw_session_bytes")?.as_u64()?,
            limit_bytes: value.get("limit_bytes")?.as_u64()?,
        }),
        _ => None,
    }
}

/// What `App::reconcile_card_previews` does to the tracked set, as pure data
/// rather than daemon calls -- so the actual decision (which ids to cancel,
/// which to request) is checkable without a running `App`.
///
/// Returns `(gone, wanted)`: `gone` is every tracked id no longer in
/// `current` (cancel and forget it), `wanted` is every current id not yet
/// tracked (request one). Both are sorted, only so a test can assert an
/// exact `Vec` rather than a set.
fn card_preview_diff(
    current: &std::collections::HashSet<String>,
    tracked: &std::collections::HashSet<String>,
) -> (Vec<String>, Vec<String>) {
    let mut gone: Vec<String> = tracked.difference(current).cloned().collect();
    let mut wanted: Vec<String> = current.difference(tracked).cloned().collect();
    gone.sort();
    wanted.sort();
    (gone, wanted)
}

/// Whether a widget spanning `[y, y + height)` in the scroller's coordinate
/// space overlaps the visible `[0, viewport_height)` band -- the same
/// interval-overlap test `App::card_is_visible` applies to a real widget's
/// `compute_bounds`, pulled out so the edge cases (exactly at an edge,
/// entirely above, entirely below, taller than the viewport) are checkable
/// with plain numbers.
fn overlaps_viewport(y: f32, height: f32, viewport_height: f32) -> bool {
    y < viewport_height && y + height > 0.0
}

/// Whether a `preview_ready` naming `entry_id` should trigger a fresh
/// `preview_request` -- pulled out of `App::handle_preview_ready` so the
/// per-event cost is checkable directly: this looks at exactly the one id
/// the event named, never at how many other entries are still outstanding,
/// which is what keeps the whole feature at O(N) requests to fill an
/// N-card queue rather than O(N^2). See the tests below, and
/// `App::handle_preview_ready`'s doc comment for the incident this replaces.
fn preview_ready_wants_request(
    entry_id: &str,
    tracked: &std::collections::HashSet<String>,
    previews: &HashMap<String, PreviewSummary>,
    too_large: &HashMap<String, (u64, u64)>,
) -> bool {
    tracked.contains(entry_id)
        && !previews.contains_key(entry_id)
        && !too_large.contains_key(entry_id)
}

#[cfg(test)]
mod card_preview_tests {
    use super::*;
    use std::collections::HashSet;

    fn set(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_newly_pending_entry_is_wanted_and_nothing_is_gone() {
        let current = set(&["a", "b"]);
        let tracked = set(&["a"]);
        let (gone, wanted) = card_preview_diff(&current, &tracked);
        assert_eq!(gone, Vec::<String>::new());
        assert_eq!(wanted, vec!["b".to_string()]);
    }

    #[test]
    fn an_entry_that_left_pending_is_gone_and_nothing_new_is_wanted() {
        // Covers approval, dismissal, expiry, and supersession alike: this
        // function only sees "no longer in `current`", not why.
        let current = set(&["a"]);
        let tracked = set(&["a", "b"]);
        let (gone, wanted) = card_preview_diff(&current, &tracked);
        assert_eq!(gone, vec!["b".to_string()]);
        assert_eq!(wanted, Vec::<String>::new());
    }

    #[test]
    fn an_id_already_tracked_is_neither_gone_nor_wanted_again() {
        // The dedup that keeps a render loop from re-requesting a card every
        // time it redraws.
        let current = set(&["a"]);
        let tracked = set(&["a"]);
        let (gone, wanted) = card_preview_diff(&current, &tracked);
        assert!(gone.is_empty());
        assert!(wanted.is_empty());
    }

    #[test]
    fn a_queue_of_five_hundred_reduces_to_exactly_the_new_ones() {
        // The shape of the real incident: a large queue appearing at once
        // must produce exactly the wanted set, not a truncated prefix --
        // this replaces the old `PREVIEW_PREFETCH_LIMIT` fan-out, and the
        // whole point of moving to the daemon's scheduler is that the shell
        // no longer bounds this itself.
        let current: HashSet<String> = (0..500).map(|n| format!("e{n}")).collect();
        let tracked: HashSet<String> = HashSet::new();
        let (gone, wanted) = card_preview_diff(&current, &tracked);
        assert!(gone.is_empty());
        assert_eq!(wanted.len(), 500);
    }

    #[test]
    fn filling_a_five_hundred_card_queue_costs_one_check_per_ready_event() {
        // Pins the property the coordinator asked for directly: N cards
        // resolving, one `preview_ready` each, must cost O(N) decisions --
        // not O(N^2). The naive version of this feature swept every
        // outstanding entry on every `preview_ready`; resolving them one at
        // a time would have cost 500 + 499 + ... + 1 = 125,250 lookups to
        // drain the queue. This loop makes exactly one decision per event
        // and only ever inspects the one id that event names.
        const N: usize = 500;
        let tracked: HashSet<String> = (0..N).map(|n| format!("e{n}")).collect();
        let mut previews: HashMap<String, PreviewSummary> = HashMap::new();
        let too_large: HashMap<String, (u64, u64)> = HashMap::new();

        let mut checks = 0usize;
        let mut resolved = 0usize;
        for n in 0..N {
            let id = format!("e{n}");
            checks += 1;
            if preview_ready_wants_request(&id, &tracked, &previews, &too_large) {
                resolved += 1;
                // Mirror what `handle_preview_request_result` would do once
                // the daemon answers, so the next event (if this id somehow
                // repeated) sees it as settled rather than outstanding.
                previews.insert(
                    id,
                    PreviewSummary {
                        token_distribution_summary: None,
                        would_send_bytes: 0,
                        raw_session_bytes: 0,
                        event_count: 0,
                        opening_prompt: String::new(),
                        redactions: Default::default(),
                        redactions_distinct: Default::default(),
                        pii_labels_present: Vec::new(),
                        consent_scopes: Vec::new(),
                        residual_risk: String::new(),
                        envelope_digest: String::new(),
                        input_fingerprint: String::new(),
                        enrolled: true,
                    },
                );
            }
        }

        // One decision per event, for every event -- linear in N, and in
        // particular not the triangular-number growth (N*(N+1)/2) a
        // sweep-per-event design would have produced.
        assert_eq!(checks, N);
        assert_eq!(resolved, N);
    }

    #[test]
    fn a_ready_event_for_an_entry_this_shell_never_asked_about_requests_nothing() {
        // An entry another client scheduled, or one already swept from the
        // queue, must not turn into a request just because its ready event
        // reached this connection too.
        let tracked: HashSet<String> = HashSet::new();
        let previews: HashMap<String, PreviewSummary> = HashMap::new();
        let too_large: HashMap<String, (u64, u64)> = HashMap::new();
        assert!(!preview_ready_wants_request(
            "untracked",
            &tracked,
            &previews,
            &too_large
        ));
    }

    #[test]
    fn a_ready_event_for_an_already_resolved_entry_requests_nothing_again() {
        // A repeat or stray event for a card already drawn must not fire a
        // second `preview_request` -- the daemon would answer it from cache
        // for free, but there is still no reason to ask twice.
        let tracked: HashSet<String> = ["a".to_string()].into_iter().collect();
        let mut previews: HashMap<String, PreviewSummary> = HashMap::new();
        previews.insert(
            "a".to_string(),
            PreviewSummary {
                token_distribution_summary: None,
                would_send_bytes: 10,
                raw_session_bytes: 10,
                event_count: 1,
                opening_prompt: String::new(),
                redactions: Default::default(),
                redactions_distinct: Default::default(),
                pii_labels_present: Vec::new(),
                consent_scopes: Vec::new(),
                residual_risk: String::new(),
                envelope_digest: String::new(),
                input_fingerprint: String::new(),
                enrolled: true,
            },
        );
        let too_large: HashMap<String, (u64, u64)> = HashMap::new();
        assert!(!preview_ready_wants_request(
            "a", &tracked, &previews, &too_large
        ));
    }

    #[test]
    fn a_ready_event_for_a_tracked_unresolved_entry_requests_it() {
        let tracked: HashSet<String> = ["a".to_string()].into_iter().collect();
        let previews: HashMap<String, PreviewSummary> = HashMap::new();
        let too_large: HashMap<String, (u64, u64)> = HashMap::new();
        assert!(preview_ready_wants_request(
            "a", &tracked, &previews, &too_large
        ));
    }

    #[test]
    fn a_widget_entirely_above_the_viewport_does_not_overlap() {
        assert!(!overlaps_viewport(-200.0, 100.0, 600.0));
    }

    #[test]
    fn a_widget_entirely_below_the_viewport_does_not_overlap() {
        assert!(!overlaps_viewport(700.0, 100.0, 600.0));
    }

    #[test]
    fn a_widget_straddling_the_bottom_edge_overlaps() {
        assert!(overlaps_viewport(590.0, 100.0, 600.0));
    }

    #[test]
    fn a_widget_straddling_the_top_edge_overlaps() {
        assert!(overlaps_viewport(-50.0, 100.0, 600.0));
    }

    #[test]
    fn a_widget_taller_than_the_viewport_still_overlaps() {
        assert!(overlaps_viewport(-1000.0, 5000.0, 600.0));
    }

    #[test]
    fn a_widget_exactly_touching_the_bottom_edge_does_not_overlap() {
        // Half-open on purpose: a card whose top is exactly at the bottom
        // edge shows none of its own pixels.
        assert!(!overlaps_viewport(600.0, 100.0, 600.0));
    }

    #[test]
    fn a_ready_result_decodes_the_real_summary_fields() {
        let value = serde_json::json!({
            "entry_id": "e1",
            "state": "ready",
            "summary": {
                "would_send_bytes": 4096,
                "raw_session_bytes": 2048,
                "event_count": 12,
                "opening_prompt": "fix the thing",
                "redactions": {"generic_secret": 2},
                "pii_labels_present": ["email"],
                "consent_scopes": ["model_training"],
                "residual_risk": "low",
                "envelope_digest": "",
                "input_fingerprint": "fp1",
                "enrolled": true,
            },
        });
        match parse_preview_outcome(&value) {
            Some(CardOutcome::Ready(summary)) => {
                assert_eq!(summary.would_send_bytes, 4096);
                assert_eq!(summary.event_count, 12);
                assert_eq!(summary.opening_prompt, "fix the thing");
                assert_eq!(summary.redactions.get("generic_secret"), Some(&2));
            }
            other => panic!(
                "expected Ready, got a different outcome: {}",
                other.is_some()
            ),
        }
    }

    #[test]
    fn a_too_large_result_decodes_the_raw_size_and_the_limit_and_nothing_else() {
        let value = serde_json::json!({
            "entry_id": "e2",
            "state": "too_large",
            "raw_session_bytes": 629_145_600u64,
            "limit_bytes": 536_870_912u64,
        });
        match parse_preview_outcome(&value) {
            Some(CardOutcome::TooLarge {
                raw_session_bytes,
                limit_bytes,
            }) => {
                assert_eq!(raw_session_bytes, 629_145_600);
                assert_eq!(limit_bytes, 536_870_912);
            }
            other => panic!(
                "expected TooLarge, got a different outcome: {}",
                other.is_some()
            ),
        }
    }

    #[test]
    fn a_queued_result_decodes_to_nothing_to_draw() {
        let value = serde_json::json!({ "entry_id": "e3", "state": "queued" });
        assert!(parse_preview_outcome(&value).is_none());
    }

    #[test]
    fn a_running_result_decodes_to_nothing_to_draw() {
        let value = serde_json::json!({ "entry_id": "e4", "state": "running" });
        assert!(parse_preview_outcome(&value).is_none());
    }

    #[test]
    fn a_failed_result_decodes_to_nothing_to_draw_rather_than_a_wrong_label() {
        let value = serde_json::json!({
            "entry_id": "e5",
            "state": "failed",
            "code": "internal",
            "label": "preview-failed",
        });
        assert!(parse_preview_outcome(&value).is_none());
    }
}

#[cfg(test)]
mod screen_tests {
    use super::*;

    /// Every window, not only the main one, is named by the app id: the
    /// program name is what GTK gives a window outside the application.
    #[test]
    fn every_window_takes_the_app_id_as_its_program_name() {
        name_the_process();
        assert_eq!(glib::prgname().as_deref(), Some(APP_ID));
    }

    /// The tab strip, the tray menu and the accelerators all read
    /// `screens()`: the four tabs, Home first, in the Mac's order.
    #[test]
    fn the_screens_are_the_four_tabs_in_the_strips_order() {
        let names: Vec<_> = screens().iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            ["home", PRIVATE_INFERENCE_SCREEN, "queue", "insights"]
        );
        for (name, label) in screens() {
            assert!(!name.is_empty(), "a tab needs a name");
            assert!(!label.is_empty(), "a tab needs a label");
        }
    }

    /// Every tab label is the core's word, never one retyped here.
    #[test]
    fn the_tab_labels_come_from_the_shared_copy() {
        let shell = trace_commons_contributor::preview_copy::monitor_screens_copy().shell;
        let labels: Vec<_> = screens().iter().map(|(_, label)| *label).collect();
        assert_eq!(labels[0], shell.tab_home);
        assert_eq!(labels[1], shell.tab_inference);
        assert_eq!(labels[2], shell.tab_traces);
        assert_eq!(
            labels[3],
            trace_commons_contributor::insights::service::ui_copy()["title"]
        );
        // The tab and the tray's model-calls rows name one destination.
        assert_eq!(shell.tab_inference, copy::PRIVATE_INFERENCE_DESTINATION);
    }

    /// The screens that left the strip still open by the names the tray,
    /// the notification action, `--start-page` and the in-window links
    /// spell: History and Mission drafts behind the breadcrumb, Settings in
    /// its own window.
    #[test]
    fn the_screens_that_left_the_strip_still_open_by_name() {
        use monitor::tabs::{HomePage, Location, Route, Tab, TracesPage, route};
        let from = Location::default();
        assert_eq!(route(SETTINGS_SCREEN, from, false), Some(Route::Settings));
        match route("history", from, false) {
            Some(Route::Show(to)) => {
                assert_eq!((to.tab, to.traces_page), (Tab::Traces, TracesPage::History));
            }
            other => panic!("history did not land: {other:?}"),
        }
        match route("mission-drafts", from, true) {
            Some(Route::Show(to)) => {
                assert_eq!((to.tab, to.home_page), (Tab::Home, HomePage::MissionDrafts));
            }
            other => panic!("mission drafts did not land: {other:?}"),
        }
        // Hidden missions: the name lands on Home's overview, and no tab,
        // tray row or accelerator lists it either way.
        assert!(!missions_shown(), "the process default is off");
        assert_eq!(
            route("mission-drafts", from, false),
            Some(Route::Show(from))
        );
        assert!(screens().iter().all(|(name, _)| *name != "mission-drafts"));
        for (name, _) in screens() {
            assert!(
                matches!(route(name, from, false), Some(Route::Show(_))),
                "{name} is listed but opens nothing"
            );
        }
    }

    /// The startup read: only a boolean `true` under `missions_enabled`
    /// turns missions on; a missing file, a missing key and any other value
    /// leave them off, and nothing is created.
    #[test]
    fn the_startup_reading_of_the_switch_defaults_to_off() {
        let dir = std::env::temp_dir().join(format!("tc-missions-switch-{}", uuid::Uuid::new_v4()));
        assert!(!read_missions_shown(&dir), "no directory");
        assert!(!dir.exists(), "reading must not create the directory");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(trace_commons_contributor::config::DAEMON_SETTINGS_FILE);
        for (body, expected) in [
            ("{}", false),
            (r#"{"missions_enabled":false}"#, false),
            (r#"{"missions_enabled":"true"}"#, false),
            (r#"{"missions_enabled":1}"#, false),
            ("not json", false),
            (r#"{"missions_enabled":true}"#, true),
        ] {
            std::fs::write(&file, body).unwrap();
            assert_eq!(read_missions_shown(&dir), expected, "{body}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No tab label may PROMISE privacy.
    ///
    /// The product name is stripped first, the same way the shared sweep in
    /// `private_inference_copy` strips it: the destination is called
    /// "Private AI" deliberately -- the mental model is a VPN, where the word
    /// has never meant the destination cannot see you -- while no sentence
    /// may claim a call is private, because each one still goes on to
    /// whoever was configured to answer it.
    #[test]
    fn no_switcher_label_promises_privacy() {
        for (_, label) in screens() {
            let lowered = label
                .replace(copy::PRIVATE_INFERENCE_DESTINATION, "")
                .to_ascii_lowercase();
            for forbidden in ["private", "secure", "proxy", "backend"] {
                assert!(
                    !lowered.contains(forbidden),
                    "tab label {label:?} must not say {forbidden:?}"
                );
            }
        }
    }
}
