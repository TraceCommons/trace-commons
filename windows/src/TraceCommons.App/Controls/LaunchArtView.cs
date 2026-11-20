using System;
using System.Collections.Generic;
using System.Numerics;
using Microsoft.UI.Composition;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Hosting;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using TraceCommons.App.Look;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Controls;

/// <summary>Path steps as a XAML geometry, filled non-zero as SVG fills a logo (GlassToolLogo.swift:34).</summary>
internal static class StepGeometry
{
    public static Geometry Build(IReadOnlyList<PathStep> steps)
    {
        var geometry = new PathGeometry { FillRule = FillRule.Nonzero };
        PathFigure? figure = null;
        foreach (PathStep s in steps)
        {
            Windows.Foundation.Point P(double x, double y) => new(x, y);
            switch (s.Kind)
            {
                case PathStepKind.Move:
                    figure = new PathFigure { StartPoint = P(s.X, s.Y), IsClosed = false, IsFilled = true };
                    geometry.Figures.Add(figure);
                    break;
                case PathStepKind.Line:
                    figure?.Segments.Add(new LineSegment { Point = P(s.X, s.Y) });
                    break;
                case PathStepKind.Cubic:
                    figure?.Segments.Add(new BezierSegment { Point1 = P(s.X1, s.Y1), Point2 = P(s.X2, s.Y2), Point3 = P(s.X, s.Y) });
                    break;
                case PathStepKind.Close:
                    if (figure is not null)
                    {
                        figure.IsClosed = true;
                    }

                    break;
            }
        }

        return geometry;
    }

    /// <summary>A tool's mark at <paramref name="side"/>: its logo in its tint, or its tinted initials.</summary>
    public static FrameworkElement Mark(string tool, double side, double initialsSize)
    {
        LaunchToolMark mark = LaunchToolMark.For(tool);
        Brush tint = ThemeBrushes.Get(mark.TintKey);
        FrameworkElement element = mark.Logo is ToolLogoId id
            ? new Path { Data = Build(ToolLogos.Fitted(id, new LayoutRect(0, 0, side, side))), Fill = tint, Width = side, Height = side }
            : new TextBlock { Text = mark.Initials, Foreground = tint, FontSize = initialsSize, FontWeight = Microsoft.UI.Text.FontWeights.Bold, VerticalAlignment = VerticalAlignment.Center };
        // Decorative: the button or dialog names the tool.
        AutomationProperties.SetAccessibilityView(element, AccessibilityView.Raw);
        return element;
    }
}

/// <summary>
/// A folder slides into a terminal window that shows the tool's logo, once,
/// over 1.2 s on the token curve, then rests (<c>ManagedFolderHandoff</c>,
/// ManagedFolderExplainer.swift:173-252; numbers in <see cref="LaunchHandoff"/>).
/// Decorative, so hidden from assistive tech. With animations off
/// (<see cref="GlassMotionSettings"/>, W5's one reading of the system
/// switch, which also hears the window's WM_SETTINGCHANGE) it is the rest
/// frame, and turning them off mid-move puts it at rest at once. The move is one composition
/// animation the compositor runs: nothing ticks on this thread, and nothing
/// runs once it has ended or the art is unloaded.
/// </summary>
public sealed class LaunchArtView : UserControl
{
    private readonly Border _folder;
    private readonly LaunchArtLife _life = new();
    private bool _subscribed;

    public LaunchArtView(string tool)
    {
        Width = LaunchHandoff.Width;
        Height = LaunchHandoff.Height;
        IsTabStop = false;
        AutomationProperties.SetAccessibilityView(this, AccessibilityView.Raw);
        var canvas = new Canvas { Width = LaunchHandoff.Width, Height = LaunchHandoff.Height };

        // The terminal, centred: ink ground, hairline rim, a title bar of
        // three dots over a hairline, the tool's logo.
        double left = (LaunchHandoff.Width - LaunchHandoff.TerminalWidth) / 2 + LaunchHandoff.TerminalX;
        double top = (LaunchHandoff.Height - LaunchHandoff.TerminalHeight) / 2;
        var ink = ThemeBrushes.Get("TextFillColorPrimaryBrush");
        bool highContrast = new Windows.UI.ViewManagement.AccessibilitySettings().HighContrast;
        (double groundInk, double dotInk) = LaunchHandoff.TerminalInks(highContrast);
        var hairline = highContrast ? ink : ThemeBrushes.Get("TcHairlineBrush");
        Add(canvas, new Rectangle
        {
            Width = LaunchHandoff.TerminalWidth, Height = LaunchHandoff.TerminalHeight,
            RadiusX = LaunchHandoff.ControlRadius, RadiusY = LaunchHandoff.ControlRadius,
            Fill = ink, Opacity = groundInk,
        }, left, top);
        for (int i = 0; i < 3; i++)
        {
            Add(canvas, new Ellipse { Width = LaunchHandoff.DotSide, Height = LaunchHandoff.DotSide, Fill = ink, Opacity = dotInk },
                left + LaunchHandoff.BarPadding + i * (LaunchHandoff.DotSide + LaunchHandoff.DotSpacing),
                top + (LaunchHandoff.BarHeight - LaunchHandoff.DotSide) / 2);
        }

        Add(canvas, new Rectangle { Width = LaunchHandoff.TerminalWidth, Height = LaunchHandoff.Hairline, Fill = hairline },
            left, top + LaunchHandoff.BarHeight);
        Add(canvas, new Rectangle
        {
            Width = LaunchHandoff.TerminalWidth, Height = LaunchHandoff.TerminalHeight,
            RadiusX = LaunchHandoff.ControlRadius, RadiusY = LaunchHandoff.ControlRadius,
            Stroke = hairline, StrokeThickness = LaunchHandoff.Hairline,
        }, left, top);
        if (LaunchToolMark.For(tool).Logo is not null)
        {
            (double lx, double ly) = LaunchHandoff.LogoOffset;
            Add(canvas, StepGeometry.Mark(tool, LaunchHandoff.LogoSide, 11),
                left + LaunchHandoff.TerminalWidth / 2 + lx - LaunchHandoff.LogoSide / 2, top + ly);
        }

        // The folder: the large folder tile, its centre offset from the art's.
        _folder = new Border
        {
            Width = LaunchHandoff.FolderTileSide, Height = LaunchHandoff.FolderTileSide,
            CornerRadius = new CornerRadius(LaunchHandoff.ControlRadius),
            Background = ThemeBrushes.Get("TcTileFolderBrush"),
            Child = new FontIcon { Glyph = LaunchHandoff.FolderGlyph, FontSize = 13, Foreground = ink },
        };
        Add(canvas, _folder, (LaunchHandoff.Width - LaunchHandoff.FolderTileSide) / 2,
            (LaunchHandoff.Height - LaunchHandoff.FolderTileSide) / 2);
        Content = canvas;

        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
    }

    private static void Add(Canvas canvas, FrameworkElement element, double x, double y)
    {
        Canvas.SetLeft(element, x);
        Canvas.SetTop(element, y);
        canvas.Children.Add(element);
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        Visual visual = ElementCompositionPreview.GetElementVisual(_folder);
        visual.CenterPoint = new Vector3((float)(LaunchHandoff.FolderTileSide / 2), (float)(LaunchHandoff.FolderTileSide / 2), 0);
        ElementCompositionPreview.SetIsTranslationEnabled(_folder, true);
        LaunchArtStep step = _life.Loaded(GlassMotionSettings.AnimationsEnabled);
        Watch(_life.Watching);
        if (step == LaunchArtStep.Animate)
        {
            Animate(visual);
        }
        else
        {
            Act(step);
        }
    }

    private void OnUnloaded(object sender, RoutedEventArgs e) => Act(_life.Unloaded(IsLoaded));

    /// <summary>
    /// The dialog this art is shown in has closed: stop watching and rest,
    /// whether or not a final Unloaded arrives (review of #2207: the late
    /// Unloaded returns early, and a dialog that raised no second one left
    /// the art in the watch list with its event handler attached).
    /// </summary>
    public void Release() => Act(_life.Closed());

    /// <summary>The system's animation switch or a window's visibility
    /// changed (W5's <see cref="GlassMotionSettings"/>, which also hears the
    /// window's WM_SETTINGCHANGE): rest if animations are now off.</summary>
    private void OnMotionChanged() => Act(_life.SettingChanged(GlassMotionSettings.AnimationsEnabled));

    /// <summary>Follows the motion setting while the life says the art is watched.</summary>
    private void Watch(bool watching)
    {
        if (watching == _subscribed)
        {
            return;
        }

        _subscribed = watching;
        if (watching)
        {
            GlassMotionSettings.Changed += OnMotionChanged;
        }
        else
        {
            GlassMotionSettings.Changed -= OnMotionChanged;
        }
    }

    private void Act(LaunchArtStep step)
    {
        Watch(_life.Watching);
        if (step != LaunchArtStep.Rest)
        {
            return;
        }

        Visual visual = ElementCompositionPreview.GetElementVisual(_folder);
        visual.StopAnimation("Translation.X");
        visual.StopAnimation("Scale.X");
        visual.StopAnimation("Scale.Y");
        Place(visual, LaunchHandoff.Folder(LaunchHandoff.Duration, false));
    }

    private static void Animate(Visual visual)
    {
        Place(visual, LaunchHandoff.Folder(0, true));
        Compositor compositor = visual.Compositor;
        CompositionEasingFunction linear = compositor.CreateLinearEasingFunction();
        ScalarKeyFrameAnimation x = compositor.CreateScalarKeyFrameAnimation();
        ScalarKeyFrameAnimation scale = compositor.CreateScalarKeyFrameAnimation();
        foreach ((double fraction, double value) in MonitorMotion.KeyFrames(t => LaunchHandoff.Folder(t, true).X, LaunchHandoff.Duration))
        {
            x.InsertKeyFrame((float)fraction, (float)value, linear);
        }

        foreach ((double fraction, double value) in MonitorMotion.KeyFrames(t => LaunchHandoff.Folder(t, true).Scale, LaunchHandoff.Duration))
        {
            scale.InsertKeyFrame((float)fraction, (float)value, linear);
        }

        x.Duration = scale.Duration = TimeSpan.FromSeconds(LaunchHandoff.Duration);
        visual.StartAnimation("Translation.X", x);
        visual.StartAnimation("Scale.X", scale);
        visual.StartAnimation("Scale.Y", scale);
    }

    private static void Place(Visual visual, (double X, double Y, double Scale) folder)
    {
        visual.Properties.InsertVector3("Translation", new Vector3((float)folder.X, (float)folder.Y, 0));
        visual.Scale = new Vector3((float)folder.Scale, (float)folder.Scale, 1);
    }
}
