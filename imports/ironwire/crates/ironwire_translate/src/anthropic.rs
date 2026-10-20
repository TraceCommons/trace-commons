//! Anthropic Messages ⇄ the IR.
//!
//! Everything this build knows about the shape of `POST /v1/messages` lives
//! here: how a request parses, how one is written, and how an answer is written
//! back. Keeping it in one file is the point of the pivot — the alternative was
//! this knowledge smeared across every pair that happens to involve Anthropic.

use serde_json::{Map, Value, json};

use ironwire_core::protocol::Protocol;

use crate::ir::{
    Block, Completion, Conversation, Dropped, ImageSource, Params, Reasoning, ReasoningRequest,
    Role, StopReason, SystemChunk, ToolChoice, ToolDef, Turn, Usage, flatten_text,
};
use crate::tool_ids;

const ME: Protocol = Protocol::AnthropicMessages;

// ---------------------------------------------------------------------------
// Request: wire → IR
// ---------------------------------------------------------------------------

/// Parse an Anthropic Messages request.
///
/// Total and lossless: anything not modelled becomes [`Block::Unknown`] holding
/// the original JSON, and no judgement is made about whether it can travel.
#[must_use]
pub fn parse_request(body: &Value) -> Conversation {
    Conversation {
        system: parse_system(body.get("system")),
        turns: body
            .get("messages")
            .and_then(Value::as_array)
            .map(|turns| turns.iter().map(parse_turn).collect())
            .unwrap_or_default(),
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
            max_tokens: body.get("max_tokens").and_then(Value::as_u64),
            temperature: body.get("temperature").and_then(Value::as_f64),
            top_p: body.get("top_p").and_then(Value::as_f64),
            stop: body
                .get("stop_sequences")
                .and_then(Value::as_array)
                .map(|s| {
                    s.iter()
                        .filter_map(Value::as_str)
                        .map(ToString::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            reasoning: body.get("thinking").map(|thinking| ReasoningRequest {
                effort: None,
                budget_tokens: thinking.get("budget_tokens").and_then(Value::as_u64),
                summary: thinking.get("type").and_then(Value::as_str) == Some("enabled"),
            }),
            stream: body
                .get("stream")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
            // Anthropic Messages has no log-probability parameter, so a
            // request that arrived here never asked for one.
            logprobs: false,
        },
    }
}

fn parse_system(system: Option<&Value>) -> Vec<SystemChunk> {
    match system {
        Some(Value::String(text)) => vec![SystemChunk {
            text: text.clone(),
            cache_breakpoint: false,
        }],
        Some(Value::Array(blocks)) => blocks
            .iter()
            .map(|block| SystemChunk {
                text: block
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                cache_breakpoint: block.get("cache_control").is_some(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_turn(turn: &Value) -> Turn {
    let role = match turn.get("role").and_then(Value::as_str) {
        Some("assistant") => Role::Assistant,
        _ => Role::User,
    };
    let content = turn.get("content");

    if let Some(text) = content.and_then(Value::as_str) {
        return Turn {
            role,
            blocks: vec![Block::Text(text.to_string())],
        };
    }

    let blocks = content
        .and_then(Value::as_array)
        .map(|blocks| blocks.iter().map(parse_block).collect())
        .unwrap_or_default();
    Turn { role, blocks }
}

/// A cache breakpoint on a *content* block.
///
/// Recorded as a synthetic chunk on the system prompt would be wrong, so it is
/// simply counted at emit time. Anthropic is the only wire with these, so a
/// same-protocol round trip loses the marker on message blocks — noted here
/// rather than silently, and it is not replayed state: a breakpoint is a
/// caching hint, and losing it costs a cache miss rather than correctness.
fn parse_block(block: &Value) -> Block {
    match block.get("type").and_then(Value::as_str) {
        Some("text") => Block::Text(
            block
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ),
        Some(kind @ ("thinking" | "redacted_thinking")) => Block::Reasoning(Reasoning {
            origin: ME,
            summary: block
                .get("thinking")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            // The signature is the part only Anthropic can validate. Kept whole
            // — including which kind of block it came from — so replaying it to
            // Anthropic reproduces the original exactly.
            opaque: json!({"type": kind, "raw": block.clone()}),
        }),
        Some("image") => Block::Image(parse_image(block)),
        Some("tool_use") => Block::ToolUse {
            id: tool_ids::decode(
                block.get("id").and_then(Value::as_str).unwrap_or_default(),
                ME,
            ),
            name: block
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            input: block.get("input").cloned().unwrap_or_else(|| json!({})),
        },
        Some("tool_result") => Block::ToolResult {
            id: tool_ids::decode(
                block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                ME,
            ),
            content: flatten_text(block.get("content")),
            is_error: block
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
        },
        other => Block::Unknown {
            origin: ME,
            kind: other.unwrap_or("<no type field>").to_string(),
            raw: block.clone(),
        },
    }
}

fn parse_image(block: &Value) -> ImageSource {
    let source = block.get("source");
    match source.and_then(|s| s.get("type")).and_then(Value::as_str) {
        Some("url") => ImageSource::Url(
            source
                .and_then(|s| s.get("url"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ),
        _ => ImageSource::Base64 {
            media_type: source
                .and_then(|s| s.get("media_type"))
                .and_then(Value::as_str)
                .unwrap_or("image/png")
                .to_string(),
            data: source
                .and_then(|s| s.get("data"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        },
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
            .get("input_schema")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
    }
}

fn parse_tool_choice(choice: &Value) -> Option<ToolChoice> {
    match choice.get("type").and_then(Value::as_str)? {
        "auto" => Some(ToolChoice::Auto),
        "any" => Some(ToolChoice::Required),
        "none" => Some(ToolChoice::None),
        "tool" => choice
            .get("name")
            .and_then(Value::as_str)
            .map(|name| ToolChoice::Named(name.to_string())),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Request: IR → wire
// ---------------------------------------------------------------------------

/// Anthropic requires `max_tokens`, so a conversation that arrived without one
/// (both OpenAI wires treat it as optional) needs a number here.
///
/// Large enough not to truncate a real answer, which is the only failure mode
/// that matters: too high is refused by the provider with a clear error, too low
/// silently cuts the model off mid-sentence.
const DEFAULT_MAX_TOKENS: u64 = 8192;

/// Write an Anthropic Messages request.
#[must_use]
pub fn emit_request(conversation: &Conversation, model: &str) -> (Value, Dropped) {
    let mut dropped = Dropped {
        // Not modelled by the IR, so no target can emit them. Named rather
        // than dropped in silence (`docs/TRANSLATION.md`).
        tools: conversation.unexpressible_tools.clone(),
        ..Dropped::default()
    };
    let mut request = Map::new();
    request.insert("model".into(), json!(model));
    request.insert(
        "max_tokens".into(),
        json!(conversation.params.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS)),
    );
    request.insert("stream".into(), json!(conversation.params.stream));

    if !conversation.system.is_empty() {
        request.insert(
            "system".into(),
            Value::Array(
                conversation
                    .system
                    .iter()
                    .map(|chunk| {
                        let mut block = Map::new();
                        block.insert("type".into(), json!("text"));
                        block.insert("text".into(), json!(chunk.text));
                        if chunk.cache_breakpoint {
                            block.insert("cache_control".into(), json!({"type": "ephemeral"}));
                        }
                        Value::Object(block)
                    })
                    .collect(),
            ),
        );
    }

    let mut turns: Vec<Value> = Vec::new();
    for turn in &conversation.turns {
        emit_turn(turn, &mut turns, &mut dropped);
    }
    request.insert("messages".into(), Value::Array(turns));

    if let Some(temperature) = conversation.params.temperature {
        request.insert("temperature".into(), json!(temperature));
    }
    if let Some(top_p) = conversation.params.top_p {
        request.insert("top_p".into(), json!(top_p));
    }
    if !conversation.params.stop.is_empty() {
        request.insert("stop_sequences".into(), json!(conversation.params.stop));
    }
    if let Some(reasoning) = &conversation.params.reasoning
        && let Some(budget) = reasoning.budget_tokens
    {
        // Only when a budget is known. Anthropic's `thinking` needs one, and
        // inventing a number is inventing a cost.
        request.insert(
            "thinking".into(),
            json!({"type": "enabled", "budget_tokens": budget}),
        );
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
                            "name": tool.name,
                            "description": tool.description,
                            "input_schema": tool.schema,
                        })
                    })
                    .collect(),
            ),
        );
        if let Some(choice) = &conversation.tool_choice {
            request.insert(
                "tool_choice".into(),
                match choice {
                    ToolChoice::Auto => json!({"type": "auto"}),
                    ToolChoice::Required => json!({"type": "any"}),
                    ToolChoice::None => json!({"type": "none"}),
                    ToolChoice::Named(name) => json!({"type": "tool", "name": name}),
                },
            );
        }
    }

    (Value::Object(request), dropped)
}

/// One IR turn becomes one Anthropic turn.
///
/// Tool results go back into a *user* turn, which is where Anthropic puts them,
/// regardless of which wire they arrived on.
fn emit_turn(turn: &Turn, out: &mut Vec<Value>, dropped: &mut Dropped) {
    let mut results: Vec<Value> = Vec::new();
    let mut content: Vec<Value> = Vec::new();

    for block in &turn.blocks {
        match block {
            Block::Text(text) if text.is_empty() => {}
            Block::Text(text) => content.push(json!({"type": "text", "text": text})),
            Block::Image(source) => content.push(emit_image(source)),
            Block::ToolUse { id, name, input } => content.push(json!({
                "type": "tool_use",
                "id": tool_ids::encode(id, ME),
                "name": name,
                "input": input,
            })),
            Block::ToolResult {
                id,
                content: text,
                is_error,
            } => results.push(json!({
                "type": "tool_result",
                "tool_use_id": tool_ids::encode(id, ME),
                "content": text,
                "is_error": is_error,
            })),
            Block::Reasoning(reasoning) => {
                // Replayed verbatim only to the API that minted it. Anywhere
                // else the signature is meaningless, and Anthropic rejects a
                // block whose signature it did not issue — so a foreign one is
                // dropped rather than forwarded as prose, which would put the
                // model's private reasoning into the visible transcript.
                if reasoning.origin == ME
                    && let Some(raw) = reasoning.opaque.get("raw")
                {
                    content.push(raw.clone());
                } else {
                    dropped.reasoning_blocks += 1;
                }
            }
            Block::Unknown { origin, kind, raw } => {
                // A block Anthropic itself sent goes back untouched, which is
                // the round trip. One from another wire has no meaning here, so
                // it is named and the route refused upstream.
                if *origin == ME {
                    content.push(raw.clone());
                } else {
                    dropped.note_unknown(kind);
                }
            }
        }
    }

    if !results.is_empty() {
        out.push(json!({"role": "user", "content": results}));
    }
    if !content.is_empty() {
        out.push(json!({
            "role": match turn.role { Role::Assistant => "assistant", Role::User => "user" },
            "content": content,
        }));
    }
}

fn emit_image(source: &ImageSource) -> Value {
    match source {
        ImageSource::Base64 { media_type, data } => json!({
            "type": "image",
            "source": {"type": "base64", "media_type": media_type, "data": data},
        }),
        ImageSource::Url(url) => json!({
            "type": "image",
            "source": {"type": "url", "url": url},
        }),
    }
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/// Parse a non-streaming Anthropic answer.
#[must_use]
pub fn parse_completion(response: &Value) -> Completion {
    Completion {
        id: response
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("msg_ironwire")
            .to_string(),
        blocks: response
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| blocks.iter().map(parse_block).collect())
            .unwrap_or_default(),
        stop: parse_stop_reason(
            response.get("stop_reason").and_then(Value::as_str),
            response.get("stop_sequence").and_then(Value::as_str),
        ),
        usage: parse_usage(response.get("usage")),
    }
}

/// Map an Anthropic `stop_reason` onto the IR.
#[must_use]
pub fn parse_stop_reason(reason: Option<&str>, sequence: Option<&str>) -> StopReason {
    match reason {
        Some("end_turn") => StopReason::EndTurn,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("tool_use") => StopReason::ToolUse,
        Some("refusal") => StopReason::Refusal,
        Some("stop_sequence") => StopReason::StopSequence(sequence.unwrap_or_default().to_string()),
        other => StopReason::Unrecognised(other.unwrap_or_default().to_string()),
    }
}

fn parse_usage(usage: Option<&Value>) -> Usage {
    let n = |key: &str| {
        usage
            .and_then(|u| u.get(key))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    Usage {
        input: n("input_tokens"),
        cached_input: n("cache_read_input_tokens"),
        output: n("output_tokens"),
        reasoning: 0,
    }
}

/// Write a non-streaming Anthropic answer.
///
/// `requested_model` is the model the client asked for, not the one that served
/// it: a foreign slug here would make the client's own bookkeeping incoherent.
#[must_use]
pub fn emit_completion(completion: &Completion, requested_model: &str) -> (Value, Dropped) {
    let mut dropped = Dropped::default();
    let content = emit_blocks(&completion.blocks, &mut dropped);
    let (stop_reason, stop_sequence) = emit_stop_reason(&completion.stop);
    (
        json!({
            "id": completion.id,
            "type": "message",
            "role": "assistant",
            "model": requested_model,
            "content": content,
            "stop_reason": stop_reason,
            "stop_sequence": stop_sequence,
            "usage": emit_usage(&completion.usage),
        }),
        dropped,
    )
}

/// The assistant-side blocks of an answer, in Anthropic shape.
#[must_use]
pub fn emit_blocks(blocks: &[Block], dropped: &mut Dropped) -> Vec<Value> {
    let mut turns = Vec::new();
    emit_turn(
        &Turn {
            role: Role::Assistant,
            blocks: blocks.to_vec(),
        },
        &mut turns,
        dropped,
    );
    turns
        .into_iter()
        .filter(|turn| turn.get("role").and_then(Value::as_str) == Some("assistant"))
        .find_map(|turn| turn.get("content").and_then(Value::as_array).cloned())
        .unwrap_or_default()
}

/// Split an IR stop reason into Anthropic's two fields.
#[must_use]
pub fn emit_stop_reason(stop: &StopReason) -> (Value, Value) {
    match stop {
        StopReason::EndTurn => (json!("end_turn"), Value::Null),
        StopReason::MaxTokens => (json!("max_tokens"), Value::Null),
        StopReason::ToolUse => (json!("tool_use"), Value::Null),
        StopReason::Refusal => (json!("refusal"), Value::Null),
        StopReason::StopSequence(sequence) => (json!("stop_sequence"), json!(sequence)),
        // Still generating, or a value we do not recognise. `null` is the
        // honest answer and the one Anthropic itself uses mid-stream.
        StopReason::Unrecognised(_) => (Value::Null, Value::Null),
    }
}

/// Write usage in Anthropic's shape.
#[must_use]
pub fn emit_usage(usage: &Usage) -> Value {
    json!({
        "input_tokens": usage.input,
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": usage.cached_input,
        "output_tokens": usage.output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude_code_body() -> Value {
        json!({
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
                {"role": "user", "content": "go ahead and fix it"}
            ]
        })
    }

    #[test]
    fn a_claude_code_turn_parses_into_the_ir() {
        let ir = parse_request(&claude_code_body());
        assert_eq!(ir.system.len(), 2);
        assert!(ir.system[0].cache_breakpoint);
        assert!(!ir.system[1].cache_breakpoint);
        assert_eq!(ir.turns.len(), 4);
        assert_eq!(ir.tools[0].name, "Bash");
        assert_eq!(ir.params.max_tokens, Some(8192));
        assert!(ir.params.stream);
        assert_eq!(
            ir.params.reasoning.as_ref().and_then(|r| r.budget_tokens),
            Some(4096)
        );

        let assistant = &ir.turns[1];
        assert_eq!(assistant.role, Role::Assistant);
        assert!(matches!(assistant.blocks[0], Block::Reasoning(_)));
        assert!(matches!(&assistant.blocks[1], Block::Text(t) if t == "Running the tests."));
        assert!(matches!(&assistant.blocks[2], Block::ToolUse { name, .. } if name == "Bash"));
    }

    /// The property the pivot buys and the pairwise design could not have: parse
    /// then emit on the same wire must lose nothing, because the IR is a
    /// superset of every wire it holds.
    #[test]
    fn an_anthropic_request_survives_a_round_trip_through_the_ir() {
        let ir = parse_request(&claude_code_body());
        let (out, dropped) = emit_request(&ir, "claude-opus-4-6");
        assert!(dropped.is_empty(), "a same-wire emit dropped {dropped:?}");

        assert_eq!(out["max_tokens"], 8192);
        assert_eq!(out["stream"], true);
        assert_eq!(out["system"][0]["text"], "You are Claude Code.");
        assert_eq!(out["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(out["system"][1]["text"], "Working directory: /repo.");
        assert_eq!(out["thinking"]["budget_tokens"], 4096);
        assert_eq!(out["tools"][0]["name"], "Bash");
        assert_eq!(out["tools"][0]["input_schema"]["type"], "object");

        // The signed block came back byte-identical, which is the one thing
        // Anthropic will reject us for getting wrong.
        let assistant = &out["messages"][1];
        assert_eq!(assistant["content"][0]["type"], "thinking");
        assert_eq!(assistant["content"][0]["signature"], "sig-abc");
        assert_eq!(assistant["content"][2]["id"], "toolu_01ABC");

        // A tool result goes back into a user turn, where Anthropic puts it.
        assert_eq!(out["messages"][2]["role"], "user");
        assert_eq!(out["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(
            out["messages"][2]["content"][0]["tool_use_id"],
            "toolu_01ABC"
        );
    }

    /// A signature from somewhere else is not replayable and must not be
    /// forwarded — Anthropic rejects a block it did not sign.
    #[test]
    fn foreign_reasoning_state_is_dropped_rather_than_replayed() {
        let ir = Conversation {
            turns: vec![Turn {
                role: Role::Assistant,
                blocks: vec![
                    Block::Reasoning(Reasoning {
                        origin: Protocol::OpenAiResponses,
                        summary: Some("thinking about it".to_string()),
                        opaque: json!({"encrypted_content": "gAAAAA"}),
                    }),
                    Block::Text("done".to_string()),
                ],
            }],
            ..Conversation::default()
        };
        let (out, dropped) = emit_request(&ir, "claude-opus-4-6");
        assert_eq!(dropped.reasoning_blocks, 1);
        assert!(
            !out.to_string().contains("gAAAAA"),
            "a foreign signature leaked into an Anthropic request"
        );
        assert_eq!(out["messages"][0]["content"][0]["text"], "done");
    }

    #[test]
    fn a_conversation_with_no_max_tokens_still_produces_a_valid_request() {
        // Anthropic requires the field; both OpenAI wires treat it as optional.
        let (out, _) = emit_request(&Conversation::default(), "claude-opus-4-6");
        assert_eq!(out["max_tokens"], DEFAULT_MAX_TOKENS);
        assert!(out["messages"].as_array().expect("array").is_empty());
        assert!(out.get("tools").is_none());
    }

    #[test]
    fn tool_choice_round_trips() {
        for (wire, ir) in [
            (json!({"type": "auto"}), ToolChoice::Auto),
            (json!({"type": "any"}), ToolChoice::Required),
            (json!({"type": "none"}), ToolChoice::None),
            (
                json!({"type": "tool", "name": "Bash"}),
                ToolChoice::Named("Bash".to_string()),
            ),
        ] {
            assert_eq!(parse_tool_choice(&wire), Some(ir.clone()));
            let conversation = Conversation {
                tools: vec![ToolDef {
                    name: "Bash".to_string(),
                    description: String::new(),
                    schema: json!({}),
                }],
                tool_choice: Some(ir),
                ..Conversation::default()
            };
            let (out, _) = emit_request(&conversation, "m");
            assert_eq!(out["tool_choice"], wire);
        }
    }

    #[test]
    fn an_answer_round_trips() {
        let response = json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "text", "text": "done"},
                {"type": "tool_use", "id": "toolu_9", "name": "Bash", "input": {"command": "ls"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 120, "cache_read_input_tokens": 20, "output_tokens": 8},
        });
        let ir = parse_completion(&response);
        assert_eq!(ir.stop, StopReason::ToolUse);
        assert_eq!(ir.usage.input, 120);
        assert_eq!(ir.usage.cached_input, 20);
        assert_eq!(ir.usage.output, 8);

        let (out, dropped) = emit_completion(&ir, "claude-opus-4-6");
        assert!(dropped.is_empty());
        assert_eq!(out["content"][0]["text"], "done");
        assert_eq!(out["content"][1]["id"], "toolu_9");
        assert_eq!(out["stop_reason"], "tool_use");
        assert_eq!(out["model"], "claude-opus-4-6");
    }

    /// The highest-consequence single field in the whole translation: report
    /// `end_turn` on a turn that issued a call and the agent stops instead of
    /// running it.
    #[test]
    fn a_pending_tool_call_never_reads_as_a_finished_turn() {
        let (reason, _) = emit_stop_reason(&StopReason::ToolUse);
        assert_eq!(reason, "tool_use");
        assert_eq!(
            parse_stop_reason(Some("tool_use"), None),
            StopReason::ToolUse
        );
    }

    #[test]
    fn an_unrecognised_block_is_named_rather_than_swallowed() {
        let ir = parse_request(&json!({"messages": [
            {"role": "user", "content": [{"type": "document", "source": {}}]}
        ]}));
        assert!(
            matches!(&ir.turns[0].blocks[0], Block::Unknown { kind, .. } if kind == "document")
        );
    }
}
