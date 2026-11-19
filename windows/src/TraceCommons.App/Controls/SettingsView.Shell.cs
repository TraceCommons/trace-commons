using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Controls;

/// <summary>
/// Settings' credit-arrival and edge rail switches (W12, W13). Every word is
/// the core's (<c>home_history.credit_icon_animation_title</c>, the Windows
/// words for the switch and its caption, <see cref="WindowsWords"/>, and
/// <c>edge_rail.setting_*</c> in the monitor table); a missing word leaves
/// its card out rather than showing one of this shell's own. The choices are
/// the shell's (<see cref="ShellPreferences"/>), never the daemon's.
/// </summary>
public sealed partial class SettingsView
{
    private bool _settingSwitches;

    private void InitializeShellSwitches()
    {
        ShellPreferences prefs = ShellPreferences.Shared;
        _settingSwitches = true;
        Card(
            CreditAnimationTitle, CreditAnimationSwitch, CreditAnimationCaption,
            MonitorShellCopy.Section("home_history", "credit_icon_animation_title"),
            WindowsWords.CreditIconAnimationToggle,
            WindowsWords.CreditIconAnimationCaption,
            prefs.CreditIconAnimation);

        // The Mac's card heads the rail's switch with a Settings eyebrow of
        // its own words and the rail's name as the title; this shell shows
        // the core's name alone (the eyebrow is not in the core's table).
        Card(
            EdgeRailTitle, EdgeRailSwitch, EdgeRailCaption,
            MonitorShellCopy.Section("edge_rail", "rail_label"),
            MonitorShellCopy.Section("edge_rail", "setting_toggle"),
            MonitorShellCopy.Section("edge_rail", "setting_caption"),
            prefs.EdgeRail);
        _settingSwitches = false;
    }

    private static void Card(
        Microsoft.UI.Xaml.Controls.TextBlock title,
        Microsoft.UI.Xaml.Controls.ToggleSwitch toggle,
        Microsoft.UI.Xaml.Controls.TextBlock caption,
        string heading,
        string label,
        string line,
        bool on)
    {
        bool shown = heading.Length > 0 && label.Length > 0;
        Visibility visibility = shown ? Visibility.Visible : Visibility.Collapsed;
        title.Text = heading;
        title.Visibility = visibility;
        toggle.Header = label;
        toggle.IsOn = on;
        toggle.Visibility = visibility;
        AutomationProperties.SetName(toggle, label);
        caption.Text = line;
        caption.Visibility = shown && line.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    private void OnCreditAnimationToggled(object sender, RoutedEventArgs e)
    {
        if (!_settingSwitches)
        {
            ShellPreferences.Shared.CreditIconAnimation = CreditAnimationSwitch.IsOn;
        }
    }

    private void OnEdgeRailToggled(object sender, RoutedEventArgs e)
    {
        if (!_settingSwitches)
        {
            ShellPreferences.Shared.EdgeRail = EdgeRailSwitch.IsOn;
        }
    }
}
