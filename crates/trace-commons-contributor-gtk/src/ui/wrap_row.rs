//! A row of controls that wraps onto further lines when its column is
//! narrower than the row (`style::wrap_row`).
//!
//! It was a `GtkFlowBox` until the third fix pass on #2160. On GTK 4.14
//! (Ubuntu 24.04's GNOME) a flow box of buttons was given less height than
//! the lines it then laid its buttons out in: the Private AI offer showed
//! "Not now" with "Turn on" clipped off below it, and a Traces folder card's
//! "Submit as…" hung over the card's foot with "Never" not drawn at all.
//! GTK 4.22 drew the same rows whole. This row measures and lays out with
//! one line-breaking rule ([`lines`]), so the height it asks for is the
//! height of the lines it draws on every GTK version.

use std::ops::Range;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// The children's widths, broken into lines no wider than `available`:
/// each child goes on the current line when it fits after `spacing`, and
/// starts a new one when it does not. A line always holds at least one
/// child, so a child wider than `available` sits on a line of its own.
pub fn lines(widths: &[i32], spacing: i32, available: i32) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut used = 0;
    for (index, width) in widths.iter().enumerate() {
        if index > start && used + spacing + width > available {
            out.push(start..index);
            start = index;
            used = *width;
        } else if index == start {
            used = *width;
        } else {
            used += spacing + width;
        }
    }
    if start < widths.len() {
        out.push(start..widths.len());
    }
    out
}

mod imp {
    use std::cell::Cell;

    use super::*;

    #[derive(Default)]
    pub struct WrapRow {
        pub spacing: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WrapRow {
        const NAME: &'static str = "TcWrapRow";
        type Type = super::WrapRow;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for WrapRow {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for WrapRow {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let children = self.obj().shown();
            let spacing = self.spacing.get();
            if orientation == gtk::Orientation::Horizontal {
                // The least is the widest child alone on its line; the most
                // is every child on one line.
                let mut minimum = 0;
                let mut natural = 0;
                for (index, child) in children.iter().enumerate() {
                    let (min, nat, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
                    minimum = minimum.max(min);
                    natural += nat + if index > 0 { spacing } else { 0 };
                }
                return (minimum, natural, -1, -1);
            }
            let layout = self.obj().layout(&children, for_size);
            let (minimum, natural) = layout
                .heights
                .iter()
                .fold((0, 0), |(min, nat), line| (min + line.0, nat + line.1));
            let gaps = spacing * (layout.heights.len() as i32 - 1).max(0);
            (minimum + gaps, natural + gaps, -1, -1)
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let row = self.obj();
            let children = row.shown();
            let spacing = self.spacing.get();
            let layout = row.layout(&children, width);
            let rtl = row.direction() == gtk::TextDirection::Rtl;
            let mut y = 0;
            for (line, (_, height)) in layout.lines.iter().zip(&layout.heights) {
                let mut x = 0;
                for index in line.clone() {
                    let child_width = layout.widths[index];
                    let left = if rtl { width - x - child_width } else { x };
                    let transform = gtk::gsk::Transform::new()
                        .translate(&gtk::graphene::Point::new(left as f32, y as f32));
                    children[index].allocate(child_width, *height, -1, Some(transform));
                    x += child_width + spacing;
                }
                y += height + spacing;
            }
        }
    }
}

glib::wrapper! {
    /// See the module's documentation.
    pub struct WrapRow(ObjectSubclass<imp::WrapRow>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// How the row's children are laid out at one width.
struct Layout {
    /// Each child's width.
    widths: Vec<i32>,
    lines: Vec<Range<usize>>,
    /// Each line's least and natural height.
    heights: Vec<(i32, i32)>,
}

impl WrapRow {
    pub fn new(spacing: i32) -> Self {
        let row: Self = glib::Object::new();
        row.imp().spacing.set(spacing);
        row
    }

    /// Adds `widget` after the row's last child.
    pub fn append(&self, widget: &impl IsA<gtk::Widget>) {
        widget.insert_before(self, None::<&gtk::Widget>);
    }

    /// The children that take room.
    fn shown(&self) -> Vec<gtk::Widget> {
        let mut out = Vec::new();
        let mut child = self.first_child();
        while let Some(widget) = child {
            if widget.should_layout() {
                out.push(widget.clone());
            }
            child = widget.next_sibling();
        }
        out
    }

    /// Each child at its natural width, but never wider than `width` nor
    /// narrower than its least, broken into lines (`lines`). A `width` under
    /// zero is no limit: every child on one line.
    fn layout(&self, children: &[gtk::Widget], width: i32) -> Layout {
        let widths: Vec<i32> = children
            .iter()
            .map(|child| {
                let (min, nat, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
                if width < 0 {
                    nat
                } else {
                    nat.min(width).max(min)
                }
            })
            .collect();
        let available = if width < 0 { i32::MAX } else { width };
        let lines = lines(&widths, self.imp().spacing.get(), available);
        let heights = lines
            .iter()
            .map(|line| {
                line.clone().fold((0, 0), |(min, nat), index| {
                    let (child_min, child_nat, _, _) =
                        children[index].measure(gtk::Orientation::Vertical, widths[index]);
                    (min.max(child_min), nat.max(child_nat))
                })
            })
            .collect();
        Layout {
            widths,
            lines,
            heights,
        }
    }
}

#[cfg(test)]
/// Every shown button in `root`, each mapped and inside `inside`'s
/// bounds, or a message naming each one that is not, with its numbers.
pub(crate) fn buttons_outside(root: &gtk::Widget, inside: &gtk::Widget) -> Vec<String> {
    let mut all = vec![root.clone()];
    let mut index = 0;
    while index < all.len() {
        let mut child = all[index].first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            all.push(next);
        }
        index += 1;
    }
    let limit = (inside.width() as f32, inside.height() as f32);
    let mut out = Vec::new();
    for button in all
        .iter()
        .filter(|widget| widget.is::<gtk::Button>() || widget.is::<gtk::MenuButton>())
        .filter(|widget| widget.is_visible())
    {
        // A menu button's own inner button is checked as the menu button.
        if button.is::<gtk::Button>()
            && button
                .parent()
                .is_some_and(|parent| parent.is::<gtk::MenuButton>())
        {
            continue;
        }
        let what = button
            .downcast_ref::<gtk::Button>()
            .and_then(|b| b.label())
            .or_else(|| {
                button
                    .downcast_ref::<gtk::MenuButton>()
                    .and_then(|b| b.label())
            })
            .map(|label| label.to_string())
            .unwrap_or_else(|| format!("{:?}", button.css_classes()));
        if !button.is_mapped() || !button.is_child_visible() {
            out.push(format!("{what}: not drawn (mapped {})", button.is_mapped()));
            continue;
        }
        match button.compute_bounds(inside) {
            Some(bounds)
                if bounds.x() >= -0.5
                    && bounds.y() >= -0.5
                    && bounds.x() + bounds.width() <= limit.0 + 0.5
                    && bounds.y() + bounds.height() <= limit.1 + 0.5
                    && bounds.height() > 0.0 => {}
            bounds => out.push(format!(
                "{what}: {bounds:?} is not inside {} x {} ({:?})",
                limit.0,
                limit.1,
                inside.css_classes()
            )),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use gtk::glib;

    use super::lines;

    #[test]
    fn children_share_a_line_while_they_fit_with_the_spacing() {
        assert_eq!(lines(&[80, 80], 8, 168), vec![0..2]);
        assert_eq!(lines(&[80, 80], 8, 167), vec![0..1, 1..2]);
    }

    #[test]
    fn a_line_always_holds_one_child_even_one_wider_than_the_row() {
        assert_eq!(lines(&[500, 40], 8, 300), vec![0..1, 1..2]);
        assert_eq!(lines(&[40, 500, 40], 8, 300), vec![0..1, 1..2, 2..3]);
    }

    #[test]
    fn three_buttons_in_a_folder_card_break_where_they_stop_fitting() {
        // "Submit all", "Submit as…", "Never" in 200: the third goes down.
        assert_eq!(lines(&[90, 90, 60], 8, 200), vec![0..2, 2..3]);
        assert_eq!(lines(&[90, 90, 60], 8, 256), vec![0..3]);
        assert!(lines(&[], 8, 100).is_empty());
    }

    /// A Traces folder card's three actions, in a column of such cards, at
    /// the 440 pane and narrower: every button is drawn inside its own
    /// card, and no card runs under the next. On GTK 4.14 the flow box this
    /// replaced hung "Submit as…" over the card's foot, under the next card,
    /// and drew no "Never".
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_wrapped_buttons_stay_inside_their_card() {
        use adw::prelude::*;

        use crate::ui::style;

        adw::init().expect("GTK display");
        let window = adw::ApplicationWindow::builder()
            .default_width(440)
            .default_height(600)
            .build();
        let column = gtk::Box::new(gtk::Orientation::Vertical, style::space::S);
        column.set_margin_start(style::space::XL);
        column.set_margin_end(style::space::XL);
        let mut cards = Vec::new();
        for name in ["payments-reconciliation-service", "web", "api"] {
            let card = style::card(gtk::Orientation::Vertical, style::space::S);
            card.append(&gtk::Label::new(Some(name)));
            let actions = style::wrap_row(style::space::S);
            style::wrap_append(&actions, &gtk::Button::with_label("Submit all"));
            style::wrap_append(
                &actions,
                &gtk::MenuButton::builder()
                    .label("Submit as\u{2026}")
                    .build(),
            );
            style::wrap_append(&actions, &gtk::Button::with_label("Never"));
            card.append(&actions);
            column.append(&card);
            cards.push(card);
        }
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&column)
            .build();
        window.set_content(Some(&scroller));
        window.present();
        let context = glib::MainContext::default();
        let settle = || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
            while std::time::Instant::now() < deadline {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        settle();
        let pane = column.width();
        for narrower in [0, 120, 200, 260] {
            column.set_margin_end(style::space::XL + narrower);
            settle();
            for (index, card) in cards.iter().enumerate() {
                let outside = super::buttons_outside(card.upcast_ref(), card.upcast_ref());
                assert!(
                    outside.is_empty(),
                    "column {} of {pane} wide, card {index}: {outside:?}",
                    column.width()
                );
                if let Some(next) = cards.get(index + 1) {
                    let this = card.compute_bounds(&column).expect("placed");
                    let below = next.compute_bounds(&column).expect("placed");
                    assert!(
                        this.y() + this.height() <= below.y() + 0.5,
                        "card {index} {this:?} runs under the next {below:?}"
                    );
                }
            }
        }
        window.close();
    }
}
