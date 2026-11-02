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
            served: { true },
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
        XCTAssertNotNil(model.switchRefusal)

        await model.restore(Self.folder)
        XCTAssertEqual(box.restored, ["claude", "repo-1"])
        XCTAssertEqual(model.answer?.folders.first?.leftOut.isEmpty, true)
        XCTAssertTrue(model.folderRefusals.isEmpty)
        XCTAssertTrue(model.restoring.isEmpty)
    }

    func testAFailedGiveBackStaysOnItsCardAndKeepsTheAnswer() async {
        final class Box: @unchecked Sendable { var reads = 0 }
        let box = Box()
        struct Refused: Error {}
        let model = InsightsEconomizerModel(calls: .init(
            served: { true },
            status: {
                box.reads += 1
                // The read after the failed write fails too.
                if box.reads > 1 { throw Refused() }
                return DaemonData.EconomizerStatus(enabled: true, folders: [InsightsEconomizerTests.folder])
            },
            setEnabled: { DaemonData.EconomizerSwitch(enabled: $0) },
            restore: { _, _ in throw Refused() }))
        await model.load()
        await model.restore(Self.folder)
        XCTAssertNotNil(model.folderRefusals["repo-1"])
        XCTAssertNil(model.switchRefusal)
        XCTAssertFalse(model.failed)
        XCTAssertEqual(model.answer?.folders.first?.key, "repo-1")
        // The page is kept, and marked as possibly out of date.
        XCTAssertTrue(model.stale)
    }

    func testAConfirmedSwitchIsDrawnEvenWhenTheReadAfterItFails() async {
        final class Box: @unchecked Sendable { var reads = 0 }
        let box = Box()
        struct Stopped: Error {}
        let model = InsightsEconomizerModel(calls: .init(
            served: { true },
            status: {
                box.reads += 1
                if box.reads > 1 { throw Stopped() }
                return DaemonData.EconomizerStatus(enabled: false, folders: [InsightsEconomizerTests.folder])
            },
            setEnabled: { DaemonData.EconomizerSwitch(enabled: $0) },
            restore: { _, _ in DaemonData.EconomizerRestored(restored: []) }))
        await model.load()
        await model.setEnabled(true)
        XCTAssertEqual(model.answer?.enabled, true)
        XCTAssertEqual(model.answer?.folders.first?.key, "repo-1")
        XCTAssertNil(model.switchRefusal)
        XCTAssertTrue(model.stale)
    }

    func testAReadBegunBeforeTheSwitchCannotOverwriteIt() async {
        // Reads 2 and 3 are each held until released: read 2 began before
        // the switch and answers "off"; read 3 is the one after it, and fails.
        final class Gates: @unchecked Sendable {
            var early: CheckedContinuation<Void, Never>?
            var after: CheckedContinuation<Void, Never>?
            var reads = 0
        }
        let gates = Gates()
        struct Stopped: Error {}
        let model = InsightsEconomizerModel(calls: .init(
            served: { true },
            status: {
                gates.reads += 1
                switch gates.reads {
                case 1: return DaemonData.EconomizerStatus(enabled: false)
                case 2:
                    await withCheckedContinuation { gates.early = $0 }
                    return DaemonData.EconomizerStatus(enabled: false)
                default:
                    await withCheckedContinuation { gates.after = $0 }
                    throw Stopped()
                }
            },
            setEnabled: { DaemonData.EconomizerSwitch(enabled: $0) },
            restore: { _, _ in DaemonData.EconomizerRestored(restored: []) }))
        await model.load()
        let early = Task { await model.load() }
        while gates.early == nil { await Task.yield() }
        let switching = Task { await model.setEnabled(true) }
        while gates.after == nil { await Task.yield() }
        // The switch is confirmed and its read is pending: the earlier read
        // lands now, then the read after the switch fails.
        gates.early?.resume()
        await early.value
        XCTAssertEqual(model.answer?.enabled, true)
        gates.after?.resume()
        await switching.value
        XCTAssertEqual(model.answer?.enabled, true)
        XCTAssertTrue(model.stale)
    }

    func testASwitchConfirmedWithNoPageShownStillDropsAnEarlierRead() async {
        // Read 1 fails, so no page is shown. Read 2 began before the switch
        // and answers "off"; read 3, after the switch, answers "on".
        final class Gates: @unchecked Sendable {
            var early: CheckedContinuation<Void, Never>?
            var after: CheckedContinuation<Void, Never>?
            var reads = 0
        }
        let gates = Gates()
        struct Stopped: Error {}
        let model = InsightsEconomizerModel(calls: .init(
            served: { true },
            status: {
                gates.reads += 1
                switch gates.reads {
                case 1: throw Stopped()
                case 2:
                    await withCheckedContinuation { gates.early = $0 }
                    return DaemonData.EconomizerStatus(enabled: false)
                default:
                    await withCheckedContinuation { gates.after = $0 }
                    return DaemonData.EconomizerStatus(enabled: true)
                }
            },
            setEnabled: { DaemonData.EconomizerSwitch(enabled: $0) },
            restore: { _, _ in DaemonData.EconomizerRestored(restored: []) }))
        await model.load()
        XCTAssertNil(model.answer)
        let early = Task { await model.load() }
        while gates.early == nil { await Task.yield() }
        let switching = Task { await model.setEnabled(true) }
        while gates.after == nil { await Task.yield() }
        // The read begun before the switch lands: it is dropped.
        gates.early?.resume()
        await early.value
        XCTAssertNil(model.answer)
        gates.after?.resume()
        await switching.value
        XCTAssertEqual(model.answer?.enabled, true)
    }

    func testAFailedReadWhenTheTabOpensIsNotHiddenByAnEarlierAnswer() async {
        final class Box: @unchecked Sendable { var up = true }
        let box = Box()
        struct Stopped: Error {}
        let model = InsightsEconomizerModel(calls: .init(
            served: { true },
            status: {
                guard box.up else { throw Stopped() }
                return DaemonData.EconomizerStatus(enabled: true, folders: [InsightsEconomizerTests.folder])
            },
            setEnabled: { DaemonData.EconomizerSwitch(enabled: $0) },
            restore: { _, _ in DaemonData.EconomizerRestored(restored: []) }))
        await model.load()
        XCTAssertNotNil(model.answer)
        // The daemon stops; the next time the tab opens it says so.
        box.up = false
        await model.load()
        XCTAssertTrue(model.failed)
        XCTAssertNil(model.answer)
        // It comes back.
        box.up = true
        await model.load()
        XCTAssertFalse(model.failed)
        XCTAssertFalse(model.stale)
        XCTAssertNotNil(model.answer)
    }

    func testTheProbeHidesAndBringsBackTheTabWithoutReadingTheStore() async {
        final class Box: @unchecked Sendable { var serves = false; var reads = 0 }
        let box = Box()
        let model = InsightsEconomizerModel(calls: .init(
            served: { box.serves },
            status: {
                box.reads += 1
                return DaemonData.EconomizerStatus(enabled: false)
            },
            setEnabled: { DaemonData.EconomizerSwitch(enabled: $0) },
            restore: { _, _ in DaemonData.EconomizerRestored(restored: []) }))
        await model.probe()
        XCTAssertFalse(model.available)
        // The daemon is upgraded while the app runs.
        box.serves = true
        await model.probe()
        XCTAssertTrue(model.available)
        XCTAssertEqual(box.reads, 0)
    }

    func testADaemonThatPredatesTheEconomizerHidesTheTab() async {
        let model = InsightsEconomizerModel(calls: .init(
            served: { true },
            status: { throw DaemonDataError.daemon(code: "unknown_method", message: "unknown_method") },
            setEnabled: { DaemonData.EconomizerSwitch(enabled: $0) },
            restore: { _, _ in DaemonData.EconomizerRestored(restored: []) }))
        XCTAssertTrue(model.available)
        await model.load()
        XCTAssertTrue(model.notServed)
        XCTAssertFalse(model.available)
    }

    func testAnOlderReplyIsDroppedButACancelledNewerReadDropsNothing() async {
        final class Box: @unchecked Sendable { var reads = 0 }
        let box = Box()
        let model = InsightsEconomizerModel(calls: .init(
            served: { true },
            status: {
                box.reads += 1
                if box.reads == 2 { throw CancellationError() }
                return DaemonData.EconomizerStatus(enabled: true)
            },
            setEnabled: { DaemonData.EconomizerSwitch(enabled: $0) },
            restore: { _, _ in DaemonData.EconomizerRestored(restored: []) }))
        await model.load()
        // The second read is cancelled: the first answer stands, nothing failed.
        await model.load()
        XCTAssertEqual(model.answer?.enabled, true)
        XCTAssertFalse(model.failed)
    }

    func testWithoutADaemonTheTabIsUnavailable() async {
        let model = InsightsEconomizerModel(calls: nil)
        XCTAssertFalse(model.available)
        await model.load()
        XCTAssertTrue(model.failed)
        XCTAssertNil(model.answer)
    }
}
