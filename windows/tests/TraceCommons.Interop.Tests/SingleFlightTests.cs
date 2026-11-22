using System;
using System.Threading.Tasks;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// A caller that arrives while the work runs joins it: the contribution
/// startup's old busy flag refused it, which dropped the click.
/// </summary>
public sealed class SingleFlightTests
{
    [Fact]
    public async Task ACallerDuringTheWorkJoinsItAndGetsItsResult()
    {
        var flight = new SingleFlight<bool>();
        var gate = new TaskCompletionSource<bool>();
        int runs = 0;
        Task<bool> first = flight.Run(() => { runs++; return gate.Task; });
        Task<bool> second = flight.Run(() => { runs++; return Task.FromResult(false); });

        Assert.True(flight.IsRunning);
        Assert.Same(first, second);
        Assert.False(second.IsCompleted, "joined, not refused with an immediate answer");
        gate.SetResult(true);
        Assert.True(await first);
        Assert.True(await second);
        Assert.Equal(1, runs);
        Assert.False(flight.IsRunning);
    }

    [Fact]
    public async Task AfterItFinishesTheNextCallRunsNewWork()
    {
        var flight = new SingleFlight<int>();
        int runs = 0;
        Assert.Equal(1, await flight.Run(async () => { await Task.Yield(); return ++runs; }));
        Assert.Equal(2, await flight.Run(async () => { await Task.Yield(); return ++runs; }));
    }

    [Fact]
    public async Task WorkThatFinishesSynchronouslyIsNeverKept()
    {
        var flight = new SingleFlight<int>();
        int runs = 0;
        Assert.Equal(1, await flight.Run(() => Task.FromResult(++runs)));
        Assert.False(flight.IsRunning);
        Assert.Equal(2, await flight.Run(() => Task.FromResult(++runs)));
    }

    [Fact]
    public async Task AFailureReachesEveryJoinerAndFreesTheNextCall()
    {
        var flight = new SingleFlight<int>();
        var gate = new TaskCompletionSource<int>();
        Task<int> first = flight.Run(() => gate.Task);
        Task<int> second = flight.Run(() => Task.FromResult(0));
        gate.SetException(new InvalidOperationException());

        await Assert.ThrowsAsync<InvalidOperationException>(() => first);
        await Assert.ThrowsAsync<InvalidOperationException>(() => second);
        Assert.False(flight.IsRunning);
        Assert.Equal(7, await flight.Run(() => Task.FromResult(7)));
    }

    [Fact]
    public async Task WorkThatThrowsBeforeItsFirstAwaitFreesTheNextCall()
    {
        var flight = new SingleFlight<int>();
        await Assert.ThrowsAsync<InvalidOperationException>(() => flight.Run(() => throw new InvalidOperationException()));
        Assert.False(flight.IsRunning);
        Assert.Equal(3, await flight.Run(() => Task.FromResult(3)));
    }
}
