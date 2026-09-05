import XCTest
@testable import TraceCommonsApp

final class QuitCoordinatorTests: XCTestCase {
    @MainActor
    func testCancelledQuitDoesNotCallStop() async {
        let coordinator = QuitCoordinator()
        var calls = 0
        XCTAssertEqual(coordinator.request(confirmed: false, deadlineSeconds: 60, stop: {
            calls += 1
            return true
        }, reply: { _ in XCTFail("cancelled request must not reply later") }), .cancel)
        await Task.yield()
        XCTAssertEqual(calls, 0)
        XCTAssertFalse(coordinator.isStopping)
    }

    @MainActor
    func testRepeatedQuitSharesOneStopAndRepliesOnce() async {
        let coordinator = QuitCoordinator()
        let started = expectation(description: "stop started")
        let replied = expectation(description: "reply")
        var finish: CheckedContinuation<Bool, Never>?
        var calls = 0
        XCTAssertEqual(coordinator.request(confirmed: true, deadlineSeconds: 60, stop: {
            calls += 1
            return await withCheckedContinuation { continuation in
                finish = continuation
                started.fulfill()
            }
        }, reply: { value in
            XCTAssertTrue(value)
            replied.fulfill()
        }), .later)
        XCTAssertEqual(coordinator.request(confirmed: true, deadlineSeconds: 60, stop: {
            XCTFail("duplicate stop")
            return false
        }, reply: { _ in XCTFail("duplicate reply") }), .later)
        await fulfillment(of: [started], timeout: 2)
        XCTAssertEqual(calls, 1)
        finish?.resume(returning: true)
        await fulfillment(of: [replied], timeout: 2)
        XCTAssertFalse(coordinator.isStopping)
    }

    @MainActor
    func testDeadlineKeepsAppAliveAndIgnoresLateSuccess() async {
        let coordinator = QuitCoordinator()
        let started = expectation(description: "stop started")
        let finished = expectation(description: "late stop finished")
        var finish: CheckedContinuation<Bool, Never>?
        var replies: [Bool] = []
        _ = coordinator.request(confirmed: true, deadlineSeconds: 60, stop: {
            let value = await withCheckedContinuation { continuation in
                finish = continuation
                started.fulfill()
            }
            finished.fulfill()
            return value
        }, reply: { replies.append($0) })
        await fulfillment(of: [started], timeout: 2)
        coordinator.deadlineExpired()
        XCTAssertEqual(replies, [false])
        XCTAssertTrue(coordinator.isStopping)
        XCTAssertEqual(coordinator.request(confirmed: true, deadlineSeconds: 60, stop: {
            XCTFail("must retain original stop")
            return true
        }, reply: { _ in XCTFail("must not install second reply") }), .cancel)
        finish?.resume(returning: true)
        await fulfillment(of: [finished], timeout: 2)
        await Task.yield()
        XCTAssertEqual(replies, [false])
        XCTAssertFalse(coordinator.isStopping)
    }

    @MainActor
    func testUnconfirmedStopRefusesTermination() async {
        let coordinator = QuitCoordinator()
        let replied = expectation(description: "reply")
        _ = coordinator.request(confirmed: true, deadlineSeconds: 60, stop: { false }, reply: {
            XCTAssertFalse($0)
            replied.fulfill()
        })
        await fulfillment(of: [replied], timeout: 2)
    }
}
