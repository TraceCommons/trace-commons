import SwiftUI

extension OpenURLAction {
    /// Opens `url` and says whether the system took it (a browser opened).
    ///
    /// The system calls `OpenURLAction`'s completion on a LaunchServices
    /// queue, not the main thread. A completion closure written inside a
    /// main-actor view is main-actor isolated, and Swift's runtime check
    /// traps when it runs on that queue: the 0.24.4 Add funds crash
    /// (2026-10-27). The completion here is built by `resuming(_:)`, outside
    /// any actor, so it may run wherever the system calls it.
    @MainActor
    func opened(_ url: URL) async -> Bool {
        await withCheckedContinuation { continuation in
            self(url, completion: Self.resuming(continuation))
        }
    }

    /// A completion that resumes `continuation` from any thread.
    nonisolated static func resuming(_ continuation: CheckedContinuation<Bool, Never>) -> @Sendable (Bool) -> Void {
        { accepted in continuation.resume(returning: accepted) }
    }
}
