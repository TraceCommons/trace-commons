using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.FirstRun;

/// <summary>
/// The first run's motion that is not a step graphic's own drawing, each as
/// the Mac's curve, duration and delay over the seconds since its change:
/// how often the stage draws, the beam's fade, the ring tightening, the
/// Tools rows rising, the Private AI count, the Traces estimate, the step
/// thread, the step crossfade, the small switch, the segmented pill, Join's
/// chosen card and Customize's card opening.
/// </summary>
public static class FirstRunMotion
{
    /// <summary>The display's own rate: the Mac's <c>GlassAnimationTimeline</c> with no interval.</summary>
    public const double DisplayRate = 0;

    public const double Thirty = 1.0 / 30, Sixty = 1.0 / 60;

    /// <summary>
    /// How often the stage draws on <paramref name="step"/>, in seconds
    /// between frames: the fastest of the Mac's timelines that is moving
    /// there. The backdrop and the N's breath run at 30 a second on every
    /// step (FirstRunStage.swift:131, :247); Welcome's view runs at the
    /// display's rate while its mark draws, its words rise or the shine
    /// crosses (WelcomeScreen.swift:107), and is otherwise still; Tools at
    /// the display's rate while it scans, 30 after (ToolsScreen.swift:419);
    /// Uses and Traces at 60 (UsesScreen.swift:431, TracesScreen.swift:160);
    /// Private AI and Join at 30 (PrivateAIScreen.swift:95, JoinScreen.swift:524).
    /// A SwiftUI animation under way (<paramref name="inFlight"/>: an actor
    /// moving between steps, the thread filling, a crossfade, the beam
    /// fading, the ring tightening, a satellite or a row springing) runs at
    /// the display's rate.
    /// </summary>
    public static double StageInterval(FirstRunStep step, bool welcomeMoving, bool scanning, bool inFlight)
    {
        if (inFlight)
        {
            return DisplayRate;
        }

        return step switch
        {
            FirstRunStep.Welcome => welcomeMoving ? DisplayRate : Thirty,
            FirstRunStep.Tools => scanning ? DisplayRate : Thirty,
            FirstRunStep.Uses or FirstRunStep.Traces => Sixty,
            _ => Thirty,
        };
    }

    /// <summary>The rate each layer's own timeline samples time at (see <see cref="FirstRunStageLayout.AtRate"/>).</summary>
    public static double LayerInterval(StageLayer layer, bool scanning) => layer switch
    {
        StageLayer.Backdrop or StageLayer.HubBreath or StageLayer.PrivateAI or StageLayer.Join => Thirty,
        StageLayer.Uses or StageLayer.Traces => Sixty,
        StageLayer.Tools => scanning ? DisplayRate : Thirty,
        _ => DisplayRate,
    };

    /// <summary>
    /// Whether Welcome's own view is moving <paramref name="elapsed"/> seconds
    /// after it opened: the mark drawing, a line rising, or the shine crossing.
    /// </summary>
    public static bool WelcomeMoving(double elapsed, bool reduceMotion) =>
        !reduceMotion && (elapsed < WelcomeMotion.RisenBy || WelcomeMotion.Shine(elapsed, false) is not null);

    /// <summary>
    /// Whether a frame <paramref name="since"/> seconds after the last one is
    /// due at <paramref name="interval"/>: within a tenth of it, so a display
    /// whose frames land a little early does not drop every other one.
    /// </summary>
    public static bool FrameDue(double since, double interval) => since >= interval * 0.9;

    /// <summary>The beam fades over 0.5 s on the token curve (ToolsScreen.swift:436-437).</summary>
    public const double BeamFade = 0.5;

    public static double BeamOpacity(double from, double to, double since, bool reduceMotion) =>
        reduceMotion ? to : from + (to - from) * FirstRunStageLayout.Timed(since, BeamFade);

    /// <summary>
    /// The dashed ring's scale once the scan is over: 118/130, on the spatial
    /// spring (ToolsScreen.swift:464-465), <paramref name="since"/> seconds
    /// after the scan ended (negative while it runs).
    /// </summary>
    public static double RingScale(double since, bool reduceMotion)
    {
        double tight = ToolsRing.TightRadius / ToolsRing.LooseRadius;
        if (since < 0)
        {
            return 1;
        }

        return reduceMotion ? tight : 1 + (tight - 1) * FirstRunStageLayout.Spatial(since);
    }

    /// <summary>The Tools rows' rise (<c>RowRise</c>, ToolsScreen.swift:740-751).</summary>
    public const double RowRiseDuration = 0.6, RowStagger = 0.08, RowLift = 18;

    /// <summary>Row <paramref name="index"/> (the add row is the row count) <paramref name="since"/> seconds after the scan: offset and opacity.</summary>
    public static (double OffsetY, double Opacity) RowRise(int index, double since, bool reduceMotion)
    {
        if (reduceMotion)
        {
            return (0, 1);
        }

        double t = since - RowStagger * (index + 1);
        double p = FirstRunStageLayout.Spring(t, RowRiseDuration, FirstRunStageLayout.SpatialBounce);
        return (RowLift * (1 - p), Math.Clamp(p, 0, 1));
    }

    /// <summary>When every row and the add row have risen.</summary>
    public static double RowsRisenBy(int rows) => RowStagger * (rows + 1) + FirstRunStageLayout.SpringSettle(RowRiseDuration);

    /// <summary>The list card fades in over 0.3 s on the token curve (ToolsScreen.swift:609).</summary>
    public const double CardFade = 0.3;

    public static double CardOpacity(double since, bool reduceMotion) => reduceMotion ? 1 : FirstRunStageLayout.Timed(since, CardFade);

    /// <summary>Traces' estimate rises 8 points on the spatial spring at 0.64 s once the count is done (TracesScreen.swift:281-285).</summary>
    public const double EstimateRise = 0.64, EstimateLift = 8;

    public static (double OffsetY, double Opacity) Estimate(double since, bool reduceMotion)
    {
        if (reduceMotion)
        {
            return (0, 1);
        }

        double p = FirstRunStageLayout.Spring(since, EstimateRise, FirstRunStageLayout.SpatialBounce);
        return (EstimateLift * (1 - p), Math.Clamp(p, 0, 1));
    }

    /// <summary>
    /// The step thread coming in or going with Welcome: opacity combined
    /// with an offset of -10, on the spatial spring
    /// (OnboardingCoordinatorView.swift:124, :131).
    /// </summary>
    public const double ThreadLift = -10;

    public static (double OffsetY, double Opacity) Thread(bool shown, double since, bool reduceMotion)
    {
        double p = reduceMotion ? 1 : FirstRunStageLayout.Spatial(since);
        double v = shown ? p : 1 - p;
        return (ThreadLift * (1 - v), Math.Clamp(v, 0, 1));
    }

    /// <summary>A thread stretch fills over 0.52 s on the token curve (FirstRunStage.swift:198).</summary>
    public const double SegmentFill = 0.52;

    public static double SegmentWidth(bool filled, bool wasFilled, double since, bool reduceMotion)
    {
        double to = filled ? FirstRunStepThread.SegmentWidth : 0;
        double from = wasFilled ? FirstRunStepThread.SegmentWidth : 0;
        return reduceMotion ? to : from + (to - from) * FirstRunStageLayout.Timed(since, SegmentFill);
    }

    /// <summary>The current title lights over 0.3 s after 0.42 s (FirstRunStage.swift:211).</summary>
    public const double TitleGlow = 0.3, TitleGlowDelay = 0.42;

    /// <summary>How lit a title is: 1 current, 0 not, crossing on the token curve.</summary>
    public static double TitleLit(bool current, bool wasCurrent, double since, bool reduceMotion)
    {
        double to = current ? 1 : 0, from = wasCurrent ? 1 : 0;
        return reduceMotion ? to : from + (to - from) * FirstRunStageLayout.Timed(since - TitleGlowDelay, TitleGlow);
    }

    /// <summary>Two steps crossfade on the spatial spring (OnboardingCoordinatorView.swift:131, :155-173).</summary>
    public static double Crossfade(double since, bool reduceMotion) =>
        reduceMotion ? 1 : Math.Clamp(FirstRunStageLayout.Spatial(since), 0, 1);

    /// <summary>
    /// An actor (the N, a tool tile) moving to where the next step draws it,
    /// on the spatial spring (<c>matchedGeometryEffect</c>, FirstRunStage.swift:517).
    /// </summary>
    public static StageFrame Actor(StageFrame from, StageFrame to, double since, bool reduceMotion) =>
        reduceMotion ? to : StageFrame.Tween(from, to, FirstRunStageLayout.Spatial(since));

    /// <summary>The small switch (<c>FirstRunSwitch</c>, FirstRunStage.swift:463-494): 30 by 18, knob 14, toggled over 0.2 s.</summary>
    public const double SwitchDuration = 0.2, SwitchWidth = 30, SwitchHeight = 18, SwitchKnob = 14;

    public static double SwitchOn(bool on, double? since, bool reduceMotion)
    {
        double p = since is double s && !reduceMotion ? FirstRunStageLayout.Timed(s, SwitchDuration) : 1;
        return on ? p : 1 - p;
    }

    /// <summary>The segmented control's chosen pill fills over 0.15 s (FirstRunStage.swift:443).</summary>
    public const double PillFill = 0.15;

    public static double PillChosen(bool chosen, double? since, bool reduceMotion)
    {
        double p = since is double s && !reduceMotion ? FirstRunStageLayout.Timed(s, PillFill) : 1;
        return chosen ? p : 1 - p;
    }

    /// <summary>Join's chosen card border, 1.5 points in toggleOn, over <c>standard</c> 0.22 s (JoinScreen.swift:688).</summary>
    public const double JoinBorder = 0.22, JoinBorderWidth = 1.5;

    public static double JoinChosen(bool chosen, double? since, bool reduceMotion)
    {
        double p = since is double s && !reduceMotion ? FirstRunStageLayout.Timed(s, JoinBorder) : 1;
        return chosen ? p : 1 - p;
    }

    /// <summary>
    /// Customize opening on Uses (<c>FirstRunFrame</c>'s <c>open</c>,
    /// FirstRunFrame.swift:186-196, :225): the graphic and the title fade
    /// out and the card rises, on the spatial spring at 0.76 s.
    /// </summary>
    public const double CustomizeOpen = 0.76;

    public static double CustomizeFadeOut(double since, bool reduceMotion)
    {
        if (reduceMotion || since >= FirstRunStageLayout.SpringSettle(CustomizeOpen))
        {
            return 0;
        }

        return 1 - Math.Clamp(FirstRunStageLayout.Spring(since, CustomizeOpen, FirstRunStageLayout.SpatialBounce), 0, 1);
    }

    /// <summary>The card's rise from <paramref name="dy"/> points off its place: offset and opacity.</summary>
    public static (double OffsetY, double Opacity) CustomizeRise(double dy, double since, bool reduceMotion)
    {
        if (reduceMotion || since >= FirstRunStageLayout.SpringSettle(CustomizeOpen))
        {
            return (0, 1);
        }

        double p = FirstRunStageLayout.Spring(since, CustomizeOpen, FirstRunStageLayout.SpatialBounce);
        return (dy * (1 - p), Math.Clamp(p, 0, 1));
    }
}

/// <summary>A layer of the stage with its own timeline on the Mac.</summary>
public enum StageLayer
{
    Backdrop,
    HubBreath,
    Welcome,
    Tools,
    PrivateAI,
    Traces,
    Uses,
    Join,
}

/// <summary>The step thread's geometry (FirstRunStage.swift:160-226).</summary>
public static class FirstRunStepThread
{
    public const double SegmentWidth = 38, Height = 28, Top = 14, TitlePadding = 10, LineDash = 2, LineGap = 4, Line = 2, Fill = 3;

    /// <summary>The glow round the leading stretch (radius 5) and a travelled one (4), and a lit title's (7), all firstRunGlow at 0.35.</summary>
    public const double LeadingGlow = 5, TravelledGlow = 4, TitleGlow = 7, GlowAlpha = 0.35;
}

/// <summary>A square actor's frame: its centre and side, in canvas points.</summary>
public readonly record struct StageFrame(StagePoint Center, double Side)
{
    public static StageFrame Tween(StageFrame from, StageFrame to, double t) => new(
        new StagePoint(from.Center.X + (to.Center.X - from.Center.X) * t, from.Center.Y + (to.Center.Y - from.Center.Y) * t),
        from.Side + (to.Side - from.Side) * t);
}

/// <summary>
/// The oversized marker check (<c>FirstRunMarkerCheckStyle</c>,
/// FirstRunStage.swift:341-405): a 22-point box inset 2 in a 26-point
/// frame, and the stroke in its own 44 box at (-6, -14), trimmed by length
/// as it draws on (0.34 s) or off (0.16 s) on the token curve.
/// </summary>
public static class MarkerCheck
{
    public const double Box = 22, BoxInset = 2, BoxRadius = 6, Border = 1.5, Frame = 26, StrokeBox = 44, StrokeX = -6, StrokeY = -14, StrokeWidth = 3.6, OnDuration = 0.34, OffDuration = 0.16, Disabled = 0.45;

    public static readonly StagePoint Start = new(11, 27);

    /// <summary>The two curves (control 1, control 2, end) (FirstRunStage.swift:401-402).</summary>
    public static readonly IReadOnlyList<(StagePoint C1, StagePoint C2, StagePoint To)> Curves = new[]
    {
        (new StagePoint(13, 29), new StagePoint(15.5, 32), new StagePoint(18, 35)),
        (new StagePoint(22, 27.5), new StagePoint(28, 18.5), new StagePoint(35, 12)),
    };

    /// <summary>How much of the stroke is drawn, <paramref name="since"/> seconds after it changed (null: never, at rest).</summary>
    public static double Trim(bool on, double? since, bool reduceMotion)
    {
        double target = on ? 1 : 0;
        if (since is not double s || reduceMotion)
        {
            return target;
        }

        double p = FirstRunStageLayout.Timed(s, on ? OnDuration : OffDuration);
        return on ? p : 1 - p;
    }

    public static bool Moving(bool on, double? since, bool reduceMotion) =>
        !reduceMotion && since is double s && s < (on ? OnDuration : OffDuration);

    /// <summary>The stroke flattened, 64 steps a curve, in its 44 box.</summary>
    public static IReadOnlyList<StagePoint> Polyline()
    {
        var points = new List<StagePoint> { Start };
        StagePoint from = Start;
        foreach (var (c1, c2, to) in Curves)
        {
            for (int i = 1; i <= 64; i++)
            {
                double t = i / 64.0, u = 1 - t;
                points.Add(new StagePoint(
                    u * u * u * from.X + 3 * u * u * t * c1.X + 3 * u * t * t * c2.X + t * t * t * to.X,
                    u * u * u * from.Y + 3 * u * u * t * c1.Y + 3 * u * t * t * c2.Y + t * t * t * to.Y));
            }

            from = to;
        }

        return points;
    }

    /// <summary>The stroke's first <paramref name="trim"/> of its length.</summary>
    public static IReadOnlyList<StagePoint> Trimmed(double trim)
    {
        IReadOnlyList<StagePoint> line = Polyline();
        double total = 0;
        for (int i = 1; i < line.Count; i++)
        {
            total += Length(line[i - 1], line[i]);
        }

        double left = total * Math.Clamp(trim, 0, 1);
        var drawn = new List<StagePoint>();
        if (left <= 0)
        {
            return drawn;
        }

        drawn.Add(line[0]);
        for (int i = 1; i < line.Count; i++)
        {
            double len = Length(line[i - 1], line[i]);
            if (left >= len)
            {
                drawn.Add(line[i]);
                left -= len;
            }
            else
            {
                double f = left / len;
                drawn.Add(new StagePoint(line[i - 1].X + (line[i].X - line[i - 1].X) * f, line[i - 1].Y + (line[i].Y - line[i - 1].Y) * f));
                break;
            }
        }

        return drawn;
    }

    private static double Length(StagePoint a, StagePoint b) => Math.Sqrt((b.X - a.X) * (b.X - a.X) + (b.Y - a.Y) * (b.Y - a.Y));
}
