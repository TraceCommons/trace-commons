using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The shield beside the queue's nav item. It is ADDED to the numeric count,
/// never a replacement for it: at 149 waiting sessions the count is the signal
/// a contributor actually reads.
/// </summary>
public class QueueShieldStateTests
{
    [Fact]
    public void AnEmptyQueueIsClear()
        => Assert.Equal(QueueShieldState.Clear, QueueShield.For(0, 0, 0));

    [Fact]
    public void AnOrdinaryQueueIsWaiting()
        => Assert.Equal(QueueShieldState.Waiting, QueueShield.For(12, 0, 0));

    /// <summary>
    /// A session where no pattern fired is the one worth slowing down on, and
    /// it is the case a count cannot state.
    /// </summary>
    [Fact]
    public void ANothingMatchedSessionRaisesAttention()
        => Assert.Equal(QueueShieldState.Attention, QueueShield.For(12, 1, 0));

    /// <summary>
    /// A conversation cut down to fit its byte budget is one the contributor
    /// has to be told about before consenting to it.
    /// </summary>
    [Fact]
    public void ATrimmedSessionRaisesAttention()
        => Assert.Equal(QueueShieldState.Attention, QueueShield.For(12, 0, 1));

    /// <summary>
    /// A flag left over from a session that has since been decided must not
    /// keep the rail warning about something the contributor cannot act on.
    /// </summary>
    [Fact]
    public void AnEmptyQueueIsClearEvenWithStaleFlags()
        => Assert.Equal(QueueShieldState.Clear, QueueShield.For(0, 3, 2));
}
