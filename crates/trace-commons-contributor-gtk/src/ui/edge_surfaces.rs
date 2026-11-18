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
    let (client, present) = gnome::spawn();
    app.edge.gnome.replace(Some(client));
    {
        let app = Rc::clone(app);
        glib::spawn_future_local(async move {
            let extension = present.recv().await.unwrap_or(false);
            let Some(display) = gtk::gdk::Display::default() else {
                return;
            };
            let modes = capability::modes(capability::detect(&display, extension));
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
    // Neither window keeps the process alive, or outlives the main one.
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
        _ => {
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

/// The slide's length: the token's quarter second, or none while the desktop
/// has animations off, where the rail fades instead of sliding, as the
/// Mac's does under Reduce Motion.
fn follow_motion(revealer: &gtk::Revealer) {
    let apply = |revealer: &gtk::Revealer, animate: bool| {
        revealer.set_transition_duration(if animate { glass::motion::SLIDE_MS } else { 0 });
        revealer.set_transition_type(if animate {
            gtk::RevealerTransitionType::SlideLeft
        } else {
            gtk::RevealerTransitionType::Crossfade
        });
    };
    match gtk::Settings::default() {
        Some(settings) => {
            apply(revealer, settings.is_gtk_enable_animations());
            let weak = revealer.downgrade();
            settings.connect_gtk_enable_animations_notify(move |settings| {
                if let Some(revealer) = weak.upgrade() {
                    apply(&revealer, settings.is_gtk_enable_animations());
                }
            });
        }
        None => apply(revealer, false),
    }
}

/// The first monitor's height, for the rail's tallest size.
fn monitor_size(window: &gtk::Window) -> (i32, i32) {
    let display = WidgetExt::display(window);
    let monitor = window
        .surface()
        .and_then(|surface| display.monitor_at_surface(&surface))
        .or_else(|| {
            display
                .monitors()
                .item(0)
                .and_then(|item| item.downcast::<gtk::gdk::Monitor>().ok())
        });
    monitor.map_or((1920, 1080), |monitor| {
        let geometry = monitor.geometry();
        (geometry.width(), geometry.height())
    })
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

/// Opens the window at a screen, and puts the surface that asked away.
fn open(app: &Rc<App>, screen: &str) {
    app.open_screen(screen);
    app.window.present();
}

// --- The rail --------------------------------------------------------------

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
    app: std::rc::Weak<App>,
}

impl Rail {
    fn new(app: &Rc<App>, mode: RailMode) -> Rc<Self> {
        let words = monitor_screens_copy();
        let rail_words = &words.edge_rail;
        let window = gtk::Window::builder()
            .title(rail_words.rail_label)
            .decorated(false)
            .resizable(false)
            .build();
        window.add_css_class("tc-edge-surface");
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
        let revealer = gtk::Revealer::builder().child(&open_row).build();
        follow_motion(&revealer);

        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.append(&zone);
        root.append(&revealer);
        window.set_child(Some(&root));

        #[cfg(all(target_os = "linux", feature = "layer-shell"))]
        if mode == RailMode::LayerShell {
            crate::edge::layer::make_rail(&window);
        }

        let opening = {
            let next_turn: crate::edge::opening::NextTurn = Box::new(|step| {
                glib::idle_add_local_once(step);
            });
            let animate: crate::edge::opening::Animate = {
                let revealer = revealer.clone();
                Box::new(move |open, done| {
                    revealer.set_reveal_child(open);
                    if open {
                        done();
                    } else {
                        let duration = u64::from(revealer.transition_duration());
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(duration + 16),
                            done,
                        );
                    }
                })
            };
            let place: crate::edge::opening::Place = {
                let (window, root, zone) = (window.clone(), root.clone(), zone.clone());
                let weak_app = Rc::downgrade(app);
                Box::new(move |open| {
                    zone.set_visible(!open);
                    let (_, height) = monitor_size(&window);
                    let (width, height) = geometry::rail_size(open, height);
                    root.set_size_request(width, height);
                    if !open {
                        // GTK keeps a window at its largest size unless asked.
                        window.set_default_size(1, 1);
                    }
                    #[cfg(all(target_os = "linux", feature = "layer-shell"))]
                    if mode == RailMode::LayerShell {
                        crate::edge::layer::set_rail_keyboard(&window, open);
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
                    if !rail.opening.is_open() {
                        rail.opening.set(true);
                    }
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
            let weak = Rc::downgrade(app);
            rail.window.connect_map(move |window| {
                if let Some(app) = weak.upgrade()
                    && let Some(client) = app.edge.gnome.borrow().as_ref()
                {
                    client.send(gnome::Request::Place {
                        title: window.title().map(|t| t.to_string()).unwrap_or_default(),
                        role: Role::Rail,
                        at: (-1, -1),
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
                let (_, height) = monitor_size(&self.window);
                let (width, height) = geometry::rail_size(open, height);
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
            client.send(gnome::Request::Release {
                title: monitor_screens_copy().edge_rail.rail_label.to_string(),
            });
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
        match peek {
            Peek::PrivateAi => private_ai_peek(app, &self.peek_body),
            Peek::Traces => traces_peek(app, &self.peek_body),
        }
        let _ = &self.revealer;
    }
}

/// One rail tile: a glyph button on the control tiers (`GlassRailTile`).
fn tile(child: &gtk::Widget, name: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(child)
        .width_request(glass::size::EDGE_RAIL_TILE)
        .height_request(glass::size::EDGE_RAIL_TILE)
        .tooltip_text(name)
        .build();
    button.add_css_class("tc-edge-rail-tile");
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    button
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
            open(&app, super::SETTINGS_SCREEN);
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

/// Private AI, compact: the listener's sentence and the port it answers
/// on, as the Private AI page states them, and a row into the tab. The
/// Mac's peek also draws the NEAR AI tool buttons, Home's Credit and
/// Balance tiles and a small Private AI map; this shell does not draw them
/// here yet.
fn private_ai_peek(app: &Rc<App>, body: &gtk::Box) {
    let reported = app.private_inference.reported.borrow().clone();
    let label = reported.as_deref().unwrap_or("");
    let card = gtk::Box::new(gtk::Orientation::Vertical, glass::space::S2);
    card.add_css_class("tc-glass-card");
    card.set_margin_top(0);
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(glass::space::S2)
        .margin_top(glass::space::CARD_PADDING_VERTICAL)
        .margin_bottom(glass::space::CARD_PADDING_VERTICAL)
        .margin_start(glass::space::CARD_PADDING_HORIZONTAL)
        .margin_end(glass::space::CARD_PADDING_HORIZONTAL)
        .build();
    inner.append(&super::private_inference::tone_row(
        crate::copy::private_inference_state_line(label),
        super::private_inference::indicator_tone(crate::copy::private_inference_state_tone(label)),
    ));
    card.append(&inner);
    body.append(&card);
    body.append(&open_row(app, Peek::PrivateAi));
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
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        for folder in &folders {
            list.append(&folder_row(app, folder));
        }
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(geometry::folder_list_height(folders.len()))
            .max_content_height(geometry::folder_list_height(folders.len()))
            .child(&list)
            .build();
        body.append(&scroller);
    }
    body.append(&graph(app).root);
}

fn folder_row(app: &Rc<App>, folder: &FolderNode) -> gtk::Button {
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
    let weak = Rc::downgrade(app);
    row.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            open(&app, Peek::Traces.screen());
        }
    });
    row
}

/// A peek's way into its tab, in the rail's words.
fn open_row(app: &Rc<App>, peek: Peek) -> gtk::Button {
    let rail = monitor_screens_copy().edge_rail;
    let text = rail.open_section.replace("{section}", peek.name());
    let label = rail.open_section_label.replace("{section}", peek.name());
    let row = gtk::Button::builder()
        .label(&text)
        .halign(gtk::Align::Start)
        .build();
    row.add_css_class("tc-edge-row");
    row.update_property(&[gtk::accessible::Property::Label(&label)]);
    let weak = Rc::downgrade(app);
    row.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            open(&app, peek.screen());
        }
    });
    row
}

// --- The panel -------------------------------------------------------------

struct Panel {
    window: gtk::Window,
    mode: PanelMode,
    body: gtk::Box,
}

impl Panel {
    fn new(app: &Rc<App>, mode: PanelMode) -> Rc<Self> {
        let window = gtk::Window::builder()
            .title(crate::copy::APP_NAME)
            .resizable(false)
            .decorated(mode == PanelMode::PlainWindow)
            .hide_on_close(true)
            .build();
        window.add_css_class("tc-edge-surface");
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(geometry::PANEL_SPACING)
            .width_request(geometry::PANEL_WIDTH)
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
        if self.mode == PanelMode::LayerShell {
            let monitor = monitor_size(&app.window);
            crate::edge::layer::place_panel(&self.window, geometry::panel_anchor(at, monitor));
        }
        app.refresh();
        self.render(app);
        self.window.present();
        if self.mode == PanelMode::GnomeExtension
            && let Some(client) = app.edge.gnome.borrow().as_ref()
        {
            let at = at.filter(|&point| point != (0, 0)).unwrap_or((-1, -1));
            client.send(gnome::Request::Place {
                title: crate::copy::APP_NAME.to_string(),
                role: Role::Panel,
                at,
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

        // Private AI: its name with its word beside it, opening its tab.
        let private = gtk::Box::new(gtk::Orientation::Horizontal, glass::space::S4);
        private.add_css_class("tc-glass-card");
        let name = gtk::Label::builder()
            .label(
                trace_commons_contributor::private_inference_copy::private_inference_copy()
                    .destination,
            )
            .xalign(0.0)
            .hexpand(true)
            .margin_top(glass::space::CARD_PADDING_VERTICAL)
            .margin_bottom(glass::space::CARD_PADDING_VERTICAL)
            .margin_start(glass::space::CARD_PADDING_HORIZONTAL)
            .build();
        name.add_css_class("tc-type-eyebrow");
        private.append(&name);
        let state = private_ai_label(app);
        state.set_margin_end(glass::space::CARD_PADDING_HORIZONTAL);
        private.append(&state);
        let private_row = gtk::Button::builder().child(&private).build();
        private_row.add_css_class("tc-edge-row");
        {
            let weak = Rc::downgrade(app);
            let window = self.window.downgrade();
            private_row.connect_clicked(move |_| {
                if let Some(window) = window.upgrade() {
                    window.set_visible(false);
                }
                if let Some(app) = weak.upgrade() {
                    open(&app, super::PRIVATE_INFERENCE_SCREEN);
                }
            });
        }
        self.body.append(&private_row);

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
        let revealer = gtk::Revealer::new();
        settings.set_gtk_enable_animations(true);
        follow_motion(&revealer);
        assert_eq!(revealer.transition_duration(), glass::motion::SLIDE_MS);
        assert_eq!(
            revealer.transition_type(),
            gtk::RevealerTransitionType::SlideLeft
        );
        settings.set_gtk_enable_animations(false);
        assert_eq!(revealer.transition_duration(), 0);
        assert_eq!(
            revealer.transition_type(),
            gtk::RevealerTransitionType::Crossfade
        );
        settings.set_gtk_enable_animations(previous);
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_compact_graph_is_shown_without_its_controls() {
        gtk::init().expect("GTK display");
        let graph = GraphFooter::new();
        graph.compact(geometry::GRAPH_HEIGHT);
        assert!(graph.root.reveals_child());
        assert_eq!(graph.root.transition_duration(), 0);
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
