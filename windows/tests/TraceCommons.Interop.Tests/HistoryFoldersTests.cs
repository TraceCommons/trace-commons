using System.Collections.Generic;
using System.Linq;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// History takes the same folder-first shape as the queue, over the same
/// rule: group on the opaque project id, never on a display label.
/// </summary>
public class HistoryFoldersTests
{
    private static HistoryRecord Record(
        string submissionId,
        string projectId = "",
        string label = "") =>
        new()
        {
            SubmissionId = submissionId,
            ProjectId = projectId,
            ProjectLabel = label,
        };

    [Fact]
    public void RecordsGroupByProjectId()
    {
        IReadOnlyList<HistoryFolder> folders = HistoryFolders.Group(new[]
        {
            Record("1", projectId: "proj_a", label: "api"),
            Record("2", projectId: "proj_b", label: "web"),
            Record("3", projectId: "proj_a", label: "api"),
        });

        Assert.Equal(2, folders.Count);
        Assert.Equal(new[] { "proj_a", "proj_b" }, folders.Select(f => f.ProjectId));
        Assert.Equal(2, folders[0].Records.Count);
    }

    /// <summary>
    /// Two projects can carry one label. Grouping on it would merge them,
    /// which is exactly the mistake the id exists to prevent.
    /// </summary>
    [Fact]
    public void TwoProjectsSharingALabelStaySeparate()
    {
        IReadOnlyList<HistoryFolder> folders = HistoryFolders.Group(new[]
        {
            Record("1", projectId: "proj_a", label: "api"),
            Record("2", projectId: "proj_b", label: "api"),
        });

        Assert.Equal(2, folders.Count);
    }

    /// <summary>
    /// Records written before the id existed cannot be backfilled -- nothing
    /// retained the key they were minted from -- so they group by label, which
    /// is what they already did.
    /// </summary>
    [Fact]
    public void RecordsWithNoIdGroupByLabelInstead()
    {
        IReadOnlyList<HistoryFolder> folders = HistoryFolders.Group(new[]
        {
            Record("1", label: "api"),
            Record("2", label: "api"),
            Record("3", label: "web"),
        });

        Assert.Equal(2, folders.Count);
        Assert.Equal(2, folders[0].Records.Count);
        Assert.Equal("api", folders[0].ProjectLabel);
        Assert.Equal("", folders[0].ProjectId);
        Assert.False(HistoryFolders.IsResolvable(folders[0]));
    }

    /// <summary>
    /// Same label, one resolvable and one not. Claiming they are the same
    /// folder is a guess; two rows is the honest answer.
    /// </summary>
    [Fact]
    public void AnIdentifiedAndAnUnidentifiedRecordDoNotMerge()
    {
        IReadOnlyList<HistoryFolder> folders = HistoryFolders.Group(new[]
        {
            Record("1", projectId: "proj_a", label: "api"),
            Record("2", projectId: "", label: "api"),
        });

        Assert.Equal(2, folders.Count);
        Assert.True(HistoryFolders.IsResolvable(folders[0]));
        Assert.False(HistoryFolders.IsResolvable(folders[1]));
    }

    [Fact]
    public void AnEmptyHistoryProducesNoFolders()
        => Assert.Empty(HistoryFolders.Group(new List<HistoryRecord>()));

    /// <summary>
    /// A record with neither an id nor a label still gets a folder rather than
    /// disappearing: a contribution that happened has to remain visible.
    /// </summary>
    [Fact]
    public void ARecordWithNeitherIdNorLabelStillGetsAFolder()
    {
        IReadOnlyList<HistoryFolder> folders = HistoryFolders.Group(new[] { Record("1") });

        Assert.Equal("Unknown project", Assert.Single(folders).ProjectLabel);
    }
}
