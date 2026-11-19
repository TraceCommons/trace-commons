using System;
using System.Collections.Generic;
using System.Linq;

namespace TraceCommons.Interop.FirstRun;

/// <summary>
/// What the first-run stage shows and when each part of it last changed:
/// the step, the tools, the sharing path, the scan, and the moments the
/// Mac's animations count from. Pure; the Win2D stage asks it for a
/// <see cref="StageSnapshot"/> each frame and draws that.
/// </summary>
/// <remarks>
/// Every change raises <see cref="Changed"/>, whatever the clock is doing,
/// so a stage with animations off still redraws each new state (checklist
/// O1). <see cref="Interval"/> says how often the stage must draw while
/// something moves, and null when nothing does, so no clock ticks then.
/// Like the Mac's <c>FirstRunStageMemory</c> (FirstRunStage.swift:548-578),
/// the scan, the Traces count and the Private AI count play once per first
/// run; going Back does not replay them.
/// </remarks>
public sealed class FirstRunStageModel
{
    private readonly Dictionary<FirstRunStep, double> _graphicTop = new();
    private readonly Dictionary<string, (double From, double To, double At)> _beams = new(StringComparer.Ordinal);
    private Dictionary<string, StageFrame> _tilesFrom = new(StringComparer.Ordinal);
    private StageFrame _hubFrom;

    /// <summary>Raised on every change the stage draws.</summary>
    public event Action? Changed;

    public FirstRunStep Step { get; private set; } = FirstRunStep.Welcome;

    public FirstRunStep? PreviousStep { get; private set; }

    public double StepChangedAt { get; private set; } = double.NegativeInfinity;

    /// <summary>When the current step opened, for the counts that play only once.</summary>
    public double StepOpenedAt { get; private set; }

    public bool ReduceMotion { get; private set; }

    public IReadOnlyList<(string Source, bool Exists)> Rows { get; private set; } = Array.Empty<(string, bool)>();

    public IReadOnlyList<string> Watched { get; private set; } = Array.Empty<string>();

    public IReadOnlyList<string> Connectable => Rows.Where(r => r.Exists && PrivateAIDiagram.Routable.Contains(r.Source)).Select(r => r.Source).ToList();

    public SharingPath Sharing { get; private set; } = SharingPath.AskMe;

    public double SharingSince { get; private set; }

    public double? ScanStartedAt { get; private set; }

    public bool ToolsScanned { get; private set; }

    public bool TracesCounted { get; private set; }

    public bool PrivateCounted { get; private set; }

    public double? SignedInAt { get; private set; }

    public ulong TracesTotal { get; private set; }

    /// <summary>The Welcome screen opened at this time (WelcomeScreen.swift:126).</summary>
    public double WelcomeOpenedAt { get; private set; }

    public void SetReduceMotion(bool reduce)
    {
        if (ReduceMotion != reduce)
        {
            ReduceMotion = reduce;
            Changed?.Invoke();
        }
    }

    /// <summary>Where the step's graphic starts down the canvas, measured by the window's layout.</summary>
    public void SetGraphicTop(FirstRunStep step, double top)
    {
        if (!_graphicTop.TryGetValue(step, out double was) || Math.Abs(was - top) > 0.01)
        {
            _graphicTop[step] = top;
            Changed?.Invoke();
        }
    }

    public double GraphicTop(FirstRunStep step) => _graphicTop.TryGetValue(step, out double t) ? t : 0;

    public void SetStep(FirstRunStep step, double now)
    {
        if (step == Step)
        {
            return;
        }

        // The actors leave from wherever they are drawn now, mid-move too.
        StageSnapshot from = Snapshot(now);
        _hubFrom = from.Hub;
        _tilesFrom = from.Tiles.ToDictionary(t => t.Source, t => t.Frame, StringComparer.Ordinal);
        PreviousStep = Step;
        Step = step;
        StepChangedAt = now;
        StepOpenedAt = now;
        if (step == FirstRunStep.Welcome)
        {
            WelcomeOpenedAt = now;
        }

        if (step == FirstRunStep.Tools)
        {
            StartScan(now);
        }

        Changed?.Invoke();
    }

    /// <summary>The Tools scan, once per first run; over at once under reduce motion (FirstRunStage.swift:563-578).</summary>
    public void StartScan(double now)
    {
        if (ScanStartedAt is not null || ToolsScanned)
        {
            return;
        }

        if (ReduceMotion)
        {
            ToolsScanned = true;
            return;
        }

        ScanStartedAt = now;
    }

    public void SetRows(IReadOnlyList<(string Source, bool Exists)> rows)
    {
        Rows = FirstRunStageLayout.RingRows(rows, r => r.Source);
        Changed?.Invoke();
    }

    /// <summary>The watched tools; each tile's beam fades in or out over 0.5 s (ToolsScreen.swift:436-437).</summary>
    public void SetWatched(IEnumerable<string> watched, double now)
    {
        var set = new HashSet<string>(watched, StringComparer.Ordinal);
        var ordered = FirstRunStageLayout.RingOrder.Where(set.Contains).ToList();
        foreach (string kind in FirstRunStageLayout.RingOrder)
        {
            bool was = Watched.Contains(kind), now2 = set.Contains(kind);
            if (was != now2)
            {
                double current = BeamOpacity(kind, now);
                _beams[kind] = (current, now2 ? 1 : 0, now);
            }
        }

        Watched = ordered;
        Changed?.Invoke();
    }

    public void SetSharing(SharingPath path, double now)
    {
        Sharing = path;
        SharingSince = now;
        Changed?.Invoke();
    }

    public void SetTracesTotal(ulong total)
    {
        TracesTotal = total;
        Changed?.Invoke();
    }

    /// <summary>A sign-in sends a ring of light out from the N on Join (JoinScreen.swift:526).</summary>
    public void SignedIn(double now)
    {
        SignedInAt = now;
        Changed?.Invoke();
    }

    /// <summary>Advance the once-only plays; call before drawing a frame.</summary>
    public void Tick(double now)
    {
        if (ScanStartedAt is double s && now - s >= FirstRunStageLayout.ScanLead + FirstRunStageLayout.ScanTurn + 0.25 && !ToolsScanned)
        {
            ToolsScanned = true;
            Changed?.Invoke();
        }

        if (ScanStartedAt is double s2 && now - s2 >= FirstRunStageLayout.ScanTotal)
        {
            ScanStartedAt = null;
            Changed?.Invoke();
        }

        if (Step == FirstRunStep.Traces && !TracesCounted && TracesGraphic.Counted(now - StepOpenedAt, ReduceMotion) >= 1)
        {
            TracesCounted = true;
        }

        if (Step == FirstRunStep.PrivateAI && !PrivateCounted)
        {
            PrivateCounted = true;
        }
    }

    public double BeamOpacity(string kind, double now)
    {
        if (!_beams.TryGetValue(kind, out var b))
        {
            return Watched.Contains(kind) ? 1 : 0;
        }

        return FirstRunMotion.BeamOpacity(b.From, b.To, now - b.At, ReduceMotion);
    }

    /// <summary>
    /// How often the stage must draw at <paramref name="now"/>, or null when
    /// nothing moves: under reduce motion, or on a step whose only motion has
    /// settled. The backdrop's breath and the N's never settle, so a shown,
    /// animating stage always draws at least 30 a second.
    /// </summary>
    public double? Interval(double now)
    {
        if (ReduceMotion)
        {
            return null;
        }

        bool inFlight = now - StepChangedAt < FirstRunStageLayout.SpatialSettle
            || _beams.Values.Any(b => now - b.At < FirstRunMotion.BeamFade)
            || (Step == FirstRunStep.Uses && SatellitesMoving(now));
        return FirstRunMotion.StageInterval(Step, FirstRunMotion.WelcomeMoving(now - WelcomeOpenedAt, false), ScanStartedAt is not null, inFlight);
    }

    private bool SatellitesMoving(double now)
    {
        double elapsed = now - SharingSince;
        for (int place = 0; place < 5; place++)
        {
            double v = UsesFlow.Satellite(Sharing, elapsed, place, false);
            if (v > 0 && Math.Abs(v - 1) > 1e-3)
            {
                return true;
            }
        }

        return false;
    }

    /// <summary>The scan's elapsed seconds while it runs and settles; null once settled.</summary>
    public double? ScanElapsed(double now) => ScanStartedAt is double s ? now - s : null;

    /// <summary>Everything the stage draws at <paramref name="now"/>, in canvas points.</summary>
    public StageSnapshot Snapshot(double now)
    {
        double since = now - StepChangedAt;
        double top = GraphicTop(Step);
        StageFrame Abs(StageFrame f, double t) => new(new StagePoint(f.Center.X, f.Center.Y + t), f.Side);
        StageFrame hubTo = Abs(StageActors.Hub(Step), top);
        StageFrame hub = PreviousStep is null ? hubTo : FirstRunMotion.Actor(_hubFrom, hubTo, since, ReduceMotion);
        var tiles = new List<StageTile>();
        foreach (var (source, exists) in Rows)
        {
            StageFrame? to = Step == FirstRunStep.Tools ? null : StageActors.Tile(Step, source, Rows, Watched, Connectable);
            if (to is StageFrame f)
            {
                StageFrame abs = Abs(f, top);
                StageFrame drawn = _tilesFrom.TryGetValue(source, out StageFrame from) && PreviousStep is not null
                    ? FirstRunMotion.Actor(from, abs, since, ReduceMotion)
                    : abs;
                tiles.Add(new StageTile(source, drawn, Watched.Contains(source)));
            }
            else if (Step == FirstRunStep.Tools)
            {
                tiles.Add(new StageTile(source, ToolsTile(source, exists, now, top), Watched.Contains(source)));
            }
        }

        return new StageSnapshot(Step, now, since, top, hub, tiles, ReduceMotion);
    }

    /// <summary>A tile on the Tools ring, drawn where the scan puts it, before the bob and the placement's scale.</summary>
    private StageFrame ToolsTile(string source, bool exists, double now, double top)
    {
        var flags = Rows.Select(r => r.Exists).ToArray();
        int index = Rows.ToList().FindIndex(r => r.Source == source);
        RingPlacement p = ToolsRing.Placement(index, flags, ScanElapsed(now));
        StagePoint c = FirstRunStageLayout.Point(new StagePoint(FirstRunStageLayout.CanvasWidth / 2, ToolsRing.CenterY + top), p.Radius, p.Angle);
        StageFrame frame = new(c, ToolsRing.Tile);
        if (_tilesFrom.TryGetValue(source, out StageFrame from) && PreviousStep is not null && ScanStartedAt is null)
        {
            frame = FirstRunMotion.Actor(from, frame, now - StepChangedAt, ReduceMotion);
        }

        return frame;
    }
}

/// <summary>A tool's tile as the stage draws it.</summary>
public readonly record struct StageTile(string Source, StageFrame Frame, bool Lit);

/// <summary>One frame of the stage, in canvas points.</summary>
public sealed record StageSnapshot(
    FirstRunStep Step, double Now, double SinceStep, double GraphicTop, StageFrame Hub, IReadOnlyList<StageTile> Tiles, bool ReduceMotion);
