using System;
using System.Collections.Generic;
using System.Linq;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.Look;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

/// <summary>
/// "AI usage this week" (product owner, 2026-10-28; <c>InsightsUsageCards</c>,
/// InsightsUsageCards.swift:24-139): one card per Claude or Codex login found,
/// with what is left of each window the provider reports, when it resets,
/// pace and banked resets, and its Refresh; then the NEAR AI card while NEAR
/// AI was called in the last 31 days. Every card is the one component and
/// ends with the same 7-day bars and "By project", which opens Breakdown on
/// By project. And Breakdown's drill-down itself (<c>InsightsDetailsTable</c>,
/// InsightsCreditViews.swift:229-357). Every word is the core's; an unknown
/// figure is the dash, never a zero.
/// </summary>
public sealed partial class InsightsView
{
    private readonly StackPanel _usageSection = new() { Spacing = 12 };
    private DispatcherQueueTimer? _usageMinute;
    private InsightsCounterWeek? _usageWeek;

    /// <summary>The limits redraw when the model says so, and each minute so resets and pace stay current.</summary>
    private void WireUsageCards()
    {
        ViewModel.Subscriptions.Changed += () => DispatcherQueue.TryEnqueue(RedrawUsageCards);
        _usageMinute = DispatcherQueue.CreateTimer();
        _usageMinute.Interval = TimeSpan.FromSeconds(60);
        _usageMinute.Tick += (_, _) =>
        {
            if (ViewModel.SubscriptionsObserved) RedrawUsageCards();
        };
        _usageMinute.Start();
        Unloaded += (_, _) => _usageMinute?.Stop();
        Loaded += (_, _) => _usageMinute?.Start();
    }

    private void RedrawUsageCards()
    {
        if (_closed) return;
        DrawUsageCards(_usageWeek, Words);
        UpdateExtrasVisibility();
    }

    /// <summary>The section's cards, in <see cref="_usageSection"/>; collapsed when there is no card.</summary>
    private void DrawUsageCards(InsightsCounterWeek? week, Func<string, string?> copy)
    {
        _usageWeek = week;
        _usageSection.Children.Clear();
        IReadOnlyList<InsightsLimitsRows.Row> logins = InsightsLimitsRows.Rows(ViewModel.Subscriptions.SupportedProviders, DateTimeOffset.Now, copy);
        InsightsNearAiCredit? nearAi = week?.NearAiCredit is { } credit && InsightsUsageWords.ShowsNearAi(credit) ? credit : null;
        if (logins.Count > 0 || nearAi is not null)
        {
            _usageSection.Children.Add(Heading(Text(copy, "analytics_usage_title"), 18));
            // Only beside a Refresh button that is not spinning (Kristi's #1687 review).
            if (logins.Count > 0 && !ViewModel.Subscriptions.IsRefreshing)
            {
                _usageSection.Children.Add(Caption(Text(copy, "analytics_subscriptions_refresh_hint")));
            }

            if (ViewModel.Subscriptions.FailureKey is { } failure)
            {
                TextBlock line = Body(Text(copy, failure));
                line.Foreground = ThemeBrushes.Get("TcStatusOutsideTextBrush");
                _usageSection.Children.Add(line);
            }

            var cards = new List<UIElement>();
            foreach (InsightsLimitsRows.Row row in logins) cards.Add(LoginCard(row, week, copy));
            if (nearAi is not null) cards.Add(NearAiCard(nearAi, copy));
            _usageSection.Children.Add(CardFlow(cards, 220, 560));
        }

        _usageSection.Visibility = _usageSection.Children.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    private UIElement LoginCard(InsightsLimitsRows.Row row, InsightsCounterWeek? week, Func<string, string?> copy)
    {
        IReadOnlyList<InsightsUsageWords.Day> bars =
            InsightsUsageWords.TokenBars(week?.UsageByDay, row.Provider == "claude" ? "claude_code" : row.Provider, copy);
        var rows = new List<UIElement>();
        // One headline leads the card (product owner, 2026-10-29): whether the limit lasts until it resets.
        if (row.Headline is { } headline)
        {
            TextBlock lead = Body(headline);
            lead.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            if (row.PaceWarns) lead.Foreground = ThemeBrushes.Get("TcStatusOutsideTextBrush");
            rows.Add(lead);
        }

        if (row.Session is { } session) rows.Add(UsageRow(Text(copy, "analytics_limits_session_left"), session));
        if (row.SessionReset is { } sessionReset) rows.Add(UsageRow(Text(copy, "analytics_usage_session_resets"), sessionReset));
        if (row.Weekly is { } weekly) rows.Add(UsageRow(Text(copy, "analytics_limits_weekly_left"), weekly));
        if (row.WeeklyReset is { } weeklyReset) rows.Add(UsageRow(Text(copy, "analytics_usage_weekly_resets"), weeklyReset));
        if (row.Headline is null) rows.Add(UsageRow(Text(copy, "analytics_limits_pace"), row.Pace, row.PaceWarns));
        if (row.Banked is { } banked) rows.Add(UsageRow(Text(copy, "analytics_subscriptions_banked"), banked));
        var panel = new StackPanel { Spacing = 8 };
        foreach (UIElement line in rows) panel.Children.Add(line);
        if (!row.Measured) panel.Children.Add(Caption(Text(copy, "analytics_limits_refresh_to_see")));
        if (row.Freshness is { } freshness) panel.Children.Add(Caption(freshness));
        // This shell has no "i": Claude's note is a caption under the title.
        string? note = row.Provider == "claude" ? Text(copy, "analytics_subscriptions_claude_detail") : null;
        return UsageCard(title: row.Account, note, RefreshButton(row, copy), panel, bars, copy);
    }

    private GlassRefreshButton RefreshButton(InsightsLimitsRows.Row row, Func<string, string?> copy)
    {
        string name = Text(copy, "analytics_subscriptions_provider_" + row.Provider);
        var button = new GlassRefreshButton
        {
            Content = MonitorShellCopy.Shell("settings_refresh"),
            IsRefreshing = ViewModel.Subscriptions.IsRefreshingLogin(row.Provider, row.LoginId),
            VerticalAlignment = VerticalAlignment.Top,
        };
        button.IsEnabled = ViewModel.Subscriptions.CanRefresh(row.Provider, row.LoginId);
        AutomationProperties.SetName(button, InsightsAnalyticsWords.Fill(Text(copy, "analytics_subscriptions_refresh_login_accessibility"),
            ("harness", name), ("login", row.LoginLabel)));
        button.Click += (_, _) => _ = ViewModel.Subscriptions.Refresh(row.Provider, row.LoginId);
        return button;
    }

    private UIElement NearAiCard(InsightsNearAiCredit credit, Func<string, string?> copy)
    {
        var panel = new StackPanel { Spacing = 8 };
        panel.Children.Add(UsageRow(Text(copy, "analytics_this_week"), InsightsCreditWords.SpendAndTokens(credit.Week, copy)));
        panel.Children.Add(UsageRow(Text(copy, "analytics_today"), InsightsCreditWords.SpendAndTokens(credit.Today, copy)));
        panel.Children.Add(UsageRow(Text(copy, "analytics_usage_last_31_days"), InsightsCreditWords.SpendAndTokens(credit.Last31Days, copy)));
        // This shell has no "i": what the figures cover is a note under the title.
        return UsageCard(title: Text(copy, "analytics_usage_near_ai"), Text(copy, "analytics_credit_scope_near_ai"), null, panel,
            InsightsUsageWords.Days(credit, copy), copy);
    }

    /// <summary>
    /// One usage card (<c>InsightsUsageCard</c>, InsightsUsageCards.swift:141-182):
    /// the title and its control, the rows, the 7-day bars when the daemon
    /// sent days, then "By project".
    /// </summary>
    private UIElement UsageCard(string title, string? note, UIElement? control, UIElement rows,
        IReadOnlyList<InsightsUsageWords.Day> bars, Func<string, string?> copy)
    {
        var panel = new StackPanel { Spacing = 10 };
        var header = new Grid { ColumnSpacing = 8, MinHeight = 32 };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        TextBlock heading = Heading(title, 15);
        heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        heading.VerticalAlignment = VerticalAlignment.Center;
        header.Children.Add(heading);
        if (control is FrameworkElement element)
        {
            Grid.SetColumn(element, 1);
            header.Children.Add(element);
        }

        panel.Children.Add(header);
        if (note is { Length: > 0 }) panel.Children.Add(Caption(note));
        panel.Children.Add(rows);
        // No bars from a daemon that sends no days, rather than an empty plot.
        if (bars.Count > 0) panel.Children.Add(UsageBars(bars, Text(copy, "analytics_usage_last_7_days")));
        var projects = new HyperlinkButton { Content = Text(copy, "analytics_usage_by_project"), Padding = new Thickness(0) };
        AutomationProperties.SetName(projects, InsightsAnalyticsWords.Fill(Text(copy, "analytics_usage_by_project_a11y"), ("label", title)));
        projects.Click += (_, _) => OnTabPressed(ViewModel.ShowProjects());
        panel.Children.Add(projects);
        Border card = Card(panel);
        card.VerticalAlignment = VerticalAlignment.Stretch;
        return card;
    }

    /// <summary>
    /// Cards side by side, as many columns as fit at <paramref name="minimum"/>
    /// wide, each at most <paramref name="maximum"/>, wrapping when the pane
    /// is narrow (<c>InsightsCardFlow</c>).
    /// </summary>
    private static UIElement CardFlow(IReadOnlyList<UIElement> cards, double minimum, double maximum)
    {
        const double gap = 12;
        var grid = new Grid { ColumnSpacing = gap, RowSpacing = gap, HorizontalAlignment = HorizontalAlignment.Left };
        int columns = -1;
        void Lay(double width)
        {
            int fit = Math.Max(1, Math.Min(cards.Count, (int)((width + gap) / (minimum + gap))));
            if (fit == columns) return;
            columns = fit;
            grid.Children.Clear();
            grid.ColumnDefinitions.Clear();
            grid.RowDefinitions.Clear();
            for (int c = 0; c < fit; c++) grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            for (int r = 0; r < (cards.Count + fit - 1) / fit; r++) grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            for (int i = 0; i < cards.Count; i++)
            {
                var cell = (FrameworkElement)cards[i];
                Grid.SetColumn(cell, i % fit);
                Grid.SetRow(cell, i / fit);
                grid.Children.Add(cell);
            }

            grid.MaxWidth = fit * maximum + (fit - 1) * gap;
        }

        var host = new Grid();
        host.Children.Add(grid);
        host.SizeChanged += (_, args) => Lay(args.NewSize.Width);
        Lay(minimum);
        return host;
    }

    /// <summary>The section shows while any of its parts has something to draw.</summary>
    private void UpdateExtrasVisibility() =>
        OverviewExtras.Visibility = OverviewExtras.Children.Any(child => child.Visibility == Visibility.Visible)
            ? Visibility.Visible
            : Visibility.Collapsed;

    // ---- Breakdown's drill-down (InsightsCreditViews.swift:229-357) ---------

    /// <summary>By project (the default, and what each usage card opens) and By model, under feed T only.</summary>
    private UIElement DetailsTable(InsightsCounterWeek week, Func<string, string?> copy)
    {
        IReadOnlyList<InsightsDetailsWords.Dimension> dimensions = InsightsDetailsWords.Dimensions(week);
        // A view this week has nothing for falls back to By project.
        InsightsDetailsWords.Dimension shown = InsightsDetailsWords.Shown(ViewModel.DetailsView, week);
        var panel = new StackPanel { Spacing = 10 };
        string title = Text(copy, "analytics_credit_title");
        panel.Children.Add(Heading(title, 18));
        // This shell has no "i": what the figures cover is a note under the title.
        panel.Children.Add(Caption(WindowsWords.InsightsCreditScope));
        if (dimensions.Count > 1)
        {
            var choices = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
            AutomationProperties.SetName(choices, title);
            foreach (InsightsDetailsWords.Dimension dimension in dimensions)
            {
                var choice = new RadioButton
                {
                    Content = Text(copy, dimension == InsightsDetailsWords.Dimension.Folder ? "analytics_card_by_project" : "analytics_card_by_model"),
                    GroupName = "InsightsDetailsView",
                    IsChecked = dimension == shown,
                };
                choice.Checked += (_, _) =>
                {
                    if (ViewModel.DetailsView == dimension) return;
                    ViewModel.DetailsView = dimension;
                    DrawWeekExtras();
                };
                choices.Children.Add(choice);
            }

            panel.Children.Add(choices);
        }

        panel.Children.Add(shown == InsightsDetailsWords.Dimension.Folder ? ProjectsTable(week, copy) : ModelsList(week, copy));
        return Card(panel);
    }

    /// <summary>One row per folder: spend at API prices, Claude Code and Codex tokens with their traces, and NEAR AI spend.</summary>
    private static UIElement ProjectsTable(InsightsCounterWeek week, Func<string, string?> copy)
    {
        IReadOnlyList<InsightsProjectRows.Row> rows = InsightsProjectRows.Rows(week, copy);
        if (rows.Count == 0) return Caption(Text(copy, "analytics_unavailable"));
        InsightsProjectRows.Columns columns = InsightsProjectRows.ColumnsOf(week, rows);
        var headings = new List<string> { Text(copy, "analytics_projects_project") };
        if (columns.Spend) headings.Add(Text(copy, "analytics_column_spend"));
        if (columns.Claude) headings.Add(Text(copy, "analytics_credit_tool_claude_code"));
        if (columns.Codex) headings.Add(Text(copy, "analytics_credit_tool_codex"));
        if (columns.NearAi) headings.Add(Text(copy, "analytics_usage_near_ai"));
        var grid = new Grid { ColumnSpacing = 24, RowSpacing = 10 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        for (int c = 1; c < headings.Count; c++) grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        for (int c = 0; c < headings.Count; c++)
        {
            TextBlock heading = Caption(headings[c]);
            if (c > 0) heading.HorizontalAlignment = HorizontalAlignment.Right;
            Place(grid, heading, 0, c);
        }

        int line = 1;
        foreach (InsightsProjectRows.Row row in rows)
        {
            grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            var cells = new List<(string Main, string? Under)> { (row.Label, null) };
            if (columns.Spend) cells.Add((row.Spend ?? Text(copy, "analytics_unavailable"), null));
            if (columns.Claude) cells.Add((InsightsProjectRows.Tokens(row.Claude, copy), InsightsProjectRows.Traces(row.Claude, copy)));
            if (columns.Codex) cells.Add((InsightsProjectRows.Tokens(row.Codex, copy), InsightsProjectRows.Traces(row.Codex, copy)));
            if (columns.NearAi) cells.Add((row.NearAi, row.NearAiTokens));
            for (int c = 0; c < cells.Count; c++)
            {
                UIElement cell;
                if (c == 0)
                {
                    TextBlock label = Body(cells[c].Main);
                    label.MaxLines = 2;
                    label.TextTrimming = TextTrimming.CharacterEllipsis;
                    cell = label;
                }
                else
                {
                    var stack = new StackPanel { HorizontalAlignment = HorizontalAlignment.Right };
                    TextBlock figure = Mono(cells[c].Main);
                    figure.HorizontalAlignment = HorizontalAlignment.Right;
                    stack.Children.Add(figure);
                    if (cells[c].Under is { } under)
                    {
                        TextBlock caption = Caption(under);
                        caption.HorizontalAlignment = HorizontalAlignment.Right;
                        stack.Children.Add(caption);
                    }

                    cell = stack;
                }

                Place(grid, cell, line, c);
            }

            line++;
        }

        return grid;
    }

    private static void Place(Grid grid, UIElement element, int row, int column)
    {
        Grid.SetRow((FrameworkElement)element, row);
        Grid.SetColumn((FrameworkElement)element, column);
        grid.Children.Add(element);
    }

    /// <summary>Claude and Codex spend by model at API prices, then NEAR AI's at the ledger's charge, each with its tokens.</summary>
    private static UIElement ModelsList(InsightsCounterWeek week, Func<string, string?> copy)
    {
        IReadOnlyList<InsightsDetailsWords.ModelLine> lines = InsightsDetailsWords.ModelLines(week, copy);
        if (lines.Count == 0) return Caption(Text(copy, "analytics_unavailable"));
        var panel = new StackPanel();
        for (int i = 0; i < lines.Count; i++)
        {
            InsightsDetailsWords.ModelLine model = lines[i];
            var row = new Grid
            {
                ColumnSpacing = 16,
                Padding = new Thickness(0, 6, 0, 6),
                BorderBrush = ThemeBrushes.Get("TcHairlineBrush"),
                BorderThickness = new Thickness(0, i == 0 ? 0 : 1, 0, 0),
            };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(72) });
            var name = new StackPanel();
            name.Children.Add(Body(model.Label));
            name.Children.Add(Caption(model.Tool));
            row.Children.Add(name);
            TextBlock tokens = Caption(model.Tokens);
            tokens.VerticalAlignment = VerticalAlignment.Center;
            Grid.SetColumn(tokens, 1);
            row.Children.Add(tokens);
            TextBlock amount = Mono(model.Amount);
            amount.HorizontalAlignment = HorizontalAlignment.Right;
            Grid.SetColumn(amount, 2);
            row.Children.Add(amount);
            AutomationProperties.SetName(row, string.Join(", ", model.Label, model.Tool, model.Tokens, model.Amount));
            panel.Children.Add(row);
        }

        if (InsightsDetailsWords.CacheSavings(week, copy) is { } savings)
        {
            panel.Children.Add(UsageRow(Text(copy, "analytics_cache_savings"), savings));
        }

        return panel;
    }
}
