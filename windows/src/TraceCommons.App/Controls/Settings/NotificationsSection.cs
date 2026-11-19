using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls.Settings;

/// <summary>
/// Settings > Notifications (W17-D2, owner ruling 2026-11-19;
/// <c>NotificationsSection</c> and <c>NudgeSettingsSection</c>,
/// StartupSection.swift and NudgeSettingsSection.swift): under the core's
/// notifications heading, the master switch, then under it one switch per
/// kind the Mac shows (the digest,
/// verdicts, idle traces, NEAR AI credit), each from a value the daemon
/// reported and in the core's words, then the core's line on the caps.
/// The held kinds are never drawn. The Mac's system-permission rows have no
/// Windows counterpart: the tray balloon needs no permission.
/// </summary>
public sealed class NotificationsSection : UserControl
{
    private static readonly OnboardingCopy? Copy = OnboardingCopy.Load();

    private readonly NotificationKindsStore _store;

    public NotificationsSection(DaemonHost host)
    {
        IsTabStop = false;
        _store = new NotificationKindsStore((method, json) => host.CallAsync(method, json));
        Loaded += async (_, _) =>
        {
            Render();
            await _store.ReadAsync();
            Render();
        };
    }

    private void Render()
    {
        string heading = Copy?.NotificationHeading ?? string.Empty;
        if (heading.Length == 0)
        {
            // No heading word, no card: nothing is drawn under a blank title.
            Content = null;
            return;
        }

        var body = new StackPanel { Spacing = 12 };
        if (!_store.SettingsAnswered)
        {
            body.Children.Add(SettingsCards.ReadNotice(_store.ReadFailed, async (_, _) =>
            {
                await _store.ReadAsync();
                Render();
            }));
        }

        if (_store.Master is { } master)
        {
            var masterToggle = new ToggleSwitch
            {
                Header = master.Label,
                IsOn = master.IsOn,
                IsEnabled = !_store.Busy,
                OnContent = string.Empty,
                OffContent = string.Empty,
            };
            AutomationProperties.SetName(masterToggle, master.Label);
            if (master.Tooltip is { } tip)
            {
                // A hover tip, never drawn as a line (as on the Mac).
                ToolTipService.SetToolTip(masterToggle, tip);
            }

            masterToggle.Toggled += async (_, _) =>
            {
                if (masterToggle.IsOn == master.IsOn)
                {
                    return;
                }

                masterToggle.IsEnabled = false;
                await _store.SetMasterAsync(masterToggle.IsOn);
                Render();
            };
            body.Children.Add(masterToggle);
        }

        foreach (NotifySwitchRow row in _store.Rows)
        {
            var toggle = new ToggleSwitch
            {
                Header = row.Label,
                IsOn = row.IsOn,
                IsEnabled = row.Enabled && !_store.Busy,
                // The Mac's switch carries no On or Off words (GlassToggleStyle).
                OnContent = string.Empty,
                OffContent = string.Empty,
            };
            AutomationProperties.SetName(toggle, row.Label);
            string kind = row.Kind;
            toggle.Toggled += async (_, _) =>
            {
                if (toggle.IsOn == row.IsOn)
                {
                    return;
                }

                toggle.IsEnabled = false;
                await _store.SetAsync(kind, toggle.IsOn);
                Render();
            };

            // Each kind sits under the master it follows, disabled (its
            // stored value still shown) while the master is off.
            var item = new StackPanel { Spacing = 4, Margin = new Thickness(_store.Master is null ? 0 : 24, 0, 0, 0) };
            item.Children.Add(toggle);
            if (row.Help is { } help)
            {
                item.Children.Add(SettingsCards.Caption(help));
            }

            body.Children.Add(item);
        }

        if (_store.Footnote is { } footnote)
        {
            body.Children.Add(SettingsCards.Caption(footnote));
        }

        // A refused switch write, under the switches it is about.
        if (_store.Refusal is { } refusal && _store.RefusedOfferKind is null)
        {
            body.Children.Add(SettingsCards.Caption(refusal));
        }

        Content = SettingsCards.Card(heading, body);
    }
}
