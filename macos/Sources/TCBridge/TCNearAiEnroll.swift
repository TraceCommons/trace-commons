import CTraceCommons
import Foundation

/// What joining a commons with a NEAR AI login says, across the C ABI.
///
/// Handle-free, like `TCAttestation`: these describe the build rather than a
/// running daemon. Every sentence comes from
/// `crates/trace-commons-contributor/src/private_inference_copy.rs`, so this
/// shell, the GTK shell and the Windows shell say one thing about a way in
/// that needs no wallet.
///
/// **Ten control names, ten sentences, and this file branches on none of
/// them.** The daemon's label goes straight through. A `switch` here would be
/// an eleventh table that agrees with the shared one until it does not.
///
/// **The three that refuse before anything is spent must not be run
/// together.** `near_ai_enroll_no_session` means sign in first,
/// `near_ai_enroll_commons_unreachable` means the network, and
/// `near_ai_enroll_commons_unsupported` means this commons does not offer the
/// path at all. A contributor told the wrong one debugs the wrong thing.
public enum TCNearAiEnroll {
    /// The sentence for one control name.
    ///
    /// Never the empty string, unlike an attestation reason: a refusal this
    /// build cannot name is the whole of what a contributor is being told,
    /// so an unfamiliar label reaches the generic sentence rather than
    /// nothing. `nil` means a caught Rust panic.
    public static func line(label: String) -> String? {
        guard let raw = label.withCString({ tc_near_ai_enroll_line($0) }) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// How firmly that sentence reads, as a raw `TC_PRIVATE_INFERENCE_TONE_*`
    /// value.
    ///
    /// Takes what the sentence takes, so the two cannot drift apart. Not
    /// every failure is a wall: not being signed in yet is `_ATTENTION`,
    /// because there is a step to take, and being already joined is `_CLEAR`,
    /// because nothing was refused.
    public static func tone(label: String) -> Int32 {
        label.withCString { tc_near_ai_enroll_tone($0) }
    }
}
