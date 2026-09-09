import Foundation

/// What one queue entry says about whether it carries proof of the model
/// call that produced it.
///
/// **Constructed for EVERY entry, and that is the opposite of
/// `ContributionEligibility`'s rule.** Eligibility asks whether this
/// contributor may send this session; it is a permission question, and it is
/// absent for an invited contributor because nobody is asking them anything.
/// The mark answers a different question -- does this session carry a
/// checkable copy of the call it came from -- which is a fact about the
/// trace, is owed to everyone, and matters to what the work is worth once
/// credit weights attestations. So there is no "no mark" state: an entry
/// whose `attestation` key never arrived carries the empty label, and the
/// shared table answers that with the unknown sentence.
///
/// The mark is carried as the daemon's own string and never parsed into a
/// Swift enum, for the reason `ContributionEligibility` carries its state
/// that way: a mark a later daemon grows would otherwise have to be spelled
/// here before it could be shown, and the shared tables already answer an
/// unfamiliar label safely.
public struct AttestationMark: Equatable, Sendable {
    /// One of `attested`, `unattested_permanent`,
    /// `unattested_configuration`, `unknown`, a label a later daemon grew,
    /// or the empty string when the key did not arrive at all. All of them
    /// reach a sentence.
    public let mark: String

    /// One of the thirteen shared reason labels, or `nil`.
    ///
    /// **PRESENCE VARIES WITHIN A MARK.** `attested` never carries one;
    /// both unattested marks always do; `unknown` carries one only
    /// sometimes -- absent when the row was never evaluated, and
    /// `receipt_unavailable` when a send was retracted because the service
    /// the receipt comes from was unreachable. That last one is the whole
    /// reason this is read off the KEY and never derived from the mark:
    /// suppressing it drops the only signal telling a contributor the answer
    /// may change.
    public let reason: String?

    public init(mark: String, reason: String? = nil) {
        self.mark = mark
        self.reason = reason
    }

    /// From a `list_pending` entry, a `snapshot` entry, or the `entry`
    /// object inside `preview` / `approve` / `preview_ready`.
    ///
    /// Never answers "nothing". A missing key, a JSON null and a present
    /// non-string all become the empty label, which the shared table renders
    /// as the unknown sentence -- because silence about whether a session
    /// carries proof is itself a claim, and it is one this shell has no
    /// grounds to make.
    ///
    /// The reason is the mirror of that: absent, null, non-string and empty
    /// all answer `nil`, and nothing is drawn for it.
    public static func parse(_ object: [String: Any]?) -> AttestationMark {
        let mark = object?["attestation"] as? String ?? ""
        let reason = object?["attestation_reason"] as? String
        return AttestationMark(mark: mark, reason: (reason?.isEmpty ?? true) ? nil : reason)
    }
}

/// The three shared tables this surface reads across the C ABI, injected so
/// `TCShellCore` can be tested without linking the dylib.
///
/// All three take the daemon's LABEL, not a decoded mark. Production wiring
/// is `TCAttestation`; see `AppModel`.
///
/// **There are three and there is no fourth.** There is no
/// `tc_contribution_attestation_control`, deliberately: the mark describes
/// the trace and offers nothing to press. Whether a session may be sent is
/// `EligibilityCalls.control`'s question, and a button drawn off the mark
/// would be an action invented out of a description.
public struct AttestationCalls: Sendable {
    /// The sentence for one `attestation` label.
    public let markLine: @Sendable (String) -> String?
    /// How firmly that sentence reads, as a raw `TC_PRIVATE_INFERENCE_TONE_*`
    /// value. Never recovered by reading the sentence.
    public let markTone: @Sendable (String) -> Int32
    /// The sentence for one `attestation_reason` label, or the empty string.
    public let reasonLine: @Sendable (String) -> String?

    public init(
        markLine: @escaping @Sendable (String) -> String?,
        markTone: @escaping @Sendable (String) -> Int32,
        reasonLine: @escaping @Sendable (String) -> String?
    ) {
        self.markLine = markLine
        self.markTone = markTone
        self.reasonLine = reasonLine
    }
}

/// What this shell renders about the proof one session carries.
///
/// Holds no words and takes no branch of its own, exactly as
/// `EligibilitySurface` does not: every sentence is a field of
/// `PrivateInferenceCopy` or comes back from `calls`, and both decisions
/// that matter -- which sentence, which tone -- are the shared tables'.
///
/// The one difference from its sibling is the optionality. Every method here
/// takes a mark rather than an optional one and answers a sentence rather
/// than possibly nothing, because there is no contributor for whom the
/// question does not apply.
public enum AttestationSurface {
    // MARK: - The mark

    /// The sentence on the row. Always one.
    ///
    /// Falls back to the payload's unknown sentence when the Rust caught a
    /// panic, never to an unattested mark: a mark this build could not read
    /// is not evidence that a contributor's session lacks proof.
    public static func markLine(
        _ mark: AttestationMark,
        copy: PrivateInferenceCopy,
        calls: AttestationCalls
    ) -> String {
        guard let sentence = calls.markLine(mark.mark), !sentence.isEmpty else {
            return copy.attestationUnknown
        }
        return sentence
    }

    /// The tone that sentence is painted in.
    ///
    /// `PrivateInferenceTone` is reused rather than duplicated, which is what
    /// the ABI asks for: a shell maps those five values onto colours once.
    /// A mark this build cannot read claims nothing -- `.neutral` -- and
    /// never reads as settled.
    public static func tone(
        _ mark: AttestationMark, calls: AttestationCalls
    ) -> PrivateInferenceTone {
        PrivateInferenceTone.fromABI(calls.markTone(mark.mark))
    }

    // MARK: - The reason

    /// The second sentence, naming why, or none.
    ///
    /// **Gated on the reason KEY and never on the mark.** Two rows can both
    /// be `unknown` and only one of them carry a reason: the row nobody
    /// evaluated has nothing to add, while a send retracted because the
    /// receipt service was unreachable carries `receipt_unavailable`, which
    /// is what tells a contributor it may work later. A branch on the mark
    /// would collapse those two into one and drop that signal.
    ///
    /// THE EMPTY STRING IS RENDERED AS NOTHING. The mark's sentence has
    /// already said what is true, and a second sentence guessing at a reason
    /// this build does not know would add a detail nobody established.
    public static func reasonLine(
        _ mark: AttestationMark, calls: AttestationCalls
    ) -> String? {
        guard let reason = mark.reason, !reason.isEmpty else { return nil }
        guard let sentence = calls.reasonLine(reason), !sentence.isEmpty else { return nil }
        return sentence
    }
}
