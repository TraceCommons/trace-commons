import XCTest

@testable import TCBridge
@testable import TCShellCore

/// The contribution-eligibility surface as it really crosses the C ABI.
///
/// Links the dylib. What this can assert and `TCShellCoreTests` cannot is
/// that the four tables answer HERE what they answer there -- so the mutation
/// proofs in that target, which only show the surface asks a table, are
/// joined to the real table's answers.
final class EligibilityExportTests: XCTestCase {
    private func copy() -> PrivateInferenceCopy? {
        guard let json = TCPrivateInference.copyJSON() else {
            XCTFail("the export returned nothing")
            return nil
        }
        guard let copy = PrivateInferenceCopy.decode(fromJSON: json) else {
            XCTFail("the payload did not decode into this shell's struct")
            return nil
        }
        return copy
    }

    /// The production wiring, verbatim -- the same four closures `AppModel`
    /// builds `eligibilityCalls` from.
    private func calls() -> EligibilityCalls {
        EligibilityCalls(
            stateLine: { TCContributionEligibility.stateLine(state: $0) },
            stateTone: { TCContributionEligibility.stateTone(state: $0) },
            control: { TCContributionEligibility.control(state: $0) },
            reasonLine: { TCContributionEligibility.reasonLine(reason: $0) },
            withheldLine: { TCContributionEligibility.withheldLine(withheld: $0) }
        )
    }

    private func at(_ state: String) -> ContributionEligibility {
        ContributionEligibility(state: state)
    }

    /// Every state the daemon can send reaches its own sentence, and one it
    /// cannot read reaches the unknown one.
    func testEachStateRendersTheSentenceTheRustExports() throws {
        let copy = try XCTUnwrap(copy())
        let line = { (label: String) -> String? in
            EligibilitySurface.stateLine(self.at(label), copy: copy, calls: self.calls())
        }
        XCTAssertEqual(line("eligible"), copy.eligibilityEligible)
        XCTAssertEqual(line("ineligible_permanent"), copy.eligibilityIneligiblePermanent)
        XCTAssertEqual(line("ineligible_configuration"), copy.eligibilityIneligibleConfiguration)
        XCTAssertEqual(line("unknown"), copy.eligibilityUnknown)
        // Neither of these may degrade to an ineligibility, which is a claim
        // about a contributor's own session.
        XCTAssertEqual(line("a_state_from_a_later_daemon"), copy.eligibilityUnknown)
        XCTAssertEqual(line(""), copy.eligibilityUnknown)
        XCTAssertNotEqual(copy.eligibilityUnknown, copy.eligibilityIneligiblePermanent)
        XCTAssertNotEqual(copy.eligibilityUnknown, copy.eligibilityIneligibleConfiguration)
    }

    /// An entry that carried no `eligibility` key reaches no sentence at all
    /// across the real boundary -- the tables are never asked.
    func testAnAbsentKeyReachesNoSentenceAcrossTheRealAbi() throws {
        let copy = try XCTUnwrap(copy())
        XCTAssertNil(EligibilitySurface.stateLine(nil, copy: copy, calls: calls()))
        XCTAssertNil(EligibilitySurface.tone(nil, calls: calls()))
        XCTAssertNil(EligibilitySurface.reasonLine(nil, calls: calls()))
        // And it keeps the send control: an invited contributor's queue is
        // entirely contributable.
        XCTAssertTrue(EligibilitySurface.offersContribute(nil, calls: calls()))
    }

    /// Exactly one state is painted as settled, and a permanent
    /// ineligibility is NOT painted as a refusal.
    ///
    /// `.refused` is the arm worth naming: nothing was refused and nothing
    /// went wrong, and painting a contributor's ordinary older work as a
    /// failure is a judgement this surface has no business making.
    func testOnlyAnEligibleSessionIsPaintedClearAndNothingIsPaintedRefused() {
        let tone = { (label: String) -> PrivateInferenceTone? in
            EligibilitySurface.tone(self.at(label), calls: self.calls())
        }
        XCTAssertEqual(tone("eligible"), .clear)
        XCTAssertEqual(tone("ineligible_permanent"), .neutral)
        XCTAssertEqual(tone("ineligible_configuration"), .attention)
        XCTAssertEqual(tone("unknown"), .neutral)
        XCTAssertEqual(tone("a_state_from_a_later_daemon"), .neutral)
        XCTAssertEqual(tone(""), .neutral)
        for label in [
            "eligible", "ineligible_permanent", "ineligible_configuration", "unknown",
            "a_state_from_a_later_daemon", "",
        ] {
            XCTAssertNotEqual(tone(label), .refused, label)
        }
    }

    /// The send control beside each state, across the boundary.
    ///
    /// The state nobody could read gets NO control. That is the arm worth
    /// naming: offering one there is how a contributor's work is sent and
    /// refused, which is the defect this whole surface exists to remove.
    func testTheSendControlBesideEachStateCrossesTheAbi() {
        let control = { (label: String) -> ContributionControl in
            EligibilitySurface.control(self.at(label), calls: self.calls())
        }
        XCTAssertEqual(control("eligible"), .contribute)
        XCTAssertEqual(control("ineligible_permanent"), ContributionControl.none)
        XCTAssertEqual(control("ineligible_configuration"), ContributionControl.none)
        XCTAssertEqual(control("unknown"), ContributionControl.none)
        XCTAssertEqual(control("a_state_from_a_later_daemon"), ContributionControl.none)
        XCTAssertEqual(control(""), ContributionControl.none)
    }

    /// Each of the thirteen reason labels reaches its own sentence, and no
    /// two share one.
    func testEachReasonLabelReachesItsOwnSentence() throws {
        let copy = try XCTUnwrap(copy())
        let expected: [(String, String)] = [
            ("no_inference_call", copy.eligibilityReasonNoCall),
            ("capture_off", copy.eligibilityReasonCaptureOff),
            ("digest_absent", copy.eligibilityReasonDigestAbsent),
            ("upstream_id_absent", copy.eligibilityReasonUpstreamIdAbsent),
            ("digest_mismatch", copy.eligibilityReasonDigestMismatch),
            ("reference_malformed", copy.eligibilityReasonReferenceMalformed),
            ("bodies_unreadable", copy.eligibilityReasonBodiesUnreadable),
            ("body_not_utf8", copy.eligibilityReasonBodyNotUtf8),
            ("body_too_large", copy.eligibilityReasonBodyTooLarge),
            ("evidence_capture_off", copy.eligibilityReasonEvidenceCaptureOff),
            ("marker_absent", copy.eligibilityReasonMarkerAbsent),
            ("request_malformed", copy.eligibilityReasonRequestMalformed),
            ("receipt_unavailable", copy.eligibilityReasonReceiptUnavailable),
        ]
        XCTAssertEqual(expected.count, 13)
        for (label, sentence) in expected {
            XCTAssertFalse(sentence.isEmpty, label)
            XCTAssertEqual(
                EligibilitySurface.reasonLine(
                    ContributionEligibility(state: "ineligible_permanent", reason: label),
                    calls: calls()),
                sentence, label)
        }
        XCTAssertEqual(Set(expected.map(\.1)).count, 13, "no two reasons share a sentence")
    }

    /// An unfamiliar reason renders NOTHING and borrows no sentence.
    ///
    /// Deliberately unlike the state line's fallback: the state sentence has
    /// already said what is true, and a second sentence guessing at a reason
    /// this build does not know would add a detail nobody established.
    func testAnUnfamiliarReasonRendersNothing() {
        for label in ["", "a_reason_from_a_later_daemon", "NO_INFERENCE_CALL"] {
            XCTAssertNil(
                EligibilitySurface.reasonLine(
                    ContributionEligibility(state: "ineligible_permanent", reason: label),
                    calls: calls()),
                label)
        }
        XCTAssertEqual(TCContributionEligibility.reasonLine(reason: "nonsense"), "")
    }

    /// Only `ineligible_configuration` names a setting.
    ///
    /// `NoCall` has no actionable answer for the session the row is about,
    /// and advice about the NEXT session is guidance rather than status. The
    /// tone carries the same rule: attention is that state alone.
    func testOnlyTheConfigurationStateNamesASetting() throws {
        let copy = try XCTUnwrap(copy())
        XCTAssertTrue(
            copy.eligibilityIneligibleConfiguration.contains("setting"),
            copy.eligibilityIneligibleConfiguration)
        XCTAssertFalse(
            copy.eligibilityIneligiblePermanent.contains("setting"),
            copy.eligibilityIneligiblePermanent)
        XCTAssertFalse(copy.eligibilityEligible.contains("setting"), copy.eligibilityEligible)
        XCTAssertFalse(copy.eligibilityUnknown.contains("setting"), copy.eligibilityUnknown)
    }

    /// A permanent ineligibility says the second half out loud: that trying
    /// again is wasted work. Without it somebody retries their own finished
    /// session, and again.
    func testAPermanentIneligibilitySaysRetryingWillNotHelp() throws {
        let copy = try XCTUnwrap(copy())
        XCTAssertTrue(
            copy.eligibilityIneligiblePermanent.lowercased().contains("again"),
            copy.eligibilityIneligiblePermanent)
    }

    /// `eligible` states an expectation and stops. The expensive checks still
    /// run at submit and the server decides; a sentence promising acceptance
    /// would be a claim this side cannot make.
    func testEligibleDoesNotPromiseAcceptance() throws {
        let copy = try XCTUnwrap(copy())
        let sentence = copy.eligibilityEligible.lowercased()
        for promise in ["will be accepted", "guaranteed", "always"] {
            XCTAssertFalse(sentence.contains(promise), copy.eligibilityEligible)
        }
        XCTAssertTrue(sentence.contains("send"), copy.eligibilityEligible)
    }

    /// The withheld sentence, across the real boundary.
    ///
    /// Empty for zero AND for a negative -- there is no gap to explain, and
    /// "0 sessions are not being sent" invents a caveat where none exists.
    /// It says how many and never why: a summary standing for up to thirteen
    /// different reasons would say nothing true about any of them.
    func testTheWithheldSentenceSaysHowManyAndNeverWhy() throws {
        let copy = try XCTUnwrap(copy())
        XCTAssertEqual(TCContributionEligibility.withheldLine(withheld: 0), "")
        for negative: Int64 in [-1, -7, Int64.min] {
            XCTAssertEqual(
                TCContributionEligibility.withheldLine(withheld: negative), "", "\(negative)")
        }
        let one = try XCTUnwrap(TCContributionEligibility.withheldLine(withheld: 1))
        let four = try XCTUnwrap(TCContributionEligibility.withheldLine(withheld: 4))
        XCTAssertTrue(one.contains("1 session"), one)
        XCTAssertTrue(four.contains("4 sessions"), four)
        // Never a reason. These are the reason sentences; none of them may
        // leak into the summary.
        for reason in [
            copy.eligibilityReasonNoCall, copy.eligibilityReasonCaptureOff,
            copy.eligibilityReasonMarkerAbsent,
        ] {
            XCTAssertFalse(four.contains(reason), four)
        }
        XCTAssertFalse(four.lowercased().contains("because"), four)
    }

    /// The offer, assembled from the real table.
    func testTheGroupOfferCrossesTheAbi() {
        let offer = EligibilitySurface.groupSubmit(
            pendingCount: 7, contributableCount: 3, fallbackPending: 7, calls: calls())
        XCTAssertEqual(offer.count, 3)
        XCTAssertEqual(offer.withheldLine, TCContributionEligibility.withheldLine(withheld: 4))
        // Absent draws no line at all, and a full folder draws none either.
        XCTAssertNil(
            EligibilitySurface.groupSubmit(
                pendingCount: 7, contributableCount: nil, fallbackPending: 7, calls: calls()
            ).withheldLine)
        XCTAssertNil(
            EligibilitySurface.groupSubmit(
                pendingCount: 7, contributableCount: 7, fallbackPending: 7, calls: calls()
            ).withheldLine)
    }

    /// Nothing on this surface has a hole a value could be poured into.
    ///
    /// These sentences are drawn beside a session's own identity, and a
    /// template marker here is how a path or a digest reaches a screen the
    /// contract keeps them off.
    func testNoEligibilitySentenceCanCarryAValue() throws {
        let copy = try XCTUnwrap(copy())
        let sentences = [
            copy.eligibilityEligible, copy.eligibilityIneligiblePermanent,
            copy.eligibilityIneligibleConfiguration, copy.eligibilityUnknown,
            copy.eligibilityReasonNoCall, copy.eligibilityReasonCaptureOff,
            copy.eligibilityReasonDigestAbsent, copy.eligibilityReasonUpstreamIdAbsent,
            copy.eligibilityReasonDigestMismatch, copy.eligibilityReasonReferenceMalformed,
            copy.eligibilityReasonBodiesUnreadable, copy.eligibilityReasonBodyNotUtf8,
            copy.eligibilityReasonBodyTooLarge, copy.eligibilityReasonEvidenceCaptureOff,
            copy.eligibilityReasonMarkerAbsent, copy.eligibilityReasonRequestMalformed,
            copy.eligibilityReasonReceiptUnavailable,
        ]
        XCTAssertEqual(sentences.count, 17)
        for sentence in sentences {
            XCTAssertFalse(sentence.isEmpty)
            for marker in ["{}", "{path}", "{reason}", "%@", "%s", "%d"] {
                XCTAssertFalse(sentence.contains(marker), "\(sentence) has a hole for a value")
            }
        }
    }
}
