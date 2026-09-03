using System;
using System.Collections.Generic;
using System.Linq;

namespace TraceCommons.Interop;

/// <summary>
/// Where the queue is: on the folder list, or inside one folder.
/// </summary>
/// <remarks>
/// A closed pair of cases rather than a nullable project id, so a caller has
/// to say which of the two it means. "No project" and "the project whose id is
/// the empty string" are different states -- the queue really does group every
/// entry the daemon could not name under an empty id -- and a null would
/// conflate them.
/// </remarks>
public abstract record QueueLocation
{
    private QueueLocation()
    {
    }

    /// <summary>The folder list.</summary>
    public static QueueLocation Root { get; } = new RootLocation();

    /// <summary>One folder's sessions.</summary>
    /// <param name="ProjectId">
    /// The id every entry in the folder shares -- <see cref="ProjectQueueGroup.ProjectId"/>,
    /// never the display label. Two projects can carry the same label; only
    /// the id identifies one.
    /// </param>
    public sealed record Project(string ProjectId) : QueueLocation;

    private sealed record RootLocation : QueueLocation;
}

/// <summary>
/// Keeps the queue's location standing on ground that still exists.
/// </summary>
/// <remarks>
/// The queue is now two levels, and the second can be pulled out from under
/// the person standing on it: approving a folder's last session removes the
/// folder, and so does an upload finishing in the background. Standing in a
/// folder that has gone shows an empty pane with a back button and no
/// explanation.
///
/// This is pure logic -- no UI type, no I/O -- so it is testable on a machine
/// that cannot build WinUI at all, which is the same reason
/// <see cref="QueueGrouping"/> lives here.
/// </remarks>
public static class QueueNavigation
{
    /// <summary>
    /// The location to render, given where the view wants to be and what the
    /// queue actually holds.
    /// </summary>
    /// <remarks>
    /// A pure function of the location and the groups rather than a mutation,
    /// so a view can call it on every redraw and never hold a stale location.
    /// That is what makes a folder emptying underneath the contributor return
    /// them to the list rather than leaving them somewhere that is no longer
    /// there.
    /// </remarks>
    public static QueueLocation Resolve(
        QueueLocation location,
        IReadOnlyList<ProjectQueueGroup> groups)
    {
        ArgumentNullException.ThrowIfNull(groups);
        return Resolve(location, groups.Select(group => group.ProjectId).ToList());
    }

    /// <summary>
    /// The same decision over bare folder keys, for a screen whose folders are
    /// not queue groups.
    /// </summary>
    /// <remarks>
    /// History drills in the same way over <see cref="HistoryFolders"/>, and
    /// the two screens must navigate identically. Sharing the function rather
    /// than the type is what makes that true without history having to
    /// manufacture a <see cref="ProjectQueueGroup"/> it has no entries for.
    /// </remarks>
    public static QueueLocation Resolve(
        QueueLocation location,
        IReadOnlyList<string> folderKeys)
    {
        ArgumentNullException.ThrowIfNull(location);
        ArgumentNullException.ThrowIfNull(folderKeys);

        if (location is not QueueLocation.Project project)
        {
            return QueueLocation.Root;
        }

        return folderKeys.Any(key =>
            string.Equals(key, project.ProjectId, StringComparison.Ordinal))
            ? location
            : QueueLocation.Root;
    }
}
