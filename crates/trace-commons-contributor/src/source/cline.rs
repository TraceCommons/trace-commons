//! Cline session adapter.
//!
//! Reads `<root>/<session-id>/<session-id>.messages.json`, where `<root>` is
//! Cline's own session data directory: `$CLINE_SESSION_DATA_DIR`, else
//! `$CLINE_DATA_DIR/sessions`, else `$CLINE_DIR/data/sessions`, else
//! `~/.cline/data/sessions`. One directory is one session; the messages
//! document is a single JSON object, not JSONL, and a sibling
//! `<session-id>.json` manifest carries the working directory, the model and
//! the start time when the session had one.
//!
//! This is the store the current Cline release (extension 4.1.17, built on
//! the `@cline/core` SDK) writes. The pre-SDK layout under VS Code's global
//! storage (`tasks/<id>/api_conversation_history.json`) is not read: upstream
//! itself treats it as read-only legacy, and it carries neither timestamps
//! nor model information per message.
//!
//! **Message-type dispatch is tolerant, and only that**, on the same terms as
//! `gemini_cli`: an unrecognised content block becomes an `Opaque` event
//! with a type marker rather than rejecting the file, because the SDK's
//! message shape is young and moving. Everything a gate depends on -- path
//! containment, the byte budget, and the requirement that the document
//! actually carry a `messages` array -- stays fail-closed.
//!
//! Image blocks are never copied: their `data` is base64 pixels, which is
//! neither text a gate scores nor something a contributor reviewed.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde_json::{Value, json};

use super::{
    SOURCE_CLINE, SessionEvent, SessionEventKind, SessionRef, SessionTranscript, TraceSource,
    real_file_within_root, session_hash,
};

/// Overrides the whole Cline directory; sessions live under `data/sessions`.
pub const CLINE_DIR_ENV: &str = "CLINE_DIR";
/// Overrides the data directory; sessions live under `sessions`.
pub const CLINE_DATA_DIR_ENV: &str = "CLINE_DATA_DIR";
/// Overrides the session directory itself.
pub const CLINE_SESSION_DATA_DIR_ENV: &str = "CLINE_SESSION_DATA_DIR";

/// The largest session document this adapter will load, shared with every
/// other adapter's budget: they all bound how much of one conversation may
/// become resident on its way to being discarded.
pub(crate) const CLINE_SESSION_BUDGET: u64 = super::claude_code::GROUP_RAW_BYTE_BUDGET;

const MESSAGES_SUFFIX: &str = ".messages.json";
const MANIFEST_SUFFIX: &str = ".json";

/// The conventional store, resolved the way Cline's own `paths.ts` does it.
/// An empty variable counts as unset, matching upstream's `.trim()` check.
pub fn conventional_root(home: &Path, env: impl Fn(&str) -> Option<String>) -> PathBuf {
    let set = |key: &str| env(key).filter(|v| !v.trim().is_empty()).map(PathBuf::from);
    if let Some(sessions) = set(CLINE_SESSION_DATA_DIR_ENV) {
        return sessions;
    }
    if let Some(data) = set(CLINE_DATA_DIR_ENV) {
        return data.join("sessions");
    }
    set(CLINE_DIR_ENV)
        .unwrap_or_else(|| home.join(".cline"))
        .join("data")
        .join("sessions")
}

/// The conventional store, resolved against this machine's real home and
/// environment.
pub fn conventional_root_this_machine() -> PathBuf {
    conventional_root(&dirs::home_dir().unwrap_or_default(), |key| {
        std::env::var(key).ok()
    })
}

pub struct ClineSource {
    root: PathBuf,
}

impl ClineSource {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

/// The messages file a session directory must hold: `<dir name>.messages.json`.
fn messages_file_for(session_dir: &Path) -> Option<PathBuf> {
    let id = session_dir.file_name()?.to_str()?;
    Some(session_dir.join(format!("{id}{MESSAGES_SUFFIX}")))
}

/// The sibling manifest, if the session wrote one.
fn manifest_for(messages_path: &Path) -> Option<PathBuf> {
    let dir = messages_path.parent()?;
    let id = dir.file_name()?.to_str()?;
    let candidate = dir.join(format!("{id}{MANIFEST_SUFFIX}"));
    candidate.is_file().then_some(candidate)
}

/// What the manifest says about the session, where it says it. Every field
/// is optional: a session interrupted before its manifest was written is
/// still a session.
#[derive(Default)]
struct Manifest {
    cwd: Option<String>,
    model: Option<String>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn read_manifest(messages_path: &Path) -> Manifest {
    let Some(path) = manifest_for(messages_path) else {
        return Manifest::default();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Manifest::default();
    };
    let Ok(doc) = serde_json::from_slice::<Value>(&bytes) else {
        return Manifest::default();
    };
    let string = |key: &str| {
        doc.get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };
    Manifest {
        cwd: string("cwd"),
        model: string("model"),
        started_at: timestamp_rfc3339(doc.get("started_at")),
    }
}

/// The label a picker renders: the basename of the working directory when
/// there is one, otherwise the session directory's own name.
fn project_label(session_dir: &Path, cwd: Option<&str>) -> Option<String> {
    cwd.map(Path::new)
        .or(Some(session_dir))
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

/// The one way a Cline `SessionRef` is built, shared by `discover` and
/// `session_at` so a scoped scan and a full sweep cannot disagree.
///
/// `None` for a file that is no longer there, which on the event path is an
/// ordinary race rather than a failure.
fn session_ref_for(path: PathBuf) -> Option<SessionRef> {
    let session_dir = path.parent()?.to_path_buf();
    let metadata = std::fs::metadata(&path).ok()?;
    let manifest = read_manifest(&path);
    let started_at = manifest.started_at.or_else(|| {
        metadata
            .modified()
            .ok()
            .map(chrono::DateTime::<chrono::Utc>::from)
    });
    let project = project_label(&session_dir, manifest.cwd.as_deref());
    Some(SessionRef {
        source: SOURCE_CLINE,
        declared_source: None,
        path,
        project,
        cwd: manifest.cwd,
        started_at,
        size_bytes: metadata.len(),
        // One document is one session. A subagent session is its own
        // directory with an `origin.parentThreadId` back-reference that this
        // adapter does not follow, so there is no group to describe.
        group_modified_at: None,
        group_member_count: 0,
    })
}

impl TraceSource for ClineSource {
    fn name(&self) -> &'static str {
        SOURCE_CLINE
    }

    fn discover(&self) -> anyhow::Result<Vec<SessionRef>> {
        let mut sessions = Vec::new();
        let mut skipped = 0usize;
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Ok(sessions);
        };
        for entry in entries {
            let Ok(entry) = entry else {
                skipped += 1;
                continue;
            };
            // `file_type` does not follow, so a symlinked session directory
            // is not descended into: a link planted under the store by any
            // same-user process must not steer collection elsewhere.
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => {}
                Ok(_) => continue,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            }
            let Some(messages) = messages_file_for(&entry.path()) else {
                continue;
            };
            match std::fs::symlink_metadata(&messages) {
                Ok(m) if m.is_file() => {}
                _ => continue,
            }
            match session_ref_for(messages) {
                Some(r) => sessions.push(r),
                None => skipped += 1,
            }
        }
        if skipped > 0 {
            tracing::warn!(
                skipped,
                "skipped unreadable cline session entries during discovery"
            );
        }
        Ok(sessions)
    }

    /// A changed messages file is its own session, on exactly the terms
    /// `discover` uses: `<root>/<id>/<id>.messages.json`, two components
    /// deep. The manifest is deliberately not mapped: it changing does not
    /// change the bytes the transcript hashes.
    fn session_for_path(&self, path: &Path) -> Option<PathBuf> {
        let path = real_file_within_root(&self.root, path)?;
        let session_dir = path.parent()?;
        if session_dir.parent() != Some(self.root.as_path()) {
            return None;
        }
        (messages_file_for(session_dir)? == path).then_some(path)
    }

    fn session_at(&self, path: &Path) -> anyhow::Result<Option<SessionRef>> {
        let Some(address) = self.session_for_path(path) else {
            return Ok(None);
        };
        Ok(session_ref_for(address))
    }

    fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript> {
        load_session(&r.path)
    }
}

fn timestamp_rfc3339(value: Option<&Value>) -> Option<chrono::DateTime<chrono::Utc>> {
    value
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// `ts` is milliseconds since the epoch, as `Date.now()` writes it.
fn timestamp_millis(value: Option<&Value>) -> Option<chrono::DateTime<chrono::Utc>> {
    value
        .and_then(|v| v.as_i64())
        .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
}

/// Text from a `content` field that may be a bare string or a block list.
/// Only `text` blocks contribute; everything else in the list is mapped as
/// its own event by the caller.
fn text_parts(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let joined = parts
                .iter()
                .filter(|p| p.get("type").and_then(|v| v.as_str()) == Some("text"))
                .filter_map(|p| p.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            (!joined.is_empty()).then_some(joined)
        }
        _ => None,
    }
}

fn opaque(record_type: &str, timestamp: Option<chrono::DateTime<chrono::Utc>>) -> SessionEvent {
    SessionEvent {
        served_by: None,
        kind: SessionEventKind::Opaque,
        timestamp,
        content: None,
        structured: json!({ "record_type": record_type }),
        tool_name: None,
        token_counts: None,
        tool_call_id: None,
        success: None,
    }
}

/// `metrics.inputTokens` and `metrics.outputTokens`, both or neither.
fn token_counts_of(message: &Value) -> Option<(u32, u32)> {
    let metrics = message.get("metrics")?;
    let input = metrics.get("inputTokens")?.as_u64()?;
    let output = metrics.get("outputTokens")?.as_u64()?;
    Some((u32::try_from(input).ok()?, u32::try_from(output).ok()?))
}

/// One message expands to its blocks, in order. A bare-string `content` is
/// one text block. The message's `ts` stamps every block: the SDK records
/// one time per message, not per block.
fn map_message(message: &Value, model: &mut Option<String>, events: &mut Vec<SessionEvent>) {
    let timestamp = timestamp_millis(message.get("ts"));
    let role = message.get("role").and_then(|v| v.as_str()).unwrap_or("");
    if model.is_none() {
        if let Some(id) = message
            .get("modelInfo")
            .and_then(|m| m.get("id"))
            .and_then(|v| v.as_str())
        {
            *model = Some(id.to_string());
        }
    }
    let text_kind = match role {
        "user" => SessionEventKind::User,
        "assistant" => SessionEventKind::Assistant,
        other => {
            events.push(opaque(other, timestamp));
            return;
        }
    };
    // Token counts belong to the assistant's step. They are attached to the
    // first text block of the message, which is what `token_counts` means on
    // every other adapter: the provider's count for the step that produced
    // this text. A user message never carries them.
    let mut token_counts = (text_kind == SessionEventKind::Assistant)
        .then(|| token_counts_of(message))
        .flatten();

    let Some(content) = message.get("content") else {
        return;
    };
    let blocks: Vec<Value> = match content {
        Value::String(s) => vec![json!({ "type": "text", "text": s })],
        Value::Array(parts) => parts.clone(),
        _ => return,
    };

    for block in &blocks {
        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match block_type {
            "text" => {
                let Some(text) = block.get("text").and_then(|v| v.as_str()) else {
                    continue;
                };
                if text.is_empty() {
                    continue;
                }
                events.push(SessionEvent {
                    served_by: None,
                    kind: text_kind.clone(),
                    timestamp,
                    content: Some(text.to_string()),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: token_counts.take(),
                    tool_call_id: None,
                    success: None,
                });
            }
            "thinking" => {
                let Some(thinking) = block.get("thinking").and_then(|v| v.as_str()) else {
                    continue;
                };
                if thinking.is_empty() {
                    continue;
                }
                events.push(SessionEvent {
                    served_by: None,
                    kind: SessionEventKind::Reasoning,
                    timestamp,
                    content: Some(thinking.to_string()),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: None,
                    tool_call_id: None,
                    success: None,
                });
            }
            "tool_use" => events.push(SessionEvent {
                served_by: None,
                kind: SessionEventKind::ToolCall,
                timestamp,
                content: None,
                structured: block.get("input").cloned().unwrap_or(Value::Null),
                tool_name: block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                token_counts: None,
                tool_call_id: block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                success: None,
            }),
            "tool_result" => events.push(SessionEvent {
                served_by: None,
                kind: SessionEventKind::ToolResult,
                timestamp,
                content: block.get("content").and_then(text_parts),
                structured: Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: block
                    .get("tool_use_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                // Only an explicit `is_error` is a verdict. Absent means the
                // harness said nothing, which is not success.
                success: block.get("is_error").and_then(|v| v.as_bool()).map(|e| !e),
            }),
            other => events.push(opaque(other, timestamp)),
        }
    }
}

fn load_session(path: &Path) -> anyhow::Result<SessionTranscript> {
    // Declined rather than truncated, and named rather than silent: a
    // half-parsed transcript would upload as though it were the whole
    // conversation. The size is the contributor's own file's and safe to
    // state; the path is not, and is deliberately absent.
    let declared = std::fs::metadata(path)?.len();
    if declared > CLINE_SESSION_BUDGET {
        return Err(super::SessionTooLarge {
            label: "cline-session-too-large",
            declared_bytes: declared,
            budget_bytes: CLINE_SESSION_BUDGET,
        }
        .into());
    }
    let bytes = std::fs::read(path)?;
    let hash = session_hash(&bytes);
    // One JSON document, so there is no streaming form to read it in: the
    // budget above is what bounds this read.
    let document: Value =
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("malformed_cline_session"))?;
    // The tolerance is for block *types*, not for the document. A file with
    // no `messages` array is not a session document at all, and accepting
    // it would offer an empty transcript as though it were a conversation.
    let messages = document
        .get("messages")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("malformed_cline_session"))?;

    let manifest = read_manifest(path);
    let mut events = Vec::new();
    let mut model: Option<String> = None;
    for message in messages {
        map_message(message, &mut model, &mut events);
    }

    let session_dir = path.parent();
    let project = session_dir.and_then(|dir| project_label(dir, manifest.cwd.as_deref()));
    let started_at = manifest
        .started_at
        .or_else(|| messages.first().and_then(|m| timestamp_millis(m.get("ts"))));
    // The document's own id, which is what the store addresses it by; the
    // directory name merely repeats it. Empty is not an id: `Some("")` would
    // suppress the directory-name fallback and then join, by equality, to
    // every ledger row that also names no session. Filtered here the same
    // way `read_manifest`'s own `string` helper filters its fields.
    let conversation_id = document
        .get("sessionId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            session_dir
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        });

    Ok(SessionTranscript {
        source: Cow::Borrowed(SOURCE_CLINE),
        // The manifest carries no extension version.
        agent_version: None,
        model: model.or(manifest.model),
        project,
        cwd: manifest.cwd,
        started_at,
        session_hash: hash,
        conversation_id,
        events,
        // A subagent session is its own directory with a back-reference this
        // adapter does not follow, so nothing is ever merged in.
        subagent_count: 0,
        subagents_dropped: 0,
        routing: Vec::new(),
        attested_call: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{SOURCE_CLINE, SessionEventKind, TraceSource};

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cline/sessions")
    }

    fn source() -> ClineSource {
        ClineSource::new(fixture_root())
    }

    #[test]
    fn the_conventional_root_follows_clines_own_precedence() {
        let home = Path::new("/home/c");
        let none = |_: &str| None;
        assert_eq!(
            conventional_root(home, none),
            PathBuf::from("/home/c/.cline/data/sessions")
        );
        let dir = |k: &str| (k == CLINE_DIR_ENV).then(|| "/opt/cline".to_string());
        assert_eq!(
            conventional_root(home, dir),
            PathBuf::from("/opt/cline/data/sessions")
        );
        let data = |k: &str| match k {
            CLINE_DIR_ENV => Some("/opt/cline".to_string()),
            CLINE_DATA_DIR_ENV => Some("/data/cl".to_string()),
            _ => None,
        };
        assert_eq!(
            conventional_root(home, data),
            PathBuf::from("/data/cl/sessions")
        );
        let sessions = |k: &str| match k {
            CLINE_DATA_DIR_ENV => Some("/data/cl".to_string()),
            CLINE_SESSION_DATA_DIR_ENV => Some("/s".to_string()),
            _ => None,
        };
        assert_eq!(conventional_root(home, sessions), PathBuf::from("/s"));
        // An empty value is unset, as upstream's `.trim()` check treats it.
        let empty = |k: &str| (k == CLINE_SESSION_DATA_DIR_ENV).then(String::new);
        assert_eq!(
            conventional_root(home, empty),
            PathBuf::from("/home/c/.cline/data/sessions")
        );
    }

    #[test]
    fn discovery_finds_each_messages_file_and_nothing_else() {
        let refs = source().discover().unwrap();
        let mut names: Vec<String> = refs
            .iter()
            .map(|r| r.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "1756900000000_k3x9q.messages.json",
                "1756900100000_p2m7z.messages.json",
                "1756900200000_bad00.messages.json",
            ],
            "the stray directory is skipped; a malformed document is still discovered and refused at load"
        );
        for r in &refs {
            assert_eq!(r.source, SOURCE_CLINE);
            assert!(r.size_bytes > 0);
            assert_eq!(r.group_member_count, 0);
        }
    }

    #[test]
    fn a_manifest_gives_discovery_the_cwd_and_project() {
        let refs = source().discover().unwrap();
        let with = refs
            .iter()
            .find(|r| r.path.to_string_lossy().contains("k3x9q"))
            .unwrap();
        assert_eq!(with.cwd.as_deref(), Some("/home/contributor/code/alpha"));
        assert_eq!(with.project.as_deref(), Some("alpha"));
        let without = refs
            .iter()
            .find(|r| r.path.to_string_lossy().contains("p2m7z"))
            .unwrap();
        assert_eq!(without.cwd, None, "no manifest, no guess");
        assert_eq!(
            without.project.as_deref(),
            Some("1756900100000_p2m7z"),
            "the session directory name is the fallback label"
        );
    }

    #[test]
    fn a_changed_messages_file_maps_to_its_own_session_and_nothing_else_does() {
        let s = source();
        let messages = fixture_root().join("1756900000000_k3x9q/1756900000000_k3x9q.messages.json");
        assert_eq!(s.session_for_path(&messages), Some(messages.clone()));
        // The manifest changing does not change the transcript's bytes.
        let manifest = fixture_root().join("1756900000000_k3x9q/1756900000000_k3x9q.json");
        assert_eq!(s.session_for_path(&manifest), None);
        // Outside the root, and a name that does not follow the rule.
        assert_eq!(s.session_for_path(Path::new("/etc/passwd")), None);
        let stray = fixture_root().join("not-a-session/notes.txt");
        assert_eq!(s.session_for_path(&stray), None);
        // A messages file whose name disagrees with its directory is not a
        // session: the id is the directory, and the file must repeat it.
        let wrong = fixture_root().join("1756900000000_k3x9q/other.messages.json");
        assert_eq!(s.session_for_path(&wrong), None);
    }

    #[test]
    fn session_at_agrees_with_discover() {
        let s = source();
        for r in s.discover().unwrap() {
            let again = s.session_at(&r.path).unwrap().expect("the same session");
            assert_eq!(again.path, r.path);
            assert_eq!(again.size_bytes, r.size_bytes);
            assert_eq!(again.cwd, r.cwd);
            assert_eq!(again.project, r.project);
        }
    }

    fn load(name: &str) -> SessionTranscript {
        let s = source();
        let r = s
            .discover()
            .unwrap()
            .into_iter()
            .find(|r| r.path.to_string_lossy().contains(name))
            .unwrap();
        s.load(&r).unwrap()
    }

    #[test]
    fn transcript_fields_come_from_the_document_and_its_manifest() {
        let t = load("k3x9q");
        assert_eq!(t.source, SOURCE_CLINE);
        assert_eq!(t.conversation_id.as_deref(), Some("1756900000000_k3x9q"));
        assert_eq!(t.model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(t.cwd.as_deref(), Some("/home/contributor/code/alpha"));
        assert_eq!(t.project.as_deref(), Some("alpha"));
        assert_eq!(
            t.started_at.map(|ts| ts.to_rfc3339()),
            Some("2026-09-03T11:20:00+00:00".to_string()),
            "the manifest's start, not the first message"
        );
        assert_eq!(t.agent_version, None);
        assert_eq!(t.subagent_count, 0);
        assert!(t.routing.is_empty());
        let bytes = std::fs::read(
            fixture_root().join("1756900000000_k3x9q/1756900000000_k3x9q.messages.json"),
        )
        .unwrap();
        assert_eq!(t.session_hash, crate::source::session_hash(&bytes));
    }

    #[test]
    fn blocks_become_events_in_document_order() {
        let t = load("k3x9q");
        let kinds: Vec<&SessionEventKind> = t.events.iter().map(|e| &e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &SessionEventKind::User,
                &SessionEventKind::Reasoning,
                &SessionEventKind::Assistant,
                &SessionEventKind::ToolCall,
                &SessionEventKind::ToolResult,
                &SessionEventKind::Assistant,
            ]
        );
        let user = &t.events[0];
        assert_eq!(user.content.as_deref(), Some("List the files in src"));
        assert_eq!(
            user.timestamp.map(|ts| ts.to_rfc3339()),
            Some("2025-09-03T11:46:40+00:00".to_string()),
            "ts is epoch milliseconds"
        );
        let reasoning = &t.events[1];
        assert_eq!(
            reasoning.content.as_deref(),
            Some("The user wants a directory listing.")
        );
        let assistant = &t.events[2];
        assert_eq!(
            assistant.content.as_deref(),
            Some("I'll list the directory.")
        );
        assert_eq!(assistant.token_counts, Some((1200, 85)));
        assert_eq!(
            assistant.served_by, None,
            "Cline does not split cache writes by duration, so the step is unpriced rather than underpriced"
        );
        let call = &t.events[3];
        assert_eq!(call.tool_name.as_deref(), Some("list_files"));
        assert_eq!(call.tool_call_id.as_deref(), Some("toolu_01"));
        assert_eq!(
            call.structured,
            serde_json::json!({ "path": "src", "recursive": false })
        );
        let result = &t.events[4];
        assert_eq!(result.tool_call_id.as_deref(), Some("toolu_01"));
        assert_eq!(result.content.as_deref(), Some("index.ts\nutil.ts"));
        assert_eq!(
            result.success, None,
            "no is_error field means no verdict, not success"
        );
        let last = &t.events[5];
        assert_eq!(last.token_counts, Some((1300, 20)));
    }

    #[test]
    fn string_content_failed_results_and_unknown_blocks_are_handled() {
        let t = load("p2m7z");
        assert_eq!(
            t.model.as_deref(),
            Some("gpt-5.5"),
            "from modelInfo when there is no manifest"
        );
        assert_eq!(t.cwd, None);
        assert_eq!(
            t.started_at.map(|ts| ts.to_rfc3339()),
            Some("2025-09-03T11:48:20+00:00".to_string()),
            "the first message's ts when there is no manifest"
        );
        let user = &t.events[0];
        assert_eq!(user.kind, SessionEventKind::User);
        assert_eq!(user.content.as_deref(), Some("Run the tests"));
        let call = &t.events[1];
        assert_eq!(call.kind, SessionEventKind::ToolCall);
        assert_eq!(call.tool_name.as_deref(), Some("execute_command"));
        assert_eq!(call.token_counts, None, "no metrics, no counts");
        let result = &t.events[2];
        assert_eq!(result.kind, SessionEventKind::ToolResult);
        assert_eq!(result.success, Some(false));
        assert_eq!(
            result.content.as_deref(),
            Some("error: no such command"),
            "text parts joined; the image part is dropped"
        );
        let image = &t.events[3];
        assert_eq!(image.kind, SessionEventKind::Opaque);
        assert_eq!(
            image.structured,
            serde_json::json!({ "record_type": "image" })
        );
        assert!(image.content.is_none());
        let unknown = &t.events[4];
        assert_eq!(unknown.kind, SessionEventKind::Opaque);
        assert_eq!(
            unknown.structured,
            serde_json::json!({ "record_type": "future_block" })
        );
        assert!(
            !t.events
                .iter()
                .filter_map(|e| e.content.as_deref())
                .any(|c| c.contains("AAAA") || c.contains("BBBB")),
            "image data never reaches an event"
        );
    }

    #[test]
    fn a_document_with_no_messages_array_is_refused_with_a_label_only() {
        let s = source();
        let r = s
            .discover()
            .unwrap()
            .into_iter()
            .find(|r| r.path.to_string_lossy().contains("bad00"))
            .unwrap();
        let err = s.load(&r).unwrap_err().to_string();
        assert_eq!(err, "malformed_cline_session");
    }

    #[test]
    fn an_empty_session_id_falls_back_to_the_directory_name() {
        // `Some("")` is not an id. It would suppress this fallback and then
        // join, by equality, to every routing ledger row that also names no
        // session -- putting another session's cost on this trace.
        for spelling in ["\"\"", "\"   \"", "null"] {
            let dir = tempfile::tempdir().unwrap();
            let session = dir.path().join("1756900400000_empty");
            std::fs::create_dir_all(&session).unwrap();
            std::fs::write(
                session.join("1756900400000_empty.messages.json"),
                format!(
                    "{{\"version\":1,\"sessionId\":{spelling},\"messages\":[{{\"role\":\"user\",\"content\":\"hi\"}}]}}"
                ),
            )
            .unwrap();
            let s = ClineSource::new(dir.path().to_path_buf());
            let r = s.discover().unwrap().into_iter().next().unwrap();
            let t = s.load(&r).unwrap();
            assert_eq!(
                t.conversation_id.as_deref(),
                Some("1756900400000_empty"),
                "sessionId {spelling} did not fall back to the directory name"
            );
        }
    }

    #[test]
    fn a_document_over_budget_is_declined_by_size_not_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("1756900300000_big00");
        std::fs::create_dir_all(&session).unwrap();
        let path = session.join("1756900300000_big00.messages.json");
        let mut body = String::from(
            "{\"version\":1,\"sessionId\":\"1756900300000_big00\",\"messages\":[{\"role\":\"user\",\"content\":\"",
        );
        body.push_str(&"x".repeat(CLINE_SESSION_BUDGET as usize + 16));
        body.push_str("\"}]}");
        std::fs::write(&path, body).unwrap();
        let s = ClineSource::new(dir.path().to_path_buf());
        let r = s.discover().unwrap().into_iter().next().unwrap();
        let err = s.load(&r).unwrap_err();
        let too_large = err
            .downcast_ref::<crate::source::SessionTooLarge>()
            .expect("a size refusal");
        assert_eq!(too_large.label, "cline-session-too-large");
        assert!(
            !err.to_string().contains("big00"),
            "a refusal names no path"
        );
    }
}
