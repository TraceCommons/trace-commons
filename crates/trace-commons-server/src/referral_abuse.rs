// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Detecting abusive referral codes
//! (docs/superpowers/specs/2026-10-29-referral-rewards-design.md, section 7,
//! and section 4 "Holds on the referral stream").
//!
//! Pure functions only: no clock, no store. The bonus pass
//! ([`crate::referral_bonus`]) computes each referee's signal labels from
//! facts it reads in the referee's tenant ([`referee_signal_labels`]),
//! writes them onto the referee's mirror row in the referrer's tenant, and
//! then scores the referrer's code over every mirror row of that code
//! ([`score_code`]) and the referrer over every mirror row of all their
//! codes ([`score_referrer`]). More than [`ReferralAbuseSettings::trip_codes`]
//! codes suspended in 24 hours switches the referral stream to `dry_run`
//! ([`should_trip_dry_run`]).
//!
//! Every threshold is a setting with the spec's default, so the operator can
//! tune it from the dry-run report before the stream goes `live`.
//!
//! Labels are safe identifiers, never a tenant, account or index.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, Duration, Utc};

/// Most of the referee's submissions were refused by scoring or the gates.
pub const REFERRAL_REFUSED_SHARE: &str = "referral_refused_share";
/// The referee has an open credit hold or an operator reversal.
pub const REFERRAL_CREDIT_HOLD_SHARE: &str = "referral_credit_hold_share";
/// The referee's starter advance was forfeited.
pub const REFERRAL_ADVANCE_FORFEIT_SHARE: &str = "referral_advance_forfeit_share";
/// The referee shares a payout recipient or subject with another referee of
/// the same code. Nothing the client reports (a device key, an install id)
/// counts: the client is open source.
pub const REFERRAL_SHARED_IDENTITY: &str = "referral_shared_identity";
/// The referee's traces fall in the same near-duplicate cluster as the
/// referrer's (`trace_gate_decisions.dedup_cluster_id`, assigned from the
/// cross-tenant simhash scan).
pub const REFERRAL_DUPLICATE_TRACES: &str = "referral_duplicate_traces";
/// Many referees applied the code within 24 hours.
pub const REFERRAL_BURST: &str = "referral_burst";
/// The referee uploads but has no settled credit 30 days after joining.
pub const REFERRAL_NEVER_SETTLES: &str = "referral_never_settles";

/// Every signal label, in the spec's order.
pub const REFERRAL_SIGNAL_LABELS: [&str; 7] = [
    REFERRAL_REFUSED_SHARE,
    REFERRAL_CREDIT_HOLD_SHARE,
    REFERRAL_ADVANCE_FORFEIT_SHARE,
    REFERRAL_SHARED_IDENTITY,
    REFERRAL_DUPLICATE_TRACES,
    REFERRAL_BURST,
    REFERRAL_NEVER_SETTLES,
];

/// Referral-stream holds on the referrer's account (spec section 4).
/// The referee's live payout link has the referrer's `recipient_index`.
/// Also written on the referee's mirror row as operator evidence, never
/// scored, so the hold opens once per referee.
pub const REFERRAL_SAME_PAYOUT_ORG: &str = "referral_same_payout_org";
/// The same for `subject_index`.
pub const REFERRAL_SAME_PAYOUT_SUBJECT: &str = "referral_same_payout_subject";
/// The referee's and the referrer's traces share a near-duplicate cluster.
pub const REFERRAL_SAME_TRACES: &str = "referral_same_traces";
/// The referee and the referrer were seen on one network prefix (a keyed
/// mark of the /24 or /48 the trusted edge observed). Also written on the
/// referee's mirror row as operator evidence, where it never counts toward
/// a code's suspension: colleagues on one office network refer each other.
pub const REFERRAL_SAME_NETWORK: &str = "referral_same_network";
pub const REFERRAL_CODE_SUSPENDED: &str = "referral_code_suspended";
pub const REFERRAL_OVER_CEILING: &str = "referral_over_ceiling";
pub const REFERRAL_REFERRER_SUSPENDED: &str = "referral_referrer_suspended";

/// Every hold label the referral stream opens.
pub const REFERRAL_HOLD_LABELS: [&str; 7] = [
    REFERRAL_SAME_PAYOUT_ORG,
    REFERRAL_SAME_PAYOUT_SUBJECT,
    REFERRAL_SAME_TRACES,
    REFERRAL_SAME_NETWORK,
    REFERRAL_CODE_SUSPENDED,
    REFERRAL_OVER_CEILING,
    REFERRAL_REFERRER_SUSPENDED,
];

/// Mirror-row labels that are evidence for an operator only: written with
/// the signal labels, shown in the dry-run report, and never counted by
/// [`score_rows`], so they can never suspend a code or a referrer.
pub const REFERRAL_EVIDENCE_LABELS: [&str; 3] = [
    REFERRAL_SAME_PAYOUT_ORG,
    REFERRAL_SAME_PAYOUT_SUBJECT,
    REFERRAL_SAME_NETWORK,
];

/// The label recorded when too many codes were suspended at once and the
/// referral stream switched itself to `dry_run`; readiness reports it.
pub const REFERRAL_TRIPPED: &str = "referral_tripped";

/// Whether a `trace_payout_holds` label holds the referral stream only.
/// Every referral-stream hold is `referral_*`, and no earned-stream hold is,
/// so the earned stream's pass passes over these labels and the referral
/// stream reads them. The same test as
/// [`crate::payout_pass::is_referral_stream_hold`].
pub fn hold_holds_referral_stream_only(label: &str) -> bool {
    crate::payout_pass::is_referral_stream_hold(label)
}

/// Why the abuse settings were refused. Names the setting, never its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("referral_abuse_setting_invalid")]
pub struct ReferralAbuseSettingError {
    pub setting: &'static str,
}

impl ReferralAbuseSettingError {
    pub fn label(self) -> &'static str {
        "referral_abuse_setting_invalid"
    }
}

pub const REFERRAL_ABUSE_MIN_REFEREES_ENV: &str = "TRACE_COMMONS_REFERRAL_ABUSE_MIN_REFEREES";
pub const REFERRAL_ABUSE_REFUSED_SHARE_BPS_ENV: &str =
    "TRACE_COMMONS_REFERRAL_ABUSE_REFUSED_SHARE_BPS";
pub const REFERRAL_ABUSE_CREDIT_HOLD_SHARE_BPS_ENV: &str =
    "TRACE_COMMONS_REFERRAL_ABUSE_CREDIT_HOLD_SHARE_BPS";
pub const REFERRAL_ABUSE_ADVANCE_FORFEIT_SHARE_BPS_ENV: &str =
    "TRACE_COMMONS_REFERRAL_ABUSE_ADVANCE_FORFEIT_SHARE_BPS";
pub const REFERRAL_ABUSE_SHARED_IDENTITY_CLUSTER_ENV: &str =
    "TRACE_COMMONS_REFERRAL_ABUSE_SHARED_IDENTITY_CLUSTER";
pub const REFERRAL_ABUSE_DUPLICATE_SHARE_BPS_ENV: &str =
    "TRACE_COMMONS_REFERRAL_ABUSE_DUPLICATE_SHARE_BPS";
pub const REFERRAL_ABUSE_BURST_COUNT_ENV: &str = "TRACE_COMMONS_REFERRAL_ABUSE_BURST_COUNT";
pub const REFERRAL_ABUSE_NEVER_SETTLES_SHARE_BPS_ENV: &str =
    "TRACE_COMMONS_REFERRAL_ABUSE_NEVER_SETTLES_SHARE_BPS";
pub const REFERRAL_ABUSE_NEVER_SETTLES_DAYS_ENV: &str =
    "TRACE_COMMONS_REFERRAL_ABUSE_NEVER_SETTLES_DAYS";
pub const REFERRAL_ABUSE_TRIP_CODES_ENV: &str = "TRACE_COMMONS_REFERRAL_ABUSE_TRIP_CODES";

/// The thresholds of section 7. Shares are in basis points of a code's
/// referees, and a share trips when it is strictly over its threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferralAbuseSettings {
    /// A code is scored once it has at least this many referees (5).
    pub min_referees: u32,
    /// Over 40%.
    pub refused_share_bps: u32,
    /// Over 20%.
    pub credit_hold_share_bps: u32,
    /// Over 20%.
    pub advance_forfeit_share_bps: u32,
    /// 3 or more referees in one cluster.
    pub shared_identity_cluster: u32,
    /// Over 20%.
    pub duplicate_share_bps: u32,
    /// More than 20 referees in 24 hours.
    pub burst_count: u32,
    /// Over 60%.
    pub never_settles_share_bps: u32,
    /// "30 days after joining".
    pub never_settles_days: u32,
    /// More than 2 codes suspended in 24 hours trips the stream to `dry_run`.
    pub trip_codes: u32,
}

impl Default for ReferralAbuseSettings {
    fn default() -> Self {
        Self {
            min_referees: 5,
            refused_share_bps: 4_000,
            credit_hold_share_bps: 2_000,
            advance_forfeit_share_bps: 2_000,
            shared_identity_cluster: 3,
            duplicate_share_bps: 2_000,
            burst_count: 20,
            never_settles_share_bps: 6_000,
            never_settles_days: 30,
            trip_codes: 2,
        }
    }
}

impl ReferralAbuseSettings {
    /// Reads every threshold through `get` (`None` for unset), each falling
    /// back to its default when unset. A set value that is not a plain
    /// decimal in range refuses the start: a typo must not silently loosen
    /// a threshold.
    pub fn from_vars(
        get: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ReferralAbuseSettingError> {
        let defaults = Self::default();
        let read = |setting: &'static str, default: u32, min: u32, max: u32| match get(setting) {
            None => Ok(default),
            Some(value) => value
                .bytes()
                .all(|byte| byte.is_ascii_digit())
                .then(|| value.parse::<u32>().ok())
                .flatten()
                .filter(|parsed| (min..=max).contains(parsed))
                .ok_or(ReferralAbuseSettingError { setting }),
        };
        Ok(Self {
            min_referees: read(
                REFERRAL_ABUSE_MIN_REFEREES_ENV,
                defaults.min_referees,
                1,
                100_000,
            )?,
            refused_share_bps: read(
                REFERRAL_ABUSE_REFUSED_SHARE_BPS_ENV,
                defaults.refused_share_bps,
                0,
                10_000,
            )?,
            credit_hold_share_bps: read(
                REFERRAL_ABUSE_CREDIT_HOLD_SHARE_BPS_ENV,
                defaults.credit_hold_share_bps,
                0,
                10_000,
            )?,
            advance_forfeit_share_bps: read(
                REFERRAL_ABUSE_ADVANCE_FORFEIT_SHARE_BPS_ENV,
                defaults.advance_forfeit_share_bps,
                0,
                10_000,
            )?,
            shared_identity_cluster: read(
                REFERRAL_ABUSE_SHARED_IDENTITY_CLUSTER_ENV,
                defaults.shared_identity_cluster,
                2,
                100_000,
            )?,
            duplicate_share_bps: read(
                REFERRAL_ABUSE_DUPLICATE_SHARE_BPS_ENV,
                defaults.duplicate_share_bps,
                0,
                10_000,
            )?,
            burst_count: read(
                REFERRAL_ABUSE_BURST_COUNT_ENV,
                defaults.burst_count,
                1,
                100_000,
            )?,
            never_settles_share_bps: read(
                REFERRAL_ABUSE_NEVER_SETTLES_SHARE_BPS_ENV,
                defaults.never_settles_share_bps,
                0,
                10_000,
            )?,
            never_settles_days: read(
                REFERRAL_ABUSE_NEVER_SETTLES_DAYS_ENV,
                defaults.never_settles_days,
                1,
                3_650,
            )?,
            trip_codes: read(
                REFERRAL_ABUSE_TRIP_CODES_ENV,
                defaults.trip_codes,
                1,
                100_000,
            )?,
        })
    }
}

/// What the pass reads about one referee in the referee's tenant. Counts and
/// flags only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefereeFacts {
    /// Submissions by the referee's principals.
    pub submissions: u64,
    /// Of those, refused (`rejected`) by scoring or the gates.
    pub refused: u64,
    /// An open credit hold on the account, or an operator reversal accrued
    /// against it.
    pub credit_hold_or_reversal: bool,
    /// The referee's starter advance was forfeited. Always false until the
    /// advance exists.
    pub advance_forfeited: bool,
    /// The referee's traces share a near-duplicate cluster with the
    /// referrer's.
    pub duplicate_traces: bool,
    /// Any credit accrual on the rail.
    pub settled_credit: bool,
    pub account_created_at: DateTime<Utc>,
}

/// The referee-level labels, from the referee's own facts. The code-level
/// labels (`referral_shared_identity`, `referral_burst`) are not referee
/// facts and are found by [`score_code`].
pub fn referee_signal_labels(
    facts: &RefereeFacts,
    now: DateTime<Utc>,
    settings: &ReferralAbuseSettings,
) -> Vec<&'static str> {
    let mut labels = Vec::new();
    // "Most": strictly more than half.
    if facts.submissions > 0 && facts.refused.saturating_mul(2) > facts.submissions {
        labels.push(REFERRAL_REFUSED_SHARE);
    }
    if facts.credit_hold_or_reversal {
        labels.push(REFERRAL_CREDIT_HOLD_SHARE);
    }
    if facts.advance_forfeited {
        labels.push(REFERRAL_ADVANCE_FORFEIT_SHARE);
    }
    if facts.duplicate_traces {
        labels.push(REFERRAL_DUPLICATE_TRACES);
    }
    let settle_by =
        facts.account_created_at + Duration::days(i64::from(settings.never_settles_days));
    if facts.submissions > 0 && !facts.settled_credit && now >= settle_by {
        labels.push(REFERRAL_NEVER_SETTLES);
    }
    labels
}

/// One mirror row, as the code check reads it. `Debug` prints counts and
/// labels only.
#[derive(Clone, PartialEq, Eq)]
pub struct RefereeMirror {
    pub referee_ref_hash: String,
    pub code_index: String,
    pub applied_at: DateTime<Utc>,
    pub signal_labels: Vec<String>,
    pub recipient_index: Option<String>,
    pub subject_index: Option<String>,
    pub voided: bool,
}

impl std::fmt::Debug for RefereeMirror {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RefereeMirror")
            .field("signal_labels", &self.signal_labels)
            .field("voided", &self.voided)
            .finish_non_exhaustive()
    }
}

/// How a code (or a referrer) scored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeScore {
    /// Referees not voided.
    pub referees: u64,
    /// Referees per signal label, the code-level labels included.
    pub counts: BTreeMap<&'static str, u64>,
    /// The labels that tripped, in [`REFERRAL_SIGNAL_LABELS`] order. Empty
    /// when the code is under the minimum or nothing tripped.
    pub tripped: Vec<&'static str>,
}

impl CodeScore {
    pub fn is_tripped(&self) -> bool {
        !self.tripped.is_empty()
    }
}

fn find(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

/// The size of each referee's identity cluster: referees joined by a shared
/// `recipient_index` or `subject_index`, transitively.
fn cluster_sizes(rows: &[&RefereeMirror]) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..rows.len()).collect();
    let mut first_seen: HashMap<(u8, &str), usize> = HashMap::new();
    for (position, row) in rows.iter().enumerate() {
        for (kind, index) in [
            (0_u8, row.recipient_index.as_deref()),
            (1_u8, row.subject_index.as_deref()),
        ] {
            let Some(index) = index else { continue };
            match first_seen.get(&(kind, index)) {
                Some(&other) => {
                    let (a, b) = (find(&mut parent, position), find(&mut parent, other));
                    if a != b {
                        parent[a] = b;
                    }
                }
                None => {
                    first_seen.insert((kind, index), position);
                }
            }
        }
    }
    let roots: Vec<usize> = (0..rows.len()).map(|n| find(&mut parent, n)).collect();
    let mut size: HashMap<usize, usize> = HashMap::new();
    for root in &roots {
        *size.entry(*root).or_default() += 1;
    }
    roots.iter().map(|root| size[root]).collect()
}

/// The most referees applied within any 24 hours.
fn busiest_day(rows: &[&RefereeMirror]) -> u64 {
    let mut times: Vec<DateTime<Utc>> = rows.iter().map(|row| row.applied_at).collect();
    times.sort();
    let mut best = 0;
    let mut start = 0;
    for end in 0..times.len() {
        while times[end] - times[start] >= Duration::hours(24) {
            start += 1;
        }
        best = best.max(end + 1 - start);
    }
    best as u64
}

/// `count / total` strictly over `threshold_bps`, without division.
fn over_share(count: u64, total: u64, threshold_bps: u32) -> bool {
    u128::from(count) * 10_000 > u128::from(total) * u128::from(threshold_bps)
}

/// Scores `rows` (one code's, or one referrer's across codes). Labels in
/// `exempt` never trip: an operator cleared the code with them exempted.
pub fn score_rows(
    rows: &[RefereeMirror],
    settings: &ReferralAbuseSettings,
    exempt: &BTreeSet<String>,
) -> CodeScore {
    let live: Vec<&RefereeMirror> = rows.iter().filter(|row| !row.voided).collect();
    let mut score = CodeScore {
        referees: live.len() as u64,
        ..CodeScore::default()
    };
    for label in [
        REFERRAL_REFUSED_SHARE,
        REFERRAL_CREDIT_HOLD_SHARE,
        REFERRAL_ADVANCE_FORFEIT_SHARE,
        REFERRAL_DUPLICATE_TRACES,
        REFERRAL_NEVER_SETTLES,
    ] {
        let count = live
            .iter()
            .filter(|row| row.signal_labels.iter().any(|held| held == label))
            .count() as u64;
        if count > 0 {
            score.counts.insert(label, count);
        }
    }
    let sizes = cluster_sizes(&live);
    let shared = sizes.iter().filter(|size| **size >= 2).count() as u64;
    if shared > 0 {
        score.counts.insert(REFERRAL_SHARED_IDENTITY, shared);
    }
    let largest = sizes.iter().copied().max().unwrap_or(0) as u64;
    let busiest = busiest_day(&live);
    if busiest > 0 {
        score.counts.insert(REFERRAL_BURST, busiest);
    }
    if score.referees < u64::from(settings.min_referees) {
        return score;
    }
    let n = score.referees;
    let count = |label: &str| score.counts.get(label).copied().unwrap_or(0);
    let mut tripped = Vec::new();
    for label in REFERRAL_SIGNAL_LABELS {
        let trips = match label {
            REFERRAL_REFUSED_SHARE => over_share(count(label), n, settings.refused_share_bps),
            REFERRAL_CREDIT_HOLD_SHARE => {
                over_share(count(label), n, settings.credit_hold_share_bps)
            }
            REFERRAL_ADVANCE_FORFEIT_SHARE => {
                over_share(count(label), n, settings.advance_forfeit_share_bps)
            }
            REFERRAL_SHARED_IDENTITY => largest >= u64::from(settings.shared_identity_cluster),
            REFERRAL_DUPLICATE_TRACES => over_share(count(label), n, settings.duplicate_share_bps),
            REFERRAL_BURST => busiest > u64::from(settings.burst_count),
            REFERRAL_NEVER_SETTLES => over_share(count(label), n, settings.never_settles_share_bps),
            _ => false,
        };
        if trips && !exempt.contains(label) {
            tripped.push(label);
        }
    }
    score.tripped = tripped;
    score
}

/// Scores one code over its own mirror rows.
pub fn score_code(
    code_index: &str,
    rows: &[RefereeMirror],
    settings: &ReferralAbuseSettings,
    exempt: &BTreeSet<String>,
) -> CodeScore {
    let own: Vec<RefereeMirror> = rows
        .iter()
        .filter(|row| row.code_index == code_index)
        .cloned()
        .collect();
    score_rows(&own, settings, exempt)
}

/// The referrer-level rollup: the same rates over every referee of every
/// code the referrer owns. Exemptions are per code, so none apply here.
pub fn score_referrer(rows: &[RefereeMirror], settings: &ReferralAbuseSettings) -> CodeScore {
    score_rows(rows, settings, &BTreeSet::new())
}

/// More than `trip_codes` codes suspended in the 24 hours up to `now`.
pub fn should_trip_dry_run(suspended_in_last_day: u64, settings: &ReferralAbuseSettings) -> bool {
    suspended_in_last_day > u64::from(settings.trip_codes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn row(n: usize, labels: &[&str]) -> RefereeMirror {
        RefereeMirror {
            referee_ref_hash: format!("sha256:{n:064x}"),
            code_index: "sha256:code".into(),
            // A week apart: no burst.
            applied_at: now() - Duration::days(7 * n as i64 + 1),
            signal_labels: labels.iter().map(|label| (*label).to_owned()).collect(),
            recipient_index: Some(format!("recipient-{n}")),
            subject_index: Some(format!("subject-{n}")),
            voided: false,
        }
    }

    fn facts() -> RefereeFacts {
        RefereeFacts {
            submissions: 10,
            refused: 0,
            credit_hold_or_reversal: false,
            advance_forfeited: false,
            duplicate_traces: false,
            settled_credit: true,
            account_created_at: now() - Duration::days(40),
        }
    }

    fn settings() -> ReferralAbuseSettings {
        ReferralAbuseSettings::default()
    }

    #[test]
    fn defaults_are_the_spec_table() {
        let s = settings();
        assert_eq!(s.min_referees, 5);
        assert_eq!(s.refused_share_bps, 4_000);
        assert_eq!(s.credit_hold_share_bps, 2_000);
        assert_eq!(s.advance_forfeit_share_bps, 2_000);
        assert_eq!(s.shared_identity_cluster, 3);
        assert_eq!(s.duplicate_share_bps, 2_000);
        assert_eq!(s.burst_count, 20);
        assert_eq!(s.never_settles_share_bps, 6_000);
        assert_eq!(s.never_settles_days, 30);
        assert_eq!(s.trip_codes, 2);
        assert_eq!(ReferralAbuseSettings::from_vars(|_| None), Ok(s));
    }

    #[test]
    fn a_malformed_threshold_refuses_and_names_its_setting() {
        for bad in ["", "-1", "10001", "1.5", " 5", "abc"] {
            let error = ReferralAbuseSettings::from_vars(|name| {
                (name == REFERRAL_ABUSE_REFUSED_SHARE_BPS_ENV).then(|| bad.to_owned())
            })
            .unwrap_err();
            assert_eq!(
                error.setting, REFERRAL_ABUSE_REFUSED_SHARE_BPS_ENV,
                "{bad:?}"
            );
            assert_eq!(error.label(), "referral_abuse_setting_invalid");
        }
        let zero_min = ReferralAbuseSettings::from_vars(|name| {
            (name == REFERRAL_ABUSE_MIN_REFEREES_ENV).then(|| "0".to_owned())
        });
        assert!(zero_min.is_err());
        let tuned = ReferralAbuseSettings::from_vars(|name| {
            (name == REFERRAL_ABUSE_BURST_COUNT_ENV).then(|| "50".to_owned())
        })
        .unwrap();
        assert_eq!(tuned.burst_count, 50);
    }

    #[test]
    fn referee_labels_follow_their_facts() {
        assert!(referee_signal_labels(&facts(), now(), &settings()).is_empty());

        let refused = RefereeFacts {
            refused: 6,
            ..facts()
        };
        assert_eq!(
            referee_signal_labels(&refused, now(), &settings()),
            vec![REFERRAL_REFUSED_SHARE]
        );
        // Exactly half is not most.
        let half = RefereeFacts {
            refused: 5,
            ..facts()
        };
        assert!(referee_signal_labels(&half, now(), &settings()).is_empty());

        let held = RefereeFacts {
            credit_hold_or_reversal: true,
            advance_forfeited: true,
            duplicate_traces: true,
            ..facts()
        };
        assert_eq!(
            referee_signal_labels(&held, now(), &settings()),
            vec![
                REFERRAL_CREDIT_HOLD_SHARE,
                REFERRAL_ADVANCE_FORFEIT_SHARE,
                REFERRAL_DUPLICATE_TRACES
            ]
        );
    }

    #[test]
    fn never_settles_waits_thirty_days_and_needs_uploads() {
        let unsettled = RefereeFacts {
            settled_credit: false,
            ..facts()
        };
        assert_eq!(
            referee_signal_labels(&unsettled, now(), &settings()),
            vec![REFERRAL_NEVER_SETTLES]
        );
        let young = RefereeFacts {
            account_created_at: now() - Duration::days(29),
            ..unsettled
        };
        assert!(referee_signal_labels(&young, now(), &settings()).is_empty());
        let idle = RefereeFacts {
            submissions: 0,
            ..unsettled
        };
        assert!(
            referee_signal_labels(&idle, now(), &settings()).is_empty(),
            "no uploads is not abuse"
        );
    }

    #[test]
    fn a_code_under_the_minimum_never_trips() {
        let rows: Vec<RefereeMirror> = (0..4).map(|n| row(n, &[REFERRAL_REFUSED_SHARE])).collect();
        let score = score_code("sha256:code", &rows, &settings(), &BTreeSet::new());
        assert_eq!(score.referees, 4);
        assert_eq!(score.counts[REFERRAL_REFUSED_SHARE], 4);
        assert!(!score.is_tripped());
    }

    #[test]
    fn each_share_trips_strictly_over_its_threshold() {
        // 2 of 5 refused is 40%: not over.
        let mut rows: Vec<RefereeMirror> = (0..5).map(|n| row(n, &[])).collect();
        rows[0].signal_labels = vec![REFERRAL_REFUSED_SHARE.into()];
        rows[1].signal_labels = vec![REFERRAL_REFUSED_SHARE.into()];
        assert!(!score_rows(&rows, &settings(), &BTreeSet::new()).is_tripped());
        rows[2].signal_labels = vec![REFERRAL_REFUSED_SHARE.into()];
        assert_eq!(
            score_rows(&rows, &settings(), &BTreeSet::new()).tripped,
            vec![REFERRAL_REFUSED_SHARE]
        );

        // 1 of 5 held is 20%: not over; 2 of 5 is.
        let mut rows: Vec<RefereeMirror> = (0..5).map(|n| row(n, &[])).collect();
        rows[0].signal_labels = vec![REFERRAL_CREDIT_HOLD_SHARE.into()];
        assert!(!score_rows(&rows, &settings(), &BTreeSet::new()).is_tripped());
        rows[1].signal_labels = vec![
            REFERRAL_CREDIT_HOLD_SHARE.into(),
            REFERRAL_DUPLICATE_TRACES.into(),
        ];
        rows[2].signal_labels = vec![REFERRAL_DUPLICATE_TRACES.into()];
        assert_eq!(
            score_rows(&rows, &settings(), &BTreeSet::new()).tripped,
            vec![REFERRAL_CREDIT_HOLD_SHARE, REFERRAL_DUPLICATE_TRACES]
        );

        // 3 of 5 never settled is 60%: not over; 4 of 5 is.
        let mut rows: Vec<RefereeMirror> = (0..5).map(|n| row(n, &[])).collect();
        for row in rows.iter_mut().take(3) {
            row.signal_labels = vec![REFERRAL_NEVER_SETTLES.into()];
        }
        assert!(!score_rows(&rows, &settings(), &BTreeSet::new()).is_tripped());
        rows[3].signal_labels = vec![REFERRAL_NEVER_SETTLES.into()];
        assert_eq!(
            score_rows(&rows, &settings(), &BTreeSet::new()).tripped,
            vec![REFERRAL_NEVER_SETTLES]
        );
    }

    #[test]
    fn three_referees_sharing_an_identity_trip_even_transitively() {
        let mut rows: Vec<RefereeMirror> = (0..6).map(|n| row(n, &[])).collect();
        // 0 and 1 share a recipient: a pair is not a cluster of 3.
        rows[1].recipient_index = rows[0].recipient_index.clone();
        let score = score_rows(&rows, &settings(), &BTreeSet::new());
        assert_eq!(score.counts[REFERRAL_SHARED_IDENTITY], 2);
        assert!(!score.is_tripped());
        // 2 shares a subject with 1: 0-1-2 is one cluster.
        rows[2].subject_index = rows[1].subject_index.clone();
        let score = score_rows(&rows, &settings(), &BTreeSet::new());
        assert_eq!(score.counts[REFERRAL_SHARED_IDENTITY], 3);
        assert_eq!(score.tripped, vec![REFERRAL_SHARED_IDENTITY]);
        // A recipient index never matches a subject index of the same text.
        let mut rows: Vec<RefereeMirror> = (0..6).map(|n| row(n, &[])).collect();
        rows[1].subject_index = rows[0].recipient_index.clone();
        rows[2].subject_index = rows[0].recipient_index.clone();
        let score = score_rows(&rows, &settings(), &BTreeSet::new());
        assert_eq!(score.counts[REFERRAL_SHARED_IDENTITY], 2);
        assert!(!score.is_tripped());
        // Rows with no link join nothing.
        let mut rows: Vec<RefereeMirror> = (0..6).map(|n| row(n, &[])).collect();
        for row in &mut rows {
            row.recipient_index = None;
            row.subject_index = None;
        }
        assert!(
            !score_rows(&rows, &settings(), &BTreeSet::new())
                .counts
                .contains_key(REFERRAL_SHARED_IDENTITY)
        );
    }

    #[test]
    fn a_burst_is_more_than_twenty_in_any_twenty_four_hours() {
        let at = |minutes: i64| now() - Duration::minutes(minutes);
        let mut rows: Vec<RefereeMirror> = (0..20)
            .map(|n| RefereeMirror {
                applied_at: at(60 * n as i64),
                ..row(n, &[])
            })
            .collect();
        let score = score_rows(&rows, &settings(), &BTreeSet::new());
        assert_eq!(score.counts[REFERRAL_BURST], 20);
        assert!(!score.is_tripped());
        // A 21st exactly 24 hours after the first falls outside its window.
        rows.push(RefereeMirror {
            applied_at: at(0) + Duration::hours(24) - Duration::minutes(60 * 19),
            ..row(20, &[])
        });
        let score = score_rows(&rows, &settings(), &BTreeSet::new());
        assert_eq!(score.counts[REFERRAL_BURST], 20);
        rows.push(RefereeMirror {
            applied_at: at(30),
            ..row(21, &[])
        });
        let score = score_rows(&rows, &settings(), &BTreeSet::new());
        assert_eq!(score.counts[REFERRAL_BURST], 21);
        assert_eq!(score.tripped, vec![REFERRAL_BURST]);
    }

    #[test]
    fn voided_referees_do_not_count_and_exempt_labels_never_trip() {
        let mut rows: Vec<RefereeMirror> =
            (0..6).map(|n| row(n, &[REFERRAL_REFUSED_SHARE])).collect();
        for row in rows.iter_mut().take(2) {
            row.voided = true;
        }
        let score = score_rows(&rows, &settings(), &BTreeSet::new());
        assert_eq!(score.referees, 4);
        assert!(!score.is_tripped(), "4 live referees is under the minimum");

        let rows: Vec<RefereeMirror> = (0..6).map(|n| row(n, &[REFERRAL_REFUSED_SHARE])).collect();
        let exempt = BTreeSet::from([REFERRAL_REFUSED_SHARE.to_owned()]);
        let score = score_rows(&rows, &settings(), &exempt);
        assert_eq!(score.counts[REFERRAL_REFUSED_SHARE], 6);
        assert!(!score.is_tripped());
    }

    #[test]
    fn a_code_scores_only_its_own_rows_and_the_referrer_scores_all() {
        let mut rows: Vec<RefereeMirror> =
            (0..6).map(|n| row(n, &[REFERRAL_REFUSED_SHARE])).collect();
        for row in rows.iter_mut().take(3) {
            row.code_index = "sha256:other".into();
        }
        assert!(!score_code("sha256:code", &rows, &settings(), &BTreeSet::new()).is_tripped());
        assert!(!score_code("sha256:other", &rows, &settings(), &BTreeSet::new()).is_tripped());
        assert_eq!(
            score_referrer(&rows, &settings()).tripped,
            vec![REFERRAL_REFUSED_SHARE]
        );
    }

    #[test]
    fn the_stream_trips_on_more_than_two_suspensions() {
        assert!(!should_trip_dry_run(2, &settings()));
        assert!(should_trip_dry_run(3, &settings()));
    }

    #[test]
    fn every_label_is_a_safe_referral_label() {
        for label in REFERRAL_SIGNAL_LABELS
            .iter()
            .chain(REFERRAL_HOLD_LABELS.iter())
            .chain(REFERRAL_EVIDENCE_LABELS.iter())
            .chain([REFERRAL_TRIPPED].iter())
        {
            assert!(label.starts_with("referral_"), "{label}");
            assert!(label.len() <= 64);
            assert!(
                label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            );
        }
        for label in REFERRAL_HOLD_LABELS {
            assert!(hold_holds_referral_stream_only(label));
        }
        for earned in [
            "payout_over_ceiling",
            "payout_reversal_unpriced",
            "payout_link_unreadable",
            crate::db::PAYOUT_ACCOUNT_CREDIT_HOLD,
        ] {
            assert!(!hold_holds_referral_stream_only(earned));
        }
    }

    /// Owner decision 2026-10-29: a shared network (or payout org or
    /// subject with the referrer) holds the referral stream, but colleagues
    /// on one office network are the main legitimate case, so evidence
    /// labels never trip a code or a referrer, however many referees carry
    /// them.
    #[test]
    fn evidence_labels_never_trip_a_code_or_a_referrer() {
        let rows: Vec<RefereeMirror> = (0..10).map(|n| row(n, &REFERRAL_EVIDENCE_LABELS)).collect();
        let code = score_code("sha256:code", &rows, &settings(), &BTreeSet::new());
        assert!(!code.is_tripped(), "{code:?}");
        for label in REFERRAL_EVIDENCE_LABELS {
            assert!(!code.counts.contains_key(label), "{label}");
        }
        assert!(!score_referrer(&rows, &settings()).is_tripped());
        for label in REFERRAL_EVIDENCE_LABELS {
            assert!(!REFERRAL_SIGNAL_LABELS.contains(&label), "{label}");
        }
        // Duplicate traces, the strong signal, still trips over 20%.
        let mut rows = rows;
        for row in rows.iter_mut().take(3) {
            row.signal_labels.push(REFERRAL_DUPLICATE_TRACES.to_owned());
        }
        assert_eq!(
            score_code("sha256:code", &rows, &settings(), &BTreeSet::new()).tripped,
            vec![REFERRAL_DUPLICATE_TRACES]
        );
    }

    #[test]
    fn mirror_debug_prints_no_index_or_hash() {
        let rendered = format!("{:?}", row(1, &[REFERRAL_BURST]));
        assert!(rendered.contains(REFERRAL_BURST));
        assert!(!rendered.contains("recipient-1"));
        assert!(!rendered.contains("subject-1"));
        assert!(!rendered.contains("sha256:"));
    }
}
