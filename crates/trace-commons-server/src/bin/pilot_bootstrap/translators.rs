//! Per-dataset translators that map one HuggingFace session (a single
//! `.jsonl` file = a sequence of agent events) to a [`SubmissionDraft`].
//!
//! Session shape (authoritative, observed on disk for all three target
//! datasets — `jedisct1/agent-traces-swival`, `badlogicgames/pi-mono`,
//! `TeichAI/DeepSeek-v4-Pro-Agent`):
//!
//! ```text
//! {"type":"session","id":"<uuid>","timestamp":"...","cwd":"..."}
//! {"type":"model_change", ...}
//! {"type":"message","id":"...","parentId":"...","timestamp":"...",
//!  "message":{"role":"user|assistant|developer",
//!             "content":"<string>"
//!                       | [ {"type":"text","text":"..."}
//!                         | {"type":"thinking","thinking":"..."}
//!                         | ... ]}}
//! ...
//! ```
//!
//! Each translator concatenates the textual content of every event in the
//! session into a single trace body. We recognize three chunk types:
//! plain-string `content`, `{type:"text", text:"..."}` chunks, and
//! `{type:"thinking", thinking:"..."}` chunks. Reasoning/thinking text is
//! a first-class part of the trace body for these agent-traces datasets.
//!
//! Divergence note: the HuggingFace datasets-server preview API
//! normalizes these JSONL files into a single per-session row with a
//! `prompt` field and a `traces` array, but that schema does NOT match
//! the on-disk repo layout. The repo ships one `.jsonl` file per session
//! with events streamed one-row-per-line, and that is the shape we
//! parse. See `docs/operator/pilot-bootstrap-dryrun-notes.md` for the
//! post-mortem of the earlier parquet-shaped loader.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Per-translator body cap so envelopes stay well under the gate-service
/// request body limit. Long sessions get truncated rather than dropped.
pub const SESSION_BODY_CAP: usize = 16_000;

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
    /// The earliest timestamp found on a session event, if the source
    /// carried one. Every event in a session gets flattened into a single
    /// trace body and a single recorded step (see `flatten_session`), so
    /// this is the one real timestamp available for that step -- never a
    /// synthesised or interpolated one. `None` when no event in the
    /// session had a parseable `timestamp` field.
    pub session_timestamp: Option<DateTime<Utc>>,
}

/// Per-dataset translator contract. Translators are intentionally small and
/// stateless so the submitter loop can swap them on a single CLI flag.
///
/// Input is the raw bytes of one `.jsonl` session file; output is one
/// submission draft (or an error if the session yields no usable content).
pub trait Translator: Send + Sync {
    #[allow(dead_code)] // logged by the pilot-bootstrap binary; test target only invokes `translate`
    fn name(&self) -> &str;
    fn translate(&self, session_name: &str, session_bytes: &[u8]) -> Result<SubmissionDraft>;
}

/// Compute a deterministic submission id from the trace body. The first 32
/// hex chars (128 bits) of SHA-256 fit comfortably into a UUID v4 wire
/// format and give us idempotency without leaking the body.
pub fn submission_id_from_body(body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..16])
}

/// Pull every non-empty text snippet out of one event row. Looks at
/// `message.content` (string OR list-of-chunks with `text` or `thinking`
/// fields) and the top-level `content` field. Returns trimmed snippets in
/// observed order; the caller joins them with blank lines so they read
/// as one trace.
///
/// Direct port of `_extract_event_text` in
/// `scripts/operator/build-agent-traces-corpus.py`, extended to recognize
/// `{type:"thinking", thinking:"..."}` chunks observed in pi-mono and
/// DeepSeek assistant turns.
fn extract_event_text(event: &Value) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();

    if let Some(msg) = event.get("message") {
        if let Some(s) = msg.get("content").and_then(Value::as_str) {
            let t = s.trim();
            if !t.is_empty() {
                parts.push(t.to_string());
            }
        } else if let Some(arr) = msg.get("content").and_then(Value::as_array) {
            for chunk in arr {
                let candidates = [
                    chunk.get("text").and_then(Value::as_str),
                    chunk.get("thinking").and_then(Value::as_str),
                ];
                for cand in candidates.iter().flatten() {
                    let trimmed = cand.trim();
                    if !trimmed.is_empty() {
                        parts.push(trimmed.to_string());
                    }
                }
            }
        }
    }

    if let Some(s) = event.get("content").and_then(Value::as_str) {
        let t = s.trim();
        if !t.is_empty() {
            parts.push(t.to_string());
        }
    }

    parts
}

/// Concatenate every event's content fields into one trace body. Lines that
/// fail to parse as JSON are skipped silently (per hash-only logging
/// convention; malformed event rows do happen in the wild and are not
/// operator-actionable). Returns the concatenated body, the first observed
/// `sessionId` (or session `id`) for `source_row_id`, and the earliest
/// parseable per-event `timestamp`, if any.
fn flatten_session(session_bytes: &[u8]) -> (String, Option<String>, Option<DateTime<Utc>>) {
    let mut parts: Vec<String> = Vec::new();
    let mut session_id: Option<String> = None;
    let mut session_timestamp: Option<DateTime<Utc>> = None;

    for line in session_bytes.split(|b| *b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let event: Value = match serde_json::from_slice(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if !event.is_object() {
            continue;
        }
        if session_id.is_none() {
            let candidate = event
                .get("sessionId")
                .or_else(|| event.get("id"))
                .and_then(Value::as_str);
            if let Some(s) = candidate {
                if !s.is_empty() {
                    session_id = Some(s.to_string());
                }
            }
        }
        if session_timestamp.is_none() {
            if let Some(raw) = event.get("timestamp").and_then(Value::as_str) {
                if let Ok(parsed) = DateTime::parse_from_rfc3339(raw) {
                    session_timestamp = Some(parsed.with_timezone(&Utc));
                }
            }
        }
        parts.extend(extract_event_text(&event));
    }

    (parts.join("\n\n"), session_id, session_timestamp)
}

/// UTF-8-safe character truncation.
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

/// Strip a trailing `.jsonl` (or `.json`) suffix and any leading path
/// segments from a sibling filename, producing a stable short row id.
fn row_id_from_sibling(sibling: &str) -> String {
    let base = sibling.rsplit('/').next().unwrap_or(sibling);
    base.strip_suffix(".jsonl")
        .or_else(|| base.strip_suffix(".json"))
        .unwrap_or(base)
        .to_string()
}

/// Shared session-concat translator. Swival/PiMono/DeepSeek all share the
/// same on-disk shape; only `source_dataset` and `source_domain_tag`
/// differ. Wrappers below pick the labels.
struct SessionConcatTranslator {
    #[allow(dead_code)]
    // read via `Translator::name` from the pilot-bootstrap binary; test target only invokes `translate`
    name: &'static str,
    source_dataset: &'static str,
    source_domain_tag: &'static str,
}

impl SessionConcatTranslator {
    fn translate_session(
        &self,
        session_name: &str,
        session_bytes: &[u8],
    ) -> Result<SubmissionDraft> {
        let (body, session_id, session_timestamp) = flatten_session(session_bytes);
        if body.is_empty() {
            anyhow::bail!("session yielded no textual content");
        }
        let body = truncate_chars(&body, SESSION_BODY_CAP);
        let id = submission_id_from_body(&body);
        let row_id = session_id.unwrap_or_else(|| row_id_from_sibling(session_name));
        Ok(SubmissionDraft {
            submission_id: id,
            trace_body: body,
            source_dataset: self.source_dataset.to_string(),
            source_row_id: row_id,
            source_domain_tag: self.source_domain_tag.to_string(),
            session_timestamp,
        })
    }
}

/// `jedisct1/agent-traces-swival` — agent traces from the Swival harness.
/// One `.jsonl` file = one session = one trace.
pub struct SwivalTranslator(SessionConcatTranslator);

impl SwivalTranslator {
    pub fn new() -> Self {
        Self(SessionConcatTranslator {
            name: "swival",
            source_dataset: "jedisct1/agent-traces-swival",
            source_domain_tag: "agent-traces/swival",
        })
    }
}

impl Default for SwivalTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl Translator for SwivalTranslator {
    fn name(&self) -> &str {
        self.0.name
    }
    fn translate(&self, session_name: &str, session_bytes: &[u8]) -> Result<SubmissionDraft> {
        self.0.translate_session(session_name, session_bytes)
    }
}

/// `badlogicgames/pi-mono` — agent traces from the pi-mono coding harness.
/// Same on-disk shape as swival: one `.jsonl` file per session, events
/// carry `message.content`. The earlier `messages`/`session_id` flat-row
/// schema does not exist on disk.
pub struct PiMonoTranslator(SessionConcatTranslator);

impl PiMonoTranslator {
    pub fn new() -> Self {
        Self(SessionConcatTranslator {
            name: "pi-mono",
            source_dataset: "badlogicgames/pi-mono",
            source_domain_tag: "agent-traces/pi-mono",
        })
    }
}

impl Default for PiMonoTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl Translator for PiMonoTranslator {
    fn name(&self) -> &str {
        self.0.name
    }
    fn translate(&self, session_name: &str, session_bytes: &[u8]) -> Result<SubmissionDraft> {
        self.0.translate_session(session_name, session_bytes)
    }
}

/// `TeichAI/DeepSeek-v4-Pro-Agent` — agent traces from the DeepSeek-v4
/// agent harness. Same pi-shaped on-disk layout as swival/pi-mono.
pub struct DeepSeekAgentTranslator(SessionConcatTranslator);

impl DeepSeekAgentTranslator {
    pub fn new() -> Self {
        Self(SessionConcatTranslator {
            name: "deepseek-agent",
            source_dataset: "TeichAI/DeepSeek-v4-Pro-Agent",
            source_domain_tag: "agent-traces/deepseek-v4-pro",
        })
    }
}

impl Default for DeepSeekAgentTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl Translator for DeepSeekAgentTranslator {
    fn name(&self) -> &str {
        self.0.name
    }
    fn translate(&self, session_name: &str, session_bytes: &[u8]) -> Result<SubmissionDraft> {
        self.0.translate_session(session_name, session_bytes)
    }
}

/// Construct a translator by short name.
#[allow(dead_code)] // called by the pilot-bootstrap binary; test target constructs concrete translators directly
pub fn translator_by_name(name: &str) -> Result<Box<dyn Translator>> {
    match name {
        "swival" => Ok(Box::new(SwivalTranslator::new())),
        "pi-mono" => Ok(Box::new(PiMonoTranslator::new())),
        "deepseek-agent" => Ok(Box::new(DeepSeekAgentTranslator::new())),
        other => Err(anyhow::anyhow!("unknown translator: {other}")),
    }
}

/// Word-count gate. Sessions with too few words yield a low-signal trace;
/// sessions with too many words inflate the envelope past the gate-service
/// body cap. Matches the corpus builder's 200..=2000-word default.
pub fn passes_word_filter(body: &str, min_words: usize, max_words: usize) -> bool {
    let n = body.split_whitespace().count();
    n >= min_words && n <= max_words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session(events: &[serde_json::Value]) -> Vec<u8> {
        let mut out = Vec::new();
        for e in events {
            out.extend_from_slice(e.to_string().as_bytes());
            out.push(b'\n');
        }
        out
    }

    #[test]
    fn swival_translator_concats_session_content_in_order() {
        let t = SwivalTranslator::new();
        let bytes = make_session(&[
            serde_json::json!({"type":"session","id":"sess-1"}),
            serde_json::json!({
                "type":"message","id":"a","parentId":null,
                "message":{"role":"user","content":[
                    {"type":"text","text":"first user text"}
                ]}
            }),
            serde_json::json!({
                "type":"message","id":"b","parentId":"a",
                "message":{"role":"assistant","content":"second assistant text"}
            }),
            serde_json::json!({"type":"system","content":"third system text"}),
        ]);
        let draft = t.translate("session-1.jsonl", &bytes).unwrap();
        let expected = "first user text\n\nsecond assistant text\n\nthird system text";
        assert_eq!(draft.trace_body, expected);
        assert_eq!(draft.source_dataset, "jedisct1/agent-traces-swival");
        assert_eq!(draft.source_row_id, "sess-1");
        assert_eq!(draft.source_domain_tag, "agent-traces/swival");
        assert_eq!(
            draft.submission_id,
            submission_id_from_body(&draft.trace_body)
        );
    }

    #[test]
    fn swival_translator_handles_empty_session() {
        let t = SwivalTranslator::new();
        let bytes = make_session(&[
            serde_json::json!({"type":"model_change","id":"a","provider":"x"}),
            serde_json::json!({"type":"thinking_level_change","id":"b"}),
        ]);
        let err = t.translate("empty.jsonl", &bytes).expect_err("err");
        assert!(
            err.to_string().contains("no textual content"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn swival_translator_skips_malformed_lines() {
        let t = SwivalTranslator::new();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"not json\n");
        bytes.extend_from_slice(b"{broken\n");
        bytes.extend_from_slice(
            serde_json::json!({"type":"m","content":"good content"})
                .to_string()
                .as_bytes(),
        );
        bytes.push(b'\n');
        let draft = t.translate("mixed.jsonl", &bytes).unwrap();
        assert_eq!(draft.trace_body, "good content");
    }

    #[test]
    fn submission_id_is_stable_across_runs() {
        let t = SwivalTranslator::new();
        let bytes = make_session(&[
            serde_json::json!({"type":"session","id":"S"}),
            serde_json::json!({
                "type":"message",
                "message":{"role":"user","content":"hello"}
            }),
            serde_json::json!({
                "type":"message",
                "message":{"role":"assistant","content":"world"}
            }),
        ]);
        let a = t.translate("S.jsonl", &bytes).unwrap();
        let b = t.translate("S.jsonl", &bytes).unwrap();
        assert_eq!(a.submission_id, b.submission_id);
        let c = t.translate("different-name.jsonl", &bytes).unwrap();
        assert_eq!(a.submission_id, c.submission_id);
    }

    #[test]
    fn pi_mono_and_deepseek_translators_use_same_session_shape() {
        let bytes = make_session(&[
            serde_json::json!({"type":"session","id":"pm-1"}),
            serde_json::json!({
                "type":"message",
                "message":{"role":"user","content":"pi-mono prompt"}
            }),
        ]);
        let pm = PiMonoTranslator::new()
            .translate("pm.jsonl", &bytes)
            .unwrap();
        assert_eq!(pm.trace_body, "pi-mono prompt");
        assert_eq!(pm.source_dataset, "badlogicgames/pi-mono");

        let ds = DeepSeekAgentTranslator::new()
            .translate("ds.jsonl", &bytes)
            .unwrap();
        assert_eq!(ds.trace_body, "pi-mono prompt");
        assert_eq!(ds.source_dataset, "TeichAI/DeepSeek-v4-Pro-Agent");
    }

    #[test]
    fn translator_extracts_thinking_chunks_alongside_text() {
        let t = SwivalTranslator::new();
        let bytes = make_session(&[serde_json::json!({
            "type":"message",
            "message":{"role":"assistant","content":[
                {"type":"thinking","thinking":"reasoning step"},
                {"type":"text","text":"final answer"}
            ]}
        })]);
        let draft = t.translate("t.jsonl", &bytes).unwrap();
        assert!(draft.trace_body.contains("reasoning step"));
        assert!(draft.trace_body.contains("final answer"));
    }

    #[test]
    fn row_id_falls_back_to_sibling_basename_when_no_session_id() {
        let t = SwivalTranslator::new();
        let bytes = make_session(&[serde_json::json!({
            "type":"message",
            "message":{"role":"user","content":"hi"}
        })]);
        let draft = t.translate("subdir/2026-01-16T_abc.jsonl", &bytes).unwrap();
        assert_eq!(draft.source_row_id, "2026-01-16T_abc");
    }

    #[test]
    fn word_filter_bounds_inclusive() {
        assert!(passes_word_filter("a b c", 1, 3));
        assert!(!passes_word_filter("a b c d", 1, 3));
        assert!(!passes_word_filter("", 1, 3));
    }
}
