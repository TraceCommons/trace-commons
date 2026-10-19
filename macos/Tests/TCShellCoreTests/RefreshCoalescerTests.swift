import XCTest
@testable import TCShellCore

/// A burst of daemon events (a 100-trace Submit all sends hundreds of
/// `queue_changed`) must not become a burst of full reloads on the main
/// thread. These count runs, never time.
@MainActor
final class RefreshCoalescerTests: XCTestCase {
    /// A run the test holds open until it says so, so requests can arrive
    /// while one is in flight.
    @MainActor private final class Gate {
        var waiting: [CheckedContinuation<Void, Never>] = []
        var entered = 0
        func pass() async {
            entered += 1
            await withCheckedContinuation { waiting.append($0) }
        }
        func open() {
            let all = waiting
            waiting = []
            all.forEach { $0.resume() }
        }
    }

    private func untilEntered(_ gate: Gate, _ count: Int) async {
        while gate.entered < count { await Task.yield() }
    }

    func test_aBurstWhileARunIsInFlightBecomesOneMoreRun() async {
        let gate = Gate()
        let coalescer = RefreshCoalescer(delay: .zero) { await gate.pass() }
        coalescer.request()
        await untilEntered(gate, 1)
        for _ in 0..<200 { coalescer.request() }
        gate.open()
        await untilEntered(gate, 2)
        gate.open()
        await coalescer.settled()
        XCTAssertEqual(coalescer.runs, 2, "200 requests during one run are one follow-up, not 200")
    }

    func test_aBurstBeforeTheFirstRunIsOneRun() async {
        var runs = 0
        let coalescer = RefreshCoalescer(delay: .zero) { runs += 1 }
        for _ in 0..<50 { coalescer.request() }
        await coalescer.settled()
        XCTAssertEqual(runs, 1)
        XCTAssertEqual(coalescer.runs, 1)
    }

    func test_aRequestAfterSettlingRunsAgain() async {
        var runs = 0
        let coalescer = RefreshCoalescer(delay: .zero) { runs += 1 }
        coalescer.request()
        await coalescer.settled()
        coalescer.request()
        await coalescer.settled()
        XCTAssertEqual(runs, 2, "a later change is never swallowed")
    }

    func test_cancelDropsTheFollowUp() async {
        let gate = Gate()
        let coalescer = RefreshCoalescer(delay: .zero) { await gate.pass() }
        coalescer.request()
        await untilEntered(gate, 1)
        coalescer.request()
        coalescer.cancel()
        gate.open()
        await coalescer.settled()
        XCTAssertEqual(gate.entered, 1, "a cancelled coalescer starts nothing new")
    }

    func test_aRequestAfterCancelStartsFresh() async {
        var runs = 0
        let coalescer = RefreshCoalescer(delay: .zero) { runs += 1 }
        coalescer.request()
        coalescer.cancel()
        await coalescer.settled()
        coalescer.request()
        await coalescer.settled()
        XCTAssertEqual(runs, 1)
    }
}
