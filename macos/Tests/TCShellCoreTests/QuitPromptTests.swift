import XCTest
@testable import TCShellCore

/// Refusing a malformed quit prompt (`tc_quit_prompt_json`). This target
/// does not link the core, so the words themselves, and that each role
/// keeps its own body, are asserted against the real export in
/// `TCBridgeTests/CoreCopyExportTests` (`testEachRoleDecodesWithItsOwnBody`).
///
/// The counterpart of the Tauri frontend's `quit-confirmation-copy.test.mjs`
/// (K8, #1173) for "a missing sentence is refused rather than guessed".
/// Unlike that test, an unknown role is NOT refused here: `QuitPrompt.role`
/// is any non-empty string, so a role the core never sends still decodes.
/// That divergence is recorded in #1212, not fixed.
final class QuitPromptTests: XCTestCase {
    private func fixture(role: String, body: String = "B") -> String {
        """
        {"role": "\(role)", "title": "T", "body": "\(body)", "confirm": "Quit", "cancel": "Cancel"}
        """
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
