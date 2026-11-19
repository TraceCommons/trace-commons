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
//!   `motion.spin`, when it is pressed, at up to 30 frames a second.
//!
//! All three stand still with `gtk-enable-animations` off, following it
//! live, and none ticks while it is not shown or its window is minimised
//! or suspended (`glass_motion::keep_ticking`).

use std::cell::Cell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::glass_motion::{self, Driver, keep_ticking};
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

/// The Mac's skeleton bar, `width` wide, or the width offered for `None`.
/// Hidden from assistive tech: the words beside it say what is loading.
pub fn skeleton(width: Option<i32>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
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
        pub driver: Driver,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Turn {
        const NAME: &'static str = "TcRefreshTurn";
        type Type = super::Turn;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Turn {
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
        let weak = self.downgrade();
        self.imp().driver.start(
            self,
            tile::SPIN,
            1.0 / 30.0,
            move |seconds| {
                let Some(turn) = weak.upgrade() else { return };
                // A full turn is where it began.
                let angle = refresh_angle(seconds, false) % 360.0;
                turn.imp().angle.set(angle);
                turn.queue_allocate();
            },
            || {},
        );
    }
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
        assert!(ring.iter().zip([198, 199, 202]).all(|(a, b)| a.abs_diff(b) <= 12), "{ring:?}");
        // Inside the ring: the ground, so the stroke stayed 1.5.
        let inside = px(2.5, 10.0);
        assert!(inside.iter().zip([242, 243, 246]).all(|(a, b)| a.abs_diff(b) <= 3), "{inside:?}");
        // Just past three o'clock, clockwise: the arc.
        let arc = px(19.0, 11.5);
        assert!(arc.iter().zip([60, 60, 67]).all(|(a, b)| a.abs_diff(b) <= 40), "{arc:?}");
    }

    /// No stock spinner is left where the Mac draws its own: the map's
    /// loading spinner and the first run's Customize card.
    #[test]
    fn the_map_and_customize_use_the_glass_spinner() {
        for (name, source) in [
            ("first_run.rs", include_str!("first_run.rs")),
            ("monitor/map.rs", include_str!("monitor/map.rs")),
        ] {
            assert!(!source.contains("gtk::Spinner"), "{name} still uses gtk::Spinner");
        }
    }

    /// `GlassSkeleton`'s pulse is a repeating Core Animation, drawn at the
    /// display's rate, so the bar's opacity is set every frame.
    #[test]
    fn the_skeleton_breathes_at_the_display_rate() {
        assert_eq!(SKELETON_INTERVAL, 0.0);
    }
}
