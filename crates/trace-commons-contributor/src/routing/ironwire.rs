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
    #[must_use]
    pub fn new(port: u16, token: String) -> Self {
        Self {
            port,
            token,
            snapshot: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Replace the snapshot from a response body, if it parses.
    ///
    /// A body we cannot read leaves the previous snapshot in place. Enrichment
    /// that is one refresh stale is better than enrichment that vanishes
    /// because a proxy served an error page once.
    pub(crate) fn absorb(&self, body: &[u8]) {
        let Ok(view) = serde_json::from_slice::<LogView>(body) else {
            return;
        };
        if let Ok(mut snapshot) = self.snapshot.write() {
            *snapshot = view.exchanges;
        }
    }

    /// Fetch the last [`REFRESH_WINDOW_HOURS`] from the proxy.
    ///
    /// Infallible from the caller's perspective: every failure -- not
    /// installed, not running, token rejected, body unreadable -- leaves the
    /// snapshot as it was. A 401 is a configuration fact, not a transient, so
    /// nothing here retries.
    pub async fn refresh(&self) {
        let since = Utc::now() - chrono::Duration::hours(REFRESH_WINDOW_HOURS);
        // The `Z` form deliberately: a literal `+` in a query string is a
        // space, so an offset-form timestamp arrives malformed.
        let url = format!(
            "http://127.0.0.1:{}/_ironwire/log?limit=1000&since={}",
            self.port,
            since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        );
        let Ok(client) = reqwest::Client::builder().timeout(REFRESH_TIMEOUT).build() else {
            return;
        };
        let Ok(response) = client
            .get(&url)
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
        self.absorb(&body);
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

    #[test]
    fn a_ledger_that_has_never_refreshed_has_no_rows() {
        // The state on every machine without the proxy, and the state during
        // the first seconds on a machine with it. Neither is an error.
        let ledger = IronWireLedger::new(8463, "t".to_string());
        assert!(ledger.exchanges_since(chrono::Utc::now()).is_empty());
    }

    #[test]
    fn a_declared_proxy_that_returns_nothing_is_distinguishable_from_no_proxy() {
        let ledger = IronWireLedger::new(8463, "t".to_string());
        assert!(!ledger.has_rows(), "declared but empty");
    }

    #[test]
    fn a_body_we_cannot_parse_leaves_the_snapshot_untouched() {
        let ledger = IronWireLedger::new(8463, "t".to_string());
        ledger.absorb(br#"{"enabled":true,"exchanges":[{"started_at":"2026-09-01T00:00:00Z","facade":"anthropic","backend":"claude-sub","rung":"same_model","attempts":1,"status":200,"client_session_id":"s-1"}]}"#);
        assert_eq!(
            ledger.exchanges_since(chrono::DateTime::UNIX_EPOCH).len(),
            1
        );

        // A proxy release renames something, or an error page is served.
        ledger.absorb(b"<html>not json</html>");
        assert_eq!(
            ledger.exchanges_since(chrono::DateTime::UNIX_EPOCH).len(),
            1,
            "the last good snapshot survives a bad response"
        );
    }
}
