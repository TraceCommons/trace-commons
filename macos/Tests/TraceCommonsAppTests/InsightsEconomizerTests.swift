import XCTest

import TCBridge
import TCShellCore
@testable import TraceCommonsApp

/// The Economizer tab (2026-11-01): its words from the core's copy, and a
/// model that shows only what the daemon confirmed.
@MainActor
final class InsightsEconomizerTests: XCTestCase {
    nonisolated static let folder = DaemonData.EconomizerFolder(
        folder: "my-app", harness: "claude", key: "repo-1", sessions: 6,
        leftOut: ["CronCreate", "TodoWrite"], givenBack: ["WebFetch"],
        told: [.init(program: "pytest", prefix: "python -m")], forming: 2)

    func testACardReadsTheCoresWords() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        XCTAssertEqual(copy["analytics_tab_economizer"], "Economizer")
        let card = InsightsEconomizerWords.card(Self.folder, copy: copy)
        XCTAssertEqual(card.title, "my-app · Claude")
        XCTAssertEqual(card.sessions, "Learned from 6 sessions")
        XCTAssertEqual(card.leftOut, "CronCreate, TodoWrite")
        XCTAssertTrue(card.canGiveBack)
        XCTAssertEqual(card.givenBack, "WebFetch")
        XCTAssertEqual(card.told, ["pytest runs as python -m pytest"])
        XCTAssertEqual(card.forming, "2 more being confirmed")

        let quiet = InsightsEconomizerWords.card(
            .init(folder: "api", harness: "opencode", key: "repo-2", sessions: 1), copy: copy)
        XCTAssertEqual(quiet.title, "api · OpenCode")
        XCTAssertEqual(quiet.sessions, "Learned from 1 session")
        XCTAssertEqual(quiet.leftOut, copy["analytics_econ_none"])
        XCTAssertFalse(quiet.canGiveBack)
        XCTAssertNil(quiet.givenBack)
        XCTAssertNil(quiet.forming)
        XCTAssertNil(InsightsEconomizerWords.truncated(0, copy: copy))
        XCTAssertEqual(InsightsEconomizerWords.truncated(3, copy: copy), "3 more folders not shown")
    }

    func testTheSwitchShowsWhatTheDaemonConfirmedAndGiveBackReadsAgain() async {
        final class Box: @unchecked Sendable {
            var enabled = false
            var folder = InsightsEconomizerTests.folder
            var restored: [String] = []
        }
        let box = Box()
        let model = InsightsEconomizerModel(calls: .init(
            status: { DaemonData.EconomizerStatus(enabled: box.enabled, folders: [box.folder]) },
            // The daemon refuses to turn on: the switch stays off.
            setEnabled: { _ in DaemonData.EconomizerSwitch(enabled: box.enabled) },
            restore: { harness, key in
                box.restored = [harness, key]
                box.folder = .init(folder: "my-app", harness: "claude", key: "repo-1", sessions: 6,
                                   givenBack: ["CronCreate", "TodoWrite", "WebFetch"])
                return DaemonData.EconomizerRestored(restored: ["CronCreate", "TodoWrite"])
            }))
        XCTAssertTrue(model.available)
        await model.load()
        XCTAssertEqual(model.answer?.enabled, false)
        await model.setEnabled(true)
        XCTAssertEqual(model.answer?.enabled, false)
        XCTAssertNotNil(model.refusal)

        await model.restore(Self.folder)
        XCTAssertEqual(box.restored, ["claude", "repo-1"])
        XCTAssertEqual(model.answer?.folders.first?.leftOut.isEmpty, true)
        XCTAssertNil(model.refusal)
        XCTAssertTrue(model.restoring.isEmpty)
    }

    func testWithoutADaemonTheTabIsUnavailable() async {
        let model = InsightsEconomizerModel(calls: nil)
        XCTAssertFalse(model.available)
        await model.load()
        XCTAssertTrue(model.failed)
        XCTAssertNil(model.answer)
    }
}
