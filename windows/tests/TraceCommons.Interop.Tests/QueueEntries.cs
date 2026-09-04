using TraceCommons.Interop;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The queue-entry fixture the grouping and navigation suites share.
///
/// One helper rather than one per file: both suites bucket entries with
/// <see cref="QueueGrouping"/>, and two fixtures building an entry slightly
/// differently would eventually make the two suites disagree about what they
/// are testing.
/// </summary>
internal static class QueueEntries
{
    internal static QueueEntry Entry(
        string entryId,
        string? projectId = null,
        string? projectLabel = null,
        long bytes = 0,
        string path = "") =>
        new()
        {
            EntryId = entryId,
            ProjectId = projectId,
            ProjectLabel = projectLabel,
            SizeBytes = bytes,
            ProjectPath = path,
        };
}
