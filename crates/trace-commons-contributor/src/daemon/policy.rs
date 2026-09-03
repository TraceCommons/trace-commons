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
use chrono::{DateTime, Duration, Utc};
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

/// How many times a project must have contributed before the app offers to
/// arm it.
///
/// Five, because the offer has to be backed by evidence the contributor
/// actually has. Arming asks someone to stop reading previews from a
/// project; the only honest basis for that question is that they have read
/// several already and kept approving. One or two is a coincidence.
pub const ARMING_SUGGESTION_THRESHOLD: u32 = 5;

/// How long "Not now" silences the offer for one project.
///
/// Thirty days, which is the difference between an offer and nagging. It is
/// deliberately not permanent: "Not now" says not now, and a suppression
/// that never lifts would make those words a lie. Settings remains the way
/// to arm a project at any point in between, without being asked.
pub const ARMING_DECLINE_COOLDOWN_DAYS: i64 = 30;

/// A project the app should offer to arm, and the evidence for offering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmingSuggestion {
    pub project_id: String,
    pub project_label: String,
    pub contributed_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPolicy {
    pub schema_version: String,
    pub projects: BTreeMap<String, ProjectEntry>,
    /// Successful uploads per project key, counted for the arming offer.
    ///
    /// Kept here rather than derived from the history cache because history
    /// is label-only by design -- it never carries a project key, so two
    /// projects sharing a final path segment are indistinguishable in it.
    /// Offering to arm the wrong repository on the strength of an ambiguous
    /// label is exactly the mistake this counter exists to avoid.
    ///
    /// `#[serde(default)]` so a policy file written before this existed
    /// still parses, and reads as "nothing counted yet" rather than failing
    /// the whole file.
    #[serde(default)]
    pub contributed: BTreeMap<String, u32>,
    /// When the contributor last said "Not now" to arming a project.
    #[serde(default)]
    pub arming_declined_at: BTreeMap<String, DateTime<Utc>>,
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
            contributed: BTreeMap::new(),
            arming_declined_at: BTreeMap::new(),
        }
    }

    /// Count one successful upload against its project.
    ///
    /// The unresolvable bucket is never counted. It can never be armed --
    /// `set_mode` and `resolve` both refuse it -- so a count for it could
    /// only ever feed an offer that cannot be delivered.
    pub fn record_contribution(&mut self, project_key: &str) {
        if project_key == UNKNOWN_PROJECT_KEY {
            return;
        }
        *self.contributed.entry(project_key.to_string()).or_insert(0) += 1;
    }

    /// Record a "Not now" against one project.
    pub fn decline_arming(&mut self, project_key: &str, now: DateTime<Utc>) {
        self.arming_declined_at.insert(project_key.to_string(), now);
    }

    /// The one project worth offering to arm right now, if any.
    ///
    /// At most one, deliberately. A queue that sprouts an offer per project
    /// is the ongoing administration the contributor asked to be rid of; the
    /// strongest single candidate is the whole of what this ever asks.
    ///
    /// A project qualifies when it has contributed at least
    /// [`ARMING_SUGGESTION_THRESHOLD`] times, is still ask-first (an armed
    /// project has nothing to offer and an ignored one has been answered
    /// already), can be armed at all, and has not been declined inside
    /// [`ARMING_DECLINE_COOLDOWN_DAYS`].
    pub fn arming_suggestion(&self, now: DateTime<Utc>) -> Option<ArmingSuggestion> {
        let cooldown = Duration::days(ARMING_DECLINE_COOLDOWN_DAYS);
        self.contributed
            .iter()
            .filter(|(key, count)| {
                **count >= ARMING_SUGGESTION_THRESHOLD
                    && key.as_str() != UNKNOWN_PROJECT_KEY
                    && self.resolve(key) == ProjectMode::NotifyOnly
                    && match self.arming_declined_at.get(*key) {
                        // A clock that went backwards lands here as "still
                        // inside the cooldown", which can only ever suppress
                        // an offer, never add one.
                        Some(declined) => now.signed_duration_since(*declined) >= cooldown,
                        None => true,
                    }
            })
            // Most contributions first; the key breaks a tie so the answer is
            // stable across runs rather than depending on map iteration.
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(key, count)| ArmingSuggestion {
                project_id: project_id_for(key),
                project_label: project_label_for(key),
                contributed_count: *count,
            })
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
/// Everything else is refused with `ERR_PROJECT_KEY_UNRECOGNIZED`.
///
/// What this does and does not buy, stated precisely, because an earlier
/// version of this comment claimed more than the code delivers. It does not
/// make an arbitrary label impossible: same-user code can `mkdir
/// /tmp/<any-string>` and then name that directory, and the basename
/// becomes a label in `list_projects` and `daemon-audit.jsonl`. What it
/// buys is that the string must first exist as a real directory on this
/// machine, which bounds it to what a filesystem will accept -- no `/`, no
/// NUL, at most 255 bytes -- and leaves it visible on disk. That is a
/// narrowing, not a seal, and the same surface is already reachable by
/// writing a session file with an arbitrary `cwd`. `MAX_PROJECT_LABEL_CHARS`
/// bounds the length independently at the render sinks, since a filesystem
/// limit is not a promise this crate makes.
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

/// The prefix every opaque project id carries.
///
/// It exists so the two identifier spaces `set_project_mode` accepts can
/// never be confused for one another: an id always starts with this and a
/// project key never does (a key is either an absolute path or the
/// `unknown-project` sentinel).
pub const PROJECT_ID_PREFIX: &str = "proj_";

/// Hex characters of SHA-256 carried in an opaque project id. 16 nibbles is
/// 64 bits, which is far more than enough to keep the handful of projects on
/// one contributor's machine distinct, and short enough to be pasted into a
/// bug report.
const PROJECT_ID_HEX_CHARS: usize = 16;

/// The fixed label `set_project_mode` refuses an unrecognized id with.
pub const ERR_PROJECT_ID_UNRECOGNIZED: &str = "project-id-unrecognized";

/// The opaque, daemon-issued identifier for a project.
///
/// This exists because a socket client could not previously name a project
/// at all. The privacy rule is that a project key -- a local filesystem path
/// -- never crosses the socket, so queue entries and `list_projects` rows
/// carry only `project_label`. But a label is not an admissible key, and
/// `project_key_is_admissible` (rightly) refuses anything that is not a real
/// path the daemon can corroborate. A GUI therefore held nothing it could
/// pass to `set_project_mode`, which made arming and ignoring a project
/// unreachable from every application this contract exists to serve.
///
/// The id is a hash of the key, not an encoding of it: it is one-way, so it
/// leaks no path component, and it is deterministic, so it is the same
/// across a daemon restart and across a policy file rebuilt from scratch --
/// nothing is stored to make it stable, because nothing needs to be.
///
/// It is *not* a capability. Knowing an id confers nothing that naming the
/// directory did not already confer; it is an identifier a client can hold,
/// and the daemon still resolves it only against projects it already knows.
pub fn project_id_for(project_key: &str) -> String {
    let digest = Sha256::digest(project_key.as_bytes());
    format!(
        "{PROJECT_ID_PREFIX}{}",
        hex_prefix(&digest, PROJECT_ID_HEX_CHARS)
    )
}

/// Resolve an opaque project id back to the key it was minted from, or
/// `None` if no project the daemon knows about has that id.
///
/// Resolution is by re-deriving ids over the known-key set rather than by
/// any stored mapping, which is what keeps ids stable with nothing to
/// migrate. The unknown-cwd sentinel is always resolvable -- it is a
/// permanent bucket rather than a discovered project, and a client that sees
/// it in a queue entry must be able to silence it -- while `set_mode` still
/// refuses to arm it.
///
/// An id can only ever name a project the daemon already knows. That is the
/// deliberate asymmetry with `project_key`: a path can name a project that
/// has never been seen (the CLI's `daemon project <path> --mode ignore`
/// before that project's first session), and an id cannot, because the
/// daemon cannot mint an id for something it has never discovered.
pub fn project_key_for_id(project_id: &str, known_keys: &[String]) -> Option<String> {
    if !project_id.starts_with(PROJECT_ID_PREFIX) {
        return None;
    }
    std::iter::once(UNKNOWN_PROJECT_KEY.to_string())
        .chain(known_keys.iter().cloned())
        .find(|key| project_id_for(key) == project_id)
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
    let label = std::path::Path::new(project_key)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| UNKNOWN_PROJECT_KEY.to_string());
    truncate_label(&label)
}

/// The ceiling on a rendered project label, in characters.
///
/// The label crosses the socket, lands in `daemon-audit.jsonl`, and goes
/// into OS notification text. Nothing bounded its length: a directory name
/// can be up to 255 bytes on every filesystem this runs on, and
/// `project_key_is_admissible` accepts any real directory, so a caller
/// willing to `mkdir` could put 255 bytes of chosen text into all three
/// sinks. That is not a leak of anything the caller did not already know,
/// but a label is a display name and it should be bounded here rather than
/// by whatever the filesystem happened to allow.
pub const MAX_PROJECT_LABEL_CHARS: usize = 64;

/// Truncate on a character boundary, never mid-codepoint.
fn truncate_label(label: &str) -> String {
    if label.chars().count() <= MAX_PROJECT_LABEL_CHARS {
        return label.to_string();
    }
    label.chars().take(MAX_PROJECT_LABEL_CHARS).collect()
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
        // A project the daemon has uploaded from is one it knows, whether or
        // not the contributor has ever set a mode for it and whether or not
        // it still has anything queued. Without this, a project could be
        // offered for arming -- the offer is built from exactly this counter
        // -- and then the answer refused as an unrecognized id, because
        // nothing else on the machine still remembered the key.
        .chain(policy.contributed.keys().cloned())
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
        // Lowercase on the case-folding platforms, which is what
        // `project_key::normalize_project_key` produces there.
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert_eq!(
            project_key_for(Some("/Users/z/code/proj")),
            "/users/z/code/proj"
        );
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
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
    fn a_label_is_length_bounded_at_the_sink() {
        // `project_key_is_admissible` accepts any real directory, and a
        // directory name can be 255 bytes. The label crosses the socket,
        // lands in daemon-audit.jsonl, and goes into notification text, so
        // it is bounded here rather than by whatever the filesystem allowed.
        let long = "n".repeat(255);
        let label = project_label_for(&format!("/Users/z/code/{long}"));
        assert_eq!(label.chars().count(), MAX_PROJECT_LABEL_CHARS);
    }

    #[test]
    fn a_multibyte_label_is_truncated_on_a_character_boundary() {
        let long = "\u{e9}".repeat(200);
        let label = project_label_for(&format!("/Users/z/code/{long}"));
        assert_eq!(label.chars().count(), MAX_PROJECT_LABEL_CHARS);
        assert!(label.chars().all(|c| c == '\u{e9}'));
    }

    #[test]
    fn a_short_label_is_left_exactly_as_it_is() {
        assert_eq!(project_label_for("/Users/z/code/my-proj"), "my-proj");
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
    fn a_project_id_leaks_no_path_component() {
        // The whole reason the label-only rule exists: this key names a
        // client the contributor may not disclose. The id crosses the same
        // socket the key is forbidden from crossing.
        let key = "/Users/z/clients/acme-secret-merger/api";
        let id = project_id_for(key);
        for fragment in ["acme", "secret", "merger", "clients", "api", "Users", "/"] {
            assert!(!id.contains(fragment), "{id} leaked {fragment}");
        }
        assert!(id.starts_with(PROJECT_ID_PREFIX), "got {id}");
        assert_eq!(id.len(), PROJECT_ID_PREFIX.len() + PROJECT_ID_HEX_CHARS);
    }

    #[test]
    fn a_project_id_is_deterministic_and_distinct_per_project() {
        // Deterministic: nothing is stored to make it stable, so it is the
        // same after a restart and after a policy file rebuilt from scratch.
        assert_eq!(
            project_id_for("/Users/z/code/proj"),
            project_id_for("/Users/z/code/proj")
        );
        assert_ne!(
            project_id_for("/Users/z/work/api"),
            project_id_for("/Users/z/client/api"),
            "colliding basenames must still get distinct ids"
        );
    }

    #[test]
    fn a_project_id_resolves_back_to_the_key_that_minted_it() {
        let keys = vec![
            "/Users/z/work/api".to_string(),
            "/Users/z/client/api".to_string(),
        ];
        for key in &keys {
            assert_eq!(
                project_key_for_id(&project_id_for(key), &keys).as_deref(),
                Some(key.as_str())
            );
        }
    }

    #[test]
    fn an_id_for_a_project_the_daemon_does_not_know_resolves_to_nothing() {
        let keys = vec!["/Users/z/work/api".to_string()];
        assert_eq!(
            project_key_for_id(&project_id_for("/Users/z/never/seen"), &keys),
            None
        );
        // A label is not an id, and neither is a path or a bare string.
        for bogus in ["api", "/Users/z/work/api", "proj_deadbeefdeadbeef", ""] {
            assert_eq!(project_key_for_id(bogus, &keys), None, "accepted {bogus}");
        }
    }

    #[test]
    fn the_unknown_bucket_has_a_resolvable_id_even_when_nothing_is_known() {
        // It is a permanent bucket rather than a discovered project, and a
        // client seeing it on a queue entry must be able to silence it.
        assert_eq!(
            project_key_for_id(&project_id_for(UNKNOWN_PROJECT_KEY), &[]).as_deref(),
            Some(UNKNOWN_PROJECT_KEY)
        );
    }

    #[test]
    fn an_id_survives_a_policy_file_rebuilt_from_scratch() {
        let (_d, store) = temp_store();
        let key = "/Users/z/code/proj";
        let mut p = ProjectPolicy::new();
        p.set_mode(key, ProjectMode::Ignore, now()).unwrap();
        p.save(&store).unwrap();
        let before = project_id_for(key);

        let reloaded = ProjectPolicy::load(&store).unwrap();
        let reloaded_key = reloaded.projects.keys().next().unwrap();
        assert_eq!(project_id_for(reloaded_key), before);
    }

    #[test]
    fn the_unknown_bucket_is_never_suffixed() {
        let keys = vec![UNKNOWN_PROJECT_KEY.to_string()];
        assert_eq!(
            disambiguated_label(UNKNOWN_PROJECT_KEY, &keys),
            UNKNOWN_PROJECT_KEY
        );
    }

    fn armed_policy() -> ProjectPolicy {
        let mut p = ProjectPolicy::new();
        for _ in 0..ARMING_SUGGESTION_THRESHOLD {
            p.record_contribution("/Users/z/code/api");
        }
        p
    }

    fn t(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn a_project_is_offered_once_it_has_contributed_enough() {
        let p = armed_policy();
        let s = p.arming_suggestion(t("2026-08-31T12:00:00Z")).unwrap();
        assert_eq!(s.project_label, "api");
        assert_eq!(s.contributed_count, ARMING_SUGGESTION_THRESHOLD);
        assert_eq!(s.project_id, project_id_for("/Users/z/code/api"));
    }

    /// The offer has to be backed by evidence the contributor actually has.
    #[test]
    fn a_project_below_the_threshold_is_not_offered() {
        let mut p = ProjectPolicy::new();
        for _ in 0..(ARMING_SUGGESTION_THRESHOLD - 1) {
            p.record_contribution("/Users/z/code/api");
        }
        assert!(p.arming_suggestion(t("2026-08-31T12:00:00Z")).is_none());
    }

    /// An armed project has nothing to offer and an ignored one has been
    /// answered already. Neither is a question worth asking again.
    #[test]
    fn an_armed_or_ignored_project_is_never_offered() {
        for mode in [ProjectMode::AutoUpload, ProjectMode::Ignore] {
            let mut p = armed_policy();
            p.set_mode("/Users/z/code/api", mode, t("2026-08-31T11:00:00Z"))
                .unwrap();
            assert!(
                p.arming_suggestion(t("2026-08-31T12:00:00Z")).is_none(),
                "{mode:?} must not be offered"
            );
        }
    }

    /// The bucket can never be armed, so counting it could only ever feed an
    /// offer the daemon would refuse.
    #[test]
    fn the_unresolvable_bucket_is_never_counted_or_offered() {
        let mut p = ProjectPolicy::new();
        for _ in 0..(ARMING_SUGGESTION_THRESHOLD * 3) {
            p.record_contribution(UNKNOWN_PROJECT_KEY);
        }
        assert!(!p.contributed.contains_key(UNKNOWN_PROJECT_KEY));
        assert!(p.arming_suggestion(t("2026-08-31T12:00:00Z")).is_none());
    }

    #[test]
    fn declining_silences_the_offer_for_the_cooldown() {
        let mut p = armed_policy();
        p.decline_arming("/Users/z/code/api", t("2026-08-01T12:00:00Z"));
        assert!(p.arming_suggestion(t("2026-08-15T12:00:00Z")).is_none());
    }

    /// "Not now" says not now. A suppression that never lifted would make
    /// those words a lie.
    #[test]
    fn the_offer_returns_after_the_cooldown() {
        let mut p = armed_policy();
        p.decline_arming("/Users/z/code/api", t("2026-08-01T12:00:00Z"));
        assert!(p.arming_suggestion(t("2026-09-01T12:00:00Z")).is_some());
    }

    /// At most one offer, ever. A queue that sprouts one per project is the
    /// ongoing administration this is supposed to remove.
    #[test]
    fn only_the_strongest_candidate_is_offered() {
        let mut p = ProjectPolicy::new();
        for _ in 0..ARMING_SUGGESTION_THRESHOLD {
            p.record_contribution("/Users/z/code/api");
        }
        for _ in 0..(ARMING_SUGGESTION_THRESHOLD + 4) {
            p.record_contribution("/Users/z/code/web");
        }
        let s = p.arming_suggestion(t("2026-08-31T12:00:00Z")).unwrap();
        assert_eq!(s.project_label, "web");
    }

    /// Two projects on the same count must not shuffle between runs.
    #[test]
    fn a_tie_is_broken_stably() {
        let mut p = ProjectPolicy::new();
        for _ in 0..ARMING_SUGGESTION_THRESHOLD {
            p.record_contribution("/Users/z/code/api");
            p.record_contribution("/Users/z/code/web");
        }
        let first = p.arming_suggestion(t("2026-08-31T12:00:00Z")).unwrap();
        for _ in 0..20 {
            assert_eq!(
                p.arming_suggestion(t("2026-08-31T12:00:00Z")).unwrap(),
                first
            );
        }
    }

    /// A policy file written before these fields existed must still parse,
    /// and read as "nothing counted yet" rather than failing the whole file
    /// and losing every mode the contributor had set.
    #[test]
    fn a_policy_file_without_the_new_fields_still_parses() {
        let json = format!(r#"{{"schema_version":"{DAEMON_PROJECTS_SCHEMA}","projects":{{}}}}"#);
        let p: ProjectPolicy = serde_json::from_str(&json).unwrap();
        assert!(p.contributed.is_empty());
        assert!(p.arming_declined_at.is_empty());
        assert!(p.arming_suggestion(t("2026-08-31T12:00:00Z")).is_none());
    }

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
}
