# Driver Liveness and Failure-Class Observability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a dead background driver visible in one query and one greppable
log label, instead of 6,999 identical hash-only warnings that read as steady
state.

**Architecture:** A new dependency-free lib module,
`crates/trace-commons-server/src/driver_liveness.rs`, holds the types, the
staleness predicate, the escalation state machine as a pure function, and an
in-memory registry. The binary adds a `classify_driver_failure` helper, hangs
the registry on `AppState`, replaces twelve hand-rolled `loop { sleep; tick;
warn }` bodies with one shared wrapper, and exposes an admin-gated read route.

**Tech Stack:** Rust 2024, axum, chrono, serde, tokio. No new dependencies.

## Global Constraints

- **No new dependencies.** Everything used here is already in
  `crates/trace-commons-server/Cargo.toml`.
- **Hash-only logging.** Log fields carry a `sha256:`-prefixed hash and a
  stable label only. Never error text, URLs, vendor response bodies,
  contributor identity, or trace content.
- **Must compile under all three CI feature configs:** default,
  `near-ai-scorer`, and `local-gpu-models` (non-CUDA). `ScorerFailure` and
  `TraceContributionError` are both unconditionally available, so the
  classifier is safe in all three.
- **Verify with warnings-as-errors.** Plain `cargo check` does not apply
  `-D warnings`; CI does. Use
  `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` and
  `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`.
- **Run `cargo fmt --all` before every commit.** The repo is not
  rustfmt-clean, and a post-edit hook can turn a one-line edit into a
  whole-file diff. Check `git show --stat` after committing.
- **No emojis** in commits, PRs, code, or comments. Short imperative commit
  subjects, no `feat:` / `fix:` prefixes.
- **Never `cd` out of the worktree.** All commands run from the worktree root.

---

### Task 1: Core types and the staleness predicate

**Files:**
- Create: `crates/trace-commons-server/src/driver_liveness.rs`
- Modify: `crates/trace-commons-server/src/lib.rs` (add `pub mod driver_liveness;`)

**Interfaces:**
- Consumes: nothing.
- Produces: `DriverFailureClass` (with `as_label`), `DriverTickOutcome`,
  `DriverLiveness` (with `new`, `stale_after_seconds`, `dead_seconds`,
  `is_stale`), and the three timing constants.

- [ ] **Step 1: Write the failing test**

Create `crates/trace-commons-server/src/driver_liveness.rs` containing ONLY
the test module for now:

```rust
//! Liveness and failure-class tracking for the binary's background driver
//! loops.
//!
//! #438: two drivers were dead in production for days and the only signal was
//! the same hash-only WARN every tick -- 6,999 of them for one of them. A
//! repeating identical warning reads as steady state, and nothing anywhere
//! answered "when did this last work?". This module supplies that answer as
//! state, and a stable failure label to grep for alongside the forensic hash.

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
        assert_eq!(DriverFailureClass::ConfigMissing.as_label(), "config_missing");
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
        assert_eq!(DriverLiveness::new("a", 45, at(0)).stale_after_seconds(), 300);
        assert_eq!(DriverLiveness::new("b", 60, at(0)).stale_after_seconds(), 300);
        // 300s driver: 3 x 300 = 900 clears the floor.
        assert_eq!(DriverLiveness::new("c", 300, at(0)).stale_after_seconds(), 900);
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p trace-commons-server --lib driver_liveness`

Expected: FAIL to compile — `cannot find type DriverLiveness in this scope`,
`cannot find type DriverFailureClass in this scope`. (The module is not yet
declared either, so also add the `lib.rs` line in Step 3.)

- [ ] **Step 3: Write minimal implementation**

Add `pub mod driver_liveness;` to `crates/trace-commons-server/src/lib.rs`,
keeping the list alphabetical — it goes between `pub mod dedup_simhash;` and
`pub mod error;`.

Then prepend to `driver_liveness.rs`, above the test module:

```rust
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
        scaled.max(MIN_ESCALATION_WINDOW_SECONDS).min(i64::MAX as u64) as i64
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p trace-commons-server --lib driver_liveness`

Expected: PASS, 4 tests.

- [ ] **Step 5: Verify with warnings-as-errors, format, and commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
git add crates/trace-commons-server/src/driver_liveness.rs crates/trace-commons-server/src/lib.rs
git commit -m "Add driver liveness types and a staleness predicate"
git show --stat HEAD
```

Confirm `git show --stat` lists exactly two files. If rustfmt rewrote
unrelated files, unstage them and recommit.

---

### Task 2: The escalation state machine

**Files:**
- Modify: `crates/trace-commons-server/src/driver_liveness.rs`

**Interfaces:**
- Consumes: `DriverLiveness`, `DriverTickOutcome`, `DriverFailureClass`,
  `ESCALATED_REPEAT_INTERVAL_SECONDS` from Task 1.
- Produces: `LogAction` and
  `observe_driver_tick(&DriverLiveness, DriverTickOutcome, DateTime<Utc>) -> (DriverLiveness, LogAction)`.

- [ ] **Step 1: Write the failing test**

Add inside the existing `mod tests` block in `driver_liveness.rs`:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p trace-commons-server --lib driver_liveness`

Expected: FAIL to compile — `cannot find function observe_driver_tick`,
`cannot find type LogAction`.

- [ ] **Step 3: Write minimal implementation**

Add to `driver_liveness.rs`, above the test module:

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p trace-commons-server --lib driver_liveness`

Expected: PASS, 8 tests.

If `five_days_of_failure_does_not_produce_one_line_per_tick` fails on the
`repeats` count, do NOT change the assertion to match the output. Print the
actual value, work out from the timing constants which number is correct, and
fix whichever side is genuinely wrong. An assertion edited to match observed
behaviour pins the bug.

- [ ] **Step 5: Format, verify, commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
git add crates/trace-commons-server/src/driver_liveness.rs
git commit -m "Escalate a driver failure on persistence, not on occurrence"
git show --stat HEAD
```

---

### Task 3: The registry

**Files:**
- Modify: `crates/trace-commons-server/src/driver_liveness.rs`

**Interfaces:**
- Consumes: everything from Tasks 1 and 2.
- Produces: `DriverLivenessRegistry` with `register(&self, &'static str, u64, DateTime<Utc>)`,
  `observe(&self, &'static str, DriverTickOutcome, DateTime<Utc>) -> LogAction`,
  and `snapshot(&self, DateTime<Utc>) -> Vec<DriverLivenessSnapshot>`;
  plus the `DriverLivenessSnapshot` serializable view.

- [ ] **Step 1: Write the failing test**

Add inside `mod tests`:

```rust
    #[test]
    fn the_registry_tracks_drivers_independently() {
        let registry = DriverLivenessRegistry::default();
        registry.register("alpha", 45, at(0));
        registry.register("beta", 45, at(0));

        registry.observe("alpha", fail("sha256:abc"), at(45));
        registry.observe("beta", DriverTickOutcome::Success, at(45));

        let snapshot = registry.snapshot(at(60));
        let alpha = snapshot.iter().find(|d| d.driver == "alpha").expect("alpha");
        let beta = snapshot.iter().find(|d| d.driver == "beta").expect("beta");

        assert_eq!(alpha.consecutive_failures, 1);
        assert_eq!(alpha.last_failure_class, Some(DriverFailureClass::UpstreamUnavailable));
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p trace-commons-server --lib driver_liveness`

Expected: FAIL to compile — `cannot find type DriverLivenessRegistry`.

- [ ] **Step 3: Write minimal implementation**

Add to `driver_liveness.rs`, above the test module. Note the two extra `use`
lines go at the top of the file with the existing imports:

```rust
use std::collections::BTreeMap;
use std::sync::Mutex;
```

```rust
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
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p trace-commons-server --lib driver_liveness`

Expected: PASS, 12 tests.

- [ ] **Step 5: Format, verify, commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
git add crates/trace-commons-server/src/driver_liveness.rs
git commit -m "Track driver liveness in a process-global registry"
git show --stat HEAD
```

---

### Task 4: Classify failures, wire the registry, convert the two incident drivers

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
  - `AppState` struct at `:1369`
  - both `AppState` construction sites (find with `grep -n "driver_liveness\|root:" ...`; see Step 3)
  - `spawn_perplexity_score_driver_task` at `:9194`
  - `spawn_pii_backstop_driver_task` at `:9242`
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`

**Interfaces:**
- Consumes: `DriverLivenessRegistry`, `DriverTickOutcome`, `DriverFailureClass`,
  `LogAction` from Tasks 1-3.
- Produces: `classify_driver_failure(&anyhow::Error) -> DriverFailureClass`,
  `emit_driver_log_action(driver: &'static str, action: LogAction)`,
  `spawn_driver_loop(&Arc<AppState>, &'static str, StdDuration, F)` where
  `F: Fn(Arc<AppState>) -> Fut + Send + Sync + 'static` and
  `Fut: Future<Output = anyhow::Result<()>> + Send`, and the driver-name
  constants `PERPLEXITY_SCORE_DRIVER_NAME` / `PII_BACKSTOP_DRIVER_NAME`.

**Note on duplication:** the liveness bookkeeping must exist exactly once, in
`spawn_driver_loop`. Task 5 converts ten more loops through the same helper;
if this task leaves the register/observe/emit sequence inline at a call site,
Task 5 will multiply it by ten.

- [ ] **Step 1: Write the failing test**

Append to `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`:

```rust
/// #438: the failure label must be read off the error's TYPE, never parsed
/// from its message, matching the discipline the per-driver transient
/// classifiers already use.
#[test]
fn driver_failures_classify_from_typed_markers_not_message_text() {
    use trace_commons_server::driver_liveness::DriverFailureClass;

    let transient_scorer = anyhow::Error::new(
        trace_commons_gate_enclave::ScorerFailure::TransientScorerFailed {
            reason: "upstream_unavailable".to_string(),
        },
    );
    assert_eq!(
        classify_driver_failure(&transient_scorer),
        DriverFailureClass::UpstreamUnavailable
    );

    // Context layers must not hide the marker: anyhow preserves the concrete
    // type underneath, which is what the existing downcasts rely on.
    let wrapped = transient_scorer.context("PerplexityScorerInferenceFailed");
    assert_eq!(
        classify_driver_failure(&wrapped),
        DriverFailureClass::UpstreamUnavailable
    );

    // The permanent variant is the content's fault, not the system's.
    let permanent_scorer = anyhow::Error::new(
        trace_commons_gate_enclave::ScorerFailure::ScorerFailed {
            reason: "prompt_too_long".to_string(),
        },
    );
    assert_eq!(
        classify_driver_failure(&permanent_scorer),
        DriverFailureClass::ContentRejected
    );

    // An unrecognised error is Unclassified, never a guess. The message here
    // deliberately contains the words of another label, to prove nothing is
    // parsed out of message text.
    let opaque = anyhow::anyhow!("something went wrong upstream and unavailable");
    assert_eq!(
        classify_driver_failure(&opaque),
        DriverFailureClass::Unclassified,
        "classification must not be inferred from message text"
    );
}
```

`ScorerFailure` has exactly two variants, both struct-form with a single
`reason: String` field: `ScorerFailed` (permanent) and
`TransientScorerFailed` (transient), with `is_transient()` true only for the
latter (`crates/trace-commons-gate-api/src/perplexity.rs:130-148`). The test
above uses those real shapes.


- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest driver_failures_classify`

Expected: FAIL to compile — `cannot find function classify_driver_failure`.

- [ ] **Step 3: Write minimal implementation**

**3a.** Add the import near the other `trace_commons_server::` uses at the top
of `trace-commons-ingest.rs`:

```rust
use trace_commons_server::driver_liveness::{
    DriverFailureClass, DriverLivenessRegistry, DriverTickOutcome, LogAction,
};
```

**3b.** Add the classifier next to the existing `is_transient_gate_scoring_failure`
at `:39297`:

```rust
/// Map a driver tick error to a short, non-sensitive label for the log.
///
/// #438: a hash alone is a fine forensic tool and a terrible triage signal --
/// recovering what `sha256:37769fdd...` meant took reading the code that
/// built the string and probing the live endpoint. A stable class alongside
/// it costs nothing and leaks nothing.
///
/// Classification reads the error's TYPE. It never inspects the message: a
/// message is not a contract, and matching on one would resurrect exactly the
/// coupling the hash-only convention exists to avoid.
fn classify_driver_failure(error: &anyhow::Error) -> DriverFailureClass {
    if let Some(failure) = error.downcast_ref::<trace_commons_gate_enclave::ScorerFailure>() {
        return if failure.is_transient() {
            DriverFailureClass::UpstreamUnavailable
        } else {
            DriverFailureClass::ContentRejected
        };
    }
    if let Some(failure) =
        error.downcast_ref::<trace_commons_protocol::trace_contribution::TraceContributionError>()
    {
        return if failure.is_transient() {
            DriverFailureClass::UpstreamUnavailable
        } else {
            DriverFailureClass::ContentRejected
        };
    }
    DriverFailureClass::Unclassified
}
```

**3c.** Add the log emitter beside it:

```rust
/// Execute the decision `observe_driver_tick` returned.
///
/// Every field here is hash-only or label-only.
fn emit_driver_log_action(driver: &'static str, action: LogAction) {
    match action {
        LogAction::None => {}
        LogAction::Warn { class, error_hash } => {
            tracing::warn!(
                driver,
                failure_class = class.as_label(),
                error_hash = %error_hash,
                "Trace Commons driver tick failed"
            );
        }
        LogAction::Escalate {
            class,
            error_hash,
            consecutive_failures,
            dead_seconds,
        } => {
            tracing::error!(
                driver,
                failure_class = class.as_label(),
                error_hash = %error_hash,
                consecutive_failures,
                dead_seconds,
                "Trace Commons driver is not working and has not been for some time"
            );
        }
        LogAction::EscalateRepeat {
            class,
            error_hash,
            consecutive_failures,
            dead_seconds,
            suppressed,
        } => {
            tracing::error!(
                driver,
                failure_class = class.as_label(),
                error_hash = %error_hash,
                consecutive_failures,
                dead_seconds,
                suppressed,
                "Trace Commons driver is still not working"
            );
        }
        LogAction::Recovered {
            failures,
            dead_seconds,
        } => {
            tracing::info!(
                driver,
                recovered_after_failures = failures,
                dead_seconds,
                "Trace Commons driver recovered"
            );
        }
    }
}
```

**3d.** Add the field to `AppState` at `:1369`, after `root: PathBuf,`:

```rust
    driver_liveness: Arc<DriverLivenessRegistry>,
```

Then find every construction site and add
`driver_liveness: Arc::new(DriverLivenessRegistry::default()),`:

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins 2>&1 | grep -A3 "missing field"
```

The compiler names each one. Add the field to each until the build is clean.
Test helpers that build an `AppState` need it too.

**3e.** Add the driver-name constants beside the spawn functions:

```rust
const PERPLEXITY_SCORE_DRIVER_NAME: &str = "perplexity_score_driver";
const PII_BACKSTOP_DRIVER_NAME: &str = "pii_backstop_driver";
```

**3f.** Add the shared loop wrapper. This is the point of the task: the
sleep/register/observe/emit logic exists ONCE, and each driver supplies only
what genuinely differs -- its own tick call and its own success log line.

```rust
/// Run `tick` forever on `interval`, recording liveness and emitting the
/// escalation decision.
///
/// #438: twelve loops each hand-rolled `loop { sleep; match tick { info!,
/// warn! } }`, so every one of them had the same blind spot and the two that
/// happened to point at a vendor that ran out of credit were dead for days
/// behind nothing louder than a repeating WARN. The liveness bookkeeping
/// lives here so a thirteenth driver cannot be added without it.
///
/// `tick` returns `Result<()>` and owns its own success logging, because the
/// summary fields differ per driver and are worth keeping verbatim. Only the
/// failure path is shared -- that is the part that was uniform and broken.
fn spawn_driver_loop<F, Fut>(
    state: &Arc<AppState>,
    driver: &'static str,
    interval: StdDuration,
    tick: F,
) where
    F: Fn(Arc<AppState>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = anyhow::Result<()>> + Send,
{
    state
        .driver_liveness
        .register(driver, interval.as_secs(), Utc::now());
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            let outcome = match tick(state.clone()).await {
                Ok(()) => DriverTickOutcome::Success,
                Err(error) => DriverTickOutcome::Failure {
                    class: classify_driver_failure(&error),
                    error_hash: safe_display_error_hash(&error),
                },
            };
            let action = state.driver_liveness.observe(driver, outcome, Utc::now());
            emit_driver_log_action(driver, action);
        }
    });
}
```

If `StdDuration` is not already the alias in scope for `std::time::Duration`
in this file, use whatever alias the neighbouring scheduler config structs
use — grep for `interval:` on one of the `*SchedulerConfig` structs.

**3g.** Convert `spawn_perplexity_score_driver_task`. Keep its existing
`tracing::info!` enable line untouched; replace the whole `tokio::spawn`
block with:

```rust
    let tick_config = config.clone();
    spawn_driver_loop(
        state,
        PERPLEXITY_SCORE_DRIVER_NAME,
        config.interval,
        move |state| {
            let config = tick_config.clone();
            async move {
                let summary = run_perplexity_score_driver_tick(state, &config).await?;
                tracing::info!(
                    scored = summary.scored,
                    skipped_duplicate = summary.skipped_duplicate,
                    cached = summary.cached,
                    failed = summary.failed,
                    transient = summary.transient,
                    breaker_tripped = summary.breaker_tripped,
                    "Trace Commons perplexity score driver tick completed"
                );
                Ok(())
            }
        },
    );
```

`PerplexityScoreDriverConfig` may not derive `Clone`. If it does not, add
`#[derive(Clone)]` to it, or wrap it in an `Arc` before the closure — pick
whichever the surrounding code already does for shared config. Do not change
its fields.

**3h.** Convert `spawn_pii_backstop_driver_task` the same way, using
`PII_BACKSTOP_DRIVER_NAME` and keeping its own success fields (`done`,
`failed`, `transient`, `exhausted`, `breaker_tripped`).


- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p trace-commons-server --bin trace-commons-ingest driver_failures_classify
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
```

Expected: the classification test passes and the whole crate still builds
warnings-clean.

- [ ] **Step 5: Format, verify, commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
git add crates/trace-commons-server/src/bin/trace-commons-ingest.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs
git commit -m "Give the two drivers that died a liveness record and a failure class"
git show --stat HEAD
```

---

### Task 5: Convert the remaining ten loops

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
  (spawn functions whose warnings are at `:9050`, `:9095`, `:9140`, `:9181`,
  `:9316`, `:9365`, `:9417`, `:9465`, `:9513`, `:9555` — line numbers will
  have shifted after Task 4; re-locate with the grep in Step 1)
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`

**Interfaces:**
- Consumes: `spawn_driver_loop`, `classify_driver_failure`,
  `emit_driver_log_action` from Task 4.
- Produces: ten more driver-name constants and a
  `ALL_DRIVER_NAMES: &[&str]` slice for the collision test.

- [ ] **Step 1: Locate every remaining loop**

```bash
grep -n 'tick failed"' crates/trace-commons-server/src/bin/trace-commons-ingest.rs
```

Expected: 10 remaining (the two from Task 4 are gone). Names, in file order:
export job scheduler, NEAR credit outbox scheduler, retention maintenance
scheduler, vector index scheduler, benchmark registry scheduler, benchmark
pipeline scheduler, credit cycle scheduler, credit settlement scheduler,
process evaluation scheduler, revocation propagation scheduler.

- [ ] **Step 2: Write the failing test**

Append to `trace_commons_ingest_internal/tests.rs`:

```rust
/// #438: the shared wrapper keys liveness by name. Two drivers sharing a name
/// would silently report one driver's health for the other -- a worse failure
/// than the one this work fixes, because it would look like it was working.
#[test]
fn every_driver_registers_a_distinct_name() {
    let mut seen = std::collections::BTreeSet::new();
    for name in ALL_DRIVER_NAMES {
        assert!(
            seen.insert(*name),
            "duplicate driver name {name}; liveness would be reported for the wrong driver"
        );
    }
    assert_eq!(
        seen.len(),
        12,
        "every spawned driver loop must register; got {seen:?}"
    );
}

/// The names are an operator-facing grep target and an admin API field, so
/// they are a wire format.
#[test]
fn driver_names_are_snake_case_and_stable() {
    for name in ALL_DRIVER_NAMES {
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "driver name {name} must be snake_case"
        );
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest every_driver_registers`

Expected: FAIL to compile — `cannot find value ALL_DRIVER_NAMES`.

- [ ] **Step 4: Write minimal implementation**

Add the ten constants beside the two from Task 4, then the slice:

```rust
const EXPORT_JOB_SCHEDULER_DRIVER_NAME: &str = "export_job_scheduler";
const NEAR_CREDIT_OUTBOX_SCHEDULER_DRIVER_NAME: &str = "near_credit_outbox_scheduler";
const RETENTION_MAINTENANCE_SCHEDULER_DRIVER_NAME: &str = "retention_maintenance_scheduler";
const VECTOR_INDEX_SCHEDULER_DRIVER_NAME: &str = "vector_index_scheduler";
const BENCHMARK_REGISTRY_SCHEDULER_DRIVER_NAME: &str = "benchmark_registry_scheduler";
const BENCHMARK_PIPELINE_SCHEDULER_DRIVER_NAME: &str = "benchmark_pipeline_scheduler";
const CREDIT_CYCLE_SCHEDULER_DRIVER_NAME: &str = "credit_cycle_scheduler";
const CREDIT_SETTLEMENT_SCHEDULER_DRIVER_NAME: &str = "credit_settlement_scheduler";
const PROCESS_EVALUATION_SCHEDULER_DRIVER_NAME: &str = "process_evaluation_scheduler";
const REVOCATION_PROPAGATION_SCHEDULER_DRIVER_NAME: &str = "revocation_propagation_scheduler";

/// Every driver the liveness registry knows about. The distinctness test
/// reads this; keep it in sync when adding a driver.
const ALL_DRIVER_NAMES: &[&str] = &[
    PERPLEXITY_SCORE_DRIVER_NAME,
    PII_BACKSTOP_DRIVER_NAME,
    EXPORT_JOB_SCHEDULER_DRIVER_NAME,
    NEAR_CREDIT_OUTBOX_SCHEDULER_DRIVER_NAME,
    RETENTION_MAINTENANCE_SCHEDULER_DRIVER_NAME,
    VECTOR_INDEX_SCHEDULER_DRIVER_NAME,
    BENCHMARK_REGISTRY_SCHEDULER_DRIVER_NAME,
    BENCHMARK_PIPELINE_SCHEDULER_DRIVER_NAME,
    CREDIT_CYCLE_SCHEDULER_DRIVER_NAME,
    CREDIT_SETTLEMENT_SCHEDULER_DRIVER_NAME,
    PROCESS_EVALUATION_SCHEDULER_DRIVER_NAME,
    REVOCATION_PROPAGATION_SCHEDULER_DRIVER_NAME,
];
```

Convert each of the ten loops with `spawn_driver_loop` from Task 4 Step 3f.
The whole point is that the liveness bookkeeping is NOT repeated: each call
site supplies only the driver name, the interval, and a closure that runs its
own tick and logs its own success line with its own summary fields. If you
find yourself writing `register` / `observe` / `emit_driver_log_action` at a
call site, you are duplicating the wrapper — call it instead.

Keep each loop's existing enable-time `tracing::info!` and its success-path
fields exactly as they are. The tick-completed lines are useful and are not
what this issue is about.

Two things differ per scheduler and must be read from the code rather than
assumed:

- **The interval field.** Several schedulers do not use `config.interval`.
  Read each config struct before writing the call.
- **Config cloning.** The closure must own its config. If a config type does
  not derive `Clone`, add `#[derive(Clone)]` or wrap in `Arc`, matching what
  the surrounding code already does. Do not change any config field.


- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p trace-commons-server --bin trace-commons-ingest every_driver_registers
cargo test -p trace-commons-server --bin trace-commons-ingest driver_names_are_snake_case
grep -c 'tick failed"' crates/trace-commons-server/src/bin/trace-commons-ingest.rs
```

Expected: both tests PASS, and the grep returns **0** — every hand-rolled
warning is now routed through the wrapper.

- [ ] **Step 6: Format, verify, commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
git add crates/trace-commons-server/src/bin/trace-commons-ingest.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs
git commit -m "Route every background driver through the liveness wrapper"
git show --stat HEAD
```

---

### Task 6: The admin route and the runbook

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
  (router in `fn app` at `:7203`; handler beside
  `rollout_smoke_evidence_handler` at `:40510`)
- Create: `docs/operator/driver-liveness.md`
- Modify: `docs/operator/README.md` (index row and alphabetical entry)
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`

**Interfaces:**
- Consumes: `state.driver_liveness`, `DriverLivenessSnapshot`.
- Produces: `GET /v1/admin/driver-liveness` returning
  `Json<Vec<DriverLivenessSnapshot>>`.

- [ ] **Step 1: Write the failing test**

Append to `trace_commons_ingest_internal/tests.rs`. `test_state` already
registers `admin-token-a` as `TokenRole::Admin` and `token-a` as a
contributor (`insert_token` calls around `:4614`), and `auth_headers` is the
existing helper for building the header map.

```rust
/// #438: liveness answers "when did this last work?" in one read. It is
/// admin-gated on purpose -- see the companion test below for why.
#[tokio::test]
async fn driver_liveness_reports_registered_drivers_to_an_admin() {
    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state(temp.path().to_path_buf());
    state
        .driver_liveness
        .register("example_driver", 45, Utc::now());

    let Json(drivers) = driver_liveness_handler(
        State(state.clone()),
        auth_headers("admin-token-a"),
    )
    .await
    .expect("admin may read driver liveness");

    assert_eq!(drivers.len(), 1);
    assert_eq!(drivers[0].driver, "example_driver");
    assert_eq!(drivers[0].consecutive_failures, 0);
    assert_eq!(drivers[0].stale_after_seconds, 300);
    assert!(!drivers[0].stale, "a just-registered driver is not stale");
}

/// A contributor must not learn which subsystem is currently dead. That is
/// operational intelligence: it says when the PII backstop is not running.
#[tokio::test]
async fn driver_liveness_is_refused_to_a_non_admin() {
    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state(temp.path().to_path_buf());
    state
        .driver_liveness
        .register("example_driver", 45, Utc::now());

    let error = driver_liveness_handler(State(state), auth_headers("token-a"))
        .await
        .expect_err("a contributor token must be refused");
    assert_eq!(error.0, StatusCode::FORBIDDEN);
}

/// The route is actually registered, not merely a reachable function.
#[tokio::test]
async fn driver_liveness_route_is_wired_and_requires_auth() {
    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state(temp.path().to_path_buf());

    let anonymous = app(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/admin/driver-liveness")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("anonymous response");
    assert_eq!(
        anonymous.status(),
        StatusCode::UNAUTHORIZED,
        "the route must exist and refuse an unauthenticated caller"
    );
}

/// #438 proposed putting liveness on `/health`. It is unauthenticated, so
/// this test exists to stop that being "fixed" back later.
#[tokio::test]
async fn health_does_not_expose_driver_liveness() {
    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state(temp.path().to_path_buf());
    state
        .driver_liveness
        .register("example_driver", 45, Utc::now());

    let response = app(state)
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("health response");
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body reads");
    let text = String::from_utf8(body.to_vec()).expect("health body is utf8");
    assert!(
        !text.contains("example_driver") && !text.contains("driver"),
        "unauthenticated /health must not name a driver: {text}"
    );
}
```

If `error.0` is not how this codebase's `ApiResult` error exposes its status,
match the shape the neighbouring `expect_err` admin tests use rather than
inventing one:

```bash
grep -n "expect_err" crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs | head -5
```


- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest driver_liveness`

Expected: FAIL — `404 Not Found`, because the route does not exist yet.

- [ ] **Step 3: Write minimal implementation**

Add the handler beside `rollout_smoke_evidence_handler`:

```rust
/// #438: "last successful tick at T" is a single value that makes a five-day
/// outage obvious at a glance; 6,999 warnings do not.
///
/// Admin-gated rather than on `/health`, which is unauthenticated: which
/// subsystem is currently dead, and for how long, is operational intelligence
/// -- it tells an anonymous caller when the PII backstop is not running.
///
/// Deliberately does no DB read, no tenant scoping, and no audit write.
/// Driver health is process-global and tenant-independent, and this should
/// stay cheap enough to poll.
async fn driver_liveness_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<trace_commons_server::driver_liveness::DriverLivenessSnapshot>>> {
    let tenant = authenticate_ctx_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(tenant.auth())?;
    Ok(Json(state.driver_liveness.snapshot(Utc::now())))
}
```

Register it in `fn app`, next to the other `/v1/admin/*` routes:

```rust
        .route("/v1/admin/driver-liveness", get(driver_liveness_handler))
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p trace-commons-server --bin trace-commons-ingest driver_liveness
```

Expected: PASS.

- [ ] **Step 5: Write the runbook**

Create `docs/operator/driver-liveness.md` covering:

- What the endpoint is and how to call it (admin EdDSA-signed JWT, as the
  pilot refuses static tokens).
- The response fields, and specifically that `stale` is derived at read time,
  so a driver whose tokio task died outright still reports `stale: true`
  despite writing no log line.
- The failure-class labels and what each one means for triage:
  `upstream_unavailable` (the vendor or a remote dependency),
  `config_missing` (this deployment), `dependency_unavailable` (DB, pool,
  object store), `content_rejected` (the input), `unclassified` (no typed
  marker matched — treat as unknown, read the hash).
- The escalation model: `max(3 x interval, 5 minutes)` without a success, and
  one ERROR per fifteen minutes thereafter rather than one per tick.
- A worked example: what the 2026-08-26 NEAR AI credit outage would have
  looked like — `stale: true`, `failure_class: upstream_unavailable`,
  `dead_seconds` in the hundreds of thousands, `consecutive_failures` near
  7,000.
- An explicit note that **nothing alerts on this yet**: pilot logs go to
  `/var/log/tracecommons/ingest.log`, outside the journal, so this endpoint
  is a thing to check, not a thing that pages you.

Add to `docs/operator/README.md`: a Quick-links row
`| Checking whether a background driver is alive | ./driver-liveness.md |`
and an entry in the alphabetical reference.

- [ ] **Step 6: Full verification and commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
git add crates/trace-commons-server/src/bin/trace-commons-ingest.rs \
        crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs \
        docs/operator/driver-liveness.md docs/operator/README.md
git commit -m "Expose driver liveness on an admin route"
git show --stat HEAD
```

---

## Final verification before opening a PR

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --features near-ai-scorer --bins
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --features local-gpu-models --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server
cargo fmt --all -- --check
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
grep -c 'tick failed"' crates/trace-commons-server/src/bin/trace-commons-ingest.rs   # must be 0
```

The `local-gpu-models` check may fail to LINK locally without a CUDA
toolchain; `cargo check` should still succeed, and that is the CI job's
requirement. If it fails to check (not link), the change is not
feature-safe and must be fixed before the PR.

Capture a baseline test count before starting Task 1 and compare at the end.
Report the real numbers, not "tests pass".
