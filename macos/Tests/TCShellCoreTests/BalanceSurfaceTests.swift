import XCTest

@testable import TCShellCore

/// The balance row's logic, without the dylib.
///
/// The injected calls deliberately do NOT reimplement the Rust's tables.
/// Several of them answer the OPPOSITE of the real one for every input, so a
/// surface that had quietly started deciding for itself could not agree with
/// them -- which is the only way a test in this target can tell "asked the
/// ABI" from "reproduced the ABI in Swift".
///
/// The distinction this file exists to hold is NULL VERSUS ZERO. Every other
/// money figure on this surface encodes absence as an out-of-range integer;
/// these amounts are signed, so absence has to travel BESIDE the integer, and
/// a shell that folded the two together would render an emptied account and
/// an unread one identically. `BalanceSurface.wire` is the one place that
/// split happens, and most of what follows is about keeping it the one place.
final class BalanceSurfaceTests: XCTestCase {
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
         "eligibility_reason_receipt_unavailable":"R-RECEIPT-UNAVAILABLE",
         "attestation_attested":"A-ATTESTED",
         "attestation_unattested_permanent":"A-UNATTESTED-PERMANENT",
         "attestation_unattested_configuration":"A-UNATTESTED-CONFIGURATION",
         "attestation_unknown":"A-UNKNOWN",
         "attestation_reason_no_call":"AR-NO-CALL",
         "attestation_reason_capture_off":"AR-CAPTURE-OFF",
         "attestation_reason_digest_absent":"AR-DIGEST-ABSENT",
         "attestation_reason_upstream_id_absent":"AR-UPSTREAM-ID-ABSENT",
         "attestation_reason_digest_mismatch":"AR-DIGEST-MISMATCH",
         "attestation_reason_reference_malformed":"AR-REFERENCE-MALFORMED",
         "attestation_reason_bodies_unreadable":"AR-BODIES-UNREADABLE",
         "attestation_reason_body_not_utf8":"AR-BODY-NOT-UTF8",
         "attestation_reason_body_too_large":"AR-BODY-TOO-LARGE",
         "attestation_reason_evidence_capture_off":"AR-EVIDENCE-CAPTURE-OFF",
         "attestation_reason_marker_absent":"AR-MARKER-ABSENT",
         "attestation_reason_request_malformed":"AR-REQUEST-MALFORMED",
         "attestation_reason_receipt_unavailable":"AR-RECEIPT-UNAVAILABLE",
         "balance_title":"BALANCE-TITLE",
         "balance_what":"BALANCE-WHAT",
         "balance_no_session":"BALANCE-NO-SESSION",
         "balance_session_expired":"BALANCE-SESSION-EXPIRED",
         "balance_no_organization":"BALANCE-NO-ORGANIZATION",
         "balance_unavailable":"BALANCE-UNAVAILABLE",
         "balance_unknown":"BALANCE-UNKNOWN",
         "balance_unreported":"BALANCE-UNREPORTED",
         "balance_no_remaining":"BALANCE-NO-REMAINING"}
        """

    /// Decoded in `setUpWithError` rather than force-unwrapped at use, for
    /// `CredentialSurfaceTests`'s reason: a `fatalError` on a fixture aborts
    /// the whole XCTest process and takes every other target's tests with it.
    private var decodedCopy: PrivateInferenceCopy!

    override func setUpWithError() throws {
        try super.setUpWithError()
        decodedCopy = try XCTUnwrap(
            PrivateInferenceCopy.decode(fromJSON: payload),
            "the fixture payload must decode")
    }

    private func copy() -> PrivateInferenceCopy { decodedCopy }

    /// Fakes that record what the ABI was asked, and answer things no
    /// payload field holds.
    ///
    /// The money fakes echo their three arguments, so a test can assert on
    /// the `present` flag the surface passed WITHOUT the surface being able
    /// to satisfy it by reading a field.
    private final class Recorder: @unchecked Sendable {
        var amountCalls: [(Int32, Int64, UInt8)] = []
        var remainingCalls: [(Int32, Int64, UInt8)] = []
    }

    private func calls(
        recorder: Recorder = Recorder(),
        line: @escaping @Sendable (String) -> String? = { $0 == "known" ? "" : "LINE:\($0)" },
        tone: @escaping @Sendable (String) -> Int32 = { _ in 21 },
        action: @escaping @Sendable (String) -> Int32 = { _ in 30 },
        amount: (@Sendable (Int32, Int64, UInt8) -> String?)? = nil,
        remaining: (@Sendable (Int32, Int64, UInt8) -> String?)? = nil,
        limit: @escaping @Sendable (Int32, Int64, UInt8) -> String? = { p, n, s in
            p == 0 ? "" : "LIMIT:\(n)@\(s)"
        },
        spent: @escaping @Sendable (Int32, Int64, UInt8) -> String? = { p, n, s in
            p == 0 ? "" : "SPENT:\(n)@\(s)"
        },
        observed: @escaping @Sendable (Int64) -> String? = { $0 < 0 ? "" : "OBSERVED:\($0)" }
    ) -> BalanceCalls {
        BalanceCalls(
            stateLine: line,
            stateTone: tone,
            action: action,
            amount: amount ?? { p, n, s in
                recorder.amountCalls.append((p, n, s))
                return p == 0 ? "" : "AMOUNT:\(n)@\(s)"
            },
            remainingLine: remaining ?? { p, n, s in
                recorder.remainingCalls.append((p, n, s))
                return p == 0 ? "NO-LIMIT-SENTENCE" : "REMAINING:\(n)@\(s)"
            },
            limitLine: limit,
            spentLine: spent,
            observedLine: observed)
    }

    private func known(
        remaining: Int64? = 8_500_000_000,
        limit: Int64? = 10_000_000_000,
        spent: Int64? = 1_500_000_000,
        scale: UInt8 = 9,
        observedAt: Date? = nil
    ) -> BalanceStatus {
        BalanceStatus(
            state: "known", remainingNanos: remaining, spendLimitNanos: limit,
            totalSpentNanos: spent, scale: scale, observedAt: observedAt)
    }

    // MARK: - Null is not zero

    /// **The rule this whole row exists to keep.** A null remaining figure
    /// and a zero one are different facts, and the surface must not be able
    /// to reach the same rendering from both.
    ///
    /// Asserted on the FLAG the surface passed across the ABI, not only on
    /// the string that came back: a surface that passed `present == 1` with
    /// a zeroed `nanos` for a null would produce "$0.00" against the real
    /// Rust, and a test that only compared two fake strings would miss it.
    func testANullAmountIsNeverRenderedAsZero() {
        let recorder = Recorder()
        let absent = BalanceSurface.remaining(
            known(remaining: nil), copy: copy(), calls: calls(recorder: recorder))
        let zero = BalanceSurface.remaining(
            known(remaining: 0), copy: copy(), calls: calls(recorder: recorder))

        // What crossed the ABI: absence travels as the flag, never as the
        // integer.
        XCTAssertEqual(recorder.amountCalls.map(\.0), [0, 1])
        XCTAssertNotEqual(absent, zero)

        // A null has NO figure at all, and falls back to the Rust's own
        // sentence for it.
        XCTAssertEqual(absent, .sentence("NO-LIMIT-SENTENCE"))
        if case .figure = absent { XCTFail("a null remaining figure must not render a figure") }

        // A zero is a real balance and renders as one.
        XCTAssertEqual(zero, .figure("AMOUNT:0@9"))
    }

    /// A real zero reaches the money formatter as a present zero, so the
    /// Rust can print "$0.00" -- which is true, and is the sentence that
    /// says the money is gone.
    func testARealZeroIsPresentAndKeepsItsFigure() {
        let recorder = Recorder()
        _ = BalanceSurface.remaining(
            known(remaining: 0), copy: copy(), calls: calls(recorder: recorder))
        XCTAssertEqual(recorder.amountCalls.count, 1)
        XCTAssertEqual(recorder.amountCalls[0].0, 1, "a zero balance is PRESENT")
        XCTAssertEqual(recorder.amountCalls[0].1, 0)
    }

    /// A negative balance is a real figure too. Folding "null" onto an
    /// out-of-range integer is exactly what this ABI refuses to do, because
    /// an overdrawn account IS negative.
    func testAnOverdrawnBalanceIsAFigureAndNotAnAbsence() {
        let overdrawn = BalanceSurface.remaining(
            known(remaining: -1_000_000_000), copy: copy(), calls: calls())
        XCTAssertEqual(overdrawn, .figure("AMOUNT:-1000000000@9"))
    }

    /// `present == 0` on the REMAINING line does not mean "empty". It means
    /// the account has no ceiling, which is the ordinary case, and the
    /// contributor must not be told they have $0.00 left.
    func testAnAbsentRemainingFigureAsksForTheNoLimitSentence() {
        let recorder = Recorder()
        let row = BalanceSurface.remaining(
            known(remaining: nil), copy: copy(), calls: calls(recorder: recorder))
        XCTAssertEqual(recorder.remainingCalls.map(\.0), [0])
        XCTAssertEqual(row, .sentence("NO-LIMIT-SENTENCE"))
        // And specifically not the empty string, which is what the LIMIT and
        // SPENT lines answer for the same absent input.
        XCTAssertNotEqual(row, .sentence(""))
    }

    /// A Rust that answered nothing at all still does not produce a figure.
    ///
    /// Both halves of `remaining` can come back empty -- a caught panic
    /// gives `nil`, and a scale this build cannot honour gives `""` from the
    /// formatter -- and the row then falls to the sentence that says the
    /// answer could not be read. **It must not fall to `$0.00`.** This is
    /// the one path the real ABI cannot be made to take from a test, so a
    /// fake takes it: without this, a fallback of `.figure("$0.00")` sits
    /// here unnoticed, because every reachable input routes around it.
    func testAnUnanswerableFigureFallsToTheUnknownSentenceAndNeverToZero() {
        for (amount, remaining) in [(nil, nil), ("", nil), (nil, ""), ("", "")]
            as [(String?, String?)]
        {
            let row = BalanceSurface.remaining(
                known(), copy: copy(),
                calls: calls(amount: { _, _, _ in amount }, remaining: { _, _, _ in remaining }))
            XCTAssertEqual(row, .sentence(copy().balanceUnknown))
            XCTAssertNotEqual(row, .figure("$0.00"))
            if case .figure(let figure) = row {
                XCTFail("an unanswerable figure rendered \(figure)")
            }
        }
    }

    /// The other two money lines DO drop out when absent -- the opposite of
    /// the remaining line, and the asymmetry is the contract's.
    func testTheLimitAndSpentLinesDropOutWhenAbsentButNotWhenZero() {
        let uncapped = known(limit: nil, spent: nil)
        XCTAssertNil(BalanceSurface.limitLine(uncapped, calls: calls()))
        XCTAssertNil(BalanceSurface.spentLine(uncapped, calls: calls()))

        let nothingSpent = known(limit: 0, spent: 0)
        XCTAssertEqual(BalanceSurface.limitLine(nothingSpent, calls: calls()), "LIMIT:0@9")
        XCTAssertEqual(BalanceSurface.spentLine(nothingSpent, calls: calls()), "SPENT:0@9")
    }

    // MARK: - The scale comes off the wire

    /// Never a constant of this shell's. A daemon that changes `scale` must
    /// move every figure here with it.
    func testEveryFigureIsFormattedWithTheWiresOwnScale() {
        let recorder = Recorder()
        let sixes = known(scale: 6)
        _ = BalanceSurface.remaining(sixes, copy: copy(), calls: calls(recorder: recorder))
        XCTAssertEqual(recorder.amountCalls.map(\.2), [6])
        XCTAssertEqual(BalanceSurface.limitLine(sixes, calls: calls()), "LIMIT:10000000000@6")
        XCTAssertEqual(BalanceSurface.spentLine(sixes, calls: calls()), "SPENT:1500000000@6")
    }

    /// A payload with no `scale` at all cannot be formatted honestly, and
    /// this shell does not pick one. It passes a scale the Rust cannot
    /// honour, whose answer is the empty string -- which reaches the reader
    /// as "came back in a form this app cannot read" and never as a figure.
    func testAMissingScaleIsRefusedRatherThanGuessed() {
        let status = BalanceStatus.parse(
            fromJSON: #"{"state":"known","remaining_nanos":8500000000}"#)
        XCTAssertEqual(status.scale, BalanceStatus.unreadableScale)
        XCTAssertNotEqual(status.scale, 9, "no scale is assumed")
    }

    // MARK: - The state line

    /// From the shared table, and the fake answers a value no payload field
    /// holds.
    func testTheStateSentenceComesFromTheSharedTable() {
        XCTAssertEqual(
            BalanceSurface.stateLine(
                BalanceStatus(state: "unavailable"), copy: copy(), calls: calls()),
            "LINE:unavailable")
    }

    /// An unrecognised state gets its OWN sentence and borrows nobody's.
    ///
    /// Proved against the real Rust in `BalanceExportTests`; here the point
    /// is that this surface does not intercept an unfamiliar label on its
    /// way to the table.
    func testAnUnrecognisedStateIsPassedThroughRatherThanMappedHere() {
        let later = BalanceSurface.stateLine(
            BalanceStatus(state: "a_state_from_2027"), copy: copy(), calls: calls())
        XCTAssertEqual(later, "LINE:a_state_from_2027")
        XCTAssertNotEqual(later, copy().balanceNoSession)
        XCTAssertNotEqual(later, copy().balanceUnavailable)
    }

    /// `known` answers the empty string, and this surface turns that into NO
    /// LINE rather than a blank one: the row's content is the figures.
    func testKnownDrawsNoStateSentenceAtAll() {
        XCTAssertNil(BalanceSurface.stateLine(known(), copy: copy(), calls: calls()))
    }

    /// A caught panic falls back to the payload's UNREPORTED sentence --
    /// never to `balanceNoSession`, which is a claim about this machine.
    func testASilentExportFallsBackToUnreportedAndNeverToNoSession() {
        let fallback = BalanceSurface.stateLine(
            BalanceStatus(state: "known"), copy: copy(), calls: calls(line: { _ in nil }))
        XCTAssertEqual(fallback, copy().balanceUnreported)
        XCTAssertNotEqual(fallback, copy().balanceNoSession)
    }

    /// The sentence and the figures are alternatives.
    ///
    /// Only `known` has a reading behind it. Drawing the figures beside a
    /// state that has a sentence would tell somebody who has never signed in
    /// that no spending limit is set on their account -- true of an uncapped
    /// account, and nonsense here, because the null those figures are read
    /// from means "nothing was read" rather than "nothing was configured".
    func testTheFiguresAreDrawnOnlyWhereThereIsNoSentence() {
        XCTAssertTrue(BalanceSurface.showsFigures(known(), calls: calls()))
        for state in ["", "no_session", "session_expired", "unavailable", "a_state_from_2027"] {
            XCTAssertFalse(
                BalanceSurface.showsFigures(BalanceStatus(state: state), calls: calls()),
                "\(state) has a sentence and must not also show figures")
        }
        // A caught panic draws no figures either.
        XCTAssertFalse(BalanceSurface.showsFigures(known(), calls: calls(line: { _ in nil })))
    }

    // MARK: - Tone

    /// The tone is the shared table's, decoded through the same five values
    /// every other row on this destination uses.
    func testTheToneIsDecodedFromTheSharedTable() {
        XCTAssertEqual(
            BalanceSurface.tone(known(), calls: calls(tone: { _ in 22 })), .clear)
        XCTAssertEqual(
            BalanceSurface.tone(known(), calls: calls(tone: { _ in 24 })), .refused)
    }

    /// Nothing here judges an amount. An emptied account and a full one are
    /// the same state and must paint the same, because a threshold is a
    /// claim nobody on this ABI made -- on an account whose ceiling may not
    /// exist.
    func testNoAmountChangesTheTone() {
        // The fake is told only the STATE, so a surface that wanted to move
        // the tone with a figure would have to do it after the table
        // answered -- which is exactly what must not happen.
        let table = calls(tone: { _ in 22 })
        let tones = [nil, Int64(-5_000_000_000), 0, 1, 10_000_000_000].map {
            BalanceSurface.tone(known(remaining: $0), calls: table)
        }
        XCTAssertEqual(Set(tones).count, 1, "an amount moved the tone: \(tones)")
        XCTAssertEqual(tones.first, .clear)
    }

    // MARK: - The one action

    /// The action is the SIGN-IN ROW's enum, decoded by the same table, so
    /// obtain here and obtain there are one button.
    func testTheActionIsTheCredentialRowsOwnEnum() {
        XCTAssertEqual(
            BalanceSurface.action(known(), calls: calls(action: { _ in 31 })), .obtain)
        XCTAssertEqual(
            BalanceSurface.action(known(), calls: calls(action: { _ in 30 })), .none)
    }

    /// A refused session is offered a sign-in and NOT a forget: forgetting
    /// would throw away a working key to fix an unrelated sign-in. Proved
    /// end to end against the Rust in `BalanceExportTests`; here the surface
    /// is shown not to substitute one for the other.
    func testAForgetIsNeverSubstitutedForTheOfferedSignIn() {
        XCTAssertEqual(
            BalanceSurface.action(
                BalanceStatus(state: "session_expired"), calls: calls(action: { _ in 31 })),
            .obtain)
    }

    /// One card must not draw the same button twice. When the sign-in row is
    /// already offering exactly this action, the balance row defers to it --
    /// and defers ONLY on an exact match, so a balance that says the session
    /// was refused still offers a sign-in beside a credential row that is
    /// offering something else, or nothing.
    func testTheBalanceDefersOnlyWhenTheSignInRowOffersTheSameButton() {
        XCTAssertEqual(BalanceSurface.actionToDraw(balance: .obtain, credential: .obtain), .none)
        XCTAssertEqual(BalanceSurface.actionToDraw(balance: .obtain, credential: .none), .obtain)
        XCTAssertEqual(BalanceSurface.actionToDraw(balance: .obtain, credential: .forget), .obtain)
        XCTAssertEqual(BalanceSurface.actionToDraw(balance: .obtain, credential: .cancel), .obtain)
        XCTAssertEqual(BalanceSurface.actionToDraw(balance: .none, credential: .obtain), .none)
    }

    // MARK: - When it was asked for

    /// The age is this shell's own arithmetic over the daemon's clock, and a
    /// missing `observed_at` crosses as the out-of-range value the ABI reads
    /// as "no age" -- never as zero, which would say "just now".
    func testAnAbsentObservationHasNoAgeRatherThanAnAgeOfZero() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        XCTAssertNil(BalanceSurface.observedLine(known(observedAt: nil), now: now, calls: calls()))
        XCTAssertEqual(
            BalanceSurface.observedLine(
                known(observedAt: now.addingTimeInterval(-120)), now: now, calls: calls()),
            "OBSERVED:120")
    }

    /// A clock that ran backwards is reported as "just now" rather than as
    /// no observation at all: something WAS observed, and a negative age is
    /// the value that means nothing was.
    func testAFutureObservationClampsToJustNowRatherThanVanishing() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        XCTAssertEqual(
            BalanceSurface.observedLine(
                known(observedAt: now.addingTimeInterval(30)), now: now, calls: calls()),
            "OBSERVED:0")
    }

    // MARK: - Parsing

    /// The numeric keys are present-and-null in every non-`known` state, and
    /// a null is not a zero on the way in either.
    func testAnExplicitNullParsesAsAbsentAndNotAsZero() {
        let status = BalanceStatus.parse(
            fromJSON: """
                {"state":"no_session","scale":9,"remaining_nanos":null,
                 "spend_limit_nanos":null,"total_spent_nanos":null,
                 "observed_at":null}
                """)
        XCTAssertEqual(status.state, "no_session")
        XCTAssertNil(status.remainingNanos)
        XCTAssertNil(status.spendLimitNanos)
        XCTAssertNil(status.totalSpentNanos)
        XCTAssertNil(status.observedAt)
    }

    func testAKnownReadingParsesEveryFigure() {
        let status = BalanceStatus.parse(
            fromJSON: """
                {"state":"known","currency":"USD","scale":9,
                 "remaining_nanos":8500000000,"spend_limit_nanos":10000000000,
                 "total_spent_nanos":1500000000,
                 "observed_at":"2026-09-08T12:00:00Z"}
                """)
        XCTAssertEqual(status.state, "known")
        XCTAssertEqual(status.remainingNanos, 8_500_000_000)
        XCTAssertEqual(status.spendLimitNanos, 10_000_000_000)
        XCTAssertEqual(status.totalSpentNanos, 1_500_000_000)
        XCTAssertNotNil(status.observedAt)
    }

    /// A body this shell cannot read at all reports the empty state, which
    /// the shared table answers as UNREPORTED -- the sentence that says the
    /// question was not answered, not one about what is in the account.
    func testAnUnreadableBodyIsUnreportedRatherThanEmpty() {
        XCTAssertEqual(BalanceStatus.parse(fromJSON: "not json").state, "")
        XCTAssertEqual(BalanceStatus.unreported.state, "")
        XCTAssertNil(BalanceStatus.unreported.remainingNanos)
    }

    /// The method is named once, here, and nothing composes it from parts.
    func testTheMethodIsNamedOnce() {
        XCTAssertEqual(BalanceSurface.statusMethod, "near_ai_balance")
    }
}
