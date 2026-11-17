//! The Monitor window's shell: three panes, and the main pane's rows.
//!
//! The GTK counterpart of `MonitorWindowView` and `MonitorMainPane` in
//! `macos/Sources/TraceCommonsApp/Views/MonitorWindowView.swift`. The main
//! pane, top to bottom:
//!
//! 1. the toolbar row, which is the window's title bar: a
//!    `gtk::WindowHandle` holding the staging tag, the Graph / Map /
//!    Inspector toggles, the Settings gear, and the desktop's own window
//!    controls on the side `gtk-decoration-layout` names;
//! 2. the tab strip;
//! 3. the breadcrumb, on the sub-pages;
//! 4. the tab's screen, with the popover notices laid over it;
//! 5. the graph footer, on Traces.
//!
//! The screens themselves are the existing GTK views, hosted unchanged.
//! Every decision about what is shown is `tabs`', `pane_layout`'s,
//! `demand`'s or `notices`'; this file builds widgets and applies those
//! answers, and authors no word.
//!
//! Deliberate platform differences from the Swift:
//!
//! - The window controls are the desktop's, at the start and the end of the
//!   toolbar row, where the Mac leaves clearance for the traffic lights.
//! - A pane opening or closing asks the compositor for the new width once
//!   (`gtk::Window::set_default_size`) and does not animate. Where the
//!   request is not honoured, and in a maximized or full-screen window, the
//!   panes reallocate inside the width the window has.
//! - The first launch seeds the panes from the display's first monitor: a
//!   Wayland client does not know which monitor its window will open on.
//! - The toolbar toggles and the gear carry themed symbolic icons until the
//!   look slice exports the Mac's glyphs.
//! - Settings' own dialogs and toasts still belong to the main window.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use trace_commons_contributor::preview_copy::monitor_screens_copy;

use super::graph_footer::{GraphData, GraphFooter};
use super::inspector::{Inspector, Selection};
use super::map::TraceMap;
use super::notices::{self, Notice};
use super::pane_layout::{self, Visibility};
use super::prefs::PanePrefs;
use super::tabs::{self, Back, Location, MapView, Route, Tab, TracesPage};
use super::three_pane::ThreePane;
use super::{demand, graph_model};
use crate::ui::{App, style};

/// The existing screens the window hosts, and the notices laid over them.
pub struct Pages<'a> {
    pub home: &'a gtk::Box,
    pub inference: &'a gtk::Box,
    pub traces: &'a gtk::Box,
    pub history: &'a gtk::Box,
    pub insights: &'a gtk::Box,
    /// Not built while missions are hidden.
    pub mission_drafts: Option<&'a gtk::Box>,
    /// Hosted in its own window, never in a tab.
    pub settings: &'a gtk::Box,
    /// Each notice's card, in the order they are stacked.
    pub notices: Vec<(Notice, gtk::Widget)>,
}

struct State {
    /// The selected tab and each tab's page. What is drawn is
    /// `Shell::shown`, which onboarding and the missions switch narrow.
    at: Cell<Location>,
    /// `None` until the core has said.
    requires_onboarding: Cell<Option<bool>>,
    prefs: RefCell<PanePrefs>,
    map_view: Cell<MapView>,
    /// The focus button: the map shows only the selection's tool.
    map_focus: Cell<bool>,
    /// What asked for the inspector last time (`demand`).
    last_demand: RefCell<BTreeSet<String>>,
    /// The opened History row's submission id; empty for none.
    opened_history: RefCell<String>,
    missions: bool,
}

pub struct Shell {
    pub root: ThreePane,
    pub inspector: Inspector,
    pub map: TraceMap,
    pub graph: Rc<GraphFooter>,
    /// The count on the Traces tab: decisions owed.
    pub traces_badge: gtk::Label,
    state: State,
    window: adw::ApplicationWindow,
    app: RefCell<Weak<App>>,
    tabs: Vec<(Tab, gtk::ToggleButton)>,
    inference_dot: gtk::Label,
    breadcrumb: gtk::Box,
    content: gtk::Stack,
    graph_toggle: gtk::ToggleButton,
    map_toggle: gtk::ToggleButton,
    inspector_toggle: gtk::ToggleButton,
    settings_button: gtk::Button,
    inspector_stack: gtk::Stack,
    history_detail: gtk::Box,
    notice_slots: Vec<(Notice, gtk::Box)>,
    settings_root: gtk::Box,
    settings_window: RefCell<Option<adw::Window>>,
    /// Set while this file moves a toggle itself, so the toggle's own
    /// handler does not read that as the person's press.
    syncing: Cell<bool>,
}

/// One toolbar toggle. Its name and tip say what pressing it does, so they
/// are set with its state (`Shell::apply`), not here.
fn toolbar_toggle(icon: &str) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::new();
    button.set_icon_name(icon);
    button.add_css_class("tc-tab");
    button.add_css_class("flat");
    button
}

/// The first monitor's width, for the first launch's seeding.
fn monitor_width() -> Option<f64> {
    let monitor = gtk::gdk::Display::default()?
        .monitors()
        .item(0)?
        .downcast::<gtk::gdk::Monitor>()
        .ok()?;
    Some(f64::from(monitor.geometry().width()))
}

impl Shell {
    pub fn new(window: &adw::ApplicationWindow, pages: Pages<'_>, missions: bool) -> Self {
        let words = monitor_screens_copy();

        // --- The toolbar row, which is the title bar --------------------
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, style::space::S);
        toolbar.set_margin_top(style::space::S);
        toolbar.set_margin_start(style::space::M);
        toolbar.set_margin_end(style::space::M);
        for side in [gtk::PackType::Start, gtk::PackType::End] {
            let controls = gtk::WindowControls::new(side);
            // A side the desktop's layout puts no button on draws nothing.
            controls
                .bind_property("empty", &controls, "visible")
                .invert_boolean()
                .sync_create()
                .build();
            if side == gtk::PackType::Start {
                toolbar.prepend(&controls);
                // A staging build's label, in the core's word; production
                // has none.
                if let Some(label) = trace_commons_contributor::health_copy::window_label() {
                    toolbar.append(&style::tag(label, style::Tone::Neutral));
                }
                let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                spacer.set_hexpand(true);
                toolbar.append(&spacer);
            } else {
                toolbar.append(&controls);
            }
        }
        let graph_toggle = toolbar_toggle("view-statistics-symbolic");
        let map_toggle = toolbar_toggle("mark-location-symbolic");
        let inspector_toggle = toolbar_toggle("sidebar-show-right-symbolic");
        let capsule = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        capsule.add_css_class("tc-switcher");
        capsule.set_valign(gtk::Align::Center);
        for toggle in [&graph_toggle, &map_toggle, &inspector_toggle] {
            capsule.append(toggle);
        }
        let settings_button = gtk::Button::from_icon_name("emblem-system-symbolic");
        settings_button.add_css_class("flat");
        settings_button.add_css_class("circular");
        settings_button.set_valign(gtk::Align::Center);
        settings_button.set_tooltip_text(Some(words.settings_title));
        settings_button.update_property(&[gtk::accessible::Property::Label(words.settings_title)]);
        // Before the end controls, after the spacer.
        let end = toolbar.last_child();
        toolbar.insert_before(&capsule, end.as_ref());
        toolbar.insert_before(&settings_button, end.as_ref());
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&toolbar));

        // --- The tab strip ---------------------------------------------
        let traces_badge = gtk::Label::builder().visible(false).build();
        traces_badge.add_css_class("tc-count-badge");
        traces_badge.set_valign(gtk::Align::Center);
        let inference_dot = gtk::Label::builder().visible(false).build();
        inference_dot.set_valign(gtk::Align::Center);
        let strip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        strip.add_css_class("tc-switcher");
        strip.set_halign(gtk::Align::Start);
        strip.set_margin_start(style::space::M);
        strip.set_margin_end(style::space::M);
        strip.set_margin_top(style::space::S);
        strip.update_property(&[gtk::accessible::Property::Label(words.shell.tabs_label)]);
        let mut tab_buttons: Vec<(Tab, gtk::ToggleButton)> = Vec::new();
        for (index, tab) in Tab::ALL.into_iter().enumerate() {
            let content = gtk::Box::new(gtk::Orientation::Horizontal, style::space::XXS);
            content.append(&gtk::Label::new(Some(tab.title())));
            match tab {
                Tab::Traces => content.append(&traces_badge),
                Tab::Inference => content.append(&inference_dot),
                Tab::Home | Tab::Insights => {}
            }
            let button = gtk::ToggleButton::builder().child(&content).build();
            button.add_css_class("tc-tab");
            button.add_css_class("flat");
            // The chord and nothing else, in GTK's own words.
            button.set_tooltip_text(crate::ui::shortcuts::screen_accel_label(index).as_deref());
            if let Some((_, first)) = tab_buttons.first() {
                button.set_group(Some(first));
            }
            strip.append(&button);
            tab_buttons.push((tab, button));
        }

        // --- The breadcrumb, the screens and the notices ----------------
        let breadcrumb = gtk::Box::new(gtk::Orientation::Horizontal, style::space::XS);
        breadcrumb.set_margin_start(style::space::M);
        breadcrumb.set_margin_end(style::space::M);
        breadcrumb.set_margin_top(style::space::S);
        breadcrumb.set_visible(false);

        let content = gtk::Stack::new();
        content.set_vexpand(true);
        content.set_hexpand(true);
        content.add_named(pages.home, Some("home"));
        content.add_named(pages.inference, Some("private-inference"));
        content.add_named(pages.traces, Some("queue"));
        content.add_named(pages.history, Some("history"));
        content.add_named(pages.insights, Some("insights"));
        if let Some(drafts) = pages.mission_drafts {
            content.add_named(drafts, Some("mission-drafts"));
        }
        // The screens were laid out for a wider column than the main
        // pane's 440. One that cannot get narrower scrolls sideways inside
        // the pane rather than pushing the pane past its width.
        let content_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&content)
            .build();

        // The notices float over the top of the tab and push nothing down.
        // The column is never taller than the tab's own area: past that it
        // scrolls, so a card's Undo or acknowledge button can always be
        // reached. With no card showing it is hidden, and takes no room and
        // no click from the screen under it.
        let notice_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let notice_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .valign(gtk::Align::Start)
            .visible(false)
            .child(&notice_column)
            .build();
        let mut notice_slots = Vec::new();
        for (notice, card) in pages.notices {
            let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
            slot.append(&card);
            notice_column.append(&slot);
            notice_slots.push((notice, slot, card));
        }
        // Each card shows and hides itself; the column follows them.
        let follow = {
            let scroller = notice_scroller.downgrade();
            let cards: Vec<(
                gtk::glib::WeakRef<gtk::Box>,
                gtk::glib::WeakRef<gtk::Widget>,
            )> = notice_slots
                .iter()
                .map(|(_, slot, card)| (slot.downgrade(), card.downgrade()))
                .collect();
            Rc::new(move || {
                let Some(scroller) = scroller.upgrade() else {
                    return;
                };
                // Each widget's own flag, not `is_visible`: that one asks
                // the ancestors too, and the column starts hidden, so no
                // card would ever look shown and it would never open.
                scroller.set_visible(cards.iter().any(|(slot, card)| {
                    slot.upgrade().is_some_and(|slot| slot.get_visible())
                        && card.upgrade().is_some_and(|card| card.get_visible())
                }));
            })
        };
        for (_, slot, card) in &notice_slots {
            for widget in [slot.upcast_ref::<gtk::Widget>(), card] {
                let follow = follow.clone();
                widget.connect_visible_notify(move |_| follow());
            }
        }
        follow();
        let notice_slots: Vec<(Notice, gtk::Box)> = notice_slots
            .into_iter()
            .map(|(notice, slot, _)| (notice, slot))
            .collect();
        let over_content = gtk::Overlay::new();
        over_content.set_child(Some(&content_scroller));
        over_content.add_overlay(&notice_scroller);
        over_content.set_vexpand(true);

        let graph = GraphFooter::new();

        let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main.add_css_class("tc-root");
        main.append(&handle);
        main.append(&strip);
        main.append(&breadcrumb);
        main.append(&over_content);
        main.append(&graph.root);

        // --- The map pane ----------------------------------------------
        let map = TraceMap::new();
        map.root.set_vexpand(true);
        let map_pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
        map_pane.add_css_class("tc-root");
        map_pane.append(&map.root);

        // --- The inspector pane ----------------------------------------
        let inspector = Inspector::new();
        let history_detail = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(style::space::M)
            .margin_top(style::space::M)
            .margin_bottom(style::space::M)
            .margin_start(style::space::M)
            .margin_end(style::space::M)
            .build();
        let history_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&history_detail)
            .build();
        let inspector_stack = gtk::Stack::new();
        inspector_stack.set_vexpand(true);
        inspector_stack.add_named(&inspector.root, Some("traces"));
        inspector_stack.add_named(&history_scroller, Some("history"));
        let inspector_pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
        inspector_pane.add_css_class("tc-root");
        inspector_pane.append(&inspector_stack);

        let root = ThreePane::new(&main, &map_pane, &inspector_pane);

        // --- The person's kept choices ---------------------------------
        let mut prefs = PanePrefs::load();
        // Seeded once, before anything is shown, from the room the monitor
        // has. A demand cannot be there yet: nothing has been read.
        if let Some(width) = monitor_width()
            && prefs.seed(width, false)
        {
            prefs.save();
        }
        root.set_kept_widths(prefs.main_width, prefs.map_width);

        let shell = Self {
            root,
            inspector,
            map,
            graph,
            traces_badge,
            state: State {
                at: Cell::new(Location::default()),
                requires_onboarding: Cell::new(None),
                prefs: RefCell::new(prefs),
                map_view: Cell::new(MapView::default()),
                map_focus: Cell::new(false),
                last_demand: RefCell::new(BTreeSet::new()),
                opened_history: RefCell::new(String::new()),
                missions,
            },
            window: window.clone(),
            app: RefCell::new(Weak::new()),
            tabs: tab_buttons,
            inference_dot,
            breadcrumb,
            content,
            graph_toggle,
            map_toggle,
            inspector_toggle,
            settings_button,
            inspector_stack,
            history_detail,
            notice_slots,
            settings_root: pages.settings.clone(),
            settings_window: RefCell::new(None),
            syncing: Cell::new(false),
        };
        shell.apply();
        // The window opens at its panes' widths, the map at its kept one.
        let (_, natural, _, _) = shell.root.measure(gtk::Orientation::Horizontal, -1);
        window.set_default_size(natural, pane_layout::WINDOW_HEIGHT as i32);
        shell
    }

    // --- What is shown ---------------------------------------------------

    /// The tab the person selected, whatever onboarding lets be drawn.
    pub fn selected_tab(&self) -> Tab {
        self.state.at.get().tab
    }

    /// What is on screen: the selected location, narrowed by onboarding
    /// and by the missions switch.
    pub fn shown(&self) -> Location {
        let at = self.state.at.get();
        Location {
            tab: tabs::shown_tab(at.tab, self.state.requires_onboarding.get()),
            home_page: at.home_page.shown(self.state.missions),
            traces_page: at.traces_page,
        }
    }

    /// The map pane's view. The Private AI view is a later slice; the pane
    /// draws the Traces placeholder for either.
    pub fn map_view(&self) -> MapView {
        self.state.map_view.get()
    }

    /// The tool the map is focused on, when the focus button is on.
    pub fn map_focus_tool(&self, app: &Rc<App>) -> Option<String> {
        self.state
            .map_focus
            .get()
            .then(|| self.selected_tool(app).map(|(id, _)| id))
            .flatten()
    }

    fn pane_visibility(&self) -> Visibility {
        let prefs = self.state.prefs.borrow();
        Visibility::new(
            prefs.shows_map,
            prefs.shows_inspector && tabs::inspector_available(self.shown(), false),
        )
    }

    /// Whether the core says onboarding is still owed. Before it has said,
    /// every tab is shown, as this window has always opened.
    pub fn set_requires_onboarding(&self, requires: bool) {
        if self.state.requires_onboarding.replace(Some(requires)) != Some(requires) {
            self.apply();
        }
    }

    // --- Navigation ------------------------------------------------------

    /// Opens a screen by the name the tray, the accelerators, the
    /// notification action, `--start-page` and the in-window links use.
    /// False for a name nothing carries, and then nothing moves.
    pub fn open(&self, app: &Rc<App>, name: &str) -> bool {
        match tabs::route(name, self.state.at.get(), self.state.missions) {
            Some(Route::Show(to)) => {
                self.go(to);
                true
            }
            Some(Route::Settings) => {
                self.present_settings(app);
                true
            }
            None => false,
        }
    }

    fn go(&self, to: Location) {
        let from = self.state.at.replace(to);
        if from.tab != to.tab
            && let Some(view) = tabs::map_view_for(to.tab)
        {
            // Picking Inference shows the map's Private AI view, picking
            // Traces its Traces view; Home and Insights keep the choice.
            self.state.map_view.set(view);
        }
        self.apply();
    }

    fn pick_tab(&self, tab: Tab) {
        let mut to = self.state.at.get();
        to.tab = tab;
        self.go(to);
    }

    fn go_back(&self, back: Back) {
        let mut to = self.state.at.get();
        match back {
            Back::Home => to.home_page = tabs::HomePage::Overview,
            Back::Traces => to.traces_page = TracesPage::Tree,
        }
        self.go(to);
    }

    /// Settings in its own window (owner, 2026-10-08 on the Mac), holding
    /// the existing Settings view unchanged. Built once and hidden on
    /// close: the view is the app's for the life of the process.
    pub fn present_settings(&self, app: &Rc<App>) {
        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.present();
            return;
        }
        let words = monitor_screens_copy();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.add_css_class("tc-root");
        // A window of its own needs the desktop's controls; the rule
        // against a header bar is the main window's.
        column.append(
            &adw::HeaderBar::builder()
                .title_widget(&adw::WindowTitle::new(words.settings_title, ""))
                .build(),
        );
        self.settings_root.set_vexpand(true);
        column.append(&self.settings_root);
        let window = adw::Window::builder()
            .title(words.settings_title)
            .default_width(780)
            .default_height(640)
            .hide_on_close(true)
            .content(&column)
            .build();
        if let Some(application) = app.window.application() {
            window.set_application(Some(&application));
        }
        // A hidden Settings window must not keep the process alive after
        // the main window has gone.
        let weak = window.downgrade();
        app.window.connect_destroy(move |_| {
            if let Some(window) = weak.upgrade() {
                window.destroy();
            }
        });
        window.present();
        self.settings_window.replace(Some(window));
    }

    // --- Panes -----------------------------------------------------------

    fn update_prefs(&self, change: impl FnOnce(&mut PanePrefs)) {
        let prefs = {
            let mut prefs = self.state.prefs.borrow_mut();
            let before = *prefs;
            change(&mut prefs);
            if *prefs == before {
                return;
            }
            *prefs
        };
        prefs.save();
    }

    pub fn set_inspector_shown(&self, _app: &Rc<App>, shown: bool) {
        self.update_prefs(|prefs| prefs.shows_inspector = shown);
        self.apply();
    }

    pub fn set_map_shown(&self, shown: bool) {
        self.update_prefs(|prefs| prefs.shows_map = shown);
        self.apply();
    }

    /// Asks the compositor for the width that keeps every other pane where
    /// it is. A maximized or full-screen window has no edge to move, and a
    /// compositor may ignore the request; either way the panes are laid
    /// out for the width the window really has.
    fn resize_window(&self, wanted: f64) {
        if self.window.is_maximized() || self.window.is_fullscreen() {
            return;
        }
        let current = self.root.laid_out_width();
        if current <= 0.0 || self.window.width() <= 0 {
            return;
        }
        let width = f64::from(self.window.width()) + (wanted - current);
        self.window
            .set_default_size(width.round() as i32, self.window.height());
    }

    // --- Selection and the inspector -------------------------------------

    /// A selection made in the Traces list. A session is a demand on the
    /// inspector: selecting one shows the pane its card lives in. A folder,
    /// or nothing, moves the selection only. The inspector never closes
    /// itself.
    pub fn select(&self, app: &Rc<App>, selection: Selection) {
        let session = matches!(selection, Selection::Session(_));
        self.inspector.select(app, selection);
        self.map
            .highlight(&self.inspector.selection(), &app.entries.borrow());
        if session {
            self.update_prefs(|prefs| prefs.shows_inspector = true);
        }
        self.refresh_demand(app);
        self.render_graph(app);
        self.apply();
    }

    /// A folder clicked on the map is the selection and the inspector's
    /// subject, on Traces.
    pub fn select_from_map(&self, app: &Rc<App>, project_id: String) {
        let mut to = self.state.at.get();
        to.tab = Tab::Traces;
        to.traces_page = TracesPage::Tree;
        self.state.at.set(to);
        self.update_prefs(|prefs| prefs.shows_inspector = true);
        self.select(app, Selection::Folder(project_id));
    }

    /// Opening a History row: its details are the inspector's, so the
    /// inspector opens. Clearing it closes nothing.
    pub fn open_history_row(&self, app: &Rc<App>, submission_id: &str) {
        self.state.opened_history.replace(submission_id.to_string());
        if !submission_id.is_empty() {
            self.update_prefs(|prefs| prefs.shows_inspector = true);
        }
        self.render_history_detail(app);
        self.apply();
    }

    fn render_history_detail(&self, app: &Rc<App>) {
        while let Some(child) = self.history_detail.first_child() {
            self.history_detail.remove(&child);
        }
        let opened = self.state.opened_history.borrow().clone();
        if opened.is_empty() {
            return;
        }
        let records = app.history_records.borrow();
        match records
            .as_ref()
            .and_then(|records| records.iter().find(|r| r.submission_id == opened))
        {
            Some(record) => self
                .history_detail
                .append(&crate::ui::history::record_detail(record)),
            // A row that has gone is the Traces selection again, never a
            // stale card.
            None if records.is_some() => self.state.opened_history.borrow_mut().clear(),
            None => {}
        }
    }

    /// What is asking for the inspector right now. Fed from what this
    /// shell holds: an approval's undo window (a row's Submit or a folder's
    /// Submit all, which this shell offers as one undo) and a selected
    /// session. The Mac's `contributed`, `kept`, `folder-undo` and
    /// `folder-submit` keys have no state behind them here yet.
    fn demand_keys(&self, app: &Rc<App>) -> BTreeSet<String> {
        let undo = app.undo.borrow();
        let selection = self.inspector.selection();
        demand::keys(&demand::Asking {
            approval_undo: undo.as_ref().map(|undo| undo.entry_ids.as_slice()),
            approval_at: undo.as_ref().map(|undo| undo.approved_at.timestamp()),
            review: match &selection {
                Selection::Session(id) => Some(id.as_str()),
                Selection::Folder(_) | Selection::None => None,
            },
            ..demand::Asking::default()
        })
    }

    /// Opens the inspector when something new asks for it. A key going
    /// away, or the same thing still asking, leaves the person's choice.
    pub fn refresh_demand(&self, app: &Rc<App>) {
        let current = self.demand_keys(app);
        let opens = demand::opens(&self.state.last_demand.borrow(), &current);
        self.state.last_demand.replace(current);
        if opens {
            self.update_prefs(|prefs| prefs.shows_inspector = true);
            self.apply();
        }
    }

    // --- Rendering from the app's reads ----------------------------------

    /// Redraws what the window level draws from the queue: the inspector's
    /// card, the map and the graph. Called with the queue's own render.
    pub fn render(&self, app: &Rc<App>) {
        self.inspector.render(app);
        self.map.render(app);
        self.render_history_detail(app);
        self.render_graph(app);
        self.refresh_demand(app);
        self.apply();
    }

    /// The selection's tool, as `(source, display name)`: a session's own,
    /// or the tool most of a folder's waiting sessions came from.
    fn selected_tool(&self, app: &Rc<App>) -> Option<(String, String)> {
        let entries = app.entries.borrow();
        let pending = entries.iter().filter(|entry| entry.state == "pending");
        match self.inspector.selection() {
            Selection::Session(id) => pending
                .into_iter()
                .find(|entry| entry.entry_id == id)
                .map(|entry| (entry.source.clone(), entry.agent_label().to_string())),
            Selection::Folder(id) => {
                let mut counts: Vec<(String, String, usize)> = Vec::new();
                for entry in pending.filter(|entry| entry.project_id == id) {
                    match counts
                        .iter_mut()
                        .find(|(source, _, _)| *source == entry.source)
                    {
                        Some(found) => found.2 += 1,
                        None => {
                            counts.push((entry.source.clone(), entry.agent_label().to_string(), 1))
                        }
                    }
                }
                // The first of equals, so the answer does not flicker.
                let most = counts.iter().map(|(_, _, count)| *count).max()?;
                counts
                    .into_iter()
                    .find(|(_, _, count)| *count == most)
                    .map(|(source, label, _)| (source, label))
            }
            Selection::None => None,
        }
    }

    pub fn render_graph(&self, app: &Rc<App>) {
        let tool = self.selected_tool(app);
        let source = tool.as_ref().map(|(source, _)| source.as_str());
        let records = app.history_records.borrow();
        let shared = graph_model::shared_times(
            records.iter().flatten().map(|record| {
                (
                    record.status.as_str(),
                    record.source.as_deref(),
                    record.submitted_at,
                )
            }),
            source,
        );
        let entries = app.entries.borrow();
        let kept = graph_model::kept_times(
            entries
                .iter()
                .filter(|entry| entry.state == "pending")
                .map(|entry| (entry.source.as_str(), entry.discovered_at)),
            source,
        );
        if tool.is_none() {
            // Nothing to focus on is not focused.
            self.state.map_focus.set(false);
        }
        self.graph.render(GraphData {
            shared,
            kept,
            history_known: records.is_some(),
            tool: tool.map(|(_, label)| label),
        });
        self.graph.set_focused(self.state.map_focus.get());
    }

    /// The Inference tab's dot: what the listener is doing, from the
    /// daemon's own report and the core's table, never the switch. An
    /// unreported state is no dot: unknown is neither on nor off.
    pub fn render_inference_state(&self, state: &str) {
        let dot = &self.inference_dot;
        for tone in [
            style::Tone::Neutral,
            style::Tone::Clear,
            style::Tone::Attention,
            style::Tone::Held,
            style::Tone::Refused,
        ] {
            dot.remove_css_class(tone.css());
        }
        if state.is_empty() {
            dot.set_visible(false);
            return;
        }
        let tone = crate::ui::private_inference::indicator_tone(
            crate::copy::private_inference_state_tone(state),
        );
        let line = crate::copy::private_inference_state_line(state);
        dot.set_label(tone.glyph());
        dot.add_css_class(tone.css());
        dot.set_tooltip_text(Some(line));
        dot.update_property(&[gtk::accessible::Property::Label(line)]);
        dot.set_visible(true);
    }

    // --- Applying the state to the widgets --------------------------------

    fn set_toggle(
        &self,
        toggle: &gtk::ToggleButton,
        shown: bool,
        available: bool,
        hide: &str,
        show: &str,
    ) {
        if toggle.is_active() != shown {
            toggle.set_active(shown);
        }
        toggle.set_sensitive(available);
        let name = if shown { hide } else { show };
        toggle.set_tooltip_text(Some(name));
        toggle.update_property(&[gtk::accessible::Property::Label(name)]);
    }

    fn render_breadcrumb(&self, at: Location) {
        while let Some(child) = self.breadcrumb.first_child() {
            self.breadcrumb.remove(&child);
        }
        let Some(crumbs) = tabs::breadcrumb(at, self.state.missions) else {
            self.breadcrumb.set_visible(false);
            return;
        };
        let go_back = |back: Back| {
            let weak = self.app.borrow().clone();
            move || {
                if let Some(app) = weak.upgrade() {
                    app.shell.go_back(back);
                }
            }
        };
        if let Some(back) = crumbs.first().and_then(|crumb| crumb.back) {
            let button = gtk::Button::from_icon_name("go-previous-symbolic");
            button.add_css_class("flat");
            let name = tabs::back_label(&crumbs);
            button.set_tooltip_text(Some(name));
            button.update_property(&[gtk::accessible::Property::Label(name)]);
            let action = go_back(back);
            button.connect_clicked(move |_| action());
            self.breadcrumb.append(&button);
        }
        for (index, crumb) in crumbs.iter().enumerate() {
            if index > 0 {
                let separator = gtk::Label::new(Some("\u{203a}"));
                separator.add_css_class("dim-label");
                separator.set_accessible_role(gtk::AccessibleRole::Presentation);
                self.breadcrumb.append(&separator);
            }
            match crumb.back {
                Some(back) => {
                    let button = gtk::Button::with_label(crumb.title);
                    button.add_css_class("flat");
                    let action = go_back(back);
                    button.connect_clicked(move |_| action());
                    self.breadcrumb.append(&button);
                }
                None => {
                    let label = gtk::Label::new(Some(crumb.title));
                    label.add_css_class("heading");
                    self.breadcrumb.append(&label);
                }
            }
        }
        self.breadcrumb.set_visible(true);
    }

    /// Draws everything this file owns from the state. Idempotent, and the
    /// only place a pane, a tab or a toggle is moved.
    fn apply(&self) {
        self.syncing.set(true);
        let words = monitor_screens_copy();
        let at = self.shown();
        let shown_tabs = Tab::shown(self.state.requires_onboarding.get());
        for (tab, button) in &self.tabs {
            button.set_visible(shown_tabs.contains(tab));
            if *tab == at.tab && !button.is_active() {
                button.set_active(true);
            }
        }
        self.content
            .set_visible_child_name(tabs::content_page(at, self.state.missions));
        self.render_breadcrumb(at);

        let prefs = *self.state.prefs.borrow();
        // The graph is the Traces tab's: elsewhere its toggle is disabled
        // and reads as hidden, as the inspector's does where it cannot open.
        let graph_available = tabs::graph_available(at.tab);
        let graph_shown = prefs.shows_graph && graph_available;
        self.set_toggle(
            &self.graph_toggle,
            graph_shown,
            graph_available,
            words.shell.hide_graph,
            words.shell.show_graph,
        );
        self.graph.root.set_reveal_child(graph_shown);
        self.set_toggle(
            &self.map_toggle,
            prefs.shows_map,
            true,
            words.shell.hide_map,
            words.shell.show_map,
        );
        let inspector_available = tabs::inspector_available(at, false);
        self.set_toggle(
            &self.inspector_toggle,
            prefs.shows_inspector && inspector_available,
            inspector_available,
            words.shell.hide_inspector,
            words.shell.show_inspector,
        );

        // An opened History row is the inspector's subject while History
        // is shown; otherwise the Traces selection's card.
        let history_row = tabs::shows_history(at) && !self.state.opened_history.borrow().is_empty();
        self.inspector_stack
            .set_visible_child_name(if history_row { "history" } else { "traces" });

        let place = tabs::place(at);
        for (notice, slot) in &self.notice_slots {
            slot.set_visible(notices::shown(*notice, place));
        }

        if let Some(wanted) = self.root.set_visibility(self.pane_visibility()) {
            self.resize_window(wanted);
        }
        self.syncing.set(false);
    }
}

/// Connects the shell's own controls. After `App` exists, because every
/// handler reaches the shell through it.
pub fn wire(app: &Rc<App>) {
    let shell = &app.shell;
    shell.app.replace(Rc::downgrade(app));

    for (tab, button) in &shell.tabs {
        let weak = Rc::downgrade(app);
        let tab = *tab;
        button.connect_toggled(move |button| {
            let Some(app) = weak.upgrade() else { return };
            if button.is_active() && !app.shell.syncing.get() {
                app.shell.pick_tab(tab);
            }
        });
    }

    let weak = Rc::downgrade(app);
    shell.graph_toggle.connect_toggled(move |toggle| {
        let Some(app) = weak.upgrade() else { return };
        if !app.shell.syncing.get() {
            let shown = toggle.is_active();
            app.shell.update_prefs(|prefs| prefs.shows_graph = shown);
            app.shell.apply();
        }
    });
    let weak = Rc::downgrade(app);
    shell.map_toggle.connect_toggled(move |toggle| {
        let Some(app) = weak.upgrade() else { return };
        if !app.shell.syncing.get() {
            app.shell.set_map_shown(toggle.is_active());
        }
    });
    let weak = Rc::downgrade(app);
    shell.inspector_toggle.connect_toggled(move |toggle| {
        let Some(app) = weak.upgrade() else { return };
        if !app.shell.syncing.get() {
            app.shell.set_inspector_shown(&app, toggle.is_active());
        }
    });
    let weak = Rc::downgrade(app);
    shell.settings_button.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            app.shell.present_settings(&app);
        }
    });

    // The focus button: focus the map on the selection's tool, or back to
    // the whole map. A hidden map is shown, on its Traces view.
    let weak = Rc::downgrade(app);
    shell.graph.focus.connect_toggled(move |toggle| {
        let Some(app) = weak.upgrade() else { return };
        let shell = &app.shell;
        if toggle.is_active() == shell.state.map_focus.get() {
            // The footer following the state, not a press.
            return;
        }
        shell.state.map_focus.set(toggle.is_active());
        shell.state.map_view.set(MapView::Traces);
        shell.graph.set_focused(toggle.is_active());
        shell.set_map_shown(true);
    });

    // The widths the person leaves the panes at, for the next launch.
    let weak = Rc::downgrade(app);
    shell.root.connect_map_width(move |width| {
        if let Some(app) = weak.upgrade() {
            app.shell.state.prefs.borrow_mut().map_width = width;
        }
    });
    let weak = Rc::downgrade(app);
    shell.root.connect_main_width(move |width| {
        if let Some(app) = weak.upgrade() {
            app.shell.update_prefs(|prefs| prefs.main_width = width);
        }
    });
    // A resize writes nothing per frame; the map's width is saved when the
    // window closes, with the rest.
    let weak = Rc::downgrade(app);
    app.window.connect_close_request(move |_| {
        if let Some(app) = weak.upgrade() {
            app.shell.state.prefs.borrow().save();
        }
        gtk::glib::Propagation::Proceed
    });

    // A demand already there when the window appears opens the inspector
    // too, over a restored closed one.
    let keys = shell.demand_keys(app);
    if demand::opens_on_appear(&keys) {
        shell.update_prefs(|prefs| prefs.shows_inspector = true);
    }
    shell.state.last_demand.replace(keys);
    shell.apply();
}

#[cfg(test)]
mod tests {
    /// The window has no header bar: the toolbar row is the title bar, a
    /// window handle carrying the desktop's own controls on both sides.
    ///
    /// Scanned, because a header bar put back would pass every behavioural
    /// test here. Only this module's body is read, never its tests.
    #[test]
    fn the_title_bar_is_a_window_handle_with_the_desktops_controls() {
        let code = include_str!("shell.rs")
            .split("\n#[cfg(test)]")
            .next()
            .expect("the module has a body before its tests");
        assert!(code.contains("gtk::WindowHandle::new()"));
        assert!(code.contains("gtk::WindowControls::new(side)"));
        assert!(code.contains("[gtk::PackType::Start, gtk::PackType::End]"));
        // The one header bar in this file is the Settings window's own.
        assert_eq!(code.matches("adw::HeaderBar").count(), 1);
        let settings = code
            .split("pub fn present_settings")
            .nth(1)
            .expect("Settings has its own window");
        assert!(
            settings
                .split("\n    }\n")
                .next()
                .unwrap()
                .contains("adw::HeaderBar"),
            "the header bar is outside the Settings window"
        );
    }

    /// The main window builds no header bar either.
    #[test]
    fn the_main_window_builds_no_header_bar() {
        let code = include_str!("../mod.rs")
            .split("\n#[cfg(test)]")
            .next()
            .expect("the module has a body before its tests");
        assert!(
            !code.contains("adw::HeaderBar::"),
            "ui/mod.rs builds a header bar; the main pane's toolbar row is the title bar"
        );
    }

    /// The toolbar's three toggles and the gear take every word from the
    /// core's shell table, by state.
    #[test]
    fn the_toolbar_names_come_from_the_shell_table() {
        let code = include_str!("shell.rs")
            .split("\n#[cfg(test)]")
            .next()
            .expect("the module has a body before its tests");
        for word in [
            "words.shell.hide_graph",
            "words.shell.show_graph",
            "words.shell.hide_map",
            "words.shell.show_map",
            "words.shell.hide_inspector",
            "words.shell.show_inspector",
            "words.settings_title",
            "words.shell.tabs_label",
        ] {
            assert!(code.contains(word), "the toolbar does not read {word}");
        }
    }

    // --- Mounted: the widgets, on a display ------------------------------

    use super::*;

    fn pump() {
        let context = gtk::glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
    }

    struct Mounted {
        window: adw::ApplicationWindow,
        shell: Shell,
        notices: Vec<(Notice, gtk::Box)>,
    }

    /// The shell around empty pages, with every pane open. No daemon and no
    /// `App`: the shell's own rules need neither.
    fn mounted(missions: bool) -> Mounted {
        adw::init().expect("GTK display");
        let window = adw::ApplicationWindow::builder().build();
        let page = || gtk::Box::new(gtk::Orientation::Vertical, 0);
        let (home, inference, traces, history, insights, drafts, settings) =
            (page(), page(), page(), page(), page(), page(), page());
        let notices: Vec<(Notice, gtk::Box)> = Notice::ALL
            .into_iter()
            .map(|notice| (notice, page()))
            .collect();
        let shell = Shell::new(
            &window,
            Pages {
                home: &home,
                inference: &inference,
                traces: &traces,
                history: &history,
                insights: &insights,
                mission_drafts: missions.then_some(&drafts),
                settings: &settings,
                notices: notices
                    .iter()
                    .map(|(notice, card)| (*notice, card.clone().upcast()))
                    .collect(),
            },
            missions,
        );
        // Whatever the monitor seeded, these tests start with every pane on.
        shell.update_prefs(|prefs| {
            prefs.shows_map = true;
            prefs.shows_inspector = true;
            prefs.shows_graph = true;
        });
        shell.apply();
        window.set_content(Some(&shell.root));
        window.present();
        pump();
        Mounted {
            window,
            shell,
            notices,
        }
    }

    fn at(tab: Tab) -> Location {
        Location {
            tab,
            ..Location::default()
        }
    }

    fn descendants(root: &gtk::Widget, found: &mut Vec<gtk::Widget>) {
        let mut child = root.first_child();
        while let Some(widget) = child {
            found.push(widget.clone());
            descendants(&widget, found);
            child = widget.next_sibling();
        }
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_window_has_a_window_handle_the_desktops_controls_and_no_header_bar() {
        let mounted = mounted(false);
        let mut all = Vec::new();
        descendants(mounted.shell.root.upcast_ref(), &mut all);
        assert!(
            !all.iter().any(|widget| widget.is::<adw::HeaderBar>()),
            "the main window has a header bar"
        );
        assert_eq!(
            all.iter()
                .filter(|widget| widget.is::<gtk::WindowHandle>())
                .count(),
            1
        );
        let sides: Vec<gtk::PackType> = all
            .iter()
            .filter_map(|widget| widget.downcast_ref::<gtk::WindowControls>())
            .map(|controls| controls.side())
            .collect();
        assert_eq!(sides, [gtk::PackType::Start, gtk::PackType::End]);
        // The controls follow the desktop's layout: a side it names no
        // button for draws nothing.
        let settings = gtk::Settings::default().expect("settings");
        let previous = settings.gtk_decoration_layout();
        settings.set_gtk_decoration_layout(Some("close:"));
        pump();
        for controls in all
            .iter()
            .filter_map(|widget| widget.downcast_ref::<gtk::WindowControls>())
        {
            assert_eq!(
                controls.is_visible(),
                controls.side() == gtk::PackType::Start,
                "{:?} with the buttons on the left",
                controls.side()
            );
        }
        settings.set_gtk_decoration_layout(previous.as_deref());
        mounted.window.close();
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_strip_is_the_four_tabs_and_narrows_before_onboarding() {
        let mounted = mounted(false);
        let shell = &mounted.shell;
        let words = monitor_screens_copy();
        let tabs: Vec<Tab> = shell.tabs.iter().map(|(tab, _)| *tab).collect();
        assert_eq!(tabs, Tab::ALL);
        assert!(shell.tabs.iter().all(|(_, button)| button.is_visible()));
        // The labels are the core's.
        let mut labels = Vec::new();
        for (_, button) in &shell.tabs {
            let mut inside = Vec::new();
            descendants(button.upcast_ref(), &mut inside);
            let label = inside
                .iter()
                .find_map(|widget| widget.downcast_ref::<gtk::Label>())
                .expect("a tab has a label");
            labels.push(label.text().to_string());
        }
        assert_eq!(
            labels,
            [
                words.shell.tab_home,
                words.shell.tab_inference,
                words.shell.tab_traces,
                Tab::Insights.title()
            ]
        );
        // The badge is the Traces tab's and the dot the Inference tab's.
        assert!(
            shell.traces_badge.is_ancestor(&shell.tabs[2].1),
            "the decisions-owed badge is not on Traces"
        );
        assert!(shell.inference_dot.is_ancestor(&shell.tabs[1].1));
        assert!(!shell.inference_dot.is_visible(), "no report is no dot");
        // Every state the daemon reports has a dot and the core's sentence.
        for state in trace_commons_contributor::private_inference_copy::STATE_LABELS {
            shell.render_inference_state(state);
            assert!(shell.inference_dot.is_visible(), "{state}");
            assert_eq!(
                shell.inference_dot.tooltip_text().as_deref(),
                Some(crate::copy::private_inference_state_line(state)),
                "{state}"
            );
        }
        shell.render_inference_state("");
        assert!(!shell.inference_dot.is_visible());

        // Before onboarding Home and Traces are not shown, and a selected
        // tab that is not shown is drawn as Inference.
        shell.pick_tab(Tab::Traces);
        shell.set_requires_onboarding(true);
        let visible: Vec<Tab> = shell
            .tabs
            .iter()
            .filter(|(_, button)| button.is_visible())
            .map(|(tab, _)| *tab)
            .collect();
        assert_eq!(visible, [Tab::Inference, Tab::Insights]);
        assert_eq!(shell.shown().tab, Tab::Inference);
        assert_eq!(shell.selected_tab(), Tab::Traces, "the choice is kept");
        assert_eq!(
            shell.content.visible_child_name().as_deref(),
            Some("private-inference")
        );
        // Once onboarding is done the selected tab is drawn again.
        shell.set_requires_onboarding(false);
        assert_eq!(shell.shown().tab, Tab::Traces);
        assert_eq!(shell.content.visible_child_name().as_deref(), Some("queue"));
        mounted.window.close();
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_pages_sit_behind_the_tabs_and_the_breadcrumb() {
        let mounted = mounted(true);
        let shell = &mounted.shell;
        let page = |shell: &Shell| {
            shell
                .content
                .visible_child_name()
                .map(|name| name.to_string())
        };
        for (tab, name) in [
            (Tab::Home, "home"),
            (Tab::Inference, "private-inference"),
            (Tab::Traces, "queue"),
            (Tab::Insights, "insights"),
        ] {
            shell.pick_tab(tab);
            assert_eq!(page(shell).as_deref(), Some(name));
            assert!(!shell.breadcrumb.is_visible(), "{tab:?} has no trail");
        }
        // History is Traces' page, behind the breadcrumb.
        let Some(Route::Show(history)) = tabs::route("history", at(Tab::Home), true) else {
            panic!("history did not land");
        };
        shell.go(history);
        assert_eq!(page(shell).as_deref(), Some("history"));
        assert!(shell.breadcrumb.is_visible());
        assert!(shell.tabs[2].1.is_active(), "History is on the Traces tab");
        shell.go_back(Back::Traces);
        assert_eq!(page(shell).as_deref(), Some("queue"));
        assert!(!shell.breadcrumb.is_visible());
        // Mission drafts is Home's page while missions are shown.
        let Some(Route::Show(drafts)) = tabs::route("mission-drafts", at(Tab::Traces), true) else {
            panic!("mission drafts did not land");
        };
        shell.go(drafts);
        assert_eq!(page(shell).as_deref(), Some("mission-drafts"));
        assert!(shell.tabs[0].1.is_active());
        assert!(shell.breadcrumb.is_visible());
        shell.go_back(Back::Home);
        assert_eq!(page(shell).as_deref(), Some("home"));
        // Settings is in no tab.
        assert!(shell.content.child_by_name("settings").is_none());
        assert_eq!(
            tabs::route("settings", at(Tab::Home), true),
            Some(Route::Settings)
        );
        mounted.window.close();
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_toggles_and_panes_follow_what_each_tab_allows() {
        let mounted = mounted(false);
        let shell = &mounted.shell;
        let words = monitor_screens_copy();

        shell.pick_tab(Tab::Traces);
        assert!(shell.graph_toggle.is_sensitive() && shell.graph_toggle.is_active());
        assert!(shell.graph.root.reveals_child());
        assert_eq!(
            shell.graph_toggle.tooltip_text().as_deref(),
            Some(words.shell.hide_graph)
        );
        assert!(shell.inspector_toggle.is_sensitive() && shell.inspector_toggle.is_active());
        assert_eq!(shell.root.visibility(), Visibility::new(true, true));

        // Home: no graph and no inspector, and both toggles read as hidden.
        shell.pick_tab(Tab::Home);
        assert!(!shell.graph_toggle.is_sensitive() && !shell.graph_toggle.is_active());
        assert!(!shell.graph.root.reveals_child());
        assert_eq!(
            shell.graph_toggle.tooltip_text().as_deref(),
            Some(words.shell.show_graph)
        );
        assert!(!shell.inspector_toggle.is_sensitive() && !shell.inspector_toggle.is_active());
        assert_eq!(
            shell.inspector_toggle.tooltip_text().as_deref(),
            Some(words.shell.show_inspector)
        );
        assert_eq!(shell.root.visibility(), Visibility::new(true, false));
        // The person's choice is untouched: back on Traces it is open again.
        shell.pick_tab(Tab::Traces);
        assert_eq!(shell.root.visibility(), Visibility::new(true, true));

        // Never on Inference; the map stays whichever tab is showing.
        shell.pick_tab(Tab::Inference);
        assert_eq!(shell.root.visibility(), Visibility::new(true, false));
        assert_eq!(shell.map_view(), MapView::PrivateAi);
        shell.pick_tab(Tab::Home);
        assert_eq!(shell.map_view(), MapView::PrivateAi, "Home keeps the view");
        shell.pick_tab(Tab::Traces);
        assert_eq!(shell.map_view(), MapView::Traces);

        // The map toggle hides the pane and names what pressing it does.
        shell.map_toggle.set_active(false);
        // Not wired without an `App`: apply the press as its handler would.
        shell.set_map_shown(false);
        assert_eq!(shell.root.visibility(), Visibility::new(false, true));
        assert_eq!(
            shell.map_toggle.tooltip_text().as_deref(),
            Some(words.shell.show_map)
        );
        pump();
        // Whatever the compositor did with the resize request, the main
        // pane kept at least its width.
        let layout = pane_layout::PaneLayout::new(
            shell.root.laid_out_width(),
            shell.root.visibility(),
            shell.root.kept_main(),
        );
        assert!(layout.main >= pane_layout::MAIN_WIDTH);
        assert_eq!(layout.map, None);
        mounted.window.close();
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_notices_are_laid_over_the_tab_they_belong_to() {
        let mounted = mounted(false);
        let shell = &mounted.shell;
        let slot_shown = |notice: Notice| {
            shell
                .notice_slots
                .iter()
                .find(|(kind, _)| *kind == notice)
                .map(|(_, slot)| slot.is_visible())
                .expect("every notice has a slot")
        };
        // Each card is inside its slot, over the content and not above it.
        for (notice, card) in &mounted.notices {
            let (_, slot) = shell
                .notice_slots
                .iter()
                .find(|(kind, _)| kind == notice)
                .unwrap();
            assert!(card.is_ancestor(slot));
            assert!(
                slot.ancestor(gtk::Overlay::static_type()).is_some(),
                "{notice:?} pushes the content down"
            );
        }
        // The undo comes first: it is the one card with a clock on it.
        assert_eq!(shell.notice_slots[0].0, Notice::Undo);
        let scroller = shell.notice_slots[0]
            .1
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
            .expect("the notices are in a column that scrolls");
        // Empty cards are showing here, so the column is too; with every
        // card hidden it is gone, and takes no click from the tab under it.
        assert!(scroller.is_visible());
        for (_, card) in &mounted.notices {
            card.set_visible(false);
        }
        assert!(
            !scroller.is_visible(),
            "an empty notice column still covers the tab"
        );
        // Cards taller than the tab scroll inside it; none is pushed out of
        // reach under the graph footer.
        shell.pick_tab(Tab::Traces);
        for (_, card) in &mounted.notices {
            card.set_size_request(-1, 400);
            card.set_visible(true);
        }
        pump();
        let overlay = scroller.parent().expect("the column is laid over the tab");
        assert!(scroller.is_visible());
        assert!(
            scroller.height() <= overlay.height(),
            "the notices run past the tab's own area"
        );
        let scroll = scroller.vadjustment();
        assert!(
            scroll.upper() > scroll.page_size(),
            "the cards that do not fit cannot be scrolled to"
        );
        for (_, card) in &mounted.notices {
            card.set_size_request(-1, -1);
        }
        for tab in Tab::ALL {
            shell.pick_tab(tab);
            let place = tabs::place(shell.shown());
            for notice in Notice::ALL {
                assert_eq!(
                    slot_shown(notice),
                    notices::shown(notice, place),
                    "{notice:?} on {tab:?}"
                );
            }
            // A safeguard is never hidden by a tab.
            assert!(slot_shown(Notice::Health) && slot_shown(Notice::Consent));
        }
        mounted.window.close();
    }
}
