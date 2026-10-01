import Combine
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

private final class DecisionsStatusDaemon: DaemonCalling {
    func call(_ method: String, params paramsJSON: String) -> String {
        guard method == "status" else {
            return #"{"id":1,"error":{"code":"unavailable","message":"unexpected-test-method"}}"#
        }
        return #"{"id":1,"result":{"queue_depth":50,"decisions_owed":5}}"#
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }
}

final class DecisionsOwedBindingTests: XCTestCase {
    private func status(_ count: Int?, depth: Int = 50) throws -> DaemonStatus {
        let field = count.map { ",\"decisions_owed\":\($0)" } ?? ""
        return try JSONDecoder().decode(DaemonStatus.self, from: Data("""
            {"queue_depth":\(depth)\(field)}
            """.utf8))
    }

    private func pendingEntry() throws -> QueueEntry {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode(QueueEntry.self, from: Data(#"{"entry_id":"e1","session_hash":"sha256:a","source":"claude_code","project_id":"p1","project_label":"repo","size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z","state":"pending","attempts":0}"#.utf8))
    }

    @MainActor
    func testBadgeUsesTheDaemonCountEvenWhenTheReviewListDiffers() throws {
        let model = AppModel()
        model.applyPendingUpdate([try pendingEntry()])
        model.setStatusForTesting(try status(3))
        XCTAssertEqual(model.decisionsOwed, 3)
        XCTAssertEqual(model.awaitingDecision.count, 1, "review lists are unchanged")
        XCTAssertEqual(MenuBarStatus.state(decisionsOwed: model.decisionsOwed, unhealthy: false, paused: false), .count("3"))
        model.applyPendingUpdate([])
        XCTAssertEqual(model.decisionsOwed, 3, "a list refresh cannot rewrite the daemon badge")
    }

    @MainActor
    func testAnOlderDaemonDoesNotFabricateAZeroOrPendingCount() throws {
        let model = AppModel()
        model.applyPendingUpdate([try pendingEntry()])
        model.setStatusForTesting(try status(nil))
        XCTAssertNil(model.decisionsOwed as Int?)
        XCTAssertEqual(MenuBarStatus.state(decisionsOwed: model.decisionsOwed, unhealthy: false, paused: false), .attention)
        XCTAssertEqual(model.awaitingDecision.count, 1)
    }

    @MainActor
    func testStatusPublishesUpdatedBadgeWithoutChangingTheQueue() throws {
        let model = AppModel()
        var publications = 0
        let subscription = model.objectWillChange.sink { _ in publications += 1 }
        defer { subscription.cancel() }
        model.setStatusForTesting(try status(4))
        XCTAssertEqual(model.decisionsOwed, 4)
        model.setStatusForTesting(try status(0))
        XCTAssertGreaterThan(publications, 0)
        XCTAssertEqual(model.decisionsOwed, 0)
        XCTAssertEqual(MenuBarStatus.state(decisionsOwed: model.decisionsOwed, unhealthy: false, paused: false), .idle)
        model.setStatusForTesting(try status(nil))
        XCTAssertNil(model.decisionsOwed as Int?, "a later older-daemon status clears the earlier count")
    }

    @MainActor
    func testRefreshStatusPublishesTheWireCountWithoutAQueueRefresh() async throws {
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: DecisionsStatusDaemon()))
        let updated = expectation(description: "daemon count published")
        let subscription = model.$status.dropFirst().sink { status in
            if status.decisionsOwed == 5 { updated.fulfill() }
        }
        defer { subscription.cancel() }

        model.refreshStatus()
        await fulfillment(of: [updated], timeout: 2)

        XCTAssertEqual(model.decisionsOwed, 5)
        XCTAssertTrue(model.awaitingDecision.isEmpty)
        XCTAssertEqual(model.status.queueDepth, 50)
    }
}
