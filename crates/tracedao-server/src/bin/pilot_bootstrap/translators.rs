//! Per-dataset translators that map an HF dataset row to a
//! [`SubmissionDraft`]. Slice 3 ships the `SwivalTranslator` plus stubs for
//! `PiMonoTranslator` and `DeepSeekAgentTranslator`; Slice 5 fills the stubs in
//! and wires auto-detection from the `--source` argument.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::hf_dataset::Row;

/// Per-translator body cap so envelopes stay well under the gate-service
/// request body limit. Tuned to mirror the swival source_code cap times a
/// small constant; long messages get truncated rather than dropped.
pub const MULTI_TURN_BODY_CAP: usize = 8000;

/// Maximum number of `source_code` characters folded into the swival trace
/// body. The spec calls out a 2000-char cap so the resulting envelope stays
/// well under the gate-service's per-request body size.
pub const SWIVAL_SOURCE_CODE_CAP: usize = 2000;

/// Translator-neutral hand-off to the submitter. The deterministic
/// `submission_id` is the idempotency anchor — re-running against the same
/// dataset yields the same id, so the ingest server's
/// `read_submission_record` path collapses the retry to a no-op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionDraft {
    pub submission_id: String,
    pub trace_body: String,
    pub source_dataset: String,
    pub source_row_id: String,
    pub source_domain_tag: String,
}

/// Per-dataset translator contract. Translators are intentionally small and
/// stateless so the submitter loop can swap them on a single CLI flag.
pub trait Translator: Send + Sync {
    fn name(&self) -> &str;
    fn translate(&self, row: &Row) -> Result<SubmissionDraft>;
}

/// Compute a deterministic submission id from the trace body. The first 32 hex
/// chars (128 bits) of SHA-256 fit comfortably into a UUID v4 wire format and
/// give us idempotency without leaking the body.
pub fn submission_id_from_body(body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..16])
}

/// Swival = `jedisct1/agent-traces-swival`. The format is one row per security
/// audit finding with `title`, `severity`, `finding_type`, a `proof` array of
/// reasoning strings, a `fix_outline`, and a (potentially long) `source_code`
/// excerpt. We concatenate them into a body suitable for the gate-service.
pub struct SwivalTranslator;

impl SwivalTranslator {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SwivalTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl Translator for SwivalTranslator {
    fn name(&self) -> &str {
        "swival"
    }

    fn translate(&self, row: &Row) -> Result<SubmissionDraft> {
        let title = row.get_str("title").unwrap_or_default();
        let severity = row.get_str("severity").unwrap_or_default();
        let finding_type = row.get_str("finding_type").unwrap_or_default();
        let proof = row.get_array_strs("proof").join("\n");
        let fix_outline = row.get_str("fix_outline").unwrap_or_default();
        let source_code = row.get_str("source_code").unwrap_or_default();

        // Char-safe truncation (parquet rows are UTF-8).
        let mut truncated = String::new();
        for ch in source_code.chars() {
            if truncated.len() + ch.len_utf8() > SWIVAL_SOURCE_CODE_CAP {
                break;
            }
            truncated.push(ch);
        }

        let body = format!(
            "{title}\n\n{severity} {finding_type}\n\n{proof}\n\n{fix_outline}\n\n{truncated}"
        );
        let id = submission_id_from_body(&body);
        Ok(SubmissionDraft {
            submission_id: id,
            trace_body: body,
            source_dataset: "jedisct1/agent-traces-swival".into(),
            source_row_id: title.to_string(),
            source_domain_tag: format!("security-audit/{finding_type}"),
        })
    }
}

/// `badlogicgames/pi-mono` and the `cfahlgren1/pi-mono-*` mirrors expose a
/// tree-structured message log per row. Slice 3 stubs this; Slice 5 implements
/// the longest-session concatenation per the spec.
pub struct PiMonoTranslator;

impl PiMonoTranslator {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PiMonoTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl Translator for PiMonoTranslator {
    fn name(&self) -> &str {
        "pi-mono"
    }

    fn translate(&self, row: &Row) -> Result<SubmissionDraft> {
        let messages = row
            .get_value("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("pi-mono row missing `messages` array"))?;
        if messages.is_empty() {
            anyhow::bail!("pi-mono row has empty `messages` array");
        }

        let session_id = row.get_str("session_id").unwrap_or_default();
        let domain = row
            .get_str("task_type")
            .or_else(|| row.get_str("domain"))
            .unwrap_or("coding-session");

        let body = longest_session_text(messages)?;
        let body = truncate_chars(&body, MULTI_TURN_BODY_CAP);
        let id = submission_id_from_body(&body);
        Ok(SubmissionDraft {
            submission_id: id,
            trace_body: body,
            source_dataset: "badlogicgames/pi-mono".into(),
            source_row_id: session_id.to_string(),
            source_domain_tag: format!("coding-session/{domain}"),
        })
    }
}

/// Walk the pi-mono message tree (each message has an `id` and optional
/// `parentId`), find the longest top-level chain, and concatenate the
/// `content` (or `text`) field of each message in chain order.
fn longest_session_text(messages: &[Value]) -> Result<String> {
    let mut by_id: HashMap<String, &Value> = HashMap::new();
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    let mut roots: Vec<String> = Vec::new();

    for msg in messages {
        let id = msg
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("msg-{}", by_id.len()));
        by_id.insert(id.clone(), msg);
        let parent = msg
            .get("parentId")
            .or_else(|| msg.get("parent_id"))
            .and_then(Value::as_str);
        match parent {
            Some(p) if !p.is_empty() => {
                children.entry(p.to_string()).or_default().push(id);
            }
            _ => roots.push(id),
        }
    }
    if roots.is_empty() {
        // No parent linkage — fall back to linear concatenation.
        return Ok(messages
            .iter()
            .filter_map(message_text)
            .collect::<Vec<_>>()
            .join("\n\n"));
    }

    let mut best: Vec<String> = Vec::new();
    for root in &roots {
        let chain = deepest_chain_from(root, &children);
        if chain.len() > best.len() {
            best = chain;
        }
    }
    let body = best
        .iter()
        .filter_map(|id| by_id.get(id).copied().and_then(message_text))
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(body)
}

fn deepest_chain_from(start: &str, children: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut best = vec![start.to_string()];
    if let Some(kids) = children.get(start) {
        for kid in kids {
            let mut sub = deepest_chain_from(kid, children);
            sub.insert(0, start.to_string());
            // Strip duplicated `start` from the recursion result.
            let extended: Vec<String> = std::iter::once(start.to_string())
                .chain(deepest_chain_from(kid, children).into_iter())
                .collect();
            let _ = sub;
            if extended.len() > best.len() {
                best = extended;
            }
        }
    }
    best
}

fn message_text(msg: &Value) -> Option<String> {
    if let Some(content) = msg.get("content").and_then(Value::as_str) {
        return Some(content.to_string());
    }
    if let Some(text) = msg.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(arr) = msg.get("content").and_then(Value::as_array) {
        let joined: String = arr
            .iter()
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    None
}

fn truncate_chars(s: &str, cap: usize) -> String {
    let mut out = String::with_capacity(s.len().min(cap));
    for ch in s.chars() {
        if out.len() + ch.len_utf8() > cap {
            break;
        }
        out.push(ch);
    }
    out
}

/// `TeichAI/DeepSeek-v4-Pro-Agent` exposes message-stream rows with
/// `role`/`content` arrays. Slice 5 implements the assistant-text concatenation
/// per the spec.
pub struct DeepSeekAgentTranslator;

impl DeepSeekAgentTranslator {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DeepSeekAgentTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl Translator for DeepSeekAgentTranslator {
    fn name(&self) -> &str {
        "deepseek-agent"
    }

    fn translate(&self, row: &Row) -> Result<SubmissionDraft> {
        let messages = row
            .get_value("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("deepseek row missing `messages` array"))?;
        if messages.is_empty() {
            anyhow::bail!("deepseek row has empty `messages` array");
        }

        let mut chunks: Vec<String> = Vec::new();
        for msg in messages {
            let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
            if role != "assistant" {
                continue;
            }
            if let Some(text) = msg.get("content").and_then(Value::as_str) {
                chunks.push(text.to_string());
                continue;
            }
            if let Some(arr) = msg.get("content").and_then(Value::as_array) {
                for part in arr {
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        chunks.push(text.to_string());
                    }
                }
            }
        }
        if chunks.is_empty() {
            anyhow::bail!("deepseek row produced no assistant text");
        }
        let body = truncate_chars(&chunks.join("\n\n"), MULTI_TURN_BODY_CAP);
        let id = submission_id_from_body(&body);
        let row_id = row
            .get_str("session_id")
            .or_else(|| row.get_str("id"))
            .unwrap_or_default()
            .to_string();
        let domain = row
            .get_str("domain")
            .or_else(|| row.get_str("task_type"))
            .unwrap_or("agent-reasoning");
        Ok(SubmissionDraft {
            submission_id: id,
            trace_body: body,
            source_dataset: "TeichAI/DeepSeek-v4-Pro-Agent".into(),
            source_row_id: row_id,
            source_domain_tag: format!("agent-reasoning/{domain}"),
        })
    }
}

/// Construct a translator by short name. Slice 3 wires the swival path; Slice
/// 5 fills the stubs and adds dataset-id auto-detection.
pub fn translator_by_name(name: &str) -> Result<Box<dyn Translator>> {
    match name {
        "swival" => Ok(Box::new(SwivalTranslator::new())),
        "pi-mono" => Ok(Box::new(PiMonoTranslator::new())),
        "deepseek-agent" => Ok(Box::new(DeepSeekAgentTranslator::new())),
        other => Err(anyhow::anyhow!("unknown translator: {other}"))
            .context("translator_by_name"),
    }
}
