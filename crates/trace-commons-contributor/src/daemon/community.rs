//! This contributor's own line on the public community roster.
//!
//! The server already computes the whole roster: `GET
//! /v1/community/leaderboard` is public and unauthenticated, and answers with
//! a pre-rendered snapshot. Nothing here recomputes any of it. This module
//! fetches that snapshot, finds the row belonging to the handle this machine
//! has been told to watch, and reduces it to the few figures the desktop
//! clients draw.
//!
//! Two rules shape everything below.
//!
//! **Absent beats approximate.** Every figure here is public: it is the exact
//! boundary of what the world can see about a person. A row that cannot be
//! found, a snapshot that cannot be parsed, a metric this daemon does not
//! recognize -- each of those produces *no standing at all* rather than a
//! partially-filled one. A zero or a dash inside this panel would be a claim
//! about someone's public standing that the daemon never received.
//!
//! **The reading is a record, never a currency.** Nothing here derives a
//! trend, a streak, a rank change, or a projection from the snapshot. The
//! daemon reports what the roster says and stops.

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// The public roster route on ingest. Unauthenticated by design, so this is
/// the one contributor-side network call that mints no claim.
const LEADERBOARD_PATH: &str = "/v1/community/leaderboard";

/// The metric the client-facing `novelty_credit` field is named for. The
/// snapshot states its own metric; see `map_entry` for why a snapshot
/// computed under any other one yields no standing.
const EXPECTED_METRIC: &str = "novelty_credit";

/// How old a cached standing may be before the daemon stops serving it.
///
/// The server publishes a 15-minute withdrawal bound and enforces it itself:
/// it refuses to serve a snapshot older than that, so removal from the roster
/// is already a server-side guarantee. This is the local backstop for the
/// other failure -- a poller that has been failing for hours while the cached
/// answer keeps being handed to the clients as if it were current.
///
/// Deliberately twice the published bound rather than equal to it. A snapshot
/// the server was willing to serve may already be nearly 15 minutes old when
/// this daemon fetches it, so a 15-minute local bound would blank a section
/// the server itself considers current, on almost every poll.
const STANDING_MAX_AGE_SECS: i64 = 1_800;

/// Request timeout. Matches the issuer client's; this is one small GET
/// against the same deployment.
const REQUEST_TIMEOUT_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// The server's wire shape.
//
// A deliberately partial mirror of `CommunitySnapshotContents` in
// `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`: only the
// fields this daemon reduces are named, so a field added to the snapshot
// cannot break the poll.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct SnapshotEntry {
    rank: i64,
    display_handle: String,
    /// The snapshot's primary metric, whatever `Snapshot::metric` says it is.
    score: f64,
    accepted_count: i64,
    /// Decimal in `0..=1`.
    accept_rate: f64,
    public_since: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct SnapshotPrivacy {
    /// Label-only control names that withheld the corpus aggregates. Empty
    /// means they were computed.
    #[serde(default)]
    analytics_withheld_controls: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Snapshot {
    computed_at: DateTime<Utc>,
    /// The rolling window the figures are measured over, in the server's own
    /// words ("7d").
    window: String,
    metric: String,
    #[serde(default)]
    privacy: Option<SnapshotPrivacy>,
    #[serde(default)]
    leaderboard: Vec<SnapshotEntry>,
    /// `None` when the corpus aggregates were withheld or never computed.
    #[serde(default)]
    analytics: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// The clients' shape.
// ---------------------------------------------------------------------------

/// One contributor's standing on the public roster, in the names the desktop
/// clients already parse (`RosterStanding` in the GTK client, `RosterSnapshot`
/// on macOS).
///
/// The names differ from the server's on purpose, and the mapping is stated
/// field by field here and in `map_entry`, because the two vocabularies are
/// not interchangeable: the server names a column for the table it lives in,
/// the client names a figure for the sentence it appears in.
///
/// No `profile_url` is emitted. Nothing on this machine is configured with the
/// community site's base address, and both clients treat an absent URL as
/// "draw no link" -- which is the right outcome, and better than a guessed
/// link that goes nowhere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommunityStanding {
    /// Server `rank`. `None` when the roster reported a non-positive
    /// position, which the clients render as a dash rather than as `#0`.
    pub rank: Option<u32>,
    /// Server `score`, under the `novelty_credit` metric.
    pub novelty_credit: f64,
    /// Server `accepted_count`, which is a count *within the snapshot's
    /// window*; the client name says so, the server name does not.
    ///
    /// Not an `Option`, unlike `rank` and `accept_rate`: both clients parse
    /// it as a bare number, so there is no absent form to send. A count that
    /// does not fit therefore withholds the whole standing in `map_entry`
    /// rather than being approximated into this field.
    pub accepted_in_window: u32,
    /// Server `accept_rate`. `None` when the value was outside `0..=1`, since
    /// the clients render it as a percentage and a nonsensical one is worse
    /// than none.
    pub accept_rate: Option<f64>,
    /// Server `window`.
    pub window_label: String,
    pub public_since: Option<DateTime<Utc>>,
    /// Server `computed_at`. The clients state the snapshot's age, because a
    /// public figure that is quietly stale is worse than one that says how old
    /// it is.
    pub snapshot_at: Option<DateTime<Utc>>,
    /// Whether the corpus aggregates were withheld. True whenever the
    /// snapshot carries no analytics or names a withholding control -- and
    /// true by default anywhere the answer is unclear, because withheld is
    /// the conservative reading.
    pub analytics_withheld: bool,
}

impl CommunityStanding {
    /// Whether this cached standing is still fit to serve at `now`. See
    /// `STANDING_MAX_AGE_SECS`.
    pub fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        let Some(at) = self.snapshot_at else {
            // A standing with no timestamp cannot be shown to be within the
            // withdrawal bound, so it is not served.
            return false;
        };
        now.signed_duration_since(at) < Duration::seconds(STANDING_MAX_AGE_SECS)
    }
}

/// Reduce a roster snapshot body to `display_handle`'s standing.
///
/// `None` covers every "no standing" case alike -- the body did not parse,
/// the snapshot is older than the daemon will serve, the handle is not on the
/// roster, the snapshot measures a metric this daemon cannot name -- because
/// the clients render all of them the same way: the section does not appear.
pub fn standing_from_payload(
    body: &serde_json::Value,
    display_handle: &str,
    now: DateTime<Utc>,
) -> Option<CommunityStanding> {
    let snapshot: Snapshot = match serde_json::from_value(body.clone()) {
        Ok(s) => s,
        Err(_) => {
            // Fixed label, no body: this is a public document, but it is
            // still not something to spill into a journal.
            tracing::warn!("community roster snapshot did not parse");
            return None;
        }
    };
    map_entry(&snapshot, display_handle, now)
}

fn map_entry(
    snapshot: &Snapshot,
    display_handle: &str,
    now: DateTime<Utc>,
) -> Option<CommunityStanding> {
    // The client field is named for the metric it carries. A score computed
    // under some other metric, rendered under the name `novelty_credit`,
    // would be a false label on a public figure -- so an unrecognized metric
    // means this daemon does not know what the number is, and reports
    // nothing rather than mislabelling it.
    if snapshot.metric != EXPECTED_METRIC {
        return None;
    }
    if now.signed_duration_since(snapshot.computed_at) >= Duration::seconds(STANDING_MAX_AGE_SECS) {
        return None;
    }
    // Exact match. Handles are claimed verbatim through
    // `submit::set_profile`, so a case-insensitive match here could bind this
    // contributor's panel to a *different* person's row.
    let entry = snapshot
        .leaderboard
        .iter()
        .find(|e| e.display_handle == display_handle)?;

    let withheld = match &snapshot.privacy {
        // Aggregates present and no control named: they were computed.
        Some(p) => snapshot.analytics.is_none() || !p.analytics_withheld_controls.is_empty(),
        // No privacy block at all is the unclear case, and unclear reads as
        // withheld.
        None => true,
    };

    // `accepted_in_window` has no absent form on the wire -- both clients
    // parse it as a bare number -- so a count this daemon cannot represent
    // suppresses the whole standing rather than being rounded into one.
    // `unwrap_or(0)` here used to render a negative or out-of-range count as
    // a definite "0 accepted", which is a false claim about someone's public
    // standing and the exact thing the module's absent-beats-approximate
    // rule exists to prevent. `rank` and `accept_rate` can drop the figure
    // alone because each of them is an `Option`; this one cannot, so it
    // drops the section.
    let accepted_in_window = u32::try_from(entry.accepted_count).ok()?;

    Some(CommunityStanding {
        rank: u32::try_from(entry.rank).ok().filter(|r| *r > 0),
        novelty_credit: entry.score,
        accepted_in_window,
        accept_rate: Some(entry.accept_rate).filter(|r| (0.0..=1.0).contains(r)),
        window_label: snapshot.window.clone(),
        public_since: Some(entry.public_since),
        snapshot_at: Some(snapshot.computed_at),
        analytics_withheld: withheld,
    })
}

/// Fetch the public roster and reduce it to `display_handle`'s standing.
///
/// `Ok(None)` is the ordinary "no standing" answer and covers the server's
/// `503` as well as a missing row. A `503` here means the snapshot is
/// withheld or has not been computed yet -- a normal state of the community
/// surface, not a failure of this daemon, and emphatically not something to
/// raise to a contributor. `Err` is reserved for a transport failure, which
/// the caller answers by continuing to serve the cache it already has.
pub async fn fetch_standing(
    ingest_url: &str,
    allowed_hosts: Option<&str>,
    display_handle: &str,
    now: DateTime<Utc>,
) -> Result<Option<CommunityStanding>> {
    let url = format!("{}{LEADERBOARD_PATH}", ingest_url.trim_end_matches('/'));
    let parsed = reqwest::Url::parse(&url).with_context(|| format!("parsing {url}"))?;
    crate::config::allowlist_for(allowed_hosts).check(&parsed)?;

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .context("building community HTTP client")?;
    let response = http
        .get(parsed)
        .send()
        .await
        .with_context(|| format!("fetching the community roster from {url}"))?;

    if response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        return Ok(None);
    }
    if !response.status().is_success() {
        // Status only, never the body: an error body from a public endpoint
        // is still not this daemon's to relay.
        anyhow::bail!("community roster refused: {}", response.status());
    }
    let body: serde_json::Value = response
        .json()
        .await
        .context("parsing the community roster response")?;
    Ok(standing_from_payload(&body, display_handle, now))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    /// A realistic body: exactly what the server pre-renders into
    /// `trace_leaderboard_snapshots.contents_jsonb` and serves verbatim.
    fn payload() -> serde_json::Value {
        serde_json::json!({
            "snapshot_id": "6f1d7d5a-1d3e-4f0a-9a3f-2c0a1b2c3d4e",
            "computed_at": "2026-08-17T12:00:00Z",
            "window": "7d",
            "metric": "novelty_credit",
            "min_cell_count": 5,
            "privacy": {
                "noise_seed_hash": "v1:no_noise_yet",
                "min_cell_count": 5,
                "tenant_cohort_size": 11,
                "epsilon_charged": null,
                "sensitivity": null,
                "analytics_withheld_controls": ["community_analytics_noise_mechanism"]
            },
            "leaderboard": [
                {
                    "rank": 1,
                    "display_handle": "someone-else",
                    "score": 4210.5,
                    "accepted_count": 41,
                    "accept_rate": 0.93,
                    "public_since": "2026-06-01T00:00:00Z"
                },
                {
                    "rank": 14,
                    "display_handle": "quiet-otter",
                    "score": 1240.0,
                    "accepted_count": 12,
                    "accept_rate": 0.75,
                    "public_since": "2026-07-09T10:30:00Z"
                }
            ],
            "contributors": {},
            "analytics": null
        })
    }

    #[test]
    fn a_realistic_payload_maps_onto_the_clients_field_names() {
        let s = standing_from_payload(&payload(), "quiet-otter", at("2026-08-17T12:05:00Z"))
            .expect("the handle is on the roster");
        assert_eq!(s.rank, Some(14));
        assert_eq!(s.novelty_credit, 1240.0);
        assert_eq!(s.accepted_in_window, 12);
        assert_eq!(s.accept_rate, Some(0.75));
        assert_eq!(s.window_label, "7d");
        assert_eq!(s.public_since, Some(at("2026-07-09T10:30:00Z")));
        assert_eq!(s.snapshot_at, Some(at("2026-08-17T12:00:00Z")));
        assert!(
            s.analytics_withheld,
            "a named withholding control means withheld"
        );
    }

    #[test]
    fn the_serialized_shape_is_the_one_the_clients_parse() {
        // The GTK client's `RosterStanding` and the macOS `RosterSnapshot`
        // are written against these exact keys. This is the contract.
        let s =
            standing_from_payload(&payload(), "quiet-otter", at("2026-08-17T12:05:00Z")).unwrap();
        let v = serde_json::to_value(&s).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "accept_rate",
                "accepted_in_window",
                "analytics_withheld",
                "novelty_credit",
                "public_since",
                "rank",
                "snapshot_at",
                "window_label",
            ]
        );
    }

    #[test]
    fn a_contributor_who_is_not_on_the_roster_has_no_standing() {
        // Not an error and not an empty panel: the section does not render.
        assert!(
            standing_from_payload(&payload(), "not-listed", at("2026-08-17T12:05:00Z")).is_none()
        );
    }

    #[test]
    fn the_handle_match_is_exact() {
        // A case-insensitive match could bind this contributor's public
        // panel to a different person's row.
        assert!(
            standing_from_payload(&payload(), "Quiet-Otter", at("2026-08-17T12:05:00Z")).is_none()
        );
    }

    #[test]
    fn a_snapshot_older_than_the_withdrawal_backstop_has_no_standing() {
        // The server enforces its own 15-minute bound; this is the local
        // backstop against serving a cache from a poller that has been
        // failing.
        assert!(
            standing_from_payload(&payload(), "quiet-otter", at("2026-08-17T12:31:00Z")).is_none()
        );
        assert!(
            standing_from_payload(&payload(), "quiet-otter", at("2026-08-17T12:29:00Z")).is_some(),
            "a snapshot inside the bound is still served"
        );
    }

    #[test]
    fn a_missing_or_malformed_snapshot_has_no_standing() {
        let now = at("2026-08-17T12:05:00Z");
        assert!(standing_from_payload(&serde_json::json!({}), "quiet-otter", now).is_none());
        assert!(standing_from_payload(&serde_json::json!(null), "quiet-otter", now).is_none());
        assert!(
            standing_from_payload(&serde_json::json!({"leaderboard": []}), "quiet-otter", now)
                .is_none()
        );
    }

    #[test]
    fn an_unrecognized_metric_yields_nothing_rather_than_a_mislabelled_figure() {
        let mut p = payload();
        p["metric"] = serde_json::json!("tail_fraction");
        assert!(standing_from_payload(&p, "quiet-otter", at("2026-08-17T12:05:00Z")).is_none());
    }

    #[test]
    fn computed_analytics_are_reported_as_not_withheld() {
        let mut p = payload();
        p["privacy"]["analytics_withheld_controls"] = serde_json::json!([]);
        p["analytics"] = serde_json::json!({
            "window": "7d",
            "total_submissions": 400,
            "total_accepted": 310,
            "total_rejected": 90,
            "accept_rate": 0.775,
            "novelty_histogram": [],
            "gate_outcomes": {}
        });
        let s = standing_from_payload(&p, "quiet-otter", at("2026-08-17T12:05:00Z")).unwrap();
        assert!(!s.analytics_withheld);
    }

    #[test]
    fn a_snapshot_with_no_privacy_block_reads_as_withheld() {
        // The conservative reading of an unclear answer.
        let mut p = payload();
        p.as_object_mut().unwrap().remove("privacy");
        p["analytics"] = serde_json::json!({
            "window": "7d",
            "total_submissions": 1,
            "total_accepted": 1,
            "total_rejected": 0,
            "accept_rate": 1.0,
            "novelty_histogram": [],
            "gate_outcomes": {}
        });
        let s = standing_from_payload(&p, "quiet-otter", at("2026-08-17T12:05:00Z")).unwrap();
        assert!(s.analytics_withheld);
    }

    #[test]
    fn a_nonsensical_accept_rate_is_dropped_rather_than_rendered() {
        let mut p = payload();
        p["leaderboard"][1]["accept_rate"] = serde_json::json!(1.4);
        let s = standing_from_payload(&p, "quiet-otter", at("2026-08-17T12:05:00Z")).unwrap();
        assert_eq!(s.accept_rate, None);
        assert_eq!(s.rank, Some(14), "the rest of the row still stands");
    }

    #[test]
    fn an_unrepresentable_accepted_count_withholds_the_standing() {
        // "0 accepted" is a definite claim about someone's public standing.
        // A count the daemon cannot represent is not evidence for it, so the
        // section does not render at all -- the same answer as a missing row.
        let mut p = payload();
        p["leaderboard"][1]["accepted_count"] = serde_json::json!(-3);
        assert!(standing_from_payload(&p, "quiet-otter", at("2026-08-17T12:05:00Z")).is_none());

        p["leaderboard"][1]["accepted_count"] = serde_json::json!(i64::from(u32::MAX) + 1);
        assert!(standing_from_payload(&p, "quiet-otter", at("2026-08-17T12:05:00Z")).is_none());

        // The boundary itself is representable and still served.
        p["leaderboard"][1]["accepted_count"] = serde_json::json!(u32::MAX);
        let s = standing_from_payload(&p, "quiet-otter", at("2026-08-17T12:05:00Z")).unwrap();
        assert_eq!(s.accepted_in_window, u32::MAX);
    }

    #[test]
    fn a_non_positive_rank_is_dropped_rather_than_rendered_as_zero() {
        let mut p = payload();
        p["leaderboard"][1]["rank"] = serde_json::json!(0);
        let s = standing_from_payload(&p, "quiet-otter", at("2026-08-17T12:05:00Z")).unwrap();
        assert_eq!(s.rank, None);
    }

    #[test]
    fn freshness_is_measured_against_the_snapshot_time() {
        let s =
            standing_from_payload(&payload(), "quiet-otter", at("2026-08-17T12:05:00Z")).unwrap();
        assert!(s.is_fresh(at("2026-08-17T12:29:00Z")));
        assert!(!s.is_fresh(at("2026-08-17T12:31:00Z")));
        let undated = CommunityStanding {
            snapshot_at: None,
            ..s
        };
        assert!(
            !undated.is_fresh(at("2026-08-17T12:00:00Z")),
            "a standing that cannot be shown to be current is not served"
        );
    }

    #[tokio::test]
    async fn a_withheld_snapshot_is_a_normal_no_standing_answer() {
        // 503 means the snapshot is withheld or not yet computed. That is a
        // state of the community surface, not an error of this daemon's.
        let router = axum::Router::new().route(
            "/v1/community/leaderboard",
            axum::routing::get(|| async {
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(serde_json::json!({"error": "community snapshot is withheld"})),
                )
            }),
        );
        let base = spawn(router).await;
        let out = fetch_standing(&base, Some("127.0.0.1"), "quiet-otter", Utc::now())
            .await
            .expect("a withheld snapshot is not an error");
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn a_served_snapshot_reduces_to_this_contributors_row() {
        let router = axum::Router::new().route(
            "/v1/community/leaderboard",
            axum::routing::get(|| async { axum::Json(payload()) }),
        );
        let base = spawn(router).await;
        let out = fetch_standing(
            &base,
            Some("127.0.0.1"),
            "quiet-otter",
            at("2026-08-17T12:05:00Z"),
        )
        .await
        .unwrap()
        .expect("the handle is on the roster");
        assert_eq!(out.rank, Some(14));
    }

    async fn spawn(router: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }
}
