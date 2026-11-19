using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.Controls;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App;

/// <summary>
/// The map pane's half of the window (slice W3): the Traces and Private AI
/// map in the pane W2 laid out, its toolbar toggle, the view a tab picks,
/// and the reads that keep it current.
/// </summary>
public sealed partial class MainWindow
{
    private TracesMapView? _map;

    /// <summary>Whether the person wants the map (the Mac's <c>monitor.showsMap</c>, MonitorWindowView.swift:182).</summary>
    private bool _wantsMap;

    /// <summary>The tab the map last followed, so only a change of tab moves its view.</summary>
    private MonitorTab? _mapFollowed;

    private void InitializeMap()
    {
        _map = new TracesMapView();
        MapPane.Child = _map;
        _map.Attach((method, parameters) => ViewModel.Host.CallAsync(method, parameters));
        _map.MapStyle = TracesMapStyleSetting.Load();
        TracesMapStyleSetting.Changed += style => _map.MapStyle = style;
        _map.FolderSelected += projectId =>
        {
            ViewModel.OpenFolder(projectId);
            OpenInspectorForFolder();
        };

        // The map reads again whenever the queue or the status moves, as the
        // Mac's stores do on the daemon's events.
        ViewModel.Host.QueueChanged += () => _ = _map.ReadAsync();
        ViewModel.Host.StatusChanged += () => _ = _map.ReadAsync();

        MapToggle.Visibility = Visibility.Visible;
        MapToggle.Click += (_, _) =>
        {
            _wantsMap = !_presented.Map;
            ApplyPaneAvailability();
        };
    }

    /// <summary>The toggle's state, name and tooltip (MonitorWindowView.swift:948-951).</summary>
    private void ApplyMapToggle(bool shown)
    {
        MapToggle.IsChecked = shown;
        string name = MonitorWords.Toggle(PaneToggle.Map, shown);
        AutomationProperties.SetName(MapToggle, name);
        ToolTipService.SetToolTip(MapToggle, name);
    }

    /// <summary>A tab picked moves the map's view (MonitorWindowView.swift:728-734).</summary>
    private void FollowTabOnMap(MonitorTab tab)
    {
        if (_map is null || _mapFollowed == tab)
        {
            return;
        }

        _mapFollowed = tab;
        _map.FollowTab(tab);
    }
}
