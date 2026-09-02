//! The IronWire control-API client.
//!
//! Reads `GET /_ironwire/log` on loopback, authenticated with the token the
//! proxy writes to `$IRONWIRE_HOME/control.token` (mode 0600). The token is
//! read at call time and never copied into our settings or logged.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{RoutedExchange, RoutingLedger};

/// How long a refresh may take before we give up and keep the old snapshot.
///
/// Short because this is a loopback call to a process on the same machine. A
/// proxy slow enough to miss this is a proxy whose numbers we would rather do
/// without than wait for.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(3);

/// How far back a refresh asks for. Generous relative to the daemon's cadence
/// so a missed tick overlaps rather than gaps.
const REFRESH_WINDOW_HOURS: i64 = 24;

/// Rows requested per page.
const PAGE_LIMIT: usize = 1000;

/// Pages a single refresh will walk before giving up.
///
/// A bound on a loop driven by another process's answers. At [`PAGE_LIMIT`]
/// rows a page this covers far more exchanges than a day of heavy use, and a
/// proxy that keeps returning full pages past it is one we should stop asking
/// rather than follow indefinitely.
const MAX_REFRESH_PAGES: usize = 50;

/// The proxy's log response. Only the fields we use.
#[derive(Debug, Deserialize)]
struct LogView {
    #[serde(default)]
    exchanges: Vec<RoutedExchange>,
}

/// A [`RoutingLedger`] backed by a local IronWire daemon.
///
/// Holds a snapshot refreshed out of band. `exchanges_since` never does I/O,
/// because it is called from `TraceSource::load`, which is synchronous and on
/// the path to a submission.
pub struct IronWireLedger {
    port: u16,
    token: String,
    snapshot: Arc<RwLock<Vec<RoutedExchange>>>,
    /// Built once at construction, not per `refresh()` call. A fresh
    /// `reqwest::Client` builds its own TLS config and connection pool, which
    /// is wasted work on every poll tick for a client that only ever talks to
    /// one loopback host with `Connection: keep-alive` semantics anyway.
    ///
    /// `Option` because building a client can fail -- this crate pins
    /// `rustls-tls-native-roots`, which loads the platform trust store during
    /// `build()`, and an unreadable or malformed store makes that fail on a
    /// real machine. A client we could not build means no enrichment, exactly
    /// the same state as a proxy that was never installed: `routing/mod.rs`
    /// treats absence and failure as the same state, and panicking here would
    /// take down the whole daemon over a condition this module otherwise
    /// shrugs off.
    client: Option<reqwest::Client>,
}

impl std::fmt::Debug for IronWireLedger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token is a credential for an API that can rewrite the user's
        // agent configs. It does not appear in a debug line.
        f.debug_struct("IronWireLedger")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl IronWireLedger {
    /// The token this ledger was built with.
    ///
    /// Test-only, and deliberately so: `Debug` omits the token because it is
    /// a credential for an API that can rewrite the contributor's agent
    /// configuration, and weakening `Debug` to make it assertable would put
    /// it in every log line that ever formats a ledger. A `#[cfg(test)]`
    /// accessor is visible to the crate's own tests and to nothing shipped.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn token_for_test(&self) -> &str {
        &self.token
    }

    #[must_use]
    pub fn new(port: u16, token: String) -> Self {
        Self {
            port,
            token,
            snapshot: Arc::new(RwLock::new(Vec::new())),
            client: reqwest::Client::builder().build().ok(),
        }
    }

    /// Fetch the last [`REFRESH_WINDOW_HOURS`] from the proxy.
    ///
    /// Infallible from the caller's perspective: every failure -- not
    /// installed, not running, token rejected, body unreadable -- leaves the
    /// snapshot as it was. A 401 is a configuration fact, not a transient, so
    /// nothing here retries.
    pub async fn refresh(&self) {
        let Some(client) = self.client.as_ref() else {
            tracing::debug!("routing ledger has no client, skipping refresh");
            return;
        };
        let since = Utc::now() - chrono::Duration::hours(REFRESH_WINDOW_HOURS);
        // The `Z` form deliberately: a literal `+` in a query string is a
        // space, so an offset-form timestamp arrives malformed.
        let since = since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        let mut rows: Vec<RoutedExchange> = Vec::new();
        let mut cursor: Option<i64> = None;

        for _ in 0..MAX_REFRESH_PAGES {
            let url = match cursor {
                Some(after) => format!(
                    "http://127.0.0.1:{}/_ironwire/log?limit={PAGE_LIMIT}&since={since}&after_id={after}",
                    self.port
                ),
                None => format!(
                    "http://127.0.0.1:{}/_ironwire/log?limit={PAGE_LIMIT}&since={since}",
                    self.port
                ),
            };
            let Ok(response) = client
                .get(&url)
                .timeout(REFRESH_TIMEOUT)
                .header("authorization", format!("Bearer {}", self.token))
                .send()
                .await
            else {
                tracing::debug!("routing ledger unreachable");
                return;
            };
            if !response.status().is_success() {
                tracing::debug!(
                    status = response.status().as_u16(),
                    "routing ledger refused"
                );
                return;
            }
            let Ok(body) = response.bytes().await else {
                return;
            };
            let Some(page) = Self::parse_page(&body) else {
                return;
            };

            let short = page.len() < PAGE_LIMIT;
            let next = page.last().and_then(|row| row.id);
            rows.extend(page);

            if short {
                break;
            }
            // A full page whose last row carries no id, or an id that has not
            // moved, means this proxy cannot page -- older than the release
            // that exposes one, or answering something we did not ask. Keep
            // what the first pages gave rather than asking the same question
            // until the cap.
            match next {
                Some(id) if Some(id) != cursor => cursor = Some(id),
                _ => break,
            }
        }

        if let Ok(mut snapshot) = self.snapshot.write() {
            *snapshot = rows;
        }
    }

    /// One page of rows, or `None` when the body is not readable JSON.
    ///
    /// Only a body that is not readable JSON at all aborts a refresh -- an
    /// error page, truncated output, a byte stream that is not JSON. Enrichment
    /// one refresh stale beats enrichment that vanishes because a proxy served
    /// an error page once.
    ///
    /// A body that DOES parse is taken at its word, even down to empty,
    /// including one carrying no `exchanges` key at all (`#[serde(default)]`
    /// makes that an empty `Vec`, not a parse failure) -- a proxy answering
    /// `{"enabled":false}`, say. That is intentional: a refresh asks for the
    /// current [`REFRESH_WINDOW_HOURS`] window and the snapshot is meant to
    /// hold what that window has, not accumulate rows the proxy no longer
    /// reports. If the last good window had rows and this one honestly does
    /// not, empty is the right answer.
    fn parse_page(body: &[u8]) -> Option<Vec<RoutedExchange>> {
        serde_json::from_slice::<LogView>(body)
            .ok()
            .map(|view| view.exchanges)
    }

    /// Whether the last refresh produced any rows.
    ///
    /// Distinguishes "declared, reading nothing" from "not declared" in the
    /// daemon's health output. Not an error state -- a machine whose proxy was
    /// installed today legitimately reports this -- but a user who declared a
    /// proxy and sees no enrichment needs somewhere to look.
    #[must_use]
    pub fn has_rows(&self) -> bool {
        self.snapshot.read().is_ok_and(|rows| !rows.is_empty())
    }
}

impl RoutingLedger for IronWireLedger {
    fn exchanges_since(&self, from: DateTime<Utc>) -> Vec<RoutedExchange> {
        let Ok(snapshot) = self.snapshot.read() else {
            return Vec::new();
        };
        let mut rows: Vec<RoutedExchange> = snapshot
            .iter()
            .filter(|row| row.started_at >= from)
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.started_at);
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ledger whose client could not be built (the `Client::builder().build()`
    /// call failed, e.g. a broken platform trust store) must not panic on
    /// `refresh()` -- it must behave exactly like any other unreachable
    /// proxy and leave the snapshot as it was. Constructs the struct directly
    /// with `client: None` rather than forcing a real build failure, which
    /// is not reproducible on a healthy test machine.
    #[tokio::test]
    async fn refresh_with_no_client_does_not_panic() {
        let ledger = IronWireLedger {
            port: 8463,
            token: "t".to_string(),
            snapshot: Arc::new(RwLock::new(Vec::new())),
            client: None,
        };
        ledger.refresh().await;
        assert!(
            ledger
                .exchanges_since(chrono::DateTime::UNIX_EPOCH)
                .is_empty()
        );
    }

    #[test]
    fn a_ledger_that_has_never_refreshed_has_no_rows() {
        // The state on every machine without the proxy, and the state during
        // the first seconds on a machine with it. Neither is an error.
        let ledger = IronWireLedger::new(8463, "t".to_string());
        assert!(ledger.exchanges_since(chrono::Utc::now()).is_empty());
    }

    /// `has_rows()` reports false on a freshly declared ledger that has not
    /// refreshed yet, which is what the daemon layer needs from it: the
    /// real "declared, reading nothing" vs. "not declared at all" distinction
    /// is `Option<Arc<IronWireLedger>>` -- `Some` with `has_rows() == false`
    /// vs. `None` -- and that Option lives on the daemon's shared state, not
    /// on `IronWireLedger` itself. See `daemon::ipc::tests::routing_transition`
    /// (searches `s.routing_transition`) for the test that actually exercises
    /// both sides of that distinction. This test only pins the half that
    /// lives in this module: an unrefreshed, declared ledger has no rows.
    #[test]
    fn has_rows_is_false_before_any_refresh() {
        let ledger = IronWireLedger::new(8463, "t".to_string());
        assert!(!ledger.has_rows(), "declared but empty");
    }

    const ONE_ROW: &[u8] = br#"{"enabled":true,"exchanges":[{"id":1,"started_at":"2026-09-01T00:00:00Z","facade":"anthropic","backend":"claude-sub","rung":"same_model","attempts":1,"status":200,"client_session_id":"s-1"}]}"#;

    #[test]
    fn a_body_we_cannot_parse_aborts_the_refresh() {
        // An error page, truncated output, a byte stream that is not JSON.
        // `refresh` returns without touching the snapshot on this, which is
        // what keeps enrichment one refresh stale instead of gone.
        assert!(IronWireLedger::parse_page(b"<html>not json</html>").is_none());
    }

    #[test]
    fn a_readable_body_is_taken_at_its_word() {
        let page = IronWireLedger::parse_page(ONE_ROW).expect("readable");
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].id, Some(1), "the cursor comes back with the row");
    }

    /// Valid JSON carrying no `exchanges` key -- a proxy answering
    /// `{"enabled":false}` -- is a readable answer of "nothing", not a parse
    /// failure. It yields an empty page, which `refresh` then commits, because
    /// the snapshot holds what the window currently has rather than
    /// accumulating rows the proxy no longer reports.
    #[test]
    fn a_valid_body_with_no_exchanges_key_is_an_empty_page_not_a_failure() {
        let page =
            IronWireLedger::parse_page(br#"{"enabled":false}"#).expect("valid JSON is readable");
        assert!(page.is_empty());
    }

    #[test]
    fn a_row_without_an_id_still_reads() {
        // A proxy older than the release that exposes a cursor. It costs
        // paging, not the window.
        let page = IronWireLedger::parse_page(
            br#"{"exchanges":[{"started_at":"2026-09-01T00:00:00Z","facade":"anthropic","backend":"claude-sub","rung":"same_model","attempts":1,"status":200}]}"#,
        )
        .expect("readable");
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].id, None);
    }

    /// The whole reason this client pages.
    ///
    /// The proxy caps a page at `limit`. A reader that takes one page per tick
    /// sees only the oldest rows in its window and never a recent exchange on
    /// a busy machine -- and it degrades quietly, because some rows still come
    /// back. This walks the window instead, advancing on `after_id` until a
    /// page comes back short.
    #[tokio::test]
    async fn refresh_walks_the_whole_window_not_just_its_first_page() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        // Two full pages then a short one. PAGE_LIMIT rows is more than a test
        // wants to serialize, so the mock reports a full page by repeating a
        // row PAGE_LIMIT times and keys off `after_id` for what to send next.
        let calls = Arc::new(AtomicUsize::new(0));
        let seen_cursors = Arc::new(RwLock::new(Vec::<Option<String>>::new()));
        let calls_for_route = Arc::clone(&calls);
        let cursors_for_route = Arc::clone(&seen_cursors);

        let router = axum::Router::new().route(
            "/_ironwire/log",
            axum::routing::get(move |req: axum::extract::Request| {
                let calls = Arc::clone(&calls_for_route);
                let cursors = Arc::clone(&cursors_for_route);
                async move {
                    let query = req.uri().query().unwrap_or("").to_string();
                    let after = query
                        .split('&')
                        .find_map(|p| p.strip_prefix("after_id=").map(str::to_string));
                    if let Ok(mut c) = cursors.write() {
                        c.push(after.clone());
                    }
                    let n = calls.fetch_add(1, Ordering::SeqCst);
                    // Pages 0 and 1 are full; page 2 is short and ends it.
                    let count = if n < 2 { PAGE_LIMIT } else { 3 };
                    let base = n * PAGE_LIMIT;
                    let rows: Vec<String> = (0..count)
                        .map(|i| {
                            format!(
                                r#"{{"id":{},"started_at":"2026-09-01T00:00:00Z","facade":"anthropic","backend":"claude-sub","rung":"same_model","attempts":1,"status":200,"client_session_id":"s-1"}}"#,
                                base + i + 1
                            )
                        })
                        .collect();
                    axum::response::Response::builder()
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(format!(
                            r#"{{"enabled":true,"exchanges":[{}]}}"#,
                            rows.join(",")
                        )))
                        .expect("response builds")
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        let ledger = IronWireLedger::new(port, "t".to_string());
        ledger.refresh().await;

        assert_eq!(
            ledger.exchanges_since(chrono::DateTime::UNIX_EPOCH).len(),
            PAGE_LIMIT * 2 + 3,
            "every page is kept, not just the first"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 3, "stopped on the short page");

        let cursors = seen_cursors.read().expect("cursors");
        assert_eq!(cursors[0], None, "the first request carries no cursor");
        assert_eq!(
            cursors[1].as_deref(),
            Some(PAGE_LIMIT.to_string().as_str()),
            "the second advances past the first page's last id"
        );
    }

    #[tokio::test]
    async fn refresh_stops_when_a_proxy_cannot_advance_the_cursor() {
        // A proxy that keeps answering full pages whose last row has no id --
        // older than the release that exposes one. Take what the first page
        // gave rather than asking the same question until the cap.
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_for_route = Arc::clone(&calls);

        let router = axum::Router::new().route(
            "/_ironwire/log",
            axum::routing::get(move || {
                let calls = Arc::clone(&calls_for_route);
                async move {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let rows: Vec<String> = (0..PAGE_LIMIT)
                        .map(|_| {
                            r#"{"started_at":"2026-09-01T00:00:00Z","facade":"anthropic","backend":"claude-sub","rung":"same_model","attempts":1,"status":200}"#
                                .to_string()
                        })
                        .collect();
                    axum::response::Response::builder()
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(format!(
                            r#"{{"exchanges":[{}]}}"#,
                            rows.join(",")
                        )))
                        .expect("response builds")
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        let ledger = IronWireLedger::new(port, "t".to_string());
        ledger.refresh().await;

        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "one request, not MAX_REFRESH_PAGES of them"
        );
        assert_eq!(
            ledger.exchanges_since(chrono::DateTime::UNIX_EPOCH).len(),
            PAGE_LIMIT,
            "the page it did get is kept"
        );
    }
}
