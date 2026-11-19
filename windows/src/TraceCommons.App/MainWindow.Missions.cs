using TraceCommons.App.Controls;

namespace TraceCommons.App;

/// <summary>
/// W23: the Missions page on Home (owner decision W23-D1 (a): the Mac's
/// Missions page replaces Mission drafts on this shell). Hosted in the Home
/// sub-page the drafts used, so the breadcrumb, Home's way in and the
/// missions switch that hides it are the shell's, unchanged
/// (<c>HomePage.shown(missions:)</c>, MonitorNavigation.swift:18-20).
/// </summary>
public sealed partial class MainWindow
{
    private void ShowMissionsPage()
    {
        if (MissionDraftsPane.Content is MissionsView missions)
        {
            missions.Render();
        }
        else
        {
            MissionDraftsPane.Content = new MissionsView(_host, () => ViewModel.ShowHome());
        }

        ViewModel.ShowMissionDrafts();
    }
}
