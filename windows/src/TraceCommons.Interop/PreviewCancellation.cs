using System;
using System.Collections.Generic;

namespace TraceCommons.Interop;

/// <summary>
/// Decides which entries have left the queue for good, as opposed to merely
/// scrolling off screen -- the distinction the design spec draws for
/// <c>preview_cancel</c>: send it when a card is dismissed or leaves the list
/// for good, never on a scroll, because visibility already handles ordering
/// for anything still in the queue.
///
/// A queue is rebuilt wholesale on every refresh rather than diffed in place
/// (see <c>MainViewModel.ReplacePending</c>), so "left the list for good" is
/// exactly "was in the previous entry-id set and is not in the new one" --
/// dismissed, submitted, expired, or superseded all look the same from here,
/// and all of them are equally reasons to stop scheduling a preview for an
/// id nothing will ask about again.
/// </summary>
public static class PreviewCancellation
{
    /// <summary>
    /// The ids present in <paramref name="previousIds"/> but absent from
    /// <paramref name="currentIds"/>.
    /// </summary>
    public static IReadOnlyList<string> EntriesRemoved(
        IEnumerable<string> previousIds,
        IEnumerable<string> currentIds)
    {
        ArgumentNullException.ThrowIfNull(previousIds);
        ArgumentNullException.ThrowIfNull(currentIds);

        var stillPresent = new HashSet<string>(currentIds, StringComparer.Ordinal);
        var removed = new List<string>();

        foreach (string id in previousIds)
        {
            if (stillPresent.Add(id))
            {
                // Added successfully means it was NOT already in the current
                // set, so it dropped out. Using Add's return value instead of
                // Contains avoids a second lookup, and reusing the same set
                // as a visited-marker is safe because previousIds is not
                // guaranteed distinct and a duplicate must not be reported
                // twice.
                removed.Add(id);
            }
        }

        return removed;
    }
}
