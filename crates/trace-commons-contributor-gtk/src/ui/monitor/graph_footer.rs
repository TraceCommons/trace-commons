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
//! Deliberate platform differences: the bars swap in place on a period
//! step (the Mac slides them), and where the Mac draws SF Symbols, which
//! are Apple's, the pills draw stock symbolic icons (`pill_symbol`, and
//! `find-location` for the binoculars).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use chrono::{DateTime, Utc};
use gtk::prelude::*;
use trace_commons_contributor::preview_copy::monitor_screens_copy;

use super::glass_paint;
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
enum Series {
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
fn legend_cell(word: &str, series: Series) -> (gtk::Box, gtk::Label) {
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

/// Draws the graph for the footer's state.
fn draw_bars(context: &gtk::cairo::Context, width: i32, state: &State) {
    let buckets = state.buckets.borrow();
    paint_bars(
        context,
        f64::from(width),
        &buckets,
        state.hovered.get(),
        dark_scheme(),
    );
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
        let context = gtk::glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
        let size = |widget: &gtk::Widget| {
            let (width, ..) = widget.measure(gtk::Orientation::Horizontal, -1);
            let (height, ..) = widget.measure(gtk::Orientation::Vertical, -1);
            (width, height)
        };
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
        footer.set_focused(true);
        assert!(footer.focus.has_css_class(FocusInk::On.css()));
        assert!(!footer.focus.has_css_class(FocusInk::Idle.css()));
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
