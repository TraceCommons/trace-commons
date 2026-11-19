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
using Windows.System;
using Windows.UI.ViewManagement;
using Color = Windows.UI.Color;

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

    /// <summary>The selection moved: the inspector shows it.</summary>
    void Selected(TreeSelection? selection);

    /// <summary>Return on a selected trace: its review.</summary>
    void Review(string entryId);

    void SubmitFolder(ViewModels.QueueGroupViewModel group, string? outcome);

    void IgnoreFolder(ViewModels.QueueGroupViewModel group);

    void SubmitTrace(ViewModels.QueueEntryViewModel entry);

    void DismissTrace(ViewModels.QueueEntryViewModel entry);

    /// <summary>The person's folder order changed: keep it.</summary>
    void OrderChanged(IReadOnlyList<string> order, bool arranged);
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
    private readonly Grid _searchField = new();
    private readonly TextBox _searchBox = new();
    private readonly TranslateTransform _searchShift = new();
    private readonly Dictionary<string, FrameworkElement> _rowById = new(StringComparer.Ordinal);
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
        _root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        _root.Children.Add(Heading());

        _content.Children.Add(_rows);
        _content.Children.Add(_footer);
        _scroller.Content = _content;
        _scroller.HorizontalScrollMode = ScrollMode.Disabled;
        _scroller.HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled;
        _scroller.IsTabStop = false;
        _scroller.Padding = new Thickness(14, 0, 14, 12);
        Grid.SetRow(_scroller, 1);
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
        ActualThemeChanged += (_, _) => Render();
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

    /// <summary>The queue's own rows changed (sizes, holds, offers): draw them again.</summary>
    public void Refresh() => Render();

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
            Grid.SetColumn(order, 2);
            heading.Children.Add(order);
        }

        return heading;
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

    private void Render()
    {
        _rows.Children.Clear();
        _rowById.Clear();
        IReadOnlyList<TreeRow> rows = _state.Rows;
        foreach (TreeRow row in rows)
        {
            FrameworkElement drawn = row switch
            {
                TreeRow.FolderRow f => FolderRow(f.Folder),
                TreeRow.SessionRow s => SessionRow(s.Entry, s.Off),
                _ => throw new InvalidOperationException(),
            };
            _rowById[row.Id] = drawn;
            _rows.Children.Add(drawn);
        }

        if (_state.Shown.Folders.Count == 0 && _state.Search.Trim().Length > 0)
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
            split.Click += (_, _) => _host.SubmitFolder(group, null);
            var verdicts = new MenuFlyout();
            foreach ((string label, string outcome) in new[]
            {
                (VerdictCopy.Worked, Verdict.Worked), (VerdictCopy.Partly, Verdict.Partly), (VerdictCopy.Failed, Verdict.Failed),
            })
            {
                var item = new MenuFlyoutItem { Text = label };
                item.Click += (_, _) => _host.SubmitFolder(group, outcome);
                verdicts.Items.Add(item);
            }

            split.Flyout = verdicts;
            Grid.SetColumn(split, 3);
            row.Children.Add(split);
        }

        // The row menu: Ignore (TracesViews.swift:627-633, :856-878).
        if (group is not null && !off && folder.Mode is not null && folder.OfferableModes.Contains(MapModes.Ignore))
        {
            var kebab = Kebab(_words["folder_menu"]);
            var menu = new MenuFlyout();
            var ignore = new MenuFlyoutItem { Text = _words["ignore_folder"] };
            ignore.Click += (_, _) => _host.IgnoreFolder(group);
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

        // Submit sends it at once, as a folder's does; turned off while it cannot be sent (TracesViews.swift:785-805).
        var submit = new Button
        {
            Content = _words["submit"],
            Style = (Style)Application.Current.Resources["TcSecondaryButtonStyle"],
            VerticalAlignment = VerticalAlignment.Center,
            IsEnabled = live is { CanContribute: true } && !off,
            IsTabStop = selected,
        };
        ToolTipService.SetToolTip(submit, _words["submit_trace_tip"]);
        if (live is not null)
        {
            submit.Click += (_, _) => _host.SubmitTrace(live);
        }

        Grid.SetColumn(submit, 3);
        row.Children.Add(submit);

        // The row menu: Dismiss trace, which asks first (TracesViews.swift:826-842, :890-922).
        if (live is not null)
        {
            var kebab = Kebab(_words["dismiss_session"]);
            var menu = new MenuFlyout();
            var dismiss = new MenuFlyoutItem { Text = _words["dismiss_session"] };
            dismiss.Click += (_, _) => _host.DismissTrace(live);
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

    private Button Kebab(string name)
    {
        var kebab = new Button
        {
            Content = new FontIcon { Glyph = "", FontSize = 12 },
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

    private void OnFrame(object? sender, object e)
    {
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

        IReadOnlyDictionary<string, MapRgba> inks = ActualTheme == ElementTheme.Dark ? MapArt.Dark : MapArt.Light;
        MapRgba c = inks.TryGetValue(token, out MapRgba found) ? found : inks["textPrimary"];
        return new SolidColorBrush(Color.FromArgb((byte)Math.Round(255 * Math.Clamp(c.A, 0, 1)), c.R, c.G, c.B));
    }
}
