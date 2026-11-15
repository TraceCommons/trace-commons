import XCTest
import TCShellCore
@testable import TraceCommonsApp

@MainActor
final class InsightsSkillsRecoveryTests: XCTestCase {
    private actor FirstFailure {
        let unreadable: Bool
        private(set) var requests: [Bool] = []
        init(unreadable: Bool) { self.unreadable = unreadable }
        func read(_ refresh: Bool) throws -> DaemonData.SkillsSnapshot {
            requests.append(refresh)
            if requests.count == 1 {
                if unreadable { return .init(enabled: true, readable: false) }
                throw DaemonDataError.notAvailableYet(method: "insights_skills")
            }
            return .init(enabled: true, followedPublishers: ["recovered"])
        }
    }

    func testFirstTransportFailureRecoversWithoutManualRefresh() async throws {
        try await assertFirstFailureRecovers(unreadable: false)
    }

    func testFirstUnreadableReplyRecoversWithoutManualRefresh() async throws {
        try await assertFirstFailureRecovers(unreadable: true)
    }

    private actor RefreshThenFailure {
        private var reads = 0
        func read() throws -> DaemonData.SkillsSnapshot {
            reads += 1
            if reads == 1 { return .init(enabled: true, refreshing: true) }
            throw DaemonDataError.unreachable
        }
    }

    func testTransportFailureBacksOffDespiteStaleRefreshingSnapshot() async {
        let replies = RefreshThenFailure()
        let model = InsightsSkillsModel(reader: { _ in try await replies.read() })
        await model.load()
        XCTAssertEqual(model.observationDelay(failureAttempt: 4), .seconds(2))
        await model.load()
        XCTAssertTrue(model.shown?.refreshing == true)
        XCTAssertNotNil(model.failureKey)
        XCTAssertEqual(model.observationDelay(failureAttempt: 1), .seconds(4))
        XCTAssertEqual(model.observationDelay(failureAttempt: 4), .seconds(30))
    }

    func testFailureBackoffIsBounded() {
        XCTAssertEqual((0...5).map { InsightsSkillsModel.retryDelay(attempt: $0) },
                       [.seconds(2), .seconds(4), .seconds(8), .seconds(16), .seconds(30), .seconds(30)])
        XCTAssertEqual(InsightsSkillsModel.retryDelay(attempt: .max), .seconds(30))
    }

    func testCancellationStopsAnUnsuccessfulObserver() async throws {
        let model = InsightsSkillsModel(reader: { _ in throw DaemonDataError.unreachable })
        let observer = Task { await model.observe() }
        for _ in 0..<100 where model.failureKey == nil {
            try await Task.sleep(for: .milliseconds(1))
        }
        XCTAssertNotNil(model.failureKey)
        observer.cancel()
        await observer.value
        XCTAssertFalse(model.busy)
        XCTAssertNil(model.shown)
    }

    private func assertFirstFailureRecovers(unreadable: Bool) async throws {
        let replies = FirstFailure(unreadable: unreadable)
        let model = InsightsSkillsModel(reader: { try await replies.read($0) }, idlePollInterval: .seconds(60))
        let observer = Task { await model.observe() }
        defer { observer.cancel() }
        let clock = ContinuousClock()
        let deadline = clock.now.advanced(by: .seconds(6))
        while model.shown == nil, clock.now < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        // Success returns to the normal minute-long poll, not the error loop.
        try await Task.sleep(for: .milliseconds(50))
        observer.cancel()
        await observer.value
        XCTAssertEqual(model.shown?.followedPublishers, ["recovered"])
        XCTAssertNil(model.failureKey)
        let requests = await replies.requests
        XCTAssertEqual(requests, [false, false], "Recovery must not force publisher refreshes")
        let count = requests.count
        try await Task.sleep(for: .milliseconds(30))
        let afterCancellation = await replies.requests
        XCTAssertEqual(afterCancellation.count, count)
    }
}
