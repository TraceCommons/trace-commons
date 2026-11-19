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
//! 4. the popover notices, each with the Mac's X inside it, laid over the
//!    top of the tab's screen as the Mac's peeking stack (`notices`);
//! 5. the tab's screen, which fits the pane's 440 and scrolls itself;
//! 6. the graph footer, on Traces.
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
//! - The toolbar toggles and the gear carry the Mac's own glyphs
//!   (`glyph`), exported from its path data, and every pane, the tab strip
//!   and the toolbar capsule are painted with the glass tiers
//!   (`brand_tokens::GLASS_*_CSS`): opaque, as the Mac draws them under
//!   Reduce Transparency.
//! - Settings' own dialogs and toasts still belong to the main window.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use trace_commons_contributor::preview_copy::monitor_screens_copy;

use super::graph_footer::{GraphData, GraphFooter};
use super::inspector::{Inspector, Selection};
use super::map::TraceMap;
use super::map_scene::MapStyle;
use super::notices::{self, Notice};
use super::pane_layout::{self, Visibility};
use super::prefs::PanePrefs;
use super::tabs::{self, Back, Location, MapView, Route, Tab, TracesPage};
use super::three_pane::ThreePane;
use super::{demand, glass_paint, glyph, graph_model};
use crate::ui::style::brand_tokens;
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
    /// Each notice's card, in the order they are stacked, the front first
    /// (`Notice::ALL`).
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
    /// The opened record's contribution details, read once per opening
    /// and kept across redraws, so a queue or History refresh does not
    /// read the server again or flash its reading line.
    history_detail_slot: RefCell<Option<(String, gtk::Box)>>,
    /// The notices closed with their X until the window is opened again.
    closed: Rc<RefCell<notices::Closed>>,
    missions: bool,
}

/// One notice's place in the stack: the slot `notices::shown` places, the
/// frame the stack shows while the card is in front, the card, and the
/// card's X.
struct NoticeSlot {
    notice: Notice,
    slot: gtk::Box,
    frame: gtk::Overlay,
    card: gtk::Widget,
    close: gtk::Button,
}

/// The stack's own ground, the pane's colour, across the whole width of the
/// pane from its top to the foot of the last card peeking out. The cards
/// keep their own column, which on Home is narrower than the screen's; on
/// the Mac the stack is exactly the tab's width and nothing of the screen
/// shows beside it, and this ground does the same here, so no sliver of a
/// covered heading shows at a card's side.
const NOTICE_BACKDROP_CLASS: &str = "tc-notice-backdrop";

/// Marks the surface a closable card keeps room in for its X
/// (`.tc-notice-closable .tc-notice-surface` in `style.css`): the surface
/// alone, not every card nested in it.
const NOTICE_SURFACE_CLASS: &str = "tc-notice-surface";

/// The window's class while the desktop asks for high contrast. The
/// generated glass sheet strokes the tiers that carry edge layers under it.
const HIGH_CONTRAST_CLASS: &str = "tc-high-contrast";

/// The tab the ember glows under: the shown tab, which is the one active
/// and visible (a hidden selected tab is drawn as the one shown).
fn glowing_tab(tabs: &[gtk::ToggleButton]) -> Option<&gtk::ToggleButton> {
    tabs.iter()
        .find(|button| button.is_active() && button.is_visible())
}

/// Keep the window's high-contrast class in step with the desktop
/// (`AdwStyleManager:high-contrast`), as the Mac follows Increase Contrast.
fn follow_high_contrast(window: &adw::ApplicationWindow) {
    let manager = adw::StyleManager::default();
    let apply = |window: &adw::ApplicationWindow, on: bool| {
        if on {
            window.add_css_class(HIGH_CONTRAST_CLASS);
        } else {
            window.remove_css_class(HIGH_CONTRAST_CLASS);
        }
    };
    apply(window, manager.is_high_contrast());
    let weak = window.downgrade();
    manager.connect_high_contrast_notify(move |manager| {
        if let Some(window) = weak.upgrade() {
            apply(&window, manager.is_high_contrast());
        }
    });
}

/// Puts [`NOTICE_SURFACE_CLASS`] on `card`'s surface while it carries an X
/// and takes it off everything else in `card`. Changes a class only when it
/// is wrong, so the card's own class watcher is not woken for nothing.
fn mark_surface(card: &gtk::Widget, closable: bool) {
    let surface = closable.then(|| card_surface(card)).flatten();
    let mut all = vec![card.clone()];
    let mut index = 0;
    while index < all.len() {
        let mut child = all[index].first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            all.push(next);
        }
        index += 1;
    }
    for widget in all {
        let wanted = surface.as_ref() == Some(&widget);
        if widget.has_css_class(NOTICE_SURFACE_CLASS) != wanted {
            if wanted {
                widget.add_css_class(NOTICE_SURFACE_CLASS);
            } else {
                widget.remove_css_class(NOTICE_SURFACE_CLASS);
            }
        }
    }
}

/// Has GTK fetch a fresh size-request mode for `card`, everything in it,
/// and each ancestor up to `top`, deepest first.
///
/// GTK 4.14 (Ubuntu 24.04) caches a widget's request mode when its parent
/// asks for it, without settling the resize it still has queued, and while
/// that resize stays queued no later `queue_resize` clears the cache. A
/// notice card asked while it sat empty behind a hidden frame keeps
/// "constant size" after it is filled, so it is measured as if its words
/// were on one line and laid out with them wrapped: the Private AI offer
/// was placed 67 high and drew its buttons at 54 to 88, clipping "Turn
/// on". Measuring a widget settles its queued resize, so the
/// `queue_resize` after it drops the stale mode. GTK 4.22 settles it when it
/// fetches the mode, and there this costs one measure.
fn renew_request_modes(card: &gtk::Widget, top: &gtk::Widget) {
    let renew = |widget: &gtk::Widget| {
        // A hidden widget is not measured, so its queued resize would stay.
        if widget.get_visible() {
            let _ = widget.measure(gtk::Orientation::Horizontal, -1);
            widget.queue_resize();
        }
    };
    let mut all = vec![card.clone()];
    let mut index = 0;
    while index < all.len() {
        let mut child = all[index].first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            all.push(next);
        }
        index += 1;
    }
    for widget in all.iter().rev() {
        renew(widget);
    }
    let mut ancestor = card.parent();
    while let Some(widget) = ancestor {
        renew(&widget);
        if &widget == top {
            break;
        }
        ancestor = widget.parent();
    }
}

/// The card's drawn surface, the first shown `tc-banner` or `tc-card` in
/// it: where its X goes. The cards wrap it in clamps and margins.
fn card_surface(card: &gtk::Widget) -> Option<gtk::Widget> {
    if card.get_visible() && (card.has_css_class("tc-banner") || card.has_css_class("tc-card")) {
        return Some(card.clone());
    }
    let mut child = card.first_child();
    while let Some(next) = child {
        if next.get_visible()
            && let Some(surface) = card_surface(&next)
        {
            return Some(surface);
        }
        child = next.next_sibling();
    }
    None
}

/// Calls `refresh` whenever a label in `card` changes its words or shows
/// or hides: what `card_text` reads. Each label is watched once.
fn watch_words(
    card: &gtk::Widget,
    watched: &RefCell<Vec<gtk::glib::WeakRef<gtk::Label>>>,
    refresh: &Rc<RefCell<Weak<dyn Fn()>>>,
) {
    if let Some(label) = card.downcast_ref::<gtk::Label>() {
        let mut watched = watched.borrow_mut();
        watched.retain(|weak| weak.upgrade().is_some());
        if !watched
            .iter()
            .any(|weak| weak.upgrade().as_ref() == Some(label))
        {
            watched.push(label.downgrade());
            for property in ["label", "visible"] {
                let refresh = Rc::clone(refresh);
                label.connect_notify_local(Some(property), move |_, _| {
                    let follow = refresh.borrow().upgrade();
                    if let Some(follow) = follow {
                        follow();
                    }
                });
            }
        }
    }
    let mut child = card.first_child();
    while let Some(next) = child {
        watch_words(&next, watched, refresh);
        child = next.next_sibling();
    }
}

/// Everything a card says, in order: what its X closed, so that new words
/// are a new card (`notices::Closed`).
fn card_text(card: &gtk::Widget) -> String {
    fn walk(widget: &gtk::Widget, text: &mut String) {
        if let Some(label) = widget.downcast_ref::<gtk::Label>()
            && label.get_visible()
        {
            text.push_str(&label.text());
            text.push('\n');
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            walk(&next, text);
            child = next.next_sibling();
        }
    }
    let mut text = String::new();
    walk(card, &mut text);
    text
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
    /// The Private AI tab's dot, and the tone it is drawn in.
    inference_dot: gtk::DrawingArea,
    inference_tone: Rc<Cell<style::Tone>>,
    breadcrumb: gtk::Box,
    content: gtk::Stack,
    graph_toggle: gtk::ToggleButton,
    map_toggle: gtk::ToggleButton,
    inspector_toggle: gtk::ToggleButton,
    settings_button: gtk::Button,
    inspector_stack: gtk::Stack,
    history_detail: gtk::Box,
    notice_slots: Vec<NoticeSlot>,
    /// Re-reads every notice's frame, X and the stack from the cards.
    refresh_notices: Rc<dyn Fn()>,
    settings_root: gtk::Box,
    settings_window: RefCell<Option<adw::Window>>,
    /// Set while this file moves a toggle itself, so the toggle's own
    /// handler does not read that as the person's press.
    syncing: Cell<bool>,
}

/// One toolbar toggle. Its name and tip say what pressing it does, so they
/// are set with its state (`Shell::apply`), not here.
fn toolbar_toggle(glyph_svg: &str) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::new();
    button.set_child(Some(&glyph::icon(glyph_svg)));
    button.add_css_class("tc-tab");
    button.add_css_class("tc-toolbar-toggle");
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
        // The ground the panes sit on, seen in the gaps between them.
        window.add_css_class("tc-glass-window");
        follow_high_contrast(window);

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
        let graph_toggle = toolbar_toggle(brand_tokens::glyph::GRAPH);
        let map_toggle = toolbar_toggle(brand_tokens::glyph::MAP);
        let inspector_toggle = toolbar_toggle(brand_tokens::glyph::INSPECTOR);
        // `GlassToolbarGroup`: 2px between its buttons.
        let capsule = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        capsule.add_css_class("tc-glass-control");
        capsule.add_css_class("tc-toolbar-capsule");
        capsule.set_valign(gtk::Align::Center);
        for toggle in [&graph_toggle, &map_toggle, &inspector_toggle] {
            capsule.append(toggle);
        }
        let settings_button = gtk::Button::new();
        settings_button.set_child(Some(&glyph::icon(brand_tokens::glyph::GEAR)));
        settings_button.add_css_class("tc-toolbar-toggle");
        settings_button.add_css_class("tc-toolbar-gear");
        settings_button.add_css_class("flat");
        settings_button.add_css_class("circular");
        settings_button.set_valign(gtk::Align::Center);
        settings_button.set_tooltip_text(Some(words.settings_title));
        settings_button.update_property(&[gtk::accessible::Property::Label(words.settings_title)]);
        // Before the end controls, after the spacer.
        let end = toolbar.last_child();
        capsule.insert_before(&toolbar, end.as_ref());
        settings_button.insert_before(&toolbar, end.as_ref());
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&toolbar));

        // --- The tab strip ---------------------------------------------
        let traces_badge = gtk::Label::builder().visible(false).build();
        traces_badge.add_css_class("tc-count-badge");
        traces_badge.set_valign(gtk::Align::Center);
        // The Mac's tab dot (`GlassStatusDot(status, size: 6)`): a 6px
        // circle in its state's colour, its state in its tip and
        // accessible name (`render_inference_state`). A fixed-size drawing,
        // so no line of text can stretch it.
        let inference_tone = Rc::new(Cell::new(style::Tone::Neutral));
        let inference_dot = gtk::DrawingArea::builder()
            .content_width(glass_paint::TAB_DOT)
            .content_height(glass_paint::TAB_DOT)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .hexpand(false)
            .vexpand(false)
            .visible(false)
            .build();
        let tone = inference_tone.clone();
        inference_dot.set_draw_func(move |_, context, width, height| {
            let dark = adw::StyleManager::default().is_dark();
            glass_paint::paint_status_dot(
                context,
                f64::from(width) / 2.0,
                f64::from(height) / 2.0,
                f64::from(glass_paint::TAB_DOT),
                glass_paint::tab_dot_ink(tone.get(), dark),
                false,
                dark,
            );
        });
        let strip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        strip.add_css_class("tc-glass-tabs");
        // The tabs share the strip's width equally (`GlassRuleTabs`), and
        // the strip's rule runs the pane's width, so it has no side margin.
        strip.set_homogeneous(true);
        strip.set_hexpand(true);
        strip.set_margin_top(style::space::S);
        strip.update_property(&[gtk::accessible::Property::Label(words.shell.tabs_label)]);
        let mut tab_buttons: Vec<(Tab, gtk::ToggleButton)> = Vec::new();
        for (index, tab) in Tab::ALL.into_iter().enumerate() {
            // `space.inlineGap` between the title and its mark.
            let content = gtk::Box::new(
                gtk::Orientation::Horizontal,
                brand_tokens::glass::space::INLINE_GAP,
            );
            let title = gtk::Label::builder()
                .label(tab.title())
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            title.add_css_class("tc-type-body");
            content.append(&title);
            match tab {
                Tab::Traces => content.append(&traces_badge),
                Tab::Inference => content.append(&inference_dot),
                Tab::Home | Tab::Insights => {}
            }
            content.set_halign(gtk::Align::Center);
            let button = gtk::ToggleButton::builder().child(&content).build();
            button.set_hexpand(true);
            button.add_css_class("tc-tab");
            button.add_css_class("flat");
            // The chord and nothing else, in GTK's own words.
            button.set_tooltip_text(crate::ui::shortcuts::screen_accel_label(index).as_deref());
            // Named by its title: the child is a box, so without a name of
            // its own a screen reader reads the tooltip's chord instead.
            button.update_property(&[gtk::accessible::Property::Label(tab.title())]);
            if let Some((_, first)) = tab_buttons.first() {
                button.set_group(Some(first));
            }
            strip.append(&button);
            tab_buttons.push((tab, button));
        }

        // The selected tab's ember line and halo (`GlassRuleTabs.emberGlow`)
        // are drawn once over the whole strip: the line reaches 0.3 of a
        // tab's width into each neighbour, which a tab's own background
        // could not.
        let ember = gtk::DrawingArea::builder()
            .can_target(false)
            .can_focus(false)
            .build();
        ember.set_accessible_role(gtk::AccessibleRole::Presentation);
        {
            let tabs: Vec<gtk::ToggleButton> = tab_buttons
                .iter()
                .map(|(_, button)| button.clone())
                .collect();
            ember.set_draw_func(move |area, context, _, height| {
                let Some(selected) = glowing_tab(&tabs) else {
                    return;
                };
                let Some(bounds) = selected.compute_bounds(area) else {
                    return;
                };
                glass_paint::paint_ember(
                    context,
                    f64::from(height),
                    f64::from(bounds.x()),
                    f64::from(bounds.width()),
                    adw::StyleManager::default().is_dark(),
                );
            });
            for (_, button) in &tab_buttons {
                let weak = ember.downgrade();
                button.connect_toggled(move |_| {
                    if let Some(ember) = weak.upgrade() {
                        ember.queue_draw();
                    }
                });
                // A tab shown or hidden moves the others (onboarding hides
                // two), and the glow with them.
                let weak = ember.downgrade();
                button.connect_visible_notify(move |_| {
                    if let Some(ember) = weak.upgrade() {
                        ember.queue_draw();
                    }
                });
            }
            let weak = ember.downgrade();
            adw::StyleManager::default().connect_dark_notify(move |_| {
                if let Some(ember) = weak.upgrade() {
                    ember.queue_draw();
                }
            });
        }
        let strip_glow = gtk::Overlay::builder().child(&strip).build();
        strip_glow.add_overlay(&ember);
        strip_glow.set_clip_overlay(&ember, true);

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
        // Every screen fits the pane's 440 and scrolls itself, as on the
        // Mac: nothing here scrolls sideways, and a screen's own height
        // never sets the window's (the Mac's window is `WINDOW_HEIGHT`
        // tall whatever the tab).
        let content_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .child(&content)
            .build();

        // The notices, laid over the top of the tab's screen as on the Mac
        // (`GlassPopoverStack`, owner decision 1 of the 2026-11-16 spec):
        // they push nothing down, and the screen keeps the whole of the
        // pane. Only the front card is drawn whole, with up to two behind
        // it peeking out under it (`notices::stack`); each card's X closes
        // it and the next comes forward. A front card taller than the pane
        // scrolls inside the stack, so its buttons stay in reach.
        let notice_stack = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let notice_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .valign(gtk::Align::Start)
            .visible(false)
            .child(&notice_stack)
            .build();
        let closed = Rc::new(RefCell::new(notices::Closed::default()));
        let mut notice_slots = Vec::new();
        for (notice, card) in pages.notices {
            // Three flags, each set by one owner: the slot by the tab
            // (`notices::shown`), the frame by the stack and the card's X,
            // the card by whatever draws it.
            let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
            // The X inside the card, at its top-right corner, as the Mac's
            // `GlassPopoverNotice` draws it; a closable card keeps the room
            // for it (`tc-notice-closable`), so it never sits on the words.
            let frame = gtk::Overlay::new();
            card.set_hexpand(true);
            frame.set_child(Some(&card));
            let close = gtk::Button::from_icon_name("window-close-symbolic");
            close.add_css_class("flat");
            close.add_css_class("circular");
            close.set_tooltip_text(Some(words.close));
            close.update_property(&[gtk::accessible::Property::Label(words.close)]);
            close.set_visible(false);
            frame.add_overlay(&close);
            frame.set_measure_overlay(&close, false);
            frame.connect_get_child_position(|frame, close| {
                let card = frame.child()?;
                let surface = card_surface(&card).unwrap_or(card);
                let bounds = surface.compute_bounds(frame)?;
                let (_, size) = close.preferred_size();
                let inset = style::space::XS as f32;
                Some(gtk::gdk::Rectangle::new(
                    (bounds.x() + bounds.width() - inset) as i32 - size.width(),
                    (bounds.y() + inset) as i32,
                    size.width(),
                    size.height(),
                ))
            });
            frame.set_visible(false);
            // The card comes to the front as on the Mac: a new one slides
            // in from the top, a revealed one fades (`popover_stack`).
            let slide = crate::ui::popover_stack::Slide::new(&frame);
            frame
                .bind_property("visible", &slide, "visible")
                .sync_create()
                .build();
            slot.append(&slide);
            notice_stack.append(&slot);
            notice_slots.push(NoticeSlot {
                notice,
                slot,
                frame,
                card,
                close,
            });
        }
        // The cards behind the front one, peeking out under it, each 4% a
        // level narrower (`popover_stack::Plates`).
        let plates = crate::ui::popover_stack::Plates::new();
        notice_stack.append(&plates);
        // Each card shows and hides itself and says what it says, and its X
        // closes it; the stack follows all three. The words are watched on
        // every label in the card, so a closed card whose words change comes
        // back as they change; a card that rebuilds its labels says so
        // through `Shell::notices_changed`.
        let follow = {
            let scroller = notice_scroller.downgrade();
            let closed = Rc::clone(&closed);
            let unset: Weak<dyn Fn()> = Weak::<fn()>::new();
            let itself = Rc::new(RefCell::new(unset));
            let watched: RefCell<Vec<gtk::glib::WeakRef<gtk::Label>>> = RefCell::new(Vec::new());
            let cards: Vec<(
                Notice,
                gtk::glib::WeakRef<gtk::Box>,
                gtk::glib::WeakRef<gtk::Overlay>,
                gtk::glib::WeakRef<gtk::Widget>,
                gtk::glib::WeakRef<gtk::Button>,
            )> = notice_slots
                .iter()
                .map(|slot| {
                    (
                        slot.notice,
                        slot.slot.downgrade(),
                        slot.frame.downgrade(),
                        slot.card.downgrade(),
                        slot.close.downgrade(),
                    )
                })
                .collect();
            let plates = plates.downgrade();
            // The cards the stack has held, so a card revealed by a close is
            // told apart from one that has just arrived; the front last
            // drawn; and whether the stack has drawn before (the first
            // front is simply there).
            let seen: RefCell<Vec<(Notice, String)>> = RefCell::default();
            let last_front: Cell<Option<usize>> = Cell::new(None);
            let started = Cell::new(false);
            let hook = Rc::clone(&itself);
            let follow: Rc<dyn Fn()> = Rc::new(move || {
                let Some(scroller) = scroller.upgrade() else {
                    return;
                };
                let mut open = Vec::with_capacity(cards.len());
                let mut parts = Vec::with_capacity(cards.len());
                for (notice, slot, frame, card, close) in &cards {
                    let (Some(slot), Some(frame), Some(card), Some(close)) = (
                        slot.upgrade(),
                        frame.upgrade(),
                        card.upgrade(),
                        close.upgrade(),
                    ) else {
                        open.push(false);
                        parts.push(None);
                        continue;
                    };
                    watch_words(&card, &watched, &hook);
                    // Each widget's own flag, not `is_visible`: that one
                    // asks the ancestors too, and the stack starts hidden,
                    // so no card would ever look shown.
                    if !card.get_visible() {
                        // The card went: when its cause recurs it is shown
                        // again.
                        closed.borrow_mut().forget(*notice);
                    }
                    let shown =
                        card.get_visible() && !closed.borrow().hides(*notice, &card_text(&card));
                    open.push(slot.get_visible() && shown);
                    parts.push(Some((*notice, frame, card, close, shown)));
                }
                let stack = notices::stack(&open);
                let mut front = None;
                for (index, part) in parts.into_iter().enumerate() {
                    let Some((notice, frame, card, close, shown)) = part else {
                        continue;
                    };
                    // A hidden or closed card shows neither its frame nor
                    // its X; of the open ones only the front is whole.
                    frame.set_visible(stack.front == Some(index));
                    if stack.front == Some(index) {
                        front = Some(card.clone());
                    }
                    let x = shown
                        && notice.close() != notices::Close::Own
                        && !card.has_css_class(notices::STAYS_CLASS);
                    close.set_visible(x);
                    if x {
                        frame.add_css_class("tc-notice-closable");
                    } else {
                        frame.remove_css_class("tc-notice-closable");
                    }
                    mark_surface(&card, x);
                }
                if let Some(plates) = plates.upgrade() {
                    plates.set_behind(stack.behind);
                }
                // A card new to the front slides in, a revealed one fades.
                if stack.front != last_front.get() {
                    if let Some((notice, _, _, card, _)) = stack.front.and_then(|i| cards.get(i))
                        && let Some(card) = card.upgrade()
                        && let Some(slide) = card
                            .ancestor(crate::ui::popover_stack::Slide::static_type())
                            .and_downcast::<crate::ui::popover_stack::Slide>()
                    {
                        let notice = *notice;
                        let fresh = !seen.borrow().contains(&(notice, card_text(&card)));
                        if started.get() && crate::ui::glass_motion::animations_enabled() {
                            slide.arrive(fresh, false);
                        } else {
                            slide.settle();
                        }
                    }
                    last_front.set(stack.front);
                }
                for (index, (notice, _, _, card, _)) in cards.iter().enumerate() {
                    if open.get(index) == Some(&true)
                        && let Some(card) = card.upgrade()
                    {
                        let key = (*notice, card_text(&card));
                        if !seen.borrow().contains(&key) {
                            seen.borrow_mut().push(key);
                        }
                    }
                }
                started.set(true);
                scroller.set_visible(stack.front.is_some());
                if let Some(card) = front {
                    renew_request_modes(&card, scroller.upcast_ref());
                }
            });
            *itself.borrow_mut() = Rc::downgrade(&follow);
            follow
        };
        for slot in &notice_slots {
            let refresh = follow.clone();
            slot.slot.connect_visible_notify(move |_| refresh());
            let refresh = follow.clone();
            slot.card.connect_visible_notify(move |_| refresh());
            let refresh = follow.clone();
            slot.card
                .connect_notify_local(Some("css-classes"), move |_, _| refresh());
        }
        follow();
        // Opening the window again shows every card its X closed.
        {
            let closed = Rc::clone(&closed);
            let refresh = follow.clone();
            window.connect_visible_notify(move |window| {
                if window.get_visible() {
                    closed.borrow_mut().clear();
                    refresh();
                }
            });
        }

        // The stack is the whole width of the pane and as tall as its cards
        // are at that width, never taller than the pane (then it scrolls).
        // Its height is measured for the width it is given: GTK 4.14's
        // default placement measured it for its natural width, one line of
        // buttons, and then gave it less height than the narrower card drew,
        // clipping the Private AI offer's "Turn on". That measure is for the
        // width given only while the card's request mode is current, which
        // on GTK 4.14 takes `renew_request_modes`.
        notice_scroller.add_css_class(NOTICE_BACKDROP_CLASS);
        let over_content = gtk::Overlay::new();
        over_content.set_child(Some(&content_scroller));
        over_content.add_overlay(&notice_scroller);
        over_content.set_vexpand(true);
        over_content.connect_get_child_position(|overlay, stack| {
            let (width, height) = (overlay.width(), overlay.height());
            if width <= 0 || height <= 0 {
                return None;
            }
            let (least, _, _, _) = stack.measure(gtk::Orientation::Horizontal, -1);
            let width = width.max(least);
            let (_, natural, _, _) = stack.measure(gtk::Orientation::Vertical, width);
            Some(gtk::gdk::Rectangle::new(0, 0, width, natural.min(height)))
        });

        let graph = GraphFooter::new();

        let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main.add_css_class("tc-glass-pane");
        // The rounded corners clip what the pane holds.
        main.set_overflow(gtk::Overflow::Hidden);
        main.append(&handle);
        main.append(&strip_glow);
        main.append(&breadcrumb);
        main.append(&over_content);
        main.append(&graph.root);

        // --- The map pane ----------------------------------------------
        let map = TraceMap::new();
        map.root.set_vexpand(true);
        let map_pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
        map_pane.add_css_class("tc-glass-pane");
        // The rounded corners clip what the pane holds.
        map_pane.set_overflow(gtk::Overflow::Hidden);
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
        inspector_pane.add_css_class("tc-glass-pane");
        // The rounded corners clip what the pane holds.
        inspector_pane.set_overflow(gtk::Overflow::Hidden);
        inspector_pane.append(&inspector_stack);

        let root = ThreePane::new(&main, &map_pane, &inspector_pane);
        // KDE's blur behind the panes, where it is granted; painted
        // everywhere else (`ui::glass_backdrop`).
        crate::ui::glass_backdrop::follow(
            window,
            vec![
                main.clone().upcast(),
                map_pane.clone().upcast(),
                inspector_pane.clone().upcast(),
            ],
        );

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
        // The Traces map's style, as the person last chose it.
        map.set_style(prefs.map_style());

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
                history_detail_slot: RefCell::new(None),
                closed,
                missions,
            },
            window: window.clone(),
            app: RefCell::new(Weak::new()),
            tabs: tab_buttons,
            inference_dot,
            inference_tone,
            breadcrumb,
            content,
            graph_toggle,
            map_toggle,
            inspector_toggle,
            settings_button,
            inspector_stack,
            history_detail,
            notice_slots,
            refresh_notices: follow,
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

    /// The map pane's view: the Traces map or the Private AI map.
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
        let window = self.settings_window_for(&app.window);
        window.present();
        self.settings_window.replace(Some(window));
    }

    /// The Settings window beside `main`, holding the Settings view.
    fn settings_window_for(&self, main: &adw::ApplicationWindow) -> adw::Window {
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
        if let Some(application) = main.application() {
            window.set_application(Some(&application));
        }
        // A hidden Settings window must not keep the process alive after
        // the main window has gone.
        let weak = window.downgrade();
        main.connect_destroy(move |_| {
            if let Some(window) = weak.upgrade() {
                window.destroy();
            }
        });
        window
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

    /// The Traces map's style (Settings): the rocket and station, or the
    /// laptop and the orb. One owner keeps it with the panes' choices, so
    /// a later save of those never undoes it.
    pub fn map_style(&self) -> MapStyle {
        self.state.prefs.borrow().map_style()
    }

    pub fn set_map_style(&self, style: MapStyle) {
        self.update_prefs(|prefs| prefs.set_map_style(style));
        self.map.set_style(style);
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

    /// The scroller the tab's screens sit in.
    #[cfg(test)]
    fn content_scroller(&self) -> gtk::ScrolledWindow {
        self.content
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast()
            .expect("the screens scroll")
    }

    // --- Notices ---------------------------------------------------------

    /// A notice's X (`notices::Close::Session`): the card is closed until
    /// the window is opened again or its words change.
    pub fn close_notice(&self, notice: Notice) {
        let Some(slot) = self.notice_slots.iter().find(|slot| slot.notice == notice) else {
            return;
        };
        if notice.close() == notices::Close::Session {
            self.state
                .closed
                .borrow_mut()
                .close(notice, card_text(&slot.card));
            (self.refresh_notices)();
        }
    }

    /// A card rebuilt its labels (the arming offer redraws its words from
    /// scratch): a closed card whose words changed comes back. A label's
    /// own words changing needs no call; the stack watches those.
    pub fn notices_changed(&self) {
        (self.refresh_notices)();
    }

    // --- Selection and the inspector -------------------------------------

    /// A selection made in the Traces list. A session is a demand on the
    /// inspector: selecting one shows the pane its card lives in. A folder,
    /// or nothing, moves the selection only. The inspector never closes
    /// itself.
    pub fn select(&self, app: &Rc<App>, selection: Selection) {
        let session = matches!(selection, Selection::Session(_));
        self.inspector.select(app, selection);
        if session {
            self.update_prefs(|prefs| prefs.shows_inspector = true);
        }
        self.refresh_demand(app);
        self.render_graph(app);
        // The selection's tool is ringed on the map, the others faded.
        self.map.render(app);
        // The folder list draws the selected row, wherever it was chosen.
        crate::ui::queue::render_list(app);
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
        // Opening a row reads its details afresh.
        self.state.history_detail_slot.replace(None);
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
            Some(record) => {
                self.history_detail
                    .append(&crate::ui::history::record_detail(record));
                // The contribution's details, read from the daemon, as the
                // Mac's `SessionDetailView` shows them beside the record.
                if !record.submission_id.is_empty() {
                    let kept = self
                        .state
                        .history_detail_slot
                        .borrow()
                        .clone()
                        .filter(|(id, _)| *id == record.submission_id);
                    let slot = match kept {
                        Some((_, slot)) => slot,
                        None => {
                            let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
                            let id = record.submission_id.clone();
                            crate::ui::history_detail::load(
                                app,
                                &slot,
                                &record.submission_id,
                                &record.status,
                                move |app| *app.shell.state.opened_history.borrow() == id,
                            );
                            self.state
                                .history_detail_slot
                                .replace(Some((record.submission_id.clone(), slot.clone())));
                            slot
                        }
                    };
                    self.history_detail.append(&slot);
                }
            }
            // A row that has gone is the Traces selection again, never a
            // stale card.
            None if records.is_some() => self.state.opened_history.borrow_mut().clear(),
            None => {}
        }
    }

    /// What is asking for the inspector right now, from what this window
    /// holds (`demand::asking`): the approval it holds open for undoing (a
    /// trace's Submit, or a folder's Submit all), the folder whose Submit
    /// all is in flight, the trace it last kept (`App::last_kept`, the
    /// Mac's `TracesStore.lastKept`), and a selected session. This shell has
    /// no keep action yet, so nothing sets `last_kept` and the `kept` key
    /// never asks; it is wired so a keep opens the inspector once one
    /// exists.
    fn demand_keys(&self, app: &Rc<App>) -> BTreeSet<String> {
        let undo = app.undo.borrow();
        let submitting = app.submitting_folder.borrow();
        let kept = app.last_kept.borrow();
        let selection = self.inspector.selection();
        demand::keys(&demand::asking(
            undo.as_ref().map(|undo| demand::Approval {
                entry_ids: undo.entry_ids.as_slice(),
                at: undo.approved_at.timestamp(),
                folder: undo.folder.as_deref(),
            }),
            submitting.as_deref(),
            kept.as_deref(),
            match &selection {
                Selection::Session(id) => Some(id.as_str()),
                Selection::Folder(_) | Selection::None => None,
            },
        ))
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
        self.render_history_detail(app);
        // The graph first: it clears the focus when nothing is selected,
        // which the map then draws.
        self.render_graph(app);
        self.map.render(app);
        self.refresh_demand(app);
        self.apply();
    }

    /// The selection's tool's source, for the map's ring.
    pub fn selected_tool_id(&self, app: &Rc<App>) -> Option<String> {
        self.selected_tool(app).map(|(source, _)| source)
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

    /// The Traces tab's text equivalent of its badge, as the Mac's
    /// segment carries it: the core's decisions-owed sentence, and nothing
    /// at zero, where there is no badge.
    pub fn describe_traces_badge(&self, decisions_owed: Option<u64>) {
        let Some((_, button)) = self.tabs.iter().find(|(tab, _)| *tab == Tab::Traces) else {
            return;
        };
        let text = trace_commons_contributor::preview_copy::decisions_owed_text(decisions_owed);
        if text.is_empty() {
            button.reset_property(gtk::AccessibleProperty::Description);
        } else {
            button.update_property(&[gtk::accessible::Property::Description(&text)]);
        }
    }

    /// The Inference tab's dot: what the listener is doing, from the
    /// daemon's own report and the core's table, never the switch. An
    /// unreported state is no dot: unknown is neither on nor off.
    pub fn render_inference_state(&self, state: &str) {
        let dot = &self.inference_dot;
        if state.is_empty() {
            dot.set_visible(false);
            return;
        }
        let tone = crate::ui::private_inference::indicator_tone(
            crate::copy::private_inference_state_tone(state),
        );
        let line = crate::copy::private_inference_state_line(state);
        self.inference_tone.set(tone);
        dot.queue_draw();
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
        self.map.set_cards_shown(tabs::transfer_cards_shown(at.tab));
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
        for slot in &self.notice_slots {
            slot.slot.set_visible(notices::shown(slot.notice, place));
        }
        // A card's words may have changed since its X closed it.
        (self.refresh_notices)();

        self.map.show_view(self.state.map_view.get());

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
    for slot in &shell.notice_slots {
        let weak = Rc::downgrade(app);
        let notice = slot.notice;
        slot.close.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.shell.close_notice(notice);
            }
        });
    }

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
        shell.map.render(&app);
    });

    // The map's view switch: the map's view only, never the tab.
    for (toggle, view) in [
        (&shell.map.traces_view, MapView::Traces),
        (&shell.map.private_view, MapView::PrivateAi),
    ] {
        let weak = Rc::downgrade(app);
        toggle.connect_toggled(move |toggle| {
            let Some(app) = weak.upgrade() else { return };
            if toggle.is_active() && !app.shell.syncing.get() {
                app.shell.state.map_view.set(view);
                app.shell.apply();
            }
        });
    }

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

/// The hosted screens against the real `App` (`tests/support`).
#[cfg(test)]
#[path = "../../../tests/support/main_pane_fit_tests.rs"]
mod fit_tests;

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
            .split("fn settings_window_for")
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

    /// Each tab button is named by its title. A tab's child is a box, not
    /// a label, so without a name of its own a screen reader falls back to
    /// the tooltip and reads the chord ("Ctrl+1").
    #[test]
    fn each_tab_is_named_by_its_title() {
        let code = include_str!("shell.rs")
            .split("\n#[cfg(test)]")
            .next()
            .expect("the module has a body before its tests");
        let strip = code
            .split("for (index, tab) in Tab::ALL")
            .nth(1)
            .expect("the strip builds one button per tab")
            .split("tab_buttons.push")
            .next()
            .unwrap();
        assert!(
            strip.contains(
                "button.update_property(&[gtk::accessible::Property::Label(tab.title())])"
            ),
            "a tab button has no accessible name of its own"
        );
    }

    // --- Mounted: the widgets, on a display ------------------------------

    use super::*;

    fn pump() {
        let context = gtk::glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
    }

    /// Runs the main loop until `done`, for at most five seconds: a layout
    /// waits for the frame clock, so what is pending now is not all there
    /// will be.
    fn pump_until(done: impl Fn() -> bool) {
        let context = gtk::glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done() && std::time::Instant::now() < deadline {
            if !context.iteration(false) {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        pump();
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
        // These tests read where cards are; a card still sliding in would
        // be offset. The test of the arrival turns motion on itself.
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_enable_animations(false);
        }
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
    fn mounted_toolbar_holds_the_toggles_and_the_gear_before_the_end_controls() {
        let mounted = mounted(false);
        let shell = &mounted.shell;
        // The toggles sit in one capsule, the gear after it, both in the
        // toolbar row, and the desktop's end buttons stay last.
        let capsule = shell.graph_toggle.parent().expect("the toggles are placed");
        let toolbar = capsule.parent().expect("the capsule is in the toolbar");
        for toggle in [
            &shell.graph_toggle,
            &shell.map_toggle,
            &shell.inspector_toggle,
        ] {
            assert_eq!(toggle.parent().as_ref(), Some(&capsule));
        }
        assert_eq!(shell.settings_button.parent().as_ref(), Some(&toolbar));
        let mut row = Vec::new();
        let mut child = toolbar.first_child();
        while let Some(widget) = child {
            row.push(widget.clone());
            child = widget.next_sibling();
        }
        let at = |widget: &gtk::Widget| row.iter().position(|w| w == widget).expect("in the row");
        assert!(at(&capsule) < at(shell.settings_button.upcast_ref()));
        assert!(
            row.last()
                .is_some_and(|last| last.is::<gtk::WindowControls>()),
            "the end controls are last"
        );
        assert!(at(shell.settings_button.upcast_ref()) < row.len() - 1);
        mounted.window.close();
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_tabs_are_named_by_their_titles() {
        let mounted = mounted(false);
        for (tab, button) in &mounted.shell.tabs {
            assert!(
                gtk::test_accessible_has_property(button, gtk::AccessibleProperty::Label),
                "the {} tab has no accessible name, so it is read as its chord",
                tab.title()
            );
        }
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
        // The ember glows under the tab drawn, not the hidden choice.
        let buttons: Vec<gtk::ToggleButton> = shell
            .tabs
            .iter()
            .map(|(_, button)| button.clone())
            .collect();
        assert_eq!(glowing_tab(&buttons), Some(&shell.tabs[1].1));
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

    fn slot(shell: &Shell, notice: Notice) -> &NoticeSlot {
        shell
            .notice_slots
            .iter()
            .find(|slot| slot.notice == notice)
            .expect("every notice has a slot")
    }

    /// The stack the notices are laid out in: the scroller laid over the
    /// tab's screen.
    fn notice_scroller(shell: &Shell) -> gtk::ScrolledWindow {
        shell.notice_slots[0]
            .slot
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
            .expect("the notices are in a stack that scrolls")
    }

    /// The frames drawn whole: the stack's front card, or none.
    fn fronts(shell: &Shell) -> Vec<Notice> {
        shell
            .notice_slots
            .iter()
            .filter(|slot| slot.frame.is_visible())
            .map(|slot| slot.notice)
            .collect()
    }

    /// The plates peeking out behind the front card.
    fn plates(shell: &Shell) -> usize {
        let mut found = Vec::new();
        descendants(notice_scroller(shell).upcast_ref(), &mut found);
        found
            .iter()
            .filter_map(|widget| widget.downcast_ref::<crate::ui::popover_stack::Plates>())
            .filter(|plates| plates.is_visible())
            .map(|plates| plates.behind())
            .sum()
    }

    /// The card a slot's frame moves in.
    fn slide(shell: &Shell, notice: Notice) -> crate::ui::popover_stack::Slide {
        slot(shell, notice)
            .frame
            .parent()
            .and_downcast::<crate::ui::popover_stack::Slide>()
            .expect("every card is held by a slide")
    }

    /// `GlassPopoverStack`'s arrival: a card new to the stack slides in from
    /// the top as it fades in; one revealed by closing the card in front of
    /// it only fades; with animations off a card is simply there.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_a_new_card_slides_in_and_a_revealed_one_fades() {
        let mounted = mounted(false);
        let shell = &mounted.shell;
        mounted.window.present();
        shell.pick_tab(Tab::Traces);
        let card = |notice: Notice| {
            mounted
                .notices
                .iter()
                .find(|(kind, _)| *kind == notice)
                .map(|(_, card)| card.clone())
                .unwrap()
        };
        for (_, card) in &mounted.notices {
            card.set_size_request(-1, 120);
            card.set_visible(false);
        }
        pump();
        let settings = gtk::Settings::default().expect("settings");
        settings.set_gtk_enable_animations(true);

        // A card the stack has never held slides in from above its place.
        // The stack remembers every card it has held, as the Mac's `seen`
        // does, and these empty cards were all shown once while the shell
        // was built, so this one gets words it has never had.
        card(Notice::Health).append(&gtk::Label::new(Some("A card never shown")));
        card(Notice::Health).set_visible(true);
        let health = slide(shell, Notice::Health);
        let slid = Cell::new(false);
        pump_until(|| {
            slid.set(slid.get() || health.offset() < -1.0);
            slid.get() && health.offset() == 0.0 && health.opacity() == 1.0
        });
        assert!(slid.get(), "a new card did not slide in");
        assert_eq!((health.offset(), health.opacity()), (0.0, 1.0));

        // A newer card in front of it slides in too; closing that one
        // reveals the Health card, which fades where it is.
        card(Notice::Undo).set_visible(true);
        pump_until(|| slide(shell, Notice::Undo).offset() == 0.0);
        card(Notice::Undo).set_visible(false);
        let faded = Cell::new(false);
        let moved = Cell::new(false);
        pump_until(|| {
            faded.set(faded.get() || health.opacity() < 1.0);
            moved.set(moved.get() || health.offset() != 0.0);
            faded.get() && health.opacity() == 1.0
        });
        assert!(faded.get(), "a revealed card did not fade in");
        assert!(!moved.get(), "a revealed card slid");

        // Under reduced motion a new card is there at once.
        settings.set_gtk_enable_animations(false);
        card(Notice::Health).set_visible(false);
        card(Notice::Consent).set_visible(true);
        pump();
        let consent = slide(shell, Notice::Consent);
        assert_eq!((consent.offset(), consent.opacity()), (0.0, 1.0));
    }

    /// The Mac's placement (owner decision 1, chosen again 2026-11-17):
    /// the notices are laid over the top of the tab, pushing nothing down,
    /// so the tab's screen keeps the whole of the pane. Only the front card
    /// is whole, with up to two peeking behind it, and a front card taller
    /// than the pane scrolls.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_notices_are_laid_over_the_tab_they_belong_to() {
        let mounted = mounted(false);
        let shell = &mounted.shell;
        let scroller = notice_scroller(shell);
        let content = shell.content_scroller();
        let overlay = content
            .parent()
            .and_downcast::<gtk::Overlay>()
            .expect("the tab's screen is under an overlay");
        assert_eq!(overlay.child().as_ref(), Some(content.upcast_ref()));
        assert_eq!(
            scroller.parent().as_ref(),
            Some(overlay.upcast_ref()),
            "the notices are not laid over the tab"
        );
        for (notice, card) in &mounted.notices {
            assert!(card.is_ancestor(&slot(shell, *notice).slot));
        }
        // The front card first: the undo, the one card with a clock on it.
        assert_eq!(shell.notice_slots[0].notice, Notice::Undo);

        // (a) Every card showing on Traces: the screen keeps the whole of
        // the pane under them.
        shell.pick_tab(Tab::Traces);
        for (_, card) in &mounted.notices {
            card.set_size_request(-1, 120);
        }
        pump_until(|| scroller.height() > 0 && content.height() > 0);
        assert!(scroller.is_visible());
        assert_eq!(
            content.height(),
            overlay.height(),
            "the notices pushed the tab's screen down"
        );
        let bounds = scroller.compute_bounds(&overlay).expect("laid out");
        assert_eq!(bounds.y(), 0.0, "the stack is not at the top of the tab");

        // (c) Collapsed as on the Mac: only the front card is whole, and two
        // behind it peek out; closing or hiding it brings the next forward.
        assert_eq!(fronts(shell), vec![Notice::Undo]);
        assert_eq!(plates(shell), notices::BEHIND);
        let card = |notice: Notice| {
            mounted
                .notices
                .iter()
                .find(|(kind, _)| *kind == notice)
                .map(|(_, card)| card.clone())
                .unwrap()
        };
        card(Notice::Undo).set_visible(false);
        assert_eq!(fronts(shell), vec![Notice::Health]);
        for notice in [
            Notice::Health,
            Notice::Consent,
            Notice::ArmingOffer,
            Notice::Account,
        ] {
            card(notice).set_visible(false);
        }
        // Only Update is left on Traces (the Private AI offer is Inference's).
        assert_eq!(fronts(shell), vec![Notice::Update]);
        assert_eq!(plates(shell), 0);
        for (_, card) in &mounted.notices {
            card.set_visible(false);
        }
        assert!(!scroller.is_visible(), "an empty stack is drawn");
        assert!(fronts(shell).is_empty());

        // A front card taller than the pane scrolls inside the stack, which
        // is never taller than the tab.
        let tall = card(Notice::Undo);
        tall.set_size_request(-1, 2000);
        tall.set_visible(true);
        let scroll = scroller.vadjustment();
        pump_until(|| scroll.upper() >= 2000.0);
        assert!(
            scroller.height() <= overlay.height(),
            "the stack ran past the tab"
        );
        assert!(
            scroll.upper() > scroll.page_size(),
            "the front card's foot cannot be scrolled to"
        );
        for (_, card) in &mounted.notices {
            card.set_size_request(-1, -1);
            card.set_visible(true);
        }

        for tab in Tab::ALL {
            shell.pick_tab(tab);
            let place = tabs::place(shell.shown());
            for notice in Notice::ALL {
                assert_eq!(
                    slot(shell, notice).slot.is_visible(),
                    notices::shown(notice, place),
                    "{notice:?} on {tab:?}"
                );
            }
            assert_eq!(fronts(shell).len(), 1, "{tab:?}: one front card");
        }
        // Health is the Traces tab's, as on the Mac.
        shell.pick_tab(Tab::Home);
        assert!(!slot(shell, Notice::Health).slot.is_visible());
        shell.pick_tab(Tab::Traces);
        assert!(slot(shell, Notice::Health).slot.is_visible());
        mounted.window.close();
    }

    /// (b) The notices lie over the top of the tab, so a card in front
    /// covers the tab's first heading; putting every card away, by its X or
    /// by its own Dismiss, Acknowledge or Not now, uncovers it.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_closing_every_notice_uncovers_the_tabs_first_heading() {
        let mounted = mounted(false);
        let shell = &mounted.shell;
        let scroller = notice_scroller(shell);
        // Every tab's first line, as each screen draws its title.
        let mut headings = Vec::new();
        for name in ["home", "private-inference", "queue", "insights"] {
            let page = shell
                .content
                .child_by_name(name)
                .and_downcast::<gtk::Box>()
                .expect("each tab is a page");
            let heading = gtk::Label::new(Some(name));
            heading.add_css_class("title-2");
            page.prepend(&heading);
            headings.push((name, heading));
        }
        // A tall card on every tab, with words in it.
        for (notice, card) in &mounted.notices {
            card.append(&gtk::Label::new(Some(&format!("{notice:?}"))));
            card.set_size_request(-1, 120);
        }
        let root: &gtk::Widget = shell.root.upcast_ref();
        let covers = |heading: &gtk::Label| {
            let title = heading
                .compute_bounds(root)
                .expect("the heading is laid out");
            shell.notice_slots.iter().any(|slot| {
                slot.frame.is_visible()
                    && slot
                        .frame
                        .compute_bounds(root)
                        .is_some_and(|card| card.intersection(&title).is_some())
            })
        };
        for (tab, (name, heading)) in Tab::ALL.into_iter().zip(&headings) {
            shell.state.closed.borrow_mut().clear();
            for (_, card) in &mounted.notices {
                card.set_visible(true);
            }
            shell.pick_tab(tab);
            pump_until(|| heading.is_mapped() && heading.height() > 0 && scroller.height() > 0);
            assert!(scroller.is_visible(), "no notice is showing on {name}");
            assert!(
                covers(heading),
                "the {name} tab's notices are not laid over it"
            );
            // Put every card away, front first, as the person would.
            for notice in Notice::ALL {
                if !slot(shell, notice).slot.get_visible() {
                    continue;
                }
                match notice.close() {
                    notices::Close::Session => shell.close_notice(notice),
                    notices::Close::Own => {
                        mounted
                            .notices
                            .iter()
                            .find(|(kind, _)| *kind == notice)
                            .unwrap()
                            .1
                            .set_visible(false);
                    }
                }
                assert!(fronts(shell).len() <= 1, "{name}: more than one card whole");
            }
            assert!(!scroller.is_visible(), "a card is left over the {name} tab");
            pump();
            assert!(
                !covers(heading),
                "the {name} tab's heading is still covered"
            );
            // And the heading is in the pane, not pushed out of it.
            let title = heading
                .compute_bounds(root)
                .expect("the heading is laid out");
            assert!(
                f64::from(title.y() + title.height()) <= f64::from(root.height()),
                "the {name} tab's heading is below the window"
            );
        }
        mounted.window.close();
    }

    /// The Mac's X, inside the card: it closes the card until the window is
    /// opened again; the same card re-shown stays closed, new words (as
    /// they change, with nothing else redrawn) or the card going and coming
    /// back show it again. A hidden card shows no frame and no X. The undo,
    /// the consent and account cards and the Private AI offer carry their
    /// own Dismiss, Acknowledge or Not now and no extra X.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_a_notices_x_closes_that_card() {
        let mounted = mounted(false);
        let shell = &mounted.shell;
        let words = monitor_screens_copy();
        let card = |notice: Notice| {
            mounted
                .notices
                .iter()
                .find(|(kind, _)| *kind == notice)
                .map(|(_, card)| card.clone())
                .unwrap()
        };
        // Each card alone, in front on a tab it belongs to: an X on the
        // cards that have one, inside the card's drawn surface.
        // Each with a card nested in it, as the consent column holds its
        // notices: only the drawn surface keeps the X's room.
        let mut nested = Vec::new();
        for (notice, card) in &mounted.notices {
            card.add_css_class("tc-banner");
            card.set_size_request(-1, 60);
            card.append(&gtk::Label::new(Some(&format!("{notice:?} says"))));
            let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
            inner.add_css_class("tc-card");
            card.append(&inner);
            nested.push(inner);
        }
        shell.notices_changed();
        for notice in Notice::ALL {
            shell.pick_tab(if notice == Notice::PrivateAiOffer {
                Tab::Inference
            } else {
                Tab::Traces
            });
            for (kind, other) in &mounted.notices {
                other.set_visible(*kind == notice);
            }
            let slot = slot(shell, notice);
            assert_eq!(fronts(shell), vec![notice]);
            let closable = notice.close() != notices::Close::Own;
            assert_eq!(slot.close.is_visible(), closable, "{notice:?}");
            assert_eq!(
                slot.close.parent().as_ref(),
                Some(slot.frame.upcast_ref()),
                "{notice:?}: the X is not in the card's frame"
            );
            if closable {
                let close = slot.close.clone();
                pump_until(|| close.is_mapped() && close.width() > 0);
                let x = slot.close.compute_bounds(&slot.frame).expect("placed");
                let surface = slot.card.compute_bounds(&slot.frame).expect("placed");
                assert!(
                    x.x() >= surface.x()
                        && x.y() >= surface.y()
                        && x.x() + x.width() <= surface.x() + surface.width()
                        && x.y() + x.height() <= surface.y() + surface.height(),
                    "{notice:?}: the X {x:?} is beside the card {surface:?}, not inside it"
                );
                assert!(slot.frame.has_css_class("tc-notice-closable"));
                assert!(
                    slot.card.has_css_class(NOTICE_SURFACE_CLASS),
                    "{notice:?}: the surface keeps no room for its X"
                );
            }
            assert!(
                nested
                    .iter()
                    .all(|inner| !inner.has_css_class(NOTICE_SURFACE_CLASS)),
                "{notice:?}: a card nested in the notice keeps the X's room"
            );
            // A hidden card: no frame and no bare X.
            slot.card.set_visible(false);
            assert!(!slot.frame.get_visible(), "{notice:?}: an empty frame");
            assert!(!slot.close.get_visible(), "{notice:?}: a bare X");
        }

        shell.pick_tab(Tab::Traces);
        for (_, other) in &mounted.notices {
            other.set_visible(false);
        }
        let said = gtk::Label::new(Some("built from source"));
        card(Notice::Update).append(&said);
        // A card that rebuilds its labels says so, as the arming offer does.
        shell.notices_changed();
        card(Notice::Update).set_visible(true);
        card(Notice::Health).set_visible(true);
        let update = slot(shell, Notice::Update);
        let health = slot(shell, Notice::Health);
        assert_eq!(update.close.tooltip_text().as_deref(), Some(words.close));
        // Health is in front of Update; its X brings Update forward.
        assert_eq!(fronts(shell), vec![Notice::Health]);
        // Not wired without an `App`: apply the press as its handler would.
        shell.close_notice(Notice::Health);
        assert!(!health.frame.is_visible(), "the X did not close the card");
        assert_eq!(fronts(shell), vec![Notice::Update], "only that card");
        card(Notice::Health).set_visible(false);
        shell.close_notice(Notice::Update);
        assert!(!update.frame.is_visible(), "the X did not close the card");
        // The same card drawn again after going is new.
        update.card.set_visible(false);
        update.card.set_visible(true);
        assert!(
            update.frame.is_visible(),
            "a card that went and came back is new"
        );
        shell.close_notice(Notice::Update);
        assert!(!update.frame.is_visible());
        shell.apply();
        assert!(
            !update.frame.is_visible(),
            "a redraw reopened a closed card"
        );
        // New words are a new card, as soon as they change: nothing else is
        // redrawn here.
        said.set_text("an update is ready");
        assert!(update.frame.is_visible(), "new words stayed closed");
        // An install under way has no X.
        update.card.add_css_class(notices::STAYS_CLASS);
        assert!(!update.close.get_visible());
        update.card.remove_css_class(notices::STAYS_CLASS);
        assert!(update.close.get_visible());
        // Opening the window again shows every closed card.
        shell.close_notice(Notice::Update);
        assert!(!update.frame.is_visible());
        mounted.window.set_visible(false);
        mounted.window.set_visible(true);
        pump();
        assert!(
            update.frame.is_visible(),
            "reopening the window kept the card closed"
        );
        mounted.window.close();
    }

    /// The main pane is 440 wide and its content needs no sideways scroll.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_the_main_pane_never_scrolls_sideways() {
        let mounted = mounted(false);
        assert_eq!(
            mounted.shell.content_scroller().hscrollbar_policy(),
            gtk::PolicyType::Never
        );
        mounted.window.close();
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_a_hidden_graph_leaves_the_tab_the_whole_pane() {
        let mounted = mounted(false);
        let shell = &mounted.shell;
        // Off Traces the graph is hidden, and the tab's screen runs down to
        // the pane's bottom edge: a hidden footer takes none of the height
        // a screen could scroll in.
        for tab in [Tab::Home, Tab::Inference, Tab::Insights] {
            shell.pick_tab(tab);
            pump_until(|| shell.graph.root.height() == 0);
            assert!(!shell.graph.root.reveals_child(), "{tab:?}");
            assert_eq!(shell.graph.root.height(), 0, "{tab:?}");
            let scroller = shell.content_scroller();
            let main = scroller
                .parent()
                .and_then(|over_content| over_content.parent())
                .expect("the screens are in the main pane");
            let bounds = scroller.compute_bounds(&main).expect("placed");
            assert_eq!(
                (bounds.y() + bounds.height()).round() as i32,
                main.height(),
                "{tab:?}: the screen stops short of the pane's bottom"
            );
        }
        mounted.window.close();
    }

    // --- Fix pass 3 on #2160: whole buttons, no slivers -------------------

    use crate::ui::wrap_row::buttons_outside;

    /// A card as the offers draw theirs: a title that wraps, then two
    /// buttons in a row that wraps (`style::wrap_row`).
    fn offer_card(card: &gtk::Box) {
        card.add_css_class("tc-card");
        card.set_spacing(style::space::M);
        let title = gtk::Label::builder()
            .label("Use Private AI for the AI requests your tools make on this computer")
            .wrap(true)
            .xalign(0.0)
            .build();
        card.append(&title);
        let actions = style::wrap_row(style::space::M);
        for word in ["Not now", "Turn on"] {
            style::wrap_append(&actions, &gtk::Button::with_label(word));
        }
        card.append(&actions);
    }

    /// The Private AI offer on Ubuntu 24.04's GTK 4.14 lost its "Turn on":
    /// the stack laid over the tab was measured for one width and given
    /// another. Every button of the front card is drawn whole, inside the
    /// stack and inside the card, however wide the pane is.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_a_front_cards_buttons_are_whole_at_every_pane_width() {
        let mounted = mounted(false);
        let shell = &mounted.shell;
        for (notice, card) in &mounted.notices {
            if *notice == Notice::PrivateAiOffer {
                offer_card(card);
            } else {
                card.set_visible(false);
            }
        }
        shell.notices_changed();
        shell.pick_tab(Tab::Inference);
        let scroller = notice_scroller(shell);
        let overlay = scroller.parent().expect("the stack is laid over the tab");
        let front = slot(shell, Notice::PrivateAiOffer).card.clone();
        pump_until(|| overlay.width() > 0 && scroller.height() > 0);
        let pane = overlay.width();
        // The pane as it opens (440), then narrowed by 60, 120 and 180.
        for narrower in [0, 60, 120, 180] {
            overlay.set_margin_end(narrower);
            let expected = pane - narrower;
            pump_until(|| overlay.width() <= expected && scroller.height() > 0);
            pump();
            assert_eq!(
                scroller.width(),
                overlay.width(),
                "the stack is not the pane's width"
            );
            let outside_stack = buttons_outside(&front, scroller.upcast_ref());
            let outside_card = buttons_outside(&front, &front);
            assert!(
                outside_stack.is_empty() && outside_card.is_empty(),
                "pane {} wide: outside the stack {outside_stack:?}, outside the card \
                 {outside_card:?}",
                overlay.width()
            );
        }
        mounted.window.close();
    }

    /// The stack's ground is the pane's colour across the pane's whole width,
    /// so a heading the front card covers shows no sliver at the card's side
    /// (Home's screen is wider than the notices' column), as on the Mac,
    /// where the stack is exactly the tab's width.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_a_covered_heading_shows_nothing_beside_the_card() {
        let mounted = mounted(false);
        let shell = &mounted.shell;
        let page = shell
            .content
            .child_by_name("home")
            .and_downcast::<gtk::Box>()
            .expect("Home is a page");
        // Home's own inset, narrower than the notices' column.
        page.set_margin_start(style::space::M);
        page.set_margin_end(style::space::M);
        let heading = gtk::Label::builder()
            .label("Home, a heading as wide as the screen")
            .hexpand(true)
            .xalign(0.0)
            .build();
        heading.add_css_class("title-2");
        page.prepend(&heading);
        for (notice, card) in &mounted.notices {
            if *notice == Notice::Update {
                offer_card(card);
            } else {
                card.set_visible(false);
            }
        }
        shell.notices_changed();
        shell.pick_tab(Tab::Home);
        let scroller = notice_scroller(shell);
        let overlay = scroller.parent().expect("the stack is laid over the tab");
        pump_until(|| heading.is_mapped() && scroller.height() > 0);
        assert!(scroller.has_css_class(NOTICE_BACKDROP_CLASS));
        let stack = scroller.compute_bounds(&overlay).expect("placed");
        let title = heading.compute_bounds(&overlay).expect("placed");
        assert!(
            title.y() < stack.y() + stack.height(),
            "the heading {title:?} is not under the stack {stack:?}"
        );
        assert!(
            stack.x() <= title.x() && stack.x() + stack.width() >= title.x() + title.width(),
            "the heading {title:?} runs past the stack's ground {stack:?}"
        );
        mounted.window.close();
    }

    /// The ground is drawn: the stylesheet gives the backdrop the pane's own
    /// colour, as `.tc-root` has.
    #[test]
    fn the_notice_stacks_ground_is_the_panes_colour() {
        let css = include_str!("../style.css");
        let rule = |selector: &str| {
            css.split(&format!("{selector} {{"))
                .nth(1)
                .and_then(|rest| rest.split('}').next())
                .unwrap_or_else(|| panic!("no {selector} rule"))
                .to_string()
        };
        assert!(rule(".tc-root").contains("background-color: @tc_bg;"));
        assert!(
            rule(&format!(".{NOTICE_BACKDROP_CLASS}")).contains("background-color: @tc_bg;"),
            "the stack's ground is not the pane's colour"
        );
        // The X's room is kept on the card's surface alone.
        assert!(
            rule(&format!(".tc-notice-closable .{NOTICE_SURFACE_CLASS}"))
                .contains("padding-right: 48px;")
        );
        assert!(
            !css.contains(".tc-notice-closable .tc-card"),
            "every card nested in a closable notice gets the X's room"
        );
    }

    /// In the glass window the screens' `.tc-root` is transparent and the
    /// ground under the tab is the pane's own fill (`@tc_glass_pane`), not
    /// the shell's `@tc_bg`: with `@tc_bg` the stack was a warm band over
    /// the cool pane in light (#F6F7F4 on #f2f3f6) and an olive one over
    /// navy in dark (#23251D on #0d1124), on the Ubuntu 24.04 and 26.04
    /// captures. The stack in a glass pane takes the pane's fill.
    #[test]
    fn the_notice_stacks_ground_in_the_glass_pane_is_the_panes_fill() {
        let css = include_str!("../style.css");
        let rule = |sheet: &str, selector: &str| {
            sheet
                .split(&format!("{selector} {{"))
                .nth(1)
                .and_then(|rest| rest.split('}').next())
                .unwrap_or_else(|| panic!("no {selector} rule"))
                .to_string()
        };
        for sheet in [brand_tokens::GLASS_LIGHT_CSS, brand_tokens::GLASS_DARK_CSS] {
            assert!(rule(sheet, ".tc-glass-pane").contains("background-color: @tc_glass_pane;"));
        }
        assert!(rule(css, ".tc-glass-pane .tc-root").contains("background-color: transparent;"));
        assert!(
            rule(css, &format!(".tc-glass-pane .{NOTICE_BACKDROP_CLASS}"))
                .contains("background-color: @tc_glass_pane;"),
            "the stack's ground in a glass pane is not the pane's fill"
        );
        // While the window is not key the pane dims (`inactiveDim`), and the
        // stack's ground dims with it, or it would draw a band of the
        // undimmed fill across the dimmed pane.
        assert!(
            rule(
                css,
                &format!("window.tc-glass-window:backdrop .tc-glass-pane .{NOTICE_BACKDROP_CLASS}")
            )
            .contains("background-color: @tc_glass_pane_inactive;"),
            "the stack's ground does not follow the dimmed pane"
        );
    }

    /// The tab strip, measured with the app's stylesheet (`GlassRuleTabs`):
    /// four tabs of one width across the strip, each `segmentedTrack` +
    /// `space.s4` (38) high, the title in body type set medium and
    /// semibold once selected (`Navigation.swift:283-286`); the Private AI
    /// dot a 6px circle whatever the line height (`Navigation.swift:70`).
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_tab_strip_is_the_macs_rule_tabs() {
        let mounted = mounted(false);
        crate::ui::style::install();
        let shell = &mounted.shell;
        shell.pick_tab(Tab::Traces);
        shell.render_inference_state(
            trace_commons_contributor::private_inference_copy::STATE_LABELS[0],
        );
        let weight = |button: &gtk::ToggleButton| {
            let mut inside = Vec::new();
            descendants(button.upcast_ref(), &mut inside);
            inside
                .iter()
                .find_map(|widget| widget.downcast_ref::<gtk::Label>().cloned())
                .and_then(|title| title.pango_context().font_description())
                .map(|font| font.weight())
        };
        let expected = |tab: Tab| {
            if tab == Tab::Traces {
                gtk::pango::Weight::Semibold
            } else {
                gtk::pango::Weight::Medium
            }
        };
        // The stylesheet lands on a frame: wait for what is asserted.
        pump_until(|| {
            shell
                .tabs
                .iter()
                .all(|(tab, button)| button.width() > 0 && weight(button) == Some(expected(*tab)))
                && shell.inference_dot.width() == 6
        });
        // A homogeneous box gives the leftover pixels to the first tabs.
        let widths: Vec<i32> = shell
            .tabs
            .iter()
            .map(|(_, button)| button.width())
            .collect();
        let least = widths.iter().min().copied().unwrap_or(0);
        let most = widths.iter().max().copied().unwrap_or(0);
        assert!(least > 0 && most - least <= 1, "{widths:?}");
        for (tab, button) in &shell.tabs {
            let (height, ..) = button.measure(gtk::Orientation::Vertical, -1);
            assert_eq!(height, 38, "{tab:?}");
            assert_eq!(weight(button), Some(expected(*tab)), "{tab:?}");
        }
        let dot = &shell.inference_dot;
        assert!(dot.is_visible());
        assert_eq!(dot.measure(gtk::Orientation::Horizontal, -1).0, 6);
        assert_eq!(dot.measure(gtk::Orientation::Vertical, -1).0, 6);
        assert_eq!((dot.width(), dot.height()), (6, 6));
    }

    /// The toolbar, measured: `GlassToolbarButton`s 28 by 24 and the round
    /// Settings button 28 across (`GlassRoundButton`).
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_toolbar_controls_are_the_macs_size() {
        let mounted = mounted(false);
        crate::ui::style::install();
        let shell = &mounted.shell;
        let size = |widget: &gtk::Widget| {
            (
                widget.measure(gtk::Orientation::Horizontal, -1).0,
                widget.measure(gtk::Orientation::Vertical, -1).0,
            )
        };
        pump_until(|| size(shell.settings_button.upcast_ref()) == (28, 28));
        for toggle in [
            &shell.graph_toggle,
            &shell.map_toggle,
            &shell.inspector_toggle,
        ] {
            assert_eq!(size(toggle.upcast_ref()), (28, 24));
        }
        assert_eq!(size(shell.settings_button.upcast_ref()), (28, 28));
    }

    /// Increase Contrast strokes the pane as an overlay (`strokeBorder`):
    /// turning it on moves nothing.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_high_contrast_takes_no_layout_space() {
        let mounted = mounted(false);
        crate::ui::style::install();
        pump();
        let mut all = Vec::new();
        descendants(mounted.shell.root.upcast_ref(), &mut all);
        let pane = all
            .iter()
            .find(|widget| widget.has_css_class("tc-glass-pane"))
            .expect("a pane")
            .clone();
        let measure = |pane: &gtk::Widget| {
            (
                pane.measure(gtk::Orientation::Horizontal, -1),
                pane.measure(gtk::Orientation::Vertical, -1),
            )
        };
        // Once the stylesheet has landed (the toolbar's gear is its size).
        let gear = mounted.shell.settings_button.clone();
        pump_until(|| gear.measure(gtk::Orientation::Vertical, -1).0 == 28);
        mounted.window.remove_css_class(HIGH_CONTRAST_CLASS);
        pump();
        let plain = measure(&pane);
        mounted.window.add_css_class(HIGH_CONTRAST_CLASS);
        // A border would grow the pane by 2 each way on the next frame.
        pump_until(|| measure(&pane) != plain);
        assert_eq!(measure(&pane), plain);
    }

    /// The window's pixels at `points` (window coordinates), rendered from
    /// its widget tree, as premultiplied B, G, R, A.
    fn window_pixels(window: &adw::ApplicationWindow, points: &[(i32, i32)]) -> Vec<[u8; 4]> {
        let (width, height) = (window.width(), window.height());
        let paintable = gtk::WidgetPaintable::new(Some(window));
        // The paintable is empty until the window has drawn a frame.
        let node = std::cell::RefCell::new(None);
        pump_until(|| {
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
            *node.borrow_mut() = snapshot.to_node();
            node.borrow().is_some()
        });
        let node = node.into_inner().expect("a render node");
        let renderer = gtk::gsk::Renderer::for_surface(&window.surface().expect("a surface"))
            .expect("a renderer");
        // The window's own box: the shadow outside it is not the window.
        let viewport = gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
        let texture = renderer.render_texture(&node, Some(&viewport));
        renderer.unrealize();
        let stride = texture.width() as usize * 4;
        let mut pixels = vec![0u8; stride * texture.height() as usize];
        texture.download(&mut pixels, stride);
        points
            .iter()
            .map(|&(x, y)| {
                let at = y as usize * stride + x as usize * 4;
                [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
            })
            .collect()
    }

    /// A window's pixels at `points`, drawn from its widget tree, as R, G, B.
    fn any_window_pixels(window: &gtk::Window, points: &[(i32, i32)]) -> Vec<[u8; 3]> {
        let (width, height) = (window.width(), window.height());
        let paintable = gtk::WidgetPaintable::new(Some(window));
        let node = std::cell::RefCell::new(None);
        pump_until(|| {
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
            *node.borrow_mut() = snapshot.to_node();
            node.borrow().is_some()
        });
        let node = node.into_inner().expect("a render node");
        let renderer = gtk::gsk::Renderer::for_surface(&window.surface().expect("a surface"))
            .expect("a renderer");
        let viewport = gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
        let texture = renderer.render_texture(&node, Some(&viewport));
        renderer.unrealize();
        let stride = texture.width() as usize * 4;
        let mut pixels = vec![0u8; stride * texture.height() as usize];
        texture.download(&mut pixels, stride);
        points
            .iter()
            .map(|&(x, y)| {
                let at = y as usize * stride + x as usize * 4;
                [pixels[at + 2], pixels[at + 1], pixels[at]]
            })
            .collect()
    }

    /// A colour the painted glass sheet of `dark` defines, as R, G, B.
    fn glass_rgb(dark: bool, name: &str) -> [u8; 3] {
        let sheet = if dark {
            style::brand_tokens::GLASS_DARK_CSS
        } else {
            style::brand_tokens::GLASS_LIGHT_CSS
        };
        let hex = sheet
            .lines()
            .find_map(|line| line.strip_prefix(&format!("@define-color {name} #")))
            .unwrap_or_else(|| panic!("no opaque {name}"))
            .trim_end_matches(';')
            .trim()
            .to_owned();
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
        [byte(0), byte(1), byte(2)]
    }

    /// The Settings window and a sheet are drawn on the Monitor's own
    /// palette, its pane's navy and its card, in both schemes; on main they
    /// drew the legacy olive grey (#23251D) in dark.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_settings_and_a_sheet_take_the_monitors_palette() {
        let mounted = mounted(false);
        crate::ui::style::install();
        let manager = adw::StyleManager::default();
        let settings = mounted.shell.settings_window_for(&mounted.window);
        // A sheet as the Look inside sheet and the dialogs are made: a
        // plain window holding a card.
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("card");
        card.set_size_request(200, 120);
        card.set_halign(gtk::Align::Center);
        card.set_valign(gtk::Align::Center);
        let sheet = adw::Window::builder()
            .default_width(400)
            .default_height(300)
            .content(&card)
            .build();
        settings.present();
        sheet.present();
        for dark in [false, true] {
            manager.set_color_scheme(if dark {
                adw::ColorScheme::ForceDark
            } else {
                adw::ColorScheme::ForceLight
            });
            let end = std::time::Instant::now() + std::time::Duration::from_millis(500);
            pump_until(|| std::time::Instant::now() >= end);
            let pane = glass_rgb(dark, "tc_glass_window_pane");
            let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3);
            // The Settings window's ground, low in the window, clear of the
            // header bar and of anything the empty Settings page draws.
            let (w, h) = (settings.width(), settings.height());
            let ground = any_window_pixels(settings.upcast_ref(), &[(w / 2, h - 20)])[0];
            assert!(
                near(ground, pane),
                "dark={dark}: Settings draws {ground:?}, not {pane:?}"
            );
            let (w, h) = (sheet.width(), sheet.height());
            let drawn = any_window_pixels(sheet.upcast_ref(), &[(8, h - 8), (w / 2, h / 2)]);
            assert!(
                near(drawn[0], pane),
                "dark={dark}: the sheet draws {:?}",
                drawn[0]
            );
            let card = glass_rgb(dark, "tc_glass_window_card");
            assert!(
                near(drawn[1], card),
                "dark={dark}: its card draws {:?}, not {card:?}",
                drawn[1]
            );
        }
        manager.set_color_scheme(adw::ColorScheme::Default);
        sheet.destroy();
        settings.destroy();
        mounted.window.close();
    }

    /// A point in the gap between the main pane and the map pane, and one
    /// inside the map pane clear of its rounded corners, in window
    /// coordinates.
    fn gap_and_pane(mounted: &Mounted) -> ((i32, i32), (i32, i32)) {
        let mut all = Vec::new();
        descendants(mounted.shell.root.upcast_ref(), &mut all);
        let panes: Vec<gtk::graphene::Rect> = all
            .iter()
            .filter(|widget| widget.has_css_class("tc-glass-pane"))
            .filter_map(|pane| pane.compute_bounds(&mounted.window))
            .collect();
        assert_eq!(panes.len(), 3, "the three panes are shown");
        let (main, map) = (&panes[0], &panes[1]);
        let gap_x = ((main.x() + main.width() + map.x()) / 2.0) as i32;
        let middle = (main.y() + main.height() / 2.0) as i32;
        assert!(
            map.x() - (main.x() + main.width()) >= 4.0,
            "a gap between the panes: {main:?} {map:?}"
        );
        // The map draws over its pane (its field, art and transfer
        // plates); the pane's own fill is what is measured here, so the
        // map's drawing is taken out of the frame.
        mounted.shell.map.root.set_opacity(0.0);
        let pane = (
            (map.x() + 30.0) as i32,
            (map.y() + map.height() - 30.0) as i32,
        );
        ((gap_x, middle), pane)
    }

    /// Weston offers no `org_kde_kwin_blur_manager`, so the probe finds no
    /// grant and the window stays painted: its gaps are opaque.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_weston_offers_no_blur_so_the_window_stays_painted() {
        let mounted = mounted(false);
        crate::ui::style::install();
        let gear = mounted.shell.settings_button.clone();
        pump_until(|| gear.width() == 28);
        // Past any style transition (libadwaita's run about 200ms).
        let end = std::time::Instant::now() + std::time::Duration::from_millis(500);
        pump_until(|| std::time::Instant::now() >= end);
        let display = gtk::prelude::WidgetExt::display(&mounted.window);
        let capability = crate::ui::glass_backdrop::probe_capability(&display);
        assert_eq!(
            capability.backend,
            crate::ui::glass_backdrop::Backend::Wayland
        );
        assert!(!capability.granted, "{capability:?}");
        assert!(!crate::ui::style::is_translucent());
        let (gap, pane) = gap_and_pane(&mounted);
        let pixels = window_pixels(&mounted.window, &[gap, pane]);
        assert_eq!(pixels[0][3], 255, "gap {pixels:?}");
        assert_eq!(pixels[1][3], 255, "pane {pixels:?}");
        mounted.window.close();
    }

    /// The translucent sheet, as GTK applies it: the gaps show the desktop
    /// (the Mac's window draws no background), with nothing over it but the
    /// panes' own drop shadow (`paneEdge`, which the Mac draws over the
    /// desktop too); the panes take the material stack's alpha
    /// (`tc_glass_pane`, 0.7491 light, 0.745 dark); and all of it is opaque
    /// again when the sheet is taken off. Loaded by hand: weston grants no
    /// blur.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_translucent_sheet_clears_the_gaps_and_tints_the_panes() {
        let mounted = mounted(false);
        crate::ui::style::install();
        let gear = mounted.shell.settings_button.clone();
        pump_until(|| gear.width() == 28);
        let end = std::time::Instant::now() + std::time::Duration::from_millis(500);
        pump_until(|| std::time::Instant::now() >= end);
        let (gap, pane) = gap_and_pane(&mounted);
        let window = mounted.window.clone();
        // The window's own follower may load the painted sheet on its first
        // frame, so keep asking until the translucent one is what draws.
        pump_until(|| {
            crate::ui::style::set_translucent(true);
            window_pixels(&window, &[gap])[0][3] < 255
        });
        let dark = adw::StyleManager::default().is_dark();
        // The pane's drop shadow, from the generated painted sheet.
        let sheet = if dark {
            brand_tokens::GLASS_DARK_CSS
        } else {
            brand_tokens::GLASS_LIGHT_CSS
        };
        let shadow: Vec<f64> = sheet
            .lines()
            .find(|line| line.starts_with(".tc-glass-pane {"))
            .and_then(|line| line.split("rgba(").nth(1))
            .and_then(|rest| rest.split(')').next())
            .expect("the pane's shadow")
            .split(',')
            .map(|v| v.trim().parse().expect("a number"))
            .collect();
        let pixels = window_pixels(&mounted.window, &[gap, pane]);
        let [b, g, r, a] = pixels[0];
        // Premultiplied: no channel brighter than the shadow's colour at
        // its own alpha, and no more alpha than the shadow has.
        let most = |channel: f64| (channel * shadow[3]).ceil() as u8 + 1;
        assert!(
            a <= (shadow[3] * 255.0).ceil() as u8
                && r <= most(shadow[0])
                && g <= most(shadow[1])
                && b <= most(shadow[2]),
            "gap {pixels:?} under shadow {shadow:?}"
        );
        let expected = if dark { 0.745 } else { 0.7491 };
        let alpha = f64::from(pixels[1][3]) / 255.0;
        assert!((alpha - expected).abs() < 0.02, "pane {pixels:?}");
        crate::ui::style::set_translucent(false);
        pump_until(|| window_pixels(&window, &[gap])[0][3] == 255);
        let pixels = window_pixels(&mounted.window, &[gap, pane]);
        assert_eq!(pixels[0][3], 255, "gap {pixels:?}");
        assert_eq!(pixels[1][3], 255, "pane {pixels:?}");
        mounted.window.close();
    }

    /// Out of focus every fill drawn from the pane's ground dims with it
    /// (`GlassPaneFill`'s inactiveDim under the translucent tiers). The
    /// generated tiers and Insights carry their `:backdrop` twins (pinned
    /// in `test_generate.py`); every rule here that paints a ground-derived
    /// fill must have one too, or it would stay bright in a dimmed pane.
    #[test]
    fn every_hand_written_glass_fill_dims_out_of_focus() {
        // The sheet without its comments.
        let mut css = String::new();
        let mut rest = include_str!("../style.css");
        while let Some(start) = rest.find("/*") {
            css.push_str(&rest[..start]);
            rest = rest[start..]
                .split_once("*/")
                .map_or("", |(_, after)| after);
        }
        css.push_str(rest);
        let tiers = [
            "pane",
            "card",
            "card_quiet",
            "well",
            "control",
            "popover",
            "menu",
            "node_card",
            "rule",
        ];
        let rules: Vec<(String, String)> = css
            .split('}')
            .filter_map(|block| {
                let (selector, body) = block.split_once('{')?;
                let selector = selector.split_whitespace().collect::<Vec<_>>().join(" ");
                Some((selector, body.to_string()))
            })
            .collect();
        let mut checked = 0;
        for (selector, body) in &rules {
            if selector.starts_with("window.tc-glass-window:backdrop")
                || selector.contains(":hover")
            {
                continue;
            }
            for tier in tiers {
                let token = format!("@tc_glass_{tier}");
                // A fill or an edge in that tier's colour, not a text ink.
                let paints = body.split(';').any(|declaration| {
                    let Some((property, value)) = declaration.split_once(':') else {
                        return false;
                    };
                    let property = property.trim();
                    (property.starts_with("background") || property.starts_with("border"))
                        && value
                            .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '@'))
                            .any(|word| word == token)
                });
                if !paints {
                    continue;
                }
                checked += 1;
                let twin = format!("window.tc-glass-window:backdrop {selector}");
                let dimmed = format!("@tc_glass_{tier}_inactive;");
                assert!(
                    rules
                        .iter()
                        .any(|(other, body)| *other == twin && body.contains(&dimmed)),
                    "{selector} paints @tc_glass_{tier} with no unfocused twin"
                );
            }
        }
        // The pane's notice ground, a card, the tab rule and the gear.
        assert!(checked >= 4, "checked {checked}");
    }
}
