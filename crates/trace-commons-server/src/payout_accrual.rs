// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Accrual: pricing settled Trace Credit on one payout rail
//! (docs/superpowers/specs/2026-10-21-nearai-credit-payout-design.md,
//! section 4 "Accrual", "Reversals" and "Holds").
//!
//! For one tenant and one rail, accrual reads finalized settlement batches,
//! reversal rows in the credit ledger, and the principal-to-account links,
//! and writes one `trace_payout_accruals` row per settled account line (a
//! credit) and per reversal of a settled event (a reversal), each priced
//! once at the rail's rate epoch in force at the batch's creation. It is a
//! pure function of stored rows: rerunning it writes nothing new, and it
//! gives the same prices whenever it runs. It makes no network call, reads
//! no other rail's rows, and never writes the settlement tables, the ledger
//! or the NEP-141 outbox (D6).
//!
//! Order within one run: credits, then reversals (priced at the version
//! their credit used), then claims of unowned accruals by the account their
//! principal is now linked to, then holds for reversals that cannot be
//! priced.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use trace_commons_gate_api::pipeline::{AtomicUnits, InstrumentId, Microcredits};
use trace_commons_gate_api::{RailId, RailRate};
use uuid::Uuid;

use crate::db::{
    Database, PayoutAccrual, PayoutAccrualKind, PayoutAccrualWrite, PayoutRateEpoch, PayoutStore,
    settlement_key_account_id,
};
use crate::payout_rate::epoch_in_force;
use crate::trace_corpus_storage::{
    TraceCreditEventRecord, TraceCreditSettlementBatchRecord, TraceCreditSettlementBatchStatus,
    TraceCreditSettlementState,
};

/// The ledger `external_ref` prefix of an operator-raised credit reversal,
/// followed by the id of the event it reverses.
pub const CREDIT_REVERSAL_EXTERNAL_REF_PREFIX: &str = "revocation_credit_reversal:";

/// The hold opened, per account and per rail, for a reversal of a settled
/// event whose finalized line cannot be found or priced.
pub const PAYOUT_REVERSAL_UNPRICED: &str = "payout_reversal_unpriced";

/// Spec section 12, default 1 (owner may override): the NEAR wallet hold
/// label on a settlement line (`none_enrolled`, `ambiguous_no_designation`)
/// says the account has no NEAR wallet to pay. No rail built today pays a
/// NEAR wallet, so the label holds no rail, and `near_ai` accrues and pays
/// the line to its own account as usual. A future rail that pays a NEAR
/// wallet (the NEP-141 token rail) would answer true here for itself. This
/// is the only place the label is consulted.
pub fn near_wallet_hold_label_holds_rail(_rail: &RailId, _hold_label: Option<&str>) -> bool {
    false
}

/// Owner decision, 2026-10-21 ("we should pay out credit that settled
/// before had an account when they create an account"; spec section 4,
/// "Which lines accrue"): every positive finalized line accrues, whether it
/// is keyed to an account or to a bare principal. A line keyed
/// `account:{id}` is owned by that account from the start; a bare-principal
/// line accrues unowned (`None`), owed to nobody until the first account
/// its principal is linked to claims it. The owner is set once and never
/// changes.
pub fn accrual_owner_for_line(credit_account_ref: &str) -> Option<Uuid> {
    settlement_key_account_id(credit_account_ref)
}

/// Why accrual stopped. One safe label each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AccrualError {
    /// A price did not fit `u128`.
    #[error("payout_accrual_overflow")]
    Overflow,
    /// A stored row could not be read or written.
    #[error("payout_accrual_store_failed")]
    Store,
    /// A stored accrual names a rate version this rail never recorded.
    #[error("payout_rate_epoch_missing")]
    EpochMissing,
}

impl AccrualError {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overflow => "payout_accrual_overflow",
            Self::Store => "payout_accrual_store_failed",
            Self::EpochMissing => "payout_rate_epoch_missing",
        }
    }
}

/// What one accrual run did on one rail. Counts and labels only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccrualReport {
    /// The rail has no rate epoch: nothing accrued.
    pub no_rate_epoch: bool,
    pub credits_inserted: u64,
    pub reversals_inserted: u64,
    pub claimed: u64,
    pub holds_opened: u64,
    /// Rows left alone, by label.
    pub skipped: BTreeMap<&'static str, u64>,
}

/// The rows accrual reads, all in one tenant.
pub struct AccrualInputs<'a> {
    pub batches: &'a [TraceCreditSettlementBatchRecord],
    pub events: &'a [TraceCreditEventRecord],
    /// Active principal-to-account links in the tenant.
    pub principal_accounts: &'a HashMap<String, Uuid>,
    /// Accruals already stored on this rail.
    pub existing: &'a [PayoutAccrual],
    /// This rail's rate epochs.
    pub epochs: &'a [PayoutRateEpoch],
}

/// What accrual will write on one rail. `Debug` prints counts only.
#[derive(Default, PartialEq, Eq)]
pub struct AccrualPlan {
    pub writes: Vec<PayoutAccrualWrite>,
    /// `(credit_account_hash, account)`: unowned accruals the account now
    /// linked to their principal claims.
    pub claims: Vec<(String, Uuid)>,
    /// Accounts to hold on this rail, by label.
    pub holds: Vec<(Uuid, &'static str)>,
    pub skipped: BTreeMap<&'static str, u64>,
}

impl std::fmt::Debug for AccrualPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccrualPlan")
            .field("writes", &self.writes.len())
            .field("claims", &self.claims.len())
            .field("holds", &self.holds.len())
            .field("skipped", &self.skipped)
            .finish()
    }
}

impl AccrualPlan {
    fn skip(&mut self, label: &'static str) {
        *self.skipped.entry(label).or_default() += 1;
    }
}

fn reversed_event_id(event: &TraceCreditEventRecord) -> Option<Uuid> {
    let raw = event
        .external_ref
        .as_deref()?
        .trim()
        .strip_prefix(CREDIT_REVERSAL_EXTERNAL_REF_PREFIX)?;
    Uuid::parse_str(raw).ok()
}

/// A ledger amount's magnitude in microcredits, without floating point.
fn magnitude_micros(points_delta: &str) -> Option<u128> {
    let trimmed = points_delta.trim();
    let magnitude = trimmed.strip_prefix('-').unwrap_or(trimmed);
    Microcredits::from_credit_decimal(magnitude)
        .ok()
        .map(|micros| u128::from(micros.get()))
}

/// Plans one rail's accrual from stored rows. Pure: no clock, no store.
pub fn plan_rail_accruals(
    rail: &RailId,
    instrument: &InstrumentId,
    inputs: &AccrualInputs<'_>,
) -> Result<AccrualPlan, AccrualError> {
    let mut plan = AccrualPlan::default();
    if inputs.epochs.is_empty() {
        return Ok(plan);
    }
    let rate_for_version = |version: &str| -> Result<&RailRate, AccrualError> {
        inputs
            .epochs
            .iter()
            .map(|epoch| &epoch.rate)
            .find(|rate| rate.version() == version)
            .ok_or(AccrualError::EpochMissing)
    };

    // Credit accruals already stored or planned in this run, by line key:
    // (rate version, owner).
    let mut credited: HashMap<(Uuid, String), (String, Option<Uuid>)> = inputs
        .existing
        .iter()
        .filter(|row| row.kind == PayoutAccrualKind::Credit)
        .map(|row| {
            (
                (row.settlement_batch_id, row.credit_account_hash.clone()),
                (row.rate_version.clone(), row.account_id),
            )
        })
        .collect();
    let reversed: BTreeSet<Uuid> = inputs
        .existing
        .iter()
        .filter_map(|row| row.reversal_event_id)
        .collect();

    // Credits: every positive line of every finalized batch.
    let finalized = inputs
        .batches
        .iter()
        .filter(|batch| batch.status == TraceCreditSettlementBatchStatus::Finalized);
    // Source event -> (batch, line) for locating reversals.
    let mut line_of_event = HashMap::new();
    // Every event a finalized batch settled, by the batch-level list:
    // settlement's own rule (`finalized_settlement_credit_event_ids` in
    // ingest). Ledger state is not it: ingest writes ordinary rows `final`
    // whether or not a batch ever carries them.
    let mut settled_events: BTreeSet<Uuid> = BTreeSet::new();
    // credit_account_hash -> credit_account_ref, for claims.
    let mut ref_of_hash: HashMap<&str, &str> = HashMap::new();
    for batch in finalized {
        settled_events.extend(batch.source_credit_event_ids.iter().copied());
        for line in &batch.line_items {
            // Every finalized line locates its events, so a reversal of an
            // event in a line that accrued nothing is found, and then
            // skipped, rather than held as unlocatable.
            for event in &line.source_credit_event_ids {
                line_of_event.insert(*event, (batch, line));
            }
            if line.settled_credit_delta_micros <= 0 {
                continue;
            }
            ref_of_hash.insert(
                line.credit_account_hash.as_str(),
                line.credit_account_ref.as_str(),
            );
            let owner = accrual_owner_for_line(&line.credit_account_ref);
            if near_wallet_hold_label_holds_rail(rail, line.near_payout_hold_reason.as_deref()) {
                // Unreachable today (spec section 12, default 1). A rail
                // that pays a NEAR wallet holds the account and still
                // accrues: a held account accrues as usual.
                if let Some(account) = owner {
                    plan.holds.push((account, "payout_near_wallet_unresolved"));
                }
            }
            let key = (batch.settlement_batch_id, line.credit_account_hash.clone());
            if credited.contains_key(&key) {
                continue;
            }
            // Checked: the `> 0` test above makes the cast exact.
            let award = u128::try_from(line.settled_credit_delta_micros)
                .map_err(|_| AccrualError::Overflow)?;
            let epoch = epoch_in_force(inputs.epochs, batch.created_at)
                .ok_or(AccrualError::EpochMissing)?;
            let rail_units = epoch
                .rate
                .price(award)
                .map_err(|_| AccrualError::Overflow)?;
            plan.writes.push(PayoutAccrualWrite {
                rail_id: rail.clone(),
                kind: PayoutAccrualKind::Credit,
                settlement_batch_id: batch.settlement_batch_id,
                credit_account_hash: line.credit_account_hash.clone(),
                reversal_event_id: None,
                account_id: owner,
                award_units: AtomicUnits::from_raw(award),
                rail_units: AtomicUnits::from_raw(rail_units),
                rate_version: epoch.rate.version().to_owned(),
                instrument_id: instrument.clone(),
            });
            credited.insert(key, (epoch.rate.version().to_owned(), owner));
        }
    }

    // Reversals: a reversed ledger row naming the event it reverses.
    let events_by_id: HashMap<Uuid, &TraceCreditEventRecord> = inputs
        .events
        .iter()
        .map(|event| (event.credit_event_id, event))
        .collect();
    for event in inputs.events {
        if event.settlement_state != TraceCreditSettlementState::Reversed {
            continue;
        }
        let Some(source_id) = reversed_event_id(event) else {
            continue;
        };
        if reversed.contains(&event.credit_event_id) {
            continue;
        }
        let source = events_by_id.get(&source_id).copied();
        let Some((batch, line)) = line_of_event.get(&source_id).copied() else {
            // Never in a finalized batch: nothing was accrued, so nothing
            // is taken back. A settled event (one a finalized batch lists)
            // whose line cannot be found holds the account on this rail.
            if !settled_events.contains(&source_id) {
                plan.skip("payout_reversal_unsettled");
                continue;
            }
            hold_reversal(&mut plan, inputs, &event.credit_account_ref);
            continue;
        };
        let key = (batch.settlement_batch_id, line.credit_account_hash.clone());
        let Some((version, owner)) = credited.get(&key).cloned() else {
            plan.skip("payout_reversal_credit_missing");
            continue;
        };
        // The reversed event's own amount, taken from the event it reverses
        // (or, without it, the reversal row's magnitude), never more than
        // its line settled.
        let amount = source
            .map(|source| source.points_delta.as_str())
            .unwrap_or(event.points_delta.as_str());
        let Some(award) = magnitude_micros(amount) else {
            hold_line_owner(&mut plan, owner);
            continue;
        };
        let line_award =
            u128::try_from(line.settled_credit_delta_micros).map_err(|_| AccrualError::Overflow)?;
        let award = award.min(line_award);
        if award == 0 {
            plan.skip("payout_reversal_zero");
            continue;
        }
        let rail_units = rate_for_version(&version)?
            .price(award)
            .map_err(|_| AccrualError::Overflow)?;
        plan.writes.push(PayoutAccrualWrite {
            rail_id: rail.clone(),
            kind: PayoutAccrualKind::Reversal,
            settlement_batch_id: batch.settlement_batch_id,
            credit_account_hash: line.credit_account_hash.clone(),
            reversal_event_id: Some(event.credit_event_id),
            account_id: owner,
            award_units: AtomicUnits::from_raw(award),
            rail_units: AtomicUnits::from_raw(rail_units),
            rate_version: version,
            instrument_id: instrument.clone(),
        });
    }

    // Claims: an unowned accrual whose principal now has an account.
    let unowned_hashes: BTreeSet<&str> = inputs
        .existing
        .iter()
        .filter(|row| row.account_id.is_none())
        .map(|row| row.credit_account_hash.as_str())
        .chain(
            plan.writes
                .iter()
                .filter(|write| write.account_id.is_none())
                .map(|write| write.credit_account_hash.as_str()),
        )
        .collect();
    let mut claims = Vec::new();
    for hash in unowned_hashes {
        let Some(principal) = ref_of_hash.get(hash) else {
            continue;
        };
        if let Some(account) = inputs.principal_accounts.get(*principal) {
            claims.push((hash.to_owned(), *account));
        }
    }
    plan.claims = claims;
    Ok(plan)
}

fn hold_line_owner(plan: &mut AccrualPlan, owner: Option<Uuid>) {
    match owner {
        Some(account) => plan.holds.push((account, PAYOUT_REVERSAL_UNPRICED)),
        None => plan.skip("payout_reversal_unpriced_unowned"),
    }
}

/// Holds the account a reversal row's principal (or account key) names.
fn hold_reversal(plan: &mut AccrualPlan, inputs: &AccrualInputs<'_>, credit_account_ref: &str) {
    let owner = settlement_key_account_id(credit_account_ref)
        .or_else(|| inputs.principal_accounts.get(credit_account_ref).copied());
    hold_line_owner(plan, owner);
}

/// Runs accrual for one tenant on one rail and records the result.
/// `actor_ref_hash` is the system actor's hash on claim and hold audit rows.
pub async fn accrue_rail(
    payouts: &dyn PayoutStore,
    corpus: &dyn Database,
    tenant: &str,
    rail: &RailId,
    instrument: &InstrumentId,
    actor_ref_hash: &str,
) -> Result<AccrualReport, AccrualError> {
    let epochs = payouts
        .payout_rate_epochs(rail)
        .await
        .map_err(|_| AccrualError::Store)?;
    if epochs.is_empty() {
        return Ok(AccrualReport {
            no_rate_epoch: true,
            ..AccrualReport::default()
        });
    }
    let batches = corpus
        .list_trace_credit_settlement_batches(tenant)
        .await
        .map_err(|_| AccrualError::Store)?;
    let events = corpus
        .list_trace_credit_events(tenant)
        .await
        .map_err(|_| AccrualError::Store)?;
    let principals: Vec<String> = batches
        .iter()
        .flat_map(|batch| batch.line_items.iter())
        .map(|line| line.credit_account_ref.clone())
        .chain(events.iter().map(|event| event.credit_account_ref.clone()))
        .filter(|reference| settlement_key_account_id(reference).is_none())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let principal_accounts = if principals.is_empty() {
        HashMap::new()
    } else {
        corpus
            .resolve_principals_to_accounts(tenant, &principals)
            .await
            .map_err(|_| AccrualError::Store)?
    };
    let existing = payouts
        .payout_accruals(tenant, rail)
        .await
        .map_err(|_| AccrualError::Store)?;
    let plan = plan_rail_accruals(
        rail,
        instrument,
        &AccrualInputs {
            batches: &batches,
            events: &events,
            principal_accounts: &principal_accounts,
            existing: &existing,
            epochs: &epochs,
        },
    )?;

    let mut report = AccrualReport {
        skipped: plan.skipped.clone(),
        ..AccrualReport::default()
    };
    let (credits, reversals): (Vec<_>, Vec<_>) = plan
        .writes
        .into_iter()
        .partition(|write| write.kind == PayoutAccrualKind::Credit);
    report.credits_inserted = payouts
        .insert_payout_accruals(tenant, &credits)
        .await
        .map_err(|_| AccrualError::Store)?;
    report.reversals_inserted = payouts
        .insert_payout_accruals(tenant, &reversals)
        .await
        .map_err(|_| AccrualError::Store)?;
    for (hash, account) in &plan.claims {
        report.claimed += payouts
            .claim_payout_accruals(tenant, rail, hash, *account, actor_ref_hash)
            .await
            .map_err(|_| AccrualError::Store)?;
    }
    let holds: BTreeSet<(Uuid, &'static str)> = plan.holds.into_iter().collect();
    for (account, label) in holds {
        if payouts
            .open_payout_hold(tenant, account, rail, label, actor_ref_hash)
            .await
            .map_err(|_| AccrualError::Store)?
        {
            report.holds_opened += 1;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::collections::BTreeMap;

    use crate::trace_corpus_storage::{
        TraceCreditAccountSettlementLineItem, TraceCreditEventType, TraceCreditSettlementNearStatus,
    };

    fn rail() -> RailId {
        RailId::new("test_keyed").unwrap()
    }

    fn instrument() -> InstrumentId {
        InstrumentId::new("test_token").unwrap()
    }

    fn line(reference: &str, micros: i64, events: &[Uuid]) -> TraceCreditAccountSettlementLineItem {
        TraceCreditAccountSettlementLineItem {
            credit_account_ref: reference.to_owned(),
            credit_account_hash: format!("sha256:{:064x}", reference.len()),
            settled_credit_delta_micros: micros,
            source_credit_event_ids: events.to_vec(),
            source_submission_ids: Vec::new(),
            source_list_hash: "sha256:0".to_owned(),
            near_status: TraceCreditSettlementNearStatus::Disabled,
            near_outbox_id: None,
            near_payout_hold_reason: None,
        }
    }

    fn batch(
        status: TraceCreditSettlementBatchStatus,
        lines: Vec<TraceCreditAccountSettlementLineItem>,
    ) -> TraceCreditSettlementBatchRecord {
        TraceCreditSettlementBatchRecord {
            tenant_id: "tenant-a".to_owned(),
            settlement_batch_id: Uuid::new_v4(),
            policy_version: "test".to_owned(),
            status,
            reason_hash: "sha256:0".to_owned(),
            issuer_approval_evidence_hash: None,
            source_credit_event_ids: Vec::new(),
            source_submission_ids: Vec::new(),
            source_list_hash: "sha256:0".to_owned(),
            settled_credit_points: "0".to_owned(),
            settled_credit_micros: 0,
            line_items: lines,
            near_contract_id: None,
            ranking_model_version: None,
            ranking_target_use: None,
            ranking_calibration_run_id: None,
            ranking_calibration_report_hash: None,
            ranking_calibration_joined_evidence_hash: None,
            ranking_credit_events_excluded_count: 0,
            ranking_credit_events_excluded_reason_counts: BTreeMap::new(),
            actor_principal_ref: "operator".to_owned(),
            created_at: Utc.timestamp_opt(1_000, 0).unwrap(),
        }
    }

    fn event(
        id: Uuid,
        reference: &str,
        points: &str,
        state: TraceCreditSettlementState,
        reverses: Option<Uuid>,
    ) -> TraceCreditEventRecord {
        TraceCreditEventRecord {
            credit_event_id: id,
            tenant_id: "tenant-a".to_owned(),
            submission_id: Uuid::new_v4(),
            trace_id: Uuid::new_v4(),
            credit_account_ref: reference.to_owned(),
            event_type: TraceCreditEventType::Accepted,
            points_delta: points.to_owned(),
            reason: "fixture".to_owned(),
            external_ref: reverses.map(|id| format!("{CREDIT_REVERSAL_EXTERNAL_REF_PREFIX}{id}")),
            actor_principal_ref: "operator".to_owned(),
            actor_role: "admin".to_owned(),
            settlement_state: state,
            occurred_at: Utc.timestamp_opt(1_000, 0).unwrap(),
            witness_provenance_class: None,
        }
    }

    fn epochs() -> Vec<PayoutRateEpoch> {
        vec![PayoutRateEpoch {
            rate: RailRate::new("v1", 1_000_000_000_000, 1).unwrap(),
            pricing_label: None,
            observed_at: Utc.timestamp_opt(10, 0).unwrap(),
        }]
    }

    fn plan(
        batches: &[TraceCreditSettlementBatchRecord],
        events: &[TraceCreditEventRecord],
        principals: &HashMap<String, Uuid>,
    ) -> AccrualPlan {
        plan_rail_accruals(
            &rail(),
            &instrument(),
            &AccrualInputs {
                batches,
                events,
                principal_accounts: principals,
                existing: &[],
                epochs: &epochs(),
            },
        )
        .unwrap()
    }

    #[test]
    fn the_wallet_hold_label_holds_no_rail_today() {
        for label in [
            None,
            Some("none_enrolled"),
            Some("ambiguous_no_designation"),
        ] {
            assert!(!near_wallet_hold_label_holds_rail(
                &RailId::near_ai(),
                label
            ));
            assert!(!near_wallet_hold_label_holds_rail(&rail(), label));
        }
    }

    #[test]
    fn an_account_line_is_owned_and_a_principal_line_is_not() {
        let account = Uuid::new_v4();
        assert_eq!(
            accrual_owner_for_line(&format!("account:{account}")),
            Some(account)
        );
        assert_eq!(accrual_owner_for_line("principal-a"), None);
    }

    #[test]
    fn amounts_above_u64_price_exactly_on_an_18_decimal_rail() {
        let account = Uuid::new_v4();
        let batches = [batch(
            TraceCreditSettlementBatchStatus::Finalized,
            vec![line(&format!("account:{account}"), i64::MAX, &[])],
        )];
        let planned = plan(&batches, &[], &HashMap::new());
        assert_eq!(planned.writes.len(), 1);
        let write = &planned.writes[0];
        assert_eq!(write.award_units.get(), i64::MAX as u128);
        assert_eq!(write.rail_units.get(), i64::MAX as u128 * 1_000_000_000_000);
        assert!(write.rail_units.get() > u128::from(u64::MAX));
    }

    #[test]
    fn a_reversal_never_takes_back_more_than_its_line_settled() {
        let account = Uuid::new_v4();
        let credited = Uuid::new_v4();
        let batches = [batch(
            TraceCreditSettlementBatchStatus::Finalized,
            vec![line(&format!("account:{account}"), 1_000_000, &[credited])],
        )];
        let events = [
            event(
                credited,
                "principal-a",
                "5",
                TraceCreditSettlementState::Final,
                None,
            ),
            event(
                Uuid::new_v4(),
                "principal-a",
                "-5",
                TraceCreditSettlementState::Reversed,
                Some(credited),
            ),
        ];
        let planned = plan(&batches, &events, &HashMap::new());
        let reversal = planned
            .writes
            .iter()
            .find(|write| write.kind == PayoutAccrualKind::Reversal)
            .unwrap();
        assert_eq!(reversal.award_units.get(), 1_000_000);
    }

    #[test]
    fn an_unparseable_reversal_amount_holds_the_line_owner() {
        let account = Uuid::new_v4();
        let credited = Uuid::new_v4();
        let batches = [batch(
            TraceCreditSettlementBatchStatus::Finalized,
            vec![line(&format!("account:{account}"), 1_000_000, &[credited])],
        )];
        let events = [
            event(
                credited,
                "principal-a",
                "0.12345678",
                TraceCreditSettlementState::Final,
                None,
            ),
            event(
                Uuid::new_v4(),
                "principal-a",
                "-0.12345678",
                TraceCreditSettlementState::Reversed,
                Some(credited),
            ),
        ];
        let planned = plan(&batches, &events, &HashMap::new());
        assert_eq!(planned.holds, vec![(account, PAYOUT_REVERSAL_UNPRICED)]);
        assert!(
            planned
                .writes
                .iter()
                .all(|write| write.kind == PayoutAccrualKind::Credit)
        );
    }

    #[test]
    fn a_reversal_in_a_line_that_accrued_nothing_is_skipped_not_held() {
        let account = Uuid::new_v4();
        let credited = Uuid::new_v4();
        let batches = [batch(
            TraceCreditSettlementBatchStatus::Finalized,
            vec![line(&format!("account:{account}"), 0, &[credited])],
        )];
        let events = [
            event(
                credited,
                "principal-a",
                "1",
                TraceCreditSettlementState::Final,
                None,
            ),
            event(
                Uuid::new_v4(),
                "principal-a",
                "-1",
                TraceCreditSettlementState::Reversed,
                Some(credited),
            ),
        ];
        let planned = plan(&batches, &events, &HashMap::new());
        assert!(planned.writes.is_empty());
        assert!(planned.holds.is_empty());
        assert_eq!(
            planned.skipped.get("payout_reversal_credit_missing"),
            Some(&1)
        );
    }

    /// Settlement decides what settled by membership in a finalized batch
    /// (`finalized_settlement_credit_event_ids`), not by ledger state:
    /// ingest writes every ordinary ledger row `final`.
    #[test]
    fn a_reversal_of_a_final_event_in_no_finalized_batch_is_skipped_not_held() {
        let credited = Uuid::new_v4();
        let account = Uuid::new_v4();
        let principals = HashMap::from([("principal-a".to_owned(), account)]);
        let events = [
            event(
                credited,
                "principal-a",
                "2.0000",
                TraceCreditSettlementState::Final,
                None,
            ),
            event(
                Uuid::new_v4(),
                "principal-a",
                "-2.0000",
                TraceCreditSettlementState::Reversed,
                Some(credited),
            ),
        ];
        // A batch that is not finalized does not settle its events either.
        let mut dry = batch(TraceCreditSettlementBatchStatus::DryRun, Vec::new());
        dry.source_credit_event_ids = vec![credited];
        let planned = plan(&[dry], &events, &principals);
        assert!(planned.holds.is_empty(), "{planned:?}");
        assert!(planned.writes.is_empty());
        assert_eq!(planned.skipped.get("payout_reversal_unsettled"), Some(&1));
    }

    #[test]
    fn a_reversal_of_a_batched_event_in_no_line_holds_the_account() {
        let credited = Uuid::new_v4();
        let account = Uuid::new_v4();
        let principals = HashMap::from([("principal-a".to_owned(), account)]);
        let events = [
            event(
                credited,
                "principal-a",
                "2.0000",
                TraceCreditSettlementState::Final,
                None,
            ),
            event(
                Uuid::new_v4(),
                "principal-a",
                "-2.0000",
                TraceCreditSettlementState::Reversed,
                Some(credited),
            ),
        ];
        let mut settled = batch(TraceCreditSettlementBatchStatus::Finalized, Vec::new());
        settled.source_credit_event_ids = vec![credited];
        let planned = plan(&[settled], &events, &principals);
        assert_eq!(planned.holds, vec![(account, PAYOUT_REVERSAL_UNPRICED)]);
        assert!(planned.writes.is_empty());
    }

    #[test]
    fn a_plan_debug_prints_counts_only() {
        let account = Uuid::new_v4();
        let batches = [batch(
            TraceCreditSettlementBatchStatus::Finalized,
            vec![line(&format!("account:{account}"), 10, &[])],
        )];
        let rendered = format!("{:?}", plan(&batches, &[], &HashMap::new()));
        assert!(!rendered.contains(&account.to_string()), "{rendered}");
        assert!(!rendered.contains("sha256:"), "{rendered}");
    }
}
