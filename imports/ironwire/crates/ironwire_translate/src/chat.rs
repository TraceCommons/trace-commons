//! OpenAI Chat Completions ⇄ the IR.
//!
//! The weakest of the three wires, and the reason the IR is its own type rather
//! than this one: no typed content blocks, no reasoning items, no response id to
//! resume from. Pivoting here would have quietly degraded every route between
//! the two formats that are richer than it.
//!
//! Its emit side has a second job — reproducing what the pairwise
//! `anthropic_to_chat_completions` produced, so the tests that covered the only
//! lane this build ever ran are the regression suite for the pivot.

use serde_json::{Map, Value, json};

use ironwire_core::protocol::Protocol;

use crate::ir::{
    Block, Completion, Conversation, Dropped, ImageSource, Params, Role, StopReason, SystemChunk,
    ToolChoice, ToolDef, Turn, Usage, flatten_text,
};
use crate::tool_ids;

const ME: Protocol = Protocol::OpenAiChat;

// ---------------------------------------------------------------------------
// Request: wire → IR
// ---------------------------------------------------------------------------

/// Parse a Chat Completions request.
#[must_use]
pub fn parse_request(body: &Value) -> Conversation {
    let mut system = Vec::new();
    let mut turns: Vec<Turn> = Vec::new();

    for message in body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
    {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        match role {
            "system" | "developer" => system.push(SystemChunk {
                text: flatten_text(message.get("content")),
                cache_breakpoint: false,
            }),
            // A tool result is a message of its own here and a block in the IR,
            // so it joins the turn in progress rather than starting one.
            "tool" => {
                let block = Block::ToolResult {
                    id: tool_ids::decode(
                        message
                            .get("tool_call_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        ME,
                    ),
                    content: flatten_text(message.get("content")),
                    // Chat Completions has nowhere to say a tool failed, so the
                    // flag is lost on this wire in both directions. The text
                    // usually says so, and inventing the boolean from a
                    // substring match would be worse than not having it.
                    is_error: false,
                };
                push_block(&mut turns, Role::User, block);
            }
            "assistant" => {
                let mut blocks = Vec::new();
                let text = flatten_text(message.get("content"));
                if !text.is_empty() {
                    blocks.push(Block::Text(text));
                }
                if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        blocks.push(parse_tool_call(call));
                    }
                }
                turns.push(Turn {
                    role: Role::Assistant,
                    blocks,
                });
            }
            _ => {
                let mut blocks = Vec::new();
                match message.get("content") {
                    Some(Value::Array(parts)) => {
                        for part in parts {
                            blocks.push(parse_content_part(part));
                        }
                    }
                    other => blocks.push(Block::Text(flatten_text(other))),
                }
                turns.push(Turn {
                    role: Role::User,
                    blocks,
                });
            }
        }
    }

    Conversation {
        system,
        turns,
        tools: body
            .get("tools")
            .and_then(Value::as_array)
            .map(|tools| tools.iter().map(parse_tool).collect())
            .unwrap_or_default(),
        // Every tool on this wire is a function; only Responses has shapes
        // that are not (`responses::sort_tool`).
        unexpressible_tools: Vec::new(),
        tool_choice: body.get("tool_choice").and_then(parse_tool_choice),
        params: Params {
            max_tokens: body
                .get("max_completion_tokens")
                .or_else(|| body.get("max_tokens"))
                .and_then(Value::as_u64),
            temperature: body.get("temperature").and_then(Value::as_f64),
            top_p: body.get("top_p").and_then(Value::as_f64),
            stop: match body.get("stop") {
                Some(Value::String(one)) => vec![one.clone()],
                Some(Value::Array(many)) => many
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToString::to_string)
                    .collect(),
                _ => Vec::new(),
            },
            reasoning: body
                .get("reasoning_effort")
                .and_then(Value::as_str)
                .map(|effort| crate::ir::ReasoningRequest {
                    effort: Some(effort.to_string()),
                    budget_tokens: None,
                    summary: false,
                }),
            stream: body
                .get("stream")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
            // Read back so a same-wire round trip is lossless. The capture
            // setting ORs into this at the pipeline; a client that asked for
            // itself is honoured either way.
            logprobs: body
                .get("logprobs")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
        },
    }
}

/// Append to the open user turn, or start one.
///
/// Chat Completions emits a `role: "tool"` message per result; Anthropic packs
/// them into one user turn. Coalescing here means the IR holds the shape both
/// can express, rather than one turn per result that Anthropic would then have
/// to merge back.
fn push_block(turns: &mut Vec<Turn>, role: Role, block: Block) {
    match turns.last_mut() {
        Some(turn) if turn.role == role => turn.blocks.push(block),
        _ => turns.push(Turn {
            role,
            blocks: vec![block],
        }),
    }
}

fn parse_content_part(part: &Value) -> Block {
    match part.get("type").and_then(Value::as_str) {
        Some("text") => Block::Text(
            part.get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ),
        Some("image_url") => {
            let url = part
                .pointer("/image_url/url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Block::Image(parse_image_url(url))
        }
        other => Block::Unknown {
            origin: ME,
            kind: other.unwrap_or("<no type field>").to_string(),
            raw: part.clone(),
        },
    }
}

/// `data:image/png;base64,AAAA` or a plain URL.
fn parse_image_url(url: &str) -> ImageSource {
    let Some(rest) = url.strip_prefix("data:") else {
        return ImageSource::Url(url.to_string());
    };
    match rest.split_once(";base64,") {
        Some((media_type, data)) => ImageSource::Base64 {
            media_type: media_type.to_string(),
            data: data.to_string(),
        },
        None => ImageSource::Url(url.to_string()),
    }
}

fn parse_tool_call(call: &Value) -> Block {
    let arguments = call
        .pointer("/function/arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}");
    Block::ToolUse {
        id: tool_ids::decode(
            call.get("id").and_then(Value::as_str).unwrap_or_default(),
            ME,
        ),
        name: call
            .pointer("/function/name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        // Chat Completions carries arguments as a JSON *string*; the IR carries
        // the parsed object. A model that emits malformed JSON must not take
        // the whole turn down — an empty object lets the agent see a failed
        // call and recover.
        input: serde_json::from_str(arguments).unwrap_or_else(|_| json!({})),
    }
}

fn parse_tool(tool: &Value) -> ToolDef {
    ToolDef {
        name: tool
            .pointer("/function/name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        description: tool
            .pointer("/function/description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        schema: tool
            .pointer("/function/parameters")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
    }
}

fn parse_tool_choice(choice: &Value) -> Option<ToolChoice> {
    match choice {
        Value::String(s) => match s.as_str() {
            "auto" => Some(ToolChoice::Auto),
            "required" => Some(ToolChoice::Required),
            "none" => Some(ToolChoice::None),
            _ => None,
        },
        other => other
            .pointer("/function/name")
            .and_then(Value::as_str)
            .map(|name| ToolChoice::Named(name.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Request: IR → wire
// ---------------------------------------------------------------------------

/// Write a Chat Completions request.
#[must_use]
pub fn emit_request(conversation: &Conversation, model: &str) -> (Value, Dropped) {
    let mut dropped = Dropped {
        // Not modelled by the IR, so no target can emit them. Named rather
        // than dropped in silence (`docs/TRANSLATION.md`).
        tools: conversation.unexpressible_tools.clone(),
        ..Dropped::default()
    };
    let mut messages: Vec<Value> = Vec::new();

    // One string, because that is all this wire has. The breakpoints are
    // counted rather than mapped: OpenAI-compatible endpoints cache
    // automatically or not at all.
    let system: Vec<&str> = conversation
        .system
        .iter()
        .inspect(|chunk| {
            if chunk.cache_breakpoint {
                dropped.cache_breakpoints += 1;
            }
        })
        .map(|chunk| chunk.text.as_str())
        .collect();
    if !system.is_empty() {
        messages.push(json!({"role": "system", "content": system.join("\n\n")}));
    }

    for turn in &conversation.turns {
        emit_turn(turn, &mut messages, &mut dropped);
    }

    let mut request = Map::new();
    request.insert("model".into(), json!(model));
    request.insert("messages".into(), Value::Array(messages));
    request.insert("stream".into(), json!(conversation.params.stream));
    if conversation.params.stream {
        // Without this the final chunk carries no usage and we lose all
        // accounting for the turn.
        request.insert("stream_options".into(), json!({"include_usage": true}));
    }
    if let Some(max_tokens) = conversation.params.max_tokens {
        request.insert("max_tokens".into(), json!(max_tokens));
    }
    if !conversation.params.stop.is_empty() {
        request.insert("stop".into(), json!(conversation.params.stop));
    }
    if let Some(temperature) = conversation.params.temperature {
        request.insert("temperature".into(), json!(temperature));
    }
    if let Some(top_p) = conversation.params.top_p {
        request.insert("top_p".into(), json!(top_p));
    }
    if let Some(effort) = conversation
        .params
        .reasoning
        .as_ref()
        .and_then(|r| r.effort.as_ref())
    {
        request.insert("reasoning_effort".into(), json!(effort));
    }
    // `logprobs` alone, never `top_logprobs`. The alternatives are model output
    // the user never sees, they inflate every frame, and the only consumer —
    // `ironwire_core::confidence` — is defined over the chosen token. Asking
    // for something nobody reads is bandwidth and exposure, both for free.
    //
    // This is the only emitter that writes it. Anthropic Messages has no such
    // parameter and rejects unknown fields; Responses has no boolean form.
    if conversation.params.logprobs {
        request.insert("logprobs".into(), json!(true));
    }

    if !conversation.tools.is_empty() {
        request.insert(
            "tools".into(),
            Value::Array(
                conversation
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type": "function",
                            "function": {
                                "name": tool.name,
                                "description": tool.description,
                                "parameters": tool.schema,
                            },
                        })
                    })
                    .collect(),
            ),
        );
        if let Some(choice) = &conversation.tool_choice {
            request.insert(
                "tool_choice".into(),
                match choice {
                    ToolChoice::Auto => json!("auto"),
                    ToolChoice::Required => json!("required"),
                    ToolChoice::None => json!("none"),
                    ToolChoice::Named(name) => {
                        json!({"type": "function", "function": {"name": name}})
                    }
                },
            );
        }
    }

    (Value::Object(request), dropped)
}

fn emit_turn(turn: &Turn, out: &mut Vec<Value>, dropped: &mut Dropped) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut parts: Vec<Value> = Vec::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    let mut has_image = false;

    for block in &turn.blocks {
        match block {
            Block::Text(text) if text.is_empty() => {}
            Block::Text(text) => {
                text_parts.push(text.clone());
                parts.push(json!({"type": "text", "text": text}));
            }
            Block::Image(source) => {
                has_image = true;
                parts.push(
                    json!({"type": "image_url", "image_url": {"url": emit_image_url(source)}}),
                );
            }
            Block::ToolUse { id, name, input } => tool_calls.push(json!({
                "id": tool_ids::encode(id, ME),
                "type": "function",
                "function": {
                    "name": name,
                    // A JSON *string* here. The single most common way this
                    // translation is written wrong.
                    "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".into()),
                },
            })),
            // Tool results must precede the rest of the turn: Anthropic packs
            // them into a user turn, Chat Completions gives each its own
            // message, and a provider reading them after the assistant turn
            // they answer will reject the sequence.
            Block::ToolResult { id, content, .. } => out.push(json!({
                "role": "tool",
                "tool_call_id": tool_ids::encode(id, ME),
                "content": content,
            })),
            // Reasoning does not cross a wire boundary in any form — not the
            // blob, which only its own provider can validate, and not the
            // summary either. Folding a summary into the visible content would
            // put the model's private reasoning into the transcript as though
            // it had said it, which is the same move `events.rs` refuses when
            // it will not write into a response stream.
            Block::Reasoning(_) => dropped.reasoning_blocks += 1,
            Block::Unknown { origin, kind, raw } => {
                if *origin == ME {
                    parts.push(raw.clone());
                } else {
                    dropped.note_unknown(kind);
                }
            }
        }
    }

    let role = match turn.role {
        Role::Assistant => "assistant",
        Role::User => "user",
    };

    if !tool_calls.is_empty() {
        let mut message = Map::new();
        message.insert("role".into(), json!("assistant"));
        // Required even when there is no prose alongside a call:
        // OpenAI-compatible servers commonly reject an assistant message with
        // no `content` key at all.
        message.insert(
            "content".into(),
            if text_parts.is_empty() {
                Value::Null
            } else {
                json!(text_parts.join("\n"))
            },
        );
        message.insert("tool_calls".into(), Value::Array(tool_calls));
        out.push(Value::Object(message));
        return;
    }

    // A multi-part body only when there is something a flat string cannot hold.
    // Every endpoint accepts the string form; the array form is newer and some
    // OpenAI-compatible servers do not implement it.
    if has_image {
        out.push(json!({"role": role, "content": parts}));
    } else if !text_parts.is_empty() {
        out.push(json!({"role": role, "content": text_parts.join("\n")}));
    }
}

fn emit_image_url(source: &ImageSource) -> String {
    match source {
        ImageSource::Base64 { media_type, data } => format!("data:{media_type};base64,{data}"),
        ImageSource::Url(url) => url.clone(),
    }
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/// Parse a non-streaming Chat Completions answer.
#[must_use]
pub fn parse_completion(response: &Value) -> Completion {
    let choice = response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first());
    let message = choice.and_then(|c| c.get("message"));

    let mut blocks = Vec::new();
    if let Some(reasoning) = message
        .and_then(|m| m.get("reasoning_content"))
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        // Several OpenAI-compatible endpoints expose the reasoning summary
        // under this name. It is prose, not provider-private state.
        blocks.push(Block::Reasoning(crate::ir::Reasoning {
            origin: ME,
            summary: Some(reasoning.to_string()),
            opaque: Value::Null,
        }));
    }
    if let Some(text) = message
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        blocks.push(Block::Text(text.to_string()));
    }
    if let Some(calls) = message
        .and_then(|m| m.get("tool_calls"))
        .and_then(Value::as_array)
    {
        blocks.extend(calls.iter().map(parse_tool_call));
    }

    Completion {
        id: response
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("chatcmpl_ironwire")
            .to_string(),
        blocks,
        stop: parse_stop_reason(
            choice
                .and_then(|c| c.get("finish_reason"))
                .and_then(Value::as_str),
        ),
        usage: parse_usage(response.get("usage")),
    }
}

/// Most per-token log-probabilities we will accumulate for one response.
///
/// The entries arrive from the upstream, one per generated token, and are
/// pushed onto a `Vec` — so this is the fourth place an upstream controls how
/// much we allocate, alongside `MAX_FRAME_BYTES`, `MAX_TOOL_CALLS` and
/// `MAX_TOOL_ARGUMENT_BYTES` in `crate::stream`. IronWire lets a user point at
/// an arbitrary OpenAI-compatible endpoint, which makes an unbounded stream of
/// `logprobs` frames reachable rather than theoretical. A hundred and thirty
/// thousand `f64`s is a megabyte, and far past any real response: the longest
/// output any of these providers will produce is tens of thousands of tokens.
///
/// Past the cap we stop pushing rather than dropping what we have. The mean is
/// then over a prefix, and `token_count` says how long that prefix was — an
/// aggregate over the first 131072 tokens is a true statement about them.
pub const MAX_TOKEN_LOGPROBS: usize = 1 << 17;

/// Append `ln p(chosen token)` for every token in one `choice`'s `logprobs`.
///
/// Values are carried as log-probabilities all the way to the reduction, which
/// exponentiates once in `f64`; see `ironwire_core::confidence` for why the
/// order matters. A malformed entry is skipped rather than repaired.
///
/// Shared by the streaming and non-streaming paths so the two cannot disagree
/// about what was captured.
pub fn accumulate_token_logprobs(choice: &Value, out: &mut Vec<f64>) {
    let Some(content) = choice
        .get("logprobs")
        .and_then(|l| l.get("content"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for entry in content {
        if out.len() >= MAX_TOKEN_LOGPROBS {
            return;
        }
        if let Some(logprob) = entry.get("logprob").and_then(Value::as_f64) {
            out.push(logprob);
        }
    }
}

/// Every log-probability in a non-streaming Chat Completions answer.
///
/// The streaming path accumulates frame by frame in `crate::stream`; a
/// `stream: false` request has the whole thing in one body, and would otherwise
/// pay for the inflated response and record nothing.
#[must_use]
pub fn completion_token_logprobs(response: &Value) -> Vec<f64> {
    let mut out = Vec::new();
    if let Some(choice) = response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
    {
        accumulate_token_logprobs(choice, &mut out);
    }
    out
}

/// Map a Chat Completions `finish_reason` onto the IR.
///
/// `tool_calls` → [`StopReason::ToolUse`] is the one that matters: get it wrong
/// and the agent stops instead of executing the call it was just handed.
#[must_use]
pub fn parse_stop_reason(finish: Option<&str>) -> StopReason {
    match finish {
        Some("stop") => StopReason::EndTurn,
        Some("length") => StopReason::MaxTokens,
        Some("tool_calls" | "function_call") => StopReason::ToolUse,
        Some("content_filter") => StopReason::Refusal,
        other => StopReason::Unrecognised(other.unwrap_or_default().to_string()),
    }
}

/// Read Chat Completions usage into the IR.
#[must_use]
pub fn parse_usage(usage: Option<&Value>) -> Usage {
    let n = |key: &str| {
        usage
            .and_then(|u| u.get(key))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let cached = usage
        .and_then(|u| u.pointer("/prompt_tokens_details/cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Usage {
        // Anthropic reports uncached and cached separately; OpenAI's
        // `prompt_tokens` is the total, so the cached part is subtracted rather
        // than double-counted.
        input: n("prompt_tokens").saturating_sub(cached),
        cached_input: cached,
        output: n("completion_tokens"),
        reasoning: usage
            .and_then(|u| u.pointer("/completion_tokens_details/reasoning_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
    }
}

/// Write a non-streaming Chat Completions answer.
#[must_use]
pub fn emit_completion(completion: &Completion, requested_model: &str) -> (Value, Dropped) {
    let mut dropped = Dropped::default();
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<Value> = Vec::new();

    for block in &completion.blocks {
        match block {
            Block::Text(text) => text_parts.push(text.clone()),
            Block::ToolUse { id, name, input } => tool_calls.push(json!({
                "id": tool_ids::encode(id, ME),
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".into()),
                },
            })),
            Block::Reasoning(_) => dropped.reasoning_blocks += 1,
            Block::Image(_) => dropped.images += 1,
            Block::ToolResult { .. } => {}
            Block::Unknown { origin, kind, raw } => {
                if *origin == ME {
                    text_parts.push(flatten_text(Some(raw)));
                } else {
                    dropped.note_unknown(kind);
                }
            }
        }
    }

    let mut message = Map::new();
    message.insert("role".into(), json!("assistant"));
    message.insert(
        "content".into(),
        if text_parts.is_empty() {
            Value::Null
        } else {
            json!(text_parts.join("\n"))
        },
    );
    if !tool_calls.is_empty() {
        message.insert("tool_calls".into(), Value::Array(tool_calls));
    }

    (
        json!({
            "id": completion.id,
            "object": "chat.completion",
            "model": requested_model,
            "choices": [{
                "index": 0,
                "message": Value::Object(message),
                "finish_reason": emit_stop_reason(&completion.stop),
            }],
            "usage": emit_usage(&completion.usage),
        }),
        dropped,
    )
}

/// Write an IR stop reason as a Chat Completions `finish_reason`.
#[must_use]
pub fn emit_stop_reason(stop: &StopReason) -> Value {
    match stop {
        StopReason::EndTurn => json!("stop"),
        // This wire has no separate value for a stop sequence; `stop` is what
        // it says when one matched.
        StopReason::StopSequence(_) => json!("stop"),
        StopReason::MaxTokens => json!("length"),
        StopReason::ToolUse => json!("tool_calls"),
        StopReason::Refusal => json!("content_filter"),
        StopReason::Unrecognised(_) => Value::Null,
    }
}

/// Write usage in Chat Completions shape.
#[must_use]
pub fn emit_usage(usage: &Usage) -> Value {
    json!({
        "prompt_tokens": usage.input + usage.cached_input,
        "completion_tokens": usage.output,
        "total_tokens": usage.input + usage.cached_input + usage.output,
        "prompt_tokens_details": {"cached_tokens": usage.cached_input},
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Reasoning;

    /// The exact conversation the pairwise translator was tested against, so
    /// the assertions below are the old suite's assertions.
    fn claude_code_ir() -> Conversation {
        crate::anthropic::parse_request(&json!({
            "model": "claude-opus-4-6",
            "max_tokens": 8192,
            "stream": true,
            "system": [
                {"type": "text", "text": "You are Claude Code.",
                 "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "Working directory: /repo."}
            ],
            "thinking": {"type": "enabled", "budget_tokens": 4096},
            "tools": [{
                "name": "Bash",
                "description": "run a command",
                "input_schema": {"type": "object", "properties": {"command": {"type": "string"}}}
            }],
            "messages": [
                {"role": "user", "content": "fix the failing test"},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "let me look", "signature": "sig-abc"},
                    {"type": "text", "text": "Running the tests."},
                    {"type": "tool_use", "id": "toolu_01ABC", "name": "Bash",
                     "input": {"command": "cargo test"}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_01ABC",
                     "content": "assertion failed", "is_error": true}
                ]},
                {"role": "assistant", "content": [{"type": "text", "text": "Off-by-one."}]},
                {"role": "user", "content": "go ahead and fix it"}
            ]
        }))
    }

    /// Capture must not disturb anything else about the translation — it is an
    /// addition to the request, not a different request.
    #[test]
    fn logprobs_do_not_perturb_the_rest_of_the_body() {
        let plain_ir = claude_code_ir();
        let mut captured_ir = claude_code_ir();
        captured_ir.params.logprobs = true;

        let (plain, _) = emit_request(&plain_ir, "near-x");
        let (captured, _) = emit_request(&captured_ir, "near-x");

        let (plain_obj, mut captured_obj) = (
            plain.as_object().expect("object").clone(),
            captured.as_object().expect("object").clone(),
        );
        assert_eq!(captured_obj.remove("logprobs"), Some(json!(true)));
        assert_eq!(
            plain_obj, captured_obj,
            "enabling capture changed something other than the logprob key"
        );
    }

    /// Parsing is lossless by rule, and this wire is the one that can say it.
    /// Reading it back is also what keeps a client's own request honoured
    /// independently of the capture setting.
    #[test]
    fn a_clients_own_logprobs_request_survives_the_round_trip() {
        let ir = parse_request(&json!({
            "model": "qwen3",
            "logprobs": true,
            "messages": [{"role": "user", "content": "hi"}]
        }));
        assert!(ir.params.logprobs);
        let (out, _) = emit_request(&ir, "near-x");
        assert_eq!(out["logprobs"], json!(true));
    }

    #[test]
    fn a_claude_code_turn_becomes_a_valid_chat_completions_request() {
        let (out, _) = emit_request(&claude_code_ir(), "near-x");
        assert_eq!(out["model"], "near-x");
        assert_eq!(out["stream"], true);
        assert_eq!(out["max_tokens"], 8192);
        assert_eq!(out["stream_options"]["include_usage"], true);

        let messages = out["messages"].as_array().expect("messages");
        // system, user, assistant(+call), tool, assistant, user
        assert_eq!(messages[0]["role"], "system");
        assert!(
            messages[0]["content"]
                .as_str()
                .expect("system text")
                .contains("You are Claude Code.")
        );
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[5]["content"], "go ahead and fix it");
    }

    #[test]
    fn both_system_blocks_survive_the_flattening() {
        let (out, _) = emit_request(&claude_code_ir(), "near-x");
        let system = out["messages"][0]["content"].as_str().expect("system");
        assert!(system.contains("Claude Code"));
        assert!(system.contains("/repo"), "second block lost: {system}");
    }

    #[test]
    fn tool_calls_carry_arguments_as_a_json_string() {
        let (out, _) = emit_request(&claude_code_ir(), "near-x");
        let call = &out["messages"][2]["tool_calls"][0];
        assert_eq!(call["type"], "function");
        assert_eq!(call["function"]["name"], "Bash");
        let args = call["function"]["arguments"].as_str().expect("a string");
        let parsed: Value = serde_json::from_str(args).expect("valid JSON in the string");
        assert_eq!(parsed["command"], "cargo test");
    }

    #[test]
    fn a_tool_result_becomes_its_own_tool_message_keyed_by_call_id() {
        let (out, _) = emit_request(&claude_code_ir(), "near-x");
        let result = &out["messages"][3];
        assert_eq!(result["role"], "tool");
        assert_eq!(result["tool_call_id"], "toolu_01ABC");
        assert_eq!(result["content"], "assertion failed");
    }

    #[test]
    fn thinking_and_cache_breakpoints_are_dropped_and_counted() {
        let (out, dropped) = emit_request(&claude_code_ir(), "near-x");
        assert_eq!(dropped.reasoning_blocks, 1);
        assert_eq!(dropped.cache_breakpoints, 1);
        assert_eq!(dropped.images, 0);
        assert!(!dropped.is_empty());
        assert!(
            !out.to_string().contains("sig-abc"),
            "a signature leaked into the foreign request"
        );
    }

    #[test]
    fn tools_translate_to_openai_function_shape() {
        let (out, _) = emit_request(&claude_code_ir(), "near-x");
        let tool = &out["tools"][0];
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], "Bash");
        assert_eq!(tool["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn tool_choice_maps_across() {
        for (anthropic, expected) in [
            (json!({"type": "auto"}), json!("auto")),
            (json!({"type": "any"}), json!("required")),
            (json!({"type": "none"}), json!("none")),
            (
                json!({"type": "tool", "name": "Bash"}),
                json!({"type": "function", "function": {"name": "Bash"}}),
            ),
        ] {
            let ir = crate::anthropic::parse_request(&json!({
                "tools": [{"name": "Bash", "input_schema": {}}],
                "tool_choice": anthropic,
                "messages": [],
            }));
            let (out, _) = emit_request(&ir, "m");
            assert_eq!(out["tool_choice"], expected);
        }
    }

    #[test]
    fn an_assistant_turn_with_only_a_tool_call_still_carries_the_content_key() {
        let ir = crate::anthropic::parse_request(&json!({"messages": [
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {}}
            ]}
        ]}));
        let (out, _) = emit_request(&ir, "m");
        assert!(out["messages"][0].get("content").is_some());
        assert!(out["messages"][0]["content"].is_null());
    }

    #[test]
    fn an_empty_conversation_produces_a_well_formed_request() {
        let (out, dropped) = emit_request(&Conversation::default(), "m");
        assert_eq!(out["model"], "m");
        assert!(out["messages"].as_array().expect("array").is_empty());
        assert!(dropped.is_empty());
        assert!(out.get("tools").is_none());
    }

    #[test]
    fn a_chat_request_survives_a_round_trip_through_the_ir() {
        let body = json!({
            "model": "qwen3",
            "stream": false,
            "max_tokens": 512,
            "temperature": 0.2,
            "stop": ["END"],
            "messages": [
                {"role": "system", "content": "be brief"},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "call_1", "type": "function",
                     "function": {"name": "ls", "arguments": "{\"path\":\"/\"}"}}
                ]},
                {"role": "tool", "tool_call_id": "call_1", "content": "a b c"},
            ],
            "tools": [{"type": "function", "function": {
                "name": "ls", "description": "list", "parameters": {"type": "object"}}}],
        });
        let (out, dropped) = emit_request(&parse_request(&body), "qwen3");
        assert!(dropped.is_empty(), "a same-wire emit dropped {dropped:?}");
        assert_eq!(out["messages"][0]["role"], "system");
        assert_eq!(out["messages"][1]["content"], "hi");
        assert_eq!(out["messages"][2]["tool_calls"][0]["id"], "call_1");
        assert_eq!(out["messages"][3]["role"], "tool");
        assert_eq!(out["messages"][3]["content"], "a b c");
        assert_eq!(out["tools"][0]["function"]["name"], "ls");
        assert_eq!(out["max_tokens"], 512);
        assert_eq!(out["stop"], json!(["END"]));
    }

    #[test]
    fn a_text_answer_becomes_an_ir_completion() {
        let response = json!({
            "id": "chatcmpl-1",
            "choices": [{"index": 0, "finish_reason": "stop",
                         "message": {"role": "assistant", "content": "done"}}],
            "usage": {"prompt_tokens": 120, "completion_tokens": 8},
        });
        let ir = parse_completion(&response);
        assert_eq!(ir.stop, StopReason::EndTurn);
        assert_eq!(ir.usage.input, 120);
        assert_eq!(ir.usage.output, 8);
        assert!(matches!(&ir.blocks[0], Block::Text(t) if t == "done"));
    }

    /// Cached prompt tokens are part of `prompt_tokens` here and separate in
    /// Anthropic's shape. Counting them twice inflates every cross-wire turn in
    /// the ledger.
    #[test]
    fn cached_prompt_tokens_are_not_counted_twice() {
        let usage = parse_usage(Some(&json!({
            "prompt_tokens": 1000,
            "completion_tokens": 10,
            "prompt_tokens_details": {"cached_tokens": 900},
        })));
        assert_eq!(usage.input, 100);
        assert_eq!(usage.cached_input, 900);
        let back = emit_usage(&usage);
        assert_eq!(back["prompt_tokens"], 1000);
    }

    #[test]
    fn a_malformed_argument_string_does_not_take_the_turn_down() {
        let block = parse_tool_call(&json!({
            "id": "call_1",
            "function": {"name": "ls", "arguments": "{not json"},
        }));
        assert!(matches!(&block, Block::ToolUse { input, .. } if input == &json!({})));
    }

    /// Neither half of a reasoning block crosses: not the blob, which only its
    /// own provider can validate, and not the summary either. Folding a summary
    /// into the answer would put the model's private reasoning into the
    /// transcript as though it had said it.
    #[test]
    fn reasoning_does_not_cross_in_either_form() {
        let completion = Completion {
            id: "x".to_string(),
            blocks: vec![
                Block::Reasoning(Reasoning {
                    origin: Protocol::OpenAiResponses,
                    summary: Some("considered the options".to_string()),
                    opaque: json!({"encrypted_content": "gAAAAA"}),
                }),
                Block::Text("here is the answer".to_string()),
            ],
            stop: StopReason::EndTurn,
            usage: Usage::default(),
        };
        let (out, dropped) = emit_completion(&completion, "m");
        assert_eq!(dropped.reasoning_blocks, 1);
        let text = out.to_string();
        assert!(!text.contains("gAAAAA"));
        assert!(!text.contains("considered the options"), "{text}");
        assert_eq!(
            out["choices"][0]["message"]["content"],
            "here is the answer"
        );
    }

    #[test]
    fn an_image_survives_a_round_trip_on_this_wire() {
        let body = json!({"messages": [{"role": "user", "content": [
            {"type": "text", "text": "what is this"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}},
        ]}]});
        let ir = parse_request(&body);
        assert!(matches!(
            &ir.turns[0].blocks[1],
            Block::Image(ImageSource::Base64 { media_type, data })
                if media_type == "image/png" && data == "AAAA"
        ));
        let (out, dropped) = emit_request(&ir, "m");
        assert!(dropped.is_empty());
        assert_eq!(
            out["messages"][0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,AAAA"
        );
    }
}
