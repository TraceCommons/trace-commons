using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Linq;
using System.Runtime.InteropServices;
using Microsoft.UI;
using Microsoft.UI.Composition;
using Microsoft.UI.Composition.SystemBackdrops;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Input;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Hosting;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using NoticeRule = TraceCommons.Interop.Shell.NoticeClose;
using TraceCommons.App.Controls;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;
using Windows.Graphics;
using Windows.System;
using Windows.UI.ViewManagement;

namespace TraceCommons.App;

/// <summary>
/// The Monitor window's shell: the toolbar row in the title bar, the tabs,
/// the breadcrumb, the panes and their growth at the screen edge, inspector
/// demand, the notice stack, the keyboard map and the clocks. Every decision
/// is one of the pure rules in <c>TraceCommons.Interop.Shell</c>, tested
/// against the Mac; this file only applies them to the window.
/// </summary>
public sealed partial class MainWindow
{
    /// <summary>What is on screen in the main pane.</summary>
    private MonitorLocation _at = MonitorLocation.Default;

    /// <summary>The person's tab order (Home always first). Kept for the run:
    /// this shell has no preference store yet.</summary>
    private string _tabOrder = string.Empty;

    /// <summary>The person's pane choices, before availability.</summary>
    private bool _wantsInspector;

    /// <summary>The panes the window is laid out for, after availability.</summary>
    private PaneVisibility _presented = new(false, false);

    /// <summary>A pane move under way, if any (the Mac's <c>transition</c>).</summary>
    private PaneTransition? _transition;

    /// <summary>The main pane's kept width beside the map.</summary>
    private double _keptMain = MonitorPaneLayout.MainWidth;

    /// <summary>Counts pane moves, so an overtaken animation stops.</summary>
    private int _moveGeneration;

    private DispatcherQueueTimer? _moveTimer;

    private readonly InspectorDemandTracker _demand = new();

    private readonly WindowClockGate _clockGate = new();

    /// <summary>Cards closed with their X until the window opens again.</summary>
    private readonly HashSet<string> _closedNotices = new(StringComparer.Ordinal);

    /// <summary>Every card this stack has held, so one revealed by a close
    /// fades in place and one that has just arrived slides in.</summary>
    private readonly HashSet<string> _seenNotices = new(StringComparer.Ordinal);

    private NoticeKind? _frontNotice;

    private readonly List<(MonitorTab Tab, ToggleButton Button, Rectangle Rule)> _tabs = new();

    private SettingsWindow? _settingsWindow;

    private readonly UISettings _uiSettings = new();

    private bool _seeded;

    private void InitializeShell()
    {
        Title = MonitorWords.AppName;

        // The toolbar row is the title bar: the window owns the bar, the
        // caption buttons stay the system's (snap layouts, window menu, their
        // own accessibility), on a transparent ground so the chrome colour
        // runs behind them.
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);
        AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Tall;
        AppWindow.TitleBar.ButtonBackgroundColor = Colors.Transparent;
        AppWindow.TitleBar.ButtonInactiveBackgroundColor = Colors.Transparent;

        TitleMark.Data = NearMarkGeometry.Build();

        string channel = MonitorWords.ChannelLabel;
        ChannelTag.Text = channel;
        ChannelTag.Visibility = channel.Length == 0 ? Visibility.Collapsed : Visibility.Visible;

        AutomationProperties.SetName(TabStrip, MonitorWords.TabsLabel);
        // The Mac's round Settings button is named with settings_title
        // (MonitorWindowView.swift:963).
        AutomationProperties.SetName(SettingsGear, MonitorWords.SettingsTitle);
        ToolTipService.SetToolTip(SettingsGear, MonitorWords.SettingsTitle);
        TracesHistoryButton.Content = MonitorWords.CrumbTitle(new Crumb(CrumbWord.History, MonitorTab.Traces, false));
        HomeOpenTraces.Content = MonitorShellCopy.Top("open_traces");
        HomeOpenMissionDrafts.Content = MonitorWords.CrumbTitle(new Crumb(CrumbWord.Missions, MonitorTab.Home, false));

        BuildTabs();
        RegisterAccelerators();
        ApplyWindowMaterial();

        AppTitleBar.SizeChanged += (_, _) => UpdateTitleBarRegions();
        AppTitleBar.Loaded += (_, _) => UpdateTitleBarRegions();
        ToolbarCapsule.SizeChanged += (_, _) => UpdateTitleBarRegions();
        AppWindow.Changed += OnAppWindowChanged;
        _clockGate.Changed += OnClocksChanged;

        ViewModel.PropertyChanged += OnShellViewModelChanged;
        ViewModel.LegacyOffer.PropertyChanged += (_, _) => RefreshNotices();
        ViewModel.GrantVoidCards.CollectionChanged += (_, _) => RefreshNotices();
        ViewModel.ArmingRewordingCards.CollectionChanged += (_, _) => RefreshNotices();
        Activated += OnShellFirstActivated;

        RefreshLocation();
    }

    // ---- Tabs, breadcrumb and location ------------------------------------

    /// <summary>
    /// Puts the tabs in the person's order, Home first, and gives each its
    /// title and accessible name: the tab's own title, whole, from the core,
    /// never its accelerator or a tooltip (checklist E1).
    /// </summary>
    private void BuildTabs()
    {
        var all = new Dictionary<MonitorTab, (ToggleButton Button, TextBlock Label, Rectangle Rule)>
        {
            [MonitorTab.Home] = (TabHome, TabHomeLabel, TabHomeRule),
            [MonitorTab.Inference] = (TabInference, TabInferenceLabel, TabInferenceRule),
            [MonitorTab.Traces] = (TabTraces, TabTracesLabel, TabTracesRule),
            [MonitorTab.Insights] = (TabInsights, TabInsightsLabel, TabInsightsRule),
        };
        TabStrip.Children.Clear();
        _tabs.Clear();
        foreach (MonitorTab tab in MonitorNavigation.Ordered(_tabOrder))
        {
            (ToggleButton button, TextBlock label, Rectangle rule) = all[tab];
            string title = MonitorWords.TabTitle(tab);
            label.Text = title;
            AutomationProperties.SetName(button, title);
            TabStrip.Children.Add(button);
            _tabs.Add((tab, button, rule));
        }

        TabHome.Click += (_, _) => OpenTab(MonitorTab.Home);
        TabInference.Click += (_, _) => OpenTab(MonitorTab.Inference);
        TabTraces.Click += (_, _) => OpenTab(MonitorTab.Traces);
        TabInsights.Click += (_, _) => OpenTab(MonitorTab.Insights);
    }

    /// <summary>Ctrl+1..4 by position, Ctrl+comma, Ctrl+Shift+M (spec W2-D5 (a)).</summary>
    private void RegisterAccelerators()
    {
        WindowRoot.KeyboardAccelerators.Clear();
        VirtualKey[] digits = { VirtualKey.Number1, VirtualKey.Number2, VirtualKey.Number3, VirtualKey.Number4 };
        for (int i = 0; i < digits.Length; i++)
        {
            int digit = i + 1;
            var accelerator = new KeyboardAccelerator { Key = digits[i], Modifiers = VirtualKeyModifiers.Control };
            accelerator.Invoked += (_, args) =>
            {
                args.Handled = true;
                if (MonitorNavigation.TabForDigit(digit, _tabOrder) is MonitorTab tab)
                {
                    OpenTab(tab);
                }
            };
            WindowRoot.KeyboardAccelerators.Add(accelerator);
        }

        // VK_OEM_COMMA (0xBC): Settings, as Cmd-comma on the Mac
        // (TraceCommonsAppMain.swift:435).
        var settings = new KeyboardAccelerator { Key = (VirtualKey)0xBC, Modifiers = VirtualKeyModifiers.Control };
        settings.Invoked += (_, args) =>
        {
            args.Handled = true;
            OnShowSettings(SettingsGear, new RoutedEventArgs());
        };
        WindowRoot.KeyboardAccelerators.Add(settings);

        var privateAI = new KeyboardAccelerator { Key = VirtualKey.M, Modifiers = VirtualKeyModifiers.Control | VirtualKeyModifiers.Shift };
        privateAI.Invoked += OnTogglePrivateInferenceAccelerator;
        WindowRoot.KeyboardAccelerators.Add(privateAI);

        // The accelerators belong to the window, not to any control: no
        // tooltip announces them, and no control's name is derived from one.
        WindowRoot.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
    }

    /// <summary>Opens a tab at its first page, through the existing routes.</summary>
    private void OpenTab(MonitorTab tab)
    {
        switch (tab)
        {
            case MonitorTab.Home:
                ViewModel.ShowHome();
                break;
            case MonitorTab.Inference:
                OnShowPrivateInference(this, new RoutedEventArgs());
                break;
            case MonitorTab.Traces:
                OnShowQueue(this, new RoutedEventArgs());
                break;
            case MonitorTab.Insights:
                ShowInsightsPane();
                break;
        }

        // A click on the tab already shown leaves its page; the Mac's tab
        // keeps its page too. The checked state is the window's, not the
        // button's own toggle.
        RefreshLocation();
    }

    /// <summary>The location the view model's pane is.</summary>
    private MonitorLocation LocationFromViewModel()
    {
        if (ViewModel.ShowingMissionDrafts)
        {
            return _at with { Tab = MonitorTab.Home, HomePage = HomePage.MissionDrafts };
        }

        if (ViewModel.ShowingHome)
        {
            return _at with { Tab = MonitorTab.Home, HomePage = HomePage.Overview };
        }

        if (ViewModel.ShowingPrivateInference)
        {
            return _at with { Tab = MonitorTab.Inference };
        }

        if (ViewModel.ShowingHistory)
        {
            return _at with { Tab = MonitorTab.Traces, TracesPage = TracesPage.History };
        }

        if (ViewModel.ShowingInsights)
        {
            return _at with { Tab = MonitorTab.Insights };
        }

        return _at with { Tab = MonitorTab.Traces, TracesPage = TracesPage.Tree };
    }

    private void RefreshLocation()
    {
        _at = LocationFromViewModel();
        foreach ((MonitorTab tab, ToggleButton button, Rectangle rule) in _tabs)
        {
            bool shown = tab == _at.Tab;
            button.IsChecked = shown;
            rule.Visibility = shown ? Visibility.Visible : Visibility.Collapsed;
        }

        RenderBreadcrumb();
        RefreshNotices();
        ApplyPaneAvailability();
    }

    private void RenderBreadcrumb()
    {
        Breadcrumb.Children.Clear();
        IReadOnlyList<Crumb>? crumbs = MonitorNavigation.Breadcrumb(_at, ViewModel.MissionsShown);
        if (crumbs is null)
        {
            Breadcrumb.Visibility = Visibility.Collapsed;
            return;
        }

        for (int i = 0; i < crumbs.Count; i++)
        {
            Crumb crumb = crumbs[i];
            string word = MonitorWords.CrumbTitle(crumb);
            if (i > 0)
            {
                // The chevron between crumbs is a glyph, not a word.
                Breadcrumb.Children.Add(new FontIcon
                {
                    Glyph = "",
                    FontSize = 10,
                    VerticalAlignment = VerticalAlignment.Center,
                    Foreground = (Brush)Application.Current.Resources["TcInkSecondaryBrush"],
                    IsTabStop = false,
                });
            }

            if (crumb.GoesBack)
            {
                var back = new HyperlinkButton { Content = word, Padding = new Thickness(0) };
                AutomationProperties.SetName(back, word);
                back.Click += (_, _) => GoBack(crumb);
                Breadcrumb.Children.Add(back);
            }
            else
            {
                Breadcrumb.Children.Add(new TextBlock
                {
                    Text = word,
                    VerticalAlignment = VerticalAlignment.Center,
                    Style = (Style)Application.Current.Resources["TcTitleCardTextStyle"],
                });
            }
        }

        Breadcrumb.Visibility = Visibility.Visible;
    }

    private void GoBack(Crumb crumb)
    {
        MonitorLocation to = MonitorNavigation.Back(_at, crumb);
        if (to.Tab == MonitorTab.Home)
        {
            ViewModel.ShowHome();
        }
        else
        {
            ViewModel.ShowQueue();
        }
    }

    private void OnShellViewModelChanged(object? sender, PropertyChangedEventArgs e)
    {
        switch (e.PropertyName)
        {
            case nameof(MainViewModel.ShowingQueue):
            case nameof(MainViewModel.ShowingHistory):
            case nameof(MainViewModel.ShowingPrivateInference):
            case nameof(MainViewModel.ShowingInsights):
            case nameof(MainViewModel.ShowingMissionDrafts):
            case nameof(MainViewModel.ShowingHome):
            case nameof(MainViewModel.MissionsShown):
                RefreshLocation();
                break;
            case nameof(MainViewModel.Demand):
            case nameof(MainViewModel.HasPrivateInferenceOffer):
            case nameof(MainViewModel.HasArmingOffer):
                RefreshNotices();
                ApplyDemand();
                break;
            case nameof(MainViewModel.IsInQueueFolder):
                ApplyPaneAvailability();
                break;
            case null:
            case nameof(MainViewModel.HasUndo):
            case nameof(MainViewModel.HasNotice):
            case nameof(MainViewModel.HasHealthBanner):
            case nameof(MainViewModel.HasWitnessBanner):
            case nameof(MainViewModel.HasGateHeldBanner):
            case nameof(MainViewModel.HasLegacyMigrationNotice):
            case nameof(MainViewModel.IsUpdateBannerVisible):
            case nameof(MainViewModel.ShowFirstContribution):
                RefreshNotices();
                break;
        }
    }

    // ---- Inspector demand -------------------------------------------------

    /// <summary>
    /// Opens the inspector when something new asks for it; never closes it
    /// (MonitorShell.swift:43-66).
    /// </summary>
    private void ApplyDemand()
    {
        var keys = new SortedSet<string>(InspectorDemand.Keys(ViewModel.Demand), StringComparer.Ordinal);
        string? arming = ViewModel.HasArmingOffer ? ViewModel.ArmingOfferProjectId : null;
        keys.UnionWith(InspectorDemand.OfferKeys(_at.Tab, ViewModel.HasPrivateInferenceOffer, arming));
        if (_demand.Update(keys) && !_wantsInspector)
        {
            _wantsInspector = true;
            ApplyPaneAvailability();
        }
    }

    /// <summary>
    /// The folder's sessions are the inspector's in this slice, so opening a
    /// folder opens the inspector: a closed one would show nothing for the
    /// click (the Mac's map-folder click does the same; its tree shows a
    /// folder's sessions in place, which W3 brings).
    /// </summary>
    private void OpenInspectorForFolder()
    {
        if (!_wantsInspector)
        {
            _wantsInspector = true;
            ApplyPaneAvailability();
        }
    }

    private void OnInspectorToggleClick(object sender, RoutedEventArgs e)
    {
        _wantsInspector = !_presented.Inspector;
        ApplyPaneAvailability();
    }

    // ---- Panes and the window's growth ------------------------------------

    /// <summary>
    /// What the panes should be on this tab: the person's choice where the
    /// inspector is available (MonitorWindowView.swift:303). No map until W3.
    /// </summary>
    private void ApplyPaneAvailability()
    {
        bool available = MonitorNavigation.InspectorAvailable(_at.Tab, _at.HomePage);
        InspectorToggle.IsEnabled = available;
        var wanted = new PaneVisibility(false, _wantsInspector && available);
        InspectorToggle.IsChecked = wanted.Inspector;
        string toggleName = MonitorWords.Toggle(PaneToggle.Inspector, wanted.Inspector);
        AutomationProperties.SetName(InspectorToggle, toggleName);
        ToolTipService.SetToolTip(InspectorToggle, toggleName);
        MovePanes(wanted);
    }

    private double Scale => Content?.XamlRoot?.RasterizationScale ?? 1.0;

    private bool IsMaximized =>
        AppWindow.Presenter is OverlappedPresenter presenter && presenter.State == OverlappedPresenterState.Maximized;

    /// <summary>
    /// Moves to <paramref name="next"/>: grows or shrinks the window by the
    /// pane that opened or closed and slides it left at the work area's edge,
    /// on the standard curve unless animations are off or the main pane is
    /// squeezed (ThreePane.swift:519-566, MonitorWindowGrowth).
    /// </summary>
    private void MovePanes(PaneVisibility next)
    {
        PaneVisibility old = _transition?.To ?? _presented;
        if (old == next)
        {
            return;
        }

        int generation = ++_moveGeneration;
        _moveTimer?.Stop();
        if (IsMaximized || PaneRow.ActualWidth <= 0)
        {
            _transition = null;
            _presented = next;
            LayoutPanes();
            return;
        }

        double clientWidth = PaneRow.ActualWidth;
        var outer = new LayoutRect(AppWindow.Position.X, AppWindow.Position.Y, AppWindow.Size.Width, AppWindow.Size.Height);
        RectInt32 work = DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Nearest).WorkArea;
        var workArea = new LayoutRect(work.X, work.Y, work.Width, work.Height);
        PaneMove move = MonitorWindowGrowth.Plan(
            outer, clientWidth, workArea, Scale, old, next, _keptMain, MonitorPaneLayout.DefaultMapWidth, _uiSettings.AnimationsEnabled);
        _keptMain = move.Main;

        if (!move.Animates)
        {
            _transition = null;
            _presented = next;
            ApplyMinimumSize();
            MoveAndResize(move.Frame);
            LayoutPanes();
            return;
        }

        _transition = new PaneTransition(old, next, MonitorPaneLayout.For(clientWidth, old.Map, old.Inspector, _keptMain).Map, _keptMain);
        ApplyMinimumSize();
        LayoutPanes();
        var start = Stopwatch.StartNew();
        _moveTimer ??= DispatcherQueue.CreateTimer();
        _moveTimer.Interval = TimeSpan.FromMilliseconds(15);
        _moveTimer.IsRepeating = true;
        _moveTimer.Tick += Step;
        _moveTimer.Start();

        void Step(DispatcherQueueTimer timer, object _)
        {
            if (generation != _moveGeneration)
            {
                timer.Tick -= Step;
                return;
            }

            double progress = Math.Min(1, start.Elapsed.TotalSeconds / MonitorMotion.Standard);
            MoveAndResize(MonitorWindowGrowth.Interpolate(outer, move.Frame, progress));
            if (progress >= 1)
            {
                timer.Stop();
                timer.Tick -= Step;
                _transition = null;
                _presented = next;
                ApplyMinimumSize();
                LayoutPanes();
            }
        }
    }

    private void MoveAndResize(LayoutRect frame) =>
        AppWindow.MoveAndResize(new RectInt32((int)frame.X, (int)frame.Y, (int)frame.Width, (int)frame.Height));

    /// <summary>Sets the columns from the Mac's arithmetic at the row's width.</summary>
    private void LayoutPanes()
    {
        double width = PaneRow.ActualWidth;
        MonitorPaneLayout layout = _transition is PaneTransition transition
            ? MonitorPaneLayout.For(width, transition)
            : MonitorPaneLayout.For(width, _presented.Map, _presented.Inspector, _keptMain);
        MainColumn.Width = new GridLength(layout.Main);
        MapGapColumn.Width = new GridLength(layout.Map is null ? 0 : layout.MapGap);
        MapColumn.Width = new GridLength(layout.Map ?? 0);
        InspectorGapColumn.Width = new GridLength(layout.Inspector is null ? 0 : MonitorPaneLayout.PaneGap);
        InspectorColumn.Width = new GridLength(layout.Inspector ?? 0);
        MapPane.Visibility = Visibility.Collapsed;
        InspectorPane.Visibility = layout.Inspector is null ? Visibility.Collapsed : Visibility.Visible;
        InspectorPane.Opacity = _transition is PaneTransition t && !t.To.Inspector ? 0 : 1;
    }

    private void OnPaneRowSizeChanged(object sender, SizeChangedEventArgs e) => LayoutPanes();

    /// <summary>
    /// The first time the window is shown: the panes its work area has room
    /// for (ThreePane.swift:208-210, MonitorWindowView.swift:835-842), never
    /// closing the inspector over something it must show, and the window
    /// sized to them on its own monitor.
    /// </summary>
    private void OnShellFirstActivated(object sender, WindowActivatedEventArgs args)
    {
        Activated -= OnShellFirstActivated;
        if (_seeded)
        {
            return;
        }

        _seeded = true;
        RectInt32 work = DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Nearest).WorkArea;
        double scale = Scale;
        PaneVisibility seed = MonitorPaneLayout.FirstLaunch(work.Width / scale);
        _wantsInspector = seed.Inspector || InspectorDemand.OpensOnAppear(InspectorDemand.Keys(ViewModel.Demand));

        bool inspector = _wantsInspector && MonitorNavigation.InspectorAvailable(_at.Tab, _at.HomePage);
        double width = MonitorPaneLayout.WindowWidth(showsMap: false, showsInspector: inspector);
        double frameExtra = AppWindow.Size.Width - AppWindow.ClientSize.Width;
        double height = MonitorPaneLayout.WindowHeight * scale + (AppWindow.Size.Height - AppWindow.ClientSize.Height);
        var outer = new LayoutRect(AppWindow.Position.X, AppWindow.Position.Y, AppWindow.Size.Width, AppWindow.Size.Height);
        LayoutRect frame = MonitorPaneLayout.WindowFrame(
            outer, MonitorPaneLayout.ToPhysical(width, scale) + frameExtra, new LayoutRect(work.X, work.Y, work.Width, work.Height));
        if (!IsMaximized)
        {
            MoveAndResize(frame with { Height = Math.Min(height, work.Height) });
        }

        _presented = new PaneVisibility(false, inspector);
        ApplyMinimumSize();
        LayoutPanes();
        ApplyDemand();
    }

    // ---- Title bar regions -------------------------------------------------

    /// <summary>
    /// The caption buttons' room, from the title bar itself, and the regions
    /// that take the pointer instead of dragging the window, in physical
    /// pixels (InputNonClientPointerSource). Recomputed on every size,
    /// layout and DPI change of the row.
    /// </summary>
    private void UpdateTitleBarRegions()
    {
        if (Content?.XamlRoot is null)
        {
            return;
        }

        double scale = Scale;
        RightInsetColumn.Width = new GridLength(AppWindow.TitleBar.RightInset / scale);
        LeftInsetColumn.Width = new GridLength(AppWindow.TitleBar.LeftInset / scale);

        var rects = new List<LayoutRect>();
        foreach (FrameworkElement element in new FrameworkElement[] { ToolbarCapsule, SettingsGear })
        {
            if (element.Visibility != Visibility.Visible || element.ActualWidth <= 0)
            {
                continue;
            }

            Windows.Foundation.Rect bounds = element.TransformToVisual(null)
                .TransformBounds(new Windows.Foundation.Rect(0, 0, element.ActualWidth, element.ActualHeight));
            rects.Add(new LayoutRect(bounds.X, bounds.Y, bounds.Width, bounds.Height));
        }

        RectInt32[] regions = TitleBarRegions.Passthrough(rects, scale)
            .Select(r => new RectInt32(r.X, r.Y, r.Width, r.Height))
            .ToArray();
        InputNonClientPointerSource.GetForWindowId(AppWindow.Id).SetRegionRects(NonClientRegionKind.Passthrough, regions);
    }

    // ---- Clocks ------------------------------------------------------------

    private void OnAppWindowChanged(AppWindow sender, AppWindowChangedEventArgs args)
    {
        if (args.DidPresenterChange || args.DidSizeChange)
        {
            bool minimised = sender.Presenter is OverlappedPresenter presenter
                && presenter.State == OverlappedPresenterState.Minimized;
            _clockGate.SetMinimised(minimised);
        }

        if (args.DidSizeChange)
        {
            UpdateTitleBarRegions();
        }
    }

    /// <summary>
    /// Stops the window's own clocks while it cannot be seen: the undo
    /// countdown, the visibility debounce, a pane move under way and the
    /// managed-sessions poll (through <see cref="WindowClocks"/>). <c>AppWindow.Hide</c> raises no
    /// visibility change, so hiding to the tray reports itself to the gate.
    /// </summary>
    private void OnClocksChanged(bool mayTick)
    {
        ViewModel.SetClocksRunning(mayTick);
        WindowClocks.Set(mayTick);
        if (!mayTick)
        {
            _visibilityDebounceTimer.Stop();
            _moveTimer?.Stop();
        }
    }

    // ---- The minimum window ------------------------------------------------

    /// <summary>
    /// The narrowest window for the panes shown (ThreePane.swift:143-146,
    /// :151-156 while a move is under way), and the Mac's minimum height,
    /// enforced through WM_GETMINMAXINFO: Windows App SDK 1.6's presenter has
    /// no minimum size of its own.
    /// </summary>
    private void ApplyMinimumSize()
    {
        double minimum = _transition is PaneTransition transition
            ? MonitorPaneLayout.WindowWidthLowerLimit(transition)
            : MonitorPaneLayout.MinimumWindowWidth(_presented.Map, _presented.Inspector);
        double scale = Scale;
        int extraWidth = AppWindow.Size.Width - AppWindow.ClientSize.Width;
        int extraHeight = AppWindow.Size.Height - AppWindow.ClientSize.Height;
        WindowMinimumSize.Set(
            WinRT.Interop.WindowNative.GetWindowHandle(this),
            MonitorPaneLayout.ToPhysical(minimum, scale) + extraWidth,
            MonitorPaneLayout.ToPhysical(MonitorPaneLayout.WindowMinHeight, scale) + extraHeight);
    }

    // ---- The window's material (W5) ----------------------------------------

    /// <summary>
    /// The hook for W5's material (W5-D1 (b): Mica where supported, painted
    /// glass otherwise). This slice paints the window's own ground
    /// (WindowRoot's background) and records whether Mica is supported, so
    /// W5 lands the material here without moving anything else.
    /// </summary>
    private void ApplyWindowMaterial()
    {
        Debug.WriteLine($"monitor: mica supported={MicaController.IsSupported()}");
    }

    // ---- Settings window ---------------------------------------------------

    private void ShowSettingsWindow()
    {
        if (_settingsWindow is null)
        {
            var settings = new SettingsView(_host, ViewModel.LegacyOffer);
            // Settings no longer holds the model-calls switch; it holds a
            // pointer at the destination that does, and this is what turns
            // that pointer into navigation.
            settings.OpenPrivateInferenceRequested += (_, _) =>
            {
                ShowPrivateInferencePane();
                BringForward();
            };
            // Set up again unenrolled this device: the first run starts,
            // with a fresh NEAR AI sign-in, as on a device never set up.
            settings.SetUpAgainRequested += async (_, _) =>
            {
                OpenOnboarding(OnboardingState.Default()).Activate();
                await ViewModel.RefreshAsync();
            };
            _settingsWindow = new SettingsWindow(settings);
            _settingsWindow.Closed += (_, _) => _settingsWindow = null;
        }

        _settingsWindow.ShowNearOwner(AppWindow);
    }

    // ---- Notices -----------------------------------------------------------

    /// <summary>The cards present now, each with its element.</summary>
    private IEnumerable<(NoticeKind Kind, FrameworkElement Card)> PresentNotices()
    {
        if (ViewModel.IsUpdateBannerVisible) yield return (NoticeCatalog.Update, UpdateCard);
        if (ViewModel.HasLegacyMigrationNotice) yield return (NoticeCatalog.LegacyNotice, LegacyNoticeCard);
        if (ViewModel.LegacyOffer.ShowsCard) yield return (NoticeCatalog.LegacyOffer, LegacyOfferCard);
        if (ViewModel.ShowFirstContribution) yield return (NoticeCatalog.FirstContribution, FirstContributionCard);
        if (ViewModel.HasArmingOffer) yield return (NoticeCatalog.ArmingOffer(ViewModel.ArmingOfferProjectId), ArmingOfferCard);
        if (ViewModel.ArmingRewordingCards.Count > 0) yield return (new NoticeKind("reworded", NoticeGroup.TracesNow, NoticeRule.None), RewordingCards);
        if (ViewModel.GrantVoidCards.Count > 0) yield return (new NoticeKind("void", NoticeGroup.TracesNow, NoticeRule.None), VoidCards);
        if (ViewModel.HasGateHeldBanner) yield return (NoticeCatalog.GateHeld, GateHeldCard);
        if (ViewModel.HasWitnessBanner) yield return (NoticeCatalog.WitnessCapacity, WitnessCard);
        if (ViewModel.HasHealthBanner) yield return (NoticeCatalog.Health, HealthCard);
        if (ViewModel.HasUndo) yield return (NoticeCatalog.Undo, UndoCard);
        if (ViewModel.HasPrivateInferenceOffer) yield return (NoticeCatalog.PrivateAIOffer, PrivateAIOfferCard);
        if (ViewModel.HasNotice) yield return (NoticeCatalog.ActionNotice, ActionNoticeCard);
    }

    private static IEnumerable<FrameworkElement> AllCards(MainWindow w) => new FrameworkElement[]
    {
        w.UpdateCard, w.LegacyNoticeCard, w.LegacyOfferCard, w.FirstContributionCard, w.ArmingOfferCard,
        w.RewordingCards, w.VoidCards, w.GateHeldCard, w.WitnessCard, w.HealthCard, w.UndoCard,
        w.PrivateAIOfferCard, w.ActionNoticeCard,
    };

    /// <summary>
    /// Shows the front card of this place's stack and the plates behind it;
    /// every other card collapsed (NoticeCatalog, NoticeStack).
    /// </summary>
    private void RefreshNotices()
    {
        if (NoticeStackHost is null)
        {
            return;
        }

        var present = PresentNotices().ToList();
        var elements = present.ToDictionary(p => p.Kind.Id, p => p.Card, StringComparer.Ordinal);
        IReadOnlyList<NoticeKind> shown = NoticeCatalog.Shown(NoticeMonitorPlace(), present.Select(p => p.Kind), _closedNotices);
        foreach (FrameworkElement card in AllCards(this))
        {
            card.Visibility = Visibility.Collapsed;
        }

        if (shown.Count == 0)
        {
            _frontNotice = null;
            NoticeStackHost.Visibility = Visibility.Collapsed;
            return;
        }

        NoticeKind front = shown[^1];
        bool arrived = NoticeStack.Slides(front.Id, _seenNotices);
        _seenNotices.UnionWith(shown.Select(card => card.Id));
        FrameworkElement frontCard = elements[front.Id];
        frontCard.Visibility = Visibility.Visible;
        if (front != _frontNotice && arrived)
        {
            Arrive(frontCard);
        }

        _frontNotice = front;
        NoticeStackHost.Visibility = Visibility.Visible;

        // The X: none for a card that goes only with its cause.
        NoticeClose.Visibility = front.Close == NoticeRule.None ? Visibility.Collapsed : Visibility.Visible;
        string closeName = front.Close == NoticeRule.Declines
            ? ViewModel.PrivateInferenceOfferDecline
            : front.Id == NoticeCatalog.LegacyOffer.Id ? ViewModel.LegacyOffer.Dismiss : MonitorWords.Close;
        AutomationProperties.SetName(NoticeClose, closeName);
        ToolTipService.SetToolTip(NoticeClose, closeName);

        LayoutPlates(shown.Select(card => card.Id).ToArray());
    }

    private NoticePlace NoticeMonitorPlace() => MonitorNavigation.Place(_at);

    private void OnNoticeFrontSizeChanged(object sender, SizeChangedEventArgs e) => RefreshPlatesOnly();

    private void RefreshPlatesOnly()
    {
        var present = PresentNotices().Select(p => p.Kind);
        LayoutPlates(NoticeCatalog.Shown(NoticeMonitorPlace(), present, _closedNotices).Select(card => card.Id).ToArray());
    }

    /// <summary>The plates behind the front card, its size, peeking below it.</summary>
    private void LayoutPlates(IReadOnlyList<string> ids)
    {
        IReadOnlyList<NoticePlate> plates = NoticeStack.Plates(ids, StackAnchor.Top);
        Border[] borders = { NoticePlate1, NoticePlate2 };
        double width = NoticeFront.ActualWidth;
        double height = NoticeFront.ActualHeight;
        for (int i = 0; i < borders.Length; i++)
        {
            Border plate = borders[i];
            if (i >= plates.Count || height <= 0)
            {
                plate.Visibility = Visibility.Collapsed;
                continue;
            }

            NoticePlate p = plates[i];
            plate.Height = height;
            plate.Opacity = p.Opacity;
            plate.RenderTransform = new CompositeTransform
            {
                ScaleX = p.ScaleX,
                CenterX = width / 2,
                TranslateY = p.OffsetY,
            };
            plate.Visibility = Visibility.Visible;
        }

        NoticeStackHost.Padding = new Thickness(0, 0, 0, NoticeStack.PlateRoom(ids));
    }

    /// <summary>
    /// A new card slides in from the top over the one it covers, on the
    /// Mac's spring (response 0.36 s, damping 0.86; PopoverNotice.swift:238),
    /// and fades in. Nothing moves with animations off.
    /// </summary>
    private void Arrive(FrameworkElement card)
    {
        if (!_uiSettings.AnimationsEnabled)
        {
            return;
        }

        Visual visual = ElementCompositionPreview.GetElementVisual(card);
        Compositor compositor = visual.Compositor;
        SpringScalarNaturalMotionAnimation slide = compositor.CreateSpringScalarAnimation();
        slide.DampingRatio = (float)NoticeStack.SpringDamping;
        slide.Period = TimeSpan.FromSeconds(NoticeStack.SpringResponse);
        slide.InitialValue = -(float)Math.Max(24, card.ActualHeight);
        slide.FinalValue = 0f;
        ElementCompositionPreview.SetIsTranslationEnabled(card, true);
        visual.StartAnimation("Translation.Y", slide);
        ScalarKeyFrameAnimation fade = compositor.CreateScalarKeyFrameAnimation();
        fade.InsertKeyFrame(0f, 0f);
        fade.InsertKeyFrame(1f, 1f);
        fade.Duration = TimeSpan.FromSeconds(NoticeStack.SpringResponse);
        visual.StartAnimation("Opacity", fade);
    }

    /// <summary>What the front card's X does, by the card's rule.</summary>
    private async void OnNoticeCloseClick(object sender, RoutedEventArgs e)
    {
        if (_frontNotice is not NoticeKind front)
        {
            return;
        }

        switch (front.Id)
        {
            case "undo":
                OnLetItSend(sender, e);
                break;
            case "action-notice":
                ViewModel.ClearNotice();
                break;
            case "legacy-notice":
                OnAcknowledgeLegacyMigration(sender, e);
                break;
            case "legacy-offer":
                ViewModel.LegacyOffer.DismissCard();
                break;
            case "private-ai-offer":
                OnDeclinePrivateInference(sender, e);
                break;
            default:
                _closedNotices.Add(front.Id);
                break;
        }

        await System.Threading.Tasks.Task.Yield();
        RefreshNotices();
    }
}

/// <summary>The N mark as a XAML geometry, from <see cref="NearMarkPath.Unit"/>.</summary>
internal static class NearMarkGeometry
{
    public static Geometry Build()
    {
        var geometry = new PathGeometry();
        PathFigure? figure = null;
        foreach (PathStep s in NearMarkPath.Unit)
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
}

/// <summary>
/// The window's minimum size through WM_GETMINMAXINFO, by subclassing the
/// window procedure (comctl32 SetWindowSubclass).
/// </summary>
internal static class WindowMinimumSize
{
    private const uint WM_GETMINMAXINFO = 0x0024;
    private static readonly Dictionary<IntPtr, (int Width, int Height)> Minimums = new();
    private static readonly SubclassProc Proc = WndProc;
    private static readonly HashSet<IntPtr> Installed = new();

    private delegate IntPtr SubclassProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam, UIntPtr id, UIntPtr data);

    [DllImport("comctl32.dll", SetLastError = true)]
    private static extern bool SetWindowSubclass(IntPtr hwnd, SubclassProc proc, UIntPtr id, UIntPtr data);

    [DllImport("comctl32.dll")]
    private static extern IntPtr DefSubclassProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);

    [StructLayout(LayoutKind.Sequential)]
    private struct Point
    {
        public int X;
        public int Y;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MinMaxInfo
    {
        public Point Reserved;
        public Point MaxSize;
        public Point MaxPosition;
        public Point MinTrackSize;
        public Point MaxTrackSize;
    }

    public static void Set(IntPtr hwnd, int width, int height)
    {
        Minimums[hwnd] = (width, height);
        if (Installed.Add(hwnd))
        {
            SetWindowSubclass(hwnd, Proc, (UIntPtr)1, UIntPtr.Zero);
        }
    }

    private static IntPtr WndProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam, UIntPtr id, UIntPtr data)
    {
        if (msg == WM_GETMINMAXINFO && Minimums.TryGetValue(hwnd, out (int Width, int Height) minimum))
        {
            var info = Marshal.PtrToStructure<MinMaxInfo>(lParam);
            info.MinTrackSize.X = minimum.Width;
            info.MinTrackSize.Y = minimum.Height;
            Marshal.StructureToPtr(info, lParam, false);
            return IntPtr.Zero;
        }

        return DefSubclassProc(hwnd, msg, wParam, lParam);
    }
}
