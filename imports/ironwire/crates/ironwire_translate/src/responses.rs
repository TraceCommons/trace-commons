//! OpenAI Responses ⇄ the IR.
//!
//! The wire Codex speaks, and the one this build could not translate into at
//! all — which is why a Claude Code session could never fall back onto a
//! ChatGPT subscription, the single most valuable route the product has.
//!
//! Shape notes worth having in one place, because two of them differ from Chat
//! Completions in ways that are easy to get subtly wrong:
//!
//! - The system prompt is `instructions`, a **string**, not a message.
//! - `input` is a flat list of *items*, not messages: a `message`, a
//!   `reasoning` item, a `function_call`, and a `function_call_output` are
//!   siblings. Tool results are therefore not attached to a turn at all.
//! - Tool definitions are flat (`{"type": "function", "name": …}`), where Chat
//!   Completions nests them under a `function` key.
//! - Content parts are directional: `input_text` on the way in, `output_text`
//!   on the way back.

use serde_json::{Map, Value, json};

use ironwire_core::protocol::Protocol;

use crate::ir::{
    Block, Completion, Conversation, Dropped, ImageSource, Params, Reasoning, ReasoningRequest,
    Role, StopReason, SystemChunk, ToolChoice, ToolDef, Turn, Usage, flatten_text,
};
use crate::tool_ids;

const ME: Protocol = Protocol::OpenAiResponses;

// ---------------------------------------------------------------------------
// Request: wire → IR
// ---------------------------------------------------------------------------

/// Parse a Responses request.
#[must_use]
pub fn parse_request(body: &Value) -> Conversation {
    let mut system = Vec::new();
    if let Some(instructions) = body
        .get("instructions")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        system.push(SystemChunk {
            text: instructions.to_string(),
            cache_breakpoint: false,
        });
    }

    let mut turns: Vec<Turn> = Vec::new();
    match body.get("input") {
        // A bare string is the shorthand for one user message.
        Some(Value::String(text)) => turns.push(Turn {
            role: Role::User,
            blocks: vec![Block::Text(text.clone())],
        }),
        Some(Value::Array(items)) => {
            for item in items {
                parse_item(item, &mut turns);
            }
        }
        _ => {}
    }

    let mut tools = Vec::new();
    let mut unexpressible_tools = Vec::new();
    if let Some(declared) = body.get("tools").and_then(Value::as_array) {
        for tool in declared {
            sort_tool(tool, &mut tools, &mut unexpressible_tools);
        }
    }

    Conversation {
        system,
        turns,
        tools,
        unexpressible_tools,
        tool_choice: body.get("tool_choice").and_then(parse_tool_choice),
        params: Params {
            max_tokens: body.get("max_output_tokens").and_then(Value::as_u64),
            temperature: body.get("temperature").and_then(Value::as_f64),
            top_p: body.get("top_p").and_then(Value::as_f64),
            stop: Vec::new(),
            reasoning: body.get("reasoning").map(|reasoning| ReasoningRequest {
                effort: reasoning
                    .get("effort")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                budget_tokens: None,
                summary: reasoning.get("summary").is_some(),
            }),
            stream: body
                .get("stream")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
            // Responses spells this as `top_logprobs` plus an `include`
            // entry rather than a boolean, and nothing here reads it yet.
            logprobs: false,
        },
    }
}

/// One `input` item becomes a block, appended to the turn it belongs with.
fn parse_item(item: &Value, turns: &mut Vec<Turn>) {
    match item.get("type").and_then(Value::as_str) {
        Some("message") | None => {
            let role = match item.get("role").and_then(Value::as_str) {
                Some("assistant") => Role::Assistant,
                Some("system" | "developer") => Role::User,
                _ => Role::User,
            };
            let blocks = match item.get("content") {
                Some(Value::Array(parts)) => parts.iter().map(parse_content_part).collect(),
                other => vec![Block::Text(flatten_text(other))],
            };
            turns.push(Turn { role, blocks });
        }
        Some("reasoning") => push_block(
            turns,
            Role::Assistant,
            Block::Reasoning(Reasoning {
                origin: ME,
                summary: item
                    .get("summary")
                    .map(|summary| flatten_text(Some(summary)))
                    .filter(|text| !text.is_empty()),
                // Kept whole so replaying it to OpenAI reproduces the original
                // item exactly, id and all.
                opaque: json!({"raw": item.clone()}),
            }),
        ),
        Some("function_call") => push_block(
            turns,
            Role::Assistant,
            Block::ToolUse {
                id: tool_ids::decode(call_id(item), ME),
                name: item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                input: item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .and_then(|args| serde_json::from_str(args).ok())
                    .unwrap_or_else(|| json!({})),
            },
        ),
        Some("function_call_output") => push_block(
            turns,
            Role::User,
            Block::ToolResult {
                id: tool_ids::decode(call_id(item), ME),
                content: flatten_text(item.get("output")),
                is_error: false,
            },
        ),
        Some(other) => push_block(
            turns,
            Role::Assistant,
            Block::Unknown {
                origin: ME,
                kind: other.to_string(),
                raw: item.clone(),
            },
        ),
    }
}

/// `call_id` is the field that pairs a call with its output; `id` identifies the
/// item itself. Using the wrong one silently unpairs every tool result.
fn call_id(item: &Value) -> &str {
    item.get("call_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
}

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
        Some("input_text" | "output_text" | "text") => Block::Text(
            part.get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ),
        Some("input_image") => {
            let url = part
                .get("image_url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Block::Image(match url.strip_prefix("data:") {
                Some(rest) => match rest.split_once(";base64,") {
                    Some((media_type, data)) => ImageSource::Base64 {
                        media_type: media_type.to_string(),
                        data: data.to_string(),
                    },
                    None => ImageSource::Url(url.to_string()),
                },
                None => ImageSource::Url(url.to_string()),
            })
        }
        other => Block::Unknown {
            origin: ME,
            kind: other.unwrap_or("<no type field>").to_string(),
            raw: part.clone(),
        },
    }
}

/// Put one entry from a Responses `tools` array where it belongs.
///
/// Only one of the three shapes Codex sends is a function:
///
/// - A **function** — `{"type": "function", "name": …}` — is the ordinary
///   case and the only one the IR models.
/// - A **built-in** — `{"type": "web_search", …}` and its siblings — has no
///   `name` at all. This used to produce a function named `""`, which no
///   OpenAI-compatible server accepts: a single built-in anywhere in a
///   client's tool list made the entire translated request a 400, and the
///   refusal named a tool index rather than anything the user had written.
/// - A **namespace** — `{"type": "namespace", "name": …, "tools": [ … ]}` —
///   is a group, one per connected MCP server. Its real functions are nested
///   inside it, and flattening them would mean choosing a name for each; the
///   model would then call by that name and the client would not recognise
///   what came back. So the group is reported as lost rather than guessed at.
///
/// Both non-function shapes are named in `unexpressible` so the emitter can
/// say what the route cost. Nothing is silently invented.
fn sort_tool(tool: &Value, functions: &mut Vec<ToolDef>, unexpressible: &mut Vec<String>) {
    let kind = tool.get("type").and_then(Value::as_str);
    let has_name = tool
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(|name| !name.is_empty());

    // A missing `type` with a name is the older function shape.
    if matches!(kind, Some("function") | None) && has_name {
        functions.push(parse_tool(tool));
        return;
    }

    let kind = kind.unwrap_or("unnamed").to_string();
    if !unexpressible.contains(&kind) {
        unexpressible.push(kind);
    }
}

fn parse_tool(tool: &Value) -> ToolDef {
    ToolDef {
        name: tool
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        description: tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        schema: tool
            .get("parameters")
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
            .get("name")
            .and_then(Value::as_str)
            .map(|name| ToolChoice::Named(name.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Request: IR → wire
// ---------------------------------------------------------------------------

/// Write a Responses request.
#[must_use]
pub fn emit_request(conversation: &Conversation, model: &str) -> (Value, Dropped) {
    let mut dropped = Dropped {
        // Not modelled by the IR, so no target can emit them. Named rather
        // than dropped in silence (`docs/TRANSLATION.md`).
        tools: conversation.unexpressible_tools.clone(),
        ..Dropped::default()
    };
    let mut input: Vec<Value> = Vec::new();

    for turn in &conversation.turns {
        emit_turn(turn, &mut input, &mut dropped);
    }

    let mut request = Map::new();
    request.insert("model".into(), json!(model));
    request.insert("input".into(), Value::Array(input));
    request.insert("stream".into(), json!(conversation.params.stream));

    let instructions: Vec<&str> = conversation
        .system
        .iter()
        .inspect(|chunk| {
            if chunk.cache_breakpoint {
                dropped.cache_breakpoints += 1;
            }
        })
        .map(|chunk| chunk.text.as_str())
        .collect();
    if !instructions.is_empty() {
        request.insert("instructions".into(), json!(instructions.join("\n\n")));
    }

    if let Some(max_tokens) = conversation.params.max_tokens {
        request.insert("max_output_tokens".into(), json!(max_tokens));
    }
    if let Some(temperature) = conversation.params.temperature {
        request.insert("temperature".into(), json!(temperature));
    }
    if let Some(top_p) = conversation.params.top_p {
        request.insert("top_p".into(), json!(top_p));
    }
    if let Some(reasoning) = &conversation.params.reasoning {
        let mut object = Map::new();
        if let Some(effort) = &reasoning.effort {
            object.insert("effort".into(), json!(effort));
        }
        if reasoning.summary {
            object.insert("summary".into(), json!("auto"));
        }
        if !object.is_empty() {
            request.insert("reasoning".into(), Value::Object(object));
        }
    }

    if !conversation.tools.is_empty() {
        request.insert(
            "tools".into(),
            Value::Array(
                conversation
                    .tools
                    .iter()
                    .map(|tool| {
                        // Flat, not nested under `function` — the difference
                        // from Chat Completions that is easiest to get wrong.
                        json!({
                            "type": "function",
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.schema,
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
                    ToolChoice::Named(name) => json!({"type": "function", "name": name}),
                },
            );
        }
    }

    (Value::Object(request), dropped)
}

fn emit_turn(turn: &Turn, out: &mut Vec<Value>, dropped: &mut Dropped) {
    let mut parts: Vec<Value> = Vec::new();
    let text_kind = match turn.role {
        Role::Assistant => "output_text",
        Role::User => "input_text",
    };

    // Items are siblings on this wire, so a turn's blocks can interleave
    // messages, calls and results. Message parts are collected and flushed
    // whenever a non-message item has to go between them, which preserves the
    // order the client sent.
    let flush = |parts: &mut Vec<Value>, out: &mut Vec<Value>| {
        if !parts.is_empty() {
            out.push(json!({
                "type": "message",
                "role": match turn.role { Role::Assistant => "assistant", Role::User => "user" },
                "content": std::mem::take(parts),
            }));
        }
    };

    for block in &turn.blocks {
        match block {
            Block::Text(text) if text.is_empty() => {}
            Block::Text(text) => parts.push(json!({"type": text_kind, "text": text})),
            Block::Image(source) => parts.push(json!({
                "type": "input_image",
                "image_url": match source {
                    ImageSource::Base64 { media_type, data } => format!("data:{media_type};base64,{data}"),
                    ImageSource::Url(url) => url.clone(),
                },
            })),
            Block::ToolUse { id, name, input } => {
                flush(&mut parts, out);
                out.push(json!({
                    "type": "function_call",
                    "call_id": tool_ids::encode(id, ME),
                    "name": name,
                    "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".into()),
                }));
            }
            Block::ToolResult { id, content, .. } => {
                flush(&mut parts, out);
                out.push(json!({
                    "type": "function_call_output",
                    "call_id": tool_ids::encode(id, ME),
                    "output": content,
                }));
            }
            Block::Reasoning(reasoning) => {
                // Replayed verbatim only to the API that minted it: an
                // `encrypted_content` from anywhere else is not decryptable
                // here, and a reasoning item OpenAI did not issue is rejected.
                if reasoning.origin == ME
                    && let Some(raw) = reasoning.opaque.get("raw")
                {
                    flush(&mut parts, out);
                    out.push(raw.clone());
                } else {
                    // Not replayable, and the summary does not travel either:
                    // turning it into visible content would put the model's
                    // private reasoning into the transcript as prose it never
                    // said.
                    dropped.reasoning_blocks += 1;
                }
            }
            Block::Unknown { origin, kind, raw } => {
                if *origin == ME {
                    flush(&mut parts, out);
                    out.push(raw.clone());
                } else {
                    dropped.note_unknown(kind);
                }
            }
        }
    }

    flush(&mut parts, out);
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/// Parse a non-streaming Responses answer.
#[must_use]
pub fn parse_completion(response: &Value) -> Completion {
    let mut turns: Vec<Turn> = Vec::new();
    for item in response
        .get("output")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
    {
        parse_item(item, &mut turns);
    }
    let blocks: Vec<Block> = turns.into_iter().flat_map(|turn| turn.blocks).collect();

    Completion {
        stop: parse_stop_reason(
            response.get("status").and_then(Value::as_str),
            response
                .pointer("/incomplete_details/reason")
                .and_then(Value::as_str),
            blocks
                .iter()
                .any(|block| matches!(block, Block::ToolUse { .. })),
        ),
        id: response
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("resp_ironwire")
            .to_string(),
        blocks,
        usage: parse_usage(response.get("usage")),
    }
}

/// Map a Responses `status` onto the IR.
///
/// This wire has no `finish_reason`: a turn that ended by asking for a tool is
/// reported as `completed` like any other, and the only way to tell is that the
/// output contains a `function_call`. Missing that is how an agent stops
/// instead of running the call it was just handed.
#[must_use]
pub fn parse_stop_reason(
    status: Option<&str>,
    incomplete_reason: Option<&str>,
    has_tool_call: bool,
) -> StopReason {
    if has_tool_call {
        return StopReason::ToolUse;
    }
    match status {
        Some("completed") => StopReason::EndTurn,
        Some("incomplete") => match incomplete_reason {
            Some("max_output_tokens") => StopReason::MaxTokens,
            Some("content_filter") => StopReason::Refusal,
            other => StopReason::Unrecognised(other.unwrap_or("incomplete").to_string()),
        },
        other => StopReason::Unrecognised(other.unwrap_or_default().to_string()),
    }
}

/// Read Responses usage into the IR.
#[must_use]
pub fn parse_usage(usage: Option<&Value>) -> Usage {
    let n = |key: &str| {
        usage
            .and_then(|u| u.get(key))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let cached = usage
        .and_then(|u| u.pointer("/input_tokens_details/cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Usage {
        input: n("input_tokens").saturating_sub(cached),
        cached_input: cached,
        output: n("output_tokens"),
        reasoning: usage
            .and_then(|u| u.pointer("/output_tokens_details/reasoning_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
    }
}

/// Write a non-streaming Responses answer.
#[must_use]
pub fn emit_completion(completion: &Completion, requested_model: &str) -> (Value, Dropped) {
    let mut dropped = Dropped::default();
    let mut output: Vec<Value> = Vec::new();
    emit_turn(
        &Turn {
            role: Role::Assistant,
            blocks: completion.blocks.clone(),
        },
        &mut output,
        &mut dropped,
    );

    let (status, incomplete) = emit_stop_reason(&completion.stop);
    let mut response = Map::new();
    response.insert("id".into(), json!(completion.id));
    response.insert("object".into(), json!("response"));
    response.insert("model".into(), json!(requested_model));
    response.insert("status".into(), status);
    if let Some(incomplete) = incomplete {
        response.insert("incomplete_details".into(), incomplete);
    }
    response.insert("output".into(), Value::Array(output));
    response.insert("usage".into(), emit_usage(&completion.usage));
    (Value::Object(response), dropped)
}

/// Split an IR stop reason into `status` and `incomplete_details`.
#[must_use]
pub fn emit_stop_reason(stop: &StopReason) -> (Value, Option<Value>) {
    match stop {
        // A turn ending in a tool call *is* a completed response here; the call
        // itself is what says more is coming.
        StopReason::EndTurn | StopReason::ToolUse | StopReason::StopSequence(_) => {
            (json!("completed"), None)
        }
        StopReason::MaxTokens => (
            json!("incomplete"),
            Some(json!({"reason": "max_output_tokens"})),
        ),
        StopReason::Refusal => (
            json!("incomplete"),
            Some(json!({"reason": "content_filter"})),
        ),
        StopReason::Unrecognised(_) => (json!("in_progress"), None),
    }
}

/// Write usage in Responses shape.
#[must_use]
pub fn emit_usage(usage: &Usage) -> Value {
    json!({
        "input_tokens": usage.input + usage.cached_input,
        "input_tokens_details": {"cached_tokens": usage.cached_input},
        "output_tokens": usage.output,
        "output_tokens_details": {"reasoning_tokens": usage.reasoning},
        "total_tokens": usage.input + usage.cached_input + usage.output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Codex turn: instructions, a tool, encrypted reasoning carried forward,
    /// a call and its result.
    fn codex_body() -> Value {
        json!({
            "model": "gpt-5.6",
            "stream": true,
            "instructions": "You are Codex, based on GPT-5.",
            "max_output_tokens": 4096,
            "reasoning": {"effort": "high", "summary": "auto"},
            "tools": [{"type": "function", "name": "shell", "description": "run it",
                       "parameters": {"type": "object"}}],
            "tool_choice": "auto",
            "input": [
                {"type": "message", "role": "user",
                 "content": [{"type": "input_text", "text": "fix the test"}]},
                {"type": "reasoning", "id": "rs_1", "encrypted_content": "gAAAAA",
                 "summary": [{"type": "summary_text", "text": "look at the failure"}]},
                {"type": "function_call", "call_id": "call_1", "name": "shell",
                 "arguments": "{\"cmd\":\"cargo test\"}"},
                {"type": "function_call_output", "call_id": "call_1", "output": "assertion failed"},
                {"type": "message", "role": "user",
                 "content": [{"type": "input_text", "text": "go ahead"}]}
            ]
        })
    }

    #[test]
    fn a_codex_turn_parses_into_the_ir() {
        let ir = parse_request(&codex_body());
        assert_eq!(ir.system[0].text, "You are Codex, based on GPT-5.");
        assert_eq!(ir.params.max_tokens, Some(4096));
        assert!(ir.params.stream);
        assert_eq!(
            ir.params.reasoning.as_ref().and_then(|r| r.effort.clone()),
            Some("high".to_string())
        );
        assert_eq!(ir.tools[0].name, "shell");
        assert_eq!(ir.tool_choice, Some(ToolChoice::Auto));

        let blocks: Vec<&Block> = ir.turns.iter().flat_map(|t| &t.blocks).collect();
        assert!(matches!(blocks[0], Block::Text(t) if t == "fix the test"));
        assert!(matches!(blocks[1], Block::Reasoning(_)));
        assert!(matches!(blocks[2], Block::ToolUse { name, .. } if name == "shell"));
        assert!(
            matches!(blocks[3], Block::ToolResult { content, .. } if content == "assertion failed")
        );
    }

    #[test]
    fn a_responses_request_survives_a_round_trip_through_the_ir() {
        let ir = parse_request(&codex_body());
        let (out, dropped) = emit_request(&ir, "gpt-5.6");
        assert!(dropped.is_empty(), "a same-wire emit dropped {dropped:?}");

        assert_eq!(out["instructions"], "You are Codex, based on GPT-5.");
        assert_eq!(out["max_output_tokens"], 4096);
        assert_eq!(out["reasoning"]["effort"], "high");
        // Flat, not nested under `function`.
        assert_eq!(out["tools"][0]["name"], "shell");
        assert_eq!(out["tools"][0]["type"], "function");

        let input = out["input"].as_array().expect("input");
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        // The encrypted item came back byte-identical, which is the one thing
        // OpenAI will reject us for getting wrong.
        assert_eq!(input[1]["type"], "reasoning");
        assert_eq!(input[1]["encrypted_content"], "gAAAAA");
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_1");
    }

    /// `call_id` pairs a call with its output; `id` names the item. Reading the
    /// wrong one unpairs every tool result, silently.
    #[test]
    fn a_call_and_its_output_stay_paired() {
        let ir = parse_request(&json!({"input": [
            {"type": "function_call", "id": "fc_9", "call_id": "call_1", "name": "shell", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "call_1", "output": "ok"},
        ]}));
        let blocks: Vec<&Block> = ir.turns.iter().flat_map(|t| &t.blocks).collect();
        let (Block::ToolUse { id: call, .. }, Block::ToolResult { id: result, .. }) =
            (blocks[0], blocks[1])
        else {
            panic!("expected a call and its output");
        };
        assert_eq!(call, result);
        assert_eq!(call.as_str(), "call_1");
    }

    /// The route this whole exercise exists for: Claude Code onto a ChatGPT
    /// subscription.
    #[test]
    fn an_anthropic_conversation_becomes_a_valid_responses_request() {
        let ir = crate::anthropic::parse_request(&json!({
            "model": "claude-opus-4-6",
            "max_tokens": 8192,
            "stream": true,
            "system": [{"type": "text", "text": "You are Claude Code.",
                        "cache_control": {"type": "ephemeral"}}],
            "tools": [{"name": "Bash", "description": "run", "input_schema": {"type": "object"}}],
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "hmm", "signature": "sig-abc"},
                    {"type": "tool_use", "id": "toolu_01ABC", "name": "Bash", "input": {"c": "ls"}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_01ABC", "content": "a b c"}
                ]}
            ]
        }));
        let (out, dropped) = emit_request(&ir, "gpt-5.6");

        // The Anthropic signature is not replayable here and must not travel.
        assert_eq!(dropped.reasoning_blocks, 1);
        assert_eq!(dropped.cache_breakpoints, 1);
        assert!(dropped.unknown_blocks.is_empty());
        assert!(!out.to_string().contains("sig-abc"));

        assert_eq!(out["instructions"], "You are Claude Code.");
        assert_eq!(out["max_output_tokens"], 8192);
        assert_eq!(out["tools"][0]["name"], "Bash");

        let input = out["input"].as_array().expect("input");
        assert_eq!(input[0]["content"][0]["text"], "hi");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["name"], "Bash");
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["output"], "a b c");
        // Same id on both, so the provider can pair them.
        assert_eq!(input[1]["call_id"], input[2]["call_id"]);
    }

    #[test]
    fn an_answer_round_trips() {
        let response = json!({
            "id": "resp_1",
            "object": "response",
            "status": "completed",
            "output": [
                {"type": "reasoning", "id": "rs_1", "encrypted_content": "gAAAAA"},
                {"type": "message", "role": "assistant",
                 "content": [{"type": "output_text", "text": "done"}]},
                {"type": "function_call", "call_id": "call_1", "name": "shell", "arguments": "{}"}
            ],
            "usage": {"input_tokens": 900, "input_tokens_details": {"cached_tokens": 800},
                      "output_tokens": 50, "output_tokens_details": {"reasoning_tokens": 20}},
        });
        let ir = parse_completion(&response);
        assert_eq!(ir.usage.input, 100);
        assert_eq!(ir.usage.cached_input, 800);
        assert_eq!(ir.usage.reasoning, 20);
        // A pending call, even though the status says `completed`.
        assert_eq!(ir.stop, StopReason::ToolUse);

        let (out, dropped) = emit_completion(&ir, "gpt-5.6");
        assert!(dropped.is_empty());
        assert_eq!(out["status"], "completed");
        assert_eq!(out["output"][0]["encrypted_content"], "gAAAAA");
        assert_eq!(out["usage"]["input_tokens"], 900);
    }

    /// The status field cannot say it, so the call in the output has to.
    #[test]
    fn a_turn_that_ends_in_a_tool_call_is_not_reported_as_finished() {
        assert_eq!(
            parse_stop_reason(Some("completed"), None, true),
            StopReason::ToolUse
        );
        assert_eq!(
            parse_stop_reason(Some("completed"), None, false),
            StopReason::EndTurn
        );
        assert_eq!(
            parse_stop_reason(Some("incomplete"), Some("max_output_tokens"), false),
            StopReason::MaxTokens
        );
    }

    #[test]
    fn a_bare_string_input_is_the_shorthand_it_looks_like() {
        let ir = parse_request(&json!({"input": "hello"}));
        assert_eq!(ir.turns.len(), 1);
        assert!(matches!(&ir.turns[0].blocks[0], Block::Text(t) if t == "hello"));
    }
}
