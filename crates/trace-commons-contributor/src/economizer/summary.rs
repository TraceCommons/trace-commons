//! What the economizer learned, per folder and tool, for a shell to show
//! (the daemon's `economizer_status`; macOS Insights' Economizer tab).
//!
//! The economizer's store is keyed by a digest of the folder's path
//! ([`repo_key`]), so it cannot list its own folders. The caller names the
//! folders it knows (managed session rows and remembered folder settings),
//! and this reads each one's state. A row carries a short label (the
//! folder's last path component, with parents only when two labels would
//! match; see [`summarize`]) and the store key as its reference, never the
//! path itself.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::Harness;
use super::store::{RepoState, Store, repo_key, safe_id};
use super::tools::Thresholds;

/// The most folders one answer carries, most sessions first.
pub const MAX_FOLDERS: usize = 100;
/// How long Give back waits for a folder's lock before refusing.
const RESTORE_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// One command the economizer tells the agent: run `program` as
/// `prefix program`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToldCommand {
    pub program: String,
    pub prefix: String,
}

/// What one tool's sessions taught the economizer about one folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderSummary {
    /// The folder's last path component, as managed session rows show it,
    /// or as many of its last components as tell it apart from another
    /// row for the same tool, with the home folder as `~`. Never the
    /// full path.
    pub folder: String,
    pub harness: Harness,
    /// The store key, to name this row in `economizer_restore`.
    pub key: String,
    /// Sessions folded into this folder's state.
    pub sessions: u32,
    /// Built-in tools left out of the next sessions, because they went
    /// unused here.
    pub left_out: Vec<String>,
    /// Tools the person gave back here; never left out again.
    pub given_back: Vec<String>,
    /// Commands the agent is told at a session's start.
    pub told: Vec<ToldCommand>,
    /// Commands seen working another way, not yet confirmed enough to tell.
    pub forming: u32,
}

impl FolderSummary {
    /// One folder's row from its state; `near-ai economizer status` and
    /// [`summarize`] both read it.
    pub fn of(folder: String, harness: Harness, key: String, state: &RepoState) -> Self {
        let given_back = state
            .tools
            .tools
            .iter()
            .filter(|(_, evidence)| evidence.restored)
            .map(|(name, _)| name.clone())
            .collect();
        Self {
            folder,
            harness,
            key,
            sessions: state.sessions,
            left_out: state.tools.deny_list(harness, &Thresholds::default()),
            given_back,
            told: state
                .facts
                .told()
                .map(|fact| ToldCommand {
                    program: fact.program.clone(),
                    prefix: fact.prefix.clone(),
                })
                .collect(),
            forming: state.facts.facts.iter().filter(|f| !f.told()).count() as u32,
        }
    }

    /// Whether there is anything to show: a folder no session was folded
    /// into, with nothing left out or told, is left out of the answer.
    fn learned_anything(&self) -> bool {
        self.sessions > 0 || !self.left_out.is_empty() || !self.told.is_empty()
    }
}

/// The whole answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub enabled: bool,
    pub folders: Vec<FolderSummary>,
    /// Folders left out past [`MAX_FOLDERS`].
    pub folders_truncated: u32,
}

/// What the economizer learned about each of `folders`, as (tool, path)
/// pairs. A folder named twice for one tool appears once; one with
/// nothing learned does not appear. Sorted by sessions, most first, then
/// by label, tool and key, so the order is the same on every read.
///
/// Each row is labelled with the folder's last path component. When two
/// rows for one tool would share a label, both take parent components
/// until they differ (`work/api`, `oss/api`), never the full path, and the
/// home folder is written `~`, never its name.
pub fn summarize(store: &Store, folders: impl IntoIterator<Item = (Harness, PathBuf)>) -> Summary {
    summarize_in(store, folders, dirs::home_dir().as_deref())
}

/// [`summarize`] with the home folder given, for tests.
pub fn summarize_in(
    store: &Store,
    folders: impl IntoIterator<Item = (Harness, PathBuf)>,
    home: Option<&Path>,
) -> Summary {
    // Home as given and as resolved, so a folder recorded under either
    // spelling (a symlinked home, `/private/var` on macOS) is seen as
    // under home.
    let homes: Vec<PathBuf> = home
        .into_iter()
        .flat_map(|home| {
            let resolved = std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
            [home.to_path_buf(), resolved]
        })
        .collect();
    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();
    for (harness, path) in folders {
        let key = repo_key(&path, harness);
        if !seen.insert(key.clone()) {
            continue;
        }
        // Labelled from the resolved path, so a folder recorded under a
        // symlinked spelling of home is still seen as under home.
        let resolved = std::fs::canonicalize(&path).unwrap_or(path);
        let row = FolderSummary::of(
            label(&resolved, 0, &homes),
            harness,
            key.clone(),
            &store.repo(&key),
        );
        if row.learned_anything() {
            rows.push((row, resolved));
        }
    }
    // Lengthen colliding labels, one parent at a time, until they differ
    // or the paths run out.
    let mut depths = vec![0usize; rows.len()];
    loop {
        let mut groups: BTreeMap<(String, &'static str), Vec<usize>> = BTreeMap::new();
        for (i, (row, _)) in rows.iter().enumerate() {
            groups
                .entry((row.folder.clone(), row.harness.as_str()))
                .or_default()
                .push(i);
        }
        let mut lengthened = false;
        for members in groups.into_values().filter(|m| m.len() > 1) {
            for i in members {
                let longer = label(&rows[i].1, depths[i] + 1, &homes);
                if longer != rows[i].0.folder {
                    depths[i] += 1;
                    rows[i].0.folder = longer;
                    lengthened = true;
                }
            }
        }
        if !lengthened {
            break;
        }
    }
    let mut rows: Vec<FolderSummary> = rows.into_iter().map(|(row, _)| row).collect();
    rows.sort_by(|a, b| {
        b.sessions
            .cmp(&a.sessions)
            .then_with(|| a.folder.cmp(&b.folder))
            .then_with(|| a.harness.as_str().cmp(b.harness.as_str()))
            .then_with(|| a.key.cmp(&b.key))
    });
    let truncated = rows.len().saturating_sub(MAX_FOLDERS) as u32;
    rows.truncate(MAX_FOLDERS);
    Summary {
        enabled: store.enabled(),
        folders: rows,
        folders_truncated: truncated,
    }
}

/// Whether two spellings name the same folder: equal, or on macOS, whose
/// default volume ignores case, equal ignoring case.
fn same_folder(a: &Path, b: &Path) -> bool {
    a == b
        || (cfg!(target_os = "macos")
            && a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase())
}

/// `path`'s last `depth + 1` components, joined with `/`. The home folder
/// is `~` and ends the label, so its name never shows.
fn label(path: &Path, depth: usize, homes: &[PathBuf]) -> String {
    let mut parts = Vec::new();
    let mut current = Some(path);
    for _ in 0..=depth {
        let Some(here) = current else { break };
        if homes.iter().any(|home| same_folder(home, here)) {
            parts.push("~".to_owned());
            break;
        }
        let Some(name) = here.file_name() else { break };
        parts.push(name.to_string_lossy().into_owned());
        current = here.parent();
    }
    parts.reverse();
    parts.join("/")
}

/// Give back every tool left out under `key` for `harness`, for good.
/// Returns the tools given back; none when nothing was left out.
///
/// Written under the folder's lock ([`Store::try_update_repo`]), so a session
/// ending in that folder at the same moment cannot undo it.
pub fn restore(store: &Store, harness: Harness, key: &str) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(safe_id(key), "not a usable economizer id");
    // Nothing is known about a folder with no state, so nothing is left
    // out there; no lock file is made for it.
    if !store.has_repo(key) {
        return Ok(Vec::new());
    }
    // Generous, but bounded: a hook stopped while holding the lock (the
    // agent suspended mid-write) must not hold a daemon thread for good.
    store.try_update_repo(key, RESTORE_LOCK_WAIT, |state| {
        let given = state.tools.restore(harness, &Thresholds::default());
        (!given.is_empty(), given)
    })
}
