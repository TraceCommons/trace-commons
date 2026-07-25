//! Source model: the `TraceSource` trait, session/transcript types shared by
//! per-agent adapters (Tasks 7-8), and deterministic hashing/id helpers.

use std::borrow::Cow;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

pub mod claude_code;
pub mod codex;

pub const SOURCE_CLAUDE_CODE: &str = "claude-code";
pub const SOURCE_CODEX: &str = "codex";

#[derive(Debug, Clone)]
pub struct SessionRef {
    pub source: &'static str,
    pub path: PathBuf,
    pub project: Option<String>, // basename only, never a full path
    pub cwd: Option<String>, // true working dir if cheaply known at discovery; used for --project matching, NEVER serialized
    pub started_at: Option<DateTime<Utc>>,
    pub size_bytes: u64,
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
}

pub trait TraceSource {
    fn name(&self) -> &'static str;
    fn discover(&self) -> anyhow::Result<Vec<SessionRef>>;
    fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript>;
}

/// Hash raw session bytes as "sha256:<hex>".
pub fn session_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex::encode(digest))
}

/// Deterministic submission id derived from the session hash string.
pub fn submission_id_for(session_hash: &str) -> uuid::Uuid {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, session_hash.as_bytes())
}

/// Construct the set of available `TraceSource` adapters, defaulting roots to
/// `~/.claude/projects` and `~/.codex/sessions` when not overridden.
pub fn all_sources(
    claude_root: Option<PathBuf>,
    codex_root: Option<PathBuf>,
) -> Vec<Box<dyn TraceSource>> {
    let claude_root = claude_root.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_default()
            .join(".claude/projects")
    });
    let codex_root =
        codex_root.unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".codex/sessions"));
    let sources: Vec<Box<dyn TraceSource>> = vec![
        Box::new(claude_code::ClaudeCodeSource::new(claude_root)),
        Box::new(codex::CodexSource::new(codex_root)),
    ];
    sources
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
