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
//! **The referral stream** (docs/superpowers/specs/2026-10-29-referral-
//! rewards-design.md, section 5). A pass given one with
//! [`PayoutPass::with_referral_stream`] also pays each active link's second
//! program, the referral program ([`PayoutRail::referral_program_for_link`]),
//! with the account's referral bonus: `owed_referral` is the sum of its
//! `trace_referral_bonus_accruals` rows, netted against what closed links'
//! referral programs already got (`final_paid_referral`). It runs through
//! its own rail object (the dedicated referral credential), its own
//! capacity and send window, its own ceiling and its own holds; lines carry
//! `stream = 'referral'`, recovery and receipt lookup read only the referral
//! program, and closing a link reads both programs. The stream is
//! independent of the earned one: a referral-stream stop
//! ([`RailRefusal::StopPass`] from the referral credential, its budget
//! spent included) stops only the referral stream for the rest of the run,
//! and a referral-stream hold (a label starting [`REFERRAL_HOLD_PREFIX`])
//! never holds earned sends. The shared account standing, an unreadable
//! link, and the rail's kill switch stop both. Its mode never exceeds the
//! pass's: in a `dry_run` pass it counts only.
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
    PayoutLinkTransition, PayoutStore, PayoutStream, PendingPayoutLineInsert,
};
use crate::payout_accrual::accrue_rail;
use crate::payout_rate::PayoutMode;

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
const PAYOUT_RECEIPT_CONFLICT: &str = "payout_receipt_conflict";
const PAYOUT_LINE_INVALID: &str = "payout_line_invalid";
const PAYOUT_OWED_INCONSISTENT: &str = "payout_owed_inconsistent";

/// Every hold that holds the referral stream alone starts with this
/// (referral rewards spec section 4, "Holds on the referral stream"): the
/// ones the bonus pass opens (`referral_same_payout_org`,
/// `referral_same_payout_subject`, `referral_code_suspended`,
/// `referral_referrer_suspended`), the per-referrer ceiling, and every hold
/// the referral stream's own sends raise. The earned stream ignores them.
pub const REFERRAL_HOLD_PREFIX: &str = "referral_";
/// The referrer's referral owed exceeds the referral ceiling.
pub const REFERRAL_OVER_CEILING: &str = "referral_over_ceiling";
pub const REFERRAL_PROGRAM_TOTAL_MISMATCH: &str = "referral_program_total_mismatch";
pub const REFERRAL_RECOVERY_UNMATCHED: &str = "referral_recovery_unmatched";
pub const REFERRAL_RECEIPT_UNRECORDED: &str = "referral_receipt_unrecorded";
pub const REFERRAL_AMOUNT_OUT_OF_RANGE: &str = "referral_amount_out_of_range";
pub const REFERRAL_RECEIPT_CONFLICT: &str = "referral_receipt_conflict";
pub const REFERRAL_LINE_INVALID: &str = "referral_line_invalid";
pub const REFERRAL_OWED_INCONSISTENT: &str = "referral_owed_inconsistent";
pub const REFERRAL_PROGRAM_MISMATCH: &str = "referral_program_mismatch";
/// A rail refusal held a referral line: the line keeps the rail's own label,
/// the hold is this one, so it holds the referral stream alone.
pub const REFERRAL_STREAM_HELD: &str = "referral_stream_held";
/// The rail has no referral program: the stream sends nothing on it.
const REFERRAL_STREAM_UNSUPPORTED: &str = "referral_stream_unsupported";
const REFERRAL_SENDING_DISABLED: &str = "referral_sending_disabled";

/// Whether `label` is a hold on the referral stream alone.
pub fn is_referral_stream_hold(label: &str) -> bool {
    label.starts_with(REFERRAL_HOLD_PREFIX)
}

/// Whether an open hold labelled `label` on the rail holds `stream`. The
/// earned stream is held by every hold that is not a referral-stream hold;
/// the referral stream by its own holds and by an unreadable link, which no
/// stream can pay. The shared account standing (closed, credit hold) holds
/// both and is not a hold row.
pub fn hold_applies_to_stream(stream: PayoutStream, label: &str) -> bool {
    match stream {
        PayoutStream::Earned => !is_referral_stream_hold(label),
        PayoutStream::Referral => is_referral_stream_hold(label) || label == PAYOUT_LINK_UNREADABLE,
    }
}

/// The label a hold raised on `stream` is opened under: the earned stream
/// keeps the pass's label; the referral stream maps it to its own, so the
/// hold stops only the referral stream. A label that is already a
/// referral-stream hold, or the link-wide [`PAYOUT_LINK_UNREADABLE`], is
/// kept; a rail's own label becomes [`REFERRAL_STREAM_HELD`].
pub fn stream_hold_label(stream: PayoutStream, label: &'static str) -> &'static str {
    if stream == PayoutStream::Earned
        || is_referral_stream_hold(label)
        || label == PAYOUT_LINK_UNREADABLE
    {
        return label;
    }
    match label {
        PAYOUT_PROGRAM_TOTAL_MISMATCH => REFERRAL_PROGRAM_TOTAL_MISMATCH,
        PAYOUT_RECOVERY_UNMATCHED => REFERRAL_RECOVERY_UNMATCHED,
        PAYOUT_RECEIPT_UNRECORDED => REFERRAL_RECEIPT_UNRECORDED,
        PAYOUT_AMOUNT_OUT_OF_RANGE => REFERRAL_AMOUNT_OUT_OF_RANGE,
        PAYOUT_RECEIPT_CONFLICT => REFERRAL_RECEIPT_CONFLICT,
        PAYOUT_LINE_INVALID => REFERRAL_LINE_INVALID,
        PAYOUT_OWED_INCONSISTENT => REFERRAL_OWED_INCONSISTENT,
        _ => REFERRAL_STREAM_HELD,
    }
}

/// The hold label a stream's status shows: the shared standing first, then
/// the first open hold (by label) that holds the stream.
pub fn stream_status_hold_label(
    stream: PayoutStream,
    standing: Option<&'static str>,
    open_labels: &[String],
) -> Option<String> {
    if let Some(label) = standing {
        return Some(label.to_owned());
    }
    let mut held: Vec<&String> = open_labels
        .iter()
        .filter(|label| hold_applies_to_stream(stream, label))
        .collect();
    held.sort();
    held.first().map(|label| (*label).clone())
}

/// What a stream's next line may still bring the link's program to, before
/// the per-send cap: `owed` less what closed links' programs of the stream
/// already got, when that is above what this link's program has confirmed.
/// `None` sends nothing (netting: owed at or below what was already sent
/// waits until later credit fills the gap).
pub fn stream_target(owed: u128, closed_final: u128, confirmed: u128) -> Option<u128> {
    owed.checked_sub(closed_final)
        .filter(|target| *target > confirmed)
}

/// One send's target: `target_full`, capped at the program's total plus
/// the rail's per-send maximum.
pub fn capped_stream_target(target_full: u128, program_total: u128, per_send_max: u128) -> u128 {
    target_full.min(program_total.saturating_add(per_send_max))
}

/// A status stream's `waiting`: owed less sent, floored at zero.
pub fn stream_waiting(owed: u128, sent: u128) -> u128 {
    owed.saturating_sub(sent)
}

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

/// The referral stream's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferralStreamSettings {
    /// Never above the pass's own mode: in a `dry_run` pass the stream
    /// counts only, whatever this says.
    pub mode: PayoutMode,
    /// The most referral credit one referrer is owed before its referral
    /// stream is held ([`REFERRAL_OVER_CEILING`]).
    pub account_ceiling: AtomicUnits,
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
    /// Earned-stream `send` calls made (new lines and recoveries).
    pub sent: u64,
    /// Earned lines confirmed.
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
    /// The referral stream's mode in this pass: `disabled` without one.
    pub referral_mode: PayoutMode,
    /// Referral-stream `send` calls made, and referral lines confirmed.
    pub referral_sent: u64,
    pub referral_confirmed: u64,
    /// The label that stopped the referral stream alone, when one did.
    pub referral_stopped: Option<&'static str>,
    /// Referral counts of a dry run (a `dry_run` pass, or a referral stream
    /// in `dry_run`).
    pub referral_dry_run_accounts: u64,
    pub referral_dry_run_units: u128,
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
            referral_mode: PayoutMode::Disabled,
            referral_sent: 0,
            referral_confirmed: 0,
            referral_stopped: None,
            referral_dry_run_accounts: 0,
            referral_dry_run_units: 0,
        }
    }

    fn skip(&mut self, label: &'static str) {
        *self.skipped.entry(label).or_default() += 1;
    }

    fn count_sent(&mut self, stream: PayoutStream) {
        match stream {
            PayoutStream::Earned => self.sent += 1,
            PayoutStream::Referral => self.referral_sent += 1,
        }
    }

    fn count_confirmed(&mut self, stream: PayoutStream) {
        match stream {
            PayoutStream::Earned => self.confirmed += 1,
            PayoutStream::Referral => self.referral_confirmed += 1,
        }
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

/// The referral stream's state for the length of one run.
enum ReferralRun {
    /// No referral stream, or one `disabled`.
    Off,
    /// Count what would be sent; write nothing, call nothing.
    DryRun,
    /// Send, inside the referral credential's own window.
    Live(SendBudget),
    /// The referral rail stopped the stream for the rest of this run.
    Stopped,
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

/// Which stream a call is about, and the rail object (credential) it goes
/// through.
#[derive(Clone, Copy)]
struct On<'a> {
    stream: PayoutStream,
    rail: &'a dyn PayoutRail,
}

/// The referral stream a pass carries.
struct ReferralStream {
    rail: Option<Arc<dyn PayoutRail>>,
    settings: ReferralStreamSettings,
}

/// One rail's payout pass.
pub struct PayoutPass {
    rail: Arc<dyn PayoutRail>,
    payouts: Arc<dyn PayoutStore>,
    corpus: Arc<dyn Database>,
    opener: Arc<dyn PayoutRecipientOpener>,
    settings: PayoutPassSettings,
    referral: Option<ReferralStream>,
    actor: String,
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
            referral: None,
            actor,
        }
    }

    /// Adds the referral stream. `rail` is the same rail driven by the
    /// referral credential; a `live` stream needs it (configuration refuses
    /// a live stream without one, and the pass, given none, sends no
    /// referral line and counts `referral_rail_unconfigured`). A rail with a
    /// different id is refused the same way: the stream never pays another
    /// rail's links.
    pub fn with_referral_stream(
        mut self,
        rail: Option<Arc<dyn PayoutRail>>,
        settings: ReferralStreamSettings,
    ) -> Self {
        let rail = rail.filter(|rail| rail.descriptor().rail_id == self.rail.descriptor().rail_id);
        self.referral = Some(ReferralStream { rail, settings });
        self
    }

    pub fn rail_id(&self) -> &RailId {
        &self.rail.descriptor().rail_id
    }

    pub fn mode(&self) -> PayoutMode {
        self.settings.mode
    }

    /// The referral stream's mode in a run: its own, never above the pass's.
    pub fn referral_mode(&self) -> PayoutMode {
        let Some(referral) = self.referral.as_ref() else {
            return PayoutMode::Disabled;
        };
        match (self.settings.mode, referral.settings.mode) {
            (PayoutMode::Disabled, _) | (_, PayoutMode::Disabled) => PayoutMode::Disabled,
            (PayoutMode::Live, PayoutMode::Live) => PayoutMode::Live,
            _ => PayoutMode::DryRun,
        }
    }

    fn earned(&self) -> On<'_> {
        On {
            stream: PayoutStream::Earned,
            rail: self.rail.as_ref(),
        }
    }

    /// The referral stream's rail, when it has one.
    fn referral_rail(&self) -> Option<On<'_>> {
        let rail = self.referral.as_ref()?.rail.as_deref()?;
        Some(On {
            stream: PayoutStream::Referral,
            rail,
        })
    }

    /// One pass. Never panics on a rail or store fault; the outcome says
    /// how it ended.
    pub async fn run_once(&self) -> PayoutPassReport {
        let mut report = PayoutPassReport::new(self.rail_id().clone(), self.settings.mode);
        report.referral_mode = self.referral_mode();
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
            referral_mode = report.referral_mode.as_label(),
            referral_sent = report.referral_sent,
            referral_confirmed = report.referral_confirmed,
            referral_stopped = report.referral_stopped.unwrap_or("none"),
            referral_dry_run_accounts = report.referral_dry_run_accounts,
            referral_dry_run_units = %report.referral_dry_run_units,
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
        let mut referral = self.referral_run(report).await;
        for tenant in tenants {
            self.run_tenant(&tenant, budget.as_mut(), &mut referral, report)
                .await?;
        }
        Ok(())
    }

    /// The referral stream's state for this run. A `live` stream reads the
    /// referral credential's own capacity once; a refusal there, or sending
    /// off, stops the referral stream alone, never the earned one.
    async fn referral_run(&self, report: &mut PayoutPassReport) -> ReferralRun {
        match self.referral_mode() {
            PayoutMode::Disabled => ReferralRun::Off,
            PayoutMode::DryRun => ReferralRun::DryRun,
            PayoutMode::Live => {
                let Some(on) = self.referral_rail() else {
                    report.referral_stopped = Some("referral_rail_unconfigured");
                    return ReferralRun::Stopped;
                };
                let label = match on.rail.preflight().await {
                    Ok(capacity) if capacity.sending_enabled => {
                        let limit = self.settings.local_send_limit;
                        return ReferralRun::Live(SendBudget {
                            per_send_max: capacity.per_send_max.get(),
                            remaining: capacity
                                .sends_remaining
                                .map_or(limit, |remaining| remaining.min(limit)),
                        });
                    }
                    Ok(capacity) => capacity
                        .stop_label
                        .map(|label| safe(RailRefusal::StopPass(label)))
                        .unwrap_or(REFERRAL_SENDING_DISABLED),
                    Err(refusal) => safe(refusal),
                };
                report.referral_stopped = Some(label);
                ReferralRun::Stopped
            }
        }
    }

    async fn run_tenant(
        &self,
        tenant: &str,
        mut budget: Option<&mut SendBudget>,
        referral: &mut ReferralRun,
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
                    self.run_account(tenant, account, links, budget, referral, report)
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
        if self.referral_mode() != PayoutMode::Disabled {
            self.count_referral_dry_run(tenant, account, &link, report)
                .await?;
        }
        if let Some(label) = self
            .standing_hold(tenant, account, PayoutStream::Earned)
            .await?
        {
            report.skip(label);
            return Ok(());
        }
        let owed = store(
            self.payouts
                .payout_owed(tenant, account, self.rail_id())
                .await,
        )?;
        let Some(owed) = owed.net() else {
            report.skip(PAYOUT_OWED_INCONSISTENT);
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

    /// A referral stream in `dry_run`: what this account's referral program
    /// would be sent, from stored rows only.
    async fn count_referral_dry_run(
        &self,
        tenant: &str,
        account: Uuid,
        link: &PayoutLink,
        report: &mut PayoutPassReport,
    ) -> Step<()> {
        if let Some(label) = self
            .standing_hold(tenant, account, PayoutStream::Referral)
            .await?
        {
            report.skip(stream_hold_label(PayoutStream::Referral, label));
            return Ok(());
        }
        let owed = store(
            self.payouts
                .payout_referral_owed(tenant, account, self.rail_id())
                .await,
        )?;
        let Some(owed) = owed.net else {
            report.skip(REFERRAL_OWED_INCONSISTENT);
            return Ok(());
        };
        if owed > self.referral_ceiling() {
            report.skip(REFERRAL_OVER_CEILING);
            return Ok(());
        }
        let closed = store(
            self.payouts
                .payout_closed_links_final_paid_referral(tenant, account, self.rail_id())
                .await,
        )?;
        let confirmed = store(
            self.payouts
                .payout_link_stream_confirmed_total(tenant, link.link_id, PayoutStream::Referral)
                .await,
        )?;
        if let Some(target) = stream_target(owed, closed, confirmed) {
            report.referral_dry_run_accounts += 1;
            report.referral_dry_run_units = report
                .referral_dry_run_units
                .saturating_add(target - confirmed);
        }
        Ok(())
    }

    fn referral_ceiling(&self) -> u128 {
        self.referral
            .as_ref()
            .map_or(0, |referral| referral.settings.account_ceiling.get())
    }

    /// The shared standing (closed, credit hold) or an open hold on this
    /// rail that holds `stream`, as a label.
    async fn standing_hold(
        &self,
        tenant: &str,
        account: Uuid,
        stream: PayoutStream,
    ) -> Step<Option<&'static str>> {
        let standing = store(self.payouts.payout_account_standing(tenant, account).await)?;
        if let Some(label) = standing.hold_label() {
            return Ok(Some(label));
        }
        Ok(self
            .stream_held(tenant, account, stream)
            .await?
            .then_some(match stream {
                PayoutStream::Earned => "payout_account_held",
                PayoutStream::Referral => "referral_account_held",
            }))
    }

    /// Whether an open hold on this rail holds `stream`.
    async fn stream_held(&self, tenant: &str, account: Uuid, stream: PayoutStream) -> Step<bool> {
        let holds = store(
            self.payouts
                .open_payout_hold_labels(tenant, account, self.rail_id())
                .await,
        )?;
        Ok(holds
            .iter()
            .any(|label| hold_applies_to_stream(stream, label)))
    }

    /// Opens a hold on `stream`: the label as [`stream_hold_label`] maps it.
    async fn hold(
        &self,
        tenant: &str,
        account: Uuid,
        stream: PayoutStream,
        label: &'static str,
        report: &mut PayoutPassReport,
    ) -> Step<()> {
        let label = stream_hold_label(stream, label);
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
        referral: &mut ReferralRun,
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
                .close_link(
                    tenant,
                    account,
                    &link,
                    waiting.as_ref(),
                    budget,
                    referral,
                    report,
                )
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

        let Some(link) = active else {
            return Ok(());
        };
        self.pay(tenant, account, &link, budget, report).await?;
        // The referral stream, after the earned one and independent of it:
        // the earned send's outcome never decides whether this one runs.
        match referral {
            ReferralRun::Off | ReferralRun::Stopped => Ok(()),
            ReferralRun::DryRun => {
                self.count_referral_dry_run(tenant, account, &link, report)
                    .await
            }
            ReferralRun::Live(referral_budget) => {
                // Re-read: the earned send may have marked the recipient.
                let link = self.reload_live(tenant, account).await?.unwrap_or(link);
                match self
                    .pay_referral(tenant, account, &link, referral_budget, report)
                    .await
                {
                    Err(Halt::Stop(label)) => {
                        self.stop_referral(referral, label, report);
                        Ok(())
                    }
                    other => other,
                }
            }
        }
    }

    /// Stops the referral stream for the rest of this run, changing no row.
    fn stop_referral(
        &self,
        referral: &mut ReferralRun,
        label: &'static str,
        report: &mut PayoutPassReport,
    ) {
        tracing::warn!(
            rail = %self.rail_id(),
            label,
            "payout referral stream stopped for this pass"
        );
        report.referral_stopped = Some(label);
        *referral = ReferralRun::Stopped;
    }

    /// Resolves a closing link by reads only and closes it, activating
    /// `successor` in the same transaction. False when it cannot close yet.
    /// A link with a referral program has both programs read and both
    /// streams' pending lines resolved before it closes.
    #[allow(clippy::too_many_arguments)]
    async fn close_link(
        &self,
        tenant: &str,
        account: Uuid,
        link: &PayoutLink,
        successor: Option<&PayoutLink>,
        budget: &mut SendBudget,
        referral: &mut ReferralRun,
        report: &mut PayoutPassReport,
    ) -> Step<bool> {
        let Some(recipient) = self.opener.open(link) else {
            self.hold(
                tenant,
                account,
                PayoutStream::Earned,
                PAYOUT_LINK_UNREADABLE,
                report,
            )
            .await?;
            return Ok(false);
        };
        budget.spend();
        let Some(total) = self
            .settle_stream_for_close(
                tenant,
                account,
                link,
                self.earned(),
                &link.program,
                &recipient,
                report,
            )
            .await?
        else {
            return Ok(false);
        };
        let referral_total = match link.referral_program.as_deref() {
            None => None,
            Some(program) => {
                // Read through the referral credential while its stream
                // runs; otherwise through the rail's own, which reads any
                // program of our org (a zero send moves nothing).
                let (rail, referral_budget, through_referral) =
                    match (self.referral_rail(), &mut *referral) {
                        (Some(on), ReferralRun::Live(referral_budget)) => {
                            (on.rail, referral_budget, true)
                        }
                        _ => (self.rail.as_ref(), &mut *budget, false),
                    };
                if referral_budget.remaining == 0 {
                    report.skip(PAYOUT_SEND_WINDOW_EXHAUSTED);
                    return Ok(false);
                }
                referral_budget.spend();
                let on = On {
                    stream: PayoutStream::Referral,
                    rail,
                };
                match self
                    .settle_stream_for_close(tenant, account, link, on, program, &recipient, report)
                    .await
                {
                    Ok(Some(total)) => Some(total),
                    Ok(None) => return Ok(false),
                    // The referral credential stopped: its stream stops,
                    // the link waits for a later pass, the earned stream
                    // goes on.
                    Err(Halt::Stop(label)) if through_referral => {
                        self.stop_referral(referral, label, report);
                        return Ok(false);
                    }
                    Err(halt) => return Err(halt),
                }
            }
        };
        let closed = store(
            self.payouts
                .close_payout_link_streams(
                    tenant,
                    link.link_id,
                    AtomicUnits::from_raw(total),
                    referral_total.map(AtomicUnits::from_raw),
                    successor.map(|successor| successor.link_id),
                    &self.actor,
                    None,
                )
                .await,
        )?;
        Ok(closed == PayoutLinkClose::Closed)
    }

    /// One stream's part of a close: the program's total, by a zero read
    /// (or, the recipient gone, from our own history), and the stream's
    /// pending line resolved against it by reads only. `None` when the link
    /// cannot close yet.
    #[allow(clippy::too_many_arguments)]
    async fn settle_stream_for_close(
        &self,
        tenant: &str,
        account: Uuid,
        link: &PayoutLink,
        on: On<'_>,
        program: &str,
        recipient: &RecipientRef,
        report: &mut PayoutPassReport,
    ) -> Step<Option<u128>> {
        let total = match on.rail.read_total(recipient, program).await {
            Ok(total) => total.get(),
            // The old recipient is gone: the program total comes from our
            // own history (spec section 5, "Old org gone").
            Err(RailRefusal::RecipientUnavailable) => {
                match on.rail.history(recipient, program).await {
                    Ok(entries) => {
                        let sum = entries
                            .iter()
                            .try_fold(0_u128, |sum, entry| sum.checked_add(entry.delta.get()));
                        match sum {
                            Some(sum) => sum,
                            None => return Ok(None),
                        }
                    }
                    Err(RailRefusal::StopPass(label)) => {
                        return Err(Halt::Stop(safe(RailRefusal::StopPass(label))));
                    }
                    Err(_) => return Ok(None),
                }
            }
            Err(RailRefusal::StopPass(label)) => {
                return Err(Halt::Stop(safe(RailRefusal::StopPass(label))));
            }
            Err(_) => return Ok(None),
        };
        if let Some(line) = store(
            self.payouts
                .pending_payout_line_for_stream(tenant, link.link_id, on.stream)
                .await,
        )? {
            if total >= line.target_total.get() {
                let line_ref = payout_line_ref_hash(line.line_id);
                let Some(rail_line) = rail_line(&line_ref, recipient, program, &line) else {
                    return Ok(None);
                };
                match self
                    .match_receipt(tenant, on, &line, &rail_line, report)
                    .await?
                {
                    ReceiptMatch::Confirmed => {}
                    ReceiptMatch::Unmatched => {
                        self.end_line(
                            tenant,
                            account,
                            on.stream,
                            &line,
                            PAYOUT_RECOVERY_UNMATCHED,
                            report,
                        )
                        .await?;
                    }
                    ReceiptMatch::Refused(RailRefusal::StopPass(label)) => {
                        return Err(Halt::Stop(safe(RailRefusal::StopPass(label))));
                    }
                    ReceiptMatch::Refused(_) => return Ok(None),
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
        Ok(Some(total))
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
                .hold(
                    tenant,
                    account,
                    PayoutStream::Earned,
                    PAYOUT_OWED_INCONSISTENT,
                    report,
                )
                .await;
        };
        // The lifetime ceiling per account on this rail: everything this
        // account would have been sent across every link, checked before
        // any call to the rail. An operator lifts the hold by raising the
        // setting (spec section 4, "Ceilings"): once owed fits, the next
        // pass clears the hold, with its audit row, and pays.
        if owed_net > self.settings.account_ceiling.get() {
            return self
                .hold(
                    tenant,
                    account,
                    PayoutStream::Earned,
                    PAYOUT_OVER_CEILING,
                    report,
                )
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
        // Referral-stream holds never hold earned sends.
        if self
            .stream_held(tenant, account, PayoutStream::Earned)
            .await?
        {
            report.skip("payout_account_held");
            return Ok(());
        }
        let Some(recipient) = self.opener.open(link) else {
            self.hold(
                tenant,
                account,
                PayoutStream::Earned,
                PAYOUT_LINK_UNREADABLE,
                report,
            )
            .await?;
            return Ok(());
        };
        let on = self.earned();

        // A pending line goes to recovery first: the same line, resent.
        if let Some(line) = store(
            self.payouts
                .pending_payout_line_for_stream(tenant, link.link_id, on.stream)
                .await,
        )? {
            if budget.remaining == 0 {
                report.skip(PAYOUT_SEND_WINDOW_EXHAUSTED);
                return Ok(());
            }
            return self
                .send_line(
                    tenant,
                    account,
                    on,
                    link,
                    &link.program,
                    &recipient,
                    &line,
                    budget,
                    report,
                )
                .await;
        }

        let closed = store(
            self.payouts
                .payout_closed_links_final_paid(tenant, account, self.rail_id())
                .await,
        )?;
        let confirmed = store(
            self.payouts
                .payout_link_stream_confirmed_total(tenant, link.link_id, on.stream)
                .await,
        )?;
        // Netting: owed below what earlier links and this one already got
        // sends nothing until later credit fills the gap.
        let Some(target_full) = stream_target(owed_net, closed, confirmed) else {
            if owed_net > 0 {
                report.skip("payout_nothing_owed");
            }
            return Ok(());
        };
        self.new_line(
            tenant,
            account,
            on,
            link,
            &link.program,
            &recipient,
            NewLine {
                target_full,
                confirmed,
                owed_count: owed.accrual_count,
            },
            budget,
            report,
        )
        .await
    }

    /// The referral stream's send for one account's active link (referral
    /// rewards spec section 5): the same bounds as an earned send, against
    /// the referral program, the referral ceiling and the referral holds.
    async fn pay_referral(
        &self,
        tenant: &str,
        account: Uuid,
        link: &PayoutLink,
        budget: &mut SendBudget,
        report: &mut PayoutPassReport,
    ) -> Step<()> {
        let Some(on) = self.referral_rail() else {
            report.skip("referral_rail_unconfigured");
            return Ok(());
        };
        if link.recipient_unavailable_at.is_some() {
            report.skip("referral_recipient_unavailable");
            return Ok(());
        }
        let standing = store(self.payouts.payout_account_standing(tenant, account).await)?;
        if let Some(label) = standing.hold_label() {
            report.skip(label);
            return Ok(());
        }
        let owed = store(
            self.payouts
                .payout_referral_owed(tenant, account, self.rail_id())
                .await,
        )?;
        let Some(owed_net) = owed.net else {
            return self
                .hold(
                    tenant,
                    account,
                    on.stream,
                    REFERRAL_OWED_INCONSISTENT,
                    report,
                )
                .await;
        };
        // Nothing ever owed and no program yet: no row to read, no call.
        if owed.bonus_count == 0 && link.referral_program.is_none() {
            return Ok(());
        }
        // The per-referrer ceiling, over everything this referrer would
        // have been sent from referrals across every link. Like the earned
        // ceiling, it lifts itself once owed fits again.
        if owed_net > self.referral_ceiling() {
            return self
                .hold(tenant, account, on.stream, REFERRAL_OVER_CEILING, report)
                .await;
        }
        if store(
            self.payouts
                .clear_payout_hold(
                    tenant,
                    account,
                    self.rail_id(),
                    REFERRAL_OVER_CEILING,
                    &self.actor,
                )
                .await,
        )? {
            report.skip("referral_over_ceiling_cleared");
        }
        if self.stream_held(tenant, account, on.stream).await? {
            report.skip("referral_account_held");
            return Ok(());
        }
        let Some(recipient) = self.opener.open(link) else {
            self.hold(tenant, account, on.stream, PAYOUT_LINK_UNREADABLE, report)
                .await?;
            return Ok(());
        };
        // The stored program wins; a link without one gets the rail's
        // derivation, which the line insert then sets on the link.
        let Some(program) = link
            .referral_program
            .clone()
            .or_else(|| on.rail.referral_program_for_link(link.link_id))
            .filter(|program| *program != link.program)
        else {
            report.skip(REFERRAL_STREAM_UNSUPPORTED);
            return Ok(());
        };

        if let Some(line) = store(
            self.payouts
                .pending_payout_line_for_stream(tenant, link.link_id, on.stream)
                .await,
        )? {
            if budget.remaining == 0 {
                report.skip(PAYOUT_SEND_WINDOW_EXHAUSTED);
                return Ok(());
            }
            return self
                .send_line(
                    tenant, account, on, link, &program, &recipient, &line, budget, report,
                )
                .await;
        }

        let closed = store(
            self.payouts
                .payout_closed_links_final_paid_referral(tenant, account, self.rail_id())
                .await,
        )?;
        let confirmed = store(
            self.payouts
                .payout_link_stream_confirmed_total(tenant, link.link_id, on.stream)
                .await,
        )?;
        let Some(target_full) = stream_target(owed_net, closed, confirmed) else {
            if owed_net > 0 {
                report.skip("referral_nothing_owed");
            }
            return Ok(());
        };
        self.new_line(
            tenant,
            account,
            on,
            link,
            &program,
            &recipient,
            NewLine {
                target_full,
                confirmed,
                owed_count: owed.bonus_count,
            },
            budget,
            report,
        )
        .await
    }

    /// Writes a new pending line on `on.stream` and sends it: the zero read
    /// of the program total, the per-send and wire bounds, the kill switch,
    /// then the line, committed before the send.
    #[allow(clippy::too_many_arguments)]
    async fn new_line(
        &self,
        tenant: &str,
        account: Uuid,
        on: On<'_>,
        link: &PayoutLink,
        program: &str,
        recipient: &RecipientRef,
        next: NewLine,
        budget: &mut SendBudget,
        report: &mut PayoutPassReport,
    ) -> Step<()> {
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
        let program_total = match on.rail.read_total(recipient, program).await {
            Ok(total) => total.get(),
            Err(refusal) => {
                return self
                    .refused_without_line(tenant, account, on.stream, link, refusal, report)
                    .await;
            }
        };
        // Every confirmed line is on the program and nothing else is: a
        // different total is an outside send, or a bug. Hold for an operator.
        if program_total != next.confirmed {
            return self
                .hold(
                    tenant,
                    account,
                    on.stream,
                    PAYOUT_PROGRAM_TOTAL_MISMATCH,
                    report,
                )
                .await;
        }
        let target = capped_stream_target(next.target_full, program_total, budget.per_send_max);
        if on
            .rail
            .descriptor()
            .wire_max
            .is_some_and(|max| target > max.get())
        {
            return self
                .hold(
                    tenant,
                    account,
                    on.stream,
                    PAYOUT_AMOUNT_OUT_OF_RANGE,
                    report,
                )
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
            owed_through_accrual_count: next.owed_count,
            stream: on.stream,
            referral_program: (on.stream == PayoutStream::Referral).then(|| program.to_owned()),
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
            PendingPayoutLineInsert::ReferralProgramMismatch => {
                return self
                    .hold(
                        tenant,
                        account,
                        on.stream,
                        REFERRAL_PROGRAM_MISMATCH,
                        report,
                    )
                    .await;
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
            stream: write.stream,
        };
        self.send_line(
            tenant, account, on, link, program, recipient, &line, budget, report,
        )
        .await
    }

    /// Sends one pending line (new or recovered) and records the outcome.
    #[allow(clippy::too_many_arguments)]
    async fn send_line(
        &self,
        tenant: &str,
        account: Uuid,
        on: On<'_>,
        link: &PayoutLink,
        program: &str,
        recipient: &RecipientRef,
        line: &PayoutLine,
        budget: &mut SendBudget,
        report: &mut PayoutPassReport,
    ) -> Step<()> {
        // The kill switch, re-read before every send.
        self.check_halt().await?;
        let line_ref = payout_line_ref_hash(line.line_id);
        let Some(rail_line) = rail_line(&line_ref, recipient, program, line) else {
            return self
                .fail_line(
                    tenant,
                    account,
                    on.stream,
                    line,
                    PAYOUT_LINE_INVALID,
                    report,
                )
                .await;
        };
        budget.spend();
        report.count_sent(on.stream);
        match on.rail.send(&rail_line).await {
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
                    PayoutLineResolve::Resolved => report.count_confirmed(on.stream),
                    PayoutLineResolve::ReceiptAlreadyClaimed => {
                        return self
                            .end_line(
                                tenant,
                                account,
                                on.stream,
                                line,
                                PAYOUT_RECEIPT_CONFLICT,
                                report,
                            )
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
                                stream = on.stream.as_str(),
                                label = PAYOUT_RECEIPT_UNRECORDED,
                                "payout send committed on a line no longer pending"
                            );
                            return self
                                .hold(
                                    tenant,
                                    account,
                                    on.stream,
                                    PAYOUT_RECEIPT_UNRECORDED,
                                    report,
                                )
                                .await;
                        }
                    }
                }
                if total_after.is_some_and(|total| total != line.target_total) {
                    self.hold(
                        tenant,
                        account,
                        on.stream,
                        PAYOUT_PROGRAM_TOTAL_MISMATCH,
                        report,
                    )
                    .await?;
                }
                Ok(())
            }
            Ok(SendOutcome::AlreadyAtOrAbove { total }) => {
                if total < line.target_total {
                    return self
                        .end_line(
                            tenant,
                            account,
                            on.stream,
                            line,
                            PAYOUT_PROGRAM_TOTAL_MISMATCH,
                            report,
                        )
                        .await;
                }
                // The line may have landed earlier: find its record.
                match self
                    .match_receipt(tenant, on, line, &rail_line, report)
                    .await?
                {
                    ReceiptMatch::Confirmed => Ok(()),
                    ReceiptMatch::Unmatched => {
                        self.end_line(
                            tenant,
                            account,
                            on.stream,
                            line,
                            PAYOUT_RECOVERY_UNMATCHED,
                            report,
                        )
                        .await
                    }
                    ReceiptMatch::Refused(refusal) => {
                        self.refused(tenant, account, on.stream, link, line, refusal, report)
                            .await
                    }
                }
            }
            Err(refusal) => {
                self.refused(tenant, account, on.stream, link, line, refusal, report)
                    .await
            }
        }
    }

    /// Pages the rail's records on the line's program for the one that
    /// answers `line`, skipping any already claimed by another line, and
    /// confirms it. The program is the stream's own, so a record on the
    /// other stream's program never answers this line.
    async fn match_receipt(
        &self,
        tenant: &str,
        on: On<'_>,
        line: &PayoutLine,
        rail_line: &RailLine<'_>,
        report: &mut PayoutPassReport,
    ) -> Step<ReceiptMatch> {
        let mut exclude: Vec<String> = Vec::new();
        for _ in 0..MAX_RECEIPT_CANDIDATES {
            let (receipt_hash, delta) = match on.rail.find_receipt(rail_line, &exclude).await {
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
                    report.count_confirmed(on.stream);
                    return Ok(ReceiptMatch::Confirmed);
                }
                PayoutLineResolve::ReceiptAlreadyClaimed => exclude.push(receipt_hash),
                PayoutLineResolve::NotPending => return Ok(ReceiptMatch::Confirmed),
            }
        }
        Ok(ReceiptMatch::Unmatched)
    }

    /// A refusal on a line, as its variant says (spec section 3).
    #[allow(clippy::too_many_arguments)]
    async fn refused(
        &self,
        tenant: &str,
        account: Uuid,
        stream: PayoutStream,
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
            // Stop the rail (or, on the referral stream, that stream); change
            // no row, the pending line included.
            RailRefusal::StopPass(_) => Err(Halt::Stop(label)),
            RailRefusal::HoldLine(_) => {
                self.end_line(tenant, account, stream, line, label, report)
                    .await
            }
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
            RailRefusal::Failed(_) => {
                self.fail_line(tenant, account, stream, line, label, report)
                    .await
            }
        }
    }

    /// A refusal on a read made before any line was written.
    async fn refused_without_line(
        &self,
        tenant: &str,
        account: Uuid,
        stream: PayoutStream,
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
                self.hold(tenant, account, stream, label, report).await
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

    /// Ends a line `held` and holds the account on the line's stream, so
    /// the next pass does not write the same line again. The line keeps the
    /// precise label; the hold is the stream's ([`stream_hold_label`]).
    async fn end_line(
        &self,
        tenant: &str,
        account: Uuid,
        stream: PayoutStream,
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
        self.hold(tenant, account, stream, label, report).await
    }

    /// Ends a line `failed` (a fault on our side) and holds the account on
    /// the line's stream.
    async fn fail_line(
        &self,
        tenant: &str,
        account: Uuid,
        stream: PayoutStream,
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
        self.hold(tenant, account, stream, label, report).await
    }
}

/// A new line's inputs: the uncapped target, what the link's program of
/// the stream has confirmed, and the owed row count the line was priced
/// through.
struct NewLine {
    target_full: u128,
    confirmed: u128,
    owed_count: i64,
}

/// The rail's view of a stored line: its reference, the stream's program,
/// its target, and what was confirmed before it (`target - delta`).
fn rail_line<'a>(
    line_ref: &'a str,
    recipient: &'a RecipientRef,
    program: &'a str,
    line: &PayoutLine,
) -> Option<RailLine<'a>> {
    let confirmed = line.target_total.get().checked_sub(line.delta.get())?;
    RailLine::new(
        line_ref,
        recipient,
        program,
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
            PAYOUT_RECEIPT_CONFLICT,
            PAYOUT_LINE_INVALID,
            PAYOUT_OWED_INCONSISTENT,
            REFERRAL_OVER_CEILING,
            REFERRAL_PROGRAM_TOTAL_MISMATCH,
            REFERRAL_RECOVERY_UNMATCHED,
            REFERRAL_RECEIPT_UNRECORDED,
            REFERRAL_AMOUNT_OUT_OF_RANGE,
            REFERRAL_RECEIPT_CONFLICT,
            REFERRAL_LINE_INVALID,
            REFERRAL_OWED_INCONSISTENT,
            REFERRAL_PROGRAM_MISMATCH,
            REFERRAL_STREAM_HELD,
            REFERRAL_STREAM_UNSUPPORTED,
            REFERRAL_SENDING_DISABLED,
            PayoutPassOutcome::Halted.label(),
            PayoutPassOutcome::LockBusy.label(),
        ] {
            assert!(RailRefusal::StopPass(label).label_is_safe(), "{label}");
        }
    }

    /// The referral stream's holds all carry the prefix, so the earned
    /// stream never reads one as its own, and every hold the referral
    /// stream raises maps to one of them (or to the link-wide unreadable
    /// hold), so a referral fault never holds earned sends.
    #[test]
    fn referral_holds_hold_only_the_referral_stream() {
        let spec_labels = [
            "referral_same_payout_org",
            "referral_same_payout_subject",
            "referral_code_suspended",
            "referral_referrer_suspended",
            REFERRAL_OVER_CEILING,
        ];
        for label in spec_labels {
            assert!(is_referral_stream_hold(label), "{label}");
            assert!(hold_applies_to_stream(PayoutStream::Referral, label));
            assert!(!hold_applies_to_stream(PayoutStream::Earned, label));
        }
        for label in [
            PAYOUT_OVER_CEILING,
            PAYOUT_PROGRAM_TOTAL_MISMATCH,
            PAYOUT_RECOVERY_UNMATCHED,
            "payout_reversal_unpriced",
        ] {
            assert!(hold_applies_to_stream(PayoutStream::Earned, label));
            assert!(
                !hold_applies_to_stream(PayoutStream::Referral, label),
                "an earned-stream hold does not hold referral sends: {label}"
            );
        }
        // An unreadable link pays on neither stream.
        assert!(hold_applies_to_stream(
            PayoutStream::Earned,
            PAYOUT_LINK_UNREADABLE
        ));
        assert!(hold_applies_to_stream(
            PayoutStream::Referral,
            PAYOUT_LINK_UNREADABLE
        ));

        for label in [
            PAYOUT_OVER_CEILING,
            PAYOUT_AMOUNT_OUT_OF_RANGE,
            PAYOUT_PROGRAM_TOTAL_MISMATCH,
            PAYOUT_RECOVERY_UNMATCHED,
            PAYOUT_RECEIPT_UNRECORDED,
            PAYOUT_RECEIPT_CONFLICT,
            PAYOUT_LINE_INVALID,
            PAYOUT_OWED_INCONSISTENT,
            "near_ai_capacity_overflow",
            "near_ai_request_refused",
        ] {
            // Earned keeps the label.
            assert_eq!(stream_hold_label(PayoutStream::Earned, label), label);
            let mapped = stream_hold_label(PayoutStream::Referral, label);
            assert!(is_referral_stream_hold(mapped), "{label} -> {mapped}");
            assert!(!hold_applies_to_stream(PayoutStream::Earned, mapped));
            assert!(RailRefusal::StopPass(mapped).label_is_safe());
        }
        assert_eq!(
            stream_hold_label(PayoutStream::Referral, "near_ai_capacity_overflow"),
            REFERRAL_STREAM_HELD
        );
        assert_eq!(
            stream_hold_label(PayoutStream::Referral, PAYOUT_LINK_UNREADABLE),
            PAYOUT_LINK_UNREADABLE
        );
        assert_eq!(
            stream_hold_label(PayoutStream::Referral, REFERRAL_OVER_CEILING),
            REFERRAL_OVER_CEILING
        );
    }

    #[test]
    fn stream_targets_net_closed_links_and_cap_per_send() {
        // Owed 1000, nothing closed, nothing confirmed: all of it.
        assert_eq!(stream_target(1_000, 0, 0), Some(1_000));
        // A closed link already got 600: this link's program needs 400.
        assert_eq!(stream_target(1_000, 600, 0), Some(400));
        // Already confirmed 400 on this link: nothing more.
        assert_eq!(stream_target(1_000, 600, 400), None);
        // Owed fell below what was sent (a reversal or void): nothing, and
        // never a negative or wrapped target.
        assert_eq!(stream_target(500, 600, 0), None);
        assert_eq!(stream_target(0, 0, 0), None);
        assert_eq!(stream_target(u128::MAX, 0, u128::MAX - 1), Some(u128::MAX));

        // The cap: program total plus the per-send maximum.
        assert_eq!(capped_stream_target(1_000, 0, 300), 300);
        assert_eq!(capped_stream_target(1_000, 900, 300), 1_000);
        assert_eq!(capped_stream_target(1_000, u128::MAX - 1, 300), 1_000);
        assert_eq!(capped_stream_target(5, 0, 0), 0);
    }

    #[test]
    fn stream_status_waits_on_owed_less_sent_and_shows_the_stream_hold() {
        assert_eq!(stream_waiting(1_000, 400), 600);
        assert_eq!(stream_waiting(400, 1_000), 0);
        assert_eq!(stream_waiting(0, 0), 0);

        let open = vec![
            "referral_same_payout_org".to_string(),
            PAYOUT_OVER_CEILING.to_string(),
        ];
        assert_eq!(
            stream_status_hold_label(PayoutStream::Earned, None, &open).as_deref(),
            Some(PAYOUT_OVER_CEILING)
        );
        assert_eq!(
            stream_status_hold_label(PayoutStream::Referral, None, &open).as_deref(),
            Some("referral_same_payout_org")
        );
        assert_eq!(
            stream_status_hold_label(PayoutStream::Referral, None, &[]),
            None
        );
        // The shared standing holds both, first.
        for stream in PayoutStream::ALL {
            assert_eq!(
                stream_status_hold_label(stream, Some("payout_account_credit_hold"), &open)
                    .as_deref(),
                Some("payout_account_credit_hold")
            );
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
            referral_program: None,
            final_paid_referral: None,
            unlink_requested_at: None,
            recipient_unavailable_at: None,
            version: 1,
        };
        assert!(UnconfiguredRecipientOpener.open(&link).is_none());
    }
}
