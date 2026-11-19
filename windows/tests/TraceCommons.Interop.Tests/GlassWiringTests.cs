using System.Collections.Generic;
using System.Linq;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The clock's wiring (AnimationTimeline.swift:4-32, GlassAnimationTimeline)
/// with a fake frame source: frames are pumped by the test, and a clock
/// draws only while its element is shown with every ancestor and its own
/// window can be seen. Windows are plain objects; nothing reads the machine.
/// </summary>
public class GlassClockWiringTests
{
    private sealed class Window
    {
    }

    private sealed class FakeClock
    {
        public double Now;
        public int Subscribed;
        public readonly List<double> Drawn = new();
        public readonly GlassClockCore<Window> Core;

        public FakeClock(WindowClockRegistry<Window> windows, Window window, double interval = 1.0 / 60)
        {
            Core = new GlassClockCore<Window>(windows, interval, () => Subscribed++, () => Subscribed--, t => Drawn.Add(t), () => Now)
            {
                AnimationsEnabled = true,
                Window = window,
                Shown = true,
            };
            Core.Loaded = true;
        }

        /// <summary>Pumps <paramref name="count"/> display frames at 60 Hz and returns how many drew.</summary>
        public int Pump(int count = 30)
        {
            int before = Drawn.Count;
            for (int i = 0; i < count; i++)
            {
                Now += 1.0 / 60;
                if (Subscribed > 0)
                {
                    Core.OnFrame();
                }
            }

            return Drawn.Count - before;
        }
    }

    [Fact]
    public void AClockInACollapsedPaneDrawsNothing()
    {
        var windows = new WindowClockRegistry<Window>();
        var clock = new FakeClock(windows, new Window());
        Assert.Equal(30, clock.Pump());

        // The tab pane above it collapses: its own visibility still reads
        // visible, but the chain does not.
        clock.Core.Shown = EffectiveVisibility.Shown(new[] { true, true, false, true });
        Assert.False(clock.Core.Subscribed);
        Assert.Equal(0, clock.Subscribed);
        Assert.Equal(0, clock.Pump());
        Assert.False(clock.Core.MayMove);

        clock.Core.Shown = EffectiveVisibility.Shown(new[] { true, true, true, true });
        Assert.Equal(30, clock.Pump());
    }

    [Fact]
    public void AClockFollowsItsOwnWindowAndNoOther()
    {
        var windows = new WindowClockRegistry<Window>();
        Window main = new(), settings = new();
        var inMain = new FakeClock(windows, main);
        var inSettings = new FakeClock(windows, settings);

        // The main window goes to the tray: the Settings window keeps moving.
        windows.Set(main, false);
        Assert.Equal(0, inMain.Pump());
        Assert.Equal(30, inSettings.Pump());
        Assert.True(inSettings.Core.MayMove);
        Assert.False(inMain.Core.MayMove);

        // The Settings window is minimised while the main window is back.
        windows.Set(main, true);
        windows.Set(settings, false);
        Assert.Equal(30, inMain.Pump());
        Assert.Equal(0, inSettings.Pump());
        Assert.Equal(0, inSettings.Subscribed);

        windows.Set(settings, true);
        Assert.Equal(30, inSettings.Pump());

        // A stopped clock is not disturbed by another window's changes: it
        // drew its resting frame once, and draws nothing more.
        windows.Set(main, false);
        int rested = inMain.Drawn.Count;
        windows.Set(settings, false);
        windows.Set(settings, true);
        Assert.Equal(rested, inMain.Drawn.Count);
    }

    [Fact]
    public void AStoppedClockDrawsItsRestingFrameOnceAndAnUnloadedOneStopsListening()
    {
        var windows = new WindowClockRegistry<Window>();
        Window window = new();
        var clock = new FakeClock(windows, window);
        clock.Pump(3);
        int drawn = clock.Drawn.Count;
        clock.Core.Paused = true;
        Assert.Equal(drawn + 1, clock.Drawn.Count);
        Assert.Equal(0, clock.Pump());

        clock.Core.Paused = false;
        clock.Core.Loaded = false;
        Assert.Equal(0, clock.Subscribed);
        // Unloaded, a change of its window's gate is no longer its concern.
        int afterUnload = clock.Drawn.Count;
        windows.Set(window, false);
        windows.Set(window, true);
        Assert.Equal(afterUnload, clock.Drawn.Count);
        Assert.Equal(0, clock.Subscribed);
    }

    [Fact]
    public void AnElementInNoWindowMayNotMoveAnUnreportedWindowMayAndAClosedOneStaysStopped()
    {
        var windows = new WindowClockRegistry<Window>();
        Assert.False(windows.MayTick(null));
        Assert.True(windows.MayTick(new Window()));

        // A clock still loaded in a window that has closed never ticks again.
        Window closing = new();
        var clock = new FakeClock(windows, closing);
        var gate = new WindowClockGate();
        gate.Changed += mayTick => windows.Set(closing, mayTick);
        gate.SetClosed();
        Assert.False(windows.MayTick(closing));
        Assert.Equal(0, clock.Pump());
    }

    [Fact]
    public void ARunCutShortNeverWritesItsEndOverTheNextOne()
    {
        var runs = new MotionRuns<string>();
        int first = runs.Begin("Translation.X");
        int second = runs.Begin("Translation.X");
        Assert.False(runs.IsCurrent("Translation.X", first));
        Assert.True(runs.IsCurrent("Translation.X", second));
        // Another property's runs are its own.
        int scale = runs.Begin("Scale.X");
        Assert.True(runs.IsCurrent("Scale.X", scale));
        Assert.True(runs.IsCurrent("Translation.X", second));
    }

    // Home (x 12, 60 wide) to Traces (x 150, 80 wide), then 0.1 s in on to
    // Insights (x 300, 70 wide): the second glide starts where the ember is
    // drawn 0.1 s into the first, not from Traces at rest.
    [Fact]
    public void ASecondSwitchMidGlideStartsWhereTheEmberIsDrawn()
    {
        (double tx, double sx) = TabEmberGlide.From(12, 60, 150, 80);
        (double atTx, double atSx) = TabEmberGlide.At(tx, sx, 0.1);
        double drawnCentre = 150 + 40 + atTx, drawnWidth = 80 * atSx;

        (double rx, double rs) = TabEmberGlide.Retarget(150, 80, tx, sx, 0.1, 300, 70);
        Assert.Equal(drawnCentre - (300 + 35), rx, 9);
        Assert.Equal(drawnWidth / 70, rs, 9);

        (double fromRest, double scaleFromRest) = TabEmberGlide.From(150, 80, 300, 70);
        Assert.NotEqual(fromRest, rx, 3);
        Assert.NotEqual(scaleFromRest, rs, 3);

        // A glide that has finished retargets from its rest, as From does.
        (double settledX, double settledS) = TabEmberGlide.Retarget(150, 80, tx, sx, 10, 300, 70);
        Assert.Equal(fromRest, settledX, 9);
        Assert.Equal(scaleFromRest, settledS, 9);
    }
}

/// <summary>
/// The re-templated controls' drawn motion (GlassTemplateMotion), which the
/// templates take from these plans and nowhere else. Expected values are
/// the Mac's: Controls.swift:1144-1177 (GlassToggleStyle), GlassPress.swift,
/// GlassHover.swift:40, FirstRunStage.swift:463-492.
/// </summary>
public class GlassTemplateMotionTests
{
    private static TransitionPlan Into(GlassTemplateKind kind, string group, string to) =>
        GlassTemplateMotion.Groups(kind).Single(g => g.Group == group).Transitions.Single(t => t.To == to);

    private static IReadOnlyList<PartValue> State(GlassTemplateKind kind, string group, string state) =>
        GlassTemplateMotion.Groups(kind).Single(g => g.Group == group).States.Single(s => s.State == state).Values;

    private static readonly SplinePoints EaseOut = new(0, 0, 0.58, 1);
    private static readonly SplinePoints TokenCurve = new(0.2, 0.8, 0.2, 1);

    [Theory]
    [InlineData(GlassTemplateKind.Switch, 0.22, 16)]
    [InlineData(GlassTemplateKind.WatchSwitch, 0.15, 16)]
    public void TheSwitchKnobSlidesOnTheMacsEaseOut(GlassTemplateKind kind, double seconds, double travel)
    {
        foreach (string to in new[] { "On", "Off" })
        {
            TransitionPlan slide = Into(kind, "ToggleStates", to);
            Assert.Null(slide.From);
            Assert.Equal(seconds, slide.Seconds, 9);
            Assert.Equal(EaseOut, slide.Spline);
            Assert.Equal(to == "On" ? travel : 0, slide.Values.Single(v => v.Part == "KnobTranslateTransform" && v.Property == "X").Value);
            Assert.Equal(to == "On" ? 1 : 0, slide.Values.Single(v => v.Part == "OnTrack").Value);
            Assert.Equal(to == "On" ? 1 : 0, slide.Values.Single(v => v.Part == "KnobOn").Value);
        }

        Assert.Equal(travel, State(kind, "ToggleStates", "On").Single(v => v.Property == "X").Value);
        Assert.Equal(0, State(kind, "ToggleStates", "Off").Single(v => v.Property == "X").Value);
    }

    [Fact]
    public void TheFirstRunSwitchIsSmallerOnTheTokenCurveWithItsLabelAfter()
    {
        TransitionPlan slide = Into(GlassTemplateKind.FirstRunSwitch, "ToggleStates", "On");
        Assert.Equal(0.2, slide.Seconds, 9);
        Assert.Equal(TokenCurve, slide.Spline);
        // 30 wide, a 14 knob padded 2 each side: 12 of travel.
        Assert.Equal(12, GlassTemplateMotion.KnobTravel(GlassTemplateKind.FirstRunSwitch));
        Assert.Equal(new PartFrame("SwitchArea", 30, 18, 0, 0), GlassTemplateMotion.Frames(GlassTemplateKind.FirstRunSwitch)[0]);
        Assert.Equal(new PartFrame("SwitchKnob", 14, 14, 2, 0), GlassTemplateMotion.Frames(GlassTemplateKind.FirstRunSwitch)[2]);
        Assert.False(GlassTemplateMotion.LabelLeads(GlassTemplateKind.FirstRunSwitch));
        Assert.Equal(10, GlassTemplateMotion.LabelGap(GlassTemplateKind.FirstRunSwitch));
        // A plain button on the Mac: no press wash.
        Assert.Empty(GlassTemplateMotion.Groups(GlassTemplateKind.FirstRunSwitch).Single(g => g.Group == "CommonStates").Transitions);
    }

    [Fact]
    public void TheSwitchIsTheMacsSizeWithItsLabelFirst()
    {
        IReadOnlyList<PartFrame> frames = GlassTemplateMotion.Frames(GlassTemplateKind.Switch);
        Assert.Equal(new PartFrame("SwitchArea", 40, 24, 0, 0), frames[0]);
        Assert.Equal(3, frames[1].Left);
        Assert.Equal(3, frames[1].Right);
        Assert.Equal(new PartFrame("SwitchKnob", 18, 18, 3, 0), frames[2]);
        IReadOnlyList<PartFrame> watch = GlassTemplateMotion.Frames(GlassTemplateKind.WatchSwitch);
        Assert.Equal(new PartFrame("SwitchArea", 38, 22, 0, 0), watch[0]);
        Assert.Equal(new PartFrame("SwitchKnob", 18, 18, 2, 0), watch[2]);
        // HStack(spacing: Space.s6), label first.
        Assert.True(GlassTemplateMotion.LabelLeads(GlassTemplateKind.Switch));
        Assert.Equal(12, GlassTemplateMotion.LabelGap(GlassTemplateKind.Switch));
    }

    [Theory]
    [InlineData(GlassTemplateKind.Switch)]
    [InlineData(GlassTemplateKind.WatchSwitch)]
    public void ThePressWashFadesOverFastOnTheTokenCurve(GlassTemplateKind kind)
    {
        GroupPlan common = GlassTemplateMotion.Groups(kind).Single(g => g.Group == "CommonStates");
        TransitionPlan down = common.Transitions.Single(t => t.To == "Pressed");
        TransitionPlan up = common.Transitions.Single(t => t.From == "Pressed");
        foreach (TransitionPlan t in new[] { down, up })
        {
            Assert.Equal(0.15, t.Seconds, 9);
            Assert.Equal(TokenCurve, t.Spline);
        }

        Assert.Equal(1, down.Values.Single(v => v.Part == "PressWash").Value);
        Assert.Equal(0, up.Values.Single(v => v.Part == "PressWash").Value);
    }

    [Fact]
    public void TheRefreshPillsHoverAndPressFadeOverFastOnTheTokenCurve()
    {
        GroupPlan common = GlassTemplateMotion.Groups(GlassTemplateKind.RefreshPill).Single();
        Assert.Equal(new[] { "PointerOver", "Pressed", "Normal" }, common.Transitions.Select(t => t.To));
        foreach (TransitionPlan t in common.Transitions)
        {
            Assert.Equal(0.15, t.Seconds, 9);
            Assert.Equal(TokenCurve, t.Spline);
        }

        Assert.Equal((1.0, 0.0), Values(common.Transitions[0]));
        Assert.Equal((1.0, 1.0), Values(common.Transitions[1]));
        Assert.Equal((0.0, 0.0), Values(common.Transitions[2]));

        static (double Hover, double Press) Values(TransitionPlan t) =>
            (t.Values.Single(v => v.Part == "HoverFill").Value, t.Values.Single(v => v.Part == "PressWash").Value);
    }

    [Fact]
    public void ASpringIsNeverAKeySpline()
    {
        Assert.Throws<System.ArgumentException>(() => GlassTemplateMotion.Spline(GlassControlMotion.TabEmberGlide));
    }
}
