using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using TraceCommons.Interop;
using Xunit;
using static TraceCommons.Interop.Tests.SkillsFixtures;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// macos/Tests/TraceCommonsAppTests/InsightsSkillsFollowTests.swift, against
/// the Windows model. Four of its eight tests were ported with the first
/// Skills tests (InsightsSkillsTests.cs: the transport failure, the refresh
/// error beside a saved follow, the unconfirmed reply and the pending
/// creator); the rest are here. The Mac's cancelled-reply test has no
/// Windows counterpart: <see cref="InsightsSkillsModel.SetFollowAsync"/>
/// takes no cancellation, so a press always runs to the daemon's reply.
/// </summary>
public class InsightsSkillsFollowTests
{
    /// <summary>The Mac's <c>FollowReplyGate</c>: only the daemon's reply is held back.</summary>
    private sealed class FollowReplyGate
    {
        private TaskCompletionSource<SkillsSnapshot>? _reply;
        public List<string> Publishers { get; } = new();
        public bool Started => _reply is not null;

        public Task<SkillsSnapshot> Write(string publisher, bool followed)
        {
            Publishers.Add(publisher);
            _reply = new TaskCompletionSource<SkillsSnapshot>(TaskCreationOptions.RunContinuationsAsynchronously);
            return _reply.Task;
        }

        public void Finish(SkillsSnapshot snapshot)
        {
            TaskCompletionSource<SkillsSnapshot>? reply = _reply;
            _reply = null;
            reply?.SetResult(snapshot);
        }
    }

    [Fact]
    public async Task PendingBelongsToOneCreatorAndConfirmedStateWaitsForReply()
    {
        var replies = new FollowReplyGate();
        var model = new InsightsSkillsModel(_ => Task.FromResult(Snapshot(enabled: true, followed: new[] { "old" })), replies.Write);
        await model.LoadAsync();
        Task operation = model.SetFollowAsync("new", true);
        Assert.True(replies.Started);
        Assert.Equal("new", model.PendingFollowPublisherId);
        Assert.False(model.CanFollow);
        Assert.Equal(new[] { "old" }, model.Shown?.FollowedPublishers); // Pending is never a saved follow.
        await model.SetFollowAsync("other", true);
        await model.LoadAsync(refresh: true);
        replies.Finish(Snapshot(enabled: true, followed: new[] { "old", "new" }));
        await operation;
        Assert.Equal(new[] { "old", "new" }, model.Shown?.FollowedPublishers);
        Assert.Null(model.PendingFollowPublisherId);
        Assert.Null(model.FollowFailurePublisherId);
        Assert.True(model.CanFollow);
        Assert.Equal(new[] { "new" }, replies.Publishers); // Reads and writes share a serial lane.
    }

    [Fact]
    public async Task RetryClearsOnlyFailureAfterDaemonConfirmation()
    {
        var replies = new FollowReplyGate();
        var model = new InsightsSkillsModel(_ => Task.FromResult(Snapshot(enabled: true)), replies.Write);
        await model.LoadAsync();
        Task failed = model.SetFollowAsync("new", true);
        Assert.True(replies.Started);
        replies.Finish(Snapshot(enabled: true, error: "failed"));
        await failed;
        Assert.Equal("new", model.FollowFailurePublisherId);
        Task retry = model.SetFollowAsync("new", true);
        Assert.True(replies.Started);
        Assert.Null(model.FollowFailurePublisherId);
        Assert.Equal("new", model.PendingFollowPublisherId);
        replies.Finish(Snapshot(enabled: true, followed: new[] { "new" }));
        await retry;
        Assert.Equal(new[] { "new" }, model.Shown?.FollowedPublishers);
        Assert.Null(model.FollowFailurePublisherId);
        Assert.Null(model.FailureKey);
    }

    [Fact]
    public async Task UnavailableReplyRevokesFollowControls()
    {
        foreach (SkillsSnapshot reply in new[] { Snapshot(enabled: true, readable: false), Snapshot(enabled: false) })
        {
            var model = new InsightsSkillsModel(_ => Task.FromResult(Snapshot(enabled: true, followed: new[] { "old" })),
                (_, _) => Task.FromResult(reply));
            await model.LoadAsync();
            Assert.True(model.CanFollow);
            await model.SetFollowAsync("new", true);
            Assert.False(model.CanFollow);
            Assert.Null(model.PendingFollowPublisherId);
            Assert.True(model.Shown is null || !model.Shown.Enabled, "the old display stays after access is lost");
        }
    }

    [Fact]
    public async Task DisabledAndMissingSnapshotsRejectFollowWrites()
    {
        foreach (SkillsSnapshot snapshot in new[] { Snapshot(enabled: false), Snapshot(enabled: true, readable: false) })
        {
            int writes = 0;
            var model = new InsightsSkillsModel(_ => Task.FromResult(snapshot), (_, _) =>
            {
                writes++;
                return Task.FromResult(Snapshot(enabled: true));
            });
            await model.LoadAsync();
            Assert.False(model.CanFollow);
            await model.SetFollowAsync("new", true);
            Assert.Null(model.PendingFollowPublisherId);
            Assert.Equal(0, writes); // Unavailable Skills must not send a follow write.
        }

        int unloadedWrites = 0;
        var unloaded = new InsightsSkillsModel(writer: (_, _) =>
        {
            unloadedWrites++;
            return Task.FromResult(Snapshot(enabled: true));
        });
        Assert.False(unloaded.CanFollow);
        await unloaded.SetFollowAsync("new", true);
        Assert.Equal(0, unloadedWrites); // A confirmed snapshot is required before following.
    }
}
