import XCTest

/// The Private AI switch lives in one place: the Private AI section of
/// Settings (owner, 2026-10-10), with the local tools and sign-in. It used
/// to be on the Private AI tab, with a pointer here; it moved back, and the
/// tab now carries a card that opens this section instead.
///
/// Two switches for one thing is two places for them to disagree, so this
/// reads both sources: the switch is drawn by the Settings panels, and the
/// tab draws none.
final class SettingsPointerTests: XCTestCase {
    private static func source(_ path: String) throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()  // TCShellCoreTests
            .deletingLastPathComponent()  // Tests
            .deletingLastPathComponent()  // macos
            .appendingPathComponent("Sources/TraceCommonsApp/Views/" + path)
        return try String(contentsOf: url, encoding: .utf8)
    }

    /// Settings draws the panels that hold the switch, and no pointer back.
    func testTheSettingsEntryHoldsTheSwitch() throws {
        let settings = try Self.source("Settings/PrivateAISection.swift")
        XCTAssertTrue(settings.contains("PrivateAISettingsPanels(store: store)"),
                      "the settings entry must draw the switch and the tools")
        XCTAssertFalse(settings.contains("copy.settingsMoved"),
                       "the settings entry must not say the switch moved away")
    }

    /// The switch is drawn once, by the Settings panels, and writes through
    /// the data contract, never the model's direct write.
    func testTheSwitchIsDrawnOnce() throws {
        let account = try Self.source("Monitor/InferenceAccount.swift")
        let panels = try XCTUnwrap(account.components(separatedBy: "struct PrivateAISettingsPanels").last)
        let tab = try XCTUnwrap(account.components(separatedBy: "struct PrivateAISettingsPanels").first)
        XCTAssertEqual(account.components(separatedBy: "PrivateAISwitchCard(").count - 1, 1)
        XCTAssertTrue(panels.contains("PrivateAISwitchCard("))
        XCTAssertFalse(tab.contains("PrivateAISwitchCard("), "the tab draws a second switch")
        XCTAssertFalse(account.contains("model.apply" + "PrivateInference"),
                       "the switch writes through the data contract")
    }
}
