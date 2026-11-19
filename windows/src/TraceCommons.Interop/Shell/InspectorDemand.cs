using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// What is asking for the inspector right now. Each field is one of the
/// Mac's demand sources (<c>InspectorDemand.keys</c>, MonitorShell.swift:47-58,
/// and <c>keys(model:traces:selection:)</c>, TracesInspectorHost.swift:172-187).
/// </summary>
/// <param name="ApprovalUndo">The entries an approval's undo window covers.</param>
/// <param name="ApprovedAt">
/// When that approval was made, so a second approval of the same session is a
/// new undo and a tick of its clock is not (TracesInspectorHost.swift:176-180).
/// </param>
/// <param name="Contributed">One trace's own Contribute, while it can be undone.</param>
/// <param name="Kept">The trace last kept.</param>
/// <param name="ContributedFolder">A folder's Submit all, while it can be undone.</param>
/// <param name="SubmittingFolder">The folder whose Submit all is in flight.</param>
/// <param name="Review">A selected session, whose card is the inspector's. A folder is not a demand.</param>
public sealed record DemandSources(
    IReadOnlyList<string>? ApprovalUndo = null,
    DateTimeOffset? ApprovedAt = null,
    string? Contributed = null,
    string? Kept = null,
    string? ContributedFolder = null,
    string? SubmittingFolder = null,
    string? Review = null);

/// <summary>
/// The rule that opens the inspector on demand (<c>InspectorDemand</c>,
/// MonitorShell.swift:43-66, with TracesInspectorHost.swift:172-208): the
/// inspector starts closed in a narrow window, but the things a person must
/// not miss open it when they appear. Something going away, or the same
/// thing still asking, leaves the person's choice: a demand never closes it.
/// </summary>
public static class InspectorDemand
{
    /// <summary>One key per thing asking for the inspector.</summary>
    public static IReadOnlySet<string> Keys(DemandSources sources)
    {
        var keys = new SortedSet<string>(StringComparer.Ordinal);
        if (sources.ApprovalUndo is { } undo)
        {
            keys.Add("undo:" + string.Join(",", undo));
        }

        if (sources.Contributed is { } contributed)
        {
            keys.Add("contributed:" + contributed);
        }

        if (sources.Kept is { } kept)
        {
            keys.Add("kept:" + kept);
        }

        if (sources.ContributedFolder is { } folder)
        {
            keys.Add("folder-undo:" + folder);
        }

        if (sources.SubmittingFolder is { } submitting)
        {
            keys.Add("folder-submit:" + submitting);
        }

        if (sources.ApprovalUndo is not null && sources.ApprovedAt is { } at)
        {
            keys.Add("undo:approval-at:" + (at.ToUnixTimeMilliseconds() / 1000.0).ToString("R", CultureInfo.InvariantCulture));
        }

        if (sources.Review is { } review)
        {
            keys.Add("review:" + review);
        }

        return keys;
    }

    /// <summary>
    /// The offers' keys, counted only where the inspector draws the prompts
    /// and may open (<c>offerKeys</c>, TracesInspectorHost.swift:193-199,
    /// gated as MonitorWindowView.swift:692-697 gates it, by
    /// <see cref="MonitorNavigation.PromptsInInspector"/> and
    /// <see cref="MonitorNavigation.InspectorAvailable"/>): beside the Traces
    /// tree an offer is already on screen, and on Home's own page the
    /// inspector cannot open, so an offer there is no new demand.
    /// </summary>
    public static IReadOnlySet<string> OfferKeys(MonitorLocation at, bool privateAIOffer, string? armingOfferProject)
    {
        var keys = new SortedSet<string>(StringComparer.Ordinal);
        if (!MonitorNavigation.PromptsInInspector(at.Tab) || !MonitorNavigation.InspectorAvailable(at.Tab, at.HomePage))
        {
            return keys;
        }

        if (privateAIOffer)
        {
            keys.Add("offer:private-ai");
        }

        if (armingOfferProject is { } project)
        {
            keys.Add("offer:arming:" + project);
        }

        return keys;
    }

    /// <summary>
    /// Every key the window feeds its tracker: the sources' and the offers'
    /// where they count (MonitorWindowView.swift:692-702).
    /// </summary>
    public static IReadOnlySet<string> WindowKeys(DemandSources sources, MonitorLocation at, bool privateAIOffer, string? armingOfferProject)
    {
        var keys = new SortedSet<string>(Keys(sources), StringComparer.Ordinal);
        keys.UnionWith(OfferKeys(at, privateAIOffer, armingOfferProject));
        return keys;
    }

    /// <summary>
    /// The inspector opens when something new asks for it
    /// (MonitorShell.swift:62-65).
    /// </summary>
    public static bool Opens(IReadOnlySet<string> previous, IReadOnlySet<string> current) =>
        current.Any(key => !previous.Contains(key));

    /// <summary>
    /// Whether the keys there when the window appears open the inspector: any
    /// at all (TracesInspectorHost.swift:206-208).
    /// </summary>
    public static bool OpensOnAppear(IReadOnlySet<string> keys) => keys.Count > 0;
}

/// <summary>
/// The window's side of <see cref="InspectorDemand"/>: it remembers the keys
/// last seen and answers whether a new set opens the inspector. Only ever
/// opens; the person's choice to close it is theirs.
/// </summary>
public sealed class InspectorDemandTracker
{
    private IReadOnlySet<string> _previous = new HashSet<string>();
    private bool _appeared;

    /// <summary>
    /// Feeds the current keys. Returns true when the inspector should open:
    /// on the first call, any key at all; afterwards, a key not seen last time.
    /// </summary>
    public bool Update(IReadOnlySet<string> current)
    {
        bool opens = _appeared ? InspectorDemand.Opens(_previous, current) : InspectorDemand.OpensOnAppear(current);
        _appeared = true;
        _previous = current;
        return opens;
    }
}
