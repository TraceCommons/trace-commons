import Foundation
import TCBridge
import TCShellCore

/// Every sentence this app says about withdrawal, read from the core's table
/// (`shell_words_copy::withdrawal_words`, through `ShellWords`).
///
/// ## The copy is not this app's to write
///
/// Three applications are built from `docs/contributor-daemon-ipc-v1_1.md`,
/// and withdrawal is the one place where a plausible-sounding phrase becomes
/// a false promise about erasure. So the per-tier confirmation bodies are
/// fixed in that document's **"Canonical confirmation copy"** table rather
/// than invented three times, and the core's three canonical bodies are
/// that table, reproduced word for word. They are not to be paraphrased,
/// shortened, or "tightened". `WithdrawalCopyCheck` fails loudly if they are.
///
/// The rules that come with them, and where each is honoured here:
///
/// 1. Never a generic "withdrawn" -- `resultSentence` always names what the
///    tier that actually applied did.
/// 2. Never claim more erasure than the tier achieved -- see the ambiguity
///    note below, which is the whole reason this file is more than three
///    strings.
/// 3. Withdrawal does not reverse settled credit and forfeits pending
///    credit -- `creditNote`, and nothing here says or implies otherwise.
/// 4. `not_found` must not disclose which -- `failureSentence`.
/// 5. Bulk withdrawal spans tiers -- `noBulkAction` explains why this app
///    does not offer it.
///
/// ## Why a confirmation cannot simply state the tier
///
/// The tier is computed by the server *during* the withdrawal, from live
/// export membership (in the endpoint: `accepted` plus
/// `count_trace_export_memberships > 0` is what makes it
/// `commons_distributed`). It arrives in the response. The confirmation has
/// to be shown before that response exists, and nothing the daemon gives
/// this app carries export membership -- `HistoryRecord` has a `status` and
/// nothing else bearing on it.
///
/// So the confirmation is keyed on what this machine actually knows:
///
/// - Pre-acceptance states (`submitted`, `received`, `quarantined`,
///   `awaiting_pii_backstop`, or `rejected`) -> `not_distributed`,
///   unambiguously. The server's own rule is `status != Accepted`, so this
///   mapping is exact and the canonical `not_distributed` body is shown alone.
/// - `accepted` -> either of the two commons tiers, and **this app cannot
///   tell which.** Showing only the `commons_not_distributed` body would be
///   claiming more erasure than may have been achieved, which is rule 2. So
///   both bodies are shown, marked as the two possibilities, with the app
///   saying plainly that it cannot tell them apart from here. A contributor
///   who reads that and withdraws anyway has not been misled if the trace
///   turns out to have been published; one shown only the gentler body would
///   have been.
///
/// The exact tier is then reported after the fact, from what the server
/// actually applied. That is a report, not the confirmation: the
/// confirmation still comes first, as the contract requires.
enum WithdrawalCopy {
    /// The core's withdrawal table (`shell_words_copy::withdrawal_words`).
    /// Nil when it does not decode, and then nothing here is confirmable.
    static var words: ShellWordsCopy.Withdrawal? { ShellWords.table?.withdrawal }

    // MARK: - The canonical bodies

    /// Canonical copy for `not_distributed`, verbatim (the core's).
    static var canonicalNotDistributed: String { words?.notDistributed ?? "" }

    /// Canonical copy for `commons_not_distributed`, verbatim (the core's).
    static var canonicalCommonsNotDistributed: String { words?.commonsNotDistributed ?? "" }

    /// Canonical copy for `commons_distributed`, verbatim (the core's). The
    /// clause from "but copies" onward is the one sentence in this whole
    /// feature that must never be softened, shortened, or quietly dropped.
    static var canonicalCommonsDistributed: String { words?.commonsDistributed ?? "" }

    static func canonicalBody(_ reach: WithdrawalReach) -> String {
        switch reach {
        case .notDistributed: return canonicalNotDistributed
        case .commonsNotDistributed: return canonicalCommonsNotDistributed
        case .commonsDistributed: return canonicalCommonsDistributed
        }
    }

    /// Settled credit is not clawed back; credit still pending is forfeited,
    /// because settlement never picks up a withdrawn trace. This app states
    /// only that -- nothing about how much credit, when it would have
    /// settled, or what it is worth.
    static var creditNote: String { words?.creditNote ?? "" }

    // MARK: - Before the action

    /// What this machine can honestly say about where a trace got to, read
    /// off the history record's status. Not the server's tier: this is the
    /// weaker thing the client knows before it asks.
    enum Stage {
        /// Any current server state other than accepted or terminal.
        /// `not_distributed`, exactly.
        case notInTheCommons
        /// `accepted`. One of the two commons tiers; not knowable which.
        case inTheCommons
        /// Any other status the daemon reports. Treated as the worst case.
        case unknown

        init(status: String) {
            switch status {
            case "submitted", "received", "quarantined", "awaiting_pii_backstop", "rejected":
                self = .notInTheCommons
            case "accepted": self = .inTheCommons
            default: self = .unknown
            }
        }
    }

    /// The confirmation, as parts rather than one blob, so the view can set
    /// the cannot-be-recalled body in the coral text token and leave the
    /// rest as body copy.
    struct Confirmation {
        /// The heading (#1146's dialog title).
        let question: String
        /// Under the heading (#1146's dialog description).
        let description: String
        /// Present only where the tier is ambiguous: says so before the
        /// canonical bodies it cannot choose between.
        let ambiguity: String?
        /// Canonical bodies that may apply, in order. One when the tier is
        /// known, two when it is not.
        let bodies: [String]
        /// Index into `bodies` of the one carrying the cannot-be-recalled
        /// clause, so the view can weight it. `nil` when none does.
        let gravest: Int?
        /// The credit note. Nil where the core's prompt carries its own.
        let credit: String?
        /// The action: #1146's "Confirm withdrawal" for every tier.
        let confirmLabel: String
        /// The action while the request is in flight.
        let busyLabel: String
    }

    /// The confirmation for `stage`, or nil when it cannot be worded: the
    /// words are the core's, and without them withdrawal is not confirmable.
    static func confirmation(for stage: Stage) -> Confirmation? {
        guard let words else { return nil }
        switch stage {
        case .notInTheCommons:
            return Confirmation(
                question: words.confirmTitle,
                description: words.confirmDescription,
                ambiguity: nil,
                bodies: [words.notDistributed],
                gravest: nil,
                credit: words.creditNote,
                confirmLabel: words.confirm,
                busyLabel: words.withdrawing
            )
        case .inTheCommons:
            return Confirmation(
                question: words.confirmTitle,
                description: words.confirmDescription,
                ambiguity: words.ambiguity,
                bodies: [words.commonsNotDistributed, words.commonsDistributed],
                gravest: 1,
                credit: words.creditNote,
                confirmLabel: words.confirm,
                busyLabel: words.withdrawing
            )
        case .unknown:
            // The core's prompt for a trace whose reach this machine cannot
            // know (`withdraw::confirmation_prompt_unknown`, through
            // `tc_withdrawal_confirmation_prompt_text`), the same text the
            // other shells show: what withdrawing does, that distributed
            // copies cannot be recalled, and the credit note, as one block
            // weighted as the gravest.
            guard let prompt = TCCoreCopy.withdrawalConfirmationPrompt(), !prompt.isEmpty else {
                return nil
            }
            return Confirmation(
                question: words.confirmTitle,
                description: words.confirmDescription,
                ambiguity: nil,
                bodies: [prompt],
                gravest: 0,
                credit: nil,
                confirmLabel: words.confirm,
                busyLabel: words.withdrawing
            )
        }
    }

    /// Said in the confirmation's place when it cannot be worded.
    static var disclosureUnavailable: String { words?.disclosureUnavailable ?? "" }

    // MARK: - After the action

    /// Over a completed withdrawal's result.
    static var resultHeading: String { words?.resultHeading ?? "" }

    /// What actually happened, from the tier the server applied. Never a
    /// generic "withdrawn": each tier has its own sentence.
    static func resultSentence(_ reach: WithdrawalReach?) -> String {
        guard let words else { return "" }
        switch reach {
        case nil:
            // The daemon sent a label this build does not know. The
            // withdrawal happened; what cannot be stated is how far the
            // trace had travelled -- so the furthest tier is not ruled out.
            return words.resultUnknown
        case .notDistributed?: return words.resultNotDistributed
        case .commonsNotDistributed?: return words.resultCommonsNotDistributed
        case .commonsDistributed?: return words.resultCommonsDistributed
        }
    }

    // MARK: - When it does not happen

    /// The account session was refused. Leads with nothing having happened.
    static var accountSessionRequired: String { words?.accountSessionRequired ?? "" }

    /// The daemon's labels for "the server has no record of this
    /// submission for this account". The server answers identically whether
    /// the submission belongs to somebody else or does not exist at all, so
    /// that accounts cannot be enumerated, and `notFound` says neither.
    static var notFoundLabels: Set<String> { Set(words?.notFoundLabels ?? []) }

    static var notFound: String { words?.notFound ?? "" }

    /// Any other failure. Never echoes the daemon's label, which is for logs.
    static func failureSentence(label: String) -> String {
        if notFoundLabels.contains(label) { return notFound }
        return words?.failed ?? ""
    }

    /// The retry control after a failure (#1146's "Try again").
    static var tryAgain: String { words?.tryAgain ?? "" }

    // MARK: - Bulk

    /// Why there is no "withdraw all of these" button: `withdraw_bulk`
    /// returns only counts, so afterwards there is no per-trace tier to
    /// report, and rule 1 -- never a generic "withdrawn" -- cannot hold.
    static var noBulkAction: String { words?.noBulkAction ?? "" }

    /// The defect notice's title when `WithdrawalCopyCheck` fails.
    static var wordingDefect: String { words?.wordingDefect ?? "" }
}

/// Assertions that belong on the copy, not on the plumbing.
///
/// Evaluated by the History screen itself and rendered as a visible defect
/// banner when they fail. The core's table carries the same properties as
/// Rust tests (`shell_words_copy`); this is the check on what this build
/// actually decoded, so a table that did not arrive, or arrived altered,
/// is visible rather than silently confirmable.
enum WithdrawalCopyCheck {
    /// Returns the failures, empty when the copy still holds.
    static func failures() -> [String] {
        var problems: [String] = []
        guard WithdrawalCopy.words != nil else {
            return ["the withdrawal wording did not arrive from the core"]
        }

        // The clause that must survive every future edit, in the canonical
        // body and everywhere that body is used.
        if !WithdrawalCopy.canonicalCommonsDistributed
            .contains("copies that have already been distributed cannot be recalled")
            || !WithdrawalCopy.canonicalCommonsDistributed.contains("does not undo that")
        {
            problems.append("the canonical commons_distributed body has been altered")
        }
        if WithdrawalCopy.canonicalNotDistributed.contains("excludes it from everything") {
            problems.append("the canonical not_distributed body has been altered")
        }
        if !WithdrawalCopy.canonicalCommonsNotDistributed
            .contains("has not been included in any published export")
        {
            problems.append("the canonical commons_not_distributed body has been altered")
        }

        // A trace already in the commons may be either commons tier, so it
        // must never be shown only the gentler one.
        let commons = WithdrawalCopy.confirmation(for: .inTheCommons)
        if commons?.bodies.contains(WithdrawalCopy.canonicalCommonsDistributed) != true {
            problems.append("an accepted trace is not warned about distributed copies")
        }
        if commons?.ambiguity == nil {
            problems.append("an accepted trace is shown a tier this app cannot know")
        }

        // ...and a trace that never entered the commons must not be told it
        // was excluded from exports it was never in.
        let outside = WithdrawalCopy.confirmation(for: .notInTheCommons)
        if outside?.bodies != [WithdrawalCopy.canonicalNotDistributed] {
            problems.append("a not-yet-in-the-commons trace is shown the wrong tier")
        }

        // Every tier says the same thing about credit, and only that.
        for stage in [WithdrawalCopy.Stage.notInTheCommons, .inTheCommons]
        where WithdrawalCopy.confirmation(for: stage)?.credit != WithdrawalCopy.creditNote {
            problems.append("a tier states something other than the verified credit note")
        }

        // A trace whose reach is unknown is shown the core's prompt, which
        // must warn about distributed copies and carry the same credit note.
        let unknown = WithdrawalCopy.confirmation(for: .unknown)?.bodies.first ?? ""
        if !unknown.contains("recalled") || !unknown.contains(WithdrawalCopy.creditNote) {
            problems.append("an unknown trace is not warned about distributed copies and credit")
        }

        // A failed withdrawal must say nothing happened, and a not-found
        // must disclose neither existence nor ownership.
        for sentence in [
            WithdrawalCopy.accountSessionRequired,
            WithdrawalCopy.notFound,
            WithdrawalCopy.failureSentence(label: "withdraw-failed"),
        ] where !sentence.contains("Nothing was withdrawn") {
            problems.append("a failure sentence does not say nothing happened")
        }
        let lowerNotFound = WithdrawalCopy.notFound.lowercased()
        if lowerNotFound.contains("belongs to") || lowerNotFound.contains("does not exist") {
            problems.append("the not-found sentence discloses existence or ownership")
        }

        // No outcome may be reported as a bare "withdrawn": each tier reads
        // differently, and the gravest one, and an unknown one, may not be
        // reported more gently than it was confirmed.
        let outcomes = [WithdrawalReach.notDistributed, .commonsNotDistributed, .commonsDistributed]
            .map { WithdrawalCopy.resultSentence($0) }
        if Set(outcomes).count != outcomes.count || outcomes.contains(where: \.isEmpty) {
            problems.append("an outcome does not name its tier")
        }
        if !WithdrawalCopy.resultSentence(.commonsDistributed).contains("cannot be recalled") {
            problems.append("a distributed trace is reported as though it could be recalled")
        }
        if !WithdrawalCopy.resultSentence(nil).contains("cannot be recalled") {
            problems.append("an unknown tier is reported as though nothing were distributed")
        }

        return problems
    }
}
