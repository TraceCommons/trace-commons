using System.Collections.Generic;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls.Settings;

/// <summary>
/// Settings > Tools' CLI options card (<c>ManagedCLIOptionsSection</c>,
/// ManagedCLIOptionsSection.swift): the core's title and scope line, then
/// for each tool the daemon keeps options for, its name, the field, and
/// Save, armed only while the field differs from the saved value; a refused
/// save says why under its row. Before the snapshot answers, the read
/// notice; a daemon without options says the core's unavailable line.
/// The card is rebuilt only when a read or save lands, so typing keeps focus.
/// </summary>
public sealed class CliOptionsCard : UserControl
{
    private readonly ManagedCliOptionsFlow _flow;
    private readonly Dictionary<string, Button> _saves = new();

    public CliOptionsCard(DaemonHost host)
    {
        IsTabStop = false;
        _flow = new ManagedCliOptionsFlow((method, json) => host.CallAsync(method, json));
        Loaded += async (_, _) =>
        {
            Render();
            await _flow.ReadAsync();
            Render();
        };
    }

    private void Render(string? focus = null)
    {
        _saves.Clear();
        var body = new StackPanel { Spacing = 12 };
        TextBlock scope = SettingsCards.SecondaryCaption(ManagedWords.Text("cli_options_scope"));
        body.Children.Add(scope);

        if (_flow.Snapshot is not { } snapshot)
        {
            body.Children.Add(SettingsCards.ReadNotice(_flow.ReadFailed, async (_, _) =>
            {
                string? at = SettingsCards.FocusedKey(this);
                await _flow.ReadAsync();
                Render(at);
            }, "cli/retry"));
        }
        else if (snapshot.CliOptions is null)
        {
            body.Children.Add(SettingsCards.Text(ManagedWords.Text("cli_options_unavailable")));
        }
        else
        {
            foreach (string tool in _flow.Rows())
            {
                body.Children.Add(Row(tool));
            }
        }

        SettingsCards.Replace(this, SettingsCards.Card(ManagedWords.Text("cli_options_title"), body), focus);
    }

    private StackPanel Row(string tool)
    {
        string toolName = ManagedWords.Text(tool);
        var row = new StackPanel { Spacing = 6 };
        row.Children.Add(SettingsCards.Text(toolName));

        var field = SettingsCards.Key(new TextBox
        {
            Text = _flow.Drafts.Value(tool, _flow.Snapshot?.CliOptions),
            IsEnabled = !_flow.Busy,
            FontFamily = (Microsoft.UI.Xaml.Media.FontFamily)Application.Current.Resources["TcMonoFontFamily"],
        }, "cli/" + tool + "/field");
        // The tool and the field's own name, as the Mac's accessibility label (:36).
        AutomationProperties.SetName(field, toolName + " " + ManagedWords.Text("cli_options_label"));
        string tooltip = ManagedWords.Text("cli_options_tooltip");
        if (tooltip.Length > 0)
        {
            ToolTipService.SetToolTip(field, tooltip);
        }

        Button save = SettingsCards.Button(
            ManagedWords.Text("cli_options_save"),
            async (_, _) =>
            {
                string? at = SettingsCards.FocusedKey(this);
                await _flow.SaveAsync(tool);
                Render(at);
            },
            ManagedWords.Text("cli_options_save_accessibility").Replace("{tool}", toolName, System.StringComparison.Ordinal));
        SettingsCards.Key(save, "cli/" + tool + "/save");
        save.IsEnabled = _flow.CanSave(tool);
        save.HorizontalAlignment = HorizontalAlignment.Right;
        _saves[tool] = save;
        field.TextChanged += (_, _) =>
        {
            _flow.Drafts.Edit(tool, field.Text);
            save.IsEnabled = _flow.CanSave(tool);
        };
        row.Children.Add(field);
        row.Children.Add(save);

        if (_flow.Refusal is { } refusal && refusal.Tool == tool)
        {
            TextBlock line = SettingsCards.SecondaryCaption(ManagedWords.Text(refusal.Key));
            row.Children.Add(line);
        }

        return row;
    }
}
