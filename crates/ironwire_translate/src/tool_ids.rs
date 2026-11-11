//! Tool-call identity across a wire boundary.
//!
//! The client replays whatever id we returned, forever — so an id we mint on
//! the way back from a foreign provider has to be recognisable, valid in the
//! client's namespace, and reversible when the client sends it out again.
//!
//! The obvious design is a per-conversation map. We deliberately do not use
//! one: a map is state that can be lost (daemon restart, eviction), and losing
//! it produces *invalid ids* — a failure the user cannot diagnose and we cannot
//! recover from. A reversible encoding has no such failure mode, costs nothing,
//! survives restarts, and reads plainly in a log.
//!
//! The rule is chosen by the **target** protocol, not hardcoded to one of them:
//!
//! | Target | Rule |
//! |---|---|
//! | `anthropic.messages` | prefix `toolu_xw_`, because Anthropic wants the `toolu_` shape |
//! | `openai.responses`, `openai.chat` | pass through — both accept arbitrary strings |

use ironwire_core::protocol::Protocol;

use crate::ir::ToolCallId;

/// Marker for an id IronWire minted while translating a foreign tool call.
///
/// `toolu_` keeps it valid in Anthropic's namespace; `xw_` distinguishes it
/// from an id Anthropic itself minted, so the reverse direction knows whether
/// to strip anything.
const MINTED_PREFIX: &str = "toolu_xw_";

/// Encode an id for a client speaking `target`.
///
/// An id already native to the target goes through untouched — that is the
/// round trip, and it is the case the provenance on [`ToolCallId`] exists to
/// recognise. Anything else gets the target's namespace rule.
///
/// The encoding is applied **unconditionally** to a foreign id, including one
/// that already begins with the marker. An earlier version skipped the prefix
/// in that case, reasoning that the id was "already ours" — what that guard
/// actually did was break the round trip for any provider that happened to mint
/// an id starting with `toolu_xw_`: the reverse direction stripped a prefix
/// nobody had added, and the client's replayed id no longer matched the call it
/// belonged to.
#[must_use]
pub fn encode(id: &ToolCallId, target: Protocol) -> String {
    if id.is_native_to(target) {
        return id.as_str().to_string();
    }
    match target {
        Protocol::AnthropicMessages => format!("{MINTED_PREFIX}{}", id.as_str()),
        Protocol::OpenAiResponses | Protocol::OpenAiChat => id.as_str().to_string(),
    }
}

/// Recover the original id from one a client speaking `source` replayed.
///
/// The marker is the whole signal: an id carrying it is one IronWire minted
/// from a foreign call, so the prefix comes off and the provider it belonged to
/// is no longer known (nor needed — every wire but Anthropic takes any string).
/// An id without the marker is native to `source`, which is what lets it go
/// home unchanged.
#[must_use]
pub fn decode(id: &str, source: Protocol) -> ToolCallId {
    if source == Protocol::AnthropicMessages
        && let Some(original) = id.strip_prefix(MINTED_PREFIX)
    {
        return ToolCallId::foreign(original);
    }
    ToolCallId::native(source, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY: [Protocol; 3] = [
        Protocol::AnthropicMessages,
        Protocol::OpenAiResponses,
        Protocol::OpenAiChat,
    ];

    #[test]
    fn a_minted_id_round_trips_on_every_wire() {
        for target in EVERY {
            let original = ToolCallId::from("call_abc123XYZ");
            let encoded = encode(&original, target);
            assert_eq!(decode(&encoded, target), original, "{target} lost the id");
        }
    }

    /// The bug the provenance field exists to fix. An id native to the target
    /// must go home **unchanged**: prefixing it produces `toolu_xw_toolu_01ABC`,
    /// which is a call id the client has never seen and cannot match to
    /// anything it is holding.
    #[test]
    fn an_id_going_back_to_its_own_wire_is_not_re_encoded() {
        for wire in EVERY {
            for raw in ["toolu_01ABC", "call_1", "fc_9"] {
                let decoded = decode(raw, wire);
                assert_eq!(
                    encode(&decoded, wire),
                    raw,
                    "{wire} re-encoded one of its own ids"
                );
            }
        }
    }

    /// And an id IronWire minted still reverses, on whichever wire it comes
    /// back through.
    #[test]
    fn an_id_we_minted_reverses_wherever_it_is_replayed() {
        let foreign = ToolCallId::native(Protocol::OpenAiChat, "call_1");
        let handed_out = encode(&foreign, Protocol::AnthropicMessages);
        assert_eq!(handed_out, "toolu_xw_call_1");

        let replayed = decode(&handed_out, Protocol::AnthropicMessages);
        assert_eq!(replayed.as_str(), "call_1");
        // Back to the provider that minted it, in its own namespace.
        assert_eq!(encode(&replayed, Protocol::OpenAiChat), "call_1");
        // And back out to the client again, unchanged.
        assert_eq!(
            encode(&replayed, Protocol::AnthropicMessages),
            "toolu_xw_call_1"
        );
    }

    #[test]
    fn an_anthropic_client_gets_an_id_its_own_api_accepts() {
        // The client replays whatever we return, forever. An id Anthropic would
        // reject is a conversation that cannot continue.
        for id in ["call_1", "", "日本語"] {
            let encoded = encode(&ToolCallId::from(id), Protocol::AnthropicMessages);
            assert!(encoded.starts_with("toolu_"), "{id:?} → {encoded}");
        }
    }

    #[test]
    fn an_openai_wire_needs_no_encoding_at_all() {
        // Both accept arbitrary strings, so touching the id would be inventing a
        // difference and then having to undo it.
        for target in [Protocol::OpenAiResponses, Protocol::OpenAiChat] {
            assert_eq!(
                encode(&ToolCallId::from("toolu_01ABC"), target),
                "toolu_01ABC"
            );
            assert_eq!(decode("toolu_01ABC", target).as_str(), "toolu_01ABC");
        }
    }

    #[test]
    fn an_anthropic_native_id_passes_through_untouched() {
        assert_eq!(
            decode("toolu_01ABCdef", Protocol::AnthropicMessages).as_str(),
            "toolu_01ABCdef"
        );
    }

    #[test]
    fn the_round_trip_holds_for_ids_that_look_like_ours() {
        // The case that motivated removing the "already ours" guard. A foreign
        // provider minting `toolu_xw_call_1` used to come back as `call_1`, so
        // the client's replayed id no longer matched the call it belonged to —
        // a mismatch neither side can diagnose.
        for id in ["toolu_xw_call_1", "toolu_xw_", "toolu_xw_toolu_xw_nested"] {
            let original = ToolCallId::native(Protocol::OpenAiChat, id);
            let encoded = encode(&original, Protocol::AnthropicMessages);
            assert_eq!(decode(&encoded, Protocol::AnthropicMessages), original);
        }
    }

    #[test]
    fn one_translation_adds_exactly_one_prefix() {
        // A bijection, not an idempotent function. The distinction is the fix:
        // idempotence would mean `encode` could not tell a foreign id that looks
        // like ours from one that is.
        assert_eq!(
            encode(&ToolCallId::from("call_1"), Protocol::AnthropicMessages),
            "toolu_xw_call_1"
        );
        assert_eq!(
            encode(
                &ToolCallId::native(Protocol::OpenAiChat, "toolu_xw_call_1"),
                Protocol::AnthropicMessages
            ),
            "toolu_xw_toolu_xw_call_1"
        );
    }

    #[test]
    fn odd_ids_survive() {
        let long = "x".repeat(2000);
        for id in [
            "",
            "call_-_/=+",
            "toolu_",
            "toolu_xw",
            "TOOLU_XW_upper",
            "call with spaces",
            "日本語の識別子",
            long.as_str(),
        ] {
            for target in EVERY {
                let original = ToolCallId::from(id);
                let encoded = encode(&original, target);
                assert_eq!(decode(&encoded, target), original, "{target} lost {id:?}");
            }
        }
    }

    #[test]
    fn the_mapping_needs_no_state_and_so_survives_a_restart() {
        // The property that motivates the encoding: two independent processes
        // agree without sharing anything.
        let id = ToolCallId::from("call_9");
        let encoded = encode(&id, Protocol::AnthropicMessages);
        assert_eq!(decode(&encoded, Protocol::AnthropicMessages), id);
    }
}
