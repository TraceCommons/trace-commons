//! `insights_sessions` and `insights_session_drill`: the Sessions tab over
//! the counter pass (feed T).
//!
//! Gated exactly as `insights_week` is (owner decision D4, open): off, both
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
//! opaque reference (`t` and the order the pass made the row in: never a
//! digest or a path), its harness, its first recorded event's time, its
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

use chrono::{DateTime, FixedOffset, Offset, Utc};

use super::insights_week::{
    FEED_COUNTER_PASS, NeverFolders, UNREADABLE_POLICY, never_project_keys,
};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, Request, Response};
use super::policy::{UNKNOWN_PROJECT_KEY, disambiguated_label, display_path_for_key, known_keys};

/// The most sessions one `insights_sessions` page lists, newest first.
/// `truncated` and `next` say when more follow.
pub const WATCHED_SESSIONS_MAX: usize = 200;
/// `session_ref` missing or not a string.
pub const ERR_SESSION_REF_INVALID: &str = "insights-session-ref-invalid";
/// No listed session has that reference: never stored, dropped since, made
/// again under a new reference when its file grew, or in a Never folder.
pub const ERR_SESSION_UNKNOWN: &str = "insights-session-unknown";

/// `before` is not a cursor an earlier page gave.
pub const ERR_SESSIONS_CURSOR_INVALID: &str = "insights-sessions-cursor-invalid";
/// `limit` is not a whole number of at least 1.
pub const ERR_SESSIONS_LIMIT_INVALID: &str = "insights-sessions-limit-invalid";

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
fn gate(shared: &DaemonShared) -> Result<(bool, NeverFolders), serde_json::Value> {
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
        COUNTER_ROWS_FILE, CounterCandidate, CounterPass, UNREADABLE_STORE,
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
            assert!(reference.starts_with('t'), "{reference}");
            assert!(
                reference[1..].chars().all(|c| c.is_ascii_digit()),
                "{reference}"
            );
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
                    "folder",
                    "session_ref",
                    "source",
                    "sparkline",
                    "started_at",
                    "tokens",
                    "turns"
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
        assert_eq!(drill["tokens"], 120 + 110);
        assert_eq!(drill["state"], "known");
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
}
