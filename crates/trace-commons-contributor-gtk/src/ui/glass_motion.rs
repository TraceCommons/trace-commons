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
//! Mac's `GlassAnimationGate`. In a window that can be seen but is not
//! focused they draw at most `UNFOCUSED_FPS` frames a second, and every
//! other animation in the shell passes its frames through the same `Pace`.
//! With `gtk-enable-animations` off they draw
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
    toplevel(widget.upcast_ref()).is_some_and(|toplevel| hides(state_of(&toplevel)))
}

/// The toplevel's state as the compositor reports it.
#[cfg(not(test))]
fn state_of(toplevel: &gdk::Toplevel) -> gdk::ToplevelState {
    toplevel.state()
}

/// The toplevel's state as the compositor reports it, with `MINIMIZED`
/// added on a window a test has minimised ([`minimise_for_test`]).
#[cfg(test)]
fn state_of(toplevel: &gdk::Toplevel) -> gdk::ToplevelState {
    let minimised = MINIMISED.with(|list| list.borrow().iter().any(|t| t == toplevel));
    if minimised {
        toplevel.state() | gdk::ToplevelState::MINIMIZED
    } else {
        toplevel.state()
    }
}

#[cfg(test)]
thread_local! {
    static MINIMISED: RefCell<Vec<gdk::Toplevel>> = const { RefCell::new(Vec::new()) };
}

/// Minimises `window` (or shows it again) as the compositor would: its
/// toplevel reads `MINIMIZED` and reports a state change, on the path a
/// real minimise takes ([`window_hidden`], [`on_window_state`]). No
/// compositor the tests run under says a window is minimised: xdg-shell
/// never reports it, and weston sends no `suspended`.
#[cfg(test)]
pub fn minimise_for_test(window: &impl IsA<gtk::Widget>, minimised: bool) {
    let toplevel = toplevel(window.upcast_ref()).expect("a shown window");
    MINIMISED.with(|list| {
        let mut list = list.borrow_mut();
        list.retain(|t| t != &toplevel);
        if minimised {
            list.push(toplevel.clone());
        }
    });
    toplevel.notify("state");
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

/// Whether the window's hidden state changed, given the last one `seen`
/// (kept up to date): `Some(hidden)` on a change, `None` for any other
/// state change, such as focus, which [`on_window_state`] also reports.
pub fn hidden_changed(seen: &std::cell::Cell<bool>, hidden: bool) -> Option<bool> {
    (seen.replace(hidden) != hidden).then_some(hidden)
}

/// The most frames a second any animation draws in a window that can be
/// seen but is not focused: the same rule on every platform's app.
pub const UNFOCUSED_FPS: f64 = 15.0;

/// How the window an animation is in can be seen, for its frame rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    /// Shown, and this app's window is the active one.
    Focused,
    /// Shown, while another app has the focus.
    Unfocused,
    /// Minimised or suspended ([`hides`]).
    Hidden,
}

/// The least time between two frames of an animation that wants one
/// every `wanted` seconds (0: every frame the display draws), in a window
/// `seen` so: its own rate while focused; at most [`UNFOCUSED_FPS`] while
/// unfocused (a slower animation keeps its own rate); `None`, no frames,
/// while hidden.
pub fn paced(seen: Seen, wanted: f64) -> Option<f64> {
    match seen {
        Seen::Focused => Some(wanted),
        Seen::Unfocused => Some(wanted.max(1.0 / UNFOCUSED_FPS)),
        Seen::Hidden => None,
    }
}

/// Whether a frame at `now` (seconds) is drawn, given when the last one
/// was due (`last`) and the least `interval` between them: `Some` with
/// the time the next interval counts from when it is. That is a whole
/// interval after the last, not `now`, so frames that land part-way
/// through a display's refresh still average exactly one per interval on
/// any display; a frame more than an interval late (a stall, or the
/// first) counts from itself. A millisecond's slack keeps a frame that
/// rounding puts a hair early.
pub fn frame_due(last: Option<f64>, now: f64, interval: f64) -> Option<f64> {
    const SLACK: f64 = 0.001;
    let Some(last) = last else {
        return Some(now);
    };
    let since = now - last;
    if interval <= 0.0 || since >= 2.0 * interval {
        return Some(now);
    }
    (since >= interval - SLACK).then_some(last + interval)
}

/// Whether this app has the focus: `widget`'s own window is active, or
/// another of the app's windows is (a dialog over it, the rail beside it),
/// so a dialog the person is using never slows the window under it.
#[cfg(not(test))]
fn app_active(widget: &gtk::Widget) -> bool {
    let own = widget
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok())
        .is_some_and(|window| window.is_active());
    own || gtk::Window::list_toplevels().iter().any(|toplevel| {
        toplevel
            .downcast_ref::<gtk::Window>()
            .is_some_and(|window| window.is_active())
    })
}

/// In tests a window is active until a test says otherwise
/// ([`unfocus_for_test`]): which window a test compositor gives the focus
/// to is not the test's to choose, and the mounted tests before this rule
/// assumed every frame.
#[cfg(test)]
fn app_active(widget: &gtk::Widget) -> bool {
    let window = widget
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok());
    !UNFOCUSED.with(|list| window.is_some_and(|window| list.borrow().iter().any(|w| w == &window)))
}

#[cfg(test)]
thread_local! {
    static UNFOCUSED: RefCell<Vec<gtk::Window>> = const { RefCell::new(Vec::new()) };
}

/// Takes the focus from `window` (or gives it back), as another app
/// taking it would, for [`app_active`].
#[cfg(test)]
pub fn unfocus_for_test(window: &gtk::Window, unfocused: bool) {
    UNFOCUSED.with(|list| {
        let mut list = list.borrow_mut();
        list.retain(|w| w != window);
        if unfocused {
            list.push(window.clone());
        }
    });
}

/// How `widget`'s window can be seen now. A widget in no window yet is
/// focused by this: its frame clock does not run anyway.
pub fn seen(widget: &impl IsA<gtk::Widget>) -> Seen {
    let widget = widget.upcast_ref();
    if window_hidden(widget) {
        Seen::Hidden
    } else if widget.root().is_none() || app_active(widget) {
        Seen::Focused
    } else {
        Seen::Unfocused
    }
}

/// The frame-rate gate every animation's frames pass through: the
/// window's focus rule ([`paced`]) over the animation's own rate, read
/// each frame, so taking the focus back restores the full rate at the
/// next frame.
#[derive(Default)]
pub struct Pace {
    last: std::cell::Cell<Option<f64>>,
}

impl Pace {
    /// Whether to draw the frame at `frame_time` (the frame clock's
    /// microseconds, or `glib::monotonic_time` for a timer) of an
    /// animation that wants one every `wanted` seconds, in `widget`'s
    /// window. Never in a hidden window.
    pub fn due(&self, widget: &impl IsA<gtk::Widget>, frame_time: i64, wanted: f64) -> bool {
        let Some(interval) = paced(seen(widget), wanted) else {
            return false;
        };
        match frame_due(self.last.get(), frame_time as f64 / 1e6, interval) {
            Some(next) => {
                self.last.set(Some(next));
                true
            }
            None => false,
        }
    }

    /// Forgets the last frame, so the next one draws: for an animation
    /// starting again.
    pub fn reset(&self) {
        self.last.set(None);
    }
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
    running: Rc<RefCell<Option<Running>>>,
}

/// One frame of a tick: given the widget and the frame clock's time in
/// microseconds, whether to keep ticking.
type Frame = Rc<dyn Fn(&gtk::Widget, i64) -> gtk::glib::ControlFlow>;

/// A [`Driver`]'s movement under way.
struct Running {
    tick: gtk::TickCallbackId,
    duration: f64,
    step: Rc<dyn Fn(f64)>,
    #[cfg_attr(not(test), allow(dead_code))]
    frame: Frame,
}

impl Driver {
    /// Starts the movement. Under reduced motion, or in a minimised or
    /// suspended window, `step(duration)` runs at once and so does `done`;
    /// either turning true part-way ends it the same way at the next
    /// frame. `min_interval` caps the frame rate (the Mac's 30 fps
    /// timelines pass 1/30); 0 draws every frame. In a window that is not
    /// focused it draws at most [`UNFOCUSED_FPS`] ([`Pace`]).
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
        let pace = Pace::default();
        let done = RefCell::new(Some(done));
        let running = Rc::clone(&self.running);
        let ticking = Rc::clone(&step);
        let frame: Frame = Rc::new(move |widget, now| {
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
            if pace.due(widget, now, min_interval) {
                ticking(elapsed);
            }
            gtk::glib::ControlFlow::Continue
        });
        let id = {
            let frame = Rc::clone(&frame);
            widget.add_tick_callback(move |widget, clock| {
                frame(widget.upcast_ref(), clock.frame_time())
            })
        };
        *self.running.borrow_mut() = Some(Running {
            tick: id,
            duration,
            step,
            frame,
        });
    }

    /// Ends a running movement at its last frame, without its `done`.
    pub fn finish(&self) {
        if let Some(running) = self.running.borrow_mut().take() {
            running.tick.remove();
            (running.step)(running.duration);
        }
    }

    /// Stops a running movement where it is, without its last frame or
    /// its `done`: for a `Tween` that turns back from there.
    pub fn stop(&self) {
        if let Some(running) = self.running.borrow_mut().take() {
            running.tick.remove();
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.borrow().is_some()
    }

    /// The running movement's frame `t` seconds in, drawn now: a fake
    /// frame clock for tests, which would otherwise depend on a frame
    /// landing part-way (a software-rendered runner can skip from the
    /// first frame to the last). Returns whether a movement was running.
    #[cfg(test)]
    pub fn step_at(&self, t: f64) -> bool {
        let step = self
            .running
            .borrow()
            .as_ref()
            .map(|running| Rc::clone(&running.step));
        step.map(|step| step(t)).is_some()
    }

    /// The running movement's tick for a frame at `frame_time` (frame
    /// clock microseconds), through the same pacing and ending a real
    /// frame takes: a fake frame clock for counting frames. Returns
    /// whether a movement was running.
    #[cfg(test)]
    pub fn tick_at(&self, widget: &impl IsA<gtk::Widget>, frame_time: i64) -> bool {
        let frame = self
            .running
            .borrow()
            .as_ref()
            .map(|running| Rc::clone(&running.frame));
        frame
            .map(|frame| frame(widget.upcast_ref(), frame_time))
            .is_some()
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
/// resting frame, and it follows the setting and the window live. In a
/// window that is not focused it draws at most [`UNFOCUSED_FPS`].
pub fn keep_ticking(widget: &gtk::Widget, interval: f64, frame: impl Fn(f64) + 'static) -> Ticking {
    let frame: Rc<dyn Fn(f64)> = Rc::new(frame);
    let state = Rc::new(RefCell::new(Loop {
        tick: None,
        began: None,
        step: None,
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
            let pace = Pace::default();
            let step: Frame = Rc::new(move |widget, now| {
                if window_hidden(widget) || !animations_enabled() {
                    ticking.borrow_mut().tick = None;
                    if !animations_enabled() {
                        frame(0.0);
                    }
                    return gtk::glib::ControlFlow::Break;
                }
                let start = *ticking.borrow_mut().began.get_or_insert(now);
                let seconds = (now - start) as f64 / 1_000_000.0;
                if pace.due(widget, now, interval) {
                    frame(seconds);
                }
                gtk::glib::ControlFlow::Continue
            });
            s.step = Some(Rc::clone(&step));
            s.tick = Some(widget.add_tick_callback(move |widget, clock| {
                step(widget.upcast_ref(), clock.frame_time())
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
    Ticking(state)
}

/// A [`keep_ticking`] loop's state.
struct Loop {
    tick: Option<gtk::TickCallbackId>,
    began: Option<i64>,
    #[cfg_attr(not(test), allow(dead_code))]
    step: Option<Frame>,
}

/// A handle on a [`keep_ticking`] loop. Dropping it leaves the loop
/// running; it is there for tests to drive the loop's frames.
pub struct Ticking(#[cfg_attr(not(test), allow(dead_code))] Rc<RefCell<Loop>>);

impl Ticking {
    /// The loop's tick for a frame at `frame_time` (frame clock
    /// microseconds), through the same pacing a real frame takes. Returns
    /// whether the loop was ticking.
    #[cfg(test)]
    pub fn tick_at(&self, widget: &impl IsA<gtk::Widget>, frame_time: i64) -> bool {
        let step = self.0.borrow().step.clone();
        let ticking = self.0.borrow().tick.is_some();
        match step {
            Some(step) if ticking => {
                step(widget.upcast_ref(), frame_time);
                true
            }
            _ => false,
        }
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

    /// A focused window animates at the rate each animation asks for; a
    /// visible window that is not focused at 15 fps at most, or slower if
    /// the animation asks for slower; a hidden one not at all.
    #[test]
    fn an_unfocused_window_animates_at_fifteen_frames_a_second() {
        let fifteen = 1.0 / 15.0;
        assert_eq!(paced(Seen::Focused, 0.0), Some(0.0));
        assert_eq!(paced(Seen::Focused, 1.0 / 30.0), Some(1.0 / 30.0));
        assert_eq!(paced(Seen::Unfocused, 0.0), Some(fifteen));
        assert_eq!(paced(Seen::Unfocused, 1.0 / 30.0), Some(fifteen));
        assert_eq!(paced(Seen::Unfocused, 0.1), Some(0.1), "10 fps stays 10");
        assert_eq!(paced(Seen::Hidden, 0.0), None);
        assert_eq!(paced(Seen::Hidden, 0.1), None);
    }

    /// Frames offered at a display's rate draw at most once per interval,
    /// on average exactly that often whatever the display's rate: 15 of
    /// 60, 15 of 144, every one at interval 0. A frame that lands a
    /// microsecond early by rounding still draws.
    #[test]
    fn frames_are_drawn_once_per_interval_at_any_display_rate() {
        fn drawn(display_hz: u32, seconds: u32, interval: f64) -> usize {
            let mut last = None;
            (0..display_hz * seconds)
                .filter(|k| {
                    let now = (u64::from(*k) * 1_000_000 / u64::from(display_hz)) as f64 / 1e6;
                    match frame_due(last, now, interval) {
                        Some(next) => {
                            last = Some(next);
                            true
                        }
                        None => false,
                    }
                })
                .count()
        }
        let fifteen = 1.0 / 15.0;
        assert_eq!(drawn(60, 1, fifteen), 15);
        assert_eq!(drawn(60, 4, fifteen), 60);
        assert_eq!(drawn(144, 4, fifteen), 60);
        assert_eq!(drawn(120, 2, fifteen), 30);
        assert_eq!(drawn(60, 1, 0.0), 60);
        assert_eq!(drawn(60, 1, 1.0 / 30.0), 30);
        // A frame long after the last (a stall) draws, and the next one
        // waits a whole interval from it, not from the stale schedule.
        assert_eq!(frame_due(Some(0.0), 5.0, fifteen), Some(5.0));
        assert_eq!(frame_due(Some(5.0), 5.05, fifteen), None);
        // The first frame always draws.
        assert_eq!(frame_due(None, 3.0, fifteen), Some(3.0));
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

    /// Shows `area` in a window and waits until it is laid out and on its
    /// frame clock; returns the window and that clock's time now, in
    /// microseconds.
    fn show(area: &gtk::DrawingArea) -> (gtk::Window, i64) {
        area.set_content_width(40);
        area.set_content_height(40);
        let window = gtk::Window::new();
        window.set_child(Some(area));
        window.present();
        pump_until(&|| area.is_mapped() && area.frame_clock().is_some());
        assert!(area.is_mapped(), "the area was never shown");
        let now = area.frame_clock().unwrap().frame_time();
        (window, now)
    }

    /// Frame `k` of a 60 Hz display from `base`, in microseconds.
    fn at_60hz(base: i64, k: i64) -> i64 {
        base + k * 1_000_000 / 60
    }

    /// A movement draws every frame of a 60 Hz display while its window
    /// is focused, 15 a second once another app takes the focus, every
    /// frame again from the first frame after the focus comes back, and
    /// ends at its last frame when the window is minimised. Frames come
    /// from a fake clock fed through the movement's own tick, so the
    /// counts are exact.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_a_movement_runs_at_fifteen_fps_while_its_window_is_unfocused() {
        gtk::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(true);
        let area = gtk::DrawingArea::new();
        let (window, base) = show(&area);
        let steps = Rc::new(RefCell::new(Vec::<f64>::new()));
        let finished = Rc::new(std::cell::Cell::new(false));
        let driver = Driver::default();
        {
            let steps = Rc::clone(&steps);
            let finished = Rc::clone(&finished);
            driver.start(
                &area,
                100.0,
                0.0,
                move |t| steps.borrow_mut().push(t),
                move || finished.set(true),
            );
        }
        let feed = |from: i64, to: i64| {
            let before = steps.borrow().len();
            for k in from..to {
                assert!(driver.tick_at(&area, at_60hz(base, k)), "frame {k}");
            }
            steps.borrow().len() - before
        };
        assert_eq!(seen(&area), Seen::Focused);
        assert_eq!(feed(0, 60), 60, "focused: every frame");
        unfocus_for_test(&window, true);
        assert_eq!(seen(&area), Seen::Unfocused);
        assert_eq!(feed(60, 120), 15, "unfocused: 15 a second");
        assert_eq!(feed(120, 180), 15, "and still 15 the next second");
        unfocus_for_test(&window, false);
        assert_eq!(feed(180, 181), 1, "refocused: the very next frame");
        assert_eq!(feed(181, 240), 59, "and every frame after it");
        assert!(!finished.get());
        minimise_for_test(&window, true);
        assert_eq!(seen(&area), Seen::Hidden);
        assert!(driver.tick_at(&area, at_60hz(base, 240)));
        assert!(finished.get(), "a minimised window ends the movement");
        assert_eq!(steps.borrow().last(), Some(&100.0), "at its last frame");
        assert!(!driver.is_running());
        minimise_for_test(&window, false);
        window.destroy();
    }

    /// A repeating animation does the same: every frame focused, 15 a
    /// second unfocused, a whole fifteenth of a second apart on the
    /// clock's seconds, every frame again on refocus, nothing minimised.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_a_loop_runs_at_fifteen_fps_while_its_window_is_unfocused() {
        gtk::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(true);
        let area = gtk::DrawingArea::new();
        let frames = Rc::new(RefCell::new(Vec::<f64>::new()));
        let ticking = {
            let frames = Rc::clone(&frames);
            keep_ticking(area.upcast_ref(), 0.0, move |t| frames.borrow_mut().push(t))
        };
        let (window, base) = show(&area);
        // Well after any frame the real clock drew while it was shown.
        let base = base + 10_000_000;
        let feed = |from: i64, to: i64| {
            let before = frames.borrow().len();
            for k in from..to {
                assert!(ticking.tick_at(&area, at_60hz(base, k)), "frame {k}");
            }
            frames.borrow().len() - before
        };
        assert_eq!(feed(0, 60), 60, "focused: every frame");
        unfocus_for_test(&window, true);
        assert_eq!(feed(60, 120), 15, "unfocused: 15 a second");
        {
            let frames = frames.borrow();
            let gaps: Vec<f64> = frames[frames.len() - 15..]
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .collect();
            assert!(
                gaps.iter().all(|gap| (gap - 1.0 / 15.0).abs() < 0.01),
                "unfocused frames a fifteenth of a second apart: {gaps:?}"
            );
        }
        unfocus_for_test(&window, false);
        assert_eq!(feed(120, 180), 60, "refocused: every frame");
        minimise_for_test(&window, true);
        let before = frames.borrow().len();
        ticking.tick_at(&area, at_60hz(base, 180));
        assert_eq!(
            frames.borrow().len(),
            before,
            "a minimised window draws nothing"
        );
        assert!(!ticking.tick_at(&area, at_60hz(base, 181)), "and stops");
        minimise_for_test(&window, false);
        window.destroy();
    }

    /// Twelve of the shell's spinners on the real frame clock, three
    /// seconds focused then three unfocused: the unfocused window draws
    /// 15 frames a second at most. Logs the frames and the process's CPU
    /// time for each half (`fps-cpu:` lines in the weston job's log); the
    /// CPU is logged, not asserted, as a shared runner's load varies.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_an_unfocused_window_draws_at_most_fifteen_frames_a_second() {
        use std::io::Write as _;
        /// The process's user and system CPU time, in milliseconds, where
        /// `/proc` has it.
        fn cpu_ms() -> Option<f64> {
            let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
            let fields: Vec<&str> = stat.rsplit_once(") ")?.1.split(' ').collect();
            let ticks: f64 =
                fields.get(11)?.parse::<f64>().ok()? + fields.get(12)?.parse::<f64>().ok()?;
            Some(ticks * 10.0)
        }
        gtk::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(true);
        let grid = gtk::Grid::new();
        for i in 0..12 {
            grid.attach(
                &super::super::glass_feedback::spinner_sized(96.0),
                i % 4,
                i / 4,
                1,
                1,
            );
        }
        let counter = gtk::DrawingArea::new();
        counter.set_content_width(4);
        counter.set_content_height(4);
        let frames = Rc::new(std::cell::Cell::new(0u32));
        let _ticking = {
            let frames = Rc::clone(&frames);
            keep_ticking(counter.upcast_ref(), 0.0, move |_| {
                frames.set(frames.get() + 1)
            })
        };
        grid.attach(&counter, 0, 3, 1, 1);
        let window = gtk::Window::new();
        window.set_child(Some(&grid));
        window.present();
        pump_until(&|| counter.is_mapped());
        assert!(counter.is_mapped(), "never shown");
        // Wake the loop at least every 20 ms, so a blocking iteration
        // cannot outlast a half.
        let wake = gtk::glib::timeout_add_local(std::time::Duration::from_millis(20), || {
            gtk::glib::ControlFlow::Continue
        });
        let context = gtk::glib::MainContext::default();
        let run = |seconds: u64| {
            let (drawn, cpu) = (frames.get(), cpu_ms());
            let end = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
            while std::time::Instant::now() < end {
                context.iteration(true);
            }
            let cpu = cpu.zip(cpu_ms()).map(|(a, b)| b - a);
            (frames.get() - drawn, cpu)
        };
        // Settle first, so the first half is not the window's first frames.
        run(1);
        let (focused, focused_cpu) = run(3);
        unfocus_for_test(&window, true);
        let (unfocused, unfocused_cpu) = run(3);
        unfocus_for_test(&window, false);
        wake.remove();
        // Straight to stderr: libtest keeps a passing test's `println!`
        // out of the log.
        let _ = writeln!(
            std::io::stderr(),
            "fps-cpu: focused {focused} frames in 3 s, cpu {focused_cpu:?} ms; \
             unfocused {unfocused} frames in 3 s, cpu {unfocused_cpu:?} ms"
        );
        assert!(focused > 0, "the clock never ran");
        assert!(
            unfocused <= 3 * 15 + 1,
            "{unfocused} frames in 3 s while unfocused"
        );
        assert!(unfocused > 0, "an unfocused window still moves");
        window.destroy();
    }

    /// Every animation in the shell passes its frames through the focus
    /// rule: each frame-clock tick outside this file calls a [`Pace`]
    /// within its first lines (`Driver`, `Tween` and `keep_ticking` do in
    /// here), save the few ticks that draw nothing and are named below,
    /// and so does each timer that steps an animation. Read from the
    /// source, file by file, so a new tick that skips the rule fails here.
    #[test]
    fn every_animation_passes_the_focus_rule() {
        // (file, ticks in it that are not animations, why).
        let not_motion: [(&str, usize, &str); 3] = [
            (
                "ui/first_run_fade.rs",
                1,
                "restyle_after_drawn waits two frames for a layout, then restyles once",
            ),
            (
                "ui/monitor/traces_list.rs",
                1,
                "scroll_into_view waits for the rows' layout, then moves on a Driver",
            ),
            (
                "ui/settings_navigation.rs",
                1,
                "the layout check reads the width each frame and draws nothing",
            ),
        ];
        // How far into a tick's body its gate must be.
        const WITHIN: usize = 40;
        fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    sources(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        sources(&root, &mut files);
        let mut ticks = 0;
        let mut wrong = Vec::new();
        for path in &files {
            let name = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if name == "ui/glass_motion.rs" {
                continue;
            }
            let text = std::fs::read_to_string(path).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            let mut ungated = 0;
            for (at, line) in lines.iter().enumerate() {
                if !line.contains("add_tick_callback(") || line.trim_start().starts_with("//") {
                    continue;
                }
                ticks += 1;
                let body = &lines[at..(at + WITHIN).min(lines.len())];
                if !body.iter().any(|l| l.contains("pace.due(")) {
                    ungated += 1;
                }
            }
            let allowed = not_motion
                .iter()
                .find(|(file, _, _)| *file == name)
                .map_or(0, |(_, n, _)| *n);
            if ungated != allowed {
                wrong.push(format!(
                    "{name}: {ungated} ungated ticks, {allowed} allowed"
                ));
            }
        }
        assert!(
            ticks >= 13,
            "found only {ticks} ticks: is the walk reading src?"
        );
        assert!(wrong.is_empty(), "ticks outside the focus rule: {wrong:#?}");
        // The timers that step an animation (rather than the frame clock).
        for (file, from) in [
            ("ui/home.rs", "fn count_up("),
            ("ui/hero_card.rs", "hero_art::field::FRAMES_PER_SECOND);"),
        ] {
            let text = std::fs::read_to_string(root.join(file)).unwrap();
            let at = text.find(from).unwrap_or_else(|| panic!("{file}: {from}"));
            let body: String = text[at..]
                .lines()
                .take(WITHIN)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                body.contains("pace.due("),
                "{file}'s timer skips the focus rule"
            );
        }
    }

    /// A window's focus change is not a change in hidden; only going
    /// hidden or shown again is reported.
    #[test]
    fn only_a_hidden_change_is_reported() {
        let seen = std::cell::Cell::new(false);
        assert_eq!(hidden_changed(&seen, false), None, "a focus change");
        assert_eq!(hidden_changed(&seen, true), Some(true));
        assert_eq!(hidden_changed(&seen, true), None);
        assert_eq!(hidden_changed(&seen, false), Some(false));
        assert_eq!(hidden_changed(&seen, false), None);
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
