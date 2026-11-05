import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

/// Home's reminder while Train coding models is off (product owner,
/// 2026-11-05): one slim amber line under the Traces hero, with Open
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

    func test_itIsOneAmberLineUnderTheHeroWithNoX() throws {
        let notice = try source("Views/NoCreditNotice.swift")
        XCTAssertTrue(notice.contains(".foregroundStyle(GlassStatus.ask.color)"))
        XCTAssertTrue(notice.contains(".fill(GlassStatus.ask.color.opacity(0.12))"))
        XCTAssertTrue(notice.contains(".buttonStyle(GlassButtonStyle(.link))"))
        XCTAssertFalse(notice.contains(".outside"), "never red")
        XCTAssertFalse(notice.contains("xmark"))
        XCTAssertFalse(notice.contains("@AppStorage"), "nothing remembers it closed")
        // Under the owner's Traces hero, which stays first on Home.
        let home = try source("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("            tracesHero\n            noCredit\n            privateAI"))
        XCTAssertTrue(home.contains("perform: openSettings)"))
        let window = try source("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("openSettings: { navigation.requestSettings(at: .consent) },"))
        // No pop-up in first run (owner, 2026-11-05).
        let join = try source("Views/FirstRun/JoinScreen.swift")
        XCTAssertFalse(join.contains("NoCreditNotice"))
    }
}
