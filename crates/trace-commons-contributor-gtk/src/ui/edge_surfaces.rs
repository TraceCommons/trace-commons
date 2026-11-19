//! The edge rail and the menu-bar panel as GTK windows.
//!
//! The rules are `crate::edge`'s; this module builds the two windows and
//! wires them to the app. The Mac's `EdgeRailView` and `MenuBarGlassPanel`
//! are the reference. What each shows, and what it does not show yet, is
//! said beside the code that draws it.
//!
//! Both read only what the window already read (`App`'s queue, history,
//! status and the Private AI switch), and only while they are shown: a
//! closed rail and a hidden panel draw nothing, read nothing and run no
//! timer. Opening either asks the app for a fresh read, as the Mac's do.
//! Neither sends anything. The one write either offers is the watcher's
//! pause and resume, the same one the Mac's popover and rail offer.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use trace_commons_contributor::preview_copy::{monitor_screens_copy, monitor_traces_copy};

use super::App;
use super::monitor::glyph;
use super::monitor::graph_footer::GraphFooter;
use super::monitor::traces_tree::{FolderNode, ProjectMode, TracesTree};
use super::settings::Section;
use super::style::{self, brand_tokens::glass};
use crate::edge::capability::{self, Modes, PanelMode, RailMode};
use crate::edge::geometry;
use crate::edge::gnome::{self, Role};
use crate::edge::opening::Opening;
use crate::edge::prefs::EdgePrefs;
use crate::edge::status::{self, WatchChoice};

/// The rail's and the panel's state, held by the app.
#[derive(Default)]
pub struct Surfaces {
    /// What this session supports; `None` until it is known.
    modes: Cell<Option<Modes>>,
    gnome: RefCell<Option<gnome::Client>>,
    rail: RefCell<Option<Rc<Rail>>>,
    panel: RefCell<Option<Rc<Panel>>>,
    /// The Settings switch is being set from the stored choice, not by a
    /// person.
    filling: Cell<bool>,
}

/// The rail's switch in Settings: the Mac's `GeneralSection.edgeRailCard`,
/// under the "Desktop" eyebrow beside the startup card. Hidden until the
/// session is known to support a rail, and kept hidden where it does not.
pub struct RailSwitch {
    pub card: gtk::Box,
    pub switch: gtk::Switch,
}

pub fn rail_switch() -> RailSwitch {
    let words = monitor_screens_copy();
    let rail = &words.edge_rail;
    let card = style::card(gtk::Orientation::Vertical, style::space::S);
    card.append(&style::eyebrow(words.shell.desktop_eyebrow));
    let title = gtk::Label::builder()
        .label(rail.rail_label)
        .xalign(0.0)
        .build();
    title.add_css_class("tc-card-title");
    card.append(&title);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, style::space::S);
    let label = gtk::Label::builder()
        .label(rail.setting_toggle)
        .xalign(0.0)
        .hexpand(true)
        .build();
    label.add_css_class("tc-body");
    let switch = gtk::Switch::builder().halign(gtk::Align::End).build();
    switch.update_property(&[gtk::accessible::Property::Label(rail.setting_toggle)]);
    row.append(&label);
    row.append(&switch);
    card.append(&row);
    let caption = gtk::Label::builder()
        .label(rail.setting_caption)
        .xalign(0.0)
        .wrap(true)
        .build();
    caption.add_css_class("tc-meta");
    card.append(&caption);
    card.set_visible(false);
    RailSwitch { card, switch }
}

/// Learns what the session supports, shows the rail if it is switched on,
/// and wires the Settings switch.
pub fn wire(app: &Rc<App>) {
    {
        let app = Rc::clone(app);
        glib::spawn_future_local(async move {
            let client = gnome::find().await;
            let extension = client.is_some();
            app.edge.gnome.replace(client);
            let Some(display) = gtk::gdk::Display::default() else {
                return;
            };
            let facts = capability::detect(&display, extension);
            let modes = capability::modes(facts);
            // Fixed labels only: which way each surface is shown here.
            eprintln!(
                "trace-commons-shell: edge surfaces {:?}, rail {:?}, panel {:?}",
                facts.backend, modes.rail, modes.panel
            );
            app.edge.modes.set(Some(modes));
            let on = EdgePrefs::load().edge_rail_enabled;
            app.edge.filling.set(true);
            app.settings.edge_rail.switch.set_active(on);
            app.edge.filling.set(false);
            app.settings.edge_rail.card.set_visible(modes.offers_rail());
            if on && modes.offers_rail() {
                show_rail(&app);
            }
        });
    }
    let weak = Rc::downgrade(app);
    app.settings
        .edge_rail
        .switch
        .connect_active_notify(move |switch| {
            let Some(app) = weak.upgrade() else { return };
            if app.edge.filling.get() {
                return;
            }
            set_rail_enabled(&app, switch.is_active());
        });
    // Neither window outlives the main one. On GNOME both belong to the
    // application (`gnome::surface_window`) and would keep the process
    // alive; destroyed here with the main window, they do not.
    let weak = Rc::downgrade(app);
    app.window.connect_destroy(move |_| {
        if let Some(app) = weak.upgrade() {
            if let Some(rail) = app.edge.rail.take() {
                rail.remove();
            }
            if let Some(panel) = app.edge.panel.take() {
                panel.window.destroy();
            }
        }
    });
}

/// The Settings switch: remembers the choice and shows or removes the rail
/// at once (`EdgeRailController.setEnabled`).
pub fn set_rail_enabled(app: &Rc<App>, enabled: bool) {
    EdgePrefs {
        edge_rail_enabled: enabled,
    }
    .save();
    if enabled {
        show_rail(app);
    } else if let Some(rail) = app.edge.rail.take() {
        rail.remove();
    }
}

/// The app's reads changed: redraws whichever surface is showing, and
/// nothing else.
pub fn data_changed(app: &Rc<App>) {
    let rail = app.edge.rail.borrow().clone();
    if let Some(rail) = rail
        && rail.opening.is_open()
    {
        rail.render(app);
    }
    let panel = app.edge.panel.borrow().clone();
    if let Some(panel) = panel
        && panel.window.is_visible()
    {
        panel.render(app);
    }
}

/// The tray icon was activated at `at` (screen coordinates, when the host
/// gave them): opens the panel there, or closes it if it is open, as the
/// Mac's menu-bar item does.
pub fn toggle_panel(app: &Rc<App>, at: Option<(i32, i32)>) {
    let mode = app
        .edge
        .modes
        .get()
        .map_or(PanelMode::PlainWindow, |modes| modes.panel);
    let existing = app.edge.panel.borrow().clone();
    let panel = match existing {
        Some(panel) if panel.mode == mode => panel,
        stale => {
            // One made before the session was known, as a plain window: gone,
            // not merely hidden, once replaced.
            if let Some(stale) = stale {
                stale.window.destroy();
            }
            let panel = Panel::new(app, mode);
            app.edge.panel.replace(Some(Rc::clone(&panel)));
            panel
        }
    };
    if panel.window.is_visible() {
        panel.window.set_visible(false);
        return;
    }
    panel.show(app, at);
}

fn show_rail(app: &Rc<App>) {
    if app.edge.rail.borrow().is_some() {
        return;
    }
    let Some(modes) = app.edge.modes.get() else {
        return;
    };
    if !modes.offers_rail() {
        return;
    }
    let rail = Rail::new(app, modes.rail);
    app.edge.rail.replace(Some(rail));
}

// --- Motion ----------------------------------------------------------------

/// The revealer only shows and hides the open rail; the slide in from the
/// trailing edge and the fade are `edge_motion::SlideX`'s, on the token
/// curve, as the Mac's `EdgeRailView` moves it.
fn follow_motion(revealer: &gtk::Revealer) {
    revealer.set_transition_duration(0);
    revealer.set_transition_type(gtk::RevealerTransitionType::None);
}

/// How far right of its place the open rail starts: off the trailing edge,
/// its own width and the inset beside it (`.move(edge: .trailing)`).
fn slide_distance(row: &gtk::Box) -> f64 {
    let width = match row.width() {
        0 => row.measure(gtk::Orientation::Horizontal, -1).1,
        laid_out => laid_out,
    };
    f64::from(width + glass::space::EDGE_RAIL_INSET)
}

/// A monitor's place and size in desktop coordinates.
fn monitor_rect(monitor: &gtk::gdk::Monitor) -> geometry::Rect {
    let area = monitor.geometry();
    geometry::Rect {
        x: area.x(),
        y: area.y(),
        width: area.width(),
        height: area.height(),
    }
}

/// The display's monitors, in its order.
fn monitors(display: &gtk::gdk::Display) -> Vec<gtk::gdk::Monitor> {
    let list = display.monitors();
    (0..list.n_items())
        .filter_map(|index| list.item(index))
        .filter_map(|item| item.downcast::<gtk::gdk::Monitor>().ok())
        .collect()
}

/// The rail's monitor: under layer-shell the first, as the Mac's rail is on
/// `NSScreen.screens.first`; under the GNOME extension the one the window
/// is on, the primary (`geometry::rail_monitor`).
fn rail_monitor(window: &gtk::Window, mode: RailMode) -> Option<gtk::gdk::Monitor> {
    let display = WidgetExt::display(window);
    let under_window = window
        .surface()
        .and_then(|surface| display.monitor_at_surface(&surface));
    geometry::rail_monitor(
        mode == RailMode::GnomeExtension,
        under_window,
        monitors(&display).into_iter().next(),
    )
}

/// The rail's monitor's height, for the rail's tallest size.
fn rail_monitor_height(window: &gtk::Window, mode: RailMode) -> i32 {
    rail_monitor(window, mode).map_or(1080, |monitor| monitor_rect(&monitor).height)
}

/// The panel's monitor: the one under the tray click, else the main
/// window's, else the first.
#[cfg_attr(
    not(all(target_os = "linux", feature = "layer-shell")),
    allow(dead_code)
)]
fn panel_monitor(app: &App, at: Option<(i32, i32)>) -> Option<gtk::gdk::Monitor> {
    let display = WidgetExt::display(&app.window);
    let all = monitors(&display);
    let rects: Vec<geometry::Rect> = all.iter().map(monitor_rect).collect();
    geometry::monitor_for_click(&rects, at)
        .and_then(|index| all.get(index).cloned())
        .or_else(|| {
            app.window
                .surface()
                .and_then(|surface| display.monitor_at_surface(&surface))
        })
        .or_else(|| all.into_iter().next())
}

// --- Glyphs ----------------------------------------------------------------

/// The rail's and the panel's glyphs that `brand_tokens::glyph` does not
/// hold, in its SVG shape so `monitor::glyph` draws them: SF Symbols'
/// shield (Private AI's rail icon, owner 2026-10-11), power (Quit), and
/// eye and eye.slash (Watching).
pub(crate) mod glyphs {
    pub const SHIELD: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="17" height="17"><path d="M12,2.5L19.5,5.5L19.5,11A9,10,0,0,1,12,21.5A9,10,0,0,1,4.5,11L4.5,5.5Z" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/></svg>"#;
    pub const POWER: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="15" height="15"><path d="M12,3L12,11M6.34,7.34A8,8,0,1,0,17.66,7.34" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/></svg>"#;
    pub const EYE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="16" height="16"><path d="M2,12A11,8,0,0,1,22,12A11,8,0,0,1,2,12ZM15,12A3,3,0,1,1,9,12A3,3,0,1,1,15,12Z" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/></svg>"#;
    pub const EYE_SLASH: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="16" height="16"><path d="M2,12A11,8,0,0,1,22,12A11,8,0,0,1,2,12ZM15,12A3,3,0,1,1,9,12A3,3,0,1,1,15,12ZM4,4L20,20" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/></svg>"#;
}

/// Traces' rail icon (`GlassTraceNodesGlyph`, owner 2026-10-22): four
/// nodes in a T, left, middle and right across the top and one under the
/// middle, joined across the top and down from the middle.
pub(crate) mod trace_nodes {
    pub const NODES: [(f64, f64); 4] = [(0.16, 0.3), (0.5, 0.3), (0.84, 0.3), (0.5, 0.84)];
    pub const NODE_RADIUS: f64 = 0.12;
    pub const LINE_WIDTH: f64 = 0.08;
    pub const LINKS: [(usize, usize); 2] = [(0, 2), (1, 3)];
}

fn trace_nodes_icon(side: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(side)
        .content_height(side)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build();
    area.set_draw_func(|area, cr, width, height| {
        #[allow(deprecated)]
        let color = area.style_context().color();
        cr.set_source_rgba(
            f64::from(color.red()),
            f64::from(color.green()),
            f64::from(color.blue()),
            f64::from(color.alpha()),
        );
        let side = f64::from(width.min(height));
        let origin = (
            (f64::from(width) - side) / 2.0,
            (f64::from(height) - side) / 2.0,
        );
        let point = |(x, y): (f64, f64)| (origin.0 + x * side, origin.1 + y * side);
        cr.set_line_width(trace_nodes::LINE_WIDTH * side);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        for (from, to) in trace_nodes::LINKS {
            let (a, b) = (
                point(trace_nodes::NODES[from]),
                point(trace_nodes::NODES[to]),
            );
            cr.move_to(a.0, a.1);
            cr.line_to(b.0, b.1);
        }
        let _ = cr.stroke();
        for node in trace_nodes::NODES {
            let (x, y) = point(node);
            cr.arc(
                x,
                y,
                trace_nodes::NODE_RADIUS * side,
                0.0,
                std::f64::consts::TAU,
            );
            let _ = cr.fill();
        }
    });
    area
}

/// A round icon button with its name as tooltip and accessible label
/// (`GlassRoundButton`).
fn round_button(svg: &str, name: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(&glyph::icon(svg))
        .tooltip_text(name)
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("tc-edge-round");
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    button
}

/// Watching as a round dropdown (`WatchingMenuButton`): the pause lengths
/// while watching, Resume while paused, insensitive while the core has not
/// answered.
fn watching_button(app: &Rc<App>) -> gtk::Button {
    let words = monitor_screens_copy();
    let state = status::watching(app.status.borrow().as_ref().map(|status| status.paused));
    let icon = if state == status::Watching::Paused {
        glyphs::EYE_SLASH
    } else {
        glyphs::EYE
    };
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let heading = gtk::Label::builder()
        .label(words.watching)
        .xalign(0.0)
        .build();
    heading.add_css_class("tc-meta");
    list.append(&heading);
    let popover = gtk::Popover::builder().child(&list).build();
    for (choice, label) in status::watch_choices(state) {
        let item = gtk::Button::builder().label(label).build();
        item.add_css_class("flat");
        let weak = Rc::downgrade(app);
        let menu = popover.downgrade();
        item.connect_clicked(move |_| {
            if let Some(menu) = menu.upgrade() {
                menu.popdown();
            }
            if let Some(app) = weak.upgrade() {
                watch(&app, choice);
            }
        });
        list.append(&item);
    }
    // A button with its own popover rather than a GtkMenuButton, whose
    // custom child needs GTK 4.6 API this crate's feature floor leaves out.
    let button = gtk::Button::builder()
        .child(&glyph::icon(icon))
        .tooltip_text(words.watching)
        .valign(gtk::Align::Center)
        .sensitive(state != status::Watching::Unknown)
        .build();
    popover.set_parent(&button);
    {
        let popover = popover.downgrade();
        button.connect_clicked(move |_| {
            if let Some(popover) = popover.upgrade() {
                popover.popup();
            }
        });
    }
    // The popover is parented by hand, so it is unparented by hand.
    button.connect_destroy(move |_| popover.unparent());
    button.add_css_class("tc-edge-round");
    button.update_property(&[
        gtk::accessible::Property::Label(words.watching),
        gtk::accessible::Property::Description(status::watching_value(state)),
    ]);
    button
}

/// Pauses or resumes, through the same daemon calls Settings makes.
fn watch(app: &Rc<App>, choice: WatchChoice) {
    let params = match choice {
        WatchChoice::Resume => {
            app.call("resume", serde_json::json!({}), |app, _| app.refresh());
            return;
        }
        WatchChoice::PauseHour => serde_json::json!({
            "until": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339()
        }),
        WatchChoice::PauseMorning => {
            serde_json::json!({ "until": super::settings::tomorrow_morning().to_rfc3339() })
        }
        WatchChoice::PauseUntilResumed => serde_json::json!({}),
    };
    app.call("pause", params, |app, _| app.refresh());
}

/// Private AI's one word with its dot (`PrivateAIStatusLabel`).
fn private_ai_label(app: &Rc<App>) -> gtk::Label {
    let state = status::private_ai(
        app.private_inference.confirmed.get(),
        app.private_inference.reported.borrow().as_deref(),
    );
    let tone = super::private_inference::indicator_tone(status::private_ai_tone(state));
    let word = status::private_ai_word(state);
    let label = gtk::Label::new(Some(&format!("{} {word}", tone.glyph())));
    label.add_css_class(tone.css());
    label.add_css_class("tc-meta");
    let sentence = crate::copy::private_inference_state_line(
        app.private_inference
            .reported
            .borrow()
            .as_deref()
            .unwrap_or(""),
    );
    label.set_tooltip_text(Some(sentence));
    label.update_property(&[gtk::accessible::Property::Description(sentence)]);
    label
}

/// The shared and kept legend over the app's bar graph, without its
/// controls, as the rail's Traces peek and the popover draw it.
fn graph(app: &Rc<App>) -> Rc<GraphFooter> {
    let graph = GraphFooter::new();
    graph.compact(geometry::GRAPH_HEIGHT);
    graph.render(app.shell.graph_data(app, None));
    graph
}

fn clear(parent: &gtk::Box) {
    while let Some(child) = parent.first_child() {
        parent.remove(&child);
    }
}

/// Opens the window at a screen, and puts the rail away first, as the
/// Mac's `EdgeRailView.open` does ("Opens the app at a destination and
/// puts the rail away").
fn open(app: &Rc<App>, screen: &str) {
    put_rail_away(app);
    app.open_screen(screen);
    app.window.present();
}

/// Opens Settings at `section` (the Mac's `open(.settings(section))`), and
/// puts the rail away first.
fn open_settings(app: &Rc<App>, section: Section) {
    put_rail_away(app);
    app.open_screen(super::SETTINGS_SCREEN);
    app.settings.show_section(section);
}

/// Closes the rail if it is open; a closed rail stays as it is.
fn put_rail_away(app: &Rc<App>) {
    let rail = app.edge.rail.borrow().clone();
    if let Some(rail) = rail
        && rail.opening.is_open()
    {
        rail.cancel_close();
        rail.opening.set(false);
    }
}

/// The Settings section a peek's Settings button opens
/// (`EdgeRailSettings.section`): Private AI's own; for Traces General,
/// which holds the rail's switch.
fn settings_section(peek: Peek) -> Section {
    match peek {
        Peek::PrivateAi => Section::PrivateAI,
        Peek::Traces => Section::General,
    }
}

// --- The rail --------------------------------------------------------------

/// The rail window's title: on GNOME with the extension, the app's name,
/// which is what Alt-Tab and the overview show for it there (owner,
/// 2026-11-18, "The app's brand name"); the extension finds it by its
/// marker (`gnome::surface_window`), never by title. Elsewhere, the rail's
/// name. Its accessible name is the rail's name either way.
fn rail_title(mode: RailMode) -> &'static str {
    match mode {
        RailMode::GnomeExtension => crate::copy::APP_NAME,
        _ => monitor_screens_copy().edge_rail.rail_label,
    }
}

/// The panel window's title: the app's name, which a plain window shows in
/// its title bar and GNOME's Alt-Tab shows for the extension's.
fn panel_title(_mode: PanelMode) -> &'static str {
    crate::copy::APP_NAME
}

/// A window for the rail or the panel: on GNOME with the extension, an
/// application window of the app carrying its role's marker, so the
/// extension can find it; elsewhere a plain window.
fn surface_window(app: &App, gnome_role: Option<Role>, title: &str) -> gtk::Window {
    new_surface_window(app.window.application().as_ref(), gnome_role, title)
}

fn new_surface_window(
    application: Option<&gtk::Application>,
    gnome_role: Option<Role>,
    title: &str,
) -> gtk::Window {
    let window = match (gnome_role, application) {
        (Some(role), Some(application)) => gnome::surface_window(application, role).upcast(),
        _ => gtk::Window::new(),
    };
    window.set_title(Some(title));
    // A group of its own: an open dialog of the app (first run, terms, a
    // preview) grabs its window group, and GTK drops the keys of every
    // other window in that group, so in the app's group the clicked rail
    // would hold the keyboard while Escape and Tab did nothing.
    gtk::WindowGroup::new().add_window(&window);
    window
}

/// The rail's icons, top to bottom (owner, 2026-10-11): one per tab of the
/// app, each peek a compact summary of that tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Peek {
    PrivateAi,
    Traces,
}

impl Peek {
    pub const ALL: [Peek; 2] = [Peek::PrivateAi, Peek::Traces];

    /// The icon's name and its peek's title: the app's tab names.
    pub fn name(self) -> &'static str {
        let words = monitor_screens_copy();
        match self {
            Peek::PrivateAi => words.settings_nav.private_ai,
            Peek::Traces => words.shell.tab_traces,
        }
    }

    /// The tab a peek's row opens.
    fn screen(self) -> &'static str {
        match self {
            Peek::PrivateAi => super::PRIVATE_INFERENCE_SCREEN,
            Peek::Traces => super::monitor::tabs::Tab::Traces.name(),
        }
    }
}

/// A folder row's second line (`TracesTreeView.folderSub`): ignored, or how
/// many traces wait in it, in the core's words. The Mac puts the folder's
/// rule before the count; this shell does not read the rules' labels yet.
pub fn folder_sub(folder: &FolderNode) -> String {
    let words = monitor_traces_copy();
    if folder.mode == Some(ProjectMode::Ignore) {
        return words.tree.ignored_folder.to_string();
    }
    match folder.sessions.len() {
        1 => words.counts.sessions_waiting_one.to_string(),
        count => words
            .counts
            .sessions_waiting
            .replace("{count}", &count.to_string()),
    }
}

struct Rail {
    window: gtk::Window,
    mode: RailMode,
    revealer: gtk::Revealer,
    opening: Rc<Opening>,
    /// The pending close: leaving the rail for a moment, as the pointer
    /// does crossing from an icon to its peek, does not close it. Only ever
    /// set while the rail is open.
    closing: RefCell<Option<glib::SourceId>>,
    peek: Cell<Peek>,
    tiles: Vec<(Peek, gtk::Button)>,
    peek_body: gtk::Box,
    /// Whether a layer-shell rail takes the keyboard (owner, 2026-11-18,
    /// "Keyboard only after a click").
    #[cfg_attr(
        not(all(target_os = "linux", feature = "layer-shell")),
        allow(dead_code)
    )]
    keyboard: Rc<crate::edge::keyboard::RailKeyboard>,
    app: std::rc::Weak<App>,
}

impl Rail {
    fn new(app: &Rc<App>, mode: RailMode) -> Rc<Self> {
        let words = monitor_screens_copy();
        let rail_words = &words.edge_rail;
        let window = surface_window(
            app,
            (mode == RailMode::GnomeExtension).then_some(Role::Rail),
            rail_title(mode),
        );
        window.set_decorated(false);
        window.set_resizable(false);
        window.add_css_class("tc-edge-surface");
        // GTK opens a window at 200 x 200 unless asked; the closed rail is
        // its four-pixel zone from the first frame.
        window.set_default_size(1, 1);
        window.update_property(&[gtk::accessible::Property::Label(rail_words.rail_label)]);

        // Closed: the handle on the edge, in a zone four pixels wide.
        let handle = gtk::Box::builder()
            .width_request(glass::size::EDGE_RAIL_HANDLE_WIDTH)
            .height_request(glass::size::EDGE_RAIL_HANDLE_HEIGHT)
            .halign(gtk::Align::End)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        handle.add_css_class("tc-edge-rail-handle");
        let zone = gtk::Box::builder()
            .width_request(glass::size::EDGE_RAIL_ZONE_WIDTH)
            .height_request(glass::size::EDGE_RAIL_ZONE_HEIGHT)
            .accessible_role(gtk::AccessibleRole::Button)
            .build();
        zone.append(&handle);
        zone.update_property(&[gtk::accessible::Property::Label(rail_words.handle_label)]);

        // Open: the peek, then the icon rail, inset from the edge.
        let peek_body = gtk::Box::new(gtk::Orientation::Vertical, glass::space::CARD_GAP);
        let peek_card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(glass::size::EDGE_RAIL_PEEK_WIDTH)
            .valign(gtk::Align::Center)
            .build();
        peek_card.add_css_class("tc-glass-pane");
        peek_card.add_css_class("tc-edge-rail-peek");
        peek_body.set_margin_top(glass::space::PANE_PADDING);
        peek_body.set_margin_bottom(glass::space::PANE_PADDING);
        peek_body.set_margin_start(glass::space::PANE_PADDING);
        peek_body.set_margin_end(glass::space::PANE_PADDING);
        peek_card.append(&peek_body);

        let icons = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(glass::space::S1)
            .width_request(glass::size::EDGE_RAIL_WIDTH)
            .valign(gtk::Align::Center)
            .margin_top(0)
            .build();
        icons.add_css_class("tc-glass-pane");
        icons.add_css_class("tc-edge-rail-icons");
        let inner = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(glass::space::S1)
            .halign(gtk::Align::Center)
            .margin_top(glass::space::S3)
            .margin_bottom(glass::space::S3)
            .build();
        icons.append(&inner);
        let mut tiles = Vec::new();
        for peek in Peek::ALL {
            let child: gtk::Widget = match peek {
                Peek::PrivateAi => glyph::icon(glyphs::SHIELD).upcast(),
                Peek::Traces => trace_nodes_icon(geometry::MARK_SIDE).upcast(),
            };
            let tile = tile(&child, peek.name());
            inner.append(&tile);
            tiles.push((peek, tile));
        }
        let rule = gtk::Box::builder()
            .width_request(glass::size::EDGE_RAIL_RULE_WIDTH)
            .height_request(1)
            .halign(gtk::Align::Center)
            .margin_top(glass::space::S2)
            .margin_bottom(glass::space::S2)
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        rule.add_css_class("tc-edge-rail-rule");
        inner.append(&rule);
        let app_tile = tile(
            &super::mark::framed(geometry::MARK_SIDE).upcast::<gtk::Widget>(),
            rail_words.open_app,
        );
        inner.append(&app_tile);

        let open_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(glass::space::EDGE_RAIL_GAP)
            .margin_start(geometry::SHADOW_ROOM)
            .margin_end(glass::space::EDGE_RAIL_INSET)
            .halign(gtk::Align::End)
            .valign(gtk::Align::Center)
            .build();
        open_row.append(&peek_card);
        open_row.append(&icons);
        let slide = super::edge_motion::SlideX::new(&open_row);
        let revealer = gtk::Revealer::builder()
            .child(&slide)
            .visible(false)
            .build();
        follow_motion(&revealer);

        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.append(&zone);
        root.append(&revealer);
        window.set_child(Some(&root));

        #[cfg(all(target_os = "linux", feature = "layer-shell"))]
        if mode == RailMode::LayerShell {
            crate::edge::layer::make_rail(&window, rail_monitor(&window, mode).as_ref());
        }

        let keyboard = Rc::new(crate::edge::keyboard::RailKeyboard::default());
        let opening = {
            let next_turn: crate::edge::opening::NextTurn = Box::new(|step| {
                glib::idle_add_local_once(step);
            });
            let animate: crate::edge::opening::Animate = {
                let revealer = revealer.clone();
                let keyboard = Rc::clone(&keyboard);
                let (slide, open_row) = (slide.clone(), open_row.clone());
                // Bumped by every open and close, so a close's hide that a
                // reopening overtook leaves the rail shown.
                let moves = Rc::new(Cell::new(0u64));
                #[cfg(all(target_os = "linux", feature = "layer-shell"))]
                let window = window.clone();
                Box::new(move |open, done| {
                    let this_move = moves.get() + 1;
                    moves.set(this_move);
                    if open {
                        revealer.set_reveal_child(true);
                    }
                    slide.slide(open, slide_distance(&open_row));
                    if open {
                        done();
                    } else {
                        // The keyboard goes back as the close starts: a
                        // hover that opens the rail again before the slide
                        // ends never places it closed.
                        let takes = keyboard.closing();
                        #[cfg(all(target_os = "linux", feature = "layer-shell"))]
                        if mode == RailMode::LayerShell {
                            crate::edge::layer::set_rail_keyboard(&window, takes);
                        }
                        let _ = takes;
                        let duration = if super::glass_motion::animations_enabled() {
                            u64::from(glass::motion::SLIDE_MS)
                        } else {
                            0
                        };
                        let revealer = revealer.clone();
                        let moves = Rc::clone(&moves);
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(duration + 16),
                            move || {
                                if moves.get() == this_move {
                                    revealer.set_reveal_child(false);
                                }
                                done();
                            },
                        );
                    }
                })
            };
            let place: crate::edge::opening::Place = {
                let (window, root, zone) = (window.clone(), root.clone(), zone.clone());
                let revealer = revealer.clone();
                let weak_app = Rc::downgrade(app);
                let keyboard = Rc::clone(&keyboard);
                Box::new(move |open| {
                    zone.set_visible(!open);
                    // A hidden revealer that slides sideways still asks for
                    // its child's height; the closed rail is the zone alone.
                    revealer.set_visible(open);
                    let (width, height) =
                        geometry::rail_size(open, rail_monitor_height(&window, mode));
                    root.set_size_request(width, height);
                    if !open {
                        // GTK keeps a window at its largest size unless asked.
                        window.set_default_size(1, 1);
                    }
                    // Opening by hover takes no keyboard; closing gives it
                    // back, whatever closed it.
                    let takes = keyboard.placed(open);
                    #[cfg(all(target_os = "linux", feature = "layer-shell"))]
                    if mode == RailMode::LayerShell {
                        crate::edge::layer::set_rail_keyboard(&window, takes);
                        // Still holding the keyboard it asked to give back
                        // as the close began: mapped afresh to let it go.
                        if !open && keyboard.placed_closed(window.is_active()) {
                            crate::edge::layer::map_afresh(&window);
                        }
                    }
                    let _ = takes;
                    // On GNOME the extension keeps the keyboard with the rail
                    // only while it is open, so it is told each time.
                    if mode == RailMode::GnomeExtension
                        && let Some(app) = weak_app.upgrade()
                        && let Some(client) = app.edge.gnome.borrow().as_ref()
                    {
                        client.send(gnome::Request::Place {
                            role: Role::Rail,
                            at: (-1, -1),
                            open,
                        });
                    }
                    if open && let Some(app) = weak_app.upgrade() {
                        // Fresh reads on opening; the peek draws what is
                        // already read meanwhile.
                        app.refresh();
                        let rail = app.edge.rail.borrow().clone();
                        if let Some(rail) = rail {
                            rail.render(&app);
                        }
                    }
                })
            };
            Opening::new(next_turn, animate, place)
        };

        let rail = Rc::new(Self {
            window,
            mode,
            revealer,
            opening,
            closing: RefCell::new(None),
            peek: Cell::new(Peek::PrivateAi),
            tiles,
            peek_body,
            keyboard,
            app: Rc::downgrade(app),
        });

        for (peek, tile) in &rail.tiles {
            let peek = *peek;
            let select = {
                let weak = Rc::downgrade(&rail);
                move || {
                    if let Some(rail) = weak.upgrade() {
                        rail.select(peek);
                    }
                }
            };
            let hover = gtk::EventControllerMotion::new();
            let on_hover = select.clone();
            hover.connect_enter(move |_, _, _| on_hover());
            tile.add_controller(hover);
            tile.connect_clicked(move |_| select());
        }
        {
            let weak = Rc::downgrade(&rail);
            app_tile.connect_clicked(move |_| {
                if let Some(rail) = weak.upgrade() {
                    rail.opening.set(false);
                    if let Some(app) = rail.app.upgrade() {
                        app.window.present();
                    }
                }
            });
        }
        {
            let weak = Rc::downgrade(&rail);
            let click = gtk::GestureClick::new();
            click.connect_released(move |_, _, _, _| {
                if let Some(rail) = weak.upgrade() {
                    rail.opening.set(true);
                }
            });
            zone.add_controller(click);
        }
        let motion = gtk::EventControllerMotion::new();
        {
            let weak = Rc::downgrade(&rail);
            motion.connect_enter(move |_, _, _| {
                if let Some(rail) = weak.upgrade() {
                    rail.cancel_close();
                    if !rail.opening.is_open() && rail.keyboard.entered() {
                        rail.opening.set(true);
                    }
                }
            });
        }
        {
            // A rail mapped afresh under a parked pointer opens on the first
            // motion, not on the enter (`RailKeyboard::entered`).
            let weak = Rc::downgrade(&rail);
            motion.connect_motion(move |_, _, _| {
                if let Some(rail) = weak.upgrade()
                    && !rail.opening.is_open()
                    && rail.keyboard.moved()
                {
                    rail.opening.set(true);
                }
            });
        }
        {
            let weak = Rc::downgrade(&rail);
            motion.connect_leave(move |_| {
                if let Some(rail) = weak.upgrade() {
                    rail.schedule_close();
                }
            });
        }
        rail.window.add_controller(motion);
        // A press anywhere inside the open rail gives it the keyboard under
        // layer-shell, before the tile or button under it sees the press.
        #[cfg(all(target_os = "linux", feature = "layer-shell"))]
        if mode == RailMode::LayerShell {
            let press = gtk::GestureClick::new();
            press.set_button(0);
            press.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak = Rc::downgrade(&rail);
            press.connect_pressed(move |_, _, _, _| {
                if let Some(rail) = weak.upgrade()
                    && rail.keyboard.pressed(rail.opening.is_open())
                {
                    crate::edge::layer::set_rail_keyboard(&rail.window, true);
                }
            });
            rail.window.add_controller(press);
        }
        let keys = gtk::EventControllerKey::new();
        {
            let weak = Rc::downgrade(&rail);
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape
                    && let Some(rail) = weak.upgrade()
                {
                    rail.cancel_close();
                    rail.opening.set(false);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        rail.window.add_controller(keys);

        if mode == RailMode::GnomeExtension {
            let (weak, opening) = (Rc::downgrade(app), Rc::downgrade(&rail.opening));
            rail.window.connect_map(move |_| {
                if let Some(app) = weak.upgrade()
                    && let Some(client) = app.edge.gnome.borrow().as_ref()
                {
                    client.send(gnome::Request::Place {
                        role: Role::Rail,
                        at: (-1, -1),
                        open: opening.upgrade().is_some_and(|opening| opening.is_open()),
                    });
                }
            });
        }

        rail.opening.reset();
        (rail.opening_place())(false);
        rail.window.present();
        rail
    }

    /// Sizes a new rail closed, the way `place(false)` does.
    fn opening_place(&self) -> impl Fn(bool) + '_ {
        move |open| {
            let root = self.window.child();
            if let Some(root) = root {
                let (width, height) =
                    geometry::rail_size(open, rail_monitor_height(&self.window, self.mode));
                root.set_size_request(width, height);
            }
        }
    }

    fn cancel_close(&self) {
        if let Some(source) = self.closing.take() {
            source.remove();
        }
    }

    /// Leaving an open rail closes it after the delay; leaving a closed one
    /// starts nothing.
    fn schedule_close(self: &Rc<Self>) {
        let Some(delay) = geometry::close_delay_on_leave(self.opening.is_open()) else {
            return;
        };
        self.cancel_close();
        let weak = Rc::downgrade(self);
        let source =
            glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
                if let Some(rail) = weak.upgrade() {
                    rail.closing.take();
                    rail.opening.set(false);
                }
            });
        self.closing.replace(Some(source));
    }

    fn select(&self, peek: Peek) {
        if self.peek.replace(peek) == peek {
            return;
        }
        if let Some(app) = self.app.upgrade() {
            self.render(&app);
        }
    }

    fn remove(&self) {
        self.cancel_close();
        self.opening.reset();
        if self.mode == RailMode::GnomeExtension
            && let Some(app) = self.app.upgrade()
            && let Some(client) = app.edge.gnome.borrow().as_ref()
        {
            client.send(gnome::Request::Release { role: Role::Rail });
        }
        self.window.destroy();
    }

    /// Draws the peek from the app's reads.
    fn render(&self, app: &Rc<App>) {
        let peek = self.peek.get();
        for (tile_peek, tile) in &self.tiles {
            if *tile_peek == peek {
                tile.add_css_class("tc-selected");
            } else {
                tile.remove_css_class("tc-selected");
            }
        }
        clear(&self.peek_body);
        self.peek_body.append(&peek_header(app, peek));
        // Private AI's peek is its header alone here: the Mac's peek draws
        // the NEAR AI tool buttons, Home's Credit and Balance tiles and a
        // small Private AI map under it, which this shell does not draw yet,
        // and no line or link in their place (owner, 2026-10-22,
        // `EdgeRailPeekCard`).
        if peek == Peek::Traces {
            traces_peek(app, &self.peek_body);
        }
        let _ = &self.revealer;
    }
}

/// One rail tile: a glyph button on the control tiers (`GlassRailTile`),
/// named for a screen reader and, on hover, beside it (`rail_tip`).
fn tile(child: &gtk::Widget, name: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(child)
        .width_request(glass::size::EDGE_RAIL_TILE)
        .height_request(glass::size::EDGE_RAIL_TILE)
        .build();
    button.add_css_class("tc-edge-rail-tile");
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    attach_rail_tip(&button, name);
    button
}

/// How long the pointer rests on a tile before its name shows: GTK's own
/// tooltip hover delay (`HOVER_TIMEOUT` in gtktooltip.c), which the
/// tooltip this replaces had.
const RAIL_TIP_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

/// The tile's name on hover, as the Mac's `.help(label)` gives it, drawn
/// beside the tile on the side away from the screen edge. Not a GTK
/// tooltip: GTK places one under its widget, and on KDE that covered the
/// next tile down (shot 72: Traces' over the NEAR AI tile). The rail is a
/// column, so beside it covers no tile. It takes no grab and the pointer
/// passes through it; leaving or pressing the tile takes it away.
fn attach_rail_tip(button: &gtk::Button, name: &str) {
    let label = gtk::Label::new(Some(name));
    label.add_css_class("tc-type-caption");
    let tip = gtk::Popover::builder()
        .child(&label)
        .position(gtk::PositionType::Left)
        .has_arrow(false)
        .autohide(false)
        .can_focus(false)
        .can_target(false)
        .build();
    tip.add_css_class("tc-edge-rail-tip");
    // The tile's name is already its accessible label; the tip is drawing.
    tip.set_accessible_role(gtk::AccessibleRole::Presentation);
    tip.set_parent(button);
    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let hide = {
        let tip = tip.downgrade();
        let pending = pending.clone();
        move || {
            if let Some(id) = pending.take() {
                id.remove();
            }
            if let Some(tip) = tip.upgrade() {
                tip.popdown();
            }
        }
    };
    let motion = gtk::EventControllerMotion::new();
    motion.connect_enter({
        let tip = tip.downgrade();
        let pending = pending.clone();
        let hide = hide.clone();
        move |_, _, _| {
            hide();
            let tip = tip.clone();
            let shown = Rc::downgrade(&pending);
            let id = glib::timeout_add_local_once(RAIL_TIP_DELAY, move || {
                if let Some(pending) = shown.upgrade() {
                    pending.take();
                }
                if let Some(tip) = tip.upgrade() {
                    tip.popup();
                }
            });
            pending.replace(Some(id));
        }
    });
    motion.connect_leave({
        let hide = hide.clone();
        move |_| hide()
    });
    button.add_controller(motion);
    button.connect_clicked({
        let hide = hide.clone();
        move |_| hide()
    });
    button.connect_destroy(move |_| {
        hide();
        tip.unparent();
    });
}

/// The name tip `attach_rail_tip` gave a tile.
#[cfg(test)]
fn rail_tip(button: &gtk::Button) -> Option<gtk::Popover> {
    let mut child = button.first_child();
    while let Some(widget) = child {
        if let Ok(tip) = widget.clone().downcast::<gtk::Popover>() {
            return Some(tip);
        }
        child = widget.next_sibling();
    }
    None
}

/// The peek's title, with Private AI's word right after it, and the
/// popover's buttons in the top right corner: Watching on Traces', then
/// Settings and Quit.
fn peek_header(app: &Rc<App>, peek: Peek) -> gtk::Box {
    let words = monitor_screens_copy();
    let header = gtk::Box::new(gtk::Orientation::Horizontal, glass::space::S2);
    let title = gtk::Label::builder()
        .label(peek.name())
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    title.add_css_class("tc-type-heading");
    title.set_accessible_role(gtk::AccessibleRole::Heading);
    header.append(&title);
    if peek == Peek::PrivateAi {
        let label = private_ai_label(app);
        label.set_margin_start(glass::space::S4);
        header.append(&label);
    }
    let spacer = gtk::Box::builder().hexpand(true).build();
    header.append(&spacer);
    if peek == Peek::Traces {
        header.append(&watching_button(app));
    }
    let settings = round_button(brand_glyph("gear"), words.settings_title);
    let weak = Rc::downgrade(app);
    settings.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            open_settings(&app, settings_section(peek));
        }
    });
    header.append(&settings);
    header.append(&quit_button(app));
    header
}

fn brand_glyph(name: &str) -> &'static str {
    glyph::named(name).unwrap_or(glyphs::SHIELD)
}

fn quit_button(app: &Rc<App>) -> gtk::Button {
    let quit = round_button(glyphs::POWER, monitor_screens_copy().quit);
    let weak = Rc::downgrade(app);
    quit.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            // The window's own close, so a quit that would stop Private AI
            // asks first, over the window.
            app.window.present();
            app.window.close();
        }
    });
    quit
}

/// Traces, compact: the watched folders as the tree lists them, every one,
/// in a list that scrolls past seven, each opening the Traces tab; then the
/// shared and kept graph. The Mac's rows also carry Submit and the
/// folder's menu; here they open the tab, where both are.
fn traces_peek(app: &Rc<App>, body: &gtk::Box) {
    let words = monitor_traces_copy();
    let tree = TracesTree::build(&app.entries.borrow(), &app.projects.borrow(), None);
    let folders: Vec<&FolderNode> = tree
        .folders
        .iter()
        .filter(|folder| folder.mode != Some(ProjectMode::Ignore))
        .collect();
    if !folders.is_empty() {
        let heading = gtk::Label::builder()
            .label(words.tree.tree_title)
            .xalign(0.0)
            .build();
        heading.add_css_class("tc-type-body-strong");
        heading.set_accessible_role(gtk::AccessibleRole::Heading);
        body.append(&heading);
        let rows: Vec<gtk::Button> = folders.iter().map(|f| folder_row(app, f)).collect();
        body.append(&folder_list(&rows));
    }
    body.append(&graph(app).root);
}

/// The folder list: every row in full up to seven, then it scrolls. Sized
/// once the rows are styled, from the tallest as drawn: a fixed pitch cut
/// the second row's line on GNOME and KDE, whose fonts set the two lines
/// taller than the Mac's row.
fn folder_list(rows: &[gtk::Button]) -> gtk::ScrolledWindow {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    for row in rows {
        list.append(row);
    }
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .build();
    let count = rows.len();
    let first = geometry::folder_list_height(count, 0);
    scroller.set_max_content_height(first);
    scroller.set_min_content_height(first);
    let rows: Vec<gtk::Button> = rows.to_vec();
    scroller.connect_realize(move |scroller| {
        let tallest = rows
            .iter()
            .map(|row| row.measure(gtk::Orientation::Vertical, -1).1)
            .max()
            .unwrap_or(0);
        let height = geometry::folder_list_height(count, tallest);
        scroller.set_max_content_height(height);
        scroller.set_min_content_height(height);
    });
    scroller
}

fn folder_row(app: &Rc<App>, folder: &FolderNode) -> gtk::Button {
    let row = folder_row_widget(folder);
    let weak = Rc::downgrade(app);
    row.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            open(&app, Peek::Traces.screen());
        }
    });
    row
}

/// A folder's row: its name over how many traces wait in it.
fn folder_row_widget(folder: &FolderNode) -> gtk::Button {
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .height_request(glass::size::LIST_ROW)
        .valign(gtk::Align::Center)
        .build();
    let title = gtk::Label::builder()
        .label(&folder.label)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    title.add_css_class("tc-type-body");
    let sub = gtk::Label::builder()
        .label(folder_sub(folder))
        .xalign(0.0)
        .build();
    sub.add_css_class("tc-type-caption");
    sub.add_css_class("tc-tertiary");
    column.append(&title);
    column.append(&sub);
    let row = gtk::Button::builder().child(&column).build();
    row.add_css_class("tc-edge-row");
    row
}

// --- The panel -------------------------------------------------------------

/// The popover's Private AI card (`MenuBarGlassPanel.privateAI`): its name
/// with its word beside it, drawn only while signed in to NEAR AI and only
/// with the core's name for it. The word opens Private AI's Settings
/// (`PrivateAIStatusLabel { open(.settings(.privateAI)) }`); the card
/// itself opens nothing. The Mac's NEAR AI tool buttons under it are not
/// drawn here yet.
fn render_private_ai(app: &Rc<App>, body: &gtk::Box, window: &gtk::Window) {
    let destination =
        trace_commons_contributor::private_inference_copy::private_inference_copy().destination;
    let weak = Rc::downgrade(app);
    let window = window.downgrade();
    let card = private_ai_card(
        app.private_inference.near_ai_signed_in.get(),
        destination,
        &private_ai_label(app),
        move |section| {
            if let Some(window) = window.upgrade() {
                window.set_visible(false);
            }
            if let Some(app) = weak.upgrade() {
                open_settings(&app, section);
            }
        },
    );
    if let Some(card) = card {
        body.append(&card);
    }
}

/// The card itself, or `None` while it is not shown: `word` is the state
/// word beside the name, and `open` is called with the Settings section a
/// press on it opens.
fn private_ai_card(
    signed_in: Option<bool>,
    destination: &str,
    word: &impl IsA<gtk::Widget>,
    open: impl Fn(Section) + 'static,
) -> Option<gtk::Box> {
    if !status::private_ai_card(signed_in, destination) {
        return None;
    }
    let card = gtk::Box::new(gtk::Orientation::Horizontal, glass::space::S4);
    card.add_css_class("tc-glass-card");
    let name = gtk::Label::builder()
        .label(destination)
        .xalign(0.0)
        .hexpand(true)
        .margin_top(glass::space::CARD_PADDING_VERTICAL)
        .margin_bottom(glass::space::CARD_PADDING_VERTICAL)
        .margin_start(glass::space::CARD_PADDING_HORIZONTAL)
        .build();
    name.add_css_class("tc-type-eyebrow");
    card.append(&name);
    let state = gtk::Button::builder()
        .child(word)
        .valign(gtk::Align::Center)
        .margin_end(glass::space::CARD_PADDING_HORIZONTAL)
        .build();
    state.add_css_class("flat");
    state.connect_clicked(move |_| open(Section::PrivateAI));
    card.append(&state);
    Some(card)
}

struct Panel {
    window: gtk::Window,
    mode: PanelMode,
    body: gtk::Box,
}

impl Panel {
    fn new(app: &Rc<App>, mode: PanelMode) -> Rc<Self> {
        let window = surface_window(
            app,
            (mode == PanelMode::GnomeExtension).then_some(Role::Panel),
            panel_title(mode),
        );
        window.set_resizable(false);
        window.set_decorated(mode == PanelMode::PlainWindow);
        window.set_hide_on_close(true);
        window.add_css_class("tc-edge-surface");
        window.update_property(&[gtk::accessible::Property::Label(crate::copy::APP_NAME)]);
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(geometry::PANEL_SPACING)
            // The Mac's 285 includes the padding round the rows.
            .width_request(geometry::PANEL_WIDTH - 2 * geometry::PANEL_PADDING)
            .build();
        let surface = gtk::Box::new(gtk::Orientation::Vertical, 0);
        surface.add_css_class("tc-glass-popover");
        surface.add_css_class("tc-menu-panel");
        body.set_margin_top(geometry::PANEL_PADDING);
        body.set_margin_bottom(geometry::PANEL_PADDING);
        body.set_margin_start(geometry::PANEL_PADDING);
        body.set_margin_end(geometry::PANEL_PADDING);
        surface.append(&body);
        window.set_child(Some(&surface));

        #[cfg(all(target_os = "linux", feature = "layer-shell"))]
        if mode == PanelMode::LayerShell {
            crate::edge::layer::make_panel(&window);
        }

        let keys = gtk::EventControllerKey::new();
        {
            let window = window.downgrade();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape
                    && let Some(window) = window.upgrade()
                {
                    window.set_visible(false);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        window.add_controller(keys);
        // A popover goes when the person turns to something else; a plain
        // window stays, as any window does.
        if mode != PanelMode::PlainWindow {
            window.connect_is_active_notify(|window| {
                if !window.is_active() && window.is_visible() {
                    window.set_visible(false);
                }
            });
        }
        let _ = app;
        Rc::new(Self { window, mode, body })
    }

    fn show(&self, app: &Rc<App>, at: Option<(i32, i32)>) {
        #[cfg(all(target_os = "linux", feature = "layer-shell"))]
        if self.mode == PanelMode::LayerShell
            && let Some(monitor) = panel_monitor(app, at)
        {
            let anchor = geometry::panel_anchor(at, monitor_rect(&monitor));
            crate::edge::layer::place_panel(&self.window, anchor, &monitor);
        }
        app.refresh();
        self.render(app);
        self.window.present();
        if self.mode == PanelMode::GnomeExtension
            && let Some(client) = app.edge.gnome.borrow().as_ref()
        {
            let at = at.filter(|&point| point != (0, 0)).unwrap_or((-1, -1));
            client.send(gnome::Request::Place {
                role: Role::Panel,
                at,
                open: true,
            });
        }
    }

    /// The popover's rows (`MenuBarGlassPanel.body`): the top row, Private
    /// AI, the legend and graph, and the way into the app. The Mac's terms
    /// hold row, suggestion, estimate and glance card, and its manual
    /// update check, are not drawn here yet.
    fn render(&self, app: &Rc<App>) {
        let words = monitor_screens_copy();
        clear(&self.body);

        // The top row: the mark, then Watching, Settings and Quit.
        let top = gtk::Box::new(gtk::Orientation::Horizontal, glass::space::S2);
        let mark = super::mark::framed(16);
        mark.set_margin_start(glass::space::S4);
        top.append(&mark);
        top.append(&gtk::Box::builder().hexpand(true).build());
        top.append(&watching_button(app));
        let settings = round_button(brand_glyph("gear"), words.settings_title);
        {
            let weak = Rc::downgrade(app);
            let window = self.window.downgrade();
            settings.connect_clicked(move |_| {
                if let Some(window) = window.upgrade() {
                    window.set_visible(false);
                }
                if let Some(app) = weak.upgrade() {
                    open(&app, super::SETTINGS_SCREEN);
                }
            });
        }
        top.append(&settings);
        top.append(&quit_button(app));
        self.body.append(&top);

        render_private_ai(app, &self.body, &self.window);

        self.body.append(&graph(app).root);

        // The way into the app, under a hairline.
        self.body
            .append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let open_app = gtk::Button::builder().label(words.open_app).build();
        open_app.add_css_class("tc-edge-row");
        {
            let weak = Rc::downgrade(app);
            let window = self.window.downgrade();
            open_app.connect_clicked(move |_| {
                if let Some(window) = window.upgrade() {
                    window.set_visible(false);
                }
                if let Some(app) = weak.upgrade() {
                    app.window.present();
                }
            });
        }
        self.body.append(&open_app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rail_glyphs_parse() {
        for svg in [
            glyphs::SHIELD,
            glyphs::POWER,
            glyphs::EYE,
            glyphs::EYE_SLASH,
        ] {
            let parsed = glyph::Glyph::parse(svg).expect("a glyph parses");
            assert_eq!(parsed.view_box, (24.0, 24.0));
            assert!(parsed.stroke.is_some());
        }
        assert!(glyph::named("gear").is_some());
    }

    #[test]
    fn the_trace_nodes_are_the_macs_t() {
        assert_eq!(
            trace_nodes::NODES,
            [(0.16, 0.3), (0.5, 0.3), (0.84, 0.3), (0.5, 0.84)]
        );
        assert_eq!(trace_nodes::LINKS, [(0, 2), (1, 3)]);
        assert_eq!(
            (trace_nodes::NODE_RADIUS, trace_nodes::LINE_WIDTH),
            (0.12, 0.08)
        );
    }

    #[test]
    fn the_peeks_are_named_by_the_apps_tabs_and_open_them() {
        let words = monitor_screens_copy();
        assert_eq!(Peek::ALL, [Peek::PrivateAi, Peek::Traces]);
        assert_eq!(Peek::PrivateAi.name(), words.settings_nav.private_ai);
        assert_eq!(Peek::Traces.name(), words.shell.tab_traces);
        let screens: Vec<&str> = super::super::screens()
            .iter()
            .map(|(name, _)| *name)
            .collect();
        for peek in Peek::ALL {
            assert!(screens.contains(&peek.screen()), "{peek:?}");
        }
    }

    /// The production half of this file, for the checks below that read
    /// what a function calls.
    fn production() -> &'static str {
        include_str!("edge_surfaces.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("production code")
    }

    fn body_of(name: &str) -> &'static str {
        let start = production()
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("fn {name}"));
        let rest = &production()[start..];
        &rest[..rest.find("\n}\n").expect("end of fn")]
    }

    /// The Settings button in each peek opens the Mac's section
    /// (`EdgeRailSettings.section`): Private AI's own, and for Traces
    /// General, where the rail's switch is, as on the Mac.
    #[test]
    fn each_peeks_settings_open_the_macs_section() {
        assert_eq!(settings_section(Peek::PrivateAi), Section::PrivateAI);
        assert_eq!(settings_section(Peek::Traces), Section::General);
        let mac = include_str!(
            "../../../../macos/Sources/TraceCommonsApp/Views/EdgeRail/EdgeRailView.swift"
        );
        assert!(mac.contains("peek == .privateAI ? .privateAI : .general"));
        let settings = include_str!("settings.rs");
        assert!(
            settings.contains("sections.body(Section::General).append(&edge_rail.card)"),
            "the rail's switch lives under General"
        );
        assert!(body_of("peek_header").contains("open_settings(&app, settings_section(peek))"));
    }

    /// The Private AI peek has no line or link under its header (owner,
    /// 2026-10-22, `EdgeRailPeekCard`), and the Mac's
    /// `MonitorEdgeRailCopy.open(section:)` has no caller.
    #[test]
    fn the_private_ai_peek_draws_nothing_under_its_header() {
        assert!(!production().contains("open_section"));
        assert!(!production().contains("fn private_ai_peek("));
        assert!(body_of("render").contains("traces_peek"));
    }

    /// Opening the app from the rail puts the rail away first
    /// (`EdgeRailView.open`).
    #[test]
    fn opening_the_app_puts_the_rail_away_first() {
        let open = body_of("open");
        let away = open.find("put_rail_away(app)").expect("closes the rail");
        let shown = open.find("open_screen").expect("opens the screen");
        assert!(away < shown);
        let settings = body_of("open_settings");
        assert!(settings.contains("put_rail_away(app)"));
    }

    /// The popover's Private AI card shows only while signed in and with
    /// the core's name for it, and its word opens Private AI's Settings, not
    /// the tab (`PrivateAIStatusLabel { open(.settings(.privateAI)) }`).
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_the_popovers_private_ai_card_follows_the_mac() {
        gtk::init().expect("GTK display");
        let destination = "Private AI";
        for (signed_in, name) in [
            (None, destination),
            (Some(false), destination),
            (Some(true), ""),
        ] {
            let card = private_ai_card(signed_in, name, &gtk::Label::new(Some("On")), |_| {
                panic!("a card that is not shown was opened")
            });
            assert!(card.is_none(), "{signed_in:?} {name:?}");
        }
        let opened: Rc<RefCell<Vec<Section>>> = Rc::default();
        let card = {
            let opened = Rc::clone(&opened);
            private_ai_card(
                Some(true),
                destination,
                &gtk::Label::new(Some("On")),
                move |section| opened.borrow_mut().push(section),
            )
        }
        .expect("shown while signed in");
        let name = card
            .first_child()
            .and_downcast::<gtk::Label>()
            .expect("the name first");
        assert_eq!(name.text(), destination);
        let word = card
            .last_child()
            .and_downcast::<gtk::Button>()
            .expect("the word, a button");
        assert_eq!(
            word.child()
                .and_downcast::<gtk::Label>()
                .map(|label| label.text().to_string())
                .as_deref(),
            Some("On")
        );
        word.emit_clicked();
        assert_eq!(*opened.borrow(), vec![Section::PrivateAI]);
    }

    /// On GNOME the rail and the panel are titled with the app's name, as
    /// Alt-Tab shows them (owner, 2026-11-18); elsewhere the rail keeps its
    /// name and the panel the app's.
    #[test]
    fn the_windows_are_titled_with_the_apps_name_on_gnome() {
        assert_eq!(rail_title(RailMode::GnomeExtension), crate::copy::APP_NAME);
        assert_eq!(
            panel_title(PanelMode::GnomeExtension),
            crate::copy::APP_NAME
        );
        assert_eq!(
            rail_title(RailMode::LayerShell),
            monitor_screens_copy().edge_rail.rail_label
        );
        assert_eq!(panel_title(PanelMode::PlainWindow), crate::copy::APP_NAME);
        assert_eq!(panel_title(PanelMode::LayerShell), crate::copy::APP_NAME);
    }

    #[test]
    fn a_folder_row_counts_its_waiting_traces_in_the_cores_words() {
        let counts = monitor_traces_copy().counts;
        let mut folder = FolderNode::new("p", "project", Some(ProjectMode::Ask));
        assert_eq!(
            folder_sub(&folder),
            counts.sessions_waiting.replace("{count}", "0")
        );
        folder.sessions.push(Default::default());
        assert_eq!(folder_sub(&folder), counts.sessions_waiting_one);
        folder.sessions.push(Default::default());
        assert_eq!(
            folder_sub(&folder),
            counts.sessions_waiting.replace("{count}", "2")
        );
        folder.mode = Some(ProjectMode::Ignore);
        assert_eq!(
            folder_sub(&folder),
            monitor_traces_copy().tree.ignored_folder
        );
    }

    /// A modal dialog of the app (first run, terms, a preview) grabs input
    /// for its window group, and GTK then drops every key sent to a window
    /// of that group outside the dialog. Seen on GNOME 46 on 2026-11-18:
    /// with the first-run window open, the clicked rail held the keyboard
    /// yet Escape and Tab did nothing. The rail and the panel stand apart
    /// from the app's windows, as the Mac's panels do from its sheets, so
    /// each is in a group of its own.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_an_open_dialog_leaves_the_rail_and_panel_their_keys() {
        gtk::init().expect("GTK display");
        let application = gtk::Application::new(
            Some("ai.tracecommons.Contributor.GroupTest"),
            gtk::gio::ApplicationFlags::NON_UNIQUE,
        );
        application
            .register(None::<&gtk::gio::Cancellable>)
            .expect("registered");
        let main = gtk::ApplicationWindow::builder()
            .application(&application)
            .build();
        let dialog = gtk::Window::builder()
            .transient_for(&main)
            .modal(true)
            .build();
        for role in [None, Some(Role::Rail), Some(Role::Panel)] {
            let window = new_surface_window(Some(&application), role, crate::copy::APP_NAME);
            assert!(window.has_group(), "{role:?}: in the app's shared group");
            assert_ne!(
                window.group(),
                dialog.group(),
                "{role:?}: shares the dialog's group, so its grab takes the keys"
            );
            assert_ne!(window.group(), main.group(), "{role:?}");
            window.destroy();
        }
        dialog.destroy();
        main.destroy();
    }

    /// A rail tile names itself on hover, as the Mac's `.help(label)`
    /// does: after GTK's own hover delay, beside the tile on the side away
    /// from the screen edge, so it covers no tile above or below it. GTK's
    /// own tooltip is placed under its widget, and on KDE it covered the
    /// next tile (shot 72: Traces' over the NEAR AI tile); the round before
    /// this one dropped the name instead. Leaving the tile, or pressing it,
    /// takes the name away.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_a_rail_tile_names_itself_beside_it_after_the_hover_delay() {
        gtk::init().expect("GTK display");
        let context = glib::MainContext::default();
        let wait = |ms: u64| {
            let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
            while std::time::Instant::now() < end {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        let column = gtk::Box::new(gtk::Orientation::Vertical, glass::space::S1);
        let names = [Peek::PrivateAi.name(), Peek::Traces.name(), "Open"];
        let tiles: Vec<gtk::Button> = names
            .iter()
            .map(|name| tile(&gtk::Label::new(None).upcast(), name))
            .collect();
        for tile in &tiles {
            column.append(tile);
        }
        let window = gtk::Window::builder().child(&column).build();
        window.present();
        wait(100);
        for (tile, name) in tiles.iter().zip(names) {
            assert!(
                !tile.has_tooltip(),
                "{name}: no GTK tooltip, which GTK places under the tile"
            );
            assert_eq!(
                tile.accessible_role(),
                gtk::AccessibleRole::Button,
                "{name}"
            );
            let tip = rail_tip(tile).unwrap_or_else(|| panic!("{name} has a name tip"));
            let label = tip
                .child()
                .and_downcast::<gtk::Label>()
                .expect("the tip is a label");
            assert_eq!(label.text(), name);
            assert_eq!(
                tip.position(),
                gtk::PositionType::Left,
                "{name}: beside the tile, toward the peek, never under it"
            );
            assert!(!tip.has_arrow());
            assert!(!tip.is_autohide(), "{name}: it takes no grab");
            assert!(!tip.can_target(), "{name}: the pointer passes through it");

            let motion = tile
                .observe_controllers()
                .into_iter()
                .filter_map(|c| c.ok())
                .find_map(|c| c.downcast::<gtk::EventControllerMotion>().ok())
                .expect("the tile watches the pointer");
            motion.emit_by_name::<()>("enter", &[&4.0f64, &4.0f64]);
            wait(RAIL_TIP_DELAY.as_millis() as u64 / 2);
            assert!(!tip.is_visible(), "{name}: not before the hover delay");
            wait(RAIL_TIP_DELAY.as_millis() as u64);
            assert!(tip.is_visible(), "{name}: shown after the hover delay");
            motion.emit_by_name::<()>("leave", &[]);
            wait(50);
            assert!(!tip.is_visible(), "{name}: gone when the pointer leaves");

            // A press takes it away too, and a pending one never shows.
            motion.emit_by_name::<()>("enter", &[&4.0f64, &4.0f64]);
            tile.emit_clicked();
            wait(RAIL_TIP_DELAY.as_millis() as u64 * 2);
            assert!(!tip.is_visible(), "{name}: a press cancels it");
            motion.emit_by_name::<()>("leave", &[]);
        }
        window.destroy();
    }

    /// The Traces peek's folder list draws every row whole, its name and
    /// the line under it, at the peek's width on this desktop's fonts. On
    /// the branch before this fix the list was a fixed 42 a row, and the
    /// second row's line was cut under the graph's chips (GNOME shot 79,
    /// KDE shot 72).
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_the_folder_list_draws_every_row_whole() {
        adw::init().expect("GTK display");
        style::install();
        let session = |id: &str| crate::model::QueueEntry {
            entry_id: id.into(),
            state: "pending".into(),
            ..Default::default()
        };
        let mut api = FolderNode::new("api", "api", Some(ProjectMode::Ask));
        api.sessions = vec![session("a"), session("b")];
        let mut web = FolderNode::new(
            "web",
            "trace-commons-contributor-web",
            Some(ProjectMode::Ask),
        );
        web.sessions = vec![session("c")];
        let rows: Vec<gtk::Button> = [&api, &web].into_iter().map(folder_row_widget).collect();
        let list = folder_list(&rows);
        let peek = gtk::Box::new(gtk::Orientation::Vertical, 0);
        peek.add_css_class("tc-glass-window");
        peek.append(&list);
        // What sits under the list in the peek, as the graph's chips do.
        peek.append(&gtk::Label::new(Some("shared")));
        let window = gtk::Window::builder()
            .default_width(geometry::OPEN_WIDTH)
            .child(&peek)
            .build();
        window.present();
        let context = glib::MainContext::default();
        let end = std::time::Instant::now() + std::time::Duration::from_millis(400);
        while std::time::Instant::now() < end {
            while context.pending() {
                context.iteration(false);
            }
        }
        let shown = list.height() as f32;
        for row in &rows {
            let at = row.compute_bounds(&list).expect("laid out");
            assert!(
                at.y() + at.height() <= shown,
                "a row reaches {} of a list {shown} tall",
                at.y() + at.height()
            );
            let sub = row
                .child()
                .and_then(|column| column.last_child())
                .expect("the line under the name");
            let line = sub.compute_bounds(&list).expect("its line is laid out");
            assert!(
                sub.is_mapped() && line.y() + line.height() <= shown,
                "its line is cut"
            );
        }
        window.destroy();
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_rail_switch_is_hidden_until_the_session_is_known() {
        gtk::init().expect("GTK display");
        let rail = monitor_screens_copy().edge_rail;
        let switch = rail_switch();
        assert!(
            !switch.card.is_visible(),
            "shown before the session is known"
        );
        assert!(
            !switch.switch.is_active(),
            "on before the stored choice is read"
        );
        let mut labels = Vec::new();
        let mut child = switch.card.first_child();
        while let Some(widget) = child {
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                labels.push(label.text().to_string());
            }
            child = widget.next_sibling();
        }
        assert!(labels.contains(&rail.rail_label.to_string()));
        assert!(labels.contains(&rail.setting_caption.to_string()));
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_rail_slides_and_fades_under_reduced_motion() {
        gtk::init().expect("GTK display");
        let settings = gtk::Settings::default().expect("settings");
        let previous = settings.is_gtk_enable_animations();
        // The revealer only shows and hides; the slide is `SlideX`'s.
        let revealer = gtk::Revealer::new();
        follow_motion(&revealer);
        assert_eq!(revealer.transition_duration(), 0);
        assert_eq!(
            revealer.transition_type(),
            gtk::RevealerTransitionType::None
        );
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.set_size_request(200, 40);
        let slide = super::super::edge_motion::SlideX::new(&row);
        let window = gtk::Window::new();
        window.set_child(Some(&slide));
        window.present();
        settings.set_gtk_enable_animations(true);
        slide.slide(true, 220.0);
        assert_eq!((slide.offset(), slide.opacity()), (220.0, 0.0));
        let between = Cell::new(false);
        let context = glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while slide.sliding() && std::time::Instant::now() < deadline {
            if slide.offset() > 1.0 && slide.offset() < 219.0 && slide.opacity() > 0.0 {
                between.set(true);
            }
            if !context.iteration(false) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        assert!(between.get(), "the rail jumped into place");
        assert_eq!((slide.offset(), slide.opacity()), (0.0, 1.0));
        settings.set_gtk_enable_animations(false);
        slide.slide(false, 220.0);
        assert_eq!((slide.offset(), slide.opacity()), (220.0, 0.0));
        settings.set_gtk_enable_animations(previous);
        window.destroy();
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_compact_graph_is_shown_without_its_controls() {
        gtk::init().expect("GTK display");
        let graph = GraphFooter::new();
        graph.compact(geometry::GRAPH_HEIGHT);
        assert!(graph.root.reveals_child());
        assert!(!graph.root.is_moving() && graph.root.fraction() == 1.0);
        // The focus pill sits in the row of controls, which is hidden.
        let controls = graph.focus.parent().expect("the controls' row");
        assert!(!controls.is_visible());
    }

    /// The session's own facts: on weston, which has no layer-shell, the
    /// rail is offered only through the GNOME extension; on a compositor
    /// with it, through layer-shell. Either way the rule and the library
    /// agree.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_detection_reads_this_sessions_backend() {
        gtk::init().expect("GTK display");
        let display = gtk::gdk::Display::default().expect("display");
        let facts = capability::detect(&display, false);
        let modes = capability::modes(facts);
        if facts.layer_shell {
            assert_eq!(modes.rail, RailMode::LayerShell);
        } else {
            assert_eq!(modes.rail, RailMode::Unavailable);
            assert_eq!(modes.panel, PanelMode::PlainWindow);
        }
        if std::env::var_os("WAYLAND_DISPLAY").is_some()
            && std::env::var("GDK_BACKEND").as_deref() == Ok("wayland")
        {
            assert_eq!(facts.backend, capability::Backend::Wayland);
        }
    }
}
