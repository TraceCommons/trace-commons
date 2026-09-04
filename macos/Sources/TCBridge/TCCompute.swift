import CTraceCommons
import Foundation

/// App-owned compute controller. It requires neither a trace daemon nor enrollment.
/// Calls perform synchronous settings I/O: invoke them on a background queue.
///
/// The lock serializes all pointer use with close, including concurrent callers
/// that retained this object before close began. The current controller cannot
/// launch a worker. Closing this handle is not evidence of a worker drain.
public final class TCCompute: @unchecked Sendable {
    private let lock = NSLock()
    private var handle: OpaquePointer?

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

    /// Return Rust's snapshot unchanged, including shared wording and capability
    /// gates. Neither a successful call nor an open handle implies availability.
    public func statusJSON() throws -> String {
        lock.lock()
        defer { lock.unlock() }
        guard let handle else { throw Failure.closed }
        return try Self.result(tc_compute_status_json(handle))
    }

    /// Returns the observed post-command snapshot; callers must not publish an
    /// optimistic enabled/running state while this command is in progress.
    public func commandJSON(_ command: Command) throws -> String {
        let json = try command.json()
        lock.lock()
        defer { lock.unlock() }
        guard let handle else { throw Failure.closed }
        let result = json.withCString { tc_compute_command_json(handle, $0) }
        return try Self.result(result)
    }

    /// Idempotent. Prevents new calls and waits for current pointer use before
    /// freeing; never frees a handle concurrently with a call through this object.
    public func close() {
        lock.lock()
        defer { lock.unlock() }
        guard let owned = handle else { return }
        handle = nil
        tc_compute_free(owned)
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
