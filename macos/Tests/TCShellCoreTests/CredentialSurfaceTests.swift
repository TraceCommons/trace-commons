import XCTest

@testable import TCShellCore

/// The credential surface's logic, without the dylib.
///
/// The injected calls deliberately do NOT reimplement the Rust's three
/// tables. Several of them answer the OPPOSITE of the real one for every
/// input, so a surface that had quietly started deciding for itself could not
/// agree with them -- which is the only way a test in this target can tell
/// "asked the ABI" from "reproduced the ABI in Swift".
final class CredentialSurfaceTests: XCTestCase {
    func testInferenceKeyDoesNotImplyACloudSession() {
        let legacy = CredentialStatus.parse(fromJSON: #"{"state":"present"}"#)
        XCTAssertEqual(legacy.state, "present")
        XCTAssertEqual(legacy.sessionState, "")
        let keyOnly = CredentialStatus.parse(fromJSON: #"{"state":"present","session_state":"absent"}"#)
        XCTAssertEqual(keyOnly.sessionState, "absent")
        let session = CredentialStatus.parse(fromJSON: #"{"state":"present","session_state":"present"}"#)
        XCTAssertEqual(session.sessionState, "present")
        XCTAssertEqual(CredentialStatus.parse(fromJSON: #"{"session_state":true}"#).sessionState, "")
    }

    /// The 15 fields this surface added, so a test can delete each in turn.
    private static let credentialFields = [
        "credential_title", "credential_what", "credential_cost", "credential_obtain",
        "credential_cancel", "credential_forget", "credential_forget_explains",
        "credential_absent", "credential_obtaining", "credential_failed",
        "credential_cancelled", "credential_present", "credential_unknown",
        "credential_unreported", "harness_needs_credential",
    ]

    /// The shared complete payload; see `PrivateInferenceCopyFixture`.
    /// A new required property is one edit there, not five here.
    private let payload = PrivateInferenceCopyFixture.complete

    /// Decoded once, in `setUpWithError`, so a fixture that stops decoding
    /// fails THIS TEST and lets the rest of the bundle run.
    ///
    /// It used to be `XCTFail` followed by `fatalError("unreachable")`, which
    /// aborted the whole XCTest process at the first fixture failure: every
    /// remaining test in every target simply never ran, no total was printed,
    /// and the log carried one error line no matter how much was broken. A
    /// red `macOS app tests` job was unreadable for that reason -- it looked
    /// far emptier than a normal failing run, and its failure count was
    /// always about one.
    ///
    /// A throw from `setUpWithError` is reported against each test in this
    /// class in turn, which is honest: every one of them needs this payload.
    private var decodedCopy: PrivateInferenceCopy!

    override func setUpWithError() throws {
        try super.setUpWithError()
        decodedCopy = try XCTUnwrap(
            PrivateInferenceCopy.decode(fromJSON: payload),
            "the fixture payload must decode")
    }

    private func copy() -> PrivateInferenceCopy { decodedCopy }

    /// The real tables' answers, so a test that is not about the ABI can say
    /// what it is about. Only the tests that PROVE the ABI is asked use a
    /// fake that disagrees with this.
    private func calls(
        line: @escaping @Sendable (String) -> String? = { "LINE:\($0)" },
        tone: @escaping @Sendable (String) -> Int32 = { _ in 21 },
        action: @escaping @Sendable (String) -> Int32 = { _ in 30 },
        notice: @escaping @Sendable (Int32) -> String = { $0 == 0 ? "NOTICE" : "" }
    ) -> CredentialCalls {
        CredentialCalls(
            stateLine: line, stateTone: tone, action: action, harnessNotice: notice)
    }

    // MARK: - The payload is all or nothing

    /// Swift's decoder is all-or-nothing, and every one of the 15 new fields
    /// is load-bearing: the cost sentence and the forget explanation are the
    /// two a shell must never render a button without, and there is no
    /// default anywhere that could hide their absence.
    func testEveryCredentialSentenceIsRequiredRatherThanDefaulted() {
        XCTAssertNotNil(PrivateInferenceCopy.decode(fromJSON: payload))
        for field in Self.credentialFields {
            let without = payload.replacingOccurrences(
                of: "\"\(field)\":\"", with: "\"\(field)_REMOVED\":\"")
            XCTAssertNil(
                PrivateInferenceCopy.decode(fromJSON: without),
                "\(field) is not optional; a payload without it must be refused whole")
        }
    }

    // MARK: - Render, do not decide

    /// The sentence comes from the shared table.
    ///
    /// The fake answers a value no payload field holds, so a surface that
    /// picked a field itself could not produce it.
    func testTheSentenceComesFromTheSharedTableAndNotFromThisShell() {
        XCTAssertEqual(
            CredentialSurface.stateLine(
                CredentialStatus(state: "present"), copy: copy(), calls: calls()),
            "LINE:present")
    }

    /// A caught panic falls back to the payload's UNREPORTED sentence --
    /// never to `credentialAbsent`, which is a claim about what this machine
    /// holds and would be made up.
    func testASilentExportFallsBackToUnreportedAndNeverToAbsent() {
        let fallback = CredentialSurface.stateLine(
            CredentialStatus(state: "present"), copy: copy(), calls: calls(line: { _ in nil }))
        XCTAssertEqual(fallback, copy().credentialUnreported)
        XCTAssertNotEqual(fallback, copy().credentialAbsent)
    }

    /// The tone is the ABI's, not this shell's.
    ///
    /// The fake inverts the real table: it answers `.clear` for every state
    /// the Rust does not, and `.neutral` for `present`, which is the only one
    /// the Rust answers `.clear` for. A surface holding its own switch would
    /// disagree with this on every input.
    func testTheToneIsTheAbisAndNotRecoveredFromTheSentence() {
        let inverted = calls(tone: { $0 == "present" ? 20 : 22 })
        XCTAssertEqual(
            CredentialSurface.tone(CredentialStatus(state: "present"), calls: inverted), .neutral)
        for state in ["absent", "obtaining", "failed", "cancelled", "a_later_daemons_state"] {
            XCTAssertEqual(
                CredentialSurface.tone(CredentialStatus(state: state), calls: inverted), .clear,
                "\(state) must take the injected table's answer, not a Swift one")
        }
    }

    /// The button is the ABI's too, and by the same proof.
    func testTheActionIsTheAbisAndNotDecidedHere() {
        // Forget where the Rust says obtain, obtain where it says forget.
        let inverted = calls(action: { $0 == "present" ? 31 : 33 })
        XCTAssertEqual(
            CredentialSurface.action(CredentialStatus(state: "present"), calls: inverted), .obtain)
        XCTAssertEqual(
            CredentialSurface.action(CredentialStatus(state: "absent"), calls: inverted), .forget)
    }

    /// Anything unknown is no action at all. `.obtain` opens a browser and
    /// mints a key at a third party, so a code this build has never heard of
    /// must not reach it.
    func testAnUnknownActionCodeOffersNothing() {
        XCTAssertEqual(CredentialAction.fromABI(31), .obtain)
        XCTAssertEqual(CredentialAction.fromABI(32), .cancel)
        XCTAssertEqual(CredentialAction.fromABI(33), .forget)
        for stranger: Int32 in [30, 0, 1, 21, 22, 23, 24, 29, 34, 41, -1, 99] {
            XCTAssertEqual(
                CredentialAction.fromABI(stranger), .none,
                "\(stranger) must offer nothing")
        }
    }

    /// An unrecognised state borrows nothing.
    ///
    /// Asked of the REAL shape of the tables rather than a fake: the shared
    /// tables answer their own sentence and `NONE` for a label they do not
    /// know, and this checks that the surface passes that through instead of
    /// reaching for a field of its own.
    func testAnUnreadStateBorrowsNoKnownSentenceAndNoButton() {
        let unknown = copy().credentialUnknown
        let unknownTables = calls(
            line: { _ in unknown }, tone: { _ in 20 }, action: { _ in 30 })
        let status = CredentialStatus(state: "a_credential_state_from_a_later_daemon")
        let line = CredentialSurface.stateLine(status, copy: copy(), calls: unknownTables)
        XCTAssertEqual(line, copy().credentialUnknown)
        for known in [
            copy().credentialAbsent, copy().credentialObtaining, copy().credentialFailed,
            copy().credentialCancelled, copy().credentialPresent,
        ] {
            XCTAssertNotEqual(line, known, "an unread state must not borrow a known sentence")
        }
        XCTAssertEqual(CredentialSurface.tone(status, calls: unknownTables), .neutral)
        let action = CredentialSurface.action(status, calls: unknownTables)
        XCTAssertEqual(action, CredentialAction.none)
        XCTAssertNil(
            CredentialSurface.actionLabel(action, copy: copy()),
            "no button at all, not a disabled one")
    }

    // MARK: - What a button may never appear without

    /// The button's words are the payload's, and `.none` has none.
    func testEachActionTakesItsLabelFromThePayload() {
        XCTAssertEqual(CredentialSurface.actionLabel(.obtain, copy: copy()), copy().credentialObtain)
        XCTAssertEqual(CredentialSurface.actionLabel(.cancel, copy: copy()), copy().credentialCancel)
        XCTAssertEqual(CredentialSurface.actionLabel(.forget, copy: copy()), copy().credentialForget)
        XCTAssertNil(CredentialSurface.actionLabel(.none, copy: copy()))
    }

    /// The cost sentence rides on the action itself, so it cannot be
    /// separated from the button by a layout change. Obtain is the only
    /// action that opens a browser and mints a key somewhere else, and it is
    /// the only one that carries it.
    func testObtainCannotBeOfferedWithoutSayingWhatItCosts() {
        XCTAssertEqual(CredentialSurface.actionExplains(.obtain, copy: copy()), copy().credentialCost)
    }

    /// Forgetting is local, and the button says only "forget". Without this
    /// sentence a contributor reads it as a revocation, which is exactly what
    /// `handle_forget`'s `revoked: false` says it is not.
    func testForgetCannotBeOfferedWithoutSayingItIsLocal() {
        XCTAssertEqual(
            CredentialSurface.actionExplains(.forget, copy: copy()),
            copy().credentialForgetExplains)
    }

    /// Cancel stops a ceremony this app started and costs nothing; `.none`
    /// has no button to qualify. Neither may borrow the other two's
    /// sentences.
    func testTheOtherTwoActionsCarryNoExplanation() {
        XCTAssertNil(CredentialSurface.actionExplains(.cancel, copy: copy()))
        XCTAssertNil(CredentialSurface.actionExplains(CredentialAction.none, copy: copy()))
    }

    // MARK: - A control this shell can always send

    /// A Cancel with no attempt to name is offered, and is sent.
    ///
    /// The ORDINARY case, not an edge one: `near_ai_credential_status`
    /// resolves `obtaining` from the ceremony the daemon holds, needing no
    /// attempt id, so an app restarted while the daemon kept running reads
    /// `obtaining` and is offered Cancel with no id to give. The daemon now
    /// accepts an unnamed cancel and stops whatever it is running, so the
    /// button is live and the call goes out -- with the field OMITTED, never
    /// sent empty, because an empty id names no running attempt.
    func testACancelWithNoAttemptToNameIsStillSent() {
        XCTAssertTrue(CredentialSurface.cancelParams(attemptID: nil).isEmpty)
        XCTAssertTrue(CredentialSurface.cancelParams(attemptID: "").isEmpty)
        XCTAssertEqual(
            CredentialSurface.cancelParams(attemptID: "a1")["attempt_id"] as? String, "a1")

        // And it is a Cancel the state table really does offer: `obtaining`
        // reached this shell without an attempt id in the first place.
        let obtaining = CredentialStatus(state: "obtaining")
        XCTAssertNil(obtaining.attemptID)
        XCTAssertEqual(
            CredentialSurface.action(obtaining, calls: calls(action: { _ in 32 })), .cancel)
    }

    /// Holding an attempt id does not touch the state sentence.
    ///
    /// Whether this shell can name the ceremony does not make it more or less
    /// true -- `obtaining` is still exactly what is happening, and the card
    /// must go on saying so. This is what stops someone later "fixing" the
    /// sentence instead of the transport: the sentence, the tone and the
    /// offered action are all read from the state label alone, and none of
    /// the three is handed an attempt id to consult.
    func testTheStateSentenceDoesNotConsultTheAttemptID() {
        let obtaining = CredentialStatus(state: "obtaining")
        let named = CredentialStatus(
            state: "obtaining", attemptID: "a1", attemptStatus: "waiting_for_browser")
        XCTAssertTrue(CredentialSurface.cancelParams(attemptID: obtaining.attemptID).isEmpty)
        XCTAssertFalse(CredentialSurface.cancelParams(attemptID: named.attemptID).isEmpty)

        // What the call carries just changed. Nothing the contributor reads moves.
        XCTAssertEqual(
            CredentialSurface.stateLine(obtaining, copy: copy(), calls: calls()),
            CredentialSurface.stateLine(named, copy: copy(), calls: calls()))
        XCTAssertEqual(
            CredentialSurface.tone(obtaining, calls: calls()),
            CredentialSurface.tone(named, calls: calls()))
        XCTAssertEqual(
            CredentialSurface.action(obtaining, calls: calls(action: { _ in 32 })),
            CredentialSurface.action(named, calls: calls(action: { _ in 32 })))
    }

    // MARK: - The tri-state on the tool list

    /// An absent field is NOT a refused connect.
    ///
    /// Three values reach the ABI, and only `false` draws the notice. A
    /// daemon that predates the credential gate reports nothing and connects
    /// tools perfectly well; a destination the contributor runs themselves
    /// reports true. Telling either of them to sign in first would be false.
    func testAnAbsentCredentialFieldIsNotARefusedConnect() {
        XCTAssertEqual(HarnessList.credentialedABIValue(nil), -1)
        XCTAssertEqual(HarnessList.credentialedABIValue(false), 0)
        XCTAssertEqual(HarnessList.credentialedABIValue(true), 1)

        XCTAssertEqual(
            CredentialSurface.harnessNotice(credentialed: false, calls: calls()), "NOTICE")
        XCTAssertNil(CredentialSurface.harnessNotice(credentialed: true, calls: calls()))
        XCTAssertNil(CredentialSurface.harnessNotice(credentialed: nil, calls: calls()))
    }

    /// The notice is the table's answer, not a Swift branch on the tri-state.
    ///
    /// The fake answers for the ABSENT value alone, which the real table
    /// never does. A surface that decided "notice when false" in Swift would
    /// draw one here and draw nothing for -1.
    func testTheNoticeIsTheTablesAnswerAndNotABranchHere() {
        let onlyAbsent = calls(notice: { $0 < 0 ? "STRANGER" : "" })
        XCTAssertEqual(
            CredentialSurface.harnessNotice(credentialed: nil, calls: onlyAbsent), "STRANGER")
        XCTAssertNil(CredentialSurface.harnessNotice(credentialed: false, calls: onlyAbsent))
    }

    /// A list from a daemon that never sent the field leaves it absent, and
    /// does not default it to false.
    func testAListWithoutTheFieldLeavesItAbsent() {
        let without = """
            {"catalog_present":true,"harnesses":[],
             "activity":{"readable":false,"window_hours":0,"families":[]}}
            """
        XCTAssertNil(HarnessSurface.list(fromJSON: without).destinationCredentialed)
        XCTAssertEqual(HarnessSurface.list(fromJSON: without).credentialedABIValue, -1)

        let refused = """
            {"catalog_present":true,"harnesses":[],
             "activity":{"readable":false,"window_hours":0,"families":[]},
             "destination_credentialed":false}
            """
        XCTAssertEqual(HarnessSurface.list(fromJSON: refused).destinationCredentialed, false)
        XCTAssertEqual(HarnessSurface.list(fromJSON: refused).credentialedABIValue, 0)

        // And `.none` -- what an unreadable payload says -- says nothing
        // about the credential either.
        XCTAssertNil(HarnessList.none.destinationCredentialed)
    }

    // MARK: - The wire

    /// `attempt_status`, not `status`. The daemon named it that on purpose,
    /// because a field called `status` beside one called `state` is a shell
    /// reading the wrong one -- and this is the shell that would.
    func testTheStatusPayloadReadsAttemptStatusAndNotStatus() {
        let parsed = CredentialStatus.parse(
            fromJSON: """
                {"state":"obtaining","attempt_id":"A1",
                 "attempt_status":"waiting_for_browser","status":"WRONG-FIELD"}
                """)
        XCTAssertEqual(parsed.state, "obtaining")
        XCTAssertEqual(parsed.attemptID, "A1")
        XCTAssertEqual(parsed.attemptStatus, "waiting_for_browser")
    }

    /// A caller that named no attempt still gets a state, and gets neither
    /// echo. The resting state is a fact about the machine, not a secret.
    func testACallerThatNamesNoAttemptStillReadsTheState() {
        let parsed = CredentialStatus.parse(fromJSON: "{\"state\":\"absent\"}")
        XCTAssertEqual(parsed.state, "absent")
        XCTAssertNil(parsed.attemptID)
        XCTAssertNil(parsed.attemptStatus)
    }

    /// An unreadable answer is the empty label, which the shared table
    /// answers as unreported -- never as a state this shell invented.
    func testAnUnreadableStatusIsTheEmptyLabel() {
        XCTAssertEqual(CredentialStatus.parse(fromJSON: "not json").state, "")
        XCTAssertEqual(CredentialStatus.parse(fromJSON: "{}").state, "")
        XCTAssertEqual(CredentialStatus.unreported.state, "")
    }

    /// The browser URL is served once, by start. An attempt missing either
    /// half cannot be carried through -- one cannot be cancelled, the other
    /// cannot be finished -- so neither is half-accepted.
    func testAnAttemptNeedsBothItsIdAndItsUrl() {
        let full = CredentialAttempt.parse(
            fromJSON: """
                {"attempt_id":"A1","browser_url":"https://example.invalid/x",
                 "status":"waiting_for_browser"}
                """)
        XCTAssertEqual(full?.attemptID, "A1")
        XCTAssertEqual(full?.browserURL, "https://example.invalid/x")
        XCTAssertEqual(full?.status, "waiting_for_browser")

        XCTAssertNil(CredentialAttempt.parse(fromJSON: "{\"attempt_id\":\"A1\"}"))
        XCTAssertNil(
            CredentialAttempt.parse(
                fromJSON: "{\"browser_url\":\"https://example.invalid/x\"}"))
        XCTAssertNil(
            CredentialAttempt.parse(
                fromJSON: "{\"attempt_id\":\"\",\"browser_url\":\"https://example.invalid/x\"}"))
    }

    /// Status and cancel both name the attempt when there is one and nothing
    /// when there is not. Neither ever sends an empty id.
    func testTheCallsNameTheAttemptOnlyWhenThereIsOne() {
        XCTAssertTrue(CredentialSurface.statusParams(attemptID: nil).isEmpty)
        XCTAssertTrue(CredentialSurface.statusParams(attemptID: "").isEmpty)
        XCTAssertEqual(
            CredentialSurface.statusParams(attemptID: "A1")["attempt_id"] as? String, "A1")
        XCTAssertEqual(
            CredentialSurface.cancelParams(attemptID: "A1")["attempt_id"] as? String, "A1")
    }
}
