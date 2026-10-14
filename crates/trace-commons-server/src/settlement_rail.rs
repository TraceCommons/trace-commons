// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The seam a settlement rail plugs into.
//!
//! A rail turns one queued delivery -- an amount of one instrument for one
//! recipient -- into an external effect, exactly once, and proves it. The
//! dispatcher holds a rail as `Arc<dyn SettlementRail>` and knows nothing
//! about any particular external system. See
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
use uuid::Uuid;

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

/// The correlation hash for one outbox row. Built from the rail, the
/// instrument and the row's random id -- never from a tenant id -- so it can
/// be written into an external system's audit field.
pub fn delivery_hash(rail_id: &str, instrument_id: &str, outbox_id: Uuid) -> String {
    sha256_ref(
        "trace-commons:settlement-rail-delivery:v1",
        &[rail_id, instrument_id, &outbox_id.to_string()],
    )
}

/// A conversion rate from microcredits to an instrument's atomic units, as an
/// integer rational. Configured, never hard-coded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversionRate {
    numerator: u128,
    denominator: u128,
}

impl ConversionRate {
    /// Parse `"<numerator>/<denominator>"`, both positive decimal integers.
    /// A bare integer is accepted as `n/1`. Anything else, zero included, is
    /// refused: a zero rate would silently settle nothing.
    pub fn parse(text: &str) -> Result<Self, RailError> {
        let refuse = || RailError::refused("conversion_rate_malformed");
        let text = text.trim();
        let (num, den) = match text.split_once('/') {
            Some((n, d)) => (n.trim(), d.trim()),
            None => (text, "1"),
        };
        let digits =
            |s: &str| !s.is_empty() && s.len() <= 30 && s.bytes().all(|b| b.is_ascii_digit());
        if !digits(num) || !digits(den) {
            return Err(refuse());
        }
        let numerator: u128 = num.parse().map_err(|_| refuse())?;
        let denominator: u128 = den.parse().map_err(|_| refuse())?;
        if numerator == 0 || denominator == 0 {
            return Err(refuse());
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    pub fn numerator(&self) -> u128 {
        self.numerator
    }

    pub fn denominator(&self) -> u128 {
        self.denominator
    }

    /// The canonical text, `n/d`, as stored beside every queued row.
    pub fn canonical(&self) -> String {
        format!("{}/{}", self.numerator, self.denominator)
    }

    /// A hash of the canonical text, recorded on each row so a later rate
    /// change can be told apart from the rate a row was priced at.
    pub fn ref_hash(&self) -> String {
        sha256_ref(
            "trace-commons:settlement-rail-rate:v1",
            &[&self.canonical()],
        )
    }

    /// Convert `micros`, adding the carried remainder from earlier batches.
    ///
    /// Returns `(units, new_carry)` with
    /// `units * den + new_carry == micros * num + carry` and
    /// `new_carry < den`, so across any sequence of batches nothing is lost
    /// to rounding and nothing is invented. `carry` must be below the
    /// denominator. Overflow refuses rather than wrapping.
    pub fn convert(&self, micros: u128, carry: u128) -> Result<(u128, u128), RailError> {
        if carry >= self.denominator {
            return Err(RailError::refused("conversion_carry_out_of_range"));
        }
        let scaled = micros
            .checked_mul(self.numerator)
            .and_then(|v| v.checked_add(carry))
            .ok_or_else(|| RailError::refused("conversion_overflow"))?;
        Ok((scaled / self.denominator, scaled % self.denominator))
    }
}

/// What a rail last wrote for a recipient, as recorded by us. A rail whose
/// external effect is "set the absolute value" uses it as its precondition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorWrite {
    pub external_receipt_hash: String,
    pub written_units: u128,
}

/// One queued delivery, as the dispatcher hands it to a rail.
#[derive(Debug, Clone)]
pub struct RailDelivery {
    pub delivery_hash: String,
    pub recipient: RecipientRef,
    pub units: u128,
    /// `None` when we have never written for this recipient on this rail.
    pub expected_prior: Option<PriorWrite>,
}

/// Evidence that a delivery's external effect exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailReceipt {
    /// What the recipient's balance or ceiling was before our write.
    pub prior_units: u128,
    /// What our write set it to.
    pub written_units: u128,
    /// `sha256:` over the external system's record id. The id itself is not
    /// stored.
    pub external_receipt_hash: String,
    /// True when this attempt found the effect already present and did not
    /// write again.
    pub replayed: bool,
}

/// The result of a delivery whose effect exists. `anomaly` is set when the
/// effect landed but something about its surroundings was wrong -- for
/// example, another writer touched the same external value between our read
/// and our write. The dispatcher records the receipt and freezes the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailOutcome {
    pub receipt: RailReceipt,
    pub anomaly: Option<&'static str>,
}

/// What the reconciler learned about one recorded delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailVerification {
    /// Present with the recorded amount, and still the current value.
    Current,
    /// Present with the recorded amount; a later write of ours superseded it.
    Superseded,
    /// Present, but a write that is not ours came after it.
    Overwritten,
    /// Not present, or present with a different amount.
    Missing,
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
    /// Deliver, exactly once. Calling it again for the same `delivery_hash`
    /// after any failure, including an ambiguous one, must not deliver twice.
    async fn deliver(&self, delivery: &RailDelivery) -> Result<RailOutcome, RailError>;
    /// Check a recorded delivery against the external system.
    async fn verify(
        &self,
        delivery: &RailDelivery,
        receipt: &RailReceipt,
    ) -> Result<RailVerification, RailError>;
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
    fn a_rate_parses_only_positive_integer_rationals() {
        assert_eq!(ConversionRate::parse("3/2").unwrap().canonical(), "3/2");
        assert_eq!(
            ConversionRate::parse(" 1000 ").unwrap().canonical(),
            "1000/1"
        );
        for bad in ["", "0/1", "1/0", "1.5/1", "-1/2", "a/b", "1/2/3", "1e3"] {
            assert_eq!(
                ConversionRate::parse(bad).unwrap_err(),
                RailError::refused("conversion_rate_malformed"),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn conversion_carries_the_remainder_and_never_loses_value() {
        let rate = ConversionRate::parse("7/3").unwrap();
        let mut carry = 0u128;
        let mut paid = 0u128;
        let batches = [1u128, 1, 1, 5, 0, 2];
        for micros in batches {
            let (units, next) = rate.convert(micros, carry).unwrap();
            assert!(next < 3);
            paid += units;
            carry = next;
        }
        let total: u128 = batches.iter().sum();
        assert_eq!(paid * 3 + carry, total * 7);
    }

    #[test]
    fn conversion_refuses_overflow_and_a_carry_at_or_above_the_denominator() {
        let rate = ConversionRate::parse("2/1").unwrap();
        assert_eq!(
            rate.convert(u128::MAX, 0).unwrap_err().label,
            "conversion_overflow"
        );
        let rate = ConversionRate::parse("1/4").unwrap();
        assert_eq!(
            rate.convert(1, 4).unwrap_err().label,
            "conversion_carry_out_of_range"
        );
    }

    #[test]
    fn the_rate_hash_changes_with_the_rate() {
        let a = ConversionRate::parse("1/1").unwrap().ref_hash();
        let b = ConversionRate::parse("2/2").unwrap().ref_hash();
        assert_ne!(a, b, "2/2 is not reduced, and is a different configuration");
        assert!(a.starts_with("sha256:") && a.len() == 71);
    }

    #[test]
    fn a_delivery_hash_is_stable_and_carries_no_input_in_clear() {
        let id = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
        let h = delivery_hash("near_ai_org_limits", "near_ai_credit_usd", id);
        assert_eq!(
            h,
            delivery_hash("near_ai_org_limits", "near_ai_credit_usd", id)
        );
        assert!(!h.contains("00000000"));
        assert_ne!(h, delivery_hash("other", "near_ai_credit_usd", id));
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
