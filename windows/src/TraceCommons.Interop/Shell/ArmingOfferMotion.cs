using System;

namespace TraceCommons.Interop.Shell;

/// <summary>The arming mark's glyph (<c>ArmingOfferMotion.symbol</c>, ArmingOfferMotion.swift:21-23).</summary>
public enum ArmingMarkGlyph
{
    /// SF Symbol <c>bolt</c>.
    Bolt,

    /// SF Symbol <c>checkmark</c>.
    Check,
}

/// <summary>The arming mark's tint (<c>ArmingOfferMotion.tint</c>, ArmingOfferMotion.swift:26-28).</summary>
public enum ArmingMarkTint
{
    /// <c>GlassColor.accentText</c>.
    Accent,

    /// <c>GlassStatus.on.color</c>.
    StatusOn,
}

/// <summary>
/// How the arming offer answers its Enable (owner, 2026-11-12), ported from
/// macos/Sources/TraceCommonsApp/Views/Monitor/ArmingOfferMotion.swift at
/// origin/main 83016f544: the bolt turns to a check with a pulse once armed,
/// and after the hold the card leaves on its own spring. Every number is the
/// Mac's; the springs' sampled values and settling times were printed by a
/// probe compiled against SwiftUI's <c>Spring</c> on this Mac.
/// </summary>
/// <remarks>
/// The view draws one frame per state with animations off
/// (<c>UISettings.AnimationsEnabled</c> false): the check at once, no pulse,
/// and the card simply goes (ArmingOfferMotion.swift:35-43, ArmedPulse :57-75).
/// </remarks>
public static class ArmingOfferMotion
{
    /// <summary>The mark change: <c>.spring(response: 0.32, dampingFraction: 0.7)</c> (ArmingOfferMotion.swift:42).</summary>
    public const double MarkResponse = 0.32, MarkDamping = 0.7;

    /// <summary>The probe's <c>Spring(response: 0.32, dampingRatio: 0.7).settlingDuration</c>.</summary>
    public const double MarkSettling = 0.6983626670155922;

    /// <summary>The card's exit: <c>.spring(response: 0.42, dampingFraction: 0.9)</c> (ArmingOfferMotion.swift:37).</summary>
    public const double ExitResponse = 0.42, ExitDamping = 0.9;

    /// <summary>The probe's <c>Spring(response: 0.42, dampingRatio: 0.9).settlingDuration</c>.</summary>
    public const double ExitSettling = 0.7114853168979396;

    /// <summary>The pulse: <c>.easeOut(duration: 0.7)</c> (ArmingOfferMotion.swift:72).</summary>
    public const double PulseDuration = 0.7;

    /// <summary>The ring's scale at rest and spread (<c>scaleEffect(spread ? 1.9 : 1)</c>, :66).</summary>
    public const double PulseFromScale = 1, PulseToScale = 1.9;

    /// <summary>The ring's opacity as it starts (<c>0.8</c>, :67); it fades to 0 as it spreads.</summary>
    public const double PulseFromOpacity = 0.8;

    /// <summary>The ring's stroke (<c>strokeBorder(tint, lineWidth: 2)</c>, :65).</summary>
    public const double PulseStroke = 2;

    /// <summary>
    /// The mark's side (<c>GlassNoticeMark.side</c>, PopoverNotice.swift:143),
    /// its glyph size (<c>glassGlyph(12, weight: .semibold)</c>, :150) and its
    /// ring's stroke (:148). MainWindow.Arming.cs sizes the mark from these.
    /// </summary>
    public const double MarkSide = 30, GlyphSize = 12, MarkStroke = 2;

    /// <summary>SwiftUI's <c>easeOut</c>: the cubic Bezier (0, 0, 0.58, 1).</summary>
    public const double EaseOutX1 = 0, EaseOutY1 = 0, EaseOutX2 = 0.58, EaseOutY2 = 1;

    /// <summary>
    /// The scale the check grows in from. SwiftUI replaces the symbol with
    /// <c>.contentTransition(.symbolEffect(.replace))</c> (PopoverNotice.swift:154),
    /// which WinUI has no equivalent of; the check instead grows from half
    /// size on the mark spring, overshooting as the spring does. Not a Mac
    /// number: the stand-in, recorded under "Not ported" in the PR.
    /// </summary>
    public const double GlyphGrowFrom = 0.5;

    /// <summary>Motion runs only with animations on and the window's clocks running (checklist P1).</summary>
    public static bool MotionAllowed(bool animationsEnabled, bool clocksRunning) => animationsEnabled && clocksRunning;

    /// <summary>
    /// What a change of step from <paramref name="before"/> to
    /// <paramref name="after"/> does to the mark (AppModel.swift:2222-2259):
    /// nothing while no card is up (a leaving card keeps what it last showed)
    /// or when the step did not change; otherwise the step's end state, which
    /// is animated, with the pulse, only on the step to Armed with motion
    /// allowed.
    /// </summary>
    public static ArmingMarkPlan PlanMark(ArmingOfferStep? before, ArmingOfferStep? after, bool offerShown, bool motionAllowed)
    {
        if (!offerShown || before == after)
        {
            return new ArmingMarkPlan(Apply: false, Animate: false, Pulse: false);
        }

        bool animate = motionAllowed && after == ArmingOfferStep.Armed;
        return new ArmingMarkPlan(Apply: true, Animate: animate, Pulse: Pulses(before, after, motionAllowed));
    }

    /// <summary>The check's scale <paramref name="seconds"/> into the mark change.</summary>
    public static double GlyphScale(double seconds) => GlyphGrowFrom + (1 - GlyphGrowFrom) * MarkProgress(seconds);

    /// <summary>The mark's glyph for a step.</summary>
    public static ArmingMarkGlyph Glyph(ArmingOfferStep? step) =>
        step == ArmingOfferStep.Armed ? ArmingMarkGlyph.Check : ArmingMarkGlyph.Bolt;

    /// <summary>The mark's tint for a step.</summary>
    public static ArmingMarkTint Tint(ArmingOfferStep? step) =>
        step == ArmingOfferStep.Armed ? ArmingMarkTint.StatusOn : ArmingMarkTint.Accent;

    /// <summary>Whether a step change from <paramref name="before"/> to <paramref name="after"/> fires the pulse.</summary>
    public static bool Pulses(ArmingOfferStep? before, ArmingOfferStep? after, bool animationsEnabled) =>
        animationsEnabled && before != ArmingOfferStep.Armed && after == ArmingOfferStep.Armed;

    /// <summary>
    /// Whether the notice stack, emptied by the front card going from
    /// <paramref name="previousFront"/> to <paramref name="nextFront"/>, fades
    /// out on the exit spring instead of going at once: only when the card
    /// that went was the arming offer and it left armed. AppModel.swift:2255
    /// removes the armed offer inside <c>withAnimation(ArmingOfferMotion.exit)</c>,
    /// and the stack's <c>.transition(.opacity)</c> (MonitorNotices.swift:76)
    /// runs inside it; every other way a stack empties is unanimated on the
    /// Mac (NoticeMotion.Plan's remarks).
    /// </summary>
    public static bool StackFadesOut(string? previousFront, string? nextFront, bool leftArmed, bool animationsEnabled) =>
        animationsEnabled && leftArmed && nextFront is null && previousFront is not null
        && previousFront.StartsWith(NoticeCatalog.ArmingOffer(string.Empty).Id, StringComparison.Ordinal);

    /// <summary>The mark change's progress <paramref name="seconds"/> in, overshooting as SwiftUI's does.</summary>
    public static double MarkProgress(double seconds) => MonitorMotion.Spring(MarkResponse, MarkDamping, seconds);

    /// <summary>The exit's progress <paramref name="seconds"/> in.</summary>
    public static double ExitProgress(double seconds) => MonitorMotion.Spring(ExitResponse, ExitDamping, seconds);

    /// <summary>The leaving card's opacity <paramref name="seconds"/> in: 1 to 0 on the exit spring.</summary>
    public static double ExitOpacity(double seconds) => Math.Clamp(1 - ExitProgress(seconds), 0, 1);

    /// <summary>The pulse ring's scale <paramref name="seconds"/> in.</summary>
    public static double PulseScale(double seconds) =>
        PulseFromScale + (PulseToScale - PulseFromScale) * EaseOut(seconds / PulseDuration);

    /// <summary>The pulse ring's opacity <paramref name="seconds"/> in.</summary>
    public static double PulseOpacity(double seconds) => PulseFromOpacity * (1 - EaseOut(seconds / PulseDuration));

    /// <summary>SwiftUI's <c>UnitCurve.easeOut</c> at <paramref name="fraction"/> of its duration.</summary>
    public static double EaseOut(double fraction) =>
        MonitorMotion.CubicBezier(EaseOutX1, EaseOutY1, EaseOutX2, EaseOutY2, Math.Clamp(fraction, 0, 1));
}

/// <summary>What the window does to the arming mark for one change of step.</summary>
/// <param name="Apply">Whether the mark changes at all.</param>
/// <param name="Animate">Whether the glyph and inks move on the mark spring; otherwise they are set.</param>
/// <param name="Pulse">Whether the ring spreads from the mark.</param>
public readonly record struct ArmingMarkPlan(bool Apply, bool Animate, bool Pulse);
