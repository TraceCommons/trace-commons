import CTraceCommons
import Foundation

/// Thin, safe wrapper around the trace-commons-contributor-ffi C ABI.
///
/// This is the ONLY place in the demo where a raw `tc_handle*` / `char*`
/// pointer appears. Every function follows the ownership rule stated in
/// `trace_commons.h`: every `char*` this library returns is owned by the
/// caller and freed here with `tc_string_free` before the wrapper method
/// returns it as a plain Swift `String`.
final class TCDaemon {
    private var handle: OpaquePointer?

    enum TCError: Error, CustomStringConvertible {
        case startFailed(String)
        var description: String {
            switch self {
            case .startFailed(let msg): return "tc_daemon_start failed: \(msg)"
            }
        }
    }

    /// Starts the daemon against `configDir`. Never touches the real
    /// ~/.claude or ~/.codex trees: this repo's C ABI has no call to set
    /// claude_root/codex_root before start, so the caller is expected to
    /// have already pre-seeded `configDir/daemon-settings.json` (see
    /// `seedSettings` in main.swift) pointing those roots at empty temp
    /// directories before calling this initializer.
    init(configDir: String) throws {
        var errPtr: UnsafeMutablePointer<CChar>?
        let h: OpaquePointer? = configDir.withCString { cDir in
            withUnsafeMutablePointer(to: &errPtr) { errOut in
                tc_daemon_start(cDir, errOut)
            }
        }
        if h == nil {
            let message: String
            if let e = errPtr {
                message = String(cString: e)
                tc_string_free(e)
            } else {
                message = "unknown error"
            }
            throw TCError.startFailed(message)
        }
        self.handle = h
    }

    /// Calls `method` with `paramsJSON` (a JSON object literal, e.g. "{}")
    /// and returns the daemon's JSON response as a Swift String. Never
    /// throws: per the header, tc_call never returns NULL, it returns a
    /// JSON error frame on failure.
    func call(_ method: String, params paramsJSON: String = "{}") -> String {
        guard let handle else { return "{\"error\":{\"code\":\"unavailable\",\"message\":\"handle-freed\"}}" }
        let resultPtr: UnsafeMutablePointer<CChar>? = method.withCString { cMethod in
            paramsJSON.withCString { cParams in
                tc_call(handle, cMethod, cParams)
            }
        }
        guard let resultPtr else {
            // Header guarantees this never happens, but stay defensive.
            return "{\"error\":{\"code\":\"unavailable\",\"message\":\"null-response\"}}"
        }
        defer { tc_string_free(resultPtr) }
        return String(cString: resultPtr)
    }

    /// Stops the daemon loop. Safe to call before `close()`; idempotent.
    func stop() {
        guard let handle else { return }
        tc_daemon_stop(handle)
    }

    /// Frees the handle. Must be called from a plain thread, never from
    /// inside a tc_subscribe callback, and never concurrently with
    /// tc_daemon_stop. This demo is single-threaded and calls stop() then
    /// close() sequentially, so both rules hold trivially.
    func close() {
        guard let h = handle else { return }
        tc_handle_free(h)
        handle = nil
    }

    deinit {
        // Best-effort: a real host should call stop()/close() explicitly,
        // as main.swift does, rather than relying on deinit ordering.
        if handle != nil {
            tc_daemon_stop(handle)
            tc_handle_free(handle)
            handle = nil
        }
    }
}
