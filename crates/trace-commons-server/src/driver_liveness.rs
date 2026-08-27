//! Liveness and failure-class tracking for the binary's background driver
//! loops.
//!
//! #438: two drivers were dead in production for days and the only signal was
//! the same hash-only WARN every tick -- 6,999 of them for one of them. A
//! repeating identical warning reads as steady state, and nothing anywhere
//! answered "when did this last work?". This module supplies that answer as
//! state, and a stable failure label to grep for alongside the forensic hash.

use chrono::{DateTime, Utc};
use serde::Serialize;

/// Multiple of a driver's own tick interval before a run of failures counts
/// as "dead" rather than "unlucky".
pub const ESCALATION_INTERVAL_MULTIPLE: u64 = 3;

/// Floor under [`ESCALATION_INTERVAL_MULTIPLE`], so a fast driver does not
/// escalate on a blip. The 45s and 60s drivers both land here.
pub const MIN_ESCALATION_WINDOW_SECONDS: u64 = 300;

/// How often an already-escalated driver re-logs while it stays dead. The
/// point of the fix is that 6,999 identical lines convey less than one line
/// every fifteen minutes carrying a duration and a count.
pub const ESCALATED_REPEAT_INTERVAL_SECONDS: i64 = 900;

/// A short, non-sensitive label for why a driver tick failed.
///
/// Sits ALONGSIDE the error hash, never instead of it: the hash stays for
/// forensics, the label makes the log greppable and comparable across ticks.
/// Neither carries error text, a URL, a vendor response body, or anything
/// about a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DriverFailureClass {
    /// A dependency outside this process was unreachable or erroring: a
    /// transport failure, a timeout, or a 5xx that survived the adapter's
    /// own retries.
    UpstreamUnavailable,
    /// Required configuration or credentials were absent, so the tick refused
    /// before doing any work.
    ConfigMissing,
    /// An in-infrastructure dependency failed: the database mirror, a
    /// connection pool, or object storage.
    DependencyUnavailable,
    /// The upstream was reached and rejected the input. A property of the
    /// content, not of the system's health.
    ContentRejected,
    /// No typed marker matched. Deliberately not a guess.
    Unclassified,
}

impl DriverFailureClass {
    /// Stable snake_case label. Changing one of these breaks an operator's
    /// grep, so treat them as a wire format.
    pub fn as_label(self) -> &'static str {
        match self {
            DriverFailureClass::UpstreamUnavailable => "upstream_unavailable",
            DriverFailureClass::ConfigMissing => "config_missing",
            DriverFailureClass::DependencyUnavailable => "dependency_unavailable",
            DriverFailureClass::ContentRejected => "content_rejected",
            DriverFailureClass::Unclassified => "unclassified",
        }
    }
}

/// The result of one driver tick, already classified and hashed by the
/// caller. Taking the hash as a `String` rather than the error keeps this
/// module free of every error type in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverTickOutcome {
    Success,
    Failure {
        class: DriverFailureClass,
        error_hash: String,
    },
}

/// Everything known about one driver's health.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DriverLiveness {
    pub driver: &'static str,
    pub interval_seconds: u64,
    pub started_at: DateTime<Utc>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub consecutive_failures: u64,
    pub last_failure_class: Option<DriverFailureClass>,
    pub last_error_hash: Option<String>,
    pub escalated: bool,
    pub last_escalated_log_at: Option<DateTime<Utc>>,
    pub suppressed_since_last_log: u64,
}

impl DriverLiveness {
    pub fn new(driver: &'static str, interval_seconds: u64, started_at: DateTime<Utc>) -> Self {
        Self {
            driver,
            interval_seconds,
            started_at,
            last_success_at: None,
            last_failure_at: None,
            consecutive_failures: 0,
            last_failure_class: None,
            last_error_hash: None,
            escalated: false,
            last_escalated_log_at: None,
            suppressed_since_last_log: 0,
        }
    }

    /// How long without a success before this driver counts as dead.
    pub fn stale_after_seconds(&self) -> i64 {
        let scaled = self
            .interval_seconds
            .saturating_mul(ESCALATION_INTERVAL_MULTIPLE);
        scaled
            .max(MIN_ESCALATION_WINDOW_SECONDS)
            .min(i64::MAX as u64) as i64
    }

    /// The instant the staleness clock runs from: the last success, or the
    /// driver's start if it has never succeeded.
    fn reference_instant(&self) -> DateTime<Utc> {
        self.last_success_at.unwrap_or(self.started_at)
    }

    /// Seconds since this driver last worked. Never negative.
    pub fn dead_seconds(&self, now: DateTime<Utc>) -> i64 {
        (now - self.reference_instant()).num_seconds().max(0)
    }

    /// True when the driver has gone longer than its threshold without a
    /// success.
    ///
    /// Deliberately NOT conditioned on `consecutive_failures > 0`. A tokio
    /// task that panicked and died stops ticking entirely: it records no
    /// failures and writes no log line ever again. Measuring from the last
    /// success catches that, and it is the case logs structurally cannot
    /// report.
    pub fn is_stale(&self, now: DateTime<Utc>) -> bool {
        self.dead_seconds(now) > self.stale_after_seconds()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn at(seconds: i64) -> chrono::DateTime<chrono::Utc> {
        Utc.timestamp_opt(1_770_000_000 + seconds, 0).unwrap()
    }

    #[test]
    fn failure_class_labels_are_stable_snake_case() {
        assert_eq!(
            DriverFailureClass::UpstreamUnavailable.as_label(),
            "upstream_unavailable"
        );
        assert_eq!(
            DriverFailureClass::ConfigMissing.as_label(),
            "config_missing"
        );
        assert_eq!(
            DriverFailureClass::DependencyUnavailable.as_label(),
            "dependency_unavailable"
        );
        assert_eq!(
            DriverFailureClass::ContentRejected.as_label(),
            "content_rejected"
        );
        assert_eq!(DriverFailureClass::Unclassified.as_label(), "unclassified");
    }

    /// The threshold scales with the driver's own interval, because the twelve
    /// loops differ by an order of magnitude and are operator-configurable
    /// from 5 seconds to 86,400.
    #[test]
    fn stale_threshold_scales_with_interval_above_the_floor() {
        // 45s and 60s drivers sit under the floor, so they get the floor.
        assert_eq!(
            DriverLiveness::new("a", 45, at(0)).stale_after_seconds(),
            300
        );
        assert_eq!(
            DriverLiveness::new("b", 60, at(0)).stale_after_seconds(),
            300
        );
        // 300s driver: 3 x 300 = 900 clears the floor.
        assert_eq!(
            DriverLiveness::new("c", 300, at(0)).stale_after_seconds(),
            900
        );
        // An hourly driver scales rather than using a flat constant.
        assert_eq!(
            DriverLiveness::new("d", 3_600, at(0)).stale_after_seconds(),
            10_800
        );
    }

    /// Before any success, staleness is measured from driver start. This is
    /// what catches a tokio task that died outright: it never ticks, so it
    /// never logs anything at all, and only state can reveal it.
    #[test]
    fn a_driver_that_never_ticks_goes_stale_from_its_start_time() {
        let live = DriverLiveness::new("never", 45, at(0));
        assert!(!live.is_stale(at(299)));
        assert!(live.is_stale(at(301)));
        assert_eq!(live.dead_seconds(at(301)), 301);
    }

    #[test]
    fn a_success_resets_the_staleness_clock() {
        let mut live = DriverLiveness::new("ok", 45, at(0));
        live.last_success_at = Some(at(280));
        assert!(!live.is_stale(at(500)));
        assert!(live.is_stale(at(600)));
    }
}
