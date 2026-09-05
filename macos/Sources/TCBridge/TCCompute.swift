import CTraceCommons
import Foundation

/// App-owned compute controller. It requires neither a trace daemon nor enrollment.
/// Calls perform synchronous settings I/O: invoke them on a background queue.
///
/// Pointer calls pin the handle and release the ownership lock before Rust I/O.
/// Resource pressure remains independent of settings writes and shutdown.
/// Production construction cannot launch; close is not evidence of a drain.
public final class TCCompute: @unchecked Sendable {
    private let lock = NSLock()
    private var handle: OpaquePointer?
    private var activeCalls = 0

    public enum Failure: Error, Equatable, Sendable {
        case refused(String)
        case closed
        case invalidInput
    }

    public enum Command: Sendable {
        case enable(ramAllowanceGiB: UInt64)
        case resume
        case pause
        case disable

        fileprivate func json() throws -> String {
            let payload: [String: Any]
            switch self {
            case .enable(let allowance):
                payload = ["command": "enable", "ram_allowance_gib": allowance]
            case .resume: payload = ["command": "resume"]
            case .pause: payload = ["command": "pause"]
            case .disable: payload = ["command": "disable"]
            }
            let data = try JSONSerialization.data(withJSONObject: payload)
            guard let value = String(data: data, encoding: .utf8) else {
                throw Failure.invalidInput
            }
            return value
        }
    }

    public init(configDirectory: String) throws {
        // C strings truncate embedded NULs. Reject them before directory lookup.
        guard !configDirectory.utf8.contains(0) else { throw Failure.invalidInput }
        var error: UnsafeMutablePointer<CChar>?
        let opened = configDirectory.withCString { tc_compute_open($0, &error) }
        guard let opened else { throw Failure.refused(Self.take(error) ?? "panic") }
        if let error { tc_string_free(error) }
        handle = opened
    }

    deinit { close() }

    #if DEBUG
    /// Test-only native bridge seam; the Rust constructor also enforces its
    /// debug/Unix gate and strict loopback/hash configuration.
    init(localConfigDirectory: String, localConfigurationJSON: String) throws {
        guard !localConfigDirectory.utf8.contains(0),
              !localConfigurationJSON.utf8.contains(0), localConfigurationJSON.utf8.count <= 4096 else {
            throw Failure.invalidInput
        }
        var error: UnsafeMutablePointer<CChar>?
        let opened = localConfigDirectory.withCString { directory in
            localConfigurationJSON.withCString { tc_compute_open_local(directory, $0, &error) }
        }
        guard let opened else { throw Failure.refused(Self.take(error) ?? "panic") }
        if let error { tc_string_free(error) }
        handle = opened
    }
    #endif

    /// Ticket is issued before native reads, not at submission time.
    public func resourceBeginJSON() -> String? {
        try? withHandle { Self.take(tc_compute_resource_begin_json($0)) }
    }

    @discardableResult
    public func resourceSampleJSON(ticket: String, reading: TCComputeResourceReading) throws -> String {
        let token = try JSONSerialization.jsonObject(with: Data(ticket.utf8))
        let fields = try JSONSerialization.jsonObject(with: JSONEncoder().encode(reading))
        return try resourceEvent(["event": "sample", "ticket": token, "reading": fields])
    }

    @discardableResult
    public func resourceSleepJSON() throws -> String { try resourceEvent(["event": "sleep"]) }
    @discardableResult
    public func resourceWakeJSON() throws -> String { try resourceEvent(["event": "wake"]) }

    private func resourceEvent(_ event: [String: Any]) throws -> String {
        let bytes = try JSONSerialization.data(withJSONObject: event)
        guard bytes.count <= 4096, let json = String(data: bytes, encoding: .utf8) else { throw Failure.invalidInput }
        return try withHandle { handle in
            try json.withCString { try Self.result(tc_compute_resource_event_json(handle, $0)) }
        }
    }

    /// Handle-free fixed vocabulary remains available when settings cannot open.
    public static func copyJSON() -> String? { take(tc_compute_copy_json()) }

    /// Bounded controller stop. The caller must inspect worker_stopped before
    /// freeing this handle; drain_outcome separately describes acknowledgement.
    public func shutdownJSON(timeoutMilliseconds: UInt64) throws -> String {
        try withHandle { try Self.result(tc_compute_shutdown($0, timeoutMilliseconds)) }
    }

    /// Return Rust's snapshot unchanged, including shared wording and capability
    /// gates. Neither a successful call nor an open handle implies availability.
    public func statusJSON() throws -> String {
        try withHandle { try Self.result(tc_compute_status_json($0)) }
    }

    /// Returns the observed post-command snapshot; callers must not publish an
    /// optimistic enabled/running state while this command is in progress.
    public func commandJSON(_ command: Command) throws -> String {
        let json = try command.json()
        return try withHandle { handle in
            #if DEBUG
            commandWillExecuteForTesting?()
            #endif
            let result = json.withCString { tc_compute_command_json(handle, $0) }
            return try Self.result(result)
        }
    }

    /// Pins lifetime without holding the ownership lock during Rust I/O.
    /// Resource ingress can proceed while commands write settings or stop.
    private func withHandle<T>(_ operation: (OpaquePointer) throws -> T) throws -> T {
        lock.lock()
        guard let handle else { lock.unlock(); throw Failure.closed }
        activeCalls += 1
        lock.unlock()
        defer {
            lock.lock()
            activeCalls -= 1
            lock.unlock()
        }
        return try operation(handle)
    }

    #if DEBUG
    var commandWillExecuteForTesting: (@Sendable () -> Void)?
    #endif

    /// Idempotent. Retains the handle when the controller cannot prove all work
    /// stopped, so callers can retry shutdown. Never races another pointer call.
    @discardableResult
    public func close() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard let owned = handle else { return true }
        guard activeCalls == 0 else { return false }
        guard let raw = tc_compute_status_json(owned),
              let json = Self.take(raw),
              let evidence = try? JSONDecoder().decode(CloseEvidence.self, from: Data(json.utf8)),
              evidence.workerStopped && !evidence.commandPending else { return false }
        handle = nil
        tc_compute_free(owned)
        return true
    }

    private struct CloseEvidence: Decodable {
        let workerStopped: Bool
        let commandPending: Bool
        enum CodingKeys: String, CodingKey {
            case workerStopped = "worker_stopped"
            case commandPending = "command_pending"
        }
    }

    private static func result(
        _ pointer: UnsafeMutablePointer<CChar>?
    ) throws -> String {
        guard let value = take(pointer) else {
            // Borrowed thread-local storage: copy immediately on this thread,
            // and never pass it to tc_string_free.
            let label = tc_last_error().map { String(cString: $0) } ?? "panic"
            throw Failure.refused(label)
        }
        return value
    }

    private static func take(_ pointer: UnsafeMutablePointer<CChar>?) -> String? {
        guard let pointer else { return nil }
        defer { tc_string_free(pointer) }
        return String(cString: pointer)
    }
}
