//! Bounded, non-destructive inspection of a request body.
//!
//! The native lane forwards bytes without re-encoding them, so everything
//! routing needs must come from a *peek*: parse once, read the handful of
//! fields policy depends on, and then forward the original bytes
//! (`docs/PROTOCOL.md` §2). Nothing in this module ever produces a body.

use serde_json::Value;

use crate::capability::{ReasoningNeed, RequestRequirements};
use crate::protocol::{BackendId, ClientIdentity, Protocol};

/// Rough bytes-per-token used to size prompts without tokenising.
///
/// Deliberately crude: this feeds cache-sacrifice and context-fit decisions
/// where being within ~30% is enough, and a real tokeniser would cost more than
/// the decision is worth. Every place this is used tolerates the error.
const BYTES_PER_TOKEN_ESTIMATE: usize = 4;

/// The identifying prefix Claude Code puts in its first system block. Its
/// presence is how the Claude-subscription backend decides a request is
/// genuinely Claude Code (`docs/TRUST.md` §3) — IronWire reads this signal, and
/// never writes it.
///
/// Compiled-in default. The live value can be refreshed through the signed
/// catalog channel (`docs/UPDATES.md`), because a marker string is exactly the
/// kind of thing that changes without notice.
pub const CLAUDE_CODE_SYSTEM_PREFIX: &str = "You are Claude Code";

/// Phrases that suggest a request is asking the model to summarize the
/// conversation so it can be compacted (`docs/PROTOCOL.md` §8).
///
/// **These are a conservative starting guess, not a verified fingerprint.**
/// Each harness words its compaction prompt differently and none of them
/// document it, so the real set has to be established by observation and then
/// shipped through the catalog channel — which is precisely what that channel is
/// for. Nothing here is load-bearing: a miss costs a slightly worse routing
/// decision on one turn, never a wrong answer (see [`RequestPeek::
/// likely_compaction`]).
pub const COMPACTION_MARKERS: &[&str] = &[
    "summary of the conversation",
    "summarize the conversation",
    "conversation so far",
    "detailed summary of the conversation",
    "condense the conversation",
];

/// Prefix of the `originator` header Codex sends on every request
/// (`codex_cli_rs` from the TUI, `codex_exec` from `codex exec`).
pub const CODEX_ORIGINATOR_PREFIX: &str = "codex";

/// Prefix of the `user-agent` Claude Code sends (`claude-cli/2.1.226 …`).
pub const CLAUDE_CODE_USER_AGENT_PREFIX: &str = "claude-cli";

/// Whether a `user-agent` names Claude Code.
///
/// The system-block prefix below is prose, and prose moves: by 2.1.226 the
/// first block is a billing header and the identifying one reads "You are a
/// Claude agent, built on Anthropic's Claude Agent SDK". An identity check
/// anchored only on the old wording stops recognising Claude Code and drops the
/// session off the subscription it belongs to. The user-agent has been stable
/// across all of that, so it is the load-bearing half and the prose is the
/// backstop.
#[must_use]
pub fn user_agent_names_claude_code(user_agent: Option<&str>, prefix: &str) -> bool {
    user_agent.is_some_and(|value| value.trim().starts_with(prefix))
}

/// Whether an `originator` header names Codex.
///
/// The body-side marker below is not enough on its own. Codex 0.145 sends no
/// `instructions` field at all on a Responses request — its system prompt moved
/// into `input` — so a body-only check silently stops recognising the very
/// client the ChatGPT subscription belongs to, and the request falls off the
/// subscription without anything looking broken.
///
/// Trusting this header is the same order of trust as trusting the body: both
/// are the client's own claim about itself, and reading one is not IronWire
/// *synthesizing* an identity to unlock someone else's subscription, which is
/// the thing `docs/TRUST.md` §3 forbids.
#[must_use]
pub fn originator_names_codex(originator: Option<&str>, prefix: &str) -> bool {
    originator.is_some_and(|value| value.trim().starts_with(prefix))
}

/// Markers used to recognise a client's own identity, so the caller can supply
/// refreshed ones without this crate depending on the catalog channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityMarkers {
    /// Prefix of one of Claude Code's system blocks.
    pub claude_code_system_prefix: String,
    /// Prefix of the `user-agent` Claude Code sends. See
    /// [`user_agent_names_claude_code`].
    pub claude_code_user_agent_prefix: String,
    /// Substring identifying Codex in a Responses `instructions` field.
    pub codex_instructions_marker: String,
    /// Prefix of the `originator` header Codex sends. See
    /// [`originator_names_codex`].
    pub codex_originator_prefix: String,
    /// Phrases suggesting a compaction turn. Advisory only.
    pub compaction_markers: Vec<String>,
}

impl Default for IdentityMarkers {
    fn default() -> Self {
        Self {
            claude_code_system_prefix: CLAUDE_CODE_SYSTEM_PREFIX.to_string(),
            claude_code_user_agent_prefix: CLAUDE_CODE_USER_AGENT_PREFIX.to_string(),
            codex_instructions_marker: "Codex".to_string(),
            codex_originator_prefix: CODEX_ORIGINATOR_PREFIX.to_string(),
            compaction_markers: COMPACTION_MARKERS
                .iter()
                .map(|m| (*m).to_string())
                .collect(),
        }
    }
}

/// How many messages a conversation needs before a summarization-shaped request
/// is worth treating as a compaction.
///
/// A short conversation containing the words "summarize the conversation" is
/// almost certainly a user asking a question, not a harness compacting. The
/// threshold costs us nothing: compaction only happens in long sessions.
const COMPACTION_MIN_MESSAGES: usize = 8;

/// What policy learned from one look at the body.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestPeek {
    /// The model the client asked for, verbatim.
    pub requested_model: Option<String>,
    /// Whether the client asked for SSE.
    pub stream: bool,
    /// What the request needs a backend to preserve.
    pub requirements: RequestRequirements,
    /// Which product's own identity the request carries, if any — the
    /// eligibility signal for subscription backends (`docs/TRUST.md` §3).
    ///
    /// Naming the product rather than answering "some identity, yes or no": a
    /// subscription is served only for the client it belongs to, and one
    /// product's identity must never unlock another's.
    pub client_identity: Option<ClientIdentity>,
    /// Number of messages, for conversation-key derivation and logging.
    pub message_count: usize,
    /// Whether this looks like a request to summarize the conversation so the
    /// client can compact it (`docs/PROTOCOL.md` §8).
    ///
    /// **Advisory, and deliberately so.** Nothing about correctness may depend
    /// on it: the detection is a heuristic over undocumented client prompts,
    /// and a fingerprint of a client's wording is exactly the sort of thing
    /// that breaks silently on a client update. All it does is tell policy to
    /// value fidelity over cost for one turn — a false positive spends a little
    /// more money, a false negative gets today's behaviour.
    pub likely_compaction: bool,
    /// The subscription login this request's client is using, when the client
    /// was pointed at a login-bound façade URL (`/anthropic/account/{id}`).
    /// `None` at the plain façade, which belongs to the default login of each
    /// product.
    ///
    /// Set by the façade from the URL, never from the body: the body is
    /// forwarded as-is, and nothing in it is a reliable statement of which of
    /// the user's logins sent it.
    pub client_login: Option<BackendId>,
    /// The client's own session id, when it sent one (`x-ironwire-session-id`,
    /// or its native header, such as Codex's `session-id`).
    ///
    /// Set by the façade from the headers; `None` from [`Self::inspect`]. The
    /// conversation key is shared by every session of one client version, so
    /// this is what tells two concurrent sessions apart where it matters:
    /// which ChatGPT account minted each one's encrypted reasoning.
    pub client_session: Option<String>,
}

impl RequestPeek {
    /// Inspect a parsed request body for the given protocol.
    ///
    /// Never fails: an unrecognised body yields a peek with no requirements,
    /// which the router treats as "anything can serve this". That is the right
    /// default — a shape we do not understand is one we must not claim to have
    /// analysed.
    #[must_use]
    pub fn inspect(protocol: Protocol, body: &Value, raw_len: usize) -> Self {
        Self::inspect_with(protocol, body, raw_len, &IdentityMarkers::default())
    }

    /// Inspect with caller-supplied identity markers.
    #[must_use]
    pub fn inspect_with(
        protocol: Protocol,
        body: &Value,
        raw_len: usize,
        markers: &IdentityMarkers,
    ) -> Self {
        match protocol {
            Protocol::AnthropicMessages => Self::inspect_anthropic(body, raw_len, markers),
            Protocol::OpenAiResponses | Protocol::OpenAiChat => {
                Self::inspect_openai(body, raw_len, markers)
            }
        }
    }

    fn inspect_anthropic(body: &Value, raw_len: usize, markers: &IdentityMarkers) -> Self {
        let messages = body.get("messages").and_then(Value::as_array);
        let system = body.get("system");

        let mut requirements = RequestRequirements {
            admission_backend: None,
            reasoning_backend: None,
            tools: body
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(|t| !t.is_empty()),
            // Set below from the history: what matters is whether the client
            // *depends* on parallel calls, not whether it permits them. Anthropic
            // permits them by default, so the permissive reading would refuse
            // every serial backend for every request.
            parallel_tool_calls: false,
            images: false,
            reasoning: if body.get("thinking").is_some() {
                ReasoningNeed::Requested
            } else {
                ReasoningNeed::None
            },
            prompt_cache: false,
            cached_prefix_tokens: 0,
            structured_output: false,
            min_context_tokens: estimate_tokens(raw_len),
            mid_tool_loop: anthropic_mid_tool_loop(messages),
            encrypted_reasoning: false,
        };

        // A signed thinking block anywhere in the replayed history means a family
        // change would lose reasoning continuity — recorded so the route can say
        // so, but not a refusal (see `capability::ReasoningNeed`).
        let mut cached_prefix_bytes = 0usize;
        if let Some(messages) = messages {
            for message in messages {
                let Some(blocks) = message.get("content").and_then(Value::as_array) else {
                    continue;
                };
                let mut tool_uses_in_turn = 0usize;
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("image") => requirements.images = true,
                        Some("thinking" | "redacted_thinking")
                            if block.get("signature").is_some() || block.get("data").is_some() =>
                        {
                            requirements.reasoning = ReasoningNeed::LoadBearing;
                        }
                        _ => {}
                    }
                    if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                        tool_uses_in_turn += 1;
                    }
                    if block.get("cache_control").is_some() {
                        requirements.prompt_cache = true;
                        // Everything up to and including this breakpoint is
                        // cacheable; approximate it by the bytes seen so far.
                        cached_prefix_bytes = cached_prefix_bytes.max(serialized_len_hint(message));
                    }
                }
                if tool_uses_in_turn > 1 {
                    requirements.parallel_tool_calls = true;
                }
            }
        }
        if let Some(system) = system
            && json_contains_key(system, "cache_control")
        {
            requirements.prompt_cache = true;
            cached_prefix_bytes = cached_prefix_bytes.max(serialized_len_hint(system));
        }
        // The cacheable prefix is everything before the last breakpoint, which
        // in practice is nearly the whole body for a long coding session.
        if requirements.prompt_cache {
            requirements.cached_prefix_tokens =
                estimate_tokens(cached_prefix_bytes.max(raw_len / 2));
        }
        if body
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.iter().any(|t| json_contains_key(t, "cache_control")))
        {
            requirements.prompt_cache = true;
        }

        Self {
            requested_model: body
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string),
            stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
            client_identity: anthropic_system_names(system, &markers.claude_code_system_prefix)
                .then_some(ClientIdentity::ClaudeCode),
            message_count: messages.map_or(0, Vec::len),
            likely_compaction: looks_like_compaction(
                messages.map_or(0, Vec::len),
                &anthropic_trailing_text(messages),
                markers,
            ),
            requirements,
            client_login: None,
            client_session: None,
        }
    }

    fn inspect_openai(body: &Value, raw_len: usize, markers: &IdentityMarkers) -> Self {
        // `input` is the Responses API; `messages` is Chat Completions.
        let items = body
            .get("input")
            .and_then(Value::as_array)
            .or_else(|| body.get("messages").and_then(Value::as_array));

        let mut requirements = RequestRequirements {
            admission_backend: None,
            reasoning_backend: None,
            tools: body
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(|t| !t.is_empty()),
            // As on the Anthropic side: set from the history, not from the
            // permissive default.
            parallel_tool_calls: false,
            images: false,
            reasoning: if body.get("reasoning").is_some() {
                ReasoningNeed::Requested
            } else {
                ReasoningNeed::None
            },
            // OpenAI caches automatically; there are no client breakpoints to
            // preserve, so there is nothing here for the gate to protect.
            prompt_cache: false,
            cached_prefix_tokens: 0,
            structured_output: body
                .pointer("/response_format/type")
                .and_then(Value::as_str)
                .is_some_and(|t| t == "json_schema")
                || body.pointer("/text/format/type").and_then(Value::as_str) == Some("json_schema"),
            min_context_tokens: estimate_tokens(raw_len),
            mid_tool_loop: openai_mid_tool_loop(items),
            encrypted_reasoning: false,
        };

        if let Some(items) = items {
            let mut pending_calls = 0usize;
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    // Responses API: consecutive calls before their outputs.
                    Some("function_call") => pending_calls += 1,
                    Some("function_call_output") => pending_calls = 0,
                    _ => {}
                }
                if pending_calls > 1 {
                    requirements.parallel_tool_calls = true;
                }
                // Chat Completions: several calls on one assistant message.
                if item
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .is_some_and(|calls| calls.len() > 1)
                {
                    requirements.parallel_tool_calls = true;
                }
                // Encrypted reasoning state: same permanence as a signed
                // Anthropic thinking block, opposite direction.
                if item.get("type").and_then(Value::as_str) == Some("reasoning")
                    && item.get("encrypted_content").is_some()
                {
                    requirements.reasoning = ReasoningNeed::LoadBearing;
                    // Stricter than the line above: only actual content is
                    // bound to an account. A `null` placeholder carries
                    // nothing a provider could reject, and treating it as
                    // bound would pin a conversation for no reason.
                    if item
                        .get("encrypted_content")
                        .and_then(Value::as_str)
                        .is_some_and(|content| !content.is_empty())
                    {
                        requirements.encrypted_reasoning = true;
                    }
                }
                if json_contains_type(item, "input_image") || json_contains_type(item, "image_url")
                {
                    requirements.images = true;
                }
            }
        }

        Self {
            requested_model: body
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string),
            stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
            // Codex identifies itself with an `originator`/`instructions` pair;
            // the presence of its instructions block is the reliable half.
            client_identity: body
                .get("instructions")
                .and_then(Value::as_str)
                .is_some_and(|s| s.contains(&markers.codex_instructions_marker))
                .then_some(ClientIdentity::Codex),
            message_count: items.map_or(0, Vec::len),
            likely_compaction: looks_like_compaction(
                items.map_or(0, Vec::len),
                &openai_trailing_text(items),
                markers,
            ),
            requirements,
            client_login: None,
            client_session: None,
        }
    }
}

/// Whether this request reads as "summarize what we have said so far".
///
/// Keyed on the *trailing* message, because that is where a harness puts its
/// compaction instruction — and keyed on a length threshold, because a short
/// conversation containing those words is a user asking a question.
fn looks_like_compaction(message_count: usize, trailing: &str, markers: &IdentityMarkers) -> bool {
    if message_count < COMPACTION_MIN_MESSAGES || trailing.is_empty() {
        return false;
    }
    let haystack = trailing.to_ascii_lowercase();
    markers
        .compaction_markers
        .iter()
        .any(|marker| haystack.contains(&marker.to_ascii_lowercase()))
}

/// Text of the last message, however the client chose to shape it.
fn anthropic_trailing_text(messages: Option<&Vec<Value>>) -> String {
    let Some(last) = messages.and_then(|m| m.last()) else {
        return String::new();
    };
    collect_text(last.get("content"))
}

/// The Responses and Chat Completions equivalents.
fn openai_trailing_text(items: Option<&Vec<Value>>) -> String {
    let Some(last) = items.and_then(|i| i.last()) else {
        return String::new();
    };
    collect_text(last.get("content"))
}

/// Flatten whatever shape a content field takes into searchable text.
///
/// Bounded: a compaction instruction is a short sentence, and scanning a
/// megabyte of replayed history for it would cost more than the decision is
/// worth.
fn collect_text(content: Option<&Value>) -> String {
    const MAX: usize = 4096;
    let mut out = String::new();
    fn walk(value: &Value, out: &mut String, max: usize) {
        if out.len() >= max {
            return;
        }
        match value {
            Value::String(s) => {
                out.push_str(&s[..s.len().min(max - out.len())]);
                out.push(' ');
            }
            Value::Array(items) => items.iter().for_each(|v| walk(v, out, max)),
            Value::Object(map) => {
                if let Some(text) = map.get("text").or_else(|| map.get("content")) {
                    walk(text, out, max);
                }
            }
            _ => {}
        }
    }
    if let Some(content) = content {
        walk(content, &mut out, MAX);
    }
    out
}

/// Whether the conversation is mid tool loop: the last message replays tool
/// results, so the model is expected to continue an exchange already in flight.
///
/// This is the cross-family switch point (`docs/PROTOCOL.md` §6). It is
/// deliberately keyed on the *last* message rather than "any tool use in
/// history" — a long session is mid-loop only between a tool call and the turn
/// that consumes its result, and a rule keyed on history would never let a
/// tool-using agent change families at all.
fn anthropic_mid_tool_loop(messages: Option<&Vec<Value>>) -> bool {
    let Some(last) = messages.and_then(|m| m.last()) else {
        return false;
    };
    if last.get("role").and_then(Value::as_str) != Some("user") {
        return false;
    }
    last.get("content")
        .and_then(Value::as_array)
        .is_some_and(|blocks| {
            blocks
                .iter()
                .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
        })
}

/// The OpenAI equivalent: a trailing `function_call_output` item (Responses) or
/// a trailing `role: "tool"` message (Chat Completions).
fn openai_mid_tool_loop(items: Option<&Vec<Value>>) -> bool {
    let Some(last) = items.and_then(|i| i.last()) else {
        return false;
    };
    last.get("type").and_then(Value::as_str) == Some("function_call_output")
        || last.get("role").and_then(Value::as_str) == Some("tool")
}

/// Anthropic's `system` is either a string or an array of blocks. Return the
/// text of the first block either way.
/// Whether any system block starts with `prefix`.
///
/// Every block, not just the first: Claude Code 2.1.226 puts a billing header
/// in block 0 and its identifying text after it, so a check anchored on the
/// first block alone sees a string that identifies nothing.
fn anthropic_system_names(system: Option<&Value>, prefix: &str) -> bool {
    let starts = |s: &str| s.starts_with(prefix);
    match system {
        Some(Value::String(s)) => starts(s),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .any(starts),
        _ => false,
    }
}

fn estimate_tokens(bytes: usize) -> u32 {
    u32::try_from(bytes / BYTES_PER_TOKEN_ESTIMATE).unwrap_or(u32::MAX)
}

/// Cheap size hint without re-serialising: string lengths dominate a request
/// body, so summing them tracks the real size closely enough for the decisions
/// that consume it.
fn serialized_len_hint(value: &Value) -> usize {
    match value {
        Value::String(s) => s.len(),
        Value::Array(items) => items.iter().map(serialized_len_hint).sum(),
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| k.len() + serialized_len_hint(v))
            .sum(),
        Value::Null => 4,
        Value::Bool(_) => 5,
        Value::Number(_) => 8,
    }
}

fn json_contains_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(map) => {
            map.contains_key(key) || map.values().any(|v| json_contains_key(v, key))
        }
        Value::Array(items) => items.iter().any(|v| json_contains_key(v, key)),
        _ => false,
    }
}

fn json_contains_type(value: &Value, ty: &str) -> bool {
    match value {
        Value::Object(map) => {
            map.get("type").and_then(Value::as_str) == Some(ty)
                || map.values().any(|v| json_contains_type(v, ty))
        }
        Value::Array(items) => items.iter().any(|v| json_contains_type(v, ty)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn peek_anthropic(body: serde_json::Value) -> RequestPeek {
        let raw = body.to_string();
        RequestPeek::inspect(Protocol::AnthropicMessages, &body, raw.len())
    }

    #[test]
    fn detects_claude_code_identity_from_string_system() {
        let p = peek_anthropic(json!({
            "model": "claude-opus-4-6",
            "system": "You are Claude Code, Anthropic's official CLI for Claude.",
            "messages": [],
        }));
        assert_eq!(p.client_identity, Some(ClientIdentity::ClaudeCode));
    }

    #[test]
    fn detects_claude_code_identity_from_block_system() {
        let p = peek_anthropic(json!({
            "model": "claude-opus-4-6",
            "system": [{"type": "text", "text": "You are Claude Code, Anthropic's official CLI."}],
            "messages": [],
        }));
        assert_eq!(p.client_identity, Some(ClientIdentity::ClaudeCode));
    }

    #[test]
    fn a_third_party_client_does_not_carry_claude_code_identity() {
        // This is the signal that keeps Aider off the Claude subscription
        // without us forging anything (TRUST.md §3).
        let p = peek_anthropic(json!({
            "model": "claude-opus-4-6",
            "system": "You are a helpful coding assistant.",
            "messages": [],
        }));
        assert_eq!(p.client_identity, None);
    }

    /// Claude Code 2.1.226 leads with a billing header, so the identifying
    /// block is no longer block 0 — a first-block-only check saw a string that
    /// identifies nothing and dropped the session off its own subscription.
    #[test]
    fn the_identifying_block_is_found_when_it_is_not_the_first() {
        let p = peek_anthropic(json!({
            "model": "claude-opus-5",
            "system": [
                {"type": "text", "text": "x-anthropic-billing-header: cc_version=2.1.226;"},
                {"type": "text", "text": "You are Claude Code, Anthropic's official CLI."},
            ],
            "messages": [],
        }));
        assert_eq!(p.client_identity, Some(ClientIdentity::ClaudeCode));
    }

    /// The other half of the same regression: by 2.1.226 the `-p` entrypoint
    /// says "You are a Claude agent", which no system-prose marker matches. The
    /// user-agent is what still names the client.
    #[test]
    fn the_user_agent_names_claude_code_when_the_prose_does_not() {
        let markers = IdentityMarkers::default();
        assert!(user_agent_names_claude_code(
            Some("claude-cli/2.1.226 (external, sdk-cli)"),
            &markers.claude_code_user_agent_prefix,
        ));
        assert!(!user_agent_names_claude_code(
            Some("aider/0.90.0"),
            &markers.claude_code_user_agent_prefix,
        ));
        assert!(!user_agent_names_claude_code(
            None,
            &markers.claude_code_user_agent_prefix
        ));
    }

    #[test]
    fn the_originator_names_codex_when_the_body_does_not() {
        let markers = IdentityMarkers::default();
        for originator in ["codex_cli_rs", "codex_exec"] {
            assert!(originator_names_codex(
                Some(originator),
                &markers.codex_originator_prefix
            ));
        }
        assert!(!originator_names_codex(
            Some("some_other_tool"),
            &markers.codex_originator_prefix
        ));
        assert!(!originator_names_codex(
            None,
            &markers.codex_originator_prefix
        ));
    }

    #[test]
    fn signed_thinking_in_history_is_load_bearing() {
        let p = peek_anthropic(json!({
            "model": "claude-opus-4-6",
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "...", "signature": "abc123"},
                    {"type": "text", "text": "done"}
                ]
            }],
        }));
        assert_eq!(p.requirements.reasoning, ReasoningNeed::LoadBearing);
    }

    #[test]
    fn requesting_thinking_without_history_is_only_requested() {
        let p = peek_anthropic(json!({
            "model": "claude-opus-4-6",
            "thinking": {"type": "enabled", "budget_tokens": 4000},
            "messages": [{"role": "user", "content": "hi"}],
        }));
        assert_eq!(p.requirements.reasoning, ReasoningNeed::Requested);
    }

    #[test]
    fn cache_control_anywhere_marks_the_prefix() {
        let p = peek_anthropic(json!({
            "model": "claude-opus-4-6",
            "system": [{"type": "text", "text": "big prompt", "cache_control": {"type": "ephemeral"}}],
            "messages": [{"role": "user", "content": "hi"}],
        }));
        assert!(p.requirements.prompt_cache);
        assert!(p.requirements.cached_prefix_tokens > 0);
    }

    #[test]
    fn images_are_detected_in_message_blocks() {
        let p = peek_anthropic(json!({
            "model": "claude-opus-4-6",
            "messages": [{
                "role": "user",
                "content": [{"type": "image", "source": {"type": "base64", "data": "AAA"}}]
            }],
        }));
        assert!(p.requirements.images);
    }

    #[test]
    fn parallel_tools_are_flagged_only_when_the_history_actually_uses_them() {
        // Anthropic permits parallel calls by default, so reading "permitted"
        // as "required" would refuse every serial backend for every request.
        let permitted =
            peek_anthropic(json!({"model": "m", "tools": [{"name": "Read"}], "messages": []}));
        assert!(!permitted.requirements.parallel_tool_calls);

        let used = peek_anthropic(json!({
            "model": "m",
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {}},
                    {"type": "tool_use", "id": "toolu_2", "name": "Read", "input": {}}
                ]
            }],
        }));
        assert!(used.requirements.parallel_tool_calls);

        let single = peek_anthropic(json!({
            "model": "m",
            "messages": [{
                "role": "assistant",
                "content": [{"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {}}]
            }],
        }));
        assert!(!single.requirements.parallel_tool_calls);
    }

    #[test]
    fn a_replayed_tool_result_marks_the_conversation_mid_tool_loop() {
        // This is the cross-family switch gate — see capability::eligible.
        let mid = peek_anthropic(json!({
            "model": "m",
            "messages": [
                {"role": "user", "content": "fix it"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "failed"}
                ]}
            ],
        }));
        assert!(mid.requirements.mid_tool_loop);
    }

    #[test]
    fn a_fresh_user_turn_is_a_clean_boundary_even_after_many_tool_calls() {
        // Keyed on the last message, not on history: a long tool-using session
        // must still be able to change families between turns.
        let boundary = peek_anthropic(json!({
            "model": "m",
            "messages": [
                {"role": "user", "content": "fix it"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "ok"}
                ]},
                {"role": "assistant", "content": [{"type": "text", "text": "done"}]},
                {"role": "user", "content": "now add a test"}
            ],
        }));
        assert!(!boundary.requirements.mid_tool_loop);
    }

    #[test]
    fn openai_mid_loop_is_detected_in_both_dialects() {
        let responses = json!({
            "model": "gpt-5.6",
            "input": [
                {"type": "function_call", "call_id": "call_1", "name": "bash"},
                {"type": "function_call_output", "call_id": "call_1", "output": "ok"}
            ],
        });
        let raw = responses.to_string();
        assert!(
            RequestPeek::inspect(Protocol::OpenAiResponses, &responses, raw.len())
                .requirements
                .mid_tool_loop
        );

        let chat = json!({
            "model": "m",
            "messages": [
                {"role": "assistant", "tool_calls": [{"id": "call_1"}]},
                {"role": "tool", "tool_call_id": "call_1", "content": "ok"}
            ],
        });
        let raw = chat.to_string();
        assert!(
            RequestPeek::inspect(Protocol::OpenAiChat, &chat, raw.len())
                .requirements
                .mid_tool_loop
        );
    }

    #[test]
    fn encrypted_openai_reasoning_is_load_bearing() {
        let body = json!({
            "model": "gpt-5.6",
            "input": [
                {"type": "reasoning", "encrypted_content": "gAAAA..."},
                {"type": "message", "role": "user", "content": "hi"}
            ],
        });
        let raw = body.to_string();
        let p = RequestPeek::inspect(Protocol::OpenAiResponses, &body, raw.len());
        assert_eq!(p.requirements.reasoning, ReasoningNeed::LoadBearing);
        assert!(
            p.requirements.encrypted_reasoning,
            "encrypted reasoning binds the conversation to the account that minted it"
        );
    }

    /// The account lock keys on this flag, so it must be precise: a Codex first
    /// turn — no reasoning replayed yet — is free to start on any account, and
    /// a `null` placeholder carries nothing a provider could reject.
    #[test]
    fn only_replayed_encrypted_content_binds_a_conversation_to_an_account() {
        for body in [
            json!({"model": "gpt-5.6", "reasoning": {"effort": "high"},
                   "include": ["reasoning.encrypted_content"],
                   "input": [{"type": "message", "role": "user", "content": "hi"}]}),
            json!({"model": "gpt-5.6",
                   "input": [{"type": "reasoning", "summary": [], "encrypted_content": null}]}),
            json!({"model": "gpt-5.6",
                   "input": [{"type": "reasoning", "summary": [], "encrypted_content": ""}]}),
        ] {
            let raw = body.to_string();
            let p = RequestPeek::inspect(Protocol::OpenAiResponses, &body, raw.len());
            assert!(!p.requirements.encrypted_reasoning, "{body}");
        }
    }

    /// The peek reads the body; which login sent it is the façade's to say,
    /// from the URL.
    #[test]
    fn the_peek_never_claims_a_login() {
        let body = json!({"model": "gpt-5.6", "instructions": "You are Codex", "input": []});
        let raw = body.to_string();
        assert_eq!(
            RequestPeek::inspect(Protocol::OpenAiResponses, &body, raw.len()).client_login,
            None
        );
        assert_eq!(
            peek_anthropic(json!({"model": "m", "messages": []})).client_login,
            None
        );
    }

    #[test]
    fn an_unrecognised_body_claims_nothing() {
        let body = json!({"something": "entirely else"});
        let p = RequestPeek::inspect(Protocol::AnthropicMessages, &body, 32);
        assert_eq!(p.requested_model, None);
        assert!(!p.stream);
        assert_eq!(p.client_identity, None);
        assert!(!p.requirements.tools);
        assert_eq!(p.message_count, 0);
    }

    #[test]
    fn a_long_session_asking_for_a_summary_reads_as_compaction() {
        let mut messages: Vec<serde_json::Value> = (0..12)
            .map(|i| json!({"role": "user", "content": format!("m{i}")}))
            .collect();
        messages.push(json!({
            "role": "user",
            "content": "Provide a detailed summary of the conversation so far."
        }));
        let p = peek_anthropic(json!({"model": "m", "messages": messages}));
        assert!(p.likely_compaction);
    }

    #[test]
    fn a_short_conversation_mentioning_a_summary_does_not() {
        // Someone asking "summarize the conversation" three turns in is a user
        // asking a question, not a harness compacting. A false positive only
        // costs money, but it should still not be free to trigger.
        let p = peek_anthropic(json!({
            "model": "m",
            "messages": [
                {"role": "user", "content": "summarize the conversation so far"},
            ],
        }));
        assert!(!p.likely_compaction);
    }

    #[test]
    fn an_ordinary_long_turn_does_not() {
        let messages: Vec<serde_json::Value> = (0..20)
            .map(|i| json!({"role": "user", "content": format!("fix the bug in file {i}")}))
            .collect();
        let p = peek_anthropic(json!({"model": "m", "messages": messages}));
        assert!(!p.likely_compaction);
    }

    #[test]
    fn the_marker_is_read_from_block_content_too() {
        // Claude Code sends content as blocks, not a bare string.
        let mut messages: Vec<serde_json::Value> = (0..12)
            .map(|i| json!({"role": "user", "content": format!("m{i}")}))
            .collect();
        messages.push(json!({
            "role": "user",
            "content": [{"type": "text", "text": "Summarize the conversation for me."}],
        }));
        let p = peek_anthropic(json!({"model": "m", "messages": messages}));
        assert!(p.likely_compaction);
    }

    #[test]
    fn detection_is_keyed_on_the_last_message_not_the_history() {
        // The instruction lives in the trailing message. A conversation that
        // merely *discussed* summarizing, twenty turns ago, is not compacting.
        let mut messages = vec![json!({
            "role": "user",
            "content": "Should I ask you for a summary of the conversation later?"
        })];
        messages.extend((0..15).map(|i| json!({"role": "user", "content": format!("m{i}")})));
        let p = peek_anthropic(json!({"model": "m", "messages": messages}));
        assert!(!p.likely_compaction);
    }

    #[test]
    fn openai_dialects_detect_it_too() {
        let mut input: Vec<serde_json::Value> = (0..12)
            .map(|i| json!({"type": "message", "role": "user", "content": format!("m{i}")}))
            .collect();
        input.push(json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Condense the conversation into a summary."}],
        }));
        let body = json!({"model": "gpt-5.6", "input": input});
        let raw = body.to_string();
        assert!(
            RequestPeek::inspect(Protocol::OpenAiResponses, &body, raw.len()).likely_compaction
        );
    }

    #[test]
    fn an_empty_marker_set_disables_detection_entirely() {
        // The catalog channel can turn this off by shipping no markers, which is
        // the escape hatch if the heuristic turns out to misfire in the field.
        let mut messages: Vec<serde_json::Value> = (0..12)
            .map(|i| json!({"role": "user", "content": format!("m{i}")}))
            .collect();
        messages.push(json!({"role": "user", "content": "summarize the conversation"}));
        let body = json!({"model": "m", "messages": messages});
        let raw = body.to_string();
        let markers = IdentityMarkers {
            compaction_markers: Vec::new(),
            ..IdentityMarkers::default()
        };
        let p = RequestPeek::inspect_with(Protocol::AnthropicMessages, &body, raw.len(), &markers);
        assert!(!p.likely_compaction);
    }

    #[test]
    fn stream_flag_round_trips() {
        assert!(peek_anthropic(json!({"model": "m", "stream": true, "messages": []})).stream);
        assert!(!peek_anthropic(json!({"model": "m", "messages": []})).stream);
    }
}
