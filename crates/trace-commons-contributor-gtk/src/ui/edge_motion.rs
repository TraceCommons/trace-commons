//! The edge rail's open and close (`EdgeRailView`,
//! `macos/Sources/TraceCommonsApp/Views/EdgeRail/EdgeRailView.swift`): the
//! open rail comes in from the trailing edge as it fades in, and goes back
//! out fading, on the token curve over `Motion.slide` (0.25 s,
//! `setOpen`'s `GlassMotion.curve(GlassTokens.Motion.slide)`). Under
//! reduced motion the Mac's transition is a fade with no animation, so the
//! rail is simply there or gone. `SlideX` draws its child moved and faded
//! without moving it in the layout.

use std::cell::Cell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::glass_motion::{self, Driver};
use super::style::brand_tokens::glass;

/// `GlassTokens.Motion.slide`, in seconds.
pub fn slide_seconds() -> f64 {
    f64::from(glass::motion::SLIDE_MS) / 1000.0
}

/// The open rail `t` seconds into opening (or closing, `opening` false):
/// how far right of its place it is drawn, for a rail `distance` points
/// from the trailing edge, and its opacity.
pub fn slid(opening: bool, t: f64, distance: f64) -> (f64, f64) {
    let fraction = t / slide_seconds();
    // The ends exactly, where the curve's solve leaves a hair.
    let p = if fraction <= 0.0 {
        0.0
    } else if fraction >= 1.0 {
        1.0
    } else {
        glass_motion::curve(fraction)
    };
    let shown = if opening { p } else { 1.0 - p };
    (distance * (1.0 - shown), shown)
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SlideX {
        pub offset: Cell<f64>,
        pub driver: Driver,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SlideX {
        const NAME: &'static str = "TcEdgeSlide";
        type Type = super::SlideX;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for SlideX {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for SlideX {
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
                    gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(offset, 0.0))
                });
                child.allocate(width, height, baseline, transform);
            }
        }
    }
}

glib::wrapper! {
    /// Holds the open rail and moves it in from the trailing edge.
    pub struct SlideX(ObjectSubclass<imp::SlideX>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SlideX {
    pub fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let slide: Self = glib::Object::new();
        child.set_parent(&slide);
        slide
    }

    /// How far right of its place the rail is drawn now.
    pub fn offset(&self) -> f64 {
        self.imp().offset.get()
    }

    fn set(&self, offset: f64, opacity: f64) {
        self.imp().offset.set(offset);
        self.set_opacity(opacity);
        self.queue_allocate();
    }

    /// Slides the rail in (`opening`) or out from `distance` points right
    /// of its place; with animations off it is in place, or gone, at once.
    /// Animations are read at each frame.
    pub fn slide(&self, opening: bool, distance: f64) {
        let length = slide_seconds();
        let (offset, opacity) = slid(opening, 0.0, distance);
        self.set(offset, opacity);
        let weak = self.downgrade();
        self.imp().driver.start(
            self,
            length,
            0.0,
            move |t| {
                let Some(slide) = weak.upgrade() else { return };
                let t = if glass_motion::animations_enabled() {
                    t
                } else {
                    length
                };
                let (offset, opacity) = slid(opening, t, distance);
                slide.set(offset, opacity);
            },
            || {},
        );
    }

    /// Whether a slide is under way.
    pub fn sliding(&self) -> bool {
        self.imp().driver.is_running()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `EdgeRailView`: in from the trailing edge with a fade, on the token
    /// curve over `Motion.slide`, read from the Swift and the tokens; a
    /// fade with no animation under reduced motion.
    #[test]
    fn the_rail_slides_as_the_macs_does() {
        let read = |path: &str| {
            std::fs::read_to_string(format!(
                "{}/../../macos/Sources/{path}",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap()
        };
        let view = read("TraceCommonsApp/Views/EdgeRail/EdgeRailView.swift");
        assert!(view.contains(
            ".transition(reduceMotion ? .opacity : .move(edge: .trailing).combined(with: .opacity))"
        ));
        assert!(view.contains(
            "request(open, reduceMotion ? nil : GlassMotion.curve(GlassTokens.Motion.slide))"
        ));
        assert!(
            read("TCDesign/Generated/GlassTokens.swift")
                .contains("public static let slide: Double = 0.25")
        );
        assert_eq!(slide_seconds(), 0.25);
        assert_eq!(slid(true, 0.0, 300.0), (300.0, 0.0));
        // Half way through, the token curve is 0.946079 of the way.
        let (offset, opacity) = slid(true, 0.125, 300.0);
        assert!((opacity - 0.946_079_254).abs() < 1e-5, "{opacity}");
        assert!((offset - 300.0 * (1.0 - 0.946_079_254)).abs() < 1e-3);
        let (offset, opacity) = slid(false, 0.125, 300.0);
        assert!((opacity - (1.0 - 0.946_079_254)).abs() < 1e-5);
        assert!((offset - 300.0 * 0.946_079_254).abs() < 1e-3);
        let (offset, opacity) = slid(true, 0.25, 300.0);
        assert!(offset.abs() < 1e-9 && (opacity - 1.0).abs() < 1e-9);
        let (offset, opacity) = slid(false, 0.25, 300.0);
        assert!((offset - 300.0).abs() < 1e-9 && opacity.abs() < 1e-9);
    }
}
