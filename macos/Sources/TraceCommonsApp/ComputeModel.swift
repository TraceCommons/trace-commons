import Foundation
import Observation
import TCBridge
import TCShellCore

/// Owns compute independently of the trace daemon and mounted windows. Controller
/// I/O and free run on a serial background queue; the handle-free copy call only
/// reads fixed shared vocabulary.
@Observable @MainActor
final class ComputeModel {
    private(set) var snapshot: ComputeSnapshot?
    private(set) var failureLabel: String?
    private(set) var busy = false
    private(set) var quitWasRefused = false
    let copy = ComputeCopy.decode(TCCompute.copyJSON() ?? "")
    private var started = false
    private var stopping = false
    private var closed = false
    @ObservationIgnored private var monitor: Task<Void, Never>?
    @ObservationIgnored private let service = ComputeService()

    var controlsBusy: Bool { busy || stopping || snapshot?.commandPending == true }

    func noteQuitRefused() { quitWasRefused = true }

    func start(configDirectory: String) async {
        guard !started, !closed else { return }
        started = true
        busy = true
        defer { busy = false }
        do { publish(try await service.open(configDirectory: configDirectory)) }
        catch { fail(error) }
    }

    func perform(_ command: TCCompute.Command) async {
        guard !closed, !controlsBusy, let snapshot else { return }
        switch command {
        case .enable: guard snapshot.available && snapshot.canEnable else { return }
        case .resume: guard snapshot.available && snapshot.canResume else { return }
        case .pause: guard snapshot.canPause else { return }
        case .disable: guard snapshot.consentGranted else { return }
        }
        busy = true
        defer { busy = false }
        do { publish(try await service.command(command)) }
        catch { fail(error) }
    }

    @discardableResult
    func close() async -> Bool {
        monitor?.cancel()
        monitor = nil
        if await service.close() {
            closed = true
            snapshot = nil
            return true
        }
        startMonitoring()
        return false
    }

    func startMonitoring() {
        guard started, snapshot != nil, !closed, monitor == nil, !stopping else { return }
        monitor = Task { [weak self] in
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(1)) }
                catch { return }
                guard let self else { return }
                await self.refresh()
            }
        }
    }

    private func refresh() async {
        guard started, !busy, !stopping else { return }
        do { publish(try await service.status()) }
        catch { fail(error) }
    }

    /// A failed or timed-out stop retains the service and its worker ownership.
    /// The app may exit after process stop even if graceful drain was unconfirmed;
    /// it must never present that result as an acknowledged handoff.
    func shutdown(timeoutMilliseconds: UInt64) async -> Bool {
        guard !stopping else { return false }
        quitWasRefused = false
        stopping = true
        monitor?.cancel()
        monitor = nil
        defer {
            stopping = false
            startMonitoring()
        }
        do {
            if let json = try await service.shutdown(timeoutMilliseconds: timeoutMilliseconds) {
                publish(json)
                guard snapshot?.workerStopped == true else { return false }
                if quitWasRefused {
                    // Shutdown seals its controller against queued/new starts.
                    // A deadline-refused Quit needs a new paused controller, but
                    // only after the old controller proves its worker stopped.
                    publish(try await service.reopenAfterStop())
                }
            }
            // Otherwise retain the sealed idle handle until actual termination.
            return true
        } catch {
            fail(error)
            return false
        }
    }

    private func publish(_ json: String) {
        guard let value = ComputeSnapshot.decode(json) else {
            snapshot = nil
            failureLabel = "compute-status-invalid"
            return
        }
        if snapshot != value { snapshot = value }
        failureLabel = nil
    }

    private func fail(_ error: Error) {
        snapshot = nil
        if case TCCompute.Failure.refused(let label) = error {
            failureLabel = label
        } else {
            failureLabel = "compute-unavailable"
        }
    }
}

/// Queue confinement keeps blocking FFI calls off Swift's cooperative executor.
/// The queue owns the handle; its lifetime is not tied to a selected tab.
private final class ComputeService: @unchecked Sendable {
    private let queue = DispatchQueue(label: "org.tracecommons.compute", qos: .utility)
    private var handle: TCCompute?
    private var configDirectory: String?
    private var closed = false

    func open(configDirectory: String) async throws -> String {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                do {
                    guard !self.closed else { throw TCCompute.Failure.closed }
                    if self.handle == nil {
                        self.handle = try TCCompute(configDirectory: configDirectory)
                        self.configDirectory = configDirectory
                    }
                    continuation.resume(returning: try self.handle!.statusJSON())
                } catch { continuation.resume(throwing: error) }
            }
        }
    }

    func command(_ command: TCCompute.Command) async throws -> String {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                do {
                    guard !self.closed, let handle = self.handle else { throw TCCompute.Failure.closed }
                    continuation.resume(returning: try handle.commandJSON(command))
                } catch { continuation.resume(throwing: error) }
            }
        }
    }

    func status() async throws -> String {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                do {
                    guard !self.closed, let handle = self.handle else { throw TCCompute.Failure.closed }
                    continuation.resume(returning: try handle.statusJSON())
                } catch { continuation.resume(throwing: error) }
            }
        }
    }

    func shutdown(timeoutMilliseconds: UInt64) async throws -> String? {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                do {
                    guard let handle = self.handle else {
                        continuation.resume(returning: nil)
                        return
                    }
                    continuation.resume(returning: try handle.shutdownJSON(timeoutMilliseconds: timeoutMilliseconds))
                } catch { continuation.resume(throwing: error) }
            }
        }
    }

    func close() async -> Bool {
        await withCheckedContinuation { continuation in
            queue.async {
                if let handle = self.handle {
                    guard let json = try? handle.statusJSON(),
                          let status = ComputeSnapshot.decode(json), status.workerStopped == true,
                          status.commandPending == false, handle.close() else {
                        continuation.resume(returning: false)
                        return
                    }
                }
                self.closed = true
                self.handle = nil
                continuation.resume(returning: true)
            }
        }
    }

    func reopenAfterStop() async throws -> String {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                do {
                    guard !self.closed, let handle = self.handle, let directory = self.configDirectory,
                          ComputeSnapshot.decode(try handle.statusJSON())?.workerStopped == true else {
                        throw TCCompute.Failure.refused("compute-stop-unconfirmed")
                    }
                    guard handle.close() else { throw TCCompute.Failure.refused("compute-stop-unconfirmed") }
                    self.handle = nil
                    let reopened = try TCCompute(configDirectory: directory)
                    self.handle = reopened
                    continuation.resume(returning: try reopened.statusJSON())
                } catch { continuation.resume(throwing: error) }
            }
        }
    }
}
