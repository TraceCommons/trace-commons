using System;
using System.Linq;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Mac's control motion on Windows (spec W5, "Animation coverage" rows
/// for W5). Every expected number was printed by a throwaway SwiftPM probe
/// that compiled macos/Sources/TCDesign at this branch's base (841d81ba5)
/// with testing enabled and called the Mac's own functions and SwiftUI's own
/// <c>Spring</c> and <c>UnitCurve</c>; the probe is not committed. Each test
/// cites the Swift it pins.
/// </summary>
public class GlassMotionTests
{
    private const double Tol = 1e-6;

    // SwiftUI UnitCurve.bezier((0.2,0.8),(0.2,1)).value(at:): the token curve
    // (GlassMotion.swift:7-11). The probe printed single-precision solver
    // output, so these hold to 1e-4.
    [Theory]
    [InlineData(0.1, 0.39669227600097656)]
    [InlineData(0.25, 0.7672843933105469)]
    [InlineData(0.5, 0.9460792541503906)]
    [InlineData(0.75, 0.9911079406738281)]
    [InlineData(0.9, 0.9988298416137695)]
    public void TheTokenCurveIsTheMacs(double t, double mac) => Assert.Equal(mac, GlassMotion.Curve(t), 4);

    // SwiftUI UnitCurve.easeOut.value(at:), the switch knob's curve (Controls.swift:1167).
    [Theory]
    [InlineData(0.1, 0.16057205200195312)]
    [InlineData(0.25, 0.37813854217529297)]
    [InlineData(0.5, 0.6846427917480469)]
    [InlineData(0.75, 0.9065351486206055)]
    [InlineData(0.9, 0.9829730987548828)]
    public void EaseOutIsSwiftUIs(double t, double mac) => Assert.Equal(mac, GlassMotion.EaseOut(t), 4);

    // SwiftUI Spring(response:dampingRatio:).value(target: 1, time:).
    [Theory]
    [InlineData(0.35, 0.85, 0.05, 0.24222934843076027)]
    [InlineData(0.35, 0.85, 0.1, 0.5882845611325372)]
    [InlineData(0.35, 0.85, 0.2, 0.9425040425837673)]
    [InlineData(0.35, 0.85, 0.3, 1.0048318481482623)]
    [InlineData(0.25, 0.8, 0.05, 0.39925126793466215)]
    [InlineData(0.25, 0.8, 0.1, 0.8134033381234506)]
    [InlineData(0.25, 0.8, 0.2, 1.0147929290947173)]
    [InlineData(0.3, 0.85, 0.1, 0.6811659930006483)]
    [InlineData(0.3, 0.85, 0.3, 1.0060199607118339)]
    public void SpringsAreSwiftUIs(double response, double damping, double t, double mac) =>
        Assert.Equal(mac, GlassMotion.Spring(response, damping, t), 6);

    // SwiftUI Spring(duration:bounce:): GlassMotion.spatial (0.9, 0.04) and
    // .expressive (0.78, 0.32), GlassMotion.swift:39-48.
    [Theory]
    [InlineData(0.05, 0.0488434941284801, 0.06724461197186637)]
    [InlineData(0.2, 0.4160443335934906, 0.5861567405227381)]
    [InlineData(0.5, 0.8807799183192158, 1.0522703188106601)]
    [InlineData(0.9, 0.992363621741842, 1.001419291941497)]
    public void TheSpatialAndExpressiveSpringsAreSwiftUIs(double t, double spatial, double expressive)
    {
        Assert.Equal(spatial, GlassMotion.Spatial(t), 6);
        Assert.Equal(expressive, GlassMotion.Expressive(t), 6);
    }

    // The rows' timings: GlassHover.swift:40, GlassPress.swift:116/:127,
    // Controls.swift:1145-1147 (probe: duration(.standard) 0.22,
    // .settings 0.22, .watch 0.15), Controls.swift:651 (expander),
    // Modal.swift:495, Navigation.swift:247, :266, :416.
    [Fact]
    public void EachRowHasTheMacsTiming()
    {
        Assert.Equal(MotionSpec.Token(0.15), GlassControlMotion.HoverFade);
        Assert.Equal(MotionSpec.Token(0.15), GlassControlMotion.Press);
        Assert.Equal(0.08, GlassControlMotion.PressDarkening);
        Assert.Equal(MotionSpec.EaseOutFor(0.22), GlassControlMotion.SwitchKnob(GlassSwitchKind.Standard));
        Assert.Equal(MotionSpec.EaseOutFor(0.22), GlassControlMotion.SwitchKnob(GlassSwitchKind.Settings));
        Assert.Equal(MotionSpec.EaseOutFor(0.15), GlassControlMotion.SwitchKnob(GlassSwitchKind.Watch));
        Assert.Equal(MotionSpec.Token(0.15), GlassControlMotion.ExpanderChevron);
        Assert.Equal(MotionSpec.Token(0.22), GlassControlMotion.ModalFade);
        Assert.Equal((MotionCurve.Spring, 0.35, 0.85), (GlassControlMotion.TabEmberGlide.Curve, GlassControlMotion.TabEmberGlide.Response, GlassControlMotion.TabEmberGlide.Damping));
        Assert.Equal((MotionCurve.Spring, 0.25, 0.8), (GlassControlMotion.TabLift.Curve, GlassControlMotion.TabLift.Response, GlassControlMotion.TabLift.Damping));
        Assert.Equal((MotionCurve.Spring, 0.3, 0.85), (GlassControlMotion.TabDrop.Curve, GlassControlMotion.TabDrop.Response, GlassControlMotion.TabDrop.Damping));
    }

    /// <summary>
    /// A spring's key frames follow the spring, including its overshoot, and
    /// end at exactly 1 (a glide that stopped short would leave the ember off
    /// its tab).
    /// </summary>
    [Fact]
    public void TheEmberGlidesOnTheSpringAndLandsOnItsTab()
    {
        var frames = GlassControlMotion.TabEmberGlide.KeyFrames();
        Assert.Equal((1.0, 1.0), frames[^1]);
        // 0.1 s in it is at the Mac's 0.58828 of the way.
        double d = GlassControlMotion.TabEmberGlide.Duration;
        (double f, double v) = frames.OrderBy(k => Math.Abs(k.Fraction * d - 0.1)).First();
        Assert.Equal(GlassMotion.Spring(0.35, 0.85, f * d), v, 9);
        Assert.Equal(0.5882845611325372, GlassMotion.Spring(0.35, 0.85, 0.1), 6);
        Assert.True(frames.Max(k => k.Value) > 1.004, "the Mac's spring overshoots (1.00483 at 0.3 s)");
    }

    // Controls.swift:1150-1177: 40x24 track, 18 knob, inset 3; watch 38x22, inset 2.
    [Theory]
    [InlineData(GlassSwitchKind.Settings, false, 3)]
    [InlineData(GlassSwitchKind.Settings, true, 19)]
    [InlineData(GlassSwitchKind.Watch, false, 2)]
    [InlineData(GlassSwitchKind.Watch, true, 18)]
    public void TheKnobSitsWhereTheMacsDoes(GlassSwitchKind kind, bool on, double x) =>
        Assert.Equal(x, GlassControlMotion.KnobOffset(kind, on));

    // Controls.swift:657 turns on its animation; :1019, ListRow.swift:150 and
    // ProjectsSection.swift:428 rotate with none.
    [Fact]
    public void OnlyTheExpandersChevronTurnsTheOthersSnap()
    {
        Assert.True(GlassControlMotion.ChevronAnimates(GlassChevron.Expander));
        Assert.False(GlassControlMotion.ChevronAnimates(GlassChevron.SplitButton));
        Assert.False(GlassControlMotion.ChevronAnimates(GlassChevron.TreeRow));
        Assert.False(GlassControlMotion.ChevronAnimates(GlassChevron.Projects));
        Assert.Equal((0.0, 90.0), (GlassControlMotion.ExpanderAngle(false), GlassControlMotion.ExpanderAngle(true)));
    }

    // GlassSpinner.angle(at:reduceMotion:), Feedback.swift:38-42: the probe
    // printed 45 at 0.1 s, 90 at 0.2, 180 at 0.4, 355.5 at 0.79, 0 at 0.8,
    // 90 at 1.0, and 0 at every time under reduce motion. Clockwise: the
    // angle grows with time (a reversed port would fall).
    [Theory]
    [InlineData(0.0, 0.0)]
    [InlineData(0.1, 45.0)]
    [InlineData(0.2, 90.0)]
    [InlineData(0.4, 180.0)]
    [InlineData(0.79, 355.5)]
    [InlineData(0.8, 0.0)]
    [InlineData(1.0, 90.0)]
    public void TheSpinnerTurnsAsTheMacs(double t, double degrees)
    {
        Assert.Equal(degrees, GlassSpinnerMotion.Angle(t, reduceMotion: false), 6);
        Assert.Equal(0, GlassSpinnerMotion.Angle(t, reduceMotion: true));
    }

    [Fact]
    public void TheSpinnerIsAQuarterArcAt60()
    {
        Assert.Equal(1.0 / 60, GlassSpinnerMotion.FrameInterval, 12);
        Assert.Equal(0.25, GlassSpinnerMotion.Arc);
        Assert.Equal(0.18, GlassSpinnerMotion.TrackAlpha);
    }

    // GlassRefreshButton.angle(elapsed:isRefreshing:reduceMotion:),
    // Containers.swift:101-105: probe 90 at 0.2, 180 at 0.4, 360 at 0.8 and
    // after (a click completes one turn); refreshing, 90 at 1.0 (it keeps
    // turning).
    [Theory]
    [InlineData(0.0, false, 0.0)]
    [InlineData(0.2, false, 90.0)]
    [InlineData(0.4, false, 180.0)]
    [InlineData(0.8, false, 360.0)]
    [InlineData(1.2, false, 360.0)]
    [InlineData(0.4, true, 180.0)]
    [InlineData(1.0, true, 90.0)]
    public void TheRefreshIconTurnsAsTheMacs(double elapsed, bool refreshing, double degrees)
    {
        Assert.Equal(degrees, GlassRefreshMotion.Angle(elapsed, refreshing, reduceMotion: false), 6);
        Assert.Equal(0, GlassRefreshMotion.Angle(elapsed, refreshing, reduceMotion: true));
    }

    // Containers.swift:107-109 (probe: false; false; true) and :112 (30 fps).
    [Fact]
    public void TheRefreshClockRunsOnlyWhileTurning()
    {
        Assert.False(GlassRefreshMotion.Animates(isRefreshing: false, completingClick: false, reduceMotion: false));
        Assert.False(GlassRefreshMotion.Animates(isRefreshing: true, completingClick: false, reduceMotion: true));
        Assert.True(GlassRefreshMotion.Animates(isRefreshing: false, completingClick: true, reduceMotion: false));
        Assert.Equal(1.0 / 30, GlassRefreshMotion.FrameInterval, 12);
    }

    // GlassSkeleton: opacity(dim:) 0.5 and 1 (Feedback.swift:93-95); the
    // pulse is BezierAnimation(duration: 0.8, the token curve) repeated with
    // autoreverses (probe: the animation's own description); none under
    // reduce motion.
    [Fact]
    public void TheSkeletonBreathesBetweenOneAndAHalf()
    {
        Assert.Equal(0.8, GlassSkeletonMotion.Leg, 12);
        Assert.Equal(1, GlassSkeletonMotion.Opacity(0, reduceMotion: false), 9);
        Assert.Equal(0.5, GlassSkeletonMotion.Opacity(0.8, reduceMotion: false), 9);
        Assert.Equal(1, GlassSkeletonMotion.Opacity(1.6, reduceMotion: false), 9);
        // 0.2 s into the dimming leg: the token curve at 0.25 of the way.
        Assert.Equal(1 - 0.5 * 0.7672843933105469, GlassSkeletonMotion.Opacity(0.2, reduceMotion: false), 4);
        // Autoreversed: 0.2 s into the brightening leg retraces 0.75 of the way.
        Assert.Equal(1 - 0.5 * 0.9911079406738281, GlassSkeletonMotion.Opacity(1.0, reduceMotion: false), 4);
        Assert.Equal(1, GlassSkeletonMotion.Opacity(0.8, reduceMotion: true));
    }

    // GlassRuleTabs.landing (Navigation.swift:369-373), probe over tabs 0...3,
    // width 100: from 1 by 30 -> 1, by 49 -> 1, by 51 -> 2, by -151 -> 0;
    // from 0 by 400 -> 3. hasMoved at 6 (:364, :377).
    [Theory]
    [InlineData(1, 30.0, 1)]
    [InlineData(1, 49.0, 1)]
    [InlineData(1, 51.0, 2)]
    [InlineData(1, -151.0, 0)]
    [InlineData(0, 400.0, 3)]
    public void ADraggedTabLandsWhereTheMacs(int from, double offset, int slot) =>
        Assert.Equal(slot, GlassTabMotion.Landing(from, offset, 100, 0, 3));

    [Fact]
    public void TheTabsNumbersAreTheMacs()
    {
        Assert.Equal((0.3, 2.0, 26.0), (GlassTabMotion.EmberReach, GlassTabMotion.EmberLine, GlassTabMotion.HaloHeight));
        Assert.Equal((1.06, 2.0, 0.18), (GlassTabMotion.LiftScale, GlassTabMotion.LiftRise, GlassTabMotion.LiftShadowOpacity));
        Assert.False(GlassTabMotion.HasMoved(5.9));
        Assert.True(GlassTabMotion.HasMoved(-6));
        // The ember reaches 30% past each side of a 100 wide tab at 40.
        Assert.Equal((10.0, 160.0), GlassTabMotion.Ember(40, 100));
    }

    /// <summary>
    /// The clock gate (AnimationTimeline.swift:96-112): no frame while the
    /// system's animations are off, the element is not loaded, the window is
    /// hidden or minimised, or the caller pauses; and frames no closer than
    /// the interval. Preconditions are set explicitly, never read from the
    /// machine running the test.
    /// </summary>
    [Fact]
    public void TheClockTicksOnlyWhileItCanBeSeen()
    {
        var gate = new GlassFrameGate(1.0 / 30) { AnimationsEnabled = true, Loaded = true, WindowVisible = true };
        Assert.True(gate.Running);
        Assert.True(gate.ShouldDraw(0));
        Assert.False(gate.ShouldDraw(1.0 / 60));
        Assert.True(gate.ShouldDraw(1.0 / 30));

        gate.WindowVisible = false;
        Assert.False(gate.Running);
        Assert.False(gate.ShouldDraw(1));
        gate.WindowVisible = true;

        gate.AnimationsEnabled = false;
        Assert.False(gate.ShouldDraw(2));
        gate.AnimationsEnabled = true;

        gate.Loaded = false;
        Assert.False(gate.ShouldDraw(3));
        gate.Loaded = true;

        gate.Paused = true;
        Assert.False(gate.ShouldDraw(4));
        gate.Paused = false;
        Assert.True(gate.ShouldDraw(5));
    }

    [Fact]
    public void ADisplayRateClockDrawsEveryFrame()
    {
        var gate = new GlassFrameGate(GlassSpinnerMotion.FrameInterval) { Loaded = true };
        int drawn = Enumerable.Range(0, 120).Count(i => gate.ShouldDraw(i / 120.0));
        // A 120 Hz display: the spinner's 60 fps cap draws every other frame.
        Assert.Equal(60, drawn);
    }
}
