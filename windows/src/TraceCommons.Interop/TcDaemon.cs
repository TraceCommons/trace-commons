using System;
using System.Runtime.InteropServices;
using System.Threading;

namespace TraceCommons.Interop;

/// <summary>
/// Safe wrapper around the trace-commons-contributor-ffi C ABI.
///
/// This type, with <see cref="TcPreview"/> and <see cref="TcSubscription"/>,
/// is the ONLY place in the Windows app where a raw handle pointer appears.
/// It is a deliberate port of <c>macos/Sources/TCBridge/TCDaemon.swift</c>:
/// same counted gate, same ordered teardown, same leak-rather-than-free rule.
/// Where the two differ, the difference is a .NET fact and is commented as
/// such -- nothing here is a redesign, because the constraints being satisfied
/// belong to the ABI, not to the host language.
///
/// WHY A COUNTED GATE AND NOT A LOCK.
///
/// The header permits tc_call / tc_preview_open / tc_subscribe to run
/// concurrently on many threads, and forbids only that tc_handle_free race
/// them. That is a shared/exclusive requirement, not a mutual-exclusion one. A
/// plain <c>lock</c> around every call would serialize a preview redaction
/// pass -- documented as blocking for as long as reading and redacting a
/// session file takes -- ahead of every unrelated status poll. So: a monitor
/// guarding an in-flight counter plus a closing flag. Any number of callers
/// may hold the handle; teardown sets the flag, waits for the count to reach
/// zero, and only then unsubscribes, stops, and frees, in that order.
///
/// AND IF THE WAIT DOES NOT COMPLETE, WE LEAK. A handle still reachable by an
/// in-flight call, or a subscription whose barrier the ABI would not confirm,
/// is never passed to tc_handle_free. The process is exiting; an unfreed
/// handle costs nothing and a use-after-free is a crash or worse.
///
/// Safe to use from any thread.
/// </summary>
public sealed class TcDaemon : IDisposable
{
    private readonly object _gate = new();
    private IntPtr _handle;
    private int _inFlight;
    private bool _closing;

    /// <summary>What teardown managed to do.</summary>
    public enum ShutdownOutcome
    {
        /// <summary>Everything drained; the handle was freed.</summary>
        Freed,

        /// <summary>
        /// Teardown could not prove the handle was idle, so it was NOT freed.
        /// A deliberate, safe outcome -- not an error to recover from.
        /// </summary>
        Leaked,
    }

    /// <summary>
    /// The response the wrapper synthesizes when teardown has refused a call.
    /// Deliberately the same <c>unavailable</c> shape the ABI itself produces
    /// for a stopped daemon, so callers need no separate branch for "the host
    /// is shutting down" versus "the daemon stopped".
    /// </summary>
    private const string UnavailableFrame =
        "{\"error\":{\"code\":\"unavailable\",\"message\":\"handle-freed\"}}";

    private const string NullResponseFrame =
        "{\"error\":{\"code\":\"unavailable\",\"message\":\"null-response\"}}";

    /// <summary>
    /// How many calls are inside the C ABI right now. Diagnostics only -- true
    /// the instant it is read and possibly not the instant after.
    /// <see cref="Shutdown"/> does not consult it; it waits on the monitor.
    /// </summary>
    public int InFlightCalls
    {
        get
        {
            lock (_gate)
            {
                return _inFlight;
            }
        }
    }

    /// <summary>
    /// Starts the daemon against <paramref name="configDir"/>.
    ///
    /// The C ABI has no call to set claude_root / codex_root after start, so
    /// the caller is expected to have already written
    /// <c>configDir/daemon-settings.json</c> pointing those roots somewhere
    /// deliberate. <c>DaemonHost</c> does exactly that; this constructor never
    /// touches a real transcript tree on its own.
    /// </summary>
    /// <exception cref="TcException">
    /// The daemon could not start -- most commonly because another daemon
    /// already holds daemon.lock for this directory.
    /// </exception>
    /// <param name="configDir">The daemon's state directory.</param>
    /// <param name="settingsJson">
    /// Settings applied BEFORE the watcher's first tick, or null to use
    /// whatever is persisted. This is the only way to point the daemon at a
    /// session store other than the real one from the very first pass --
    /// <c>set_settings</c> only takes effect on an already-running daemon, by
    /// which time the first scan has already happened. Accepts the same fields
    /// <c>set_settings</c> does; an unrecognized key is rejected, not ignored.
    /// </param>
    public TcDaemon(string configDir, string? settingsJson = null)
    {
        ArgumentNullException.ThrowIfNull(configDir);

        IntPtr errPtr;
        IntPtr handle = settingsJson is null
            ? NativeMethods.tc_daemon_start(configDir, out errPtr)
            : NativeMethods.tc_daemon_start_with_settings(configDir, settingsJson, out errPtr);
        if (handle == IntPtr.Zero)
        {
            // The error string is owned even though the call failed; taking it
            // frees it.
            string message = NativeMethods.TakeOwnedString(errPtr) ?? "unknown error";
            throw new TcException($"tc_daemon_start failed: {message}");
        }

        // Defensive: a non-null handle with a non-null err would be an ABI
        // contract break, but leaking the string if it ever happened is worse
        // than one branch here.
        if (errPtr != IntPtr.Zero)
        {
            NativeMethods.tc_string_free(errPtr);
        }

        _handle = handle;
    }

    /// <summary>
    /// Runs <paramref name="body"/> with the raw handle, or returns
    /// <paramref name="refused"/> if the handle is gone or teardown has begun.
    ///
    /// Every use of the pointer goes through here. That is the whole
    /// invariant: while <paramref name="body"/> runs the in-flight count is
    /// nonzero, and <see cref="Shutdown"/> cannot reach tc_handle_free while
    /// the count is nonzero. It deliberately does NOT serialize callers
    /// against each other, because the header permits that concurrency.
    /// </summary>
    private T WithHandle<T>(Func<IntPtr, T> body, T refused)
    {
        lock (_gate)
        {
            if (_closing || _handle == IntPtr.Zero)
            {
                return refused;
            }

            _inFlight++;
        }

        try
        {
            // Read outside the lock: the pointer value cannot change while the
            // in-flight count is nonzero, because only teardown clears it and
            // teardown cannot proceed past a nonzero count.
            return body(_handle);
        }
        finally
        {
            lock (_gate)
            {
                _inFlight--;
                if (_inFlight == 0)
                {
                    Monitor.PulseAll(_gate);
                }
            }
        }
    }

    /// <summary>
    /// Calls <paramref name="method"/> with <paramref name="paramsJson"/> and
    /// returns the daemon's raw JSON response.
    ///
    /// Never throws: per the header tc_call never returns NULL, it returns a
    /// JSON error frame. Once teardown has begun this refuses with the same
    /// <c>unavailable</c> frame rather than racing teardown for the pointer.
    /// </summary>
    public string Call(string method, string paramsJson = "{}")
    {
        ArgumentNullException.ThrowIfNull(method);
        ArgumentNullException.ThrowIfNull(paramsJson);

        return WithHandle(
            h => NativeMethods.TakeOwnedString(NativeMethods.tc_call(h, method, paramsJson))
                 ?? NullResponseFrame,
            UnavailableFrame);
    }

    /// <summary>
    /// Opens the in-process preview for <paramref name="entryId"/>: the
    /// redacted body plus a summary. This is the C ABI's deliberate content
    /// exemption -- the socket's <c>preview</c> method returns the summary
    /// only.
    ///
    /// Blocks the calling thread for the duration of the redaction pass, so
    /// callers must run it off the UI thread.
    /// </summary>
    /// <exception cref="TcException">
    /// The entry is unknown, the redacted body cannot be represented as a
    /// NUL-terminated C string, or the daemon handle is already gone.
    /// </exception>
    public TcPreview OpenPreview(string entryId)
    {
        ArgumentNullException.ThrowIfNull(entryId);

        // The err out-param is written by the native call, so it is carried
        // out of the closure rather than captured by it.
        (IntPtr preview, IntPtr err, bool refused) = WithHandle(
            h =>
            {
                IntPtr p = NativeMethods.tc_preview_open(h, entryId, out IntPtr e);
                return (p, e, false);
            },
            (IntPtr.Zero, IntPtr.Zero, true));

        if (refused)
        {
            throw new TcException("daemon handle already freed");
        }

        if (preview == IntPtr.Zero)
        {
            string message = NativeMethods.TakeOwnedString(err) ?? "unknown error";
            throw new TcException($"tc_preview_open failed: {message}");
        }

        if (err != IntPtr.Zero)
        {
            NativeMethods.tc_string_free(err);
        }

        return new TcPreview(preview);
    }

    /// <summary>
    /// Registers <paramref name="handler"/>, invoked with each JSON event
    /// frame the daemon publishes, ON A RUST BACKGROUND THREAD. The handler
    /// must therefore marshal to the UI thread itself before touching
    /// observable state; <c>DaemonHost</c> is the one call site that does.
    ///
    /// Returns null if the ABI refused -- a NULL handle or a stopped daemon;
    /// token 0 is never valid.
    /// </summary>
    public TcSubscription? Subscribe(Action<string> handler)
    {
        ArgumentNullException.ThrowIfNull(handler);

        // Both the delegate and the ctx box must outlive every possible
        // callback, i.e. until tc_unsubscribe RETURNS. The delegate needs its
        // own root because native code holding a function pointer does not
        // keep the managed delegate alive; without this the GC is free to
        // collect it and the next event is a crash in native code.
        var box = new CallbackBox(handler);
        GCHandle ctxHandle = GCHandle.Alloc(box);
        IntPtr ctx = GCHandle.ToIntPtr(ctxHandle);
        NativeMethods.TcEventCallback callback = OnEvent;
        GCHandle callbackHandle = GCHandle.Alloc(callback);

        ulong token = WithHandle(h => NativeMethods.tc_subscribe(h, callback, ctx), 0UL);

        if (token == 0)
        {
            // Refused, so no subscription was ever registered and no callback
            // can fire. Both roots are ours to drop.
            callbackHandle.Free();
            ctxHandle.Free();
            return null;
        }

        return new TcSubscription(token, ctxHandle, callbackHandle);
    }

    /// <summary>
    /// The single native callback entry point. Static so that the delegate
    /// closes over nothing: the per-subscription state travels through ctx,
    /// which is what the ABI is for.
    /// </summary>
    private static void OnEvent(IntPtr eventJson, IntPtr ctx)
    {
        if (eventJson == IntPtr.Zero || ctx == IntPtr.Zero)
        {
            return;
        }

        // A managed exception must never unwind into Rust. The ABI catches its
        // own panics at this boundary; this is the mirror obligation on our
        // side, and it is why the whole body is wrapped.
        try
        {
            if (GCHandle.FromIntPtr(ctx).Target is not CallbackBox box)
            {
                return;
            }

            // The event_json pointer is borrowed for this call only, so it is
            // copied before anything else happens.
            string? json = NativeMethods.BorrowedString(eventJson);
            if (json is not null)
            {
                box.Handler(json);
            }
        }
        catch
        {
            // Swallowed deliberately. There is no channel to report on that
            // would not itself risk unwinding, and a handler that throws is a
            // host bug the host must find in its own logs.
        }
    }

    /// <summary>
    /// Cancels <paramref name="subscription"/> and releases its roots.
    ///
    /// tc_unsubscribe returns void and REFUSES SILENTLY when called from a
    /// thread inside any tokio runtime context. A host that assumed success
    /// would free ctx while callbacks can still fire. So the prior
    /// tc_last_error is noted first and a NEW error after the call means the
    /// barrier did not hold: the roots are kept, the token stays valid, and
    /// this returns false so the caller can retry from a plain thread.
    ///
    /// Call this from a plain thread. The .NET thread pool is not a tokio
    /// runtime, so it qualifies -- but see <see cref="AttemptUnsubscribe"/>
    /// for why that is checked rather than assumed.
    /// </summary>
    public bool Unsubscribe(TcSubscription subscription)
    {
        ArgumentNullException.ThrowIfNull(subscription);

        bool confirmed = WithHandle(h => AttemptUnsubscribe(h, subscription.Token), false);
        if (!confirmed)
        {
            return false;
        }

        subscription.ReleaseRoots();
        return true;
    }

    /// <summary>
    /// One tc_unsubscribe attempt against a handle the caller has already
    /// established is live. Returns whether the barrier held; does NOT release
    /// the roots, because only the caller knows whether it is done retrying.
    ///
    /// The pre/post comparison is the best a thread-local, last-error-only
    /// surface allows: a refusal that recorded a genuinely identical string to
    /// one already sitting there would read as stale. Both reads and the call
    /// itself are on this one thread with no await between them, which is what
    /// makes the thread-local read meaningful at all.
    /// </summary>
    private static bool AttemptUnsubscribe(IntPtr handle, ulong token)
    {
        string? before = NativeMethods.BorrowedString(NativeMethods.tc_last_error());
        NativeMethods.tc_unsubscribe(handle, token);
        string? after = NativeMethods.BorrowedString(NativeMethods.tc_last_error());
        return after is null || after == before;
    }

    /// <summary>
    /// Stops the daemon loop. Idempotent. Does NOT end subscriptions and is
    /// not a synchronization point for them -- unsubscribe first.
    /// </summary>
    public void Stop()
    {
        WithHandle<object?>(
            h =>
            {
                NativeMethods.tc_daemon_stop(h);
                return null;
            },
            null);
    }

    /// <summary>
    /// Ordered teardown: refuse new calls, wait for in-flight calls to leave,
    /// unsubscribe (the ABI's only real barrier), stop, free.
    ///
    /// MUST be called from a plain thread that is not inside a tc_subscribe
    /// callback. Blocks the calling thread for up to
    /// <paramref name="drainTimeout"/> plus <paramref name="unsubscribeTimeout"/>.
    /// That is the point: teardown that does not block is teardown that does
    /// not know whether it is safe to free.
    ///
    /// Returns <see cref="ShutdownOutcome.Leaked"/> -- WITHOUT freeing -- if
    /// either wait fails.
    /// </summary>
    public ShutdownOutcome Shutdown(
        TcSubscription? subscription = null,
        TimeSpan? drainTimeout = null,
        TimeSpan? unsubscribeTimeout = null)
    {
        TimeSpan drain = drainTimeout ?? TimeSpan.FromSeconds(3);
        TimeSpan unsubscribe = unsubscribeTimeout ?? TimeSpan.FromSeconds(2);

        IntPtr handle;
        bool drained;

        lock (_gate)
        {
            if (_closing || _handle == IntPtr.Zero)
            {
                // A second call. The first either freed the handle or
                // deliberately leaked it; re-entering teardown is exactly the
                // concurrent free the header forbids.
                return ShutdownOutcome.Freed;
            }

            _closing = true;

            // Monitor.Wait can wake spuriously and consumes the timeout per
            // wait, so the deadline is absolute rather than a per-iteration
            // budget.
            DateTime deadline = DateTime.UtcNow + drain;
            while (_inFlight > 0)
            {
                TimeSpan remaining = deadline - DateTime.UtcNow;
                if (remaining <= TimeSpan.Zero || !Monitor.Wait(_gate, remaining))
                {
                    break;
                }
            }

            drained = _inFlight == 0;
            handle = _handle;

            // Nulled under the lock whatever happens next: no managed path may
            // reach this pointer again, freed or leaked.
            _handle = IntPtr.Zero;
        }

        if (handle == IntPtr.Zero)
        {
            return ShutdownOutcome.Freed;
        }

        if (!drained)
        {
            // A call is still inside the C ABI with this pointer. Freeing it
            // now is the use-after-free. Leak it.
            return ShutdownOutcome.Leaked;
        }

        if (subscription is not null)
        {
            bool confirmed = AttemptUnsubscribe(handle, subscription.Token);
            if (!confirmed)
            {
                confirmed = RetryUnsubscribeOnPlainThread(handle, subscription.Token, unsubscribe);
            }

            if (!confirmed)
            {
                // The barrier did not hold, or we stopped waiting for it. A
                // callback can still fire, using ctx and the handle. So the
                // roots are NOT released and the handle is NOT freed -- which
                // is precisely why leaking is safe: the pointer stays valid
                // forever.
                return ShutdownOutcome.Leaked;
            }

            subscription.ReleaseRoots();
        }

        // Sequential, on this one thread: tc_daemon_stop must not run
        // concurrently with tc_handle_free, and nothing else can reach the
        // handle now.
        NativeMethods.tc_daemon_stop(handle);
        NativeMethods.tc_handle_free(handle);
        return ShutdownOutcome.Freed;
    }

    /// <summary>
    /// Retries the unsubscribe on a brand-new plain thread and waits for it,
    /// bounded.
    ///
    /// A dedicated <see cref="Thread"/> rather than a thread-pool work item on
    /// purpose: the ABI's refusal check is "am I inside any tokio runtime
    /// context", which is thread-local, so the retry has to run on a thread
    /// that has demonstrably never entered one. A pooled thread has run
    /// arbitrary prior work and cannot make that claim.
    ///
    /// The wait is bounded because tc_unsubscribe blocks until the callback is
    /// guaranteed not to fire again, and a hung callback must not hang app
    /// termination. On timeout the thread is abandoned -- legal precisely
    /// because the caller then leaks the handle rather than freeing it.
    /// </summary>
    private static bool RetryUnsubscribeOnPlainThread(IntPtr handle, ulong token, TimeSpan timeout)
    {
        bool result = false;
        using var finished = new ManualResetEventSlim(false);

        var thread = new Thread(() =>
        {
            try
            {
                result = AttemptUnsubscribe(handle, token);
            }
            finally
            {
                // The waiter may already have timed out and disposed the
                // event; signalling a disposed event must not take the
                // process down on a teardown path.
                try
                {
                    finished.Set();
                }
                catch (ObjectDisposedException)
                {
                }
            }
        })
        {
            IsBackground = true,
            Name = "tc-unsubscribe-retry",
        };

        thread.Start();
        return finished.Wait(timeout) && result;
    }

    /// <summary>
    /// Best-effort teardown for callers that never subscribed. A host should
    /// call <see cref="Shutdown"/> explicitly with its subscription.
    /// </summary>
    public void Dispose() => Shutdown();

    /// <summary>
    /// Heap box carrying the managed handler across the <c>void* ctx</c>
    /// boundary. A C function pointer cannot capture, so the handler travels
    /// this way.
    /// </summary>
    private sealed class CallbackBox
    {
        internal CallbackBox(Action<string> handler) => Handler = handler;

        internal Action<string> Handler { get; }
    }
}

/// <summary>An error raised by the trace-commons C ABI boundary.</summary>
public sealed class TcException : Exception
{
    public TcException(string message)
        : base(message)
    {
    }
}
