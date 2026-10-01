using System;
using System.Threading.Tasks;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

public class CoalescingRefreshTests
{
    [Fact]
    public async Task AStatusEventDuringHistoryReadProducesOneFollowUpSnapshot()
    {
        var refresh = new CoalescingRefresh();
        var historyRead = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        int daemonDecisions = 1;
        int displayedDecisions = -1;
        int snapshots = 0;
        int active = 0;
        int maximumActive = 0;
        async Task ReadSnapshot()
        {
            maximumActive = Math.Max(maximumActive, ++active);
            snapshots++;
            displayedDecisions = daemonDecisions;
            if (snapshots == 1) await historyRead.Task;
            active--;
        }

        Task first = refresh.RunAsync(ReadSnapshot);
        Assert.Equal(1, displayedDecisions);
        daemonDecisions = 0;
        await refresh.RunAsync(ReadSnapshot);
        await refresh.RunAsync(ReadSnapshot);
        historyRead.SetResult();
        await first;

        Assert.Equal(0, displayedDecisions);
        Assert.Equal(2, snapshots);
        Assert.Equal(1, maximumActive);
        await refresh.RunAsync(ReadSnapshot);
        Assert.Equal(3, snapshots);
    }

    [Fact]
    public async Task AFailedSnapshotStillDrainsAnEventAndReleasesTheCoordinator()
    {
        var refresh = new CoalescingRefresh();
        var historyRead = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        int snapshots = 0;
        async Task ReadSnapshot()
        {
            snapshots++;
            if (snapshots == 1)
            {
                await historyRead.Task;
                throw new InvalidOperationException("history unavailable");
            }
        }

        Task first = refresh.RunAsync(ReadSnapshot);
        await refresh.RunAsync(ReadSnapshot);
        historyRead.SetResult();
        await Assert.ThrowsAsync<InvalidOperationException>(() => first);
        Assert.Equal(2, snapshots);
        await refresh.RunAsync(ReadSnapshot);
        Assert.Equal(3, snapshots);
    }
}
