// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `MockKeyedPayoutRail`: an in-memory, amount-mode payout rail for tests.
//!
//! It meets the [`PayoutRail`] exactly-once contract the way an
//! idempotency-keyed rail does: it applies a line's `delta` once per
//! `line_ref_hash` and answers a repeat from the first receipt. Its
//! instrument is an 18-decimal ERC-20 token, not a credit account, so the
//! generic payout pass, storage and drill can be run against a rail that is
//! nothing like the NEAR AI one.
//!
//! A test double like [`crate::MockPerplexityScorer`]: it never moves value
//! and must never be built from configuration. `production_qualified()` is
//! `false`.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};

use sha2::{Digest, Sha256};
use trace_commons_gate_api::async_trait;
use trace_commons_gate_api::payout_rail::{
    PayoutRail, RailCapacity, RailDescriptor, RailHistoryEntry, RailId, RailLine, RailRate,
    RailRefusal, ReceiptLookup, RecipientKind, RecipientRef, SendOutcome,
};
use trace_commons_gate_api::pipeline::{
    AtomicUnits, InstrumentDescriptor, InstrumentId, InstrumentKind,
};
use uuid::Uuid;

/// The keyed test rail's id.
pub const MOCK_KEYED_RAIL_ID: &str = "test_keyed";
/// The keyed test rail's instrument id.
pub const MOCK_KEYED_INSTRUMENT_ID: &str = "test_token";
/// The keyed test rail's recipient-link kind.
pub const MOCK_KEYED_RECIPIENT_KIND: &str = "test_address";
/// A synthetic token contract address. It names no real token.
pub const MOCK_KEYED_TOKEN_CONTRACT: &str = "0x00000000000000000000000000000000000000a1";
/// The keyed test rail's decimals.
pub const MOCK_KEYED_DECIMALS: u8 = 18;

#[derive(Debug, Clone)]
struct KeyedReceipt {
    receipt_hash: String,
    recipient: RecipientRef,
    program: String,
    delta: AtomicUnits,
    total_after: AtomicUnits,
}

#[derive(Debug)]
struct KeyedState {
    receipts: HashMap<String, KeyedReceipt>,
    totals: HashMap<(RecipientRef, String), u128>,
    history: Vec<KeyedReceipt>,
    effects: u64,
    next_refusal: Option<RailRefusal>,
    lose_next_response: bool,
    sending_enabled: bool,
    per_send_max: AtomicUnits,
    unavailable: HashSet<RecipientRef>,
}

/// In-memory, idempotency-keyed payout rail. See the module documentation.
#[derive(Debug)]
pub struct MockKeyedPayoutRail {
    descriptor: RailDescriptor,
    state: Mutex<KeyedState>,
}

impl Default for MockKeyedPayoutRail {
    fn default() -> Self {
        Self::new()
    }
}

impl MockKeyedPayoutRail {
    pub fn new() -> Self {
        let descriptor = RailDescriptor {
            rail_id: RailId::new(MOCK_KEYED_RAIL_ID).expect("static rail id is safe"),
            instrument_id: InstrumentId::new(MOCK_KEYED_INSTRUMENT_ID)
                .expect("static instrument id is safe"),
            instrument: InstrumentDescriptor {
                kind: InstrumentKind::Erc20,
                network: "1".to_string(),
                contract: MOCK_KEYED_TOKEN_CONTRACT.to_string(),
                decimals: MOCK_KEYED_DECIMALS,
            },
            recipient_kind: RecipientKind::new(MOCK_KEYED_RECIPIENT_KIND)
                .expect("static recipient kind is safe"),
            wire_max: None,
        };
        Self {
            descriptor,
            state: Mutex::new(KeyedState {
                receipts: HashMap::new(),
                totals: HashMap::new(),
                history: Vec::new(),
                effects: 0,
                next_refusal: None,
                lose_next_response: false,
                sending_enabled: true,
                per_send_max: AtomicUnits::from_raw(u128::MAX),
                unavailable: HashSet::new(),
            }),
        }
    }

    /// A synthetic rate for this rail: one six-decimal credit unit is
    /// `10^12` token units, so one whole credit is one whole token.
    pub fn rate() -> RailRate {
        RailRate::new("test-keyed-v1", 1_000_000_000_000, 1).expect("static rate is valid")
    }

    /// External effects applied so far. A repeated send never adds one.
    pub fn effects(&self) -> u64 {
        self.state().effects
    }

    /// The next call to any method that reaches the rail returns `refusal`
    /// and changes nothing.
    pub fn refuse_next(&self, refusal: RailRefusal) {
        self.state().next_refusal = Some(refusal);
    }

    /// The next send that commits returns `Retry("lost_response")` after
    /// committing, as a response lost on the wire would look.
    pub fn lose_next_response(&self) {
        self.state().lose_next_response = true;
    }

    pub fn set_sending_enabled(&self, enabled: bool) {
        self.state().sending_enabled = enabled;
    }

    pub fn set_per_send_max(&self, max: AtomicUnits) {
        self.state().per_send_max = max;
    }

    /// `probe_recipient` answers `RecipientUnavailable` for `recipient`.
    pub fn mark_recipient_unavailable(&self, recipient: &RecipientRef) {
        self.state().unavailable.insert(recipient.clone());
    }

    fn state(&self) -> MutexGuard<'_, KeyedState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn receipt_hash(&self, line_ref_hash: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.descriptor.rail_id.as_str().as_bytes());
        hasher.update([0u8]);
        hasher.update(line_ref_hash.as_bytes());
        format!("sha256:{:x}", hasher.finalize())
    }
}

impl KeyedReceipt {
    /// Whether this receipt, stored under a line's key, is that line: the
    /// same recipient, program and delta.
    fn answers(&self, line: &RailLine<'_>) -> bool {
        self.recipient == *line.recipient
            && self.program == line.program
            && self.delta == line.delta
    }
}

impl KeyedState {
    fn take_refusal(&mut self) -> Result<(), RailRefusal> {
        match self.next_refusal.take() {
            Some(refusal) => Err(refusal),
            None => Ok(()),
        }
    }

    fn total(&self, recipient: &RecipientRef, program: &str) -> u128 {
        self.totals
            .get(&(recipient.clone(), program.to_string()))
            .copied()
            .unwrap_or(0)
    }
}

fn committed(receipt: &KeyedReceipt) -> SendOutcome {
    SendOutcome::Committed {
        receipt_hash: receipt.receipt_hash.clone(),
        delta: receipt.delta,
        total_after: Some(receipt.total_after),
    }
}

#[async_trait]
impl PayoutRail for MockKeyedPayoutRail {
    fn descriptor(&self) -> &RailDescriptor {
        &self.descriptor
    }

    fn adapter_identity(&self) -> &str {
        "mock_keyed_payout_rail"
    }

    fn program_for_link(&self, link_id: Uuid) -> String {
        format!("tk1-{}", link_id.simple())
    }

    async fn preflight(&self) -> Result<RailCapacity, RailRefusal> {
        let mut state = self.state();
        state.take_refusal()?;
        Ok(RailCapacity {
            sending_enabled: state.sending_enabled,
            stop_label: (!state.sending_enabled).then_some("payout_rail_sending_disabled"),
            per_send_max: state.per_send_max,
            sends_remaining: None,
        })
    }

    async fn read_total(
        &self,
        recipient: &RecipientRef,
        program: &str,
    ) -> Result<AtomicUnits, RailRefusal> {
        let mut state = self.state();
        state.take_refusal()?;
        Ok(AtomicUnits::from_raw(state.total(recipient, program)))
    }

    async fn send(&self, line: &RailLine<'_>) -> Result<SendOutcome, RailRefusal> {
        let mut state = self.state();
        state.take_refusal()?;
        if let Some(existing) = state.receipts.get(line.line_ref_hash) {
            return if existing.answers(line) {
                Ok(committed(existing))
            } else {
                Err(RailRefusal::Failed("payout_line_conflict"))
            };
        }
        let current = state.total(line.recipient, line.program);
        if line.delta == AtomicUnits::ZERO {
            // A zero delta means our confirmed total already equals the
            // target. The rail agrees only if it holds at least that much;
            // below it, the two totals disagree and the line is held.
            return if current >= line.target_total.get() {
                Ok(SendOutcome::AlreadyAtOrAbove {
                    total: AtomicUnits::from_raw(current),
                })
            } else {
                Err(RailRefusal::HoldLine("payout_total_mismatch"))
            };
        }
        if !state.sending_enabled {
            return Err(RailRefusal::StopPass("payout_rail_sending_disabled"));
        }
        if line.delta > state.per_send_max {
            return Err(RailRefusal::HoldLine("payout_send_over_max"));
        }
        let total_after = current
            .checked_add(line.delta.get())
            .ok_or(RailRefusal::HoldLine("payout_amount_out_of_range"))?;
        let receipt = KeyedReceipt {
            receipt_hash: self.receipt_hash(line.line_ref_hash),
            recipient: line.recipient.clone(),
            program: line.program.to_string(),
            delta: line.delta,
            total_after: AtomicUnits::from_raw(total_after),
        };
        state.totals.insert(
            (line.recipient.clone(), line.program.to_string()),
            total_after,
        );
        state
            .receipts
            .insert(line.line_ref_hash.to_string(), receipt.clone());
        state.history.push(receipt.clone());
        state.effects += 1;
        if std::mem::take(&mut state.lose_next_response) {
            return Err(RailRefusal::Retry("lost_response"));
        }
        Ok(committed(&receipt))
    }

    async fn find_receipt(
        &self,
        line: &RailLine<'_>,
        exclude: &[String],
    ) -> Result<ReceiptLookup, RailRefusal> {
        let mut state = self.state();
        state.take_refusal()?;
        // A key reused with other content is refused here exactly as `send`
        // refuses it, so a recovery that rebuilt the line differently cannot
        // be answered with the first line's receipt.
        match state.receipts.get(line.line_ref_hash) {
            Some(receipt) if !receipt.answers(line) => {
                Err(RailRefusal::Failed("payout_line_conflict"))
            }
            Some(receipt) if !exclude.contains(&receipt.receipt_hash) => Ok(ReceiptLookup::Found {
                receipt_hash: receipt.receipt_hash.clone(),
                delta: receipt.delta,
            }),
            _ => Ok(ReceiptLookup::NotFound),
        }
    }

    async fn history(
        &self,
        recipient: &RecipientRef,
        program: &str,
    ) -> Result<Vec<RailHistoryEntry>, RailRefusal> {
        let mut state = self.state();
        state.take_refusal()?;
        Ok(state
            .history
            .iter()
            .filter(|receipt| receipt.recipient == *recipient && receipt.program == program)
            .map(|receipt| RailHistoryEntry {
                receipt_hash: receipt.receipt_hash.clone(),
                program: receipt.program.clone(),
                delta: receipt.delta,
                total_after: Some(receipt.total_after),
            })
            .collect())
    }

    async fn probe_recipient(&self, recipient: &RecipientRef) -> Result<(), RailRefusal> {
        let mut state = self.state();
        state.take_refusal()?;
        if state.unavailable.contains(recipient) {
            return Err(RailRefusal::RecipientUnavailable);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_gate_api::payout_rail::{
        PayoutRail, RailLine, RailRefusal, ReceiptLookup, RecipientRef, SendOutcome,
    };
    use trace_commons_gate_api::pipeline::{AtomicUnits, InstrumentKind};

    const LINE_A: &str = "sha256:00000000000000000000000000000000000000000000000000000000000000aa";
    const LINE_B: &str = "sha256:00000000000000000000000000000000000000000000000000000000000000bb";

    fn recipient() -> RecipientRef {
        RecipientRef::new("synthetic-recipient-1").unwrap()
    }

    fn line<'a>(
        line_ref: &'a str,
        recipient: &'a RecipientRef,
        program: &'a str,
        target: u128,
        confirmed: u128,
    ) -> RailLine<'a> {
        RailLine::new(
            line_ref,
            recipient,
            program,
            AtomicUnits::from_raw(target),
            AtomicUnits::from_raw(confirmed),
        )
        .unwrap()
    }

    #[test]
    fn the_keyed_mock_is_an_eighteen_decimal_erc20_rail_that_is_not_qualified() {
        let rail = MockKeyedPayoutRail::new();
        let descriptor = rail.descriptor();
        descriptor.validate().unwrap();
        assert_eq!(descriptor.rail_id.as_str(), MOCK_KEYED_RAIL_ID);
        assert_eq!(descriptor.instrument.kind, InstrumentKind::Erc20);
        assert_eq!(descriptor.instrument.decimals, 18);
        assert_eq!(descriptor.wire_max, None);
        assert!(!rail.production_qualified());
        let link = uuid::Uuid::from_u128(7);
        assert_eq!(rail.program_for_link(link), rail.program_for_link(link));
        assert_ne!(
            rail.program_for_link(link),
            rail.program_for_link(uuid::Uuid::from_u128(8))
        );
        assert_eq!(
            MockKeyedPayoutRail::rate().price(1_000_000),
            Ok(10u128.pow(18))
        );
    }

    #[tokio::test]
    async fn keyed_mock_repeated_send_has_one_effect() {
        let rail = MockKeyedPayoutRail::new();
        let recipient = recipient();
        let program = rail.program_for_link(uuid::Uuid::from_u128(1));
        let first_line = line(LINE_A, &recipient, &program, 500, 0);

        let first = rail.send(&first_line).await.unwrap();
        let SendOutcome::Committed {
            receipt_hash,
            delta,
            total_after,
        } = first.clone()
        else {
            panic!("expected a commit, got {first:?}");
        };
        assert!(receipt_hash.starts_with("sha256:"));
        assert_eq!(delta, AtomicUnits::from_raw(500));
        assert_eq!(total_after, Some(AtomicUnits::from_raw(500)));

        // The same line again, even rebuilt: the first outcome, no effect.
        let again = rail.send(&first_line).await.unwrap();
        assert_eq!(again, first);
        let rebuilt = line(LINE_A, &recipient, &program, 500, 0);
        assert_eq!(rail.send(&rebuilt).await.unwrap(), first);
        assert_eq!(rail.effects(), 1);
        assert_eq!(
            rail.read_total(&recipient, &program).await.unwrap(),
            AtomicUnits::from_raw(500)
        );

        // The same key with other content is refused, never a second effect.
        let conflicting = line(LINE_A, &recipient, &program, 900, 0);
        assert_eq!(
            rail.send(&conflicting).await,
            Err(RailRefusal::Failed("payout_line_conflict"))
        );
        assert_eq!(rail.effects(), 1);

        // A different line is a different effect; a zero delta sends nothing.
        let second = line(LINE_B, &recipient, &program, 800, 500);
        assert!(matches!(
            rail.send(&second).await.unwrap(),
            SendOutcome::Committed { .. }
        ));
        assert_eq!(rail.effects(), 2);
        let settled = line(
            "sha256:00000000000000000000000000000000000000000000000000000000000000cc",
            &recipient,
            &program,
            800,
            800,
        );
        assert_eq!(
            rail.send(&settled).await.unwrap(),
            SendOutcome::AlreadyAtOrAbove {
                total: AtomicUnits::from_raw(800)
            }
        );
        assert_eq!(rail.effects(), 2);
        assert_eq!(rail.history(&recipient, &program).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn keyed_mock_lost_response_is_found_by_receipt_lookup() {
        let rail = MockKeyedPayoutRail::new();
        let recipient = recipient();
        let program = rail.program_for_link(uuid::Uuid::from_u128(2));
        let pending = line(LINE_A, &recipient, &program, 250, 0);

        assert_eq!(
            rail.find_receipt(&pending, &[]).await.unwrap(),
            ReceiptLookup::NotFound
        );

        rail.lose_next_response();
        assert_eq!(
            rail.send(&pending).await,
            Err(RailRefusal::Retry("lost_response"))
        );
        // The effect happened even though the caller never saw it.
        assert_eq!(rail.effects(), 1);

        let found = rail.find_receipt(&pending, &[]).await.unwrap();
        let ReceiptLookup::Found {
            receipt_hash,
            delta,
        } = found
        else {
            panic!("the lost send must be found");
        };
        assert_eq!(delta, AtomicUnits::from_raw(250));
        // A receipt already claimed by another line does not answer this one.
        assert_eq!(
            rail.find_receipt(&pending, std::slice::from_ref(&receipt_hash))
                .await
                .unwrap(),
            ReceiptLookup::NotFound
        );
        // Resending after the loss lands nothing new and returns the receipt.
        let resent = rail.send(&pending).await.unwrap();
        assert_eq!(
            resent,
            SendOutcome::Committed {
                receipt_hash,
                delta: AtomicUnits::from_raw(250),
                total_after: Some(AtomicUnits::from_raw(250)),
            }
        );
        assert_eq!(rail.effects(), 1);
    }

    #[tokio::test]
    async fn keyed_mock_carries_amounts_above_u64() {
        let rail = MockKeyedPayoutRail::new();
        let recipient = recipient();
        let program = rail.program_for_link(uuid::Uuid::from_u128(3));
        let big = 10u128.pow(30);
        assert!(big > u128::from(u64::MAX));
        let carried = line(LINE_A, &recipient, &program, big, 0);
        rail.send(&carried).await.unwrap();
        assert_eq!(
            rail.read_total(&recipient, &program).await.unwrap(),
            AtomicUnits::from_raw(big)
        );
        let history = rail.history(&recipient, &program).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].delta, AtomicUnits::from_raw(big));
        assert_eq!(history[0].program, program);
    }

    #[tokio::test]
    async fn the_keyed_mock_is_held_as_a_trait_object() {
        let rail: std::sync::Arc<dyn PayoutRail> = std::sync::Arc::new(MockKeyedPayoutRail::new());
        let recipient = recipient();
        let program = rail.program_for_link(uuid::Uuid::from_u128(5));
        let pending = line(LINE_A, &recipient, &program, 3, 0);
        assert!(matches!(
            rail.send(&pending).await.unwrap(),
            SendOutcome::Committed { .. }
        ));
        assert_eq!(
            rail.read_total(&recipient, &program).await.unwrap(),
            AtomicUnits::from_raw(3)
        );
    }

    #[tokio::test]
    async fn keyed_mock_refuses_a_reused_key_whatever_field_changed() {
        let rail = MockKeyedPayoutRail::new();
        let recipient = recipient();
        let other_recipient = RecipientRef::new("synthetic-recipient-2").unwrap();
        let program = rail.program_for_link(uuid::Uuid::from_u128(10));
        let other_program = rail.program_for_link(uuid::Uuid::from_u128(11));
        let original = line(LINE_A, &recipient, &program, 40, 0);
        rail.send(&original).await.unwrap();

        let conflict = RailRefusal::Failed("payout_line_conflict");
        for rebuilt in [
            line(LINE_A, &other_recipient, &program, 40, 0),
            line(LINE_A, &recipient, &other_program, 40, 0),
            line(LINE_A, &recipient, &program, 45, 0),
        ] {
            assert_eq!(rail.send(&rebuilt).await, Err(conflict), "{rebuilt:?}");
            // The lookup path answers a reused key the same way as send,
            // never with the first line's receipt.
            assert_eq!(
                rail.find_receipt(&rebuilt, &[]).await,
                Err(conflict),
                "{rebuilt:?}"
            );
        }
        assert_eq!(rail.effects(), 1);
        assert!(matches!(
            rail.find_receipt(&original, &[]).await.unwrap(),
            ReceiptLookup::Found { .. }
        ));
    }

    #[tokio::test]
    async fn keyed_mock_send_honours_the_switch_and_an_inclusive_max() {
        let rail = MockKeyedPayoutRail::new();
        let recipient = recipient();
        let program = rail.program_for_link(uuid::Uuid::from_u128(12));
        let pending = line(LINE_A, &recipient, &program, 5, 0);

        rail.set_sending_enabled(false);
        assert_eq!(
            rail.send(&pending).await,
            Err(RailRefusal::StopPass("payout_rail_sending_disabled"))
        );
        assert_eq!(rail.effects(), 0);

        rail.set_sending_enabled(true);
        rail.set_per_send_max(AtomicUnits::from_raw(5));
        // A delta equal to the maximum is within it.
        assert!(matches!(
            rail.send(&pending).await.unwrap(),
            SendOutcome::Committed { .. }
        ));
        let over = line(LINE_B, &recipient, &program, 11, 5);
        assert_eq!(
            rail.send(&over).await,
            Err(RailRefusal::HoldLine("payout_send_over_max"))
        );
        assert_eq!(rail.effects(), 1);
    }

    #[tokio::test]
    async fn keyed_mock_answers_a_zero_delta_by_the_rail_total() {
        let rail = MockKeyedPayoutRail::new();
        let recipient = recipient();
        let program = rail.program_for_link(uuid::Uuid::from_u128(13));
        // We think 5 is confirmed; the rail holds nothing. That is a
        // disagreement to hold, not "already at or above".
        let disagreeing = line(LINE_A, &recipient, &program, 5, 5);
        assert_eq!(
            rail.send(&disagreeing).await,
            Err(RailRefusal::HoldLine("payout_total_mismatch"))
        );
        rail.send(&line(LINE_B, &recipient, &program, 5, 0))
            .await
            .unwrap();
        let settled = line(
            "sha256:00000000000000000000000000000000000000000000000000000000000000cc",
            &recipient,
            &program,
            5,
            5,
        );
        assert_eq!(
            rail.send(&settled).await.unwrap(),
            SendOutcome::AlreadyAtOrAbove {
                total: AtomicUnits::from_raw(5)
            }
        );
        assert_eq!(rail.effects(), 1);
    }

    #[tokio::test]
    async fn keyed_mock_history_is_per_recipient_and_program() {
        let rail = MockKeyedPayoutRail::new();
        let first = recipient();
        let second = RecipientRef::new("synthetic-recipient-2").unwrap();
        let program = rail.program_for_link(uuid::Uuid::from_u128(14));
        rail.send(&line(LINE_A, &first, &program, 3, 0))
            .await
            .unwrap();
        rail.send(&line(LINE_B, &second, &program, 7, 0))
            .await
            .unwrap();
        let history = rail.history(&first, &program).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].delta, AtomicUnits::from_raw(3));
        let history = rail.history(&second, &program).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].delta, AtomicUnits::from_raw(7));
    }

    #[tokio::test]
    async fn every_refusal_the_keyed_mock_returns_has_a_safe_label() {
        let rail = MockKeyedPayoutRail::new();
        let recipient = recipient();
        let program = rail.program_for_link(uuid::Uuid::from_u128(15));
        let mut seen = Vec::new();

        rail.set_sending_enabled(false);
        let capacity = rail.preflight().await.unwrap();
        let stop = capacity.stop_label.expect("a stopped rail names why");
        assert!(RailRefusal::StopPass(stop).label_is_safe(), "{stop}");
        seen.push(
            rail.send(&line(LINE_A, &recipient, &program, 1, 0))
                .await
                .unwrap_err(),
        );
        rail.set_sending_enabled(true);

        rail.set_per_send_max(AtomicUnits::from_raw(1));
        seen.push(
            rail.send(&line(LINE_A, &recipient, &program, 2, 0))
                .await
                .unwrap_err(),
        );
        rail.set_per_send_max(AtomicUnits::from_raw(u128::MAX));

        rail.lose_next_response();
        seen.push(
            rail.send(&line(LINE_A, &recipient, &program, u128::MAX, 0))
                .await
                .unwrap_err(),
        );
        seen.push(
            rail.send(&line(LINE_B, &recipient, &program, 1, 0))
                .await
                .unwrap_err(),
        );
        seen.push(
            rail.send(&line(LINE_A, &recipient, &program, 1, 0))
                .await
                .unwrap_err(),
        );
        let fresh = rail.program_for_link(uuid::Uuid::from_u128(16));
        seen.push(
            rail.send(&line(LINE_B, &recipient, &fresh, 1, 1))
                .await
                .unwrap_err(),
        );
        rail.mark_recipient_unavailable(&recipient);
        seen.push(rail.probe_recipient(&recipient).await.unwrap_err());

        assert_eq!(
            seen.iter()
                .map(|refusal| refusal.label())
                .collect::<Vec<_>>(),
            [
                "payout_rail_sending_disabled",
                "payout_send_over_max",
                "lost_response",
                "payout_amount_out_of_range",
                "payout_line_conflict",
                "payout_total_mismatch",
                "payout_recipient_unavailable",
            ]
        );
        for refusal in seen {
            assert!(refusal.label_is_safe(), "{refusal:?}");
        }
    }

    #[tokio::test]
    async fn keyed_mock_refusal_knobs_change_nothing() {
        let rail = MockKeyedPayoutRail::new();
        let recipient = recipient();
        let program = rail.program_for_link(uuid::Uuid::from_u128(4));
        let pending = line(LINE_A, &recipient, &program, 10, 0);

        rail.refuse_next(RailRefusal::StopPass("payout_credential_refused"));
        assert_eq!(
            rail.send(&pending).await,
            Err(RailRefusal::StopPass("payout_credential_refused"))
        );
        assert_eq!(rail.effects(), 0);
        assert_eq!(
            rail.read_total(&recipient, &program).await.unwrap(),
            AtomicUnits::ZERO
        );

        rail.set_sending_enabled(false);
        let capacity = rail.preflight().await.unwrap();
        assert!(!capacity.sending_enabled);
        assert!(capacity.stop_label.is_some());

        rail.mark_recipient_unavailable(&recipient);
        assert_eq!(
            rail.probe_recipient(&recipient).await,
            Err(RailRefusal::RecipientUnavailable)
        );
        assert_eq!(
            rail.probe_recipient(&RecipientRef::new("other").unwrap())
                .await,
            Ok(())
        );

        rail.set_sending_enabled(true);
        rail.set_per_send_max(AtomicUnits::from_raw(5));
        assert_eq!(
            rail.send(&pending).await,
            Err(RailRefusal::HoldLine("payout_send_over_max"))
        );
        assert_eq!(rail.effects(), 0);
    }
}
