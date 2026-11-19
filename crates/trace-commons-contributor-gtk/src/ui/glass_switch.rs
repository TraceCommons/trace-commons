//! A switch whose knob slides as the Mac's does (`GlassToggleStyle` in
//! `macos/Sources/TCDesign/Components/Controls.swift`): SwiftUI's
//! `.easeOut` over `motion.standard` (0.22 s), or `motion.fast` (0.15 s)
//! for a tree row's watch switch.
//!
//! GTK's own switch moves its knob in code, on a fixed curve no stylesheet
//! reaches, so this draws the same CSS nodes (`switch` holding a `slider`,
//! and the on and off marks the toolkit shows for high contrast) and moves
//! the slider itself. It keeps `gtk::Switch`'s contract: `active` is where
//! the knob is, `state` is what the track shows (`:checked`), and a
//! `state-set` handler that returns `Stop` keeps the old state, as a switch
//! waiting on a confirmation or the daemon does. `set_active` emits
//! `state-set` as GTK's does, but moves the knob at once: on the Mac only a
//! press animates. The track's colour is `motion_css`'s transition.
//!
//! The slide runs on `glass_motion::Tween`, so with `gtk-enable-animations`
//! off the knob is at its end at once, and that is followed live.

use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::glass_motion::{self, Tween};
use super::style::brand_tokens::motion;

/// How long the knob takes: `GlassToggleStyle.duration`.
pub fn slide_duration(watch: bool) -> f64 {
    if watch {
        motion::FAST
    } else {
        motion::STANDARD
    }
}

/// The knob's place, 0 at the start and 1 at the end, `t` seconds into a
/// slide from `from` to `to`: what `GlassSwitch`'s tween draws, as a
/// function for the tests.
#[cfg(test)]
pub fn knob_at(from: f64, to: f64, t: f64, watch: bool) -> f64 {
    let duration = slide_duration(watch);
    glass_motion::tween_value(from, to, glass_motion::ease_out(t / duration))
}

/// A switch's checked state, as assistive tech reads it.
fn tristate(on: bool) -> gtk::AccessibleTristate {
    if on {
        gtk::AccessibleTristate::True
    } else {
        gtk::AccessibleTristate::False
    }
}

/// The class the tree row's watch switch carries (`style.css`).
pub const WATCH_CLASS: &str = "tc-tree-switch";

mod slider {
    use super::*;

    #[derive(Default)]
    pub struct Slider;

    #[glib::object_subclass]
    impl ObjectSubclass for Slider {
        const NAME: &'static str = "TcGlassSwitchSlider";
        type Type = super::Slider;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("slider");
        }
    }

    impl ObjectImpl for Slider {}
    impl WidgetImpl for Slider {}
}

glib::wrapper! {
    /// The knob: an empty `slider` node the theme draws.
    pub struct Slider(ObjectSubclass<slider::Slider>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

mod imp {
    use std::sync::OnceLock;

    use glib::subclass::Signal;

    use super::*;

    pub struct GlassSwitch {
        pub active: Cell<bool>,
        pub state: Cell<bool>,
        pub watch: Cell<bool>,
        /// The knob's place, 0 (off) to 1 (on).
        pub knob: Tween,
        pub slider: RefCell<Option<Slider>>,
        pub marks: RefCell<Option<(gtk::Image, gtk::Image)>>,
    }

    impl Default for GlassSwitch {
        fn default() -> Self {
            Self {
                active: Cell::new(false),
                state: Cell::new(false),
                watch: Cell::new(false),
                knob: Tween::new(0.0),
                slider: RefCell::new(None),
                marks: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GlassSwitch {
        const NAME: &'static str = "TcGlassSwitch";
        type Type = super::GlassSwitch;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("switch");
            klass.set_accessible_role(gtk::AccessibleRole::Switch);
            klass.set_activate_signal_from_name("activate");
            for key in [
                gtk::gdk::Key::space,
                gtk::gdk::Key::KP_Space,
                gtk::gdk::Key::Return,
                gtk::gdk::Key::ISO_Enter,
                gtk::gdk::Key::KP_Enter,
            ] {
                klass.add_binding_signal(key, gtk::gdk::ModifierType::empty(), "activate");
            }
        }
    }

    impl ObjectImpl for GlassSwitch {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
            PROPERTIES.get_or_init(|| {
                vec![
                    glib::ParamSpecBoolean::builder("active").build(),
                    glib::ParamSpecBoolean::builder("state").build(),
                ]
            })
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            let on = value.get::<bool>().unwrap_or(false);
            match pspec.name() {
                "active" => self.obj().set_active(on),
                "state" => self.obj().set_state(on),
                _ => {}
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "active" => self.active.get().to_value(),
                _ => self.state.get().to_value(),
            }
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // GTK's `state-set`: handlers run first; one returning
                    // true stops the emission before the default handler
                    // makes the state the knob's.
                    Signal::builder("state-set")
                        .param_types([bool::static_type()])
                        .return_type::<bool>()
                        .run_last()
                        .accumulator(|_, acc, value| {
                            let handled = value.get::<bool>().unwrap_or(false);
                            *acc = value.clone();
                            !handled
                        })
                        .class_handler(|_, values| {
                            let switch = values[0].get::<super::GlassSwitch>().ok()?;
                            let on = values[1].get::<bool>().ok()?;
                            switch.set_state(on);
                            Some(true.to_value())
                        })
                        .build(),
                    Signal::builder("activate")
                        .action()
                        .run_first()
                        .class_handler(|_, values| {
                            if let Ok(switch) = values[0].get::<super::GlassSwitch>() {
                                switch.toggle();
                            }
                            None
                        })
                        .build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_focusable(true);
            obj.set_overflow(gtk::Overflow::Hidden);
            let on = gtk::Image::from_icon_name("switch-on-symbolic");
            let off = gtk::Image::from_icon_name("switch-off-symbolic");
            on.set_accessible_role(gtk::AccessibleRole::Presentation);
            off.set_accessible_role(gtk::AccessibleRole::Presentation);
            on.set_parent(&*obj);
            off.set_parent(&*obj);
            let slider: Slider = glib::Object::new();
            slider.set_parent(&*obj);
            *self.slider.borrow_mut() = Some(slider);
            *self.marks.borrow_mut() = Some((on, off));
            let click = gtk::GestureClick::new();
            let weak = obj.downgrade();
            click.connect_released(move |gesture, _, x, y| {
                let Some(switch) = weak.upgrade() else { return };
                gesture.set_state(gtk::EventSequenceState::Claimed);
                if switch.contains(x, y) {
                    switch.toggle();
                }
            });
            obj.add_controller(click);
            obj.update_state(&[gtk::accessible::State::Checked(tristate(false))]);
        }

        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for GlassSwitch {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let measure = |widget: &gtk::Widget| widget.measure(orientation, -1);
            let slider = self.slider.borrow();
            let marks = self.marks.borrow();
            let (mut min, mut nat) = slider.as_ref().map_or((0, 0), |s| {
                let (min, nat, _, _) = measure(s.upcast_ref());
                (min, nat)
            });
            if let Some((on, off)) = marks.as_ref() {
                for mark in [on, off] {
                    let (_, mark_nat, _, _) = measure(mark.upcast_ref());
                    min = min.max(mark_nat);
                    nat = nat.max(mark_nat);
                }
            }
            // As GTK's: the track is two knobs wide.
            if orientation == gtk::Orientation::Horizontal {
                (min * 2, nat * 2, -1, -1)
            } else {
                (min, nat, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let half = width / 2;
            if let Some((on, off)) = self.marks.borrow().as_ref() {
                for (mark, x) in [(on, 0), (off, half)] {
                    let (_, w, _, _) = mark.measure(gtk::Orientation::Horizontal, -1);
                    let (_, h, _, _) = mark.measure(gtk::Orientation::Vertical, -1);
                    let rect = gtk::Allocation::new(x + (half - w) / 2, (height - h) / 2, w, h);
                    mark.size_allocate(&rect, -1);
                }
            }
            if let Some(slider) = self.slider.borrow().as_ref() {
                let x = (self.knob.value() * f64::from(width - half)).round() as i32;
                slider.size_allocate(&gtk::Allocation::new(x, 0, half, height), -1);
            }
        }
    }
}

glib::wrapper! {
    /// The Mac's switch: `GlassToggleStyle` motion on GTK's switch nodes.
    pub struct GlassSwitch(ObjectSubclass<imp::GlassSwitch>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for GlassSwitch {
    fn default() -> Self {
        Self::new()
    }
}

impl GlassSwitch {
    /// A Settings switch, off.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// A Settings switch already `on`, knob and track, with no signal.
    pub fn with_active(on: bool) -> Self {
        let switch = Self::new();
        let imp = switch.imp();
        imp.active.set(on);
        imp.knob.set(if on { 1.0 } else { 0.0 }, |_| {});
        switch.set_state(on);
        switch.update_state(&[gtk::accessible::State::Checked(tristate(on))]);
        switch
    }

    /// A tree row's watch switch: `motion.fast`, and its own size
    /// (`WATCH_CLASS`).
    pub fn watch(on: bool) -> Self {
        let switch = Self::with_active(on);
        switch.imp().watch.set(true);
        switch.add_css_class(WATCH_CLASS);
        switch
    }

    pub fn is_active(&self) -> bool {
        self.imp().active.get()
    }

    pub fn state(&self) -> bool {
        self.imp().state.get()
    }

    /// Where the knob is drawn, 0 (off) to 1 (on).
    pub fn knob(&self) -> f64 {
        self.imp().knob.value()
    }

    /// Moves the knob to `on` at once and emits `state-set`, as GTK's
    /// `set_active` does.
    pub fn set_active(&self, on: bool) {
        self.change(on, false);
    }

    /// What the track shows.
    pub fn set_state(&self, on: bool) {
        let imp = self.imp();
        if on {
            self.set_state_flags(gtk::StateFlags::CHECKED, false);
        } else {
            self.unset_state_flags(gtk::StateFlags::CHECKED);
        }
        if imp.state.replace(on) != on {
            self.notify("state");
        }
    }

    /// A press: the knob slides over the Mac's duration.
    fn toggle(&self) {
        if self.is_sensitive() {
            self.change(!self.is_active(), true);
        }
    }

    fn change(&self, on: bool, slide: bool) {
        let imp = self.imp();
        if imp.active.get() == on {
            return;
        }
        imp.active.set(on);
        let target = if on { 1.0 } else { 0.0 };
        let weak = self.downgrade();
        let frame = move |_| {
            if let Some(switch) = weak.upgrade() {
                switch.queue_allocate();
            }
        };
        if slide {
            let watch = imp.watch.get();
            imp.knob.to(
                self,
                target,
                slide_duration(watch),
                glass_motion::ease_out,
                frame,
            );
        } else {
            imp.knob.set(target, frame);
        }
        self.update_state(&[gtk::accessible::State::Checked(tristate(on))]);
        let _handled: bool = self.emit_by_name("state-set", &[&on]);
        self.notify("active");
    }

    /// GTK's `state-set`: `Stop` keeps the track where it is.
    pub fn connect_state_set<F: Fn(&Self, bool) -> glib::Propagation + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "state-set",
            false,
            glib::closure_local!(move |switch: &GlassSwitch, on: bool| -> bool {
                matches!(f(switch, on), glib::Propagation::Stop)
            }),
        )
    }

    pub fn connect_active_notify<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_notify_local(Some("active"), move |switch, _| f(switch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTROLS: &str =
        include_str!("../../../../macos/Sources/TCDesign/Components/Controls.swift");

    /// `GlassToggleStyle`: `.easeOut` over standard for a toggle and fast
    /// for the watch switch, and only a press is animated.
    #[test]
    fn the_knob_slides_as_the_macs_does() {
        assert!(
            CONTROLS
                .contains("kind == .watch ? GlassTokens.Motion.fast : GlassTokens.Motion.standard")
        );
        assert!(CONTROLS.contains(
            "withAnimation(GlassMotion.systemReducesMotion ? nil : .easeOut(duration: Self.duration(kind)))"
        ));
        assert_eq!(slide_duration(false), 0.22);
        assert_eq!(slide_duration(true), 0.15);
        // Half-way through, SwiftUI's easeOut is at 0.684642792 (a Swift
        // probe of `UnitCurve.easeOut`).
        assert!((knob_at(0.0, 1.0, 0.11, false) - 0.684_642_792).abs() < 1e-6);
        assert!((knob_at(0.0, 1.0, 0.075, true) - 0.684_642_792).abs() < 1e-6);
        assert!((knob_at(1.0, 0.0, 0.11, false) - (1.0 - 0.684_642_792)).abs() < 1e-6);
        assert_eq!(knob_at(0.0, 1.0, 0.22, false), 1.0);
    }

    /// No stock switch is left: its knob would move on GTK's own curve.
    #[test]
    fn no_stock_switch_is_left() {
        for (name, source) in [
            ("settings.rs", include_str!("settings.rs")),
            ("private_inference.rs", include_str!("private_inference.rs")),
            (
                "monitor/traces_list.rs",
                include_str!("monitor/traces_list.rs"),
            ),
        ] {
            assert!(
                !source.contains("gtk::Switch"),
                "{name} still builds gtk::Switch"
            );
        }
    }

    fn pump_until(done: &dyn Fn() -> bool) {
        let context = glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done() && std::time::Instant::now() < deadline {
            context.iteration(false);
        }
    }

    /// A press slides the knob through the middle to the end and sets the
    /// state; `set_active` moves it at once; a `Stop` keeps the state; and
    /// with motion off a press lands at once.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_a_pressed_switch_slides_and_keeps_gtks_contract() {
        gtk::init().unwrap();
        let settings = gtk::Settings::default().unwrap();
        settings.set_gtk_enable_animations(true);
        let switch = GlassSwitch::new();
        let window = gtk::Window::new();
        window.set_child(Some(&switch));
        window.present();
        pump_until(&|| switch.is_mapped());
        assert!(switch.width() >= 2 * switch.first_child().unwrap().width() - 1);
        switch.activate();
        assert!(switch.is_active() && switch.state());
        assert!(switch.has_css_class("switch") || switch.css_name() == "switch");
        let between = Cell::new(false);
        pump_until(&|| {
            let k = switch.knob();
            between.set(between.get() || (k > 0.0 && k < 1.0));
            k == 1.0
        });
        assert!(between.get(), "the knob jumped");
        assert_eq!(switch.knob(), 1.0);
        let slider = switch.last_child().unwrap();
        assert_eq!(
            slider.allocation().x(),
            switch.width() - slider.width(),
            "the knob sits at the end"
        );

        // Programmatic: at once, and state-set still runs.
        let asked = std::rc::Rc::new(Cell::new(0));
        let seen = std::rc::Rc::clone(&asked);
        let id = switch.connect_state_set(move |_, _| {
            seen.set(seen.get() + 1);
            glib::Propagation::Stop
        });
        switch.set_active(false);
        assert_eq!(switch.knob(), 0.0);
        assert_eq!(asked.get(), 1);
        assert!(switch.state(), "a Stop keeps the track on");
        switch.disconnect(id);

        settings.set_gtk_enable_animations(false);
        switch.activate();
        assert_eq!(switch.knob(), 1.0, "motion off: no slide");
        settings.set_gtk_enable_animations(true);
        window.destroy();
    }
}
