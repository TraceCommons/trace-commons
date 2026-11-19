using System;
using System.Collections.Generic;
using TraceCommons.Interop.Shell;

namespace TraceCommons.Interop.FirstRun;

/// <summary>A point on the first run's canvas, in points.</summary>
public readonly record struct StagePoint(double X, double Y);

/// <summary>
/// The animated first run's stage, as numbers: a port of the Mac's
/// <c>FirstRunStageLayout</c> (macos/Sources/TraceCommonsApp/Views/FirstRun/FirstRunStage.swift:11-107,
/// :524-546), Welcome's choreography (<c>WelcomeScreenLayout</c>, WelcomeScreen.swift:10-61),
/// the token curve as the first run samples it (<c>GlassMotionCurve</c>, WelcomeScreen.swift:65-84)
/// and the springs of <c>GlassMotion</c> (TCDesign/Styling/GlassMotion.swift:37-47).
/// </summary>
/// <remarks>
/// Pure: a step, a time and whether motion is reduced go in, numbers come
/// out. The Win2D stage draws them; the tests pin them to what the Mac's own
/// Swift prints. Nothing here carries a word: the stage is decorative and
/// hidden from assistive technology, as on the Mac.
/// </remarks>
public static class FirstRunStageLayout
{
    /// <summary>The design's canvas, the Mac's first-run window size (FirstRunStage.swift:14).</summary>
    public const double CanvasWidth = 860, CanvasHeight = 760;

    /// <summary>
    /// The scale the canvas is drawn at in a host: never larger than the
    /// design (<c>FirstRunStageFit</c>, OnboardingCoordinatorView.swift:209-222).
    /// </summary>
    public static double FitScale(double width, double height) =>
        Math.Min(1, Math.Min(width / CanvasWidth, height / CanvasHeight));

    /// <summary>The step thread's position of a step; null on Welcome (FirstRunStage.swift:18-20).</summary>
    public static int? ThreadIndex(FirstRunStep step)
    {
        int i = FirstRunNavigation.ThreadSteps.IndexOf(step);
        return i < 0 ? null : i;
    }

    /// <summary>How a stretch of the thread reads (FirstRunStage.swift:24-34).</summary>
    public static ThreadSegment Segment(int before, int current) =>
        before == current ? ThreadSegment.Leading : before < current ? ThreadSegment.Travelled : ThreadSegment.Ahead;

    /// <summary>Evenly spaced from the top, clockwise, in degrees (FirstRunStage.swift:37-40).</summary>
    public static double RingAngle(int index, int count) => count <= 0 ? -90 : -90 + 360.0 / count * index;

    /// <summary>Where a watched tile circles the N on Join (FirstRunStage.swift:44-52).</summary>
    public static double JoinAngle(int index, int count) => count switch
    {
        1 => -90,
        2 => new double[] { 180, 0 }[Math.Min(index, 1)],
        3 => new double[] { -90, 30, 150 }[Math.Min(index, 2)],
        4 => new double[] { -135, -45, 45, 135 }[Math.Min(index, 3)],
        _ => RingAngle(index, count),
    };

    /// <summary>The point at <paramref name="radius"/> and <paramref name="degrees"/> round a centre (FirstRunStage.swift:55-58).</summary>
    public static StagePoint Point(StagePoint center, double radius, double degrees)
    {
        double r = degrees * Math.PI / 180;
        return new StagePoint(center.X + radius * Math.Cos(r), center.Y + radius * Math.Sin(r));
    }

    /// <summary>The Tools scan (FirstRunStage.swift:62-64).</summary>
    public const double ScanLead = 1, ScanTurn = 2, SweepStart = -36;

    /// <summary>When the sweep reaches tile <paramref name="index"/> (FirstRunStage.swift:68-72).</summary>
    public static double ScanMoment(int index, int count)
    {
        if (count <= 0)
        {
            return ScanLead;
        }

        double angle = 360.0 / count * index;
        return ScanLead + ScanTurn * (angle - SweepStart) / (360 - SweepStart);
    }

    /// <summary>Every tile has settled (FirstRunStage.swift:75-77).</summary>
    public static bool ScanDone(double elapsed) => elapsed >= ScanLead + ScanTurn + 0.25;

    /// <summary>
    /// How long the scan runs before the stage forgets it: the turn, then the
    /// 0.9 s re-spacing, rounded up to the Mac's 1 s sleep
    /// (<c>FirstRunStageMemory.startScan</c>, FirstRunStage.swift:563-571).
    /// </summary>
    public const double ScanTotal = ScanLead + ScanTurn + 0.25 + 1;

    /// <summary>The sweep's rotation, degrees clockwise from the top; null outside its turn (FirstRunStage.swift:81-85).</summary>
    public static double? SweepRotation(double elapsed)
    {
        double into = elapsed - ScanLead;
        if (into < 0 || into > ScanTurn)
        {
            return null;
        }

        return SweepStart + (360 - SweepStart) * into / ScanTurn;
    }

    /// <summary>
    /// The sweep's opacity: up over 0.4 s from the lead and down over the
    /// last 0.4 s of the turn (ToolsScreen.swift:466-468).
    /// </summary>
    public static double SweepFade(double elapsed)
    {
        double turnEnd = ScanLead + ScanTurn;
        return Math.Min(1, Math.Min(Math.Max(0, (elapsed - ScanLead) / 0.4), Math.Max(0, (turnEnd - elapsed) / 0.4)));
    }

    /// <summary>The ease-out cubic the Traces count rises along (FirstRunStage.swift:88-91).</summary>
    public static double CountEase(double progress)
    {
        double p = Math.Clamp(progress, 0, 1);
        return 1 - Math.Pow(1 - p, 3);
    }

    /// <summary>The overshooting settle the tiles burst out of the N on (FirstRunStage.swift:95-100).</summary>
    public static double Burst(double elapsed, double duration)
    {
        if (elapsed <= 0)
        {
            return 0;
        }

        if (elapsed >= duration)
        {
            return 1;
        }

        double t = elapsed / duration;
        return 1 - Math.Exp(-6 * t) * Math.Cos(7.5 * t);
    }

    /// <summary>
    /// The token curve as the first run samples it for a value: the Bezier
    /// solved for x by eight Newton steps, as <c>GlassMotionCurve.value</c>
    /// (WelcomeScreen.swift:65-84). The shine and the ring's re-spacing use
    /// it; SwiftUI's own timed animations use <see cref="Timed"/>.
    /// </summary>
    public static double Curve(double t)
    {
        double p = Math.Clamp(t, 0, 1);
        const double x1 = MonitorMotion.EaseX1, y1 = MonitorMotion.EaseY1, x2 = MonitorMotion.EaseX2, y2 = MonitorMotion.EaseY2;
        static double Bezier(double s, double a, double b) => 3 * (1 - s) * (1 - s) * s * a + 3 * (1 - s) * s * s * b + s * s * s;
        double s = p;
        for (int i = 0; i < 8; i++)
        {
            double x = Bezier(s, x1, x2) - p;
            double dx = 3 * (1 - s) * (1 - s) * x1 + 6 * (1 - s) * s * (x2 - x1) + 3 * s * s * (1 - x2);
            if (Math.Abs(dx) <= 1e-6)
            {
                break;
            }

            s = Math.Clamp(s - x / dx, 0, 1);
        }

        return Bezier(s, y1, y2);
    }

    /// <summary>
    /// <c>GlassMotion.timed(duration:)</c> (GlassMotion.swift:31-33): the
    /// token curve over <paramref name="duration"/>, as SwiftUI's
    /// <c>timingCurve</c> runs it, <paramref name="since"/> seconds after the
    /// change.
    /// </summary>
    public static double Timed(double since, double duration) => MonitorMotion.Ease(since / duration);

    /// <summary>
    /// SwiftUI's <c>Spring(duration:bounce:)</c> from 0 to 1, <paramref name="t"/>
    /// seconds in: stiffness (2 pi / duration) squared, damping ratio 1 - bounce.
    /// </summary>
    public static double Spring(double t, double duration, double bounce) =>
        MonitorMotion.Spring(duration, 1 - bounce, t);

    /// <summary><c>GlassMotion.spatial</c> (GlassMotion.swift:37-40): 0.9 s, bounce 0.04.</summary>
    public const double SpatialDuration = 0.9, SpatialBounce = 0.04;

    /// <summary><c>GlassMotion.expressive</c> (GlassMotion.swift:44-47): bounce 0.32.</summary>
    public const double ExpressiveBounce = 0.32;

    /// <summary>
    /// SwiftUI's settling time of the spatial spring at 0.9 s (1.3353 s,
    /// printed by the probe), rounded up: after it the move is drawn at rest.
    /// </summary>
    public const double SpatialSettle = 1.4;

    /// <summary>How long a bounce-0.04 spring of <paramref name="duration"/> is drawn before it is settled.</summary>
    public static double SpringSettle(double duration) => duration * SpatialSettle / SpatialDuration;

    /// <summary>The spatial spring at its default 0.9 s, settled after <see cref="SpatialSettle"/>.</summary>
    public static double Spatial(double t) => t >= SpatialSettle ? 1 : Spring(t, SpatialDuration, SpatialBounce);

    /// <summary>The order the Tools ring holds the tools (FirstRunStage.swift:533).</summary>
    public static readonly IReadOnlyList<string> RingOrder = new[] { "claude-code", "gemini-cli", "codex", "cline", "opencode" };

    /// <summary>Discovery's rows in ring order; a kind not reported is left out (FirstRunStage.swift:537-539).</summary>
    public static IReadOnlyList<T> RingRows<T>(IEnumerable<T> rows, Func<T, string> source)
    {
        var list = new List<T>(rows);
        var ordered = new List<T>();
        foreach (string kind in RingOrder)
        {
            int i = list.FindIndex(r => source(r) == kind);
            if (i >= 0)
            {
                ordered.Add(list[i]);
            }
        }

        return ordered;
    }

    /// <summary>
    /// Time held to a timeline's rate: a layer the Mac draws on a
    /// <c>GlassAnimationTimeline(minimumInterval:)</c> sees time move in
    /// steps of that interval, even while the stage draws faster for a
    /// layer beside it. Zero is the display's rate.
    /// </summary>
    public static double AtRate(double time, double interval) =>
        interval <= 0 ? time : Math.Floor(time / interval + 1e-9) * interval;
}

/// <summary>A stretch of the step thread (FirstRunStage.swift:24-28).</summary>
public enum ThreadSegment
{
    Ahead,
    Travelled,
    Leading,
}

/// <summary>Welcome's choreography (WelcomeScreen.swift:10-61).</summary>
public static class WelcomeMotion
{
    public const double DrawDelay = 0.2, DrawDuration = 1, ShineFirst = 1.5, ShineEvery = 15, ShineCrossing = 1.35;
    public const double BodyDelay = 1.45, ButtonDelay = 1.8, RiseDuration = 0.76;

    /// <summary>The words' delays (WelcomeScreen.swift:16).</summary>
    public static readonly IReadOnlyList<double> WordDelays = new[] { 1.05, 1.14, 1.23 };

    /// <summary>The hero title over the type scale's largest step (WelcomeScreen.swift:21).</summary>
    public const double HeroScale = 44.0 / 26;

    /// <summary>How far below its place a line starts (<c>Rise</c>, WelcomeScreen.swift:176).</summary>
    public const double RiseLift = 18;

    /// <summary>The N's side on Welcome and the gap under it (WelcomeScreen.swift:117-118).</summary>
    public const double Hub = 132, HubGap = 52;

    /// <summary>When the last line has risen: nothing on Welcome moves after this but the shine.</summary>
    public const double RisenBy = ButtonDelay + RiseDuration;

    public static double Drawn(double elapsed, bool reduceMotion) =>
        reduceMotion ? 1 : Ease((elapsed - DrawDelay) / DrawDuration);

    public static double? Shine(double elapsed, bool reduceMotion)
    {
        if (reduceMotion || elapsed < ShineFirst)
        {
            return null;
        }

        double into = (elapsed - ShineFirst) % ShineEvery;
        return into < ShineCrossing ? FirstRunStageLayout.Curve(into / ShineCrossing) : null;
    }

    public static double WordDelay(int index) => WordDelays[Math.Min(index, WordDelays.Count - 1)];

    public static double Risen(double elapsed, double delay, bool reduceMotion)
    {
        if (reduceMotion)
        {
            return 1;
        }

        return FirstRunStageLayout.Burst(elapsed - delay, RiseDuration) * 0.98 + (elapsed - delay >= RiseDuration ? 0.02 : 0);
    }

    /// <summary>A risen line's drawn opacity and offset (<c>Rise</c>, WelcomeScreen.swift:171-178).</summary>
    public static (double Opacity, double OffsetY) Rise(double amount) =>
        (Math.Clamp(amount, 0, 1), RiseLift * (1 - amount));

    public static double Glow(double elapsed, bool reduceMotion) => reduceMotion ? 1 : Ease((elapsed - 0.15) / 1.1);

    public static double Ease(double t)
    {
        double p = Math.Clamp(t, 0, 1);
        return p < 0.5 ? 4 * p * p * p : 1 - Math.Pow(-2 * p + 2, 3) / 2;
    }

    /// <summary>
    /// The title's words, raised one at a time (WelcomeScreen.swift:58-60):
    /// split on spaces, empty pieces dropped; each but the last is drawn with
    /// its space (:149).
    /// </summary>
    public static IReadOnlyList<string> Words(string title)
    {
        string[] words = title.Split(' ', StringSplitOptions.RemoveEmptyEntries);
        var drawn = new string[words.Length];
        for (int i = 0; i < words.Length; i++)
        {
            drawn[i] = i < words.Length - 1 ? words[i] + " " : words[i];
        }

        return drawn;
    }
}

/// <summary>
/// The NEAR mark as the first run draws it: the marker line it is inked
/// along, the pen tip and the shine band (TCDesign/Logos/GlassBrandMarkDrawing.swift:4-62, :115-138).
/// Coordinates are the brand tile's 400 by 396.
/// </summary>
public static class BrandMarkDrawing
{
    public static readonly IReadOnlyList<StagePoint> MarkerLine = new StagePoint[]
    {
        new(167.5, 234.6), new(101, 312), new(101, 86), new(299, 320), new(299, 84), new(226, 170),
    };

    public const double MarkerWidth = 44;

    public const double ShineFrom = -240, ShineTo = 470;

    public static double MarkerLength
    {
        get
        {
            double sum = 0;
            for (int i = 1; i < MarkerLine.Count; i++)
            {
                sum += Distance(MarkerLine[i - 1], MarkerLine[i]);
            }

            return sum;
        }
    }

    public static StagePoint MarkerPoint(double distance)
    {
        double left = Math.Max(0, distance);
        for (int i = 1; i < MarkerLine.Count; i++)
        {
            StagePoint from = MarkerLine[i - 1], to = MarkerLine[i];
            double length = Distance(from, to);
            if (left <= length && length > 0)
            {
                double t = left / length;
                return new StagePoint(from.X + (to.X - from.X) * t, from.Y + (to.Y - from.Y) * t);
            }

            left -= length;
        }

        return MarkerLine[^1];
    }

    /// <summary>The marker line up to <paramref name="drawn"/> of its length (<c>trimmedPath(from: 0, to: drawn)</c>).</summary>
    public static IReadOnlyList<StagePoint> MarkerTrim(double drawn)
    {
        double target = Math.Clamp(drawn, 0, 1) * MarkerLength;
        var line = new List<StagePoint> { MarkerLine[0] };
        double walked = 0;
        for (int i = 1; i < MarkerLine.Count; i++)
        {
            double length = Distance(MarkerLine[i - 1], MarkerLine[i]);
            if (walked + length >= target)
            {
                line.Add(MarkerPoint(target));
                return line;
            }

            walked += length;
            line.Add(MarkerLine[i]);
        }

        return line;
    }

    /// <summary>The band's left edge for a shine <paramref name="progress"/> (GlassBrandMarkDrawing.swift:117-118).</summary>
    public static double ShineX(double progress) => ShineFrom + (ShineTo - ShineFrom) * progress;

    /// <summary>
    /// The 110-wide band from y 60 to 340 leaning 18 degrees, its left edge
    /// at <paramref name="x"/>, and the gradient's ends square to the lean
    /// (GlassBrandMarkDrawing.swift:49-59).
    /// </summary>
    public static (StagePoint[] Corners, StagePoint From, StagePoint To) ShineBand(double x)
    {
        double angle = 18 * Math.PI / 180;
        double lean = Math.Tan(angle);
        StagePoint Skewed(double px, double py) => new(px - lean * py, py);
        StagePoint from = Skewed(x, 200);
        double across = 110 * Math.Cos(angle);
        return (
            new[] { Skewed(x, 60), Skewed(x + 110, 60), Skewed(x + 110, 340), Skewed(x, 340) },
            from,
            new StagePoint(from.X + across * Math.Cos(angle), from.Y + across * Math.Sin(angle)));
    }

    /// <summary>The shine's gradient stops: white at 0, 0.7, 0 (GlassBrandMarkDrawing.swift:123-127).</summary>
    public static readonly IReadOnlyList<(double Location, double Alpha)> ShineStops = new[] { (0.0, 0.0), (0.5, 0.7), (1.0, 0.0) };

    /// <summary>
    /// The pen tip while the mark draws: its centre in tile coordinates, its
    /// radius (10.5, scaled by the caller) and its opacity; null once drawn
    /// (GlassBrandMarkDrawing.swift:126-137). Its glow is a shadow of
    /// radius 1.6 x the tip's.
    /// </summary>
    public static (StagePoint Center, double Radius, double Opacity)? PenTip(double drawn)
    {
        if (drawn <= 0 || drawn >= 1)
        {
            return null;
        }

        double front = drawn * MarkerLength + MarkerWidth / 2 - 8;
        double fade = Math.Min(1, Math.Min(drawn / 0.05, (1 - drawn) / 0.08));
        return (MarkerPoint(front), 10.5, Math.Max(0, fade));
    }

    public const double PenGlow = 1.6;

    private static double Distance(StagePoint a, StagePoint b) => Math.Sqrt((b.X - a.X) * (b.X - a.X) + (b.Y - a.Y) * (b.Y - a.Y));
}

/// <summary>The scene and the N's glow (FirstRunStage.swift:109-145, :233-285).</summary>
public static class SceneMotion
{
    /// <summary>The breathing glow behind the N: its scale and opacity (FirstRunStage.swift:275-284).</summary>
    public static (double Scale, double Opacity) Breathing(double time, bool charged, bool reduceMotion)
    {
        if (reduceMotion)
        {
            return (1, 1);
        }

        if (charged)
        {
            double c = 0.5 - 0.5 * Math.Cos(2 * Math.PI * time / 1.8);
            return (1.02 + 0.12 * c, 0.9 + 0.1 * c);
        }

        double p = 0.5 - 0.5 * Math.Cos(2 * Math.PI * time / 4);
        return (1 + 0.08 * p, 1 - 0.25 * p);
    }

    /// <summary>The corner glow's breath: seven seconds each way (FirstRunStage.swift:132-133).</summary>
    public static double BackdropPhase(double time, bool reduceMotion) =>
        reduceMotion ? 1 : 0.5 - 0.5 * Math.Cos(Math.PI * time / 7);

    /// <summary>The breathing ellipse's frame, offset, blur and the phase's scale and opacity (FirstRunStage.swift:134-142).</summary>
    public const double BreathWidth = 640, BreathHeight = 580, BreathOffset = -140, BreathBlur = 30, BreathRadius = 300;

    public static double BreathScale(double phase) => 0.9 + 0.25 * phase;

    public static double BreathOpacity(double phase) => 0.6 + 0.4 * phase;

    /// <summary>The glow strengths, dark only: the first run is dark (FirstRunStage.swift:116-117; Ron, 2026-11-05).</summary>
    public const double GlowStrong = 0.5, GlowSoft = 0.14;

    /// <summary>The N's glow disc: 220/132 of the mark's side, its gradient to 0.31 of that, 0.35 alpha in dark (FirstRunStage.swift:249-260).</summary>
    public const double DiscPerSide = 220.0 / 132, DiscReach = 0.31, DiscAlpha = 0.35;
}

/// <summary>The Tools ring (ToolsScreen.swift:345-410, :411-587).</summary>
public static class ToolsRing
{
    public const double Height = 300, CenterY = 158, LooseRadius = 130, TightRadius = 118, Tile = 52, Hub = 64, GoneRadius = 470;

    /// <summary>Where a tile is as the scan runs, or settled when <paramref name="elapsed"/> is null (ToolsScreen.swift:365-391).</summary>
    public static RingPlacement Placement(int index, IReadOnlyList<bool> exists, double? elapsed)
    {
        var found = new List<int>();
        for (int i = 0; i < exists.Count; i++)
        {
            if (exists[i])
            {
                found.Add(i);
            }
        }

        double start = FirstRunStageLayout.RingAngle(index, exists.Count);
        bool gone = !exists[index];
        int at = found.IndexOf(index);
        double settled = at >= 0 ? FirstRunStageLayout.RingAngle(at, found.Count) : start;
        if (elapsed is not double e)
        {
            return gone ? new RingPlacement(start, GoneRadius, 0.6, 0) : new RingPlacement(settled, TightRadius, 1, 1);
        }

        double burst = FirstRunStageLayout.Burst(e - (0.26 + 0.08 * index), 0.78);
        double angle = start, radius = LooseRadius * burst, scale = 0.3 + 0.7 * Math.Min(1, burst), opacity = Math.Min(1, burst * 1.6);
        if (gone)
        {
            double leaving = e - (FirstRunStageLayout.ScanMoment(index, exists.Count) + 0.12);
            double exit = Math.Pow(Math.Clamp(leaving / 0.8, 0, 1), 2);
            radius += (GoneRadius - LooseRadius) * exit;
            scale *= 1 - 0.4 * exit;
            opacity *= 1 - exit;
        }
        else if (FirstRunStageLayout.ScanDone(e))
        {
            double respace = FirstRunStageLayout.Curve((e - FirstRunStageLayout.ScanLead - FirstRunStageLayout.ScanTurn - 0.25) / 0.9);
            angle = start + (settled - start) * respace;
            radius = LooseRadius + (TightRadius - LooseRadius) * respace;
        }

        return new RingPlacement(angle, radius, scale, opacity);
    }

    /// <summary>The flash as the sweep reaches a tile: 0 to 1 over 0.52 s (ToolsScreen.swift:395-399).</summary>
    public static double? Flash(int index, int count, double? elapsed)
    {
        if (elapsed is not double e)
        {
            return null;
        }

        double into = (e - (FirstRunStageLayout.ScanMoment(index, count) - 0.06)) / 0.52;
        return into >= 0 && into <= 1 ? into : null;
    }

    /// <summary>The flash ring's scale and opacity (ToolsScreen.swift:509-512).</summary>
    public static (double Scale, double Opacity) FlashLook(double flash) =>
        (0.88 + 0.28 * flash, flash < 0.4 ? flash / 0.4 : (1 - flash) / 0.6);

    /// <summary>A tile's bob, each tool on its own phase (ToolsScreen.swift:402-405).</summary>
    public static double Bob(string kind, double time)
    {
        double[] phases = { 0, 1.1, 2.3, 3.4, 4.2 };
        int i = Math.Max(0, IndexOf(FirstRunStageLayout.RingOrder, kind));
        return -2 + 2 * Math.Cos(2 * Math.PI * (time + phases[i]) / 5);
    }

    /// <summary>The dashed ring's slow turn: once per 90 s (ToolsScreen.swift:463).</summary>
    public static double Turn(double time, bool reduceMotion) => reduceMotion ? 0 : (time / 90 % 1) * 360;

    /// <summary>The dashed ring's dash and line (ToolsScreen.swift:461).</summary>
    public static readonly double[] RingDash = { 3, 7 };

    public const double RingLine = 1.5;

    /// <summary>The ring fades in from 0.3 s over 0.7 s of the scan (ToolsScreen.swift:466).</summary>
    public static double RingOpacity(double? elapsed) => Math.Clamp(((elapsed ?? 10) - 0.3) / 0.7, 0, 1);

    /// <summary>The sweep's conic stops: location (degrees from its start, of 360), colour and alpha (ToolsScreen.swift:476-485).</summary>
    public static readonly IReadOnlyList<(double Location, bool Beam, double Alpha)> SweepStops = new[]
    {
        (0.0, false, 0.0), (140.0 / 360, false, 0.0), (210.0 / 360, false, 0.03), (280.0 / 360, false, 0.11),
        (330.0 / 360, false, 0.24), (352.0 / 360, true, 0.4), (358.0 / 360, true, 0.5), (1.0, false, 0.0),
    };

    /// <summary>The beam's ends: from radius 90 at the tile's settled angle into radius 16 (ToolsScreen.swift:433-435).</summary>
    public const double BeamFrom = 90, BeamTo = 16;

    /// <summary>The beam's breath, period 2.2 s (ToolsScreen.swift:552).</summary>
    public static double BeamBreath(double time, bool reduceMotion) => reduceMotion ? 0 : 0.5 - 0.5 * Math.Cos(2 * Math.PI * time / 2.2);

    /// <summary>
    /// How far the pulses have flowed: 26.1 points per 1.1 s
    /// (ToolsScreen.swift:553). The stroke's dash phase is minus this
    /// (<c>dashPhase: -flow</c>, :574), so the pulses travel from the tile
    /// into the N; see <see cref="DashFlow"/>.
    /// </summary>
    public static double BeamFlow(double time, bool reduceMotion) => reduceMotion ? 0 : (time / 1.1 % 1) * 26.1;

    /// <summary>The pulses' CoreGraphics dash phase: <c>dashPhase: -flow</c> (ToolsScreen.swift:574).</summary>
    public static double BeamDashPhase(double time, bool reduceMotion) => -BeamFlow(time, reduceMotion);

    /// <summary>The pulses' dash and width (ToolsScreen.swift:574).</summary>
    public static readonly double[] BeamDash = { 0.1, 26 };

    public const double BeamPulseWidth = 8;

    private static int IndexOf(IReadOnlyList<string> list, string value)
    {
        for (int i = 0; i < list.Count; i++)
        {
            if (list[i] == value)
            {
                return i;
            }
        }

        return -1;
    }
}

/// <summary>A tile's place on the Tools ring.</summary>
public readonly record struct RingPlacement(double Angle, double Radius, double Scale, double Opacity);

/// <summary>
/// A dashed stroke's pattern moving along its path. CoreGraphics and
/// Direct2D read a dash phase the same way: a positive phase starts the
/// pattern that far into itself, so the dashes sit further back along the
/// path; the Mac's <c>dashPhase: -flow</c> moves them forward, from the
/// path's start to its end. Direct2D takes the phase and the dashes in units
/// of the stroke's width (<c>D2D1_STROKE_STYLE_PROPERTIES.dashOffset</c>).
/// </summary>
public static class DashFlow
{
    /// <summary>
    /// Where each "on" dash starts along a path of <paramref name="length"/>
    /// under a CoreGraphics dash phase of <paramref name="phase"/>.
    /// </summary>
    public static IReadOnlyList<double> DashStarts(IReadOnlyList<double> dash, double phase, double length)
    {
        double period = 0;
        foreach (double d in dash)
        {
            period += d;
        }

        var starts = new List<double>();
        if (period <= 0)
        {
            return starts;
        }

        // The pattern begins `phase` into itself at the path's start, so the
        // first period's "on" dash sits at -phase, wrapped into one period.
        double first = -(phase % period);
        if (first > 0)
        {
            first -= period;
        }

        for (double s = first; s < length; s += period)
        {
            if (s + dash[0] > 0)
            {
                starts.Add(Math.Max(0, s));
            }
        }

        return starts;
    }

    /// <summary>The Direct2D dash offset, in stroke widths, for a CoreGraphics phase.</summary>
    public static float Direct2DOffset(double phase, double strokeWidth) => (float)(phase / strokeWidth);

    /// <summary>The Direct2D dashes, in stroke widths, for CoreGraphics dashes in points.</summary>
    public static float[] Direct2DDashes(IReadOnlyList<double> dash, double strokeWidth)
    {
        var scaled = new float[dash.Count];
        for (int i = 0; i < dash.Count; i++)
        {
            scaled[i] = (float)(dash[i] / strokeWidth);
        }

        return scaled;
    }
}

/// <summary>The Private AI diagram (PrivateAIScreen.swift:7-49, :97-131).</summary>
public static class PrivateAIDiagram
{
    public const double Height = 221, HubX = 440, HubY = 101, HubSide = 66, TileX = 230, Tile = 42, RouteStart = 254, PillWidth = 88, PillHeight = 24, DestinationX = 650;

    /// <summary>The tools Settings can connect (PrivateAIScreen.swift:10).</summary>
    public static readonly IReadOnlyList<string> Routable = new[] { "claude-code", "codex", "opencode" };

    public static double RowY(int index, int count) => count <= 1 ? 101 : 21 + 160.0 * index / (count - 1);

    /// <summary>The figure counts up from 0.9 s, 0.24 s apart (PrivateAIScreen.swift:28-32).</summary>
    public static int Counted(int count, double elapsed, bool reduceMotion)
    {
        if (reduceMotion)
        {
            return count;
        }

        if (elapsed < 0.9)
        {
            return 0;
        }

        return Math.Min(count, 1 + (int)((elapsed - 0.9) / 0.24));
    }

    /// <summary>The dashes' phase: -22 points per 1.1 s (PrivateAIScreen.swift:97, :111).</summary>
    public static double DashPhase(double time, bool reduceMotion) => reduceMotion ? 0 : -22 * (time / 1.1 % 1);

    public static readonly double[] Dash = { 5, 6 };

    public const double Line = 2;
}

/// <summary>The Traces graphic (TracesScreen.swift:91-122, :160-256).</summary>
public static class TracesGraphic
{
    public const double Height = 228, CenterX = 557, CenterY = 114, Ring = 220, RingLine = 8, Hub = 36, Tile = 36, TileX = 207, LabelX = 237, LabelWidth = 110, RowPitch = 62;
    public const double CountDelay = 0.9, CountDuration = 1.9;

    public static double Counted(double elapsed, bool reduceMotion) =>
        reduceMotion ? 1 : FirstRunStageLayout.CountEase((elapsed - CountDelay) / CountDuration);

    public static double RowY(int index, int count, double center) => center - (count - 1) * RowPitch / 2 + index * RowPitch;

    public static (StagePoint, StagePoint, StagePoint, StagePoint) Stream(StagePoint start, StagePoint end) =>
        (start, new StagePoint(start.X + 52, start.Y), new StagePoint(end.X - 36, end.Y), end);

    public static StagePoint Bezier((StagePoint, StagePoint, StagePoint, StagePoint) c, double t)
    {
        double u = 1 - t;
        double a = u * u * u, b = 3 * u * u * t, cc = 3 * u * t * t, d = t * t * t;
        return new StagePoint(
            a * c.Item1.X + b * c.Item2.X + cc * c.Item3.X + d * c.Item4.X,
            a * c.Item1.Y + b * c.Item2.Y + cc * c.Item3.Y + d * c.Item4.Y);
    }

    /// <summary>Whether the orbs flow (TracesScreen.swift:240).</summary>
    public static bool Flowing(double elapsed, bool reduceMotion) =>
        !reduceMotion && elapsed >= 0.5 && elapsed < CountDelay + CountDuration + 0.4;

    /// <summary>
    /// Orb <paramref name="orb"/> (of three) on row <paramref name="row"/>:
    /// its eased place on the stream and its opacity, or null before it
    /// starts (TracesScreen.swift:246-255).
    /// </summary>
    public static (double Along, double Opacity)? Orb(int orb, int row, double elapsed)
    {
        double into = (elapsed - 0.5 - orb * 0.46 - row * 0.15) / 1.38;
        if (into < 0)
        {
            return null;
        }

        double t = into % 1;
        return (FirstRunStageLayout.Curve(t), t < 0.12 ? t / 0.12 : t > 0.88 ? (1 - t) / 0.12 : 1);
    }
}

/// <summary>The Uses flow (UsesScreen.swift:320-421, :422-600).</summary>
public static class UsesFlow
{
    public const double LineStart = 270, LineEnd = 600, LineY = 46, Height = 112, MacX = 210, GateX = 430, CommonsX = 650, CaptionY = 101, CaptionWidth = 160, Hub = 52, Gate = 58, Satellites = 92;

    public static double Period(SharingPath path) => path == SharingPath.Automatic ? 2.6 : 4.4;

    public static double Delay(SharingPath path) => path == SharingPath.Automatic ? 0.6 : 1;

    public static double Arrival(SharingPath path) => path == SharingPath.Automatic ? 0.92 : 0.93;

    public static double? Loop(SharingPath path, double elapsed)
    {
        double into = elapsed - Delay(path);
        return into < 0 ? null : into / Period(path) % 1;
    }

    public static (double Along, double Opacity) Orb(SharingPath path, double t) => path == SharingPath.AskMe
        ? (Keyframes(t, (0, 0), (0.32, 0.397), (0.64, 0.397), (0.71, 0.573), (0.93, 1), (1, 1)),
            Keyframes(t, (0, 0), (0.05, 1), (0.93, 1), (1, 0)))
        : (Keyframes(t, (0, 0), (0.92, 1), (1, 1)),
            Keyframes(t, (0, 0), (0.08, 1), (0.92, 1), (1, 0)));

    public static UsesGate GateLook(SharingPath path, double t) => path == SharingPath.AskMe
        ? new UsesGate(
            Keyframes(t, (0, 0), (0.33, 0), (0.37, 1), (0.6, 1), (0.68, 0), (1, 0)),
            Keyframes(t, (0, 0.86), (0.33, 0.86), (0.36, 1.34), (0.4, 0.94), (0.44, 1.08), (0.48, 1.02), (0.6, 1.02), (0.68, 0.86), (1, 0.86)),
            Keyframes(t, (0, 0), (0.33, 0), (0.36, -18), (0.4, 9), (0.44, -4), (0.48, 0), (1, 0)),
            Keyframes(t, (0, 0), (0.33, 0), (0.36, 1), (0.6, 1), (0.68, 0), (1, 0)))
        : new UsesGate(
            Keyframes(t, (0, 0), (0.39, 0), (0.42, 1), (0.54, 1), (0.62, 0), (1, 0)),
            Keyframes(t, (0, 0.86), (0.39, 0.86), (0.42, 1.3), (0.45, 0.95), (0.48, 1.02), (0.54, 1.02), (0.62, 0.86), (1, 0.86)),
            Keyframes(t, (0, 0), (0.39, 0), (0.42, -16), (0.45, 8), (0.48, 0), (1, 0)),
            Keyframes(t, (0, 0), (0.39, 0), (0.42, 1), (0.54, 1), (0.62, 0), (1, 0)));

    public static (double Scale, double Opacity) Pulse(SharingPath path, double t)
    {
        double at = path == SharingPath.Automatic ? 0.9 : 0.92;
        double peak = path == SharingPath.Automatic ? 0.93 : 0.94;
        return (Keyframes(t, (0, 0.7), (at, 0.7), (peak, 0.75), (1, 1.45)), Keyframes(t, (0, 0), (at, 0), (peak, 0.9), (1, 0)));
    }

    /// <summary>How many of the five places are filled (UsesScreen.swift:398-406).</summary>
    public static int SatellitesFilled(SharingPath path, double elapsed)
    {
        double first = Delay(path) + Period(path) * Arrival(path);
        if (elapsed < first)
        {
            return 0;
        }

        double since = elapsed - first;
        int index = (int)(since / Period(path));
        int filled = index % 5 + 1;
        return filled == 5 && since - index * Period(path) >= 1.3 ? 0 : filled;
    }

    /// <summary>
    /// The commons ring's turn: once per 40 s from when the path was chosen
    /// (UsesScreen.swift:531).
    /// </summary>
    public static double CommonsTurn(double elapsed, bool reduceMotion) => reduceMotion ? 0 : (elapsed / 40 % 1) * 360;

    /// <summary>The satellites pop on <c>expressive(duration: 0.46)</c> (UsesScreen.swift:578-582).</summary>
    public const double SatelliteDuration = 0.46;

    /// <summary>The expressive spring at 0.46 s settles in 0.995 s (probe); drawn to rest after 1 s.</summary>
    public const double SatelliteSettle = 1.0;

    /// <summary>
    /// Satellite <paramref name="place"/>'s scale (and opacity, up to 1),
    /// <paramref name="elapsed"/> seconds into the flow: it springs in as its
    /// orb arrives and out when the ring clears. Under reduce motion the Mac
    /// fills none (UsesScreen.swift:529).
    /// </summary>
    public static double Satellite(SharingPath path, double elapsed, int place, bool reduceMotion)
    {
        if (reduceMotion)
        {
            return 0;
        }

        double first = Delay(path) + Period(path) * Arrival(path);
        if (elapsed < first)
        {
            return 0;
        }

        int cycle = (int)((elapsed - first) / Period(path)) / 5;
        double Pop(double since) => FirstRunStageLayout.Spring(since, SatelliteDuration, FirstRunStageLayout.ExpressiveBounce);
        double ClearedAt(int c) => first + Period(path) * (5 * c + 4) + 1.3;
        bool shown = place < SatellitesFilled(path, elapsed);
        double filledAt = first + Period(path) * (5 * cycle + place);
        if (shown)
        {
            return Pop(elapsed - filledAt);
        }

        if (elapsed >= ClearedAt(cycle))
        {
            return 1 - Pop(elapsed - ClearedAt(cycle));
        }

        return cycle > 0 ? 1 - Pop(elapsed - ClearedAt(cycle - 1)) : 0;
    }

    /// <summary>Linear interpolation through (time, value) points (UsesScreen.swift:409-417).</summary>
    public static double Keyframes(double t, params (double Time, double Value)[] points)
    {
        if (points.Length == 0)
        {
            return 0;
        }

        if (t <= points[0].Time)
        {
            return points[0].Value;
        }

        for (int i = 1; i < points.Length; i++)
        {
            var (a, b) = (points[i - 1], points[i]);
            if (t <= b.Time)
            {
                double span = b.Time - a.Time;
                return span > 0 ? a.Value + (b.Value - a.Value) * (t - a.Time) / span : b.Value;
            }
        }

        return points[^1].Value;
    }

    /// <summary>The tiles folded into This computer: their spread and place (UsesScreen.swift:487-493).</summary>
    public static double TileX(int index, int count) => MacX + (index - (count - 1) / 2.0) * (count > 3 ? 19 : 28);

    public const double TileY = LineY - 8, TileSide = 24;
}

/// <summary>The Uses gate's look.</summary>
public readonly record struct UsesGate(double Fill, double Scale, double Turn, double White)
{
    public static readonly UsesGate Rest = new(0, 1, 0, 0);
}

/// <summary>Join's orbit (JoinScreen.swift:521-560).</summary>
public static class JoinOrbit
{
    public const double Box = 166, Hub = 56, Ring = 140, TileRadius = 70, Tile = 26;

    public static readonly double[] RingDash = { 3, 4 };

    /// <summary>The orbit ring's turn: once per 90 s (JoinScreen.swift:531).</summary>
    public static double Turn(double time, bool reduceMotion) => reduceMotion ? 0 : (time / 90 % 1) * 360;

    /// <summary>A sign-in's ring of light over 1.6 s: scale and opacity (JoinScreen.swift:526, :532-544).</summary>
    public static (double Scale, double Opacity)? Pulse(double since, bool reduceMotion)
    {
        double p = since / 1.6;
        if (reduceMotion || p < 0 || p >= 1)
        {
            return null;
        }

        return (0.3 + 1.6 * p, p < 0.18 ? p / 0.18 : (1 - p) / 0.82);
    }
}

/// <summary>Terms' N above its title (TermsScreen.swift).</summary>
public static class TermsGraphic
{
    public const double Hub = 56;
}

/// <summary>
/// Where each step draws its actors, the N and the tool tiles, settled, in
/// the step graphic's coordinates (x across the 860-wide canvas, y down the
/// graphic): the frames <c>firstRunActor</c>'s matched geometry moves them
/// between (FirstRunStage.swift:511-521). From WelcomeScreen.swift:117-118,
/// ToolsScreen.swift:446-449, PrivateAIScreen.swift:100-125,
/// TracesScreen.swift:167-198, UsesScreen.swift:487-493, :527-534,
/// TermsScreen.swift:27-30 and JoinScreen.swift:545-555.
/// </summary>
public static class StageActors
{
    /// <summary>The step graphic's height.</summary>
    public static double GraphicHeight(FirstRunStep step) => step switch
    {
        FirstRunStep.Welcome => WelcomeMotion.Hub,
        FirstRunStep.Tools => ToolsRing.Height,
        FirstRunStep.PrivateAI => PrivateAIDiagram.Height,
        FirstRunStep.Traces => TracesGraphic.Height,
        FirstRunStep.Uses => UsesFlow.Height,
        FirstRunStep.Terms => TermsGraphic.Hub,
        _ => JoinOrbit.Box,
    };

    /// <summary>Where the N rests on <paramref name="step"/>.</summary>
    public static StageFrame Hub(FirstRunStep step)
    {
        double mid = FirstRunStageLayout.CanvasWidth / 2;
        return step switch
        {
            FirstRunStep.Welcome => new(new StagePoint(mid, WelcomeMotion.Hub / 2), WelcomeMotion.Hub),
            FirstRunStep.Tools => new(new StagePoint(mid, ToolsRing.CenterY), ToolsRing.Hub),
            FirstRunStep.PrivateAI => new(new StagePoint(PrivateAIDiagram.HubX, PrivateAIDiagram.HubY), PrivateAIDiagram.HubSide),
            FirstRunStep.Traces => new(new StagePoint(TracesGraphic.CenterX, TracesGraphic.CenterY - 52), TracesGraphic.Hub),
            FirstRunStep.Uses => new(new StagePoint(UsesFlow.CommonsX, UsesFlow.LineY), UsesFlow.Hub),
            FirstRunStep.Terms => new(new StagePoint(mid, TermsGraphic.Hub / 2), TermsGraphic.Hub),
            _ => new(new StagePoint(mid, JoinOrbit.Box / 2), JoinOrbit.Hub),
        };
    }

    /// <summary>
    /// Where tool <paramref name="kind"/>'s tile rests on <paramref name="step"/>,
    /// or null where the step shows none for it. <paramref name="rows"/> are
    /// discovery's rows in ring order; <paramref name="watched"/> the watched
    /// tools in ring order; <paramref name="connectable"/> Private AI's.
    /// </summary>
    public static StageFrame? Tile(
        FirstRunStep step, string kind, IReadOnlyList<(string Source, bool Exists)> rows,
        IReadOnlyList<string> watched, IReadOnlyList<string> connectable)
    {
        double mid = FirstRunStageLayout.CanvasWidth / 2;
        switch (step)
        {
            case FirstRunStep.Tools:
                {
                    int index = -1;
                    var exists = new bool[rows.Count];
                    for (int i = 0; i < rows.Count; i++)
                    {
                        exists[i] = rows[i].Exists;
                        if (rows[i].Source == kind)
                        {
                            index = i;
                        }
                    }

                    if (index < 0)
                    {
                        return null;
                    }

                    RingPlacement place = ToolsRing.Placement(index, exists, null);
                    if (place.Opacity <= 0)
                    {
                        return null;
                    }

                    return new StageFrame(
                        FirstRunStageLayout.Point(new StagePoint(mid, ToolsRing.CenterY), place.Radius, place.Angle), ToolsRing.Tile * place.Scale);
                }

            case FirstRunStep.PrivateAI:
                {
                    int i = IndexOf(connectable, kind);
                    return i < 0 ? null : new StageFrame(new StagePoint(PrivateAIDiagram.TileX, PrivateAIDiagram.RowY(i, connectable.Count)), PrivateAIDiagram.Tile);
                }

            case FirstRunStep.Traces:
                {
                    int i = IndexOf(watched, kind);
                    return i < 0 ? null : new StageFrame(new StagePoint(TracesGraphic.TileX, TracesGraphic.RowY(i, watched.Count, TracesGraphic.CenterY)), TracesGraphic.Tile);
                }

            case FirstRunStep.Uses:
                {
                    int i = IndexOf(watched, kind);
                    return i < 0 ? null : new StageFrame(new StagePoint(UsesFlow.TileX(i, watched.Count), UsesFlow.TileY), UsesFlow.TileSide);
                }

            case FirstRunStep.Join:
                {
                    int i = IndexOf(watched, kind);
                    if (i < 0)
                    {
                        return null;
                    }

                    StagePoint at = FirstRunStageLayout.Point(new StagePoint(mid, JoinOrbit.Box / 2), JoinOrbit.TileRadius, FirstRunStageLayout.JoinAngle(i, watched.Count));
                    return new StageFrame(at, JoinOrbit.Tile);
                }

            default:
                return null;
        }
    }

    private static int IndexOf(IReadOnlyList<string> list, string value)
    {
        for (int i = 0; i < list.Count; i++)
        {
            if (list[i] == value)
            {
                return i;
            }
        }

        return -1;
    }
}
