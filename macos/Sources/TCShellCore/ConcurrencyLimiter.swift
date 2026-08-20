import Foundation

/// Bounds how many callers may hold a slot at once.
///
/// Built for `AppModel.loadMissingSummaries` / `requestSummary(for:)`: a
/// contributor with 500 queued sessions used to have every entry spawn its
/// own `Task.detached` calling `previewSummary` the instant the queue
/// snapshot arrived. A preview is not cheap -- it reads the session file,
/// parses its JSON, and runs the redaction pipeline -- so 500 of them at
/// once meant 500 JSON parses contending for the CPU simultaneously, which
/// is what actually pinned the machine (see the 2026-08-20 incident: 1.7
/// sustained cores, 1.34 GB resident, load average 649, all inside serde
/// JSON parsing in the FFI dylib).
///
/// This does not decide WHO gets a slot or WHEN to ask -- that's the
/// caller's job (in `AppModel`, "a row asked for its summary because it
/// scrolled into view"). It only ever guarantees that no more than `limit`
/// callers are inside their protected section at the same time, first come
/// first served among those waiting.
///
/// A plain `actor` rather than `DispatchSemaphore`: callers are Swift
/// `Task`s, not threads, and blocking a thread to wait for a Task-based
/// producer risks starving the cooperative pool. `withCheckedContinuation`
/// suspends the calling Task instead of a thread.
public actor ConcurrencyLimiter {
    private let limit: Int
    private var available: Int
    private var waiters: [CheckedContinuation<Void, Never>] = []

    public init(limit: Int) {
        precondition(limit > 0, "a limiter with no slots would deadlock every caller")
        self.limit = limit
        self.available = limit
    }

    /// Suspends until a slot is free, then takes it. Always pair with
    /// `release()` -- there is no scoped `withSlot` here because the caller
    /// needs to hop to `@MainActor` between acquiring and releasing (to
    /// publish the result) and a `defer` inside an `actor`-isolated `async`
    /// function cannot do that hop for you.
    public func acquire() async {
        if available > 0 {
            available -= 1
            return
        }
        await withCheckedContinuation { waiters.append($0) }
    }

    /// Frees the caller's slot. If anyone is waiting, hands it directly to
    /// the longest-waiting one instead of incrementing `available`, so a
    /// slot is never briefly "free" for a third party to steal between one
    /// caller's release and the next waiter's resume.
    public func release() {
        guard waiters.isEmpty else {
            let next = waiters.removeFirst()
            next.resume()
            return
        }
        available += 1
    }

    /// Test-only: how many slots are currently held. Never read this from
    /// production code to make a decision -- it exists so a test can assert
    /// "never more than K in flight" from outside the actor's isolation.
    public var _testOnly_inUse: Int {
        limit - available
    }
}
