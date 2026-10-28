//! The held subscription usage reading and its wire form.
//!
//! One login in this slice: the contributor's default Codex sign-in. The
//! reading is replaced only by a probe the contributor asked for with
//! Refresh; nothing here polls. `subscription_usage` answers from what is
//! held, synchronously, and never waits on a probe.
//!
//! Every way of not knowing is a [`Reason`] label, never a number: a window
//! the provider did not report is absent, and a failed refresh keeps the
//! previous reading marked stale with the failure's reason.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use super::UsageSource;

/// The shortest time between two probes that launched Codex. A refresh
/// inside it answers the held snapshot and launches nothing.
pub const PROBE_FLOOR: Duration = Duration::from_secs(60);

/// Why a login has no current reading. The wire labels are the contract in
/// `docs/contributor-daemon-ipc-v1_1.md`; no reason carries upstream text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The provider's switch is off. Not reached while the whole surface is
    /// off, which answers `enabled: false`; kept for the contract.
    Off,
    /// The switch is on and the contributor has not chosen Refresh yet.
    NeverRefreshed,
    /// No `codex` executable where the daemon looks for one.
    CodexNotFound,
    /// A Codex process was already running, so none was launched.
    CodexRunning,
    /// Codex reported no sign-in.
    SignedOut,
    /// Codex is signed in with something other than ChatGPT (an API key).
    NotSubscription,
    /// A budget ran out.
    Timeout,
    /// An answer did not parse, failed a check, or passed the line cap.
    Unreadable,
    /// macOS refused to launch the executable.
    Blocked,
    /// Anything else: the launch failed, Codex exited, or it refused the
    /// usage read for a reason it did not name.
    Failed,
}

impl Reason {
    /// The wire label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::NeverRefreshed => "never_refreshed",
            Self::CodexNotFound => "codex_not_found",
            Self::CodexRunning => "codex_running",
            Self::SignedOut => "signed_out",
            Self::NotSubscription => "not_subscription",
            Self::Timeout => "timeout",
            Self::Unreadable => "unreadable",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
        }
    }
}

/// The ChatGPT plan, as a fixed label. Display names live in the copy table
/// (`subscription_usage_plan_*`); a value outside this list is `Unknown`,
/// never free text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Free,
    Plus,
    Pro,
    Team,
    Business,
    Enterprise,
    Edu,
    Unknown,
}

impl Plan {
    /// Codex's `PlanType`, pinned against `codex app-server
    /// generate-json-schema` from codex-cli 0.153.4. Variants of one plan
    /// family (`self_serve_business_*`, `enterprise_*`, `edu_*`) read as that
    /// family. `go`, `prolite` and `ent26` are plans this table has no name
    /// for, and read as `Unknown` rather than as the nearest name, which
    /// would state a plan the contributor does not have.
    #[must_use]
    pub fn from_codex(value: &str) -> Self {
        match value {
            "free" => Self::Free,
            "plus" => Self::Plus,
            "pro" => Self::Pro,
            "team" => Self::Team,
            "business" => Self::Business,
            "enterprise" => Self::Enterprise,
            "edu" => Self::Edu,
            v if v.starts_with("self_serve_business_") => Self::Business,
            v if v.starts_with("enterprise_") => Self::Enterprise,
            v if v.starts_with("edu_") => Self::Edu,
            _ => Self::Unknown,
        }
    }

    /// The wire label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Plus => "plus",
            Self::Pro => "pro",
            Self::Team => "team",
            Self::Business => "business",
            Self::Enterprise => "enterprise",
            Self::Edu => "edu",
            Self::Unknown => "unknown",
        }
    }
}

/// Which limit a window measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// The short window (five hours, when Codex says so).
    Session,
    /// The weekly window.
    Weekly,
    /// A limit on one model, named by its checked label.
    Model,
}

impl Lane {
    /// The wire label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Weekly => "weekly",
            Self::Model => "model",
        }
    }
}

/// One reported usage window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub lane: Lane,
    /// A model limit's checked name; `None` for session and weekly windows.
    pub label: Option<String>,
    pub duration_mins: Option<u32>,
    /// 0..=1000.
    pub used_permille: u16,
    pub resets_at: Option<DateTime<Utc>>,
}

/// Credits on the plan, as Codex reports them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credits {
    pub has_credits: bool,
    pub unlimited: bool,
    /// A non-negative decimal, or `None`.
    pub balance: Option<String>,
}

/// One successful probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub at: DateTime<Utc>,
    pub plan: Plan,
    pub limit_reached: Option<bool>,
    pub windows: Vec<Window>,
    pub credits: Option<Credits>,
    pub reset_credits: Option<u32>,
}

#[derive(Default)]
struct Held {
    /// The last good reading, kept across a failed refresh.
    reading: Option<Reading>,
    /// The last refresh's failure, cleared by the next good reading.
    failure: Option<Reason>,
    refreshing: bool,
    /// When a probe last launched Codex. A refresh that stopped before the
    /// launch (`codex_running`, `codex_not_found`) does not set it, so the
    /// contributor who closes Codex can choose Refresh again at once.
    last_launch: Option<Instant>,
    /// Bumped by [`SubscriptionUsage::clear`], so a probe that was running
    /// when the switch went off cannot put its reading back.
    generation: u64,
}

/// What [`SubscriptionUsage::refresh`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheduled {
    Started,
    AlreadyRefreshing,
    /// Inside [`PROBE_FLOOR`] of the last launch.
    Floor,
}

/// The daemon's held snapshot and the one probe that may be running.
pub struct SubscriptionUsage {
    held: Mutex<Held>,
    /// Signalled whenever a probe finishes.
    finished: Condvar,
    source: Mutex<Arc<dyn UsageSource>>,
    /// Called once a probe's answer is held: the daemon publishes
    /// `status_changed`, and the shell re-reads `subscription_usage`.
    on_finish: Box<dyn Fn() + Send + Sync>,
}

impl SubscriptionUsage {
    #[must_use]
    pub fn new(source: Arc<dyn UsageSource>, on_finish: Box<dyn Fn() + Send + Sync>) -> Self {
        Self {
            held: Mutex::new(Held::default()),
            finished: Condvar::new(),
            source: Mutex::new(source),
            on_finish,
        }
    }

    /// Replace the source the next probe uses. Tests swap in fakes here.
    pub fn set_source(&self, source: Arc<dyn UsageSource>) {
        if let Ok(mut slot) = self.source.lock() {
            *slot = source;
        }
    }

    /// Schedule one probe on its own thread, unless one is running or the
    /// last launch is inside the floor. Never waits for it.
    pub fn refresh(self: &Arc<Self>) -> Scheduled {
        let generation = {
            let Ok(mut held) = self.held.lock() else {
                return Scheduled::AlreadyRefreshing;
            };
            if held.refreshing {
                return Scheduled::AlreadyRefreshing;
            }
            if held
                .last_launch
                .is_some_and(|at| at.elapsed() < PROBE_FLOOR)
            {
                return Scheduled::Floor;
            }
            held.refreshing = true;
            held.generation
        };
        let source = self.source.lock().map(|slot| Arc::clone(&slot)).ok();
        let this = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("subscription-usage".to_string())
            .spawn(move || {
                let outcome = match source {
                    Some(source) => this.probe(source.as_ref(), generation),
                    None => Err(Reason::Failed),
                };
                this.finish(generation, outcome);
            });
        if spawned.is_err() {
            self.finish(generation, Err(Reason::Failed));
        }
        Scheduled::Started
    }

    fn probe(&self, source: &dyn UsageSource, generation: u64) -> Result<Reading, Reason> {
        source.ready()?;
        // The switch may have gone off while `ready` looked for Codex: then
        // nothing is launched, and `finish` discards this outcome.
        {
            let Ok(mut held) = self.held.lock() else {
                return Err(Reason::Failed);
            };
            if held.generation != generation {
                return Err(Reason::Off);
            }
            held.last_launch = Some(Instant::now());
        }
        source.probe(Utc::now())
    }

    fn finish(&self, generation: u64, outcome: Result<Reading, Reason>) {
        if let Ok(mut held) = self.held.lock() {
            held.refreshing = false;
            if held.generation == generation {
                match outcome {
                    Ok(reading) => {
                        held.reading = Some(reading);
                        held.failure = None;
                    }
                    Err(reason) => held.failure = Some(reason),
                }
            }
        }
        self.finished.notify_all();
        (self.on_finish)();
    }

    /// Forget the reading, the failure and the floor: the switch went off.
    /// A probe still running finishes into nothing.
    pub fn clear(&self) {
        if let Ok(mut held) = self.held.lock() {
            let generation = held.generation.wrapping_add(1);
            let refreshing = held.refreshing;
            *held = Held {
                refreshing,
                generation,
                ..Held::default()
            };
        }
    }

    /// Wait until no probe is running, for at most `timeout`. Whether it
    /// finished.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let Ok(held) = self.held.lock() else {
            return false;
        };
        self.finished
            .wait_timeout_while(held, timeout, |held| held.refreshing)
            .is_ok_and(|(held, _)| !held.refreshing)
    }

    /// The `subscription_usage` answer while the switch is on.
    #[must_use]
    pub fn value(&self) -> serde_json::Value {
        let login = match self.held.lock() {
            Ok(held) => login_value(held.reading.as_ref(), held.failure, held.refreshing),
            // A poisoned lock reads as unknown: fail closed, never a guess.
            Err(_) => login_value(None, Some(Reason::Failed), false),
        };
        serde_json::json!({ "enabled": true, "logins": [login] })
    }
}

/// One `logins[]` entry.
pub(crate) fn login_value(
    reading: Option<&Reading>,
    failure: Option<Reason>,
    refreshing: bool,
) -> serde_json::Value {
    let Some(reading) = reading else {
        return serde_json::json!({
            "provider": "codex",
            "state": "unknown",
            "reason": failure.unwrap_or(Reason::NeverRefreshed).as_str(),
            "refreshing": refreshing,
            "source": null,
            "updated_at": null,
            "stale": false,
            "plan": null,
            "limit_reached": null,
            "windows": [],
            "credits": null,
            "reset_credits": null,
        });
    };
    // A window whose own reset has passed stays in the answer, so the shell
    // can say "Usage unknown" for it (design 2.6) rather than draw nothing;
    // the shell never shows its figure once `resets_at` is past.
    let windows: Vec<serde_json::Value> = reading
        .windows
        .iter()
        .map(|window| {
            serde_json::json!({
                "lane": window.lane.as_str(),
                "label": window.label,
                "duration_mins": window.duration_mins,
                "used_permille": window.used_permille,
                "resets_at": window.resets_at.map(|at| at.to_rfc3339()),
            })
        })
        .collect();
    serde_json::json!({
        "provider": "codex",
        "state": "observed",
        "reason": failure.map(Reason::as_str),
        "refreshing": refreshing,
        "source": "cli",
        "updated_at": reading.at.to_rfc3339(),
        "stale": failure.is_some(),
        "plan": reading.plan.as_str(),
        "limit_reached": reading.limit_reached,
        "windows": windows,
        "credits": reading.credits.as_ref().map(|credits| serde_json::json!({
            "has_credits": credits.has_credits,
            "unlimited": credits.unlimited,
            "balance": credits.balance,
        })),
        "reset_credits": reading.reset_credits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 26, 12, 0, 0).unwrap()
    }

    fn reading() -> Reading {
        Reading {
            at: now(),
            plan: Plan::Plus,
            limit_reached: Some(false),
            windows: vec![
                Window {
                    lane: Lane::Session,
                    label: None,
                    duration_mins: Some(300),
                    used_permille: 420,
                    resets_at: Some(now() + chrono::Duration::hours(2)),
                },
                Window {
                    lane: Lane::Weekly,
                    label: None,
                    duration_mins: Some(10_080),
                    used_permille: 130,
                    resets_at: None,
                },
            ],
            credits: Some(Credits {
                has_credits: true,
                unlimited: false,
                balance: Some("12.50".to_string()),
            }),
            reset_credits: Some(1),
        }
    }

    /// A source that answers what it is told and counts its launches.
    struct Fake {
        ready: Result<(), Reason>,
        outcome: Result<Reading, Reason>,
        launches: AtomicUsize,
    }

    impl UsageSource for Fake {
        fn ready(&self) -> Result<(), Reason> {
            self.ready
        }

        fn probe(&self, _now: DateTime<Utc>) -> Result<Reading, Reason> {
            self.launches.fetch_add(1, Ordering::SeqCst);
            self.outcome.clone()
        }
    }

    fn fake(ready: Result<(), Reason>, outcome: Result<Reading, Reason>) -> Arc<Fake> {
        Arc::new(Fake {
            ready,
            outcome,
            launches: AtomicUsize::new(0),
        })
    }

    fn holder(source: Arc<Fake>) -> (Arc<SubscriptionUsage>, Arc<AtomicUsize>) {
        let finishes = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&finishes);
        let holder = Arc::new(SubscriptionUsage::new(
            source,
            Box::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }),
        ));
        (holder, finishes)
    }

    fn login(holder: &SubscriptionUsage) -> serde_json::Value {
        holder.value()["logins"][0].clone()
    }

    #[test]
    fn before_any_refresh_the_login_is_unknown_and_never_refreshed() {
        let (holder, _) = holder(fake(Ok(()), Ok(reading())));
        assert_eq!(
            holder.value(),
            serde_json::json!({"enabled": true, "logins": [{
                "provider": "codex",
                "state": "unknown",
                "reason": "never_refreshed",
                "refreshing": false,
                "source": null,
                "updated_at": null,
                "stale": false,
                "plan": null,
                "limit_reached": null,
                "windows": [],
                "credits": null,
                "reset_credits": null,
            }]})
        );
    }

    #[test]
    fn a_good_probe_is_observed_and_announced() {
        let source = fake(Ok(()), Ok(reading()));
        let (holder, finishes) = holder(Arc::clone(&source));
        assert_eq!(holder.refresh(), Scheduled::Started);
        assert!(holder.wait_idle(Duration::from_secs(5)));
        let login = login(&holder);
        assert_eq!(login["state"], "observed");
        assert_eq!(login["reason"], serde_json::Value::Null);
        assert_eq!(login["stale"], false);
        assert_eq!(login["source"], "cli");
        assert_eq!(login["plan"], "plus");
        assert_eq!(login["updated_at"], now().to_rfc3339());
        assert_eq!(login["windows"].as_array().unwrap().len(), 2);
        assert_eq!(login["windows"][0]["used_permille"], 420);
        assert_eq!(login["windows"][1]["resets_at"], serde_json::Value::Null);
        assert_eq!(
            login["credits"],
            serde_json::json!({"has_credits": true, "unlimited": false, "balance": "12.50"})
        );
        assert_eq!(login["reset_credits"], 1);
        assert_eq!(source.launches.load(Ordering::SeqCst), 1);
        assert_eq!(finishes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_second_refresh_inside_the_floor_launches_nothing() {
        let source = fake(Ok(()), Ok(reading()));
        let (holder, _) = holder(Arc::clone(&source));
        assert_eq!(holder.refresh(), Scheduled::Started);
        assert!(holder.wait_idle(Duration::from_secs(5)));
        assert_eq!(holder.refresh(), Scheduled::Floor);
        assert!(holder.wait_idle(Duration::from_secs(5)));
        assert_eq!(source.launches.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn codex_running_skips_the_launch_and_does_not_start_the_floor() {
        let source = fake(Err(Reason::CodexRunning), Ok(reading()));
        let (holder, _) = holder(Arc::clone(&source));
        assert_eq!(holder.refresh(), Scheduled::Started);
        assert!(holder.wait_idle(Duration::from_secs(5)));
        let skipped = login(&holder);
        assert_eq!(skipped["state"], "unknown");
        assert_eq!(skipped["reason"], "codex_running");
        assert_eq!(source.launches.load(Ordering::SeqCst), 0);
        // Codex closed: Refresh works again at once.
        holder.set_source(fake(Ok(()), Ok(reading())));
        assert_eq!(holder.refresh(), Scheduled::Started);
        assert!(holder.wait_idle(Duration::from_secs(5)));
        assert_eq!(login(&holder)["state"], "observed");
    }

    #[test]
    fn a_failed_refresh_keeps_the_reading_stale_with_the_reason() {
        let (holder, _) = holder(fake(Ok(()), Ok(reading())));
        assert_eq!(holder.refresh(), Scheduled::Started);
        assert!(holder.wait_idle(Duration::from_secs(5)));
        holder.set_source(fake(Ok(()), Err(Reason::Timeout)));
        holder.held.lock().unwrap().last_launch = None;
        assert_eq!(holder.refresh(), Scheduled::Started);
        assert!(holder.wait_idle(Duration::from_secs(5)));
        let login = login(&holder);
        assert_eq!(login["state"], "observed");
        assert_eq!(login["stale"], true);
        assert_eq!(login["reason"], "timeout");
        assert_eq!(login["plan"], "plus");
        assert_eq!(login["windows"][0]["used_permille"], 420);
    }

    #[test]
    fn a_window_past_its_reset_stays_with_its_reset_time() {
        // Read three hours on: the session window has reset since.
        let value = login_value(Some(&reading()), None, false);
        let windows = value["windows"].as_array().unwrap();
        assert_eq!(windows.len(), 2, "the shell says Usage unknown for it");
        assert_eq!(windows[0]["lane"], "session");
        assert_eq!(
            windows[0]["resets_at"],
            (now() + chrono::Duration::hours(2)).to_rfc3339()
        );
    }

    /// A source whose `ready` runs a hook first, so a test can turn the
    /// switch off between the checks and the launch.
    struct Hooked {
        hook: Box<dyn Fn() + Send + Sync>,
        launches: AtomicUsize,
    }

    impl UsageSource for Hooked {
        fn ready(&self) -> Result<(), Reason> {
            (self.hook)();
            Ok(())
        }

        fn probe(&self, _now: DateTime<Utc>) -> Result<Reading, Reason> {
            self.launches.fetch_add(1, Ordering::SeqCst);
            Ok(reading())
        }
    }

    #[test]
    fn a_clear_after_the_checks_launches_nothing() {
        let slot: Arc<std::sync::OnceLock<std::sync::Weak<SubscriptionUsage>>> = Arc::default();
        let target = Arc::clone(&slot);
        let source = Arc::new(Hooked {
            hook: Box::new(move || {
                if let Some(holder) = target.get().and_then(std::sync::Weak::upgrade) {
                    holder.clear();
                }
            }),
            launches: AtomicUsize::new(0),
        });
        let holder = Arc::new(SubscriptionUsage::new(source.clone(), Box::new(|| {})));
        slot.set(Arc::downgrade(&holder)).unwrap();
        assert_eq!(holder.refresh(), Scheduled::Started);
        assert!(holder.wait_idle(Duration::from_secs(5)));
        assert_eq!(source.launches.load(Ordering::SeqCst), 0);
        let login = login(&holder);
        assert_eq!(login["state"], "unknown");
        assert_eq!(login["reason"], "never_refreshed");
        // No launch, so no floor either.
        assert!(holder.held.lock().unwrap().last_launch.is_none());
    }

    #[test]
    fn clearing_forgets_the_reading_and_a_running_probe_lands_nowhere() {
        let (holder, _) = holder(fake(Ok(()), Ok(reading())));
        assert_eq!(holder.refresh(), Scheduled::Started);
        assert!(holder.wait_idle(Duration::from_secs(5)));
        holder.clear();
        assert_eq!(login(&holder)["reason"], "never_refreshed");
        // The floor went with it.
        assert_eq!(holder.refresh(), Scheduled::Started);
        // A probe that finishes after a clear is discarded.
        let generation = holder.held.lock().unwrap().generation;
        holder.wait_idle(Duration::from_secs(5));
        holder.clear();
        holder.finish(generation, Ok(reading()));
        assert_eq!(login(&holder)["state"], "unknown");
    }

    #[test]
    fn plan_types_map_to_the_fixed_labels() {
        for (wire, plan) in [
            ("free", Plan::Free),
            ("plus", Plan::Plus),
            ("pro", Plan::Pro),
            ("team", Plan::Team),
            ("business", Plan::Business),
            ("self_serve_business_prolite", Plan::Business),
            ("self_serve_business_usage_based", Plan::Business),
            ("enterprise", Plan::Enterprise),
            ("enterprise_cbp_automation", Plan::Enterprise),
            ("enterprise_cbp_usage_based", Plan::Enterprise),
            ("edu", Plan::Edu),
            ("edu_plus", Plan::Edu),
            ("edu_pro", Plan::Edu),
            ("go", Plan::Unknown),
            ("prolite", Plan::Unknown),
            ("ent26", Plan::Unknown),
            ("unknown", Plan::Unknown),
            ("Plus", Plan::Unknown),
            ("", Plan::Unknown),
        ] {
            assert_eq!(Plan::from_codex(wire), plan, "{wire}");
        }
    }
}
