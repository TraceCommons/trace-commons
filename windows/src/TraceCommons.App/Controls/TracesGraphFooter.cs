using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.Linq;
using Microsoft.Graphics.Canvas.Text;
using Microsoft.Graphics.Canvas.UI.Xaml;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using TraceCommons.Interop.Map;
using Windows.Foundation;
using Windows.UI.ViewManagement;
using Color = Windows.UI.Color;
using TraceCommons.App.Look;

namespace TraceCommons.App.Controls;

/// <summary>
/// The graph under the Traces tab (<c>TracesGraphFooter</c>,
/// MonitorShell.swift:188-346; MonitorWindowView.swift:400-418): the
/// legend, the shared-over-kept dot matrix and six pills round the range.
/// Every decision and every number is <see cref="TracesGraphState"/>'s and
/// <see cref="BarGraph"/>'s; this draws them. Every word is the core's
/// (<see cref="TracesGraphWords"/>).
/// </summary>
internal sealed class TracesGraphFooter : UserControl
{
    /// <summary>The graph row: columns 96 tall, a 4 pt gap, labels on 16 pt lines (Indicators.swift:419-438).</summary>
    private const double LabelGap = 4;
    private const double LabelLine = 16;

    private readonly TracesGraphState _state = new();
    private readonly TracesGraphWords _words = TracesGraphWords.Core;
    private readonly UISettings _uiSettings = new();
    private readonly AccessibilitySettings _accessibility = new();

    private readonly CanvasControl _canvas = new() { Height = BarGraph.ColumnHeight + LabelGap + LabelLine };
    private readonly Grid _columns = new() { Height = BarGraph.ColumnHeight + LabelGap + LabelLine };
    private readonly TextBlock _sharedValue = Figure();
    private readonly TextBlock _keptValue = Figure();
    private readonly TextBlock _range = new() { TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center };
    private readonly Button _previous;
    private readonly Button _focus;
    private readonly FontIcon _focusGlyph = new() { Glyph = "", FontSize = 11 };
    private readonly Button _zoomOut;
    private readonly Button _zoomIn;
    private readonly Button _next;
    private readonly Button _jump;

    private IReadOnlyList<MapHistoryRow>? _history;
    private bool _complete;
    private IReadOnlyList<MapEntry> _sessions = Array.Empty<MapEntry>();
    private string? _tool;
    private bool _focused;
    private GraphFace? _face;

    // The bars drawn before the last step or zoom, fading out under the new ones.
    private GraphFace? _leaving;
    private BezierCurve? _change;
    private GraphSlide _slide;
    private readonly Stopwatch _changeClock = new();
    private bool _ticking;

    /// <summary>The binoculars: focus the map on the selection's tool, or back to the whole map.</summary>
    public event Action? FocusRequested;

    public TracesGraphFooter()
    {
        IsTabStop = false;

        var stack = new StackPanel { Spacing = 8 };

        // The legend (MonitorShell.swift:224-228): shared, then kept.
        var legend = new Grid { ColumnSpacing = 6 };
        legend.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        legend.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        legend.Children.Add(LegendCell(_words.Shared, "dataShared", _sharedValue));
        UIElement kept = LegendCell(_words.Kept, "dataKept", _keptValue);
        Grid.SetColumn((FrameworkElement)kept, 1);
        legend.Children.Add(kept);
        stack.Children.Add(legend);

        // The bars, and over each column a transparent element that takes
        // the pointer and carries the bar's words (:229-233, Indicators.swift:439-442).
        var graph = new Grid();
        _canvas.Draw += OnDraw;
        _canvas.IsHitTestVisible = false;
        AutomationProperties.SetAccessibilityView(_canvas, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        graph.Children.Add(_canvas);
        graph.Children.Add(_columns);
        graph.SizeChanged += (_, _) => LayoutColumns();
        stack.Children.Add(graph);

        // The pills (:234-269).
        var pills = new Grid { ColumnSpacing = 6 };
        foreach (GridLength width in new[] { GridLength.Auto, GridLength.Auto, GridLength.Auto, new GridLength(1, GridUnitType.Star), GridLength.Auto, GridLength.Auto, GridLength.Auto })
        {
            pills.ColumnDefinitions.Add(new ColumnDefinition { Width = width });
        }

        _previous = Pill(_words.Previous, "‹", () => { _state.Previous(); Changed(); });
        _focus = Pill(_words.Focus, null, () => FocusRequested?.Invoke(), _focusGlyph);
        _zoomOut = Pill(_words.ZoomOut, _words.ZoomOutGlyph, () => { _state.ZoomOut(); Changed(); });
        _zoomIn = Pill(_words.ZoomIn, _words.ZoomInGlyph, () => { _state.ZoomIn(); Changed(); });
        _next = Pill(_words.Next, "›", () => { _state.Next(); Changed(); });
        _jump = Pill(_words.JumpToNow, _words.JumpToNowGlyph, () => { _state.JumpToNow(); Changed(); });

        // The range capsule: ink 0.07, label type, primary text (:243-252).
        _range.Style = (Style)Application.Current.Resources["TcCaptionTextStyle"];
        _range.FontSize = 12;
        _range.FontWeight = FontWeights.Medium;
        _range.Foreground = Brush("TcInkPrimaryBrush");
        var rangePill = new Grid
        {
            Height = 26,
            Padding = new Thickness(8, 0, 8, 0),
            CornerRadius = new CornerRadius(13),
        };
        rangePill.Children.Add(_range);
        _rangeFill = rangePill;

        int column = 0;
        foreach (FrameworkElement pill in new FrameworkElement[] { _previous, _focus, _zoomOut, rangePill, _zoomIn, _next, _jump })
        {
            Grid.SetColumn(pill, column++);
            pills.Children.Add(pill);
        }

        stack.Children.Add(pills);

        // Insets: the window's main inset across, s5 above and below (MonitorWindowView.swift:408-409).
        var body = new Grid { Padding = new Thickness(16, 10, 16, 10) };
        body.Children.Add(stack);

        // The 0.5 pt rule along its top (:414-416).
        var root = new Grid { Height = TracesGraphState.Height };
        root.Children.Add(body);
        root.Children.Add(new Rectangle
        {
            Height = 0.5,
            VerticalAlignment = VerticalAlignment.Top,
            Fill = Brush("TcHairlineDividerBrush"),
        });
        Content = root;

        ActualThemeChanged += (_, _) => { Paint(); _canvas.Invalidate(); };
        // A live theme or high contrast switch too (ThemeBrushes).
        ThemeBrushes.RedrawOnChange(this, () => { Paint(); _canvas.Invalidate(); });
        Unloaded += (_, _) => StopTicking();
        Paint();
        Render();
    }

    private readonly Grid _rangeFill;

    /// <summary>
    /// The History page the bars draw from: the last page of a read that
    /// did not fail, or null (<see cref="TracesGraphState.Readable"/>), and
    /// the traces still waiting.
    /// </summary>
    public void SetData(IReadOnlyList<MapHistoryRow>? history, IReadOnlyList<MapEntry> sessions)
    {
        (_history, _complete) = TracesGraphState.Readable(history);
        _sessions = sessions;
        Render();
    }

    /// <summary>The selection's tool, and whether the map is focused on it (MonitorShell.swift:199-201).</summary>
    public void SetTool(string? tool, bool focused)
    {
        _tool = tool;
        _focused = focused;
        Render();
    }

    private bool Motion => _uiSettings.AnimationsEnabled;

    private void Changed()
    {
        _leaving = _face;
        _change = _state.Change(reduceMotion: !Motion);
        _slide = _state.Slide;
        Render();
        if (_change is null || _leaving is null)
        {
            _leaving = null;
            return;
        }

        _changeClock.Restart();
        if (!_ticking)
        {
            _ticking = true;
            CompositionTarget.Rendering += OnRendering;
        }
    }

    private void OnRendering(object? sender, object e)
    {
        if (_change is null || _changeClock.Elapsed.TotalSeconds >= _change.Duration)
        {
            _leaving = null;
            StopTicking();
        }

        _canvas.Invalidate();
    }

    private void StopTicking()
    {
        if (_ticking)
        {
            _ticking = false;
            CompositionTarget.Rendering -= OnRendering;
        }

        _changeClock.Reset();
    }

    private void Render()
    {
        GraphFace face = _state.Face(_history, _complete, _sessions, _tool, _focused, DateTimeOffset.Now, TimeZoneInfo.Local, CultureInfo.CurrentCulture, _words);
        _face = face;
        _sharedValue.Text = face.SharedFigure;
        _keptValue.Text = face.KeptFigure;
        _range.Text = face.RangeLabel;
        AutomationProperties.SetName(_focus, face.FocusTip);
        ToolTipService.SetToolTip(_focus, face.FocusTip);
        _focusGlyph.Foreground = new SolidColorBrush(Ink(face.FocusInk));
        _focus.IsEnabled = face.CanFocus;
        _zoomOut.IsEnabled = face.CanZoomOut;
        _zoomIn.IsEnabled = face.CanZoomIn;
        _next.IsEnabled = face.CanStepNext;
        _jump.IsEnabled = face.CanJump;
        LayoutColumns();
        _canvas.Invalidate();
    }

    /// <summary>One transparent element per column: it takes the pointer and is read with the bar's words.</summary>
    private void LayoutColumns()
    {
        if (_face is not { } face)
        {
            return;
        }

        double width = _columns.ActualWidth;
        IReadOnlyList<(double X, double Width)> frames = BarGraph.ColumnFrames(width, face.Bars.Count);
        while (_columns.Children.Count > face.Bars.Count)
        {
            _columns.Children.RemoveAt(_columns.Children.Count - 1);
        }

        while (_columns.Children.Count < face.Bars.Count)
        {
            var cell = new Grid { Background = new SolidColorBrush(Color.FromArgb(0, 0, 0, 0)), HorizontalAlignment = HorizontalAlignment.Left };
            cell.PointerEntered += (s, _) => Hover((string)((FrameworkElement)s).Tag);
            cell.PointerExited += (s, _) =>
            {
                if (_state.Hovered == (string)((FrameworkElement)s).Tag)
                {
                    Hover(null);
                }
            };
            _columns.Children.Add(cell);
        }

        for (int i = 0; i < face.Bars.Count; i++)
        {
            var cell = (Grid)_columns.Children[i];
            cell.Tag = face.Bars[i].Id;
            cell.Width = frames.Count > i ? frames[i].Width : 0;
            cell.Margin = new Thickness(frames.Count > i ? frames[i].X : 0, 0, 0, 0);
            AutomationProperties.SetName(cell, face.Bars[i].Description);
        }
    }

    private void Hover(string? id)
    {
        if (_state.Hovered == id)
        {
            return;
        }

        _state.Hovered = id;
        Render();
    }

    private void OnDraw(CanvasControl sender, CanvasDrawEventArgs args)
    {
        if (_face is not { } face)
        {
            return;
        }

        double width = sender.ActualWidth;
        double p = 1;
        if (_leaving is { } leaving && _change is { } change)
        {
            p = Math.Clamp(change.At(_changeClock.Elapsed.TotalSeconds), 0, 1);

            // The window it replaces fades out where it was (GlassBarSlide removal: .opacity).
            DrawBars(args, leaving, width, 0, 1 - p, hovered: null);
        }

        // A later window from the trailing side, an earlier one from the
        // leading side, with a fade; a zoom fades in place.
        double from = _leaving is null ? 0 : _slide switch
        {
            GraphSlide.FromTrailing => width,
            GraphSlide.FromLeading => -width,
            _ => 0,
        };
        DrawBars(args, face, width, from * (1 - p), _leaving is null ? 1 : p, _state.Hovered);
    }

    private void DrawBars(CanvasDrawEventArgs args, GraphFace face, double width, double dx, double alpha, string? hovered)
    {
        var ds = args.DrawingSession;
        int count = face.Bars.Count;
        bool thin = BarGraph.Thin(count);
        double maximum = BarGraph.Maximum(face.Bars.Select(b => (b.Up, b.Down)), TracesGraphState.ScaleFloor);
        IReadOnlyList<(double X, double Width)> frames = BarGraph.ColumnFrames(width, count);
        Color shared = Ink("dataShared");
        Color kept = Ink("dataKept");
        var format = new CanvasTextFormat
        {
            FontSize = thin ? 10 : 11,
            WordWrapping = CanvasWordWrapping.NoWrap,
            VerticalAlignment = CanvasVerticalAlignment.Top,
        };

        for (int i = 0; i < count; i++)
        {
            GraphBar bar = face.Bars[i];
            bool isHovered = hovered == bar.Id;
            double x = frames[i].X + dx;
            double w = frames[i].Width;

            // Under the pointer the column takes a solid wash, never an outline.
            double wash = BarGraph.WashAlpha(isHovered);
            if (wash > 0)
            {
                double r = BarGraph.WashRadius(thin, 8);
                ds.FillRoundedRectangle(new Rect(x, 0, w, BarGraph.ColumnHeight), (float)r, (float)r, Ink("ink", wash * alpha));
            }

            BarMatrix matrix = BarGraph.Matrix(w, BarGraph.ColumnHeight, thin);
            double[] up = BarGraph.DotAlphas(bar.Up, maximum);
            double[] down = BarGraph.DotAlphas(bar.Down, maximum);
            Color unlit = Ink("ink", BarGraph.UnlitAlpha(isHovered) * alpha);
            for (int d = 0; d < matrix.Up.Count; d++)
            {
                int row = d % BarGraph.Rows;
                Dot(matrix.Up[d], up[row], shared);
                Dot(matrix.Down[d], down[row], kept);
            }

            (double ax, double ay, double aw, double ah) = matrix.Axis;
            ds.FillRectangle(new Rect(x + ax, ay, aw, ah), Ink("ink", BarGraph.AxisAlpha * alpha));

            // The label: centred on its column, never wrapped or cut; in a
            // thin graph the first and last hug the outer edge.
            if (bar.Label.Length > 0)
            {
                int align = BarGraph.LabelAlignment(i, count, thin);
                format.HorizontalAlignment = align < 0 ? CanvasHorizontalAlignment.Left
                    : align > 0 ? CanvasHorizontalAlignment.Right : CanvasHorizontalAlignment.Center;
                double top = BarGraph.ColumnHeight + LabelGap;
                Rect box = align < 0 ? new Rect(x, top, 200, LabelLine)
                    : align > 0 ? new Rect(x + w - 200, top, 200, LabelLine)
                    : new Rect(x + w / 2 - 100, top, 200, LabelLine);
                ds.DrawText(bar.Label, box, Ink(BarGraph.LabelTextInk(isHovered), alpha), format);
            }

            void Dot((double X, double Y) at, double lit, Color ink)
            {
                Color fill = lit > 0 ? With(ink, lit * alpha) : unlit;
                ds.FillCircle((float)(x + at.X), (float)at.Y, (float)matrix.Radius, fill);
            }
        }
    }

    /// <summary>The inks that follow the theme: the hairline and the range capsule's fill.</summary>
    private void Paint()
    {
        _rangeFill.Background = new SolidColorBrush(Ink("ink", 0.07));
        _focusGlyph.Foreground = new SolidColorBrush(Ink(_face?.FocusInk ?? "graphFocusOff"));
        foreach ((Ellipse dot, string token) in _dots)
        {
            dot.Fill = new SolidColorBrush(Ink(token));
            dot.Stroke = new SolidColorBrush(Ink("ink", 0.08));
        }
    }

    private readonly List<(Ellipse, string)> _dots = new();

    private bool HighContrast => _accessibility.HighContrast;

    private bool Dark => ThemeBrushes.Dark;

    /// <summary>A generated flat ink for the theme (MapArt); under High Contrast the system's text colour.</summary>
    private Color Ink(string token, double alpha = 1)
    {
        if (HighContrast)
        {
            Color system = _uiSettings.UIElementColor(UIElementType.WindowText);
            return Color.FromArgb((byte)Math.Round(255 * Math.Clamp(alpha, 0, 1)), system.R, system.G, system.B);
        }

        IReadOnlyDictionary<string, MapRgba> inks = Dark ? MapArt.Dark : MapArt.Light;
        if (!inks.TryGetValue(token, out MapRgba c))
        {
            c = inks["textPrimary"];
        }

        return Color.FromArgb((byte)Math.Round(255 * Math.Clamp(c.A * alpha, 0, 1)), c.R, c.G, c.B);
    }

    private static Color With(Color c, double alpha) => Color.FromArgb((byte)Math.Round(c.A * Math.Clamp(alpha, 0, 1)), c.R, c.G, c.B);

    private static Brush Brush(string key) => ThemeBrushes.Get(key);

    private static TextBlock Figure() => new()
    {
        FontWeight = FontWeights.SemiBold,
        FontSize = 12,
        VerticalAlignment = VerticalAlignment.Center,
        HorizontalAlignment = HorizontalAlignment.Right,
    };

    /// <summary>
    /// A legend cell (<c>GlassLegendCell</c>, Containers.swift:555-596):
    /// an 8 pt dot with its halo, the label in secondary ink, the value in
    /// primary semibold, 28 tall in a well, 10 in from each side.
    /// </summary>
    private UIElement LegendCell(string label, string token, TextBlock value)
    {
        var dot = new Ellipse { Width = 8, Height = 8, StrokeThickness = 1, VerticalAlignment = VerticalAlignment.Center };
        _dots.Add((dot, token));
        AutomationProperties.SetAccessibilityView(dot, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        var text = new TextBlock
        {
            Text = label,
            FontSize = 12,
            Foreground = Brush("TcInkSecondaryBrush"),
            TextTrimming = TextTrimming.CharacterEllipsis,
            VerticalAlignment = VerticalAlignment.Center,
        };
        value.Foreground = Brush("TcInkPrimaryBrush");
        var row = new Grid { ColumnSpacing = 8, Padding = new Thickness(10, 0, 10, 0), Height = 28 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var name = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 7 };
        name.Children.Add(dot);
        name.Children.Add(text);
        row.Children.Add(name);
        Grid.SetColumn(value, 1);
        row.Children.Add(value);
        var cell = new Grid
        {
            CornerRadius = (CornerRadius)Application.Current.Resources["TcControlCornerRadius"],
            Background = Brush("TcSurfaceInsetBrush"),
        };
        cell.Children.Add(row);

        // Read as one: the label, then the figure (accessibilityElement(children: .combine)).
        AutomationProperties.SetName(cell, label);
        return cell;
    }

    /// <summary>A 30 by 26 pill with a glyph; named with its core word, the glyph never read (Controls.swift:372-416).</summary>
    private static Button Pill(string name, string? glyph, Action onClick, FontIcon? icon = null)
    {
        var button = new Button
        {
            Width = 30,
            Height = 26,
            MinWidth = 0,
            MinHeight = 0,
            Padding = new Thickness(0),
            Style = (Style)Application.Current.Resources["TcSecondaryButtonStyle"],
            CornerRadius = new CornerRadius(13),
            Content = icon ?? new FontIcon { Glyph = Symbol(glyph ?? string.Empty), FontSize = 11 },
        };
        AutomationProperties.SetName(button, name);
        ToolTipService.SetToolTip(button, name);
        button.Click += (_, _) => onClick();
        return button;
    }

    /// <summary>
    /// The icon drawn for the core's glyph (<c>GlassPillIconButton.symbol</c>,
    /// Controls.swift:424-432): the glyph names the control; the icon is
    /// only how it is drawn, optically centred.
    /// </summary>
    private static string Symbol(string glyph) => glyph switch
    {
        "‹" or "<" => "",
        "›" or ">" => "",
        "−" or "-" => "",
        "+" => "",
        "›|" or ">|" => "",
        _ => glyph,
    };
}
