//! `insights_submitted`: every trace the contributor has submitted, whatever
//! became of it (accepted, quarantined, withdrawn, rejected or still
//! pending), joined to what the counter pass (feed T) stored about its
//! session. Product owner, 2026-10-29: "analyse submitted work, not even
//! admitted work, all the traces submitted".
//!
//! Gated as `insights_benchmarks` is: off answers `enabled: false` without
//! opening the store or the key; on, a store, key, folder-rule or history
//! read that fails answers `readable: false` with its fixed reason, never an
//! empty answer. A submission from a Never folder enters no answer: it is
//! counted in `left_out.never_folder` only.
//!
//! # The join
//!
//! The history cache (`daemon-history.jsonl`) holds every submission with
//! its status and credit. Its `session_hash` is a hash of the session's
//! bytes at upload, which the counter store does not keep: the store keys
//! each row by a keyed digest of the harness and the session file's
//! address. The upload queue ties the two: the entry a submission was
//! uploaded from carries its `submission_id`, its adapter and the file's
//! path, and an `Uploaded` entry is never expired or compacted (the History
//! view joins on it), so the join outlives `queue_ttl_days`. A submission
//! is found in the queue by its `submission_id`, else by its
//! `session_hash` and adapter; its row by the keyed digest of that adapter
//! and path, as the pass made it. Nothing is guessed: a submission with no
//! queue entry, of an adapter the pass does not read, or whose row is gone
//! (aged out of the store's 13 weeks, evicted, or not read yet) is counted
//! as unmatched with its reason, and still counted in every history
//! figure. A row describes its session file as the pass last read it,
//! which for a session continued after upload holds more than was sent.
//!
//! No minimum: one submitted trace answers every figure, with the counts
//! behind it, and the shell decides what is too few to compare.
//!
//! # What crosses the socket
//!
//! Counts, credit points as the history stores them (summed, rounded to
//! 0.001), USD micros at API list prices, fixed snake_case keys, declared
//! model labels, the project labels History already shows, the server's
//! credit outcome labels, and each submission's `submission_id`, the
//! reference History rows already carry. Never a path, a file name, a
//! session ID or hash, a project key or digest, a prompt or a command.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Offset, TimeZone, Utc};
use uuid::Uuid;

use super::history::{
    HistoryCache, HistoryRecord, STATUS_ACCEPTED, STATUS_QUARANTINED, STATUS_REJECTED,
    STATUS_SUBMITTED, is_taken_back, status_bucket,
};
use super::insights_sessions::gate;
use super::insights_week::submitted::{StoredAddress, StoredLookup, StoredSession, tables_of};
use super::insights_week::{CounterPass, FEED_COUNTER_PASS, NeverFolders};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, Request, Response};
use crate::insights::analytics_constants::COUNTER_PASS_EXCLUDES_NEVER_FOLDERS;
use crate::insights::benchmarks::BenchCategory;
use crate::insights::session_facts::ExtensionClassCounts;
use crate::insights::week_rollup::local_week_start;
use crate::source::{SOURCE_CLAUDE_CODE, SOURCE_CODEX};

/// `weeks` that is not 4, 13 or null.
pub const ERR_SUBMITTED_WEEKS_INVALID: &str = "weeks_invalid";
/// The windows the method answers over, in local weeks; null is all time.
pub const SUBMITTED_WEEKS: [u32; 2] = [4, 13];
/// The most `traces` rows an answer carries, newest first; `traces_total`
/// counts them all.
pub const SUBMITTED_MAX_TRACES: usize = 500;
/// The most `by_week` rows an all-time answer carries, newest kept.
pub const SUBMITTED_MAX_WEEKS: usize = 520;
/// The history cache could not be read; nothing is shown in its place.
pub const UNREADABLE_HISTORY: &str = "history_unreadable";

/// `unmatched_by_reason`: no queue entry carries the submission.
const NO_QUEUE_ENTRY: &str = "no_queue_entry";
/// `unmatched_by_reason`: an adapter the counter pass does not read.
const NOT_READ: &str = "source_not_read";
/// `unmatched_by_reason`: the queue names the session, the store holds no
/// row for it (aged out, evicted, or not read yet).
const NO_SESSION_ROW: &str = "no_session_row";

/// One upload queue entry, as far as the join needs it. Never crosses.
#[derive(Debug, Clone)]
pub(crate) struct QueuedSubmission {
    pub submission_id: Option<Uuid>,
    pub uploaded: bool,
    pub session_hash: String,
    pub source: String,
    pub path: PathBuf,
    pub project_key: String,
}

/// The answer's settings.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SubmittedQuery {
    /// Local weeks ending this one; `None` is all time.
    pub weeks: Option<u32>,
    pub tz: FixedOffset,
    pub now: DateTime<Utc>,
    /// Whether route tallies may be read (the ledger feed is on and a
    /// ledger is declared).
    pub routing_available: bool,
}

/// `weeks`: 4, 13 or null (all time); absent is all time.
pub(crate) fn submitted_weeks(params: &serde_json::Value) -> Result<Option<u32>, &'static str> {
    match params.get("weeks") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|weeks| u32::try_from(weeks).ok())
            .filter(|weeks| SUBMITTED_WEEKS.contains(weeks))
            .map(Some)
            .ok_or(ERR_SUBMITTED_WEEKS_INVALID),
    }
}

/// `insights_submitted`.
pub fn handle_submitted(shared: &DaemonShared, req: &Request) -> Response {
    let weeks = match submitted_weeks(&req.params) {
        Ok(weeks) => weeks,
        Err(label) => return Response::err(req.id, ERR_BAD_PARAMS, label),
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
    let (enabled, never) = match gate(shared) {
        Ok(gate) => gate,
        Err(answer) => return Response::ok(req.id, answer),
    };
    if !enabled {
        return Response::ok(req.id, off_value());
    }
    // A poisoned lock reads as off: fail closed, never a guess.
    let ledger_feed = shared
        .settings
        .lock()
        .map(|settings| settings.insights_ledger_feed)
        .unwrap_or(false);
    let routing_available = ledger_feed && super::insights_week::any_ledger(shared);
    let history = HistoryCache::load(&shared.store).ok();
    let queue: Vec<QueuedSubmission> = match shared.queue.lock() {
        Ok(queue) => queue
            .all()
            .iter()
            .filter(|entry| {
                entry.submission_id.is_some() || entry.state == super::queue::QueueState::Uploaded
            })
            .map(|entry| QueuedSubmission {
                submission_id: entry.submission_id,
                uploaded: entry.state == super::queue::QueueState::Uploaded,
                session_hash: entry.session_hash.clone(),
                source: entry.source.clone(),
                path: entry.path.clone(),
                project_key: entry.project_key.clone(),
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    let query = SubmittedQuery {
        weeks,
        tz,
        now: Utc::now(),
        routing_available,
    };
    Response::ok(
        req.id,
        submitted_value(
            &shared.insights_counter,
            enabled,
            &never,
            history.as_deref(),
            &queue,
            &query,
        ),
    )
}

fn off_value() -> serde_json::Value {
    serde_json::json!({"enabled": false, "feed": FEED_COUNTER_PASS})
}

fn unreadable(reason: &str) -> serde_json::Value {
    serde_json::json!({
        "enabled": true,
        "feed": FEED_COUNTER_PASS,
        "readable": false,
        "reason": reason,
    })
}

/// The status a submission is counted under: `withdrawn` when taken back
/// (here or on the web), else the server's status with `processing` read as
/// `pending` (`status_bucket`); a status this build does not know is
/// `other`.
fn status_key(record: &HistoryRecord) -> &'static str {
    if is_taken_back(record) {
        return "withdrawn";
    }
    match status_bucket(&record.status) {
        STATUS_ACCEPTED => "accepted",
        STATUS_QUARANTINED => "quarantined",
        STATUS_REJECTED => "rejected",
        STATUS_SUBMITTED => "pending",
        _ => "other",
    }
}

const STATUS_KEYS: [&str; 6] = [
    "accepted",
    "pending",
    "quarantined",
    "rejected",
    "withdrawn",
    "other",
];

fn task_key(task: BenchCategory) -> &'static str {
    match task {
        BenchCategory::BugFix => "bug_fix",
        BenchCategory::Feature => "feature",
        BenchCategory::Refactor => "refactor",
        BenchCategory::Tests => "tests",
        BenchCategory::Docs => "docs",
        BenchCategory::Understand => "understand",
        BenchCategory::Review => "review",
        BenchCategory::Setup => "setup",
    }
}

/// Credit points summed in `f64` and rounded to 0.001, so a sum of the
/// history's `f32` figures crosses as the figure a person would add up.
fn points(sum: f64) -> f64 {
    (sum * 1_000.0).round() / 1_000.0
}

/// Submitted, accepted and credit, for any grouping.
#[derive(Debug, Default, Clone)]
struct Tally {
    submitted: u64,
    accepted: u64,
    credit_final: f64,
    credit_pending: f64,
    matched: u64,
}

impl Tally {
    fn add(&mut self, record: &HistoryRecord, matched: bool) {
        self.submitted += 1;
        if status_key(record) == "accepted" {
            self.accepted += 1;
        }
        // A final figure, 0 included, means decided: whatever pending
        // figure the server still reports beside it is not pending (the
        // History rollup's rule).
        match record.credit_points_final {
            Some(points) => self.credit_final += f64::from(points),
            None => self.credit_pending += f64::from(record.credit_points_pending),
        }
        if matched {
            self.matched += 1;
        }
    }

    fn fields(&self, object: &mut serde_json::Map<String, serde_json::Value>) {
        object.insert("submitted".into(), self.submitted.into());
        object.insert("accepted".into(), self.accepted.into());
        object.insert("credit_final".into(), points(self.credit_final).into());
        object.insert("credit_pending".into(), points(self.credit_pending).into());
        object.insert("matched".into(), self.matched.into());
    }

    fn value_with(&self, extra: serde_json::Value) -> serde_json::Value {
        let mut object = match extra {
            serde_json::Value::Object(object) => object,
            _ => serde_json::Map::new(),
        };
        self.fields(&mut object);
        serde_json::Value::Object(object)
    }
}

/// A count known in part: the known sum and whether any was unknown.
#[derive(Debug, Default, Clone, Copy)]
struct Known {
    known: u64,
    partial: bool,
}

impl Known {
    fn add(&mut self, value: Option<u32>) {
        match value {
            Some(value) => self.known += u64::from(value),
            None => self.partial = true,
        }
    }
}

/// Edits kept: edits less undone and declined, each session clamped at 0;
/// unknown when its edits or undone are (declined unknown reads as none
/// declined, as Benchmarks reads it).
fn edits_kept(session: &StoredSession) -> Option<u32> {
    let edits = session.edits?;
    let undone = session.undone?;
    Some(edits.saturating_sub(undone.saturating_add(session.denied.unwrap_or(0))))
}

/// Edit and line counts over a set of sessions, each known part beside its
/// own `partial` flag, as `insights_week`'s `work` gives them.
#[derive(Debug, Default, Clone)]
struct Edits {
    edits: Known,
    kept: Known,
    undone: Known,
    denied: Known,
    lines_added: Known,
    lines_removed: Known,
}

impl Edits {
    fn add(&mut self, session: &StoredSession) {
        self.edits.add(session.edits);
        self.kept.add(edits_kept(session));
        self.undone.add(session.undone);
        self.denied.add(session.denied);
        self.lines_added.add(session.lines_added);
        self.lines_removed.add(session.lines_removed);
    }

    fn fields(&self, object: &mut serde_json::Map<String, serde_json::Value>) {
        let pairs = [
            ("edits", self.edits),
            ("edits_kept", self.kept),
            ("edits_undone", self.undone),
            ("edits_denied", self.denied),
        ];
        for (name, count) in pairs {
            object.insert(format!("{name}_known"), count.known.into());
            object.insert(format!("{name}_partial"), count.partial.into());
        }
        object.insert("lines_added_known".into(), self.lines_added.known.into());
        object.insert(
            "lines_removed_known".into(),
            self.lines_removed.known.into(),
        );
        object.insert(
            "lines_partial".into(),
            (self.lines_added.partial || self.lines_removed.partial).into(),
        );
    }
}

/// Spend at list prices over a set of sessions.
#[derive(Debug, Default, Clone, Copy)]
struct Spend {
    usd_micros: u64,
    priced: u64,
    unpriced: u64,
    near_ai: u64,
}

impl Spend {
    fn add(&mut self, session: &StoredSession) {
        if session.near_ai {
            self.near_ai += 1;
        } else if let Some(micros) = session.usd_micros {
            self.usd_micros = self.usd_micros.saturating_add(micros);
            self.priced += 1;
        } else {
            self.unpriced += 1;
        }
    }

    fn fields(&self, object: &mut serde_json::Map<String, serde_json::Value>) {
        object.insert("usd_micros_known".into(), self.usd_micros.into());
        object.insert("usd_partial".into(), (self.unpriced > 0).into());
        object.insert("priced_sessions".into(), self.priced.into());
        object.insert("unpriced_sessions".into(), self.unpriced.into());
        object.insert("near_ai_sessions".into(), self.near_ai.into());
    }
}

fn files_value(files: &ExtensionClassCounts) -> serde_json::Value {
    serde_json::json!({
        "source": files.source,
        "test": files.test,
        "doc": files.doc,
        "config": files.config,
        "total": files.total(),
    })
}

/// One model's submitted sessions.
#[derive(Debug, Default)]
struct ModelRow {
    tally: Tally,
    spend: Spend,
    edits: Edits,
    tasks: BTreeMap<&'static str, u64>,
}

/// The local Monday a window of `weeks` ending `now`'s week starts on, and
/// that midnight as an instant.
fn window_start(weeks: u32, now: DateTime<Utc>, tz: &FixedOffset) -> (NaiveDate, DateTime<Utc>) {
    let monday = local_week_start(&now, tz) - Duration::weeks(i64::from(weeks.max(1)) - 1);
    let midnight = tz
        .from_local_datetime(&monday.and_hms_opt(0, 0, 0).unwrap_or_default())
        .single()
        .map(|at| at.with_timezone(&Utc))
        .unwrap_or(now);
    (monday, midnight)
}

/// The answer, from the history cache (`None`: unreadable), the queue
/// entries that carry a submission, and the counter store.
pub(crate) fn submitted_value(
    pass: &CounterPass,
    enabled: bool,
    never: &NeverFolders,
    history: Option<&[HistoryRecord]>,
    queue: &[QueuedSubmission],
    query: &SubmittedQuery,
) -> serde_json::Value {
    if !enabled {
        return off_value();
    }
    let Some(history) = history else {
        return unreadable(UNREADABLE_HISTORY);
    };
    let window = query
        .weeks
        .map(|weeks| window_start(weeks, query.now, &query.tz));
    let in_window: Vec<&HistoryRecord> = history
        .iter()
        .filter(|record| window.is_none_or(|(_, start)| record.submitted_at >= start))
        .collect();

    // Never folders, by the keys the queue holds and by the opaque project
    // id the history row carries (a one-way digest of the same key).
    let (never_every, never_keys): (bool, BTreeSet<&str>) = match never {
        _ if !COUNTER_PASS_EXCLUDES_NEVER_FOLDERS => (false, BTreeSet::new()),
        NeverFolders::Every => (true, BTreeSet::new()),
        NeverFolders::These(keys) => (false, keys.iter().map(String::as_str).collect()),
    };
    let never_ids: BTreeSet<String> = never_keys
        .iter()
        .map(|key| super::policy::project_id_for(key))
        .collect();

    // The queue entry each submission was uploaded from.
    let by_id: BTreeMap<Uuid, &QueuedSubmission> = queue
        .iter()
        .filter_map(|entry| entry.submission_id.map(|id| (id, entry)))
        .collect();
    let mut by_hash: BTreeMap<(&str, &str), &QueuedSubmission> = BTreeMap::new();
    for entry in queue {
        let slot = by_hash
            .entry((entry.session_hash.as_str(), entry.source.as_str()))
            .or_insert(entry);
        if entry.uploaded && !slot.uploaded {
            *slot = entry;
        }
    }
    let entries: Vec<Option<&QueuedSubmission>> = in_window
        .iter()
        .map(|record| {
            by_id.get(&record.submission_id).copied().or_else(|| {
                by_hash
                    .get(&(record.session_hash.as_str(), record.source.as_str()))
                    .copied()
            })
        })
        .collect();
    let read = |source: &str| source == SOURCE_CLAUDE_CODE || source == SOURCE_CODEX;
    let addresses: Vec<StoredAddress<'_>> = entries
        .iter()
        .filter_map(|entry| *entry)
        .filter(|entry| read(&entry.source))
        .map(|entry| StoredAddress {
            source: &entry.source,
            path: &entry.path,
        })
        .collect();
    let lookups = match pass.submitted_lookups(&addresses, never, query.routing_available) {
        Ok(lookups) => lookups,
        Err(reason) => return unreadable(reason),
    };
    let mut lookups = lookups.into_iter();

    let mut never_folder = 0u64;
    let mut rows: Vec<Row<'_>> = Vec::new();
    for (record, entry) in in_window.iter().zip(&entries) {
        let lookup = match entry {
            Some(entry) if read(&entry.source) => lookups.next(),
            _ => None,
        };
        let in_never = never_every
            || never_ids.contains(&record.project_id)
            || entry.is_some_and(|entry| never_keys.contains(entry.project_key.as_str()))
            || lookup == Some(StoredLookup::NeverFolder);
        if in_never {
            never_folder += 1;
            continue;
        }
        let (session, reason) = match (entry, lookup) {
            (None, _) => (None, Some(NO_QUEUE_ENTRY)),
            (Some(_), None) => (None, Some(NOT_READ)),
            (Some(_), Some(StoredLookup::Found(session))) => (Some(*session), None),
            (Some(_), Some(_)) => (None, Some(NO_SESSION_ROW)),
        };
        rows.push((*record, session, reason));
    }
    // Newest first, as History lists them.
    rows.sort_by(|a, b| {
        b.0.submitted_at
            .cmp(&a.0.submitted_at)
            .then(b.0.submission_id.cmp(&a.0.submission_id))
    });
    build_answer(pass, &rows, window, query, never_folder)
}

type Row<'a> = (
    &'a HistoryRecord,
    Option<StoredSession>,
    Option<&'static str>,
);

fn build_answer(
    pass: &CounterPass,
    rows: &[Row<'_>],
    window: Option<(NaiveDate, DateTime<Utc>)>,
    query: &SubmittedQuery,
    never_folder: u64,
) -> serde_json::Value {
    let mut totals = Tally::default();
    let mut by_status: BTreeMap<&str, u64> = STATUS_KEYS.iter().map(|key| (*key, 0)).collect();
    let mut by_reason: BTreeMap<String, u64> = BTreeMap::new();
    let mut unmatched: BTreeMap<&str, u64> = [NO_QUEUE_ENTRY, NOT_READ, NO_SESSION_ROW]
        .into_iter()
        .map(|key| (key, 0))
        .collect();
    let mut by_model: BTreeMap<(String, &'static str), ModelRow> = BTreeMap::new();
    let mut model_left_out: BTreeMap<&str, u64> =
        [("unmatched", 0), ("mixed_models", 0), ("no_model", 0)]
            .into_iter()
            .collect();
    let mut by_task: BTreeMap<BenchCategory, Tally> = BenchCategory::ALL
        .iter()
        .map(|task| (*task, Tally::default()))
        .collect();
    let mut task_unknown = Tally::default();
    let mut files = ExtensionClassCounts::default();
    let mut files_sessions = 0u64;
    let mut code_edits = Edits::default();
    let mut spend = Spend::default();
    let mut by_week: BTreeMap<NaiveDate, Tally> = BTreeMap::new();
    let mut by_project: BTreeMap<&str, Tally> = BTreeMap::new();
    let mut traces = Vec::new();

    for (record, session, reason) in rows {
        let session = session.as_ref();
        let matched = session.is_some();
        totals.add(record, matched);
        *by_status.entry(status_key(record)).or_insert(0) += 1;
        if let Some(reason) = &record.credit_outcome_reason {
            *by_reason.entry(reason.clone()).or_insert(0) += 1;
        }
        by_week
            .entry(local_week_start(&record.submitted_at, &query.tz))
            .or_default()
            .add(record, matched);
        by_project
            .entry(record.project_label.as_str())
            .or_default()
            .add(record, matched);
        match session {
            None => {
                *unmatched
                    .entry(reason.unwrap_or(NO_SESSION_ROW))
                    .or_insert(0) += 1;
                *model_left_out.entry("unmatched").or_insert(0) += 1;
                task_unknown.add(record, false);
            }
            Some(session) => {
                spend.add(session);
                code_edits.add(session);
                if let Some(kinds) = &session.files {
                    files_sessions += 1;
                    files.test = files.test.saturating_add(kinds.test);
                    files.doc = files.doc.saturating_add(kinds.doc);
                    files.source = files.source.saturating_add(kinds.source);
                    files.config = files.config.saturating_add(kinds.config);
                }
                match session.task {
                    Some(task) => by_task.entry(task).or_default().add(record, true),
                    None => task_unknown.add(record, true),
                }
                match &session.model {
                    Some(model) => {
                        let row = by_model.entry((model.clone(), session.tool)).or_default();
                        row.tally.add(record, true);
                        row.spend.add(session);
                        row.edits.add(session);
                        if let Some(task) = session.task {
                            *row.tasks.entry(task_key(task)).or_insert(0) += 1;
                        }
                    }
                    None if session.mixed_models => {
                        *model_left_out.entry("mixed_models").or_insert(0) += 1;
                    }
                    None => *model_left_out.entry("no_model").or_insert(0) += 1,
                }
            }
        }
        if traces.len() < SUBMITTED_MAX_TRACES {
            traces.push(trace_value(record, session, *reason));
        }
    }

    // Every local week from the window's first (or the earliest
    // submission's) to this one, empty weeks included.
    let this_week = local_week_start(&query.now, &query.tz);
    let first_week = window
        .map(|(monday, _)| monday)
        .or_else(|| by_week.keys().next().copied())
        .unwrap_or(this_week)
        .min(this_week);
    let last_week = by_week
        .keys()
        .next_back()
        .copied()
        .unwrap_or(this_week)
        .max(this_week);
    let mut weeks_out = Vec::new();
    let mut week = last_week;
    while week >= first_week && weeks_out.len() < SUBMITTED_MAX_WEEKS {
        let tally = by_week.get(&week).cloned().unwrap_or_default();
        weeks_out.push(tally.value_with(serde_json::json!({"week_start": week.to_string()})));
        week -= Duration::weeks(1);
    }
    weeks_out.reverse();

    let mut models: Vec<(&(String, &'static str), &ModelRow)> = by_model.iter().collect();
    models.sort_by(|a, b| {
        b.1.tally
            .submitted
            .cmp(&a.1.tally.submitted)
            .then(a.0.cmp(b.0))
    });
    let models: Vec<serde_json::Value> = models
        .into_iter()
        .map(|((label, tool), row)| {
            let mut object = serde_json::Map::new();
            object.insert("label".into(), label.clone().into());
            object.insert("tool".into(), (*tool).into());
            row.tally.fields(&mut object);
            row.spend.fields(&mut object);
            row.edits.fields(&mut object);
            object.insert("tasks".into(), serde_json::json!(row.tasks));
            serde_json::Value::Object(object)
        })
        .collect();
    let tasks: Vec<serde_json::Value> = by_task
        .iter()
        .map(|(task, tally)| tally.value_with(serde_json::json!({"key": task_key(*task)})))
        .collect();
    let mut projects: Vec<(&&str, &Tally)> = by_project.iter().collect();
    projects.sort_by(|a, b| b.1.submitted.cmp(&a.1.submitted).then(a.0.cmp(b.0)));
    let projects: Vec<serde_json::Value> = projects
        .into_iter()
        .map(|(label, tally)| tally.value_with(serde_json::json!({"label": label})))
        .collect();

    let mut code = serde_json::Map::new();
    code.insert("sessions".into(), totals.matched.into());
    code.insert("sessions_with_files".into(), files_sessions.into());
    code.insert("files".into(), files_value(&files));
    code.insert(
        "files_partial".into(),
        (files_sessions < totals.matched).into(),
    );
    code_edits.fields(&mut code);
    spend.fields(&mut code);

    let mut totals_object = serde_json::Map::new();
    totals.fields(&mut totals_object);
    totals_object.insert(
        "unmatched".into(),
        (totals.submitted - totals.matched).into(),
    );
    totals_object.insert("by_status".into(), serde_json::json!(by_status));
    totals_object.insert("by_outcome_reason".into(), serde_json::json!(by_reason));
    totals_object.insert("unmatched_by_reason".into(), serde_json::json!(unmatched));

    let found = rows.iter().filter_map(|row| row.1.as_ref());
    let tables: Vec<serde_json::Value> = tables_of(found)
        .into_iter()
        .map(|table| serde_json::json!({"table_id": table.table_id, "version": table.version}))
        .collect();

    serde_json::json!({
        "enabled": true,
        "feed": FEED_COUNTER_PASS,
        "readable": true,
        "updated_at": pass.submitted_updated_at(),
        "weeks": query.weeks,
        "window_start": window.map(|(_, start)| start.to_rfc3339()),
        "window_end": query.now.to_rfc3339(),
        "pricing": {
            "basis": crate::insights::session_pricing::PRICING_BASIS_API_LIST,
            "tables": tables,
        },
        "totals": serde_json::Value::Object(totals_object),
        "by_model": models,
        "by_model_left_out": model_left_out,
        "by_task": tasks,
        "by_task_unknown": task_unknown.value_with(serde_json::json!({})),
        "code": serde_json::Value::Object(code),
        "by_week": weeks_out,
        "by_project": projects,
        "left_out": {"never_folder": never_folder},
        "traces_total": rows.len(),
        "traces": traces,
    })
}

fn trace_value(
    record: &HistoryRecord,
    session: Option<&StoredSession>,
    reason: Option<&'static str>,
) -> serde_json::Value {
    serde_json::json!({
        "submission_id": record.submission_id.to_string(),
        "submitted_at": record.submitted_at.to_rfc3339(),
        "project_label": record.project_label,
        "source": record.source,
        "status": status_key(record),
        "credit_points_pending": points(f64::from(record.credit_points_pending)),
        "credit_points_final": record.credit_points_final.map(|p| points(f64::from(p))),
        "credit_outcome_reason": record.credit_outcome_reason,
        "matched": session.is_some(),
        "unmatched_reason": if session.is_some() { None } else { reason },
        "tool": session.map(|s| s.tool),
        "model": session.and_then(|s| s.model.clone()),
        "mixed_models": session.is_some_and(|s| s.mixed_models),
        "task": session.and_then(|s| s.task).map(task_key),
        "files": session.and_then(|s| s.files.as_ref()).map(files_value),
        "edits": session.and_then(|s| s.edits),
        "edits_kept": session.and_then(edits_kept),
        "edits_undone": session.and_then(|s| s.undone),
        "edits_denied": session.and_then(|s| s.denied),
        "usd_micros": session.and_then(|s| s.usd_micros),
        "near_ai": session.is_some_and(|s| s.near_ai),
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use chrono::{Duration, TimeZone, Utc};
    use trace_commons_protocol::insights_usage_series::InMemoryDigestKeyStore;

    use super::super::insights_week::{COUNTER_ROWS_FILE, CounterCandidate, CounterPass};
    use super::*;

    const QUIET: u64 = 1_800;
    const PROJECT: &str = "/Users/PRIVATE_USER/code/PRIVATE_REPO";
    const OTHER_PROJECT: &str = "/Users/PRIVATE_USER/code/PRIVATE_OTHER";
    const OPUS: &str = "claude-opus-5-5";
    const SONNET: &str = "claude-sonnet-5-5";

    fn written() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 20, 10, 0, 0).unwrap()
    }

    fn now() -> DateTime<Utc> {
        written() + Duration::hours(2)
    }

    fn day(month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, month, day, 15, 0, 0).unwrap()
    }

    /// A digest-shaped session hash, so the privacy test sees one if it
    /// crossed.
    fn hash(n: u8) -> String {
        format!("sha256:{}", hex::encode([n; 32]))
    }

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(0x7e57_0000_0000_4000_8000_0000_0000_0000 | n)
    }

    /// A Claude Code session whose only edit is to `file`, with one turn on
    /// `model`.
    fn session(id: &str, model: &str, file: &str, output_tokens: u64) -> Vec<u8> {
        let line = |kind: &str, at: &str, uuid: &str, message: serde_json::Value| {
            serde_json::json!({
                "sessionId": id,
                "cwd": PROJECT,
                "version": "2.0.0",
                "uuid": uuid,
                "timestamp": at,
                "type": kind,
                "message": message,
            })
            .to_string()
        };
        let usage = serde_json::json!({
            "input_tokens": 50,
            "output_tokens": output_tokens,
            "cache_read_input_tokens": 1000,
            "cache_creation_input_tokens": 200,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 200,
                "ephemeral_1h_input_tokens": 0
            },
            "service_tier": "standard"
        });
        [
            line(
                "user",
                "2026-09-14T09:00:00.000Z",
                "u1",
                serde_json::json!({"role": "user", "content": "PRIVATE_PROMPT change the file"}),
            ),
            line(
                "assistant",
                "2026-09-14T09:00:05.000Z",
                "u2",
                serde_json::json!({
                    "id": format!("msg_PRIVATE_{id}"),
                    "model": model,
                    "role": "assistant",
                    "content": [{"type": "tool_use", "id": "toolu_PRIVATE_A", "name": "Edit",
                        "input": {"file_path": format!("{PROJECT}/{file}"),
                            "old_string": "PRIVATE_OLD", "new_string": "PRIVATE_NEW"}}],
                    "usage": usage,
                }),
            ),
            line(
                "user",
                "2026-09-14T09:00:06.000Z",
                "u3",
                serde_json::json!({"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_PRIVATE_A", "content": "ok"}]}),
            ),
        ]
        .join("\n")
        .into_bytes()
    }

    fn record(
        n: u128,
        at: DateTime<Utc>,
        status: &str,
        credit: (f32, Option<f32>),
        reason: Option<&str>,
    ) -> HistoryRecord {
        HistoryRecord {
            submission_id: id(n),
            submitted_at: at,
            project_id: String::new(),
            project_label: "my-app".to_string(),
            source: SOURCE_CLAUDE_CODE.to_string(),
            session_hash: hash(n as u8),
            status: status.to_string(),
            consent_scopes: vec!["research".to_string()],
            credit_points_pending: credit.0,
            credit_points_final: credit.1,
            credit_outcome_reason: reason.map(str::to_string),
            explanations: vec!["PRIVATE_EXPLANATION".to_string()],
            last_refreshed_at: Some(at),
            withdrawn_at: None,
            revoked_at: None,
            approved_unattended: Some(false),
            approved_verdict: None,
            uploaded_bytes: Some(1_000),
        }
    }

    fn queued(
        n: u128,
        with_id: bool,
        source: &str,
        path: &Path,
        project: &str,
    ) -> QueuedSubmission {
        QueuedSubmission {
            submission_id: with_id.then(|| id(n)),
            uploaded: true,
            session_hash: hash(n as u8),
            source: source.to_string(),
            path: path.to_path_buf(),
            project_key: project.to_string(),
        }
    }

    struct Fixture {
        dir: tempfile::TempDir,
        pass: CounterPass,
        history: Vec<HistoryRecord>,
        queue: Vec<QueuedSubmission>,
    }

    impl Fixture {
        fn empty() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let pass = CounterPass::with_keys(
                dir.path().join(COUNTER_ROWS_FILE),
                Box::new(InMemoryDigestKeyStore::with_seed([3; 32])),
            );
            Self {
                dir,
                pass,
                history: Vec::new(),
                queue: Vec::new(),
            }
        }

        fn write(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
            let path = self.dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }

        fn run(&self, paths: &[&Path]) {
            let candidates: Vec<CounterCandidate> = paths
                .iter()
                .map(|path| CounterCandidate {
                    origin: 0,
                    source: SOURCE_CLAUDE_CODE,
                    path: path.to_path_buf(),
                    size_bytes: std::fs::metadata(path).unwrap().len(),
                    modified_at: written(),
                    started_at: None,
                    group_member_count: 0,
                    role: crate::daemon::insights_week::CandidateRole::Watched,
                })
                .collect();
            self.pass
                .run_pass(&candidates, &NeverFolders::NONE, now(), QUIET, &mut |_| {
                    (PROJECT.to_string(), false)
                })
                .unwrap();
        }

        /// Three stored sessions on two models, and eight submissions:
        ///
        /// 1. Rust source on Opus, accepted with 5 points, joined by id.
        /// 2. Markdown on Sonnet, accepted at 0 (`policy_mismatch`), joined
        ///    by session hash (its queue entry carries no id).
        /// 3. A test file on Sonnet, quarantined, 2.5 pending.
        /// 4. Still processing, with no queue entry.
        /// 5. An opencode session, withdrawn: not read by the pass.
        /// 6. A file the pass never read, rejected (`novelty_failed`).
        /// 7. August 1st, accepted with 1 point: in 13 weeks, not in 4.
        /// 8. June 1st, accepted with 3 points: in all time only.
        fn standard() -> Self {
            let mut f = Self::empty();
            let rust = f.write(
                "PRIVATE-a.jsonl",
                &session("PRIVATE-A", OPUS, "src/lib.rs", 400),
            );
            let docs = f.write(
                "PRIVATE-b.jsonl",
                &session("PRIVATE-B", SONNET, "README.md", 200),
            );
            let tests = f.write(
                "PRIVATE-c.jsonl",
                &session("PRIVATE-C", SONNET, "tests/lib_test.rs", 300),
            );
            f.run(&[&rust, &docs, &tests]);
            let gone = f.dir.path().join("PRIVATE-gone.jsonl");
            f.history = vec![
                record(1, day(9, 14), STATUS_ACCEPTED, (0.0, Some(5.0)), None),
                record(
                    2,
                    day(9, 15),
                    STATUS_ACCEPTED,
                    (0.0, Some(0.0)),
                    Some("policy_mismatch"),
                ),
                record(3, day(9, 16), STATUS_QUARANTINED, (2.5, None), None),
                record(4, day(9, 17), "processing", (1.5, None), None),
                {
                    let mut withdrawn = record(5, day(9, 18), STATUS_ACCEPTED, (0.0, None), None);
                    withdrawn.source = "opencode".to_string();
                    withdrawn.withdrawn_at = Some(day(9, 19));
                    withdrawn.status = "withdrawn".to_string();
                    withdrawn
                },
                {
                    let mut rejected = record(
                        6,
                        day(9, 19),
                        STATUS_REJECTED,
                        (0.0, Some(0.0)),
                        Some("novelty_failed"),
                    );
                    rejected.project_label = "other-app".to_string();
                    rejected
                },
                record(7, day(8, 1), STATUS_ACCEPTED, (0.0, Some(1.0)), None),
                record(8, day(6, 1), STATUS_ACCEPTED, (0.0, Some(3.0)), None),
            ];
            f.queue = vec![
                queued(1, true, SOURCE_CLAUDE_CODE, &rust, PROJECT),
                queued(2, false, SOURCE_CLAUDE_CODE, &docs, PROJECT),
                queued(3, true, SOURCE_CLAUDE_CODE, &tests, PROJECT),
                queued(5, true, "opencode", &gone, OTHER_PROJECT),
                queued(6, true, SOURCE_CLAUDE_CODE, &gone, OTHER_PROJECT),
            ];
            f
        }

        fn answer(&self, never: &NeverFolders, weeks: Option<u32>) -> serde_json::Value {
            submitted_value(
                &self.pass,
                true,
                never,
                Some(&self.history),
                &self.queue,
                &SubmittedQuery {
                    weeks,
                    tz: FixedOffset::east_opt(0).unwrap(),
                    now: now(),
                    routing_available: false,
                },
            )
        }
    }

    fn shared() -> (tempfile::TempDir, DaemonShared) {
        let (dir, store) = crate::config::tests_support::temp_store();
        (dir, DaemonShared::load(store).unwrap())
    }

    fn call(params: serde_json::Value) -> Request {
        Request {
            id: 1,
            method: "insights_submitted".to_string(),
            params,
        }
    }

    fn trace(value: &serde_json::Value, n: u128) -> &serde_json::Value {
        value["traces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["submission_id"] == id(n).to_string())
            .unwrap_or_else(|| panic!("no trace {n}: {value}"))
    }

    #[test]
    fn params_take_four_thirteen_or_null_and_refuse_anything_else() {
        let ok = |params| submitted_weeks(&params);
        assert_eq!(ok(serde_json::json!({})), Ok(None));
        assert_eq!(ok(serde_json::Value::Null), Ok(None));
        assert_eq!(ok(serde_json::json!({"weeks": null})), Ok(None));
        assert_eq!(ok(serde_json::json!({"weeks": 4})), Ok(Some(4)));
        assert_eq!(ok(serde_json::json!({"weeks": 13})), Ok(Some(13)));
        for bad in [
            serde_json::json!({"weeks": 5}),
            serde_json::json!({"weeks": "4"}),
            serde_json::json!({"weeks": -4}),
            serde_json::json!({"weeks": 4.5}),
        ] {
            assert_eq!(ok(bad.clone()), Err(ERR_SUBMITTED_WEEKS_INVALID), "{bad}");
        }
        let (_dir, s) = shared();
        let r = handle_submitted(&s, &call(serde_json::json!({"weeks": 2})));
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
        let r = handle_submitted(&s, &call(serde_json::json!({"tz": 999_999})));
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[test]
    fn switched_off_without_opening_the_store_or_making_a_key() {
        let f = Fixture::standard();
        let off = submitted_value(
            &f.pass,
            false,
            &NeverFolders::NONE,
            Some(&f.history),
            &f.queue,
            &SubmittedQuery {
                weeks: None,
                tz: FixedOffset::east_opt(0).unwrap(),
                now: now(),
                routing_available: false,
            },
        );
        assert_eq!(
            off,
            serde_json::json!({"enabled": false, "feed": "counter_pass"})
        );
        let fresh = Fixture::empty();
        let off = submitted_value(
            &fresh.pass,
            false,
            &NeverFolders::NONE,
            None,
            &[],
            &SubmittedQuery {
                weeks: None,
                tz: FixedOffset::east_opt(0).unwrap(),
                now: now(),
                routing_available: false,
            },
        );
        assert_eq!(off["enabled"], false);
        assert!(fresh.pass.key_is_absent_for_test(), "no key made");
        assert!(!fresh.dir.path().join(COUNTER_ROWS_FILE).exists());
        let (_dir, s) = shared();
        s.settings.lock().unwrap().insights_counter_pass = false;
        let r = handle_submitted(&s, &call(serde_json::json!({})));
        assert_eq!(
            r.result.unwrap(),
            serde_json::json!({"enabled": false, "feed": "counter_pass"})
        );
    }

    #[test]
    fn an_unreadable_store_or_history_answers_readable_false() {
        let mut f = Fixture::empty();
        f.history = vec![record(
            1,
            day(9, 14),
            STATUS_ACCEPTED,
            (0.0, Some(5.0)),
            None,
        )];
        f.queue = vec![queued(
            1,
            true,
            SOURCE_CLAUDE_CODE,
            Path::new("/x"),
            PROJECT,
        )];
        std::fs::write(f.dir.path().join(COUNTER_ROWS_FILE), b"{not json").unwrap();
        f.pass.keys_load_or_create_for_test();
        let value = f.answer(&NeverFolders::NONE, None);
        assert_eq!(value["enabled"], true);
        assert_eq!(value["readable"], false);
        assert!(value["reason"].is_string());
        assert!(value.get("totals").is_none(), "never an empty answer");
        let none = submitted_value(
            &f.pass,
            true,
            &NeverFolders::NONE,
            None,
            &[],
            &SubmittedQuery {
                weeks: None,
                tz: FixedOffset::east_opt(0).unwrap(),
                now: now(),
                routing_available: false,
            },
        );
        assert_eq!(none["readable"], false);
        assert_eq!(none["reason"], UNREADABLE_HISTORY);
    }

    #[test]
    fn the_method_is_advertised_dispatched_and_local() {
        let method = "insights_submitted";
        assert!(super::super::ipc::METHODS.contains(&method));
        assert!(super::super::ipc::DEV_DRY_RUN_LOCAL_METHODS.contains(&method));
        let (_dir, mut s) = shared();
        s.dev_dry_run = true;
        s.settings.lock().unwrap().insights_counter_pass = false;
        let r = super::super::ipc::handle_request(&s, &call(serde_json::json!({"weeks": 13})));
        assert_eq!(r.result.unwrap()["enabled"], false);
    }

    #[test]
    fn the_handler_reads_the_history_cache_and_the_queue() {
        let (_dir, s) = shared();
        HistoryCache::save(
            &s.store,
            &[record(
                1,
                Utc::now(),
                STATUS_ACCEPTED,
                (0.0, Some(2.0)),
                None,
            )],
        )
        .unwrap();
        let r = handle_submitted(&s, &call(serde_json::json!({"weeks": 4, "tz": 0})));
        let v = r.result.unwrap();
        assert_eq!(v["readable"], true, "{v}");
        assert_eq!(v["totals"]["submitted"], 1);
        assert_eq!(v["totals"]["unmatched_by_reason"][NO_QUEUE_ENTRY], 1);
        assert_eq!(v["totals"]["credit_final"], 2.0);
    }

    #[test]
    fn one_submitted_trace_already_answers_every_part() {
        let mut f = Fixture::empty();
        let path = f.write(
            "PRIVATE-one.jsonl",
            &session("PRIVATE-ONE", OPUS, "src/lib.rs", 100),
        );
        f.run(&[&path]);
        f.history = vec![record(
            1,
            day(9, 14),
            STATUS_ACCEPTED,
            (0.0, Some(5.0)),
            None,
        )];
        f.queue = vec![queued(1, true, SOURCE_CLAUDE_CODE, &path, PROJECT)];
        let v = f.answer(&NeverFolders::NONE, Some(4));
        assert_eq!(v["totals"]["submitted"], 1);
        assert_eq!(v["totals"]["matched"], 1);
        let models = v["by_model"].as_array().unwrap();
        assert_eq!(models.len(), 1, "no floor: {v}");
        assert_eq!(models[0]["label"], OPUS);
        assert_eq!(models[0]["submitted"], 1);
        assert_eq!(models[0]["credit_final"], 5.0);
        assert!(models[0]["usd_micros_known"].as_u64().unwrap() > 0);
        assert_eq!(models[0]["priced_sessions"], 1);
        assert_eq!(v["code"]["files"]["source"], 1);
        assert_eq!(v["traces"].as_array().unwrap().len(), 1);
        assert_eq!(v["traces"][0]["model"], OPUS);
        assert!(v["traces"][0]["task"].is_string());
    }

    #[test]
    fn joins_by_submission_id_then_session_hash_and_counts_the_rest_unmatched() {
        let f = Fixture::standard();
        let v = f.answer(&NeverFolders::NONE, None);
        assert_eq!(v["readable"], true, "{v}");
        let totals = &v["totals"];
        assert_eq!(totals["submitted"], 8);
        assert_eq!(totals["matched"], 3);
        assert_eq!(totals["unmatched"], 5);
        assert_eq!(
            totals["unmatched_by_reason"],
            serde_json::json!({"no_queue_entry": 3, "no_session_row": 1, "source_not_read": 1})
        );
        assert_eq!(trace(&v, 1)["matched"], true);
        assert_eq!(trace(&v, 1)["model"], OPUS);
        assert_eq!(trace(&v, 1)["files"]["source"], 1);
        assert_eq!(trace(&v, 2)["matched"], true, "joined by hash");
        assert_eq!(trace(&v, 2)["model"], SONNET);
        assert_eq!(trace(&v, 2)["task"], "docs");
        assert_eq!(trace(&v, 2)["files"]["doc"], 1);
        assert_eq!(trace(&v, 3)["files"]["test"], 1);
        assert_eq!(trace(&v, 4)["unmatched_reason"], NO_QUEUE_ENTRY);
        assert_eq!(trace(&v, 4)["model"], serde_json::Value::Null);
        assert_eq!(trace(&v, 5)["unmatched_reason"], NOT_READ);
        assert_eq!(trace(&v, 6)["unmatched_reason"], NO_SESSION_ROW);
        // Newest first.
        let order: Vec<&str> = v["traces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["submitted_at"].as_str().unwrap())
            .collect();
        let mut sorted = order.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(order, sorted);
        assert_eq!(v["traces_total"], 8);
        // By model: Sonnet carries two, Opus one; the rest are unmatched.
        let models = v["by_model"].as_array().unwrap();
        assert_eq!(models[0]["label"], SONNET);
        assert_eq!(models[0]["submitted"], 2);
        assert_eq!(models[0]["accepted"], 1);
        assert_eq!(models[0]["credit_pending"], 2.5);
        assert_eq!(models[0]["tasks"]["docs"], 1);
        assert_eq!(models[1]["label"], OPUS);
        assert_eq!(models[1]["credit_final"], 5.0);
        assert_eq!(v["by_model_left_out"]["unmatched"], 5);
        // Code: one file of each kind, three edits all kept.
        let code = &v["code"];
        assert_eq!(code["sessions"], 3);
        assert_eq!(
            code["files"],
            serde_json::json!({"source": 1, "test": 1, "doc": 1, "config": 0, "total": 3})
        );
        assert_eq!(code["edits_known"], 3);
        assert_eq!(code["edits_kept_known"], 3);
        assert_eq!(code["edits_partial"], false);
        assert_eq!(code["edits_kept_partial"], false);
        assert_eq!(code["edits_undone_partial"], false);
        // Never asked, so declined is unknown: kept reads it as none.
        assert_eq!(code["edits_denied_partial"], true);
        assert_eq!(code["priced_sessions"], 3);
        assert!(code["usd_micros_known"].as_u64().unwrap() > 0);
        // Projects by History's label.
        let projects = v["by_project"].as_array().unwrap();
        assert_eq!(projects[0]["label"], "my-app");
        assert_eq!(projects[0]["submitted"], 7);
        assert_eq!(projects[1]["label"], "other-app");
        assert_eq!(projects[1]["matched"], 0);
    }

    #[test]
    fn statuses_credit_and_outcome_reasons_follow_history() {
        let f = Fixture::standard();
        let v = f.answer(&NeverFolders::NONE, None);
        let totals = &v["totals"];
        assert_eq!(
            totals["by_status"],
            serde_json::json!({
                "accepted": 4, "pending": 1, "quarantined": 1,
                "rejected": 1, "withdrawn": 1, "other": 0,
            })
        );
        assert_eq!(totals["accepted"], 4);
        // Final: 5 + 0 + 0 + 1 + 3; pending only where nothing is final.
        assert_eq!(totals["credit_final"], 9.0);
        assert_eq!(totals["credit_pending"], 4.0);
        assert_eq!(
            totals["by_outcome_reason"],
            serde_json::json!({"novelty_failed": 1, "policy_mismatch": 1})
        );
        assert_eq!(
            trace(&v, 4)["status"],
            "pending",
            "processing reads as pending"
        );
        assert_eq!(trace(&v, 5)["status"], "withdrawn");
        assert_eq!(trace(&v, 2)["credit_outcome_reason"], "policy_mismatch");
        let task_total: u64 = v["by_task"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["submitted"].as_u64().unwrap())
            .sum();
        assert_eq!(
            task_total + v["by_task_unknown"]["submitted"].as_u64().unwrap(),
            8
        );
        let keys: Vec<&str> = v["by_task"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["key"].as_str().unwrap())
            .collect();
        assert_eq!(
            keys,
            [
                "bug_fix",
                "feature",
                "refactor",
                "tests",
                "docs",
                "understand",
                "review",
                "setup"
            ]
        );
    }

    #[test]
    fn the_weeks_filter_counts_by_local_week() {
        let f = Fixture::standard();
        let four = f.answer(&NeverFolders::NONE, Some(4));
        assert_eq!(four["weeks"], 4);
        assert_eq!(four["window_start"], "2026-08-24T00:00:00+00:00");
        assert_eq!(four["totals"]["submitted"], 6);
        let weeks: Vec<&str> = four["by_week"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["week_start"].as_str().unwrap())
            .collect();
        assert_eq!(
            weeks,
            ["2026-08-24", "2026-08-31", "2026-09-07", "2026-09-14"]
        );
        assert_eq!(four["by_week"][3]["submitted"], 6);
        assert_eq!(four["by_week"][0]["submitted"], 0, "empty weeks are drawn");
        let thirteen = f.answer(&NeverFolders::NONE, Some(13));
        assert_eq!(thirteen["totals"]["submitted"], 7);
        assert_eq!(thirteen["by_week"].as_array().unwrap().len(), 13);
        let all = f.answer(&NeverFolders::NONE, None);
        assert_eq!(all["weeks"], serde_json::Value::Null);
        assert_eq!(all["window_start"], serde_json::Value::Null);
        assert_eq!(all["totals"]["submitted"], 8);
        assert_eq!(all["by_week"][0]["week_start"], "2026-06-01");
        assert_eq!(all["by_week"][0]["credit_final"], 3.0);
        // A local week: 23:30 UTC on Sunday the 13th is Monday the 14th
        // an hour east.
        let mut late = Fixture::empty();
        late.history = vec![record(
            1,
            Utc.with_ymd_and_hms(2026, 9, 13, 23, 30, 0).unwrap(),
            STATUS_ACCEPTED,
            (0.0, Some(1.0)),
            None,
        )];
        let east = submitted_value(
            &late.pass,
            true,
            &NeverFolders::NONE,
            Some(&late.history),
            &[],
            &SubmittedQuery {
                weeks: Some(4),
                tz: FixedOffset::east_opt(3_600).unwrap(),
                now: now(),
                routing_available: false,
            },
        );
        assert_eq!(east["by_week"][3]["week_start"], "2026-09-14");
        assert_eq!(east["by_week"][3]["submitted"], 1);
        late.history[0].submitted_at = Utc.with_ymd_and_hms(2026, 8, 23, 22, 30, 0).unwrap();
        let utc = late.answer(&NeverFolders::NONE, Some(4));
        assert_eq!(utc["totals"]["submitted"], 0, "the week before the window");
    }

    #[test]
    fn never_folders_enter_no_answer() {
        let f = Fixture::standard();
        let never = NeverFolders::These(vec![PROJECT.to_string()]);
        let v = f.answer(&never, None);
        assert_eq!(v["readable"], true);
        assert_eq!(v["left_out"]["never_folder"], 3);
        assert_eq!(v["totals"]["submitted"], 5);
        assert_eq!(v["totals"]["matched"], 0);
        assert!(v["by_model"].as_array().unwrap().is_empty());
        for n in [1, 2, 3] {
            assert!(
                !v["traces"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["submission_id"] == id(n).to_string()),
                "{n} crossed"
            );
        }
        // By the history row's own project id too, with no queue entry.
        let mut by_id = Fixture::standard();
        by_id.history[3].project_id = super::super::policy::project_id_for(OTHER_PROJECT);
        let v = by_id.answer(&NeverFolders::These(vec![OTHER_PROJECT.to_string()]), None);
        assert_eq!(v["left_out"]["never_folder"], 3, "4, 5 and 6: {v}");
        let every = f.answer(&NeverFolders::Every, None);
        assert_eq!(every["totals"]["submitted"], 0);
        assert_eq!(every["left_out"]["never_folder"], 8);
    }

    #[test]
    fn a_store_with_no_rows_yet_counts_every_submission_unmatched() {
        let mut f = Fixture::standard();
        f.pass = CounterPass::with_keys(
            f.dir.path().join("other-rows.json"),
            Box::new(InMemoryDigestKeyStore::with_seed([4; 32])),
        );
        let v = f.answer(&NeverFolders::NONE, None);
        assert_eq!(v["readable"], true);
        assert_eq!(v["totals"]["matched"], 0);
        assert_eq!(v["totals"]["unmatched_by_reason"][NO_SESSION_ROW], 4);
        assert!(f.pass.key_is_absent_for_test(), "the answer makes no key");
    }

    #[test]
    fn traces_are_capped_with_a_total() {
        let mut f = Fixture::empty();
        f.history = (0..(SUBMITTED_MAX_TRACES as u128 + 7))
            .map(|n| {
                record(
                    n,
                    day(9, 14) - Duration::minutes(n as i64),
                    STATUS_ACCEPTED,
                    (0.0, Some(1.0)),
                    None,
                )
            })
            .collect();
        let v = f.answer(&NeverFolders::NONE, None);
        assert_eq!(v["traces"].as_array().unwrap().len(), SUBMITTED_MAX_TRACES);
        assert_eq!(v["traces_total"], SUBMITTED_MAX_TRACES + 7);
        assert_eq!(v["totals"]["submitted"], SUBMITTED_MAX_TRACES + 7);
        assert_eq!(
            v["totals"]["credit_final"],
            (SUBMITTED_MAX_TRACES + 7) as f64
        );
    }

    #[test]
    fn nothing_identifying_crosses() {
        let f = Fixture::standard();
        let value = f.answer(&NeverFolders::NONE, None);
        let mut text = serde_json::to_string(&value).unwrap();
        for table in value["pricing"]["tables"].as_array().unwrap() {
            text = text.replace(table["table_id"].as_str().unwrap(), "TABLE");
        }
        for forbidden in [
            "PRIVATE",
            "/Users",
            "msg_",
            "toolu_",
            "sha256:",
            "session_hash",
            "session_ref",
            "project_id",
            "proj_",
            ".jsonl",
            "README",
            "lib.rs",
            "explanations",
            f.dir.path().to_str().unwrap(),
        ] {
            assert!(!text.contains(forbidden), "{forbidden} crossed: {text}");
        }
        let hex_run = text
            .split(|c: char| !c.is_ascii_hexdigit())
            .map(str::len)
            .max()
            .unwrap_or(0);
        assert!(hex_run < 16, "a digest-like run crossed: {text}");
    }

    /// The whole answer, printed between markers for the shells' decode
    /// fixture (`cargo test ... -- --nocapture`), and compared to it.
    #[test]
    fn a_submitted_answer_for_shell_fixtures() {
        let f = Fixture::standard();
        let value = f.answer(&NeverFolders::NONE, Some(13));
        println!("BEGIN insights_submitted");
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
        println!("END insights_submitted");
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/insights-analytics/insights_submitted.json"
        ))
        .unwrap();
        assert_eq!(
            value, fixture,
            "re-record tests/fixtures/insights-analytics/insights_submitted.json \
             from this test's --nocapture output"
        );
    }
}
