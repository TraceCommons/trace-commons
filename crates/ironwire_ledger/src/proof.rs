//! Whether an exchange carries a provider proof, and the rollup over it.
//!
//! "Proof" here means one specific thing: a NEAR AI receipt for this
//! exchange's upstream id, signed by the serving model's own `provider_tee`
//! ed25519 key, over exactly the request and response digests this row
//! recorded, where that key is one a verified TDX quote binds. Anything less is
//! not proof and is labelled as what it is instead:
//!
//! - a **gateway** receipt proves which gateway relayed the call, not which
//!   model answered it -- [`ProofStatus::GatewayOnly`];
//! - a `provider_tee` receipt whose signature and digests check out but whose
//!   key could not be tied to a verified quote -- [`ProofStatus::Unattested`];
//! - no receipt at all, which is the permanent answer for a model NEAR AI
//!   brokers rather than hosts -- [`ProofStatus::Unavailable`].
//!
//! The status is written in two steps. The exchange is recorded as
//! [`ProofStatus::Pending`] (a NEAR AI backend) or [`ProofStatus::Outside`]
//! (anything else) on the response path, which costs nothing; the check itself
//! runs later, off the response path, and moves a `pending` row to exactly one
//! settled label. Nothing ever moves a row *out* of a settled label, so a
//! verdict cannot be overwritten by a later, weaker one.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What is known about an exchange's provider proof.
///
/// Serialised as the lowercase label (`"verified"`, `"gateway_only"`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum ProofStatus {
    /// Not a NEAR AI backend: a call that went outside, to a provider that
    /// issues no receipts. Never checked, and never will be.
    Outside,
    /// A NEAR AI exchange that has not been checked yet -- or cannot be,
    /// because receipt verification is off. Never shown as proof.
    Pending,
    /// No receipt can be obtained or checked for this exchange: the endpoint
    /// holds none (a brokered model), the row has no upstream id or no body
    /// digests to check one against, the provider rewrote the body, or the
    /// fetch kept failing until the retry budget ran out.
    Unavailable,
    /// A receipt exists and checks out, but it is the gateway's. It says which
    /// gateway relayed the call, not which model answered, so it is not proof.
    GatewayOnly,
    /// A `provider_tee` receipt whose signature and digests check out, but
    /// whose signing key could not be tied to a verified TDX quote. Not proof.
    Unattested,
    /// Proof: a `provider_tee` receipt over this row's digests, signed by a
    /// key a verified quote binds for the model.
    Verified,
    /// A receipt was served and does not check out: a bad signature, digests
    /// over other bytes, a different model, a signer outside the attested set,
    /// or a document that is not a receipt at all.
    Failed,
}

impl ProofStatus {
    /// Every status, in display order.
    pub const ALL: [Self; 7] = [
        Self::Verified,
        Self::GatewayOnly,
        Self::Unattested,
        Self::Pending,
        Self::Unavailable,
        Self::Failed,
        Self::Outside,
    ];

    /// The label stored in the ledger and sent on the wire.
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

    /// Parse a stored label. `None` for anything else -- which is what a newer
    /// IronWire writing into the same file would leave, and reading it as an
    /// absence is the downgrade-safe answer.
    #[must_use]
    pub fn parse(label: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == label)
    }

    /// Whether this is the one label that counts as proof.
    #[must_use]
    pub fn is_proof(self) -> bool {
        self == Self::Verified
    }

    /// Whether a later check may still change this label.
    #[must_use]
    pub fn is_settled(self) -> bool {
        self != Self::Pending
    }
}

/// Deserialise an optional status, reading an unrecognised label as absent.
///
/// A reader older than the writer must not fail a whole `/log` page over one
/// label it has not heard of.
pub(crate) fn lenient<'de, D>(deserializer: D) -> Result<Option<ProofStatus>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let label: Option<String> = Option::deserialize(deserializer)?;
    Ok(label.as_deref().and_then(ProofStatus::parse))
}

/// What the background check needs from a `pending` row, and nothing more.
///
/// No bodies: a receipt is checked against the digests, which the row keeps
/// even after its bodies have been released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofCandidate {
    /// Row id.
    pub id: i64,
    /// Backend that served it.
    pub backend: String,
    /// Model the provider said served it.
    pub served_model: Option<String>,
    /// Model the client asked for.
    pub requested_model: Option<String>,
    /// The provider's id for the response, which the receipt is keyed by.
    pub upstream_id: Option<String>,
    /// Digest of the request body as sent.
    pub request_sha256: Option<String>,
    /// Digest of the response body as received.
    pub response_sha256: Option<String>,
    /// Set when the provider substituted a model and rewrote the body.
    pub model_alias_resolved: Option<String>,
    /// How many times the check has been deferred already.
    pub attempts: u32,
}

/// Which side of the routed/outside line an exchange fell on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// Served by a NEAR AI backend, where a receipt can exist.
    Routed,
    /// Served by anything else.
    Outside,
    /// No status to place it by: recorded before this ledger tracked proof
    /// status, or a refusal no backend answered.
    Unknown,
}

/// Exchange counts by proof status, one field per label.
///
/// Fields rather than a map so that a consumer reading a label this version
/// never wrote sees a zero, not a missing key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProofCounts {
    /// See [`ProofStatus::Verified`].
    pub verified: i64,
    /// See [`ProofStatus::GatewayOnly`].
    pub gateway_only: i64,
    /// See [`ProofStatus::Unattested`].
    pub unattested: i64,
    /// See [`ProofStatus::Pending`].
    pub pending: i64,
    /// See [`ProofStatus::Unavailable`].
    pub unavailable: i64,
    /// See [`ProofStatus::Failed`].
    pub failed: i64,
    /// See [`ProofStatus::Outside`].
    pub outside: i64,
    /// Rows with no status at all: written before the column existed, a
    /// refusal no backend answered, or a label from a newer IronWire that this
    /// one does not know.
    pub unrecorded: i64,
}

impl ProofCounts {
    fn add(&mut self, status: Option<ProofStatus>, n: i64) {
        let slot = match status {
            Some(ProofStatus::Verified) => &mut self.verified,
            Some(ProofStatus::GatewayOnly) => &mut self.gateway_only,
            Some(ProofStatus::Unattested) => &mut self.unattested,
            Some(ProofStatus::Pending) => &mut self.pending,
            Some(ProofStatus::Unavailable) => &mut self.unavailable,
            Some(ProofStatus::Failed) => &mut self.failed,
            Some(ProofStatus::Outside) => &mut self.outside,
            None => &mut self.unrecorded,
        };
        *slot += n;
    }
}

/// One line of the rollup: every exchange in the window for one model on one
/// backend, on one side of the routed/outside line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProofRollup {
    /// The model that served, or the one requested when the provider named
    /// none (an error before any response, typically). `None` when neither is
    /// known.
    pub model: Option<String>,
    /// Backend id.
    pub backend: String,
    /// Routed through NEAR AI, outside, or unknown.
    pub route: Route,
    /// What kind of work these calls were. **Always `null` today**: nothing in
    /// IronWire classifies work, and a guessed label would be an invented
    /// number by another name. Present so a consumer can be written against
    /// the field before a real source exists.
    pub work_kind: Option<String>,
    /// Exchanges.
    pub calls: i64,
    /// Exchanges that carried a price. `cost_usd` sums only these, so a
    /// reader can tell "cost $0.40" from "cost $0.40 plus some we could not
    /// price".
    pub priced_calls: i64,
    /// Summed USD over the priced exchanges.
    pub cost_usd: f64,
    /// Exchanges by proof status.
    pub proof: ProofCounts,
}

/// Calls and cost on one side of the line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RouteTotal {
    /// Exchanges.
    pub calls: i64,
    /// Exchanges that carried a price.
    pub priced_calls: i64,
    /// Summed USD over the priced exchanges.
    pub cost_usd: f64,
    /// Exchanges by proof status.
    pub proof: ProofCounts,
}

/// The whole rollup for a window.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProofSummary {
    /// Per model, backend and route, most calls first.
    pub groups: Vec<ProofRollup>,
    /// Totals for exchanges routed through NEAR AI.
    pub routed: RouteTotal,
    /// Totals for exchanges that went outside.
    pub outside: RouteTotal,
    /// Totals for exchanges recorded before proof status existed.
    pub unknown: RouteTotal,
}

/// Which side of the line a stored status puts a row on.
pub(crate) fn route_of(status: Option<ProofStatus>) -> Route {
    match status {
        Some(ProofStatus::Outside) => Route::Outside,
        Some(_) => Route::Routed,
        None => Route::Unknown,
    }
}

/// One `GROUP BY` row, before it is folded into [`ProofSummary`].
pub(crate) struct GroupRow {
    pub model: Option<String>,
    pub backend: String,
    pub status: Option<ProofStatus>,
    pub calls: i64,
    pub priced_calls: i64,
    pub cost_usd: f64,
}

/// Fold grouped rows into the summary. Separate from the SQL so the folding is
/// testable without a database.
pub(crate) fn fold(rows: Vec<GroupRow>) -> ProofSummary {
    let mut groups: BTreeMap<(Option<String>, String, Route), ProofRollup> = BTreeMap::new();
    let mut summary = ProofSummary::default();
    for row in rows {
        let route = route_of(row.status);
        let group = groups
            .entry((row.model.clone(), row.backend.clone(), route))
            .or_insert_with(|| ProofRollup {
                model: row.model,
                backend: row.backend,
                route,
                work_kind: None,
                calls: 0,
                priced_calls: 0,
                cost_usd: 0.0,
                proof: ProofCounts::default(),
            });
        group.calls += row.calls;
        group.priced_calls += row.priced_calls;
        group.cost_usd += row.cost_usd;
        group.proof.add(row.status, row.calls);

        let total = match route {
            Route::Routed => &mut summary.routed,
            Route::Outside => &mut summary.outside,
            Route::Unknown => &mut summary.unknown,
        };
        total.calls += row.calls;
        total.priced_calls += row.priced_calls;
        total.cost_usd += row.cost_usd;
        total.proof.add(row.status, row.calls);
    }
    let mut groups: Vec<ProofRollup> = groups.into_values().collect();
    // Most calls first; ties broken by the key so the order is stable across
    // polls and a UI does not reshuffle rows that did not change.
    groups.sort_by(|a, b| {
        b.calls
            .cmp(&a.calls)
            .then_with(|| a.backend.cmp(&b.backend))
            .then_with(|| a.model.cmp(&b.model))
            .then_with(|| a.route.cmp(&b.route))
    });
    summary.groups = groups;
    summary
}

/// The instant a rollup window starts, for callers that want a default.
#[must_use]
pub fn default_window_start(now: DateTime<Utc>) -> DateTime<Utc> {
    now - chrono::Duration::hours(24)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_label_round_trips_and_nothing_else_parses() {
        for status in ProofStatus::ALL {
            assert_eq!(ProofStatus::parse(status.as_str()), Some(status));
            let json = serde_json::to_string(&status).expect("serialises");
            assert_eq!(json, format!("\"{}\"", status.as_str()));
        }
        assert_eq!(ProofStatus::parse("VERIFIED"), None);
        assert_eq!(ProofStatus::parse("proof"), None);
        assert_eq!(ProofStatus::parse(""), None);
    }

    /// The chip is binary for the user, so exactly one label may light it.
    #[test]
    fn only_verified_counts_as_proof() {
        let proofs: Vec<_> = ProofStatus::ALL
            .into_iter()
            .filter(|s| s.is_proof())
            .collect();
        assert_eq!(proofs, vec![ProofStatus::Verified]);
    }

    #[test]
    fn only_pending_is_unsettled() {
        let open: Vec<_> = ProofStatus::ALL
            .into_iter()
            .filter(|s| !s.is_settled())
            .collect();
        assert_eq!(open, vec![ProofStatus::Pending]);
    }

    #[test]
    fn a_gateway_receipt_is_routed_but_not_proof() {
        assert_eq!(route_of(Some(ProofStatus::GatewayOnly)), Route::Routed);
        assert!(!ProofStatus::GatewayOnly.is_proof());
    }

    #[test]
    fn folding_splits_routed_from_outside_and_counts_each_label() {
        let row = |model: &str, backend: &str, status, calls, priced, cost| GroupRow {
            model: Some(model.into()),
            backend: backend.into(),
            status,
            calls,
            priced_calls: priced,
            cost_usd: cost,
        };
        let summary = fold(vec![
            row("qwen", "nearai", Some(ProofStatus::Verified), 3, 3, 0.3),
            row("qwen", "nearai", Some(ProofStatus::Pending), 1, 0, 0.0),
            row("opus", "claude-sub", Some(ProofStatus::Outside), 2, 2, 1.0),
            row("opus", "claude-sub", None, 5, 5, 2.0),
        ]);
        assert_eq!(summary.routed.calls, 4);
        assert_eq!(summary.routed.priced_calls, 3);
        assert_eq!(summary.routed.proof.verified, 3);
        assert_eq!(summary.routed.proof.pending, 1);
        assert_eq!(summary.outside.calls, 2);
        assert_eq!(summary.outside.proof.outside, 2);
        assert_eq!(summary.unknown.calls, 5);
        assert_eq!(summary.unknown.proof.unrecorded, 5);

        // Legacy rows are a group of their own, not merged into "outside".
        assert_eq!(summary.groups.len(), 3);
        assert_eq!(summary.groups[0].route, Route::Unknown);
        let near = summary
            .groups
            .iter()
            .find(|g| g.backend == "nearai")
            .expect("a nearai group");
        assert_eq!(near.calls, 4);
        assert_eq!(near.work_kind, None);
        assert!((near.cost_usd - 0.3).abs() < 1e-9);
    }
}
