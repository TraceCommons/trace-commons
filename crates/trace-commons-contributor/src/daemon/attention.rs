//! Deciding whether anything besides the digest may interrupt, and how.
//!
//! Every re-engagement notification kind (idle sessions, verdict news, the
//! weekly recap, Insights tips) is a *candidate* here. The arbiter decides,
//! once per tick and after the digest decision, whether one of them folds
//! into the digest as its third sentence, posts on its own, or waits. It
//! emits at most one notification per tick, and it holds the one budget both
//! the upsell and Insights features draw on, so neither can post around the
//! other.
//!
//! This module is pure. It takes snapshots (candidates, the digest plan, the
//! attention log, a notification-settings snapshot, the clock and the local
//! timezone) and returns an [`Outcome`]. It takes no locks, reads no files
//! and logs nothing; the caller stamps the log and publishes. Its output is
//! kind and reason labels only -- never a project, a path, an id or a count
//! tied to a project.
//!
//! Every cap, gap and window below is DRAFT, owner decision 4 or 5 in the
//! upsell decisions list, and is a named constant so changing one is a
//! one-line edit. None is user-tunable in phase 1.
//!
//! **A clock that goes backwards can only suppress.** A log entry stamped
//! after `now` is read as having happened *now*: it counts toward the gap,
//! the daily cap, the weekly cap and the per-kind caps. It is never ignored,
//! which is the opposite of `notify::interval_elapsed`'s choice for the
//! digest, and deliberately so: a stale digest stamp must not hold the one
//! notification that exists today off for days, while a stale re-engagement
//! stamp can only make a bonus notification wait.

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Timelike, Utc};

use super::settings::DigestSchedule;

/// At most this many standalone notifications per local calendar day, across
/// every kind of both features (upsell and Insights) together. DRAFT, owner
/// decision 4.
pub const STANDALONE_PER_DAY: u32 = 1;

/// At most this many standalone notifications per local ISO week, across every
/// kind of both features together. DRAFT, owner decision 4.
pub const STANDALONE_PER_WEEK: u32 = 3;

/// How many of the [`STANDALONE_PER_WEEK`] slots are held for idle sessions
/// (N1) while it has candidates and has not used one this week, so no other
/// kind can push an idle announcement past its queue entry's expiry. DRAFT,
/// owner decision 4.
pub const N1_RESERVED_WEEKLY_SLOTS: u32 = 1;

/// Minimum gap between any two notifications from the app, digest included.
/// It can delay a digest by at most this much and can never cause one (see
/// [`digest_held_by_gap`]). DRAFT, owner decision 4.
pub const MIN_GAP_ANY: Duration = Duration::hours(1);

/// No standalone within this long after a digest. DRAFT, owner decision 4.
pub const COALESCE_AFTER_DIGEST: Duration = Duration::hours(2);

/// No standalone when a digest with something to say is due within this long;
/// a foldable item waits to fold into it instead. DRAFT, owner decision 4.
pub const COALESCE_BEFORE_DIGEST: Duration = Duration::hours(1);

/// Start of the local quiet window for standalone kinds under
/// `DigestSchedule::Interval` (21:00 local). The digest keeps today's
/// behaviour. DRAFT, owner decision 5.
pub const QUIET_START_HOUR: u32 = 21;

/// End of the local quiet window for standalone kinds under
/// `DigestSchedule::Interval` (09:00 local). DRAFT, owner decision 5.
pub const QUIET_END_HOUR: u32 = 9;

/// Under `DigestSchedule::Evening{hour}`, standalone kinds post only on the
/// evening tick, read as the local hour starting at `hour:00`. The poll runs
/// every minute, so a single-tick reading would be lost to one late poll; an
/// hour keeps the rule ("the person chose one moment a day") without that
/// fragility, and [`STANDALONE_PER_DAY`] keeps it to one. DRAFT, owner
/// decisions 4 and 13 (the Evening rule in spec section 4.2).
pub const EVENING_TICK_WINDOW: Duration = Duration::hours(1);

/// Verdict news (N2): at most this many standalone notifications per local
/// ISO week. Folding is unlimited, because a fold adds no notification.
/// DRAFT, owner decision 4.
pub const N2_STANDALONE_PER_WEEK: u32 = 2;

/// Verdict news (N2): minimum interval between two standalone verdict
/// notifications. DRAFT, owner decision 4.
pub const N2_MIN_INTERVAL: Duration = Duration::hours(24);

/// Verdict news (N2) stays eligible for a notification this long after it
/// landed, then it is only a History card. Stale news is never announced.
/// DRAFT, owner decision 4 (spec section 4.2, coalescing rule 3).
pub const N2_NEWS_TTL: Duration = Duration::days(7);

/// Weekly recap (N3): at most this many per local ISO week, folded or
/// standalone, shared with Insights' recap. DRAFT, owner decisions 1 and 4.
pub const N3_PER_WEEK: u32 = 1;

/// Weekly recap (N3): minimum interval. DRAFT, owner decisions 1 and 4.
pub const N3_MIN_INTERVAL: Duration = Duration::days(7);

/// Insights tip (N4): at most this many standalone notifications per local
/// ISO week; Insights may lower it. DRAFT, owner decisions 4 and 25.
pub const N4_STANDALONE_PER_WEEK: u32 = 2;

/// Insights tip (N4): minimum interval, at least 24 h; Insights may raise it.
/// DRAFT, owner decisions 4 and 25.
pub const N4_MIN_INTERVAL: Duration = Duration::hours(24);

/// A re-engagement notification kind. The digest (N0) is not one: it is
/// decided by `notify::digest_due_for_schedule` and arrives here as a
/// [`DigestPlan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// N1, U4: finished sessions idle for the threshold. The primary trigger.
    IdleSessions,
    /// N4: an Insights tip. Time-bound, so it never folds.
    InsightsTip,
    /// N2, U2: verdicts landed.
    VerdictsLanded,
    /// N3, U3: the weekly recap. Held for the gamification ruling.
    WeeklyRecap,
}

impl Kind {
    /// Every kind, highest precedence first: N1 > N4 > N2 > N3.
    pub const ALL: [Kind; 4] = [
        Kind::IdleSessions,
        Kind::InsightsTip,
        Kind::VerdictsLanded,
        Kind::WeeklyRecap,
    ];

    /// The stable label used on the wire, in settings keys and in logs.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Kind::IdleSessions => "idle_sessions",
            Kind::InsightsTip => "insights_tip",
            Kind::VerdictsLanded => "verdicts_landed",
            Kind::WeeklyRecap => "weekly_recap",
        }
    }

    /// Lower is stronger. N1 is a decision and wins; N4 is worthless after
    /// the session it is about; verdicts keep; the recap keeps longest.
    #[must_use]
    pub fn precedence(self) -> u8 {
        match self {
            Kind::IdleSessions => 0,
            Kind::InsightsTip => 1,
            Kind::VerdictsLanded => 2,
            Kind::WeeklyRecap => 3,
        }
    }

    /// The default per-kind cap for this kind. N1's interval depends on the
    /// queue TTL (`idle_repeat_days_eff`), so the caller passes it.
    #[must_use]
    pub fn default_caps(self, idle_repeat: Duration) -> KindCaps {
        match self {
            Kind::IdleSessions => KindCaps {
                min_interval: idle_repeat,
                per_week: None,
                counts_folds: true,
            },
            Kind::InsightsTip => KindCaps {
                min_interval: N4_MIN_INTERVAL,
                per_week: Some(N4_STANDALONE_PER_WEEK),
                counts_folds: false,
            },
            Kind::VerdictsLanded => KindCaps {
                min_interval: N2_MIN_INTERVAL,
                per_week: Some(N2_STANDALONE_PER_WEEK),
                counts_folds: false,
            },
            Kind::WeeklyRecap => KindCaps {
                min_interval: N3_MIN_INTERVAL,
                per_week: Some(N3_PER_WEEK),
                counts_folds: true,
            },
        }
    }
}

/// The fact that resolves a candidate. A label only: the arbiter never
/// evaluates it. The caller drops a candidate once its fact holds, so its
/// eligibility clears by fact, never by a click or by being seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fact {
    /// The idle entries were reviewed, kept, dismissed, superseded or expired.
    QueueDecided,
    /// `nudge_opened{verdicts_landed}` was sent from the nudge's own action.
    NewsAcknowledged,
    /// The local ISO week the recap describes is over.
    WeekEnded,
    /// The session an Insights tip was about ended, or a new one started
    /// below the tip's threshold.
    SessionEnded,
}

impl Fact {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Fact::QueueDecided => "queue_decided",
            Fact::NewsAcknowledged => "news_acknowledged",
            Fact::WeekEnded => "week_ended",
            Fact::SessionEnded => "session_ended",
        }
    }
}

/// How often one kind may notify, inside the global caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindCaps {
    /// Minimum time between two notifications of this kind.
    pub min_interval: Duration,
    /// At most this many per local ISO week, or no weekly cap of its own.
    pub per_week: Option<u32>,
    /// Whether a fold counts against this cap (N1, N3) or only a standalone
    /// does (N2, N4: a fold adds no notification).
    pub counts_folds: bool,
}

/// Something that would like to interrupt.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub kind: Kind,
    /// Whether it can be the digest's third sentence. N4 cannot.
    pub foldable: bool,
    pub resolves: Fact,
    /// When it became eligible. With `ttl`, stale items are never announced.
    pub since: DateTime<Utc>,
    pub ttl: Option<Duration>,
    pub caps: KindCaps,
}

/// The digest decision for this tick, made before the arbiter runs and never
/// reordered by it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DigestPlan {
    /// The digest posts this tick (already decided, gap included).
    pub fires_this_tick: bool,
    pub pending: usize,
    pub contributed: usize,
    pub last_digest_at: Option<DateTime<Utc>>,
    /// When the digest is next due by its schedule. Read as "a digest with
    /// something to say" only while `pending > 0 || contributed > 0`.
    pub next_due_at: Option<DateTime<Utc>>,
    pub schedule: DigestSchedule,
}

/// A snapshot of the notification settings the arbiter needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotifySettings {
    /// "Notifications from Trace Commons". Off silences every kind.
    pub master: bool,
    /// `notify.digest`.
    pub digest: bool,
    pub idle_sessions: bool,
    pub insights_tip: bool,
    pub verdicts_landed: bool,
    pub weekly_recap: bool,
    /// At least one live subscriber declared it can render `reengage_due`.
    /// Gates standalone only: a fold rides in `digest_due`.
    pub renderer: bool,
}

impl NotifySettings {
    #[must_use]
    pub fn kind_on(&self, kind: Kind) -> bool {
        match kind {
            Kind::IdleSessions => self.idle_sessions,
            Kind::InsightsTip => self.insights_tip,
            Kind::VerdictsLanded => self.verdicts_landed,
            Kind::WeeklyRecap => self.weekly_recap,
        }
    }
}

/// How an announced kind reached the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// As the digest's third sentence.
    Folded,
    /// As its own `reengage_due` notification.
    Standalone,
}

/// One announcement. Kind label and time only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttentionEntry {
    pub kind: Kind,
    pub at: DateTime<Utc>,
    pub route: Route,
}

/// Why a candidate waits. A label, safe to log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Inside the local quiet window under `Interval`, or outside the evening
    /// tick under `Evening`.
    QuietHours,
    DailyCap,
    WeeklyCap,
    KindCap,
    /// A digest is due within [`COALESCE_BEFORE_DIGEST`], is guaranteed
    /// (foldable kind, digest on, sessions pending), or posts this tick and
    /// carried another item.
    NearDigest,
    AfterDigest,
    /// Too soon after the last notification, or another item took this
    /// tick's one notification.
    MinGap,
    NoRenderer,
    /// The master switch or the kind's own switch is off.
    Muted,
    /// Past its TTL. Stale news is never announced.
    Expired,
}

impl Reason {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Reason::QuietHours => "quiet_hours",
            Reason::DailyCap => "daily_cap",
            Reason::WeeklyCap => "weekly_cap",
            Reason::KindCap => "kind_cap",
            Reason::NearDigest => "near_digest",
            Reason::AfterDigest => "after_digest",
            Reason::MinGap => "min_gap",
            Reason::NoRenderer => "no_renderer",
            Reason::Muted => "muted",
            Reason::Expired => "expired",
        }
    }
}

/// The arbiter's answer for one tick. At most one of `fold` and `standalone`
/// is set; every other candidate is in `deferred` with its reason.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    pub fold: Option<Kind>,
    pub standalone: Option<Kind>,
    pub deferred: Vec<(Kind, Reason)>,
}

/// Whether `now` is inside the local quiet window ([`QUIET_START_HOUR`] to
/// [`QUIET_END_HOUR`]). Read in local time from `tz` each call, so a DST
/// change moves the window with the wall clock.
#[must_use]
pub fn in_quiet_hours<Tz: TimeZone>(now: DateTime<Utc>, tz: &Tz) -> bool {
    let hour = now.with_timezone(tz).hour();
    !(QUIET_END_HOUR..QUIET_START_HOUR).contains(&hour)
}

// The quiet window crosses midnight; `in_quiet_hours` relies on it. A window
// that does not would need that function changed, so the build says so.
const _: () = assert!(QUIET_END_HOUR < QUIET_START_HOUR && QUIET_START_HOUR <= 24);

/// Whether `now` is on the evening tick for `hour`: inside
/// [`EVENING_TICK_WINDOW`] from the most recent local `hour:00`.
#[must_use]
pub fn evening_tick<Tz: TimeZone>(now: DateTime<Utc>, hour: u8, tz: &Tz) -> bool {
    let today = now.with_timezone(tz).date_naive();
    [today, today - Duration::days(1)]
        .into_iter()
        .filter_map(|date| local_target(date, hour, tz))
        .any(|target| target <= now && now < target + EVENING_TICK_WINDOW)
}

/// `hour:00` on local `date` as a UTC instant. A repeated hour (fall back)
/// takes the earlier pass, as `notify` does; a skipped hour (spring forward)
/// takes the first real instant after it.
fn local_target<Tz: TimeZone>(date: NaiveDate, hour: u8, tz: &Tz) -> Option<DateTime<Utc>> {
    let naive = date.and_hms_opt(u32::from(hour.min(23)), 0, 0)?;
    (0..=240)
        .map(|m| naive + Duration::minutes(m))
        .find_map(|n| tz.from_local_datetime(&n).earliest())
        .map(|dt| dt.with_timezone(&Utc))
}

/// Rule 4 of spec section 4.2: a digest that is due within [`MIN_GAP_ANY`]
/// after a standalone waits out the remainder. It can delay a digest by at
/// most [`MIN_GAP_ANY`] and can never cause one -- the answer is only ever
/// "hold". A fold rode inside a digest, which has its own interval, so only
/// standalone entries count.
#[must_use]
pub fn digest_held_by_gap(log: &[AttentionEntry], now: DateTime<Utc>) -> bool {
    log.iter()
        .filter(|e| e.route == Route::Standalone)
        .any(|e| within(e.at, now, MIN_GAP_ANY))
}

/// Decide this tick. See the module doc and spec section 4.2.
///
/// The digest decision is already made and arrives in `digest`; nothing here
/// can make a digest fire. Candidates are considered strongest first
/// ([`Kind::precedence`]); `deferred` lists every candidate not chosen, in
/// that order, with the first reason that held it.
#[must_use]
pub fn arbitrate<Tz: TimeZone>(
    candidates: &[Candidate],
    digest: &DigestPlan,
    log: &[AttentionEntry],
    settings: &NotifySettings,
    now: DateTime<Utc>,
    tz: &Tz,
) -> Outcome {
    let mut out = Outcome::default();
    let mut ordered: Vec<&Candidate> = candidates.iter().collect();
    ordered.sort_by_key(|c| c.kind.precedence());

    let mut eligible: Vec<&Candidate> = Vec::new();
    for c in ordered {
        if !settings.master || !settings.kind_on(c.kind) {
            out.deferred.push((c.kind, Reason::Muted));
        } else if c.ttl.is_some_and(|ttl| now > c.since + ttl) {
            out.deferred.push((c.kind, Reason::Expired));
        } else {
            eligible.push(c);
        }
    }

    // 1. A digest posts this tick: the strongest foldable item becomes its
    //    third sentence and nothing posts on its own.
    if digest.fires_this_tick {
        for c in eligible {
            let capped = c.caps.counts_folds && kind_capped(c, log, now, tz);
            if out.fold.is_none() && c.foldable && !capped {
                out.fold = Some(c.kind);
            } else if c.foldable && capped {
                out.deferred.push((c.kind, Reason::KindCap));
            } else {
                out.deferred.push((c.kind, Reason::NearDigest));
            }
        }
        return out;
    }

    // 2. No digest this tick: at most one standalone, if every gate passes.
    let digest_guaranteed = settings.digest && digest.pending > 0;
    let something_to_say = digest.pending > 0 || digest.contributed > 0;
    let digest_near = settings.digest
        && something_to_say
        && digest
            .next_due_at
            .is_some_and(|due| due - now < COALESCE_BEFORE_DIGEST);
    let after_digest = digest
        .last_digest_at
        .is_some_and(|last| within(last, now, COALESCE_AFTER_DIGEST));
    let gap_held = log.iter().any(|e| within(e.at, now, MIN_GAP_ANY));
    let quiet = match digest.schedule {
        DigestSchedule::Evening { hour } if settings.digest => !evening_tick(now, hour, tz),
        _ => in_quiet_hours(now, tz),
    };
    let today = local_date(now, tz);
    let this_week = local_week(now, tz);
    let standalone_today = log
        .iter()
        .filter(|e| e.route == Route::Standalone && local_date(seen_at(e.at, now), tz) == today)
        .count() as u32;
    let standalone_week = |kind: Option<Kind>| {
        log.iter()
            .filter(|e| e.route == Route::Standalone && kind.is_none_or(|k| e.kind == k))
            .filter(|e| local_week(seen_at(e.at, now), tz) == this_week)
            .count() as u32
    };
    let week_total = standalone_week(None);
    let reserve_for_n1 = eligible.iter().any(|c| c.kind == Kind::IdleSessions)
        && standalone_week(Some(Kind::IdleSessions)) == 0;

    for c in eligible {
        let weekly_limit = if reserve_for_n1 && c.kind != Kind::IdleSessions {
            STANDALONE_PER_WEEK.saturating_sub(N1_RESERVED_WEEKLY_SLOTS)
        } else {
            STANDALONE_PER_WEEK
        };
        let reason = if c.foldable && digest_guaranteed {
            Some(Reason::NearDigest)
        } else if !settings.renderer {
            Some(Reason::NoRenderer)
        } else if gap_held {
            Some(Reason::MinGap)
        } else if after_digest {
            Some(Reason::AfterDigest)
        } else if digest_near {
            Some(Reason::NearDigest)
        } else if quiet {
            Some(Reason::QuietHours)
        } else if standalone_today >= STANDALONE_PER_DAY {
            Some(Reason::DailyCap)
        } else if week_total >= weekly_limit {
            Some(Reason::WeeklyCap)
        } else if kind_capped(c, log, now, tz) {
            Some(Reason::KindCap)
        } else if out.standalone.is_some() {
            // One notification per tick; the next may come after the gap.
            Some(Reason::MinGap)
        } else {
            None
        };
        match reason {
            Some(r) => out.deferred.push((c.kind, r)),
            None => out.standalone = Some(c.kind),
        }
    }
    out
}

/// Whether `c`'s own cap holds it now: its minimum interval since the last
/// notification of its kind, or its weekly count. Folds count only when the
/// kind's cap says so.
fn kind_capped<Tz: TimeZone>(
    c: &Candidate,
    log: &[AttentionEntry],
    now: DateTime<Utc>,
    tz: &Tz,
) -> bool {
    let counted = || {
        log.iter()
            .filter(move |e| e.kind == c.kind)
            .filter(move |e| e.route == Route::Standalone || c.caps.counts_folds)
    };
    if counted().any(|e| within(e.at, now, c.caps.min_interval)) {
        return true;
    }
    c.caps.per_week.is_some_and(|cap| {
        let week = local_week(now, tz);
        counted()
            .filter(|e| local_week(seen_at(e.at, now), tz) == week)
            .count() as u32
            >= cap
    })
}

/// Whether `at` is less than `window` before `now`. A stamp after `now` (the
/// clock went backwards) is inside every window: it can only suppress.
fn within(at: DateTime<Utc>, now: DateTime<Utc>, window: Duration) -> bool {
    now.signed_duration_since(at) < window
}

/// A stamp after `now` is read as `now`, so it counts in today's and this
/// week's caps rather than in some future bucket nothing ever reads.
fn seen_at(at: DateTime<Utc>, now: DateTime<Utc>) -> DateTime<Utc> {
    at.min(now)
}

fn local_date<Tz: TimeZone>(at: DateTime<Utc>, tz: &Tz) -> NaiveDate {
    at.with_timezone(tz).date_naive()
}

fn local_week<Tz: TimeZone>(at: DateTime<Utc>, tz: &Tz) -> (i32, u32) {
    let w = at.with_timezone(tz).iso_week();
    (w.year(), w.week())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::notify;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    const IDLE_REPEAT: Duration = Duration::days(7);

    fn cand(kind: Kind, since: DateTime<Utc>) -> Candidate {
        Candidate {
            kind,
            foldable: kind != Kind::InsightsTip,
            resolves: match kind {
                Kind::IdleSessions => Fact::QueueDecided,
                Kind::InsightsTip => Fact::SessionEnded,
                Kind::VerdictsLanded => Fact::NewsAcknowledged,
                Kind::WeeklyRecap => Fact::WeekEnded,
            },
            since,
            ttl: (kind == Kind::VerdictsLanded).then_some(N2_NEWS_TTL),
            caps: kind.default_caps(IDLE_REPEAT),
        }
    }

    fn all_on() -> NotifySettings {
        NotifySettings {
            master: true,
            digest: true,
            idle_sessions: true,
            insights_tip: true,
            verdicts_landed: true,
            weekly_recap: true,
            renderer: true,
        }
    }

    /// Nothing pending, nothing contributed, digest not firing, last digest
    /// long ago: the quiet case where a standalone is possible.
    fn quiet_plan() -> DigestPlan {
        DigestPlan {
            fires_this_tick: false,
            pending: 0,
            contributed: 0,
            last_digest_at: None,
            next_due_at: None,
            schedule: DigestSchedule::Interval,
        }
    }

    fn entry(kind: Kind, at: DateTime<Utc>, route: Route) -> AttentionEntry {
        AttentionEntry { kind, at, route }
    }

    /// The invariants every call must hold, whatever its inputs.
    fn check_invariants(cands: &[Candidate], plan: &DigestPlan, out: &Outcome) {
        assert!(
            out.fold.is_none() || out.standalone.is_none(),
            "two notifications in one tick: {out:?}"
        );
        if plan.fires_this_tick {
            assert!(out.standalone.is_none(), "standalone beside a digest");
        } else {
            assert!(out.fold.is_none(), "fold without a digest");
        }
        let chosen = usize::from(out.fold.is_some()) + usize::from(out.standalone.is_some());
        assert_eq!(chosen + out.deferred.len(), cands.len(), "{out:?}");
    }

    fn run(
        cands: &[Candidate],
        plan: &DigestPlan,
        log: &[AttentionEntry],
        s: &NotifySettings,
        now: DateTime<Utc>,
    ) -> Outcome {
        let out = arbitrate(cands, plan, log, s, now, &Utc);
        check_invariants(cands, plan, &out);
        out
    }

    fn reason_for(out: &Outcome, kind: Kind) -> Option<Reason> {
        out.deferred
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, r)| *r)
    }

    // A noon on a Wednesday, outside quiet hours, mid ISO week.
    const NOON: &str = "2026-03-18T12:00:00Z";

    #[test]
    fn precedence_is_n1_then_n4_then_n2_then_n3() {
        let mut kinds = Kind::ALL.to_vec();
        kinds.reverse();
        kinds.sort_by_key(|k| k.precedence());
        assert_eq!(
            kinds,
            vec![
                Kind::IdleSessions,
                Kind::InsightsTip,
                Kind::VerdictsLanded,
                Kind::WeeklyRecap
            ]
        );
        assert_eq!(Kind::IdleSessions.label(), "idle_sessions");
        assert_eq!(Reason::NoRenderer.label(), "no_renderer");
    }

    #[test]
    fn a_lone_candidate_posts_standalone_when_nothing_holds_it() {
        let now = at(NOON);
        let mut s = all_on();
        s.digest = false;
        let cands = [cand(Kind::VerdictsLanded, now - Duration::hours(1))];
        let out = run(&cands, &quiet_plan(), &[], &s, now);
        assert_eq!(out.standalone, Some(Kind::VerdictsLanded));
    }

    #[test]
    fn at_most_one_notification_per_tick_and_the_strongest_wins() {
        let now = at(NOON);
        let mut s = all_on();
        s.digest = false;
        let cands: Vec<_> = Kind::ALL
            .iter()
            .rev()
            .map(|k| cand(*k, now - Duration::hours(1)))
            .collect();
        let out = run(&cands, &quiet_plan(), &[], &s, now);
        assert_eq!(out.standalone, Some(Kind::IdleSessions));
        assert_eq!(out.deferred.len(), 3);
        assert!(out.deferred.iter().all(|(_, r)| *r == Reason::MinGap));
    }

    #[test]
    fn a_due_digest_takes_the_strongest_foldable_and_nothing_posts_standalone() {
        let now = at(NOON);
        let plan = DigestPlan {
            fires_this_tick: true,
            pending: 2,
            ..quiet_plan()
        };
        let cands: Vec<_> = Kind::ALL
            .iter()
            .map(|k| cand(*k, now - Duration::hours(1)))
            .collect();
        let out = run(&cands, &plan, &[], &all_on(), now);
        assert_eq!(out.fold, Some(Kind::IdleSessions));
        assert_eq!(out.standalone, None);
        // N4 cannot fold and cannot post beside a digest.
        assert_eq!(
            reason_for(&out, Kind::InsightsTip),
            Some(Reason::NearDigest)
        );
    }

    #[test]
    fn a_fold_skips_a_kind_whose_cap_counts_folds_and_is_spent() {
        let now = at(NOON);
        let plan = DigestPlan {
            fires_this_tick: true,
            pending: 2,
            ..quiet_plan()
        };
        let log = [entry(
            Kind::IdleSessions,
            now - Duration::days(2),
            Route::Folded,
        )];
        let cands = [
            cand(Kind::IdleSessions, now - Duration::hours(1)),
            cand(Kind::VerdictsLanded, now - Duration::hours(1)),
        ];
        let out = run(&cands, &plan, &log, &all_on(), now);
        assert_eq!(out.fold, Some(Kind::VerdictsLanded));
        assert_eq!(reason_for(&out, Kind::IdleSessions), Some(Reason::KindCap));
    }

    #[test]
    fn verdict_folding_is_unlimited() {
        let now = at(NOON);
        let plan = DigestPlan {
            fires_this_tick: true,
            pending: 1,
            ..quiet_plan()
        };
        let log: Vec<_> = (1..=5)
            .map(|h| {
                entry(
                    Kind::VerdictsLanded,
                    now - Duration::hours(4 * h),
                    Route::Folded,
                )
            })
            .collect();
        let cands = [cand(Kind::VerdictsLanded, now - Duration::hours(1))];
        let out = run(&cands, &plan, &log, &all_on(), now);
        assert_eq!(out.fold, Some(Kind::VerdictsLanded));
    }

    #[test]
    fn foldable_kinds_never_post_standalone_while_a_digest_is_guaranteed() {
        let now = at(NOON);
        let plan = DigestPlan {
            pending: 3,
            last_digest_at: Some(now - Duration::hours(3)),
            next_due_at: Some(now + Duration::hours(3)),
            ..quiet_plan()
        };
        for kind in [Kind::IdleSessions, Kind::VerdictsLanded, Kind::WeeklyRecap] {
            let cands = [cand(kind, now - Duration::hours(1))];
            let out = run(&cands, &plan, &[], &all_on(), now);
            assert_eq!(out.standalone, None, "{kind:?}");
            assert_eq!(reason_for(&out, kind), Some(Reason::NearDigest));
        }
        // With the digest switched off the guarantee is gone.
        let mut s = all_on();
        s.digest = false;
        let cands = [cand(Kind::VerdictsLanded, now - Duration::hours(1))];
        let out = run(&cands, &plan, &[], &s, now);
        assert_eq!(out.standalone, Some(Kind::VerdictsLanded));
    }

    #[test]
    fn no_standalone_within_two_hours_after_a_digest() {
        let now = at(NOON);
        let mut plan = DigestPlan {
            last_digest_at: Some(now - Duration::minutes(119)),
            ..quiet_plan()
        };
        let cands = [cand(Kind::InsightsTip, now - Duration::minutes(5))];
        let out = run(&cands, &plan, &[], &all_on(), now);
        assert_eq!(
            reason_for(&out, Kind::InsightsTip),
            Some(Reason::AfterDigest)
        );
        plan.last_digest_at = Some(now - COALESCE_AFTER_DIGEST);
        let out = run(&cands, &plan, &[], &all_on(), now);
        assert_eq!(out.standalone, Some(Kind::InsightsTip));
    }

    #[test]
    fn no_standalone_within_an_hour_before_a_digest_with_something_to_say() {
        let now = at(NOON);
        let mut plan = DigestPlan {
            contributed: 1,
            last_digest_at: Some(now - Duration::hours(3)),
            next_due_at: Some(now + Duration::minutes(59)),
            ..quiet_plan()
        };
        let cands = [cand(Kind::InsightsTip, now - Duration::minutes(5))];
        let out = run(&cands, &plan, &[], &all_on(), now);
        assert_eq!(
            reason_for(&out, Kind::InsightsTip),
            Some(Reason::NearDigest)
        );
        // Nothing to say: the digest will not fire, so it holds nothing.
        plan.contributed = 0;
        let out = run(&cands, &plan, &[], &all_on(), now);
        assert_eq!(out.standalone, Some(Kind::InsightsTip));
    }

    #[test]
    fn min_gap_holds_after_any_notification() {
        let now = at(NOON);
        let mut s = all_on();
        s.digest = false;
        let log = [entry(
            Kind::InsightsTip,
            now - Duration::minutes(59),
            Route::Standalone,
        )];
        let cands = [cand(Kind::VerdictsLanded, now - Duration::hours(1))];
        let out = run(&cands, &quiet_plan(), &log, &s, now);
        assert_eq!(reason_for(&out, Kind::VerdictsLanded), Some(Reason::MinGap));
    }

    #[test]
    fn quiet_hours_hold_standalone_under_interval() {
        let mut s = all_on();
        s.digest = false;
        for (time, posts) in [
            ("2026-03-18T20:59:00Z", true),
            ("2026-03-18T21:00:00Z", false),
            ("2026-03-19T03:00:00Z", false),
            ("2026-03-19T08:59:00Z", false),
            ("2026-03-19T09:00:00Z", true),
        ] {
            let now = at(time);
            let cands = [cand(Kind::VerdictsLanded, now - Duration::hours(1))];
            let out = run(&cands, &quiet_plan(), &[], &s, now);
            assert_eq!(out.standalone.is_some(), posts, "{time}");
            if !posts {
                assert_eq!(
                    reason_for(&out, Kind::VerdictsLanded),
                    Some(Reason::QuietHours)
                );
            }
        }
    }

    #[test]
    fn evening_posts_standalone_only_on_the_evening_tick() {
        let mut s = all_on();
        s.digest = true;
        let plan = DigestPlan {
            schedule: DigestSchedule::Evening { hour: 18 },
            ..quiet_plan()
        };
        for (time, posts) in [
            ("2026-03-18T12:00:00Z", false),
            ("2026-03-18T17:59:00Z", false),
            ("2026-03-18T18:00:00Z", true),
            ("2026-03-18T18:59:00Z", true),
            ("2026-03-18T19:00:00Z", false),
        ] {
            let now = at(time);
            let cands = [cand(Kind::VerdictsLanded, now - Duration::hours(1))];
            let out = run(&cands, &plan, &[], &s, now);
            assert_eq!(out.standalone.is_some(), posts, "{time}");
        }
        // A late evening hour is not quiet under Evening: the person chose it.
        let plan = DigestPlan {
            schedule: DigestSchedule::Evening { hour: 22 },
            ..quiet_plan()
        };
        let now = at("2026-03-18T22:10:00Z");
        let cands = [cand(Kind::VerdictsLanded, now - Duration::hours(1))];
        assert!(run(&cands, &plan, &[], &s, now).standalone.is_some());
    }

    #[test]
    fn daily_and_weekly_caps_bound_standalones() {
        let mut s = all_on();
        s.digest = false;
        let now = at(NOON);
        let cands = [cand(Kind::InsightsTip, now - Duration::hours(1))];
        // Earlier the same local day.
        let log = [entry(
            Kind::VerdictsLanded,
            at("2026-03-18T09:30:00Z"),
            Route::Standalone,
        )];
        let out = run(&cands, &quiet_plan(), &log, &s, now);
        assert_eq!(reason_for(&out, Kind::InsightsTip), Some(Reason::DailyCap));
        // Three earlier this ISO week (Monday 16th onward).
        let log = [
            entry(
                Kind::VerdictsLanded,
                at("2026-03-16T10:00:00Z"),
                Route::Standalone,
            ),
            entry(
                Kind::IdleSessions,
                at("2026-03-17T10:00:00Z"),
                Route::Standalone,
            ),
            entry(Kind::WeeklyRecap, at("2026-03-17T18:00:00Z"), Route::Folded),
            entry(
                Kind::VerdictsLanded,
                at("2026-03-15T10:00:00Z"),
                Route::Standalone,
            ),
        ];
        // Only two standalone this week (the 15th is last week): posts.
        let out = run(&cands, &quiet_plan(), &log, &s, now);
        assert_eq!(out.standalone, Some(Kind::InsightsTip));
        let mut log = log.to_vec();
        log.push(entry(
            Kind::InsightsTip,
            at("2026-03-16T15:00:00Z"),
            Route::Standalone,
        ));
        let out = run(&cands, &quiet_plan(), &log, &s, now);
        assert_eq!(reason_for(&out, Kind::InsightsTip), Some(Reason::WeeklyCap));
    }

    #[test]
    fn one_weekly_slot_is_reserved_for_idle_sessions() {
        let mut s = all_on();
        s.digest = false;
        let now = at(NOON);
        let log = [
            entry(
                Kind::VerdictsLanded,
                at("2026-03-16T10:00:00Z"),
                Route::Standalone,
            ),
            entry(
                Kind::InsightsTip,
                at("2026-03-17T10:00:00Z"),
                Route::Standalone,
            ),
        ];
        // N1 is held by its own cap this tick, so it cannot post; the third
        // slot is still kept for it.
        let n1_log = [
            log[0],
            log[1],
            entry(
                Kind::IdleSessions,
                at("2026-03-12T10:00:00Z"),
                Route::Standalone,
            ),
        ];
        let cands = [
            cand(Kind::IdleSessions, now - Duration::days(1)),
            cand(Kind::VerdictsLanded, now - Duration::hours(1)),
        ];
        let out = run(&cands, &quiet_plan(), &n1_log, &s, now);
        assert_eq!(out.standalone, None);
        assert_eq!(reason_for(&out, Kind::IdleSessions), Some(Reason::KindCap));
        assert_eq!(
            reason_for(&out, Kind::VerdictsLanded),
            Some(Reason::WeeklyCap)
        );
        // Without an N1 candidate, the third slot is free.
        let out = run(&cands[1..], &quiet_plan(), &log, &s, now);
        assert_eq!(out.standalone, Some(Kind::VerdictsLanded));
        // And N1 itself may take it.
        let out = run(&cands[..1], &quiet_plan(), &log, &s, now);
        assert_eq!(out.standalone, Some(Kind::IdleSessions));
    }

    #[test]
    fn per_kind_caps_hold_inside_the_global_ones() {
        let mut s = all_on();
        s.digest = false;
        let now = at(NOON);
        let cands = [cand(Kind::VerdictsLanded, now - Duration::hours(1))];
        let log = [entry(
            Kind::VerdictsLanded,
            now - Duration::hours(23),
            Route::Standalone,
        )];
        let out = run(&cands, &quiet_plan(), &log, &s, now);
        // Yesterday 13:00 is another local day and well past the gap: only
        // N2's own 24-hour interval holds it.
        assert_eq!(
            reason_for(&out, Kind::VerdictsLanded),
            Some(Reason::KindCap)
        );
        assert!(out.standalone.is_none());
        // A folded verdict does not spend N2's standalone cap.
        let log = [entry(
            Kind::VerdictsLanded,
            now - Duration::hours(2),
            Route::Folded,
        )];
        let mut plan = quiet_plan();
        plan.last_digest_at = Some(now - Duration::hours(2));
        let out = run(&cands, &plan, &log, &s, now);
        assert_eq!(out.standalone, Some(Kind::VerdictsLanded));
    }

    #[test]
    fn stale_news_is_never_announced() {
        let mut s = all_on();
        s.digest = false;
        let now = at(NOON);
        let cands = [cand(
            Kind::VerdictsLanded,
            now - N2_NEWS_TTL - Duration::minutes(1),
        )];
        let out = run(&cands, &quiet_plan(), &[], &s, now);
        assert_eq!(
            reason_for(&out, Kind::VerdictsLanded),
            Some(Reason::Expired)
        );
        let plan = DigestPlan {
            fires_this_tick: true,
            pending: 1,
            ..quiet_plan()
        };
        let out = run(&cands, &plan, &[], &all_on(), now);
        assert_eq!(out.fold, None);
    }

    #[test]
    fn muted_kinds_and_master_off_never_fold_or_post() {
        let now = at(NOON);
        let cands: Vec<_> = Kind::ALL
            .iter()
            .map(|k| cand(*k, now - Duration::hours(1)))
            .collect();
        let plan = DigestPlan {
            fires_this_tick: true,
            pending: 1,
            ..quiet_plan()
        };
        let mut s = all_on();
        s.master = false;
        let out = run(&cands, &plan, &[], &s, now);
        assert_eq!(
            out,
            Outcome {
                fold: None,
                standalone: None,
                deferred: Kind::ALL.iter().map(|k| (*k, Reason::Muted)).collect()
            }
        );
        let mut s = all_on();
        s.idle_sessions = false;
        let out = run(&cands, &plan, &[], &s, now);
        assert_eq!(out.fold, Some(Kind::VerdictsLanded));
        assert_eq!(reason_for(&out, Kind::IdleSessions), Some(Reason::Muted));
    }

    #[test]
    fn no_renderer_defers_standalone_but_not_a_fold() {
        let now = at(NOON);
        let mut s = all_on();
        s.digest = false;
        s.renderer = false;
        let cands = [cand(Kind::VerdictsLanded, now - Duration::hours(1))];
        let out = run(&cands, &quiet_plan(), &[], &s, now);
        assert_eq!(
            reason_for(&out, Kind::VerdictsLanded),
            Some(Reason::NoRenderer)
        );
        // Budget is spent only by the caller stamping the log for a chosen
        // kind; a deferral returns nothing to stamp, so the next tick with a
        // renderer still has the whole budget.
        s.renderer = true;
        let out = run(&cands, &quiet_plan(), &[], &s, now + Duration::minutes(1));
        assert_eq!(out.standalone, Some(Kind::VerdictsLanded));
        // A fold rides in digest_due, which every shell renders.
        let mut s = all_on();
        s.renderer = false;
        let plan = DigestPlan {
            fires_this_tick: true,
            pending: 1,
            ..quiet_plan()
        };
        assert_eq!(
            run(&cands, &plan, &[], &s, now).fold,
            Some(Kind::VerdictsLanded)
        );
    }

    #[test]
    fn a_backwards_clock_only_suppresses() {
        let mut s = all_on();
        s.digest = false;
        let now = at(NOON);
        let cands = [cand(Kind::InsightsTip, now - Duration::hours(1))];
        // A notification stamped three days in the future: the clock moved
        // back. It counts as having happened now, everywhere.
        let future = [entry(
            Kind::VerdictsLanded,
            now + Duration::days(3),
            Route::Standalone,
        )];
        let out = run(&cands, &quiet_plan(), &future, &s, now);
        assert_eq!(out.standalone, None);
        assert_eq!(reason_for(&out, Kind::InsightsTip), Some(Reason::MinGap));
        // A future digest stamp holds standalones too.
        let plan = DigestPlan {
            last_digest_at: Some(now + Duration::days(3)),
            ..quiet_plan()
        };
        let out = run(&cands, &plan, &[], &s, now);
        assert_eq!(
            reason_for(&out, Kind::InsightsTip),
            Some(Reason::AfterDigest)
        );
        // And a future per-kind stamp holds that kind.
        let n4 = [entry(
            Kind::InsightsTip,
            now + Duration::days(10),
            Route::Standalone,
        )];
        let later = now + Duration::days(20);
        let cands_later = [cand(Kind::InsightsTip, later - Duration::hours(1))];
        // Twenty days on, the stamp is in the past and long expired: posts.
        assert_eq!(
            run(&cands_later, &quiet_plan(), &n4, &s, later).standalone,
            Some(Kind::InsightsTip)
        );
        assert_eq!(run(&cands, &quiet_plan(), &n4, &s, now).standalone, None);
        // A future stamp also holds a digest by the gap, never causes one.
        assert!(digest_held_by_gap(&future, now));
    }

    #[test]
    fn the_digest_gap_only_ever_holds_and_for_at_most_an_hour() {
        let now = at(NOON);
        assert!(!digest_held_by_gap(&[], now));
        let log = [entry(
            Kind::InsightsTip,
            now - Duration::minutes(30),
            Route::Standalone,
        )];
        assert!(digest_held_by_gap(&log, now));
        assert!(!digest_held_by_gap(&log, now + Duration::minutes(30)));
        // A fold rode inside the digest itself; it holds nothing.
        let log = [entry(
            Kind::VerdictsLanded,
            now - Duration::minutes(1),
            Route::Folded,
        )];
        assert!(!digest_held_by_gap(&log, now));
    }

    #[test]
    fn dst_transitions_only_suppress() {
        let tz = chrono_tz::America::New_York;
        let mut s = all_on();
        s.digest = false;
        // Spring forward, 2026-03-08 02:00 EST -> 03:00 EDT. 09:30 EDT is
        // 13:30Z; quiet until 09:00 local, read in local time.
        let morning = at("2026-03-08T12:59:00Z"); // 08:59 EDT
        let cands = [cand(Kind::VerdictsLanded, morning - Duration::hours(1))];
        let out = arbitrate(&cands, &quiet_plan(), &[], &s, morning, &tz);
        assert_eq!(out.standalone, None);
        let open = at("2026-03-08T13:00:00Z"); // 09:00 EDT
        let out = arbitrate(&cands, &quiet_plan(), &[], &s, open, &tz);
        assert_eq!(out.standalone, Some(Kind::VerdictsLanded));
        // Fall back, 2026-11-01: the repeated 01:00 hour is inside quiet
        // hours on both passes, and one local day still holds one standalone.
        let first = at("2026-11-01T05:30:00Z"); // 01:30 EDT
        let second = at("2026-11-01T06:30:00Z"); // 01:30 EST
        for now in [first, second] {
            let cands = [cand(Kind::VerdictsLanded, now - Duration::hours(1))];
            assert_eq!(
                arbitrate(&cands, &quiet_plan(), &[], &s, now, &tz).standalone,
                None
            );
        }
        let posted = [entry(
            Kind::InsightsTip,
            at("2026-11-01T14:00:00Z"),
            Route::Standalone,
        )];
        let evening = at("2026-11-02T01:00:00Z"); // 20:00 EST, same local day
        let cands = [cand(Kind::VerdictsLanded, evening - Duration::hours(1))];
        let out = arbitrate(&cands, &quiet_plan(), &posted, &s, evening, &tz);
        assert_eq!(out.standalone, None);
        assert_eq!(out.deferred, vec![(Kind::VerdictsLanded, Reason::DailyCap)]);
    }

    // ---------------------------------------------------------------
    // The 8-week simulation.
    // ---------------------------------------------------------------

    /// xorshift64*: a deterministic stream for the simulation, so a failure
    /// reproduces from its seed. Not for anything but tests.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        /// True with probability `per_mille / 1000`.
        fn chance(&mut self, per_mille: u64) -> bool {
            self.next() % 1000 < per_mille
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Sched {
        Interval(u64),
        Evening(u8),
        DigestOff,
        MasterOff,
    }

    const SCHEDULES: [Sched; 6] = [
        Sched::Interval(4),
        Sched::Interval(24),
        Sched::Interval(1),
        Sched::Evening(18),
        Sched::DigestOff,
        Sched::MasterOff,
    ];

    #[derive(Default)]
    struct Tally {
        digests: Vec<DateTime<Utc>>,
        standalones: Vec<(Kind, DateTime<Utc>)>,
        folds: usize,
    }

    /// Run eight weeks of five-minute ticks. `saturate` keeps every kind a
    /// candidate and something pending the whole time; otherwise kinds and
    /// pending sessions come and go at random from `seed`.
    fn simulate(sched: Sched, start: DateTime<Utc>, seed: u64, saturate: bool) -> Tally {
        let tz = chrono_tz::America::New_York;
        let step = Duration::minutes(5);
        let end = start + Duration::weeks(8);
        let mut rng = Rng(seed | 1);
        let settings = NotifySettings {
            master: sched != Sched::MasterOff,
            digest: !matches!(sched, Sched::DigestOff | Sched::MasterOff),
            ..all_on()
        };
        let (schedule, interval_secs) = match sched {
            Sched::Interval(h) => (DigestSchedule::Interval, h * 3600),
            Sched::Evening(hour) => (DigestSchedule::Evening { hour }, 14_400),
            _ => (DigestSchedule::Interval, 14_400),
        };
        let mut pending = usize::from(saturate);
        let mut contributed = 0usize;
        let mut present: [Option<DateTime<Utc>>; 4] = [None; 4];
        let mut last_digest_at: Option<DateTime<Utc>> = None;
        let mut log: Vec<AttentionEntry> = Vec::new();
        let mut tally = Tally::default();
        let mut now = start;
        while now < end {
            // The world moves. Draw the same number of values every tick so
            // the stream does not depend on the arbiter's answers.
            let draws: Vec<bool> = (0..12)
                .map(|i| rng.chance([4, 3, 2, 2, 2, 2, 1, 2, 1, 2, 1, 2][i]))
                .collect();
            if !saturate {
                if draws[0] {
                    pending += 1;
                }
                if draws[1] && pending > 0 {
                    pending = 0;
                }
                if draws[2] {
                    contributed += 1;
                }
                for (i, kind) in Kind::ALL.iter().enumerate() {
                    let (arrive, leave) = (draws[3 + 2 * i], draws[4 + 2 * i]);
                    let allowed = *kind != Kind::IdleSessions || pending > 0;
                    if !allowed || (leave && present[i].is_some()) {
                        present[i] = None;
                    } else if arrive && present[i].is_none() {
                        present[i] = Some(now);
                    }
                }
            } else {
                pending = 1;
                for slot in present.iter_mut() {
                    slot.get_or_insert(now);
                }
            }
            log.retain(|e| now.signed_duration_since(e.at) < Duration::days(8));

            let due = settings.master
                && settings.digest
                && notify::digest_due_for_schedule(
                    schedule,
                    last_digest_at,
                    now,
                    interval_secs,
                    &tz,
                    pending,
                    contributed,
                )
                && !digest_held_by_gap(&log, now);
            let next_due_at = Some(match schedule {
                DigestSchedule::Interval => last_digest_at
                    .map(|l| l + Duration::seconds(interval_secs as i64))
                    .unwrap_or(now),
                DigestSchedule::Evening { hour } => {
                    notify::next_evening_at(last_digest_at, now, hour, &tz)
                }
            });
            let plan = DigestPlan {
                fires_this_tick: due,
                pending,
                contributed,
                last_digest_at,
                next_due_at,
                schedule,
            };
            let cands: Vec<Candidate> = Kind::ALL
                .iter()
                .zip(present.iter())
                .filter_map(|(k, p)| p.map(|since| cand(*k, since)))
                .collect();
            let out = arbitrate(&cands, &plan, &log, &settings, now, &tz);
            check_invariants(&cands, &plan, &out);

            if let Some(kind) = out.standalone {
                // Section 4.2's contract, checked at the moment of posting.
                let local = now.with_timezone(&tz);
                assert!(settings.master && settings.renderer);
                if let DigestSchedule::Evening { hour } = schedule {
                    if settings.digest {
                        assert!(evening_tick(now, hour, &tz), "{local}");
                    }
                } else {
                    let h = local.hour();
                    assert!((QUIET_END_HOUR..QUIET_START_HOUR).contains(&h), "{local}");
                }
                if kind != Kind::InsightsTip && settings.digest {
                    assert_eq!(pending, 0, "foldable standalone beside a guaranteed digest");
                }
                if let Some(last) = last_digest_at {
                    assert!(now - last >= COALESCE_AFTER_DIGEST);
                }
                let slot = Kind::ALL.iter().position(|k| *k == kind).unwrap();
                tally.standalones.push((kind, now));
                log.push(entry(kind, now, Route::Standalone));
                present[slot] = None;
            }
            if due {
                if let Some(kind) = out.fold {
                    tally.folds += 1;
                    log.push(entry(kind, now, Route::Folded));
                    let slot = Kind::ALL.iter().position(|k| *k == kind).unwrap();
                    present[slot] = None;
                }
                tally.digests.push(now);
                last_digest_at = Some(now);
                contributed = 0;
            }
            now += step;
        }
        tally
    }

    fn per_bucket<K: Ord>(times: impl Iterator<Item = K>) -> std::collections::BTreeMap<K, u32> {
        let mut m = std::collections::BTreeMap::new();
        for k in times {
            *m.entry(k).or_insert(0) += 1;
        }
        m
    }

    fn check_tally(sched: Sched, tally: &Tally) {
        let tz = chrono_tz::America::New_York;
        let days = per_bucket(tally.standalones.iter().map(|(_, t)| local_date(*t, &tz)));
        assert!(
            days.values().all(|n| *n <= STANDALONE_PER_DAY),
            "{sched:?} {days:?}"
        );
        let weeks = per_bucket(tally.standalones.iter().map(|(_, t)| local_week(*t, &tz)));
        assert!(
            weeks.values().all(|n| *n <= STANDALONE_PER_WEEK),
            "{sched:?} {weeks:?}"
        );
        for kind in [Kind::VerdictsLanded, Kind::InsightsTip] {
            let weeks = per_bucket(
                tally
                    .standalones
                    .iter()
                    .filter(|(k, _)| *k == kind)
                    .map(|(_, t)| local_week(*t, &tz)),
            );
            assert!(
                weeks.values().all(|n| *n <= 2),
                "{sched:?} {kind:?} {weeks:?}"
            );
        }
        // A standalone is an hour from every other notification, digest
        // included. Two digests close together (a missed evening answered
        // just before tonight's) are today's behaviour and not the
        // arbiter's to change.
        let mut all: Vec<(DateTime<Utc>, bool)> = tally
            .digests
            .iter()
            .map(|t| (*t, false))
            .chain(tally.standalones.iter().map(|(_, t)| (*t, true)))
            .collect();
        all.sort();
        for pair in all.windows(2) {
            if pair[0].1 || pair[1].1 {
                assert!(pair[1].0 - pair[0].0 >= MIN_GAP_ANY, "{sched:?} {pair:?}");
            }
        }
        // The weekly totals of section 4.2's table, as upper bounds.
        let digest_weeks = per_bucket(tally.digests.iter().map(|t| local_week(*t, &tz)));
        let digest_bound = match sched {
            Sched::Interval(h) => (168 / h) as u32 + 1,
            // One per evening target; a silent evening answered after
            // midnight belongs to the previous day's target.
            Sched::Evening(_) => 8,
            Sched::DigestOff | Sched::MasterOff => 0,
        };
        assert!(
            digest_weeks.values().all(|n| *n <= digest_bound),
            "{sched:?} {digest_weeks:?}"
        );
        if sched == Sched::MasterOff {
            assert!(tally.standalones.is_empty() && tally.digests.is_empty());
        }
    }

    #[test]
    fn eight_random_weeks_never_exceed_the_budget_at_any_schedule() {
        // Two starts: one spanning the March DST change, one the November.
        let starts = [at("2026-02-23T05:00:00Z"), at("2026-10-12T04:00:00Z")];
        for sched in SCHEDULES {
            let (mut standalones, mut folds) = (0, 0);
            for start in starts {
                for seed in [0x9E37_79B9_7F4A_7C15_u64, 0xD1B5_4A32_D192_ED03] {
                    let tally = simulate(sched, start, seed, false);
                    check_tally(sched, &tally);
                    standalones += tally.standalones.len();
                    folds += tally.folds;
                }
            }
            // Not vacuous: wherever the arbiter may speak, it did.
            if sched != Sched::MasterOff {
                assert!(standalones > 0, "{sched:?} never posted a standalone");
            }
            if !matches!(sched, Sched::DigestOff | Sched::MasterOff) {
                assert!(folds > 0, "{sched:?} never folded");
            }
        }
    }

    #[test]
    fn eight_saturated_weeks_reproduce_the_weekly_table() {
        // Monday 2026-01-05 00:00 EST: eight whole ISO weeks with no DST
        // change, so every week has 168 hours.
        let start = at("2026-01-05T05:00:00Z");
        let tz = chrono_tz::America::New_York;
        for (sched, digests, total_max) in [
            (Sched::Interval(4), 42, 45),
            (Sched::Interval(24), 7, 10),
            (Sched::Interval(1), 168, 171),
            (Sched::Evening(18), 7, 7),
            (Sched::DigestOff, 0, 3),
            (Sched::MasterOff, 0, 0),
        ] {
            let tally = simulate(sched, start, 1, true);
            check_tally(sched, &tally);
            let d = per_bucket(tally.digests.iter().map(|t| local_week(*t, &tz)));
            let s = per_bucket(tally.standalones.iter().map(|(_, t)| local_week(*t, &tz)));
            // Skip the first week: the first digest's alignment is arbitrary.
            for week in 2..=8u32 {
                let key = (2026, week);
                let dn = d.get(&key).copied().unwrap_or(0);
                let sn = s.get(&key).copied().unwrap_or(0);
                assert!(dn + sn <= total_max, "{sched:?} week {week}: {dn}+{sn}");
                assert_eq!(dn, digests, "{sched:?} week {week} digests");
                if sched == Sched::DigestOff {
                    // Every kind wants out: the global cap is what binds.
                    assert_eq!(sn, STANDALONE_PER_WEEK, "week {week}");
                }
            }
            assert!(tally.folds > 0 || digests == 0, "{sched:?}");
        }
    }
}
