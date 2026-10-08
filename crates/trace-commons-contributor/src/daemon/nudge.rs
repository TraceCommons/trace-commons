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
//! Phase 1 starts with one kind, U1 `review_backlog` (previewed, undecided
//! sessions in folders set to Ask me). Later slices add idle sessions (U4)
//! and verdict news (U2) ahead of and behind it.
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

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// U1 leads once at least this many previewed, undecided Ask-me sessions are
/// waiting. Mirrors `policy::ARMING_SUGGESTION_THRESHOLD`. DRAFT, owner
/// decision 10 in the upsell decisions list.
pub const NUDGE_BACKLOG_THRESHOLD: usize = 5;

/// How long an in-app "Not now" silences a suggestion, before the queue TTL
/// caps it (see [`decline_cooldown`]). DRAFT, owner decision 10.
pub const NUDGE_DECLINE_COOLDOWN_DAYS: i64 = 7;

/// A suggestion kind. The wire, ledger keys and logs use [`NudgeKind::label`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NudgeKind {
    /// U1: previewed, undecided sessions in folders set to Ask me
    /// (`queue::unpurposed_traces`). It has no notification kind of its own:
    /// the digest already covers that moment.
    ReviewBacklog,
}

impl NudgeKind {
    /// Every kind, highest precedence first.
    pub const ALL: [NudgeKind; 1] = [NudgeKind::ReviewBacklog];

    /// The stable label used on the wire, as a ledger key and in logs. A
    /// kind that is also a notification kind takes `attention::Kind::label`,
    /// so a ledger key and a notification key can never drift apart.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            NudgeKind::ReviewBacklog => "review_backlog",
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

/// Everything `lead` reads besides the ledger and the clock, as snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// kind that would otherwise lead.
    pub cooldown_until: Option<DateTime<Utc>>,
}

impl NudgeLead {
    fn quiet(state: NudgeState) -> Self {
        Self {
            state,
            lead: None,
            count: None,
            cooldown_until: None,
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
    // Decisions owed first: the arming offer outranks the backlog.
    if inputs.arming_offer_present || unpurposed < NUDGE_BACKLOG_THRESHOLD {
        return NudgeLead::quiet(NudgeState::None);
    }
    let kind = NudgeKind::ReviewBacklog;
    if let Some(until) = cooldown_until(kind, inputs.queue_ttl_days, ledger, now) {
        return NudgeLead {
            cooldown_until: Some(until),
            ..NudgeLead::quiet(NudgeState::None)
        };
    }
    NudgeLead {
        state: NudgeState::Armed,
        lead: Some(kind),
        count: Some(unpurposed),
        cooldown_until: None,
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
