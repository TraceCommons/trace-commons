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
//! Deliberate platform differences: the card a new one covers is hidden at
//! once rather than fading under it, and the plates show below the front
//! card only, not in the corners its rounding leaves.

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

mod plates {
    use super::*;

    #[derive(Default)]
    pub struct Plates {
        pub behind: Cell<usize>,
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

    impl WidgetImpl for Plates {
        fn measure(&self, orientation: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Vertical => {
                    let h = (PEEK * self.behind.get() as f64) as i32;
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
            // Back first, so the nearer plate is drawn over the farther.
            for level in (1..=self.behind.get()).rev() {
                let (x, w, opacity) = plate(level, width);
                let h = PEEK * level as f64;
                let y = if self.bottom.get() { height - h } else { 0.0 };
                snapshot.push_opacity(opacity);
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

    pub fn set_behind(&self, behind: usize) {
        if self.imp().behind.replace(behind) != behind {
            self.queue_resize();
        }
        self.set_visible(behind > 0);
    }
}

mod slide {
    use super::*;

    #[derive(Default)]
    pub struct Slide {
        pub offset: Cell<f64>,
        pub driver: Driver,
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
        self.set_opacity(opacity);
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
                slide.set_opacity(opacity);
                slide.queue_allocate();
            },
            || {},
        );
    }

    /// Ends any movement with the card in place.
    pub fn settle(&self) {
        self.imp().driver.finish();
        self.imp().offset.set(0.0);
        self.set_opacity(1.0);
        self.queue_allocate();
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
}
