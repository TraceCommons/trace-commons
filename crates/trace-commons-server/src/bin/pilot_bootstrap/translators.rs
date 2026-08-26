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

/// Per-translator text budget, in characters. Long sessions get truncated
/// rather than dropped.
///
/// Measured, not guessed. The ceiling this has to respect is
/// `MAX_TRACE_ENVELOPE_BYTES` (16,000,000). Across 35 real sessions pulled
/// from the three target datasets, a session's redacted envelope came out at
/// roughly 1.1x its body character count plus ~18 KB of fixed metadata, so a
/// 1,000,000-character text budget plus the equal `SESSION_TOOL_PAYLOAD_CAP`
/// lands around 2.4 MB -- a ~6x margin on real content, and still inside the
/// limit with a ~2x margin under the pathological case where every character
/// is a 4-byte one.
///
/// The previous value was 16,000, which was 0.1% of the limit it cited. It
/// kept 3% of the largest sampled session (475,268 extractable characters)
/// and truncated 43% of sampled sessions. Raising it changes `trace_body`,
/// and therefore `submission_id`, for every session that used to be
/// truncated -- see the idempotency note in
/// `docs/operator/pilot-bootstrap.md` before re-running the loader over a
/// dataset that has already been ingested.
pub const SESSION_BODY_CAP: usize = 1_000_000;

/// Separate budget, in characters, for tool-call arguments and tool-result
/// content across one session. Kept separate from `SESSION_BODY_CAP` so that
/// admitting tool payloads can never displace prose that already reached the
/// envelope: the text budget behaves exactly as it did before tool events
/// existed. Largest single payload observed in the sampled real sessions was
/// 51,310 characters and the largest per-session total was 469,769, so this
/// budget is not reached by real data; it exists to bound the envelope
/// against a pathological session rather than to shape normal ones.
pub const SESSION_TOOL_PAYLOAD_CAP: usize = 1_000_000;

/// Translator-neutral hand-off to the submitter. The deterministic
/// `submission_id` is the idempotency anchor — re-running against the same
/// dataset yields the same id, so the ingest server's
/// `read_submission_record` path collapses the retry to a no-op.
#[derive(Debug, Clone, PartialEq)]
pub struct SubmissionDraft {
    pub submission_id: String,
    /// The whole session flattened to one string, in event order. Still
    /// drives `submission_id` and `passes_word_filter` unchanged from
    /// before this field existed -- those only need the text, not the
    /// per-event structure. What the submitter actually sends as steps is
    /// `session_events`, not this.
    pub trace_body: String,
    pub source_dataset: String,
    pub source_row_id: String,
    pub source_domain_tag: String,
    /// The same session, kept as one entry per event instead of flattened,
    /// each with its own real timestamp where the source line had one
    /// (never synthesised) and a role classified only where unambiguous.
    /// The submitter turns each entry into its own `TraceStep`, so a
    /// multi-turn session produces a multi-step trace instead of
    /// collapsing into one instant. Bounded by the same `SESSION_BODY_CAP`
    /// character budget as `trace_body` -- see `cap_events`.
    pub session_events: Vec<SessionEvent>,
}

/// A session event's role, classified only when unambiguous. Everything
/// that is not clearly `message.role == "user"` or `"assistant"` -- system
/// prompts, developer prompts, tool calls/results, and anything else --
/// stays `Other` rather than being guessed at. `Other` maps to the same
/// catch-all (`TraceResponse::UserInput`) the whole-session flatten used
/// for every event before this change, so nothing that used to appear in
/// the trace now goes missing for lack of a role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEventRole {
    User,
    Assistant,
    Other,
}

/// One classified session event: its flattened text, its own real
/// timestamp if the source line carried one, its role, and the tool
/// activity the record carried, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionEvent {
    pub text: String,
    pub timestamp: Option<DateTime<Utc>>,
    pub role: SessionEventRole,
    /// Tool calls or tool results parsed out of this record. `None` for a
    /// plain prose record, and also for a tool-shaped record we could not
    /// read unambiguously -- an unclassified event is better than a
    /// misclassified one, and either way the record still reaches the
    /// envelope through `text`.
    pub tool: Option<SessionEventTool>,
}

/// The tool activity one session record carried.
///
/// A record carries calls or results, never both -- confirmed across 36 real
/// sessions from all three target datasets. If one ever did, calls win and
/// the results stay in the catch-all rather than being split across a shape
/// we have never seen.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEventTool {
    Calls(Vec<SessionToolCall>),
    Results(Vec<SessionToolResult>),
}

/// One tool call, in the shape `TraceToolCall` needs.
///
/// Both dataset families give a call a real id -- swival's `tool_use.id` and
/// pi-mono/DeepSeek's `toolCall.id` -- which is what lets a result name the
/// call it answers instead of being paired by array position.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// One tool result, in the shape `ExpectedToolResult` needs. `name` is the
/// tool that produced it: read off the record where the source carries it
/// (pi-mono/DeepSeek `toolName`), or resolved from the call the result's id
/// names (swival, whose `tool_result` chunks carry no name).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionToolResult {
    pub tool_call_id: String,
    pub name: String,
    pub content: String,
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

/// Join a chunk list's `text` fields the same way [`extract_event_text`]
/// joins them, so a tool-result record's parsed content and its extracted
/// text are the same string and the submitter can recognize the one as the
/// other instead of emitting both.
fn join_chunk_text(content: &Value) -> String {
    if let Some(s) = content.as_str() {
        return s.trim().to_string();
    }
    let Some(arr) = content.as_array() else {
        return String::new();
    };
    arr.iter()
        .filter_map(|chunk| chunk.get("text").and_then(Value::as_str))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Pull the tool activity out of one event row.
///
/// Two shapes, both confirmed against real downloaded sessions rather than
/// fixtures:
///
/// - swival: `{"type":"tool_use","id":..,"name":..,"input":{..}}` chunks on
///   an assistant record, answered by
///   `{"type":"tool_result","tool_use_id":..,"content":..}` chunks on a user
///   record. The result chunk carries no tool name.
/// - pi-mono and DeepSeek: `{"type":"toolCall","id":..,"name":..,
///   "arguments":{..}}` chunks on an assistant record, answered by a whole
///   record whose `message.role` is `"toolResult"` and which carries
///   `toolCallId` and `toolName`.
///
/// Anything that does not match one of those exactly returns `None` and
/// keeps the existing catch-all. A record missing the id that pairs it is
/// ambiguous by definition and is left unclassified.
fn extract_event_tool(event: &Value) -> Option<SessionEventTool> {
    let msg = event.get("message")?;

    // pi-mono / DeepSeek: the whole record is one tool result.
    if msg.get("role").and_then(Value::as_str) == Some("toolResult") {
        let tool_call_id = non_empty(msg.get("toolCallId"))?;
        let name = non_empty(msg.get("toolName"))?;
        let content = msg.get("content").map(join_chunk_text).unwrap_or_default();
        return Some(SessionEventTool::Results(vec![SessionToolResult {
            tool_call_id,
            name,
            content,
        }]));
    }

    let chunks = msg.get("content")?.as_array()?;
    let mut calls = Vec::new();
    let mut results = Vec::new();
    for chunk in chunks {
        match chunk.get("type").and_then(Value::as_str) {
            Some("tool_use") | Some("toolCall") => {
                let Some(id) = non_empty(chunk.get("id")) else {
                    continue;
                };
                let Some(name) = non_empty(chunk.get("name")) else {
                    continue;
                };
                let arguments = chunk
                    .get("input")
                    .or_else(|| chunk.get("arguments"))
                    .cloned()
                    .unwrap_or(Value::Null);
                calls.push(SessionToolCall {
                    id,
                    name,
                    arguments,
                });
            }
            Some("tool_result") | Some("toolResult") => {
                let Some(tool_call_id) =
                    non_empty(chunk.get("tool_use_id").or_else(|| chunk.get("toolCallId")))
                else {
                    continue;
                };
                results.push(SessionToolResult {
                    tool_call_id,
                    // Filled in by `resolve_result_names` from the call this
                    // result answers; an unresolvable one is dropped there.
                    name: String::new(),
                    content: chunk
                        .get("content")
                        .map(join_chunk_text)
                        .unwrap_or_default(),
                });
            }
            _ => {}
        }
    }

    if !calls.is_empty() {
        Some(SessionEventTool::Calls(calls))
    } else if !results.is_empty() {
        Some(SessionEventTool::Results(results))
    } else {
        None
    }
}

/// Read a JSON field as a non-empty string.
fn non_empty(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Give every result the name of the call it answers, pairing by id.
///
/// Pairing by array position is the failure mode issue #298 called out, and
/// both dataset families carry a real call id, so we never need it. A result
/// whose id names no call in the session cannot be attributed to a tool at
/// all: its whole `SessionEventTool` is cleared so the record falls back to
/// the catch-all it had before, rather than being labelled with a guess.
fn resolve_result_names(events: &mut [SessionEvent]) {
    let mut names: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for event in events.iter() {
        if let Some(SessionEventTool::Calls(calls)) = &event.tool {
            for call in calls {
                names.insert(call.id.clone(), call.name.clone());
            }
        }
    }
    for event in events.iter_mut() {
        let Some(SessionEventTool::Results(results)) = &mut event.tool else {
            continue;
        };
        for result in results.iter_mut() {
            if result.name.is_empty() {
                if let Some(name) = names.get(&result.tool_call_id) {
                    result.name = name.clone();
                }
            }
        }
        if results.iter().any(|r| r.name.is_empty()) {
            event.tool = None;
        }
    }
}

/// Parse every event in a session into a [`SessionEvent`]. An event whose
/// text extracts to nothing contributes no event, matching the previous
/// flatten's behavior exactly (such an event contributed nothing to the
/// joined body either). Lines that fail to parse as JSON are skipped
/// silently (per hash-only logging convention; malformed event rows do
/// happen in the wild and are not operator-actionable). Returns the events
/// in order plus the first observed `sessionId` (or session `id`) for
/// `source_row_id`, if any.
///
/// Multiple text/thinking chunks within one event are joined with the same
/// `"\n\n"` separator the old whole-session join used between chunks, so
/// joining every returned event's `text` with `"\n\n"` reproduces the old
/// flattened body byte-for-byte (`"\n\n"`-joining is associative over how
/// the chunks are grouped).
fn flatten_session(session_bytes: &[u8]) -> (Vec<SessionEvent>, Option<String>) {
    let mut events: Vec<SessionEvent> = Vec::new();
    let mut session_id: Option<String> = None;

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
        let text = extract_event_text(&event).join("\n\n");
        let tool = extract_event_tool(&event);
        if text.is_empty() && tool.is_none() {
            continue;
        }
        let timestamp = event
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
            .map(|dt| dt.with_timezone(&Utc));
        let role = match event
            .get("message")
            .and_then(|m| m.get("role"))
            .and_then(Value::as_str)
        {
            Some("user") => SessionEventRole::User,
            Some("assistant") => SessionEventRole::Assistant,
            _ => SessionEventRole::Other,
        };
        events.push(SessionEvent {
            text,
            timestamp,
            role,
            tool,
        });
    }

    resolve_result_names(&mut events);
    (events, session_id)
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

/// Apply the same `SESSION_BODY_CAP` character budget to the per-event
/// sequence that `truncate_chars` applies to the flattened `trace_body`.
/// Counts the `"\n\n"` separator between events the same way the flattened
/// join does, so concatenating the returned events' text with `"\n\n"`
/// reproduces `truncate_chars(&raw_body, cap)` exactly. An event straddling
/// the boundary is truncated in place, not dropped whole; only events past
/// the boundary are dropped -- the session as a whole gets cut off, same as
/// before, not any one event picked out for its content or classification.
fn cap_events(events: Vec<SessionEvent>, cap: usize) -> Vec<SessionEvent> {
    let mut out = Vec::new();
    let mut used = 0usize;
    let mut emitted_text = false;
    for mut event in events.into_iter() {
        // A record that carried only tool activity contributes nothing to
        // the flattened body, so it spends nothing from the text budget --
        // admitting tool events must not displace prose that reached the
        // envelope before tool events existed. `cap_tool_payloads` bounds
        // these instead.
        if event.text.is_empty() {
            out.push(event);
            continue;
        }
        let sep = if emitted_text { 2 } else { 0 };
        if used + sep >= cap {
            break;
        }
        let remaining = cap - used - sep;
        let len = event.text.chars().count();
        if len <= remaining {
            used += sep + len;
            emitted_text = true;
            out.push(event);
        } else {
            event.text = truncate_chars(&event.text, remaining);
            if !event.text.is_empty() {
                out.push(event);
            }
            break;
        }
    }
    out
}

/// Bound the tool-call arguments and tool-result content one session carries,
/// against a budget separate from the text one.
///
/// Nothing is dropped: a tool event past the budget keeps its call id and
/// tool name, which is what makes the trace legible as a sequence of tool
/// invocations, and loses only the payload that would not fit. Result
/// content is a string and is truncated in place; call arguments are
/// structured and cannot be cut mid-value, so an over-budget argument object
/// is withheld whole (`Value::Null`) rather than mangled into invalid JSON.
fn cap_tool_payloads(events: Vec<SessionEvent>, cap: usize) -> Vec<SessionEvent> {
    let mut used = 0usize;
    events
        .into_iter()
        .map(|mut event| {
            match &mut event.tool {
                Some(SessionEventTool::Calls(calls)) => {
                    for call in calls.iter_mut() {
                        let len = call.arguments.to_string().chars().count();
                        if used + len <= cap {
                            used += len;
                        } else {
                            call.arguments = Value::Null;
                        }
                    }
                }
                Some(SessionEventTool::Results(results)) => {
                    for result in results.iter_mut() {
                        let len = result.content.chars().count();
                        if used + len <= cap {
                            used += len;
                        } else {
                            let remaining = cap.saturating_sub(used);
                            result.content = truncate_chars(&result.content, remaining);
                            used = cap;
                        }
                    }
                }
                None => {}
            }
            event
        })
        .collect()
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
        let (events, session_id) = flatten_session(session_bytes);
        if events.is_empty() {
            anyhow::bail!("session yielded no textual content");
        }
        // Tool-only records carry no text and contribute nothing here, so
        // `trace_body` -- and therefore `submission_id` -- is exactly what it
        // was before tool events were recognized.
        let raw_body = events
            .iter()
            .map(|e| e.text.as_str())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let body = truncate_chars(&raw_body, SESSION_BODY_CAP);
        let id = submission_id_from_body(&body);
        let row_id = session_id.unwrap_or_else(|| row_id_from_sibling(session_name));
        let session_events = cap_tool_payloads(
            cap_events(events, SESSION_BODY_CAP),
            SESSION_TOOL_PAYLOAD_CAP,
        );
        Ok(SubmissionDraft {
            submission_id: id,
            trace_body: body,
            source_dataset: self.source_dataset.to_string(),
            source_row_id: row_id,
            source_domain_tag: self.source_domain_tag.to_string(),
            session_events,
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
    fn session_events_carry_distinct_timestamps_and_roles() {
        // The multi-step point of this change: a session that flattens to
        // one string must not collapse to one step. Each record keeps its
        // own real timestamp and gets classified where unambiguous.
        let t = SwivalTranslator::new();
        let bytes = make_session(&[
            serde_json::json!({"type":"session","id":"sess-multi"}),
            serde_json::json!({
                "type":"message","timestamp":"2026-01-01T00:00:00Z",
                "message":{"role":"user","content":"first turn"}
            }),
            serde_json::json!({
                "type":"message","timestamp":"2026-01-01T00:00:05Z",
                "message":{"role":"assistant","content":"first reply"}
            }),
        ]);
        let draft = t.translate("multi.jsonl", &bytes).unwrap();
        assert_eq!(draft.session_events.len(), 2);
        assert_eq!(draft.session_events[0].role, SessionEventRole::User);
        assert_eq!(draft.session_events[1].role, SessionEventRole::Assistant);
        assert_ne!(
            draft.session_events[0].timestamp,
            draft.session_events[1].timestamp
        );
        assert!(draft.session_events[0].timestamp.is_some());
        assert!(draft.session_events[1].timestamp.is_some());
    }

    #[test]
    fn a_record_without_a_timestamp_stays_without_one() {
        // Never synthesise: a source line with no `timestamp` field yields
        // `None`, not an invented value.
        let t = SwivalTranslator::new();
        let bytes = make_session(&[serde_json::json!({
            "type":"message",
            "message":{"role":"user","content":"no timestamp here"}
        })]);
        let draft = t.translate("no-ts.jsonl", &bytes).unwrap();
        assert_eq!(draft.session_events.len(), 1);
        assert_eq!(draft.session_events[0].timestamp, None);
    }

    #[test]
    fn unclassified_records_fall_back_to_the_catch_all_role_not_dropped() {
        // A record whose role isn't unambiguously user/assistant (a
        // top-level `content` field, no `message` wrapper) still becomes an
        // event -- classified `Other`, not silently dropped.
        let t = SwivalTranslator::new();
        let bytes = make_session(&[serde_json::json!({
            "type":"system","content":"system prompt text"
        })]);
        let draft = t.translate("sys.jsonl", &bytes).unwrap();
        assert_eq!(draft.session_events.len(), 1);
        assert_eq!(draft.session_events[0].role, SessionEventRole::Other);
        assert_eq!(draft.session_events[0].text, "system prompt text");
    }

    #[test]
    fn capped_events_joined_match_the_truncated_trace_body() {
        // `cap_events` must apply the exact same character budget
        // `truncate_chars` applies to the flattened body, so the envelope's
        // steps never carry more content than the body cap allows.
        let t = SwivalTranslator::new();
        let long_a = "a".repeat(20);
        let long_b = "b".repeat(20);
        let bytes = make_session(&[
            serde_json::json!({"type":"message","message":{"role":"user","content": long_a}}),
            serde_json::json!({"type":"message","message":{"role":"assistant","content": long_b}}),
        ]);
        let draft = t.translate("cap.jsonl", &bytes).unwrap();
        // Sanity: the full session is longer than a small cap we impose by
        // re-running the capping logic directly at a tiny budget.
        let (events, _) = flatten_session(&bytes);
        let raw_body = events
            .iter()
            .map(|e| e.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        assert!(raw_body.chars().count() > 25);
        let capped = cap_events(events, 25);
        let capped_joined = capped
            .iter()
            .map(|e| e.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        assert_eq!(capped_joined, truncate_chars(&raw_body, 25));
        // Untouched by the assertion above (SESSION_BODY_CAP is generous),
        // draft itself keeps both events since the real cap is 16000 chars.
        assert_eq!(draft.session_events.len(), 2);
    }

    #[test]
    fn word_filter_bounds_inclusive() {
        assert!(passes_word_filter("a b c", 1, 3));
        assert!(!passes_word_filter("a b c d", 1, 3));
        assert!(!passes_word_filter("", 1, 3));
    }
}

#[cfg(test)]
mod tool_event_tests {
    use super::*;
    use serde_json::json;

    fn make_session(events: &[serde_json::Value]) -> Vec<u8> {
        let mut out = Vec::new();
        for e in events {
            out.extend_from_slice(e.to_string().as_bytes());
            out.push(b'\n');
        }
        out
    }

    /// Swival shape, confirmed against real downloaded sessions:
    /// `tool_use` chunks on an assistant record, `tool_result` chunks on a
    /// user record, paired by `id` / `tool_use_id`. The result chunk carries
    /// no tool name, so the name has to come from the call it answers.
    #[test]
    fn swival_tool_use_and_tool_result_chunks_become_tool_events() {
        let bytes = make_session(&[
            json!({"type":"session","id":"s"}),
            json!({"type":"assistant","message":{"role":"assistant","content":[
                {"type":"tool_use","id":"call_1","name":"read_file",
                 "input":{"file_path":"a.c","offset":10}}
            ]}}),
            json!({"type":"user","message":{"role":"user","content":[
                {"type":"tool_result","tool_use_id":"call_1","content":"file body here"}
            ]}}),
        ]);
        let (events, _) = flatten_session(&bytes);
        assert_eq!(events.len(), 2, "call and result records both survive");
        match &events[0].tool {
            Some(SessionEventTool::Calls(calls)) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_1");
                assert_eq!(calls[0].name, "read_file");
                assert_eq!(calls[0].arguments["file_path"], json!("a.c"));
            }
            other => panic!("expected tool calls, got {other:?}"),
        }
        match &events[1].tool {
            Some(SessionEventTool::Results(results)) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].tool_call_id, "call_1");
                assert_eq!(
                    results[0].name, "read_file",
                    "the result's name comes from the call it answers, by id"
                );
                assert_eq!(results[0].content, "file body here");
            }
            other => panic!("expected tool results, got {other:?}"),
        }
    }

    /// pi-mono / DeepSeek shape, confirmed against real downloaded sessions:
    /// `toolCall` chunks on an assistant record, and a whole record whose
    /// `message.role` is `toolResult` carrying `toolCallId` and `toolName`.
    #[test]
    fn pi_mono_tool_call_chunks_and_tool_result_records_become_tool_events() {
        let bytes = make_session(&[
            json!({"type":"session","id":"s"}),
            json!({"type":"message","message":{"role":"assistant","content":[
                {"type":"toolCall","id":"toolu_1","name":"bash",
                 "arguments":{"command":"ls"}}
            ]}}),
            json!({"type":"message","message":{"role":"toolResult",
                "toolCallId":"toolu_1","toolName":"bash","isError":false,
                "content":[{"type":"text","text":"a.txt"}]}}),
        ]);
        let (events, _) = flatten_session(&bytes);
        assert_eq!(events.len(), 2);
        match &events[0].tool {
            Some(SessionEventTool::Calls(calls)) => {
                assert_eq!(calls[0].id, "toolu_1");
                assert_eq!(calls[0].name, "bash");
                assert_eq!(calls[0].arguments["command"], json!("ls"));
            }
            other => panic!("expected tool calls, got {other:?}"),
        }
        match &events[1].tool {
            Some(SessionEventTool::Results(results)) => {
                assert_eq!(results[0].tool_call_id, "toolu_1");
                assert_eq!(results[0].name, "bash");
                assert_eq!(results[0].content, "a.txt");
            }
            other => panic!("expected tool results, got {other:?}"),
        }
    }

    /// The dominant real shape in pi-mono and DeepSeek: one assistant record
    /// carries prose AND tool calls. Neither may be lost.
    #[test]
    fn a_record_carrying_both_prose_and_tool_calls_keeps_both() {
        let bytes = make_session(&[json!({"type":"message","message":{
        "role":"assistant","content":[
            {"type":"text","text":"let me look"},
            {"type":"toolCall","id":"t1","name":"bash","arguments":{"command":"ls"}}
        ]}})]);
        let (events, _) = flatten_session(&bytes);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].text, "let me look");
        assert!(matches!(events[0].tool, Some(SessionEventTool::Calls(_))));
    }

    /// A result naming a call this session never made is genuinely
    /// ambiguous -- we cannot say which tool produced it. It keeps the
    /// existing catch-all rather than being misclassified, and it is not
    /// dropped.
    #[test]
    fn a_tool_result_with_no_matching_call_keeps_the_catch_all() {
        let bytes = make_session(&[json!({"type":"user","message":{
        "role":"user","content":[
            {"type":"tool_result","tool_use_id":"unknown","content":"orphan output"}
        ]}})]);
        let (events, _) = flatten_session(&bytes);
        assert_eq!(events.len(), 1, "the record is not dropped");
        assert!(
            events[0].tool.is_none(),
            "an unpairable result stays unclassified rather than being guessed at"
        );
    }

    /// Tool payloads get their own budget so a tool-heavy session cannot
    /// blow the envelope, and the text budget is untouched by them.
    #[test]
    fn tool_payloads_are_bounded_by_their_own_budget() {
        let big = "x".repeat(5_000);
        let mut records = vec![json!({"type":"session","id":"s"})];
        for i in 0..10 {
            records.push(
                json!({"type":"message","message":{"role":"assistant","content":[
                    {"type":"toolCall","id":format!("t{i}"),"name":"bash",
                     "arguments":{"command":big}}
                ]}}),
            );
        }
        let bytes = make_session(&records);
        let (events, _) = flatten_session(&bytes);
        let capped = cap_tool_payloads(events, 12_000);
        assert_eq!(capped.len(), 10, "no tool record is dropped by the budget");
        let total: usize = capped
            .iter()
            .map(|e| match &e.tool {
                Some(SessionEventTool::Calls(c)) => c
                    .iter()
                    .map(|c| c.arguments.to_string().chars().count())
                    .sum(),
                Some(SessionEventTool::Results(r)) => {
                    r.iter().map(|r| r.content.chars().count()).sum()
                }
                None => 0,
            })
            .sum();
        assert!(
            total <= 12_000,
            "tool payloads must stay inside their budget, got {total}"
        );
    }

    /// The body cap must admit a realistic agent session. Measured on real
    /// downloaded sessions, the largest sampled session extracts 475,268
    /// characters of text across 320 records; under the old 16,000-character
    /// cap that session reached the envelope as 10 events. A session of that
    /// size must now survive whole.
    #[test]
    fn the_body_cap_admits_a_realistic_agent_session() {
        let turn = "word ".repeat(300);
        let mut records = vec![json!({"type":"session","id":"s"})];
        for _ in 0..320 {
            records.push(json!({"type":"message",
                "message":{"role":"assistant","content":turn}}));
        }
        let draft = SwivalTranslator::new()
            .translate("big.jsonl", &make_session(&records))
            .unwrap();
        assert_eq!(
            draft.session_events.len(),
            320,
            "a ~480k-character session must not be truncated"
        );
        assert_eq!(
            draft.trace_body.chars().count(),
            320 * turn.trim().len() + 319 * 2
        );
    }
}
