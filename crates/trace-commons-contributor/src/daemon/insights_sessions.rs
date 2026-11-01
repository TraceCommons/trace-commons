//! `insights_sessions` and `insights_session_drill`: the Sessions tab over
//! the counter pass (feed T).
//!
//! Gated exactly as `insights_week` is (on by default; product owner,
//! 2026-10-29): off, both
//! answer `enabled: false` without opening the store or the key; on, a store,
//! key or folder-rule read that fails answers `readable: false` with its
//! fixed reason, never an empty list. Sessions in a folder that is Never are
//! neither listed nor drilled.
//!
//! # What crosses the socket
//!
//! `insights_sessions` lists the stored sessions newest first, a page at a
//! time (owner request 2026-10-20: every session in a lazily loaded
//! scroll): at most [`WATCHED_SESSIONS_MAX`] a page, with an opaque cursor
//! for the next ([`select_page`] has the order). No params is the first
//! page, as the list answered before it was paged. Each row carries an
//! opaque reference (`t` and the order the pass made the row in, `.` and
//! the store's tag, as the cursor names the store: never a digest or a
//! path), its harness, its first recorded event's time, its
//! turn count (Claude Code only), its tokens and an integer sparkline of
//! tokens per turn (`insights::session_sparkline`), and its folder's display
//! label (owner decision 2026-10-20).
//!
//! The folder label is resolved at answer time: the row stores only a keyed
//! digest of its folder's project key, and the daemon matches it against the
//! digests of the project keys it already knows (folder rules, the queue,
//! uploads and the folders its watcher has seen), giving the same label
//! `list_projects` shows for that folder. Nothing new is stored, no path or
//! key crosses, and a digest with no current match crosses as `null`, which
//! a shell draws with the core's `analytics_folder_unknown`.
//!
//! `insights_session_drill` answers one session by that reference with the
//! same drill shape a saved snapshot's `session_drill` has, computed by the
//! same engine (`insights::session_drill`) from the stored counters, with
//! `feed: "counter_pass"`.
//!
//! # The person's own labels
//!
//! Each row also carries the task the rules infer from its stored facts
//! (`inferred_task`) and the person's own labels (`user_task`,
//! `user_outcome`; Insights PM request, 2026-10-21), and
//! `insights_session_label` sets or clears them by the row's reference,
//! answering the row as listed. Two closed sets
//! (`insights::session_labels`), checked before the gate; gated as the list
//! is, and never written while the pass is off. The daemon keeps a label in
//! its labels file beside the counter store, by the row's keyed session
//! digest, so it survives the row being made again under a new reference
//! and the store being begun again (`insights_week` has the rules), and it
//! is never sent anywhere.

use chrono::{DateTime, FixedOffset, Offset, Utc};

use super::insights_week::{
    FEED_COUNTER_PASS, NeverFolders, UNREADABLE_POLICY, never_project_keys,
};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use super::policy::{UNKNOWN_PROJECT_KEY, disambiguated_label, display_path_for_key, known_keys};
use crate::insights::session_labels::{LabelledTask, SessionOutcome};

/// The most sessions one `insights_sessions` page lists, newest first.
/// `truncated` and `next` say when more follow.
pub const WATCHED_SESSIONS_MAX: usize = 200;
/// `session_ref` missing or not a string.
pub const ERR_SESSION_REF_INVALID: &str = "insights-session-ref-invalid";
/// No listed session has that reference: never stored, dropped since, made
/// again under a new reference when its file grew, in a Never folder, or
/// answered from a store since begun again.
pub const ERR_SESSION_UNKNOWN: &str = "insights-session-unknown";

/// `before` is not a cursor an earlier page gave.
pub const ERR_SESSIONS_CURSOR_INVALID: &str = "insights-sessions-cursor-invalid";
/// `limit` is not a whole number of at least 1.
pub const ERR_SESSIONS_LIMIT_INVALID: &str = "insights-sessions-limit-invalid";
/// `task` or `outcome` is not one of its closed words, or `null`; or
/// neither was given.
pub const ERR_SESSION_LABEL_INVALID: &str = "insights-session-label-invalid";
/// The labels file could not be written, or is not in the labels shape and
/// is never written over; the label is as it was.
pub const ERR_SESSION_LABEL_WRITE_FAILED: &str = "insights-session-label-write-failed";

/// What one `insights_session_label` call does to one of a row's labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LabelEdit<T> {
    /// The param was absent: the label stays as it is.
    Keep,
    /// `null`: the label is unset (the inferred task shows again).
    Clear,
    Set(T),
}

impl<T: Copy> LabelEdit<T> {
    /// Apply it to the field `name` of a label as the labels file keeps
    /// it, by `wire` word.
    fn apply_stored(
        self,
        stored: &mut serde_json::Map<String, serde_json::Value>,
        name: &str,
        wire: fn(T) -> &'static str,
    ) {
        match self {
            LabelEdit::Keep => {}
            LabelEdit::Clear => {
                stored.remove(name);
            }
            LabelEdit::Set(value) => {
                stored.insert(name.to_string(), wire(value).into());
            }
        }
    }
}

/// An `insights_session_label` call's change to a row's labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LabelChange {
    pub(crate) task: LabelEdit<LabelledTask>,
    pub(crate) outcome: LabelEdit<SessionOutcome>,
}

impl LabelChange {
    /// Apply it to a label as the labels file keeps it: only the fields it
    /// names change, so a field or word another build wrote is kept.
    pub(crate) fn apply_stored(self, stored: &mut serde_json::Map<String, serde_json::Value>) {
        self.task.apply_stored(stored, "task", LabelledTask::wire);
        self.outcome
            .apply_stored(stored, "outcome", SessionOutcome::wire);
    }
}

/// `session_ref` (required), and `task` and `outcome`, each absent (kept),
/// `null` (cleared) or one of its closed words; at least one of the two.
pub(crate) fn label_params(
    params: &serde_json::Value,
) -> Result<(String, LabelChange), &'static str> {
    let reference = params
        .get("session_ref")
        .and_then(|value| value.as_str())
        .ok_or(ERR_SESSION_REF_INVALID)?;
    fn edit<T>(
        value: Option<&serde_json::Value>,
        parse: fn(&str) -> Option<T>,
    ) -> Result<LabelEdit<T>, &'static str> {
        match value {
            None => Ok(LabelEdit::Keep),
            Some(serde_json::Value::Null) => Ok(LabelEdit::Clear),
            Some(serde_json::Value::String(wire)) => parse(wire)
                .map(LabelEdit::Set)
                .ok_or(ERR_SESSION_LABEL_INVALID),
            Some(_) => Err(ERR_SESSION_LABEL_INVALID),
        }
    }
    let change = LabelChange {
        task: edit(params.get("task"), LabelledTask::from_wire)?,
        outcome: edit(params.get("outcome"), SessionOutcome::from_wire)?,
    };
    if change.task == LabelEdit::Keep && change.outcome == LabelEdit::Keep {
        return Err(ERR_SESSION_LABEL_INVALID);
    }
    Ok((reference.to_string(), change))
}

/// Where one page of the list starts: the recency key of the last row the
/// previous page listed by recency. Crosses as an opaque string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SessionsCursor {
    pub(crate) at: DateTime<Utc>,
    pub(crate) seq: u64,
}

impl SessionsCursor {
    /// `p`, then seconds, nanoseconds, the row's order and the store tag,
    /// each separated by `.`: no digest, path or reference, and no long hex
    /// run. The tag names the store (its key) the cursor was answered from,
    /// so a cursor from another store is refused rather than followed.
    pub(crate) fn encode(&self, store: u64) -> String {
        format!(
            "p{}.{}.{}.{}",
            self.at.timestamp(),
            self.at.timestamp_subsec_nanos(),
            self.seq,
            store
        )
    }

    /// The cursor and its store tag. Every part is plain ASCII digits: no
    /// sign, space or empty part, which integer parsing alone would accept.
    pub(crate) fn decode(text: &str) -> Option<(Self, u64)> {
        let mut parts = text.strip_prefix('p')?.split('.');
        let secs_text = parts.next()?;
        let nanos_text = parts.next()?;
        let seq_text = parts.next()?;
        let store_text = parts.next()?;
        if parts.next().is_some() {
            return None;
        }
        let digits = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
        if ![secs_text, nanos_text, seq_text, store_text]
            .iter()
            .all(|part| digits(part))
        {
            return None;
        }
        let secs: i64 = secs_text.parse().ok()?;
        let nanos: u32 = nanos_text.parse().ok()?;
        let seq: u64 = seq_text.parse().ok()?;
        let store: u64 = store_text.parse().ok()?;
        if nanos >= 1_000_000_000 {
            return None;
        }
        Some((
            Self {
                at: DateTime::from_timestamp(secs, nanos)?,
                seq,
            },
            store,
        ))
    }
}

/// One page of `insights_sessions`: rows older (by recency) than `before`,
/// at most `limit` of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SessionsPage {
    pub(crate) before: Option<SessionsCursor>,
    /// The store tag `before` carried; `0` when there is no cursor.
    pub(crate) store: u64,
    pub(crate) limit: usize,
}

impl SessionsPage {
    /// The first page, as `insights_sessions` with no params answers it.
    pub(crate) const FIRST: Self = Self {
        before: None,
        store: 0,
        limit: WATCHED_SESSIONS_MAX,
    };
}

/// `before` (optional, a cursor an earlier page gave) and `limit` (optional,
/// at least 1; above [`WATCHED_SESSIONS_MAX`] is held to it). No params is
/// [`SessionsPage::FIRST`].
pub(crate) fn page_params(params: &serde_json::Value) -> Result<SessionsPage, &'static str> {
    if params.get("before").is_none() && params.get("limit").is_none() {
        return Ok(SessionsPage::FIRST);
    }
    let (before, store) = match params.get("before") {
        None | Some(serde_json::Value::Null) => (None, 0),
        Some(serde_json::Value::String(text)) => {
            let (cursor, store) =
                SessionsCursor::decode(text).ok_or(ERR_SESSIONS_CURSOR_INVALID)?;
            (Some(cursor), store)
        }
        Some(_) => return Err(ERR_SESSIONS_CURSOR_INVALID),
    };
    let limit = match params.get("limit") {
        None | Some(serde_json::Value::Null) => WATCHED_SESSIONS_MAX,
        Some(value) => match value.as_u64() {
            Some(limit) if limit >= 1 => usize::try_from(limit)
                .map_or(WATCHED_SESSIONS_MAX, |limit| {
                    limit.min(WATCHED_SESSIONS_MAX)
                }),
            _ => return Err(ERR_SESSIONS_LIMIT_INVALID),
        },
    };
    Ok(SessionsPage {
        before,
        store,
        limit,
    })
}

/// The rows one page lists, from every visible row, and the cursor for the
/// next page (`None` at the end of the list).
///
/// The bound picks by recency alone, as #1498 settled: rows are taken newest
/// first by `key` (the first recorded event, else where discovery placed
/// the row, else its file's last write; then the order it was made), from
/// strictly older than `before`. Only then are the taken rows ordered dated
/// first, the undated ones after in their recency order, so within a page an
/// undated row never leads. The next cursor is the last row taken by
/// recency, never the last row as listed.
pub(crate) fn select_page<T>(
    rows: Vec<T>,
    key: impl Fn(&T) -> SessionsCursor,
    dated: impl Fn(&T) -> bool,
    page: SessionsPage,
) -> (Vec<T>, Option<SessionsCursor>) {
    let mut keyed: Vec<(SessionsCursor, T)> = rows
        .into_iter()
        .map(|row| (key(&row), row))
        .filter(|(at, _)| page.before.is_none_or(|before| *at < before))
        .collect();
    keyed.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    let more = keyed.len() > page.limit;
    keyed.truncate(page.limit);
    let next = if more {
        keyed.last().map(|(at, _)| *at)
    } else {
        None
    };
    let mut taken: Vec<T> = keyed.into_iter().map(|(_, row)| row).collect();
    // Stable: the undated rows keep their recency order.
    taken.sort_by_key(|row| !dated(row));
    (taken, next)
}

/// Whether the pass is on, and the Never folders while it is; `Err` is the
/// answer to send instead (the folder rules could not be read).
pub(crate) fn gate(shared: &DaemonShared) -> Result<(bool, NeverFolders), serde_json::Value> {
    // A poisoned lock reads as off: fail closed, never a guess.
    let enabled = shared
        .settings
        .lock()
        .map(|settings| settings.insights_counter_pass)
        .unwrap_or(false);
    if !enabled {
        return Ok((false, NeverFolders::NONE));
    }
    match never_project_keys(shared) {
        Some(never) => Ok((true, never)),
        None => Err(serde_json::json!({
            "enabled": true,
            "feed": FEED_COUNTER_PASS,
            "readable": false,
            "reason": UNREADABLE_POLICY,
        })),
    }
}

/// Every project key the daemon knows, each with the label `list_projects`
/// shows for it, the unknown bucket left out. Taken lock by lock, one at a
/// time: the watcher's folders first, then the folder rules with the queue,
/// as `list_projects` takes them.
pub(crate) fn folder_labels(shared: &DaemonShared) -> Vec<(String, String)> {
    let watched: Vec<String> = match shared.state.lock() {
        Ok(state) => state
            .cwd_cache
            .values()
            .filter_map(|cached| cached.project_key.clone())
            .collect(),
        Err(_) => Vec::new(),
    };
    let (known, shown) = {
        let (Ok(policy), Ok(queue)) = (shared.policy.lock(), shared.queue.lock()) else {
            return Vec::new();
        };
        let known = known_keys(&policy, queue.all().iter().map(|e| e.project_key.clone()));
        let mut shown: std::collections::BTreeMap<String, Option<String>> =
            std::collections::BTreeMap::new();
        for (key, entry) in &policy.projects {
            shown.insert(key.clone(), entry.display_path.clone());
        }
        for entry in queue.all() {
            shown
                .entry(entry.project_key.clone())
                .or_insert_with(|| entry.project_path.clone());
        }
        for key in known.iter().chain(watched.iter()) {
            shown.entry(key.clone()).or_insert(None);
        }
        (known, shown)
    };
    // Labelled with no lock held: a folder with no stored spelling is
    // looked up on disk.
    shown
        .into_iter()
        .filter(|(key, _)| key != UNKNOWN_PROJECT_KEY)
        .map(|(key, path)| {
            let label = folder_label(&key, path, &known);
            (key, label)
        })
        .collect()
}

/// A folder's label as `list_projects` renders it. A key with no stored
/// spelling (a folder only the watcher has seen) is spelled the way the
/// filesystem spells it, never in the folded key's case.
fn folder_label(key: &str, shown: Option<String>, known: &[String]) -> String {
    let shown = shown.or_else(|| display_path_for_key(key));
    disambiguated_label(key, shown.as_deref(), known)
}

/// `tz`, seconds east; the daemon's own offset when absent.
fn tz_param(req: &Request) -> Option<FixedOffset> {
    match req.params.get("tz") {
        None | Some(serde_json::Value::Null) => Some(chrono::Local::now().offset().fix()),
        Some(_) => super::insights_glance::parse_tz(&req.params),
    }
}

/// `insights_sessions`.
pub fn handle_sessions(shared: &DaemonShared, req: &Request) -> Response {
    let page = match page_params(&req.params) {
        Ok(page) => page,
        Err(label) => return Response::err(req.id, ERR_BAD_PARAMS, label),
    };
    let (enabled, never) = match gate(shared) {
        Ok(gate) => gate,
        Err(answer) => return Response::ok(req.id, answer),
    };
    // Resolved only while the pass is on: off, nothing is read.
    let folders = if enabled {
        folder_labels(shared)
    } else {
        Vec::new()
    };
    // A cursor from another store is refused as a bad cursor, so the shell
    // reads the first page.
    match shared
        .insights_counter
        .sessions_page_value(enabled, &never, &folders, page)
    {
        Ok(answer) => Response::ok(req.id, answer),
        Err(label) => Response::err(req.id, ERR_BAD_PARAMS, label),
    }
}

/// `insights_session_label`.
pub fn handle_session_label(shared: &DaemonShared, req: &Request) -> Response {
    let (reference, change) = match label_params(&req.params) {
        Ok(found) => found,
        Err(label) => return Response::err(req.id, ERR_BAD_PARAMS, label),
    };
    let (enabled, never) = match gate(shared) {
        Ok(gate) => gate,
        Err(answer) => return Response::ok(req.id, answer),
    };
    // Resolved only while the pass is on: off, nothing is read or written.
    let folders = if enabled {
        folder_labels(shared)
    } else {
        Vec::new()
    };
    match shared
        .insights_counter
        .label_value(enabled, &reference, change, &never, &folders)
    {
        Ok(answer) => Response::ok(req.id, answer),
        Err(ERR_SESSION_LABEL_WRITE_FAILED) => {
            Response::err(req.id, ERR_UNAVAILABLE, ERR_SESSION_LABEL_WRITE_FAILED)
        }
        Err(label) => Response::err(req.id, ERR_BAD_PARAMS, label),
    }
}

/// `insights_session_drill`.
pub fn handle_session_drill(shared: &DaemonShared, req: &Request) -> Response {
    let Some(reference) = req.params.get("session_ref").and_then(|v| v.as_str()) else {
        return Response::err(req.id, ERR_BAD_PARAMS, ERR_SESSION_REF_INVALID);
    };
    let Some(tz) = tz_param(req) else {
        return Response::err(
            req.id,
            ERR_BAD_PARAMS,
            super::insights_glance::ERR_TZ_INVALID,
        );
    };
    let (enabled, never) = match gate(shared) {
        Ok(gate) => gate,
        Err(answer) => return Response::ok(req.id, answer),
    };
    match shared
        .insights_counter
        .drill_value(enabled, reference, &never, tz)
    {
        Ok(value) => Response::ok(req.id, value),
        Err(label) => Response::err(req.id, ERR_BAD_PARAMS, label),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use chrono::{DateTime, Duration, TimeZone, Utc};
    use trace_commons_protocol::insights_usage_series::InMemoryDigestKeyStore;

    use super::super::insights_week::{
        COUNTER_ROWS_FILE, CounterCandidate, CounterPass, SESSION_LABELS_FILE, UNREADABLE_STORE,
    };
    use super::*;
    use crate::source::{SOURCE_CLAUDE_CODE, SOURCE_CODEX};

    const QUIET: u64 = 1_800;
    const PROJECT: &str = "/Users/PRIVATE_USER/code/PRIVATE_REPO";
    const OTHER: &str = "/Users/PRIVATE_USER/code/OTHER_REPO";

    fn claude_bytes() -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/insights/claude-turn-series/session.jsonl"),
        )
        .unwrap()
    }

    /// One Codex session: two token counts a minute apart, after a nonzero
    /// baseline, with earlier records that are not counters.
    /// Two turns: 100 input (60 cached) and 20 output, then 80 input (40
    /// cached) and 30 output. The observed change between the first and
    /// last counter is the second turn's 110; the per-turn series holds
    /// both, 230.
    fn codex_bytes() -> Vec<u8> {
        let usage = |at: &str, total: [u64; 3], last: [u64; 3]| {
            serde_json::json!({
                "type": "event_msg",
                "timestamp": at,
                "payload": {"type": "token_count", "info": {"total_token_usage": {
                    "input_tokens": total[0], "cached_input_tokens": total[1],
                    "output_tokens": total[2], "reasoning_output_tokens": 0,
                    "total_tokens": total[0] + total[2]
                }, "last_token_usage": {
                    "input_tokens": last[0], "cached_input_tokens": last[1],
                    "cache_write_input_tokens": 0,
                    "output_tokens": last[2], "reasoning_output_tokens": 0,
                    "total_tokens": last[0] + last[2]
                }}}
            })
        };
        [
            serde_json::json!({"type": "session_meta", "timestamp": "2026-09-14T08:29:00Z", "payload": {"id": "PRIVATE-CODEX-ID", "model_provider": "openai"}}),
            serde_json::json!({"type": "turn_context", "timestamp": "2026-09-14T08:29:30Z", "payload": {"model": "gpt-6"}}),
            usage("2026-09-14T08:30:00Z", [100, 60, 20], [100, 60, 20]),
            serde_json::json!({"type": "response_item", "timestamp": "2026-09-14T08:30:30Z", "payload": {"type": "message", "role": "assistant", "content": []}}),
            usage("2026-09-14T08:31:00Z", [180, 100, 50], [80, 40, 30]),
        ]
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
    }

    /// `claude_bytes` with every recorded time moved `days` later, so two
    /// copies are two sessions at two times.
    fn claude_bytes_shifted(days: i64) -> Vec<u8> {
        let text = String::from_utf8(claude_bytes()).unwrap();
        let day = 14 + days;
        text.replace("2026-09-14T", &format!("2026-09-{day:02}T"))
            .into_bytes()
    }

    fn written() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 20, 10, 0, 0).unwrap()
    }

    fn now() -> DateTime<Utc> {
        written() + Duration::hours(2)
    }

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    struct Fixture {
        dir: tempfile::TempDir,
        pass: CounterPass,
        seed: u8,
    }

    impl Fixture {
        fn new() -> Self {
            Self::with_seed(3)
        }

        fn with_seed(seed: u8) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let pass = CounterPass::with_keys(
                dir.path().join(COUNTER_ROWS_FILE),
                Box::new(InMemoryDigestKeyStore::with_seed([seed; 32])),
            );
            Self { dir, pass, seed }
        }

        /// Rewrite every stored Codex row as made before the per-turn
        /// reader, with no series, and open the store again.
        fn without_codex_series(mut self) -> Self {
            let path = self.dir.path().join(COUNTER_ROWS_FILE);
            let mut value: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            for row in value["rows"].as_object_mut().unwrap().values_mut() {
                let body = row["body"].as_object_mut().unwrap();
                if body["kind"] == "codex" {
                    assert!(body.remove("series").is_some());
                }
            }
            std::fs::write(&path, value.to_string()).unwrap();
            self.pass = CounterPass::with_keys(
                path,
                Box::new(InMemoryDigestKeyStore::with_seed([self.seed; 32])),
            );
            self.pass.keys_load_or_create_for_test();
            self
        }

        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }

        /// One pass over `files`, each in the folder its entry names.
        fn run(&self, files: &[(&'static str, &Path, &str)]) {
            let candidates: Vec<CounterCandidate> = files
                .iter()
                .enumerate()
                .map(|(origin, (source, path, _))| CounterCandidate {
                    origin,
                    source,
                    path: path.to_path_buf(),
                    size_bytes: std::fs::metadata(path).unwrap().len(),
                    modified_at: written(),
                    started_at: None,
                    group_member_count: 0,
                    role: crate::daemon::insights_week::CandidateRole::Watched,
                })
                .collect();
            self.pass
                .run_pass(&candidates, &NeverFolders::NONE, now(), QUIET, &mut |c| {
                    (files[c.origin].2.to_string(), false)
                })
                .unwrap();
        }

        fn list(&self, never: &[&str], folders: &[(&str, &str)]) -> serde_json::Value {
            self.list_bounded(never, folders, WATCHED_SESSIONS_MAX)
        }

        fn list_bounded(
            &self,
            never: &[&str],
            folders: &[(&str, &str)],
            limit: usize,
        ) -> serde_json::Value {
            let folders: Vec<(String, String)> = folders
                .iter()
                .map(|(k, l)| (k.to_string(), l.to_string()))
                .collect();
            self.pass.sessions_value(
                true,
                &NeverFolders::These(never.iter().map(|k| k.to_string()).collect()),
                &folders,
                limit,
            )
        }

        fn page(&self, before: Option<&str>, limit: usize) -> serde_json::Value {
            self.page_result(before, limit).unwrap()
        }

        fn page_result(
            &self,
            before: Option<&str>,
            limit: usize,
        ) -> Result<serde_json::Value, &'static str> {
            let params = match before {
                Some(text) => serde_json::json!({"before": text, "limit": limit}),
                None => serde_json::json!({"limit": limit}),
            };
            self.pass
                .sessions_page_value(true, &NeverFolders::NONE, &[], page_params(&params)?)
        }

        fn drill(
            &self,
            reference: &str,
            never: &[&str],
        ) -> Result<serde_json::Value, &'static str> {
            self.pass.drill_value(
                true,
                reference,
                &NeverFolders::These(never.iter().map(|k| k.to_string()).collect()),
                utc(),
            )
        }
    }

    fn rows(value: &serde_json::Value) -> Vec<serde_json::Value> {
        value["sessions"].as_array().unwrap().clone()
    }

    /// No path, project key, session ID, message ID, digest or transcript
    /// text in an answer.
    fn assert_nothing_identifying(f: &Fixture, value: &serde_json::Value) {
        let text = value.to_string();
        for forbidden in [
            "PRIVATE_USER",
            "/Users",
            "PRIVATESESS",
            "PRIVATE-CODEX",
            "msg_",
            "PRIVATE_SUMMARY",
            f.dir.path().to_str().unwrap(),
        ] {
            assert!(!text.contains(forbidden), "{forbidden} crossed: {text}");
        }
        // No digest-length hex run (a keyed digest is 32 bytes).
        let hex_run = text
            .split(|c: char| !c.is_ascii_hexdigit())
            .map(str::len)
            .max()
            .unwrap_or(0);
        assert!(hex_run < 16, "a digest-like run crossed: {text}");
    }

    #[test]
    fn off_answers_disabled_and_reads_nothing() {
        let f = Fixture::new();
        let off = f.pass.sessions_value(false, &NeverFolders::NONE, &[], 10);
        assert_eq!(
            off,
            serde_json::json!({"enabled": false, "feed": "counter_pass"})
        );
        let drill = f
            .pass
            .drill_value(false, "t0", &NeverFolders::NONE, utc())
            .unwrap();
        assert_eq!(
            drill,
            serde_json::json!({"enabled": false, "feed": "counter_pass"})
        );
        assert!(f.pass.key_is_absent_for_test(), "no key made");
        assert!(!f.dir.path().join(COUNTER_ROWS_FILE).exists());
    }

    #[test]
    fn before_any_pass_the_list_is_readable_and_empty() {
        let f = Fixture::new();
        let value = f.list(&[], &[]);
        assert_eq!(value["enabled"], true);
        assert_eq!(value["readable"], true);
        assert_eq!(value["feed"], "counter_pass");
        assert_eq!(rows(&value), Vec::<serde_json::Value>::new());
        assert_eq!(f.drill("t0", &[]), Err(ERR_SESSION_UNKNOWN));
        assert!(f.pass.key_is_absent_for_test(), "a read never makes a key");
    }

    #[test]
    fn an_unreadable_store_is_named_never_empty() {
        let f = Fixture::new();
        std::fs::write(f.dir.path().join(COUNTER_ROWS_FILE), b"{not json").unwrap();
        // A key is held (a pass made it); the store does not parse.
        f.pass.keys_load_or_create_for_test();
        let expected = serde_json::json!({
            "enabled": true,
            "feed": "counter_pass",
            "readable": false,
            "reason": UNREADABLE_STORE,
        });
        assert_eq!(
            f.pass.sessions_value(true, &NeverFolders::NONE, &[], 10),
            expected
        );
        assert_eq!(
            f.pass.drill_value(true, "t0", &NeverFolders::NONE, utc()),
            Ok(expected)
        );
    }

    #[test]
    fn sessions_list_newest_first_with_opaque_references() {
        let f = Fixture::new();
        let older = f.write("older.jsonl", &claude_bytes_shifted(0));
        let newer = f.write("newer.jsonl", &claude_bytes_shifted(2));
        let codex = f.write("codex.jsonl", &codex_bytes());
        f.run(&[
            (SOURCE_CLAUDE_CODE, &older, PROJECT),
            (SOURCE_CODEX, &codex, PROJECT),
            (SOURCE_CLAUDE_CODE, &newer, PROJECT),
        ]);
        let value = f.list(&[], &[(PROJECT, "PRIVATE_REPO")]);
        assert_eq!(value["readable"], true);
        assert_eq!(value["sessions_stored"], 3);
        assert_eq!(value["truncated"], false);
        let listed = rows(&value);
        let started: Vec<&str> = listed
            .iter()
            .map(|row| row["started_at"].as_str().unwrap())
            .collect();
        assert_eq!(
            started,
            [
                "2026-09-16T09:00:00+00:00",
                "2026-09-14T09:00:00+00:00",
                "2026-09-14T08:30:00+00:00",
            ]
        );
        let sources: Vec<&str> = listed
            .iter()
            .map(|row| row["source"].as_str().unwrap())
            .collect();
        assert_eq!(sources, ["claude_code", "claude_code", "codex"]);
        // Every row read turn by turn carries its turn count, Codex's
        // included (owner decision D11, settled).
        assert!(listed[0]["turns"].as_u64().unwrap() > 0);
        assert_eq!(listed[0]["turns"], listed[1]["turns"]);
        assert_eq!(listed[2]["turns"], 2);
        let mut references = std::collections::BTreeSet::new();
        for row in &listed {
            let reference = row["session_ref"].as_str().unwrap();
            // `t`, the row's order, `.`, the store's tag: digits only.
            let (order, tag) = reference
                .strip_prefix('t')
                .and_then(|rest| rest.split_once('.'))
                .unwrap_or_else(|| panic!("{reference}"));
            assert!(
                [order, tag]
                    .iter()
                    .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit())),
                "{reference}"
            );
            assert_eq!(tag, f.pass.store_tag().to_string());
            assert!(references.insert(reference.to_string()), "unique");
            let keys: std::collections::BTreeSet<&str> = row
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                keys,
                [
                    "analyzable",
                    "folder",
                    "inferred_task",
                    "session_ref",
                    "source",
                    "sparkline",
                    "started_at",
                    "tokens",
                    "turns",
                    "user_outcome",
                    "user_task",
                ]
                .into(),
                "only these fields cross"
            );
        }
        assert_nothing_identifying(&f, &value);
    }

    /// A session with no recorded time lists after every dated one, so the
    /// tab's first row (its default selection) is the newest dated session,
    /// even when the undated file was written last.
    #[test]
    fn undated_sessions_list_after_dated_ones() {
        let f = Fixture::new();
        let older = f.write("older.jsonl", &claude_bytes_shifted(0));
        let newer = f.write("newer.jsonl", &claude_bytes_shifted(2));
        // No line carries a time or a usage counter.
        let undated = f.write(
            "undated.jsonl",
            b"{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"PRIVATE_SUMMARY\"}}\n",
        );
        f.run(&[
            (SOURCE_CLAUDE_CODE, &older, PROJECT),
            (SOURCE_CLAUDE_CODE, &newer, PROJECT),
            (SOURCE_CLAUDE_CODE, &undated, PROJECT),
        ]);
        let listed = rows(&f.list(&[], &[]));
        let started: Vec<serde_json::Value> =
            listed.iter().map(|row| row["started_at"].clone()).collect();
        assert_eq!(
            started,
            [
                serde_json::json!("2026-09-16T09:00:00+00:00"),
                serde_json::json!("2026-09-14T09:00:00+00:00"),
                serde_json::Value::Null,
            ]
        );
    }

    /// The bound picks which sessions are listed by recency, dated or not;
    /// only then are the picked ones ordered dated first. So a session
    /// placed just now is not cut by a long tail of older dated ones, and
    /// it still lists after the dated rows that were kept.
    #[test]
    fn a_recent_undated_session_survives_the_bound_and_lists_last() {
        let f = Fixture::new();
        let a = f.write("a.jsonl", &claude_bytes_shifted(0));
        let b = f.write("b.jsonl", &claude_bytes_shifted(1));
        let c = f.write("c.jsonl", &claude_bytes_shifted(2));
        let undated = f.write(
            "undated.jsonl",
            b"{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"PRIVATE_SUMMARY\"}}\n",
        );
        f.run(&[
            (SOURCE_CLAUDE_CODE, &a, PROJECT),
            (SOURCE_CLAUDE_CODE, &b, PROJECT),
            (SOURCE_CLAUDE_CODE, &c, PROJECT),
            (SOURCE_CLAUDE_CODE, &undated, PROJECT),
        ]);
        let value = f.list_bounded(&[], &[], 2);
        assert_eq!(value["sessions_stored"], 4);
        assert_eq!(value["truncated"], true);
        let started: Vec<serde_json::Value> = rows(&value)
            .iter()
            .map(|row| row["started_at"].clone())
            .collect();
        assert_eq!(
            started,
            [
                serde_json::json!("2026-09-16T09:00:00+00:00"),
                serde_json::Value::Null,
            ]
        );
    }

    #[test]
    fn the_list_is_bounded_and_says_so() {
        let f = Fixture::new();
        let a = f.write("a.jsonl", &claude_bytes_shifted(0));
        let b = f.write("b.jsonl", &claude_bytes_shifted(1));
        let c = f.write("c.jsonl", &claude_bytes_shifted(2));
        f.run(&[
            (SOURCE_CLAUDE_CODE, &a, PROJECT),
            (SOURCE_CLAUDE_CODE, &b, PROJECT),
            (SOURCE_CLAUDE_CODE, &c, PROJECT),
        ]);
        let value = f.list_bounded(&[], &[], 2);
        assert_eq!(value["sessions_stored"], 3);
        assert_eq!(value["truncated"], true);
        let listed = rows(&value);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0]["started_at"], "2026-09-16T09:00:00+00:00");
        assert_eq!(listed[1]["started_at"], "2026-09-15T09:00:00+00:00");
    }

    #[test]
    fn a_never_folder_is_neither_listed_nor_drilled() {
        let f = Fixture::new();
        let mine = f.write("mine.jsonl", &claude_bytes_shifted(0));
        let other = f.write("other.jsonl", &claude_bytes_shifted(1));
        f.run(&[
            (SOURCE_CLAUDE_CODE, &mine, PROJECT),
            (SOURCE_CLAUDE_CODE, &other, OTHER),
        ]);
        let all = rows(&f.list(&[], &[]));
        assert_eq!(all.len(), 2);
        let hidden = all
            .iter()
            .find(|row| row["started_at"] == "2026-09-15T09:00:00+00:00")
            .unwrap()["session_ref"]
            .as_str()
            .unwrap()
            .to_string();
        let value = f.list(&[OTHER], &[]);
        assert_eq!(value["sessions_stored"], 1);
        let listed = rows(&value);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["started_at"], "2026-09-14T09:00:00+00:00");
        assert_eq!(f.drill(&hidden, &[OTHER]), Err(ERR_SESSION_UNKNOWN));
        assert!(f.drill(&hidden, &[]).is_ok());
        // A global Never override hides every folder.
        let every = f.pass.sessions_value(true, &NeverFolders::Every, &[], 10);
        assert_eq!(rows(&every), Vec::<serde_json::Value>::new());
        assert_eq!(
            f.pass
                .drill_value(true, &hidden, &NeverFolders::Every, utc()),
            Err(ERR_SESSION_UNKNOWN)
        );
    }

    #[test]
    fn the_folder_is_its_current_label_or_null() {
        let f = Fixture::new();
        let mine = f.write("mine.jsonl", &claude_bytes_shifted(0));
        let other = f.write("other.jsonl", &claude_bytes_shifted(1));
        f.run(&[
            (SOURCE_CLAUDE_CODE, &mine, PROJECT),
            (SOURCE_CLAUDE_CODE, &other, OTHER),
        ]);
        let folder_of = |value: &serde_json::Value, started: &str| {
            rows(value)
                .into_iter()
                .find(|row| row["started_at"] == started)
                .unwrap()["folder"]
                .clone()
        };
        let labelled = f.list(
            &[],
            &[(PROJECT, "trace-commons"), ("/elsewhere", "elsewhere")],
        );
        assert_eq!(
            folder_of(&labelled, "2026-09-14T09:00:00+00:00"),
            "trace-commons"
        );
        // No known folder has that digest: null, which the shell draws with
        // the core's word.
        assert_eq!(
            folder_of(&labelled, "2026-09-15T09:00:00+00:00"),
            serde_json::Value::Null
        );
        let none = f.list(&[], &[]);
        assert_eq!(
            folder_of(&none, "2026-09-14T09:00:00+00:00"),
            serde_json::Value::Null
        );
        assert_nothing_identifying(&f, &labelled);
    }

    /// The saved drill for `bytes`, through the same import a saved
    /// snapshot takes.
    fn saved_drill(
        f: &Fixture,
        name: &str,
        format: crate::insights::SourceFormat,
        bytes: &[u8],
    ) -> serde_json::Value {
        let path = f.write(name, bytes);
        let store = crate::insights::LocalInsightStore::open_with_digest_keys(
            &f.dir.path().join("saved-store"),
            Box::new(InMemoryDigestKeyStore::with_seed([9; 32])),
        )
        .unwrap();
        let insight = store.import(format, &path).unwrap();
        serde_json::to_value(crate::insights::session_drill::saved_session_drill(
            &insight,
            utc(),
        ))
        .unwrap()
    }

    fn without_feed_and_ref(mut drill: serde_json::Value) -> serde_json::Value {
        let object = drill.as_object_mut().unwrap();
        object.remove("feed");
        object.remove("session_ref");
        drill
    }

    #[test]
    fn the_drill_matches_the_saved_drill_for_the_same_bytes() {
        for (source, format, bytes) in [
            (
                SOURCE_CLAUDE_CODE,
                crate::insights::SourceFormat::ClaudeCode,
                claude_bytes(),
            ),
            (
                SOURCE_CODEX,
                crate::insights::SourceFormat::Codex,
                codex_bytes(),
            ),
        ] {
            let f = Fixture::new();
            let path = f.write("watched.jsonl", &bytes);
            f.run(&[(source, &path, PROJECT)]);
            // A saved snapshot holds no Codex per-turn series: compare
            // with a row made before the per-turn reader.
            let f = if source == SOURCE_CODEX {
                f.without_codex_series()
            } else {
                f
            };
            let listed = rows(&f.list(&[], &[]));
            assert_eq!(listed.len(), 1);
            let reference = listed[0]["session_ref"].as_str().unwrap().to_string();
            let answer = f.drill(&reference, &[]).unwrap();
            assert_eq!(answer["enabled"], true);
            assert_eq!(answer["readable"], true);
            assert_eq!(answer["feed"], "counter_pass");
            let drill = answer["session"].clone();
            assert_eq!(drill["feed"], "counter_pass");
            assert_eq!(drill["session_ref"], reference.as_str());
            let saved = saved_drill(&f, "saved.jsonl", format, &bytes);
            assert_eq!(saved["feed"], "saved");
            assert_eq!(
                without_feed_and_ref(drill),
                without_feed_and_ref(saved),
                "{source}"
            );
            assert_nothing_identifying(&f, &answer);
        }
    }

    #[test]
    fn a_codex_drill_draws_its_per_turn_series() {
        let f = Fixture::new();
        let path = f.write("codex.jsonl", &codex_bytes());
        f.run(&[(SOURCE_CODEX, &path, PROJECT)]);
        let listed = rows(&f.list(&[], &[]));
        let reference = listed[0]["session_ref"].as_str().unwrap().to_string();
        let drill = f.drill(&reference, &[]).unwrap()["session"].clone();
        assert_eq!(drill["source"], "codex");
        assert_eq!(drill["series_unavailable"], serde_json::Value::Null);
        assert_eq!(drill["series"].as_array().unwrap().len(), 2);
        assert_eq!(drill["turns"], 2);
        // The observed change, the one basis a Codex row's tokens have: the
        // same figure once its series is trimmed (below).
        assert_eq!(drill["tokens"], 110);
        assert_eq!(drill["state"], "partial");
        assert_eq!(
            drill["reasons"],
            serde_json::json!(["codex_baseline_excluded"])
        );
        // The listed row counts what the drill draws.
        assert_eq!(listed[0]["tokens"], drill["tokens"]);
        assert_eq!(listed[0]["turns"], 2);
        assert!(listed[0]["sparkline"].is_object());
        assert_nothing_identifying(&f, &drill);
    }

    #[test]
    fn a_codex_drill_says_per_turn_usage_is_not_recorded() {
        let f = Fixture::new();
        let path = f.write("codex.jsonl", &codex_bytes());
        f.run(&[(SOURCE_CODEX, &path, PROJECT)]);
        // Made before the per-turn reader.
        let f = f.without_codex_series();
        let reference = rows(&f.list(&[], &[]))[0]["session_ref"]
            .as_str()
            .unwrap()
            .to_string();
        let drill = f.drill(&reference, &[]).unwrap()["session"].clone();
        assert_eq!(drill["source"], "codex");
        assert_eq!(drill["series"], serde_json::Value::Null);
        assert_eq!(drill["series_unavailable"], "not_recorded");
        assert_eq!(drill["tokens"], 110);
        // The time between its first and last token count: Codex's time
        // evidence is its counters, as a saved snapshot's is.
        assert_eq!(drill["span_secs"], 60);
    }

    #[test]
    fn a_row_stored_before_its_span_was_kept_is_read_once_more() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[(SOURCE_CLAUDE_CODE, &path, PROJECT)]);
        // The store as a daemon before this change wrote it.
        let rows_path = f.dir.path().join(COUNTER_ROWS_FILE);
        let mut store: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&rows_path).unwrap()).unwrap();
        for row in store["rows"].as_object_mut().unwrap().values_mut() {
            let row = row.as_object_mut().unwrap();
            for field in ["earliest_event_at", "latest_event_at", "event_span_read"] {
                assert!(row.remove(field).is_some(), "{field}");
            }
        }
        std::fs::write(&rows_path, serde_json::to_vec(&store).unwrap()).unwrap();
        let old = Fixture {
            pass: CounterPass::with_keys(
                rows_path.clone(),
                Box::new(InMemoryDigestKeyStore::with_seed([3; 32])),
            ),
            dir: f.dir,
            seed: 3,
        };
        old.pass.keys_load_or_create_for_test();
        let reference = rows(&old.list(&[], &[]))[0]["session_ref"]
            .as_str()
            .unwrap()
            .to_string();
        let before = old.drill(&reference, &[]).unwrap();
        assert_eq!(before["session"]["span_secs"], serde_json::Value::Null);
        assert_eq!(before["session"]["date"], serde_json::Value::Null);
        // The file is unchanged, and still read again, once.
        old.run(&[(SOURCE_CLAUDE_CODE, &path, PROJECT)]);
        let reference = rows(&old.list(&[], &[]))[0]["session_ref"]
            .as_str()
            .unwrap()
            .to_string();
        let after = old.drill(&reference, &[]).unwrap();
        assert!(after["session"]["span_secs"].as_u64().unwrap() > 0);
        assert_eq!(after["session"]["date"], "2026-09-14");
        let again = rows(&old.list(&[], &[]))[0]["session_ref"].clone();
        old.run(&[(SOURCE_CLAUDE_CODE, &path, PROJECT)]);
        assert_eq!(
            rows(&old.list(&[], &[]))[0]["session_ref"],
            again,
            "not read a third time"
        );
    }

    #[test]
    fn a_folder_only_the_watcher_saw_keeps_its_own_spelling() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("IronWire");
        std::fs::create_dir(&folder).unwrap();
        let (key, _) = super::super::policy::project_for(folder.to_str());
        assert_eq!(folder_label(&key, None, &[]), "IronWire");
        // A stored spelling wins, as `list_projects` uses it.
        assert_eq!(
            folder_label(&key, Some(folder.display().to_string()), &[]),
            "IronWire"
        );
    }

    // Paging (owner request 2026-10-20: every session, newest first, in a
    // lazily loaded scroll).

    fn undated_bytes() -> &'static [u8] {
        b"{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"PRIVATE_SUMMARY\"}}\n"
    }

    fn refs(value: &serde_json::Value) -> Vec<String> {
        rows(value)
            .iter()
            .map(|row| row["session_ref"].as_str().unwrap().to_string())
            .collect()
    }

    /// Five dated sessions a day apart and one undated one, in one folder.
    fn five_dated_and_one_undated(f: &Fixture) -> Vec<(&'static str, PathBuf, &'static str)> {
        let mut files: Vec<(&'static str, PathBuf, &'static str)> = (0..5)
            .map(|day| {
                (
                    SOURCE_CLAUDE_CODE,
                    f.write(&format!("d{day}.jsonl"), &claude_bytes_shifted(day)),
                    PROJECT,
                )
            })
            .collect();
        files.push((
            SOURCE_CLAUDE_CODE,
            f.write("undated.jsonl", undated_bytes()),
            PROJECT,
        ));
        files
    }

    fn run_owned(f: &Fixture, files: &[(&'static str, PathBuf, &'static str)]) {
        let borrowed: Vec<(&'static str, &Path, &str)> = files
            .iter()
            .map(|(source, path, project)| (*source, path.as_path(), *project))
            .collect();
        f.run(&borrowed);
    }

    #[test]
    fn a_cursor_round_trips_and_garbage_is_refused() {
        let cursor = SessionsCursor {
            at: Utc.with_ymd_and_hms(2026, 9, 16, 9, 0, 0).unwrap() + Duration::nanoseconds(7),
            seq: 12,
        };
        let text = cursor.encode(77);
        assert!(text.starts_with('p'), "{text}");
        assert_eq!(SessionsCursor::decode(&text), Some((cursor, 77)));
        for bad in [
            "",
            "p",
            "t12",
            "p1.2",
            "p1.2.3",
            "p1.2.3.4.5",
            "px.0.1.2",
            "p1.2000000000.3.4",
            "p1.-2.3.4",
            // A sign is not a digit, though integer parsing accepts one.
            "p+5.0.+3.7",
            "p5.+0.3.7",
            "p5.0.3.+7",
            "p5.0.+3.7",
            "p5.0.3.",
            "p 5.0.3.7",
        ] {
            assert_eq!(SessionsCursor::decode(bad), None, "{bad}");
        }
    }

    /// A cursor from before the store started again (a cleared key, a new
    /// device) is refused, so the client reads the first page: its
    /// references `t0`, `t1`, ... now name other sessions.
    #[test]
    fn a_cursor_from_another_store_is_refused() {
        let f = Fixture::new();
        let files = five_dated_and_one_undated(&f);
        run_owned(&f, &files);
        let first = f.page(None, 3);
        let cursor = first["next"].as_str().unwrap().to_string();
        assert!(f.page_result(Some(&cursor), 3).is_ok(), "same store");

        // The store started again under another key: its `t0`, `t1`, ...
        // are other sessions.
        let g = Fixture::with_seed(4);
        let other = five_dated_and_one_undated(&g);
        run_owned(&g, &other);
        assert_eq!(
            g.page_result(Some(&cursor), 3),
            Err(ERR_SESSIONS_CURSOR_INVALID)
        );
        // The first page of the new store is answered and its cursor works.
        let again = g.page(None, 3);
        assert!(
            g.page_result(Some(again["next"].as_str().unwrap()), 3)
                .is_ok()
        );
    }

    /// `revision` moves only when a row is made: a pass that stores nothing
    /// leaves it (and so a client's stamp) as it was, while `updated_at`
    /// moves on every pass; a new row moves it.
    #[test]
    fn the_revision_moves_only_when_a_row_is_made() {
        let f = Fixture::new();
        let mut files = five_dated_and_one_undated(&f);
        run_owned(&f, &files);
        let before = f.page(None, 3);
        assert!(before["revision"].is_u64(), "{before}");
        run_owned(&f, &files);
        assert_eq!(
            f.page(None, 3)["revision"],
            before["revision"],
            "nothing stored"
        );
        files.push((
            SOURCE_CLAUDE_CODE,
            f.write("d9.jsonl", &claude_bytes_shifted(9)),
            PROJECT,
        ));
        run_owned(&f, &files);
        assert_ne!(
            f.page(None, 3)["revision"],
            before["revision"],
            "a row was made"
        );
    }

    /// A store started again under the same key (its file unreadable, so
    /// the pass begins an empty one) hands `t0`, `t1`, ... to other
    /// sessions: a cursor from before is refused, though the key is the
    /// same.
    #[test]
    fn a_cursor_from_a_store_restarted_under_the_same_key_is_refused() {
        let mut f = Fixture::new();
        let files = five_dated_and_one_undated(&f);
        run_owned(&f, &files);
        let cursor = f.page(None, 3)["next"].as_str().unwrap().to_string();
        assert!(f.page_result(Some(&cursor), 3).is_ok(), "same store");

        // The file is unreadable when the daemon starts again, under the
        // same key: the pass starts an empty store and reads the sessions
        // into it afresh.
        let path = f.dir.path().join(COUNTER_ROWS_FILE);
        std::fs::write(&path, b"not a store").unwrap();
        f.pass = CounterPass::with_keys(path, Box::new(InMemoryDigestKeyStore::with_seed([3; 32])));
        run_owned(&f, &files);
        assert_eq!(
            f.page_result(Some(&cursor), 3),
            Err(ERR_SESSIONS_CURSOR_INVALID)
        );
        let again = f.page(None, 3);
        assert!(
            f.page_result(Some(again["next"].as_str().unwrap()), 3)
                .is_ok()
        );
    }

    /// Rows are placed by their first event, and stored late: a row whose
    /// event falls between page 1's top and its cursor is above the cursor,
    /// so no later page returns it. It lists at its place on the next
    /// first-page read.
    #[test]
    fn a_late_row_between_the_top_and_the_cursor_waits_for_a_first_page_read() {
        let f = Fixture::new();
        let mut files = five_dated_and_one_undated(&f);
        run_owned(&f, &files);
        // The undated row is the most recent by recency, so four rows by
        // recency list the undated one last.
        let first = f.page(None, 4);
        let first_refs = refs(&first);
        assert_eq!(
            rows(&first)[0]["started_at"],
            "2026-09-18T09:00:00+00:00",
            "page 1's top"
        );
        assert_eq!(
            rows(&first)[2]["started_at"],
            "2026-09-16T09:00:00+00:00",
            "page 1's cursor"
        );
        files.push((
            SOURCE_CLAUDE_CODE,
            f.write("late.jsonl", &claude_bytes_shifted(3)),
            PROJECT,
        ));
        run_owned(&f, &files);
        let fresh = f.page(None, 5);
        let late = refs(&fresh)[1].clone();
        assert!(
            !first_refs.contains(&late),
            "a row the first read did not list"
        );
        // Paging on from the old cursor never lists it.
        let second = f.page(first["next"].as_str(), 7);
        assert!(!refs(&second).contains(&late), "{late} between the pages");
        assert_eq!(second["sessions_stored"], 7, "the count says a row came");
        // The next first-page read lists it, between the old top and cursor.
        assert_eq!(rows(&fresh)[1]["started_at"], "2026-09-17T09:00:00+00:00");
    }

    /// A page is chosen without copying a row: `select_page` takes rows of
    /// a type that cannot be cloned, so the compiler holds it to moving
    /// (the daemon passes `&CounterRow`), and only the kept rows are
    /// serialized.
    #[test]
    fn a_page_is_chosen_without_copying_a_row() {
        struct Uncopyable(i64);
        let at = |secs: i64| SessionsCursor {
            at: DateTime::<Utc>::from_timestamp(secs, 0).unwrap(),
            seq: secs as u64,
        };
        let rows = (0..6).map(Uncopyable).collect::<Vec<_>>();
        let (taken, next) = select_page(
            rows,
            |row| at(row.0),
            |_| true,
            SessionsPage {
                before: None,
                store: 0,
                limit: 3,
            },
        );
        assert_eq!(taken.iter().map(|row| row.0).collect::<Vec<_>>(), [5, 4, 3]);
        assert_eq!(next, Some(at(3)));
    }

    #[test]
    fn page_params_default_to_the_first_page_and_refuse_bad_values() {
        assert_eq!(page_params(&serde_json::json!({})), Ok(SessionsPage::FIRST));
        assert_eq!(
            page_params(&serde_json::Value::Null),
            Ok(SessionsPage::FIRST)
        );
        assert_eq!(
            page_params(&serde_json::json!({"before": null, "limit": null})),
            Ok(SessionsPage::FIRST)
        );
        assert_eq!(
            page_params(&serde_json::json!({"limit": 5000})),
            Ok(SessionsPage::FIRST),
            "held to the bound"
        );
        let cursor = SessionsCursor {
            at: Utc.with_ymd_and_hms(2026, 9, 16, 9, 0, 0).unwrap(),
            seq: 3,
        };
        assert_eq!(
            page_params(&serde_json::json!({"before": cursor.encode(41), "limit": 25})),
            Ok(SessionsPage {
                before: Some(cursor),
                store: 41,
                limit: 25
            })
        );
        for bad in [
            serde_json::json!({"before": 7}),
            serde_json::json!({"before": "t3"}),
        ] {
            assert_eq!(page_params(&bad), Err(ERR_SESSIONS_CURSOR_INVALID), "{bad}");
        }
        for bad in [
            serde_json::json!({"limit": 0}),
            serde_json::json!({"limit": -1}),
            serde_json::json!({"limit": 2.5}),
            serde_json::json!({"limit": "10"}),
        ] {
            assert_eq!(page_params(&bad), Err(ERR_SESSIONS_LIMIT_INVALID), "{bad}");
        }
    }

    /// Pages of two walk every session exactly once: newest first, each
    /// page's undated rows last, and the last page says it is the end.
    #[test]
    fn pages_walk_every_session_once_newest_first() {
        let f = Fixture::new();
        run_owned(&f, &five_dated_and_one_undated(&f));
        let whole = f.list(&[], &[]);
        assert_eq!(whole["next"], serde_json::Value::Null, "one page holds all");
        let mut walked: Vec<serde_json::Value> = Vec::new();
        let mut before: Option<String> = None;
        let mut pages = 0;
        loop {
            let page = f.page(before.as_deref(), 2);
            pages += 1;
            assert_eq!(page["sessions_stored"], 6, "the total, never page-relative");
            walked.extend(rows(&page));
            match page["next"].as_str() {
                Some(next) => {
                    assert_eq!(page["truncated"], true);
                    assert_nothing_identifying(&f, &page);
                    before = Some(next.to_string());
                }
                None => {
                    assert_eq!(page["truncated"], false);
                    break;
                }
            }
            assert!(pages < 10, "the walk ends");
        }
        assert_eq!(pages, 3);
        let walked_refs: Vec<String> = walked
            .iter()
            .map(|row| row["session_ref"].as_str().unwrap().to_string())
            .collect();
        let unique: std::collections::BTreeSet<&String> = walked_refs.iter().collect();
        assert_eq!(unique.len(), 6, "no duplicates: {walked_refs:?}");
        let mut every = refs(&whole);
        every.sort();
        let mut seen = walked_refs.clone();
        seen.sort();
        assert_eq!(seen, every, "no gaps");
        // The undated row is the most recent by its file's write, so it is
        // in the first page, after that page's dated row.
        let started: Vec<serde_json::Value> =
            walked.iter().map(|row| row["started_at"].clone()).collect();
        assert_eq!(
            started,
            [
                serde_json::json!("2026-09-18T09:00:00+00:00"),
                serde_json::Value::Null,
                serde_json::json!("2026-09-17T09:00:00+00:00"),
                serde_json::json!("2026-09-16T09:00:00+00:00"),
                serde_json::json!("2026-09-15T09:00:00+00:00"),
                serde_json::json!("2026-09-14T09:00:00+00:00"),
            ]
        );
    }

    /// A page's next cursor is its last row by recency, not its last row as
    /// listed: an undated row listed last in a page must not send the next
    /// page back to it, nor skip the dated row after it.
    #[test]
    fn the_cursor_follows_recency_not_the_listed_order() {
        let f = Fixture::new();
        run_owned(&f, &five_dated_and_one_undated(&f));
        // By recency the undated row (written 2026-09-20) and 09-18 are
        // taken; the page lists 09-18 first and the undated row last.
        let first = f.page(None, 2);
        assert_eq!(
            rows(&first)
                .iter()
                .map(|row| row["started_at"].clone())
                .collect::<Vec<_>>(),
            [
                serde_json::json!("2026-09-18T09:00:00+00:00"),
                serde_json::Value::Null,
            ]
        );
        // A cursor taken from the listed last row (the undated one) would
        // list 09-18 again; the recency cursor goes on to 09-17.
        let second = f.page(first["next"].as_str(), 2);
        assert_eq!(
            rows(&second)
                .iter()
                .map(|row| row["started_at"].clone())
                .collect::<Vec<_>>(),
            [
                serde_json::json!("2026-09-17T09:00:00+00:00"),
                serde_json::json!("2026-09-16T09:00:00+00:00"),
            ]
        );
    }

    /// A session stored between two page reads lists at the top on the next
    /// first-page read; the pages already under way neither repeat a row nor
    /// skip one.
    #[test]
    fn a_session_stored_between_pages_neither_repeats_nor_shifts_a_row() {
        let f = Fixture::new();
        let mut files = five_dated_and_one_undated(&f);
        run_owned(&f, &files);
        let first = f.page(None, 3);
        let first_refs = refs(&first);
        files.push((
            SOURCE_CLAUDE_CODE,
            f.write("d9.jsonl", &claude_bytes_shifted(9)),
            PROJECT,
        ));
        run_owned(&f, &files);
        let second = f.page(first["next"].as_str(), 3);
        let second_refs = refs(&second);
        assert_eq!(second["next"], serde_json::Value::Null);
        assert_eq!(second_refs.len(), 3);
        for reference in &second_refs {
            assert!(!first_refs.contains(reference), "{reference} repeated");
        }
        assert_eq!(second["sessions_stored"], 7);
        // The new one leads a fresh read.
        assert_eq!(
            rows(&f.page(None, 3))[0]["started_at"],
            "2026-09-23T09:00:00+00:00"
        );
    }

    #[test]
    fn a_cursor_past_the_end_is_an_empty_last_page() {
        let f = Fixture::new();
        run_owned(&f, &five_dated_and_one_undated(&f));
        let all = f.page(None, 6);
        assert_eq!(
            all["next"],
            serde_json::Value::Null,
            "exactly full is the end"
        );
        let oldest = SessionsCursor {
            at: Utc.with_ymd_and_hms(2000, 1, 1, 0, 0, 0).unwrap(),
            seq: 0,
        };
        let past = f.page(Some(&oldest.encode(f.pass.store_tag())), 6);
        assert_eq!(rows(&past), Vec::<serde_json::Value>::new());
        assert_eq!(past["next"], serde_json::Value::Null);
        assert_eq!(past["truncated"], false);
    }

    /// Each row carries its tokens and a short integer sparkline: a Claude
    /// Code row both, a Codex row its total and no sparkline (no per-turn
    /// series is recorded), a row with no counters neither.
    #[test]
    fn rows_carry_tokens_and_a_bounded_sparkline() {
        let f = Fixture::new();
        let claude = f.write("claude.jsonl", &claude_bytes_shifted(2));
        let codex = f.write("codex.jsonl", &codex_bytes());
        let undated = f.write("undated.jsonl", undated_bytes());
        f.run(&[
            (SOURCE_CLAUDE_CODE, &claude, PROJECT),
            (SOURCE_CODEX, &codex, PROJECT),
            (SOURCE_CLAUDE_CODE, &undated, PROJECT),
        ]);
        let value = f.list(&[], &[]);
        let listed = rows(&value);
        let claude_row = &listed[0];
        assert_eq!(claude_row["source"], "claude_code");
        let points = claude_row["sparkline"]["points"].as_array().unwrap();
        assert!(!points.is_empty());
        assert!(points.len() <= crate::insights::session_sparkline::SPARKLINE_POINTS);
        assert!(points.iter().all(serde_json::Value::is_u64));
        let peak = claude_row["sparkline"]["peak_tokens"].as_u64().unwrap();
        assert_eq!(
            points.iter().filter_map(serde_json::Value::as_u64).max(),
            Some(peak)
        );
        assert!(claude_row["sparkline"]["peak_turn"].as_u64().unwrap() >= 1);
        // The row's tokens are the drill's header total for the same row.
        let drill = f
            .drill(claude_row["session_ref"].as_str().unwrap(), &[])
            .unwrap();
        assert_eq!(claude_row["tokens"], drill["session"]["tokens"]);
        let codex_row = &listed[1];
        assert_eq!(codex_row["source"], "codex");
        // Drawn over its per-turn series, as a Claude row's is.
        assert!(codex_row["sparkline"].is_object());
        let codex_drill = f
            .drill(codex_row["session_ref"].as_str().unwrap(), &[])
            .unwrap();
        assert_eq!(codex_row["tokens"], codex_drill["session"]["tokens"]);
        assert!(codex_row["tokens"].is_u64());
        let undated_row = &listed[2];
        assert_eq!(undated_row["tokens"], serde_json::Value::Null);
        assert_eq!(undated_row["sparkline"], serde_json::Value::Null);
        assert_nothing_identifying(&f, &value);
    }

    // The methods, through the daemon.

    fn shared() -> (tempfile::TempDir, DaemonShared) {
        let (dir, store) = crate::config::tests_support::temp_store();
        (dir, DaemonShared::load(store).unwrap())
    }

    fn call(method: &str, params: serde_json::Value) -> Request {
        Request {
            id: 1,
            method: method.to_string(),
            params,
        }
    }

    #[test]
    fn the_methods_are_advertised_dispatched_and_local() {
        for method in ["insights_sessions", "insights_session_drill"] {
            assert!(super::super::ipc::METHODS.contains(&method), "{method}");
            assert!(
                super::super::ipc::DEV_DRY_RUN_LOCAL_METHODS.contains(&method),
                "{method}"
            );
            let (_dir, mut s) = shared();
            s.dev_dry_run = true;
            s.settings.lock().unwrap().insights_counter_pass = false;
            let r = super::super::ipc::handle_request(
                &s,
                &call(method, serde_json::json!({"session_ref": "t0"})),
            );
            assert_eq!(r.result.unwrap()["enabled"], false, "{method}");
        }
    }

    /// A bad page param is refused whether counting is on or off; no params,
    /// or `before: null`, is the first page as before paging existed.
    #[test]
    fn the_list_refuses_a_bad_page_and_takes_none() {
        let (_dir, s) = shared();
        for on in [false, true] {
            s.settings.lock().unwrap().insights_counter_pass = on;
            for (params, label) in [
                (
                    serde_json::json!({"before": "nope"}),
                    ERR_SESSIONS_CURSOR_INVALID,
                ),
                (serde_json::json!({"limit": 0}), ERR_SESSIONS_LIMIT_INVALID),
            ] {
                let r = handle_sessions(&s, &call("insights_sessions", params));
                let err = r.error.unwrap();
                assert_eq!(err.code, ERR_BAD_PARAMS);
                assert_eq!(err.message, label);
            }
            for params in [serde_json::json!({}), serde_json::json!({"before": null})] {
                let r = handle_sessions(&s, &call("insights_sessions", params));
                assert_eq!(r.result.unwrap()["enabled"], on);
            }
        }
        let r = handle_sessions(&s, &call("insights_sessions", serde_json::json!({})));
        let value = r.result.unwrap();
        assert_eq!(value["readable"], true);
        assert_eq!(value["next"], serde_json::Value::Null);
        assert_eq!(value["truncated"], false);
    }

    #[test]
    fn the_drill_refuses_a_bad_reference_or_tz() {
        let (_dir, s) = shared();
        for on in [false, true] {
            s.settings.lock().unwrap().insights_counter_pass = on;
            for params in [serde_json::json!({}), serde_json::json!({"session_ref": 7})] {
                let r = handle_session_drill(&s, &call("insights_session_drill", params));
                let err = r.error.unwrap();
                assert_eq!(err.code, ERR_BAD_PARAMS);
                assert_eq!(err.message, ERR_SESSION_REF_INVALID);
            }
            let r = handle_session_drill(
                &s,
                &call(
                    "insights_session_drill",
                    serde_json::json!({"session_ref": "t0", "tz": "UTC"}),
                ),
            );
            assert_eq!(
                r.error.unwrap().message,
                super::super::insights_glance::ERR_TZ_INVALID
            );
        }
        let r = handle_session_drill(
            &s,
            &call(
                "insights_session_drill",
                serde_json::json!({"session_ref": "t0", "tz": 0}),
            ),
        );
        assert_eq!(r.error.unwrap().message, ERR_SESSION_UNKNOWN);
    }

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
        let expected = serde_json::json!({
            "enabled": true,
            "feed": "counter_pass",
            "readable": false,
            "reason": UNREADABLE_POLICY,
        });
        let list = handle_sessions(&s, &call("insights_sessions", serde_json::json!({})));
        assert_eq!(list.result.unwrap(), expected);
        let drill = handle_session_drill(
            &s,
            &call(
                "insights_session_drill",
                serde_json::json!({"session_ref": "t0", "tz": 0}),
            ),
        );
        assert_eq!(drill.result.unwrap(), expected);
    }

    #[test]
    fn the_daemon_labels_a_folder_as_list_projects_does() {
        use crate::daemon::test_paths::abs_key;
        let (dir, s) = shared();
        s.settings.lock().unwrap().insights_counter_pass = true;
        let project = abs_key("Users/z/code/trace-commons");
        let unknown = abs_key("Users/z/code/not-known");
        s.policy
            .lock()
            .unwrap()
            .set_mode(
                &project,
                super::super::policy::ProjectMode::NotifyOnly,
                now(),
            )
            .unwrap();
        let a = dir.path().join("a.jsonl");
        let b = dir.path().join("b.jsonl");
        std::fs::write(&a, claude_bytes_shifted(0)).unwrap();
        std::fs::write(&b, claude_bytes_shifted(1)).unwrap();
        let folders = [project.clone(), unknown.clone()];
        let candidates: Vec<CounterCandidate> = [&a, &b]
            .iter()
            .enumerate()
            .map(|(origin, path)| CounterCandidate {
                origin,
                source: SOURCE_CLAUDE_CODE,
                path: path.to_path_buf(),
                size_bytes: std::fs::metadata(path).unwrap().len(),
                modified_at: written(),
                started_at: None,
                group_member_count: 0,
                role: crate::daemon::insights_week::CandidateRole::Watched,
            })
            .collect();
        s.insights_counter
            .run_pass(&candidates, &NeverFolders::NONE, now(), QUIET, &mut |c| {
                (folders[c.origin].clone(), false)
            })
            .unwrap();
        let listed = handle_sessions(&s, &call("insights_sessions", serde_json::json!({})))
            .result
            .unwrap();
        let label =
            super::super::ipc::handle_request(&s, &call("list_projects", serde_json::json!({})))
                .result
                .unwrap()["projects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["mode"] == "notify_only")
                .unwrap()["project_label"]
                .clone();
        assert_eq!(label, "trace-commons");
        let sessions = listed["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0]["folder"], serde_json::Value::Null);
        assert_eq!(sessions[1]["folder"], label);
        let text = listed.to_string();
        assert!(!text.contains("/Users"), "{text}");
        assert!(!text.contains(dir.path().to_str().unwrap()), "{text}");
        // The drill answers the listed reference.
        let reference = sessions[1]["session_ref"].as_str().unwrap();
        let drill = handle_session_drill(
            &s,
            &call(
                "insights_session_drill",
                serde_json::json!({"session_ref": reference, "tz": 0}),
            ),
        )
        .result
        .unwrap();
        assert_eq!(drill["session"]["session_ref"], reference);
        assert_eq!(drill["session"]["feed"], "counter_pass");
    }

    // The person's own labels (`insights_session_label`).

    impl Fixture {
        /// One pass over `files` at `at`, with `never` the Never folders;
        /// a file in one of them is not read.
        fn run_at(&self, files: &[(&'static str, &Path, &str)], never: &[&str], at: DateTime<Utc>) {
            let candidates: Vec<CounterCandidate> = files
                .iter()
                .enumerate()
                .map(|(origin, (source, path, _))| CounterCandidate {
                    origin,
                    source,
                    path: path.to_path_buf(),
                    size_bytes: std::fs::metadata(path).unwrap().len(),
                    modified_at: written(),
                    started_at: None,
                    group_member_count: 0,
                    role: crate::daemon::insights_week::CandidateRole::Watched,
                })
                .collect();
            self.pass
                .run_pass(
                    &candidates,
                    &NeverFolders::These(never.iter().map(|k| k.to_string()).collect()),
                    at,
                    QUIET,
                    &mut |c| {
                        let folder = files[c.origin].2;
                        (folder.to_string(), never.contains(&folder))
                    },
                )
                .unwrap();
        }

        fn label(
            &self,
            params: serde_json::Value,
            never: &[&str],
        ) -> Result<serde_json::Value, &'static str> {
            let (reference, change) = label_params(&params)?;
            self.pass.label_value(
                true,
                &reference,
                change,
                &NeverFolders::These(never.iter().map(|k| k.to_string()).collect()),
                &[(PROJECT.to_string(), "PRIVATE_REPO".to_string())],
            )
        }

        /// The labels the labels file holds, however many rows there are.
        fn stored_labels(&self) -> usize {
            let path = self.dir.path().join(SESSION_LABELS_FILE);
            let Ok(text) = std::fs::read_to_string(path) else {
                return 0;
            };
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            value
                .get("labels")
                .and_then(|labels| labels.as_object())
                .map_or(0, |labels| labels.len())
        }
    }

    fn only(value: &serde_json::Value) -> serde_json::Value {
        let listed = rows(value);
        assert_eq!(listed.len(), 1, "{value}");
        listed[0].clone()
    }

    #[test]
    fn a_label_is_set_cleared_and_answered_as_the_listed_row() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[(SOURCE_CLAUDE_CODE, &path, PROJECT)]);
        let listed = only(&f.list(&[], &[(PROJECT, "PRIVATE_REPO")]));
        // Before any label: the inferred task, and nothing of the person's.
        assert!(listed["inferred_task"].is_string(), "{listed}");
        assert!(listed["user_task"].is_null());
        assert!(listed["user_outcome"].is_null());
        let reference = listed["session_ref"].as_str().unwrap().to_string();

        let answer = f
            .label(
                serde_json::json!({"session_ref": reference, "task": "bug_fix"}),
                &[],
            )
            .unwrap();
        assert_eq!(answer["enabled"], true);
        assert_eq!(answer["readable"], true);
        let row = &answer["session"];
        assert_eq!(row["user_task"], "bug_fix");
        assert!(row["user_outcome"].is_null(), "absent keeps it unset");
        // The answer is the row as the list now gives it.
        assert_eq!(*row, only(&f.list(&[], &[(PROJECT, "PRIVATE_REPO")])));
        assert_eq!(row["inferred_task"], listed["inferred_task"]);

        let row = f
            .label(
                serde_json::json!({"session_ref": reference, "outcome": "partly_kept"}),
                &[],
            )
            .unwrap()["session"]
            .clone();
        assert_eq!(row["user_task"], "bug_fix", "absent keeps the task");
        assert_eq!(row["user_outcome"], "partly_kept");

        let row = f
            .label(
                serde_json::json!({"session_ref": reference, "task": null}),
                &[],
            )
            .unwrap()["session"]
            .clone();
        assert!(row["user_task"].is_null(), "null clears it");
        assert_eq!(row["user_outcome"], "partly_kept");
        assert_eq!(f.stored_labels(), 1);

        f.label(
            serde_json::json!({"session_ref": reference, "outcome": null}),
            &[],
        )
        .unwrap();
        assert_eq!(f.stored_labels(), 0, "a label with nothing set is not kept");
        assert_nothing_identifying(&f, &f.list(&[], &[]));
    }

    #[test]
    fn only_the_closed_words_are_taken() {
        for params in [
            serde_json::json!({"task": "bug_fix"}),
            serde_json::json!({"session_ref": 7, "task": "bug_fix"}),
        ] {
            assert_eq!(
                label_params(&params),
                Err(ERR_SESSION_REF_INVALID),
                "{params}"
            );
        }
        for params in [
            serde_json::json!({"session_ref": "t0"}),
            serde_json::json!({"session_ref": "t0", "task": "no_edits"}),
            serde_json::json!({"session_ref": "t0", "task": "Bug fix"}),
            serde_json::json!({"session_ref": "t0", "task": 1}),
            serde_json::json!({"session_ref": "t0", "outcome": "accepted"}),
            serde_json::json!({"session_ref": "t0", "outcome": true}),
            serde_json::json!({"session_ref": "t0", "task": "docs", "outcome": "kept!"}),
        ] {
            assert_eq!(
                label_params(&params),
                Err(ERR_SESSION_LABEL_INVALID),
                "{params}"
            );
        }
        let (reference, change) = label_params(
            &serde_json::json!({"session_ref": "t3", "task": "tests", "outcome": null}),
        )
        .unwrap();
        assert_eq!(reference, "t3");
        assert_eq!(change.task, LabelEdit::Set(LabelledTask::Tests));
        assert_eq!(change.outcome, LabelEdit::Clear);
    }

    /// A row read again when its file grows is made under a new reference,
    /// and keeps the person's label: it is keyed by the session, not the
    /// reference.
    #[test]
    fn a_label_survives_the_row_being_read_again() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[(SOURCE_CLAUDE_CODE, &path, PROJECT)]);
        let before = only(&f.list(&[], &[]))["session_ref"]
            .as_str()
            .unwrap()
            .to_string();
        f.label(
            serde_json::json!({"session_ref": before, "task": "refactor", "outcome": "kept"}),
            &[],
        )
        .unwrap();

        let mut grown = claude_bytes();
        grown.extend_from_slice(b"\n");
        std::fs::write(&path, grown).unwrap();
        f.run(&[(SOURCE_CLAUDE_CODE, &path, PROJECT)]);
        let row = only(&f.list(&[], &[]));
        let after = row["session_ref"].as_str().unwrap();
        assert_ne!(after, before, "made again under a new reference");
        assert_eq!(row["user_task"], "refactor");
        assert_eq!(row["user_outcome"], "kept");
        // The old reference names nothing now.
        assert_eq!(
            f.label(
                serde_json::json!({"session_ref": before, "outcome": "thrown_away"}),
                &[]
            ),
            Err(ERR_SESSION_UNKNOWN)
        );
        // A store opened again reads the same label.
        let reopened = CounterPass::with_keys(
            f.dir.path().join(COUNTER_ROWS_FILE),
            Box::new(InMemoryDigestKeyStore::with_seed([f.seed; 32])),
        );
        reopened.keys_load_or_create_for_test();
        let row = only(&reopened.sessions_value(true, &NeverFolders::NONE, &[], 10));
        assert_eq!(row["user_task"], "refactor");
    }

    /// No label is set on a row in a Never folder, and one whose folder
    /// becomes Never goes with its row: it does not come back when the
    /// folder is watched again.
    #[test]
    fn a_never_folder_keeps_no_label() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[(SOURCE_CLAUDE_CODE, &path, PROJECT)]);
        let reference = only(&f.list(&[], &[]))["session_ref"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            f.label(
                serde_json::json!({"session_ref": reference, "task": "docs"}),
                &[PROJECT]
            ),
            Err(ERR_SESSION_UNKNOWN)
        );
        assert_eq!(f.stored_labels(), 0);

        f.label(
            serde_json::json!({"session_ref": reference, "task": "docs"}),
            &[],
        )
        .unwrap();
        assert_eq!(f.stored_labels(), 1);
        // Never: hidden at once, and gone from the store with the row at
        // the next pass.
        assert_eq!(
            rows(&f.list(&[PROJECT], &[])),
            Vec::<serde_json::Value>::new()
        );
        f.run_at(&[(SOURCE_CLAUDE_CODE, &path, PROJECT)], &[PROJECT], now());
        assert_eq!(f.stored_labels(), 0);

        // Watched again: read again, with no label.
        f.run_at(&[(SOURCE_CLAUDE_CODE, &path, PROJECT)], &[], now());
        let row = only(&f.list(&[], &[]));
        assert!(row["user_task"].is_null(), "{row}");
    }

    /// A label goes with its row when the row ages out of the kept weeks.
    #[test]
    fn a_label_ages_out_with_its_row() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[(SOURCE_CLAUDE_CODE, &path, PROJECT)]);
        let reference = only(&f.list(&[], &[]))["session_ref"]
            .as_str()
            .unwrap()
            .to_string();
        f.label(
            serde_json::json!({"session_ref": reference, "outcome": "kept"}),
            &[],
        )
        .unwrap();
        let later = now()
            + Duration::weeks(crate::insights::analytics_constants::COUNTER_STORE_KEEP_WEEKS)
            + Duration::days(1);
        f.run_at(&[(SOURCE_CLAUDE_CODE, &path, PROJECT)], &[], later);
        assert_eq!(rows(&f.list(&[], &[])), Vec::<serde_json::Value>::new());
        assert_eq!(f.stored_labels(), 0);
    }

    #[test]
    fn clear_removes_every_label() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[(SOURCE_CLAUDE_CODE, &path, PROJECT)]);
        let reference = only(&f.list(&[], &[]))["session_ref"]
            .as_str()
            .unwrap()
            .to_string();
        f.label(
            serde_json::json!({"session_ref": reference, "task": "tests"}),
            &[],
        )
        .unwrap();
        assert!(f.dir.path().join(SESSION_LABELS_FILE).exists());
        f.pass.clear().unwrap();
        assert!(!f.dir.path().join(COUNTER_ROWS_FILE).exists());
        assert!(!f.dir.path().join(SESSION_LABELS_FILE).exists());
        // A label after the clear names nothing, and makes no key.
        assert_eq!(
            f.label(
                serde_json::json!({"session_ref": reference, "task": "tests"}),
                &[]
            ),
            Err(ERR_SESSION_UNKNOWN)
        );
        assert!(f.pass.key_is_absent_for_test());
        f.run(&[(SOURCE_CLAUDE_CODE, &path, PROJECT)]);
        assert!(only(&f.list(&[], &[]))["user_task"].is_null());
    }

    #[test]
    fn off_writes_nothing_and_an_automated_row_takes_no_label() {
        let f = Fixture::new();
        let (reference, change) =
            label_params(&serde_json::json!({"session_ref": "t0", "task": "docs"})).unwrap();
        assert_eq!(
            f.pass
                .label_value(false, &reference, change, &NeverFolders::NONE, &[]),
            Ok(serde_json::json!({"enabled": false, "feed": "counter_pass"}))
        );
        assert!(f.pass.key_is_absent_for_test(), "no key made");
        assert!(!f.dir.path().join(COUNTER_ROWS_FILE).exists());

        let automated = String::from_utf8(codex_bytes()).unwrap().replace(
            r#""model_provider":"openai""#,
            r#""model_provider":"openai","source":{"subagent":"review"}"#,
        );
        let path = f.write("auto.jsonl", automated.as_bytes());
        f.run(&[(SOURCE_CODEX, &path, PROJECT)]);
        assert_eq!(
            f.list(&[], &[])["sessions_stored"],
            0,
            "automated, not listed"
        );
        assert_eq!(
            f.label(
                serde_json::json!({"session_ref": "t0", "task": "docs"}),
                &[]
            ),
            Err(ERR_SESSION_UNKNOWN)
        );
    }

    /// Back-compat: the store is written as before labels, which live in
    /// their own file, and lists every row as before, plus the new keys.
    #[test]
    fn a_store_from_before_labels_reads_and_is_written_as_before() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[(SOURCE_CLAUDE_CODE, &path, PROJECT)]);
        let text = std::fs::read_to_string(f.dir.path().join(COUNTER_ROWS_FILE)).unwrap();
        assert!(!text.contains("\"labels\""), "{text}");
        assert!(!f.dir.path().join(SESSION_LABELS_FILE).exists());
        let row = only(&f.list(&[], &[]));
        let keys: std::collections::BTreeSet<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for added in ["inferred_task", "user_task", "user_outcome"] {
            assert!(keys.contains(added), "{added}");
        }
        // A label moves no key a row carried before.
        let reference = row["session_ref"].as_str().unwrap().to_string();
        let labelled = f
            .label(
                serde_json::json!({"session_ref": reference, "task": "other"}),
                &[],
            )
            .unwrap()["session"]
            .clone();
        for key in [
            "session_ref",
            "source",
            "started_at",
            "turns",
            "tokens",
            "sparkline",
            "inferred_task",
        ] {
            assert_eq!(labelled[key], row[key], "{key}");
        }
        assert_eq!(labelled["user_task"], "other");
        let text = std::fs::read_to_string(f.dir.path().join(COUNTER_ROWS_FILE)).unwrap();
        assert!(
            !text.contains("\"labels\""),
            "the store is not written: {text}"
        );
    }

    fn labelled_once(f: &Fixture, path: &Path, params: serde_json::Value) -> String {
        f.run(&[(SOURCE_CLAUDE_CODE, path, PROJECT)]);
        let reference = only(&f.list(&[], &[]))["session_ref"]
            .as_str()
            .unwrap()
            .to_string();
        let mut params = params;
        params["session_ref"] = reference.clone().into();
        f.label(params, &[]).unwrap();
        reference
    }

    /// The store is begun again whenever it cannot be read, a store a
    /// later build wrote included. The labels are not in it: while its rows
    /// come back a poll's budget at a time, a label waits for its row.
    #[test]
    fn a_label_survives_the_store_being_begun_again() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        labelled_once(
            &f,
            &path,
            serde_json::json!({"task": "tests", "outcome": "kept"}),
        );
        // A field this build does not know: the store is unreadable.
        let rows_file = f.dir.path().join(COUNTER_ROWS_FILE);
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&rows_file).unwrap()).unwrap();
        value["a_later_field"] = serde_json::json!(1);
        std::fs::write(&rows_file, value.to_string()).unwrap();
        let f = Fixture {
            pass: CounterPass::with_keys(
                rows_file,
                Box::new(InMemoryDigestKeyStore::with_seed([f.seed; 32])),
            ),
            ..f
        };
        f.pass.keys_load_or_create_for_test();
        assert_eq!(f.list(&[], &[])["readable"], false);
        // Begun again with a poll's worth of other sessions read first: the
        // labelled one waits for the next poll, and so does its label.
        let others: Vec<PathBuf> = (0
            ..crate::insights::analytics_constants::COUNTER_PASS_MAX_READS_PER_TICK)
            .map(|i| f.write(&format!("o{i}.jsonl"), &codex_bytes()))
            .collect();
        let mut files: Vec<(&'static str, &Path, &str)> = others
            .iter()
            .map(|other| (SOURCE_CODEX, other.as_path(), OTHER))
            .collect();
        files.push((SOURCE_CLAUDE_CODE, &path, PROJECT));
        f.run(&files);
        assert!(
            rows(&f.list(&[], &[]))
                .iter()
                .all(|row| row["source"] == "codex"),
            "not read yet"
        );
        assert_eq!(f.stored_labels(), 1);
        f.run(&files);
        let row = rows(&f.list(&[], &[]))
            .into_iter()
            .find(|row| row["source"] == "claude_code")
            .unwrap();
        assert_eq!(row["user_task"], "tests");
        assert_eq!(row["user_outcome"], "kept");
        assert_eq!(f.stored_labels(), 1);
    }

    /// A store begun again numbers its rows from `t0` again, so a reference
    /// answered before it would name another session. It carries the
    /// store's tag, and is refused: no label or drill lands on the session
    /// that now has its order.
    #[test]
    fn a_reference_from_before_the_store_was_begun_again_is_refused() {
        let f = Fixture::new();
        let first = f.write("first.jsonl", &claude_bytes());
        let second = f.write("second.jsonl", &codex_bytes());
        f.run(&[(SOURCE_CLAUDE_CODE, &first, PROJECT)]);
        let stale = only(&f.list(&[], &[]))["session_ref"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(stale.starts_with("t0."), "{stale}");
        // Unreadable: begun again, and another session is read first.
        std::fs::write(f.dir.path().join(COUNTER_ROWS_FILE), b"{not json").unwrap();
        let f = Fixture {
            pass: CounterPass::with_keys(
                f.dir.path().join(COUNTER_ROWS_FILE),
                Box::new(InMemoryDigestKeyStore::with_seed([f.seed; 32])),
            ),
            ..f
        };
        f.pass.keys_load_or_create_for_test();
        f.run(&[(SOURCE_CODEX, &second, OTHER)]);
        let now_first = only(&f.list(&[], &[]));
        assert_eq!(now_first["source"], "codex");
        let fresh = now_first["session_ref"].as_str().unwrap();
        assert!(fresh.starts_with("t0."), "the order begins again: {fresh}");
        assert_ne!(fresh, stale);
        assert_eq!(
            f.label(
                serde_json::json!({"session_ref": stale, "task": "docs"}),
                &[]
            ),
            Err(ERR_SESSION_UNKNOWN)
        );
        assert_eq!(f.stored_labels(), 0);
        assert_eq!(f.drill(&stale, &[]).unwrap_err(), ERR_SESSION_UNKNOWN);
        // Nor does the short name alone name a row.
        assert_eq!(
            f.label(
                serde_json::json!({"session_ref": "t0", "task": "docs"}),
                &[]
            ),
            Err(ERR_SESSION_UNKNOWN)
        );
        // The reference listed now does.
        assert_eq!(
            f.label(
                serde_json::json!({"session_ref": fresh, "task": "docs"}),
                &[]
            )
            .unwrap()["session"]["user_task"],
            "docs"
        );
    }

    /// A label written by another build: an unknown field, a task word
    /// this build does not know and another schema name are read past,
    /// answered as unset, and kept when the person changes the other label.
    #[test]
    fn a_label_file_from_another_build_is_read_and_kept() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        let reference = labelled_once(&f, &path, serde_json::json!({"task": "docs"}));
        let labels = f.dir.path().join(SESSION_LABELS_FILE);
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&labels).unwrap()).unwrap();
        value["schema"] = "trace_commons.insights_session_labels.v9".into();
        value["a_later_field"] = serde_json::json!({"kept": true});
        let session = value["labels"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        value["labels"][&session] = serde_json::json!({"task": "migration", "confidence": 3});
        std::fs::write(&labels, value.to_string()).unwrap();
        let f = Fixture {
            pass: CounterPass::with_keys(
                f.dir.path().join(COUNTER_ROWS_FILE),
                Box::new(InMemoryDigestKeyStore::with_seed([f.seed; 32])),
            ),
            ..f
        };
        f.pass.keys_load_or_create_for_test();
        let row = only(&f.list(&[], &[]));
        assert_eq!(f.list(&[], &[])["readable"], true);
        assert!(
            row["user_task"].is_null(),
            "an unknown word is unset: {row}"
        );
        let row = f
            .label(
                serde_json::json!({"session_ref": reference, "outcome": "kept"}),
                &[],
            )
            .unwrap()["session"]
            .clone();
        assert_eq!(row["user_outcome"], "kept");
        assert!(row["user_task"].is_null());
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&labels).unwrap()).unwrap();
        assert_eq!(
            written["labels"][&session],
            serde_json::json!({"task": "migration", "confidence": 3, "outcome": "kept"})
        );
        assert_eq!(written["a_later_field"], serde_json::json!({"kept": true}));
        // A file not in the labels shape at all is never written over.
        std::fs::write(&labels, b"[1, 2]").unwrap();
        let f = Fixture {
            pass: CounterPass::with_keys(
                f.dir.path().join(COUNTER_ROWS_FILE),
                Box::new(InMemoryDigestKeyStore::with_seed([f.seed; 32])),
            ),
            ..f
        };
        f.pass.keys_load_or_create_for_test();
        assert!(only(&f.list(&[], &[]))["user_outcome"].is_null());
        assert_eq!(
            f.label(
                serde_json::json!({"session_ref": reference, "task": "docs"}),
                &[]
            ),
            Err(ERR_SESSION_LABEL_WRITE_FAILED)
        );
        assert_eq!(std::fs::read(&labels).unwrap(), b"[1, 2]");
    }

    /// A labels file that fails to read (here, unreadable by its owner) is
    /// read again on the next call, not answered as no labels until a
    /// restart.
    #[cfg(unix)]
    #[test]
    fn a_labels_file_that_failed_to_read_is_read_again() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        labelled_once(&f, &path, serde_json::json!({"task": "docs"}));
        let labels = f.dir.path().join(SESSION_LABELS_FILE);
        let f = Fixture {
            pass: CounterPass::with_keys(
                f.dir.path().join(COUNTER_ROWS_FILE),
                Box::new(InMemoryDigestKeyStore::with_seed([f.seed; 32])),
            ),
            ..f
        };
        f.pass.keys_load_or_create_for_test();
        std::fs::set_permissions(&labels, std::fs::Permissions::from_mode(0o000)).unwrap();
        let unreadable = only(&f.list(&[], &[]));
        std::fs::set_permissions(&labels, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(unreadable["user_task"].is_null(), "{unreadable}");
        assert_eq!(only(&f.list(&[], &[]))["user_task"], "docs");
    }

    /// A label that could not be written is not answered, listed or kept
    /// for a later write.
    #[cfg(unix)]
    #[test]
    fn a_failed_label_write_keeps_the_label_as_it_was() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        let reference = labelled_once(&f, &path, serde_json::json!({"task": "docs"}));
        let dir = f.dir.path();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let refused = f.label(
            serde_json::json!({"session_ref": reference, "task": "tests", "outcome": "kept"}),
            &[],
        );
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(refused, Err(ERR_SESSION_LABEL_WRITE_FAILED));
        let row = only(&f.list(&[], &[]));
        assert_eq!(row["user_task"], "docs");
        assert!(row["user_outcome"].is_null());
        // The next write writes only its own change.
        let row = f
            .label(
                serde_json::json!({"session_ref": reference, "outcome": "partly_kept"}),
                &[],
            )
            .unwrap()["session"]
            .clone();
        assert_eq!(row["user_task"], "docs");
        assert_eq!(row["user_outcome"], "partly_kept");
    }

    #[test]
    fn the_label_method_is_advertised_dispatched_gated_and_local() {
        let method = "insights_session_label";
        assert!(super::super::ipc::METHODS.contains(&method));
        assert!(super::super::ipc::DEV_DRY_RUN_LOCAL_METHODS.contains(&method));
        let (_dir, mut s) = shared();
        s.dev_dry_run = true;
        s.settings.lock().unwrap().insights_counter_pass = false;
        let params = serde_json::json!({"session_ref": "t0", "task": "docs"});
        let r = super::super::ipc::handle_request(&s, &call(method, params.clone()));
        assert_eq!(
            r.result.unwrap(),
            serde_json::json!({"enabled": false, "feed": "counter_pass"})
        );
        // Params are checked before the gate.
        let r = super::super::ipc::handle_request(
            &s,
            &call(
                method,
                serde_json::json!({"session_ref": "t0", "task": "no_edits"}),
            ),
        );
        let error = r.error.unwrap();
        assert_eq!(error.code, ERR_BAD_PARAMS);
        assert_eq!(error.message, ERR_SESSION_LABEL_INVALID);
        // On, with no row by that reference.
        s.settings.lock().unwrap().insights_counter_pass = true;
        let r = super::super::ipc::handle_request(&s, &call(method, params));
        let error = r.error.unwrap();
        assert_eq!(error.code, ERR_BAD_PARAMS);
        assert_eq!(error.message, ERR_SESSION_UNKNOWN);
    }
}
