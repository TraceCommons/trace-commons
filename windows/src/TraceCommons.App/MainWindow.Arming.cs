using System;
using System.Numerics;
using Microsoft.UI.Composition;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Hosting;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App;

/// <summary>
/// The arming offer's mark: the bolt that turns to a check with a pulse once
/// the daemon has armed the project (ArmingOfferMotion.swift, MonitorNotices.swift:349-356,
/// PopoverNotice.swift:116-159 at origin/main 83016f544). Every number and
/// every decision is <see cref="ArmingOfferMotion"/>'s, tested there; this
/// file only applies them. The compositor draws the key frames at
/// the display's rate, as SwiftUI does. Nothing runs with animations off
/// (<c>UISettings.AnimationsEnabled</c>, read at each change, so a change in
/// Settings applies at once) or while the window's clocks are stopped; the
/// mark then shows its end state.
/// </summary>
public sealed partial class MainWindow
{
    private ArmingOfferStep? _armingStepShown;

    private void InitArmingMotion()
    {
        ViewModel.Arming.Changed += OnArmingChanged;
        _clockGate.Changed += mayTick =>
        {
            if (!mayTick)
            {
                FinishArmingMotion();
            }
        };
        SizeArmingMark();
        ApplyArmingMark(animate: false);
        ElementCompositionPreview.GetElementVisual(ArmingPulse).Opacity = 0;
    }

    /// <summary>The Mac's notice coin: side, ring and glyph from <see cref="ArmingOfferMotion"/>, the only copy.</summary>
    private void SizeArmingMark()
    {
        ArmingMark.Width = ArmingMark.Height = ArmingOfferMotion.MarkSide;
        ArmingMarkRing.StrokeThickness = ArmingMarkRingOn.StrokeThickness = ArmingOfferMotion.MarkStroke;
        ArmingPulse.StrokeThickness = ArmingOfferMotion.PulseStroke;
        ArmingMarkBolt.FontSize = ArmingMarkCheck.FontSize = ArmingOfferMotion.GlyphSize;
    }

    private bool ArmingMotionAllowed => ArmingOfferMotion.MotionAllowed(_uiSettings.AnimationsEnabled, _clockGate.MayTick);

    /// <summary>
    /// Whether the card now leaving was armed, so it leaves on the arming
    /// exit spring. The flow sets this before it announces the change, so the
    /// notice stack's fade reads it whichever handler runs first.
    /// </summary>
    private bool ArmingExitPending() => ViewModel.Arming.LeftArmed;

    private void OnArmingChanged()
    {
        ArmingOfferStep? after = ViewModel.Arming.Step;
        // A leaving card keeps what it last showed (a SwiftUI removal draws
        // the view's last state); the mark is reset for the next offer.
        ArmingMarkPlan plan = ArmingOfferMotion.PlanMark(
            _armingStepShown, after, ViewModel.Arming.Offer is not null, ArmingMotionAllowed);
        if (!plan.Apply)
        {
            return;
        }

        _armingStepShown = after;
        ApplyArmingMark(plan.Animate);
        if (plan.Pulse)
        {
            Pulse();
        }
    }

    /// <summary>
    /// The glyph and its tint for the step: bolt in the accent, check in the
    /// "on" colour. Animated, the check replaces the bolt on the mark spring,
    /// growing in and overshooting as the Mac's symbol bounce does, while the
    /// inks crossfade on the same spring.
    /// </summary>
    private void ApplyArmingMark(bool animate)
    {
        bool check = ArmingOfferMotion.Glyph(ViewModel.Arming.Step) == ArmingMarkGlyph.Check;
        bool on = ArmingOfferMotion.Tint(ViewModel.Arming.Step) == ArmingMarkTint.StatusOn;
        FrameworkElement shown = check ? ArmingMarkCheck : ArmingMarkBolt;
        StopMark();
        if (!animate)
        {
            SetMarkAtRest(check, on);
            return;
        }

        Fade(ArmingMarkCheck, check);
        Fade(ArmingMarkBolt, !check);
        Fade(ArmingMarkRingOn, on);
        Fade(ArmingMarkRing, !on);
        Visual glyph = ElementCompositionPreview.GetElementVisual(shown);
        CenterPoint(glyph, shown);
        RunScale(glyph, ArmingOfferMotion.GlyphScale, ArmingOfferMotion.MarkSettling);
    }

    private static void Fade(UIElement element, bool toShown)
    {
        Visual visual = ElementCompositionPreview.GetElementVisual(element);
        Compositor compositor = visual.Compositor;
        CompositionEasingFunction linear = compositor.CreateLinearEasingFunction();
        ScalarKeyFrameAnimation animation = compositor.CreateScalarKeyFrameAnimation();
        float from = visual.Opacity;
        float to = toShown ? 1 : 0;
        foreach ((double fraction, double value) in MonitorMotion.KeyFrames(
                     t => Math.Clamp(ArmingOfferMotion.MarkProgress(t), 0, 1), ArmingOfferMotion.MarkSettling))
        {
            animation.InsertKeyFrame((float)fraction, from + (to - from) * (float)value, linear);
        }

        animation.Duration = TimeSpan.FromSeconds(ArmingOfferMotion.MarkSettling);
        visual.StartAnimation("Opacity", animation);
    }

    private void StopMark()
    {
        foreach (UIElement element in new UIElement[] { ArmingMarkBolt, ArmingMarkCheck, ArmingMarkRing, ArmingMarkRingOn })
        {
            Visual visual = ElementCompositionPreview.GetElementVisual(element);
            visual.StopAnimation("Opacity");
            visual.StopAnimation("Scale");
        }
    }

    private void SetMarkAtRest(bool check, bool on)
    {
        ElementCompositionPreview.GetElementVisual(ArmingMarkCheck).Opacity = check ? 1 : 0;
        ElementCompositionPreview.GetElementVisual(ArmingMarkBolt).Opacity = check ? 0 : 1;
        ElementCompositionPreview.GetElementVisual(ArmingMarkRingOn).Opacity = on ? 1 : 0;
        ElementCompositionPreview.GetElementVisual(ArmingMarkRing).Opacity = on ? 0 : 1;
        ElementCompositionPreview.GetElementVisual(ArmingMarkCheck).Scale = Vector3.One;
        ElementCompositionPreview.GetElementVisual(ArmingMarkBolt).Scale = Vector3.One;
    }

    /// <summary>The ring spreads from 1 to 1.9 and fades from 0.8 on ease-out over 0.7 s, once.</summary>
    private void Pulse()
    {
        Visual ring = ElementCompositionPreview.GetElementVisual(ArmingPulse);
        CenterPoint(ring, ArmingPulse);
        Compositor compositor = ring.Compositor;
        CompositionEasingFunction linear = compositor.CreateLinearEasingFunction();

        ScalarKeyFrameAnimation opacity = compositor.CreateScalarKeyFrameAnimation();
        Vector3KeyFrameAnimation scale = compositor.CreateVector3KeyFrameAnimation();
        foreach ((double fraction, double value) in MonitorMotion.KeyFrames(ArmingOfferMotion.PulseOpacity, ArmingOfferMotion.PulseDuration))
        {
            opacity.InsertKeyFrame((float)fraction, (float)value, linear);
        }

        foreach ((double fraction, double value) in MonitorMotion.KeyFrames(ArmingOfferMotion.PulseScale, ArmingOfferMotion.PulseDuration))
        {
            scale.InsertKeyFrame((float)fraction, new Vector3((float)value, (float)value, 1), linear);
        }

        opacity.Duration = scale.Duration = TimeSpan.FromSeconds(ArmingOfferMotion.PulseDuration);
        ring.StartAnimation("Opacity", opacity);
        ring.StartAnimation("Scale", scale);
    }

    /// <summary>Every arming motion at its end state (checklist P1): the step's glyph whole, no ring.</summary>
    private void FinishArmingMotion()
    {
        StopMark();
        SetMarkAtRest(
            ArmingOfferMotion.Glyph(ViewModel.Arming.Step) == ArmingMarkGlyph.Check,
            ArmingOfferMotion.Tint(ViewModel.Arming.Step) == ArmingMarkTint.StatusOn);
        Visual ring = ElementCompositionPreview.GetElementVisual(ArmingPulse);
        ring.StopAnimation("Opacity");
        ring.StopAnimation("Scale");
        ring.Opacity = 0;
        ring.Scale = Vector3.One;
        EndArmingStackExit();
    }

    private FrameworkElement? _armingStackLeaving;
    private int _armingStackExitToken;

    /// <summary>
    /// When the armed offer was the last card, the whole stack fades on the
    /// exit spring rather than going at once (<see cref="ArmingOfferMotion.StackFadesOut"/>;
    /// AppModel.swift:2255, MonitorNotices.swift:76). The card stays drawn
    /// under the fading stack and takes no input; returns false, changing
    /// nothing, for every other way the stack empties.
    /// </summary>
    private bool BeginArmingStackExit(string? previousFront, FrameworkElement? previousCard)
    {
        // A refresh while the stack is already leaving and still empty: the
        // fade carries on, and the card the refresh collapsed is drawn again.
        if (_armingStackLeaving is FrameworkElement leaving)
        {
            leaving.Visibility = Visibility.Visible;
            return true;
        }

        if (previousCard != ArmingOfferCard
            || !ArmingOfferMotion.StackFadesOut(previousFront, null, ArmingExitPending(), ArmingMotionAllowed))
        {
            return false;
        }

        _armingStackLeaving = ArmingOfferCard;
        ArmingOfferCard.Visibility = Visibility.Visible;
        NoticeStackHost.IsHitTestVisible = false;
        int token = ++_armingStackExitToken;
        Animate(NoticeStackHost, "Opacity", ArmingOfferMotion.ExitOpacity, () =>
        {
            if (token == _armingStackExitToken)
            {
                EndArmingStackExit();
            }
        }, ArmingOfferMotion.ExitSettling);
        return true;
    }

    /// <summary>
    /// Ends the stack's exit fade, early or on time: the stack whole again
    /// for its next card, and collapsed if no card has come since.
    /// </summary>
    private void EndArmingStackExit()
    {
        if (_armingStackLeaving is not FrameworkElement card)
        {
            return;
        }

        _armingStackLeaving = null;
        _armingStackExitToken++;
        Visual host = ElementCompositionPreview.GetElementVisual(NoticeStackHost);
        host.StopAnimation("Opacity");
        host.Opacity = 1;
        NoticeStackHost.IsHitTestVisible = true;
        if (card != _frontCard)
        {
            card.Visibility = Visibility.Collapsed;
        }

        if (_frontCard is null)
        {
            NoticeStackHost.Visibility = Visibility.Collapsed;
        }
    }

    private static void RunScale(Visual visual, Func<double, double> scaleAt, double settling)
    {
        Compositor compositor = visual.Compositor;
        CompositionEasingFunction linear = compositor.CreateLinearEasingFunction();
        Vector3KeyFrameAnimation animation = compositor.CreateVector3KeyFrameAnimation();
        foreach ((double fraction, double value) in MonitorMotion.KeyFrames(scaleAt, settling))
        {
            animation.InsertKeyFrame((float)fraction, new Vector3((float)value, (float)value, 1), linear);
        }

        animation.Duration = TimeSpan.FromSeconds(settling);
        visual.StartAnimation("Scale", animation);
    }

    private static void CenterPoint(Visual visual, FrameworkElement element) =>
        visual.CenterPoint = new Vector3((float)(element.ActualWidth / 2), (float)(element.ActualHeight / 2), 0);
}
