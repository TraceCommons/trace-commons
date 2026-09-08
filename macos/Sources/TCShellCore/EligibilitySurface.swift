import Foundation

/// The one control a shell may offer beside a queue entry, decoded from
/// `TC_CONTRIBUTION_CONTROL_*`.
///
/// A range of its own, disjoint from the tone range and from
/// `CredentialAction`'s. Both this and `CredentialAction` have a "nothing"
/// member and they govern different controls; one numbering shared between
/// them is one renumbering away from drawing a sign-in button on a queue row.
///
/// `.none` IS NOT "HIDE THE ROW". Every session on the contributor's computer
/// is shown; hiding their own work is its own dishonesty, and makes the app
/// look as though it had not noticed files they know it can see. The row is
/// present, unoffered, and carries its sentence.
public enum ContributionControl: Equatable, Sendable {
    case none
    case contribute

    /// Spelled out rather than derived from declaration order, and anything
    /// unknown is `.none`. Neutral is not the safe direction here: `.contribute`
    /// sends a contributor's work and has it refused, which is the defect this
    /// whole surface exists to remove.
    public static func fromABI(_ value: Int32) -> ContributionControl {
        switch value {
        case 51: return .contribute
        default: return .none
        }
    }
}

/// What one queue entry says about whether it can be contributed.
///
/// **Constructed only when the wire carried an `eligibility` key.** An absent
/// key is not a state: it means the contributor was invited rather than
/// admitted on evidence, or the daemon could not read the setting, and they
/// have no eligibility question. `parse` answers `nil` there, and a row with
/// `nil` renders exactly as it did before this surface existed. Reading the
/// absence as `unknown` would put a caveat on work that carries none.
///
/// The state is carried as the daemon's own string and never parsed into a
/// Swift enum, for the reason `CredentialStatus` carries its label that way:
/// a state a later daemon grows would otherwise have to be spelled here
/// before it could be shown, and the shared tables already answer an
/// unfamiliar label safely.
public struct ContributionEligibility: Equatable, Sendable {
    public let state: String
    /// `nil` on every `eligible` row -- there is nothing to explain -- and on
    /// a daemon that sent a state without one.
    public let reason: String?

    public init(state: String, reason: String? = nil) {
        self.state = state
        self.reason = reason
    }

    /// From a `list_pending` entry, a `snapshot` entry, or the `entry` object
    /// inside `preview` / `approve` / `preview_ready`.
    ///
    /// TESTS FOR THE KEY, never for a value. A JSON null and a missing key
    /// both answer `nil`, because neither is the daemon saying anything about
    /// eligibility; a present non-string is the same silence. What is NOT
    /// `nil` is an empty or unfamiliar string, which is the daemon saying
    /// something this build cannot read -- and that reaches the unknown
    /// sentence rather than nothing at all.
    public static func parse(_ object: [String: Any]?) -> ContributionEligibility? {
        guard let state = object?["eligibility"] as? String else { return nil }
        let reason = object?["eligibility_reason"] as? String
        return ContributionEligibility(
            state: state, reason: (reason?.isEmpty ?? true) ? nil : reason)
    }
}

/// The four shared tables this surface reads across the C ABI, injected so
/// `TCShellCore` can be tested without linking the dylib.
///
/// All four take the daemon's LABEL, not a decoded state, so a state a later
/// daemon grows never has to be spelled in Swift before it can be shown.
/// Production wiring is `TCContributionEligibility`; see `AppModel`.
public struct EligibilityCalls: Sendable {
    /// The sentence for one `eligibility` label.
    public let stateLine: @Sendable (String) -> String?
    /// How firmly that sentence reads, as a raw `TC_PRIVATE_INFERENCE_TONE_*`
    /// value. Never recovered by reading the sentence.
    public let stateTone: @Sendable (String) -> Int32
    /// The one control offered for that state, as a raw
    /// `TC_CONTRIBUTION_CONTROL_*` value.
    public let control: @Sendable (String) -> Int32
    /// The sentence for one `eligibility_reason` label, or the empty string.
    public let reasonLine: @Sendable (String) -> String?
    /// How many sessions a group submit leaves behind, as a sentence.
    /// Empty for zero and for a negative.
    public let withheldLine: @Sendable (Int64) -> String?

    public init(
        stateLine: @escaping @Sendable (String) -> String?,
        stateTone: @escaping @Sendable (String) -> Int32,
        control: @escaping @Sendable (String) -> Int32,
        reasonLine: @escaping @Sendable (String) -> String?,
        withheldLine: @escaping @Sendable (Int64) -> String?
    ) {
        self.stateLine = stateLine
        self.stateTone = stateTone
        self.control = control
        self.reasonLine = reasonLine
        self.withheldLine = withheldLine
    }
}

/// What a folder's group submit control offers.
///
/// `count` is `nil` when NO CONTROL IS DRAWN -- ruled for a folder with
/// nothing contributable in it, on the same rule an ineligible row follows.
/// It is never zero: a button reading "Submit all (0)" is a control that
/// does nothing, discovered on the press.
///
/// `withheldLine` is the shared sentence saying how many the button leaves
/// behind, and `nil` when there is no gap to explain. It never says why --
/// the reason a particular session cannot be sent is that row's own
/// sentence, one level in.
public struct GroupSubmitOffer: Equatable, Sendable {
    public let count: Int?
    public let withheldLine: String?

    public init(count: Int?, withheldLine: String?) {
        self.count = count
        self.withheldLine = withheldLine
    }
}

/// What this shell renders about whether one session can be contributed.
///
/// Holds no words and takes no branch of its own. Every sentence is a field
/// of `PrivateInferenceCopy` or comes back from `calls`, and the three
/// decisions that matter -- which sentence, which tone, whether a send
/// control is offered -- are all the shared tables'. A `switch` here would be
/// the third copy of a decision that draws the button which sends a
/// contributor's work.
///
/// Every method takes the eligibility as an OPTIONAL and answers nothing for
/// `nil`. That is the one branch this file does take, and it is not a branch
/// on a state: it is the difference between a contributor who has an
/// eligibility question and one who does not.
public enum EligibilitySurface {
    // MARK: - The state

    /// The sentence on the row, or none at all.
    ///
    /// `nil` for an entry that carried no `eligibility` key -- an invited
    /// contributor gets the row they always got. Falls back to the payload's
    /// unknown sentence when the Rust caught a panic, never to an
    /// ineligibility: a state this build could not read is not evidence about
    /// a contributor's session.
    public static func stateLine(
        _ eligibility: ContributionEligibility?,
        copy: PrivateInferenceCopy,
        calls: EligibilityCalls
    ) -> String? {
        guard let eligibility else { return nil }
        return calls.stateLine(eligibility.state) ?? copy.eligibilityUnknown
    }

    /// The tone that sentence is painted in.
    ///
    /// `PrivateInferenceTone` is reused rather than duplicated, which is what
    /// the ABI asks for: a shell maps those five values onto colours once,
    /// and a second enum with the same five meanings is a second mapping to
    /// keep in agreement. `nil` for an absent key, so a row with no
    /// eligibility question is painted no differently than before.
    public static func tone(
        _ eligibility: ContributionEligibility?, calls: EligibilityCalls
    ) -> PrivateInferenceTone? {
        guard let eligibility else { return nil }
        return PrivateInferenceTone.fromABI(calls.stateTone(eligibility.state))
    }

    // MARK: - The one control

    /// Whether a send control may be drawn beside this row.
    ///
    /// **An absent key answers `.contribute`.** That is not a fail-open: an
    /// invited contributor has no eligibility question, everything in their
    /// queue is contributable, and withholding the button from them would
    /// break the surface for the population it was never about. The daemon
    /// decides which of the two it is by sending the field or not; this
    /// shell does not guess.
    public static func control(
        _ eligibility: ContributionEligibility?, calls: EligibilityCalls
    ) -> ContributionControl {
        guard let eligibility else { return .contribute }
        return ContributionControl.fromABI(calls.control(eligibility.state))
    }

    /// The same answer as a boolean, for a view that has a button to arm
    /// rather than a button to draw.
    ///
    /// The preview sheet's Contribute is the only approve control in the
    /// product and it already exists on screen; there it is disarmed rather
    /// than removed, because a control that vanished mid-read is its own
    /// confusion. The queue card draws no button at all -- see `control`.
    public static func offersContribute(
        _ eligibility: ContributionEligibility?, calls: EligibilityCalls
    ) -> Bool {
        control(eligibility, calls: calls) == .contribute
    }

    // MARK: - A sheet that is already open

    /// The copy of a held entry a still-open surface must gate on.
    ///
    /// **A GATE APPLIED ONCE AT OPEN TIME IS NOT A GATE.** A sheet holds the
    /// entry it was handed when it opened, and the queue behind it is
    /// republished whenever a snapshot arrives -- including the snapshot
    /// that carries a submit-time failure written back into the row
    /// (Decision 1 of the design). A surface still reading its opening copy
    /// goes on offering to send a session the queue already knows cannot be
    /// sent, which is this surface's own defect reproduced by staleness
    /// rather than by a missing check.
    ///
    /// Falls back to the held copy when the queue no longer lists it: an
    /// entry that has just been approved or dismissed is leaving the screen
    /// anyway, and blanking its sentence on the way out would be a flicker
    /// that says something it does not mean.
    public static func current<Entry>(
        _ held: Entry, in queue: [Entry], id: (Entry) -> String
    ) -> Entry {
        let heldID = id(held)
        return queue.first { id($0) == heldID } ?? held
    }

    // MARK: - A group-level submit

    /// Which of a group's entries a group-level submit may actually send.
    ///
    /// **A GROUP-LEVEL SUBMIT MEANS "ALL ELIGIBLE", NEVER "ALL".** One
    /// button that approves a whole folder by project id, without asking
    /// this, is the defect this surface exists to remove reproduced one
    /// layer up -- and worse there than on a card, because a contributor who
    /// pressed it never saw the sessions it sent. Ruled 2026-09-08.
    ///
    /// Entries with no `eligibility` key are kept, on the same rule
    /// `control` follows: an invited contributor's folder is entirely
    /// contributable and this must not quietly empty it.
    ///
    /// Order is preserved, so the caller can report what it sent in the
    /// order the folder showed it.
    public static func contributable<Entry>(
        _ entries: [Entry],
        eligibility: (Entry) -> ContributionEligibility?,
        calls: EligibilityCalls
    ) -> [Entry] {
        entries.filter { offersContribute(eligibility($0), calls: calls) }
    }

    /// What a folder's group control offers, from the counts the daemon
    /// reports on its `list_projects` row.
    ///
    /// `contributableCount` is **absent when eligibility does not apply** --
    /// an invited contributor has no "3 of 7" to be told about, and
    /// `pendingCount` alone is their answer. Test for the key; it is never
    /// null and never zero-as-absent.
    ///
    /// The counts come from the daemon rather than being recomputed here,
    /// which is what changed when the group approve moved server-side: the
    /// daemon decides what a group selector takes, so the daemon is what
    /// says how many that is. This shell no longer classifies rows to
    /// answer it.
    ///
    /// `fallbackPending` covers a project the queue is showing before
    /// `list_projects` has answered for it -- the folder is on screen either
    /// way and must say something.
    public static func groupSubmit(
        pendingCount: Int?,
        contributableCount: Int?,
        fallbackPending: Int,
        calls: EligibilityCalls
    ) -> GroupSubmitOffer {
        let pending = pendingCount ?? fallbackPending
        guard let contributable = contributableCount else {
            // No eligibility question. The folder submits whole, exactly as
            // it did before this surface existed, and there is no gap to
            // explain.
            return GroupSubmitOffer(count: pending == 0 ? nil : pending, withheldLine: nil)
        }
        // RULED: nothing contributable offers no control at all, the same
        // rule an ineligible row follows. A disabled button on a folder
        // where every session is unsendable is something to hunt for a
        // setting about; the rows inside already say why.
        guard contributable > 0 else { return GroupSubmitOffer(count: nil, withheldLine: nil) }
        let sentence = calls.withheldLine(Int64(pending - contributable))
        return GroupSubmitOffer(
            count: contributable,
            withheldLine: (sentence?.isEmpty ?? true) ? nil : sentence)
    }

    // MARK: - The reason

    /// The second sentence, naming why, or none.
    ///
    /// THE EMPTY STRING IS RENDERED AS NOTHING, and that is deliberately not
    /// the hedge the state line makes: the state sentence has already said
    /// what is true, and a second sentence guessing at a reason this build
    /// does not know would add a detail nobody established. An `eligible` row
    /// carries no reason at all, for the same reason -- there is nothing to
    /// explain.
    public static func reasonLine(
        _ eligibility: ContributionEligibility?, calls: EligibilityCalls
    ) -> String? {
        guard let reason = eligibility?.reason, !reason.isEmpty else { return nil }
        guard let sentence = calls.reasonLine(reason), !sentence.isEmpty else { return nil }
        return sentence
    }
}
