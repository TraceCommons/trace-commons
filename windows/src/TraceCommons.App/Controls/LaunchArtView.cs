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
using TraceCommons.Interop.Shell;
using Windows.UI.ViewManagement;

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
        Brush tint = (Brush)Application.Current.Resources[mark.TintKey];
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
/// (<c>UISettings.AnimationsEnabled</c>) it is the rest frame, and turning
/// them off mid-move puts it at rest at once. The move is one composition
/// animation the compositor runs: nothing ticks on this thread, and nothing
/// runs once it has ended or the art is unloaded.
/// </summary>
public sealed class LaunchArtView : UserControl
{
    private readonly Border _folder;
    private readonly UISettings _settings = new();
    private bool _resting;

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
        var ink = (Brush)Application.Current.Resources["TextFillColorPrimaryBrush"];
        var hairline = (Brush)Application.Current.Resources["TcHairlineBrush"];
        Add(canvas, new Rectangle
        {
            Width = LaunchHandoff.TerminalWidth, Height = LaunchHandoff.TerminalHeight,
            RadiusX = LaunchHandoff.ControlRadius, RadiusY = LaunchHandoff.ControlRadius,
            Fill = ink, Opacity = LaunchHandoff.TerminalInk,
        }, left, top);
        for (int i = 0; i < 3; i++)
        {
            Add(canvas, new Ellipse { Width = LaunchHandoff.DotSide, Height = LaunchHandoff.DotSide, Fill = ink, Opacity = LaunchHandoff.DotInk },
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
            Background = (Brush)Application.Current.Resources["TcTileFolderBrush"],
            Child = new FontIcon { Glyph = "", FontSize = 13, Foreground = ink },
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
        // Followed live from Windows 10 2004; before it, read once here.
        if (OperatingSystem.IsWindowsVersionAtLeast(10, 0, 19041))
        {
            _settings.AnimationsEnabledChanged += OnAnimationsEnabledChanged;
        }

        Visual visual = ElementCompositionPreview.GetElementVisual(_folder);
        visual.CenterPoint = new Vector3((float)(LaunchHandoff.FolderTileSide / 2), (float)(LaunchHandoff.FolderTileSide / 2), 0);
        ElementCompositionPreview.SetIsTranslationEnabled(_folder, true);
        bool animate = !LaunchHandoff.StartsAtRest(_settings.AnimationsEnabled);
        Place(visual, LaunchHandoff.Folder(animate ? 0 : LaunchHandoff.Duration, animate));
        if (!animate)
        {
            _resting = true;
            return;
        }

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

    private void OnUnloaded(object sender, RoutedEventArgs e)
    {
        // The dialog's popup raises a late Unloaded while the art is still
        // shown; resting then stopped the move before its first frame.
        if (!LaunchHandoff.RestsOnUnloaded(IsLoaded))
        {
            return;
        }

        if (OperatingSystem.IsWindowsVersionAtLeast(10, 0, 19041))
        {
            _settings.AnimationsEnabledChanged -= OnAnimationsEnabledChanged;
        }

        Rest();
    }

    [System.Runtime.Versioning.SupportedOSPlatform("windows10.0.19041.0")]
    private void OnAnimationsEnabledChanged(UISettings sender, UISettingsAnimationsEnabledChangedEventArgs args)
    {
        if (!sender.AnimationsEnabled)
        {
            DispatcherQueue?.TryEnqueue(Rest);
        }
    }

    /// <summary>Stops the move and draws the rest frame.</summary>
    private void Rest()
    {
        if (_resting)
        {
            return;
        }

        _resting = true;
        Visual visual = ElementCompositionPreview.GetElementVisual(_folder);
        visual.StopAnimation("Translation.X");
        visual.StopAnimation("Scale.X");
        visual.StopAnimation("Scale.Y");
        Place(visual, LaunchHandoff.Folder(LaunchHandoff.Duration, false));
    }

    private static void Place(Visual visual, (double X, double Y, double Scale) folder)
    {
        visual.Properties.InsertVector3("Translation", new Vector3((float)folder.X, (float)folder.Y, 0));
        visual.Scale = new Vector3((float)folder.Scale, (float)folder.Scale, 1);
    }
}
