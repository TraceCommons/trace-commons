import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

/// Home's reminder while Train coding models is off (product owner,
/// 2026-11-05): one slim amber line at the top of Home, with Open
/// Settings, until it is on. Amber, never red, no X, and none in first run.
@MainActor
final class NoCreditReminderTests: XCTestCase {
    private func source(_ path: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(path), encoding: .utf8)
    }

    func test_theWordsAreTheOwners() throws {
        let home = try XCTUnwrap(MonitorWords.table).homeHistory
        XCTAssertEqual(
            home.noCreditBody,
            "You won't earn credit until Train coding models is on. You can turn it on anytime in Settings.")
        XCTAssertEqual(home.noCreditOpenSettings, "Open Settings")
    }

    func test_homeRemindsASignedInAccountUntilTrainingIsOn() {
        XCTAssertTrue(NoCreditNotice.showsOnHome(loggedIn: true, scopes: ["debugging_evaluation"]))
        XCTAssertFalse(NoCreditNotice.showsOnHome(loggedIn: true, scopes: ["debugging_evaluation", "model_training"]))
        XCTAssertFalse(NoCreditNotice.showsOnHome(loggedIn: false, scopes: []))
    }

    func test_itIsOneAmberLineUnderTheTracesHeadingWithNoX() throws {
        let notice = try source("Views/NoCreditNotice.swift")
        XCTAssertTrue(notice.contains(".foregroundStyle(GlassStatus.ask.color)"))
        XCTAssertTrue(notice.contains(".fill(GlassStatus.ask.color.opacity(0.12))"))
        // Open Settings is a glass pill, not a link (Ron, 2026-11-09).
        XCTAssertTrue(notice.contains(".buttonStyle(GlassButtonStyle(.glass))"))
        XCTAssertFalse(notice.contains(".buttonStyle(GlassButtonStyle(.link))"))
        XCTAssertFalse(notice.contains(".outside"), "never red")
        XCTAssertFalse(notice.contains("xmark"))
        XCTAssertFalse(notice.contains("@AppStorage"), "nothing remembers it closed")
        // Under the Traces heading, over its cards (Ron, 2026-11-09; it led
        // Home from 2026-11-05), and nowhere else.
        let home = try source("Views/Monitor/HomeViews.swift")
        let traces = try XCTUnwrap(home.range(of: "HomeSection(words?.sectionTraces) {\n"))
        let line = try XCTUnwrap(home.range(of: "                noCredit\n                if showsTracesWalkthrough {",
                                            range: traces.upperBound..<home.endIndex))
        XCTAssertLessThan(traces.upperBound, line.lowerBound)
        XCTAssertEqual(home.components(separatedBy: "noCredit\n").count - 1, 1, "drawn once")
        XCTAssertTrue(home.contains("perform: openSettings)"))
        let window = try source("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("openSettings: { navigation.requestSettings(at: .consent) },"))
        // No pop-up in first run (owner, 2026-11-05).
        let join = try source("Views/FirstRun/JoinScreen.swift")
        XCTAssertFalse(join.contains("NoCreditNotice"))
    }
}
