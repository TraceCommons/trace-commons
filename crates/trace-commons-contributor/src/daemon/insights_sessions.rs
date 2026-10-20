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
//! `insights_sessions` lists the stored sessions newest first, bounded by
//! [`WATCHED_SESSIONS_MAX`]. Each row carries an opaque reference (`t` and
//! the order the pass made the row in: never a digest or a path), its
//! harness, its first recorded event's time, its turn count (Claude Code
//! only) and its folder's display label (owner decision 2026-10-20).
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

use chrono::{FixedOffset, Offset};

use super::insights_week::{
    FEED_COUNTER_PASS, NeverFolders, UNREADABLE_POLICY, never_project_keys,
};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, Request, Response};
use super::policy::{UNKNOWN_PROJECT_KEY, disambiguated_label, display_path_for_key, known_keys};

/// The most sessions one `insights_sessions` answer lists, newest first.
/// `truncated` says when more are stored.
pub const WATCHED_SESSIONS_MAX: usize = 200;
/// `session_ref` missing or not a string.
pub const ERR_SESSION_REF_INVALID: &str = "insights-session-ref-invalid";
/// No listed session has that reference: never stored, dropped since, made
/// again under a new reference when its file grew, or in a Never folder.
pub const ERR_SESSION_UNKNOWN: &str = "insights-session-unknown";

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
    Response::ok(
        req.id,
        shared
            .insights_counter
            .sessions_value(enabled, &never, &folders, WATCHED_SESSIONS_MAX),
    )
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
            serde_json::json!({"type": "session_meta", "timestamp": "2026-09-14T08:29:00Z", "payload": {"id": "PRIVATE-CODEX-ID", "model_provider": "openai"}}),
            serde_json::json!({"type": "turn_context", "timestamp": "2026-09-14T08:29:30Z", "payload": {"model": "gpt-6"}}),
            usage("2026-09-14T08:30:00Z", 100, 60, 20),
            serde_json::json!({"type": "response_item", "timestamp": "2026-09-14T08:30:30Z", "payload": {"type": "message", "role": "assistant", "content": []}}),
            usage("2026-09-14T08:31:00Z", 180, 100, 50),
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
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let pass = CounterPass::with_keys(
                dir.path().join(COUNTER_ROWS_FILE),
                Box::new(InMemoryDigestKeyStore::with_seed([3; 32])),
            );
            Self { dir, pass }
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
        // Claude Code rows carry their turn count; Codex records none.
        assert!(listed[0]["turns"].as_u64().unwrap() > 0);
        assert_eq!(listed[0]["turns"], listed[1]["turns"]);
        assert_eq!(listed[2]["turns"], serde_json::Value::Null);
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
                ["folder", "session_ref", "source", "started_at", "turns"].into(),
                "only these fields cross"
            );
        }
        assert_nothing_identifying(&f, &value);
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
    fn a_codex_drill_says_per_turn_usage_is_not_recorded() {
        let f = Fixture::new();
        let path = f.write("codex.jsonl", &codex_bytes());
        f.run(&[(SOURCE_CODEX, &path, PROJECT)]);
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
