//! Matching contribution missions on this Mac (K16, #1173).
//!
//! The rules are the consent design's "Missions" section:
//!
//! - **M1. Matching stays on this Mac.** It reads only sessions from tools
//!   the contributor has left on, in folders not set to Never, and under a
//!   Never contribution override it reads nothing. What it reads, and what
//!   it finds, go into no network request, audit row or log line.
//! - **M2. A mission sends nothing by itself.** Matching takes the policy
//!   by shared reference and the queue not at all: it cannot arm, approve or
//!   widen anything.
//!
//! [`readable_sessions`] is where M1's read rule lives, and the only place:
//! everything after it, including [`crate::contribution_missions::match_missions`],
//! sees only what it let through.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::contribution_missions::{
    ContributionMissionCatalogue, LANGUAGE_MARKERS, LocalFacts, SessionFact, match_missions,
};

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, Request, Response};
use super::policy::{ProjectMode, ProjectPolicy, UNKNOWN_PROJECT_KEY};

/// The one log line matching writes: a label, nothing about what was read
/// or found (M1).
pub const LOG_LABEL: &str = "mission-matches-answered";

/// A session the daemon has seen, as its cwd cache records it: the tool,
/// if one was recorded, and the folder's project key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenSession {
    pub tool: Option<String>,
    pub folder: String,
}

/// Whether sessions from `tool` may be read: its adapter is one the
/// contributor has left on. An imported Antigravity conversation is read by
/// the trajectory adapter, so it is on when that is.
pub fn tool_is_on(tool: &str, tools_on: &BTreeSet<String>) -> bool {
    let adapter = if tool == crate::source::DECLARED_SOURCE_ANTIGRAVITY {
        crate::source::SOURCE_TRAJECTORY
    } else {
        tool
    };
    tools_on.contains(adapter)
}

/// M1's read rule: the sessions matching may read, out of everything seen.
///
/// - A session from a tool that is switched off is not read, nor one with
///   no recorded tool, which cannot be shown to be on.
/// - A session in a folder whose mode in force is Never is not read. That
///   is `resolve`, not the folder's own mode, so a Never contribution
///   override (#1208) makes every folder Never and nothing is read; and a
///   folder set to Never stays Never under every other override.
/// - Folders that are not armed are read; nothing read leaves the device.
///
/// `policy` is borrowed, never mutably: matching changes nothing (M2).
pub fn readable_sessions(
    policy: &ProjectPolicy,
    tools_on: &BTreeSet<String>,
    seen: &[SeenSession],
) -> Vec<SessionFact> {
    seen.iter()
        .filter_map(|s| {
            let tool = s.tool.as_deref()?;
            (tool_is_on(tool, tools_on) && policy.resolve(&s.folder) != ProjectMode::Ignore).then(
                || SessionFact {
                    tool: tool.to_string(),
                    folder: s.folder.clone(),
                },
            )
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
    let cached: Vec<(String, Option<String>, Option<String>)> = {
        let state = shared.state.lock().expect("state lock");
        state
            .cwd_cache
            .values()
            .filter_map(|e| {
                let tool = e.tool.clone().filter(|t| tool_is_on(t, &tools_on))?;
                Some((tool, e.project_key.clone(), e.cwd.clone()))
            })
            .collect()
    };
    // A key the cache has not filled in yet is worked out here and not
    // written back: matching leaves the cache as it found it.
    let seen: Vec<SeenSession> = cached
        .into_iter()
        .map(|(tool, key, cwd)| SeenSession {
            tool: Some(tool),
            folder: key.unwrap_or_else(|| super::policy::project_for(cwd.as_deref()).0),
        })
        .collect();
    let (sessions, roots) = {
        let policy = shared.policy.lock().expect("policy lock");
        let sessions = readable_sessions(&policy, &tools_on, &seen);
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

    fn seen(tool: Option<&str>, folder: &str) -> SeenSession {
        SeenSession {
            tool: tool.map(str::to_string),
            folder: folder.to_string(),
        }
    }

    fn tools(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    /// A folder set to Never and a tool switched off are not read: their
    /// sessions never reach the matcher, a Never folder's root is never
    /// looked at, and a mission only they would fit does not match.
    #[test]
    fn never_folders_and_switched_off_tools_are_not_read() {
        let mut policy = ProjectPolicy::new();
        policy
            .set_mode("/work/never", ProjectMode::Ignore, Utc::now())
            .unwrap();
        let on = tools(&["claude-code", "trajectory"]);
        let all = vec![
            seen(Some("claude-code"), "/work/ask"),
            seen(Some("claude-code"), "/work/never"),
            seen(Some("codex"), "/work/ask"),
            seen(Some("antigravity"), "/work/ask"),
            seen(None, "/work/ask"),
        ];
        let read = readable_sessions(&policy, &on, &all);
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
            seen(Some("claude-code"), "/work/ask"),
            seen(Some("claude-code"), "/work/unconfigured"),
            seen(Some("claude-code"), UNKNOWN_PROJECT_KEY),
        ];
        let read = readable_sessions(&policy, &on, &all);
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

    use super::super::ipc::handle_request;
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
        // With a single scoped subscriber alive, tracing works out a
        // callsite's interest from whichever thread registers it first --
        // another test's, which has none, so the handler's line would be
        // cached as unwanted. A second, silent dispatcher makes it ask every
        // live subscriber instead; then every callsite is asked again.
        let _second = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
        let reply = tracing::subscriber::with_default(capture.clone(), || {
            tracing::callsite::rebuild_interest_cache();
            ask(&s, json!({"catalogue": catalogue()}))
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
