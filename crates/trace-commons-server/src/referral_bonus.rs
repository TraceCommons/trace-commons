// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The referral bonus: pricing a referrer's share of a referee's NEAR AI
//! credit (docs/superpowers/specs/2026-10-29-referral-rewards-design.md,
//! section 4 "The bonus", and section 7).
//!
//! [`plan_referral_bonus`] is pure, like
//! [`crate::payout_accrual::plan_rail_accruals`]: given a referee's
//! attribution, the referee's accruals on the rail with their batch's
//! creation time, the bonus rows already stored for that referee, the bps
//! and the end of the earning window, it returns the bonus and reversal rows
//! to write. Rules:
//!
//! - a `credit` accrual owned by the referee whose batch was created at or
//!   after `applied_at` and before `earn_until` gives a `bonus` row,
//!   `bonus_units = floor(rail_units * bps / 10000)`, computed in `u128`
//!   without overflow; an unowned accrual counts once it is claimed;
//! - a `reversal` accrual gives a `reversal` row only when its credit (the
//!   credit of the same settlement line) produced a bonus row, priced at
//!   that bonus row's bps so it never takes back more than was given;
//! - nothing new is written once the referral is voided (an attribution,
//!   mirror or `void` row says so), and no new `bonus` row while the
//!   referee's own account is held (credit hold, closed); reversals still
//!   are, since they only take back;
//! - every row is keyed by its source accrual, and a source already priced
//!   is skipped, so a rerun writes nothing new.
//!
//! [`ReferralBonusHook`] is what the `near_ai` payout pass calls for each
//! tenant after accrual (step 1a). It reads the tenant's attributions, then
//! writes each referrer's rows in a second transaction under the referrer's
//! tenant; a crash between the two re-runs harmlessly on the next pass. It
//! also refreshes the referee's mirror row (signals, link indexes), scores
//! the code and the referrer ([`crate::referral_abuse`]), and opens the
//! referral-stream holds.
//!
//! The client is open source, so nothing it sends is evidence (owner
//! decision, 2026-10-29). Besides a shared payout org or subject, the pass
//! compares two things the server observed itself, each read under its own
//! side's tenant: the near-duplicate clusters of the referee's and the
//! referrer's traces (`referral_same_traces`, and `referral_duplicate_traces`
//! on the mirror, which counts toward the code's score), and their keyed
//! network marks (`referral_same_network`, on the mirror as evidence only,
//! never scored). A match holds; it never voids and never pays. Each of
//! these holds, and the payout org and subject holds, opens when the
//! referee first shows the match (its label is not yet on the mirror; the
//! org and subject labels are evidence only, never scored), so an operator
//! who clears it is not overruled by the next pass; a new referee with the
//! same match opens it again. It sends nothing: the referral stream's sends are
//! a later change, and this pass runs in `dry_run` only.
//!
//! Logs carry counts and labels only.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};
use trace_commons_gate_api::RailId;
use uuid::Uuid;

use crate::db::{
    PayoutAccrualKind, PayoutStore, ReferralAttributionRecord, ReferralBonusKind,
    ReferralBonusRecord, ReferralBonusStore, ReferralBonusWrite, ReferralRefereeAccrual,
};
use crate::payout_rate::PayoutMode;
use crate::referral_abuse::{
    REFERRAL_CODE_SUSPENDED, REFERRAL_DUPLICATE_TRACES, REFERRAL_OVER_CEILING,
    REFERRAL_REFERRER_SUSPENDED, REFERRAL_SAME_NETWORK, REFERRAL_SAME_PAYOUT_ORG,
    REFERRAL_SAME_PAYOUT_SUBJECT, REFERRAL_SAME_TRACES, REFERRAL_TRIPPED, RefereeFacts,
    referee_signal_labels, score_code, score_referrer, should_trip_dry_run,
};
use crate::referral_config::ReferralBonusSettings;
use crate::referral_network::network_mark_retention_days;

/// How far before the referee's account creation the trace comparison
/// looks, on both sides: a self-referrer may have uploaded the content from
/// the first account before making the second.
pub const REFERRAL_TRACE_LOOKBACK_DAYS: i64 = 30;
/// The most recent submissions per side whose clusters are compared.
pub const REFERRAL_TRACE_COMPARE_LIMIT: i64 = 500;
/// The most recently seen network marks per side that are compared.
pub const REFERRAL_NETWORK_COMPARE_LIMIT: i64 = 256;

/// The void label when the referrer's tenant or account is gone.
pub const REFERRAL_REFERRER_GONE: &str = "referral_referrer_gone";
/// The void label propagated to a referee's attribution when its code or
/// referral was voided in the referrer's tenant.
pub const REFERRAL_VOIDED_BY_OPERATOR: &str = "referral_voided_by_operator";

/// The pseudo-rail whose kill switch records that the referral stream
/// tripped itself to `dry_run` (section 7). Its `trace_payout_control` rows
/// are written with the config writer role like a rail's halt; no pass runs
/// on it.
pub const NEAR_AI_REFERRAL_CONTROL_RAIL: &str = "near_ai.referral";

pub fn near_ai_referral_control_rail() -> RailId {
    RailId::new(NEAR_AI_REFERRAL_CONTROL_RAIL).expect("a safe rail id")
}

/// The pass actor's hash on audit and hold rows.
pub fn referral_bonus_actor(rail: &RailId) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(format!("referral-bonus-pass-v1:{rail}").as_bytes())
    )
}

/// `earn_until = referee_account_created_at + earn_days`, read when priced,
/// so raising the setting extends every live referral.
pub fn earn_until(referee_account_created_at: DateTime<Utc>, earn_days: u32) -> DateTime<Utc> {
    referee_account_created_at + Duration::days(i64::from(earn_days))
}

/// `floor(base * bps / 10000)` in `u128`, exact for every `base` and every
/// `bps <= 10000`: `base = q * 10000 + r`, so the product splits into
/// `q * bps` (at most `base`) and `r * bps / 10000` (under 10000).
pub fn bonus_units(base: u128, bps: u32) -> u128 {
    let bps = u128::from(bps.min(10_000));
    (base / 10_000) * bps + (base % 10_000) * bps / 10_000
}

/// The rows planning reads, for one referee.
pub struct BonusInputs<'a> {
    pub attribution: &'a ReferralAttributionRecord,
    /// The referee's accruals on the rail (any owner; the plan filters).
    pub accruals: &'a [ReferralRefereeAccrual],
    /// The bonus rows already stored for this referee on the rail.
    pub existing: &'a [ReferralBonusRecord],
    pub bps: u32,
    pub earn_until: DateTime<Utc>,
    /// The referee's own account is held (credit hold, closed).
    pub referee_held: bool,
    /// The referrer's mirror row is voided.
    pub mirror_voided: bool,
}

/// What planning will write. `Debug` prints counts only.
#[derive(Default, PartialEq, Eq)]
pub struct BonusPlan {
    pub writes: Vec<ReferralBonusWrite>,
    /// The referral is void: nothing was planned.
    pub voided: bool,
    pub skipped: BTreeMap<&'static str, u64>,
}

impl std::fmt::Debug for BonusPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BonusPlan")
            .field("writes", &self.writes.len())
            .field("voided", &self.voided)
            .field("skipped", &self.skipped)
            .finish()
    }
}

impl BonusPlan {
    fn skip(&mut self, label: &'static str) {
        *self.skipped.entry(label).or_default() += 1;
    }
}

/// A settlement line: `(settlement_batch_id, credit_account_hash)`.
type LineKey = (Uuid, String);

/// Plans one referee's bonus rows on `rail`. Pure: no clock, no store.
pub fn plan_referral_bonus(rail: &RailId, inputs: &BonusInputs<'_>) -> BonusPlan {
    let mut plan = BonusPlan::default();
    let attribution = inputs.attribution;
    if attribution.voided
        || inputs.mirror_voided
        || inputs
            .existing
            .iter()
            .any(|row| row.kind == ReferralBonusKind::Void)
    {
        plan.voided = true;
        return plan;
    }
    let referee = attribution.account_id;
    let priced: BTreeSet<Uuid> = inputs
        .existing
        .iter()
        .filter_map(|row| row.source_accrual_id)
        .collect();
    let accrual_by_id: HashMap<Uuid, &ReferralRefereeAccrual> = inputs
        .accruals
        .iter()
        .map(|accrual| (accrual.accrual_id, accrual))
        .collect();
    // The credit accrual of each line, and the bonus row it produced (bps,
    // bonus units), stored or planned.
    let mut credit_of_line: HashMap<LineKey, Uuid> = HashMap::new();
    for accrual in inputs.accruals {
        if accrual.kind == PayoutAccrualKind::Credit {
            credit_of_line.insert(
                (
                    accrual.settlement_batch_id,
                    accrual.credit_account_hash.clone(),
                ),
                accrual.accrual_id,
            );
        }
    }
    let mut bonus_of_credit: HashMap<Uuid, (u32, u128)> = inputs
        .existing
        .iter()
        .filter(|row| row.kind == ReferralBonusKind::Bonus)
        .filter_map(|row| Some((row.source_accrual_id?, (row.bps, row.bonus_units))))
        .collect();
    // What each credit's bonus has had taken back by stored reversal rows.
    let mut taken_back: HashMap<Uuid, u128> = HashMap::new();
    for row in inputs
        .existing
        .iter()
        .filter(|row| row.kind == ReferralBonusKind::Reversal)
    {
        let Some(reversal) = row.source_accrual_id.and_then(|id| accrual_by_id.get(&id)) else {
            continue;
        };
        let key = (
            reversal.settlement_batch_id,
            reversal.credit_account_hash.clone(),
        );
        if let Some(credit) = credit_of_line.get(&key) {
            let taken = taken_back.entry(*credit).or_default();
            *taken = taken.saturating_add(row.bonus_units);
        }
    }

    let mut ordered: Vec<&ReferralRefereeAccrual> = inputs.accruals.iter().collect();
    ordered.sort_by_key(|accrual| (accrual.batch_created_at, accrual.accrual_id));

    // Credits first, so a reversal later in the same run finds its bonus.
    for accrual in ordered
        .iter()
        .filter(|accrual| accrual.kind == PayoutAccrualKind::Credit)
    {
        match accrual.account_id {
            Some(owner) if owner == referee => {}
            Some(_) => continue,
            None => {
                plan.skip("referral_accrual_unowned");
                continue;
            }
        }
        if priced.contains(&accrual.accrual_id) {
            continue;
        }
        if accrual.batch_created_at < attribution.applied_at
            || accrual.batch_created_at >= inputs.earn_until
        {
            plan.skip("referral_outside_window");
            continue;
        }
        if inputs.referee_held {
            plan.skip("referral_referee_held");
            continue;
        }
        if accrual.rail_units == 0 {
            plan.skip("referral_base_zero");
            continue;
        }
        let units = bonus_units(accrual.rail_units, inputs.bps);
        bonus_of_credit.insert(accrual.accrual_id, (inputs.bps, units));
        plan.writes.push(ReferralBonusWrite {
            rail_id: rail.clone(),
            account_id: attribution.referrer_account_id,
            referee_ref_hash: attribution.referee_ref_hash.clone(),
            kind: ReferralBonusKind::Bonus,
            source_accrual_id: Some(accrual.accrual_id),
            bps: inputs.bps,
            base_units: accrual.rail_units,
            bonus_units: units,
            rate_version: accrual.rate_version.clone(),
            instrument_id: accrual.instrument_id.clone(),
        });
    }

    for accrual in ordered
        .iter()
        .filter(|accrual| accrual.kind == PayoutAccrualKind::Reversal)
    {
        if accrual.account_id != Some(referee) {
            continue;
        }
        if priced.contains(&accrual.accrual_id) {
            continue;
        }
        let key = (
            accrual.settlement_batch_id,
            accrual.credit_account_hash.clone(),
        );
        let Some((bps, credit_bonus)) = credit_of_line
            .get(&key)
            .and_then(|credit| bonus_of_credit.get(credit).copied())
        else {
            plan.skip("referral_reversal_without_bonus");
            continue;
        };
        if accrual.rail_units == 0 {
            plan.skip("referral_base_zero");
            continue;
        }
        let credit = credit_of_line[&key];
        let taken = taken_back.entry(credit).or_default();
        // Floors sum to at most the floor of the sum, so a line's reversals
        // never exceed its bonus; the cap only guards stored oddities.
        let units = bonus_units(accrual.rail_units, bps).min(credit_bonus.saturating_sub(*taken));
        *taken = taken.saturating_add(units);
        plan.writes.push(ReferralBonusWrite {
            rail_id: rail.clone(),
            account_id: attribution.referrer_account_id,
            referee_ref_hash: attribution.referee_ref_hash.clone(),
            kind: ReferralBonusKind::Reversal,
            source_accrual_id: Some(accrual.accrual_id),
            bps,
            base_units: accrual.rail_units,
            bonus_units: units,
            rate_version: accrual.rate_version.clone(),
            instrument_id: accrual.instrument_id.clone(),
        });
    }
    plan
}

/// What one tenant's referral work did. Counts and labels only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferralBonusReport {
    pub referees: u64,
    pub bonus_rows: u64,
    pub reversal_rows: u64,
    pub voided: u64,
    pub suspended: u64,
    pub tripped: bool,
    pub held: BTreeMap<&'static str, u64>,
    pub skipped: BTreeMap<&'static str, u64>,
}

impl ReferralBonusReport {
    fn skip(&mut self, label: &'static str) {
        *self.skipped.entry(label).or_default() += 1;
    }
}

/// Why one referee's work stopped. The pass goes on with the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Store,
}

type Step<T> = Result<T, Stop>;

fn store<T>(result: Result<T, crate::error::DatabaseError>) -> Step<T> {
    result.map_err(|_| Stop::Store)
}

/// The referral work the `near_ai` payout pass runs after accrual.
pub struct ReferralBonusHook {
    store: Arc<dyn ReferralBonusStore>,
    payouts: Arc<dyn PayoutStore>,
    settings: ReferralBonusSettings,
    rail: RailId,
    actor: String,
    /// The referral stream's trip, as last read; `None` before a pass.
    tripped: Mutex<Option<bool>>,
}

impl std::fmt::Debug for ReferralBonusHook {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReferralBonusHook")
            .field("rail", &self.rail)
            .field("mode", &self.settings.mode)
            .finish_non_exhaustive()
    }
}

impl ReferralBonusHook {
    /// The hook for the `near_ai` rail.
    pub fn new(
        store: Arc<dyn ReferralBonusStore>,
        payouts: Arc<dyn PayoutStore>,
        settings: ReferralBonusSettings,
    ) -> Self {
        let rail = RailId::near_ai();
        let actor = referral_bonus_actor(&rail);
        Self {
            store,
            payouts,
            settings,
            rail,
            actor,
            tripped: Mutex::new(None),
        }
    }

    pub fn mode(&self) -> PayoutMode {
        self.settings.mode
    }

    pub fn settings(&self) -> &ReferralBonusSettings {
        &self.settings
    }

    pub fn rail_id(&self) -> &RailId {
        &self.rail
    }

    pub fn store(&self) -> &Arc<dyn ReferralBonusStore> {
        &self.store
    }

    pub fn payouts(&self) -> &Arc<dyn PayoutStore> {
        &self.payouts
    }

    pub fn actor(&self) -> &str {
        &self.actor
    }

    /// Whether the referral stream tripped itself to `dry_run`, as last
    /// read. Readiness reports `referral_tripped` while it is.
    pub fn tripped(&self) -> Option<bool> {
        *self
            .tripped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Records an operator's halt or resume of the referral stream, which
    /// the control route has just written, so readiness reports it at once
    /// rather than at the next pass.
    pub fn record_operator_control(&self, halted: bool) {
        self.record_tripped(halted);
    }

    fn record_tripped(&self, tripped: bool) {
        *self
            .tripped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(tripped);
    }

    /// One tenant: every referee attribution in it. Never fails the payout
    /// pass; a store fault ends that referee's work with a label.
    pub async fn run_tenant(&self, tenant: &str, rail: &RailId) -> ReferralBonusReport {
        let mut report = ReferralBonusReport::default();
        if self.settings.mode == PayoutMode::Disabled || rail != &self.rail {
            return report;
        }
        if let Ok(control) = self
            .payouts
            .payout_control(&near_ai_referral_control_rail())
            .await
        {
            self.record_tripped(control.halted);
        }
        // Retention: network marks outlive the earning period by 30 days.
        // Reads also ignore older marks, so a missed prune never matches.
        if self
            .store
            .prune_referral_network_marks(tenant, self.mark_retention_days())
            .await
            .is_err()
        {
            report.skip("referral_mark_prune_failed");
        }
        let attributions = match self.store.referral_attributions(tenant).await {
            Ok(attributions) => attributions,
            Err(_) => {
                report.skip("referral_store_failed");
                self.log(&report);
                return report;
            }
        };
        for attribution in attributions {
            report.referees += 1;
            if self
                .run_referee(tenant, &attribution, &mut report)
                .await
                .is_err()
            {
                report.skip("referral_store_failed");
            }
        }
        if report.referees > 0 {
            self.log(&report);
        }
        report
    }

    fn mark_retention_days(&self) -> u32 {
        network_mark_retention_days(self.settings.earn_days)
    }

    /// Whether the referee's and the referrer's traces share a
    /// near-duplicate cluster. Each side is read in its own tenant; the
    /// cluster ids come from the cross-tenant simhash scan, so they compare.
    async fn shares_traces(
        &self,
        tenant: &str,
        attribution: &ReferralAttributionRecord,
    ) -> Step<bool> {
        let since =
            attribution.referee_account_created_at - Duration::days(REFERRAL_TRACE_LOOKBACK_DAYS);
        let theirs: BTreeSet<Uuid> = store(
            self.store
                .referral_trace_clusters(
                    tenant,
                    attribution.account_id,
                    since,
                    REFERRAL_TRACE_COMPARE_LIMIT,
                )
                .await,
        )?
        .into_iter()
        .collect();
        if theirs.is_empty() {
            return Ok(false);
        }
        let mine = store(
            self.store
                .referral_trace_clusters(
                    &attribution.referrer_tenant_id,
                    attribution.referrer_account_id,
                    since,
                    REFERRAL_TRACE_COMPARE_LIMIT,
                )
                .await,
        )?;
        Ok(mine.iter().any(|cluster| theirs.contains(cluster)))
    }

    /// Whether the referee and the referrer share a keyed network mark.
    async fn shares_network(
        &self,
        tenant: &str,
        attribution: &ReferralAttributionRecord,
    ) -> Step<bool> {
        let retention = self.mark_retention_days();
        let theirs: BTreeSet<String> = store(
            self.store
                .referral_network_marks(
                    tenant,
                    attribution.account_id,
                    retention,
                    REFERRAL_NETWORK_COMPARE_LIMIT,
                )
                .await,
        )?
        .into_iter()
        .collect();
        if theirs.is_empty() {
            return Ok(false);
        }
        let mine = store(
            self.store
                .referral_network_marks(
                    &attribution.referrer_tenant_id,
                    attribution.referrer_account_id,
                    retention,
                    REFERRAL_NETWORK_COMPARE_LIMIT,
                )
                .await,
        )?;
        Ok(mine.iter().any(|mark| theirs.contains(mark)))
    }

    fn log(&self, report: &ReferralBonusReport) {
        tracing::info!(
            rail = %self.rail,
            mode = self.settings.mode.as_label(),
            referees = report.referees,
            bonus_rows = report.bonus_rows,
            reversal_rows = report.reversal_rows,
            voided = report.voided,
            suspended = report.suspended,
            tripped = report.tripped,
            held = ?report.held,
            skipped = ?report.skipped,
            "referral bonus"
        );
    }

    async fn run_referee(
        &self,
        tenant: &str,
        attribution: &ReferralAttributionRecord,
        report: &mut ReferralBonusReport,
    ) -> Step<()> {
        let referrer_tenant = attribution.referrer_tenant_id.as_str();
        let referrer = attribution.referrer_account_id;
        let view = store(
            self.store
                .referral_referrer_view(
                    referrer_tenant,
                    referrer,
                    &attribution.referee_ref_hash,
                    &self.rail,
                )
                .await,
        )?;
        if !view.referrer_exists || view.mirror.is_none() {
            // The referrer's tenant or account is gone (or its mirror row,
            // which only goes with the tenant): void the attribution.
            if !attribution.voided
                && store(
                    self.store
                        .void_referral_attribution(
                            tenant,
                            attribution.account_id,
                            REFERRAL_REFERRER_GONE,
                            &self.actor,
                        )
                        .await,
                )?
            {
                report.voided += 1;
            }
            return Ok(());
        }
        let mirror_voided = view.mirror.as_ref().is_some_and(|mirror| mirror.voided);
        if attribution.voided && !mirror_voided {
            // Voided in the referee's tenant (an operator void whose second
            // transaction did not commit): finish it in the referrer's.
            store(
                self.store
                    .void_referral_referee(
                        referrer_tenant,
                        &attribution.referee_ref_hash,
                        REFERRAL_VOIDED_BY_OPERATOR,
                        &self.rail,
                        &self.actor,
                    )
                    .await,
            )?;
            report.voided += 1;
            return Ok(());
        }
        if !attribution.voided && mirror_voided {
            // Voided in the referrer's tenant (a code or referral void):
            // carry it to the attribution.
            if store(
                self.store
                    .void_referral_attribution(
                        tenant,
                        attribution.account_id,
                        REFERRAL_VOIDED_BY_OPERATOR,
                        &self.actor,
                    )
                    .await,
            )? {
                report.voided += 1;
            }
            return Ok(());
        }
        if attribution.voided {
            return Ok(());
        }

        // Price.
        let referee = attribution.account_id;
        let standing = store(self.payouts.payout_account_standing(tenant, referee).await)?;
        let accruals = store(
            self.store
                .referral_referee_accruals(tenant, &self.rail, referee)
                .await,
        )?;
        let plan = plan_referral_bonus(
            &self.rail,
            &BonusInputs {
                attribution,
                accruals: &accruals,
                existing: &view.bonus_rows,
                bps: self.settings.bps,
                earn_until: earn_until(
                    attribution.referee_account_created_at,
                    self.settings.earn_days,
                ),
                referee_held: standing.hold_label().is_some(),
                mirror_voided,
            },
        );
        for (label, count) in &plan.skipped {
            *report.skipped.entry(label).or_default() += count;
        }
        if !plan.writes.is_empty() {
            store(
                self.store
                    .insert_referral_bonus_rows(referrer_tenant, &plan.writes)
                    .await,
            )?;
            for write in &plan.writes {
                match write.kind {
                    ReferralBonusKind::Bonus => report.bonus_rows += 1,
                    ReferralBonusKind::Reversal => report.reversal_rows += 1,
                    ReferralBonusKind::Void => {}
                }
            }
        }

        // Signals.
        let now = store(self.payouts.payout_database_now().await)?;
        let facts = store(
            self.store
                .referral_referee_facts(tenant, &self.rail, referee)
                .await,
        )?;
        // Server-observed signals against the referrer.
        let same_traces = self.shares_traces(tenant, attribution).await?;
        let same_network = self.shares_network(tenant, attribution).await?;
        let mut labels: Vec<String> = referee_signal_labels(
            &RefereeFacts {
                submissions: facts.submissions,
                refused: facts.refused,
                credit_hold_or_reversal: standing.credit_held || facts.reversed,
                advance_forfeited: false,
                duplicate_traces: same_traces,
                settled_credit: facts.credited,
                account_created_at: attribution.referee_account_created_at,
            },
            now,
            &self.settings.abuse,
        )
        .into_iter()
        .map(str::to_owned)
        .collect();
        // Same payout org or subject as the referrer, from the two live
        // links' blind indexes.
        let referee_link = store(
            self.payouts
                .payout_live_link(tenant, referee, &self.rail)
                .await,
        )?;
        let referrer_link = store(
            self.payouts
                .payout_live_link(referrer_tenant, referrer, &self.rail)
                .await,
        )?;
        let (same_org, same_subject) = match (&referrer_link, &referee_link) {
            (Some(mine), Some(theirs)) => (
                mine.recipient.recipient_index == theirs.recipient.recipient_index,
                mine.recipient.subject_index == theirs.recipient.subject_index,
            ),
            _ => (false, false),
        };
        // Evidence for an operator; `score_rows` never counts these.
        for (matched, label) in [
            (same_org, REFERRAL_SAME_PAYOUT_ORG),
            (same_subject, REFERRAL_SAME_PAYOUT_SUBJECT),
            (same_network, REFERRAL_SAME_NETWORK),
        ] {
            if matched {
                labels.push(label.to_owned());
            }
        }
        // Each server-observed hold opens when this referee first shows the
        // match (the mirror does not carry its label yet), before the
        // mirror is refreshed, so a failed refresh retries the hold too, and
        // an operator's clear stands until another referee matches.
        let shown = |label: &str| {
            view.mirror
                .as_ref()
                .is_some_and(|mirror| mirror.signal_labels.iter().any(|held| held == label))
        };
        for (matched, evidence, hold) in [
            (same_org, REFERRAL_SAME_PAYOUT_ORG, REFERRAL_SAME_PAYOUT_ORG),
            (
                same_subject,
                REFERRAL_SAME_PAYOUT_SUBJECT,
                REFERRAL_SAME_PAYOUT_SUBJECT,
            ),
            (same_traces, REFERRAL_DUPLICATE_TRACES, REFERRAL_SAME_TRACES),
            (same_network, REFERRAL_SAME_NETWORK, REFERRAL_SAME_NETWORK),
        ] {
            if matched && !shown(evidence) {
                self.hold(referrer_tenant, referrer, hold, report).await?;
            }
        }
        let (recipient_index, subject_index) = match &referee_link {
            Some(link) => (
                Some(link.recipient.recipient_index.as_str()),
                Some(link.recipient.subject_index.as_str()),
            ),
            None => (None, None),
        };
        store(
            self.store
                .refresh_referral_referee_signals(
                    referrer_tenant,
                    &attribution.referee_ref_hash,
                    &labels,
                    recipient_index,
                    subject_index,
                )
                .await,
        )?;

        self.score(
            referrer_tenant,
            referrer,
            &attribution.code_index,
            now,
            report,
        )
        .await?;
        self.check_ceiling(referrer_tenant, referrer, report).await
    }

    /// Scores the referee's code and the referrer, suspends a tripped code,
    /// holds the referrer's referral stream, and trips the stream when too
    /// many codes were suspended in a day.
    async fn score(
        &self,
        tenant: &str,
        referrer: Uuid,
        code_index: &str,
        now: DateTime<Utc>,
        report: &mut ReferralBonusReport,
    ) -> Step<()> {
        let rows = store(self.store.referral_mirror_rows(tenant, referrer).await)?;
        let codes = store(self.store.referral_codes(tenant).await)?;
        if let Some(code) = codes.iter().find(|code| code.code_index == code_index)
            && !code.voided
            && code.suspended_at.is_none()
        {
            let score = score_code(code_index, &rows, &self.settings.abuse, &code.exempt_labels);
            if score.is_tripped() {
                let labels: Vec<String> = score
                    .tripped
                    .iter()
                    .map(|label| (*label).to_owned())
                    .collect();
                if store(
                    self.store
                        .suspend_referral_code(tenant, code_index, &labels, &self.actor)
                        .await,
                )? {
                    report.suspended += 1;
                    self.maybe_trip(now, report).await?;
                }
            }
        }
        let codes = store(self.store.referral_codes(tenant).await)?;
        let any_suspended = codes
            .iter()
            .any(|code| code.account_id == referrer && !code.voided && code.suspended_at.is_some());
        if any_suspended {
            self.hold(tenant, referrer, REFERRAL_CODE_SUSPENDED, report)
                .await?;
        }
        if score_referrer(&rows, &self.settings.abuse).is_tripped() {
            self.hold(tenant, referrer, REFERRAL_REFERRER_SUSPENDED, report)
                .await?;
        }
        Ok(())
    }

    async fn maybe_trip(&self, now: DateTime<Utc>, report: &mut ReferralBonusReport) -> Step<()> {
        let since = now - Duration::hours(24);
        let recent = store(self.store.referral_codes_suspended_since(since).await)?;
        if !should_trip_dry_run(recent, &self.settings.abuse) {
            return Ok(());
        }
        let control_rail = near_ai_referral_control_rail();
        if store(self.payouts.payout_control(&control_rail).await)?.halted {
            self.record_tripped(true);
            return Ok(());
        }
        store(
            self.payouts
                .set_payout_control(&control_rail, true, Some(REFERRAL_TRIPPED), &self.actor)
                .await,
        )?;
        self.record_tripped(true);
        report.tripped = true;
        tracing::warn!(
            rail = %self.rail,
            label = REFERRAL_TRIPPED,
            "referral stream switched itself to dry_run"
        );
        Ok(())
    }

    /// Opens `referral_over_ceiling` while the referrer's referral owed is
    /// over the ceiling, and lifts it once it fits again.
    async fn check_ceiling(
        &self,
        tenant: &str,
        referrer: Uuid,
        report: &mut ReferralBonusReport,
    ) -> Step<()> {
        let owed = store(
            self.store
                .referral_bonus_owed(tenant, referrer, &self.rail)
                .await,
        )?;
        if owed.owed > self.settings.account_ceiling {
            return self
                .hold(tenant, referrer, REFERRAL_OVER_CEILING, report)
                .await;
        }
        let open = store(
            self.payouts
                .open_payout_hold_labels(tenant, referrer, &self.rail)
                .await,
        )?;
        if open.iter().any(|label| label == REFERRAL_OVER_CEILING) {
            store(
                self.payouts
                    .clear_payout_hold(
                        tenant,
                        referrer,
                        &self.rail,
                        REFERRAL_OVER_CEILING,
                        &self.actor,
                    )
                    .await,
            )?;
        }
        Ok(())
    }

    async fn hold(
        &self,
        tenant: &str,
        account: Uuid,
        label: &'static str,
        report: &mut ReferralBonusReport,
    ) -> Step<()> {
        if store(
            self.payouts
                .open_payout_hold(tenant, account, &self.rail, label, &self.actor)
                .await,
        )? {
            *report.held.entry(label).or_default() += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use trace_commons_gate_api::pipeline::InstrumentId;

    use super::*;

    fn at(day: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + Duration::days(day)
    }

    const REFEREE: Uuid = Uuid::from_u128(7);
    const REFERRER: Uuid = Uuid::from_u128(9);

    fn attribution() -> ReferralAttributionRecord {
        ReferralAttributionRecord {
            account_id: REFEREE,
            code_index: format!("sha256:{}", "c".repeat(64)),
            referrer_tenant_id: "nearai-referrer".into(),
            referrer_account_id: REFERRER,
            referee_ref_hash: format!("sha256:{}", "e".repeat(64)),
            referee_account_created_at: at(0),
            applied_at: at(1),
            voided: false,
        }
    }

    fn accrual(
        n: u128,
        kind: PayoutAccrualKind,
        line: u128,
        owner: Option<Uuid>,
        units: u128,
        batch_day: i64,
    ) -> ReferralRefereeAccrual {
        ReferralRefereeAccrual {
            accrual_id: Uuid::from_u128(1_000 + n),
            kind,
            settlement_batch_id: Uuid::from_u128(line),
            credit_account_hash: format!("sha256:{line:064x}"),
            account_id: owner,
            rail_units: units,
            rate_version: "v1".into(),
            instrument_id: InstrumentId::new("near_ai_credit_usd").unwrap(),
            batch_created_at: at(batch_day),
        }
    }

    fn credit(n: u128, units: u128, day: i64) -> ReferralRefereeAccrual {
        accrual(n, PayoutAccrualKind::Credit, n, Some(REFEREE), units, day)
    }

    fn reversal(n: u128, of_line: u128, units: u128, day: i64) -> ReferralRefereeAccrual {
        accrual(
            n,
            PayoutAccrualKind::Reversal,
            of_line,
            Some(REFEREE),
            units,
            day,
        )
    }

    struct Case {
        attribution: ReferralAttributionRecord,
        accruals: Vec<ReferralRefereeAccrual>,
        existing: Vec<ReferralBonusRecord>,
        bps: u32,
        earn_days: u32,
        referee_held: bool,
        mirror_voided: bool,
    }

    impl Case {
        fn new(accruals: Vec<ReferralRefereeAccrual>) -> Self {
            Self {
                attribution: attribution(),
                accruals,
                existing: Vec::new(),
                bps: 1_000,
                earn_days: 92,
                referee_held: false,
                mirror_voided: false,
            }
        }

        fn plan(&self) -> BonusPlan {
            plan_referral_bonus(
                &RailId::near_ai(),
                &BonusInputs {
                    attribution: &self.attribution,
                    accruals: &self.accruals,
                    existing: &self.existing,
                    bps: self.bps,
                    earn_until: earn_until(
                        self.attribution.referee_account_created_at,
                        self.earn_days,
                    ),
                    referee_held: self.referee_held,
                    mirror_voided: self.mirror_voided,
                },
            )
        }
    }

    fn stored(writes: &[ReferralBonusWrite]) -> Vec<ReferralBonusRecord> {
        writes
            .iter()
            .map(|write| ReferralBonusRecord {
                kind: write.kind,
                source_accrual_id: write.source_accrual_id,
                bps: write.bps,
                base_units: write.base_units,
                bonus_units: write.bonus_units,
            })
            .collect()
    }

    #[test]
    fn bonus_units_floor_exactly_at_every_size() {
        assert_eq!(bonus_units(0, 1_000), 0);
        assert_eq!(bonus_units(9, 1_000), 0);
        assert_eq!(bonus_units(10, 1_000), 1);
        assert_eq!(bonus_units(19_999, 1_000), 1_999);
        assert_eq!(bonus_units(12_345, 1), 1);
        assert_eq!(bonus_units(12_345, 10_000), 12_345);
        // The full range, no overflow: floor(MAX / 10).
        assert_eq!(bonus_units(u128::MAX, 1_000), u128::MAX / 10);
        assert_eq!(bonus_units(u128::MAX, 10_000), u128::MAX);
        assert_eq!(bonus_units(u128::MAX, 1), u128::MAX / 10_000);
        for base in [1_u128, 7, 9_999, 10_001, 123_456_789, 10u128.pow(30) + 3] {
            for bps in [1_u32, 333, 1_000, 9_999, 10_000] {
                assert_eq!(
                    bonus_units(base, bps),
                    base * u128::from(bps) / 10_000,
                    "{base} {bps}"
                );
            }
        }
    }

    #[test]
    fn earn_until_is_creation_plus_the_setting() {
        assert_eq!(earn_until(at(0), 92), at(92));
        assert_eq!(earn_until(at(0), 180), at(180));
    }

    #[test]
    fn a_credit_in_the_window_gives_a_tenth() {
        let plan = Case::new(vec![credit(1, 1_000_000_007, 10)]).plan();
        assert_eq!(plan.writes.len(), 1);
        let write = &plan.writes[0];
        assert_eq!(write.kind, ReferralBonusKind::Bonus);
        assert_eq!(write.account_id, REFERRER);
        assert_eq!(write.referee_ref_hash, attribution().referee_ref_hash);
        assert_eq!(write.source_accrual_id, Some(Uuid::from_u128(1_001)));
        assert_eq!(write.bps, 1_000);
        assert_eq!(write.base_units, 1_000_000_007);
        assert_eq!(write.bonus_units, 100_000_000);
        assert_eq!(write.rate_version, "v1");
        assert_eq!(write.instrument_id.as_str(), "near_ai_credit_usd");
        assert_eq!(write.rail_id, RailId::near_ai());
    }

    #[test]
    fn the_window_is_from_applied_at_to_earn_until() {
        let plan = Case::new(vec![
            // Before applied (day 1): a batch from before the code.
            credit(1, 100, 0),
            // At applied_at exactly: in.
            accrual(2, PayoutAccrualKind::Credit, 2, Some(REFEREE), 100, 1),
            credit(3, 100, 91),
            // At earn_until exactly: out.
            credit(4, 100, 92),
            credit(5, 100, 200),
        ])
        .plan();
        let sources: Vec<Uuid> = plan
            .writes
            .iter()
            .filter_map(|write| write.source_accrual_id)
            .collect();
        assert_eq!(
            sources,
            vec![Uuid::from_u128(1_002), Uuid::from_u128(1_003)]
        );
        assert_eq!(plan.skipped["referral_outside_window"], 3);
    }

    #[test]
    fn raising_the_earning_days_extends_the_window_without_a_data_change() {
        let mut case = Case::new(vec![credit(1, 100, 120)]);
        assert!(case.plan().writes.is_empty());
        case.earn_days = 180;
        assert_eq!(case.plan().writes.len(), 1);
    }

    #[test]
    fn unowned_accruals_count_once_claimed_and_others_never() {
        let mut case = Case::new(vec![
            accrual(1, PayoutAccrualKind::Credit, 1, None, 100, 10),
            accrual(
                2,
                PayoutAccrualKind::Credit,
                2,
                Some(Uuid::from_u128(99)),
                100,
                10,
            ),
        ]);
        let plan = case.plan();
        assert!(plan.writes.is_empty());
        assert_eq!(plan.skipped["referral_accrual_unowned"], 1);
        // Claimed by the referee: it counts, if its batch is in the window.
        case.accruals[0].account_id = Some(REFEREE);
        let plan = case.plan();
        assert_eq!(plan.writes.len(), 1);
        assert_eq!(
            plan.writes[0].source_accrual_id,
            Some(Uuid::from_u128(1_001))
        );
    }

    #[test]
    fn a_rerun_writes_nothing_new() {
        let mut case = Case::new(vec![credit(1, 100, 10), credit(2, 200, 11)]);
        let first = case.plan();
        assert_eq!(first.writes.len(), 2);
        case.existing = stored(&first.writes);
        assert!(case.plan().writes.is_empty());
        // A new accrual later prices only itself.
        case.accruals.push(credit(3, 300, 12));
        let third = case.plan();
        assert_eq!(third.writes.len(), 1);
        assert_eq!(
            third.writes[0].source_accrual_id,
            Some(Uuid::from_u128(1_003))
        );
    }

    #[test]
    fn a_reversal_takes_back_only_what_its_credit_gave() {
        // Line 1 earned a bonus; line 2's credit was before the window.
        let case = Case::new(vec![
            credit(1, 1_000, 10),
            credit(2, 1_000, 0),
            reversal(11, 1, 400, 10),
            reversal(12, 2, 400, 0),
        ]);
        let plan = case.plan();
        let kinds: Vec<(ReferralBonusKind, u128, u128)> = plan
            .writes
            .iter()
            .map(|write| (write.kind, write.base_units, write.bonus_units))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (ReferralBonusKind::Bonus, 1_000, 100),
                (ReferralBonusKind::Reversal, 400, 40),
            ]
        );
        assert_eq!(plan.skipped["referral_reversal_without_bonus"], 1);
    }

    #[test]
    fn a_reversal_is_priced_at_its_bonus_rows_bps_and_capped_by_it() {
        let mut case = Case::new(vec![credit(1, 1_000, 10)]);
        case.bps = 500;
        let first = case.plan();
        assert_eq!(first.writes[0].bonus_units, 50);
        case.existing = stored(&first.writes);
        // The setting changed since; the reversal keeps the bonus's 5%.
        case.bps = 2_000;
        case.accruals.push(reversal(11, 1, 1_000, 10));
        let plan = case.plan();
        assert_eq!(plan.writes.len(), 1);
        assert_eq!(plan.writes[0].kind, ReferralBonusKind::Reversal);
        assert_eq!(plan.writes[0].bps, 500);
        assert_eq!(plan.writes[0].bonus_units, 50);
    }

    #[test]
    fn reversals_of_one_line_never_sum_past_its_bonus() {
        // 3 reversals of 333 + 334 + 333 against a bonus of floor(1000/10).
        let mut case = Case::new(vec![
            credit(1, 1_000, 10),
            reversal(11, 1, 333, 10),
            reversal(12, 1, 334, 10),
            reversal(13, 1, 333, 10),
        ]);
        let plan = case.plan();
        let taken: u128 = plan
            .writes
            .iter()
            .filter(|write| write.kind == ReferralBonusKind::Reversal)
            .map(|write| write.bonus_units)
            .sum();
        assert!(taken <= 100);
        assert_eq!(taken, 33 + 33 + 33);
        // Stored, a rerun adds nothing.
        case.existing = stored(&plan.writes);
        assert!(case.plan().writes.is_empty());
    }

    #[test]
    fn a_reversal_found_on_a_later_run_still_finds_its_stored_bonus() {
        let mut case = Case::new(vec![credit(1, 1_000, 10)]);
        case.existing = stored(&case.plan().writes);
        case.accruals.push(reversal(11, 1, 1_000, 10));
        let plan = case.plan();
        assert_eq!(plan.writes.len(), 1);
        assert_eq!(plan.writes[0].bonus_units, 100);
    }

    #[test]
    fn a_held_referee_earns_no_new_bonus_but_reversals_still_take_back() {
        let mut case = Case::new(vec![credit(1, 1_000, 10)]);
        case.existing = stored(&case.plan().writes);
        case.accruals.push(credit(2, 1_000, 11));
        case.accruals.push(reversal(11, 1, 1_000, 10));
        case.referee_held = true;
        let plan = case.plan();
        assert_eq!(plan.writes.len(), 1);
        assert_eq!(plan.writes[0].kind, ReferralBonusKind::Reversal);
        assert_eq!(plan.skipped["referral_referee_held"], 1);
        // The hold clears: the held credit is priced on the next run.
        case.existing.extend(stored(&plan.writes));
        case.referee_held = false;
        let plan = case.plan();
        assert_eq!(plan.writes.len(), 1);
        assert_eq!(
            plan.writes[0].source_accrual_id,
            Some(Uuid::from_u128(1_002))
        );
    }

    #[test]
    fn a_voided_referral_plans_nothing() {
        let mut voided = Case::new(vec![credit(1, 1_000, 10)]);
        voided.attribution.voided = true;
        assert!(voided.plan().voided && voided.plan().writes.is_empty());

        let mut mirror = Case::new(vec![credit(1, 1_000, 10)]);
        mirror.mirror_voided = true;
        assert!(mirror.plan().voided);

        let mut void_row = Case::new(vec![credit(1, 1_000, 10)]);
        void_row.existing = vec![ReferralBonusRecord {
            kind: ReferralBonusKind::Void,
            source_accrual_id: None,
            bps: 1_000,
            base_units: 0,
            bonus_units: 0,
        }];
        assert!(void_row.plan().voided);
    }

    #[test]
    fn a_zero_unit_credit_writes_no_row() {
        // The schema refuses a bonus row with base_units 0.
        let plan = Case::new(vec![credit(1, 0, 10)]).plan();
        assert!(plan.writes.is_empty());
        assert_eq!(plan.skipped["referral_base_zero"], 1);
        // A tiny credit still writes its row, with a zero bonus.
        let plan = Case::new(vec![credit(1, 5, 10)]).plan();
        assert_eq!(plan.writes[0].bonus_units, 0);
        assert_eq!(plan.writes[0].base_units, 5);
    }

    #[test]
    fn rows_are_planned_in_batch_order_whatever_the_read_order() {
        let a = Case::new(vec![credit(2, 200, 12), credit(1, 100, 10)]).plan();
        let b = Case::new(vec![credit(1, 100, 10), credit(2, 200, 12)]).plan();
        assert_eq!(a.writes, b.writes);
        assert_eq!(a.writes[0].source_accrual_id, Some(Uuid::from_u128(1_001)));
    }

    #[test]
    fn plan_debug_prints_counts_only() {
        let plan = Case::new(vec![credit(1, 1_000, 10)]).plan();
        let rendered = format!("{plan:?} {:?}", plan.writes);
        assert!(!rendered.contains(&REFERRER.to_string()));
        assert!(!rendered.contains("eeee"));
        assert!(!rendered.contains(&attribution().code_index));
    }

    #[test]
    fn the_control_rail_and_actor_are_stable_safe_ids() {
        assert_eq!(near_ai_referral_control_rail().as_str(), "near_ai.referral");
        let actor = referral_bonus_actor(&RailId::near_ai());
        assert!(actor.starts_with("sha256:") && actor.len() == 71);
        assert_eq!(actor, referral_bonus_actor(&RailId::near_ai()));
    }
}
