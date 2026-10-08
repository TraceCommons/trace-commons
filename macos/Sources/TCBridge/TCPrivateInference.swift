import CTraceCommons
import Foundation

/// The private-inference surface, across the C ABI.
///
/// Handle-free: every call here describes the build, not a running daemon.
/// The words, the two branch tables and the decision about whether to ask at
/// all all come from `crates/trace-commons-contributor/src/
/// private_inference_copy.rs`, so this shell, the GTK shell and the Windows
/// shell print one set of sentences and make one set of decisions.
///
/// Nothing in this file authors a sentence, and nothing in it decides a
/// tone. A `nil` here means a caught Rust panic, and the caller renders
/// nothing rather than filling the hole in.
public enum TCPrivateInference {
    /// Every fixed word on the offer and the settings card, as JSON.
    ///
    /// One call and not one per string: a per-string export would let this
    /// shell take four sentences and hand-write the fifth, and the one it
    /// would hand-write is the sentence saying that anything else on the
    /// machine can spend the configured accounts while the switch is on.
    public static func copyJSON() -> String? {
        guard let raw = tc_private_inference_copy() else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// The sentence for one `private_inference_state` label.
    ///
    /// An unfamiliar nonempty label reads as unavailable,
    /// which claims nothing. The Rust decides that, not this shell.
    public static func stateLine(state: String) -> String? {
        guard let raw = state.withCString({ tc_private_inference_state_line($0) }) else {
            return nil
        }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// The runtime tile's word for one `private_inference_state` label, as
    /// the core's `runtime_word` picks it: every running label is on, a
    /// stopped one off, and an unreported or unfamiliar label unknown,
    /// never off. Nil only on a caught panic.
    public static func runtimeWord(state: String) -> String? {
        guard let raw = state.withCString({ tc_private_inference_runtime_word($0) }) else {
            return nil
        }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// How firmly that sentence reads, as a raw
    /// `TC_PRIVATE_INFERENCE_TONE_*` value.
    ///
    /// Takes what the sentence takes, so the two stay in step. Never
    /// recover it by reading the sentence: two refusal sentences begin with the
    /// same two words. There is no failure value -- an unknown state, and a
    /// caught panic, both answer the tone that claims nothing.
    public static func stateTone(state: String) -> Int32 {
        state.withCString { tc_private_inference_state_tone($0) }
    }

    /// Where the listener is answering, or the empty string when there is
    /// no port. A `nil` port crosses as `0`, which is the same case.
    public static func servingLine(port: UInt16?) -> String? {
        guard let raw = tc_private_inference_serving_line(Int32(port ?? 0)) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// Whether to put the offer in front of the contributor.
    ///
    /// The branch crosses, not only the words. Three shells each deciding
    /// when to interrupt somebody is three chances to re-ask a contributor
    /// who already said no.
    public static func quitNeedsNotice(on: Bool, state: String) -> Bool {
        state.withCString { tc_private_inference_quit_needs_notice(on ? 1 : 0, $0) != 0 }
    }

    public static func writeConfirmed(requestedOn: Bool?, echoedSeen: Bool?, echoedOn: Bool?) -> Bool {
        func encode(_ value: Bool?) -> Int32 { value.map { $0 ? 1 : 0 } ?? -1 }
        return tc_private_inference_write_confirmed(encode(requestedOn), encode(echoedSeen), encode(echoedOn)) != 0
    }

    public static func shouldOffer(answered: Bool, on: Bool) -> Bool {
        tc_private_inference_should_offer(answered ? 1 : 0, on ? 1 : 0) != 0
    }
}
