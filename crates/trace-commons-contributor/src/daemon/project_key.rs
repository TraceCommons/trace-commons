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

/// A recorded working directory resolved into the two forms the daemon
/// needs, which are deliberately not the same string.
///
/// `key` is case-folded on macOS and Windows ([`fold_case`]) because those
/// filesystems are case-insensitive: without folding one project mints two
/// keys depending on how the agent spelled the cwd, and a project set to
/// `Ignore` under one spelling lapses under the other. Everything that
/// decides something -- policy lookup, grouping, `project_id_for` -- keys
/// on this.
///
/// `display_path` is the same directory with the case the filesystem
/// actually holds (or, for a directory that no longer exists, the case the
/// recording carried). Nothing decides on it. It exists because a
/// contributor reading `~/code/ironwire` is being shown a directory that
/// is not spelled that way anywhere on their machine.
///
/// No `Debug`, no `Serialize`, and no `Clone` of convenience: `display_path`
/// is a local filesystem path, admissible only on the rendering paths that
/// already carry one (see `ipc::display_path`), and a derived `Debug` is
/// how such a string reaches a log line by accident.
pub struct NormalizedProject {
    pub key: String,
    pub display_path: String,
}

/// Normalize a recorded working directory into a project key.
///
/// `None` when the input cannot be a key at all: empty, blank, relative, or
/// with no usable final path segment. Those go to the unknown bucket, which
/// is `policy::project_key_for`'s job, not this one's.
///
/// Kept beside [`normalize_project`] rather than replaced by it. Most
/// callers -- `policy::project_key_for`, `ProjectPolicy::rekey`'s counter
/// and cooldown maps -- want the key and nothing else, and handing them a
/// struct whose other half must not be logged is how that half ends up
/// somewhere it should not be.
pub fn normalize_project_key(cwd: &str) -> Option<String> {
    normalize_project(cwd).map(|p| p.key)
}

/// [`normalize_project_key`] with the home directory injected, for tests.
pub fn normalize_project_key_within(cwd: &str, home: Option<&Path>) -> Option<String> {
    normalize_project_within(cwd, home).map(|p| p.key)
}

/// Normalize a recorded working directory into both the folded key and the
/// unfolded path a person should be shown.
pub fn normalize_project(cwd: &str) -> Option<NormalizedProject> {
    normalize_project_within(cwd, home_dir().as_deref())
}

/// The body of [`normalize_project`], with the home directory injected so
/// the "a marker in $HOME is not a project root" rule is testable without
/// touching the real environment.
pub fn normalize_project_within(cwd: &str, home: Option<&Path>) -> Option<NormalizedProject> {
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
    let resolved = std::fs::canonicalize(path)
        .map(strip_verbatim)
        .unwrap_or_else(|_| lexically_clean(path));
    if resolved.file_name().is_none_or(|n| n.is_empty()) {
        return None;
    }

    // Canonicalize home the same way, or the guard below compares two
    // different spellings of one directory and never fires. On macOS the
    // start path resolves through `/var -> /private/var` and a `$HOME` that
    // did not would sit on the other side of that symlink.
    let home = home.map(|h| {
        std::fs::canonicalize(h)
            .map(strip_verbatim)
            .unwrap_or_else(|_| lexically_clean(h))
    });

    let rooted = repo_root_of(&resolved, home.as_deref()).unwrap_or(resolved);
    // One string, two spellings. `std::fs::canonicalize` returns the case
    // the filesystem holds on both macOS and Windows, so the display half
    // is the true spelling of the directory rather than whatever the agent
    // happened to record; a directory that has since been deleted took the
    // `lexically_clean` fallback above and keeps the recorded spelling,
    // which is the most honest thing left to say about it.
    let display_path = path_to_key(&rooted);
    Some(NormalizedProject {
        key: fold_case(&display_path),
        display_path,
    })
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

/// Remove Windows' verbatim (`\\?\`) prefix from a canonicalized path.
///
/// `std::fs::canonicalize` returns a verbatim path on Windows. Left in, an
/// existing directory keys as `\\?\c:\users\z\repo` while one that has
/// since been deleted takes the [`lexically_clean`] fallback and keys as
/// `c:\users\z\repo` -- one project under two keys, exactly the collision
/// this module exists to prevent. It also defeats the UI's
/// `strip_prefix(home)`, which would render `\\?\c:\users\z\repo` rather
/// than `~\repo`.
///
/// `\\?\UNC\server\share` is restored to `\\server\share`; every other
/// verbatim path loses the prefix outright. A path with no such prefix is
/// returned untouched, so this is a no-op off Windows.
#[cfg(windows)]
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let stripped = {
        let text = path.to_string_lossy();
        text.strip_prefix(r"\\?\UNC\")
            .map(|rest| PathBuf::from(format!(r"\\{rest}")))
            .or_else(|| text.strip_prefix(r"\\?\").map(PathBuf::from))
    };
    stripped.unwrap_or(path)
}

#[cfg(not(windows))]
fn strip_verbatim(path: PathBuf) -> PathBuf {
    path
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
pub(crate) fn fold_case(key: &str) -> String {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        key.to_lowercase()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        key.to_string()
    }
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::test_paths::{abs, abs_key};

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
    fn a_resolved_key_never_carries_a_verbatim_prefix() {
        // Asserted against literals rather than against another
        // `normalize_project_key` call: every other test here compares one
        // normalization to another, so both sides would carry Windows'
        // `\\?\` prefix and agree while the key was still wrong.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repo");
        std::fs::create_dir(&path).unwrap();

        let key = normalize_project_key(path.to_str().unwrap()).unwrap();
        assert!(
            !key.starts_with(r"\\?\"),
            "a verbatim prefix leaked into the key: {key}"
        );
        assert!(
            key.ends_with("repo"),
            "expected a plain path ending in the directory name, got {key}"
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

    /// One directory, two spellings, and neither may drift into the other.
    ///
    /// Asserted as a pair on purpose. The key must stay folded -- that is
    /// what stops `~/Code/Api` and `~/code/api` minting two keys and
    /// letting an `Ignore` set under one spelling lapse under the other --
    /// and the display half must keep the capitals, or a contributor reads
    /// a path that exists nowhere on their disk. A future change that
    /// satisfies either one by breaking the other fails here.
    ///
    /// Built by creating the directory and normalizing a SUBDIRECTORY of
    /// it, so both expected strings come out of the real walk rather than
    /// being spelled by the test.
    #[test]
    fn a_normalized_project_folds_only_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("IronWire");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("Crates").join("Inner");
        std::fs::create_dir_all(&sub).unwrap();

        let p = normalize_project(sub.to_str().unwrap()).unwrap();
        assert!(
            p.display_path.ends_with("IronWire"),
            "the display half lost the capitals the disk holds: {}",
            p.display_path
        );
        assert_eq!(
            p.key,
            fold_case(&p.display_path),
            "the key must be the folded spelling of the same directory"
        );
        // Where folding is a no-op the two are equal by construction, so
        // the inequality is asserted only where it is a real claim.
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert_ne!(
            p.key, p.display_path,
            "on a case-insensitive filesystem the key must still be folded"
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
        // Spelled for the host: a leading-slash path is not absolute on
        // Windows, so this would have tested the relative-path rejection
        // rather than the textual fallback it is named for.
        let recorded = format!(
            "{}{}",
            abs("No/Such/Directory/Here"),
            std::path::MAIN_SEPARATOR
        );
        let key = normalize_project_key(&recorded).unwrap();
        assert_eq!(key, abs_key("No/Such/Directory/Here"));
    }
}
