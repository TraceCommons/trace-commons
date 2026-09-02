using System;

namespace TraceCommons.Interop;

/// <summary>
/// Decides whether a daemon event may re-ask IronWire, and whether an answer
/// that comes back is still about the declaration the contributor has now.
/// </summary>
/// <remarks>
/// <para>
/// The routing surface used to read <c>status.routing</c> once, on load, and
/// again only after the contributor wrote something. A state change while the
/// pane sat open -- the proxy coming up, the daemon restarting, the reader
/// moving from awaiting-rows to rows-seen -- was invisible until they touched
/// the card. The GTK shell has always repainted on every daemon event; this
/// type is the part of that behaviour worth testing off Windows, so the
/// WinUI half stays a subscription and two calls.
/// </para>
/// <para>
/// It is deliberately not a timer. The shell already receives daemon events
/// (<c>DaemonHost.StatusChanged</c>, which the ABI's lag and resync frames
/// also raise), and a second clock ticking beside them would ask IronWire on
/// a schedule nobody asked for.
/// </para>
/// <para>
/// <b>No wording, no words.</b> Nothing here reads a source mode, a
/// declaration switch, or a rendered string: every per-tool word stays a
/// function of what <c>probe_routed_tools</c> answered. This decides only
/// <i>when</i> to ask.
/// </para>
/// </remarks>
public sealed class RoutingRefreshGate
{
    /// <summary>
    /// How long a held answer keeps a daemon event from asking again.
    /// </summary>
    /// <remarks>
    /// A backstop, not the primary invalidation -- that is a contributor
    /// changing the declaration, which calls <see cref="Forget"/>. Expiry
    /// cannot flicker a word, because a re-ask does not drop the held answer:
    /// the previous verdict stays on screen until a new one lands, so an
    /// expiry is invisible unless the answer actually changed. Matches the
    /// GTK shell's <c>EVIDENCE_BACKSTOP_TTL</c>.
    /// </remarks>
    public static readonly TimeSpan EvidenceBackstop = TimeSpan.FromMinutes(5);

    private bool _inFlight;
    private DateTimeOffset? _answeredAt;
    private long _generation;

    /// <summary>Whether a probe started here has not yet been completed.</summary>
    public bool IsProbeInFlight => _inFlight;

    /// <summary>Whether an answer is held, and so when it was taken.</summary>
    public DateTimeOffset? AnsweredAt => _answeredAt;

    /// <summary>
    /// A daemon event arrived. Returns whether it may ask IronWire, and the
    /// ticket the answer must be completed with.
    /// </summary>
    /// <remarks>
    /// Three refusals, each for its own reason: nothing is declared, so there
    /// is nothing to ask about; a call is already in flight, so a second
    /// would open a second connection to the same proxy; or an answer taken
    /// less than <see cref="EvidenceBackstop"/> ago is still held, and events
    /// arrive far faster than a proxy's wiring changes.
    /// </remarks>
    public bool TryBeginProbe(bool declared, DateTimeOffset now, out long ticket)
    {
        ticket = _generation;
        if (!declared || _inFlight)
        {
            return false;
        }

        if (_answeredAt is { } answered && now - answered < EvidenceBackstop)
        {
            return false;
        }

        _inFlight = true;
        return true;
    }

    /// <summary>
    /// A contributor pressed something. Always allowed: the guards above are
    /// about events nobody asked for, and this is the one path where a fresh
    /// answer is owed whatever is held.
    /// </summary>
    public long BeginProbe()
    {
        _inFlight = true;
        return _generation;
    }

    /// <summary>
    /// An answer came back. Returns whether it may still be used.
    /// </summary>
    /// <remarks>
    /// False when the declaration changed while the call was in flight: that
    /// answer is about a port or a folder this machine is no longer pointed
    /// at, and painting it would be exactly the stale verdict
    /// <see cref="Forget"/> exists to prevent. The stamp is only taken on an
    /// answer that may be used, so a discarded one does not hold the next
    /// event off.
    /// </remarks>
    public bool CompleteWithAnswer(long ticket, DateTimeOffset now)
    {
        _inFlight = false;
        if (ticket != _generation)
        {
            return false;
        }

        _answeredAt = now;
        return true;
    }

    /// <summary>
    /// The call did not run, or answered something unreadable.
    /// </summary>
    /// <remarks>
    /// No stamp is taken. A call that did not run is not a fact about any
    /// tool, and stamping it would hold the next event off for five minutes
    /// on the strength of having learned nothing. Returns whether the ticket
    /// is still current, so the caller can tell a failed check about the
    /// current declaration -- which is worth saying -- from one about a
    /// declaration that has since been replaced, which is not.
    /// </remarks>
    public bool CompleteWithoutAnswer(long ticket)
    {
        _inFlight = false;
        return ticket == _generation;
    }

    /// <summary>
    /// The declaration changed. Drops the held stamp and invalidates every
    /// ticket already out, so the next event asks again and no in-flight
    /// answer about the old declaration can land.
    /// </summary>
    public void Forget()
    {
        _answeredAt = null;
        _generation++;
    }
}
