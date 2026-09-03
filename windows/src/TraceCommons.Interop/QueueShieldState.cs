namespace TraceCommons.Interop;

/// <summary>
/// What the queue's nav item says about the state of what is waiting.
/// </summary>
public enum QueueShieldState
{
    /// <summary>Nothing waiting.</summary>
    Clear,

    /// <summary>Decisions owed, and nothing about them wants a second look.</summary>
    Waiting,

    /// <summary>
    /// Something waiting matched nothing, or was trimmed to fit. Either is
    /// worth slowing down on.
    /// </summary>
    Attention,
}

/// <summary>
/// Decides which <see cref="QueueShieldState"/> the rail draws.
/// </summary>
/// <remarks>
/// <para>
/// <b>Added to the numeric count, never replacing it.</b> The request was to
/// swap the count for an icon; that half is not adopted. At 149 waiting
/// sessions the count is the signal a contributor is actually reading, and an
/// icon that means "some" is a downgrade exactly at the scale that produced
/// the feedback. The shield adds the state the count could never carry.
/// </para>
/// <para>
/// Its own type rather than a member of <see cref="QueueShieldState"/>,
/// which is an enum and cannot hold one.
/// </para>
/// <para>
/// Pure logic, tested in this assembly, for the same reason
/// <see cref="QueueGrouping"/> is: a nav item quietly showing the wrong tone
/// is a bug nobody sees, and a red test today beats a screenshot review
/// someday.
/// </para>
/// </remarks>
public static class QueueShield
{
    /// <summary>
    /// The state for a queue holding <paramref name="waiting"/> sessions, of
    /// which <paramref name="nothingMatched"/> had no pattern fire and
    /// <paramref name="trimmed"/> were cut down to fit.
    /// </summary>
    /// <remarks>
    /// An empty queue is <see cref="QueueShieldState.Clear"/> whatever the
    /// flags say. A count left over from a session that has since been decided
    /// must not keep the rail in an attention tone that points at nothing: the
    /// contributor would have no way to act on it, and a warning with nothing
    /// behind it is what teaches people to ignore warnings.
    /// </remarks>
    public static QueueShieldState For(int waiting, int nothingMatched, int trimmed)
    {
        if (waiting <= 0)
        {
            return QueueShieldState.Clear;
        }

        return nothingMatched > 0 || trimmed > 0
            ? QueueShieldState.Attention
            : QueueShieldState.Waiting;
    }
}
