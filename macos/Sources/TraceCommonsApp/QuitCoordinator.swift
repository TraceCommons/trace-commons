import Foundation

/// One outstanding stop operation across every macOS Quit entry point.
/// A UI deadline cancels termination, never worker ownership. A late successful
/// stop cannot unexpectedly terminate an app whose Quit request already timed out.
@MainActor
final class QuitCoordinator {
    enum Decision: Equatable { case cancel, later }
    private enum Phase { case idle, waiting, timedOut }
    private var phase: Phase = .idle
    private var reply: ((Bool) -> Void)?
    private var deadline: DispatchWorkItem?
    private var stopTask: Task<Void, Never>?

    var isStopping: Bool { phase != .idle }

    func request(
        confirmed: Bool,
        deadlineSeconds: TimeInterval,
        stop: @escaping @MainActor () async -> Bool,
        reply: @escaping (Bool) -> Void
    ) -> Decision {
        switch phase {
        case .waiting: return .later
        case .timedOut: return .cancel
        case .idle: break
        }
        guard confirmed else { return .cancel }
        phase = .waiting
        self.reply = reply
        let deadline = DispatchWorkItem { [weak self] in
            MainActor.assumeIsolated { self?.deadlineExpired() }
        }
        self.deadline = deadline
        DispatchQueue.main.asyncAfter(deadline: .now() + deadlineSeconds, execute: deadline)
        stopTask = Task { [weak self] in
            let stopped = await stop()
            self?.finished(stopped: stopped)
        }
        return .later
    }

    /// Internal seam for deterministic timeout tests; production reaches it only
    /// through the scheduled deadline above.
    func deadlineExpired() {
        guard phase == .waiting else { return }
        phase = .timedOut
        let reply = self.reply
        self.reply = nil
        reply?(false)
    }

    private func finished(stopped: Bool) {
        deadline?.cancel()
        deadline = nil
        let reply = self.reply
        self.reply = nil
        phase = .idle
        stopTask = nil
        reply?(stopped)
    }
}
