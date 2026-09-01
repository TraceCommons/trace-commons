//! Source model: the `TraceSource` trait, session/transcript types shared by
//! per-agent adapters (Tasks 7-8), and deterministic hashing/id helpers.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::daemon::settings::SourceDeclaration;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

pub mod claude_code;
pub mod codex;
pub mod discovery;
pub mod gemini_cli;
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
pub const SOURCE_GEMINI_CLI: &str = "gemini-cli";

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

/// `Default` exists for tests that only care about one or two fields and
/// want to fill the rest with something rather than hand-write every field
/// every time -- see the `..Default::default()` construction pattern used
/// throughout this crate's test modules. A defaulted transcript (empty
/// events, no source, no session hash) is not a real session and must never
/// be treated as one in production code.
#[derive(Debug, Clone, Default)]
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
    /// The source's own identifier for this session -- the on-disk stem
    /// each adapter already resolves to address the session (the Claude
    /// Code session uuid, the Codex rollout filename, the trajectory file
    /// name). Flows unchanged onto the envelope's `conversation_id`:
    /// attribution only, never a gate or scoring input (issue #298 S4a).
    /// `None` when a source cannot resolve one.
    pub conversation_id: Option<String>,
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
    /// Routing and cost data for the inference hops behind this session, when
    /// a local proxy recorded them and the session could be joined to them.
    ///
    /// Empty is the normal state, not a failure: most contributors run no
    /// proxy, and a session that predates one is only partly covered even
    /// where one exists. The transcript is the single carrier of everything
    /// the envelope builder needs, which is why this lives here rather than
    /// being threaded through the four builders separately.
    pub routing: Vec<crate::routing::RoutedExchange>,
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

/// What is constructed for a source the contributor has never been asked
/// about.
///
/// The two answers are not a style choice. `Conventional` is what the
/// claude-code and codex adapters have always done, and the application
/// shells are stopped from reaching it by
/// [`crate::daemon::settings::roots_declared`]; only
/// `trace-commons-contributor daemon`, which is somebody typing a command
/// on purpose, still gets those defaults. A source added AFTER those shells
/// shipped cannot rely on that gate -- every installed client declares
/// claude and codex and has no field for anything newer -- so it takes
/// `Nothing`, and an absent declaration constructs no adapter and scans no
/// directory. A contributor who wants it says so, either by declaring it or
/// by using the CLI, which asks for [`SourceRoots::conventional`]
/// explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Undeclared {
    Conventional,
    Nothing,
}

/// One native adapter's row in the registration table.
///
/// Adding an adapter is a new module, a name constant and a row here. It is
/// deliberately not a parameter on [`all_sources`]: a positional parameter
/// per source made adapter N+1 cost a signature change plus every call site
/// in the daemon, the watcher, the preview scheduler and the CLI, which is
/// the reason a harness nobody had written an adapter for stayed
/// unsupported.
struct SourceSpec {
    name: &'static str,
    /// The per-user store to use when the contributor has never been asked
    /// and the caller accepts conventional locations.
    conventional_root: fn() -> PathBuf,
    build: fn(PathBuf) -> Box<dyn TraceSource>,
    undeclared: Undeclared,
}

/// Every native adapter, in the order sources are offered.
static NATIVE_SOURCES: &[SourceSpec] = &[
    SourceSpec {
        name: SOURCE_CLAUDE_CODE,
        conventional_root: || {
            dirs::home_dir()
                .unwrap_or_default()
                .join(".claude/projects")
        },
        build: |path| Box::new(claude_code::ClaudeCodeSource::new(path)),
        undeclared: Undeclared::Conventional,
    },
    SourceSpec {
        name: SOURCE_CODEX,
        conventional_root: || dirs::home_dir().unwrap_or_default().join(".codex/sessions"),
        build: |path| Box::new(codex::CodexSource::new(path)),
        undeclared: Undeclared::Conventional,
    },
    SourceSpec {
        name: SOURCE_GEMINI_CLI,
        conventional_root: gemini_cli::conventional_root_this_machine,
        build: |path| Box::new(gemini_cli::GeminiCliSource::new(path)),
        // See `Undeclared`: every desktop client that has already shipped
        // declares claude and codex and carries no gemini field, so an
        // absent declaration must construct nothing rather than fall back
        // to the contributor's real `~/.gemini`.
        undeclared: Undeclared::Nothing,
    },
];

/// The subdirectory of the contributor state directory that trajectory
/// files may be staged in. Placing a file there IS the opt-in, which is why
/// nothing in it needs a name suffix.
pub const TRAJECTORY_STAGING_SUBDIR: &str = "trajectories";

/// Which trajectory files, if any, this caller wants read.
///
/// Trajectory has no conventional per-user store, so unlike the native
/// adapters it is not a row in `NATIVE_SOURCES`; it is asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TrajectorySelection {
    /// No trajectory source at all. What the daemon asks for: its working
    /// directory is whatever a service manager gave it and means nothing.
    #[default]
    None,
    /// Exactly this file or directory, named by the contributor with
    /// `--trajectory`. A file here that fails to parse is a hard error,
    /// because the contributor said it was a trajectory.
    Declared(PathBuf),
    /// Bounded auto-discovery: the working directory, suffix-gated, plus
    /// the state directory's staging folder. Never `$HOME` at large and
    /// never a recursive walk. A file that fails to parse is skipped here,
    /// because it never claimed to be a trajectory.
    Auto {
        working_dir: Option<PathBuf>,
        staging_dir: Option<PathBuf>,
    },
}

/// What the contributor declared, keyed by adapter name.
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
/// - absent -- never asked. What happens then is the adapter's own
///   [`Undeclared`] policy, not one rule for everybody.
#[derive(Clone, Default)]
pub struct SourceRoots {
    declared: BTreeMap<&'static str, SourceDeclaration>,
    trajectory: TrajectorySelection,
    /// A routing overlay to attach to every adapter these roots build.
    /// `None` -- the majority case -- leaves adapters bare. Not `Debug`: a
    /// trait object can hold anything a proprietary ledger implementation
    /// wants, so `SourceRoots` implements `Debug` by hand below rather than
    /// deriving it and requiring `dyn RoutingLedger: Debug`.
    routing: Option<Arc<dyn crate::routing::RoutingLedger>>,
}

impl std::fmt::Debug for SourceRoots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceRoots")
            .field("declared", &self.declared)
            .field("trajectory", &self.trajectory)
            .field("routing", &self.routing.is_some())
            .finish()
    }
}

impl SourceRoots {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a routing ledger, so every source built from these roots
    /// carries the routing overlay. `None` leaves sources bare -- the
    /// majority case.
    pub fn with_routing(mut self, ledger: Option<Arc<dyn crate::routing::RoutingLedger>>) -> Self {
        self.routing = ledger;
        self
    }

    /// Whether a routing ledger is attached. Test-only: production code has
    /// no reason to branch on this, only to build sources from it, but a
    /// test pinning "nothing attaches a ledger yet" needs to see the field
    /// without constructing a session to prove it indirectly.
    #[cfg(test)]
    pub(crate) fn is_routed(&self) -> bool {
        self.routing.is_some()
    }

    /// Record what the contributor said about one source. `None` leaves it
    /// undeclared rather than declaring it off -- they are different
    /// answers.
    pub fn declare(mut self, name: &'static str, declaration: Option<SourceDeclaration>) -> Self {
        match declaration {
            Some(d) => {
                self.declared.insert(name, d);
            }
            None => {
                self.declared.remove(name);
            }
        }
        self
    }

    pub fn with_trajectory(mut self, trajectory: TrajectorySelection) -> Self {
        self.trajectory = trajectory;
        self
    }

    /// Whether this source has been asked about at all.
    pub fn is_declared(&self, name: &str) -> bool {
        self.declared.contains_key(name)
    }

    /// Every native store at its conventional per-user location, declared
    /// explicitly.
    ///
    /// This is the CLI's answer, and it is spelled as a declaration rather
    /// than left to the undeclared fallback so that "the CLI keeps its
    /// defaults" is a decision one caller makes out loud, not a property a
    /// new adapter silently inherits.
    pub fn conventional() -> Self {
        let mut roots = Self::new();
        for spec in NATIVE_SOURCES {
            roots = roots.declare(
                spec.name,
                Some(SourceDeclaration::Watch {
                    path: (spec.conventional_root)(),
                }),
            );
        }
        roots
    }
}

/// The CLI's sources: every native store at its conventional location, plus
/// trajectory files -- the path `--trajectory` named, or bounded
/// auto-discovery when it did not.
///
/// The staging directory is resolved through `ConfigStore`, so it honours
/// `TRACE_COMMONS_CONTRIBUTOR_DIR`, inherits the state directory's `0700`
/// mode, and is cleared by `logout` along with everything else there.
pub fn cli_source_roots(trajectory: Option<&Path>) -> SourceRoots {
    let selection = match trajectory {
        Some(path) => TrajectorySelection::Declared(path.to_path_buf()),
        None => TrajectorySelection::Auto {
            working_dir: std::env::current_dir().ok(),
            staging_dir: crate::config::ConfigStore::resolve(None)
                .ok()
                .map(|store| store.dir().join(TRAJECTORY_STAGING_SUBDIR)),
        },
    };
    SourceRoots::conventional().with_trajectory(selection)
}

/// Construct the set of available `TraceSource` adapters from what the
/// contributor declared.
///
/// See [`SourceRoots`] for the three declaration states and [`Undeclared`]
/// for what an absent one means, which is per-adapter.
pub fn all_sources(roots: &SourceRoots) -> Vec<Box<dyn TraceSource>> {
    let mut sources: Vec<Box<dyn TraceSource>> = Vec::new();

    for spec in NATIVE_SOURCES {
        match roots.declared.get(spec.name) {
            Some(SourceDeclaration::Off) => {}
            Some(SourceDeclaration::Watch { path }) => sources.push((spec.build)(path.clone())),
            None => match spec.undeclared {
                Undeclared::Conventional => sources.push((spec.build)((spec.conventional_root)())),
                Undeclared::Nothing => {}
            },
        }
    }

    match &roots.trajectory {
        TrajectorySelection::None => {}
        TrajectorySelection::Declared(path) => {
            sources.push(Box::new(trajectory::TrajectorySource::new(path.clone())))
        }
        TrajectorySelection::Auto {
            working_dir,
            staging_dir,
        } => sources.push(Box::new(trajectory::TrajectorySource::auto(
            working_dir.clone(),
            staging_dir.clone(),
        ))),
    }

    // One insertion point for the whole overlay. Without a declared proxy the
    // adapters are returned bare, which is the majority case and costs one
    // branch.
    let Some(routing) = roots.routing.clone() else {
        return sources;
    };
    sources
        .into_iter()
        .map(|source| {
            Box::new(crate::routing::enriched::RoutingEnrichedSource::new(
                source,
                Arc::clone(&routing),
            )) as Box<dyn TraceSource>
        })
        .collect()
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

    fn watch(path: &str) -> Option<SourceDeclaration> {
        Some(SourceDeclaration::Watch {
            path: PathBuf::from(path),
        })
    }

    /// The fail-open this slice closes, stated as a test.
    ///
    /// "I don't use Codex" used to be spelled `codex_root: None`, which
    /// `all_sources` turned into `~/.codex/sessions`. On a real machine that
    /// is thousands of session files the contributor never agreed to.
    #[test]
    fn a_source_declared_off_is_not_constructed_at_all() {
        let sources = all_sources(
            &SourceRoots::new()
                .declare(SOURCE_CLAUDE_CODE, watch("/declared/claude"))
                .declare(SOURCE_CODEX, Some(SourceDeclaration::Off)),
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
            &SourceRoots::new()
                .declare(SOURCE_CLAUDE_CODE, Some(SourceDeclaration::Off))
                .declare(SOURCE_CODEX, Some(SourceDeclaration::Off)),
        );
        assert!(
            sources.is_empty(),
            "declaring every source off is a legitimate answer and must \
             watch nothing, not fall back to everything"
        );
    }

    /// A minimal, real Claude Code session file: one line, one project dir.
    /// Mirrors the record shape `claude_code.rs`'s own fixtures use, so this
    /// is a real adapter round trip (discover + load), not a stub.
    fn claude_code_fixture(session: &str) -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("-Users-testuser-code-myproj");
        std::fs::create_dir_all(&project_dir).unwrap();
        let file = project_dir.join(format!("{session}.jsonl"));
        std::fs::write(
            &file,
            format!(
                "{{\"type\":\"user\",\"cwd\":\"/Users/testuser/code/myproj\",\
                 \"sessionId\":\"{session}\",\
                 \"message\":{{\"role\":\"user\",\"content\":\"hi\"}}}}\n"
            ),
        )
        .unwrap();
        (root, file)
    }

    #[test]
    fn without_a_declared_proxy_the_adapters_are_returned_bare() {
        // "Bare" has to be observed, not assumed from an empty source list --
        // the old version of this test declared both sources Off and checked
        // `is_empty()`, which passes whether or not the decoration branch
        // exists and never actually looks at a returned source.
        //
        // Declare one real source and load a real session through it twice:
        // once with no proxy declared, once with a proxy declared whose
        // ledger holds an exchange for exactly that session. `load` is the
        // only place a `RoutingEnrichedSource` differs observably from its
        // inner adapter -- it overwrites `transcript.routing` from the
        // ledger. So the routing-declared case must come back non-empty
        // (proving the wrapper ran and can see this session) while the
        // no-proxy case must come back empty -- proving only that no overlay
        // is attached in the bare case, not that the returned source is some
        // different concrete type: a wrapper over an empty ledger would pass
        // this half identically. The contrast with the wrapped arm below is
        // what actually carries the test.
        let session = "33333333-3333-3333-3333-333333333333";
        let (root, _file) = claude_code_fixture(session);
        let root_path = root.path().to_str().unwrap();

        let bare_sources = all_sources(
            &SourceRoots::new()
                .declare(SOURCE_CLAUDE_CODE, watch(root_path))
                .declare(SOURCE_CODEX, Some(SourceDeclaration::Off)),
        );
        assert_eq!(bare_sources.len(), 1);
        let bare_refs = bare_sources[0].discover().expect("discover succeeds");
        assert_eq!(
            bare_refs.len(),
            1,
            "fixture must produce exactly one session"
        );
        let bare_transcript = bare_sources[0].load(&bare_refs[0]).expect("load succeeds");
        assert!(
            bare_transcript.routing.is_empty(),
            "without a declared proxy the returned source must not attach \
             any routing overlay"
        );

        let ledger: Arc<dyn crate::routing::RoutingLedger> =
            Arc::new(crate::routing::FixedLedger::new(vec![
                crate::routing::RoutedExchange {
                    started_at: chrono::Utc::now(),
                    client_session_id: Some(session.to_string()),
                    total_ms: Some(1200),
                    facade: "anthropic".to_string(),
                    backend: "claude-sub".to_string(),
                    requested_model: None,
                    served_model: None,
                    rung: "same_model".to_string(),
                    attempts: 1,
                    input_tokens: Some(1000),
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                    output_tokens: Some(200),
                    cost_usd: Some(0.02),
                    status: 200,
                },
            ]));
        let wrapped_sources = all_sources(
            &SourceRoots::new()
                .declare(SOURCE_CLAUDE_CODE, watch(root_path))
                .declare(SOURCE_CODEX, Some(SourceDeclaration::Off))
                .with_routing(Some(ledger)),
        );
        assert_eq!(wrapped_sources.len(), 1);
        let wrapped_refs = wrapped_sources[0].discover().expect("discover succeeds");
        let wrapped_transcript = wrapped_sources[0]
            .load(&wrapped_refs[0])
            .expect("load succeeds");
        assert_eq!(
            wrapped_transcript.routing.len(),
            1,
            "with a declared proxy the same session must come back enriched"
        );
    }

    #[test]
    fn a_declared_proxy_decorates_every_adapter_without_adding_one() {
        let ledger: Arc<dyn crate::routing::RoutingLedger> =
            Arc::new(crate::routing::FixedLedger::new(Vec::new()));
        let bare =
            all_sources(&SourceRoots::new().declare(SOURCE_CLAUDE_CODE, watch("/declared/claude")))
                .len();
        let wrapped = all_sources(
            &SourceRoots::new()
                .declare(SOURCE_CLAUDE_CODE, watch("/declared/claude"))
                .with_routing(Some(ledger)),
        )
        .len();
        assert_eq!(bare, wrapped, "decorating must not add or drop a source");
    }

    #[test]
    fn off_never_reaches_the_conventional_location() {
        // Pinned separately from the count above: the failure mode that
        // matters is not "an extra adapter appeared", it is "an adapter
        // appeared pointing at the contributor's real home directory".
        let home = dirs::home_dir().unwrap_or_default();
        for sources in [
            all_sources(
                &SourceRoots::new()
                    .declare(SOURCE_CLAUDE_CODE, Some(SourceDeclaration::Off))
                    .declare(SOURCE_CODEX, Some(SourceDeclaration::Off)),
            ),
            all_sources(
                &SourceRoots::new()
                    .declare(SOURCE_CLAUDE_CODE, Some(SourceDeclaration::Off))
                    .declare(SOURCE_CODEX, watch("/declared/codex")),
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
        let sources = all_sources(&SourceRoots::new());
        let names: Vec<&str> = sources.iter().map(|s| s.name()).collect();
        assert_eq!(names, vec![SOURCE_CLAUDE_CODE, SOURCE_CODEX]);
    }

    /// The A2 rule, stated as a test.
    ///
    /// Every desktop client that has already shipped declares claude and
    /// codex and has no gemini field at all. If an absent gemini
    /// declaration fell back to the conventional store the way an absent
    /// claude one does, the next release of those clients would start
    /// reading `~/.gemini` on machines whose owner was never asked.
    #[test]
    fn an_absent_gemini_declaration_constructs_no_adapter() {
        for roots in [
            SourceRoots::new(),
            SourceRoots::new()
                .declare(SOURCE_CLAUDE_CODE, watch("/declared/claude"))
                .declare(SOURCE_CODEX, watch("/declared/codex")),
        ] {
            assert!(
                !all_sources(&roots)
                    .iter()
                    .any(|s| s.name() == SOURCE_GEMINI_CLI),
                "an undeclared gemini source must construct nothing, not \
                 fall back to {}",
                gemini_cli::conventional_root_this_machine().display()
            );
        }

        assert!(
            !all_sources(
                &SourceRoots::new().declare(SOURCE_GEMINI_CLI, Some(SourceDeclaration::Off))
            )
            .iter()
            .any(|s| s.name() == SOURCE_GEMINI_CLI),
            "and declaring it off must not construct one either"
        );
    }

    /// A declared gemini root IS watched, and the CLI -- which declares
    /// every conventional store explicitly rather than relying on the
    /// undeclared fallback -- gets one too.
    #[test]
    fn a_declared_gemini_root_is_watched_and_the_cli_declares_one() {
        let sources = all_sources(
            &SourceRoots::new()
                .declare(SOURCE_CLAUDE_CODE, Some(SourceDeclaration::Off))
                .declare(SOURCE_CODEX, Some(SourceDeclaration::Off))
                .declare(SOURCE_GEMINI_CLI, watch("/declared/gemini")),
        );
        let names: Vec<&str> = sources.iter().map(|s| s.name()).collect();
        assert_eq!(names, vec![SOURCE_GEMINI_CLI]);

        let names: Vec<&str> = all_sources(&SourceRoots::conventional())
            .iter()
            .map(|s| s.name())
            .collect();
        assert_eq!(
            names,
            vec![SOURCE_CLAUDE_CODE, SOURCE_CODEX, SOURCE_GEMINI_CLI],
            "adding an adapter to the table must reach the CLI without a \
             call-site change"
        );
    }

    /// Trajectory is asked for, never inferred: the daemon's working
    /// directory is whatever a service manager handed it.
    #[test]
    fn the_trajectory_source_is_only_constructed_when_it_is_asked_for() {
        let has_trajectory = |roots: &SourceRoots| {
            all_sources(roots)
                .iter()
                .any(|s| s.name() == SOURCE_TRAJECTORY)
        };
        assert!(!has_trajectory(&SourceRoots::new()));
        assert!(has_trajectory(&SourceRoots::new().with_trajectory(
            TrajectorySelection::Declared(PathBuf::from("/some/run.jsonl"))
        )));
        assert!(has_trajectory(&SourceRoots::new().with_trajectory(
            TrajectorySelection::Auto {
                working_dir: Some(PathBuf::from("/some/project")),
                staging_dir: None,
            }
        )));
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
