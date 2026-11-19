using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using TraceCommons.App.Controls;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App;

/// <summary>
/// W23: the commons catalogue, Home's own page beside Mission drafts, as on
/// the Mac (HomeViews.swift:87-98). Home's Missions card opens the drafts
/// (<c>openMissionDrafts</c>); only the catalogue's own card opens this page
/// (<c>openMissions</c>), and that card is there only while missions are on
/// and the catalogue was read (HomeViews.swift:274-277), so a live launch,
/// whose daemon has no catalogue yet, never shows it.
/// </summary>
public sealed partial class MainWindow
{
    private void OnShowMissions(object sender, RoutedEventArgs e) => ShowMissionsPage();

    private void ShowMissionsPage()
    {
        if (!ViewModel.MissionsShown)
        {
            return;
        }

        if (MissionsPane.Content is MissionsView missions)
        {
            missions.Render();
        }
        else
        {
            MissionsPane.Content = new MissionsView(_host, () => ViewModel.ShowHome());
        }

        ViewModel.ShowMissions();
    }

    /// <summary>
    /// The catalogue's way in on Home: its word and subtitle are the core's
    /// (<c>home_history.mission_catalogue</c>, <c>catalogue_subtitle</c>),
    /// shown only while missions are on and the catalogue decodes.
    /// </summary>
    private void ApplyMissionCatalogueEntry()
    {
        string title = MonitorWords.CrumbTitle(new Crumb(CrumbWord.MissionCatalogue, MonitorTab.Home, false));
        (MissionRead read, string? json) = _host.MissionCatalogue();
        bool shown = ViewModel.MissionsShown
            && title.Length > 0
            && MissionPage.Settle(read, json, MonitorShellCopy.Top).Catalogue is not null;
        HomeOpenMissionCatalogue.Content = title;
        string subtitle = MonitorShellCopy.Section("home_history", "catalogue_subtitle");
        AutomationProperties.SetHelpText(HomeOpenMissionCatalogue, subtitle);
        HomeOpenMissionCatalogue.Visibility = shown ? Visibility.Visible : Visibility.Collapsed;
    }
}
