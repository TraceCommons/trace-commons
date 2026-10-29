//! `insights_analyze_session`: Analyze one watched session, by the opaque
//! reference the Sessions feed lists it under (Insights PM, 2026-10-21:
//! terminal users never see their session files, so Analyze no longer asks
//! for one).
//!
//! Gated exactly as `insights_sessions` is: off, it answers `enabled: false`
//! and reads nothing; a store, key or folder-rule read that fails answers
//! `readable: false` with its fixed reason. A session in a folder that is
//! Never is not analyzed, by the row's stored folder or by its folder's rule
//! now, the contribution override included, and answers as unknown.
//!
//! # What is read
//!
//! The caller names a row, never a file or a store. The daemon lists the
//! Claude Code and Codex sessions its watcher's own sources find, the same
//! discovery a watcher poll runs, and matches the row to one of them by the
//! keyed session digest the counter pass stored it under
//! ([`CounterPass::locate`](super::insights_week::CounterPass::locate)). Only
//! that file is read, and only by the same analysis the app's in-process
//! `analyze` runs on it (`insights::service`), so the answer is that
//! operation's own `analyze` response. With `save` the result is stored as a
//! dated snapshot in the standard Insights store, the one the app reads when
//! no `--insights-store` is given; no other directory can be named. Nothing
//! reaches the network. So a dry-run daemon refuses the method: a save
//! writes the person's real store, outside the daemon's state directory.
//!
//! A file that analysis refuses whole, so every attempt would fail the same
//! way, is answered with why and not read: one over 16 MiB, or a Codex file
//! that does not record exactly one session. The pass notes both when it
//! reads the file, and the Sessions row says `analyzable: false`.
//!
//! # What crosses the socket
//!
//! The core's `analyze` response, as the in-process call returns it: the
//! snapshot (its derived observations and the source's SHA-256 digest, as
//! every saved snapshot carries) and the episodes a save invalidated; a
//! saved snapshot also carries the keyed digests a saved import does (its
//! session identity and, for Claude Code, its per-turn series), under the
//! Insights store's own digest key. Never the file's path.

use super::insights_sessions::{ERR_SESSION_REF_INVALID, ERR_SESSION_UNKNOWN, gate};
use super::insights_week::{FEED_COUNTER_PASS, LocateError, NotAnalyzable};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use super::policy::ProjectMode;
use crate::insights::service::{LocalInsightsOperation, LocalInsightsRequest, dispatch_json};
use crate::source::{SOURCE_CLAUDE_CODE, SOURCE_CODEX, SessionRef, TraceSource, all_sources};

/// `save` is present and not `true` or `false`.
pub const ERR_ANALYZE_SAVE_INVALID: &str = "insights-analyze-save-invalid";
/// The row is listed, but its file is no longer where the watcher finds
/// sessions: moved or deleted since the pass read it.
pub const ERR_SESSION_FILE_GONE: &str = "insights-session-file-gone";
/// The session's file is larger than the analysis reads
/// (`insights::MAX_SOURCE_BYTES`, 16 MiB): it is counted, never analyzed.
pub const ERR_SESSION_TOO_LARGE: &str = "insights-session-too-large";
/// A Codex session file that does not record exactly one session (one
/// `session_meta`), which the analysis refuses.
pub const ERR_SESSION_NOT_ONE_SESSION: &str = "insights-session-not-one-session";

/// A session only the counter pass reads (a compressed Codex rollout, or
/// one in a NEAR AI managed profile), which Analyze & save does not find
/// among the watched sessions.
pub const ERR_SESSION_INSIGHTS_ONLY: &str = "insights-session-insights-only";

/// The answer for a file the analysis refuses, by why.
fn not_analyzable(reason: NotAnalyzable) -> &'static str {
    match reason {
        NotAnalyzable::TooLarge => ERR_SESSION_TOO_LARGE,
        NotAnalyzable::NotOneSession => ERR_SESSION_NOT_ONE_SESSION,
        NotAnalyzable::InsightsOnly => ERR_SESSION_INSIGHTS_ONLY,
    }
}

/// `session_ref` (required) and `save` (optional, `false` when absent).
fn params(params: &serde_json::Value) -> Result<(String, bool), &'static str> {
    let reference = params
        .get("session_ref")
        .and_then(|value| value.as_str())
        .ok_or(ERR_SESSION_REF_INVALID)?;
    let save = match params.get("save") {
        None | Some(serde_json::Value::Null) => false,
        Some(value) => value.as_bool().ok_or(ERR_ANALYZE_SAVE_INVALID)?,
    };
    Ok((reference.to_string(), save))
}

/// The Claude Code and Codex sessions the watcher's sources find now, each
/// with the source that found it. A source whose discovery fails lists
/// none, as it lists none to a poll.
fn watched_sessions(
    shared: &DaemonShared,
) -> (Vec<Box<dyn TraceSource>>, Vec<(usize, SessionRef)>) {
    let sources = all_sources(&shared.source_roots_with_routing());
    let mut found = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        if !matches!(source.name(), SOURCE_CLAUDE_CODE | SOURCE_CODEX) {
            continue;
        }
        if let Ok(refs) = source.discover() {
            found.extend(refs.into_iter().map(|session_ref| (index, session_ref)));
        }
    }
    (sources, found)
}

/// `insights_analyze_session`.
pub fn handle_analyze_session(shared: &DaemonShared, req: &Request) -> Response {
    let (reference, save) = match params(&req.params) {
        Ok(found) => found,
        Err(label) => return Response::err(req.id, ERR_BAD_PARAMS, label),
    };
    let (enabled, never) = match gate(shared) {
        Ok(gate) => gate,
        Err(answer) => return Response::ok(req.id, answer),
    };
    if !enabled {
        // Off: nothing is listed, matched or read.
        return Response::ok(
            req.id,
            serde_json::json!({"enabled": false, "feed": FEED_COUNTER_PASS}),
        );
    }
    let (sources, found) = watched_sessions(shared);
    let paths: Vec<(&str, &std::path::Path)> = found
        .iter()
        .map(|(_, session_ref)| (session_ref.source, session_ref.path.as_path()))
        .collect();
    let located = match shared.insights_counter.locate(&reference, &never, &paths) {
        Ok(Some(located)) => located,
        Ok(None) => return Response::err(req.id, ERR_UNAVAILABLE, ERR_SESSION_FILE_GONE),
        Err(LocateError::Unreadable(answer)) => return Response::ok(req.id, answer),
        Err(LocateError::Unknown) => {
            return Response::err(req.id, ERR_BAD_PARAMS, ERR_SESSION_UNKNOWN);
        }
        // Refused as the pass found the file: no read would go otherwise.
        Err(LocateError::NotAnalyzable(reason)) => {
            return Response::err(req.id, ERR_UNAVAILABLE, not_analyzable(reason));
        }
    };
    let (source_index, session_ref) = &found[located.index];
    // The folder's rule now, as the pass resolves it: a folder made Never
    // since the row was stored is not read, though its row has not yet
    // been dropped.
    let project_key =
        super::watcher::project_key_of(shared, sources[*source_index].as_ref(), session_ref);
    let never_now = shared
        .policy
        .lock()
        .map(|policy| policy.resolve(&project_key) == ProjectMode::Ignore)
        // A poisoned lock reads as Never: fail closed.
        .unwrap_or(true);
    if never_now {
        return Response::err(req.id, ERR_BAD_PARAMS, ERR_SESSION_UNKNOWN);
    }
    // Discovery listed it a moment ago; it may have gone since, or grown
    // past what the analysis reads since the pass read it.
    match std::fs::metadata(&session_ref.path) {
        Ok(meta) if meta.is_file() => {
            if meta.len() > crate::insights::MAX_SOURCE_BYTES {
                return Response::err(req.id, ERR_UNAVAILABLE, ERR_SESSION_TOO_LARGE);
            }
        }
        _ => return Response::err(req.id, ERR_UNAVAILABLE, ERR_SESSION_FILE_GONE),
    }
    let request = LocalInsightsRequest {
        // The standard store, the app's own: never a directory a caller
        // names.
        store_dir: store_dir(shared),
        operation: LocalInsightsOperation::Analyze {
            source: located.format,
            file: session_ref.path.clone(),
            save,
        },
    };
    let analysis = serde_json::to_vec(&request)
        .map_err(|_| anyhow::anyhow!("insights-request-invalid"))
        .and_then(|bytes| dispatch_json(&bytes))
        .and_then(|json| {
            serde_json::from_str::<serde_json::Value>(&json)
                .map_err(|_| anyhow::anyhow!("insights-response-invalid"))
        });
    match analysis {
        Ok(analysis) => Response::ok(
            req.id,
            serde_json::json!({
                "enabled": true,
                "feed": FEED_COUNTER_PASS,
                "readable": true,
                "analysis": analysis,
            }),
        ),
        // Gone while it was read.
        Err(_) if !session_ref.path.is_file() => {
            Response::err(req.id, ERR_UNAVAILABLE, ERR_SESSION_FILE_GONE)
        }
        // `dispatch_json` gives fixed labels only, never a path.
        Err(label) => Response::err(req.id, ERR_UNAVAILABLE, &label.to_string()),
    }
}

/// The store a save goes to: the standard one (`None`). In a test build,
/// always a directory inside the daemon's own state directory, which every
/// test makes in a temporary directory: no test, on any thread, can write to
/// the developer's real store.
#[cfg_attr(not(test), allow(unused_variables))]
fn store_dir(shared: &DaemonShared) -> Option<std::path::PathBuf> {
    #[cfg(test)]
    return Some(shared.store.daemon_path(TEST_STORE_DIR));
    #[cfg(not(test))]
    None
}

/// The Insights store a test build saves to, in the daemon's state
/// directory.
#[cfg(test)]
const TEST_STORE_DIR: &str = "insights-test-store";

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use chrono::{Duration, Utc};
    use serde_json::json;

    use super::*;
    use crate::daemon::insights_week::{CounterCandidate, NeverFolders};
    use crate::daemon::ipc::handle_request;
    use crate::daemon::settings::SourceDeclaration;

    /// A daemon watching one Claude Code session (Codex off) in a temporary
    /// root, with Insights saving to a store in its temporary state
    /// directory.
    struct Fixture {
        shared: DaemonShared,
        root: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let (dir, store) = crate::config::tests_support::temp_store();
            std::mem::forget(dir);
            let shared = DaemonShared::load(store).unwrap();
            let root = tempfile::tempdir().unwrap();
            let project = root.path().join("proj");
            std::fs::create_dir_all(&project).unwrap();
            std::fs::write(
                project.join("session.jsonl"),
                std::fs::read(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("fixtures/insights/claude-turn-series/session.jsonl"),
                )
                .unwrap(),
            )
            .unwrap();
            {
                let mut settings = shared.settings.lock().unwrap();
                settings.claude_source = Some(SourceDeclaration::Watch {
                    path: root.path().to_path_buf(),
                });
                settings.codex_source = Some(SourceDeclaration::Off);
                settings.insights_counter_pass = true;
            }
            Self { shared, root }
        }

        /// The one session the watcher finds.
        fn session(&self) -> SessionRef {
            let (_, found) = watched_sessions(&self.shared);
            assert_eq!(found.len(), 1);
            found[0].1.clone()
        }

        /// The session's folder as the pass resolves it now.
        fn folder(&self) -> String {
            let (sources, found) = watched_sessions(&self.shared);
            let (index, session) = &found[0];
            super::super::watcher::project_key_of(&self.shared, sources[*index].as_ref(), session)
        }

        /// One counter pass over the session, storing it under `folder`.
        fn count(&self, folder: &str) {
            let session = self.session();
            let written = Utc::now() - Duration::hours(2);
            let candidate = CounterCandidate {
                origin: 0,
                source: session.source,
                size_bytes: std::fs::metadata(&session.path).unwrap().len(),
                path: session.path.clone(),
                modified_at: written,
                started_at: Some(written),
                group_member_count: 0,
            };
            let summary = self
                .shared
                .insights_counter
                .run_pass(
                    &[candidate],
                    &NeverFolders::NONE,
                    Utc::now(),
                    60,
                    &mut |_| (folder.to_string(), false),
                )
                .unwrap();
            assert_eq!(summary.read, 1);
        }

        fn listed(&self) -> String {
            let value = handle_request(&self.shared, &req("insights_sessions", json!({})))
                .result
                .unwrap();
            value["sessions"][0]["session_ref"]
                .as_str()
                .unwrap()
                .to_string()
        }

        fn analyze(&self, params: serde_json::Value) -> Response {
            handle_request(&self.shared, &req("insights_analyze_session", params))
        }

        fn saved(&self) -> PathBuf {
            self.shared.store.daemon_path(TEST_STORE_DIR)
        }
    }

    fn req(method: &str, params: serde_json::Value) -> Request {
        Request {
            id: 7,
            method: method.to_string(),
            params,
        }
    }

    fn error(response: &Response) -> (String, String) {
        let error = response.error.as_ref().expect("an error");
        (error.code.clone(), error.message.clone())
    }

    #[test]
    fn off_answers_disabled_and_reads_nothing() {
        let f = Fixture::new();
        f.count("/proj");
        let reference = f.listed();
        f.shared.settings.lock().unwrap().insights_counter_pass = false;
        let answer = f
            .analyze(json!({"session_ref": reference, "save": true}))
            .result
            .unwrap();
        assert_eq!(answer, json!({"enabled": false, "feed": "counter_pass"}));
        assert!(!f.saved().exists(), "nothing saved");
    }

    #[test]
    fn analyze_and_save_answers_the_core_analyze_response_and_saves_it() {
        let f = Fixture::new();
        f.count("/proj");
        let reference = f.listed();
        let answer = f
            .analyze(json!({"session_ref": reference, "save": true}))
            .result
            .unwrap();
        assert_eq!(answer["enabled"], true);
        assert_eq!(answer["feed"], "counter_pass");
        assert_eq!(answer["readable"], true);
        let analysis = &answer["analysis"];
        assert_eq!(analysis["type"], "analyze");
        assert_eq!(analysis["insight"]["source_format"], "claude_code");
        let id = analysis["insight"]["id"].as_str().unwrap();
        // Saved in the store the app reads, as the in-process save does.
        let saved = crate::insights::service::list_saved(Some(&f.saved())).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, id);
        // No path crosses: not the root, not the session's file.
        let text = answer.to_string();
        assert!(!text.contains(f.root.path().to_str().unwrap()), "{text}");
        assert!(!text.contains("session.jsonl"), "{text}");
    }

    #[test]
    fn without_save_nothing_is_stored() {
        let f = Fixture::new();
        f.count("/proj");
        let reference = f.listed();
        for params in [
            json!({"session_ref": reference}),
            json!({"session_ref": reference, "save": false}),
        ] {
            let answer = f.analyze(params).result.unwrap();
            assert_eq!(answer["analysis"]["type"], "analyze");
            assert!(answer["analysis"]["insight"]["id"].is_string());
        }
        assert!(!f.saved().exists(), "no store made");
    }

    #[test]
    fn bad_params_are_refused_before_the_gate() {
        let f = Fixture::new();
        f.shared.settings.lock().unwrap().insights_counter_pass = false;
        assert_eq!(
            error(&f.analyze(json!({}))),
            (ERR_BAD_PARAMS.into(), ERR_SESSION_REF_INVALID.into())
        );
        assert_eq!(
            error(&f.analyze(json!({"session_ref": 3}))),
            (ERR_BAD_PARAMS.into(), ERR_SESSION_REF_INVALID.into())
        );
        assert_eq!(
            error(&f.analyze(json!({"session_ref": "t0", "save": "yes"}))),
            (ERR_BAD_PARAMS.into(), ERR_ANALYZE_SAVE_INVALID.into())
        );
    }

    #[test]
    fn an_unknown_or_remade_reference_is_unknown() {
        let f = Fixture::new();
        // Before any pass: no key, no store.
        assert_eq!(
            error(&f.analyze(json!({"session_ref": "t0", "save": true}))),
            (ERR_BAD_PARAMS.into(), ERR_SESSION_UNKNOWN.into())
        );
        f.count("/proj");
        let reference = f.listed();
        assert_eq!(
            error(&f.analyze(json!({"session_ref": "t99", "save": true}))),
            (ERR_BAD_PARAMS.into(), ERR_SESSION_UNKNOWN.into())
        );
        // The file grew: the pass makes the row again under a new reference.
        let session = f.session();
        let mut bytes = std::fs::read(&session.path).unwrap();
        bytes.extend_from_slice(b"\n");
        std::fs::write(&session.path, bytes).unwrap();
        f.count("/proj");
        assert_ne!(f.listed(), reference);
        assert_eq!(
            error(&f.analyze(json!({"session_ref": reference, "save": true}))),
            (ERR_BAD_PARAMS.into(), ERR_SESSION_UNKNOWN.into())
        );
        assert!(!f.saved().exists());
    }

    #[test]
    fn a_file_that_is_gone_says_so() {
        let f = Fixture::new();
        f.count("/proj");
        let reference = f.listed();
        std::fs::remove_file(f.session().path).unwrap();
        assert_eq!(
            error(&f.analyze(json!({"session_ref": reference, "save": true}))),
            (ERR_UNAVAILABLE.into(), ERR_SESSION_FILE_GONE.into())
        );
        assert!(!f.saved().exists());
    }

    /// A Never folder is never read: by the row's stored folder, and by
    /// the folder's rule now, though the row is still stored.
    #[test]
    fn a_never_folder_is_not_analyzed() {
        let f = Fixture::new();
        let folder = f.folder();
        f.count(&folder);
        let reference = f.listed();
        f.shared
            .policy
            .lock()
            .unwrap()
            .set_mode(&folder, ProjectMode::Ignore, Utc::now())
            .unwrap();
        assert_eq!(
            error(&f.analyze(json!({"session_ref": reference, "save": true}))),
            (ERR_BAD_PARAMS.into(), ERR_SESSION_UNKNOWN.into())
        );

        // Stored under another folder: listed, but the rule now says Never.
        let f = Fixture::new();
        let folder = f.folder();
        f.count("/elsewhere");
        let reference = f.listed();
        f.shared
            .policy
            .lock()
            .unwrap()
            .set_mode(&folder, ProjectMode::Ignore, Utc::now())
            .unwrap();
        assert_eq!(
            error(&f.analyze(json!({"session_ref": reference, "save": true}))),
            (ERR_BAD_PARAMS.into(), ERR_SESSION_UNKNOWN.into())
        );
        assert!(!f.saved().exists());
    }

    /// Advertised, and refused in a dry run: a save writes the real
    /// Insights store, outside the daemon's state directory.
    #[test]
    fn the_method_is_advertised_and_refused_in_a_dry_run() {
        assert!(super::super::ipc::METHODS.contains(&"insights_analyze_session"));
        assert!(
            !super::super::ipc::DEV_DRY_RUN_LOCAL_METHODS.contains(&"insights_analyze_session")
        );
    }

    /// A saved test snapshot lands in the fixture's own state directory,
    /// from any thread.
    #[test]
    fn a_save_off_the_fixtures_thread_stays_in_its_state_directory() {
        let f = Fixture::new();
        f.count("/proj");
        let reference = f.listed();
        let answer = std::thread::scope(|scope| {
            scope
                .spawn(|| f.analyze(json!({"session_ref": reference, "save": true})))
                .join()
                .unwrap()
        });
        let id = answer.result.unwrap()["analysis"]["insight"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let saved = crate::insights::service::list_saved(Some(&f.saved())).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, id);
    }

    /// A file the whole-file analysis refuses says why, and is not read:
    /// over its byte bound when the pass counted it, or grown past it since.
    #[test]
    fn a_session_too_large_to_analyze_says_so() {
        let f = Fixture::new();
        f.count("/proj");
        let reference = f.listed();
        let session = f.session();
        let mut bytes = std::fs::read(&session.path).unwrap();
        bytes.extend(std::iter::repeat_n(b' ', 17 * 1024 * 1024));
        std::fs::write(&session.path, &bytes).unwrap();
        // Grown since it was counted: the row is as it was.
        assert_eq!(
            error(&f.analyze(json!({"session_ref": reference, "save": true}))),
            (ERR_UNAVAILABLE.into(), ERR_SESSION_TOO_LARGE.into())
        );
        // Counted as it is now: the row says so, and so does the answer.
        f.count("/proj");
        let listed = handle_request(&f.shared, &req("insights_sessions", json!({})))
            .result
            .unwrap();
        assert_eq!(listed["sessions"][0]["analyzable"], false);
        assert_eq!(
            error(&f.analyze(json!({"session_ref": f.listed(), "save": true}))),
            (ERR_UNAVAILABLE.into(), ERR_SESSION_TOO_LARGE.into())
        );
        assert!(!f.saved().exists());
    }
}
