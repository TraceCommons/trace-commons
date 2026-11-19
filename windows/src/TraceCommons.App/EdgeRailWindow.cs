using System;
using System.Collections.Generic;
using System.Diagnostics;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using TraceCommons.Interop;
using TraceCommons.Interop.Generated;
using TraceCommons.Interop.Shell;
using Windows.UI;
using Windows.UI.ViewManagement;

namespace TraceCommons.App;

/// <summary>
/// The edge rail (W13; the Mac's <c>EdgeRailController</c> and
/// <c>EdgeRailView</c>, Views/EdgeRail/): off until turned on in Settings.
/// Closed, a 4 by 240 zone with its 3 by 72 handle on the primary screen's
/// right edge; with the pointer on it, a 484 by 680 rail slides in from the
/// trailing edge: a peek of the hovered tab beside a column of icons, the
/// tabs' and then the app's N. It closes 300 ms after the pointer leaves.
/// </summary>
/// <remarks>
/// Hover never takes the keyboard; a click inside does, and it is given back
/// as the close starts (<see cref="EdgeRailHover"/>). The window is a tool
/// window, out of Alt-Tab and the taskbar, never matched by its title
/// (<see cref="ToolWindows"/>).
/// </remarks>
internal sealed class EdgeRailWindow : Window
{
    private readonly IntPtr _hwnd;
    private readonly EdgeRailHover _hover = new();
    private readonly DispatcherQueueTimer _closeTimer;
    private readonly UISettings _uiSettings = new();
    private readonly Grid _root = new();
    private readonly Grid _closed = new();
    private readonly Grid _open = new();
    private readonly TranslateTransform _slide = new();
    private readonly Stopwatch _slideClock = new();
    private readonly StackPanel _peek = new() { Spacing = HomeTrayInks.Measures.CardGap };
    private readonly Func<IReadOnlyList<HomeTile>> _privateAITiles;
    private readonly Func<IReadOnlyList<HomeTile>> _tracesTiles;
    private readonly Action<MonitorTab?> _openApp;
    private bool _sliding;
    private bool _slidingOpen;
    private IntPtr _previousFocus;
    private MonitorTab _peekTab = MonitorTab.Inference;

    public EdgeRailWindow(Func<IReadOnlyList<HomeTile>> privateAITiles, Func<IReadOnlyList<HomeTile>> tracesTiles, Action<MonitorTab?> openApp)
    {
        _privateAITiles = privateAITiles;
        _tracesTiles = tracesTiles;
        _openApp = openApp;
        Title = MonitorShellCopy.Section("edge_rail", "rail_label");
        _hwnd = ToolWindows.Prepare(this);

        _closeTimer = DispatcherQueue.GetForCurrentThread().CreateTimer();
        _closeTimer.Interval = TimeSpan.FromMilliseconds(50);
        _closeTimer.Tick += (_, _) => Step(_hover.Tick(DateTimeOffset.UtcNow));

        BuildClosed();
        BuildOpen();
        _root.Background = (Brush)Application.Current.Resources["TcWindowBackgroundBrush"];
        _root.Children.Add(_closed);
        _root.Children.Add(_open);
        _open.Visibility = Visibility.Collapsed;
        _root.PointerEntered += (_, _) => Step(_hover.PointerEntered());
        _root.PointerExited += (_, _) =>
        {
            _hover.PointerExited(DateTimeOffset.UtcNow);
            _closeTimer.Start();
        };
        _root.PointerPressed += (_, _) =>
        {
            if (_hover.Open && !_hover.HoldsKeyboard)
            {
                _previousFocus = ToolWindows.Foreground();
                _hover.Clicked();
                ToolWindows.TakeKeyboard(_hwnd);
            }
        };
        _root.KeyDown += (_, e) =>
        {
            if (e.Key == Windows.System.VirtualKey.Escape)
            {
                Step(_hover.CloseNow());
                e.Handled = true;
            }
        };
        AutomationProperties.SetName(_root, MonitorShellCopy.Section("edge_rail", "rail_label"));
        Content = _root;
        Place(open: false);
    }

    /// <summary>Puts the window on the primary screen's right edge at the size the state needs (EdgeRailController.swift:102-106).</summary>
    private void Place(bool open)
    {
        (double scale, (int X, int Y, int Width, int Height) work) = ToolWindows.Primary();
        ToolWindows.PlaceAndShow(this, EdgeRailGeometry.FramePixels(open, work, scale));
    }

    private void Step(EdgeRailStep step)
    {
        switch (step)
        {
            case EdgeRailStep.GrowThenOpen:
                // The window grows before the rail draws open into it
                // (EdgeRailController.swift:142-147).
                Place(open: true);
                ShowPeek(_peekTab);
                _closed.Visibility = Visibility.Collapsed;
                _open.Visibility = Visibility.Visible;
                StartSlide(opening: true);
                break;
            case EdgeRailStep.CloseThenShrink:
                _closeTimer.Stop();

                // The keyboard goes back as the close starts (checklist M2).
                ToolWindows.ReleaseKeyboard(_hwnd);
                ToolWindows.ReturnFocus(_previousFocus);
                _previousFocus = IntPtr.Zero;
                StartSlide(opening: false);
                break;
        }

        if (!_hover.Open && _hover.CloseAt is null)
        {
            _closeTimer.Stop();
        }
    }

    /// <summary>The slide: from the trailing edge with a fade over 0.25 s on the token curve; a fade alone under reduce motion.</summary>
    private void StartSlide(bool opening)
    {
        _slidingOpen = opening;
        _slideClock.Restart();
        if (!_sliding)
        {
            _sliding = true;
            CompositionTarget.Rendering += OnSlideFrame;
        }

        OnSlideFrame(null, EventArgs.Empty);
    }

    private void OnSlideFrame(object? sender, object e)
    {
        bool reduce = !_uiSettings.AnimationsEnabled;
        (double offset, double opacity, bool done) = reduce && !_slidingOpen
            ? (0, 0, true)
            : EdgeRailHover.Slide(_slidingOpen, reduce ? EdgeRailHover.SlideSeconds : _slideClock.Elapsed.TotalSeconds, reduce);
        _slide.X = offset * EdgeRailGeometry.OpenWidth;
        _open.Opacity = opacity;
        if (!done)
        {
            return;
        }

        _sliding = false;
        CompositionTarget.Rendering -= OnSlideFrame;
        if (!_slidingOpen)
        {
            // Shrinks only once the rail has gone (EdgeRailController.swift:148-155).
            _open.Visibility = Visibility.Collapsed;
            _closed.Visibility = Visibility.Visible;
            Place(open: false);
        }
    }

    private void BuildClosed()
    {
        Color handle = Argb(ActualDark ? HomeTrayInks.Dark.EdgeRailHandle : HomeTrayInks.Light.EdgeRailHandle);
        _closed.Children.Add(new Rectangle
        {
            Width = EdgeRailGeometry.HandleWidth,
            Height = EdgeRailGeometry.HandleHeight,
            RadiusX = EdgeRailGeometry.HandleWidth / 2 + 0.5,
            RadiusY = EdgeRailGeometry.HandleWidth / 2 + 0.5,
            Fill = new SolidColorBrush(handle),
            HorizontalAlignment = HorizontalAlignment.Right,
            VerticalAlignment = VerticalAlignment.Center,
        });
        AutomationProperties.SetName(_closed, MonitorShellCopy.Section("edge_rail", "handle_label"));
    }

    private void BuildOpen()
    {
        _open.RenderTransform = _slide;
        _open.Padding = new Thickness(EdgeRailGeometry.ShadowRoom, 0, EdgeRailGeometry.Inset, 0);
        _open.ColumnSpacing = EdgeRailGeometry.Gap;
        _open.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(EdgeRailGeometry.PeekWidth) });
        _open.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(EdgeRailGeometry.RailWidth) });

        var peekCard = new Border
        {
            Padding = new Thickness(HomeTrayInks.Measures.PanePadding),
            CornerRadius = new CornerRadius(HomeTrayInks.Measures.CardRadius),
            Background = (Brush)Application.Current.Resources["TcSurfaceCardBrush"],
            Child = _peek,
            VerticalAlignment = VerticalAlignment.Center,
        };
        _open.Children.Add(peekCard);

        var icons = new StackPanel
        {
            Spacing = HomeTrayInks.Measures.S1,
            Padding = new Thickness(0, HomeTrayInks.Measures.S3, 0, HomeTrayInks.Measures.S3),
            VerticalAlignment = VerticalAlignment.Center,
            HorizontalAlignment = HorizontalAlignment.Center,
        };
        icons.Children.Add(Tile(MonitorTab.Inference, new FontIcon { Glyph = "", FontSize = 17 }));
        icons.Children.Add(Tile(MonitorTab.Traces, TraceNodes()));
        icons.Children.Add(new Rectangle
        {
            Width = EdgeRailGeometry.RuleWidth,
            Height = 1,
            Fill = (Brush)Application.Current.Resources["TcHairlineBrush"],
            Margin = new Thickness(0, HomeTrayInks.Measures.S2, 0, HomeTrayInks.Measures.S2),
        });
        var app = new Button { Width = EdgeRailGeometry.Tile, Height = EdgeRailGeometry.Tile, Content = Mark(20) };
        string openApp = MonitorShellCopy.Section("edge_rail", "open_app");
        AutomationProperties.SetName(app, openApp);
        ToolTipService.SetToolTip(app, openApp);
        app.Click += (_, _) =>
        {
            Step(_hover.CloseNow());
            _openApp(null);
        };
        icons.Children.Add(app);
        Grid.SetColumn(icons, 1);
        _open.Children.Add(icons);
    }

    /// <summary>One icon: hovering or clicking shows its peek (EdgeRailView.swift:136-160).</summary>
    private Button Tile(MonitorTab tab, UIElement glyph)
    {
        var button = new Button { Width = EdgeRailGeometry.Tile, Height = EdgeRailGeometry.Tile, Content = glyph };
        string name = MonitorWords.TabTitle(tab);
        AutomationProperties.SetName(button, name);
        ToolTipService.SetToolTip(button, name);
        button.PointerEntered += (_, _) => ShowPeek(tab);
        button.Click += (_, _) => ShowPeek(tab);
        return button;
    }

    /// <summary>
    /// The hovered icon's peek: its tab's name, the Settings button, and its
    /// figures as Home's tiles draw them. The Mac's peek also holds the
    /// folder list, the shared and kept graph, the tool buttons and a small
    /// Private AI map (EdgeRailView.swift:236-280); those surfaces are other
    /// slices' and are listed as not ported in W13.
    /// </summary>
    private void ShowPeek(MonitorTab tab)
    {
        _peekTab = tab;
        _peek.Children.Clear();
        var header = new Grid();
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.Children.Add(new TextBlock { Text = MonitorWords.TabTitle(tab), Style = (Style)Application.Current.Resources["TcTitleCardTextStyle"] });
        var settings = new Button { Content = new FontIcon { Glyph = "", FontSize = 14 } };
        AutomationProperties.SetName(settings, MonitorWords.SettingsTitle);
        ToolTipService.SetToolTip(settings, MonitorWords.SettingsTitle);
        settings.Click += (_, _) =>
        {
            Step(_hover.CloseNow());
            _openApp(null);
        };
        Grid.SetColumn(settings, 1);
        header.Children.Add(settings);
        _peek.Children.Add(header);
        foreach (HomeTile tile in tab == MonitorTab.Inference ? _privateAITiles() : _tracesTiles())
        {
            var stack = new StackPanel { Spacing = HomeTrayInks.Measures.S2 };
            stack.Children.Add(new TextBlock { Text = tile.Label, Style = (Style)Application.Current.Resources["TcEyebrowTextStyle"] });
            stack.Children.Add(new TextBlock { Text = tile.Value, Style = (Style)Application.Current.Resources["TcMetricValueTextStyle"] });
            _peek.Children.Add(stack);
        }
    }

    private bool ActualDark => Application.Current.RequestedTheme == ApplicationTheme.Dark;

    /// <summary>Four nodes in a T (<c>GlassTraceNodesGlyph</c>, Logos/GlassTraceNodesGlyph.swift), 20 points.</summary>
    private static UIElement TraceNodes()
    {
        const double side = 20;
        var canvas = new Canvas { Width = side, Height = side };
        (double X, double Y)[] nodes = { (0.16, 0.3), (0.5, 0.3), (0.84, 0.3), (0.5, 0.84) };
        Brush ink = (Brush)Application.Current.Resources["TcInkPrimaryBrush"];
        foreach ((int from, int to) in new[] { (0, 2), (1, 3) })
        {
            canvas.Children.Add(new Line
            {
                X1 = nodes[from].X * side,
                Y1 = nodes[from].Y * side,
                X2 = nodes[to].X * side,
                Y2 = nodes[to].Y * side,
                Stroke = ink,
                StrokeThickness = 0.08 * side,
                StrokeStartLineCap = PenLineCap.Round,
                StrokeEndLineCap = PenLineCap.Round,
            });
        }

        double r = 0.12 * side;
        foreach ((double x, double y) in nodes)
        {
            var dot = new Ellipse { Width = r * 2, Height = r * 2, Fill = ink };
            Canvas.SetLeft(dot, x * side - r);
            Canvas.SetTop(dot, y * side - r);
            canvas.Children.Add(dot);
        }

        AutomationProperties.SetAccessibilityView(canvas, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        return canvas;
    }

    /// <summary>The NEAR AI N in the brand blue on the app's tile (EdgeRailView.swift:111-114).</summary>
    private static UIElement Mark(double side)
    {
        var path = new Microsoft.UI.Xaml.Shapes.Path
        {
            Data = NearMarkGeometry.Build(),
            Stretch = Stretch.Uniform,
            Fill = new SolidColorBrush(Argb(HomeTrayInks.Dark.HeroTints[1])),
            Width = side,
            Height = side,
        };
        AutomationProperties.SetAccessibilityView(path, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        return path;
    }

    private static Color Argb(uint argb) =>
        Color.FromArgb((byte)(argb >> 24), (byte)(argb >> 16), (byte)(argb >> 8), (byte)argb);
}
