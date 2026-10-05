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

use super::cline::{
    CLINE_DATA_DIR_ENV, CLINE_DIR_ENV, CLINE_SESSION_DATA_DIR_ENV, conventional_root as cline_root,
};
use super::gemini_cli::{GEMINI_CLI_HOME_ENV, conventional_root};
use super::{
    SOURCE_CLAUDE_CODE, SOURCE_CLINE, SOURCE_CODEX, SOURCE_GEMINI_CLI, SOURCE_OPENCODE,
    SOURCE_TRAJECTORY, source_answers_at,
};

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

/// The session-file suffix the Claude Code and Codex stores use.
const JSONL_SUFFIX: &str = ".jsonl";

/// Gemini CLI writes one JSON document per session rather than JSONL, so
/// counting `.jsonl` under its store would report every machine as empty.
const JSON_SUFFIX: &str = ".json";

/// Cline writes `<id>.messages.json` beside a `<id>.json` manifest. Counting
/// `.json` would report two sessions for every one, so the suffix names the
/// messages file specifically.
const MESSAGES_JSON_SUFFIX: &str = ".messages.json";

/// OpenCode's own export writes one `<session-id>.json` file per session,
/// directly in the declared folder -- see `source::opencode`, which reads
/// that folder non-recursively. The count here is flat too, so "N sessions
/// found" never includes a file the adapter would not read.
const OPENCODE_JSON_SUFFIX: &str = ".json";

/// How far [`count_sessions`] may look below the store it was handed.
#[derive(Debug, Clone, Copy)]
enum Walk {
    /// Descend into every subdirectory. Only for the conventional stores
    /// [`probe`] derives itself, whose layouts nest by project or by date.
    Recursive,
    /// Read the one directory and nothing below it, stopping after
    /// `entry_budget` directory entries. For a folder a shell named -- if it
    /// passes `$HOME` by mistake, this reads one directory's first entries on
    /// the calling thread rather than the whole tree.
    Flat { entry_budget: usize },
}

/// One candidate session store, described well enough to consent to.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SourceCandidate {
    /// The adapter name this candidate belongs to, e.g. `claude-code`,
    /// `codex` or `gemini-cli`.
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
    /// The vendor this tool's own calls answer at by default, e.g.
    /// `"Anthropic"` for Claude Code -- so a folder screen can show "answers
    /// at Anthropic" beside the row the way the design does.
    ///
    /// `None` for a tool this build does not name a default for, which
    /// includes both an unrecognised source and one -- Cline, OpenCode --
    /// that ships with no single default to name. This is a fixed label
    /// from [`super::source_answers_at`]'s table, not a claim this daemon
    /// checked or verified for the copy actually running on this machine.
    pub answers_at: Option<String>,
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
    let gemini_relocated = env(GEMINI_CLI_HOME_ENV).is_some_and(|v| !v.is_empty());
    let cline_relocated = [
        CLINE_SESSION_DATA_DIR_ENV,
        CLINE_DATA_DIR_ENV,
        CLINE_DIR_ENV,
    ]
    .iter()
    .any(|key| env(key).is_some_and(|v| !v.trim().is_empty()));

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
            JSONL_SUFFIX,
            Walk::Recursive,
        ),
        describe(
            SOURCE_CODEX,
            codex_base
                .clone()
                .unwrap_or_else(|| home.join(".codex"))
                .join("sessions"),
            codex_base.is_some(),
            JSONL_SUFFIX,
            Walk::Recursive,
        ),
        // Appended rather than inserted: a shell written before this source
        // existed indexes the first two rows by position.
        describe(
            SOURCE_GEMINI_CLI,
            // The store is `<gemini home>/tmp`, which holds one directory
            // per project; the session documents are two levels below it.
            conventional_root(home, &env),
            gemini_relocated,
            JSON_SUFFIX,
            Walk::Recursive,
        ),
        // Appended: shells index the first rows by position.
        describe(
            SOURCE_CLINE,
            // One directory per session, each holding the messages document
            // and, usually, a manifest beside it.
            cline_root(home, &env),
            cline_relocated,
            MESSAGES_JSON_SUFFIX,
            Walk::Recursive,
        ),
    ]
}

/// Probe using this machine's real home and environment.
///
/// OpenCode is deliberately absent from this list. Every other row here has
/// a conventional per-user location this build can guess before anyone has
/// said anything -- that guess is exactly what `Undeclared::Conventional` in
/// `source::mod` is for. OpenCode has none: its export folder is picked by
/// the contributor, one at a time, so there is nothing to probe blind and no
/// path this call could put in a row without inventing one. See
/// [`describe_opencode`] for the folder a contributor has actually named.
pub fn probe_this_machine() -> Vec<SourceCandidate> {
    let home = super::home_dir();
    probe(&home, |key| std::env::var(key).ok())
}

/// Describe a folder the contributor has already named as their OpenCode
/// export directory, the way [`probe`]'s rows describe a conventional store.
///
/// Unlike every source `probe` returns, this is never called blind: OpenCode
/// has no conventional location (see [`probe_this_machine`]), so the only
/// path worth describing is one the contributor picked, typically moments
/// earlier through a folder chooser. A shell calls this right after that
/// pick to say "N sessions found" with a real count rather than trusting the
/// folder name alone, and again later to refresh the row for a folder
/// recorded in settings (`opencode_source` in
/// `daemon::settings::DaemonSettings`).
#[must_use]
pub fn describe_opencode(path: &Path) -> SourceCandidate {
    describe(
        SOURCE_OPENCODE,
        path.to_path_buf(),
        false,
        OPENCODE_JSON_SUFFIX,
        Walk::Flat {
            entry_budget: super::opencode::DISCOVERY_ENTRY_BUDGET,
        },
    )
}

/// How many directory entries one kind's layout walk in [`describe_folder`]
/// may read before it stops counting.
///
/// A picked folder can be anything -- `$HOME`, `~/Downloads` -- and this runs
/// on the calling thread. Each walk is already bounded in depth by the
/// layout it checks; this bounds its breadth. It sits well above a real
/// store (thousands of sessions), so a real count is not truncated.
const FOLDER_ENTRY_BUDGET: usize = 65_536;

/// Recognise a folder the contributor picked by its layout alone, so "add
/// your tool" can say which tool's sessions it holds.
///
/// Runs each parsed kind's layout check over `path` and returns one
/// candidate per kind whose layout matches, in a fixed order -- Claude Code
/// (`<encoded-cwd>/<uuid>.jsonl`), Codex (`YYYY/MM/DD/rollout-*.jsonl`),
/// Gemini CLI (`<project>/chats/session-*.json`), Cline
/// (`<id>/<id>.messages.json`), OpenCode (flat `*.json`) and trajectory
/// (flat `*.json`/`*.jsonl`). A kind matches when at least one session file
/// sits where its layout puts one.
///
/// It never guesses. A folder whose layout fits two kinds -- a flat folder
/// of `.json` files is both an OpenCode export and a trajectory export --
/// reports both, and the shell asks; a folder that fits none, or is not
/// there, reports nothing, and the shell refuses it. Each row's `path` is
/// the picked folder itself.
///
/// Like everything in this module it reads directory entries and metadata
/// only, follows no symlink, and never opens a file.
#[must_use]
pub fn describe_folder(path: &Path) -> Vec<SourceCandidate> {
    if !path.is_dir() {
        return Vec::new();
    }
    let flat = Walk::Flat {
        entry_budget: super::opencode::DISCOVERY_ENTRY_BUDGET,
    };
    let opencode = describe_opencode(path);
    let (json, json_recent) = count_sessions(path, JSON_SUFFIX, flat);
    let (jsonl, jsonl_recent) = count_sessions(path, JSONL_SUFFIX, flat);
    let trajectory = Tally {
        count: json + jsonl,
        most_recent: json_recent.max(jsonl_recent),
        budget: 0,
    };

    [
        (SOURCE_CLAUDE_CODE, claude_code_layout(path)),
        (SOURCE_CODEX, codex_layout(path)),
        (SOURCE_GEMINI_CLI, gemini_layout(path)),
        (SOURCE_CLINE, cline_layout(path)),
        (
            SOURCE_OPENCODE,
            Tally {
                count: opencode.session_count,
                most_recent: opencode.most_recent,
                budget: 0,
            },
        ),
        (SOURCE_TRAJECTORY, trajectory),
    ]
    .into_iter()
    .filter(|(_, tally)| tally.count > 0)
    .map(|(source, tally)| SourceCandidate {
        source: source.to_string(),
        path: path.to_path_buf(),
        exists: true,
        session_count: tally.count,
        most_recent: tally.most_recent,
        relocated_by_env: false,
        answers_at: source_answers_at(source).map(str::to_string),
    })
    .collect()
}

/// One layout walk's running count, and what is left of its entry budget.
struct Tally {
    count: u64,
    most_recent: Option<DateTime<Utc>>,
    budget: usize,
}

impl Tally {
    fn new() -> Self {
        Tally {
            count: 0,
            most_recent: None,
            budget: FOLDER_ENTRY_BUDGET,
        }
    }

    /// `dir`'s entries, up to what is left of the budget. An unreadable
    /// directory contributes nothing.
    fn entries(&mut self, dir: &Path) -> Vec<std::fs::DirEntry> {
        let Ok(read) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in read {
            if self.budget == 0 {
                break;
            }
            self.budget -= 1;
            if let Ok(entry) = entry {
                out.push(entry);
            }
        }
        out
    }

    /// `dir`'s subdirectories, never through a symlink: `file_type` does not
    /// follow one.
    fn subdirs(&mut self, dir: &Path) -> Vec<std::fs::DirEntry> {
        self.entries(dir)
            .into_iter()
            .filter(|e| e.file_type().is_ok_and(|ft| ft.is_dir()))
            .collect()
    }

    /// `dir`'s regular files whose name satisfies `named`.
    fn files(&mut self, dir: &Path, named: impl Fn(&str) -> bool) -> Vec<std::fs::DirEntry> {
        self.entries(dir)
            .into_iter()
            .filter(|e| e.file_type().is_ok_and(|ft| ft.is_file()))
            .filter(|e| e.file_name().to_str().is_some_and(&named))
            .collect()
    }

    /// Count one session file, from metadata only.
    fn note(&mut self, meta: Option<std::fs::Metadata>) {
        self.count += 1;
        if let Some(modified) = meta.and_then(|m| m.modified().ok()) {
            let stamp: DateTime<Utc> = modified.into();
            self.most_recent = Some(match self.most_recent {
                Some(current) if current >= stamp => current,
                _ => stamp,
            });
        }
    }
}

/// `<encoded-cwd>/<uuid>.jsonl`. The stem must be a hyphenated UUID, as
/// Claude Code names every top-level session: a `.jsonl` file two levels
/// down is otherwise too common a shape to call a Claude Code store.
fn claude_code_layout(root: &Path) -> Tally {
    let is_session = |name: &str| {
        name.strip_suffix(JSONL_SUFFIX)
            .is_some_and(|stem| stem.len() == 36 && uuid::Uuid::try_parse(stem).is_ok())
    };
    let mut tally = Tally::new();
    for project in tally.subdirs(root) {
        for file in tally.files(&project.path(), is_session) {
            tally.note(file.metadata().ok());
        }
    }
    tally
}

/// `YYYY/MM/DD/rollout-*.jsonl`, the date directories all digits.
fn codex_layout(root: &Path) -> Tally {
    fn digits(entry: &std::fs::DirEntry, width: usize) -> bool {
        entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.len() == width && n.bytes().all(|b| b.is_ascii_digit()))
    }
    let mut tally = Tally::new();
    for year in tally.subdirs(root) {
        if !digits(&year, 4) {
            continue;
        }
        for month in tally.subdirs(&year.path()) {
            if !digits(&month, 2) {
                continue;
            }
            for day in tally.subdirs(&month.path()) {
                if !digits(&day, 2) {
                    continue;
                }
                for file in tally.files(&day.path(), super::codex::is_rollout_file_name) {
                    tally.note(file.metadata().ok());
                }
            }
        }
    }
    tally
}

/// `<project>/chats/session-*.json`.
fn gemini_layout(root: &Path) -> Tally {
    let mut tally = Tally::new();
    for project in tally.subdirs(root) {
        let chats = project.path().join(super::gemini_cli::CHATS_DIR);
        if !std::fs::symlink_metadata(&chats).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        for file in tally.files(&chats, super::gemini_cli::is_session_file_name) {
            tally.note(file.metadata().ok());
        }
    }
    tally
}

/// `<id>/<id>.messages.json`: one stat per session directory, no listing.
fn cline_layout(root: &Path) -> Tally {
    let mut tally = Tally::new();
    for session in tally.subdirs(root) {
        let Some(messages) = super::cline::messages_file_for(&session.path()) else {
            continue;
        };
        if let Ok(meta) = std::fs::symlink_metadata(&messages)
            && meta.is_file()
        {
            tally.note(Some(meta));
        }
    }
    tally
}

fn describe(
    source: &str,
    path: PathBuf,
    relocated_by_env: bool,
    suffix: &str,
    walk: Walk,
) -> SourceCandidate {
    let (exists, session_count, most_recent) = if path.is_dir() {
        let (count, recent) = count_sessions(&path, suffix, walk);
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
        answers_at: source_answers_at(source).map(str::to_string),
    }
}

/// Count files whose name ends in `suffix` under `root`, and note the most
/// recent mtime. A suffix rather than an extension, because one store's
/// session file is told apart from its sibling manifest by more than the
/// part after the last dot.
///
/// Walks with an explicit stack rather than recursion, and follows no
/// symlinks: a symlinked directory could point anywhere, and this is a
/// counting pass whose whole justification is that it stays inside the store
/// it is describing.
fn count_sessions(root: &Path, suffix: &str, walk: Walk) -> (u64, Option<DateTime<Utc>>) {
    let mut count = 0_u64;
    let mut most_recent: Option<DateTime<Utc>> = None;
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for (index, entry) in entries.enumerate() {
            if let Walk::Flat { entry_budget } = walk
                && index >= entry_budget
            {
                break;
            }
            let Ok(entry) = entry else {
                continue;
            };
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                if matches!(walk, Walk::Recursive) {
                    stack.push(path);
                }
                continue;
            }
            let is_session = entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(suffix));
            if !is_session {
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
    fn probes_the_gemini_store_and_counts_its_json_sessions() {
        let home = Scratch::new("gemini");
        let chats = home.path().join(".gemini/tmp/proj/chats");
        write_session(&chats, "session-a.json");
        write_session(&chats, "session-b.json");
        // The other two stores count `.jsonl`; this one counts `.json`, so
        // a `.jsonl` sitting beside a session must not be counted here.
        write_session(&chats, "session-c.jsonl");

        let found = probe(home.path(), no_env);
        let gemini = found
            .iter()
            .find(|c| c.source == SOURCE_GEMINI_CLI)
            .expect("the roots screen must be able to ask about gemini too");
        assert_eq!(gemini.path, home.path().join(".gemini/tmp"));
        assert!(gemini.exists);
        assert_eq!(gemini.session_count, 2);
        assert!(gemini.most_recent.is_some());
        assert!(!gemini.relocated_by_env);
    }

    #[test]
    fn the_gemini_home_variable_relocates_the_store_and_says_so() {
        let home = Scratch::new("gemini-env-home");
        let elsewhere = Scratch::new("gemini-env-target");
        write_session(&elsewhere.path().join("tmp/proj/chats"), "session-a.json");

        let target = elsewhere.path().to_str().unwrap().to_string();
        let found = probe(home.path(), |key| {
            (key == GEMINI_CLI_HOME_ENV).then(|| target.clone())
        });
        let gemini = found
            .iter()
            .find(|c| c.source == SOURCE_GEMINI_CLI)
            .unwrap();
        assert_eq!(gemini.path, elsewhere.path().join("tmp"));
        assert!(gemini.relocated_by_env);
        assert_eq!(gemini.session_count, 1);
    }

    #[test]
    fn probes_the_cline_store_fourth_and_counts_its_sessions() {
        let home = Scratch::new("cline");
        let session = home.path().join(".cline/data/sessions/1756900000000_k3x9q");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("1756900000000_k3x9q.messages.json"), "{}").unwrap();
        std::fs::write(session.join("1756900000000_k3x9q.json"), "{}").unwrap();
        let found = probe(home.path(), no_env);
        assert_eq!(
            found[3].source, SOURCE_CLINE,
            "appended, so older shells indexing by position are unaffected"
        );
        assert_eq!(found[3].path, home.path().join(".cline/data/sessions"));
        assert!(found[3].exists);
        assert_eq!(found[3].session_count, 1, "the manifest is not a session");
        assert!(found[3].most_recent.is_some());
        assert!(!found[3].relocated_by_env);
    }

    #[test]
    fn a_cline_environment_variable_relocates_the_store_and_says_so() {
        let home = Scratch::new("cline-env");
        let found = probe(home.path(), |k| {
            (k == CLINE_SESSION_DATA_DIR_ENV).then(|| "/elsewhere".to_string())
        });
        assert_eq!(found[3].path, PathBuf::from("/elsewhere"));
        assert!(found[3].relocated_by_env);
    }

    /// The first two rows are what every shell written before this source
    /// existed indexes by position.
    #[test]
    fn the_existing_rows_keep_their_positions() {
        let home = Scratch::new("ordering");
        let found = probe(home.path(), no_env);
        assert_eq!(found[0].source, SOURCE_CLAUDE_CODE);
        assert_eq!(found[1].source, SOURCE_CODEX);
        assert_eq!(found[2].source, SOURCE_GEMINI_CLI);
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

    #[test]
    fn opencode_is_never_in_the_blind_probe() {
        // OpenCode has no conventional location to guess at, unlike every
        // other row here -- see `describe_opencode` for the folder a
        // contributor has actually named.
        let home = Scratch::new("opencode-blind");
        let found = probe(home.path(), no_env);
        assert!(!found.iter().any(|c| c.source == SOURCE_OPENCODE));
    }

    #[test]
    fn opencode_is_discovered_once_a_folder_is_named() {
        let exports = Scratch::new("opencode-exports");
        write_session(exports.path(), "ses_a.json");
        write_session(exports.path(), "ses_b.json");

        let candidate = describe_opencode(exports.path());
        assert_eq!(candidate.source, SOURCE_OPENCODE);
        assert!(candidate.exists);
        assert_eq!(candidate.session_count, 2);
        assert!(!candidate.relocated_by_env);
    }

    #[test]
    fn an_opencode_folder_is_counted_flat_like_the_adapter_reads_it() {
        // `source::opencode` reads the declared folder non-recursively, so a
        // nested `.json` must not show up in "N sessions found".
        let exports = Scratch::new("opencode-flat");
        write_session(exports.path(), "ses_a.json");
        let nested = exports.path().join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        write_session(&nested, "ses_b.json");

        assert_eq!(describe_opencode(exports.path()).session_count, 1);
    }

    #[test]
    fn an_opencode_folder_count_stops_at_the_adapters_entry_budget() {
        // A shell that passes a huge folder (say `$HOME` by mistake) gets a
        // bounded read on the calling thread, not a full listing.
        let exports = Scratch::new("opencode-budget");
        let budget = crate::source::opencode::DISCOVERY_ENTRY_BUDGET;
        for i in 0..budget + 10 {
            write_session(exports.path(), &format!("ses_{i}.json"));
        }

        let counted = describe_opencode(exports.path()).session_count;
        assert_eq!(counted, budget as u64);
    }

    #[test]
    fn a_folder_named_for_opencode_that_is_not_there_is_reported_absent() {
        let home = Scratch::new("opencode-missing");
        let candidate = describe_opencode(&home.path().join("never-created"));
        assert!(!candidate.exists);
        assert_eq!(candidate.session_count, 0);
        assert_eq!(candidate.most_recent, None);
    }

    #[test]
    fn each_tools_answers_at_label_is_a_fixed_word_or_absent() {
        let home = Scratch::new("answers-at");
        let found = probe(home.path(), no_env);

        let by_source = |name: &str| {
            found
                .iter()
                .find(|c| c.source == name)
                .unwrap_or_else(|| panic!("probe must report {name}"))
                .answers_at
                .clone()
        };
        assert_eq!(by_source(SOURCE_CLAUDE_CODE), Some("Anthropic".to_string()));
        assert_eq!(by_source(SOURCE_CODEX), Some("OpenAI".to_string()));
        assert_eq!(by_source(SOURCE_GEMINI_CLI), Some("Google".to_string()));
        assert_eq!(
            by_source(SOURCE_CLINE),
            None,
            "Cline ships with no single default vendor to name"
        );

        assert_eq!(
            describe_opencode(home.path()).answers_at,
            None,
            "OpenCode ships with no single default vendor to name"
        );
    }

    const A_UUID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

    fn sources_of(found: &[SourceCandidate]) -> Vec<&str> {
        found.iter().map(|c| c.source.as_str()).collect()
    }

    fn only(found: &[SourceCandidate], source: &str) -> SourceCandidate {
        assert_eq!(sources_of(found), vec![source], "exactly one kind expected");
        found[0].clone()
    }

    #[test]
    fn describe_folder_recognises_a_claude_code_store() {
        let picked = Scratch::new("folder-claude");
        let project = picked.path().join("-Users-someone-code-app");
        write_session(&project, &format!("{A_UUID}.jsonl"));
        write_session(&project, "0e1d2c3b-4a59-4687-9b0a-1b2c3d4e5f60.jsonl");

        let row = only(&describe_folder(picked.path()), SOURCE_CLAUDE_CODE);
        assert_eq!(row.path, picked.path());
        assert!(row.exists);
        assert_eq!(row.session_count, 2);
        assert!(row.most_recent.is_some());
        assert!(!row.relocated_by_env);
        assert_eq!(row.answers_at, Some("Anthropic".to_string()));
    }

    #[test]
    fn describe_folder_recognises_a_codex_store() {
        let picked = Scratch::new("folder-codex");
        let day = picked.path().join("2026/08/20");
        write_session(&day, "rollout-2026-08-20T10-00-00-abc.jsonl");
        write_session(&day, "rollout-2026-08-20T11-00-00-def.jsonl");
        write_session(&picked.path().join("2026/08/21"), "rollout-x.jsonl");

        let row = only(&describe_folder(picked.path()), SOURCE_CODEX);
        assert_eq!(row.path, picked.path());
        assert_eq!(row.session_count, 3);
    }

    #[test]
    fn describe_folder_recognises_a_gemini_store() {
        let picked = Scratch::new("folder-gemini");
        let chats = picked.path().join("proj/chats");
        write_session(&chats, "session-a.json");
        write_session(&chats, "session-b.json");

        let row = only(&describe_folder(picked.path()), SOURCE_GEMINI_CLI);
        assert_eq!(row.session_count, 2);
    }

    #[test]
    fn describe_folder_recognises_a_cline_store() {
        let picked = Scratch::new("folder-cline");
        let session = picked.path().join("1767000000000");
        write_session(&session, "1767000000000.messages.json");
        write_session(&session, "1767000000000.json");

        let row = only(&describe_folder(picked.path()), SOURCE_CLINE);
        assert_eq!(row.session_count, 1);
        assert_eq!(row.answers_at, None);
    }

    #[test]
    fn describe_folder_recognises_a_trajectory_jsonl_folder() {
        let picked = Scratch::new("folder-trajectory");
        write_session(picked.path(), "run-1.jsonl");
        write_session(picked.path(), "run-2.jsonl");

        let row = only(&describe_folder(picked.path()), SOURCE_TRAJECTORY);
        assert_eq!(row.session_count, 2);
    }

    #[test]
    fn describe_folder_reports_both_kinds_for_a_flat_json_folder() {
        // OpenCode exports and Letta trajectory exports are both flat
        // `.json`; the layout cannot tell them apart and must not guess.
        let picked = Scratch::new("folder-flat-json");
        write_session(picked.path(), "ses_a.json");
        write_session(picked.path(), "ses_b.json");

        let found = describe_folder(picked.path());
        assert_eq!(sources_of(&found), vec![SOURCE_OPENCODE, SOURCE_TRAJECTORY]);
        assert!(found.iter().all(|c| c.session_count == 2));
    }

    #[test]
    fn describe_folder_reports_nothing_for_an_unrelated_folder() {
        let picked = Scratch::new("folder-unrelated");
        std::fs::write(picked.path().join("notes.txt"), b"x").unwrap();
        std::fs::write(picked.path().join("photo.png"), b"x").unwrap();
        // Near misses for every layout.
        write_session(&picked.path().join("exports"), "data.jsonl");
        write_session(&picked.path().join("a/b"), "deep.jsonl");
        write_session(&picked.path().join("2026/08/20"), "not-a-rollout.jsonl");
        write_session(&picked.path().join("proj/chats"), "chat.json");
        write_session(&picked.path().join("abc"), "other.messages.json");
        std::fs::create_dir_all(picked.path().join("empty")).unwrap();

        assert_eq!(describe_folder(picked.path()), Vec::new());

        let empty = Scratch::new("folder-empty");
        assert_eq!(describe_folder(empty.path()), Vec::new());
        assert_eq!(
            describe_folder(&empty.path().join("never-created")),
            Vec::new()
        );
    }

    #[cfg(unix)]
    #[test]
    fn describe_folder_never_opens_a_file() {
        use std::os::unix::fs::PermissionsExt;
        // Mode 000 on every session file: any attempt to open one fails, so
        // a recognition that read contents would miss them. Vacuous when the
        // tests run as root, who can open anything.
        let picked = Scratch::new("folder-unreadable");
        let project = picked.path().join("-Users-someone-code-app");
        let session = project.join(format!("{A_UUID}.jsonl"));
        write_session(&project, &format!("{A_UUID}.jsonl"));
        let flat = picked.path().join("loose.json");
        std::fs::write(&flat, b"{}").unwrap();
        for file in [&session, &flat] {
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o000)).unwrap();
        }

        let found = describe_folder(picked.path());
        assert_eq!(
            sources_of(&found),
            vec![SOURCE_CLAUDE_CODE, SOURCE_OPENCODE, SOURCE_TRAJECTORY]
        );
        assert!(found.iter().all(|c| c.session_count == 1));
        assert!(found.iter().all(|c| c.most_recent.is_some()));

        for file in [&session, &flat] {
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn describe_folder_follows_no_symlink() {
        let elsewhere = Scratch::new("folder-link-target");
        write_session(&elsewhere.path().join("2026/08/20"), "rollout-x.jsonl");
        let picked = Scratch::new("folder-link");
        std::os::unix::fs::symlink(elsewhere.path().join("2026"), picked.path().join("2026"))
            .unwrap();

        assert_eq!(describe_folder(picked.path()), Vec::new());
    }
}
