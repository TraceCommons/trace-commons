// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The payout rail seam: one way a settled Trace Credit award is paid out
//! to a credit account's linked recipient, outside the credit ledger.
//!
//! The **award** is the settled Trace Credit on a finalized settlement batch
//! line. A **rail** pays that award out in its own instrument. Every rail
//! reads the same finalized lines; none reads another rail's records, and
//! none consumes the award. The payout pass in `trace-commons-server` holds
//! every rail as `Arc<dyn PayoutRail>`, never a concrete type, so a
//! substituted backend can slot in at this seam.
//!
//! The contract is rail-neutral. Nothing here assumes how a rail meets the
//! exactly-once rule: a rail that sends a running total, a rail that sends
//! an amount under an idempotency key derived from the line, and an
//! on-chain rail that carries the line in a memo or nonce all fit. The
//! keyed test rail in `trace-commons-gate-enclave` proves the second shape.
//!
//! This is not [`crate::SettlementAdapter`]: that one performs the award
//! instrument's own effect per settlement leg inside the pipeline's settle
//! step. A rail pays an award out per account, after settlement.

use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::pipeline::{
    AtomicUnits, ContractError, InstrumentDescriptor, InstrumentId, is_safe_identifier, is_sha256,
};

/// The NEAR AI inference-credit rail. Never `near`: that label belongs to
/// the NEP-141 outbox rail ([`crate::SettlementAdapter::payout_rail`]).
pub const NEAR_AI_RAIL_ID: &str = "near_ai";
/// The Trace Credit rail. It is a rail in name and in readiness reporting
/// only: its "send" is the settlement batch line itself, and the payout
/// pass never drives it.
pub const TRACE_CREDIT_RAIL_ID: &str = "trace_credit";
/// The recipient-link kind of the `near_ai` rail.
pub const NEAR_AI_ORG_RECIPIENT_KIND: &str = "near_ai_org";
/// Longest accepted rate version label.
pub const MAX_RAIL_RATE_VERSION_LEN: usize = 64;
/// What [`RailRefusal::safe_label`] and `Display` print in place of a
/// rail's label that is not a safe identifier.
pub const RAIL_REFUSAL_FALLBACK_LABEL: &str = "payout_rail_label_invalid";
/// Longest accepted opaque recipient value, in bytes.
pub const MAX_RECIPIENT_REF_LEN: usize = 4096;

/// A payout rail's identifier: the same rule as [`InstrumentId`], 1 to 64
/// lowercase ASCII letters, digits, `_`, `-`, or `.`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(try_from = "String", into = "String")]
pub struct RailId(String);

impl TryFrom<String> for RailId {
    type Error = ContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RailId> for String {
    fn from(id: RailId) -> Self {
        id.0
    }
}

impl fmt::Display for RailId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl RailId {
    pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if !is_safe_identifier(&value) {
            return Err(ContractError::InvalidRailId);
        }
        Ok(Self(value))
    }

    /// The only constructor the server uses for the NEAR AI rail.
    pub fn near_ai() -> Self {
        Self(NEAR_AI_RAIL_ID.to_string())
    }

    pub fn trace_credit() -> Self {
        Self(TRACE_CREDIT_RAIL_ID.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a rail's recipient link holds, and which proof verifier the link
/// route runs for it. A safe label under the [`InstrumentId`] rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(try_from = "String", into = "String")]
pub struct RecipientKind(String);

impl TryFrom<String> for RecipientKind {
    type Error = ContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RecipientKind> for String {
    fn from(kind: RecipientKind) -> Self {
        kind.0
    }
}

impl RecipientKind {
    pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if !is_safe_identifier(&value) {
            return Err(ContractError::InvalidRecipientKind);
        }
        Ok(Self(value))
    }

    pub fn near_ai_org() -> Self {
        Self(NEAR_AI_ORG_RECIPIENT_KIND.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a rail pays in and to whom.
///
/// Every amount the pass handles for a rail is [`AtomicUnits`] in
/// `instrument`. A rail whose wire is narrower than `u128` states its
/// largest amount in `wire_max`, narrows at its own boundary, and refuses
/// an amount out of range there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailDescriptor {
    pub rail_id: RailId,
    pub instrument_id: InstrumentId,
    pub instrument: InstrumentDescriptor,
    pub recipient_kind: RecipientKind,
    pub wire_max: Option<AtomicUnits>,
}

impl RailDescriptor {
    /// Checks the instrument descriptor (kind, location, `decimals`), then
    /// refuses a `wire_max` of zero, which no send could fit under, as a
    /// fault in the rail's own descriptor.
    pub fn validate(&self) -> Result<(), ContractError> {
        self.instrument.validate()?;
        if self.wire_max == Some(AtomicUnits::ZERO) {
            return Err(ContractError::InvalidRailDescriptor);
        }
        Ok(())
    }
}

/// A rate version label: 1 to 64 ASCII letters (either case), digits, `_`,
/// `-`, or `.`. Wider than [`InstrumentId`]'s rule because a rate version is
/// taken as its source spells it (a settlement table's `version` may carry
/// capitals); a version is never lower-cased, so two spellings stay two
/// versions.
fn is_safe_rate_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_RAIL_RATE_VERSION_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

/// One immutable rate: `rail_units = floor(award_atomic_units * numerator /
/// denominator)`. A version names one rate forever.
///
/// The fraction is kept as given, not reduced: `(2, 1)` and `(4, 2)` are
/// different terms, so a caller that checks whether a version's rate
/// changed compares the raw `(numerator, denominator)` pair on purpose.
/// [`RailRate::price`] refuses when `award * numerator` overflows `u128`,
/// even where the quotient would fit: a refusal, never an approximation.
///
/// `Debug` prints the version only: rates stay out of logs.
#[derive(Clone, PartialEq, Eq)]
pub struct RailRate {
    version: String,
    numerator: u128,
    denominator: u128,
}

impl fmt::Debug for RailRate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RailRate")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl RailRate {
    /// Refuses a zero numerator or denominator and an unsafe version label.
    pub fn new(
        version: impl Into<String>,
        numerator: u128,
        denominator: u128,
    ) -> Result<Self, ContractError> {
        let version = version.into();
        if numerator == 0 || denominator == 0 || !is_safe_rate_version(&version) {
            return Err(ContractError::InvalidRailRate);
        }
        Ok(Self {
            version,
            numerator,
            denominator,
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn numerator(&self) -> u128 {
        self.numerator
    }

    pub fn denominator(&self) -> u128 {
        self.denominator
    }

    /// `floor(award_units * numerator / denominator)`, checked. A product
    /// that does not fit `u128` is refused, never approximated.
    pub fn price(&self, award_units: u128) -> Result<u128, ContractError> {
        award_units
            .checked_mul(self.numerator)
            .map(|product| product / self.denominator)
            .ok_or(ContractError::AtomicUnitOverflow)
    }
}

/// A rail's recipient, opaque to everything but the rail. It is unsealed
/// from storage for the length of one call. `Debug` never prints it, and it
/// has no `Display` and no `Serialize`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RecipientRef(String);

impl fmt::Debug for RecipientRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RecipientRef(..)")
    }
}

impl RecipientRef {
    /// Refuses an empty value or one longer than [`MAX_RECIPIENT_REF_LEN`].
    pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_RECIPIENT_REF_LEN {
            return Err(ContractError::InvalidRecipientRef);
        }
        Ok(Self(value))
    }

    /// The recipient value, for the rail's own wire call only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// One send: what the pass asks a rail to land, identified by its line.
///
/// `target_total` is everything this link's program should have received
/// once the line lands; `confirmed_total` is what we have confirmed on the
/// program; `delta` is the difference. A rail uses whichever of `delta` and
/// `target_total` its exactly-once mechanism needs.
///
/// Outside this crate a line is built only through [`RailLine::new`], which
/// derives `delta` and checks the reference; its fields stay readable:
///
/// ```
/// use trace_commons_gate_api::payout_rail::{RailLine, RecipientRef};
/// use trace_commons_gate_api::pipeline::AtomicUnits;
///
/// let recipient = RecipientRef::new("r").unwrap();
/// let line = RailLine::new(
///     "sha256:0000000000000000000000000000000000000000000000000000000000000001",
///     &recipient,
///     "p",
///     AtomicUnits::from_raw(3),
///     AtomicUnits::from_raw(1),
/// )
/// .unwrap();
/// assert_eq!(line.delta, AtomicUnits::from_raw(2));
/// ```
///
/// A struct literal, which would skip those checks, does not compile:
///
/// ```compile_fail
/// use trace_commons_gate_api::payout_rail::{RailLine, RecipientRef};
/// use trace_commons_gate_api::pipeline::AtomicUnits;
///
/// let recipient = RecipientRef::new("r").unwrap();
/// let _line = RailLine {
///     line_ref_hash: "sha256:0000000000000000000000000000000000000000000000000000000000000001",
///     recipient: &recipient,
///     program: "p",
///     target_total: AtomicUnits::from_raw(3),
///     delta: AtomicUnits::from_raw(9),
///     confirmed_total: AtomicUnits::from_raw(1),
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RailLine<'a> {
    /// `sha256:` reference that identifies the line. A rail derives any
    /// idempotency key from this.
    pub line_ref_hash: &'a str,
    pub recipient: &'a RecipientRef,
    /// The link's program on this rail ([`PayoutRail::program_for_link`]).
    pub program: &'a str,
    pub target_total: AtomicUnits,
    pub delta: AtomicUnits,
    pub confirmed_total: AtomicUnits,
}

impl<'a> RailLine<'a> {
    /// Builds a line with `delta = target_total - confirmed_total`. Refuses
    /// a line reference that is not a lowercase `sha256:` hash, an empty
    /// program, and a target below the confirmed total (that is netting,
    /// which sends nothing).
    pub fn new(
        line_ref_hash: &'a str,
        recipient: &'a RecipientRef,
        program: &'a str,
        target_total: AtomicUnits,
        confirmed_total: AtomicUnits,
    ) -> Result<Self, ContractError> {
        if !is_sha256(line_ref_hash) || program.is_empty() {
            return Err(ContractError::InvalidRailLine);
        }
        let delta = target_total
            .get()
            .checked_sub(confirmed_total.get())
            .map(AtomicUnits::from_raw)
            .ok_or(ContractError::InvalidRailLine)?;
        Ok(Self {
            line_ref_hash,
            recipient,
            program,
            target_total,
            delta,
            confirmed_total,
        })
    }
}

/// What a send achieved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// The line landed (now, or on an earlier call for the same line).
    /// `receipt_hash` is a `sha256:` hash of the rail's own receipt id.
    Committed {
        receipt_hash: String,
        delta: AtomicUnits,
        total_after: Option<AtomicUnits>,
    },
    /// Nothing was sent: the program already holds `total`, at or above the
    /// line's target.
    AlreadyAtOrAbove { total: AtomicUnits },
}

/// The external record that answers a line, if there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptLookup {
    Found {
        receipt_hash: String,
        delta: AtomicUnits,
    },
    NotFound,
}

/// What a rail can send right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RailCapacity {
    pub sending_enabled: bool,
    /// A safe label saying why sending is off, when it is.
    pub stop_label: Option<&'static str>,
    /// The most one send may carry. Meaningless when `sends_remaining` is
    /// `Some(0)`: a rail may report 0 then, and the pass waits for the
    /// window rather than reading it as a stop.
    pub per_send_max: AtomicUnits,
    /// Sends left in the rail's current window, when the rail has one.
    pub sends_remaining: Option<u32>,
}

/// One external record on a program, for reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailHistoryEntry {
    pub receipt_hash: String,
    pub program: String,
    pub delta: AtomicUnits,
    pub total_after: Option<AtomicUnits>,
}

/// Every rail error, mapped into one closed set. The pass acts on the
/// variant only, never on a rail's raw error. Each carried label must be a
/// safe identifier (the [`InstrumentId`] rule); a rail is outside this
/// crate, so that is checked at run time ([`RailRefusal::label_is_safe`]),
/// and `Display` prints [`RailRefusal::safe_label`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailRefusal {
    /// Try again later; the line stays pending.
    Retry(&'static str),
    /// Stop this rail's pass and change no row. Every credential fault is
    /// this.
    StopPass(&'static str),
    /// Hold this line for an operator.
    HoldLine(&'static str),
    /// The recipient is gone or will not accept: hold, and mark the link.
    RecipientUnavailable,
    /// A fault on our side.
    Failed(&'static str),
}

impl RailRefusal {
    pub fn label(self) -> &'static str {
        match self {
            Self::Retry(label) | Self::StopPass(label) | Self::HoldLine(label) => label,
            Self::Failed(label) => label,
            Self::RecipientUnavailable => "payout_recipient_unavailable",
        }
    }

    /// Whether [`RailRefusal::label`] is a safe identifier.
    pub fn label_is_safe(self) -> bool {
        is_safe_identifier(self.label())
    }

    /// The label when it is safe, otherwise [`RAIL_REFUSAL_FALLBACK_LABEL`].
    /// The payout pass logs and stores this, never [`RailRefusal::label`].
    pub fn safe_label(self) -> &'static str {
        if self.label_is_safe() {
            self.label()
        } else {
            RAIL_REFUSAL_FALLBACK_LABEL
        }
    }
}

impl fmt::Display for RailRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.safe_label())
    }
}

impl std::error::Error for RailRefusal {}

/// One external payout rail.
///
/// **Exactly-once contract.** A send is identified by its line
/// ([`RailLine::line_ref_hash`]). A repeated `send` for the same line never
/// repeats the effect: it returns the first call's outcome, or
/// `AlreadyAtOrAbove`. Each rail meets this with its own mechanism.
///
/// **Recovery** depends on it (payout spec section 4, "Crash recovery, per
/// rail"). The pass records a line as pending before it calls `send`, so
/// after a crash or a lost response the next pass:
///
/// 1. calls `send` again for the **same** line first. If the first send
///    never committed, it lands now (`Committed`); if it did, a keyed rail
///    answers `Committed` with the first receipt and a running-total rail
///    answers `AlreadyAtOrAbove`;
/// 2. only on `AlreadyAtOrAbove`, calls `find_receipt` for the line,
///    excluding receipts already claimed by other lines;
/// 3. if that finds no record, holds the line as
///    `payout_recovery_unmatched` for an operator. It never treats
///    `AlreadyAtOrAbove` alone as proof that this line was paid.
///
/// A rail that cannot answer `read_total`, `find_receipt` and `history`
/// exactly cannot meet the recovery and close rules, and does not qualify.
#[async_trait]
pub trait PayoutRail: Send + Sync {
    /// Rail id, instrument with decimals, recipient-link kind, wire bound.
    fn descriptor(&self) -> &RailDescriptor;

    /// A safe label that names the implementation in hash-only reports.
    fn adapter_identity(&self) -> &str;

    /// `false` by default. Readiness fails closed on `false`.
    fn production_qualified(&self) -> bool {
        false
    }

    /// The program a link's sends land in. Derived from the link id alone,
    /// so a new link (a changed recipient) starts a new program.
    fn program_for_link(&self, link_id: Uuid) -> String;

    /// The second program a link carries for referral credit (referral
    /// rewards spec section 5), derived from the link id alone and never
    /// equal to [`PayoutRail::program_for_link`]. `None` by default: a rail
    /// without one carries no referral stream, and the pass sends none.
    fn referral_program_for_link(&self, _link_id: Uuid) -> Option<String> {
        None
    }

    /// Whether sending is enabled, the per-send maximum, and the sends left
    /// in the current window.
    async fn preflight(&self) -> Result<RailCapacity, RailRefusal>;

    /// The external total on `program` for `recipient`, exact.
    async fn read_total(
        &self,
        recipient: &RecipientRef,
        program: &str,
    ) -> Result<AtomicUnits, RailRefusal>;

    /// Lands one line, exactly once (see the trait documentation).
    async fn send(&self, line: &RailLine<'_>) -> Result<SendOutcome, RailRefusal>;

    /// The external record that answers `line`, skipping any receipt in
    /// `exclude` (receipts already claimed by other lines).
    async fn find_receipt(
        &self,
        line: &RailLine<'_>,
        exclude: &[String],
    ) -> Result<ReceiptLookup, RailRefusal>;

    /// Every external record on `program` for `recipient`, for the
    /// reconciliation drill.
    async fn history(
        &self,
        recipient: &RecipientRef,
        program: &str,
    ) -> Result<Vec<RailHistoryEntry>, RailRefusal>;

    /// Whether `recipient` can receive on this rail, checked at link time.
    async fn probe_recipient(&self, recipient: &RecipientRef) -> Result<(), RailRefusal>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::InstrumentKind;

    fn keyed_descriptor(decimals: u8) -> RailDescriptor {
        RailDescriptor {
            rail_id: RailId::new("test_keyed").unwrap(),
            instrument_id: InstrumentId::new("test_token").unwrap(),
            instrument: InstrumentDescriptor {
                kind: InstrumentKind::Erc20,
                network: "1".to_string(),
                contract: "0x00000000000000000000000000000000000000a1".to_string(),
                decimals,
            },
            recipient_kind: RecipientKind::new("test_address").unwrap(),
            wire_max: None,
        }
    }

    #[test]
    fn rail_id_follows_the_instrument_id_rule() {
        assert_eq!(RailId::new("near_ai").unwrap().as_str(), NEAR_AI_RAIL_ID);
        assert_eq!(RailId::near_ai().as_str(), "near_ai");
        assert_eq!(RailId::trace_credit().as_str(), TRACE_CREDIT_RAIL_ID);
        for refused in ["", "Near", "near ai", "near/ai", &"a".repeat(65)] {
            assert_eq!(
                RailId::new(refused),
                Err(ContractError::InvalidRailId),
                "{refused:?} must be refused"
            );
            // The same strings are refused as instrument ids: one rule.
            assert!(InstrumentId::new(refused).is_err());
        }
        assert!(RailId::new("a".repeat(64)).is_ok());
        // The NEAR AI rail is never the NEP-141 outbox rail's label.
        assert_ne!(RailId::near_ai().as_str(), "near");

        let parsed: RailId = serde_json::from_str("\"near_ai\"").unwrap();
        assert_eq!(parsed, RailId::near_ai());
        assert!(serde_json::from_str::<RailId>("\"Near_AI\"").is_err());
        assert_eq!(serde_json::to_string(&parsed).unwrap(), "\"near_ai\"");

        assert_eq!(
            RecipientKind::near_ai_org().as_str(),
            NEAR_AI_ORG_RECIPIENT_KIND
        );
        assert_eq!(
            RecipientKind::new("Org"),
            Err(ContractError::InvalidRecipientKind)
        );
    }

    #[test]
    fn rail_rate_prices_with_floor_and_checks_overflow() {
        let rate = RailRate::new("v1", 3, 2).unwrap();
        assert_eq!(rate.price(5), Ok(7));
        assert_eq!(rate.price(0), Ok(0));
        assert_eq!(rate.price(1), Ok(1));
        assert_eq!(
            RailRate::new("v1", 2, 1).unwrap().price(u128::MAX),
            Err(ContractError::AtomicUnitOverflow)
        );
        // Six-decimal credit into an 18-decimal token: 10^12 per unit.
        let wide = RailRate::new("v1", 1_000_000_000_000, 1).unwrap();
        assert_eq!(wide.price(1_000_000), Ok(1_000_000_000_000_000_000));
        // A large award with a large denominator does not overflow early
        // when the product fits.
        let narrow = RailRate::new("v1", 1, 1_000_000).unwrap();
        assert_eq!(narrow.price(u128::MAX), Ok(u128::MAX / 1_000_000));
        assert_eq!(rate.version(), "v1");
        assert_eq!(rate.numerator(), 3);
        assert_eq!(rate.denominator(), 2);
    }

    #[test]
    fn rail_rate_refuses_zero_terms_and_unsafe_versions() {
        assert_eq!(
            RailRate::new("v1", 0, 1),
            Err(ContractError::InvalidRailRate)
        );
        assert_eq!(
            RailRate::new("v1", 1, 0),
            Err(ContractError::InvalidRailRate)
        );
        for refused in ["", "v 1", "v/1", "v1\n", "s\u{e9}", "{v1}", &"a".repeat(65)] {
            assert_eq!(
                RailRate::new(refused, 1, 1),
                Err(ContractError::InvalidRailRate),
                "{refused:?} must be refused"
            );
        }
        // Settlement-table version labels are ASCII letters, digits, `-`
        // and `_`, in either case; each is accepted as it is spelled.
        for accepted in ["s3", "S3", "p2-2026_10", "v1.2", &"a".repeat(64)] {
            assert_eq!(RailRate::new(accepted, 1, 1).unwrap().version(), accepted);
        }
    }

    #[test]
    fn rail_rate_debug_prints_the_version_only() {
        let rate = RailRate::new("s3", 1_234_567_000, 7_654_321).unwrap();
        for rendered in [format!("{rate:?}"), format!("{rate:#?}")] {
            assert!(rendered.contains("s3"), "{rendered}");
            assert!(!rendered.contains("1234567000"), "{rendered}");
            assert!(!rendered.contains("7654321"), "{rendered}");
        }
    }

    #[test]
    fn a_descriptor_with_bad_decimals_is_refused() {
        assert_eq!(keyed_descriptor(18).validate(), Ok(()));
        assert_eq!(keyed_descriptor(38).validate(), Ok(()));
        assert_eq!(
            keyed_descriptor(39).validate(),
            Err(ContractError::InvalidInstrumentDescriptor)
        );
        let mut bad_contract = keyed_descriptor(18);
        bad_contract.instrument.contract = "0xABC".to_string();
        assert_eq!(
            bad_contract.validate(),
            Err(ContractError::InvalidInstrumentDescriptor)
        );
        // A zero wire bound is the rail descriptor's own fault, not the
        // instrument's.
        let mut zero_wire = keyed_descriptor(18);
        zero_wire.wire_max = Some(AtomicUnits::ZERO);
        assert_eq!(
            zero_wire.validate(),
            Err(ContractError::InvalidRailDescriptor)
        );
        zero_wire.wire_max = Some(AtomicUnits::from_raw(1));
        assert_eq!(zero_wire.validate(), Ok(()));
    }

    #[test]
    fn recipient_ref_debug_never_prints_the_value() {
        let value = "recipient-secret-value-0001";
        let recipient = RecipientRef::new(value).unwrap();
        assert_eq!(recipient.expose(), value);
        for rendered in [format!("{recipient:?}"), format!("{recipient:#?}")] {
            assert_eq!(rendered, "RecipientRef(..)");
        }
        let line = RailLine::new(
            "sha256:0000000000000000000000000000000000000000000000000000000000000001",
            &recipient,
            "program-1",
            AtomicUnits::from_raw(10),
            AtomicUnits::from_raw(4),
        )
        .unwrap();
        assert_eq!(line.delta, AtomicUnits::from_raw(6));
        let rendered = format!("{line:?}");
        assert!(!rendered.contains(value), "{rendered}");
        assert_eq!(
            RecipientRef::new(""),
            Err(ContractError::InvalidRecipientRef)
        );
        assert_eq!(
            RecipientRef::new("a".repeat(MAX_RECIPIENT_REF_LEN + 1)),
            Err(ContractError::InvalidRecipientRef)
        );
    }

    #[test]
    fn a_line_refuses_a_bad_ref_or_a_target_below_confirmed() {
        let recipient = RecipientRef::new("r").unwrap();
        let good_ref = "sha256:0000000000000000000000000000000000000000000000000000000000000001";
        assert_eq!(
            RailLine::new(
                "not-a-hash",
                &recipient,
                "p",
                AtomicUnits::from_raw(1),
                AtomicUnits::ZERO
            )
            .unwrap_err(),
            ContractError::InvalidRailLine
        );
        assert_eq!(
            RailLine::new(
                good_ref,
                &recipient,
                "p",
                AtomicUnits::from_raw(1),
                AtomicUnits::from_raw(2)
            )
            .unwrap_err(),
            ContractError::InvalidRailLine
        );
        assert_eq!(
            RailLine::new(
                good_ref,
                &recipient,
                "",
                AtomicUnits::from_raw(1),
                AtomicUnits::ZERO
            )
            .unwrap_err(),
            ContractError::InvalidRailLine
        );
        let at = RailLine::new(
            good_ref,
            &recipient,
            "p",
            AtomicUnits::from_raw(2),
            AtomicUnits::from_raw(2),
        )
        .unwrap();
        assert_eq!(at.delta, AtomicUnits::ZERO);
    }

    #[test]
    fn refusal_labels_are_safe_identifiers() {
        let refusals = [
            RailRefusal::Retry("payout_rail_unavailable"),
            RailRefusal::StopPass("payout_credential_refused"),
            RailRefusal::HoldLine("payout_amount_out_of_range"),
            RailRefusal::RecipientUnavailable,
            RailRefusal::Failed("payout_rail_response_invalid"),
        ];
        for refusal in refusals {
            assert!(
                is_safe_identifier(refusal.label()),
                "{:?} has an unsafe label",
                refusal
            );
            assert_eq!(refusal.to_string(), refusal.label());
        }
        assert_eq!(
            RailRefusal::RecipientUnavailable.label(),
            "payout_recipient_unavailable"
        );
        assert_eq!(RailRefusal::Retry("x").label(), "x");
        assert_eq!(RailRefusal::StopPass("y").label(), "y");
        assert_eq!(RailRefusal::HoldLine("z").label(), "z");
        assert_eq!(RailRefusal::Failed("w").label(), "w");
    }

    #[test]
    fn an_unsafe_refusal_label_is_replaced_before_it_can_be_logged() {
        for refusal in [
            RailRefusal::Retry("payout_rail_unavailable"),
            RailRefusal::RecipientUnavailable,
        ] {
            assert!(refusal.label_is_safe());
            assert_eq!(refusal.safe_label(), refusal.label());
            assert_eq!(refusal.to_string(), refusal.label());
        }
        for refusal in [
            RailRefusal::Retry("has a space"),
            RailRefusal::StopPass("Token=abc"),
            RailRefusal::HoldLine(""),
            RailRefusal::Failed("https://example.invalid/x"),
        ] {
            assert!(!refusal.label_is_safe(), "{refusal:?}");
            assert_eq!(refusal.safe_label(), RAIL_REFUSAL_FALLBACK_LABEL);
            // Display is what a log line carries: never the unsafe label.
            assert_eq!(refusal.to_string(), RAIL_REFUSAL_FALLBACK_LABEL);
        }
        assert!(is_safe_identifier(RAIL_REFUSAL_FALLBACK_LABEL));
    }
}
