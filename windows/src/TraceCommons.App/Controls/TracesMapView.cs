using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Numerics;
using System.Threading.Tasks;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Brushes;
using Microsoft.Graphics.Canvas.Effects;
using Microsoft.Graphics.Canvas.Geometry;
using Microsoft.Graphics.Canvas.Text;
using Microsoft.Graphics.Canvas.UI.Xaml;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Markup;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop;
using TraceCommons.Interop.Map;
using TraceCommons.Interop.Shell;
using Windows.Foundation;
using Windows.UI.ViewManagement;
using Color = Windows.UI.Color;

namespace TraceCommons.App.Controls;

/// <summary>Which view the map pane shows (MonitorWindowView.swift:114-117).</summary>
public enum MapView
{
    Traces,
    PrivateAI,
}

/// <summary>
/// The map pane: the Mac's <c>MonitorMapPane</c>, <c>FlowMapView</c>,
/// <c>MapToolPile</c> and <c>GlassMapStage</c>
/// (MonitorWindowView.swift:1088-1258, FlowMapView.swift, MapToolPile.swift,
/// Indicators.swift:563-885), drawn with Win2D. Everything it draws is
/// computed by the pure ports in <c>TraceCommons.Interop.Map</c>; this
/// control replays them. Nodes stay focusable buttons named with the core's
/// words, so Narrator and the keyboard reach what the pointer can.
/// </summary>
/// <remarks>
/// <para>
/// Motion follows the Mac row by row (spec "Animation coverage"): the arcs'
/// dashes at 30 frames a second, the station and orb at 20, transfers and
/// the pile at 60, the shimmer and the pointer light on their own curves.
/// One <see cref="DispatcherQueueTimer"/> drives them at the fastest rate
/// anything needs; it runs only while the control is loaded, the window can
/// be seen (<see cref="WindowClocks"/>) and the system lets things move
/// (<see cref="UISettings.AnimationsEnabled"/>, read on every tick and on
/// every activation, so a change applies at once). With motion off the map
/// draws its still frame, and every model change still redraws it.
/// </para>
/// <para>
/// Colours come from the generated flat inks (<see cref="MapArt"/>) for the
/// control's theme, or from the system's own colours under High Contrast.
/// </para>
/// </remarks>
public sealed partial class TracesMapView : UserControl
{
    private readonly Grid _root = new();
    private readonly CanvasControl _canvas = new();
    private readonly Canvas _targets = new();
    private readonly Border _card = new();
    private readonly TextBlock _cardTitle = new();
    private readonly TextBlock _cardDetail = new();
    private readonly StackPanel _foot = new();
    private readonly StackPanel _legend = new() { Orientation = Orientation.Horizontal, Spacing = 10 };
    private readonly Button _zoomOut = new();
    private readonly Button _zoomIn = new();
    private readonly StackPanel _switch = new() { Orientation = Orientation.Horizontal, Spacing = 2 };
    private readonly ToggleButton _tracesView = new();
    private readonly ToggleButton _privateView = new();
    private readonly TextBlock _stateLine = new() { TextWrapping = TextWrapping.Wrap };
    private readonly ProgressRing _spinner = new() { Width = MapGeometry.LoadingSpinner, Height = MapGeometry.LoadingSpinner, IsActive = false };
    private readonly UISettings _uiSettings = new();
    private readonly AccessibilitySettings _accessibility = new();
    private readonly Stopwatch _clock = Stopwatch.StartNew();
    private readonly ToolPileState _pile = new();
    private readonly MapTransferPlayer _player = new();
    private DispatcherQueueTimer? _timer;

    private Func<string, string, Task<DaemonResponse>>? _call;
    private MapSnapshot? _snapshot;
    private IReadOnlyList<MapHistoryRow>? _lastHistory;
    private bool _reading;
    private bool _readAgain;
    private bool _readOnce;
    private FlowMapScene _scene = FlowMapScene.Empty;
    private MapView _view = MapView.Traces;
    private TracesMapStyle _style = TracesMapStyle.Launch;
    private double _zoom = 1;
    private string? _selectedTool;
    private string? _focusTool;
    private Dictionary<string, int> _revealed = new(StringComparer.Ordinal);
    private string? _hovered;
    private string? _focused;
    private Point? _pointer;
    private Point? _lastPointer;
    private double _pointerChangedAt = -1;
    private double _shownAt;
    private bool _loaded;
    private CanvasGeometry? _markUnit;
    private CanvasGeometry? _tile;
    private Size _tileSize;
    private readonly Dictionary<string, CanvasGeometry> _logos = new(StringComparer.Ordinal);
    private readonly Dictionary<string, Button> _buttons = new(StringComparer.Ordinal);

    /// <summary>A folder node was clicked: its project id (FlowMapView.swift:24-26).</summary>
    public event Action<string>? FolderSelected;

    public TracesMapView()
    {
        IsTabStop = false;
        _canvas.Draw += OnDraw;
        _canvas.CreateResources += (_, _) => DropResources();
        _canvas.IsHitTestVisible = false;
        AutomationProperties.SetAccessibilityView(_canvas, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);

        _cardTitle.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        _cardDetail.TextWrapping = TextWrapping.Wrap;
        _cardDetail.Margin = new Thickness(0, 3, 0, 0);
        _card.Child = new StackPanel { Children = { _cardTitle, _cardDetail } };
        _card.Padding = new Thickness(14, 12, 14, 12);
        _card.Width = MapGeometry.NodeCardWidth;
        _card.CornerRadius = new CornerRadius(12);
        _card.IsHitTestVisible = false;
        _card.Visibility = Visibility.Collapsed;
        AutomationProperties.SetAccessibilityView(_card, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);

        _zoomOut.Content = new FontIcon { Glyph = "", FontSize = 12 };
        _zoomIn.Content = new FontIcon { Glyph = "", FontSize = 12 };
        _zoomOut.Click += (_, _) => Zoom(-MapGeometry.ZoomStep);
        _zoomIn.Click += (_, _) => Zoom(MapGeometry.ZoomStep);
        var zoom = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2, Children = { _zoomOut, _zoomIn } };
        var footRow = new Grid { ColumnSpacing = 12 };
        footRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        footRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        footRow.Children.Add(_legend);
        Grid.SetColumn(zoom, 1);
        footRow.Children.Add(zoom);
        _foot.Children.Add(footRow);
        _foot.VerticalAlignment = VerticalAlignment.Bottom;
        _foot.Margin = new Thickness(14);

        _tracesView.Click += (_, _) => SetView(MapView.Traces);
        _privateView.Click += (_, _) => SetView(MapView.PrivateAI);
        _switch.Children.Add(_tracesView);
        _switch.Children.Add(_privateView);
        _switch.HorizontalAlignment = HorizontalAlignment.Right;
        _switch.VerticalAlignment = VerticalAlignment.Top;
        _switch.Margin = new Thickness(14);

        _stateLine.HorizontalAlignment = HorizontalAlignment.Left;
        _stateLine.VerticalAlignment = VerticalAlignment.Bottom;
        _stateLine.Margin = new Thickness(16, 0, 16, 60);
        _stateLine.Visibility = Visibility.Collapsed;
        _spinner.HorizontalAlignment = HorizontalAlignment.Center;
        _spinner.VerticalAlignment = VerticalAlignment.Center;

        _root.Children.Add(_canvas);
        _root.Children.Add(_targets);
        _root.Children.Add(_card);
        _root.Children.Add(_foot);
        _root.Children.Add(_switch);
        _root.Children.Add(_stateLine);
        _root.Children.Add(_spinner);
        _root.Background = new SolidColorBrush(Color.FromArgb(0, 0, 0, 0));
        _root.PointerMoved += OnPointerMoved;
        _root.PointerExited += (_, _) => SetPointer(null);
        _root.SizeChanged += (_, _) => Rebuild();
        Content = _root;

        _player.Changed += () => { Rebuild(); Kick(); };
        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
        ActualThemeChanged += (_, _) => { DropResources(); Redraw(); };
        ApplyWords();
    }

    // ---- Inputs --------------------------------------------------------

    /// <summary>The map's reads, through the window's daemon host.</summary>
    public void Attach(Func<string, string, Task<DaemonResponse>> call)
    {
        _call = call;
        _ = ReadAsync();
    }

    /// <summary>Reads again (the queue or the status changed); bursts coalesce into one more read.</summary>
    public async Task ReadAsync()
    {
        if (_call is null)
        {
            return;
        }

        if (_reading)
        {
            _readAgain = true;
            return;
        }

        _reading = true;
        try
        {
            do
            {
                _readAgain = false;
                MapSnapshot snapshot = await MapFeed.ReadAsync(_call).ConfigureAwait(true);
                // A row History gained since the last read is a contribution
                // landing (MapTransferFeed.swift:31-33); the first read finds none.
                _player.Enqueue(MapTransferDetector.Contributions(_lastHistory, snapshot.History), Now);
                _lastHistory = snapshot.History;
                _snapshot = snapshot;
                _readOnce = true;
                Rebuild();
            }
            while (_readAgain);
        }
        finally
        {
            _reading = false;
        }
    }

    /// <summary>The map's style (Settings, General; TracesMapStyle.swift): it changes at once.</summary>
    public TracesMapStyle MapStyle
    {
        get => _style;
        set
        {
            _style = value;
            _player.Style = value;
            Redraw();
        }
    }

    /// <summary>The selection's tool: rung, the rest faded (FlowMapScene.swift:393-396).</summary>
    public string? SelectedTool
    {
        get => _selectedTool;
        set
        {
            _selectedTool = value;
            Rebuild();
        }
    }

    /// <summary>
    /// The tool the map is focused on, scaled 1.6 times about it
    /// (FlowMapView.swift:20-22; the Mac's binoculars in the graph footer).
    /// </summary>
    public string? FocusTool
    {
        get => _focusTool;
        set
        {
            _focusTool = value;
            Rebuild();
        }
    }

    /// <summary>The view a tab picks (MonitorWindowView.swift:728-734): Private AI for Inference, Traces for Traces.</summary>
    public void FollowTab(MonitorTab tab)
    {
        if (tab == MonitorTab.Inference)
        {
            SetView(MapView.PrivateAI);
        }
        else if (tab == MonitorTab.Traces)
        {
            SetView(MapView.Traces);
        }
    }

    private void SetView(MapView view)
    {
        _view = view;
        _tracesView.IsChecked = view == MapView.Traces;
        _privateView.IsChecked = view == MapView.PrivateAI;
        Rebuild();
    }

    private void Zoom(double delta)
    {
        _zoom = Math.Clamp(_zoom + delta, MapGeometry.ZoomMin, MapGeometry.ZoomMax);
        _zoomOut.IsEnabled = _zoom > MapGeometry.ZoomMin;
        _zoomIn.IsEnabled = _zoom < MapGeometry.ZoomMax;
        Rebuild();
    }

    // ---- Words --------------------------------------------------------

    private static FlowMapCopy? Words => FlowMapCopy.Table;

    private void ApplyWords()
    {
        string tracesTitle = MonitorWords.TabTitle(MonitorTab.Traces);
        string privateTitle = MonitorWords.TabTitle(MonitorTab.Inference);
        _tracesView.Content = tracesTitle;
        AutomationProperties.SetName(_tracesView, tracesTitle);
        _privateView.Content = privateTitle;
        AutomationProperties.SetName(_privateView, privateTitle);
        _privateView.Visibility = string.IsNullOrEmpty(privateTitle) ? Visibility.Collapsed : Visibility.Visible;
        AutomationProperties.SetName(_switch, MonitorShellCopy.Shell("map_views_label"));
        string reduce = MonitorShellCopy.Top("reduce"), enlarge = MonitorShellCopy.Top("enlarge");
        AutomationProperties.SetName(_zoomOut, reduce);
        ToolTipService.SetToolTip(_zoomOut, reduce);
        AutomationProperties.SetName(_zoomIn, enlarge);
        ToolTipService.SetToolTip(_zoomIn, enlarge);
        AutomationProperties.SetName(this, Words?.MapLabel ?? tracesTitle);
        _tracesView.IsChecked = true;
    }

    private void BuildLegend()
    {
        _legend.Children.Clear();
        if (_view != MapView.Traces)
        {
            return;
        }

        bool dark = Dark;
        foreach (string mode in new[] { MapModes.AutoUpload, MapModes.Ask, MapModes.Ignore })
        {
            (string fill, string stroke) = MapGeometry.Rule(mode);
            var dot = new Microsoft.UI.Xaml.Shapes.Ellipse
            {
                Width = 10,
                Height = 10,
                Fill = new SolidColorBrush(Ink(fill)),
                Stroke = new SolidColorBrush(Ink(stroke)),
                StrokeThickness = 1.5,
            };
            _legend.Children.Add(LegendItem(dot, WatchCopy.ModeLabel(mode)));
        }

        if (Words is { } words)
        {
            var ring = new Microsoft.UI.Xaml.Shapes.Ellipse
            {
                Width = 10,
                Height = 10,
                Stroke = new SolidColorBrush(Ink("statusOff")),
                StrokeThickness = 1.5,
                StrokeDashArray = new DoubleCollection { 2 / 1.5, 2 / 1.5 },
            };
            _legend.Children.Add(LegendItem(ring, words.LegendNotWatched));
            var lockGlyph = new FontIcon { Glyph = "", FontSize = 8, Foreground = new SolidColorBrush(Ink("textTertiary")) };
            _legend.Children.Add(LegendItem(lockGlyph, words.LegendMemory));
        }

        _ = dark;
    }

    private UIElement LegendItem(FrameworkElement mark, string word)
    {
        mark.VerticalAlignment = VerticalAlignment.Center;
        var item = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        item.Children.Add(new Border { Width = 10, Height = 10, Child = mark });
        item.Children.Add(new TextBlock { Text = word, FontSize = 11, Foreground = new SolidColorBrush(Ink("textTertiary")), VerticalAlignment = VerticalAlignment.Center });
        AutomationProperties.SetAccessibilityView(item, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        return item;
    }

    // ---- The scene ----------------------------------------------------

    private MapSize PaneSize => new(Math.Max(1, _root.ActualWidth), Math.Max(1, _root.ActualHeight));

    /// <summary>
    /// Rebuilds the scene from the last read and redraws: every model change
    /// redraws, whatever the clock is doing (checklist O1).
    /// </summary>
    private void Rebuild()
    {
        MapSize space = FlowMapScene.SpaceFor(PaneSize);
        if (_view == MapView.Traces)
        {
            if (_snapshot is { } s)
            {
                TracesTree tree = s.Tree(MapFeed.CoreScansWhenUnset);
                _scene = FlowMapScene.Traces(tree, s.Gate, Words, s.Contributions, _selectedTool, _revealed, space);
            }
            else
            {
                _scene = FlowMapScene.Empty;
            }
        }
        else
        {
            PrivateInferenceCopy? copy = PrivateInferenceSurface.Copy();
            _scene = _snapshot is { } s
                ? FlowMapScene.PrivateAI(
                    s.Harnesses, MonitorWords.TabTitle(MonitorTab.Inference), s.Destinations?.PrivateAi ?? s.Status?.PrivateInferenceState,
                    row => copy is null ? null : HarnessSurface.RowSentence(row, copy),
                    row => HarnessSurface.State(row.StateLabel), Words, s.Destinations, space)
                    .AddingManagedRoutes(s.Destinations, Words)
                : FlowMapScene.Empty with { Space = space };
        }

        _spinner.IsActive = !_readOnce;
        _spinner.Visibility = _readOnce ? Visibility.Collapsed : Visibility.Visible;
        bool failed = _readOnce && _snapshot is { Ready: false } && _view == MapView.Traces;
        _stateLine.Text = failed ? MonitorShellCopy.Top("core_unreachable") : string.Empty;
        _stateLine.Visibility = failed ? Visibility.Visible : Visibility.Collapsed;
        _foot.Visibility = _view == MapView.Traces ? Visibility.Visible : Visibility.Collapsed;
        BuildLegend();
        PlaceTargets();
        PlaceCard();
        Redraw();
        Kick();
    }

    private MapGeometry Fit => new(PaneSize, _zoom, _focusTool is null ? null : _scene.FocusPoint(_focusTool), _scene.Space);

    private bool Healthy => _snapshot is { Ready: true } s && s.Status is { Paused: false };

    // ---- Nodes as controls (FlowMapView.swift:598-628) ------------------

    private static readonly ControlTemplate ClearTemplate = (ControlTemplate)XamlReader.Load(
        "<ControlTemplate xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\" TargetType=\"Button\">"
        + "<Grid Background=\"Transparent\" /></ControlTemplate>");

    private void PlaceTargets()
    {
        MapGeometry fit = Fit;
        var keep = new HashSet<string>(StringComparer.Ordinal);
        bool traces = _scene.Stack is not null;
        foreach (MapNode node in _scene.Nodes)
        {
            keep.Add(node.Id);
            if (!_buttons.TryGetValue(node.Id, out Button? button))
            {
                button = new Button { Template = ClearTemplate, Padding = new Thickness(0), UseSystemFocusVisuals = true };
                string id = node.Id;
                button.Click += (_, _) => OnNodeClick(id);
                button.PointerEntered += (_, _) => { _hovered = id; PlaceCard(); Redraw(); };
                button.PointerExited += (_, _) => { if (_hovered == id) { _hovered = null; PlaceCard(); Redraw(); } };
                button.GotFocus += (_, _) => { _focused = id; PlaceCard(); Redraw(); };
                button.LostFocus += (_, _) => { if (_focused == id) { _focused = null; PlaceCard(); Redraw(); } };
                _buttons[id] = button;
                _targets.Children.Add(button);
            }

            double side = Math.Max(24, (node.Radius * 2 + 8) * fit.Scale);
            MapPoint at = fit.Point(node.At);
            button.Width = side;
            button.Height = side;
            Canvas.SetLeft(button, at.X - side / 2);
            Canvas.SetTop(button, at.Y - side / 2);
            AutomationProperties.SetName(button, node.Label);
            AutomationProperties.SetHelpText(button, node.Detail);
            ToolTipService.SetToolTip(button, node.Label);
            // On the Traces map the coins take no clicks: the pile under the
            // pointer is the card's (FlowMapView.swift:621).
            button.IsHitTestVisible = !traces || !node.Id.StartsWith("tool:", StringComparison.Ordinal);
        }

        foreach (string gone in _buttons.Keys.Where(k => !keep.Contains(k)).ToList())
        {
            _targets.Children.Remove(_buttons[gone]);
            _buttons.Remove(gone);
            if (_focused == gone)
            {
                _focused = null;
            }

            if (_hovered == gone)
            {
                _hovered = null;
            }
        }

        PlaceReveals(fit);
    }

    private readonly List<Button> _reveals = new();

    /// <summary>The folder pill (FlowMapView.swift:630-669): "+N", or the core's fold-back word.</summary>
    private void PlaceReveals(MapGeometry fit)
    {
        foreach (Button b in _reveals)
        {
            _targets.Children.Remove(b);
        }

        _reveals.Clear();
        foreach (MapFolderReveal reveal in _scene.Reveals)
        {
            bool folds = reveal.Hidden == 0;
            string? text = folds ? Words?.FoldFolders : "+" + reveal.Hidden;
            if (text is null)
            {
                continue;
            }

            string name = (folds ? Words?.FoldFoldersLabel : Words?.RevealFolders(reveal.Next)) ?? text;
            double unit = Math.Max(fit.Scale, MapGeometry.LabelFloor);
            double size = MapGeometry.LabelType(fit.Scale).LabelSize;
            MapPoint column = fit.Point(reveal.At);
            double end = column.X - FlowMapScene.FolderRadius * fit.Scale - 6 * unit;
            var pill = new Button
            {
                Content = new TextBlock { Text = text, FontSize = size, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold },
                Padding = new Thickness(8 * unit, 0, 8 * unit, 0),
                Height = size + 8 * unit,
                CornerRadius = new CornerRadius((size + 8 * unit) / 2),
                Opacity = reveal.Dim,
            };
            AutomationProperties.SetName(pill, name);
            ToolTipService.SetToolTip(pill, name);
            MapFolderReveal captured = reveal;
            pill.Click += (_, _) =>
            {
                _revealed = new Dictionary<string, int>(FlowMapScene.Revealing(captured, _revealed), StringComparer.Ordinal);
                Rebuild();
            };
            pill.Loaded += (_, _) =>
            {
                Canvas.SetLeft(pill, end - pill.ActualWidth);
                Canvas.SetTop(pill, column.Y - pill.ActualHeight / 2);
            };
            Canvas.SetLeft(pill, end - 40);
            Canvas.SetTop(pill, column.Y - pill.Height / 2);
            _reveals.Add(pill);
            _targets.Children.Add(pill);
        }
    }

    private void OnNodeClick(string id)
    {
        if (id.StartsWith("folder:", StringComparison.Ordinal))
        {
            FolderSelected?.Invoke(id["folder:".Length..]);
        }
    }

    private string? Selection => _hovered ?? _focused;

    /// <summary>The node card beside the hovered or focused node (FlowMapView.swift:680-735).</summary>
    private void PlaceCard()
    {
        MapNode? node = Selection is { } id ? _scene.Nodes.FirstOrDefault(n => n.Id == id) : null;
        if (node is null)
        {
            _card.Visibility = Visibility.Collapsed;
            return;
        }

        MapGeometry fit = Fit;
        MapPoint centre = fit.Point(node.At);
        double half = MapGeometry.DrawnHalfWidth(node, _scene.Stack is not null, _style) * fit.Scale;
        double x = MapGeometry.CardX(centre.X, half, MapGeometry.NodeCardWidth, _root.ActualWidth);
        double y = MapGeometry.CardY(centre.Y, _root.ActualHeight);
        _cardTitle.Text = node.CardTitle;
        _cardDetail.Text = node.Detail;
        _cardTitle.Foreground = new SolidColorBrush(Ink("textPrimary"));
        _cardDetail.Foreground = new SolidColorBrush(Ink("textSecondary"));
        _card.Background = new SolidColorBrush(Ink("paneOpaque"));
        _card.HorizontalAlignment = HorizontalAlignment.Left;
        _card.VerticalAlignment = VerticalAlignment.Top;
        _card.Visibility = Visibility.Visible;
        _card.Measure(new Size(MapGeometry.NodeCardWidth, double.PositiveInfinity));
        _card.Margin = new Thickness(x - MapGeometry.NodeCardWidth / 2, Math.Max(0, y - _card.DesiredSize.Height / 2), 0, 0);
    }

    // ---- The clock ----------------------------------------------------

    private double Now => _clock.Elapsed.TotalSeconds;

    private bool Motion => _uiSettings.AnimationsEnabled;

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        _loaded = true;
        _shownAt = Now;
        WindowClocks.Changed += OnWindowClocksChanged;
        Rebuild();
    }

    private void OnUnloaded(object sender, RoutedEventArgs e)
    {
        _loaded = false;
        WindowClocks.Changed -= OnWindowClocksChanged;
        StopTimer();
    }

    private void OnWindowClocksChanged(bool running)
    {
        if (running)
        {
            _shownAt = Now;
            Kick();
        }
        else
        {
            StopTimer();
        }
    }

    /// <summary>
    /// What the map needs next: the interval of its fastest moving part, or
    /// null when nothing moves and only a model change redraws it.
    /// </summary>
    private double? WantedInterval()
    {
        if (!MapMotion.ClockRuns(Motion, _loaded, WindowClocks.Running) || Visibility != Visibility.Visible)
        {
            return null;
        }

        double now = Now;
        bool library = _scene.Stack is not null && _scene.Nodes.Any(n => n.Kind == MapNodeKind.Library);
        bool transfers = _scene.Stack is not null && _player.Run is { } run && now - run.Start <= run.End;
        bool pile = _pile.Moving(now);
        bool light = _pointerChangedAt >= 0 && now - _pointerChangedAt < MapMotion.PointerLightFade.Duration;
        bool shimmer = MapField.UntilNextPass(now - _shownAt) == 0;
        if (transfers || pile || light || shimmer)
        {
            return MapMotion.TransferFrameInterval;
        }

        (double interval, bool paused) = MapMotion.Resting(false, _scene.Flows, Healthy, library);
        if (!paused)
        {
            return interval;
        }

        // Nothing moves until the next shimmer pass or the next card to go.
        double wait = MapField.UntilNextPass(now - _shownAt);
        if (_player.NextDeadline is { } deadline)
        {
            wait = Math.Min(wait, Math.Max(0, deadline - now));
        }

        return wait;
    }

    private void Kick()
    {
        if (WantedInterval() is not { } interval)
        {
            StopTimer();
            return;
        }

        _timer ??= CreateTimer();
        _timer.Interval = TimeSpan.FromSeconds(Math.Max(interval, MapMotion.TransferFrameInterval));
        if (!_timer.IsRunning)
        {
            _timer.Start();
        }
    }

    private DispatcherQueueTimer CreateTimer()
    {
        DispatcherQueueTimer timer = DispatcherQueue.CreateTimer();
        timer.IsRepeating = true;
        timer.Tick += (_, _) =>
        {
            _player.Tick(Now);
            Redraw();
            Kick();
        };
        return timer;
    }

    private void StopTimer() => _timer?.Stop();

    private void Redraw() => _canvas.Invalidate();

    // ---- The pointer --------------------------------------------------

    private void OnPointerMoved(object sender, PointerRoutedEventArgs e) => SetPointer(e.GetCurrentPoint(_root).Position);

    private void SetPointer(Point? point)
    {
        bool had = _pointer is not null;
        _pointer = point;
        if (point is { } p)
        {
            _lastPointer = p;
        }

        if (had != (point is not null))
        {
            _pointerChangedAt = Now;
        }

        if (_scene.Stack is { } stack)
        {
            MapGeometry fit = Fit;
            (double tall, double wide, _) = _pile.Stages(Now, !Motion);
            bool moving = _player.Run is { } run && run.Moving(Now - run.Start);
            double height = Lerp(stack.Height(false), stack.Height(true), moving ? 0 : tall) * fit.Scale;
            double width = Lerp(MapToolStack.Width, MapToolStack.OpenWidth, moving ? 0 : wide) * fit.Scale;
            MapPoint left = fit.Point(stack.PortLeft);
            bool inside = point is { } q && q.X >= left.X && q.X <= left.X + width && q.Y >= left.Y - height / 2 && q.Y <= left.Y + height / 2;
            _pile.Hover(inside, Now);
        }

        Redraw();
        Kick();
    }

    private static double Lerp(double a, double b, double t) => a + (b - a) * t;

    // ---- Colours ------------------------------------------------------

    private bool Dark => ActualTheme == ElementTheme.Dark;

    private bool HighContrast => _accessibility.HighContrast;

    /// <summary>A generated flat ink for the theme; under High Contrast the system's own text or ground colour.</summary>
    private Color Ink(string token, double alpha = 1)
    {
        if (HighContrast)
        {
            Color system = token is "paneOpaque" or "wellFill" or "mapFieldInner" or "mapFieldOuter"
                ? _uiSettings.UIElementColor(UIElementType.Window)
                : _uiSettings.UIElementColor(UIElementType.WindowText);
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

    private static Color Rgba(MapRgba c, double alpha = 1) => Color.FromArgb((byte)Math.Round(255 * Math.Clamp(c.A * alpha, 0, 1)), c.R, c.G, c.B);

    // ---- Drawing ------------------------------------------------------

    private void DropResources()
    {
        _markUnit = null;
        _tile = null;
        _logos.Clear();
    }

    private void OnDraw(CanvasControl sender, CanvasDrawEventArgs args)
    {
        CanvasDrawingSession ds = args.DrawingSession;
        double now = Now;
        bool motion = Motion && WindowClocks.Running;
        var size = new Size(sender.ActualWidth, sender.ActualHeight);
        if (size.Width <= 0 || size.Height <= 0)
        {
            return;
        }

        DrawField(ds, sender, size, now, motion);
        if (!_scene.DrawsMap)
        {
            return;
        }

        MapGeometry fit = Fit;
        DrawArcs(ds, sender, fit, motion ? now : 0, motion);
        foreach (MapNode node in _scene.Nodes)
        {
            DrawNode(ds, sender, node, fit);
        }

        foreach (MapOverflow overflow in _scene.Overflows)
        {
            DrawText(ds, "+" + overflow.Count, fit.Point(overflow.At), 11, Ink("textTertiary"), CanvasHorizontalAlignment.Center, bold: false);
        }

        if (_scene.Stack is { } stack)
        {
            if (_scene.Nodes.FirstOrDefault(n => n.Id == "library") is { } library)
            {
                DrawLibrary(ds, sender, library, fit, motion ? DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() / 1000.0 - 978307200 : 0, !motion);
            }

            MapTransferRun? run = motion && _player.Run is { } r ? r : null;
            double t = run is null ? 0 : now - run.Start;
            if (run is not null)
            {
                DrawTransfers(ds, sender, run, t, fit);
            }

            if (_style == TracesMapStyle.Launch && run?.Action != TransferAction.Contribute
                && _scene.Nodes.FirstOrDefault(n => n.Id == "hub") is { } hub)
            {
                DrawRocket(ds, sender, fit.Point(TransferDrawing.RocketRest(hub.At)), -Math.PI / 2, fit.Scale, null, 0, null);
            }

            DrawPile(ds, sender, stack, fit, run, t, now, motion);
        }
    }

    private void DrawField(CanvasDrawingSession ds, ICanvasResourceCreator device, Size size, double now, bool motion)
    {
        float w = (float)size.Width, h = (float)size.Height;
        Color outer = Ink("mapFieldOuter"), inner = Ink("mapFieldInner");
        ds.FillRectangle(0, 0, w, h, outer);
        using (var field = new CanvasRadialGradientBrush(device, inner, outer)
        {
            Center = new Vector2(w * (float)MapField.CentreX, h * (float)MapField.CentreY),
            RadiusX = w * (float)MapField.RadiusX,
            RadiusY = h * (float)MapField.RadiusY,
        })
        {
            ds.FillRectangle(0, 0, w, h, field);
        }

        if (HighContrast)
        {
            return;
        }

        CanvasGeometry tile = Tile(device, size);
        (MapRgba resting, MapRgba lit, MapRgba crest, MapRgba bloom) = MapField.Inks(Dark);
        var fadeStops = MapField.FadeStops.Select(s => new CanvasGradientStop { Position = (float)s.At, Color = Color.FromArgb((byte)Math.Round(255 * s.Opacity), 255, 255, 255) }).ToArray();
        using var fade = new CanvasRadialGradientBrush(device, fadeStops)
        {
            Center = new Vector2(w * (float)MapField.FadeCentreX, h * (float)MapField.FadeCentreY),
            RadiusX = w * (float)MapField.FadeRadius,
            RadiusY = h * (float)MapField.FadeRadius,
        };
        using (ds.CreateLayer(fade))
        {
            ds.FillGeometry(tile, Rgba(resting));

            // The shimmer: the whole tile filled once with a band of light (Indicators.swift:809-830).
            double sweep = motion ? MapField.Sweep(now - _shownAt, false) : 0;
            if (sweep > 0)
            {
                (MapPoint start, MapPoint end) = MapField.ShimmerBand(sweep, new MapSize(w, h));
                Color c = Rgba(crest);
                using var band = new CanvasLinearGradientBrush(device, new[]
                {
                    new CanvasGradientStop { Position = 0, Color = With(c, 0) },
                    new CanvasGradientStop { Position = 0.5f, Color = c },
                    new CanvasGradientStop { Position = 1, Color = With(c, 0) },
                })
                {
                    StartPoint = new Vector2((float)start.X, (float)start.Y),
                    EndPoint = new Vector2((float)end.X, (float)end.Y),
                };
                ds.FillGeometry(tile, band);
            }

            // The marks near the pointer, lit, fading on Motion.fast (:835-865).
            double fadeIn = LightOpacity(now, motion);
            if (fadeIn > 0 && (_pointer ?? _lastPointer) is { } p)
            {
                var near = (p.X - MapField.Reach, p.Y - MapField.Reach, MapField.Reach * 2, MapField.Reach * 2);
                CanvasGeometry mark = MarkUnit(device);
                foreach (MapPoint origin in MapField.Origins(new MapSize(w, h), near))
                {
                    double half = MapField.MarkSize / 2;
                    double l = MapField.Light(Math.Sqrt(Math.Pow(origin.X + half - p.X, 2) + Math.Pow(origin.Y + half - p.Y, 2)));
                    if (l > 0)
                    {
                        ds.FillGeometry(mark.Transform(Matrix3x2.CreateScale((float)MapField.MarkSize) * Matrix3x2.CreateTranslation((float)origin.X, (float)origin.Y)), Rgba(lit, l * fadeIn));
                    }
                }

                foreach (MapPoint dot in MapField.Dots(new MapSize(w, h), near))
                {
                    double l = MapField.Light(Math.Sqrt(Math.Pow(dot.X - p.X, 2) + Math.Pow(dot.Y - p.Y, 2)));
                    if (l > 0)
                    {
                        ds.FillCircle((float)dot.X, (float)dot.Y, (float)(MapField.DotSize / 2), Rgba(lit, l * fadeIn));
                    }
                }
            }
        }

        // The bloom under the pointer, over the marks (:868-885).
        double bloomFade = LightOpacity(now, motion);
        if (bloomFade > 0 && (_pointer ?? _lastPointer) is { } q)
        {
            Color b = Rgba(bloom, bloomFade);
            using var glow = new CanvasRadialGradientBrush(device, b, With(b, 0))
            {
                Center = new Vector2((float)q.X, (float)q.Y),
                RadiusX = (float)MapField.Reach,
                RadiusY = (float)MapField.Reach,
            };
            ds.FillCircle((float)q.X, (float)q.Y, (float)MapField.Reach, glow);
        }
    }

    /// <summary>The pointer light's opacity: in and out on Motion.fast, at once under reduce motion.</summary>
    private double LightOpacity(double now, bool motion)
    {
        bool on = _pointer is not null;
        if (!motion || _pointerChangedAt < 0)
        {
            return on ? 1 : 0;
        }

        double p = MapMotion.PointerLightFade.At(now - _pointerChangedAt);
        return on ? p : 1 - p;
    }

    private CanvasGeometry MarkUnit(ICanvasResourceCreator device) => _markUnit ??= PathGeometry(device, NearMarkPath.Unit);

    /// <summary>Every mark and dot of the field as one geometry, built once per size (checklist O3).</summary>
    private CanvasGeometry Tile(ICanvasResourceCreator device, Size size)
    {
        if (_tile is not null && _tileSize == size)
        {
            return _tile;
        }

        var parts = new List<CanvasGeometry>();
        CanvasGeometry mark = MarkUnit(device);
        var space = new MapSize(size.Width, size.Height);
        foreach (MapPoint origin in MapField.Origins(space))
        {
            parts.Add(mark.Transform(Matrix3x2.CreateScale((float)MapField.MarkSize) * Matrix3x2.CreateTranslation((float)origin.X, (float)origin.Y)));
        }

        foreach (MapPoint dot in MapField.Dots(space))
        {
            parts.Add(CanvasGeometry.CreateCircle(device, (float)dot.X, (float)dot.Y, (float)(MapField.DotSize / 2)));
        }

        _tile = CanvasGeometry.CreateGroup(device, parts.ToArray(), CanvasFilledRegionDetermination.Winding);
        _tileSize = size;
        return _tile;
    }

    private static CanvasGeometry PathGeometry(ICanvasResourceCreator device, IReadOnlyList<PathStep> steps)
    {
        using var builder = new CanvasPathBuilder(device);
        builder.SetFilledRegionDetermination(CanvasFilledRegionDetermination.Winding);
        bool open = false;
        foreach (PathStep s in steps)
        {
            switch (s.Kind)
            {
                case PathStepKind.Move:
                    if (open)
                    {
                        builder.EndFigure(CanvasFigureLoop.Open);
                    }

                    builder.BeginFigure((float)s.X, (float)s.Y);
                    open = true;
                    break;
                case PathStepKind.Line:
                    if (!open)
                    {
                        builder.BeginFigure((float)s.X, (float)s.Y);
                        open = true;
                    }
                    else
                    {
                        builder.AddLine((float)s.X, (float)s.Y);
                    }

                    break;
                case PathStepKind.Cubic:
                    builder.AddCubicBezier(new Vector2((float)s.X1, (float)s.Y1), new Vector2((float)s.X2, (float)s.Y2), new Vector2((float)s.X, (float)s.Y));
                    break;
                case PathStepKind.Close:
                    if (open)
                    {
                        builder.EndFigure(CanvasFigureLoop.Closed);
                        open = false;
                    }

                    break;
            }
        }

        if (open)
        {
            builder.EndFigure(CanvasFigureLoop.Open);
        }

        return CanvasGeometry.CreatePath(builder);
    }

    /// <summary>A tool's logo in its viewBox units, parsed once.</summary>
    private CanvasGeometry? Logo(ICanvasResourceCreator device, string name)
    {
        if (_logos.TryGetValue(name, out CanvasGeometry? cached))
        {
            return cached;
        }

        if (!MapArt.Logos.TryGetValue(name, out LogoArtwork? art))
        {
            return null;
        }

        var parts = new List<CanvasGeometry>();
        foreach (string data in art.Paths)
        {
            if (SvgPathData.Parse(data) is { } steps)
            {
                parts.Add(PathGeometry(device, steps));
            }
        }

        CanvasGeometry geometry = CanvasGeometry.CreateGroup(device, parts.ToArray(), CanvasFilledRegionDetermination.Winding);
        _logos[name] = geometry;
        return geometry;
    }

    /// <summary>The tool's logo in its tint centred in a square, or its initials (FlowMapView.swift:528-536).</summary>
    private void DrawMark(CanvasDrawingSession ds, ICanvasResourceCreator device, GlassToolArt tool, MapPoint centre, double side, double alpha = 1)
    {
        Color tint = Ink(tool.TintToken, alpha);
        if (tool.Logo is { } name && MapArt.Logos.TryGetValue(name, out LogoArtwork? art) && Logo(device, name) is { } geometry)
        {
            double scale = Math.Min(side / art.Width, side / art.Height);
            Matrix3x2 m = Matrix3x2.CreateTranslation(-(float)(art.X + art.Width / 2), -(float)(art.Y + art.Height / 2))
                * Matrix3x2.CreateScale((float)scale)
                * Matrix3x2.CreateTranslation((float)centre.X, (float)centre.Y);
            ds.FillGeometry(geometry.Transform(m), tint);
        }
        else
        {
            DrawText(ds, tool.Initials, centre, Math.Max(9, side * 0.6), tint, CanvasHorizontalAlignment.Center, bold: true, mono: true);
        }
    }

    private void DrawArcs(CanvasDrawingSession ds, ICanvasResourceCreator device, MapGeometry fit, double seconds, bool motion)
    {
        bool strong = HighContrast;
        float width = (float)(MapGeometry.ArcWidth * fit.Scale);
        foreach (MapArc arc in _scene.Arcs)
        {
            using var builder = new CanvasPathBuilder(device);
            MapPoint a = fit.Point(arc.From), c1 = fit.Point(arc.Control1), c2 = fit.Point(arc.Control2), b = fit.Point(arc.To);
            builder.BeginFigure((float)a.X, (float)a.Y);
            builder.AddCubicBezier(new Vector2((float)c1.X, (float)c1.Y), new Vector2((float)c2.X, (float)c2.Y), new Vector2((float)b.X, (float)b.Y));
            builder.EndFigure(CanvasFigureLoop.Open);
            using CanvasGeometry path = CanvasGeometry.CreatePath(builder);
            (string ink, double alpha, double[]? dash, bool round) = MapGeometry.ArcStroke(arc.Style, Healthy, strong);
            Color colour = ink == "white" ? Color.FromArgb((byte)Math.Round(255 * alpha * arc.Dim), 255, 255, 255) : Ink(ink, alpha * arc.Dim);
            using var style = new CanvasStrokeStyle();
            if (round)
            {
                style.StartCap = style.EndCap = style.DashCap = CanvasCapStyle.Round;
            }

            if (dash is not null)
            {
                style.CustomDashStyle = dash.Select(d => (float)(d * fit.Scale / width)).ToArray();
            }

            ds.DrawGeometry(path, colour, width, style);
            if (arc.Style == MapArcStyle.Flowing && Healthy)
            {
                // The moving dash, 18 points a second from the arc's start
                // (FlowMapView.swift:290-295; dashPhase: -phase * scale).
                float flowWidth = (float)(width * MapGeometry.FlowingWidthFactor);
                using var flow = new CanvasStrokeStyle
                {
                    StartCap = CanvasCapStyle.Round,
                    EndCap = CanvasCapStyle.Round,
                    DashCap = CanvasCapStyle.Round,
                    CustomDashStyle = MapGeometry.FlowingDash.Select(d => (float)(d * fit.Scale / flowWidth)).ToArray(),
                    DashOffset = (float)MapMotion.DashOffsetInWidths(MapMotion.DashPhase(seconds, fit.Scale, !motion), flowWidth),
                };
                ds.DrawGeometry(path, Ink("statusOn", arc.Dim), flowWidth, flow);
            }
        }
    }

    private void DrawNode(CanvasDrawingSession ds, ICanvasResourceCreator device, MapNode node, MapGeometry fit)
    {
        bool traces = _scene.Stack is not null;
        if (traces && node.Kind == MapNodeKind.Tool)
        {
            return;
        }

        if (traces && node.Kind == MapNodeKind.Library)
        {
            return;
        }

        MapPoint centre = fit.Point(node.At);
        float r = (float)(node.Radius * fit.Scale);
        double s = fit.Scale;
        double dim = node.Dim;
        bool selected = Selection == node.Id;
        switch (node.Kind)
        {
            case MapNodeKind.Hub when traces:
                if (_style == TracesMapStyle.Launch)
                {
                    DrawPad(ds, device, centre, s, dim);
                }
                else
                {
                    DrawLaptop(ds, device, centre, s, dim);
                }

                break;
            case MapNodeKind.Hub:
                ds.FillCircle((float)centre.X, (float)centre.Y, r, Ink("statusOff", dim));
                Ring(ds, centre, r, s, Ink("ink", 0.25 * dim));
                break;
            case MapNodeKind.Library:
                ds.FillCircle((float)centre.X, (float)centre.Y, r, Ink(node.Active ? "blue" : "mapNodeOff", dim));
                if (node.Active)
                {
                    Ring(ds, centre, r, s, Ink("blue", 0.35 * dim));
                }

                break;
            case MapNodeKind.Folder:
            {
                (string fill, string stroke) = MapGeometry.Rule(node.Mode);
                ds.FillCircle((float)centre.X, (float)centre.Y, r, Ink(fill, dim));
                ds.DrawCircle((float)centre.X, (float)centre.Y, r, Ink(stroke, dim), (float)(2 * s));
                break;
            }

            case MapNodeKind.Harness:
                ds.FillCircle((float)centre.X, (float)centre.Y, r, Ink("ink", 0.12 * dim));
                if (node.Tool is { } tool)
                {
                    DrawMark(ds, device, tool, centre, 16 * s, dim);
                }

                break;
            case MapNodeKind.Device:
                DrawLaptop(ds, device, new MapPoint(centre.X, centre.Y + 4 * s), s * MapGeometry.DeviceScale, dim);
                break;
            case MapNodeKind.Destination:
            {
                bool lit = node.Lamp is MapLamp.Answering or MapLamp.Running;
                ds.FillCircle((float)centre.X, (float)centre.Y, r, Ink(lit ? "mapCredentialOn" : "mapNodeOff", dim));
                if (node.Lamp == MapLamp.Answering)
                {
                    Ring(ds, centre, r, s, Ink("statusOn", 0.35 * dim));
                }

                if (node.Lamp == MapLamp.Unknown)
                {
                    float rimWidth = (float)(1.5 * s);
                    using var dotted = new CanvasStrokeStyle
                    {
                        StartCap = CanvasCapStyle.Round,
                        EndCap = CanvasCapStyle.Round,
                        DashCap = CanvasCapStyle.Round,
                        CustomDashStyle = new[] { (float)(0.5 * s / rimWidth), (float)(4 * s / rimWidth) },
                    };
                    ds.DrawCircle((float)centre.X, (float)centre.Y, r, Color.FromArgb((byte)Math.Round(255 * 0.5 * dim), 255, 255, 255), rimWidth, dotted);
                }

                DrawKey(ds, device, centre, s, Ink("textPrimary", dim));
                break;
            }
        }

        bool drawnElsewhere = traces && (node.Id == "hub" || node.Id == "library");
        if (selected && !drawnElsewhere)
        {
            ds.DrawCircle((float)centre.X, (float)centre.Y, r, Ink("purpleText"), (float)(2 * s));
        }

        var type = MapGeometry.LabelType(s);
        if (node.LabelBeside)
        {
            string text = node.Kind == MapNodeKind.Folder ? MapGeometry.ShortLabel(node.Label) : node.Label;
            var at = new MapPoint(centre.X - r - 6 * Math.Max(s, MapGeometry.LabelFloor), centre.Y);
            DrawText(ds, text, at, type.LabelSize, Ink("textPrimary", dim), CanvasHorizontalAlignment.Right, bold: true);
        }
        else
        {
            DrawText(ds, node.Label, new MapPoint(centre.X, centre.Y + r + type.LabelOffset), type.LabelSize, Ink("textPrimary", dim), CanvasHorizontalAlignment.Center, bold: true);
        }

        if (node.Sublabel is { } sub)
        {
            DrawText(ds, sub, new MapPoint(centre.X, centre.Y + r + type.SublabelOffset), type.SublabelSize, Ink("statusOff", dim), CanvasHorizontalAlignment.Center, bold: false);
        }
    }

    private void Ring(CanvasDrawingSession ds, MapPoint centre, float radius, double scale, Color colour)
    {
        (double offset, double width) = MapGeometry.RingMetrics(scale);
        ds.DrawCircle((float)centre.X, (float)centre.Y, radius + (float)offset, colour, (float)width);
    }

    /// <summary>#1146's credential key (FlowMapView.swift:488-500).</summary>
    private static void DrawKey(CanvasDrawingSession ds, ICanvasResourceCreator device, MapPoint c, double s, Color colour)
    {
        using var style = new CanvasStrokeStyle { StartCap = CanvasCapStyle.Round, EndCap = CanvasCapStyle.Round, LineJoin = CanvasLineJoin.Round };
        float w = (float)(2 * s);
        ds.DrawCircle((float)(c.X - 9 * s), (float)c.Y, (float)(6 * s), colour, w, style);
        ds.DrawLine((float)(c.X - 3 * s), (float)c.Y, (float)(c.X + 11 * s), (float)c.Y, colour, w, style);
        ds.DrawLine((float)(c.X + 7 * s), (float)c.Y, (float)(c.X + 7 * s), (float)(c.Y + 5 * s), colour, w, style);
        ds.DrawLine((float)(c.X + 11 * s), (float)c.Y, (float)(c.X + 11 * s), (float)(c.Y + 4 * s), colour, w, style);
    }

    /// <summary>This computer as a launch pad (FlowMapView.swift:455-470).</summary>
    private void DrawPad(CanvasDrawingSession ds, ICanvasResourceCreator device, MapPoint c, double s, double dim)
    {
        float half = (float)(FlowMapScene.PadHalfWidth * s);
        Color off = Ink("statusOff", dim);
        using (var slab = new CanvasLinearGradientBrush(device, With(off, 0.75), off)
        {
            StartPoint = new Vector2((float)c.X, (float)(c.Y - 5 * s)),
            EndPoint = new Vector2((float)c.X, (float)(c.Y + 5 * s)),
        })
        {
            ds.FillRoundedRectangle(new Rect(c.X - half, c.Y - 5 * s, half * 2, 10 * s), (float)(4 * s), (float)(4 * s), slab);
        }

        ds.FillRoundedRectangle(new Rect(c.X - 24 * s, c.Y + 5 * s, 48 * s, 6 * s), (float)(3 * s), (float)(3 * s), Ink("mapNodeOff", dim));
        Color steel = Ink("statusOff", 0.7 * dim);
        ds.FillRoundedRectangle(new Rect(c.X + 28 * s, c.Y - 55 * s, 4 * s, 50 * s), (float)(2 * s), (float)(2 * s), steel);
        ds.FillRoundedRectangle(new Rect(c.X + 14 * s, c.Y - 43 * s, 16 * s, 3 * s), (float)(1.5 * s), (float)(1.5 * s), steel);
        ds.FillRoundedRectangle(new Rect(c.X + 18 * s, c.Y - 27 * s, 12 * s, 3 * s), (float)(1.5 * s), (float)(1.5 * s), steel);
    }

    /// <summary>This computer as a laptop on the standard map (FlowMapView.swift:435-453).</summary>
    private void DrawLaptop(CanvasDrawingSession ds, ICanvasResourceCreator device, MapPoint c, double s, double dim)
    {
        double half = FlowMapScene.PadHalfWidth * s;
        Color steel = Ink("statusOff", dim);
        ds.FillRoundedRectangle(new Rect(c.X - half, c.Y - 3 * s, half * 2, 6 * s), (float)(3 * s), (float)(3 * s), steel);
        ds.FillRoundedRectangle(new Rect(c.X - 7 * s, c.Y - 3 * s, 14 * s, 2 * s), (float)s, (float)s, Ink("mapNodeOff", dim));
        double top = c.Y - TransferDrawing.LaptopScreenTop * s;
        var lid = new Rect(c.X - 25 * s, top, 50 * s, (TransferDrawing.LaptopScreenTop - 4) * s);
        ds.FillRoundedRectangle(lid, (float)(3.5 * s), (float)(3.5 * s), With(steel, 0.85));
        var display = new Rect(lid.X + 2.5 * s, lid.Y + 2.5 * s, lid.Width - 5 * s, lid.Height - 5 * s);
        using var screen = new CanvasLinearGradientBrush(device, With(Ink("heroCampaignBright", dim), 0.9), Ink("heroCampaignDeep", dim))
        {
            StartPoint = new Vector2((float)display.X, (float)display.Y),
            EndPoint = new Vector2((float)(display.X + display.Width), (float)(display.Y + display.Height)),
        };
        ds.FillRoundedRectangle(display, (float)(2 * s), (float)(2 * s), screen);
    }

    /// <summary>The station or the orb at the library's place (FlowMapView.swift:236-249).</summary>
    private void DrawLibrary(CanvasDrawingSession ds, ICanvasResourceCreator device, MapNode library, MapGeometry fit, double seconds, bool still)
    {
        MapPoint at = fit.Point(library.At);
        Color brand = Ink("heroCampaignBright");
        if (_style == TracesMapStyle.Launch)
        {
            double w = LibraryStation.FrameWidth * fit.Scale, h = LibraryStation.FrameHeight * fit.Scale;
            double angle = LibraryStation.Angle(seconds, still);
            using var station = new CanvasCommandList(device);
            using (CanvasDrawingSession layer = station.CreateDrawingSession())
            {
                using var seam = new CanvasStrokeStyle { LineJoin = CanvasLineJoin.Round };
                foreach (DrawnFace face in LibraryStation.Frame(w, h, angle))
                {
                    using var builder = new CanvasPathBuilder(device);
                    builder.BeginFigure((float)(at.X - w / 2 + face.Points[0].X), (float)(at.Y - h / 2 + face.Points[0].Y));
                    for (int i = 1; i < face.Points.Count; i++)
                    {
                        builder.AddLine((float)(at.X - w / 2 + face.Points[i].X), (float)(at.Y - h / 2 + face.Points[i].Y));
                    }

                    builder.EndFigure(CanvasFigureLoop.Closed);
                    using CanvasGeometry polygon = CanvasGeometry.CreatePath(builder);
                    Color colour = Color.FromArgb(255, (byte)Math.Round(face.R * 255), (byte)Math.Round(face.G * 255), (byte)Math.Round(face.B * 255));
                    layer.FillGeometry(polygon, colour);
                    layer.DrawGeometry(polygon, colour, 0.6f, seam);
                }
            }

            DrawGlow(ds, station, With(brand, LibraryStation.Glow(library.Active)), LibraryStation.GlowRadius);
            ds.DrawImage(station);
        }
        else
        {
            double frame = TransferDrawing.OrbFrame * fit.Scale;
            double r = LibraryOrb.Radius(frame, frame);
            double angle = LibraryOrb.Angle(seconds, still);
            using var orb = new CanvasCommandList(device);
            using (CanvasDrawingSession layer = orb.CreateDrawingSession())
            {
                DrawOrb(layer, device, at, r, angle);
            }

            DrawGlow(ds, orb, With(brand, LibraryOrb.Glow(library.Active)), LibraryOrb.GlowRadius);
            using (ds.CreateLayer((float)LibraryOrb.Opacity(library.Active)))
            {
                ds.DrawImage(orb);
            }
        }
    }

    /// <summary>A shadow-style glow of <paramref name="radius"/> round an image (SwiftUI's <c>.shadow</c>).</summary>
    private static void DrawGlow(CanvasDrawingSession ds, ICanvasImage image, Color colour, double radius)
    {
        using var shadow = new ShadowEffect { Source = image, BlurAmount = (float)(radius / 2), ShadowColor = colour };
        ds.DrawImage(shadow);
    }

    /// <summary>The orb (LibraryOrb.swift:76-120).</summary>
    private void DrawOrb(CanvasDrawingSession ds, ICanvasResourceCreator device, MapPoint c, double r, double angle)
    {
        Color blue = Ink("heroCampaignBright"), deep = Ink("heroCampaignDeep");
        var light = new Vector2((float)(c.X - r * 0.35), (float)(c.Y - r * 0.4));
        using (var glass = new CanvasRadialGradientBrush(device, new[]
        {
            new CanvasGradientStop { Position = 0, Color = With(blue, 0.95) },
            new CanvasGradientStop { Position = 0.5f, Color = With(deep, 0.95) },
            new CanvasGradientStop { Position = 1, Color = deep },
        })
        {
            Center = light,
            RadiusX = (float)(r * 1.5),
            RadiusY = (float)(r * 1.5),
        })
        {
            ds.FillCircle((float)c.X, (float)c.Y, (float)r, glass);
        }

        using CanvasGeometry sphere = CanvasGeometry.CreateCircle(device, (float)c.X, (float)c.Y, (float)r);
        using (ds.CreateLayer(1, sphere))
        {
            Matrix3x2 saved = ds.Transform;
            ds.Transform = Matrix3x2.CreateRotation((float)LibraryOrb.Lean) * Matrix3x2.CreateTranslation((float)c.X, (float)c.Y) * saved;
            foreach (double latitude in new[] { -0.5, 0.0, 0.5 })
            {
                double y = r * latitude, half = r * Math.Sqrt(1 - latitude * latitude);
                ds.DrawEllipse(0, (float)y, (float)half, (float)(half * 0.22), Color.FromArgb((byte)Math.Round(255 * 0.18), 255, 255, 255), 1);
            }

            foreach ((double width, bool near) in LibraryOrb.MeridianWidths(angle))
            {
                double w = r * width;
                ds.DrawEllipse(0, 0, (float)w, (float)r, Color.FromArgb((byte)Math.Round(255 * (near ? 0.32 : 0.12)), 255, 255, 255), near ? 1.2f : 0.8f);
            }

            ds.Transform = saved;
        }

        double side = r * 0.95;
        CanvasGeometry mark = MarkUnit(device);
        ds.FillGeometry(mark.Transform(Matrix3x2.CreateScale((float)side) * Matrix3x2.CreateTranslation((float)(c.X - side / 2), (float)(c.Y - side / 2))), Color.FromArgb(255, 255, 255, 255));
        using (var shine = new CanvasLinearGradientBrush(device, Color.FromArgb((byte)Math.Round(255 * 0.45), 255, 255, 255), Color.FromArgb(0, 255, 255, 255))
        {
            StartPoint = new Vector2((float)c.X, (float)(c.Y - r * 0.78)),
            EndPoint = new Vector2((float)c.X, (float)(c.Y - r * 0.4)),
        })
        {
            ds.FillEllipse((float)(c.X - r * 0.62 + r * 0.35), (float)(c.Y - r * 0.78 + r * 0.19), (float)(r * 0.35), (float)(r * 0.19), shine);
        }

        ds.DrawCircle((float)c.X, (float)c.Y, (float)r, Color.FromArgb((byte)Math.Round(255 * 0.35), 255, 255, 255), 1);
    }

    // ---- Transfers (MapToolPile.swift:233-276) --------------------------

    private void DrawTransfers(CanvasDrawingSession ds, ICanvasResourceCreator device, MapTransferRun run, double t, MapGeometry fit)
    {
        if (_scene.Stack is not { } stack)
        {
            return;
        }

        double s = fit.Scale;
        Color glow = Ink(TransferDrawing.GlowToken(run.Action));
        MapPoint hub = _scene.Nodes.FirstOrDefault(n => n.Id == "hub")?.At ?? FlowMapScene.Hub(_scene.Space);
        MapPoint library = _scene.Nodes.FirstOrDefault(n => n.Id == "library")?.At ?? FlowMapScene.Library(_scene.Space);
        MapArc? outArc = _scene.Arcs.LastOrDefault(a => a.From == stack.PortRight);
        MapPoint rest = TransferDrawing.RocketRest(hub);
        var screen = new MapPoint(hub.X, hub.Y - TransferDrawing.LaptopScreenTop);
        var orbBase = new MapPoint(library.X, library.Y + TransferDrawing.OrbRadius);
        bool contributing = run.Action == TransferAction.Contribute;
        MapPoint pulseAt = run.Rises ? library : contributing ? rest : hub;
        for (int i = 0; i < run.Transfers.Count; i++)
        {
            Leg leg = run.LegOf(i, t);
            switch (leg.Kind)
            {
                case LegKind.Inbound when TransferDrawing.FolderArc(run.Transfers[i].Folder, _scene) is { } arc:
                    DrawTraceOrb(ds, device, fit.Point(arc.PointAt(TransferDrawing.Ease(leg.Value))), glow, s);
                    break;
                case LegKind.Outbound when outArc is not null:
                    DrawTraceOrb(ds, device, fit.Point(outArc.PointAt(TransferDrawing.Ease(leg.Value))), glow, s);
                    break;
                case LegKind.Ascent:
                    if (leg.Value < 0.3)
                    {
                        DrawPulse(ds, device, fit.Point(screen), leg.Value / 0.3, glow, s * 0.6);
                    }

                    foreach ((MapPoint at, double opacity, double size) in TransferDrawing.Rise(screen, orbBase, leg.Value))
                    {
                        using (ds.CreateLayer((float)opacity))
                        {
                            DrawTraceOrb(ds, device, fit.Point(at), glow, s * size);
                        }
                    }

                    break;
                case LegKind.Arrived when leg.Value < MapTransferRun.Pulse:
                    DrawPulse(ds, device, fit.Point(pulseAt), leg.Value / MapTransferRun.Pulse, glow, s);
                    break;
            }
        }

        if (contributing && run.Style == TracesMapStyle.Launch)
        {
            double? flight = run.FlightAt(t);
            if (flight is >= 1)
            {
                DrawDock(ds, fit.Point(library), t - run.LiftOff - MapTransferRun.Flight, s, run);
            }
            else
            {
                var rocket = TransferDrawing.Rocket(rest, library, flight, t);
                int charge = Math.Min(run.ArrivedBy(t), 4);
                if (flight is { } f && f < 0.6)
                {
                    foreach ((double offset, double radius, double alpha) in TransferDrawing.Smoke(f / 0.6))
                    {
                        MapPoint baseAt = fit.Point(new MapPoint(rest.X, rest.Y + TransferDrawing.RocketLength / 2));
                        var centre = new Vector2((float)(baseAt.X + offset * s), (float)(baseAt.Y - 4 * s));
                        using var puff = new CanvasRadialGradientBrush(device, Color.FromArgb((byte)Math.Round(255 * alpha), 219, 219, 219), Color.FromArgb(0, 168, 168, 168))
                        {
                            Center = centre,
                            RadiusX = (float)(radius * s),
                            RadiusY = (float)(radius * s),
                        };
                        ds.FillCircle(centre, (float)(radius * s), puff);
                    }
                }

                Color glowColour = flight is not null ? FlameOuter : charge > 0 ? Ink("mapOrbShared") : With(FlameOuter, 0.5);
                double glowRadius = flight is not null ? 8 * s : TransferDrawing.PadCharge(charge) * s;
                MapPoint at = fit.Point(rocket.At);
                DrawRocket(ds, device, new Vector2((float)(fit.OriginX + rocket.At.X * s), (float)at.Y), rocket.Angle, s, rocket.Burn, glowRadius, glowColour);
            }
        }

        if (run.Rises)
        {
            DrawDock(ds, fit.Point(library), t - run.AllArrived, s, run);
        }

        if (TransferDrawing.LineUnderPad(run, t, Words) is { } line)
        {
            var type = MapGeometry.LabelType(s);
            MapPoint c = fit.Point(hub);
            DrawText(ds, line, new MapPoint(c.X, c.Y + 16 * s + type.SublabelOffset), type.SublabelSize, Ink("textTertiary"), CanvasHorizontalAlignment.Center, bold: false);
        }
    }

    /// <summary>The flame's colours, the Mac's literals (MapToolPile.swift:416-417).</summary>
    private static readonly Color FlameOuter = Color.FromArgb(255, 255, 179, 92);
    private static readonly Color FlameInner = Color.FromArgb(255, 255, 242, 201);

    private void DrawTraceOrb(CanvasDrawingSession ds, ICanvasResourceCreator device, MapPoint c, Color glow, double s)
    {
        float r = (float)(TransferDrawing.OrbDot * s);
        using (var halo = new CanvasCommandList(device))
        {
            using (CanvasDrawingSession layer = halo.CreateDrawingSession())
            {
                layer.FillCircle((float)c.X, (float)c.Y, r * 2, With(glow, 0.45));
            }

            using var blur = new GaussianBlurEffect { Source = halo, BlurAmount = (float)(6 * s / 2) };
            ds.DrawImage(blur);
        }

        using var core = new CanvasRadialGradientBrush(device, new[]
        {
            new CanvasGradientStop { Position = 0, Color = Color.FromArgb(255, 255, 255, 255) },
            new CanvasGradientStop { Position = 0.5f, Color = With(glow, 0.9) },
            new CanvasGradientStop { Position = 1, Color = With(glow, 0) },
        })
        {
            Center = new Vector2((float)(c.X - r * 0.2), (float)(c.Y - r * 0.25)),
            RadiusX = r * 1.2f,
            RadiusY = r * 1.2f,
        };
        ds.FillCircle((float)c.X, (float)c.Y, r, core);
    }

    private static void DrawPulse(CanvasDrawingSession ds, ICanvasResourceCreator device, MapPoint c, double progress, Color glow, double s)
    {
        (double radius, double alpha) = TransferDrawing.PulseAt(progress);
        float r = (float)(radius * s);
        using var brush = new CanvasRadialGradientBrush(device, new[]
        {
            new CanvasGradientStop { Position = 0, Color = Color.FromArgb((byte)Math.Round(255 * alpha), 255, 255, 255) },
            new CanvasGradientStop { Position = 0.5f, Color = With(glow, alpha * 0.75) },
            new CanvasGradientStop { Position = 1, Color = With(glow, 0) },
        })
        {
            Center = new Vector2((float)c.X, (float)c.Y),
            RadiusX = r,
            RadiusY = r,
        };
        ds.FillCircle((float)c.X, (float)c.Y, r, brush);
    }

    private void DrawDock(CanvasDrawingSession ds, MapPoint c, double since, double s, MapTransferRun run)
    {
        if (TransferDrawing.Dock(since) is { } ring)
        {
            ds.DrawCircle((float)c.X, (float)c.Y, (float)(ring.Radius * s), Ink("blue", ring.Alpha), (float)(3 * s));
        }

        if (TransferDrawing.DockCount(since))
        {
            DrawText(ds, "+" + run.Transfers.Count, new MapPoint(c.X + 40 * s, c.Y - 30 * s), Math.Max(9, 10 * s), Ink("statusOnText"), CanvasHorizontalAlignment.Center, bold: true);
        }
    }

    /// <summary>The rocket, nose along <paramref name="angle"/> (MapToolPile.swift:421-452, shape :514-600).</summary>
    private void DrawRocket(CanvasDrawingSession ds, ICanvasResourceCreator device, MapPoint centre, double angle, double s, double? burn, double glowRadius, Color? glow)
        => DrawRocket(ds, device, new Vector2((float)centre.X, (float)centre.Y), angle, s, burn, glowRadius, glow);

    private void DrawRocket(CanvasDrawingSession ds, ICanvasResourceCreator device, Vector2 centre, double angle, double s, double? burn, double glowRadius, Color? glow)
    {
        double k = TransferDrawing.RocketLength * s / 30;
        Matrix3x2 m = Matrix3x2.CreateTranslation(-15, -9) * Matrix3x2.CreateScale((float)k) * Matrix3x2.CreateRotation((float)angle) * Matrix3x2.CreateTranslation(centre);
        using var art = new CanvasCommandList(device);
        using (CanvasDrawingSession layer = art.CreateDrawingSession())
        {
            layer.Transform = m;
            if (burn is { } b)
            {
                Matrix3x2 flame = Matrix3x2.CreateTranslation(-9, -9) * Matrix3x2.CreateScale((float)b) * Matrix3x2.CreateTranslation(9, 9) * m;
                layer.Transform = flame;
                using CanvasGeometry outer = RocketShape(device, RocketPart.Flame);
                using CanvasGeometry inner = RocketShape(device, RocketPart.FlameCore);
                layer.FillGeometry(outer, FlameOuter);
                layer.FillGeometry(inner, FlameInner);
                layer.Transform = m;
            }

            Color outline = Ink("statusOff"), accent = Ink("purpleSoft");
            using CanvasGeometry nozzle = RocketShape(device, RocketPart.Nozzle);
            using CanvasGeometry fins = RocketShape(device, RocketPart.Fins);
            using CanvasGeometry body = RocketShape(device, RocketPart.Body);
            using CanvasGeometry nose = RocketShape(device, RocketPart.Nose);
            layer.FillGeometry(nozzle, outline);
            layer.DrawGeometry(nozzle, outline, 0.9f);
            layer.FillGeometry(fins, accent);
            layer.DrawGeometry(fins, outline, 0.9f);
            layer.FillGeometry(body, Color.FromArgb(255, 242, 242, 242));
            layer.FillGeometry(nose, accent);
            layer.DrawGeometry(body, outline, 0.9f);
            layer.FillCircle(18.6f, 9, 2, Ink("blue"));
            layer.DrawCircle(18.6f, 9, 2, outline, 0.9f);
        }

        if (glowRadius > 0 && glow is { } g)
        {
            DrawGlow(ds, art, g, glowRadius);
        }

        ds.DrawImage(art);
    }

    private enum RocketPart
    {
        Body,
        Nose,
        Fins,
        Nozzle,
        Flame,
        FlameCore,
    }

    /// <summary>The rocket's paths in its own 30 by 18 units, nose right (MapToolPile.swift:530-600).</summary>
    private static CanvasGeometry RocketShape(ICanvasResourceCreator device, RocketPart part)
    {
        using var p = new CanvasPathBuilder(device);
        Vector2 V(double x, double y) => new((float)x, (float)y);
        switch (part)
        {
            case RocketPart.Body:
                p.BeginFigure(V(10, 5.6));
                p.AddLine(V(20, 5.6));
                p.AddCubicBezier(V(24.5, 5.6), V(28, 7), V(29.5, 9));
                p.AddCubicBezier(V(28, 11), V(24.5, 12.4), V(20, 12.4));
                p.AddLine(V(10, 12.4));
                p.EndFigure(CanvasFigureLoop.Closed);
                break;
            case RocketPart.Nose:
                p.BeginFigure(V(23.5, 6.1));
                p.AddCubicBezier(V(26, 6.6), V(28.3, 7.6), V(29.5, 9));
                p.AddCubicBezier(V(28.3, 10.4), V(26, 11.4), V(23.5, 11.9));
                p.AddQuadraticBezier(V(24.4, 9), V(23.5, 6.1));
                p.EndFigure(CanvasFigureLoop.Closed);
                break;
            case RocketPart.Fins:
                p.BeginFigure(V(17, 5.6));
                p.AddCubicBezier(V(15, 3.2), V(12.5, 1.2), V(10.5, 0.8));
                p.AddCubicBezier(V(9.4, 0.6), V(9.6, 3.5), V(10, 5.6));
                p.EndFigure(CanvasFigureLoop.Closed);
                p.BeginFigure(V(17, 12.4));
                p.AddCubicBezier(V(15, 14.8), V(12.5, 16.8), V(10.5, 17.2));
                p.AddCubicBezier(V(9.4, 17.4), V(9.6, 14.5), V(10, 12.4));
                p.EndFigure(CanvasFigureLoop.Closed);
                break;
            case RocketPart.Nozzle:
                p.BeginFigure(V(10.4, 7.2));
                p.AddLine(V(8, 6.4));
                p.AddLine(V(8, 11.6));
                p.AddLine(V(10.4, 10.8));
                p.EndFigure(CanvasFigureLoop.Closed);
                break;
            case RocketPart.Flame:
                p.BeginFigure(V(0, 9));
                p.AddCubicBezier(V(4, 5.5), V(7, 6.5), V(9, 9));
                p.AddCubicBezier(V(7, 11.5), V(4, 12.5), V(0, 9));
                p.EndFigure(CanvasFigureLoop.Closed);
                break;
            default:
                p.BeginFigure(V(3, 9));
                p.AddCubicBezier(V(5, 7.5), V(7, 8), V(9, 9));
                p.AddCubicBezier(V(7, 10), V(5, 10.5), V(3, 9));
                p.EndFigure(CanvasFigureLoop.Closed);
                break;
        }

        return CanvasGeometry.CreatePath(p);
    }

    // ---- The tool pile (MapToolPile.swift:36-200) -----------------------

    private void DrawPile(CanvasDrawingSession ds, ICanvasResourceCreator device, MapToolStack stack, MapGeometry fit, MapTransferRun? run, double t, double now, bool motion)
    {
        double s = fit.Scale;
        bool moving = run?.Moving(t) ?? false;
        (double tall, double wide, double names) = _pile.Stages(now, !motion);
        bool open = _pile.Open(moving);
        IReadOnlyList<string> order = run?.Order(stack.Tools, t) ?? stack.Tools;
        IReadOnlyDictionary<string, double> orbits = run?.Orbits(t) ?? new Dictionary<string, double>();
        double height = Lerp(stack.Height(false), stack.Height(true), moving ? 0 : tall) * s;
        double width = Lerp(MapToolStack.Width, MapToolStack.OpenWidth, moving ? 0 : wide) * s;
        MapPoint left = fit.Point(stack.PortLeft);
        Color fill = open && wide > 0.5 ? Ink("paneOpaque") : Ink("wellFill");
        ds.FillRoundedRectangle(new Rect(left.X, left.Y - height / 2, width, height), (float)(26 * s), (float)(26 * s), fill);
        if (Words?.ToolsHeading is { Length: > 0 } heading)
        {
            DrawText(ds, heading.ToUpperInvariant(), new MapPoint(left.X + 30 * s, left.Y - height / 2 - 12 * s), Math.Max(9, 10 * s), Ink("textTertiary"), CanvasHorizontalAlignment.Center, bold: true);
        }

        // Back of the pile first, so the front coin is drawn last; a grown coin over all.
        var drawn = new List<(MapNode Node, int Index, double Place, double Opening, double Scale, bool Grown)>();
        for (int index = 0; index < order.Count; index++)
        {
            MapNode? node = _scene.Nodes.FirstOrDefault(n => n.Id == order[index]);
            if (node is null)
            {
                continue;
            }

            bool grown = orbits.ContainsKey(node.Id);
            (double place, double opening, double scale) = _pile.Coin(node.Id, index, open, grown, now, !motion);
            drawn.Add((node, index, place, opening, scale, grown));
        }

        foreach (var coin in drawn.OrderBy(c => c.Grown ? 1 : 0).ThenByDescending(c => c.Index))
        {
            MapPoint closed = stack.CoinCenter(0, false), opened = stack.CoinCenter(0, true);
            double y = Lerp(closed.Y + coin.Place * MapToolStack.Step, opened.Y + coin.Place * MapToolStack.OpenStep, coin.Opening);
            MapPoint centre = fit.Point(new MapPoint(stack.Center.X, y));
            DrawCoin(ds, device, coin.Node, centre, s, coin.Scale, coin.Grown, run?.Action, open || coin.Index == 0);
            if (names > 0)
            {
                string? line = coin.Node.Dashed ? Words?.LegendNotWatched : coin.Node.Waiting is { } waiting ? Words?.CoinWaiting(waiting) : null;
                double shown = (coin.Node.Dashed ? 0.55 : 1) * names;
                DrawText(ds, coin.Node.Label, new MapPoint(centre.X + 32 * s, centre.Y - (line is null ? 0 : 6 * s)), Math.Max(9.5, 11 * s), Ink("textPrimary", shown), CanvasHorizontalAlignment.Left, bold: true);
                if (line is not null)
                {
                    DrawText(ds, line, new MapPoint(centre.X + 32 * s, centre.Y + 7 * s), Math.Max(9, 10 * s), Ink("textTertiary", shown), CanvasHorizontalAlignment.Left, bold: false);
                }
            }
        }

        // The rim light, once round each coin a trace is in (:276-292).
        if (run is not null)
        {
            Color glow = Ink(TransferDrawing.GlowToken(run.Action));
            foreach ((string id, double p) in orbits)
            {
                int index = order.ToList().IndexOf(id);
                if (index >= 0)
                {
                    DrawTraceOrb(ds, device, fit.Point(TransferDrawing.RimLight(stack.CoinCenter(index, false), p)), glow, s * 0.75);
                }
            }
        }
    }

    private void DrawCoin(CanvasDrawingSession ds, ICanvasResourceCreator device, MapNode node, MapPoint c, double s, double scale, bool grown, TransferAction? action, bool carriesLock)
    {
        GlassToolArt tool = node.Tool ?? GlassToolArt.Other("?");
        double opacity = node.Dashed ? 0.55 : node.Dim;
        float r = (float)(MapToolStack.Coin / 2 * s * scale);
        using var coin = new CanvasCommandList(device);
        using (CanvasDrawingSession layer = coin.CreateDrawingSession())
        {
            Color tint = Ink(tool.TintToken);
            layer.FillCircle((float)c.X, (float)(c.Y + 3 * s * scale), r, With(tint, 0.45));
            layer.FillCircle((float)c.X, (float)c.Y, r, Ink("paneOpaque"));
            layer.FillCircle((float)c.X, (float)c.Y, r, Ink("ink", 0.12));
            float rim = (float)(2 * s * scale);
            using var dash = new CanvasStrokeStyle();
            if (node.Dashed)
            {
                dash.CustomDashStyle = new[] { 3f / 2, 3f / 2 };
            }

            layer.DrawCircle((float)c.X, (float)c.Y, r - rim / 2, tint, rim, dash);
            DrawMark(layer, device, tool, c, 20 * s * scale);
            if (node.Off)
            {
                double side = MapToolStack.Coin * s * scale;
                layer.DrawLine((float)(c.X - side * 0.25), (float)(c.Y - side * 0.25), (float)(c.X + side * 0.25), (float)(c.Y + side * 0.25), Ink("textPrimary"), (float)(2 * s * scale));
            }

            if (!node.Dashed && carriesLock)
            {
                // The lock: the tool's memory stays on this computer (:96-108).
                var pip = new Vector2((float)(c.X + MapToolStack.Coin / 2 * s * scale + 1 * s), (float)c.Y);
                layer.FillCircle(pip, (float)(7.5 * s), Ink("paneOpaque"));
                DrawText(layer, "", new MapPoint(pip.X, pip.Y), 6.5 * s, Ink("textSecondary"), CanvasHorizontalAlignment.Center, bold: false, icons: true);
            }
        }

        if (grown)
        {
            DrawGlow(ds, coin, With(Ink(TransferDrawing.GlowToken(action)), 0.45), MapMotion.CoinGlowRadius * s);
        }

        using (ds.CreateLayer((float)opacity))
        {
            ds.DrawImage(coin);
        }
    }

    // ---- Text ---------------------------------------------------------

    private static void DrawText(CanvasDrawingSession ds, string text, MapPoint at, double size, Color colour, CanvasHorizontalAlignment align, bool bold, bool mono = false, bool icons = false)
    {
        if (string.IsNullOrEmpty(text))
        {
            return;
        }

        using var format = new CanvasTextFormat
        {
            FontFamily = icons ? "Segoe Fluent Icons, Segoe MDL2 Assets" : mono ? "Cascadia Mono, Consolas" : "Segoe UI Variable Text, Segoe UI",
            FontSize = (float)size,
            FontWeight = bold ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal,
            WordWrapping = CanvasWordWrapping.NoWrap,
            HorizontalAlignment = CanvasHorizontalAlignment.Left,
            VerticalAlignment = CanvasVerticalAlignment.Center,
        };
        using var layout = new CanvasTextLayout(ds, text, format, 0, 0);
        double width = layout.LayoutBounds.Width;
        double x = align switch
        {
            CanvasHorizontalAlignment.Right => at.X - width,
            CanvasHorizontalAlignment.Center => at.X - width / 2,
            _ => at.X,
        };
        ds.DrawTextLayout(layout, (float)x, (float)at.Y, colour);
    }
}
