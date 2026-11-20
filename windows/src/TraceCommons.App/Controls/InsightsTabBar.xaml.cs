using System;
using System.Collections.Generic;
using System.Linq;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using TraceCommons.App.ViewModels;

namespace TraceCommons.App.Controls;

/// <summary>
/// Insights' tabs: one segment per tab, in <see cref="InsightsViewModel.ShownTabs"/>
/// order, each an equal share of the track (Navigation.swift:115), exactly
/// one checked. The host shows the pages and reads the figures.
/// </summary>
public sealed partial class InsightsTabBar : UserControl
{
    private readonly Dictionary<int, ToggleButton> _segments = new();

    public InsightsTabBar()
    {
        InitializeComponent();
        _segments[InsightsViewModel.OverviewTab] = Segment0;
        _segments[InsightsViewModel.PatternsTab] = Segment1;
        _segments[InsightsViewModel.BenchmarksTab] = Segment2;
        _segments[InsightsViewModel.SessionsTab] = Segment3;
        _segments[InsightsViewModel.AnalyzeTab] = Segment4;
        foreach (ToggleButton segment in _segments.Values)
        {
            segment.Click += OnSegment;
        }

        LayOut();
        Show(InsightsViewModel.OverviewTab);
    }

    /// <summary>Raised with the tab a segment was pressed for, the one shown included.</summary>
    public event Action<int>? Pressed;

    /// <summary>Adds the Skills segment at the Mac's place, named by the core's word.</summary>
    public void AddSkills(string title)
    {
        if (_segments.ContainsKey(InsightsViewModel.SkillsTab))
        {
            return;
        }

        var segment = new MonitorTabButton { Style = (Style)Track.Resources["InsightsSegmentStyle"], Content = title };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(segment, "insights-tab-skills");
        segment.Click += OnSegment;
        Track.Children.Add(segment);
        _segments[InsightsViewModel.SkillsTab] = segment;
        LayOut();
    }

    /// <summary>Removes the Skills segment.</summary>
    public void RemoveSkills()
    {
        if (_segments.Remove(InsightsViewModel.SkillsTab, out ToggleButton? segment))
        {
            Track.Children.Remove(segment);
            LayOut();
        }
    }

    /// <summary>
    /// Checks <paramref name="tab"/>'s segment and no other, in the Mac's
    /// label weight: semibold selected, medium otherwise (Navigation.swift:109).
    /// </summary>
    public void Show(int tab)
    {
        foreach ((int id, ToggleButton segment) in _segments)
        {
            bool shown = id == tab;
            segment.IsChecked = shown;
            segment.FontWeight = shown ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Medium;
        }
    }

    // Pressing the segment already shown keeps it checked: the host shows
    // the tab again, which sets the selection, never clears it
    // (Navigation.swift:101-104).
    private void OnSegment(object sender, RoutedEventArgs args)
    {
        int tab = _segments.FirstOrDefault(pair => ReferenceEquals(pair.Value, sender), new(-1, null!)).Key;
        if (tab >= 0)
        {
            Pressed?.Invoke(tab);
        }
    }

    private void LayOut()
    {
        Track.ColumnDefinitions.Clear();
        int column = 0;
        foreach (int tab in InsightsViewModel.ShownTabs(_segments.ContainsKey(InsightsViewModel.SkillsTab)))
        {
            if (!_segments.TryGetValue(tab, out ToggleButton? segment))
            {
                continue;
            }

            Track.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            Grid.SetColumn(segment, column++);
        }
    }
}
