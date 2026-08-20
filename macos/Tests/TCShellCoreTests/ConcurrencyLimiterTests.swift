import TCShellCore
import XCTest

/// `ConcurrencyLimiter` is the seam that stops the queue from firing every
/// row's `previewSummary` call at once (the 2026-08-20 incident: 500 queued
/// entries, 500 simultaneous JSON parses, load average 649). These tests
/// assert the two things `AppModel` actually depends on: the ceiling holds
/// under real concurrent load, and it never blocks forever.
final class ConcurrencyLimiterTests: XCTestCase {
    /// Fires `taskCount` tasks at a limiter with `limit` slots and has each
    /// one record the number of callers concurrently inside its section
    /// (via a plain actor-guarded counter, not the limiter's own bookkeeping
    /// -- that would make the assertion tautological). Asserts the observed
    /// peak never exceeds `limit`, and that every task actually ran.
    func testNeverExceedsTheLimitUnderConcurrentLoad() async {
        let limit = 4
        let taskCount = 50
        let limiter = ConcurrencyLimiter(limit: limit)
        let tracker = ConcurrencyTracker()

        await withTaskGroup(of: Void.self) { group in
            for _ in 0..<taskCount {
                group.addTask {
                    await limiter.acquire()
                    await tracker.enter()
                    // Yield so overlapping callers actually have a chance to
                    // run concurrently instead of the group serializing by
                    // accident on a single-core scheduler decision.
                    await Task.yield()
                    await tracker.exit()
                    await limiter.release()
                }
            }
        }

        let peak = await tracker.peakConcurrent
        let completed = await tracker.completedCount
        XCTAssertLessThanOrEqual(peak, limit, "observed more callers inside the section than the limit allows")
        XCTAssertEqual(completed, taskCount, "every task should eventually acquire and complete")
    }

    /// A single slot serializes every caller: with `limit: 1`, two `Task`s
    /// racing to acquire must never both report success before either
    /// releases -- proven by widening the critical section with a delay and
    /// checking the tracker's peak is exactly 1, not merely `<= 1` (which a
    /// no-op limiter would also satisfy).
    func testASingleSlotFullySerializes() async {
        let limiter = ConcurrencyLimiter(limit: 1)
        let tracker = ConcurrencyTracker()

        await withTaskGroup(of: Void.self) { group in
            for _ in 0..<10 {
                group.addTask {
                    await limiter.acquire()
                    await tracker.enter()
                    try? await Task.sleep(nanoseconds: 1_000_000)
                    await tracker.exit()
                    await limiter.release()
                }
            }
        }

        let peak = await tracker.peakConcurrent
        XCTAssertEqual(peak, 1)
    }

    /// A duplicate request for the same id must produce exactly one call --
    /// this is `AppModel`'s in-flight dedupe contract, modeled here with a
    /// small stand-in so it is testable without the FFI-linked `AppModel`
    /// itself. `requestOnce` mirrors the guard `AppModel.requestSummary(for:)`
    /// uses: skip if already in flight, mark in flight, call, clear.
    func testDuplicateRequestsForOneIDResultInOneCall() async {
        let gate = DedupeGate()
        let callCount = CallCounter()

        await withTaskGroup(of: Void.self) { group in
            for _ in 0..<20 {
                group.addTask {
                    await gate.requestOnce(id: "entry-1") {
                        await callCount.increment()
                    }
                }
            }
        }

        let calls = await callCount.value
        XCTAssertEqual(calls, 1, "20 concurrent requests for the same id must dedupe to a single call")
    }
}

/// Test-only counter of concurrent entrants, isolated as an actor so
/// increments/decrements from many tasks are race-free without touching
/// `ConcurrencyLimiter`'s own state.
private actor ConcurrencyTracker {
    private(set) var peakConcurrent = 0
    private(set) var completedCount = 0
    private var current = 0

    func enter() {
        current += 1
        peakConcurrent = max(peakConcurrent, current)
    }

    func exit() {
        current -= 1
        completedCount += 1
    }
}

private actor CallCounter {
    private(set) var value = 0
    func increment() { value += 1 }
}

/// Minimal stand-in for the "skip if already in flight" guard
/// `AppModel.requestSummary(for:)` applies around `previewSummary`. Kept
/// here, not in the app target, because `AppModel` links the FFI dylib and
/// cannot be unit tested; this reproduces the dedupe logic exactly enough to
/// prove the guard shape is race-free under real concurrency.
private actor DedupeGate {
    private var inFlight: Set<String> = []

    func requestOnce(id: String, _ work: () async -> Void) async {
        guard !inFlight.contains(id) else { return }
        inFlight.insert(id)
        await work()
        inFlight.remove(id)
    }
}
