// INTEGRATION: AppModel owns one DaemonStartup and calls start from its main-
// actor startDaemon entry point. Publish running/needsRoots/refused only in the
// completion. Capture AppModel weakly and return false if it cannot adopt the
// result; return true after retaining a successful daemon or handling an error.
// Call cancel() before AppModel shutdown clears its client. Already-adopted
// daemons keep the existing subscription-aware shutdown path. Constructors and
// shutdown closures are injectable for barrier tests; both run off the UI.

import Foundation
import TCBridge

/// Owns a pending daemon until the main actor accepts it. A cancelled start
/// never publishes its result and disposes of any late daemon.
@MainActor
final class DaemonStartup {
    typealias Construct = @Sendable (String, String?) throws -> TCDaemon
    typealias Shutdown = @Sendable (TCDaemon) -> Void
    typealias Completion = @MainActor @Sendable (Result<TCDaemon, Error>) -> Bool

    private let worker = DispatchQueue(label: "ai.tracecommons.daemon-startup", qos: .userInitiated)
    private let construct: Construct
    private let shutdown: Shutdown
    private var pending: Pending?

    var isStarting: Bool { pending != nil }

    init(
        construct: @escaping Construct = { directory, settings in
            try TCDaemon(configDir: directory, settingsJSON: settings)
        },
        shutdown: @escaping Shutdown = { daemon in daemon.shutdown() }
    ) {
        self.construct = construct
        self.shutdown = shutdown
    }

    /// Refuses a duplicate while a start is pending. The synchronous C
    /// initializer cannot be interrupted: cancelling and then starting again
    /// stops the late daemon before the serial worker begins the next start.
    @discardableResult
    func start(
        configDirectory: String,
        settingsJSON: String?,
        completion: @escaping Completion
    ) -> Bool {
        guard pending == nil else { return false }
        let attempt = Pending(worker: worker, shutdown: shutdown)
        pending = attempt
        let construct = construct
        worker.async { [weak self] in
            guard attempt.mayConstruct else { return }
            attempt.finish(Result { try construct(configDirectory, settingsJSON) })
            Task { @MainActor [weak self] in
                guard let self, self.pending === attempt else {
                    attempt.cancel()
                    return
                }
                attempt.deliver(to: completion)
                if self.pending === attempt { self.pending = nil }
            }
        }
        return true
    }

    /// Returns without waiting for an OS prompt or daemon teardown.
    func cancel() {
        pending?.cancel()
        pending = nil
    }

    deinit {
        pending?.cancel()
    }

    /// TCDaemon has no Sendable conformance. This box transfers its ownership
    /// once rather than making that conformance on its behalf: result and
    /// cancelled are accessed under lock; construction and disposal happen
    /// on worker; only deliver takes a result onto the main actor. A rejected
    /// result is retained here before the main-actor reference is released,
    /// so its blocking destructor cannot run there with a live handle.
    private final class Pending: @unchecked Sendable {
        private let lock = NSLock()
        private let worker: DispatchQueue
        private let shutdown: Shutdown
        private var cancelled = false
        private var result: Result<TCDaemon, Error>?

        init(worker: DispatchQueue, shutdown: @escaping Shutdown) {
            self.worker = worker
            self.shutdown = shutdown
        }

        var mayConstruct: Bool {
            lock.lock()
            defer { lock.unlock() }
            return !cancelled
        }

        // Called only on worker, which may dispose immediately on cancellation.
        func finish(_ result: Result<TCDaemon, Error>) {
            lock.lock()
            self.result = result
            let discard = cancelled
            lock.unlock()
            if discard { dispose() }
        }

        func cancel() {
            lock.lock()
            cancelled = true
            lock.unlock()
            worker.async { self.dispose() }
        }

        @MainActor
        func deliver(to completion: Completion) {
            lock.lock()
            guard !cancelled, let result else {
                lock.unlock()
                return
            }
            self.result = nil
            lock.unlock()
            if !completion(result) {
                lock.lock()
                self.result = result
                cancelled = true
                lock.unlock()
                worker.async { self.dispose() }
            }
        }

        // Never called on the main actor. Taking under the lock makes repeated
        // cancellation and a finishing constructor dispose at most once.
        private func dispose() {
            lock.lock()
            let result = self.result
            self.result = nil
            lock.unlock()
            if case .success(let daemon) = result { shutdown(daemon) }
        }
    }
}
