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
/// second copy of it, so this carries only what a header renders and what a
/// project-group submit needs to send.
/// </remarks>
public sealed class ProjectQueueGroup
{
    internal ProjectQueueGroup(string projectId, string projectLabel, int count)
    {
        ProjectId = projectId;
        ProjectLabel = projectLabel;
        Count = count;
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

    /// <summary>The display label shown on the group's header.</summary>
    public string ProjectLabel { get; }

    /// <summary>How many pending entries this project has.</summary>
    public int Count { get; }

    /// <summary>
    /// Whether the header offers a "Submit all" action.
    ///
    /// Only when there is more than one entry, matching the macOS shell's
    /// <c>ProjectQueueGroup</c>: a single-entry group's own row already has a
    /// <c>Submit</c> button that does exactly what a group action would, so a
    /// second control offering the identical decision would be noise, not a
    /// second choice.
    /// </summary>
    public bool ShowSubmitAll => Count > 1;
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

        foreach (QueueEntry entry in entries)
        {
            ArgumentNullException.ThrowIfNull(entry);
            string key = entry.ProjectId ?? string.Empty;

            if (!counts.ContainsKey(key))
            {
                order.Add(key);
                counts[key] = 0;
                labels[key] = LabelOf(entry);
            }

            counts[key]++;
        }

        return order
            .Select(key => new ProjectQueueGroup(key, labels[key], counts[key]))
            .ToList();
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
