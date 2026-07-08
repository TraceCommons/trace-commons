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
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                let started_at = metadata
                    .modified()
                    .ok()
                    .map(chrono::DateTime::<chrono::Utc>::from);
                sessions.push(SessionRef {
                    source: SOURCE_CLAUDE_CODE,
                    path,
                    project: None,
                    started_at,
                    size_bytes: metadata.len(),
                });
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
    fn discovers_fixture_session() {
        let src = ClaudeCodeSource::new(fixture_root());
        let found = src.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, "claude-code");
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
