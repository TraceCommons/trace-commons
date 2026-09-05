//! Routing and cost data for the inference hops behind a session.
//!
//! A local inference proxy (IronWire) sits at the point every model call
//! passes through and records what each one cost, which backend served it and
//! how long it took. Our scrapers read session files and cannot see any of
//! that. This module reads the proxy's own local ledger and hands the rows to
//! the decorating source in `enriched`, which joins them onto the sessions
//! we already build.
//!
//! Three rules hold everywhere in here:
//!
//! - **Absence and failure are the same state.** Not installed, not running,
//!   token unreadable, JSON we cannot parse: every one resolves to no rows.
//!   The correct behaviour on failure is identical to the correct behaviour on
//!   absence, so there is no permanent-versus-transient distinction to model.
//! - **Nothing here can fail a submission.** No method returns an error to the
//!   load path.
//! - **Attribution only.** These numbers are corpus metadata. They must never
//!   reach a gate, a scoring input, or a credit computation. They come from a
//!   proxy the contributor can patch.

use chrono::{DateTime, Utc};
use serde::Deserialize;

pub mod attestation_report;
pub mod attested;
pub mod enriched;
pub mod ironwire;
pub mod receipt;

/// One inference hop, as the proxy recorded it.
///
/// Deliberately a local type deserialized from the proxy's JSON rather than
/// its own row struct: this crate takes no dependency on IronWire, which pulls
/// an Ironclaw tree this repo does not have and must not gain.
///
/// Unknown fields are ignored, so a proxy release that adds a column does not
/// break us. Missing fields that we need are `Option`, so one that goes away
/// degrades a row rather than dropping it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RoutedExchange {
    /// The proxy's rowid for this exchange, and the cursor a reader pages on.
    ///
    /// `None` on a proxy older than the release that exposes it, which costs
    /// only the ability to page: a single window still reads.
    #[serde(default)]
    pub id: Option<i64>,
    pub started_at: DateTime<Utc>,
    /// The agent session the proxy saw on the request. `None` on a proxy older
    /// than the release that records it, which simply means this row cannot be
    /// joined to anything.
    #[serde(default)]
    pub client_session_id: Option<String>,
    #[serde(default)]
    pub total_ms: Option<i64>,
    pub facade: String,
    pub backend: String,
    #[serde(default)]
    pub requested_model: Option<String>,
    #[serde(default)]
    pub served_model: Option<String>,
    /// The provider's own identifier for this exchange, when it gave one.
    ///
    /// NEAR AI's `chat_id`, and the only handle that reaches its receipt
    /// endpoint. `None` on a proxy older than the release that records it,
    /// and on every backend that returns no such identifier -- which simply
    /// means this hop can never be attested.
    ///
    /// The one exception to this module's attribution-only rule, and it is
    /// not really an exception: nothing *reads* this as a number. It is a
    /// lookup key, and what it unlocks (a receipt over the two digests below)
    /// is checked cryptographically by a party that is not this process.
    #[serde(default)]
    pub upstream_id: Option<String>,
    /// SHA-256 of the request body exactly as it went upstream, hex.
    ///
    /// After a model override, after the privacy filter, after translation --
    /// because those are the bytes the inference enclave hashed. `None` when
    /// body capture is off, or when the body could not be held whole.
    #[serde(default)]
    pub request_sha256: Option<String>,
    /// SHA-256 of the response body exactly as it came back, hex.
    ///
    /// For a streamed response, the digest of the raw concatenated event
    /// stream. `None` unless the stream was read to its end: a response that
    /// was cancelled, restarted or truncated has no honest digest, and that
    /// absence is how [`attested`] recognises an unattestable call.
    #[serde(default)]
    pub response_sha256: Option<String>,
    /// Where the proxy put this exchange's verbatim bodies, under
    /// `$IRONWIRE_HOME/bodies`. `None` when nothing was captured.
    #[serde(default)]
    pub body_ref: Option<String>,
    pub rung: String,
    pub attempts: i64,
    #[serde(default)]
    pub input_tokens: Option<i64>,
    #[serde(default)]
    pub cache_read_tokens: Option<i64>,
    #[serde(default)]
    pub cache_write_tokens: Option<i64>,
    #[serde(default)]
    pub output_tokens: Option<i64>,
    /// What the hop cost, priced by the proxy from observed tokens.
    ///
    /// Priced, not billed: work served on a subscription is priced at what it
    /// *would* have cost on the meter. No surface may render it as money the
    /// contributor spent.
    #[serde(default)]
    pub cost_usd: Option<f64>,
    pub status: i64,
}

/// A source of routing rows.
///
/// Synchronous by design. `TraceSource::load` is synchronous, so anything the
/// join needs must already be in memory by the time it runs -- see
/// [`ironwire::IronWireLedger::refresh`]. Returns rows rather than a `Result`:
/// there is no failure a caller could act on differently from absence.
pub trait RoutingLedger: Send + Sync {
    /// Rows at or after `from`, oldest first.
    fn exchanges_since(&self, from: DateTime<Utc>) -> Vec<RoutedExchange>;
}

/// A ledger over a fixed set of rows. Used by tests and by the `Off` state.
#[derive(Debug, Default)]
pub struct FixedLedger {
    rows: Vec<RoutedExchange>,
}

impl FixedLedger {
    #[must_use]
    pub fn new(rows: Vec<RoutedExchange>) -> Self {
        Self { rows }
    }
}

impl RoutingLedger for FixedLedger {
    fn exchanges_since(&self, from: DateTime<Utc>) -> Vec<RoutedExchange> {
        let mut rows: Vec<RoutedExchange> = self
            .rows
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
    use chrono::TimeZone;

    fn at(offset: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + offset, 0).unwrap()
    }

    fn row(session: &str, offset: i64) -> RoutedExchange {
        RoutedExchange {
            id: None,
            started_at: at(offset),
            client_session_id: Some(session.to_string()),
            total_ms: Some(1200),
            facade: "anthropic".to_string(),
            backend: "claude-sub".to_string(),
            requested_model: Some("claude-opus-4-6".to_string()),
            served_model: Some("claude-opus-4-6".to_string()),
            upstream_id: None,
            request_sha256: None,
            response_sha256: None,
            body_ref: None,
            rung: "same_model".to_string(),
            attempts: 1,
            input_tokens: Some(1000),
            cache_read_tokens: Some(500),
            cache_write_tokens: None,
            output_tokens: Some(200),
            cost_usd: Some(0.02),
            status: 200,
        }
    }

    #[test]
    fn a_snapshot_returns_only_rows_at_or_after_the_cutoff() {
        let ledger = FixedLedger::new(vec![row("s-1", 0), row("s-1", 60), row("s-1", 120)]);
        let seen = ledger.exchanges_since(at(60));
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].started_at, at(60));
    }

    #[test]
    fn an_empty_ledger_is_not_an_error() {
        // Absence and failure must be the same state everywhere downstream.
        let ledger = FixedLedger::new(Vec::new());
        assert!(ledger.exchanges_since(at(0)).is_empty());
    }
}
