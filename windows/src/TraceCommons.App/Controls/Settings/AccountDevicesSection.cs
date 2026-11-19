using System;
using System.Globalization;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;
using Windows.ApplicationModel.DataTransfer;

namespace TraceCommons.App.Controls.Settings;

/// <summary>
/// Settings > Account > Devices (W16-D3 (a); <c>AccountDevicesView</c>,
/// AccountDevicesView.swift): the account's devices, this one first and
/// marked, each other one with Unlink and its confirmation; Link a server,
/// which mints a code and shows the command to run with it until the server
/// uses it. Drawn only once the list was read, or, for an ended session, as
/// the session line alone. Every sentence is the core's.
/// </summary>
public sealed class AccountDevicesSection : UserControl
{
    private readonly AccountDevicesFlow _flow;
    private readonly StackPanel _root = new() { Spacing = 12 };

    public AccountDevicesSection(DaemonHost host)
    {
        AccountDevicesCopy? copy = AccountDevicesCopy.Live;
        _flow = new AccountDevicesFlow(
            new AccountDevicesClient((method, json) => host.CallAsync(method, json)),
            copy is null ? null : new AccountDevicesSurface(copy));
        _flow.Changed += Render;
        Content = _root;
        Loaded += async (_, _) => await _flow.RefreshAsync();
        Render();
    }

    /// <summary>Reads the list again, after a sign-in or a change of account.</summary>
    public System.Threading.Tasks.Task RefreshAsync() => _flow.RefreshAsync();

    private void Render()
    {
        _root.Children.Clear();
        if (_flow.Surface is not { } surface)
        {
            return;
        }

        if (_flow.Devices is { } devices)
        {
            var body = new StackPanel { Spacing = 10 };
            body.Children.Add(SettingsCards.Caption(surface.Copy.Body));
            foreach (AccountDevice device in devices)
            {
                body.Children.Add(DeviceRow(surface, device));
            }

            body.Children.Add(Linking(surface));
            if (_flow.Refusal is { } refusal)
            {
                body.Children.Add(SettingsCards.Caption(surface.RefusalLine(refusal)));
            }

            _root.Children.Add(SettingsCards.Card(surface.Copy.Title, body));
        }
        else if (_flow.ListRefusal == AccountDevicesRefusal.SessionEnded)
        {
            // The session ended: the line is the action, as on the Mac. The
            // sign-in itself is the account section's; this re-reads after it.
            _root.Children.Add(SettingsCards.Card(
                surface.Copy.Title,
                SettingsCards.Button(surface.Copy.SessionEnded, async (_, _) => await _flow.RefreshAsync())));
        }
    }

    private UIElement DeviceRow(AccountDevicesSurface surface, AccountDevice device)
    {
        var row = new StackPanel { Spacing = 4 };
        var line = new Grid { ColumnSpacing = 8 };
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var names = new StackPanel { Spacing = 2 };
        var title = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        title.Children.Add(SettingsCards.Text(surface.Name(device)));
        if (device.ThisDevice)
        {
            title.Children.Add(SettingsCards.Text(surface.Copy.ThisDevice, "TcMonoChipTextStyle"));
        }

        names.Children.Add(title);
        if (surface.LastSeenLine(device, Relative) is { } seen)
        {
            names.Children.Add(SettingsCards.Caption(seen));
        }

        line.Children.Add(names);
        if (AccountDevicesSurface.CanUnlink(device) && _flow.Confirming != device)
        {
            Button unlink = SettingsCards.Button(
                surface.Copy.UnlinkButton, (_, _) => _flow.Confirm(device), surface.UnlinkAccessibilityName(device));
            unlink.IsEnabled = !_flow.Busy;
            unlink.VerticalAlignment = VerticalAlignment.Top;
            Grid.SetColumn(unlink, 1);
            line.Children.Add(unlink);
        }

        row.Children.Add(line);
        if (_flow.Confirming == device)
        {
            row.Children.Add(SettingsCards.Caption(surface.UnlinkConfirm(device)));
            Button confirm = SettingsCards.Button(
                surface.Copy.UnlinkConfirmButton, async (_, _) => await _flow.UnlinkAsync(device),
                surface.UnlinkConfirmAccessibilityName(device));
            confirm.IsEnabled = !_flow.Busy;
            row.Children.Add(SettingsCards.Row(confirm, SettingsCards.Button(surface.Copy.CancelButton, (_, _) => _flow.Confirm(null))));
        }

        return row;
    }

    private UIElement Linking(AccountDevicesSurface surface)
    {
        var panel = new StackPanel { Spacing = 6 };
        switch (_flow.Link)
        {
            case DeviceLinkPhase.Waiting waiting:
            {
                string command = surface.Command(waiting.Code);
                panel.Children.Add(SettingsCards.Caption(surface.Copy.LinkBody));
                var commandText = SettingsCards.Text(command, "TcMonoCodeTextStyle");
                commandText.IsTextSelectionEnabled = true;
                commandText.IsHyphenationEnabled = false;
                panel.Children.Add(commandText);
                string copyLabel = _flow.CopiedCode == waiting.Code.CodeId ? surface.Copy.Copied : surface.Copy.CopyButton;
                panel.Children.Add(SettingsCards.Row(
                    SettingsCards.Button(copyLabel, (_, _) =>
                    {
                        var package = new DataPackage();
                        package.SetText(command);
                        Clipboard.SetContent(package);
                        _flow.Copied(waiting.Code);
                    }),
                    SettingsCards.Button(surface.Copy.DoneButton, (_, _) => _flow.DismissLink())));
                var progress = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
                var ring = new ProgressRing { IsActive = true, Width = 14, Height = 14 };
                AutomationProperties.SetAccessibilityView(ring, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
                progress.Children.Add(ring);
                progress.Children.Add(SettingsCards.Caption(surface.Copy.Waiting));
                panel.Children.Add(progress);
                break;
            }

            case DeviceLinkPhase.Linked:
                panel.Children.Add(SettingsCards.Row(
                    SettingsCards.Text(surface.Copy.Linked),
                    SettingsCards.Button(surface.Copy.DoneButton, (_, _) => _flow.DismissLink())));
                break;

            case DeviceLinkPhase.Expired:
                panel.Children.Add(SettingsCards.Caption(surface.Copy.Expired));
                panel.Children.Add(LinkButton(surface));
                break;

            case DeviceLinkPhase.Refused refused:
                panel.Children.Add(SettingsCards.Caption(surface.RefusalLine(refused.Refusal)));
                panel.Children.Add(LinkButton(surface));
                break;

            default:
                panel.Children.Add(LinkButton(surface));
                break;
        }

        return panel;
    }

    private Button LinkButton(AccountDevicesSurface surface)
    {
        Button link = SettingsCards.Button(
            surface.Copy.LinkButton, async (_, _) => await _flow.LinkServerAsync(), surface.Copy.LinkAccessibilityName);
        link.IsEnabled = !_flow.Busy && _flow.Link is not DeviceLinkPhase.Minting;
        return link;
    }

    /// <summary>
    /// "5 minutes ago" for the server's RFC 3339 text, in the system's words,
    /// as the Mac's RelativeDateTimeFormatter gives; null when unreadable.
    /// </summary>
    private static string? Relative(string text)
    {
        if (!DateTimeOffset.TryParse(text, CultureInfo.InvariantCulture, DateTimeStyles.RoundtripKind, out DateTimeOffset at))
        {
            return null;
        }

        return at.LocalDateTime.ToString("g", CultureInfo.CurrentCulture);
    }
}
