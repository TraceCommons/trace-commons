//! Which in-app suggestion, if any, leads right now.
//!
//! A *nudge* is a suggestion drawn in the app -- a card on Traces or History,
//! a row in the menu-bar panel -- that brings a contributor back to the
//! decisions only they can make. This module decides which one leads, from
//! snapshots the caller has already taken. Whether anything *interrupts* is a
//! different question, answered by `attention::arbitrate`.
//!
//! This module is pure. It takes no locks, reads no files and logs nothing:
//! `lead` is a function of its inputs, a snapshot of the nudge ledger and the
//! clock, tested the way `notify::digest_due` is. The status builder takes
//! every snapshot before its policy and queue section, each in its own short
//! lock, so nothing here can add a lock ordering.
//!
//! Phase 1 has three kinds, highest first: U4 `idle_sessions` (Ask-me
//! sessions nobody has written to for [`IDLE_DAYS`], the primary trigger),
//! U1 `review_backlog` (previewed, undecided sessions in folders set to Ask
//! me) and U2 `verdicts_landed` (submissions that newly reached a verdict
//! since the daemon last looked). U4 and U1 both ask for decisions on the
//! same waiting sessions, so they share one "Not now".
//!
//! **Verdicts are diffed against a high-water mark the daemon owns.** The
//! history cache is shared: the CLI's `history` command and `note_uploads`
//! both write it, so a verdict can already be in the cache before
//! `refresh_history` sees it. [`verdict_delta`] therefore compares the cache
//! with `DaemonState::verdict_marks`, which only the daemon writes, and the
//! first poll after an upgrade (or after `unenroll`) seeds those marks
//! silently, so history that already existed never reads as news.
//!
//! **Shared gates.** Nothing leads while the daemon is paused, while the
//! consent gate holds the enrollment, while signed out, while unhealthy or
//! upload-blocked, or while suggestions are switched off. Decisions owed come
//! first: U1 is hidden while the arming offer is present, because "decide on
//! these" beside "arm this folder" is two asks at once.
//!
//! **The menu-bar mark** ([`mark`]) is decided here too, from the same
//! snapshots: `news` (a hollow ring below paused, only while nothing is
//! owed) for verdict news that is unacknowledged and younger than
//! [`NEWS_MARK_TTL`], and `ready` (a halo around the badge, only while
//! something is owed) for idle-session candidates. It clears by fact, never
//! because a panel or window was opened.
//!
//! **A clock that goes backwards can only suppress.** A "Not now" stamped
//! after `now` reads as still inside its cooldown, as `policy::arming_suggestion`
//! reads its own decline.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::history::{HistoryRecord, STATUS_ACCEPTED, STATUS_QUARANTINED, is_taken_back};
use super::policy::{self, ProjectMode, ProjectPolicy};

/// U1 leads once at least this many previewed, undecided Ask-me sessions are
/// waiting. Mirrors `policy::ARMING_SUGGESTION_THRESHOLD`. DRAFT, owner
/// decision 10 in the nudge decisions list.
pub const NUDGE_BACKLOG_THRESHOLD: usize = 5;

/// How long an in-app "Not now" silences a suggestion, before the queue TTL
/// caps it (see [`decline_cooldown`]). DRAFT, owner decision 10.
pub const NUDGE_DECLINE_COOLDOWN_DAYS: i64 = 7;

/// U2 reads `unknown` once the last successful history poll is older than
/// this many history poll intervals (`history_poll_secs`): a failed poll
/// serves the cache as it was, so a stale cache is never news. DRAFT, spec
/// section 3 ("Shared gates", history stale); not a numbered owner decision.
pub const HISTORY_STALE_POLL_MULTIPLE: i64 = 2;

/// U4: a waiting Ask-me session is an idle candidate once nobody has written
/// to it for this many days, before the queue TTL shortens it (see
/// [`idle_window`]). DRAFT, owner decision 26.
pub const IDLE_DAYS: i64 = 3;

/// U4: the longest the idle-session kind waits between two announcements
/// (`idle_repeat_days_eff`'s cap), before the queue TTL shortens it. DRAFT,
/// owner decisions 4 and 26.
pub const IDLE_REPEAT_DAYS: i64 = 7;

/// U4: the days `idle_repeat_days_eff` leaves between the threshold plus the
/// repeat interval and the queue TTL, so an announcement that slips a day
/// (quiet hours, the evening tick) still lands before its entry expires.
/// DRAFT, owner decision 26.
pub const IDLE_REPEAT_MARGIN_DAYS: i64 = 2;

/// U4: below this queue TTL the idle-session kind is off, because there is
/// no honest room to announce a candidate before it expires. DRAFT, owner
/// decision 26.
pub const IDLE_MIN_TTL_DAYS: i64 = 4;

/// U4: the idle threshold is at most the queue TTL divided by this, so a
/// short TTL still leaves room to announce before expiry
/// (`idle_days_eff = min(IDLE_DAYS, ttl / IDLE_TTL_DIVISOR)`). DRAFT, owner
/// decision 26.
pub const IDLE_TTL_DIVISOR: i64 = 4;

/// U4's effective threshold and repeat interval for one queue TTL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleWindow {
    /// `idle_days_eff`: days without a write before a session is a candidate.
    pub idle_days: i64,
    /// `idle_repeat_days_eff`: the kind's minimum interval between
    /// announcements.
    pub repeat_days: i64,
}

impl IdleWindow {
    /// The threshold as a duration.
    #[must_use]
    pub fn idle(self) -> Duration {
        Duration::days(self.idle_days)
    }

    /// The repeat interval as a duration: what the attention arbiter's N1
    /// cap (`attention::Kind::default_caps`) is given.
    #[must_use]
    pub fn repeat(self) -> Duration {
        Duration::days(self.repeat_days)
    }
}

/// U4's window for `queue_ttl_days`, or `None` when the kind is off (a TTL
/// below [`IDLE_MIN_TTL_DAYS`]):
///
/// - `idle_days_eff = min(IDLE_DAYS, max(1, ttl / IDLE_TTL_DIVISOR))`
/// - `idle_repeat_days_eff = min(IDLE_REPEAT_DAYS, max(1, ttl - idle_days_eff - IDLE_REPEAT_MARGIN_DAYS))`
///
/// so `idle_days_eff + idle_repeat_days_eff + 1 < ttl` always holds, and a
/// candidate is announced before its queue entry can expire.
#[must_use]
pub fn idle_window(queue_ttl_days: i64) -> Option<IdleWindow> {
    if queue_ttl_days < IDLE_MIN_TTL_DAYS {
        return None;
    }
    let idle_days = IDLE_DAYS.min((queue_ttl_days / IDLE_TTL_DIVISOR).max(1));
    let repeat_days =
        IDLE_REPEAT_DAYS.min((queue_ttl_days - idle_days - IDLE_REPEAT_MARGIN_DAYS).max(1));
    Some(IdleWindow {
        idle_days,
        repeat_days,
    })
}

/// Drop every announced entry id that is no longer pending, so the batching
/// set (`DaemonState::idle_announced`) never outgrows the queue and an id
/// can never be read back for a session that has left it. Returns whether
/// anything was dropped, so the caller saves only when it must.
pub fn prune_idle_announced(
    announced: &mut BTreeSet<uuid::Uuid>,
    pending: &BTreeSet<uuid::Uuid>,
) -> bool {
    let before = announced.len();
    announced.retain(|id| pending.contains(id));
    announced.len() != before
}

/// A suggestion kind. The wire, ledger keys and logs use [`NudgeKind::label`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NudgeKind {
    /// U4: Ask-me sessions nobody has written to for the idle threshold
    /// (`queue::idle_candidates`). The primary phase-1 trigger. Its "Not
    /// now" is U1's: the two ask for decisions on the same sessions.
    IdleSessions,
    /// U1: previewed, undecided sessions in folders set to Ask me
    /// (`queue::unpurposed_traces`). It has no notification kind of its own:
    /// the digest already covers that moment.
    ReviewBacklog,
    /// U2: submissions that newly reached accepted, held for privacy review
    /// or final credit since the daemon's high-water mark. Information, not
    /// an ask: it has no "Not now", and it retires when its own action
    /// acknowledges it (`nudge_opened`).
    VerdictsLanded,
}

impl NudgeKind {
    /// Every kind, highest precedence first.
    pub const ALL: [NudgeKind; 3] = [
        NudgeKind::IdleSessions,
        NudgeKind::ReviewBacklog,
        NudgeKind::VerdictsLanded,
    ];

    /// The stable label used on the wire, as a ledger key and in logs. A
    /// kind that is also a notification kind takes `attention::Kind::label`,
    /// so a ledger key and a notification key can never drift apart.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            NudgeKind::IdleSessions => super::attention::Kind::IdleSessions.label(),
            NudgeKind::ReviewBacklog => "review_backlog",
            NudgeKind::VerdictsLanded => super::attention::Kind::VerdictsLanded.label(),
        }
    }

    /// Whether an in-app "Not now" (`nudge_decline`) applies to this kind.
    /// U2 is news, not an ask, so it has none (spec section 3).
    #[must_use]
    pub fn declinable(self) -> bool {
        match self {
            NudgeKind::IdleSessions | NudgeKind::ReviewBacklog => true,
            NudgeKind::VerdictsLanded => false,
        }
    }

    /// The kind a label names, or `None` for a label this daemon does not
    /// know.
    #[must_use]
    pub fn parse(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.label() == label)
    }
}

/// One kind's (or one kind and subject's) bookkeeping. Times only; never a
/// path, a label or a count. Every field is `#[serde(default)]` so a later
/// field never makes an older state file unreadable.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NudgeLedger {
    /// When the kind last armed. Stamped by a later slice's tick, never by a
    /// `status` read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub armed_at: Option<DateTime<Utc>>,
    /// When the nudge's own action was last used (`nudge_opened`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_at: Option<DateTime<Utc>>,
    /// When the in-app "Not now" was last pressed (`nudge_decline`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declined_at: Option<DateTime<Utc>>,
    /// When the kind last retired by fact. Stamped by a later slice's tick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retired_at: Option<DateTime<Utc>>,
    /// When a notification last announced it. Stamped by the arbiter's
    /// caller in a later slice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub announced_at: Option<DateTime<Utc>>,
}

/// The ledger map's key for `kind`, optionally narrowed to one opaque
/// subject id (`kind:id`). Never a path or a label.
#[must_use]
pub fn ledger_key(kind: NudgeKind, subject: Option<&str>) -> String {
    match subject {
        Some(id) => format!("{}:{id}", kind.label()),
        None => kind.label().to_string(),
    }
}

/// How long an in-app "Not now" lasts: [`NUDGE_DECLINE_COOLDOWN_DAYS`],
/// capped at half the queue TTL so a declined suggestion always comes back
/// while the sessions it is about are still queued. Never negative.
///
/// The spec states both "7 days" and "shorter than `queue_ttl_days / 2`",
/// which cannot both hold at the default TTL of 14; this keeps the 7 and
/// pins "at most half the TTL, and always shorter than the TTL".
#[must_use]
pub fn decline_cooldown(queue_ttl_days: i64) -> Duration {
    let half_ttl_hours = queue_ttl_days.max(0).saturating_mul(12);
    Duration::days(NUDGE_DECLINE_COOLDOWN_DAYS).min(Duration::hours(half_ttl_hours))
}

/// What the daemon last saw of one submission: the high-water mark U2 is
/// diffed against. Flags only, never a status string, a label or a count.
/// A flag once set stays set while the submission is in the cache, so a
/// verdict that flips back and forth is news once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictMark {
    /// Reported `accepted`.
    #[serde(default)]
    pub accepted: bool,
    /// Reported `quarantined`: held for privacy review. Reported beside
    /// accepted, not instead of it (DRAFT, owner decision 2).
    #[serde(default)]
    pub held: bool,
    /// Carries a final credit figure.
    #[serde(default)]
    pub final_credit: bool,
}

/// Verdicts that landed and have not been acknowledged: counts and times,
/// and the opaque submission ids behind them. What `status.nudge` reports for
/// U2 (counts and `since` only) and what a later slice folds into a digest or
/// a notification.
///
/// The counts and times are a summary of [`Self::submissions`], kept in the
/// state file beside it, so that a poll can re-check each submission against
/// the cache it read (see [`after_history_poll`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerdictDelta {
    /// Submissions that newly reached `accepted`.
    #[serde(default)]
    pub newly_accepted: u32,
    /// Submissions newly held for privacy review (`quarantined`). Reported
    /// beside accepted ones (DRAFT, owner decision 2).
    #[serde(default)]
    pub newly_held: u32,
    /// Submissions whose credit newly became final.
    #[serde(default)]
    pub newly_final: u32,
    /// The final credit those submissions carry, summed from each one's
    /// figure as written ([`decimal_credit`]). On the wire as
    /// `status.nudge.credit_final`, through [`Self::credit_final_tenths`];
    /// never in a log line. An f64: a state file an f32 build wrote loads
    /// unchanged.
    #[serde(default)]
    pub credit_final_delta: f64,
    /// The poll that first saw a verdict in this delta: `status.nudge.since`.
    pub since: DateTime<Utc>,
    /// The poll that last added to this delta. The news mark ages out from
    /// it.
    pub newest_at: DateTime<Utc>,
    /// Each submission behind the counts, by submission id (opaque), with
    /// what it contributed. Never on the wire.
    ///
    /// A delta saved by a build that kept only the counts loads with this
    /// empty. Nothing can re-check such counts against the cache, so the
    /// next poll drops them rather than keep news lit that may have been
    /// taken back since; that build never shipped (only this change's own
    /// pre-release builds wrote one), so at most a pre-release tester's
    /// waiting news goes dark once.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub submissions: BTreeMap<String, PendingVerdict>,
}

/// One submission's part in a [`VerdictDelta`]: which of its verdicts are
/// news, and the polls that found them. Flags, a credit figure and times
/// only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingVerdict {
    #[serde(default)]
    pub accepted: bool,
    #[serde(default)]
    pub held: bool,
    #[serde(default)]
    pub final_credit: bool,
    /// The final credit, when `final_credit`. Never on the wire.
    #[serde(default)]
    pub credit_final: f32,
    /// The poll that first found news for this submission.
    pub since: DateTime<Utc>,
    /// The poll that last found news for it.
    pub newest_at: DateTime<Utc>,
}

impl PendingVerdict {
    /// Fold a later poll's news for the same submission into this one.
    fn absorb(&mut self, later: &PendingVerdict) {
        self.accepted |= later.accepted;
        self.held |= later.held;
        if later.final_credit && !self.final_credit {
            self.final_credit = true;
            self.credit_final = later.credit_final;
        }
        self.since = self.since.min(later.since);
        self.newest_at = self.newest_at.max(later.newest_at);
    }
}

impl VerdictDelta {
    /// Every verdict in the delta that is news. Credit becoming final is
    /// news only when there is a figure to say: finals whose credit rounds
    /// to zero would otherwise arm a card with nothing in it.
    #[must_use]
    pub fn total(&self) -> u32 {
        let finals = if self.credit_final_tenths().is_some() {
            self.newly_final
        } else {
            0
        };
        self.newly_accepted
            .saturating_add(self.newly_held)
            .saturating_add(finals)
    }

    /// The final credit in tenths, rounded half away from zero, or `None`
    /// when it rounds to zero: absent, never 0.
    ///
    /// Rounded in whole millionths first, so a sum that should be 0.35 but
    /// reads 0.34999999999999998 still gives 4 tenths, not 3.
    #[must_use]
    pub fn credit_final_tenths(&self) -> Option<u64> {
        let millionths = (self.credit_final_delta * 1e6).round() as i64;
        let tenths = millionths.saturating_add(50_000).div_euclid(100_000);
        u64::try_from(tenths).ok().filter(|t| *t >= 1)
    }

    /// The summary of `submissions`, or `None` when no submission carries
    /// news.
    #[must_use]
    pub fn from_submissions(submissions: BTreeMap<String, PendingVerdict>) -> Option<Self> {
        let count = |flag: fn(&PendingVerdict) -> bool| {
            u32::try_from(submissions.values().filter(|v| flag(v)).count()).unwrap_or(u32::MAX)
        };
        let delta = Self {
            newly_accepted: count(|v| v.accepted),
            newly_held: count(|v| v.held),
            newly_final: count(|v| v.final_credit),
            credit_final_delta: submissions
                .values()
                .map(|v| decimal_credit(v.credit_final))
                .sum(),
            since: submissions.values().map(|v| v.since).min()?,
            newest_at: submissions.values().map(|v| v.newest_at).max()?,
            submissions,
        };
        (delta.total() > 0).then_some(delta)
    }
}

/// A final credit figure as the decimal the server wrote: the f32's
/// shortest round-trip form, read back as an f64. Widening the f32 itself
/// turns 0.35 into 0.3499999940..., which rounds to 3 tenths.
fn decimal_credit(credit: f32) -> f64 {
    credit.to_string().parse().unwrap_or(f64::from(credit))
}

/// Whether a final credit figure is above zero in whole millionths, the
/// precision [`VerdictDelta::credit_final_tenths`] rounds through.
fn carries_credit(credit: f32) -> bool {
    (decimal_credit(credit) * 1e6).round() >= 1.0
}

/// The folders whose verdicts never count as news: every folder that now
/// resolves to Never, or all of them while a Never override is in force.
/// Opaque project ids only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NeverProjects {
    /// A Never contribution override is in force: every record is Never,
    /// including one whose project id cannot be resolved.
    pub all: bool,
    /// Project ids (`policy::project_id_for`) that resolve to Never.
    pub ids: BTreeSet<String>,
}

impl NeverProjects {
    /// Read from the policy over every project key the daemon knows
    /// (`policy::known_keys`). Takes references only; no lock.
    #[must_use]
    pub fn from_policy(policy: &ProjectPolicy, known_keys: &[String]) -> Self {
        let ids = std::iter::once(policy::UNKNOWN_PROJECT_KEY)
            .chain(known_keys.iter().map(String::as_str))
            .filter(|key| policy.resolve(key) == ProjectMode::Ignore)
            .map(policy::project_id_for)
            .collect();
        Self {
            all: policy.holds_every_send(),
            ids,
        }
    }

    /// Whether a record from `project_id` is a Never folder's.
    #[must_use]
    pub fn contains(&self, project_id: &str) -> bool {
        self.all || self.ids.contains(project_id)
    }
}

/// Diff the history cache against the high-water marks. Returns what newly
/// landed (`None` when nothing did) and the next marks: one per submission
/// still in `next`, so marks for ids gone from the cache are pruned.
///
/// Withdrawn or revoked records, and records from a Never folder, are never
/// counted, but their marks still advance, so a folder later set back to
/// Ask me does not replay its old verdicts. `rejected` is not news.
#[must_use]
pub fn verdict_delta(
    marks: &BTreeMap<String, VerdictMark>,
    next: &[HistoryRecord],
    never: &NeverProjects,
    now: DateTime<Utc>,
) -> (Option<VerdictDelta>, BTreeMap<String, VerdictMark>) {
    let mut found = BTreeMap::new();
    let mut next_marks = BTreeMap::new();
    for record in next {
        let key = record.submission_id.to_string();
        let before = marks.get(&key).copied().unwrap_or_default();
        let seen = VerdictMark {
            accepted: record.status == STATUS_ACCEPTED,
            held: record.status == STATUS_QUARANTINED,
            final_credit: record.credit_points_final.is_some(),
        };
        // Silent for a record that was taken back or belongs to a Never
        // folder, but its mark still advances below.
        if counts_as_news(record, never) {
            // A final with no credit is never news, and is not kept with
            // the news: kept, it would be counted all at once when a later
            // final's credit made the finals worth reporting.
            let paid = record.credit_points_final.is_some_and(carries_credit);
            let news = PendingVerdict {
                accepted: seen.accepted && !before.accepted,
                held: seen.held && !before.held,
                final_credit: seen.final_credit && !before.final_credit && paid,
                credit_final: 0.0,
                since: now,
                newest_at: now,
            };
            if news.accepted || news.held || news.final_credit {
                let credit_final = if news.final_credit {
                    record.credit_points_final.unwrap_or(0.0)
                } else {
                    0.0
                };
                found.insert(
                    key.clone(),
                    PendingVerdict {
                        credit_final,
                        ..news
                    },
                );
            }
        }
        next_marks.insert(
            key,
            VerdictMark {
                accepted: before.accepted || seen.accepted,
                held: before.held || seen.held,
                final_credit: before.final_credit || seen.final_credit,
            },
        );
    }
    (VerdictDelta::from_submissions(found), next_marks)
}

/// Whether a record's verdicts can be news: not taken back, and not in a
/// Never folder.
fn counts_as_news(record: &HistoryRecord, never: &NeverProjects) -> bool {
    !is_taken_back(record) && !never.contains(&record.project_id)
}

/// What one successful history poll does to the verdict bookkeeping.
#[derive(Debug, Clone, PartialEq)]
pub struct VerdictPoll {
    /// The next `verdict_marks`.
    pub marks: BTreeMap<String, VerdictMark>,
    /// The next `verdicts_pending`.
    pub pending: Option<VerdictDelta>,
    /// What this poll alone found, for `history_changed`. `None` when it
    /// found nothing, and always on the seeding poll.
    pub landed: Option<VerdictDelta>,
}

/// Apply one successful history poll. While `seeded` is false (the first
/// poll after an upgrade or after `unenroll`) the marks are seeded silently
/// and nothing lands. Afterwards what lands is added to `pending`.
///
/// Either way, what was already pending is re-checked against this poll's
/// cache first: a submission since withdrawn or revoked, in a folder now set
/// to Never, or gone from the cache drops out, so news that is no longer
/// news stops counting (and the mark goes dark when nothing is left). A
/// pending delta that names no submissions cannot be re-checked and is
/// dropped; see [`VerdictDelta::submissions`].
#[must_use]
pub fn after_history_poll(
    seeded: bool,
    marks: &BTreeMap<String, VerdictMark>,
    pending: Option<&VerdictDelta>,
    next: &[HistoryRecord],
    never: &NeverProjects,
    now: DateTime<Utc>,
) -> VerdictPoll {
    let (found, marks) = verdict_delta(marks, next, never, now);
    let landed = if seeded { found } else { None };
    let live: BTreeMap<String, &HistoryRecord> = next
        .iter()
        .map(|r| (r.submission_id.to_string(), r))
        .collect();
    let mut submissions = pending.map(|p| p.submissions.clone()).unwrap_or_default();
    submissions.retain(|id, _| live.get(id).is_some_and(|r| counts_as_news(r, never)));
    for (id, news) in landed.iter().flat_map(|d| &d.submissions) {
        match submissions.get_mut(id) {
            Some(earlier) => earlier.absorb(news),
            None => {
                submissions.insert(id.clone(), news.clone());
            }
        }
    }
    let pending = VerdictDelta::from_submissions(submissions);
    VerdictPoll {
        marks,
        pending,
        landed,
    }
}

/// Whether the last successful history poll is recent enough for U2 to be
/// read: no older than [`HISTORY_STALE_POLL_MULTIPLE`] poll intervals. No
/// poll yet is stale, and so is a poll stamped after `now` (the clock went
/// backwards): either way U2 reads `unknown`, which can only suppress.
#[must_use]
pub fn history_is_fresh(
    last_poll_at: Option<DateTime<Utc>>,
    history_poll_secs: u64,
    now: DateTime<Utc>,
) -> bool {
    let Some(last) = last_poll_at else {
        return false;
    };
    let age = now.signed_duration_since(last);
    let secs = i64::try_from(history_poll_secs).unwrap_or(i64::MAX);
    let bound = Duration::try_seconds(secs.saturating_mul(HISTORY_STALE_POLL_MULTIPLE))
        .unwrap_or(Duration::MAX);
    age >= Duration::zero() && age <= bound
}

/// Everything `lead` reads besides the ledger and the clock, as snapshots.
#[derive(Debug, Clone, PartialEq)]
pub struct LeadInputs {
    /// `status.paused`.
    pub paused: bool,
    /// `status.consent_hold` is non-null.
    pub consent_hold: bool,
    /// An enrollment is live (`status.logged_in`): config and device key.
    pub enrolled: bool,
    /// No health label is set and the daily budget is not blocking uploads:
    /// the same health that turns the menu-bar strip to attention.
    pub healthy: bool,
    /// The `suggestions_enabled` setting.
    pub suggestions_enabled: bool,
    /// `arming_suggestion` currently returns an offer.
    pub arming_offer_present: bool,
    /// `queue::unpurposed_traces`, or `None` when it could not be computed.
    pub unpurposed_traces: Option<usize>,
    /// The `queue_ttl_days` setting, which caps the decline cooldown.
    pub queue_ttl_days: i64,
    /// `DaemonState::last_history_poll_at`: when history was last read back.
    pub last_history_poll_at: Option<DateTime<Utc>>,
    /// The `history_poll_secs` setting, which [`history_is_fresh`] scales.
    pub history_poll_secs: u64,
    /// `DaemonState::verdicts_pending`: verdicts landed and unacknowledged.
    pub verdicts_pending: Option<VerdictDelta>,
    /// U4: how many entries `queue::idle_candidates` returns. Zero while the
    /// kind is off ([`idle_window`] is `None`).
    pub idle_candidates: usize,
}

/// Three-valued on purpose: a shell draws nothing for `Unknown` or `None`,
/// and `Unknown` is never read as "nothing to suggest".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NudgeState {
    /// The daemon cannot say: an input could not be computed, or the daemon
    /// is unhealthy and its view may not be current.
    Unknown,
    /// Nothing leads: a gate is closed, nothing qualifies, or a "Not now"
    /// is still in force.
    None,
    /// `lead` names the suggestion to draw.
    Armed,
}

impl NudgeState {
    /// The wire label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            NudgeState::Unknown => "unknown",
            NudgeState::None => "none",
            NudgeState::Armed => "armed",
        }
    }
}

/// What `lead` decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NudgeLead {
    pub state: NudgeState,
    /// Present exactly when `state` is `Armed`.
    pub lead: Option<NudgeKind>,
    /// The leading kind's count, present exactly when `lead` is.
    pub count: Option<usize>,
    /// When a "Not now" in force lapses; present only while one silences a
    /// kind that would otherwise lead, and nothing else leads.
    pub cooldown_until: Option<DateTime<Utc>>,
    /// U2's breakdown, present exactly when `lead` is `VerdictsLanded`.
    pub verdicts: Option<VerdictSummary>,
}

/// U2's counts for the card and the panel row, and the final credit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerdictSummary {
    pub accepted: u32,
    pub held: u32,
    /// How many submissions' credit became final.
    pub final_credit: u32,
    /// [`VerdictDelta::credit_final_tenths`].
    pub credit_final_tenths: Option<u64>,
    /// [`VerdictDelta::since`].
    pub since: DateTime<Utc>,
}

impl NudgeLead {
    fn quiet(state: NudgeState) -> Self {
        Self {
            state,
            lead: None,
            count: None,
            cooldown_until: None,
            verdicts: None,
        }
    }
}

/// Which suggestion leads now. Pure: see the module doc.
#[must_use]
pub fn lead(
    inputs: &LeadInputs,
    ledger: &BTreeMap<String, NudgeLedger>,
    now: DateTime<Utc>,
) -> NudgeLead {
    // Gates a person closed, or that close by themselves: nothing new may
    // appear, whatever the counts say.
    if inputs.paused || inputs.consent_hold || !inputs.enrolled || !inputs.suggestions_enabled {
        return NudgeLead::quiet(NudgeState::None);
    }
    // An unhealthy daemon's view may not be current, and a count it could not
    // compute is not zero.
    if !inputs.healthy {
        return NudgeLead::quiet(NudgeState::Unknown);
    }
    let Some(unpurposed) = inputs.unpurposed_traces else {
        return NudgeLead::quiet(NudgeState::Unknown);
    };
    // Decisions owed first: the arming offer outranks every suggestion.
    if inputs.arming_offer_present {
        return NudgeLead::quiet(NudgeState::None);
    }
    // U4, then U1. Both ask for decisions on the same waiting sessions, so
    // one "Not now" silences both, and lets U2 lead behind them.
    let asks = [
        (NudgeKind::IdleSessions, inputs.idle_candidates, 1),
        (
            NudgeKind::ReviewBacklog,
            unpurposed,
            NUDGE_BACKLOG_THRESHOLD,
        ),
    ];
    let mut silenced_until = None;
    if let Some((kind, count, _)) = asks.into_iter().find(|(_, n, at_least)| n >= at_least) {
        match shared_cooldown_until(inputs.queue_ttl_days, ledger, now) {
            Some(until) => silenced_until = Some(until),
            None => {
                return NudgeLead {
                    state: NudgeState::Armed,
                    lead: Some(kind),
                    count: Some(count),
                    cooldown_until: None,
                    verdicts: None,
                };
            }
        }
    }
    // U2 only from a fresh history poll: a stale cache is never news, and
    // "no news" from a stale cache is not known either.
    if !history_is_fresh(inputs.last_history_poll_at, inputs.history_poll_secs, now) {
        return NudgeLead {
            cooldown_until: silenced_until,
            ..NudgeLead::quiet(NudgeState::Unknown)
        };
    }
    match inputs.verdicts_pending.as_ref().filter(|d| d.total() > 0) {
        Some(news) => NudgeLead {
            state: NudgeState::Armed,
            lead: Some(NudgeKind::VerdictsLanded),
            count: Some(news.total() as usize),
            cooldown_until: None,
            verdicts: Some(VerdictSummary {
                accepted: news.newly_accepted,
                held: news.newly_held,
                // `final` is part of `count` exactly when `total` counts
                // it: never "count: 1, final: 2".
                final_credit: if news.credit_final_tenths().is_some() {
                    news.newly_final
                } else {
                    0
                },
                credit_final_tenths: news.credit_final_tenths(),
                since: news.since,
            }),
        },
        None => NudgeLead {
            cooldown_until: silenced_until,
            ..NudgeLead::quiet(NudgeState::None)
        },
    }
}

/// The news mark ages out this long after the newest verdict in the
/// unacknowledged news (`VerdictDelta::newest_at`). A later verdict re-arms
/// it. DRAFT, owner decision 19.
pub const NEWS_MARK_TTL: Duration = Duration::hours(72);

/// Everything [`mark`] reads besides the ledger and the clock, as snapshots.
/// The gates, the counts and the news are [`lead`]'s own inputs, so the
/// mark and the lead can never read two different worlds.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkInputs {
    /// What `lead` reads. `suggestions_enabled` governs the mark as well as
    /// the cards and the panel row: its copy says it covers the menu bar
    /// (OWNER DECISION 2026-10-08). `arming_offer_present` never governs the
    /// mark.
    pub lead: LeadInputs,
    /// `status.decisions_owed`, or `None` when it could not be computed.
    pub decisions_owed: Option<usize>,
    /// The `menu_bar_mark_enabled` setting: the mark's own switch, a finer
    /// control under `suggestions_enabled` and separate from every
    /// notification switch.
    pub menu_bar_mark_enabled: bool,
    /// The `notify.idle_sessions` setting. Muting the idle-session kind
    /// clears the halo too (spec section 4.1, "The halo").
    pub notify_idle_sessions: bool,
}

/// The menu-bar icon state the daemon reports. Four-valued: `News` and
/// `Ready` are the two lit states, and `Unknown` is never read as `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkState {
    /// The daemon cannot say: it is unhealthy, an input could not be
    /// computed, or (with nothing owed) the history poll is stale.
    Unknown,
    /// Nothing is lit.
    None,
    /// The news mark: something new to look at, nothing to decide. Only
    /// with `decisions_owed == 0`.
    News,
    /// The halo around the badge: some of the decisions owed are idle
    /// sessions. Only with `decisions_owed > 0`.
    Ready,
}

impl MarkState {
    /// The wire label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            MarkState::Unknown => "unknown",
            MarkState::None => "none",
            MarkState::News => "news",
            MarkState::Ready => "ready",
        }
    }
}

/// What [`mark`] decided: the state and the kinds that lit it, highest
/// precedence first (`attention::Kind::precedence`). `kinds` is empty
/// exactly when nothing is lit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub state: MarkState,
    pub kinds: Vec<super::attention::Kind>,
}

impl Mark {
    fn quiet(state: MarkState) -> Self {
        Self {
            state,
            kinds: Vec::new(),
        }
    }

    fn lit(state: MarkState, kind: super::attention::Kind) -> Self {
        Self {
            state,
            kinds: vec![kind],
        }
    }
}

/// The menu-bar icon state. Pure: see the module doc.
///
/// - `ready` (the halo) when decisions are owed, every gate is open, U4 has
///   at least one candidate, `notify.idle_sessions` is on and no in-app
///   "Not now" silences U4.
/// - `news` when nothing is owed, every gate is open, the history poll is
///   fresh, and verdict news is waiting unacknowledged and younger than
///   [`NEWS_MARK_TTL`]. Today U2 is the only mark kind; the weekly recap
///   and Insights tips register later.
///
/// One needs `decisions_owed > 0` and the other `== 0`, so the two can
/// never hold together. Nothing here reads whether a panel or window was
/// opened: the news clears only when `nudge_opened {verdicts_landed}` takes
/// `verdicts_pending`, or when it ages out.
#[must_use]
pub fn mark(
    inputs: &MarkInputs,
    ledger: &BTreeMap<String, NudgeLedger>,
    now: DateTime<Utc>,
) -> Mark {
    use super::attention::Kind;
    let gates = &inputs.lead;
    if gates.paused
        || gates.consent_hold
        || !gates.enrolled
        || !gates.suggestions_enabled
        || !inputs.menu_bar_mark_enabled
    {
        return Mark::quiet(MarkState::None);
    }
    if !gates.healthy {
        return Mark::quiet(MarkState::Unknown);
    }
    let Some(owed) = inputs.decisions_owed else {
        return Mark::quiet(MarkState::Unknown);
    };
    if owed > 0 {
        let ready = gates.idle_candidates > 0
            && inputs.notify_idle_sessions
            && shared_cooldown_until(gates.queue_ttl_days, ledger, now).is_none();
        return if ready {
            Mark::lit(MarkState::Ready, Kind::IdleSessions)
        } else {
            Mark::quiet(MarkState::None)
        };
    }
    // U2 only from a fresh history poll, as in `lead`.
    if !history_is_fresh(gates.last_history_poll_at, gates.history_poll_secs, now) {
        return Mark::quiet(MarkState::Unknown);
    }
    // News stamped after now (the clock went backwards) is not lit: it can
    // only suppress.
    let news = gates.verdicts_pending.as_ref().is_some_and(|d| {
        let age = now.signed_duration_since(d.newest_at);
        d.total() > 0 && age >= Duration::zero() && age < NEWS_MARK_TTL
    });
    if news {
        Mark::lit(MarkState::News, Kind::VerdictsLanded)
    } else {
        Mark::quiet(MarkState::None)
    }
}

/// When the "Not now" in force against U4 and U1 lapses, or `None` when
/// none is in force. The two share it: a decline of either counts, and the
/// later decline governs. A decline stamped after `now` is in force: a clock
/// that went backwards can only suppress.
fn shared_cooldown_until(
    queue_ttl_days: i64,
    ledger: &BTreeMap<String, NudgeLedger>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let declined = [NudgeKind::IdleSessions, NudgeKind::ReviewBacklog]
        .into_iter()
        .filter_map(|kind| ledger.get(&ledger_key(kind, None))?.declined_at)
        .max()?;
    let cooldown = decline_cooldown(queue_ttl_days);
    (now.signed_duration_since(declined) < cooldown).then(|| declined + cooldown)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-07T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    /// Every gate open, the arming offer absent, and a backlog at the
    /// threshold: U1 leads.
    fn open() -> LeadInputs {
        LeadInputs {
            paused: false,
            consent_hold: false,
            enrolled: true,
            healthy: true,
            suggestions_enabled: true,
            arming_offer_present: false,
            unpurposed_traces: Some(NUDGE_BACKLOG_THRESHOLD),
            queue_ttl_days: 14,
            last_history_poll_at: Some(now() - Duration::minutes(5)),
            history_poll_secs: 1800,
            verdicts_pending: None,
            idle_candidates: 0,
        }
    }

    fn empty() -> BTreeMap<String, NudgeLedger> {
        BTreeMap::new()
    }

    fn declined(at: DateTime<Utc>) -> BTreeMap<String, NudgeLedger> {
        BTreeMap::from([(
            ledger_key(NudgeKind::ReviewBacklog, None),
            NudgeLedger {
                declined_at: Some(at),
                ..NudgeLedger::default()
            },
        )])
    }

    #[test]
    fn a_backlog_at_the_threshold_leads_with_its_count() {
        let got = lead(&open(), &empty(), now());
        assert_eq!(got.state, NudgeState::Armed);
        assert_eq!(got.lead, Some(NudgeKind::ReviewBacklog));
        assert_eq!(got.count, Some(NUDGE_BACKLOG_THRESHOLD));
        assert_eq!(got.cooldown_until, None);
    }

    #[test]
    fn a_backlog_below_the_threshold_leads_nothing() {
        let inputs = LeadInputs {
            unpurposed_traces: Some(NUDGE_BACKLOG_THRESHOLD - 1),
            ..open()
        };
        assert_eq!(
            lead(&inputs, &empty(), now()),
            NudgeLead::quiet(NudgeState::None)
        );
    }

    #[test]
    fn an_uncomputable_count_is_unknown_never_none() {
        let inputs = LeadInputs {
            unpurposed_traces: None,
            ..open()
        };
        assert_eq!(
            lead(&inputs, &empty(), now()),
            NudgeLead::quiet(NudgeState::Unknown)
        );
    }

    /// Paused, consent-held, signed out or switched off: nothing new
    /// appears, and the reply carries no count either.
    #[test]
    fn a_closed_gate_leads_nothing() {
        let gates: [(&str, LeadInputs); 4] = [
            (
                "paused",
                LeadInputs {
                    paused: true,
                    ..open()
                },
            ),
            (
                "consent_hold",
                LeadInputs {
                    consent_hold: true,
                    ..open()
                },
            ),
            (
                "unenrolled",
                LeadInputs {
                    enrolled: false,
                    ..open()
                },
            ),
            (
                "suggestions off",
                LeadInputs {
                    suggestions_enabled: false,
                    ..open()
                },
            ),
        ];
        for (name, inputs) in gates {
            assert_eq!(
                lead(&inputs, &empty(), now()),
                NudgeLead::quiet(NudgeState::None),
                "{name}"
            );
        }
    }

    /// An unhealthy daemon's view may not be current, so it is `unknown`,
    /// and like every closed gate it names no lead and no count.
    #[test]
    fn an_unhealthy_daemon_is_unknown() {
        let inputs = LeadInputs {
            healthy: false,
            ..open()
        };
        assert_eq!(
            lead(&inputs, &empty(), now()),
            NudgeLead::quiet(NudgeState::Unknown)
        );
    }

    /// A gate outranks an uncomputable count too: paused and unknown is
    /// still never armed.
    #[test]
    fn no_gate_combination_ever_arms() {
        for bits in 0u8..32 {
            let inputs = LeadInputs {
                paused: bits & 1 != 0,
                consent_hold: bits & 2 != 0,
                enrolled: bits & 4 == 0,
                healthy: bits & 8 == 0,
                suggestions_enabled: bits & 16 == 0,
                ..open()
            };
            let got = lead(&inputs, &empty(), now());
            if bits == 0 {
                assert_eq!(got.state, NudgeState::Armed);
            } else {
                assert_ne!(got.state, NudgeState::Armed, "gate bits {bits:05b}");
                assert_eq!(got.lead, None);
                assert_eq!(got.count, None);
            }
        }
    }

    /// Spec section 3: if `arming_suggestion` returns an offer, U1 is
    /// hidden; the arming offer takes priority.
    #[test]
    fn the_arming_offer_hides_the_backlog() {
        let inputs = LeadInputs {
            arming_offer_present: true,
            unpurposed_traces: Some(NUDGE_BACKLOG_THRESHOLD * 10),
            ..open()
        };
        assert_eq!(
            lead(&inputs, &empty(), now()),
            NudgeLead::quiet(NudgeState::None)
        );
    }

    #[test]
    fn not_now_silences_the_backlog_until_the_cooldown_lapses() {
        let cooldown = decline_cooldown(14);
        let at = now() - Duration::days(1);
        let got = lead(&open(), &declined(at), now());
        assert_eq!(got.state, NudgeState::None);
        assert_eq!(got.lead, None);
        assert_eq!(got.count, None);
        assert_eq!(got.cooldown_until, Some(at + cooldown));

        let lapsed = lead(&open(), &declined(at), at + cooldown);
        assert_eq!(lapsed.state, NudgeState::Armed);
        assert_eq!(lapsed.cooldown_until, None);
    }

    /// A cooldown that would silence nothing is not reported: there is no
    /// lead for it to hide.
    #[test]
    fn a_cooldown_with_nothing_to_silence_is_not_reported() {
        let inputs = LeadInputs {
            unpurposed_traces: Some(0),
            ..open()
        };
        let got = lead(&inputs, &declined(now()), now());
        assert_eq!(got, NudgeLead::quiet(NudgeState::None));
    }

    /// A "Not now" stamped in the future (the clock went backwards since)
    /// reads as still inside the cooldown: it can only suppress.
    #[test]
    fn a_backwards_clock_only_suppresses() {
        let future = now() + Duration::days(30);
        let got = lead(&open(), &declined(future), now());
        assert_eq!(got.state, NudgeState::None);
        assert_eq!(got.lead, None);
        // And it never makes a suggestion appear that would not otherwise.
        let below = LeadInputs {
            unpurposed_traces: Some(0),
            ..open()
        };
        assert_eq!(
            lead(&below, &declined(future), now()).state,
            NudgeState::None
        );
    }

    /// A ledger entry for a subject (`kind:id`) is not the kind's own entry:
    /// it never silences the kind as a whole.
    #[test]
    fn a_subject_entry_does_not_silence_the_kind() {
        let ledger = BTreeMap::from([(
            ledger_key(NudgeKind::ReviewBacklog, Some("abc")),
            NudgeLedger {
                declined_at: Some(now()),
                ..NudgeLedger::default()
            },
        )]);
        assert_eq!(lead(&open(), &ledger, now()).state, NudgeState::Armed);
    }

    /// Owner decision 10: the cooldown is at most half the queue TTL and
    /// always shorter than the TTL, over every TTL a person can set and a
    /// few they cannot.
    #[test]
    fn the_cooldown_is_at_most_half_the_queue_ttl() {
        assert_eq!(decline_cooldown(14), Duration::days(7));
        for ttl in -1..=60i64 {
            let cooldown = decline_cooldown(ttl);
            assert!(cooldown >= Duration::zero(), "ttl {ttl}");
            assert!(
                cooldown <= Duration::days(NUDGE_DECLINE_COOLDOWN_DAYS),
                "ttl {ttl}"
            );
            if ttl > 0 {
                assert!(cooldown * 2 <= Duration::days(ttl), "ttl {ttl}");
                assert!(cooldown < Duration::days(ttl), "ttl {ttl}");
            }
        }
    }

    #[test]
    fn labels_round_trip_and_keys_carry_no_path() {
        for kind in NudgeKind::ALL {
            assert_eq!(NudgeKind::parse(kind.label()), Some(kind));
        }
        assert_eq!(NudgeKind::parse("/Users/x/project"), None);
        assert_eq!(ledger_key(NudgeKind::ReviewBacklog, None), "review_backlog");
        assert_eq!(
            ledger_key(NudgeKind::ReviewBacklog, Some("p1")),
            "review_backlog:p1"
        );
    }

    #[test]
    fn an_empty_ledger_entry_serializes_to_nothing_and_reads_back() {
        let entry = NudgeLedger::default();
        assert_eq!(serde_json::to_string(&entry).unwrap(), "{}");
        let back: NudgeLedger = serde_json::from_str("{}").unwrap();
        assert_eq!(back, entry);
        // A field a later version adds is ignored by this one, not refused.
        let later: NudgeLedger =
            serde_json::from_str(r#"{"declined_at":"2026-10-07T12:00:00Z","later":1}"#).unwrap();
        assert_eq!(later.declined_at, Some(now()));
    }

    // ---- U2: verdict news ----

    use crate::daemon::history::{STATUS_REJECTED, STATUS_REVOKED, STATUS_SUBMITTED};

    fn sid(n: u8) -> uuid::Uuid {
        uuid::Uuid::from_bytes([n; 16])
    }

    fn rec(n: u8, status: &str) -> HistoryRecord {
        HistoryRecord {
            submission_id: sid(n),
            submitted_at: now() - Duration::days(1),
            project_id: "p_ask".to_string(),
            project_label: "ask".to_string(),
            source: "claude".to_string(),
            session_hash: format!("h{n}"),
            status: status.to_string(),
            consent_scopes: vec![],
            credit_points_pending: 1.0,
            credit_points_final: None,
            explanations: vec![],
            last_refreshed_at: Some(now()),
            withdrawn_at: None,
            revoked_at: None,
            approved_unattended: Some(false),
            approved_verdict: None,
            uploaded_bytes: None,
        }
    }

    fn final_rec(n: u8, status: &str, credit: f32) -> HistoryRecord {
        HistoryRecord {
            credit_points_final: Some(credit),
            ..rec(n, status)
        }
    }

    fn no_never() -> NeverProjects {
        NeverProjects::default()
    }

    /// A delta with counts only, as `lead` and `mark` read it; it names no
    /// submissions, so a poll would drop it.
    fn delta(a: u32, h: u32, f: u32, at: DateTime<Utc>) -> VerdictDelta {
        VerdictDelta {
            newly_accepted: a,
            newly_held: h,
            newly_final: f,
            credit_final_delta: 0.0,
            since: at,
            newest_at: at,
            submissions: BTreeMap::new(),
        }
    }

    /// Accepted, held and final each count once, against the marks; a
    /// submission still waiting, or rejected, is not news.
    #[test]
    fn verdict_delta_counts_accepted_held_and_final_transitions() {
        let marks = BTreeMap::new();
        let next = [
            rec(1, STATUS_ACCEPTED),
            rec(2, STATUS_QUARANTINED),
            final_rec(3, STATUS_ACCEPTED, 2.5),
            rec(4, STATUS_SUBMITTED),
            rec(5, STATUS_REJECTED),
        ];
        let (got, marks) = verdict_delta(&marks, &next, &no_never(), now());
        let got = got.expect("verdicts landed");
        assert_eq!(got.newly_accepted, 2);
        assert_eq!(got.newly_held, 1);
        assert_eq!(got.newly_final, 1);
        assert!((got.credit_final_delta - 2.5).abs() < f64::EPSILON);
        assert_eq!(got.since, now());
        assert_eq!(got.newest_at, now());
        assert_eq!(marks.len(), 5, "every record is marked: {marks:?}");

        // The same cache again is nothing new.
        let (again, same) = verdict_delta(&marks, &next, &no_never(), now());
        assert_eq!(again, None);
        assert_eq!(same, marks);

        // Credit becoming final on an already-accepted submission is new,
        // and only that.
        let later = [final_rec(1, STATUS_ACCEPTED, 1.0)];
        let (fin, _) = verdict_delta(&marks, &later, &no_never(), now());
        let fin = fin.expect("final landed");
        assert_eq!(
            (fin.newly_accepted, fin.newly_held, fin.newly_final),
            (0, 0, 1)
        );
    }

    /// Held then released for acceptance is two pieces of news; accepted
    /// then held again and back is not a third.
    #[test]
    fn a_mark_is_a_high_water_mark() {
        let (_, marks) = verdict_delta(
            &BTreeMap::new(),
            &[rec(1, STATUS_QUARANTINED)],
            &no_never(),
            now(),
        );
        let (released, marks) =
            verdict_delta(&marks, &[rec(1, STATUS_ACCEPTED)], &no_never(), now());
        assert_eq!(released.map(|d| d.newly_accepted), Some(1));
        let (held_again, marks) =
            verdict_delta(&marks, &[rec(1, STATUS_QUARANTINED)], &no_never(), now());
        assert_eq!(held_again, None);
        let (back, _) = verdict_delta(&marks, &[rec(1, STATUS_ACCEPTED)], &no_never(), now());
        assert_eq!(back, None);
    }

    /// Withdrawn and revoked records, and a Never folder's, never count, but
    /// their marks advance, so setting the folder back later does not replay
    /// them.
    #[test]
    fn withdrawn_and_never_records_count_nothing_and_never_replay() {
        let withdrawn = HistoryRecord {
            withdrawn_at: Some(now()),
            ..rec(1, STATUS_ACCEPTED)
        };
        let revoked = rec(2, STATUS_REVOKED);
        let never_rec = HistoryRecord {
            project_id: "p_never".to_string(),
            ..rec(3, STATUS_ACCEPTED)
        };
        let never = NeverProjects {
            all: false,
            ids: BTreeSet::from(["p_never".to_string()]),
        };
        let next = [withdrawn, revoked, never_rec.clone()];
        let (got, marks) = verdict_delta(&BTreeMap::new(), &next, &never, now());
        assert_eq!(got, None, "a Never folder's verdicts give 0");
        assert!(marks[&sid(3).to_string()].accepted, "{marks:?}");

        // The folder is set back to Ask me: its old verdict stays old.
        let (replayed, _) = verdict_delta(&marks, &[never_rec], &no_never(), now());
        assert_eq!(replayed, None);
    }

    /// A Never override covers every record, including one whose project id
    /// predates the field and cannot be resolved.
    #[test]
    fn a_never_override_covers_every_record() {
        let never = NeverProjects {
            all: true,
            ids: BTreeSet::new(),
        };
        let unresolved = HistoryRecord {
            project_id: String::new(),
            ..rec(1, STATUS_ACCEPTED)
        };
        assert!(never.contains(""));
        let (got, _) = verdict_delta(
            &BTreeMap::new(),
            std::slice::from_ref(&unresolved),
            &never,
            now(),
        );
        assert_eq!(got, None);
        // Without the override an unresolvable id is not Never: it counts.
        let (counted, _) = verdict_delta(&BTreeMap::new(), &[unresolved], &no_never(), now());
        assert_eq!(counted.map(|d| d.newly_accepted), Some(1));
    }

    /// Marks for submissions no longer in the cache are pruned.
    #[test]
    fn marks_are_pruned_to_the_cache() {
        let (_, marks) = verdict_delta(
            &BTreeMap::new(),
            &[rec(1, STATUS_ACCEPTED), rec(2, STATUS_ACCEPTED)],
            &no_never(),
            now(),
        );
        let (_, pruned) = verdict_delta(&marks, &[rec(2, STATUS_ACCEPTED)], &no_never(), now());
        assert_eq!(
            pruned.keys().cloned().collect::<Vec<_>>(),
            vec![sid(2).to_string()]
        );
    }

    /// The first poll after an upgrade (or unenroll) seeds silently:
    /// existing history never fires.
    #[test]
    fn the_first_poll_seeds_silently() {
        let next = [rec(1, STATUS_ACCEPTED), rec(2, STATUS_QUARANTINED)];
        let poll = after_history_poll(false, &BTreeMap::new(), None, &next, &no_never(), now());
        assert_eq!(poll.landed, None);
        assert_eq!(poll.pending, None);
        assert_eq!(poll.marks.len(), 2);
        // The next poll of the same cache is still nothing.
        let again = after_history_poll(true, &poll.marks, None, &next, &no_never(), now());
        assert_eq!(again.landed, None);
        assert_eq!(again.pending, None);
    }

    /// A verdict the CLI (or `note_uploads`) already saved into the cache is
    /// still news, exactly once: the diff is against the daemon's marks, not
    /// against the cache the daemon replaced.
    #[test]
    fn a_verdict_already_in_the_cache_fires_once() {
        let seeded = after_history_poll(
            false,
            &BTreeMap::new(),
            None,
            &[rec(1, STATUS_SUBMITTED)],
            &no_never(),
            now() - Duration::hours(1),
        );
        // The CLI's `history` joined the verdict into the cache in between;
        // the daemon's next poll reads the same cache back.
        let cache = [rec(1, STATUS_ACCEPTED)];
        let first = after_history_poll(true, &seeded.marks, None, &cache, &no_never(), now());
        assert_eq!(first.landed.as_ref().map(|d| d.newly_accepted), Some(1));
        assert_eq!(first.pending, first.landed);
        let second = after_history_poll(
            true,
            &first.marks,
            first.pending.as_ref(),
            &cache,
            &no_never(),
            now() + Duration::minutes(30),
        );
        assert_eq!(second.landed, None, "never twice");
        assert_eq!(second.pending, first.pending, "still unacknowledged");
    }

    /// Unacknowledged news accumulates: counts add, `since` keeps the first
    /// poll and `newest_at` takes the last.
    #[test]
    fn unacknowledged_news_accumulates() {
        let earlier = now() - Duration::hours(2);
        let first = after_history_poll(
            true,
            &BTreeMap::new(),
            None,
            &[rec(1, STATUS_ACCEPTED)],
            &no_never(),
            earlier,
        );
        let poll = after_history_poll(
            true,
            &first.marks,
            first.pending.as_ref(),
            &[rec(1, STATUS_ACCEPTED), rec(2, STATUS_QUARANTINED)],
            &no_never(),
            now(),
        );
        let got = poll.pending.expect("pending");
        assert_eq!((got.newly_accepted, got.newly_held), (1, 1));
        assert_eq!(got.since, earlier);
        assert_eq!(got.newest_at, now());
        assert_eq!(poll.landed.map(|d| d.newly_held), Some(1));
    }

    /// News pending from earlier polls is re-checked against every poll's
    /// cache: a submission since withdrawn, revoked, or in a folder now set
    /// to Never drops out, whichever way it was taken back.
    #[test]
    fn pending_news_drops_submissions_taken_back_or_set_to_never() {
        let earlier = now() - Duration::hours(1);
        let landed = after_history_poll(
            true,
            &BTreeMap::new(),
            None,
            &[rec(1, STATUS_ACCEPTED), rec(2, STATUS_QUARANTINED)],
            &no_never(),
            earlier,
        );
        let pending = landed.pending.clone().expect("two landed");
        assert_eq!(pending.total(), 2);
        let never_p_ask = NeverProjects {
            all: false,
            ids: BTreeSet::from(["p_ask".to_string()]),
        };
        let withdrawn = HistoryRecord {
            withdrawn_at: Some(now()),
            ..rec(1, STATUS_ACCEPTED)
        };
        let never_other = NeverProjects {
            all: false,
            ids: BTreeSet::from(["p_other".to_string()]),
        };
        let moved = HistoryRecord {
            project_id: "p_other".to_string(),
            ..rec(1, STATUS_ACCEPTED)
        };
        for (case, cache, never) in [
            (
                "withdrawn",
                vec![withdrawn, rec(2, STATUS_QUARANTINED)],
                no_never(),
            ),
            (
                "revoked",
                vec![rec(1, STATUS_REVOKED), rec(2, STATUS_QUARANTINED)],
                no_never(),
            ),
            (
                "never",
                vec![moved, rec(2, STATUS_QUARANTINED)],
                never_other,
            ),
        ] {
            let poll =
                after_history_poll(true, &landed.marks, Some(&pending), &cache, &never, now());
            let left = poll.pending.expect(case);
            assert_eq!(
                (left.newly_accepted, left.newly_held, left.total()),
                (0, 1, 1),
                "{case}"
            );
            assert_eq!(left.since, earlier, "{case}");
            assert_eq!(poll.landed, None, "{case}: nothing new landed");
        }
        // Everything taken back: nothing is pending, and the mark goes dark.
        let gone = after_history_poll(
            true,
            &landed.marks,
            Some(&pending),
            &[rec(1, STATUS_ACCEPTED), rec(2, STATUS_QUARANTINED)],
            &never_p_ask,
            now(),
        );
        assert_eq!(gone.pending, None);
        let before = mark(&mark_open(with_verdicts(pending)), &empty(), now());
        assert_eq!(before.state, MarkState::News);
        let after = mark(
            &mark_open(LeadInputs {
                unpurposed_traces: Some(0),
                verdicts_pending: gone.pending,
                ..open()
            }),
            &empty(),
            now(),
        );
        assert_eq!(after, quiet_mark(MarkState::None));
    }

    /// A pending delta from a build that kept only counts names no
    /// submission, so no poll can re-check it: the next poll drops it
    /// rather than keep the mark lit on counts nothing can verify.
    #[test]
    fn a_pending_delta_naming_no_submissions_is_dropped_by_the_next_poll() {
        let legacy = delta(2, 0, 0, now() - Duration::hours(1));
        let state: crate::daemon::state::DaemonState = serde_json::from_value(serde_json::json!({
            "schema_version": crate::daemon::state::DAEMON_STATE_SCHEMA,
            "cwd_cache": {}, "prior_uploads": {}, "last_observation": {},
            "last_digest_at": null, "day_bucket": null,
            "uploads_today": 0, "bytes_today": 0,
            "verdicts_pending": serde_json::to_value(&legacy).unwrap(),
        }))
        .expect("an older state file still loads");
        assert_eq!(
            state.verdicts_pending.as_ref().map(VerdictDelta::total),
            Some(2)
        );
        let poll = after_history_poll(
            true,
            &BTreeMap::new(),
            state.verdicts_pending.as_ref(),
            &[rec(1, STATUS_ACCEPTED)],
            &no_never(),
            now(),
        );
        // Rec 1 is new news on its own account; the two legacy counts are gone.
        assert_eq!(poll.pending.map(|d| d.total()), Some(1));
    }

    #[test]
    fn history_freshness_has_a_bound_and_fails_closed() {
        let interval = 1800u64;
        let bound = Duration::seconds(interval as i64 * HISTORY_STALE_POLL_MULTIPLE);
        assert!(history_is_fresh(Some(now()), interval, now()));
        assert!(history_is_fresh(Some(now() - bound), interval, now()));
        assert!(!history_is_fresh(
            Some(now() - bound - Duration::seconds(1)),
            interval,
            now()
        ));
        assert!(!history_is_fresh(None, interval, now()), "never polled");
        assert!(
            !history_is_fresh(Some(now() + Duration::minutes(1)), interval, now()),
            "a poll after now: the clock went backwards"
        );
    }

    fn with_verdicts(pending: VerdictDelta) -> LeadInputs {
        LeadInputs {
            unpurposed_traces: Some(0),
            verdicts_pending: Some(pending),
            ..open()
        }
    }

    #[test]
    fn verdicts_lead_when_the_backlog_does_not() {
        let at = now() - Duration::hours(1);
        let news = VerdictDelta {
            credit_final_delta: 2.25,
            ..delta(2, 1, 1, at)
        };
        let got = lead(&with_verdicts(news), &empty(), now());
        assert_eq!(got.state, NudgeState::Armed);
        assert_eq!(got.lead, Some(NudgeKind::VerdictsLanded));
        assert_eq!(got.count, Some(4));
        assert_eq!(
            got.verdicts,
            Some(VerdictSummary {
                accepted: 2,
                held: 1,
                final_credit: 1,
                credit_final_tenths: Some(23),
                since: at,
            })
        );
        assert_eq!(got.cooldown_until, None);
    }

    /// The final credit is reported to one decimal, half away from zero,
    /// and only when it says something: under 0.05 it is absent, never 0.
    #[test]
    fn the_final_credit_is_tenths_and_absent_when_it_rounds_to_zero() {
        let at = now() - Duration::hours(1);
        for (credit, want) in [
            (0.0, None),
            (0.04, None),
            (0.05, Some(1)),
            (0.35, Some(4)),
            (1.0, Some(10)),
            (1.05, Some(11)),
            (2.25, Some(23)),
        ] {
            let news = VerdictDelta {
                credit_final_delta: credit,
                ..delta(1, 0, 1, at)
            };
            let got = lead(&with_verdicts(news), &empty(), now());
            assert_eq!(
                got.verdicts.map(|v| v.credit_final_tenths),
                Some(want),
                "{credit}"
            );
        }
    }

    /// One submission's final credit, as a poll finds it.
    fn final_news(credit: f32) -> PendingVerdict {
        PendingVerdict {
            accepted: false,
            held: false,
            final_credit: true,
            credit_final: credit,
            since: now(),
            newest_at: now(),
        }
    }

    /// The server's figure arrives as an f32, which holds 0.35 as
    /// 0.3499999940...; widened and rounded as it stands, 40 of the 100
    /// figures x.x5 up to 9.95 rounded down (poldsam's #1298 review). Each
    /// rounds half away from zero, from one submission or summed from two.
    #[test]
    fn a_final_credit_ending_in_five_rounds_up() {
        for whole in 0..10u64 {
            for tenth in 0..10u64 {
                let credit: f32 = format!("{whole}.{tenth}5").parse().unwrap();
                let submissions = BTreeMap::from([("a".to_string(), final_news(credit))]);
                let got = VerdictDelta::from_submissions(submissions).expect("news");
                assert_eq!(
                    got.credit_final_tenths(),
                    Some(whole * 10 + tenth + 1),
                    "{credit}"
                );
            }
        }
        let two = BTreeMap::from([
            ("a".to_string(), final_news(0.35)),
            ("b".to_string(), final_news(0.7)),
        ]);
        let got = VerdictDelta::from_submissions(two).expect("news");
        assert_eq!(got.credit_final_tenths(), Some(11), "0.35 + 0.7");
    }

    /// Credit becoming final with nothing to say is not news: a delta of
    /// finals only, whose credit rounds to zero, arms nothing and lights
    /// nothing. Accepted or held beside it still is news, and the zero
    /// finals do not add to the count.
    #[test]
    fn finals_with_no_credit_are_not_news() {
        let at = now() - Duration::hours(1);
        let empty_finals = delta(0, 0, 2, at);
        assert_eq!(empty_finals.total(), 0);
        let got = lead(&with_verdicts(empty_finals.clone()), &empty(), now());
        assert_eq!(got.lead, None);
        let mut news = news_inputs(at);
        news.lead.verdicts_pending = Some(empty_finals);
        assert_eq!(mark(&news, &empty(), now()).state, MarkState::None);

        let with_accepted = delta(1, 0, 2, at);
        assert_eq!(with_accepted.total(), 1);
        // On the wire too: `count: 1` goes out with `final: 0`, not 2.
        let got = lead(&with_verdicts(with_accepted), &empty(), now());
        assert_eq!(got.count, Some(1));
        assert_eq!(got.verdicts.map(|v| v.final_credit), Some(0));
        let paid = VerdictDelta {
            credit_final_delta: 3.0,
            ..delta(0, 0, 2, at)
        };
        assert_eq!(paid.total(), 2, "finals with credit are news");
    }

    /// Zero-credit finals are not kept as news, so a later final whose
    /// credit says something counts once, not once for every zero final
    /// that waited beside it (poldsam's #1298 review, finding 4).
    #[test]
    fn a_later_final_does_not_count_the_zero_finals_before_it() {
        let seed = [
            rec(1, STATUS_ACCEPTED),
            rec(2, STATUS_ACCEPTED),
            rec(3, STATUS_ACCEPTED),
        ];
        let seeded = after_history_poll(false, &BTreeMap::new(), None, &seed, &no_never(), now());
        let first = [
            rec(1, STATUS_ACCEPTED),
            final_rec(2, STATUS_ACCEPTED, 0.0),
            final_rec(3, STATUS_ACCEPTED, 0.0),
            rec(4, STATUS_ACCEPTED),
        ];
        let one = after_history_poll(
            true,
            &seeded.marks,
            seeded.pending.as_ref(),
            &first,
            &no_never(),
            now(),
        );
        let pending = one.pending.clone().expect("the accepted one is news");
        assert_eq!(pending.total(), 1);
        let second = [
            final_rec(1, STATUS_ACCEPTED, 0.5),
            final_rec(2, STATUS_ACCEPTED, 0.0),
            final_rec(3, STATUS_ACCEPTED, 0.0),
            rec(4, STATUS_ACCEPTED),
        ];
        let two = after_history_poll(
            true,
            &one.marks,
            one.pending.as_ref(),
            &second,
            &no_never(),
            now(),
        );
        let pending = two.pending.expect("news");
        assert_eq!(pending.newly_final, 1);
        assert_eq!(pending.total(), 2, "one accepted and one paid final");
        let got = lead(&with_verdicts(pending), &empty(), now());
        assert_eq!(got.count, Some(2));
        assert_eq!(got.verdicts.map(|v| v.final_credit), Some(1));
    }

    /// A zero-credit final seen by the poll advances the marks but is not
    /// reported.
    #[test]
    fn a_zero_credit_final_alone_is_not_a_delta() {
        let marks = BTreeMap::new();
        let first = [rec(1, STATUS_ACCEPTED)];
        let (_, marks) = verdict_delta(&marks, &first, &no_never(), now());
        let later = [final_rec(1, STATUS_ACCEPTED, 0.0)];
        let (got, next) = verdict_delta(&marks, &later, &no_never(), now());
        assert_eq!(got, None);
        assert_ne!(next, marks, "the final is still marked as seen");
    }

    /// Precedence: U1 first. U2 leads behind a declined backlog, and then
    /// the backlog's cooldown is not reported, because something leads.
    #[test]
    fn the_backlog_leads_before_verdicts() {
        let inputs = LeadInputs {
            unpurposed_traces: Some(NUDGE_BACKLOG_THRESHOLD),
            ..with_verdicts(delta(1, 0, 0, now()))
        };
        let got = lead(&inputs, &empty(), now());
        assert_eq!(got.lead, Some(NudgeKind::ReviewBacklog));
        assert_eq!(got.verdicts, None);

        let behind = lead(&inputs, &declined(now() - Duration::days(1)), now());
        assert_eq!(behind.lead, Some(NudgeKind::VerdictsLanded));
        assert_eq!(behind.cooldown_until, None);
    }

    /// A stale history poll is `unknown` once U1 does not lead, with or
    /// without pending verdicts; U1 itself does not depend on history.
    #[test]
    fn a_stale_history_poll_is_unknown() {
        for pending in [None, Some(delta(1, 0, 0, now()))] {
            for last in [None, Some(now() - Duration::days(1))] {
                let inputs = LeadInputs {
                    unpurposed_traces: Some(0),
                    last_history_poll_at: last,
                    verdicts_pending: pending.clone(),
                    ..open()
                };
                assert_eq!(
                    lead(&inputs, &empty(), now()),
                    NudgeLead::quiet(NudgeState::Unknown),
                    "{pending:?} {last:?}"
                );
            }
        }
        let backlog = LeadInputs {
            last_history_poll_at: None,
            ..open()
        };
        assert_eq!(
            lead(&backlog, &empty(), now()).lead,
            Some(NudgeKind::ReviewBacklog)
        );
    }

    /// No pending verdicts, or a delta that counts nothing, leads nothing.
    #[test]
    fn no_verdicts_lead_nothing() {
        let quiet = LeadInputs {
            unpurposed_traces: Some(0),
            ..open()
        };
        assert_eq!(
            lead(&quiet, &empty(), now()),
            NudgeLead::quiet(NudgeState::None)
        );
        assert_eq!(
            lead(&with_verdicts(delta(0, 0, 0, now())), &empty(), now()),
            NudgeLead::quiet(NudgeState::None)
        );
    }

    /// Every gate closes U2 as it closes U1, and so does the arming offer.
    #[test]
    fn gates_and_the_arming_offer_close_verdicts_too() {
        let base = with_verdicts(delta(1, 0, 0, now()));
        for (name, inputs, state) in [
            (
                "paused",
                LeadInputs {
                    paused: true,
                    ..base.clone()
                },
                NudgeState::None,
            ),
            (
                "consent_hold",
                LeadInputs {
                    consent_hold: true,
                    ..base.clone()
                },
                NudgeState::None,
            ),
            (
                "unenrolled",
                LeadInputs {
                    enrolled: false,
                    ..base.clone()
                },
                NudgeState::None,
            ),
            (
                "suggestions off",
                LeadInputs {
                    suggestions_enabled: false,
                    ..base.clone()
                },
                NudgeState::None,
            ),
            (
                "arming offer",
                LeadInputs {
                    arming_offer_present: true,
                    ..base.clone()
                },
                NudgeState::None,
            ),
            (
                "unhealthy",
                LeadInputs {
                    healthy: false,
                    ..base.clone()
                },
                NudgeState::Unknown,
            ),
        ] {
            assert_eq!(
                lead(&inputs, &empty(), now()),
                NudgeLead::quiet(state),
                "{name}"
            );
        }
    }

    /// U2 has no "Not now"; U1 does. Its label is the notification kind's,
    /// so a ledger key and a notification key cannot drift apart.
    #[test]
    fn verdicts_landed_is_not_declinable_and_shares_the_attention_label() {
        assert!(NudgeKind::ReviewBacklog.declinable());
        assert!(!NudgeKind::VerdictsLanded.declinable());
        assert_eq!(
            NudgeKind::VerdictsLanded.label(),
            crate::daemon::attention::Kind::VerdictsLanded.label()
        );
    }

    /// The Never set read from policy: a folder set to Never, every folder
    /// under a Never override, and nothing else.
    #[test]
    fn never_projects_follow_the_policy() {
        let mut policy = ProjectPolicy::new();
        policy
            .set_mode("/tmp/never-a", ProjectMode::Ignore, now())
            .expect("set never");
        let keys = vec!["/tmp/never-a".to_string(), "/tmp/ask-b".to_string()];
        let never = NeverProjects::from_policy(&policy, &keys);
        assert!(!never.all);
        assert!(never.contains(&policy::project_id_for("/tmp/never-a")));
        assert!(!never.contains(&policy::project_id_for("/tmp/ask-b")));

        policy
            .set_contribution_override(ProjectMode::Ignore, now(), None)
            .expect("override");
        let all = NeverProjects::from_policy(&policy, &keys);
        assert!(all.all);
        assert!(all.contains(&policy::project_id_for("/tmp/ask-b")));
    }

    // ---- U4: idle sessions ----

    /// Over every TTL from 1 to 60 days the kind is either disabled or
    /// leaves room to announce a candidate before its entry expires:
    /// threshold + repeat interval + one day of slack < TTL.
    #[test]
    fn the_idle_window_always_fits_inside_the_queue_ttl() {
        for ttl in 1..=60_i64 {
            match idle_window(ttl) {
                None => assert!(ttl < IDLE_MIN_TTL_DAYS, "ttl {ttl} must not be disabled"),
                Some(w) => {
                    assert!(ttl >= IDLE_MIN_TTL_DAYS, "ttl {ttl} must be disabled");
                    assert!(w.idle_days >= 1 && w.repeat_days >= 1, "ttl {ttl}: {w:?}");
                    assert!(
                        w.idle_days + w.repeat_days + 1 < ttl,
                        "ttl {ttl}: {w:?} does not fit"
                    );
                    assert!(w.idle_days <= IDLE_DAYS, "ttl {ttl}: {w:?}");
                    assert!(w.repeat_days <= IDLE_REPEAT_DAYS, "ttl {ttl}: {w:?}");
                }
            }
        }
    }

    /// The defaults the spec states: 3 days idle and a 7-day repeat at the
    /// default 14-day TTL, and the kind off below a 4-day TTL.
    #[test]
    fn the_idle_window_at_the_default_ttl_is_three_and_seven() {
        assert_eq!(
            idle_window(14),
            Some(IdleWindow {
                idle_days: 3,
                repeat_days: 7
            })
        );
        assert_eq!(
            idle_window(4),
            Some(IdleWindow {
                idle_days: 1,
                repeat_days: 1
            })
        );
        assert_eq!(idle_window(3), None);
        assert_eq!(idle_window(0), None);
        assert_eq!(idle_window(-5), None);
        assert_eq!(
            idle_window(14).unwrap().repeat(),
            Duration::days(7),
            "the attention arbiter's N1 interval comes from here"
        );
    }

    fn with_idle(n: usize) -> LeadInputs {
        LeadInputs {
            idle_candidates: n,
            ..open()
        }
    }

    /// U4 leads ahead of U1, with the candidate count.
    #[test]
    fn idle_sessions_lead_ahead_of_the_backlog() {
        let got = lead(&with_idle(2), &empty(), now());
        assert_eq!(got.state, NudgeState::Armed);
        assert_eq!(got.lead, Some(NudgeKind::IdleSessions));
        assert_eq!(got.count, Some(2));
        assert_eq!(got.verdicts, None);
    }

    /// One candidate is enough, even with no backlog and verdict news
    /// waiting: U4 > U1 > U2.
    #[test]
    fn one_idle_session_leads_ahead_of_verdicts() {
        let inputs = LeadInputs {
            unpurposed_traces: Some(0),
            verdicts_pending: Some(delta(1, 0, 0, now())),
            idle_candidates: 1,
            ..open()
        };
        let got = lead(&inputs, &empty(), now());
        assert_eq!(got.lead, Some(NudgeKind::IdleSessions));
        assert_eq!(got.count, Some(1));
    }

    /// No candidates: U1 leads exactly as before.
    #[test]
    fn no_idle_sessions_leave_the_backlog_leading() {
        let got = lead(&with_idle(0), &empty(), now());
        assert_eq!(got.lead, Some(NudgeKind::ReviewBacklog));
    }

    /// The arming offer outranks U4, and every closed gate closes it.
    #[test]
    fn the_arming_offer_and_the_gates_close_idle_sessions() {
        let closed = [
            LeadInputs {
                arming_offer_present: true,
                ..with_idle(3)
            },
            LeadInputs {
                paused: true,
                ..with_idle(3)
            },
            LeadInputs {
                consent_hold: true,
                ..with_idle(3)
            },
            LeadInputs {
                enrolled: false,
                ..with_idle(3)
            },
            LeadInputs {
                suggestions_enabled: false,
                ..with_idle(3)
            },
        ];
        for inputs in closed {
            assert_eq!(
                lead(&inputs, &empty(), now()),
                NudgeLead::quiet(NudgeState::None),
                "{inputs:?}"
            );
        }
        let unhealthy = LeadInputs {
            healthy: false,
            ..with_idle(3)
        };
        assert_eq!(
            lead(&unhealthy, &empty(), now()),
            NudgeLead::quiet(NudgeState::Unknown)
        );
    }

    fn declined_kind(kind: NudgeKind, at: DateTime<Utc>) -> BTreeMap<String, NudgeLedger> {
        BTreeMap::from([(
            ledger_key(kind, None),
            NudgeLedger {
                declined_at: Some(at),
                ..NudgeLedger::default()
            },
        )])
    }

    /// U1 and U4 share one "Not now": both ask for decisions on the same
    /// waiting sessions, so declining either must not put the other up in
    /// its place. Verdict news, which is not an ask, still leads behind it.
    #[test]
    fn idle_and_backlog_share_one_not_now() {
        let at = now() - Duration::days(1);
        let until = at + decline_cooldown(14);
        for kind in [NudgeKind::IdleSessions, NudgeKind::ReviewBacklog] {
            let got = lead(&with_idle(4), &declined_kind(kind, at), now());
            assert_eq!(got.state, NudgeState::None, "{kind:?}");
            assert_eq!(got.lead, None, "{kind:?}");
            assert_eq!(got.cooldown_until, Some(until), "{kind:?}");

            let news = LeadInputs {
                verdicts_pending: Some(delta(1, 0, 0, now())),
                ..with_idle(4)
            };
            let got = lead(&news, &declined_kind(kind, at), now());
            assert_eq!(got.lead, Some(NudgeKind::VerdictsLanded), "{kind:?}");
        }
        // Lapsed: U4 leads again.
        let lapsed = now() - decline_cooldown(14) - Duration::minutes(1);
        let got = lead(
            &with_idle(4),
            &declined_kind(NudgeKind::IdleSessions, lapsed),
            now(),
        );
        assert_eq!(got.lead, Some(NudgeKind::IdleSessions));
    }

    /// The later of the two declines governs the shared cooldown.
    #[test]
    fn the_later_decline_governs_the_shared_cooldown() {
        let early = now() - Duration::days(6);
        let late = now() - Duration::days(1);
        let mut ledger = declined_kind(NudgeKind::ReviewBacklog, early);
        ledger.extend(declined_kind(NudgeKind::IdleSessions, late));
        let got = lead(&with_idle(1), &ledger, now());
        assert_eq!(got.cooldown_until, Some(late + decline_cooldown(14)));
    }

    /// U4 has a "Not now" and shares the notification kind's label.
    #[test]
    fn idle_sessions_is_declinable_and_shares_the_attention_label() {
        assert!(NudgeKind::IdleSessions.declinable());
        assert_eq!(
            NudgeKind::IdleSessions.label(),
            crate::daemon::attention::Kind::IdleSessions.label()
        );
        assert_eq!(
            NudgeKind::parse("idle_sessions"),
            Some(NudgeKind::IdleSessions)
        );
        assert_eq!(NudgeKind::ALL[0], NudgeKind::IdleSessions, "U4 ranks first");
    }

    /// The batching set is pruned to entries still pending.
    #[test]
    fn idle_announced_is_pruned_to_pending_entries() {
        let a = uuid::Uuid::from_u128(1);
        let b = uuid::Uuid::from_u128(2);
        let c = uuid::Uuid::from_u128(3);
        let mut announced = BTreeSet::from([a, b]);
        let pending = BTreeSet::from([b, c]);
        assert!(prune_idle_announced(&mut announced, &pending));
        assert_eq!(announced, BTreeSet::from([b]));
        assert!(!prune_idle_announced(&mut announced, &pending), "no change");
    }

    // ---- A3: the news mark and the halo ----

    /// Every gate open, nothing owed, the mark on, and fresh history with
    /// no news yet.
    fn mark_open(lead: LeadInputs) -> MarkInputs {
        MarkInputs {
            lead: LeadInputs {
                unpurposed_traces: Some(0),
                ..lead
            },
            decisions_owed: Some(0),
            menu_bar_mark_enabled: true,
            notify_idle_sessions: true,
        }
    }

    fn news_inputs(at: DateTime<Utc>) -> MarkInputs {
        mark_open(with_verdicts(delta(2, 1, 0, at)))
    }

    /// `idle` candidates, all of them owed.
    fn ready_inputs(idle: usize) -> MarkInputs {
        MarkInputs {
            decisions_owed: Some(idle.max(1)),
            ..mark_open(LeadInputs {
                idle_candidates: idle,
                ..open()
            })
        }
    }

    fn quiet_mark(state: MarkState) -> Mark {
        Mark {
            state,
            kinds: Vec::new(),
        }
    }

    #[test]
    fn verdicts_waiting_with_nothing_owed_light_the_news_mark() {
        let got = mark(&news_inputs(now() - Duration::hours(1)), &empty(), now());
        assert_eq!(
            got,
            Mark {
                state: MarkState::News,
                kinds: vec![super::super::attention::Kind::VerdictsLanded],
            }
        );
        assert_eq!(got.state.label(), "news");
    }

    #[test]
    fn nothing_waiting_is_none() {
        let got = mark(&mark_open(open()), &empty(), now());
        assert_eq!(got, quiet_mark(MarkState::None));
        assert_eq!(MarkState::None.label(), "none");
        assert_eq!(MarkState::Unknown.label(), "unknown");
        assert_eq!(MarkState::Ready.label(), "ready");
    }

    /// News is impossible while any decision is owed: the badge takes the
    /// slot. When the badge clears, unacknowledged news returns.
    #[test]
    fn news_is_impossible_while_decisions_are_owed() {
        let at = now() - Duration::hours(1);
        for owed in [1, 2, 99, 10_000] {
            let inputs = MarkInputs {
                decisions_owed: Some(owed),
                ..news_inputs(at)
            };
            let got = mark(&inputs, &empty(), now());
            assert_ne!(got.state, MarkState::News, "owed {owed}");
        }
        assert_eq!(
            mark(&news_inputs(at), &empty(), now()).state,
            MarkState::News,
            "returns once the badge clears"
        );
    }

    /// Paused, consent-held, signed out or the mark switched off: `none`.
    /// Unhealthy, or an owed count that could not be computed: `unknown`.
    /// Never `news`, never `ready`.
    #[test]
    fn a_closed_gate_lights_nothing() {
        let at = now() - Duration::hours(1);
        let closed: [(&str, fn(&mut MarkInputs), MarkState); 7] = [
            ("paused", |m| m.lead.paused = true, MarkState::None),
            (
                "consent hold",
                |m| m.lead.consent_hold = true,
                MarkState::None,
            ),
            ("unenrolled", |m| m.lead.enrolled = false, MarkState::None),
            (
                "mark off",
                |m| m.menu_bar_mark_enabled = false,
                MarkState::None,
            ),
            (
                "suggestions off",
                |m| m.lead.suggestions_enabled = false,
                MarkState::None,
            ),
            ("unhealthy", |m| m.lead.healthy = false, MarkState::Unknown),
            (
                "owed unknown",
                |m| m.decisions_owed = None,
                MarkState::Unknown,
            ),
        ];
        for (name, close, want) in closed {
            for base in [news_inputs(at), ready_inputs(3)] {
                let mut inputs = base;
                close(&mut inputs);
                assert_eq!(mark(&inputs, &empty(), now()), quiet_mark(want), "{name}");
            }
        }
    }

    /// Every combination of gates, owed counts, news, candidates, switches
    /// and cooldowns: `news` only with nothing owed and every gate open,
    /// `ready` only with something owed and every gate open, and so never
    /// both. Each lit state names exactly its one kind.
    #[test]
    fn the_truth_table_keeps_news_and_ready_apart() {
        use super::super::attention::Kind;
        let at = now() - Duration::hours(1);
        let bools = [false, true];
        for paused in bools {
            for consent_hold in bools {
                for enrolled in bools {
                    for healthy in bools {
                        for mark_on in bools {
                            for idle_on in bools {
                                for owed in [None, Some(0), Some(1), Some(7)] {
                                    for idle in [0, 1, 4] {
                                        for news in [None, Some(delta(1, 0, 0, at))] {
                                            for ledger in
                                                [empty(), declined(now() - Duration::days(1))]
                                            {
                                                let inputs = MarkInputs {
                                                    lead: LeadInputs {
                                                        paused,
                                                        consent_hold,
                                                        enrolled,
                                                        healthy,
                                                        idle_candidates: idle,
                                                        verdicts_pending: news.clone(),
                                                        ..open()
                                                    },
                                                    decisions_owed: owed,
                                                    menu_bar_mark_enabled: mark_on,
                                                    notify_idle_sessions: idle_on,
                                                };
                                                let got = mark(&inputs, &ledger, now());
                                                let gates = !paused
                                                    && !consent_hold
                                                    && enrolled
                                                    && healthy
                                                    && mark_on;
                                                let ctx = format!("{inputs:?} {ledger:?}");
                                                match got.state {
                                                    MarkState::News => {
                                                        assert!(gates, "{ctx}");
                                                        assert_eq!(owed, Some(0), "{ctx}");
                                                        assert!(news.is_some(), "{ctx}");
                                                        assert_eq!(
                                                            got.kinds,
                                                            [Kind::VerdictsLanded],
                                                            "{ctx}"
                                                        );
                                                    }
                                                    MarkState::Ready => {
                                                        assert!(gates && idle_on, "{ctx}");
                                                        assert!(owed > Some(0), "{ctx}");
                                                        assert!(idle > 0, "{ctx}");
                                                        assert!(ledger.is_empty(), "{ctx}");
                                                        assert_eq!(
                                                            got.kinds,
                                                            [Kind::IdleSessions],
                                                            "{ctx}"
                                                        );
                                                    }
                                                    MarkState::None | MarkState::Unknown => {
                                                        assert!(got.kinds.is_empty(), "{ctx}");
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// The news ages out [`NEWS_MARK_TTL`] after its newest verdict, and a
    /// later verdict re-arms it.
    #[test]
    fn the_news_ages_out_and_a_later_verdict_rearms_it() {
        let at = now() - NEWS_MARK_TTL;
        assert_eq!(
            mark(&news_inputs(at), &empty(), now()),
            quiet_mark(MarkState::None),
            "aged out at exactly the TTL"
        );
        let just_inside = now() - NEWS_MARK_TTL + Duration::seconds(1);
        assert_eq!(
            mark(&news_inputs(just_inside), &empty(), now()).state,
            MarkState::News
        );

        let first = after_history_poll(
            true,
            &BTreeMap::new(),
            None,
            &[rec(1, STATUS_ACCEPTED)],
            &no_never(),
            at,
        );
        let pending = after_history_poll(
            true,
            &first.marks,
            first.pending.as_ref(),
            &[rec(1, STATUS_ACCEPTED), rec(2, STATUS_QUARANTINED)],
            &no_never(),
            now() - Duration::minutes(5),
        )
        .pending
        .expect("both pending");
        let rearmed = mark_open(with_verdicts(pending));
        assert_eq!(mark(&rearmed, &empty(), now()).state, MarkState::News);
    }

    /// The TTL is the owner's 72 hours (decision 19).
    #[test]
    fn the_news_mark_ttl_is_72_hours() {
        assert_eq!(NEWS_MARK_TTL, Duration::hours(72));
    }

    /// Ageing out clears the mark, not the card: the news stays on the panel
    /// row and in the ledger until its own action acknowledges it.
    #[test]
    fn ageing_out_leaves_the_lead_in_place() {
        let inputs = news_inputs(now() - NEWS_MARK_TTL - Duration::hours(1));
        assert_eq!(mark(&inputs, &empty(), now()).state, MarkState::None);
        assert_eq!(
            lead(&inputs.lead, &empty(), now()).lead,
            Some(NudgeKind::VerdictsLanded)
        );
    }

    /// Acknowledged news (`nudge_opened` takes `verdicts_pending`) lights
    /// nothing; so does news stamped after now: the clock went backwards,
    /// which can only suppress.
    #[test]
    fn acknowledged_or_future_news_lights_nothing() {
        let acked = mark_open(open());
        assert_eq!(acked.lead.verdicts_pending, None);
        assert_eq!(mark(&acked, &empty(), now()), quiet_mark(MarkState::None));
        let future = news_inputs(now() + Duration::minutes(1));
        assert_eq!(mark(&future, &empty(), now()), quiet_mark(MarkState::None));
    }

    /// With nothing owed, a stale history poll is `unknown`: a stale cache
    /// is never news, and "no news" from it is not known either. The halo
    /// does not read history.
    #[test]
    fn stale_history_is_unknown_for_news_but_not_for_the_halo() {
        for last in [None, Some(now() - Duration::days(1))] {
            let mut news = news_inputs(now() - Duration::hours(1));
            news.lead.last_history_poll_at = last;
            assert_eq!(mark(&news, &empty(), now()), quiet_mark(MarkState::Unknown));
            let mut ready = ready_inputs(2);
            ready.lead.last_history_poll_at = last;
            assert_eq!(mark(&ready, &empty(), now()).state, MarkState::Ready);
        }
    }

    /// The suggestions switch governs the mark (OWNER DECISION 2026-10-08:
    /// its copy says it covers the menu bar), and the mark's own switch is a
    /// finer control under it. The arming offer still does not touch it.
    #[test]
    fn suggestions_govern_the_mark_and_the_arming_offer_does_not() {
        let mut news = news_inputs(now() - Duration::hours(1));
        news.lead.arming_offer_present = true;
        assert_eq!(mark(&news, &empty(), now()).state, MarkState::News);
        news.lead.suggestions_enabled = false;
        assert_eq!(mark(&news, &empty(), now()).state, MarkState::None);
        let mut ready = ready_inputs(3);
        ready.lead.suggestions_enabled = false;
        assert_eq!(mark(&ready, &empty(), now()).state, MarkState::None);
    }

    #[test]
    fn idle_candidates_under_a_lit_badge_draw_the_halo() {
        assert_eq!(
            mark(&ready_inputs(2), &empty(), now()),
            Mark {
                state: MarkState::Ready,
                kinds: vec![super::super::attention::Kind::IdleSessions],
            }
        );
    }

    /// The halo clears when the candidate set empties, whatever else is
    /// still owed.
    #[test]
    fn the_halo_clears_when_the_candidates_empty() {
        let mut inputs = ready_inputs(3);
        inputs.lead.idle_candidates = 0;
        inputs.decisions_owed = Some(5);
        assert_eq!(mark(&inputs, &empty(), now()), quiet_mark(MarkState::None));
    }

    /// Muting `notify.idle_sessions` clears the halo; the in-app "Not now"
    /// does too, until it lapses.
    #[test]
    fn the_halo_obeys_its_mute_and_the_not_now() {
        let mut muted = ready_inputs(2);
        muted.notify_idle_sessions = false;
        assert_eq!(mark(&muted, &empty(), now()), quiet_mark(MarkState::None));

        let declined_at = now() - Duration::days(1);
        for kind in [NudgeKind::IdleSessions, NudgeKind::ReviewBacklog] {
            let ledger = BTreeMap::from([(
                ledger_key(kind, None),
                NudgeLedger {
                    declined_at: Some(declined_at),
                    ..NudgeLedger::default()
                },
            )]);
            assert_eq!(
                mark(&ready_inputs(2), &ledger, now()),
                quiet_mark(MarkState::None),
                "{kind:?}"
            );
            let lapsed = declined_at + decline_cooldown(14);
            assert_eq!(
                mark(&ready_inputs(2), &ledger, lapsed).state,
                MarkState::Ready,
                "{kind:?}"
            );
        }
    }

    /// The module is pure: no lock is ever taken in it, so the status
    /// builder can call it anywhere without adding a lock ordering.
    #[test]
    fn this_module_takes_no_locks() {
        let source = include_str!("nudge.rs");
        let body = source.split("#[cfg(test)]").next().unwrap();
        assert!(!body.contains(".lock("), "nudge.rs must take no locks");
        assert!(!body.contains("Mutex"), "nudge.rs must hold no locks");
    }
}
