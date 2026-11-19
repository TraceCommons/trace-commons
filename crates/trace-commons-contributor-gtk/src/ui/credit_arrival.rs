//! The tray icon's celebration when credit arrives, and the window's
//! request for attention: the Mac's `MenuBarStripLabel` credit burst and
//! `DockCreditAnimator.celebrate` (#2197), as the owner chose for Linux
//! (2026-11-18, "Tray animation + taskbar flash"): the tray icon gets the
//! menu-bar item's gleam, hop and mini coins, and the window the Dock's
//! informational bounce, both behind Settings' credit-arrival switch.
//!
//! - **What fires it** is `credit_arrival::Tracker`: a whole credit
//!   arriving while the app runs, never one from before it started.
//! - **Frames** are the Mac's schedule: 41 frames at k/24 s, 42 ms apart,
//!   then the icon at rest. Under reduced motion, one frame at 0.45 s held
//!   1.6 s and no attention request, as the Mac skips the bounce when
//!   still. Reduced motion is read at each frame, so turning it on mid-way
//!   moves to the still frame at once.
//! - **The switch** is read before each frame; off, the icon is at rest at
//!   once (`GeneralSection`'s `DockCreditAnimator.shared.stop()`).
//! - **Attention** is asked only of a window that is shown and not active,
//!   as `requestUserAttention(.informationalRequest)` does nothing for the
//!   active app. A window closed to the tray is never shown for it. On X11
//!   it is the window's urgency hint (`_NET_WM_STATE_DEMANDS_ATTENTION`
//!   through `XWMHints`), cleared once the window is active; a taskbar or
//!   dock flashes or highlights the entry. On Wayland it is GTK's
//!   xdg-activation request (`gdk_toplevel_focus`) for a window that does
//!   not hold the keyboard: GNOME Shell shows its "is ready" notice, KDE
//!   Plasma highlights the task, Sway and Hyprland mark the window urgent;
//!   none moves focus, because the token carries no current input.
//!
//! Nothing here ticks at rest: each frame schedules the next, and the last
//! schedules nothing.

use std::cell::Cell;
use std::rc::{Rc, Weak};

use crate::credit_arrival::{Tracker, icon_animation};

/// What a celebration does in the world, so the schedule can be tested
/// without a display or a tray host.
pub struct Hooks {
    /// Whether the desktop asks for reduced motion now.
    pub reduce_motion: Box<dyn Fn() -> bool>,
    /// The window's request for attention.
    pub attention: Box<dyn Fn()>,
    /// Draw the icon `t` seconds in, or at rest.
    pub show: Box<dyn Fn(Option<f64>)>,
    /// Run the callback after this many milliseconds.
    pub after: Box<dyn Fn(u64, Box<dyn FnOnce()>)>,
}

/// One frame of a celebration and how long it stays: `None` is the icon at
/// rest, the last entry.
pub fn plan(still: bool) -> Vec<(Option<f64>, u64)> {
    let wait = if still {
        icon_animation::REDUCE_MOTION_HOLD_MS
    } else {
        icon_animation::frame_interval_ms()
    };
    icon_animation::frame_times(still)
        .into_iter()
        .map(|t| (Some(t), wait))
        .chain(std::iter::once((None, 0)))
        .collect()
}

pub struct CreditArrival {
    tracker: Cell<Tracker>,
    enabled: Cell<bool>,
    /// Bumped by every start and stop, so a frame scheduled by an earlier
    /// celebration finds it is no longer current and does nothing.
    generation: Cell<u64>,
    hooks: Hooks,
}

impl CreditArrival {
    pub fn new(enabled: bool, hooks: Hooks) -> Rc<Self> {
        Rc::new(Self {
            tracker: Cell::new(Tracker::default()),
            enabled: Cell::new(enabled),
            generation: Cell::new(0),
            hooks,
        })
    }

    /// A NEAR AI balance read: celebrates a whole credit received.
    pub fn observe_received(self: &Rc<Self>, received_total_nanos: Option<i64>) {
        let mut tracker = self.tracker.get();
        let fired = tracker.observe(received_total_nanos);
        self.tracker.set(tracker);
        if fired {
            self.celebrate();
        }
    }

    /// A history rollup read: celebrates a whole Trace credit settled.
    pub fn observe_credit_final(self: &Rc<Self>, credit_final: Option<f64>) {
        let mut tracker = self.tracker.get();
        let fired = tracker.observe_credit_final(credit_final);
        self.tracker.set(tracker);
        if fired {
            self.celebrate();
        }
    }

    /// Settings' switch. Off, a celebration in progress stops at once.
    pub fn set_enabled(&self, on: bool) {
        self.enabled.set(on);
        if !on {
            self.stop();
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled.get()
    }

    /// `DockCreditAnimator.celebrate`: one celebration from its start; a
    /// second while one runs starts it again.
    pub fn celebrate(self: &Rc<Self>) {
        if !self.enabled.get() {
            self.stop();
            return;
        }
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let still = (self.hooks.reduce_motion)();
        if !still {
            (self.hooks.attention)();
        }
        self.frame(generation, Rc::new(plan(still)), 0, still);
    }

    /// The icon at rest, and no frame still to come.
    pub fn stop(&self) {
        self.generation.set(self.generation.get() + 1);
        (self.hooks.show)(None);
    }

    fn frame(
        self: &Rc<Self>,
        generation: u64,
        frames: Rc<Vec<(Option<f64>, u64)>>,
        index: usize,
        still: bool,
    ) {
        if generation != self.generation.get() {
            return;
        }
        if !self.enabled.get() {
            (self.hooks.show)(None);
            return;
        }
        if !still && (self.hooks.reduce_motion)() {
            self.frame(generation, Rc::new(plan(true)), 0, true);
            return;
        }
        let (shown, wait) = frames[index];
        (self.hooks.show)(shown);
        if index + 1 < frames.len() {
            let weak: Weak<Self> = Rc::downgrade(self);
            (self.hooks.after)(
                wait,
                Box::new(move || {
                    if let Some(this) = weak.upgrade() {
                        this.frame(generation, frames, index + 1, still);
                    }
                }),
            );
        }
    }
}

/// Asks the desktop to draw attention to `window`, as the Mac's
/// informational request does, and only when it is shown and not active.
pub fn request_attention(window: &gtk::Window) {
    use gtk::prelude::*;
    if !window.is_visible() || window.is_active() {
        return;
    }
    let Some(surface) = window.surface() else {
        return;
    };
    #[cfg(target_os = "linux")]
    {
        if let Some(x11) = surface.downcast_ref::<gdk4_x11::X11Surface>() {
            x11.set_urgency_hint(true);
            return;
        }
        if let Some(toplevel) = surface.downcast_ref::<gdk4_wayland::WaylandToplevel>() {
            use gtk::gdk::prelude::ToplevelExt;
            toplevel.focus(gtk::gdk::CURRENT_TIME);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = surface;
}

/// Clears the X11 urgency hint once `window` is active again, as the Dock
/// stops bouncing when the app comes forward.
pub fn clear_attention_when_active(window: &gtk::Window) {
    use gtk::prelude::*;
    window.connect_is_active_notify(|window| {
        #[cfg(target_os = "linux")]
        if window.is_active()
            && let Some(x11) = window
                .surface()
                .and_then(|surface| surface.downcast::<gdk4_x11::X11Surface>().ok())
        {
            x11.set_urgency_hint(false);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = window;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A recorded world: what the icon showed, the waits asked for, how
    /// often attention was asked, with the timer drained by hand.
    #[derive(Default)]
    struct World {
        shown: RefCell<Vec<Option<f64>>>,
        waits: RefCell<Vec<u64>>,
        attention: Cell<usize>,
        reduce: Cell<bool>,
        queue: RefCell<Vec<Box<dyn FnOnce()>>>,
    }

    fn arrival(world: &Rc<World>, enabled: bool) -> Rc<CreditArrival> {
        let (a, b, c, d) = (
            Rc::clone(world),
            Rc::clone(world),
            Rc::clone(world),
            Rc::clone(world),
        );
        CreditArrival::new(
            enabled,
            Hooks {
                reduce_motion: Box::new(move || a.reduce.get()),
                attention: Box::new(move || b.attention.set(b.attention.get() + 1)),
                show: Box::new(move |t| c.shown.borrow_mut().push(t)),
                after: Box::new(move |ms, f| {
                    d.waits.borrow_mut().push(ms);
                    d.queue.borrow_mut().push(f);
                }),
            },
        )
    }

    /// Runs every frame waiting, one at a time, as the timer would.
    fn drain(world: &World, mut between: impl FnMut(usize)) {
        let mut ran = 0;
        loop {
            let next = {
                let mut queue = world.queue.borrow_mut();
                if queue.is_empty() {
                    break;
                }
                queue.remove(0)
            };
            between(ran);
            next();
            ran += 1;
        }
    }

    #[test]
    fn a_whole_credit_draws_41_frames_42_ms_apart_then_rests() {
        let world = Rc::new(World::default());
        let arrival = arrival(&world, true);
        arrival.observe_received(Some(5_000_000_000));
        arrival.observe_credit_final(Some(2.0));
        assert!(
            world.shown.borrow().is_empty(),
            "a baseline celebrates nothing"
        );
        arrival.observe_received(Some(6_000_000_000));
        drain(&world, |_| {});
        let shown = world.shown.borrow();
        assert_eq!(shown.len(), 42);
        for (k, t) in shown[..41].iter().enumerate() {
            assert_eq!(*t, Some(k as f64 / 24.0));
        }
        assert_eq!(shown[41], None);
        assert_eq!(*world.waits.borrow(), vec![42; 41]);
        assert_eq!(world.attention.get(), 1);
        drop(shown);
        // The Trace credit fires on its own.
        world.shown.borrow_mut().clear();
        arrival.observe_credit_final(Some(3.0));
        drain(&world, |_| {});
        assert_eq!(world.shown.borrow().len(), 42);
        assert_eq!(world.attention.get(), 2);
    }

    #[test]
    fn reduced_motion_holds_the_still_frame_and_asks_no_attention() {
        let world = Rc::new(World::default());
        world.reduce.set(true);
        let arrival = arrival(&world, true);
        arrival.celebrate();
        drain(&world, |_| {});
        assert_eq!(*world.shown.borrow(), vec![Some(0.45), None]);
        assert_eq!(*world.waits.borrow(), vec![1600]);
        assert_eq!(world.attention.get(), 0);
    }

    /// Reduced motion turned on mid-way is followed at the next frame.
    #[test]
    fn reduced_motion_is_followed_live() {
        let world = Rc::new(World::default());
        let arrival = arrival(&world, true);
        arrival.celebrate();
        let w = Rc::clone(&world);
        drain(&world, |ran| {
            if ran == 4 {
                w.reduce.set(true);
            }
        });
        let shown = world.shown.borrow();
        assert_eq!(
            shown[..5],
            [0.0, 1.0, 2.0, 3.0, 4.0].map(|k| Some(k / 24.0))
        );
        assert_eq!(shown[5..], [Some(0.45), None]);
        assert_eq!(*world.waits.borrow().last().unwrap(), 1600);
    }

    /// Off, nothing draws; switched off mid-way, the icon rests at once
    /// and no frame follows.
    #[test]
    fn the_switch_gates_the_celebration_and_stops_it() {
        let world = Rc::new(World::default());
        let arrival = arrival(&world, false);
        arrival.celebrate();
        assert_eq!(*world.shown.borrow(), vec![None]);
        assert!(world.queue.borrow().is_empty());
        assert_eq!(world.attention.get(), 0);

        world.shown.borrow_mut().clear();
        arrival.set_enabled(true);
        arrival.celebrate();
        let a = Rc::clone(&arrival);
        drain(&world, |ran| {
            if ran == 3 {
                a.set_enabled(false);
            }
        });
        let shown = world.shown.borrow();
        assert_eq!(shown.last(), Some(&None));
        assert_eq!(shown.len(), 5, "{shown:?}");
    }

    /// A second arrival starts again from the first frame; the first
    /// celebration's pending frame draws nothing.
    #[test]
    fn a_second_arrival_restarts() {
        let world = Rc::new(World::default());
        let arrival = arrival(&world, true);
        arrival.celebrate();
        let a = Rc::clone(&arrival);
        drain(&world, |ran| {
            if ran == 10 {
                a.celebrate();
            }
        });
        let shown = world.shown.borrow();
        // 11 frames of the first, then all 41 of the second, then rest.
        assert_eq!(shown.len(), 11 + 41 + 1);
        assert_eq!(shown[11], Some(0.0));
        assert_eq!(world.attention.get(), 2);
    }

    #[test]
    fn the_plan_is_the_macs_schedule() {
        let moving = plan(false);
        assert_eq!(moving.len(), 42);
        assert_eq!(moving[0], (Some(0.0), 42));
        assert_eq!(moving[41], (None, 0));
        assert_eq!(plan(true), vec![(Some(0.45), 1600), (None, 0)]);
    }
}
