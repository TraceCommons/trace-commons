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
/// the daemon has armed the project (ArmingOfferMotion.swift, MonitorNotices.swift:880-884,
/// PopoverNotice.swift:146-158 at origin/main 409066936). Every number is
/// <see cref="ArmingOfferMotion"/>'s; the compositor draws the key frames at
/// the display's rate, as SwiftUI does. Nothing runs with animations off
/// (<c>UISettings.AnimationsEnabled</c>, read at each change, so a change in
/// Settings applies at once) or while the window's clocks are stopped; the
/// mark then shows its end state.
/// </summary>
public sealed partial class MainWindow
{
    private ArmingOfferStep? _armingStepShown;
    private bool _armingExitArmed;

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
        ApplyArmingMark(animate: false);
    }

    private bool ArmingMotionAllowed => _uiSettings.AnimationsEnabled && _clockGate.MayTick;

    /// <summary>
    /// Whether the card now leaving was armed, so it leaves on the arming
    /// exit spring. Read once by the notice stack's fade.
    /// </summary>
    private bool ArmingExitPending()
    {
        bool armed = _armingExitArmed;
        _armingExitArmed = false;
        return armed;
    }

    private void OnArmingChanged()
    {
        ArmingOfferStep? before = _armingStepShown;
        ArmingOfferStep? after = ViewModel.Arming.Step;
        if (ViewModel.Arming.Offer is null)
        {
            // The card leaves showing what it last showed (a SwiftUI removal
            // draws the view's last state); the mark is reset for the next
            // offer when one arrives.
            _armingExitArmed = before == ArmingOfferStep.Armed;
            return;
        }

        if (before == after)
        {
            return;
        }

        _armingStepShown = after;
        ApplyArmingMark(animate: ArmingMotionAllowed && after == ArmingOfferStep.Armed);
        if (ArmingOfferMotion.Pulses(before, after, ArmingMotionAllowed))
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
        RunScale(glyph, ArmingOfferMotion.MarkProgress, ArmingOfferMotion.MarkSettling, from: 0.5);
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
    }

    private static void RunScale(Visual visual, Func<double, double> progress, double settling, double from)
    {
        Compositor compositor = visual.Compositor;
        CompositionEasingFunction linear = compositor.CreateLinearEasingFunction();
        Vector3KeyFrameAnimation animation = compositor.CreateVector3KeyFrameAnimation();
        foreach ((double fraction, double value) in MonitorMotion.KeyFrames(t => from + (1 - from) * progress(t), settling))
        {
            animation.InsertKeyFrame((float)fraction, new Vector3((float)value, (float)value, 1), linear);
        }

        animation.Duration = TimeSpan.FromSeconds(settling);
        visual.StartAnimation("Scale", animation);
    }

    private static void CenterPoint(Visual visual, FrameworkElement element) =>
        visual.CenterPoint = new Vector3((float)(element.ActualWidth / 2), (float)(element.ActualHeight / 2), 0);
}
