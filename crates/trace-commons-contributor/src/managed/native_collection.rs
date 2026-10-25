//! Explicit post-exit collection from isolated Claude Code and Codex profiles.
//! Existing local transcripts are never swept into a later opted-in launch.

use super::{
    AccountId, ManagedError, ToolId,
    profiles::{NativeProfiles, private_directory},
};
use crate::{
    config::ConfigStore,
    source::{self, TraceSource},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

pub const STAGING: &str = "managed-native-exports";

pub struct NativeCollection {
    tool: ToolId,
    root: PathBuf,
    before: Option<BTreeSet<PathBuf>>,
    id: Uuid,
    codex: Option<super::session_binding::CodexBinding>,
}

fn tool_directory(tool: ToolId) -> Result<&'static str, ManagedError> {
    match tool {
        ToolId::Claude => Ok("claude"),
        ToolId::Codex => Ok("codex"),
        _ => Err(ManagedError::InvalidRequest),
    }
}

fn suffix(tool: ToolId) -> Result<&'static str, ManagedError> {
    match tool {
        ToolId::Claude => Ok("projects"),
        ToolId::Codex => Ok("sessions"),
        _ => Err(ManagedError::InvalidRequest),
    }
}

fn files(root: &Path) -> Result<BTreeSet<PathBuf>, ManagedError> {
    fn visit(root: &Path, path: &Path, found: &mut BTreeSet<PathBuf>) -> Result<(), ManagedError> {
        let meta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(ManagedError::StorageUnavailable),
        };
        if meta.file_type().is_symlink() {
            return Ok(());
        }
        if meta.is_dir() {
            for entry in std::fs::read_dir(path).map_err(|_| ManagedError::StorageUnavailable)? {
                visit(
                    root,
                    &entry.map_err(|_| ManagedError::StorageUnavailable)?.path(),
                    found,
                )?;
            }
        } else if meta.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
            found.insert(
                path.strip_prefix(root)
                    .map_err(|_| ManagedError::InvalidRequest)?
                    .to_path_buf(),
            );
        }
        Ok(())
    }
    let mut found = BTreeSet::new();
    visit(root, root, &mut found)?;
    Ok(found)
}

impl NativeCollection {
    /// Taken immediately before spawning the native coding process, only when
    /// this launch explicitly opted in. No transcript content is read here.
    pub fn snapshot(
        store: &ConfigStore,
        tool: ToolId,
        account: AccountId,
        id: Uuid,
    ) -> Result<Self, ManagedError> {
        let root = NativeProfiles::new(store)
            .profile_root(account)?
            .join(suffix(tool)?);
        let usable = !std::fs::symlink_metadata(&root)
            .is_ok_and(|meta| !meta.is_dir() || meta.file_type().is_symlink());
        let before = usable.then(|| files(&root).ok()).flatten();
        let codex = if tool == ToolId::Codex {
            super::session_binding::prepare(store, id).ok()
        } else {
            None
        };
        Ok(Self {
            tool,
            root,
            before,
            id,
            codex,
        })
    }

    /// Installs only this launch's trusted tracker. A configuration that cannot
    /// preserve the caller's hooks leaves collection unavailable, not blocked.
    pub fn configure_codex(&mut self, command: &mut Command) -> Result<(), ManagedError> {
        self.codex
            .as_ref()
            .ok_or(ManagedError::InvalidRequest)?
            .configure(command)
    }

    fn source_at(&self, root: PathBuf) -> Result<Box<dyn TraceSource>, ManagedError> {
        match self.tool {
            ToolId::Claude => Ok(Box::new(source::claude_code::ClaudeCodeSource::new(root))),
            ToolId::Codex => Ok(Box::new(source::codex::CodexSource::new(root))),
            _ => Err(ManagedError::InvalidRequest),
        }
    }

    fn codex_identity(
        source: &dyn TraceSource,
        path: &Path,
        expected: Uuid,
    ) -> Result<(), ManagedError> {
        let suffix = format!("-{expected}");
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or(ManagedError::InvalidRequest)?;
        if !stem.starts_with("rollout-") || !stem.ends_with(&suffix) {
            return Err(ManagedError::InvalidRequest);
        }
        let session = source
            .session_at(path)
            .map_err(|_| ManagedError::InvalidRequest)?
            .ok_or(ManagedError::NotFound)?;
        let loaded = source
            .load(&session)
            .map_err(|_| ManagedError::InvalidRequest)?;
        if loaded.source_session
            != source::native_session_identity(source::SOURCE_CODEX, Some(&expected.to_string()))
        {
            return Err(ManagedError::InvalidRequest);
        }
        Ok(())
    }

    /// Stage only exactly bound, newly created native conversations. Preserve
    /// their original IDs and filenames: attestation enrichment joins on them.
    pub fn collect(&self, store: &ConfigStore) -> Result<(), ManagedError> {
        let before = self
            .before
            .as_ref()
            .ok_or(ManagedError::StorageUnavailable)?;
        let mut selected = BTreeSet::new();
        let mut parents = BTreeMap::new();
        match self.tool {
            ToolId::Claude => {
                let owned = source::claude_code::ClaudeCodeSource::new(self.root.clone())
                    .managed_session_paths(&self.id.to_string())
                    .map_err(|_| ManagedError::InvalidRequest)?;
                let parent = owned.first().ok_or(ManagedError::NotFound)?;
                let relative = parent
                    .strip_prefix(&self.root)
                    .map_err(|_| ManagedError::InvalidRequest)?;
                if before.contains(relative) {
                    return Err(ManagedError::InvalidRequest);
                }
                parents.insert(relative.to_path_buf(), self.id);
                for path in owned {
                    let relative = path
                        .strip_prefix(&self.root)
                        .map_err(|_| ManagedError::InvalidRequest)?;
                    if !before.contains(relative) {
                        selected.insert(relative.to_path_buf());
                    }
                }
            }
            ToolId::Codex => {
                let records = self
                    .codex
                    .as_ref()
                    .ok_or(ManagedError::NotFound)?
                    .records()?;
                let available = files(&self.root)?;
                let canonical_root =
                    std::fs::canonicalize(&self.root).map_err(|_| ManagedError::NotFound)?;
                let source = self.source_at(self.root.clone())?;
                for record in records {
                    let canonical = std::fs::canonicalize(&record.transcript_path)
                        .map_err(|_| ManagedError::NotFound)?;
                    if !canonical.starts_with(&canonical_root) {
                        return Err(ManagedError::InvalidRequest);
                    }
                    // Match against the non-symlink inventory; never follow a
                    // receipt outside the account profile or guess by cwd/time.
                    let matches = available
                        .iter()
                        .filter(|relative| {
                            std::fs::canonicalize(self.root.join(relative))
                                .ok()
                                .as_ref()
                                == Some(&canonical)
                        })
                        .collect::<Vec<_>>();
                    if matches.len() != 1 {
                        return Err(ManagedError::InvalidRequest);
                    }
                    let relative = matches[0];
                    if before.contains(relative) {
                        continue;
                    }
                    Self::codex_identity(
                        source.as_ref(),
                        &self.root.join(relative),
                        record.session_id,
                    )?;
                    if parents
                        .insert(relative.clone(), record.session_id)
                        .is_some()
                    {
                        return Err(ManagedError::InvalidRequest);
                    }
                    selected.insert(relative.clone());
                }
            }
            ToolId::Opencode => return Err(ManagedError::InvalidRequest),
        }
        if parents.is_empty() {
            return Err(ManagedError::NotFound);
        }
        let staged_root = store.dir().join(STAGING);
        private_directory(&staged_root)?;
        let tool_root = staged_root.join(tool_directory(self.tool)?);
        private_directory(&tool_root)?;
        let final_root = tool_root.join(self.id.to_string());
        if final_root.exists() {
            return Err(ManagedError::Conflict);
        }
        let pending = tool_root.join(format!("{}.pending", self.id));
        private_directory(&pending)?;
        let result = (|| {
            for relative in selected {
                let destination = pending.join(&relative);
                private_directory(destination.parent().ok_or(ManagedError::InvalidRequest)?)?;
                std::fs::copy(self.root.join(relative), &destination)
                    .map_err(|_| ManagedError::StorageUnavailable)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o600))
                        .map_err(|_| ManagedError::StorageUnavailable)?;
                }
            }
            let staged = self.source_at(pending.clone())?;
            let sessions = staged
                .discover()
                .map_err(|_| ManagedError::InvalidRequest)?;
            if sessions.len() != parents.len() {
                return Err(ManagedError::InvalidRequest);
            }
            for (relative, expected) in parents {
                match self.tool {
                    ToolId::Claude => {
                        source::claude_code::ClaudeCodeSource::new(pending.clone())
                            .managed_session_paths(&expected.to_string())
                            .map_err(|_| ManagedError::InvalidRequest)?;
                    }
                    ToolId::Codex => {
                        Self::codex_identity(staged.as_ref(), &pending.join(relative), expected)?
                    }
                    ToolId::Opencode => return Err(ManagedError::InvalidRequest),
                }
            }
            for session in sessions {
                staged
                    .load(&session)
                    .map_err(|_| ManagedError::InvalidRequest)?;
            }
            std::fs::rename(&pending, &final_root).map_err(|_| ManagedError::StorageUnavailable)
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&pending);
        }
        result
    }
}

/// Explicit staged roots are separate from automatic native discovery. Only
/// successfully published, real UUID directories can enter the watcher.
pub fn roots(store: &ConfigStore, tool: ToolId) -> Vec<PathBuf> {
    let Ok(tool) = tool_directory(tool) else {
        return vec![];
    };
    let base = store.dir().join(STAGING);
    if !std::fs::symlink_metadata(&base).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        return vec![];
    }
    let base = base.join(tool);
    if !std::fs::symlink_metadata(&base).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        return vec![];
    }
    let Ok(entries) = std::fs::read_dir(base) else {
        return vec![];
    };
    let mut roots = entries
        .flatten()
        .filter_map(|entry| {
            if Uuid::parse_str(entry.file_name().to_str()?).is_err()
                || !entry.file_type().ok()?.is_dir()
            {
                return None;
            }
            Some(entry.path())
        })
        .collect::<Vec<_>>();
    roots.sort();
    roots
}
