//! What the cross-family translation loses, and what it refuses to lose.
//!
//! `docs/PROTOCOL.md` §6: everything that would genuinely break a request is
//! refused outright rather than silently degraded. The translation module's own
//! doc says anything dropped is dropped "deliberately and namedly" — which was
//! true for the three cases it modelled and false for everything else, because
//! unrecognised blocks fell through a `_ => {}` and vanished without appearing
//! in the report at all.
//!
//! That is not a hypothetical. Anthropic ships new content-block types
//! regularly, and the shape of the failure is the worst available: a `document`
//! block a user asked a question about would be discarded, and the model would
//! answer as though it had never been sent.

use ironwire_core::protocol::Protocol;
use ironwire_translate::{Dropped, emit_request, parse_request};

/// Anthropic in, Chat Completions out — the pair these edges were found on.
///
/// Parsing no longer judges anything, so what used to be one call is now
/// parse-then-emit: the unknown block is recorded on the way in and refused
/// on the way out, by the target that cannot express it.
fn anthropic_to_chat_completions(
    body: &serde_json::Value,
    model: &str,
    _stream: bool,
) -> (serde_json::Value, Dropped) {
    let ir = parse_request(Protocol::AnthropicMessages, body);
    emit_request(Protocol::OpenAiChat, &ir, model)
}
use serde_json::json;

fn body_with(block: serde_json::Value) -> serde_json::Value {
    json!({
        "model": "claude-opus-4-6",
        "system": "You are Claude Code",
        "messages": [{"role": "user", "content": [block]}],
    })
}

#[test]
fn a_block_type_this_build_does_not_model_is_named() {
    let (_out, dropped) =
        anthropic_to_chat_completions(&body_with(json!({"type": "document"})), "near-x", false);
    assert_eq!(dropped.unknown_blocks, vec!["document".to_string()]);
    assert!(!dropped.is_empty(), "an unknown block must count as loss");
}

#[test]
fn several_unknown_types_are_all_named_once_each() {
    let body = json!({
        "model": "m",
        "messages": [{"role": "user", "content": [
            {"type": "document"},
            {"type": "search_result"},
            {"type": "document"},
            {"type": "server_tool_use"},
        ]}],
    });
    let (_out, dropped) = anthropic_to_chat_completions(&body, "near-x", false);
    assert_eq!(
        dropped.unknown_blocks.len(),
        3,
        "{:?}",
        dropped.unknown_blocks
    );
    assert!(dropped.unknown_blocks.contains(&"document".to_string()));
    assert!(
        dropped
            .unknown_blocks
            .contains(&"search_result".to_string())
    );
}

#[test]
fn a_block_with_no_type_is_named_rather_than_ignored() {
    // A malformed block is still content the user sent, and still something we
    // cannot faithfully carry across.
    let (_out, dropped) =
        anthropic_to_chat_completions(&body_with(json!({"text": "orphaned"})), "near-x", false);
    assert_eq!(dropped.unknown_blocks.len(), 1);
}

#[test]
fn the_types_we_do_model_are_not_reported_as_unknown() {
    // The regression that would make the refusal useless: if a type we handle
    // perfectly well were named here, every translated request would be
    // refused and the fallback lane would never be used at all.
    let body = json!({
        "model": "m",
        "system": [{"type": "text", "text": "sys", "cache_control": {"type": "ephemeral"}}],
        "messages": [
            {"role": "user", "content": [{"type": "text", "text": "hi"}]},
            {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "...", "signature": "sig"},
                {"type": "text", "text": "on it"},
                {"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"cmd": "ls"}},
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_1", "content": "ok"},
            ]},
        ],
    });
    let (_out, dropped) = anthropic_to_chat_completions(&body, "near-x", false);
    assert!(
        dropped.unknown_blocks.is_empty(),
        "a modelled type was reported as unknown: {:?}",
        dropped.unknown_blocks
    );
    // The losses we *do* accept are still counted.
    assert_eq!(dropped.reasoning_blocks, 1);
    assert!(dropped.cache_breakpoints >= 1);
}

#[test]
fn the_translation_itself_still_produces_a_usable_request() {
    // Refusing happens a layer up, in the pipeline. The translation must still
    // return something coherent so the refusal is a routing decision rather
    // than a crash.
    let (out, _dropped) =
        anthropic_to_chat_completions(&body_with(json!({"type": "document"})), "near-x", false);
    assert_eq!(out["model"], "near-x");
    assert!(out["messages"].is_array());
}

#[test]
fn an_empty_content_array_is_not_an_unknown_block() {
    let body = json!({"model": "m", "messages": [{"role": "user", "content": []}]});
    let (_out, dropped) = anthropic_to_chat_completions(&body, "near-x", false);
    assert!(dropped.unknown_blocks.is_empty());
}

#[test]
fn a_plain_string_content_is_not_an_unknown_block() {
    // The commonest shape of all — Anthropic allows `content` to be a bare
    // string rather than an array of blocks.
    let body = json!({"model": "m", "messages": [{"role": "user", "content": "just text"}]});
    let (out, dropped) = anthropic_to_chat_completions(&body, "near-x", false);
    assert!(dropped.unknown_blocks.is_empty());
    assert!(
        out["messages"].as_array().is_some_and(|m| !m.is_empty()),
        "the message was lost entirely: {out}"
    );
}

/// Codex declares three shapes in one `tools` array, and only one is a
/// function.
///
/// A built-in has no `name`; a namespace keeps its functions in a nested
/// list. Both used to come out of the parser as `ToolDef`s — the built-in as
/// a function named `""`, the namespace as a function with an empty schema —
/// so a translated Codex request carried a nameless tool that every
/// OpenAI-compatible server rejects, and quietly lost every MCP tool the user
/// had connected.
#[test]
fn a_responses_tool_list_yields_only_real_functions() {
    let body = serde_json::json!({
        "model": "gpt-5.6-codex",
        "input": "hi",
        "tools": [
            {
                "type": "function",
                "name": "exec_command",
                "description": "runs a command",
                "parameters": {"type": "object", "properties": {}}
            },
            {"type": "web_search", "external_web_access": false},
            {
                "type": "namespace",
                "name": "mcp__notion",
                "description": "Create docs",
                "tools": [
                    {"type": "function", "name": "search", "parameters": {"type": "object"}}
                ]
            }
        ]
    });

    let ir = ironwire_translate::parse_request(Protocol::OpenAiResponses, &body);
    assert_eq!(ir.tools.len(), 1);
    assert_eq!(ir.tools[0].name, "exec_command");
    assert_eq!(
        ir.unexpressible_tools,
        vec!["web_search".to_string(), "namespace".to_string()]
    );

    let (chat, dropped) =
        ironwire_translate::emit_request(Protocol::OpenAiChat, &ir, "qwen3-coder");
    let emitted = chat["tools"].as_array().expect("tools");
    assert_eq!(emitted.len(), 1, "{chat}");
    for tool in emitted {
        let name = tool["function"]["name"].as_str().unwrap_or_default();
        assert!(!name.is_empty(), "a nameless function is a 400: {tool}");
    }
    // Lost, but said out loud.
    assert_eq!(
        dropped.tools,
        vec!["web_search".to_string(), "namespace".to_string()]
    );
}
