//! Normalizing a recorded working directory into a stable project key.
//!
//! `policy::project_key_for` used to key on the raw `cwd` string an agent
//! recorded. Two sessions in one directory therefore landed in two project
//! groups whenever the recorded strings differed -- which they routinely
//! do: a symlinked path, a trailing separator, or (the case that separates
//! Codex from Claude Code) one agent recording the repository root and the
//! other recording the subdirectory the session started in.
//!
//! Normalizing here rather than at each call site is what makes the key one
//! thing. See `docs/superpowers/specs/2026-09-03-contributor-shell-queue-ux-design.md`.

use std::path::{Path, PathBuf};

/// Directory names that mark a repository root.
const VCS_MARKERS: [&str; 3] = [".git", ".hg", ".jj"];

/// How far above the recorded directory the repo-root walk will look.
///
/// A bound rather than "until the filesystem root" because the walk does
/// filesystem work per level, and because a marker twenty-four levels above
/// a session is not a project boundary anyone intended.
const MAX_WALK_DEPTH: usize = 24;

/// Normalize a recorded working directory into a project key.
///
/// `None` when the input cannot be a key at all: empty, blank, relative, or
/// with no usable final path segment. Those go to the unknown bucket, which
/// is `policy::project_key_for`'s job, not this one's.
pub fn normalize_project_key(cwd: &str) -> Option<String> {
    normalize_project_key_within(cwd, home_dir().as_deref())
}

/// The body of [`normalize_project_key`], with the home directory injected
/// so the "a marker in $HOME is not a project root" rule is testable
/// without touching the real environment.
pub fn normalize_project_key_within(cwd: &str, home: Option<&Path>) -> Option<String> {
    let trimmed = cwd.trim();
    if trimmed.is_empty() {
        return None;
    }

    let path = Path::new(trimmed);
    if !path.is_absolute() {
        return None;
    }

    // Judged on the RECORDED form, before any cleaning. `/Users/z/code/..`
    // lexically cleans to `/Users/z`, which would be a perfectly usable key
    // -- but a cwd an agent recorded that way is one it could not name, and
    // `policy`'s `a_cwd_with_no_usable_basename_goes_to_the_unknown_bucket`
    // keeps every such cwd in the unknown bucket rather than minting a key
    // from a directory the session was never really in.
    if path.file_name().is_none_or(|n| n.is_empty()) {
        return None;
    }

    // Strip trailing separators and any `.`/`..` the recording carried, and
    // resolve symlinks where the directory still exists. A directory that
    // has since been deleted keeps the textual form -- the watcher can
    // legitimately report a cwd that is already gone, and dropping such a
    // session's key would put it in the unknown bucket rather than with its
    // siblings.
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| lexically_clean(path));
    if resolved.file_name().is_none_or(|n| n.is_empty()) {
        return None;
    }

    // Canonicalize home the same way, or the guard below compares two
    // different spellings of one directory and never fires. On macOS the
    // start path resolves through `/var -> /private/var` and a `$HOME` that
    // did not would sit on the other side of that symlink.
    let home = home.map(|h| std::fs::canonicalize(h).unwrap_or_else(|_| lexically_clean(h)));

    let rooted = repo_root_of(&resolved, home.as_deref()).unwrap_or(resolved);
    Some(fold_case(&path_to_key(&rooted)))
}

/// The nearest enclosing repository root, if one sits within
/// [`MAX_WALK_DEPTH`] levels and is not the home directory or above it.
fn repo_root_of(start: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let mut current = start;
    for _ in 0..MAX_WALK_DEPTH {
        // A repository marker in $HOME (a dotfiles repo, most often) would
        // otherwise make every project on the machine one group. The walk
        // refuses to adopt home, or anything above it, as a root.
        let at_or_above_home = home.is_some_and(|h| h.starts_with(current) || h == current);
        if !at_or_above_home
            && VCS_MARKERS
                .iter()
                .any(|marker| current.join(marker).exists())
        {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
    None
}

/// Remove `.` components and resolve `..` textually, for a path that does
/// not exist and so cannot be canonicalized.
fn lexically_clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn path_to_key(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// Case-fold the key on platforms whose filesystems are case-insensitive in
/// practice, so `~/Code/api` and `~/code/api` are one project.
///
/// Deliberately platform-gated rather than probing the volume: Linux is
/// case-sensitive and folding there would merge two genuinely different
/// directories. The folded string is still a usable path on macOS and
/// Windows, which is what keeps `project_key_is_admissible` working against
/// it.
fn fold_case(key: &str) -> String {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        key.to_lowercase()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        key.to_string()
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_separator_is_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repo");
        std::fs::create_dir(&path).unwrap();
        let bare = normalize_project_key(path.to_str().unwrap()).unwrap();
        let trailing = normalize_project_key(&format!("{}/", path.to_str().unwrap())).unwrap();
        assert_eq!(bare, trailing);
    }

    // Gated whole rather than returning early inside the body: the early
    // return compiles unconditionally, which makes every assertion below it
    // unreachable under `-D warnings` and, when it did compile, left a test
    // that proved nothing on Windows.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_directory_normalizes_to_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(
            normalize_project_key(link.to_str().unwrap()),
            normalize_project_key(real.to_str().unwrap())
        );
    }

    #[test]
    fn a_subdirectory_of_a_git_repo_normalizes_to_the_repo_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("crates").join("thing");
        std::fs::create_dir_all(&sub).unwrap();

        assert_eq!(
            normalize_project_key(sub.to_str().unwrap()),
            normalize_project_key(root.to_str().unwrap())
        );
    }

    #[test]
    fn a_directory_with_no_repo_above_it_is_its_own_key() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain");
        std::fs::create_dir(&plain).unwrap();
        let key = normalize_project_key(plain.to_str().unwrap()).unwrap();
        assert!(
            key.ends_with("plain"),
            "expected the directory itself, got {key}"
        );
    }

    #[test]
    fn a_repo_marker_in_the_home_directory_never_becomes_a_key() {
        // A `.git` in $HOME would otherwise swallow every project on the
        // machine into one group. The walk stops below home.
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(home.join(".git")).unwrap();
        let project = home.join("code").join("thing");
        std::fs::create_dir_all(&project).unwrap();

        let key = normalize_project_key_within(project.to_str().unwrap(), Some(&home)).unwrap();
        assert!(
            key.ends_with("thing"),
            "expected the project itself, got {key}"
        );
    }

    #[test]
    fn an_empty_or_relative_cwd_has_no_key() {
        assert_eq!(normalize_project_key(""), None);
        assert_eq!(normalize_project_key("   "), None);
        assert_eq!(normalize_project_key("relative/path"), None);
    }

    #[test]
    fn a_path_that_does_not_exist_still_normalizes_textually() {
        // The watcher can see a cwd for a directory that has since been
        // deleted. That must still key consistently rather than vanishing.
        let key = normalize_project_key("/No/Such/Directory/Here/").unwrap();
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert_eq!(key, "/no/such/directory/here");
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert_eq!(key, "/No/Such/Directory/Here");
    }
}
