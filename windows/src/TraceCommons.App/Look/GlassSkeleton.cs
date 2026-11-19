using System;
using Microsoft.UI.Composition;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Hosting;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Look;

/// <summary>
/// The Mac's loading bar (Feedback.swift:74-107, GlassSkeleton): ink at 0.08
/// in the control radius, 12 tall, its opacity breathing 1 to 0.5 and back,
/// 0.8 s each way on the token curve; still under reduce motion and while
/// its window cannot be seen. Hidden from assistive technology, as on the
/// Mac: the words or a spinner beside it say that something is loading.
/// </summary>
public sealed class GlassSkeleton : ContentControl
{
    private readonly GlassClockCore<XamlRoot> _core;
    private readonly AncestorVisibility _shown;

    public GlassSkeleton()
    {
        Style = (Style)Application.Current.Resources["TcGlassSkeletonStyle"];
        IsTabStop = false;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAccessibilityView(this, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        // The same gate as every clock (GlassClockCore): the pulse runs on
        // the compositor, so "subscribed" starts it and "unsubscribed" puts
        // the bar at rest.
        _core = new GlassClockCore<XamlRoot>(WindowClocks.Windows, 0, Start, Stop, _ => { }, () => GlassClock.Now);
        _shown = new AncestorVisibility(this, shown => _core.Shown = shown);
        Loaded += (_, _) =>
        {
            GlassMotionSettings.Changed += Update;
            _core.Window = XamlRoot;
            _core.AnimationsEnabled = GlassMotionSettings.AnimationsEnabled;
            _shown.Attach();
            _core.Loaded = true;
        };
        Unloaded += (_, _) =>
        {
            GlassMotionSettings.Changed -= Update;
            _shown.Detach();
            _core.Loaded = false;
        };
    }

    private void Update()
    {
        _core.AnimationsEnabled = GlassMotionSettings.AnimationsEnabled;
        _core.Window = XamlRoot;
        _shown.Recheck();
    }

    private void Start()
    {
        Visual visual = ElementCompositionPreview.GetElementVisual(this);
        Compositor compositor = visual.Compositor;
        ScalarKeyFrameAnimation pulse = compositor.CreateScalarKeyFrameAnimation();
        CompositionEasingFunction linear = compositor.CreateLinearEasingFunction();
        double period = 2 * GlassSkeletonMotion.Leg;
        foreach ((double fraction, double value) in MonitorMotion.KeyFrames(t => GlassSkeletonMotion.Opacity(t, false), period))
        {
            pulse.InsertKeyFrame((float)fraction, (float)value, linear);
        }

        pulse.Duration = TimeSpan.FromSeconds(period);
        pulse.IterationBehavior = AnimationIterationBehavior.Forever;
        visual.StartAnimation("Opacity", pulse);
    }

    private void Stop() => GlassComposition.Settle(this, "Opacity", 1);
}
