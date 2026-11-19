//! When credit arrives while the app runs, and the schedule the tray
//! icon's celebration keeps.
//!
//! A port of `BalanceArrival.Tracker` and `BalanceArrival.IconAnimation`
//! (`macos/Sources/TCShellCore/BalanceArrival.swift`, #2197). Unlike Home's
//! Balance flourish (`ui::hero_art::arrival`), which persists the figure
//! last shown, the icon celebrates only credit that arrives during this
//! session: the first known reading is the baseline, an unread one keeps
//! it, a smaller one (another organisation) replaces it, and one at least
//! a whole credit above it celebrates and becomes the new baseline. The
//! Trace credit (`credit_final`) is tracked too, so the icon celebrates
//! for someone not signed in to NEAR AI.

/// `BalanceArrival.Tracker`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tracker {
    pub last_received_nanos: Option<i64>,
    pub last_credit_final: Option<f64>,
}

impl Tracker {
    /// `minimumCreditFinal`: one credit.
    pub const MINIMUM_CREDIT_FINAL: f64 = 1.0;
    /// `minimumReceivedNanos`: $1.00.
    pub const MINIMUM_RECEIVED_NANOS: i64 = 1_000_000_000;
    const CREDIT_EPSILON: f64 = 1e-6;

    /// `observe(_:)`: records NEAR AI's received total and says whether a
    /// whole credit or more arrived since the baseline.
    pub fn observe(&mut self, received_total_nanos: Option<i64>) -> bool {
        let Some(now) = received_total_nanos else {
            return false;
        };
        let Some(before) = self.last_received_nanos else {
            self.last_received_nanos = Some(now);
            return false;
        };
        if now < before {
            self.last_received_nanos = Some(now);
            return false;
        }
        if now - before < Self::MINIMUM_RECEIVED_NANOS {
            return false;
        }
        self.last_received_nanos = Some(now);
        true
    }

    /// `observeCreditFinal(_:)`: the same for the settled Trace credit.
    pub fn observe_credit_final(&mut self, credit_final: Option<f64>) -> bool {
        let Some(now) = credit_final.filter(|value| value.is_finite()) else {
            return false;
        };
        let Some(before) = self.last_credit_final else {
            self.last_credit_final = Some(now);
            return false;
        };
        if now < before - Self::CREDIT_EPSILON {
            self.last_credit_final = Some(now);
            return false;
        }
        if now - before < Self::MINIMUM_CREDIT_FINAL - Self::CREDIT_EPSILON {
            return false;
        }
        self.last_credit_final = Some(now);
        true
    }
}

/// `BalanceArrival.IconAnimation`: one schedule for the icon's frames.
pub mod icon_animation {
    /// `GlassCoinBurst.duration`.
    pub const DURATION: f64 = 1.7;
    /// `framesPerSecond`: the icon is a bitmap, redrawn 24 times a second.
    pub const FRAMES_PER_SECOND: f64 = 24.0;
    /// `stillAt`: the one frame drawn under reduced motion.
    pub const STILL_AT: f64 = 0.45;
    /// `reduceMotionHold`: how long that frame stays.
    pub const REDUCE_MOTION_HOLD_MS: u64 = 1600;

    /// `frameInterval`: 1000 / 24, rounded, in milliseconds.
    pub fn frame_interval_ms() -> u64 {
        (1000.0 / FRAMES_PER_SECOND).round() as u64
    }

    /// `frameTimes(reduceMotion:)`: the seconds each frame draws.
    pub fn frame_times(reduce_motion: bool) -> Vec<f64> {
        if reduce_motion {
            return vec![STILL_AT];
        }
        let count = ((DURATION * FRAMES_PER_SECOND).ceil() as usize).max(1);
        (0..count)
            .map(|frame| frame as f64 / FRAMES_PER_SECOND)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOLLAR: i64 = 1_000_000_000;

    /// `testIconArrivalTrackerFiresOnlyOnLiveIncreases`, line for line.
    #[test]
    fn the_tracker_fires_only_on_a_whole_credit_arriving_live() {
        let mut tracker = Tracker::default();
        assert!(!tracker.observe(None));
        assert_eq!(tracker.last_received_nanos, None);
        assert!(!tracker.observe(Some(4 * DOLLAR)));
        assert_eq!(tracker.last_received_nanos, Some(4 * DOLLAR));
        assert!(!tracker.observe(Some(4 * DOLLAR)));
        assert!(!tracker.observe(None));
        assert_eq!(tracker.last_received_nanos, Some(4 * DOLLAR));
        assert!(!tracker.observe(Some(4 * DOLLAR)));
        assert!(!tracker.observe(Some(4 * DOLLAR + 400_000_000)));
        assert_eq!(tracker.last_received_nanos, Some(4 * DOLLAR));
        assert!(!tracker.observe(Some(4 * DOLLAR + 900_000_000)));
        assert_eq!(tracker.last_received_nanos, Some(4 * DOLLAR));
        assert!(tracker.observe(Some(6 * DOLLAR)));
        assert_eq!(tracker.last_received_nanos, Some(6 * DOLLAR));
        assert!(!tracker.observe(Some(6 * DOLLAR)));
        assert!(!tracker.observe(Some(2 * DOLLAR)));
        assert_eq!(tracker.last_received_nanos, Some(2 * DOLLAR));
        assert!(tracker.observe(Some(3 * DOLLAR)));

        assert!(!tracker.observe_credit_final(None));
        assert!(!tracker.observe_credit_final(Some(1.5)));
        assert_eq!(tracker.last_credit_final, Some(1.5));
        assert!(!tracker.observe_credit_final(Some(1.5)));
        assert!(!tracker.observe_credit_final(None));
        assert_eq!(tracker.last_credit_final, Some(1.5));
        assert!(!tracker.observe_credit_final(Some(1.9)));
        assert_eq!(tracker.last_credit_final, Some(1.5));
        assert!(!tracker.observe_credit_final(Some(2.3)));
        assert_eq!(tracker.last_credit_final, Some(1.5));
        assert!(tracker.observe_credit_final(Some(2.5)));
        assert_eq!(tracker.last_credit_final, Some(2.5));
        assert!(!tracker.observe_credit_final(Some(1.0)));
        assert_eq!(tracker.last_credit_final, Some(1.0));
        assert!(!tracker.observe_credit_final(Some(1.5)));
        assert!(tracker.observe_credit_final(Some(2.0)));
        assert!(!tracker.observe_credit_final(Some(f64::NAN)));
    }

    /// The Mac's schedule, from a Swift probe of `frameTimes` and
    /// `frameInterval` (scratchpad motion/probe-surfaces): 41 frames at
    /// k/24 s, 42 ms apart; one frame at 0.45 s under reduced motion.
    #[test]
    fn the_icon_keeps_the_macs_frame_schedule() {
        let frames = icon_animation::frame_times(false);
        assert_eq!(frames.len(), 41);
        assert_eq!(icon_animation::frame_interval_ms(), 42);
        for (k, t) in frames.iter().enumerate() {
            assert!((t - k as f64 / 24.0).abs() < 1e-12);
        }
        assert!(*frames.last().unwrap() < icon_animation::DURATION);
        assert_eq!(icon_animation::frame_times(true), vec![0.45]);
        assert_eq!(icon_animation::REDUCE_MOTION_HOLD_MS, 1600);
    }
}
