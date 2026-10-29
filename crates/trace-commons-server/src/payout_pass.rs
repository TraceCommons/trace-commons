// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The payout pass: one rail's sends, exactly once
//! (docs/superpowers/specs/2026-10-21-nearai-credit-payout-design.md,
//! section 3 "Independence", section 4 "Holds", "Ceilings", "Modes and the
//! kill switch", "One pass, per rail", "Crash recovery, per rail" and
//! "Fail-closed rules", and section 5 "Change and unlink").
//!
//! A [`PayoutPass`] drives one rail, held as `Arc<dyn PayoutRail>`. A run:
//!
//! 0. takes the rail's own single-flight lock (`payout-pass-v1:{rail}`) on a
//!    pinned connection, released on every exit; a held lock ends the run
//!    as [`PayoutPassOutcome::LockBusy`];
//! 1. reads the rail's kill switch and, in `live`, its capacity once;
//! 2. per tenant with a link on the rail: accrues, moves a live link with an
//!    unlink or a waiting successor to `closing` and closes it by reads
//!    only, activates a waiting first link, then per account recovers a
//!    pending line by resending the same line, or writes a new line
//!    `pending` (committed) before the send and records the outcome.
//!
//! Every bound (shared account standing, rail holds, the account ceiling,
//! the per-send maximum, the wire maximum) is checked before a line is
//! written, and the kill switch is re-read before every send. A
//! [`RailRefusal::StopPass`] ends the run and changes no row: not the line,
//! not a hold, not a link. One rail's pass never reads or writes another
//! rail's rows, and holds no lock another rail's pass waits on.
//!
//! In `dry_run` the pass accrues and counts what it would send. It makes no
//! write call to the rail (on `near_ai` not even a zero read, which is a
//! POST), no capacity call, moves no link, and writes no line or hold.
//!
//! Logs carry the rail id, the mode, labels and counts. Never a tenant,
//! account, recipient, program or rate.

use std::collections::BTreeMap;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use trace_commons_gate_api::pipeline::AtomicUnits;
use trace_commons_gate_api::{
    PayoutRail, RailId, RailLine, RailRefusal, ReceiptLookup, RecipientRef, SendOutcome,
};
use uuid::Uuid;

use crate::db::{
    Database, PayoutAuditAction, PayoutLine, PayoutLineResolution, PayoutLineResolve,
    PayoutLineState, PayoutLineWrite, PayoutLink, PayoutLinkClose, PayoutLinkState,
    PayoutLinkTransition, PayoutStore, PendingPayoutLineInsert,
};
use crate::payout_accrual::accrue_rail;
use crate::payout_rate::PayoutMode;
use crate::referral_abuse::hold_holds_referral_stream_only;
use crate::referral_bonus::ReferralBonusHook;

/// The most write calls (sends and zero reads) one pass makes on a rail,
/// below `near_ai`'s per-minute quota (#1271 `max_sends_per_minute`),
/// whatever the capacity reports.
pub const PAYOUT_LOCAL_SEND_LIMIT: u32 = 50;

/// How many already-claimed external records recovery skips before it
/// gives up on a line and holds it.
const MAX_RECEIPT_CANDIDATES: usize = 16;

/// Holds and skip labels the pass itself raises.
pub const PAYOUT_LINK_UNREADABLE: &str = "payout_link_unreadable";
pub const PAYOUT_OVER_CEILING: &str = "payout_over_ceiling";
pub const PAYOUT_AMOUNT_OUT_OF_RANGE: &str = "payout_amount_out_of_range";
pub const PAYOUT_PROGRAM_TOTAL_MISMATCH: &str = "payout_program_total_mismatch";
pub const PAYOUT_RECOVERY_UNMATCHED: &str = "payout_recovery_unmatched";
pub const PAYOUT_LINK_CLOSE_BLOCKED: &str = "payout_link_close_blocked";
pub const PAYOUT_RECEIPT_UNRECORDED: &str = "payout_receipt_unrecorded";
const PAYOUT_SEND_WINDOW_EXHAUSTED: &str = "payout_send_window_exhausted";
const PAYOUT_STORE_FAILED: &str = "payout_store_failed";
const PAYOUT_RAIL_SENDING_DISABLED: &str = "payout_rail_sending_disabled";
pub const PAYOUT_RATE_UNAVAILABLE: &str = "payout_rate_unavailable";

/// The line reference a rail is handed: a `sha256:` hash of the line id,
/// never the id itself. Deterministic, so a recovery resends the same line.
pub fn payout_line_ref_hash(line_id: Uuid) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(format!("payout-line-ref-v1:{line_id}").as_bytes())
    )
}

/// The actor hash the pass writes on the audit and hold rows it makes.
pub fn payout_pass_actor(rail: &RailId) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(format!("payout-pass-actor-v1:{}", rail.as_str()).as_bytes())
    )
}

/// Opens a link's sealed recipient for the length of one call. `None` is a
/// link that will not open (a key the deployment does not hold, a seal that
/// does not verify): the pass holds that account on the rail as
/// `payout_link_unreadable` and never guesses a recipient.
pub trait PayoutRecipientOpener: Send + Sync {
    fn open(&self, link: &PayoutLink) -> Option<RecipientRef>;
}

/// The opener a deployment uses until the link routes, which seal
/// recipients, are built: it opens nothing, so any link found is held
/// `payout_link_unreadable` and nothing is sent.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnconfiguredRecipientOpener;

impl PayoutRecipientOpener for UnconfiguredRecipientOpener {
    fn open(&self, _link: &PayoutLink) -> Option<RecipientRef> {
        None
    }
}

/// One rail's pass settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayoutPassSettings {
    pub mode: PayoutMode,
    /// The lifetime ceiling per account on this rail, in its atomic units.
    pub account_ceiling: AtomicUnits,
    /// At most this many sends per pass ([`PAYOUT_LOCAL_SEND_LIMIT`]).
    pub local_send_limit: u32,
    /// The rail's rate source has a rate now. False when the source is gone
    /// (no settlement table, or one without a positive rate): the pass then
    /// stops as `payout_rate_unavailable` before accrual or any call, so a
    /// stored epoch alone never keeps a rail paying.
    pub rate_in_force: bool,
}

/// How a pass ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayoutPassOutcome {
    Completed,
    /// The rail's mode is `disabled`: nothing ran.
    Disabled,
    /// Another pass of the same rail holds its lock.
    LockBusy,
    /// The rail's kill switch is set (read at the start, or before a send).
    Halted,
    /// The rail stopped the pass ([`RailRefusal::StopPass`], or capacity
    /// with sending off). No row was changed by the stop.
    Stopped {
        label: &'static str,
    },
    /// A store read or write failed.
    Failed {
        label: &'static str,
    },
}

impl PayoutPassOutcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Disabled => "disabled",
            Self::LockBusy => "lock_busy",
            Self::Halted => "payout_halted",
            Self::Stopped { label } | Self::Failed { label } => label,
        }
    }
}

/// What one pass did. Counts and labels only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayoutPassReport {
    pub rail_id: RailId,
    pub mode: PayoutMode,
    pub outcome: PayoutPassOutcome,
    pub tenants: u64,
    /// Accrual rows written.
    pub accrued: u64,
    /// `send` calls made (new lines and recoveries).
    pub sent: u64,
    pub confirmed: u64,
    pub no_op: u64,
    pub retried: u64,
    pub activated: u64,
    pub closed: u64,
    pub close_blocked: u64,
    /// Holds opened, by label.
    pub held: BTreeMap<&'static str, u64>,
    /// Accounts or links passed over without a write, by label.
    pub skipped: BTreeMap<&'static str, u64>,
    /// `dry_run`: accounts that would be sent to, and the units.
    pub dry_run_accounts: u64,
    pub dry_run_units: u128,
}

impl PayoutPassReport {
    fn new(rail_id: RailId, mode: PayoutMode) -> Self {
        Self {
            rail_id,
            mode,
            outcome: PayoutPassOutcome::Completed,
            tenants: 0,
            accrued: 0,
            sent: 0,
            confirmed: 0,
            no_op: 0,
            retried: 0,
            activated: 0,
            closed: 0,
            close_blocked: 0,
            held: BTreeMap::new(),
            skipped: BTreeMap::new(),
            dry_run_accounts: 0,
            dry_run_units: 0,
        }
    }

    fn skip(&mut self, label: &'static str) {
        *self.skipped.entry(label).or_default() += 1;
    }
}

/// Why a run ends early.
enum Halt {
    Stop(&'static str),
    Halted,
    Store,
}

type Step<T> = Result<T, Halt>;

fn store<T>(result: Result<T, crate::error::DatabaseError>) -> Step<T> {
    result.map_err(|_| Halt::Store)
}

/// A rail's own label, checked: never a label that is not a safe identifier.
fn safe(refusal: RailRefusal) -> &'static str {
    refusal.safe_label()
}

/// What this pass may still send. `remaining` counts every write call to
/// the rail -- a send, and a zero read of a program total, which on
/// `near_ai` is a POST in the same per-minute window -- so a pass stays
/// inside the window the rail reported. Reads of our own org (capacity,
/// history, receipts) are GETs and do not count.
struct SendBudget {
    per_send_max: u128,
    remaining: u32,
}

impl SendBudget {
    fn spend(&mut self) {
        self.remaining = self.remaining.saturating_sub(1);
    }
}

/// An account's links on the rail, by state.
#[derive(Default)]
struct AccountLinks {
    waiting: Option<PayoutLink>,
    active: Option<PayoutLink>,
    closing: Option<PayoutLink>,
}

/// What recovery's receipt search found.
enum ReceiptMatch {
    Confirmed,
    Unmatched,
    Refused(RailRefusal),
}

/// One rail's payout pass.
pub struct PayoutPass {
    rail: Arc<dyn PayoutRail>,
    payouts: Arc<dyn PayoutStore>,
    corpus: Arc<dyn Database>,
    opener: Arc<dyn PayoutRecipientOpener>,
    settings: PayoutPassSettings,
    actor: String,
    /// The referral bonus work run after accrual in each tenant
    /// ([`crate::referral_bonus`]); `None` runs none.
    referral: Option<Arc<ReferralBonusHook>>,
}

impl PayoutPass {
    pub fn new(
        rail: Arc<dyn PayoutRail>,
        payouts: Arc<dyn PayoutStore>,
        corpus: Arc<dyn Database>,
        opener: Arc<dyn PayoutRecipientOpener>,
        settings: PayoutPassSettings,
    ) -> Self {
        let actor = payout_pass_actor(&rail.descriptor().rail_id);
        Self {
            rail,
            payouts,
            corpus,
            opener,
            settings,
            actor,
            referral: None,
        }
    }

    /// Runs `hook` after accrual in each tenant this pass visits.
    pub fn with_referral_bonus(mut self, hook: Arc<ReferralBonusHook>) -> Self {
        self.referral = Some(hook);
        self
    }

    pub fn rail_id(&self) -> &RailId {
        &self.rail.descriptor().rail_id
    }

    pub fn mode(&self) -> PayoutMode {
        self.settings.mode
    }

    /// One pass. Never panics on a rail or store fault; the outcome says
    /// how it ended.
    pub async fn run_once(&self) -> PayoutPassReport {
        let mut report = PayoutPassReport::new(self.rail_id().clone(), self.settings.mode);
        if self.settings.mode == PayoutMode::Disabled {
            report.outcome = PayoutPassOutcome::Disabled;
            return report;
        }
        if !self.settings.rate_in_force {
            report.outcome = PayoutPassOutcome::Stopped {
                label: PAYOUT_RATE_UNAVAILABLE,
            };
            self.log(&report);
            return report;
        }
        let lock = match self
            .payouts
            .try_acquire_payout_pass_lock(self.rail_id())
            .await
        {
            Ok(Some(lock)) => lock,
            Ok(None) => {
                report.outcome = PayoutPassOutcome::LockBusy;
                self.log(&report);
                return report;
            }
            Err(_) => {
                report.outcome = PayoutPassOutcome::Failed {
                    label: PAYOUT_STORE_FAILED,
                };
                self.log(&report);
                return report;
            }
        };
        let result = self.run_locked(&mut report).await;
        report.outcome = match result {
            Ok(()) => PayoutPassOutcome::Completed,
            Err(Halt::Stop(label)) => PayoutPassOutcome::Stopped { label },
            Err(Halt::Halted) => PayoutPassOutcome::Halted,
            Err(Halt::Store) => PayoutPassOutcome::Failed {
                label: PAYOUT_STORE_FAILED,
            },
        };
        if lock.release().await.is_err() {
            tracing::warn!(
                rail = %self.rail_id(),
                "payout pass lock release failed; its connection was closed"
            );
        }
        self.log(&report);
        report
    }

    fn log(&self, report: &PayoutPassReport) {
        tracing::info!(
            rail = %report.rail_id,
            mode = report.mode.as_label(),
            outcome = report.outcome.label(),
            tenants = report.tenants,
            accrued = report.accrued,
            sent = report.sent,
            confirmed = report.confirmed,
            no_op = report.no_op,
            retried = report.retried,
            activated = report.activated,
            closed = report.closed,
            close_blocked = report.close_blocked,
            held = ?report.held,
            skipped = ?report.skipped,
            dry_run_accounts = report.dry_run_accounts,
            dry_run_units = %report.dry_run_units,
            "payout pass"
        );
    }

    async fn check_halt(&self) -> Step<()> {
        if store(self.payouts.payout_control(self.rail_id()).await)?.halted {
            return Err(Halt::Halted);
        }
        Ok(())
    }

    async fn run_locked(&self, report: &mut PayoutPassReport) -> Step<()> {
        self.check_halt().await?;
        let tenants = store(
            self.payouts
                .tenants_with_live_payout_links(self.rail_id())
                .await,
        )?;
        report.tenants = tenants.len() as u64;
        // Nothing linked on this rail: no call to the rail at all, so an
        // idle `live` rail costs one enumeration query per tick.
        if tenants.is_empty() {
            return Ok(());
        }
        let mut budget = None;
        if self.settings.mode == PayoutMode::Live {
            // Capacity once per pass, before any line is written: the
            // per-send maximum is one of the bounds a line is checked
            // against.
            let capacity = self
                .rail
                .preflight()
                .await
                .map_err(|refusal| Halt::Stop(safe(refusal)))?;
            if !capacity.sending_enabled {
                let label = capacity
                    .stop_label
                    .map(|label| safe(RailRefusal::StopPass(label)))
                    .unwrap_or(PAYOUT_RAIL_SENDING_DISABLED);
                return Err(Halt::Stop(label));
            }
            let limit = self.settings.local_send_limit;
            budget = Some(SendBudget {
                per_send_max: capacity.per_send_max.get(),
                remaining: capacity
                    .sends_remaining
                    .map_or(limit, |remaining| remaining.min(limit)),
            });
        }
        for tenant in tenants {
            self.run_tenant(&tenant, budget.as_mut(), report).await?;
        }
        Ok(())
    }

    async fn run_tenant(
        &self,
        tenant: &str,
        mut budget: Option<&mut SendBudget>,
        report: &mut PayoutPassReport,
    ) -> Step<()> {
        match accrue_rail(
            self.payouts.as_ref(),
            self.corpus.as_ref(),
            tenant,
            self.rail.descriptor(),
            &self.actor,
        )
        .await
        {
            Ok(accrued) => report.accrued += accrued.credits_inserted + accrued.reversals_inserted,
            // Owed may be missing a reversal: send nothing in this tenant
            // until accrual runs clean.
            Err(error) => {
                report.skip(error.label());
                return Ok(());
            }
        }
        // Step 1b: the referral bonus, written in each referrer's tenant.
        // Its faults are its own: they never stop this tenant's sends.
        if let Some(referral) = &self.referral {
            referral.run_tenant(tenant, self.rail_id()).await;
        }
        let links = store(
            self.payouts
                .payout_links(
                    tenant,
                    self.rail_id(),
                    &[
                        PayoutLinkState::Waiting,
                        PayoutLinkState::Active,
                        PayoutLinkState::Closing,
                    ],
                )
                .await,
        )?;
        let mut accounts: BTreeMap<Uuid, AccountLinks> = BTreeMap::new();
        for link in links {
            let entry = accounts.entry(link.account_id).or_default();
            match link.state {
                PayoutLinkState::Waiting => entry.waiting = Some(link),
                PayoutLinkState::Active => entry.active = Some(link),
                PayoutLinkState::Closing => entry.closing = Some(link),
                PayoutLinkState::Closed => {}
            }
        }
        for (account, links) in accounts {
            match budget.as_deref_mut() {
                Some(budget) => {
                    self.run_account(tenant, account, links, budget, report)
                        .await?
                }
                None => self.count_dry_run(tenant, account, links, report).await?,
            }
        }
        Ok(())
    }

    /// `dry_run`: what this account would be sent, from stored rows only.
    async fn count_dry_run(
        &self,
        tenant: &str,
        account: Uuid,
        links: AccountLinks,
        report: &mut PayoutPassReport,
    ) -> Step<()> {
        let Some(link) = links.active.or(links.waiting) else {
            return Ok(());
        };
        if let Some(label) = self.standing_hold(tenant, account).await? {
            report.skip(label);
            return Ok(());
        }
        let owed = store(
            self.payouts
                .payout_owed(tenant, account, self.rail_id())
                .await,
        )?;
        let Some(owed) = owed.net() else {
            report.skip("payout_owed_inconsistent");
            return Ok(());
        };
        if owed > self.settings.account_ceiling.get() {
            report.skip(PAYOUT_OVER_CEILING);
            return Ok(());
        }
        let closed = store(
            self.payouts
                .payout_closed_links_final_paid(tenant, account, self.rail_id())
                .await,
        )?;
        let confirmed = store(
            self.payouts
                .payout_link_confirmed_total(tenant, link.link_id)
                .await,
        )?;
        let Some(target) = owed.checked_sub(closed) else {
            return Ok(());
        };
        if let Some(units) = target.checked_sub(confirmed).filter(|units| *units > 0) {
            report.dry_run_accounts += 1;
            report.dry_run_units = report.dry_run_units.saturating_add(units);
        }
        Ok(())
    }

    /// The shared standing (closed account, credit hold) or any open hold on
    /// this rail, as a label.
    async fn standing_hold(&self, tenant: &str, account: Uuid) -> Step<Option<&'static str>> {
        let standing = store(self.payouts.payout_account_standing(tenant, account).await)?;
        if let Some(label) = standing.hold_label() {
            return Ok(Some(label));
        }
        let holds = store(
            self.payouts
                .open_payout_hold_labels(tenant, account, self.rail_id())
                .await,
        )?;
        // A referral-stream hold (`referral_*`) never holds the earned
        // stream.
        let held = holds
            .iter()
            .any(|label| !hold_holds_referral_stream_only(label));
        Ok(held.then_some("payout_account_held"))
    }

    async fn hold(
        &self,
        tenant: &str,
        account: Uuid,
        label: &'static str,
        report: &mut PayoutPassReport,
    ) -> Step<()> {
        let opened = store(
            self.payouts
                .open_payout_hold(tenant, account, self.rail_id(), label, &self.actor)
                .await,
        )?;
        if opened {
            *report.held.entry(label).or_default() += 1;
        }
        Ok(())
    }

    async fn reload_live(&self, tenant: &str, account: Uuid) -> Step<Option<PayoutLink>> {
        store(
            self.payouts
                .payout_live_link(tenant, account, self.rail_id())
                .await,
        )
    }

    async fn run_account(
        &self,
        tenant: &str,
        account: Uuid,
        links: AccountLinks,
        budget: &mut SendBudget,
        report: &mut PayoutPassReport,
    ) -> Step<()> {
        let AccountLinks {
            waiting,
            mut active,
            mut closing,
        } = links;

        // Apply requests: an unlink, or a confirmed waiting successor,
        // moves the live link to closing (spec section 5, "Change and
        // unlink"). Only the pass moves a link, under its lock.
        if let Some(link) = active.take() {
            let unlink = link.unlink_requested_at.is_some();
            if unlink || waiting.is_some() {
                let label = if unlink {
                    "payout_link_unlink"
                } else {
                    "payout_link_change"
                };
                let moved = store(
                    self.payouts
                        .transition_payout_link(
                            tenant,
                            link.link_id,
                            PayoutLinkState::Active,
                            PayoutLinkState::Closing,
                            PayoutAuditAction::LinkClosing,
                            &self.actor,
                            Some(label),
                        )
                        .await,
                )?;
                if moved == PayoutLinkTransition::Moved && closing.is_none() {
                    closing = Some(PayoutLink {
                        state: PayoutLinkState::Closing,
                        ..link
                    });
                } else {
                    report.skip("payout_link_not_moved");
                    return Ok(());
                }
            } else {
                active = Some(link);
            }
        }

        if let Some(link) = closing {
            // Closing reads the program total with a zero send, which spends
            // the window like any send.
            if budget.remaining == 0 {
                report.skip(PAYOUT_SEND_WINDOW_EXHAUSTED);
                return Ok(());
            }
            if !self
                .close_link(tenant, account, &link, waiting.as_ref(), budget, report)
                .await?
            {
                report.close_blocked += 1;
                report.skip(PAYOUT_LINK_CLOSE_BLOCKED);
                return Ok(());
            }
            report.closed += 1;
            if waiting.is_some() {
                report.activated += 1;
            }
            active = self.reload_live(tenant, account).await?;
        } else if active.is_none() {
            if let Some(link) = waiting {
                let moved = store(
                    self.payouts
                        .transition_payout_link(
                            tenant,
                            link.link_id,
                            PayoutLinkState::Waiting,
                            PayoutLinkState::Active,
                            PayoutAuditAction::LinkActivated,
                            &self.actor,
                            None,
                        )
                        .await,
                )?;
                if moved == PayoutLinkTransition::Moved {
                    report.activated += 1;
                }
                active = self.reload_live(tenant, account).await?;
            }
        }

        match active {
            Some(link) => self.pay(tenant, account, &link, budget, report).await,
            None => Ok(()),
        }
    }

    /// Resolves a closing link by reads only and closes it, activating
    /// `successor` in the same transaction. False when it cannot close yet.
    async fn close_link(
        &self,
        tenant: &str,
        account: Uuid,
        link: &PayoutLink,
        successor: Option<&PayoutLink>,
        budget: &mut SendBudget,
        report: &mut PayoutPassReport,
    ) -> Step<bool> {
        let Some(recipient) = self.opener.open(link) else {
            self.hold(tenant, account, PAYOUT_LINK_UNREADABLE, report)
                .await?;
            return Ok(false);
        };
        budget.spend();
        let total = match self.rail.read_total(&recipient, &link.program).await {
            Ok(total) => total.get(),
            // The old recipient is gone: the program total comes from our
            // own history (spec section 5, "Old org gone").
            Err(RailRefusal::RecipientUnavailable) => {
                match self.rail.history(&recipient, &link.program).await {
                    Ok(entries) => {
                        let sum = entries
                            .iter()
                            .try_fold(0_u128, |sum, entry| sum.checked_add(entry.delta.get()));
                        match sum {
                            Some(sum) => sum,
                            None => return Ok(false),
                        }
                    }
                    Err(RailRefusal::StopPass(label)) => {
                        return Err(Halt::Stop(safe(RailRefusal::StopPass(label))));
                    }
                    Err(_) => return Ok(false),
                }
            }
            Err(RailRefusal::StopPass(label)) => {
                return Err(Halt::Stop(safe(RailRefusal::StopPass(label))));
            }
            Err(_) => return Ok(false),
        };
        if let Some(line) = store(
            self.payouts
                .pending_payout_line_for_link(tenant, link.link_id)
                .await,
        )? {
            if total >= line.target_total.get() {
                let line_ref = payout_line_ref_hash(line.line_id);
                let Some(rail_line) = rail_line(&line_ref, &recipient, link, &line) else {
                    return Ok(false);
                };
                match self
                    .match_receipt(tenant, &line, &rail_line, report)
                    .await?
                {
                    ReceiptMatch::Confirmed => {}
                    ReceiptMatch::Unmatched => {
                        self.end_line(tenant, account, &line, PAYOUT_RECOVERY_UNMATCHED, report)
                            .await?;
                    }
                    ReceiptMatch::Refused(RailRefusal::StopPass(label)) => {
                        return Err(Halt::Stop(safe(RailRefusal::StopPass(label))));
                    }
                    ReceiptMatch::Refused(_) => return Ok(false),
                }
            } else {
                // The total never reached the line's target: it did not land.
                if store(
                    self.payouts
                        .resolve_payout_line(tenant, line.line_id, &PayoutLineResolution::NoOp)
                        .await,
                )? == PayoutLineResolve::Resolved
                {
                    report.no_op += 1;
                }
            }
        }
        let closed = store(
            self.payouts
                .close_payout_link(
                    tenant,
                    link.link_id,
                    AtomicUnits::from_raw(total),
                    successor.map(|successor| successor.link_id),
                    &self.actor,
                    None,
                )
                .await,
        )?;
        Ok(closed == PayoutLinkClose::Closed)
    }

    async fn pay(
        &self,
        tenant: &str,
        account: Uuid,
        link: &PayoutLink,
        budget: &mut SendBudget,
        report: &mut PayoutPassReport,
    ) -> Step<()> {
        if link.recipient_unavailable_at.is_some() {
            report.skip("payout_recipient_unavailable");
            return Ok(());
        }
        let standing = store(self.payouts.payout_account_standing(tenant, account).await)?;
        if let Some(label) = standing.hold_label() {
            report.skip(label);
            return Ok(());
        }
        let owed = store(
            self.payouts
                .payout_owed(tenant, account, self.rail_id())
                .await,
        )?;
        let Some(owed_net) = owed.net() else {
            return self
                .hold(tenant, account, "payout_owed_inconsistent", report)
                .await;
        };
        // The lifetime ceiling per account on this rail: everything this
        // account would have been sent across every link, checked before
        // any call to the rail. An operator lifts the hold by raising the
        // setting (spec section 4, "Ceilings"): once owed fits, the next
        // pass clears the hold, with its audit row, and pays.
        if owed_net > self.settings.account_ceiling.get() {
            return self
                .hold(tenant, account, PAYOUT_OVER_CEILING, report)
                .await;
        }
        if store(
            self.payouts
                .clear_payout_hold(
                    tenant,
                    account,
                    self.rail_id(),
                    PAYOUT_OVER_CEILING,
                    &self.actor,
                )
                .await,
        )? {
            report.skip("payout_over_ceiling_cleared");
        }
        let rail_holds = store(
            self.payouts
                .open_payout_hold_labels(tenant, account, self.rail_id())
                .await,
        )?;
        if rail_holds
            .iter()
            .any(|label| !hold_holds_referral_stream_only(label))
        {
            report.skip("payout_account_held");
            return Ok(());
        }
        let Some(recipient) = self.opener.open(link) else {
            self.hold(tenant, account, PAYOUT_LINK_UNREADABLE, report)
                .await?;
            return Ok(());
        };

        // A pending line goes to recovery first: the same line, resent.
        if let Some(line) = store(
            self.payouts
                .pending_payout_line_for_link(tenant, link.link_id)
                .await,
        )? {
            if budget.remaining == 0 {
                report.skip(PAYOUT_SEND_WINDOW_EXHAUSTED);
                return Ok(());
            }
            return self
                .send_line(tenant, account, link, &recipient, &line, budget, report)
                .await;
        }

        let closed = store(
            self.payouts
                .payout_closed_links_final_paid(tenant, account, self.rail_id())
                .await,
        )?;
        let confirmed = store(
            self.payouts
                .payout_link_confirmed_total(tenant, link.link_id)
                .await,
        )?;
        // Netting: owed below what earlier links and this one already got
        // sends nothing until later credit fills the gap.
        let Some(target_full) = owed_net.checked_sub(closed).filter(|t| *t > confirmed) else {
            if owed_net > 0 {
                report.skip("payout_nothing_owed");
            }
            return Ok(());
        };
        // A new line costs two write calls: the zero read of the program
        // total (a POST on `near_ai`), then the send. Start one only when
        // the window has room for both.
        if budget.remaining < 2 {
            report.skip(PAYOUT_SEND_WINDOW_EXHAUSTED);
            return Ok(());
        }
        if budget.per_send_max == 0 {
            report.skip("payout_per_send_max_zero");
            return Ok(());
        }
        budget.spend();
        let program_total = match self.rail.read_total(&recipient, &link.program).await {
            Ok(total) => total.get(),
            Err(refusal) => {
                return self
                    .refused_without_line(tenant, account, link, refusal, report)
                    .await;
            }
        };
        // Every confirmed line is on the program and nothing else is: a
        // different total is an outside send, or a bug. Hold for an operator.
        if program_total != confirmed {
            return self
                .hold(tenant, account, PAYOUT_PROGRAM_TOTAL_MISMATCH, report)
                .await;
        }
        let target = target_full.min(program_total.saturating_add(budget.per_send_max));
        if self
            .rail
            .descriptor()
            .wire_max
            .is_some_and(|max| target > max.get())
        {
            return self
                .hold(tenant, account, PAYOUT_AMOUNT_OUT_OF_RANGE, report)
                .await;
        }

        self.check_halt().await?;
        let write = PayoutLineWrite {
            line_id: Uuid::new_v4(),
            link_id: link.link_id,
            account_id: account,
            rail_id: self.rail_id().clone(),
            target_total: AtomicUnits::from_raw(target),
            delta: AtomicUnits::from_raw(target - program_total),
            owed_through_accrual_count: owed.accrual_count,
        };
        match store(
            self.payouts
                .insert_pending_payout_line(tenant, &write)
                .await,
        )? {
            PendingPayoutLineInsert::Inserted => {}
            PendingPayoutLineInsert::LinkHasPendingLine => {
                report.skip("payout_line_pending");
                return Ok(());
            }
            PendingPayoutLineInsert::LinkNotActive => {
                report.skip("payout_link_not_active");
                return Ok(());
            }
        }
        let line = PayoutLine {
            line_id: write.line_id,
            link_id: write.link_id,
            account_id: write.account_id,
            rail_id: write.rail_id,
            target_total: write.target_total,
            delta: write.delta,
            state: PayoutLineState::Pending,
            receipt_hash: None,
            attempts: 0,
            last_error_label: None,
        };
        self.send_line(tenant, account, link, &recipient, &line, budget, report)
            .await
    }

    /// Sends one pending line (new or recovered) and records the outcome.
    #[allow(clippy::too_many_arguments)]
    async fn send_line(
        &self,
        tenant: &str,
        account: Uuid,
        link: &PayoutLink,
        recipient: &RecipientRef,
        line: &PayoutLine,
        budget: &mut SendBudget,
        report: &mut PayoutPassReport,
    ) -> Step<()> {
        // The kill switch, re-read before every send.
        self.check_halt().await?;
        let line_ref = payout_line_ref_hash(line.line_id);
        let Some(rail_line) = rail_line(&line_ref, recipient, link, line) else {
            return self
                .fail_line(tenant, account, line, "payout_line_invalid", report)
                .await;
        };
        budget.spend();
        report.sent += 1;
        match self.rail.send(&rail_line).await {
            Ok(SendOutcome::Committed {
                receipt_hash,
                delta,
                total_after,
            }) => {
                match store(
                    self.payouts
                        .resolve_payout_line(
                            tenant,
                            line.line_id,
                            &PayoutLineResolution::Confirmed {
                                receipt_hash: receipt_hash.clone(),
                                delta,
                            },
                        )
                        .await,
                )? {
                    PayoutLineResolve::Resolved => report.confirmed += 1,
                    PayoutLineResolve::ReceiptAlreadyClaimed => {
                        return self
                            .end_line(tenant, account, line, "payout_receipt_conflict", report)
                            .await;
                    }
                    // The send committed, but the line was ended by another
                    // path first. If a line already holds this receipt it
                    // is recorded; otherwise money moved that no row
                    // records, so never drop it: hold the account for an
                    // operator, who finds the record in the rail's history.
                    PayoutLineResolve::NotPending => {
                        if store(
                            self.payouts
                                .payout_receipt_is_claimed(tenant, self.rail_id(), &receipt_hash)
                                .await,
                        )? {
                            report.skip("payout_line_not_pending");
                        } else {
                            tracing::error!(
                                rail = %self.rail_id(),
                                label = PAYOUT_RECEIPT_UNRECORDED,
                                "payout send committed on a line no longer pending"
                            );
                            return self
                                .hold(tenant, account, PAYOUT_RECEIPT_UNRECORDED, report)
                                .await;
                        }
                    }
                }
                if total_after.is_some_and(|total| total != line.target_total) {
                    self.hold(tenant, account, PAYOUT_PROGRAM_TOTAL_MISMATCH, report)
                        .await?;
                }
                Ok(())
            }
            Ok(SendOutcome::AlreadyAtOrAbove { total }) => {
                if total < line.target_total {
                    return self
                        .end_line(tenant, account, line, PAYOUT_PROGRAM_TOTAL_MISMATCH, report)
                        .await;
                }
                // The line may have landed earlier: find its record.
                match self.match_receipt(tenant, line, &rail_line, report).await? {
                    ReceiptMatch::Confirmed => Ok(()),
                    ReceiptMatch::Unmatched => {
                        self.end_line(tenant, account, line, PAYOUT_RECOVERY_UNMATCHED, report)
                            .await
                    }
                    ReceiptMatch::Refused(refusal) => {
                        self.refused(tenant, account, link, line, refusal, report)
                            .await
                    }
                }
            }
            Err(refusal) => {
                self.refused(tenant, account, link, line, refusal, report)
                    .await
            }
        }
    }

    /// Pages the rail's records for the one that answers `line`, skipping
    /// any already claimed by another line, and confirms it.
    async fn match_receipt(
        &self,
        tenant: &str,
        line: &PayoutLine,
        rail_line: &RailLine<'_>,
        report: &mut PayoutPassReport,
    ) -> Step<ReceiptMatch> {
        let mut exclude: Vec<String> = Vec::new();
        for _ in 0..MAX_RECEIPT_CANDIDATES {
            let (receipt_hash, delta) = match self.rail.find_receipt(rail_line, &exclude).await {
                Ok(ReceiptLookup::Found {
                    receipt_hash,
                    delta,
                }) => (receipt_hash, delta),
                Ok(ReceiptLookup::NotFound) => return Ok(ReceiptMatch::Unmatched),
                Err(refusal) => return Ok(ReceiptMatch::Refused(refusal)),
            };
            if store(
                self.payouts
                    .payout_receipt_is_claimed(tenant, self.rail_id(), &receipt_hash)
                    .await,
            )? {
                exclude.push(receipt_hash);
                continue;
            }
            match store(
                self.payouts
                    .resolve_payout_line(
                        tenant,
                        line.line_id,
                        &PayoutLineResolution::Confirmed {
                            receipt_hash: receipt_hash.clone(),
                            delta,
                        },
                    )
                    .await,
            )? {
                PayoutLineResolve::Resolved => {
                    report.confirmed += 1;
                    return Ok(ReceiptMatch::Confirmed);
                }
                PayoutLineResolve::ReceiptAlreadyClaimed => exclude.push(receipt_hash),
                PayoutLineResolve::NotPending => return Ok(ReceiptMatch::Confirmed),
            }
        }
        Ok(ReceiptMatch::Unmatched)
    }

    /// A refusal on a line, as its variant says (spec section 3).
    async fn refused(
        &self,
        tenant: &str,
        account: Uuid,
        link: &PayoutLink,
        line: &PayoutLine,
        refusal: RailRefusal,
        report: &mut PayoutPassReport,
    ) -> Step<()> {
        let label = safe(refusal);
        match refusal {
            RailRefusal::Retry(_) => {
                store(
                    self.payouts
                        .resolve_payout_line(
                            tenant,
                            line.line_id,
                            &PayoutLineResolution::Retry {
                                label: label.to_owned(),
                            },
                        )
                        .await,
                )?;
                report.retried += 1;
                Ok(())
            }
            // Stop the rail; change no row, the pending line included.
            RailRefusal::StopPass(_) => Err(Halt::Stop(label)),
            RailRefusal::HoldLine(_) => self.end_line(tenant, account, line, label, report).await,
            RailRefusal::RecipientUnavailable => {
                store(
                    self.payouts
                        .resolve_payout_line(
                            tenant,
                            line.line_id,
                            &PayoutLineResolution::Held {
                                label: label.to_owned(),
                            },
                        )
                        .await,
                )?;
                store(
                    self.payouts
                        .mark_payout_link_recipient_unavailable(
                            tenant,
                            link.link_id,
                            &self.actor,
                            Some(label),
                        )
                        .await,
                )?;
                Ok(())
            }
            RailRefusal::Failed(_) => self.fail_line(tenant, account, line, label, report).await,
        }
    }

    /// A refusal on a read made before any line was written.
    async fn refused_without_line(
        &self,
        tenant: &str,
        account: Uuid,
        link: &PayoutLink,
        refusal: RailRefusal,
        report: &mut PayoutPassReport,
    ) -> Step<()> {
        let label = safe(refusal);
        match refusal {
            RailRefusal::Retry(_) => {
                report.skip(label);
                Ok(())
            }
            RailRefusal::StopPass(_) => Err(Halt::Stop(label)),
            RailRefusal::HoldLine(_) | RailRefusal::Failed(_) => {
                self.hold(tenant, account, label, report).await
            }
            RailRefusal::RecipientUnavailable => {
                store(
                    self.payouts
                        .mark_payout_link_recipient_unavailable(
                            tenant,
                            link.link_id,
                            &self.actor,
                            Some(label),
                        )
                        .await,
                )?;
                Ok(())
            }
        }
    }

    /// Ends a line `held` and holds the account on this rail, so the next
    /// pass does not write the same line again.
    async fn end_line(
        &self,
        tenant: &str,
        account: Uuid,
        line: &PayoutLine,
        label: &'static str,
        report: &mut PayoutPassReport,
    ) -> Step<()> {
        store(
            self.payouts
                .resolve_payout_line(
                    tenant,
                    line.line_id,
                    &PayoutLineResolution::Held {
                        label: label.to_owned(),
                    },
                )
                .await,
        )?;
        self.hold(tenant, account, label, report).await
    }

    /// Ends a line `failed` (a fault on our side) and holds the account.
    async fn fail_line(
        &self,
        tenant: &str,
        account: Uuid,
        line: &PayoutLine,
        label: &'static str,
        report: &mut PayoutPassReport,
    ) -> Step<()> {
        store(
            self.payouts
                .resolve_payout_line(
                    tenant,
                    line.line_id,
                    &PayoutLineResolution::Failed {
                        label: label.to_owned(),
                    },
                )
                .await,
        )?;
        self.hold(tenant, account, label, report).await
    }
}

/// The rail's view of a stored line: its reference, the link's program,
/// its target, and what was confirmed before it (`target - delta`).
fn rail_line<'a>(
    line_ref: &'a str,
    recipient: &'a RecipientRef,
    link: &'a PayoutLink,
    line: &PayoutLine,
) -> Option<RailLine<'a>> {
    let confirmed = line.target_total.get().checked_sub(line.delta.get())?;
    RailLine::new(
        line_ref,
        recipient,
        &link.program,
        line.target_total,
        AtomicUnits::from_raw(confirmed),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_refs_are_stable_hashes_of_the_line_id() {
        let id = Uuid::from_u128(42);
        let reference = payout_line_ref_hash(id);
        assert_eq!(reference, payout_line_ref_hash(id));
        assert_ne!(reference, payout_line_ref_hash(Uuid::from_u128(43)));
        assert!(reference.starts_with("sha256:"));
        assert_eq!(reference.len(), 7 + 64);
        assert!(!reference.contains(&id.to_string()));
    }

    #[test]
    fn the_pass_actor_is_per_rail() {
        let near = payout_pass_actor(&RailId::near_ai());
        assert!(near.starts_with("sha256:"));
        assert_ne!(near, payout_pass_actor(&RailId::new("test_keyed").unwrap()));
    }

    #[test]
    fn every_label_the_pass_raises_is_safe() {
        for label in [
            PAYOUT_LINK_UNREADABLE,
            PAYOUT_OVER_CEILING,
            PAYOUT_AMOUNT_OUT_OF_RANGE,
            PAYOUT_PROGRAM_TOTAL_MISMATCH,
            PAYOUT_RECOVERY_UNMATCHED,
            PAYOUT_LINK_CLOSE_BLOCKED,
            PAYOUT_RECEIPT_UNRECORDED,
            PAYOUT_STORE_FAILED,
            PAYOUT_RAIL_SENDING_DISABLED,
            PAYOUT_RATE_UNAVAILABLE,
            PayoutPassOutcome::Halted.label(),
            PayoutPassOutcome::LockBusy.label(),
        ] {
            assert!(RailRefusal::StopPass(label).label_is_safe(), "{label}");
        }
    }

    #[test]
    fn the_unconfigured_opener_opens_nothing() {
        use crate::db::SealedRecipient;
        let link = PayoutLink {
            account_id: Uuid::from_u128(1),
            rail_id: RailId::near_ai(),
            link_id: Uuid::from_u128(2),
            recipient_kind: trace_commons_gate_api::RecipientKind::near_ai_org(),
            state: PayoutLinkState::Active,
            recipient: SealedRecipient {
                sealed: serde_json::json!({}),
                recipient_index: "sha256:00".into(),
                recipient_key_ref: "key".into(),
                index_pepper_ref: "pepper".into(),
                subject_index: "sha256:11".into(),
                recipient_ref: None,
            },
            program: "tc1-0".into(),
            final_paid: None,
            unlink_requested_at: None,
            recipient_unavailable_at: None,
            version: 1,
        };
        assert!(UnconfiguredRecipientOpener.open(&link).is_none());
    }
}
