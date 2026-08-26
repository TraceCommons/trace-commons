import Foundation

/// One entry `approve` did not act on, and why.
///
/// Mirrors the daemon's `skipped` array entries
/// (`crates/trace-commons-contributor/src/daemon/ipc.rs`, the `approve`
/// handler): `entry_id` plus the wire `reason_label`, never a human string
/// -- `SubmitToast.reasonLabel` is what turns the wire label into words, and
/// it does that once, in one place, for every shell.
public struct ApproveSkip: Decodable, Equatable, Sendable {
    public let entryID: String
    public let reasonLabel: String

    public init(entryID: String, reasonLabel: String) {
        self.entryID = entryID
        self.reasonLabel = reasonLabel
    }

    enum CodingKeys: String, CodingKey {
        case entryID = "entry_id"
        case reasonLabel = "reason_label"
    }
}

/// The full response of one `approve` call: `{"approved", "flagged",
/// "redactions", "skipped", "hold_secs", "hold_until"}`.
///
/// `holdUntil` stays a raw wire string rather than a `Date`: it is not
/// rendered anywhere today (`AppModel.Undo` deliberately counts up from a
/// real instant instead of down to a deadline this process cannot observe --
/// see the comment on that type), and decoding it here would tie this
/// module, which links no FFI and carries none of the app's date-decoding
/// plumbing, to a decoder it does not own. A caller that later needs it
/// parses the string itself.
public struct ApproveResponse: Decodable, Equatable, Sendable {
    public let approved: UInt64
    public let flagged: UInt64
    public let redactions: [String: UInt32]
    public let skipped: [ApproveSkip]
    public let holdSecs: UInt64
    public let holdUntil: String?

    public init(
        approved: UInt64,
        flagged: UInt64,
        redactions: [String: UInt32],
        skipped: [ApproveSkip],
        holdSecs: UInt64,
        holdUntil: String?
    ) {
        self.approved = approved
        self.flagged = flagged
        self.redactions = redactions
        self.skipped = skipped
        self.holdSecs = holdSecs
        self.holdUntil = holdUntil
    }

    enum CodingKeys: String, CodingKey {
        case approved
        case flagged
        case redactions
        case skipped
        case holdSecs = "hold_secs"
        case holdUntil = "hold_until"
    }

    /// Whether this response is the correction-credential refusal.
    ///
    /// Distinguished from every other skip because it is the only one the
    /// contributor caused and the only one they can fix, and because the
    /// advice that goes with it -- rotate the credential, it has already
    /// been typed -- is not advice any other refusal carries. A caller that
    /// sees this shows `CorrectionCopy.credentialHeadline` and its body
    /// instead of the ordinary toast.
    public var wasRefusedForACorrectionCredential: Bool {
        skipped.contains { $0.reasonLabel == CorrectionCopy.credentialRefusalLabel }
    }

    /// The sum of the `redactions` map -- what `SubmitToast` names, never a
    /// category.
    public var totalRedactions: UInt64 {
        redactions.values.reduce(0) { $0 + UInt64($1) }
    }

    /// The toast this response renders as, in one call: sums `redactions`,
    /// passes `skipped`'s wire reason labels through in response order. See
    /// `SubmitToast.render`.
    public var toast: SubmitToast {
        SubmitToast.render(
            approved: approved,
            redactions: totalRedactions,
            flagged: flagged,
            skipped: skipped.map(\.reasonLabel)
        )
    }

    /// Which of `attempted` this response actually approved.
    ///
    /// `approve` reports a count, not a list -- so the entries it approved
    /// are `attempted` minus `skipped`'s ids, computed against the ids the
    /// caller sent (or, for a project call, the ids it read off its own
    /// pending list at the moment of the click). A daemon-side race between
    /// that snapshot and the call landing can make this set slightly wrong
    /// in either direction; the consequence is bounded by `cancel` itself,
    /// which the caller uses this list to drive -- it refuses anything not
    /// presently `Approved` rather than acting on a bad guess.
    public func approvedEntryIDs(attempted: [String]) -> [String] {
        let skippedIDs = Set(skipped.map(\.entryID))
        return attempted.filter { !skippedIDs.contains($0) }
    }
}
