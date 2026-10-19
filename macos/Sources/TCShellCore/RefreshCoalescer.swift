import Foundation

/// Collapses a burst of refresh requests into as few runs as it can, never
/// fewer than the changes need.
///
/// The daemon sends `queue_changed` for every entry that moves, so one
/// Submit all over a hundred traces is hundreds of events in a second or
/// two. Reloading on each one rebuilt the whole tree on the main thread
/// hundreds of times in a row. With this, a request while a run is waiting
/// or in flight only marks that another run is wanted, and at most one more
/// runs once the current one finishes -- so the last change is always read,
/// and a burst costs two reads, not hundreds.
@MainActor
public final class RefreshCoalescer {
    private let delay: Duration
    private let work: @MainActor () async -> Void
    private var wanted = false
    private var task: Task<Void, Never>?
    /// Bumped by `cancel`, so a cancelled drain that finishes late never
    /// clears the task a newer request started.
    private var generation = 0
    /// How many times `work` has started. For tests and nothing else.
    public private(set) var runs = 0

    /// `delay` is how long the first request waits before running, so the
    /// rest of its burst lands inside one run rather than just after it.
    public init(delay: Duration, work: @escaping @MainActor () async -> Void) {
        self.delay = delay
        self.work = work
    }

    /// Asks for a run. Starts one if none is waiting or running; otherwise
    /// marks that one more is wanted after it.
    public func request() {
        wanted = true
        guard task == nil else { return }
        let mine = generation
        task = Task { [weak self] in
            await self?.drain(mine)
        }
    }

    /// Stops any run not yet started. A run already in `work` finishes, and
    /// nothing follows it.
    public func cancel() {
        generation += 1
        wanted = false
        task?.cancel()
        task = nil
    }

    /// Waits until nothing is waiting or running.
    public func settled() async {
        while let current = task {
            await current.value
            if task == current { return }
        }
    }

    private func drain(_ mine: Int) async {
        if delay > .zero {
            try? await Task.sleep(for: delay)
        }
        while wanted, mine == generation, !Task.isCancelled {
            wanted = false
            runs += 1
            await work()
        }
        if mine == generation {
            task = nil
        }
    }
}
