//! The graph under the Traces tab, shown while the toolbar's Graph is on.
//!
//! The GTK counterpart of `TracesGraphFooter` in
//! `macos/Sources/TraceCommonsApp/Views/Monitor/MonitorShell.swift`: the
//! legend, the bars, and the pills around the range: previous, the focus
//! button, zoom out, the range, zoom in, next, and jump to now. Every
//! number is `graph_model`'s and every word the core's.
//!
//! Deliberate platform differences: the bars swap in place on a period
//! step (the Mac slides them), and the focus button carries a themed icon
//! until the look slice exports the Mac's glyphs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use chrono::{DateTime, Utc};
use gtk::prelude::*;
use trace_commons_contributor::preview_copy::monitor_screens_copy;

use super::graph_model::{self, Bucket, Window};

/// The Mac's fixed footer height, rule included.
pub const HEIGHT: i32 = 206;

/// What the graph is drawn from.
#[derive(Clone, Debug, Default)]
pub struct GraphData {
    pub shared: Vec<DateTime<Utc>>,
    pub kept: Vec<DateTime<Utc>>,
    /// Whether History was read: when it was not, the shared figure is a
    /// dash, never a count drawn as a total.
    pub history_known: bool,
    /// The selection's tool, by its display name; the focus button can
    /// focus the map on it.
    pub tool: Option<String>,
}

struct State {
    window: Cell<Window>,
    hovered: Cell<Option<usize>>,
    data: RefCell<GraphData>,
    buckets: RefCell<Vec<Bucket>>,
    focused: Cell<bool>,
}

pub struct GraphFooter {
    pub root: gtk::Revealer,
    state: Rc<State>,
    bars: gtk::DrawingArea,
    shared_value: gtk::Label,
    kept_value: gtk::Label,
    range: gtk::Label,
    zoom_out: gtk::Button,
    zoom_in: gtk::Button,
    next: gtk::Button,
    jump: gtk::Button,
    pub focus: gtk::ToggleButton,
}

fn pill(glyph: &str, name: &str) -> gtk::Button {
    let button = gtk::Button::with_label(glyph);
    button.add_css_class("tc-quiet");
    button.set_tooltip_text(Some(name));
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    button
}

fn legend_cell(word: &str, class: &str) -> (gtk::Box, gtk::Label) {
    let cell = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = gtk::Label::new(Some(word));
    name.add_css_class("tc-meta");
    let value = gtk::Label::new(None);
    value.add_css_class("tc-ledger");
    value.add_css_class(class);
    cell.append(&name);
    cell.append(&value);
    (cell, value)
}

impl GraphFooter {
    pub fn new() -> Rc<Self> {
        let words = monitor_screens_copy();
        let graph = &words.traces_graph;

        let (shared_cell, shared_value) = legend_cell(words.shared, "tc-clear");
        let (kept_cell, kept_value) = legend_cell(words.kept, "tc-neutral");
        let legend = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        legend.append(&shared_cell);
        legend.append(&kept_cell);

        let bars = gtk::DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .build();
        bars.set_accessible_role(gtk::AccessibleRole::Img);

        let previous = pill("\u{2039}", words.previous);
        let focus = gtk::ToggleButton::new();
        focus.set_icon_name("zoom-fit-best-symbolic");
        focus.add_css_class("tc-quiet");
        let zoom_out = pill(graph.zoom_out_glyph, graph.zoom_out);
        let range = gtk::Label::builder()
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        range.add_css_class("tc-meta");
        let zoom_in = pill(graph.zoom_in_glyph, graph.zoom_in);
        let next = pill("\u{203a}", words.next);
        let jump = pill(graph.jump_to_now_glyph, graph.jump_to_now);
        let pills = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for widget in [
            previous.upcast_ref::<gtk::Widget>(),
            focus.upcast_ref(),
            zoom_out.upcast_ref(),
            range.upcast_ref(),
            zoom_in.upcast_ref(),
            next.upcast_ref(),
            jump.upcast_ref(),
        ] {
            pills.append(widget);
        }

        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(10)
            .margin_bottom(10)
            .margin_start(16)
            .margin_end(16)
            .build();
        column.append(&legend);
        column.append(&bars);
        column.append(&pills);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        // The rule above the footer.
        body.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        body.append(&column);
        body.set_size_request(-1, HEIGHT);

        // Never expanding: the bars' vexpand would otherwise pass up through
        // the revealer, hidden or not, and a hidden footer would take a
        // share of the main pane's height from the tab's screen.
        let root = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .vexpand(false)
            .child(&body)
            .build();
        follow_reduced_motion(&root);

        let footer = Rc::new(Self {
            root,
            state: Rc::new(State {
                window: Cell::new(Window::default()),
                hovered: Cell::new(None),
                data: RefCell::new(GraphData::default()),
                buckets: RefCell::new(Vec::new()),
                focused: Cell::new(false),
            }),
            bars,
            shared_value,
            kept_value,
            range,
            zoom_out,
            zoom_in,
            next,
            jump,
            focus,
        });

        let state = footer.state.clone();
        footer
            .bars
            .set_draw_func(move |area, context, width, height| {
                draw_bars(area, context, width, height, &state);
            });

        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&footer);
        motion.connect_motion(move |_, x, _| {
            if let Some(footer) = weak.upgrade() {
                let count = footer.state.buckets.borrow().len();
                let width = f64::from(footer.bars.width());
                let hovered = (count > 0 && width > 0.0)
                    .then(|| ((x / width * count as f64) as usize).min(count - 1));
                if footer.state.hovered.replace(hovered) != hovered {
                    footer.repaint();
                }
            }
        });
        let weak = Rc::downgrade(&footer);
        motion.connect_leave(move |_| {
            if let Some(footer) = weak.upgrade()
                && footer.state.hovered.take().is_some()
            {
                footer.repaint();
            }
        });
        footer.bars.add_controller(motion);

        let step = |button: &gtk::Button, change: fn(Window) -> Window| {
            let weak = Rc::downgrade(&footer);
            button.connect_clicked(move |_| {
                if let Some(footer) = weak.upgrade() {
                    footer.state.window.set(change(footer.state.window.get()));
                    footer.state.hovered.set(None);
                    footer.rebucket();
                }
            });
        };
        step(&previous, |window| window.stepped(-1));
        step(&footer.next, |window| window.stepped(1));
        step(&footer.zoom_out, |window| {
            window.zoomed(window.range.zoomed_out())
        });
        step(&footer.zoom_in, |window| {
            window.zoomed(window.range.zoomed_in())
        });
        step(&footer.jump, Window::jumped_to_now);

        footer.rebucket();
        footer
    }

    /// Draws the graph from new data. The range and the period stay where
    /// the person left them.
    pub fn render(&self, data: GraphData) {
        *self.state.data.borrow_mut() = data;
        self.rebucket();
    }

    /// Whether the map is focused on the selection's tool, for the focus
    /// button's state and tip.
    pub fn set_focused(&self, focused: bool) {
        self.state.focused.set(focused);
        self.repaint();
    }

    pub fn window(&self) -> Window {
        self.state.window.get()
    }

    fn rebucket(&self) {
        let window = self.state.window.get();
        let buckets = {
            let data = self.state.data.borrow();
            graph_model::buckets(
                &data.shared,
                &data.kept,
                window.offset,
                chrono::Local::now(),
                window.range,
            )
        };
        *self.state.buckets.borrow_mut() = buckets;
        self.repaint();
    }

    fn repaint(&self) {
        let words = monitor_screens_copy();
        let window = self.state.window.get();
        let buckets = self.state.buckets.borrow();
        let data = self.state.data.borrow();
        let hovered = self
            .state
            .hovered
            .get()
            .and_then(|index| buckets.get(index));
        let shared = hovered.map_or_else(|| buckets.iter().map(|b| b.shared).sum(), |b| b.shared);
        let kept = hovered.map_or_else(|| buckets.iter().map(|b| b.kept).sum(), |b| b.kept);
        self.shared_value
            .set_text(&graph_model::figure(shared, data.history_known));
        self.kept_value.set_text(&kept.to_string());
        let range = graph_model::range_label(
            window.range,
            window.offset,
            hovered.map(|b| b.start),
            &words.traces_graph,
        );
        self.range.set_text(&range);
        // The bars' text equivalent: the window, then each bar that has
        // anything in it, in the core's words.
        let described: Vec<String> = buckets
            .iter()
            .filter(|b| b.shared + b.kept > 0)
            .map(|b| graph_model::bar_description(b, &words.traces_graph))
            .collect();
        self.bars.update_property(&[
            gtk::accessible::Property::Label(&range),
            gtk::accessible::Property::Description(&described.join("\n")),
        ]);
        self.zoom_out
            .set_sensitive(window.range.zoomed_out().is_some());
        self.zoom_in
            .set_sensitive(window.range.zoomed_in().is_some());
        self.next.set_sensitive(!window.at_now());
        self.jump.set_sensitive(!window.at_now());
        let focused = self.state.focused.get() && data.tool.is_some();
        let tip = graph_model::focus_tip(data.tool.as_deref(), focused);
        self.focus.set_tooltip_text(Some(&tip));
        self.focus
            .update_property(&[gtk::accessible::Property::Label(&tip)]);
        self.focus.set_sensitive(data.tool.is_some());
        if self.focus.is_active() != focused {
            self.focus.set_active(focused);
        }
        self.bars.queue_draw();
    }
}

/// Shared above the middle line, kept below it, against a floor of five so
/// a single trace is not drawn as a full bar (`GlassBarGraph`'s
/// `scaleFloor`).
fn draw_bars(
    area: &gtk::DrawingArea,
    context: &gtk::cairo::Context,
    width: i32,
    height: i32,
    state: &State,
) {
    let buckets = state.buckets.borrow();
    if buckets.is_empty() || width <= 0 || height <= 0 {
        return;
    }
    #[allow(deprecated)]
    let ink = area.style_context().color();
    let (red, green, blue) = (
        f64::from(ink.red()),
        f64::from(ink.green()),
        f64::from(ink.blue()),
    );
    let labels = 14.0;
    let plot = (f64::from(height) - labels).max(2.0);
    let middle = plot / 2.0;
    let peak = buckets
        .iter()
        .map(|b| b.shared.max(b.kept))
        .max()
        .unwrap_or(0)
        .max(5) as f64;
    let slot = f64::from(width) / buckets.len() as f64;
    let bar = (slot * 0.6).max(1.0);
    for (index, bucket) in buckets.iter().enumerate() {
        let x = slot * index as f64 + (slot - bar) / 2.0;
        let lit = state.hovered.get() == Some(index);
        let up = middle * bucket.shared as f64 / peak;
        let down = middle * bucket.kept as f64 / peak;
        context.set_source_rgba(red, green, blue, if lit { 0.9 } else { 0.7 });
        context.rectangle(x, middle - up, bar, up);
        let _ = context.fill();
        context.set_source_rgba(red, green, blue, if lit { 0.5 } else { 0.3 });
        context.rectangle(x, middle, bar, down);
        let _ = context.fill();
        if !bucket.label.is_empty() {
            context.set_source_rgba(red, green, blue, 0.6);
            context.set_font_size(9.0);
            if let Ok(extents) = context.text_extents(&bucket.label) {
                context.move_to(
                    x + bar / 2.0 - extents.width() / 2.0,
                    f64::from(height) - 2.0,
                );
                let _ = context.show_text(&bucket.label);
            }
        }
    }
    context.set_source_rgba(red, green, blue, 0.12);
    context.rectangle(0.0, middle - 0.5, f64::from(width), 1.0);
    let _ = context.fill();
}

/// The footer opens and closes over the Mac's .25s, and at once when the
/// desktop has animations off.
pub fn follow_reduced_motion(reveal: &gtk::Revealer) {
    let duration = |animate: bool| if animate { 250 } else { 0 };
    match gtk::Settings::default() {
        Some(settings) => {
            reveal.set_transition_duration(duration(settings.is_gtk_enable_animations()));
            let weak = reveal.downgrade();
            settings.connect_gtk_enable_animations_notify(move |settings| {
                if let Some(reveal) = weak.upgrade() {
                    reveal.set_transition_duration(duration(settings.is_gtk_enable_animations()));
                }
            });
        }
        None => reveal.set_transition_duration(0),
    }
}

#[cfg(test)]
mod tests {
    use super::graph_model::Range;
    use super::*;

    #[test]
    fn the_footer_is_the_macs_height() {
        assert_eq!(HEIGHT, 206);
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_footer_steps_zooms_and_names_every_pill() {
        gtk::init().expect("GTK display");
        let footer = GraphFooter::new();
        let words = monitor_screens_copy();
        footer.render(GraphData {
            shared: vec![Utc::now()],
            kept: vec![Utc::now(), Utc::now()],
            history_known: true,
            tool: None,
        });
        assert_eq!(footer.shared_value.text(), "1");
        assert_eq!(footer.kept_value.text(), "2");
        assert_eq!(footer.range.text(), "Last 11 days");
        // At now there is no next period and nothing to jump to.
        assert!(!footer.next.is_sensitive());
        assert!(!footer.jump.is_sensitive());
        // Nothing selected: the focus button is off and says why.
        assert!(!footer.focus.is_sensitive());
        assert_eq!(
            footer.focus.tooltip_text().as_deref(),
            Some(words.shell.focus_needs_selection)
        );
        for (button, name) in [
            (&footer.zoom_out, words.traces_graph.zoom_out),
            (&footer.zoom_in, words.traces_graph.zoom_in),
            (&footer.next, words.next),
            (&footer.jump, words.traces_graph.jump_to_now),
        ] {
            assert_eq!(button.tooltip_text().as_deref(), Some(name));
        }

        footer.zoom_out.emit_clicked();
        assert_eq!(footer.window().range, Range::Weeks);
        assert!(!footer.zoom_out.is_sensitive(), "42 days is the widest");
        footer.zoom_in.emit_clicked();
        footer.zoom_in.emit_clicked();
        assert_eq!(footer.window().range, Range::Hours);
        assert!(!footer.zoom_in.is_sensitive(), "24 hours is the narrowest");
        assert_eq!(footer.range.text(), "Last 24 hours");

        // An unread History is a dash, never zero shared.
        footer.render(GraphData::default());
        assert_eq!(footer.shared_value.text(), "\u{2014}");
        assert_eq!(
            footer.window().range,
            Range::Hours,
            "new data keeps the range"
        );

        // A selected tool can be focused, and focusing names the way back.
        footer.render(GraphData {
            tool: Some("Codex".into()),
            ..GraphData::default()
        });
        assert!(footer.focus.is_sensitive());
        assert_eq!(
            footer.focus.tooltip_text().as_deref(),
            Some("Show Codex in the map")
        );
        footer.set_focused(true);
        assert!(footer.focus.is_active());
        assert_eq!(
            footer.focus.tooltip_text().as_deref(),
            Some(words.shell.focus_whole_map)
        );
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_footer_opens_at_once_under_reduced_motion() {
        gtk::init().expect("GTK display");
        let settings = gtk::Settings::default().expect("settings");
        let previous = settings.is_gtk_enable_animations();
        let footer = GraphFooter::new();
        settings.set_gtk_enable_animations(false);
        assert_eq!(footer.root.transition_duration(), 0);
        settings.set_gtk_enable_animations(true);
        assert_eq!(footer.root.transition_duration(), 250);
        settings.set_gtk_enable_animations(previous);
    }
}
