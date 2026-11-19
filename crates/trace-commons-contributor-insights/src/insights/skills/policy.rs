//! Canonical exclusions shared by every local inventory reader.
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

pub(super) struct ReadPolicy {
    denied: Vec<PathBuf>,
}

impl ReadPolicy {
    pub(super) fn new(denied_roots: &[PathBuf]) -> io::Result<Self> {
        if denied_roots.len() > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "skills_policy_too_large",
            ));
        }
        let mut denied = Vec::new();
        for root in denied_roots {
            // An unresolved exclusion is never silently treated as permission.
            // Retain its lexical form too, so an excluded alias is not followed.
            denied.push(absolute(root)?);
            denied.push(root.canonicalize()?);
        }
        Ok(Self { denied })
    }

    pub(super) fn has_exclusions(&self) -> bool {
        !self.denied.is_empty()
    }

    pub(super) fn canonical_allowed(&self, path: &Path) -> io::Result<PathBuf> {
        let absolute = absolute(path)?;
        self.check(&absolute)?;
        let canonical = absolute.canonicalize()?;
        self.check(&canonical)?;
        Ok(canonical)
    }

    fn check(&self, path: &Path) -> io::Result<()> {
        if self.denied.iter().any(|root| path.starts_with(root)) {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "skills_path_excluded",
            ))
        } else {
            Ok(())
        }
    }

    pub(super) fn read_dir(&self, path: &Path) -> io::Result<fs::ReadDir> {
        fs::read_dir(self.canonical_allowed(path)?)
    }

    pub(super) fn read(&self, path: &Path, limit: u64) -> io::Result<Vec<u8>> {
        let canonical = self.canonical_allowed(path)?;
        let file = trace_commons_contributor_platform::local_files::open_import_file(&canonical)
            .map_err(|_| io::Error::other("skill_file_unreadable"))?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("skill_not_regular_file"));
        }
        let mut bytes = Vec::new();
        file.take(limit + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            return Err(io::Error::other("skill_file_too_large"));
        }
        Ok(bytes)
    }
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    let input = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut result = PathBuf::new();
    for component in input.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    Ok(result)
}
