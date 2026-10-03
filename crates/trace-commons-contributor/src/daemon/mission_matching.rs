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
use std::path::Path;

use crate::contribution_missions::{LANGUAGE_MARKERS, LocalFacts, SessionFact};

use super::policy::{ProjectMode, ProjectPolicy, UNKNOWN_PROJECT_KEY};

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
}
