//! The Insights counter pass (feed T) and `insights_week`.
//!
//! Gated on owner decision D4 (open), through the `insights_counter_pass`
//! setting, off by default. While it is off the watcher runs no pass, nothing
//! is read for Insights, and `insights_week` answers `enabled: false` without
//! opening the store or the key.
//!
//! # The pass
//!
//! On a full watcher poll, after the session has been quiet for the same
//! `quiescence_secs` the watcher waits before offering it, the pass reads the
//! Claude Code and Codex sessions the watcher already finds and stores their
//! counter rows: Claude per-turn rows with the extractor saved analysis uses
//! (`insights::turn_series`), and Codex's first-to-last observed change only
//! (owner decision D11, open). Sessions in a folder whose own rule is Never
//! are not read, and rows already stored for one are dropped (owner decision
//! D4, open). A Claude session's delegated transcripts are not read; a
//! session that has any is marked truncated, so it counts as partial
//! coverage and never as complete.
//!
//! A poll reads a bounded number of files and bytes, newest first, so a first
//! pass over a large corpus catches up across polls. A file unchanged in size
//! and time since its row was made is not read again; a file seen again with
//! new bytes replaces its row.
//!
//! # The store
//!
//! One daemon-local file, capped by sessions, turns and tool calls, oldest
//! going first, and limited to recent weeks. Rows are keyed by a keyed digest
//! of the harness and the session's address, and carry a keyed digest of the
//! folder's project key (owner decision D7, open), never the key itself or a
//! path. The key is the daemon's own, in the OS keychain behind the
//! `DigestKeyStore` trait (owner decision D16, open); tests use an in-memory
//! store and never reach a keychain. The store and its key are removed on
//! unenroll.
//!
//! Anyone holding both the store and the key could confirm a guessed path;
//! a store copied without the key reveals none.
//!
//! # What crosses the socket
//!
//! The week's rollup: counts, dates, coverage reasons, declared model labels
//! and fixed labels. Never a session or project digest, a path, a message or
//! session ID. Nothing here is logged except fixed labels.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, Offset, Utc, Weekday};
use serde::{Deserialize, Serialize};
use trace_commons_protocol::insights_usage_series::{
    DigestKey, DigestKeyError, DigestKeyStore, InMemoryDigestKeyStore, KeyedDigest, UsageSeries,
    key_fingerprint, project_digest, session_key,
};

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, Request, Response};
use crate::config::ConfigStore;
use crate::insights::analytics_constants::PATTERN_BAR_WEEKS;
use crate::insights::analytics_constants::{
    COUNTER_PASS_EXCLUDES_NEVER_FOLDERS, COUNTER_PASS_MAX_BYTES_PER_TICK,
    COUNTER_PASS_MAX_READS_PER_TICK, COUNTER_PASS_MAX_SESSION_BYTES, COUNTER_STORE_KEEP_WEEKS,
    COUNTER_STORE_MAX_SESSIONS, COUNTER_STORE_MAX_TOOL_CALLS, COUNTER_STORE_MAX_TURNS,
    DIGEST_KEY_CUSTODY, DigestKeyCustody,
};
use crate::insights::goals::WeekFigures;
use crate::insights::usage::NativeTokenCounts;
use crate::insights::usage_evidence::{PersistedUsageEvidence, extract_codex_usage_evidence};
use crate::insights::week_glance::{counter_overview, dated_weeks};
use crate::insights::week_patterns::{week_figures, week_patterns};
use crate::insights::week_rollup::{
    AnalyticsSource, CodexObserved, Feed, SessionBody, SessionInput, UnknownReason, WeekRollup,
    change_vs_last_week, comparable, local_week_start, week_rollup,
};
use crate::source::{SOURCE_CLAUDE_CODE, SOURCE_CODEX};

/// The feed label every answer carries, so a shell can say which feed it
/// shows.
pub const FEED_COUNTER_PASS: &str = "counter_pass";
/// The store, in the daemon directory.
pub const COUNTER_ROWS_FILE: &str = "insights-counter-rows.json";
/// Where the key lives under the key-file custody (owner decision D16).
pub const COUNTER_KEY_DIR: &str = "insights-counter-key";
pub const COUNTER_STORE_SCHEMA: &str = "trace_commons.insights_counter_rows.v1";
/// `iso_week` not a `YYYY-Www` string naming a real ISO week.
pub const ERR_ISO_WEEK_INVALID: &str = "iso-week-invalid";
/// The digest key could not be read; nothing is shown in its place.
pub const UNREADABLE_KEY: &str = "key_unavailable";
/// The store could not be read; nothing is shown in its place.
pub const UNREADABLE_STORE: &str = "store_unreadable";
/// The folder rules could not be read, so the Never folders are unknown;
/// nothing is shown in their place.
pub const UNREADABLE_POLICY: &str = "policy_unreadable";
/// The store could not be written; the rows stay as they were.
pub const STORE_WRITE_FAILED: &str = "store_write_failed";
/// The store could not be removed on unenroll.
pub const ERR_CLEAR_FAILED: &str = "insights-counter-clear-failed";

/// One stored session's counters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RowBody {
    Claude {
        /// Declared labels the turns' `model_label_ix` index into.
        model_labels: Vec<String>,
        series: UsageSeries,
    },
    Codex {
        observed: Option<CodexObserved>,
    },
    Unknown {
        source: AnalyticsSource,
        reason: UnknownReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CounterRow {
    /// The observation the row was made from, so an unchanged file is not
    /// read again.
    size_bytes: u64,
    modified_at: DateTime<Utc>,
    /// The session's first recorded event, from discovery. Never the time it
    /// was counted.
    placed_at: Option<DateTime<Utc>>,
    /// Keyed digest of the folder's project key (owner decision D7, open).
    project: KeyedDigest,
    /// Order the row was made in, for the rollup's overlap rule.
    seq: u64,
    body: RowBody,
}

impl CounterRow {
    fn turns(&self) -> usize {
        match &self.body {
            RowBody::Claude { series, .. } => series.turns.len(),
            _ => 0,
        }
    }

    fn tool_calls(&self) -> usize {
        match &self.body {
            RowBody::Claude { series, .. } => series.tool_calls.len(),
            _ => 0,
        }
    }

    fn session_input(&self) -> SessionInput {
        SessionInput {
            // The order the row was made in, so the engine can tell the
            // sessions apart. Never a digest; every wire form drops it.
            session_ref: format!("t{}", self.seq),
            import_seq: self.seq,
            placed_at: self.placed_at,
            body: match &self.body {
                RowBody::Claude {
                    model_labels,
                    series,
                } => SessionBody::Claude {
                    series: series.clone(),
                    model_labels: model_labels.clone(),
                },
                RowBody::Codex { observed } => SessionBody::Codex {
                    observed: *observed,
                },
                RowBody::Unknown { source, reason } => SessionBody::Unknown {
                    source: Some(*source),
                    reason: *reason,
                },
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CounterStore {
    schema: String,
    /// Names the key every digest here was made under, without revealing it.
    key_fingerprint: KeyedDigest,
    next_seq: u64,
    rows: BTreeMap<KeyedDigest, CounterRow>,
}

impl CounterStore {
    fn empty(key_fingerprint: KeyedDigest) -> Self {
        Self {
            schema: COUNTER_STORE_SCHEMA.to_string(),
            key_fingerprint,
            next_seq: 0,
            rows: BTreeMap::new(),
        }
    }
}

/// One session the watcher found, as the pass sees it.
#[derive(Debug, Clone)]
pub(crate) struct CounterCandidate {
    /// The caller's own index, for resolving the session's folder.
    pub origin: usize,
    /// The adapter that found it.
    pub source: &'static str,
    pub path: PathBuf,
    /// The whole group's size and latest write, as the watcher judges
    /// quiescence.
    pub size_bytes: u64,
    pub modified_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    /// Delegated transcripts beside the session file, which are not read.
    pub group_member_count: u32,
}

/// What one pass did. Counts only.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PassSummary {
    pub read: usize,
    /// Left for a later poll by the per-poll budget.
    pub deferred: usize,
    pub dropped: usize,
}

/// What the daemon's settings add to an `insights_week` answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WeekOptions {
    /// `insights_recap_card_enabled`.
    pub recap_card_enabled: bool,
    /// `insights_context_threshold`; unset, nothing is counted against one.
    pub context_threshold: Option<u32>,
}

impl Default for WeekOptions {
    fn default() -> Self {
        Self {
            recap_card_enabled: crate::insights::analytics_constants::RECAP_CARD_DEFAULT_ON,
            context_threshold: None,
        }
    }
}

/// The daemon's counter pass and its store.
pub struct CounterPass {
    rows_path: PathBuf,
    keys: Box<dyn DigestKeyStore + Send + Sync>,
    /// The store as last read or written; `None` until first needed.
    cached: Mutex<Option<Arc<CounterStore>>>,
    /// When a pass last finished, for the answer's `updated_at`.
    last_pass_at: Mutex<Option<DateTime<Utc>>>,
    /// The last failure label logged, so a refused keychain is logged once.
    last_failure: Mutex<Option<&'static str>>,
}

impl CounterPass {
    /// The pass for a daemon over `store`, with the key custody owner
    /// decision D16 (open) names.
    pub fn for_store(store: &ConfigStore) -> Self {
        Self::with_keys(
            store.daemon_path(COUNTER_ROWS_FILE),
            default_counter_keys(store),
        )
    }

    pub(crate) fn with_keys(
        rows_path: PathBuf,
        keys: Box<dyn DigestKeyStore + Send + Sync>,
    ) -> Self {
        Self {
            rows_path,
            keys,
            cached: Mutex::new(None),
            last_pass_at: Mutex::new(None),
            last_failure: Mutex::new(None),
        }
    }

    /// Run one pass. `project_of` resolves a candidate's project key and
    /// whether its folder's own rule is Never; it is asked only for a
    /// session about to be read. `never_project_keys` are every folder whose
    /// rule is Never, so rows for sessions no longer on disk drop too.
    pub(crate) fn run_pass(
        &self,
        candidates: &[CounterCandidate],
        never_project_keys: &[String],
        now: DateTime<Utc>,
        quiescence_secs: u64,
        project_of: &mut dyn FnMut(&CounterCandidate) -> (String, bool),
    ) -> Result<PassSummary, &'static str> {
        let result = self.pass_inner(
            candidates,
            never_project_keys,
            now,
            quiescence_secs,
            project_of,
        );
        let failure = result.as_ref().err().copied();
        if let Ok(mut last) = self.last_failure.lock() {
            if *last != failure {
                if let Some(label) = failure {
                    // A fixed label, never the error: its context can carry
                    // a path.
                    tracing::warn!(label, "insights counter pass skipped");
                }
                *last = failure;
            }
        }
        if result.is_ok() {
            if let Ok(mut at) = self.last_pass_at.lock() {
                *at = Some(now);
            }
        }
        result
    }

    fn pass_inner(
        &self,
        candidates: &[CounterCandidate],
        never_project_keys: &[String],
        now: DateTime<Utc>,
        quiescence_secs: u64,
        project_of: &mut dyn FnMut(&CounterCandidate) -> (String, bool),
    ) -> Result<PassSummary, &'static str> {
        let key = self.keys.load_or_create().map_err(|_| UNREADABLE_KEY)?;
        let fingerprint = key_fingerprint(&key);
        // A store that cannot be read, or was made under another key, is
        // started again: its rows are re-derived from the sessions on disk.
        let (current, restart) = match self.load_cached() {
            Ok(Some(store)) if store.key_fingerprint == fingerprint => (store, false),
            Ok(None) => (Arc::new(CounterStore::empty(fingerprint)), false),
            Ok(Some(_)) | Err(_) => (Arc::new(CounterStore::empty(fingerprint)), true),
        };
        let cutoff = now - Duration::weeks(COUNTER_STORE_KEEP_WEEKS);
        let never = never_digests(&key, never_project_keys);
        let mut summary = PassSummary::default();
        let mut removals: BTreeSet<KeyedDigest> = current
            .rows
            .iter()
            .filter(|(_, row)| never.contains(&row.project) || row.modified_at < cutoff)
            .map(|(session, _)| *session)
            .collect();

        let mut order: Vec<&CounterCandidate> = candidates.iter().collect();
        order.sort_by_key(|candidate| std::cmp::Reverse(candidate.modified_at));
        let mut reads: Vec<(KeyedDigest, CounterRow)> = Vec::new();
        let mut bytes_budgeted = 0u64;
        for candidate in order {
            let Some(source) = analytics_source(candidate.source) else {
                continue;
            };
            if candidate.modified_at < cutoff
                || now
                    .signed_duration_since(candidate.modified_at)
                    .num_seconds()
                    < i64::try_from(quiescence_secs).unwrap_or(i64::MAX)
            {
                continue;
            }
            let session = session_key(&key, candidate.source, &candidate.path.to_string_lossy());
            let unchanged = current.rows.get(&session).is_some_and(|row| {
                row.size_bytes == candidate.size_bytes && row.modified_at == candidate.modified_at
            });
            if unchanged && !removals.contains(&session) {
                continue;
            }
            let (project_key, never_folder) = project_of(candidate);
            if never_folder && COUNTER_PASS_EXCLUDES_NEVER_FOLDERS {
                if current.rows.contains_key(&session) {
                    removals.insert(session);
                }
                continue;
            }
            // Charged by the one file read, not the group: delegated
            // transcripts are counted in `size_bytes` but never read.
            let read_len = std::fs::metadata(&candidate.path)
                .map(|meta| meta.len())
                .unwrap_or(candidate.size_bytes);
            let cost = read_len.min(COUNTER_PASS_MAX_SESSION_BYTES);
            if reads.len() >= COUNTER_PASS_MAX_READS_PER_TICK
                || (!reads.is_empty()
                    && bytes_budgeted.saturating_add(cost) > COUNTER_PASS_MAX_BYTES_PER_TICK)
            {
                summary.deferred += 1;
                continue;
            }
            bytes_budgeted = bytes_budgeted.saturating_add(cost);
            removals.remove(&session);
            reads.push((
                session,
                CounterRow {
                    size_bytes: candidate.size_bytes,
                    modified_at: candidate.modified_at,
                    placed_at: candidate.started_at,
                    project: project_digest(&key, &project_key),
                    seq: 0,
                    body: read_body(source, candidate, &key),
                },
            ));
        }
        summary.read = reads.len();

        if reads.is_empty() && removals.is_empty() && !restart {
            return Ok(summary);
        }
        let mut store = (*current).clone();
        for session in &removals {
            if store.rows.remove(session).is_some() {
                summary.dropped += 1;
            }
        }
        for (session, mut row) in reads {
            row.seq = store.next_seq;
            store.next_seq += 1;
            store.rows.insert(session, row);
        }
        enforce_caps(
            &mut store,
            COUNTER_STORE_MAX_SESSIONS,
            COUNTER_STORE_MAX_TURNS,
            COUNTER_STORE_MAX_TOOL_CALLS,
        );
        self.save(store)?;
        Ok(summary)
    }

    /// The `insights_week` answer.
    pub(crate) fn week_value(
        &self,
        enabled: bool,
        week: Option<NaiveDate>,
        tz: FixedOffset,
        now: DateTime<Utc>,
        never_project_keys: &[String],
        options: WeekOptions,
    ) -> serde_json::Value {
        if !enabled {
            // Owner decision D4, open: nothing is read for Insights.
            return serde_json::json!({"enabled": false, "feed": FEED_COUNTER_PASS});
        }
        let unreadable = |reason: &str| {
            serde_json::json!({
                "enabled": true,
                "feed": FEED_COUNTER_PASS,
                "readable": false,
                "reason": reason,
            })
        };
        // Never creates a key: no key means no pass has stored a row yet.
        let key = match self.keys.load() {
            Ok(key) => key,
            Err(_) => return unreadable(UNREADABLE_KEY),
        };
        let store = match self.load_cached() {
            Ok(store) => store,
            Err(label) => return unreadable(label),
        };
        let inputs: Vec<SessionInput> = match (&key, &store) {
            (Some(key), Some(store)) if store.key_fingerprint == key_fingerprint(key) => {
                let never = never_digests(key, never_project_keys);
                store
                    .rows
                    .values()
                    .filter(|row| !never.contains(&row.project))
                    .map(CounterRow::session_input)
                    .collect()
            }
            // Rows made under another key are not this key's; the next
            // pass starts the store again.
            _ => Vec::new(),
        };
        let week_start = week.unwrap_or_else(|| local_week_start(&now, &tz));
        let this = week_rollup(Feed::CounterPass, &inputs, week_start, &tz);
        let last = week_rollup(
            Feed::CounterPass,
            &inputs,
            this.week_start - Duration::days(7),
            &tz,
        );
        let changes: Vec<serde_json::Value> = this
            .sources
            .iter()
            .map(
                |line| match change_vs_last_week(&this, &last, line.source) {
                    Ok(permille) => serde_json::json!({
                        "source": line.source,
                        "permille": permille,
                        "unavailable": null,
                    }),
                    Err(reason) => serde_json::json!({
                        "source": line.source,
                        "permille": null,
                        "unavailable": reason,
                    }),
                },
            )
            .collect();
        // Every kept week up to the current one, oldest first, whatever week
        // is on screen: goals, the lever and the weekly summary card read
        // them in process (owner decision D4, open).
        let current = local_week_start(&now, &tz);
        let threshold = options.context_threshold.map(u64::from);
        let history: Vec<WeekFigures> = (0..COUNTER_STORE_KEEP_WEEKS)
            .rev()
            .map(|back| {
                week_figures(
                    Feed::CounterPass,
                    &inputs,
                    current - Duration::weeks(back),
                    tz,
                    threshold,
                )
            })
            .collect();
        let earlier: Vec<WeekRollup> = (1..=COUNTER_STORE_KEEP_WEEKS)
            .map(|back| {
                week_rollup(
                    Feed::CounterPass,
                    &inputs,
                    this.week_start - Duration::weeks(back),
                    &tz,
                )
            })
            .filter(|week| week.coverage.sessions() > 0)
            .collect();
        let picker = dated_weeks(&inputs, &tz);
        let overview = counter_overview(&this, &earlier, tz.local_minus_utc(), picker.clone());
        let mut patterns = week_patterns(
            Feed::CounterPass,
            &inputs,
            this.week_start,
            tz,
            PATTERN_BAR_WEEKS,
        );
        patterns.weeks = picker;
        let comparable = comparable(&this);
        let updated_at = self
            .last_pass_at
            .lock()
            .ok()
            .and_then(|at| *at)
            .map(|at| at.to_rfc3339());
        let iso = this.week_start.iso_week();
        serde_json::json!({
            "enabled": true,
            "feed": FEED_COUNTER_PASS,
            "readable": true,
            "updated_at": updated_at,
            "sessions_stored": inputs.len(),
            "iso_week": format!("{}-W{:02}", iso.year(), iso.week()),
            "week_start": this.week_start.to_string(),
            "comparable": comparable.is_ok(),
            "unavailable": comparable.err(),
            "change_vs_last_week": changes,
            "rollup": rollup_value(&this),
            "overview": overview,
            "patterns": patterns,
            "history": history,
            "recap_card_enabled": options.recap_card_enabled,
        })
    }

    /// Remove the store and forget the key. Idempotent. The key is forgotten
    /// whether or not a store was there: a pass makes the key before it knows
    /// whether it will write a row, so a missing store does not mean a
    /// missing key. Forgetting an absent key is not an error.
    pub(crate) fn clear(&self) -> Result<(), &'static str> {
        let mut cached = self.cached.lock().map_err(|_| ERR_CLEAR_FAILED)?;
        match std::fs::remove_file(&self.rows_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(ERR_CLEAR_FAILED),
        }
        *cached = None;
        if let Ok(mut at) = self.last_pass_at.lock() {
            *at = None;
        }
        if self.keys.clear().is_err() {
            // The rows are gone; the next clear tries the key again. A fixed
            // label only.
            tracing::warn!("insights counter key could not be forgotten");
        }
        Ok(())
    }

    /// Whether no key is held, for tests that check a clear forgot it.
    #[cfg(test)]
    pub(crate) fn key_is_absent_for_test(&self) -> bool {
        matches!(self.keys.load(), Ok(None))
    }

    /// The store as cached, read from disk the first time. `Ok(None)` when
    /// there is no store yet; an unreadable store is an error, never empty.
    fn load_cached(&self) -> Result<Option<Arc<CounterStore>>, &'static str> {
        let mut cached = self.cached.lock().map_err(|_| UNREADABLE_STORE)?;
        if let Some(store) = cached.as_ref() {
            return Ok(Some(Arc::clone(store)));
        }
        let bytes = match std::fs::read(&self.rows_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(UNREADABLE_STORE),
        };
        let store: CounterStore = serde_json::from_slice(&bytes).map_err(|_| UNREADABLE_STORE)?;
        if store.schema != COUNTER_STORE_SCHEMA {
            return Err(UNREADABLE_STORE);
        }
        let store = Arc::new(store);
        *cached = Some(Arc::clone(&store));
        Ok(Some(store))
    }

    fn save(&self, store: CounterStore) -> Result<(), &'static str> {
        let body = serde_json::to_vec(&store).map_err(|_| STORE_WRITE_FAILED)?;
        let dir = self.rows_path.parent().ok_or(STORE_WRITE_FAILED)?;
        let mut cached = self.cached.lock().map_err(|_| STORE_WRITE_FAILED)?;
        crate::config::write_atomic_0600(dir, &self.rows_path, &body)
            .map_err(|_| STORE_WRITE_FAILED)?;
        *cached = Some(Arc::new(store));
        Ok(())
    }
}

fn analytics_source(adapter: &str) -> Option<AnalyticsSource> {
    match adapter {
        SOURCE_CLAUDE_CODE => Some(AnalyticsSource::ClaudeCode),
        SOURCE_CODEX => Some(AnalyticsSource::Codex),
        _ => None,
    }
}

fn never_digests(key: &DigestKey, never_project_keys: &[String]) -> BTreeSet<KeyedDigest> {
    if !COUNTER_PASS_EXCLUDES_NEVER_FOLDERS {
        return BTreeSet::new();
    }
    never_project_keys
        .iter()
        .map(|project_key| project_digest(key, project_key))
        .collect()
}

/// One session's counters, or unknown when they cannot be read. Never zero
/// in place of unknown.
fn read_body(source: AnalyticsSource, candidate: &CounterCandidate, key: &DigestKey) -> RowBody {
    let unknown = RowBody::Unknown {
        source,
        reason: UnknownReason::NoUsageCounters,
    };
    // The bound is on the file read; `size_bytes` is the whole group's.
    let Some(bytes) = bounded_read(&candidate.path, COUNTER_PASS_MAX_SESSION_BYTES) else {
        return unknown;
    };
    match source {
        AnalyticsSource::ClaudeCode => {
            match crate::insights::turn_series::extract_claude_turn_series(&bytes, key) {
                Ok(evidence) => {
                    let mut series = evidence.series;
                    // Delegated transcripts are not read: partial, never
                    // complete.
                    if candidate.group_member_count > 0 {
                        series.truncated = true;
                    }
                    RowBody::Claude {
                        model_labels: evidence.model_labels,
                        series,
                    }
                }
                Err(_) => unknown,
            }
        }
        AnalyticsSource::Codex => match extract_codex_usage_evidence(&bytes) {
            Ok(evidence) => RowBody::Codex {
                observed: codex_observed(&evidence),
            },
            Err(_) => unknown,
        },
    }
}

/// Codex's first-to-last observed change. Cached input is a subset of input
/// and `total` is input + output; nothing is added again.
fn codex_observed(evidence: &PersistedUsageEvidence) -> Option<CodexObserved> {
    let interval = evidence.interval.as_ref()?;
    let NativeTokenCounts::Codex {
        input,
        cached_input,
        output,
        total,
        ..
    } = interval.observed_delta
    else {
        return None;
    };
    let baseline_excluded = match interval.unpriced_prior {
        NativeTokenCounts::Codex { total, input, .. } => total > 0 || input > 0,
        NativeTokenCounts::ClaudeCode { .. } => return None,
    };
    Some(CodexObserved {
        first_at: interval.baseline.recorded_at,
        last_at: interval.final_snapshot.recorded_at,
        input,
        cached_input,
        output,
        total,
        baseline_excluded,
    })
}

fn default_counter_keys(store: &ConfigStore) -> Box<dyn DigestKeyStore + Send + Sync> {
    // A test build never reaches a keychain or writes a key file: a past test
    // leak left thousands of orphaned keychain items.
    if cfg!(any(test, feature = "test-credential-store")) {
        return Box::new(InMemoryDigestKeyStore::with_seed([0x7c; 32]));
    }
    match DIGEST_KEY_CUSTODY {
        DigestKeyCustody::OsKeychain => Box::new(OsCounterKeyStore),
        DigestKeyCustody::KeyFileInStore => Box::new(KeyFileInDir(
            crate::insights::turn_series::FileDigestKeyStore::new(
                store.daemon_path(COUNTER_KEY_DIR),
            ),
            store.daemon_path(COUNTER_KEY_DIR),
        )),
    }
}

/// The daemon's counter key in the OS keychain (owner decision D16, open):
/// its own entry, apart from the in-process Insights store's key.
struct OsCounterKeyStore;

impl OsCounterKeyStore {
    fn backend() -> Result<super::os_secret_store::OsSecretBackend, DigestKeyError> {
        crate::insights::turn_series::refuse_in_test_build()?;
        super::os_secret_store::OsSecretBackend::insights_digest_key()
            .map_err(|_| DigestKeyError::Unavailable("insights_counter_key_keychain"))
    }
}

impl DigestKeyStore for OsCounterKeyStore {
    fn load(&self) -> Result<Option<DigestKey>, DigestKeyError> {
        use super::credential_store::CredentialError;
        match Self::backend()?.read_insights_counter_key() {
            Ok(stored) => crate::insights::turn_series::decode_key(&stored).map(Some),
            Err(CredentialError::NoEntry) => Ok(None),
            Err(_) => Err(DigestKeyError::Unavailable("insights_counter_key_keychain")),
        }
    }

    fn load_or_create(&self) -> Result<DigestKey, DigestKeyError> {
        if let Some(key) = self.load()? {
            return Ok(key);
        }
        let bytes = crate::insights::turn_series::new_key_bytes()?;
        Self::backend()?
            .write_insights_counter_key(hex::encode(bytes).as_bytes())
            .map_err(|_| DigestKeyError::Unavailable("insights_counter_key_keychain"))?;
        // Read back: the stored key is the one every digest is made under.
        self.load()?
            .ok_or(DigestKeyError::Unavailable("insights_counter_key_keychain"))
    }

    fn clear(&self) -> Result<(), DigestKeyError> {
        use super::credential_store::CredentialError;
        match Self::backend()?.delete_insights_counter_key() {
            Ok(()) | Err(CredentialError::NoEntry) => Ok(()),
            Err(_) => Err(DigestKeyError::Unavailable("insights_counter_key_keychain")),
        }
    }
}

/// The key-file custody under D16, in a directory of its own made on first
/// use.
struct KeyFileInDir(crate::insights::turn_series::FileDigestKeyStore, PathBuf);

impl DigestKeyStore for KeyFileInDir {
    fn load(&self) -> Result<Option<DigestKey>, DigestKeyError> {
        self.0.load()
    }

    fn load_or_create(&self) -> Result<DigestKey, DigestKeyError> {
        std::fs::create_dir_all(&self.1)
            .map_err(|_| DigestKeyError::Unavailable("insights_counter_key_file"))?;
        self.0.load_or_create()
    }

    fn clear(&self) -> Result<(), DigestKeyError> {
        self.0.clear()
    }
}

/// Drop the oldest sessions, by last write and then by the order they were
/// counted, until every cap holds. Whole sessions go, never part of one.
fn enforce_caps(store: &mut CounterStore, max_sessions: usize, max_turns: usize, max_tools: usize) {
    let mut turns: usize = store.rows.values().map(CounterRow::turns).sum();
    let mut tools: usize = store.rows.values().map(CounterRow::tool_calls).sum();
    if store.rows.len() <= max_sessions && turns <= max_turns && tools <= max_tools {
        return;
    }
    let mut oldest: Vec<(DateTime<Utc>, u64, KeyedDigest)> = store
        .rows
        .iter()
        .map(|(session, row)| (row.modified_at, row.seq, *session))
        .collect();
    oldest.sort();
    for (_, _, session) in oldest {
        if store.rows.len() <= max_sessions && turns <= max_turns && tools <= max_tools {
            break;
        }
        if let Some(row) = store.rows.remove(&session) {
            turns -= row.turns();
            tools -= row.tool_calls();
        }
    }
}

/// At most `max` bytes of `path`, or `None` when it is longer or cannot be
/// read. A file that grows while it is read is refused, not cut.
fn bounded_read(path: &Path, max: u64) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > max {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= max).then_some(bytes)
}

/// `YYYY-Www`, naming a real ISO week. Its Monday.
fn parse_iso_week(text: &str) -> Option<NaiveDate> {
    let bytes = text.as_bytes();
    if bytes.len() != 8
        || bytes[4] != b'-'
        || bytes[5] != b'W'
        || !bytes[..4].iter().all(u8::is_ascii_digit)
        || !bytes[6..].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let year: i32 = text[..4].parse().ok()?;
    let week: u32 = text[6..].parse().ok()?;
    NaiveDate::from_isoywd_opt(year, week, Weekday::Mon)
}

/// The rollup as it crosses the socket: the per-session rows keep their
/// figures and coverage, and lose the opaque reference.
fn rollup_value(rollup: &WeekRollup) -> serde_json::Value {
    let mut value = serde_json::to_value(rollup).unwrap_or(serde_json::Value::Null);
    if let Some(sessions) = value.get_mut("sessions").and_then(|s| s.as_array_mut()) {
        for session in sessions {
            if let Some(object) = session.as_object_mut() {
                object.remove("session_ref");
            }
        }
    }
    value
}

/// Every folder whose own rule is Never.
/// `None` when the folder rules cannot be read: the Never folders are then
/// unknown, and an unknown is never read as "none".
fn never_project_keys(shared: &DaemonShared) -> Option<Vec<String>> {
    let policy = shared.policy.lock().ok()?;
    Some(
        policy
            .projects
            .keys()
            .filter(|key| policy.folder_mode(key) == super::policy::ProjectMode::Ignore)
            .cloned()
            .collect(),
    )
}

/// `insights_week`.
pub fn handle_week(shared: &DaemonShared, req: &Request) -> Response {
    let week = match req.params.get("iso_week") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => match value.as_str().and_then(parse_iso_week) {
            Some(monday) => Some(monday),
            None => return Response::err(req.id, ERR_BAD_PARAMS, ERR_ISO_WEEK_INVALID),
        },
    };
    let tz = match req.params.get("tz") {
        None | Some(serde_json::Value::Null) => chrono::Local::now().offset().fix(),
        Some(_) => match super::insights_glance::parse_tz(&req.params) {
            Some(tz) => tz,
            None => {
                return Response::err(
                    req.id,
                    ERR_BAD_PARAMS,
                    super::insights_glance::ERR_TZ_INVALID,
                );
            }
        },
    };
    // A poisoned lock reads as off: fail closed, never a guess.
    let (enabled, options) = match shared.settings.lock() {
        Ok(settings) => (
            settings.insights_counter_pass,
            WeekOptions {
                recap_card_enabled: settings.insights_recap_card_enabled,
                context_threshold: settings.insights_context_threshold,
            },
        ),
        Err(_) => (false, WeekOptions::default()),
    };
    let never = if enabled {
        match never_project_keys(shared) {
            Some(keys) => keys,
            None => {
                return Response::ok(
                    req.id,
                    serde_json::json!({
                        "enabled": true,
                        "feed": FEED_COUNTER_PASS,
                        "readable": false,
                        "reason": UNREADABLE_POLICY,
                    }),
                );
            }
        }
    } else {
        Vec::new()
    };
    Response::ok(
        req.id,
        shared
            .insights_counter
            .week_value(enabled, week, tz, Utc::now(), &never, options),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use trace_commons_protocol::insights_usage_series::InMemoryDigestKeyStore;

    const QUIET: u64 = 1_800;
    const PROJECT: &str = "/Users/PRIVATE_USER/code/PRIVATE_REPO";

    fn claude_bytes() -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/insights/claude-turn-series/session.jsonl"),
        )
        .unwrap()
    }

    /// One Codex session on Monday 2026-09-14: 80 input (40 cached) and 30
    /// output between the first and last counter, after a nonzero baseline.
    fn codex_bytes() -> Vec<u8> {
        let usage = |at: &str, input: u64, cached: u64, output: u64| {
            serde_json::json!({
                "type": "event_msg",
                "timestamp": at,
                "payload": {"type": "token_count", "info": {"total_token_usage": {
                    "input_tokens": input, "cached_input_tokens": cached,
                    "output_tokens": output, "reasoning_output_tokens": 0,
                    "total_tokens": input + output
                }}}
            })
        };
        [
            serde_json::json!({"type": "session_meta", "payload": {"id": "PRIVATE-CODEX-ID", "model_provider": "openai"}}),
            serde_json::json!({"type": "turn_context", "payload": {"model": "gpt-6"}}),
            usage("2026-09-14T09:00:00Z", 100, 60, 20),
            serde_json::json!({"type": "response_item", "payload": {"type": "message", "role": "assistant", "content": []}}),
            usage("2026-09-14T09:01:00Z", 180, 100, 50),
        ]
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
    }

    /// The fixture's three known turns.
    const CLAUDE_TOKENS: u64 =
        (3 + 12_000 + 40) + (5 + 12_000 + 500 + 60) + (7 + 12_500 + 100 + 20);

    /// Last written 10:00 UTC on Monday 2026-09-14.
    fn written() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 14, 10, 0, 0).unwrap()
    }

    fn now() -> DateTime<Utc> {
        written() + Duration::hours(2)
    }

    fn monday() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 14).unwrap()
    }

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    struct Fixture {
        dir: tempfile::TempDir,
        pass: CounterPass,
    }

    impl Fixture {
        fn new() -> Self {
            Self::keyed(3)
        }

        fn keyed(seed: u8) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let pass = CounterPass::with_keys(
                dir.path().join(COUNTER_ROWS_FILE),
                Box::new(InMemoryDigestKeyStore::with_seed([seed; 32])),
            );
            Self { dir, pass }
        }

        fn rekeyed(&self, seed: u8) -> CounterPass {
            CounterPass::with_keys(
                self.dir.path().join(COUNTER_ROWS_FILE),
                Box::new(InMemoryDigestKeyStore::with_seed([seed; 32])),
            )
        }

        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }

        fn rows_text(&self) -> String {
            std::fs::read_to_string(self.dir.path().join(COUNTER_ROWS_FILE)).unwrap()
        }

        fn run(&self, candidates: &[CounterCandidate]) -> PassSummary {
            run_with(&self.pass, candidates, &[], false)
        }

        fn week(&self, never: &[String]) -> serde_json::Value {
            self.week_with(Some(monday()), never, WeekOptions::default())
        }

        fn week_with(
            &self,
            week: Option<NaiveDate>,
            never: &[String],
            options: WeekOptions,
        ) -> serde_json::Value {
            self.pass
                .week_value(true, week, utc(), now(), never, options)
        }
    }

    fn run_with(
        pass: &CounterPass,
        candidates: &[CounterCandidate],
        never_keys: &[String],
        never: bool,
    ) -> PassSummary {
        pass.run_pass(candidates, never_keys, now(), QUIET, &mut |_| {
            (PROJECT.to_string(), never)
        })
        .unwrap()
    }

    fn candidate(source: &'static str, path: &Path) -> CounterCandidate {
        CounterCandidate {
            origin: 0,
            source,
            path: path.to_path_buf(),
            size_bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(1),
            modified_at: written(),
            started_at: Some(written() - Duration::hours(1)),
            group_member_count: 0,
        }
    }

    fn claude_line(value: &serde_json::Value) -> serde_json::Value {
        value["rollup"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|line| line["source"] == "claude_code")
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }

    #[test]
    fn a_quiesced_session_gets_one_row_holding_nothing_readable() {
        let f = Fixture::new();
        let path = f.write("PRIVATE-SESSION.jsonl", &claude_bytes());
        let summary = f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        assert_eq!(summary.read, 1);
        let text = f.rows_text();
        for forbidden in [
            "PRIVATE",
            "SESSION",
            "/Users",
            path.to_str().unwrap(),
            f.dir.path().to_str().unwrap(),
        ] {
            assert!(!text.contains(forbidden), "{forbidden} stored: {text}");
        }
        let value = f.week(&[]);
        assert_eq!(value["enabled"], true);
        assert_eq!(value["feed"], "counter_pass");
        assert_eq!(value["readable"], true);
        assert_eq!(value["iso_week"], "2026-W38");
        assert_eq!(value["week_start"], "2026-09-14");
        assert_eq!(value["sessions_stored"], 1);
        let line = claude_line(&value);
        assert_eq!(line["tokens"], CLAUDE_TOKENS);
        assert_eq!(line["sessions"], 1);
        assert_eq!(
            value["rollup"]["coverage"]["partial"], 1,
            "two turns unknown"
        );
    }

    #[test]
    fn an_unchanged_session_is_not_read_again_and_a_grown_one_replaces_its_row() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        let first = candidate(SOURCE_CLAUDE_CODE, &path);
        assert_eq!(f.run(&[first]).read, 1);
        let again = candidate(SOURCE_CLAUDE_CODE, &path);
        assert_eq!(f.run(&[again]).read, 0, "same size and time: no read");

        let mut grown = candidate(SOURCE_CLAUDE_CODE, &path);
        grown.size_bytes += 1;
        grown.modified_at = written() + Duration::minutes(1);
        assert_eq!(f.run(&[grown]).read, 1);
        let value = f.week(&[]);
        assert_eq!(value["sessions_stored"], 1, "replaced, not added");
        assert_eq!(claude_line(&value)["tokens"], CLAUDE_TOKENS);
    }

    #[test]
    fn a_session_still_being_written_is_not_read() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        let mut busy = candidate(SOURCE_CLAUDE_CODE, &path);
        busy.modified_at = now() - Duration::seconds(QUIET as i64 - 1);
        assert_eq!(f.run(&[busy]).read, 0);
        assert!(!f.dir.path().join(COUNTER_ROWS_FILE).exists());
    }

    #[test]
    fn only_claude_code_and_codex_are_counted() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        for source in [
            crate::source::SOURCE_TRAJECTORY,
            crate::source::SOURCE_GEMINI_CLI,
            crate::source::SOURCE_CLINE,
            crate::source::SOURCE_OPENCODE,
        ] {
            assert_eq!(f.run(&[candidate(source, &path)]).read, 0, "{source}");
        }
    }

    #[test]
    fn a_never_folder_is_not_read_and_its_rows_are_dropped() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        // A Never folder from the start: not read.
        let summary = run_with(&f.pass, &[candidate(SOURCE_CLAUDE_CODE, &path)], &[], true);
        assert_eq!(summary.read, 0);

        // Counted, then the folder is set to Never: hidden from the next
        // read at once, and dropped by the next pass, even if the session
        // is no longer on disk.
        assert_eq!(f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]).read, 1);
        let never = vec![PROJECT.to_string()];
        let hidden = f.week(&never);
        assert_eq!(hidden["rollup"]["coverage"]["known"], 0);
        assert_eq!(hidden["rollup"]["coverage"]["partial"], 0);
        let summary = run_with(&f.pass, &[], &never, false);
        assert_eq!(summary.dropped, 1);
        assert_eq!(f.week(&[])["sessions_stored"], 0);
    }

    #[test]
    fn reads_per_poll_are_budgeted_and_the_rest_wait() {
        const { assert!(COUNTER_PASS_MAX_READS_PER_TICK < 64) };
        let f = Fixture::new();
        let extra = 3;
        let candidates: Vec<CounterCandidate> = (0..COUNTER_PASS_MAX_READS_PER_TICK + extra)
            .map(|i| {
                let path = f.write(&format!("c{i}.jsonl"), &codex_bytes());
                let mut c = candidate(SOURCE_CODEX, &path);
                // Newest first: the oldest wait.
                c.modified_at = written() - Duration::minutes(i as i64);
                c
            })
            .collect();
        let first = f.run(&candidates);
        assert_eq!(first.read, COUNTER_PASS_MAX_READS_PER_TICK);
        assert_eq!(first.deferred, extra);
        let second = f.run(&candidates);
        assert_eq!(second.read, extra);
        assert_eq!(second.deferred, 0);
    }

    #[test]
    fn an_unreadable_or_oversized_session_is_unknown_never_zero() {
        let f = Fixture::new();
        let missing = candidate(SOURCE_CLAUDE_CODE, &f.dir.path().join("gone.jsonl"));
        // Sparse: one byte over the bound without writing it.
        let big = f.dir.path().join("big.jsonl");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(COUNTER_PASS_MAX_SESSION_BYTES + 1)
            .unwrap();
        let huge = candidate(SOURCE_CODEX, &big);
        assert_eq!(f.run(&[missing, huge]).read, 2);
        let value = f.week(&[]);
        assert_eq!(value["rollup"]["coverage"]["unknown"], 2);
        for line in value["rollup"]["sources"].as_array().unwrap() {
            assert_eq!(line["tokens"], serde_json::Value::Null, "{line}");
        }
    }

    #[test]
    fn a_large_file_is_never_read_past_the_bound() {
        let f = Fixture::new();
        let path = f.write("big.jsonl", &vec![b'x'; 64]);
        assert!(bounded_read(&path, 63).is_none());
        assert_eq!(bounded_read(&path, 64).unwrap().len(), 64);
    }

    #[test]
    fn delegated_transcripts_not_read_make_the_session_partial() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        let mut group = candidate(SOURCE_CLAUDE_CODE, &path);
        group.group_member_count = 2;
        f.run(&[group]);
        let value = f.week(&[]);
        assert_eq!(value["rollup"]["coverage"]["reasons"]["truncated"], 1);
        assert_eq!(value["rollup"]["coverage"]["known"], 0);
    }

    #[test]
    fn a_small_session_with_large_delegated_transcripts_is_partial_not_unknown() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        let mut group = candidate(SOURCE_CLAUDE_CODE, &path);
        group.group_member_count = 2;
        // The group is over the per-session bound; the file read is not.
        group.size_bytes = COUNTER_PASS_MAX_SESSION_BYTES + 1;
        f.run(&[group]);
        let value = f.week(&[]);
        assert_eq!(value["rollup"]["coverage"]["unknown"], 0);
        assert_eq!(value["rollup"]["coverage"]["reasons"]["truncated"], 1);
    }

    #[test]
    fn the_byte_budget_is_charged_by_the_file_read_not_the_group() {
        const SESSIONS: usize = 5;
        const { assert!(COUNTER_PASS_MAX_READS_PER_TICK >= SESSIONS) };
        const {
            assert!(
                COUNTER_PASS_MAX_SESSION_BYTES * (SESSIONS as u64 - 1)
                    >= COUNTER_PASS_MAX_BYTES_PER_TICK
            )
        };
        let f = Fixture::new();
        let candidates: Vec<CounterCandidate> = (0..SESSIONS)
            .map(|i| {
                let path = f.write(&format!("s{i}.jsonl"), &claude_bytes());
                let mut c = candidate(SOURCE_CLAUDE_CODE, &path);
                c.group_member_count = 2;
                c.size_bytes = COUNTER_PASS_MAX_BYTES_PER_TICK;
                c.modified_at = written() - Duration::minutes(i as i64);
                c
            })
            .collect();
        let summary = f.run(&candidates);
        assert_eq!(summary.read, SESSIONS);
        assert_eq!(summary.deferred, 0);
    }

    #[test]
    fn codex_counts_its_observed_change_only() {
        let f = Fixture::new();
        let path = f.write("c.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &path)]);
        let value = f.week(&[]);
        let line = value["rollup"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|line| line["source"] == "codex")
            .cloned()
            .unwrap();
        assert_eq!(line["tokens"], 80 + 30);
        assert_eq!(line["cache_share"]["numerator"], 40);
        assert_eq!(line["cache_share"]["denominator"], 80);
        assert_eq!(
            value["rollup"]["coverage"]["reasons"]["codex_baseline_excluded"],
            1
        );
    }

    #[test]
    fn the_oldest_sessions_go_first_past_a_cap() {
        let mut store = CounterStore::empty(KeyedDigest([0; 32]));
        for i in 0..5u8 {
            store.rows.insert(
                KeyedDigest([i; 32]),
                CounterRow {
                    size_bytes: 1,
                    modified_at: written() + Duration::minutes(i64::from(i)),
                    placed_at: None,
                    project: KeyedDigest([9; 32]),
                    seq: u64::from(i),
                    body: RowBody::Codex { observed: None },
                },
            );
        }
        enforce_caps(&mut store, 3, usize::MAX, usize::MAX);
        let kept: Vec<u8> = store.rows.keys().map(|k| k.0[0]).collect();
        assert_eq!(kept, vec![2, 3, 4]);
    }

    #[test]
    fn the_turn_cap_evicts_whole_sessions() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let mut store = (*f.pass.load_cached().unwrap().unwrap()).clone();
        let turns = store.rows.values().map(CounterRow::turns).sum::<usize>();
        assert!(turns > 1);
        enforce_caps(&mut store, usize::MAX, turns - 1, usize::MAX);
        assert!(store.rows.is_empty());
    }

    #[test]
    fn sessions_older_than_the_kept_weeks_are_not_read_and_old_rows_drop() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        let mut old = candidate(SOURCE_CLAUDE_CODE, &path);
        old.modified_at = now() - Duration::weeks(COUNTER_STORE_KEEP_WEEKS) - Duration::hours(1);
        assert_eq!(f.run(&[old]).read, 0);

        assert_eq!(f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]).read, 1);
        let later = now() + Duration::weeks(COUNTER_STORE_KEEP_WEEKS + 1);
        let summary = f
            .pass
            .run_pass(&[], &[], later, QUIET, &mut |_| {
                (PROJECT.to_string(), false)
            })
            .unwrap();
        assert_eq!(summary.dropped, 1);
    }

    #[test]
    fn a_new_key_starts_the_store_again() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let other = f.rekeyed(4);
        // Rows made under another key are not read as this key's.
        let value = other.week_value(
            true,
            Some(monday()),
            utc(),
            now(),
            &[],
            WeekOptions::default(),
        );
        assert_eq!(value["sessions_stored"], 0);
        assert_eq!(
            run_with(&other, &[candidate(SOURCE_CLAUDE_CODE, &path)], &[], false).read,
            1
        );
        assert_eq!(
            other.week_value(
                true,
                Some(monday()),
                utc(),
                now(),
                &[],
                WeekOptions::default()
            )["sessions_stored"],
            1
        );
    }

    #[test]
    fn a_corrupt_store_reads_as_unreadable_and_the_pass_starts_again() {
        let f = Fixture::new();
        f.write(COUNTER_ROWS_FILE, b"{not json");
        let value = f.week(&[]);
        assert_eq!(
            value,
            serde_json::json!({"enabled": true, "feed": "counter_pass", "readable": false, "reason": "store_unreadable"})
        );
        let path = f.write("s.jsonl", &claude_bytes());
        assert_eq!(f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]).read, 1);
        assert_eq!(f.week(&[])["readable"], true);
    }

    #[test]
    fn clear_removes_the_rows_and_forgets_the_key() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        assert!(f.pass.keys.load().unwrap().is_some());
        f.pass.clear().unwrap();
        assert!(!f.dir.path().join(COUNTER_ROWS_FILE).exists());
        assert!(f.pass.keys.load().unwrap().is_none());
        assert_eq!(f.week(&[])["sessions_stored"], 0);
        // Idempotent.
        f.pass.clear().unwrap();
    }

    #[test]
    fn clear_forgets_a_key_made_by_a_pass_that_wrote_no_rows() {
        let f = Fixture::new();
        f.run(&[]);
        assert!(f.pass.keys.load().unwrap().is_some(), "the pass made a key");
        assert!(!f.dir.path().join(COUNTER_ROWS_FILE).exists());
        f.pass.clear().unwrap();
        assert!(f.pass.keys.load().unwrap().is_none());
    }

    #[test]
    fn with_the_setting_off_nothing_is_read_or_created() {
        let f = Fixture::new();
        let value = f
            .pass
            .week_value(false, None, utc(), now(), &[], WeekOptions::default());
        assert_eq!(
            value,
            serde_json::json!({"enabled": false, "feed": "counter_pass"})
        );
        assert!(f.pass.keys.load().unwrap().is_none(), "no key made");
        assert!(!f.dir.path().join(COUNTER_ROWS_FILE).exists());
    }

    #[test]
    fn a_read_never_creates_a_key() {
        let f = Fixture::new();
        let value = f.week(&[]);
        assert_eq!(value["readable"], true);
        assert_eq!(value["sessions_stored"], 0);
        assert_eq!(value["updated_at"], serde_json::Value::Null);
        assert!(f.pass.keys.load().unwrap().is_none());
    }

    #[test]
    fn comparisons_need_comparable_weeks() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let value = f.week(&[]);
        assert_eq!(value["comparable"], false);
        assert_eq!(value["unavailable"], "below_coverage_floor");
        assert_eq!(
            value["change_vs_last_week"],
            serde_json::json!([{"source": "claude_code", "permille": null, "unavailable": "below_coverage_floor"}])
        );
    }

    #[test]
    fn nothing_identifying_crosses() {
        let f = Fixture::new();
        let path = f.write("PRIVATE-SESSION.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let key = f.pass.keys.load().unwrap().unwrap();
        let session = hex::encode(session_key(&key, SOURCE_CLAUDE_CODE, &path.to_string_lossy()).0);
        let project = hex::encode(project_digest(&key, PROJECT).0);
        let text = f.week(&[]).to_string();
        for forbidden in [
            "PRIVATE",
            "/Users",
            "session_ref",
            "msg_key",
            session.as_str(),
            &session[..16],
            project.as_str(),
            &project[..16],
            path.to_str().unwrap(),
        ] {
            assert!(!text.contains(forbidden), "{forbidden} crossed: {text}");
        }
    }

    #[test]
    fn the_answer_carries_the_cores_overview_and_patterns_for_feed_t() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let value = f.week(&[]);
        let overview = &value["overview"];
        assert_eq!(overview["feed"], "counter_pass");
        assert_eq!(overview["week_start"], "2026-09-14");
        assert_eq!(overview["week_end"], "2026-09-20");
        assert_eq!(overview["sources"][0]["source"], "claude_code");
        assert_eq!(overview["sources"][0]["tokens"], CLAUDE_TOKENS);
        assert_eq!(overview["sources"][0]["change"], "below_coverage_floor");
        assert_eq!(overview["sources"][0]["best_week"], "below_coverage_floor");
        assert_eq!(overview["by_project"], "held_for_project_decision");
        assert_eq!(overview["weeks"], serde_json::json!(["2026-09-14"]));
        let patterns = &value["patterns"];
        assert_eq!(patterns["feed"], "counter_pass");
        assert_eq!(patterns["week_start"], "2026-09-14");
        assert_eq!(patterns["cards"].as_array().unwrap().len(), 4);
        assert_eq!(patterns["weeks"], serde_json::json!(["2026-09-14"]));
        assert_eq!(
            value["recap_card_enabled"],
            crate::insights::analytics_constants::RECAP_CARD_DEFAULT_ON
        );
    }

    #[test]
    fn history_runs_to_the_current_week_whatever_week_is_asked_for() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        for asked in [Some(monday()), Some(monday() - Duration::weeks(3)), None] {
            let value = f.week_with(asked, &[], WeekOptions::default());
            let history = value["history"].as_array().unwrap();
            assert_eq!(history.len(), COUNTER_STORE_KEEP_WEEKS as usize);
            assert_eq!(history.last().unwrap()["week_start"], "2026-09-14");
            assert_eq!(
                history.first().unwrap()["week_start"],
                (monday() - Duration::weeks(COUNTER_STORE_KEEP_WEEKS - 1)).to_string()
            );
            let this = history.last().unwrap();
            assert_eq!(this["tokens"]["claude_code"], CLAUDE_TOKENS);
            assert_eq!(this["sessions"], 1);
            assert_eq!(this["comparable"], false);
            assert_eq!(this["past_threshold"], serde_json::Value::Null);
            // An earlier week with no session has no figure, never zero.
            assert_eq!(history[0]["tokens"], serde_json::json!({}));
            assert_eq!(history[0]["sessions"], 0);
        }
    }

    #[test]
    fn the_threshold_count_and_the_summary_switch_follow_the_settings() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let value = f.week_with(
            Some(monday()),
            &[],
            WeekOptions {
                recap_card_enabled: false,
                context_threshold: Some(12_000),
            },
        );
        assert_eq!(value["recap_card_enabled"], false);
        assert_eq!(
            value["history"].as_array().unwrap().last().unwrap()["past_threshold"],
            serde_json::json!({"threshold": 12_000, "sessions": 1})
        );
        let higher = f.week_with(
            Some(monday()),
            &[],
            WeekOptions {
                recap_card_enabled: true,
                context_threshold: Some(1_000_000),
            },
        );
        assert_eq!(
            higher["history"].as_array().unwrap().last().unwrap()["past_threshold"]["sessions"],
            0
        );
    }

    #[test]
    fn a_never_folder_leaves_no_figure_in_the_overview_or_history() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let value = f.week(&[PROJECT.to_string()]);
        assert_eq!(value["overview"]["sources"], serde_json::json!([]));
        let history = value["history"].as_array().unwrap();
        assert!(history.iter().all(|week| week["sessions"] == 0));
    }

    #[test]
    fn the_week_is_a_strict_iso_week() {
        assert_eq!(parse_iso_week("2026-W38"), Some(monday()));
        assert_eq!(
            parse_iso_week("2026-W01"),
            NaiveDate::from_ymd_opt(2025, 12, 29)
        );
        for bad in [
            "2026-W54",
            "2026-W00",
            "2026-38",
            "2026-w38",
            "26-W38",
            "2026-W3",
            "2026-W038",
            " 2026-W38",
            "2026-W38 ",
        ] {
            assert_eq!(parse_iso_week(bad), None, "{bad}");
        }
    }

    fn shared() -> (tempfile::TempDir, DaemonShared) {
        let (dir, store) = crate::config::tests_support::temp_store();
        (dir, DaemonShared::load(store).unwrap())
    }

    fn call(params: serde_json::Value) -> Request {
        Request {
            id: 1,
            method: "insights_week".to_string(),
            params,
        }
    }

    #[test]
    fn the_method_answers_off_by_default_and_refuses_bad_params() {
        let (_dir, s) = shared();
        let r = handle_week(&s, &call(serde_json::json!({})));
        assert_eq!(
            r.result.unwrap(),
            serde_json::json!({"enabled": false, "feed": "counter_pass"})
        );
        for on in [false, true] {
            s.settings.lock().unwrap().insights_counter_pass = on;
            let bad_week = handle_week(&s, &call(serde_json::json!({"iso_week": "last"})));
            let err = bad_week.error.unwrap();
            assert_eq!(err.code, ERR_BAD_PARAMS);
            assert_eq!(err.message, ERR_ISO_WEEK_INVALID);
            let bad_tz = handle_week(&s, &call(serde_json::json!({"tz": "UTC"})));
            assert_eq!(
                bad_tz.error.unwrap().message,
                super::super::insights_glance::ERR_TZ_INVALID
            );
        }
        let on = handle_week(
            &s,
            &call(serde_json::json!({"iso_week": "2026-W38", "tz": 0})),
        )
        .result
        .unwrap();
        assert_eq!(on["readable"], true);
        assert_eq!(on["week_start"], "2026-09-14");
    }

    /// The Never folders cannot be read, so nothing is shown: a poisoned
    /// policy lock must not read as "no folder is Never" and let a Never
    /// folder's rows through.
    #[test]
    fn an_unreadable_folder_policy_shows_nothing() {
        let (_dir, s) = shared();
        s.settings.lock().unwrap().insights_counter_pass = true;
        std::thread::scope(|scope| {
            let _ = scope
                .spawn(|| {
                    let _held = s.policy.lock().unwrap();
                    panic!("poison the policy lock");
                })
                .join();
        });
        assert!(s.policy.is_poisoned());
        let r = handle_week(
            &s,
            &call(serde_json::json!({"iso_week": "2026-W38", "tz": 0})),
        )
        .result
        .unwrap();
        assert_eq!(
            r,
            serde_json::json!({
                "enabled": true,
                "feed": "counter_pass",
                "readable": false,
                "reason": UNREADABLE_POLICY,
            })
        );
    }

    #[test]
    fn the_method_is_advertised_dispatched_and_local() {
        assert!(super::super::ipc::METHODS.contains(&"insights_week"));
        assert!(super::super::ipc::DEV_DRY_RUN_LOCAL_METHODS.contains(&"insights_week"));
        let (_dir, mut s) = shared();
        s.dev_dry_run = true;
        let r = super::super::ipc::handle_request(&s, &call(serde_json::json!({})));
        assert_eq!(r.result.unwrap()["enabled"], false);
    }
}
