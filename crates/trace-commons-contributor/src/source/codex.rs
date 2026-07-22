//! Codex rollout transcript adapter.
//!
//! Reads `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`
//! session files and maps them into the shared `SessionTranscript` model.
//! See `docs/superpowers/plans/` (Task 8) for the format facts and mapping
//! rules; the key privacy invariant is that `Opaque` events (covering
//! `reasoning`, `event_msg`, `web_search_call`, and any unknown payload/record
//! type) carry only the record type string, never the record payload.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    SOURCE_CODEX, SessionEvent, SessionEventKind, SessionRef, SessionTranscript, TraceSource,
    session_hash,
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
        if !file_name.starts_with("rollout-") || !file_name.ends_with(".jsonl") {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => {
                *skipped += 1;
                continue;
            }
        };
        let started_at = metadata
            .modified()
            .ok()
            .map(chrono::DateTime::<chrono::Utc>::from);
        let cwd = peek_cwd(&path);
        sessions.push(SessionRef {
            source: SOURCE_CODEX,
            path,
            project: None,
            cwd,
            started_at,
            size_bytes: metadata.len(),
        });
    }
}

/// Cheap discovery-time peek at a session file's true working directory:
/// parses each line as JSON in turn and stops at the first `session_meta`
/// record carrying a `payload.cwd` field, skipping the full parse of the
/// file's events. Reads the file the same way `load_session` does
/// (`std::fs::read` then `String::from_utf8_lossy`), so an invalid-UTF-8
/// line elsewhere in the file does not abort the scan before it reaches a
/// later cwd-bearing line. `load_session` never errors on bad UTF-8 either,
/// so peek and load must tolerate it identically, or `--project` filtering
/// can silently disagree with what `submit_sessions` actually delivers.
/// Mirrors the exact field path `load_session` uses
/// (`payload.and_then(|p| p.get("cwd"))`). Returns `None` if the file
/// cannot be read or no record carries `cwd`.
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

    Ok(SessionTranscript {
        source: SOURCE_CODEX,
        agent_version,
        model,
        project,
        cwd,
        started_at,
        session_hash: hash,
        events,
    })
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
            });
        }
        other => {
            events.push(SessionEvent {
                kind: SessionEventKind::Opaque,
                timestamp,
                content: None,
                structured: json!({ "record_type": other }),
                tool_name: None,
                token_counts: None,
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

    #[test]
    fn discovers_nested_rollout_files() {
        let src = CodexSource::new(fixture_root());
        let found = src.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, "codex");
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
                SessionEventKind::Opaque, // reasoning
                SessionEventKind::ToolCall,
                SessionEventKind::ToolResult,
                SessionEventKind::Assistant,
            ]
        );
        assert_eq!(t.events[2].tool_name.as_deref(), Some("shell"));
        assert_eq!(t.events[2].structured["command"], "ls src/");
        // Reasoning summary text must not survive.
        assert!(!format!("{:?}", t.events).contains("thinking about it"));
    }
}
