using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using Microsoft.UI;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Animation;
using Microsoft.UI.Xaml.Shapes;
using TraceCommons.Interop;
using TraceCommons.Interop.Generated;
using TraceCommons.Interop.Shell;
using Windows.UI;
using Windows.UI.ViewManagement;
using TraceCommons.App.Look;

namespace TraceCommons.App.Controls;

/// <summary>
/// Home's overview (W11): the Mac's <c>HomeOverview</c>
/// (macos/Sources/TraceCommonsApp/Views/Monitor/HomeViews.swift:150-554) as
/// this shell can draw it. What it draws, and what it leaves out, is
/// <see cref="HomeOverviewModel"/>'s; this control lays that out and owns the
/// clocks: the hero glyphs at 10 fps, the coin burst once a display frame,
/// and the Balance figure's count-up in 24 steps of 50 ms. Every clock runs
/// only while Home is on screen in a window that can be seen, and none runs
/// while the system's animations are off, when each shows its last frame.
/// </summary>
public sealed class HomeView : UserControl
{
    private readonly Grid _root = new();
    private readonly StackPanel _column = new() { Spacing = HomeTrayInks.Measures.CardGap };
    private readonly Canvas _burstLayer = new() { IsHitTestVisible = false };
    private readonly DispatcherQueueTimer _glyphTimer;
    private readonly DispatcherQueueTimer _countTimer;
    private readonly DispatcherQueueTimer _burstEnd;
    private readonly List<HeroGlyphView> _glyphs = new();
    private readonly Stopwatch _burstClock = new();
    private HomeOverviewModel? _model;
    private string? _drawnTheme;
    private TextBlock? _balanceValue;
    private FrameworkElement? _balanceTile;
    private IReadOnlyList<long> _countSteps = Array.Empty<long>();
    private int _countIndex;
    private byte? _countScale;
    private CoinBurstView? _burst;
    private bool _burstTicking;
    private bool _loaded;

    public HomeView()
    {
        DispatcherQueue queue = DispatcherQueue.GetForCurrentThread();
        _glyphTimer = queue.CreateTimer();
        _glyphTimer.Interval = GlyphClock.Frame;
        _glyphTimer.Tick += (_, _) =>
        {
            if (MayTick)
            {
                DrawGlyphs();
            }
            else
            {
                UpdateClocks();
            }
        };
        _countTimer = queue.CreateTimer();
        _countTimer.Interval = BalanceArrival.CountFrame;
        _countTimer.Tick += (_, _) => CountStep();
        _burstEnd = queue.CreateTimer();
        _burstEnd.IsRepeating = false;
        _burstEnd.Tick += (_, _) => EndBurst();

        var scroll = new ScrollViewer
        {
            Content = _column,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            Padding = new Thickness(22, 24, 22, 22),
        };
        _root.Children.Add(scroll);
        _root.Children.Add(_burstLayer);
        Content = _root;
        IsTabStop = false;

        // The window's visibility and the system's animation setting are
        // followed while Home is loaded, and let go when it is unloaded:
        // both are static events, which would otherwise hold this view.
        Loaded += (_, _) =>
        {
            _loaded = true;
            WindowClocks.Changed -= OnWindowClocks;
            WindowClocks.Changed += OnWindowClocks;
            WindowClocks.Windows.Changed -= OnWindowFocus;
            WindowClocks.Windows.Changed += OnWindowFocus;
            SystemSettingWatch.Changed -= OnSystemSettings;
            SystemSettingWatch.Changed += OnSystemSettings;
            ThemeBrushes.Changed -= OnSystemSettings;
            ThemeBrushes.Changed += OnSystemSettings;
            OnSystemSettings();
        };
        Unloaded += (_, _) =>
        {
            _loaded = false;
            WindowClocks.Changed -= OnWindowClocks;
            WindowClocks.Windows.Changed -= OnWindowFocus;
            SystemSettingWatch.Changed -= OnSystemSettings;
            ThemeBrushes.Changed -= OnSystemSettings;
            UpdateClocks();
            EndBurst();
        };
        IsEnabledChanged += (_, _) => UpdateClocks();
    }

    private void OnWindowClocks(bool running) => DispatcherQueue.TryEnqueue(UpdateClocks);

    /// <summary>The window's focus changed: its clocks' rate follows (FocusFrameRule).</summary>
    private void OnWindowFocus(XamlRoot root, bool mayTick)
    {
        if (ReferenceEquals(root, XamlRoot))
        {
            DispatcherQueue.TryEnqueue(UpdateClocks);
        }
    }

    /// <summary>
    /// The clocks follow the motion setting; the cards, whose colours are
    /// picked in code, are drawn again when the theme or a high contrast
    /// theme changes (ThemeBrushes).
    /// </summary>
    private void OnSystemSettings()
    {
        UpdateClocks();
        if (_model is HomeOverviewModel model && _drawnTheme != ThemeBrushes.Key)
        {
            _model = null;
            Show(model);
        }
    }

    /// <summary>Where a tile or a hero leads; the window opens it.</summary>
    public event Action<HomeLink>? LinkRequested;

    /// <summary>The campaign hero's X: closes it as it is now.</summary>
    public event Action? CampaignClosed;

    /// <summary>Whether Home is the page on screen; set by the window as the page changes.</summary>
    public bool Showing
    {
        get => _showing;
        set
        {
            _showing = value;
            UpdateClocks();
        }
    }

    private bool _showing;

    private bool AnimationsOn => SystemSettingWatch.AnimationsEnabled;

    /// <summary>
    /// Whether a clock may run: loaded, the page shown, the window neither
    /// hidden nor minimised (<see cref="WindowClocks"/>), and the system's
    /// animations on (checklist P1).
    /// </summary>
    private bool MayTick => _loaded && _showing && WindowClocks.Running && AnimationsOn;

    private readonly ContentControl _trainingSlot = new()
    {
        HorizontalContentAlignment = HorizontalAlignment.Stretch,
        IsTabStop = false,
        Visibility = Visibility.Collapsed,
    };

    /// <summary>
    /// The sent-traces training line under the Insights card, or nothing
    /// (<c>SentTracesTrainingLine</c>, SentTracesTrainingViews.swift:95-127).
    /// </summary>
    public void SetTrainingLine(FrameworkElement? line)
    {
        _trainingSlot.Content = line;
        _trainingSlot.Visibility = line is null ? Visibility.Collapsed : Visibility.Visible;
    }

    /// <summary>Draws <paramref name="model"/>. Only what changed is rebuilt; the glyphs keep their clock.</summary>
    public void Show(HomeOverviewModel model)
    {
        if (model.Same(_model) && _drawnTheme == ThemeBrushes.Key)
        {
            return;
        }

        _model = model;
        _drawnTheme = ThemeBrushes.Key;
        _column.Children.Clear();
        _glyphs.Clear();
        _balanceValue = null;
        _balanceTile = null;

        if (model.PrivateAI is HomePrivateAI privateAI)
        {
            AddPrivateAI(privateAI);
        }

        AddSection(model.TracesHeading, model.TraceTiles, HomeLink.Traces, balanceIndex: -1, model.NoCredit is HomeNotice notice ? BuildNotice(notice) : null);
        AddSection(model.AccountHeading, model.AccountTiles, HomeLink.PrivateAI, balanceIndex: 2,
            underHeading: model.AccountReadPending ? BuildAccountLoading() : null,
            accessory: model.AccountSignIn is HomeAccountSignIn signIn ? BuildAccountSignIn(signIn) : null);

        foreach (HomeHero hero in model.Heroes)
        {
            _column.Children.Add(BuildHero(hero, model));
        }

        // Sent traces without Train coding models, and Review, under the
        // Insights card while there are any (HomeViews.swift:270-273).
        _column.Children.Add(_trainingSlot);

        if (model.Catalogue is HomeCard catalogue)
        {
            _column.Children.Add(BuildHostedCard(catalogue));
        }

        DrawGlyphs();
        UpdateClocks();
    }

    private void AddSection(string heading, IReadOnlyList<HomeTile> tiles, HomeLink link, int balanceIndex, UIElement? underHeading = null, UIElement? accessory = null)
    {
        if (heading.Length > 0)
        {
            var title = new TextBlock
            {
                Text = heading,
                Style = (Style)Application.Current.Resources["TcTitleSectionTextStyle"],
                Margin = new Thickness(0, HomeTrayInks.Measures.S4, 0, 0),
            };
            if (accessory is null)
            {
                _column.Children.Add(title);
            }
            else
            {
                // Just after the title, on its line (HomeSection's accessory,
                // HomeViews.swift:725-748).
                var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = HomeTrayInks.Measures.S5 };
                title.VerticalAlignment = VerticalAlignment.Bottom;
                row.Children.Add(title);
                row.Children.Add(accessory);
                _column.Children.Add(row);
            }
        }

        if (underHeading is not null)
        {
            _column.Children.Add(underHeading);
        }

        if (tiles.Count == 0)
        {
            return;
        }

        var grid = new Grid { ColumnSpacing = HomeTrayInks.Measures.S3 };
        for (int i = 0; i < tiles.Count; i++)
        {
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            HomeLink target = link;
            if (link == HomeLink.Traces && i == 1)
            {
                // Contributed opens History (HomeViews.swift:262-263).
                target = HomeLink.History;
            }

            FrameworkElement tile = BuildTile(tiles[i], target, out TextBlock value);
            Grid.SetColumn(tile, i);
            grid.Children.Add(tile);
            if (i == balanceIndex)
            {
                _balanceValue = value;
                _balanceTile = tile;
            }
        }

        _column.Children.Add(grid);
    }

    private static UIElement BuildAccountLoading()
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = HomeTrayInks.Measures.S3 };
        var ring = new ProgressRing { IsActive = true, Width = 16, Height = 16, VerticalAlignment = VerticalAlignment.Center };
        AutomationProperties.SetAccessibilityView(ring, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        row.Children.Add(ring);
        row.Children.Add(new TextBlock
        {
            Text = MonitorShellCopy.Section("history_actions", "checking_account"),
            Style = (Style)Application.Current.Resources["TcCaptionTextStyle"],
            Foreground = ThemeBrushes.Get("TcInkSecondaryBrush"),
            VerticalAlignment = VerticalAlignment.Center,
        });
        return row;
    }

    /// <summary>
    /// Beside the Account heading while its figures wait on a sign-in
    /// (<c>HomeAccountSignIn</c>, HomeViews.swift:626-649): the core's
    /// "No data", a dash the screen reader skips, then Sign in as a link,
    /// which begins the NEAR AI sign-in as the Private AI section's does.
    /// </summary>
    private UIElement BuildAccountSignIn(HomeAccountSignIn signIn)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = HomeTrayInks.Measures.S2, VerticalAlignment = VerticalAlignment.Bottom };
        row.Children.Add(new TextBlock
        {
            Text = signIn.NoData,
            Style = (Style)Application.Current.Resources["TcCaptionTextStyle"],
            Foreground = ThemeBrushes.Get("TcInkSecondaryBrush"),
            VerticalAlignment = VerticalAlignment.Center,
        });
        var dash = new TextBlock
        {
            Text = "\u2013",
            Style = (Style)Application.Current.Resources["TcCaptionTextStyle"],
            Foreground = ThemeBrushes.Get("TcInkTertiaryBrush"),
            VerticalAlignment = VerticalAlignment.Center,
        };
        AutomationProperties.SetAccessibilityView(dash, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        row.Children.Add(dash);
        var link = new HyperlinkButton { Content = signIn.SignIn, Padding = new Thickness(0), VerticalAlignment = VerticalAlignment.Center };
        if (signIn.SignInName is string name)
        {
            AutomationProperties.SetName(link, name);
        }

        link.Click += (_, _) => LinkRequested?.Invoke(HomeLink.PrivateAISignIn);
        row.Children.Add(link);
        return row;
    }

    /// <summary>
    /// Home's Private AI section (HomeViews.swift:466-490), smaller than
    /// Traces: signed in, one slim row of the calls and how many are verified
    /// that opens the Private AI tab (<c>HomePrivateAIRow</c>); signed out,
    /// the core's line and Sign in (<c>HomePrivateAISignInCard</c>).
    /// </summary>
    private void AddPrivateAI(HomePrivateAI section)
    {
        _column.Children.Add(new TextBlock
        {
            Text = section.Heading,
            Style = (Style)Application.Current.Resources["TcTitleSectionTextStyle"],
            Margin = new Thickness(0, HomeTrayInks.Measures.S4, 0, 0),
        });
        if (section.Figures is IReadOnlyList<HomeTile> figures)
        {
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = HomeTrayInks.Measures.S5 * 2 };
            foreach (HomeTile figure in figures)
            {
                var pair = new StackPanel { Orientation = Orientation.Horizontal, Spacing = HomeTrayInks.Measures.S3 };
                pair.Children.Add(new TextBlock { Text = figure.Value, Style = (Style)Application.Current.Resources["TcTitleCardTextStyle"], VerticalAlignment = VerticalAlignment.Center });
                pair.Children.Add(new TextBlock { Text = figure.Label, Style = (Style)Application.Current.Resources["TcCaptionTextStyle"], VerticalAlignment = VerticalAlignment.Center });
                row.Children.Add(pair);
            }

            var button = new Button
            {
                Content = row,
                HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Left,
                Padding = new Thickness(12, 10, 12, 10),
                CornerRadius = new CornerRadius(HomeTrayInks.Measures.CardRadius),
                Background = ThemeBrushes.Get("TcSurfaceCardBrush"),
            };
            AutomationProperties.SetName(button, section.Heading);
            AutomationProperties.SetHelpText(button, string.Join(", ", figures.Select(f => f.Value + " " + f.Label)));
            button.Click += (_, _) => LinkRequested?.Invoke(HomeLink.PrivateAI);
            _column.Children.Add(button);
        }
        else if (section.SignIn is HomeNotice signIn)
        {
            var card = new Grid
            {
                ColumnSpacing = HomeTrayInks.Measures.S4,
                Padding = new Thickness(HomeTrayInks.Measures.CardPaddingHorizontal, 10, HomeTrayInks.Measures.CardPaddingHorizontal, 10),
                CornerRadius = new CornerRadius(HomeTrayInks.Measures.CardRadius),
                Background = ThemeBrushes.Get("TcSurfaceCardBrush"),
            };
            card.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            card.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            card.Children.Add(new TextBlock
            {
                Text = signIn.Line,
                Style = (Style)Application.Current.Resources["TcBodyTextStyle"],
                TextWrapping = TextWrapping.WrapWholeWords,
                VerticalAlignment = VerticalAlignment.Center,
            });
            var button = new Button
            {
                Content = signIn.Action,
                Style = (Style)Application.Current.Resources["TcSecondaryButtonStyle"],
                VerticalAlignment = VerticalAlignment.Center,
            };
            if (section.SignInName is string name)
            {
                AutomationProperties.SetName(button, name);
            }

            button.Click += (_, _) => LinkRequested?.Invoke(HomeLink.PrivateAISignIn);
            Grid.SetColumn(button, 1);
            card.Children.Add(button);
            _column.Children.Add(card);
        }
    }

    /// <summary>
    /// The no-credit reminder (<c>NoCreditNotice</c>, NoCreditNotice.swift):
    /// one slim line on the ask status at 12%, never red, the ask mark (the
    /// filled circle with its exclamation, as the Mac's), the
    /// core's sentence, and Open Settings at its end. No X: it goes once the
    /// use is on.
    /// </summary>
    private UIElement BuildNotice(HomeNotice notice)
    {
        var row = new Grid
        {
            ColumnSpacing = HomeTrayInks.Measures.S3,
            Padding = new Thickness(HomeTrayInks.Measures.S5, HomeTrayInks.Measures.S3, HomeTrayInks.Measures.S5, HomeTrayInks.Measures.S3),
            CornerRadius = new CornerRadius(HomeTrayInks.Measures.ControlRadius),
            Background = new SolidColorBrush(Faded(ThemeBrushes.Get("TcStatusAskBrush"), 0.12)),
        };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var mark = new FontIcon
        {
            Glyph = "\uEB90",
            FontSize = 14,
            Foreground = ThemeBrushes.Get("TcStatusAskBrush"),
            VerticalAlignment = VerticalAlignment.Center,
        };
        AutomationProperties.SetAccessibilityView(mark, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        row.Children.Add(mark);
        var line = new TextBlock
        {
            Text = notice.Line,
            Style = (Style)Application.Current.Resources["TcBodyTextStyle"],
            TextWrapping = TextWrapping.WrapWholeWords,
            VerticalAlignment = VerticalAlignment.Center,
        };
        Grid.SetColumn(line, 1);
        row.Children.Add(line);
        var open = new Button
        {
            Content = notice.Action,
            Style = (Style)Application.Current.Resources["TcSecondaryButtonStyle"],
            VerticalAlignment = VerticalAlignment.Center,
        };
        AutomationProperties.SetHelpText(open, notice.Line);
        open.Click += (_, _) => LinkRequested?.Invoke(HomeLink.Settings);
        Grid.SetColumn(open, 2);
        row.Children.Add(open);
        return row;
    }

    /// <summary>
    /// A way into a screen (<c>hostedCard</c>, HomeViews.swift:313-321):
    /// the title over the core's subtitle on the card fill, and a chevron;
    /// the whole card opens the screen.
    /// </summary>
    private FrameworkElement BuildHostedCard(HomeCard card)
    {
        var grid = new Grid { ColumnSpacing = HomeTrayInks.Measures.S4 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var words = new StackPanel { Spacing = HomeTrayInks.Measures.S1 };
        words.Children.Add(new TextBlock { Text = card.Title, Style = (Style)Application.Current.Resources["TcTitleCardTextStyle"] });
        if (card.Subtitle is string subtitle)
        {
            words.Children.Add(new TextBlock
            {
                Text = subtitle,
                Style = (Style)Application.Current.Resources["TcCaptionTextStyle"],
                TextWrapping = TextWrapping.WrapWholeWords,
            });
        }

        grid.Children.Add(words);
        var chevron = new FontIcon { Glyph = "\uE76C", FontSize = 12, VerticalAlignment = VerticalAlignment.Center };
        AutomationProperties.SetAccessibilityView(chevron, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        Grid.SetColumn(chevron, 1);
        grid.Children.Add(chevron);
        var button = new Button
        {
            Content = grid,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            Padding = new Thickness(HomeTrayInks.Measures.CardPaddingHorizontal, 12, HomeTrayInks.Measures.CardPaddingHorizontal, 12),
            CornerRadius = new CornerRadius(HomeTrayInks.Measures.CardRadius),
            Background = ThemeBrushes.Get("TcSurfaceCardBrush"),
        };
        AutomationProperties.SetName(button, card.Title);
        if (card.Subtitle is string help)
        {
            AutomationProperties.SetHelpText(button, help);
        }

        button.Click += (_, _) => LinkRequested?.Invoke(card.Link);
        return button;
    }

    private static Color Faded(Brush brush, double alpha)
    {
        Color c = brush is SolidColorBrush solid ? solid.Color : Microsoft.UI.Colors.Transparent;
        return Color.FromArgb((byte)Math.Round(255 * alpha * c.A / 255.0), c.R, c.G, c.B);
    }

    /// <summary>
    /// A stat tile (<c>GlassStatCard</c> through <c>HomeStatTile</c>): the
    /// eyebrow word over the bold figure, the line under it; no icon (Ron,
    /// 2026-11-05). The whole tile is the button to where its number lives.
    /// </summary>
    private FrameworkElement BuildTile(HomeTile tile, HomeLink link, out TextBlock value)
    {
        var stack = new StackPanel { Spacing = HomeTrayInks.Measures.S2 };
        stack.Children.Add(new TextBlock
        {
            Text = tile.Label,
            Style = (Style)Application.Current.Resources["TcEyebrowTextStyle"],
            TextWrapping = TextWrapping.WrapWholeWords,
        });
        value = new TextBlock { Text = tile.Value, Style = (Style)Application.Current.Resources["TcMetricValueTextStyle"] };
        stack.Children.Add(value);
        if (tile.Detail is { Length: > 0 } detail)
        {
            stack.Children.Add(new TextBlock
            {
                Text = detail,
                Style = (Style)Application.Current.Resources["TcCaptionTextStyle"],
                TextWrapping = TextWrapping.WrapWholeWords,
            });
        }

        var button = new Button
        {
            Content = stack,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Left,
            VerticalAlignment = VerticalAlignment.Stretch,
            Padding = new Thickness(12, 10, 12, 10),
            CornerRadius = new CornerRadius(HomeTrayInks.Measures.CardRadius),
            Background = ThemeBrushes.Get("TcSurfaceCardBrush"),
        };

        // Named by its visible label, its figure the help text (checklist E1).
        AutomationProperties.SetName(button, tile.Label);
        AutomationProperties.SetHelpText(button, tile.Detail is { Length: > 0 } d ? tile.Value + " " + d : tile.Value);
        button.Click += (_, _) => LinkRequested?.Invoke(link);
        return button;
    }

    /// <summary>
    /// A hero card (<c>GlassHeroCard</c>, TCDesign/Components/HeroCard.swift):
    /// a solid navy ground, the pixel glyph tilted -12 degrees and running
    /// past the trailing and bottom edges, the title, the line, the white
    /// pill, and for the campaign the X.
    /// </summary>
    private FrameworkElement BuildHero(HomeHero hero, HomeOverviewModel model)
    {
        bool highContrast = SystemSettingWatch.HighContrast;
        HeroGround ground = hero.Ground switch
        {
            HomeGround.Campaign => HeroGround.Campaign,
            HomeGround.Missions => HeroGround.Missions,
            _ => HeroGround.Insights,
        };
        Color inkColor = HeroColor(HeroPart.Ink, ground, highContrast);
        var ink = new SolidColorBrush(inkColor);
        var card = new Grid
        {
            MinHeight = HeroCard.MinHeight,
            CornerRadius = new CornerRadius(HomeTrayInks.Measures.CardRadius),
            Background = new SolidColorBrush(HeroColor(HeroPart.Ground, ground, highContrast)),

            // Under high contrast the card is outlined in the ink, as the
            // tiles above it are.
            BorderBrush = highContrast ? ink : null,
            BorderThickness = new Thickness(highContrast ? 1 : 0),
        };

        // The graphic, behind the words, past the card's edges.
        var art = new Canvas { IsHitTestVisible = false };
        card.Children.Add(art);
        if (!HeroCard.DrawsArt(highContrast))
        {
            // No art under high contrast: decoration in fixed blues.
        }
        else if (hero.Symbol is string symbol && HeroGlyph.HeroMask(symbol) is HeroGlyph.Mask mask)
        {
            var glyph = new HeroGlyphView(mask, art);
            _glyphs.Add(glyph);
            card.SizeChanged += (_, e) => glyph.Place(e.NewSize.Width, e.NewSize.Height);
        }
        else if (hero.Ground == HomeGround.Insights && model.InsightsBars.Count > 0)
        {
            var bars = BuildInsightsBars(model.InsightsBars, inkColor);
            art.Children.Add(bars);
            card.SizeChanged += (_, e) =>
            {
                bars.Measure(new Windows.Foundation.Size(double.PositiveInfinity, double.PositiveInfinity));
                Canvas.SetLeft(bars, e.NewSize.Width - HeroCard.GraphicWidth + 30);
                Canvas.SetTop(bars, e.NewSize.Height - bars.DesiredSize.Height + 40);
            };
        }

        // The hover wash: heroInk at 6%, faded in and out over the fast
        // token on the token curve (HeroCard.swift:113-115, :146).
        var wash = new Border { Background = ink, Opacity = 0, IsHitTestVisible = false };
        card.Children.Add(wash);

        var words = new StackPanel
        {
            Spacing = HomeTrayInks.Measures.S3,
            Padding = new Thickness(HomeTrayInks.Measures.CardPaddingHorizontal),
            Margin = new Thickness(0, 0, HeroCard.GraphicReserve, 0),
        };
        var titles = new StackPanel { Spacing = HomeTrayInks.Measures.S2 };
        titles.Children.Add(new TextBlock
        {
            Text = hero.Title,
            Foreground = ink,

            // The display type, heavy (HeroCard.swift:105-106).
            FontSize = HomeTrayInks.Measures.DisplaySize,
            FontWeight = FontWeights.ExtraBold,
            TextWrapping = TextWrapping.WrapWholeWords,
        });
        if (hero.Subtitle is string subtitle)
        {
            titles.Children.Add(new TextBlock
            {
                Text = subtitle,
                Foreground = ink,
                Opacity = HeroCard.SubtitleInk,
                TextWrapping = TextWrapping.WrapWholeWords,
                FontSize = HomeTrayInks.Measures.LabelSize,
            });
        }

        words.Children.Add(titles);
        if (hero.Ground == HomeGround.Insights && model.InsightsLine is string line)
        {
            words.Children.Add(new TextBlock
            {
                Text = line,
                Foreground = ink,
                Opacity = model.InsightsBars.Count == 0 ? HeroCard.ContentInk : 1,
                FontWeight = model.InsightsBars.Count == 0 ? FontWeights.SemiBold : FontWeights.Bold,
                FontSize = model.InsightsBars.Count == 0 ? HomeTrayInks.Measures.LabelSize : HomeTrayInks.Measures.HeadingSize,
                TextWrapping = TextWrapping.WrapWholeWords,
            });
        }

        if (hero.Action is string action)
        {
            words.Children.Add(new Border
            {
                Background = new SolidColorBrush(HeroColor(HeroPart.PillFill, ground, highContrast)),
                BorderBrush = highContrast ? ink : null,
                BorderThickness = new Thickness(highContrast ? 1 : 0),
                CornerRadius = new CornerRadius(HomeTrayInks.Measures.PillRadius),
                Padding = new Thickness(HeroCard.PillPaddingX, HeroCard.PillPaddingY, HeroCard.PillPaddingX, HeroCard.PillPaddingY),
                Margin = new Thickness(0, HomeTrayInks.Measures.S1, 0, 0),
                HorizontalAlignment = HorizontalAlignment.Left,
                Child = new TextBlock
                {
                    Text = action,
                    FontWeight = FontWeights.SemiBold,
                    FontSize = HomeTrayInks.Measures.LabelSize,
                    Foreground = new SolidColorBrush(HeroColor(HeroPart.PillInk, ground, highContrast)),
                },
            });
        }

        card.Children.Add(words);

        // The whole card is one button; the pill draws the action, it is not
        // a second control (HeroCard.swift:84-96).
        var button = new Button
        {
            Content = card,
            Padding = new Thickness(0),
            BorderThickness = new Thickness(0),
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            Background = new SolidColorBrush(Colors.Transparent),
            CornerRadius = new CornerRadius(HomeTrayInks.Measures.CardRadius),
        };
        AutomationProperties.SetName(button, hero.Title);
        if (hero.Subtitle is string help)
        {
            AutomationProperties.SetHelpText(button, help);
        }

        button.Click += (_, _) => LinkRequested?.Invoke(hero.Link);
        button.PointerEntered += (_, _) => Fade(wash, HeroCard.HoverWash);
        button.PointerExited += (_, _) => Fade(wash, 0);

        if (!hero.Closes)
        {
            return button;
        }

        // The X: a control of its own over the card's button, in heroInk
        // at 0.75, whole under the pointer (HeroCard.swift:197-224).
        var outer = new Grid();
        outer.Children.Add(button);
        var glyphInk = new SolidColorBrush(inkColor) { Opacity = HeroCard.CloseInk };
        var close = new Button
        {
            Content = new FontIcon { Glyph = "", FontSize = HeroCard.CloseGlyph, Foreground = glyphInk },
            Background = new SolidColorBrush(Colors.Transparent),
            BorderThickness = new Thickness(0),
            Padding = new Thickness(HomeTrayInks.Measures.S4),
            HorizontalAlignment = HorizontalAlignment.Right,
            VerticalAlignment = VerticalAlignment.Top,
        };
        string closeWord = MonitorShellCopy.Top("close");
        AutomationProperties.SetName(close, closeWord);
        ToolTipService.SetToolTip(close, closeWord);
        close.PointerEntered += (_, _) => glyphInk.Opacity = HeroCard.CloseInkHover;
        close.PointerExited += (_, _) => glyphInk.Opacity = HeroCard.CloseInk;
        close.Click += (_, _) => CampaignClosed?.Invoke();
        outer.Children.Add(close);
        return outer;
    }

    /// <summary>
    /// Today's tokens per tool as bars, the longest first, heroInk fading
    /// from 0.95 to 0.45, cut off by the card's bottom edge
    /// (HomeInsightsVisual.swift:61-85).
    /// </summary>
    private static FrameworkElement BuildInsightsBars(IReadOnlyList<GlanceBar> bars, Color ink)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 14, VerticalAlignment = VerticalAlignment.Bottom };
        ulong most = bars[0].Tokens;
        foreach (GlanceBar bar in bars)
        {
            var fill = new LinearGradientBrush { StartPoint = new Windows.Foundation.Point(0, 0), EndPoint = new Windows.Foundation.Point(0, 1) };
            fill.GradientStops.Add(new GradientStop { Color = Color.FromArgb((byte)Math.Round(255 * 0.95), ink.R, ink.G, ink.B), Offset = 0 });
            fill.GradientStops.Add(new GradientStop { Color = Color.FromArgb((byte)Math.Round(255 * 0.45), ink.R, ink.G, ink.B), Offset = 1 });
            row.Children.Add(new Border
            {
                Width = HomeFacts.BarWidth,
                Height = HomeFacts.BarHeight(bar.Tokens, most),
                CornerRadius = new CornerRadius(6),
                Background = fill,
                VerticalAlignment = VerticalAlignment.Bottom,
                Margin = new Thickness(11, 0, 11, 0),
            });
        }

        return row;
    }

    /// <summary>
    /// A hero part's colour (<see cref="HeroCard.Ink"/>): the Mac's literal,
    /// or under high contrast the theme brush the Home tiles take, read now.
    /// </summary>
    private static Color HeroColor(HeroPart part, HeroGround ground, bool highContrast)
    {
        HeroInk ink = HeroCard.Ink(part, ground, highContrast);
        if (ink.ThemeKey is string key && ThemeBrushes.Get(key) is SolidColorBrush themed)
        {
            return themed.Color;
        }

        // A key that did not resolve keeps the Mac's colour, never a blank.
        return Argb(ink.ThemeKey is null ? ink.Argb : HeroCard.Ink(part, ground, highContrast: false).Argb);
    }

    /// <summary>A fade over the fast token (0.15 s) on the token curve (0.2, 0.8, 0.2, 1).</summary>
    private void Fade(UIElement target, double to)
    {
        if (!AnimationsOn)
        {
            target.Opacity = to;
            return;
        }

        var frames = new DoubleAnimationUsingKeyFrames();
        frames.KeyFrames.Add(new SplineDoubleKeyFrame
        {
            KeyTime = KeyTime.FromTimeSpan(TimeSpan.FromSeconds(MonitorMotion.Fast)),
            Value = to,
            KeySpline = new KeySpline
            {
                ControlPoint1 = new Windows.Foundation.Point(MonitorMotion.EaseX1, MonitorMotion.EaseY1),
                ControlPoint2 = new Windows.Foundation.Point(MonitorMotion.EaseX2, MonitorMotion.EaseY2),
            },
        });
        Storyboard.SetTarget(frames, target);
        Storyboard.SetTargetProperty(frames, "Opacity");
        var story = new Storyboard();
        story.Children.Add(frames);
        story.Begin();
    }

    private void UpdateClocks()
    {
        // At most 15 a second while the window is not focused (FocusFrameRule).
        if (GlyphClock.Interval(_loaded && _showing && WindowClocks.Running, _glyphs.Count > 0, AnimationsOn) is TimeSpan wanted
            && WindowClocks.Windows.Interval(XamlRoot, wanted.TotalSeconds) is double seconds)
        {
            TimeSpan every = TimeSpan.FromSeconds(seconds);
            if (_glyphTimer.Interval != every)
            {
                _glyphTimer.Interval = every;
            }

            if (!_glyphTimer.IsRunning)
            {
                _glyphTimer.Start();
            }
        }
        else if (_glyphTimer.IsRunning)
        {
            _glyphTimer.Stop();
        }

        // Still under reduce motion: the glyph at t = 0 (PixelGlyph.swift:62).
        DrawGlyphs();
        if (_burstTicking)
        {
            if (!AnimationsOn)
            {
                // Animations turned off mid-burst: its still moment, held
                // until the burst's time is up (CoinBurst.swift:137-139).
                StopBurstTicks();
                _burst?.Draw(CoinBurst.StillAt, flash: false);
            }
            else if (!MayTick)
            {
                // Hidden mid-burst: it completes its end state, no coins.
                EndBurst();
            }
        }
    }

    private void DrawGlyphs()
    {
        long begun = LookFrames.Begin();
        double time = PixelField.Time(DateTimeOffset.UtcNow, still: !AnimationsOn);
        bool dark = ActualTheme != ElementTheme.Light;
        foreach (HeroGlyphView glyph in _glyphs)
        {
            glyph.Draw(time, dark);
        }

        LookFrames.End("home-glyphs", begun);
    }

    /// <summary>
    /// Marks NEAR AI credit arriving on the Balance tile
    /// (<c>BalanceArrivalFlourish.check</c>, BalanceArrivalFlourish.swift:99-117):
    /// from the reading last shown, kept across launches, to this one.
    /// </summary>
    public void CheckBalance(BalanceArrival.Reading now, BalanceArrival.Reading? shown, byte? scale)
    {
        BalanceArrival.Effect effect = BalanceArrival.EffectOf(shown, now, reduceMotion: !AnimationsOn);
        switch (effect.Kind)
        {
            case BalanceArrival.EffectKind.Highlight:
                StartBurstAfterLayout();
                break;
            case BalanceArrival.EffectKind.CountUp:
                StartBurstAfterLayout();
                _countSteps = BalanceArrival.CountSteps(effect.FromNanos, effect.ToNanos, BalanceArrival.CountFrames);
                _countIndex = 0;
                _countScale = scale;
                _countTimer.Interval = TimeSpan.FromSeconds(
                    WindowClocks.Windows.Interval(XamlRoot, BalanceArrival.CountFrame.TotalSeconds) ?? BalanceArrival.CountFrame.TotalSeconds);
                _countTimer.Start();
                CountStep();
                break;
        }
    }

    private void CountStep()
    {
        if (_balanceValue is null || _countIndex >= _countSteps.Count)
        {
            _countTimer.Stop();
            if (_model is { AccountTiles.Count: > 2 } model && _balanceValue is not null)
            {
                _balanceValue.Text = model.AccountTiles[2].Value;
            }

            return;
        }

        string figure = NearAiBalanceSurface.Amount(_countSteps[_countIndex++], _countScale);
        if (figure.Length > 0)
        {
            _balanceValue.Text = figure;
        }
    }

    /// <summary>Starts the burst once the tile has its place: a tile drawn this turn has no layout yet.</summary>
    private void StartBurstAfterLayout() =>
        DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, StartBurst);

    /// <summary>
    /// Gold coins burst out of the Balance tile's centre (<c>GlassCoinBurst</c>),
    /// over everything on Home: in flight for 1.7 s once a display frame, or
    /// under reduce motion drawn still at 0.45 s for 1.6 s with no flash.
    /// </summary>
    private void StartBurst()
    {
        if (_balanceTile is null || _balanceTile.ActualWidth <= 0)
        {
            return;
        }

        EndBurst();
        var transform = _balanceTile.TransformToVisual(_burstLayer);
        Windows.Foundation.Point centre = transform.TransformPoint(new Windows.Foundation.Point(_balanceTile.ActualWidth / 2, _balanceTile.ActualHeight / 2));
        double room = _burstLayer.ActualWidth - (centre.X + _balanceTile.ActualWidth / 2) + HomeTrayInks.Measures.S3;
        _burst = new CoinBurstView(_burstLayer, centre.X, centre.Y, CoinBurst.Sideways(_balanceTile.ActualWidth / 2 + room));
        bool still = !AnimationsOn;
        _burstEnd.Interval = BalanceArrival.CoinsStay(still);
        _burstEnd.Start();
        if (still)
        {
            _burst.Draw(CoinBurst.StillAt, flash: false);
            return;
        }

        _burstClock.Restart();
        _burstTicking = true;
        CompositionTarget.Rendering += OnBurstFrame;
        _burst.Draw(0, flash: true);
    }

    private readonly FocusFramePace<XamlRoot> _burstFramePace = new(WindowClocks.Windows);

    private void OnBurstFrame(object? sender, object e)
    {
        // The window's frame rule (FocusFrameRule): at most 15 a second while unfocused.
        if (_burstFramePace.Skips(XamlRoot, GlassClock.Now))
        {
            return;
        }

        if (_burst is null)
        {
            EndBurst();
            return;
        }

        if (!MayTick)
        {
            UpdateClocks();
            return;
        }

        double t = _burstClock.Elapsed.TotalSeconds;
        if (t >= CoinBurst.Duration)
        {
            EndBurst();
            return;
        }

        _burst.Draw(t, flash: true);
    }

    private void StopBurstTicks()
    {
        if (_burstTicking)
        {
            _burstTicking = false;
            CompositionTarget.Rendering -= OnBurstFrame;
        }
    }

    private void EndBurst()
    {
        StopBurstTicks();

        _burstEnd.Stop();
        _burst?.Remove();
        _burst = null;
    }

    private static Color Argb(uint argb) =>
        Color.FromArgb((byte)(argb >> 24), (byte)(argb >> 16), (byte)(argb >> 8), (byte)argb);

    /// <summary>
    /// One hero's pixel glyph: a 252-point canvas of cells, turned -12
    /// degrees about its centre (HeroCard.swift:249-251), placed so its
    /// centre sits past the card's trailing and bottom edges by the bleed.
    /// Each lit cell is a rectangle whose fill is one of six brushes (three
    /// blues, bright and dim); a frame only swaps fills.
    /// </summary>
    private sealed class HeroGlyphView
    {
        private readonly HeroGlyph.Mask _mask;
        private readonly Canvas _canvas = new() { IsHitTestVisible = false };
        private readonly Dictionary<(int Column, int Row), Rectangle> _cells = new();
        private SolidColorBrush[]? _brushes;
        private bool _dark;

        public HeroGlyphView(HeroGlyph.Mask mask, Canvas host)
        {
            _mask = mask;
            double side = HeroGlyph.CanvasSide;
            _canvas.Width = side;
            _canvas.Height = side;
            _canvas.RenderTransformOrigin = new Windows.Foundation.Point(0.5, 0.5);
            (double m11, double m12, double m21, double m22) = HeroGlyph.Rotation();
            _canvas.RenderTransform = new MatrixTransform { Matrix = new Matrix(m11, m12, m21, m22, 0, 0) };
            AutomationProperties.SetAccessibilityView(_canvas, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
            double cell = Math.Max(HeroGlyph.Cell - 1, 1);
            foreach (HeroGlyph.Lit lit in HeroGlyph.LitCells(mask, 0, 3))
            {
                var rect = new Rectangle { Width = cell, Height = cell };
                Canvas.SetLeft(rect, lit.Column * HeroGlyph.Cell);
                Canvas.SetTop(rect, lit.Row * HeroGlyph.Cell);
                _canvas.Children.Add(rect);
                _cells[(lit.Column, lit.Row)] = rect;
            }

            host.Children.Add(_canvas);
        }

        public void Place(double width, double height)
        {
            (double x, double y) = HeroGlyph.Centre(width, height);
            Canvas.SetLeft(_canvas, x - HeroGlyph.CanvasSide / 2);
            Canvas.SetTop(_canvas, y - HeroGlyph.CanvasSide / 2);
        }

        public void Draw(double time, bool dark)
        {
            if (_brushes is null || dark != _dark)
            {
                _dark = dark;
                IReadOnlyList<uint> tints = HeroGlyph.Tints(dark);
                _brushes = new SolidColorBrush[tints.Count * 2];
                for (int i = 0; i < tints.Count; i++)
                {
                    _brushes[i * 2] = new SolidColorBrush(Argb(tints[i])) { Opacity = HeroGlyph.DimCells };
                    _brushes[i * 2 + 1] = new SolidColorBrush(Argb(tints[i])) { Opacity = HeroGlyph.BrightCells };
                }
            }

            foreach (HeroGlyph.Lit lit in HeroGlyph.LitCells(_mask, time, _brushes.Length / 2))
            {
                if (_cells.TryGetValue((lit.Column, lit.Row), out Rectangle? rect))
                {
                    rect.Fill = _brushes[lit.Shade * 2 + (lit.Bright ? 1 : 0)];
                }
            }
        }
    }

    /// <summary>
    /// The coins of one burst as shapes on Home's top layer: the flash, each
    /// coin a turned ellipse in the gold gradient with its rim and glint, and
    /// three sparks behind each while it is fast (CoinBurst.swift:150-210).
    /// Built once per burst; a frame only moves and scales them.
    /// </summary>
    private sealed class CoinBurstView
    {
        private readonly Canvas _layer;
        private readonly double _cx;
        private readonly double _cy;
        private readonly double _sideways;
        private readonly Ellipse _flash = new() { IsHitTestVisible = false };
        private readonly List<(Coin Coin, Ellipse Face, ScaleTransform Width, Ellipse Glint, Ellipse[] Sparks)> _coins = new();
        private readonly LinearGradientBrush _litFace;
        private readonly LinearGradientBrush _backFace;

        public CoinBurstView(Canvas layer, double cx, double cy, double sideways)
        {
            _layer = layer;
            _cx = cx;
            _cy = cy;
            _sideways = sideways;
            _litFace = Gradient(HomeTrayInks.CoinGoldLight, HomeTrayInks.CoinGold, HomeTrayInks.CoinGoldDeep);
            _backFace = Gradient(HomeTrayInks.CoinGold, HomeTrayInks.CoinGoldDeep);
            var glow = new RadialGradientBrush();
            glow.GradientStops.Add(new GradientStop { Color = Argb(HomeTrayInks.CoinGoldLight), Offset = 0 });
            glow.GradientStops.Add(new GradientStop { Color = Argb(HomeTrayInks.CoinGold), Offset = 0.5 });
            glow.GradientStops.Add(new GradientStop { Color = Argb(HomeTrayInks.CoinGold & 0x00FFFFFFU), Offset = 1 });
            _flash.Fill = glow;
            layer.Children.Add(_flash);
            foreach (Coin coin in CoinBurst.Coins)
            {
                var sparks = new Ellipse[3];
                for (int s = 0; s < 3; s++)
                {
                    sparks[s] = new Ellipse { Fill = new SolidColorBrush(Argb(HomeTrayInks.CoinSpark)), IsHitTestVisible = false };
                    layer.Children.Add(sparks[s]);
                }

                var width = new ScaleTransform();
                var group = new TransformGroup();
                group.Children.Add(width);
                group.Children.Add(new RotateTransform { Angle = coin.Tilt * 180 / Math.PI });
                var face = new Ellipse
                {
                    Width = coin.Diameter,
                    Height = coin.Diameter,
                    Stroke = new SolidColorBrush(Argb(HomeTrayInks.CoinGoldRim)),
                    StrokeThickness = 0.6,
                    RenderTransformOrigin = new Windows.Foundation.Point(0.5, 0.5),
                    RenderTransform = group,
                    IsHitTestVisible = false,
                };
                var glint = new Ellipse
                {
                    Width = coin.Diameter * 0.32,
                    Height = coin.Diameter * 0.32,
                    Fill = new SolidColorBrush(Argb(HomeTrayInks.CoinGlint)),
                    IsHitTestVisible = false,
                };
                layer.Children.Add(face);
                layer.Children.Add(glint);
                _coins.Add((coin, face, width, glint, sparks));
            }
        }

        private static LinearGradientBrush Gradient(params uint[] colours)
        {
            var brush = new LinearGradientBrush { StartPoint = new Windows.Foundation.Point(0, 0), EndPoint = new Windows.Foundation.Point(1, 1) };
            for (int i = 0; i < colours.Length; i++)
            {
                brush.GradientStops.Add(new GradientStop { Color = Argb(colours[i]), Offset = colours.Length == 1 ? 0 : (double)i / (colours.Length - 1) });
            }

            return brush;
        }

        public void Draw(double t, bool flash)
        {
            if (flash && t < CoinBurst.FlashDuration)
            {
                double left = 1 - t / CoinBurst.FlashDuration;
                double radius = CoinBurst.FlashRadius * (0.4 + 0.6 * (t / CoinBurst.FlashDuration));
                _flash.Width = _flash.Height = radius * 2;
                Canvas.SetLeft(_flash, _cx - radius);
                Canvas.SetTop(_flash, _cy - radius);

                // The stops' own alphas are 0.75 and 0.3 of `left`; the
                // brush is drawn at `left` over stops at 0.75 and 0.3.
                _flash.Opacity = left;
                var stops = ((RadialGradientBrush)_flash.Fill).GradientStops;
                Color light = Argb(HomeTrayInks.CoinGoldLight), gold = Argb(HomeTrayInks.CoinGold);
                stops[0].Color = Color.FromArgb((byte)Math.Round(255 * 0.75), light.R, light.G, light.B);
                stops[1].Color = Color.FromArgb((byte)Math.Round(255 * 0.3), gold.R, gold.G, gold.B);
                _flash.Visibility = Visibility.Visible;
            }
            else
            {
                _flash.Visibility = Visibility.Collapsed;
            }

            double opacity = CoinBurst.Opacity(t);
            foreach ((Coin coin, Ellipse face, ScaleTransform width, Ellipse glint, Ellipse[] sparks) in _coins)
            {
                double flown = t - coin.Delay;
                bool shown = flown > 0;
                face.Visibility = shown ? Visibility.Visible : Visibility.Collapsed;
                glint.Visibility = Visibility.Collapsed;
                for (int s = 0; s < 3; s++)
                {
                    sparks[s].Visibility = Visibility.Collapsed;
                }

                if (!shown)
                {
                    continue;
                }

                Offset at = CoinBurst.OffsetOf(coin, t, _sideways);
                double x = _cx + at.Dx, y = _cy + at.Dy;
                double turn = coin.Phase + coin.Spin * flown;
                double facing = CoinDrawing.Facing(turn);
                width.ScaleX = Math.Max(0.2, Math.Abs(facing));
                face.Fill = facing > 0 ? _litFace : _backFace;
                face.Opacity = opacity;
                Canvas.SetLeft(face, x - coin.Diameter / 2);
                Canvas.SetTop(face, y - coin.Diameter / 2);
                if (Math.Abs(facing) > 0.92)
                {
                    double g = coin.Diameter * 0.32;
                    double lx = -coin.Diameter * 0.28 + g / 2, ly = -coin.Diameter * 0.3 + g / 2;
                    double cos = Math.Cos(coin.Tilt), sin = Math.Sin(coin.Tilt);
                    Canvas.SetLeft(glint, x + lx * cos - ly * sin - g / 2);
                    Canvas.SetTop(glint, y + lx * sin + ly * cos - g / 2);
                    glint.Opacity = opacity;
                    glint.Visibility = Visibility.Visible;
                }

                if (flown < CoinBurst.SparkWhile)
                {
                    for (int s = 0; s < 3; s++)
                    {
                        int step = s + 1;
                        Offset back = CoinBurst.OffsetOf(coin, t - step * CoinBurst.SparkStep, _sideways);
                        double dot = coin.Diameter * (0.42 - step * 0.08);
                        sparks[s].Width = sparks[s].Height = dot;
                        Canvas.SetLeft(sparks[s], _cx + back.Dx - dot / 2);
                        Canvas.SetTop(sparks[s], _cy + back.Dy - dot / 2);
                        sparks[s].Opacity = (0.75 - step * 0.2) * (1 - flown / CoinBurst.SparkWhile) * opacity;
                        sparks[s].Visibility = Visibility.Visible;
                    }
                }
            }
        }

        public void Remove() => _layer.Children.Clear();
    }
}
