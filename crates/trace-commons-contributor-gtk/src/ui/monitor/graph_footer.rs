//! The graph under the Traces tab, shown while the toolbar's Graph is on.
//!
//! The GTK counterpart of `TracesGraphFooter` in
//! `macos/Sources/TraceCommonsApp/Views/Monitor/MonitorShell.swift`: the
//! legend, the bars, and the pills around the range: previous, the focus
//! button, zoom out, the range, zoom in, next, and jump to now. Every
//! number is `graph_model`'s and every word the core's.
//!
//! The bars are `GlassBarGraph`'s tapered dot matrix: every period is a
//! faint dot column, lit from the axis out in proportion to its count. The
//! legend is two equal capsule wells (`GlassLegendCell`), and the controls
//! are round glass pills (`GlassPillIconButton`).
//!
//! How it moves, as the Mac's (`map_motion` holds the numbers): a period
//! step brings the new bars in from its side, fading in, over 0.45 s on
//! `(0.4, 0, 0.2, 1)`, while the old ones fade out, moving toward the side
//! they themselves came in from (SwiftUI removes a view with the
//! transition of its last render); a zoom crossfades them in place over
//! 0.15 s on the token curve (`Swap`). The
//! footer opens from 0 to its 206 points and closes again over 0.25 s on
//! the token curve, moving up from the window's foot and fading
//! (`Drawer`). Each is at its end at once with animations off, followed
//! live.
//!
//! Deliberate platform difference: where the Mac draws SF Symbols, which
//! are Apple's, the pills draw stock symbolic icons (`pill_symbol`, and
//! `find-location` for the binoculars).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use chrono::{DateTime, Utc};
use gtk::prelude::*;
use trace_commons_contributor::preview_copy::monitor_screens_copy;

use super::glass_paint;
use super::graph_model::{self, Bucket, Window};
use super::map_motion::{self, Edge};
use crate::ui::glass_motion::{self, Driver};
use gtk::subclass::prelude::ObjectSubclassIsExt;

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
    /// The bars on their way out while a step or a zoom swaps them.
    swap: RefCell<Option<Swap>>,
    swapping: Driver,
    /// The edge of the last swap (`TracesGraphFooter.slide`): none at
    /// first and after a zoom. The bars it brought in leave with it.
    slide: Cell<Option<Edge>>,
}

/// The bars of the period before a step or a zoom, on their way out:
/// the edge they came in from (`was`), the side the step slides from
/// (none for a zoom), and how far in it is.
#[derive(Clone, Debug)]
pub struct Swap {
    pub old: Vec<Bucket>,
    pub was: Option<Edge>,
    pub edge: Option<Edge>,
    pub t: f64,
}

pub struct GraphFooter {
    pub root: Drawer,
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
    pills: gtk::Box,
    column: gtk::Box,
    body: gtk::Box,
    rule: gtk::Separator,
}

/// The class every graph control wears: a round glass control
/// (`GlassPillIconButton`, 30 by `size.control`).
const PILL_CLASS: &str = "tc-graph-pill";

/// The range label's capsule: `ink(0.07)`, primary ink, 8 each side,
/// `size.control` high.
const RANGE_CLASS: &str = "tc-graph-range";

/// The focus button's icon. The Mac draws the SF Symbol `binoculars`, which
/// is Apple's; GTK draws the stock symbolic icon for finding a place on a
/// map, which Adwaita and Breeze both carry.
const FOCUS_ICON: &str = "find-location-symbolic";

/// The focus button's glyph ink (`TracesGraphFooter.focusInk`): only the
/// glyph changes, never the pill's fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusInk {
    /// `graphFocusOn`: the map is focused on the tool.
    On,
    /// `graphFocusIdle`: a tool can be focused.
    Idle,
    /// `graphFocusOff`: nothing to focus on.
    Off,
}

impl FocusInk {
    pub fn of(focused: bool, can_focus: bool) -> Self {
        if !can_focus {
            Self::Off
        } else if focused {
            Self::On
        } else {
            Self::Idle
        }
    }

    pub fn css(self) -> &'static str {
        match self {
            Self::On => "tc-focus-on",
            Self::Idle => "tc-focus-idle",
            Self::Off => "tc-focus-off",
        }
    }
}

/// The stock symbolic icon drawn for a known text glyph, as the Mac's
/// `GlassPillIconButton.symbol(for:)` draws an SF Symbol for it
/// (`Controls.swift:424-432`): the glyph stays the core's word for the
/// control, and an unknown glyph is drawn as text. Each name is in the
/// Adwaita icon theme (GNOME 46 on) and in Breeze.
pub fn pill_symbol(glyph: &str) -> Option<&'static str> {
    match glyph {
        // chevron.left
        "\u{2039}" | "<" => Some("go-previous-symbolic"),
        // chevron.right
        "\u{203a}" | ">" => Some("go-next-symbolic"),
        // minus
        "\u{2212}" | "-" => Some("list-remove-symbolic"),
        // plus
        "+" => Some("list-add-symbolic"),
        // chevron.right.to.line
        "\u{203a}|" | ">|" => Some("go-last-symbolic"),
        _ => None,
    }
}

/// The pills' glyph size (`glassGlyph(11, weight: .semibold)`).
const PILL_GLYPH: i32 = 11;

fn pill_icon(name: &str) -> gtk::Image {
    let icon = gtk::Image::from_icon_name(name);
    icon.set_pixel_size(PILL_GLYPH);
    icon
}

fn pill(glyph: &str, name: &str) -> gtk::Button {
    let button = match pill_symbol(glyph) {
        Some(icon) => gtk::Button::builder().child(&pill_icon(icon)).build(),
        None => gtk::Button::with_label(glyph),
    };
    button.add_css_class("tc-glass-control");
    button.add_css_class(PILL_CLASS);
    button.set_tooltip_text(Some(name));
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    button
}

/// Which series a legend cell or a dot stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Series {
    Shared,
    Kept,
}

impl Series {
    /// The series' ink in a scheme (`dataShared`, `dataKept`).
    fn ink(self, dark: bool) -> &'static str {
        use crate::ui::style::brand_tokens::glass::data;
        match (self, dark) {
            (Series::Shared, false) => data::SHARED_LIGHT,
            (Series::Shared, true) => data::SHARED_DARK,
            (Series::Kept, false) => data::KEPT_LIGHT,
            (Series::Kept, true) => data::KEPT_DARK,
        }
    }
}

fn dark_scheme() -> bool {
    adw::StyleManager::default().is_dark()
}

/// `GlassStatusDot` at `size.dotLarge` (8) with its halo, in a box just
/// big enough for the halo, so the word sits 7 past the dot as on the Mac.
fn legend_dot(series: Series) -> gtk::DrawingArea {
    let size = 8.0;
    let area = gtk::DrawingArea::builder()
        .content_width(12)
        .content_height(12)
        .valign(gtk::Align::Center)
        .build();
    area.set_accessible_role(gtk::AccessibleRole::Presentation);
    area.set_draw_func(move |_, context, width, height| {
        let dark = dark_scheme();
        glass_paint::paint_status_dot(
            context,
            f64::from(width) / 2.0,
            f64::from(height) / 2.0,
            size,
            series.ink(dark),
            true,
            dark,
        );
    });
    area
}

/// `GlassLegendCell`: a capsule well, the dot and the word on the left and
/// the figure on the right, semibold and tabular.
pub(super) fn legend_cell(word: &str, series: Series) -> (gtk::Box, gtk::Label) {
    let cell = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    cell.add_css_class("tc-glass-well");
    cell.add_css_class("tc-graph-legend-cell");
    cell.set_hexpand(true);
    // 7 from the dot's 8 to the word: the halo's box is 2 wider each side.
    let lead = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    lead.append(&legend_dot(series));
    let name = gtk::Label::builder()
        .label(word)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .xalign(0.0)
        .build();
    name.add_css_class("tc-graph-legend-name");
    lead.append(&name);
    lead.set_hexpand(true);
    let value = gtk::Label::new(None);
    value.add_css_class("tc-graph-legend-value");
    cell.append(&lead);
    cell.append(&value);
    (cell, value)
}

/// Dots in each half of a column, from the axis out (`GlassBarGraph.rows`).
pub const DOT_ROWS: usize = 7;

/// The space between the matrix and the column's top and foot.
const MATRIX_END: f64 = 6.0;

/// Each column's matrix is a fixed 96 high (`GlassBarGraph`'s canvas,
/// `.frame(height: 96)`), the columns top-aligned, the label under it.
pub const MATRIX_HEIGHT: f64 = 96.0;

/// Between the matrix and its label (`space.s2`).
const LABEL_GAP: f64 = 4.0;

/// The label row's least height (`.frame(minHeight: 12)`).
const LABEL_MIN: f64 = 12.0;

/// A bar label's size: caption (11), or micro (10) set regular in a thin
/// graph (`GlassTokens.swift:722,724`).
pub fn label_size(thin: bool) -> f64 {
    if thin { 10.0 } else { 11.0 }
}

/// The label's line: caption's 16, micro's 13.
fn label_line(thin: bool) -> f64 {
    if thin { 13.0 } else { 16.0 }
}

/// The tapered matrix (`GlassBarGraph.dotAlphas`): each half of a column is
/// `rows` dots, lit from the axis out in proportion to the value. Lit dots
/// brighten toward the tip, the last carries the remainder as a partial
/// dot, a count of one always lights a dot, and 0 is an unlit dot.
pub fn dot_alphas(value: f64, maximum: f64, rows: usize) -> Vec<f64> {
    if value.is_nan() || value <= 0.0 || maximum <= 0.0 {
        return vec![0.0; rows];
    }
    let lit = (value / maximum * rows as f64).clamp(1.0, rows as f64);
    let tip = lit.ceil();
    (0..rows)
        .map(|row| {
            let fill = (lit - row as f64).clamp(0.0, 1.0);
            if fill <= 0.0 {
                return 0.0;
            }
            let taper = 0.35 + 0.65 * (row as f64 + 1.0) / tip;
            taper * if fill < 1.0 { fill.max(0.35) } else { 1.0 }
        })
        .collect()
}

/// The gap between columns and the matrix's inset inside each
/// (`GlassBarGraph.gaps`, never compact here): a thin graph's, one dot
/// across, or the full graph's.
pub fn gaps(thin: bool) -> (f64, f64) {
    if thin { (2.0, 1.0) } else { (6.0, 5.0) }
}

/// Over 14 periods the graph is thin: one dot across, smaller.
pub fn is_thin(count: usize) -> bool {
    count > 14
}

/// Each column's left edge and width, `count` columns across `width` with
/// the graph's gap between them.
pub fn columns(width: f64, count: usize) -> Vec<(f64, f64)> {
    if count == 0 || width <= 0.0 {
        return Vec::new();
    }
    let (between, _) = gaps(is_thin(count));
    let column = ((width - between * (count - 1) as f64) / count as f64).max(1.0);
    (0..count)
        .map(|index| (index as f64 * (column + between), column))
        .collect()
}

impl GraphFooter {
    pub fn new() -> Rc<Self> {
        let words = monitor_screens_copy();
        let graph = &words.traces_graph;

        let (shared_cell, shared_value) = legend_cell(words.shared, Series::Shared);
        let (kept_cell, kept_value) = legend_cell(words.kept, Series::Kept);
        // Two equal cells, `space.s3` apart.
        let legend = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        legend.set_homogeneous(true);
        legend.append(&shared_cell);
        legend.append(&kept_cell);

        let bars = gtk::DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .content_height((MATRIX_HEIGHT + LABEL_GAP + LABEL_MIN) as i32)
            .build();
        bars.set_accessible_role(gtk::AccessibleRole::Img);

        let previous = pill("\u{2039}", words.previous);
        let focus = gtk::ToggleButton::new();
        focus.set_child(Some(&pill_icon(FOCUS_ICON)));
        focus.add_css_class("tc-glass-control");
        focus.add_css_class(PILL_CLASS);
        focus.add_css_class(FocusInk::Off.css());
        let zoom_out = pill(graph.zoom_out_glyph, graph.zoom_out);
        let range = gtk::Label::builder()
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        range.add_css_class("tc-type-label");
        // The range sits in its own capsule (`MonitorShell.swift:249-257`).
        range.add_css_class(RANGE_CLASS);
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
        let rule = gtk::Separator::new(gtk::Orientation::Horizontal);
        body.append(&rule);
        body.append(&column);
        body.set_size_request(-1, HEIGHT);

        let root = Drawer::new(&body);

        let footer = Rc::new(Self {
            root,
            state: Rc::new(State {
                window: Cell::new(Window::default()),
                hovered: Cell::new(None),
                data: RefCell::new(GraphData::default()),
                buckets: RefCell::new(Vec::new()),
                focused: Cell::new(false),
                swap: RefCell::new(None),
                swapping: Driver::default(),
                slide: Cell::new(None),
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
            pills,
            column,
            body,
            rule,
        });

        let state = footer.state.clone();
        footer.bars.set_draw_func(move |_, context, width, _| {
            draw_bars(context, width, &state);
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

        // Each control's swap: previous slides the bars from the trailing
        // edge, next and jump to now from the leading one, a zoom swaps
        // them in place (`TracesGraphFooter.step`, `.zoom`).
        let step = |button: &gtk::Button, edge: Option<Edge>, change: fn(Window) -> Window| {
            let weak = Rc::downgrade(&footer);
            button.connect_clicked(move |_| {
                if let Some(footer) = weak.upgrade() {
                    footer.state.window.set(change(footer.state.window.get()));
                    footer.state.hovered.set(None);
                    footer.swap_bars(edge);
                }
            });
        };
        step(&previous, Some(Edge::Trailing), |window| window.stepped(-1));
        step(&footer.next, Some(Edge::Leading), |window| {
            window.stepped(1)
        });
        step(&footer.zoom_out, None, |window| {
            window.zoomed(window.range.zoomed_out())
        });
        step(&footer.zoom_in, None, |window| {
            window.zoomed(window.range.zoomed_in())
        });
        step(&footer.jump, Some(Edge::Leading), Window::jumped_to_now);

        // Animations turned off part-way: the bars are at their end.
        if let Some(settings) = gtk::Settings::default() {
            let weak = Rc::downgrade(&footer);
            settings.connect_gtk_enable_animations_notify(move |settings| {
                if let Some(footer) = weak.upgrade()
                    && !settings.is_gtk_enable_animations()
                {
                    footer.state.swapping.finish();
                    footer.state.swap.replace(None);
                    footer.bars.queue_draw();
                }
            });
        }

        footer.rebucket();
        footer
    }

    /// The graph as the edge rail's Traces peek and the menu-bar panel draw
    /// it: the legend over the bars, `height` high, shown, with no rule,
    /// margins or controls (the Mac's `GlassBarGraph` there has none).
    pub fn compact(&self, height: i32) {
        self.rule.set_visible(false);
        self.pills.set_visible(false);
        self.body.set_size_request(-1, -1);
        for margin in [
            gtk::Widget::set_margin_top,
            gtk::Widget::set_margin_bottom,
            gtk::Widget::set_margin_start,
            gtk::Widget::set_margin_end,
        ] {
            margin(self.column.upcast_ref(), 0);
        }
        self.bars.set_size_request(-1, height);
        self.bars.set_vexpand(false);
        self.root.set_transition_duration(0);
        self.root.set_reveal_child(true);
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

    /// The bars swapping now, if they are.
    pub fn swap(&self) -> Option<Swap> {
        self.state.swap.borrow().clone()
    }

    /// The new period's bars, the old ones swapping out toward `edge`
    /// (a step) or in place (a zoom), on the bars' frame clock; at once
    /// with animations off.
    fn swap_bars(&self, edge: Option<Edge>) {
        self.state.swapping.finish();
        let old = self.state.buckets.borrow().clone();
        let was = self.state.slide.replace(edge);
        self.rebucket();
        if !glass_motion::animations_enabled() {
            self.state.swap.replace(None);
            return;
        }
        self.state.swap.replace(Some(Swap {
            old,
            was,
            edge,
            t: 0.0,
        }));
        let state = self.state.clone();
        let done = self.state.clone();
        let bars = self.bars.clone();
        let drawn = self.bars.clone();
        self.state.swapping.start(
            &self.bars,
            map_motion::bars_swap_ease(edge).length(),
            0.0,
            move |t| {
                if let Some(swap) = state.swap.borrow_mut().as_mut() {
                    swap.t = t;
                }
                bars.queue_draw();
            },
            move || {
                done.swap.replace(None);
                drawn.queue_draw();
            },
        );
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
        let ink = FocusInk::of(self.state.focused.get(), data.tool.is_some());
        for each in [FocusInk::On, FocusInk::Idle, FocusInk::Off] {
            if each != ink {
                self.focus.remove_css_class(each.css());
            }
        }
        self.focus.add_css_class(ink.css());
        self.bars.queue_draw();
    }
}

/// Draws the graph for the footer's state: while a step or a zoom swaps
/// the bars, the old set going and the new coming (`paint_swap`).
fn draw_bars(context: &gtk::cairo::Context, width: i32, state: &State) {
    let buckets = state.buckets.borrow();
    let swap = state.swap.borrow();
    match swap.as_ref() {
        Some(swap) => paint_swap(
            context,
            f64::from(width),
            swap,
            &buckets,
            state.hovered.get(),
            dark_scheme(),
        ),
        None => paint_bars(
            context,
            f64::from(width),
            &buckets,
            state.hovered.get(),
            dark_scheme(),
        ),
    }
}

/// The bars part-way through `swap`: each set offset across the bars'
/// width and faded as `map_motion::bars_swap` has it, the old under the
/// new.
pub fn paint_swap(
    context: &gtk::cairo::Context,
    width: f64,
    swap: &Swap,
    buckets: &[Bucket],
    hovered: Option<usize>,
    dark: bool,
) {
    let ((old_x, old_alpha), (new_x, new_alpha)) =
        map_motion::bars_swap(swap.was, swap.edge, swap.t);
    for (bars, x, alpha, hovered) in [
        (&swap.old[..], old_x, old_alpha, None),
        (buckets, new_x, new_alpha, hovered),
    ] {
        if alpha <= 0.0 {
            continue;
        }
        let _ = context.save();
        context.push_group();
        context.translate(x * width, 0.0);
        paint_bars(context, width, bars, hovered, dark);
        let _ = context.pop_group_to_source();
        let _ = context.paint_with_alpha(alpha);
        let _ = context.restore();
    }
}

/// `GlassBarGraph`: shared rises from the axis and kept hangs from it, as
/// tapered dot columns, against a floor of five so a single trace is not
/// drawn as a full column (`scaleFloor`). Each column is its own 96-high
/// canvas, as on the Mac: its unlit dots in `ink(0.09)` (0.2 under the
/// pointer), the hovered one on an `ink(0.07)` wash, and its own stretch of
/// the 1pt `ink(0.14)` axis rule, so the rule breaks at the gaps. Labels
/// sit under the columns in `statusOff`, primary under the pointer.
pub fn paint_bars(
    context: &gtk::cairo::Context,
    width: f64,
    buckets: &[Bucket],
    hovered: Option<usize>,
    dark: bool,
) {
    use crate::ui::style::brand_tokens::glass::data;
    if buckets.is_empty() || width <= 0.0 {
        return;
    }
    let shared_ink = Series::Shared.ink(dark);
    let kept_ink = Series::Kept.ink(dark);
    let thin = is_thin(buckets.len());
    let (_, inset) = gaps(thin);
    let (across_dots, radius) = if thin { (1, 1.5) } else { (3, 1.9) };
    let half = MATRIX_HEIGHT / 2.0;
    // Rows keep one pitch in every column, whatever its width.
    let pitch = (half - 1.0 - MATRIX_END) / DOT_ROWS as f64;
    let maximum = buckets
        .iter()
        .map(|b| b.shared.max(b.kept))
        .max()
        .unwrap_or(0)
        .max(5) as f64;
    let set = |(red, green, blue, alpha): (f64, f64, f64, f64)| {
        context.set_source_rgba(red, green, blue, alpha);
    };
    let dot = |x: f64, y: f64, alpha: f64, lit: &str, unlit: (f64, f64, f64, f64)| {
        if alpha > 0.0 {
            let (red, green, blue) = glass_paint::channels(lit);
            context.set_source_rgba(red, green, blue, alpha);
        } else {
            set(unlit);
        }
        context.new_path();
        context.arc(x, y, radius, 0.0, std::f64::consts::TAU);
        let _ = context.fill();
    };
    for (index, (bucket, (left, column))) in buckets
        .iter()
        .zip(columns(width, buckets.len()))
        .enumerate()
    {
        let is_hovered = hovered == Some(index);
        if is_hovered {
            let corner = if thin {
                3.0
            } else {
                f64::from(crate::ui::style::brand_tokens::glass::radius::CONTROL)
            };
            context.new_path();
            rounded_rect(context, left, 0.0, column, MATRIX_HEIGHT, corner);
            set(glass_paint::ink(dark, 0.07));
            let _ = context.fill();
        }
        let unlit = glass_paint::ink(dark, if is_hovered { 0.2 } else { 0.09 });
        let across = pitch.min((column - inset * 2.0) / across_dots as f64);
        let first = left + (column - across * across_dots as f64) / 2.0 + across / 2.0;
        let up = dot_alphas(bucket.shared as f64, maximum, DOT_ROWS);
        let down = dot_alphas(bucket.kept as f64, maximum, DOT_ROWS);
        for dot_column in 0..across_dots {
            let x = first + dot_column as f64 * across;
            for row in 0..DOT_ROWS {
                let offset = pitch * (row as f64 + 0.5) + 0.5;
                dot(x, half - offset, up[row], shared_ink, unlit);
                dot(x, half + offset, down[row], kept_ink, unlit);
            }
        }
        // This column's stretch of the axis rule.
        context.new_path();
        context.rectangle(left, half - 0.5, column, 1.0);
        set(glass_paint::ink(dark, 0.14));
        let _ = context.fill();
        if !bucket.label.is_empty() {
            let ink = match (is_hovered, dark) {
                (true, false) => data::TEXT_PRIMARY_LIGHT,
                (true, true) => data::TEXT_PRIMARY_DARK,
                (false, false) => data::LABEL_LIGHT,
                (false, true) => data::LABEL_DARK,
            };
            let (red, green, blue) = glass_paint::channels(ink);
            context.set_source_rgb(red, green, blue);
            let size = label_size(thin);
            context.set_font_size(size);
            if let Ok(extents) = context.text_extents(&bucket.label) {
                // Centred under the column; in a thin graph the first and
                // last hug the outer edge, so neither is cut.
                let last = index + 1 == buckets.len();
                let x = if thin && index == 0 {
                    left
                } else if thin && last {
                    left + column - extents.x_advance()
                } else {
                    left + column / 2.0 - extents.x_advance() / 2.0
                };
                // On the label line's baseline, the line under the gap.
                let line = label_line(thin);
                let baseline = MATRIX_HEIGHT + LABEL_GAP + (line - size) / 2.0 + size * 0.8;
                context.move_to(x, baseline);
                let _ = context.show_text(&bucket.label);
            }
        }
    }
}

fn rounded_rect(context: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::FRAC_PI_2;
    let r = r.min(w / 2.0).min(h / 2.0);
    context.new_sub_path();
    context.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    context.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    context.arc(x + r, y + h - r, r, FRAC_PI_2, 2.0 * FRAC_PI_2);
    context.arc(x + r, y + r, r, 2.0 * FRAC_PI_2, 3.0 * FRAC_PI_2);
    context.close_path();
}

mod drawer {
    use super::*;
    use gtk::glib;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub struct Drawer {
        pub child: RefCell<Option<gtk::Widget>>,
        /// How far open it is drawn, 0 to 1, and whether it is opening.
        pub shown: Cell<f64>,
        pub open: Cell<bool>,
        pub driver: Driver,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Drawer {
        const NAME: &'static str = "TcGraphDrawer";
        type Type = super::Drawer;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Drawer {
        fn dispose(&self) {
            if let Some(child) = self.child.take() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Drawer {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(child) = self.child.borrow().clone() else {
                return (0, 0, -1, -1);
            };
            let (minimum, natural, _, _) = child.measure(orientation, for_size);
            match orientation {
                gtk::Orientation::Vertical => {
                    let height = (f64::from(natural) * self.shown.get().max(0.0)).round() as i32;
                    (height, height, -1, -1)
                }
                _ => (minimum, natural, -1, -1),
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, baseline: i32) {
            let Some(child) = self.child.borrow().clone() else {
                return;
            };
            // The footer at its whole height from the drawer's top: the
            // part not yet open lies past the window's foot, clipped.
            let (_, natural, _, _) = child.measure(gtk::Orientation::Vertical, width);
            child.allocate(width, natural, baseline, None);
        }
    }
}

gtk::glib::wrapper! {
    /// The footer's holder: it opens from nothing to the footer's height
    /// and closes again over `Motion.slide` on the token curve, the footer
    /// moving up from the foot and fading in as it goes
    /// (`MonitorWindowView`'s `.move(edge: .bottom)` with `.opacity`);
    /// at once with animations off, followed live.
    pub struct Drawer(ObjectSubclass<drawer::Drawer>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Drawer {
    pub fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let drawer: Self = gtk::glib::Object::new();
        drawer.set_overflow(gtk::Overflow::Hidden);
        // Never expanding: the bars' vexpand would otherwise pass up, and a
        // hidden footer would take a share of the main pane's height.
        drawer.set_vexpand(false);
        drawer.set_hexpand(true);
        child.set_parent(&drawer);
        drawer.imp().child.replace(Some(child.clone().upcast()));
        drawer.apply(0.0);
        if let Some(settings) = gtk::Settings::default() {
            let weak = drawer.downgrade();
            settings.connect_gtk_enable_animations_notify(move |settings| {
                if let Some(drawer) = weak.upgrade()
                    && !settings.is_gtk_enable_animations()
                {
                    drawer.imp().driver.finish();
                }
            });
        }
        drawer
    }

    /// Opens or closes it.
    pub fn set_reveal_child(&self, open: bool) {
        let imp = self.imp();
        if imp.open.replace(open) == open {
            // Asked again on the way (the shell syncs often): it keeps
            // going; at rest it is where it should be.
            if !imp.driver.is_running() {
                self.apply(if open { 1.0 } else { 0.0 });
            }
            return;
        }
        let from = imp.shown.get();
        let to = if open { 1.0 } else { 0.0 };
        let weak = self.downgrade();
        imp.driver.start(
            self,
            map_motion::footer().length(),
            0.0,
            move |t| {
                if let Some(drawer) = weak.upgrade() {
                    drawer.apply(map_motion::drawer_at(from, to, t));
                }
            },
            || {},
        );
    }

    /// Whether it is open, or opening.
    pub fn reveals_child(&self) -> bool {
        self.imp().open.get()
    }

    /// How far open it is drawn now, 0 to 1.
    pub fn fraction(&self) -> f64 {
        self.imp().shown.get()
    }

    fn apply(&self, shown: f64) {
        let imp = self.imp();
        imp.shown.set(shown);
        if let Some(child) = imp.child.borrow().as_ref() {
            child.set_opacity(shown.clamp(0.0, 1.0));
            // Closed, it takes no focus and no pointer.
            child.set_child_visible(shown > 0.0 || imp.open.get());
        }
        self.queue_resize();
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

    /// `GlassBarGraph.dotAlphas`, as the Mac's parity test pins it: nothing
    /// lit for nothing, one dot for a count of one, a full column at the
    /// maximum brightening to the tip, and a partial last dot that keeps 9
    /// apart from 11.
    #[test]
    fn the_dot_columns_taper_as_the_macs() {
        assert_eq!(dot_alphas(0.0, 40.0, DOT_ROWS), vec![0.0; 7]);
        let one = dot_alphas(1.0, 40.0, DOT_ROWS);
        assert_eq!(one.iter().filter(|a| **a > 0.0).count(), 1);
        assert!((one[0] - 1.0).abs() < 1e-4);
        let full = dot_alphas(40.0, 40.0, DOT_ROWS);
        assert_eq!(full.len(), 7);
        assert!((full[6] - 1.0).abs() < 1e-4);
        let mut sorted = full.clone();
        sorted.sort_by(f64::total_cmp);
        assert_eq!(full, sorted);
        assert_ne!(
            dot_alphas(9.0, 40.0, DOT_ROWS),
            dot_alphas(11.0, 40.0, DOT_ROWS)
        );
        assert!(
            dot_alphas(f64::INFINITY, 40.0, DOT_ROWS)
                .iter()
                .all(|a| *a <= 1.0)
        );
    }

    /// Every period has its column, with the Mac's gap between columns,
    /// and the columns span the graph exactly.
    #[test]
    fn every_period_draws_a_column_across_the_graph() {
        for count in [7, 11, 14, 24, 42] {
            let frames = columns(400.0, count);
            assert_eq!(frames.len(), count);
            let (between, _) = gaps(is_thin(count));
            for pair in frames.windows(2) {
                assert!((pair[1].0 - (pair[0].0 + pair[0].1) - between).abs() < 1e-9);
            }
            let (left, width) = frames[count - 1];
            assert!((left + width - 400.0).abs() < 1e-9, "count {count}");
        }
        assert_eq!(gaps(false), (6.0, 5.0));
        assert_eq!(gaps(true), (2.0, 1.0));
        assert!(!is_thin(14) && is_thin(15));
        assert!(columns(0.0, 7).is_empty());
    }

    /// Runs the main loop until `done`, for at most five seconds.
    fn pump_until(done: impl Fn() -> bool) {
        let context = gtk::glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done() && std::time::Instant::now() < deadline {
            if !context.iteration(false) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    /// Lets frames pass once `widget` is laid out, so a style change has
    /// reached it.
    fn pump_until_drawn(widget: &impl IsA<gtk::Widget>) {
        pump_until(|| widget.as_ref().width() > 0);
        // Past any style transition (libadwaita's run about 200ms).
        let end = std::time::Instant::now() + std::time::Duration::from_millis(500);
        pump_until(|| std::time::Instant::now() >= end);
    }

    fn empty_week() -> Vec<Bucket> {
        ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
            .iter()
            .map(|label| Bucket {
                start: chrono::NaiveDate::from_ymd_opt(2026, 11, 9)
                    .and_then(|day| day.and_hms_opt(0, 0, 0))
                    .expect("a date"),
                shared: 0,
                kept: 0,
                label: (*label).to_string(),
            })
            .collect()
    }

    /// Halfway through a step back (from the trailing edge) both sets of
    /// bars have moved 78% of the way toward it: the first column's dot
    /// at x 14 has left its place, the new first column is 55 points in,
    /// and the old set, 191 points along, is all but faded. A zoom moves
    /// neither set.
    #[test]
    fn a_period_step_slides_the_bars_and_a_zoom_does_not() {
        use super::glass_paint::paint_tests::{near, paint};
        let ground = (0xf2, 0xf3, 0xf6);
        let flat = (0xf2_i32, 0xf3_i32, 0xf6_i32);
        let swap = |edge, t| Swap {
            old: empty_week(),
            was: edge,
            edge,
            t,
        };
        let sliding = paint(246, 120, ground, |context| {
            paint_swap(
                context,
                246.0,
                &swap(Some(Edge::Trailing), 0.225),
                &empty_week(),
                None,
                false,
            );
        });
        assert!(
            near(sliding(14, 44), flat, 0),
            "the old dot left: {:?}",
            sliding(14, 44)
        );
        let moved = 14 + (246.0 * (1.0 - 0.775_561_333)) as i32;
        assert!(!near(sliding(moved, 44), flat, 0), "no new dot at {moved}");
        let zooming = paint(246, 120, ground, |context| {
            paint_swap(
                context,
                246.0,
                &swap(None, 0.075),
                &empty_week(),
                None,
                false,
            );
        });
        assert!(!near(zooming(14, 44), flat, 0), "a zoom swaps in place");
    }

    /// `GlassBarGraph` drawn over the pane's ground: seven 30-wide columns
    /// 6 apart (246 across). Unlit dots are `ink(0.09)`, black in light,
    /// not the text colour; the hovered column is washed in `ink(0.07)`
    /// with its dots at 0.2; and each column's canvas draws its own stretch
    /// of the `ink(0.14)` axis, so the gaps between columns have none
    /// (`Indicators.swift:405-420,505-528`).
    #[test]
    fn the_bars_are_drawn_in_the_macs_overlay_ink_column_by_column() {
        use super::glass_paint::paint_tests::{near, over, paint};
        let ground = (0xf2, 0xf3, 0xf6);
        let black = (0, 0, 0);
        let buckets = empty_week();
        let pixel = paint(246, 120, ground, |context| {
            paint_bars(context, 246.0, &buckets, Some(3), false);
        });
        // Column 0's middle dot nearest the axis: (15, 44.6), radius 1.9,
        // at the pitch (48 - 1 - 6) / 7.
        assert!(
            near(pixel(14, 44), over(black, 0.09, ground), 1),
            "unlit {:?}",
            pixel(14, 44)
        );
        // The same dot in the hovered column (3, from 108): 0.2 on the wash.
        let washed = over(black, 0.07, ground);
        let washed = (washed.0 as u8, washed.1 as u8, washed.2 as u8);
        assert!(
            near(pixel(122, 44), over(black, 0.2, washed), 2),
            "hovered {:?}",
            pixel(122, 44)
        );
        // The wash itself, away from the dots, and none on other columns.
        assert!(
            near(pixel(123, 2), over(black, 0.07, ground), 1),
            "wash {:?}",
            pixel(123, 2)
        );
        assert!(near(pixel(15, 2), (0xf2, 0xf3, 0xf6), 1));
        // The axis: a 1pt rule at 47.5..48.5, half on row 47, inside a
        // column; nothing in the 6pt gap between columns 0 and 1.
        assert!(
            near(pixel(2, 47), over(black, 0.07, ground), 1),
            "axis {:?}",
            pixel(2, 47)
        );
        assert!(
            near(pixel(32, 47), (0xf2, 0xf3, 0xf6), 1),
            "gap {:?}",
            pixel(32, 47)
        );
        // Dark: white overlay ink.
        let ground = (0x17, 0x1b, 0x2d);
        let pixel = paint(246, 120, ground, |context| {
            paint_bars(context, 246.0, &buckets, None, true);
        });
        assert!(
            near(pixel(14, 44), over((255, 255, 255), 0.09, ground), 1),
            "dark unlit {:?}",
            pixel(14, 44)
        );
        assert!(near(pixel(32, 47), (0x17, 0x1b, 0x2d), 1));
    }

    /// Bar labels: caption (11) under a full graph, micro set regular (10)
    /// under a thin one (`Indicators.swift:427`, `GlassTokens.swift:722,724`),
    /// under a 96-high matrix.
    #[test]
    fn the_bar_labels_are_caption_or_micro() {
        assert_eq!(label_size(false), 11.0);
        assert_eq!(label_size(true), 10.0);
        assert_eq!(MATRIX_HEIGHT, 96.0);
    }

    /// The pills draw the symbol the Mac draws for each of the core's text
    /// glyphs (`GlassPillIconButton.symbol(for:)`): chevron.left,
    /// chevron.right, minus, plus and chevron.right.to.line, as stock
    /// icons of the same shape; an unknown glyph stays text.
    #[test]
    fn the_pills_draw_the_macs_symbols_as_stock_icons() {
        let graph = monitor_screens_copy().traces_graph;
        assert_eq!(pill_symbol("\u{2039}"), Some("go-previous-symbolic"));
        assert_eq!(pill_symbol("\u{203a}"), Some("go-next-symbolic"));
        assert_eq!(
            pill_symbol(graph.zoom_out_glyph),
            Some("list-remove-symbolic")
        );
        assert_eq!(pill_symbol(graph.zoom_in_glyph), Some("list-add-symbolic"));
        assert_eq!(
            pill_symbol(graph.jump_to_now_glyph),
            Some("go-last-symbolic")
        );
        assert_eq!(pill_symbol("\u{00bb}"), None);
    }

    /// The focus button's glyph (`TracesGraphFooter.focusInk`): off with
    /// nothing to focus, on while focused, idle otherwise.
    #[test]
    fn the_focus_glyph_takes_the_macs_three_inks() {
        assert_eq!(FocusInk::of(true, false), FocusInk::Off);
        assert_eq!(FocusInk::of(false, false), FocusInk::Off);
        assert_eq!(FocusInk::of(true, true), FocusInk::On);
        assert_eq!(FocusInk::of(false, true), FocusInk::Idle);
    }

    /// Measured, with the app's stylesheet: the pills are 30 by 26
    /// (`GlassPillIconButton`), the legend cells 28 high
    /// (`size.controlLarge`), the range capsule 26 high (`size.control`),
    /// and the focus pill takes the ink for its state (`focusInk`,
    /// `MonitorShell.swift:242,305-308`).
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_footer_controls_are_the_macs_sizes() {
        adw::init().expect("GTK display");
        crate::ui::style::install();
        let footer = GraphFooter::new();
        let window = gtk::Window::builder().child(&footer.root).build();
        footer.root.set_reveal_child(true);
        window.present();
        let size = |widget: &gtk::Widget| {
            let (width, ..) = widget.measure(gtk::Orientation::Horizontal, -1);
            let (height, ..) = widget.measure(gtk::Orientation::Vertical, -1);
            (width, height)
        };
        // The stylesheet lands on a frame.
        pump_until(|| size(footer.zoom_in.upcast_ref()) == (30, 26));
        for pill in [
            &footer.zoom_out,
            &footer.zoom_in,
            &footer.next,
            &footer.jump,
        ] {
            assert_eq!(size(pill.upcast_ref()), (30, 26));
        }
        assert_eq!(size(footer.focus.upcast_ref()), (30, 26));
        assert_eq!(size(footer.range.upcast_ref()).1, 26);
        let cell = footer.kept_value.parent().expect("the kept cell");
        assert_eq!(size(&cell).1, 28);
        footer.render(GraphData {
            tool: Some("Codex".into()),
            ..GraphData::default()
        });
        assert!(footer.focus.has_css_class(FocusInk::Idle.css()));
        // The pill's fill, 4 in from its left edge on its middle line,
        // rendered from the widget: only the glyph may change with focus.
        let fill = |footer: &GraphFooter| {
            let pill = &footer.focus;
            let (width, height) = (pill.width(), pill.height());
            let paintable = gtk::WidgetPaintable::new(Some(pill));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
            let node = snapshot.to_node().expect("a render node");
            let renderer = gtk::gsk::Renderer::for_surface(&window.surface().expect("a surface"))
                .expect("a renderer");
            let texture = renderer.render_texture(&node, None);
            renderer.unrealize();
            let stride = texture.width() as usize * 4;
            let mut pixels = vec![0u8; stride * texture.height() as usize];
            texture.download(&mut pixels, stride);
            let at = (texture.height() as usize / 2) * stride + 4 * 4;
            pixels[at..at + 4].to_vec()
        };
        pump_until_drawn(&footer.focus);
        let idle = fill(&footer);
        footer.set_focused(true);
        assert!(footer.focus.has_css_class(FocusInk::On.css()));
        assert!(!footer.focus.has_css_class(FocusInk::Idle.css()));
        assert!(footer.focus.is_active());
        pump_until_drawn(&footer.focus);
        assert_eq!(fill(&footer), idle, "the focused pill changed its fill");
        window.close();
    }

    /// Writes the footer's cairo pieces, the legend dots, the tab dot and
    /// the tab ember into `TC_LOOK_RENDER_DIR` as raw ARGB32 at 2x, for
    /// comparison with the Mac's renders. No display needed.
    #[test]
    #[ignore = "writes comparison renders; set TC_LOOK_RENDER_DIR"]
    fn write_look_renders() {
        use super::glass_paint;
        let Ok(dir) = std::env::var("TC_LOOK_RENDER_DIR") else {
            return;
        };
        let save = |name: &str,
                    width: i32,
                    height: i32,
                    ground: (f64, f64, f64),
                    paint: &dyn Fn(&gtk::cairo::Context)| {
            let surface =
                gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width * 2, height * 2)
                    .expect("a surface");
            {
                let context = gtk::cairo::Context::new(&surface).expect("a context");
                context.set_source_rgb(ground.0, ground.1, ground.2);
                let _ = context.paint();
                context.scale(2.0, 2.0);
                paint(&context);
            }
            surface.flush();
            let stride = surface.stride();
            let mut surface = surface;
            let data = surface.data().expect("pixels").to_vec();
            std::fs::write(
                format!("{dir}/{name}-{}x{}x{stride}.argb", width * 2, height * 2),
                data,
            )
            .expect("written");
        };
        let days = [
            "Sat", "Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun", "Mon", "Tue",
        ];
        let counts = [
            (0, 0),
            (2, 1),
            (5, 0),
            (0, 3),
            (9, 2),
            (1, 1),
            (0, 0),
            (12, 4),
            (3, 0),
            (0, 6),
            (7, 1),
        ];
        let buckets: Vec<Bucket> = days
            .iter()
            .zip(counts)
            .map(|(label, (shared, kept))| Bucket {
                start: chrono::NaiveDate::from_ymd_opt(2026, 11, 1)
                    .and_then(|day| day.and_hms_opt(0, 0, 0))
                    .expect("a date"),
                shared,
                kept,
                label: (*label).to_string(),
            })
            .collect();
        for dark in [false, true] {
            let scheme = if dark { "dark" } else { "light" };
            let pane = if dark {
                (0x17, 0x1b, 0x2d)
            } else {
                (0xf2, 0xf3, 0xf6)
            };
            let pane = (
                f64::from(pane.0) / 255.0,
                f64::from(pane.1) / 255.0,
                f64::from(pane.2) / 255.0,
            );
            save(&format!("bars-{scheme}"), 408, 115, pane, &|context| {
                paint_bars(context, 408.0, &buckets, Some(4), dark);
            });
            let well = if dark {
                (0x0e, 0x11, 0x1d)
            } else {
                (0xdb, 0xdd, 0xe3)
            };
            let well = (
                f64::from(well.0) / 255.0,
                f64::from(well.1) / 255.0,
                f64::from(well.2) / 255.0,
            );
            save(&format!("legend-dot-{scheme}"), 14, 14, well, &|context| {
                glass_paint::paint_status_dot(
                    context,
                    7.0,
                    7.0,
                    8.0,
                    Series::Shared.ink(dark),
                    true,
                    dark,
                );
            });
            save(&format!("tab-dots-{scheme}"), 60, 12, pane, &|context| {
                for (index, tone) in [
                    crate::ui::style::Tone::Clear,
                    crate::ui::style::Tone::Attention,
                    crate::ui::style::Tone::Refused,
                    crate::ui::style::Tone::Held,
                    crate::ui::style::Tone::Neutral,
                ]
                .into_iter()
                .enumerate()
                {
                    let ink = glass_paint::tab_dot_ink(tone, dark);
                    glass_paint::paint_status_dot(
                        context,
                        6.0 + 12.0 * index as f64,
                        6.0,
                        6.0,
                        ink,
                        false,
                        dark,
                    );
                }
            });
            save(&format!("tab-ember-{scheme}"), 440, 38, pane, &|context| {
                glass_paint::paint_ember(context, 38.0, 220.0, 110.0, dark);
            });
        }
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

    /// Previous slides the bars from the trailing edge, next and jump
    /// from the leading one, a zoom swaps them in place; each runs on the
    /// bars' clock and ends, and with animations off none runs.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_footer_slides_a_step_and_fades_a_zoom() {
        gtk::init().expect("GTK display");
        let settings = gtk::Settings::default().expect("settings");
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(true);
        let footer = GraphFooter::new();
        let window = gtk::Window::builder()
            .default_width(600)
            .default_height(400)
            .child(&footer.root)
            .build();
        window.present();
        footer.root.set_reveal_child(true);
        pump_until(|| footer.bars.width() > 0 && footer.root.fraction() >= 1.0);
        let back = find_pill(&footer, monitor_screens_copy().previous);
        back.emit_clicked();
        let swap = footer.swap().expect("a step slides");
        assert_eq!(swap.edge, Some(Edge::Trailing));
        assert_eq!(swap.was, None, "the first bars only fade out");
        pump_until(|| footer.swap().is_some_and(|swap| swap.t > 0.0));
        assert!(footer.swap().is_some_and(|swap| swap.t < 0.45), "part-way");
        pump_until(|| footer.swap().is_none());
        assert!(footer.swap().is_none(), "and done");
        footer.next.emit_clicked();
        assert_eq!(
            footer.swap().map(|swap| swap.was),
            Some(Some(Edge::Trailing)),
            "the bars leave with the edge they came in from"
        );
        assert_eq!(
            footer.swap().map(|swap| swap.edge),
            Some(Some(Edge::Leading))
        );
        footer.zoom_out.emit_clicked();
        assert_eq!(
            footer.swap().map(|swap| swap.edge),
            Some(None),
            "a zoom in place"
        );
        settings.set_gtk_enable_animations(false);
        assert!(footer.swap().is_none(), "followed live");
        footer.zoom_in.emit_clicked();
        assert!(footer.swap().is_none(), "none with animations off");
        settings.set_gtk_enable_animations(previous);
        window.close();
    }

    /// The pill named `name`.
    fn find_pill(footer: &GraphFooter, name: &str) -> gtk::Button {
        let mut stack = vec![footer.root.clone().upcast::<gtk::Widget>()];
        while let Some(widget) = stack.pop() {
            if let Some(button) = widget.downcast_ref::<gtk::Button>()
                && button.tooltip_text().as_deref() == Some(name)
            {
                return button.clone();
            }
            let mut child = widget.first_child();
            while let Some(next) = child {
                child = next.next_sibling();
                stack.push(next);
            }
        }
        panic!("no pill named {name}");
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_footer_opens_at_once_under_reduced_motion() {
        gtk::init().expect("GTK display");
        let settings = gtk::Settings::default().expect("settings");
        let previous = settings.is_gtk_enable_animations();
        let footer = GraphFooter::new();
        // Under the tab's screen, as in the window: the footer takes its
        // own height from the column, not the window's.
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let screen = gtk::Box::new(gtk::Orientation::Vertical, 0);
        screen.set_vexpand(true);
        column.append(&screen);
        column.append(&footer.root);
        let window = gtk::Window::builder()
            .default_width(600)
            .default_height(400)
            .child(&column)
            .build();
        window.present();
        settings.set_gtk_enable_animations(false);
        footer.root.set_reveal_child(true);
        assert_eq!(footer.root.fraction(), 1.0, "open at once");
        footer.root.set_reveal_child(false);
        assert_eq!(footer.root.fraction(), 0.0, "closed at once");
        settings.set_gtk_enable_animations(true);
        footer.root.set_reveal_child(true);
        // Part-way open on the token curve, then open; then turned off
        // part-way through closing, it is closed at once.
        let seen = Rc::new(Cell::new(false));
        {
            let seen = seen.clone();
            let root = footer.root.clone();
            pump_until(move || {
                let f = root.fraction();
                if f > 0.05 && f < 0.95 {
                    seen.set(true);
                }
                f >= 1.0
            });
        }
        assert!(seen.get(), "never part-way open");
        assert_eq!(footer.root.fraction(), 1.0);
        pump_until(|| footer.root.height() == HEIGHT);
        assert_eq!(footer.root.height(), HEIGHT, "its 206 points");
        footer.root.set_reveal_child(false);
        settings.set_gtk_enable_animations(false);
        assert_eq!(footer.root.fraction(), 0.0, "followed live");
        settings.set_gtk_enable_animations(previous);
        window.close();
    }
}
