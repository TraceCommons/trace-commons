//! What is on this machine, so the roots screen can ask about something
//! specific.
//!
//! Discovery is not consent, and this module is careful about the
//! difference. It finds the conventional session stores and describes them
//! -- where, whether they exist, how many session files, how recently
//! touched -- so a contributor is agreeing to "946 Claude Code sessions,
//! most recent 2 hours ago" rather than to an empty text field they have to
//! fill from memory. Nothing here selects anything, and nothing here writes
//! a declaration.
//!
//! It reads directory entries and file metadata only. It never opens a
//! session file: the point is to describe the store well enough to consent
//! to, and reading the contents before consent would be the thing consent is
//! for.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::{SOURCE_CLAUDE_CODE, SOURCE_CODEX};

/// The environment variable Claude Code uses to relocate its config
/// directory, and therefore its `projects/` session store.
///
/// Verified against the installed binary on the machine this was written on
/// (`claude` 2.1.235) rather than taken from memory, per this repo's rule
/// about never recommending a config key that has not been checked. If a
/// future version renames it, discovery silently falls back to the
/// conventional location -- it does not invent a second guess.
pub const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";

/// The environment variable Codex uses to relocate its home directory, and
/// therefore its `sessions/` store. Verified from `codex --help` on the same
/// machine, which documents `$CODEX_HOME/<name>.config.toml`.
pub const CODEX_HOME_ENV: &str = "CODEX_HOME";

/// The session-file extension both stores use.
const SESSION_EXTENSION: &str = "jsonl";

/// One candidate session store, described well enough to consent to.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SourceCandidate {
    /// `claude-code` or `codex`, matching the adapter names.
    pub source: String,
    /// Where this store would be watched.
    pub path: PathBuf,
    /// Whether that directory exists right now.
    pub exists: bool,
    /// How many session files were found, counted recursively.
    ///
    /// Zero on a directory that exists but holds none, which is a materially
    /// different thing to show than a directory that is not there.
    pub session_count: u64,
    /// The most recent session-file mtime, if any.
    pub most_recent: Option<DateTime<Utc>>,
    /// Whether an environment variable relocated this store, so a screen can
    /// say why the path is not the usual one.
    pub relocated_by_env: bool,
}

/// Probe both conventional session stores.
///
/// `home` and `env` are injected so this is testable without touching the
/// machine's real directories -- which matters more than usual here, since
/// the thing being tested is code that looks at a developer's actual work.
pub fn probe<F>(home: &Path, env: F) -> Vec<SourceCandidate>
where
    F: Fn(&str) -> Option<String>,
{
    let claude_base = env(CLAUDE_CONFIG_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let codex_base = env(CODEX_HOME_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);

    vec![
        describe(
            SOURCE_CLAUDE_CODE,
            // The session store is the `projects` SUBdirectory, not the
            // config directory itself. Watching the parent would take in
            // settings, plugins and anything else that lives beside it --
            // more than the contributor agreed to.
            claude_base
                .clone()
                .unwrap_or_else(|| home.join(".claude"))
                .join("projects"),
            claude_base.is_some(),
        ),
        describe(
            SOURCE_CODEX,
            codex_base
                .clone()
                .unwrap_or_else(|| home.join(".codex"))
                .join("sessions"),
            codex_base.is_some(),
        ),
    ]
}

/// Probe using this machine's real home and environment.
pub fn probe_this_machine() -> Vec<SourceCandidate> {
    let home = dirs::home_dir().unwrap_or_default();
    probe(&home, |key| std::env::var(key).ok())
}

fn describe(source: &str, path: PathBuf, relocated_by_env: bool) -> SourceCandidate {
    let (exists, session_count, most_recent) = if path.is_dir() {
        let (count, recent) = count_sessions(&path);
        (true, count, recent)
    } else {
        (false, 0, None)
    };
    SourceCandidate {
        source: source.to_string(),
        path,
        exists,
        session_count,
        most_recent,
        relocated_by_env,
    }
}

/// Count `.jsonl` files under `root`, and note the most recent mtime.
///
/// Walks with an explicit stack rather than recursion, and follows no
/// symlinks: a symlinked directory could point anywhere, and this is a
/// counting pass whose whole justification is that it stays inside the store
/// it is describing.
fn count_sessions(root: &Path) -> (u64, Option<DateTime<Utc>>) {
    let mut count = 0_u64;
    let mut most_recent: Option<DateTime<Utc>> = None;
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some(SESSION_EXTENSION) {
                continue;
            }
            count += 1;
            if let Ok(meta) = entry.metadata()
                && let Ok(modified) = meta.modified()
            {
                let stamp: DateTime<Utc> = modified.into();
                most_recent = Some(match most_recent {
                    Some(current) if current >= stamp => current,
                    _ => stamp,
                });
            }
        }
    }

    (count, most_recent)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("tc-discovery-{tag}-{unique}"));
            std::fs::create_dir_all(&path).unwrap();
            Scratch(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn write_session(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), b"{}\n").unwrap();
    }

    #[test]
    fn probes_the_session_subdirectories_not_the_parent_dot_directories() {
        let home = Scratch::new("nesting");
        let found = probe(home.path(), no_env);

        assert_eq!(found[0].path, home.path().join(".claude/projects"));
        assert_eq!(found[1].path, home.path().join(".codex/sessions"));
        // Watching ~/.claude rather than ~/.claude/projects would take in
        // settings, plugins, and history alongside the sessions.
        assert_ne!(found[0].path, home.path().join(".claude"));
    }

    #[test]
    fn an_absent_store_is_reported_as_absent_rather_than_empty() {
        let home = Scratch::new("absent");
        let found = probe(home.path(), no_env);

        assert!(!found[0].exists);
        assert_eq!(found[0].session_count, 0);
        assert_eq!(found[0].most_recent, None);
    }

    #[test]
    fn an_existing_but_empty_store_is_distinguishable_from_an_absent_one() {
        let home = Scratch::new("empty");
        std::fs::create_dir_all(home.path().join(".claude/projects")).unwrap();
        let found = probe(home.path(), no_env);

        assert!(found[0].exists, "the directory is there");
        assert_eq!(found[0].session_count, 0, "and holds no sessions");
    }

    #[test]
    fn counts_sessions_recursively_and_reports_the_most_recent() {
        let home = Scratch::new("counting");
        let projects = home.path().join(".claude/projects");
        write_session(&projects, "a.jsonl");
        write_session(&projects.join("nested/deeper"), "b.jsonl");
        write_session(&projects, "notes.txt");

        let found = probe(home.path(), no_env);
        assert_eq!(
            found[0].session_count, 2,
            "only .jsonl counts, and nesting is followed"
        );
        assert!(found[0].most_recent.is_some());
    }

    #[test]
    fn an_env_override_relocates_the_store_and_says_so() {
        let home = Scratch::new("env-home");
        let elsewhere = Scratch::new("env-target");
        write_session(&elsewhere.path().join("projects"), "a.jsonl");

        let target = elsewhere.path().to_str().unwrap().to_string();
        let found = probe(home.path(), |key| {
            (key == CLAUDE_CONFIG_DIR_ENV).then(|| target.clone())
        });

        assert_eq!(found[0].path, elsewhere.path().join("projects"));
        assert!(found[0].relocated_by_env);
        assert_eq!(found[0].session_count, 1);
        assert!(
            !found[1].relocated_by_env,
            "codex was not relocated and must not claim to be"
        );
    }

    #[test]
    fn an_empty_env_override_is_treated_as_unset() {
        let home = Scratch::new("blank-env");
        let found = probe(home.path(), |key| (key == CODEX_HOME_ENV).then(String::new));
        assert_eq!(found[1].path, home.path().join(".codex/sessions"));
        assert!(!found[1].relocated_by_env);
    }

    #[test]
    fn discovery_never_opens_a_session_file() {
        // A file that would fail to parse proves the counting pass does not
        // read contents: if it did, this would error rather than count.
        let home = Scratch::new("unreadable");
        let projects = home.path().join(".claude/projects");
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::write(projects.join("garbage.jsonl"), b"\xff\xfe not json at all").unwrap();

        let found = probe(home.path(), no_env);
        assert_eq!(found[0].session_count, 1);
    }
}
