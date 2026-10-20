//! Read-only SSE observation.
//!
//! The forward path streams bytes verbatim. This module reads a *copy* to learn
//! usage and served-model, and it is built around one rule: **the observer must
//! never be able to stall or fail the forward path** (`docs/PROTOCOL.md` §2).
//! Every failure mode here — an oversized frame, malformed JSON, an event shape
//! we do not know — degrades to "we learned less", never to a broken stream.

use crate::observe::{Observation, UsageReading, anthropic_usage, openai_usage};

/// Largest single SSE frame we will buffer while observing.
///
/// Frames above this are forwarded normally and simply not parsed. Anthropic's
/// largest frames (`message_start` with a big usage block) are a few KB; a
/// megabyte means something unusual is happening and holding it in the observer
/// would be the wrong response.
const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Which stream dialect to interpret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// Anthropic Messages SSE.
    Anthropic,
    /// OpenAI Responses SSE.
    OpenAiResponses,
    /// OpenAI Chat Completions SSE.
    OpenAiChat,
}

/// Incremental observer over an SSE byte stream.
#[derive(Debug)]
pub struct SseObserver {
    dialect: Dialect,
    buffer: Vec<u8>,
    observation: Observation,
    /// Set when a frame exceeded the buffer cap, so callers can distinguish
    /// "nothing to report" from "we stopped looking".
    truncated: bool,
    /// Whether the response is an event stream.
    ///
    /// A response that is not carries its usage in a single JSON document
    /// rather than in frames, and reading only frames meant a non-streaming
    /// client reported no tokens, no cost, and therefore nothing against a
    /// spend cap — a cap that silently could not fire.
    streaming: bool,
}

impl SseObserver {
    /// New observer for a dialect.
    #[must_use]
    pub fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            buffer: Vec::new(),
            observation: Observation::default(),
            truncated: false,
            streaming: true,
        }
    }

    /// New observer for a response that is a single JSON document.
    ///
    /// Chosen from the response's own `content-type` rather than from the
    /// request's `stream` flag: what matters is the shape that actually came
    /// back, and a provider is free to answer a streaming request with a plain
    /// body when something went wrong.
    #[must_use]
    pub fn for_document(dialect: Dialect) -> Self {
        Self {
            streaming: false,
            ..Self::new(dialect)
        }
    }

    /// Feed a chunk. Infallible by construction.
    pub fn push(&mut self, chunk: &[u8]) {
        if !self.streaming {
            // Same ceiling as a frame, for the same reason: a hostile upstream
            // must not be able to grow our memory without bound. Over it we
            // stop accumulating and record nothing — the body still reaches the
            // client untouched, because our bookkeeping never affects it.
            if self.buffer.len().saturating_add(chunk.len()) > MAX_FRAME_BYTES {
                self.truncated = true;
                self.buffer.clear();
                return;
            }
            if !self.truncated {
                self.buffer.extend_from_slice(chunk);
            }
            return;
        }
        if self.buffer.len().saturating_add(chunk.len()) > MAX_FRAME_BYTES {
            // Drop what we have and resynchronise at the next frame boundary
            // rather than growing without bound.
            self.truncated = true;
            self.buffer.clear();
            if let Some(pos) = find_boundary(chunk) {
                self.buffer.extend_from_slice(&chunk[pos..]);
            }
            return;
        }
        self.buffer.extend_from_slice(chunk);
        while let Some(pos) = find_boundary(&self.buffer) {
            let frame: Vec<u8> = self.buffer.drain(..pos).collect();
            self.consume_frame(&frame);
        }
    }

    /// Finish and return what was learned.
    #[must_use]
    pub fn finish(mut self) -> Observation {
        if !self.streaming {
            let body = std::mem::take(&mut self.buffer);
            if !self.truncated
                && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&body)
            {
                self.consume_document(&value);
            }
            return self.observation;
        }
        if !self.buffer.is_empty() {
            let frame = std::mem::take(&mut self.buffer);
            self.consume_frame(&frame);
        }
        self.observation
    }

    /// Whether a frame was skipped for size.
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    fn consume_frame(&mut self, frame: &[u8]) {
        let Ok(text) = std::str::from_utf8(frame) else {
            return;
        };
        // An SSE frame may carry multiple `data:` lines that concatenate.
        let mut data = String::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("data:") {
                data.push_str(rest.trim_start());
            }
        }
        if data.is_empty() || data == "[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) else {
            return;
        };
        match self.dialect {
            Dialect::Anthropic => self.consume_anthropic(&value),
            Dialect::OpenAiResponses => self.consume_openai_responses(&value),
            Dialect::OpenAiChat => self.consume_openai_chat(&value),
        }
    }

    /// Read usage from a whole, non-streamed response.
    ///
    /// The same fields as the streamed shapes, one level up: a streamed
    /// Anthropic response nests usage under `message`, and a streamed Responses
    /// one under `response`, because each frame wraps the object it is
    /// reporting on. A complete document *is* that object.
    fn consume_document(&mut self, value: &serde_json::Value) {
        if let Some(model) = value.get("model").and_then(serde_json::Value::as_str) {
            self.observation.served_model = Some(model.to_string());
        }
        self.note_upstream_id(value.get("id"));
        let usage = match self.dialect {
            Dialect::Anthropic => value.get("usage").and_then(anthropic_usage),
            Dialect::OpenAiResponses | Dialect::OpenAiChat => {
                value.get("usage").and_then(openai_usage)
            }
        };
        if let Some(usage) = usage {
            self.merge_usage(usage);
        }
    }

    /// Record the provider's response id the first time a frame carries one.
    ///
    /// First wins, deliberately. A streamed response repeats its id on many
    /// frames and they agree; if some future provider shape ever disagreed,
    /// the opening frame is the one that named the response the receipt will
    /// be about.
    fn note_upstream_id(&mut self, value: Option<&serde_json::Value>) {
        if self.observation.upstream_id.is_some() {
            return;
        }
        if let Some(id) = value.and_then(serde_json::Value::as_str)
            && !id.is_empty()
        {
            self.observation.upstream_id = Some(id.to_string());
        }
    }

    fn consume_anthropic(&mut self, value: &serde_json::Value) {
        match value.get("type").and_then(serde_json::Value::as_str) {
            Some("message_start") => {
                if let Some(model) = value
                    .pointer("/message/model")
                    .and_then(serde_json::Value::as_str)
                {
                    self.observation.served_model = Some(model.to_string());
                }
                self.note_upstream_id(value.pointer("/message/id"));
                if let Some(usage) = value.pointer("/message/usage").and_then(anthropic_usage) {
                    self.merge_usage(usage);
                }
            }
            Some("message_delta") => {
                if let Some(usage) = value.get("usage").and_then(anthropic_usage) {
                    self.merge_usage(usage);
                }
            }
            _ => {}
        }
    }

    fn consume_openai_responses(&mut self, value: &serde_json::Value) {
        if let Some(model) = value
            .pointer("/response/model")
            .and_then(serde_json::Value::as_str)
        {
            self.observation.served_model = Some(model.to_string());
        }
        self.note_upstream_id(value.pointer("/response/id"));
        if let Some(usage) = value.pointer("/response/usage").and_then(openai_usage) {
            self.merge_usage(usage);
        }
    }

    fn consume_openai_chat(&mut self, value: &serde_json::Value) {
        if let Some(model) = value.get("model").and_then(serde_json::Value::as_str) {
            self.observation.served_model = Some(model.to_string());
        }
        self.note_upstream_id(value.get("id"));
        if let Some(usage) = value.get("usage").and_then(openai_usage) {
            self.merge_usage(usage);
        }
    }

    fn merge_usage(&mut self, usage: UsageReading) {
        self.observation
            .usage
            .get_or_insert_with(UsageReading::default)
            .merge(usage);
    }
}

/// Byte offset just past the next frame boundary (`\n\n` or `\r\n\r\n`).
fn find_boundary(buf: &[u8]) -> Option<usize> {
    let lf = buf.windows(2).position(|w| w == b"\n\n").map(|p| p + 2);
    let crlf = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANTHROPIC_STREAM: &str = concat!(
        "event: message_start\n",
        r#"data: {"type":"message_start","message":{"model":"claude-opus-4-6","usage":{"input_tokens":12,"cache_read_input_tokens":98000,"output_tokens":1}}}"#,
        "\n\n",
        "event: content_block_delta\n",
        r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}"#,
        "\n\n",
        "event: message_delta\n",
        r#"data: {"type":"message_delta","usage":{"output_tokens":40}}"#,
        "\n\n",
        "event: message_delta\n",
        r#"data: {"type":"message_delta","usage":{"output_tokens":137}}"#,
        "\n\n",
        "event: message_stop\n",
        r#"data: {"type":"message_stop"}"#,
        "\n\n",
    );

    #[test]
    fn reads_usage_and_model_from_an_anthropic_stream() {
        let mut observer = SseObserver::new(Dialect::Anthropic);
        observer.push(ANTHROPIC_STREAM.as_bytes());
        let obs = observer.finish();
        assert_eq!(obs.served_model.as_deref(), Some("claude-opus-4-6"));
        let usage = obs.usage.expect("usage observed");
        assert_eq!(usage.input_tokens, 12);
        assert_eq!(usage.cache_read_tokens, 98_000);
        assert_eq!(usage.output_tokens, 137, "cumulative deltas must not sum");
    }

    #[test]
    fn byte_by_byte_delivery_yields_the_same_result() {
        // Real streams split frames arbitrarily; the parser must not depend on
        // chunk boundaries lining up with frames.
        let mut observer = SseObserver::new(Dialect::Anthropic);
        for byte in ANTHROPIC_STREAM.as_bytes() {
            observer.push(&[*byte]);
        }
        let usage = observer.finish().usage.expect("usage observed");
        assert_eq!(usage.output_tokens, 137);
        assert_eq!(usage.input_tokens, 12);
    }

    #[test]
    fn handles_crlf_framing() {
        let stream = ANTHROPIC_STREAM.replace('\n', "\r\n");
        let mut observer = SseObserver::new(Dialect::Anthropic);
        observer.push(stream.as_bytes());
        assert_eq!(
            observer
                .finish()
                .usage
                .expect("usage observed")
                .output_tokens,
            137
        );
    }

    #[test]
    fn malformed_frames_reduce_knowledge_rather_than_breaking() {
        let mut observer = SseObserver::new(Dialect::Anthropic);
        observer.push(b"data: {not json at all\n\n");
        observer.push(b"event: ping\n\n");
        observer.push(b"data: [DONE]\n\n");
        observer.push(&[0xff, 0xfe, b'\n', b'\n']); // invalid UTF-8
        let obs = observer.finish();
        assert!(obs.is_empty(), "garbage must not synthesise an observation");
    }

    #[test]
    fn an_oversized_frame_is_skipped_and_the_stream_resynchronises() {
        let mut observer = SseObserver::new(Dialect::Anthropic);
        let giant = vec![b'x'; MAX_FRAME_BYTES + 1];
        observer.push(&giant);
        assert!(observer.truncated());
        // The stream keeps working afterwards.
        observer.push(b"\n\n");
        observer.push(ANTHROPIC_STREAM.as_bytes());
        let obs = observer.finish();
        assert_eq!(obs.served_model.as_deref(), Some("claude-opus-4-6"));
    }

    #[test]
    fn reads_usage_from_an_openai_responses_stream() {
        let stream = concat!(
            "event: response.created\n",
            r#"data: {"type":"response.created","response":{"model":"gpt-5.6"}}"#,
            "\n\n",
            "event: response.completed\n",
            r#"data: {"type":"response.completed","response":{"model":"gpt-5.6","usage":{"input_tokens":900,"input_tokens_details":{"cached_tokens":800},"output_tokens":50}}}"#,
            "\n\n",
        );
        let mut observer = SseObserver::new(Dialect::OpenAiResponses);
        observer.push(stream.as_bytes());
        let obs = observer.finish();
        assert_eq!(obs.served_model.as_deref(), Some("gpt-5.6"));
        let usage = obs.usage.expect("usage observed");
        assert_eq!(usage.cache_read_tokens, 800);
        assert_eq!(usage.input_tokens, 100);
    }

    #[test]
    fn a_stream_that_ends_without_a_trailing_boundary_is_still_read() {
        let mut observer = SseObserver::new(Dialect::Anthropic);
        observer.push(br#"data: {"type":"message_start","message":{"model":"claude-opus-4-6"}}"#);
        assert_eq!(
            observer.finish().served_model.as_deref(),
            Some("claude-opus-4-6")
        );
    }

    #[test]
    fn multi_line_data_fields_concatenate() {
        let mut observer = SseObserver::new(Dialect::Anthropic);
        observer
            .push(b"data: {\"type\":\"message_start\",\ndata: \"message\":{\"model\":\"m\"}}\n\n");
        assert_eq!(observer.finish().served_model.as_deref(), Some("m"));
    }
}

#[cfg(test)]
mod document_tests {
    use super::*;

    fn observe(dialect: Dialect, body: &str) -> Observation {
        let mut observer = SseObserver::for_document(dialect);
        // In chunks, because a real body arrives in several.
        for chunk in body.as_bytes().chunks(7) {
            observer.push(chunk);
        }
        observer.finish()
    }

    /// The bug this exists for: a client that does not stream reported no
    /// tokens, so no cost, so nothing against a spend cap — a cap that could
    /// not fire.
    #[test]
    fn a_non_streamed_chat_completion_reports_its_usage() {
        let observation = observe(
            Dialect::OpenAiChat,
            r#"{"id":"c1","model":"gpt-4.1","choices":[],
                "usage":{"prompt_tokens":1200,"completion_tokens":340}}"#,
        );
        let usage = observation.usage.expect("usage was reported");
        assert_eq!(usage.input_tokens, 1200);
        assert_eq!(usage.output_tokens, 340);
        assert_eq!(observation.served_model.as_deref(), Some("gpt-4.1"));
    }

    /// A streamed Anthropic response nests usage under `message` because each
    /// frame wraps the object it reports on; a whole document *is* that object.
    #[test]
    fn a_non_streamed_anthropic_message_reports_its_usage() {
        let observation = observe(
            Dialect::Anthropic,
            r#"{"id":"m1","type":"message","model":"claude-opus-4-6",
                "usage":{"input_tokens":12,"cache_read_input_tokens":98000,"output_tokens":40}}"#,
        );
        let usage = observation.usage.expect("usage was reported");
        assert_eq!(usage.input_tokens, 12);
        assert_eq!(usage.cache_read_tokens, 98_000);
        assert_eq!(usage.output_tokens, 40);
        assert_eq!(observation.served_model.as_deref(), Some("claude-opus-4-6"));
    }

    #[test]
    fn a_non_streamed_responses_body_reports_its_usage() {
        let observation = observe(
            Dialect::OpenAiResponses,
            r#"{"id":"r1","model":"gpt-5.6","usage":{"input_tokens":7,"output_tokens":9}}"#,
        );
        let usage = observation.usage.expect("usage was reported");
        assert_eq!(usage.input_tokens, 7);
        assert_eq!(usage.output_tokens, 9);
    }

    /// An error body, or HTML from a proxy in the way, is not usage — and is
    /// certainly not zero usage.
    #[test]
    fn a_body_that_is_not_json_reports_nothing_rather_than_zero() {
        assert!(
            observe(Dialect::OpenAiChat, "<html>502 Bad Gateway</html>")
                .usage
                .is_none()
        );
        assert!(observe(Dialect::OpenAiChat, "").usage.is_none());
    }

    /// The client's response must never be affected by our bookkeeping, so an
    /// oversized body is simply not parsed. `push` is infallible either way —
    /// what this pins is that it neither grows without bound nor panics.
    #[test]
    fn an_oversized_body_is_dropped_rather_than_buffered() {
        let mut observer = SseObserver::for_document(Dialect::OpenAiChat);
        let chunk = vec![b'x'; 64 * 1024];
        for _ in 0..(MAX_FRAME_BYTES / chunk.len() + 2) {
            observer.push(&chunk);
        }
        assert!(observer.truncated(), "an unbounded body was accumulated");
        assert!(observer.finish().usage.is_none());
    }

    /// A response that says it is a stream is still read as frames, whatever
    /// the request asked for.
    #[test]
    fn the_streaming_path_is_untouched() {
        let mut observer = SseObserver::new(Dialect::OpenAiChat);
        observer.push(
            b"data: {\"model\":\"gpt-4.1\",\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":6}}\n\n",
        );
        let usage = observer.finish().usage.expect("usage was reported");
        assert_eq!(usage.input_tokens, 5);
    }
}

#[cfg(test)]
mod upstream_id_tests {
    use super::*;

    /// The id is what makes a provider's own receipt reachable later. Without
    /// it, `GET /v1/signature/{id}` has nothing to ask about.
    #[test]
    fn a_whole_openai_response_yields_its_id() {
        let mut obs = SseObserver::new(Dialect::OpenAiChat);
        obs.consume_document(
            &serde_json::json!({"id": "c54961ab1d594cf591e5566caa21196b", "model": "Qwen/Qwen3.6-27B-FP8"}),
        );
        assert_eq!(
            obs.observation.upstream_id.as_deref(),
            Some("c54961ab1d594cf591e5566caa21196b")
        );
    }

    #[test]
    fn an_anthropic_stream_yields_the_id_from_message_start() {
        let mut obs = SseObserver::new(Dialect::Anthropic);
        obs.consume_anthropic(&serde_json::json!({
            "type": "message_start",
            "message": {"id": "msg_0123", "model": "claude-opus-4-6"}
        }));
        assert_eq!(obs.observation.upstream_id.as_deref(), Some("msg_0123"));
    }

    #[test]
    fn a_responses_stream_yields_the_id_from_the_response_object() {
        let mut obs = SseObserver::new(Dialect::OpenAiResponses);
        obs.consume_openai_responses(&serde_json::json!({
            "response": {"id": "resp_77", "model": "gpt-5"}
        }));
        assert_eq!(obs.observation.upstream_id.as_deref(), Some("resp_77"));
    }

    /// A streamed response repeats its id on many frames. First wins, so a
    /// later frame cannot rename the response the receipt is about.
    #[test]
    fn a_later_frame_does_not_rename_the_response() {
        let mut obs = SseObserver::new(Dialect::OpenAiChat);
        obs.consume_openai_chat(&serde_json::json!({"id": "first", "model": "m"}));
        obs.consume_openai_chat(&serde_json::json!({"id": "second", "model": "m"}));
        assert_eq!(obs.observation.upstream_id.as_deref(), Some("first"));
    }

    /// A provider that reports no id leaves the field empty rather than
    /// inventing one -- the same rule the rest of this module follows for
    /// usage and capacity.
    #[test]
    fn no_id_reported_is_none_not_a_placeholder() {
        let mut obs = SseObserver::new(Dialect::OpenAiChat);
        obs.consume_openai_chat(&serde_json::json!({"model": "m"}));
        assert_eq!(obs.observation.upstream_id, None);
    }

    #[test]
    fn an_empty_id_is_not_recorded() {
        let mut obs = SseObserver::new(Dialect::OpenAiChat);
        obs.consume_openai_chat(&serde_json::json!({"id": "", "model": "m"}));
        assert_eq!(obs.observation.upstream_id, None);
    }
}
