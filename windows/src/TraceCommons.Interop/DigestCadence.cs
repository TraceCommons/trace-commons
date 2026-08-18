using System;

namespace TraceCommons.Interop;

/// <summary>
/// The one rule that governs every interruption this app is allowed to make:
/// at most one digest every four hours, and none at all when nothing is
/// waiting.
/// </summary>
/// <remarks>
/// <para>
/// That is not a preference. The onboarding "Done" screen tells a
/// contributor, in <c>OnboardingWindow.xaml</c> and word for word on all
/// three shells, that they "will get at most one notification every 4 hours,
/// and none at all if there's nothing waiting". A notification path that can
/// exceed it turns that sentence into a false promise, and the contributor
/// has no way to check.
/// </para>
/// <para>
/// <b>Why this exists when the daemon already decides.</b> The daemon is the
/// primary gate: <c>daemon/notify.rs::digest_due</c> refuses on an empty
/// queue and otherwise only fires once per <c>digest_interval_secs</c>,
/// stamping <c>last_digest_at</c> into persisted state so the spacing
/// survives a restart. The shell's job is only to render the
/// <c>digest_due</c> event it receives. This class is a second, in-process
/// backstop for the ways a shell can still over-notify with a
/// correctly-behaving daemon behind it: a resubscribe that replays, a
/// duplicate handler registration, a future caller that decides to post a
/// digest from somewhere other than the event. It never *causes* a digest --
/// it can only suppress one.
/// </para>
/// <para>
/// <b>Why claim-and-stamp is one call.</b> A separate <c>ShouldPost</c> and
/// <c>RecordPosted</c> pair can be got wrong in the direction that breaks the
/// promise: ask, post, forget to record, ask again a minute later, post
/// again. <see cref="TryClaim"/> stamps as it answers, so the only way to be
/// told yes is to have consumed the window.
/// </para>
/// <para>
/// Deliberately not persisted. On restart the daemon's own
/// <c>last_digest_at</c> still holds the spacing, and persisting a second
/// copy would introduce a way for the two to disagree.
/// </para>
/// </remarks>
public sealed class DigestCadence
{
    /// <summary>
    /// Four hours, matching the daemon's default <c>digest_interval_secs</c>
    /// (14400) and the sentence the onboarding screen shows.
    /// </summary>
    public static readonly TimeSpan MinimumInterval = TimeSpan.FromHours(4);

    private readonly TimeSpan _interval;
    private DateTimeOffset? _lastClaimedAt;

    /// <summary>
    /// Uses the shipped four-hour interval.
    /// </summary>
    public DigestCadence()
        : this(MinimumInterval)
    {
    }

    /// <summary>
    /// Uses an explicit interval. For tests only -- nothing in the app may
    /// shorten this, because the promise on the onboarding screen names four
    /// hours rather than "whatever is configured".
    /// </summary>
    internal DigestCadence(TimeSpan interval)
    {
        _interval = interval;
    }

    /// <summary>
    /// When the last digest was claimed, or null if none has been. Exposed
    /// for diagnostics; a count is never derived from it.
    /// </summary>
    public DateTimeOffset? LastClaimedAt => _lastClaimedAt;

    /// <summary>
    /// Claims the right to show one digest now, consuming the window if it
    /// grants it.
    /// </summary>
    /// <param name="pendingCount">
    /// Decisions owed, as the daemon reported them. Zero means silence: a
    /// notification saying nothing is worse than no notification.
    /// </param>
    /// <param name="now">The current instant, passed in so this is testable.</param>
    /// <returns>True at most once per <see cref="MinimumInterval"/>.</returns>
    public bool TryClaim(int pendingCount, DateTimeOffset now)
    {
        if (pendingCount <= 0)
        {
            // Not a claim, and deliberately not a stamp either. An empty
            // queue must not consume the window that a real digest arriving
            // a minute later would need.
            return false;
        }

        if (_lastClaimedAt is { } last && now - last < _interval)
        {
            return false;
        }

        // A clock that went backwards (a manual change, a resume from
        // hibernation with a bad RTC) lands here as "the interval has not
        // elapsed", because `now - last` is negative and therefore less than
        // the interval. That is the safe direction: it can only ever suppress
        // a notification, never add one.
        _lastClaimedAt = now;
        return true;
    }
}
