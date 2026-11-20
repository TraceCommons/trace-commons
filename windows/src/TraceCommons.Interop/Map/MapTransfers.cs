using System;
using System.Collections.Generic;
using System.Linq;

namespace TraceCommons.Interop.Map;

/// <summary>What a transfer does (MapTransfers.swift:11).</summary>
public enum TransferAction
{
    Contribute,
    Keep,
    Dismiss,
}

/// <summary>The Traces map's library and computer (TracesMapStyle.swift:8-11).</summary>
public enum TracesMapStyle
{
    /// <summary>The rocket on its pad and the space station: the default.</summary>
    Launch,

    /// <summary>The laptop and the turning orb.</summary>
    Standard,
}

/// <summary>
/// A trace the map shows moving, a port of <c>MapTransfer</c>
/// (MapTransfers.swift:10-48). Built only from what the core reported.
/// </summary>
public sealed record MapTransfer(string Id, string Folder, string Tool, TransferAction Action)
{
    public DateTimeOffset? At { get; init; }

    public string? FolderLabel { get; init; }

    public int Count { get; init; } = 1;

    public bool MixedFolders { get; init; }

    public bool MixedTools { get; init; }

    /// <summary>One card for a run of several traces (:38-47).</summary>
    public static MapTransfer? Batch(IReadOnlyList<MapTransfer> transfers)
    {
        if (transfers.Count == 0)
        {
            return null;
        }

        MapTransfer first = transfers[0];
        if (transfers.Count == 1)
        {
            return first;
        }

        return first with
        {
            Id = "batch:" + first.Id,
            Count = transfers.Count,
            MixedFolders = transfers.Select(t => t.Folder).Distinct().Count() > 1,
            MixedTools = transfers.Select(t => t.Tool).Distinct().Count() > 1,
        };
    }
}

/// <summary>Finds transfers between two reads (MapTransfers.swift:57-92).</summary>
public static class MapTransferDetector
{
    /// <summary>History rows sent after everything the earlier read held (:58-72).</summary>
    public static IReadOnlyList<MapTransfer> Contributions(IReadOnlyList<MapHistoryRow>? old, IReadOnlyList<MapHistoryRow>? fresh)
    {
        if (old is null || fresh is null)
        {
            return Array.Empty<MapTransfer>();
        }

        var seen = old.Select(r => r.SubmissionId).ToHashSet(StringComparer.Ordinal);
        DateTimeOffset newest = old.Select(r => r.SubmittedAt).Where(d => d is not null).Select(d => d!.Value).DefaultIfEmpty(DateTimeOffset.MinValue).Max();
        return fresh
            .Where(r => !seen.Contains(r.SubmissionId) && r.SubmittedAt is { } sent && sent > newest && r.ProjectId is not null && r.Source is not null)
            .Select(r => new MapTransfer(r.SubmissionId, r.ProjectId!, r.Source!, TransferAction.Contribute) { At = r.SubmittedAt, FolderLabel = r.ProjectLabel })
            .OrderBy(t => t.At ?? DateTimeOffset.MinValue)
            .ToList();
    }

    /// <summary>The entry just kept, among <paramref name="sessions"/> (:76-78).</summary>
    public static MapTransfer? Kept(string? entryId, IReadOnlyList<MapEntry> sessions) => Decided(entryId, TransferAction.Keep, sessions);

    /// <summary>The entry just dismissed (:81-83).</summary>
    public static MapTransfer? Dismissed(string? entryId, IReadOnlyList<MapEntry> sessions) => Decided(entryId, TransferAction.Dismiss, sessions);

    private static MapTransfer? Decided(string? entryId, TransferAction action, IReadOnlyList<MapEntry> sessions)
    {
        MapEntry? entry = entryId is null ? null : sessions.FirstOrDefault(s => s.EntryId == entryId);
        return entry is null
            ? null
            : new MapTransfer(entry.EntryId, entry.ProjectId, entry.Source, action) { At = entry.StartedAt, FolderLabel = entry.ProjectLabel };
    }
}

/// <summary>Where one trace is in a run (MapTransfers.swift:131-145).</summary>
public enum LegKind
{
    Waiting,
    Inbound,
    Orbit,
    Outbound,
    Ascent,
    Arrived,
}

/// <summary>A leg and its progress (0..1), or the seconds since arrival.</summary>
public readonly record struct Leg(LegKind Kind, double Value = 0);

/// <summary>
/// One run of transfers played together, every moment a function of the
/// seconds since it began (MapTransferRun, MapTransfers.swift:108-232).
/// </summary>
public sealed record MapTransferRun(IReadOnlyList<MapTransfer> Transfers, double Start, TracesMapStyle Style = TracesMapStyle.Launch)
{
    public const double Step = 1.05;
    public const double Inbound = 0.70;
    public const double Orbit = 0.45;
    public const double Outbound = 0.70;
    public const double Settle = 0.5;
    public const double Flight = 2.6;
    public const double Linger = 1.3;
    public const double KeptLinger = 1.6;
    public const double Pulse = 0.9;
    public const double Ascent = 1.1;

    public TransferAction Action => Transfers.Count > 0 ? Transfers[0].Action : TransferAction.Contribute;

    /// <summary>The standard map's contributions rise to the orb one by one (:148).</summary>
    public bool Rises => Style == TracesMapStyle.Standard && Action == TransferAction.Contribute;

    /// <summary>:150-163.</summary>
    public Leg LegOf(int index, double t)
    {
        double local = t - index * Step;
        if (local < 0)
        {
            return new Leg(LegKind.Waiting);
        }

        if (local < Inbound)
        {
            return new Leg(LegKind.Inbound, local / Inbound);
        }

        local -= Inbound;
        if (local < Orbit)
        {
            return new Leg(LegKind.Orbit, local / Orbit);
        }

        local -= Orbit;
        if (local < Outbound)
        {
            return new Leg(LegKind.Outbound, local / Outbound);
        }

        local -= Outbound;
        if (Rises)
        {
            if (local < Ascent)
            {
                return new Leg(LegKind.Ascent, local / Ascent);
            }

            local -= Ascent;
        }

        return new Leg(LegKind.Arrived, local);
    }

    /// <summary>:166-169.</summary>
    public double AllArrived => Math.Max(Transfers.Count - 1, 0) * Step + Inbound + Orbit + Outbound + (Rises ? Ascent : 0);

    public double LiftOff => AllArrived + Settle;

    /// <summary>:175-178.</summary>
    public double End => Action != TransferAction.Contribute ? AllArrived + KeptLinger
        : Rises ? AllArrived + Linger : LiftOff + Flight + Linger;

    public int ArrivedBy(double t) => Enumerable.Range(0, Transfers.Count).Count(i => LegOf(i, t).Kind == LegKind.Arrived);

    /// <summary>The pile, each set-off trace's tool moved to the front (:187-196).</summary>
    public IReadOnlyList<string> Order(IReadOnlyList<string> pile, double t)
    {
        var order = pile.ToList();
        for (int i = 0; i < Transfers.Count; i++)
        {
            if (LegOf(i, t).Kind == LegKind.Waiting)
            {
                continue;
            }

            string id = "tool:" + Transfers[i].Tool;
            int at = order.IndexOf(id);
            if (at < 0)
            {
                continue;
            }

            order.RemoveAt(at);
            order.Insert(0, id);
        }

        return order;
    }

    /// <summary>The coins a trace is in, and how far round the rim light is (:200-206).</summary>
    public IReadOnlyDictionary<string, double> Orbits(double t)
    {
        var orbits = new Dictionary<string, double>(StringComparer.Ordinal);
        for (int i = 0; i < Transfers.Count; i++)
        {
            Leg leg = LegOf(i, t);
            if (leg.Kind == LegKind.Orbit)
            {
                orbits["tool:" + Transfers[i].Tool] = leg.Value;
            }
        }

        return orbits;
    }

    /// <summary>The latest to set off that has not yet arrived (:210-217).</summary>
    public MapTransfer? Current(double t)
    {
        for (int i = Transfers.Count - 1; i >= 0; i--)
        {
            if (LegOf(i, t).Kind is not (LegKind.Waiting or LegKind.Arrived))
            {
                return Transfers[i];
            }
        }

        return null;
    }

    public bool Moving(double t) => ArrivedBy(t) < Transfers.Count;

    /// <summary>The rocket's flight, 0..1, or null (:225-228).</summary>
    public double? FlightAt(double t) =>
        Action != TransferAction.Contribute || Rises || t < LiftOff ? null : Math.Min(1, (t - LiftOff) / Flight);

    /// <summary>Still for the ignition's first 26%, then pow(x, 2.4) (:233-238).</summary>
    public static double Climb(double progress)
    {
        const double Ignition = 0.26;
        if (progress <= Ignition)
        {
            return 0;
        }

        double x = (progress - Ignition) / (1 - Ignition);
        return Math.Pow(x, 2.4);
    }
}

/// <summary>
/// Plays runs one at a time and keeps the cards at the map's foot
/// (MapTransferPlayer, MapTransfers.swift:250-334), on an explicit clock: the
/// caller passes the time, in seconds, to every call and to <see cref="Tick"/>,
/// so a run's end and each card's hold are tested without waiting.
/// </summary>
public sealed class MapTransferPlayer
{
    /// <summary>How long a card stays past its run's end, or past a dismissal (:264).</summary>
    public const double Hold = 5;

    /// <summary>The most cards kept; the oldest go first past it (:266).</summary>
    public const int StackLimit = 6;

    private readonly List<MapTransfer> _queue = new();
    private readonly HashSet<string> _played = new(StringComparer.Ordinal);
    private readonly List<MapTransfer> _cards = new();
    private readonly Dictionary<string, double> _expiries = new(StringComparer.Ordinal);

    /// <summary>The run playing now; null between runs.</summary>
    public MapTransferRun? Run { get; private set; }

    /// <summary>The cards, oldest first; the last is in front.</summary>
    public IReadOnlyList<MapTransfer> Cards => _cards;

    public MapTransfer? Front => _cards.Count > 0 ? _cards[^1] : null;

    /// <summary>The style for runs still to play; one already playing keeps its own.</summary>
    public TracesMapStyle Style { get; set; } = TracesMapStyle.Launch;

    /// <summary>Raised whenever the run or the cards change.</summary>
    public event Action? Changed;

    /// <summary>Queues new contributions and keeps (:271-276).</summary>
    public void Enqueue(IEnumerable<MapTransfer> transfers, double now)
    {
        var fresh = transfers.Where(t => t.Action != TransferAction.Dismiss && _played.Add(t.Id)).ToList();
        if (fresh.Count == 0)
        {
            return;
        }

        _queue.AddRange(fresh);
        if (Run is null)
        {
            Next(now);
        }

        Changed?.Invoke();
    }

    /// <summary>A card's X: that card alone goes (:280-283).</summary>
    public void Close(string id)
    {
        _expiries.Remove(id);
        if (_cards.RemoveAll(c => c.Id == id) > 0)
        {
            Changed?.Invoke();
        }
    }

    /// <summary>A dismissed trace: its card alone, for <see cref="Hold"/> (:287-290).</summary>
    public void Announce(MapTransfer transfer, double now)
    {
        if (!_played.Add(transfer.Id))
        {
            return;
        }

        Add(transfer, now + Hold);
        Changed?.Invoke();
    }

    /// <summary>
    /// Advances to <paramref name="now"/>: the run that ended gives way to the
    /// next, and cards past their hold go. True when anything changed.
    /// </summary>
    public bool Tick(double now)
    {
        bool changed = false;
        while (Run is { } run && now - run.Start >= run.End)
        {
            Next(run.Start + run.End);
            changed = true;
        }

        foreach (string id in _expiries.Where(e => now >= e.Value).Select(e => e.Key).ToList())
        {
            _expiries.Remove(id);
            _cards.RemoveAll(c => c.Id == id);
            changed = true;
        }

        if (changed)
        {
            Changed?.Invoke();
        }

        return changed;
    }

    /// <summary>The next moment anything here changes on its own; null when nothing will.</summary>
    public double? NextDeadline
    {
        get
        {
            double? next = Run is { } run ? run.Start + run.End : null;
            foreach (double expiry in _expiries.Values)
            {
                next = next is null ? expiry : Math.Min(next.Value, expiry);
            }

            return next;
        }
    }

    private void Next(double now)
    {
        if (_queue.Count == 0)
        {
            Run = null;
            return;
        }

        TransferAction action = _queue[0].Action;
        int count = _queue.TakeWhile(t => t.Action == action).Count();
        var batch = _queue.Take(count).ToList();
        _queue.RemoveRange(0, count);
        var run = new MapTransferRun(batch, now, Style);
        Run = run;
        if (MapTransfer.Batch(run.Transfers) is { } card)
        {
            Add(card, now + run.End + Hold);
        }
    }

    private void Add(MapTransfer transfer, double until)
    {
        _cards.RemoveAll(c => c.Id == transfer.Id);
        _cards.Add(transfer);
        while (_cards.Count > StackLimit)
        {
            _expiries.Remove(_cards[0].Id);
            _cards.RemoveAt(0);
        }

        _expiries[transfer.Id] = Math.Max(until, 0);
    }
}

/// <summary>
/// What a run looks like at a moment, as numbers the map draws
/// (<c>MapTransferDrawing</c>, MapToolPile.swift:215-532). Positions are in
/// the design space; the map scales them.
/// </summary>
public static class TransferDrawing
{
    /// <summary>The orb's frame on the standard map and its sphere's radius (:221-222).</summary>
    public const double OrbFrame = 104;

    public static double OrbRadius => OrbFrame / 2 * (1 - LibraryOrb.Inset);

    /// <summary>The rocket's length in design units (:370).</summary>
    public const double RocketLength = 44;

    /// <summary>How far above the computer the laptop's screen rises (FlowMapView.swift:427).</summary>
    public const double LaptopScreenTop = 36;

    /// <summary>The trace orb's radius (:340).</summary>
    public const double OrbDot = 7;

    /// <summary>A run's glow token: shared blue, kept orange (:229-231).</summary>
    public static string GlowToken(TransferAction? action) => action == TransferAction.Contribute ? "mapOrbShared" : "mapOrbKept";

    /// <summary>Cubic in-out on the legs (:329-333).</summary>
    public static double Ease(double t) => t < 0.5 ? 4 * t * t * t : 1 - Math.Pow(-2 * t + 2, 3) / 2;

    /// <summary>The vanish pulse at <paramref name="progress"/>: radius and alpha (:349-355).</summary>
    public static (double Radius, double Alpha) PulseAt(double progress) =>
        (6 + 38 * progress, progress < 0.2 ? progress / 0.2 * 0.95 : 0.95 * (1 - (progress - 0.2) / 0.8));

    /// <summary>The rise's four orbs, trail first: position, opacity and size (:295-310).</summary>
    public static IReadOnlyList<(MapPoint At, double Opacity, double Size)> Rise(MapPoint start, MapPoint end, double progress)
    {
        var orbs = new List<(MapPoint, double, double)>();
        for (int step = 3; step >= 0; step--)
        {
            double lag = step * 0.045;
            double along = Math.Pow(Math.Max(0, progress - lag), 1.7);
            orbs.Add((new MapPoint(start.X + (end.X - start.X) * along, start.Y + (end.Y - start.Y) * along), 1 - step * 0.28, 1 - step * 0.18));
        }

        return orbs;
    }

    /// <summary>
    /// The rim light <paramref name="progress"/> of the way round a coin,
    /// counter-clockwise from its base: up its right side first (:278-292).
    /// </summary>
    public static MapPoint RimLight(MapPoint centre, double progress)
    {
        const double R = MapToolStack.Coin / 2 * 1.3;
        double angle = 2 * Math.PI * progress;
        return new MapPoint(centre.X + R * Math.Sin(angle), centre.Y + R * Math.Cos(angle));
    }

    /// <summary>The station's dock ring: radius and alpha, null once faded (:469-476).</summary>
    public static (double Radius, double Alpha)? Dock(double since)
    {
        if (since < 0 || since >= 1.1)
        {
            return null;
        }

        double p = since / 1.1;
        return (28 + 26 * p, 0.75 * (1 - p));
    }

    /// <summary>Whether the dock's "+N" still shows (:477).</summary>
    public static bool DockCount(double since) => since >= 0 && since < MapTransferRun.Linger;

    /// <summary>The two smoke puffs: x offset, radius and alpha (:455-466).</summary>
    public static IReadOnlyList<(double Offset, double Radius, double Alpha)> Smoke(double progress)
    {
        var puffs = new List<(double, double, double)>();
        foreach ((double offset, double delay) in new[] { (0.0, 0.0), (-14.0, 0.15) })
        {
            double p = Math.Max(0, progress - delay) / (1 - delay);
            if (p > 0)
            {
                puffs.Add((offset, 6 + 30 * p, 0.5 * (1 - p)));
            }
        }

        return puffs;
    }

    /// <summary>The rocket standing on the pad (:388, :419).</summary>
    public static MapPoint RocketRest(MapPoint hub) => new(hub.X, hub.Y - 5 - RocketLength / 2);

    /// <summary>
    /// The rocket's centre, nose angle (radians, y down) and flame scale
    /// (null while it stands) at <paramref name="flight"/> of its flight; the
    /// ignition shake reads <paramref name="t"/> (:373-412).
    /// </summary>
    public static (MapPoint At, double Angle, double? Burn) Rocket(MapPoint rest, MapPoint library, double? flight, double t)
    {
        var path = new MapArc(rest, new MapPoint(rest.X, rest.Y - 118), new MapPoint(library.X, library.Y + 110), library, MapArcStyle.Quiet);
        double along = MapTransferRun.Climb(flight ?? 0);
        MapPoint at = path.PointAt(along);
        MapPoint ahead = path.PointAt(Math.Min(1, along + 0.01));
        double angle = Math.Atan2(ahead.Y - at.Y, ahead.X - at.X);
        if (along >= 0.999 || ahead == at)
        {
            angle = -Math.PI / 2;
        }

        double x = at.X;
        if (flight is not null && flight < 0.26)
        {
            x += Math.Sin(t * 90) * 0.7;
        }

        return (new MapPoint(x, at.Y), angle, flight is { } f ? 0.35 + 1.55 * f : null);
    }

    /// <summary>The pad's charge glow radius while traces gather (:398-400).</summary>
    public static double PadCharge(int arrived) => 5 + Math.Min(arrived, 4) * 4;

    /// <summary>The folder line a trace rides in on (:313-323).</summary>
    public static MapArc? FolderArc(string folder, FlowMapScene scene)
    {
        MapNode? node = scene.Nodes.FirstOrDefault(n => n.Id == "folder:" + folder);
        if (node is null)
        {
            return null;
        }

        var start = new MapPoint(node.At.X + FlowMapScene.FolderRadius + 1, node.At.Y);
        return scene.Arcs.FirstOrDefault(a => Math.Abs(a.From.X - start.X) < 0.5 && Math.Abs(a.From.Y - start.Y) < 0.5);
    }

    /// <summary>The line under this computer while a run plays (:497-512), in the "this computer" words.</summary>
    public static string? LineUnderPad(MapTransferRun run, double t, FlowMapCopy? words)
    {
        if (words is null)
        {
            return null;
        }

        int arrived = run.ArrivedBy(t);
        return run.Action switch
        {
            TransferAction.Contribute => run.Style == TracesMapStyle.Launch && run.Transfers.Count > 1 && arrived > 0 && t < run.LiftOff
                ? words.Collected(arrived, run.Transfers.Count) : null,
            TransferAction.Keep => arrived == run.Transfers.Count ? words.KeptHere(arrived) : null,
            _ => null,
        };
    }
}
