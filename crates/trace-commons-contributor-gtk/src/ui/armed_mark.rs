//! The arming offer's mark and how it answers Enable (`ArmingOfferMotion`
//! and `ArmedPulse` in
//! `macos/Sources/TraceCommonsApp/Views/Monitor/ArmingOfferMotion.swift`,
//! drawn as `GlassNoticeMark`).
//!
//! A 30-point coin on the pane's colour with a 2-point rim and a glyph, in
//! the accent while the offer asks: a bolt. Once the daemon has armed the
//! project the glyph turns to a check in the "on" green on a spring
//! (response 0.32, damping 0.7) and a ring spreads from the coin, 1 to 1.9
//! times its size, fading from 0.8, ease-out over 0.7 s. The card stays
//! `HOLD` and then goes, fading on a spring (0.42, 0.9). No words: the
//! change is drawn, not said. With `gtk-enable-animations` off the check
//! is drawn at once, with no pulse and no bounce, and the card simply goes.
//!
//! The bolt and check are SF Symbols on the Mac; here they are drawn by
//! hand to look like them (owner, 2026-11-18), sized from its renders.

use std::cell::Cell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::glass_motion::{self, Driver};
use super::style::brand_tokens::map;
use super::tool_tile::{self, Colour};

/// Where the offer's Enable is (`ArmingOfferStep`): sent, then armed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Arming,
    Armed,
}

/// How long the armed card stays before it leaves (`ArmingOfferMotion.hold`).
pub const HOLD_MS: u64 = 900;
/// The coin's side (`GlassNoticeMark.side`).
pub const SIDE: f64 = 30.0;
/// The mark's change (`ArmingOfferMotion.markChange`).
pub const MARK_SPRING: (f64, f64) = (0.32, 0.7);
/// The card's exit (`ArmingOfferMotion.exit`).
pub const EXIT_SPRING: (f64, f64) = (0.42, 0.9);
/// The pulse: how far the ring spreads, how long, from what opacity.
pub const PULSE_SCALE: f64 = 1.9;
pub const PULSE_SECONDS: f64 = 0.7;
pub const PULSE_OPACITY: f64 = 0.8;

/// Enable is pressed once: locked from the press until the card goes.
pub fn enable_locked(step: Option<Step>) -> bool {
    step.is_some()
}

/// The offer after a daemon refresh: the card pressed stays until its own
/// exit, so a refresh that already reads the project as armed does not
/// pull it from under the check (`offerAfterRefresh`).
pub fn offer_after_refresh<T>(
    current: Option<T>,
    answered: Option<T>,
    step: Option<Step>,
) -> Option<T> {
    if step.is_none() { answered } else { current }
}

/// The ring at `t` seconds after the project is armed: its scale and
/// opacity (`ArmedPulse`).
pub fn pulse_at(t: f64) -> (f64, f64) {
    let p = glass_motion::ease_out(t / PULSE_SECONDS);
    (1.0 + (PULSE_SCALE - 1.0) * p, PULSE_OPACITY * (1.0 - p))
}

/// The mark's tint: the accent while asking, "on" once armed.
pub fn tint(step: Option<Step>, dark: bool) -> Colour {
    Colour::token(match (step == Some(Step::Armed), dark) {
        (true, false) => map::light::STATUS_ON,
        (true, true) => map::dark::STATUS_ON,
        (false, false) => map::light::PURPLE_TEXT,
        (false, true) => map::dark::PURPLE_TEXT,
    })
}

/// SF Symbols' `bolt`, 12 points semibold, centred in the 30-point coin
/// with its top-left at the origin.
pub fn bolt(cr: &gtk::cairo::Context) {
    cr.new_path();
    cr.move_to(18.6, 8.9);
    cr.line_to(11.4, 15.7);
    cr.line_to(15.9, 15.7);
    cr.line_to(13.4, 21.4);
    cr.line_to(19.1, 14.1);
    cr.line_to(14.7, 14.1);
    cr.close_path();
    cr.set_line_width(1.3);
    cr.set_line_join(gtk::cairo::LineJoin::Round);
    let _ = cr.stroke();
}

/// SF Symbols' `checkmark`, 12 points semibold, likewise.
pub fn check(cr: &gtk::cairo::Context) {
    cr.new_path();
    cr.move_to(10.45, 15.3);
    cr.line_to(13.9, 19.3);
    cr.line_to(19.55, 10.45);
    cr.set_line_width(1.4);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.set_line_join(gtk::cairo::LineJoin::Round);
    let _ = cr.stroke();
}

/// Draws the coin with its top-left at the origin: `change` is the mark's
/// spring from bolt (0) to check (1), `pulse` the seconds since armed, or
/// `None` for no ring.
pub fn draw(
    cr: &gtk::cairo::Context,
    step: Option<Step>,
    change: f64,
    pulse: Option<f64>,
    dark: bool,
) {
    let centre = SIDE / 2.0;
    let colour = tint(step, dark);
    let ground = Colour::token(if dark {
        map::dark::PANE_OPAQUE
    } else {
        map::light::PANE_OPAQUE
    });
    ground.set(cr);
    cr.arc(centre, centre, centre, 0.0, 2.0 * std::f64::consts::PI);
    let _ = cr.fill();
    colour.set(cr);
    cr.set_line_width(2.0);
    cr.arc(
        centre,
        centre,
        centre - 1.0,
        0.0,
        2.0 * std::f64::consts::PI,
    );
    let _ = cr.stroke();
    // The glyph: the bolt shrinks away as the check grows in over it
    // (`.symbolEffect(.replace)`, then `.bounce`).
    let glyph = |scale: f64, alpha: f64, draw: fn(&gtk::cairo::Context)| {
        if alpha <= 0.0 || scale <= 0.0 {
            return;
        }
        let _ = cr.save();
        cr.translate(centre, centre);
        cr.scale(scale, scale);
        cr.translate(-centre, -centre);
        colour.alpha(alpha).set(cr);
        draw(cr);
        let _ = cr.restore();
    };
    if step == Some(Step::Armed) {
        let out = change.clamp(0.0, 1.0);
        glyph(1.0 - 0.5 * out, 1.0 - out, bolt);
        glyph(0.5 + 0.5 * change, change.clamp(0.0, 1.0), check);
    } else {
        glyph(1.0, 1.0, bolt);
    }
    if let Some(t) = pulse.filter(|t| *t < PULSE_SECONDS) {
        let (scale, alpha) = pulse_at(t);
        colour.alpha(alpha).set(cr);
        cr.set_line_width(2.0 * scale);
        cr.arc(
            centre,
            centre,
            (centre - 1.0) * scale,
            0.0,
            2.0 * std::f64::consts::PI,
        );
        let _ = cr.stroke();
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ArmedMark {
        pub step: Cell<Option<Step>>,
        pub change: Cell<f64>,
        pub pulse: Cell<Option<f64>>,
        pub driver: Driver,
        pub scheme: std::cell::RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ArmedMark {
        const NAME: &'static str = "TcArmedMark";
        type Type = super::ArmedMark;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ArmedMark {
        fn constructed(&self) {
            self.parent_constructed();
            let weak = self.obj().downgrade();
            let id = adw::StyleManager::default().connect_dark_notify(move |_| {
                if let Some(mark) = weak.upgrade() {
                    mark.queue_draw();
                }
            });
            *self.scheme.borrow_mut() = Some(id);
        }

        fn dispose(&self) {
            if let Some(id) = self.scheme.borrow_mut().take() {
                adw::StyleManager::default().disconnect(id);
            }
        }
    }

    impl WidgetImpl for ArmedMark {
        fn measure(&self, _: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            (SIDE as i32, SIDE as i32, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            // Room for the ring, which spreads past the coin.
            let reach = SIDE * (PULSE_SCALE - 1.0) / 2.0 + 1.0;
            let bounds = gtk::graphene::Rect::new(
                -reach as f32,
                -reach as f32,
                (SIDE + 2.0 * reach) as f32,
                (SIDE + 2.0 * reach) as f32,
            );
            let cr = snapshot.append_cairo(&bounds);
            draw(
                &cr,
                self.step.get(),
                self.change.get(),
                self.pulse.get(),
                tool_tile::dark(),
            );
        }
    }
}

glib::wrapper! {
    /// The offer's coin as a widget. Decorative: the card's words say it.
    pub struct ArmedMark(ObjectSubclass<imp::ArmedMark>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ArmedMark {
    fn default() -> Self {
        Self::new()
    }
}

impl ArmedMark {
    pub fn new() -> Self {
        let mark: Self = glib::Object::builder()
            .property("accessible-role", gtk::AccessibleRole::Presentation)
            .build();
        mark.set_valign(gtk::Align::Start);
        mark
    }

    pub fn step(&self) -> Option<Step> {
        self.imp().step.get()
    }

    /// How far the bolt has turned to the check, and the ring's seconds.
    pub fn motion(&self) -> (f64, Option<f64>) {
        (self.imp().change.get(), self.imp().pulse.get())
    }

    /// Moves the mark to `step`. Armed turns the bolt to a check on its
    /// spring and spreads the ring once; under reduced motion the check is
    /// drawn at once, with neither.
    pub fn set_step(&self, step: Option<Step>) {
        let imp = self.imp();
        let was = imp.step.replace(step);
        if step != Some(Step::Armed) || was == Some(Step::Armed) {
            imp.change
                .set(if step == Some(Step::Armed) { 1.0 } else { 0.0 });
            imp.pulse.set(None);
            self.queue_draw();
            return;
        }
        if !glass_motion::animations_enabled() {
            imp.change.set(1.0);
            imp.pulse.set(None);
            self.queue_draw();
            return;
        }
        let (response, damping) = MARK_SPRING;
        let length = glass_motion::spring_settles(response, damping).max(PULSE_SECONDS);
        let weak = self.downgrade();
        imp.driver.start(
            self,
            length,
            0.0,
            move |t| {
                let Some(mark) = weak.upgrade() else { return };
                let imp = mark.imp();
                imp.change.set(if t >= length {
                    1.0
                } else {
                    glass_motion::spring(response, damping, t)
                });
                imp.pulse.set((t < PULSE_SECONDS).then_some(t));
                mark.queue_draw();
            },
            || {},
        );
    }
}

/// Fades `card` out on the exit spring, then hides it and runs `gone`.
/// Under reduced motion it simply goes.
pub fn exit(card: &impl IsA<gtk::Widget>, gone: impl FnOnce() + 'static) {
    let card = card.as_ref().clone();
    let (response, damping) = EXIT_SPRING;
    let length = glass_motion::spring_settles(response, damping);
    let driver = std::rc::Rc::new(Driver::default());
    let keep = std::rc::Rc::clone(&driver);
    let fading = card.downgrade();
    let ending = card.downgrade();
    driver.start(
        &card,
        length,
        0.0,
        move |t| {
            if let Some(card) = fading.upgrade() {
                let p = if t >= length {
                    1.0
                } else {
                    glass_motion::spring(response, damping, t)
                };
                card.set_opacity((1.0 - p).clamp(0.0, 1.0));
            }
        },
        move || {
            drop(keep);
            if let Some(card) = ending.upgrade() {
                card.set_visible(false);
                card.set_opacity(1.0);
            }
            gone();
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(step: Option<Step>, change: f64, dark: bool) -> gtk::cairo::ImageSurface {
        let s = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 120, 120).unwrap();
        {
            let cr = gtk::cairo::Context::new(&s).unwrap();
            cr.scale(4.0, 4.0);
            draw(&cr, step, change, None, dark);
        }
        s
    }

    /// The glyph's extent in points: pixels of the tint inside the rim.
    fn glyph_extent(s: &mut gtk::cairo::ImageSurface) -> (f64, f64, f64, f64) {
        let stride = s.stride() as usize;
        let data = s.data().unwrap();
        let (mut x0, mut y0, mut x1, mut y1) = (120usize, 120usize, 0usize, 0usize);
        for y in 0..120 {
            for x in 0..120 {
                let (dx, dy) = (x as f64 / 4.0 - 15.0, y as f64 / 4.0 - 15.0);
                if dx.hypot(dy) > 11.0 {
                    continue;
                }
                let at = y * stride + x * 4;
                // Light: the pane is near white; the glyph is dark.
                if u32::from(data[at]) + u32::from(data[at + 1]) + u32::from(data[at + 2]) < 450 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        (
            x0 as f64 / 4.0,
            y0 as f64 / 4.0,
            x1 as f64 / 4.0,
            y1 as f64 / 4.0,
        )
    }

    /// `ArmingOfferMotion`'s numbers.
    #[test]
    fn the_offer_answers_as_the_macs_does() {
        assert_eq!(HOLD_MS, 900);
        assert_eq!(MARK_SPRING, (0.32, 0.7));
        assert_eq!(EXIT_SPRING, (0.42, 0.9));
        assert!(!enable_locked(None));
        assert!(enable_locked(Some(Step::Arming)));
        assert!(enable_locked(Some(Step::Armed)));
        // A refresh while the card is answering keeps the card pressed.
        assert_eq!(
            offer_after_refresh(Some(1), None, Some(Step::Armed)),
            Some(1)
        );
        assert_eq!(
            offer_after_refresh(Some(1), Some(2), Some(Step::Arming)),
            Some(1)
        );
        assert_eq!(offer_after_refresh(Some(1), None, None), None);
        assert_eq!(offer_after_refresh(None, Some(2), None), Some(2));
    }

    /// `ArmedPulse`: 1 to 1.9, opacity 0.8 to 0, ease-out over 0.7 s.
    #[test]
    fn the_pulse_spreads_as_the_macs_does() {
        assert_eq!(pulse_at(0.0), (1.0, 0.8));
        let (scale, alpha) = pulse_at(0.7);
        assert!((scale - 1.9).abs() < 1e-9 && alpha.abs() < 1e-9);
        // SwiftUI's easeOut at half time is 0.6846.
        let (scale, alpha) = pulse_at(0.35);
        assert!((scale - (1.0 + 0.9 * 0.684_643)).abs() < 1e-5, "{scale}");
        assert!((alpha - 0.8 * (1.0 - 0.684_643)).abs() < 1e-5, "{alpha}");
    }

    /// The tints: the accent text colour asking (#023e83 light), the "on"
    /// green armed (#1a8f4c light, #30d158 dark).
    #[test]
    fn the_tints_are_the_macs() {
        assert_eq!(tint(None, false), Colour::token(("#023e83", 1.0)));
        assert_eq!(
            tint(Some(Step::Arming), true),
            Colour::token(("#c6e1ff", 1.0))
        );
        assert_eq!(
            tint(Some(Step::Armed), false),
            Colour::token(("#1a8f4c", 1.0))
        );
        assert_eq!(
            tint(Some(Step::Armed), true),
            Colour::token(("#30d158", 1.0))
        );
    }

    /// The drawn bolt and check cover the boxes SF Symbols' `bolt` and
    /// `checkmark` cover in the Mac's coin, within half a point: the bolt
    /// 10.75..19.5 by 8.25..21.75, the check 9.75..20.25 by 9.75..20.
    #[test]
    fn the_glyphs_are_the_symbols_boxes() {
        let (x0, y0, x1, y1) = glyph_extent(&mut render(None, 0.0, false));
        for (got, want) in [(x0, 10.75), (y0, 8.25), (x1, 19.5), (y1, 21.75)] {
            assert!((got - want).abs() <= 0.5, "bolt {x0},{y0} {x1},{y1}");
        }
        let (x0, y0, x1, y1) = glyph_extent(&mut render(Some(Step::Armed), 1.0, false));
        for (got, want) in [(x0, 9.75), (y0, 9.75), (x1, 20.25), (y1, 20.0)] {
            assert!((got - want).abs() <= 0.5, "check {x0},{y0} {x1},{y1}");
        }
    }
    /// Armed with motion on, the check comes in on its spring and the ring
    /// spreads once; with motion off the check is there at once, with no
    /// ring.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_the_mark_turns_to_a_check_with_a_pulse() {
        gtk::init().unwrap();
        let settings = gtk::Settings::default().unwrap();
        settings.set_gtk_enable_animations(true);
        let mark = ArmedMark::new();
        let window = gtk::Window::new();
        window.set_child(Some(&mark));
        window.present();
        let context = gtk::glib::MainContext::default();
        let pump_until = |done: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !done() && std::time::Instant::now() < deadline {
                context.iteration(false);
            }
        };
        assert_eq!(mark.motion(), (0.0, None));
        mark.set_step(Some(Step::Arming));
        assert_eq!(mark.motion(), (0.0, None));
        mark.set_step(Some(Step::Armed));
        let pulsed = std::cell::Cell::new(false);
        pump_until(&|| {
            pulsed.set(pulsed.get() || mark.motion().1.is_some_and(|t| t > 0.0));
            pulsed.get() && mark.motion() == (1.0, None)
        });
        assert!(pulsed.get(), "no ring spread");
        assert_eq!(mark.motion(), (1.0, None));

        settings.set_gtk_enable_animations(false);
        let still = ArmedMark::new();
        window.set_child(Some(&still));
        still.set_step(Some(Step::Armed));
        assert_eq!(still.motion(), (1.0, None));
        window.destroy();
        settings.set_gtk_enable_animations(true);
    }
}
