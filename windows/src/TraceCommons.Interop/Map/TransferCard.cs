using System;
using System.Globalization;

namespace TraceCommons.Interop.Map;

/// <summary>
/// What a transfer card shows (<c>TraceTransferCard</c>,
/// TraceTransferCard.swift): the tool, what is happening to the trace, its
/// folder and time, and behind Details the tool's memory files, which stay
/// on this computer, and the note. Every word is the core's
/// <c>flow_map</c> table's; the memory and keep lines are its "this
/// computer" variants, as on Linux.
/// </summary>
public sealed record TransferCardFace(
    string Title,
    string Tag,
    bool TagOn,
    string Eyebrow,
    string? Folder,
    string? Time,
    string Note,
    GlassToolArt? Mark,
    string? MemoryFiles,
    string? MemoryStays)
{
    /// <summary>The card's name for assistive technology: its title.</summary>
    public string AccessibleName => Title;

    /// <summary>
    /// The card for <paramref name="transfer"/>. <paramref name="fromMap"/>
    /// is the folder's name on the map, for a transfer that carries none.
    /// </summary>
    public static TransferCardFace For(
        MapTransfer transfer, FlowMapCopy words, string? fromMap, TimeZoneInfo? zone, CultureInfo? culture = null)
    {
        string? tool = transfer.MixedTools ? null : MapTools.DisplayName(transfer.Tool);
        (string tag, bool on, string note) = transfer.Action switch
        {
            // TraceTransferCard.swift:72-81.
            TransferAction.Contribute => (words.TransferContributing, true,
                tool is null ? words.TransferContributeNoteMixed : words.TransferNote(true, tool)),
            TransferAction.Keep => (words.TransferKeeping, false, words.TransferNote(false, string.Empty)),
            _ => (words.TransferDismissed, false, words.TransferDismissNote),
        };

        // A batch from several tools names none: its count is the title (:29-30).
        string title = tool ?? words.TransferTraces(transfer.Count);
        string eyebrow = (transfer.Count > 1 ? words.TransferTraces(transfer.Count) : words.TransferTrace)
            .ToUpper(culture ?? CultureInfo.CurrentCulture);
        string? folder = transfer.MixedFolders ? null : transfer.FolderLabel ?? fromMap ?? transfer.Folder;
        string? time = transfer.At is { } at ? FormatTime(at, zone ?? TimeZoneInfo.Local, culture ?? CultureInfo.CurrentCulture) : null;
        return new TransferCardFace(
            title, tag, on, eyebrow, folder, time, note,
            transfer.MixedTools ? null : MapTools.Art(transfer.Tool),
            transfer.MixedTools ? null : words.MemoryFiles(transfer.Tool),
            transfer.MixedTools ? null : words.MemoryStays);
    }

    /// <summary>
    /// <c>date: .abbreviated, time: .shortened</c> (:113): the month's short
    /// name, day and year, then the culture's short time.
    /// </summary>
    public static string FormatTime(DateTimeOffset at, TimeZoneInfo zone, CultureInfo culture)
    {
        DateTimeOffset local = TimeZoneInfo.ConvertTime(at, zone);
        return local.ToString("MMM d, yyyy", culture) + ", " + local.ToString("t", culture);
    }

    /// <summary>
    /// Whether the cards ride at the map's foot: only while a tab other than
    /// Traces is shown, and only on the Traces map (FlowMapView.swift:176,
    /// MonitorWindowView.swift:449). Traces stacks them with its notices.
    /// </summary>
    public static bool OnMap(bool tracesTabShown, bool mapShowsTraces, int cards) =>
        !tracesTabShown && mapShowsTraces && cards > 0;
}
