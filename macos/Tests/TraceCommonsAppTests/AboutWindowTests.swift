import XCTest

@testable import TraceCommonsApp

/// The app's own About window (owner, 2026-11-09): the standard panel's
/// version form, a Copy button for it, and the app menu's About opening it.
final class AboutWindowTests: XCTestCase {
    func testVersionLineMatchesTheStandardPanel() {
        XCTAssertEqual(AboutLines.line(short: "0.0.0-dev", build: "1"), "0.0.0-dev (1)")
        XCTAssertEqual(AboutLines.line(short: "0.24.15", build: "0.24.15"), "0.24.15", "no repeated build")
        XCTAssertEqual(AboutLines.line(short: "0.24.15", build: nil), "0.24.15")
        XCTAssertEqual(AboutLines.line(short: "0.24.15", build: ""), "0.24.15")
        XCTAssertEqual(AboutLines.line(short: nil, build: "7"), "7")
    }

    func testTheAppMenuOpensTheAboutWindowWithACopyButton() throws {
        let main = try SettingsParityTests.text("TraceCommonsAppMain.swift")
        XCTAssertTrue(main.contains("CommandGroup(replacing: .appInfo) {\n            OpenAboutButton()"))
        XCTAssertTrue(main.contains("openWindow(id: WindowID.about)"))
        XCTAssertTrue(main.contains("Window(MonitorWords.table?.aboutApp ?? \"\", id: WindowID.about)"))
        let view = try SettingsParityTests.text("Views/AboutWindowView.swift")
        XCTAssertTrue(view.contains("Button(SettingsLegacyWords.copy) { AboutLines.copyVersion() }"))
        XCTAssertTrue(view.contains("NSPasteboard.general.setString(version, forType: .string)"))
    }
}
