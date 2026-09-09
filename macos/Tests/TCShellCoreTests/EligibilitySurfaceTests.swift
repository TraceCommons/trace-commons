import XCTest

@testable import TCShellCore

/// The eligibility surface's logic, without the dylib.
///
/// The injected calls deliberately do NOT reimplement the Rust's four
/// tables. Several of them answer the OPPOSITE of the real one for every
/// input, so a surface that had quietly started deciding for itself could not
/// agree with them -- which is the only way a test in this target can tell
/// "asked the ABI" from "reproduced the ABI in Swift".
/// Captures what the group-control table was actually handed, so a test can
/// assert the ARGUMENT rather than the outcome. Absent-as-negative is only
/// observable that way: a fake that answered correctly for both spellings
/// would hide the bug.
private final class Recorder: @unchecked Sendable {
    private(set) var pending: Int64 = 0
    private(set) var contributable: Int64 = 0

    func record(pending: Int64, contributable: Int64) {
        self.pending = pending
        self.contributable = contributable
    }
}

final class EligibilitySurfaceTests: XCTestCase {
    /// The 17 fields this surface added, so a test can delete each in turn.
    private static let eligibilityFields = [
        "eligibility_eligible", "eligibility_ineligible_permanent",
        "eligibility_ineligible_configuration", "eligibility_unknown",
        "eligibility_reason_no_call", "eligibility_reason_capture_off",
        "eligibility_reason_digest_absent", "eligibility_reason_upstream_id_absent",
        "eligibility_reason_digest_mismatch", "eligibility_reason_reference_malformed",
        "eligibility_reason_bodies_unreadable", "eligibility_reason_body_not_utf8",
        "eligibility_reason_body_too_large", "eligibility_reason_evidence_capture_off",
        "eligibility_reason_marker_absent", "eligibility_reason_request_malformed",
        "eligibility_reason_receipt_unavailable",
    ]

    /// The shared complete payload; see `PrivateInferenceCopyFixture`.
    /// A new required property is one edit there, not five here.
    private let payload = PrivateInferenceCopyFixture.complete

    /// Decoded once, in `setUpWithError`, so a fixture that stops decoding
    /// fails THIS TEST and lets the rest of the bundle run. See
    /// `CredentialSurfaceTests` for why that matters to a red CI log.
    private var decodedCopy: PrivateInferenceCopy!

    override func setUpWithError() throws {
        try super.setUpWithError()
        decodedCopy = try XCTUnwrap(
            PrivateInferenceCopy.decode(fromJSON: payload),
            "the fixture payload must decode")
    }

    /// Named `words` rather than `copy` because `copy()` on an `XCTestCase`
    /// collides with `NSObject.copy()`.
    private func words() -> PrivateInferenceCopy { decodedCopy }

    /// The real tables' answers, so a test that is not about the ABI can say
    /// what it is about. Only the tests that PROVE the ABI is asked use a
    /// fake that disagrees with this.
    private func calls(
        line: @escaping @Sendable (String) -> String? = { "LINE:\($0)" },
        tone: @escaping @Sendable (String) -> Int32 = { _ in 20 },
        control: @escaping @Sendable (String) -> Int32 = { _ in 50 },
        reason: @escaping @Sendable (String) -> String? = { "REASON:\($0)" },
        withheld: @escaping @Sendable (Int64) -> String? = { $0 > 0 ? "WITHHELD:\($0)" : "" },
        groupControl: @escaping @Sendable (Int64, Int64) -> Int32 = { pending, contributable in
            // The real table's rule, for tests that are not about it: a
            // negative contributable means the key was absent and the
            // control rides on `pending` alone.
            (contributable < 0 ? pending > 0 : contributable > 0) ? 51 : 50
        }
    ) -> EligibilityCalls {
        EligibilityCalls(
            stateLine: line, stateTone: tone, control: control, reasonLine: reason,
            withheldLine: withheld, groupControl: groupControl)
    }

    // MARK: - The payload is all or nothing

    /// Swift's decoder is all-or-nothing, and every one of the 17 new fields
    /// is load-bearing: the four state sentences are the only thing a row
    /// with no send button says about why, and there is no default anywhere
    /// that could hide their absence.
    func testEveryEligibilitySentenceIsRequiredRatherThanDefaulted() {
        XCTAssertNotNil(PrivateInferenceCopy.decode(fromJSON: payload))
        for field in Self.eligibilityFields {
            let without = payload.replacingOccurrences(
                of: "\"\(field)\":\"", with: "\"\(field)_REMOVED\":\"")
            XCTAssertNil(
                PrivateInferenceCopy.decode(fromJSON: without),
                "\(field) is not optional; a payload without it must be refused whole")
        }
    }

    // MARK: - Absent is not a state

    /// The distinction the whole contract turns on.
    ///
    /// An invited contributor's entry carries NO `eligibility` key. That is
    /// not `unknown`, not null, and not a state -- it is a contributor who
    /// has no eligibility question, and their row must render exactly as it
    /// did before this surface existed.
    func testAnAbsentKeyIsNotAState() {
        XCTAssertNil(ContributionEligibility.parse([:]))
        XCTAssertNil(ContributionEligibility.parse(nil))
        XCTAssertNil(ContributionEligibility.parse(["entry_id": "e1"]))
        // A JSON null decodes to NSNull, which is the daemon saying nothing
        // about eligibility just as an absent key is.
        XCTAssertNil(ContributionEligibility.parse(["eligibility": NSNull()]))
    }

    /// `unknown` ARRIVES ON THE WIRE and is a real state. Degrading the
    /// absence into it would put a caveat on work that carries none;
    /// degrading it into the absence would silently drop a real answer.
    func testUnknownOnTheWireIsNotTheSameAsAnAbsentKey() {
        let unknown = ContributionEligibility.parse(["eligibility": "unknown"])
        XCTAssertEqual(unknown?.state, "unknown")
        XCTAssertNil(ContributionEligibility.parse([:]))
    }

    /// An absent key draws no sentence at all, rather than the unknown one.
    func testAnAbsentKeyDrawsNoSentenceAndNoTone() {
        XCTAssertNil(EligibilitySurface.stateLine(nil, copy: words(), calls: calls()))
        XCTAssertNil(EligibilitySurface.tone(nil, calls: calls()))
        XCTAssertNil(EligibilitySurface.reasonLine(nil, calls: calls()))
    }

    /// An absent key KEEPS the send control. Everything in an invited
    /// contributor's queue is contributable, and withholding the button from
    /// them would break the surface for the population it was never about.
    ///
    /// Proved against a table that refuses EVERY state, so a surface that had
    /// started asking the table for `nil` would fail here.
    func testAnAbsentKeyStillOffersTheSendControl() {
        let refusesEverything = calls(control: { _ in 50 })
        XCTAssertEqual(EligibilitySurface.control(nil, calls: refusesEverything), .contribute)
        XCTAssertTrue(EligibilitySurface.offersContribute(nil, calls: refusesEverything))
    }

    // MARK: - The sentence is the ABI's

    /// The state sentence comes back from the table, not from a `switch`
    /// here. The fake returns a string no branch in Swift could produce.
    func testTheStateSentenceIsWhateverTheTableAnswered() {
        let line = EligibilitySurface.stateLine(
            ContributionEligibility(state: "eligible"),
            copy: words(),
            calls: calls(line: { _ in "FROM-THE-TABLE" }))
        XCTAssertEqual(line, "FROM-THE-TABLE")
    }

    /// A caught panic falls back to the payload's UNKNOWN sentence, never to
    /// an ineligibility: a state this build could not read is not evidence
    /// about a contributor's session.
    func testACaughtPanicFallsBackToUnknownAndNotToAnIneligibility() {
        let line = EligibilitySurface.stateLine(
            ContributionEligibility(state: "ineligible_permanent"),
            copy: words(),
            calls: calls(line: { _ in nil }))
        XCTAssertEqual(line, words().eligibilityUnknown)
        XCTAssertNotEqual(line, words().eligibilityIneligiblePermanent)
    }

    /// An unrecognised state borrows no other state's sentence.
    ///
    /// Asserted against the four real sentences rather than against the
    /// fake's, because the failure worth catching is a shell that had picked
    /// `eligibilityIneligiblePermanent` for a label it did not know.
    func testAnUnrecognisedStateBorrowsNoOtherStatesSentence() {
        let c = words()
        // The state table's own arm for an unfamiliar label, mimicked here
        // exactly as the Rust behaves: anything not one of the three named
        // states answers the unknown sentence.
        let table = calls(line: { label in
            switch label {
            case "eligible": return c.eligibilityEligible
            case "ineligible_permanent": return c.eligibilityIneligiblePermanent
            case "ineligible_configuration": return c.eligibilityIneligibleConfiguration
            default: return c.eligibilityUnknown
            }
        })
        for label in ["", "a_state_from_a_later_daemon", "ELIGIBLE", "ineligible"] {
            let line = EligibilitySurface.stateLine(
                ContributionEligibility(state: label), copy: c, calls: table)
            XCTAssertEqual(line, c.eligibilityUnknown, label)
            XCTAssertNotEqual(line, c.eligibilityEligible, label)
            XCTAssertNotEqual(line, c.eligibilityIneligiblePermanent, label)
            XCTAssertNotEqual(line, c.eligibilityIneligibleConfiguration, label)
        }
    }

    // MARK: - The tone is the ABI's

    /// The tone comes from the table and is never recovered by reading the
    /// sentence.
    ///
    /// The fake answers CLEAR for every state including the permanent
    /// ineligibility, which no correct Swift branch would ever produce. A
    /// surface computing its own tone could not agree with it.
    func testTheToneIsWhateverTheTableAnsweredAndNotComputedHere() {
        let alwaysClear = calls(tone: { _ in 22 })
        for state in ["eligible", "ineligible_permanent", "ineligible_configuration", "unknown"] {
            XCTAssertEqual(
                EligibilitySurface.tone(
                    ContributionEligibility(state: state), calls: alwaysClear),
                .clear, state)
        }
        let alwaysRefused = calls(tone: { _ in 24 })
        XCTAssertEqual(
            EligibilitySurface.tone(
                ContributionEligibility(state: "eligible"), calls: alwaysRefused),
            .refused)
    }

    /// A tone value this build has never heard of is neutral, and the
    /// control range is not a tone range: a shell that cross-wired the two
    /// mappers would paint a row's colour from a button code.
    func testTheControlRangeAndTheToneRangeDoNotOverlap() {
        for tone: Int32 in [21, 22, 23, 24] {
            XCTAssertEqual(
                ContributionControl.fromABI(tone), .none, "tone \(tone) is not a control")
        }
        for control: Int32 in [50, 51] {
            XCTAssertEqual(
                PrivateInferenceTone.fromABI(control), .neutral,
                "control \(control) is not a tone")
        }
        // And not the credential surface's range either: both have a
        // "nothing" member and they govern different controls.
        for action: Int32 in [31, 32, 33] {
            XCTAssertEqual(
                ContributionControl.fromABI(action), .none,
                "credential action \(action) is not a contribution control")
        }
        XCTAssertEqual(CredentialAction.fromABI(51), .none)
    }

    // MARK: - The control is the ABI's

    /// Which rows get a send button is the table's decision.
    ///
    /// The fake OFFERS the button for `ineligible_permanent` and WITHHOLDS
    /// it for `eligible` -- the exact inverse of the real table. A surface
    /// that had its own `switch` would answer the real way and fail here.
    func testWhichRowsGetASendControlIsTheTablesDecision() {
        let inverted = calls(control: { $0 == "ineligible_permanent" ? 51 : 50 })
        XCTAssertEqual(
            EligibilitySurface.control(
                ContributionEligibility(state: "ineligible_permanent"), calls: inverted),
            .contribute)
        XCTAssertEqual(
            EligibilitySurface.control(
                ContributionEligibility(state: "eligible"), calls: inverted),
            .none)
    }

    /// A control code this build cannot read offers nothing. Neutral is not
    /// the safe direction here: the dangerous value is `.contribute`, which
    /// sends a contributor's work and has it refused.
    func testAnUnreadableControlCodeOffersNothing() {
        for code: Int32 in [0, -1, 49, 52, 99] {
            XCTAssertEqual(
                EligibilitySurface.control(
                    ContributionEligibility(state: "eligible"), calls: calls(control: { _ in code })),
                .none, "\(code)")
        }
    }

    // MARK: - A sheet that is already open

    /// The defect GTK found in its own shell, in the shape macOS can have
    /// it: not a widget poked back to sensitive, but a gate applied once at
    /// open time and never re-consulted.
    ///
    /// A sheet holds the entry it was handed. When a snapshot downgrades
    /// that row -- which is exactly what a submit-time failure written back
    /// into it produces -- the sheet must gate on the queue's copy, not on
    /// the one it opened with.
    func testAnOpenSheetGatesOnTheQueuesCopyNotItsOpeningCopy() {
        struct Row { let id: String; let eligibility: ContributionEligibility? }
        let opened = Row(id: "e1", eligibility: ContributionEligibility(state: "eligible"))
        let downgraded = Row(
            id: "e1",
            eligibility: ContributionEligibility(
                state: "ineligible_permanent", reason: "digest_mismatch"))
        let live = EligibilitySurface.current(
            opened, in: [Row(id: "e0", eligibility: nil), downgraded], id: \.id)
        XCTAssertEqual(live.eligibility?.state, "ineligible_permanent")
        XCTAssertEqual(live.eligibility?.reason, "digest_mismatch")

        let table = calls(control: { $0 == "eligible" ? 51 : 50 })
        XCTAssertFalse(
            EligibilitySurface.offersContribute(live.eligibility, calls: table),
            "a row the queue has downgraded must not still offer to send")
        // And the opening copy, which is what a sheet reading `entry`
        // directly would have gated on, still says yes -- so this test
        // fails if the resolution is dropped.
        XCTAssertTrue(EligibilitySurface.offersContribute(opened.eligibility, calls: table))
    }

    /// An upgrade is honoured too. The rule is "read the queue", not
    /// "assume the worst": a row the daemon re-evaluated into eligibility
    /// must become sendable without closing and reopening the sheet.
    func testAnOpenSheetAlsoSeesARowBecomeEligible() {
        struct Row { let id: String; let eligibility: ContributionEligibility? }
        let opened = Row(id: "e1", eligibility: ContributionEligibility(state: "unknown"))
        let upgraded = Row(id: "e1", eligibility: ContributionEligibility(state: "eligible"))
        let table = calls(control: { $0 == "eligible" ? 51 : 50 })
        XCTAssertTrue(
            EligibilitySurface.offersContribute(
                EligibilitySurface.current(opened, in: [upgraded], id: \.id).eligibility,
                calls: table))
    }

    /// A row the queue no longer lists keeps its held copy. It has just
    /// been approved or dismissed and is leaving the screen; blanking its
    /// sentence on the way out would be a flicker saying something it does
    /// not mean.
    func testARowTheQueueNoLongerListsKeepsItsHeldCopy() {
        struct Row: Equatable { let id: String; let eligibility: ContributionEligibility? }
        let opened = Row(id: "e1", eligibility: ContributionEligibility(state: "eligible"))
        XCTAssertEqual(EligibilitySurface.current(opened, in: [], id: \.id), opened)
        XCTAssertEqual(
            EligibilitySurface.current(
                opened, in: [Row(id: "e2", eligibility: nil)], id: \.id),
            opened)
    }

    /// Resolution is by id, never by position: the queue reorders and
    /// shrinks between snapshots, and an index would hand the sheet a
    /// different session's answer.
    func testResolutionIsByIdAndNotByPosition() {
        struct Row { let id: String; let eligibility: ContributionEligibility? }
        let opened = Row(id: "e3", eligibility: ContributionEligibility(state: "eligible"))
        let queue = [
            Row(id: "e9", eligibility: ContributionEligibility(state: "ineligible_permanent")),
            Row(id: "e3", eligibility: ContributionEligibility(state: "unknown")),
        ]
        XCTAssertEqual(
            EligibilitySurface.current(opened, in: queue, id: \.id).eligibility?.state, "unknown")
    }

    // MARK: - A group-level submit means "all eligible"

    /// The ruling, at the seam a folder header uses.
    ///
    /// One button approving a whole folder by project id is this surface's
    /// own defect one layer up, and worse there: a contributor who pressed
    /// it never saw the sessions it sent.
    func testAGroupLevelSubmitCoversTheEligibleOnly() {
        let table = calls(control: { $0 == "eligible" ? 51 : 50 })
        let group: [ContributionEligibility?] = [
            ContributionEligibility(state: "eligible"),
            ContributionEligibility(state: "ineligible_permanent", reason: "no_inference_call"),
            ContributionEligibility(state: "eligible"),
            ContributionEligibility(state: "unknown"),
            ContributionEligibility(state: "ineligible_configuration", reason: "capture_off"),
        ]
        let sendable = EligibilitySurface.contributable(
            group, eligibility: { $0 }, calls: table)
        XCTAssertEqual(sendable.count, 2)
        XCTAssertTrue(sendable.allSatisfy { $0?.state == "eligible" })
    }

    /// The counts come from the DAEMON now, not from this shell walking the
    /// folder. Its row says three of seven; the button says three.
    func testTheButtonCountsWhatTheDaemonSaysIsContributable() {
        let offer = EligibilitySurface.groupSubmit(
            pendingCount: 7, contributableCount: 3, fallbackPending: 99, calls: calls())
        XCTAssertEqual(offer.count, 3)
        XCTAssertTrue(offer.offersContribute)
        XCTAssertEqual(offer.withheldLine, "WITHHELD:4")
    }

    /// **Absent is spelled to the table as a NEGATIVE, never as zero.**
    ///
    /// The trap this pins: passing `0` for an absent `contributable_count`
    /// refuses the control to an invited contributor whose sessions are all
    /// perfectly sendable. The fake records what it was handed, so this
    /// asserts the actual argument rather than the outcome.
    func testAnAbsentContributableCountReachesTheTableAsANegative() {
        let seen = Recorder()
        _ = EligibilitySurface.groupSubmit(
            pendingCount: 7, contributableCount: nil, fallbackPending: 99,
            calls: calls(groupControl: { pending, contributable in
                seen.record(pending: pending, contributable: contributable)
                return 51
            }))
        XCTAssertEqual(seen.pending, 7)
        XCTAssertLessThan(seen.contributable, 0, "absent must not be spelled as 0")
    }

    /// And an invited contributor keeps their control and gets no withheld
    /// line: there is no gap to explain.
    func testAnAbsentContributableCountSubmitsTheFolderWhole() {
        let offer = EligibilitySurface.groupSubmit(
            pendingCount: 7, contributableCount: nil, fallbackPending: 99, calls: calls())
        XCTAssertEqual(offer.count, 7)
        XCTAssertTrue(offer.offersContribute)
        XCTAssertNil(offer.withheldLine)
    }

    /// **At zero the header offers no control, and says why instead.**
    ///
    /// Disabling was ratified and reversed: the withheld line now answers
    /// what a dead button was there to communicate, and an inert control
    /// with its own explanation beside it is worse than none.
    ///
    /// The sentence is the load-bearing half, so it is asserted here rather
    /// than left to the control test: removing the button WITHOUT it would
    /// leave the folder silent about five sessions it is showing.
    func testAFolderWithNothingContributableOffersNoControlAndSaysWhy() {
        let offer = EligibilitySurface.groupSubmit(
            pendingCount: 5, contributableCount: 0, fallbackPending: 5, calls: calls())
        XCTAssertFalse(offer.offersContribute, "nothing here can be sent")
        XCTAssertEqual(
            offer.withheldLine, "WITHHELD:5",
            "and the folder must still say so -- the sentence is what replaces the button")
    }

    /// The three cases the contract turns on, stated together so the
    /// absent/zero distinction cannot drift apart across tests.
    ///
    /// | contributable | count | control |
    /// | absent  | pending | offered  |
    /// | 0       | 0       | withheld |
    /// | 3 of 7  | 3       | offered  |
    func testAbsentZeroAndPositiveAreThreeDifferentAnswers() {
        let absent = EligibilitySurface.groupSubmit(
            pendingCount: 7, contributableCount: nil, fallbackPending: 7, calls: calls())
        XCTAssertEqual(absent.count, 7)
        XCTAssertTrue(absent.offersContribute)
        XCTAssertNil(absent.withheldLine)

        let zero = EligibilitySurface.groupSubmit(
            pendingCount: 7, contributableCount: 0, fallbackPending: 7, calls: calls())
        XCTAssertEqual(zero.count, 0)
        XCTAssertFalse(zero.offersContribute)
        XCTAssertEqual(zero.withheldLine, "WITHHELD:7")

        let positive = EligibilitySurface.groupSubmit(
            pendingCount: 7, contributableCount: 3, fallbackPending: 7, calls: calls())
        XCTAssertEqual(positive.count, 3)
        XCTAssertTrue(positive.offersContribute)
        XCTAssertEqual(positive.withheldLine, "WITHHELD:4")
    }

    /// Whether the control may be pressed is the TABLE's answer, never a
    /// comparison of the count to zero here.
    ///
    /// The fake inverts the real rule exactly, so a surface that had written
    /// `count > 0` would disagree with it.
    func testWhetherTheGroupControlIsOfferedIsTheTablesDecision() {
        let inverted = calls(groupControl: { _, contributable in contributable > 0 ? 50 : 51 })
        XCTAssertFalse(
            EligibilitySurface.groupSubmit(
                pendingCount: 7, contributableCount: 3, fallbackPending: 7, calls: inverted
            ).offersContribute)
        XCTAssertTrue(
            EligibilitySurface.groupSubmit(
                pendingCount: 7, contributableCount: 0, fallbackPending: 7, calls: inverted
            ).offersContribute)
        // And a control code this build cannot read offers nothing.
        for code: Int32 in [0, -1, 49, 52, 99] {
            XCTAssertFalse(
                EligibilitySurface.groupSubmit(
                    pendingCount: 7, contributableCount: 3, fallbackPending: 7,
                    calls: calls(groupControl: { _, _ in code })
                ).offersContribute, "\(code)")
        }
    }

    /// A project the queue is showing before `list_projects` answered for it
    /// still draws its folder, on what the queue itself says.
    func testAProjectWithNoRowYetFallsBackToTheQueuesOwnCount() {
        let offer = EligibilitySurface.groupSubmit(
            pendingCount: nil, contributableCount: nil, fallbackPending: 4, calls: calls())
        XCTAssertEqual(offer.count, 4)
        XCTAssertTrue(offer.offersContribute)
        XCTAssertNil(offer.withheldLine)
    }

    /// The withheld sentence is the SHARED table's, never authored here.
    func testTheWithheldSentenceIsTheTablesAndAnEmptyOneDrawsNothing() {
        XCTAssertEqual(
            EligibilitySurface.groupSubmit(
                pendingCount: 9, contributableCount: 2,
                fallbackPending: 9, calls: calls(withheld: { _ in "FROM-THE-TABLE" })
            ).withheldLine, "FROM-THE-TABLE")
        for empty in ["", nil] {
            XCTAssertNil(
                EligibilitySurface.groupSubmit(
                    pendingCount: 9, contributableCount: 2,
                    fallbackPending: 9, calls: calls(withheld: { _ in empty })
                ).withheldLine)
        }
    }

    // MARK: - The press decides what is sent

    /// Draw time decides what is OFFERED; the press decides what is SENT,
    /// and only the second is load-bearing.
    ///
    /// A snapshot landing between the render that armed a control and the
    /// tap that fires it leaves the tap acting on what was drawn. The
    /// press-time question is asked against the queue as it is now.
    func testThePressIsRefusedOnARowTheQueueHasDowngraded() {
        struct Row { let id: String; let eligibility: ContributionEligibility? }
        let table = calls(control: { $0 == "eligible" ? 51 : 50 })
        let armed = Row(id: "e1", eligibility: ContributionEligibility(state: "eligible"))
        let queue = [
            Row(
                id: "e1",
                eligibility: ContributionEligibility(
                    state: "ineligible_permanent", reason: "digest_mismatch"))
        ]
        XCTAssertFalse(
            EligibilitySurface.mayProceed(
                armed, in: queue, id: \.id, eligibility: { $0.eligibility }, calls: table),
            "the queue downgraded this row after it was drawn armed")
        // The copy the control was drawn from still says yes, which is what
        // makes the window a real one.
        XCTAssertTrue(
            EligibilitySurface.offersContribute(armed.eligibility, calls: table))
    }

    /// An ordinary press on a row the queue still calls eligible proceeds,
    /// and so does one on a row with no eligibility question at all.
    func testAnOrdinaryPressProceeds() {
        struct Row { let id: String; let eligibility: ContributionEligibility? }
        let table = calls(control: { $0 == "eligible" ? 51 : 50 })
        let eligible = Row(id: "e1", eligibility: ContributionEligibility(state: "eligible"))
        XCTAssertTrue(
            EligibilitySurface.mayProceed(
                eligible, in: [eligible], id: \.id, eligibility: { $0.eligibility },
                calls: table))
        let invited = Row(id: "e2", eligibility: nil)
        XCTAssertTrue(
            EligibilitySurface.mayProceed(
                invited, in: [invited], id: \.id, eligibility: { $0.eligibility },
                calls: table))
    }

    /// A row that left the queue between the draw and the press keeps its
    /// held answer, so an approval already in flight is not refused by a
    /// list that has moved on.
    func testAPressOnARowTheQueueNoLongerListsUsesItsHeldAnswer() {
        struct Row { let id: String; let eligibility: ContributionEligibility? }
        let table = calls(control: { $0 == "eligible" ? 51 : 50 })
        let held = Row(id: "e1", eligibility: ContributionEligibility(state: "eligible"))
        XCTAssertTrue(
            EligibilitySurface.mayProceed(
                held, in: [], id: \.id, eligibility: { $0.eligibility }, calls: table))
    }

    // MARK: - An unanticipated tone still says something

    /// A tone this build has never seen must not silence the row.
    ///
    /// The sentence is drawn on the presence of the SENTENCE, never on the
    /// tone being one of the two the surface anticipates. A `_HELD` or
    /// `_REFUSED` from a later daemon therefore still draws its sentence,
    /// in the tone the shared mapper gives it. A shell that renders nothing
    /// for an unanticipated tone is the same silent dead end this whole
    /// surface exists to remove.
    func testAnUnanticipatedToneStillDrawsTheSentence() {
        for code: Int32 in [21, 24, 0, 99, -1] {
            let line = EligibilitySurface.stateLine(
                ContributionEligibility(state: "eligible"),
                copy: words(),
                calls: calls(tone: { _ in code }))
            XCTAssertNotNil(line, "tone \(code) must not silence the row")
            XCTAssertFalse(line?.isEmpty ?? true, "tone \(code)")
        }
        // And the tone itself is carried through rather than flattened to
        // one of the two arms this surface anticipates.
        XCTAssertEqual(
            EligibilitySurface.tone(
                ContributionEligibility(state: "eligible"), calls: calls(tone: { _ in 21 })),
            .held)
        XCTAssertEqual(
            EligibilitySurface.tone(
                ContributionEligibility(state: "eligible"), calls: calls(tone: { _ in 24 })),
            .refused)
    }

    // MARK: - The reason

    /// The reason sentence is the table's, and an empty answer renders
    /// nothing rather than a blank line.
    func testAnEmptyReasonSentenceRendersNothing() {
        XCTAssertNil(
            EligibilitySurface.reasonLine(
                ContributionEligibility(state: "ineligible_permanent", reason: "who_knows"),
                calls: calls(reason: { _ in "" })))
        XCTAssertEqual(
            EligibilitySurface.reasonLine(
                ContributionEligibility(state: "ineligible_permanent", reason: "no_inference_call"),
                calls: calls(reason: { _ in "FROM-THE-TABLE" })),
            "FROM-THE-TABLE")
    }

    /// A caught panic on the reason table renders nothing. That is
    /// deliberately different from the state line, which still has to say
    /// something: an unknown reason has nothing honest to say.
    func testACaughtPanicOnTheReasonTableRendersNothing() {
        XCTAssertNil(
            EligibilitySurface.reasonLine(
                ContributionEligibility(state: "ineligible_permanent", reason: "no_inference_call"),
                calls: calls(reason: { _ in nil })))
    }

    /// An `eligible` row carries no reason at all -- there is nothing to
    /// explain -- and an entry whose reason arrived empty is the same.
    func testAnEligibleRowCarriesNoReason() {
        XCTAssertNil(
            EligibilitySurface.reasonLine(
                ContributionEligibility(state: "eligible"), calls: calls()))
        XCTAssertNil(ContributionEligibility.parse(
            ["eligibility": "eligible", "eligibility_reason": ""])?.reason)
    }
}
