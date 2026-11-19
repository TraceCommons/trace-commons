using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.Interop;
using Xunit;
using static TraceCommons.Interop.Tests.SkillsFixtures;

namespace TraceCommons.Interop.Tests;

/// <summary>macos/Tests/TraceCommonsAppTests/InsightsSkillsRecoveryTests.swift, against the Windows model.</summary>
public class InsightsSkillsRecoveryTests
{
    private sealed class FirstFailure
    {
        private readonly bool _unreadable;
        private readonly object _gate = new();
        private readonly List<bool> _requests = new();

        public FirstFailure(bool unreadable) => _unreadable = unreadable;

        public bool[] Requests
        {
            get { lock (_gate) return _requests.ToArray(); }
        }

        public Task<SkillsSnapshot> Read(bool refresh)
        {
            int count;
            lock (_gate)
            {
                _requests.Add(refresh);
                count = _requests.Count;
            }

            if (count == 1)
            {
                return _unreadable
                    ? Task.FromResult(Snapshot(enabled: true, readable: false))
                    : Task.FromException<SkillsSnapshot>(new InvalidOperationException("insights_skills not available yet"));
            }

            return Task.FromResult(Snapshot(enabled: true, followed: new[] { "recovered" }));
        }
    }

    [Fact]
    public Task FirstTransportFailureRecoversWithoutManualRefresh() => AssertFirstFailureRecovers(unreadable: false);

    [Fact]
    public Task FirstUnreadableReplyRecoversWithoutManualRefresh() => AssertFirstFailureRecovers(unreadable: true);

    [Fact]
    public async Task TransportFailureBacksOffDespiteStaleRefreshingSnapshot()
    {
        int reads = 0;
        var model = new InsightsSkillsModel(_ => ++reads == 1
            ? Task.FromResult(Snapshot(enabled: true, refreshing: true))
            : Task.FromException<SkillsSnapshot>(new InvalidOperationException("unreachable")));
        await model.LoadAsync();
        Assert.Equal(TimeSpan.FromSeconds(2), model.ObservationDelay(4));
        await model.LoadAsync();
        Assert.True(model.Shown?.Refreshing);
        Assert.NotNull(model.FailureKey);
        Assert.Equal(TimeSpan.FromSeconds(4), model.ObservationDelay(1));
        Assert.Equal(TimeSpan.FromSeconds(30), model.ObservationDelay(4));
    }

    [Fact]
    public void FailureBackoffIsBounded()
    {
        Assert.Equal(new[] { 2.0, 4, 8, 16, 30, 30 }, Enumerable.Range(0, 6).Select(a => InsightsSkillsModel.RetryDelay(a).TotalSeconds));
        Assert.Equal(TimeSpan.FromSeconds(30), InsightsSkillsModel.RetryDelay(int.MaxValue));
    }

    [Fact]
    public async Task CancellationStopsAnUnsuccessfulObserver()
    {
        var model = new InsightsSkillsModel(_ => Task.FromException<SkillsSnapshot>(new InvalidOperationException("unreachable")));
        using var cancel = new CancellationTokenSource();
        Task observer = model.ObserveAsync(cancel.Token);
        for (int i = 0; i < 100 && model.FailureKey is null; i++) await Task.Delay(1);
        Assert.NotNull(model.FailureKey);
        cancel.Cancel();
        await observer;
        Assert.False(model.Busy);
        Assert.Null(model.Shown);
    }

    private static async Task AssertFirstFailureRecovers(bool unreadable)
    {
        var replies = new FirstFailure(unreadable);
        var model = new InsightsSkillsModel(replies.Read, idlePollInterval: TimeSpan.FromSeconds(60));
        using var cancel = new CancellationTokenSource();
        Task observer = model.ObserveAsync(cancel.Token);
        var clock = Stopwatch.StartNew();
        while (model.Shown is null && clock.Elapsed < TimeSpan.FromSeconds(6)) await Task.Delay(10);
        // Success returns to the normal poll, not the error loop.
        await Task.Delay(50);
        cancel.Cancel();
        await observer;
        Assert.Equal(new[] { "recovered" }, model.Shown?.FollowedPublishers);
        Assert.Null(model.FailureKey);
        Assert.Equal(new[] { false, false }, replies.Requests); // Recovery must not force publisher refreshes.
        int count = replies.Requests.Length;
        await Task.Delay(30);
        Assert.Equal(count, replies.Requests.Length);
    }
}
