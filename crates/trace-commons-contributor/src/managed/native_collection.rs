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
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const STAGING: &str = "managed-native-exports";

pub struct NativeCollection {
    tool: ToolId,
    root: PathBuf,
    before: BTreeSet<PathBuf>,
    id: Uuid,
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
        if std::fs::symlink_metadata(&root)
            .is_ok_and(|meta| !meta.is_dir() || meta.file_type().is_symlink())
        {
            return Err(ManagedError::InvalidRequest);
        }
        let before = files(&root)?;
        Ok(Self {
            tool,
            root,
            before,
            id,
        })
    }

    fn source_at(&self, root: PathBuf) -> Result<Box<dyn TraceSource>, ManagedError> {
        match self.tool {
            ToolId::Claude => Ok(Box::new(source::claude_code::ClaudeCodeSource::new(root))),
            ToolId::Codex => Ok(Box::new(source::codex::CodexSource::new(root))),
            _ => Err(ManagedError::InvalidRequest),
        }
    }

    /// Validate newly created sessions using the ordinary parser, then stage
    /// their files atomically. A failure preserves all native profile files.
    pub fn collect(&self, store: &ConfigStore) -> Result<(), ManagedError> {
        let source = self.source_at(self.root.clone())?;
        let sessions = source
            .discover()
            .map_err(|_| ManagedError::InvalidRequest)?;
        let mut selected = BTreeSet::new();
        for session in sessions {
            let relative = session
                .path
                .strip_prefix(&self.root)
                .map_err(|_| ManagedError::InvalidRequest)?;
            if !self.before.contains(relative) {
                selected.insert(session.path);
            }
        }
        if selected.is_empty() {
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
            for relative in files(&self.root)?.difference(&self.before) {
                let path = self.root.join(relative);
                if !source
                    .session_for_path(&path)
                    .is_some_and(|parent| selected.contains(&parent))
                {
                    continue;
                }
                let destination = pending.join(relative);
                let parent = destination.parent().ok_or(ManagedError::InvalidRequest)?;
                private_directory(parent)?;
                std::fs::copy(&path, &destination).map_err(|_| ManagedError::StorageUnavailable)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o600))
                        .map_err(|_| ManagedError::StorageUnavailable)?;
                }
            }
            // Validate the bytes actually copied, not a mutable original.
            let staged = self.source_at(pending.clone())?;
            let sessions = staged
                .discover()
                .map_err(|_| ManagedError::InvalidRequest)?;
            if sessions.len() != selected.len() {
                return Err(ManagedError::InvalidRequest);
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
