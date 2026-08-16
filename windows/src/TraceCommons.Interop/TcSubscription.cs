using System;
using System.Runtime.InteropServices;

namespace TraceCommons.Interop;

/// <summary>
/// A live subscription token plus the GC roots keeping its callback and ctx
/// alive.
///
/// Opaque on purpose: only <see cref="TcDaemon.Unsubscribe"/> and
/// <see cref="TcDaemon.Shutdown"/> may release the roots, and only after the
/// ABI has confirmed the unsubscribe barrier actually held. Releasing them any
/// earlier -- on a refused unsubscribe, or on a mere tc_daemon_stop -- hands
/// Rust a dangling function pointer and a dangling ctx.
///
/// This type deliberately does NOT implement <see cref="IDisposable"/>. A
/// using-block would suggest the roots can be released locally, which is the
/// one thing that must not happen without the daemon's confirmation.
/// </summary>
public sealed class TcSubscription
{
    private GCHandle _ctxHandle;
    private GCHandle _callbackHandle;

    internal TcSubscription(ulong token, GCHandle ctxHandle, GCHandle callbackHandle)
    {
        Token = token;
        _ctxHandle = ctxHandle;
        _callbackHandle = callbackHandle;
    }

    /// <summary>
    /// The nonzero token tc_subscribe returned. Zero is never valid, so a
    /// constructed instance always carries a real subscription.
    /// </summary>
    internal ulong Token { get; }

    /// <summary>
    /// Whether the roots are still held. False once a confirmed unsubscribe
    /// has released them; a subscription in that state must not be reused.
    /// </summary>
    public bool RootsHeld => _ctxHandle.IsAllocated || _callbackHandle.IsAllocated;

    /// <summary>
    /// Releases the delegate and ctx roots. Called ONLY after tc_unsubscribe
    /// has been confirmed to have held -- at which point the header guarantees
    /// no further callback can fire, so nothing native can reach either root
    /// again.
    ///
    /// Idempotent: a double release would otherwise throw on an already-freed
    /// GCHandle, and teardown paths are exactly where a redundant call is
    /// likeliest.
    /// </summary>
    internal void ReleaseRoots()
    {
        if (_ctxHandle.IsAllocated)
        {
            _ctxHandle.Free();
        }

        if (_callbackHandle.IsAllocated)
        {
            _callbackHandle.Free();
        }
    }
}
