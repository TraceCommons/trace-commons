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
//! Phase 1 has two kinds so far: U1 `review_backlog` (previewed, undecided
//! sessions in folders set to Ask me) and, behind it, U2 `verdicts_landed`
//! (submissions that newly reached a verdict since the daemon last looked).
//! A later slice adds idle sessions (U4) ahead of both.
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
/// decision 10 in the upsell decisions list.
pub const NUDGE_BACKLOG_THRESHOLD: usize = 5;

/// How long an in-app "Not now" silences a suggestion, before the queue TTL
/// caps it (see [`decline_cooldown`]). DRAFT, owner decision 10.
pub const NUDGE_DECLINE_COOLDOWN_DAYS: i64 = 7;

/// U2 reads `unknown` once the last successful history poll is older than
/// this many history poll intervals (`history_poll_secs`): a failed poll
/// serves the cache as it was, so a stale cache is never news. DRAFT, spec
/// section 3 ("Shared gates", history stale); not a numbered owner decision.
pub const HISTORY_STALE_POLL_MULTIPLE: i64 = 2;

/// A suggestion kind. The wire, ledger keys and logs use [`NudgeKind::label`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NudgeKind {
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
    pub const ALL: [NudgeKind; 2] = [NudgeKind::ReviewBacklog, NudgeKind::VerdictsLanded];

    /// The stable label used on the wire, as a ledger key and in logs. A
    /// kind that is also a notification kind takes `attention::Kind::label`,
    /// so a ledger key and a notification key can never drift apart.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            NudgeKind::ReviewBacklog => "review_backlog",
            NudgeKind::VerdictsLanded => super::attention::Kind::VerdictsLanded.label(),
        }
    }

    /// Whether an in-app "Not now" (`nudge_decline`) applies to this kind.
    /// U2 is news, not an ask, so it has none (spec section 3).
    #[must_use]
    pub fn declinable(self) -> bool {
        match self {
            NudgeKind::ReviewBacklog => true,
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

/// Verdicts that landed and have not been acknowledged: counts and times
/// only. What `status.nudge` reports for U2 and what a later slice folds into
/// a digest or a notification.
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
    /// The final credit those submissions carry. Kept in the state file for
    /// the later slice that may word it (owner decision 12); never on the
    /// wire from this slice.
    #[serde(default)]
    pub credit_final_delta: f32,
    /// The poll that first saw a verdict in this delta: `status.nudge.since`.
    pub since: DateTime<Utc>,
    /// The poll that last added to this delta. `nudge_opened` acknowledges
    /// through it, and the news mark ages out from it.
    pub newest_at: DateTime<Utc>,
}

impl VerdictDelta {
    /// Every verdict in the delta, of whatever kind.
    #[must_use]
    pub fn total(&self) -> u32 {
        self.newly_accepted
            .saturating_add(self.newly_held)
            .saturating_add(self.newly_final)
    }

    /// Add a later delta to this one: counts add, `since` keeps the
    /// earlier, `newest_at` takes the later.
    pub fn absorb(&mut self, later: &VerdictDelta) {
        self.newly_accepted = self.newly_accepted.saturating_add(later.newly_accepted);
        self.newly_held = self.newly_held.saturating_add(later.newly_held);
        self.newly_final = self.newly_final.saturating_add(later.newly_final);
        self.credit_final_delta += later.credit_final_delta;
        self.since = self.since.min(later.since);
        self.newest_at = self.newest_at.max(later.newest_at);
    }
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
    let mut found = VerdictDelta {
        newly_accepted: 0,
        newly_held: 0,
        newly_final: 0,
        credit_final_delta: 0.0,
        since: now,
        newest_at: now,
    };
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
        if !is_taken_back(record) && !never.contains(&record.project_id) {
            if seen.accepted && !before.accepted {
                found.newly_accepted += 1;
            }
            if seen.held && !before.held {
                found.newly_held += 1;
            }
            if seen.final_credit && !before.final_credit {
                found.newly_final += 1;
                found.credit_final_delta += record.credit_points_final.unwrap_or(0.0);
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
    ((found.total() > 0).then_some(found), next_marks)
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
/// poll after an upgrade or after `unenroll`) the marks are seeded silently:
/// nothing lands and `pending` is left as it is. Afterwards what lands is
/// added to `pending`.
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
    let pending = match (pending, &landed) {
        (Some(before), Some(new)) => {
            let mut sum = before.clone();
            sum.absorb(new);
            Some(sum)
        }
        (before, new) => before.cloned().or_else(|| new.clone()),
    };
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

/// U2's counts for the card and the panel row: no credit figure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerdictSummary {
    pub accepted: u32,
    pub held: u32,
    pub final_credit: u32,
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
    // U1 first. A "Not now" in force lets U2 lead behind it.
    let mut silenced_until = None;
    if unpurposed >= NUDGE_BACKLOG_THRESHOLD {
        let kind = NudgeKind::ReviewBacklog;
        match cooldown_until(kind, inputs.queue_ttl_days, ledger, now) {
            Some(until) => silenced_until = Some(until),
            None => {
                return NudgeLead {
                    state: NudgeState::Armed,
                    lead: Some(kind),
                    count: Some(unpurposed),
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
                final_credit: news.newly_final,
                since: news.since,
            }),
        },
        None => NudgeLead {
            cooldown_until: silenced_until,
            ..NudgeLead::quiet(NudgeState::None)
        },
    }
}

/// When the "Not now" in force against `kind` lapses, or `None` when none
/// is in force. A decline stamped after `now` is in force: a clock that went
/// backwards can only suppress.
fn cooldown_until(
    kind: NudgeKind,
    queue_ttl_days: i64,
    ledger: &BTreeMap<String, NudgeLedger>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let declined = ledger.get(&ledger_key(kind, None))?.declined_at?;
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

    fn delta(a: u32, h: u32, f: u32, at: DateTime<Utc>) -> VerdictDelta {
        VerdictDelta {
            newly_accepted: a,
            newly_held: h,
            newly_final: f,
            credit_final_delta: 0.0,
            since: at,
            newest_at: at,
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
        assert!((got.credit_final_delta - 2.5).abs() < f32::EPSILON);
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
        let pending = delta(1, 0, 0, earlier);
        let poll = after_history_poll(
            true,
            &BTreeMap::new(),
            Some(&pending),
            &[rec(1, STATUS_QUARANTINED)],
            &no_never(),
            now(),
        );
        let got = poll.pending.expect("pending");
        assert_eq!((got.newly_accepted, got.newly_held), (1, 1));
        assert_eq!(got.since, earlier);
        assert_eq!(got.newest_at, now());
        assert_eq!(poll.landed.map(|d| d.newly_held), Some(1));
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
        let got = lead(&with_verdicts(delta(2, 1, 1, at)), &empty(), now());
        assert_eq!(got.state, NudgeState::Armed);
        assert_eq!(got.lead, Some(NudgeKind::VerdictsLanded));
        assert_eq!(got.count, Some(4));
        assert_eq!(
            got.verdicts,
            Some(VerdictSummary {
                accepted: 2,
                held: 1,
                final_credit: 1,
                since: at,
            })
        );
        assert_eq!(got.cooldown_until, None);
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
