//! The machine-wide settlement estimate: the settlement table in force, when
//! the traces on this Mac are scanned again, and the rounded dollar band that
//! `status.settlement_estimate` reports.
//!
//! **Nothing here sends anything about this Mac** (spec S9). The table comes
//! in through the existing, unauthenticated estimate-table fetch in
//! `daemon::refresh_estimate_table`, which hands this module the outcome and
//! never the client. The scan reads transcripts through
//! [`settlement_scan::scan`], which takes the queue by shared reference and so
//! cannot enqueue, preview, redact or upload.
//!
//! **When a scan runs** ([`RescanSchedule`]): a watcher pass that queued or
//! superseded an entry, once ten minutes have passed since and at most hourly;
//! a daily rediscovery; and at once when the table's `version` or
//! `pricing_version` changes, when a folder or the contribution override moves
//! to or from Never, and when the switch is turned on. Every trigger runs the
//! same scan; the feature cache makes an unchanged trace free (plan D7).
//!
//! **The off switch** (`settlement_estimate_enabled`) stops a scan between
//! traces, discards its result, hides the estimate and deletes the feature
//! cache. A scan's result is published only when its generation is still the
//! current one, so a scan that was running when the switch went off can
//! neither publish nor write the cache back.
//!
//! **Lock order.** `DaemonShared::settlement` is a leaf: nothing here takes
//! another lock while holding it, so it may be taken under any other lock.
//! Snapshots of the policy, queue, state and settings are each taken under
//! their own short lock first.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use chrono::{DateTime, Duration, Utc};
use trace_commons_protocol::settlement_estimate::{
    SettlementEstimateTable, SettlementTotal, settlement_total,
};

use crate::credit_estimate_table::{EstimateTableClientError, SettlementFetchEffect};
use crate::source::{SourceRoots, TraceSource};

use super::ipc::{DaemonShared, EVENT_SETTLEMENT_ESTIMATE_CHANGED, EVENT_STATUS_CHANGED};
use super::policy::ProjectPolicy;
use super::queue::Queue;
use super::settlement_scan::{self, FeatureCache, ScanInputs, ScanOutcome};
use super::state::{CwdCacheEntry, PriorUpload, SettlementNotified};

/// How long after a watcher change the scan waits for things to go quiet.
pub const RESCAN_DEBOUNCE: Duration = Duration::minutes(10);

/// The closest together two change-driven scans may be.
pub const RESCAN_MIN_INTERVAL: Duration = Duration::hours(1);

/// A scan runs at least this often, change or no change, so traces that
/// appeared without a watcher pass noticing them are found.
pub const RESCAN_DAILY: Duration = Duration::hours(24);

/// When the next scan is due. Pure: its inputs are the clock and what the
/// daemon told it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RescanSchedule {
    /// Something asked for a scan now: a table or pricing change, a move to
    /// or from Never, or the switch turned on.
    immediate: bool,
    /// The last watcher change not yet scanned for. Each change restarts
    /// the debounce.
    changed_at: Option<DateTime<Utc>>,
    /// When the last scan started. `None` until the first, which is due at
    /// once.
    last_scan_at: Option<DateTime<Utc>>,
}

impl RescanSchedule {
    /// A watcher pass queued or superseded an entry at `now`.
    pub fn note_sessions_changed(&mut self, now: DateTime<Utc>) {
        self.changed_at = Some(now);
    }

    /// Scan on the next tick, whatever the clock says.
    pub fn request_immediate(&mut self) {
        self.immediate = true;
    }

    #[must_use]
    pub fn due(&self, now: DateTime<Utc>) -> bool {
        if self.immediate {
            return true;
        }
        let Some(last) = self.last_scan_at else {
            return true;
        };
        let since_last = now - last;
        let settled_change = self
            .changed_at
            .is_some_and(|changed| now - changed >= RESCAN_DEBOUNCE);
        (settled_change && since_last >= RESCAN_MIN_INTERVAL) || since_last >= RESCAN_DAILY
    }

    /// A scan started at `now`: it covers every request and change before it.
    pub fn started(&mut self, now: DateTime<Utc>) {
        self.immediate = false;
        self.changed_at = None;
        self.last_scan_at = Some(now);
    }
}

/// The estimate readers were last told about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementShown {
    pub total: SettlementTotal,
    pub table_version: String,
    pub pricing_version: Option<String>,
    pub estimated_at: DateTime<Utc>,
}

impl SettlementShown {
    /// Whether a reader would draw `other` exactly as `self`: the rounded
    /// band, the "up to" form, and the table and pricing it came from. The
    /// counts and the time are not drawn, so they do not count.
    fn draws_like(&self, other: &Self) -> bool {
        self.total.low_usd_micros == other.total.low_usd_micros
            && self.total.high_usd_micros == other.total.high_usd_micros
            && self.total.up_to() == other.total.up_to()
            && self.table_version == other.table_version
            && self.pricing_version == other.pricing_version
    }
}

/// A grown low end must be at least this much times the last announced low
/// end, in millionths (1.25). DRAFT, owner decision S4.
pub const SETTLEMENT_NOTIFY_GROWTH_MICROS: u64 = 250_000;

/// And at least this many dollar micros above it ($1). DRAFT, owner decision
/// S4.
pub const SETTLEMENT_NOTIFY_MIN_RISE_USD_MICROS: u64 = 1_000_000;

/// The news mark lights only when the low end is at least this much ($1).
pub const SETTLEMENT_MARK_MIN_LOW_USD_MICROS: u64 = 1_000_000;

/// Whether the current estimate is worth a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementCandidacy {
    /// Announce it. `pricing_changed` picks the pricing wording.
    Candidate { pricing_changed: bool },
    /// The first drawable estimate: record it as the baseline and say
    /// nothing.
    SeedBaseline,
    /// The table's pricing changed but moved the total too little to
    /// announce. Record the new `pricing_version` against the last
    /// announcement (its low and mid stay), so a later move under the same
    /// pricing is not taken for a pricing change. Say nothing.
    AdoptPricing,
    /// Nothing to announce.
    Quiet,
}

/// Decide whether `shown` is a notification candidate against what was last
/// announced. Pure, integer arithmetic only. With no record the estimate is
/// the first one and silently becomes the baseline.
#[must_use]
pub fn settlement_candidacy(
    shown: &SettlementShown,
    last: Option<&SettlementNotified>,
) -> SettlementCandidacy {
    let Some(last) = last else {
        return SettlementCandidacy::SeedBaseline;
    };
    let low = u128::from(shown.total.low_usd_micros);
    let last_low = u128::from(last.low_usd_micros);
    let growth = 1_000_000 + u128::from(SETTLEMENT_NOTIFY_GROWTH_MICROS);
    let grown = low * 1_000_000 >= last_low * growth
        && low >= last_low + u128::from(SETTLEMENT_NOTIFY_MIN_RISE_USD_MICROS);

    let mid = u128::from(shown.total.mid_usd_micros());
    let last_mid = u128::from(last.mid_usd_micros);
    let moved = mid.abs_diff(last_mid) * 1_000_000
        >= last_mid * u128::from(SETTLEMENT_NOTIFY_GROWTH_MICROS);
    let pricing_differs = shown.pricing_version != last.pricing_version;
    let pricing_changed = pricing_differs && moved;

    if grown || pricing_changed {
        SettlementCandidacy::Candidate { pricing_changed }
    } else if pricing_differs {
        SettlementCandidacy::AdoptPricing
    } else {
        SettlementCandidacy::Quiet
    }
}

/// Everything the daemon holds for the settlement estimate. In memory only:
/// a restart fetches the table on its schedule and scans again through the
/// cache.
#[derive(Debug, Default)]
pub struct SettlementEstimateState {
    /// The settlement table in force. `None` until one is fetched, and after
    /// a fetch that refused one or found none (never a stale one).
    pub table: Option<SettlementEstimateTable>,
    pub schedule: RescanSchedule,
    /// The estimate on `status`, when there is one.
    pub shown: Option<SettlementShown>,
    /// A scan is running; no other starts.
    pub scanning: bool,
    /// Bumped by every scan start and by the switch turning off. A scan
    /// publishes only if this has not moved since it started.
    pub generation: u64,
    /// The switch as this module last acted on it, so a change made where
    /// `set_settings` does not reach (the C ABI's pre-start override writes
    /// the file) is still acted on at the next tick.
    last_enabled: Option<bool>,
}

fn switch_on(shared: &DaemonShared) -> bool {
    shared
        .settings
        .lock()
        .expect("settings lock")
        .settlement_estimate_enabled
}

fn publish_changed(shared: &DaemonShared) {
    shared.publish(EVENT_SETTLEMENT_ESTIMATE_CHANGED, serde_json::json!({}));
    shared.publish(EVENT_STATUS_CHANGED, serde_json::json!({}));
}

/// Apply one settlement-table fetch outcome (see [`SettlementFetchEffect`]).
/// A table whose `version` or `pricing_version` differs from the one in
/// force asks for a scan at once; a cleared table takes the estimate with it.
pub(crate) fn apply_table_fetch(
    shared: &DaemonShared,
    outcome: Result<SettlementEstimateTable, EstimateTableClientError>,
) {
    let effect = SettlementFetchEffect::of(&outcome);
    let hidden = {
        let mut st = shared.settlement.lock().expect("settlement lock");
        match (effect, outcome) {
            (SettlementFetchEffect::Replace, Ok(table)) => {
                let moved = st.table.as_ref().is_none_or(|current| {
                    current.version != table.version
                        || current.pricing_version != table.pricing_version
                });
                if moved {
                    st.schedule.request_immediate();
                }
                st.table = Some(table);
                false
            }
            (SettlementFetchEffect::Clear, _) => {
                st.table = None;
                st.shown.take().is_some()
            }
            _ => false,
        }
    };
    if hidden {
        publish_changed(shared);
    }
}

/// The switch was set to `on`. Off: any running scan's result is discarded,
/// the estimate is hidden and the feature cache deleted. On: a full scan at
/// the next tick.
pub(crate) fn set_enabled(shared: &DaemonShared, on: bool) {
    let hidden = {
        let mut st = shared.settlement.lock().expect("settlement lock");
        st.last_enabled = Some(on);
        if on {
            st.schedule.request_immediate();
            false
        } else {
            st.generation = st.generation.wrapping_add(1);
            // Under the lock, so a scan finishing now cannot write the
            // cache back after it is gone.
            if FeatureCache::delete(&shared.store).is_err() {
                tracing::warn!("could not delete the settlement feature cache");
            }
            st.shown.take().is_some()
        }
    };
    if hidden {
        publish_changed(shared);
    }
}

/// A watcher pass queued or superseded at least one entry.
pub(crate) fn note_sessions_changed(shared: &DaemonShared, now: DateTime<Utc>) {
    shared
        .settlement
        .lock()
        .expect("settlement lock")
        .schedule
        .note_sessions_changed(now);
}

/// A folder's resolved mode, or the contribution override, changed. A move
/// to or from Never changes which traces count, so it asks for a scan now.
pub(crate) fn note_never_change(shared: &DaemonShared, was_never: bool, is_never: bool) {
    if was_never != is_never {
        shared
            .settlement
            .lock()
            .expect("settlement lock")
            .schedule
            .request_immediate();
    }
}

/// `status.settlement_estimate`: present only while the switch is on, a table
/// is in force and drawable at `now`, and at least one trace's tier is known.
pub(crate) fn status_value(shared: &DaemonShared, now: DateTime<Utc>) -> Option<serde_json::Value> {
    if !switch_on(shared) {
        return None;
    }
    let st = shared.settlement.lock().expect("settlement lock");
    st.table.as_ref().filter(|table| table.drawable_at(now))?;
    let shown = st.shown.as_ref().filter(|shown| shown.total.known > 0)?;
    Some(serde_json::json!({
        "low_usd_micros": shown.total.low_usd_micros,
        "high_usd_micros": shown.total.high_usd_micros,
        "up_to": shown.total.up_to(),
        "known": shown.total.known,
        "unknown": shown.total.unknown,
        "table_version": shown.table_version,
        "pricing_version": shown.pricing_version,
        "estimated_at": shown.estimated_at,
        "text": crate::nudge_render::settlement_total_line(&shown.total),
        "share_text": crate::nudge_render::settlement_share_line(&shown.total),
    }))
}

/// A claimed scan: owned snapshots, so the blocking work holds no lock.
struct ScanJob {
    generation: u64,
    sources: Vec<Box<dyn TraceSource>>,
    policy: ProjectPolicy,
    queue: Queue,
    prior_uploads: BTreeMap<String, PriorUpload>,
    cwd_cache: BTreeMap<String, CwdCacheEntry>,
    quiescence_secs: u64,
    now: DateTime<Utc>,
}

/// One supervisor tick. Returns at once -- `None` -- when the switch is off,
/// no table is in force, a scan is running, or none is due. Otherwise starts
/// the scan off the runtime's workers and returns its handle; the supervisor
/// drops it, so the tick never waits on a scan.
pub(crate) fn tick(
    shared: &Arc<DaemonShared>,
    now: DateTime<Utc>,
) -> Option<tokio::task::JoinHandle<()>> {
    tick_with_sources(shared, now, settlement_scan::estimation_sources)
}

/// [`tick`] over the sources `sources_for` builds from the declared roots.
/// Split out so tests can hand it sources that do not read this machine.
pub(crate) fn tick_with_sources(
    shared: &Arc<DaemonShared>,
    now: DateTime<Utc>,
    sources_for: impl FnOnce(&SourceRoots) -> Vec<Box<dyn TraceSource>>,
) -> Option<tokio::task::JoinHandle<()>> {
    if !observe_switch(shared) {
        return None;
    }
    if !scan_wanted(&shared.settlement.lock().expect("settlement lock"), now) {
        return None;
    }
    // Snapshots, each under its own short lock and none nested.
    let (quiescence_secs, roots) = {
        let settings = shared.settings.lock().expect("settings lock");
        (
            settings.quiescence_secs,
            settings.source_roots(&shared.store),
        )
    };
    let policy = shared.policy.lock().expect("policy lock").clone();
    let queue = shared.queue.lock().expect("queue lock").clone();
    let (prior_uploads, cwd_cache) = {
        let state = shared.state.lock().expect("state lock");
        (state.prior_uploads.clone(), state.cwd_cache.clone())
    };
    let sources = sources_for(&roots);
    let generation = {
        let mut st = shared.settlement.lock().expect("settlement lock");
        // Re-checked: the lock was let go for the snapshots.
        if !scan_wanted(&st, now) {
            return None;
        }
        st.scanning = true;
        st.generation = st.generation.wrapping_add(1);
        st.schedule.started(now);
        st.generation
    };
    let job = ScanJob {
        generation,
        sources,
        policy,
        queue,
        prior_uploads,
        cwd_cache,
        quiescence_secs,
        now,
    };
    let shared = Arc::clone(shared);
    Some(tokio::spawn(async move {
        let worker = Arc::clone(&shared);
        let scanned = tokio::task::spawn_blocking(move || run_scan(&worker, &job)).await;
        match scanned {
            Ok((outcome, cache)) => {
                finish(&shared, generation, &outcome, &cache, now);
            }
            Err(_) => {
                // The scan panicked or was cancelled: publish nothing, and
                // let the next due tick try again.
                shared.settlement.lock().expect("settlement lock").scanning = false;
                tracing::warn!("settlement estimate scan did not finish");
            }
        }
    }))
}

fn scan_wanted(st: &SettlementEstimateState, now: DateTime<Utc>) -> bool {
    !st.scanning && st.table.is_some() && st.schedule.due(now)
}

/// Act on a switch change this module has not seen yet; the switch's state.
fn observe_switch(shared: &DaemonShared) -> bool {
    let on = switch_on(shared);
    let seen = shared
        .settlement
        .lock()
        .expect("settlement lock")
        .last_enabled;
    if seen != Some(on) {
        set_enabled(shared, on);
    }
    on
}

/// The blocking part: load the cache and scan. Stops between traces when the
/// daemon is shutting down, the switch is off, or the generation moved.
fn run_scan(shared: &DaemonShared, job: &ScanJob) -> (ScanOutcome, FeatureCache) {
    let mut cache = FeatureCache::load(&shared.store);
    let inputs = ScanInputs {
        policy: &job.policy,
        queue: &job.queue,
        prior_uploads: &job.prior_uploads,
        cwd_cache: &job.cwd_cache,
        quiescence_secs: job.quiescence_secs,
        now: job.now,
    };
    let should_stop = || {
        shared.shutdown.load(Ordering::Relaxed)
            || !switch_on(shared)
            || shared
                .settlement
                .lock()
                .expect("settlement lock")
                .generation
                != job.generation
    };
    let outcome = settlement_scan::scan(&job.sources, &inputs, &mut cache, &should_stop);
    (outcome, cache)
}

/// A scan finished. Its result is kept only when its generation is still the
/// current one, the switch is on, and it was not stopped: then the cache is
/// saved, the total computed against the table in force now, and the event
/// published when what a reader draws changed. Returns whether it published.
fn finish(
    shared: &DaemonShared,
    generation: u64,
    outcome: &ScanOutcome,
    cache: &FeatureCache,
    now: DateTime<Utc>,
) -> bool {
    let enabled = switch_on(shared);
    let changed = {
        let mut st = shared.settlement.lock().expect("settlement lock");
        st.scanning = false;
        if st.generation != generation || !enabled || outcome.stopped {
            return false;
        }
        // Under the lock, so the off switch cannot delete the cache between
        // the generation check and this write.
        if cache.save(&shared.store).is_err() {
            tracing::warn!("could not save the settlement feature cache");
        }
        let next = st.table.as_ref().and_then(|table| {
            settlement_total(outcome.features.iter().map(Option::as_ref), table, now).map(|total| {
                SettlementShown {
                    total,
                    table_version: table.version.clone(),
                    pricing_version: table.pricing_version.clone(),
                    estimated_at: now,
                }
            })
        });
        let changed = match (&st.shown, &next) {
            (None, None) => false,
            (Some(before), Some(after)) => !before.draws_like(after),
            _ => true,
        };
        st.shown = next;
        changed
    };
    let known = outcome.features.iter().filter(|f| f.is_some()).count();
    tracing::info!(
        known,
        unknown = outcome.features.len() - known,
        ignored = outcome.ignored,
        excluded = outcome.excluded,
        skipped_sources = outcome.skipped_sources.len(),
        "settlement estimate scanned"
    );
    if changed {
        publish_changed(shared);
    }
    changed
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use axum::{Json, Router, http::StatusCode, response::IntoResponse};
    use serde_json::json;
    use trace_commons_protocol::local_credit_estimate::{
        LOCAL_ESTIMATE_FEATURES_VERSION, LocalEstimateFeatures,
    };
    use trace_commons_protocol::settlement_estimate::SETTLEMENT_TABLE_PATH;

    use super::*;
    use crate::config::DAEMON_SETTLEMENT_FEATURES_FILE;
    use crate::credit_estimate_table::{ESTIMATE_TABLE_PATH, EstimateTableSchedule};
    use crate::daemon::ipc::{Request, handle_request};
    use crate::source::{SessionEvent, SessionEventKind, SessionRef, SessionTranscript};

    /// Every figure below is synthetic.
    fn now() -> DateTime<Utc> {
        crate::daemon::test_support::at("2026-10-12T12:00:00Z")
    }

    fn shown_at(low: u64, high: u64, pricing: &str) -> SettlementShown {
        SettlementShown {
            total: SettlementTotal {
                low_usd_micros: low,
                high_usd_micros: high,
                known: 3,
                unknown: 0,
            },
            table_version: "s1".to_string(),
            pricing_version: Some(pricing.to_string()),
            estimated_at: now(),
        }
    }

    fn notified(low: u64, mid: u64, pricing: &str) -> SettlementNotified {
        SettlementNotified {
            low_usd_micros: low,
            mid_usd_micros: mid,
            pricing_version: Some(pricing.to_string()),
            at: now(),
        }
    }

    #[test]
    fn the_first_estimate_seeds_the_baseline_silently() {
        assert_eq!(
            settlement_candidacy(&shown_at(5_000_000, 9_000_000, "p2"), None),
            SettlementCandidacy::SeedBaseline
        );
    }

    #[test]
    fn growth_needs_both_25_percent_and_one_dollar() {
        let last = notified(4_000_000, 6_000_000, "p2");
        let cand = SettlementCandidacy::Candidate {
            pricing_changed: false,
        };
        // Same pricing; the mid stays inside 25% of 6_000_000 by keeping
        // low + high close to the last mid.
        assert_eq!(
            settlement_candidacy(&shown_at(5_000_000, 6_000_000, "p2"), Some(&last)),
            cand
        );
        assert_eq!(
            settlement_candidacy(&shown_at(4_999_999, 6_000_000, "p2"), Some(&last)),
            SettlementCandidacy::Quiet
        );
        let small = notified(2_000_000, 3_000_000, "p2");
        assert_eq!(
            settlement_candidacy(&shown_at(2_500_000, 3_000_000, "p2"), Some(&small)),
            SettlementCandidacy::Quiet,
            "25% but only $0.50"
        );
        assert_eq!(
            settlement_candidacy(&shown_at(3_000_000, 3_000_000, "p2"), Some(&small)),
            cand
        );
    }

    #[test]
    fn a_pricing_change_moves_the_mid_25_percent_either_way() {
        let last = notified(8_000_000, 8_000_000, "p2");
        let pricing = SettlementCandidacy::Candidate {
            pricing_changed: true,
        };
        // Mid 10_000_000 and 6_000_000 under the new pricing.
        assert_eq!(
            settlement_candidacy(&shown_at(8_000_000, 12_000_000, "p3"), Some(&last)),
            pricing
        );
        assert_eq!(
            settlement_candidacy(&shown_at(4_000_000, 8_000_000, "p3"), Some(&last)),
            pricing
        );
        // Mid 9_999_999 / 9_999_998: just under 25%. The new pricing is
        // adopted silently.
        assert_eq!(
            settlement_candidacy(&shown_at(7_999_999, 11_999_999, "p3"), Some(&last)),
            SettlementCandidacy::AdoptPricing
        );
        // Same pricing, mid 6_000_000, low fell: quiet.
        assert_eq!(
            settlement_candidacy(&shown_at(4_000_000, 8_000_000, "p2"), Some(&last)),
            SettlementCandidacy::Quiet
        );
    }

    #[test]
    fn a_drop_after_an_adopted_pricing_change_is_quiet() {
        let last = notified(8_000_000, 10_000_000, "p1");
        assert_eq!(
            settlement_candidacy(&shown_at(8_500_000, 12_500_000, "p2"), Some(&last)),
            SettlementCandidacy::AdoptPricing
        );
        let adopted = notified(8_000_000, 10_000_000, "p2");
        assert_eq!(
            settlement_candidacy(&shown_at(5_000_000, 9_000_000, "p2"), Some(&adopted)),
            SettlementCandidacy::Quiet
        );
    }

    fn table_value() -> serde_json::Value {
        json!({
            "schema_version": 2,
            "features_version": "lef1",
            "version": "s1",
            "pricing_version": "p2",
            "instrument_id": "trace_credit",
            "atomic_units_per_credit": 1_000_000,
            "usd_micros_per_credit": 1_000_000,
            "published_at": "2026-10-12T00:00:00Z",
            "bytes_per_token": 4,
            "chunk_target_tokens": 2048,
            "chunk_cap": 16,
            "weights": [{"term": "ln_content_bytes", "weight": 1.0}],
            "cut_offs": [5.0],
            "tiers": [
                {"mean_atomic_units": 1_000_000, "withheld_share_micros": 0},
                {"mean_atomic_units": 1_010_000, "withheld_share_micros": 0}
            ],
            "aggregate_band": {"low_ratio_micros": 600_000, "high_ratio_micros": 1_400_000},
            "fit_rows": 40
        })
    }

    fn table_with(mutate: impl FnOnce(&mut serde_json::Value)) -> SettlementEstimateTable {
        let mut value = table_value();
        mutate(&mut value);
        SettlementEstimateTable::from_value(&value).expect("synthetic table parses")
    }

    fn table() -> SettlementEstimateTable {
        table_with(|_| {})
    }

    /// Features in the lower tier (`ln(1 + 10)` is under the cut-off) or the
    /// higher one.
    fn features(higher: bool) -> LocalEstimateFeatures {
        LocalEstimateFeatures {
            version: LOCAL_ESTIMATE_FEATURES_VERSION.to_string(),
            content_bytes: if higher { 10_000 } else { 10 },
            tool_result_bytes: 0,
            agent_prose_bytes: 0,
            user_messages: 1,
            distinct_tools: 0,
            byte_entropy_milli: None,
            deflate_ratio_milli: None,
        }
    }

    fn shared() -> Arc<DaemonShared> {
        let (dir, store) = crate::config::tests_support::temp_store();
        std::mem::forget(dir);
        Arc::new(DaemonShared::load(store).unwrap())
    }

    fn req(method: &str, params: serde_json::Value) -> Request {
        Request {
            id: 1,
            method: method.to_string(),
            params,
        }
    }

    fn due(s: &DaemonShared) -> bool {
        s.settlement.lock().unwrap().schedule.due(now())
    }

    /// Mark the schedule as just scanned, so only a new request makes it due.
    fn settle_schedule(s: &DaemonShared) {
        s.settlement.lock().unwrap().schedule.started(now());
    }

    /// Finish a scan that found `features`, as the spawned task would.
    fn finish_with(s: &DaemonShared, features: Vec<Option<LocalEstimateFeatures>>) -> bool {
        let generation = {
            let mut st = s.settlement.lock().unwrap();
            st.scanning = true;
            st.generation = st.generation.wrapping_add(1);
            st.generation
        };
        let outcome = ScanOutcome {
            features,
            ..ScanOutcome::default()
        };
        finish(s, generation, &outcome, &FeatureCache::default(), now())
    }

    fn count(
        rx: &mut tokio::sync::broadcast::Receiver<super::super::ipc::Event>,
        name: &str,
    ) -> usize {
        let mut n = 0;
        while let Ok(event) = rx.try_recv() {
            if event.event == name {
                n += 1;
            }
        }
        n
    }

    fn cache_file(s: &DaemonShared) -> Option<Vec<u8>> {
        s.store
            .read_daemon_file(DAEMON_SETTLEMENT_FEATURES_FILE)
            .unwrap()
    }

    /// A real directory, so a ref's cwd resolves to a real project key.
    fn project_dir() -> PathBuf {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("project");
        std::fs::create_dir_all(&p).unwrap();
        std::mem::forget(d);
        std::fs::canonicalize(&p).unwrap()
    }

    fn session(path: &Path, cwd: &Path) -> SessionRef {
        SessionRef {
            source: "fixture",
            declared_source: None,
            path: path.to_path_buf(),
            project: None,
            cwd: Some(cwd.to_string_lossy().to_string()),
            started_at: None,
            size_bytes: 100,
            group_modified_at: Some(now() - Duration::hours(2)),
            group_member_count: 0,
        }
    }

    /// A source of settled refs whose `load` returns a short synthetic
    /// transcript, optionally waiting for the test first.
    struct FixtureSource {
        refs: Vec<SessionRef>,
        /// Signalled when `load` is entered, then waited on before returning.
        gate: Option<(
            Mutex<std::sync::mpsc::Sender<()>>,
            Mutex<std::sync::mpsc::Receiver<()>>,
        )>,
    }

    impl TraceSource for FixtureSource {
        fn name(&self) -> &'static str {
            "fixture"
        }
        fn discover(&self) -> anyhow::Result<Vec<SessionRef>> {
            Ok(self.refs.clone())
        }
        fn discover_unopened(&self) -> anyhow::Result<Vec<SessionRef>> {
            Ok(self.refs.clone())
        }
        fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript> {
            if let Some((entered, release)) = &self.gate {
                let _ = entered.lock().unwrap().send(());
                let _ = release
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(30));
            }
            Ok(SessionTranscript {
                cwd: r.cwd.clone(),
                events: vec![SessionEvent {
                    kind: SessionEventKind::User,
                    content: Some("a synthetic message".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            })
        }
    }

    fn fixture_refs(n: usize) -> Vec<SessionRef> {
        let cwd = project_dir();
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();
        std::mem::forget(dir);
        (0..n)
            .map(|i| session(&base.join(format!("s{i}.jsonl")), &cwd))
            .collect()
    }

    #[test]
    fn the_debounce_waits_ten_minutes_and_at_most_hourly() {
        let t = now();
        let mut schedule = RescanSchedule::default();
        assert!(schedule.due(t), "never scanned is due");
        schedule.started(t);
        assert!(!schedule.due(t));

        schedule.note_sessions_changed(t + Duration::minutes(5));
        assert!(!schedule.due(t + Duration::minutes(14)));
        assert!(!schedule.due(t + Duration::minutes(59)));
        assert!(schedule.due(t + Duration::minutes(60)));

        schedule.started(t + Duration::minutes(60));
        schedule.note_sessions_changed(t + Duration::hours(2));
        assert!(!schedule.due(t + Duration::hours(2)));
        assert!(!schedule.due(t + Duration::hours(2) + Duration::minutes(9)));
        assert!(schedule.due(t + Duration::hours(2) + Duration::minutes(10)));
    }

    #[test]
    fn the_daily_scan_is_due_without_a_change() {
        let t = now();
        let mut schedule = RescanSchedule::default();
        schedule.started(t);
        assert!(!schedule.due(t + Duration::hours(23) + Duration::minutes(59)));
        assert!(schedule.due(t + RESCAN_DAILY));
    }

    #[tokio::test]
    async fn a_version_or_pricing_change_rescans_immediately() {
        let s = shared();
        apply_table_fetch(&s, Ok(table()));
        assert!(due(&s), "the first table is a change");
        settle_schedule(&s);

        apply_table_fetch(&s, Ok(table()));
        assert!(!due(&s), "the same table again is not");

        apply_table_fetch(&s, Ok(table_with(|v| v["pricing_version"] = json!("p3"))));
        assert!(due(&s), "a pricing change rescans");
        settle_schedule(&s);

        apply_table_fetch(
            &s,
            Ok(table_with(|v| {
                v["pricing_version"] = json!("p3");
                v["version"] = json!("s2");
            })),
        );
        assert!(due(&s), "a table version change rescans");
        settle_schedule(&s);

        // An outage keeps the table and asks for nothing.
        apply_table_fetch(&s, Err(EstimateTableClientError::Unavailable));
        assert!(!due(&s));
        assert!(s.settlement.lock().unwrap().table.is_some());

        // A 404 clears it.
        apply_table_fetch(&s, Err(EstimateTableClientError::NotFound));
        assert!(s.settlement.lock().unwrap().table.is_none());
    }

    #[tokio::test]
    async fn a_mode_change_to_or_from_never_rescans_immediately() {
        let s = shared();
        let key = crate::daemon::policy::project_key_for(Some(&project_dir().to_string_lossy()));
        settle_schedule(&s);

        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                json!({"project_key": key, "mode": "ignore"}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(due(&s), "to Never rescans");
        settle_schedule(&s);

        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                json!({"project_key": key, "mode": "notify_only"}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(due(&s), "from Never rescans");
        settle_schedule(&s);

        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                json!({"project_key": key, "mode": "notify_only"}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(!due(&s), "Ask to Ask is not a Never change");

        let r = handle_request(
            &s,
            &req("set_contribution_override", json!({"mode": "ignore"})),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(due(&s), "the Never override rescans");
        settle_schedule(&s);

        let r = handle_request(&s, &req("clear_contribution_override", json!({})));
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(due(&s), "clearing the Never override rescans");
    }

    #[tokio::test]
    async fn status_carries_the_estimate_only_when_drawable() {
        let s = shared();
        let three = || vec![Some(features(false)); 3];

        // No table.
        finish_with(&s, three());
        assert!(status_value(&s, now()).is_none());
        assert!(
            s.status_value_at(now())
                .get("settlement_estimate")
                .is_none()
        );

        // A table without a rate parses and draws nothing.
        apply_table_fetch(
            &s,
            Ok(table_with(|v| {
                v.as_object_mut().unwrap().remove("usd_micros_per_credit");
            })),
        );
        finish_with(&s, three());
        assert!(status_value(&s, now()).is_none());
        assert!(
            s.status_value_at(now())
                .get("settlement_estimate")
                .is_none()
        );

        // Drawable, three known traces: 3 credits at $1, times 0.6 and 1.4,
        // rounded to $0.50 steps.
        apply_table_fetch(&s, Ok(table()));
        finish_with(&s, three());
        let status = s.status_value_at(now());
        let estimate = &status["settlement_estimate"];
        assert_eq!(estimate["known"], 3);
        assert_eq!(estimate["unknown"], 0);
        assert_eq!(estimate["low_usd_micros"], 1_500_000);
        assert_eq!(estimate["high_usd_micros"], 4_500_000);
        assert_eq!(estimate["up_to"], false);
        assert_eq!(estimate["table_version"], "s1");
        assert_eq!(estimate["pricing_version"], "p2");
        let text = estimate["text"].as_str().unwrap();
        assert!(text.starts_with("About "), "{text}");
        // Home's call to share, over the same band.
        let share = estimate["share_text"].as_str().unwrap();
        assert_eq!(
            share,
            "You could make about $1.50 to $4.50 of inference if you share these"
        );

        // Stale by the clock alone: gone, with no scan.
        let later = now() + Duration::days(8);
        assert!(status_value(&s, later).is_none());

        // No known trace: nothing at all, never $0.
        finish_with(&s, vec![None, None]);
        assert!(status_value(&s, now()).is_none());
    }

    #[tokio::test]
    async fn an_unrounded_change_publishes_no_event() {
        let s = shared();
        apply_table_fetch(&s, Ok(table()));
        let mut rx = s.events.subscribe();

        // 3.00 credits and 3.01 credits: different sums, one rounded band.
        let a = vec![Some(features(false)); 3];
        let b = vec![
            Some(features(false)),
            Some(features(false)),
            Some(features(true)),
        ];
        let t = table();
        let total_a = settlement_total(a.iter().map(Option::as_ref), &t, now()).unwrap();
        let total_b = settlement_total(b.iter().map(Option::as_ref), &t, now()).unwrap();
        assert_eq!(total_a, total_b, "the two scans round alike");

        assert!(finish_with(&s, a));
        assert!(!finish_with(&s, b));
        assert_eq!(count(&mut rx, EVENT_SETTLEMENT_ESTIMATE_CHANGED), 1);
    }

    #[tokio::test]
    async fn the_off_switch_deletes_the_cache_and_hides_the_estimate() {
        let s = shared();
        apply_table_fetch(&s, Ok(table()));
        finish_with(&s, vec![Some(features(false)); 3]);
        assert!(cache_file(&s).is_some(), "a finished scan saved its cache");
        assert!(status_value(&s, now()).is_some());
        let mut rx = s.events.subscribe();

        let r = handle_request(
            &s,
            &req(
                "set_settings",
                json!({"settlement_estimate_enabled": false}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(cache_file(&s).is_none(), "the cache is deleted");
        assert!(
            s.status_value_at(now())
                .get("settlement_estimate")
                .is_none()
        );
        assert_eq!(count(&mut rx, EVENT_SETTLEMENT_ESTIMATE_CHANGED), 1);

        // And no scan starts while it is off.
        assert!(tick_with_sources(&s, now(), |_| Vec::new()).is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn turning_off_mid_scan_publishes_nothing_and_leaves_no_cache() {
        let s = shared();
        apply_table_fetch(&s, Ok(table()));
        let mut rx = s.events.subscribe();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let source = FixtureSource {
            refs: fixture_refs(2),
            gate: Some((Mutex::new(entered_tx), Mutex::new(release_rx))),
        };
        let handle = tick_with_sources(&s, now(), move |_| {
            vec![Box::new(source) as Box<dyn TraceSource>]
        })
        .expect("a scan starts");

        // Wait until the scan is inside a transcript, then turn it off.
        let entered = tokio::task::spawn_blocking(move || {
            entered_rx.recv_timeout(std::time::Duration::from_secs(30))
        })
        .await
        .unwrap();
        assert!(entered.is_ok(), "the scan reached a transcript");
        let r = handle_request(
            &s,
            &req(
                "set_settings",
                json!({"settlement_estimate_enabled": false}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        release_tx.send(()).unwrap();
        let _ = release_tx.send(());
        handle.await.unwrap();

        assert!(cache_file(&s).is_none(), "the cache stays deleted");
        assert!(s.settlement.lock().unwrap().shown.is_none());
        assert!(!s.settlement.lock().unwrap().scanning);
        assert!(
            s.status_value_at(now())
                .get("settlement_estimate")
                .is_none()
        );
        assert_eq!(count(&mut rx, EVENT_SETTLEMENT_ESTIMATE_CHANGED), 0);
    }

    #[tokio::test]
    async fn turning_on_requests_a_full_scan() {
        let s = shared();
        settle_schedule(&s);
        let r = handle_request(
            &s,
            &req(
                "set_settings",
                json!({"settlement_estimate_enabled": false}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        settle_schedule(&s);
        assert!(!due(&s));
        let r = handle_request(
            &s,
            &req("set_settings", json!({"settlement_estimate_enabled": true})),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(due(&s), "turning on asks for a full scan");
    }

    /// The settlement table is fetched on its schedule whatever the switch,
    /// so turning the switch on starts a full scan at the next tick instead
    /// of waiting a day for the next fetch. The fetch's timing never depends
    /// on the switch.
    #[tokio::test]
    async fn turning_on_after_a_fetch_made_while_off_starts_a_scan() {
        let app = Router::new().fallback(move |uri: axum::http::Uri| async move {
            if uri.path() == SETTLEMENT_TABLE_PATH {
                Json(table_value()).into_response()
            } else {
                StatusCode::NOT_FOUND.into_response()
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let s = shared();
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = base.clone();
        cfg.allowed_hosts = Some("127.0.0.1".to_string());
        s.store.save_config(&cfg).unwrap();
        let r = handle_request(
            &s,
            &req(
                "set_settings",
                json!({"settlement_estimate_enabled": false}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(tick_with_sources(&s, now(), |_| Vec::new()).is_none());

        let mut schedule = EstimateTableSchedule::starting(now());
        super::super::refresh_estimate_table(&s, now(), &mut schedule).await;
        assert!(
            s.settlement.lock().unwrap().table.is_some(),
            "the table is fetched while the switch is off"
        );
        assert!(
            s.status_value_at(now())
                .get("settlement_estimate")
                .is_none_or(|v| v.is_null())
        );

        let r = handle_request(
            &s,
            &req("set_settings", json!({"settlement_estimate_enabled": true})),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        let source = FixtureSource {
            refs: fixture_refs(1),
            gate: None,
        };
        tick_with_sources(&s, now(), move |_| {
            vec![Box::new(source) as Box<dyn TraceSource>]
        })
        .expect("turning the switch on starts a full scan")
        .await
        .unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn a_scan_and_estimate_make_no_request_but_the_table_fetch() {
        let paths: Arc<Mutex<BTreeSet<String>>> = Arc::new(Mutex::new(BTreeSet::new()));
        let recorded = Arc::clone(&paths);
        let app = Router::new().fallback(move |uri: axum::http::Uri| {
            let recorded = Arc::clone(&recorded);
            async move {
                let path = uri.path().to_string();
                recorded.lock().unwrap().insert(path.clone());
                if path == SETTLEMENT_TABLE_PATH {
                    Json(table_value()).into_response()
                } else if path == ESTIMATE_TABLE_PATH {
                    StatusCode::NOT_FOUND.into_response()
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let s = shared();
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.consent_scopes_chosen = Some(true);
        cfg.ingest_url = base.clone();
        cfg.allowed_hosts = Some("127.0.0.1".to_string());
        cfg.pii_filter = Some(format!("{base}/privacy-filter"));
        cfg.inference_receipt_endpoint = Some(format!("{base}/inference"));
        s.store.save_config(&cfg).unwrap();
        let queue_before = s.queue.lock().unwrap().all().to_vec();

        let mut schedule = EstimateTableSchedule::starting(now());
        assert!(schedule.due(now()));
        super::super::refresh_estimate_table(&s, now(), &mut schedule).await;
        assert!(
            s.settlement.lock().unwrap().table.is_some(),
            "the settlement table was fetched and accepted"
        );

        let source = FixtureSource {
            refs: fixture_refs(3),
            gate: None,
        };
        tick_with_sources(&s, now(), move |_| {
            vec![Box::new(source) as Box<dyn TraceSource>]
        })
        .expect("a scan starts")
        .await
        .unwrap();

        let recorded = paths.lock().unwrap().clone();
        let allowed: BTreeSet<String> = [ESTIMATE_TABLE_PATH, SETTLEMENT_TABLE_PATH]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert!(recorded.is_subset(&allowed), "{recorded:?}");
        assert!(recorded.contains(SETTLEMENT_TABLE_PATH));
        assert_eq!(s.queue.lock().unwrap().all().to_vec(), queue_before);
        assert_eq!(
            s.status_value_at(now())["settlement_estimate"]["known"],
            3,
            "the scan was estimated"
        );
        server.abort();
    }
}
