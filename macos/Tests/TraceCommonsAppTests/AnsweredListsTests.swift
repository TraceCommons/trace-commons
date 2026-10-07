import TCBridge
import XCTest

@testable import TraceCommonsApp

/// Answers `list_pending` and `list_history` with empty lists, or fails
/// every call.
private final class EmptyListsDaemon: DaemonCalling, @unchecked Sendable {
    let fails: Bool

    init(fails: Bool) {
        self.fails = fails
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        guard !fails else {
            return #"{"id":1,"error":{"code":"unavailable","message":"synthetic-test-failure"}}"#
        }
        switch method {
        case "list_pending": return #"{"id":1,"result":{"pending":[]}}"#
        case "list_history": return #"{"id":1,"result":{"history":[]}}"#
        default: return #"{"id":1,"error":{"code":"unavailable","message":"unexpected-test-method"}}"#
        }
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }
}

/// An empty queue or history is an answer only once the daemon gave it:
/// before then (and after a failed read) the lists are a placeholder, and
/// nothing may read them as "none".
final class AnsweredListsTests: XCTestCase {
    @MainActor
    func testTheListsAreUnansweredUntilTheDaemonAnswers() async throws {
        let model = AppModel()
        XCTAssertFalse(model.queueAnswered)
        XCTAssertFalse(model.historyAnswered)

        model.setClientForTesting(DaemonClient(daemon: EmptyListsDaemon(fails: false)))
        model.refreshQueue()
        model.refreshHistory()
        try await waitUntil { model.queueAnswered && model.historyAnswered }
        XCTAssertTrue(model.awaitingDecision.isEmpty)
        XCTAssertTrue(model.history.isEmpty)
    }

    @MainActor
    func testAFailedReadIsNotAnAnswer() async throws {
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: EmptyListsDaemon(fails: true)))
        model.refreshQueue()
        model.refreshHistory()
        try await waitUntil { model.lastActionError != nil }
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertFalse(model.queueAnswered)
        XCTAssertFalse(model.historyAnswered)
    }

    /// A snapshot carries the queue too.
    @MainActor
    func testASnapshotAnswersTheQueue() {
        let model = AppModel()
        model.applyPendingUpdate([])
        XCTAssertTrue(model.queueAnswered)
        XCTAssertFalse(model.historyAnswered)
    }

    @MainActor
    private func waitUntil(
        timeout: Duration = .seconds(3),
        condition: @escaping @MainActor () -> Bool
    ) async throws {
        let clock = ContinuousClock()
        let deadline = clock.now.advanced(by: timeout)
        while !condition() {
            if clock.now >= deadline {
                XCTFail("Timed out waiting for the daemon's answer")
                return
            }
            try await Task.sleep(for: .milliseconds(20))
        }
    }
}
