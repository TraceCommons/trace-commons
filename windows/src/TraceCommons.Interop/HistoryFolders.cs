using System;
using System.Collections.Generic;
using System.Linq;

namespace TraceCommons.Interop;

/// <summary>
/// One folder's worth of history: the records submitted from one project.
/// </summary>
/// <param name="Key">
/// The grouping key. Either a project id, or
/// <see cref="HistoryFolders.UnresolvedPrefix"/> followed by a label for a
/// record that carries no id.
/// </param>
/// <param name="ProjectId">
/// The opaque project id, or empty for an unidentified group. This is what a
/// folder path is resolved by, against the live <c>list_projects</c> rows.
/// </param>
/// <param name="ProjectLabel">The display name shown on the folder row.</param>
/// <param name="Records">The records in this folder, in the order given.</param>
public sealed record HistoryFolder(
    string Key,
    string ProjectId,
    string ProjectLabel,
    IReadOnlyList<HistoryRecord> Records);

/// <summary>
/// Groups history the way the queue is grouped, so the two screens navigate
/// identically.
/// </summary>
/// <remarks>
/// <para>
/// Keys on <see cref="HistoryRecord.ProjectId"/>. Grouping on the label is not
/// an option: a label is a display name, is not unique across two projects,
/// and grouping on it would merge them, which is the same mistake
/// <see cref="ProjectQueueGroup"/>'s own doc comment exists to forbid.
/// </para>
/// <para>
/// A record with no id falls back to <see cref="UnresolvedPrefix"/> plus its
/// label. A real id always starts with <c>proj_</c>, so the two key spaces
/// cannot collide, and <b>an unidentified record is never merged into an
/// identified one</b>: same label, one resolvable and one not, is a guess, and
/// two rows is the honest answer.
/// </para>
/// <para>
/// Records written before the id existed are not backfillable -- nothing
/// retained the key they were minted from -- and faking it is worse than
/// grouping them by label, which is what they already do today. History gets
/// folder grouping for everything submitted after the upgrade.
/// </para>
/// </remarks>
public static class HistoryFolders
{
    /// <summary>
    /// Marks a key derived from a label rather than an id.
    /// </summary>
    /// <remarks>
    /// A real project id always starts with <c>proj_</c>, so this prefix
    /// cannot collide with one. A caller rendering a folder row shows the
    /// label alone for one of these: no path can be resolved for a project the
    /// daemon cannot be asked about.
    /// </remarks>
    internal const string UnresolvedPrefix = "label:";

    /// <summary>
    /// Whether a folder's path can be looked up at all.
    /// </summary>
    /// <remarks>
    /// Public where <see cref="UnresolvedPrefix"/> is not, so a caller asks
    /// this question rather than re-deriving the prefix rule and risking the
    /// two drifting apart.
    /// </remarks>
    public static bool IsResolvable(HistoryFolder folder)
    {
        ArgumentNullException.ThrowIfNull(folder);
        return !folder.Key.StartsWith(UnresolvedPrefix, StringComparison.Ordinal);
    }

    /// <summary>
    /// Buckets <paramref name="records"/> into folders, in first-seen order.
    /// </summary>
    /// <remarks>
    /// First-seen order is the order history itself is in, so a list somebody
    /// has already scanned does not reshuffle the moment it grows folders.
    /// </remarks>
    public static IReadOnlyList<HistoryFolder> Group(IReadOnlyList<HistoryRecord> records)
    {
        ArgumentNullException.ThrowIfNull(records);

        var order = new List<string>();
        var byKey = new Dictionary<string, List<HistoryRecord>>(StringComparer.Ordinal);
        var labels = new Dictionary<string, string>(StringComparer.Ordinal);
        var ids = new Dictionary<string, string>(StringComparer.Ordinal);

        foreach (HistoryRecord record in records)
        {
            ArgumentNullException.ThrowIfNull(record);
            string key = KeyOf(record);

            if (!byKey.ContainsKey(key))
            {
                order.Add(key);
                byKey[key] = new List<HistoryRecord>();
                labels[key] = LabelOf(record);
                ids[key] = record.ProjectId;
            }

            byKey[key].Add(record);
        }

        return order
            .Select(key => new HistoryFolder(key, ids[key], labels[key], byKey[key]))
            .ToList();
    }

    /// <summary>
    /// The bucketing key for one record: its project id, or
    /// <see cref="UnresolvedPrefix"/> plus its label when it has none.
    /// </summary>
    public static string KeyOf(HistoryRecord record)
    {
        ArgumentNullException.ThrowIfNull(record);
        return record.ProjectId.Length > 0
            ? record.ProjectId
            : UnresolvedPrefix + record.ProjectLabel;
    }

    /// <summary>
    /// What a folder row calls the project: its label, falling back to the id,
    /// falling back to a fixed placeholder. Never a path, which history does
    /// not carry and this must not invent.
    /// </summary>
    private static string LabelOf(HistoryRecord record) =>
        !string.IsNullOrWhiteSpace(record.ProjectLabel) ? record.ProjectLabel
        : !string.IsNullOrWhiteSpace(record.ProjectId) ? record.ProjectId
        : "Unknown project";
}
