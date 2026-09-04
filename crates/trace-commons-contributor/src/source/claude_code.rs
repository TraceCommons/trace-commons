//! Claude Code transcript adapter.
//!
//! Reads `~/.claude/projects/<encoded-cwd>/<uuid>.jsonl` session files and
//! maps them into the shared `SessionTranscript` model. See
//! `docs/superpowers/plans/` (Task 7) for the format facts and mapping
//! rules; `Opaque` events carry only a record-type marker and never a
//! payload. Reasoning (`thinking`) blocks are captured as `Reasoning`
//! events and redacted like any other content.
//!
//! # Subagent grouping
//!
//! A conversation is not one file. Claude Code writes each delegated
//! subagent's turns to `<project>/<session-uuid>/subagents/<agent>.jsonl`,
//! beside the top-level `<project>/<session-uuid>.jsonl`. On one probed
//! machine that was 842 subagent transcripts against 69 actual
//! conversations -- 911 files, one group of which had 114 members.
//!
//! Those 842 were previously discovered as sessions in their own right,
//! which was wrong twice over. It put 911 cards in a queue that describes
//! 69 conversations, and a subagent transcript misrepresents itself: its
//! opening prompt was written by the *parent agent*, and the queue card
//! renders the first user message as `opening_prompt`, so a contributor
//! read a machine-written instruction as if it were their own.
//!
//! So `discover` emits one `SessionRef` per top-level session and `load`
//! merges the parent with every member of its `subagents/` directory into a
//! single `SessionTranscript`. The rules that keep that honest:
//!
//! - **Membership is the directory**, verified by `sessionId`. All 842
//!   members agreed with their grandparent directory name; a member that
//!   disagrees is a format change or a planted file, and everything
//!   discovered here is a candidate for upload, so it is excluded and
//!   counted rather than trusted. A member with no `sessionId` at all is
//!   included -- absence is not disagreement.
//! - **An orphan member is skipped entirely**, never offered standalone. A
//!   fragment whose opening prompt misrepresents itself is worse than no
//!   trace.
//! - **The hash covers the whole group** (`group_session_hash`), so the
//!   uploader's re-hash guard refuses an approval that no longer describes
//!   the bytes -- membership changes included. Members are sorted by raw
//!   file-name bytes because `read_dir` order is unspecified and both
//!   `entry_id_for` and `submission_id_for` derive from this hash: a
//!   non-deterministic hash would re-offer every group forever.
//! - **A group with no members hashes exactly as the bare file does**, so
//!   adopting grouping does not churn the queue for conversations that
//!   never delegated anything.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    SOURCE_CLAUDE_CODE, SessionEvent, SessionEventKind, SessionRef, SessionTranscript, TraceSource,
    real_file_within_root, session_hash,
};

/// The most raw bytes one merged group may hash and load.
///
/// `MAX_TRACE_ENVELOPE_BYTES` is 16 MB *after* redaction and mapping, and
/// the one measurement on record is 42 MB of raw session producing a 2.8 MB
/// envelope -- roughly 15:1. Trusting that ratio would put the ceiling near
/// 240 MB; it is a single observation, so this stays well under a quarter of
/// it. Being conservative here is cheap and the failure it prevents is not:
/// a group that overruns the envelope cap is refused as a whole, losing the
/// entire conversation rather than a tail of it.
///
/// When a group exceeds this, `load` drops its largest members until it
/// fits. That decision is made in `load` -- never at send time -- so the
/// preview a contributor reads and the bytes an upload sends are the same
/// bytes by construction, and the drop is counted into
/// `SessionTranscript::subagents_dropped` rather than silently trimming a
/// conversation.
///
/// The budget bounds what is *read*, not merely what is kept. It is decided
/// from the sizes `group_members_for` has already stat'd, before any member
/// file is opened, so a `subagents/` directory holding a gigabyte never
/// becomes resident in the daemon on its way to being discarded. A member
/// that grows between that stat and its read can overshoot by the amount it
/// grew; the hash covers exactly the bytes that were read either way, so the
/// consent invariant is untouched.
pub const GROUP_RAW_BYTE_BUDGET: u64 = 64_000_000;

/// How far into a member file `peek_session_id` reads looking for a
/// `sessionId`. Claude Code writes it on the first record; a bounded head
/// read makes the per-member verification strictly cheaper than the
/// whole-file `peek_cwd` that discovery already pays on the parent.
const SESSION_ID_PEEK_BYTES: u64 = 64 * 1024;

/// How many memoized `sessionId` peeks to hold before starting over.
///
/// The probed machine had 842 members, so this is roughly an order of
/// magnitude of headroom over the largest tree on record. When it is
/// reached the whole memo is dropped rather than evicted one entry at a
/// time: an LRU would need either a dependency or a second index, and the
/// cost of being wrong here is one discovery pass paying what every pass
/// used to pay.
const SESSION_ID_MEMO_CAP: usize = 8192;

/// One memoized answer from `peek_session_id`, valid while the file it
/// describes still reports the same size and mtime.
#[derive(Debug, Clone)]
struct SessionIdMemo {
    size_bytes: u64,
    modified_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The peeked id, or `None` for a file that carries none in its head --
    /// memoized too, because absence is the answer for any member written
    /// without the field, and re-deriving it would cost the same read every
    /// pass.
    session_id: Option<String>,
}

/// Process-wide memo for `peek_session_id`, keyed on the member's path.
///
/// `discover` runs on every watcher tick and again inside every
/// `find_session`, and it verifies every member's `sessionId`. Unmemoized
/// that is one open and up to `SESSION_ID_PEEK_BYTES` per member per pass --
/// on the 842-member tree this adapter was written for, up to ~54 MB of
/// reads a minute to re-derive an answer that only changes when a file
/// changes. This mirrors what `watcher::resolve_cwd` already does for the
/// far more expensive whole-file cwd peek.
///
/// It lives at module scope rather than on `ClaudeCodeSource` because
/// `watcher::tick_blocking` builds its sources fresh on every tick: a memo
/// owned by the adapter would be discarded before the next pass could use
/// it, and that next pass is precisely the one that needs it.
///
/// Keying on (size, mtime) makes a rewritten member re-peek, and assumes
/// only what the cwd cache already assumes: that a file whose size and mtime
/// are unchanged has unchanged contents. That is not a trust boundary. The
/// `sessionId` check catches format drift and stray files, not an adversary
/// -- anyone able to backdate a member's mtime can equally well write the
/// matching `sessionId` into it, so the memo grants no capability the
/// unmemoized path withheld.
static SESSION_ID_MEMO: LazyLock<Mutex<HashMap<PathBuf, SessionIdMemo>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// One validated member of a session group.
#[derive(Debug, Clone)]
struct GroupMember {
    path: PathBuf,
    size_bytes: u64,
    modified_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct ClaudeCodeSource {
    root: PathBuf,
    /// The raw byte budget applied when merging a group, so a test can
    /// exercise the drop path without writing 64 MB to disk. Production
    /// always uses `GROUP_RAW_BYTE_BUDGET`.
    group_budget: u64,
}

impl ClaudeCodeSource {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            group_budget: GROUP_RAW_BYTE_BUDGET,
        }
    }

    #[cfg(test)]
    fn with_budget(root: PathBuf, group_budget: u64) -> Self {
        Self { root, group_budget }
    }
}

impl TraceSource for ClaudeCodeSource {
    fn name(&self) -> &'static str {
        SOURCE_CLAUDE_CODE
    }

    fn discover(&self) -> anyhow::Result<Vec<SessionRef>> {
        let mut sessions = Vec::new();
        let mut skipped = 0usize;
        let Ok(project_dirs) = std::fs::read_dir(&self.root) else {
            return Ok(sessions);
        };
        for project_dir in project_dirs {
            let project_dir = match project_dir {
                Ok(d) => d,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            let is_dir = match project_dir.file_type() {
                Ok(ft) => ft.is_dir(),
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            if !is_dir {
                continue;
            }
            let discovery_project = discovery_project_label(&project_dir.file_name());
            let Ok(entries) = std::fs::read_dir(project_dir.path()) else {
                continue;
            };
            // Two passes, because `read_dir` order is unspecified and the
            // orphan rule needs the full set of top-level sessions before it
            // can tell a delegated transcript from a fragment whose parent
            // is gone.
            let mut file_entries = Vec::new();
            let mut session_dirs = Vec::new();
            for entry in entries {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                match entry.file_type() {
                    Ok(ft) if ft.is_dir() => session_dirs.push(entry.file_name()),
                    Ok(_) => file_entries.push(entry),
                    Err(_) => skipped += 1,
                }
            }
            let session_stems: std::collections::HashSet<std::ffi::OsString> = file_entries
                .iter()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
                .filter_map(|p| p.file_stem().map(|s| s.to_os_string()))
                .collect();

            // An orphan: `<uuid>/subagents/*.jsonl` with no `<uuid>.jsonl`
            // beside it. Skipped rather than queued standalone -- see the
            // module doc -- but counted, because silently shedding
            // transcripts is exactly how a format change would go unnoticed.
            for dir_name in session_dirs {
                if session_stems.contains(&dir_name) {
                    continue;
                }
                let Some(orphan_parent) =
                    session_file_for_session_dir(&project_dir.path().join(&dir_name))
                else {
                    continue;
                };
                let (members, excluded) = group_members_for(&orphan_parent);
                skipped += members.len() + excluded;
            }

            for entry in &file_entries {
                if let Some(session) =
                    group_session_ref(entry.path(), discovery_project.clone(), &mut skipped)
                {
                    sessions.push(session);
                }
            }
        }
        if skipped > 0 {
            tracing::warn!(
                skipped,
                "skipped unreadable, orphaned, or mismatched claude-code session entries during discovery"
            );
        }
        Ok(sessions)
    }

    fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript> {
        load_group(&r.path, self.group_budget)
    }

    /// A changed path under a Claude Code root, mapped to the session that
    /// covers it.
    ///
    /// A delegated transcript maps to its PARENT, not to itself. That is
    /// the same group address `discover` emits and the queue keys on: a
    /// subagent transcript is not a session, and offering one as its own
    /// address would put back the 911-cards-for-69-conversations bug from
    /// the other direction. The rule itself is not restated here --
    /// `parent_session_for_member` inverts `subagents_dir_for`, the one
    /// `group_members_for` reads forwards.
    ///
    /// Everything else under the root is `None`, and the exclusions matter
    /// as much as the mappings: the project directory itself, a `CLAUDE.md`,
    /// and anything under the private `memory/` directory all fail to be a
    /// session here for the same reasons `discover` never collects them.
    fn session_for_path(&self, path: &Path) -> Option<PathBuf> {
        // Containment first: real file, really under the declared root, no
        // `..` and no symlink in the chain. Refusing a symlinked member is
        // at least as strict as `group_members_for`, which excludes one.
        let path = real_file_within_root(&self.root, path)?;
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            return None;
        }
        if let Some(parent) = parent_session_for_member(&path) {
            // The parent must itself be a real session file under the root.
            // A `subagents/` directory whose `<uuid>.jsonl` is missing is an
            // orphan, and `discover` refuses to offer those too.
            let parent = real_file_within_root(&self.root, &parent)?;
            return is_top_level_session_path(&self.root, &parent).then_some(parent);
        }
        is_top_level_session_path(&self.root, &path).then_some(path)
    }

    /// The group ref for whichever session a changed path belongs to.
    ///
    /// Two steps, neither of them new: `session_for_path` resolves the
    /// address -- a member resolving to its parent -- and
    /// `group_session_ref` describes it, which is the very function
    /// `discover` builds its refs with. The scoped path and the full sweep
    /// therefore cannot disagree about a session's size or group mtime.
    ///
    /// The cost is the point: this `lstat`s the members of ONE session's
    /// `subagents/` directory, which is what `group_modified_at` is, rather
    /// than every member of every session in the corpus.
    fn session_at(&self, path: &Path) -> anyhow::Result<Option<SessionRef>> {
        let Some(address) = self.session_for_path(path) else {
            return Ok(None);
        };
        let discovery_project = address
            .parent()
            .and_then(|dir| dir.file_name())
            .and_then(discovery_project_label);
        // A session deleted between the event and this lookup is an
        // ordinary race, not a failure: `group_session_ref` returns `None`
        // for a file that is no longer there, and that is `Ok(None)` here.
        // The skip counter it keeps is a discovery-warning statistic and
        // has nothing to say about a single lookup.
        let mut ignored_skips = 0usize;
        Ok(group_session_ref(
            address,
            discovery_project,
            &mut ignored_skips,
        ))
    }
}

/// The best-effort project label for a session, from the name of the
/// project directory holding it.
///
/// Claude Code encodes the session's cwd as a directory name by replacing
/// every '/' with '-' (e.g. `/Users/testuser/code/myproj` becomes
/// `-Users-testuser-code-myproj`). As a placeholder for discovery listings,
/// take the segment after the final '-' as the project basename. This is
/// unreliable for hyphenated project names (a project literally named
/// "my-proj" would only capture "proj" here) -- `load()` overrides it with
/// the true cwd basename read from the session file itself, so it is only
/// ever seen before a session has been loaded.
///
/// Shared by `discover` and `session_at` so a scoped lookup labels a
/// session exactly as a full sweep would.
fn discovery_project_label(project_dir_name: &std::ffi::OsStr) -> Option<String> {
    project_dir_name
        .to_str()
        .and_then(|name| name.rsplit('-').next())
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
}

/// The one way a Claude Code `SessionRef` is built, used by `discover` for
/// every session it walks to and by `session_at` for the single session an
/// event named.
///
/// It covers the top-level session file at `path` *and* every validated
/// transcript under its `<session-uuid>/subagents/` directory.
/// `size_bytes` and `group_modified_at` describe the whole group so the
/// daemon's eligibility check can see a member appear or grow; `path` stays
/// the parent file, which is the one stable address the queue and the
/// upload state key on.
///
/// Shared rather than reimplemented because a scoped scan and a full sweep
/// that described the same session differently would reach different
/// eligibility decisions for the same bytes.
///
/// `None` for anything that is not a session file, including one that has
/// been deleted. `skipped` counts the entries that were unreadable rather
/// than uninteresting, which is what discovery warns on.
fn group_session_ref(
    path: PathBuf,
    discovery_project: Option<String>,
    skipped: &mut usize,
) -> Option<SessionRef> {
    if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
        return None;
    }
    // Refuse symlinks. A later `std::fs::read` follows them, so a symlinked
    // `.jsonl` is a path out of the transcript root and into any file the
    // user can read. `symlink_metadata` does not follow, so it is the check
    // that can tell the difference -- and it is the same stat that supplies
    // the size and mtime below, since for a real file the two agree.
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(m) => m,
        // A vanished session is not an unreadable entry: the file being
        // gone is an answer, and on the event path it is a routine race.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => {
            *skipped += 1;
            return None;
        }
    };
    if !metadata.is_file() {
        return None;
    }
    let parent_modified = metadata
        .modified()
        .ok()
        .map(chrono::DateTime::<chrono::Utc>::from);
    let (members, excluded) = group_members_for(&path);
    *skipped += excluded;

    let size_bytes = members
        .iter()
        .fold(metadata.len(), |acc, m| acc.saturating_add(m.size_bytes));
    let group_modified_at = members
        .iter()
        .filter_map(|m| m.modified_at)
        .chain(parent_modified)
        .max();

    // Keyed on the PARENT's own size and mtime, not the group's: `peek_cwd`
    // reads only the parent file, so a subagent transcript appearing beside
    // it changes `size_bytes` without changing the answer, and keying on the
    // group total would re-read the parent every time a member landed.
    let cwd = peek_cwd_memoized(&path, metadata.len(), parent_modified);
    Some(SessionRef {
        source: SOURCE_CLAUDE_CODE,
        // A native adapter is what it is; nothing to declare.
        declared_source: None,
        path,
        project: discovery_project,
        cwd,
        // The conversation's own start, which is the parent's -- a subagent
        // is spawned partway through it and can only ever be later.
        started_at: parent_modified,
        size_bytes,
        group_modified_at,
        group_member_count: members.len() as u32,
    })
}

/// The directory name Claude Code writes delegated transcripts into.
const SUBAGENTS_DIR_NAME: &str = "subagents";

/// Where the delegated transcripts of the top-level session file `parent`
/// live: `<project-dir>/<session-uuid>/subagents/`.
///
/// The single statement of the parent-to-member layout. `group_members_for`
/// reads it forwards and `parent_session_for_member` inverts it; neither
/// spells the layout out for itself, so the two cannot drift apart.
fn subagents_dir_for(parent: &Path) -> Option<PathBuf> {
    let (stem, dir) = (parent.file_stem()?, parent.parent()?);
    Some(dir.join(stem).join(SUBAGENTS_DIR_NAME))
}

/// The top-level session file that owns the session directory
/// `<project-dir>/<session-uuid>/`.
///
/// Appends the extension rather than `set_extension`, which would eat an
/// existing suffix -- a directory named `a.b` must yield `a.b.jsonl` or
/// nothing, never `a.jsonl`, which names a different session.
fn session_file_for_session_dir(session_dir: &Path) -> Option<PathBuf> {
    let mut file_name = session_dir.file_name()?.to_os_string();
    file_name.push(".jsonl");
    Some(session_dir.with_file_name(file_name))
}

/// The top-level session file a delegated transcript belongs to, or `None`
/// if `member` is not laid out as one.
///
/// This is the group-address rule -- the reason `SessionRef::path` stays the
/// parent and `group_modified_at` exists -- read backwards, and it is
/// deliberately not an independent re-derivation of it. The candidate parent
/// is proposed by inverting the layout and is then required to round-trip
/// through `subagents_dir_for`, the same function `group_members_for` uses
/// to find members in the first place. If the two ever disagreed, this would
/// return `None` rather than invent an address the group rule would not
/// recognise.
///
/// Purely lexical: it asks what a path is named, not what exists on disk.
/// Callers check the filesystem.
fn parent_session_for_member(member: &Path) -> Option<PathBuf> {
    let subagents_dir = member.parent()?;
    let session_dir = subagents_dir.parent()?;
    let candidate = session_file_for_session_dir(session_dir)?;
    (subagents_dir_for(&candidate)?.as_path() == subagents_dir).then_some(candidate)
}

/// Whether `path` is addressed as a top-level session file directly inside a
/// project directory of `root`, which is the only shape `discover` emits a
/// `SessionRef` for: `<root>/<project-dir>/<session-uuid>.jsonl`.
fn is_top_level_session_path(root: &Path, path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("jsonl")
        && path.parent().and_then(|p| p.parent()) == Some(root)
}

/// Every validated delegated transcript belonging to the top-level session
/// file `parent`, sorted by raw file-name bytes, plus a count of the ones
/// excluded.
///
/// The sort is load-bearing rather than tidy: `group_session_hash` folds
/// members in this order, and `entry_id_for`/`submission_id_for` both derive
/// from that hash. `read_dir` order is unspecified, so an unsorted group
/// would mint a different id on every poll and re-offer the same
/// conversation forever.
///
/// This is the single definition of membership, re-derived from the
/// filesystem on every call -- discovery and `load` ask the same question and
/// get the same answer, so the bytes a contributor previews are the bytes an
/// upload sends. Member paths are deliberately never persisted on a queue
/// entry, which would be a second source of truth able to disagree with the
/// disk.
fn group_members_for(parent: &Path) -> (Vec<GroupMember>, usize) {
    let mut members = Vec::new();
    let mut excluded = 0usize;
    let Some(stem) = parent.file_stem() else {
        return (members, excluded);
    };
    // Known layout only: `<project-dir>/<session-uuid>/subagents/*.jsonl`.
    // Deliberately not a general recursive walk -- an unrelated nested
    // directory under a session-uuid dir must not be swept in.
    let Some(subagents_dir) = subagents_dir_for(parent) else {
        return (members, excluded);
    };
    // `read_dir` FOLLOWS a symlinked directory. A `subagents` symlink
    // planted by any process with write access under the transcript root
    // would otherwise steer discovery at arbitrary directories, and
    // everything discovered here is a candidate for upload.
    // `symlink_metadata` does not follow, so a link reports as a symlink and
    // is refused. (The top-level walk is already safe: `DirEntry::file_type`
    // does not follow either.)
    match std::fs::symlink_metadata(&subagents_dir) {
        Ok(md) if md.is_dir() => {}
        _ => return (members, excluded),
    }
    let Ok(entries) = std::fs::read_dir(&subagents_dir) else {
        return (members, excluded);
    };
    let session_uuid = stem.to_string_lossy().into_owned();
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                excluded += 1;
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        // Refuse symlinks, for the same reason and by the same test as the
        // top-level walk: a symlinked member is a path out of the transcript
        // root, and `std::fs::read` would follow it.
        match entry.file_type() {
            Ok(ft) if ft.is_file() => {}
            Ok(_) => continue,
            Err(_) => {
                excluded += 1;
                continue;
            }
        }
        // Stat first, because the memo below is keyed on what a stat
        // returns and the member needs these two facts regardless.
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => {
                excluded += 1;
                continue;
            }
        };
        let size_bytes = metadata.len();
        let modified_at = metadata
            .modified()
            .ok()
            .map(chrono::DateTime::<chrono::Utc>::from);
        // The directory decides membership; `sessionId` verifies it. On the
        // probed machine all 842 members agreed with their grandparent
        // directory, so a disagreement means either a format change or a
        // planted file. Exclude on disagreement, include on absence.
        if let Some(id) = peek_session_id_memoized(&path, size_bytes, modified_at) {
            if id != session_uuid {
                excluded += 1;
                continue;
            }
        }
        members.push(GroupMember {
            path,
            size_bytes,
            modified_at,
        });
    }
    members.sort_by_key(|m| file_name_bytes(&m.path));
    (members, excluded)
}

/// A path's file name as raw bytes, for an ordering that cannot depend on
/// locale or Unicode normalization.
fn file_name_bytes(path: &Path) -> Vec<u8> {
    path.file_name()
        .map(|n| n.as_encoded_bytes().to_vec())
        .unwrap_or_default()
}

/// `peek_session_id`, answered from `SESSION_ID_MEMO` when the file still
/// reports the size and mtime the memoized answer was derived from.
///
/// Discovery calls this once per member per pass, so on a large tree it is
/// the difference between re-reading every delegated transcript every minute
/// and stat-ing them. See `SESSION_ID_MEMO` for why the memo is process-wide
/// and why keying on (size, mtime) is sound here.
fn peek_session_id_memoized(
    path: &Path,
    size_bytes: u64,
    modified_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Option<String> {
    // A poisoned memo is a cache, not state anything depends on: fall
    // through to the read rather than propagating another thread's panic
    // into discovery.
    if let Ok(memo) = SESSION_ID_MEMO.lock() {
        if let Some(hit) = memo.get(path) {
            if hit.size_bytes == size_bytes && hit.modified_at == modified_at {
                return hit.session_id.clone();
            }
        }
    }
    let session_id = peek_session_id(path);
    if let Ok(mut memo) = SESSION_ID_MEMO.lock() {
        if memo.len() >= SESSION_ID_MEMO_CAP && !memo.contains_key(path) {
            memo.clear();
        }
        memo.insert(
            path.to_path_buf(),
            SessionIdMemo {
                size_bytes,
                modified_at,
                session_id: session_id.clone(),
            },
        );
    }
    session_id
}

/// Bounded head-read for a member file's `sessionId`.
///
/// Reads at most `SESSION_ID_PEEK_BYTES` and parses line by line, stopping
/// at the first record carrying the field. A line truncated by the cap
/// simply fails to parse and is skipped, which is the same lenient handling
/// `load_session` gives a malformed line. Returns `None` when the file
/// cannot be read or carries no `sessionId` in its head.
fn peek_session_id(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(SESSION_ID_PEEK_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    let text = String::from_utf8_lossy(&bytes);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let record: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(id) = record.get("sessionId").and_then(|v| v.as_str()) {
            return Some(id.to_string());
        }
    }
    None
}

/// The session hash for a merged group: a domain-separated fold over the
/// parent's digest and each kept member's digest, in sorted order.
///
/// A group with no members and nothing dropped hashes exactly as the bare
/// parent file does. That is not a special case bolted on -- the hash
/// describes the bytes `load` returns, and for a conversation that never
/// delegated anything those bytes are precisely the parent file. It also
/// means adopting grouping leaves every non-delegating session's queue entry
/// and upload receipt undisturbed.
///
/// `dropped` is folded in so a group trimmed to fit the byte budget can
/// never collide with a genuinely smaller group that happens to have the
/// same kept members.
///
/// Member *file names* are deliberately not folded in, and that is a
/// consent decision rather than an oversight. The hash has to cover exactly
/// what the contributor was shown, and a name never reaches them: the
/// boundary marker `load_group` emits carries only `index`, derived from
/// sorted position, and the loaded bytes are the file's contents. So a
/// rename splits into two cases and both are already correct. A rename that
/// leaves the member in the same sorted position produces byte-identical
/// output -- same contents, same concatenation order, same indices -- and
/// the hash rightly does not move, because nothing the approval described
/// has changed. A rename that moves the member in sort order changes the
/// concatenation order, and the fold is order-sensitive, so the hash moves
/// and the uploader's re-hash guard fires. Folding names in would buy
/// nothing for the first case except a spurious re-ask for a conversation
/// that did not change, which is its own consent failure: an approval
/// invalidated for no reason a contributor can see teaches them to click
/// through the next one. `renaming_a_member_moves_the_hash_only_when_sort_
/// order_moves` pins both halves.
fn group_session_hash<B: AsRef<[u8]>>(
    parent_bytes: &[u8],
    member_bytes: &[B],
    dropped: u32,
) -> String {
    if member_bytes.is_empty() && dropped == 0 {
        return session_hash(parent_bytes);
    }
    let mut hasher = Sha256::new();
    hasher.update(b"trace-commons:claude-code-group:v1\0");
    hasher.update(Sha256::digest(parent_bytes));
    for bytes in member_bytes {
        hasher.update(Sha256::digest(bytes.as_ref()));
    }
    hasher.update(dropped.to_le_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// Drop members largest-first until the group fits `GROUP_RAW_BYTE_BUDGET`,
/// returning how many were dropped.
///
/// This runs on the stat'd `GroupMember` list, before a single member file
/// is opened, so the budget caps the bytes the daemon *reads* rather than
/// the bytes it decides to keep afterwards. Deciding after reading meant a
/// `subagents/` directory holding a gigabyte was fully resident before
/// anything was discarded, which is a memory profile set by whatever is on
/// disk rather than by this constant.
///
/// The parent is never dropped: a conversation without the human's own turns
/// is not a conversation. Ties on size break on file name descending, so the
/// choice never depends on `read_dir` order -- two loads of an unchanged
/// group must drop exactly the same members, or the hash moves underneath an
/// approval.
fn apply_group_budget(members: &mut Vec<GroupMember>, parent_len: u64, budget: u64) -> u32 {
    let mut total = members
        .iter()
        .fold(parent_len, |acc, m| acc.saturating_add(m.size_bytes));
    if total <= budget {
        return 0;
    }
    let mut order: Vec<usize> = (0..members.len()).collect();
    order.sort_by(|&a, &b| {
        members[b]
            .size_bytes
            .cmp(&members[a].size_bytes)
            .then_with(|| file_name_bytes(&members[b].path).cmp(&file_name_bytes(&members[a].path)))
    });
    let mut doomed = std::collections::HashSet::new();
    for i in order {
        if total <= budget {
            break;
        }
        total = total.saturating_sub(members[i].size_bytes);
        doomed.insert(i);
    }
    let dropped = doomed.len() as u32;
    let mut index = 0usize;
    members.retain(|_| {
        let keep = !doomed.contains(&index);
        index += 1;
        keep
    });
    dropped
}

/// Cheap discovery-time peek at a session file's true working directory:
/// parses each line as JSON in turn and stops at the first record carrying
/// a `cwd` field, skipping the full parse of the file's events. Reads the
/// file the same way `load_session` does (`std::fs::read` then
/// `String::from_utf8_lossy`), so an invalid-UTF-8 line elsewhere in the
/// file does not abort the scan before it reaches a later cwd-bearing line.
/// `load_session` never errors on bad UTF-8 either, so peek and load must
/// tolerate it identically, or `--project` filtering can silently disagree
/// with what `submit_sessions` actually delivers. Mirrors the exact field
/// access `load_session` uses (`record.get("cwd").and_then(|v|
/// v.as_str())`). Returns `None` if the file cannot be read or no record
/// carries `cwd`.
///
/// Cost: the file is still read whole (as `load_session` does); what is
/// saved is parsing and building every event. Discovery therefore pays one
/// read per session file, which is far less than the full loads the
/// interactive picker already performs.
fn peek_cwd(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        // `read_until` and a lossy conversion per line, not `lines()`: the
        // old whole-file `from_utf8_lossy` tolerated invalid UTF-8 anywhere
        // in the file, and `lines()` would abort the iteration instead. A
        // transcript whose later records are unreadable must still report
        // its cwd, or `--project` filtering silently disagrees with what
        // `submit_sessions` delivers.
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => return None,
            Ok(_) => {}
            Err(_) => return None,
        }
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let record: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(c) = record.get("cwd").and_then(|v| v.as_str()) {
            return Some(c.to_string());
        }
    }
}

/// How many memoized cwd answers to hold. See `SESSION_ID_MEMO_CAP`.
const CWD_MEMO_CAP: usize = 8192;

/// One memoized answer from `peek_cwd`, valid while the file it describes
/// still reports the same size and mtime.
#[derive(Debug, Clone)]
struct CwdMemo {
    size_bytes: u64,
    modified_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Memoized even when absent: a transcript carrying no `cwd` is the
    /// expensive case, because establishing that means reading to the end.
    cwd: Option<String>,
}

/// Process-wide memo for `peek_cwd`, keyed on the transcript's path.
///
/// The sibling of `SESSION_ID_MEMO`, for the peek that comment already calls
/// "the far more expensive whole-file cwd peek". Discovery runs on every
/// watcher tick and peeked every parent transcript unmemoized; on a real
/// corpus that is ~0.9GB of Claude transcripts re-read every
/// `poll_interval_secs` to re-derive an answer that changes only when a file
/// does. Same (size, mtime) key and the same reasoning about why that is not
/// a trust boundary.
static CWD_MEMO: LazyLock<Mutex<HashMap<PathBuf, CwdMemo>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// `peek_cwd`, answered from `CWD_MEMO` when the file still reports the size
/// and mtime the memoized answer was derived from.
fn peek_cwd_memoized(
    path: &Path,
    size_bytes: u64,
    modified_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Option<String> {
    if let Ok(memo) = CWD_MEMO.lock() {
        if let Some(hit) = memo.get(path) {
            if hit.size_bytes == size_bytes && hit.modified_at == modified_at {
                return hit.cwd.clone();
            }
        }
    }
    let cwd = peek_cwd(path);
    if let Ok(mut memo) = CWD_MEMO.lock() {
        if memo.len() >= CWD_MEMO_CAP && !memo.contains_key(path) {
            memo.clear();
        }
        memo.insert(
            path.to_path_buf(),
            CwdMemo {
                size_bytes,
                modified_at,
                cwd: cwd.clone(),
            },
        );
    }
    cwd
}

/// Everything one transcript file contributes: its mapped events plus the
/// session-level metadata its records carry.
///
/// Split out of the old `load_session` so a group can parse its parent and
/// each member through exactly the same code. A second spelling of this
/// mapping is how a delegated transcript would start being read by
/// different rules than the conversation it belongs to.
struct ParsedSession {
    events: Vec<SessionEvent>,
    model: Option<String>,
    agent_version: Option<String>,
    cwd: Option<String>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    unparseable: usize,
}

/// Read one top-level session file and every delegated transcript beside
/// it, as a single conversation.
///
/// The budget is applied here rather than at send time, so `build_preview`
/// and the upload both describe the same bytes by construction -- see
/// `GROUP_RAW_BYTE_BUDGET`.
fn load_group(parent: &Path, budget: u64) -> anyhow::Result<SessionTranscript> {
    let parent_bytes = std::fs::read(parent)?;
    let (mut member_refs, _excluded) = group_members_for(parent);
    // Budget first, then read. `group_members_for` has already stat'd every
    // member, so the over-budget tail can be dropped without ever being
    // opened -- see `apply_group_budget`.
    let dropped = apply_group_budget(&mut member_refs, parent_bytes.len() as u64, budget);
    // A member that vanished or turned unreadable between the directory
    // listing and here is simply not in the group: the hash covers what was
    // actually read, so a preview and the upload that follows it cannot
    // describe different bytes. It is not counted as `dropped`, which means
    // one specific thing -- left out to fit the budget.
    let members: Vec<(PathBuf, Vec<u8>)> = member_refs
        .into_iter()
        .filter_map(|m| std::fs::read(&m.path).ok().map(|bytes| (m.path, bytes)))
        .collect();
    let kept = members.len() as u32;

    let member_slices: Vec<&[u8]> = members.iter().map(|(_, b)| b.as_slice()).collect();
    let hash = group_session_hash(&parent_bytes, &member_slices, dropped);

    let parsed = parse_session(&parent_bytes);
    let mut events = parsed.events;
    let mut unparseable = parsed.unparseable;

    if kept > 0 || dropped > 0 {
        // One header for the whole delegated section, emitted even when the
        // budget left nothing, so a trimmed conversation says so rather than
        // reading as one that never delegated. Structural only: counts, no
        // content -- the same contract every other `Opaque` event honours.
        events.push(SessionEvent {
            served_by: None,
            kind: SessionEventKind::Opaque,
            timestamp: None,
            content: None,
            structured: json!({
                "record_type": "subagent_group",
                "subagent_count": kept,
                "subagents_dropped": dropped,
            }),
            tool_name: None,
            token_counts: None,
            tool_call_id: None,
            success: None,
        });
    }
    for (index, (_, bytes)) in members.iter().enumerate() {
        // A boundary marker rather than a parent/child link. `SessionEvent`
        // carries no id and no `tool_call_id`, and event ids are minted at
        // envelope-build time, so tying a member's events to the specific
        // `Task` call that spawned them would be a correspondence this
        // adapter cannot actually verify. Segment order is a fact; parentage
        // would be a claim.
        events.push(SessionEvent {
            served_by: None,
            kind: SessionEventKind::Opaque,
            timestamp: None,
            content: None,
            structured: json!({ "record_type": "subagent_transcript", "index": index }),
            tool_name: None,
            token_counts: None,
            tool_call_id: None,
            success: None,
        });
        let member = parse_session(bytes);
        unparseable += member.unparseable;
        events.extend(member.events);
    }

    if unparseable > 0 {
        tracing::warn!(unparseable, "skipped unparseable Claude Code record lines");
    }

    // Session metadata describes the conversation, so it comes from the
    // parent alone. A subagent runs under the same cwd and harness, and
    // may well run under a different model; taking any of it from a member
    // would let a delegated transcript relabel the conversation.
    let cwd = parsed.cwd;
    let project = cwd
        .as_deref()
        .map(Path::new)
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());

    // The top-level session file's own stem, already the identifier
    // `group_members_for` uses to verify member `sessionId`s against --
    // see `session_uuid` there. Not invented for this purpose; it is the
    // address this session has always been read at.
    let conversation_id = parent
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string());

    Ok(SessionTranscript {
        source: Cow::Borrowed(SOURCE_CLAUDE_CODE),
        agent_version: parsed.agent_version,
        model: parsed.model,
        project,
        cwd,
        started_at: parsed.started_at,
        session_hash: hash,
        conversation_id,
        events,
        subagent_count: kept,
        subagents_dropped: dropped,
        routing: Vec::new(),
        attested_call: None,
    })
}

fn parse_session(bytes: &[u8]) -> ParsedSession {
    let text = String::from_utf8_lossy(bytes);

    let mut events = Vec::new();
    let mut model: Option<String> = None;
    let mut agent_version: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut started_at: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut unparseable = 0usize;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let record: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                unparseable += 1;
                continue;
            }
        };

        let record_timestamp = record
            .get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));
        if started_at.is_none() {
            if let Some(ts) = record_timestamp {
                started_at = Some(ts);
            }
        }
        if cwd.is_none() {
            if let Some(c) = record.get("cwd").and_then(|v| v.as_str()) {
                cwd = Some(c.to_string());
            }
        }
        if agent_version.is_none() {
            if let Some(v) = record.get("version").and_then(|v| v.as_str()) {
                agent_version = Some(v.to_string());
            }
        }

        let record_type = record.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match record_type {
            "user" => {
                map_user_record(&record, record_timestamp, &mut events);
            }
            "assistant" => {
                if model.is_none() {
                    if let Some(m) = record.pointer("/message/model").and_then(|v| v.as_str()) {
                        model = Some(m.to_string());
                    }
                }
                map_assistant_record(&record, record_timestamp, &mut events);
            }
            other => {
                events.push(SessionEvent {
                    served_by: None,
                    kind: SessionEventKind::Opaque,
                    timestamp: record_timestamp,
                    content: None,
                    structured: json!({ "record_type": other }),
                    tool_name: None,
                    token_counts: None,
                    tool_call_id: None,
                    success: None,
                });
            }
        }
    }

    ParsedSession {
        events,
        model,
        agent_version,
        cwd,
        started_at,
        unparseable,
    }
}

fn map_user_record(
    record: &Value,
    timestamp: Option<chrono::DateTime<chrono::Utc>>,
    events: &mut Vec<SessionEvent>,
) {
    let content = record.pointer("/message/content");
    match content {
        Some(Value::String(s)) => {
            events.push(SessionEvent {
                served_by: None,
                kind: SessionEventKind::User,
                timestamp,
                content: Some(s.clone()),
                structured: Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: None,
                success: None,
            });
        }
        Some(Value::Array(blocks)) => {
            let mut texts = Vec::new();
            for block in blocks {
                match block.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                            texts.push(t.to_string());
                        }
                    }
                    Some("tool_result") => {
                        let flattened = flatten_block_content(block.get("content"));
                        events.push(SessionEvent {
                            served_by: None,
                            kind: SessionEventKind::ToolResult,
                            timestamp,
                            content: flattened,
                            structured: Value::Null,
                            tool_name: None,
                            token_counts: None,
                            tool_call_id: block
                                .get("tool_use_id")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            // `is_error` is the transcript's own verdict on
                            // the call. Absent, the harness did not say.
                            success: block.get("is_error").and_then(|v| v.as_bool()).map(|e| !e),
                        });
                    }
                    _ => {}
                }
            }
            if !texts.is_empty() {
                events.push(SessionEvent {
                    served_by: None,
                    kind: SessionEventKind::User,
                    timestamp,
                    content: Some(texts.join("\n")),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: None,
                    tool_call_id: None,
                    success: None,
                });
            }
        }
        _ => {}
    }
}

fn flatten_block_content(content: Option<&Value>) -> Option<String> {
    match content {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(blocks)) => {
            let texts: Vec<String> = blocks
                .iter()
                .filter_map(|b| {
                    if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
                        Some(t.to_string())
                    } else {
                        b.as_str().map(|s| s.to_string())
                    }
                })
                .collect();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n"))
            }
        }
        _ => None,
    }
}

fn map_assistant_record(
    record: &Value,
    timestamp: Option<chrono::DateTime<chrono::Utc>>,
    events: &mut Vec<SessionEvent>,
) {
    let usage = record.pointer("/message/usage");
    let token_counts = usage.map(|u| {
        let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let output = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        (input, output)
    });
    // What it would have cost needs more than those two counts, and is built
    // only from a record that states all of it -- see `served_by_of`.
    let served_by = usage.and_then(|u| served_by_of(record, u));

    let Some(Value::Array(blocks)) = record.pointer("/message/content") else {
        return;
    };
    let record_start = events.len();

    // Contiguous runs of text blocks are joined into one Assistant event and
    // emitted where the run ends, so every block keeps its position relative
    // to the reasoning and tool calls around it. Joining ALL text and hoisting
    // it to the first text position (the previous behavior) reordered the
    // transcript: prose written after a thinking block or a tool result
    // appeared before it. For a corpus whose value is showing how an agent
    // actually proceeded, that ordering is the signal.
    //
    // `token_counts` belongs to the record as a whole, so it is attached to
    // the first emitted Assistant event only, rather than being duplicated
    // across every run.
    let mut texts: Vec<String> = Vec::new();
    let mut token_counts_unused = token_counts;
    let mut served_by_unused = served_by;
    macro_rules! flush_text {
        () => {
            if !texts.is_empty() {
                events.push(SessionEvent {
                    served_by: served_by_unused.take(),
                    kind: SessionEventKind::Assistant,
                    timestamp,
                    content: Some(texts.join("\n")),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: token_counts_unused.take(),
                    tool_call_id: None,
                    success: None,
                });
                texts.clear();
            }
        };
    }
    for block in blocks {
        match block.get("type").and_then(|v| v.as_str()) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    texts.push(t.to_string());
                }
            }
            Some("tool_use") => {
                flush_text!();
                let name = block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let input = block.get("input").cloned().unwrap_or(Value::Null);
                events.push(SessionEvent {
                    served_by: None,
                    kind: SessionEventKind::ToolCall,
                    timestamp,
                    content: None,
                    structured: input,
                    tool_name: name,
                    token_counts: None,
                    tool_call_id: block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    success: None,
                });
            }
            Some("thinking") => {
                flush_text!();
                // Reasoning is captured as a first-class event and redacted
                // through the same client-side pipeline as every other kind.
                if let Some(t) = block.get("thinking").and_then(|v| v.as_str()) {
                    if !t.is_empty() {
                        events.push(SessionEvent {
                            served_by: None,
                            kind: SessionEventKind::Reasoning,
                            timestamp,
                            content: Some(t.to_string()),
                            structured: Value::Null,
                            tool_name: None,
                            token_counts: None,
                            tool_call_id: None,
                            success: None,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    flush_text!();

    // A record whose content is only tool calls (or only reasoning) emits no
    // Assistant event, and its usage was dropped on the floor -- with it, the
    // whole price of the step. Tool-only turns are most of a coding session,
    // so where no text event claimed the usage, it goes to the first event
    // the record did produce. Still once per record, never duplicated.
    if let Some(first) = events.get_mut(record_start) {
        if first.token_counts.is_none() {
            first.token_counts = token_counts_unused.take();
            first.served_by = served_by_unused.take();
        }
    }
}

/// The provider's complete usage report for one assistant record, or `None`
/// where the record does not state one this crate can price.
///
/// Every branch that returns `None` here is a refusal to guess:
///
/// * an unnamed model -- a step served by an unknown model has no price;
/// * a count the record did not state -- reading it as zero would understate
///   the step, and `input_tokens`/`output_tokens` are checked here even
///   though [`SessionEvent::token_counts`] reads them with a zero default,
///   so a defaulted zero can never reach a price;
/// * cache-creation tokens with no 5m/1h breakdown, or one that does not add
///   up to the total the record itself reports -- the two durations are
///   priced 1.25x and 2x base input, so guessing the cheaper one understates
///   by up to 1.6x;
/// * a pricing modifier [`crate::pricing`] does not model -- a non-standard
///   service tier, fast mode, or US-pinned inference, each of which changes
///   what the tokens cost.
fn served_by_of(record: &Value, usage: &Value) -> Option<crate::source::ServedBy> {
    let model = record
        .pointer("/message/model")
        .and_then(|v| v.as_str())
        .filter(|m| !m.is_empty())?;

    // An absent field is the common case (older transcripts predate it) and
    // means the default: standard tier, standard speed, global inference.
    // A value that is present and not one this crate can price is a refusal.
    // `not_available` is what Claude Code writes when the request carried no
    // `inference_geo` at all, so it reads as the unpinned default; the 1.1x
    // multiplier applies only to inference explicitly pinned to `us`.
    fn modifier_is_standard(field: Option<&Value>, standard: &[&str]) -> bool {
        match field {
            None | Some(Value::Null) => true,
            Some(Value::String(value)) => standard.contains(&value.as_str()),
            Some(_) => false,
        }
    }
    if !modifier_is_standard(usage.get("service_tier"), &["standard"])
        || !modifier_is_standard(usage.get("speed"), &["standard"])
        || !modifier_is_standard(usage.get("inference_geo"), &["global", "not_available"])
    {
        return None;
    }

    fn stated_count(object: &Value, key: &str) -> Option<u32> {
        u32::try_from(object.get(key)?.as_u64()?).ok()
    }

    stated_count(usage, "input_tokens")?;
    stated_count(usage, "output_tokens")?;
    let cache_read_tokens = stated_count(usage, "cache_read_input_tokens")?;

    let created = stated_count(usage, "cache_creation_input_tokens")?;
    let (cache_write_5m_tokens, cache_write_1h_tokens) = if created == 0 {
        (0, 0)
    } else {
        let split = usage.get("cache_creation")?;
        let five_minute = stated_count(split, "ephemeral_5m_input_tokens")?;
        let one_hour = stated_count(split, "ephemeral_1h_input_tokens")?;
        if five_minute.checked_add(one_hour)? != created {
            return None;
        }
        (five_minute, one_hour)
    };

    Some(crate::source::ServedBy {
        model: model.to_string(),
        cache_read_tokens,
        cache_write_5m_tokens,
        cache_write_1h_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{SessionEventKind, TraceSource};
    use std::path::PathBuf;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code")
    }

    #[test]
    #[cfg(unix)]
    fn discovery_refuses_symlinks_that_escape_the_transcript_root() {
        // Everything discovery returns is a candidate for upload, so a
        // symlink planted under the transcript root by any same-user process
        // must not be able to steer it at files elsewhere on disk.
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        std::fs::write(
            outside.path().join("secrets.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"private\"}}\n",
        )
        .unwrap();

        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("-Users-testuser-code-myproj");

        // Case 1: the `subagents` directory itself is a symlink outward.
        let session_a = project_dir.join("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
        std::fs::create_dir_all(&session_a).unwrap();
        symlink(outside.path(), session_a.join("subagents")).unwrap();

        // Case 2: a real `subagents` directory holding a symlinked file.
        let session_b = project_dir.join("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
        let subagents_b = session_b.join("subagents");
        std::fs::create_dir_all(&subagents_b).unwrap();
        symlink(
            outside.path().join("secrets.jsonl"),
            subagents_b.join("agent-link.jsonl"),
        )
        .unwrap();

        let found = ClaudeCodeSource::new(root.path().to_path_buf())
            .discover()
            .unwrap();
        assert!(
            found.is_empty(),
            "symlinks must not be discovered, got: {found:?}"
        );
    }

    /// Claude Code's private auto-memory must never be collected.
    ///
    /// `~/.claude/projects/<encoded-cwd>/` holds more than transcripts: it
    /// also carries a `memory/` directory of `.md` files that are the
    /// assistant's own notes about its user -- who they are, what they have
    /// been told to do differently, what they are working on. That is some of
    /// the most personal material on the machine, and this daemon watches the
    /// directory it lives in.
    ///
    /// Nothing collects it today, and this test exists because that was an
    /// accident rather than a decision. TWO independent mechanisms exclude
    /// it, and it matters which does what -- the obvious answer is wrong:
    ///
    /// - `memory/` is a DIRECTORY, so the top-level `.jsonl` extension filter
    ///   never sees the files inside it at all. What keeps them out is that
    ///   descent is restricted to the known `<uuid>/subagents/` layout and is
    ///   deliberately not a general recursive walk. Widening the extension
    ///   filter alone does not expose them; widening descent does.
    /// - The extension filter is what excludes a `.md` sitting directly in
    ///   the project directory, such as a `CLAUDE.md`.
    ///
    /// Both are asserted below, each with a fixture that fails if its own
    /// mechanism is removed. The `.jsonl` planted inside `memory/` is the one
    /// that pins the descent rule: it cannot be excluded by extension, so if
    /// it is ever collected, discovery has started walking directories it
    /// should not.
    ///
    /// This asserts the INTENT: private auto-memory is out of scope for
    /// collection, deliberately. Anyone widening what discovery sweeps --
    /// collecting `.md`, walking subdirectories generally, following a
    /// "richer context" instinct -- should have to delete a test that says
    /// why not, rather than silently start uploading a user's memory.
    #[test]
    fn private_auto_memory_is_never_collected() {
        let session = "33333333-3333-3333-3333-333333333333";
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("-Users-testuser-code-myproj");

        // The real layout, per Claude Code's own directory documentation.
        let memory_dir = project_dir.join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::write(
            memory_dir.join("MEMORY.md"),
            "- [A note](note.md) - something the assistant recorded\n",
        )
        .unwrap();
        std::fs::write(
            memory_dir.join("some_note.md"),
            "---\nname: some-note\n---\n\nA private fact about the user.\n",
        )
        .unwrap();
        // Pins the descent rule specifically: no extension filter can save
        // this one, so collecting it would mean discovery walked into a
        // directory outside the known layout.
        std::fs::write(
            memory_dir.join("not-a-transcript.jsonl"),
            record(session, "must never be collected from memory/"),
        )
        .unwrap();

        // Pins the extension filter: a markdown file directly in the project
        // directory, which the top-level walk does see.
        std::fs::write(
            project_dir.join("CLAUDE.md"),
            "# Project instructions\n\nPrivate project notes.\n",
        )
        .unwrap();

        // A real transcript beside it, so a test that passes by discovering
        // nothing at all cannot masquerade as this property holding.
        std::fs::write(
            project_dir.join(format!("{session}.jsonl")),
            record(session, "a real transcript"),
        )
        .unwrap();

        let found = ClaudeCodeSource::new(root.path().to_path_buf())
            .discover()
            .unwrap();

        assert_eq!(
            found.len(),
            1,
            "expected exactly the transcript to be discovered, got: {found:?}"
        );
        assert!(
            found[0].path.extension().and_then(|e| e.to_str()) == Some("jsonl"),
            "the discovered session must be the transcript, got: {:?}",
            found[0].path
        );
        for session_ref in &found {
            let path = session_ref.path.to_string_lossy();
            assert!(
                !path.contains("memory"),
                "a file under the project's memory/ directory was discovered: {path}"
            );
            assert!(
                !path.ends_with(".md"),
                "a markdown file was discovered: {path}"
            );
        }

        // Group membership is checked too, not just the discovered refs. A
        // memory file swept in as a *member* would upload with the session
        // and never appear as a ref of its own, so the assertions above
        // would not see it. This project directory has no `subagents/`
        // layout at all, so the only correct answer is zero.
        assert_eq!(
            found[0].group_member_count, 0,
            "memory files were attached to the session as group members"
        );
    }

    /// A delegated transcript is not a session; its parent is.
    ///
    /// This is the group-address rule seen from the event side. If a
    /// subagent transcript mapped to itself, event-driven scanning would
    /// scope a scan to an address the queue has never heard of, and the 911
    /// files describing 69 conversations would be back.
    #[test]
    fn a_subagent_transcript_maps_to_the_parent_session() {
        let (root, parent) = group_fixture(&[("agent-a.jsonl", "a"), ("agent-b.jsonl", "b")]);
        let source = ClaudeCodeSource::new(root.path().to_path_buf());
        let subagents = subagents_dir_for(&parent).unwrap();

        for member in ["agent-a.jsonl", "agent-b.jsonl"] {
            assert_eq!(
                source.session_for_path(&subagents.join(member)),
                Some(parent.clone()),
                "{member} must address the conversation, not itself"
            );
        }

        // And the address it produces is the one discovery emits, not
        // merely something shaped like it.
        let found = source.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, parent);
    }

    #[test]
    fn a_top_level_session_maps_to_itself() {
        let (root, parent) = group_fixture(&[("agent-a.jsonl", "a")]);
        let source = ClaudeCodeSource::new(root.path().to_path_buf());
        assert_eq!(source.session_for_path(&parent), Some(parent.clone()));
    }

    #[test]
    fn nothing_that_is_not_a_session_maps_to_one() {
        let (root, parent) = group_fixture(&[("agent-a.jsonl", "a")]);
        let source = ClaudeCodeSource::new(root.path().to_path_buf());
        let project_dir = parent.parent().unwrap().to_path_buf();

        // The private auto-memory directory, by both of the mechanisms
        // `private_auto_memory_is_never_collected` pins: a `.md` beside the
        // transcripts, and a `.jsonl` inside `memory/` that no extension
        // filter could exclude.
        let memory = project_dir.join("memory");
        std::fs::create_dir_all(&memory).unwrap();
        std::fs::write(memory.join("some_note.md"), "private").unwrap();
        std::fs::write(memory.join("not-a-transcript.jsonl"), "{}\n").unwrap();
        std::fs::write(project_dir.join("CLAUDE.md"), "notes").unwrap();

        // An orphan: a delegated transcript whose parent file is gone. The
        // walk refuses to offer those, so no event may name one either.
        let orphan_dir = project_dir
            .join("44444444-4444-4444-4444-444444444444")
            .join("subagents");
        std::fs::create_dir_all(&orphan_dir).unwrap();
        std::fs::write(orphan_dir.join("agent-x.jsonl"), "{}\n").unwrap();

        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("elsewhere.jsonl"), "{}\n").unwrap();

        for path in [
            memory.join("some_note.md"),
            memory.join("not-a-transcript.jsonl"),
            project_dir.join("CLAUDE.md"),
            orphan_dir.join("agent-x.jsonl"),
            // Directories are not sessions, at any level.
            project_dir.clone(),
            root.path().to_path_buf(),
            subagents_dir_for(&parent).unwrap(),
            // Outside the root entirely.
            outside.path().join("elsewhere.jsonl"),
            // Missing files are not sessions either.
            project_dir.join("55555555-5555-5555-5555-555555555555.jsonl"),
        ] {
            assert_eq!(
                source.session_for_path(&path),
                None,
                "{} must not address a session",
                path.display()
            );
        }
    }

    /// The mapping is an addressing surface fed by the operating system, so
    /// it must be at least as strict as
    /// `discovery_refuses_symlinks_that_escape_the_transcript_root`.
    ///
    /// Both escapes that test plants are checked here, against a fixture
    /// whose parent session file really exists -- so a `None` answer is the
    /// symlink being refused and not merely an orphan being skipped -- plus
    /// the traversal case, which has no discovery equivalent because
    /// discovery only ever walks outwards from the root while this is
    /// handed a path.
    #[test]
    #[cfg(unix)]
    fn path_mapping_refuses_symlinks_and_traversal_out_of_the_root() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secrets.jsonl");
        std::fs::write(&secret, "{}\n").unwrap();

        let (root, parent) = group_fixture(&[("agent-real.jsonl", "real")]);
        let source = ClaudeCodeSource::new(root.path().to_path_buf());
        let project_dir = parent.parent().unwrap().to_path_buf();
        let subagents = subagents_dir_for(&parent).unwrap();

        // A symlinked member beside a real one, under a real parent.
        let linked_member = subagents.join("agent-link.jsonl");
        symlink(&secret, &linked_member).unwrap();

        // A second session whose `subagents` directory is itself a symlink
        // pointing out of the root.
        let session_b = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
        let parent_b = project_dir.join(format!("{session_b}.jsonl"));
        std::fs::write(&parent_b, record(session_b, "b")).unwrap();
        std::fs::create_dir_all(project_dir.join(session_b)).unwrap();
        symlink(
            outside.path(),
            project_dir.join(session_b).join(SUBAGENTS_DIR_NAME),
        )
        .unwrap();

        for escape in [
            linked_member,
            project_dir
                .join(session_b)
                .join(SUBAGENTS_DIR_NAME)
                .join("secrets.jsonl"),
            // Spelled with the root as a prefix, but not under it.
            project_dir.join("..").join("..").join("secrets.jsonl"),
            root.path().join("..").join("secrets.jsonl"),
        ] {
            assert_eq!(
                source.session_for_path(&escape),
                None,
                "{} must not address a session",
                escape.display()
            );
        }

        // The control: the real member beside the symlinked one still maps,
        // so this test cannot pass by refusing everything.
        assert_eq!(
            source.session_for_path(&subagents.join("agent-real.jsonl")),
            Some(parent.clone())
        );
    }

    /// Every field of a `SessionRef`, as one comparable string.
    ///
    /// `SessionRef` is not `PartialEq`, and hand-listing its fields in an
    /// assertion would quietly stop covering the next one somebody adds --
    /// which is exactly the field a scoped scan and a full sweep would then
    /// be free to disagree about. `Debug` covers all of them, now and later.
    fn every_field(r: &SessionRef) -> String {
        format!("{r:?}")
    }

    /// The property the whole extraction exists for: a scoped lookup and a
    /// full sweep describe the same session identically.
    ///
    /// If they diverged on `size_bytes` or `group_modified_at`, the two
    /// paths would reach different eligibility decisions for the same
    /// bytes -- the drift event-driven watching is meant to avoid rather
    /// than introduce.
    #[test]
    fn session_at_describes_a_session_exactly_as_discover_does() {
        let (root, parent) = group_fixture(&[("agent-a.jsonl", "a"), ("agent-b.jsonl", "b")]);
        let source = ClaudeCodeSource::new(root.path().to_path_buf());

        let discovered = source.discover().unwrap();
        assert_eq!(discovered.len(), 1);
        let scoped = source.session_at(&parent).unwrap().expect("a session");

        assert_eq!(every_field(&scoped), every_field(&discovered[0]));
        // Named individually too, so a failure says which fact moved.
        assert_eq!(scoped.path, parent);
        assert_eq!(scoped.source, SOURCE_CLAUDE_CODE);
        assert_eq!(scoped.group_member_count, 2);
        assert_eq!(scoped.size_bytes, discovered[0].size_bytes);
        assert_eq!(scoped.group_modified_at, discovered[0].group_modified_at);
        assert_eq!(scoped.cwd, discovered[0].cwd);
        assert_eq!(scoped.project, discovered[0].project);
    }

    /// A member event returns the PARENT's ref, and the group mtime it
    /// carries is the member's.
    ///
    /// This is the reason `session_at` exists rather than a bare address:
    /// the thing that moved is the member, and the fact eligibility is
    /// judged on is a group mtime that must have noticed it.
    #[test]
    fn session_at_on_a_member_returns_the_parent_ref_dated_by_that_member() {
        let (root, parent) = group_fixture(&[("agent-a.jsonl", "a")]);
        let source = ClaudeCodeSource::new(root.path().to_path_buf());
        let member = subagents_dir_for(&parent).unwrap().join("agent-a.jsonl");

        // Stamp the member an hour ahead of the parent, so "the group mtime
        // is the member's" cannot pass by the two happening to be equal at
        // filesystem timestamp resolution.
        let ahead = std::fs::metadata(&parent).unwrap().modified().unwrap()
            + std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&member)
            .unwrap()
            .set_modified(ahead)
            .unwrap();

        let scoped = source.session_at(&member).unwrap().expect("a session");
        assert_eq!(scoped.path, parent, "a member addresses its parent");
        assert_eq!(scoped.group_member_count, 1);
        assert_eq!(
            scoped.group_modified_at,
            Some(chrono::DateTime::<chrono::Utc>::from(ahead)),
            "the group mtime must reflect the member that moved"
        );
        assert_ne!(
            scoped.group_modified_at, scoped.started_at,
            "the parent's own mtime is not the group's"
        );
        // And still identical to what a full sweep would produce.
        let discovered = source.discover().unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(every_field(&scoped), every_field(&discovered[0]));
    }

    /// These paths come from filesystem events, so a session deleted
    /// between the event and the lookup is an ordinary race. It is
    /// `Ok(None)`, never an error.
    #[test]
    fn a_session_that_vanished_between_the_event_and_the_lookup_is_ok_none() {
        let (root, parent) = group_fixture(&[("agent-a.jsonl", "a")]);
        let source = ClaudeCodeSource::new(root.path().to_path_buf());
        let member = subagents_dir_for(&parent).unwrap().join("agent-a.jsonl");

        assert!(source.session_at(&parent).unwrap().is_some());

        // The member is gone: nothing to scan, and the parent is still
        // addressable in its own right.
        std::fs::remove_file(&member).unwrap();
        assert!(
            source.session_at(&member).unwrap().is_none(),
            "a deleted member must be Ok(None)"
        );
        assert!(source.session_at(&parent).unwrap().is_some());

        // The whole conversation is gone.
        std::fs::remove_file(&parent).unwrap();
        assert!(
            source.session_at(&parent).unwrap().is_none(),
            "a deleted session must be Ok(None), not an error"
        );
        assert!(source.discover().unwrap().is_empty());
    }

    /// `session_at` inherits every refusal `session_for_path` makes,
    /// because it is built on it rather than beside it. Same escapes as
    /// `path_mapping_refuses_symlinks_and_traversal_out_of_the_root`.
    #[test]
    #[cfg(unix)]
    fn session_at_refuses_everything_the_mapping_refuses() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secrets.jsonl");
        std::fs::write(&secret, "{}\n").unwrap();

        let (root, parent) = group_fixture(&[("agent-real.jsonl", "real")]);
        let source = ClaudeCodeSource::new(root.path().to_path_buf());
        let project_dir = parent.parent().unwrap().to_path_buf();
        let subagents = subagents_dir_for(&parent).unwrap();
        let linked_member = subagents.join("agent-link.jsonl");
        symlink(&secret, &linked_member).unwrap();

        for escape in [
            secret.clone(),
            linked_member,
            project_dir.join("..").join("..").join("secrets.jsonl"),
            project_dir.join("CLAUDE.md"),
            project_dir.clone(),
        ] {
            assert!(
                source.session_at(&escape).unwrap().is_none(),
                "{} must not resolve to a session",
                escape.display()
            );
        }
        assert!(
            source
                .session_at(&subagents.join("agent-real.jsonl"))
                .unwrap()
                .is_some(),
            "the real member must still resolve, or this proves nothing"
        );
    }

    /// The inverse rule is checked against the forward one rather than
    /// restated, so a path that does not round-trip is refused instead of
    /// being given an address `group_members_for` would not recognise.
    #[test]
    fn the_parent_rule_round_trips_through_the_member_rule() {
        let parent = PathBuf::from("/r/-proj/22222222-2222-2222-2222-222222222222.jsonl");
        let subagents = subagents_dir_for(&parent).unwrap();
        assert_eq!(
            parent_session_for_member(&subagents.join("agent-a.jsonl")),
            Some(parent)
        );
        // A file directly in the session directory, and one nested deeper
        // than the known layout, are neither of them members.
        assert_eq!(
            parent_session_for_member(Path::new(
                "/r/-proj/22222222-2222-2222-2222-222222222222/loose.jsonl"
            )),
            None
        );
        assert_eq!(
            parent_session_for_member(&subagents.join("deeper").join("agent-a.jsonl")),
            None
        );
        // A dotted session directory must not be truncated into a
        // different session's address.
        assert_eq!(
            parent_session_for_member(Path::new("/r/-proj/a.b/subagents/agent-a.jsonl")),
            Some(PathBuf::from("/r/-proj/a.b.jsonl"))
        );
    }

    /// A minimal top-level session record. `session` is the uuid that names
    /// both the file and the `subagents/` directory beside it.
    fn record(session: &str, text: &str) -> String {
        format!(
            "{{\"type\":\"user\",\"cwd\":\"/Users/testuser/code/myproj\",\
             \"sessionId\":\"{session}\",\
             \"message\":{{\"role\":\"user\",\"content\":\"{text}\"}}}}\n"
        )
    }

    /// A project dir holding one top-level session plus `members` delegated
    /// transcripts, each stamped with the parent's `sessionId`. Returns the
    /// root and the parent's path.
    fn group_fixture(members: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
        let session = "22222222-2222-2222-2222-222222222222";
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("-Users-testuser-code-myproj");
        std::fs::create_dir_all(&project_dir).unwrap();
        let parent = project_dir.join(format!("{session}.jsonl"));
        std::fs::write(&parent, record(session, "parent turn")).unwrap();
        if !members.is_empty() {
            let subagents = project_dir.join(session).join("subagents");
            std::fs::create_dir_all(&subagents).unwrap();
            for (name, text) in members {
                std::fs::write(subagents.join(name), record(session, text)).unwrap();
            }
        }
        (root, parent)
    }

    #[test]
    fn a_session_and_its_subagents_are_one_ref_sized_as_a_group() {
        // 911 files describing 69 conversations is the bug. One card per
        // conversation is the fix, and its size has to describe everything
        // the card covers or eligibility is judging the wrong bytes.
        let (root, parent) = group_fixture(&[
            ("agent-a.jsonl", "a"),
            ("agent-b.jsonl", "b"),
            ("agent-c.jsonl", "c"),
        ]);
        let found = ClaudeCodeSource::new(root.path().to_path_buf())
            .discover()
            .unwrap();
        assert_eq!(found.len(), 1, "one ref per conversation: {found:?}");
        assert_eq!(found[0].path, parent);
        assert_eq!(found[0].group_member_count, 3);

        let subagents = parent.parent().unwrap().join(parent.file_stem().unwrap());
        let expected: u64 = ["agent-a.jsonl", "agent-b.jsonl", "agent-c.jsonl"]
            .iter()
            .map(|n| {
                std::fs::metadata(subagents.join("subagents").join(n))
                    .unwrap()
                    .len()
            })
            .sum::<u64>()
            + std::fs::metadata(&parent).unwrap().len();
        assert_eq!(
            found[0].size_bytes, expected,
            "size must cover all four files"
        );
        assert!(found[0].group_modified_at.is_some());
    }

    #[test]
    #[cfg(unix)]
    fn a_symlinked_member_contributes_nothing_while_the_parent_is_still_offered() {
        // The companion to `discovery_refuses_symlinks_that_escape_the_
        // transcript_root`, which has no parent files and so proves only
        // that nothing is discovered. Here the conversation IS discovered,
        // and the point is that the symlink adds no bytes to it.
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secrets.jsonl");
        std::fs::write(
            &secret,
            record("22222222-2222-2222-2222-222222222222", "private"),
        )
        .unwrap();

        let (root, parent) = group_fixture(&[("agent-a.jsonl", "a")]);
        let subagents = parent
            .parent()
            .unwrap()
            .join(parent.file_stem().unwrap())
            .join("subagents");
        symlink(&secret, subagents.join("agent-link.jsonl")).unwrap();

        let found = ClaudeCodeSource::new(root.path().to_path_buf())
            .discover()
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].group_member_count, 1,
            "the symlinked member must not join the group"
        );

        let t = ClaudeCodeSource::new(root.path().to_path_buf())
            .load(&found[0])
            .unwrap();
        assert_eq!(t.subagent_count, 1);
        let body = serde_json::to_string(
            &t.events
                .iter()
                .filter_map(|e| e.content.clone())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(!body.contains("private"), "symlink target must not be read");
    }

    #[test]
    fn load_appends_every_member_behind_its_own_boundary_marker() {
        let (root, parent) =
            group_fixture(&[("agent-a.jsonl", "alpha"), ("agent-b.jsonl", "beta")]);
        let src = ClaudeCodeSource::new(root.path().to_path_buf());
        let t = load_group(&parent, GROUP_RAW_BYTE_BUDGET).unwrap();
        let _ = src;

        let texts: Vec<String> = t.events.iter().filter_map(|e| e.content.clone()).collect();
        assert_eq!(
            texts,
            vec![
                "parent turn".to_string(),
                "alpha".to_string(),
                "beta".to_string()
            ],
            "parent first, then each member in name order"
        );
        let markers: Vec<&Value> = t
            .events
            .iter()
            .filter(|e| e.kind == SessionEventKind::Opaque)
            .map(|e| &e.structured)
            .collect();
        assert_eq!(markers.len(), 3, "one group header plus one per member");
        assert_eq!(markers[0]["record_type"], "subagent_group");
        assert_eq!(markers[0]["subagent_count"], 2);
        assert_eq!(markers[0]["subagents_dropped"], 0);
        assert_eq!(markers[1]["record_type"], "subagent_transcript");
        assert_eq!(markers[1]["index"], 0);
        assert_eq!(markers[2]["index"], 1);
        assert_eq!(t.subagent_count, 2);
        assert_eq!(t.subagents_dropped, 0);
        // Session metadata describes the conversation, not a delegate.
        assert_eq!(t.cwd.as_deref(), Some("/Users/testuser/code/myproj"));
    }

    #[test]
    fn the_group_hash_is_stable_and_independent_of_read_dir_order() {
        // `read_dir` order is unspecified and both `entry_id_for` and
        // `submission_id_for` derive from this hash, so an order-dependent
        // fold would re-offer the same conversation on every single poll.
        let (root, parent) = group_fixture(&[("agent-a.jsonl", "a"), ("agent-b.jsonl", "b")]);
        let first = load_group(&parent, GROUP_RAW_BYTE_BUDGET)
            .unwrap()
            .session_hash;
        let second = load_group(&parent, GROUP_RAW_BYTE_BUDGET)
            .unwrap()
            .session_hash;
        assert_eq!(first, second);
        drop(root);

        // Two loads agreeing only shows stability -- `read_dir` may well have
        // returned the same order both times. The property that actually
        // carries the guarantee is that `group_members_for` sorts, so assert
        // that directly: members created in an order that is NOT name order
        // still come back in name-byte order.
        let session = "22222222-2222-2222-2222-222222222222";
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("-Users-testuser-code-myproj");
        let subagents = project_dir.join(session).join("subagents");
        std::fs::create_dir_all(&subagents).unwrap();
        let parent = project_dir.join(format!("{session}.jsonl"));
        std::fs::write(&parent, record(session, "parent")).unwrap();
        for name in ["agent-z.jsonl", "agent-m.jsonl", "agent-a.jsonl"] {
            std::fs::write(subagents.join(name), record(session, name)).unwrap();
        }
        let (members, _) = group_members_for(&parent);
        let names: Vec<String> = members
            .iter()
            .map(|m| m.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["agent-a.jsonl", "agent-m.jsonl", "agent-z.jsonl"],
            "membership must be sorted regardless of creation or read_dir order"
        );

        // And the fold itself is order-SENSITIVE by design -- which is why
        // the sort above is what the determinism actually rests on. If
        // permuting the list did not move the hash, this test would prove
        // nothing about the sort.
        let parent_bytes = b"parent".to_vec();
        let a = b"aaa".to_vec();
        let b = b"bbb".to_vec();
        assert_ne!(
            group_session_hash(&parent_bytes, &[a.clone(), b.clone()], 0),
            group_session_hash(&parent_bytes, &[b, a], 0)
        );
    }

    #[test]
    fn any_membership_or_content_change_moves_the_group_hash() {
        // This is the consent invariant: an approval covers exactly the
        // bytes previewed, and the uploader's re-hash guard is what enforces
        // it. If a member could appear, vanish, or change without moving the
        // hash, an approval would ride along onto content it never covered.
        let (root, parent) = group_fixture(&[("agent-a.jsonl", "a")]);
        let subagents = parent
            .parent()
            .unwrap()
            .join(parent.file_stem().unwrap())
            .join("subagents");
        let base = load_group(&parent, GROUP_RAW_BYTE_BUDGET)
            .unwrap()
            .session_hash;

        let session = "22222222-2222-2222-2222-222222222222";
        std::fs::write(subagents.join("agent-b.jsonl"), record(session, "b")).unwrap();
        let added = load_group(&parent, GROUP_RAW_BYTE_BUDGET)
            .unwrap()
            .session_hash;
        assert_ne!(base, added, "a new member must move the hash");

        std::fs::write(subagents.join("agent-b.jsonl"), record(session, "b-edited")).unwrap();
        let edited = load_group(&parent, GROUP_RAW_BYTE_BUDGET)
            .unwrap()
            .session_hash;
        assert_ne!(added, edited, "editing a member must move the hash");

        std::fs::remove_file(subagents.join("agent-b.jsonl")).unwrap();
        let removed = load_group(&parent, GROUP_RAW_BYTE_BUDGET)
            .unwrap()
            .session_hash;
        assert_eq!(base, removed, "removing it must return to the same group");
        drop(root);
    }

    #[test]
    fn a_group_gets_a_different_entry_id_than_its_parent_file_alone() {
        use crate::daemon::queue::entry_id_for;
        let (root, parent) = group_fixture(&[("agent-a.jsonl", "a")]);
        let group = load_group(&parent, GROUP_RAW_BYTE_BUDGET)
            .unwrap()
            .session_hash;
        let parent_only = session_hash(&std::fs::read(&parent).unwrap());
        assert_ne!(group, parent_only);
        assert_ne!(entry_id_for(&group), entry_id_for(&parent_only));
        drop(root);
    }

    #[test]
    fn a_session_with_no_subagents_keeps_the_plain_file_hash() {
        // Adopting grouping must not churn the queue for the conversations
        // that never delegated anything: the bytes are the same, so the
        // identity is the same.
        let (root, parent) = group_fixture(&[]);
        let group = load_group(&parent, GROUP_RAW_BYTE_BUDGET)
            .unwrap()
            .session_hash;
        assert_eq!(group, session_hash(&std::fs::read(&parent).unwrap()));
        drop(root);
    }

    #[test]
    fn an_orphan_subagent_is_not_discovered_at_all() {
        // A fragment whose opening prompt was written by the parent agent
        // is worse than no trace: a contributor would read a machine's
        // instruction as their own message.
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("-Users-testuser-code-myproj");
        let session = "33333333-3333-3333-3333-333333333333";
        let subagents = project_dir.join(session).join("subagents");
        std::fs::create_dir_all(&subagents).unwrap();
        std::fs::write(subagents.join("agent-a.jsonl"), record(session, "orphan")).unwrap();

        let found = ClaudeCodeSource::new(root.path().to_path_buf())
            .discover()
            .unwrap();
        assert!(found.is_empty(), "orphan must not be queued: {found:?}");
    }

    #[test]
    fn a_member_whose_session_id_disagrees_is_excluded_and_not_queued_separately() {
        let (root, parent) = group_fixture(&[("agent-a.jsonl", "a")]);
        let subagents = parent
            .parent()
            .unwrap()
            .join(parent.file_stem().unwrap())
            .join("subagents");
        std::fs::write(
            subagents.join("agent-impostor.jsonl"),
            record("99999999-9999-9999-9999-999999999999", "impostor"),
        )
        .unwrap();

        let found = ClaudeCodeSource::new(root.path().to_path_buf())
            .discover()
            .unwrap();
        assert_eq!(found.len(), 1, "still one conversation: {found:?}");
        assert_eq!(found[0].group_member_count, 1, "impostor excluded");

        let t = load_group(&parent, GROUP_RAW_BYTE_BUDGET).unwrap();
        assert_eq!(t.subagent_count, 1);
        assert!(
            !t.events
                .iter()
                .any(|e| e.content.as_deref() == Some("impostor")),
            "an excluded member's content must not be loaded"
        );
    }

    #[test]
    fn a_member_with_no_session_id_is_included() {
        // Absence is not disagreement: the directory is what decides
        // membership, and `sessionId` only ever verifies it.
        let (root, parent) = group_fixture(&[("agent-a.jsonl", "a")]);
        let subagents = parent
            .parent()
            .unwrap()
            .join(parent.file_stem().unwrap())
            .join("subagents");
        std::fs::write(
            subagents.join("agent-quiet.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"quiet\"}}\n",
        )
        .unwrap();

        let t = load_group(&parent, GROUP_RAW_BYTE_BUDGET).unwrap();
        assert_eq!(t.subagent_count, 2);
        assert!(
            t.events
                .iter()
                .any(|e| e.content.as_deref() == Some("quiet"))
        );
        drop(root);
    }

    #[test]
    fn an_over_budget_group_drops_its_largest_members_deterministically() {
        // The alternative is a `Refused` envelope, which loses the whole
        // conversation rather than its tail -- and the drop has to be
        // identical on every load or the hash moves under an approval.
        let session = "22222222-2222-2222-2222-222222222222";
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("-Users-testuser-code-myproj");
        let subagents = project_dir.join(session).join("subagents");
        std::fs::create_dir_all(&subagents).unwrap();
        let parent = project_dir.join(format!("{session}.jsonl"));
        std::fs::write(&parent, record(session, "parent turn")).unwrap();
        // One deliberately huge member and two small ones.
        std::fs::write(
            subagents.join("agent-big.jsonl"),
            record(session, &"x".repeat(4000)),
        )
        .unwrap();
        std::fs::write(subagents.join("agent-s1.jsonl"), record(session, "s1")).unwrap();
        std::fs::write(subagents.join("agent-s2.jsonl"), record(session, "s2")).unwrap();

        let budget = 1_000_u64;
        let first = load_group(&parent, budget).unwrap();
        let second = load_group(&parent, budget).unwrap();
        assert_eq!(first.subagents_dropped, 1, "only the big one need go");
        assert_eq!(first.subagent_count, 2);
        assert_eq!(first.session_hash, second.session_hash, "must be identical");
        assert_eq!(first.subagents_dropped, second.subagents_dropped);
        assert!(
            !first
                .events
                .iter()
                .any(|e| e.content.as_deref().is_some_and(|c| c.len() > 3000)),
            "the dropped member's content must not be loaded"
        );
        // And the drop is stated rather than silent.
        let header = first
            .events
            .iter()
            .find(|e| e.structured.get("record_type") == Some(&json!("subagent_group")))
            .expect("group header");
        assert_eq!(header.structured["subagents_dropped"], 1);
    }

    #[test]
    fn a_group_trimmed_to_nothing_still_says_so() {
        // The edge the per-member markers alone would miss: if every member
        // is dropped, there is no member marker left to carry the count.
        let (root, parent) = group_fixture(&[("agent-a.jsonl", "a")]);
        let t = load_group(&parent, 1).unwrap();
        assert_eq!(t.subagent_count, 0);
        assert_eq!(t.subagents_dropped, 1);
        let header = t
            .events
            .iter()
            .find(|e| e.structured.get("record_type") == Some(&json!("subagent_group")))
            .expect("a trimmed group must still announce itself");
        assert_eq!(header.structured["subagents_dropped"], 1);
        drop(root);
    }

    #[test]
    fn the_source_applies_the_production_budget_by_default() {
        // `with_budget` exists for the tests above; production must not be
        // quietly running on a test value.
        let (root, _parent) = group_fixture(&[("agent-a.jsonl", "a")]);
        let src = ClaudeCodeSource::new(root.path().to_path_buf());
        assert_eq!(src.group_budget, GROUP_RAW_BYTE_BUDGET);
        let tight = ClaudeCodeSource::with_budget(root.path().to_path_buf(), 1);
        let r = &tight.discover().unwrap()[0];
        assert_eq!(tight.load(r).unwrap().subagents_dropped, 1);
        assert_eq!(src.load(r).unwrap().subagents_dropped, 0);
    }

    #[test]
    fn project_selection_still_reads_the_parents_cwd_not_a_members() {
        // `--project` and the daemon's per-project policy both key on the
        // ref's `cwd` (`discover_filtered`, `watcher::resolve_cwd`). A
        // delegated transcript recording some other directory must not be
        // able to move the conversation into a different project -- least of
        // all one the contributor has configured differently.
        let (root, parent) = group_fixture(&[("agent-a.jsonl", "a")]);
        let subagents = parent
            .parent()
            .unwrap()
            .join(parent.file_stem().unwrap())
            .join("subagents");
        std::fs::write(
            subagents.join("agent-elsewhere.jsonl"),
            "{\"type\":\"user\",\"cwd\":\"/Users/testuser/client/secret\",\
             \"sessionId\":\"22222222-2222-2222-2222-222222222222\",\
             \"message\":{\"role\":\"user\",\"content\":\"elsewhere\"}}\n",
        )
        .unwrap();

        let src = ClaudeCodeSource::new(root.path().to_path_buf());
        let found = src.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].cwd.as_deref(), Some("/Users/testuser/code/myproj"));

        let t = src.load(&found[0]).unwrap();
        assert_eq!(t.cwd.as_deref(), Some("/Users/testuser/code/myproj"));
        assert_eq!(t.project.as_deref(), Some("myproj"));
        assert_eq!(t.subagent_count, 2, "the member is still merged in");
    }

    #[test]
    fn peek_session_id_reads_the_head_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.jsonl");
        std::fs::write(&path, "{\"type\":\"user\",\"sessionId\":\"abc\"}\n").unwrap();
        assert_eq!(peek_session_id(&path), Some("abc".to_string()));

        let none = dir.path().join("n.jsonl");
        std::fs::write(&none, "{\"type\":\"user\"}\n").unwrap();
        assert_eq!(peek_session_id(&none), None);

        // A first line longer than the cap is truncated, so it fails to
        // parse and is skipped rather than yielding a mangled id.
        let huge = dir.path().join("h.jsonl");
        let mut body = String::from("{\"pad\":\"");
        body.push_str(&"p".repeat(SESSION_ID_PEEK_BYTES as usize));
        body.push_str("\",\"sessionId\":\"late\"}\n");
        std::fs::write(&huge, body).unwrap();
        assert_eq!(peek_session_id(&huge), None);
    }

    #[test]
    fn a_member_peek_is_memoized_until_its_size_or_mtime_moves() {
        // Discovery verifies every member on every pass, and on the tree
        // this adapter was written for that was 842 opens a minute. The memo
        // is what makes a pass cost a stat per member instead of a read.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-memo.jsonl");
        std::fs::write(&path, "{\"type\":\"user\",\"sessionId\":\"aaa\"}\n").unwrap();
        let md = std::fs::metadata(&path).unwrap();
        let size = md.len();
        let mtime = md
            .modified()
            .ok()
            .map(chrono::DateTime::<chrono::Utc>::from);
        assert_eq!(
            peek_session_id_memoized(&path, size, mtime),
            Some("aaa".to_string())
        );

        // Same length, different id. Passing the original (size, mtime) is
        // the memo's key, so the answer must come back from the memo rather
        // than from the file -- that is the whole saving, stated as a fact
        // rather than as a timing.
        std::fs::write(&path, "{\"type\":\"user\",\"sessionId\":\"bbb\"}\n").unwrap();
        assert_eq!(
            peek_session_id_memoized(&path, size, mtime),
            Some("aaa".to_string()),
            "an unchanged (size, mtime) must not re-read the file"
        );

        // And a file that did change is re-peeked, so a member rewritten
        // under a different sessionId stops being trusted.
        let moved = mtime.map(|t| t + chrono::Duration::seconds(1));
        assert_eq!(
            peek_session_id_memoized(&path, size, moved),
            Some("bbb".to_string())
        );
    }

    #[test]
    #[cfg(unix)]
    fn the_budget_is_decided_before_any_member_is_read() {
        // The budget has to bound what is READ, not merely what is kept: a
        // `subagents/` directory holding a gigabyte must not become resident
        // in the daemon on its way to being discarded.
        //
        // Made observable by an oversized member the process cannot open. If
        // the drop were decided after reading, the unreadable member would
        // fall out as unreadable and the group would report nothing dropped;
        // deciding from the stat'd size reports the drop it actually made.
        // (Running as root defeats the permission bit, in which case this
        // asserts the same outcome for the ordinary reason.)
        use std::os::unix::fs::PermissionsExt;

        let session = "22222222-2222-2222-2222-222222222222";
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("-Users-testuser-code-myproj");
        let subagents = project_dir.join(session).join("subagents");
        std::fs::create_dir_all(&subagents).unwrap();
        let parent = project_dir.join(format!("{session}.jsonl"));
        std::fs::write(&parent, record(session, "parent turn")).unwrap();
        std::fs::write(subagents.join("agent-s1.jsonl"), record(session, "s1")).unwrap();
        std::fs::write(subagents.join("agent-s2.jsonl"), record(session, "s2")).unwrap();
        let big = subagents.join("agent-big.jsonl");
        std::fs::write(&big, record(session, &"x".repeat(4000))).unwrap();
        // Membership itself is unaffected either way -- an unreadable head
        // peeks as "no sessionId", and absence is not disagreement -- but
        // assert it here so a failure below is unambiguously about the
        // budget and not about the member having fallen out of the group.
        let (members, _) = group_members_for(&parent);
        assert_eq!(members.len(), 3);
        std::fs::set_permissions(&big, std::fs::Permissions::from_mode(0o000)).unwrap();

        let t = load_group(&parent, 1_000).unwrap();
        assert_eq!(
            t.subagents_dropped, 1,
            "the oversized member is dropped on its stat'd size, never opened"
        );
        assert_eq!(t.subagent_count, 2);

        // Leave the fixture deletable.
        std::fs::set_permissions(&big, std::fs::Permissions::from_mode(0o644)).unwrap();
        drop(root);
    }

    #[test]
    fn renaming_a_member_moves_the_hash_only_when_sort_order_moves() {
        // `group_session_hash` folds member CONTENTS and never member names,
        // which reads like a gap until you ask what the contributor was
        // shown. Names never reach them: the boundary marker carries only a
        // sorted `index`, and the bytes are the file's contents. So a rename
        // that leaves sort order alone changes nothing about the previewed
        // conversation and must NOT invalidate an approval -- a spurious
        // re-ask is its own consent failure -- while a rename that reorders
        // the fold changes the bytes and must.
        let (root, parent) = group_fixture(&[
            ("agent-a.jsonl", "alpha"),
            ("agent-m.jsonl", "middle"),
            ("agent-z.jsonl", "omega"),
        ]);
        let subagents = parent
            .parent()
            .unwrap()
            .join(parent.file_stem().unwrap())
            .join("subagents");
        let base = load_group(&parent, GROUP_RAW_BYTE_BUDGET)
            .unwrap()
            .session_hash;

        // Still sorts between `agent-a` and `agent-z`, so the concatenation
        // and every index is byte-identical.
        std::fs::rename(
            subagents.join("agent-m.jsonl"),
            subagents.join("agent-n.jsonl"),
        )
        .unwrap();
        let in_place = load_group(&parent, GROUP_RAW_BYTE_BUDGET)
            .unwrap()
            .session_hash;
        assert_eq!(
            base, in_place,
            "a rename the contributor cannot see must not invalidate their approval"
        );

        // Now move the same member to the end of the order. The fold is
        // order-sensitive, so the hash moves and the uploader's re-hash
        // guard refuses the stale approval.
        std::fs::rename(
            subagents.join("agent-n.jsonl"),
            subagents.join("agent-zz.jsonl"),
        )
        .unwrap();
        let reordered = load_group(&parent, GROUP_RAW_BYTE_BUDGET)
            .unwrap()
            .session_hash;
        assert_ne!(
            base, reordered,
            "reordering the members reorders the bytes, so the hash must move"
        );
        drop(root);
    }

    #[test]
    fn an_unrelated_nested_directory_is_never_swept_in() {
        // Only `<project-dir>/<session-uuid>/subagents/*.jsonl` is a known
        // layout. Everything discovery returns is a candidate for upload, so
        // a sibling directory holding a `.jsonl` must contribute nothing --
        // and its file must not become a session of its own either.
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("-Users-testuser-code-myproj");
        let session = "22222222-2222-2222-2222-222222222222";
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join(format!("{session}.jsonl")),
            record(session, "hi"),
        )
        .unwrap();

        let subagents = project_dir.join(session).join("subagents");
        std::fs::create_dir_all(&subagents).unwrap();
        std::fs::write(
            subagents.join("agent-abc123.jsonl"),
            record(session, "sub hi"),
        )
        .unwrap();

        let unrelated = project_dir.join(session).join("scratch");
        std::fs::create_dir_all(&unrelated).unwrap();
        std::fs::write(
            unrelated.join("not-a-session.jsonl"),
            record(session, "nope"),
        )
        .unwrap();

        let src = ClaudeCodeSource::new(root.path().to_path_buf());
        let found = src.discover().unwrap();
        assert_eq!(found.len(), 1, "one conversation, not three: {found:?}");
        assert!(found[0].path.ends_with(format!("{session}.jsonl")));
        assert_eq!(
            found[0].group_member_count, 1,
            "only the subagents dir counts"
        );
        assert_eq!(found[0].cwd.as_deref(), Some("/Users/testuser/code/myproj"));

        let t = src.load(&found[0]).unwrap();
        assert!(
            !t.events
                .iter()
                .any(|e| e.content.as_deref() == Some("nope")),
            "an unrelated nested directory was swept into the group"
        );
    }

    #[test]
    fn discovers_fixture_session() {
        let src = ClaudeCodeSource::new(fixture_root());
        let found = src.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, "claude-code");
        // Fixture dir is `-Users-testuser-code-myproj`; the discovery-time
        // heuristic takes the segment after the final '-' as a best-effort
        // project basename, ahead of `load()` reading the true cwd.
        assert_eq!(found[0].project, Some("myproj".to_string()));
        // Discovery now peeks the true cwd cheaply too, matching `load()`.
        assert_eq!(found[0].cwd.as_deref(), Some("/Users/testuser/code/myproj"));
    }

    #[test]
    fn peek_cwd_reads_first_hit_and_round_trips_hyphenated_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"user\",\"cwd\":\"/Users/dev/code/my-hack\"}\n{\"type\":\"assistant\"}\n",
        )
        .unwrap();
        assert_eq!(peek_cwd(&path), Some("/Users/dev/code/my-hack".to_string()));
    }

    #[test]
    fn peek_cwd_returns_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        std::fs::write(&path, "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n").unwrap();
        assert_eq!(peek_cwd(&path), None);
    }

    #[test]
    fn peek_cwd_tolerates_invalid_utf8_before_the_cwd_line() {
        // A malformed-UTF-8 line ahead of the cwd-bearing line must not abort
        // the scan: `load_session` reads via `String::from_utf8_lossy` and
        // keeps going past bad bytes, so `peek_cwd` must match exactly or
        // `--project` filtering can silently disagree with what
        // `submit_sessions` actually delivers.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(b"{\"type\":\"user\",\"bad\":\"");
        bytes.extend_from_slice(&[0xFF, 0xFE]); // invalid UTF-8 sequence
        bytes.extend_from_slice(b"not json}\n");
        bytes.extend_from_slice(b"{\"type\":\"user\",\"cwd\":\"/Users/dev/code/my-hack\"}\n");
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(peek_cwd(&path), Some("/Users/dev/code/my-hack".to_string()));
    }

    #[test]
    fn interleaved_text_blocks_keep_their_position() {
        // A single assistant record can interleave prose with reasoning and
        // tool calls. Merging every text block and hoisting it to the first
        // text position reorders the transcript: text written AFTER the model
        // finished thinking would appear before the reasoning that produced
        // it. That misrepresents what happened, which is precisely what makes
        // the trace worth collecting.
        let record = serde_json::json!({
            "message": {
                "content": [
                    { "type": "text", "text": "First." },
                    { "type": "thinking", "thinking": "considering options" },
                    { "type": "text", "text": "Second." },
                    { "type": "tool_use", "name": "Read", "input": {"path": "a"} },
                    { "type": "text", "text": "Third." }
                ]
            }
        });
        let mut events = Vec::new();
        super::map_assistant_record(&record, None, &mut events);

        let kinds: Vec<_> = events.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                SessionEventKind::Assistant,
                SessionEventKind::Reasoning,
                SessionEventKind::Assistant,
                SessionEventKind::ToolCall,
                SessionEventKind::Assistant,
            ],
            "block order must be preserved"
        );
        assert_eq!(events[0].content.as_deref(), Some("First."));
        assert_eq!(events[2].content.as_deref(), Some("Second."));
        assert_eq!(events[4].content.as_deref(), Some("Third."));
    }

    #[test]
    fn contiguous_text_blocks_are_joined() {
        // Adjacent text blocks are a rendering artifact, not separate turns.
        let record = serde_json::json!({
            "message": {
                "content": [
                    { "type": "text", "text": "One." },
                    { "type": "text", "text": "Two." },
                    { "type": "tool_use", "name": "Read", "input": {} }
                ]
            }
        });
        let mut events = Vec::new();
        super::map_assistant_record(&record, None, &mut events);
        let kinds: Vec<_> = events.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![SessionEventKind::Assistant, SessionEventKind::ToolCall]
        );
        assert_eq!(events[0].content.as_deref(), Some("One.\nTwo."));
    }

    #[test]
    fn thinking_blocks_become_reasoning_events() {
        let record = serde_json::json!({
            "message": {
                "content": [
                    { "type": "thinking", "thinking": "the user wants X, so I should Y" },
                    { "type": "text", "text": "Here is the answer." }
                ]
            }
        });
        let mut events = Vec::new();
        super::map_assistant_record(&record, None, &mut events);

        let kinds: Vec<_> = events.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![SessionEventKind::Reasoning, SessionEventKind::Assistant]
        );
        assert_eq!(
            events[0].content.as_deref(),
            Some("the user wants X, so I should Y")
        );
    }

    #[test]
    fn loads_and_maps_events_leniently() {
        let src = ClaudeCodeSource::new(fixture_root());
        let r = &src.discover().unwrap()[0];
        let t = src.load(r).unwrap();
        assert_eq!(t.model.as_deref(), Some("claude-fable-5"));
        assert_eq!(t.agent_version.as_deref(), Some("2.0.1"));
        assert_eq!(t.cwd.as_deref(), Some("/Users/testuser/code/myproj"));
        assert_eq!(t.project.as_deref(), Some("myproj"));
        let kinds: Vec<_> = t.events.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                SessionEventKind::User,
                SessionEventKind::Reasoning,
                SessionEventKind::Assistant,
                SessionEventKind::ToolCall,
                SessionEventKind::ToolResult,
                SessionEventKind::Assistant,
                SessionEventKind::Opaque, // system
                SessionEventKind::Opaque, // attachment
                SessionEventKind::Opaque, // future-unknown-record
            ]
        );
        // Token counts captured on the assistant text event.
        assert_eq!(t.events[2].token_counts, Some((100, 25)));
        assert_eq!(t.events[3].tool_name.as_deref(), Some("Read"));
        // Opaque events carry only the record type, never payloads.
        let serialized = serde_json::to_string(&t.events[7].structured).unwrap();
        assert!(!serialized.contains("do not leak me"));
        assert!(serialized.contains("attachment"));
        assert!(
            t.events
                .iter()
                .any(|e| e.kind == SessionEventKind::Reasoning
                    && e.content.as_deref() == Some("secret reasoning")),
            "reasoning is now captured as a first-class event"
        );
    }

    /// The two things a price needs beyond the token counts: which model
    /// served the step, and how the cached tokens split across the two cache
    /// durations. Both come from the fixture record, which states them the
    /// way a real Claude Code transcript does.
    #[test]
    fn the_serving_model_and_cache_split_are_captured() {
        let src = ClaudeCodeSource::new(fixture_root());
        let r = &src.discover().unwrap()[0];
        let t = src.load(r).unwrap();
        assert_eq!(
            t.events[2].served_by,
            Some(crate::source::ServedBy {
                model: "claude-fable-5".to_string(),
                cache_read_tokens: 1000,
                cache_write_5m_tokens: 200,
                cache_write_1h_tokens: 300,
            })
        );
    }

    /// The second assistant record in the fixture reports only input and
    /// output tokens -- no cache report at all, the shape an older transcript
    /// has. Its tokens are still captured; it simply goes unpriced, rather
    /// than being priced as though nothing was cached.
    #[test]
    fn a_record_with_no_cache_report_keeps_its_tokens_and_stays_unpriced() {
        let src = ClaudeCodeSource::new(fixture_root());
        let r = &src.discover().unwrap()[0];
        let t = src.load(r).unwrap();
        assert_eq!(t.events[5].token_counts, Some((150, 12)));
        assert_eq!(t.events[5].served_by, None);
    }

    fn priceable_usage() -> Value {
        json!({
            "input_tokens": 10,
            "output_tokens": 20,
            "cache_read_input_tokens": 30,
            "cache_creation_input_tokens": 0,
        })
    }

    fn record_with(usage: Value) -> Value {
        json!({ "message": { "model": "claude-opus-5", "usage": usage } })
    }

    /// The baseline every case below mutates away from. Without this
    /// assertion a `None` result would prove nothing -- it could mean the
    /// helper never returns anything at all.
    #[test]
    fn a_complete_usage_report_is_read() {
        let record = record_with(priceable_usage());
        assert_eq!(
            served_by_of(&record, record.pointer("/message/usage").unwrap()),
            Some(crate::source::ServedBy {
                model: "claude-opus-5".to_string(),
                cache_read_tokens: 30,
                cache_write_5m_tokens: 0,
                cache_write_1h_tokens: 0,
            })
        );
    }

    /// Each of these is a record this crate must refuse to price. The list is
    /// the specification: a count the record did not state (which must not
    /// read as zero), a cache-creation total with no duration breakdown or
    /// one that does not reconcile, and any pricing modifier the price table
    /// does not model.
    #[test]
    fn an_incomplete_or_modified_usage_report_is_refused() {
        let mut cases: Vec<(&str, Value)> = Vec::new();

        for missing in [
            "input_tokens",
            "output_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ] {
            let mut usage = priceable_usage();
            usage.as_object_mut().unwrap().remove(missing);
            cases.push((missing, usage));
        }

        let mut no_split = priceable_usage();
        no_split["cache_creation_input_tokens"] = json!(500);
        cases.push(("cache creation with no 5m/1h split", no_split));

        let mut bad_split = priceable_usage();
        bad_split["cache_creation_input_tokens"] = json!(500);
        bad_split["cache_creation"] = json!({
            "ephemeral_5m_input_tokens": 200,
            "ephemeral_1h_input_tokens": 299,
        });
        cases.push(("a split that does not add up", bad_split));

        let mut half_split = priceable_usage();
        half_split["cache_creation_input_tokens"] = json!(500);
        half_split["cache_creation"] = json!({ "ephemeral_5m_input_tokens": 500 });
        cases.push(("a split missing the 1h half", half_split));

        for (label, value) in [
            ("service_tier", json!("priority")),
            ("speed", json!("fast")),
            ("inference_geo", json!("us")),
        ] {
            let mut usage = priceable_usage();
            usage[label] = value;
            cases.push((label, usage));
        }

        let mut too_large = priceable_usage();
        too_large["input_tokens"] = json!(u64::from(u32::MAX) + 1);
        cases.push(("a count wider than u32", too_large));

        for (label, usage) in cases {
            let record = record_with(usage);
            assert_eq!(
                served_by_of(&record, record.pointer("/message/usage").unwrap()),
                None,
                "must refuse to price: {label}"
            );
        }
    }

    /// The modifiers that leave standard pricing in force are read as such,
    /// so the refusals above are refusing something real rather than
    /// refusing every record that carries the fields at all.
    #[test]
    fn standard_pricing_modifiers_are_not_refused() {
        for (label, value) in [
            ("service_tier", json!("standard")),
            ("speed", json!("standard")),
            ("inference_geo", json!("global")),
            ("inference_geo", json!("not_available")),
        ] {
            let mut usage = priceable_usage();
            usage[label] = value.clone();
            let record = record_with(usage);
            assert!(
                served_by_of(&record, record.pointer("/message/usage").unwrap()).is_some(),
                "{label}={value} is standard pricing and must still be read"
            );
        }
    }

    #[test]
    fn a_record_that_does_not_name_its_model_is_refused() {
        for message in [
            json!({ "usage": priceable_usage() }),
            json!({ "model": "", "usage": priceable_usage() }),
            json!({ "model": 7, "usage": priceable_usage() }),
        ] {
            let record = json!({ "message": message });
            assert_eq!(
                served_by_of(&record, record.pointer("/message/usage").unwrap()),
                None
            );
        }
    }

    /// A turn made entirely of tool calls emits no assistant text event, and
    /// its usage used to be dropped with it. Those turns are most of a coding
    /// session, so a session total that skipped them was not a total.
    #[test]
    fn a_tool_only_turn_keeps_its_usage() {
        let record = json!({
            "message": {
                "model": "claude-opus-5",
                "usage": priceable_usage(),
                "content": [
                    { "type": "tool_use", "id": "tu_9", "name": "Read", "input": {} },
                ],
            }
        });
        let mut events = Vec::new();
        map_assistant_record(&record, None, &mut events);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, SessionEventKind::ToolCall);
        assert_eq!(events[0].token_counts, Some((10, 20)));
        assert!(events[0].served_by.is_some());
    }

    /// One record, one usage report: attaching it to more than one event
    /// would double-count both the tokens and the price.
    #[test]
    fn usage_is_attached_exactly_once_per_record() {
        let record = json!({
            "message": {
                "model": "claude-opus-5",
                "usage": priceable_usage(),
                "content": [
                    { "type": "text", "text": "first" },
                    { "type": "tool_use", "id": "tu_1", "name": "Read", "input": {} },
                    { "type": "text", "text": "second" },
                ],
            }
        });
        let mut events = Vec::new();
        map_assistant_record(&record, None, &mut events);
        assert_eq!(events.len(), 3);
        assert_eq!(
            events.iter().filter(|e| e.token_counts.is_some()).count(),
            1
        );
        assert_eq!(events.iter().filter(|e| e.served_by.is_some()).count(), 1);
        // The text event, not the tool call: unchanged from before there was
        // a price to attach.
        assert_eq!(events[0].token_counts, Some((10, 20)));
    }

    #[test]
    fn a_call_and_its_result_carry_the_same_id() {
        // The transcript names both halves of a call with `tu_1`, and this
        // adapter read the block and dropped the id, leaving array order as
        // the only way to tell which result answered which call.
        let src = ClaudeCodeSource::new(fixture_root());
        let r = &src.discover().unwrap()[0];
        let t = src.load(r).unwrap();
        let call = t
            .events
            .iter()
            .find(|e| e.kind == SessionEventKind::ToolCall)
            .expect("a tool call");
        let result = t
            .events
            .iter()
            .find(|e| e.kind == SessionEventKind::ToolResult)
            .expect("a tool result");
        assert_eq!(call.tool_call_id.as_deref(), Some("tu_1"));
        assert_eq!(result.tool_call_id.as_deref(), Some("tu_1"));
    }

    #[test]
    fn an_errored_tool_result_says_so() {
        // `is_error` is the transcript's own verdict, and it was read by
        // nothing. It is the cheapest signal there is for the failed traces
        // that reported no error at all.
        let record = json!({
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "tu_9",
                 "is_error": true, "content": "permission denied"}
            ]}
        });
        let mut events = Vec::new();
        super::map_user_record(&record, None, &mut events);
        assert_eq!(events[0].success, Some(false));
        assert_eq!(events[0].tool_call_id.as_deref(), Some("tu_9"));
    }

    #[test]
    fn a_tool_result_with_no_verdict_claims_none() {
        // Absent `is_error`, the harness did not say. `None` is not success
        // and it is not failure.
        let record = json!({
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "tu_9", "content": "ok"}
            ]}
        });
        let mut events = Vec::new();
        super::map_user_record(&record, None, &mut events);
        assert_eq!(events[0].success, None);
    }
}
