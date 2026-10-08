//! The published local-credit-estimate table: an unauthenticated, bounded
//! fetch, and the fixed schedule the daemon fetches it on.
//!
//! Nudge value addendum, 4.6 (OWNER DECISION E11). The table is public,
//! non-personal data -- aggregates over many decisions across tenants -- so
//! the request carries no enrollment, account, tenant, session or API-key
//! credential, exactly like [`crate::mission_catalog::MissionCatalogClient`].
//!
//! When it is fetched says nothing about this machine: the schedule
//! ([`EstimateTableSchedule`]) is a fixed interval plus a random offset, and
//! its only inputs are the clock and that offset. No queue event, preview,
//! list or status call can move it.
//!
//! What a fetch does to the table in force is [`TableFetchEffect`]:
//! an accepted table replaces it; a refused one falls back to the built-in
//! table, never to a stale fetched one; a 404 or a network failure keeps
//! whatever is in force, so a client works against a server that does not
//! publish a table at all.

use chrono::{DateTime, Duration, Utc};
use reqwest::{StatusCode, Url};
use trace_commons_operator_client::host_allowlist::HostAllowlist;
use trace_commons_protocol::local_credit_estimate::LocalEstimateTable;

use crate::config::allowlist_for;

const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The public route the table is served on (`trace-commons-ingest`).
pub const ESTIMATE_TABLE_PATH: &str = "/v1/credit-estimate/table";

/// The largest table document read. A valid table is a few hundred bytes:
/// at most `ESTIMATE_MAX_TIERS` bands, `ESTIMATE_MAX_WEIGHTS` weights and
/// short labels. A bound on the read, not a policy value; anything larger
/// is refused.
pub const ESTIMATE_TABLE_MAX_BYTES: usize = 16 * 1024;

/// How often the table is fetched.
///
/// OWNER DECISION E11: every 24 hours, with jitter.
pub const ESTIMATE_TABLE_REFRESH: Duration = Duration::hours(24);

/// The random offset added to every scheduled fetch, drawn uniformly from
/// zero up to this, so fetches from many machines spread out and none of
/// them line up with anything local.
///
/// OWNER DECISION E11 says "with jitter" and does not value it; four hours
/// is this implementation's choice and is open for the owner to set.
pub const ESTIMATE_TABLE_REFRESH_JITTER: Duration = Duration::hours(4);

/// How long an accepted table stays in force without being fetched again.
/// After it the built-in table is in force, never the stale fetched one.
///
/// OWNER DECISION E11: 7 days.
pub const ESTIMATE_TABLE_MAX_AGE: Duration = Duration::days(7);

/// Fixed, content-free outcomes of a table fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EstimateTableClientError {
    #[error("estimate-table-endpoint-invalid")]
    EndpointInvalid,
    #[error("estimate-table-host-not-allowed")]
    HostNotAllowed,
    /// The server answered, and what it sent is not a table this build
    /// accepts (`LocalEstimateTable::from_value` refused it, or it was not
    /// JSON at all).
    #[error("estimate-table-refused")]
    Refused,
    #[error("estimate-table-response-too-large")]
    ResponseTooLarge,
    /// The server publishes no table (a server without the route).
    #[error("estimate-table-not-found")]
    NotFound,
    #[error("estimate-table-unavailable")]
    Unavailable,
}

/// What one fetch outcome does to the table in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableFetchEffect {
    /// An accepted table: it becomes the table in force.
    Replace,
    /// A refused table: the built-in table comes back into force. A table
    /// the server now sends that this build cannot read says the one this
    /// build last accepted is out of date, so it is not kept.
    FallBack,
    /// Nothing was learned: no table published, the network, or this
    /// machine's own configuration. The table in force stays.
    Keep,
}

impl TableFetchEffect {
    #[must_use]
    pub fn of(outcome: &Result<LocalEstimateTable, EstimateTableClientError>) -> Self {
        match outcome {
            Ok(_) => Self::Replace,
            Err(EstimateTableClientError::Refused | EstimateTableClientError::ResponseTooLarge) => {
                Self::FallBack
            }
            Err(
                EstimateTableClientError::NotFound
                | EstimateTableClientError::Unavailable
                | EstimateTableClientError::EndpointInvalid
                | EstimateTableClientError::HostNotAllowed,
            ) => Self::Keep,
        }
    }
}

/// When the next fetch is due.
///
/// Its only inputs are the clock and a random draw in `[0, 1)`; it holds no
/// reference to the queue, the preview scheduler or any request. The daemon
/// keeps the one instance as a local of its supervisor loop, out of reach
/// of every IPC handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EstimateTableSchedule {
    next_at: DateTime<Utc>,
}

impl EstimateTableSchedule {
    /// The first fetch falls a random offset (up to
    /// [`ESTIMATE_TABLE_REFRESH_JITTER`]) after `started_at`, the daemon's
    /// start.
    #[must_use]
    pub fn starting(started_at: DateTime<Utc>, draw: f64) -> Self {
        Self {
            next_at: started_at + jitter(draw),
        }
    }

    #[must_use]
    pub fn due(&self, now: DateTime<Utc>) -> bool {
        now >= self.next_at
    }

    #[must_use]
    pub fn next_at(&self) -> DateTime<Utc> {
        self.next_at
    }

    /// Record an attempt at `now`, whatever its outcome: the next is
    /// [`ESTIMATE_TABLE_REFRESH`] plus a fresh random offset later. A failed
    /// fetch is not retried sooner, so a failure is no more visible on the
    /// wire than a success.
    pub fn attempted(&mut self, now: DateTime<Utc>, draw: f64) {
        self.next_at = now + ESTIMATE_TABLE_REFRESH + jitter(draw);
    }
}

fn jitter(draw: f64) -> Duration {
    let draw = if draw.is_finite() {
        draw.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let span = ESTIMATE_TABLE_REFRESH_JITTER.num_seconds() as f64;
    Duration::seconds((span * draw) as i64)
}

/// A uniform draw in `[0, 1)` from the operating system's randomness, by
/// way of a v4 UUID (no further dependency).
#[must_use]
pub fn random_draw() -> f64 {
    let bits = uuid::Uuid::new_v4().as_u128() as u64 >> 11;
    bits as f64 / (1_u64 << 53) as f64
}

/// An unauthenticated client for the public estimate table.
///
/// Deliberately not an `operator_client::Client`, which always attaches a
/// bearer token.
pub struct EstimateTableClient {
    http: reqwest::Client,
    origin: Url,
    allowlist: HostAllowlist,
}

impl EstimateTableClient {
    /// A client for a configured ingest origin. HTTPS is required except
    /// for literal loopback IPs; a base path is refused, as for the mission
    /// catalogue.
    pub fn new(
        ingest_url: &str,
        allowed_hosts: Option<&str>,
    ) -> Result<Self, EstimateTableClientError> {
        let origin =
            Url::parse(ingest_url).map_err(|_| EstimateTableClientError::EndpointInvalid)?;
        if !crate::mission_catalog::origin_is_valid(&origin) {
            return Err(EstimateTableClientError::EndpointInvalid);
        }
        let allowlist = allowlist_for(allowed_hosts);
        allowlist
            .check(&origin)
            .map_err(|_| EstimateTableClientError::HostNotAllowed)?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| EstimateTableClientError::Unavailable)?;
        Ok(Self {
            http,
            origin,
            allowlist,
        })
    }

    /// Fetch the published table, accepted only through
    /// [`LocalEstimateTable::from_value`].
    pub async fn fetch(&self) -> Result<LocalEstimateTable, EstimateTableClientError> {
        let mut url = self.origin.clone();
        url.set_path(ESTIMATE_TABLE_PATH);
        url.set_query(None);
        url.set_fragment(None);
        self.allowlist
            .check(&url)
            .map_err(|_| EstimateTableClientError::HostNotAllowed)?;
        let mut response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|_| EstimateTableClientError::Unavailable)?;
        match response.status() {
            StatusCode::NOT_FOUND => return Err(EstimateTableClientError::NotFound),
            status if !status.is_success() => return Err(EstimateTableClientError::Unavailable),
            _ => {}
        }
        if response
            .content_length()
            .is_some_and(|length| length > ESTIMATE_TABLE_MAX_BYTES as u64)
        {
            return Err(EstimateTableClientError::ResponseTooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| EstimateTableClientError::Unavailable)?
        {
            if body.len().saturating_add(chunk.len()) > ESTIMATE_TABLE_MAX_BYTES {
                return Err(EstimateTableClientError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&body).map_err(|_| EstimateTableClientError::Refused)?;
        LocalEstimateTable::from_value(&value).map_err(|_| EstimateTableClientError::Refused)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::{
        Json, Router, http::HeaderMap, http::StatusCode, response::IntoResponse, routing::get,
    };
    use chrono::TimeZone as _;
    use serde_json::json;

    use super::*;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap()
    }

    async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{address}"), task)
    }

    fn three_tier() -> serde_json::Value {
        json!({
            "schema_version": 1,
            "features_version": "lef1",
            "version": "t2",
            "credit_quality_calibration": "cq3",
            "bytes_per_token": 4,
            "chunk_target_tokens": 2048,
            "chunk_cap": 16,
            "weights": [{"term": "ln_content_bytes", "weight": 1.0}],
            "cut_offs": [4.0, 6.0],
            "bands": [
                {"low": 0.9, "high": 1.9},
                {"low": 1.3, "high": 2.4},
                {"low": 1.8, "high": 3.1}
            ]
        })
    }

    #[tokio::test]
    async fn the_request_is_exact_and_carries_no_credentials() {
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&hits);
        let app = Router::new().route(
            ESTIMATE_TABLE_PATH,
            get(move |headers: HeaderMap| {
                let counted = Arc::clone(&counted);
                async move {
                    counted.fetch_add(1, Ordering::SeqCst);
                    for header in ["authorization", "cookie", "x-api-key", "x-tenant-id"] {
                        assert!(!headers.contains_key(header), "{header} was sent");
                    }
                    Json(three_tier())
                }
            }),
        );
        let (base, server) = serve(app).await;
        let client = EstimateTableClient::new(&base, Some("127.0.0.1")).unwrap();
        let table = client.fetch().await.unwrap();
        assert_eq!(table.tier_count(), 3);
        assert_eq!(table.calibration_label(), "lef1.t2/cq3");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn a_table_this_build_refuses_is_refused_and_falls_back() {
        let mut newer = three_tier();
        newer["schema_version"] = json!(2);
        for body in [newer, json!({"bands": []}), json!("not a table")] {
            let app = Router::new().route(
                ESTIMATE_TABLE_PATH,
                get(move || {
                    let body = body.clone();
                    async move { Json(body) }
                }),
            );
            let (base, server) = serve(app).await;
            let client = EstimateTableClient::new(&base, Some("127.0.0.1")).unwrap();
            let outcome = client.fetch().await;
            assert_eq!(outcome, Err(EstimateTableClientError::Refused));
            assert_eq!(TableFetchEffect::of(&outcome), TableFetchEffect::FallBack);
            server.abort();
        }
    }

    #[tokio::test]
    async fn a_server_without_the_route_keeps_the_table_in_force() {
        let (base, server) = serve(Router::new()).await;
        let client = EstimateTableClient::new(&base, Some("127.0.0.1")).unwrap();
        let outcome = client.fetch().await;
        assert_eq!(outcome, Err(EstimateTableClientError::NotFound));
        assert_eq!(TableFetchEffect::of(&outcome), TableFetchEffect::Keep);
        server.abort();

        let app = Router::new().route(
            ESTIMATE_TABLE_PATH,
            get(|| async { StatusCode::SERVICE_UNAVAILABLE.into_response() }),
        );
        let (base, server) = serve(app).await;
        let client = EstimateTableClient::new(&base, Some("127.0.0.1")).unwrap();
        let outcome = client.fetch().await;
        assert_eq!(outcome, Err(EstimateTableClientError::Unavailable));
        assert_eq!(TableFetchEffect::of(&outcome), TableFetchEffect::Keep);
        server.abort();
    }

    #[tokio::test]
    async fn an_oversized_document_is_refused() {
        let app = Router::new().route(
            ESTIMATE_TABLE_PATH,
            get(|| async { "x".repeat(ESTIMATE_TABLE_MAX_BYTES + 1) }),
        );
        let (base, server) = serve(app).await;
        let client = EstimateTableClient::new(&base, Some("127.0.0.1")).unwrap();
        assert_eq!(
            client.fetch().await,
            Err(EstimateTableClientError::ResponseTooLarge)
        );
        server.abort();
    }

    #[test]
    fn plain_http_off_loopback_and_base_paths_are_refused() {
        for url in [
            "http://example.com/",
            "https://example.com/prefix/",
            "https://user@example.com/",
            "not a url",
        ] {
            assert_eq!(
                EstimateTableClient::new(url, Some("example.com")).err(),
                Some(EstimateTableClientError::EndpointInvalid),
                "{url}"
            );
        }
    }

    /// The schedule's only inputs are the clock and the draw: whatever
    /// happens locally between two attempts, the next attempt falls
    /// `ESTIMATE_TABLE_REFRESH` plus at most the jitter after the last.
    #[test]
    fn the_schedule_is_a_fixed_interval_plus_jitter_and_nothing_else() {
        let schedule = EstimateTableSchedule::starting(t0(), 0.5);
        assert_eq!(schedule.next_at(), t0() + Duration::hours(2));
        assert!(!schedule.due(t0()));
        assert!(schedule.due(t0() + Duration::hours(2)));

        for draw in [0.0, 0.25, 0.999_999, 1.0, -3.0, f64::NAN, f64::INFINITY] {
            let mut schedule = EstimateTableSchedule::starting(t0(), draw);
            let attempt = t0() + Duration::hours(3);
            schedule.attempted(attempt, draw);
            let next = schedule.next_at();
            assert!(next >= attempt + ESTIMATE_TABLE_REFRESH, "{draw}");
            assert!(
                next <= attempt + ESTIMATE_TABLE_REFRESH + ESTIMATE_TABLE_REFRESH_JITTER,
                "{draw}"
            );
            // Not due again a moment later, however often it is asked.
            for minutes in [0, 1, 60, 60 * 23] {
                assert!(!schedule.due(attempt + Duration::minutes(minutes)));
            }
        }
    }

    #[test]
    fn draws_land_in_the_unit_interval() {
        for _ in 0..1000 {
            let draw = random_draw();
            assert!((0.0..1.0).contains(&draw), "{draw}");
        }
    }

    /// Nothing that answers a request or reacts to the queue may reach the
    /// fetch or the schedule: the daemon calls them from its supervisor
    /// loop alone. Checked against the source text, because "is never
    /// called from" is not observable from a call that happens to work.
    #[test]
    fn only_the_supervisor_loop_drives_the_fetch() {
        let names = [
            "EstimateTableSchedule",
            "EstimateTableClient::",
            "refresh_estimate_table",
        ];
        for (file, source) in [
            ("daemon/ipc.rs", include_str!("daemon/ipc.rs")),
            ("daemon/watcher.rs", include_str!("daemon/watcher.rs")),
            ("daemon/queue.rs", include_str!("daemon/queue.rs")),
            ("daemon/preview.rs", include_str!("daemon/preview.rs")),
            (
                "daemon/preview_scheduler.rs",
                include_str!("daemon/preview_scheduler.rs"),
            ),
            ("daemon/nudge.rs", include_str!("daemon/nudge.rs")),
        ] {
            for name in names {
                assert!(!source.contains(name), "{file} names {name}");
            }
        }
        let supervisor = include_str!("daemon/mod.rs");
        let production = supervisor
            .split("#[cfg(test)]\nmod tests {")
            .next()
            .unwrap();
        assert_eq!(
            production.matches("refresh_estimate_table(").count(),
            2,
            "one definition and one call, from the supervisor loop"
        );
    }
}
