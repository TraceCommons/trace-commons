//! Which projects may upload without asking.
//!
//! Autonomy is per-project and opt-in. An unknown project is `NotifyOnly`, so
//! a freshly installed daemon uploads nothing until the contributor has
//! deliberately said otherwise about a specific project.
//!
//! The policy key is the session's true working directory. It is deliberately
//! *not* derived from the project basename: Claude Code encodes a cwd into a
//! directory name by replacing every `/` with `-`, which is ambiguous for any
//! hyphenated project name, and guessing wrong here would apply one project's
//! autonomy to a different project's traces.
//!
//! Sessions whose working directory cannot be resolved go to a single locked
//! bucket that can never be granted autonomy. Subagent transcripts and
//! normalized trajectory files land there. Since the daemon cannot tell which
//! project such a session belongs to, it cannot honour any opt-in for it.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{ConfigStore, DAEMON_PROJECTS_FILE};

pub const DAEMON_PROJECTS_SCHEMA: &str = "trace_commons.daemon_projects.v1";

/// The bucket for sessions with no resolvable working directory. Permanently
/// notify-only.
pub const UNKNOWN_PROJECT_KEY: &str = "unknown-project";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectMode {
    /// Upload without asking.
    AutoUpload,
    /// Queue it and mention it in the next digest.
    NotifyOnly,
    /// Never offer sessions from this project at all.
    Ignore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub mode: ProjectMode,
    pub added_at: DateTime<Utc>,
    /// Display name for consumers. Shells render this; they never render the
    /// key, which is a full local path.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPolicy {
    pub schema_version: String,
    pub projects: BTreeMap<String, ProjectEntry>,
}

impl Default for ProjectPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectPolicy {
    pub fn new() -> Self {
        Self {
            schema_version: DAEMON_PROJECTS_SCHEMA.to_string(),
            projects: BTreeMap::new(),
        }
    }

    pub fn load(store: &ConfigStore) -> Result<Self> {
        let Some(body) = store.read_daemon_file(DAEMON_PROJECTS_FILE)? else {
            return Ok(Self::new());
        };
        serde_json::from_slice(&body).context("parsing daemon project policy")
    }

    pub fn save(&self, store: &ConfigStore) -> Result<()> {
        let body = serde_json::to_vec_pretty(self).context("serializing daemon project policy")?;
        store.write_daemon_file(DAEMON_PROJECTS_FILE, &body)
    }

    /// The mode in force for a project key.
    ///
    /// Autonomy for the unknown-cwd bucket is denied here, after the stored
    /// map is consulted rather than before it, so a hand-edited or tampered
    /// policy file cannot grant autonomy to sessions the daemon cannot
    /// attribute. `Ignore` is still honoured for that bucket: refusing to
    /// upload it unattended is not a reason to refuse to silence it.
    pub fn resolve(&self, project_key: &str) -> ProjectMode {
        let stored = self
            .projects
            .get(project_key)
            .map(|e| e.mode)
            .unwrap_or(ProjectMode::NotifyOnly);
        if project_key == UNKNOWN_PROJECT_KEY && stored == ProjectMode::AutoUpload {
            return ProjectMode::NotifyOnly;
        }
        stored
    }

    /// Record a mode for `project_key`.
    ///
    /// The label is **derived here**, from the key, and is never a caller
    /// argument. It used to be one, and `set_project_mode` passed straight
    /// through whatever a socket client sent -- so any client could write
    /// an arbitrary string (a full filesystem path, a token, a fragment of
    /// somebody's transcript) into `list_projects` output and into
    /// `daemon-audit.jsonl`, the two sinks this crate's label-only rule
    /// exists to protect. Deriving it removes the injection path by
    /// construction rather than by validation.
    ///
    /// The stored label is the bare basename (`project_label_for`);
    /// disambiguation against colliding basenames happens at render time,
    /// so a stored label never goes stale when a colliding project appears
    /// later.
    pub fn set_mode(
        &mut self,
        project_key: &str,
        mode: ProjectMode,
        now: DateTime<Utc>,
    ) -> Result<()> {
        if project_key == UNKNOWN_PROJECT_KEY && mode == ProjectMode::AutoUpload {
            bail!(
                "unknown-project sessions cannot be set to auto_upload: \
                 their working directory could not be resolved, so no \
                 per-project opt-in can apply to them"
            );
        }
        self.projects.insert(
            project_key.to_string(),
            ProjectEntry {
                mode,
                added_at: now,
                label: project_label_for(project_key),
            },
        );
        Ok(())
    }
}

/// The fixed label `set_project_mode` refuses an unrecognized key with.
pub const ERR_PROJECT_KEY_UNRECOGNIZED: &str = "project-key-unrecognized";

/// Whether the daemon will accept `project_key` from a socket client.
///
/// A socket client used to be able to name any key at all. The key is not
/// itself echoed anywhere, but its *basename* becomes the project label --
/// which crosses the socket in `list_projects` and lands in
/// `daemon-audit.jsonl` -- so `"/x/ghp_realtokenvalue"` put a token into
/// both sinks with one call.
///
/// A key is admissible when it is one of:
///
/// * the locked unknown-cwd sentinel (which `set_mode` still refuses to
///   arm, and whose label is the sentinel name, not a path segment);
/// * a key the daemon already knows -- one it has discovered on a queued
///   session, or one already in the policy file, so its label is one the
///   daemon itself derived;
/// * an absolute path that exists on this machine as a directory and
///   canonicalizes to itself. This is the only admissible *new* key, and it
///   is exactly what both producers of keys emit: the watcher takes the
///   cwd an agent recorded, and `daemon project <path>` canonicalizes an
///   existing directory. Keeping it means a project can still be set to
///   `ignore` (or armed) before its first session is ever seen, which is
///   the whole point of that CLI flow.
///
/// Everything else is refused with `ERR_PROJECT_KEY_UNRECOGNIZED`. A caller
/// can therefore still choose *which* local directory it names, but it can
/// no longer conjure a label out of an arbitrary string.
pub fn project_key_is_admissible(project_key: &str, known_keys: &[String]) -> bool {
    if project_key == UNKNOWN_PROJECT_KEY {
        return true;
    }
    if known_keys.iter().any(|k| k == project_key) {
        return true;
    }
    let path = std::path::Path::new(project_key);
    if !path.is_absolute() {
        return false;
    }
    match std::fs::canonicalize(path) {
        Ok(resolved) => resolved.is_dir() && resolved.as_os_str() == path.as_os_str(),
        Err(_) => false,
    }
}

/// Whether a working directory yields a usable display label -- i.e. has a
/// final path segment at all.
///
/// `Path::file_name` returns `None` for `/`, for anything ending in `..`,
/// and for the empty string. Every one of those is a real cwd a coding
/// agent can record.
fn has_usable_basename(cwd: &str) -> bool {
    std::path::Path::new(cwd)
        .file_name()
        .is_some_and(|n| !n.is_empty())
}

/// The policy key for a session: its true working directory, or the locked
/// unknown bucket. Never falls back to a basename heuristic.
///
/// A cwd with no usable final path segment goes to the unknown bucket
/// rather than becoming a key of its own. Such a key has no label but
/// itself, and `project_label` crosses the socket, lands in
/// `daemon-audit.jsonl`, in OS notification text, and in `HistoryRecord` --
/// so the fallback turned a full local path into every one of those, in
/// direct violation of the invariant `audit`'s own
/// `an_audit_entry_never_carries_a_path` test asserts. It can also never be
/// armed, which is the right answer for a directory the daemon cannot even
/// name.
pub fn project_key_for(cwd: Option<&str>) -> String {
    match cwd {
        Some(cwd) if !cwd.trim().is_empty() && has_usable_basename(cwd) => cwd.to_string(),
        _ => UNKNOWN_PROJECT_KEY.to_string(),
    }
}

/// A display label for a project key: the final path segment, or the bucket
/// name. Consumers render this instead of the key, which is a local path.
///
/// A key with no final path segment reports the bucket name rather than
/// echoing the key. `project_key_for` already prevents such a key from
/// being created, so this is the second line of defence -- it covers a
/// hand-edited or older `daemon-projects.json`, whose keys reach here
/// having never gone through `project_key_for` at all. Under no
/// circumstances does a raw path leave this function.
pub fn project_label_for(project_key: &str) -> String {
    if project_key == UNKNOWN_PROJECT_KEY || !has_usable_basename(project_key) {
        return UNKNOWN_PROJECT_KEY.to_string();
    }
    std::path::Path::new(project_key)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| UNKNOWN_PROJECT_KEY.to_string())
}

/// The known-key set every disambiguation call site must agree on: every
/// configured project plus every project already sitting in the queue.
/// `queue_project_keys` takes an iterator rather than a `Queue` so this
/// module does not need to depend on `queue`; call sites pass
/// `queue.all().iter().map(|e| e.project_key.clone())`.
pub fn known_keys(
    policy: &ProjectPolicy,
    queue_project_keys: impl Iterator<Item = String>,
) -> Vec<String> {
    policy
        .projects
        .keys()
        .cloned()
        .chain(queue_project_keys)
        .collect()
}

/// A display label unique within `known_keys`. Adds a short stable hash
/// suffix only when the basename collides.
///
/// The suffix is derived from `sha256(project_key)`, never from any path
/// segment: labels cross the IPC socket and must never leak which directory
/// a colliding project lives in.
pub fn disambiguated_label(project_key: &str, known_keys: &[String]) -> String {
    let label = project_label_for(project_key);
    if project_key == UNKNOWN_PROJECT_KEY {
        return label;
    }

    let collides = known_keys
        .iter()
        .any(|other| other != project_key && project_label_for(other) == label);
    if !collides {
        return label;
    }

    let digest = Sha256::digest(project_key.as_bytes());
    let suffix = hex_prefix(&digest, 4);
    format!("{label} ({suffix})")
}

fn hex_prefix(bytes: &[u8], chars: usize) -> String {
    bytes
        .iter()
        .flat_map(|b| [b >> 4, b & 0x0f])
        .take(chars)
        .map(|nibble| char::from_digit(nibble as u32, 16).expect("nibble is < 16"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;

    fn now() -> DateTime<Utc> {
        "2026-08-08T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn an_unknown_project_defaults_to_notify_only() {
        let p = ProjectPolicy::new();
        assert_eq!(
            p.resolve("/Users/z/code/never-seen"),
            ProjectMode::NotifyOnly
        );
    }

    #[test]
    fn sessions_without_a_cwd_land_in_the_unknown_bucket() {
        assert_eq!(project_key_for(None), UNKNOWN_PROJECT_KEY);
        assert_eq!(project_key_for(Some("   ")), UNKNOWN_PROJECT_KEY);
        assert_eq!(
            project_key_for(Some("/Users/z/code/proj")),
            "/Users/z/code/proj"
        );
    }

    #[test]
    fn the_unknown_bucket_cannot_be_set_to_auto_upload() {
        let mut p = ProjectPolicy::new();
        let err = p
            .set_mode(UNKNOWN_PROJECT_KEY, ProjectMode::AutoUpload, now())
            .unwrap_err();
        assert!(err.to_string().contains("unknown-project"));
        assert_eq!(p.resolve(UNKNOWN_PROJECT_KEY), ProjectMode::NotifyOnly);
    }

    #[test]
    fn the_unknown_bucket_stays_notify_only_even_if_the_file_says_auto() {
        // Defence against a hand-edited or tampered policy file.
        let mut p = ProjectPolicy::new();
        p.projects.insert(
            UNKNOWN_PROJECT_KEY.to_string(),
            ProjectEntry {
                mode: ProjectMode::AutoUpload,
                added_at: now(),
                label: "unknown".into(),
            },
        );
        assert_eq!(p.resolve(UNKNOWN_PROJECT_KEY), ProjectMode::NotifyOnly);
    }

    #[test]
    fn the_unknown_bucket_may_still_be_ignored() {
        // Refusing autonomy is not the same as refusing to be silenced.
        let mut p = ProjectPolicy::new();
        p.set_mode(UNKNOWN_PROJECT_KEY, ProjectMode::Ignore, now())
            .unwrap();
        assert_eq!(p.resolve(UNKNOWN_PROJECT_KEY), ProjectMode::Ignore);
    }

    #[test]
    fn set_and_resolve_round_trip_for_a_real_project() {
        let mut p = ProjectPolicy::new();
        p.set_mode("/Users/z/code/proj", ProjectMode::AutoUpload, now())
            .unwrap();
        assert_eq!(p.resolve("/Users/z/code/proj"), ProjectMode::AutoUpload);
        p.set_mode("/Users/z/code/proj", ProjectMode::Ignore, now())
            .unwrap();
        assert_eq!(p.resolve("/Users/z/code/proj"), ProjectMode::Ignore);
    }

    #[test]
    fn labels_are_basenames_so_consumers_never_need_the_path() {
        assert_eq!(project_label_for("/Users/z/code/my-proj"), "my-proj");
        assert_eq!(project_label_for(UNKNOWN_PROJECT_KEY), UNKNOWN_PROJECT_KEY);
    }

    #[test]
    fn a_cwd_with_no_usable_basename_goes_to_the_unknown_bucket() {
        // `Path::file_name` is None for every one of these, and each is a
        // real cwd an agent can record. Before this, the key became the raw
        // path and so did the label -- which then crossed the socket,
        // landed in daemon-audit.jsonl, in OS notification text, and in
        // HistoryRecord.
        for cwd in ["/", "/Users/z/code/..", "..", ""] {
            assert_eq!(
                project_key_for(Some(cwd)),
                UNKNOWN_PROJECT_KEY,
                "cwd {cwd:?} must not become a policy key of its own"
            );
        }
    }

    #[test]
    fn a_degenerate_key_never_renders_as_a_raw_path() {
        // Second line of defence: a hand-edited or older
        // daemon-projects.json can hold such a key without it ever having
        // gone through `project_key_for`.
        for key in ["/", "/Users/z/secret-client/..", ".."] {
            let label = project_label_for(key);
            assert_eq!(label, UNKNOWN_PROJECT_KEY, "key {key:?} leaked as {label}");
            assert!(!label.contains('/'), "key {key:?} leaked as {label}");
        }
    }

    #[test]
    fn a_degenerate_key_is_never_suffixed_into_a_path_either() {
        // `disambiguated_label` only suffixes a hash, but it starts from
        // `project_label_for`, so a leak there would leak through here too.
        let keys = vec!["/".to_string(), "/Users/z/client/..".to_string()];
        for key in &keys {
            let label = disambiguated_label(key, &keys);
            assert!(!label.contains('/'), "{key} leaked as {label}");
        }
    }

    #[test]
    fn a_degenerate_key_cannot_be_armed() {
        // It resolves to the locked bucket, which `resolve` refuses to
        // report as AutoUpload however the file was written.
        let mut p = ProjectPolicy::new();
        assert!(
            p.set_mode("/", ProjectMode::AutoUpload, now()).is_ok(),
            "the key itself is not the sentinel, so set_mode does not refuse it"
        );
        // But no session can ever resolve to it: every cwd with no usable
        // basename is bucketed before policy is consulted.
        assert_eq!(project_key_for(Some("/")), UNKNOWN_PROJECT_KEY);
        assert_eq!(p.resolve(UNKNOWN_PROJECT_KEY), ProjectMode::NotifyOnly);
    }

    #[test]
    fn policy_round_trips_through_the_store() {
        let (_d, store) = temp_store();
        let mut p = ProjectPolicy::new();
        p.set_mode("/Users/z/code/proj", ProjectMode::AutoUpload, now())
            .unwrap();
        p.save(&store).unwrap();
        assert_eq!(ProjectPolicy::load(&store).unwrap(), p);
    }

    #[test]
    fn policy_defaults_when_the_file_is_absent() {
        let (_d, store) = temp_store();
        assert_eq!(ProjectPolicy::load(&store).unwrap(), ProjectPolicy::new());
    }

    #[test]
    fn a_unique_basename_is_left_alone() {
        let keys = vec![
            "/Users/z/code/alpha".to_string(),
            "/Users/z/code/beta".to_string(),
        ];
        assert_eq!(disambiguated_label("/Users/z/code/alpha", &keys), "alpha");
    }

    #[test]
    fn colliding_basenames_get_distinct_stable_suffixes() {
        // The dangerous case: one of these is the client's repo.
        let keys = vec![
            "/Users/z/work/api".to_string(),
            "/Users/z/client/api".to_string(),
        ];
        let a = disambiguated_label("/Users/z/work/api", &keys);
        let b = disambiguated_label("/Users/z/client/api", &keys);
        assert_ne!(a, b, "colliding projects must be distinguishable");
        assert!(a.starts_with("api ("), "got {a}");
        assert_eq!(
            a,
            disambiguated_label("/Users/z/work/api", &keys),
            "must be stable"
        );
    }

    #[test]
    fn a_suffix_never_contains_a_path_segment() {
        // The suffix is a hash, not a directory name: paths never cross the wire.
        let keys = vec![
            "/Users/z/work/api".to_string(),
            "/Users/z/client/api".to_string(),
        ];
        let a = disambiguated_label("/Users/z/work/api", &keys);
        assert!(!a.contains("work") && !a.contains('/'), "got {a}");
    }

    #[test]
    fn the_unknown_bucket_is_never_suffixed() {
        let keys = vec![UNKNOWN_PROJECT_KEY.to_string()];
        assert_eq!(
            disambiguated_label(UNKNOWN_PROJECT_KEY, &keys),
            UNKNOWN_PROJECT_KEY
        );
    }
}
