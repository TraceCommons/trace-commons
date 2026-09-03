# Contributor Shell UX -- Daemon and FFI Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the three contributor shells the daemon-side facts they need to
render a folder-first queue and an honest scrubber -- normalized project keys,
a project path on the socket, a project id on history records, distinct-value
redaction counts, and a count-only search over pre-redaction text.

**Architecture:** Everything here lands below the shells, in
`trace-commons-contributor` (daemon), `trace-commons-protocol` (redaction),
and `trace-commons-contributor-ffi` (C ABI). No shell code is touched. Each
task is verifiable with `cargo test` alone, so the whole plan can be
completed and reviewed before any UI work starts.

**Tech Stack:** Rust 2024, `serde`/`serde_json`, `sha2`, `chrono`, `anyhow`,
`tempfile` (dev). No new dependencies -- see Global Constraints.

**Spec:** [`docs/superpowers/specs/2026-09-03-contributor-shell-queue-ux-design.md`](../specs/2026-09-03-contributor-shell-queue-ux-design.md)

## Scope: this is plan 1 of 4

The spec covers four independent subsystems (a daemon/FFI foundation and
three shells). Per the scope check, they are separate plans. This is the
foundation, and it is a prerequisite for all three shells because it defines
the fields and calls they consume. The other three -- macOS, GTK, Windows --
each implement spec §2-§5 against the interfaces this plan produces, and
should be written once these interfaces exist rather than against their
predicted shape.

This plan alone produces working, testable software: a daemon that groups
projects correctly and reports what the shells will need.

## Global Constraints

- **No new dependencies.** Everything below uses crates already in
  `crates/trace-commons-contributor/Cargo.toml`. Do not add one; if a task
  seems to need one, stop and ask.
- **These crates are `MIT OR Apache-2.0`.** `-contributor`, `-protocol`, and
  `-contributor-ffi` are permissive. Do not add a `-server`, `-gate-api`, or
  `-gate-enclave` dependency to any of them --
  `crates/trace-commons-server/tests/license_boundary.rs` fails if you do,
  and editing its expected sets is not the fix.
- **No new `.rs` copyright header needed.** The two-line AGPL header is for
  the server and gate crates only. Files in these three crates carry none;
  match the file you are editing.
- **Verification is warnings-as-errors.** CI applies `RUSTFLAGS=-D warnings`
  to check and test, and plain `cargo check` will not catch what CI catches.
  Every "run the tests" step below uses the `RUSTFLAGS` form. Run
  `cargo fmt --all` before every commit; `cargo fmt --check` gates every PR.
- **No emojis** in commits, PRs, code, or comments. Commit subjects are
  short and imperative, with no `feat:` / `fix:` prefix.
- **A local filesystem path must never reach `daemon-audit.jsonl`, OS
  notification text, or a `HistoryRecord`.** This is the invariant the whole
  spec is careful around. `project_path` (Task 5) is for the IPC response
  only.
- **The C ABI header exists in two copies** --
  `crates/trace-commons-contributor-ffi/include/trace_commons.h` and
  `macos/Sources/CTraceCommons/include/trace_commons.h` -- and CI enforces
  them byte-for-byte identical. Task 8 edits both.

---

### Task 1: The project-key normalizer

The pure function, with no callers yet. `project_key_for` keys on the raw
`cwd` an agent recorded, so one directory reached two ways becomes two
project groups. This is the function that collapses them.

**Files:**
- Create: `crates/trace-commons-contributor/src/daemon/project_key.rs`
- Modify: `crates/trace-commons-contributor/src/daemon/mod.rs` (add `pub mod project_key;`)
- Test: same file, `#[cfg(test)] mod tests` (this crate tests inline; follow `policy.rs`)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub fn normalize_project_key(cwd: &str) -> Option<String>` --
  `None` when the input is not usable as a key (empty, relative, or no
  usable basename), otherwise the normalized absolute path as a `String`.

- [ ] **Step 1: Write the failing tests**

Create `crates/trace-commons-contributor/src/daemon/project_key.rs` with
only the test module and a stub, so the file compiles and the tests fail on
behavior rather than on a missing symbol:

```rust
//! Normalizing a recorded working directory into a stable project key.

/// Normalize a recorded working directory into a project key.
pub fn normalize_project_key(_cwd: &str) -> Option<String> {
    None
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

    #[test]
    fn a_symlinked_directory_normalizes_to_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(not(unix))]
        return;
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
        assert!(key.ends_with("plain"), "expected the directory itself, got {key}");
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
        assert!(key.ends_with("thing"), "expected the project itself, got {key}");
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
        let key = normalize_project_key("/no/such/directory/here/").unwrap();
        assert_eq!(key, "/no/such/directory/here");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib daemon::project_key`

Expected: compile error, `cannot find function normalize_project_key_within`,
plus failures in the other tests (they get `None` from the stub).

- [ ] **Step 3: Write the implementation**

Replace the stub in `project_key.rs`:

```rust
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

    let rooted = repo_root_of(&resolved, home).unwrap_or(resolved);
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
```

Then register the module in `crates/trace-commons-contributor/src/daemon/mod.rs`,
beside the existing `pub mod policy;`:

```rust
pub mod project_key;
```

- [ ] **Step 4: Fix the case-folding test expectation**

`a_path_that_does_not_exist_still_normalizes_textually` asserts an exact
string, and on macOS the key is folded to lowercase. Change that one
assertion to be fold-aware:

```rust
    #[test]
    fn a_path_that_does_not_exist_still_normalizes_textually() {
        let key = normalize_project_key("/No/Such/Directory/Here/").unwrap();
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert_eq!(key, "/no/such/directory/here");
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert_eq!(key, "/No/Such/Directory/Here");
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib daemon::project_key`

Expected: 7 passed.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/daemon/project_key.rs \
        crates/trace-commons-contributor/src/daemon/mod.rs
git commit -m "Add a project-key normalizer"
```

---

### Task 2: Key the policy on the normalized path

Wire the normalizer into `project_key_for`, so every newly discovered
session groups by normalized key. Nothing migrates yet -- that is Task 3,
and keeping them separate is what lets a reviewer reject the migration
without rejecting the normalizer.

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/policy.rs:392-397` (`project_key_for`)
- Test: same file, existing `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `daemon::project_key::normalize_project_key(&str) -> Option<String>` (Task 1).
- Produces: `project_key_for` unchanged in signature --
  `pub fn project_key_for(cwd: Option<&str>) -> String` -- but returning a
  normalized key.

- [ ] **Step 1: Write the failing test**

Add to `policy.rs`'s test module:

```rust
    #[test]
    fn two_recordings_of_one_directory_share_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("crates").join("inner");
        std::fs::create_dir_all(&sub).unwrap();

        // What Claude Code records, and what Codex records, for one repo.
        let from_root = project_key_for(Some(root.to_str().unwrap()));
        let from_sub = project_key_for(Some(sub.to_str().unwrap()));

        assert_eq!(from_root, from_sub);
        assert_eq!(project_id_for(&from_root), project_id_for(&from_sub));
    }

    #[test]
    fn an_unusable_cwd_still_lands_in_the_unknown_bucket() {
        assert_eq!(project_key_for(None), UNKNOWN_PROJECT_KEY);
        assert_eq!(project_key_for(Some("")), UNKNOWN_PROJECT_KEY);
        assert_eq!(project_key_for(Some("/")), UNKNOWN_PROJECT_KEY);
        assert_eq!(project_key_for(Some("relative")), UNKNOWN_PROJECT_KEY);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib daemon::policy::tests::two_recordings_of_one_directory_share_a_key`

Expected: FAIL -- `assertion failed: left == right`, the two keys differing
by the `crates/inner` suffix.

- [ ] **Step 3: Write the implementation**

Replace `project_key_for` in `policy.rs`. **Leave `has_usable_basename`
alone** -- it looks dead once the normalizer owns that judgement for
`project_key_for`, but `project_label_for` still calls it at `policy.rs:407`
as its second line of defence against a hand-edited policy file. Deleting it
breaks that.

```rust
/// The policy key for a session: its normalized working directory, or the
/// locked unknown bucket. Never falls back to a basename heuristic.
///
/// Normalization (`project_key::normalize_project_key`) is what makes one
/// directory one project regardless of how the recording spelled it. A cwd
/// with no usable final path segment -- `/`, anything ending in `..`, the
/// empty string, a relative path -- yields no key and goes to the unknown
/// bucket rather than becoming a key of its own. Such a key has no label
/// but itself, and `project_label` crosses the socket, lands in
/// `daemon-audit.jsonl`, in OS notification text, and in `HistoryRecord` --
/// so the fallback turned a full local path into every one of those, in
/// direct violation of the invariant `audit`'s own
/// `an_audit_entry_never_carries_a_path` test asserts.
pub fn project_key_for(cwd: Option<&str>) -> String {
    cwd.and_then(crate::daemon::project_key::normalize_project_key)
        .unwrap_or_else(|| UNKNOWN_PROJECT_KEY.to_string())
}
```

- [ ] **Step 4: Run the whole policy suite**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib daemon::policy`

Expected: all pass. If `a_project_id_is_deterministic_and_distinct_per_project`
or `a_project_id_resolves_back_to_the_key_that_minted_it` now fail, they are
asserting over literal paths like `/Users/z/code/proj` that do not exist on
the test machine -- the normalizer handles those textually, so they should
still pass. If one fails on case folding under macOS, lowercase the literal
in the test rather than weakening the normalizer.

- [ ] **Step 5: Run the full crate suite for fallout**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`

Expected: all pass. Record the count; Task 3 compares against it.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/daemon/policy.rs
git commit -m "Key project policy on the normalized working directory"
```

---

### Task 3: Re-key the policy file on upgrade

Task 2 changed what `project_key_for` returns, and project ids are derived
from keys rather than stored. So every id changes on upgrade, and
`daemon-projects.json` -- whose three maps are all keyed by project key --
is orphaned. The consequence that matters: **a project the contributor set
to `Ignore` silently re-arms to `NotifyOnly`.** This task is what stops
that.

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/policy.rs:29` (schema constant), `:170-176` (`load`)
- Test: same file, existing `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `daemon::project_key::normalize_project_key` (Task 1).
- Produces:
  - `pub const DAEMON_PROJECTS_SCHEMA: &str = "trace_commons.daemon_projects.v2";`
  - `pub const DAEMON_PROJECTS_SCHEMA_V1: &str = "trace_commons.daemon_projects.v1";`
  - `impl ProjectPolicy { pub fn rekey(&mut self); }` -- idempotent.
  - `ProjectMode::more_restrictive(self, other: ProjectMode) -> ProjectMode`

- [ ] **Step 1: Write the failing tests**

Add to `policy.rs`'s test module:

```rust
    #[test]
    fn ignore_survives_a_rekey_that_merges_two_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("crates").join("inner");
        std::fs::create_dir_all(&sub).unwrap();

        // A v1 file: two entries that Task 1 collapses into one key, with
        // the more permissive mode listed second so a naive last-write-wins
        // would lose the Ignore.
        let mut p = ProjectPolicy::new();
        p.schema_version = DAEMON_PROJECTS_SCHEMA_V1.to_string();
        p.projects.insert(
            root.to_string_lossy().to_string(),
            ProjectEntry {
                mode: ProjectMode::Ignore,
                added_at: now(),
                label: "repo".to_string(),
            },
        );
        p.projects.insert(
            sub.to_string_lossy().to_string(),
            ProjectEntry {
                mode: ProjectMode::AutoUpload,
                added_at: now(),
                label: "inner".to_string(),
            },
        );

        p.rekey();

        let key = project_key_for(Some(root.to_str().unwrap()));
        assert_eq!(p.projects.len(), 1);
        assert_eq!(p.resolve(&key), ProjectMode::Ignore);
        assert_eq!(p.schema_version, DAEMON_PROJECTS_SCHEMA);
    }

    #[test]
    fn a_rekey_carries_contribution_counts_and_declines_across() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("sub");
        std::fs::create_dir(&sub).unwrap();

        let mut p = ProjectPolicy::new();
        p.schema_version = DAEMON_PROJECTS_SCHEMA_V1.to_string();
        p.contributed.insert(root.to_string_lossy().to_string(), 3);
        p.contributed.insert(sub.to_string_lossy().to_string(), 4);
        p.arming_declined_at
            .insert(sub.to_string_lossy().to_string(), now());

        p.rekey();

        let key = project_key_for(Some(root.to_str().unwrap()));
        // Counts for two spellings of one project are one project's count.
        assert_eq!(p.contributed.get(&key), Some(&7));
        assert!(p.arming_declined_at.contains_key(&key));
    }

    #[test]
    fn a_rekey_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();

        let mut p = ProjectPolicy::new();
        p.schema_version = DAEMON_PROJECTS_SCHEMA_V1.to_string();
        p.set_mode(root.to_str().unwrap(), ProjectMode::Ignore, now())
            .unwrap();

        p.rekey();
        let once = p.clone();
        p.rekey();
        assert_eq!(p, once);
    }

    #[test]
    fn loading_a_v1_file_rekeys_it() {
        let (_d, store) = temp_store();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("sub");
        std::fs::create_dir(&sub).unwrap();

        let mut p = ProjectPolicy::new();
        p.schema_version = DAEMON_PROJECTS_SCHEMA_V1.to_string();
        p.projects.insert(
            sub.to_string_lossy().to_string(),
            ProjectEntry {
                mode: ProjectMode::Ignore,
                added_at: now(),
                label: "sub".to_string(),
            },
        );
        p.save(&store).unwrap();

        let loaded = ProjectPolicy::load(&store).unwrap();
        let key = project_key_for(Some(root.to_str().unwrap()));
        assert_eq!(loaded.resolve(&key), ProjectMode::Ignore);
    }

    #[test]
    fn the_unknown_bucket_is_never_rekeyed() {
        let mut p = ProjectPolicy::new();
        p.schema_version = DAEMON_PROJECTS_SCHEMA_V1.to_string();
        p.projects.insert(
            UNKNOWN_PROJECT_KEY.to_string(),
            ProjectEntry {
                mode: ProjectMode::Ignore,
                added_at: now(),
                label: UNKNOWN_PROJECT_KEY.to_string(),
            },
        );
        p.rekey();
        assert_eq!(p.resolve(UNKNOWN_PROJECT_KEY), ProjectMode::Ignore);
    }

    #[test]
    fn ignore_is_the_most_restrictive_mode() {
        use ProjectMode::*;
        assert_eq!(Ignore.more_restrictive(AutoUpload), Ignore);
        assert_eq!(AutoUpload.more_restrictive(Ignore), Ignore);
        assert_eq!(NotifyOnly.more_restrictive(AutoUpload), NotifyOnly);
        assert_eq!(AutoUpload.more_restrictive(AutoUpload), AutoUpload);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib daemon::policy::tests::ignore_survives_a_rekey_that_merges_two_entries`

Expected: compile error, `no method named rekey`.

- [ ] **Step 3: Write the implementation**

In `policy.rs`, change the schema constant and add its predecessor:

```rust
/// The current policy file schema.
///
/// Bumped to v2 when project keys became normalized
/// (`project_key::normalize_project_key`). A v1 file's keys are raw
/// recorded working directories, which no longer match anything
/// `project_key_for` produces, so `load` re-keys it. The version is what
/// makes that happen exactly once.
pub const DAEMON_PROJECTS_SCHEMA: &str = "trace_commons.daemon_projects.v2";

/// The pre-normalization schema. Recognized only so `load` can migrate it.
pub const DAEMON_PROJECTS_SCHEMA_V1: &str = "trace_commons.daemon_projects.v1";
```

Add the mode comparison, beside the `ProjectMode` enum:

```rust
impl ProjectMode {
    /// The more restrictive of two modes.
    ///
    /// `Ignore` beats everything, then `NotifyOnly`, then `AutoUpload`.
    /// This is the merge rule when normalization collapses two policy
    /// entries into one key, and the direction is not arbitrary: merging
    /// toward the permissive mode would take a project the contributor had
    /// silenced and start offering it again, or take one they had left
    /// ask-first and upload from it unattended. A merge may only ever ask
    /// more permission than before, never less.
    pub fn more_restrictive(self, other: ProjectMode) -> ProjectMode {
        use ProjectMode::*;
        match (self, other) {
            (Ignore, _) | (_, Ignore) => Ignore,
            (NotifyOnly, _) | (_, NotifyOnly) => NotifyOnly,
            (AutoUpload, AutoUpload) => AutoUpload,
        }
    }
}
```

Add `rekey` inside `impl ProjectPolicy`:

```rust
    /// Re-key every map through `normalize_project_key`, merging entries
    /// that collapse onto one key.
    ///
    /// Idempotent: a key that is already normalized normalizes to itself,
    /// so running this on a v2 file changes nothing. The unknown bucket is
    /// not a path and is carried across untouched.
    ///
    /// A key that no longer normalizes at all -- a relative path from a
    /// hand-edited file, say -- is dropped rather than kept under its old
    /// spelling, because nothing will ever look it up again.
    pub fn rekey(&mut self) {
        let renamed = |key: &str| -> Option<String> {
            if key == UNKNOWN_PROJECT_KEY {
                return Some(UNKNOWN_PROJECT_KEY.to_string());
            }
            crate::daemon::project_key::normalize_project_key(key)
        };

        let mut projects: BTreeMap<String, ProjectEntry> = BTreeMap::new();
        for (key, entry) in std::mem::take(&mut self.projects) {
            let Some(fresh) = renamed(&key) else { continue };
            let label = project_label_for(&fresh);
            projects
                .entry(fresh)
                .and_modify(|existing| {
                    existing.mode = existing.mode.more_restrictive(entry.mode);
                    // The earlier of the two: the contributor's decision
                    // about this project is as old as its oldest half.
                    existing.added_at = existing.added_at.min(entry.added_at);
                    existing.label = label.clone();
                })
                .or_insert(ProjectEntry {
                    mode: entry.mode,
                    added_at: entry.added_at,
                    label,
                });
        }
        self.projects = projects;

        let mut contributed: BTreeMap<String, u32> = BTreeMap::new();
        for (key, count) in std::mem::take(&mut self.contributed) {
            let Some(fresh) = renamed(&key) else { continue };
            *contributed.entry(fresh).or_insert(0) += count;
        }
        self.contributed = contributed;

        let mut declined: BTreeMap<String, DateTime<Utc>> = BTreeMap::new();
        for (key, at) in std::mem::take(&mut self.arming_declined_at) {
            let Some(fresh) = renamed(&key) else { continue };
            // The most recent decline wins: a merged project was declined
            // as recently as its most recent half, which is what the
            // cooldown should measure from.
            declined
                .entry(fresh)
                .and_modify(|existing| *existing = (*existing).max(at))
                .or_insert(at);
        }
        self.arming_declined_at = declined;

        self.schema_version = DAEMON_PROJECTS_SCHEMA.to_string();
    }
```

Change `load` to run it:

```rust
    pub fn load(store: &ConfigStore) -> Result<Self> {
        let Some(body) = store.read_daemon_file(DAEMON_PROJECTS_FILE)? else {
            return Ok(Self::new());
        };
        let mut policy: Self =
            serde_json::from_slice(&body).context("parsing daemon project policy")?;
        // Migrate in memory on every load rather than rewriting the file
        // here: `load` has no business writing, and the next `save` -- which
        // every mutation already performs -- persists the v2 form. A file
        // that is never mutated again is migrated identically on every read,
        // which costs one normalization pass and is always correct.
        if policy.schema_version != DAEMON_PROJECTS_SCHEMA {
            policy.rekey();
        }
        Ok(policy)
    }
```

- [ ] **Step 4: Run the policy suite to verify it passes**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib daemon::policy`

Expected: all pass, including the six new tests.

- [ ] **Step 5: Run the full crate suite**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`

Expected: the same count as Task 2 Step 5, plus the six new tests. Any test
asserting the literal string `trace_commons.daemon_projects.v1` needs its
expectation moved to `DAEMON_PROJECTS_SCHEMA`, not to a hardcoded `v2`.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/daemon/policy.rs
git commit -m "Re-key the project policy file when keys are normalized"
```

---

### Task 4: Keep the recorded working directory on the entry

Normalization means `project_key` is now the repo root, not where the
session actually ran. The folder detail view shows the difference, so the
raw recorded cwd has to survive.

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/queue.rs:51-80` (`QueueEntry`)
- Modify: whichever call site constructs `QueueEntry` from a discovered session (find with the grep in Step 1)
- Test: `crates/trace-commons-contributor/src/daemon/queue.rs`, inline test module

**Interfaces:**
- Consumes: nothing new.
- Produces: `QueueEntry.session_cwd: Option<String>`, `#[serde(default)]`.

- [ ] **Step 1: Find the construction site**

Run: `grep -rn "project_key: " crates/trace-commons-contributor/src --include='*.rs' | grep -v "pub project_key"`

Note every line. Each is a `QueueEntry` literal that will need the new
field, and the compiler will point at all of them anyway once the field
exists.

- [ ] **Step 2: Write the failing test**

Add to `queue.rs`'s test module (match the helper the neighbouring tests use
to build an entry; if they use a `fn entry(...)` constructor, extend it
rather than writing a fresh literal):

```rust
    #[test]
    fn an_entry_remembers_where_the_session_actually_ran() {
        let mut e = sample_entry();
        e.project_key = "/repo".to_string();
        e.session_cwd = Some("/repo/crates/inner".to_string());

        let round_tripped: QueueEntry =
            serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(
            round_tripped.session_cwd.as_deref(),
            Some("/repo/crates/inner")
        );
    }

    #[test]
    fn an_entry_written_before_session_cwd_existed_still_loads() {
        let mut value = serde_json::to_value(sample_entry()).unwrap();
        value.as_object_mut().unwrap().remove("session_cwd");
        let loaded: QueueEntry = serde_json::from_value(value).unwrap();
        assert_eq!(loaded.session_cwd, None);
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib daemon::queue`

Expected: compile error, `no field session_cwd on type QueueEntry`.

- [ ] **Step 4: Add the field**

In `queue.rs`, immediately after `pub project_key: String,`:

```rust
    /// The working directory the session actually recorded, when it differs
    /// from `project_key`.
    ///
    /// `project_key` is normalized to the enclosing repository root, which
    /// is what makes one repo one group no matter which agent recorded the
    /// session or which subdirectory it started in. That normalization
    /// throws away a real fact -- *where* it ran -- and the folder detail
    /// view puts it back. Local-only, exactly like `project_key` and
    /// `path`: it is a filesystem path and never reaches an audit row, a
    /// notification, a history record, or the wire.
    ///
    /// `#[serde(default)]` because `daemon-queue.jsonl` written before this
    /// field existed must still load; a required field here would make the
    /// daemon refuse its own queue after an upgrade.
    #[serde(default)]
    pub session_cwd: Option<String>,
```

- [ ] **Step 5: Fill it at every construction site**

At each line found in Step 1, set `session_cwd` to the raw recorded cwd --
the same `Option<&str>` that was passed to `project_key_for` -- as
`cwd.map(|c| c.to_string())`. Where a construction site is in a test and has
no meaningful cwd, use `session_cwd: None`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/daemon/queue.rs crates/trace-commons-contributor/src
git commit -m "Keep a session's recorded working directory on its queue entry"
```

---

### Task 5: Put the project path on the socket

The one place the path-privacy rule is relaxed, and the task that carries
the test proving it was relaxed nowhere else.

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs:829-855` (`entry_value`)
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs:915-948` (`list_projects`, both `json!` arms)
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs:44-50` (the module doc's statement of the rule)
- Test: `crates/trace-commons-contributor/src/daemon/ipc.rs`, inline test module

**Interfaces:**
- Consumes: `QueueEntry.session_cwd` (Task 4).
- Produces: two new keys on the queue-entry wire shape, `project_path:
  String` and `session_path: String | null`; one new key on each
  `list_projects` row, `project_path: String`. All `~`-abbreviated.
- Produces: `pub fn display_path(key: &str) -> String` in `ipc.rs`.

- [ ] **Step 1: Write the failing tests**

Add to `ipc.rs`'s test module:

```rust
    #[test]
    fn a_queue_entry_carries_a_displayable_project_path() {
        let mut e = sample_entry();
        e.project_key = "/tmp/somewhere/repo".to_string();
        e.session_cwd = Some("/tmp/somewhere/repo/crates/inner".to_string());

        let v = entry_value(&e);
        assert_eq!(v["project_path"], "/tmp/somewhere/repo");
        assert_eq!(v["session_path"], "/tmp/somewhere/repo/crates/inner");
    }

    #[test]
    fn a_home_relative_project_path_is_abbreviated() {
        let home = std::env::var("HOME").expect("HOME is set in the test environment");
        assert_eq!(display_path(&format!("{home}/code/api")), "~/code/api");
        assert_eq!(display_path("/opt/elsewhere"), "/opt/elsewhere");
    }

    #[test]
    fn the_unknown_bucket_has_no_path_to_show() {
        assert_eq!(display_path(UNKNOWN_PROJECT_KEY), UNKNOWN_PROJECT_KEY);
    }

    #[test]
    fn session_path_is_absent_when_it_matches_the_project() {
        let mut e = sample_entry();
        e.project_key = "/tmp/somewhere/repo".to_string();
        e.session_cwd = Some("/tmp/somewhere/repo".to_string());
        assert!(entry_value(&e)["session_path"].is_null());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib daemon::ipc`

Expected: compile error, `cannot find function display_path`.

- [ ] **Step 3: Write the implementation**

Add to `ipc.rs`, above `entry_value`:

```rust
/// A project key rendered for display: `~`-abbreviated, never modified
/// otherwise.
///
/// This is the ONE place in this crate that deliberately puts a local
/// filesystem path on the socket, and the bound is stated where the
/// function is rather than in a comment somewhere else:
///
/// > A path may be rendered. It may not be logged, audited, notified, or
/// > persisted to history.
///
/// `project_label` remains the basename and remains the only project string
/// that reaches `daemon-audit.jsonl`, notification text, or a
/// `HistoryRecord` -- see `an_audit_entry_never_carries_a_path` and
/// `no_sink_carries_a_project_path`. The relaxation exists because
/// `disambiguated_label` can keep two projects DISTINCT (`api` and
/// `api (3f9c)`) but can never make them IDENTIFIABLE, and a contributor
/// deciding what to upload from which repository needs the second.
pub fn display_path(project_key: &str) -> String {
    if project_key == UNKNOWN_PROJECT_KEY {
        return UNKNOWN_PROJECT_KEY.to_string();
    }
    let Some(home) = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| h.to_string_lossy().to_string())
        .filter(|h| !h.is_empty())
    else {
        return project_key.to_string();
    };
    match project_key.strip_prefix(&home) {
        Some(rest) if rest.is_empty() => "~".to_string(),
        Some(rest) if rest.starts_with('/') || rest.starts_with('\\') => format!("~{rest}"),
        _ => project_key.to_string(),
    }
}
```

In `entry_value`, immediately after the `"project_label"` line:

```rust
        // Rendered, never logged -- see `display_path`.
        "project_path": display_path(&e.project_key),
        // Only when the session ran somewhere other than the project root,
        // which is the fact normalization discards and the folder detail
        // view puts back. Null rather than a repeat of `project_path`, so a
        // shell can render the line only when it says something.
        "session_path": e
            .session_cwd
            .as_deref()
            .map(display_path)
            .filter(|p| p != &display_path(&e.project_key))
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null),
```

In both `list_projects` `json!` arms, after the `"project_label"` line:

```rust
                        "project_path": display_path(key),
```

Then update the module doc at `ipc.rs:44-50`, which currently states the old
absolute rule. Replace the sentence "Queue entries carry `project_label`,
never `project_key` or `path`." with:

```rust
//! is a fixed label. Queue entries carry `project_label` and, for display
//! only, `project_path` -- never `project_key` or `path`. The path is on
//! this socket and nowhere else: see `display_path` for the bound, and
//! `no_sink_carries_a_project_path` for what enforces it. Project labels
//! are derived by the daemon from
```

- [ ] **Step 4: Write the sink test**

This is the test the whole relaxation rests on. Add it to `ipc.rs`'s test
module:

```rust
    /// The path is on the socket and nowhere else.
    ///
    /// Deliberately asserts over the SINKS rather than over `display_path`:
    /// the risk is not that this function is wrong, it is that some later
    /// change pipes its output into an audit row or a notification. The
    /// audit sink has its own long-standing guard
    /// (`an_audit_entry_never_carries_a_path`); this covers the history
    /// record, which gained a project field in the same change.
    #[test]
    fn no_sink_carries_a_project_path() {
        let key = "/tmp/somewhere/secret-client-name";
        let record = crate::daemon::history::HistoryRecord {
            submission_id: uuid::Uuid::new_v4(),
            submitted_at: chrono::Utc::now(),
            project_id: crate::daemon::policy::project_id_for(key),
            project_label: crate::daemon::policy::project_label_for(key),
            source: "claude_code".to_string(),
            session_hash: "sha256:abc".to_string(),
            status: "accepted".to_string(),
            consent_scopes: vec![],
            credit_points_pending: 0.0,
            credit_points_final: None,
            explanations: vec![],
            last_refreshed_at: None,
            withdrawn_at: None,
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(
            !json.contains("/tmp/somewhere"),
            "a history record must never carry a path: {json}"
        );
        assert!(
            !json.contains(&crate::daemon::ipc::display_path(key)),
            "not even an abbreviated one: {json}"
        );
    }
```

This test will not compile until Task 6 adds `project_id` to
`HistoryRecord`. That is deliberate -- write it now, watch it fail to
compile, and let Task 6 be what makes it pass.

- [ ] **Step 5: Run the tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib daemon::ipc`

Expected: the four Step 1 tests PASS; `no_sink_carries_a_project_path` fails
to compile with `struct HistoryRecord has no field named project_id`.
Temporarily comment out `no_sink_carries_a_project_path` to confirm the
other four pass, then uncomment it before committing and note in the commit
that Task 6 completes it.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/daemon/ipc.rs
git commit -m "Report a project path on the socket, and nowhere else"
```

---

### Task 6: Put the project id on history records

History groups by folder like the queue does. It cannot group on the label
-- a label is a display name and is not unique across two projects -- so it
needs the opaque id. The id is a one-way hash carrying no path component,
which is why it is admissible in a sink a path is not.

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/history.rs:41-59` (`HistoryRecord`)
- Modify: whichever call site constructs `HistoryRecord` (find with the grep in Step 1)
- Test: `crates/trace-commons-contributor/src/daemon/history.rs`, inline test module

**Interfaces:**
- Consumes: `policy::project_id_for` (existing).
- Produces: `HistoryRecord.project_id: String`, `#[serde(default)]`, placed
  immediately before `project_label`.

- [ ] **Step 1: Find the construction site**

Run: `grep -rn "HistoryRecord {" crates/trace-commons-contributor/src --include='*.rs'`

- [ ] **Step 2: Write the failing test**

Add to `history.rs`'s test module:

```rust
    #[test]
    fn a_history_record_carries_the_opaque_project_id_and_no_path() {
        let key = "/tmp/somewhere/repo";
        let record = HistoryRecord {
            submission_id: Uuid::new_v4(),
            submitted_at: Utc::now(),
            project_id: crate::daemon::policy::project_id_for(key),
            project_label: crate::daemon::policy::project_label_for(key),
            source: "claude_code".to_string(),
            session_hash: "sha256:abc".to_string(),
            status: "accepted".to_string(),
            consent_scopes: vec![],
            credit_points_pending: 0.0,
            credit_points_final: None,
            explanations: vec![],
            last_refreshed_at: None,
            withdrawn_at: None,
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(json.contains("proj_"), "expected an opaque id: {json}");
        assert!(!json.contains("/tmp"), "a path leaked: {json}");
    }

    #[test]
    fn a_history_record_written_before_project_id_existed_still_loads() {
        let value = serde_json::json!({
            "submission_id": Uuid::new_v4(),
            "submitted_at": Utc::now(),
            "project_label": "repo",
            "source": "claude_code",
            "session_hash": "sha256:abc",
            "status": "accepted",
            "consent_scopes": [],
            "credit_points_pending": 0.0,
            "explanations": [],
        });
        let loaded: HistoryRecord = serde_json::from_value(value).unwrap();
        assert_eq!(loaded.project_id, "");
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib daemon::history`

Expected: compile error, `struct HistoryRecord has no field named project_id`.

- [ ] **Step 4: Add the field**

In `history.rs`, immediately before `pub project_label: String,`:

```rust
    /// The opaque project handle, so a shell can group history by folder
    /// the way it groups the queue.
    ///
    /// Grouping on `project_label` instead is not an option: a label is a
    /// display name, is not unique across two projects, and grouping on it
    /// would merge two different repositories into one row -- the same
    /// mistake `QueueGroup`'s own doc comment exists to forbid.
    ///
    /// This is admissible in a history record where a path is not, and by
    /// construction rather than by policy: `project_id_for` is a one-way
    /// SHA-256 prefix that leaks no path component. It is an identifier a
    /// client can hold, not a capability.
    ///
    /// `#[serde(default)]` -- empty on records cached before this field
    /// existed. Those cannot be resolved to a folder and group under their
    /// label alone, which is what they already did. Backfilling is not
    /// possible: nothing retained the key they were minted from.
    #[serde(default)]
    pub project_id: String,
```

- [ ] **Step 5: Fill it at the construction site**

At each line found in Step 1, set `project_id: crate::daemon::policy::project_id_for(&<the project key in scope>)`.
Where the constructing code holds a `QueueEntry`, that is
`project_id_for(&entry.project_key)`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`

Expected: all pass, including Task 5's `no_sink_carries_a_project_path`,
which compiles for the first time here.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/daemon/history.rs crates/trace-commons-contributor/src
git commit -m "Carry the opaque project id on history records"
```

---

### Task 7: Report distinct redaction values, not just occurrences

`185 local path` is an occurrence count. The redactor already assigns one
placeholder per distinct value, so the distinct count is sitting in the
placeholder map unreported -- and it is the number a person estimating risk
actually wants.

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs:4308-4327` (`PlaceholderMap`)
- Modify: `crates/trace-commons-contributor/src/daemon/preview.rs:324-332` (`PreviewSummary`), `:406-432` (`PreviewCardSummary`), `:634-650` (where `redactions` is populated)
- Test: `crates/trace-commons-protocol/src/trace_contribution.rs` inline tests; `crates/trace-commons-contributor/src/daemon/preview.rs` inline tests

**Interfaces:**
- Consumes: nothing new.
- Produces: `PreviewSummary.redactions_distinct: BTreeMap<String, u32>` and
  the same field on `PreviewCardSummary`, keyed by the same label strings as
  `redactions`.

- [ ] **Step 1: Write the failing protocol test**

The distinct-per-value property is real but untested, and the new field
depends on it. Add to `trace_contribution.rs`'s test module:

```rust
    #[test]
    fn one_value_gets_one_placeholder_however_often_it_appears() {
        let mut map = PlaceholderMap::default();
        let first = map.placeholder_for("local_path", "/Users/z/code/api");
        let again = map.placeholder_for("local_path", "/Users/z/code/api");
        let other = map.placeholder_for("local_path", "/Users/z/code/web");

        assert_eq!(first, again, "one value must reuse its placeholder");
        assert_ne!(first, other, "two values must not share one");
        assert_eq!(map.distinct_count("local_path"), 2);
        assert_eq!(map.distinct_count("secret"), 0);
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol --lib one_value_gets_one_placeholder`

Expected: compile error, `no method named distinct_count`.

- [ ] **Step 3: Add `distinct_count`**

In `trace_contribution.rs`, inside `impl PlaceholderMap`:

```rust
    /// How many DISTINCT values this label has had a placeholder minted
    /// for.
    ///
    /// Not the same number as `RedactionReport`'s count for that label,
    /// which counts occurrences. One path referenced two hundred times is
    /// two hundred occurrences and one distinct value, and the second
    /// number is the one that says how much of a session's surface was
    /// really touched.
    fn distinct_count(&self, label: &str) -> u32 {
        self.next_by_label.get(label).copied().unwrap_or(0)
    }

    /// Every label's distinct-value count.
    pub(crate) fn distinct_counts(&self) -> BTreeMap<String, u32> {
        self.next_by_label.clone().into_iter().collect()
    }
```

- [ ] **Step 4: Carry it out to the envelope**

`redaction_counts` is populated from `RedactionReport`. Add the distinct map
beside it. In `trace_contribution.rs`, find the struct holding
`redaction_counts` (the `privacy` field's type) and add:

```rust
    /// Distinct values redacted, per label. See
    /// `PlaceholderMap::distinct_count` for why this is not
    /// `redaction_counts`.
    ///
    /// `#[serde(default)]` so an envelope written before this field existed
    /// still parses.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub redaction_distinct_counts: BTreeMap<String, u32>,
```

Populate it in `redact_trace`, where `report` is finalized, from
`state.placeholders.distinct_counts()`.

**Stop and check before continuing:** `envelope_digest` hashes the envelope,
and `preview.rs:101` lists the fields it covers. Adding a field to the
privacy block may move the golden digest that
`crates/trace-commons-contributor` pins. Run:

`RUSTFLAGS="-D warnings" cargo test --workspace 2>&1 | grep -i digest`

If a digest test fails, that is the pin doing its job. Recompute and update
the constant, and say so in the commit message -- the test's own comment
explains the ordering it is protecting.

- [ ] **Step 5: Surface it on the preview summary**

In `preview.rs`, add to both `PreviewSummary` and `PreviewCardSummary`,
beside their existing `redactions` field:

```rust
    /// Distinct values removed per label, beside the occurrence counts in
    /// `redactions`. A shell renders "185 local path (12 distinct)".
    pub redactions_distinct: std::collections::BTreeMap<String, u32>,
```

Populate it where `redactions` is populated (`preview.rs:634`), from
`envelope.privacy.redaction_distinct_counts.clone()`, and carry it through
`PreviewCardSummary`'s `From`/conversion at `:431`.

- [ ] **Step 6: Write the preview test**

Add to `preview.rs`'s test module, modelled on the existing
`preview_reports_what_redaction_actually_removed`:

```rust
    #[tokio::test]
    async fn preview_reports_distinct_values_beside_occurrences() {
        let summary = summary_for_session_with_a_planted_secret().await;
        let occurrences: u32 = summary.redactions.values().sum();
        let distinct: u32 = summary.redactions_distinct.values().sum();
        assert!(occurrences > 0, "the fixture must have something to redact");
        assert!(distinct > 0, "distinct counts must be reported too");
        assert!(
            distinct <= occurrences,
            "distinct ({distinct}) can never exceed occurrences ({occurrences})"
        );
    }
```

Replace `summary_for_session_with_a_planted_secret()` with whatever helper
the neighbouring test at `:1071` uses to build its summary -- reuse it
rather than writing a second fixture.

- [ ] **Step 7: Run the tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol -p trace-commons-contributor`

Expected: all pass.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-protocol/src/trace_contribution.rs \
        crates/trace-commons-contributor/src/daemon/preview.rs
git commit -m "Report distinct redacted values beside occurrence counts"
```

---

### Task 8: Count-only search over pre-redaction text

`tc_preview_search` scans the redacted body, so a value that was removed
returns zero matches -- indistinguishable from a value that was never
there. Those are the two answers a worried contributor most needs to tell
apart. This adds the one call that separates them, and it returns a number
and never a byte.

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (new request handler, beside `open_preview`)
- Modify: `crates/trace-commons-contributor-ffi/src/lib.rs:1543` (beside `tc_preview_search`)
- Modify: `crates/trace-commons-contributor-ffi/include/trace_commons.h:639`
- Modify: `macos/Sources/CTraceCommons/include/trace_commons.h` (identical edit -- CI enforces byte equality)
- Test: `crates/trace-commons-contributor/src/daemon/ipc.rs` inline tests; `crates/trace-commons-contributor-ffi/tests/`

**Interfaces:**
- Consumes: the queue's `path` field for an entry (existing).
- Produces:
  - Daemon: `pub async fn search_original(shared: &DaemonShared, entry_id:
    uuid::Uuid, needle: &str) -> Result<u32, &'static str>` -- note the
    shared-state type is `DaemonShared` (as `shared_of` in the FFI crate
    returns `Arc<ipc::DaemonShared>`), not `Shared`.
  - Daemon: request method `"search_original"`, params `{entry_id, needle}`,
    result `{ "matches": <u32> }`.
  - FFI: `pub unsafe extern "C" fn tc_search_original(handle: *mut tc_handle, entry_id: *const c_char, needle: *const c_char) -> i32`

- [ ] **Step 1: Write the failing daemon test**

Add to `ipc.rs`'s test module, following the shape of the existing
`open_preview` tests:

```rust
    #[tokio::test]
    async fn search_original_counts_a_value_that_redaction_removed() {
        // A session whose text contains a secret the redactor will replace.
        let (shared, entry_id) = fixture_with_a_planted_secret().await;

        let found = search_original(&shared, entry_id, "planted-secret-value")
            .await
            .unwrap();
        assert_eq!(found, 1, "the raw session contains it once");

        let absent = search_original(&shared, entry_id, "never-appeared-anywhere")
            .await
            .unwrap();
        assert_eq!(absent, 0);
    }

    #[tokio::test]
    async fn search_original_refuses_an_unknown_entry() {
        let (shared, _) = fixture_with_a_planted_secret().await;
        assert!(
            search_original(&shared, uuid::Uuid::new_v4(), "anything")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn search_original_returns_no_content() {
        // The whole bound of this call: a count, never bytes. Asserted on
        // the wire shape rather than on the function, because the wire is
        // what a shell can reach.
        let (shared, entry_id) = fixture_with_a_planted_secret().await;
        let value = serde_json::json!({
            "matches": search_original(&shared, entry_id, "planted-secret-value")
                .await
                .unwrap()
        });
        assert_eq!(value.as_object().unwrap().len(), 1);
        assert!(!value.to_string().contains("planted-secret-value"));
    }
```

Reuse whatever fixture the existing preview tests use to build a session
with a plantable secret -- `preview.rs`'s test module has one at `:890`
("A session with a planted secret, so redaction has something to do"). If
`ipc.rs` has no equivalent helper, write `fixture_with_a_planted_secret` in
`ipc.rs`'s test module by calling the same session-file builder those tests
use.

- [ ] **Step 2: Run it to verify it fails**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib daemon::ipc::tests::search_original`

Expected: compile error, `cannot find function search_original`.

- [ ] **Step 3: Write the daemon function**

Add to `ipc.rs`, beside `open_preview`:

```rust
/// Count occurrences of `needle` in an entry's PRE-redaction session text.
///
/// This is the only call in this crate that reads unredacted session bytes
/// on behalf of a socket client, and the bound is what makes it acceptable:
/// it returns a COUNT. No offsets, no context, no bytes, nothing that can
/// be reassembled into content. A caller learns only the answer to a
/// question they already knew how to ask, about a needle they typed
/// themselves.
///
/// It exists because `tc_preview_search` scans the REDACTED body, so a
/// value that redaction removed returns zero matches -- which is
/// indistinguishable from a value that was never in the session at all.
/// Those are precisely the two answers a contributor checking for a client
/// name needs to tell apart, and without this the search tab cannot tell
/// them apart either.
///
/// The file is read, counted, and dropped inside this function. Nothing
/// retains it. That is why this takes an entry id rather than hanging off
/// an open preview: a preview lives as long as a sheet is on screen, and an
/// unredacted transcript must not.
///
/// Errors are fixed labels, never a path or a fragment of content.
pub async fn search_original(
    shared: &DaemonShared,
    entry_id: uuid::Uuid,
    needle: &str,
) -> Result<u32, &'static str> {
    if needle.is_empty() {
        return Ok(0);
    }
    let path = {
        let queue = shared.queue.lock().expect("queue lock");
        queue
            .all()
            .iter()
            .find(|e| e.entry_id == entry_id)
            .map(|e| e.path.clone())
            .ok_or("entry-id-unknown")?
    };
    let body = tokio::fs::read_to_string(&path)
        .await
        .map_err(|_| "session-unreadable")?;
    let mut count: u32 = 0;
    let mut start = 0usize;
    while let Some(pos) = body[start..].find(needle) {
        count = count.saturating_add(1);
        start = start + pos + needle.len();
        if start > body.len() {
            break;
        }
    }
    Ok(count)
}
```

Register the method in the request `match` in `ipc.rs`, beside `"preview"`,
and add `"search_original"` to the method list at `:251`:

```rust
        "search_original" => {
            let Some(entry_id) = req
                .params
                .get("entry_id")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<uuid::Uuid>().ok())
            else {
                return Response::err(req.id, ERR_BAD_PARAMS, "entry-id-invalid");
            };
            let needle = req.params.get("needle").and_then(|v| v.as_str()).unwrap_or("");
            match search_original(shared, entry_id, needle).await {
                Ok(matches) => Response::ok(req.id, serde_json::json!({ "matches": matches })),
                Err(label) => Response::err(req.id, ERR_BAD_PARAMS, label),
            }
        }
```

- [ ] **Step 4: Run the daemon tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib daemon::ipc`

Expected: all pass.

- [ ] **Step 5: Write the failing FFI test**

Add to the FFI crate's integration tests
(`crates/trace-commons-contributor-ffi/tests/`, in the file that already
exercises `tc_preview_search`; match its harness for starting a daemon and
queueing a session):

```rust
#[test]
fn search_original_finds_what_redaction_removed() {
    let h = start_daemon_with_a_planted_secret();
    let entry_id = first_entry_id(&h);

    let needle = std::ffi::CString::new("planted-secret-value").unwrap();
    let id = std::ffi::CString::new(entry_id).unwrap();
    let found = unsafe { tc_search_original(h.raw(), id.as_ptr(), needle.as_ptr()) };
    assert_eq!(found, 1);

    let absent = std::ffi::CString::new("never-appeared-anywhere").unwrap();
    assert_eq!(
        unsafe { tc_search_original(h.raw(), id.as_ptr(), absent.as_ptr()) },
        0
    );
}

#[test]
fn search_original_refuses_a_dead_handle() {
    let needle = std::ffi::CString::new("anything").unwrap();
    let id = std::ffi::CString::new(uuid::Uuid::new_v4().to_string()).unwrap();
    assert_eq!(
        unsafe { tc_search_original(std::ptr::null_mut(), id.as_ptr(), needle.as_ptr()) },
        -1
    );
}
```

- [ ] **Step 6: Run it to verify it fails**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-ffi search_original`

Expected: compile error, `cannot find function tc_search_original`.

- [ ] **Step 7: Write the FFI function**

Add to `lib.rs`, after `tc_preview_search`:

```rust
/// Count occurrences of `needle` in an entry's PRE-redaction session text.
///
/// Returns the match count, or -1 on error. Reports a COUNT ONLY: no
/// offsets, no context, no bytes.
///
/// Takes a handle and an entry id rather than a `tc_preview*` deliberately.
/// `tc_preview` holds `body` and `summary_json`, both post-redaction, and
/// must not acquire pre-redaction bytes: hanging the raw session off the
/// preview would keep an unredacted transcript resident for as long as a
/// sheet stays open. The daemon reads the file, counts, and drops it.
///
/// # Safety
/// `handle` must be a live pointer from `tc_daemon_start` (or NULL, which
/// returns -1), and must not be freed concurrently by another thread.
/// `entry_id` and `needle` must be valid NUL-terminated C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_search_original(
    handle: *mut tc_handle,
    entry_id: *const c_char,
    needle: *const c_char,
) -> i32 {
    let outcome = guard(|| {
        if handle.is_null() {
            anyhow::bail!("null-handle");
        }
        if !handle_pointer_is_live(handle) {
            anyhow::bail!("{ERR_INVALID_HANDLE_POINTER}");
        }
        let handle = unsafe { &*handle };
        let entry_id: uuid::Uuid = unsafe { borrow_str(entry_id) }?
            .parse()
            .map_err(|_| anyhow::anyhow!("entry-id-invalid"))?;
        let needle = unsafe { borrow_str(needle) }?.to_string();
        let Some(shared) = shared_of(handle) else {
            anyhow::bail!("daemon-stopped");
        };
        // A dedicated thread with its own runtime, for the same reason
        // `tc_preview_open` uses one: this is callable from inside a
        // `tc_subscribe` callback, where `block_on` on a runtime worker
        // panics with "Cannot start a runtime from within a runtime".
        let count = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            // `ipc` is already imported at lib.rs:100.
            rt.block_on(ipc::search_original(&shared, entry_id, &needle))
            .map_err(|label| anyhow::anyhow!("{label}"))
        })
        .join()
        .map_err(|_| anyhow::anyhow!("panic"))??;
        i32::try_from(count).map_err(|_| anyhow::anyhow!("too-many-matches"))
    });
    outcome.unwrap_or_else(|e| {
        set_last_error(&e);
        -1
    })
}
```

`shared_of` returns `Arc<ipc::DaemonShared>` (lib.rs:401), which moves into
the thread. If it turns out not to be `Send`, hold the count computation on
the handle's own runtime instead, matching however `tc_preview_open` resolved
the same problem at `:1309`.

- [ ] **Step 8: Update both header copies**

Add to `crates/trace-commons-contributor-ffi/include/trace_commons.h`,
immediately after the `tc_preview_search` declaration:

```c
/* Count occurrences of needle in an entry's PRE-redaction session text.
 * Returns the match count, or -1 on error. Reports a COUNT ONLY: no
 * offsets, no context, no bytes.
 *
 * This is the one call here that reads unredacted session bytes on behalf
 * of a caller, and the count is the bound. It exists because
 * tc_preview_search scans the REDACTED body, so a value redaction removed
 * returns zero -- indistinguishable from a value that was never present.
 *
 * Takes a handle and an entry id, not a tc_preview*: a preview must not
 * hold pre-redaction bytes for as long as a sheet is open. The daemon
 * reads the file, counts, and drops it.
 */
int32_t     tc_search_original(tc_handle*, const char* entry_id, const char* needle);
```

Then copy the file verbatim to the second location:

```bash
cp crates/trace-commons-contributor-ffi/include/trace_commons.h \
   macos/Sources/CTraceCommons/include/trace_commons.h
```

Also update the "THE PREVIEW EXEMPTION" paragraph at the top of the header
(both copies), which currently says the exemption is "bounded to
post-redaction content only". Append:

```
 * One narrow addition: tc_search_original answers a yes/no-with-a-number
 * question about pre-redaction content. It returns a count and never a
 * byte, for a needle the caller supplied, on an entry they already hold.
```

- [ ] **Step 9: Run the FFI tests and the header check**

Run:
```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-ffi
diff crates/trace-commons-contributor-ffi/include/trace_commons.h \
     macos/Sources/CTraceCommons/include/trace_commons.h && echo "headers identical"
```

Expected: tests pass, and `headers identical`.

- [ ] **Step 10: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/daemon/ipc.rs \
        crates/trace-commons-contributor-ffi/src/lib.rs \
        crates/trace-commons-contributor-ffi/include/trace_commons.h \
        macos/Sources/CTraceCommons/include/trace_commons.h \
        crates/trace-commons-contributor-ffi/tests
git commit -m "Answer whether a value was removed, with a count and no content"
```

---

### Task 9: Full verification

Nothing new is built here. This is the gate before the shell plans start
consuming these interfaces, and it runs what CI runs.

- [ ] **Step 1: Format**

```bash
cargo fmt --all -- --check
```

Expected: no output. If it complains, run `cargo fmt --all` and commit the
result.

- [ ] **Step 2: Warnings-as-errors check, default features**

```bash
RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
```

Expected: clean.

- [ ] **Step 3: Clippy, with the repo's allow-list**

```bash
cargo clippy --workspace --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

Expected: clean. Do not widen the allow-list to make it clean.

- [ ] **Step 4: The full test suite**

```bash
RUSTFLAGS="-D warnings" cargo test --workspace
```

Expected: all pass. Paste the summary line into the PR body -- no success
claim without it.

- [ ] **Step 5: The license boundary**

```bash
cargo test -p trace-commons-server --test license_boundary
```

Expected: pass. This plan added no dependencies, so it should be untouched;
running it is how you know.

- [ ] **Step 6: Confirm the Swift package still builds against the header**

The macOS app links the FFI dylib and imports the header this plan edited,
and `swift test` is the only thing that exercises it.

```bash
cargo build -p trace-commons-contributor-ffi
cd macos && swift build
```

Expected: builds. A failure here means the header copy drifted from the Rust
signature.

- [ ] **Step 7: Open the PR**

```bash
git push -u origin shell-ux-feedback
gh pr create --repo zmanian/trace-commons-server \
  --title "Daemon and FFI foundation for the folder-first contributor queue" \
  --body "Implements docs/superpowers/plans/2026-09-03-contributor-shell-ux-foundation.md.

Normalizes project keys so one directory is one project regardless of how an
agent recorded it, re-keys the policy file on upgrade taking the more
restrictive mode on a merge, reports a project path on the IPC socket and
nowhere else, carries the opaque project id on history records, reports
distinct redacted values beside occurrence counts, and adds a count-only
search over pre-redaction text.

No shell code is touched; the three shell plans build on these interfaces.

Spec: docs/superpowers/specs/2026-09-03-contributor-shell-queue-ux-design.md"
```

---

## Self-Review

**Spec coverage.**

| Spec section | Task |
|---|---|
| §1.1 project path on the socket | Task 5 |
| §1.1 sink test | Task 5 Step 4, completed by Task 6 |
| §1.2 normalization steps 1-4 | Task 1, wired in Task 2 |
| §1.2 `session_path` on the entry | Task 4 (field), Task 5 (wire) |
| §1.2 policy re-key migration, more-restrictive merge | Task 3 |
| §3.1 distinct-value counts | Task 7 |
| §3.2 `tc_search_original` | Task 8 |
| §5 `project_id` on `HistoryRecord` | Task 6 |
| §6 testing | Distributed through each task; Task 9 runs the CI set |
| §2, §3.1 chips, §3.3, §3.4, §4, §5 rendering | **Not in this plan** -- shell work, plans 2-4 |

**One gap accepted deliberately.** The spec's §1.2 mentions case-folding on
case-insensitive volumes; Task 1 implements it platform-gated
(macOS/Windows) rather than by probing the volume, and says why in the
code comment. Probing would be more precise and is not worth a filesystem
round-trip per session.

**Two places the implementer will have to look rather than copy.** Task 4
Step 5, Task 6 Step 5, and Task 8 Step 1's fixture all say "find the call
site" or "reuse the neighbouring helper" rather than quoting code. That is
because the construction sites are in files this plan does not otherwise
touch and quoting them would go stale; each step gives the exact grep.

**Type consistency check.** `normalize_project_key` /
`normalize_project_key_within` (Task 1) are consumed by `project_key_for`
(Task 2) and `rekey` (Task 3) with matching signatures.
`ProjectMode::more_restrictive` is defined in Task 3 Step 3 and used in Task
3 Step 3 only. `QueueEntry.session_cwd` (Task 4) is read by `entry_value`
(Task 5). `HistoryRecord.project_id` (Task 6) is written by Task 5's test
and by Task 6 Step 5. `display_path` (Task 5) is used in Task 5 only.
`search_original` (Task 8 Step 3) is called by `tc_search_original` (Task 8
Step 7) with `(&Shared, Uuid, &str) -> Result<u32, &'static str>` in both.

**One risk flagged in-plan rather than resolved.** Task 7 Step 4 may move a
pinned envelope digest. The plan tells the implementer to expect it, how to
detect it, and to say so in the commit rather than silently updating the
constant.
