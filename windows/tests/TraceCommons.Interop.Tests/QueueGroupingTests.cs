using System.Collections.Generic;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The queue's project grouping: the bucketing rule, group order, and when
/// "Submit all" appears. All three are decisions a rendering bug could get
/// wrong silently, which is exactly why they live here instead of in XAML --
/// this project compiles and its tests run on a machine that cannot build
/// WinUI at all.
/// </summary>
public sealed class QueueGroupingTests
{
    private static QueueEntry Entry(string entryId, string? projectId, string? projectLabel) =>
        new()
        {
            EntryId = entryId,
            ProjectId = projectId,
            ProjectLabel = projectLabel,
        };

    /// <summary>
    /// The bucketing key is project_id, never project_label. Two entries
    /// with the same label but different ids are two different projects to
    /// the daemon, and must stay two different groups here.
    /// </summary>
    [Fact]
    public void EntriesGroupByProjectIdNotLabel()
    {
        var entries = new List<QueueEntry>
        {
            Entry("e1", "proj_a", "Shared Label"),
            Entry("e2", "proj_b", "Shared Label"),
            Entry("e3", "proj_a", "Shared Label"),
        };

        IReadOnlyList<ProjectQueueGroup> groups = QueueGrouping.ByProject(entries);

        Assert.Equal(2, groups.Count);
        Assert.Equal(2, groups[0].Count);
        Assert.Equal("proj_a", groups[0].ProjectId);
        Assert.Equal(1, groups[1].Count);
        Assert.Equal("proj_b", groups[1].ProjectId);
    }

    /// <summary>
    /// Two entries sharing an id but disagreeing on label (a label that
    /// changed between them being queued) must still land in ONE group --
    /// the id is what the daemon considers the project, and a client that
    /// forked on label drift would silently split a project the daemon
    /// never split.
    /// </summary>
    [Fact]
    public void MismatchedLabelsUnderTheSameIdStillFormOneGroup()
    {
        var entries = new List<QueueEntry>
        {
            Entry("e1", "proj_a", "Old Name"),
            Entry("e2", "proj_a", "New Name"),
        };

        IReadOnlyList<ProjectQueueGroup> groups = QueueGrouping.ByProject(entries);

        Assert.Single(groups);
        Assert.Equal(2, groups[0].Count);
    }

    /// <summary>Groups appear in first-seen order, which is also the queue's own order.</summary>
    [Fact]
    public void GroupsAppearInFirstSeenOrder()
    {
        var entries = new List<QueueEntry>
        {
            Entry("e1", "proj_c", "C"),
            Entry("e2", "proj_a", "A"),
            Entry("e3", "proj_c", "C"),
            Entry("e4", "proj_b", "B"),
        };

        IReadOnlyList<ProjectQueueGroup> groups = QueueGrouping.ByProject(entries);

        Assert.Equal(new[] { "proj_c", "proj_a", "proj_b" }, new[]
        {
            groups[0].ProjectId, groups[1].ProjectId, groups[2].ProjectId,
        });
    }

    /// <summary>
    /// "Submit all" is offered only on a group with more than one entry,
    /// matching the macOS shell's rule: a single-entry group's own row
    /// already has a Submit button that does exactly what the group action
    /// would.
    /// </summary>
    [Fact]
    public void SubmitAllAppearsOnlyOnMultiEntryGroups()
    {
        var entries = new List<QueueEntry>
        {
            Entry("e1", "proj_solo", "Solo"),
            Entry("e2", "proj_pair", "Pair"),
            Entry("e3", "proj_pair", "Pair"),
        };

        IReadOnlyList<ProjectQueueGroup> groups = QueueGrouping.ByProject(entries);

        ProjectQueueGroup solo = Assert.Single(groups, g => g.ProjectId == "proj_solo");
        ProjectQueueGroup pair = Assert.Single(groups, g => g.ProjectId == "proj_pair");

        Assert.False(solo.ShowSubmitAll);
        Assert.True(pair.ShowSubmitAll);
    }

    /// <summary>
    /// A missing project id does not vanish from the queue: every entry with
    /// no id groups together under one placeholder group rather than being
    /// dropped, so a decision is still visibly owed for it.
    /// </summary>
    [Fact]
    public void EntriesWithNoProjectIdGroupTogetherRatherThanDisappearing()
    {
        var entries = new List<QueueEntry>
        {
            Entry("e1", null, null),
            Entry("e2", "", null),
        };

        IReadOnlyList<ProjectQueueGroup> groups = QueueGrouping.ByProject(entries);

        Assert.Single(groups);
        Assert.Equal(2, groups[0].Count);
        Assert.Equal("Unknown project", groups[0].ProjectLabel);
    }

    /// <summary>
    /// The label fallback chain: label, then id, then the fixed placeholder --
    /// the same chain QueueEntryViewModel's own ProjectLabel uses, so a row
    /// and the group it sits in never disagree about what to call the
    /// project.
    /// </summary>
    [Fact]
    public void LabelFallsBackToIdThenToAPlaceholder()
    {
        IReadOnlyList<ProjectQueueGroup> labelled =
            QueueGrouping.ByProject(new[] { Entry("e1", "proj_a", "Readable Name") });
        IReadOnlyList<ProjectQueueGroup> idOnly =
            QueueGrouping.ByProject(new[] { Entry("e2", "proj_b", null) });
        IReadOnlyList<ProjectQueueGroup> neither =
            QueueGrouping.ByProject(new[] { Entry("e3", null, null) });

        Assert.Equal("Readable Name", labelled[0].ProjectLabel);
        Assert.Equal("proj_b", idOnly[0].ProjectLabel);
        Assert.Equal("Unknown project", neither[0].ProjectLabel);
    }

    [Fact]
    public void AnEmptyQueueProducesNoGroups()
    {
        Assert.Empty(QueueGrouping.ByProject(new List<QueueEntry>()));
    }
}
