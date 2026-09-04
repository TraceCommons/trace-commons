using System;
using System.Collections.Generic;
using System.Linq;

namespace TraceCommons.Interop;

/// <summary>
/// One project's slice of the queue: every pending entry that shares a
/// <c>project_id</c>, and what the "Submit all" group action needs to know
/// about them.
/// </summary>
/// <remarks>
/// Deliberately holds counts, not entries. The queue row list already owns
/// the entries themselves; a group is a view over the same data, not a
/// second copy of it, so this carries only what a folder row renders and what
/// a project-group submit needs to send.
///
/// Under the folder-first layout a row renders two more scalars -- the
/// project's display path and the total bytes waiting in it -- and they are
/// here for exactly the same reason the count is: they are facts about the
/// group, computed once in the grouping pass, not a second copy of the
/// entries.
/// </remarks>
public sealed class ProjectQueueGroup
{
    internal ProjectQueueGroup(
        string projectId,
        string projectLabel,
        string projectPath,
        int count,
        long sizeBytes)
    {
        ProjectId = projectId;
        ProjectLabel = projectLabel;
        ProjectPath = projectPath;
        Count = count;
        SizeBytes = sizeBytes;
    }

    /// <summary>
    /// The id every entry in this group shares -- the one <c>entry_value</c>
    /// publishes as <c>project_id</c>, and the one a group's "Submit all"
    /// must send. Never <see cref="ProjectLabel"/>, which is display text and
    /// not stable: two entries can carry the same label under different ids,
    /// or the same id under a label that changed, and grouping on the label
    /// would silently merge or split groups the daemon does not consider the
    /// same project.
    /// </summary>
    public string ProjectId { get; }

    /// <summary>The display label shown on the folder row.</summary>
    public string ProjectLabel { get; }

    /// <summary>
    /// The project's folder, <c>~</c>-abbreviated, taken from the first entry
    /// in the group. Empty against a daemon predating the field, in which
    /// case the row renders its label alone.
    /// </summary>
    /// <remarks>
    /// Every entry sharing a <see cref="ProjectId"/> shares a project key, so
    /// the first member's path is the group's path -- there is no
    /// disagreement to reconcile. It is display-only: renderable, never
    /// logged, audited, notified, or persisted to history. See
    /// <see cref="QueueEntry.ProjectPath"/>.
    /// </remarks>
    public string ProjectPath { get; }

    /// <summary>How many pending entries this project has.</summary>
    public int Count { get; }

    /// <summary>
    /// The session bytes on disk this folder is holding, summed over its
    /// entries.
    /// </summary>
    /// <remarks>
    /// Sessions on disk, never a would-send figure. That number only a
    /// preview computes, and stating one as the other on a consent surface
    /// is the app's first false statement about what leaves the machine --
    /// the same rule the card's own size line follows.
    /// </remarks>
    public long SizeBytes { get; }

    /// <summary>
    /// Whether the folder row offers a "Submit all" action. Always -- see the
    /// remark.
    /// </summary>
    /// <remarks>
    /// This used to be <c>Count &gt; 1</c>, on the reasoning that a
    /// single-entry group's own row already had a Submit doing exactly the
    /// same thing. That was true of a flat list where the row and the header
    /// were on screen together. Under the folder-first layout the row is a
    /// level down, so hiding this would mean opening a folder to do the thing
    /// the folder is offering. The rule expired with the layout it was
    /// written for; the property stays so callers do not have to know that.
    /// </remarks>
    public bool ShowSubmitAll => true;
}

/// <summary>
/// Buckets the pending queue by project, for the group headers and their
/// <c>Submit all</c> action.
///
/// This is pure logic -- no UI type, no I/O -- so it is testable here, on a
/// machine that cannot build WinUI at all. The bucketing rule (project_id,
/// never project_label), the group order, and whether a group's action
/// appears are exactly the decisions a rendering bug could get wrong
/// silently; keeping them here means a wrong bucket or a wrongly-shown
/// action is a red test today rather than a screenshot review someday.
/// </summary>
public static class QueueGrouping
{
    /// <summary>
    /// Groups <paramref name="entries"/> by <see cref="QueueEntry.ProjectId"/>.
    ///
    /// Group order is first-seen: the order the first entry of each project
    /// appears in <paramref name="entries"/>, which is also the order the
    /// queue itself is in. A contributor who has already scanned the list
    /// top to bottom should not see it reshuffle the moment it grows a
    /// header.
    ///
    /// An entry with no project id (<c>null</c> or empty) groups with every
    /// other entry that also has none, under an empty id -- it does not
    /// silently disappear from the queue, and it does not get a project id
    /// invented for it.
    /// </summary>
    public static IReadOnlyList<ProjectQueueGroup> ByProject(IEnumerable<QueueEntry> entries)
    {
        ArgumentNullException.ThrowIfNull(entries);

        var order = new List<string>();
        var counts = new Dictionary<string, int>(StringComparer.Ordinal);
        var labels = new Dictionary<string, string>(StringComparer.Ordinal);
        var paths = new Dictionary<string, string>(StringComparer.Ordinal);
        var bytes = new Dictionary<string, long>(StringComparer.Ordinal);

        foreach (QueueEntry entry in entries)
        {
            ArgumentNullException.ThrowIfNull(entry);
            string key = KeyOf(entry);

            if (!counts.ContainsKey(key))
            {
                order.Add(key);
                counts[key] = 0;
                labels[key] = LabelOf(entry);
                paths[key] = entry.ProjectPath;
                bytes[key] = 0;
            }

            counts[key]++;
            bytes[key] += entry.SizeBytes;
        }

        return order
            .Select(key => new ProjectQueueGroup(
                key,
                labels[key],
                paths[key],
                counts[key],
                bytes[key]))
            .ToList();
    }

    /// <summary>
    /// The bucketing key for one entry -- <see cref="QueueEntry.ProjectId"/>,
    /// empty-string if absent, never <see cref="QueueEntry.ProjectLabel"/>.
    /// Public so a caller reconstructing which rows belong to a
    /// <see cref="ProjectQueueGroup"/> (this method reports counts, not
    /// membership -- see <see cref="ProjectQueueGroup"/>'s remarks) uses
    /// exactly the same rule <see cref="ByProject"/> bucketed them with,
    /// rather than restating it and risking the two rules drifting apart.
    /// </summary>
    public static string KeyOf(QueueEntry entry)
    {
        ArgumentNullException.ThrowIfNull(entry);
        return entry.ProjectId ?? string.Empty;
    }

    /// <summary>
    /// The label a group's header shows: the project's own label, falling
    /// back to the id, falling back to a fixed placeholder. Never a path,
    /// which the daemon does not send and this must not invent. Matches
    /// <c>QueueEntryViewModel.ProjectLabel</c>'s fallback chain exactly --
    /// a row and its own group must never disagree about what to call the
    /// project it is in.
    /// </summary>
    private static string LabelOf(QueueEntry entry) =>
        !string.IsNullOrWhiteSpace(entry.ProjectLabel) ? entry.ProjectLabel!
        : !string.IsNullOrWhiteSpace(entry.ProjectId) ? entry.ProjectId!
        : "Unknown project";
}
