using System;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The arming offer's motion, pinned to the Mac
/// (macos/Sources/TraceCommonsApp/Views/Monitor/ArmingOfferMotion.swift at
/// origin/main 83016f544). Expected curve values were printed by a throwaway
/// probe compiled against SwiftUI on this Mac:
/// <c>Spring(response:dampingRatio:).value(target: 1, time:)</c>,
/// <c>.settlingDuration</c> and <c>UnitCurve.easeOut.value(at:)</c>.
/// </summary>
public class ArmingOfferMotionTests
{
    // probe: Spring(response: 0.32, dampingRatio: 0.7).value(target: 1, time: t)
    [Theory]
    [InlineData(0.05, 0.29764276018486346)]
    [InlineData(0.1, 0.7131000954174702)]
    [InlineData(0.2, 1.0396433516394763)]
    [InlineData(0.3, 1.0217265735181065)]
    [InlineData(0.4, 0.9993072748634816)]
    public void TheMarkChangesOnTheMacsSpring(double t, double want)
    {
        Assert.Equal(want, ArmingOfferMotion.MarkProgress(t), 6);
    }

    // probe: Spring(response: 0.42, dampingRatio: 0.9).value(target: 1, time: t)
    [Theory]
    [InlineData(0.05, 0.17946819309559603)]
    [InlineData(0.1, 0.4672107164086582)]
    [InlineData(0.2, 0.8473379890446229)]
    [InlineData(0.3, 0.9729266454733057)]
    [InlineData(0.4, 0.9991368011487772)]
    public void TheCardLeavesOnTheMacsExitSpring(double t, double want)
    {
        Assert.Equal(want, ArmingOfferMotion.ExitProgress(t), 6);
        Assert.Equal(Math.Clamp(1 - want, 0, 1), ArmingOfferMotion.ExitOpacity(t), 6);
    }

    [Fact]
    public void EachSpringRunsForSwiftUIsSettlingTime()
    {
        // probe: .settlingDuration for each.
        Assert.Equal(0.6983626670155922, ArmingOfferMotion.MarkSettling);
        Assert.Equal(0.7114853168979396, ArmingOfferMotion.ExitSettling);
        Assert.True(Math.Abs(1 - ArmingOfferMotion.MarkProgress(ArmingOfferMotion.MarkSettling)) < 0.01);
        Assert.True(Math.Abs(1 - ArmingOfferMotion.ExitProgress(ArmingOfferMotion.ExitSettling)) < 0.01);
    }

    // probe: UnitCurve.easeOut.value(at: p) (Float precision in SwiftUI).
    [Theory]
    [InlineData(0.1, 0.16057205200195312)]
    [InlineData(0.25, 0.37813854217529297)]
    [InlineData(0.5, 0.6846427917480469)]
    [InlineData(0.75, 0.9065351486206055)]
    public void ThePulseEasesOutAsSwiftUIsEaseOut(double p, double want)
    {
        Assert.Equal(want, ArmingOfferMotion.EaseOut(p), 4);
    }

    /// <summary>
    /// ArmedPulse (ArmingOfferMotion.swift:57-75): the ring spreads OUT from
    /// the mark, 1 to 1.9, while it fades from 0.8 to 0 over 0.7 s. A port
    /// that shrinks the ring or fades it in reads the wrong sign here.
    /// </summary>
    [Fact]
    public void TheRingSpreadsOutwardAndFades()
    {
        Assert.Equal(1, ArmingOfferMotion.PulseScale(0), 9);
        Assert.Equal(0.8, ArmingOfferMotion.PulseOpacity(0), 9);
        Assert.Equal(1 + 0.9 * 0.6846427917480469, ArmingOfferMotion.PulseScale(0.35), 4);
        Assert.Equal(1.9, ArmingOfferMotion.PulseScale(0.7), 9);
        Assert.Equal(0, ArmingOfferMotion.PulseOpacity(0.7), 9);
        double previous = ArmingOfferMotion.PulseScale(0);
        for (double t = 0.05; t <= 0.7; t += 0.05)
        {
            double now = ArmingOfferMotion.PulseScale(t);
            Assert.True(now > previous, $"scale fell at {t}");
            previous = now;
        }
    }

    [Fact]
    public void TheBoltTurnsToAGreenCheckOnlyOnceArmed()
    {
        Assert.Equal(ArmingMarkGlyph.Bolt, ArmingOfferMotion.Glyph(null));
        Assert.Equal(ArmingMarkGlyph.Bolt, ArmingOfferMotion.Glyph(ArmingOfferStep.Arming));
        Assert.Equal(ArmingMarkGlyph.Check, ArmingOfferMotion.Glyph(ArmingOfferStep.Armed));
        Assert.Equal(ArmingMarkTint.Accent, ArmingOfferMotion.Tint(ArmingOfferStep.Arming));
        Assert.Equal(ArmingMarkTint.StatusOn, ArmingOfferMotion.Tint(ArmingOfferStep.Armed));
    }

    [Fact]
    public void ThePulseFiresOnceOnArmingAndNeverUnderReduceMotion()
    {
        Assert.True(ArmingOfferMotion.Pulses(ArmingOfferStep.Arming, ArmingOfferStep.Armed, animationsEnabled: true));
        Assert.False(ArmingOfferMotion.Pulses(ArmingOfferStep.Arming, ArmingOfferStep.Armed, animationsEnabled: false));
        Assert.False(ArmingOfferMotion.Pulses(ArmingOfferStep.Armed, ArmingOfferStep.Armed, animationsEnabled: true));
        Assert.False(ArmingOfferMotion.Pulses(null, ArmingOfferStep.Arming, animationsEnabled: true));
    }

    [Fact]
    public void TheMarkIsTheMacsNoticeCoin()
    {
        // PopoverNotice.swift:143 side 30, :150 glyph 12, :148 stroke 2;
        // ArmingOfferMotion.swift:65 ring lineWidth 2. MainWindow.Arming.cs
        // sizes the mark from these, so they are the only copy.
        Assert.Equal(30, ArmingOfferMotion.MarkSide);
        Assert.Equal(12, ArmingOfferMotion.GlyphSize);
        Assert.Equal(2, ArmingOfferMotion.MarkStroke);
        Assert.Equal(2, ArmingOfferMotion.PulseStroke);
    }

    // AppModel.swift:2255 removes the armed offer inside
    // withAnimation(ArmingOfferMotion.exit), and the stack's own
    // .transition(.opacity) (MonitorNotices.swift:76) runs inside it: when the
    // armed offer is the last card, the whole stack fades on the exit spring.
    // Any other last card, a declined offer, or reduce motion: it goes at once
    // (NoticeMotion.Plan's remarks).
    [Fact]
    public void TheStackFadesOnTheExitSpringOnlyWhenTheArmedOfferWasTheLastCard()
    {
        string offer = NoticeCatalog.ArmingOffer("p-api").Id;
        Assert.True(ArmingOfferMotion.StackFadesOut(offer, null, leftArmed: true, animationsEnabled: true));
        Assert.False(ArmingOfferMotion.StackFadesOut(offer, null, leftArmed: true, animationsEnabled: false));
        Assert.False(ArmingOfferMotion.StackFadesOut(offer, null, leftArmed: false, animationsEnabled: true));
        Assert.False(ArmingOfferMotion.StackFadesOut(offer, "update", leftArmed: true, animationsEnabled: true));
        Assert.False(ArmingOfferMotion.StackFadesOut("update", null, leftArmed: true, animationsEnabled: true));
        Assert.False(ArmingOfferMotion.StackFadesOut(null, null, leftArmed: true, animationsEnabled: true));
    }

    /// <summary>
    /// What the window does with each change of step (AppModel.swift:2222-2259,
    /// ArmedPulse :57-75): nothing while no card is up or the step did not
    /// change; the mark animates and pulses only on the step to Armed with
    /// motion allowed; any other change, or no motion, sets the end state.
    /// </summary>
    [Fact]
    public void EachStepChangeIsPlannedAsTheMacDrawsIt()
    {
        ArmingMarkPlan armed = ArmingOfferMotion.PlanMark(ArmingOfferStep.Arming, ArmingOfferStep.Armed, offerShown: true, motionAllowed: true);
        Assert.Equal(new ArmingMarkPlan(Apply: true, Animate: true, Pulse: true), armed);

        Assert.Equal(new ArmingMarkPlan(Apply: true, Animate: false, Pulse: false),
            ArmingOfferMotion.PlanMark(ArmingOfferStep.Arming, ArmingOfferStep.Armed, offerShown: true, motionAllowed: false));
        Assert.Equal(new ArmingMarkPlan(Apply: true, Animate: false, Pulse: false),
            ArmingOfferMotion.PlanMark(null, ArmingOfferStep.Arming, offerShown: true, motionAllowed: true));
        Assert.Equal(new ArmingMarkPlan(Apply: true, Animate: false, Pulse: false),
            ArmingOfferMotion.PlanMark(ArmingOfferStep.Arming, null, offerShown: true, motionAllowed: true));
        Assert.Equal(new ArmingMarkPlan(Apply: false, Animate: false, Pulse: false),
            ArmingOfferMotion.PlanMark(ArmingOfferStep.Armed, ArmingOfferStep.Armed, offerShown: true, motionAllowed: true));
        // The card leaves showing what it last showed: no change is applied.
        Assert.Equal(new ArmingMarkPlan(Apply: false, Animate: false, Pulse: false),
            ArmingOfferMotion.PlanMark(ArmingOfferStep.Armed, null, offerShown: false, motionAllowed: true));
    }

    /// <summary>
    /// Motion runs only with animations on and the window's clocks running
    /// (checklist P1): a minimised or hidden window draws end states.
    /// </summary>
    [Theory]
    [InlineData(true, true, true)]
    [InlineData(false, true, false)]
    [InlineData(true, false, false)]
    [InlineData(false, false, false)]
    public void MotionNeedsAnimationsOnAndTheClocksRunning(bool animations, bool clocks, bool allowed)
    {
        Assert.Equal(allowed, ArmingOfferMotion.MotionAllowed(animations, clocks));
    }

    /// <summary>
    /// The check grows in from half size on the mark spring, overshooting
    /// past whole and settling at whole: the stand-in for SwiftUI's
    /// <c>.symbolEffect(.replace)</c> (PopoverNotice.swift:154), which has no
    /// WinUI equivalent. It grows, never shrinks in.
    /// </summary>
    [Fact]
    public void TheCheckGrowsInOvershootsAndSettlesWhole()
    {
        Assert.Equal(0.5, ArmingOfferMotion.GlyphScale(0), 9);
        Assert.True(ArmingOfferMotion.GlyphScale(0.2) > 1, "no overshoot");
        Assert.Equal(1, ArmingOfferMotion.GlyphScale(ArmingOfferMotion.MarkSettling), 2);
        Assert.True(ArmingOfferMotion.GlyphScale(0.05) > ArmingOfferMotion.GlyphScale(0));
    }
}
