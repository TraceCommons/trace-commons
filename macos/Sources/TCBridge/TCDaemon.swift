import CTraceCommons
import Foundation

/// Thin, safe wrapper around the trace-commons-contributor-ffi C ABI.
///
/// This file, together with `TCPreview.swift` and `TCSubscription.swift`, is
/// the ONLY place in this package where a raw `tc_handle*` / `tc_preview*` /
/// `char*` pointer appears. Every function follows the ownership rule stated
/// in `trace_commons.h`: every `char*` this library returns is owned by the
/// caller and freed here with `tc_string_free` before the wrapper method
/// returns it as a plain Swift `String`.
public final class TCDaemon {
    /// Not `private` so `TCPreview`/`TCSubscription` in this same module can
    /// pass it back to the C ABI. Nothing outside this module ever sees it.
    internal var handle: OpaquePointer?

    public enum TCError: Error, CustomStringConvertible {
        case startFailed(String)
        case previewFailed(String)
        case daemonGone

        public var description: String {
            switch self {
            case .startFailed(let msg): return "tc_daemon_start failed: \(msg)"
            case .previewFailed(let msg): return "tc_preview_open failed: \(msg)"
            case .daemonGone: return "daemon handle already freed"
            }
        }
    }

    /// Starts the daemon against `configDir`. Never touches the real
    /// ~/.claude or ~/.codex trees: this repo's C ABI has no call to set
    /// claude_root/codex_root before start, so the caller is expected to
    /// have already pre-seeded `configDir/daemon-settings.json` pointing
    /// those roots somewhere deliberate before calling this initializer.
    /// `TraceCommonsApp/DaemonHost.swift` does exactly that.
    public init(configDir: String) throws {
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
    public func call(_ method: String, params paramsJSON: String = "{}") -> String {
        guard let handle else {
            return "{\"error\":{\"code\":\"unavailable\",\"message\":\"handle-freed\"}}"
        }
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

    // MARK: - Preview

    /// Opens the in-process preview for `entryID`: the redacted body plus a
    /// summary. This is the C ABI's deliberate content exemption -- the
    /// socket's `preview` method returns the summary only.
    ///
    /// Blocks the calling thread for the duration of the redaction pass, so
    /// callers run it off the main thread.
    public func openPreview(entryID: String) throws -> TCPreview {
        guard let handle else { throw TCError.daemonGone }
        var errPtr: UnsafeMutablePointer<CChar>?
        let p: OpaquePointer? = entryID.withCString { cEntry in
            withUnsafeMutablePointer(to: &errPtr) { errOut in
                tc_preview_open(handle, cEntry, errOut)
            }
        }
        guard let p else {
            let message: String
            if let e = errPtr {
                message = String(cString: e)
                tc_string_free(e)
            } else {
                message = "unknown error"
            }
            throw TCError.previewFailed(message)
        }
        return TCPreview(pointer: p)
    }

    // MARK: - Subscription

    /// Registers `handler`, invoked with each JSON event frame the daemon
    /// publishes, on a Rust background thread. `handler` must therefore do
    /// its own hop to the main actor before touching observable state; see
    /// `DaemonHost` for the one call site that does.
    ///
    /// Returns nil if the ABI refused (NULL handle or a stopped daemon --
    /// token 0 is never valid).
    public func subscribe(_ handler: @escaping (String) -> Void) -> TCSubscription? {
        guard let handle else { return nil }
        // ctx must stay alive until tc_unsubscribe RETURNS, per the header's
        // SUBSCRIPTION LIFETIME rule -- retained here, released only by a
        // tc_unsubscribe that we confirmed was not refused.
        let box = TCCallbackBox(handler)
        let ctx = Unmanaged.passRetained(box).toOpaque()
        let token = tc_subscribe(
            handle,
            { eventJSON, ctx in
                guard let eventJSON, let ctx else { return }
                let box = Unmanaged<TCCallbackBox>.fromOpaque(ctx).takeUnretainedValue()
                // The event_json pointer is borrowed for this call only, so
                // it is copied into a Swift String before anything else.
                box.handler(String(cString: eventJSON))
            },
            ctx
        )
        if token == 0 {
            Unmanaged<TCCallbackBox>.fromOpaque(ctx).release()
            return nil
        }
        return TCSubscription(token: token, ctx: ctx)
    }

    /// Cancels `subscription` and releases its ctx.
    ///
    /// `tc_unsubscribe` returns void and, per the header, refuses SILENTLY
    /// when called from a thread that is inside any tokio runtime context --
    /// including a host's own unrelated runtime. A host that assumed success
    /// would free ctx while callbacks can still fire. So the prior
    /// `tc_last_error` value is noted first, and a NEW error after the call
    /// means the barrier did not hold: ctx is kept alive, the token stays
    /// valid, and this returns false so the caller can retry from a plain
    /// thread.
    ///
    /// The pre/post comparison is the best a thread-local, last-error-only
    /// surface allows -- a genuinely identical error string recorded by the
    /// refusal would read as stale. Call this from a plain thread (the main
    /// thread qualifies; Swift's cooperative pool is not a tokio runtime).
    @discardableResult
    public func unsubscribe(_ subscription: TCSubscription) -> Bool {
        guard let handle else { return false }
        let before = tc_last_error().map { String(cString: $0) }
        tc_unsubscribe(handle, subscription.token)
        let after = tc_last_error().map { String(cString: $0) }
        if let after, after != before {
            return false
        }
        Unmanaged<TCCallbackBox>.fromOpaque(subscription.ctx).release()
        return true
    }

    // MARK: - Teardown

    /// Stops the daemon loop. Idempotent. Does NOT end subscriptions and is
    /// not a synchronization point for them -- unsubscribe first.
    public func stop() {
        guard let handle else { return }
        tc_daemon_stop(handle)
    }

    /// Frees the handle. Must be called from a plain thread, never from
    /// inside a tc_subscribe callback, and never concurrently with
    /// tc_daemon_stop. The app calls unsubscribe(), stop() and close()
    /// sequentially on the main thread, so all three rules hold.
    public func close() {
        guard let h = handle else { return }
        tc_handle_free(h)
        handle = nil
    }

    deinit {
        // Best-effort: a real host should call stop()/close() explicitly,
        // as DaemonHost.shutdown() does, rather than relying on deinit.
        if handle != nil {
            tc_daemon_stop(handle)
            tc_handle_free(handle)
            handle = nil
        }
    }
}

/// Heap box carrying a Swift closure across the C `void* ctx` boundary. A C
/// function pointer cannot capture, so the closure has to travel this way.
final class TCCallbackBox {
    let handler: (String) -> Void
    init(_ handler: @escaping (String) -> Void) {
        self.handler = handler
    }
}

/// A live subscription token plus the retained ctx it was registered with.
/// Opaque on purpose: only `TCDaemon.unsubscribe` may release the ctx, and
/// only after the ABI confirms the barrier held.
public final class TCSubscription {
    internal let token: UInt64
    internal let ctx: UnsafeMutableRawPointer

    internal init(token: UInt64, ctx: UnsafeMutableRawPointer) {
        self.token = token
        self.ctx = ctx
    }
}
