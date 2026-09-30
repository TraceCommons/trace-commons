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
pub mod proof_attestor;
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
///
/// `Default` exists for code that builds a row by hand (fixtures, mostly):
/// name the fields you care about and end the literal with
/// `..Default::default()`, so a field added here later does not break the
/// build of every crate that constructs one. The default row is not a
/// meaningful exchange -- epoch start, empty facade and backend -- and nothing
/// in the load path produces one.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
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
    /// Whether the proxy proved which model answered this hop: IronWire's own
    /// label for the row, read verbatim (`ironwire_proxy::proof`). It is the
    /// field the Inference tab (`inference_calls`, K8) passes through
    /// verbatim, because it is the stored verdict, not
    /// something to re-derive.
    ///
    /// - `None` means an older proxy, or a row that predates proof tracking,
    ///   or a label this client does not know. It is not `outside`.
    /// - Only [`ProofStatus::Verified`] is proof.
    /// - [`ProofStatus::GatewayOnly`] is never proof: a gateway receipt names
    ///   the relay, not the model.
    ///
    /// Parsed leniently, as IronWire parses its own column: an unrecognised or
    /// mistyped value is `None`, never an error that drops the row and never
    /// `Verified`.
    #[serde(default, deserialize_with = "lenient_proof_status")]
    pub proof: Option<ProofStatus>,
}

/// IronWire's proof label for one exchange, mirroring
/// `ironwire_ledger::proof::ProofStatus` label for label.
///
/// A local copy rather than IronWire's type, for the reason
/// [`RoutedExchange`] is local: the labels are the contract, and this crate
/// reads them off IronWire's JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProofStatus {
    /// Not a NEAR AI backend: an outside call, never checked.
    Outside,
    /// A NEAR AI exchange not checked yet, or receipt checks are off.
    Pending,
    /// No receipt to be had or checked.
    Unavailable,
    /// A valid receipt, but the gateway's. Not proof.
    GatewayOnly,
    /// A valid model receipt whose key nothing tied to a verified quote.
    Unattested,
    /// Proof: a model receipt over this hop's digests, signed by a key a
    /// verified, measurement-pinned TDX quote binds.
    Verified,
    /// A receipt that does not check out.
    Failed,
}

impl ProofStatus {
    /// Every label, in IronWire's display order.
    pub const ALL: [Self; 7] = [
        Self::Verified,
        Self::GatewayOnly,
        Self::Unattested,
        Self::Pending,
        Self::Unavailable,
        Self::Failed,
        Self::Outside,
    ];

    /// IronWire's spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Outside => "outside",
            Self::Pending => "pending",
            Self::Unavailable => "unavailable",
            Self::GatewayOnly => "gateway_only",
            Self::Unattested => "unattested",
            Self::Verified => "verified",
            Self::Failed => "failed",
        }
    }

    /// Parse IronWire's spelling. Exact match: case is not folded and
    /// whitespace is not trimmed, because a near miss of `verified` is a
    /// label IronWire did not write.
    #[must_use]
    pub fn parse(label: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == label)
    }

    /// Whether this label is proof. Only `verified` is.
    #[must_use]
    pub fn is_proof(self) -> bool {
        self == Self::Verified
    }
}

/// Read `proof` as a label if it is a string IronWire could have written, and
/// as `None` otherwise -- including a value of the wrong JSON type -- so no
/// proxy change can cost a row or promote one.
fn lenient_proof_status<'de, D>(deserializer: D) -> Result<Option<ProofStatus>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value.as_str().and_then(ProofStatus::parse))
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
            ..Default::default()
        }
    }

    #[test]
    fn a_snapshot_returns_only_rows_at_or_after_the_cutoff() {
        let ledger = FixedLedger::new(vec![row("s-1", 0), row("s-1", 60), row("s-1", 120)]);
        let seen = ledger.exchanges_since(at(60));
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].started_at, at(60));
    }

    /// A `/_ironwire/log` row as IronWire serializes it, with `proof` set to
    /// `label` verbatim -- or omitted when `label` is `None`.
    fn log_row(label: Option<&str>) -> String {
        let proof = match label {
            Some(label) => format!(r#","proof":{}"#, serde_json::to_string(label).unwrap()),
            None => String::new(),
        };
        format!(
            r#"{{"id":7,"started_at":"2026-09-28T12:00:00Z","facade":"openai","backend":"nearai","served_model":"Qwen/Qwen3.6-35B-A3B-FP8","upstream_id":"c54961ab1d594cf591e5566caa21196b","rung":"same_model","attempts":1,"cost_usd":0.01,"status":200{proof}}}"#
        )
    }

    fn parse(label: Option<&str>) -> RoutedExchange {
        serde_json::from_str(&log_row(label)).expect("a /log row parses")
    }

    #[test]
    fn each_of_ironwires_seven_labels_parses_to_its_variant() {
        for (label, expected) in [
            ("outside", ProofStatus::Outside),
            ("pending", ProofStatus::Pending),
            ("unavailable", ProofStatus::Unavailable),
            ("gateway_only", ProofStatus::GatewayOnly),
            ("unattested", ProofStatus::Unattested),
            ("verified", ProofStatus::Verified),
            ("failed", ProofStatus::Failed),
        ] {
            assert_eq!(parse(Some(label)).proof, Some(expected), "{label}");
            assert_eq!(expected.as_str(), label, "the label round-trips");
        }
    }

    /// An older proxy, or a row written before proof tracking, sends no field.
    #[test]
    fn a_row_without_the_field_has_no_proof_label() {
        let row = parse(None);
        assert_eq!(row.proof, None);
        assert_eq!(row.id, Some(7));
    }

    #[test]
    fn a_null_label_is_no_label() {
        let json = log_row(None).replace(r#""status":200"#, r#""status":200,"proof":null"#);
        let row: RoutedExchange = serde_json::from_str(&json).expect("parses");
        assert_eq!(row.proof, None);
    }

    /// A label a newer IronWire might add degrades to no label, and does not
    /// take the rest of the row with it.
    #[test]
    fn an_unknown_label_is_no_label_and_the_row_survives() {
        let row = parse(Some("quantum_verified"));
        assert_eq!(row.proof, None);
        assert_eq!(row.backend, "nearai");
        assert_eq!(
            row.upstream_id.as_deref(),
            Some("c54961ab1d594cf591e5566caa21196b")
        );
        assert_eq!(row.cost_usd, Some(0.01));
        assert_eq!(row.status, 200);
    }

    /// A field of the wrong type is also no label rather than a lost row.
    #[test]
    fn a_non_string_label_is_no_label_and_the_row_survives() {
        let json = log_row(None).replace(r#""status":200"#, r#""status":200,"proof":true"#);
        let row: RoutedExchange = serde_json::from_str(&json).expect("the row survives");
        assert_eq!(row.proof, None);
        assert_eq!(row.status, 200);
    }

    /// Exact match only. Anything that merely resembles `verified` is not
    /// proof: folding case or trimming would give a label IronWire never
    /// wrote a meaning it never had.
    #[test]
    fn only_the_exact_label_verified_is_verified() {
        for near_miss in [
            "Verified",
            "VERIFIED",
            " verified",
            "verified ",
            "verified\n",
            "verifed",
            "",
        ] {
            let proof = parse(Some(near_miss)).proof;
            assert_ne!(proof, Some(ProofStatus::Verified), "{near_miss:?}");
            assert_eq!(proof, None, "{near_miss:?} is no label at all");
        }
    }

    /// Only one label is proof, and `gateway_only` is not it.
    #[test]
    fn only_verified_is_proof() {
        let proofs: Vec<_> = ProofStatus::ALL
            .into_iter()
            .filter(|status| status.is_proof())
            .collect();
        assert_eq!(proofs, vec![ProofStatus::Verified]);
        assert!(!ProofStatus::GatewayOnly.is_proof());
    }

    #[test]
    fn an_empty_ledger_is_not_an_error() {
        // Absence and failure must be the same state everywhere downstream.
        let ledger = FixedLedger::new(Vec::new());
        assert!(ledger.exchanges_since(at(0)).is_empty());
    }
}
