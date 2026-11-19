using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls.Settings;

/// <summary>
/// Settings > Insights (Settings IA D13; <c>InsightsSection</c> and
/// <c>InsightsSettingsCard</c>, InsightsSettingsCard.swift): until the
/// daemon's settings answer, the read notice; then, only when the daemon
/// reports the weekly-summary switch, the core's Insights title over that
/// switch, with the request-failed line under it after a refused write.
/// The settings are read again each time the section loads, because Hide
/// on the summary card writes the same value. The subscription-usage
/// permission under it on the Mac is the Home track's (W11).
/// </summary>
public sealed class InsightsSection : UserControl
{
    private readonly InsightsRecapSwitch _switch;

    public InsightsSection(DaemonHost host)
    {
        IsTabStop = false;
        _switch = new InsightsRecapSwitch((method, json) => host.CallAsync(method, json));
        Loaded += async (_, _) =>
        {
            Render();
            await _switch.ReadAsync();
            Render();
        };
    }

    private void Render()
    {
        var root = new StackPanel { Spacing = 12 };
        if (!_switch.SettingsAnswered)
        {
            root.Children.Add(SettingsCards.ReadNotice(_switch.ReadFailed, async (_, _) =>
            {
                await _switch.ReadAsync();
                Render();
            }));
        }

        if (_switch.On is { } on)
        {
            string label = InsightsSettingsWords.Text("analytics_setting_recap_card");
            var toggle = new ToggleSwitch
            {
                Header = label,
                IsOn = on,
                IsEnabled = !_switch.Busy,
                // The Mac's switch carries no On or Off words (GlassToggleStyle).
                OnContent = string.Empty,
                OffContent = string.Empty,
            };
            AutomationProperties.SetName(toggle, label);
            toggle.Toggled += async (_, _) =>
            {
                if (toggle.IsOn == _switch.On)
                {
                    return;
                }

                toggle.IsEnabled = false;
                await _switch.SetAsync(toggle.IsOn);
                Render();
            };

            var body = new StackPanel { Spacing = 8 };
            body.Children.Add(toggle);
            if (_switch.Refusal is { } refusal)
            {
                body.Children.Add(SettingsCards.Caption(refusal));
            }

            root.Children.Add(SettingsCards.Card(InsightsSettingsWords.Text("title"), body));
        }

        Content = root;
    }
}
