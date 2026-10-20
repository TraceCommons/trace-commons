//! The capability gate.
//!
//! IronWire routes a request to a backend **only if that backend can preserve
//! the request's semantics**. Anything less is a refusal, not a downgrade
//! (`docs/DESIGN.md` §2). This module is where that rule is enforced, and it is
//! deliberately the smallest, most testable thing in the codebase: everything
//! about routing quality is a heuristic, but everything here should be a hard
//! fact about what the wire can carry.
//!
//! Keeping that distinction honest is the whole job. A gate that refuses a
//! route which would merely be *worse* is not caution — it is a bug that
//! silently deletes capacity the user is paying for. Every rule below is either
//! "the agent breaks" or "the user loses more money than the route saves";
//! anything that is only a quality loss belongs in the route's reason string,
//! not here.

use serde::{Deserialize, Serialize};

use crate::protocol::{Protocol, Wires};

/// How much the request depends on model reasoning state.
///
/// [`ReasoningNeed::LoadBearing`] means the history carries *signed* Anthropic
/// thinking blocks or *encrypted* OpenAI reasoning items. That state cannot be
/// reproduced by another provider — but it does not have to be: a foreign
/// provider never validates it, and the API that minted it **drops** such
/// blocks from the prompt rather than rejecting them. So this is a *quality*
/// signal (reasoning continuity is lost across a family change), not an
/// eligibility one. The eligibility rule lives in
/// [`RequestRequirements::mid_tool_loop`] — see `docs/PROTOCOL.md` §6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningNeed {
    /// No reasoning requested.
    #[default]
    None,
    /// Reasoning requested for this turn, but nothing in the history depends
    /// on provider-private state.
    Requested,
    /// The conversation history contains signed or encrypted provider
    /// reasoning state that will be replayed. Crossing families drops it and
    /// loses continuity; it does not make the route illegal.
    LoadBearing,
}

/// What a request needs a backend to preserve. Built by [`crate::peek`] from a
/// bounded scan of the body — never by re-serialising it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RequestRequirements {
    /// Explicit admission binds routing to one backend; pins cannot widen it.
    #[serde(default)]
    pub admission_backend: Option<String>,
    /// Tool definitions are present.
    pub tools: bool,
    /// The history contains an assistant turn that issued more than one tool
    /// call at once — so the client genuinely depends on parallel calls, rather
    /// than merely permitting them.
    pub parallel_tool_calls: bool,
    /// Image content blocks are present.
    pub images: bool,
    /// Reasoning dependency; see [`ReasoningNeed`]. Informational.
    pub reasoning: ReasoningNeed,
    /// `cache_control` breakpoints are present.
    pub prompt_cache: bool,
    /// Approximate size of the cacheable prefix, in tokens. Used to decide
    /// whether losing the cache is survivable.
    pub cached_prefix_tokens: u32,
    /// A strict JSON schema response format is requested.
    pub structured_output: bool,
    /// Minimum usable context window, in tokens.
    pub min_context_tokens: u32,
    /// The conversation is **mid tool loop**: the client is replaying tool
    /// results and expects the model to continue an exchange already in flight.
    ///
    /// This is the cross-family gate. Switching families here means the next
    /// assistant turn comes back without the provider-private reasoning state
    /// its `tool_use` block is expected to carry, and replaying that history to
    /// the original family then risks a hard rejection. At a turn boundary
    /// there is no in-flight state to be missing, so the switch is clean.
    /// See `docs/PROTOCOL.md` §6.
    pub mid_tool_loop: bool,
    /// The history replays a Responses `reasoning` item carrying
    /// `encrypted_content`.
    ///
    /// Narrower than [`ReasoningNeed::LoadBearing`], and kept separate because
    /// it answers a different question. `LoadBearing` is about continuity
    /// across a *family* change, where the foreign provider simply never sees
    /// the state. This is about an *account* change on the same wire: OpenAI
    /// rejects encrypted content minted under another organization
    /// (`invalid_encrypted_content`), so a conversation carrying it can only
    /// be replayed to the ChatGPT account that produced it.
    #[serde(default)]
    pub encrypted_reasoning: bool,
    /// The native backend whose observed credential scope matches the origin
    /// recorded for this replay. Set by policy, never trusted from the wire.
    #[serde(skip)]
    pub reasoning_backend: Option<String>,
}

/// What a backend+model can preserve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Wires this backend speaks natively, preferred first.
    pub wires: Wires,
    /// Tool calling.
    pub tools: bool,
    /// More than one tool call per assistant turn.
    pub parallel_tool_calls: bool,
    /// Image inputs.
    pub images: bool,
    /// Any form of reasoning/thinking. Informational: a model without it still
    /// answers, just without extended reasoning.
    pub reasoning: bool,
    /// Prompt caching with explicit breakpoints.
    pub prompt_cache: bool,
    /// Strict structured output.
    pub structured_output: bool,
    /// Context window in tokens.
    pub context_tokens: u32,
    /// Responses `tools` entry `type`s this backend rejects outright.
    ///
    /// Responses lets a client declare tool entries that are not functions,
    /// and providers do not agree on which of them exist. OpenAI accepts
    /// everything Codex sends; NEAR AI answers a `namespace` entry with a
    /// deserialisation error naming the variants it knows, and refuses the
    /// whole request over it — so a Codex turn with any MCP server connected
    /// could not reach NEAR AI at all.
    ///
    /// Named per backend rather than filtered to some lowest common set,
    /// because dropping a tool the provider would have accepted is a real
    /// capability loss. Empty for every backend not *known* to reject
    /// something: this list is evidence, not caution.
    #[serde(default)]
    pub unsupported_responses_tools: Vec<String>,
}

impl Capabilities {
    /// A conservative unknown-backend baseline: serial tool calls, no images,
    /// no reasoning, no cache, 128k context. Used only for user-configured
    /// OpenAI-compatible endpoints we have never seen.
    #[must_use]
    pub fn conservative(protocol: Protocol) -> Self {
        Self {
            wires: Wires::only(protocol),
            tools: true,
            parallel_tool_calls: false,
            images: false,
            reasoning: false,
            prompt_cache: false,
            structured_output: false,
            context_tokens: 128_000,
            unsupported_responses_tools: Vec::new(),
        }
    }
}

/// Why a route was refused. Carried into logs, the control API, and the rung-3
/// announcement so a user can always find out *why* their agent is not on the
/// model they expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ineligible {
    /// Candidate cannot preserve the registered admission route.
    AdmissionRouteMismatch,
    /// The conversation is mid tool loop, so a family change would leave the
    /// next assistant turn missing the reasoning state its `tool_use` is
    /// expected to carry. Eligible again at the next turn boundary.
    MidToolLoop,
    /// Losing a large warm prefix would cost more than the route saves.
    WouldDiscardLargePromptCache,
    /// Request uses tools; backend has none. The agent would hang waiting for
    /// a call that can never come.
    ToolsUnsupported,
    /// The client already issues several tool calls per turn; a serial backend
    /// would silently drop all but one.
    ParallelToolsUnsupported,
    /// Request carries images; backend is text-only and simply cannot see them.
    ImagesUnsupported,
    /// Request needs a strict schema; backend cannot guarantee one, and the
    /// client will fail to parse the answer.
    StructuredOutputUnsupported,
    /// Prompt does not fit.
    ContextTooSmall,
    /// `privacy.mode = "full"` and this backend is not in `trusted_backends`.
    ///
    /// The one variant here that is not a statement about what the *wire* can
    /// carry. Everything else in this gate answers "would the agent break";
    /// this answers "did the user forbid it", and it belongs here for the same
    /// reason: eligibility is the only place a constraint cannot be sorted
    /// around. Expressed as a preference it would be one exhausted backend away
    /// from being silently ignored.
    NotTrustedUnderFullPrivacy,
    /// The conversation replays encrypted reasoning, which is bound to the
    /// ChatGPT account that produced it, and this is a different account.
    ///
    /// Like [`Self::MidToolLoop`], a fact about where the conversation has
    /// been rather than about this backend: it stays on the account it is on,
    /// or crosses to another family at a turn boundary. A new conversation is
    /// free to start anywhere.
    ReasoningBoundToAccount,
}

/// The tokens-of-warm-cache threshold above which discarding the cache makes a
/// cross-family route a net loss.
///
/// Below this, a cold start is cheap enough that the route is worth taking.
/// Above it, re-priming costs more (in money and in latency) than staying put
/// and waiting out a short rate-limit window — which is the whole reason
/// fallback has hysteresis (`docs/CRITIQUE.md` §1).
pub const CACHE_SACRIFICE_THRESHOLD_TOKENS: u32 = 4_000;

/// Decide whether `caps` can serve `req` without breaking it.
///
/// `cross_family` is true when taking this route means translating between API
/// families. Two rules apply only then: the mid-tool-loop rule, and the
/// prompt-cache economics.
///
/// # Errors
///
/// Returns [`Ineligible`] describing the first unmet requirement.
pub fn eligible(
    req: &RequestRequirements,
    caps: &Capabilities,
    cross_family: bool,
    backend: Option<(&str, crate::protocol::Protocol)>,
) -> Result<(), Ineligible> {
    if req.encrypted_reasoning
        && let Some((id, Protocol::OpenAiResponses)) = backend
        && req.reasoning_backend.as_deref() != Some(id)
    {
        return Err(Ineligible::ReasoningBoundToAccount);
    }
    if req
        .admission_backend
        .as_deref()
        .is_some_and(|required| Some((required, crate::protocol::Protocol::OpenAiChat)) != backend)
    {
        return Err(Ineligible::AdmissionRouteMismatch);
    }
    // The one cross-family rule that is about correctness rather than cost:
    // switch families at a turn boundary, never mid tool loop.
    if cross_family && req.mid_tool_loop {
        return Err(Ineligible::MidToolLoop);
    }
    if req.tools && !caps.tools {
        return Err(Ineligible::ToolsUnsupported);
    }
    if req.parallel_tool_calls && !caps.parallel_tool_calls {
        return Err(Ineligible::ParallelToolsUnsupported);
    }
    if req.images && !caps.images {
        return Err(Ineligible::ImagesUnsupported);
    }
    if req.structured_output && !caps.structured_output {
        return Err(Ineligible::StructuredOutputUnsupported);
    }
    if req.min_context_tokens > caps.context_tokens {
        return Err(Ineligible::ContextTooSmall);
    }
    // Economic rather than semantic, but it belongs in the same gate: a route
    // that "works" while costing the user 10x is not a route we take silently.
    if cross_family
        && req.prompt_cache
        && !caps.prompt_cache
        && req.cached_prefix_tokens > CACHE_SACRIFICE_THRESHOLD_TOKENS
    {
        return Err(Ineligible::WouldDiscardLargePromptCache);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_caps() -> Capabilities {
        Capabilities {
            wires: Wires::only(Protocol::AnthropicMessages),
            tools: true,
            parallel_tool_calls: true,
            images: true,
            reasoning: true,
            prompt_cache: true,
            structured_output: true,
            context_tokens: 200_000,
            unsupported_responses_tools: Vec::new(),
        }
    }

    /// What Claude Code actually sends between turns: thinking enabled, signed
    /// blocks in history, tools declared, a big cached prefix.
    fn claude_code_turn() -> RequestRequirements {
        RequestRequirements {
            admission_backend: None,
            reasoning_backend: None,
            tools: true,
            parallel_tool_calls: false,
            images: false,
            reasoning: ReasoningNeed::LoadBearing,
            prompt_cache: true,
            cached_prefix_tokens: 120_000,
            structured_output: false,
            min_context_tokens: 40_000,
            mid_tool_loop: false,
            encrypted_reasoning: false,
        }
    }

    fn nearai_caps() -> Capabilities {
        Capabilities {
            wires: Wires::only(Protocol::OpenAiChat),
            tools: true,
            parallel_tool_calls: true,
            images: false,
            reasoning: false,
            prompt_cache: false,
            structured_output: false,
            context_tokens: 128_000,
            unsupported_responses_tools: Vec::new(),
        }
    }

    #[test]
    fn admission_requires_exact_backend_and_actual_outbound_wire() {
        let req = RequestRequirements {
            admission_backend: Some("bound".into()),
            ..Default::default()
        };
        let caps = Capabilities::conservative(Protocol::OpenAiChat);
        assert_eq!(
            eligible(&req, &caps, false, Some(("bound", Protocol::OpenAiChat))),
            Ok(())
        );
        for route in [
            None,
            Some(("other", Protocol::OpenAiChat)),
            Some(("bound", Protocol::OpenAiResponses)),
        ] {
            assert_eq!(
                eligible(&req, &caps, false, route),
                Err(Ineligible::AdmissionRouteMismatch)
            );
        }
    }

    #[test]
    fn signed_thinking_alone_does_not_block_a_family_change() {
        // The correction that motivated this gate's rewrite: a foreign provider
        // never validates an Anthropic signature, and Anthropic drops rather
        // than rejects foreign reasoning state. Refusing here deleted a whole
        // capacity pool for no reason.
        let mut req = claude_code_turn();
        req.prompt_cache = false; // isolate the reasoning question
        assert_eq!(eligible(&req, &nearai_caps(), true, None), Ok(()));
    }

    #[test]
    fn a_family_change_is_refused_mid_tool_loop() {
        let mut req = claude_code_turn();
        req.prompt_cache = false;
        req.mid_tool_loop = true;
        assert_eq!(
            eligible(&req, &nearai_caps(), true, None),
            Err(Ineligible::MidToolLoop)
        );
    }

    #[test]
    fn the_same_conversation_becomes_eligible_at_the_next_turn_boundary() {
        // This is the whole point of the rule: it defers a switch, it does not
        // permanently disqualify the conversation.
        let mut req = claude_code_turn();
        req.prompt_cache = false;
        req.mid_tool_loop = true;
        assert!(eligible(&req, &nearai_caps(), true, None).is_err());
        req.mid_tool_loop = false;
        assert_eq!(eligible(&req, &nearai_caps(), true, None), Ok(()));
    }

    #[test]
    fn mid_tool_loop_is_fine_within_a_family() {
        // Rung 2 replays the same history to the same wire format; nothing is
        // missing, so there is nothing to refuse.
        let mut req = claude_code_turn();
        req.mid_tool_loop = true;
        assert_eq!(eligible(&req, &full_caps(), false, None), Ok(()));
    }

    #[test]
    fn requesting_thinking_does_not_require_a_thinking_backend() {
        // A model without extended reasoning still answers; that is a quality
        // loss, not a broken request.
        let req = RequestRequirements {
            reasoning: ReasoningNeed::Requested,
            ..Default::default()
        };
        assert_eq!(eligible(&req, &nearai_caps(), true, None), Ok(()));
    }

    #[test]
    fn a_serial_backend_is_refused_only_once_parallel_calls_are_actually_used() {
        // Permitting parallel calls is the Anthropic default; depending on them
        // is what a serial backend would break.
        let serial = Capabilities {
            parallel_tool_calls: false,
            ..nearai_caps()
        };
        let unused = RequestRequirements {
            tools: true,
            parallel_tool_calls: false,
            ..Default::default()
        };
        assert_eq!(eligible(&unused, &serial, true, None), Ok(()));

        let used = RequestRequirements {
            parallel_tool_calls: true,
            ..unused
        };
        assert_eq!(
            eligible(&used, &serial, true, None),
            Err(Ineligible::ParallelToolsUnsupported)
        );
    }

    #[test]
    fn large_warm_cache_blocks_cross_family_but_not_same_family() {
        let req = RequestRequirements {
            prompt_cache: true,
            cached_prefix_tokens: 120_000,
            ..Default::default()
        };
        let no_cache = Capabilities {
            prompt_cache: false,
            ..full_caps()
        };
        assert_eq!(
            eligible(&req, &no_cache, true, None),
            Err(Ineligible::WouldDiscardLargePromptCache)
        );
        assert_eq!(eligible(&req, &no_cache, false, None), Ok(()));
    }

    #[test]
    fn small_cache_is_worth_sacrificing() {
        let req = RequestRequirements {
            prompt_cache: true,
            cached_prefix_tokens: 500,
            ..Default::default()
        };
        let no_cache = Capabilities {
            prompt_cache: false,
            ..full_caps()
        };
        assert_eq!(eligible(&req, &no_cache, true, None), Ok(()));
    }

    #[test]
    fn missing_modalities_are_refusals_not_downgrades() {
        let req = RequestRequirements {
            images: true,
            ..Default::default()
        };
        let text_only = Capabilities {
            images: false,
            ..full_caps()
        };
        assert_eq!(
            eligible(&req, &text_only, false, None),
            Err(Ineligible::ImagesUnsupported)
        );
    }

    #[test]
    fn tools_without_a_tool_capable_backend_would_hang_the_agent() {
        let req = RequestRequirements {
            tools: true,
            ..Default::default()
        };
        let toolless = Capabilities {
            tools: false,
            ..full_caps()
        };
        assert_eq!(
            eligible(&req, &toolless, true, None),
            Err(Ineligible::ToolsUnsupported)
        );
    }

    #[test]
    fn context_must_actually_fit() {
        let req = RequestRequirements {
            min_context_tokens: 400_000,
            ..Default::default()
        };
        assert_eq!(
            eligible(&req, &full_caps(), false, None),
            Err(Ineligible::ContextTooSmall)
        );
    }

    #[test]
    fn an_empty_request_fits_a_conservative_backend() {
        let req = RequestRequirements::default();
        let caps = Capabilities::conservative(Protocol::OpenAiChat);
        assert_eq!(eligible(&req, &caps, true, None), Ok(()));
    }
}
