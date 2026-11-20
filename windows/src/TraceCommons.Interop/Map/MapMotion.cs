using System;
using System.Collections.Generic;
using TraceCommons.Interop.Shell;

namespace TraceCommons.Interop.Map;

/// <summary>A timing curve or a spring, as the Mac animates a value.</summary>
public abstract record MotionCurve
{
    /// <summary>The value, 0 to 1 (a spring may overshoot), <paramref name="seconds"/> in.</summary>
    public abstract double At(double seconds);

    /// <summary>How long it takes; a spring's settling time.</summary>
    public abstract double Duration { get; }
}

/// <summary>A cubic Bezier timing curve over a duration (SwiftUI <c>timingCurve</c>, <c>UnitCurve.bezier</c>).</summary>
public sealed record BezierCurve(double X1, double Y1, double X2, double Y2, double Seconds) : MotionCurve
{
    public override double At(double seconds) => MonitorMotion.CubicBezier(X1, Y1, X2, Y2, Math.Clamp(seconds / Seconds, 0, 1));

    public override double Duration => Seconds;

    /// <summary>SwiftUI's <c>easeOut</c>: (0, 0, 0.58, 1).</summary>
    public static BezierCurve EaseOut(double seconds) => new(0, 0, 0.58, 1, seconds);

    /// <summary>SwiftUI's <c>easeInOut</c>: (0.42, 0, 0.58, 1).</summary>
    public static BezierCurve EaseInOut(double seconds) => new(0.42, 0, 0.58, 1, seconds);

    /// <summary>The glass token curve (0.2, 0.8, 0.2, 1) (GlassTokens.swift:698-701).</summary>
    public static BezierCurve Token(double seconds) => new(MonitorMotion.EaseX1, MonitorMotion.EaseY1, MonitorMotion.EaseX2, MonitorMotion.EaseY2, seconds);
}

/// <summary>SwiftUI's <c>.spring(response:dampingFraction:)</c> (MonitorMotion.Spring).</summary>
public sealed record SpringCurve(double Response, double Damping) : MotionCurve
{
    public override double At(double seconds) => MonitorMotion.Spring(Response, Damping, seconds);

    private double? _duration;

    /// <summary>Until it stays within 0.1% of its target, sampled at 60 per second; worked out once.</summary>
    public override double Duration => _duration ??= Settle();

    private double Settle()
    {
        double last = 0;
        for (int frame = 0; frame < 600; frame++)
        {
            double t = frame / 60.0;
            if (Math.Abs(At(t) - 1) > 0.001)
            {
                last = t;
            }
        }

        return last + 1.0 / 60;
    }
}

/// <summary>One step of a staged hover: which part, after how long, on what curve.</summary>
public readonly record struct PileStage(PilePart Part, double Delay, MotionCurve Curve);

/// <summary>The tool pile's three staged parts (MapToolPile.swift:26-28).</summary>
public enum PilePart
{
    Tall,
    Wide,
    Names,
}

/// <summary>
/// Every animation the map's slices own, with the Mac's curve, duration,
/// spring, frame rate and direction (spec "Animation coverage", rows for W2's
/// deferred map and graph motion, W3, W4 and W6). Each is a value a test
/// pins and the Win2D map or the XAML reads; nothing here uses a WinUI
/// default timing.
/// </summary>
public static class MapMotion
{
    // Arcs (FlowMapView.swift:80-84, :239-298).

    /// <summary>The flowing dash moves 18 design points a second (:84).</summary>
    public const double DashSpeed = 18;

    /// <summary>The arcs tick at 30 frames a second while dashes move (:280-285).</summary>
    public const double ArcFrameInterval = 1.0 / 30;

    /// <summary>
    /// The flowing dash's phase at <paramref name="seconds"/>, in the Mac's
    /// CoreGraphics convention: <c>dashPhase: -phase * scale</c> (:294), so it
    /// grows more negative with time and the dashes travel from the arc's
    /// start toward its end. Zero under reduce motion.
    /// </summary>
    public static double DashPhase(double seconds, double scale, bool reduceMotion) =>
        reduceMotion ? 0 : -(seconds * DashSpeed) * scale;

    /// <summary>
    /// The phase as Direct2D's dash offset, which counts in stroke widths and
    /// shifts the pattern the same way CoreGraphics' phase does: the phase
    /// over the stroke's width.
    /// </summary>
    public static double DashOffsetInWidths(double phase, double strokeWidth) => phase / strokeWidth;

    /// <summary>
    /// How the map's resting clock runs (:280-285): 30 a second while dashes
    /// move, the library's 20 while only it turns, paused otherwise.
    /// </summary>
    public static (double Interval, bool Paused) Resting(bool reduceMotion, bool flows, bool healthy, bool library)
    {
        bool dashes = !reduceMotion && flows && healthy;
        bool turns = !reduceMotion && library;
        return (dashes ? ArcFrameInterval : LibraryStation.FrameInterval, !(dashes || turns));
    }

    /// <summary>The transfer layer and the pile tick at 60 a second while a run moves (:107, :140).</summary>
    public const double TransferFrameInterval = 1.0 / 60;

    /// <summary>
    /// The map's one clock: the interval its fastest moving part needs now,
    /// or null when it must not tick at all (hidden, minimised, unloaded,
    /// its pane closed, or motion off), in which case only a model change
    /// redraws it. With nothing moving, it waits for the next shimmer pass
    /// or the next card's expiry.
    /// </summary>
    public static double? ClockInterval(MapClockInputs i)
    {
        if (!ClockRuns(i.AnimationsEnabled, i.Loaded, i.WindowVisible) || !i.PaneShown)
        {
            return null;
        }

        bool shimmer = MapField.UntilNextPass(i.SecondsShown) == 0;
        if (i.TransferPlaying || i.PileMoving || i.LightFading || shimmer)
        {
            return TransferFrameInterval;
        }

        (double interval, bool paused) = Resting(false, i.Flows, i.Healthy, i.HasLibrary);
        if (!paused)
        {
            return interval;
        }

        double wait = MapField.UntilNextPass(i.SecondsShown);
        if (i.NextDeadlineIn is { } deadline)
        {
            wait = Math.Min(wait, Math.Max(0, deadline));
        }

        return Math.Max(wait, TransferFrameInterval);
    }

    /// <summary>
    /// Whether a tick draws the main layer (arcs, station, transfers): every
    /// tick while a transfer, the pile or the pointer's light moves;
    /// otherwise at the layer's own resting rate (30 a second for the arcs,
    /// 20 for the station), so a field pass, which ticks the clock at 60, does
    /// not draw them three times as often (checklist O3). A clock that only
    /// waits draws when it fires, as before.
    /// </summary>
    public static bool MainLayerDue(MapClockInputs i, double sinceLastDraw)
    {
        if (i.TransferPlaying || i.PileMoving || i.LightFading)
        {
            return true;
        }

        (double interval, bool paused) = Resting(false, i.Flows, i.Healthy, i.HasLibrary);
        return paused || sinceLastDraw >= interval - TransferFrameInterval / 2;
    }

    /// <summary>
    /// Whether any map clock may tick: only while the map is loaded and its
    /// window can be seen (<c>GlassAnimationGate.paused</c>, AnimationTimeline.swift:84-87).
    /// </summary>
    public static bool ClockRuns(bool wanted, bool loaded, bool windowVisible) => wanted && loaded && windowVisible;

    /// <summary>
    /// Whether the still layer (field, nodes, a resting pile) is drawn again
    /// on this tick: while the field's shimmer or pointer light moves, the
    /// frame after it stops, and when a pile crosses between the layers. A
    /// scene with no pile (the Private AI view) never crosses.
    /// </summary>
    public static bool StillRedraws(bool fieldMoves, bool fieldMoved, bool hasPile, bool pileMoving, bool pileInStill) =>
        fieldMoves || fieldMoved || (hasPile && pileMoving == pileInStill);

    /// <summary>
    /// Whether the running clock must be started again to take a new
    /// interval. A DispatcherQueueTimer keeps the wait it was started with
    /// when its interval changes, so a clock waiting ~30 s for the next
    /// shimmer on the Private AI view left the Traces view's arcs and station
    /// still for that long after switching back.
    /// </summary>
    public static bool ClockRestarts(bool running, double currentInterval, double wantedInterval) =>
        !running || Math.Abs(currentInterval - wantedInterval) > 1e-6;

    // The tool pile (MapToolPile.swift:137-199).

    /// <summary>A coin moving to a new place in the pile (:139).</summary>
    public static readonly SpringCurve CoinShuffle = new(0.42, 0.8);

    /// <summary>The coins opening or closing with the pile (:140).</summary>
    public static readonly SpringCurve CoinOpen = new(0.36, 0.82);

    /// <summary>A coin growing as a trace arrives in it, to 1.3 times (:136, :141).</summary>
    public static readonly SpringCurve CoinGrow = new(0.3, 0.7);

    public const double CoinGrown = 1.3;

    /// <summary>The coin's glow radius while grown, times the map scale (:137).</summary>
    public const double CoinGlowRadius = 9;

    /// <summary>The pile opening under the pointer: tall, wide after 260 ms, names after 200 ms more (:177-188).</summary>
    public static readonly IReadOnlyList<PileStage> PileOpen = new[]
    {
        new PileStage(PilePart.Tall, 0, new SpringCurve(0.34, 0.84)),
        new PileStage(PilePart.Wide, 0.26, new SpringCurve(0.3, 0.86)),
        new PileStage(PilePart.Names, 0.46, BezierCurve.EaseOut(0.18)),
    };

    /// <summary>Leaving reverses it: names, wide after 100 ms, tall after 220 ms more (:189-198).</summary>
    public static readonly IReadOnlyList<PileStage> PileClose = new[]
    {
        new PileStage(PilePart.Names, 0, BezierCurve.EaseOut(0.15)),
        new PileStage(PilePart.Wide, 0.10, new SpringCurve(0.28, 0.9)),
        new PileStage(PilePart.Tall, 0.32, new SpringCurve(0.34, 0.86)),
    };

    /// <summary>
    /// The pile's part at <paramref name="seconds"/> after the pointer
    /// entered (or left), from <paramref name="from"/> toward its target;
    /// under reduce motion every part is at its end at once.
    /// </summary>
    public static double PilePartAt(PilePart part, bool entering, double seconds, double from, bool reduceMotion)
    {
        double target = entering ? 1 : 0;
        if (reduceMotion)
        {
            return target;
        }

        foreach (PileStage stage in entering ? PileOpen : PileClose)
        {
            if (stage.Part == part)
            {
                double t = seconds - stage.Delay;
                return t <= 0 ? from : from + (target - from) * stage.Curve.At(t);
            }
        }

        return target;
    }

    // Map art (Indicators.swift:660-885).

    /// <summary>The pointer light fading in and out: fast 0.15 (:862, :882).</summary>
    public static readonly BezierCurve PointerLightFade = BezierCurve.Token(MonitorMotion.Fast);

    /// <summary>The shimmer pass (:672-674).</summary>
    public static readonly BezierCurve ShimmerPass = new(MapField.ShimmerX1, MapField.ShimmerY1, MapField.ShimmerX2, MapField.ShimmerY2, MapField.ShimmerDuration);
}

/// <summary>What the map's clock decision reads (<see cref="MapMotion.ClockInterval"/>).</summary>
public readonly record struct MapClockInputs(
    bool AnimationsEnabled,
    bool Loaded,
    bool WindowVisible,
    bool PaneShown,
    bool HasLibrary,
    bool Flows,
    bool Healthy,
    bool TransferPlaying,
    bool PileMoving,
    bool LightFading,
    double SecondsShown,
    double? NextDeadlineIn);

/// <summary>
/// The map's still layer (field, nodes, a resting pile) and when it must be
/// drawn again. The window's tick and its still-layer draw both report here
/// with the scene they drew, so whether a scene has a pile is read from the
/// scene, never passed in by the caller (review W1 of #2215: a caller's
/// boolean was the one input no test could see).
/// </summary>
public sealed class StillLayer
{
    private bool _pileInStill;
    private bool _fieldMoved;

    /// <summary>
    /// One tick of the map's clock. True when the still layer must be drawn
    /// again: the field's shimmer or pointer light moves, or stopped on the
    /// last tick, or the pile crosses between the layers.
    /// </summary>
    public bool Tick(FlowMapScene scene, bool fieldMoves, bool pileMoving)
    {
        bool redraws = MapMotion.StillRedraws(fieldMoves, _fieldMoved, scene.DrawsMap && scene.Stack is not null, pileMoving, _pileInStill);
        _fieldMoved = fieldMoves;
        return redraws;
    }

    /// <summary>
    /// The still layer was drawn with <paramref name="scene"/>: a resting pile
    /// went into it, or none did.
    /// </summary>
    public void Drawn(FlowMapScene scene, bool pileMoving) =>
        _pileInStill = scene.DrawsMap && scene.Stack is not null && !pileMoving;
}
