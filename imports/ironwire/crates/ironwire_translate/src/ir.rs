//! The pivot: one representation every wire parses into and emits from.
//!
//! `docs/TRANSLATION.md` is the design. The short version: three protocols make
//! six ordered pairs and eighteen mappings done pairwise, so everything goes
//! through a canonical form instead — three parsers in, three emitters out.
//!
//! Two properties hold everywhere in this module, and everything else depends
//! on them:
//!
//! 1. **The IR is a superset, not one of the wires.** Pivoting on Chat
//!    Completions would have been cheaper and would have silently degraded
//!    every route between the two formats that are *richer* than it.
//! 2. **Parsing is lossless; emitting reports the loss.** What a client sent is
//!    a fact. What a target can carry is a fact about that target. One function
//!    per pair conflates them; a pivot cannot.

use serde_json::Value;

use ironwire_core::protocol::Protocol;

/// Who produced a turn.
///
/// Tool results are deliberately **not** a role here — see [`Block::ToolResult`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The user, or the agent acting on their behalf.
    User,
    /// The model.
    Assistant,
}

/// One piece of a system prompt.
///
/// A list rather than a string because Anthropic sends a list and marks cache
/// breakpoints on individual entries. Flattening on parse would throw away the
/// breakpoint positions before any target got to decide whether it wanted them.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemChunk {
    /// The text.
    pub text: String,
    /// Whether the client marked a prompt-cache breakpoint here.
    pub cache_breakpoint: bool,
}

/// Where an image's bytes are.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageSource {
    /// Inline base64.
    Base64 {
        /// e.g. `image/png`.
        media_type: String,
        /// The base64 payload, exactly as it arrived.
        data: String,
    },
    /// A URL the provider is expected to fetch.
    Url(String),
}

/// Provider-private reasoning state.
///
/// Signed Anthropic `thinking` and encrypted OpenAI reasoning items are
/// replayable **only** to the provider that minted them, and inert-but-harmless
/// anywhere else (`docs/PROTOCOL.md` §6). Carrying the origin is what lets an
/// emitter make that call from a fact rather than a guess: replay the opaque
/// part when emitting back to `origin`, drop it — counted — otherwise.
///
/// `summary` is the human-readable reasoning text, and it is parsed even though
/// **no emitter forwards it across a wire boundary**. That is not an oversight:
/// parsing is lossless by rule, and having the field is what lets the emitters
/// be tested for dropping it rather than leaving "the parser threw it away"
/// indistinguishable from "the emitter refused to send it". Folding a summary
/// into visible content would put the model's private reasoning into the
/// transcript as prose it never said, which is the move `events.rs` refuses
/// when it will not write into a response stream.
#[derive(Debug, Clone, PartialEq)]
pub struct Reasoning {
    /// The wire that minted the opaque part.
    pub origin: Protocol,
    /// Human-readable reasoning text. Never emitted across a wire boundary; see
    /// the type's own note.
    pub summary: Option<String>,
    /// The signature / encrypted payload, meaningful only to `origin`.
    pub opaque: Value,
}

/// A tool-call identifier, in its **original** namespace.
///
/// Always the id the minting provider used, never a half-translated one:
/// encoding into a target's namespace happens at the emitter
/// (`crate::tool_ids`).
///
/// `origin` is what makes encoding reversible. Anthropic wants ids shaped like
/// `toolu_*`, so a foreign one is carried as `toolu_xw_<original>` — but an
/// Anthropic-native id must go back **unchanged**, and the string alone cannot
/// tell the two apart. Recording where an id came from can: native to the
/// target means pass it through, anything else means encode it.
///
/// `None` means "minted elsewhere, and we no longer know where" — what is left
/// after decoding an id IronWire itself handed out.
#[derive(Debug, Clone)]
pub struct ToolCallId {
    origin: Option<Protocol>,
    id: String,
}

impl ToolCallId {
    /// An id native to `origin`.
    #[must_use]
    pub fn native(origin: Protocol, id: impl Into<String>) -> Self {
        Self {
            origin: Some(origin),
            id: id.into(),
        }
    }

    /// An id from a provider we can no longer name.
    #[must_use]
    pub fn foreign(id: impl Into<String>) -> Self {
        Self {
            origin: None,
            id: id.into(),
        }
    }

    /// Borrow the identifier itself.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.id
    }

    /// Whether this id is already valid, unchanged, on `protocol`.
    #[must_use]
    pub fn is_native_to(&self, protocol: Protocol) -> bool {
        self.origin == Some(protocol)
    }
}

/// Identity **is** the string. `origin` is provenance, carried so an emitter can
/// choose an encoding — two ids with the same string denote the same call
/// whatever route each took to get here, and pairing a call with its result
/// depends on exactly that.
impl PartialEq for ToolCallId {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for ToolCallId {}

impl std::hash::Hash for ToolCallId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl From<&str> for ToolCallId {
    fn from(s: &str) -> Self {
        Self::foreign(s)
    }
}

/// One piece of a turn.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// Prose.
    Text(String),
    /// An image input.
    Image(ImageSource),
    /// The model asking for a tool to be run.
    ToolUse {
        /// Identifier the client will replay.
        id: ToolCallId,
        /// Tool name.
        name: String,
        /// Arguments, as a parsed object.
        input: Value,
    },
    /// The result of running one.
    ///
    /// A block rather than a role because the three wires disagree about where
    /// it lives: Anthropic packs results into a user turn, Chat Completions
    /// gives each its own `role: "tool"` message, and Responses uses a
    /// `function_call_output` item. Modelling it as a block and letting each
    /// emitter place it is the only version that survives all three.
    ToolResult {
        /// The call this answers.
        id: ToolCallId,
        /// Result text, flattened.
        content: String,
        /// Whether the tool reported a failure.
        is_error: bool,
    },
    /// Provider-private reasoning; see [`Reasoning`].
    Reasoning(Reasoning),
    /// Content this build does not model, kept whole.
    ///
    /// Carried rather than discarded so a same-protocol round trip preserves it
    /// and a cross-protocol emit can name it precisely. `origin` is what makes
    /// both possible: a block goes back to the wire it came from untouched —
    /// which is the round trip — and anywhere else it is reported in
    /// [`Dropped::unknown_blocks`] and the route is refused rather than
    /// degraded, because a `document` the user asked about is indistinguishable
    /// from a decorative one.
    Unknown {
        /// The wire it arrived on.
        origin: Protocol,
        /// The `type` field, or a stand-in when there was none.
        kind: String,
        /// The original JSON.
        raw: Value,
    },
}

/// One turn of the conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    /// Who produced it.
    pub role: Role,
    /// Its content, in order.
    pub blocks: Vec<Block>,
}

/// A tool the client declared.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDef {
    /// Name the model calls.
    pub name: String,
    /// What it does.
    pub description: String,
    /// JSON Schema for the arguments.
    pub schema: Value,
}

/// How the client wants tool selection constrained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolChoice {
    /// The model decides.
    Auto,
    /// The model must call something.
    Required,
    /// The model must not call anything.
    None,
    /// The model must call this one.
    Named(String),
}

/// How much reasoning the client asked for.
///
/// The three wires spell this differently — Anthropic budgets tokens, OpenAI
/// names an effort level, Chat Completions has no standard field at all — so
/// both spellings are carried and each emitter takes what it can use.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReasoningRequest {
    /// `low` / `medium` / `high`, where the wire says it that way.
    pub effort: Option<String>,
    /// Token budget, where the wire says it that way.
    pub budget_tokens: Option<u64>,
    /// Whether a reasoning summary was requested.
    pub summary: bool,
}

/// Sampling and length controls.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Params {
    /// Output cap. Anthropic requires one; the others treat it as optional.
    pub max_tokens: Option<u64>,
    /// Sampling temperature.
    pub temperature: Option<f64>,
    /// Nucleus sampling.
    pub top_p: Option<f64>,
    /// Stop sequences.
    pub stop: Vec<String>,
    /// Reasoning request, if any.
    pub reasoning: Option<ReasoningRequest>,
    /// Whether the client asked for SSE.
    pub stream: bool,
    /// Whether per-token log-probabilities were asked for.
    ///
    /// Only one wire can express this. Chat Completions has `logprobs: true`;
    /// Anthropic Messages has no such parameter at all and rejects an unknown
    /// field, and Responses spells it as `top_logprobs` plus an `include`
    /// entry rather than a boolean. So this is carried in the pivot and
    /// honoured by exactly one emitter — the other two drop it silently, which
    /// is the one place in this module that is correct rather than a bug: a
    /// request the target cannot express is not a request that lost content.
    pub logprobs: bool,
}

/// A whole request, wire-independent.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Conversation {
    /// System prompt, in pieces.
    pub system: Vec<SystemChunk>,
    /// The exchange so far.
    pub turns: Vec<Turn>,
    /// Declared tools, as functions. Only functions: see
    /// [`Self::unexpressible_tools`].
    pub tools: Vec<ToolDef>,
    /// Tool entries the IR cannot carry, by their wire `type`.
    ///
    /// Responses lets a client declare things that are not functions at all —
    /// a provider built-in like `web_search`, or a `namespace` holding a
    /// nested tool list. Neither has an equivalent on the other wires, and
    /// neither can be honestly bent into one: a built-in has no name, and a
    /// namespace's members are called by a convention this build does not
    /// know. They are left out of [`Self::tools`] and named here, so the
    /// emitter can report the loss rather than invent a definition.
    pub unexpressible_tools: Vec<String>,
    /// Tool selection constraint.
    pub tool_choice: Option<ToolChoice>,
    /// Sampling and length.
    pub params: Params,
}

/// Why generation stopped.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum StopReason {
    /// The model finished.
    #[default]
    EndTurn,
    /// It hit the output cap.
    MaxTokens,
    /// It is waiting for a tool result.
    ///
    /// The highest-consequence value in this enum. Reported as `EndTurn` by
    /// mistake, the agent stops instead of running the call it was just handed.
    ToolUse,
    /// Content filtering.
    Refusal,
    /// A stop sequence matched.
    StopSequence(String),
    /// Still generating, or a value this build does not recognise.
    Unrecognised(String),
}

/// Token accounting, normalised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    /// Prompt tokens that were **not** served from cache.
    pub input: u64,
    /// Prompt tokens served from a cache.
    pub cached_input: u64,
    /// Generated tokens.
    pub output: u64,
    /// Reasoning tokens, where the provider separates them.
    pub reasoning: u64,
}

/// A finished, non-streaming answer.
///
/// [`Block`] is reused from the request side deliberately: an assistant turn in
/// a response is the same thing as an assistant turn in the next request, and
/// the client will replay it as one. Two types would drift.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    /// Provider's id for the message.
    pub id: String,
    /// What the model produced.
    pub blocks: Vec<Block>,
    /// Why it stopped.
    pub stop: StopReason,
    /// Token accounting.
    pub usage: Usage,
}

/// One step of a streamed answer, wire-independent.
///
/// Tool calls appear here **complete**. Chat Completions streams arguments as
/// fragments of a JSON string, Anthropic's `tool_use` carries a parsed object,
/// and Responses emits typed argument deltas; there is nothing any target can
/// emit until the arguments are whole, so the buffering happens once, in the
/// stream driver, rather than in each of six state machines.
#[derive(Debug, Clone, PartialEq)]
pub enum Delta {
    /// The answer has begun.
    Start {
        /// Provider's id, where it gave one.
        id: String,
        /// Model to report to the client — the one it asked for.
        model: String,
    },
    /// Prose, incremental.
    Text(String),
    /// Reasoning summary text, incremental.
    ReasoningText(String),
    /// A complete tool call.
    ToolCall {
        /// Position among this turn's calls.
        index: usize,
        /// Identifier, in its original namespace.
        id: ToolCallId,
        /// Tool name.
        name: String,
        /// Arguments as a JSON string, exactly as accumulated.
        arguments: String,
    },
    /// The answer is finished.
    Stop {
        /// Why.
        reason: StopReason,
        /// Final accounting.
        usage: Usage,
    },
}

/// What an emitter could not carry into its target.
///
/// Returned by emitters rather than accumulated during parsing, because the
/// answer depends entirely on which target is being emitted to. Nothing in here
/// is a decision: the caller decides whether a loss is tolerable (reasoning
/// state on a foreign provider: yes) or disqualifying (an unrecognised block:
/// no).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dropped {
    /// Provider-private reasoning payloads left behind. The summary text, where
    /// there was any, still crossed.
    pub reasoning_blocks: usize,
    /// `cache_control` breakpoints. Only Anthropic has them; the others cache
    /// automatically or not at all, so there is nothing to map onto.
    pub cache_breakpoints: usize,
    /// Image blocks the target cannot accept. The capability gate refuses this
    /// route when images are present, so a non-zero count here is a bug.
    pub images: usize,
    /// Tool declarations the target cannot express, by their source `type`.
    ///
    /// Unlike [`Self::unknown_blocks`] this does not disqualify the route: the
    /// model simply does not get those tools. Reported because the alternative
    /// — what this build used to do — was to emit a function named `""` for a
    /// built-in, which every OpenAI-compatible server rejects outright.
    pub tools: Vec<String>,
    /// Content types this build does not recognise, by name.
    ///
    /// A non-empty list makes the **route** ineligible rather than degrading the
    /// request: we cannot tell whether an unrecognised block was load-bearing,
    /// and the native lane handles it perfectly.
    pub unknown_blocks: Vec<String>,
}

impl Dropped {
    /// Whether anything was lost.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Record an unrecognised block, once per kind.
    pub fn note_unknown(&mut self, kind: &str) {
        if !self.unknown_blocks.iter().any(|seen| seen == kind) {
            self.unknown_blocks.push(kind.to_string());
        }
    }
}

/// Flatten a value that is a string, a list of text-bearing blocks, or neither.
///
/// Every wire has at least one field shaped this way — Anthropic's `system` and
/// tool-result content, Responses' `content` arrays, Chat Completions'
/// multi-part user content — so the rule lives here rather than three times.
#[must_use]
pub fn flatten_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(text) => Some(text.clone()),
                other => other
                    .get("text")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_flattens_from_every_shape_a_wire_uses() {
        assert_eq!(flatten_text(Some(&json!("plain"))), "plain");
        assert_eq!(
            flatten_text(Some(&json!([{"type": "text", "text": "a"}, {"text": "b"}]))),
            "a\nb"
        );
        assert_eq!(flatten_text(Some(&json!(["a", "b"]))), "a\nb");
        assert_eq!(flatten_text(None), "");
        assert_eq!(flatten_text(Some(&Value::Null)), "");
    }

    /// A block with no text at all must not vanish into an empty string that
    /// reads like the model said nothing.
    #[test]
    fn a_shape_with_no_text_is_kept_as_json_rather_than_erased() {
        assert_eq!(flatten_text(Some(&json!({"n": 1}))), r#"{"n":1}"#);
    }

    #[test]
    fn an_unknown_kind_is_recorded_once_however_often_it_appears() {
        let mut dropped = Dropped::default();
        dropped.note_unknown("document");
        dropped.note_unknown("document");
        dropped.note_unknown("search_result");
        assert_eq!(dropped.unknown_blocks, ["document", "search_result"]);
        assert!(!dropped.is_empty());
    }
}
