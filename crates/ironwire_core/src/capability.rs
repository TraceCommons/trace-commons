//! The capability gate.
//!
//! IronWire routes a request to a backend **only if that backend can preserve
//! the request's semantics**. Anything less is a refusal, not a downgrade
//! (`docs/ironwire/DESIGN.md` §2). This module is where that rule is enforced, and it is
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
/// [`RequestRequirements::mid_tool_loop`] — see `docs/ironwire/PROTOCOL.md` §6.
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
    /// The prompt's size in tokens, estimated from the body's bytes. Not the
    /// whole requirement: [`eligible`] adds a margin for the estimate and the
    /// answer's room ([`Self::output_tokens`]).
    pub min_context_tokens: u32,
    /// The longest answer the client asked for: Anthropic's `max_tokens`,
    /// Chat Completions' `max_completion_tokens` or `max_tokens`, Responses'
    /// `max_output_tokens`. `None` when the body names none.
    ///
    /// Read from the client's body, because it is the request's own
    /// allowance that shares the window with the prompt, not the model's
    /// largest.
    #[serde(default)]
    pub output_tokens: Option<u32>,
    /// The conversation is **mid tool loop**: the client is replaying tool
    /// results and expects the model to continue an exchange already in flight.
    ///
    /// This is the cross-family gate. Switching families here means the next
    /// assistant turn comes back without the provider-private reasoning state
    /// its `tool_use` block is expected to carry, and replaying that history to
    /// the original family then risks a hard rejection. At a turn boundary
    /// there is no in-flight state to be missing, so the switch is clean.
    /// See `docs/ironwire/PROTOCOL.md` §6.
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
    /// No origin is recorded for this replay, and more than one account could
    /// have minted it. Set by policy, never trusted from the wire.
    ///
    /// Refuses every route, translated ones included. A translated route
    /// drops the reasoning safely, which is why a *known* origin that cannot
    /// serve may cross families at a turn boundary; but with the origin
    /// unknown, the account that minted it may be right there, and moving the
    /// conversation to another model and another bill would be choosing for
    /// the user on a guess. Starting a new session is the user's call.
    #[serde(skip)]
    pub reasoning_origin_ambiguous: bool,
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
    /// Responses `tools` entry `type`s a custom backend rejects outright.
    ///
    /// Built-in backends leave this empty. A driver may declare an
    /// unsupported type from provider evidence; the proxy logs any removal
    /// because dropping a tool the provider accepts is a capability loss.
    #[serde(default)]
    pub unsupported_responses_tools: Vec<String>,
    /// Whether [`Self::context_tokens`] is the window the provider stated for
    /// the model being served, rather than the backend's compiled-in default.
    ///
    /// Only a stated window is checked with room for the answer and a margin
    /// for the estimate (see [`eligible`]). Set when sizing a request, never
    /// configured, so it is not serialized.
    #[serde(skip)]
    pub context_from_provider: bool,
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
            context_from_provider: false,
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
/// fallback has hysteresis (`docs/ironwire/CRITIQUE.md` §1).
pub const CACHE_SACRIFICE_THRESHOLD_TOKENS: u32 = 4_000;

/// The answer's room when the client names no output limit, in tokens, on a
/// provider-stated window.
///
/// Routing headroom, not a claim about any provider: with no limit named, a
/// provider may answer up to the rest of the window, so any finite figure is a
/// floor. 8,192 is what the translated lane already writes as `max_tokens`
/// when it must send one and the client named none, so a route admitted here
/// is not refused for the number IronWire itself put on the wire.
pub const DEFAULT_OUTPUT_TOKENS: u32 = 8_192;

/// The prompt estimate is raised by `1 / ESTIMATE_MARGIN_DIVISOR` of itself.
///
/// The estimate is body bytes / 4, and code-heavy prompts run under 4 bytes
/// per token, so it comes out low exactly on the large coding turns that
/// approach a window. One eighth (12.5%) covers about 3.55 bytes per token.
const ESTIMATE_MARGIN_DIVISOR: u64 = 8;

/// Tokens a request needs from `caps`' window.
///
/// On a window the provider stated for the model, the prompt estimate with
/// its margin plus room for the answer the request asked for: the provider
/// refuses a prompt plus `max_tokens` over its stated context, and that
/// window is the provider's exact edge. Any stated window at or above the
/// backend's default refuses nothing the default admitted.
///
/// On the backend's compiled-in default, the bare estimate, as before
/// providers' windows were read. The estimate also counts base64 images and
/// JSON at four bytes a token, so it can run high, and a refusal here is
/// worse than the provider's own: when every backend refuses, the client gets
/// a 503, which Claude Code retries, not the provider's "prompt is too long",
/// on which it compacts. A reserve there would refuse a single-backend
/// session turns the provider would have served, with nowhere else to go.
///
/// In `u64`, so no sum wraps.
fn context_needed(req: &RequestRequirements, caps: &Capabilities) -> u64 {
    let prompt = u64::from(req.min_context_tokens);
    if !caps.context_from_provider {
        return prompt;
    }
    let answer = u64::from(req.output_tokens.unwrap_or(DEFAULT_OUTPUT_TOKENS));
    prompt + prompt / ESTIMATE_MARGIN_DIVISOR + answer
}

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
    if req.encrypted_reasoning && req.reasoning_origin_ambiguous {
        return Err(Ineligible::ReasoningBoundToAccount);
    }
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
    if context_needed(req, caps) > u64::from(caps.context_tokens) {
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
            context_from_provider: false,
        }
    }

    /// What Claude Code actually sends between turns: thinking enabled, signed
    /// blocks in history, tools declared, a big cached prefix.
    fn claude_code_turn() -> RequestRequirements {
        RequestRequirements {
            admission_backend: None,
            reasoning_backend: None,
            reasoning_origin_ambiguous: false,
            tools: true,
            parallel_tool_calls: false,
            images: false,
            reasoning: ReasoningNeed::LoadBearing,
            prompt_cache: true,
            cached_prefix_tokens: 120_000,
            structured_output: false,
            min_context_tokens: 40_000,
            output_tokens: Some(32_000),
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
            context_from_provider: false,
        }
    }

    #[test]
    fn a_replay_of_ambiguous_origin_is_refused_on_every_wire_including_translated() {
        let req = RequestRequirements {
            encrypted_reasoning: true,
            reasoning_origin_ambiguous: true,
            ..Default::default()
        };
        for (cross, backend) in [
            (false, Some(("codex-sub", Protocol::OpenAiResponses))),
            (true, Some(("nearai", Protocol::OpenAiChat))),
            (true, Some(("anthropic-key", Protocol::AnthropicMessages))),
            (true, None),
        ] {
            assert_eq!(
                eligible(&req, &full_caps(), cross, backend),
                Err(Ineligible::ReasoningBoundToAccount),
                "{backend:?}"
            );
        }
        // Without encrypted state there is nothing to bind, ambiguous or not.
        let plain = RequestRequirements {
            reasoning_origin_ambiguous: true,
            ..Default::default()
        };
        assert_eq!(
            eligible(
                &plain,
                &full_caps(),
                true,
                Some(("nearai", Protocol::OpenAiChat))
            ),
            Ok(())
        );
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
    fn the_requested_answer_must_fit_beside_the_prompt() {
        let caps = Capabilities {
            context_tokens: 1_048_576,
            context_from_provider: true,
            ..full_caps()
        };
        let req = RequestRequirements {
            min_context_tokens: 900_000,
            output_tokens: Some(128_000),
            ..Default::default()
        };
        assert_eq!(
            eligible(&req, &caps, false, None),
            Err(Ineligible::ContextTooSmall)
        );
        let req = RequestRequirements {
            output_tokens: Some(16),
            ..req
        };
        assert_eq!(eligible(&req, &caps, false, None), Ok(()));
    }

    #[test]
    fn a_request_naming_no_answer_size_still_reserves_room_for_one() {
        let caps = Capabilities {
            context_tokens: 32_000,
            context_from_provider: true,
            ..full_caps()
        };
        // Fits on its own, with the margin; not with the default answer.
        let req = RequestRequirements {
            min_context_tokens: 24_000,
            output_tokens: None,
            ..Default::default()
        };
        assert_eq!(
            eligible(&req, &caps, false, None),
            Err(Ineligible::ContextTooSmall)
        );
        let req = RequestRequirements {
            min_context_tokens: 20_000,
            ..req
        };
        assert_eq!(eligible(&req, &caps, false, None), Ok(()));
    }

    #[test]
    fn an_estimate_short_of_the_window_by_less_than_its_margin_is_refused() {
        let caps = Capabilities {
            context_tokens: 100_000,
            context_from_provider: true,
            ..full_caps()
        };
        let req = RequestRequirements {
            min_context_tokens: 90_000,
            output_tokens: Some(1),
            ..Default::default()
        };
        assert_eq!(
            eligible(&req, &caps, false, None),
            Err(Ineligible::ContextTooSmall)
        );
    }

    /// Where the reserve and margin put the edge on a provider-stated 200K
    /// window for a turn asking for 32,000 tokens: about
    /// (200,000 - 32,000) * 8/9.
    #[test]
    fn a_stated_200k_window_with_a_32k_answer_admits_a_149k_token_estimate() {
        let caps = Capabilities {
            context_tokens: 200_000,
            context_from_provider: true,
            ..full_caps()
        };
        let at = |min_context_tokens| RequestRequirements {
            min_context_tokens,
            output_tokens: Some(32_000),
            ..Default::default()
        };
        assert_eq!(eligible(&at(149_334), &caps, false, None), Ok(()));
        assert_eq!(
            eligible(&at(149_335), &caps, false, None),
            Err(Ineligible::ContextTooSmall)
        );
    }

    /// The backend's compiled-in window is checked against the bare estimate,
    /// as before providers' windows were read: a refusal there sends a 503 the
    /// client retries, where the provider's own answer would make it compact.
    #[test]
    fn a_default_window_still_admits_what_it_admitted_before_stated_windows() {
        let caps = Capabilities {
            context_tokens: 200_000,
            ..full_caps()
        };
        let req = RequestRequirements {
            min_context_tokens: 190_000,
            output_tokens: Some(32_000),
            ..Default::default()
        };
        assert_eq!(eligible(&req, &caps, false, None), Ok(()));
        let req = RequestRequirements {
            min_context_tokens: 200_001,
            ..req
        };
        assert_eq!(
            eligible(&req, &caps, false, None),
            Err(Ineligible::ContextTooSmall)
        );
    }

    #[test]
    fn the_largest_requirements_do_not_wrap_into_a_fit() {
        let req = RequestRequirements {
            min_context_tokens: u32::MAX,
            output_tokens: Some(u32::MAX),
            ..Default::default()
        };
        let caps = Capabilities {
            context_from_provider: true,
            ..full_caps()
        };
        assert_eq!(
            eligible(&req, &caps, false, None),
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
