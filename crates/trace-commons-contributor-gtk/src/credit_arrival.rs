//! When credit arrives, and the schedule the tray icon's celebration keeps.
//!
//! A port of `BalanceArrival.Tracker` and `BalanceArrival.IconAnimation`
//! (`macos/Sources/TCShellCore/BalanceArrival.swift`). Every real increase
//! celebrates. The baseline is saved per account, so credit that arrived
//! while the app was closed or signed out plays once on the next reading.
//! An account's first reading only sets its baseline, an unread one keeps
//! it, and a smaller one (another organisation) replaces it. The Trace
//! credit (`credit_final`) is tracked too, so the icon celebrates for
//! someone not signed in to NEAR AI.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// `BalanceArrival.Tracker`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tracker {
    pub last_received_nanos: Option<i64>,
    pub last_credit_final: Option<f64>,
}

/// The saved baseline: the account's hashed scope and its two totals.
#[derive(Debug, Deserialize, Serialize)]
struct SavedBaseline {
    scope: String,
    received_nanos: Option<i64>,
    credit_final: Option<f64>,
}

impl Tracker {
    /// `creditNoiseFloor`: the daemon sums single-precision values, so
    /// re-reading an unchanged total can differ in its last digits.
    pub const CREDIT_NOISE_FLOOR: f64 = 0.001;

    /// `stored(forScope:in:)`: the baseline saved for `scope`, or an empty
    /// tracker with no scope, no file, or another account's baseline.
    pub fn stored(path: Option<&Path>, scope: Option<&str>) -> Self {
        let (Some(path), Some(scope)) = (path, scope) else {
            return Self::default();
        };
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<SavedBaseline>(&bytes).ok())
            .filter(|saved| saved.scope == scope)
            .map_or_else(Self::default, |saved| Self {
                last_received_nanos: saved.received_nanos,
                last_credit_final: saved.credit_final.filter(|value| value.is_finite()),
            })
    }

    /// `save(forScope:in:)`: best effort, and nothing without a scope.
    pub fn save(&self, path: Option<&Path>, scope: Option<&str>) {
        let (Some(path), Some(scope)) = (path, scope) else {
            return;
        };
        let saved = SavedBaseline {
            scope: scope.to_string(),
            received_nanos: self.last_received_nanos,
            credit_final: self.last_credit_final,
        };
        let Ok(bytes) = serde_json::to_vec(&saved) else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let temp = path.with_extension("tmp");
        if std::fs::write(&temp, bytes).is_ok() {
            let _ = std::fs::rename(&temp, path);
        }
    }

    /// `observe(_:)`: records NEAR AI's received total and says whether it
    /// rose.
    pub fn observe(&mut self, received_total_nanos: Option<i64>) -> bool {
        let Some(now) = received_total_nanos else {
            return false;
        };
        let Some(before) = self.last_received_nanos else {
            self.last_received_nanos = Some(now);
            return false;
        };
        if now == before {
            return false;
        }
        self.last_received_nanos = Some(now);
        now > before
    }

    /// `observeCreditFinal(_:)`: the same for the settled Trace credit,
    /// above rounding noise.
    pub fn observe_credit_final(&mut self, credit_final: Option<f64>) -> bool {
        let Some(now) = credit_final.filter(|value| value.is_finite()) else {
            return false;
        };
        let Some(before) = self.last_credit_final else {
            self.last_credit_final = Some(now);
            return false;
        };
        if (now - before).abs() < Self::CREDIT_NOISE_FLOOR {
            return false;
        }
        self.last_credit_final = Some(now);
        now > before
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

    /// `testIconArrivalTrackerFiresOnAnyRealIncrease`, line for line.
    #[test]
    fn the_tracker_fires_on_any_real_increase() {
        let mut tracker = Tracker::default();
        assert!(!tracker.observe(None));
        assert_eq!(tracker.last_received_nanos, None);
        assert!(!tracker.observe(Some(4 * DOLLAR)));
        assert_eq!(tracker.last_received_nanos, Some(4 * DOLLAR));
        assert!(!tracker.observe(Some(4 * DOLLAR)));
        assert!(!tracker.observe(None));
        assert_eq!(tracker.last_received_nanos, Some(4 * DOLLAR));
        assert!(tracker.observe(Some(4 * DOLLAR + 1)));
        assert_eq!(tracker.last_received_nanos, Some(4 * DOLLAR + 1));
        assert!(!tracker.observe(Some(4 * DOLLAR + 1)));
        assert!(!tracker.observe(Some(2 * DOLLAR)));
        assert_eq!(tracker.last_received_nanos, Some(2 * DOLLAR));
        assert!(tracker.observe(Some(3 * DOLLAR)));

        assert!(!tracker.observe_credit_final(None));
        assert!(!tracker.observe_credit_final(Some(1.5)));
        assert_eq!(tracker.last_credit_final, Some(1.5));
        assert!(!tracker.observe_credit_final(Some(1.5)));
        assert!(!tracker.observe_credit_final(None));
        assert!(tracker.observe_credit_final(Some(1.6)));
        assert_eq!(tracker.last_credit_final, Some(1.6));
        assert!(!tracker.observe_credit_final(Some(1.0)));
        assert_eq!(tracker.last_credit_final, Some(1.0));
        assert!(tracker.observe_credit_final(Some(1.1)));
        assert!(!tracker.observe_credit_final(Some(f64::NAN)));
    }

    /// The daemon sums single-precision credit: a re-read is not an arrival.
    #[test]
    fn floating_point_noise_is_not_an_arrival() {
        let mut tracker = Tracker::default();
        assert!(!tracker.observe_credit_final(Some(40.77798080444336)));
        assert!(!tracker.observe_credit_final(Some(40.77798080444336 + 0.00001)));
        assert_eq!(tracker.last_credit_final, Some(40.77798080444336));
    }

    fn scratch_file(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tc-credit-arrival-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("baseline.json")
    }

    /// Credit that lands while away plays once on the next reading.
    #[test]
    fn a_saved_baseline_celebrates_what_arrived_while_away() {
        let path = scratch_file("away");
        let mut before = Tracker::default();
        assert!(!before.observe(Some(4 * DOLLAR)));
        assert!(!before.observe_credit_final(Some(10.0)));
        before.save(Some(&path), Some("sha256:a"));

        let mut relaunched = Tracker::stored(Some(&path), Some("sha256:a"));
        assert_eq!(relaunched, before);
        assert!(relaunched.observe(Some(5 * DOLLAR)));
        assert!(relaunched.observe_credit_final(Some(10.1)));
        relaunched.save(Some(&path), Some("sha256:a"));

        let mut again = Tracker::stored(Some(&path), Some("sha256:a"));
        assert!(!again.observe(Some(5 * DOLLAR)));
        assert!(!again.observe_credit_final(Some(10.1)));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Another account's larger total is not new credit, and with no
    /// account nothing is saved.
    #[test]
    fn a_saved_baseline_belongs_to_its_account() {
        let path = scratch_file("account");
        let mut first = Tracker::default();
        assert!(!first.observe(Some(DOLLAR)));
        first.save(Some(&path), Some("sha256:a"));

        let mut other = Tracker::stored(Some(&path), Some("sha256:b"));
        assert_eq!(other, Tracker::default());
        assert!(!other.observe(Some(9 * DOLLAR)));
        assert_eq!(Tracker::stored(Some(&path), None), Tracker::default());

        let unsaved = scratch_file("none");
        first.save(Some(&unsaved), None);
        assert!(!unsaved.exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
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
