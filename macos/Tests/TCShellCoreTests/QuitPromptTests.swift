import XCTest
@testable import TCShellCore

/// When quitting asks, and what a malformed prompt does. This target does
/// not link the core, so the words themselves are asserted against the real
/// export in `TCBridgeTests/CoreCopyExportTests`.
final class QuitPromptTests: XCTestCase {
    private let valid = #"{"title": "T", "body": "B.", "confirm": "Quit", "cancel": "Cancel"}"#

    /// Nil, never a partly-filled value: an empty sentence or a missing
    /// field is refused rather than shown half-written.
    func testRefusesAnEmptyFieldAMissingFieldAndNoPayload() {
        XCTAssertNil(QuitPrompt.decode(fromJSON: #"{"title": "T", "body": "", "confirm": "Quit", "cancel": "Cancel"}"#))
        XCTAssertNil(QuitPrompt.decode(fromJSON: #"{"title": "T"}"#))
        XCTAssertNil(QuitPrompt.decode(fromJSON: "not json"))
        XCTAssertNil(QuitPrompt.decode(fromJSON: nil))
    }

    /// Nothing live: quit at once, and the core is not even asked.
    func testNothingLiveQuitsWithoutAsking() {
        var asked = false
        let decision = QuitPrompt.decision(stopsCompute: false, stopsPrivateAI: false) { _, _ in
            asked = true
            return self.valid
        }
        XCTAssertEqual(decision, .quitNow)
        XCTAssertFalse(asked)
    }

    /// Something live asks, with the facts passed through to the core.
    func testSomethingLiveAsksWithTheCoresPrompt() throws {
        for (compute, privateAI) in [(true, false), (false, true), (true, true)] {
            var passed: (Bool, Bool)?
            let decision = QuitPrompt.decision(stopsCompute: compute, stopsPrivateAI: privateAI) { c, p in
                passed = (c, p)
                return self.valid
            }
            XCTAssertEqual(decision, .ask(try XCTUnwrap(QuitPrompt.decode(fromJSON: valid))))
            XCTAssertEqual(passed?.0, compute)
            XCTAssertEqual(passed?.1, privateAI)
        }
    }

    /// A prompt that cannot be read while something is live refuses the
    /// quit. It never becomes a silent quit.
    func testAnUnreadablePromptWhileSomethingIsLiveRefuses() {
        XCTAssertEqual(QuitPrompt.decision(stopsCompute: true, stopsPrivateAI: false) { _, _ in nil }, .refuse)
        XCTAssertEqual(QuitPrompt.decision(stopsCompute: false, stopsPrivateAI: true) { _, _ in "{}" }, .refuse)
    }
}
