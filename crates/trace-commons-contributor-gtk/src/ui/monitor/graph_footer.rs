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
//! step (the Mac slides them), and the pills carry text glyphs and the
//! focus button a themed icon, because the Mac draws SF Symbols there,
//! which are Apple's and are not exported.

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

/// The class every graph control wears: a round glass control
/// (`GlassPillIconButton`, 30 by `size.control`).
const PILL_CLASS: &str = "tc-graph-pill";

fn pill(glyph: &str, name: &str) -> gtk::Button {
    let button = gtk::Button::with_label(glyph);
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

fn rgba(hex: &str) -> gtk::gdk::RGBA {
    gtk::gdk::RGBA::parse(hex).unwrap_or(gtk::gdk::RGBA::BLACK)
}

fn dark_scheme() -> bool {
    adw::StyleManager::default().is_dark()
}

/// `GlassStatusDot` at `size.dotLarge` with its halo: the series' ink, and
/// a ring of it at a quarter.
fn legend_dot(series: Series) -> gtk::DrawingArea {
    let size = 8.0;
    let area = gtk::DrawingArea::builder()
        .content_width(14)
        .content_height(14)
        .valign(gtk::Align::Center)
        .build();
    area.set_accessible_role(gtk::AccessibleRole::Presentation);
    area.set_draw_func(move |_, context, width, height| {
        let ink = rgba(series.ink(dark_scheme()));
        let (x, y) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
        let (r, g, b) = (
            f64::from(ink.red()),
            f64::from(ink.green()),
            f64::from(ink.blue()),
        );
        context.set_source_rgba(r, g, b, 0.25);
        context.set_line_width(3.0);
        context.arc(x, y, size / 2.0 + 1.5, 0.0, std::f64::consts::TAU);
        let _ = context.stroke();
        context.set_source_rgba(r, g, b, 1.0);
        context.arc(x, y, size / 2.0, 0.0, std::f64::consts::TAU);
        let _ = context.fill();
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
    let lead = gtk::Box::new(gtk::Orientation::Horizontal, 7);
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
            .build();
        bars.set_accessible_role(gtk::AccessibleRole::Img);

        let previous = pill("\u{2039}", words.previous);
        let focus = gtk::ToggleButton::new();
        focus.set_icon_name("zoom-fit-best-symbolic");
        focus.add_css_class("tc-glass-control");
        focus.add_css_class(PILL_CLASS);
        let zoom_out = pill(graph.zoom_out_glyph, graph.zoom_out);
        let range = gtk::Label::builder()
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        range.add_css_class("tc-type-label");
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

/// Shared rises from the axis and kept hangs from it, as tapered dot
/// columns over a 1pt axis rule (`GlassBarGraph.drawMatrix`), against a
/// floor of five so a single trace is not drawn as a full column
/// (`scaleFloor`). Every period draws its column, lit or not; the hovered
/// one takes a solid wash.
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
    let dark = dark_scheme();
    let shared_ink = rgba(Series::Shared.ink(dark));
    let kept_ink = rgba(Series::Kept.ink(dark));
    let label_ink = rgba(if dark {
        crate::ui::style::brand_tokens::glass::data::LABEL_DARK
    } else {
        crate::ui::style::brand_tokens::glass::data::LABEL_LIGHT
    });
    let thin = is_thin(buckets.len());
    let (_, inset) = gaps(thin);
    let (across_dots, radius) = if thin { (1, 1.5) } else { (3, 1.9) };
    // The label row under the columns (caption, micro when thin), with
    // `space.s2` above it.
    let labels = 16.0;
    let plot = (f64::from(height) - labels).max(2.0);
    let half = plot / 2.0;
    let pitch = ((half - 1.0 - MATRIX_END) / DOT_ROWS as f64).max(0.5);
    let maximum = buckets
        .iter()
        .map(|b| b.shared.max(b.kept))
        .max()
        .unwrap_or(0)
        .max(5) as f64;
    let dot = |x: f64, y: f64, alpha: f64, lit: &gtk::gdk::RGBA, unlit: f64| {
        if alpha > 0.0 {
            context.set_source_rgba(
                f64::from(lit.red()),
                f64::from(lit.green()),
                f64::from(lit.blue()),
                alpha,
            );
        } else {
            context.set_source_rgba(red, green, blue, unlit);
        }
        context.arc(x, y, radius, 0.0, std::f64::consts::TAU);
        let _ = context.fill();
    };
    for (index, (bucket, (left, column))) in buckets
        .iter()
        .zip(columns(f64::from(width), buckets.len()))
        .enumerate()
    {
        let hovered = state.hovered.get() == Some(index);
        if hovered {
            let corner = if thin {
                3.0
            } else {
                f64::from(crate::ui::style::brand_tokens::glass::radius::CONTROL)
            };
            rounded_rect(context, left, 0.0, column, plot, corner);
            context.set_source_rgba(red, green, blue, 0.07);
            let _ = context.fill();
        }
        let unlit = if hovered { 0.2 } else { 0.09 };
        let across = pitch.min((column - inset * 2.0) / across_dots as f64);
        let first = left + (column - across * across_dots as f64) / 2.0 + across / 2.0;
        let up = dot_alphas(bucket.shared as f64, maximum, DOT_ROWS);
        let down = dot_alphas(bucket.kept as f64, maximum, DOT_ROWS);
        for dot_column in 0..across_dots {
            let x = first + dot_column as f64 * across;
            for row in 0..DOT_ROWS {
                let offset = pitch * (row as f64 + 0.5) + 0.5;
                dot(x, half - offset, up[row], &shared_ink, unlit);
                dot(x, half + offset, down[row], &kept_ink, unlit);
            }
        }
        if !bucket.label.is_empty() {
            if hovered {
                context.set_source_rgba(red, green, blue, 1.0);
            } else {
                context.set_source_rgba(
                    f64::from(label_ink.red()),
                    f64::from(label_ink.green()),
                    f64::from(label_ink.blue()),
                    1.0,
                );
            }
            context.set_font_size(if thin { 9.0 } else { 10.0 });
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
                context.move_to(x, f64::from(height) - 3.0);
                let _ = context.show_text(&bucket.label);
            }
        }
    }
    context.set_source_rgba(red, green, blue, 0.14);
    context.rectangle(0.0, half - 0.5, f64::from(width), 1.0);
    let _ = context.fill();
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
        assert_ne!(dot_alphas(9.0, 40.0, DOT_ROWS), dot_alphas(11.0, 40.0, DOT_ROWS));
        assert!(dot_alphas(f64::INFINITY, 40.0, DOT_ROWS).iter().all(|a| *a <= 1.0));
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

    /// The legend is two equal capsule wells and the controls round glass
    /// pills, each styled in the sheet.
    #[test]
    fn the_legend_and_pills_are_glass() {
        let css = include_str!("../style.css");
        let rule = |selector: &str| {
            css.split(&format!("\n{selector} {{"))
                .nth(1)
                .and_then(|rest| rest.split('}').next())
                .unwrap_or_else(|| panic!("no {selector} rule"))
                .to_string()
        };
        let cell = rule(".tc-graph-legend-cell");
        assert!(cell.contains("border-radius: 999px;"));
        assert!(cell.contains("min-height: 28px;"));
        assert!(rule(".tc-graph-legend-value").contains("font-weight: 600;"));
        let pill = rule(&format!(".{PILL_CLASS}"));
        assert!(pill.contains("min-width: 30px;"));
        assert!(pill.contains("min-height: 26px;"));
        assert!(pill.contains("border-radius: 999px;"));
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
