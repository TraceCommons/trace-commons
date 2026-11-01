//! What the economizer learned, per folder and tool, for a shell to show
//! (the daemon's `economizer_status`; macOS Insights' Economizer tab).
//!
//! The economizer's store is keyed by a digest of the folder's path
//! ([`repo_key`]), so it cannot list its own folders. The caller names the
//! folders it knows (managed session rows and remembered folder settings),
//! and this reads each one's state. A row carries the folder's last path
//! component as its label and the store key as its reference, never the
//! path itself.

use serde::Serialize;
use std::collections::BTreeSet;
use std::path::PathBuf;

use super::Harness;
use super::store::{RepoState, Store, repo_key};
use super::tools::Thresholds;

/// The most folders one answer carries, most sessions first.
pub const MAX_FOLDERS: usize = 100;

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
    /// The folder's last path component, as managed session rows show it.
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
    fn of(folder: String, harness: Harness, key: String, state: &RepoState) -> Self {
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
/// by label and tool.
pub fn summarize(store: &Store, folders: impl IntoIterator<Item = (Harness, PathBuf)>) -> Summary {
    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();
    for (harness, path) in folders {
        let key = repo_key(&path, harness);
        if !seen.insert(key.clone()) {
            continue;
        }
        let label = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let row = FolderSummary::of(label, harness, key.clone(), &store.repo(&key));
        if row.learned_anything() {
            rows.push(row);
        }
    }
    rows.sort_by(|a, b| {
        b.sessions
            .cmp(&a.sessions)
            .then_with(|| a.folder.cmp(&b.folder))
            .then_with(|| a.harness.as_str().cmp(b.harness.as_str()))
    });
    let truncated = rows.len().saturating_sub(MAX_FOLDERS) as u32;
    rows.truncate(MAX_FOLDERS);
    Summary {
        enabled: store.enabled(),
        folders: rows,
        folders_truncated: truncated,
    }
}

/// Give back every tool left out under `key` for `harness`, for good.
/// Returns the tools given back; none when nothing was left out.
pub fn restore(store: &Store, harness: Harness, key: &str) -> anyhow::Result<Vec<String>> {
    let mut state = store.repo(key);
    let given = state.tools.restore(harness, &Thresholds::default());
    if !given.is_empty() {
        store.save_repo(key, &state)?;
    }
    Ok(given)
}
