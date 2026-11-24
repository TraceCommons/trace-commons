using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Shapes;
using TraceCommons.App.Look;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

public sealed partial class ManagedSessionsView
{
    private UIElement SessionRow(ManagedSession session, ManagedCapabilities capabilities)
    {
        string title = session.Purpose == "login" ? T("sign_in") : session.ProjectLabel;
        string tool = T(session.Tool);
        string state = T(session.State is "starting" or "running" or "exited" or "failed"
            ? $"state_{session.State}" : "state_unknown");
        string caption = string.Join(" · ", new[]
        {
            tool,
            session.AccountLabel,
            T(session.Connection),
            session.Model is { } model ? _modelCatalogues.GetValueOrDefault(session.Tool)?.ModelDisplayName(model) ?? model : null,
            T($"collection_{session.Collection}"),
            session.ExitCode is int exitCode ? T("exit_code").Replace("{code}", exitCode.ToString()) : null,
            session.ResumedFrom.HasValue ? T("resumed") : null,
            session.ResumedFrom is DateTimeOffset resumed ? T("resumed_from").Replace("{time}", resumed.ToLocalTime().ToString("t")) : null,
            session.StartedAt?.ToLocalTime().ToString("g"),
        }.Where(part => !string.IsNullOrWhiteSpace(part)));

        var titleText = new TextBlock
        {
            Text = title, TextWrapping = TextWrapping.Wrap,
            Style = (Style)Application.Current.Resources["TcTitleCardTextStyle"],
        };
        var identity = new StackPanel { Spacing = 2 };
        identity.Children.Add(titleText);
        identity.Children.Add(SessionCaption(caption));
        var face = new Grid { ColumnDefinitions = { new() { Width = GridLength.Auto }, new() } };
        face.ColumnSpacing = 8;
        var mark = StepGeometry.Mark(session.Tool, 22, 10);
        mark.VerticalAlignment = VerticalAlignment.Top;
        face.Children.Add(mark);
        Grid.SetColumn(identity, 1);
        face.Children.Add(identity);

        var actions = ManagedSessionActions.For(session, capabilities,
            !string.IsNullOrEmpty(_host.ConfigDir) && File.Exists(ManagedHelperPath));
        var row = new Grid
        {
            ColumnSpacing = 6, Padding = new Thickness(0, 6, 0, 4),
            BorderBrush = ThemeBrushes.Get("TcHairlineDividerBrush"), BorderThickness = new Thickness(0, 1, 0, 0),
            ColumnDefinitions = { new(), new() { Width = GridLength.Auto } },
        };
        if (actions.Menu.Count > 0)
        {
            var button = new Button
            {
                Content = face, Padding = new Thickness(4, 4, 6, 4),
                HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Stretch,
                VerticalAlignment = VerticalAlignment.Stretch,
                Background = null, BorderThickness = new Thickness(0),
                IsEnabled = !_busy,
            };
            AutomationProperties.SetName(button, string.Join(" · ", new[] { title, tool, state }.Where(s => s.Length > 0)));
            AutomationProperties.SetAutomationId(button, $"ManagedSessionRow-{session.Id}");
            AutomationProperties.SetHelpText(button, caption);
            if (actions.Menu.Contains(ManagedSessionAction.Show))
            {
                button.Click += async (_, _) => await RunAsync(() => RunRowActionAsync(session, ManagedSessionAction.Show));
            }
            else
            {
                button.Flyout = SessionMenu(session, actions);
            }
            row.Children.Add(button);
        }
        else
        {
            face.Margin = new Thickness(4);
            row.Children.Add(face);
        }

        var trailing = new StackPanel
        {
            Spacing = 3, HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Center,
        };
        var status = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4, HorizontalAlignment = HorizontalAlignment.Right };
        status.Children.Add(new Ellipse
        {
            Width = 6, Height = 6, VerticalAlignment = VerticalAlignment.Center,
            Fill = ThemeBrushes.Get(session.State switch
            {
                "running" => "TcStatusOnBrush", "starting" => "TcStatusAskBrush",
                "failed" => "TcStatusOutsideBrush", _ => "TcStatusOffBrush",
            }),
        });
        status.Children.Add(SessionCaption(state));
        trailing.Children.Add(status);
        if (actions.Primary is { } primary)
        {
            Button primaryButton = Button(ActionKey(primary), () => RunRowActionAsync(session, primary));
            AutomationProperties.SetAutomationId(primaryButton, $"ManagedSessionPrimary-{session.Id}");
            trailing.Children.Add(primaryButton);
        }
        if (actions.Menu.Count > 0)
        {
            var more = new Button
            {
                Content = new FontIcon { Glyph = "\uE712", FontSize = 13 },
                Padding = new Thickness(6, 3, 6, 3), HorizontalAlignment = HorizontalAlignment.Right,
                IsEnabled = !_busy, Flyout = SessionMenu(session, actions),
            };
            AutomationProperties.SetName(more, $"{T("title")} · {title} · {tool}");
            AutomationProperties.SetAutomationId(more, $"ManagedSessionMenu-{session.Id}");
            trailing.Children.Add(more);
        }
        Grid.SetColumn(trailing, 1);
        row.Children.Add(trailing);
        return row;
    }

    private static TextBlock SessionCaption(string text) => new()
    {
        Text = text, TextWrapping = TextWrapping.Wrap,
        Style = (Style)Application.Current.Resources["TcCaptionTextStyle"],
        Foreground = ThemeBrushes.Get("TcInkSecondaryBrush"),
    };

    private MenuFlyout SessionMenu(ManagedSession session, ManagedSessionActions actions)
    {
        var menu = new MenuFlyout();
        foreach (ManagedSessionAction action in actions.Menu)
        {
            var item = new MenuFlyoutItem { Text = T(ActionKey(action)) };
            item.Click += async (_, _) => await RunAsync(() => RunRowActionAsync(session, action));
            menu.Items.Add(item);
        }
        return menu;
    }

    private static string ActionKey(ManagedSessionAction action) => action switch
    {
        ManagedSessionAction.Show => "show",
        ManagedSessionAction.Stop => "stop",
        ManagedSessionAction.Resume => "resume",
        ManagedSessionAction.StartAgain => "start_again",
        ManagedSessionAction.CopyResumeCommand => "copy_resume_command",
        _ => "dismiss",
    };

    private static string ManagedHelperPath => System.IO.Path.Combine(AppContext.BaseDirectory, "near-ai.exe");

    private async Task RunRowActionAsync(ManagedSession session, ManagedSessionAction action)
    {
        var sessionId = new { session_id = session.Id };
        switch (action)
        {
            case ManagedSessionAction.Show:
                await CallAsync("managed_session_focus", sessionId);
                break;
            case ManagedSessionAction.Stop:
                string description = T("stop_description").Replace("{tool}", T(session.Tool))
                    .Replace("{folder}", session.ProjectLabel);
                ContentDialog confirm = Dialog("stop_question", SessionCaption(description), "stop");
                confirm.DefaultButton = ContentDialogButton.Close;
                if (await DialogGuard.ShowOnceAsync(confirm) == ContentDialogResult.Primary)
                {
                    await CallAsync("managed_session_stop", sessionId);
                }
                break;
            case ManagedSessionAction.Resume:
            case ManagedSessionAction.StartAgain:
                string method = action == ManagedSessionAction.Resume ? "managed_session_resume" : "managed_session_start_again";
                await OpenAsync(await CallAsync(method, new { session_id = session.Id, request_id = Guid.NewGuid().ToString() }));
                break;
            case ManagedSessionAction.CopyResumeCommand:
                string command = $"& {QuotePowerShell(ManagedHelperPath)} --config-dir {QuotePowerShell(_host.ConfigDir)} sessions resume {QuotePowerShell(session.Id)}";
                if (!ClipboardCopy.TryCopy(command, asCopyOperation: true)) throw new InvalidOperationException();
                _error.Text = T("copied");
                break;
            case ManagedSessionAction.Dismiss:
                await CallAsync("managed_session_dismiss", sessionId);
                break;
        }
    }

    private static string QuotePowerShell(string text) => "'" + text.Replace("'", "''") + "'";
}
