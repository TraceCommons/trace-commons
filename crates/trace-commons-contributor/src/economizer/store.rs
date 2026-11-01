//! The economizer's local state, owner-only, under `<config dir>/economizer`:
//!
//! - `enabled`: present when the person turned the economizer on;
//! - `repos/<key>.json`: one folder's [`RepoState`] for one harness, where
//!   `<key>` is a digest of both ([`repo_key`]), never the path;
//! - `sessions/<id>.json`: one running session's [`SessionState`], folded
//!   into its folder's state when the session ends, then removed;
//! - `repos/<key>.lock`, `sessions/<id>.lock`: empty lock files. A
//!   session's is removed only by the stale prune, never while a hook may
//!   hold it; a folder's stays beside its state.
//!
//! Unreadable or foreign state reads as empty: the economizer then does
//! less, never something else. Every change to a folder's state is made
//! under that folder's lock ([`Store::try_update_repo`]), so two sessions
//! ending at once, or a Give back, never lose each other's change.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::BTreeSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::Harness;
use super::facts::{FactBook, Observations};
use super::loop_guard::{Envelope, TurnState};
use super::signals::short_digest;
use super::tools::ToolLedger;
use crate::managed::profiles::{private_directory, read_regular_file};

pub const DIRECTORY: &str = "economizer";
const ENABLED: &str = "enabled";
const REPOS: &str = "repos";
const SESSIONS: &str = "sessions";
const SCHEMA: u32 = 1;
/// The largest state file read back.
const MAX_FILE: u64 = 1024 * 1024;
/// A session file left behind (a crash, a killed terminal) is removed after
/// this long.
pub const STALE_SESSION: chrono::Duration = chrono::Duration::days(7);

/// Another writer held a lock past the wait. The only lock error that
/// trying again can fix.
#[derive(Debug, thiserror::Error)]
#[error("the economizer state is busy")]
pub struct StateBusy;

/// What the economizer knows about one folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoState {
    pub schema: u32,
    #[serde(default)]
    pub tools: ToolLedger,
    #[serde(default)]
    pub facts: FactBook,
    #[serde(default)]
    pub envelope: Envelope,
    /// Sessions folded into this state.
    #[serde(default)]
    pub sessions: u32,
}

impl Default for RepoState {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            tools: ToolLedger::default(),
            facts: FactBook::default(),
            envelope: Envelope::default(),
            sessions: 0,
        }
    }
}

/// One running session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub schema: u32,
    /// The folder's [`repo_key`].
    pub repo: String,
    pub started_at: DateTime<Utc>,
    #[serde(default)]
    pub prompts: u32,
    #[serde(default)]
    pub used_tools: BTreeSet<String>,
    /// The current turn.
    #[serde(default)]
    pub turn: TurnState,
    #[serde(default)]
    pub observations: Observations,
}

impl SessionState {
    pub fn new(repo: &str, now: DateTime<Utc>) -> Self {
        Self {
            schema: SCHEMA,
            repo: repo.to_owned(),
            started_at: now,
            prompts: 0,
            used_tools: BTreeSet::new(),
            turn: TurnState::default(),
            observations: Observations::default(),
        }
    }
}

/// The key one harness's state for a folder is kept under: a digest of the
/// harness and the folder's canonical path.
pub fn repo_key(cwd: &Path, harness: Harness) -> String {
    let path = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    short_digest(
        "repo",
        &format!("{}\0{}", harness.as_str(), path.to_string_lossy()),
    )
}

/// Whether `id` can name a file: a session id or a [`repo_key`].
fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// The economizer's directory.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// The store inside a contributor config directory.
    pub fn new(config_dir: &Path) -> Self {
        Self::at(config_dir.join(DIRECTORY))
    }

    /// The store at `root` itself, as the hook command is given it.
    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn enabled(&self) -> bool {
        std::fs::symlink_metadata(self.root.join(ENABLED)).is_ok_and(|meta| meta.is_file())
    }

    pub fn set_enabled(&self, on: bool) -> Result<()> {
        let path = self.root.join(ENABLED);
        if on {
            self.directory(None)?;
            crate::config::write_atomic_0600(&self.root, &path, b"1\n")
        } else {
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e).context("turning the economizer off"),
            }
        }
    }

    pub fn repo(&self, key: &str) -> RepoState {
        self.read(REPOS, key)
            .filter(|state: &RepoState| state.schema == SCHEMA)
            .unwrap_or_default()
    }

    pub fn save_repo(&self, key: &str, state: &RepoState) -> Result<()> {
        self.write(REPOS, key, state)
    }

    /// Whether `key` has a state file.
    pub fn has_repo(&self, key: &str) -> bool {
        safe_id(key)
            && std::fs::symlink_metadata(self.root.join(REPOS).join(format!("{key}.json")))
                .is_ok_and(|meta| meta.is_file())
    }

    /// Read, change and write one folder's state under an exclusive lock
    /// on `repos/<key>.lock`, so two writers (a session ending, a Give
    /// back) never lose each other's change. Returns what `change`
    /// returns; the state is written only when `change` says so.
    ///
    /// Gives up after `wait` if another writer holds the lock: nothing is
    /// written and an error is returned, so a stalled writer (an agent
    /// suspended mid-write) never holds the hook or a daemon thread for
    /// good, and giving up never undoes another writer's change.
    pub fn try_update_repo<T>(
        &self,
        key: &str,
        wait: Duration,
        change: impl FnOnce(&mut RepoState) -> (bool, T),
    ) -> Result<T> {
        let lock = self.lock(REPOS, key, wait)?;
        let mut state = self.repo(key);
        let (write, out) = change(&mut state);
        if write {
            self.save_repo(key, &state)?;
        }
        // Dropping the file releases the lock.
        drop(lock);
        Ok(out)
    }

    /// Whether `id` has a session file, readable or not.
    pub fn has_session(&self, id: &str) -> bool {
        safe_id(id)
            && std::fs::symlink_metadata(self.root.join(SESSIONS).join(format!("{id}.json")))
                .is_ok_and(|meta| meta.is_file())
    }

    pub fn session(&self, id: &str) -> Option<SessionState> {
        self.read(SESSIONS, id)
            .filter(|state: &SessionState| state.schema == SCHEMA)
    }

    pub fn save_session(&self, id: &str, state: &SessionState) -> Result<()> {
        self.write(SESSIONS, id, state)
    }

    /// Remove a session's state. Its lock file stays: a hook of the same
    /// session may hold it or be waiting on it, and a new file in its
    /// place would let two hooks run at once. The stale prune removes it.
    pub fn remove_session(&self, id: &str) {
        if safe_id(id) {
            let _ = std::fs::remove_file(self.root.join(SESSIONS).join(format!("{id}.json")));
        }
    }

    /// An exclusive lock on one session's state, held until the returned
    /// file is dropped. The coding tool runs the hooks of parallel tool
    /// calls as concurrent processes; each holds this lock from reading the
    /// session to writing it back, so none loses another's update. Gives
    /// up after `wait`, so a stalled holder never holds the agent.
    pub fn lock_session(&self, id: &str, wait: Duration) -> Result<File> {
        self.lock(SESSIONS, id, wait)
    }

    /// An exclusive lock on `<sub>/<id>.lock`, made owner-only if missing.
    /// Gives up once `wait` has passed. Every wait is bounded.
    fn lock(&self, sub: &str, id: &str, wait: Duration) -> Result<File> {
        anyhow::ensure!(safe_id(id), "not a usable economizer id");
        let dir = self.directory(Some(sub))?;
        let path = dir.join(format!("{id}.lock"));
        if std::fs::symlink_metadata(&path)
            .is_ok_and(|meta| !meta.is_file() || meta.file_type().is_symlink())
        {
            anyhow::bail!("economizer lock is not a regular file");
        }
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path).context("opening the economizer lock")?;
        let deadline = std::time::Instant::now() + wait;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(std::fs::TryLockError::WouldBlock) => return Err(StateBusy.into()),
                Err(std::fs::TryLockError::Error(e)) => {
                    return Err(e).context("locking the economizer state");
                }
            }
        }
    }

    /// Remove session files and lock files not written for
    /// [`STALE_SESSION`]: idle ones, never a long-running live session,
    /// whose file every event rewrites. A lock beside a live session file
    /// stays.
    pub fn prune_sessions(&self, now: DateTime<Utc>) {
        let Ok(entries) = std::fs::read_dir(self.root.join(SESSIONS)) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if !safe_id(id) {
                continue;
            }
            let idle = |path: &Path| {
                std::fs::metadata(path)
                    .and_then(|meta| meta.modified())
                    .ok()
                    .map(DateTime::<Utc>::from)
                    .is_some_and(|modified| now - modified > STALE_SESSION)
            };
            match path.extension().and_then(|ext| ext.to_str()) {
                Some("json") if idle(&path) => {
                    let _ = std::fs::remove_file(&path);
                    let _ = std::fs::remove_file(path.with_extension("lock"));
                }
                Some("lock") if idle(&path) && !path.with_extension("json").exists() => {
                    let _ = std::fs::remove_file(&path);
                }
                _ => {}
            }
        }
    }

    fn directory(&self, sub: Option<&str>) -> Result<PathBuf> {
        private_directory(&self.root).context("creating the economizer directory")?;
        let Some(sub) = sub else {
            return Ok(self.root.clone());
        };
        let dir = self.root.join(sub);
        private_directory(&dir).context("creating an economizer directory")?;
        Ok(dir)
    }

    fn read<T: DeserializeOwned>(&self, sub: &str, id: &str) -> Option<T> {
        if !safe_id(id) {
            return None;
        }
        let bytes = read_regular_file(&self.root.join(sub).join(format!("{id}.json")), MAX_FILE)?;
        serde_json::from_slice(&bytes).ok()
    }

    fn write<T: Serialize>(&self, sub: &str, id: &str, value: &T) -> Result<()> {
        anyhow::ensure!(safe_id(id), "not a usable economizer id");
        let dir = self.directory(Some(sub))?;
        let body = serde_json::to_vec(value)?;
        anyhow::ensure!(body.len() as u64 <= MAX_FILE, "economizer state too large");
        crate::config::write_atomic_0600(&dir, &dir.join(format!("{id}.json")), &body)
    }
}
