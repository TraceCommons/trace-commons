import CTraceCommons
import Foundation

/// The credential surface's three branch tables, across the C ABI.
///
/// Handle-free, like `TCPrivateInference`: every call here describes the
/// build, not a running daemon. The sentence, the tone and the button all
/// come from `crates/trace-commons-contributor/src/private_inference_copy.rs`,
/// so this shell, the GTK shell and the Windows shell say one thing about a
/// key and offer one button beside it.
///
/// Nothing in this file authors a sentence and nothing in it decides a tone.
/// A `nil` means a caught Rust panic, and the caller renders the payload's
/// unreported sentence rather than filling the hole in.
public enum TCNearAiCredential {
    /// The sentence for one `near_ai_credential_status` state label.
    ///
    /// An empty label reports that this daemon does not answer the question;
    /// an unfamiliar one reports that the state could not be read. Neither
    /// says that no key is kept here -- that is a claim about the machine,
    /// and the Rust decides it, not this shell.
    public static func stateLine(state: String) -> String? {
        guard let raw = state.withCString({ tc_near_ai_credential_state_line($0) }) else {
            return nil
        }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// How firmly that sentence reads, as a raw `TC_PRIVATE_INFERENCE_TONE_*`
    /// value -- the same five the listener state row uses.
    ///
    /// Takes what the sentence takes, so the two stay in step. There is no
    /// failure value: an unknown state and a caught panic both answer the
    /// tone that claims nothing.
    public static func stateTone(state: String) -> Int32 {
        state.withCString { tc_near_ai_credential_state_tone($0) }
    }

    /// The one action this state may offer, as a raw
    /// `TC_CREDENTIAL_ACTION_*` value.
    ///
    /// The branch table crosses, not only the words. Three shells each
    /// deciding which button belongs beside which state is three chances to
    /// draw a sign-in next to a key that is already here.
    public static func action(state: String) -> Int32 {
        state.withCString { tc_near_ai_credential_action($0) }
    }

    /// Why a connect control is not on offer, or the empty string.
    ///
    /// `credentialed` is the tri-state `HarnessList.credentialedABIValue`
    /// produces: negative for a field the daemon never sent, `0` false, `1`
    /// true. Drawn once beside the connect controls, because the fact is
    /// about the destination and not about any one tool.
    public static func harnessNotice(credentialed: Int32) -> String? {
        guard let raw = tc_harness_credential_notice(credentialed) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
