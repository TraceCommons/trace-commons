//! NEAR AI credit arriving in the verified payout organization, and the
//! NEAR AI balance's change over the last 24 hours.
//!
//! **Arrivals are read, never inferred.** A contributor's balance goes up when
//! credit arrives and down when Private AI spends it, so a rise in the balance
//! cannot tell an arrival from a quiet hour. cloud-api keeps a ledger of
//! credit transfers, and an owner session can read the transfers INTO its
//! organization (`GET /v1/organizations/{org}/credit/transfers?direction=incoming`,
//! `credits:read`). Every row there is credit that arrived; each is counted
//! once, by its id. Spending never appears in it, so spending can never look
//! like an arrival.
//!
//! **The 24-hour change is the balance's own.** It compares what is left now
//! with what was left about a day ago, from samples this daemon took. It goes
//! up with an arrival and down with spending, which is what it is meant to
//! say. With no sample from about a day ago there is no change to report, and
//! nothing is reported -- never a zero.
//!
//! **What is kept** (`near-ai-credit-arrivals.json`, beside the spending
//! organization record): the organization's id, balance samples for 48 hours,
//! the running total received, the total as of the last notification, and the
//! ids and times of recent transfers so none is counted twice. No names, no
//! memo, no sender. It is forgotten with the NEAR AI sign-in, and reset when
//! the linked payout recipient changes. Private AI balance samples are kept
//! separately, so reading a different spending organization cannot reset arrivals.
//!
//! **Sessions.** Every read here goes through the balance module's lock
//! ([`balance::with_management_session`] and [`balance::read`]), so a poll can
//! never race a balance read for the refresh token, which rotates on every
//! exchange.

use super::api::{CloudApi, IncomingTransfer, TransferCursor};
use super::balance::{self, BalanceReport};
use super::loopback::SessionTokens;
use crate::config::ConfigStore;
use crate::daemon::ipc::DaemonShared;
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

/// The record's file, in the daemon's directory.
pub const ARRIVALS_FILE: &str = "near-ai-credit-arrivals.json";

/// How long balance samples are kept. Twice the comparison window, so a
/// sample from a day ago is still there when the daemon was off for a while.
pub const SAMPLE_RETENTION: Duration = Duration::hours(48);

/// The closest together two kept samples may be. A balance read every
/// minute keeps one sample per spacing, not sixty.
pub const SAMPLE_SPACING: Duration = Duration::minutes(10);

/// What "the last 24 hours" compares against.
pub const CHANGE_WINDOW: Duration = Duration::hours(24);

/// How much older than [`CHANGE_WINDOW`] the comparison sample may be. A
/// Mac that was asleep at this time yesterday compares with the last
/// reading before then, as long as it is no older than this allows; past
/// it, there is nothing from "about a day ago" and no change is reported.
pub const CHANGE_TOLERANCE: Duration = Duration::hours(6);

/// How far back each poll re-reads the transfer ledger, before the newest
/// transfer it has already seen. A transfer committed late with an earlier
/// time is still found; anything this old was seen by an earlier poll.
pub const LOOKBACK: Duration = Duration::days(2);

/// How often the daemon reads the ledger and the balance on its own. DRAFT,
/// owner decision: each read past five minutes exchanges the refresh token
/// once, so this is kept well apart.
pub const POLL_INTERVAL: Duration = Duration::minutes(30);

/// The most pages one poll reads. 100 transfers a page.
pub const MAX_PAGES: usize = 20;

/// A notification needs at least this much new credit since the last one
/// ($1, in nano-dollars). DRAFT, owner decision 3 (2026-10-23).
pub const NOTIFY_MIN_NANOS: i64 = 1_000_000_000;

/// And at least this fraction of everything received as of the last one,
/// in percent (25%). DRAFT, owner decision 3 (2026-10-23).
pub const NOTIFY_GROWTH_PERCENT: i64 = 25;

/// The most samples kept, whatever the clock does.
const MAX_SAMPLES: usize = 400;
/// The most transfer ids kept.
const MAX_SEEN: usize = 2_000;
/// A record larger than this is not one this build wrote.
const MAX_FILE_BYTES: usize = 512 * 1024;

/// One balance reading, kept for the 24-hour comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BalanceSample {
    pub(crate) at: DateTime<Utc>,
    pub(crate) remaining_nanos: i64,
}

/// One transfer already counted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SeenTransfer {
    pub(crate) id: String,
    pub(crate) at: DateTime<Utc>,
}

/// An incoming amount and its ledger time. No sender, memo or account data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ReceivedTransfer {
    at: DateTime<Utc>,
    amount: i64,
}

/// A complete rolling-window read, independent of the notification baseline.
/// Older record versions have no window until a successful ledger scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReceivedWindow {
    checked_at: DateTime<Utc>,
    transfers: Vec<ReceivedTransfer>,
}

/// Everything kept about one organization's credit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct ArrivalsRecord {
    /// The organization every figure below belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) organization_id: Option<String>,
    /// The confirmed link generation whose ledger produced this record.
    /// Absent on old records until a fresh verified poll binds them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) recipient_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) samples: Vec<BalanceSample>,
    /// Every transfer counted so far, added up, in nano-dollars.
    #[serde(default)]
    pub(crate) received_total_nanos: i64,
    /// Whether one whole read of the ledger has finished. Until it has,
    /// [`received_total_nanos`](Self::received_total_nanos) is not a total.
    #[serde(default)]
    pub(crate) scanned: bool,
    /// The received total as of the last notification; `None` before the
    /// first one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) notified_received_total_nanos: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) seen: Vec<SeenTransfer>,
    /// The newest transfer time counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) high_water: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) received_window: Option<ReceivedWindow>,
}

/// Whether credit received is worth a notification: `Some(amount)` to
/// announce, in nano-dollars, or `None`.
///
/// The first credit ever received always is. After that, the credit received
/// since the last notification must reach both [`NOTIFY_MIN_NANOS`] and
/// [`NOTIFY_GROWTH_PERCENT`] of everything received as of that notification.
/// Integer arithmetic only.
#[must_use]
pub fn credit_candidacy(received_total: i64, notified_total: Option<i64>) -> Option<i64> {
    if received_total <= 0 {
        return None;
    }
    let Some(baseline) = notified_total else {
        return Some(received_total);
    };
    let grown = i128::from(received_total) - i128::from(baseline);
    let enough = grown >= i128::from(NOTIFY_MIN_NANOS)
        && grown * 100 >= i128::from(baseline.max(0)) * i128::from(NOTIFY_GROWTH_PERCENT);
    enough.then(|| i64::try_from(grown).unwrap_or(i64::MAX))
}

impl ArrivalsRecord {
    /// Make this the record of `organization_id`. A different organization
    /// starts a new record: its balance and its transfers are not this one's.
    pub(crate) fn adopt(&mut self, organization_id: &str) {
        if self.organization_id.as_deref() != Some(organization_id) {
            *self = Self {
                organization_id: Some(organization_id.to_owned()),
                ..Self::default()
            };
        }
    }

    /// Keep a balance reading taken at `at`. A `None` (no limit set, so no
    /// remaining figure) is not a reading and is not kept.
    pub(crate) fn record_sample(&mut self, at: DateTime<Utc>, remaining_nanos: Option<i64>) {
        let Some(remaining_nanos) = remaining_nanos else {
            return;
        };
        // A newer reading inside the spacing is not kept, rather than
        // replacing the last one: a replaced sample would slide forward with
        // every read and never grow a day old.
        if self
            .samples
            .last()
            .is_some_and(|last| at - last.at < SAMPLE_SPACING)
        {
            return;
        }
        self.samples.push(BalanceSample {
            at,
            remaining_nanos,
        });
        self.samples.retain(|s| s.at >= at - SAMPLE_RETENTION);
        if self.samples.len() > MAX_SAMPLES {
            let excess = self.samples.len() - MAX_SAMPLES;
            self.samples.drain(..excess);
        }
    }

    /// What is left now minus what was left about a day before `now`, or
    /// `None` when either is unknown.
    #[must_use]
    pub(crate) fn change_24h(&self, now: DateTime<Utc>, remaining_now: Option<i64>) -> Option<i64> {
        let remaining_now = remaining_now?;
        let latest = now - CHANGE_WINDOW;
        let earliest = latest - CHANGE_TOLERANCE;
        let then = self
            .samples
            .iter()
            .filter(|s| s.at <= latest && s.at >= earliest)
            .max_by_key(|s| s.at)?;
        remaining_now.checked_sub(then.remaining_nanos)
    }

    /// The oldest transfer time the next poll needs to read back to, or
    /// `None` to read the whole ledger.
    #[must_use]
    pub(crate) fn scan_cutoff(&self) -> Option<DateTime<Utc>> {
        self.high_water.map(|at| at - LOOKBACK)
    }

    /// Count one finished read of the ledger. Each transfer is counted once,
    /// by id; one older than [`Self::scan_cutoff`] was read by an earlier
    /// poll and is skipped. Answers the credit newly counted.
    pub(crate) fn apply_transfers(&mut self, rows: &[IncomingTransfer]) -> i64 {
        let cutoff = self.scan_cutoff();
        let mut added: i64 = 0;
        for row in rows {
            if row.amount <= 0
                || cutoff.is_some_and(|cutoff| row.created_at < cutoff)
                || self.seen.iter().any(|seen| seen.id == row.id)
            {
                continue;
            }
            added = added.saturating_add(row.amount);
            self.received_total_nanos = self.received_total_nanos.saturating_add(row.amount);
            self.seen.push(SeenTransfer {
                id: row.id.clone(),
                at: row.created_at,
            });
            self.high_water = Some(
                self.high_water
                    .map_or(row.created_at, |high| high.max(row.created_at)),
            );
        }
        // Keep the ids the next poll can read back over, with a day to
        // spare, and never more than the cap.
        if let Some(keep_from) = self.scan_cutoff().map(|cutoff| cutoff - Duration::days(1)) {
            self.seen.retain(|seen| seen.at >= keep_from);
        }
        if self.seen.len() > MAX_SEEN {
            self.seen.sort_by_key(|seen| seen.at);
            let excess = self.seen.len() - MAX_SEEN;
            self.seen.drain(..excess);
        }
        self.scanned = true;
        added
    }

    /// Replace the window only after a complete fetch covering at least the
    /// preceding 24 hours. Rebuilding from ledger rows also hydrates old files
    /// whose deduplication entries have timestamps but no amounts.
    fn record_received_window(&mut self, rows: &[IncomingTransfer], checked_at: DateTime<Utc>) {
        let mut ids = HashSet::new();
        let transfers: Vec<_> = rows
            .iter()
            .filter(|row| {
                row.amount > 0
                    && row.created_at > checked_at - CHANGE_WINDOW
                    && row.created_at <= checked_at
                    && ids.insert(&row.id)
            })
            .take(MAX_SEEN + 1)
            .map(|row| ReceivedTransfer {
                at: row.created_at,
                amount: row.amount,
            })
            .collect();
        self.received_window = (transfers.len() <= MAX_SEEN).then_some(ReceivedWindow {
            checked_at,
            transfers,
        });
    }

    /// Incoming credit within (now - 24h, now], without balance/spending.
    /// A missing, stale or future-dated scan is unknown, never a zero. The
    /// display is as current as the poll cadence; expired transfers age out
    /// even before the next network read.
    fn received_24h(&self, now: DateTime<Utc>) -> Option<i64> {
        let window = self.received_window.as_ref()?;
        if now < window.checked_at
            || now - window.checked_at > POLL_INTERVAL
            || window.transfers.len() > MAX_SEEN
            || window.transfers.iter().any(|row| row.amount <= 0)
        {
            return None;
        }
        window
            .transfers
            .iter()
            .filter(|row| row.at > now - CHANGE_WINDOW && row.at <= now)
            .try_fold(0i64, |sum, row| sum.checked_add(row.amount))
    }

    /// What a notification would announce now, if anything.
    #[must_use]
    pub(crate) fn candidacy(&self) -> Option<i64> {
        if !self.scanned {
            return None;
        }
        credit_candidacy(
            self.received_total_nanos,
            self.notified_received_total_nanos,
        )
    }
}

// ---- the record, in memory and on disk ----

/// Each directory's record, loaded once and written through. The arbiter
/// reads it every tick, so it is read from here and not from the file.
///
/// A leaf lock: nothing is taken while it is held.
fn records() -> &'static Mutex<HashMap<PathBuf, ArrivalsRecord>> {
    static RECORDS: OnceLock<Mutex<HashMap<PathBuf, ArrivalsRecord>>> = OnceLock::new();
    RECORDS.get_or_init(Default::default)
}

/// Spending-balance samples are independent of the payout recipient. The
/// two organizations may differ and neither read may reset the other's data.
const BALANCE_SAMPLES_FILE: &str = "near-ai-balance-samples.json";

fn balance_records() -> &'static Mutex<HashMap<PathBuf, ArrivalsRecord>> {
    static RECORDS: OnceLock<Mutex<HashMap<PathBuf, ArrivalsRecord>>> = OnceLock::new();
    RECORDS.get_or_init(Default::default)
}

fn balance_snapshot(store: &ConfigStore) -> ArrivalsRecord {
    balance_records()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(store.dir().to_path_buf())
        .or_insert_with(|| load_file(store, BALANCE_SAMPLES_FILE))
        .clone()
}

/// The record on disk. Missing, oversized or unreadable reads as empty: it
/// holds nothing that cannot be read again from the service, and the cost of
/// starting over is at most one notification for credit already announced.
fn load(store: &ConfigStore) -> ArrivalsRecord {
    load_file(store, ARRIVALS_FILE)
}

fn load_file(store: &ConfigStore, file: &str) -> ArrivalsRecord {
    match store.read_daemon_file(file) {
        Ok(Some(bytes)) if bytes.len() <= MAX_FILE_BYTES => {
            serde_json::from_slice(&bytes).unwrap_or_default()
        }
        _ => ArrivalsRecord::default(),
    }
}

/// A copy of this directory's record.
pub(crate) fn snapshot(store: &ConfigStore) -> ArrivalsRecord {
    records()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(store.dir().to_path_buf())
        .or_insert_with(|| load(store))
        .clone()
}

/// Change the record and write it through, or do nothing when no NEAR AI
/// sign-in is kept: under the credential commit lock, so a forget cannot
/// land between the check and the write and have a poll bring the record
/// back.
fn update_record<R>(
    shared: &DaemonShared,
    records: &Mutex<HashMap<PathBuf, ArrivalsRecord>>,
    file: &str,
    change: impl FnOnce(&mut ArrivalsRecord) -> R,
) -> Option<R> {
    update_record_checked(shared, records, file, None, change)
}

fn update_record_checked<R>(
    shared: &DaemonShared,
    records: &Mutex<HashMap<PathBuf, ArrivalsRecord>>,
    file: &str,
    expected_recipient: Option<Option<&str>>,
    change: impl FnOnce(&mut ArrivalsRecord) -> R,
) -> Option<R> {
    let store = &shared.store;
    let locks = super::session::coordination(store.dir()).ok()?;
    let _commit = locks.commit.lock().ok()?;
    shared
        .settings
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .near_ai_session
        .as_ref()?;
    let apply = || {
        let mut map = records.lock().unwrap_or_else(|e| e.into_inner());
        let record = map
            .entry(store.dir().to_path_buf())
            .or_insert_with(|| load_file(store, file));
        let out = change(record);
        let written = serde_json::to_vec(record)
            .ok()
            .is_some_and(|bytes| store.write_daemon_file(file, &bytes).is_ok());
        if !written {
            tracing::warn!(target: "near_ai_credit", "near_ai_credit_record_not_written");
        }
        out
    };
    match expected_recipient {
        Some(expected) => {
            crate::daemon::payout_link::with_near_ai_recipient_ref(shared, expected, apply)
        }
        None => Some(apply()),
    }
}

/// Forget the record. The caller holds the credential commit lock: this is
/// the forget path's. Best effort, like the spending organization's record.
pub(crate) fn clear_locked(store: &ConfigStore) {
    for (records, file) in [
        (records(), ARRIVALS_FILE),
        (balance_records(), BALANCE_SAMPLES_FILE),
    ] {
        records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(store.dir());
        match std::fs::remove_file(store.daemon_path(file)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => tracing::warn!(target: "near_ai_credit", "near_ai_credit_record_not_cleared"),
        }
    }
}

/// Keep a balance just read for `organization_id` as a sample. Called by the
/// balance read itself, so every read -- the app's and the poll's -- counts.
pub(crate) fn note_balance(
    shared: &DaemonShared,
    organization_id: Option<&str>,
    reading: &balance::BalanceReading,
) {
    let Some(organization_id) = organization_id else {
        return;
    };
    update_record(shared, balance_records(), BALANCE_SAMPLES_FILE, |record| {
        record.adopt(organization_id);
        record.record_sample(reading.observed_at, reading.remaining_nanos);
    });
}

/// A notification candidate tied to the exact payout-link generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingCredit {
    pub amount: i64,
    pub received_total: i64,
    recipient_ref: String,
}

pub(crate) fn pending_notification(shared: &DaemonShared) -> Option<PendingCredit> {
    let record = snapshot(&shared.store);
    let recipient_ref = record.recipient_ref.clone()?;
    crate::daemon::payout_link::with_near_ai_recipient_ref(shared, Some(&recipient_ref), || {
        record.candidacy().map(|amount| PendingCredit {
            amount,
            received_total: record.received_total_nanos,
            recipient_ref: recipient_ref.clone(),
        })
    })
    .flatten()
}

/// Recheck identity and publish while holding the same local link lock used
/// by confirm/unlink. A stale candidate neither publishes nor moves the
/// notification baseline on the newly linked organization.
pub(crate) fn publish_notification(
    shared: &DaemonShared,
    credit: &PendingCredit,
    publish: impl FnOnce(),
) -> bool {
    update_record_checked(
        shared,
        records(),
        ARRIVALS_FILE,
        Some(Some(&credit.recipient_ref)),
        |record| {
            if record.recipient_ref.as_deref() != Some(credit.recipient_ref.as_str())
                || !record.scanned
                || record.received_total_nanos < credit.received_total
                || record
                    .notified_received_total_nanos
                    .is_some_and(|total| total >= credit.received_total)
            {
                return false;
            }
            publish();
            record.notified_received_total_nanos = Some(credit.received_total);
            true
        },
    )
    .unwrap_or(false)
}

/// The organization Private AI spends from, when a NEAR AI sign-in is kept.
fn spending_organization(shared: &DaemonShared) -> Option<String> {
    let settings = shared.settings.lock().unwrap_or_else(|e| e.into_inner());
    settings.near_ai_session.as_ref()?;
    settings
        .near_ai_inference
        .as_ref()
        .map(|key| key.organization_id.clone())
}

// ---- the balance reply ----

/// `near_ai_balance`'s answer: the balance, and beside it the 24-hour change
/// and cumulative/rolling incoming credit for the same organization.
///
/// `change_24h_nanos` and `change_24h_text` are null with no reading from
/// about a day ago. Known zero/subcent changes have neutral copy. Incoming
/// 24-hour fields require a fresh complete ledger scan. Separate unavailable
/// copy explains unknown metrics without fabricating numeric values. No
/// summary fields are added in states other than `known`.
pub(crate) fn balance_value(shared: &DaemonShared, report: BalanceReport) -> serde_json::Value {
    let mut value = report.to_value();
    let BalanceReport::Known(reading) = report else {
        return value;
    };
    let Some(organization_id) = spending_organization(shared) else {
        return value;
    };
    let samples = balance_snapshot(&shared.store);
    let record = snapshot(&shared.store);
    let change = (samples.organization_id.as_deref() == Some(organization_id.as_str()))
        .then(|| samples.change_24h(reading.observed_at, reading.remaining_nanos))
        .flatten();
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "change_24h_unavailable_text".into(),
            crate::private_inference_copy::BALANCE_CHANGE_24H_UNAVAILABLE.into(),
        );
        object.insert(
            "received_24h_unavailable_text".into(),
            crate::private_inference_copy::CREDIT_RECEIVED_24H_UNAVAILABLE.into(),
        );
        if let Some(change) = change {
            object.insert("change_24h_nanos".into(), change.into());
            // Color reflects the displayed cents, while the raw numeric
            // amount remains precise for non-visual consumers.
            let direction = if change.unsigned_abs() < 10_000_000 {
                0
            } else {
                change.signum()
            };
            object.insert("change_24h_direction".into(), direction.into());
            let text = crate::private_inference_copy::balance_change_line(change, BALANCE_SCALE);
            if !text.is_empty() {
                object.insert("change_24h_text".into(), text.into());
                object.remove("change_24h_unavailable_text");
            }
        }
        if record.scanned
            && record.organization_id.as_deref() == Some(organization_id.as_str())
            && record.recipient_ref.as_deref().is_some_and(|reference| {
                crate::daemon::payout_link::with_near_ai_recipient_ref(
                    shared,
                    Some(reference),
                    || (),
                )
                .is_some()
            })
        {
            object.insert(
                "received_total_nanos".into(),
                record.received_total_nanos.into(),
            );
            if let Some(received) = record.received_24h(Utc::now()) {
                object.insert("received_24h_nanos".into(), received.into());
                object.insert(
                    "received_24h_text".into(),
                    crate::private_inference_copy::credit_received_24h_line(
                        received,
                        BALANCE_SCALE,
                    )
                    .into(),
                );
                if let Some(window) = &record.received_window {
                    object.insert(
                        "received_24h_checked_at".into(),
                        window.checked_at.to_rfc3339().into(),
                    );
                }
                object.remove("received_24h_unavailable_text");
            }
        }
    }
    value
}

/// The wire's scale for every amount here: nano-dollars.
const BALANCE_SCALE: u8 = 9;

// ---- the poll ----

/// When each directory last polled. In memory: a restarted daemon polls at
/// once, which is one read.
fn last_polls() -> &'static Mutex<HashMap<PathBuf, DateTime<Utc>>> {
    static LAST: OnceLock<Mutex<HashMap<PathBuf, DateTime<Utc>>>> = OnceLock::new();
    LAST.get_or_init(Default::default)
}

/// Whether a poll is due at `now`, and if so, mark it started.
fn take_poll_due_with_fresh(store: &ConfigStore, now: DateTime<Utc>, fresh: bool) -> bool {
    let mut map = last_polls().lock().unwrap_or_else(|e| e.into_inner());
    let last = map.get(store.dir()).copied();
    let interval = if fresh {
        Duration::seconds(5)
    } else {
        POLL_INTERVAL
    };
    // A clock that went backwards reads as a poll just now.
    if last.is_some_and(|last| now - last < interval && last <= now + interval) {
        return false;
    }
    map.insert(store.dir().to_path_buf(), now);
    true
}

/// One supervisor tick: read the balance and the transfer ledger when a
/// poll is due and a NEAR AI sign-in is kept. The payout recipient must be
/// verified against the commons before any incoming transfers are read.
/// Cheap otherwise. Logs labels only.
pub async fn poll(shared: &DaemonShared, now: DateTime<Utc>) {
    poll_due(shared, now, false).await;
}

/// Balance reads also initialize the receipt summary; an explicit Refresh
/// reads it again with the same five-second floor as the balance button.
/// Called after the balance read releases the management-session lock.
pub(super) async fn refresh_if_due(shared: &DaemonShared, fresh: bool) {
    poll_due(shared, Utc::now(), fresh).await;
}

async fn poll_due(shared: &DaemonShared, now: DateTime<Utc>, fresh: bool) {
    // A refresh button and the supervisor can race. Serialize complete polls
    // so an earlier snapshot cannot overwrite a newer window after awaiting
    // the session lock. No synchronous map lock crosses an await.
    static POLLS: OnceLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let lock = POLLS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(shared.store.dir().to_path_buf())
        .or_default()
        .clone();
    let _poll = lock.lock().await;
    if shared
        .settings
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .near_ai_session
        .is_none()
        || !take_poll_due_with_fresh(&shared.store, now, fresh)
    {
        return;
    }
    let Ok(api) = CloudApi::live() else {
        return;
    };
    if poll_with(shared, &api).await.is_err() {
        tracing::debug!(target: "near_ai_credit", "near_ai_credit_poll_unavailable");
    }
}

/// [`poll`] against an explicit client, without the schedule. Answers the
/// credit newly counted.
pub(crate) async fn poll_with(shared: &DaemonShared, api: &CloudApi) -> Result<i64> {
    // Spending balance sampling remains independent of payout linkage.
    if spending_organization(shared).is_some() {
        let _ = balance::read_via(shared, api).await;
    }
    let expected_recipient = crate::daemon::payout_link::near_ai_recipient_ref_snapshot(shared)
        .map_err(|_| anyhow::anyhow!("near_ai_credit_recipient_unavailable"))?;
    let verified = crate::daemon::payout_link::verified_near_ai_recipient(shared, api).await;
    let organization_id = match verified {
        Ok(Some(organization_id)) => organization_id,
        result => {
            // Preserve seen transfer IDs, but suppress notifications while
            // this generation no longer has a verified active recipient.
            let invalidated = update_record_checked(
                shared,
                records(),
                ARRIVALS_FILE,
                Some(expected_recipient.as_deref()),
                |record| {
                    let was_visible = record.recipient_ref.is_some();
                    record.recipient_ref = None;
                    record.received_window = None;
                    was_visible
                },
            )
            .unwrap_or(false);
            if invalidated {
                shared.publish(
                    crate::daemon::ipc::EVENT_NEAR_AI_BALANCE_CHANGED,
                    serde_json::json!({}),
                );
            }
            return match result {
                Ok(None) => Ok(0),
                Err(()) => Err(anyhow::anyhow!("near_ai_credit_recipient_unavailable")),
                Ok(Some(_)) => unreachable!(),
            };
        }
    };
    let checked_at = Utc::now();
    let cutoff = {
        let record = snapshot(&shared.store);
        (record.organization_id.as_deref() == Some(organization_id.as_str()))
            .then(|| {
                record
                    .scan_cutoff()
                    .map(|cutoff| cutoff.min(checked_at - CHANGE_WINDOW))
            })
            .flatten()
    };
    let rows = balance::with_management_session(shared, api, |tokens| {
        fetch_incoming(api, tokens, organization_id.clone(), cutoff)
    })
    .await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            let cleared = update_record_checked(
                shared,
                records(),
                ARRIVALS_FILE,
                Some(expected_recipient.as_deref()),
                |record| {
                    record.received_window = None;
                },
            );
            if cleared.is_some() {
                shared.publish(
                    crate::daemon::ipc::EVENT_NEAR_AI_BALANCE_CHANGED,
                    serde_json::json!({}),
                );
            }
            return Err(error);
        }
    };
    let applied = update_record_checked(
        shared,
        records(),
        ARRIVALS_FILE,
        Some(expected_recipient.as_deref()),
        |record| {
            record.adopt(&organization_id);
            record.recipient_ref = expected_recipient.clone();
            let first_read = !record.scanned;
            let added = record.apply_transfers(&rows);
            record.record_received_window(&rows, checked_at);
            // Credit already in the ledger at the first read is history -- an
            // upgrade, or a new organization -- not an arrival. It becomes the
            // baseline without a notification, so "the first credit ever" is
            // the first to land after an empty first read. DRAFT, owner review.
            if first_read && added > 0 {
                record.notified_received_total_nanos = Some(record.received_total_nanos);
            }
            added
        },
    );
    // A complete zero-arrival scan can turn unknown into zero or age a
    // previous arrival out of the window, so it also refreshes the Home tile.
    if applied.is_some() {
        shared.publish(
            crate::daemon::ipc::EVENT_NEAR_AI_BALANCE_CHANGED,
            serde_json::json!({}),
        );
    }
    Ok(applied.unwrap_or(0))
}

/// Read the ledger, newest first, back to `cutoff` (or all of it). May run
/// twice under one poll (see [`balance::with_management_session`]): it only
/// reads.
async fn fetch_incoming(
    api: &CloudApi,
    tokens: SessionTokens,
    organization_id: String,
    cutoff: Option<DateTime<Utc>>,
) -> Result<Vec<IncomingTransfer>> {
    let mut rows = Vec::new();
    let mut cursor: Option<TransferCursor> = None;
    for _ in 0..MAX_PAGES {
        let page = api
            .incoming_transfers_page(&tokens, &organization_id, cursor.as_ref())
            .await?;
        let oldest = page.transfers.last().map(|t| t.created_at);
        let done = page.transfers.is_empty()
            || cutoff
                .zip(oldest)
                .is_some_and(|(cutoff, oldest)| oldest < cutoff);
        rows.extend(page.transfers);
        match page.next_cursor {
            Some(next) if !done => cursor = Some(next),
            _ => return Ok(rows),
        }
    }
    // A bounded scan is not a complete ledger. Do not publish an undercount
    // as a total or establish a notification baseline from partial pages.
    Err(anyhow::anyhow!("near_ai_credit_scan_incomplete"))
}

/// Put `record` in place for `store`, in memory and on disk, as if earlier
/// polls had left it.
#[cfg(test)]
pub(crate) fn set_for_test(store: &ConfigStore, mut record: ArrivalsRecord) {
    if let Some(organization) = &record.organization_id {
        let key = uuid::Uuid::new_v4();
        let org = uuid::Uuid::parse_str(organization).unwrap_or_else(|_| uuid::Uuid::new_v4());
        store
            .write_daemon_file(
                crate::config::DAEMON_PAYOUT_LINK_FILE,
                &serde_json::to_vec(&serde_json::json!({"rails": {"near_ai": {
                    "idempotency_key": key, "organization_id": org
                }}}))
                .unwrap(),
            )
            .unwrap();
        record.recipient_ref = Some(trace_commons_protocol::payout_link::near_ai_recipient_ref(
            key, org,
        ));
    }
    balance_records()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(store.dir().to_path_buf(), record.clone());
    if let Ok(bytes) = serde_json::to_vec(&record) {
        let _ = store.write_daemon_file(ARRIVALS_FILE, &bytes);
    }
    records()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(store.dir().to_path_buf(), record);
}

#[cfg(test)]
#[path = "arrivals_tests.rs"]
mod tests;
