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
///
/// It is also the only place that decides WHEN the handle may be freed. The
/// host calls methods here from many threads at once and then quits; the
/// gate below is what makes those two facts compatible. Safe to use from any
/// thread.
public final class TCDaemon {
    // MARK: - Handle lifetime
    //
    // WHY A COUNTED GATE AND NOT AN ACTOR, A SERIAL QUEUE, OR A PLAIN LOCK.
    //
    // What `trace_commons.h` actually requires is narrower than "one call at
    // a time". It says tc_call / tc_preview_open / tc_subscribe MAY run
    // concurrently on other threads, and that tc_handle_free "must not be
    // called concurrently with any other call still using the same pointer"
    // and must run on a plain thread outside any tokio runtime. So the real
    // requirement is a shared/exclusive one: many concurrent users of the
    // pointer, exactly one exclusive teardown, and no way for a new user to
    // start once teardown has begun.
    //
    // - An `actor` would serialize everything, including a preview redaction
    //   pass that blocks for as long as it takes to read and redact a
    //   session file. That call is documented as blocking; parking it on an
    //   actor would block every unrelated `status` behind it, and blocking
    //   work on Swift's cooperative pool starves that pool. It would also
    //   force every synchronous `DaemonClient` method to become `async`.
    // - A serial DispatchQueue has the same serialization cost plus a second
    //   thread-affinity question the header cares about (teardown must be on
    //   a plain thread), which a queue does not let us state directly.
    // - A plain mutex held across the C call would serialize just the same,
    //   and a mutex cannot express "wait until the last in-flight call has
    //   left" without a condition variable anyway.
    //
    // So: an NSCondition guarding an in-flight counter plus a `closing`
    // flag. `withHandle` admits any number of concurrent callers while the
    // handle is live and refuses every caller once teardown starts;
    // `shutdown` flips the flag, waits for the counter to reach zero, and
    // only then performs unsubscribe / stop / free, in that order, on the
    // calling thread. NSCondition is Foundation, so no new dependency.
    //
    // AND IF THE WAIT DOES NOT COMPLETE, WE LEAK. A handle that is still
    // reachable by an in-flight C call, or a subscription whose barrier the
    // ABI refused to confirm, is never passed to tc_handle_free. The process
    // is exiting: an unfreed handle costs nothing, and a use-after-free is a
    // crash or worse. `ShutdownOutcome.leaked` says which case it was.
    private let gate = NSCondition()
    private var handle: OpaquePointer?
    private var inFlight = 0
    private var closing = false

    /// What teardown managed to do. `.leaked` is a deliberate, safe outcome,
    /// not an error to recover from -- see the note above.
    public enum ShutdownOutcome: Equatable {
        /// Everything drained; the handle was freed.
        case freed
        /// Teardown could not prove the handle was idle, so it was NOT
        /// freed. The payload is a fixed label, safe to log.
        case leaked(String)
    }

    public enum TCError: Error, CustomStringConvertible {
        case startFailed(String)
        case previewFailed(String)
        case daemonGone
        /// The contributor has not said which session folders to watch, so
        /// the daemon refused to start and has scanned nothing.
        ///
        /// Its own case rather than a `startFailed` carrying a label,
        /// because the two lead somewhere different: this one routes to the
        /// screen that collects the folders, and every other start failure
        /// to a notice with nothing to do. Flattening them is what left the
        /// old shell with a refusal it could never clear.
        case rootsNotDeclared

        public var description: String {
            switch self {
            case .startFailed(let msg): return "tc_daemon_start failed: \(msg)"
            case .previewFailed(let msg): return "tc_preview_open failed: \(msg)"
            case .daemonGone: return "daemon handle already freed"
            case .rootsNotDeclared:
                return """
                    This app hasn't been told which session folders to watch, and it \
                    won't guess. Nothing is being watched.
                    """
            }
        }
    }

    /// The fixed label the C ABI reports for a roots refusal. Matched, not
    /// parsed: `trace_commons.h` documents it as a fixed, content-free
    /// string precisely so a host can branch on it.
    private static let rootsNotDeclaredLabel = "roots-not-declared"

    /// How many calls are inside the C ABI with the handle right now.
    /// Diagnostics only -- true the instant it is read and possibly not the
    /// instant after. `shutdown` does not consult it; it waits on the
    /// condition instead.
    public var inFlightCalls: Int {
        gate.lock()
        defer { gate.unlock() }
        return inFlight
    }

    /// Runs `body` with the raw handle, or returns nil if the handle is gone
    /// or teardown has begun. Every use of the pointer in this file goes
    /// through here -- that is the whole invariant: while `body` runs, the
    /// in-flight count is nonzero, and `shutdown` cannot reach
    /// tc_handle_free while the count is nonzero.
    ///
    /// Callable from any thread. It does NOT serialize `body` against other
    /// callers, because the header explicitly permits that concurrency.
    private func withHandle<T>(_ body: (OpaquePointer) throws -> T) rethrows -> T? {
        gate.lock()
        guard !closing, let h = handle else {
            gate.unlock()
            return nil
        }
        inFlight += 1
        gate.unlock()
        defer {
            gate.lock()
            inFlight -= 1
            if inFlight == 0 { gate.broadcast() }
            gate.unlock()
        }
        return try body(h)
    }

    /// Starts the daemon against `configDir`.
    ///
    /// Never touches the real ~/.claude or ~/.codex trees, and no longer
    /// relies on the caller to have arranged that: the C ABI itself refuses
    /// to start unless both session roots are declared, reporting
    /// `roots-not-declared`, which surfaces here as
    /// `TCError.rootsNotDeclared`.
    ///
    /// `settingsJSON` is applied and durably persisted BEFORE the watcher's
    /// first tick, and the roots check runs after it -- so passing a settings
    /// object naming both folders is how a caller turns a refusal into a
    /// running daemon in one call. That is the roots screen's entire
    /// mechanism. Passing nil (the default) uses whatever is already
    /// persisted, exactly like the old initializer.
    ///
    /// Do not build `settingsJSON` by string concatenation: a folder a
    /// contributor picked can contain quotes and backslashes. Encode it.
    public init(configDir: String, settingsJSON: String? = nil) throws {
        var errPtr: UnsafeMutablePointer<CChar>?
        let h: OpaquePointer? = configDir.withCString { cDir in
            withUnsafeMutablePointer(to: &errPtr) { errOut in
                if let settingsJSON {
                    return settingsJSON.withCString { cSettings in
                        tc_daemon_start_with_settings(cDir, cSettings, errOut)
                    }
                }
                return tc_daemon_start(cDir, errOut)
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
            if message == Self.rootsNotDeclaredLabel {
                throw TCError.rootsNotDeclared
            }
            throw TCError.startFailed(message)
        }
        self.handle = h
    }

    /// Calls `method` with `paramsJSON` (a JSON object literal, e.g. "{}")
    /// and returns the daemon's JSON response as a Swift String. Never
    /// throws: per the header, tc_call never returns NULL, it returns a
    /// JSON error frame on failure.
    ///
    /// Once `shutdown` has begun this refuses with the same `unavailable`
    /// frame the ABI itself would produce for a stopped daemon, rather than
    /// racing teardown for the pointer.
    public func call(_ method: String, params paramsJSON: String = "{}") -> String {
        let result: UnsafeMutablePointer<CChar>?? = withHandle { h in
            method.withCString { cMethod in
                paramsJSON.withCString { cParams in
                    tc_call(h, cMethod, cParams)
                }
            }
        }
        // Outer optional: teardown refused us. Inner optional: the ABI
        // returned NULL, which the header says never happens.
        guard let inner = result else {
            return "{\"error\":{\"code\":\"unavailable\",\"message\":\"handle-freed\"}}"
        }
        guard let resultPtr = inner else {
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
        var errPtr: UnsafeMutablePointer<CChar>?
        let opened: OpaquePointer?? = withHandle { h in
            entryID.withCString { cEntry in
                withUnsafeMutablePointer(to: &errPtr) { errOut in
                    tc_preview_open(h, cEntry, errOut)
                }
            }
        }
        // Outer optional nil: the handle is gone or teardown started. The
        // redaction pass itself runs inside the gate, so the handle cannot
        // be freed underneath it.
        guard let inner = opened else { throw TCError.daemonGone }
        guard let p = inner else {
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
        // ctx must stay alive until tc_unsubscribe RETURNS, per the header's
        // SUBSCRIPTION LIFETIME rule -- retained here, released only by a
        // tc_unsubscribe that we confirmed was not refused.
        let box = TCCallbackBox(handler)
        let ctx = Unmanaged.passRetained(box).toOpaque()
        let registered: UInt64? = withHandle { h in
            tc_subscribe(
                h,
                { eventJSON, ctx in
                    guard let eventJSON, let ctx else { return }
                    let box = Unmanaged<TCCallbackBox>.fromOpaque(ctx).takeUnretainedValue()
                    // The event_json pointer is borrowed for this call only,
                    // so it is copied into a Swift String before anything
                    // else.
                    box.handler(String(cString: eventJSON))
                },
                ctx
            )
        }
        // Refused by teardown: no subscription was ever registered, so no
        // callback can fire and the ctx retain is ours to drop.
        let token = registered ?? 0
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
        let confirmed: Bool? = withHandle { h in
            Self.attemptUnsubscribe(h, subscription)
        }
        guard confirmed == true else { return false }
        Unmanaged<TCCallbackBox>.fromOpaque(subscription.ctx).release()
        return true
    }

    /// One tc_unsubscribe attempt against a handle the caller has already
    /// established is live. Returns whether the barrier held; does NOT
    /// release ctx -- that is the caller's decision, because only the caller
    /// knows whether it is done retrying.
    private static func attemptUnsubscribe(
        _ h: OpaquePointer,
        _ subscription: TCSubscription
    ) -> Bool {
        let before = tc_last_error().map { String(cString: $0) }
        tc_unsubscribe(h, subscription.token)
        let after = tc_last_error().map { String(cString: $0) }
        if let after, after != before { return false }
        return true
    }

    // MARK: - Teardown

    /// Stops the daemon loop. Idempotent. Does NOT end subscriptions and is
    /// not a synchronization point for them -- unsubscribe first. A no-op
    /// once `shutdown` has begun, which already stops the daemon itself.
    public func stop() {
        _ = withHandle { h in tc_daemon_stop(h) }
    }

    /// Ordered teardown: refuse new calls, wait for in-flight calls to
    /// leave, unsubscribe (the ABI's only real barrier), stop, free.
    ///
    /// MUST be called from a plain thread that is not inside a tokio runtime
    /// and not inside a tc_subscribe callback. The main thread qualifies;
    /// Swift's cooperative pool is not a tokio runtime.
    ///
    /// Blocks the calling thread for up to `drainTimeout` plus
    /// `unsubscribeTimeout`. That is the point: teardown that does not block
    /// is teardown that does not know whether it is safe to free. The
    /// previous implementation slept a fixed 200ms and freed regardless,
    /// which is the defect this replaces -- a sleep is a guess, and a guess
    /// that loses is a use-after-free.
    ///
    /// Returns `.leaked` -- WITHOUT freeing -- if either wait fails. See the
    /// note at the top of this type for why leaking is the correct answer.
    @discardableResult
    public func shutdown(
        unsubscribing subscription: TCSubscription? = nil,
        drainTimeout: TimeInterval = 3.0,
        unsubscribeTimeout: TimeInterval = 2.0
    ) -> ShutdownOutcome {
        gate.lock()
        if closing || handle == nil {
            // A second call. The first one either freed the handle or
            // deliberately leaked it; either way there is nothing here to
            // do, and re-entering teardown is exactly the concurrent-free
            // the header forbids.
            gate.unlock()
            return .freed
        }
        closing = true
        let deadline = Date().addingTimeInterval(drainTimeout)
        while inFlight > 0 {
            if !gate.wait(until: deadline) { break }
        }
        let drained = inFlight == 0
        let h = handle
        // Nulled under the lock whatever happens next: no Swift path may
        // reach this pointer again, freed or leaked.
        handle = nil
        gate.unlock()

        guard let h else { return .freed }
        guard drained else {
            // A call is still inside the C ABI with this pointer. Freeing it
            // now is the use-after-free. Leak it.
            return .leaked("in-flight-calls-did-not-drain")
        }

        if let subscription {
            // The main thread is not inside a tokio runtime, so the first
            // attempt should hold. If the ABI refuses anyway (it refuses
            // silently, hence the tc_last_error comparison inside
            // attemptUnsubscribe), retry once on a genuinely fresh thread
            // and JOIN it -- with a deadline, because tc_unsubscribe blocks
            // until the callback is guaranteed not to fire again and we
            // cannot let a hung callback hang app termination.
            var confirmed = Self.attemptUnsubscribe(h, subscription)
            if !confirmed {
                confirmed = Self.retryUnsubscribeOnPlainThread(
                    h,
                    subscription,
                    timeout: unsubscribeTimeout
                )
            }
            guard confirmed else {
                // The barrier did not hold, or we stopped waiting for it. A
                // callback can still fire, using ctx and the handle. So ctx
                // is NOT released and the handle is NOT freed. If the retry
                // thread is still running it is operating on a pointer that
                // stays valid forever, which is precisely why leaking is
                // safe here.
                return .leaked("unsubscribe-not-confirmed")
            }
            Unmanaged<TCCallbackBox>.fromOpaque(subscription.ctx).release()
        }

        // Sequential, on this one thread: tc_daemon_stop must not run
        // concurrently with tc_handle_free, and nothing else can reach `h`
        // now.
        tc_daemon_stop(h)
        tc_handle_free(h)
        return .freed
    }

    /// Retries `attemptUnsubscribe` on a new plain Thread and waits for it,
    /// bounded. A Thread rather than a task or a queue on purpose: the
    /// header's refusal check is "am I inside any tokio runtime context",
    /// which is thread-local, so the retry has to be a thread that has
    /// demonstrably never entered one.
    private static func retryUnsubscribeOnPlainThread(
        _ h: OpaquePointer,
        _ subscription: TCSubscription,
        timeout: TimeInterval
    ) -> Bool {
        let done = NSCondition()
        // A reference box rather than captured `var`s: the retry thread can
        // outlive this function (that is the whole timeout case), and a
        // class the box and this frame share keeps that legal instead of
        // mutating a stack variable from two threads.
        let state = UnsubscribeRetryState()
        // Both the handle and the subscription cross a thread boundary
        // here on purpose, which is exactly what the header asks for (a
        // plain thread with no runtime context). `SendableHandle` states
        // that intent rather than leaving it to a warning: `h` is not freed
        // by anyone while this retry can still run -- shutdown either
        // confirms the retry and frees afterwards, or gives up and leaks.
        let carried = SendableHandle(raw: h, subscription: subscription)
        let thread = Thread {
            let ok = attemptUnsubscribe(carried.raw, carried.subscription)
            done.lock()
            state.result = ok
            state.finished = true
            done.broadcast()
            done.unlock()
        }
        thread.start()

        let deadline = Date().addingTimeInterval(timeout)
        done.lock()
        while !state.finished {
            if !done.wait(until: deadline) { break }
        }
        let outcome = state.finished ? state.result : false
        done.unlock()
        return outcome
    }

    /// Kept for callers that never subscribed (the FFI demo). Same drain and
    /// same leak-rather-than-free rule as `shutdown`.
    @discardableResult
    public func close() -> ShutdownOutcome {
        shutdown()
    }

    deinit {
        // Best-effort only, and deliberately timid: a host should call
        // shutdown() explicitly, as AppModel.shutdown() does. Anything that
        // was still using the handle would have kept this object alive, so
        // reaching deinit with work in flight should be impossible -- but if
        // the counters say otherwise, leak rather than free.
        gate.lock()
        let h = handle
        let idle = inFlight == 0 && !closing
        handle = nil
        gate.unlock()
        guard let h, idle else { return }
        tc_daemon_stop(h)
        tc_handle_free(h)
    }
}

/// The handle plus subscription, moved to the retry thread. Unchecked
/// because a raw C pointer carries no Swift-visible ownership: the safety
/// argument is the leak-rather-than-free rule on `TCDaemon.shutdown`, not
/// anything the compiler can see.
private struct SendableHandle: @unchecked Sendable {
    let raw: OpaquePointer
    let subscription: TCSubscription
}

/// Shared state for the bounded unsubscribe retry. Every field is read and
/// written only while that retry's own NSCondition is held.
private final class UnsubscribeRetryState: @unchecked Sendable {
    var finished = false
    var result = false
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
