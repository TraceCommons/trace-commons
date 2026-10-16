import XCTest

@testable import TCBridge
@testable import TCShellCore

/// The private-inference surface as it really crosses the C ABI.
///
/// Links the dylib. What this can assert and `TCShellCoreTests` cannot is
/// that the payload the Rust exports and the struct this shell decodes are
/// the same set, and that the two branch tables answer here what they answer
/// there.
final class PrivateInferenceExportTests: XCTestCase {
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

    func testQuitTracksOwnedWorkRatherThanTheRequestedSwitch() {
        XCTAssertTrue(TCPrivateInference.quitNeedsNotice(on: false, state: "stopping"))
        XCTAssertFalse(TCPrivateInference.quitNeedsNotice(on: true, state: "running_elsewhere"))
        XCTAssertFalse(TCPrivateInference.quitNeedsNotice(on: true, state: "off"))
    }

    func testWriteConfirmationDistinguishesMissingAndExplicitFalse() {
        for seen: Bool? in [nil, false, true] {
            for on: Bool? in [nil, false, true] {
                XCTAssertEqual(TCPrivateInference.writeConfirmed(requestedOn: nil, echoedSeen: seen, echoedOn: on), seen == true)
                XCTAssertEqual(TCPrivateInference.writeConfirmed(requestedOn: true, echoedSeen: seen, echoedOn: on), seen == true && on == true)
                XCTAssertEqual(TCPrivateInference.writeConfirmed(requestedOn: false, echoedSeen: seen, echoedOn: on), seen == true && on == false)
            }
        }
    }

    private func calls() -> PrivateInferenceCalls {
        // The production wiring, verbatim.
        PrivateInferenceCalls(
            stateLine: { TCPrivateInference.stateLine(state: $0) },
            stateTone: { TCPrivateInference.stateTone(state: $0) },
            servingLine: { TCPrivateInference.servingLine(port: $0) },
            shouldOffer: { TCPrivateInference.shouldOffer(answered: $0, on: $1) },
            quitNeedsNotice: { TCPrivateInference.quitNeedsNotice(on: $0, state: $1) }
        )
    }

    /// The exported field set and the decoded one are the same set, in both
    /// directions.
    ///
    /// A field the Rust grew and this struct dropped would sail past a test
    /// that only checked the fields it knows about.
    func testEveryExportedFieldIsDecodedAndNoneIsInvented() throws {
        let json = try XCTUnwrap(TCPrivateInference.copyJSON())
        let data = try XCTUnwrap(json.data(using: .utf8))
        let exported = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data) as? [String: Any])
        let declared = Set(PrivateInferenceCopy.CodingKeys.allCases.map(\.rawValue))
        XCTAssertEqual(Set(exported.keys), declared)
    }

    /// Every sentence arrives finished, and none of them is a template this
    /// shell would have to fill in.
    func testEverySentenceArrivesFinished() throws {
        let json = try XCTUnwrap(TCPrivateInference.copyJSON())
        let data = try XCTUnwrap(json.data(using: .utf8))
        let exported = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data) as? [String: String])
        for (field, text) in exported {
            XCTAssertFalse(
                text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                "\(field) arrived empty")
            for marker in ["{}", "{port}", "%@", "%s", "%d"] {
                XCTAssertFalse(text.contains(marker), "\(field) crossed as a template: \(text)")
            }
        }
    }

    /// The sentence this whole surface exists to print.
    ///
    /// Named rather than checked as "some field mentions accounts": a build
    /// that dropped it would still render an offer, and the offer would be a
    /// lie by omission.
    func testTheOfferSaysWhatTurningItOnExposes() throws {
        let copy = try XCTUnwrap(copy())
        XCTAssertTrue(copy.offerExposure.contains("anything else running"), copy.offerExposure)
        XCTAssertTrue(copy.offerExposure.contains("accounts"), copy.offerExposure)
    }

    /// The state sentences the daemon can produce cross intact, and
    /// each is the payload field that names it.
    func testEachStateRendersTheSentenceTheRustExports() throws {
        let copy = try XCTUnwrap(copy())
        let calls = calls()
        let line = { (label: String) -> String in
            PrivateInferenceSurface.stateLine(
                PrivateInferenceState(label: label, port: nil), copy: copy, calls: calls)
        }
        XCTAssertEqual(line("off"), copy.stateOff)
        XCTAssertEqual(line("stopping"), copy.stateStopping)
        XCTAssertEqual(line("running"), copy.stateRunning)
        XCTAssertEqual(line("running_no_backends"), copy.stateRunningNoBackends)
        XCTAssertEqual(line("running_elsewhere"), copy.stateRunningElsewhere)
        XCTAssertEqual(line("port_in_use"), copy.statePortInUse)
        XCTAssertEqual(line("start_failed"), copy.stateStartFailed)
        XCTAssertEqual(line("crashed"), copy.stateCrashed)
        // A state a later daemon grows, and no state at all, claim nothing.
        XCTAssertEqual(line("a_state_from_a_later_daemon"), copy.stateUnknown)
        XCTAssertEqual(line(""), copy.stateUnreported)
    }

    /// Exactly one state may be painted as working, and it is not the one
    /// with nowhere to send a call.
    func testOnlyAListenerWithSomewhereToSendIsPaintedClear() {
        let calls = calls()
        let tone = { (label: String) -> PrivateInferenceTone in
            PrivateInferenceSurface.tone(
                PrivateInferenceState(label: label, port: nil), calls: calls)
        }
        XCTAssertEqual(tone("running"), .clear)
        XCTAssertEqual(tone("running_no_backends"), .attention)
        XCTAssertNotEqual(tone("running_no_backends"), .clear)
        XCTAssertEqual(tone("running_elsewhere"), .held)
        XCTAssertEqual(tone("stopping"), .held)
        XCTAssertEqual(tone("off"), .neutral)
        for failure in ["port_in_use", "start_failed", "crashed"] {
            XCTAssertEqual(tone(failure), .refused, failure)
        }
        XCTAssertEqual(tone("a_state_from_a_later_daemon"), .neutral)
    }

    /// A refusal names the way out, across the boundary.
    func testEveryRefusalNamesTheWayOut() throws {
        let copy = try XCTUnwrap(copy())
        for sentence in [copy.statePortInUse, copy.stateStartFailed, copy.stateCrashed] {
            XCTAssertTrue(sentence.contains("off and on"), sentence)
        }
        XCTAssertTrue(copy.stateCrashed.contains("won't retry on its own"), copy.stateCrashed)
    }

    func testTheServingSentenceNamesAPortOrIsEmpty() {
        XCTAssertEqual(TCPrivateInference.servingLine(port: nil), "")
        XCTAssertEqual(TCPrivateInference.servingLine(port: 0), "")
        XCTAssertTrue(TCPrivateInference.servingLine(port: 8463)?.contains("8463") == true)
    }

    /// Whether to ask crosses the ABI, so this shell cannot come to disagree
    /// with the other two about who has already been asked.
    func testWhetherToOfferCrossesTheAbi() {
        XCTAssertTrue(TCPrivateInference.shouldOffer(answered: false, on: false))
        XCTAssertFalse(TCPrivateInference.shouldOffer(answered: true, on: false))
        XCTAssertFalse(TCPrivateInference.shouldOffer(answered: false, on: true))
        XCTAssertFalse(TCPrivateInference.shouldOffer(answered: true, on: true))
    }
}
