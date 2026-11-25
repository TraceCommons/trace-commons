using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.Look;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

public sealed partial class ManagedSessionsView
{
    private void RenderSavedAccounts()
    {
        _root.Children.Clear();
        var body = new StackPanel { Spacing = 8 };
        var heading = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        heading.Children.Add(SectionTitle("accounts_title"));
        heading.Children.Add(RefreshGlyph(_snapshot is null ? "refresh" : "accounts_refresh_accessibility"));
        if (_snapshot?.Capabilities.TerminalDestination is { } destination)
        {
            string scope = T("terminal_scope").Replace("{destination}", destination);
            var info = new Button
            {
                Content = new FontIcon { Glyph = "\uE946", FontSize = 14 },
                Background = null,
                BorderThickness = new Thickness(0),
                Flyout = new Flyout { Content = SessionCaption(scope) },
            };
            ToolTipService.SetToolTip(info, scope);
            AutomationProperties.SetName(info, scope);
            heading.Children.Add(info);
        }
        body.Children.Add(heading);
        body.Children.Add(SessionCaption(T(_snapshot is null ? "connecting" : "description")));
        if (_snapshot is { } snapshot)
        {
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            if (snapshot.SetupTools.Length > 0) actions.Children.Add(Button("add", AddAsync));
            if (snapshot.OffersSavedAccountLaunch)
            {
                Button launch = Button("launch", LaunchAsync);
                launch.Style = (Style)Application.Current.Resources["TcPrimaryButtonStyle"];
                actions.Children.Add(launch);
            }
            if (actions.Children.Count > 0) body.Children.Add(actions);

            foreach (string line in _failure.CardLines(T, snapshot.Capabilities.TerminalDestination))
            {
                var failure = SessionCaption(line);
                failure.IsTextSelectionEnabled = true;
                body.Children.Add(failure);
            }
            if (_unsupportedVersion)
            {
                foreach (ManagedTool tool in snapshot.VersionRecoveryTools(_unsupportedTool))
                {
                    body.Children.Add(Button("update", async () =>
                        await Windows.System.Launcher.LaunchUriAsync(new Uri(tool.InstallUrl))));
                }
            }
            if (snapshot.Capabilities.TerminalDestination is null)
                body.Children.Add(SessionCaption(T("terminal_unavailable")));
            foreach (ManagedAccount account in snapshot.Accounts)
                body.Children.Add(SavedAccountRow(account, snapshot));
        }

        _root.Children.Add(new Border
        {
            Child = body,
            Padding = new Thickness(12),
            BorderThickness = new Thickness(1),
            CornerRadius = (CornerRadius)Application.Current.Resources["TcCardCornerRadius"],
            Background = ThemeBrushes.Get("TcSurfaceCardBrush"),
            BorderBrush = ThemeBrushes.Get("TcHairlineBrush"),
        });
    }

    private UIElement SavedAccountRow(ManagedAccount account, ManagedSnapshot snapshot)
    {
        var row = new Grid
        {
            ColumnDefinitions = { new() { Width = GridLength.Auto }, new(), new() { Width = GridLength.Auto } },
            ColumnSpacing = 8,
            Padding = new Thickness(0, 6, 0, 0),
            BorderBrush = ThemeBrushes.Get("TcHairlineDividerBrush"),
            BorderThickness = new Thickness(0, 1, 0, 0),
        };
        row.Children.Add(StepGeometry.Mark(account.Tool, 22, 10));
        var body = new StackPanel { Spacing = 3 };
        var name = new Grid { ColumnDefinitions = { new(), new() { Width = GridLength.Auto } }, ColumnSpacing = 8 };
        name.Children.Add(new TextBlock { Text = account.Label, TextWrapping = TextWrapping.Wrap });
        if (snapshot.IsDefault(account))
        {
            var tag = SessionCaption(T("default"));
            Grid.SetColumn(tag, 1);
            name.Children.Add(tag);
        }
        body.Children.Add(name);
        if (!snapshot.CanLaunch(account.Tool))
        {
            body.Children.Add(SessionCaption(snapshot.ToolUnavailable(account.Tool)));
            if (snapshot.Tools?.FirstOrDefault(tool => tool.Tool == account.Tool) is { } missing)
                body.Children.Add(InstallButton(missing));
        }
        string meta = string.Join(" · ", new[] { T(account.Tool), T(account.Connection), snapshot.AuthenticationLabel(account) }
            .Where(word => !string.IsNullOrWhiteSpace(word)));
        if (meta.Length > 0) body.Children.Add(SessionCaption(meta));
        Grid.SetColumn(body, 1);
        row.Children.Add(body);

        var more = new Button
        {
            Content = new FontIcon { Glyph = "\uE712", FontSize = 13 },
            VerticalAlignment = VerticalAlignment.Center,
            IsEnabled = !_busy,
            Flyout = SavedAccountMenu(account, snapshot),
        };
        AutomationProperties.SetName(more, $"{T("manage")} · {account.Label}");
        AutomationProperties.SetAutomationId(more, $"SavedAccountMenu-{account.Id}");
        Grid.SetColumn(more, 2);
        row.Children.Add(more);
        return row;
    }

    private MenuFlyout SavedAccountMenu(ManagedAccount account, ManagedSnapshot snapshot)
    {
        bool held = snapshot.Sessions.Any(session => session.AccountId == account.Id && session.HoldsAccount);
        var menu = new MenuFlyout();
        void Item(string key, Func<Task> action, bool enabled = true)
        {
            var item = new MenuFlyoutItem { Text = T(key), IsEnabled = enabled };
            item.Click += async (_, _) =>
            {
                if (_busy) return;
                _returnFocus = ($"SavedAccountMenu-{account.Id}", 0);
                await RunAsync(action);
            };
            menu.Items.Add(item);
        }

        Item("use_default", async () =>
        {
            ulong generation = snapshot.Generations.GetValueOrDefault(account.Tool);
            await CallAsync("managed_select", new
            {
                selection = new { tool = account.Tool, connection = account.Connection, account_id = account.Id, generation },
                expected_generation = generation,
            });
        });
        Item("rename_short", () => RenameAsync(account));
        if (snapshot.OffersReconnect(account))
        {
            Item("reconnect", async () => await OpenAsync(
                await CallAsync("managed_account_reconnect", new { account_id = account.Id })),
                !held && snapshot.Capabilities.TerminalLaunch);
        }
        Item("verify", async () => await CallAsync("managed_account_verify", new { account_id = account.Id }));
        menu.Items.Add(new MenuFlyoutSeparator());
        Item("remove", async () =>
        {
            var confirm = Dialog("remove_question", SessionCaption(T("remove_description")), "remove");
            confirm.DefaultButton = ContentDialogButton.Close;
            if (await DialogGuard.ShowOnceAsync(confirm) == ContentDialogResult.Primary)
                await CallAsync("managed_account_remove", new { account_id = account.Id });
        }, !held);
        return menu;
    }
}
