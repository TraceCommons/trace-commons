//! The window's three panes as one container.
//!
//! The GTK counterpart of `GlassThreePane` in
//! `macos/Sources/TCDesign/Components/ThreePane.swift`: the main pane (the
//! tabs), the map and the inspector, a gap apart, the map and the inspector
//! hiding independently. Every width is `pane_layout`'s answer for the
//! width this widget was actually allocated; nothing here decides one.
//!
//! Because allocation is a pure function of the allocated width, a
//! compositor that does not honour a request to grow or shrink the window
//! (spec spike S2) needs no detection: the panes reallocate inside the
//! width the window kept, the main pane holding its 440.
//!
//! A hidden pane is not allocated and reserves nothing. The container's
//! minimum width is the sum of the shown panes, the map at its minimum,
//! which is what gives the window its minimum.

use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::pane_layout::{self, PaneLayout, Visibility};

type Callback = Box<dyn Fn(f64)>;

mod imp {
    use super::*;

    pub struct ThreePane {
        pub main: RefCell<Option<gtk::Widget>>,
        pub map: RefCell<Option<gtk::Widget>>,
        pub inspector: RefCell<Option<gtk::Widget>>,
        /// The grip drawn in the gap beside the map while the divider is
        /// pointed at or dragged. Decoration only: the drag is the
        /// container's own gesture, measured in its own coordinates, so the
        /// divider moving under the pointer does not feed back into it.
        pub grip: RefCell<Option<gtk::Widget>>,
        pub shows_map: Cell<bool>,
        pub shows_inspector: Cell<bool>,
        /// The main pane's width beside the map, as the divider was last
        /// left.
        pub kept_main: Cell<f64>,
        /// The map's width as the person last left it, which the natural
        /// width opens it at.
        pub kept_map: Cell<f64>,
        /// The main pane's width when a drag began; `None` between drags.
        pub drag_start: Cell<Option<f64>>,
        /// The width the panes were last laid out at.
        pub width: Cell<f64>,
        pub on_map_width: RefCell<Option<Callback>>,
        pub on_main_width: RefCell<Option<Callback>>,
    }

    impl Default for ThreePane {
        fn default() -> Self {
            Self {
                main: RefCell::default(),
                map: RefCell::default(),
                inspector: RefCell::default(),
                grip: RefCell::default(),
                shows_map: Cell::new(true),
                shows_inspector: Cell::new(true),
                kept_main: Cell::new(pane_layout::MAIN_WIDTH),
                kept_map: Cell::new(pane_layout::MAP_WIDTH),
                drag_start: Cell::new(None),
                width: Cell::new(0.0),
                on_map_width: RefCell::default(),
                on_main_width: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ThreePane {
        const NAME: &'static str = "TcThreePane";
        type Type = super::ThreePane;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ThreePane {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_overflow(gtk::Overflow::Hidden);

            let grip = gtk::Box::new(gtk::Orientation::Vertical, 0);
            grip.add_css_class("tc-pane-grip");
            grip.set_can_target(false);
            grip.set_opacity(0.0);
            grip.set_accessible_role(gtk::AccessibleRole::Presentation);
            grip.set_parent(&*obj);
            self.grip.replace(Some(grip.upcast()));

            // The divider: the gap itself, reaching a little into the map.
            // Capture phase, so the press is the divider's before a pane's
            // own widgets see it; a press anywhere else is denied at once.
            let drag = gtk::GestureDrag::new();
            drag.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak = obj.downgrade();
            drag.connect_drag_begin(move |gesture, x, _| {
                let Some(obj) = weak.upgrade() else { return };
                let imp = obj.imp();
                match imp.divider_layout(x) {
                    Some(layout) => {
                        imp.drag_start.set(Some(layout.main));
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                        imp.show_grip(true);
                    }
                    None => {
                        gesture.set_state(gtk::EventSequenceState::Denied);
                    }
                }
            });
            let weak = obj.downgrade();
            drag.connect_drag_update(move |_, dx, _| {
                let Some(obj) = weak.upgrade() else { return };
                let imp = obj.imp();
                let Some(start) = imp.drag_start.get() else {
                    return;
                };
                imp.kept_main.set(pane_layout::dragged_main(
                    start,
                    dx,
                    imp.width.get(),
                    imp.shows_inspector.get(),
                ));
                obj.queue_allocate();
            });
            let weak = obj.downgrade();
            drag.connect_drag_end(move |_, dx, _| {
                let Some(obj) = weak.upgrade() else { return };
                let imp = obj.imp();
                let Some(start) = imp.drag_start.take() else {
                    return;
                };
                imp.show_grip(false);
                // Only a drag that moved ends one: a bare click never writes
                // the width it happens to see.
                if dx == 0.0 {
                    return;
                }
                let kept = pane_layout::dragged_main(
                    start,
                    dx,
                    imp.width.get(),
                    imp.shows_inspector.get(),
                );
                imp.kept_main.set(kept);
                obj.queue_allocate();
                if let Some(callback) = imp.on_main_width.borrow().as_ref() {
                    callback(kept);
                }
                imp.remember_map(true);
            });
            obj.add_controller(drag);

            // The left-right resize cursor, and the grip, over the divider.
            let motion = gtk::EventControllerMotion::new();
            let weak = obj.downgrade();
            motion.connect_motion(move |_, x, _| {
                let Some(obj) = weak.upgrade() else { return };
                let imp = obj.imp();
                let over = imp.divider_layout(x).is_some();
                if imp.drag_start.get().is_none() {
                    imp.show_grip(over);
                }
                obj.set_cursor_from_name(over.then_some("col-resize"));
            });
            let weak = obj.downgrade();
            motion.connect_leave(move |_| {
                let Some(obj) = weak.upgrade() else { return };
                if obj.imp().drag_start.get().is_none() {
                    obj.imp().show_grip(false);
                }
                obj.set_cursor_from_name(None);
            });
            obj.add_controller(motion);
        }

        fn dispose(&self) {
            for slot in [&self.main, &self.map, &self.inspector, &self.grip] {
                if let Some(child) = slot.take() {
                    child.unparent();
                }
            }
        }
    }

    impl WidgetImpl for ThreePane {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let shows = self.visibility();
            // Every shown child is measured, so each may be allocated.
            let mut tallest = 0;
            for (slot, shown) in [
                (&self.main, true),
                (&self.map, shows.map),
                (&self.inspector, shows.inspector),
                (&self.grip, shows.map),
            ] {
                if let Some(child) = slot.borrow().as_ref().filter(|_| shown) {
                    let (minimum, _, _, _) = child.measure(gtk::Orientation::Vertical, -1);
                    tallest = tallest.max(minimum);
                    let _ = child.measure(gtk::Orientation::Horizontal, -1);
                }
            }
            if orientation == gtk::Orientation::Horizontal {
                let minimum = pane_layout::minimum_window_width(shows).ceil() as i32;
                let natural =
                    pane_layout::window_width(shows, self.kept_map.get(), self.kept_main.get())
                        .ceil() as i32;
                (minimum, natural.max(minimum), -1, -1)
            } else {
                let minimum = tallest.max(pane_layout::WINDOW_MIN_HEIGHT as i32);
                (
                    minimum,
                    minimum.max(pane_layout::WINDOW_HEIGHT as i32),
                    -1,
                    -1,
                )
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            self.width.set(f64::from(width));
            let layout = self.layout();
            let place = |slot: &RefCell<Option<gtk::Widget>>, x: f64, pane: Option<f64>| {
                let slot = slot.borrow();
                let Some(child) = slot.as_ref() else { return };
                child.set_child_visible(pane.is_some());
                let Some(pane) = pane else { return };
                let transform = gtk::gsk::Transform::new()
                    .translate(&gtk::graphene::Point::new(x.round() as f32, 0.0));
                child.allocate(pane.round().max(0.0) as i32, height, -1, Some(transform));
            };
            place(&self.main, pane_layout::WINDOW_PADDING, Some(layout.main));
            place(&self.map, layout.map_x(), layout.map);
            place(&self.inspector, layout.inspector_x(), layout.inspector);
            place(
                &self.grip,
                pane_layout::WINDOW_PADDING + layout.main,
                layout.map.map(|_| pane_layout::PANE_GAP),
            );
            // The map's width is kept as the person resizes the window with
            // it shown, never mid-drag.
            if self.drag_start.get().is_none() {
                self.remember_map(false);
            }
        }
    }

    impl ThreePane {
        pub fn visibility(&self) -> Visibility {
            Visibility::new(self.shows_map.get(), self.shows_inspector.get())
        }

        pub fn layout(&self) -> PaneLayout {
            PaneLayout::new(self.width.get(), self.visibility(), self.kept_main.get())
        }

        /// The layout, when `x` is over the divider: the gap before the map
        /// and `DIVIDER_REACH` into it. `None` anywhere else, and while the
        /// map is hidden.
        fn divider_layout(&self, x: f64) -> Option<PaneLayout> {
            if !self.shows_map.get() {
                return None;
            }
            let layout = self.layout();
            let from = pane_layout::WINDOW_PADDING + layout.main;
            let to = layout.map_x() + pane_layout::DIVIDER_REACH;
            (x >= from && x < to).then_some(layout)
        }

        fn show_grip(&self, shown: bool) {
            if let Some(grip) = self.grip.borrow().as_ref() {
                grip.set_opacity(if shown { 0.6 } else { 0.0 });
            }
        }

        /// Keeps the map's width for the next launch: only at or over its
        /// minimum, and only a change of a whole pixel unless `force`.
        fn remember_map(&self, force: bool) {
            let Some(map) = self.layout().map else { return };
            if map < pane_layout::MAP_MIN_WIDTH {
                return;
            }
            if !force && (map - self.kept_map.get()).abs() < 1.0 {
                return;
            }
            self.kept_map.set(map);
            if let Some(callback) = self.on_map_width.borrow().as_ref() {
                callback(map);
            }
        }
    }
}

glib::wrapper! {
    pub struct ThreePane(ObjectSubclass<imp::ThreePane>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ThreePane {
    pub fn new(
        main: &impl IsA<gtk::Widget>,
        map: &impl IsA<gtk::Widget>,
        inspector: &impl IsA<gtk::Widget>,
    ) -> Self {
        let pane: Self = glib::Object::new();
        let imp = pane.imp();
        // Before the grip, so focus and drawing run main, map, inspector.
        let grip = imp.grip.borrow().clone();
        for (slot, child) in [
            (&imp.main, main.upcast_ref::<gtk::Widget>()),
            (&imp.map, map.upcast_ref()),
            (&imp.inspector, inspector.upcast_ref()),
        ] {
            match grip.as_ref() {
                Some(grip) => child.insert_before(&pane, Some(grip)),
                None => child.set_parent(&pane),
            }
            slot.replace(Some(child.clone()));
        }
        pane
    }

    pub fn visibility(&self) -> Visibility {
        self.imp().visibility()
    }

    /// The width the panes were last laid out at; 0 before the first
    /// allocation.
    pub fn laid_out_width(&self) -> f64 {
        self.imp().width.get()
    }

    pub fn kept_main(&self) -> f64 {
        self.imp().kept_main.get()
    }

    pub fn kept_map(&self) -> f64 {
        self.imp().kept_map.get()
    }

    /// The widths the person last left the panes at, from the kept choices.
    pub fn set_kept_widths(&self, main: f64, map: f64) {
        self.imp().kept_main.set(main.max(pane_layout::MAIN_WIDTH));
        self.imp().kept_map.set(map.max(pane_layout::MAP_MIN_WIDTH));
        self.queue_resize();
    }

    /// Shows or hides the map and the inspector. Returns the width the
    /// window should take so that no other pane moves
    /// (`pane_layout::window_width_after`), or `None` when nothing changed
    /// or the panes have not been laid out yet.
    pub fn set_visibility(&self, new: Visibility) -> Option<f64> {
        let imp = self.imp();
        let old = imp.visibility();
        if old == new {
            return None;
        }
        let current = imp.width.get();
        // The main pane keeps the width it has, whatever opens or closes.
        let before = imp.layout();
        if current > 0.0 {
            imp.kept_main.set(before.main);
        }
        imp.shows_map.set(new.map);
        imp.shows_inspector.set(new.inspector);
        self.queue_resize();
        (current > 0.0).then(|| {
            pane_layout::window_width_after(
                current,
                old,
                new,
                imp.kept_main.get(),
                imp.kept_map.get(),
            )
        })
    }

    /// Called with the map's width when the person leaves it at a new one.
    pub fn connect_map_width(&self, callback: impl Fn(f64) + 'static) {
        self.imp().on_map_width.replace(Some(Box::new(callback)));
    }

    /// Called with the main pane's width when a divider drag ends.
    pub fn connect_main_width(&self, callback: impl Fn(f64) + 'static) {
        self.imp().on_main_width.replace(Some(Box::new(callback)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pump() {
        let context = glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
    }

    /// Runs the main loop until `done`, for at most five seconds: a
    /// compositor answers `present` and a resize request asynchronously, and
    /// a layout waits for the frame clock, so what is pending now is not all
    /// there will be.
    fn pump_until(done: impl Fn() -> bool) {
        let context = glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done() && std::time::Instant::now() < deadline {
            if !context.iteration(false) {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        pump();
    }

    fn mounted(width: i32) -> (gtk::Window, ThreePane, [gtk::Box; 3]) {
        let panes = [
            gtk::Box::new(gtk::Orientation::Vertical, 0),
            gtk::Box::new(gtk::Orientation::Vertical, 0),
            gtk::Box::new(gtk::Orientation::Vertical, 0),
        ];
        let pane = ThreePane::new(&panes[0], &panes[1], &panes[2]);
        let window = gtk::Window::builder()
            .default_width(width)
            .default_height(640)
            .child(&pane)
            .build();
        window.present();
        pump_until(|| pane.laid_out_width() > 0.0);
        (window, pane, panes)
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_panes_take_the_layouts_widths_and_a_hidden_pane_takes_none() {
        gtk::init().expect("GTK display");
        let (window, pane, [main, map, inspector]) = mounted(1193);
        let width = pane.laid_out_width();
        assert!(width > 0.0, "the panes were never laid out");
        let layout = PaneLayout::new(width, Visibility::new(true, true), pane_layout::MAIN_WIDTH);
        assert_eq!(f64::from(main.width()), layout.main.round());
        assert_eq!(Some(f64::from(map.width())), layout.map.map(f64::round));
        assert_eq!(f64::from(inspector.width()), pane_layout::INSPECTOR_WIDTH);
        assert!(map.is_child_visible() && inspector.is_child_visible());
        // 10px gaps: the map starts a gap after the main pane.
        let bounds = map.compute_bounds(&pane).expect("the map is placed");
        assert_eq!(f64::from(bounds.x()), layout.map_x());

        // Whatever the compositor does with the request, the panes are laid
        // out for the width the window really has.
        let wanted = pane.set_visibility(Visibility::new(false, true));
        assert_eq!(
            wanted,
            Some(pane_layout::window_width_after(
                width,
                Visibility::new(true, true),
                Visibility::new(false, true),
                layout.main,
                pane.kept_map(),
            ))
        );
        pump_until(|| !map.is_child_visible());
        assert!(!map.is_child_visible(), "a hidden map is still drawn");
        let after = PaneLayout::new(
            pane.laid_out_width(),
            Visibility::new(false, true),
            pane.kept_main(),
        );
        assert_eq!(f64::from(main.width()), after.main.round());
        assert_eq!(f64::from(inspector.width()), pane_layout::INSPECTOR_WIDTH);
        assert!(main.width() >= pane_layout::MAIN_WIDTH as i32);
        window.close();
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_minimum_width_is_the_sum_of_the_shown_panes() {
        gtk::init().expect("GTK display");
        let (window, pane, _) = mounted(1193);
        for shows in [
            Visibility::new(true, true),
            Visibility::new(true, false),
            Visibility::new(false, true),
            Visibility::new(false, false),
        ] {
            pane.set_visibility(shows);
            let (minimum, natural, _, _) = pane.measure(gtk::Orientation::Horizontal, -1);
            assert_eq!(
                f64::from(minimum),
                pane_layout::minimum_window_width(shows),
                "{shows:?}"
            );
            assert!(natural >= minimum);
        }
        let (minimum, _, _, _) = pane.measure(gtk::Orientation::Vertical, -1);
        assert!(f64::from(minimum) >= pane_layout::WINDOW_MIN_HEIGHT);
        window.close();
    }

    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_kept_widths_open_the_window_and_never_go_under_a_panes_floor() {
        gtk::init().expect("GTK display");
        let (window, pane, _) = mounted(1193);
        pane.set_kept_widths(512.0, 620.0);
        let (_, natural, _, _) = pane.measure(gtk::Orientation::Horizontal, -1);
        assert_eq!(
            f64::from(natural),
            pane_layout::window_width(Visibility::new(true, true), 620.0, 512.0)
        );
        pane.set_kept_widths(10.0, 10.0);
        assert_eq!(pane.kept_main(), pane_layout::MAIN_WIDTH);
        assert_eq!(pane.kept_map(), pane_layout::MAP_MIN_WIDTH);
        window.close();
    }
}
