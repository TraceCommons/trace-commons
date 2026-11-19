//! The Mac's popover stack as it moves (`GlassPopoverStack` in
//! `macos/Sources/TCDesign/Components/PopoverNotice.swift`): the cards
//! behind the front one, and how a card comes to the front.
//!
//! - Behind the front card, up to two plates peek out `PEEK` points a
//!   level, each 4% a level narrower than the front card, centred, at 0.9
//!   and 0.7 opacity (`plate`). `Plates` draws them in the
//!   `tc-notice-plate` style under the front card.
//! - A card new to the stack slides in from the side the stack hangs from,
//!   fading in; a card revealed by closing the one in front of it only
//!   fades in where it is. Both on a spring (response 0.36, damping 0.86)
//!   (`arrival`). `Slide` holds a card and moves it. With
//!   `gtk-enable-animations` off a card is simply there.
//!
//! - The card the new front replaces fades where it is, under the new one,
//!   on the same spring (`arrival`'s `.asymmetric(removal: .opacity)`):
//!   `Slide::arrive_over` draws the old card's last frame, taken as it is
//!   hidden, beneath the arriving card (`covered`).
//! - The plates follow the count behind on the same spring: the stack's
//!   foot moves with the padding, and a plate added or taken away fades
//!   (`Plates::set_behind`).
//!
//! Deliberate platform difference: the plates show below the front card
//! only, not in the corners its rounding leaves.

use std::cell::Cell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::glass_motion::{self, Driver};

/// How far each card behind shows past the one in front, and how many
/// cards are drawn (`GlassPopoverStack.peek`, `.drawn`).
pub const PEEK: f64 = 8.0;
pub const DRAWN: usize = 3;
/// The stack's spring.
pub const SPRING: (f64, f64) = (0.36, 0.86);

/// A plate `level` back behind a front card `width` wide: its left edge,
/// width and opacity.
pub fn plate(level: usize, width: f64) -> (f64, f64, f64) {
    let level = level as f64;
    let narrowed = width * (1.0 - 0.04 * level);
    (
        (width - narrowed) / 2.0,
        narrowed,
        if level <= 1.0 { 0.9 } else { 0.7 },
    )
}

/// A card coming to the front, `t` seconds in: how far it is from its
/// place (a card `height` tall; negative is above), and its opacity. A new
/// card slides from the side the stack hangs from, the top unless
/// `from_foot`; a revealed one only fades.
pub fn arrival(fresh: bool, t: f64, height: f64, from_foot: bool) -> (f64, f64) {
    let (response, damping) = SPRING;
    let p = glass_motion::spring(response, damping, t);
    let side = if from_foot { 1.0 } else { -1.0 };
    let offset = if fresh {
        side * height * (1.0 - p)
    } else {
        0.0
    };
    (offset, p.clamp(0.0, 1.0))
}

/// How much of the card a new front replaces still shows, `t` seconds in:
/// SwiftUI's opacity removal on the stack's spring, `1 - p`, never below
/// nothing while the spring overshoots.
pub fn covered(t: f64) -> f64 {
    let (response, damping) = SPRING;
    (1.0 - glass_motion::spring(response, damping, t)).clamp(0.0, 1.0)
}

/// The plates `t` seconds into a change from `from` cards behind to `to`:
/// how many plates' height the stack's foot takes (it moves on the spring,
/// as the Mac's padding does), and each level's share of its opacity, a
/// level only one side has fading in or out.
pub fn plates_between(from: usize, to: usize, t: f64) -> (f64, Vec<f64>) {
    let (response, damping) = SPRING;
    let p = glass_motion::spring(response, damping, t);
    let height = from as f64 + (to as f64 - from as f64) * p;
    let shown = p.clamp(0.0, 1.0);
    let levels = (1..=from.max(to))
        .map(|level| match (level <= from, level <= to) {
            (true, true) => 1.0,
            (false, true) => shown,
            (true, false) => 1.0 - shown,
            (false, false) => 0.0,
        })
        .collect();
    (height.max(0.0), levels)
}

mod plates {
    use super::*;

    #[derive(Default)]
    pub struct Plates {
        pub behind: Cell<usize>,
        /// While the count changes: the count it left and how far in.
        pub from: Cell<usize>,
        pub at: Cell<Option<f64>>,
        pub driver: Driver,
        /// Hung from the foot (the map's cards): the plates peek out above
        /// the front card rather than below it.
        pub bottom: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Plates {
        const NAME: &'static str = "TcNoticePlates";
        type Type = super::Plates;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Plates {}

    impl Plates {
        /// The plates' height in levels and each level's share, now.
        pub(super) fn shown(&self) -> (f64, Vec<f64>) {
            let to = self.behind.get();
            match self.at.get() {
                Some(t) => plates_between(self.from.get(), to, t),
                None => (to as f64, vec![1.0; to]),
            }
        }
    }

    impl WidgetImpl for Plates {
        fn measure(&self, orientation: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Vertical => {
                    let (shown, _) = self.shown();
                    let h = (PEEK * shown).round() as i32;
                    (h, h, -1, -1)
                }
                _ => (0, 0, -1, -1),
            }
        }

        #[allow(deprecated)]
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let width = f64::from(widget.width());
            let height = f64::from(widget.height());
            let context = widget.style_context();
            let (_, levels) = self.shown();
            // Back first, so the nearer plate is drawn over the farther.
            for level in (1..=levels.len()).rev() {
                let share = levels[level - 1];
                if share <= 0.0 {
                    continue;
                }
                let (x, w, opacity) = plate(level, width);
                let h = PEEK * level as f64;
                let y = if self.bottom.get() { height - h } else { 0.0 };
                snapshot.push_opacity(opacity * share);
                snapshot.render_background(&context, x, y, w, h);
                snapshot.render_frame(&context, x, y, w, h);
                snapshot.pop();
            }
        }
    }
}

glib::wrapper! {
    /// The plates under the front card, in the `tc-notice-plate` style.
    pub struct Plates(ObjectSubclass<plates::Plates>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Plates {
    fn default() -> Self {
        Self::new()
    }
}

impl Plates {
    pub fn new() -> Self {
        let plates: Self = glib::Object::builder()
            .property("accessible-role", gtk::AccessibleRole::Presentation)
            .build();
        plates.add_css_class("tc-notice-plate");
        plates.set_can_target(false);
        plates.set_visible(false);
        plates
    }

    /// Plates for a stack hung from the foot, peeking out above the front
    /// card, in `class`'s style.
    pub fn hung_from_foot(class: &str) -> Self {
        let plates = Self::new();
        plates.remove_css_class("tc-notice-plate");
        plates.add_css_class(class);
        plates.imp().bottom.set(true);
        plates
    }

    /// How many plates peek out.
    pub fn behind(&self) -> usize {
        self.imp().behind.get()
    }

    /// The count behind the front card; a change moves the stack's foot
    /// and fades the plate added or taken away on the stack's spring, with
    /// GTK's animations on and the stack shown.
    pub fn set_behind(&self, behind: usize) {
        let imp = self.imp();
        let before = imp.behind.replace(behind);
        if before == behind {
            self.set_visible(behind > 0 || imp.at.get().is_some());
            return;
        }
        imp.driver.finish();
        // The plates are hidden while none peek out, so whether the stack
        // is on screen is their parent's to say.
        let shown = self.parent().is_some_and(|stack| stack.is_mapped());
        if !shown || !glass_motion::animations_enabled() {
            imp.at.set(None);
            self.set_visible(behind > 0);
            self.queue_resize();
            return;
        }
        imp.from.set(before);
        imp.at.set(Some(0.0));
        self.set_visible(true);
        let (response, damping) = SPRING;
        let length = glass_motion::spring_settles(response, damping);
        let weak = self.downgrade();
        let ended = self.downgrade();
        imp.driver.start(
            self,
            length,
            0.0,
            move |t| {
                let Some(plates) = weak.upgrade() else { return };
                plates.imp().at.set((t < length).then_some(t));
                plates.queue_resize();
            },
            move || {
                if let Some(plates) = ended.upgrade() {
                    plates.imp().at.set(None);
                    plates.set_visible(plates.imp().behind.get() > 0);
                    plates.queue_resize();
                }
            },
        );
    }

    /// Whether the plates are moving between counts.
    pub fn moving(&self) -> bool {
        self.imp().at.get().is_some()
    }
}

mod slide {
    use super::*;

    #[derive(Default)]
    pub struct Slide {
        pub offset: Cell<f64>,
        pub driver: Driver,
        /// The card this one replaced, as last drawn, and how much of it
        /// still shows beneath this one; hung from the foot it sits on the
        /// slide's foot.
        pub covered: std::cell::RefCell<Option<(gtk::gsk::RenderNode, f64)>>,
        /// The card as this slide last drew it, and the slide's height then.
        pub last: std::cell::RefCell<Option<(gtk::gsk::RenderNode, f64)>>,
        pub covered_opacity: Cell<f64>,
        pub from_foot: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Slide {
        const NAME: &'static str = "TcNoticeSlide";
        type Type = super::Slide;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Slide {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Slide {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            self.obj()
                .first_child()
                .map_or((0, 0, -1, -1), |child| child.measure(orientation, for_size))
        }

        fn request_mode(&self) -> gtk::SizeRequestMode {
            self.obj()
                .first_child()
                .map_or(gtk::SizeRequestMode::ConstantSize, |child| {
                    child.request_mode()
                })
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let height = f64::from(widget.height());
            let opacity = self.covered_opacity.get();
            if let Some((node, h)) = self.covered.borrow().as_ref()
                && opacity > 0.0
            {
                let y = if self.from_foot.get() {
                    height - h
                } else {
                    0.0
                };
                snapshot.save();
                snapshot.translate(&gtk::graphene::Point::new(0.0, y as f32));
                snapshot.push_opacity(opacity);
                snapshot.append_node(node);
                snapshot.pop();
                snapshot.restore();
            }
            if let Some(child) = widget.first_child() {
                // Kept, so the card can fade where it is when the next
                // front replaces it; the node is the one drawn, not a copy.
                let own = gtk::Snapshot::new();
                widget.snapshot_child(&child, &own);
                if let Some(node) = own.to_node() {
                    snapshot.append_node(&node);
                    *self.last.borrow_mut() = Some((node, height));
                }
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let offset = self.offset.get() as f32;
            if let Some(child) = self.obj().first_child() {
                let transform = (offset != 0.0).then(|| {
                    gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(0.0, offset))
                });
                child.allocate(width, height, baseline, transform);
            }
        }
    }
}

glib::wrapper! {
    /// Holds a card and moves it as it comes to the front.
    pub struct Slide(ObjectSubclass<slide::Slide>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Slide {
    pub fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let slide: Self = glib::Object::new();
        child.set_parent(&slide);
        slide
    }

    /// The card's offset now, in points above its place.
    pub fn offset(&self) -> f64 {
        self.imp().offset.get()
    }

    /// Brings the card in: sliding from the top (or the foot) when
    /// `fresh`, else fading.
    pub fn arrive(&self, fresh: bool, from_foot: bool) {
        self.arrive_over(fresh, from_foot, None);
    }

    /// [`Self::arrive`], over `beneath`, the card this one replaces as its
    /// slide last drew it ([`Self::last_drawn`]), which fades where it is
    /// beneath this one on the same spring.
    pub fn arrive_over(
        &self,
        fresh: bool,
        from_foot: bool,
        beneath: Option<(gtk::gsk::RenderNode, f64)>,
    ) {
        let imp = self.imp();
        imp.from_foot.set(from_foot);
        imp.covered_opacity
            .set(if beneath.is_some() { 1.0 } else { 0.0 });
        *imp.covered.borrow_mut() = beneath;
        // Not laid out yet when it has just been shown: its height for the
        // width it will have.
        let height = match self.height() {
            0 => {
                let width = self.parent().map_or(0, |parent| parent.width());
                let for_width = if width > 0 { width } else { -1 };
                f64::from(self.measure(gtk::Orientation::Vertical, for_width).1)
            }
            laid_out => f64::from(laid_out),
        };
        let (response, damping) = SPRING;
        let length = glass_motion::spring_settles(response, damping);
        // The first frame is drawn before the clock's first tick.
        let (offset, opacity) = arrival(fresh, 0.0, height, from_foot);
        self.imp().offset.set(offset);
        self.set_opacity(1.0);
        if let Some(child) = self.first_child() {
            child.set_opacity(opacity);
        }
        self.queue_allocate();
        let weak = self.downgrade();
        self.imp().driver.start(
            self,
            length,
            0.0,
            move |t| {
                let Some(slide) = weak.upgrade() else { return };
                let (offset, opacity) = if t >= length {
                    (0.0, 1.0)
                } else {
                    arrival(fresh, t, height, from_foot)
                };
                slide.imp().offset.set(offset);
                slide.set_opacity(1.0);
                // The arriving card's own fade is its child's, so the card
                // it covers keeps its own opacity beneath it.
                if let Some(child) = slide.first_child() {
                    child.set_opacity(opacity);
                }
                slide
                    .imp()
                    .covered_opacity
                    .set(if t >= length { 0.0 } else { covered(t) });
                if t >= length {
                    slide.imp().covered.borrow_mut().take();
                }
                slide.queue_allocate();
                slide.queue_draw();
            },
            || {},
        );
    }

    /// Ends any movement with the card in place.
    pub fn settle(&self) {
        self.imp().driver.finish();
        self.imp().offset.set(0.0);
        self.set_opacity(1.0);
        if let Some(child) = self.first_child() {
            child.set_opacity(1.0);
        }
        self.imp().covered.borrow_mut().take();
        self.imp().covered_opacity.set(0.0);
        self.queue_allocate();
        self.queue_draw();
    }

    /// The card as this slide last drew it, with the slide's height then.
    pub fn last_drawn(&self) -> Option<(gtk::gsk::RenderNode, f64)> {
        self.imp().last.borrow().clone()
    }

    /// How much of the card this one replaced still shows beneath it.
    pub fn covered_opacity(&self) -> f64 {
        if self.imp().covered.borrow().is_some() {
            self.imp().covered_opacity.get()
        } else {
            0.0
        }
    }

    /// The arriving card's own opacity, while it fades in.
    pub fn card_opacity(&self) -> f64 {
        self.first_child().map_or(1.0, |child| child.opacity())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `GlassPopoverStack`: 8 points a level, three cards drawn, each plate
    /// 4% a level narrower and centred, at 0.9 then 0.7.
    #[test]
    fn the_plates_are_the_macs() {
        assert_eq!((PEEK, DRAWN), (8.0, 3));
        assert_eq!(plate(1, 400.0), (8.0, 384.0, 0.9));
        assert_eq!(plate(2, 400.0), (16.0, 368.0, 0.7));
        assert_eq!(super::super::monitor::notices::PEEK as f64, PEEK);
        assert_eq!(super::super::monitor::notices::BEHIND, DRAWN - 1);
    }

    /// A new card slides in from the top on the stack's spring, fading in;
    /// a revealed one only fades.
    #[test]
    fn a_card_arrives_as_the_macs_does() {
        assert_eq!(SPRING, (0.36, 0.86));
        let (offset, opacity) = arrival(true, 0.0, 120.0, false);
        assert_eq!((offset, opacity), (-120.0, 0.0));
        // SwiftUI's spring at 0.1 s is 0.567733 of the way.
        let (offset, opacity) = arrival(true, 0.1, 120.0, false);
        assert!(
            (offset + 120.0 * (1.0 - 0.567_733)).abs() < 1e-4,
            "{offset}"
        );
        assert!((opacity - 0.567_733).abs() < 1e-5);
        let (offset, opacity) = arrival(false, 0.1, 120.0, false);
        assert_eq!(offset, 0.0);
        assert!((opacity - 0.567_733).abs() < 1e-5);
        // Past the overshoot the opacity holds at 1.
        assert_eq!(arrival(true, 0.3, 120.0, false).1, 1.0);
        // Hung from the foot (the map's cards), a new card rises into place.
        assert_eq!(arrival(true, 0.0, 120.0, true).0, 120.0);
    }

    fn swift(path: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/../../macos/Sources/{path}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    /// The card a new front replaces fades out where it is on the stack's
    /// spring (`removal: .opacity` under `.spring(0.36, 0.86)`), at the
    /// values SwiftUI's spring gives (the glass_motion probe).
    #[test]
    fn the_covered_card_fades_on_the_stacks_spring() {
        let source = swift("TCDesign/Components/PopoverNotice.swift");
        assert!(source.contains(
            "return .asymmetric(insertion: fresh ? slide : .opacity, removal: .opacity)"
        ));
        assert!(source.contains(
            ".animation(reduceMotion ? nil : .spring(response: 0.36, dampingFraction: 0.86), value: front.id)"
        ));
        assert_eq!(covered(0.0), 1.0);
        for (t, p) in [
            (0.05, 0.231_173_19),
            (0.1, 0.567_732_99),
            (0.2, 0.928_487_26),
        ] {
            assert!((covered(t) - (1.0 - p)).abs() < 1e-6, "{t}: {}", covered(t));
        }
        // Past the overshoot nothing of it shows.
        assert_eq!(covered(0.3), 0.0);
    }

    /// The stack's foot follows the count behind on the spring, as the
    /// Mac's `.padding(_, peek * behind)` does under the stack's animation,
    /// and a plate added fades in, one taken away fades out.
    #[test]
    fn the_plates_follow_the_count_on_the_spring() {
        let source = swift("TCDesign/Components/PopoverNotice.swift");
        assert!(
            source.contains(
                ".padding(anchor == .bottom ? .top : .bottom, Self.peek * CGFloat(behind))"
            )
        );
        let (height, levels) = plates_between(2, 1, 0.0);
        assert_eq!((height, levels), (2.0, vec![1.0, 1.0]));
        let (height, levels) = plates_between(2, 1, 0.1);
        assert!((height - (2.0 - 0.567_732_99)).abs() < 1e-6, "{height}");
        assert!((levels[1] - (1.0 - 0.567_732_99)).abs() < 1e-6);
        assert_eq!(levels[0], 1.0);
        let (height, levels) = plates_between(0, 2, 0.1);
        assert!((height - 2.0 * 0.567_732_99).abs() < 1e-6);
        assert!((levels[0] - 0.567_732_99).abs() < 1e-6 && levels[0] == levels[1]);
        let (height, levels) = plates_between(1, 0, 1.0);
        assert!(height.abs() < 1e-3, "{height}");
        assert!(levels[0] < 1e-3);
    }
}
