import XCTest

@testable import TCBridge
@testable import TCShellCore

/// The balance row as it really crosses the C ABI.
///
/// Links the dylib. What this can assert and `TCShellCoreTests` cannot is
/// that the sentences, the tone and the one button this shell renders are the
/// Rust's own -- so this shell, the GTK shell and the Windows shell say one
/// thing about somebody's money.
///
/// The two facts here that no fixture can establish: a null amount really
/// does come back as no figure from the real formatter, and a zero really
/// does come back as "$0.00". Those are the same call with one argument
/// changed, and the whole reason `present` is a separate argument.
final class BalanceExportTests: XCTestCase {
    /// Every state the daemon can report, plus the two this build must
    /// handle without having been told about them.
    private static let known = [
        "known", "no_session", "session_expired", "no_organization", "unavailable",
    ]

    private func payload() throws -> PrivateInferenceCopy {
        let json = try XCTUnwrap(TCPrivateInference.copyJSON())
        return try XCTUnwrap(PrivateInferenceCopy.decode(fromJSON: json))
    }

    /// The production wiring, verbatim -- the same closures `AppModel`
    /// installs, so this exercises what ships rather than a parallel path.
    private func calls() -> BalanceCalls {
        BalanceCalls(
            stateLine: { TCNearAiBalance.stateLine(state: $0) },
            stateTone: { TCNearAiBalance.stateTone(state: $0) },
            action: { TCNearAiBalance.action(state: $0) },
            amount: { TCNearAiBalance.amount(present: $0, nanos: $1, scale: $2) },
            remainingLine: { TCNearAiBalance.remainingLine(present: $0, nanos: $1, scale: $2) },
            limitLine: { TCNearAiBalance.limitLine(present: $0, nanos: $1, scale: $2) },
            spentLine: { TCNearAiBalance.spentLine(present: $0, nanos: $1, scale: $2) },
            observedLine: { TCNearAiBalance.observedLine(secondsAgo: $0) })
    }

    private func status(
        _ state: String, remaining: Int64? = nil, limit: Int64? = nil, spent: Int64? = nil,
        scale: UInt8 = 9
    ) -> BalanceStatus {
        BalanceStatus(
            state: state, remainingNanos: remaining, spendLimitNanos: limit,
            totalSpentNanos: spent, scale: scale, observedAt: nil)
    }

    // MARK: - Null versus zero, against the real formatter

    /// **The claim this file exists to make.** Against the Rust itself: a
    /// null renders no figure, and a zero renders $0.00.
    ///
    /// A shell that folded absence onto the integer would produce the same
    /// string for both, and every other assertion about this row would still
    /// pass.
    func testANullRendersNoFigureAndAZeroRendersRealMoney() throws {
        let absent = BalanceSurface.remaining(
            status("known", remaining: nil), copy: try payload(), calls: calls())
        let zero = BalanceSurface.remaining(
            status("known", remaining: 0), copy: try payload(), calls: calls())

        XCTAssertEqual(zero, .figure("$0.00"))
        XCTAssertNotEqual(absent, zero)
        if case .figure(let figure) = absent {
            XCTFail("a null balance rendered the figure \(figure)")
        }
        // And the sentence it gets instead says the account is UNCAPPED --
        // not that it is empty.
        XCTAssertEqual(absent, .sentence(try payload().balanceNoRemaining))
    }

    /// `present == 0` on the remaining line is "no ceiling", the ordinary
    /// case, and never "$0.00 left".
    func testAnUncappedAccountIsToldItHasNoLimitRatherThanNothingLeft() throws {
        let sentence = try XCTUnwrap(
            TCNearAiBalance.remainingLine(present: 0, nanos: 0, scale: 9))
        XCTAssertEqual(sentence, try payload().balanceNoRemaining)
        XCTAssertFalse(sentence.contains("$0.00"))
        XCTAssertFalse(sentence.isEmpty, "the remaining line does NOT drop out when absent")
    }

    /// The bare amount, at the ABI. A null is the empty string; a zero is
    /// money; and they are not the same string.
    func testTheBareAmountDistinguishesAbsenceFromZero() {
        XCTAssertEqual(TCNearAiBalance.amount(present: 0, nanos: 0, scale: 9), "")
        XCTAssertEqual(TCNearAiBalance.amount(present: 1, nanos: 0, scale: 9), "$0.00")
        XCTAssertEqual(TCNearAiBalance.amount(present: 1, nanos: 8_500_000_000, scale: 9), "$8.50")
    }

    /// An overdrawn account keeps its sign, which is the reason absence
    /// could not be folded onto an out-of-range integer in the first place.
    func testAnOverdrawnAccountRendersAsADebt() {
        let figure = TCNearAiBalance.amount(present: 1, nanos: -1_000_000_000, scale: 9)
        XCTAssertEqual(figure, "-$1.00")
    }

    /// The scale is honoured, so a daemon that changes it moves the figure.
    func testTheWiresScaleDecidesTheFigure() {
        XCTAssertEqual(TCNearAiBalance.amount(present: 1, nanos: 8_500_000_000, scale: 9), "$8.50")
        XCTAssertEqual(
            TCNearAiBalance.amount(present: 1, nanos: 8_500_000_000, scale: 6), "$8500.00")
        XCTAssertEqual(TCNearAiBalance.amount(present: 1, nanos: 850, scale: 2), "$8.50")
    }

    /// A scale this build cannot honour produces no figure rather than a
    /// wrong one -- which is what `BalanceStatus.unreadableScale` relies on
    /// when the wire carries no scale at all.
    func testAnUnhonourableScaleProducesNoFigure() throws {
        XCTAssertEqual(
            TCNearAiBalance.amount(
                present: 1, nanos: 8_500_000_000, scale: BalanceStatus.unreadableScale),
            "")
        // And the row falls to the sentence that says so, rather than to the
        // one about an uncapped account.
        let row = BalanceSurface.remaining(
            status("known", remaining: 8_500_000_000, scale: BalanceStatus.unreadableScale),
            copy: try payload(), calls: calls())
        XCTAssertEqual(row, .sentence(try payload().balanceUnknown))
        XCTAssertNotEqual(row, .sentence(try payload().balanceNoRemaining))
    }

    /// The limit and spent lines take the OPPOSITE branch on absence, and
    /// the asymmetry is deliberate: the remaining line has already said the
    /// thing that matters about an uncapped account.
    func testTheOtherTwoMoneyLinesDropOutOnAbsenceButNotOnZero() throws {
        let uncapped = status("known", remaining: nil, limit: nil, spent: nil)
        XCTAssertNil(BalanceSurface.limitLine(uncapped, calls: calls()))
        XCTAssertNil(BalanceSurface.spentLine(uncapped, calls: calls()))

        let nothingSpent = status("known", remaining: 0, limit: 0, spent: 0)
        let spent = try XCTUnwrap(BalanceSurface.spentLine(nothingSpent, calls: calls()))
        XCTAssertTrue(spent.contains("$0.00"), "a zero spend is a real $0.00: \(spent)")
    }

    // MARK: - The sentences

    /// Every state gets the payload's own sentence, and `known` gets none.
    func testEveryStateGetsItsOwnSentenceAndKnownGetsNone() throws {
        let copy = try payload()
        let expected: [String: String?] = [
            "": copy.balanceUnreported,
            "known": nil,
            "no_session": copy.balanceNoSession,
            "session_expired": copy.balanceSessionExpired,
            "no_organization": copy.balanceNoOrganization,
            "unavailable": copy.balanceUnavailable,
        ]
        for (state, sentence) in expected {
            XCTAssertEqual(
                BalanceSurface.stateLine(status(state), copy: copy, calls: calls()), sentence,
                "state \(state.isEmpty ? "<empty>" : state)")
        }
    }

    /// **An unrecognised state borrows nobody's sentence.** It gets the one
    /// written for it, and specifically not the claim that no sign-in is
    /// kept here, nor the claim that the service failed to answer.
    func testAnUnrecognisedStateGetsTheUnknownSentenceAndBorrowsNoOthers() throws {
        let copy = try payload()
        let line = try XCTUnwrap(
            BalanceSurface.stateLine(status("a_state_from_2027"), copy: copy, calls: calls()))
        XCTAssertEqual(line, copy.balanceUnknown)
        for borrowed in [
            copy.balanceNoSession, copy.balanceSessionExpired, copy.balanceNoOrganization,
            copy.balanceUnavailable, copy.balanceUnreported, copy.balanceNoRemaining,
        ] {
            XCTAssertNotEqual(line, borrowed)
        }
    }

    /// An unread state and an unanswered question are different sentences.
    func testUnreportedAndUnknownAreNotTheSameSentence() throws {
        let copy = try payload()
        XCTAssertNotEqual(copy.balanceUnknown, copy.balanceUnreported)
    }

    // MARK: - Tone

    /// `known` is the only CLEAR, and it means the READ succeeded.
    func testOnlyAReadThatSucceededReadsAsSettled() {
        XCTAssertEqual(BalanceSurface.tone(status("known"), calls: calls()), .clear)
        for state in Self.known.dropFirst() + ["", "a_state_from_2027"] {
            XCTAssertNotEqual(
                BalanceSurface.tone(status(state), calls: calls()), .clear,
                "\(state) is not a successful read")
        }
    }

    /// **No amount moves the tone.** An account with nothing in it and an
    /// account with plenty paint identically, because nothing across this
    /// ABI judges an amount and a threshold would be this shell's invention.
    func testTheToneIgnoresTheAmountEntirely() {
        let tones = [nil, Int64(-5_000_000_000), 0, 1, 10_000_000_000].map {
            BalanceSurface.tone(status("known", remaining: $0), calls: calls())
        }
        XCTAssertEqual(Set(tones).count, 1, "an amount changed the tone: \(tones)")
        XCTAssertEqual(tones.first, .clear)
    }

    // MARK: - The one action

    /// **OBTAIN on exactly two states, and NONE on every other.**
    func testSignInIsOfferedOnExactlyTheTwoStatesThatAnswerIt() {
        let offering = ["no_session", "session_expired"]
        for state in Self.known + ["", "a_state_from_2027", "no_organization"] {
            let action = BalanceSurface.action(status(state), calls: calls())
            XCTAssertEqual(
                action, offering.contains(state) ? .obtain : CredentialAction.none,
                "state \(state.isEmpty ? "<empty>" : state)")
        }
    }

    /// A refused session is offered a sign-in and never a forget: the
    /// ceremony overwrites both records, and forgetting first would throw
    /// away a key that still works.
    func testARefusedSessionIsNeverOfferedAForget() {
        for state in Self.known + ["", "a_state_from_2027"] {
            let action = BalanceSurface.action(status(state), calls: calls())
            XCTAssertNotEqual(action, .forget, "\(state) must not offer a forget")
            XCTAssertNotEqual(action, .cancel, "\(state) has no ceremony to cancel")
        }
    }

    /// It is the SIGN-IN row's enum, so the button beside a balance and the
    /// button beside a key are the same button.
    func testTheActionSharesTheCredentialRowsEnum() throws {
        let label = try XCTUnwrap(
            CredentialSurface.actionLabel(
                BalanceSurface.action(status("no_session"), calls: calls()), copy: try payload()))
        XCTAssertEqual(label, try payload().credentialObtain)
    }

    // MARK: - When it was asked for

    /// A negative age is the ABI's "no observation", and is drawn as nothing
    /// rather than as "just now".
    func testAnAbsentObservationDrawsNoAgeLine() {
        XCTAssertEqual(TCNearAiBalance.observedLine(secondsAgo: -1), "")
        XCTAssertNotEqual(TCNearAiBalance.observedLine(secondsAgo: 0), "")
    }
}
