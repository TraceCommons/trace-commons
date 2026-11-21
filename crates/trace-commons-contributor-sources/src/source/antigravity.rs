//! Antigravity session adapter.
//!
//! Reads `<brain_root>/<conversation-id>/.system_generated/logs/transcript_full.jsonl`
//! (falling back to `transcript.jsonl` when the full log is absent), where
//! `<brain_root>` is `$ANTIGRAVITY_HOME/brain` or one of the conventional
//! Antigravity stores under `~/.gemini/`:
//! - `~/.gemini/antigravity-cli/brain` (Antigravity CLI)
//! - `~/.gemini/antigravity-ide/brain` (Antigravity IDE)
//! - `~/.gemini/antigravity/brain` (Antigravity App)
//!
//! Each file is JSONL (one step object per line) and is streamed line-by-line
//! up to [`super::MAX_SESSION_BYTES`]. Step-type dispatch is tolerant (unknown
//! step types become `Opaque` events); path containment, symlink refusal, the
//! byte budget, and validating that the file is an Antigravity transcript stay
//! fail-closed.

use std::borrow::Cow;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{
    MAX_SESSION_BYTES, SOURCE_ANTIGRAVITY, ServedBy, SessionEvent, SessionEventKind, SessionRef,
    SessionTooLarge, SessionTranscript, TraceSource, real_file_within_root,
};

/// Optional environment variable relocating the Antigravity appDataDir.
pub const ANTIGRAVITY_HOME_ENV: &str = "ANTIGRAVITY_HOME";

/// Subdirectory under each Antigravity appDataDir holding one directory per conversation.
pub const BRAIN_SUBDIR: &str = "brain";

/// Per-conversation subdirectory chain holding transcript JSONL logs.
pub(crate) const SYSTEM_GENERATED_DIR: &str = ".system_generated";
pub(crate) const LOGS_DIR: &str = "logs";

/// Preferred untruncated transcript log file name.
pub(crate) const TRANSCRIPT_FULL_FILE: &str = "transcript_full.jsonl";

/// Compact transcript log file name (used when `transcript_full.jsonl` is absent).
pub(crate) const TRANSCRIPT_COMPACT_FILE: &str = "transcript.jsonl";

/// Conventional Antigravity appDataDir names under `~/.gemini/`.
const CONVENTIONAL_APP_DIRS: &[&str] = &["antigravity-cli", "antigravity-ide", "antigravity"];

/// All conventional Antigravity brain roots on a machine given `home` and `env`.
///
/// When `ANTIGRAVITY_HOME` is set and non-empty, only `$ANTIGRAVITY_HOME/brain`
/// is returned. Otherwise all three `~/.gemini/antigravity*/brain` directories
/// (CLI, IDE, App) are returned in priority order.
pub fn conventional_roots(home: &Path, env: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    if let Some(relocated) = env(ANTIGRAVITY_HOME_ENV).filter(|v| !v.is_empty()) {
        return vec![PathBuf::from(relocated).join(BRAIN_SUBDIR)];
    }
    let gemini = home.join(".gemini");
    CONVENTIONAL_APP_DIRS
        .iter()
        .map(|dir| gemini.join(dir).join(BRAIN_SUBDIR))
        .collect()
}

/// The primary conventional root to display for this machine: the first
/// existing store among [`conventional_roots`], or `~/.gemini/antigravity-cli/brain`
/// when none exist yet.
pub fn conventional_root(home: &Path, env: impl Fn(&str) -> Option<String>) -> PathBuf {
    let roots = conventional_roots(home, env);
    roots
        .iter()
        .find(|p| p.is_dir())
        .cloned()
        .unwrap_or_else(|| {
            roots
                .into_iter()
                .next()
                .unwrap_or_else(|| home.join(".gemini/antigravity-cli/brain"))
        })
}

/// All conventional roots resolved against this machine's real home and env.
pub fn conventional_roots_this_machine() -> Vec<PathBuf> {
    conventional_roots(&super::home_dir(), super::machine_env)
}

/// Primary conventional root resolved against this machine's real home and env.
pub fn conventional_root_this_machine() -> PathBuf {
    super::conventional_root_on_this_machine(conventional_root)
}

/// Whether `file_name` is an Antigravity transcript log file.
pub(crate) fn is_transcript_file_name(file_name: &str) -> bool {
    file_name == TRANSCRIPT_FULL_FILE || file_name == TRANSCRIPT_COMPACT_FILE
}

/// Returns the canonical transcript file inside `<conversation_dir>/.system_generated/logs/`
/// if one exists as a regular file (never following symlinks).
/// Prefers `transcript_full.jsonl` over `transcript.jsonl`.
pub(crate) fn canonical_transcript_in(conversation_dir: &Path) -> Option<PathBuf> {
    let sys_dir = conversation_dir.join(SYSTEM_GENERATED_DIR);
    if !std::fs::symlink_metadata(&sys_dir)
        .map(|m| m.file_type().is_dir())
        .unwrap_or(false)
    {
        return None;
    }
    let logs_dir = sys_dir.join(LOGS_DIR);
    if !std::fs::symlink_metadata(&logs_dir)
        .map(|m| m.file_type().is_dir())
        .unwrap_or(false)
    {
        return None;
    }
    for name in [TRANSCRIPT_FULL_FILE, TRANSCRIPT_COMPACT_FILE] {
        let candidate = logs_dir.join(name);
        if std::fs::symlink_metadata(&candidate)
            .map(|m| m.file_type().is_file())
            .unwrap_or(false)
        {
            return Some(candidate);
        }
    }
    None
}

pub struct AntigravitySource {
    root: PathBuf,
}

impl AntigravitySource {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Builds a source for a declared path. When `path` matches one of this
    /// machine's conventional Antigravity brain directories (and tests are not
    /// isolating home), all conventional Antigravity brain directories (CLI,
    /// IDE, App) are watched together under the single `antigravity` adapter.
    /// Any custom folder watches only that exact folder.
    pub(super) fn for_declared_path(path: PathBuf) -> Box<dyn TraceSource> {
        let conventional = conventional_roots_this_machine();
        if conventional.len() > 1 && conventional.iter().any(|root| root == &path) {
            let sources: Vec<Box<dyn TraceSource>> = conventional
                .into_iter()
                .map(|r| Box::new(Self::new(r)) as Box<dyn TraceSource>)
                .collect();
            return Box::new(super::multi_root::MultiRootSource::new(
                SOURCE_ANTIGRAVITY,
                sources,
            ));
        }
        Box::new(Self::new(path))
    }
}

fn conversation_id_for(transcript_path: &Path) -> Option<String> {
    // <root>/<conversation-id>/.system_generated/logs/<transcript*.jsonl>
    let logs = transcript_path.parent()?;
    if logs.file_name()? != LOGS_DIR {
        return None;
    }
    let sys = logs.parent()?;
    if sys.file_name()? != SYSTEM_GENERATED_DIR {
        return None;
    }
    let conv_dir = sys.parent()?;
    let name = conv_dir.file_name()?.to_str()?;
    (!name.is_empty()).then(|| name.to_string())
}

fn session_ref_for(path: PathBuf) -> Option<SessionRef> {
    let metadata = std::fs::metadata(&path).ok()?;
    let started_at = metadata
        .modified()
        .ok()
        .map(DateTime::<Utc>::from);
    Some(SessionRef {
        source: SOURCE_ANTIGRAVITY,
        declared_source: None,
        path,
        project: None,
        cwd: None,
        started_at,
        size_bytes: metadata.len(),
        group_modified_at: None,
        group_member_count: 0,
    })
}

impl TraceSource for AntigravitySource {
    fn name(&self) -> &'static str {
        SOURCE_ANTIGRAVITY
    }

    fn discover_unopened(&self) -> anyhow::Result<Vec<SessionRef>> {
        self.discover()
    }

    fn discover(&self) -> anyhow::Result<Vec<SessionRef>> {
        let mut sessions = Vec::new();
        let Some(entries) = super::read_dir_for_discovery(&self.root)? else {
            return Ok(sessions);
        };
        for entry in entries {
            let Ok(entry) = entry else { continue };
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => {}
                _ => continue,
            }
            if let Some(transcript) = canonical_transcript_in(&entry.path()) {
                if let Some(r) = session_ref_for(transcript) {
                    sessions.push(r);
                }
            }
        }
        sessions.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(sessions)
    }

    fn session_for_path(&self, path: &Path) -> Option<PathBuf> {
        let real = real_file_within_root(&self.root, path)?;
        let file_name = real.file_name()?.to_str()?;
        if !is_transcript_file_name(file_name) {
            return None;
        }
        let logs = real.parent()?;
        if logs.file_name()? != LOGS_DIR {
            return None;
        }
        let sys = logs.parent()?;
        if sys.file_name()? != SYSTEM_GENERATED_DIR {
            return None;
        }
        let conv_dir = sys.parent()?;
        if conv_dir.parent()? != self.root.as_path() {
            return None;
        }
        // If watcher noticed `transcript.jsonl` but `transcript_full.jsonl`
        // exists alongside it, resolve to the canonical full file.
        if file_name == TRANSCRIPT_COMPACT_FILE {
            let full = logs.join(TRANSCRIPT_FULL_FILE);
            if std::fs::symlink_metadata(&full)
                .map(|m| m.file_type().is_file())
                .unwrap_or(false)
            {
                return Some(full);
            }
        }
        Some(real)
    }

    fn session_at(&self, path: &Path) -> anyhow::Result<Option<SessionRef>> {
        let Some(address) = self.session_for_path(path) else {
            return Ok(None);
        };
        Ok(session_ref_for(address))
    }

    fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript> {
        let real = real_file_within_root(&self.root, &r.path)
            .ok_or_else(|| anyhow!("antigravity-session-outside-root"))?;
        load_session(&real, r.cwd.clone())
    }
}

fn timestamp_of(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

fn non_empty_str(value: Option<&Value>) -> Option<String> {
    value
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn parse_tool_args(raw: Option<&Value>) -> Value {
    match raw {
        Some(Value::Object(map)) => {
            // Antigravity compact/full logs occasionally double-quote string
            // arg values (e.g. `"Path": "\"/abs/path\""`). Keep structured
            // object intact, unwrapping clean JSON-encoded string scalars where
            // appropriate so downstream redaction sees clean paths/commands.
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                let unwrapped = match v {
                    Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or_else(|_| v.clone()),
                    other => other.clone(),
                };
                out.insert(k.clone(), unwrapped);
            }
            Value::Object(out)
        }
        Some(other) => other.clone(),
        None => Value::Null,
    }
}

fn map_step(step: &Value, model: &mut Option<String>, events: &mut Vec<SessionEvent>) {
    let timestamp = timestamp_of(step.get("created_at"));
    let step_type = step.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let status = step.get("status").and_then(|v| v.as_str());

    match step_type {
        "USER_INPUT" => {
            events.push(SessionEvent {
                served_by: None,
                kind: SessionEventKind::User,
                timestamp,
                content: non_empty_str(step.get("content")),
                structured: Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: None,
                success: None,
            });
        }
        "PLANNER_RESPONSE" => {
            let step_model = non_empty_str(step.get("model"));
            if model.is_none() {
                if let Some(m) = step_model.as_ref() {
                    *model = Some(m.clone());
                }
            }
            let cache_read = step
                .get("cache_read_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .min(u32::MAX as u64) as u32;
            let served_by = step_model.map(|m| ServedBy {
                model: m,
                cache_read_tokens: cache_read,
                cache_write_5m_tokens: 0,
                cache_write_1h_tokens: 0,
            });
            let token_counts = match (
                step.get("input_tokens").and_then(|v| v.as_u64()),
                step.get("output_tokens").and_then(|v| v.as_u64()),
            ) {
                (Some(input), Some(output)) => Some((
                    input.min(u32::MAX as u64) as u32,
                    output.min(u32::MAX as u64) as u32,
                )),
                _ => None,
            };

            if let Some(thinking) = non_empty_str(step.get("thinking")) {
                events.push(SessionEvent {
                    served_by: None,
                    kind: SessionEventKind::Reasoning,
                    timestamp,
                    content: Some(thinking),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: None,
                    tool_call_id: None,
                    success: None,
                });
            }

            let mut usage_attached = false;
            if let Some(content) = non_empty_str(step.get("content")) {
                events.push(SessionEvent {
                    served_by: served_by.clone(),
                    kind: SessionEventKind::Assistant,
                    timestamp,
                    content: Some(content),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts,
                    tool_call_id: None,
                    success: None,
                });
                usage_attached = true;
            }

            if let Some(calls) = step.get("tool_calls").and_then(|v| v.as_array()) {
                for call in calls {
                    let attach = !usage_attached;
                    usage_attached = true;
                    events.push(SessionEvent {
                        served_by: if attach { served_by.clone() } else { None },
                        kind: SessionEventKind::ToolCall,
                        timestamp,
                        content: None,
                        structured: parse_tool_args(call.get("args")),
                        tool_name: non_empty_str(call.get("name")),
                        token_counts: if attach { token_counts } else { None },
                        tool_call_id: non_empty_str(call.get("id")),
                        success: None,
                    });
                }
            }
        }
        "GENERIC" => {
            let success = status.map(|s| s == "DONE");
            events.push(SessionEvent::tool_result(
                timestamp,
                non_empty_str(step.get("content")),
                None,
                success,
            ));
        }
        _ => {
            events.push(SessionEvent {
                served_by: None,
                kind: SessionEventKind::Opaque,
                timestamp,
                content: non_empty_str(step.get("content")),
                structured: serde_json::json!({ "type": step_type }),
                tool_name: None,
                token_counts: None,
                tool_call_id: None,
                success: None,
            });
        }
    }
}

fn load_session(path: &Path, cwd: Option<String>) -> anyhow::Result<SessionTranscript> {
    let declared = std::fs::metadata(path)?.len();
    if declared > MAX_SESSION_BYTES {
        return Err(SessionTooLarge {
            label: "antigravity-session-too-large",
            declared_bytes: declared,
            budget_bytes: MAX_SESSION_BYTES,
        }
        .into());
    }

    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut bytes_read: u64 = 0;
    let mut line_buf = Vec::new();
    let mut events = Vec::new();
    let mut model: Option<String> = None;
    let mut started_at: Option<DateTime<Utc>> = None;
    let mut saw_step = false;

    loop {
        line_buf.clear();
        let n = reader.read_until(b'\n', &mut line_buf)?;
        if n == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(n as u64);
        if bytes_read > MAX_SESSION_BYTES {
            return Err(SessionTooLarge {
                label: "antigravity-session-too-large",
                declared_bytes: bytes_read,
                budget_bytes: MAX_SESSION_BYTES,
            }
            .into());
        }
        hasher.update(&line_buf);
        let trimmed = std::str::from_utf8(&line_buf)
            .map_err(|_| anyhow!("malformed_antigravity_session"))?
            .trim();
        if trimmed.is_empty() {
            continue;
        }
        let step: Value =
            serde_json::from_str(trimmed).map_err(|_| anyhow!("malformed_antigravity_session"))?;
        let Some(obj) = step.as_object() else {
            return Err(anyhow!("malformed_antigravity_session"));
        };
        if !obj.contains_key("type") && !obj.contains_key("step_index") {
            return Err(anyhow!("malformed_antigravity_session"));
        }
        saw_step = true;
        if started_at.is_none() {
            started_at = timestamp_of(step.get("created_at"));
        }
        map_step(&step, &mut model, &mut events);
    }

    if !saw_step {
        return Err(anyhow!("empty_antigravity_session"));
    }

    let hash = format!("sha256:{:x}", hasher.finalize());
    let conversation_id = conversation_id_for(path);
    let source_session = conversation_id.as_ref().and_then(|id| {
        let identity = trace_commons_protocol::trace_contribution::SourceSessionIdentity {
            adapter: SOURCE_ANTIGRAVITY.to_string(),
            native_id: id.clone(),
        };
        trace_commons_protocol::trace_contribution::validate_source_session_identity(&identity)
            .ok()
            .map(|()| identity)
    });

    Ok(SessionTranscript {
        source: Cow::Borrowed(SOURCE_ANTIGRAVITY),
        agent_version: None,
        model,
        project: None,
        cwd,
        started_at,
        session_hash: hash,
        conversation_id,
        source_session,
        events,
        subagent_count: 0,
        subagents_dropped: 0,
        routing: Vec::new(),
        attested_call: None,
        attested_refusal: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_transcript(root: &Path, conv_id: &str, file: &str, body: &str) -> PathBuf {
        let logs = root
            .join(conv_id)
            .join(SYSTEM_GENERATED_DIR)
            .join(LOGS_DIR);
        fs::create_dir_all(&logs).unwrap();
        let path = logs.join(file);
        fs::write(&path, body).unwrap();
        path
    }

    fn sample_jsonl() -> &'static str {
        concat!(
            r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-19T02:06:58Z","content":"fix the bug"}"#,
            "\n",
            r#"{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-11-19T02:06:59Z","model":"gemini-4-neon-medium","input_tokens":1200,"cache_read_tokens":300,"output_tokens":80,"thinking":"Inspecting the file","tool_calls":[{"name":"read_file","args":{"Path":"\"/repo/lib.rs\""}}]}"#,
            "\n",
            r#"{"step_index":2,"source":"MODEL","type":"GENERIC","status":"DONE","created_at":"2026-11-19T02:07:00Z","content":"fn main() {}"}"#,
            "\n",
            r#"{"step_index":3,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-11-19T02:07:02Z","model":"gemini-4-neon-medium","input_tokens":1500,"cache_read_tokens":1200,"output_tokens":40,"content":"Fixed."}"#,
            "\n",
            r#"{"step_index":4,"source":"SYSTEM","type":"SYSTEM_MESSAGE","status":"DONE","created_at":"2026-11-19T02:07:03Z","content":"task finished"}"#,
            "\n",
        )
    }

    #[test]
    fn discovers_full_over_compact_and_loads_events() {
        let dir = tempdir().unwrap();
        let conv = "860f7a90-3ddf-4173-bd8a-99abcadfab60";
        let compact = write_transcript(dir.path(), conv, TRANSCRIPT_COMPACT_FILE, sample_jsonl());
        let full = write_transcript(dir.path(), conv, TRANSCRIPT_FULL_FILE, sample_jsonl());

        let source = AntigravitySource::new(dir.path().to_path_buf());
        let found = source.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, SOURCE_ANTIGRAVITY);
        assert_eq!(found[0].path, full);

        // Watching compact resolves to canonical full file.
        assert_eq!(source.session_for_path(&compact), Some(full));

        let transcript = source.load(&found[0]).unwrap();
        assert_eq!(transcript.source, SOURCE_ANTIGRAVITY);
        assert_eq!(transcript.model.as_deref(), Some("gemini-4-neon-medium"));
        assert_eq!(transcript.conversation_id.as_deref(), Some(conv));
        assert!(transcript.source_session.is_some());
        assert_eq!(transcript.events.len(), 6);
        assert_eq!(transcript.events[0].kind, SessionEventKind::User);
        assert_eq!(transcript.events[1].kind, SessionEventKind::Reasoning);
        assert_eq!(transcript.events[2].kind, SessionEventKind::ToolCall);
        assert_eq!(transcript.events[2].tool_name.as_deref(), Some("read_file"));
        assert_eq!(transcript.events[2].structured["Path"], "/repo/lib.rs");
        assert_eq!(transcript.events[2].token_counts, Some((1200, 80)));
        assert_eq!(
            transcript.events[2].served_by.as_ref().map(|s| s.cache_read_tokens),
            Some(300)
        );
        assert_eq!(transcript.events[3].kind, SessionEventKind::ToolResult);
        assert_eq!(transcript.events[3].success, Some(true));
        assert_eq!(transcript.events[4].kind, SessionEventKind::Assistant);
        assert_eq!(transcript.events[4].content.as_deref(), Some("Fixed."));
        assert_eq!(transcript.events[5].kind, SessionEventKind::Opaque);
    }

    #[test]
    fn falls_back_to_compact_when_full_absent() {
        let dir = tempdir().unwrap();
        let conv = "conv-compact-only";
        let compact = write_transcript(dir.path(), conv, TRANSCRIPT_COMPACT_FILE, sample_jsonl());

        let source = AntigravitySource::new(dir.path().to_path_buf());
        let found = source.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, compact);
    }

    #[test]
    fn refuses_malformed_and_empty_files() {
        let dir = tempdir().unwrap();
        let bad = write_transcript(dir.path(), "bad", TRANSCRIPT_FULL_FILE, "not json\n");
        let empty = write_transcript(dir.path(), "empty", TRANSCRIPT_FULL_FILE, "\n  \n");
        let source = AntigravitySource::new(dir.path().to_path_buf());

        let bad_ref = session_ref_for(bad).unwrap();
        let empty_ref = session_ref_for(empty).unwrap();
        assert!(source.load(&bad_ref).is_err());
        assert!(source.load(&empty_ref).is_err());
    }

    #[test]
    fn conventional_roots_include_cli_ide_and_app_or_env_override() {
        let home = Path::new("/home/dev");
        let defaults = conventional_roots(home, |_| None);
        assert_eq!(
            defaults,
            vec![
                PathBuf::from("/home/dev/.gemini/antigravity-cli/brain"),
                PathBuf::from("/home/dev/.gemini/antigravity-ide/brain"),
                PathBuf::from("/home/dev/.gemini/antigravity/brain"),
            ]
        );
        let overridden = conventional_roots(home, |k| {
            (k == ANTIGRAVITY_HOME_ENV).then(|| "/custom/agy".to_string())
        });
        assert_eq!(overridden, vec![PathBuf::from("/custom/agy/brain")]);
    }
}
