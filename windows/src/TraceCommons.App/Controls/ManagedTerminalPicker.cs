using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

/// <summary>
/// "Open sessions in": the terminal managed sessions open in, in Settings
/// under Private AI, as the macOS app has it (#1406). The label is the
/// core's (<c>terminal_choice</c>); the options are the terminal names the
/// daemon sends in the managed snapshot. Collapsed when the daemon offers
/// none. The selection moves only by a write the daemon accepted: after
/// <c>set_managed_terminal</c> the snapshot is read again.
/// </summary>
public sealed class ManagedTerminalPicker : UserControl
{
    private readonly DaemonHost _host;
    private readonly ComboBox _choices = new() { MinWidth = 220 };
    private readonly TextBlock _error = new() { TextWrapping = TextWrapping.Wrap };
    private ManagedSnapshot? _snapshot;
    private bool _filling;

    public ManagedTerminalPicker(DaemonHost host)
    {
        _host = host;
        var panel = new StackPanel { Spacing = 6 };
        panel.Children.Add(_choices);
        panel.Children.Add(_error);
        Content = panel;
        Visibility = Visibility.Collapsed;
        _choices.SelectionChanged += async (_, _) => await ChooseAsync();
        Loaded += async (_, _) => await RefreshAsync();
    }

    private string T(string key) => _snapshot?.Text(key) ?? string.Empty;

    private async Task RefreshAsync()
    {
        try
        {
            var response = await _host.CallAsync("managed_snapshot", "{}");
            if (response.IsError || response.Result is null) return;
            _snapshot = ManagedSnapshot.Parse(response.Result.Value.GetRawText());
            Render();
        }
        catch { }
    }

    private void Render()
    {
        var choices = _snapshot?.TerminalChoices ?? [];
        if (_snapshot is null || choices.Length == 0)
        {
            Visibility = Visibility.Collapsed;
            return;
        }
        _filling = true;
        try
        {
            _choices.Header = T("terminal_choice");
            AutomationProperties.SetName(_choices, T("terminal_choice"));
            _choices.Items.Clear();
            foreach (var choice in choices) _choices.Items.Add(choice.Name);
            _choices.SelectedIndex = _snapshot.TerminalSelectedIndex;
        }
        finally { _filling = false; }
        Visibility = Visibility.Visible;
    }

    private async Task ChooseAsync()
    {
        if (_filling || _snapshot is null) return;
        var choices = _snapshot.TerminalChoices;
        var index = _choices.SelectedIndex;
        if (index < 0 || index >= choices.Length || index == _snapshot.TerminalSelectedIndex) return;
        _choices.IsEnabled = false;
        _error.Text = string.Empty;
        try
        {
            var response = await _host.CallAsync("set_managed_terminal",
                JsonSerializer.Serialize(new { terminal = choices[index].Id }));
            if (response.IsError) _error.Text = T("action_failed");
        }
        catch { _error.Text = T("action_failed"); }
        finally
        {
            _choices.IsEnabled = true;
            await RefreshAsync();
        }
    }
}
