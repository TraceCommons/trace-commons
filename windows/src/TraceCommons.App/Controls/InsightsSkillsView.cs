using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Globalization;
using System.Linq;
using System.Numerics;
using Microsoft.UI;
using Microsoft.UI.Composition;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Hosting;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Markup;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Animation;
using Microsoft.UI.Xaml.Shapes;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;
using Windows.UI.ViewManagement;
using Color = Windows.UI.Color;

namespace TraceCommons.App.Controls;

/// <summary>
/// The Skills tab (<c>InsightsSkillsTab</c> and <c>InsightsSkillsHomeView</c>,
/// macos/Sources/TraceCommonsApp/Views/InsightsSkillsTab.swift and
/// InsightsSkillsHomeView.swift): Home, Index, Usage, Browse and Following
/// over the daemon's snapshot. Skills are observed and followed here; a
/// publisher's own instructions own setup. Every word is the snapshot's copy
/// or the core's Insights table; every decision is
/// <see cref="InsightsSkillsModel"/>'s or <see cref="InsightsSkillsHome"/>'s.
/// Built in code because Home's stories are a ranked, mixed list.
/// </summary>
public sealed class InsightsSkillsView : UserControl
{
    public static IReadOnlyList<string> Sections => InsightsSkillsModel.Sections;

    private readonly InsightsSkillsModel _model;
    private readonly Action? _reviewSettings;
    private readonly UISettings _ui = new();
    private readonly ScrollViewer _scroll = new() { HorizontalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    private readonly StackPanel _page = new() { Spacing = 12, Padding = new Thickness(28), MaxWidth = 1000, HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly TextBox _inventoryQuery = new();
    private readonly TextBox _catalogQuery = new();
    private readonly List<FrameworkElement> _scrollCards = new();
    private readonly HashSet<string> _expanded = new(StringComparer.Ordinal);
    private readonly HashSet<string> _grown = new(StringComparer.Ordinal);
    private readonly Dictionary<string, int> _shownCounts = new(StringComparer.Ordinal);
    private readonly List<RollingNumber> _rolls = new();

    private string _section = "home";
    private bool _evidenceOpen;
    private string _inventoryAgent = "", _inventoryAvailability = "", _inventorySort = "name";
    private int _usageDays = 30;
    private string? _selectedSkillId, _selectedUsageId, _selectedCatalogId, _selectedCatalogInstallationId;
    private bool _rendering;

    /// <summary>
    /// The follow control a save started from. Saving disables it, which
    /// moves keyboard focus away; once the reply is drawn, focus returns to
    /// the same publisher's control, whatever it now says.
    /// </summary>
    private string? _refocusId;

    public InsightsSkillsView(InsightsSkillsModel model, Action? reviewSettings)
    {
        _model = model;
        _reviewSettings = reviewSettings;
        _scroll.Content = _page;
        Content = _scroll;
        AutomationProperties.SetAutomationId(this, "insights-skills");
        _inventoryQuery.TextChanged += (_, _) => Render();
        _catalogQuery.TextChanged += (_, _) => Render();
        _model.PropertyChanged += OnModelChanged;
        _scroll.ViewChanged += (_, _) => ApplyScrollMotion();
        _scroll.SizeChanged += (_, _) => ApplyScrollMotion();
        ActualThemeChanged += (_, _) => Render();
        WindowClocks.Changed += OnClocksChanged;
        Unloaded += (_, _) => _model.InvalidateDetails();
        Render();
    }

    /// <summary>The tab was removed: this view stops listening, so a removed view sends nothing.</summary>
    public void Close()
    {
        _model.PropertyChanged -= OnModelChanged;
        WindowClocks.Changed -= OnClocksChanged;
        foreach (RollingNumber roll in _rolls) roll.Finish();
        _rolls.Clear();
    }

    /// <summary>What may move: the system's animation setting, and only while the window's clocks run.</summary>
    private SkillsHomeMotion Motion => new(!(_ui.AnimationsEnabled && WindowClocks.Running));

    public string ShownSection => _section;

    public void SelectSection(string section)
    {
        if (_rendering || !Sections.Contains(section) || section == _section) return;
        _model.InvalidateDetails();
        if (section != "catalog")
        {
            _selectedCatalogId = null;
            _selectedCatalogInstallationId = null;
        }

        _section = section;
        // After the toggle's own event: the tab row is rebuilt.
        DispatcherQueue.TryEnqueue(Render);
    }

    private void OnModelChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (!_rendering) DispatcherQueue?.TryEnqueue(Render);
    }

    private void OnClocksChanged(bool running)
    {
        // Stopped clocks stop every roll at its value: nothing ticks while minimised.
        if (!running) DispatcherQueue?.TryEnqueue(() => { foreach (RollingNumber roll in _rolls) roll.Finish(); });
    }

    // MARK: Words and parts

    private string T(string key) => _model.Text(key);

    private string T(string key, string name, string value) => _model.Text(key, name, value);

    private string T(string key, params (string Name, string Value)[] values) =>
        _model.Text(key, values.ToDictionary(v => v.Name, v => v.Value));

    private static string N(int value) => value.ToString(CultureInfo.InvariantCulture);

    private static Brush R(string key) => (Brush)Application.Current.Resources[key];

    private static TextBlock Line(string text, string? style = null, Brush? brush = null)
    {
        var block = new TextBlock { Text = text, TextWrapping = TextWrapping.Wrap };
        if (style is not null && Application.Current.Resources.TryGetValue(style, out object? s) && s is Style found) block.Style = found;
        if (brush is not null) block.Foreground = brush;
        return block;
    }

    private static TextBlock Heading(string text) => Line(text, "TcTitleCardTextStyle");

    private static TextBlock Caption(string text) => Line(text, "TcCaptionTextStyle", R("TcInkSecondaryBrush"));

    private static TextBlock Note(string text) => Line(text, "TcBodyTextStyle", R("TcInkSecondaryBrush"));

    private static TextBlock Mono(string text)
    {
        TextBlock block = Line(text, "TcMonoCodeTextStyle");
        block.IsTextSelectionEnabled = true;
        return block;
    }

    private static TextBlock Error(string text)
    {
        TextBlock block = Line(text, "TcBodyTextStyle", R("TcStatusOutsideTextBrush"));
        AutomationProperties.SetLiveSetting(block, AutomationLiveSetting.Polite);
        return block;
    }

    private static Button Action(string label, Action click, string? name = null)
    {
        var button = new Button { Content = label };
        if (name is not null) AutomationProperties.SetName(button, name);
        button.Click += (_, _) => click();
        return button;
    }

    private static HyperlinkButton Link(string label, Action click, string? name = null)
    {
        var button = new HyperlinkButton { Content = label, Padding = new Thickness(0) };
        if (name is not null) AutomationProperties.SetName(button, name);
        button.Click += (_, _) => click();
        return button;
    }

    private static Border Card(UIElement content) => new()
    {
        Background = R("TcSurfaceCardBrush"),
        BorderBrush = R("TcHairlineBrush"),
        BorderThickness = new Thickness(1),
        CornerRadius = new CornerRadius(12),
        Padding = new Thickness(16, 14, 16, 14),
        Child = content,
    };

    private StackPanel Section(string key, params UIElement[] content)
    {
        var body = new StackPanel { Spacing = 10 };
        foreach (UIElement element in content) body.Children.Add(element);
        var panel = new StackPanel { Spacing = 8 };
        TextBlock heading = Heading(T(key));
        AutomationProperties.SetHeadingLevel(heading, AutomationHeadingLevel.Level2);
        panel.Children.Add(heading);
        panel.Children.Add(Card(body));
        return panel;
    }

    // MARK: Rendering

    private void Render()
    {
        if (_rendering) return;
        _rendering = true;
        // A rebuild replaces the controls: the focused one is found again
        // afterwards by its name, and a search box keeps its caret.
        (string? Name, Type? Kind, int Caret) focused = (null, null, 0);
        if (XamlRoot is not null && FocusManager.GetFocusedElement(XamlRoot) is FrameworkElement had && IsInside(had))
        {
            focused = (had is TextBox box ? "\u0000" + (ReferenceEquals(box, _inventoryQuery) ? "inventory" : "catalog")
                : AutomationProperties.GetName(had), had.GetType(), had is TextBox caret ? caret.SelectionStart : 0);
        }

        try
        {
            _scrollCards.Clear();
            foreach (RollingNumber roll in _rolls) roll.Finish();
            _rolls.Clear();
            Detach(_inventoryQuery);
            Detach(_catalogQuery);
            _page.Children.Clear();
            _page.Children.Add(Header());
            _page.Children.Add(SectionTabs());
            if (_evidenceOpen) _page.Children.Add(EvidencePanel());
            if (_model.FailureKey is { } failure && (failure != "skills_follow_failed" || !FollowFailureIsInline()))
            {
                _page.Children.Add(Error(T(failure)));
            }

            if (_model.Shown is { } snapshot)
            {
                var content = new StackPanel { Spacing = 16 };
                AutomationProperties.SetAutomationId(content, "insights-skills-" + _section);
                if (!snapshot.Enabled)
                {
                    content.Children.Add(Note(T("skills_disabled")));
                }
                else
                {
                    switch (_section)
                    {
                        case "home": content.Children.Add(Home(snapshot)); break;
                        case "inventory": content.Children.Add(Inventory(snapshot)); break;
                        case "usage": content.Children.Add(Usage(snapshot)); break;
                        case "catalog":
                            if (_selectedCatalogId is null) content.Children.Add(Publishers(snapshot));
                            content.Children.Add(Catalog(snapshot));
                            break;
                        default: content.Children.Add(Following(snapshot)); break;
                    }
                }

                _page.Children.Add(content);
            }

            DispatcherQueue?.TryEnqueue(ApplyScrollMotion);
            if (focused.Name is { Length: > 0 } name) DispatcherQueue?.TryEnqueue(() => Refocus(name, focused.Kind, focused.Caret));
            if (_refocusId is { } follow)
            {
                DispatcherQueue?.TryEnqueue(() =>
                {
                    if (FindId(_page, follow) is Control { IsEnabled: true } control)
                    {
                        control.Focus(FocusState.Keyboard);
                        if (!_model.Busy) _refocusId = null;
                    }
                });
            }
        }
        finally
        {
            _rendering = false;
        }
    }

    private bool IsInside(DependencyObject element)
    {
        for (DependencyObject? at = element; at is not null; at = VisualTreeHelper.GetParent(at))
        {
            if (ReferenceEquals(at, this)) return true;
        }

        return false;
    }

    private void Refocus(string name, Type? kind, int caret)
    {
        if (name == "\u0000inventory" || name == "\u0000catalog")
        {
            TextBox box = name == "\u0000inventory" ? _inventoryQuery : _catalogQuery;
            if (box.Parent is null) return;
            box.Focus(FocusState.Programmatic);
            box.SelectionStart = Math.Min(caret, box.Text.Length);
            return;
        }

        if (Find(_page, name, kind) is Control control) control.Focus(FocusState.Programmatic);
    }

    private static DependencyObject? FindId(DependencyObject root, string id)
    {
        if (root is FrameworkElement element && AutomationProperties.GetAutomationId(element) == id) return root;
        int count = VisualTreeHelper.GetChildrenCount(root);
        for (int i = 0; i < count; i++)
        {
            if (FindId(VisualTreeHelper.GetChild(root, i), id) is { } found) return found;
        }

        return null;
    }

    private static DependencyObject? Find(DependencyObject root, string name, Type? kind)
    {
        if (root is FrameworkElement element && (kind is null || element.GetType() == kind) && AutomationProperties.GetName(element) == name)
            return root;
        int count = VisualTreeHelper.GetChildrenCount(root);
        for (int i = 0; i < count; i++)
        {
            if (Find(VisualTreeHelper.GetChild(root, i), name, kind) is { } found) return found;
        }

        return null;
    }

    private static void Detach(FrameworkElement element)
    {
        if (element.Parent is Panel panel) panel.Children.Remove(element);
    }

    private bool FollowFailureIsInline()
    {
        if (_model.FollowFailurePublisherId is not { } failed || _model.Shown is not { Enabled: true } snapshot) return false;
        return _section switch
        {
            "home" => InsightsSkillsHome.Make(snapshot, _model, DateTimeOffset.Now, TimeZoneInfo.Local).DefaultCreators.Any(c => c.Id == failed),
            "catalog" => _selectedCatalogId is null && snapshot.Catalog.Publishers.Any(p => p.Id == failed),
            _ => false,
        };
    }

    private UIElement Header()
    {
        var row = new Grid { ColumnSpacing = 12 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        TextBlock title = Line(T("skills_title"), "TcTitleSectionTextStyle");
        AutomationProperties.SetHeadingLevel(title, AutomationHeadingLevel.Level1);
        row.Children.Add(title);
        if ((_model.Busy && _model.PendingFollowPublisherId is null) || _model.Shown?.Refreshing == true)
        {
            var ring = new ProgressRing { IsActive = true, Width = 16, Height = 16 };
            AutomationProperties.SetName(ring, T("skills_loading"));
            Grid.SetColumn(ring, 1);
            row.Children.Add(ring);
        }

        Button refresh = Action(T("skills_refresh"), async () => await _model.LoadAsync(refresh: true), T("skills_refresh_accessibility"));
        refresh.IsEnabled = !_model.Busy;
        Grid.SetColumn(refresh, 2);
        row.Children.Add(refresh);
        return row;
    }

    private UIElement SectionTabs()
    {
        var tabs = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        AutomationProperties.SetName(tabs, T("skills_title"));
        foreach (string section in Sections)
        {
            var tab = new ToggleButton { Content = T(InsightsSkillsModel.SectionCopyKey(section)), IsChecked = section == _section };
            AutomationProperties.SetAutomationId(tab, "skills-tab-" + section);
            // Checked, not Click: UI Automation's Toggle and the keyboard
            // reach a ToggleButton through Checked as a click does.
            tab.Checked += (_, _) => SelectSection(section);
            tab.Unchecked += (_, _) =>
            {
                if (section == _section) tab.IsChecked = true;
            };
            tabs.Children.Add(tab);
        }

        return tabs;
    }

    private UIElement EvidencePanel()
    {
        var panel = new StackPanel { Spacing = 6 };
        var top = new Grid();
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        top.Children.Add(Heading(T("skills_evidence_title")));
        Button close = Action(T("skills_close"), () => { _evidenceOpen = false; _model.InvalidateDetails(); });
        Grid.SetColumn(close, 1);
        top.Children.Add(close);
        panel.Children.Add(top);
        if (_model.EvidenceFailure)
        {
            panel.Children.Add(Note(T("skills_evidence_unavailable")));
        }
        else if (_model.Evidence?.Session is { } session)
        {
            if (session.Source is { } source) panel.Children.Add(Line(_model.AgentLabel(source)));
            if (session.Date is { } date) panel.Children.Add(Line(date));
            if (session.Turns is { } turns) panel.Children.Add(Line(T("skills_evidence_turns", "count", turns.ToString(CultureInfo.InvariantCulture))));
            if (session.Tokens is { } tokens) panel.Children.Add(Line(T("skills_evidence_tokens", "count", tokens.ToString(CultureInfo.InvariantCulture))));
        }
        else if (_model.Evidence?.SkillUsage is { } observations)
        {
            if (_model.Evidence.Agent is { } agent) panel.Children.Add(Caption(_model.AgentLabel(agent)));
            panel.Children.Add(Caption(_model.PopulationLabel(_model.Evidence.Population ?? "unknown")));
            panel.Children.Add(Caption(T("skills_coverage_" + observations.Coverage)));
            if (observations.Partial) panel.Children.Add(Note(T("skills_partial_usage")));
            foreach (SkillEvidenceCount count in observations.Skills)
            {
                panel.Children.Add(Heading(count.Name));
                panel.Children.Add(Line(T("skills_invocations", "count", N(count.Invocations))));
                panel.Children.Add(Line(T("skills_loads", "count", N(count.Loads))));
            }
        }
        else
        {
            panel.Children.Add(Note(T("skills_loading")));
        }

        return Card(panel);
    }

    // MARK: Home

    private void Open(SkillsHomeRoute? route)
    {
        if (route is null) return;
        _evidenceOpen = false;
        _model.InvalidateDetails();
        switch (route.Section)
        {
            case "inventory":
                _inventoryQuery.Text = "";
                _inventoryAgent = "";
                _inventoryAvailability = "";
                _selectedSkillId = null;
                break;
            case "usage":
                _usageDays = 7;
                _selectedUsageId = route.UsageId;
                break;
            case "catalog":
                _catalogQuery.Text = "";
                _selectedCatalogId = route.CatalogId;
                _selectedCatalogInstallationId = route.InstalledId;
                break;
        }

        _section = route.Section;
        Render();
        _scroll.ChangeView(null, 0, null, disableAnimation: !Motion.Scrolls);
    }

    private UIElement Home(SkillsSnapshot snapshot)
    {
        DateTimeOffset now = DateTimeOffset.Now;
        InsightsSkillsHome.Snapshot feed = InsightsSkillsHome.Make(snapshot, _model, now, TimeZoneInfo.Local);
        IReadOnlyList<InsightsSkillsHome.Story> stories = feed.RankedStories(now, TimeZoneInfo.Local);
        string? hero = InsightsSkillsHome.Snapshot.RankedHeroId(stories);
        var column = new StackPanel { Spacing = 20, MaxWidth = SkillsHomeMotion.ColumnMaximum, HorizontalAlignment = HorizontalAlignment.Center };
        AutomationProperties.SetAutomationId(column, "skills-home");
        if (stories.Count == 0 && feed.DefaultCreators.Count == 0)
        {
            var empty = new StackPanel { Spacing = 8 };
            empty.Children.Add(Note(T("skills_home_empty")));
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            actions.Children.Add(Action(T("skills_home_index"), () => Open(SkillsHomeRoute.Index(snapshot)), T("skills_home_index_accessibility")));
            actions.Children.Add(Action(T("skills_home_browse"), () => Open(SkillsHomeRoute.Browse(snapshot)), T("skills_home_browse_accessibility")));
            empty.Children.Add(actions);
            column.Children.Add(Card(empty));
        }

        foreach (InsightsSkillsHome.Story story in stories)
        {
            column.Children.Add(HomeCard(StoryContent(story, story.Id == hero, snapshot, now), story.Id));
        }

        if (feed.DefaultCreators.Count > 0)
        {
            var creators = new StackPanel { Spacing = 10 };
            TextBlock heading = Caption(T("skills_home_creators"));
            heading.Margin = new Thickness(16, 0, 0, 0);
            AutomationProperties.SetHeadingLevel(heading, AutomationHeadingLevel.Level2);
            creators.Children.Add(heading);
            foreach (InsightsSkillsHome.DefaultCreator creator in feed.DefaultCreators)
            {
                creators.Children.Add(HomeCard(DefaultCreator(creator), "default:" + creator.Id));
            }

            column.Children.Add(creators);
        }

        return column;
    }

    /// <summary>One story's card: it rises 1.5 under the pointer and eases in as it scrolls into view, unless motion is off.</summary>
    private FrameworkElement HomeCard(UIElement content, string id)
    {
        Border card = Card(content);
        AutomationProperties.SetAutomationId(card, "skills-story-" + id.Replace("\0", ":", StringComparison.Ordinal));
        var transform = new CompositeTransform();
        card.RenderTransform = transform;
        card.RenderTransformOrigin = new Windows.Foundation.Point(0.5, 0.5);
        card.PointerEntered += (_, _) => Lift(transform, -SkillsHomeMotion.HoverLift);
        card.PointerExited += (_, _) => Lift(transform, 0);
        _scrollCards.Add(card);
        return card;
    }

    private void Lift(CompositeTransform transform, double to)
    {
        if (!Motion.Lifts)
        {
            transform.TranslateY = 0;
            return;
        }

        Animate(transform, "TranslateY", to, SkillsHomeMotion.HoverSeconds);
    }

    /// <summary>Eases a property to a value with the Mac's <c>.easeOut</c> curve.</summary>
    private static void Animate(DependencyObject target, string property, double to, double seconds, double? from = null)
    {
        var frames = new DoubleAnimationUsingKeyFrames();
        if (from is { } start) frames.KeyFrames.Add(new DiscreteDoubleKeyFrame { KeyTime = KeyTime.FromTimeSpan(TimeSpan.Zero), Value = start });
        frames.KeyFrames.Add(new SplineDoubleKeyFrame
        {
            KeyTime = KeyTime.FromTimeSpan(TimeSpan.FromSeconds(seconds)),
            Value = to,
            KeySpline = new KeySpline { ControlPoint1 = new Windows.Foundation.Point(0, 0), ControlPoint2 = new Windows.Foundation.Point(0.58, 1) },
        });
        Storyboard.SetTarget(frames, target);
        Storyboard.SetTargetProperty(frames, property);
        var board = new Storyboard();
        board.Children.Add(frames);
        board.Begin();
    }

    /// <summary>
    /// <c>scrollTransition(.interactive)</c>: a card wholly in view is
    /// itself; one partly out fades toward 0.6 and shrinks toward 0.985 by
    /// how much of it is out.
    /// </summary>
    private void ApplyScrollMotion()
    {
        bool scrolls = Motion.Scrolls;
        double height = _scroll.ViewportHeight;
        foreach (FrameworkElement card in _scrollCards)
        {
            if (card.RenderTransform is not CompositeTransform transform) continue;
            double shown = 1;
            if (scrolls && height > 0 && card.ActualHeight > 0 && card.XamlRoot is not null)
            {
                try
                {
                    Windows.Foundation.Rect bounds = card.TransformToVisual(_scroll).TransformBounds(
                        new Windows.Foundation.Rect(0, 0, card.ActualWidth, card.ActualHeight));
                    double visible = Math.Max(0, Math.Min(bounds.Bottom, height) - Math.Max(bounds.Top, 0));
                    shown = Math.Clamp(visible / bounds.Height, 0, 1);
                }
                catch (ArgumentException)
                {
                    shown = 1;
                }
            }

            card.Opacity = SkillsHomeMotion.ScrollOpacity + (1 - SkillsHomeMotion.ScrollOpacity) * shown;
            double scale = SkillsHomeMotion.ScrollScale + (1 - SkillsHomeMotion.ScrollScale) * shown;
            transform.ScaleX = scale;
            transform.ScaleY = scale;
        }
    }

    private UIElement StoryContent(InsightsSkillsHome.Story story, bool hero, SkillsSnapshot snapshot, DateTimeOffset now) => story switch
    {
        InsightsSkillsHome.Story.UpdateStory { Item: var item } => UpdateStory(item, hero, snapshot),
        InsightsSkillsHome.Story.TrendStory { Item: var item } => TrendStory(item, snapshot),
        InsightsSkillsHome.Story.CreatorStory { Item: var item } => CreatorStory(item, hero, snapshot, now),
        InsightsSkillsHome.Story.RollupStory { Item: var item } => RollupStory(item, snapshot, now),
        InsightsSkillsHome.Story.WeekStory { Items: var items } => WeekStory(items, snapshot),
        InsightsSkillsHome.Story.ActiveStory { Items: var items } => ActiveStory(items, snapshot),
        _ => new TextBlock(),
    };

    private UIElement UpdateStory(InsightsSkillsHome.Update item, bool hero, SkillsSnapshot snapshot)
    {
        CatalogSkill skill = item.CatalogSkill;
        string agents = string.Join(", ", item.Agents.Select(_model.AgentLabel));
        var byline = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        TextBlock repository = Caption(skill.Repository);
        repository.TextTrimming = TextTrimming.CharacterEllipsis;
        repository.TextWrapping = TextWrapping.NoWrap;
        byline.Children.Add(repository);
        byline.Children.Add(AgentLogos(item.Agents));
        return CatalogStory(item.Id, hero, skill, item.PublisherName ?? skill.Repository, "skills_home_story_update", item.Name, null,
            T("skills_home_open_skill", "name", item.Name + ", " + agents + ", " + skill.Repository + "/" + skill.Path),
            () => SkillsHomeRoute.Catalog(skill.Id, item.InstalledId, snapshot), byline);
    }

    private UIElement CreatorStory(InsightsSkillsHome.CreatorUpdate item, bool hero, SkillsSnapshot snapshot, DateTimeOffset now) =>
        CatalogStory("creator:" + item.Id, hero, item.Skill, item.Publisher.Name, "skills_home_story_" + item.Event.Kind, item.Skill.Name,
            When(item.ObservedAt, now),
            T("skills_home_open_creator", ("name", item.Skill.Name + ", " + item.Skill.Path), ("publisher", item.Publisher.Name)),
            () => SkillsHomeRoute.Catalog(item.Skill.Id, null, snapshot), Caption(item.Publisher.Name));

    /// <summary>A story with a cover: a tile beside the words or, for the hero, a band above them. Its description opens in place.</summary>
    private UIElement CatalogStory(string id, bool hero, CatalogSkill skill, string publisher, string headline, string name, string? time,
        string view, Func<SkillsHomeRoute?> destination, UIElement byline)
    {
        SkillCoverArt.Spec spec = SkillCoverArt.Spec.For(skill.Id, skill.PublisherId);
        string initials = SkillCoverArt.Initials(publisher);
        bool open = _expanded.Contains(id);
        var words = new StackPanel { Spacing = 4 };
        words.Children.Add(HeadlineRow(T(headline), name, time, hero));
        words.Children.Add(byline);
        if (skill.Description.Length > 0)
        {
            TextBlock description = Note(skill.Description);
            description.MaxLines = open ? 0 : hero ? 3 : 2;
            description.TextTrimming = TextTrimming.CharacterEllipsis;
            description.Margin = new Thickness(0, 2, 0, 0);
            words.Children.Add(description);
        }

        if (open)
        {
            TextBlock path = Mono(skill.Repository + "/" + skill.Path);
            words.Children.Add(path);
            Reveal(path);
        }

        var actions = new Grid { Margin = new Thickness(0, 6, 0, 0) };
        actions.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        actions.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        actions.Children.Add(Link(T("skills_home_view"), () => Open(destination()), view));
        var chevron = new Button
        {
            Content = new FontIcon { Glyph = open ? "" : "", FontSize = 11 },
            Padding = new Thickness(6, 2, 6, 2),
            Background = new SolidColorBrush(Colors.Transparent),
            BorderThickness = new Thickness(0),
        };
        AutomationProperties.SetName(chevron, T("skills_inspect", "name", name));
        chevron.Click += (_, _) =>
        {
            if (!_expanded.Remove(id)) _expanded.Add(id);
            Render();
        };
        Grid.SetColumn(chevron, 1);
        actions.Children.Add(chevron);
        words.Children.Add(actions);

        if (hero)
        {
            var stack = new StackPanel { Spacing = 12 };
            stack.Children.Add(new SkillCover(spec, initials, cornerMark: true, SkillCoverArt.BandRadius) { Height = SkillsHomeMotion.HeroBand });
            stack.Children.Add(words);
            return stack;
        }

        return Beside(new SkillCover(spec, initials, cornerMark: false, SkillCoverArt.TileRadius)
        {
            Width = SkillsHomeMotion.MarkSize, Height = SkillsHomeMotion.MarkSize, VerticalAlignment = VerticalAlignment.Top,
        }, words);
    }

    /// <summary>
    /// A story opening in place: the new line's default opacity transition
    /// runs on the Mac's snappy spring (<c>withAnimation(moves.expand)</c>),
    /// so it fades in along that spring until it settles.
    /// </summary>
    private void Reveal(UIElement element)
    {
        if (!Motion.RollsNumbers) return;
        element.Opacity = 0;
        var frames = new DoubleAnimationUsingKeyFrames();
        foreach ((double fraction, double value) in MonitorMotion.KeyFrames(SkillsHomeMotion.Snappy, SkillsHomeMotion.SnappySettling))
        {
            frames.KeyFrames.Add(new LinearDoubleKeyFrame
            {
                KeyTime = KeyTime.FromTimeSpan(TimeSpan.FromSeconds(fraction * SkillsHomeMotion.SnappySettling)),
                Value = Math.Clamp(value, 0, 1),
            });
        }

        Storyboard.SetTarget(frames, element);
        Storyboard.SetTargetProperty(frames, "Opacity");
        var board = new Storyboard();
        board.Children.Add(frames);
        board.Begin();
    }

    private static Grid Beside(FrameworkElement mark, UIElement words)
    {
        var row = new Grid { ColumnSpacing = 12 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.Children.Add(mark);
        Grid.SetColumn((FrameworkElement)words, 1);
        row.Children.Add(words);
        return row;
    }

    /// <summary>The sentence with the skill's name set strong; the name is placed as plain characters, never parsed.</summary>
    private static TextBlock Headline(string template, string name, bool hero)
    {
        TextBlock block = Line(string.Empty, hero ? "TcTitleSectionTextStyle" : "TcBodyTextStyle");
        int slot = template.IndexOf("{name}", StringComparison.Ordinal);
        if (slot < 0)
        {
            block.Inlines.Add(new Run { Text = name, FontWeight = FontWeights.SemiBold });
            return block;
        }

        if (slot > 0) block.Inlines.Add(new Run { Text = template[..slot] });
        block.Inlines.Add(new Run { Text = name, FontWeight = FontWeights.SemiBold });
        string rest = template[(slot + "{name}".Length)..];
        if (rest.Length > 0) block.Inlines.Add(new Run { Text = rest });
        return block;
    }

    private static UIElement HeadlineRow(string template, string name, string? time, bool hero)
    {
        var row = new Grid { ColumnSpacing = 8 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.Children.Add(Headline(template, name, hero));
        if (!string.IsNullOrEmpty(time))
        {
            TextBlock when = Caption(time);
            when.TextWrapping = TextWrapping.NoWrap;
            Grid.SetColumn(when, 1);
            row.Children.Add(when);
        }

        return row;
    }

    private UIElement RollupStory(InsightsSkillsHome.Rollup item, SkillsSnapshot snapshot, DateTimeOffset now)
    {
        var words = new StackPanel { Spacing = 8 };
        var top = new Grid { ColumnSpacing = 8 };
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        top.Children.Add(Line(T("skills_home_rollup", ("count", N(item.Changes.Count)), ("publisher", item.Publisher.Name))));
        TextBlock when = Caption(When(item.Changes[0].ObservedAt, now));
        Grid.SetColumn(when, 1);
        top.Children.Add(when);
        words.Children.Add(top);
        words.Children.Add(CoverStrip(item.Changes.Select(c => c.Skill).ToArray(), item.Publisher.Name, snapshot));
        return Beside(PublisherTile(item.Publisher), words);
    }

    private SkillCover PublisherTile(SkillsPublisher publisher) =>
        new(SkillCoverArt.Spec.For("", publisher.Id), SkillCoverArt.Initials(publisher.Name), cornerMark: false, SkillCoverArt.TileRadius)
        {
            Width = SkillsHomeMotion.MarkSize, Height = SkillsHomeMotion.MarkSize, VerticalAlignment = VerticalAlignment.Top,
        };

    /// <summary>Skills as a row of covers, each opening its catalog entry.</summary>
    private UIElement CoverStrip(IReadOnlyList<CatalogSkill> skills, string publisher, SkillsSnapshot snapshot)
    {
        var strip = new VariableSizedWrapGrid
        {
            Orientation = Orientation.Horizontal,
            ItemWidth = SkillsHomeMotion.StripMinimum + 8,
            ItemHeight = SkillsHomeMotion.StripHeight + 30,
        };
        foreach (CatalogSkill skill in skills)
        {
            var tile = new StackPanel { Spacing = 4, Margin = new Thickness(0, 0, 8, 8) };
            tile.Children.Add(new SkillCover(SkillCoverArt.Spec.For(skill.Id, skill.PublisherId), "", cornerMark: false, SkillCoverArt.TileRadius)
            {
                Height = SkillsHomeMotion.StripHeight,
            });
            TextBlock label = Caption(skill.Name);
            label.TextWrapping = TextWrapping.NoWrap;
            label.TextTrimming = TextTrimming.CharacterEllipsis;
            tile.Children.Add(label);
            var button = new Button
            {
                Content = tile, Padding = new Thickness(0), Background = new SolidColorBrush(Colors.Transparent),
                BorderThickness = new Thickness(0), HorizontalContentAlignment = HorizontalAlignment.Stretch,
                HorizontalAlignment = HorizontalAlignment.Stretch,
            };
            ToolTipService.SetToolTip(button, skill.Name);
            AutomationProperties.SetName(button, T("skills_home_open_creator", ("name", skill.Name + ", " + skill.Path), ("publisher", publisher)));
            button.Click += (_, _) => Open(SkillsHomeRoute.Catalog(skill.Id, null, snapshot));
            strip.Children.Add(button);
        }

        return strip;
    }

    private UIElement DefaultCreator(InsightsSkillsHome.DefaultCreator creator)
    {
        SkillsSnapshot snapshot = _model.Shown!;
        var words = new StackPanel { Spacing = 8 };
        var top = new Grid { ColumnSpacing = 8 };
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        top.Children.Add(Heading(creator.Publisher.Name));
        UIElement follow = FollowButton(creator.Publisher);
        Grid.SetColumn((FrameworkElement)follow, 1);
        top.Children.Add(follow);
        words.Children.Add(top);
        TextBlock repository = Caption(creator.Publisher.Repository);
        repository.TextTrimming = TextTrimming.CharacterEllipsis;
        repository.TextWrapping = TextWrapping.NoWrap;
        words.Children.Add(repository);
        words.Children.Add(CoverStrip(creator.Skills, creator.Publisher.Name, snapshot));
        return Beside(PublisherTile(creator.Publisher), words);
    }

    private UIElement ActiveStory(IReadOnlyList<InsightsSkillsHome.Active> publishers, SkillsSnapshot snapshot)
    {
        var panel = new StackPanel { Spacing = 16 };
        TextBlock heading = Heading(T("skills_home_active"));
        AutomationProperties.SetHeadingLevel(heading, AutomationHeadingLevel.Level3);
        panel.Children.Add(heading);
        foreach (InsightsSkillsHome.Active item in publishers)
        {
            var words = new StackPanel { Spacing = 8 };
            TextBlock name = Headline("{name}", item.Publisher.Name, false);
            name.TextTrimming = TextTrimming.CharacterEllipsis;
            words.Children.Add(name);
            var counts = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
            var stars = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
            stars.Children.Add(new FontIcon { Glyph = "", FontSize = 9, Foreground = R("TcInkSecondaryBrush") });
            stars.Children.Add(Caption(T("skills_home_stars", "count", InsightsSkillsHome.Active.Short(item.Stars))));
            // The exact count for assistive technology.
            AutomationProperties.SetName(stars, T("skills_home_stars", "count", N(item.Stars)));
            counts.Children.Add(stars);
            counts.Children.Add(Caption(T("skills_home_recent_commits", "count", N(item.Commits))));
            words.Children.Add(counts);
            words.Children.Add(CoverStrip(item.Skills, item.Publisher.Name, snapshot));
            panel.Children.Add(Beside(PublisherTile(item.Publisher), words));
        }

        return panel;
    }

    private UIElement TrendStory(InsightsSkillsHome.Trend item, SkillsSnapshot snapshot)
    {
        Color tint = Tint(item.Agent);
        string line = T(item.CurrentUses > item.PreviousUses ? "skills_home_compare_up" : "skills_home_compare_down",
            ("count", N(item.CurrentUses)), ("previous", N(item.PreviousUses)));
        var mark = new Grid { Width = SkillsHomeMotion.MarkSize, Height = SkillsHomeMotion.MarkSize, VerticalAlignment = VerticalAlignment.Top };
        mark.Children.Add(new Ellipse { Fill = new SolidColorBrush(Color.FromArgb((byte)(0.16 * 255), tint.R, tint.G, tint.B)) });
        FrameworkElement logo = AgentLogo(item.Agent, SkillsHomeMotion.MarkSize * 0.45);
        logo.HorizontalAlignment = HorizontalAlignment.Center;
        logo.VerticalAlignment = VerticalAlignment.Center;
        mark.Children.Add(logo);
        var words = new StackPanel { Spacing = 4 };
        words.Children.Add(HeadlineRow(T(item.Direction == InsightsSkillsHome.Direction.Less ? "skills_home_story_less" : "skills_home_story_more"),
            item.Name, T("skills_home_trend_period"), false));
        if (item.Bars.Count > 0)
        {
            FrameworkElement bars = UsageBars(item.Bars, tint, "trend:" + item.Id);
            AutomationProperties.SetName(bars, T("skills_usage_chart_accessibility", "name", item.Name));
            AutomationProperties.SetHelpText(bars, line);
            words.Children.Add(bars);
        }

        TextBlock figure = Note(line);
        Roll(figure, "trend:" + item.Id, item.CurrentUses,
            value => T(item.CurrentUses > item.PreviousUses ? "skills_home_compare_up" : "skills_home_compare_down",
                ("count", N(value)), ("previous", N(item.PreviousUses))));
        words.Children.Add(figure);
        HyperlinkButton view = Link(T("skills_home_view"), () => Open(SkillsHomeRoute.Usage(item.Id, snapshot)),
            T("skills_home_open_usage", "name", item.Name + ", " + _model.AgentLabel(item.Agent)));
        view.Margin = new Thickness(0, 6, 0, 0);
        words.Children.Add(view);
        return Beside(mark, words);
    }

    /// <summary>Fourteen days: the previous seven muted, the last seven in the agent's tint, rising from 2% once.</summary>
    private FrameworkElement UsageBars(IReadOnlyList<InsightsSkillsHome.TrendBar> bars, Color tint, string key)
    {
        int most = Math.Max(1, bars.Max(b => b.Uses));
        var grid = new Grid { Height = 44, Margin = new Thickness(0, 4, 0, 4), ColumnSpacing = 0 };
        Brush muted = R("TcInkTertiaryBrush");
        for (int i = 0; i < bars.Count; i++)
        {
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            var bar = new Rectangle
            {
                Fill = bars[i].Recent ? new SolidColorBrush(tint) : muted,
                RadiusX = 2, RadiusY = 2,
                VerticalAlignment = VerticalAlignment.Bottom,
                Height = 44.0 * bars[i].Uses / most,
                Margin = new Thickness(2.5, 0, 2.5, 0),
            };
            Grid.SetColumn(bar, i);
            grid.Children.Add(bar);
        }

        var hairline = new Rectangle { Height = 1, Fill = R("TcHairlineBrush"), VerticalAlignment = VerticalAlignment.Bottom };
        Grid.SetColumnSpan(hairline, Math.Max(1, bars.Count));
        grid.Children.Add(hairline);
        Grow(grid, key, vertical: true);
        return grid;
    }

    /// <summary>A chart's bars rise once, when the card first appears (<c>grow</c>: 0.5 s ease-out from 2%).</summary>
    private void Grow(FrameworkElement element, string key, bool vertical)
    {
        var transform = new ScaleTransform();
        element.RenderTransform = transform;
        element.RenderTransformOrigin = vertical ? new Windows.Foundation.Point(0.5, 1) : new Windows.Foundation.Point(0, 0.5);
        if (!Motion.GrowsBars || !_grown.Add(key)) return;
        string property = vertical ? "ScaleY" : "ScaleX";
        if (vertical) transform.ScaleY = SkillsHomeMotion.GrowFrom;
        else transform.ScaleX = SkillsHomeMotion.GrowFrom;
        element.Loaded += (_, _) => Animate(transform, property, 1, SkillsHomeMotion.GrowSeconds, SkillsHomeMotion.GrowFrom);
    }

    /// <summary>The week's most-used skills, each a row that opens its usage.</summary>
    private UIElement WeekStory(IReadOnlyList<InsightsSkillsHome.WeekSkill> skills, SkillsSnapshot snapshot)
    {
        int most = Math.Max(1, skills.Count == 0 ? 1 : skills.Max(s => s.Uses));
        var panel = new StackPanel { Spacing = 12 };
        var top = new Grid();
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        TextBlock heading = Heading(T("skills_home_week"));
        AutomationProperties.SetHeadingLevel(heading, AutomationHeadingLevel.Level3);
        top.Children.Add(heading);
        TextBlock period = Caption(T("skills_home_trend_period"));
        Grid.SetColumn(period, 1);
        top.Children.Add(period);
        panel.Children.Add(top);
        foreach (InsightsSkillsHome.WeekSkill skill in skills)
        {
            Color tint = Tint(skill.Agent);
            var row = new StackPanel { Spacing = 4 };
            var line = new Grid { ColumnSpacing = 8 };
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            line.Children.Add(AgentLogos(new[] { skill.Agent }));
            TextBlock name = Line(skill.Name);
            name.TextWrapping = TextWrapping.NoWrap;
            name.TextTrimming = TextTrimming.CharacterEllipsis;
            Grid.SetColumn(name, 1);
            line.Children.Add(name);
            TextBlock uses = Caption(T("skills_invocations", "count", N(skill.Uses)));
            Roll(uses, "week:" + skill.Id, skill.Uses, value => T("skills_invocations", "count", N(value)));
            Grid.SetColumn(uses, 2);
            line.Children.Add(uses);
            row.Children.Add(line);
            row.Children.Add(Share((double)skill.Uses / most, tint, "week:" + skill.Id));
            var button = new Button
            {
                Content = row, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
                Background = new SolidColorBrush(Colors.Transparent), BorderThickness = new Thickness(0), Padding = new Thickness(0),
            };
            AutomationProperties.SetName(button, T("skills_home_open_usage", "name", skill.Name + ", " + _model.AgentLabel(skill.Agent)));
            AutomationProperties.SetHelpText(button, T("skills_invocations", "count", N(skill.Uses)));
            button.Click += (_, _) => Open(SkillsHomeRoute.Usage(skill.Id, snapshot));
            panel.Children.Add(button);
        }

        return panel;
    }

    /// <summary>A share of the week's most-used skill: a 4-high capsule, growing from 2% once.</summary>
    private FrameworkElement Share(double share, Color tint, string key)
    {
        var track = new Grid { Height = 4 };
        track.Children.Add(new Rectangle { Fill = R("TcSurfaceInsetBrush"), RadiusX = 2, RadiusY = 2 });
        var fill = new Rectangle { Fill = new SolidColorBrush(tint), RadiusX = 2, RadiusY = 2, HorizontalAlignment = HorizontalAlignment.Left };
        double fraction = Math.Max(0.02, Math.Min(1, share));
        track.SizeChanged += (_, e) => fill.Width = e.NewSize.Width * fraction;
        track.Children.Add(fill);
        Grow(fill, key, vertical: false);
        AutomationProperties.SetAccessibilityView(track, AccessibilityView.Raw);
        return track;
    }

    /// <summary>
    /// A count rolls to its new value over the Mac's 0.25 s (<c>.numericText()</c>
    /// under <c>.snappy</c>): from the value this card last showed, never
    /// on the first showing, and not at all when motion is off.
    /// </summary>
    private void Roll(TextBlock block, string key, int value, Func<int, string> format)
    {
        bool had = _shownCounts.TryGetValue(key, out int previous);
        _shownCounts[key] = value;
        if (!had || previous == value || !Motion.RollsNumbers) return;
        var roll = new RollingNumber(block, previous, value, format);
        _rolls.Add(roll);
        roll.Start();
    }

    private sealed class RollingNumber
    {
        private readonly TextBlock _block;
        private readonly int _from, _to;
        private readonly Func<int, string> _format;
        private DateTime _start;
        private bool _running;

        public RollingNumber(TextBlock block, int from, int to, Func<int, string> format)
        {
            _block = block;
            _from = from;
            _to = to;
            _format = format;
        }

        public void Start()
        {
            _start = DateTime.UtcNow;
            _running = true;
            _block.Text = _format(_from);
            CompositionTarget.Rendering += OnFrame;
        }

        public void Finish()
        {
            if (!_running) return;
            _running = false;
            CompositionTarget.Rendering -= OnFrame;
            _block.Text = _format(_to);
        }

        private void OnFrame(object? sender, object e)
        {
            double seconds = (DateTime.UtcNow - _start).TotalSeconds;
            if (seconds >= SkillsHomeMotion.SnappySettling || !WindowClocks.Running)
            {
                Finish();
                return;
            }

            // The spring overshoots by about 0.6%; a count never passes its
            // new value on the way there.
            _block.Text = _format((int)Math.Round(_from + (_to - _from) * Math.Min(1, SkillsHomeMotion.Snappy(seconds))));
        }
    }

    private static string When(DateTimeOffset date, DateTimeOffset now)
    {
        DateTimeOffset local = date.ToLocalTime();
        return local.Date == now.ToLocalTime().Date
            ? local.ToString("t", CultureInfo.CurrentCulture)
            : local.ToString("MMM d", CultureInfo.CurrentCulture);
    }

    // MARK: Agents

    /// <summary>A tool's tint (<c>GlassTool.tint</c>): its token, or the primary ink for a tool without a logo.</summary>
    private static Color Tint(string agent) => R(SkillsToolMark.For(agent)?.TintKey ?? "TcInkPrimaryBrush") is SolidColorBrush brush
        ? brush.Color
        : Colors.Gray;

    private FrameworkElement AgentLogo(string agent, double size)
    {
        if (SkillsToolMark.For(agent) is { } mark)
        {
            var canvas = new Viewbox { Width = size, Height = size };
            var art = new Canvas { Width = mark.Logo.ViewBox.Width, Height = mark.Logo.ViewBox.Height };
            foreach (string data in mark.Logo.Paths)
            {
                art.Children.Add(new Microsoft.UI.Xaml.Shapes.Path
                {
                    Data = (Geometry)XamlBindingHelper.ConvertValue(typeof(Geometry), data),
                    Fill = new SolidColorBrush(Tint(agent)),
                });
            }

            canvas.Child = art;
            AutomationProperties.SetName(canvas, _model.AgentLabel(agent));
            ToolTipService.SetToolTip(canvas, _model.AgentLabel(agent));
            return canvas;
        }

        string label = _model.AgentLabel(agent);
        TextBlock initials = Caption(label.Length > 2 ? label[..2] : label);
        AutomationProperties.SetName(initials, label);
        ToolTipService.SetToolTip(initials, label);
        return initials;
    }

    private UIElement AgentLogos(IEnumerable<string> agents)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
        foreach (string agent in agents) row.Children.Add(AgentLogo(agent, 14));
        return row;
    }

    // MARK: Follow

    private UIElement FollowButton(SkillsPublisher publisher)
    {
        bool followed = _model.Shown?.FollowedPublishers.Contains(publisher.Id) == true;
        bool pending = _model.PendingFollowPublisherId == publisher.Id;
        bool failed = _model.FollowFailurePublisherId == publisher.Id;
        var panel = new StackPanel { Spacing = 4, HorizontalAlignment = HorizontalAlignment.Right };
        var label = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        if (pending) label.Children.Add(new ProgressRing { IsActive = true, Width = 12, Height = 12 });
        else label.Children.Add(new FontIcon { Glyph = followed ? "" : "", FontSize = 12 });
        label.Children.Add(new TextBlock { Text = T(followed ? "skills_following" : "skills_follow") });
        Button control;
        if (followed)
        {
            var flyout = new MenuFlyout();
            var unfollow = new MenuFlyoutItem { Text = T("skills_unfollow") };
            AutomationProperties.SetName(unfollow, T("skills_unfollow_accessibility", "publisher", publisher.Name));
            unfollow.Click += async (_, _) =>
            {
                _refocusId = "skills-follow-" + publisher.Id;
                await _model.SetFollowAsync(publisher.Id, false);
            };
            flyout.Items.Add(unfollow);
            control = new DropDownButton { Content = label, Flyout = flyout };
            AutomationProperties.SetName(control, T("skills_following_accessibility", "publisher", publisher.Name));
        }
        else
        {
            control = new Button { Content = label };
            control.Click += async (_, _) =>
            {
                _refocusId = "skills-follow-" + publisher.Id;
                await _model.SetFollowAsync(publisher.Id, true);
            };
            AutomationProperties.SetName(control, T("skills_follow_accessibility", "publisher", publisher.Name));
        }

        control.IsEnabled = _model.CanFollow;
        AutomationProperties.SetAutomationId(control, "skills-follow-" + publisher.Id);
        if (pending) AutomationProperties.SetHelpText(control, T("skills_follow_saving_accessibility", "publisher", publisher.Name));
        panel.Children.Add(control);
        if (failed)
        {
            TextBlock error = Error(T("skills_follow_failed"));
            error.TextAlignment = TextAlignment.Right;
            AutomationProperties.SetName(error, T("skills_follow_failed_accessibility", "publisher", publisher.Name));
            panel.Children.Add(error);
        }

        return panel;
    }

    // MARK: Index

    private ComboBox Picker(string name, IEnumerable<(string Tag, string Label)> choices, string selected, Action<string> pick)
    {
        var box = new ComboBox { MinWidth = 140 };
        AutomationProperties.SetName(box, name);
        box.Header = name;
        int index = 0, chosen = 0;
        foreach ((string tag, string label) in choices)
        {
            box.Items.Add(new ComboBoxItem { Content = label, Tag = tag });
            if (tag == selected) chosen = index;
            index++;
        }

        box.SelectedIndex = chosen;
        box.SelectionChanged += (_, _) =>
        {
            if (box.SelectedItem is ComboBoxItem { Tag: string tag } && tag != selected) pick(tag);
        };
        return box;
    }

    private UIElement Inventory(SkillsSnapshot snapshot)
    {
        IReadOnlyList<InstalledSkill> skills = _model.InventorySkills(snapshot, _inventoryQuery.Text, _inventoryAgent,
            _inventoryAvailability, _inventorySort, _usageDays);
        IReadOnlyList<InsightsSkillsIndex.Group> groups = InsightsSkillsIndex.Groups(snapshot.Inventory.Skills, skills.Select(s => s.Id).ToArray());
        var body = new List<UIElement>();
        _inventoryQuery.PlaceholderText = T("skills_search");
        AutomationProperties.SetName(_inventoryQuery, T("skills_search"));
        body.Add(_inventoryQuery);
        var filters = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        filters.Children.Add(Picker(T("skills_filter_agent"),
            new[] { ("", T("skills_all_agents")) }.Concat(snapshot.Inventory.Skills.Select(s => s.Agent).Distinct()
                .OrderBy(a => a, StringComparer.Ordinal).Select(a => (a, _model.AgentLabel(a)))),
            _inventoryAgent, tag => { _inventoryAgent = tag; Render(); }));
        filters.Children.Add(Picker(T("skills_filter_availability"),
            new[] { ("", T("skills_all_availability")) }.Concat(new[] { "available", "disabled", "cached", "missing", "unknown" }
                .Select(a => (a, T("skills_availability_" + a)))),
            _inventoryAvailability, tag => { _inventoryAvailability = tag; Render(); }));
        body.Add(filters);
        var order = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        order.Children.Add(Picker(T("skills_window"), InsightsSkillsUsage.Periods.Select(d => (N(d), T("skills_days", "count", N(d)))),
            N(_usageDays), tag => { _usageDays = int.Parse(tag, CultureInfo.InvariantCulture); Render(); }));
        order.Children.Add(Picker(T("skills_sort"), new[] { "name", "sessions", "recent" }.Select(s => (s, T("skills_sort_" + s))),
            _inventorySort, tag => { _inventorySort = tag; Render(); }));
        body.Add(order);
        if (skills.Count == 0)
        {
            body.Add(Note(T(snapshot.Refreshing ? "skills_loading" : snapshot.Inventory.Skills.Count == 0 ? "skills_inventory_empty" : "skills_search_empty")));
        }

        if (!snapshot.Usage.Enabled)
        {
            body.Add(Note(T("skills_analysis_off")));
            if (_reviewSettings is { } review) body.Add(Action(T("skills_review_settings"), () => { _model.InvalidateDetails(); review(); }));
        }

        foreach (InsightsSkillsIndex.Group group in groups) body.Add(InventoryRow(group, snapshot));
        body.Add(Coverage(snapshot));
        return Section("skills_inventory", body.ToArray());
    }

    private UIElement InventoryRow(InsightsSkillsIndex.Group group, SkillsSnapshot snapshot)
    {
        InstalledSkill skill = group.Representative;
        InstalledSkill definitionSkill = group.DefinitionInstallation(snapshot);
        bool open = _selectedSkillId == group.Id;
        var row = new StackPanel { Spacing = 4 };
        var top = new Grid { ColumnSpacing = 8 };
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        top.Children.Add(Heading(skill.Name));
        UIElement logos = AgentLogos(group.Agents);
        Grid.SetColumn((FrameworkElement)logos, 1);
        top.Children.Add(logos);
        var chevron = new FontIcon { Glyph = open ? "" : "", FontSize = 11 };
        Grid.SetColumn(chevron, 2);
        top.Children.Add(chevron);
        var toggle = new Button
        {
            Content = top, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
            Background = new SolidColorBrush(Colors.Transparent), BorderThickness = new Thickness(0), Padding = new Thickness(0),
        };
        AutomationProperties.SetName(toggle, T("skills_inspect", "name", skill.Name + ", " + string.Join(", ", group.Agents.Select(_model.AgentLabel))));
        toggle.Click += (_, _) =>
        {
            _selectedSkillId = open ? null : group.Id;
            _model.InvalidateDetails();
            Render();
        };
        row.Children.Add(toggle);
        if (skill.Description.Length > 0) row.Children.Add(Line(skill.Description));
        row.Children.Add(Caption(string.Join(" · ", group.Installations
            .Select(i => T("skills_availability_" + (snapshot.Inventory.Details.TryGetValue(i.Id, out SkillInstallationDetails? d) ? d.Availability : "unknown")))
            .Distinct().OrderBy(s => s, StringComparer.Ordinal))));
        if (snapshot.Inventory.Details.TryGetValue(skill.Id, out SkillInstallationDetails? details) && details.DefinitionStatus is "invalid" or "unreadable")
        {
            row.Children.Add(Note(T("skills_definition_status_" + details.DefinitionStatus)));
        }

        var usageRows = InsightsSkillsIndex.UsageRows(group, snapshot, _model).Where(r => r.Windows.Any(w => w.Days == _usageDays)).ToArray();
        if (usageRows.Length > 0 && snapshot.Usage.Enabled)
        {
            foreach (SkillsUsageRow usage in usageRows)
            {
                SkillsUsageWindow window = usage.Windows.First(w => w.Days == _usageDays);
                row.Children.Add(Caption(_model.AgentLabel(usage.Agent)));
                row.Children.Add(Caption(_model.PopulationLabel(usage.Population)));
                row.Children.Add(Caption(T("skills_coverage_name_only")));
                row.Children.Add(Line(T("skills_sessions", "count", N(window.Sessions))));
                if (snapshot.Usage.Coverage.Any(c => c.Agent == usage.Agent && c.Status == "explicit_invocations"))
                    row.Children.Add(Line(T("skills_invocations", "count", N(window.Invocations))));
                if (usage.LastSeen is { } seen) row.Children.Add(Caption(T("skills_last_seen", "date", InsightsSkillsModel.Timestamp(seen))));
                if (open)
                {
                    row.Children.Add(Line(T("skills_loads", "count", N(window.Loads))));
                    row.Children.Add(Caption(T("skills_undated", "count", N(usage.UndatedEvents))));
                    row.Children.Add(EvidenceLinks(usage));
                    foreach (SkillsUsageDay day in usage.Daily)
                    {
                        row.Children.Add(Caption(string.Join("  ", day.Day, T("skills_invocations", "count", N(day.Invocations)),
                            T("skills_loads", "count", N(day.Loads)))));
                    }
                }
            }
        }
        else
        {
            row.Children.Add(Note(T(_model.UsageState(skill, snapshot))));
        }

        if (open)
        {
            foreach (InstalledSkill installation in group.Installations)
            {
                var where = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
                where.Children.Add(AgentLogos(new[] { installation.Agent }));
                where.Children.Add(Mono(installation.Location));
                AutomationProperties.SetName(where, _model.AgentLabel(installation.Agent) + ", " + installation.Location);
                row.Children.Add(where);
                row.Children.Add(Caption(_model.ScopeLabel(installation.Scope)));
                if (installation.Id != skill.Id)
                {
                    row.Children.Add(Caption(T("skills_availability_" + (snapshot.Inventory.Details.TryGetValue(installation.Id, out SkillInstallationDetails? d)
                        ? d.Availability : "unknown"))));
                }
            }

            if (_model.Definition?.SkillId == definitionSkill.Id && _model.Definition.Definition is { } body) row.Children.Add(Mono(body));
            else if (_model.DefinitionFailure) row.Children.Add(Note(T("skills_definition_unavailable")));
            else
            {
                row.Children.Add(Note(T("skills_loading")));
                // Once per id until the details are dropped: the model sends nothing for a repeat.
                string id = definitionSkill.Id;
                DispatcherQueue.TryEnqueue(async () => await _model.LoadDefinitionAsync(id));
            }
        }

        return row;
    }

    private UIElement EvidenceLinks(SkillsUsageRow row)
    {
        var panel = new StackPanel { Spacing = 4 };
        var expander = new Expander { Header = T("skills_evidence_links"), Content = panel, HorizontalAlignment = HorizontalAlignment.Stretch };
        IReadOnlyList<SkillsUsageEvidence> links = InsightsSkillsModel.EvidenceLinks(row);
        if (links.Count == 0) panel.Children.Add(Note(T("skills_evidence_unavailable")));
        string who = row.Name + " · " + _model.AgentLabel(row.Agent);
        for (int i = 0; i < links.Count; i++)
        {
            SkillsUsageEvidence link = links[i];
            string count = N(i + 1);
            bool child = row.Population == "delegated";
            panel.Children.Add(Link(T(child ? "skills_evidence_child" : "skills_evidence_session", "count", count), async () =>
            {
                _evidenceOpen = true;
                Render();
                await _model.LoadEvidenceAsync(link.SessionRef);
            }, T(child ? "skills_evidence_child_accessibility" : "skills_evidence_accessibility", ("name", who), ("count", count))));
            if (link.ParentSessionRef is { } parent)
            {
                panel.Children.Add(Link(T("skills_evidence_parent"), async () =>
                {
                    _evidenceOpen = true;
                    Render();
                    await _model.LoadEvidenceAsync(parent);
                }, T("skills_evidence_parent_accessibility", ("name", who), ("count", count))));
            }
        }

        return expander;
    }

    private UIElement Coverage(SkillsSnapshot snapshot)
    {
        var panel = new StackPanel { Spacing = 4 };
        if (snapshot.Inventory.Partial) panel.Children.Add(Note(T("skills_partial_inventory")));
        if (snapshot.Usage.Partial) panel.Children.Add(Note(T("skills_partial_usage")));
        if (snapshot.Usage.LastAnalysisAt is { } checkedAt) panel.Children.Add(Line(T("skills_checked_at", "date", InsightsSkillsModel.Timestamp(checkedAt))));
        if (snapshot.Usage.EarliestEvidence is { } earliest) panel.Children.Add(Line(T("skills_earliest", "date", InsightsSkillsModel.Timestamp(earliest))));
        SkillsUsage usage = snapshot.Usage;
        panel.Children.Add(Caption(T("skills_population_direct")));
        panel.Children.Add(Note(T("skills_sessions_observed", "count", N(usage.SessionsObserved))));
        if (usage.SessionsUnknown > 0) panel.Children.Add(Note(T("skills_sessions_unknown", "count", N(usage.SessionsUnknown))));
        panel.Children.Add(Caption(T("skills_population_delegated")));
        panel.Children.Add(Note(usage.DelegatedSessionsObserved is { } delegated
            ? T("skills_sessions_observed", "count", N(delegated))
            : T("skills_usage_pending")));
        if (usage.DelegatedSessionsUnknown is > 0) panel.Children.Add(Note(T("skills_sessions_unknown", "count", N(usage.DelegatedSessionsUnknown.Value))));
        if (usage.UnclassifiedSessionsObserved is > 0 || usage.UnclassifiedSessionsUnknown is > 0)
        {
            panel.Children.Add(Caption(T("skills_population_unknown")));
            if (usage.UnclassifiedSessionsObserved is { } observed) panel.Children.Add(Note(T("skills_sessions_observed", "count", N(observed))));
            if (usage.UnclassifiedSessionsUnknown is > 0) panel.Children.Add(Note(T("skills_sessions_unknown", "count", N(usage.UnclassifiedSessionsUnknown.Value))));
        }

        foreach (SkillsUsageCoverage coverage in usage.Coverage)
            panel.Children.Add(Line(_model.AgentLabel(coverage.Agent) + " · " + T("skills_coverage_" + coverage.Status)));
        foreach (SkillsInventoryCoverage coverage in snapshot.Inventory.Coverage)
        {
            panel.Children.Add(Mono(coverage.Location));
            panel.Children.Add(Caption(T("skills_status_" + coverage.Status)));
        }

        return new Expander { Header = T("skills_coverage"), Content = panel, HorizontalAlignment = HorizontalAlignment.Stretch };
    }

    // MARK: Usage

    private UIElement Usage(SkillsSnapshot snapshot)
    {
        DateTimeOffset now = DateTimeOffset.Now;
        IReadOnlyList<InsightsSkillsUsage.Skill> ranked = InsightsSkillsUsage.Ranked(snapshot.Usage, _usageDays, now, TimeZoneInfo.Local, _selectedUsageId);
        InsightsSkillsUsage.Skill? picked = _selectedUsageId is null ? ranked.FirstOrDefault() : ranked.FirstOrDefault(s => s.Id == _selectedUsageId);
        IReadOnlyList<InstalledSkill> candidates = InsightsSkillsUsage.RemovalCandidates(snapshot, _usageDays, _model, now, TimeZoneInfo.Local);
        var panel = new StackPanel { Spacing = 16 };
        var periods = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        AutomationProperties.SetName(periods, T("skills_usage_period_label"));
        foreach (int days in InsightsSkillsUsage.Periods)
        {
            var button = new ToggleButton { Content = T("skills_days", "count", N(days)), IsChecked = days == _usageDays };
            button.Checked += (_, _) =>
            {
                if (_usageDays == days) return;
                _usageDays = days;
                DispatcherQueue.TryEnqueue(Render);
            };
            button.Unchecked += (_, _) =>
            {
                if (_usageDays == days) button.IsChecked = true;
            };
            periods.Children.Add(button);
        }

        panel.Children.Add(periods);
        var most = new List<UIElement>();
        if (snapshot.Usage.Partial) most.Add(Note(T("skills_partial_usage")));
        if (ranked.Count == 0)
        {
            most.Add(Note(T(InsightsSkillsUsage.EmptyState(snapshot.Usage, _usageDays))));
            if (!snapshot.Usage.Enabled && _reviewSettings is { } review)
                most.Add(Action(T("skills_review_settings"), () => { _model.InvalidateDetails(); review(); }));
        }

        int top = Math.Max(1, ranked.FirstOrDefault()?.Uses ?? 1);
        foreach (InsightsSkillsUsage.Skill skill in ranked)
        {
            var row = new StackPanel { Spacing = 4, Padding = new Thickness(6) };
            var line = new Grid();
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            line.Children.Add(Heading(skill.Name));
            TextBlock count = Line(T("skills_invocations", "count", N(skill.Uses)));
            Grid.SetColumn(count, 1);
            line.Children.Add(count);
            row.Children.Add(line);
            row.Children.Add(Caption(_model.AgentLabel(skill.Agent)));
            row.Children.Add(Share((double)skill.Uses / top, ChartSeries(), "usage:" + _usageDays + ":" + skill.Id));
            bool selected = picked?.Id == skill.Id;
            var button = new Button
            {
                Content = row, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
                Background = selected ? R("TcSurfaceSelectedBrush") : new SolidColorBrush(Colors.Transparent), BorderThickness = new Thickness(0),
                Padding = new Thickness(0), CornerRadius = new CornerRadius(6),
            };
            AutomationProperties.SetName(button, T("skills_select_usage_accessibility", "name", skill.Name + ", " + _model.AgentLabel(skill.Agent)));
            AutomationProperties.SetHelpText(button, T("skills_invocations", "count", N(skill.Uses)));
            button.Click += (_, _) => { _selectedUsageId = skill.Id; _model.InvalidateDetails(); Render(); };
            most.Add(button);
        }

        panel.Children.Add(Section("skills_most_used", most.ToArray()));
        if (candidates.Count > 0)
        {
            var list = new List<UIElement> { Caption(T("skills_no_use_period", "count", N(_usageDays))) };
            foreach (InstalledSkill skill in candidates)
            {
                list.Add(Heading(skill.Name));
                list.Add(Caption(_model.AgentLabel(skill.Agent)));
                list.Add(Mono(skill.Location));
            }

            panel.Children.Add(Section("skills_removal_candidates", list.ToArray()));
        }

        if (picked is null && _selectedUsageId is not null) panel.Children.Add(Note(T("skills_usage_unavailable")));
        if (picked is not null)
        {
            var card = new StackPanel { Spacing = 6 };
            var line = new Grid();
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            line.Children.Add(Heading(picked.Name));
            TextBlock uses = Note(T("skills_invocations", "count", N(picked.Uses)));
            Grid.SetColumn(uses, 1);
            line.Children.Add(uses);
            card.Children.Add(line);
            card.Children.Add(Caption(_model.AgentLabel(picked.Agent)));
            if (picked.Daily.Count > 0) card.Children.Add(UsageChart(picked));
            panel.Children.Add(Card(card));
        }

        return panel;
    }

    /// <summary>The usage bars' colour: the glass token <c>chartSeries1</c>, the system text colour under high contrast.</summary>
    private static Color ChartSeries() => R("TcChartSeries1Brush") is SolidColorBrush brush ? brush.Color : Colors.Gray;

    /// <summary>One bar per local day across the whole window, so days with no evidence stay as gaps.</summary>
    private UIElement UsageChart(InsightsSkillsUsage.Skill skill)
    {
        DateTime first = skill.Start.ToLocalTime().Date, last = skill.End.ToLocalTime().Date;
        int days = Math.Max(1, (int)(last - first).TotalDays + 1);
        int most = Math.Max(1, skill.Daily.Max(d => d.Uses));
        var uses = skill.Daily.ToDictionary(d => d.Id, d => d.Uses, StringComparer.Ordinal);
        var chart = new Grid { Height = 120 };
        for (int i = 0; i < days; i++)
        {
            chart.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            string day = first.AddDays(i).ToString("yyyy-MM-dd", CultureInfo.InvariantCulture);
            if (!uses.TryGetValue(day, out int count)) continue;
            var bar = new Rectangle
            {
                Fill = new SolidColorBrush(ChartSeries()), VerticalAlignment = VerticalAlignment.Bottom,
                Height = 100.0 * count / most, Margin = new Thickness(days > 40 ? 0.5 : 2, 0, days > 40 ? 0.5 : 2, 0),
            };
            AutomationProperties.SetName(bar, day);
            AutomationProperties.SetHelpText(bar, T("skills_invocations", "count", N(count)));
            Grid.SetColumn(bar, i);
            chart.Children.Add(bar);
        }

        var grid = new Rectangle { Height = 1, Fill = R("TcHairlineBrush"), VerticalAlignment = VerticalAlignment.Bottom };
        Grid.SetColumnSpan(grid, days);
        chart.Children.Add(grid);
        var axis = new Grid();
        axis.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        axis.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        axis.Children.Add(Caption(first.ToString("MMM d", CultureInfo.CurrentCulture)));
        TextBlock end = Caption(last.ToString("MMM d", CultureInfo.CurrentCulture));
        Grid.SetColumn(end, 1);
        axis.Children.Add(end);
        var stack = new StackPanel { Spacing = 4 };
        stack.Children.Add(chart);
        stack.Children.Add(axis);
        AutomationProperties.SetName(stack, T("skills_usage_chart_accessibility", "name", skill.Name + ", " + _model.AgentLabel(skill.Agent)));
        return stack;
    }

    // MARK: Browse and Following

    private UIElement Publishers(SkillsSnapshot snapshot)
    {
        var rows = new List<UIElement>();
        foreach (SkillsPublisher publisher in snapshot.Catalog.Publishers)
        {
            var row = new StackPanel { Spacing = 4 };
            var top = new Grid();
            top.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            top.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            top.Children.Add(Heading(publisher.Name));
            UIElement follow = FollowButton(publisher);
            Grid.SetColumn((FrameworkElement)follow, 1);
            top.Children.Add(follow);
            row.Children.Add(top);
            row.Children.Add(Mono(publisher.Repository));
            if (snapshot.Catalog.PublisherStates.FirstOrDefault(s => s.PublisherId == publisher.Id) is { } state)
            {
                row.Children.Add(Caption(T("skills_status_" + state.Status)));
                if (state.CheckedAt is { } checkedAt) row.Children.Add(Caption(T("skills_checked_at", "date", InsightsSkillsModel.Timestamp(checkedAt))));
            }

            if (Instructions(publisher.InstructionsUrl, publisher.Name) is { } link) row.Children.Add(link);
            rows.Add(row);
        }

        return Section("skills_publishers", rows.ToArray());
    }

    private UIElement Catalog(SkillsSnapshot snapshot)
    {
        var skills = _model.CatalogSkills(snapshot, _catalogQuery.Text)
            .OrderBy(s => s.Id == _selectedCatalogId ? 0 : 1)
            .ThenBy(s => s.Name, Comparer<string>.Create((a, b) => string.Compare(a, b, CultureInfo.CurrentCulture, CompareOptions.IgnoreCase)))
            .ToArray();
        var rows = new List<UIElement>();
        if (snapshot.Catalog.Partial) rows.Add(Note(T("skills_partial_catalog")));
        if (snapshot.Catalog.CheckedAt is { } checkedAt) rows.Add(Caption(T("skills_checked_at", "date", InsightsSkillsModel.Timestamp(checkedAt))));
        _catalogQuery.PlaceholderText = T("skills_search");
        AutomationProperties.SetName(_catalogQuery, T("skills_search"));
        rows.Add(_catalogQuery);
        if (skills.Length == 0) rows.Add(Note(T(snapshot.Catalog.Skills.Count == 0 ? "skills_catalog_empty" : "skills_search_empty")));
        foreach (CatalogSkill skill in skills)
        {
            var row = new StackPanel { Spacing = 4 };
            row.Children.Add(Heading(skill.Name));
            if (skill.Description.Length > 0) row.Children.Add(Line(skill.Description));
            row.Children.Add(Mono(skill.Repository + "/" + skill.Path));
            if (SkillsHomeRoute.CatalogUpdate(skill.Id, skill.Id == _selectedCatalogId ? _selectedCatalogInstallationId : null, snapshot) is { } update)
                row.Children.Add(Caption(T("skills_status_" + update.Status)));
            if (Instructions(skill.InstructionsUrl, skill.Name) is { } link) row.Children.Add(link);
            rows.Add(row);
        }

        return Section("skills_catalog", rows.ToArray());
    }

    private UIElement Following(SkillsSnapshot snapshot)
    {
        var events = snapshot.Feed.Where(e => snapshot.FollowedPublishers.Contains(e.PublisherId)).ToArray();
        var rows = new List<UIElement>();
        if (snapshot.FeedPartial) rows.Add(Note(T("skills_partial_feed")));
        if (events.Length == 0) rows.Add(Note(T("skills_feed_empty")));
        foreach (SkillsFeedEvent e in events)
        {
            CatalogSkill? skill = snapshot.Catalog.Skills.FirstOrDefault(s => s.Id == e.SkillId);
            SkillsPublisher? publisher = snapshot.Catalog.Publishers.FirstOrDefault(p => p.Id == e.PublisherId);
            var row = new StackPanel { Spacing = 4 };
            row.Children.Add(Heading(skill?.Name ?? e.SkillId));
            if (publisher is not null) row.Children.Add(Caption(publisher.Name));
            row.Children.Add(Note(T("skills_feed_" + e.Kind)));
            row.Children.Add(Caption(InsightsSkillsModel.Timestamp(e.ObservedAt)));
            if (skill is not null && Instructions(skill.InstructionsUrl, skill.Name) is { } link) row.Children.Add(link);
            rows.Add(row);
        }

        return Section("skills_feed", rows.ToArray());
    }

    /// <summary>A publisher's instructions: an https link only, opened in the browser.</summary>
    private UIElement? Instructions(string url, string name)
    {
        if (!Uri.TryCreate(url, UriKind.Absolute, out Uri? uri) || uri.Scheme != Uri.UriSchemeHttps || string.IsNullOrEmpty(uri.Host)) return null;
        var link = new HyperlinkButton { Content = T("skills_instructions"), NavigateUri = uri, Padding = new Thickness(0) };
        AutomationProperties.SetName(link, T("skills_instructions_accessibility", "name", name));
        return link;
    }
}

/// <summary>
/// A catalog skill's cover (<c>SkillCover</c>, InsightsSkillsCover.swift:164-202):
/// the publisher's gradient, the skill's pattern in white at 16%, and the
/// publisher's initials. Decorative: hidden from UI Automation, since the
/// story beside it names the skill and the publisher in words.
/// </summary>
public sealed class SkillCover : Grid
{
    private readonly SkillCoverArt.Spec _spec;
    private readonly Canvas _art = new();
    private readonly double _radius;

    public SkillCover(SkillCoverArt.Spec spec, string initials, bool cornerMark, double radius)
    {
        _spec = spec;
        _radius = radius;
        (uint from, uint to) = SkillCoverArt.Palettes[spec.Palette];
        Background = new LinearGradientBrush
        {
            StartPoint = spec.Flipped ? new Windows.Foundation.Point(1, 0) : new Windows.Foundation.Point(0, 0),
            EndPoint = spec.Flipped ? new Windows.Foundation.Point(0, 1) : new Windows.Foundation.Point(1, 1),
            GradientStops = { new GradientStop { Color = Rgb(from), Offset = 0 }, new GradientStop { Color = Rgb(to), Offset = 1 } },
        };
        CornerRadius = new CornerRadius(radius);
        Children.Add(_art);
        if (initials.Length > 0) Children.Add(Mark(initials, cornerMark));
        AutomationProperties.SetAccessibilityView(this, AccessibilityView.Raw);
        SizeChanged += (_, _) => Draw();
    }

    private static Color Rgb(uint rgb) => Color.FromArgb(255, (byte)(rgb >> 16), (byte)(rgb >> 8), (byte)rgb);

    private static UIElement Mark(string initials, bool corner)
    {
        if (corner)
        {
            return new Border
            {
                Background = new SolidColorBrush(Color.FromArgb((byte)(SkillCoverArt.MarkOpacity * 255), 0, 0, 0)),
                CornerRadius = new CornerRadius(99),
                Padding = new Thickness(6, 2, 6, 2),
                Margin = new Thickness(8),
                HorizontalAlignment = HorizontalAlignment.Left,
                VerticalAlignment = VerticalAlignment.Bottom,
                Child = new TextBlock { Text = initials, FontSize = 11, FontWeight = FontWeights.SemiBold, Foreground = new SolidColorBrush(Colors.White) },
            };
        }

        // The Mac's text shadow (black at 0.35, 0.5 down) as a second, offset copy.
        var both = new Grid { HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        both.Children.Add(new TextBlock
        {
            Text = initials, FontSize = 13, FontWeight = FontWeights.SemiBold,
            Foreground = new SolidColorBrush(Color.FromArgb((byte)(SkillCoverArt.ShadowOpacity * 255), 0, 0, 0)),
            RenderTransform = new TranslateTransform { Y = SkillCoverArt.ShadowOffsetY },
        });
        both.Children.Add(new TextBlock { Text = initials, FontSize = 13, FontWeight = FontWeights.SemiBold, Foreground = new SolidColorBrush(Colors.White) });
        return both;
    }

    /// <summary>The pattern for the current size (<c>SkillCoverArt.draw</c>), clipped to the rounded cover.</summary>
    private void Draw()
    {
        double width = ActualWidth, height = ActualHeight;
        _art.Children.Clear();
        if (width <= 0 || height <= 0) return;
        var ink = new SolidColorBrush(Color.FromArgb((byte)Math.Round(SkillCoverArt.InkOpacity * 255), 255, 255, 255));
        Panel host = _art;
        if (_spec.Pattern == SkillCoverArt.Pattern.Bars)
        {
            var rotated = new Canvas
            {
                RenderTransform = new CompositeTransform { TranslateX = width / 2, TranslateY = height / 2, Rotation = _spec.Angle },
            };
            _art.Children.Add(rotated);
            host = rotated;
        }

        foreach (SkillCoverArt.Mark mark in SkillCoverArt.Marks(_spec, width, height))
        {
            Shape shape = mark.Ellipse ? new Ellipse() : new Rectangle();
            shape.Width = mark.Width;
            shape.Height = mark.Height;
            if (mark.Stroke > 0)
            {
                shape.Stroke = ink;
                shape.StrokeThickness = mark.Stroke;
            }
            else
            {
                shape.Fill = ink;
            }

            Canvas.SetLeft(shape, mark.X);
            Canvas.SetTop(shape, mark.Y);
            host.Children.Add(shape);
        }

        Visual visual = ElementCompositionPreview.GetElementVisual(_art);
        Compositor compositor = visual.Compositor;
        CompositionRoundedRectangleGeometry rounded = compositor.CreateRoundedRectangleGeometry();
        rounded.Size = new Vector2((float)width, (float)height);
        rounded.CornerRadius = new Vector2((float)_radius, (float)_radius);
        visual.Clip = compositor.CreateGeometricClip(rounded);
    }
}
