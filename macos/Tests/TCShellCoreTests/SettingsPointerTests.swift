import XCTest

/// Settings points at the model-calls destination; it does not hold the
/// switch any more.
///
/// The entry stays. A contributor who learned where the switch was should
/// find a pointer there, not a hole -- which is why this reads the settings
/// source for the pointer as well as for the absence of the control. The
/// sentence is `settingsMoved` and it comes from the Rust like every other
/// word on this surface; the label on the way out is `destination`, the same
/// word the sidebar carries.
final class SettingsPointerTests: XCTestCase {
    private static func settingsSource() throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()  // TCShellCoreTests
            .deletingLastPathComponent()  // Tests
            .deletingLastPathComponent()  // macos
            .appendingPathComponent("Sources/TraceCommonsApp/Views/SettingsView.swift")
        return try String(contentsOf: url, encoding: .utf8)
    }

    /// The pointer is there, and it is the Rust's sentence.
    func testTheSettingsEntryShowsThePointerSentence() throws {
        let source = try Self.settingsSource()
        XCTAssertTrue(
            source.contains("copy.settingsMoved"),
            "the settings entry must show the sentence saying where the switch went")
        XCTAssertTrue(
            source.contains("navigation?.section = .privateInference"),
            "the pointer must be a way to the destination, not only a sentence")
    }

    /// The control itself is gone from here. Two switches for one thing is
    /// two places for them to disagree, and the one that stays is the one on
    /// the destination that also reports what actually happened.
    func testTheSwitchIsNoLongerOnTheSettingsEntry() throws {
        let source = try Self.settingsSource()
        XCTAssertFalse(
            source.contains("copy." + "settingsToggle"),
            "the switch belongs on the model-calls destination, not on Settings")
        XCTAssertFalse(
            source.contains("model.apply" + "PrivateInference"),
            "Settings must not write the setting; the destination does")
    }
}
