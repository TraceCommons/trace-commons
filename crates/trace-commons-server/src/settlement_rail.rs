// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The seam a settlement rail plugs into.
//!
//! A rail publishes one recipient's **cumulative** total of one instrument to
//! an external system. The total only ever grows, and the external side keeps
//! the maximum it has seen, so publishing is idempotent and a retried, late or
//! duplicated publish is harmless: no compare-and-swap, no idempotency key. The
//! dispatcher holds a rail as `Arc<dyn SettlementRail>` and knows nothing
//! about any particular external system. How much a recipient earns is not
//! decided here: accrual asks a
//! [`trace_commons_gate_api::SettlementAwardPolicy`]. See
//! `docs/superpowers/specs/2026-10-14-near-ai-inference-credit-settlement-design.md`.
//!
//! # Hash-only
//!
//! Every error a rail returns is a label from a fixed vocabulary
//! (`^[a-z0-9_]{1,64}$`). No error carries a response body, an account
//! reference, a recipient id or a tenant id, so no `Display`, `Debug` or
//! `?err` of one can leak them.

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::fmt;

/// A recipient reference as the rail's external system names it. Its
/// `Debug` is redacted: the value is an external account identifier and must
/// never reach a log line.
#[derive(Clone, PartialEq, Eq)]
pub struct RecipientRef(String);

impl RecipientRef {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value, for the one place that sends it to the external system.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RecipientRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecipientRef(<redacted>)")
    }
}

/// `sha256:<hex>` over a domain-separated string.
pub fn sha256_ref(domain: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    for part in parts {
        hasher.update(b"\n");
        hasher.update(part.as_bytes());
    }
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// The reference published beside a cumulative total, so the external
/// system's audit row can be matched to our ledger. Built from the rail, a
/// hash of the recipient and the total -- never from a tenant id.
pub fn statement_ref(rail_id: &str, recipient_ref_hash: &str, cumulative_units: u128) -> String {
    sha256_ref(
        "trace-commons:settlement-rail-statement:v1",
        &[rail_id, recipient_ref_hash, &cumulative_units.to_string()],
    )
}

/// One recipient's cumulative total, as the dispatcher hands it to a rail.
#[derive(Debug, Clone)]
pub struct CumulativeStatement {
    pub recipient: RecipientRef,
    /// A hash of the recipient, for audit.
    pub recipient_ref_hash: String,
    /// Everything ever accrued to this recipient on this rail, in atomic
    /// units of the rail's instrument.
    pub cumulative_units: u128,
    pub statement_ref: String,
}

/// What publishing did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Published {
    /// The external system now holds `effective_units` (after the call).
    /// `applied` is false when it already held at least our total.
    Recorded {
        effective_units: u128,
        applied: bool,
    },
    /// The external system applies statements on its own schedule (pull);
    /// nothing is known yet about what it holds.
    Deferred,
}

/// How a published or observed external total compares with our ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    /// It holds exactly our cumulative total.
    Current,
    /// It holds less than our total but no less than what it confirmed
    /// before: more is still to publish.
    Behind,
    /// It holds more than we ever accrued. Someone else wrote it, or our
    /// ledger lost an accrual. Freeze.
    Ahead,
    /// It holds less than it confirmed before. A monotonic total went down.
    /// Freeze.
    Regressed,
}

/// Compare an external total with our cumulative total and with what the
/// external side last confirmed.
pub fn compare(external: u128, cumulative: u128, confirmed: u128) -> Comparison {
    if external > cumulative {
        Comparison::Ahead
    } else if external < confirmed {
        Comparison::Regressed
    } else if external == cumulative {
        Comparison::Current
    } else {
        Comparison::Behind
    }
}

/// The disposition a failed attempt asks the dispatcher for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailErrorKind {
    /// Transient: back off and try again from the beginning.
    Retry,
    /// Stop this row until an operator acts. Never retried automatically.
    Frozen,
    /// A required control (configuration, credential) is absent or invalid.
    /// The whole rail stops.
    Refused,
    /// Dry run: everything but the external write succeeded.
    DryRun,
}

/// A rail failure. Label only; see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{kind:?}:{label}")]
pub struct RailError {
    pub kind: RailErrorKind,
    pub label: &'static str,
}

impl RailError {
    pub const fn retry(label: &'static str) -> Self {
        Self {
            kind: RailErrorKind::Retry,
            label,
        }
    }
    pub const fn frozen(label: &'static str) -> Self {
        Self {
            kind: RailErrorKind::Frozen,
            label,
        }
    }
    pub const fn refused(label: &'static str) -> Self {
        Self {
            kind: RailErrorKind::Refused,
            label,
        }
    }
    pub const fn dry_run(label: &'static str) -> Self {
        Self {
            kind: RailErrorKind::DryRun,
            label,
        }
    }
}

/// A settlement rail. Held as a trait object at the seam.
#[async_trait]
pub trait SettlementRail: Send + Sync {
    /// The `rail_id` column value this rail drains.
    fn rail_id(&self) -> &'static str;
    /// The instrument it pays in.
    fn instrument_id(&self) -> &'static str;
    /// The instrument's decimals: atomic units per whole unit is `10^decimals`.
    fn instrument_decimals(&self) -> u8;
    /// Publish a recipient's cumulative total. Safe to repeat.
    async fn publish(&self, statement: &CumulativeStatement) -> Result<Published, RailError>;
    /// Read what the external system holds for a recipient. `Ok(None)` when
    /// this rail cannot observe it (a pull rail without read access).
    async fn observe(&self, recipient: &RecipientRef) -> Result<Option<u128>, RailError>;
}

/// Whether a label fits the column vocabulary.
pub fn is_safe_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 64
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_statement_ref_is_stable_and_depends_on_every_input() {
        let h = statement_ref("near_ai_trace_commons", "sha256:aa", 42);
        assert_eq!(h, statement_ref("near_ai_trace_commons", "sha256:aa", 42));
        assert_ne!(h, statement_ref("near_ai_trace_commons", "sha256:aa", 43));
        assert_ne!(h, statement_ref("near_ai_trace_commons", "sha256:ab", 42));
        assert!(h.starts_with("sha256:") && h.len() == 71);
    }

    #[test]
    fn comparison_freezes_on_ahead_or_regressed_and_continues_behind() {
        // cumulative 100, previously confirmed 60.
        assert_eq!(compare(100, 100, 60), Comparison::Current);
        assert_eq!(compare(80, 100, 60), Comparison::Behind);
        assert_eq!(compare(60, 100, 60), Comparison::Behind);
        assert_eq!(compare(101, 100, 60), Comparison::Ahead);
        assert_eq!(compare(59, 100, 60), Comparison::Regressed);
        assert_eq!(compare(0, 0, 0), Comparison::Current);
    }

    #[test]
    fn a_recipient_never_debug_prints() {
        let r = RecipientRef::new("6f1c0b6a-0000-4000-8000-00000000abcd");
        assert_eq!(format!("{r:?}"), "RecipientRef(<redacted>)");
    }

    #[test]
    fn labels_are_checked_against_the_column_shape() {
        assert!(is_safe_label("near_ai_grant_bucket_foreign_write"));
        assert!(!is_safe_label("Has Space"));
        assert!(!is_safe_label(&"a".repeat(65)));
    }
}
