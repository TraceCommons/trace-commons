using System;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls.Settings;

/// <summary>
/// Settings > Payouts, the payout link (W16-D2 (a); <c>PayoutLinkView</c>,
/// PayoutLinkView.swift): which organization the commons sends NEAR AI
/// credit to. Hidden unless the commons lists the NEAR AI rail, or the
/// account session ended, which it says. Nothing is preselected; Choose,
/// after the consent for the selected organization is on screen, is the
/// confirmation. Every sentence is the core's.
/// </summary>
public sealed class PayoutLinkSection : UserControl
{
    private readonly PayoutLinkFlow _flow;
    private readonly StackPanel _root = new() { Spacing = 12 };
    private bool _visible;

    public PayoutLinkSection(DaemonHost host)
    {
        // A container, not a stop: Tab goes to the controls inside it.
        IsTabStop = false;
        PayoutLinkCopy? copy = PayoutLinkCopy.Live;
        _flow = new PayoutLinkFlow(
            new PayoutLinkClient((method, json) => host.CallAsync(method, json)),
            copy is null ? null : new PayoutLinkSurface(copy));
        _flow.Changed += Render;
        Content = _root;
        Loaded += async (_, _) => await _flow.RefreshAsync();
    }

    /// <summary>Raised when the section starts or stops drawing anything: the list shows its row only then (D2).</summary>
    public event Action<bool>? VisibilityChanged;

    public bool Shown => _visible;

    public System.Threading.Tasks.Task RefreshAsync() => _flow.RefreshAsync();

    private void Render()
    {
        _root.Children.Clear();
        if (_flow.Surface is { } surface)
        {
            if (_flow.Rail is { } rail)
            {
                _root.Children.Add(SettingsCards.Card(surface.Copy.Title, RailContent(surface, rail)));
            }
            else if (_flow.StatusRefusal is { } refusal)
            {
                _root.Children.Add(SettingsCards.Card(
                    surface.Copy.Title,
                    refusal == PayoutLinkRefusal.SessionEnded
                        ? SettingsCards.Button(surface.RefusalLine(refusal), async (_, _) => await _flow.RefreshAsync())
                        : SettingsCards.Text(surface.RefusalLine(refusal))));
            }
        }

        if (_flow.Visible != _visible)
        {
            _visible = _flow.Visible;
            VisibilityChanged?.Invoke(_visible);
        }
    }

    private UIElement RailContent(PayoutLinkSurface surface, PayoutRailStatus rail)
    {
        var body = new StackPanel { Spacing = 8 };
        if (surface.StatusLine(rail) is { } line)
        {
            body.Children.Add(SettingsCards.Text(line));
        }

        if (PayoutLinkSurface.CanUnlink(rail))
        {
            body.Children.Add(SettingsCards.Caption(surface.Copy.UnlinkBody));
            Button unlink = SettingsCards.Button(
                surface.Copy.UnlinkButton, async (_, _) => await _flow.StopAsync(),
                surface.UnlinkAccessibilityName(rail) ?? surface.Copy.UnlinkButton);
            unlink.IsEnabled = !_flow.Busy;
            body.Children.Add(unlink);
        }

        if (PayoutLinkSurface.ShowsChangeButton(rail, _flow.Picker))
        {
            Button change = SettingsCards.Button(
                surface.Copy.ChangeButton, async (_, _) => await _flow.ChangeAsync(), surface.Copy.ChangeAccessibilityName);
            change.IsEnabled = !_flow.Busy;
            body.Children.Add(change);
        }

        if (PayoutLinkSurface.ShowsPicker(rail, _flow.Picker))
        {
            Picker(surface, body);
        }

        if (_flow.Refusal is { } refusal)
        {
            body.Children.Add(SettingsCards.Caption(surface.RefusalLine(refusal)));
        }

        return body;
    }

    private void Picker(PayoutLinkSurface surface, StackPanel body)
    {
        switch (_flow.Picker.Current)
        {
            case PayoutPicker.Phase.NoOrganization:
                body.Children.Add(SettingsCards.Caption(surface.Copy.NoOrganization));
                break;

            case PayoutPicker.Phase.Choosing or PayoutPicker.Phase.Working:
            {
                // RadioButtons with nothing selected until the person picks
                // one (checklist L5: no preselection).
                var group = new StackPanel { Spacing = 2 };
                foreach (PayoutOrganization org in _flow.Picker.Rows)
                {
                    var option = new RadioButton
                    {
                        Content = org.Name,
                        GroupName = "payout-org",
                        IsChecked = _flow.Picker.SelectedId == org.Id,
                        IsEnabled = !_flow.Busy,
                    };
                    AutomationProperties.SetName(option, org.Name);
                    option.Checked += (_, _) => _flow.Select(org.Id);
                    group.Children.Add(option);
                }

                body.Children.Add(group);
                if (_flow.Picker.ConsentFor(surface.Copy) is { } consent)
                {
                    body.Children.Add(SettingsCards.Caption(consent.Scope));
                    Button choose = SettingsCards.Button(
                        consent.Button, async (_, _) => await _flow.ChooseAsync(), consent.AccessibilityName, primary: true);
                    choose.IsEnabled = !_flow.Busy;
                    Button notNow = SettingsCards.Button(consent.Secondary, (_, _) => _flow.NotNow());
                    notNow.IsEnabled = !_flow.Busy;
                    body.Children.Add(SettingsCards.Row(choose, notNow));
                }

                break;
            }
        }
    }
}
