using System;
using System.Collections.Generic;
using System.Runtime.CompilerServices;
using Microsoft.UI.Composition;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Hosting;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Look;

/// <summary>
/// Runs a <see cref="MotionSpec"/> on a composition property: the Mac's
/// curve or spring sampled 60 times a second into key frames, which the
/// compositor plays at the display's rate. When things may not move, the
/// property is set to its end at once. Each run or settle of a property
/// takes a new generation, so a run cut short by the next one never writes
/// its end over the new run when its batch completes (MotionRuns).
/// </summary>
internal static class GlassComposition
{
    private static readonly ConditionalWeakTable<Visual, MotionRuns<string>> Runs = new();

    /// <summary>
    /// Animates <paramref name="property"/> (a scalar: "Opacity",
    /// "RotationAngleInDegrees", "Translation.X", "Scale.X") of the
    /// element's visual from <paramref name="from"/> to <paramref name="to"/>.
    /// </summary>
    public static void Run(UIElement element, string property, float from, float to, MotionSpec spec, Action? completed = null)
    {
        Visual visual = ElementCompositionPreview.GetElementVisual(element);
        EnsureTranslation(visual, property);
        int run = Runs.GetOrCreateValue(visual).Begin(property);
        visual.StopAnimation(property);
        if (!GlassMotionSettings.MayMoveFor(element))
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
            if (!Runs.GetOrCreateValue(visual).IsCurrent(property, run))
            {
                return;
            }

            Set(visual, property, to);
            completed?.Invoke();
        };
    }

    /// <summary>Stops <paramref name="property"/> and puts it at <paramref name="value"/>.</summary>
    public static void Settle(UIElement element, string property, float value)
    {
        Visual visual = ElementCompositionPreview.GetElementVisual(element);
        EnsureTranslation(visual, property);
        Runs.GetOrCreateValue(visual).Begin(property);
        visual.StopAnimation(property);
        Set(visual, property, value);
    }

    /// <summary>
    /// Translation is a property set entry that exists only once written;
    /// stopping or animating it before then throws "property not found".
    /// </summary>
    private static void EnsureTranslation(Visual visual, string property)
    {
        if (property.StartsWith("Translation", StringComparison.Ordinal)
            && visual.Properties.TryGetVector3("Translation", out _) != CompositionGetValueStatus.Succeeded)
        {
            visual.Properties.InsertVector3("Translation", System.Numerics.Vector3.Zero);
        }
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
