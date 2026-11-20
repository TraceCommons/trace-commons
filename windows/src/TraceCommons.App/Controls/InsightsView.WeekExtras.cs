using System;
using System.Collections.Generic;
using System.Linq;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using TraceCommons.App.Look;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

/// <summary>
/// Feed T's parts beyond the week itself, as the Mac draws them (W20 round 2):
/// on Overview the weekly summary card, the spend and top-model tiles, the
/// NEAR AI usage card, spend by model and trends
/// (<c>InsightsOverviewTab</c>, InsightsOverviewTab.swift:61-100); on
/// Breakdown the goals set with <c>/goal</c> (InsightsPatternsTab.swift:56-60,
/// :97-128). Each is drawn only behind the Mac's gate, every word is the
/// core's, and an unknown figure is the dash. Under the saved feed none of it
/// is drawn. Built in code, into the two slots the XAML leaves for it.
/// </summary>
public sealed partial class InsightsView
{
    private InsightsCounterWeek? _drawnWeek;
    private InsightsRecap? _drawnRecap;
    private bool _extrasWired;

    private Func<string, string?> Words => key => ViewModel[key] is { Length: > 0 } word ? word : null;

    /// <summary>Follows the window's week and the summary card; called once the control is built.</summary>
    private void WireWeekExtras()
    {
        if (_extrasWired) return;
        _extrasWired = true;
        ViewModel.PropertyChanged += (_, _) =>
        {
            if (!ReferenceEquals(ViewModel.CounterWeek, _drawnWeek)) DrawWeekExtras();
        };
        ViewModel.Comparisons.Changed += () => DispatcherQueue.TryEnqueue(() =>
        {
            if (!ReferenceEquals(ViewModel.Comparisons.Recap, _drawnRecap)) DrawWeekExtras();
        });
        ActualThemeChanged += (_, _) => DrawWeekExtras();
        DrawWeekExtras();
    }

    private void DrawWeekExtras()
    {
        if (_closed) return;
        InsightsCounterWeek? week = ViewModel.CounterFeed ? ViewModel.CounterWeek : null;
        _drawnWeek = ViewModel.CounterWeek;
        _drawnRecap = ViewModel.Comparisons.Recap;
        Func<string, string?> copy = Words;

        OverviewExtras.Children.Clear();
        if (week is not null && ViewModel.Comparisons.Recap is { } recap)
        {
            OverviewExtras.Children.Add(RecapCard(recap, copy));
        }

        IReadOnlyList<InsightsWorkWords.Tile> tiles = InsightsWorkWords.Tiles(week?.Work, copy);
        if (tiles.Count > 0)
        {
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
            foreach (InsightsWorkWords.Tile tile in tiles) row.Children.Add(TileCard(tile));
            OverviewExtras.Children.Add(row);
        }

        if (week?.NearAiCredit is { } credit && InsightsUsageWords.ShowsNearAi(credit))
        {
            OverviewExtras.Children.Add(Heading(Text(copy, "analytics_usage_title"), 18));
            OverviewExtras.Children.Add(NearAiCard(credit, copy));
        }

        if (week?.Work is { } work)
        {
            if (InsightsWorkWords.ModelRows(work, copy).Count > 0) OverviewExtras.Children.Add(SpendByModelCard(work, copy));
            if (InsightsWorkWords.ShowsTrends(work, week.ChangeVsLastWeek, copy)) OverviewExtras.Children.Add(TrendsCard(work, week.ChangeVsLastWeek, copy));
        }

        OverviewExtras.Visibility = OverviewExtras.Children.Count > 0 ? Visibility.Visible : Visibility.Collapsed;

        PatternsGoals.Children.Clear();
        if (InsightsGoalsWords.Shows(week)) PatternsGoals.Children.Add(GoalsCard(week!.Goals!, copy));
        PatternsGoals.Visibility = PatternsGoals.Children.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    // ---- The weekly summary card (InsightsOverviewTab.swift:127-167) --------

    private UIElement RecapCard(InsightsRecap recap, Func<string, string?> copy)
    {
        var panel = new StackPanel { Spacing = 10 };
        panel.Children.Add(Heading(InsightsComparisonsWords.RecapTitle(recap, copy), 18));
        var sources = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 32 };
        foreach (InsightsRecapSource source in recap.Sources)
        {
            var column = new StackPanel { Spacing = 4 };
            column.Children.Add(Figure(InsightsAnalyticsWords.Figure(copy, source.Tokens)));
            column.Children.Add(Caption(InsightsAnalyticsWords.SourceLine(copy, source.Source)));
            if (InsightsComparisonsWords.RecapChange(source, recap.Sessions, copy) is { } change) column.Children.Add(Caption(change));
            sources.Children.Add(column);
        }

        panel.Children.Add(sources);
        foreach (InsightsRecapItem item in recap.Items)
        {
            if (InsightsComparisonsWords.RecapItem(item, copy) is { } line) panel.Children.Add(Body(line));
        }

        if (InsightsComparisonsWords.RecapThreshold(recap, copy) is { } threshold) panel.Children.Add(Body(threshold));

        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        var open = new Button { Content = Text(copy, "analytics_recap_open"), Style = (Style)Application.Current.Resources["AccentButtonStyle"] };
        open.Click += async (_, _) => await ViewModel.OpenRecapAsync();
        actions.Children.Add(open);
        // The decline is a link, after the action (Ron's #1287 review).
        if (ViewModel.CanHideRecap)
        {
            var hide = new HyperlinkButton { Content = Text(copy, "analytics_recap_turn_off") };
            hide.Click += async (_, _) => await ViewModel.HideRecapAsync();
            actions.Children.Add(hide);
        }

        panel.Children.Add(actions);
        return Card(panel);
    }

    // ---- The tiles (InsightsWeekTiles, InsightsWorkViews.swift:178-218) -----

    private static UIElement TileCard(InsightsWorkWords.Tile tile)
    {
        var panel = new StackPanel { Spacing = 4 };
        panel.Children.Add(Eyebrow(tile.Label));
        panel.Children.Add(Figure(tile.Value));
        panel.Children.Add(Caption(tile.Note));
        Border card = Card(panel);
        card.Width = 240;
        AutomationProperties.SetName(card, string.Join(", ", tile.Label, tile.Value, tile.Note));
        return card;
    }

    // ---- NEAR AI (InsightsUsageCards.swift:125-145) -------------------------

    private static UIElement NearAiCard(InsightsNearAiCredit credit, Func<string, string?> copy)
    {
        var panel = new StackPanel { Spacing = 8 };
        string name = Text(copy, "analytics_usage_near_ai");
        TextBlock title = Heading(name, 15);
        panel.Children.Add(title);
        // This shell has no "i": what the figures cover is a note under the title.
        panel.Children.Add(Caption(Text(copy, "analytics_credit_scope_near_ai")));
        panel.Children.Add(UsageRow(Text(copy, "analytics_this_week"), InsightsCreditWords.SpendAndTokens(credit.Week, copy)));
        panel.Children.Add(UsageRow(Text(copy, "analytics_today"), InsightsCreditWords.SpendAndTokens(credit.Today, copy)));
        panel.Children.Add(UsageRow(Text(copy, "analytics_usage_last_31_days"), InsightsCreditWords.SpendAndTokens(credit.Last31Days, copy)));
        IReadOnlyList<InsightsUsageWords.Day> days = InsightsUsageWords.Days(credit, copy);
        // No bars from a daemon that sends no days, rather than an empty plot.
        if (days.Count > 0) panel.Children.Add(UsageBars(days, Text(copy, "analytics_usage_last_7_days")));
        Border card = Card(panel);
        card.MaxWidth = 560;
        card.HorizontalAlignment = HorizontalAlignment.Left;
        return card;
    }

    private static UIElement UsageRow(string label, string value)
    {
        var grid = new Grid { ColumnSpacing = 12 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.Children.Add(Caption(label));
        TextBlock figure = Body(value);
        figure.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        Grid.SetColumn(figure, 1);
        grid.Children.Add(figure);
        AutomationProperties.SetName(grid, label + ", " + value);
        return grid;
    }

    /// <summary>The last 7 days, oldest first; an unknown day draws no bar and reads as the dash.</summary>
    private static UIElement UsageBars(IReadOnlyList<InsightsUsageWords.Day> days, string caption)
    {
        var panel = new StackPanel { Spacing = 4 };
        panel.Children.Add(Caption(caption));
        double top = Math.Max(days.Max(d => d.Value ?? 0), double.Epsilon);
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
        foreach (InsightsUsageWords.Day day in days)
        {
            var column = new StackPanel { Spacing = 2, Width = 24 };
            var track = new Grid { Height = 56 };
            if (day.Value is { } value)
            {
                track.Children.Add(new Rectangle
                {
                    Fill = ThemeBrushes.Get("TcAccentBrush"),
                    Height = Math.Max(1, 56 * value / top),
                    VerticalAlignment = VerticalAlignment.Bottom,
                    RadiusX = 2,
                    RadiusY = 2,
                });
            }

            column.Children.Add(track);
            TextBlock tick = Caption(day.Label);
            tick.HorizontalAlignment = HorizontalAlignment.Center;
            column.Children.Add(tick);
            ToolTipService.SetToolTip(column, day.Amount);
            AutomationProperties.SetName(column, day.Label + ", " + day.Amount);
            row.Children.Add(column);
        }

        panel.Children.Add(row);
        return panel;
    }

    // ---- Spend by model (InsightsWorkViews.swift:221-268) -------------------

    private UIElement SpendByModelCard(InsightsWork work, Func<string, string?> copy)
    {
        IReadOnlyList<InsightsWorkWords.ModelRow> rows = InsightsWorkWords.ModelRows(work, copy);
        // The tool under each model only when there are two to tell apart.
        bool tools = rows.Select(r => r.Tool).Distinct(StringComparer.Ordinal).Count() > 1;
        var panel = new StackPanel { Spacing = 8 };
        var header = new Grid { ColumnSpacing = 12 };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.Children.Add(Heading(Text(copy, "analytics_by_model_title"), 18));
        TextBlock prices = Caption(InsightsWorkWords.ApiPrices(work, copy));
        prices.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(prices, 1);
        header.Children.Add(prices);
        panel.Children.Add(header);
        foreach (InsightsWorkWords.ModelRow row in rows)
        {
            var grid = new Grid { ColumnSpacing = 12 };
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(160) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(80) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(44) });
            var name = new StackPanel();
            TextBlock label = Body(row.Label);
            label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            label.TextWrapping = TextWrapping.NoWrap;
            label.TextTrimming = TextTrimming.CharacterEllipsis;
            name.Children.Add(label);
            if (tools) name.Children.Add(Caption(row.Tool));
            grid.Children.Add(name);
            var track = new Grid { Height = 8, VerticalAlignment = VerticalAlignment.Center };
            track.Children.Add(new Rectangle { Fill = ThemeBrushes.Get("TcSurfaceInsetBrush"), RadiusX = 4, RadiusY = 4 });
            var fill = new Rectangle { Fill = ThemeBrushes.Get("TcAccentBrush"), RadiusX = 4, RadiusY = 4, HorizontalAlignment = HorizontalAlignment.Left };
            track.Children.Add(fill);
            // After the bar is in its track: the width follows the track's.
            SetFraction(fill, row.Share);
            Grid.SetColumn(track, 1);
            grid.Children.Add(track);
            TextBlock amount = Mono(row.Amount);
            amount.HorizontalAlignment = HorizontalAlignment.Right;
            Grid.SetColumn(amount, 2);
            grid.Children.Add(amount);
            TextBlock percent = Caption(row.Percent);
            percent.HorizontalAlignment = HorizontalAlignment.Right;
            Grid.SetColumn(percent, 3);
            grid.Children.Add(percent);
            AutomationProperties.SetName(grid, string.Join(", ", tools ? new[] { row.Label, row.Tool, row.Amount, row.Percent } : new[] { row.Label, row.Amount, row.Percent }));
            panel.Children.Add(grid);
        }

        var benchmarks = new HyperlinkButton { Content = Text(copy, "analytics_tab_benchmarks") };
        benchmarks.Click += (_, _) => OnTabPressed(InsightsViewModel.BenchmarksTab);
        panel.Children.Add(benchmarks);
        return Card(panel);
    }

    // ---- Trends (InsightsWorkViews.swift:270-319) ---------------------------

    private static UIElement TrendsCard(InsightsWork work, IReadOnlyList<InsightsWeekChange>? tokens, Func<string, string?> copy)
    {
        IReadOnlyList<InsightsWorkWords.DayPair> days = InsightsWorkWords.Days(work);
        IReadOnlyList<InsightsWorkWords.ChangeLine> changes = InsightsWorkWords.Changes(work, tokens, copy);
        string thisWeek = Text(copy, "analytics_this_week"), before = Text(copy, "analytics_trends_week_before");
        var panel = new StackPanel { Spacing = 8 };
        panel.Children.Add(Heading(Text(copy, "analytics_trends_title"), 18));
        var body = new Grid { ColumnSpacing = 24 };
        body.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        body.ColumnDefinitions.Add(new ColumnDefinition { Width = changes.Count > 0 ? new GridLength(260) : new GridLength(0) });

        var chart = new StackPanel { Spacing = 4 };
        chart.Children.Add(Caption(Text(copy, "analytics_trends_spend_per_day")));
        double top = Math.Max(days.Max(d => Math.Max(d.Week, d.LastWeek ?? 0)), double.Epsilon);
        var bars = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 14 };
        foreach (InsightsWorkWords.DayPair day in days)
        {
            var column = new StackPanel { Spacing = 2 };
            var pair = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2, Height = 120 };
            if (day.LastWeek is { } last) pair.Children.Add(Bar(last, top, "TcSurfaceInsetBrush"));
            pair.Children.Add(Bar(day.Week, top, "TcAccentBrush"));
            column.Children.Add(pair);
            TextBlock tick = Caption(day.Label);
            tick.HorizontalAlignment = HorizontalAlignment.Center;
            column.Children.Add(tick);
            string read = day.Label + ", " + thisWeek + " " + InsightsWorkWords.Dollars((long)Math.Round(day.Week * 1_000_000))
                + (day.LastWeek is { } l ? ", " + before + " " + InsightsWorkWords.Dollars((long)Math.Round(l * 1_000_000)) : "");
            ToolTipService.SetToolTip(column, read);
            AutomationProperties.SetName(column, read);
            bars.Children.Add(column);
        }

        chart.Children.Add(bars);
        var legend = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 16 };
        legend.Children.Add(Swatch(thisWeek, "TcAccentBrush"));
        legend.Children.Add(Swatch(before, "TcSurfaceInsetBrush"));
        chart.Children.Add(legend);
        body.Children.Add(chart);

        if (changes.Count > 0)
        {
            var list = new StackPanel { Spacing = 6 };
            foreach (InsightsWorkWords.ChangeLine change in changes) list.Children.Add(UsageRow(change.Label, change.Words));
            Grid.SetColumn(list, 1);
            body.Children.Add(list);
        }

        panel.Children.Add(body);
        return Card(panel);
    }

    private static UIElement Bar(double value, double top, string brush) => new Rectangle
    {
        Fill = ThemeBrushes.Get(brush),
        Width = 12,
        Height = Math.Max(value > 0 ? 1 : 0, 120 * value / top),
        VerticalAlignment = VerticalAlignment.Bottom,
        RadiusX = 2,
        RadiusY = 2,
    };

    private static UIElement Swatch(string label, string brush)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        row.Children.Add(new Rectangle { Fill = ThemeBrushes.Get(brush), Width = 10, Height = 10, RadiusX = 2, RadiusY = 2, VerticalAlignment = VerticalAlignment.Center });
        row.Children.Add(Caption(label));
        return row;
    }

    // ---- Goals (InsightsPatternsTab.swift:97-128) ---------------------------

    private static UIElement GoalsCard(InsightsWeekGoals goals, Func<string, string?> copy)
    {
        var panel = new StackPanel { Spacing = 8 };
        panel.Children.Add(Heading(Text(copy, "analytics_goals_title"), 18));
        // This shell has no "i": where the goals come from is a note under the title.
        panel.Children.Add(Caption(Text(copy, "analytics_goals_note")));
        panel.Children.Add(Figure(InsightsGoalsWords.MetOf(goals, copy)));
        foreach (InsightsWeekGoal goal in goals.Goals)
        {
            var row = new Grid { ColumnSpacing = 12, Padding = new Thickness(0, 6, 0, 6), BorderBrush = ThemeBrushes.Get("TcHairlineBrush"), BorderThickness = new Thickness(0, 1, 0, 0) };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            string tool = InsightsGoalsWords.Tool(goal.Tool, copy);
            string status = InsightsGoalsWords.Status(goal.Status, copy);
            string line = InsightsGoalsWords.Line(goal, copy);
            row.Children.Add(Body(tool));
            TextBlock statusText = Body(status);
            statusText.Foreground = ThemeBrushes.Get(goal.Met ? "TcGreenTextBrush" : "TcInkSecondaryBrush");
            Grid.SetColumn(statusText, 1);
            row.Children.Add(statusText);
            TextBlock figures = Mono(line);
            Grid.SetColumn(figures, 3);
            row.Children.Add(figures);
            if (goal.Folder is { } folder)
            {
                TextBlock where = Caption(folder);
                where.TextWrapping = TextWrapping.NoWrap;
                where.TextTrimming = TextTrimming.CharacterEllipsis;
                where.VerticalAlignment = VerticalAlignment.Center;
                Grid.SetColumn(where, 4);
                row.Children.Add(where);
            }

            AutomationProperties.SetName(row, string.Join(", ", new[] { tool, status, line, goal.Folder }.Where(p => p is not null)));
            panel.Children.Add(row);
        }

        return Card(panel);
    }

    // ---- Pieces -------------------------------------------------------------

    private static string Text(Func<string, string?> copy, string key) => copy(key) ?? string.Empty;

    private static Border Card(UIElement child) => new()
    {
        Background = ThemeBrushes.Get("TcSurfaceCardBrush"),
        BorderBrush = ThemeBrushes.Get("TcHairlineBrush"),
        BorderThickness = new Thickness(1),
        CornerRadius = new CornerRadius(8),
        Padding = new Thickness(14, 12, 14, 12),
        Child = child,
    };

    private static TextBlock Heading(string text, double size)
    {
        var block = new TextBlock { Text = text, FontSize = size, TextWrapping = TextWrapping.Wrap };
        AutomationProperties.SetHeadingLevel(block, AutomationHeadingLevel.Level3);
        return block;
    }

    private static TextBlock Eyebrow(string text) =>
        new() { Text = text, FontSize = 12, Foreground = ThemeBrushes.Get("TcInkSecondaryBrush"), TextWrapping = TextWrapping.Wrap };

    private static TextBlock Figure(string text) =>
        new() { Text = text, FontSize = 20, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap };

    private static TextBlock Body(string text) => new() { Text = text, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true };

    private static TextBlock Caption(string text) =>
        new() { Text = text, FontSize = 12, Foreground = ThemeBrushes.Get("TcInkSecondaryBrush"), TextWrapping = TextWrapping.Wrap };

    private static TextBlock Mono(string text) =>
        new() { Text = text, FontFamily = (FontFamily)Application.Current.Resources["TcMonoFontFamily"], VerticalAlignment = VerticalAlignment.Center };
}
