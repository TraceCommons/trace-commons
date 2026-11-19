using System;
using System.Threading.Tasks;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The window starts its daemon where the Mac does, at launch, so the
/// feature flags (and the Skills tab they gate) are read on Insights too;
/// and a later contribution start joins that start instead of racing it.
/// </summary>
public class DaemonStartTests
{
    private static TcException RootsNotDeclared() => new("roots not declared", TcException.RootsNotDeclaredLabel);

    [Fact]
    public async Task AStartUnderWayIsJoinedNotRaced()
    {
        var gate = new DaemonStartGate();
        bool running = false;
        int starts = 0;
        var first = new TaskCompletionSource();
        Task launch = gate.RunAsync(async () =>
        {
            starts++;
            await first.Task;
            running = true;
        }, () => running);
        Assert.True(gate.Starting);

        Task contribution = gate.RunAsync(() =>
        {
            starts++;
            return Task.CompletedTask;
        }, () => running);
        Assert.False(contribution.IsCompleted);
        first.SetResult();
        await launch;
        await contribution;
        Assert.Equal(1, starts);
        Assert.False(gate.Starting);
    }

    [Fact]
    public async Task AJoinedRefusalIsTheJoinersUnlessItBringsNewSettings()
    {
        var gate = new DaemonStartGate();
        bool running = false;
        int starts = 0;
        var refused = new TaskCompletionSource();
        Task launch = gate.RunAsync(async () =>
        {
            starts++;
            await refused.Task;
        }, () => running);
        Task contribution = gate.RunAsync(() =>
        {
            starts++;
            return Task.CompletedTask;
        }, () => running);
        refused.SetException(RootsNotDeclared());
        await Assert.ThrowsAsync<TcException>(() => launch);
        TcException joined = await Assert.ThrowsAsync<TcException>(() => contribution);
        Assert.True(joined.IsRootsNotDeclared); // The roots screen still opens from the explicit start.
        Assert.Equal(1, starts);

        // The roots screen's answer starts again with its settings.
        await gate.RunAsync(() =>
        {
            starts++;
            running = true;
            return Task.CompletedTask;
        }, () => running, retryAfterFailure: true);
        Assert.Equal(2, starts);
        Assert.True(running);
    }

    [Fact]
    public async Task ARunningDaemonIsNotStartedAgain()
    {
        var gate = new DaemonStartGate();
        int starts = 0;
        await gate.RunAsync(() =>
        {
            starts++;
            return Task.CompletedTask;
        }, () => true);
        Assert.Equal(0, starts);
    }

    [Fact]
    public async Task TheLaunchStartIsQuiet()
    {
        Assert.False(await LaunchDaemonStart.StartQuietlyAsync(() => Task.FromException(RootsNotDeclared())));
        Assert.False(await LaunchDaemonStart.StartQuietlyAsync(() => throw new TcException("another instance holds the lock", null)));
        Assert.True(await LaunchDaemonStart.StartQuietlyAsync(() => Task.CompletedTask));
    }
}
