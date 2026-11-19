//! The Mac's motion curves, as numbers, and the frame-clock drivers for the
//! decorative movements ported from it.
//!
//! - `curve`: `GlassMotion.curve`, the one token curve
//!   (`cubic-bezier(easeX1, easeY1, easeX2, easeY2)` from
//!   `brand_tokens::motion`).
//! - `ease_out`: SwiftUI's `.easeOut`, `cubic-bezier(0, 0, 0.58, 1)`.
//! - `spring`: SwiftUI's `.spring(response:dampingFraction:)`, a damped
//!   spring whose undamped period is `response` seconds; `spatial` and
//!   `expressive` are `GlassMotion`'s two `.spring(duration:bounce:)`s.
//!
//! `Driver` runs one movement and `Tween` one value that can turn back
//! part-way; `keep_ticking` runs a repeating one. All three run on the
//! widget's frame clock, which GTK ticks only while the widget is mapped,
//! and none runs while its window is minimised or suspended (`hides`), the
//! Mac's `GlassAnimationGate`. With `gtk-enable-animations` off they draw
//! the last (or resting) frame, and they follow that setting while they
//! run, not only when they start.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;

use super::style::brand_tokens::{motion, tile};

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

/// SwiftUI's `.spring(duration:bounce:)` at `t` seconds: response
/// `duration`, damping ratio `1 - bounce`.
pub fn spring_bounce(duration: f64, bounce: f64, t: f64) -> f64 {
    spring(duration, 1.0 - bounce, t)
}

/// `GlassMotion.spatial`: the first run's actors moving between steps.
pub fn spatial(t: f64) -> f64 {
    let (duration, bounce) = motion::SPATIAL;
    spring_bounce(duration, bounce, t)
}

/// `GlassMotion.expressive`: a pop that overshoots and settles.
pub fn expressive(t: f64) -> f64 {
    let (duration, bounce) = motion::EXPRESSIVE;
    spring_bounce(duration, bounce, t)
}

/// Whether this desktop wants motion.
pub fn animations_enabled() -> bool {
    gtk::Settings::default().is_none_or(|settings| settings.is_gtk_enable_animations())
}

/// Runs `changed(enabled)` each time `gtk-enable-animations` changes,
/// until `widget` is destroyed: for a widget whose motion must follow the
/// setting live rather than read it once.
pub fn on_animations_changed(widget: &impl IsA<gtk::Widget>, changed: impl Fn(bool) + 'static) {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    let id = settings.connect_gtk_enable_animations_notify(move |settings| {
        changed(settings.is_gtk_enable_animations());
    });
    let settings = settings.downgrade();
    let id = RefCell::new(Some(id));
    widget.connect_destroy(move |_| {
        if let (Some(settings), Some(id)) = (settings.upgrade(), id.borrow_mut().take()) {
            settings.disconnect(id);
        }
    });
}

/// `GDK_TOPLEVEL_STATE_SUSPENDED` (GTK 4.12): the compositor has said the
/// window cannot be seen, fully covered or on another workspace. Named by
/// value because the binding gates the constant behind a feature.
const SUSPENDED: u32 = 1 << 16;

/// Whether a window in `state` cannot be seen: minimised, or suspended.
pub fn hides(state: gdk::ToplevelState) -> bool {
    state
        .intersects(gdk::ToplevelState::MINIMIZED | gdk::ToplevelState::from_bits_retain(SUSPENDED))
}

/// The window `widget` is in, as a toplevel surface.
fn toplevel(widget: &gtk::Widget) -> Option<gdk::Toplevel> {
    widget.root()?.surface()?.downcast().ok()
}

/// Whether `widget`'s window is minimised or suspended. A widget in no
/// window yet is not hidden by this: its frame clock does not run anyway.
pub fn window_hidden(widget: &impl IsA<gtk::Widget>) -> bool {
    toplevel(widget.upcast_ref()).is_some_and(|toplevel| hides(toplevel.state()))
}

/// Calls `changed(widget)` each time the window `widget` is in is
/// minimised, suspended or shown again, while `widget` is mapped: for a
/// clock of its own to hold while it cannot be seen (`window_hidden`).
pub fn on_window_state(widget: &impl IsA<gtk::Widget>, changed: impl Fn(&gtk::Widget) + 'static) {
    let widget: &gtk::Widget = widget.upcast_ref();
    let changed = Rc::new(changed);
    let held: Rc<RefCell<Option<(gdk::Toplevel, gtk::glib::SignalHandlerId)>>> = Rc::default();
    {
        let held = Rc::clone(&held);
        widget.connect_map(move |widget| {
            let Some(window) = toplevel(widget) else {
                return;
            };
            let weak = widget.downgrade();
            let changed = Rc::clone(&changed);
            let id = window.connect_state_notify(move |_| {
                if let Some(widget) = weak.upgrade() {
                    changed(&widget);
                }
            });
            if let Some((old, old_id)) = held.borrow_mut().replace((window, id)) {
                old.disconnect(old_id);
            }
        });
    }
    widget.connect_unmap(move |_| {
        if let Some((window, id)) = held.borrow_mut().take() {
            window.disconnect(id);
        }
    });
}

/// A tween's value `progress` (eased, 0 to 1) of the way from `from` to `to`.
pub fn tween_value(from: f64, to: f64, progress: f64) -> f64 {
    from + (to - from) * progress
}

/// One movement on a widget's frame clock: `step` gets the seconds since
/// it began, up to `duration`, then `done` runs once. A new `start` ends
/// the one before at its last frame.
#[derive(Default)]
pub struct Driver {
    running: Rc<RefCell<Option<(gtk::TickCallbackId, f64, Rc<dyn Fn(f64)>)>>>,
}

impl Driver {
    /// Starts the movement. Under reduced motion, or in a minimised or
    /// suspended window, `step(duration)` runs at once and so does `done`;
    /// either turning true part-way ends it the same way at the next
    /// frame. `min_interval` caps the frame rate (the Mac's 30 fps
    /// timelines pass 1/30); 0 draws every frame.
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
        if !animations_enabled() || duration <= 0.0 || window_hidden(widget) {
            step(duration.max(0.0));
            done();
            return;
        }
        let began = Rc::new(std::cell::Cell::new(None::<i64>));
        let last = std::cell::Cell::new(f64::NEG_INFINITY);
        let done = RefCell::new(Some(done));
        let running = Rc::clone(&self.running);
        let ticking = Rc::clone(&step);
        let id = widget.add_tick_callback(move |widget, clock| {
            let now = clock.frame_time();
            let start = *began.get().get_or_insert(now);
            began.set(Some(start));
            let elapsed = (now - start) as f64 / 1_000_000.0;
            // The setting is read every frame, so turning motion off ends a
            // movement that is already running (a property read, no parse).
            if elapsed >= duration || !animations_enabled() || window_hidden(widget) {
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

    /// Stops a running movement where it is, without its last frame or
    /// its `done`: for a `Tween` that turns back from there.
    pub fn stop(&self) {
        if let Some((id, _, _)) = self.running.borrow_mut().take() {
            id.remove();
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.borrow().is_some()
    }
}

/// One value moving on a widget's frame clock toward a target that can
/// change part-way (a hover wash, a switch's knob, a chevron's turn): a
/// new target runs from where the value is, over the whole duration, as
/// SwiftUI starts a reversed animation from the presented value.
#[derive(Default)]
pub struct Tween {
    value: Rc<std::cell::Cell<f64>>,
    target: std::cell::Cell<f64>,
    driver: Driver,
}

impl Tween {
    pub fn new(value: f64) -> Self {
        let tween = Self::default();
        tween.value.set(value);
        tween.target.set(value);
        tween
    }

    /// Where the value is now.
    pub fn value(&self) -> f64 {
        self.value.get()
    }

    /// Where it is going.
    pub fn target(&self) -> f64 {
        self.target.get()
    }

    /// Moves to `target` over `duration` on `ease`, calling `frame` with
    /// each new value. Under reduced motion, or in a hidden window, it is
    /// there at once. A target it is already at or moving to is ignored.
    pub fn to(
        &self,
        widget: &impl IsA<gtk::Widget>,
        target: f64,
        duration: f64,
        ease: fn(f64) -> f64,
        frame: impl Fn(f64) + 'static,
    ) {
        if target == self.target.get() && (self.driver.is_running() || self.value.get() == target) {
            return;
        }
        self.target.set(target);
        self.driver.stop();
        let from = self.value.get();
        let value = Rc::clone(&self.value);
        self.driver.start(
            widget,
            duration,
            0.0,
            move |t| {
                let progress = if duration > 0.0 {
                    ease(t / duration)
                } else {
                    1.0
                };
                let now = if t >= duration {
                    target
                } else {
                    tween_value(from, target, progress)
                };
                value.set(now);
                frame(now);
            },
            || {},
        );
    }

    /// Jumps to `target`, ending any movement, and draws it.
    pub fn set(&self, target: f64, frame: impl Fn(f64)) {
        self.driver.stop();
        self.target.set(target);
        self.value.set(target);
        frame(target);
    }
}

/// `GlassExpander`'s details: opened by a click or a key, they fade in
/// over `motion.fast` on the token curve (the button's toggle runs in
/// `withAnimation(GlassMotion.fast)`, and SwiftUI's default transition is
/// opacity). Opened from code (a list rebuilt with its rows kept open, a
/// result shown) they are whole at once, as the Mac animates only the
/// button. Closed, they go at once: the toolkit takes the child out of the
/// expander, so nothing is left to fade. The chevron's turn is
/// `motion_css`'s.
pub fn expander(expander: &gtk::Expander) {
    let pressed = Rc::new(std::cell::Cell::new(false));
    {
        // `activate` is what a click on the title and Enter or Space emit,
        // just before the toolkit flips `expanded`.
        let pressed = Rc::clone(&pressed);
        expander.connect_activate(move |_| pressed.set(true));
    }
    let tween = Tween::new(1.0);
    expander.connect_expanded_notify(move |expander| {
        let by_hand = pressed.replace(false);
        let Some(child) = expander.child() else {
            return;
        };
        if !expander.is_expanded() || !by_hand {
            tween.set(1.0, |_| {});
            child.set_opacity(1.0);
            return;
        }
        tween.set(0.0, |_| {});
        child.set_opacity(0.0);
        let weak = child.downgrade();
        tween.to(expander, 1.0, motion::FAST, curve, move |opacity| {
            if let Some(child) = weak.upgrade() {
                child.set_opacity(opacity);
            }
        });
    });
}

/// The first run's folder disclosure (`ToolsScreen`'s repo rows): the
/// chevron turns over `motion.standard`, and the past traces appear at
/// once, since the Mac animates only the chevron there.
pub fn disclosure(expander: &gtk::Expander) {
    expander.add_css_class(DISCLOSURE_CLASS);
}

/// The class `motion_css` turns a disclosure's chevron by.
pub const DISCLOSURE_CLASS: &str = "tc-disclosure-standard";

/// Runs `frame(seconds)` on `widget`'s frame clock, at most once per
/// `interval` (0: every frame), while the widget is mapped, its window is
/// neither minimised nor suspended, and motion is wanted. The seconds
/// count from its first frame and keep counting while it is held, so it
/// picks up where it would have been, as the Mac's timelines take their
/// phase from the clock. With motion off it draws `frame(0.0)`, the
/// resting frame, and it follows the setting and the window live.
pub fn keep_ticking(widget: &gtk::Widget, interval: f64, frame: impl Fn(f64) + 'static) {
    struct Loop {
        tick: Option<gtk::TickCallbackId>,
        began: Option<i64>,
    }
    let frame: Rc<dyn Fn(f64)> = Rc::new(frame);
    let state = Rc::new(RefCell::new(Loop {
        tick: None,
        began: None,
    }));
    // Starts or stops the tick to match the widget, its window and the
    // setting. Called on every change of any of them.
    let sync: Rc<dyn Fn(&gtk::Widget)> = {
        let state = Rc::clone(&state);
        let frame = Rc::clone(&frame);
        Rc::new(move |widget: &gtk::Widget| {
            let enabled = animations_enabled();
            let run = widget.is_mapped() && enabled && !window_hidden(widget);
            let mut s = state.borrow_mut();
            if !run {
                if let Some(id) = s.tick.take() {
                    id.remove();
                }
                drop(s);
                if !enabled {
                    frame(0.0);
                }
                return;
            }
            if s.tick.is_some() {
                return;
            }
            let frame = Rc::clone(&frame);
            let ticking = Rc::clone(&state);
            let last = std::cell::Cell::new(f64::NEG_INFINITY);
            s.tick = Some(widget.add_tick_callback(move |widget, clock| {
                if window_hidden(widget) || !animations_enabled() {
                    ticking.borrow_mut().tick = None;
                    if !animations_enabled() {
                        frame(0.0);
                    }
                    return gtk::glib::ControlFlow::Break;
                }
                let now = clock.frame_time();
                let start = *ticking.borrow_mut().began.get_or_insert(now);
                let seconds = (now - start) as f64 / 1_000_000.0;
                if seconds - last.get() >= interval {
                    last.set(seconds);
                    frame(seconds);
                }
                gtk::glib::ControlFlow::Continue
            }));
        })
    };
    {
        let sync = Rc::clone(&sync);
        widget.connect_map(move |widget| sync(widget));
    }
    {
        let sync = Rc::clone(&sync);
        widget.connect_unmap(move |widget| sync(widget));
    }
    {
        let sync = Rc::clone(&sync);
        on_window_state(widget, move |widget| sync(widget));
    }
    let weak = widget.downgrade();
    on_animations_changed(widget, move |_| {
        if let Some(widget) = weak.upgrade() {
            sync(&widget);
        }
    });
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

    /// The tokens are the Mac's (`GlassTokens.Motion`, `GlassMotion`):
    /// generated from the JSON and from `GlassMotion.swift`, and read here
    /// from the Swift once more so a hand edit on either side fails.
    #[test]
    fn the_motion_tokens_are_the_macs() {
        let swift = include_str!("../../../../macos/Sources/TCDesign/Generated/GlassTokens.swift");
        for (name, ours) in [
            ("fast", motion::FAST),
            ("standard", motion::STANDARD),
            ("slow", motion::SLOW),
            ("reveal", motion::REVEAL),
            ("slide", motion::SLIDE),
            ("spin", motion::SPIN),
            ("pulse", motion::PULSE),
        ] {
            let line = format!("public static let {name}: Double = {ours}");
            assert!(swift.contains(&line), "{line}");
        }
        assert_eq!(motion::EASE, [0.2, 0.8, 0.2, 1.0]);
        let glass_motion =
            include_str!("../../../../macos/Sources/TCDesign/Styling/GlassMotion.swift");
        assert!(
            glass_motion.contains("func spatial(_ reduceMotion: Bool, duration: Double = 0.9)")
        );
        assert!(glass_motion.contains(".spring(duration: duration, bounce: 0.04)"));
        assert!(
            glass_motion.contains("func expressive(_ reduceMotion: Bool, duration: Double = 0.78)")
        );
        assert!(glass_motion.contains(".spring(duration: duration, bounce: 0.32)"));
        assert_eq!(motion::SPATIAL, (0.9, 0.04));
        assert_eq!(motion::EXPRESSIVE, (0.78, 0.32));
    }

    /// SwiftUI's `Spring(duration: 0.9, bounce: 0.04)` and
    /// `Spring(duration: 0.78, bounce: 0.32)`, `value(target: 1, time:)`,
    /// from a Swift probe.
    #[test]
    fn spatial_and_expressive_are_swiftuis() {
        for (t, want) in [
            (0.05, 0.048_843_494),
            (0.1, 0.157_440_324),
            (0.2, 0.416_044_334),
            (0.4, 0.785_885_515),
            (0.7, 0.967_346_093),
        ] {
            assert!(
                (spatial(t) - want).abs() < 1e-6,
                "spatial {t}: {}",
                spatial(t)
            );
        }
        for (t, want) in [
            (0.05, 0.067_244_612),
            (0.1, 0.221_075_557),
            (0.2, 0.586_156_741),
            (0.4, 1.006_700_401),
            (0.7, 1.028_599_566),
        ] {
            assert!(
                (expressive(t) - want).abs() < 1e-6,
                "expressive {t}: {}",
                expressive(t)
            );
        }
        // SwiftUI's settlingDuration: 1.335 s and 1.602 s.
        let (d, b) = motion::SPATIAL;
        assert!((spring_settles(d, 1.0 - b) - 1.335).abs() < 0.05);
        let (d, b) = motion::EXPRESSIVE;
        assert!((spring_settles(d, 1.0 - b) - 1.602).abs() < 0.1);
    }

    /// A window minimised, or suspended by the compositor (fully covered,
    /// on another workspace), holds its animations, as the Mac's
    /// `GlassAnimationGate` does for an occluded or miniaturised window.
    #[test]
    fn a_minimised_or_suspended_window_holds_still() {
        use gtk::gdk::ToplevelState as S;
        assert!(!hides(S::empty()));
        assert!(!hides(S::FOCUSED | S::MAXIMIZED));
        assert!(hides(S::MINIMIZED));
        assert!(hides(S::from_bits_retain(1 << 16)));
        assert!(hides(S::FOCUSED | S::from_bits_retain(1 << 16)));
    }

    /// A tween reversed part-way runs from where it is, not from its old
    /// start: a hover that leaves halfway through its fade fades back
    /// from there.
    #[test]
    fn a_reversed_tween_starts_from_where_it_is() {
        assert_eq!(tween_value(0.0, 1.0, 0.0), 0.0);
        assert!((tween_value(0.0, 1.0, curve(0.5)) - 0.946_079_254).abs() < 1e-6);
        // Back from 0.6 to 0: a quarter of the way along the curve.
        let want = 0.6 - 0.6 * 0.767_284_393;
        assert!((tween_value(0.6, 0.0, curve(0.25)) - want).abs() < 1e-6);
        assert_eq!(tween_value(0.6, 0.0, 1.0), 0.0);
    }

    fn pump_until(done: &dyn Fn() -> bool) {
        let context = gtk::glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done() && std::time::Instant::now() < deadline {
            context.iteration(false);
        }
    }

    /// Turning motion off ends a running movement at its last frame and
    /// runs its `done`, at the next frame, not only when it starts.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_a_running_driver_follows_reduce_motion_live() {
        gtk::init().unwrap();
        let settings = gtk::Settings::default().unwrap();
        settings.set_gtk_enable_animations(true);
        let area = gtk::DrawingArea::new();
        let window = gtk::Window::new();
        window.set_child(Some(&area));
        window.present();
        let seen = Rc::new(std::cell::Cell::new(-1.0));
        let finished = Rc::new(std::cell::Cell::new(false));
        let driver = Driver::default();
        {
            let seen = Rc::clone(&seen);
            let finished = Rc::clone(&finished);
            driver.start(
                &area,
                30.0,
                0.0,
                move |t| seen.set(t),
                move || finished.set(true),
            );
        }
        pump_until(&|| seen.get() > 0.0);
        assert!(
            seen.get() > 0.0 && seen.get() < 30.0,
            "it never ticked: {}",
            seen.get()
        );
        settings.set_gtk_enable_animations(false);
        area.queue_draw();
        pump_until(&|| finished.get());
        assert!(finished.get(), "the movement kept running with motion off");
        assert_eq!(seen.get(), 30.0);
        settings.set_gtk_enable_animations(true);
        window.destroy();
    }

    /// A repeating animation draws its resting frame when motion is turned
    /// off and starts again when it is turned back on, with no restart of
    /// the widget.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_a_loop_follows_reduce_motion_live() {
        gtk::init().unwrap();
        let settings = gtk::Settings::default().unwrap();
        settings.set_gtk_enable_animations(true);
        let area = gtk::DrawingArea::new();
        let frames = Rc::new(RefCell::new(Vec::<f64>::new()));
        {
            let frames = Rc::clone(&frames);
            keep_ticking(area.upcast_ref(), 0.0, move |t| frames.borrow_mut().push(t));
        }
        let window = gtk::Window::new();
        window.set_child(Some(&area));
        window.present();
        pump_until(&|| frames.borrow().iter().any(|t| *t > 0.0));
        assert!(frames.borrow().iter().any(|t| *t > 0.0), "it never ticked");
        settings.set_gtk_enable_animations(false);
        assert_eq!(frames.borrow().last(), Some(&0.0), "no resting frame");
        let count = frames.borrow().len();
        let context = gtk::glib::MainContext::default();
        let until = std::time::Instant::now() + std::time::Duration::from_millis(300);
        while std::time::Instant::now() < until {
            area.queue_draw();
            context.iteration(false);
        }
        assert_eq!(frames.borrow().len(), count, "it ticked with motion off");
        settings.set_gtk_enable_animations(true);
        pump_until(&|| frames.borrow().len() > count + 1);
        assert!(frames.borrow().len() > count + 1, "it did not start again");
        window.destroy();
    }

    /// A clock that runs on its own (not through `keep_ticking` or a
    /// `Driver`) holds while its window is minimised or suspended, and
    /// starts again when the window is shown: the first-run stage, the
    /// map's clock and shimmer, the hero art, and the refresh turn.
    #[test]
    fn every_long_clock_holds_while_its_window_is_hidden() {
        for (name, source) in [
            (
                "first_run_stage_view.rs",
                include_str!("first_run_stage_view.rs"),
            ),
            ("monitor/map.rs", include_str!("monitor/map.rs")),
            ("hero_card.rs", include_str!("hero_card.rs")),
            ("glass_feedback.rs", include_str!("glass_feedback.rs")),
        ] {
            assert!(
                source.contains("window_hidden("),
                "{name}: its clock runs on while the window is hidden"
            );
            assert!(
                source.contains("on_window_state("),
                "{name}: its clock does not start again when the window is shown"
            );
        }
    }

    /// Every expander the shell builds moves as `GlassExpander` does (or
    /// as the first run's disclosure, which turns only its chevron): one
    /// `expander` or `disclosure` call per expander built, file by file.
    #[test]
    fn every_expander_takes_the_macs_motion() {
        let sources = [
            ("first_run.rs", include_str!("first_run.rs")),
            ("first_run_screens.rs", include_str!("first_run_screens.rs")),
            ("history.rs", include_str!("history.rs")),
            ("insights.rs", include_str!("insights.rs")),
            ("private_inference.rs", include_str!("private_inference.rs")),
            ("queue.rs", include_str!("queue.rs")),
            ("settings.rs", include_str!("settings.rs")),
            ("monitor/summary.rs", include_str!("monitor/summary.rs")),
            (
                "monitor/transfer_card.rs",
                include_str!("monitor/transfer_card.rs"),
            ),
        ];
        let mut total = 0;
        for (name, source) in sources {
            let built = source.matches("gtk::Expander::builder()").count()
                + source.matches("gtk::Expander::new(").count();
            let moved = source.matches("glass_motion::expander(").count()
                + source.matches("glass_motion::disclosure(").count();
            assert!(
                built > 0,
                "{name} builds no expander: drop it from the list"
            );
            assert_eq!(built, moved, "{name}: {built} built, {moved} moved");
            total += built;
        }
        // Each file that builds one is listed: a new file must join it.
        assert_eq!(total, 15);
    }

    /// An expander opened by hand fades its details in over `motion.fast`,
    /// passing through the middle; opened from code, or with motion off,
    /// they are whole at once.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_an_opened_expanders_details_fade_in() {
        gtk::init().unwrap();
        let settings = gtk::Settings::default().unwrap();
        settings.set_gtk_enable_animations(true);
        let details = gtk::Label::new(Some("details"));
        let control = gtk::Expander::builder()
            .label("more")
            .child(&details)
            .build();
        expander(&control);
        let window = gtk::Window::new();
        window.set_child(Some(&control));
        window.present();
        pump_until(&|| control.is_mapped());
        // Opened from code (a rebuilt list keeping a row open): at once.
        control.set_expanded(true);
        assert_eq!(details.opacity(), 1.0, "opened from code, it faded");
        control.set_expanded(false);
        // Opened by hand: from clear.
        control.activate();
        assert!(control.is_expanded());
        assert_eq!(details.opacity(), 0.0, "the details start clear");
        let between = std::cell::Cell::new(false);
        pump_until(&|| {
            let o = details.opacity();
            between.set(between.get() || (o > 0.0 && o < 1.0));
            o == 1.0
        });
        assert!(between.get(), "no frame between clear and whole");
        assert_eq!(details.opacity(), 1.0);
        control.set_expanded(false);
        settings.set_gtk_enable_animations(false);
        control.activate();
        assert_eq!(details.opacity(), 1.0, "motion off: whole at once");
        settings.set_gtk_enable_animations(true);
        window.destroy();
    }
}
