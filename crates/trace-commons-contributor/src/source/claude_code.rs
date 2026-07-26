//! Claude Code transcript adapter.
//!
//! Reads `~/.claude/projects/<encoded-cwd>/<uuid>.jsonl` session files and
//! maps them into the shared `SessionTranscript` model. See
//! `docs/superpowers/plans/` (Task 7) for the format facts and mapping
//! rules; the key privacy invariant is that `Opaque` events (covering
//! `system`, `attachment`, and any unknown record `type`) carry only the
//! record type string, never the record payload.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    SOURCE_CLAUDE_CODE, SessionEvent, SessionEventKind, SessionRef, SessionTranscript, TraceSource,
    session_hash,
};

pub struct ClaudeCodeSource {
    root: PathBuf,
}

impl ClaudeCodeSource {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
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
            // Claude Code encodes the session's cwd as a directory name by
            // replacing every '/' with '-' (e.g. `/Users/testuser/code/myproj`
            // becomes `-Users-testuser-code-myproj`). As a best-effort
            // placeholder for discovery listings, take the segment after the
            // final '-' as the project basename. This is unreliable for
            // hyphenated project names (a project literally named "my-proj"
            // would only capture "proj" here) -- `load()` overrides this with
            // the true cwd basename read from the session file itself, so
            // this is only ever seen before a session has been loaded.
            let encoded_dir_name = project_dir.file_name();
            let discovery_project = encoded_dir_name
                .to_str()
                .and_then(|name| name.rsplit('-').next())
                .filter(|segment| !segment.is_empty())
                .map(|segment| segment.to_string());
            let Ok(entries) = std::fs::read_dir(project_dir.path()) else {
                continue;
            };
            for entry in entries {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                let path = entry.path();
                let entry_is_dir = match entry.file_type() {
                    Ok(ft) => ft.is_dir(),
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                if entry_is_dir {
                    // Known layout only: `<project-dir>/<session-uuid>/subagents/*.jsonl`.
                    // Deliberately not a general recursive walk -- an
                    // unrelated nested directory under a session-uuid dir
                    // must not be swept in.
                    let subagents_dir = path.join("subagents");
                    // `read_dir` FOLLOWS a symlinked directory. A `subagents`
                    // symlink planted by any process with write access under
                    // the transcript root would otherwise steer discovery at
                    // arbitrary directories, and everything discovered here is
                    // a candidate for upload. `symlink_metadata` does not
                    // follow, so a link reports as a symlink and is refused.
                    // (The top-level walk is already safe: `DirEntry::
                    // file_type` does not follow either.)
                    match std::fs::symlink_metadata(&subagents_dir) {
                        Ok(md) if md.is_dir() => {}
                        _ => continue,
                    }
                    let Ok(subagent_entries) = std::fs::read_dir(&subagents_dir) else {
                        continue;
                    };
                    for sub_entry in subagent_entries {
                        let sub_entry = match sub_entry {
                            Ok(e) => e,
                            Err(_) => {
                                skipped += 1;
                                continue;
                            }
                        };
                        push_session_if_jsonl(
                            &sub_entry,
                            discovery_project.clone(),
                            &mut sessions,
                            &mut skipped,
                        );
                    }
                    continue;
                }
                push_session_if_jsonl(
                    &entry,
                    discovery_project.clone(),
                    &mut sessions,
                    &mut skipped,
                );
            }
        }
        if skipped > 0 {
            tracing::warn!(
                skipped,
                "skipped unreadable claude-code session entries during discovery"
            );
        }
        Ok(sessions)
    }

    fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript> {
        load_session(&r.path)
    }
}

/// Builds a `SessionRef` for `entry` if it is a `.jsonl` file, pushing it
/// onto `sessions`. Shared between the top-level `<project-dir>/*.jsonl`
/// walk and the nested `<project-dir>/<session-uuid>/subagents/*.jsonl`
/// walk so both produce identically-shaped `SessionRef`s. Each subagent
/// transcript becomes its own `SessionRef` (not merged into its parent
/// session), matching the existing one-file-one-session model.
fn push_session_if_jsonl(
    entry: &std::fs::DirEntry,
    discovery_project: Option<String>,
    sessions: &mut Vec<SessionRef>,
    skipped: &mut usize,
) {
    let path = entry.path();
    if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
        return;
    }
    // Refuse symlinks. `entry.metadata()` and the later `std::fs::read` both
    // follow them, so a symlinked `.jsonl` is a path out of the transcript
    // root and into any file the user can read. `DirEntry::file_type` does
    // not follow, so it is the check that can tell the difference.
    match entry.file_type() {
        Ok(ft) if ft.is_file() => {}
        Ok(_) => return,
        Err(_) => {
            *skipped += 1;
            return;
        }
    }
    let metadata = match entry.metadata() {
        Ok(m) => m,
        Err(_) => {
            *skipped += 1;
            return;
        }
    };
    let started_at = metadata
        .modified()
        .ok()
        .map(chrono::DateTime::<chrono::Utc>::from);
    let cwd = peek_cwd(&path);
    sessions.push(SessionRef {
        source: SOURCE_CLAUDE_CODE,
        path,
        project: discovery_project,
        cwd,
        started_at,
        size_bytes: metadata.len(),
    });
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
    let bytes = std::fs::read(path).ok()?;
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
        if let Some(c) = record.get("cwd").and_then(|v| v.as_str()) {
            return Some(c.to_string());
        }
    }
    None
}

fn load_session(path: &Path) -> anyhow::Result<SessionTranscript> {
    let bytes = std::fs::read(path)?;
    let hash = session_hash(&bytes);
    let text = String::from_utf8_lossy(&bytes);

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
                    kind: SessionEventKind::Opaque,
                    timestamp: record_timestamp,
                    content: None,
                    structured: json!({ "record_type": other }),
                    tool_name: None,
                    token_counts: None,
                });
            }
        }
    }

    if unparseable > 0 {
        tracing::warn!(unparseable, "skipped unparseable Claude Code record lines");
    }

    let project = cwd
        .as_deref()
        .map(Path::new)
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());

    Ok(SessionTranscript {
        source: SOURCE_CLAUDE_CODE,
        agent_version,
        model,
        project,
        cwd,
        started_at,
        session_hash: hash,
        events,
    })
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
                kind: SessionEventKind::User,
                timestamp,
                content: Some(s.clone()),
                structured: Value::Null,
                tool_name: None,
                token_counts: None,
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
                            kind: SessionEventKind::ToolResult,
                            timestamp,
                            content: flattened,
                            structured: Value::Null,
                            tool_name: None,
                            token_counts: None,
                        });
                    }
                    _ => {}
                }
            }
            if !texts.is_empty() {
                events.push(SessionEvent {
                    kind: SessionEventKind::User,
                    timestamp,
                    content: Some(texts.join("\n")),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: None,
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

    let Some(Value::Array(blocks)) = record.pointer("/message/content") else {
        return;
    };

    // Text blocks are joined into a single Assistant event, inserted at the
    // position of the first text block encountered so event order still
    // reflects the original block order relative to tool_use events.
    let mut texts = Vec::new();
    let mut text_insert_at: Option<usize> = None;
    for block in blocks {
        match block.get("type").and_then(|v| v.as_str()) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    if text_insert_at.is_none() {
                        text_insert_at = Some(events.len());
                    }
                    texts.push(t.to_string());
                }
            }
            Some("tool_use") => {
                let name = block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let input = block.get("input").cloned().unwrap_or(Value::Null);
                events.push(SessionEvent {
                    kind: SessionEventKind::ToolCall,
                    timestamp,
                    content: None,
                    structured: input,
                    tool_name: name,
                    token_counts: None,
                });
            }
            Some("thinking") => {
                // Deliberately dropped: v1 privacy posture excludes model
                // reasoning traces from the transcript entirely.
            }
            _ => {}
        }
    }

    if let Some(idx) = text_insert_at {
        events.insert(
            idx,
            SessionEvent {
                kind: SessionEventKind::Assistant,
                timestamp,
                content: Some(texts.join("\n")),
                structured: Value::Null,
                tool_name: None,
                token_counts,
            },
        );
    }
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

    #[test]
    fn discovers_subagent_transcripts_one_level_deeper() {
        // Claude Code stores subagent transcripts at
        // `<project-dir>/<session-uuid>/subagents/agent-<id>.jsonl`, one
        // level below ordinary top-level sessions
        // (`<project-dir>/<uuid>.jsonl`). discover() must walk into that
        // known layout and surface each subagent transcript as its own
        // SessionRef, without recursing into unrelated nested directories.
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("-Users-testuser-code-myproj");
        std::fs::create_dir_all(&project_dir).unwrap();

        // An ordinary top-level session.
        std::fs::write(
            project_dir.join("11111111-1111-1111-1111-111111111111.jsonl"),
            "{\"type\":\"user\",\"cwd\":\"/Users/testuser/code/myproj\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n",
        )
        .unwrap();

        // A subagent transcript nested under the session's subagents dir.
        let subagents_dir = project_dir
            .join("22222222-2222-2222-2222-222222222222")
            .join("subagents");
        std::fs::create_dir_all(&subagents_dir).unwrap();
        std::fs::write(
            subagents_dir.join("agent-abc123.jsonl"),
            "{\"type\":\"user\",\"cwd\":\"/Users/testuser/code/myproj\",\"message\":{\"role\":\"user\",\"content\":\"sub hi\"}}\n",
        )
        .unwrap();

        // An unrelated nested directory that must NOT be swept in.
        let unrelated_dir = project_dir
            .join("22222222-2222-2222-2222-222222222222")
            .join("scratch");
        std::fs::create_dir_all(&unrelated_dir).unwrap();
        std::fs::write(
            unrelated_dir.join("not-a-session.jsonl"),
            "{\"type\":\"user\"}\n",
        )
        .unwrap();

        let src = ClaudeCodeSource::new(root.path().to_path_buf());
        let mut found = src.discover().unwrap();
        assert_eq!(found.len(), 2, "found: {found:?}");

        found.sort_by(|a, b| a.path.cmp(&b.path));
        let top_level = &found[0];
        let subagent = &found[1];

        assert_eq!(top_level.source, "claude-code");
        assert!(
            top_level
                .path
                .ends_with("11111111-1111-1111-1111-111111111111.jsonl")
        );

        assert_eq!(subagent.source, "claude-code");
        assert!(subagent.path.ends_with("subagents/agent-abc123.jsonl"));
        assert_eq!(subagent.cwd.as_deref(), Some("/Users/testuser/code/myproj"));

        // The unrelated nested directory must not be discovered.
        assert!(
            !found
                .iter()
                .any(|s| s.path.ends_with("not-a-session.jsonl")),
            "unrelated nested directory was swept into discovery"
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
                SessionEventKind::Assistant,
                SessionEventKind::ToolCall,
                SessionEventKind::ToolResult,
                SessionEventKind::Assistant,
                SessionEventKind::Opaque, // system
                SessionEventKind::Opaque, // attachment
                SessionEventKind::Opaque, // future-unknown-record
            ]
        );
        // Thinking dropped; token counts captured on the assistant text event.
        assert_eq!(t.events[1].token_counts, Some((100, 25)));
        assert_eq!(t.events[2].tool_name.as_deref(), Some("Read"));
        // Opaque events carry only the record type, never payloads.
        let serialized = serde_json::to_string(&t.events[6].structured).unwrap();
        assert!(!serialized.contains("do not leak me"));
        assert!(serialized.contains("attachment"));
        // Thinking text is gone entirely.
        let all = format!("{:?}", t.events);
        assert!(!all.contains("secret reasoning"));
    }
}
