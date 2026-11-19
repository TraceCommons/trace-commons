using System;
using System.Collections.Generic;
using System.Linq;
using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The first-run stage's numbers, pinned to what the Mac's own Swift
/// prints. Every expected value below was printed by a throwaway probe that
/// compiled, verbatim from macos/Sources at origin/main 409066936,
/// <c>WelcomeScreenLayout</c> and <c>GlassMotionCurve</c> (WelcomeScreen.swift:10-84),
/// <c>FirstRunStageLayout</c> (FirstRunStage.swift:11-107, :524-546),
/// <c>ToolsRingLayout</c> (ToolsScreen.swift:345-406), <c>UsesFlowLayout</c>
/// (UsesScreen.swift:320-418), <c>TracesScreenLayout</c> (TracesScreen.swift:91-119),
/// <c>PrivateAIScreenLayout</c> (PrivateAIScreen.swift:27-47) and the
/// <c>GlassBrandMark</c> drawing extension (GlassBrandMarkDrawing.swift:4-62),
/// and SwiftUI's own <c>Spring(duration:bounce:).value(target:time:)</c> and
/// <c>UnitCurve.bezier</c>. Nothing here is computed by the port.
/// </summary>
public class FirstRunStageTests
{
    private static void Near(double expected, double actual, double tolerance = 1e-6) =>
        Assert.True(Math.Abs(expected - actual) < tolerance, $"expected {expected}, got {actual}");

    [Fact]
    public void TheTokenCurveSampledForAValueIsTheMacs()
    {
        // GlassMotionCurve.value (WelcomeScreen.swift:65-84).
        (double T, double V)[] probe =
        {
            (0.05, 0.202240783), (0.1, 0.396693549), (0.2, 0.681792440), (0.25, 0.767284301),
            (0.3, 0.827630707), (0.5, 0.946079462), (0.75, 0.991108187), (0.9, 0.998830259),
        };
        foreach (var (t, v) in probe)
        {
            Near(v, FirstRunStageLayout.Curve(t));
        }
    }

    [Fact]
    public void TimedAnimationsRunSwiftUIsTimingCurve()
    {
        // UnitCurve.bezier((0.2, 0.8), (0.2, 1)).value(at:), which
        // GlassMotion.timed's timingCurve runs; SwiftUI solves to about 1e-6.
        (double T, double V)[] probe =
        {
            (0.05, 0.202239037), (0.1, 0.396692276), (0.2, 0.681792259), (0.3, 0.827630997),
            (0.5, 0.946079254), (0.75, 0.991107941), (0.9, 0.998829842),
        };
        foreach (var (t, v) in probe)
        {
            Near(v, FirstRunStageLayout.Timed(t, 1), 1e-5);
            // Over the marker check's 0.34 s, at the same fraction.
            Near(v, FirstRunStageLayout.Timed(t * 0.34, 0.34), 1e-5);
        }
    }

    [Theory]
    [InlineData(0.9, 0.04, new[] { 0.048843494, 0.157440324, 0.416044334, 0.634402070, 0.880779918, 0.983876831, 0.999437369 })]
    [InlineData(0.6, 0.04, new[] { 0.098587691, 0.287061283, 0.634402070, 0.839319030, 0.976941085, 0.999437369, 1.000012636 })]
    [InlineData(0.64, 0.04, new[] { 0.088423026, 0.262329303, 0.598536450, 0.810881946, 0.968039403, 0.998838165, 1.000018674 })]
    [InlineData(0.76, 0.04, new[] { 0.065799296, 0.204190556, 0.503962574, 0.726084709, 0.933048052, 0.994727183, 0.999979151 })]
    [InlineData(0.46, 0.32, new[] { 0.168822492, 0.478497745, 0.933952998, 1.053248768, 1.005737063, 0.999550318, 0.999994660 })]
    [InlineData(0.78, 0.32, new[] { 0.067244612, 0.221075557, 0.586156741, 0.862920712, 1.052270319, 1.011432792, 0.998097261 })]
    public void SpringsAreSwiftUIs(double duration, double bounce, double[] values)
    {
        double[] times = { 0.05, 0.1, 0.2, 0.3, 0.5, 0.8, 1.2 };
        for (int i = 0; i < times.Length; i++)
        {
            Near(values[i], FirstRunStageLayout.Spring(times[i], duration, bounce));
        }
    }

    [Fact]
    public void TheSpatialSpringIsDrawnUntilSwiftUISettlesIt()
    {
        // Spring(duration: 0.9, bounce: 0.04).settlingDuration = 1.335296594.
        Assert.True(FirstRunStageLayout.SpatialSettle >= 1.335296594);
        Assert.True(FirstRunStageLayout.SpatialSettle < 1.5);
        Near(0.634402070, FirstRunStageLayout.Spatial(0.3));
        Assert.Equal(1, FirstRunStageLayout.Spatial(1.4));
        // The 0.6 s rows spring settles in 0.926129168 s; the stage stops
        // drawing it after the scaled settle.
        Assert.True(FirstRunStageLayout.SpringSettle(0.6) >= 0.926129168);
    }

    [Fact]
    public void WelcomeDrawsTheMarkThenRisesItsWords()
    {
        (double T, double V)[] drawn = { (0, 0), (0.3, 0.004), (0.5, 0.108), (0.7, 0.5), (1.0, 0.968), (1.2, 1), (1.5, 1) };
        foreach (var (t, v) in drawn)
        {
            Near(v, WelcomeMotion.Drawn(t, false));
        }

        Assert.Null(WelcomeMotion.Shine(1.4, false));
        Near(0, WelcomeMotion.Shine(1.5, false)!.Value);
        Near(0.885440749, WelcomeMotion.Shine(2.0, false)!.Value);
        Assert.Null(WelcomeMotion.Shine(2.85, false));
        Assert.Null(WelcomeMotion.Shine(2.9, false));
        Near(0, WelcomeMotion.Shine(16.5, false)!.Value);
        Near(0.885440749, WelcomeMotion.Shine(17.0, false)!.Value);

        (double T, double V)[] risen = { (1.05, 0), (1.2, 0.952889079), (1.5, 0.987532067), (1.81, 1), (1.9, 1) };
        foreach (var (t, v) in risen)
        {
            Near(v, WelcomeMotion.Risen(t, 1.05, false));
        }

        (double T, double V)[] glow = { (0.1, 0), (0.5, 0.128850488), (0.7, 0.5), (1.25, 1), (1.4, 1) };
        foreach (var (t, v) in glow)
        {
            Near(v, WelcomeMotion.Glow(t, false));
        }

        // The words at 1.05/1.14/1.23, a fourth with the third
        // (WelcomeScreen.swift:16, :147); body 1.45, buttons 1.8, 0.76 s, 18 pt.
        Assert.Equal(new[] { 1.05, 1.14, 1.23, 1.23 }, Enumerable.Range(0, 4).Select(WelcomeMotion.WordDelay));
        Assert.Equal((1.45, 1.8, 0.76, 18.0), (WelcomeMotion.BodyDelay, WelcomeMotion.ButtonDelay, WelcomeMotion.RiseDuration, WelcomeMotion.RiseLift));
        Near(44.0 / 26, WelcomeMotion.HeroScale);
        // Split as Swift's split(separator: " "): empty pieces dropped.
        Assert.Equal(new[] { "Welcome ", "to ", "Near ", "AI" }, WelcomeMotion.Words("Welcome to  Near AI"));
        Assert.Equal((0.0, 18.0), WelcomeMotion.Rise(0));
    }

    [Fact]
    public void ReduceMotionShowsWelcomesFinalFrameAtOnce()
    {
        Assert.Equal(1, WelcomeMotion.Drawn(0, true));
        Assert.Null(WelcomeMotion.Shine(1.5, true));
        Assert.Equal(1, WelcomeMotion.Risen(0, 1.05, true));
        Assert.Equal(1, WelcomeMotion.Glow(0, true));
        Assert.False(FirstRunMotion.WelcomeMoving(0.5, true));
    }

    [Fact]
    public void TheMarkersLineAndShineAreTheMacs()
    {
        Near(983.378253422, BrandMarkDrawing.MarkerLength);
        (double D, double X, double Y)[] points =
        {
            (0, 167.5, 234.6), (100, 102.332133574, 310.449516712), (500, 212.073542590, 217.268732152),
            (900, 279.956871173, 106.434370947), (2000, 226, 170),
        };
        foreach (var (d, x, y) in points)
        {
            StagePoint p = BrandMarkDrawing.MarkerPoint(d);
            Near(x, p.X);
            Near(y, p.Y);
        }

        var band = BrandMarkDrawing.ShineBand(100);
        Near(35.016060753, band.From.X);
        Near(200, band.From.Y);
        Near(134.511995444, band.To.X);
        Near(232.328188876, band.To.Y);
        Assert.Equal((-240.0, 470.0), (BrandMarkDrawing.ShineFrom, BrandMarkDrawing.ShineTo));
    }

    [Fact]
    public void TheToolsScanIsTheMacs()
    {
        (double T, double V)[] burst = { (0.1, 0.734832702), (0.3, 1.096223573), (0.5, 0.997967236), (0.7, 0.995865198) };
        foreach (var (t, v) in burst)
        {
            Near(v, FirstRunStageLayout.Burst(t, 0.78));
        }

        double[] five = { 1.181818182, 1.545454545, 1.909090909, 2.272727273, 2.636363636 };
        for (int i = 0; i < 5; i++)
        {
            Near(five[i], FirstRunStageLayout.ScanMoment(i, 5));
        }

        Assert.Null(FirstRunStageLayout.SweepRotation(0.9));
        Near(-36, FirstRunStageLayout.SweepRotation(1.0)!.Value);
        Near(63, FirstRunStageLayout.SweepRotation(1.5)!.Value);
        Near(162, FirstRunStageLayout.SweepRotation(2.0)!.Value);
        Near(360, FirstRunStageLayout.SweepRotation(3.0)!.Value);
        Assert.Null(FirstRunStageLayout.SweepRotation(3.1));
        Assert.Equal(new[] { "claude-code", "gemini-cli", "codex", "cline", "opencode" }, FirstRunStageLayout.RingOrder);
    }

    [Fact]
    public void TilesBurstSettleAndLeaveAsTheMacs()
    {
        bool[] exists = { true, false, true, true, false };
        // (index, elapsed or null, angle, radius, scale, opacity) from the probe.
        (int I, double? E, double A, double R, double S, double O)[] probe =
        {
            (0, null, -90, 118, 1, 1), (1, null, -18, 470, 0.6, 0), (2, null, 30, 118, 1, 1), (3, null, 150, 118, 1, 1), (4, null, 198, 470, 0.6, 0),
            (0, 0.3, -90, 41.413565736, 0.522996123, 0.509705424), (0, 1.0, -90, 129.704890859, 0.998410951, 1), (0, 3.5, -90, 120.360372477, 1, 1),
            (1, 1.0, -18, 129.190527483, 0.995641302, 1), (1, 2.2, -18, 281.798760331, 0.821413223, 0.553533058), (1, 3.5, -18, 470, 0.6, 0),
            (2, 3.5, 34.720744954, 120.360372477, 1, 1), (2, 4.5, 30, 118, 1, 1), (3, 3.5, 145.279255046, 120.360372477, 1, 1),
            (4, 1.0, 198, 133.206743139, 1, 1), (4, 3.5, 198, 423.778615702, 0.654378099, 0.135945248),
        };
        foreach (var (i, e, a, r, s, o) in probe)
        {
            RingPlacement p = ToolsRing.Placement(i, exists, e);
            Near(a, p.Angle);
            Near(r, p.Radius);
            Near(s, p.Scale);
            Near(o, p.Opacity);
        }

        Near(0.342657343, ToolsRing.Flash(0, 5, 1.3)!.Value);
        Near(0.919580420, ToolsRing.Flash(0, 5, 1.6)!.Value);
        Near(0.220279720, ToolsRing.Flash(1, 5, 1.6)!.Value);
        Near(0.552447552, ToolsRing.Flash(3, 5, 2.5)!.Value);
        Assert.Null(ToolsRing.Flash(0, 5, 2.5));
        Assert.Null(ToolsRing.Flash(2, 5, 1.0));
    }

    [Fact]
    public void TilesBobOnTheirOwnPhases()
    {
        (string K, double T, double V)[] probe =
        {
            ("claude-code", 0, 0), ("claude-code", 1, -1.381966011), ("claude-code", 2.5, -4),
            ("gemini-cli", 0, -1.625237371), ("codex", 1, -3.071653590), ("cline", 2.5, -1.148441417), ("opencode", 1, -0.062833678),
        };
        foreach (var (k, t, v) in probe)
        {
            Near(v, ToolsRing.Bob(k, t));
        }
    }

    [Fact]
    public void TheBeamsPulsesTravelFromTheTileIntoTheN()
    {
        // ToolsScreen.swift:553, :574: flow = 26.1 x frac(t / 1.1), and the
        // stroke's dashPhase is -flow. The path runs tile (radius 90) to N
        // (radius 16), so a dash that moves to a larger distance moves toward
        // the N. Read the first dash's start at two moments.
        double length = ToolsRing.BeamFrom - ToolsRing.BeamTo;
        double At(double time) => DashFlow.DashStarts(ToolsRing.BeamDash, ToolsRing.BeamDashPhase(time, false), length).Min();
        double early = At(0.2), later = At(0.4);
        Assert.True(later > early, $"the pulses moved from {early} to {later}: toward the tile, not into the N");
        Near(26.1 * 0.2 / 1.1, early);
        // Direct2D reads the offset in stroke widths with the same sign as CoreGraphics' phase.
        Assert.Equal((float)(-ToolsRing.BeamFlow(0.2, false) / 8), DashFlow.Direct2DOffset(-ToolsRing.BeamFlow(0.2, false), ToolsRing.BeamPulseWidth));
        Assert.Equal(new[] { 0.1f / 8, 26f / 8 }, DashFlow.Direct2DDashes(ToolsRing.BeamDash, ToolsRing.BeamPulseWidth));
        Assert.Equal(0, ToolsRing.BeamFlow(0.5, true));
    }

    [Fact]
    public void PrivateAIsDashesRunTowardTheN()
    {
        // PrivateAIScreen.swift:97, :111: dashPhase -22 x frac(t / 1.1); each
        // route runs from its tile to the N, and the N's on to NEAR AI.
        Near(-22 * (0.55 / 1.1), PrivateAIDiagram.DashPhase(0.55, false));
        double At(double time) => DashFlow.DashStarts(PrivateAIDiagram.Dash, PrivateAIDiagram.DashPhase(time, false), 200).Min();
        Assert.True(At(0.3) > At(0.1));
        Assert.Equal(0, PrivateAIDiagram.DashPhase(0.55, true));
        Assert.Equal(new[] { 0, 1, 1, 1, 2, 3, 3 }, new[] { 0.5, 0.9, 1.0, 1.14, 1.15, 1.5, 3.0 }.Select(e => PrivateAIDiagram.Counted(3, e, false)));
        Assert.Equal(3, PrivateAIDiagram.Counted(3, 0, true));
        Assert.Equal(new[] { 101.0, 21, 181, 21, 101, 181 }, new[] { (0, 1), (0, 2), (1, 2), (0, 3), (1, 3), (2, 3) }.Select(p => PrivateAIDiagram.RowY(p.Item1, p.Item2)));
    }

    [Fact]
    public void TracesCountAndStreamAsTheMacs()
    {
        (double T, double V)[] counted = { (0.5, 0), (0.9, 0), (1.5, 0.679690917), (2.8, 1), (3.0, 1) };
        foreach (var (t, v) in counted)
        {
            Near(v, TracesGraphic.Counted(t, false));
        }

        Assert.Equal(new[] { 114.0, 83, 145, 52, 114, 176, 21, 83, 145, 207 },
            new[] { (0, 1), (0, 2), (1, 2), (0, 3), (1, 3), (2, 3), (0, 4), (1, 4), (2, 4), (3, 4) }.Select(p => TracesGraphic.RowY(p.Item1, p.Item2, 114)));
        var curve = TracesGraphic.Stream(new StagePoint(359, 52), new StagePoint(443, 114));
        (double T, double X, double Y)[] bez = { (0.25, 389, 61.6875), (0.5, 407, 83), (0.75, 422, 104.3125) };
        foreach (var (t, x, y) in bez)
        {
            StagePoint p = TracesGraphic.Bezier(curve, t);
            Near(x, p.X);
            Near(y, p.Y);
        }
    }

    [Fact]
    public void UsesFlowIsTheMacs()
    {
        // (t, along, opacity, fill, scale, turn, white, pulse scale, pulse opacity)
        (double T, double[] V)[] ask =
        {
            (0.02, new[] { 0.0248125, 0.4, 0, 0.86, 0, 0, 0.7, 0 }),
            (0.35, new[] { 0.397, 1, 0.5, 1.18, -12, 0.666666667, 0.7, 0 }),
            (0.38, new[] { 0.397, 1, 1, 1.14, -4.5, 1, 0.7, 0 }),
            (0.42, new[] { 0.397, 1, 1, 1.01, 2.5, 1, 0.7, 0 }),
            (0.66, new[] { 0.447285714, 1, 0.25, 0.9, 0, 0.25, 0.7, 0 }),
            (0.935, new[] { 1, 0.928571429, 0, 0.86, 0, 0, 0.7375, 0.675 }),
            (0.96, new[] { 1, 0.571428571, 0, 0.86, 0, 0, 0.983333333, 0.6 }),
        };
        (double T, double[] V)[] auto =
        {
            (0.42, new[] { 0.456521739, 1, 1, 1.3, -16, 1, 0.7, 0 }),
            (0.91, new[] { 0.989130435, 1, 0, 0.86, 0, 0, 0.716666667, 0.3 }),
            (0.935, new[] { 1, 0.8125, 0, 0.86, 0, 0, 0.8, 0.835714286 }),
            (0.96, new[] { 1, 0.5, 0, 0.86, 0, 0, 1.05, 0.514285714 }),
        };
        foreach (var (path, rows) in new[] { (SharingPath.AskMe, ask), (SharingPath.Automatic, auto) })
        {
            foreach (var (t, v) in rows)
            {
                var orb = UsesFlow.Orb(path, t);
                UsesGate g = UsesFlow.GateLook(path, t);
                var pulse = UsesFlow.Pulse(path, t);
                double[] got = { orb.Along, orb.Opacity, g.Fill, g.Scale, g.Turn, g.White, pulse.Scale, pulse.Opacity };
                for (int i = 0; i < v.Length; i++)
                {
                    Near(v[i], got[i]);
                }
            }
        }

        double[] times = { 0.5, 4.0, 5.1, 9.5, 14.0, 18.5, 22.9, 23.3, 23.6, 24.0, 27.6 };
        Assert.Equal(new[] { 0, 0, 1, 2, 3, 4, 5, 5, 5, 0, 1 }, times.Select(e => UsesFlow.SatellitesFilled(SharingPath.AskMe, e)));
        Assert.Equal(new[] { 0, 1, 1, 3, 5, 1, 3, 3, 3, 4, 5 }, times.Select(e => UsesFlow.SatellitesFilled(SharingPath.Automatic, e)));
        Near(0.454545455, UsesFlow.Loop(SharingPath.AskMe, 3.0)!.Value);
        Near(0.884615385, UsesFlow.Loop(SharingPath.Automatic, 5.5)!.Value);
        Assert.Null(UsesFlow.Loop(SharingPath.AskMe, 0.5));
    }

    [Fact]
    public void SatellitesPopOnTheExpressiveSpringAndNoneUnderReduceMotion()
    {
        // UsesScreen.swift:578-582: scale and opacity 0 to 1 on
        // expressive(duration: 0.46), bounce 0.32 (probe values).
        (double T, double V)[] pop = { (0.05, 0.168822492), (0.1, 0.478497745), (0.2, 0.933952998), (0.3, 1.053248768), (0.5, 1.005737063) };
        foreach (SharingPath path in new[] { SharingPath.AskMe, SharingPath.Automatic })
        {
            double first = UsesFlow.Delay(path) + UsesFlow.Period(path) * UsesFlow.Arrival(path);
            for (int place = 0; place < 5; place++)
            {
                double arrives = first + UsesFlow.Period(path) * place;
                Assert.Equal(0, UsesFlow.Satellite(path, arrives - 0.01, place, false));
                foreach (var (t, v) in pop)
                {
                    Near(v, UsesFlow.Satellite(path, arrives + t, place, false));
                }

                // UsesScreen.swift:529: under reduce motion none is filled.
                Assert.Equal(0, UsesFlow.Satellite(path, arrives + 0.5, place, true));
            }

            double clears = first + 4 * UsesFlow.Period(path) + 1.3;
            foreach (var (t, v) in pop)
            {
                Near(1 - v, UsesFlow.Satellite(path, clears + t, 0, false));
            }
        }
    }

    [Fact]
    public void TheCommonsAndTheOrbitsTurnAtTheMacsRates()
    {
        // UsesScreen.swift:531: one turn per 40 s; ToolsScreen.swift:463 and
        // JoinScreen.swift:531: one per 90 s; clockwise (degrees rise).
        Near(90, UsesFlow.CommonsTurn(10, false));
        Near(4, ToolsRing.Turn(1, false));
        Near(4, JoinOrbit.Turn(1, false));
        Assert.Equal(0, UsesFlow.CommonsTurn(10, true));
        Assert.Equal(0, ToolsRing.Turn(10, true));
        var pulse = JoinOrbit.Pulse(0.16, false)!.Value;
        Near(0.3 + 1.6 * 0.1, pulse.Scale);
        Near(0.1 / 0.18, pulse.Opacity);
        Near((1 - 0.5) / 0.82, JoinOrbit.Pulse(0.8, false)!.Value.Opacity);
        Assert.Null(JoinOrbit.Pulse(1.6, false));
        Assert.Null(JoinOrbit.Pulse(0.5, true));
    }

    [Fact]
    public void TheNBreathesAndTheBackdropGlowsAsTheMacs()
    {
        // FirstRunStage.swift:275-284, :132-142.
        var rest = SceneMotion.Breathing(1, false, false);
        double p = 0.5 - 0.5 * Math.Cos(2 * Math.PI / 4);
        Near(1 + 0.08 * p, rest.Scale);
        Near(1 - 0.25 * p, rest.Opacity);
        var charged = SceneMotion.Breathing(0.45, true, false);
        double c = 0.5 - 0.5 * Math.Cos(2 * Math.PI * 0.45 / 1.8);
        Near(1.02 + 0.12 * c, charged.Scale);
        Near(0.9 + 0.1 * c, charged.Opacity);
        Assert.Equal((1.0, 1.0), SceneMotion.Breathing(1, true, true));
        Near(0.5, SceneMotion.BackdropPhase(3.5, false));
        Assert.Equal(1, SceneMotion.BackdropPhase(3.5, true));
        Near(0.9 + 0.25 * 0.5, SceneMotion.BreathScale(0.5));
        Near(0.6 + 0.4 * 0.5, SceneMotion.BreathOpacity(0.5));
    }

    [Fact]
    public void EachStepDrawsAtTheMacsRate()
    {
        // WelcomeScreen.swift:107 (display rate), ToolsScreen.swift:419 (display
        // while scanning, 30 after), UsesScreen.swift:431 and
        // TracesScreen.swift:160 (60), PrivateAIScreen.swift:95,
        // JoinScreen.swift:524 and FirstRunStage.swift:131/:247 (30).
        Assert.Equal(0, FirstRunMotion.StageInterval(FirstRunStep.Welcome, true, false, false));
        Assert.Equal(1.0 / 30, FirstRunMotion.StageInterval(FirstRunStep.Welcome, false, false, false));
        Assert.Equal(0, FirstRunMotion.StageInterval(FirstRunStep.Tools, false, true, false));
        Assert.Equal(1.0 / 30, FirstRunMotion.StageInterval(FirstRunStep.Tools, false, false, false));
        Assert.Equal(1.0 / 60, FirstRunMotion.StageInterval(FirstRunStep.Uses, false, false, false));
        Assert.Equal(1.0 / 60, FirstRunMotion.StageInterval(FirstRunStep.Traces, false, false, false));
        foreach (FirstRunStep step in new[] { FirstRunStep.PrivateAI, FirstRunStep.Join, FirstRunStep.Terms })
        {
            Assert.Equal(1.0 / 30, FirstRunMotion.StageInterval(step, false, false, false));
            Assert.Equal(0, FirstRunMotion.StageInterval(step, false, false, true));
        }

        // A 30-a-second layer drawn on a 60-a-second frame holds its time.
        Assert.Equal(FirstRunStageLayout.AtRate(0.04, 1.0 / 30), FirstRunStageLayout.AtRate(0.05, 1.0 / 30));
        Assert.NotEqual(FirstRunStageLayout.AtRate(0.04, 1.0 / 60), FirstRunStageLayout.AtRate(0.05, 1.0 / 60));
        Assert.False(FirstRunMotion.FrameDue(0.0167, 1.0 / 30));
        Assert.True(FirstRunMotion.FrameDue(0.0333, 1.0 / 30));
        Assert.True(FirstRunMotion.WelcomeMoving(1.0, false));
        Assert.False(FirstRunMotion.WelcomeMoving(5.0, false));
        Assert.True(FirstRunMotion.WelcomeMoving(16.6, false));
    }

    [Fact]
    public void TheToolsRowsRiseOneAfterAnother()
    {
        // ToolsScreen.swift:749: spatial(duration: 0.6) delayed 0.08 x (index + 1), from 18 pt.
        (double T, double P)[] probe = { (0.05, 0.098587691), (0.1, 0.287061283), (0.2, 0.634402070), (0.3, 0.839319030) };
        foreach (int index in new[] { 0, 1, 4 })
        {
            double delay = 0.08 * (index + 1);
            Assert.Equal((18.0, 0.0), FirstRunMotion.RowRise(index, delay - 0.01, false));
            foreach (var (t, p) in probe)
            {
                var (y, o) = FirstRunMotion.RowRise(index, delay + t, false);
                Near(18 * (1 - p), y);
                Near(p, o);
            }
        }

        Assert.Equal((0.0, 1.0), FirstRunMotion.RowRise(3, 0, true));
        Near(0.827630997, FirstRunMotion.CardOpacity(0.09, false), 1e-5);
    }

    [Fact]
    public void TheEstimateAndTheCustomizeCardRiseOnTheirSprings()
    {
        // TracesScreen.swift:285 spatial 0.64 from 8 pt; FirstRunFrame.swift:225 spatial 0.76.
        var (y, o) = FirstRunMotion.Estimate(0.2, false);
        Near(8 * (1 - 0.598536450), y);
        Near(0.598536450, o);
        Assert.Equal((0.0, 1.0), FirstRunMotion.Estimate(0, true));
        Near(1 - 0.503962574, FirstRunMotion.CustomizeFadeOut(0.2, false));
        Near(100 * (1 - 0.726084709), FirstRunMotion.CustomizeRise(100, 0.3, false).OffsetY);
        Assert.Equal(0, FirstRunMotion.CustomizeFadeOut(0.2, true));
    }

    [Fact]
    public void TheThreadAndItsTitlesMoveOnTheMacsCurves()
    {
        // OnboardingCoordinatorView.swift:124 (offset -10, spatial);
        // FirstRunStage.swift:198 (0.52 s), :211 (0.3 s after 0.42 s).
        var (y, o) = FirstRunMotion.Thread(true, 0.3, false);
        Near(-10 * (1 - 0.634402070), y);
        Near(0.634402070, o);
        Near(38 * 0.827630997, FirstRunMotion.SegmentWidth(true, false, 0.3 * 0.52, false), 1e-4);
        Assert.Equal(0, FirstRunMotion.TitleLit(true, false, 0.42, false), 6);
        Near(0.827630997, FirstRunMotion.TitleLit(true, false, 0.42 + 0.09, false), 1e-5);
        Assert.Equal(1, FirstRunMotion.TitleLit(true, false, 0, true));
        Near(0.634402070, FirstRunMotion.Crossfade(0.3, false));
    }

    [Fact]
    public void TheMarkerCheckAndSmallControlsMoveOnTheTokenCurve()
    {
        // FirstRunStage.swift:373 (0.34 on, 0.16 off), :443 (0.15), :473 (0.2);
        // JoinScreen.swift:688 (0.22).
        Near(0.827630997, MarkerCheck.Trim(true, 0.3 * 0.34, false), 1e-5);
        Near(1 - 0.827630997, MarkerCheck.Trim(false, 0.3 * 0.16, false), 1e-5);
        Assert.Equal(1, MarkerCheck.Trim(true, 0.01, true));
        Assert.Equal(0, MarkerCheck.Trim(false, null, false));
        Near(0.827630997, FirstRunMotion.PillChosen(true, 0.3 * 0.15, false), 1e-5);
        Near(0.827630997, FirstRunMotion.SwitchOn(true, 0.3 * 0.2, false), 1e-5);
        Near(0.827630997, FirstRunMotion.JoinChosen(true, 0.3 * 0.22, false), 1e-5);
        IReadOnlyList<StagePoint> full = MarkerCheck.Trimmed(1);
        Assert.Equal(new StagePoint(11, 27), full[0]);
        Near(35, full[^1].X);
        Near(12, full[^1].Y);
        Assert.Empty(MarkerCheck.Trimmed(0));
    }

    [Fact]
    public void ActorsRestWhereEachStepDrawsThem()
    {
        Assert.Equal(new StageFrame(new StagePoint(430, 66), 132), StageActors.Hub(FirstRunStep.Welcome));
        Assert.Equal(new StageFrame(new StagePoint(430, 158), 64), StageActors.Hub(FirstRunStep.Tools));
        Assert.Equal(new StageFrame(new StagePoint(440, 101), 66), StageActors.Hub(FirstRunStep.PrivateAI));
        Assert.Equal(new StageFrame(new StagePoint(557, 62), 36), StageActors.Hub(FirstRunStep.Traces));
        Assert.Equal(new StageFrame(new StagePoint(650, 46), 52), StageActors.Hub(FirstRunStep.Uses));
        Assert.Equal(new StageFrame(new StagePoint(430, 28), 56), StageActors.Hub(FirstRunStep.Terms));
        Assert.Equal(new StageFrame(new StagePoint(430, 83), 56), StageActors.Hub(FirstRunStep.Join));
        var rows = new[] { ("claude-code", true), ("codex", true) };
        string[] watched = { "claude-code", "codex" };
        Assert.Equal(52, StageActors.Tile(FirstRunStep.Tools, "codex", rows, watched, watched)!.Value.Side);
        Assert.Equal(42, StageActors.Tile(FirstRunStep.PrivateAI, "codex", rows, watched, watched)!.Value.Side);
        Assert.Equal(new StagePoint(207, 145), StageActors.Tile(FirstRunStep.Traces, "codex", rows, watched, watched)!.Value.Center);
        Assert.Equal(new StagePoint(224, 38), StageActors.Tile(FirstRunStep.Uses, "codex", rows, watched, watched)!.Value.Center);
        StagePoint join = StageActors.Tile(FirstRunStep.Join, "codex", rows, watched, watched)!.Value.Center;
        Near(500, join.X);
        Near(83, join.Y);
        Assert.Null(StageActors.Tile(FirstRunStep.Welcome, "codex", rows, watched, watched));
        // The move between steps is the spatial spring.
        StageFrame mid = FirstRunMotion.Actor(StageActors.Hub(FirstRunStep.Welcome), StageActors.Hub(FirstRunStep.Tools), 0.3, false);
        Near(132 + (64 - 132) * 0.634402070, mid.Side);
    }
}
