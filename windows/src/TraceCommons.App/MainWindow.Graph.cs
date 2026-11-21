using System;
using System.Diagnostics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TraceCommons.App.Controls;
using TraceCommons.Interop.Map;
using TraceCommons.App.Look;
using TraceCommons.Interop.Shell;
using Windows.UI.ViewManagement;

namespace TraceCommons.App;

/// <summary>
/// The Traces graph footer's half of the window (W3, moved from W2
/// 2026-11-19): its toolbar toggle, its place under the tab, opening and
/// closing over 0.25 s, its reads, and the binoculars that focus the map
/// (MonitorWindowView.swift:400-418, :735-741, :935-947, :1028-1035).
/// </summary>
public sealed partial class MainWindow
{
    private TracesGraphFooter? _graph;

    /// <summary>Whether the person wants the graph (<c>monitor.showsGraph</c>, on by default, MonitorWindowView.swift:189).</summary>
    private bool _wantsGraph = true;

    /// <summary>The selection's tool, which the graph counts and the binoculars focus on.</summary>
    private string? _graphTool;

    /// <summary>Whether the map is focused on <see cref="_graphTool"/> (<c>mapFocus</c>, :194).</summary>
    private bool _graphFocus;

    private bool _graphShown;
    private double _graphFrom;
    private double _graphTo;
    private readonly Stopwatch _graphClock = new();
    private bool _graphTicking;
    private readonly UISettings _graphSettings = new();

    private void InitializeGraph(TracesMapView map)
    {
        _graph = new TracesGraphFooter { VerticalAlignment = VerticalAlignment.Bottom };
        GraphFooterHost.Children.Add(_graph);
        map.SnapshotRead += snapshot =>
            _graph.SetData(snapshot.History, snapshot.Tree(MapFeed.CoreScansWhenUnset).AllSessions);
        _graph.FocusRequested += FocusMapOnGraphTool;

        GraphToggle.Visibility = Visibility.Visible;
        GraphToggle.Click += (_, _) =>
        {
            // The toggle flips what the person wants; off Traces it is disabled.
            _wantsGraph = !_wantsGraph;
            ApplyGraph();
        };
        ApplyGraph();
    }

    /// <summary>
    /// The selection's tool, from the Traces tree (MonitorWindowView.swift:711-724):
    /// the graph counts only it and the map rings it.
    /// </summary>
    internal void SetGraphTool(string? tool)
    {
        _graphTool = tool;
        _graph?.SetTool(tool, _graphFocus);
        if (_map is not null)
        {
            _map.FocusTool = _graphFocus ? tool : null;
        }
    }

    /// <summary>
    /// The binoculars: focus the map on the selected tool, or back to the
    /// whole map; a hidden map is shown, on its Traces view (:735-741).
    /// </summary>
    private void FocusMapOnGraphTool()
    {
        _graphFocus = !_graphFocus;
        if (_map is not null)
        {
            _map.FollowTab(MonitorTab.Traces);
            _map.FocusTool = _graphFocus ? _graphTool : null;
        }

        _graph?.SetTool(_graphTool, _graphFocus);
        if (!_wantsMap)
        {
            _wantsMap = true;
            ApplyPaneAvailability();
        }
    }

    /// <summary>
    /// The toggle's state and name, and the footer opening or closing: the
    /// graph is the Traces tab's; elsewhere its toggle is disabled and reads
    /// as hidden (:935-944, :679).
    /// </summary>
    private void ApplyGraph()
    {
        bool shown = TracesGraphState.Shown(_wantsGraph, _at.Tab);
        GraphToggle.IsEnabled = TracesGraphState.Available(_at.Tab);
        GraphToggle.IsChecked = shown;
        string name = MonitorWords.Toggle(PaneToggle.Graph, shown);
        AutomationProperties.SetName(GraphToggle, name);
        ToolTipService.SetToolTip(GraphToggle, name);
        if (shown == _graphShown)
        {
            return;
        }

        _graphShown = shown;
        _graphFrom = GraphFooterHost.ActualHeight;
        _graphTo = shown ? TracesGraphState.Height : 0;
        if (shown)
        {
            GraphFooterHost.Visibility = Visibility.Visible;
        }

        if (!_graphSettings.AnimationsEnabled || GraphFooterHost.XamlRoot is null)
        {
            SetGraphFrame(1);
            return;
        }

        _graphClock.Restart();
        if (!_graphTicking)
        {
            _graphTicking = true;
            CompositionTarget.Rendering += OnGraphFrame;
        }
    }

    private readonly FocusFramePace<XamlRoot> _graphFramePace = new(WindowClocks.Windows);

    private void OnGraphFrame(object? sender, object e)
    {
        // The window's frame rule (FocusFrameRule): at most 15 a second while unfocused.
        if (_graphFramePace.Skips(OpenRoot, GlassClock.Now))
        {
            return;
        }

        BezierCurve curve = TracesGraphState.Footer;
        double t = _graphClock.Elapsed.TotalSeconds;
        SetGraphFrame(curve.At(t));
        if (t >= curve.Duration)
        {
            _graphTicking = false;
            CompositionTarget.Rendering -= OnGraphFrame;
            _graphClock.Reset();
        }
    }

    /// <summary>
    /// The footer at <paramref name="p"/> of its move: height 0 to 206,
    /// rising from the pane's bottom edge with a fade (:417).
    /// </summary>
    private void SetGraphFrame(double p)
    {
        double height = _graphFrom + (_graphTo - _graphFrom) * p;
        GraphFooterHost.Height = Math.Max(0, height);
        GraphFooterHost.Opacity = TracesGraphState.Height > 0 ? Math.Clamp(height / TracesGraphState.Height, 0, 1) : 1;
        if (p >= 1 && !_graphShown)
        {
            GraphFooterHost.Visibility = Visibility.Collapsed;
        }
    }
}
