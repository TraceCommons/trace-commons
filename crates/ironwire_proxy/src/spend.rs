//! What has been spent today, and whether that is more than the user allows.
//!
//! Two rules shape this, and both are about not lying with a number:
//!
//! 1. **Only metered money counts.** `Exchange::cost_usd` is populated for
//!    subscription exchanges on purpose — "what this would have cost on the
//!    meter" is what makes a subscription legible — but it is not money anyone
//!    was billed. Summing it into a cap would cap capacity the user has already
//!    paid for, within minutes of them setting one.
//! 2. **The request path never queries the ledger.** Spend is accumulated in
//!    memory as exchanges are recorded and seeded once at startup, so a routing
//!    decision costs no SQLite scan.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use ironwire_core::protocol::BackendId;

/// Running spend against the current window.
#[derive(Debug, Default)]
pub struct SpendTracker {
    /// Start of the window this total belongs to — the most recent local
    /// midnight.
    window_started: Option<DateTime<Utc>>,
    by_backend: HashMap<BackendId, f64>,
    /// Backends whose cap has already been announced this window, so the event
    /// fires once rather than on every subsequent request. The bus drops on
    /// lag, and a per-request event would be the thing doing the lagging.
    announced: HashSet<BackendId>,
}

/// The most recent local midnight, as an instant.
///
/// Local rather than UTC because a spend cap is a human's day: someone in
/// Auckland setting a daily cap means their day, and a window rolling over at
/// lunchtime would be indefensible. Falls back to UTC only if the local
/// midnight is ambiguous or nonexistent, which happens for one hour a year at
/// a DST boundary and must not be a panic.
#[must_use]
pub fn window_start(now: DateTime<Utc>) -> DateTime<Utc> {
    use chrono::{Local, TimeZone};
    let local = now.with_timezone(&Local);
    Local
        .from_local_datetime(&local.date_naive().and_hms_opt(0, 0, 0).unwrap_or_default())
        .single()
        .map_or_else(
            || {
                now.date_naive()
                    .and_hms_opt(0, 0, 0)
                    .unwrap_or_default()
                    .and_utc()
            },
            |midnight| midnight.with_timezone(&Utc),
        )
}

impl SpendTracker {
    /// A tracker seeded with what has already been spent in this window.
    #[must_use]
    pub fn seeded(spent: impl IntoIterator<Item = (BackendId, f64)>, now: DateTime<Utc>) -> Self {
        Self {
            window_started: Some(window_start(now)),
            by_backend: spent.into_iter().collect(),
            announced: HashSet::new(),
        }
    }

    /// Roll the window over if the clock has crossed local midnight.
    ///
    /// Checked on read rather than on a timer: a daemon that is idle across
    /// midnight has nothing to do, and a timer would be a second source of
    /// truth about which window we are in.
    fn roll(&mut self, now: DateTime<Utc>) {
        let start = window_start(now);
        if self.window_started != Some(start) {
            self.window_started = Some(start);
            self.by_backend.clear();
            self.announced.clear();
        }
    }

    /// Record what an exchange cost. Metered backends only — the caller knows
    /// which those are.
    pub fn record(&mut self, backend: &BackendId, cost_usd: f64, now: DateTime<Utc>) {
        if !cost_usd.is_finite() || cost_usd <= 0.0 {
            return;
        }
        self.roll(now);
        *self.by_backend.entry(backend.clone()).or_insert(0.0) += cost_usd;
    }

    /// Spend against one backend in the current window.
    pub fn spent(&mut self, backend: &BackendId, now: DateTime<Utc>) -> f64 {
        self.roll(now);
        self.by_backend.get(backend).copied().unwrap_or(0.0)
    }

    /// Spend across every metered backend in the current window.
    pub fn total(&mut self, now: DateTime<Utc>) -> f64 {
        self.roll(now);
        self.by_backend.values().sum()
    }

    /// Whether this backend's breach still needs announcing, marking it
    /// announced. Latched per backend per window.
    pub fn announce_once(&mut self, backend: &BackendId, now: DateTime<Utc>) -> bool {
        self.roll(now);
        self.announced.insert(backend.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000 + secs, 0).expect("valid timestamp")
    }

    fn backend() -> BackendId {
        BackendId::from("anthropic-key")
    }

    #[test]
    fn spend_accumulates_within_a_window() {
        let mut tracker = SpendTracker::seeded([], t(0));
        tracker.record(&backend(), 1.25, t(0));
        tracker.record(&backend(), 0.75, t(60));
        assert!((tracker.spent(&backend(), t(60)) - 2.0).abs() < 1e-9);
        assert!((tracker.total(t(60)) - 2.0).abs() < 1e-9);
    }

    /// A daemon restarted after $8 of a $10 cap resumes at $8, not zero —
    /// otherwise a cap could be reset by restarting, which is the one thing a
    /// cap must not permit.
    #[test]
    fn a_tracker_can_be_seeded_from_the_ledger() {
        let mut tracker = SpendTracker::seeded([(backend(), 8.0)], t(0));
        assert!((tracker.spent(&backend(), t(0)) - 8.0).abs() < 1e-9);
    }

    #[test]
    fn crossing_the_window_boundary_starts_over() {
        let mut tracker = SpendTracker::seeded([(backend(), 8.0)], t(0));
        // A day and a half later is unambiguously a different local day.
        let tomorrow = t(0) + chrono::Duration::hours(36);
        assert!(tracker.spent(&backend(), tomorrow).abs() < 1e-9);
    }

    #[test]
    fn a_breach_is_announced_once_per_window() {
        let mut tracker = SpendTracker::seeded([], t(0));
        assert!(tracker.announce_once(&backend(), t(0)));
        assert!(!tracker.announce_once(&backend(), t(1)));
        // ...and again in the next window, because it is news again.
        assert!(tracker.announce_once(&backend(), t(0) + chrono::Duration::hours(36)));
    }

    /// The provider reports no usage on plenty of exchanges, and a NaN or a
    /// negative would poison a total that gates spending.
    #[test]
    fn a_nonsense_cost_is_ignored_rather_than_accumulated() {
        let mut tracker = SpendTracker::seeded([], t(0));
        tracker.record(&backend(), f64::NAN, t(0));
        tracker.record(&backend(), -5.0, t(0));
        tracker.record(&backend(), f64::INFINITY, t(0));
        assert!(tracker.total(t(0)).abs() < 1e-9);
    }
}
