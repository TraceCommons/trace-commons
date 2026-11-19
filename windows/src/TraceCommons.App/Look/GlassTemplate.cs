using System;
using System.Linq;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Animation;
using TraceCommons.Interop.Shell;
using Windows.Foundation;

namespace TraceCommons.App.Look;

/// <summary>
/// Builds a re-templated control's motion from <see cref="GlassTemplateMotion"/>
/// (spec W5-D3 (a)): each visual state's values as zero-length animations,
/// each transition as key frames at the plan's time on the plan's spline,
/// and the switch's track, knob and inset. The templates in
/// GlassControls.xaml carry only the states' names, so every number the
/// control draws is the plan's, which the Interop tests pin to the Mac's.
/// Set <c>look:GlassTemplate.Kind</c> on the control (its style does); the
/// <see cref="GlassVisualStateManager"/> builds the plan into the template on
/// its first state change, once per template instance.
/// </summary>
public static class GlassTemplate
{
    public static readonly DependencyProperty KindProperty = DependencyProperty.RegisterAttached(
        "Kind", typeof(string), typeof(GlassTemplate), new PropertyMetadata(null));

    private static readonly DependencyProperty BuiltProperty = DependencyProperty.RegisterAttached(
        "Built", typeof(bool), typeof(GlassTemplate), new PropertyMetadata(false));

    public static string? GetKind(DependencyObject element) => (string?)element.GetValue(KindProperty);

    public static void SetKind(DependencyObject element, string? value) => element.SetValue(KindProperty, value);

    /// <summary>Builds <paramref name="control"/>'s plan into its template root; once.</summary>
    internal static void Install(Control control, FrameworkElement root)
    {
        if ((bool)root.GetValue(BuiltProperty))
        {
            return;
        }

        root.SetValue(BuiltProperty, true);
        if (!Enum.TryParse(GetKind(control) ?? GetKind(root), out GlassTemplateKind kind))
        {
            return;
        }

        foreach (PartFrame frame in GlassTemplateMotion.Frames(kind))
        {
            if (root.FindName(frame.Part) is FrameworkElement part)
            {
                if (!double.IsNaN(frame.Width))
                {
                    part.Width = frame.Width;
                    part.Height = frame.Height;
                }

                part.Margin = new Thickness(frame.Left, 0, frame.Right, 0);
            }
        }

        if (control is ToggleSwitch toggle)
        {
            Arrange(toggle, root, kind);
        }

        var groups = VisualStateManager.GetVisualStateGroups(root);
        foreach (GroupPlan plan in GlassTemplateMotion.Groups(kind))
        {
            VisualStateGroup? group = groups.FirstOrDefault(g => g.Name == plan.Group);
            if (group is null)
            {
                continue;
            }

            foreach (StatePlan statePlan in plan.States.Where(p => p.Values.Count > 0))
            {
                VisualState? state = group.States.FirstOrDefault(s => s.Name == statePlan.State);
                if (state is not null)
                {
                    state.Storyboard = Board(root, statePlan.Values, 0, default);
                }
            }

            group.Transitions.Clear();
            foreach (TransitionPlan transition in plan.Transitions)
            {
                var visual = new VisualTransition { Storyboard = Board(root, transition.Values, transition.Seconds, transition.Spline) };
                if (transition.From is not null)
                {
                    visual.From = transition.From;
                }

                if (transition.To is not null)
                {
                    visual.To = transition.To;
                }

                group.Transitions.Add(visual);
            }
        }
    }

    /// <summary>
    /// The label first and the switch after it, or the reverse for the first
    /// run's switch, with the Mac's gap only when there is a label.
    /// </summary>
    private static void Arrange(ToggleSwitch toggle, FrameworkElement root, GlassTemplateKind kind)
    {
        bool leads = GlassTemplateMotion.LabelLeads(kind);
        if (root.FindName("LabelArea") is FrameworkElement label)
        {
            Grid.SetColumn(label, leads ? 0 : 2);
        }

        if (root.FindName("SwitchArea") is FrameworkElement area)
        {
            Grid.SetColumn(area, leads ? 2 : 0);
        }

        void Gap()
        {
            if (root.FindName("LabelGap") is ColumnDefinition gap)
            {
                bool labelled = toggle.Header is not null
                    || toggle.OnContent is not (null or "")
                    || toggle.OffContent is not (null or "");
                gap.Width = new GridLength(labelled ? GlassTemplateMotion.LabelGap(kind) : 0);
            }
        }

        Gap();
        toggle.RegisterPropertyChangedCallback(ToggleSwitch.HeaderProperty, (_, _) => Gap());
        toggle.RegisterPropertyChangedCallback(ToggleSwitch.OnContentProperty, (_, _) => Gap());
        toggle.RegisterPropertyChangedCallback(ToggleSwitch.OffContentProperty, (_, _) => Gap());
    }

    private static Storyboard Board(FrameworkElement root, System.Collections.Generic.IReadOnlyList<PartValue> values, double seconds, SplinePoints spline)
    {
        var board = new Storyboard();
        foreach (PartValue value in values)
        {
            if (root.FindName(value.Part) is not DependencyObject target)
            {
                continue;
            }

            Timeline animation;
            if (seconds <= 0)
            {
                animation = new DoubleAnimation { To = value.Value, Duration = new Duration(TimeSpan.Zero) };
            }
            else
            {
                var frames = new DoubleAnimationUsingKeyFrames();
                frames.KeyFrames.Add(new SplineDoubleKeyFrame
                {
                    KeyTime = KeyTime.FromTimeSpan(TimeSpan.FromSeconds(seconds)),
                    Value = value.Value,
                    KeySpline = new KeySpline
                    {
                        ControlPoint1 = new Point(spline.X1, spline.Y1),
                        ControlPoint2 = new Point(spline.X2, spline.Y2),
                    },
                });
                animation = frames;
            }

            Storyboard.SetTarget(animation, target);
            Storyboard.SetTargetProperty(animation, value.Property);
            board.Children.Add(animation);
        }

        return board;
    }
}
