using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// Turns "the scroll settled, here is what's on screen now" into the params
/// for one <c>preview_visible</c> call, or into nothing at all when the
/// on-screen set has not actually changed.
///
/// <c>preview_visible</c> is documented as cheap and idempotent -- safe to
/// call on every settle even when nothing changed. This exists anyway because
/// a shell should not lean on that guarantee to justify calling it needlessly:
/// deduping here is a few comparisons against a set the shell already
/// computed, and it is what makes "debounced, not per frame" a fact about the
/// wire rather than only about the timer that triggers the recompute.
///
/// Pure and stateful only in the sense of remembering the last set sent --
/// no daemon call, no UI type, testable on a machine that cannot build WinUI
/// at all.
/// </summary>
public sealed class PreviewVisibilityTracker
{
    private HashSet<string> _lastSent = new(StringComparer.Ordinal);
    private bool _hasSent;

    /// <summary>
    /// Records a fresh on-screen set and returns the <c>preview_visible</c>
    /// params to send, or null when the set is unchanged from the last call
    /// (or the initial call arrives with an empty set matching the initial,
    /// implicit "nothing is on screen yet" state).
    /// </summary>
    public string? OnSettled(IEnumerable<string> visibleEntryIds)
    {
        ArgumentNullException.ThrowIfNull(visibleEntryIds);

        var next = new HashSet<string>(visibleEntryIds, StringComparer.Ordinal);
        if (_hasSent && next.SetEquals(_lastSent))
        {
            return null;
        }

        _lastSent = next;
        _hasSent = true;

        // Sorted so the wire params are deterministic -- useful for tests and
        // for anyone diffing two calls by eye -- even though the daemon does
        // not care about order.
        string[] sorted = next.OrderBy(id => id, StringComparer.Ordinal).ToArray();
        return JsonSerializer.Serialize(new PreviewVisibleParams(sorted));
    }

    /// <summary>
    /// Forgets the last-sent set, so the next call sends regardless of
    /// whether it happens to match. Used after a resync, where the daemon's
    /// own idea of what is visible cannot be assumed to still match ours.
    /// </summary>
    public void Reset()
    {
        _lastSent = new HashSet<string>(StringComparer.Ordinal);
        _hasSent = false;
    }

    private sealed class PreviewVisibleParams
    {
        public PreviewVisibleParams(string[] entryIds) => EntryIds = entryIds;

        [JsonPropertyName("entry_ids")]
        public string[] EntryIds { get; }
    }
}
