//! Streaming translation, through the pivot.
//!
//! Every coding agent streams, so a translated lane that only handled the
//! non-streaming shape would be a translated lane that never runs.
//!
//! This is the layer that justifies the IR. Done pairwise it would be six SSE
//! state machines, each with its own framing, its own tool-call accumulator and
//! its own answer to "what does a half-finished call look like" — six chances to
//! disagree, in the part of the system where disagreement is invisible until an
//! agent hangs. Here there is one framer, three parsers and three emitters.
//!
//! Two rules, both inherited from the pairwise translator that came before:
//!
//! - **Text is forwarded incrementally.** The user watches tokens appear, which
//!   is the whole point of streaming.
//! - **Tool calls are buffered until complete.** Arguments arrive as fragments
//!   of a JSON string on two of the three wires, and a parsed object is what the
//!   third needs. Buffering costs no perceived latency, because no agent can act
//!   on half a call.

use serde_json::{Value, json};

use ironwire_core::protocol::Protocol;

use crate::ir::{Delta, StopReason, Usage};
use crate::{anthropic, chat, responses, tool_ids};

/// Largest SSE frame we will accumulate before giving up on finding a boundary.
///
/// The buffer holds bytes until a `\n\n` arrives. An upstream that never sends
/// one — broken, or hostile — would otherwise grow it without limit. Real frames
/// are a few kilobytes; a megabyte is far past anything legitimate and still
/// small enough that discarding one costs nothing.
const MAX_FRAME_BYTES: usize = 1 << 20;

/// Most parallel tool calls a single response may declare.
///
/// The index comes from the upstream and drives a `Vec::resize`, so without a
/// bound a single frame saying `"index": 4000000000` allocates until the process
/// dies. IronWire lets a user point at an arbitrary OpenAI-compatible endpoint,
/// which makes that reachable rather than theoretical.
const MAX_TOOL_CALLS: usize = 256;

/// Most bytes of accumulated arguments across all tool calls in one response.
///
/// Arguments arrive as fragments that are concatenated, so this is the third
/// place an upstream controls how much we allocate.
const MAX_TOOL_ARGUMENT_BYTES: usize = 4 << 20;

/// A tool call arriving in fragments.
#[derive(Debug, Default, Clone)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
    /// Set when a fragment was refused because the accumulated arguments grew
    /// past what we accept. Such a call is dropped rather than emitted with
    /// truncated JSON — the client would hand it to a tool.
    overflowed: bool,
    /// Already emitted as a complete [`Delta::ToolCall`].
    emitted: bool,
}

/// Accumulates streamed tool calls, with the three upstream-controlled growth
/// paths bounded in one place rather than three.
#[derive(Debug, Default)]
struct ToolCallBuffer {
    calls: Vec<PartialToolCall>,
    argument_bytes: usize,
}

impl ToolCallBuffer {
    fn slot(&mut self, index: usize) -> Option<&mut PartialToolCall> {
        if index >= MAX_TOOL_CALLS {
            tracing::warn!(index, "ignoring a tool call with an implausible index");
            return None;
        }
        if self.calls.len() <= index {
            self.calls.resize(index + 1, PartialToolCall::default());
        }
        self.calls.get_mut(index)
    }

    fn set_identity(&mut self, index: usize, id: Option<&str>, name: Option<&str>) {
        let Some(slot) = self.slot(index) else { return };
        if let Some(id) = id.filter(|id| !id.is_empty()) {
            slot.id = id.to_string();
        }
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            slot.name = name.to_string();
        }
    }

    fn push_arguments(&mut self, index: usize, fragment: &str) {
        // Truncating would produce a call with unparseable input, which the
        // client would hand to a tool; refusing the fragment and dropping the
        // call is the lesser failure.
        if self.argument_bytes + fragment.len() > MAX_TOOL_ARGUMENT_BYTES {
            if let Some(slot) = self.slot(index)
                && !slot.overflowed
            {
                slot.overflowed = true;
                tracing::warn!("tool-call arguments exceeded the accepted size");
            }
            return;
        }
        let length = fragment.len();
        if let Some(slot) = self.slot(index) {
            slot.arguments.push_str(fragment);
            self.argument_bytes += length;
        }
    }

    /// Emit one call as complete, if it has not been emitted already.
    fn complete(&mut self, index: usize, out: &mut Vec<Delta>, source: Protocol) {
        let Some(slot) = self.calls.get_mut(index) else {
            return;
        };
        if slot.emitted || slot.name.is_empty() {
            return;
        }
        if slot.overflowed {
            tracing::warn!(
                name = %slot.name,
                "dropping a tool call whose arguments exceeded the accepted size"
            );
            slot.emitted = true;
            return;
        }
        slot.emitted = true;
        out.push(Delta::ToolCall {
            index,
            id: tool_ids::decode(&slot.id, source),
            name: slot.name.clone(),
            arguments: if slot.arguments.is_empty() {
                "{}".to_string()
            } else {
                slot.arguments.clone()
            },
        });
    }

    /// Emit everything still outstanding.
    fn complete_all(&mut self, out: &mut Vec<Delta>, source: Protocol) {
        for index in 0..self.calls.len() {
            self.complete(index, out, source);
        }
    }

    fn any(&self) -> bool {
        !self.calls.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Parsers: wire SSE → Delta
// ---------------------------------------------------------------------------

/// Reads one wire's event stream into [`Delta`]s.
#[derive(Debug)]
struct Parser {
    source: Protocol,
    tool_calls: ToolCallBuffer,
    started: bool,
    stop: Option<StopReason>,
    usage: Usage,
    /// Responses only: which output index is which call.
    responses_call_index: Vec<usize>,
    /// `ln p(chosen token)` for each generated token, in order, when the
    /// request asked for log-probabilities. Empty otherwise, which is the
    /// default.
    ///
    /// Accumulated but never forwarded: no client asked for these and the
    /// target's shape may have nowhere to put them. They exist to be reduced
    /// locally into `ironwire_core::confidence` aggregates. Bounded by
    /// `chat::MAX_TOKEN_LOGPROBS`.
    token_logprobs: Vec<f64>,
}

impl Parser {
    fn new(source: Protocol) -> Self {
        Self {
            source,
            tool_calls: ToolCallBuffer::default(),
            started: false,
            stop: None,
            usage: Usage::default(),
            responses_call_index: Vec::new(),
            token_logprobs: Vec::new(),
        }
    }

    fn feed(&mut self, event: Option<&str>, data: &Value, out: &mut Vec<Delta>) {
        match self.source {
            Protocol::AnthropicMessages => self.feed_anthropic(data, out),
            Protocol::OpenAiResponses => self.feed_responses(event, data, out),
            Protocol::OpenAiChat => self.feed_chat(data, out),
        }
    }

    fn start(&mut self, id: &str, out: &mut Vec<Delta>) {
        if self.started {
            return;
        }
        self.started = true;
        out.push(Delta::Start {
            id: id.to_string(),
            // Filled in by the emitter, which knows what the client asked for.
            model: String::new(),
        });
    }

    /// Everything owed at the end of the stream.
    fn finish(&mut self, out: &mut Vec<Delta>) {
        self.start("", out);
        self.tool_calls.complete_all(out, self.source);
        let stop = self.stop.clone().unwrap_or({
            // A stream that ended without saying why, but handed us a call, was
            // waiting for a tool. Reporting `EndTurn` here stops the agent.
            if self.tool_calls.any() {
                StopReason::ToolUse
            } else {
                StopReason::EndTurn
            }
        });
        out.push(Delta::Stop {
            reason: stop,
            usage: self.usage,
        });
    }

    // -- Anthropic ---------------------------------------------------------

    fn feed_anthropic(&mut self, data: &Value, out: &mut Vec<Delta>) {
        match data.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                self.start(
                    data.pointer("/message/id")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    out,
                );
                if let Some(usage) = data.pointer("/message/usage") {
                    self.usage = anthropic_usage(usage);
                }
            }
            Some("content_block_start") => {
                let index = index_of(data);
                match data.pointer("/content_block/type").and_then(Value::as_str) {
                    Some("tool_use") => {
                        self.tool_calls.set_identity(
                            index,
                            data.pointer("/content_block/id").and_then(Value::as_str),
                            data.pointer("/content_block/name").and_then(Value::as_str),
                        );
                        // Anthropic sends the input as `input_json_delta`
                        // fragments even when it also sends an empty object
                        // here, so nothing is accumulated from this event.
                    }
                    _ => self.start("", out),
                }
            }
            Some("content_block_delta") => {
                match data.pointer("/delta/type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(text) = data.pointer("/delta/text").and_then(Value::as_str) {
                            out.push(Delta::Text(text.to_string()));
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = data.pointer("/delta/thinking").and_then(Value::as_str)
                        {
                            out.push(Delta::ReasoningText(text.to_string()));
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(fragment) =
                            data.pointer("/delta/partial_json").and_then(Value::as_str)
                        {
                            self.tool_calls.push_arguments(index_of(data), fragment);
                        }
                    }
                    _ => {}
                }
            }
            Some("content_block_stop") => {
                self.tool_calls.complete(index_of(data), out, self.source);
            }
            Some("message_delta") => {
                self.stop = Some(anthropic::parse_stop_reason(
                    data.pointer("/delta/stop_reason").and_then(Value::as_str),
                    data.pointer("/delta/stop_sequence").and_then(Value::as_str),
                ));
                if let Some(usage) = data.get("usage") {
                    let read = anthropic_usage(usage);
                    // `message_delta` reports output tokens only; the input
                    // count came with `message_start` and must not be zeroed.
                    self.usage.output = read.output;
                    if read.input > 0 {
                        self.usage.input = read.input;
                    }
                }
            }
            _ => {}
        }
    }

    // -- Chat Completions --------------------------------------------------

    fn feed_chat(&mut self, data: &Value, out: &mut Vec<Delta>) {
        if let Some(usage) = data.get("usage").filter(|u| !u.is_null()) {
            self.usage = chat::parse_usage(Some(usage));
        }
        let Some(choice) = data
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return;
        };
        self.start(data.get("id").and_then(Value::as_str).unwrap_or(""), out);

        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.stop = Some(chat::parse_stop_reason(Some(reason)));
        }
        // Before the delta check on purpose: `logprobs` is a sibling of `delta`,
        // and the chunk carrying the finish reason often has one without the
        // other. Accumulating after would drop the final token. Bounded inside,
        // so a frame that is nothing but `logprobs` cannot grow the vector
        // without limit while producing no output.
        chat::accumulate_token_logprobs(choice, &mut self.token_logprobs);
        let Some(delta) = choice.get("delta") else {
            return;
        };
        if let Some(text) = delta
            .get("content")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
        {
            out.push(Delta::Text(text.to_string()));
        }
        // Several OpenAI-compatible endpoints stream a reasoning summary here.
        if let Some(text) = delta
            .get("reasoning_content")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
        {
            out.push(Delta::ReasoningText(text.to_string()));
        }
        for call in delta
            .get("tool_calls")
            .and_then(Value::as_array)
            .unwrap_or(&Vec::new())
        {
            let index = call
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|i| usize::try_from(i).ok())
                .unwrap_or(0);
            self.tool_calls.set_identity(
                index,
                call.get("id").and_then(Value::as_str),
                call.pointer("/function/name").and_then(Value::as_str),
            );
            if let Some(fragment) = call.pointer("/function/arguments").and_then(Value::as_str) {
                self.tool_calls.push_arguments(index, fragment);
            }
        }
        // Only complete on the terminal frame: this wire never says a single
        // call is done, only that the whole turn is.
        if self.stop.is_some() {
            self.tool_calls.complete_all(out, self.source);
        }
    }

    // -- Responses ---------------------------------------------------------

    fn feed_responses(&mut self, event: Option<&str>, data: &Value, out: &mut Vec<Delta>) {
        let kind = data
            .get("type")
            .and_then(Value::as_str)
            .or(event)
            .unwrap_or_default();
        match kind {
            "response.created" => self.start(
                data.pointer("/response/id")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                out,
            ),
            "response.output_text.delta" => {
                if let Some(text) = data.get("delta").and_then(Value::as_str) {
                    out.push(Delta::Text(text.to_string()));
                }
            }
            "response.reasoning_summary_text.delta" => {
                if let Some(text) = data.get("delta").and_then(Value::as_str) {
                    out.push(Delta::ReasoningText(text.to_string()));
                }
            }
            "response.output_item.added" => {
                if data.pointer("/item/type").and_then(Value::as_str) == Some("function_call") {
                    let index = self.responses_slot(output_index(data));
                    self.tool_calls.set_identity(
                        index,
                        data.pointer("/item/call_id").and_then(Value::as_str),
                        data.pointer("/item/name").and_then(Value::as_str),
                    );
                }
            }
            "response.function_call_arguments.delta" => {
                let index = self.responses_slot(output_index(data));
                if let Some(fragment) = data.get("delta").and_then(Value::as_str) {
                    self.tool_calls.push_arguments(index, fragment);
                }
            }
            // The one wire that says when a single call is finished, rather
            // than only when the whole turn is.
            "response.function_call_arguments.done" => {
                let index = self.responses_slot(output_index(data));
                if let Some(arguments) = data.get("arguments").and_then(Value::as_str)
                    && let Some(slot) = self.tool_calls.slot(index)
                    && slot.arguments.is_empty()
                {
                    slot.arguments = arguments.to_string();
                }
                self.tool_calls.complete(index, out, self.source);
            }
            "response.output_item.done" => {
                if data.pointer("/item/type").and_then(Value::as_str) == Some("function_call") {
                    let index = self.responses_slot(output_index(data));
                    self.tool_calls.set_identity(
                        index,
                        data.pointer("/item/call_id").and_then(Value::as_str),
                        data.pointer("/item/name").and_then(Value::as_str),
                    );
                    if let Some(arguments) = data.pointer("/item/arguments").and_then(Value::as_str)
                        && let Some(slot) = self.tool_calls.slot(index)
                        && slot.arguments.is_empty()
                    {
                        slot.arguments = arguments.to_string();
                    }
                    self.tool_calls.complete(index, out, self.source);
                }
            }
            "response.completed" | "response.incomplete" | "response.failed" => {
                self.tool_calls.complete_all(out, self.source);
                if let Some(usage) = data.pointer("/response/usage") {
                    self.usage = responses::parse_usage(Some(usage));
                }
                self.stop = Some(responses::parse_stop_reason(
                    data.pointer("/response/status").and_then(Value::as_str),
                    data.pointer("/response/incomplete_details/reason")
                        .and_then(Value::as_str),
                    self.tool_calls.any(),
                ));
            }
            _ => {}
        }
    }

    /// Map an `output_index` onto a dense tool-call slot.
    ///
    /// The output index counts *all* items — a reasoning item and a message
    /// occupy positions too — so using it directly would leave holes and make
    /// the call indices the client sees depend on how much the model thought.
    fn responses_slot(&mut self, output_index: usize) -> usize {
        match self
            .responses_call_index
            .iter()
            .position(|seen| *seen == output_index)
        {
            Some(slot) => slot,
            None => {
                self.responses_call_index.push(output_index);
                self.responses_call_index.len() - 1
            }
        }
    }
}

fn index_of(data: &Value) -> usize {
    data.get("index")
        .and_then(Value::as_u64)
        .and_then(|i| usize::try_from(i).ok())
        .unwrap_or(0)
}

fn output_index(data: &Value) -> usize {
    data.get("output_index")
        .and_then(Value::as_u64)
        .and_then(|i| usize::try_from(i).ok())
        .unwrap_or(0)
}

fn anthropic_usage(usage: &Value) -> Usage {
    let n = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    Usage {
        input: n("input_tokens"),
        cached_input: n("cache_read_input_tokens"),
        output: n("output_tokens"),
        reasoning: 0,
    }
}

// ---------------------------------------------------------------------------
// Emitters: Delta → wire SSE
// ---------------------------------------------------------------------------

/// Writes [`Delta`]s as one wire's event stream.
#[derive(Debug)]
struct Emitter {
    target: Protocol,
    model: String,
    id: String,
    started: bool,
    /// Anthropic: whether the text block at index 0 is open.
    text_open: bool,
    /// Anthropic: next content-block index to allocate.
    next_index: usize,
    /// Responses: next output index.
    output_index: usize,
}

impl Emitter {
    fn new(target: Protocol, model: String) -> Self {
        Self {
            target,
            model,
            id: String::new(),
            started: false,
            text_open: false,
            next_index: 0,
            output_index: 0,
        }
    }

    fn write(&mut self, delta: &Delta, out: &mut Vec<u8>) {
        match self.target {
            Protocol::AnthropicMessages => self.write_anthropic(delta, out),
            Protocol::OpenAiResponses => self.write_responses(delta, out),
            Protocol::OpenAiChat => self.write_chat(delta, out),
        }
    }

    // -- Anthropic ---------------------------------------------------------

    fn write_anthropic(&mut self, delta: &Delta, out: &mut Vec<u8>) {
        match delta {
            Delta::Start { id, .. } => {
                if self.started {
                    return;
                }
                self.started = true;
                self.id = if id.is_empty() {
                    "msg_ironwire".to_string()
                } else {
                    id.clone()
                };
                write_event(
                    out,
                    "message_start",
                    &json!({
                        "type": "message_start",
                        "message": {
                            "id": self.id,
                            "type": "message",
                            "role": "assistant",
                            "model": self.model,
                            "content": [],
                            "stop_reason": Value::Null,
                            "stop_sequence": Value::Null,
                            "usage": {"input_tokens": 0, "output_tokens": 0},
                        },
                    }),
                );
            }
            Delta::Text(text) => {
                self.ensure_anthropic_text(out);
                write_event(
                    out,
                    "content_block_delta",
                    &json!({
                        "type": "content_block_delta",
                        "index": 0,
                        "delta": {"type": "text_delta", "text": text},
                    }),
                );
            }
            // A `thinking` block needs a signature Anthropic issued, and one it
            // did not issue is rejected when the client replays it. There is no
            // shape for unsigned reasoning here, so it does not travel — see
            // the emitters' rule in `docs/TRANSLATION.md` §6.
            Delta::ReasoningText(_) => {}
            Delta::ToolCall {
                id,
                name,
                arguments,
                ..
            } => {
                // Anthropic closes a content block before opening the next, and
                // a client tracking blocks by index has every right to expect
                // that. Leaving the text block open while a `tool_use` starts
                // beside it is a shape the real API never produces.
                self.close_anthropic_text(out);
                let index = self.next_index;
                self.next_index = index + 1;
                let block = json!({
                    "type": "tool_use",
                    "id": tool_ids::encode(id, Protocol::AnthropicMessages),
                    "name": name,
                    "input": serde_json::from_str::<Value>(arguments).unwrap_or_else(|_| json!({})),
                });
                write_event(
                    out,
                    "content_block_start",
                    &json!({
                        "type": "content_block_start",
                        "index": index,
                        "content_block": block,
                    }),
                );
                write_event(
                    out,
                    "content_block_stop",
                    &json!({"type": "content_block_stop", "index": index}),
                );
            }
            Delta::Stop { reason, usage } => {
                self.close_anthropic_text(out);
                let (stop_reason, stop_sequence) = anthropic::emit_stop_reason(reason);
                write_event(
                    out,
                    "message_delta",
                    &json!({
                        "type": "message_delta",
                        "delta": {"stop_reason": stop_reason, "stop_sequence": stop_sequence},
                        "usage": anthropic::emit_usage(usage),
                    }),
                );
                write_event(out, "message_stop", &json!({"type": "message_stop"}));
            }
        }
    }

    fn close_anthropic_text(&mut self, out: &mut Vec<u8>) {
        if !self.text_open {
            return;
        }
        self.text_open = false;
        write_event(
            out,
            "content_block_stop",
            &json!({"type": "content_block_stop", "index": 0}),
        );
    }

    fn ensure_anthropic_text(&mut self, out: &mut Vec<u8>) {
        if self.text_open {
            return;
        }
        self.text_open = true;
        self.next_index = 1;
        write_event(
            out,
            "content_block_start",
            &json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": {"type": "text", "text": ""},
            }),
        );
    }

    // -- Chat Completions --------------------------------------------------

    fn write_chat(&mut self, delta: &Delta, out: &mut Vec<u8>) {
        match delta {
            Delta::Start { id, .. } => {
                self.id = if id.is_empty() {
                    "chatcmpl_ironwire".to_string()
                } else {
                    id.clone()
                };
                self.started = true;
                self.write_chat_chunk(
                    out,
                    json!({"role": "assistant", "content": ""}),
                    Value::Null,
                );
            }
            Delta::Text(text) => {
                self.write_chat_chunk(out, json!({"content": text}), Value::Null);
            }
            Delta::ReasoningText(_) => {}
            Delta::ToolCall {
                index,
                id,
                name,
                arguments,
            } => self.write_chat_chunk(
                out,
                json!({"tool_calls": [{
                    "index": index,
                    "id": tool_ids::encode(id, Protocol::OpenAiChat),
                    "type": "function",
                    "function": {"name": name, "arguments": arguments},
                }]}),
                Value::Null,
            ),
            Delta::Stop { reason, usage } => {
                self.write_chat_chunk(out, json!({}), chat::emit_stop_reason(reason));
                // The usage-only tail, which is what `include_usage` asks for.
                write_data(
                    out,
                    &json!({
                        "id": self.id,
                        "object": "chat.completion.chunk",
                        "model": self.model,
                        "choices": [],
                        "usage": chat::emit_usage(usage),
                    }),
                );
                out.extend_from_slice(b"data: [DONE]\n\n");
            }
        }
    }

    fn write_chat_chunk(&mut self, out: &mut Vec<u8>, delta: Value, finish: Value) {
        write_data(
            out,
            &json!({
                "id": self.id,
                "object": "chat.completion.chunk",
                "model": self.model,
                "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
            }),
        );
    }

    // -- Responses ---------------------------------------------------------

    fn write_responses(&mut self, delta: &Delta, out: &mut Vec<u8>) {
        match delta {
            Delta::Start { id, .. } => {
                if self.started {
                    return;
                }
                self.started = true;
                self.id = if id.is_empty() {
                    "resp_ironwire".to_string()
                } else {
                    id.clone()
                };
                write_event(
                    out,
                    "response.created",
                    &json!({
                        "type": "response.created",
                        "response": {
                            "id": self.id,
                            "object": "response",
                            "model": self.model,
                            "status": "in_progress",
                            "output": [],
                        },
                    }),
                );
            }
            Delta::Text(text) => write_event(
                out,
                "response.output_text.delta",
                &json!({
                    "type": "response.output_text.delta",
                    "output_index": self.output_index,
                    "delta": text,
                }),
            ),
            Delta::ReasoningText(_) => {}
            Delta::ToolCall {
                id,
                name,
                arguments,
                ..
            } => {
                self.output_index += 1;
                let item = json!({
                    "type": "function_call",
                    "call_id": tool_ids::encode(id, Protocol::OpenAiResponses),
                    "name": name,
                    "arguments": arguments,
                });
                write_event(
                    out,
                    "response.output_item.added",
                    &json!({
                        "type": "response.output_item.added",
                        "output_index": self.output_index,
                        "item": item,
                    }),
                );
                write_event(
                    out,
                    "response.output_item.done",
                    &json!({
                        "type": "response.output_item.done",
                        "output_index": self.output_index,
                        "item": item,
                    }),
                );
            }
            Delta::Stop { reason, usage } => {
                let (status, incomplete) = responses::emit_stop_reason(reason);
                let name = if incomplete.is_some() {
                    "response.incomplete"
                } else {
                    "response.completed"
                };
                let mut response = serde_json::Map::new();
                response.insert("id".into(), json!(self.id));
                response.insert("object".into(), json!("response"));
                response.insert("model".into(), json!(self.model));
                response.insert("status".into(), status);
                if let Some(incomplete) = incomplete {
                    response.insert("incomplete_details".into(), incomplete);
                }
                response.insert("usage".into(), responses::emit_usage(usage));
                write_event(
                    out,
                    name,
                    &json!({"type": name, "response": Value::Object(response)}),
                );
            }
        }
    }
}

fn write_event(out: &mut Vec<u8>, name: &str, payload: &Value) {
    out.extend_from_slice(format!("event: {name}\n").as_bytes());
    write_data(out, payload);
}

fn write_data(out: &mut Vec<u8>, payload: &Value) {
    out.extend_from_slice(b"data: ");
    out.extend_from_slice(payload.to_string().as_bytes());
    out.extend_from_slice(b"\n\n");
}

// ---------------------------------------------------------------------------
// The driver
// ---------------------------------------------------------------------------

/// Translates one wire's event stream into another's.
///
/// Feed it upstream SSE bytes with [`Translator::push`]; it returns the bytes to
/// forward downstream. Call [`Translator::finish`] when the upstream ends.
#[derive(Debug)]
pub struct Translator {
    parser: Parser,
    emitter: Emitter,
    buffer: Vec<u8>,
    /// Set after discarding an oversized frame: the bytes we dropped may have
    /// been the middle of one, so everything until the next boundary is
    /// unusable too. Clearing the buffer alone is not enough — the surviving
    /// junk prefix would be glued to the next real frame and swallow it.
    resyncing: bool,
    closed: bool,
}

impl Translator {
    /// A translator from `source` to `target`, reporting `requested_model` back
    /// to the client — the model it asked for, never the one that served it.
    #[must_use]
    pub fn new(source: Protocol, target: Protocol, requested_model: impl Into<String>) -> Self {
        Self {
            parser: Parser::new(source),
            emitter: Emitter::new(target, requested_model.into()),
            buffer: Vec::new(),
            resyncing: false,
            closed: false,
        }
    }

    /// `ln p(chosen token)` for each token the model emitted, in order.
    ///
    /// Empty unless the request asked for log-probabilities and the backend
    /// honoured it. Log-probabilities rather than probabilities: the reduction
    /// exponentiates once, in `f64`, and narrowing earlier is what
    /// `ironwire_core::confidence` documents as the thing to avoid.
    #[must_use]
    pub fn token_logprobs(&self) -> &[f64] {
        &self.parser.token_logprobs
    }

    /// Feed upstream bytes; returns downstream bytes to forward.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.buffer.extend_from_slice(chunk);
        let mut out = Vec::new();

        // Discard through the next boundary before parsing anything: after an
        // oversized frame we are mid-garbage, and treating the remainder as a
        // frame would glue it to the next real one.
        if self.resyncing {
            match find_boundary(&self.buffer) {
                Some(pos) => {
                    self.buffer.drain(..pos);
                    self.resyncing = false;
                }
                None => {
                    self.buffer.clear();
                    return out;
                }
            }
        }

        while let Some(pos) = find_boundary(&self.buffer) {
            let frame: Vec<u8> = self.buffer.drain(..pos).collect();
            self.consume(&frame, &mut out);
        }

        // No boundary in sight and the buffer is past anything a real frame
        // could be: the upstream is broken or hostile. Drop what we have and
        // pick up at the next boundary rather than growing without limit — a
        // lost frame is a lost delta, and an OOM is every conversation on the
        // machine.
        if self.buffer.len() > MAX_FRAME_BYTES {
            tracing::warn!(
                bytes = self.buffer.len(),
                "discarding an oversized SSE frame with no boundary"
            );
            self.buffer.clear();
            self.resyncing = true;
        }
        out
    }

    /// Close the stream, emitting whatever terminal events are still owed.
    pub fn finish(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        // A tail we were discarding is not a frame; parsing it would be reading
        // the middle of one.
        if self.resyncing {
            self.buffer.clear();
        }
        if !self.buffer.is_empty() {
            let frame = std::mem::take(&mut self.buffer);
            self.consume(&frame, &mut out);
        }
        self.close(&mut out);
        out
    }

    fn consume(&mut self, frame: &[u8], out: &mut Vec<u8>) {
        let Ok(text) = std::str::from_utf8(frame) else {
            return;
        };
        let mut event: Option<&str> = None;
        let mut data = String::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("event:") {
                event = Some(rest.trim());
            } else if let Some(rest) = line.strip_prefix("data:") {
                data.push_str(rest.trim_start());
            }
        }
        if data.is_empty() {
            return;
        }
        if data == "[DONE]" {
            self.close(out);
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            return;
        };

        let mut deltas = Vec::new();
        self.parser.feed(event, &value, &mut deltas);
        for delta in &deltas {
            // A stop from the wire closes the stream here, so a second one —
            // or a trailing `[DONE]` — cannot write a second terminal event.
            if matches!(delta, Delta::Stop { .. }) {
                if self.closed {
                    continue;
                }
                self.closed = true;
            }
            self.emitter.write(delta, out);
        }
    }

    fn close(&mut self, out: &mut Vec<u8>) {
        if self.closed {
            return;
        }
        self.closed = true;
        // A stream that produced nothing at all still owes the client a
        // well-formed message; an agent waiting on the terminal event otherwise
        // hangs until its own timeout.
        let mut deltas = Vec::new();
        self.parser.finish(&mut deltas);
        for delta in &deltas {
            self.emitter.write(delta, out);
        }
    }
}

/// End of the first `\n\n`-delimited frame, if there is one.
fn find_boundary(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(2)
        .position(|w| w == b"\n\n")
        .map(|pos| pos + 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::ToolCallId;

    const EVERY: [Protocol; 3] = [
        Protocol::AnthropicMessages,
        Protocol::OpenAiResponses,
        Protocol::OpenAiChat,
    ];

    fn chat_stream() -> Vec<u8> {
        let frames = [
            r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{"role":"assistant","content":""}}]}"#,
            r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":"Hello"}}]}"#,
            r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":" there"}}]}"#,
            r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"Bash","arguments":"{\"cmd\":"}}]}}]}"#,
            r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"ls\"}"}}]}}]}"#,
            r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
            r#"data: {"id":"chatcmpl-1","choices":[],"usage":{"prompt_tokens":100,"completion_tokens":5}}"#,
            "data: [DONE]",
        ];
        frames.join("\n\n").into_bytes()
    }

    fn responses_stream() -> Vec<u8> {
        let frames = [
            concat!(
                "event: response.created\n",
                r#"data: {"type":"response.created","response":{"id":"resp_1","model":"gpt-5.6"}}"#
            ),
            concat!(
                "event: response.output_text.delta\n",
                r#"data: {"type":"response.output_text.delta","output_index":0,"delta":"Hello"}"#
            ),
            concat!(
                "event: response.output_text.delta\n",
                r#"data: {"type":"response.output_text.delta","output_index":0,"delta":" there"}"#
            ),
            concat!(
                "event: response.output_item.added\n",
                r#"data: {"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"call_1","name":"Bash"}}"#
            ),
            concat!(
                "event: response.function_call_arguments.delta\n",
                r#"data: {"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"cmd\":\"ls\"}"}"#
            ),
            concat!(
                "event: response.function_call_arguments.done\n",
                r#"data: {"type":"response.function_call_arguments.done","output_index":1,"arguments":"{\"cmd\":\"ls\"}"}"#
            ),
            concat!(
                "event: response.completed\n",
                r#"data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","usage":{"input_tokens":100,"output_tokens":5}}}"#
            ),
        ];
        frames.join("\n\n").into_bytes()
    }

    fn anthropic_stream() -> Vec<u8> {
        let frames = [
            concat!(
                "event: message_start\n",
                r#"data: {"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":100,"output_tokens":0}}}"#
            ),
            concat!(
                "event: content_block_start\n",
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#
            ),
            concat!(
                "event: content_block_delta\n",
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#
            ),
            concat!(
                "event: content_block_delta\n",
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" there"}}"#
            ),
            concat!(
                "event: content_block_stop\n",
                r#"data: {"type":"content_block_stop","index":0}"#
            ),
            concat!(
                "event: content_block_start\n",
                r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"Bash","input":{}}}"#
            ),
            concat!(
                "event: content_block_delta\n",
                r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"cmd\":\"ls\"}"}}"#
            ),
            concat!(
                "event: content_block_stop\n",
                r#"data: {"type":"content_block_stop","index":1}"#
            ),
            concat!(
                "event: message_delta\n",
                r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}"#
            ),
            concat!("event: message_stop\n", r#"data: {"type":"message_stop"}"#),
        ];
        frames.join("\n\n").into_bytes()
    }

    fn source_stream(protocol: Protocol) -> Vec<u8> {
        match protocol {
            Protocol::AnthropicMessages => anthropic_stream(),
            Protocol::OpenAiResponses => responses_stream(),
            Protocol::OpenAiChat => chat_stream(),
        }
    }

    fn run(source: Protocol, target: Protocol, bytes: &[u8]) -> String {
        let mut translator = Translator::new(source, target, "requested-model");
        let mut out = translator.push(bytes);
        out.extend(translator.finish());
        String::from_utf8(out).expect("utf-8 out")
    }

    /// The matrix. Every stream, onto every wire: the text arrives, the tool
    /// call arrives whole and parseable, and the terminal event says a call is
    /// pending rather than that the turn is over.
    #[test]
    fn every_stream_pair_delivers_the_text_and_the_whole_tool_call() {
        for source in EVERY {
            let bytes = source_stream(source);
            for target in EVERY {
                let out = run(source, target, &bytes);
                assert!(
                    out.contains("Hello") && out.contains(" there"),
                    "{source} → {target} lost text: {out}"
                );
                assert!(
                    out.contains("Bash"),
                    "{source} → {target} lost the tool call: {out}"
                );
                assert!(
                    out.contains(r#"cmd\":\"ls"#) || out.contains(r#""cmd":"ls""#),
                    "{source} → {target} lost the call arguments: {out}"
                );
                assert!(
                    out.contains("requested-model"),
                    "{source} → {target} reported the wrong model: {out}"
                );
            }
        }
    }

    // -- log-probability capture ------------------------------------------

    fn chat_frame(value: serde_json::Value) -> String {
        format!("data: {value}\n\n")
    }

    /// A stream that was never asked for log-probabilities accumulates none,
    /// which is every stream by default.
    #[test]
    fn nothing_is_accumulated_when_the_backend_sends_no_logprobs() {
        let mut t = Translator::new(Protocol::OpenAiChat, Protocol::AnthropicMessages, "m");
        t.push(&chat_stream());
        t.finish();
        assert!(t.token_logprobs().is_empty());
    }

    #[test]
    fn logprobs_accumulate_across_frames_in_order() {
        let mut t = Translator::new(Protocol::OpenAiChat, Protocol::AnthropicMessages, "m");
        for (text, logprob) in [("a", -0.5f64), ("b", -1.5)] {
            t.push(
                chat_frame(json!({"id": "c", "choices": [{
                    "index": 0,
                    "delta": {"content": text},
                    "logprobs": {"content": [{"token": text, "logprob": logprob}]}
                }]}))
                .as_bytes(),
            );
        }
        assert_eq!(t.token_logprobs(), [-0.5, -1.5]);
    }

    /// `logprobs` is a sibling of `delta`, so a frame carrying a distribution
    /// and a finish reason but no delta must still be counted. Accumulating
    /// after the delta check would silently drop the final token.
    #[test]
    fn logprobs_survive_a_frame_with_no_delta() {
        let mut t = Translator::new(Protocol::OpenAiChat, Protocol::AnthropicMessages, "m");
        t.push(
            chat_frame(json!({"id": "c", "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "logprobs": {"content": [{"token": "!", "logprob": -0.25}]}
            }]}))
            .as_bytes(),
        );
        assert_eq!(t.token_logprobs(), [-0.25]);
    }

    /// The same frame twice is two tokens; one frame is not counted twice.
    #[test]
    fn a_frame_is_counted_once() {
        let mut t = Translator::new(Protocol::OpenAiChat, Protocol::AnthropicMessages, "m");
        let frame = chat_frame(json!({"id": "c", "choices": [{
            "index": 0,
            "delta": {"content": "x"},
            "logprobs": {"content": [{"token": "x", "logprob": -0.1}]}
        }]}));
        t.push(frame.as_bytes());
        assert_eq!(t.token_logprobs().len(), 1);
        t.push(frame.as_bytes());
        assert_eq!(t.token_logprobs().len(), 2);
    }

    /// An entry with no usable `logprob` is skipped rather than repaired: a
    /// stand-in value would be folded into the mean and never seen again.
    #[test]
    fn a_malformed_logprob_entry_is_skipped() {
        let mut t = Translator::new(Protocol::OpenAiChat, Protocol::AnthropicMessages, "m");
        t.push(
            chat_frame(json!({"id": "c", "choices": [{
                "index": 0,
                "delta": {"content": "x"},
                "logprobs": {"content": [
                    {"token": "ok", "logprob": -0.5},
                    {"token": "bad", "logprob": "not a number"},
                    {"token": "worse"}
                ]}
            }]}))
            .as_bytes(),
        );
        assert_eq!(t.token_logprobs(), [-0.5]);
    }

    /// The fourth place an upstream controls how much we allocate. An endpoint
    /// that streams `logprobs` entries forever must not grow this vector
    /// forever — the aggregate over the prefix is still a true statement, and
    /// an OOM is every conversation on the machine.
    #[test]
    fn accumulation_stops_at_the_cap_rather_than_growing() {
        let mut t = Translator::new(Protocol::OpenAiChat, Protocol::AnthropicMessages, "m");
        let entries: Vec<serde_json::Value> = (0..4096)
            .map(|_| json!({"token": "x", "logprob": -0.1}))
            .collect();
        let frame = chat_frame(json!({"id": "c", "choices": [{
            "index": 0,
            "delta": {"content": "x"},
            "logprobs": {"content": entries}
        }]}));
        for _ in 0..40 {
            t.push(frame.as_bytes());
        }
        assert_eq!(t.token_logprobs().len(), chat::MAX_TOKEN_LOGPROBS);
    }

    /// Capture must not change a single downstream byte. The whole safety
    /// argument for asking for log-probabilities is that the client sees the
    /// same stream it would have seen.
    #[test]
    fn capture_does_not_alter_what_the_client_receives() {
        let frames = |with_logprobs: bool| {
            let mut t = Translator::new(Protocol::OpenAiChat, Protocol::AnthropicMessages, "m");
            let mut choice = json!({"index": 0, "delta": {"content": "Hello"}});
            if with_logprobs {
                choice["logprobs"] = json!({"content": [{"token": "Hello", "logprob": -0.1}]});
            }
            let mut out = t.push(chat_frame(json!({"id": "c", "choices": [choice]})).as_bytes());
            out.extend(t.finish());
            out
        };
        assert_eq!(
            frames(false),
            frames(true),
            "asking for log-probabilities changed what the client receives"
        );
    }

    /// The single highest-consequence field in the translation. Reported as a
    /// finished turn, the agent stops instead of running the call it was just
    /// handed — on every one of the nine pairs.
    #[test]
    fn a_pending_tool_call_is_never_reported_as_a_finished_turn() {
        for source in EVERY {
            let bytes = source_stream(source);
            for target in EVERY {
                let out = run(source, target, &bytes);
                match target {
                    Protocol::AnthropicMessages => assert!(
                        out.contains(r#""stop_reason":"tool_use""#),
                        "{source} → {target}: {out}"
                    ),
                    Protocol::OpenAiChat => assert!(
                        out.contains(r#""finish_reason":"tool_calls""#),
                        "{source} → {target}: {out}"
                    ),
                    // This wire has no stop reason at all: a turn ending in a
                    // call is `completed`, and the call in the output is what
                    // says more is coming. So the assertion is that the call
                    // item is actually there.
                    Protocol::OpenAiResponses => assert!(
                        out.contains(r#""type":"function_call""#),
                        "{source} → {target}: {out}"
                    ),
                }
            }
        }
    }

    #[test]
    fn usage_survives_every_pair() {
        for source in EVERY {
            let bytes = source_stream(source);
            for target in EVERY {
                let out = run(source, target, &bytes);
                assert!(
                    out.contains("100"),
                    "{source} → {target} lost the input token count: {out}"
                );
                assert!(
                    out.contains(r#""output_tokens":5"#)
                        || out.contains(r#""completion_tokens":5"#),
                    "{source} → {target} lost the output token count: {out}"
                );
            }
        }
    }

    /// Every target owes the client a terminal event, even when the upstream
    /// says nothing at all. An agent waiting for one otherwise hangs until its
    /// own timeout.
    #[test]
    fn an_empty_stream_still_closes_the_message() {
        for target in EVERY {
            let out = run(Protocol::OpenAiChat, target, b"");
            assert!(!out.is_empty(), "{target} produced nothing at all");
            let terminal = match target {
                Protocol::AnthropicMessages => "message_stop",
                Protocol::OpenAiResponses => "response.completed",
                Protocol::OpenAiChat => "[DONE]",
            };
            assert!(out.contains(terminal), "{target}: {out}");
        }
    }

    #[test]
    fn a_terminal_event_is_written_exactly_once() {
        // `[DONE]` follows the finish_reason frame, and `finish` runs after
        // both. Neither may produce a second terminal event.
        let out = run(
            Protocol::OpenAiChat,
            Protocol::AnthropicMessages,
            &chat_stream(),
        );
        assert_eq!(out.matches("event: message_stop").count(), 1, "{out}");
    }

    /// Anthropic closes a content block before opening the next, and a client
    /// tracking blocks by index has every right to expect that. An open text
    /// block sitting beside a `tool_use` is a shape the real API never emits.
    #[test]
    fn content_blocks_never_overlap() {
        for source in EVERY {
            let out = run(source, Protocol::AnthropicMessages, &source_stream(source));
            let mut open: Option<i64> = None;
            for line in out.lines().filter(|line| line.starts_with("data: ")) {
                let Ok(value) = serde_json::from_str::<Value>(&line[6..]) else {
                    continue;
                };
                match value.get("type").and_then(Value::as_str) {
                    Some("content_block_start") => {
                        assert!(open.is_none(), "{source}: a block opened inside another");
                        open = value.get("index").and_then(Value::as_i64);
                    }
                    Some("content_block_stop") => {
                        assert_eq!(
                            open,
                            value.get("index").and_then(Value::as_i64),
                            "{source}: closed a block that was not the open one"
                        );
                        open = None;
                    }
                    _ => {}
                }
            }
            assert!(open.is_none(), "{source} left a content block open");
        }
    }

    #[test]
    fn text_is_forwarded_incrementally_rather_than_buffered() {
        // The whole point of streaming: the first token reaches the client
        // before the upstream has finished.
        let mut translator = Translator::new(
            Protocol::OpenAiChat,
            Protocol::AnthropicMessages,
            "requested-model",
        );
        let first = translator.push(
            br#"data: {"id":"c","choices":[{"index":0,"delta":{"content":"Hello"}}]}

"#,
        );
        let text = String::from_utf8(first).expect("utf-8");
        assert!(text.contains("Hello"), "{text}");
        assert!(!text.contains("message_stop"));
    }

    /// An index the upstream controls drives a `Vec::resize`. IronWire points at
    /// whatever OpenAI-compatible endpoint a user names, so this is reachable
    /// rather than theoretical.
    #[test]
    fn an_implausible_tool_call_index_is_refused_rather_than_allocated() {
        let out = run(
            Protocol::OpenAiChat,
            Protocol::AnthropicMessages,
            br#"data: {"id":"c","choices":[{"index":0,"delta":{"tool_calls":[{"index":4000000000,"id":"x","function":{"name":"Bash","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}

"#,
        );
        assert!(!out.contains("Bash"), "an absurd index was honoured: {out}");
        assert!(out.contains("message_stop"));
    }

    #[test]
    fn an_oversized_frame_is_discarded_and_the_stream_resyncs() {
        let mut translator = Translator::new(
            Protocol::OpenAiChat,
            Protocol::AnthropicMessages,
            "requested-model",
        );
        let mut junk = b"data: ".to_vec();
        junk.extend(std::iter::repeat_n(b'x', MAX_FRAME_BYTES + 1));
        let _ = translator.push(&junk);
        // The tail of the junk frame, then a real one.
        let out = translator.push(
            br#"trailing junk

data: {"id":"c","choices":[{"index":0,"delta":{"content":"after"}}]}

"#,
        );
        let text = String::from_utf8(out).expect("utf-8");
        assert!(text.contains("after"), "did not resync: {text}");
    }

    #[test]
    fn a_frame_that_is_not_json_is_skipped_rather_than_fatal() {
        let out = run(
            Protocol::OpenAiChat,
            Protocol::AnthropicMessages,
            b"data: {not json\n\ndata: {\"id\":\"c\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"}}]}\n\n",
        );
        assert!(out.contains("ok"), "{out}");
    }

    /// Reasoning does not cross a wire boundary in any form: an unsigned
    /// `thinking` block is rejected when the client replays it, and turning the
    /// summary into visible text would put words in the model's mouth.
    #[test]
    fn streamed_reasoning_does_not_reach_a_foreign_client() {
        let out = run(
            Protocol::OpenAiChat,
            Protocol::AnthropicMessages,
            br#"data: {"id":"c","choices":[{"index":0,"delta":{"reasoning_content":"PRIVATE","content":"answer"}}]}

"#,
        );
        assert!(!out.contains("PRIVATE"), "{out}");
        assert!(out.contains("answer"));
    }

    /// The output index counts every item, including reasoning, so a model that
    /// thinks first would otherwise hand the client a call at index 2 and leave
    /// holes at 0 and 1.
    #[test]
    fn responses_call_indices_are_dense_however_much_the_model_thought() {
        let stream = [
            concat!("event: response.output_item.added\n", r#"data: {"type":"response.output_item.added","output_index":7,"item":{"type":"function_call","call_id":"call_a","name":"A"}}"#),
            concat!("event: response.function_call_arguments.done\n", r#"data: {"type":"response.function_call_arguments.done","output_index":7,"arguments":"{}"}"#),
            concat!("event: response.output_item.added\n", r#"data: {"type":"response.output_item.added","output_index":9,"item":{"type":"function_call","call_id":"call_b","name":"B"}}"#),
            concat!("event: response.function_call_arguments.done\n", r#"data: {"type":"response.function_call_arguments.done","output_index":9,"arguments":"{}"}"#),
        ]
        .join("\n\n");
        let out = run(
            Protocol::OpenAiResponses,
            Protocol::OpenAiChat,
            stream.as_bytes(),
        );
        assert!(out.contains(r#""index":0"#), "{out}");
        assert!(out.contains(r#""index":1"#), "{out}");
        assert!(out.contains("call_a") && out.contains("call_b"));
    }

    /// A foreign id has to come back in a shape the Anthropic client will
    /// accept, and reverse to the original when it is replayed.
    #[test]
    fn a_foreign_tool_id_reaches_an_anthropic_client_in_its_own_namespace() {
        let out = run(
            Protocol::OpenAiChat,
            Protocol::AnthropicMessages,
            &chat_stream(),
        );
        assert!(out.contains("toolu_xw_call_1"), "{out}");
        assert_eq!(
            tool_ids::decode("toolu_xw_call_1", Protocol::AnthropicMessages),
            ToolCallId::from("call_1")
        );
    }
}
