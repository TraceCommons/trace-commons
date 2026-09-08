import XCTest

@testable import TCShellCore

/// The eligibility surface's logic, without the dylib.
///
/// The injected calls deliberately do NOT reimplement the Rust's four
/// tables. Several of them answer the OPPOSITE of the real one for every
/// input, so a surface that had quietly started deciding for itself could not
/// agree with them -- which is the only way a test in this target can tell
/// "asked the ABI" from "reproduced the ABI in Swift".
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

    private let payload = """
        {"destination":"DESTINATION","subtitle":"SUBTITLE",
         "offer_title":"T","offer_what":"WHAT","offer_exposure":"EXPOSURE",
         "offer_no_repoint":"NO-REPOINT","offer_accept":"ACCEPT",
         "offer_decline":"DECLINE","offer_asked_once":"ONCE",
         "settings_title":"S-TITLE","settings_toggle":"S-TOGGLE",
         "settings_applies_at_once":"S-AT-ONCE","state_off":"S-OFF","state_unknown":"S-UNKNOWN","state_unreported":"S-UNREPORTED","state_stopping":"S-STOPPING",
         "state_running":"S-RUNNING","state_running_no_backends":"S-NO-BACKENDS","state_running_answered_elsewhere":"S-ELSEWHERE","state_running_destination_unknown":"S-DEST-UNKNOWN",
         "state_running_elsewhere":"S-ELSEWHERE","state_port_in_use":"S-PORT",
         "state_start_failed":"S-FAILED","state_crashed":"S-CRASHED",
         "quit_also_stops":"QUIT","write_unconfirmed":"UNCONFIRMED","settings_moved":"MOVED","tray_turn_off":"TRAYOFF","tray_open_to_turn_on":"TRAYON",
         "harnesses_title":"H-TITLE","harnesses_what":"H-WHAT",
         "harnesses_spend_scope":"H-SPEND-SCOPE",
         "harness_not_connected":"H-NOT-CONNECTED",
         "harness_connected_nothing_seen":"H-NOTHING-SEEN",
         "harness_answering":"H-ANSWERING","harness_connect":"H-CONNECT",
         "harness_disconnect":"H-DISCONNECT",
         "harness_preview_title":"H-PREVIEW","harness_preview_confirm":"H-CONFIRM",
         "harness_preview_cancel":"H-CANCEL","harness_slot_taken":"H-TAKEN",
         "harness_needs_restart":"H-RESTART","harnesses_none_found":"H-NONE",
         "harness_unreadable_config":"H-UNREADABLE",
         "harness_not_installed":"H-NOT-INSTALLED",
         "harness_plan_nothing_to_change":"H-NOTHING-TO-CHANGE",
         "harness_plan_entry_unusable":"H-ENTRY-UNUSABLE",
         "harness_plan_no_config_path":"H-NO-CONFIG-PATH",
         "credential_title":"C-TITLE","credential_what":"C-WHAT",
         "credential_cost":"C-COST","credential_obtain":"C-OBTAIN",
         "credential_cancel":"C-CANCEL","credential_forget":"C-FORGET",
         "credential_forget_explains":"C-FORGET-EXPLAINS",
         "credential_absent":"C-ABSENT","credential_obtaining":"C-OBTAINING",
         "credential_failed":"C-FAILED","credential_cancelled":"C-CANCELLED",
         "credential_present":"C-PRESENT","credential_unknown":"C-UNKNOWN",
         "credential_unreported":"C-UNREPORTED",
         "harness_needs_credential":"H-NEEDS-CREDENTIAL",
         "eligibility_eligible":"E-ELIGIBLE",
         "eligibility_ineligible_permanent":"E-PERMANENT",
         "eligibility_ineligible_configuration":"E-CONFIGURATION",
         "eligibility_unknown":"E-UNKNOWN",
         "eligibility_reason_no_call":"R-NO-CALL",
         "eligibility_reason_capture_off":"R-CAPTURE-OFF",
         "eligibility_reason_digest_absent":"R-DIGEST-ABSENT",
         "eligibility_reason_upstream_id_absent":"R-UPSTREAM-ID-ABSENT",
         "eligibility_reason_digest_mismatch":"R-DIGEST-MISMATCH",
         "eligibility_reason_reference_malformed":"R-REFERENCE-MALFORMED",
         "eligibility_reason_bodies_unreadable":"R-BODIES-UNREADABLE",
         "eligibility_reason_body_not_utf8":"R-BODY-NOT-UTF8",
         "eligibility_reason_body_too_large":"R-BODY-TOO-LARGE",
         "eligibility_reason_evidence_capture_off":"R-EVIDENCE-CAPTURE-OFF",
         "eligibility_reason_marker_absent":"R-MARKER-ABSENT",
         "eligibility_reason_request_malformed":"R-REQUEST-MALFORMED",
         "eligibility_reason_receipt_unavailable":"R-RECEIPT-UNAVAILABLE"}
        """

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
        reason: @escaping @Sendable (String) -> String? = { "REASON:\($0)" }
    ) -> EligibilityCalls {
        EligibilityCalls(
            stateLine: line, stateTone: tone, control: control, reasonLine: reason)
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
        XCTAssertEqual(
            EligibilitySurface.withheldCount(group, eligibility: { $0 }, calls: table), 3)
    }

    /// Order survives, so a caller can report what it sent in the order the
    /// folder showed it.
    func testAGroupLevelSubmitKeepsTheFoldersOrder() {
        let table = calls(control: { $0 == "eligible" ? 51 : 50 })
        let group = [
            ContributionEligibility(state: "ineligible_permanent"),
            ContributionEligibility(state: "eligible", reason: nil),
            ContributionEligibility(state: "unknown"),
            ContributionEligibility(state: "eligible", reason: nil),
        ]
        let sendable = EligibilitySurface.contributable(
            group.enumerated().map { ($0.offset, $0.element) },
            eligibility: { $0.1 }, calls: table)
        XCTAssertEqual(sendable.map(\.0), [1, 3])
    }

    /// An invited contributor's folder is untouched: every entry carries no
    /// key, every entry is still sent, and nothing is withheld.
    func testAFolderWithNoEligibilityQuestionSubmitsWhole() {
        let refusesEverything = calls(control: { _ in 50 })
        let group: [ContributionEligibility?] = [nil, nil, nil]
        XCTAssertEqual(
            EligibilitySurface.contributable(
                group, eligibility: { $0 }, calls: refusesEverything
            ).count, 3)
        XCTAssertEqual(
            EligibilitySurface.withheldCount(
                group, eligibility: { $0 }, calls: refusesEverything), 0)
    }

    /// A folder with nothing eligible in it sends nothing. Not an error --
    /// the rows are all still shown, one level in, each with its sentence.
    func testAFolderWithNothingEligibleSendsNothing() {
        let table = calls(control: { $0 == "eligible" ? 51 : 50 })
        let group = [
            ContributionEligibility(state: "ineligible_permanent"),
            ContributionEligibility(state: "unknown"),
        ]
        XCTAssertTrue(
            EligibilitySurface.contributable(group, eligibility: { $0 }, calls: table).isEmpty)
    }

    /// Which entries a group submit covers is the TABLE's decision, not a
    /// Swift filter on the state string.
    ///
    /// The fake inverts the real table exactly, so a surface that had
    /// reimplemented "eligible means sendable" would answer the other two.
    func testWhichEntriesAGroupSubmitCoversIsTheTablesDecision() {
        let inverted = calls(control: { $0 == "eligible" ? 50 : 51 })
        let group = [
            ContributionEligibility(state: "eligible"),
            ContributionEligibility(state: "ineligible_permanent"),
        ]
        let sendable = EligibilitySurface.contributable(
            group, eligibility: { $0 }, calls: inverted)
        XCTAssertEqual(sendable.map(\.state), ["ineligible_permanent"])
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
