import Foundation

/// Every sentence this app says about withdrawal, in one file.
///
/// ## The copy is not this app's to write
///
/// Three applications are built from `docs/contributor-daemon-ipc-v1_1.md`,
/// and withdrawal is the one place where a plausible-sounding phrase becomes
/// a false promise about erasure. So the per-tier confirmation bodies are
/// fixed in that document's **"Canonical confirmation copy"** table rather
/// than invented three times, and the three `canonical*` constants below are
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
/// 3. Withdrawal does not reverse settled credit -- `creditNote`, and
///    nothing here says or implies otherwise.
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
/// - `submitted` / `quarantined` -> `not_distributed`, unambiguously. The
///   server's own rule is `status != Accepted`, so this mapping is exact and
///   the canonical `not_distributed` body is shown alone.
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
    // MARK: - The canonical bodies

    /// Canonical copy for `not_distributed`, verbatim.
    static let canonicalNotDistributed =
        "This trace never entered the commons. Withdrawing deletes it. Nothing was "
        + "distributed and nothing needs recalling."

    /// Canonical copy for `commons_not_distributed`, verbatim.
    static let canonicalCommonsNotDistributed =
        "This trace is in the commons but has not been included in any published export "
        + "or benchmark yet. Withdrawing deletes it and excludes it from everything "
        + "published from here on."

    /// Canonical copy for `commons_distributed`, verbatim. The clause from
    /// "but copies" onward is the one sentence in this whole feature that
    /// must never be softened, shortened, or quietly dropped.
    static let canonicalCommonsDistributed =
        "This trace has already been included in a published export or benchmark. "
        + "Withdrawing deletes our copy and excludes it from everything published from "
        + "here on, but copies that have already been distributed cannot be recalled. "
        + "Withdrawing does not undo that."

    static func canonicalBody(_ reach: WithdrawalReach) -> String {
        switch reach {
        case .notDistributed: return canonicalNotDistributed
        case .commonsNotDistributed: return canonicalCommonsNotDistributed
        case .commonsDistributed: return canonicalCommonsDistributed
        }
    }

    /// Credit is not clawed back. Verified rather than assumed: the
    /// endpoint's response sets `credit_retained: true` unconditionally
    /// ("Always true. Withdrawal is not a punishment: credit already awarded
    /// stays awarded"). This app states only that -- nothing about how much
    /// credit, when it settles, or what it is worth.
    static let creditNote = "Credit already recorded stays."

    // MARK: - Before the action

    /// What this machine can honestly say about where a trace got to, read
    /// off the history record's status. Not the server's tier: this is the
    /// weaker thing the client knows before it asks.
    enum Stage {
        /// `submitted` or `quarantined`. `not_distributed`, exactly.
        case notInTheCommons
        /// `accepted`. One of the two commons tiers; not knowable which.
        case inTheCommons
        /// Any other status the daemon reports. Treated as the worst case.
        case unknown

        init(status: String) {
            switch status {
            case "submitted", "quarantined": self = .notInTheCommons
            case "accepted": self = .inTheCommons
            default: self = .unknown
            }
        }
    }

    /// The confirmation, as parts rather than one blob, so the view can set
    /// the cannot-be-recalled body in the coral text token and leave the
    /// rest as body copy.
    struct Confirmation {
        let question: String
        /// Present only where the tier is ambiguous: says so, in this app's
        /// own words, before the canonical bodies it cannot choose between.
        let ambiguity: String?
        /// Canonical bodies that may apply, in order. One when the tier is
        /// known, two when it is not.
        let bodies: [String]
        /// Index into `bodies` of the one carrying the cannot-be-recalled
        /// clause, so the view can weight it. `nil` when none does.
        let gravest: Int?
        let credit: String
        /// "Withdraw" where the outcome is unambiguous, "Withdraw anyway"
        /// where the contributor is being asked to accept a limit.
        let confirmLabel: String
    }

    static func confirmation(for stage: Stage) -> Confirmation {
        switch stage {
        case .notInTheCommons:
            return Confirmation(
                question: "Withdraw this trace?",
                ambiguity: nil,
                bodies: [canonicalNotDistributed],
                gravest: nil,
                credit: creditNote,
                confirmLabel: "Withdraw"
            )
        case .inTheCommons:
            return Confirmation(
                question: "Withdraw this trace?",
                ambiguity: "This trace is in the commons. Whether it has already gone into "
                    + "a published export or benchmark is decided on the server, and this "
                    + "app cannot tell from here which of these two applies:",
                bodies: [canonicalCommonsNotDistributed, canonicalCommonsDistributed],
                gravest: 1,
                credit: creditNote,
                confirmLabel: "Withdraw anyway"
            )
        case .unknown:
            return Confirmation(
                question: "Withdraw this trace?",
                ambiguity: "This app does not recognise what stage this trace reached, so "
                    + "it cannot rule out the furthest one:",
                bodies: [canonicalCommonsDistributed],
                gravest: 0,
                credit: creditNote,
                confirmLabel: "Withdraw anyway"
            )
        }
    }

    // MARK: - After the action

    /// What actually happened, from the tier the server applied. Never a
    /// generic "withdrawn": the canonical body for the tier that applied is
    /// what says which of the three outcomes this was.
    static func resultSentence(_ reach: WithdrawalReach?) -> String {
        guard let reach else {
            // The daemon sent a label this build does not know. The
            // withdrawal happened; what cannot be stated is how far the
            // trace had travelled -- so the furthest tier is not ruled out.
            return "Withdrawn, but the server did not report which of the three tiers "
                + "applied, so this app cannot tell you whether it had already been "
                + "included in a published export or benchmark. If it had, copies that "
                + "have already been distributed cannot be recalled."
        }
        return "Withdrawn. " + canonicalBody(reach)
    }

    // MARK: - When it does not happen

    /// Withdrawal is authenticated by an account session, which this build
    /// has no way to obtain. Leads with the fact that nothing happened: a
    /// contributor must not walk away from a failed withdrawal believing
    /// their trace was taken back.
    static let accountSessionRequired =
        "Nothing was withdrawn and nothing was deleted. Withdrawal is an account-level "
        + "act, so it is authenticated by your Trace Commons account rather than by this "
        + "device -- that is what lets you withdraw a trace after losing the machine that "
        + "sent it. This build has no account sign-in yet, so it cannot make the request."

    /// The daemon's label for "the server has no record of this submission
    /// for this account".
    ///
    /// The server answers identically whether the submission belongs to
    /// somebody else or does not exist at all, so that accounts cannot be
    /// enumerated, and this app must not undo that by guessing out loud --
    /// see rule 4. `notFound` therefore says neither.
    ///
    /// Unreachable today: `daemon/withdraw.rs` collapses every
    /// `WithdrawError` into the single label `withdraw-failed`, so the 404
    /// the client crate carefully classifies never reaches this process.
    /// Handled anyway, because the day that label is passed through is not
    /// the day to be inventing this sentence.
    static let notFoundLabels: Set<String> = ["not-found", "not_found", "submission-not-found"]

    static let notFound =
        "Nothing was withdrawn and nothing was deleted. There is no trace with that id "
        + "under your account."

    /// Any other failure. Same first clause, same reason.
    static func failureSentence(label: String) -> String {
        if notFoundLabels.contains(label) { return notFound }
        return "Nothing was withdrawn and nothing was deleted. The request did not go "
            + "through (\(label)). You can try again."
    }

    // MARK: - Bulk

    /// Why there is no "withdraw all of these" button.
    ///
    /// The contract's rule 5 permits bulk only if the confirmation can say
    /// that the selected traces may fall into different tiers and that some
    /// may already have been distributed. This app has a second problem on
    /// top of that one, and it is the reason bulk is left out rather than
    /// worded around: `withdraw_bulk` returns only `withdrawn` and `failed`
    /// counts, so afterwards there is no per-trace tier to report and rule 1
    /// -- never a generic "withdrawn" -- cannot be honoured at all.
    static let noBulkAction =
        "There is no button here that withdraws all of them at once. The daemon's bulk "
        + "call reports only how many succeeded, never what happened to any one trace, "
        + "and it chooses what to withdraw from this machine's copy of your history, "
        + "which can be out of date -- so it could not tell you afterwards which of "
        + "these had already been distributed. Withdraw them one at a time below and "
        + "each one tells you what it actually did."
}

/// Assertions that belong on the copy, not on the plumbing.
///
/// The Swift package has no test target, so these are evaluated by the
/// History screen itself and rendered as a visible defect banner when they
/// fail, rather than sitting in a test nobody runs. They are cheap string
/// comparisons and they exist for one reason: this file is a second copy of
/// wording whose canonical form lives in a document, and an edit that
/// shortens the cannot-be-recalled clause, or that hands an `accepted` trace
/// only the gentler body, is exactly the change nobody would notice in
/// review.
enum WithdrawalCopyCheck {
    /// Returns the failures, empty when the copy still holds.
    static func failures() -> [String] {
        var problems: [String] = []

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
        if !commons.bodies.contains(WithdrawalCopy.canonicalCommonsDistributed) {
            problems.append("an accepted trace is not warned about distributed copies")
        }
        if commons.ambiguity == nil {
            problems.append("an accepted trace is shown a tier this app cannot know")
        }

        // ...and a trace that never entered the commons must not be told it
        // was excluded from exports it was never in.
        let outside = WithdrawalCopy.confirmation(for: .notInTheCommons)
        if outside.bodies != [WithdrawalCopy.canonicalNotDistributed] {
            problems.append("a not-yet-in-the-commons trace is shown the wrong tier")
        }

        // Every tier says the same thing about credit, and only that.
        for stage in [WithdrawalCopy.Stage.notInTheCommons, .inTheCommons, .unknown]
        where WithdrawalCopy.confirmation(for: stage).credit != WithdrawalCopy.creditNote {
            problems.append("a tier states something other than the verified credit note")
        }

        // A failed withdrawal must lead with the fact that nothing happened,
        // and a not-found must disclose neither existence nor ownership.
        for sentence in [
            WithdrawalCopy.accountSessionRequired,
            WithdrawalCopy.notFound,
            WithdrawalCopy.failureSentence(label: "withdraw-failed"),
        ] where !sentence.hasPrefix("Nothing was withdrawn") {
            problems.append("a failure sentence does not open by saying nothing happened")
        }
        let lowerNotFound = WithdrawalCopy.notFound.lowercased()
        if lowerNotFound.contains("belongs to") || lowerNotFound.contains("does not exist") {
            problems.append("the not-found sentence discloses existence or ownership")
        }

        // No outcome may be reported as a bare "withdrawn", and the gravest
        // one may not be reported more gently than it was confirmed.
        for reach in [
            WithdrawalReach.notDistributed, .commonsNotDistributed, .commonsDistributed,
        ] where !WithdrawalCopy.resultSentence(reach)
            .contains(WithdrawalCopy.canonicalBody(reach))
        {
            problems.append("an outcome does not carry its tier's canonical wording")
        }
        if !WithdrawalCopy.resultSentence(nil).contains("cannot be recalled") {
            problems.append("an unknown tier is reported as though nothing were distributed")
        }

        return problems
    }
}
