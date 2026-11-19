//! The Mac's own loading marks (`GlassSpinner` and `GlassSkeleton` in
//! `macos/Sources/TCDesign/Components/Feedback.swift`, and the turn of
//! `GlassRefreshButton` in `Containers.swift`), drawn here instead of the
//! stock spinner.
//!
//! - `Spinner`: a 12-point ring at ink 0.18 with a quarter arc in the
//!   secondary ink, 1.5 points wide, turning once every `motion.spin`
//!   (0.8 s) at up to 60 frames a second.
//! - `Skeleton`: a faint bar (ink 0.08, the control radius, 12 points
//!   tall) whose opacity breathes to `skeletonDim` (0.5) and back over
//!   `motion.pulse` (1.6 s) on the token curve, every frame.
//! - `Turn`: holds a refresh button's icon and turns it once, over
//!   `motion.spin`, when it is pressed, and on and on while a read is out
//!   (`set_refreshing`), at up to 30 frames a second.
//!
//! All three stand still with `gtk-enable-animations` off, following it
//! live, and none ticks while it is not shown or its window is minimised
//! or suspended (`glass_motion::keep_ticking`).

use std::cell::Cell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::glass_motion::{self, keep_ticking};
use super::style::brand_tokens::{map, tile};
use super::tool_tile::{self, Colour};

/// The spinner's arc, in degrees, `seconds` into its turning: a full turn
/// per `motion.spin`, none under reduced motion (`GlassSpinner.angle`).
pub fn spinner_angle(seconds: f64, reduce_motion: bool) -> f64 {
    if reduce_motion {
        return 0.0;
    }
    seconds.rem_euclid(tile::SPIN) / tile::SPIN * 360.0
}

/// The skeleton's opacity `seconds` into its breathing: down to the dim
/// over half a pulse on the token curve, then back (`GlassSkeleton`).
pub fn skeleton_opacity(seconds: f64, reduce_motion: bool) -> f64 {
    if reduce_motion {
        return 1.0;
    }
    let half = tile::PULSE / 2.0;
    let phase = seconds.rem_euclid(tile::PULSE);
    let p = if phase < half {
        glass_motion::curve(phase / half)
    } else {
        1.0 - glass_motion::curve((phase - half) / half)
    };
    1.0 + (tile::SKELETON_DIM - 1.0) * p
}

/// A refresh icon's turn, in degrees, `seconds` after the press: one full
/// turn over `motion.spin`, then still (`GlassRefreshButton.angle`, for a
/// press that is not waiting on a long read).
pub fn refresh_angle(seconds: f64, reduce_motion: bool) -> f64 {
    if reduce_motion {
        return 0.0;
    }
    (seconds.max(0.0) / tile::SPIN).min(1.0) * 360.0
}

/// `GlassRefreshButton.angle`: `elapsed` seconds after the press (or since
/// the read began, with no press), one turn per `motion.spin`; while
/// `refreshing` it keeps turning, otherwise it stops after one.
pub fn refresh_turn(elapsed: f64, refreshing: bool, reduce_motion: bool) -> f64 {
    if reduce_motion {
        return 0.0;
    }
    let turns = elapsed.max(0.0) / tile::SPIN;
    (if refreshing {
        turns.fract()
    } else {
        turns.min(1.0)
    }) * 360.0
}

/// `GlassRefreshButton.animates`: while a read is out, or the press's
/// first turn is still completing.
pub fn refresh_animates(refreshing: bool, completing_click: bool, reduce_motion: bool) -> bool {
    !reduce_motion && (refreshing || completing_click)
}

/// The refresh icon's frame interval: the Mac's 30 fps timeline.
pub const REFRESH_INTERVAL: f64 = 1.0 / 30.0;

/// Draws the spinner at `angle` degrees, its top-left at the origin.
pub fn draw_spinner(cr: &gtk::cairo::Context, angle: f64, dark: bool) {
    draw_spinner_sized(cr, tile::SPINNER, angle, dark);
}

/// Draws a `side`-point spinner (`GlassSpinner(size:)`): the ring grows,
/// the stroke stays `spinnerStroke`.
pub fn draw_spinner_sized(cr: &gtk::cairo::Context, side: f64, angle: f64, dark: bool) {
    let stroke = tile::SPINNER_STROKE;
    let centre = side / 2.0;
    let radius = (side - stroke) / 2.0;
    tool_tile::ink(0.18, dark).set(cr);
    cr.set_line_width(stroke);
    cr.new_path();
    cr.arc(centre, centre, radius, 0.0, 2.0 * std::f64::consts::PI);
    let _ = cr.stroke();
    // SwiftUI trims a circle from three o'clock, clockwise; the quarter arc
    // then turns with the angle.
    Colour::token(if dark {
        map::dark::TEXT_SECONDARY
    } else {
        map::light::TEXT_SECONDARY
    })
    .set(cr);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    let start = angle.to_radians();
    cr.new_path();
    cr.arc(
        centre,
        centre,
        radius,
        start,
        start + std::f64::consts::FRAC_PI_2,
    );
    let _ = cr.stroke();
}

/// Draws a skeleton bar `width` wide at full opacity.
pub fn draw_skeleton(cr: &gtk::cairo::Context, width: f64, dark: bool) {
    tool_tile::ink(0.08, dark).set(cr);
    tool_tile::rounded(
        cr,
        0.0,
        0.0,
        width,
        tile::SKELETON_HEIGHT,
        tile::RADIUS_CONTROL,
    );
    let _ = cr.fill();
}

/// The Mac's spinner as a widget. To assistive tech it is a progress bar
/// with no value, as the stock spinner was.
pub fn spinner() -> gtk::DrawingArea {
    spinner_sized(tile::SPINNER)
}

/// `GlassSpinner(size:)`: the spinner on a `side`-point ring.
pub fn spinner_sized(side: f64) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(side as i32);
    area.set_content_height(side as i32);
    area.set_halign(gtk::Align::Start);
    area.set_valign(gtk::Align::Center);
    area.set_accessible_role(gtk::AccessibleRole::ProgressBar);
    area.add_css_class("tc-spinner");
    let angle = std::rc::Rc::new(Cell::new(0.0));
    {
        let angle = std::rc::Rc::clone(&angle);
        area.set_draw_func(move |_, cr, _, _| {
            draw_spinner_sized(cr, side, angle.get(), tool_tile::dark())
        });
    }
    let weak = area.downgrade();
    keep_ticking(area.upcast_ref(), 1.0 / 60.0, move |seconds| {
        angle.set(spinner_angle(seconds, !glass_motion::animations_enabled()));
        if let Some(area) = weak.upgrade() {
            area.queue_draw();
        }
    });
    area
}

/// The skeleton's frame interval: every frame. The Mac's pulse is a
/// repeating Core Animation, which runs at the display's rate.
pub const SKELETON_INTERVAL: f64 = 0.0;

/// The class every skeleton bar carries, so a view can find its bars
/// among its other drawing areas.
pub const SKELETON_CLASS: &str = "tc-skeleton";

/// The Mac's skeleton bar, `width` wide, or the width offered for `None`.
/// Hidden from assistive tech: the words beside it say what is loading.
pub fn skeleton(width: Option<i32>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.add_css_class(SKELETON_CLASS);
    area.set_content_height(tile::SKELETON_HEIGHT as i32);
    match width {
        Some(width) => {
            area.set_content_width(width);
            area.set_halign(gtk::Align::Start);
        }
        None => area.set_hexpand(true),
    }
    area.set_accessible_role(gtk::AccessibleRole::Presentation);
    area.set_draw_func(|_, cr, w, _| draw_skeleton(cr, f64::from(w), tool_tile::dark()));
    let weak = area.downgrade();
    keep_ticking(area.upcast_ref(), SKELETON_INTERVAL, move |seconds| {
        if let Some(area) = weak.upgrade() {
            area.set_opacity(skeleton_opacity(
                seconds,
                !glass_motion::animations_enabled(),
            ));
        }
    });
    area
}

mod turn {
    use super::*;

    #[derive(Default)]
    pub struct Turn {
        pub angle: Cell<f64>,
        /// The press, in frame-clock microseconds.
        pub clicked: Cell<Option<i64>>,
        /// Whether a read is out, and since when.
        pub refreshing: Cell<bool>,
        pub since: Cell<i64>,
        pub tick: std::cell::RefCell<Option<gtk::TickCallbackId>>,
        pub last: Cell<f64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Turn {
        const NAME: &'static str = "TcRefreshTurn";
        type Type = super::Turn;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Turn {
        fn constructed(&self) {
            self.parent_constructed();
            // A read still out when the window comes back, or motion is
            // turned back on, starts the icon turning again.
            let obj = self.obj();
            let weak = obj.downgrade();
            glass_motion::on_window_state(&*obj, move |_| {
                if let Some(turn) = weak.upgrade() {
                    turn.run();
                }
            });
            let weak = obj.downgrade();
            glass_motion::on_animations_changed(&*obj, move |_| {
                if let Some(turn) = weak.upgrade() {
                    turn.run();
                }
            });
            let weak = obj.downgrade();
            obj.connect_map(move |_| {
                if let Some(turn) = weak.upgrade() {
                    turn.run();
                }
            });
        }

        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Turn {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            self.obj()
                .first_child()
                .map_or((0, 0, -1, -1), |child| child.measure(orientation, for_size))
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let Some(child) = self.obj().first_child() else {
                return;
            };
            let angle = self.angle.get() as f32;
            let transform = (angle != 0.0).then(|| {
                let (cx, cy) = (width as f32 / 2.0, height as f32 / 2.0);
                gtk::gsk::Transform::new()
                    .translate(&gtk::graphene::Point::new(cx, cy))
                    .rotate(angle)
                    .translate(&gtk::graphene::Point::new(-cx, -cy))
            });
            child.allocate(width, height, baseline, transform);
        }
    }
}

glib::wrapper! {
    /// Holds a refresh button's icon and turns it.
    pub struct Turn(ObjectSubclass<turn::Turn>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Turn {
    pub fn angle(&self) -> f64 {
        self.imp().angle.get()
    }

    /// One turn, from the press.
    pub fn spin(&self) {
        self.imp().clicked.set(Some(gtk::glib::monotonic_time()));
        self.run();
    }

    /// While `on`, the icon keeps turning (`isRefreshing`); off, it comes
    /// to rest once any press's first turn is done.
    pub fn set_refreshing(&self, on: bool) {
        let imp = self.imp();
        if imp.refreshing.replace(on) == on {
            return;
        }
        if on {
            imp.since.set(gtk::glib::monotonic_time());
        }
        self.run();
    }

    fn show(&self, angle: f64) {
        if self.imp().angle.replace(angle) != angle {
            self.queue_allocate();
        }
    }

    /// Ticks at 30 fps while the icon should turn, in a window that can be
    /// seen; otherwise rests.
    fn run(&self) {
        let imp = self.imp();
        if imp.tick.borrow().is_some() {
            return;
        }
        if !imp.refreshing.get() && imp.clicked.get().is_none() {
            return;
        }
        if !glass_motion::animations_enabled() || glass_motion::window_hidden(self) {
            imp.clicked.set(None);
            self.show(0.0);
            return;
        }
        imp.last.set(f64::NEG_INFINITY);
        let id = self.add_tick_callback(|turn, clock| {
            let imp = turn.imp();
            let now = clock.frame_time();
            let refreshing = imp.refreshing.get();
            let elapsed = match imp.clicked.get() {
                Some(at) => (now - at) as f64 / 1_000_000.0,
                None if refreshing => (now - imp.since.get()) as f64 / 1_000_000.0,
                None => 0.0,
            };
            let completing = imp.clicked.get().is_some() && elapsed < tile::SPIN;
            let reduce = !glass_motion::animations_enabled() || glass_motion::window_hidden(turn);
            if !refresh_animates(refreshing, completing, reduce) {
                imp.clicked.set(None);
                imp.tick.borrow_mut().take();
                turn.show(0.0);
                return gtk::glib::ControlFlow::Break;
            }
            if elapsed - imp.last.get() >= REFRESH_INTERVAL {
                imp.last.set(elapsed);
                // A full turn is where it began.
                turn.show(refresh_turn(elapsed, refreshing, false) % 360.0);
            }
            gtk::glib::ControlFlow::Continue
        });
        *imp.tick.borrow_mut() = Some(id);
    }
}

/// The Mac's refresh pill (`GlassRefreshPill`): the word, then the icon
/// that turns. `title` is the word drawn, `name` its tooltip and
/// accessible name; both are the caller's, from shared copy.
pub fn refresh_pill(title: &str, name: &str) -> (gtk::Button, Turn) {
    let icon = gtk::Image::from_icon_name("view-refresh-symbolic");
    icon.set_accessible_role(gtk::AccessibleRole::Presentation);
    let turn: Turn = glib::Object::new();
    icon.set_parent(&turn);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.append(&gtk::Label::new(Some(title)));
    row.append(&turn);
    let button = gtk::Button::new();
    button.set_child(Some(&row));
    // A word and an icon: the toolkit's padding for one.
    button.add_css_class("image-text-button");
    button.set_tooltip_text(Some(name));
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    let spinning = turn.clone();
    button.connect_clicked(move |_| spinning.spin());
    (button, turn)
}

/// A flat refresh button whose icon turns once when it is pressed. `name`
/// is its tooltip and accessible name.
pub fn refresh_button(name: &str) -> (gtk::Button, Turn) {
    let icon = gtk::Image::from_icon_name("view-refresh-symbolic");
    let turn: Turn = glib::Object::new();
    icon.set_parent(&turn);
    let button = gtk::Button::new();
    button.set_child(Some(&turn));
    button.add_css_class("flat");
    button.set_tooltip_text(Some(name));
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    let spinning = turn.clone();
    button.connect_clicked(move |_| spinning.spin());
    (button, turn)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `GlassSpinner.angle`: a turn per 0.8 s, none under reduced motion.
    #[test]
    fn the_spinner_turns_as_the_macs_does() {
        assert_eq!(tile::SPIN, 0.8);
        assert_eq!(spinner_angle(0.0, false), 0.0);
        assert!((spinner_angle(0.2, false) - 90.0).abs() < 1e-9);
        assert!((spinner_angle(1.0, false) - 90.0).abs() < 1e-9);
        assert_eq!(spinner_angle(0.5, true), 0.0);
        assert_eq!((tile::SPINNER, tile::SPINNER_STROKE), (12.0, 1.5));
    }

    /// `GlassRefreshButton.angle` for a press: one turn over 0.8 s, then
    /// still; none under reduced motion.
    #[test]
    fn a_refresh_turns_once() {
        assert_eq!(refresh_angle(0.0, false), 0.0);
        assert!((refresh_angle(0.4, false) - 180.0).abs() < 1e-9);
        assert_eq!(refresh_angle(0.8, false), 360.0);
        assert_eq!(refresh_angle(3.0, false), 360.0);
        assert_eq!(refresh_angle(0.4, true), 0.0);
    }

    /// `GlassSkeleton`: down to 0.5 over half of 1.6 s on the token curve
    /// and back; still under reduced motion.
    #[test]
    fn the_skeleton_breathes_as_the_macs_does() {
        assert_eq!((tile::PULSE, tile::SKELETON_DIM), (1.6, 0.5));
        assert!((skeleton_opacity(0.0, false) - 1.0).abs() < 1e-9);
        assert!((skeleton_opacity(0.8, false) - 0.5).abs() < 1e-9);
        // A quarter of the way down at 0.2 s: the token curve at 0.25.
        let want = 1.0 - 0.5 * 0.767_284_393;
        assert!((skeleton_opacity(0.2, false) - want).abs() < 1e-6);
        assert!((skeleton_opacity(1.6, false) - 1.0).abs() < 1e-9);
        assert_eq!(skeleton_opacity(0.8, true), 1.0);
    }

    /// The ring and arc where the Mac's render (4x, light, at angle 0)
    /// puts them: the arc at three o'clock in the secondary ink, the ring
    /// at nine o'clock in ink 0.18.
    #[test]
    fn the_spinner_draws_the_macs_ring() {
        let s = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 48, 48).unwrap();
        {
            let cr = gtk::cairo::Context::new(&s).unwrap();
            cr.scale(4.0, 4.0);
            Colour::token(map::light::PANE_OPAQUE).set(&cr);
            let _ = cr.paint();
            draw_spinner(&cr, 0.0, false);
        }
        let mut s = s;
        let stride = s.stride() as usize;
        let data = s.data().unwrap();
        let px = |x: f64, y: f64| {
            let at = (y * 4.0) as usize * stride + (x * 4.0) as usize * 4;
            [data[at + 2], data[at + 1], data[at]]
        };
        // Nine o'clock: the ring, black at 0.18 over #f2f3f6.
        let ring = px(0.75, 6.0);
        assert!(
            ring.iter()
                .zip([198, 199, 202])
                .all(|(a, b)| a.abs_diff(b) <= 12),
            "{ring:?}"
        );
        // Just past three o'clock, clockwise: the arc, near #3c3c43.
        let arc = px(11.0, 7.0);
        assert!(
            arc.iter()
                .zip([60, 60, 67])
                .all(|(a, b)| a.abs_diff(b) <= 40),
            "{arc:?}"
        );
    }
    /// A pressed refresh turns its icon once and comes to rest; with
    /// motion off it does not turn.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_a_pressed_refresh_turns_once() {
        gtk::init().unwrap();
        let settings = gtk::Settings::default().unwrap();
        settings.set_gtk_enable_animations(true);
        let (button, turn) = refresh_button("refresh");
        let window = gtk::Window::new();
        window.set_child(Some(&button));
        window.present();
        let context = gtk::glib::MainContext::default();
        let pump_until = |done: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !done() && std::time::Instant::now() < deadline {
                context.iteration(false);
            }
        };
        button.emit_clicked();
        let turned = std::cell::Cell::new(false);
        pump_until(&|| {
            turned.set(turned.get() || turn.angle() > 0.0);
            turned.get() && turn.angle() == 0.0
        });
        assert!(turned.get(), "the icon did not turn");
        assert_eq!(turn.angle(), 0.0);
        settings.set_gtk_enable_animations(false);
        button.emit_clicked();
        assert_eq!(turn.angle(), 0.0);
        window.destroy();
        settings.set_gtk_enable_animations(true);
    }

    /// The Mac's map spinner is `GlassSpinner(size: 20)`: the same 1.5
    /// stroke on a 20-point ring (`FlowMapView.loadingSpinner`).
    #[test]
    fn a_sized_spinner_keeps_the_stroke_on_a_larger_ring() {
        let swift = include_str!(
            "../../../../macos/Sources/TraceCommonsApp/Views/Monitor/FlowMapView.swift"
        );
        assert!(swift.contains("static let loadingSpinner: CGFloat = 20"));
        let s = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 80, 80).unwrap();
        {
            let cr = gtk::cairo::Context::new(&s).unwrap();
            cr.scale(4.0, 4.0);
            Colour::token(map::light::PANE_OPAQUE).set(&cr);
            let _ = cr.paint();
            draw_spinner_sized(&cr, 20.0, 0.0, false);
        }
        let mut s = s;
        let stride = s.stride() as usize;
        let data = s.data().unwrap();
        let px = |x: f64, y: f64| {
            let at = (y * 4.0) as usize * stride + (x * 4.0) as usize * 4;
            [data[at + 2], data[at + 1], data[at]]
        };
        // Nine o'clock on the 9.25-point radius: the ring.
        let ring = px(0.75, 10.0);
        assert!(
            ring.iter()
                .zip([198, 199, 202])
                .all(|(a, b)| a.abs_diff(b) <= 12),
            "{ring:?}"
        );
        // Inside the ring: the ground, so the stroke stayed 1.5.
        let inside = px(2.5, 10.0);
        assert!(
            inside
                .iter()
                .zip([242, 243, 246])
                .all(|(a, b)| a.abs_diff(b) <= 3),
            "{inside:?}"
        );
        // Just past three o'clock, clockwise: the arc.
        let arc = px(19.0, 11.5);
        assert!(
            arc.iter()
                .zip([60, 60, 67])
                .all(|(a, b)| a.abs_diff(b) <= 40),
            "{arc:?}"
        );
    }

    /// No stock spinner is left where the Mac draws its own: the map's
    /// loading spinner and the first run's Customize card.
    #[test]
    fn the_map_and_customize_use_the_glass_spinner() {
        for (name, source) in [
            ("first_run.rs", include_str!("first_run.rs")),
            ("monitor/map.rs", include_str!("monitor/map.rs")),
        ] {
            assert!(
                !source.contains("gtk::Spinner"),
                "{name} still uses gtk::Spinner"
            );
        }
    }

    /// `GlassSkeleton`'s pulse is a repeating Core Animation, drawn at the
    /// display's rate, so the bar's opacity is set every frame.
    #[test]
    fn the_skeleton_breathes_at_the_display_rate() {
        assert_eq!(SKELETON_INTERVAL, 0.0);
    }

    /// `GlassRefreshButton.angle`: a fast answer still completes the
    /// click's turn; while a read is out it keeps turning, a turn per
    /// 0.8 s; none under reduced motion (Swift's own formula, read here).
    #[test]
    fn a_refresh_keeps_turning_while_it_reads() {
        let swift = include_str!("../../../../macos/Sources/TCDesign/Components/Containers.swift");
        assert!(swift.contains(
            "return (isRefreshing ? turns.truncatingRemainder(dividingBy: 1) : min(turns, 1)) * 360"
        ));
        assert!(swift.contains("GlassAnimationTimeline(minimumInterval: 1 / 30,"));
        assert_eq!(REFRESH_INTERVAL, 1.0 / 30.0);
        assert!((refresh_turn(0.4, true, false) - 180.0).abs() < 1e-9);
        assert!((refresh_turn(1.0, true, false) - 90.0).abs() < 1e-9);
        assert!((refresh_turn(2.0, true, false) - 180.0).abs() < 1e-9);
        assert_eq!(refresh_turn(1.0, false, false), 360.0);
        assert_eq!(refresh_turn(1.0, true, true), 0.0);
        assert!(refresh_animates(true, false, false));
        assert!(refresh_animates(false, true, false));
        assert!(!refresh_animates(false, false, false));
        assert!(!refresh_animates(true, true, true));
    }

    /// The Mac's seven refresh controls that have a Linux surface turn
    /// their icon: Managed sessions, Mission drafts and Settings' uses.
    #[test]
    fn the_refresh_controls_turn_their_icon() {
        for (name, source) in [
            ("managed_sessions.rs", include_str!("managed_sessions.rs")),
            ("mission_drafts.rs", include_str!("mission_drafts.rs")),
            ("consent_settings.rs", include_str!("consent_settings.rs")),
        ] {
            assert!(
                source.contains("glass_feedback::refresh_button(")
                    || source.contains("glass_feedback::refresh_pill("),
                "{name} has no turning refresh"
            );
        }
    }

    /// The Mac keeps an icon turning only where it passes `isRefreshing`:
    /// Insights' per-login subscription refresh and the inference
    /// balance card, neither of which has a Linux surface yet. Its
    /// Managed sessions, Mission drafts and Consent buttons turn once per
    /// press, so the Linux ones do too: none of them holds a read open.
    #[test]
    fn only_the_macs_long_reads_keep_turning() {
        for (name, swift, call) in [
            (
                "ManagedSessionsView.swift",
                include_str!(
                    "../../../../macos/Sources/TraceCommonsApp/Views/ManagedSessionsView.swift"
                ),
                "GlassRefreshButton(model.managedText(\"refresh\"))",
            ),
            (
                "MissionDraftsView.swift",
                include_str!(
                    "../../../../macos/Sources/TraceCommonsApp/Views/MissionDraftsView.swift"
                ),
                "GlassRefreshButton(model.text(\"refresh\")",
            ),
            (
                "ConsentSection.swift",
                include_str!(
                    "../../../../macos/Sources/TraceCommonsApp/Views/Settings/ConsentSection.swift"
                ),
                "GlassRefreshButton(SettingsLegacyWords.refreshUses",
            ),
        ] {
            let line = swift
                .lines()
                .find(|line| line.contains(call))
                .unwrap_or_else(|| panic!("{name} has no {call}"));
            assert!(
                !line.contains("isRefreshing"),
                "{name} now keeps turning: {line}"
            );
        }
        for (name, source) in [
            ("managed_sessions.rs", include_str!("managed_sessions.rs")),
            ("mission_drafts.rs", include_str!("mission_drafts.rs")),
            ("consent_settings.rs", include_str!("consent_settings.rs")),
        ] {
            assert!(
                !source.contains("set_refreshing("),
                "{name} keeps turning, the Mac's does not"
            );
        }
        let usage = include_str!(
            "../../../../macos/Sources/TraceCommonsApp/Views/InsightsUsageCards.swift"
        );
        assert!(usage.contains("isRefreshing: subscriptions?.isRefreshing("));
        let balance = include_str!(
            "../../../../macos/Sources/TraceCommonsApp/Views/Monitor/InferenceAccount.swift"
        );
        assert!(balance.contains("isRefreshing: model.balanceRefreshing"));
    }

    /// While a read is out the icon keeps turning; when it ends the icon
    /// comes to rest; with motion off it never turns.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_a_refresh_turns_until_its_read_ends() {
        gtk::init().unwrap();
        let settings = gtk::Settings::default().unwrap();
        settings.set_gtk_enable_animations(true);
        let (button, turn) = refresh_pill("Refresh", "Refresh the list");
        let window = gtk::Window::new();
        window.set_child(Some(&button));
        window.present();
        let context = gtk::glib::MainContext::default();
        let pump_for = |ms: u64| {
            let until = std::time::Instant::now() + std::time::Duration::from_millis(ms);
            while std::time::Instant::now() < until {
                context.iteration(false);
            }
        };
        turn.set_refreshing(true);
        let mut angles = std::collections::BTreeSet::new();
        let until = std::time::Instant::now() + std::time::Duration::from_millis(2000);
        while std::time::Instant::now() < until {
            context.iteration(false);
            angles.insert((turn.angle() / 30.0) as i32);
        }
        assert!(angles.len() >= 6, "it did not keep turning: {angles:?}");
        turn.set_refreshing(false);
        pump_for(200);
        assert_eq!(turn.angle(), 0.0, "it did not come to rest");
        settings.set_gtk_enable_animations(false);
        turn.set_refreshing(true);
        pump_for(200);
        assert_eq!(turn.angle(), 0.0, "motion off: still");
        turn.set_refreshing(false);
        settings.set_gtk_enable_animations(true);
        window.destroy();
    }
}
