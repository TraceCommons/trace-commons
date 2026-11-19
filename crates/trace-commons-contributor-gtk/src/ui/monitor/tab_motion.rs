//! The tab strip's motion (`GlassRuleTabs`,
//! `macos/Sources/TCDesign/Components/Navigation.swift`): the ember line
//! and halo glide to a newly chosen tab on `spring(0.35, 0.85)`, the
//! strip's `.animation(_, value: selection)`, which with the ember's
//! `matchedGeometryEffect` moves the glow from the old tab's frame to the
//! new one's. A movable tab (every tab but Home) is lifted under the
//! pointer and while pressed or dragged: 6% larger, 2 points up, on a
//! control's surface with a soft shadow, on `spring(0.25, 0.8)`; dragged,
//! it follows the pointer and trades places past a neighbour's middle,
//! and let go it drops into its slot on `spring(0.3, 0.85)`. `TabLift`
//! draws that around one tab; `shell` moves it.

use std::cell::Cell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::ui::glass_motion;

/// `.spring(response: 0.25, dampingFraction: 0.8)` on the lift.
pub const LIFT_SPRING: (f64, f64) = (0.25, 0.8);
/// `drop()`'s `.spring(response: 0.3, dampingFraction: 0.85)`.
pub const DROP_SPRING: (f64, f64) = (0.3, 0.85);
/// `GlassRuleTabs.liftScale`, `liftRise`, `liftShadowOpacity`, and the
/// shadow's `radius: 8, y: 3`.
pub const LIFT_SCALE: f64 = 1.06;
pub const LIFT_RISE: f64 = 2.0;
pub const LIFT_SHADOW_OPACITY: f64 = 0.18;
pub const LIFT_SHADOW_RADIUS: f64 = 8.0;
pub const LIFT_SHADOW_Y: f64 = 3.0;

/// A value `t` seconds into a spring from `from` to `to`.
pub fn sprung(spring: (f64, f64), from: f64, to: f64, t: f64) -> f64 {
    from + (to - from) * glass_motion::spring(spring.0, spring.1, t)
}

/// The tab's scale and rise for a lift `amount` of the way up (it may
/// overshoot past 1 on the spring, as SwiftUI's does).
pub fn lifted(amount: f64) -> (f64, f64) {
    (1.0 + (LIFT_SCALE - 1.0) * amount, -LIFT_RISE * amount)
}

/// `.spring(response: 0.35, dampingFraction: 0.85)` on the selection.
pub const EMBER_SPRING: (f64, f64) = (0.35, 0.85);

/// How long the glide runs.
pub fn ember_settles() -> f64 {
    glass_motion::spring_settles(EMBER_SPRING.0, EMBER_SPRING.1)
}

/// The ember's tab frame `(left, width)` `t` seconds into a glide from
/// `from` to `to`: both edges move together on the spring, overshooting a
/// little, as the matched frame does.
pub fn glide(from: (f64, f64), to: (f64, f64), t: f64) -> (f64, f64) {
    let p = glass_motion::spring(EMBER_SPRING.0, EMBER_SPRING.1, t);
    (from.0 + (to.0 - from.0) * p, from.1 + (to.1 - from.1) * p)
}

mod lift {
    use super::*;

    #[derive(Default)]
    pub struct TabLift {
        /// How far up the tab is, 0 at rest and 1 lifted.
        pub amount: Cell<f64>,
        /// Where the lift is going, and the spring moving it.
        pub target: Cell<f64>,
        pub lift_driver: glass_motion::Driver,
        /// The dragged tab's offset from its slot, and the drop moving it.
        pub drag: Cell<f64>,
        pub drop_driver: glass_motion::Driver,
        /// Bumped by each new lift or drop, so the one it interrupts does
        /// not jump to its own end as it is stopped.
        pub lift_generation: Cell<u64>,
        pub drop_generation: Cell<u64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TabLift {
        const NAME: &'static str = "TcTabLift";
        type Type = super::TabLift;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for TabLift {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for TabLift {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            // The tab's own size; the surface takes whatever the tab has.
            self.obj()
                .last_child()
                .map_or((0, 0, -1, -1), |tab| tab.measure(orientation, for_size))
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let obj = self.obj();
            let (scale, rise) = lifted(self.amount.get());
            let (w, h) = (width as f32, height as f32);
            let transform = gtk::gsk::Transform::new()
                .translate(&gtk::graphene::Point::new(
                    w / 2.0 + self.drag.get() as f32,
                    h / 2.0 + rise as f32,
                ))
                .scale(scale as f32, scale as f32)
                .translate(&gtk::graphene::Point::new(-w / 2.0, -h / 2.0));
            // The surface sits `space.s1` inside the tab's top and foot.
            let inset = crate::ui::style::brand_tokens::glass::space::S1;
            if let Some(surface) = obj.first_child()
                && obj.last_child().as_ref() != Some(&surface)
            {
                surface.allocate(
                    width,
                    (height - 2 * inset).max(0),
                    -1,
                    Some(
                        transform
                            .clone()
                            .translate(&gtk::graphene::Point::new(0.0, inset as f32)),
                    ),
                );
            }
            if let Some(tab) = obj.last_child() {
                tab.allocate(width, height, baseline, Some(transform));
            }
        }
    }
}

glib::wrapper! {
    /// One tab of the strip, lifted and moved as `GlassRuleTabs` lifts and
    /// moves a movable tab: the control surface behind it, then the tab.
    pub struct TabLift(ObjectSubclass<lift::TabLift>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl TabLift {
    pub fn new(tab: &impl IsA<gtk::Widget>) -> Self {
        let lift: Self = glib::Object::new();
        let surface = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        surface.add_css_class("tc-glass-control");
        surface.add_css_class("tc-tab-lift");
        surface.set_can_target(false);
        surface.set_opacity(0.0);
        surface.set_accessible_role(gtk::AccessibleRole::Presentation);
        surface.set_parent(&lift);
        tab.set_parent(&lift);
        lift
    }

    fn surface(&self) -> Option<gtk::Widget> {
        self.first_child()
            .filter(|surface| self.last_child().as_ref() != Some(surface))
    }

    fn set_amount(&self, amount: f64) {
        self.imp().amount.set(amount);
        if let Some(surface) = self.surface() {
            surface.set_opacity(amount.clamp(0.0, 1.0));
        }
        self.queue_allocate();
    }

    /// How far up the tab is now.
    pub fn amount(&self) -> f64 {
        self.imp().amount.get()
    }

    /// Lifts the tab (`up`) or lets it down, on the lift spring from where
    /// it is; at once under reduced motion.
    pub fn set_lifted(&self, up: bool) {
        let to = if up { 1.0 } else { 0.0 };
        let imp = self.imp();
        if imp.target.replace(to) == to && (imp.lift_driver.is_running() || self.amount() == to) {
            return;
        }
        let from = self.amount();
        let generation = imp.lift_generation.get() + 1;
        imp.lift_generation.set(generation);
        let length = glass_motion::spring_settles(LIFT_SPRING.0, LIFT_SPRING.1);
        let weak = self.downgrade();
        imp.lift_driver.start(
            self,
            length,
            0.0,
            move |t| {
                if let Some(lift) = weak.upgrade()
                    && lift.imp().lift_generation.get() == generation
                {
                    let now = if t >= length || !glass_motion::animations_enabled() {
                        to
                    } else {
                        sprung(LIFT_SPRING, from, to, t)
                    };
                    lift.set_amount(now);
                }
            },
            || {},
        );
    }

    /// Whether the lift is still moving.
    pub fn imp_running(&self) -> bool {
        self.imp().lift_driver.is_running()
    }

    /// Whether the tab is lifted or on its way up.
    pub fn is_lifted(&self) -> bool {
        self.imp().target.get() > 0.0
    }

    /// The dragged tab's offset from its slot, following the pointer.
    pub fn set_drag(&self, offset: f64) {
        // The pointer takes the tab where it is: a drop under way stops.
        self.imp()
            .drop_generation
            .set(self.imp().drop_generation.get() + 1);
        self.imp().drop_driver.finish();
        self.imp().drag.set(offset);
        self.queue_allocate();
    }

    pub fn drag(&self) -> f64 {
        self.imp().drag.get()
    }

    /// Lets the dragged tab down into its slot on the drop spring; at once
    /// under reduced motion.
    pub fn drop_into_slot(&self) {
        let from = self.drag();
        if from == 0.0 {
            return;
        }
        let generation = self.imp().drop_generation.get() + 1;
        self.imp().drop_generation.set(generation);
        let length = glass_motion::spring_settles(DROP_SPRING.0, DROP_SPRING.1);
        let weak = self.downgrade();
        self.imp().drop_driver.start(
            self,
            length,
            0.0,
            move |t| {
                if let Some(lift) = weak.upgrade()
                    && lift.imp().drop_generation.get() == generation
                {
                    let now = if t >= length || !glass_motion::animations_enabled() {
                        0.0
                    } else {
                        sprung(DROP_SPRING, from, 0.0, t)
                    };
                    lift.imp().drag.set(now);
                    lift.queue_allocate();
                }
            },
            || {},
        );
    }

    /// Whether the drop is still moving.
    pub fn dropping(&self) -> bool {
        self.imp().drop_driver.is_running()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swift(path: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/../../macos/Sources/{path}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    /// The spring is the one `GlassRuleTabs` animates the selection on,
    /// read from the Swift.
    #[test]
    fn the_ember_glides_on_the_macs_spring() {
        let source = swift("TCDesign/Components/Navigation.swift");
        assert!(source.contains(
            ".animation(reduceMotion ? nil : .spring(response: 0.35, dampingFraction: 0.85), value: selection)"
        ));
        assert!(source.contains("emberGlow.matchedGeometryEffect(id: \"ember\", in: ember)"));
        assert_eq!(EMBER_SPRING, (0.35, 0.85));
    }

    /// The glide's values, from SwiftUI's own
    /// `Spring(response: 0.35, dampingRatio: 0.85).value(target:time:)` (a
    /// Swift probe, scratchpad motion/probe-surfaces), from a tab at 0 to
    /// one at 100, both 80 wide then 120.
    #[test]
    fn the_glide_follows_swiftuis_spring() {
        let swift = [
            (0.05, 0.242_229_348_430_760_27),
            (0.1, 0.588_284_561_132_537_2),
            (0.2, 0.942_504_042_583_767_3),
            (0.3, 1.004_831_848_148_262_3),
            (0.5, 1.000_776_100_098_912),
        ];
        for (t, p) in swift {
            let (left, width) = glide((0.0, 80.0), (100.0, 120.0), t);
            assert!((left - 100.0 * p).abs() < 1e-6, "{t}: {left}");
            assert!((width - (80.0 + 40.0 * p)).abs() < 1e-6, "{t}: {width}");
        }
        assert!((ember_settles() - 0.635_448_428_568_898_7).abs() < 0.05);
        assert_eq!(glide((0.0, 80.0), (100.0, 120.0), 0.0), (0.0, 80.0));
    }

    /// The lift and the drop are `GlassRuleTabs`' own numbers, read from
    /// the Swift, at the values SwiftUI's springs give (the probe).
    #[test]
    fn the_lift_and_the_drop_are_the_macs() {
        let source = swift("TCDesign/Components/Navigation.swift");
        for line in [
            ".animation(reduceMotion ? nil : .spring(response: 0.25, dampingFraction: 0.8), value: up)",
            "withAnimation(reduceMotion ? nil : .spring(response: 0.3, dampingFraction: 0.85)) {",
            "static var liftScale: CGFloat { 1.06 }",
            "static var liftRise: CGFloat { 2 }",
            "static var liftShadowOpacity: Double { 0.18 }",
            ".shadow(color: .black.opacity(Self.liftShadowOpacity), radius: 8, y: 3)",
            ".padding(.vertical, GlassTokens.Space.s1)",
        ] {
            assert!(source.contains(line), "{line}");
        }
        assert_eq!((LIFT_SPRING, DROP_SPRING), ((0.25, 0.8), (0.3, 0.85)));
        assert_eq!(
            (LIFT_SCALE, LIFT_RISE, LIFT_SHADOW_OPACITY),
            (1.06, 2.0, 0.18)
        );
        assert_eq!((LIFT_SHADOW_RADIUS, LIFT_SHADOW_Y), (8.0, 3.0));
        for (t, p) in [
            (0.05, 0.399_251_267_934_662_15),
            (0.1, 0.813_403_338_123_450_6),
            (0.2, 1.014_792_929_094_717_3),
            (0.3, 1.003_594_574_829_922_2),
        ] {
            assert!(
                (sprung(LIFT_SPRING, 0.0, 1.0, t) - p).abs() < 1e-6,
                "lift {t}"
            );
            let (scale, rise) = lifted(p);
            assert!((scale - (1.0 + 0.06 * p)).abs() < 1e-12);
            assert!((rise + 2.0 * p).abs() < 1e-12);
        }
        for (t, p) in [
            (0.05, 0.303_069_016_791_464_03),
            (0.1, 0.681_165_993_000_648_3),
            (0.2, 0.979_974_981_729_186_7),
            (0.3, 1.006_019_960_711_833_9),
        ] {
            // A tab let go 40 points from its slot.
            assert!(
                (sprung(DROP_SPRING, 40.0, 0.0, t) - 40.0 * (1.0 - p)).abs() < 1e-6,
                "drop {t}"
            );
        }
        assert_eq!(lifted(0.0), (1.0, 0.0));
    }
}
