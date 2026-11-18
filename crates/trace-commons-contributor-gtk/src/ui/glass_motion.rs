//! The Mac's motion curves, as numbers, and one frame-clock driver for the
//! small decorative movements ported from it.
//!
//! - `curve`: `GlassMotion.curve`, the one token curve
//!   (`cubic-bezier(easeX1, easeY1, easeX2, easeY2)` from
//!   `brand_tokens::tile`).
//! - `ease_out`: SwiftUI's `.easeOut`, `cubic-bezier(0, 0, 0.58, 1)`.
//! - `spring`: SwiftUI's `.spring(response:dampingFraction:)`, a damped
//!   spring whose undamped period is `response` seconds.
//!
//! `Driver` runs a movement on the widget's frame clock, which GTK ticks
//! only while the widget is mapped, so nothing runs for a hidden widget.
//! With `gtk-enable-animations` off it draws the last frame at once.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

use super::style::brand_tokens::tile;

/// A cubic Bezier easing from (0, 0) to (1, 1) through `(x1, y1)` and
/// `(x2, y2)`, at time fraction `t`, as CSS and SwiftUI evaluate it.
pub fn bezier(x1: f64, y1: f64, x2: f64, y2: f64, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let at = |a: f64, b: f64, s: f64| {
        let u = 1.0 - s;
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
    };
    // Solve x(s) = t for s by bisection: x is monotonic on [0, 1] for
    // control points inside the unit square.
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..48 {
        let mid = (lo + hi) / 2.0;
        if at(x1, x2, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at(y1, y2, (lo + hi) / 2.0)
}

/// `GlassMotion.curve`: the token curve at time fraction `t`.
pub fn curve(t: f64) -> f64 {
    bezier(
        tile::EASE_X1,
        tile::EASE_Y1,
        tile::EASE_X2,
        tile::EASE_Y2,
        t,
    )
}

/// SwiftUI's `.easeOut` at time fraction `t`.
pub fn ease_out(t: f64) -> f64 {
    bezier(0.0, 0.0, 0.58, 1.0, t)
}

/// SwiftUI's `.spring(response:dampingFraction:)` at `t` seconds: the
/// fraction of the way from start to end, overshooting when underdamped.
pub fn spring(response: f64, damping: f64, t: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    let omega = 2.0 * std::f64::consts::PI / response;
    if damping >= 1.0 {
        return 1.0 - (1.0 + omega * t) * (-omega * t).exp();
    }
    let decay = damping * omega;
    let damped = omega * (1.0 - damping * damping).sqrt();
    1.0 - (-decay * t).exp() * ((damped * t).cos() + decay / damped * (damped * t).sin())
}

/// How long a spring runs: until its envelope is within a ten-thousandth
/// of its end, within 0.05 s of SwiftUI's `settlingDuration`.
pub fn spring_settles(response: f64, damping: f64) -> f64 {
    let omega = 2.0 * std::f64::consts::PI / response;
    (10_000f64).ln() / (damping.min(1.0) * omega)
}

/// Whether this desktop wants motion.
pub fn animations_enabled() -> bool {
    gtk::Settings::default().is_none_or(|settings| settings.is_gtk_enable_animations())
}

/// One movement on a widget's frame clock: `step` gets the seconds since
/// it began, up to `duration`, then `done` runs once. A new `start` ends
/// the one before at its last frame.
#[derive(Default)]
pub struct Driver {
    running: Rc<RefCell<Option<(gtk::TickCallbackId, f64, Rc<dyn Fn(f64)>)>>>,
}

impl Driver {
    /// Starts the movement. Under reduced motion `step(duration)` runs at
    /// once and so does `done`. `min_interval` caps the frame rate (the
    /// Mac's 30 fps timelines pass 1/30); 0 draws every frame.
    pub fn start(
        &self,
        widget: &impl IsA<gtk::Widget>,
        duration: f64,
        min_interval: f64,
        step: impl Fn(f64) + 'static,
        done: impl FnOnce() + 'static,
    ) {
        self.finish();
        let step: Rc<dyn Fn(f64)> = Rc::new(step);
        if !animations_enabled() || duration <= 0.0 {
            step(duration.max(0.0));
            done();
            return;
        }
        let began = Rc::new(std::cell::Cell::new(None::<i64>));
        let last = std::cell::Cell::new(f64::NEG_INFINITY);
        let done = RefCell::new(Some(done));
        let running = Rc::clone(&self.running);
        let ticking = Rc::clone(&step);
        let id = widget.add_tick_callback(move |_, clock| {
            let now = clock.frame_time();
            let start = *began.get().get_or_insert(now);
            began.set(Some(start));
            let elapsed = (now - start) as f64 / 1_000_000.0;
            if elapsed >= duration {
                ticking(duration);
                running.borrow_mut().take();
                if let Some(done) = done.borrow_mut().take() {
                    done();
                }
                return gtk::glib::ControlFlow::Break;
            }
            if elapsed - last.get() >= min_interval {
                last.set(elapsed);
                ticking(elapsed);
            }
            gtk::glib::ControlFlow::Continue
        });
        *self.running.borrow_mut() = Some((id, duration, step));
    }

    /// Ends a running movement at its last frame, without its `done`.
    pub fn finish(&self) {
        if let Some((id, duration, step)) = self.running.borrow_mut().take() {
            id.remove();
            step(duration);
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.borrow().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The token curve is the Mac's `cubic-bezier(0.2, 0.8, 0.2, 1)`, at
    /// the values SwiftUI's own `UnitCurve.bezier` gives (a Swift probe).
    #[test]
    fn the_token_curve_is_the_macs() {
        assert_eq!(
            [tile::EASE_X1, tile::EASE_Y1, tile::EASE_X2, tile::EASE_Y2],
            [0.2, 0.8, 0.2, 1.0]
        );
        for (t, swift) in [
            (0.1, 0.396_692_276),
            (0.25, 0.767_284_393),
            (0.5, 0.946_079_254),
            (0.75, 0.991_107_941),
        ] {
            assert!((curve(t) - swift).abs() < 1e-5, "{t}: {}", curve(t));
        }
        assert!(curve(0.0).abs() < 1e-9 && (curve(1.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn ease_out_is_swiftuis() {
        assert!((ease_out(0.5) - 0.684_642_792).abs() < 1e-5);
        assert!((ease_out(0.25) - 0.378_138_542).abs() < 1e-5);
    }

    /// SwiftUI's `Spring(response:dampingRatio:).value(target: 1, time:)`
    /// for the notice stack's spring (0.36, 0.86), the arming card's exit
    /// (0.42, 0.9) and its mark (0.32, 0.7), from a Swift probe.
    #[test]
    fn springs_are_swiftuis() {
        let swift: [((f64, f64), [f64; 5], f64); 3] = [
            (
                (0.36, 0.86),
                [
                    0.231_173_19,
                    0.567_732_99,
                    0.928_487_26,
                    1.001_427_79,
                    1.001_037_63,
                ],
                0.645,
            ),
            (
                (0.42, 0.9),
                [
                    0.179_468_19,
                    0.467_210_72,
                    0.847_337_99,
                    0.972_926_65,
                    1.001_475_63,
                ],
                0.711,
            ),
            (
                (0.32, 0.7),
                [
                    0.297_642_76,
                    0.713_100_10,
                    1.039_643_35,
                    1.021_726_57,
                    0.998_550_72,
                ],
                0.698,
            ),
        ];
        for ((response, damping), values, settles) in swift {
            for (t, want) in [0.05, 0.1, 0.2, 0.3, 0.5].into_iter().zip(values) {
                let got = spring(response, damping, t);
                assert!(
                    (got - want).abs() < 1e-6,
                    "{response}/{damping} at {t}: {got}"
                );
            }
            let ours = spring_settles(response, damping);
            assert!(
                (ours - settles).abs() < 0.05,
                "{response}/{damping}: {ours}"
            );
        }
    }
}
