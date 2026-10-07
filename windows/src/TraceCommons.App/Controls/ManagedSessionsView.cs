using System;
using System.Linq;
using System.Collections.Generic;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

/// <summary>Shared daemon accounts and sessions; all fixed copy comes from its snapshot.</summary>
public sealed class ManagedSessionsView : UserControl
{
    private readonly DaemonHost _host;
    private readonly StackPanel _root = new() { Spacing = 10 };
    private readonly StackPanel _rows = new() { Spacing = 8 };
    private readonly TextBlock _error = new() { TextWrapping = TextWrapping.Wrap };
    private readonly DispatcherTimer _timer = new() { Interval = TimeSpan.FromSeconds(10) };
    private ManagedSnapshot? _snapshot;
    private bool _busy;
    private bool _reading;

    public ManagedSessionsView(DaemonHost host)
    {
        _host = host;
        Content = _root;
        Loaded += async (_, _) => { await RefreshAsync(); _timer.Start(); };
        Unloaded += (_, _) => _timer.Stop();
        _timer.Tick += async (_, _) => await RefreshAsync();
    }

    private string T(string key) => _snapshot?.Text(key) ?? string.Empty;

    private async Task<JsonElement> CallAsync(string method, object parameters)
    {
        var response = await _host.CallAsync(method, JsonSerializer.Serialize(parameters));
        if (response.IsError || response.Result is null) throw new InvalidOperationException();
        return response.Result.Value.Clone();
    }

    private async Task RefreshAsync()
    {
        if (_reading) return;
        _reading = true;
        try
        {
            var value = await CallAsync("managed_snapshot", new { });
            var next = ManagedSnapshot.Parse(value.GetRawText());
            if (next is null || (_snapshot is not null && next.Revision < _snapshot.Revision)) return;
            _snapshot = next;
            Render();
        }
        catch { _error.Text = T("action_failed"); }
        finally { _reading = false; }
    }

    private Button Button(string key, Func<Task> action, bool enabled = true)
    {
        var button = new Button { Content = T(key), IsEnabled = enabled && !_busy };
        button.Click += async (_, _) => await RunAsync(action);
        return button;
    }

    private async Task RunAsync(Func<Task> action)
    {
        if (_busy) return;
        _busy = true;
        _error.Text = string.Empty;
        try { await action(); }
        catch { _error.Text = T("action_failed"); }
        finally { _busy = false; await RefreshAsync(); }
    }

    private void Render()
    {
        if (_snapshot is null) return;
        _root.Children.Clear();
        _root.Children.Add(new TextBlock { Text = T("title"), FontSize = 22 });
        _root.Children.Add(new TextBlock { Text = T("description"), TextWrapping = TextWrapping.Wrap });
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        actions.Children.Add(Button("launch", LaunchAsync, _snapshot.Capabilities.TerminalLaunch && _snapshot.Accounts.Length > 0));
        actions.Children.Add(Button("add", AddAsync));
        actions.Children.Add(Button("refresh", RefreshAsync));
        _root.Children.Add(actions);
        _root.Children.Add(new TextBlock { Text = _snapshot.Capabilities.TerminalDestination is string destination
            ? T("terminal_scope").Replace("{destination}", destination) : T("terminal_unavailable"), TextWrapping = TextWrapping.Wrap });
        _root.Children.Add(_rows);
        _rows.Children.Clear();
        foreach (var account in _snapshot.Accounts)
        {
            bool held = _snapshot.Sessions.Any(s => s.AccountId == account.Id && s.HoldsAccount);
            var row = new StackPanel { Spacing = 4 };
            row.Children.Add(new TextBlock { Text = $"{account.Label} · {T(account.Tool)} · {T(account.Connection)} · {account.AuthState}" });
            var controls = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            controls.Children.Add(Button("use_default", async () => {
                ulong generation = _snapshot.Generations.GetValueOrDefault(account.Tool);
                await CallAsync("managed_select", new { selection = new { tool = account.Tool, connection = account.Connection, account_id = account.Id, generation }, expected_generation = generation });
            }));
            controls.Children.Add(Button("rename_short", () => RenameAsync(account)));
            if (account.Connection == "subscription") controls.Children.Add(Button("reconnect", async () => {
                await OpenAsync(await CallAsync("managed_account_reconnect", new { account_id = account.Id }));
            }, !held && _snapshot.Capabilities.TerminalLaunch));
            controls.Children.Add(Button("remove", async () => {
                var dialog = Dialog("remove_question", new TextBlock { Text = T("remove_description"), TextWrapping = TextWrapping.Wrap }, "remove");
                if (await dialog.ShowAsync() == ContentDialogResult.Primary) await CallAsync("managed_account_remove", new { account_id = account.Id });
            }, !held));
            row.Children.Add(controls);
            _rows.Children.Add(row);
        }
        foreach (var session in _snapshot.Sessions)
        {
            var row = new StackPanel { Spacing = 4 };
            row.Children.Add(new TextBlock { Text = $"{(session.Purpose == "login" ? T("sign_in") : session.ProjectLabel)} · {T(session.Tool)} · {session.AccountLabel} · {T(session.Connection)} · {session.State}" });
            if (!session.HoldsAccount) row.Children.Add(Button("dismiss", async () => { await CallAsync("managed_session_dismiss", new { session_id = session.Id }); }));
            _rows.Children.Add(row);
        }
        if (_snapshot.Sessions.Length == 0) _rows.Children.Add(new TextBlock { Text = T("empty") });
        _root.Children.Add(_error);
        _root.Children.Add(new TextBlock { Text = T("global_title"), FontSize = 20 });
        _root.Children.Add(new TextBlock { Text = T("global_scope"), TextWrapping = TextWrapping.Wrap });
    }

    private ContentDialog Dialog(string title, UIElement content, string primary) => new() {
        XamlRoot = XamlRoot, Title = T(title), Content = content,
        PrimaryButtonText = T(primary), CloseButtonText = T("cancel")
    };

    private async Task OpenAsync(JsonElement prepared)
    {
        await CallAsync("managed_terminal_launch", new { session_id = prepared.GetProperty("session_id").GetString(), ticket = prepared.GetProperty("ticket").GetString() });
    }

    private async Task RenameAsync(ManagedAccount account)
    {
        var name = new TextBox { Text = account.Label, Header = T("label") };
        if (await Dialog("rename", name, "save").ShowAsync() == ContentDialogResult.Primary)
            await CallAsync("managed_account_rename", new { account_id = account.Id, label = name.Text });
    }

    private async Task AddAsync()
    {
        string[] tools = ["claude", "codex"], connections = ["subscription", "api_key", "near_ai"];
        var tool = new ComboBox { Header = T("tool"), ItemsSource = tools.Select(T).ToArray(), SelectedIndex = 0 };
        var connection = new ComboBox { Header = T("connection"), ItemsSource = connections.Select(T).ToArray(), SelectedIndex = 0 };
        var label = new TextBox { Header = T("label") };
        var key = new PasswordBox { Header = T("api_key"), Visibility = Visibility.Collapsed };
        connection.SelectionChanged += (_, _) => key.Visibility = connection.SelectedIndex == 0 ? Visibility.Collapsed : Visibility.Visible;
        var form = new StackPanel { Spacing = 10 };
        form.Children.Add(tool); form.Children.Add(connection); form.Children.Add(label); form.Children.Add(key);
        form.Children.Add(new TextBlock { Text = T("login_description"), TextWrapping = TextWrapping.Wrap });
        var dialog = Dialog("save_title", form, "save");
        if (await dialog.ShowAsync() != ContentDialogResult.Primary || string.IsNullOrWhiteSpace(label.Text)) return;
        string kind = connections[connection.SelectedIndex];
        if (kind == "subscription" && _snapshot?.Capabilities.TerminalLaunch != true) throw new InvalidOperationException();
        var account = await CallAsync("managed_account_add", new { tool = tools[tool.SelectedIndex], connection = kind, label = label.Text });
        string id = account.GetProperty("id").GetString()!;
        if (kind == "subscription") await OpenAsync(await CallAsync("managed_account_reconnect", new { account_id = id }));
        else { await CallAsync("managed_account_set_key", new { account_id = id, key = key.Password }); key.Password = string.Empty; }
    }

    private async Task LaunchAsync()
    {
        if (_snapshot is null) return;
        var accounts = _snapshot.Accounts;
        var account = new ComboBox { Header = T("saved_account"), ItemsSource = accounts.Select(a => $"{T(a.Tool)} · {a.Label} · {T(a.Connection)}").ToArray(), SelectedIndex = 0 };
        var project = new TextBox { Header = T("project_placeholder") };
        var form = new StackPanel { Spacing = 10 };
        form.Children.Add(account); form.Children.Add(project);
        var choose = new Button { Content = T("choose_folder") };
        choose.Click += async (_, _) => {
            var picker = new Windows.Storage.Pickers.FolderPicker();
            picker.FileTypeFilter.Add("*");
            var handle = Microsoft.UI.Win32Interop.GetWindowFromWindowId(XamlRoot.ContentIslandEnvironment.AppWindowId);
            WinRT.Interop.InitializeWithWindow.Initialize(picker, handle);
            var selectedFolder = await picker.PickSingleFolderAsync();
            if (selectedFolder is not null) project.Text = selectedFolder.Path;
        };
        form.Children.Add(choose);
        form.Children.Add(new TextBlock { Text = T("launch_scope").Replace("{destination}", _snapshot.Capabilities.TerminalDestination ?? T("terminal")), TextWrapping = TextWrapping.Wrap });
        if (await Dialog("launch", form, "launch_short").ShowAsync() != ContentDialogResult.Primary || string.IsNullOrWhiteSpace(project.Text)) return;
        var selected = accounts[account.SelectedIndex];
        await OpenAsync(await CallAsync("managed_launch_prepare", new { request_id = Guid.NewGuid(), purpose = "coding", tool = selected.Tool, connection = selected.Connection, account_id = selected.Id, cwd = project.Text, expected_generation = _snapshot.Generations.GetValueOrDefault(selected.Tool), save_default = false }));
    }
}
