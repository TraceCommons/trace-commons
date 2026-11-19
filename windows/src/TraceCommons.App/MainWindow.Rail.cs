using System;
using System.Collections.Generic;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App;

/// <summary>
/// The tray flyout and the edge rail (W13): the window's half. The flyout is
/// made on the first tray click; the rail exists only while Settings' switch
/// is on (off by default, as on the Mac).
/// </summary>
public sealed partial class MainWindow
{
    private TrayFlyoutWindow? _flyout;
    private EdgeRailWindow? _rail;
    private bool _flyoutWired;

    /// <summary>Shows or removes the rail to match Settings' switch (EdgeRailController.apply).</summary>
    private void ApplyEdgeRail()
    {
        if (!_flyoutWired)
        {
            _flyoutWired = true;
            _tray.FlyoutRequested += () => DispatcherQueue.TryEnqueue(ShowFlyout);
        }

        if (_closed)
        {
            return;
        }

        if (_prefs.EdgeRail && _rail is null)
        {
            _rail = new EdgeRailWindow(
                () => _homeModel?.AccountTiles ?? Array.Empty<HomeTile>(),
                () => _homeModel?.TraceTiles ?? Array.Empty<HomeTile>(),
                tab =>
                {
                    BringForward();
                    if (tab is MonitorTab t)
                    {
                        OpenTab(t);
                    }
                });
        }
        else if (!_prefs.EdgeRail && _rail is not null)
        {
            _rail.Close();
            _rail = null;
        }
    }

    private void ShowFlyout()
    {
        if (_closed)
        {
            return;
        }

        _flyout ??= new TrayFlyoutWindow(BringForward, ShowSettingsWindow);
        _flyout.ShowAt(ToolWindows.Cursor(), _tray.HeaderLine, _tray.WeekLine, _tray.PrivateAILine, _tray.MarkTooltip);
    }

    /// <summary>Closes the tool windows with the main window, so none outlives it.</summary>
    private void CloseToolWindows()
    {
        _flyout?.Close();
        _flyout = null;
        _rail?.Close();
        _rail = null;
    }
}
