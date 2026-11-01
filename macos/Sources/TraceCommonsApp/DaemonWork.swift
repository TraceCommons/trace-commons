import Foundation

/// Where this app runs a blocking daemon call: a concurrent GCD queue of its
/// own, never the main thread and never Swift's cooperative pool.
///
/// `TCDaemon.call` blocks its thread until the daemon answers, and some
/// answers wait on the network: an account-session renewal, a NEAR AI
/// ceremony, a payout read. `Task.detached` keeps such a call off the main
/// actor, but parks one of the cooperative pool's few threads (about one per
/// core) for the whole wait, and a handful of them waiting together stalls
/// every other task in the app, the UI's included. GCD grows its pool for a
/// blocked worker; the cooperative pool does not. `LiveDaemonClient.perform`
/// does the same for the data client.
enum DaemonWork {
    private static let queue = DispatchQueue(
        label: "org.tracecommons.app.daemon-calls",
        qos: .userInitiated,
        attributes: .concurrent
    )

    /// Runs `work` on the daemon-call queue and resumes with its answer.
    static func run<T: Sendable>(_ work: @escaping @Sendable () -> T) async -> T {
        await withCheckedContinuation { continuation in
            queue.async { continuation.resume(returning: work()) }
        }
    }
}
