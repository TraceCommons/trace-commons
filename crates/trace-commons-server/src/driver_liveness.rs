// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Liveness and failure-class tracking for the binary's background driver
//! loops.
//!
//! #438: two drivers were dead in production for days and the only signal was
//! the same hash-only WARN every tick -- 6,999 of them for one of them. A
//! repeating identical warning reads as steady state, and nothing anywhere
//! answered "when did this last work?". This module supplies that answer as
//! state, and a stable failure label to grep for alongside the forensic hash.

use std::collections::BTreeMap;
use std::sync::Mutex;

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

/// What the caller should log for a tick. Returned rather than logged here so
/// every rule below is testable without a subscriber, a spawned task, or a
/// clock trait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogAction {
    /// Nothing to say: a healthy tick, or a failure already covered by a
    /// standing escalation.
    None,
    /// First failure of a run, or the failure class changed.
    Warn {
        class: DriverFailureClass,
        error_hash: String,
    },
    /// This driver just crossed its staleness threshold.
    Escalate {
        class: DriverFailureClass,
        error_hash: String,
        consecutive_failures: u64,
        dead_seconds: i64,
    },
    /// Still dead, and the repeat interval has elapsed since the last line.
    EscalateRepeat {
        class: DriverFailureClass,
        error_hash: String,
        consecutive_failures: u64,
        dead_seconds: i64,
        suppressed: u64,
    },
    /// First success after one or more failures. The incident had no
    /// equivalent: its end was as invisible as its start.
    Recovered { failures: u64, dead_seconds: i64 },
}

/// Fold one tick outcome into a driver's liveness, and decide what to log.
///
/// Pure: same inputs, same outputs, no clock and no I/O. `now` is a
/// parameter precisely so the five-day scenario in the tests runs instantly.
pub fn observe_driver_tick(
    prev: &DriverLiveness,
    outcome: DriverTickOutcome,
    now: DateTime<Utc>,
) -> (DriverLiveness, LogAction) {
    let mut next = prev.clone();

    match outcome {
        DriverTickOutcome::Success => {
            let failures = prev.consecutive_failures;
            let dead_seconds = prev.dead_seconds(now);
            next.last_success_at = Some(now);
            next.consecutive_failures = 0;
            next.escalated = false;
            next.last_escalated_log_at = None;
            next.suppressed_since_last_log = 0;
            let action = if failures > 0 {
                LogAction::Recovered {
                    failures,
                    dead_seconds,
                }
            } else {
                LogAction::None
            };
            (next, action)
        }
        DriverTickOutcome::Failure { class, error_hash } => {
            let class_changed = prev.last_failure_class != Some(class);
            next.last_failure_at = Some(now);
            next.consecutive_failures = prev.consecutive_failures.saturating_add(1);
            next.last_failure_class = Some(class);
            next.last_error_hash = Some(error_hash.clone());

            let dead_seconds = next.dead_seconds(now);
            let stale = dead_seconds > next.stale_after_seconds();

            if !stale {
                // Below the threshold this is "unlucky", not "dead". Say so
                // once per run, and again if the reason changes.
                if prev.consecutive_failures == 0 || class_changed {
                    (next, LogAction::Warn { class, error_hash })
                } else {
                    (next, LogAction::None)
                }
            } else if !prev.escalated {
                next.escalated = true;
                next.last_escalated_log_at = Some(now);
                next.suppressed_since_last_log = 0;
                let consecutive_failures = next.consecutive_failures;
                (
                    next,
                    LogAction::Escalate {
                        class,
                        error_hash,
                        consecutive_failures,
                        dead_seconds,
                    },
                )
            } else {
                let since_last_log = next
                    .last_escalated_log_at
                    .map(|at| (now - at).num_seconds())
                    .unwrap_or(i64::MAX);
                if since_last_log >= ESCALATED_REPEAT_INTERVAL_SECONDS {
                    let suppressed = next.suppressed_since_last_log;
                    next.last_escalated_log_at = Some(now);
                    next.suppressed_since_last_log = 0;
                    let consecutive_failures = next.consecutive_failures;
                    (
                        next,
                        LogAction::EscalateRepeat {
                            class,
                            error_hash,
                            consecutive_failures,
                            dead_seconds,
                            suppressed,
                        },
                    )
                } else {
                    next.suppressed_since_last_log =
                        next.suppressed_since_last_log.saturating_add(1);
                    (next, LogAction::None)
                }
            }
        }
    }
}

/// A read-time view of one driver's health, with `stale` derived rather than
/// stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DriverLivenessSnapshot {
    pub driver: &'static str,
    pub interval_seconds: u64,
    pub started_at: DateTime<Utc>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub consecutive_failures: u64,
    pub last_failure_class: Option<DriverFailureClass>,
    pub last_error_hash: Option<String>,
    /// Seconds since this driver last worked.
    pub dead_seconds: i64,
    /// Threshold this driver is measured against, so a reader can tell why
    /// `stale` is what it is without knowing the constants.
    pub stale_after_seconds: i64,
    pub stale: bool,
}

/// Process-global driver health, keyed by driver name.
///
/// In memory deliberately. A persisted `last_success_at` would survive a
/// restart, and would then be actively misleading: after a restart the
/// process does not know whether the driver works, and `None` says so.
#[derive(Debug, Default)]
pub struct DriverLivenessRegistry {
    inner: Mutex<BTreeMap<&'static str, DriverLiveness>>,
}

impl DriverLivenessRegistry {
    /// Record a driver as running. Called once per driver at spawn, before
    /// its first tick.
    pub fn register(&self, driver: &'static str, interval_seconds: u64, now: DateTime<Utc>) {
        let mut inner = self.lock();
        inner.insert(driver, DriverLiveness::new(driver, interval_seconds, now));
    }

    /// Fold a tick outcome into the named driver and return what to log.
    ///
    /// An unregistered name is inert rather than a panic: a driver that never
    /// registered is a wiring bug, and taking down a production tick loop
    /// over it would be a worse outcome than the missing telemetry.
    pub fn observe(
        &self,
        driver: &'static str,
        outcome: DriverTickOutcome,
        now: DateTime<Utc>,
    ) -> LogAction {
        let mut inner = self.lock();
        let Some(prev) = inner.get(driver) else {
            return LogAction::None;
        };
        let (next, action) = observe_driver_tick(prev, outcome, now);
        inner.insert(driver, next);
        action
    }

    /// Every driver's health, ordered by name.
    pub fn snapshot(&self, now: DateTime<Utc>) -> Vec<DriverLivenessSnapshot> {
        let inner = self.lock();
        inner
            .values()
            .map(|live| DriverLivenessSnapshot {
                driver: live.driver,
                interval_seconds: live.interval_seconds,
                started_at: live.started_at,
                last_success_at: live.last_success_at,
                last_failure_at: live.last_failure_at,
                consecutive_failures: live.consecutive_failures,
                last_failure_class: live.last_failure_class,
                last_error_hash: live.last_error_hash.clone(),
                dead_seconds: live.dead_seconds(now),
                stale_after_seconds: live.stale_after_seconds(),
                stale: live.is_stale(now),
            })
            .collect()
    }

    /// Recover rather than propagate a poisoned lock. The guarded data is
    /// telemetry; a panic in one tick must not disable liveness reporting for
    /// every other driver.
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<&'static str, DriverLiveness>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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

    fn fail(hash: &str) -> DriverTickOutcome {
        DriverTickOutcome::Failure {
            class: DriverFailureClass::UpstreamUnavailable,
            error_hash: hash.to_string(),
        }
    }

    /// The incident, simulated. A 45s driver failing every tick for five days
    /// produced 6,999 identical WARNs. It must now produce one WARN, one
    /// escalation, and then one line per repeat interval -- not one per tick.
    #[test]
    fn five_days_of_failure_does_not_produce_one_line_per_tick() {
        let mut live = DriverLiveness::new("pii_backstop", 45, at(0));
        let mut warns = 0;
        let mut escalations = 0;
        let mut repeats = 0;
        let mut silent = 0;

        // 5 days at one tick per 45s.
        let ticks = 5 * 24 * 60 * 60 / 45;
        for tick in 1..=ticks {
            let now = at(tick * 45);
            let (next, action) = observe_driver_tick(&live, fail("sha256:abc"), now);
            live = next;
            match action {
                LogAction::Warn { .. } => warns += 1,
                LogAction::Escalate { .. } => escalations += 1,
                LogAction::EscalateRepeat { .. } => repeats += 1,
                LogAction::None => silent += 1,
                LogAction::Recovered { .. } => unreachable!("no success was fed"),
            }
        }

        assert_eq!(warns, 1, "only the first failure warns");
        assert_eq!(escalations, 1, "escalation happens once, not per tick");
        // 5 days / 15 min, minus the interval already covered by the initial
        // escalation line.
        assert_eq!(repeats, 479);
        assert_eq!(warns + escalations + repeats, 481);
        assert!(
            silent > 9_000,
            "the rest are suppressed, got {silent} suppressed of {ticks} ticks"
        );
    }

    /// Escalation must mean "dead", not "unreliable". A driver that fails and
    /// recovers repeatedly never crosses the threshold, because each success
    /// resets the clock.
    #[test]
    fn a_flapping_driver_never_escalates() {
        let mut live = DriverLiveness::new("flappy", 45, at(0));
        for tick in 0..200 {
            let now = at(tick * 45);
            let outcome = if tick % 2 == 0 {
                fail("sha256:abc")
            } else {
                DriverTickOutcome::Success
            };
            let (next, action) = observe_driver_tick(&live, outcome, now);
            live = next;
            assert!(
                !matches!(
                    action,
                    LogAction::Escalate { .. } | LogAction::EscalateRepeat { .. }
                ),
                "flapping escalated at tick {tick}"
            );
            assert!(!live.escalated);
        }
    }

    #[test]
    fn recovery_emits_exactly_one_line_carrying_the_real_duration() {
        let mut live = DriverLiveness::new("recovering", 45, at(0));
        for tick in 1..=40 {
            let (next, _) = observe_driver_tick(&live, fail("sha256:abc"), at(tick * 45));
            live = next;
        }
        assert!(live.escalated);

        let (recovered, action) = observe_driver_tick(&live, DriverTickOutcome::Success, at(1_845));
        assert_eq!(
            action,
            LogAction::Recovered {
                failures: 40,
                dead_seconds: 1_845,
            }
        );
        assert_eq!(recovered.consecutive_failures, 0);
        assert!(!recovered.escalated);
        assert_eq!(recovered.last_success_at, Some(at(1_845)));

        // A second success is silent -- recovery is an edge, not a state.
        let (_, action) = observe_driver_tick(&recovered, DriverTickOutcome::Success, at(1_890));
        assert_eq!(action, LogAction::None);
    }

    /// A different failure class is news even mid-outage: a 402 becoming a
    /// 500 means something changed. Suppression must not mask it.
    #[test]
    fn a_changed_failure_class_warns_even_while_suppressed() {
        let mut live = DriverLiveness::new("changing", 45, at(0));
        let (next, action) = observe_driver_tick(&live, fail("sha256:abc"), at(45));
        live = next;
        assert!(matches!(action, LogAction::Warn { .. }));

        // Same class, below threshold: suppressed.
        let (next, action) = observe_driver_tick(&live, fail("sha256:abc"), at(90));
        live = next;
        assert_eq!(action, LogAction::None);

        // Different class, still below threshold: warns.
        let (_, action) = observe_driver_tick(
            &live,
            DriverTickOutcome::Failure {
                class: DriverFailureClass::ConfigMissing,
                error_hash: "sha256:def".to_string(),
            },
            at(135),
        );
        assert!(
            matches!(
                action,
                LogAction::Warn {
                    class: DriverFailureClass::ConfigMissing,
                    ..
                }
            ),
            "a class change must not be suppressed, got {action:?}"
        );
    }

    #[test]
    fn the_registry_tracks_drivers_independently() {
        let registry = DriverLivenessRegistry::default();
        registry.register("alpha", 45, at(0));
        registry.register("beta", 45, at(0));

        registry.observe("alpha", fail("sha256:abc"), at(45));
        registry.observe("beta", DriverTickOutcome::Success, at(45));

        let snapshot = registry.snapshot(at(60));
        let alpha = snapshot
            .iter()
            .find(|d| d.driver == "alpha")
            .expect("alpha");
        let beta = snapshot.iter().find(|d| d.driver == "beta").expect("beta");

        assert_eq!(alpha.consecutive_failures, 1);
        assert_eq!(
            alpha.last_failure_class,
            Some(DriverFailureClass::UpstreamUnavailable)
        );
        assert_eq!(beta.consecutive_failures, 0);
        assert_eq!(beta.last_success_at, Some(at(45)));
    }

    /// A snapshot derives `stale` at read time rather than storing it, so a
    /// driver that has stopped ticking entirely still reports correctly --
    /// nothing has run to update a stored flag.
    #[test]
    fn snapshot_derives_staleness_at_read_time() {
        let registry = DriverLivenessRegistry::default();
        registry.register("silent", 45, at(0));

        assert!(!registry.snapshot(at(100))[0].stale);
        assert!(registry.snapshot(at(400))[0].stale);
        assert_eq!(registry.snapshot(at(400))[0].dead_seconds, 400);
    }

    /// Observing an unregistered driver must not panic or poison the lock.
    #[test]
    fn observing_an_unknown_driver_is_inert() {
        let registry = DriverLivenessRegistry::default();
        assert_eq!(
            registry.observe("ghost", fail("sha256:abc"), at(45)),
            LogAction::None
        );
        assert!(registry.snapshot(at(45)).is_empty());
    }

    #[test]
    fn snapshot_is_ordered_by_driver_name() {
        let registry = DriverLivenessRegistry::default();
        registry.register("zulu", 45, at(0));
        registry.register("alpha", 45, at(0));
        let names: Vec<&str> = registry.snapshot(at(0)).iter().map(|d| d.driver).collect();
        assert_eq!(names, vec!["alpha", "zulu"]);
    }
}
