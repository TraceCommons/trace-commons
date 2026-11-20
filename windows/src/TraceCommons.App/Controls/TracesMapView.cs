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
using TraceCommons.App.Look;

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
/// dashes at 30 frames a second, the station and orb at 15, transfers and
/// the pile at 60, the shimmer and the pointer light on their own curves.
/// One <see cref="DispatcherQueueTimer"/> drives the moving layer at the
/// fastest rate it needs and turns the library's own layer when its frame
/// changes (#2268); a second drives the library alone while the first is
/// slower than it. Each runs only while the control is loaded, the window can
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

    /// <summary>The still layer under the moving one: the field and the nodes, drawn on change (FlowMapView.swift:91-96).</summary>
    private readonly CanvasControl _still = new();

    /// <summary>
    /// The library's own layer over the moving one (#2268): drawn again only
    /// when the station's 15-a-second frame changes, not at the dashes' 30
    /// (<see cref="TurnLibrary"/>). While a run plays or the pile is on the
    /// moving layer, the library is drawn there instead, under the run and
    /// the pile as before.
    /// </summary>
    private readonly CanvasControl _libraryCanvas = new();
    private readonly Canvas _targets = new();
    private readonly Border _card = new();
    private readonly TextBlock _cardTitle = new();
    private readonly TextBlock _cardDetail = new();
    private readonly StackPanel _foot = new();
    private readonly FlowPanel _legend = new() { Spacing = 10, LineSpacing = 4 };
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

    /// <summary>The transfer cards changed: one arrived, closed or passed its hold.</summary>
    public event Action? TransfersChanged;

    /// <summary>
    /// The map's foot above the legend and zoom, where the transfer cards
    /// ride while another tab than Traces is shown (FlowMapView.swift:166-187).
    /// The window fills it.
    /// </summary>
    public Grid TransferFoot { get; } = new() { MaxWidth = MapGeometry.TransferCardWidth, HorizontalAlignment = HorizontalAlignment.Center };

    /// <summary>The transfer cards, oldest first; the last is in front (<c>MapTransferPlayer.cards</c>).</summary>
    public IReadOnlyList<MapTransfer> TransferCards => _player.Cards;

    /// <summary>Whether the map shows the Traces view, the only one that carries the cards (FlowMapView.swift:176).</summary>
    public bool ShowsTraces => _view == MapView.Traces;

    /// <summary>A card's X: that card alone goes (MapTransfers.swift:280-283).</summary>
    public void CloseTransfer(string id) => _player.Close(id);

    /// <summary>The folder's name on the map, for a transfer that carries none (FlowMapView.swift:180-182).</summary>
    public string? FolderLabel(string folder) =>
        _scene.Nodes.FirstOrDefault(n => n.Id == "folder:" + folder)?.Label;

    private DispatcherQueueTimer? _holdTimer;

    /// <summary>
    /// Wakes once at the player's next deadline, so a card passes its hold
    /// while the map's own clock is stopped (pane closed, motion off); a
    /// one-shot per deadline, never a frame clock.
    /// </summary>
    private void ScheduleHold()
    {
        _holdTimer?.Stop();
        if (_player.NextDeadline is not { } deadline)
        {
            return;
        }

        _holdTimer ??= CreateHoldTimer();
        _holdTimer.Interval = TimeSpan.FromSeconds(Math.Max(deadline - Now, MapMotion.TransferFrameInterval));
        _holdTimer.Start();
    }

    private DispatcherQueueTimer CreateHoldTimer()
    {
        DispatcherQueueTimer timer = DispatcherQueue.CreateTimer();
        timer.IsRepeating = false;
        timer.Tick += (_, _) =>
        {
            if (!_player.Tick(Now))
            {
                ScheduleHold();
            }
        };
        return timer;
    }

    public TracesMapView()
    {
        IsTabStop = false;
        _canvas.Draw += OnDraw;
        _canvas.CreateResources += (_, _) => DropResources();
        _canvas.IsHitTestVisible = false;
        AutomationProperties.SetAccessibilityView(_canvas, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        _libraryCanvas.Draw += OnDrawLibrary;
        _libraryCanvas.CreateResources += (_, _) => DropResources();
        _libraryCanvas.IsHitTestVisible = false;
        AutomationProperties.SetAccessibilityView(_libraryCanvas, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        _still.Draw += OnDrawStill;
        _still.CreateResources += (_, _) => DropResources();
        _still.IsHitTestVisible = false;
        AutomationProperties.SetAccessibilityView(_still, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);

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
        _legend.VerticalAlignment = VerticalAlignment.Bottom;
        zoom.VerticalAlignment = VerticalAlignment.Bottom;
        footRow.Children.Add(_legend);
        Grid.SetColumn(zoom, 1);
        footRow.Children.Add(zoom);
        _foot.Spacing = 12;
        _foot.Children.Add(TransferFoot);
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

        _root.Children.Add(_still);
        _root.Children.Add(_canvas);
        _root.Children.Add(_libraryCanvas);
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

        _player.Changed += () => { Rebuild(); Kick(); ScheduleHold(); TransfersChanged?.Invoke(); };
        // Reduce motion followed live where the system reports changes
        // (Windows 10 2004 and later); elsewhere on activation and on every tick.
        if (OperatingSystem.IsWindowsVersionAtLeast(10, 0, 19041))
        {
            _uiSettings.AnimationsEnabledChanged += (_, _) => DispatcherQueue.TryEnqueue(Recheck);
        }
        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
        ActualThemeChanged += (_, _) => { DropResources(); Redraw(); };
        // A live theme or high contrast switch too (ThemeBrushes).
        ThemeBrushes.RedrawOnChange(this, () => { DropResources(); Redraw(); });
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
                SnapshotRead?.Invoke(snapshot);
            }
            while (_readAgain);
        }
        finally
        {
            _reading = false;
        }
    }

    /// <summary>
    /// Each read as it lands: the Traces graph draws from the same History
    /// page and queue (MonitorWindowView.swift:403-406 reads HomeStore's
    /// history and the tree the map draws).
    /// </summary>
    public event Action<MapSnapshot>? SnapshotRead;

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
        TransfersChanged?.Invoke();
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
        var text = new TextBlock { Text = word, FontSize = 11, Foreground = new SolidColorBrush(Ink("textTertiary")), VerticalAlignment = VerticalAlignment.Center };
        item.Children.Add(new Border { Width = 10, Height = 10, Child = mark });
        item.Children.Add(text);
        // The legend is hidden from assistive tech, as the Mac's is (FlowMapView.swift:191).
        foreach (UIElement element in new UIElement[] { item, mark, text })
        {
            AutomationProperties.SetAccessibilityView(element, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        }

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
        // The player's runs end on its own clock even while nothing moves on
        // screen (MapTransfers.swift:243-246: under Reduce Motion the runs
        // still play).
        _player.Tick(Now);
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

    private readonly Dictionary<string, (Button Pill, TextBlock Text)> _reveals = new(StringComparer.Ordinal);
    private readonly Dictionary<string, MapFolderReveal> _revealNow = new(StringComparer.Ordinal);

    /// <summary>
    /// The folder pill (FlowMapView.swift:630-669): "+N", or the core's
    /// fold-back word. Each pill is kept across rebuilds, so pressing it
    /// keeps the keyboard on it.
    /// </summary>
    private void PlaceReveals(MapGeometry fit)
    {
        var keep = new HashSet<string>(StringComparer.Ordinal);
        foreach (MapFolderReveal reveal in _scene.Reveals)
        {
            bool folds = reveal.Hidden == 0;
            string? text = folds ? Words?.FoldFolders : "+" + reveal.Hidden;
            if (text is null)
            {
                continue;
            }

            keep.Add(reveal.Tool);
            _revealNow[reveal.Tool] = reveal;
            if (!_reveals.TryGetValue(reveal.Tool, out var made))
            {
                var label = new TextBlock { FontWeight = Microsoft.UI.Text.FontWeights.SemiBold };
                var button = new Button { Content = label };
                string key = reveal.Tool;
                button.Click += (_, _) =>
                {
                    if (_revealNow.TryGetValue(key, out MapFolderReveal? current))
                    {
                        _revealed = new Dictionary<string, int>(FlowMapScene.Revealing(current, _revealed), StringComparer.Ordinal);
                        Rebuild();
                    }
                };
                button.SizeChanged += (_, _) => PositionReveal(key);
                made = (button, label);
                _reveals[key] = made;
                _targets.Children.Add(button);
            }

            string name = (folds ? Words?.FoldFoldersLabel : Words?.RevealFolders(reveal.Next)) ?? text;
            double unit = Math.Max(fit.Scale, MapGeometry.LabelFloor);
            double size = MapGeometry.LabelType(fit.Scale).LabelSize;
            made.Text.Text = text;
            made.Text.FontSize = size;
            made.Pill.Padding = new Thickness(8 * unit, 0, 8 * unit, 0);
            made.Pill.Height = size + 8 * unit;
            made.Pill.CornerRadius = new CornerRadius((size + 8 * unit) / 2);
            made.Pill.Opacity = reveal.Dim;
            AutomationProperties.SetName(made.Pill, name);
            ToolTipService.SetToolTip(made.Pill, name);
            PositionReveal(reveal.Tool);
        }

        foreach (string gone in _reveals.Keys.Where(k => !keep.Contains(k)).ToList())
        {
            _targets.Children.Remove(_reveals[gone].Pill);
            _reveals.Remove(gone);
            _revealNow.Remove(gone);
        }
    }

    /// <summary>The pill's trailing edge where the folder names end, on its line.</summary>
    private void PositionReveal(string key)
    {
        if (!_reveals.TryGetValue(key, out var made) || !_revealNow.TryGetValue(key, out MapFolderReveal? reveal))
        {
            return;
        }

        MapGeometry fit = Fit;
        double unit = Math.Max(fit.Scale, MapGeometry.LabelFloor);
        MapPoint column = fit.Point(reveal.At);
        double end = column.X - FlowMapScene.FolderRadius * fit.Scale - 6 * unit;
        Canvas.SetLeft(made.Pill, end - made.Pill.ActualWidth);
        Canvas.SetTop(made.Pill, column.Y - made.Pill.Height / 2);
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
#if DEBUG
        StartDemoIfAsked();
#endif
        Rebuild();
    }

#if DEBUG
    private DispatcherQueueTimer? _demo;
    private int _demoRound;

    /// <summary>
    /// The Mac's dry-run review aid (MapTransferFeed.swift:47-77), Debug
    /// builds only: with <c>TRACE_COMMONS_MAP_DEMO=1</c> a run plays every
    /// 12 seconds from the folders the map draws -- two contributions, a
    /// keep, then a dismissal -- so the motion can be looked at without
    /// anything leaving this computer. Nothing is sent, kept or dismissed.
    /// </summary>
    private void StartDemoIfAsked()
    {
        if (_demo is not null || Environment.GetEnvironmentVariable("TRACE_COMMONS_MAP_DEMO") != "1")
        {
            return;
        }

        _demo = DispatcherQueue.CreateTimer();
        _demo.Interval = TimeSpan.FromSeconds(12);
        _demo.Tick += (_, _) => DemoRound();
        _demo.Start();
        DispatcherQueue.TryEnqueue(DemoRound);
    }

    private void DemoRound()
    {
        if (!_loaded || _snapshot is not { } snapshot)
        {
            return;
        }

        TracesTree tree = snapshot.Tree(MapFeed.CoreScansWhenUnset);
        var pairs = FlowMapScene.Directories(tree).Take(FlowMapScene.FoldersShown)
            .Select(f => (Folder: f, Tool: FlowMapScene.Carriers(f, tree).FirstOrDefault()))
            .Where(p => p.Tool is not null).ToList();
        if (pairs.Count == 0)
        {
            return;
        }

        TransferAction action = (_demoRound % 4) switch
        {
            2 => TransferAction.Keep,
            3 => TransferAction.Dismiss,
            _ => TransferAction.Contribute,
        };
        var picked = action == TransferAction.Contribute
            ? pairs.Skip(_demoRound % pairs.Count).Concat(pairs).Take(Math.Min(3, pairs.Count)).ToList()
            : new() { pairs[_demoRound % pairs.Count] };
        var transfers = picked.Select(p => new MapTransfer($"demo-{_demoRound}-{p.Folder.Id}", p.Folder.Id, p.Tool!.Id, action)
        {
            At = DateTimeOffset.Now,
            FolderLabel = p.Folder.Label,
        }).ToList();
        if (action == TransferAction.Dismiss)
        {
            _player.Announce(transfers[0], Now);
        }
        else
        {
            _player.Enqueue(transfers, Now);
        }

        _demoRound++;
    }
#endif

    private void OnUnloaded(object sender, RoutedEventArgs e)
    {
        _loaded = false;
#if DEBUG
        _demo?.Stop();
        _demo = null;
#endif
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
    /// <summary>Whether the map's pane is open: the window collapses the pane, not this control.</summary>
    public bool PaneShown
    {
        get => _paneShown;
        set
        {
            if (_paneShown == value)
            {
                return;
            }

            _paneShown = value;
            if (value)
            {
                _shownAt = Now;
                Rebuild();
            }
            else
            {
                StopTimer();
            }
        }
    }

    private bool _paneShown = true;

    /// <summary>The system's motion setting or the window's state may have changed: decide again.</summary>
    public void Recheck()
    {
        _player.Tick(Now);
        Redraw();
        Kick();
    }

    private double? WantedInterval() => MapMotion.ClockInterval(ClockInputs());

    /// <summary>What the library's own clock needs (#2268), or null.</summary>
    private double? LibraryInterval() => MapMotion.LibraryInterval(ClockInputs());

    private MapClockInputs ClockInputs()
    {
        double now = Now;
        return new MapClockInputs(
            AnimationsEnabled: Motion,
            Loaded: _loaded,
            WindowVisible: WindowClocks.Running,
            PaneShown: _paneShown && Visibility == Visibility.Visible,
            HasLibrary: _scene.Stack is not null && _scene.Nodes.Any(n => n.Kind == MapNodeKind.Library),
            Flows: _scene.Flows,
            Healthy: Healthy,
            TransferPlaying: _scene.Stack is not null && _player.Run is { } run && now - run.Start <= run.End,
            PileMoving: _pile.Moving(now),
            LightFading: _pointerChangedAt >= 0 && now - _pointerChangedAt < MapMotion.PointerLightFade.Duration,
            SecondsShown: now - _shownAt,
            NextDeadlineIn: _player.NextDeadline is { } deadline ? deadline - now : null,
            LibraryOnMovingLayer: LibraryOnMovingLayer(now));
    }

    private void Kick()
    {
        KickLibrary();

        // The window's rule on top of the map's own: at most 15 a second
        // while the window is not focused (FocusFrameRule).
        if (WantedInterval() is not { } own || WindowClocks.Windows.Interval(XamlRoot, own) is not { } interval)
        {
            _timer?.Stop();
            return;
        }

        _timer ??= CreateTimer();
        TimeSpan wanted = TimeSpan.FromSeconds(Math.Max(interval, MapMotion.TransferFrameInterval));
        if (MapMotion.ClockRestarts(_timer.IsRunning, _timer.Interval.TotalSeconds, wanted.TotalSeconds))
        {
            // A running timer keeps the wait it started with: start it again on the new one.
            _timer.Stop();
            _timer.Interval = wanted;
            _timer.Start();
        }
    }

    private DispatcherQueueTimer CreateTimer()
    {
        DispatcherQueueTimer timer = DispatcherQueue.CreateTimer();
        timer.IsRepeating = true;
        timer.Tick += (_, _) =>
        {
            double now = Now;
            _player.Tick(now);
            PlaceMovingCanvas();
            _canvas.Invalidate();
            TurnLibrary();
            bool fieldMoves = MapField.Sweep(now - _shownAt, false) > 0 || MapField.UntilNextPass(now - _shownAt) == 0
                || (_pointerChangedAt >= 0 && now - _pointerChangedAt < MapMotion.PointerLightFade.Duration + MapMotion.TransferFrameInterval);
            // The pile moves between the layers as it starts and stops.
            if (_stillLayer.Tick(_scene, fieldMoves, PileMoves(now)))
            {
                _still.Invalidate();
            }

            Kick();
        };
        return timer;
    }

    /// <summary>The library's own clock while the moving one is slower (#2268), under the same window rule.</summary>
    private void KickLibrary()
    {
        if (LibraryInterval() is not { } library || WindowClocks.Windows.Interval(XamlRoot, library) is not { } interval)
        {
            _libraryTimer?.Stop();
            return;
        }

        _libraryTimer ??= CreateLibraryTimer();
        TimeSpan wanted = TimeSpan.FromSeconds(interval);
        if (MapMotion.ClockRestarts(_libraryTimer.IsRunning, _libraryTimer.Interval.TotalSeconds, wanted.TotalSeconds))
        {
            _libraryTimer.Stop();
            _libraryTimer.Interval = wanted;
            _libraryTimer.Start();
        }
    }

    private DispatcherQueueTimer? _libraryTimer;

    private DispatcherQueueTimer CreateLibraryTimer()
    {
        DispatcherQueueTimer timer = DispatcherQueue.CreateTimer();
        timer.IsRepeating = true;
        timer.Tick += (_, _) => TurnLibrary();
        return timer;
    }

    /// <summary>The library's frame last asked for on a tick.</summary>
    private int _libraryIndex = -1;

    /// <summary>
    /// Draws the library's layer again when the station's (or orb's) frame of
    /// its turn has changed, at most 15 a second, from whichever clock ticks
    /// (#2268: not at the dashes' rate). The moving layer has it while a run
    /// plays or the pile is on that layer.
    /// </summary>
    private void TurnLibrary()
    {
        double turn = _style == TracesMapStyle.Launch ? LibraryStation.TurnSeconds : LibraryOrb.TurnSeconds;
        int index = MapMotion.LibraryFrameIndex(LibrarySeconds(true), turn);
        if (index != _libraryIndex)
        {
            _libraryIndex = index;
            _libraryCanvas.Invalidate();
        }
    }

    private void StopTimer()
    {
        _timer?.Stop();
        _libraryTimer?.Stop();
    }

    private void Redraw()
    {
        _still.Invalidate();
        PlaceMovingCanvas();
        _canvas.Invalidate();
        _libraryCanvas.Invalidate();
    }

    /// <summary>The moving layer's box (MovingLayerBounds), or null while it takes the whole pane.</summary>
    private LayoutRect? _movingBox;

    /// <summary>
    /// Sizes the moving layer's canvas to the box its resting parts draw in,
    /// or back to the whole pane while a run plays or the pile moves, so a
    /// frame clears and composes only those pixels (MovingLayerBounds).
    /// <see cref="OnDraw"/> draws through the canvas's own offset, so the
    /// parts land where they would on a whole-pane canvas.
    /// </summary>
    private void PlaceMovingCanvas()
    {
        double now = Now;
        PlaceLibraryCanvas(now);
        LayoutRect? box = MovingBox(now);
        if (box == _movingBox)
        {
            return;
        }

        _movingBox = box;
        Place(_canvas, box);
    }

    private static void Place(CanvasControl canvas, LayoutRect? box)
    {
        if (box is { } b)
        {
            canvas.HorizontalAlignment = HorizontalAlignment.Left;
            canvas.VerticalAlignment = VerticalAlignment.Top;
            canvas.Margin = new Thickness(b.X, b.Y, 0, 0);
            canvas.Width = b.Width;
            canvas.Height = b.Height;
        }
        else
        {
            canvas.HorizontalAlignment = HorizontalAlignment.Stretch;
            canvas.VerticalAlignment = VerticalAlignment.Stretch;
            canvas.Margin = default;
            canvas.Width = double.NaN;
            canvas.Height = double.NaN;
        }
    }

    /// <summary>The library layer's box (<see cref="LibraryBox"/>) and whether the library is on the moving layer, as last placed.</summary>
    private LayoutRect? _libraryBox;
    private bool? _libraryOnMoving;

    /// <summary>
    /// Sizes the library's layer to the library's frame and glow, and hides
    /// it while the library is drawn on the moving layer; the layer that
    /// takes the library over is drawn again at once.
    /// </summary>
    private void PlaceLibraryCanvas(double now)
    {
        bool onMoving = LibraryOnMovingLayer(now);
        if (onMoving != _libraryOnMoving)
        {
            _libraryOnMoving = onMoving;
            _libraryCanvas.Visibility = onMoving ? Visibility.Collapsed : Visibility.Visible;
            _libraryCanvas.Invalidate();
            _canvas.Invalidate();
        }

        LayoutRect? box = LibraryBox();
        if (box != _libraryBox)
        {
            _libraryBox = box;
            Place(_libraryCanvas, box);
        }
    }

    /// <summary>
    /// Whether the library is drawn on the moving layer: while that layer
    /// takes the whole pane, for a run or a pile on it, so the run and the
    /// pile still draw over the library. Otherwise it has its own layer.
    /// </summary>
    private bool LibraryOnMovingLayer(double now) =>
        (Motion && WindowClocks.Running && _player.Run is not null) || PileMoves(now);

    /// <summary>
    /// Every part <see cref="OnDraw"/> draws while the layer rests, in pane
    /// DIPs: each arc with its widest stroke, the library's frame with its
    /// glow, the rocket at its rest. Null (the whole pane) while a transfer
    /// plays or the pile is on this layer, since those reach anywhere.
    /// </summary>
    private LayoutRect? MovingBox(double now)
    {
        if (!_scene.DrawsMap)
        {
            return null;
        }

        bool whole = LibraryOnMovingLayer(now);
        MapGeometry fit = Fit;
        var parts = new List<LayoutRect>();
        double half = MapGeometry.ArcWidth * MapGeometry.FlowingWidthFactor * fit.Scale / 2;
        foreach (MapArc arc in _scene.Arcs)
        {
            parts.Add(MovingLayerBounds.Curve(fit.Point(arc.From), fit.Point(arc.Control1), fit.Point(arc.Control2), fit.Point(arc.To), half));
        }

        if (_scene.Stack is not null)
        {
            bool launch = _style == TracesMapStyle.Launch;
            if (launch && _scene.Nodes.FirstOrDefault(n => n.Id == "hub") is { } hub)
            {
                double reach = 2 * TransferDrawing.RocketLength * fit.Scale;
                parts.Add(MovingLayerBounds.Around(fit.Point(TransferDrawing.RocketRest(hub.At)), reach, reach));
            }
        }

        return MovingLayerBounds.Resting(PaneSize, XamlRoot?.RasterizationScale ?? 1, whole, parts);
    }

    /// <summary>The library's frame with its glow, in pane DIPs, on whole device pixels; null (the whole pane) with no library.</summary>
    private LayoutRect? LibraryBox()
    {
        if (!_scene.DrawsMap || _scene.Stack is null || _scene.Nodes.FirstOrDefault(n => n.Id == "library") is not { } library)
        {
            return null;
        }

        MapGeometry fit = Fit;
        bool launch = _style == TracesMapStyle.Launch;
        double w = (launch ? LibraryStation.FrameWidth : TransferDrawing.OrbFrame) * fit.Scale;
        double h = (launch ? LibraryStation.FrameHeight : TransferDrawing.OrbFrame) * fit.Scale;
        double pad = (launch ? LibraryStation.GlowRadius : LibraryOrb.GlowRadius) * 2;
        return MovingLayerBounds.Resting(PaneSize, XamlRoot?.RasterizationScale ?? 1, false,
            new[] { MovingLayerBounds.Around(fit.Point(library.At), w + 2 * pad, h + 2 * pad) });
    }

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

    private bool Dark => ThemeBrushes.Dark;

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
        DropFieldLayers();
        _logos.Clear();
        _libraryFrames.Clear();
        _stationGlow.Clear();
    }

    private void OnDraw(CanvasControl sender, CanvasDrawEventArgs args)
    {
        long begun = LookFrames.Begin();
        try
        {
            DrawMoving(sender, args);
        }
        finally
        {
            LookFrames.End("map", begun);
        }
    }

    private void DrawMoving(CanvasControl sender, CanvasDrawEventArgs args)
    {
        CanvasDrawingSession ds = args.DrawingSession;
        double now = Now;
        bool motion = Motion && WindowClocks.Running;
        var size = new Size(sender.ActualWidth, sender.ActualHeight);
        if (size.Width <= 0 || size.Height <= 0)
        {
            return;
        }

        if (!_scene.DrawsMap)
        {
            return;
        }

        // The canvas may be sized to the moving parts' box (PlaceMovingCanvas):
        // draw through its offset in the pane, read from where layout put it.
        Vector3 offset = sender.ActualOffset;
        ds.Transform = Matrix3x2.CreateTranslation(-offset.X, -offset.Y);
        MapGeometry fit = Fit;
        DrawArcs(ds, sender, fit, motion ? now : 0, motion);
        if (_scene.Stack is { } stack)
        {
            if (LibraryOnMovingLayer(now) && _scene.Nodes.FirstOrDefault(n => n.Id == "library") is { } library)
            {
                DrawLibrary(ds, sender, library, fit, LibrarySeconds(motion), !motion);
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

            if (PileMoves(now))
            {
                DrawPile(ds, sender, stack, fit, run, t, now, motion);
            }
        }
    }

    /// <summary>
    /// Whether the pile is drawn on the moving layer, over the arcs and the
    /// rocket: while a run plays through it, while its stages or coins move,
    /// and whenever it is open or opening, since the Mac draws the open card
    /// over everything round it (MapToolPile.swift; FlowMapView.swift:141-142).
    /// Only a closed pile at rest goes in the still layer.
    /// </summary>
    private bool PileMoves(double now) =>
        (Motion && WindowClocks.Running && ((_player.Run is { } run && run.Moving(now - run.Start)) || _pile.Moving(now)))
        || _pile.Stages(now, !(Motion && WindowClocks.Running)).Tall > 0;

    /// <summary>The seconds the library's turn reads: the Mac's reference-date clock.</summary>
    private static double LibrarySeconds(bool motion) =>
        motion ? DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() / 1000.0 - 978307200 : 0;

    private void OnDrawLibrary(CanvasControl sender, CanvasDrawEventArgs args)
    {
        long begun = LookFrames.Begin();
        try
        {
            DrawLibraryLayer(sender, args);
        }
        finally
        {
            LookFrames.End("map-library", begun);
        }
    }

    /// <summary>The library on its own layer (#2268), unless the moving layer has it.</summary>
    private void DrawLibraryLayer(CanvasControl sender, CanvasDrawEventArgs args)
    {
        double now = Now;
        if (sender.ActualWidth > 0 && sender.ActualHeight > 0 && _scene.DrawsMap && _scene.Stack is not null
            && !LibraryOnMovingLayer(now)
            && _scene.Nodes.FirstOrDefault(n => n.Id == "library") is { } library)
        {
            // Sized to the library's box (PlaceLibraryCanvas): draw through its offset.
            CanvasDrawingSession ds = args.DrawingSession;
            Vector3 offset = sender.ActualOffset;
            ds.Transform = Matrix3x2.CreateTranslation(-offset.X, -offset.Y);
            bool motion = Motion && WindowClocks.Running;
            DrawLibrary(ds, sender, library, Fit, LibrarySeconds(motion), !motion);
        }
    }

    /// <summary>The field, the nodes and the overflow counts: redrawn on a change, a pointer move or a shimmer pass.</summary>
    private void OnDrawStill(CanvasControl sender, CanvasDrawEventArgs args)
    {
        long begun = LookFrames.Begin();
        try
        {
            DrawStill(sender, args);
        }
        finally
        {
            LookFrames.End("map-still", begun);
        }
    }

    private void DrawStill(CanvasControl sender, CanvasDrawEventArgs args)
    {
        CanvasDrawingSession ds = args.DrawingSession;
        var size = new Size(sender.ActualWidth, sender.ActualHeight);
        if (size.Width <= 0 || size.Height <= 0)
        {
            return;
        }

        DrawField(ds, sender, size, Now, Motion && WindowClocks.Running);
        if (!_scene.DrawsMap)
        {
            _stillLayer.Drawn(_scene, pileMoving: false);
            return;
        }

        MapGeometry fit = Fit;
        foreach (MapNode node in _scene.Nodes)
        {
            DrawNode(ds, sender, node, fit);
        }

        foreach (MapOverflow overflow in _scene.Overflows)
        {
            DrawText(ds, "+" + overflow.Count, fit.Point(overflow.At), 11, Ink("textTertiary"), CanvasHorizontalAlignment.Center, bold: false);
        }

        // A pile at rest is drawn here, once, not on every tick.
        double now = Now;
        bool pileMoving = PileMoves(now);
        if (_scene.Stack is { } stack && !pileMoving)
        {
            DrawPile(ds, sender, stack, fit, null, 0, now, Motion && WindowClocks.Running);
        }

        _stillLayer.Drawn(_scene, pileMoving);
    }

    /// <summary>When the still layer is drawn again (MapMotion.cs <see cref="StillLayer"/>).</summary>
    private readonly StillLayer _stillLayer = new();

    private void DrawField(CanvasDrawingSession ds, ICanvasResourceCreator device, Size size, double now, bool motion)
    {
        float w = (float)size.Width, h = (float)size.Height;
        if (HighContrast)
        {
            Color outer = Ink("mapFieldOuter"), inner = Ink("mapFieldInner");
            ds.FillRectangle(0, 0, w, h, outer);
            using var field = new CanvasRadialGradientBrush(device, inner, outer)
            {
                Center = new Vector2(w * (float)MapField.CentreX, h * (float)MapField.CentreY),
                RadiusX = w * (float)MapField.RadiusX,
                RadiusY = h * (float)MapField.RadiusY,
            };
            ds.FillRectangle(0, 0, w, h, field);
            return;
        }

        // The field at rest and its marks' mask, rasterised once per size,
        // theme and DPI (checklist O3): a shimmer pass draws the still layer
        // at 60 frames a second, and filling every mark twice under a
        // gradient layer each frame cost about 3.5 s of CPU a pass on WARP.
        (CanvasRenderTarget Base, CanvasRenderTarget Mask) layers = FieldLayers(device, size);
        ds.DrawImage(layers.Base, 0, 0);
        (MapRgba _, MapRgba lit, MapRgba crest, MapRgba bloom) = MapField.Inks(Dark);

        // The shimmer: the whole tile filled once with a band of light
        // (Indicators.swift:809-830), through the marks' mask, which carries
        // the field's fade.
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
            using var masked = new CanvasImageBrush(device, layers.Mask);
            using (ds.CreateLayer(masked))
            {
                ds.FillRectangle(0, 0, w, h, band);
            }
        }

        // The marks near the pointer, lit, fading on Motion.fast (:835-865),
        // through the field's fade, as the marks themselves are.
        double fadeIn = LightOpacity(now, motion);
        if (fadeIn > 0 && (_pointer ?? _lastPointer) is { } p)
        {
            var fadeStops = MapField.FadeStops.Select(s => new CanvasGradientStop { Position = (float)s.At, Color = Color.FromArgb((byte)Math.Round(255 * s.Opacity), 255, 255, 255) }).ToArray();
            using var fade = new CanvasRadialGradientBrush(device, fadeStops)
            {
                Center = new Vector2(w * (float)MapField.FadeCentreX, h * (float)MapField.FadeCentreY),
                RadiusX = w * (float)MapField.FadeRadius,
                RadiusY = h * (float)MapField.FadeRadius,
            };
            using (ds.CreateLayer(fade))
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

    private CanvasRenderTarget? _fieldBase;
    private CanvasRenderTarget? _fieldMask;
    private string? _fieldLayerKey;

    /// <summary>
    /// The field at rest (its ground, its glow and every mark under the
    /// fade) and the marks' mask (white, under the same fade), for this size,
    /// theme and DPI; built again only when one of those changes.
    /// </summary>
    private (CanvasRenderTarget Base, CanvasRenderTarget Mask) FieldLayers(ICanvasResourceCreator device, Size size)
    {
        float dpi = _still.Dpi;
        string key = $"{size.Width}x{size.Height}|{Dark}|{dpi}";
        if (_fieldBase is not null && _fieldMask is not null && _fieldLayerKey == key)
        {
            return (_fieldBase, _fieldMask);
        }

        DropFieldLayers();
        float w = (float)size.Width, h = (float)size.Height;
        CanvasGeometry tile = Tile(device, size);
        (MapRgba resting, MapRgba _, MapRgba _, MapRgba _) = MapField.Inks(Dark);
        var fadeStops = MapField.FadeStops.Select(s => new CanvasGradientStop { Position = (float)s.At, Color = Color.FromArgb((byte)Math.Round(255 * s.Opacity), 255, 255, 255) }).ToArray();
        CanvasRadialGradientBrush Fade() => new(device, fadeStops)
        {
            Center = new Vector2(w * (float)MapField.FadeCentreX, h * (float)MapField.FadeCentreY),
            RadiusX = w * (float)MapField.FadeRadius,
            RadiusY = h * (float)MapField.FadeRadius,
        };

        var ground = new CanvasRenderTarget(device, w, h, dpi);
        using (CanvasDrawingSession g = ground.CreateDrawingSession())
        {
            Color outer = Ink("mapFieldOuter"), inner = Ink("mapFieldInner");
            g.FillRectangle(0, 0, w, h, outer);
            using (var field = new CanvasRadialGradientBrush(device, inner, outer)
            {
                Center = new Vector2(w * (float)MapField.CentreX, h * (float)MapField.CentreY),
                RadiusX = w * (float)MapField.RadiusX,
                RadiusY = h * (float)MapField.RadiusY,
            })
            {
                g.FillRectangle(0, 0, w, h, field);
            }

            using CanvasRadialGradientBrush fade = Fade();
            using (g.CreateLayer(fade))
            {
                g.FillGeometry(tile, Rgba(resting));
            }
        }

        var mask = new CanvasRenderTarget(device, w, h, dpi);
        using (CanvasDrawingSession m = mask.CreateDrawingSession())
        {
            m.Clear(Color.FromArgb(0, 0, 0, 0));
            using CanvasRadialGradientBrush fade = Fade();
            using (m.CreateLayer(fade))
            {
                m.FillGeometry(tile, Color.FromArgb(255, 255, 255, 255));
            }
        }

        _fieldBase = ground;
        _fieldMask = mask;
        _fieldLayerKey = key;
        return (ground, mask);
    }

    private void DropFieldLayers()
    {
        _fieldBase?.Dispose();
        _fieldMask?.Dispose();
        _fieldBase = null;
        _fieldMask = null;
        _fieldLayerKey = null;
    }

    private CanvasGeometry MarkUnit(ICanvasResourceCreator device) => _markUnit ??= MapPainter.MarkUnit(device);

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

    /// <summary>A tool's logo in its viewBox units, parsed once.</summary>
    private CanvasGeometry? Logo(ICanvasResourceCreator device, string name)
    {
        if (_logos.TryGetValue(name, out CanvasGeometry? cached))
        {
            return cached;
        }

        if (MapPainter.Logo(device, name) is not { } geometry)
        {
            return null;
        }

        _logos[name] = geometry;
        return geometry;
    }

    /// <summary>The tool's logo in its tint centred in a square, or its initials (FlowMapView.swift:528-536).</summary>
    private void DrawMark(CanvasDrawingSession ds, ICanvasResourceCreator device, GlassToolArt tool, MapPoint centre, double side, double alpha = 1)
    {
        Color tint = Ink(tool.TintToken, alpha);
        if (tool.Logo is { } name && MapArt.Logos.TryGetValue(name, out LogoArtwork? art) && Logo(device, name) is { } geometry)
        {
            ds.FillGeometry(geometry.Transform(MapPainter.LogoFit(art, new Vector2((float)centre.X, (float)centre.Y), side)), tint);
        }
        else
        {
            DrawText(ds, tool.Initials, centre, Math.Max(9, side * 0.6), tint, CanvasHorizontalAlignment.Center, bold: true, mono: true);
        }
    }

    private void DrawArcs(CanvasDrawingSession ds, ICanvasResourceCreator device, MapGeometry fit, double seconds, bool motion)
    {
        bool strong = HighContrast;
        foreach (MapArc arc in _scene.Arcs)
        {
            using var builder = new CanvasPathBuilder(device);
            MapPoint a = fit.Point(arc.From), c1 = fit.Point(arc.Control1), c2 = fit.Point(arc.Control2), b = fit.Point(arc.To);
            builder.BeginFigure((float)a.X, (float)a.Y);
            builder.AddCubicBezier(new Vector2((float)c1.X, (float)c1.Y), new Vector2((float)c2.X, (float)c2.Y), new Vector2((float)b.X, (float)b.Y));
            builder.EndFigure(CanvasFigureLoop.Open);
            using CanvasGeometry path = CanvasGeometry.CreatePath(builder);
            MapPainter.DrawArc(ds, path, arc.Style, Healthy, strong, fit.Scale, seconds, !motion, stroke => stroke.Ink == "white"
                ? Color.FromArgb((byte)Math.Round(255 * stroke.Alpha * arc.Dim), 255, 255, 255)
                : Ink(stroke.Ink, stroke.Alpha * arc.Dim));
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
        // The slab and the tower take their own tokens, darker than
        // statusOff in light, so each clears 3:1 where it is drawn lightest
        // (owner, 2026-11-20: "Raise to pass, both apps").
        Color off = Ink("mapPadSlab", dim);
        using (var slab = new CanvasLinearGradientBrush(device, With(off, 0.75), off)
        {
            StartPoint = new Vector2((float)c.X, (float)(c.Y - 5 * s)),
            EndPoint = new Vector2((float)c.X, (float)(c.Y + 5 * s)),
        })
        {
            ds.FillRoundedRectangle(new Rect(c.X - half, c.Y - 5 * s, half * 2, 10 * s), (float)(4 * s), (float)(4 * s), slab);
        }

        ds.FillRoundedRectangle(new Rect(c.X - 24 * s, c.Y + 5 * s, 48 * s, 6 * s), (float)(3 * s), (float)(3 * s), Ink("mapNodeOff", dim));
        Color steel = Ink("mapPadTower", 0.7 * dim);
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

    /// <summary>The station's or the orb's drawn frames for the current look.</summary>
    private readonly LibraryFrameCache<CanvasRenderTarget> _libraryFrames = new();

    /// <summary>
    /// The station or the orb at the library's place (FlowMapView.swift:236-249),
    /// with its glow. Each of its 15-a-second frames is drawn once per size,
    /// style, theme and lit state and then reused: the turn repeats, and the
    /// mesh is the costliest thing the map draws. The station's glow is drawn
    /// once from its ring (<see cref="StationGlow"/>, #2268), under every
    /// frame; the orb's is still drawn into each frame from its art, which is
    /// rasterised once per frame into a bitmap the glow and the frame both
    /// read (checklist O3: a command list was rasterised twice).
    /// </summary>
    private void DrawLibrary(CanvasDrawingSession ds, ICanvasResourceCreator device, MapNode library, MapGeometry fit, double seconds, bool still)
    {
        MapPoint at = fit.Point(library.At);
        bool launch = _style == TracesMapStyle.Launch;
        double turn = launch ? LibraryStation.TurnSeconds : LibraryOrb.TurnSeconds;
        int frames = MapMotion.LibraryFrames(turn);
        int index = still ? -1 : MapMotion.LibraryFrameIndex(seconds, turn);
        _libraryFrames.Look($"{_style}|{library.Active}|{Math.Round(fit.Scale, 3)}|{Dark}|{HighContrast}");

        double w = (launch ? LibraryStation.FrameWidth : TransferDrawing.OrbFrame) * fit.Scale;
        double h = (launch ? LibraryStation.FrameHeight : TransferDrawing.OrbFrame) * fit.Scale;
        double pad = (launch ? LibraryStation.GlowRadius : LibraryOrb.GlowRadius) * 2;
        float side = (float)(w + 2 * pad), tall = (float)(h + 2 * pad);
        Color brand = Ink("heroCampaignBright");
        float x = (float)(at.X - w / 2 - pad), y = (float)(at.Y - h / 2 - pad);
        if (launch)
        {
            ds.DrawImage(StationGlow(device, side, tall, _canvas.Dpi, pad, w, h, library.Active, brand), x, y);
        }

        CanvasRenderTarget? frame = _libraryFrames.Get(index);
        if (frame is null)
        {
            double angle = launch
                ? (still ? LibraryStation.RestAngle : LibraryStation.RestAngle + (index + 0.0) / frames * 2 * Math.PI)
                : (still ? LibraryOrb.RestAngle : LibraryOrb.RestAngle + (index + 0.0) / frames * 2 * Math.PI);
            frame = new CanvasRenderTarget(device, side, tall, _canvas.Dpi);
            using (var art = new CanvasRenderTarget(device, side, tall, _canvas.Dpi))
            {
                using (CanvasDrawingSession layer = art.CreateDrawingSession())
                {
                    layer.Clear(Color.FromArgb(0, 0, 0, 0));
                    if (launch)
                    {
                        MapPainter.Station(layer, device, new Vector2((float)pad, (float)pad), w, h, angle);
                    }
                    else
                    {
                        MapPainter.Orb(layer, device, new Vector2((float)(pad + w / 2), (float)(pad + h / 2)), LibraryOrb.Radius(w, h), angle, brand, Ink("heroCampaignDeep"));
                    }
                }

                using CanvasDrawingSession f = frame.CreateDrawingSession();
                f.Clear(Color.FromArgb(0, 0, 0, 0));
                if (!launch)
                {
                    DrawGlow(f, art, With(brand, LibraryOrb.Glow(library.Active)), LibraryOrb.GlowRadius);
                }

                using (f.CreateLayer(launch ? 1 : (float)LibraryOrb.Opacity(library.Active)))
                {
                    f.DrawImage(art);
                }
            }

            // Kept for the next turn only while a whole turn's frames stay
            // small (64 MB); past that each frame is drawn and let go.
            if (side * tall * 4 * frames <= 64.0 * 1024 * 1024)
            {
                _libraryFrames.Put(index, frame);
            }
            else
            {
                ds.DrawImage(frame, x, y);
                frame.Dispose();
                return;
            }
        }

        ds.DrawImage(frame, x, y);
    }

    /// <summary>The station's glow, kept for one size, scale, lit state and colour.</summary>
    private readonly LibraryFrameCache<CanvasRenderTarget> _stationGlow = new();

    /// <summary>
    /// The station's glow on a <paramref name="side"/> by <paramref name="tall"/>
    /// image whose frame starts <paramref name="pad"/> in (#2268,
    /// <c>GlassLibraryStationGlow</c>): the ring's outline, which is the same
    /// at every angle of the turn, filled even-odd and blurred as the glow
    /// round the whole mesh was, in the same colour, strength and radius,
    /// once per size, scale, lit state and colour rather than on every frame.
    /// </summary>
    private CanvasRenderTarget StationGlow(ICanvasResourceCreator device, float side, float tall, float dpi, double pad, double w, double h, bool lit, Color brand)
    {
        _stationGlow.Look($"{side}|{tall}|{dpi}|{lit}|{brand}");
        if (_stationGlow.Get(0) is { } kept)
        {
            return kept;
        }

        var glow = new CanvasRenderTarget(device, side, tall, dpi);
        using (var ring = new CanvasRenderTarget(device, side, tall, dpi))
        {
            using (CanvasDrawingSession r = ring.CreateDrawingSession())
            {
                r.Clear(Color.FromArgb(0, 0, 0, 0));
                using var path = new CanvasPathBuilder(device);
                path.SetFilledRegionDetermination(CanvasFilledRegionDetermination.Alternate);
                foreach (MapPoint[] loop in LibraryStation.RingOutline(w, h))
                {
                    path.BeginFigure((float)(pad + loop[0].X), (float)(pad + loop[0].Y));
                    for (int i = 1; i < loop.Length; i++)
                    {
                        path.AddLine((float)(pad + loop[i].X), (float)(pad + loop[i].Y));
                    }

                    path.EndFigure(CanvasFigureLoop.Closed);
                }

                using CanvasGeometry outline = CanvasGeometry.CreatePath(path);
                r.FillGeometry(outline, Color.FromArgb(255, 0, 0, 0));
            }

            using CanvasDrawingSession g = glow.CreateDrawingSession();
            g.Clear(Color.FromArgb(0, 0, 0, 0));
            DrawGlow(g, ring, With(brand, LibraryStation.Glow(lit)), LibraryStation.GlowRadius);
        }

        _stationGlow.Put(0, glow);
        return glow;
    }

    /// <summary>A shadow-style glow of <paramref name="radius"/> round an image (SwiftUI's <c>.shadow</c>).</summary>
    private static void DrawGlow(CanvasDrawingSession ds, ICanvasImage image, Color colour, double radius)
    {
        using var shadow = new ShadowEffect { Source = image, BlurAmount = (float)(radius / 2), ShadowColor = colour };
        ds.DrawImage(shadow);
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
                DrawRocket(ds, device, fit.Point(rocket.At), rocket.Angle, s, rocket.Burn, glowRadius, glowColour);
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
            DrawText(ds, heading.ToUpperInvariant(), new MapPoint(left.X + 30 * s, left.Y - height / 2 - 12 * s), Math.Max(9, 10 * s), Ink("textTertiary"), CanvasHorizontalAlignment.Center, bold: true, tracking: MapFonts.ToolsHeadingTracking);
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

    /// <summary>Text formats by size and weight, made once (checklist O3).</summary>
    private static readonly Dictionary<string, CanvasTextFormat> Formats = new(StringComparer.Ordinal);

    /// <summary>
    /// The installed families, read once. DirectWrite takes one family name,
    /// never a list (<see cref="MapFonts"/>).
    /// </summary>
    private static readonly string[] InstalledFamilies = CanvasTextFormat.GetSystemFontFamilies();

    private static void DrawText(CanvasDrawingSession ds, string text, MapPoint at, double size, Color colour, CanvasHorizontalAlignment align, bool bold, bool mono = false, bool icons = false, double tracking = 0)
    {
        if (string.IsNullOrEmpty(text))
        {
            return;
        }

        string key = $"{size:F2}|{bold}|{mono}|{icons}";
        if (!Formats.TryGetValue(key, out CanvasTextFormat? format))
        {
            // Sizes follow the zoom; a resize must not grow this without end.
            if (Formats.Count >= 64)
            {
                foreach (CanvasTextFormat old in Formats.Values)
                {
                    old.Dispose();
                }

                Formats.Clear();
            }

            format = new CanvasTextFormat
            {
                FontFamily = MapFonts.Family(icons ? MapFontRole.Icons : mono ? MapFontRole.Mono : MapFontRole.Text, InstalledFamilies),
                FontSize = (float)size,
                FontWeight = bold ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal,
                WordWrapping = CanvasWordWrapping.NoWrap,
                HorizontalAlignment = CanvasHorizontalAlignment.Left,
                VerticalAlignment = CanvasVerticalAlignment.Center,
            };
            Formats[key] = format;
        }

        using var layout = new CanvasTextLayout(ds, text, format, 0, 0);
        if (tracking > 0)
        {
            // SwiftUI's tracking: space after every character.
            layout.SetCharacterSpacing(0, text.Length, 0, (float)tracking, 0);
        }

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

/// <summary>
/// Lays its children out in rows, each whole, wrapping onto another row
/// where the next would not fit: the legend at the map's foot wraps rather
/// than run under the zoom (FlowMapView.swift:181-186, <c>HistoryFilterFlow</c>).
/// </summary>
internal sealed partial class FlowPanel : Panel
{
    public double Spacing { get; set; }

    public double LineSpacing { get; set; }

    protected override Size MeasureOverride(Size available)
    {
        double x = 0, y = 0, line = 0, widest = 0;
        foreach (UIElement child in Children)
        {
            child.Measure(new Size(double.PositiveInfinity, double.PositiveInfinity));
            Size d = child.DesiredSize;
            if (x > 0 && x + d.Width > available.Width)
            {
                y += line + LineSpacing;
                x = 0;
                line = 0;
            }

            x += d.Width + Spacing;
            line = Math.Max(line, d.Height);
            widest = Math.Max(widest, x - Spacing);
        }

        return new Size(double.IsInfinity(available.Width) ? widest : Math.Min(widest, available.Width), y + line);
    }

    protected override Size ArrangeOverride(Size final)
    {
        double x = 0, y = 0, line = 0;
        foreach (UIElement child in Children)
        {
            Size d = child.DesiredSize;
            if (x > 0 && x + d.Width > final.Width)
            {
                y += line + LineSpacing;
                x = 0;
                line = 0;
            }

            child.Arrange(new Rect(x, y, d.Width, d.Height));
            x += d.Width + Spacing;
            line = Math.Max(line, d.Height);
        }

        return final;
    }
}

