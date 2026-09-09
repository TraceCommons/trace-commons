import CTraceCommons
import Foundation

/// The attestation mark's three branch tables, across the C ABI.
///
/// Handle-free, like `TCContributionEligibility`: every call here describes
/// the build, not a running daemon. The sentence and the tone both come from
/// `crates/trace-commons-contributor/src/private_inference_copy.rs`, so this
/// shell, the GTK shell and the Windows shell say one thing about the proof
/// a session carries.
///
/// Nothing in this file authors a sentence and nothing in it decides a tone.
/// A `nil` means a caught Rust panic, and the caller renders the payload's
/// unknown sentence rather than filling the hole in.
///
/// **Call this for EVERY entry**, including one from a contributor who was
/// invited rather than admitted on evidence. That is the opposite of
/// `TCContributionEligibility`'s rule and it is deliberate: eligibility asks
/// whether somebody may send a session, and an invited contributor is not
/// being asked; the mark says whether the session carries a checkable copy
/// of the model call it came from, which is true or false of the trace
/// whoever holds it.
///
/// **There is no `tc_contribution_attestation_control`.** The mark describes
/// and does not offer; sendability stays
/// `tc_contribution_eligibility_control`'s question.
public enum TCAttestation {
    /// The sentence for one `attestation` label.
    ///
    /// An empty or unfamiliar label reports that the answer has not been
    /// worked out. IT NEVER REPORTS AN UNATTESTED SESSION: a mark this build
    /// cannot read is not evidence about what a contributor's session
    /// carries, and once credit weights attestations, saying it is would
    /// understate their work.
    public static func markLine(mark: String) -> String? {
        guard let raw = mark.withCString({ tc_contribution_attestation_line($0) }) else {
            return nil
        }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// How firmly that sentence reads, as a raw `TC_PRIVATE_INFERENCE_TONE_*`
    /// value -- the same five the listener state row uses.
    ///
    /// Takes what the sentence takes, so the two stay in step. There is no
    /// failure value: an unknown mark and a caught panic both answer the
    /// tone that claims nothing. Nothing on this surface is ever painted as
    /// a refusal -- nothing was refused.
    public static func markTone(mark: String) -> Int32 {
        mark.withCString { tc_contribution_attestation_tone($0) }
    }

    /// The sentence for one `attestation_reason` label, or the empty string.
    ///
    /// The thirteen labels are the eligibility reason's thirteen and the
    /// SENTENCES ARE NOT. Reaching for
    /// `tc_contribution_eligibility_reason_line` here would compile, render,
    /// and tell a contributor their session had been refused.
    ///
    /// The empty string for an unfamiliar, empty or absent reason, and the
    /// caller renders nothing for it. That is deliberately different from
    /// the mark line's fallback: an unknown mark still has to say something,
    /// an unknown reason has nothing honest to say.
    public static func reasonLine(reason: String) -> String? {
        guard let raw = reason.withCString({ tc_contribution_attestation_reason_line($0) }) else {
            return nil
        }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
