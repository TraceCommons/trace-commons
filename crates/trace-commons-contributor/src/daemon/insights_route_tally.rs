//! The per-session route tally: where a feed T session's proxy calls went,
//! as counts, folded from the proxy ledger this daemon already reads.
//!
//! - Owner question Q3 (default taken): the tally is persisted in the counter
//!   store, counts only, so the split outlives the ledger's 24-hour window.
//!   It is kept for the store's weeks, capped like its rows, and removed with
//!   the store on unenroll.
//! - Owner decision D15, extended to feed T (default taken): a tally is keyed
//!   by the keyed harness-session digest under the counter key, the same
//!   digest a counter row stores, so the two join inside the daemon and
//!   nowhere else.
//! - Owner decision D3, open and unchanged: nothing is folded while
//!   `insights_ledger_feed` is off.
//! - Owner decision D5, open: `cost_usd` is never read.
//!
//! Every label here comes from IronWire's proof label and never from a
//! backend or model name. Only `verified` is proof; `gateway_only` names the
//! relay, not the model. A session with no tally is `unobserved`, never
//! "outside": no proxy record is not evidence of where a call went.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use trace_commons_protocol::insights_usage_series::{
    DigestKey, KeyedDigest, harness_session_digest,
};

use super::inference_map::{Speakers, attribute};
use crate::insights::ledger_series::call_tokens;
use crate::routing::{ProofStatus, RoutedExchange};
use crate::source::{SOURCE_CLAUDE_CODE, SOURCE_CODEX};

/// One figure per proof bucket.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouteBuckets {
    /// `verified`, the only proof.
    pub verified: u64,
    /// `gateway_only`, `unattested`, `pending` or `unavailable`: through the
    /// proxy's NEAR AI route, not proven.
    pub routed_unverified: u64,
    /// `failed`: a receipt that did not check out.
    pub check_failed: u64,
    /// `outside`: not a NEAR AI backend.
    pub outside: u64,
    /// No proof label recorded (an older proxy, or one this build does not
    /// know). Not `outside`.
    pub unrecorded: u64,
}

impl RouteBuckets {
    fn slot(&mut self, proof: Option<ProofStatus>) -> &mut u64 {
        match proof {
            Some(ProofStatus::Verified) => &mut self.verified,
            Some(
                ProofStatus::GatewayOnly
                | ProofStatus::Unattested
                | ProofStatus::Pending
                | ProofStatus::Unavailable,
            ) => &mut self.routed_unverified,
            Some(ProofStatus::Failed) => &mut self.check_failed,
            Some(ProofStatus::Outside) => &mut self.outside,
            None => &mut self.unrecorded,
        }
    }

    fn routed(&self) -> u64 {
        self.verified
            .saturating_add(self.routed_unverified)
            .saturating_add(self.check_failed)
    }

    fn labelled(&self) -> u64 {
        self.routed().saturating_add(self.outside)
    }

    /// Every call or token, labelled or not.
    pub(crate) fn total(&self) -> u64 {
        self.labelled().saturating_add(self.unrecorded)
    }
}

/// One session's tally. Counts and one time only: never an id, a digest of
/// one, a backend, a model, a price or a body reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouteTally {
    /// Calls per bucket. The category is drawn from these, never from
    /// tokens, so a call whose counters are unknown still counts where it
    /// went.
    pub calls: RouteBuckets,
    /// Tokens per bucket, from calls whose counters are all known:
    /// uncached input, cache reads, cache writes and output, normalized per
    /// facade as feed L does.
    pub tokens: RouteBuckets,
    /// Calls counted in `calls` whose tokens are unknown, and so in no token
    /// bucket. Never read as zero tokens.
    pub calls_without_counts: u64,
    /// The latest call folded, for ageing the tally out.
    pub last_call_at: DateTime<Utc>,
}

/// Where a session's calls went, checked in this order, first match wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteCategory {
    /// No tally. Never "outside".
    Unobserved,
    /// Calls, none with a proof label.
    Unrecorded,
    /// Every labelled call `outside`.
    Outside,
    /// Labelled calls both outside and through the route.
    Mixed,
    /// All labelled calls through the route, at least one `failed`.
    CheckFailed,
    /// All labelled calls `verified`.
    RoutedVerified,
    /// All labelled calls through the route, at least one not proven.
    RoutedUnverified,
}

impl RouteCategory {
    /// The wire spelling. None of them says "private".
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "read by the insights_week answer's routing field, the next step"
        )
    )]
    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unobserved => "unobserved",
            Self::Unrecorded => "unrecorded",
            Self::Outside => "outside",
            Self::Mixed => "mixed",
            Self::CheckFailed => "check_failed",
            Self::RoutedVerified => "routed_verified",
            Self::RoutedUnverified => "routed_unverified",
        }
    }
}

/// Why a category needs a qualifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteReason {
    /// Some calls carry a proof label and some do not; the category is the
    /// labelled calls'.
    SomeCallsUnrecorded,
}

impl RouteReason {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "read by the insights_week answer's routing field, the next step"
        )
    )]
    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SomeCallsUnrecorded => "some_calls_unrecorded",
        }
    }
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "read by the insights_week answer's routing field, the next step"
    )
)]
/// A session's category, and its reason when some calls are unrecorded.
#[must_use]
pub(crate) fn route_category(tally: Option<&RouteTally>) -> (RouteCategory, Option<RouteReason>) {
    let Some(calls) = tally.map(|tally| tally.calls) else {
        return (RouteCategory::Unobserved, None);
    };
    if calls.total() == 0 {
        return (RouteCategory::Unobserved, None);
    }
    if calls.labelled() == 0 {
        return (RouteCategory::Unrecorded, None);
    }
    let reason = (calls.unrecorded > 0).then_some(RouteReason::SomeCallsUnrecorded);
    let category = if calls.routed() == 0 {
        RouteCategory::Outside
    } else if calls.outside > 0 {
        RouteCategory::Mixed
    } else if calls.check_failed > 0 {
        RouteCategory::CheckFailed
    } else if calls.routed_unverified == 0 {
        RouteCategory::RoutedVerified
    } else {
        RouteCategory::RoutedUnverified
    };
    (category, reason)
}

/// The rows a fold takes, oldest id first, and where its cursor ends.
#[derive(Debug)]
pub(crate) struct FoldPlan<'a> {
    pub rows: Vec<&'a RoutedExchange>,
    pub through: Option<i64>,
}

impl FoldPlan<'_> {
    /// Whether attributing the rows needs the connected tools: only when one
    /// carries a session id at all.
    pub(crate) fn needs_speakers(&self) -> bool {
        self.rows.iter().any(|row| session_of(row).is_some())
    }
}

/// What one fold did. Counts only.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct FoldSummary {
    pub folded: usize,
    pub skipped: usize,
}

/// The rows of `window` not yet folded, and the cursor after them.
///
/// - A row without an id is never taken: nothing could say it was folded
///   once only.
/// - The cursor is the highest id folded through. A window whose highest id
///   is below it is a ledger that started over, the rule
///   `IronWireLedger::take_added_rows` applies; unlike that pulse, which
///   re-baselines and hands out nothing, a tally takes the new ledger's rows,
///   since each is a call not counted before.
/// - Known gap, shared with `take_added_rows`: a ledger that started over and
///   climbed past the cursor between two folds cannot be told from one that
///   only grew, so its rows at or below the cursor are never folded.
/// - An empty window leaves the cursor where it is.
#[must_use]
pub(crate) fn fold_plan(window: &[RoutedExchange], folded_through: Option<i64>) -> FoldPlan<'_> {
    let highest = window.iter().filter_map(|row| row.id).max();
    let from = match (folded_through, highest) {
        (Some(cursor), Some(highest)) if highest < cursor => None,
        (cursor, _) => cursor,
    };
    let mut rows: Vec<&RoutedExchange> = window
        .iter()
        .filter(|row| row.id.is_some_and(|id| from.is_none_or(|from| id > from)))
        .collect();
    rows.sort_by_key(|row| row.id);
    FoldPlan {
        rows,
        through: highest.or(folded_through),
    }
}

/// Fold `plan`'s rows into `routing`, keyed by the harness-session digest
/// under `key`.
///
/// A row is skipped when its session id is missing or empty (the guard
/// `routing::enriched` applies before any join), or when no single tool can
/// be named for it, or the tool is not one feed T counts. The tool is never
/// guessed.
pub(crate) fn fold_into(
    routing: &mut BTreeMap<KeyedDigest, RouteTally>,
    key: &DigestKey,
    plan: &FoldPlan<'_>,
    speakers: &Speakers,
) -> FoldSummary {
    let mut summary = FoldSummary::default();
    for row in &plan.rows {
        let Some(session) = session_of(row) else {
            summary.skipped += 1;
            continue;
        };
        let tool = attribute(row, speakers);
        if tool != SOURCE_CLAUDE_CODE && tool != SOURCE_CODEX {
            summary.skipped += 1;
            continue;
        }
        let digest = harness_session_digest(key, tool, session);
        let tally = routing.entry(digest).or_insert_with(|| RouteTally {
            calls: RouteBuckets::default(),
            tokens: RouteBuckets::default(),
            calls_without_counts: 0,
            last_call_at: row.started_at,
        });
        let calls = tally.calls.slot(row.proof);
        *calls = calls.saturating_add(1);
        match call_tokens(row) {
            Some(tokens) => {
                let bucket = tally.tokens.slot(row.proof);
                *bucket = bucket.saturating_add(tokens);
            }
            None => tally.calls_without_counts = tally.calls_without_counts.saturating_add(1),
        }
        tally.last_call_at = tally.last_call_at.max(row.started_at);
        summary.folded += 1;
    }
    summary
}

/// Drop tallies no stored row matches once their last call is older than
/// `cutoff`, and drop `gone` (sessions whose rows were removed for a Never
/// folder) at once; then, past `max`, the oldest by last call.
pub(crate) fn prune(
    routing: &mut BTreeMap<KeyedDigest, RouteTally>,
    matched: &BTreeSet<KeyedDigest>,
    gone: &BTreeSet<KeyedDigest>,
    cutoff: DateTime<Utc>,
    max: usize,
) {
    routing.retain(|digest, tally| {
        !gone.contains(digest) && (matched.contains(digest) || tally.last_call_at >= cutoff)
    });
    if routing.len() <= max {
        return;
    }
    let mut oldest: Vec<(DateTime<Utc>, KeyedDigest)> = routing
        .iter()
        .map(|(digest, tally)| (tally.last_call_at, *digest))
        .collect();
    oldest.sort();
    let excess = routing.len() - max;
    for (_, digest) in oldest.into_iter().take(excess) {
        routing.remove(&digest);
    }
}

/// The session id a row carries; an empty one is none.
fn session_of(row: &RoutedExchange) -> Option<&str> {
    row.client_session_id
        .as_deref()
        .filter(|session| !session.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use trace_commons_protocol::insights_usage_series::{DigestKeyStore, InMemoryDigestKeyStore};

    /// The convention `inference_map`'s tests use for an id that must never
    /// cross.
    const SESSION: &str = "SESSION-SECRET";

    fn key() -> DigestKey {
        InMemoryDigestKeyStore::with_seed([3; 32])
            .load_or_create()
            .unwrap()
    }

    fn at(minute: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 14, 9, 0, 0).unwrap() + Duration::minutes(minute)
    }

    /// A Codex call on its own endpoint, so the tool is named without any
    /// connected speaker: 100 input of which 40 cached, 10 output, no cache
    /// write.
    fn codex(id: i64, proof: Option<ProofStatus>) -> RoutedExchange {
        RoutedExchange {
            id: Some(id),
            started_at: at(id),
            client_session_id: Some(SESSION.to_string()),
            facade: "openai".to_string(),
            path: Some("/v1/responses".to_string()),
            backend: "SECRET-BACKEND".to_string(),
            input_tokens: Some(100),
            cache_read_tokens: Some(40),
            cache_write_tokens: Some(0),
            output_tokens: Some(10),
            cost_usd: Some(1.0),
            proof,
            ..Default::default()
        }
    }

    fn fold(
        routing: &mut BTreeMap<KeyedDigest, RouteTally>,
        cursor: &mut Option<i64>,
        window: &[RoutedExchange],
    ) -> FoldSummary {
        let plan = fold_plan(window, *cursor);
        let summary = fold_into(routing, &key(), &plan, &Vec::new());
        *cursor = plan.through;
        summary
    }

    fn only(routing: &BTreeMap<KeyedDigest, RouteTally>) -> &RouteTally {
        assert_eq!(routing.len(), 1);
        routing
            .get(&harness_session_digest(&key(), SOURCE_CODEX, SESSION))
            .expect("keyed by the harness-session digest")
    }

    #[test]
    fn every_proof_label_lands_in_its_bucket() {
        let cases: [(Option<ProofStatus>, fn(&RouteBuckets) -> u64); 8] = [
            (Some(ProofStatus::Verified), |b| b.verified),
            (Some(ProofStatus::GatewayOnly), |b| b.routed_unverified),
            (Some(ProofStatus::Unattested), |b| b.routed_unverified),
            (Some(ProofStatus::Pending), |b| b.routed_unverified),
            (Some(ProofStatus::Unavailable), |b| b.routed_unverified),
            (Some(ProofStatus::Failed), |b| b.check_failed),
            (Some(ProofStatus::Outside), |b| b.outside),
            (None, |b| b.unrecorded),
        ];
        for (proof, bucket) in cases {
            let mut routing = BTreeMap::new();
            let summary = fold(&mut routing, &mut None, &[codex(1, proof)]);
            assert_eq!(
                summary,
                FoldSummary {
                    folded: 1,
                    skipped: 0
                },
                "{proof:?}"
            );
            let tally = only(&routing);
            assert_eq!(bucket(&tally.calls), 1, "{proof:?}");
            assert_eq!(tally.calls.total(), 1, "{proof:?}");
            // OpenAI's cached input is inside input: 60 + 40 + 0 + 10.
            assert_eq!(bucket(&tally.tokens), 110, "{proof:?}");
            assert_eq!(tally.tokens.total(), 110, "{proof:?}");
            assert_eq!(tally.calls_without_counts, 0);
            assert_eq!(tally.last_call_at, at(1));
        }
    }

    #[test]
    fn anthropic_counters_add_as_recorded() {
        let row = RoutedExchange {
            facade: "anthropic".to_string(),
            path: Some("/v1/messages".to_string()),
            input_tokens: Some(3),
            cache_read_tokens: Some(10),
            cache_write_tokens: Some(5),
            output_tokens: Some(2),
            ..codex(1, Some(ProofStatus::Verified))
        };
        let mut routing = BTreeMap::new();
        fold(&mut routing, &mut None, &[row]);
        let tally = routing
            .get(&harness_session_digest(&key(), SOURCE_CLAUDE_CODE, SESSION))
            .expect("Claude Code's digest");
        assert_eq!(tally.tokens.verified, 20);
    }

    #[test]
    fn a_call_with_unknown_counters_counts_where_it_went_and_adds_no_tokens() {
        let mut unknown = codex(2, Some(ProofStatus::Verified));
        unknown.output_tokens = None;
        let mut routing = BTreeMap::new();
        fold(
            &mut routing,
            &mut None,
            &[codex(1, Some(ProofStatus::Outside)), unknown],
        );
        let tally = only(&routing);
        assert_eq!(tally.calls.verified, 1);
        assert_eq!(tally.tokens.verified, 0);
        assert_eq!(tally.calls_without_counts, 1);
        assert_eq!(route_category(Some(tally)).0, RouteCategory::Mixed);
    }

    #[test]
    fn rows_that_cannot_be_joined_or_named_are_skipped() {
        let mut empty = codex(1, Some(ProofStatus::Verified));
        empty.client_session_id = Some(String::new());
        let mut missing = codex(2, Some(ProofStatus::Verified));
        missing.client_session_id = None;
        // An endpoint no connection writes: the tool is unknown.
        let mut unnamed = codex(3, Some(ProofStatus::Verified));
        unnamed.path = Some("/v1/chat/completions".to_string());
        // A facade no tool speaks, with no endpoint recorded.
        let mut unspoken = codex(4, Some(ProofStatus::Verified));
        unspoken.path = None;
        let mut no_id = codex(5, Some(ProofStatus::Verified));
        no_id.id = None;
        let mut routing = BTreeMap::new();
        let mut cursor = None;
        let summary = fold(
            &mut routing,
            &mut cursor,
            &[empty, missing, unnamed, unspoken, no_id],
        );
        // The id-less row is not even a candidate.
        assert_eq!(
            summary,
            FoldSummary {
                folded: 0,
                skipped: 4
            }
        );
        assert!(routing.is_empty());
        assert_eq!(cursor, Some(4));
    }

    #[test]
    fn an_old_proxy_row_is_named_by_the_one_connected_speaker() {
        let mut row = codex(1, Some(ProofStatus::Verified));
        row.path = None;
        let plan = fold_plan(std::slice::from_ref(&row), None);
        let mut routing = BTreeMap::new();
        let summary = fold_into(&mut routing, &key(), &plan, &vec![(SOURCE_CODEX, "openai")]);
        assert_eq!(summary.folded, 1);
        only(&routing);
    }

    #[test]
    fn folding_the_same_window_twice_counts_each_call_once() {
        let window: Vec<RoutedExchange> = (1..=3)
            .map(|id| codex(id, Some(ProofStatus::Verified)))
            .collect();
        let mut routing = BTreeMap::new();
        let mut cursor = None;
        assert_eq!(fold(&mut routing, &mut cursor, &window).folded, 3);
        assert_eq!(fold(&mut routing, &mut cursor, &window).folded, 0);
        assert_eq!(only(&routing).calls.verified, 3);
        assert_eq!(cursor, Some(3));

        // A later window that grew by one, and lost its oldest row to age.
        let mut grown = window[1..].to_vec();
        grown.push(codex(4, Some(ProofStatus::Verified)));
        assert_eq!(fold(&mut routing, &mut cursor, &grown).folded, 1);
        assert_eq!(only(&routing).calls.verified, 4);
        assert_eq!(cursor, Some(4));

        // An empty window keeps the cursor.
        assert_eq!(fold(&mut routing, &mut cursor, &[]).folded, 0);
        assert_eq!(cursor, Some(4));
    }

    #[test]
    fn a_ledger_that_started_over_is_folded_from_its_start() {
        let mut routing = BTreeMap::new();
        let mut cursor = None;
        let window: Vec<RoutedExchange> = (1..=5)
            .map(|id| codex(id, Some(ProofStatus::Verified)))
            .collect();
        fold(&mut routing, &mut cursor, &window);
        assert_eq!(cursor, Some(5));
        // The proxy's ledger started over: ids 1 and 2 again, new calls.
        let restarted = [
            codex(1, Some(ProofStatus::Outside)),
            codex(2, Some(ProofStatus::Outside)),
        ];
        assert_eq!(fold(&mut routing, &mut cursor, &restarted).folded, 2);
        assert_eq!(cursor, Some(2));
        let tally = only(&routing);
        assert_eq!((tally.calls.verified, tally.calls.outside), (5, 2));
        // And from there it grows as any ledger does.
        assert_eq!(fold(&mut routing, &mut cursor, &restarted).folded, 0);
    }

    fn tally(calls: RouteBuckets) -> RouteTally {
        RouteTally {
            calls,
            tokens: RouteBuckets::default(),
            calls_without_counts: 0,
            last_call_at: at(0),
        }
    }

    fn buckets(v: u64, u: u64, f: u64, o: u64, n: u64) -> RouteBuckets {
        RouteBuckets {
            verified: v,
            routed_unverified: u,
            check_failed: f,
            outside: o,
            unrecorded: n,
        }
    }

    #[test]
    fn categories_follow_the_precedence() {
        use RouteCategory::*;
        let unrecorded = Some(RouteReason::SomeCallsUnrecorded);
        let cases = [
            (buckets(0, 0, 0, 0, 0), (Unobserved, None)),
            (buckets(0, 0, 0, 0, 3), (Unrecorded, None)),
            (buckets(0, 0, 0, 2, 0), (Outside, None)),
            (buckets(0, 0, 0, 2, 1), (Outside, unrecorded)),
            (buckets(1, 0, 0, 2, 0), (Mixed, None)),
            (buckets(0, 0, 1, 1, 0), (Mixed, None)),
            (buckets(1, 1, 1, 0, 0), (CheckFailed, None)),
            (buckets(0, 0, 1, 0, 1), (CheckFailed, unrecorded)),
            (buckets(4, 0, 0, 0, 0), (RoutedVerified, None)),
            (buckets(4, 0, 0, 0, 1), (RoutedVerified, unrecorded)),
            (buckets(4, 1, 0, 0, 0), (RoutedUnverified, None)),
            (buckets(0, 1, 0, 0, 2), (RoutedUnverified, unrecorded)),
        ];
        for (calls, expected) in cases {
            assert_eq!(route_category(Some(&tally(calls))), expected, "{calls:?}");
        }
        assert_eq!(route_category(None), (Unobserved, None));
    }

    #[test]
    fn gateway_only_alone_is_never_verified() {
        let mut routing = BTreeMap::new();
        fold(
            &mut routing,
            &mut None,
            &[codex(1, Some(ProofStatus::GatewayOnly))],
        );
        assert_eq!(
            route_category(Some(only(&routing))),
            (RouteCategory::RoutedUnverified, None)
        );
    }

    #[test]
    fn wire_spellings_never_say_private() {
        use RouteCategory::*;
        for category in [
            Unobserved,
            Unrecorded,
            Outside,
            Mixed,
            CheckFailed,
            RoutedVerified,
            RoutedUnverified,
        ] {
            assert!(!category.as_str().to_lowercase().contains("private"));
        }
        assert_eq!(
            RouteReason::SomeCallsUnrecorded.as_str(),
            "some_calls_unrecorded"
        );
    }

    #[test]
    fn a_tally_holds_no_id_backend_or_price() {
        let mut routing = BTreeMap::new();
        fold(
            &mut routing,
            &mut None,
            &[codex(1, Some(ProofStatus::Verified))],
        );
        let text = serde_json::to_string(only(&routing)).unwrap();
        for leak in [SESSION, "SECRET-BACKEND", "cost", "1.0"] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
    }

    #[test]
    fn prune_ages_out_unmatched_tallies_drops_gone_ones_and_caps() {
        let digest = |n: u8| KeyedDigest([n; 32]);
        let mut routing: BTreeMap<KeyedDigest, RouteTally> = (1..=4)
            .map(|n| {
                let mut t = tally(buckets(1, 0, 0, 0, 0));
                t.last_call_at = at(i64::from(n));
                (digest(n), t)
            })
            .collect();
        let cutoff = at(3);
        // 1 is old and matched: kept. 2 is old and unmatched: dropped.
        // 4 is gone: dropped whatever its age.
        let matched = BTreeSet::from([digest(1)]);
        let gone = BTreeSet::from([digest(4)]);
        prune(&mut routing, &matched, &gone, cutoff, 10);
        assert_eq!(
            routing.keys().copied().collect::<Vec<_>>(),
            vec![digest(1), digest(3)]
        );
        // Past the cap the oldest goes, matched or not.
        prune(&mut routing, &matched, &BTreeSet::new(), cutoff, 1);
        assert_eq!(routing.keys().copied().collect::<Vec<_>>(), vec![digest(3)]);
    }
}
