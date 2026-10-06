//! Matching contribution missions on this Mac (K16, #1173).
//!
//! The rules are the consent design's "Missions" section:
//!
//! - **M1. Matching stays on this Mac.** It reads only sessions from
//!   adapters the contributor has left on, in folders not set to Never, and
//!   under a Never contribution override it reads nothing. What it reads,
//!   and what it finds, go into no network request, audit row or log line.
//! - **M2. A mission sends nothing by itself.** Matching takes the policy
//!   by shared reference and the queue only to read, never mutably: it
//!   cannot arm, approve or widen anything. A session counts toward a
//!   mission only when it is contributed through one of the three consent
//!   paths; a kept session explicitly declined that, and a withdrawn one
//!   reversed it, so [`readable_sessions`] excludes both (#1227 review).
//!   An already-uploaded session, and one still being written
//!   (not-yet-quiescent), stay candidates -- neither has been kept or
//!   withdrawn, and quiescence is a fact about *when* a session is ready to
//!   read, not about whether it was ever offered.
//!
//! [`readable_sessions`] is where M1's and M2's read rules live, and the
//! only place: everything after it, including
//! [`crate::contribution_missions::match_missions`], sees only what it let
//! through. The handler ([`handle_mission_matches`]) applies no gate of its
//! own before calling it -- a pre-filter there would make the rule live in
//! two places, and removing one silently would stop being caught by a test
//! that only exercises the other (#1227 review, finding 1).
//!
//! # Nits carried over from review (#1227)
//!
//! - **The adapter, not the self-declared tool, is what gates reading.**
//!   [`SeenSession::adapter`] is `SessionRef::source` -- the adapter that
//!   actually read the bytes -- recorded in [`super::state::CwdCacheEntry`]
//!   when the cache entry is written. `SeenSession::tool` is the
//!   contributor-facing display name (`SessionRef::displayed_source`),
//!   self-declared for a staged import and never gated on here, only used
//!   to label a readable session once it has already passed the adapter
//!   gate. A cache entry written before the adapter field existed carries
//!   `adapter: None` and is not read: fails closed, same as a missing
//!   `tool`, rather than trusting an un-recorded adapter.
//! - **The unknown-project bucket is read.** A session whose folder key is
//!   [`UNKNOWN_PROJECT_KEY`] resolves to `NotifyOnly`
//!   (`ProjectPolicy::resolve`), not `Ignore`, so it is read and counted in
//!   `read.folders` like any other folder not set to Never. It is kept out
//!   of the language probe (`local_facts` never asks `languages_of` about
//!   it), because it has no folder root to look at.
//! - **An unset `claude-code` or `codex` declaration counts as on.**
//!   `SourceRoots::source_identities` gives both a conventional root when
//!   the contributor has never been asked (`Undeclared::Conventional`),
//!   matching what the watcher reads from by default. Gemini, Cline and
//!   OpenCode are `Undeclared::Nothing` and count as off until declared.
//! - **The case-folded language-root fallback fails closed.** When a
//!   folder has no `display_path` recorded in the policy (never armed, or
//!   discovered only through the cwd cache), [`handle_mission_matches`]
//!   falls back to the project key itself, which is case-folded on macOS
//!   and Windows. On a case-sensitive volume that fallback can miss a
//!   marker file that is really there under its original case, so
//!   `languages_at` under-reports rather than over-reports -- the safe
//!   direction for a signal that can only add a mission, never remove one.
//!
//! Every known adapter name is registered in `source::NATIVE_SOURCES`
//! verbatim as `SessionRef::source`, so gating on the adapter needs no
//! alias: unlike the old `tool` gate, there is no case here where an
//! imported Antigravity conversation's adapter reads as anything other than
//! `trajectory`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::contribution_missions::{
    ContributionMissionCatalogue, LANGUAGE_MARKERS, LocalFacts, SessionFact, match_missions,
};

use super::history::{HistoryCache, is_taken_back};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, Request, Response};
use super::policy::{ProjectMode, ProjectPolicy, UNKNOWN_PROJECT_KEY};

/// The one log line matching writes: a label, nothing about what was read
/// or found (M1).
pub const LOG_LABEL: &str = "mission-matches-answered";

/// A session the daemon has seen, as its cwd cache records it: the
/// self-declared tool (display only), the adapter that actually read it,
/// the folder's project key, and the cache's own path key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenSession {
    pub tool: Option<String>,
    pub adapter: Option<String>,
    pub folder: String,
    /// The cwd cache's own key: the session file's path. Used only to look
    /// up whether this session is kept or withdrawn (M2) -- never put into
    /// a [`SessionFact`], logged, or sent.
    pub path: String,
}

/// Whether sessions from `adapter` may be read: it is one the contributor
/// has left on. `adapter` is `SessionRef::source`, the adapter that
/// actually discovered the session -- never the self-declared display name
/// (`SessionRef::displayed_source`), which a staged import can spoof
/// (#1227 review, finding 2).
pub fn adapter_is_on(adapter: &str, tools_on: &BTreeSet<String>) -> bool {
    tools_on.contains(adapter)
}

/// M1's and M2's read rule: the sessions matching may read, out of
/// everything seen. The only place either rule lives (#1227 review, finding
/// 1): the handler applies no gate of its own before calling this.
///
/// - A session whose adapter is switched off is not read, nor one with no
///   recorded adapter -- a cache entry written before that field existed --
///   which cannot be shown to be on. That is fail-closed, not a default
///   grant (M1).
/// - A session in a folder whose mode in force is Never is not read. That
///   is `resolve`, not the folder's own mode, so a Never contribution
///   override (#1208) makes every folder Never and nothing is read; and a
///   folder set to Never stays Never under every other override (M1).
/// - A session that is kept, or whose upload was withdrawn, is not read:
///   a mission counts a session only when it is contributed through one of
///   the three consent paths, and a keep explicitly declined that while a
///   withdrawal reversed it (M2). An already-uploaded session that is
///   neither kept nor withdrawn, and one still being written
///   (not-yet-quiescent), are excluded from neither set and stay readable.
/// - Folders that are not armed are read; nothing read leaves the device.
///
/// `policy` is borrowed, never mutably: matching changes nothing (M2).
pub fn readable_sessions(
    policy: &ProjectPolicy,
    tools_on: &BTreeSet<String>,
    excluded_paths: &BTreeSet<String>,
    seen: &[SeenSession],
) -> Vec<SessionFact> {
    seen.iter()
        .filter_map(|s| {
            let tool = s.tool.as_deref()?;
            let adapter = s.adapter.as_deref()?;
            (adapter_is_on(adapter, tools_on)
                && policy.resolve(&s.folder) != ProjectMode::Ignore
                && !excluded_paths.contains(&s.path))
            .then(|| SessionFact {
                tool: tool.to_string(),
                folder: s.folder.clone(),
            })
        })
        .collect()
}

/// The facts the matcher gets: the readable sessions, and, only when a
/// mission asks about languages, the languages of the folders they ran in.
/// `languages_of` is asked about readable folders only, and never about the
/// unknown bucket, which has no folder to look in.
pub fn local_facts(
    sessions: Vec<SessionFact>,
    needs_languages: bool,
    mut languages_of: impl FnMut(&str) -> BTreeSet<String>,
) -> LocalFacts {
    let mut folder_languages = BTreeMap::new();
    if needs_languages {
        let folders: BTreeSet<&str> = sessions
            .iter()
            .map(|s| s.folder.as_str())
            .filter(|f| *f != UNKNOWN_PROJECT_KEY)
            .collect();
        for folder in folders {
            folder_languages.insert(folder.to_string(), languages_of(folder));
        }
    }
    LocalFacts {
        sessions,
        folder_languages,
    }
}

/// The languages a folder is worked in, from which of
/// [`LANGUAGE_MARKERS`] sit at its root. Existence only: no file is opened,
/// and a link is not followed.
pub fn languages_at(root: &Path) -> BTreeSet<String> {
    LANGUAGE_MARKERS
        .iter()
        .filter(|(marker, _)| {
            std::fs::symlink_metadata(root.join(marker)).is_ok_and(|m| m.is_file())
        })
        .map(|(_, language)| (*language).to_string())
        .collect()
}

/// `mission_matches {catalogue}`: the ids of the catalogue's missions that
/// this contributor's local work fits, and how many tools and folders
/// matching was allowed to read -- counts only.
///
/// The catalogue is a parameter until Z7/Z8's server catalogue exists; the
/// daemon fetches nothing here. Every input is read under its own lock and
/// released, none mutably: no policy, queue, state or file is written, no
/// audit row is appended, and the log gets [`LOG_LABEL`] and nothing else
/// (M1, M2). The answer goes back over the local socket only.
pub fn handle_mission_matches(shared: &DaemonShared, req: &Request) -> Response {
    let Some(raw) = req.params.get("catalogue") else {
        return Response::err(req.id, ERR_BAD_PARAMS, "catalogue-required");
    };
    let catalogue = match ContributionMissionCatalogue::from_value(raw) {
        Ok(catalogue) => catalogue,
        Err(e) => return Response::err(req.id, ERR_BAD_PARAMS, e.label()),
    };
    let tools_on: BTreeSet<String> = {
        let settings = shared.settings.lock().expect("settings lock");
        settings
            .source_roots(&shared.store)
            .source_identities()
            .into_keys()
            .map(str::to_string)
            .collect()
    };
    // Kept and withdrawn sessions can never count toward a mission (M2): a
    // keep is the contributor explicitly declining the one path a session
    // could have counted through, and a withdrawal reverses a contribution
    // that already happened. Both are read here, never written -- matching
    // changes neither the queue nor the history cache. An already-uploaded
    // session that is neither kept nor withdrawn, and one still being
    // written, are excluded from neither set and stay candidates: see the
    // module doc.
    let excluded_paths: BTreeSet<String> = {
        let queue = shared.queue.lock().expect("queue lock");
        let withdrawn_submissions: BTreeSet<uuid::Uuid> = HistoryCache::load(&shared.store)
            .unwrap_or_default()
            .iter()
            .filter(|r| is_taken_back(r))
            .map(|r| r.submission_id)
            .collect();
        queue
            .all()
            .iter()
            .filter(|e| {
                e.is_kept()
                    || e.submission_id
                        .is_some_and(|id| withdrawn_submissions.contains(&id))
            })
            .map(|e| e.path.to_string_lossy().to_string())
            .collect()
    };
    // A key the cache has not filled in yet is worked out here and not
    // written back: matching leaves the cache as it found it.
    let seen: Vec<SeenSession> = {
        let state = shared.state.lock().expect("state lock");
        state
            .cwd_cache
            .iter()
            .map(|(path, e)| SeenSession {
                tool: e.tool.clone(),
                adapter: e.adapter.clone(),
                folder: e
                    .project_key
                    .clone()
                    .unwrap_or_else(|| super::policy::project_for(e.cwd.as_deref()).0),
                path: path.clone(),
            })
            .collect()
    };
    let (sessions, roots) = {
        let policy = shared.policy.lock().expect("policy lock");
        let sessions = readable_sessions(&policy, &tools_on, &excluded_paths, &seen);
        let roots: BTreeMap<String, PathBuf> = sessions
            .iter()
            .map(|s| {
                let shown = policy
                    .projects
                    .get(&s.folder)
                    .and_then(|e| e.display_path.clone())
                    .unwrap_or_else(|| s.folder.clone());
                (s.folder.clone(), PathBuf::from(shown))
            })
            .collect();
        (sessions, roots)
    };
    let facts = local_facts(sessions, catalogue.needs_languages(), |folder| {
        roots
            .get(folder)
            .map(|root| languages_at(root))
            .unwrap_or_default()
    });
    let matches = match_missions(&catalogue, &facts);
    tracing::debug!("{LOG_LABEL}");
    Response::ok(
        req.id,
        serde_json::json!({
            "matches": matches,
            "read": {"tools": facts.tools_read(), "folders": facts.folders_read()},
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contribution_missions::{ContributionMissionCatalogue, match_missions};
    use chrono::Utc;
    use serde_json::json;

    fn seen(tool: Option<&str>, adapter: Option<&str>, folder: &str) -> SeenSession {
        // A unique-enough path: nothing here exercises the kept/withdrawn
        // exclusion, so it only needs to not collide within one `all` list.
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        SeenSession {
            tool: tool.map(str::to_string),
            adapter: adapter.map(str::to_string),
            folder: folder.to_string(),
            path: format!("/seen/{n}"),
        }
    }

    fn tools(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    /// A folder set to Never and an adapter switched off are not read: their
    /// sessions never reach the matcher, a Never folder's root is never
    /// looked at, and a mission only they would fit does not match. Nor is
    /// a session with no recorded adapter -- a cache entry written before
    /// that field existed -- read: that fails closed exactly like a missing
    /// tool, even though its folder and (self-declared) tool are otherwise
    /// fine (#1227 review, finding 2).
    #[test]
    fn never_folders_switched_off_adapters_and_legacy_entries_are_not_read() {
        let mut policy = ProjectPolicy::new();
        policy
            .set_mode("/work/never", ProjectMode::Ignore, Utc::now())
            .unwrap();
        let on = tools(&["claude-code", "trajectory"]);
        let all = vec![
            seen(Some("claude-code"), Some("claude-code"), "/work/ask"),
            seen(Some("claude-code"), Some("claude-code"), "/work/never"),
            seen(Some("codex"), Some("codex"), "/work/ask"),
            // An imported Antigravity conversation: displayed as
            // "antigravity", but read by the trajectory adapter, which is on.
            seen(Some("antigravity"), Some("trajectory"), "/work/ask"),
            seen(None, Some("claude-code"), "/work/ask"),
            // A legacy cache entry: a tool was recorded, but no adapter.
            seen(Some("claude-code"), None, "/work/ask"),
        ];
        let read = readable_sessions(&policy, &on, &BTreeSet::new(), &all);
        assert_eq!(
            read,
            vec![
                SessionFact {
                    tool: "claude-code".into(),
                    folder: "/work/ask".into()
                },
                SessionFact {
                    tool: "antigravity".into(),
                    folder: "/work/ask".into()
                },
            ]
        );

        let mut asked = Vec::new();
        let facts = local_facts(read, true, |folder| {
            asked.push(folder.to_string());
            // Only the Never folder is a Rust repository.
            if folder == "/work/never" {
                BTreeSet::from(["rust".to_string()])
            } else {
                BTreeSet::new()
            }
        });
        assert_eq!(asked, vec!["/work/ask"], "a Never folder is not looked at");
        let cat = ContributionMissionCatalogue::from_value(&json!({
            "schema_version": 1,
            "missions": [
                {"mission_id": "codex", "criteria": {"tools": ["codex"]}},
                {"mission_id": "rust", "criteria": {"languages": ["rust"]}},
                {"mission_id": "claude-2", "criteria": {"tools": ["claude-code"], "min_sessions": 2}},
                {"mission_id": "claude", "criteria": {"tools": ["claude-code"]}},
            ],
        }))
        .unwrap();
        assert_eq!(match_missions(&cat, &facts), vec!["claude"]);
        assert_eq!((facts.tools_read(), facts.folders_read()), (2, 1));
    }

    /// Under a Never contribution override nothing is read, from any
    /// folder, whatever its own mode, so nothing matches.
    #[test]
    fn the_never_override_reads_nothing() {
        let mut policy = ProjectPolicy::new();
        policy
            .set_mode("/work/ask", ProjectMode::NotifyOnly, Utc::now())
            .unwrap();
        policy
            .set_contribution_override(ProjectMode::Ignore, Utc::now(), None)
            .unwrap();
        let on = tools(&["claude-code"]);
        let all = vec![
            seen(Some("claude-code"), Some("claude-code"), "/work/ask"),
            seen(
                Some("claude-code"),
                Some("claude-code"),
                "/work/unconfigured",
            ),
            seen(
                Some("claude-code"),
                Some("claude-code"),
                UNKNOWN_PROJECT_KEY,
            ),
        ];
        let read = readable_sessions(&policy, &on, &BTreeSet::new(), &all);
        assert!(read.is_empty(), "{read:?}");
        let facts = local_facts(read, true, |folder| {
            panic!("no folder is looked at under a Never override: {folder}")
        });
        let cat = ContributionMissionCatalogue::from_value(&json!({
            "schema_version": 1, "missions": [{"mission_id": "any"}],
        }))
        .unwrap();
        assert!(match_missions(&cat, &facts).is_empty());
        assert_eq!((facts.tools_read(), facts.folders_read()), (0, 0));
    }

    /// A staged import that declares itself `claude-code` is read by
    /// whichever adapter actually discovered it -- `trajectory`, for one
    /// staged as a trajectory file -- and gating follows that adapter, not
    /// the self-declared claim. With trajectory off and claude-code on, the
    /// import is not read, and a mission asking only for `claude-code` does
    /// not match on it; once trajectory is also on, the same import is read
    /// and counts under the name it declared (#1227 review, finding 2).
    #[test]
    fn a_staged_claude_code_import_is_gated_on_its_adapter_not_its_claim() {
        let policy = ProjectPolicy::new();
        let staged = vec![seen(Some("claude-code"), Some("trajectory"), "/work/ask")];
        let cat = ContributionMissionCatalogue::from_value(&json!({
            "schema_version": 1,
            "missions": [{"mission_id": "claude", "criteria": {"tools": ["claude-code"]}}],
        }))
        .unwrap();

        let claude_code_only = tools(&["claude-code"]);
        let read = readable_sessions(&policy, &claude_code_only, &BTreeSet::new(), &staged);
        assert!(read.is_empty(), "{read:?}");
        let facts = local_facts(read, false, |_| BTreeSet::new());
        assert!(match_missions(&cat, &facts).is_empty());

        let both_on = tools(&["claude-code", "trajectory"]);
        let read = readable_sessions(&policy, &both_on, &BTreeSet::new(), &staged);
        let facts = local_facts(read, false, |_| BTreeSet::new());
        assert_eq!(match_missions(&cat, &facts), vec!["claude"]);
    }

    /// A session whose folder key is the unknown-project bucket resolves to
    /// `NotifyOnly`, not `Ignore`, so it is read and counted in
    /// `folders_read` like any other folder not set to Never -- but it is
    /// never asked about languages, since it has no folder root to look at
    /// (#1227 review, finding 4).
    #[test]
    fn the_unknown_project_bucket_is_read_but_never_language_probed() {
        let policy = ProjectPolicy::new();
        let on = tools(&["claude-code"]);
        let all = vec![
            seen(Some("claude-code"), Some("claude-code"), "/work/ask"),
            seen(
                Some("claude-code"),
                Some("claude-code"),
                UNKNOWN_PROJECT_KEY,
            ),
        ];
        let read = readable_sessions(&policy, &on, &BTreeSet::new(), &all);
        assert_eq!(read.len(), 2, "{read:?}");
        let mut asked = Vec::new();
        let facts = local_facts(read, true, |folder| {
            asked.push(folder.to_string());
            BTreeSet::new()
        });
        assert_eq!(
            asked,
            vec!["/work/ask"],
            "the unknown bucket is never looked at"
        );
        assert_eq!(facts.folders_read(), 2, "the unknown bucket still counts");
    }

    /// The marker files decide a folder's languages; a directory of the
    /// same name, or a link, does not.
    #[test]
    fn languages_come_from_marker_files_at_the_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        std::fs::create_dir(dir.path().join("go.mod")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("Cargo.toml"), dir.path().join("Gemfile"))
            .unwrap();
        assert_eq!(
            languages_at(dir.path()),
            BTreeSet::from(["rust".to_string()])
        );
        assert!(languages_at(&dir.path().join("missing")).is_empty());
    }
    // ---- mission_matches over IPC ----

    use super::super::history::{HistoryRecord, STATUS_ACCEPTED, STATUS_WITHDRAWN};
    use super::super::ipc::handle_request;
    use super::super::queue::{QueueEntry, QueueState};
    use super::super::settings::SourceDeclaration;
    use super::super::state::CwdCacheEntry;

    fn shared() -> (tempfile::TempDir, DaemonShared) {
        let (dir, store) = crate::config::tests_support::temp_store();
        (dir, DaemonShared::load(store).unwrap())
    }

    fn ask(s: &DaemonShared, params: serde_json::Value) -> Response {
        handle_request(
            s,
            &Request {
                id: 7,
                method: "mission_matches".to_string(),
                params,
            },
        )
    }

    /// A daemon with Claude Code on and Codex off, an Ask me folder that is
    /// a Rust repository, a Never folder that is a Python one, and sessions
    /// in both from both tools -- one with its project key not yet cached.
    fn seeded() -> (tempfile::TempDir, DaemonShared, tempfile::TempDir) {
        let (dir, s) = shared();
        let work = tempfile::tempdir().unwrap();
        let ask_dir = work.path().join("ask-repo");
        let never_dir = work.path().join("never-repo");
        std::fs::create_dir_all(&ask_dir).unwrap();
        std::fs::create_dir_all(&never_dir).unwrap();
        std::fs::write(ask_dir.join("Cargo.toml"), "").unwrap();
        std::fs::write(never_dir.join("pyproject.toml"), "").unwrap();
        {
            let mut settings = s.settings.lock().unwrap();
            settings.claude_source = Some(SourceDeclaration::Watch {
                path: work.path().join("claude-root"),
            });
            settings.codex_source = Some(SourceDeclaration::Off);
        }
        let key = |p: &Path| super::super::policy::project_for(Some(p.to_str().unwrap())).0;
        let (ask_key, never_key) = (key(&ask_dir), key(&never_dir));
        s.policy
            .lock()
            .unwrap()
            .set_mode(&never_key, ProjectMode::Ignore, Utc::now())
            .unwrap();
        let mut state = s.state.lock().unwrap();
        let mut put = |path: &str, tool: &str, cwd: &Path, cached_key: Option<&str>| {
            state.cwd_cache.insert(
                path.to_string(),
                CwdCacheEntry {
                    size_bytes: 1,
                    modified_at: Utc::now(),
                    cwd: Some(cwd.to_str().unwrap().to_string()),
                    project_key: cached_key.map(str::to_string),
                    tool: Some(tool.to_string()),
                    adapter: Some(tool.to_string()),
                },
            );
        };
        put(
            "/s/claude-ask-1.jsonl",
            "claude-code",
            &ask_dir,
            Some(&ask_key),
        );
        put("/s/claude-ask-2.jsonl", "claude-code", &ask_dir, None);
        put(
            "/s/claude-never.jsonl",
            "claude-code",
            &never_dir,
            Some(&never_key),
        );
        put("/s/codex-ask.jsonl", "codex", &ask_dir, Some(&ask_key));
        drop(state);
        (dir, s, work)
    }

    fn catalogue() -> serde_json::Value {
        json!({
            "schema_version": 1,
            "missions": [
                {"mission_id": "rust-claude", "title": "Rust with Claude",
                 "criteria": {"tools": ["claude-code"], "languages": ["rust"], "min_sessions": 2}},
                {"mission_id": "python", "title": "Python work",
                 "criteria": {"languages": ["python"]}},
                {"mission_id": "codex", "title": "Codex work",
                 "criteria": {"tools": ["codex"]}},
                {"mission_id": "claude-3", "title": "Three Claude sessions",
                 "criteria": {"tools": ["claude-code"], "min_sessions": 3}},
            ],
        })
    }

    /// Over IPC: the Never folder's sessions and the switched-off tool's
    /// are not read, so only the mission the Ask me folder's Claude Code
    /// sessions fit matches, and the counts say one tool, one folder.
    #[test]
    fn mission_matches_reads_only_tools_on_in_folders_not_never() {
        let (_dir, s, _work) = seeded();
        let result = ask(&s, json!({"catalogue": catalogue()})).result.unwrap();
        assert_eq!(
            result,
            json!({"matches": ["rust-claude"], "read": {"tools": 1, "folders": 1}})
        );
    }

    /// Under a Never contribution override, nothing is read and nothing
    /// matches.
    #[test]
    fn mission_matches_under_a_never_override_reads_nothing() {
        let (_dir, s, _work) = seeded();
        s.policy
            .lock()
            .unwrap()
            .set_contribution_override(ProjectMode::Ignore, Utc::now(), None)
            .unwrap();
        let result = ask(
            &s,
            json!({"catalogue": {"schema_version": 1, "missions": [{"mission_id": "any"}]}}),
        )
        .result
        .unwrap();
        assert_eq!(
            result,
            json!({"matches": [], "read": {"tools": 0, "folders": 0}})
        );
    }

    /// An IPC-level pin for finding 1 (#1227 review): the handler applies
    /// no adapter gate of its own before calling `readable_sessions`, so
    /// this fails if that gate is ever removed from `readable_sessions`.
    /// The pre-fix handler filtered the cache by the same rule before
    /// building `seen`, so the same scenario kept passing even with the
    /// gate missing from `readable_sessions` alone -- see the module doc.
    #[test]
    fn mission_matches_over_ipc_does_not_read_a_switched_off_adapter() {
        let (_dir, s, _work) = seeded();
        let result = ask(
            &s,
            json!({"catalogue": {
                "schema_version": 1,
                "missions": [{"mission_id": "codex", "criteria": {"tools": ["codex"]}}],
            }}),
        )
        .result
        .unwrap();
        assert_eq!(
            result,
            json!({"matches": [], "read": {"tools": 1, "folders": 1}}),
            "{result}"
        );
    }

    fn claude_two_mission() -> serde_json::Value {
        json!({
            "schema_version": 1,
            "missions": [{"mission_id": "claude-2",
                "criteria": {"tools": ["claude-code"], "min_sessions": 2}}],
        })
    }

    /// A minimal history record, for the kept/withdrawn tests below. Most
    /// fields are irrelevant to them; only `submission_id` and `status`
    /// matter.
    fn history_record(submission_id: uuid::Uuid, status: &str) -> HistoryRecord {
        HistoryRecord {
            submission_id,
            submitted_at: Utc::now(),
            project_id: String::new(),
            project_label: "ask".to_string(),
            source: "claude-code".to_string(),
            session_hash: "sha256:seed".to_string(),
            status: status.to_string(),
            consent_scopes: vec![],
            credit_points_pending: 0.0,
            credit_points_final: None,
            explanations: vec![],
            last_refreshed_at: None,
            withdrawn_at: None,
            revoked_at: None,
            approved_unattended: None,
            approved_verdict: None,
            uploaded_bytes: None,
        }
    }

    /// A kept session can never count toward a mission: M2 says a session
    /// counts only when it is contributed through one of the three consent
    /// paths, and a keep is the contributor explicitly declining that
    /// (#1227 review, finding 3).
    #[test]
    fn mission_matches_excludes_a_kept_session() {
        let (_dir, s, _work) = seeded();
        let before = ask(&s, json!({"catalogue": claude_two_mission()}))
            .result
            .unwrap();
        assert_eq!(before["matches"], json!(["claude-2"]), "{before}");

        let kept_id = uuid::Uuid::new_v4();
        s.queue
            .lock()
            .unwrap()
            .upsert(
                QueueEntry {
                    entry_id: kept_id,
                    session_hash: "sha256:kept".to_string(),
                    source: "claude-code".to_string(),
                    project_key: "/seed".to_string(),
                    project_label: "seed".to_string(),
                    path: PathBuf::from("/s/claude-ask-1.jsonl"),
                    size_bytes: 1,
                    discovered_at: Utc::now(),
                    ..Default::default()
                },
                500,
            )
            .unwrap();
        s.queue.lock().unwrap().keep(kept_id).unwrap();

        let after = ask(&s, json!({"catalogue": claude_two_mission()}))
            .result
            .unwrap();
        assert_eq!(
            after,
            json!({"matches": [], "read": {"tools": 1, "folders": 1}}),
            "a kept session is not a candidate: {after}"
        );
    }

    /// A withdrawn session can never count toward a mission: the withdrawal
    /// reverses a contribution that already happened (#1227 review, finding
    /// 3).
    #[test]
    fn mission_matches_excludes_a_withdrawn_session() {
        let (_dir, s, _work) = seeded();
        let before = ask(&s, json!({"catalogue": claude_two_mission()}))
            .result
            .unwrap();
        assert_eq!(before["matches"], json!(["claude-2"]), "{before}");

        let submission_id = uuid::Uuid::new_v4();
        s.queue
            .lock()
            .unwrap()
            .upsert(
                QueueEntry {
                    entry_id: uuid::Uuid::new_v4(),
                    session_hash: "sha256:withdrawn".to_string(),
                    source: "claude-code".to_string(),
                    project_key: "/seed".to_string(),
                    project_label: "seed".to_string(),
                    path: PathBuf::from("/s/claude-ask-1.jsonl"),
                    size_bytes: 1,
                    discovered_at: Utc::now(),
                    state: QueueState::Uploaded,
                    submission_id: Some(submission_id),
                    ..Default::default()
                },
                500,
            )
            .unwrap();
        HistoryCache::save(&s.store, &[history_record(submission_id, STATUS_WITHDRAWN)]).unwrap();

        let after = ask(&s, json!({"catalogue": claude_two_mission()}))
            .result
            .unwrap();
        assert_eq!(
            after,
            json!({"matches": [], "read": {"tools": 1, "folders": 1}}),
            "a withdrawn session is not a candidate: {after}"
        );
    }

    /// An already-uploaded session that is neither kept nor withdrawn stays
    /// a candidate: M2 excludes only a keep (explicitly declined) and a
    /// withdrawal (reversed); an ordinary accepted upload did neither.
    /// `seeded()`'s own sessions, which carry no queue entry at all, are
    /// already the not-yet-quiescent case: this pins both halves of the
    /// module doc's claim (#1227 review, finding 3).
    #[test]
    fn mission_matches_still_counts_an_accepted_upload() {
        let (_dir, s, _work) = seeded();
        let submission_id = uuid::Uuid::new_v4();
        s.queue
            .lock()
            .unwrap()
            .upsert(
                QueueEntry {
                    entry_id: uuid::Uuid::new_v4(),
                    session_hash: "sha256:accepted".to_string(),
                    source: "claude-code".to_string(),
                    project_key: "/seed".to_string(),
                    project_label: "seed".to_string(),
                    path: PathBuf::from("/s/claude-ask-1.jsonl"),
                    size_bytes: 1,
                    discovered_at: Utc::now(),
                    state: QueueState::Uploaded,
                    submission_id: Some(submission_id),
                    ..Default::default()
                },
                500,
            )
            .unwrap();
        HistoryCache::save(&s.store, &[history_record(submission_id, STATUS_ACCEPTED)]).unwrap();

        let result = ask(&s, json!({"catalogue": catalogue()})).result.unwrap();
        assert_eq!(
            result,
            json!({"matches": ["rust-claude"], "read": {"tools": 1, "folders": 1}}),
            "an accepted upload that is neither kept nor withdrawn still counts: {result}"
        );
    }

    /// Every file in the store directory, by name.
    fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().is_file())
            .map(|e| {
                (
                    e.file_name().to_string_lossy().to_string(),
                    std::fs::read(e.path()).unwrap(),
                )
            })
            .collect()
    }

    /// M2: matching changes no policy, queue or daemon state, in memory or
    /// on disk -- not even the cache key it had to work out.
    #[test]
    fn mission_matches_changes_no_policy_or_queue_state() {
        let (_dir, s, _work) = seeded();
        s.queue
            .lock()
            .unwrap()
            .upsert(
                super::super::queue::QueueEntry {
                    entry_id: uuid::Uuid::new_v4(),
                    session_hash: "sha256:seed".to_string(),
                    source: "claude-code".to_string(),
                    project_key: "/seed".to_string(),
                    project_label: "seed".to_string(),
                    path: PathBuf::from("/s/claude-ask-1.jsonl"),
                    size_bytes: 1,
                    discovered_at: Utc::now(),
                    ..Default::default()
                },
                500,
            )
            .unwrap();
        s.policy.lock().unwrap().save(&s.store).unwrap();
        let snapshot = |s: &DaemonShared| {
            (
                s.policy.lock().unwrap().clone(),
                s.queue.lock().unwrap().all().to_vec(),
                serde_json::to_value(&s.state.lock().unwrap().cwd_cache).unwrap(),
                files(s.store.dir()),
            )
        };
        let before = snapshot(&s);
        let reply = ask(&s, json!({"catalogue": catalogue()}));
        assert!(reply.result.is_some(), "{reply:?}");
        assert!(
            before == snapshot(&s),
            "matching changed policy, queue or state"
        );
    }

    /// Records every event and span field a test's code emits.
    #[derive(Clone, Default)]
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    struct Fields(String);

    impl tracing::field::Visit for Fields {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            use std::fmt::Write;
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }

    impl tracing::Subscriber for Capture {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            let mut fields = Fields(span.metadata().name().to_string());
            span.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
            let mut fields = Fields(String::new());
            values.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
        }
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut fields = Fields(event.metadata().target().to_string());
            event.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    /// M1: what matching writes to the log is its label, and to the audit
    /// log nothing -- no folder, tool, mission or count.
    #[test]
    fn mission_matching_logs_labels_only_and_audits_nothing() {
        let (_dir, s, work) = seeded();
        let audit_before = super::super::audit::load(&s.store).unwrap();
        let capture = Capture::default();
        // Run the capture on a dedicated, short-lived thread rather than
        // whichever pool thread cargo's test harness happened to reuse for
        // this test (#1227 review: "not verified -- could be flaky under
        // parallel test threads"). Tracing's per-callsite interest cache is
        // process-wide -- a single atomic per callsite, rebuilt by asking
        // every currently-registered `Dispatch`, including ones from
        // unrelated tests concurrently alive on other threads -- so a
        // dedicated thread at least guarantees nothing else ever ran on it
        // before this block, and that the thread, and every `Dispatch`
        // created on it, is gone the moment `scope` returns.
        //
        // With a single scoped subscriber alive, tracing works out a
        // callsite's interest from whichever thread registers it first --
        // another test's, which has none, so the handler's line would be
        // cached as unwanted. A second, silent dispatcher makes it ask every
        // live subscriber instead, which settles the callsite's cached
        // interest at `Interest::sometimes()` rather than a fixed yes/no.
        // From then on every event at that callsite asks whichever
        // dispatch is active on the thread raising it, which on this
        // thread stays `capture` for as long as `with_default` is in
        // scope here -- regardless of what any other test thread does
        // concurrently.
        let reply = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    tracing::subscriber::with_default(capture.clone(), || {
                        let _second =
                            tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
                        tracing::callsite::rebuild_interest_cache();
                        ask(&s, json!({"catalogue": catalogue()}))
                    })
                })
                .join()
                .unwrap()
        });
        assert_eq!(reply.result.unwrap()["matches"], json!(["rust-claude"]));
        let lines = capture.0.lock().unwrap().clone();
        assert!(
            lines.iter().any(|l| l.contains(LOG_LABEL)),
            "the label is logged: {lines:?}"
        );
        let logged = lines.join("\n");
        let work = work.path().to_string_lossy().to_string();
        for secret in [
            work.as_str(),
            "ask-repo",
            "never-repo",
            "claude-code",
            "codex",
            "rust",
            "python",
            "Rust with Claude",
            "/s/",
        ] {
            assert!(
                !logged.contains(secret),
                "the log names {secret:?}: {lines:?}"
            );
        }
        assert!(
            lines.iter().all(|l| l.trim_end().ends_with(LOG_LABEL)),
            "nothing but the label: {lines:?}"
        );
        assert_eq!(super::super::audit::load(&s.store).unwrap(), audit_before);
    }

    /// The catalogue is required and read strictly about its version.
    #[test]
    fn mission_matches_refuses_a_missing_or_newer_catalogue() {
        let (_dir, s) = shared();
        let label = |r: Response| r.error.unwrap().message;
        assert_eq!(label(ask(&s, json!({}))), "catalogue-required");
        assert_eq!(
            label(ask(&s, json!({"catalogue": {"schema_version": 2}}))),
            "catalogue-schema-unsupported"
        );
        assert_eq!(
            label(ask(&s, json!({"catalogue": "missions"}))),
            "catalogue-invalid"
        );
    }
}
