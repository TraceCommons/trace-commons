using System;
using System.Collections.Generic;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The queue is now two levels, and the second can be pulled out from under
/// the person standing on it: approving a folder's last session removes the
/// folder, and so does an upload finishing in the background.
/// </summary>
public class QueueNavigationTests
{
    private static IReadOnlyList<ProjectQueueGroup> Groups(params string[] projectIds)
        => QueueGrouping.ByProject(
            Array.ConvertAll(
                projectIds,
                id => QueueEntries.Entry("e-" + id, id, id)));

    [Fact]
    public void RootStaysRoot()
    {
        Assert.Equal(
            QueueLocation.Root,
            QueueNavigation.Resolve(QueueLocation.Root, Groups("a")));
        Assert.Equal(
            QueueLocation.Root,
            QueueNavigation.Resolve(QueueLocation.Root, Groups()));
    }

    [Fact]
    public void AProjectThatStillExistsIsKept()
        => Assert.Equal(
            new QueueLocation.Project("a"),
            QueueNavigation.Resolve(new QueueLocation.Project("a"), Groups("a", "b")));

    /// <summary>
    /// Submit all inside a folder: the folder goes, and standing in it would
    /// show an empty pane with a back button and no explanation.
    /// </summary>
    [Fact]
    public void AProjectThatEmptiedFallsBackToRoot()
        => Assert.Equal(
            QueueLocation.Root,
            QueueNavigation.Resolve(new QueueLocation.Project("a"), Groups("b")));

    [Fact]
    public void TheLastProjectEmptyingFallsBackToRoot()
        => Assert.Equal(
            QueueLocation.Root,
            QueueNavigation.Resolve(new QueueLocation.Project("a"), Groups()));

    [Fact]
    public void ResolutionIsByIdNotLabel()
    {
        // Two projects can share a label; only the id identifies one.
        IReadOnlyList<ProjectQueueGroup> groups = QueueGrouping.ByProject(new[]
        {
            QueueEntries.Entry("e1", "proj_1", "api"),
            QueueEntries.Entry("e2", "proj_2", "api"),
        });

        Assert.Equal(
            new QueueLocation.Project("proj_2"),
            QueueNavigation.Resolve(new QueueLocation.Project("proj_2"), groups));
        Assert.Equal(
            QueueLocation.Root,
            QueueNavigation.Resolve(new QueueLocation.Project("proj_3"), groups));
    }

    /// <summary>
    /// The bucket the daemon could not name is a real folder with a real
    /// (empty) id, and it must be enterable like any other -- which is why
    /// the location is a closed pair of cases rather than a nullable id.
    /// </summary>
    [Fact]
    public void TheUnnamedProjectBucketIsAFolderLikeAnyOther()
    {
        IReadOnlyList<ProjectQueueGroup> groups =
            QueueGrouping.ByProject(new[] { QueueEntries.Entry("e1") });

        Assert.Equal(
            new QueueLocation.Project(""),
            QueueNavigation.Resolve(new QueueLocation.Project(""), groups));
    }

    /// <summary>
    /// History drills in over the same function, so the two screens cannot
    /// come to disagree about what happens when a folder goes.
    /// </summary>
    [Fact]
    public void BareFolderKeysResolveTheSameWay()
    {
        Assert.Equal(
            new QueueLocation.Project("label:api"),
            QueueNavigation.Resolve(
                new QueueLocation.Project("label:api"),
                new[] { "proj_a", "label:api" }));
        Assert.Equal(
            QueueLocation.Root,
            QueueNavigation.Resolve(
                new QueueLocation.Project("label:api"),
                new[] { "proj_a" }));
    }
}
