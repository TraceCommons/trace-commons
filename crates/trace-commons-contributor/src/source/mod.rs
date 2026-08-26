//! Source model: the `TraceSource` trait, session/transcript types shared by
//! per-agent adapters (Tasks 7-8), and deterministic hashing/id helpers.

use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};

use crate::daemon::settings::SourceDeclaration;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

pub mod claude_code;
pub mod codex;
pub mod discovery;
pub mod trajectory;

/// A load declined because of what the session *is*, not because of what
/// the machine happened to be doing when it was asked.
///
/// The distinction is the whole point of the type. A source that refuses a
/// session over its own byte budget will refuse the same session on every
/// poll for the rest of its life, so the contributor has to be able to find
/// out; a read that failed because a file was momentarily unreadable will
/// very likely succeed sixty seconds later, and treating the two alike
/// means either flagging a healthy daemon over an IO blip or staying silent
/// about a session that is never going to be offered. The callers that care
/// downcast for this rather than matching on message text -- see
/// `daemon::watcher::visit_session`.
///
/// `label` is the refusal's existing wire name, carried on the type so the
/// message a source already emits does not change: `source::codex` says
/// `rollout-too-large`, and a shell or a log line that recognises that
/// string keeps recognising it.
///
/// Both byte counts describe the contributor's own file against a constant
/// compiled into this binary. Neither is operator-secret and both are safe
/// to state; the path is neither, and is deliberately absent.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("{label}: {declared_bytes} bytes exceeds the {budget_bytes}-byte budget")]
pub struct SessionTooLarge {
    pub label: &'static str,
    pub declared_bytes: u64,
    pub budget_bytes: u64,
}

pub const SOURCE_CLAUDE_CODE: &str = "claude-code";
pub const SOURCE_CODEX: &str = "codex";
pub const SOURCE_TRAJECTORY: &str = "trajectory";

#[derive(Debug, Clone)]
pub struct SessionRef {
    pub source: &'static str,
    pub path: PathBuf,
    pub project: Option<String>, // basename only, never a full path
    pub cwd: Option<String>, // true working dir if cheaply known at discovery; used for --project matching, NEVER serialized
    pub started_at: Option<DateTime<Utc>>,
    /// The total bytes this ref will hash and load: one file for most
    /// sources, a session file plus its subagent transcripts for
    /// claude-code. The daemon's eligibility check keys size stability on
    /// this, so it must describe everything `load` reads -- a ref whose
    /// size covered only its primary file would report a group quiescent
    /// while a sibling transcript was still growing.
    pub size_bytes: u64,
    /// The most recent mtime across every file this ref covers, when the
    /// source knows it cheaply. `None` means "no group; stat `path`", which
    /// is what every single-file source reports.
    ///
    /// Same reason as `size_bytes`: quiescence is judged on the whole group
    /// or it is judged on nothing. `path` stays the primary file so the
    /// queue, the upload state, and `find_session` all keep addressing a
    /// ref by one stable path, which is exactly why the parent's own mtime
    /// cannot be the thing quiescence is measured against.
    pub group_modified_at: Option<DateTime<Utc>>,
    /// How many additional transcripts beyond the primary file this ref
    /// covers. Zero for every single-file source. Surfaced on the queue
    /// entry so a card covering a hundred delegated transcripts can say so
    /// -- that is material to the consent decision, not decoration.
    pub group_member_count: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionEventKind {
    User,
    Assistant,
    Reasoning,
    ToolCall,
    ToolResult,
    Opaque,
}

#[derive(Debug, Clone)]
pub struct SessionEvent {
    pub kind: SessionEventKind,
    pub timestamp: Option<DateTime<Utc>>,
    pub content: Option<String>,
    pub structured: serde_json::Value, // Value::Null when absent
    pub tool_name: Option<String>,
    pub token_counts: Option<(u32, u32)>, // (input, output)
    /// The harness's own id for the call: `tool_use.id` in Claude Code,
    /// `call_id` in Codex, `id`/`tool_call_id` in a trajectory file. Set on
    /// both halves of a call so a result can be paired with the call it
    /// answers -- every adapter read these ids and threw them away, which
    /// left array order as the only pairing signal (issue #298).
    pub tool_call_id: Option<String>,
    /// Whether the step did what it was asked, where the transcript says so.
    /// `None` means the harness did not record an outcome, which is not the
    /// same as failure.
    pub success: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct SessionTranscript {
    /// Provenance: the harness that produced this session. For the native
    /// adapters this equals the adapter name; for trajectory files it is the
    /// file's own `meta.source`, so a session normalized from OpenHands is
    /// attributed to OpenHands rather than to the trajectory reader.
    /// Distinct from `SessionRef.source`, which is the adapter routing key.
    pub source: Cow<'static, str>,
    pub agent_version: Option<String>,
    pub model: Option<String>,
    pub project: Option<String>, // basename
    pub cwd: Option<String>, // full path; used for redactor prefixes + hashing, NEVER serialized
    pub started_at: Option<DateTime<Utc>>,
    pub session_hash: String, // "sha256:<hex>" of raw file bytes
    pub events: Vec<SessionEvent>,
    /// How many delegated transcripts were merged into this one, and how
    /// many were left out because the group exceeded the raw byte budget.
    ///
    /// These are load-time facts, not discovery-time ones: they describe
    /// what `session_hash` actually covers. A dropped member means the
    /// contributor is being shown a deliberately trimmed conversation, so
    /// the count travels with the transcript onto the queue entry rather
    /// than being decided again at send time.
    pub subagent_count: u32,
    pub subagents_dropped: u32,
}

/// `Send + Sync` because the background daemon holds source adapters across
/// await points on a multi-threaded runtime. Every adapter is stateless --
/// each one holds only a root path -- so this costs nothing.
pub trait TraceSource: Send + Sync {
    fn name(&self) -> &'static str;
    fn discover(&self) -> anyhow::Result<Vec<SessionRef>>;
    fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript>;

    /// Which session, if any, a changed filesystem path belongs to.
    ///
    /// Event-driven watching learns that *a path* moved and has to turn
    /// that into *a session* before anything can be scanned. The answer is
    /// source-specific -- a Codex rollout is its own session, a Claude Code
    /// transcript under `<uuid>/subagents/` belongs to the parent -- so it
    /// belongs here, beside `discover`, rather than in the daemon where it
    /// would have to re-derive each adapter's layout.
    ///
    /// Returns the session's stable address: the same `PathBuf` that
    /// `SessionRef::path` carries, so the queue, the upload state and a
    /// scoped scan all keep addressing a session by one path. `None` means
    /// the path is not part of any session this source owns.
    ///
    /// **This is fed paths that came from the operating system**, so it is
    /// an addressing surface, not a convenience. Every implementation must
    /// refuse a path that is not really inside the declared root -- `..`
    /// traversal and symlinks included -- and must be at least as strict as
    /// the adapter's own discovery: a mapping laxer than discovery would be
    /// a way to name a file the contributor never agreed to watch.
    ///
    /// The default answers `None`, which is the correct answer for a source
    /// that cannot map paths at all: the reconciliation sweep still finds
    /// its sessions, just on the slow path.
    fn session_for_path(&self, _path: &Path) -> Option<PathBuf> {
        None
    }

    /// The full `SessionRef` for the session a changed path belongs to.
    ///
    /// `session_for_path` answers *which* session; this answers *what it
    /// looks like right now* -- size, group mtime, cwd -- which is what a
    /// scoped scan needs before it can judge eligibility. Resolving the
    /// address and describing the session are separate steps on purpose:
    /// the address rule is shared with the daemon's bookkeeping, while this
    /// is the part that touches the disk.
    ///
    /// The ref MUST be identical to the one `discover` produces for the
    /// same session. A scoped scan and a full sweep that disagreed about a
    /// session's size or group mtime would reach different eligibility
    /// decisions for the same bytes, which is the drift event-driven
    /// watching exists to avoid rather than introduce. Implementations
    /// therefore share one ref-construction function with `discover` rather
    /// than building a second one.
    ///
    /// `Ok(None)` covers both "not a session" and "was a session, is now
    /// gone": these paths come from filesystem events, so a session
    /// deleted between the event and this lookup is an ordinary race and
    /// not a failure. Errors are reserved for I/O failures that are not
    /// "it is gone".
    ///
    /// The default resolves the address and then finds it in `discover`,
    /// which is correct for any source and costs a full scan -- so a source
    /// that maps paths should override it, and one that does not
    /// (`session_for_path` returning `None`) never reaches the scan at all.
    fn session_at(&self, path: &Path) -> anyhow::Result<Option<SessionRef>> {
        let Some(address) = self.session_for_path(path) else {
            return Ok(None);
        };
        Ok(self
            .discover()?
            .into_iter()
            .find(|candidate| candidate.path == address))
    }
}

/// `path` if it is a real file genuinely inside `root`, otherwise `None`.
///
/// The one containment check every adapter's `session_for_path` runs
/// before it applies its own layout rule. Three refusals, and all three
/// have already happened to this codebase's discovery walks:
///
/// - **Not under the root at all**, including anything reachable only by
///   `..`. Components are inspected rather than the string compared, so
///   `<root>/proj/../../etc/x.jsonl` is refused even though it is spelled
///   with the root as a prefix.
/// - **A symlink anywhere in the chain below the root.** Every intermediate
///   component must be a real directory and the leaf a real file, checked
///   with `symlink_metadata`, which does not follow. This is the same rule
///   `push_group_if_jsonl` and `group_members_for` already enforce with
///   `DirEntry::file_type` and `symlink_metadata`: a symlink planted under
///   a session root by any same-user process must not steer collection at
///   files elsewhere on disk.
/// - **Anything that is not a regular file**, so a directory event never
///   becomes a session address.
///
/// The root itself is deliberately not required to be a real directory: it
/// is what the contributor declared, and a declared root that happens to be
/// a symlink is their choice, made once and explicitly. What must not
/// happen is a path *below* it leaving it.
pub(crate) fn real_file_within_root(root: &Path, path: &Path) -> Option<PathBuf> {
    let relative = path.strip_prefix(root).ok()?;
    let mut walked = root.to_path_buf();
    let mut components = relative.components().peekable();
    let mut any = false;
    while let Some(component) = components.next() {
        let name = match component {
            Component::Normal(name) => name,
            // `.` is inert; everything else -- `..`, a root, a Windows
            // prefix -- means this path does not describe a location under
            // `root` even though it was spelled with it as a prefix.
            Component::CurDir => continue,
            _ => return None,
        };
        walked.push(name);
        let metadata = std::fs::symlink_metadata(&walked).ok()?;
        let last = components.peek().is_none();
        if last {
            if !metadata.is_file() {
                return None;
            }
        } else if !metadata.is_dir() {
            return None;
        }
        any = true;
    }
    // An empty relative path means `path` IS the root; a root is not a
    // session file.
    any.then_some(walked)
}

/// Hash raw session bytes as "sha256:<hex>".
pub fn session_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex::encode(digest))
}

/// The same hash, accumulated a chunk at a time.
///
/// `session_hash` needs the whole file in memory, which is exactly what the
/// adapters stopped doing: a rollout can be hundreds of megabytes, and
/// holding one whole to hash it -- then again as a lossy `String` -- is what
/// made a first scan cost gigabytes of resident memory. Feeding the same
/// bytes in file order produces the identical digest, so a streaming loader
/// and a whole-file one agree on the session id.
#[derive(Default)]
pub struct SessionHasher(Sha256);

impl SessionHasher {
    pub fn new() -> Self {
        Self(Sha256::new())
    }

    /// Feed the next chunk. Callers must pass the file's bytes in order and
    /// unmodified, terminators included, or the digest will not match what
    /// `session_hash` would have produced for the same file.
    pub fn update(&mut self, chunk: &[u8]) {
        self.0.update(chunk);
    }

    pub fn finish(self) -> String {
        format!("sha256:{}", hex::encode(self.0.finalize()))
    }
}

/// Deterministic submission id derived from the session hash string.
pub fn submission_id_for(session_hash: &str) -> uuid::Uuid {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, session_hash.as_bytes())
}

/// Deterministic pre-enrollment preview id derived from the session hash.
///
/// Real submission ids are UUIDv5. Preview ids use UUIDv8 with an explicit
/// domain separator, so the UUID version bits make the two namespaces
/// structurally disjoint even for the same session hash.
pub fn preview_submission_id_for(session_hash: &str) -> uuid::Uuid {
    let mut hasher = Sha256::new();
    hasher.update(b"trace-commons:unenrolled-preview:v1\0");
    hasher.update(session_hash.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}

/// Construct the set of available `TraceSource` adapters from what the
/// contributor declared.
///
/// Three states per source, and the difference between two of them is the
/// whole point:
///
/// - `Some(Watch { path })` -- watch that directory.
/// - `Some(Off)` -- the contributor said they do not use this agent. **No
///   source is constructed and there is no fallback.** This is the state
///   that previously did not exist, and its absence is what made "I don't
///   use Codex" indistinguishable from "nobody has asked yet" and therefore
///   equal to watching the real `~/.codex`.
/// - `None` -- never asked. Only here does the conventional per-user
///   location still apply, and only the CLI can reach it: every application
///   shell refuses to start until both sources are declared
///   (`daemon::settings::roots_declared`). `trace-commons-contributor daemon`
///   is somebody typing a command on purpose and keeps its defaults.
///
/// The trajectory source is included only when an explicit path is supplied,
/// because trajectory files have no conventional local store.
pub fn all_sources(
    claude: Option<SourceDeclaration>,
    codex: Option<SourceDeclaration>,
    trajectory_path: Option<PathBuf>,
) -> Vec<Box<dyn TraceSource>> {
    let mut sources: Vec<Box<dyn TraceSource>> = Vec::new();

    match claude {
        Some(SourceDeclaration::Off) => {}
        Some(SourceDeclaration::Watch { path }) => {
            sources.push(Box::new(claude_code::ClaudeCodeSource::new(path)))
        }
        None => sources.push(Box::new(claude_code::ClaudeCodeSource::new(
            dirs::home_dir()
                .unwrap_or_default()
                .join(".claude/projects"),
        ))),
    }

    match codex {
        Some(SourceDeclaration::Off) => {}
        Some(SourceDeclaration::Watch { path }) => {
            sources.push(Box::new(codex::CodexSource::new(path)))
        }
        None => sources.push(Box::new(codex::CodexSource::new(
            dirs::home_dir().unwrap_or_default().join(".codex/sessions"),
        ))),
    }

    if let Some(path) = trajectory_path {
        sources.push(Box::new(trajectory::TrajectorySource::new(path)));
    }
    sources
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The streaming hash and the whole-file hash must agree.
    ///
    /// They name the same thing -- the session id every receipt, dedup check
    /// and prior-upload record is keyed on. If chunking changed the digest,
    /// every already-uploaded session would look new the day a loader
    /// started streaming, and the queue would re-offer the entire corpus.
    #[test]
    fn the_streaming_hash_matches_the_whole_file_hash() {
        let body: Vec<u8> = (0..40_000u32)
            .flat_map(|i| format!("line {i}\n").into_bytes())
            .collect();

        for chunk in [1usize, 7, 512, 8192] {
            let mut hasher = SessionHasher::new();
            for part in body.chunks(chunk) {
                hasher.update(part);
            }
            assert_eq!(
                hasher.finish(),
                session_hash(&body),
                "chunking at {chunk} bytes changed the digest"
            );
        }
    }

    /// The fail-open this slice closes, stated as a test.
    ///
    /// "I don't use Codex" used to be spelled `codex_root: None`, which
    /// `all_sources` turned into `~/.codex/sessions`. On a real machine that
    /// is thousands of session files the contributor never agreed to.
    #[test]
    fn a_source_declared_off_is_not_constructed_at_all() {
        let sources = all_sources(
            Some(SourceDeclaration::Watch {
                path: PathBuf::from("/declared/claude"),
            }),
            Some(SourceDeclaration::Off),
            None,
        );
        let names: Vec<&str> = sources.iter().map(|s| s.name()).collect();
        assert_eq!(
            names,
            vec![SOURCE_CLAUDE_CODE],
            "a source declared off must produce no adapter, and therefore \
             nothing that can discover or read a file"
        );
    }

    #[test]
    fn both_declared_off_watches_nothing() {
        let sources = all_sources(
            Some(SourceDeclaration::Off),
            Some(SourceDeclaration::Off),
            None,
        );
        assert!(
            sources.is_empty(),
            "declaring every source off is a legitimate answer and must \
             watch nothing, not fall back to everything"
        );
    }

    #[test]
    fn off_never_reaches_the_conventional_location() {
        // Pinned separately from the count above: the failure mode that
        // matters is not "an extra adapter appeared", it is "an adapter
        // appeared pointing at the contributor's real home directory".
        let home = dirs::home_dir().unwrap_or_default();
        for sources in [
            all_sources(
                Some(SourceDeclaration::Off),
                Some(SourceDeclaration::Off),
                None,
            ),
            all_sources(
                Some(SourceDeclaration::Off),
                Some(SourceDeclaration::Watch {
                    path: PathBuf::from("/declared/codex"),
                }),
                None,
            ),
        ] {
            for source in &sources {
                assert_ne!(
                    source.name(),
                    SOURCE_CLAUDE_CODE,
                    "claude was declared off; no claude adapter may exist, \
                     least of all one rooted at {}",
                    home.join(".claude/projects").display()
                );
            }
        }
    }

    #[test]
    fn never_asked_still_defaults_so_the_cli_is_unaffected() {
        // The application shells cannot reach this: roots_declared() gates
        // them. The CLI can, and deliberately keeps its defaults.
        let sources = all_sources(None, None, None);
        let names: Vec<&str> = sources.iter().map(|s| s.name()).collect();
        assert_eq!(names, vec![SOURCE_CLAUDE_CODE, SOURCE_CODEX]);
    }

    #[test]
    fn session_hash_is_prefixed_and_deterministic() {
        let h = session_hash(b"abc");
        assert!(h.starts_with("sha256:"));
        assert_eq!(h, session_hash(b"abc"));
        assert_ne!(h, session_hash(b"abd"));
    }

    #[test]
    fn submission_id_is_deterministic_per_session() {
        let a = submission_id_for("sha256:aa");
        assert_eq!(a, submission_id_for("sha256:aa"));
        assert_ne!(a, submission_id_for("sha256:bb"));
    }

    #[test]
    fn preview_ids_are_deterministic_and_disjoint_from_submission_ids() {
        let preview = preview_submission_id_for("sha256:aa");
        assert_eq!(preview, preview_submission_id_for("sha256:aa"));
        assert_ne!(preview, preview_submission_id_for("sha256:bb"));
        assert_ne!(preview, submission_id_for("sha256:aa"));
        assert_eq!(preview.get_version_num(), 8);
        assert_eq!(submission_id_for("sha256:aa").get_version_num(), 5);
    }
}
