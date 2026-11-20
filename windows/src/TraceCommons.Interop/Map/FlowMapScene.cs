using System;
using System.Collections.Generic;
using System.Linq;

namespace TraceCommons.Interop.Map;

/// <summary>A point in the map's design space.</summary>
public readonly record struct MapPoint(double X, double Y);

/// <summary>A size in the map's design space.</summary>
public readonly record struct MapSize(double Width, double Height);

/// <summary>What a map node is (FlowMapScene.swift:13-30).</summary>
public enum MapNodeKind
{
    Hub,
    Library,
    Tool,
    Folder,
    Harness,
    Destination,
    Device,
}

/// <summary>The Private AI destination's lamp (FlowMapScene.swift:35-42).</summary>
public enum MapLamp
{
    Answering,
    Running,
    Off,
    Unknown,
}

/// <summary>An arc's look (FlowMapScene.swift:117-130).</summary>
public enum MapArcStyle
{
    Quiet,
    Dashed,
    Flowing,
    Shared,
    Unknown,
}

/// <summary>One node (FlowMapScene.swift:12-77).</summary>
public sealed record MapNode(string Id, MapNodeKind Kind, MapPoint At, double Radius, string Label, string Detail)
{
    public double Dim { get; init; } = 1;

    public string? Title { get; init; }

    public bool Dashed { get; init; }

    public bool Ringed { get; init; }

    public string? Sublabel { get; init; }

    public int? Waiting { get; init; }

    /// <summary>A library's lit state.</summary>
    public bool Active { get; init; }

    /// <summary>A tool's crossed-out state.</summary>
    public bool Off { get; init; }

    /// <summary>A tool's or harness's art.</summary>
    public GlassToolArt? Tool { get; init; }

    /// <summary>A folder's wire mode, null when the core has not listed it.</summary>
    public string? Mode { get; init; }

    /// <summary>A harness's installed state.</summary>
    public bool Installed { get; init; }

    /// <summary>A destination's lamp.</summary>
    public MapLamp Lamp { get; init; }

    /// <summary>Folders and Private AI tools: the label sits left of the node (:69-74).</summary>
    public bool LabelBeside => Kind is MapNodeKind.Folder or MapNodeKind.Harness;

    public string CardTitle => Title ?? Label;
}

/// <summary>One arc, a cubic from <see cref="From"/> to <see cref="To"/> (FlowMapScene.swift:116-138).</summary>
public sealed record MapArc(MapPoint From, MapPoint Control1, MapPoint Control2, MapPoint To, MapArcStyle Style, double Dim = 1)
{
    /// <summary>The point <paramref name="t"/> of the way along (MapToolPile.swift:315-327).</summary>
    public MapPoint PointAt(double t)
    {
        double v = t, u = 1 - v;
        double a = u * u * u, b = 3 * u * u * v, c = 3 * u * v * v, d = v * v * v;
        return new MapPoint(
            a * From.X + b * Control1.X + c * Control2.X + d * To.X,
            a * From.Y + b * Control1.Y + c * Control2.Y + d * To.Y);
    }
}

/// <summary>A "+N" count of what is not drawn (FlowMapScene.swift:141-144).</summary>
public sealed record MapOverflow(MapPoint At, int Count);

/// <summary>The folder column's pill (FlowMapScene.swift:149-165).</summary>
public sealed record MapFolderReveal(string Tool, string ToolName, MapPoint At, int Hidden, double Dim = 1)
{
    public int Next => Math.Min(Hidden, FlowMapScene.FoldersRevealed);
}

/// <summary>The tool group's card and pile (FlowMapScene.swift:290-315).</summary>
public sealed record MapToolStack(MapPoint Center, IReadOnlyList<string> Tools)
{
    public const double Coin = 46;
    public const double Width = 66;
    public const double OpenWidth = 172;
    public const double Step = 9;
    public const double OpenStep = 34;
    public const double Padding = 11;

    public MapPoint PortLeft => new(Center.X - Width / 2, Center.Y);

    public MapPoint PortRight => new(Center.X + Width / 2, Center.Y);

    public double Height(bool open) => Math.Max(Tools.Count - 1, 0) * (open ? OpenStep : Step) + Coin + 2 * Padding;

    public MapPoint CoinCenter(int index, bool open)
    {
        double step = open ? OpenStep : Step;
        return new MapPoint(Center.X, Center.Y + (index - (Tools.Count - 1) / 2.0) * step);
    }
}

/// <summary>Another device on the account (FlowMapScene.swift:527-534).</summary>
public sealed record MapOtherDevice(string Id, string Name, string Detail, int? Contributed = null);

/// <summary>
/// What decides whether traces move to the commons; anything unread is closed
/// (FlowMapScene.swift:339-368).
/// </summary>
public sealed record CommonsGate(bool Ready, MapStatus? Status, MapDestinations? Destinations)
{
    /// <summary>The <c>sessions_route</c> values with somewhere to send (:360).</summary>
    public static readonly IReadOnlySet<string> Routes = new HashSet<string>(StringComparer.Ordinal) { "witness", "local" };

    public bool Open =>
        Ready
        && Status is { } s && Destinations is { } d
        && s.Paused == false && s.LoggedIn == true
        
        && s.HeldSessions == 0
        && s.GrantVoids == 0
        && d.SessionsRoute is { } route && Routes.Contains(route);

    public bool SendsToCommons(string kind) =>
        Destinations?.Tools.FirstOrDefault(t => t.Tool == kind) is { Watch: "watched" } row && row.SessionsTo.Contains("commons");
}

/// <summary>
/// What has been contributed from this machine, from History (FlowMapScene.swift:85-114).
/// Null counts while History is unread, failed or capped.
/// </summary>
public sealed record MapContributions(int? Total, IReadOnlyDictionary<string, int> ByTool, IReadOnlyDictionary<string, int> ByFolder)
{
    public static readonly MapContributions Unread = new(null, new Dictionary<string, int>(), new Dictionary<string, int>());

    /// <summary>The page the counts take (<c>HomeStore.historyLimit</c>, HomeStore.swift:50).</summary>
    public const int HistoryLimit = 100;

    /// <summary>
    /// History statuses that left and still stand
    /// (<c>TracesGraphModel.contributedStatuses</c>, MonitorShell.swift:122).
    /// </summary>
    public static readonly IReadOnlySet<string> Statuses = new HashSet<string>(StringComparer.Ordinal) { "accepted", "submitted", "quarantined" };

    /// <summary>FlowMapScene.swift:98-110 with SummaryInspector.swift:298-308.</summary>
    public static MapContributions From(IReadOnlyList<MapHistoryRow>? history, bool failed = false)
    {
        if (failed || history is null || history.Count >= HistoryLimit)
        {
            return Unread;
        }

        var byTool = new Dictionary<string, int>(StringComparer.Ordinal);
        var byFolder = new Dictionary<string, int>(StringComparer.Ordinal);
        int total = 0;
        foreach (MapHistoryRow row in history)
        {
            if (!Statuses.Contains(row.Status ?? string.Empty))
            {
                continue;
            }

            total++;
            if (row.Source is { } tool && MapTools.Known(tool))
            {
                byTool[tool] = byTool.TryGetValue(tool, out int n) ? n + 1 : 1;
            }

            if (row.ProjectId is { } folder)
            {
                byFolder[folder] = byFolder.TryGetValue(folder, out int n) ? n + 1 : 1;
            }
        }

        return new MapContributions(total, byTool, byFolder);
    }

    public int? Tool(string kind) => Total is null ? null : ByTool.TryGetValue(kind, out int n) ? n : 0;

    public int? Folder(string id) => Total is null ? null : ByFolder.TryGetValue(id, out int n) ? n : 0;
}

/// <summary>
/// What the flow map draws, a port of the Mac's <c>FlowMapScene</c>
/// (macos/Sources/TraceCommonsApp/Views/Monitor/FlowMapScene.swift): nodes,
/// arcs and what each node says, in a design space of #1146's 580 by 760 or
/// one shaped like the pane. Pure data, so it is tested without drawing.
/// </summary>
public sealed record FlowMapScene
{
    public IReadOnlyList<MapNode> Nodes { get; init; } = Array.Empty<MapNode>();

    public IReadOnlyList<MapArc> Arcs { get; init; } = Array.Empty<MapArc>();

    public IReadOnlyList<MapOverflow> Overflows { get; init; } = Array.Empty<MapOverflow>();

    public IReadOnlyList<MapFolderReveal> Reveals { get; init; } = Array.Empty<MapFolderReveal>();

    /// <summary>The Traces map's tool group; null on the Private AI map.</summary>
    public MapToolStack? Stack { get; init; }

    public MapPoint? AddDevice { get; init; }

    public MapSize Space { get; init; } = Size;

    /// <summary>#1146's design space (:181).</summary>
    public static readonly MapSize Size = new(580, 760);

    /// <summary>An empty scene: a canvas starts in its own empty state, never another view's.</summary>
    public static readonly FlowMapScene Empty = new();

    /// <summary>Anything moving (:221).</summary>
    public bool Flows => Arcs.Any(a => a.Style == MapArcStyle.Flowing);

    /// <summary>Whether there is anything to draw (:739).</summary>
    public bool DrawsMap => Nodes.Count > 0;

    /// <summary>A design space with the pane's proportions and #1146's area (:187-192).</summary>
    public static MapSize SpaceFor(MapSize pane)
    {
        if (pane.Width <= 0 || pane.Height <= 0)
        {
            return Size;
        }

        double aspect = Math.Min(Math.Max(pane.Width / pane.Height, 0.42), 1.7);
        double area = Size.Width * Size.Height;
        return new MapSize(Math.Sqrt(area * aspect), Math.Sqrt(area / aspect));
    }

    /// <summary>The band nodes are laid out in (:196-198).</summary>
    public static (double Lower, double Upper) Band(MapSize space) => (space.Height * 0.13, space.Height * 0.87);

    /// <summary>Positions for <paramref name="weights"/> stacked down the band (:204-218).</summary>
    public static IReadOnlyList<(double Y, double Room)> StackDown(
        IReadOnlyList<double> weights, (double Lower, double Upper) band, double maxStep, double minStep = 0)
    {
        if (weights.Count == 0)
        {
            return Array.Empty<(double, double)>();
        }

        double units = weights.Sum();
        double length = band.Upper - band.Lower;
        double step = Math.Max(minStep, Math.Min(maxStep, length / Math.Max(units, 1)));
        double y = (band.Lower + band.Upper) / 2 - step * units / 2;
        var slots = new List<(double, double)>();
        foreach (double weight in weights)
        {
            double room = step * weight;
            slots.Add((y + room / 2, room));
            y += room;
        }

        return slots;
    }

    // Traces (:225-484).

    public static double FolderX(MapSize space) => Math.Max(124, space.Width * 0.3);

    public static double ToolX(MapSize space) => space.Width * 0.5;

    public static MapPoint Hub(MapSize space)
    {
        var band = Band(space);
        return new MapPoint(space.Width * 0.72, (band.Lower + band.Upper) / 2);
    }

    public static MapPoint Library(MapSize space) => new(Hub(space).X, Band(space).Lower + space.Height * 0.04);

    public const double FolderStepMin = 22;
    public const double FolderStepMax = 34;
    public const int FoldersShown = 10;
    public const double FolderRadius = 7;
    public const int FoldersRevealed = 6;
    public const string FolderColumn = "folders";
    public const double PadHalfWidth = 32;

    /// <summary>The reveal after a click on the pill (:262-270).</summary>
    public static IReadOnlyDictionary<string, int> Revealing(MapFolderReveal reveal, IReadOnlyDictionary<string, int> revealed)
    {
        var next = new Dictionary<string, int>(revealed, StringComparer.Ordinal);
        if (reveal.Hidden > 0)
        {
            next[reveal.Tool] = (next.TryGetValue(reveal.Tool, out int n) ? n : 0) + reveal.Next;
        }
        else
        {
            next.Remove(reveal.Tool);
        }

        return next;
    }

    /// <summary>The space grown until the band holds <paramref name="units"/> lines (:275-282).</summary>
    public static MapSize Holding(MapSize space, double units)
    {
        var band = Band(space);
        double length = band.Upper - band.Lower;
        double needed = units * FolderStepMin;
        if (length <= 0 || needed <= length)
        {
            return space;
        }

        double factor = needed / length;
        return new MapSize(space.Width * factor, space.Height * factor);
    }

    /// <summary>The folders the column draws (:321-325).</summary>
    public static IReadOnlyList<TracesTree.FolderNode> Directories(TracesTree tree)
    {
        if (tree.Folders.Count > 0)
        {
            return tree.Folders;
        }

        var seen = new HashSet<string>(StringComparer.Ordinal);
        return tree.Tools.SelectMany(t => t.Folders).Concat(tree.Unplaced).Where(f => seen.Add(f.Id)).ToList();
    }

    /// <summary>The tools a folder's traces come from (:329-334).</summary>
    public static IReadOnlyList<TracesTree.ToolNode> Carriers(TracesTree.FolderNode folder, TracesTree tree)
    {
        var sources = folder.Sessions.Select(s => s.Source).ToHashSet(StringComparer.Ordinal);
        return tree.Tools.Where(t => t.Folders.Any(f => f.Id == folder.Id) || sources.Contains(t.Id)).ToList();
    }

    /// <summary>Every tool, the tree's first, then the unwatched (:373-379).</summary>
    public static IReadOnlyList<(TracesTree.ToolNode Tool, bool Watched)> MapToolsOf(TracesTree tree)
    {
        var all = tree.Tools.Select(t => (t, true)).ToList();
        foreach (string kind in MapTools.All)
        {
            if (!tree.Tools.Any(t => t.Kind == kind))
            {
                all.Add((new TracesTree.ToolNode(kind, SourceMode.Unset, Array.Empty<TracesTree.FolderNode>()), false));
            }
        }

        return all;
    }

    /// <summary>The Traces map (:397-484).</summary>
    public static FlowMapScene Traces(
        TracesTree tree, CommonsGate gate, FlowMapCopy? words, MapContributions? contributed = null,
        string? selectedTool = null, IReadOnlyDictionary<string, int>? revealed = null, MapSize? space = null)
    {
        contributed ??= MapContributions.Unread;
        revealed ??= new Dictionary<string, int>();
        var tools = MapToolsOf(tree).Where(entry =>
        {
            if (gate.Destinations is not { } destinations || !destinations.Tools.Any(t => t.Installed == true))
            {
                return true;
            }

            if (entry.Watched || (contributed.Tool(entry.Tool.Kind) ?? 0) > 0)
            {
                return true;
            }

            if (destinations.ManagedSessions?.Any(m => m.Source == entry.Tool.Id) == true)
            {
                return true;
            }

            MapToolDestination? row = destinations.Tools.FirstOrDefault(t => t.Tool == entry.Tool.Id);
            return row?.Installed != false || (row?.SessionCount ?? 0) > 0;
        }).ToList();
        IReadOnlyList<TracesTree.FolderNode> folders = Directories(tree);
        int shown = Math.Min(folders.Count, FoldersShown + Math.Max(0, revealed.TryGetValue(FolderColumn, out int more) ? more : 0));
        bool pill = folders.Count > FoldersShown;
        MapSize sized = Holding(space ?? Size, shown + (pill ? 1 : 0));
        MapPoint hubPoint = Hub(sized);
        MapPoint libraryPoint = Library(sized);
        double folderX = FolderX(sized);
        var band = Band(sized);
        var group = new MapToolStack(new MapPoint(ToolX(sized), (band.Lower + band.Upper) / 2), tools.Select(t => "tool:" + t.Tool.Id).ToList());

        var nodes = new List<MapNode>();
        var arcs = new List<MapArc>();
        var reveals = new List<MapFolderReveal>();
        var slots = StackDown(Enumerable.Repeat(1.0, shown + (pill ? 1 : 0)).ToList(), band, FolderStepMax, FolderStepMin);
        bool flowing = false;
        for (int index = 0; index < shown; index++)
        {
            TracesTree.FolderNode folder = folders[index];
            var point = new MapPoint(folderX, slots[index].Y);
            IReadOnlyList<TracesTree.ToolNode> carriers = Carriers(folder, tree);
            var live = carriers.Where(c => c.Mode != SourceMode.Off).ToList();
            bool armed = folder.Mode == MapModes.AutoUpload && live.Count > 0;
            bool moves = armed && gate.Open && live.Any(c => gate.SendsToCommons(c.Kind));
            flowing = flowing || moves;
            bool faded = selectedTool is not null && !carriers.Any(c => c.Id == selectedTool);
            double dim = faded ? 0.3 : 1;
            arcs.Add(Curve(new MapPoint(point.X + FolderRadius + 1, point.Y), group.PortLeft,
                moves ? MapArcStyle.Flowing : armed ? MapArcStyle.Dashed : MapArcStyle.Quiet, dim));
            nodes.Add(new MapNode("folder:" + folder.Id, MapNodeKind.Folder, point, FolderRadius, folder.Label,
                FolderDetail(folder, words, contributed.Folder(folder.Id)))
            {
                Dim = dim,
                Mode = folder.Mode,
            });
        }

        if (pill && slots.Count > 0)
        {
            reveals.Add(new MapFolderReveal(FolderColumn, string.Empty, new MapPoint(folderX, slots[^1].Y), folders.Count - shown));
        }

        arcs.Add(Curve(group.PortRight, new MapPoint(hubPoint.X - PadHalfWidth, hubPoint.Y),
            flowing ? MapArcStyle.Flowing : MapArcStyle.Quiet, 1));

        for (int index = 0; index < tools.Count; index++)
        {
            (TracesTree.ToolNode tool, bool watched) = tools[index];
            bool off = tool.Mode == SourceMode.Off;
            bool faded = selectedTool is not null && selectedTool != tool.Id;
            nodes.Add(new MapNode("tool:" + tool.Id, MapNodeKind.Tool, group.CoinCenter(index, false), MapToolStack.Coin / 2,
                MapTools.DisplayName(tool.Kind), ToolDetail(tool, words) + ManagedDetail(tool.Id, gate.Destinations))
            {
                Tool = MapTools.Art(tool.Kind),
                Off = off,
                Dim = (off ? 0.4 : 1) * (faded ? 0.3 : 1),
                Title = ToolTitle(tool, words),
                Dashed = !watched,
                Ringed = selectedTool == tool.Id,
                Waiting = tool.Waiting,
            });
        }

        int waiting = tree.Tools.Sum(t => t.Waiting) + tree.Unplaced.Sum(f => f.Sessions.Count);
        nodes.Insert(0, new MapNode("hub", MapNodeKind.Hub, hubPoint, 16, words?.Top("computer") ?? string.Empty,
            words?.Hub(waiting, contributed.Total) ?? Pair(words?.Top("waiting") ?? string.Empty, waiting)));
        nodes.Add(new MapNode("library", MapNodeKind.Library, libraryPoint, 22, words?.Top("commons") ?? string.Empty,
            words?.Library(contributed.Total) ?? Pair(words?.Top("contributed") ?? string.Empty, contributed.Total))
        {
            Active = flowing || (contributed.Total ?? 0) > 0,
        });
        return new FlowMapScene { Space = sized, Stack = group, Nodes = nodes, Arcs = arcs, Reveals = reveals };
    }

    /// <summary>The camera's centre for a tool (:488-491).</summary>
    public MapPoint? FocusPoint(string? tool) =>
        tool is null ? null : Nodes.FirstOrDefault(n => n.Id == "tool:" + tool)?.At;

    /// <summary>A tool's card title (:497-499).</summary>
    public static string ToolTitle(TracesTree.ToolNode tool, FlowMapCopy? words) =>
        words?.ToolTitle(MapTools.DisplayName(tool.Kind), tool.Folders.Count) ?? MapTools.DisplayName(tool.Kind);

    /// <summary>A tool's card (:503-511).</summary>
    public static string ToolDetail(TracesTree.ToolNode tool, FlowMapCopy? words) =>
        words?.Tool(tool.Mode, tool.Waiting) ?? Pair(string.Empty, tool.Waiting);

    /// <summary>A folder's card (:515-520).</summary>
    public static string FolderDetail(TracesTree.FolderNode folder, FlowMapCopy? words, int? contributed = null) =>
        words?.Folder(folder.Path, folder.Mode is { } mode ? WatchCopy.ModeLabel(mode) : null, folder.Sessions.Count, contributed)
        ?? Pair(string.Empty, folder.Sessions.Count);

    // Other devices (:522-616).

    public const int DevicesShown = 4;
    public const double DeviceClearance = 72;

    /// <summary>The other devices, below this computer (:569-616).</summary>
    public FlowMapScene AddingDevices(IReadOnlyList<MapOtherDevice> devices, bool offersLink = false)
    {
        MapNode? hub = Nodes.FirstOrDefault(n => n.Id == "hub");
        if ((devices.Count == 0 && !offersLink) || !Nodes.Any(n => n.Id == "library") || hub is null)
        {
            return this;
        }

        var band = Band(Space);
        var shown = devices.Take(DevicesShown).ToList();
        int more = devices.Count - shown.Count;
        double top = hub.At.Y + DeviceClearance;
        int weights = shown.Count + (more > 0 ? 1 : 0);
        double room = Math.Max(band.Upper - top, 1);
        double units = weights + (offersLink ? 1 : 0);
        double step = Math.Min(58, Math.Max(offersLink ? 24 : 40, room / Math.Max(units, 1)));
        var nodes = Nodes.ToList();
        for (int index = 0; index < shown.Count; index++)
        {
            MapOtherDevice device = shown[index];
            nodes.Add(new MapNode("device:" + device.Id, MapNodeKind.Device, new MapPoint(hub.At.X, top + step * index), 12,
                device.Name, device.Detail));
        }

        if (devices.Any(d => (d.Contributed ?? 0) > 0))
        {
            int at = nodes.FindIndex(n => n.Id == "library");
            nodes[at] = nodes[at] with { Active = true };
        }

        var overflows = Overflows.ToList();
        if (more > 0)
        {
            overflows.Add(new MapOverflow(new MapPoint(hub.At.X, top + step * shown.Count - step * 0.4), more));
        }

        MapPoint? add = AddDevice;
        if (offersLink)
        {
            double slot = shown.Count + (more > 0 ? 0.7 : 0);
            add = new MapPoint(hub.At.X, Math.Min(top + step * slot, band.Upper));
        }

        return this with { Nodes = nodes, Overflows = overflows, AddDevice = add };
    }

    // Private AI (:618-795).

    public static MapPoint Destination(MapSize space)
    {
        var band = Band(space);
        return new MapPoint(space.Width * 0.68, (band.Lower + band.Upper) / 2);
    }

    public static double HarnessX(MapSize space) => Math.Max(150, space.Width * 0.36);

    public const double HarnessStepMin = 30;
    public const double HarnessStepMax = 72;

    /// <summary>
    /// The Private AI map (:645-735). <paramref name="harnesses"/> null is an
    /// unread list, which draws nothing.
    /// </summary>
    public static FlowMapScene PrivateAI(
        IReadOnlyList<HarnessRow>? harnesses, string destinationLabel, string? privateAI,
        Func<HarnessRow, string?> sentence, Func<HarnessRow, HarnessState> state, FlowMapCopy? words,
        MapDestinations? destinations = null, MapSize? space = null)
    {
        MapSize sized = space ?? Size;
        if (harnesses is null)
        {
            return new FlowMapScene { Space = sized };
        }

        bool hasInstalled = destinations?.Tools.Any(t => t.Installed == true) == true || harnesses.Any(h => h.Installed);
        var rows = harnesses.Where(row =>
        {
            if (!hasInstalled)
            {
                return true;
            }

            string source = row.Id == "claude" ? MapTools.ClaudeCode : row.Id;
            return destinations?.Tools.FirstOrDefault(t => t.Tool == source)?.Installed ?? row.Installed;
        }).ToList();
        MapPoint destinationPoint = Destination(sized);
        double x = HarnessX(sized);
        var band = Band(sized);
        int fit = Math.Max(1, (int)((band.Upper - band.Lower) / HarnessStepMin));
        var shown = rows.Count > fit ? rows.Take(fit - 1).ToList() : rows;
        var weights = Enumerable.Repeat(1.0, shown.Count).ToList();
        if (shown.Count < rows.Count)
        {
            weights.Add(0.7);
        }

        var slots = StackDown(weights, band, HarnessStepMax, HarnessStepMin);
        var overflows = new List<MapOverflow>();
        if (shown.Count < rows.Count && slots.Count > 0)
        {
            overflows.Add(new MapOverflow(new MapPoint(x - 24, slots[^1].Y), rows.Count - shown.Count));
        }

        bool anyAnswering = false;
        int connected = 0;
        var nodes = new List<MapNode>();
        var arcs = new List<MapArc>();
        for (int index = 0; index < shown.Count; index++)
        {
            HarnessRow row = shown[index];
            double y = slots[index].Y;
            MapArcStyle style;
            switch (state(row))
            {
                case HarnessState.Answering:
                    style = MapArcStyle.Flowing;
                    anyAnswering = true;
                    connected++;
                    break;
                case HarnessState.ConnectedNoCalls:
                    style = MapArcStyle.Quiet;
                    connected++;
                    break;
                case HarnessState.ActivityShared:
                    style = MapArcStyle.Shared;
                    connected++;
                    break;
                case HarnessState.NotConnected:
                    style = MapArcStyle.Dashed;
                    break;
                default:
                    style = MapArcStyle.Unknown;
                    break;
            }

            double bend = (x + destinationPoint.X) / 2;
            arcs.Add(new MapArc(new MapPoint(x + 12, y), new MapPoint(bend, y), new MapPoint(bend, destinationPoint.Y), destinationPoint, style));
            nodes.Add(new MapNode("harness:" + row.Id, MapNodeKind.Harness, new MapPoint(x, y), 13, row.Name, sentence(row) ?? "—")
            {
                Tool = MapTools.ForHarness(row.Id) ?? GlassToolArt.Other(Initials(row.Name)),
                Installed = row.Installed,
                Dim = row.Installed ? 1 : 0.5,
            });
        }

        foreach (HarnessRow row in rows.Skip(shown.Count))
        {
            switch (state(row))
            {
                case HarnessState.Answering:
                    anyAnswering = true;
                    connected++;
                    break;
                case HarnessState.ConnectedNoCalls or HarnessState.ActivityShared:
                    connected++;
                    break;
            }
        }

        MapLamp lamp = Lamp(privateAI, anyAnswering);
        string detail = words?.Connected(connected, true) ?? Pair(words?.Top("connected") ?? string.Empty, connected);
        if (lamp == MapLamp.Unknown)
        {
            detail += " " + (words?.Top("unknown") ?? string.Empty);
        }

        nodes.Insert(0, new MapNode("destination", MapNodeKind.Destination, destinationPoint, 26,
            words?.Credential ?? destinationLabel, detail)
        {
            Lamp = lamp,
            Sublabel = rows.Count == 0 ? words?.NoneFound : words?.Connected(connected, false),
        });
        return new FlowMapScene { Space = sized, Nodes = nodes, Arcs = arcs, Overflows = overflows };
    }

    /// <summary>Managed routes (:743-777).</summary>
    public FlowMapScene AddingManagedRoutes(MapDestinations? destinations, FlowMapCopy? words = null)
    {
        if (destinations?.ManagedSessions is not { } sessions || destinations.ManagedCopy is not { } copy)
        {
            return this;
        }

        var groups = sessions.GroupBy(s => s.Tool + ":" + s.Provider).ToDictionary(g => g.Key, g => g.ToList(), StringComparer.Ordinal);
        var keys = groups.Keys.OrderBy(k => k, StringComparer.Ordinal).ToList();
        var nodes = Nodes.ToList();
        var arcs = Arcs.ToList();
        for (int index = 0; index < keys.Count; index++)
        {
            List<MapManagedSession> rows = groups[keys[index]];
            MapManagedSession first = rows[0];
            var band = Band(Space);
            double top = band.Lower + (band.Upper - band.Lower) * 0.66;
            double y = keys.Count > 1
                ? top + index * (band.Upper - top) / (keys.Count - 1)
                : band.Lower + (band.Upper - band.Lower) * 0.85;
            var start = new MapPoint(Space.Width * 0.8, y);
            var end = new MapPoint(Space.Width * 0.93, y);
            string details = string.Join("\n", rows.Take(4).Select(r => r.Detail(copy)));
            string overflow = rows.Count > 4 ? "\n+" + (rows.Count - 4) : string.Empty;
            var lines = new List<string>();
            if (copy.TryGetValue("map_scope", out string? scope))
            {
                lines.Add(scope);
            }

            lines.Add(details + overflow);
            string detail = string.Join("\n", lines);
            string name = copy.TryGetValue(first.Tool, out string? named) ? named : first.Tool;
            copy.TryGetValue("title", out string? title);
            nodes.Add(new MapNode("managed:" + keys[index], MapNodeKind.Harness, start, 13, name, detail)
            {
                Tool = MapTools.ForHarness(first.Tool) ?? GlassToolArt.Other(Initials(name)),
                Installed = true,
                Title = title,
                Sublabel = title is null ? null : DotPair(title, rows.Count),
            });
            nodes.Add(new MapNode("managed-provider:" + keys[index], MapNodeKind.Destination, end, 16,
                copy.TryGetValue("provider_" + first.Provider, out string? provider) ? provider : words?.Top("unknown") ?? string.Empty, detail)
            {
                Lamp = MapLamp.Unknown,
            });
            arcs.Add(Curve(start, end, MapArcStyle.Dashed, 1));
        }

        return this with { Nodes = nodes, Arcs = arcs };
    }

    private static string ManagedDetail(string source, MapDestinations? destinations)
    {
        if (destinations?.ManagedCopy is not { } copy || destinations.ManagedSessions is not { } all)
        {
            return string.Empty;
        }

        var rows = all.Where(r => r.Source == source).ToList();
        if (rows.Count == 0)
        {
            return string.Empty;
        }

        string detail = string.Join("\n", rows.Take(4).Select(r => r.Detail(copy)));
        string overflow = rows.Count > 4 ? "\n+" + (rows.Count - 4) : string.Empty;
        return "\n" + (copy.TryGetValue("title", out string? title) ? title : string.Empty) + "\n" + detail + overflow;
    }

    /// <summary>The lamp from the core's Private AI state (:789-795).</summary>
    public static MapLamp Lamp(string? privateAI, bool answering) => privateAI switch
    {
        "running" => answering ? MapLamp.Answering : MapLamp.Running,
        "off" => MapLamp.Off,
        _ => MapLamp.Unknown,
    };

    /// <summary>Two letters of a name (:801-808).</summary>
    public static string Initials(string name)
    {
        var words = new List<string>();
        var current = new System.Text.StringBuilder();
        foreach (System.Text.Rune rune in name.EnumerateRunes())
        {
            if (System.Text.Rune.IsLetterOrDigit(rune))
            {
                current.Append(rune.ToString());
            }
            else if (current.Length > 0)
            {
                words.Add(current.ToString());
                current.Clear();
            }
        }

        if (current.Length > 0)
        {
            words.Add(current.ToString());
        }

        if (words.Count == 0)
        {
            return "?";
        }

        string FirstUpper(string w) => System.Globalization.StringInfo.GetNextTextElement(w).ToUpperInvariant();
        if (words.Count > 1)
        {
            return FirstUpper(words[0]) + FirstUpper(words[1]);
        }

        string first = System.Globalization.StringInfo.GetNextTextElement(words[0]);
        string rest = words[0][first.Length..];
        string second = rest.Length > 0 ? System.Globalization.StringInfo.GetNextTextElement(rest).ToLowerInvariant() : string.Empty;
        return first.ToUpperInvariant() + second;
    }

    /// <summary>"Waiting 3", or a dash for an unreported count (:819-826).</summary>
    public static string Pair(string word, int? count) => word + " " + (count?.ToString(System.Globalization.CultureInfo.InvariantCulture) ?? "—");

    /// <summary>"Sessions · 3" (:830-832).</summary>
    public static string DotPair(string word, int? count) => word + " · " + (count?.ToString(System.Globalization.CultureInfo.InvariantCulture) ?? "—");

    private static MapArc Curve(MapPoint from, MapPoint to, MapArcStyle style, double dim)
    {
        double midX = (from.X + to.X) / 2;
        return new MapArc(from, new MapPoint(midX, from.Y), new MapPoint(midX, to.Y), to, style, dim);
    }
}
