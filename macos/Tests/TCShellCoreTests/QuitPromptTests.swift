import XCTest
@testable import TCShellCore

/// Decoding the core's quit prompt (`tc_quit_prompt_json`). The words
/// themselves, per role, are asserted against the real export in
/// `TCBridgeTests/CoreCopyExportTests`.
///
/// The counterpart of the Tauri frontend's `quit-confirmation-copy.test.mjs`
/// (K8, #1173): "each watcher role keeps the sentence Rust chose for it",
/// and "an unknown role or a missing sentence is refused rather than
/// guessed".
final class QuitPromptTests: XCTestCase {
    private func fixture(role: String, body: String = "B") -> String {
        """
        {"role": "\(role)", "title": "T", "body": "\(body)", "confirm": "Quit", "cancel": "Cancel"}
        """
    }

    func testEachRoleDecodesWithItsOwnBody() throws {
        for role in ["hosting", "attached", "unavailable"] {
            let prompt = try XCTUnwrap(QuitPrompt.decode(fromJSON: fixture(role: role, body: "\(role) body")))
            XCTAssertEqual(prompt.role, role)
            XCTAssertEqual(prompt.body, "\(role) body")
            XCTAssertEqual(prompt.title, "T")
            XCTAssertEqual(prompt.confirm, "Quit")
            XCTAssertEqual(prompt.cancel, "Cancel")
        }
    }

    /// Nil, never a partly-filled value: an empty sentence or a missing
    /// field is refused rather than shown half-written, and the quit is not
    /// confirmable before the true sentence for this process is shown.
    func testRefusesAnEmptyFieldAMissingFieldAndNoPayload() {
        XCTAssertNil(QuitPrompt.decode(fromJSON: fixture(role: "hosting", body: "")))
        XCTAssertNil(QuitPrompt.decode(fromJSON: #"{"role": "hosting"}"#))
        XCTAssertNil(QuitPrompt.decode(fromJSON: "not json"))
        XCTAssertNil(QuitPrompt.decode(fromJSON: nil))
    }
}
