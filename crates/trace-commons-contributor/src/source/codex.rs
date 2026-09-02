//! Codex rollout transcript adapter.
//!
//! Reads `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`
//! session files and maps them into the shared `SessionTranscript` model.
//! See `docs/superpowers/plans/` (Task 8) for the format facts and mapping
//! rules; `Opaque` events (covering `event_msg`, `web_search_call`, and any
//! unknown payload/record type) carry only a record-type marker, never a
//! payload. `reasoning` items are captured as `Reasoning` events.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use serde_json::{Value, json};

use super::{
    SOURCE_CODEX, SessionEvent, SessionEventKind, SessionHasher, SessionRef, SessionTranscript,
    TraceSource, real_file_within_root,
};

pub struct CodexSource {
    root: PathBuf,
}

impl CodexSource {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl TraceSource for CodexSource {
    fn name(&self) -> &'static str {
        SOURCE_CODEX
    }

    fn discover(&self) -> anyhow::Result<Vec<SessionRef>> {
        let mut sessions = Vec::new();
        let mut skipped = 0usize;
        collect_rollout_files(&self.root, &mut sessions, &mut skipped);
        if skipped > 0 {
            tracing::warn!(
                skipped,
                "skipped unreadable codex session entries during discovery"
            );
        }
        Ok(sessions)
    }

    fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript> {
        load_session(&r.path)
    }

    /// A changed rollout file is its own session.
    ///
    /// Codex rollouts are peer files with no parent/child convention -- the
    /// same fact `SessionRef::group_member_count` records as zero -- so the
    /// mapping is the identity for anything `discover` would have collected
    /// and `None` for everything else. `is_rollout_file_name` is the one
    /// naming rule, shared with `collect_rollout_files`, so a file the walk
    /// would pass over cannot be addressed here.
    fn session_for_path(&self, path: &Path) -> Option<PathBuf> {
        let path = real_file_within_root(&self.root, path)?;
        // Stricter than the walk in one respect, deliberately:
        // `collect_rollout_files` reaches a symlinked *file* whose name
        // matches, because `DirEntry::file_type` only stops it descending
        // into symlinked directories. Refusing one here costs nothing --
        // the reconciliation sweep still finds it -- while keeping this
        // addressing surface uniform with the Claude Code one.
        is_rollout_file_name(path.file_name()?.to_str()?).then_some(path)
    }

    /// The ref for whichever rollout a changed path names.
    ///
    /// `session_for_path` resolves the address and `rollout_session_ref`
    /// describes it -- the same function `collect_rollout_files` builds
    /// every discovered ref with, so a scoped scan and a full sweep cannot
    /// disagree about a rollout's size or cwd. A rollout deleted between
    /// the event and this lookup is `Ok(None)`.
    fn session_at(&self, path: &Path) -> anyhow::Result<Option<SessionRef>> {
        let Some(address) = self.session_for_path(path) else {
            return Ok(None);
        };
        let mut ignored_skips = 0usize;
        Ok(rollout_session_ref(address, &mut ignored_skips))
    }
}

/// The rollout naming rule, in one place: `rollout-<...>.jsonl`.
fn is_rollout_file_name(file_name: &str) -> bool {
    file_name.starts_with("rollout-") && file_name.ends_with(".jsonl")
}

fn collect_rollout_files(dir: &Path, sessions: &mut Vec<SessionRef>, skipped: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                *skipped += 1;
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => {
                *skipped += 1;
                continue;
            }
        };
        if file_type.is_dir() {
            collect_rollout_files(&path, sessions, skipped);
            continue;
        }
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if !is_rollout_file_name(file_name) {
            continue;
        }
        if let Some(session) = rollout_session_ref(path, skipped) {
            sessions.push(session);
        }
    }
}

/// The one way a Codex `SessionRef` is built, used by `collect_rollout_files`
/// for every rollout it walks to and by `session_at` for the single rollout
/// an event named.
///
/// Shared rather than reimplemented because a scoped scan and a full sweep
/// that described the same session differently would reach different
/// eligibility decisions for the same bytes.
///
/// `None` for a file that is no longer there, which on the event path is an
/// ordinary race rather than a failure. `skipped` counts the entries that
/// were unreadable rather than merely gone.
fn rollout_session_ref(path: PathBuf, skipped: &mut usize) -> Option<SessionRef> {
    let metadata = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => {
            *skipped += 1;
            return None;
        }
    };
    let started_at = metadata
        .modified()
        .ok()
        .map(chrono::DateTime::<chrono::Utc>::from);
    let cwd = peek_cwd_memoized(&path, metadata.len(), started_at);
    // Derive the label from the cwd we just peeked, the same way
    // `load_session` does further down. Leaving this `None` meant every
    // Codex row in the picker rendered as `-`, so a contributor choosing
    // what to submit could not tell one session from another - while the
    // submitted envelope carried the correct project all along, because
    // load_session computes it. `--project` filtering was unaffected too,
    // since that matches on `cwd`. Only the thing a human reads was wrong.
    let project = cwd
        .as_deref()
        .map(Path::new)
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());
    Some(SessionRef {
        source: SOURCE_CODEX,
        declared_source: None,
        path,
        project,
        cwd,
        started_at,
        size_bytes: metadata.len(),
        // Codex rollouts are peer files: `collect_rollout_files` recurses
        // arbitrarily deep, but every match is its own session and no
        // record links one to another. There is no group to describe.
        group_modified_at: None,
        group_member_count: 0,
    })
}

/// Cheap discovery-time peek at a session file's true working directory:
/// parses each line as JSON in turn and stops at the first `session_meta`
/// record carrying a `payload.cwd` field, skipping the full parse of the
/// file's events. Tolerates invalid UTF-8 the same way `load_session` does
/// -- both convert one line at a time with `String::from_utf8_lossy`, so an
/// unreadable line elsewhere in the file does not abort the scan before it
/// reaches a later cwd-bearing line. `load_session` never errors on bad
/// UTF-8 either, so peek and load must tolerate it identically, or
/// `--project` filtering can silently disagree with what `submit_sessions`
/// actually delivers.
/// Mirrors the exact field path `load_session` uses
/// (`payload.and_then(|p| p.get("cwd"))`). Returns `None` if the file
/// cannot be read or no record carries `cwd`.
///
/// Cost: one line at a time, stopping at the answer. `session_meta` is the
/// first record a rollout writes, so in practice this reads a few hundred
/// bytes of a file that may be hundreds of megabytes. It used to
/// `std::fs::read` the whole file and then allocate a second copy through
/// `String::from_utf8_lossy` -- on a real machine, 11.5GB of Codex rollouts
/// read end to end on every watcher tick to learn what line one already
/// said. Prefer `peek_cwd_memoized` from discovery; this is the uncached
/// read behind it.
fn peek_cwd(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        // `read_until` rather than `lines()`, and a lossy conversion per
        // line rather than over the file: `lines()` fails the whole
        // iteration on invalid UTF-8, where the old whole-file
        // `from_utf8_lossy` tolerated it. A session whose later records are
        // unreadable must still report its cwd, or `--project` filtering
        // silently disagrees with what `submit_sessions` delivers.
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
        if record.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
            continue;
        }
        if let Some(c) = record
            .get("payload")
            .and_then(|p| p.get("cwd"))
            .and_then(|v| v.as_str())
        {
            return Some(c.to_string());
        }
    }
}

/// How many memoized cwd answers to hold before dropping the lot.
///
/// Mirrors `claude_code::SESSION_ID_MEMO_CAP`, and for the same reason: a
/// bound that a real corpus does not reach, so the memo does not grow
/// without limit on a machine that accumulates sessions for years. Clearing
/// wholesale rather than evicting one entry costs one discovery pass paying
/// what every pass used to pay.
const CWD_MEMO_CAP: usize = 8192;

/// One memoized answer from `peek_cwd`, valid while the file it describes
/// still reports the same size and mtime.
#[derive(Debug, Clone)]
struct CwdMemo {
    size_bytes: u64,
    modified_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Memoized even when absent: a rollout carrying no `session_meta` is
    /// the expensive case, because establishing that means reading it to
    /// the end. Re-deriving that absence every tick is the worst thing this
    /// memo can prevent.
    cwd: Option<String>,
}

/// Process-wide memo for `peek_cwd`, keyed on the rollout's path.
///
/// `discover` runs on every watcher tick, and it peeks every rollout. This
/// mirrors `claude_code::SESSION_ID_MEMO` exactly, including why it lives at
/// module scope: `watcher::tick_blocking` builds its sources fresh each
/// tick, so a memo owned by the adapter would be discarded before the pass
/// that needs it.
///
/// Keying on (size, mtime) assumes only what `watcher::resolve_cwd`'s own
/// cwd cache already assumes -- that a file whose size and mtime are
/// unchanged has unchanged contents. That is not a trust boundary: the cwd
/// decides which project a session is filed under, and anyone able to
/// backdate an mtime can equally well write the cwd they want into the
/// file, so the memo grants no capability the unmemoized path withheld.
static CWD_MEMO: LazyLock<Mutex<HashMap<PathBuf, CwdMemo>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// `peek_cwd`, answered from `CWD_MEMO` when the file still reports the size
/// and mtime the memoized answer was derived from.
fn peek_cwd_memoized(
    path: &Path,
    size_bytes: u64,
    modified_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Option<String> {
    // A poisoned memo is a cache, not state anything depends on: fall
    // through to the read rather than propagating another thread's panic
    // into discovery.
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

/// The largest rollout this adapter will load, mirroring
/// `claude_code::GROUP_RAW_BYTE_BUDGET` and set to the same 64 MB.
///
/// Streaming stopped the loader holding two copies of a file, but the events
/// it builds are still proportional to what it parses, and a rollout has no
/// upper bound: the machine this was measured on had a 385 MB one. The
/// Claude adapter has bounded its equivalent since it was written; Codex
/// bounding nothing was an asymmetry rather than a decision.
///
/// One number, not two, because these bound the same thing -- how much of
/// one conversation may become resident on its way to being discarded.
///
/// On the measured corpus this declines 10 of 3,066 rollouts (0.3%). It is a
/// guard against pathology, not a filter: the median rollout is 541 KB and
/// the 99th percentile is 40 MB.
const ROLLOUT_BYTE_BUDGET: u64 = super::claude_code::GROUP_RAW_BYTE_BUDGET;

fn load_session(path: &Path) -> anyhow::Result<SessionTranscript> {
    // Declined rather than truncated, and named rather than silent. A
    // half-parsed transcript would upload as though it were the whole
    // conversation, which is worse than not offering it: the contributor
    // would be consenting to something the preview misdescribes. The size is
    // the contributor's own file's, not operator-secret, so it is safe to
    // state -- but the path is not, and is deliberately absent.
    //
    // Typed, not a bare `bail!`, so a caller can tell this refusal apart
    // from the IO errors around it without matching on message text. The
    // rendered message is unchanged. See `source::SessionTooLarge`.
    let declared = std::fs::metadata(path)?.len();
    if declared > ROLLOUT_BYTE_BUDGET {
        return Err(super::SessionTooLarge {
            label: "rollout-too-large",
            declared_bytes: declared,
            budget_bytes: ROLLOUT_BYTE_BUDGET,
        }
        .into());
    }
    // Streamed rather than read whole. A rollout can be hundreds of
    // megabytes, and the old `fs::read` plus `from_utf8_lossy` held two
    // copies of it at once before a single event was built. The hash is
    // accumulated over the same bytes in the same order, so the session id
    // is unchanged -- see `SessionHasher`.
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = SessionHasher::new();
    let mut raw = Vec::new();

    let mut events = Vec::new();
    let mut model: Option<String> = None;
    let mut agent_version: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut started_at: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut unparseable = 0usize;

    loop {
        raw.clear();
        if reader.read_until(b'\n', &mut raw)? == 0 {
            break;
        }
        hasher.update(&raw);
        let line = String::from_utf8_lossy(&raw);
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

        let record_type = record.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let payload = record.get("payload");

        match record_type {
            "session_meta" => {
                if cwd.is_none() {
                    if let Some(c) = payload.and_then(|p| p.get("cwd")).and_then(|v| v.as_str()) {
                        cwd = Some(c.to_string());
                    }
                }
                if agent_version.is_none() {
                    if let Some(v) = payload
                        .and_then(|p| p.get("cli_version"))
                        .and_then(|v| v.as_str())
                    {
                        agent_version = Some(v.to_string());
                    }
                }
            }
            "turn_context" => {
                if model.is_none() {
                    if let Some(m) = payload
                        .and_then(|p| p.get("model"))
                        .and_then(|v| v.as_str())
                    {
                        model = Some(m.to_string());
                    }
                }
            }
            "response_item" => {
                map_response_item(payload, record_timestamp, &mut events);
            }
            other => {
                events.push(SessionEvent {
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

    if unparseable > 0 {
        tracing::warn!(unparseable, "skipped unparseable Codex record lines");
    }

    let project = cwd
        .as_deref()
        .map(Path::new)
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());

    // The rollout file's own stem -- `rollout-<timestamp>-<uuid>` -- the
    // identifier this session is already addressed by throughout discovery
    // and the queue. Not invented for this purpose.
    let conversation_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string());

    Ok(SessionTranscript {
        source: Cow::Borrowed(SOURCE_CODEX),
        agent_version,
        model,
        project,
        cwd,
        started_at,
        session_hash: hasher.finish(),
        conversation_id,
        events,
        subagent_count: 0,
        subagents_dropped: 0,
        routing: Vec::new(),
    })
}

/// Codex names both halves of a call with the same `call_id`, which is what
/// lets a result be paired with its call without trusting array order.
fn call_id(payload: &Value) -> Option<String> {
    payload
        .get("call_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn map_response_item(
    payload: Option<&Value>,
    timestamp: Option<chrono::DateTime<chrono::Utc>>,
    events: &mut Vec<SessionEvent>,
) {
    let Some(payload) = payload else {
        events.push(SessionEvent {
            kind: SessionEventKind::Opaque,
            timestamp,
            content: None,
            structured: json!({ "record_type": "response_item" }),
            tool_name: None,
            token_counts: None,
            tool_call_id: None,
            success: None,
        });
        return;
    };

    let item_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match item_type {
        "message" => {
            let role = payload.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let kind = match role {
                "user" => SessionEventKind::User,
                "assistant" => SessionEventKind::Assistant,
                _ => {
                    events.push(SessionEvent {
                        kind: SessionEventKind::Opaque,
                        timestamp,
                        content: None,
                        structured: json!({ "record_type": "message" }),
                        tool_name: None,
                        token_counts: None,
                        tool_call_id: None,
                        success: None,
                    });
                    return;
                }
            };
            let text = payload
                .get("content")
                .and_then(|v| v.as_array())
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            events.push(SessionEvent {
                kind,
                timestamp,
                content: Some(text),
                structured: Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: None,
                success: None,
            });
        }
        "function_call" | "custom_tool_call" => {
            let name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let arg_key = if item_type == "function_call" {
                "arguments"
            } else {
                "input"
            };
            let structured = match payload.get(arg_key) {
                Some(Value::String(s)) => serde_json::from_str::<Value>(s)
                    .unwrap_or_else(|_| json!({ "arguments_raw_len": s.len() })),
                Some(other) => other.clone(),
                None => Value::Null,
            };
            events.push(SessionEvent {
                kind: SessionEventKind::ToolCall,
                timestamp,
                content: None,
                structured,
                tool_name: name,
                token_counts: None,
                tool_call_id: call_id(payload),
                success: None,
            });
        }
        "function_call_output" | "custom_tool_call_output" => {
            let content = match payload.get("output") {
                Some(Value::String(s)) => Some(s.clone()),
                Some(Value::Object(obj)) => obj
                    .get("output")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| serde_json::to_string(obj).ok()),
                Some(other) => serde_json::to_string(other).ok(),
                None => None,
            };
            events.push(SessionEvent {
                kind: SessionEventKind::ToolResult,
                timestamp,
                content,
                structured: Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: call_id(payload),
                // Only an explicit verdict counts. An exit code would have to
                // be interpreted per tool, and guessing here would put a
                // fabricated outcome on a real trace.
                success: payload
                    .get("output")
                    .and_then(|output| output.get("success"))
                    .and_then(|v| v.as_bool()),
            });
        }
        "reasoning" => {
            // Reasoning is captured as a first-class event and redacted
            // through the same client-side pipeline as every other kind.
            let mut parts = Vec::new();
            for key in ["summary", "content"] {
                if let Some(blocks) = payload.get(key).and_then(|v| v.as_array()) {
                    for block in blocks {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                parts.push(text.to_string());
                            }
                        }
                    }
                }
            }
            // A reasoning item with no recoverable text carries no signal;
            // emitting an empty event would only add noise to the transcript.
            if !parts.is_empty() {
                events.push(SessionEvent {
                    kind: SessionEventKind::Reasoning,
                    timestamp,
                    content: Some(parts.join("\n")),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: None,
                    tool_call_id: None,
                    success: None,
                });
            }
        }
        other => {
            events.push(SessionEvent {
                kind: SessionEventKind::Opaque,
                timestamp,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{SessionEventKind, TraceSource};
    use std::path::PathBuf;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/codex")
    }

    /// A rollout is its own session, and nothing else in the tree is one.
    #[test]
    fn a_rollout_maps_to_itself_and_nothing_else_maps_at_all() {
        let root = tempfile::tempdir().unwrap();
        let day = root.path().join("2026/08/20");
        std::fs::create_dir_all(&day).unwrap();
        let rollout = day.join("rollout-2026-08-20T10-00-00-abc.jsonl");
        std::fs::write(&rollout, "{}\n").unwrap();
        // A stray file the walk passes over, and a directory.
        std::fs::write(day.join("notes.txt"), "x").unwrap();
        std::fs::write(day.join("not-a-rollout.jsonl"), "{}\n").unwrap();

        let outside = tempfile::tempdir().unwrap();
        let elsewhere = outside.path().join("rollout-2026-08-20T10-00-00-xyz.jsonl");
        std::fs::write(&elsewhere, "{}\n").unwrap();

        let source = CodexSource::new(root.path().to_path_buf());
        assert_eq!(source.session_for_path(&rollout), Some(rollout.clone()));
        // The address matches what discovery emits for the same file.
        let found = source.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, rollout);

        for path in [
            day.join("notes.txt"),
            day.join("not-a-rollout.jsonl"),
            day.clone(),
            root.path().to_path_buf(),
            day.join("rollout-never-written.jsonl"),
            elsewhere,
        ] {
            assert_eq!(
                source.session_for_path(&path),
                None,
                "{} must not address a session",
                path.display()
            );
        }
    }

    /// A scoped lookup and a full sweep must describe a rollout
    /// identically, or the two paths judge the same bytes differently.
    /// `Debug` rather than a hand-listed field set, so a field added later
    /// is covered too.
    #[test]
    fn session_at_describes_a_rollout_exactly_as_discover_does() {
        let root = tempfile::tempdir().unwrap();
        let day = root.path().join("2026/08/20");
        std::fs::create_dir_all(&day).unwrap();
        let rollout = day.join("rollout-2026-08-20T10-00-00-abc.jsonl");
        std::fs::write(
            &rollout,
            format!(
                "{}\n",
                serde_json::to_string(&json!({
                    "type": "session_meta", "payload": { "cwd": "/Users/z/code/proj" }
                }))
                .unwrap()
            ),
        )
        .unwrap();

        let source = CodexSource::new(root.path().to_path_buf());
        let discovered = source.discover().unwrap();
        assert_eq!(discovered.len(), 1);
        let scoped = source.session_at(&rollout).unwrap().expect("a session");

        assert_eq!(format!("{scoped:?}"), format!("{:?}", discovered[0]));
        assert_eq!(scoped.path, rollout);
        assert_eq!(scoped.cwd.as_deref(), Some("/Users/z/code/proj"));
        assert_eq!(scoped.project.as_deref(), Some("proj"));
        assert_eq!(scoped.group_member_count, 0);
    }

    /// A rollout deleted between the event and the lookup, and anything
    /// outside the root: `Ok(None)`, never an error.
    #[test]
    fn a_vanished_or_foreign_rollout_is_ok_none() {
        let root = tempfile::tempdir().unwrap();
        let day = root.path().join("2026/08/20");
        std::fs::create_dir_all(&day).unwrap();
        let rollout = day.join("rollout-2026-08-20T10-00-00-abc.jsonl");
        std::fs::write(&rollout, "{}\n").unwrap();

        let outside = tempfile::tempdir().unwrap();
        let elsewhere = outside.path().join("rollout-2026-08-20T10-00-00-xyz.jsonl");
        std::fs::write(&elsewhere, "{}\n").unwrap();

        let source = CodexSource::new(root.path().to_path_buf());
        assert!(source.session_at(&rollout).unwrap().is_some());
        assert!(source.session_at(&elsewhere).unwrap().is_none());
        assert!(source.session_at(&day.join("notes.txt")).unwrap().is_none());

        std::fs::remove_file(&rollout).unwrap();
        assert!(
            source.session_at(&rollout).unwrap().is_none(),
            "a deleted rollout must be Ok(None), not an error"
        );
    }

    /// The mapping is fed paths from the operating system, so a symlink or
    /// a `..` must not become a way to name a file outside the declared
    /// root. `collect_rollout_files` already refuses to descend a symlinked
    /// directory; this refuses the symlinked file too.
    #[test]
    #[cfg(unix)]
    fn path_mapping_refuses_symlinks_and_traversal_out_of_the_codex_root() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        let secret = outside
            .path()
            .join("rollout-2026-08-20T10-00-00-secret.jsonl");
        std::fs::write(&secret, "{}\n").unwrap();

        let root = tempfile::tempdir().unwrap();
        let day = root.path().join("2026/08/20");
        std::fs::create_dir_all(&day).unwrap();
        let real = day.join("rollout-2026-08-20T10-00-00-real.jsonl");
        std::fs::write(&real, "{}\n").unwrap();

        let linked_file = day.join("rollout-2026-08-20T10-00-01-link.jsonl");
        symlink(&secret, &linked_file).unwrap();
        let linked_dir = root.path().join("linked");
        symlink(outside.path(), &linked_dir).unwrap();

        let source = CodexSource::new(root.path().to_path_buf());
        for escape in [
            linked_file,
            linked_dir.join("rollout-2026-08-20T10-00-00-secret.jsonl"),
            day.join("..").join("..").join("..").join("..").join(
                secret
                    .file_name()
                    .map(std::path::PathBuf::from)
                    .unwrap_or_default(),
            ),
        ] {
            assert_eq!(
                source.session_for_path(&escape),
                None,
                "{} must not address a session",
                escape.display()
            );
        }
        assert_eq!(
            source.session_for_path(&real),
            Some(real.clone()),
            "the real rollout must still map, or this test proves nothing"
        );
    }

    /// A rollout past the budget is declined by name, not truncated.
    ///
    /// Truncating would upload a fragment described by a preview that says
    /// it is the whole conversation. The refusal names the size, which is
    /// the contributor's own, and never the path.
    #[test]
    fn a_rollout_past_the_budget_is_declined_rather_than_part_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-2026-08-20T10-00-02-big.jsonl");
        let head = serde_json::to_string(&json!({
            "type": "session_meta", "payload": { "cwd": "/Users/z/code/proj" }
        }))
        .unwrap();
        let mut bytes = head.into_bytes();
        bytes.push(b'\n');
        bytes.resize(ROLLOUT_BYTE_BUDGET as usize + 1, b'x');
        std::fs::write(&path, &bytes).unwrap();

        let raw = load_session(&path).unwrap_err();
        // Typed, so the daemon can tell this refusal -- a verdict that will
        // decide the same way on every poll -- from the IO errors around
        // it, which very likely will not. Matching on the message text
        // would make the wording load-bearing.
        let typed = raw
            .downcast_ref::<crate::source::SessionTooLarge>()
            .expect("an oversized rollout is refused by type, not only by message");
        assert_eq!(typed.budget_bytes, ROLLOUT_BYTE_BUDGET);
        assert!(typed.declared_bytes > ROLLOUT_BYTE_BUDGET);
        let err = raw.to_string();
        assert!(
            err.contains("rollout-too-large"),
            "expected a named refusal, got: {err}"
        );
        assert!(
            !err.contains("rollout-2026-08-20"),
            "the refusal must not carry the path: {err}"
        );

        // The same file one byte under the budget still loads, so the bound
        // is a bound and not an off-by-one that declines healthy sessions.
        bytes.truncate(ROLLOUT_BYTE_BUDGET as usize);
        std::fs::write(&path, &bytes).unwrap();
        assert!(load_session(&path).is_ok());
    }

    /// `peek_cwd` must answer from the head of the file and stop.
    ///
    /// The tail here is invalid UTF-8 and megabytes long. A whole-file read
    /// would pay for all of it on every discovery pass to learn something
    /// the first line already said; streaming stops at the answer. The
    /// invalid bytes also pin the lenient handling the old
    /// `from_utf8_lossy` gave: a session whose later records are unreadable
    /// still reports its cwd rather than vanishing from discovery.
    #[test]
    fn peek_cwd_answers_from_the_first_record_and_ignores_the_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-2026-08-20T10-00-00-x.jsonl");
        let mut bytes = serde_json::to_vec(&json!({
            "type": "session_meta",
            "payload": { "cwd": "/Users/z/code/proj" }
        }))
        .unwrap();
        bytes.push(b'\n');
        bytes.extend(std::iter::repeat_n(0xF5u8, 4 * 1024 * 1024));
        std::fs::write(&path, &bytes).unwrap();

        assert_eq!(peek_cwd(&path).as_deref(), Some("/Users/z/code/proj"));
    }

    /// A file whose size and mtime have not changed must not be read twice.
    ///
    /// Discovery runs on every watcher tick, so an unmemoized peek re-reads
    /// the whole corpus every `poll_interval_secs` to re-derive an answer
    /// that only changes when a file does. This rewrites the contents while
    /// holding size and mtime fixed: a memoized peek keeps the first
    /// answer, and a re-reading one reports the second.
    #[test]
    fn peek_cwd_is_memoized_while_size_and_mtime_hold() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-2026-08-20T10-00-01-y.jsonl");

        let first = serde_json::to_string(&json!({
            "type": "session_meta", "payload": { "cwd": "/first/answer" }
        }))
        .unwrap();
        let second = serde_json::to_string(&json!({
            "type": "session_meta", "payload": { "cwd": "/secnd/answer" }
        }))
        .unwrap();
        assert_eq!(
            first.len(),
            second.len(),
            "sizes must match to hold size fixed"
        );

        std::fs::write(&path, format!("{first}\n")).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let size = meta.len();
        let mtime = meta
            .modified()
            .ok()
            .map(chrono::DateTime::<chrono::Utc>::from);

        assert_eq!(
            peek_cwd_memoized(&path, size, mtime).as_deref(),
            Some("/first/answer")
        );

        std::fs::write(&path, format!("{second}\n")).unwrap();
        // Restore the mtime through std rather than adding a dependency for
        // it: the memo key is (size, mtime), so the rewrite has to be
        // invisible to that key for this test to mean anything.
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(meta.modified().unwrap())
            .unwrap();

        assert_eq!(
            peek_cwd_memoized(&path, size, mtime).as_deref(),
            Some("/first/answer"),
            "an unchanged file must be answered from the memo, not re-read"
        );
    }

    #[test]
    fn discovers_nested_rollout_files() {
        let src = CodexSource::new(fixture_root());
        let found = src.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, "codex");
    }

    #[test]
    fn discovery_labels_the_project_not_just_load() {
        // Discovery is what fills the picker a contributor chooses from. It
        // used to hardcode `project: None`, so every Codex row rendered as
        // `-` and one session was indistinguishable from another - even
        // though `load()` derived the same value correctly from the same cwd,
        // which is why submitted envelopes were right and only the list was
        // wrong. Assert both agree.
        let src = CodexSource::new(fixture_root());
        let found = src.discover().unwrap();
        assert_eq!(
            found[0].project.as_deref(),
            Some("otherproj"),
            "discovery must label the project, not leave it for load()"
        );
        let loaded = src.load(&found[0]).unwrap();
        assert_eq!(
            found[0].project.as_deref(),
            loaded.project.as_deref(),
            "discovery and load must agree on the project label"
        );
    }

    #[test]
    fn maps_response_items() {
        let src = CodexSource::new(fixture_root());
        let r = &src.discover().unwrap()[0];
        let t = src.load(r).unwrap();
        assert_eq!(t.model.as_deref(), Some("gpt-5.2-codex"));
        assert_eq!(t.agent_version.as_deref(), Some("0.48.0"));
        assert_eq!(t.project.as_deref(), Some("otherproj"));
        let kinds: Vec<_> = t.events.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                SessionEventKind::User,
                SessionEventKind::Reasoning,
                SessionEventKind::ToolCall,
                SessionEventKind::ToolResult,
                SessionEventKind::Assistant,
            ]
        );
        assert_eq!(t.events[1].content.as_deref(), Some("thinking about it"));
        assert_eq!(t.events[2].tool_name.as_deref(), Some("shell"));
        assert_eq!(t.events[2].structured["command"], "ls src/");
    }

    #[test]
    fn reasoning_items_become_reasoning_events() {
        let payload = serde_json::json!({
            "type": "reasoning",
            "summary": [{ "type": "summary_text", "text": "planning the edit" }],
            "content": [{ "type": "reasoning_text", "text": "file A needs a guard" }]
        });
        let mut events = Vec::new();
        super::map_response_item(Some(&payload), None, &mut events);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, SessionEventKind::Reasoning);
        assert_eq!(
            events[0].content.as_deref(),
            Some("planning the edit\nfile A needs a guard")
        );
    }

    #[test]
    fn reasoning_items_with_no_text_are_dropped() {
        let payload = serde_json::json!({ "type": "reasoning" });
        let mut events = Vec::new();
        super::map_response_item(Some(&payload), None, &mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn both_halves_of_a_call_carry_its_call_id() {
        // Codex names the call and its output with the same `call_id`. This
        // adapter parsed both records and kept neither id, so pairing a
        // result with its call came down to array order.
        let call = serde_json::json!({
            "type": "function_call",
            "name": "shell",
            "arguments": "{\"command\":\"ls\"}",
            "call_id": "c7"
        });
        let output = serde_json::json!({
            "type": "function_call_output",
            "call_id": "c7",
            "output": "src"
        });
        let mut events = Vec::new();
        super::map_response_item(Some(&call), None, &mut events);
        super::map_response_item(Some(&output), None, &mut events);
        assert_eq!(events[0].tool_call_id.as_deref(), Some("c7"));
        assert_eq!(events[1].tool_call_id.as_deref(), Some("c7"));
    }

    #[test]
    fn only_an_explicit_verdict_sets_success() {
        // An exit code would have to be read per tool, so it is not read at
        // all: a guessed outcome on a real trace is worse than no outcome.
        let explicit = serde_json::json!({
            "type": "function_call_output",
            "call_id": "c8",
            "output": {"success": false, "output": "boom"}
        });
        let bare = serde_json::json!({
            "type": "function_call_output",
            "call_id": "c9",
            "output": "fine"
        });
        let mut events = Vec::new();
        super::map_response_item(Some(&explicit), None, &mut events);
        super::map_response_item(Some(&bare), None, &mut events);
        assert_eq!(events[0].success, Some(false));
        assert_eq!(events[1].success, None);
    }
}
