using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;

namespace TraceCommons.Interop.Map;

/// <summary>
/// Which Traces tree rows to build again. Each row's key holds every value
/// it draws, by value: the map's read makes new objects every time, and a
/// row whose values did not change is kept as it is. A folder's Automatic
/// press then builds one row, not the whole tree (owner, 2026-11-20, from
/// the profiling in #2278: each press rebuilt every row).
/// </summary>
public static class TreeRowKeys
{
    /// <summary>What a folder row draws from the queue's group: the Submit offer and what it leaves behind.</summary>
    public sealed record GroupFacts(bool ShowSubmitAll, int OfferedCount, string? Withheld);

    /// <summary>What a trace row draws from the queue's own row.</summary>
    public sealed record LiveFacts(
        long SizeBytes, string? ReasonCode, bool CanContribute, string? EligibilityText, bool WasTrimmed,
        string? AttestationText, bool AttestationClear, bool AttestationAttention, bool MaySubmit, bool IsSubmitting = false);

    private const char Unit = '\u001f';

    /// <summary>A folder row's key.</summary>
    public static string Folder(
        TracesTree.FolderNode folder, bool selected, bool open, GroupFacts? group, bool writing, string? refusal, double? dragOffset)
    {
        var key = new StringBuilder("f");
        Add(key, folder.Id, folder.Label, folder.Mode, folder.Disclosure, folder.Path);
        Add(key, folder.IsBucket, folder.PendingCount, folder.ContributableCount);
        Add(key, folder.OfferableModes.Count);
        foreach (string mode in folder.OfferableModes)
        {
            Add(key, mode);
        }

        Add(key, folder.Sessions.Count);
        foreach (MapEntry entry in folder.Sessions)
        {
            Entry(key, entry);
        }

        Add(key, selected, open, writing, refusal, dragOffset);
        if (group is null)
        {
            Add(key, "-");
        }
        else
        {
            Add(key, group.ShowSubmitAll, group.OfferedCount, group.Withheld);
        }

        return key.ToString();
    }

    /// <summary>A trace row's key.</summary>
    public static string Session(MapEntry entry, bool off, bool selected, LiveFacts? live)
    {
        var key = new StringBuilder("s");
        Entry(key, entry);
        Add(key, off, selected);
        if (live is null)
        {
            Add(key, "-");
        }
        else
        {
            Add(key, live.SizeBytes, live.ReasonCode, live.CanContribute, live.EligibilityText, live.WasTrimmed);
            Add(key, live.AttestationText, live.AttestationClear, live.AttestationAttention, live.MaySubmit, live.IsSubmitting);
        }

        return key.ToString();
    }

    /// <summary>
    /// The rows to build: those not drawn, those whose key changed, and
    /// <paramref name="force"/> when drawn (a switch moved that wrote
    /// nothing, so its values are unchanged but its control is not). The rest
    /// stay where they are when the rows' order is unchanged
    /// (<see cref="TreeRowPlan.InPlace"/>); otherwise the list is laid again,
    /// with the kept rows reused.
    /// </summary>
    public static TreeRowPlan Plan(
        IReadOnlyList<string> drawnIds, IReadOnlyDictionary<string, string> drawnKeys,
        IReadOnlyList<(string Id, string Key)> rows, string? force = null)
    {
        var build = new List<string>();
        foreach ((string id, string key) in rows)
        {
            if (!drawnKeys.TryGetValue(id, out string? drawn) || drawn != key || id == force)
            {
                build.Add(id);
            }
        }

        bool inPlace = drawnIds.Count == rows.Count;
        for (int i = 0; inPlace && i < rows.Count; i++)
        {
            inPlace = string.Equals(drawnIds[i], rows[i].Id, StringComparison.Ordinal);
        }

        return new TreeRowPlan(build, inPlace);
    }

    private static void Entry(StringBuilder key, MapEntry e)
    {
        Add(key, e.EntryId, e.Source, e.DeclaredSource, e.ProjectId, e.ProjectLabel, e.Title);
        Add(key, e.StartedAt?.UtcTicks, e.DiscoveredAt?.UtcTicks, e.StartedAt?.Offset.Ticks, e.DiscoveredAt?.Offset.Ticks);
    }

    private static void Add(StringBuilder key, params object?[] values)
    {
        foreach (object? value in values)
        {
            key.Append(Unit);
            switch (value)
            {
                case null:
                    key.Append('\u0000');
                    break;
                case string s:
                    // The length first, so no value can run into the next.
                    key.Append(s.Length.ToString(CultureInfo.InvariantCulture)).Append(':').Append(s);
                    break;
                case IFormattable f:
                    key.Append(f.ToString(null, CultureInfo.InvariantCulture));
                    break;
                default:
                    key.Append(value);
                    break;
            }
        }
    }
}

/// <summary>The rows to build, and whether every other row stays in its place.</summary>
public sealed record TreeRowPlan(IReadOnlyList<string> Build, bool InPlace);
