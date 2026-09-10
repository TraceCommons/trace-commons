import XCTest

@testable import TCBridge
@testable import TCShellCore

/// The credential surface as it really crosses the C ABI.
///
/// Links the dylib. What this can assert and `TCShellCoreTests` cannot is
/// that the three tables answer HERE what they answer there -- so the
/// mutation proofs in that target, which only show the surface asks a table,
/// are joined to the real table's answers.
final class CredentialExportTests: XCTestCase {
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

    /// The production wiring, verbatim.
    private func calls() -> CredentialCalls {
        CredentialCalls(
            stateLine: { TCNearAiCredential.stateLine(state: $0) },
            stateTone: { TCNearAiCredential.stateTone(state: $0) },
            action: { TCNearAiCredential.action(state: $0) },
            harnessNotice: { TCNearAiCredential.harnessNotice(credentialed: $0) ?? "" }
        )
    }

    /// Every state the daemon can report reaches its own sentence, and the
    /// two that could not be read reach theirs.
    func testEachStateRendersTheSentenceTheRustExports() throws {
        let copy = try XCTUnwrap(copy())
        let line = { (label: String) -> String in
            CredentialSurface.stateLine(
                CredentialStatus(state: label), copy: copy, calls: self.calls())
        }
        XCTAssertEqual(line("absent"), copy.credentialAbsent)
        XCTAssertEqual(line("obtaining"), copy.credentialObtaining)
        XCTAssertEqual(line("failed"), copy.credentialFailed)
        XCTAssertEqual(line("cancelled"), copy.credentialCancelled)
        XCTAssertEqual(line("present"), copy.credentialPresent)
        // Neither of these may degrade to `credentialAbsent`, which is a
        // claim about what this machine holds.
        XCTAssertEqual(line("a_credential_state_from_a_later_daemon"), copy.credentialUnknown)
        XCTAssertEqual(line(""), copy.credentialUnreported)
        XCTAssertNotEqual(copy.credentialUnknown, copy.credentialAbsent)
        XCTAssertNotEqual(copy.credentialUnreported, copy.credentialAbsent)
    }

    /// Exactly one state is painted as settled, and it is the one that means
    /// a key is here.
    func testOnlyAKeyThatIsHereIsPaintedClear() {
        let tone = { (label: String) -> PrivateInferenceTone in
            CredentialSurface.tone(CredentialStatus(state: label), calls: self.calls())
        }
        XCTAssertEqual(tone("present"), .clear)
        XCTAssertEqual(tone("obtaining"), .held)
        XCTAssertEqual(tone("failed"), .refused)
        XCTAssertEqual(tone("absent"), .neutral)
        XCTAssertEqual(tone("cancelled"), .neutral)
        XCTAssertEqual(tone("a_credential_state_from_a_later_daemon"), .neutral)
        XCTAssertEqual(tone(""), .neutral)
    }

    /// The button beside each state, across the boundary.
    ///
    /// The state nobody could read gets NO button. That is the one arm worth
    /// naming: `obtain` there is how a contributor ends up holding a second
    /// key in an account nothing on the screen will ever mention again.
    func testTheButtonBesideEachStateCrossesTheAbi() {
        let action = { (label: String) -> CredentialAction in
            CredentialSurface.action(CredentialStatus(state: label), calls: self.calls())
        }
        XCTAssertEqual(action("absent"), .obtain)
        XCTAssertEqual(action("failed"), .obtain)
        XCTAssertEqual(action("cancelled"), .obtain)
        XCTAssertEqual(action("obtaining"), .cancel)
        XCTAssertEqual(action("present"), .forget)
        XCTAssertEqual(action("a_credential_state_from_a_later_daemon"), CredentialAction.none)
        XCTAssertEqual(action(""), CredentialAction.none)
    }

    /// The action codes are their own range, disjoint from the tones. A shell
    /// that cross-wired the two mappers would draw a button from a colour, so
    /// neither decoder may give the other's values a meaning.
    func testTheActionRangeAndTheToneRangeDoNotOverlap() {
        for tone: Int32 in [21, 22, 23, 24] {
            XCTAssertEqual(CredentialAction.fromABI(tone), .none, "tone \(tone) is not an action")
        }
        for action: Int32 in [31, 32, 33] {
            XCTAssertEqual(
                PrivateInferenceTone.fromABI(action), .neutral,
                "action \(action) is not a tone")
        }
    }

    /// The sentence in front of the sign-in names all three consequences.
    ///
    /// Named rather than checked as "some field is non-empty": a build that
    /// dropped one of them would still draw a button, and the button would be
    /// a lie by omission.
    func testTheCostSentenceNamesWhatPressingItDoes() throws {
        let copy = try XCTUnwrap(copy())
        let cost = copy.credentialCost
        XCTAssertTrue(cost.contains("browser"), cost)
        XCTAssertTrue(cost.contains("your own Private AI account"), cost)
        XCTAssertEqual(CredentialSurface.actionExplains(.obtain, copy: copy), cost)
    }

    /// Forgetting says it is local. `handle_forget` answers `revoked: false`
    /// and always will; this is that fact in words, beside the button.
    func testForgettingSaysTheKeyStaysValidElsewhere() throws {
        let copy = try XCTUnwrap(copy())
        XCTAssertEqual(
            CredentialSurface.actionExplains(.forget, copy: copy),
            copy.credentialForgetExplains)
        XCTAssertFalse(copy.credentialForgetExplains.isEmpty)
    }

    /// The tri-state crosses as three values, and only the middle one draws
    /// a sentence.
    func testTheConnectNoticeCrossesAsATriState() throws {
        let copy = try XCTUnwrap(copy())
        XCTAssertEqual(
            CredentialSurface.harnessNotice(credentialed: false, calls: calls()),
            copy.harnessNeedsCredential)
        XCTAssertNil(CredentialSurface.harnessNotice(credentialed: true, calls: calls()))
        XCTAssertNil(CredentialSurface.harnessNotice(credentialed: nil, calls: calls()))
    }

    /// Nothing on this surface has a hole a key could be poured into.
    ///
    /// The export guard already refuses a template marker in any field; this
    /// says the same thing about the words a reader would recognise as an
    /// invitation to interpolate an identifier.
    func testNoCredentialSentenceCanCarryAValue() throws {
        let copy = try XCTUnwrap(copy())
        let sentences = [
            copy.credentialTitle, copy.credentialWhat, copy.credentialCost,
            copy.credentialObtain, copy.credentialGoogle, copy.credentialGithub, copy.credentialCancel, copy.credentialForget,
            copy.credentialForgetExplains, copy.credentialAbsent, copy.credentialObtaining,
            copy.credentialFailed, copy.credentialCancelled, copy.credentialPresent,
            copy.credentialUnknown, copy.credentialUnreported, copy.harnessNeedsCredential,
        ]
        for sentence in sentences {
            XCTAssertFalse(sentence.isEmpty)
            for marker in ["{}", "{key}", "{prefix}", "%@", "%s", "%d"] {
                XCTAssertFalse(sentence.contains(marker), "\(sentence) has a hole for a value")
            }
        }
    }
}
