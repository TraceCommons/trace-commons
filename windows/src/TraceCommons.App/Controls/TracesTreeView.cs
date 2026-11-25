using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.Linq;
using Microsoft.UI.Input;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop;
using TraceCommons.Interop.Map;
using TraceCommons.Interop.Shell;
using Windows.System;
using Windows.UI.ViewManagement;
using Color = Windows.UI.Color;
using TraceCommons.App.Look;

namespace TraceCommons.App.Controls;

/// <summary>
/// What the tree asks of its window: the queue's own rows for what the
/// tree draws beside the map's read (a trace's size, holds and whether it
/// can be sent; a folder's Submit offer), and the window's existing consent
/// paths for every action. The tree makes no consent call of its own.
/// </summary>
internal interface ITracesTreeHost
{
    /// <summary>The queue's folder group, or null while the queue has not listed it.</summary>
    ViewModels.QueueGroupViewModel? Group(string projectId);

    /// <summary>The queue's row for a trace, or null while it has not listed it.</summary>
    ViewModels.QueueEntryViewModel? Entry(string entryId);

    bool IsTraceSubmitting(string entryId);

    /// <summary>The selection moved: the inspector shows it.</summary>
    void Selected(TreeSelection? selection);

    /// <summary>Return on a selected trace: its review.</summary>
    void Review(string entryId);

    void SubmitFolder(ViewModels.QueueGroupViewModel group, string? outcome);

    void IgnoreFolder(ViewModels.QueueGroupViewModel group);

    void SubmitTrace(ViewModels.QueueEntryViewModel entry);

    void DismissTrace(ViewModels.QueueEntryViewModel entry);

    /// <summary>The folders' Automatic writes: which are in flight, and each one's refusal.</summary>
    FolderAutomatic AutomaticWrites { get; }

    /// <summary>A folder row's Automatic switch moved (TracesViews.swift:685-691).</summary>
    void SetFolderAutomatic(TracesTree.FolderNode folder, bool on);

    /// <summary>The person's folder order changed: keep it.</summary>
    void OrderChanged(IReadOnlyList<string> order, bool arranged);

    /// <summary>
    /// Automatic for every folder (SharingChoiceCard.swift:144-163): what is
    /// in force, or null while the grant or its words are unknown, when the
    /// eye is not drawn.
    /// </summary>
    TraceCommons.Interop.FirstRun.SharingPath? AutomaticForEveryFolder { get; }

    /// <summary>Whether a press of the eye is still running; the eye is off meanwhile.</summary>
    bool SharingChoiceWorking { get; }

    /// <summary>The eye pressed: the Mac's steps; the refusal to show under it, or null.</summary>
    System.Threading.Tasks.Task<string?> PressAutomaticForEveryFolderAsync();
}

/// <summary>
/// The Traces tree (the Mac's <c>TracesTreeView</c>, TracesViews.swift, with
/// its rows' <c>GlassListRow</c>, ListRow.swift, and the heading's search,
/// NudgeViews.swift:65-160). Folders at the top level, each opening onto its
/// traces; one tab stop whose arrow keys move the selection; folders dragged
/// into place; the selection kept on screen. Every rule is
/// <see cref="TracesTreeState"/>'s and <see cref="TracesTreeRows"/>'s; this
/// control draws what they return and forwards what the person does.
/// </summary>
internal sealed class TracesTreeView : UserControl
{
    private readonly TracesTreeState _state;
    private readonly ITracesTreeHost _host;
    private readonly TreeWords _words = TreeWords.Core.Value;
    private readonly UISettings _ui = new();
    private readonly AccessibilitySettings _accessibility = new();
    private readonly Grid _root = new();
    private readonly ScrollViewer _scroller = new();
    private readonly StackPanel _rows = new() { Spacing = 2 };
    private readonly StackPanel _content = new();
    private readonly StackPanel _footer = new() { Margin = new Thickness(-14, 12, -14, 0) };
    private readonly TextBlock _empty = new();
    private readonly Button _searchButton = new();
    private readonly Grid _eyeSlot = new();

    // The column titles, drawn over a tree with folders (TracesViews.swift:193-198).
    private Grid? _titles;

    // The heading's search and order: drawn on an empty tree only when a
    // filter emptied it (ShowListControls).
    private readonly List<FrameworkElement> _listControls = new();
    private readonly Grid _searchField = new();
    private readonly TextBox _searchBox = new();
    private readonly TranslateTransform _searchShift = new();
    private readonly Dictionary<string, FrameworkElement> _rowById = new(StringComparer.Ordinal);

    /// <summary>Each drawn row's key (<see cref="TreeRowKeys"/>), in the order drawn: a row whose key holds is kept.</summary>
    private readonly Dictionary<string, string> _rowKeys = new(StringComparer.Ordinal);
    private readonly List<string> _drawnIds = new();
    private bool _refreshQueued;
    private readonly Stopwatch _clock = new();
    private bool _searching;
    private bool _ticking;

    // The search field's move: from where, to where (0 hidden, 1 shown).
    private double _searchFrom;
    private double _searchTo;
    private double _searchAt = -1;

    // The scroll to a row.
    private double _scrollFrom;
    private double _scrollTo;
    private double _scrollAt = -1;

    // A folder being dragged into place (TracesViews.swift:729-768).
    private string? _pressedFolder;
    private uint _pointer;
    private double _pressY;
    private string? _draggingFolder;
    private double _dragStartMid;
    private double _dragOffset;
    private double _releaseFrom;
    private double _releaseAt = -1;
    private bool _suppressClick;

    public TracesTreeView(TracesTreeState state, ITracesTreeHost host)
    {
        _state = state;
        _host = host;
        IsTabStop = true;
        UseSystemFocusVisuals = false;
        AutomationProperties.SetName(this, _words["tree_label"]);

        _root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        _root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        _root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        _root.Children.Add(Heading());
        if (ColumnTitles() is { } titles)
        {
            Grid.SetRow(titles, 1);
            _root.Children.Add(titles);
            _titles = titles;
        }

        _content.Children.Add(_rows);
        _content.Children.Add(_footer);
        _scroller.Content = _content;
        _scroller.HorizontalScrollMode = ScrollMode.Disabled;
        _scroller.HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled;
        _scroller.IsTabStop = false;
        _scroller.Padding = new Thickness(14, 0, 14, 12);
        Grid.SetRow(_scroller, 2);
        _root.Children.Add(_scroller);

        _empty.TextWrapping = TextWrapping.Wrap;
        _empty.HorizontalAlignment = HorizontalAlignment.Center;
        _empty.Margin = new Thickness(0, 16, 0, 0);

        Content = _root;
        KeyDown += OnKeyDown;

        // A drag is followed on the list, which lives through the redraws a
        // move makes; a row would lose the pointer as it is drawn again.
        _rows.PointerMoved += OnRowsPointerMoved;
        _rows.PointerReleased += OnRowsPointerReleased;
        _rows.PointerCaptureLost += (_, _) => EndDrag();
        ActualThemeChanged += (_, _) => RenderAll();
        // A live theme or high contrast switch too (ThemeBrushes).
        ThemeBrushes.RedrawOnChange(this, () => { RenderAll(); });
        Unloaded += (_, _) => StopTicking();
    }

    /// <summary>
    /// The window's own sections that follow the folders (the certificate
    /// list, the scrubbing caveat, the week band): they scroll with the tree
    /// rather than take its room.
    /// </summary>
    public void AddFooter(UIElement section) => _footer.Children.Add(section);

    /// <summary>The tree's state, as read and arranged.</summary>
    public TracesTreeState State => _state;

    /// <summary>The scroller the window watches for a settle.</summary>
    public ScrollViewer Scroller => _scroller;

    private bool Motion => _ui.AnimationsEnabled;

    /// <summary>A new read from the core.</summary>
    public void Read(MapSnapshot snapshot, IReadOnlySet<string> scansWhenUnset)
    {
        _state.Read(snapshot, scansWhenUnset);
        Render();
    }

    /// <summary>The queue's own rows changed (sizes, holds, offers): draw again the rows that changed.</summary>
    public void Refresh() => Render();

    /// <summary>
    /// <see cref="Refresh"/>, once, after the rest of this turn: a queue
    /// refresh clears and refills its collections a row at a time, and each
    /// change asked for a whole draw.
    /// </summary>
    public void RequestRefresh()
    {
        if (_refreshQueued)
        {
            return;
        }

        _refreshQueued = DispatcherQueue.TryEnqueue(() =>
        {
            _refreshQueued = false;
            Render();
        });
        if (!_refreshQueued)
        {
            Render();
        }
    }

    /// <summary>Draws <paramref name="folderId"/>'s row again even when nothing it draws changed: a switch that wrote nothing goes back.</summary>
    public void Redraw(string folderId) => Render(new TreeSelection.Folder(folderId).RawValue);

    /// <summary>Selects a row from elsewhere (the map, a notice): its folder opens and it is scrolled to.</summary>
    public void Select(TreeSelection? selection)
    {
        if (selection == _state.Selection)
        {
            return;
        }

        _state.Select(selection);
        Render();
    }

    // ---- The heading: title, search, order --------------------------------

    private Grid Heading()
    {
        var heading = new Grid { Padding = new Thickness(22, 10, 14, 6), ColumnSpacing = 6 };
        heading.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        heading.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        heading.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        heading.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var title = new TextBlock
        {
            Text = _words["tree_title"],
            Style = (Style)Application.Current.Resources["TcTitleCardTextStyle"],
            VerticalAlignment = VerticalAlignment.Center,
            TextTrimming = TextTrimming.CharacterEllipsis,
        };
        AutomationProperties.SetHeadingLevel(title, AutomationHeadingLevel.Level2);
        heading.Children.Add(title);

        // The search icon, or the open field with its clear button (NudgeViews.swift:139-160).
        var search = new Grid();
        Grid.SetColumn(search, 1);
        _searchButton.Content = new FontIcon { Glyph = "", FontSize = 12 };
        _searchButton.Style = (Style)Application.Current.Resources["TcSecondaryButtonStyle"];
        AutomationProperties.SetName(_searchButton, _words["search"]);
        ToolTipService.SetToolTip(_searchButton, _words["search"]);
        _searchButton.Click += (_, _) => OpenSearch(true);
        search.Children.Add(_searchButton);

        _searchField.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        _searchField.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        _searchField.ColumnSpacing = 6;
        _searchField.MaxWidth = 220;
        _searchField.Visibility = Visibility.Collapsed;
        _searchField.RenderTransform = _searchShift;
        _searchBox.PlaceholderText = _words["search_prompt"];
        _searchBox.MinWidth = 160;
        AutomationProperties.SetName(_searchBox, _words["search"]);
        _searchBox.TextChanged += (_, _) => ApplySearch(_searchBox.Text);
        _searchBox.KeyDown += (_, e) =>
        {
            if (e.Key == VirtualKey.Escape)
            {
                e.Handled = true;
                OpenSearch(false);
            }
        };
        _searchField.Children.Add(_searchBox);
        var clear = new Button
        {
            Content = new FontIcon { Glyph = "", FontSize = 10 },
            Style = (Style)Application.Current.Resources["TcSecondaryButtonStyle"],
        };
        AutomationProperties.SetName(clear, _words["search_clear"]);
        ToolTipService.SetToolTip(clear, _words["search_clear"]);
        clear.Click += (_, _) => OpenSearch(false);
        Grid.SetColumn(clear, 1);
        _searchField.Children.Add(clear);
        search.Children.Add(_searchField);
        heading.Children.Add(search);
        _listControls.Add(search);

        // Automatic for every folder, an eye left of the order control
        // (owner, 2026-11-02; NudgeViews.swift:69-71, :147-150). Drawn by
        // DrawEye once the grant is known.
        Grid.SetColumn(_eyeSlot, 2);
        _eyeSlot.VerticalAlignment = VerticalAlignment.Center;
        heading.Children.Add(_eyeSlot);
        _listControls.Add(_eyeSlot);

        if (TreeOrderWords.Label.Length > 0)
        {
            var order = new Button
            {
                Content = new FontIcon { Glyph = "", FontSize = 12 },
                Style = (Style)Application.Current.Resources["TcSecondaryButtonStyle"],
            };
            AutomationProperties.SetName(order, TreeOrderWords.Label);
            ToolTipService.SetToolTip(order, TreeOrderWords.Label);
            var menu = new MenuFlyout();
            menu.Opening += (_, _) => FillOrderMenu(menu);
            order.Flyout = menu;
            Grid.SetColumn(order, 3);
            heading.Children.Add(order);
            _listControls.Add(order);
        }

        return heading;
    }

    /// <summary>
    /// The eye (SharingChoiceCard.swift:144-163): on while Automatic is in
    /// force; its tooltip is its name over the count line; off while a press
    /// runs. Pressing it takes the window's steps, and a refusal opens under
    /// it in the core's words. Not drawn without the grant or the words.
    /// </summary>
    private void DrawEye()
    {
        _eyeSlot.Children.Clear();
        string name = _words["automatic_all"];
        if (name.Length == 0 || _words["automatic_count"].Length == 0
            || _host.AutomaticForEveryFolder is not TraceCommons.Interop.FirstRun.SharingPath current)
        {
            return;
        }

        bool on = current == TraceCommons.Interop.FirstRun.SharingPath.Automatic;
        string line = SharingChoice.AutomaticCount(_state.Tree.Folders, _words["automatic_count"]);
        var eye = new ToggleButton
        {
            Content = new FontIcon { Glyph = SharingChoice.ToolbarGlyph, FontSize = 12 },
            IsChecked = on,
            IsEnabled = !_host.SharingChoiceWorking,
            Padding = new Thickness(11, 4, 11, 4),
            CornerRadius = (CornerRadius)Application.Current.Resources["TcControlCornerRadius"],
            VerticalAlignment = VerticalAlignment.Center,
        };
        AutomationProperties.SetName(eye, name);
        AutomationProperties.SetHelpText(eye, line);
        ToolTipService.SetToolTip(eye, SharingChoice.ToolbarHelp(name, line));
        eye.Click += async (_, _) =>
        {
            // The core's answer draws it; the press itself changes nothing.
            eye.IsChecked = on;
            eye.IsEnabled = false;
            string? refusal = await _host.PressAutomaticForEveryFolderAsync();
            Render();
            if (refusal is { Length: > 0 })
            {
                new Flyout
                {
                    Content = new TextBlock { Text = refusal, TextWrapping = TextWrapping.Wrap, MaxWidth = 280 },
                    Placement = FlyoutPlacementMode.Bottom,
                }.ShowAt(_eyeSlot);
            }
        };
        _eyeSlot.Children.Add(eye);
    }

    /// <summary>
    /// The titles over the tree's columns while the Automatic switches show
    /// (TracesViews.swift:193-198, :1537-1566): the folders, Submit centred
    /// over the row's split button, and Auto over its switch, before the row
    /// menu's 22 px column. Eyebrow type, tertiary ink, one line; the rows
    /// already name their controls, so screen readers skip the titles.
    /// </summary>
    private Grid? ColumnTitles()
    {
        if (SharingChoice.ColumnTitles(_words) is not { } titles)
        {
            return null;
        }

        titles = SharingChoice.Eyebrow(titles, CultureInfo.CurrentCulture);

        var grid = new Grid
        {
            // The scroller's 14 px, then the row's own insets (8 leading, 6 trailing).
            Margin = new Thickness(14 + 8, 0, 14 + 6, 4),
            ColumnSpacing = 8,
        };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(22) });
        TextBlock Title(string text, int column, double? width, double minWidth, TextAlignment alignment)
        {
            var block = new TextBlock
            {
                Text = text,
                Style = (Style)Application.Current.Resources["TcEyebrowTextStyle"],
                TextAlignment = alignment,
                TextTrimming = TextTrimming.CharacterEllipsis,
                MaxLines = 1,
                MinWidth = minWidth,
            };
            if (width is double w)
            {
                block.Width = w;
            }

            Grid.SetColumn(block, column);
            grid.Children.Add(block);
            return block;
        }

        Title(titles.Folders, 0, null, 0, TextAlignment.Left);
        Title(titles.Submit, 1, null, SharingChoice.SubmitColumnMinWidth, TextAlignment.Center);
        Title(titles.Automatic, 2, GlassTokens.Size.WatchSwitchWidth, 0, TextAlignment.Center);
        AutomationProperties.SetAccessibilityView(grid, AccessibilityView.Raw);
        return grid;
    }

    /// <summary>The order menu (NudgeViews.swift:104-130): Oldest first, Your order, and Show ignored folders.</summary>
    private void FillOrderMenu(MenuFlyout menu)
    {
        menu.Items.Clear();
        var queue = new RadioMenuFlyoutItem { Text = TreeOrderWords.Queue, GroupName = "order", IsChecked = _state.KeepsOrder && !_state.Arranged };
        queue.Click += (_, _) =>
        {
            _state.ChooseQueueOrder();
            _host.OrderChanged(_state.FolderOrder, _state.Arranged);
            Render();
        };
        menu.Items.Add(queue);
        if (TreeOrderWords.Arranged.Length > 0)
        {
            var arranged = new RadioMenuFlyoutItem { Text = TreeOrderWords.Arranged, GroupName = "order", IsChecked = _state.Arranged };
            arranged.Click += (_, _) =>
            {
                _state.ChooseArranged();
                _host.OrderChanged(_state.FolderOrder, _state.Arranged);
                Render();
            };
            menu.Items.Add(arranged);
        }

        menu.Items.Add(new MenuFlyoutSeparator());
        var ignored = new ToggleMenuFlyoutItem { Text = MonitorShellCopy.Top("show_ignored_folders"), IsChecked = _state.ShowsIgnored };
        ignored.Click += (_, _) =>
        {
            _state.ShowIgnored(ignored.IsChecked);
            Render();
        };
        menu.Items.Add(ignored);
    }

    private void OpenSearch(bool open)
    {
        if (open == _searching)
        {
            return;
        }

        _searching = open;
        if (!open)
        {
            _searchBox.Text = string.Empty;
        }

        // In from the trailing edge with a fade, GlassMotion.fast (NudgeViews.swift:150-161).
        _searchFrom = _searchAt < 0 ? (open ? 0 : 1) : SearchProgress();
        _searchTo = open ? 1 : 0;
        _searchField.Visibility = Visibility.Visible;
        _searchButton.Visibility = open ? Visibility.Collapsed : Visibility.Visible;
        if (open)
        {
            _searchBox.Focus(FocusState.Programmatic);
        }
        else
        {
            Focus(FocusState.Programmatic);
        }

        if (!Motion)
        {
            SetSearchFrame(_searchTo);
            return;
        }

        _searchAt = _clock.Elapsed.TotalSeconds;
        StartTicking();
    }

    private double SearchProgress()
    {
        if (_searchAt < 0)
        {
            return _searchTo;
        }

        double p = TracesTreeRows.SearchFieldSlide.At(_clock.Elapsed.TotalSeconds - _searchAt);
        return _searchFrom + (_searchTo - _searchFrom) * p;
    }

    private void SetSearchFrame(double shown)
    {
        _searchField.Opacity = shown;
        _searchShift.X = (1 - shown) * Math.Max(_searchField.ActualWidth, _searchField.MaxWidth);
        if (shown <= 0 && !_searching)
        {
            _searchField.Visibility = Visibility.Collapsed;
        }
    }

    private void ApplySearch(string query)
    {
        _state.SetSearch(query);
        Render();
    }

    // ---- The rows --------------------------------------------------------

    /// <summary>Every row built again: the theme changed, which no row's key holds.</summary>
    private void RenderAll()
    {
        _rowKeys.Clear();
        Render();
    }

    /// <summary>
    /// Draws the rows, building again only those whose drawn values changed
    /// (<see cref="TreeRowKeys"/>) and <paramref name="force"/>; the others
    /// are kept, in their place when the order holds.
    /// </summary>
    private void Render(string? force = null)
    {
        DrawEye();
        ShowListControls();
        IReadOnlyList<TreeRow> rows = _state.Rows;
        var keyed = new List<(string Id, string Key)>(rows.Count);
        foreach (TreeRow row in rows)
        {
            keyed.Add((row.Id, Key(row)));
        }

        TreeRowPlan plan = TreeRowKeys.Plan(_drawnIds, _rowKeys, keyed, force);
        var build = new HashSet<string>(plan.Build, StringComparer.Ordinal);
        var drawn = new List<FrameworkElement>(rows.Count);
        for (int i = 0; i < rows.Count; i++)
        {
            TreeRow row = rows[i];
            if (!build.Contains(row.Id) && _rowById.TryGetValue(row.Id, out FrameworkElement? kept))
            {
                drawn.Add(kept);
                continue;
            }

            drawn.Add(row switch
            {
                TreeRow.FolderRow f => FolderRow(f.Folder),
                TreeRow.SessionRow s => SessionRow(s.Entry, s.Off),
                _ => throw new InvalidOperationException(),
            });
        }

        bool searchNone = _state.Shown.Folders.Count == 0 && _state.Search.Trim().Length > 0;
        if (plan.InPlace && !searchNone && _rows.Children.Count == rows.Count)
        {
            for (int i = 0; i < drawn.Count; i++)
            {
                if (!ReferenceEquals(_rows.Children[i], drawn[i]))
                {
                    _rows.Children[i] = drawn[i];
                }
            }
        }
        else
        {
            _rows.Children.Clear();
            foreach (FrameworkElement element in drawn)
            {
                _rows.Children.Add(element);
            }
        }

        _rowById.Clear();
        _rowKeys.Clear();
        _drawnIds.Clear();
        for (int i = 0; i < rows.Count; i++)
        {
            _rowById[keyed[i].Id] = drawn[i];
            _rowKeys[keyed[i].Id] = keyed[i].Key;
            _drawnIds.Add(keyed[i].Id);
        }

        if (searchNone)
        {
            _empty.Text = _words["search_none"];
            _empty.Foreground = Brush("textTertiary");
            _rows.Children.Add(_empty);
        }

        if (_state.ScrollTarget is { } target)
        {
            DispatcherQueue.TryEnqueue(() => ScrollTo(target));
        }
    }

    /// <summary>
    /// The heading's search and order on an empty tree: only when a filter
    /// emptied it, so the way out stays (TracesViews.swift:181-187, 235-239).
    /// Otherwise they are not drawn, and are no Tab stop sitting unseen
    /// beside the empty state (box tab walk, empty and coreDown samples,
    /// 2026-11-20).
    /// </summary>
    private void ShowListControls()
    {
        bool shown = _state.Tree.Folders.Count > 0
            || _state.Search.Trim().Length > 0
            || TracesTreeRows.KeepsControlsWhenEmpty(idleOnly: false, _state.Tree.HiddenIgnored);
        foreach (FrameworkElement control in _listControls)
        {
            control.Visibility = shown ? Visibility.Visible : Visibility.Collapsed;
        }

        if (_titles is not null)
        {
            _titles.Visibility = _state.Tree.Folders.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        }
    }

    /// <summary>The leading inset: 8, and 18 for each level below the folders (ListRow.swift:279-281).</summary>
    private static double Inset(bool session) => 8 + (session ? 18 : 0);

    private TreeRowGrid RowFrame(bool session, bool selected, string title, string? sub, TreeFlag? flag, bool off, ToolTileKind tile)
    {
        var row = new TreeRowGrid(session)
        {
            MinHeight = 40,
            Padding = new Thickness(Inset(session), 0, 6, 0),
            ColumnSpacing = 8,
            CornerRadius = new CornerRadius(11),
            Background = selected ? Brush("selection") : new SolidColorBrush(Color.FromArgb(0, 0, 0, 0)),
        };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(16) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(22) });

        // Hover wash (ListRow.swift:119-122); never takes the keyboard.
        if (!selected)
        {
            row.PointerEntered += (_, _) => row.Background = Brush("rowHover");
            row.PointerExited += (_, _) => row.Background = new SolidColorBrush(Color.FromArgb(0, 0, 0, 0));
        }

        var tileView = new ToolTileView(tile) { VerticalAlignment = VerticalAlignment.Center, Opacity = off ? 0.6 : 1 };
        Grid.SetColumn(tileView, 1);
        row.Children.Add(tileView);

        var text = new StackPanel { VerticalAlignment = VerticalAlignment.Center, Padding = new Thickness(0, 4, 0, 4) };
        text.Children.Add(new TextBlock
        {
            Text = title,
            FontSize = 13,
            FontWeight = session ? FontWeights.Medium : FontWeights.SemiBold,
            Foreground = selected ? Brush("textOnAccent") : off ? Brush("textSecondary") : Brush("textPrimary"),
            TextTrimming = TextTrimming.CharacterEllipsis,
            MaxLines = 1,
        });
        if (!string.IsNullOrEmpty(sub))
        {
            text.Children.Add(new TextBlock
            {
                Text = sub,
                FontSize = 11,
                LineHeight = 16,
                TextWrapping = TextWrapping.Wrap,
                Foreground = selected ? Brush("textOnAccent")
                    // The status inks as text, which hold 4.5:1 on the row's
                    // ground; the Mac draws statusAsk, 4.16:1 measured on the
                    // light row (box, round 2).
                    : flag == TreeFlag.Ask ? Brush("statusAskText")
                    : flag == TreeFlag.On ? Brush("statusOnText")
                    : Brush("textTertiary"),
            });
        }

        Grid.SetColumn(text, 2);
        row.Children.Add(text);

        // A row is named by its title and read with its sub line (ListRow.swift:264-266).
        AutomationProperties.SetName(row, title);
        AutomationProperties.SetHelpText(row, sub ?? string.Empty);
        AutomationProperties.SetAccessibilityView(row, AccessibilityView.Content);
        if (!session)
        {
            AutomationProperties.SetHeadingLevel(row, AutomationHeadingLevel.Level2);
        }

        return row;
    }

    /// <summary>Everything <paramref name="row"/> draws, by value (<see cref="TreeRowKeys"/>).</summary>
    private string Key(TreeRow row)
    {
        switch (row)
        {
            case TreeRow.FolderRow { Folder: var folder }:
                ViewModels.QueueGroupViewModel? group = _host.Group(folder.Id);
                return TreeRowKeys.Folder(
                    folder,
                    _state.Selection == new TreeSelection.Folder(folder.Id),
                    _state.Expanded.Contains(folder.Id),
                    group is null ? null : new TreeRowKeys.GroupFacts(group.ShowSubmitAll, group.OfferedCount, group.HasWithheldText ? group.WithheldText : null),
                    _host.AutomaticWrites.Writing.Contains(folder.Id),
                    _host.AutomaticWrites.Refusal(folder.Id),
                    _draggingFolder == folder.Id ? _dragOffset : null);
            case TreeRow.SessionRow { Entry: var entry, Off: var off }:
                ViewModels.QueueEntryViewModel? live = _host.Entry(entry.EntryId);
                return TreeRowKeys.Session(
                    entry,
                    off,
                    _state.Selection == new TreeSelection.Session(entry.EntryId),
                    live is null ? null : new TreeRowKeys.LiveFacts(
                        live.SizeBytes, live.ReasonCode, live.CanContribute, live.HasEligibilityText ? live.EligibilityText : null, live.WasTrimmed,
                        live.AttestationText, live.AttestationIsClear, live.AttestationIsAttention,
                        _state.MaySubmit(live.EntryId, live.CanContribute, live.StateCode, live.ReasonCode),
                        _host.IsTraceSubmitting(entry.EntryId)));
            default:
                throw new InvalidOperationException();
        }
    }

    private FrameworkElement FolderRow(TracesTree.FolderNode folder)
    {
        var id = new TreeSelection.Folder(folder.Id);
        bool selected = _state.Selection == id;
        bool off = folder.Mode == MapModes.Ignore;
        bool open = _state.Expanded.Contains(folder.Id);
        ViewModels.QueueGroupViewModel? group = _host.Group(folder.Id);
        string sub = TracesTreeRows.FolderSub(folder, _words, WatchCopy.ModeLabel);
        TreeRowGrid row = RowFrame(false, selected, folder.Label, sub, null, off, ToolTileKind.Folder);

        // The chevron, when the folder has traces (ListRow.swift:143-167).
        if (folder.Sessions.Count > 0)
        {
            var chevron = new FontIcon
            {
                Glyph = "",
                FontSize = 10,
                Foreground = selected ? Brush("textOnAccent") : Brush("statusOff"),
                RenderTransformOrigin = new Windows.Foundation.Point(0.5, 0.5),
                RenderTransform = new RotateTransform { Angle = open ? 90 : 0 },
            };
            var target = new Grid { Width = 28, Height = 40, Margin = new Thickness(-8, 0, -4, 0), Background = new SolidColorBrush(Color.FromArgb(0, 0, 0, 0)) };
            target.Children.Add(chevron);
            target.Tapped += (_, e) =>
            {
                e.Handled = true;
                _state.ToggleOpen(folder.Id);
                Render();
            };
            AutomationProperties.SetAccessibilityView(target, AccessibilityView.Raw);
            row.Children.Add(target);
        }

        // Submit, then the Automatic switch, side by side (TracesViews.swift:1195-1222).
        var accessory = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, VerticalAlignment = VerticalAlignment.Center };
        Grid.SetColumn(accessory, 3);
        row.Children.Add(accessory);

        // Submit: the shared table's offer, never for a folder that is not sent (TracesViews.swift:612-670).
        if (group is { ShowSubmitAll: true } && !off)
        {
            string help = TracesTreeRows.SubmitHelp(group.HasWithheldText ? group.WithheldText : null, _words);
            var split = new SplitButton
            {
                Content = TracesTreeRows.SubmitTitle(group.OfferedCount, _words),
                VerticalAlignment = VerticalAlignment.Center,
            };
            AutomationProperties.SetName(split, TracesTreeRows.SubmitTitle(group.OfferedCount, _words));
            AutomationProperties.SetHelpText(split, help);
            ToolTipService.SetToolTip(split, help);
            split.Click += (_, _) => _host.SubmitFolder(_host.Group(folder.Id) ?? group, null);
            var verdicts = new MenuFlyout();
            foreach ((string label, string outcome) in new[]
            {
                (VerdictCopy.Worked, Verdict.Worked), (VerdictCopy.Partly, Verdict.Partly), (VerdictCopy.Failed, Verdict.Failed),
            })
            {
                var item = new MenuFlyoutItem { Text = label };
                item.Click += (_, _) => _host.SubmitFolder(_host.Group(folder.Id) ?? group, outcome);
                verdicts.Items.Add(item);
            }

            split.Flyout = verdicts;
            accessory.Children.Add(split);
        }

        // The Automatic switch (owner, 2026-10-30; TracesViews.swift:438-451,
        // :639): on at Automatic, off at Ask, when the core offers the other
        // and has the switch's name. Off while its write is in flight.
        string automaticWord = _words["automatic_folder"];
        if (automaticWord.Length > 0 && FolderAutomatic.Switch(folder) is bool automatic)
        {
            string label = automaticWord.Replace("{label}", folder.Label, StringComparison.Ordinal);
            var toggle = new ToggleSwitch
            {
                IsOn = automatic,
                OnContent = string.Empty,
                OffContent = string.Empty,
                MinWidth = 0,
                VerticalAlignment = VerticalAlignment.Center,
                IsEnabled = !_host.AutomaticWrites.Writing.Contains(folder.Id),
            };
            AutomationProperties.SetName(toggle, label);
            ToolTipService.SetToolTip(toggle, label);
            toggle.Toggled += (_, _) =>
            {
                if (toggle.IsOn != automatic)
                {
                    _host.SetFolderAutomatic(folder, toggle.IsOn);
                }
            };
            accessory.Children.Add(toggle);
        }

        // The row menu: Ignore (TracesViews.swift:627-633, :856-878).
        if (group is not null && !off && folder.Mode is not null && folder.OfferableModes.Contains(MapModes.Ignore))
        {
            var kebab = Kebab(_words["folder_menu"], selected);
            var menu = new MenuFlyout();
            var ignore = new MenuFlyoutItem { Text = _words["ignore_folder"] };
            ignore.Click += (_, _) => _host.IgnoreFolder(_host.Group(folder.Id) ?? group);
            menu.Items.Add(ignore);
            kebab.Flyout = menu;
            Grid.SetColumn(kebab, 4);
            row.Children.Add(kebab);
        }

        var stack = new StackPanel { Spacing = 2 };
        stack.Children.Add(row);

        // The lines under a folder: why the bucket can never be armed, and what Submit leaves behind (:590-597, :642-647).
        var notes = new List<string>();
        if (folder.IsBucket && _words["unresolved_bucket_note"] is { Length: > 0 } note)
        {
            notes.Add(note);
        }

        if (_host.AutomaticWrites.Refusal(folder.Id) is { } refused)
        {
            notes.Add(refused);
        }

        if (!off && group is { HasWithheldText: true })
        {
            notes.Add(TracesTreeRows.WithheldNote(group.WithheldText, group.OfferedCount, _words));
        }

        foreach (string line in notes)
        {
            stack.Children.Add(new TextBlock
            {
                Text = line,
                FontSize = 11,
                LineHeight = 16,
                TextWrapping = TextWrapping.Wrap,
                Foreground = Brush("textTertiary"),
                Margin = new Thickness(TracesTreeRows.NoteInset, 0, TracesTreeRows.NoteInset, 4),
            });
        }

        // Selected by a click; dragged into place past 6 pt (TracesViews.swift:717-768).
        row.PointerPressed += (_, e) => OnFolderPressed(folder.Id, row, e);
        row.Tapped += (_, e) =>
        {
            if (_suppressClick)
            {
                _suppressClick = false;
                return;
            }

            e.Handled = true;
            Click(id);
        };
        if (_draggingFolder == folder.Id)
        {
            stack.RenderTransform = new TranslateTransform { Y = _dragOffset };
            Canvas.SetZIndex(stack, 1);
        }

        return stack;
    }

    private FrameworkElement SessionRow(MapEntry entry, bool off)
    {
        var id = new TreeSelection.Session(entry.EntryId);
        bool selected = _state.Selection == id;
        ViewModels.QueueEntryViewModel? live = _host.Entry(entry.EntryId);
        bool held = live is not null && TracesTreeRows.IsHeld(live.ReasonCode);
        string? ineligible = live is not null && !live.CanContribute && live.HasEligibilityText ? live.EligibilityText : null;
        TreeAttestationTone? tone = live is null ? null
            : live.AttestationIsClear ? TreeAttestationTone.Clear
            : live.AttestationIsAttention ? TreeAttestationTone.Attention
            : TreeAttestationTone.Plain;
        string? sub = TracesTreeRows.SessionSub(
            live?.SizeBytes, held, _words.Word("held"), ineligible, live?.WasTrimmed == true,
            live?.AttestationText, _words, CultureInfo.CurrentCulture);
        TreeFlag? flag = TracesTreeRows.Flag(held, ineligible is not null, tone);
        string title = TracesTreeRows.When(entry, TimeZoneInfo.Local, CultureInfo.CurrentCulture);
        TreeRowGrid row = RowFrame(true, selected, title, sub, flag, off, ToolTile.ForSession(entry.Source, entry.DeclaredSource));
        AutomationProperties.SetHelpText(row, string.Join(" ", new[] { sub, MapTools.Known(entry.DeclaredSource ?? entry.Source) ? MapTools.DisplayName(entry.DeclaredSource ?? entry.Source) : null }.Where(s => !string.IsNullOrEmpty(s))));

        if (live is not null && !_host.IsTraceSubmitting(entry.EntryId))
        {
            var submit = new Button
            {
                Content = _words["submit"],
                Style = (Style)Application.Current.Resources["TcSecondaryButtonStyle"],
                VerticalAlignment = VerticalAlignment.Center,
                IsEnabled = _state.MaySubmit(live.EntryId, live.CanContribute, live.StateCode, live.ReasonCode),
                IsTabStop = selected,
            };
            ToolTipService.SetToolTip(submit, _words["submit_trace_tip"]);
            submit.Click += (_, _) => _host.SubmitTrace(live);
            Grid.SetColumn(submit, 3);
            row.Children.Add(submit);
        }

        // The row menu: Dismiss trace, which asks first (TracesViews.swift:826-842, :890-922).
        if (live is not null)
        {
            var kebab = Kebab(_words["dismiss_session"], selected);
            var menu = new MenuFlyout();
            var dismiss = new MenuFlyoutItem { Text = _words["dismiss_session"] };
            dismiss.Click += (_, _) => _host.DismissTrace(_host.Entry(entry.EntryId) ?? live);
            menu.Items.Add(dismiss);
            kebab.Flyout = menu;
            Grid.SetColumn(kebab, 4);
            row.Children.Add(kebab);
        }

        row.Tapped += (_, e) =>
        {
            e.Handled = true;
            Click(id);
        };
        return row;
    }

    private Button Kebab(string name, bool selected)
    {
        var glyph = new FontIcon { Glyph = "", FontSize = 12 };
        if (TracesTreeRows.KebabInk(selected) is { } ink)
        {
            glyph.Foreground = Brush(ink);
        }

        var kebab = new Button
        {
            Content = glyph,
            Padding = new Thickness(0),
            Width = 22,
            Height = 22,
            Background = new SolidColorBrush(Color.FromArgb(0, 0, 0, 0)),
            BorderThickness = new Thickness(0),
            VerticalAlignment = VerticalAlignment.Center,
        };
        AutomationProperties.SetName(kebab, name);
        ToolTipService.SetToolTip(kebab, name);
        return kebab;
    }

    private void Click(TreeSelection row)
    {
        _state.Click(row);
        _host.Selected(_state.Selection);
        Render();
        Focus(FocusState.Programmatic);
    }

    // ---- The keyboard (TracesViews.swift:340-352, :522-543) ------------------

    private void OnKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.OriginalSource is TextBox)
        {
            return;
        }

        TreeSelection? before = _state.Selection;
        switch (e.Key)
        {
            case VirtualKey.Down:
            case VirtualKey.Up:
                _state.Move(e.Key == VirtualKey.Down);
                break;
            case VirtualKey.Left:
            case VirtualKey.Right:
                _state.Disclose(e.Key == VirtualKey.Right);
                break;
            case VirtualKey.Enter when _state.Selection is TreeSelection.Session session:
                _host.Review(session.EntryId);
                e.Handled = true;
                return;
            default:
                return;
        }

        e.Handled = true;
        if (_state.Selection != before)
        {
            _host.Selected(_state.Selection);
        }

        Render();
    }

    // ---- Keeping the selection on screen (TracesViews.swift:300-317, :500-506) ----

    private void ScrollTo(string rowId)
    {
        _state.Scrolled();
        if (!_rowById.TryGetValue(rowId, out FrameworkElement? row) || row.XamlRoot is null)
        {
            return;
        }

        double top = row.TransformToVisual(_rows).TransformPoint(new Windows.Foundation.Point(0, 0)).Y;
        double bottom = top + row.ActualHeight;
        double view = _scroller.ViewportHeight;
        double at = _scroller.VerticalOffset;
        double to = top < at ? top : bottom > at + view ? bottom - view : at;
        if (Math.Abs(to - at) < 0.5)
        {
            return;
        }

        if (!Motion)
        {
            _scroller.ChangeView(null, to, null, disableAnimation: true);
            return;
        }

        _scrollFrom = at;
        _scrollTo = to;
        _scrollAt = _clock.Elapsed.TotalSeconds;
        StartTicking();
    }

    // ---- Dragging a folder into place ------------------------------------

    private void OnFolderPressed(string folderId, UIElement row, PointerRoutedEventArgs e)
    {
        if (!e.GetCurrentPoint(row).Properties.IsLeftButtonPressed)
        {
            return;
        }

        _pressedFolder = folderId;
        _suppressClick = false;
        _pointer = e.Pointer.PointerId;
        _pressY = e.GetCurrentPoint(_rows).Position.Y;
    }

    private void OnRowsPointerMoved(object sender, PointerRoutedEventArgs e)
    {
        if (_pressedFolder is not { } folderId || e.Pointer.PointerId != _pointer
            || !e.GetCurrentPoint(_rows).Properties.IsLeftButtonPressed)
        {
            return;
        }

        double y = e.GetCurrentPoint(_rows).Position.Y;
        if (_draggingFolder is null)
        {
            if (Math.Abs(y - _pressY) < TracesTreeRows.FolderDragThreshold || Mid(folderId) is not { } mid)
            {
                return;
            }

            _draggingFolder = folderId;
            _dragStartMid = mid;
            _releaseAt = -1;
            _rows.CapturePointer(e.Pointer);
        }

        // Where the dragged folder's middle is under the pointer, against the slot it holds now (:735-757).
        double center = _dragStartMid + (y - _pressY);
        List<string> shown = _state.Shown.Folders.Select(f => f.Id).ToList();
        int index = shown.IndexOf(folderId);
        if (index >= 0)
        {
            (string, double)? above = index > 0 && Mid(shown[index - 1]) is { } a ? (shown[index - 1], a) : null;
            (string, double)? below = index + 1 < shown.Count && Mid(shown[index + 1]) is { } b ? (shown[index + 1], b) : null;
            if (TracesTreeRows.FolderSwap(center, above, below) is { } other && _state.MoveFolder(folderId, other))
            {
                Render();
                _rows.UpdateLayout();
            }
        }

        _dragOffset = center - (Mid(folderId) ?? center);
        SetDragOffset(_dragOffset);
        e.Handled = true;
    }

    private void OnRowsPointerReleased(object sender, PointerRoutedEventArgs e)
    {
        _pressedFolder = null;
        if (_draggingFolder is not null && _releaseAt < 0)
        {
            _suppressClick = true;
            _host.OrderChanged(_state.FolderOrder, _state.Arranged);
            EndDrag();
            _rows.ReleasePointerCapture(e.Pointer);
            e.Handled = true;
        }
    }

    /// <summary>The release: back into its slot on spring(0.3, 0.85), none under reduced motion (:760-767).</summary>
    private void EndDrag()
    {
        _pressedFolder = null;
        if (_draggingFolder is null || _releaseAt >= 0)
        {
            return;
        }

        if (!Motion)
        {
            SetDragOffset(0);
            _draggingFolder = null;
            return;
        }

        _releaseFrom = _dragOffset;
        _releaseAt = _clock.Elapsed.TotalSeconds;
        StartTicking();
    }

    private double? Mid(string folderId)
    {
        if (!_rowById.TryGetValue(new TreeSelection.Folder(folderId).RawValue, out FrameworkElement? row) || row.XamlRoot is null)
        {
            return null;
        }

        double offset = row.RenderTransform is TranslateTransform t ? t.Y : 0;
        double top = row.TransformToVisual(_rows).TransformPoint(new Windows.Foundation.Point(0, 0)).Y - offset;
        return top + row.ActualHeight / 2;
    }

    private void SetDragOffset(double offset)
    {
        _dragOffset = offset;
        if (_draggingFolder is { } id && _rowById.TryGetValue(new TreeSelection.Folder(id).RawValue, out FrameworkElement? row))
        {
            row.RenderTransform = new TranslateTransform { Y = offset };
            Canvas.SetZIndex(row, 1);
        }
    }

    // ---- One clock for the moves -----------------------------------------

    private void StartTicking()
    {
        if (!_clock.IsRunning)
        {
            _clock.Start();
        }

        if (!_ticking)
        {
            _ticking = true;
            CompositionTarget.Rendering += OnFrame;
        }
    }

    private void StopTicking()
    {
        if (_ticking)
        {
            _ticking = false;
            CompositionTarget.Rendering -= OnFrame;
        }
    }

    private readonly FocusFramePace<XamlRoot> _framePace = new(WindowClocks.Windows);

    private void OnFrame(object? sender, object e)
    {
        // The window's frame rule (FocusFrameRule): at most 15 a second while unfocused.
        if (_framePace.Skips(XamlRoot, GlassClock.Now))
        {
            return;
        }

        double now = _clock.Elapsed.TotalSeconds;
        bool moving = false;
        if (_searchAt >= 0)
        {
            double t = now - _searchAt;
            SetSearchFrame(SearchProgress());
            if (t >= TracesTreeRows.SearchFieldSlide.Duration)
            {
                _searchAt = -1;
                SetSearchFrame(_searchTo);
            }
            else
            {
                moving = true;
            }
        }

        if (_scrollAt >= 0)
        {
            double t = now - _scrollAt;
            double p = TracesTreeRows.ScrollToRow.At(t);
            _scroller.ChangeView(null, _scrollFrom + (_scrollTo - _scrollFrom) * p, null, disableAnimation: true);
            if (t >= TracesTreeRows.ScrollToRow.Duration)
            {
                _scrollAt = -1;
            }
            else
            {
                moving = true;
            }
        }

        if (_releaseAt >= 0)
        {
            double t = now - _releaseAt;
            SetDragOffset(_releaseFrom * (1 - TracesTreeRows.FolderDragRelease.At(t)));
            if (t >= TracesTreeRows.FolderDragRelease.Duration)
            {
                _releaseAt = -1;
                SetDragOffset(0);
                _draggingFolder = null;
            }
            else
            {
                moving = true;
            }
        }

        if (!moving)
        {
            StopTicking();
        }
    }

    protected override AutomationPeer OnCreateAutomationPeer() => new TreePeer(this);

    /// <summary>The tree as assistive tech reads it: one tree, named by the core (TracesViews.swift:337).</summary>
    private sealed class TreePeer : FrameworkElementAutomationPeer
    {
        public TreePeer(TracesTreeView owner)
            : base(owner)
        {
        }

        protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.Tree;

        protected override string GetClassNameCore() => nameof(TracesTreeView);
    }

    /// <summary>A row, read by its title with its sub line (ListRow.swift:262-273); not a tab stop.</summary>
    private sealed class TreeRowGrid : Grid
    {
        private readonly bool _session;

        public TreeRowGrid(bool session) => _session = session;

        protected override AutomationPeer OnCreateAutomationPeer() => new RowPeer(this, _session);

        private sealed class RowPeer : FrameworkElementAutomationPeer
        {
            private readonly bool _session;

            public RowPeer(TreeRowGrid owner, bool session)
                : base(owner) => _session = session;

            // Both levels are tree items; a folder carries heading level 2 as well.
            protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.TreeItem;

            protected override string GetClassNameCore() => _session ? "TracesTraceRow" : "TracesFolderRow";
        }
    }

    // ---- Inks ----------------------------------------------------------------

    /// <summary>A generated ink for this theme; the system's in High Contrast.</summary>
    private SolidColorBrush Brush(string token)
    {
        if (_accessibility.HighContrast)
        {
            UIElementType type = token switch
            {
                "selection" => UIElementType.Highlight,
                "textOnAccent" => UIElementType.HighlightText,
                "rowHover" => UIElementType.Window,
                _ => UIElementType.WindowText,
            };
            return new SolidColorBrush(_ui.UIElementColor(type));
        }

        IReadOnlyDictionary<string, MapRgba> inks = ThemeBrushes.Dark ? MapArt.Dark : MapArt.Light;
        MapRgba c = inks.TryGetValue(token, out MapRgba found) ? found : inks["textPrimary"];
        return new SolidColorBrush(Color.FromArgb((byte)Math.Round(255 * Math.Clamp(c.A, 0, 1)), c.R, c.G, c.B));
    }
}
