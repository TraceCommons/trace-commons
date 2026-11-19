using System;
using Microsoft.UI.Composition;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Hosting;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Look;

/// <summary>
/// Runs a <see cref="MotionSpec"/> on a composition property: the Mac's
/// curve or spring sampled 60 times a second into key frames, which the
/// compositor plays at the display's rate. When things may not move, the
/// property is set to its end at once.
/// </summary>
internal static class GlassComposition
{
    /// <summary>
    /// Animates <paramref name="property"/> (a scalar: "Opacity",
    /// "RotationAngleInDegrees", "Translation.X", "Scale.X") of the
    /// element's visual from <paramref name="from"/> to <paramref name="to"/>.
    /// </summary>
    public static void Run(UIElement element, string property, float from, float to, MotionSpec spec, Action? completed = null)
    {
        Visual visual = ElementCompositionPreview.GetElementVisual(element);
        visual.StopAnimation(property);
        if (!GlassMotionSettings.MayMove)
        {
            Set(visual, property, to);
            completed?.Invoke();
            return;
        }

        Compositor compositor = visual.Compositor;
        ScalarKeyFrameAnimation animation = compositor.CreateScalarKeyFrameAnimation();
        CompositionEasingFunction linear = compositor.CreateLinearEasingFunction();
        foreach ((double fraction, double value) in spec.KeyFrames())
        {
            animation.InsertKeyFrame((float)fraction, (float)(from + (to - from) * value), linear);
        }

        animation.Duration = TimeSpan.FromSeconds(spec.Duration);
        CompositionScopedBatch batch = compositor.CreateScopedBatch(CompositionBatchTypes.Animation);
        visual.StartAnimation(property, animation);
        batch.End();
        batch.Completed += (_, _) =>
        {
            Set(visual, property, to);
            completed?.Invoke();
        };
    }

    /// <summary>Stops <paramref name="property"/> and puts it at <paramref name="value"/>.</summary>
    public static void Settle(UIElement element, string property, float value)
    {
        Visual visual = ElementCompositionPreview.GetElementVisual(element);
        visual.StopAnimation(property);
        Set(visual, property, value);
    }

    private static void Set(Visual visual, string property, float value)
    {
        switch (property)
        {
            case "Opacity":
                visual.Opacity = value;
                break;
            case "RotationAngleInDegrees":
                visual.RotationAngleInDegrees = value;
                break;
            case "Scale.X":
                visual.Scale = new System.Numerics.Vector3(value, visual.Scale.Y, visual.Scale.Z);
                break;
            case "Translation.X":
                visual.Properties.InsertVector3("Translation", new System.Numerics.Vector3(value, 0, 0));
                break;
            default:
                throw new ArgumentException($"no setter for {property}", nameof(property));
        }
    }
}
