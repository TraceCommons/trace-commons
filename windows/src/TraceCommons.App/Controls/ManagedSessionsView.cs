using System;
using System.Linq;
using System.Collections.Generic;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;

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
    private bool _unsupportedVersion;
    /// <summary>The launch button a closed dialog hands focus back to: the
    /// dialog's own return can land after the rebuild, or nowhere.</summary>
    private (string Name, int Index)? _returnFocus;
    private string? _unsupportedTool;
    /// <summary>The last refusal, said on the card for a launch begun there.</summary>
    private readonly ManagedFailure _failure = new();
    /// <summary>Whether the action now running was begun from the card.</summary>
    private bool _fromCard;

    /// <summary>A managed action the daemon refused, with its label.</summary>
    private sealed class Refused(string code) : Exception
    {
        public string Code { get; } = code;
    }

    public ManagedSessionsView(DaemonHost host)
    {
        _host = host;
        Content = _root;
        // The 10 s poll stops while the window is hidden to the tray or
        // minimised (WindowClocks), and resumes with a read when it is back.
        // The subscription lives while the view is loaded: WindowClocks is
        // static, and a handler left on it would keep this view alive.
        Loaded += async (_, _) =>
        {
            WindowClocks.Changed -= OnClocksChanged;
            WindowClocks.Changed += OnClocksChanged;
            await RefreshAsync();
            if (WindowClocks.Running) _timer.Start();
        };
        Unloaded += (_, _) =>
        {
            WindowClocks.Changed -= OnClocksChanged;
            _timer.Stop();
        };
        _timer.Tick += async (_, _) => await RefreshAsync();
    }

    private async void OnClocksChanged(bool running)
    {
        if (!running)
        {
            _timer.Stop();
        }
        else if (IsLoaded)
        {
            await RefreshAsync();
            _timer.Start();
        }
    }

    private string T(string key) => _snapshot?.Text(key) ?? string.Empty;

    private async Task<JsonElement> CallAsync(string method, object parameters)
    {
        var response = await _host.CallAsync(method, JsonSerializer.Serialize(parameters));
        if (response.Error?.Message == "managed-unsupported-version")
        {
            _unsupportedVersion = true;
            var request = JsonSerializer.SerializeToElement(parameters);
            _unsupportedTool = request.TryGetProperty("tool", out var tool) ? tool.GetString() : null;
        }
        if (response.IsError) throw new Refused(response.Error!.Message);
        if (response.Result is null) throw new InvalidOperationException();
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

    /// <summary>Install for one tool, named with the tool for Narrator;
    /// <paramref name="withName"/> shows the name too, as the Mac's card does.</summary>
    private Button InstallButton(ManagedTool tool, bool withName = false)
    {
        var install = Button("install", async () => { await Windows.System.Launcher.LaunchUriAsync(new Uri(tool.InstallUrl)); });
        string named = $"{T("install")} {T(tool.Tool)}";
        if (withName) install.Content = named;
        AutomationProperties.SetName(install, named);
        return install;
    }

    /// <summary>A bare refresh glyph: no text, its tooltip and accessible name "Refresh".</summary>
    private Button RefreshGlyph()
    {
        var glyph = new Button { Content = new FontIcon { Glyph = "\uE72C", FontSize = 14 }, Background = null, BorderThickness = new Thickness(0), IsEnabled = !_busy };
        glyph.Click += async (_, _) => await RefreshAsync();
        ToolTipService.SetToolTip(glyph, T("refresh"));
        AutomationProperties.SetName(glyph, T("refresh"));
        return glyph;
    }

    private async Task RunAsync(Func<Task> action)
    {
        if (_busy) return;
        _busy = true;
        _error.Text = string.Empty;
        _failure.Begin();
        _unsupportedVersion = false;
        _unsupportedTool = null;
        try { await action(); }
        catch (Refused refused)
        {
            // `{terminal}` is where the session opens: herdr while grouped
            // there. This shell runs no tool setup, so every refusal here is
            // a launch's. A launch begun on the card is said under its
            // buttons (ManagedSessionsView.swift:131-136, :160-163); an
            // unsupported version keeps its recovery row below.
            string key = _unsupportedVersion ? "unsupported_version" : ManagedSignIn.ErrorKey(refused.Code);
            if (_fromCard && !_unsupportedVersion) _failure.Refuse(key, fromCard: true);
            else _error.Text = ManagedSignIn.FillTerminal(T(key), _snapshot?.Capabilities.SessionDestination, T("terminal"));
        }
        catch
        {
            if (_fromCard) _failure.Refuse("action_failed", fromCard: true);
            else _error.Text = T("action_failed");
        }
        finally { _busy = false; _fromCard = false; await RefreshAsync(); }
    }

    /// <summary>
    /// Rebuilds the view, keeping keyboard focus on the same button: every
    /// read rebuilds it, so without this the 10 s poll, or closing the launch
    /// dialog, dropped focus to the window's first control (box, 2026-11-19).
    /// </summary>
    private void Render()
    {
        if (_snapshot is null) return;
        // While a dialog is open (busy) a poll's rebuild keeps the return.
        var (target, keep) = ManagedFocus.Rebuild(FocusedButtonKey(), _busy, _returnFocus);
        _returnFocus = keep;
        RenderContent();
        if (target is { } key) RestoreFocus(key);
    }

    private static string ButtonName(Button button) =>
        AutomationProperties.GetName(button) is { Length: > 0 } name ? name : button.Content as string ?? string.Empty;

    private static IEnumerable<Button> Buttons(Panel panel)
    {
        foreach (var child in panel.Children)
        {
            if (child is Button button) yield return button;
            else if (child is Panel inner) foreach (var nested in Buttons(inner)) yield return nested;
        }
    }

    /// <summary>The focused button's name and its place among buttons of that name, or null.</summary>
    private (string Name, int Index)? FocusedButtonKey()
    {
        if (XamlRoot is null || FocusManager.GetFocusedElement(XamlRoot) is not Button focused) return null;
        var buttons = Buttons(_root).ToList();
        return ManagedFocus.KeyOf(buttons.Select(ButtonName).ToList(), buttons.IndexOf(focused));
    }

    private void RestoreFocus((string Name, int Index) key)
    {
        var buttons = Buttons(_root).ToList();
        int at = ManagedFocus.Find(buttons.Select(ButtonName).ToList(), key);
        if (at < 0) return;
        Button match = buttons[at];
        DispatcherQueue?.TryEnqueue(() => match.Focus(FocusState.Programmatic));
    }

    private void RenderContent()
    {
        if (_snapshot is null) return;
        _root.Children.Clear();
        RenderSignIn();
        _root.Children.Add(new TextBlock { Text = T("title"), FontSize = 22 });
        _root.Children.Add(new TextBlock { Text = T("description"), TextWrapping = TextWrapping.Wrap });
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        actions.Children.Add(Button("launch", LaunchAsync, _snapshot.Capabilities.TerminalLaunch && _snapshot.Accounts.Any(a => _snapshot.CanLaunch(a.Tool))));
        // Hidden, not disabled, with no tool to add an account for (Ron, 2026-10-15).
        if (_snapshot.SetupTools.Length > 0) actions.Children.Add(Button("add", AddAsync));
        _root.Children.Add(actions);
        // Signed in, the card above holds the Install links (box,
        // 2026-11-19: both drew them); signed out, this block stands in for
        // the Mac's setup row (ManagedSessionsView.swift:34-36).
        if (_snapshot.InstallTools.Length > 0 && !_snapshot.NearAiSignedIn)
        {
            // One bare refresh glyph beside the heading; no per-tool copies.
            var heading = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
            heading.Children.Add(new TextBlock { Text = T("tools_title"), FontSize = 20, VerticalAlignment = VerticalAlignment.Center });
            heading.Children.Add(RefreshGlyph());
            _root.Children.Add(heading);
            _root.Children.Add(new TextBlock { Text = T("tools_empty"), TextWrapping = TextWrapping.Wrap });
            foreach (var tool in _snapshot.InstallTools)
            {
                var install = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
                install.Children.Add(new TextBlock { Text = T(tool.Tool), VerticalAlignment = VerticalAlignment.Center });
                install.Children.Add(InstallButton(tool));
                _root.Children.Add(install);
            }
        }
        // After an unsupported-version refusal: the refused tool, with its
        // setup guide to update from and the refresh glyph.
        if (_unsupportedVersion)
        {
            foreach (var tool in _snapshot.VersionRecoveryTools(_unsupportedTool))
            {
                var recovery = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
                recovery.Children.Add(new TextBlock { Text = $"{T(tool.Tool)} · {T("unsupported_version")}", TextWrapping = TextWrapping.Wrap, VerticalAlignment = VerticalAlignment.Center });
                recovery.Children.Add(Button("update", async () => { await Windows.System.Launcher.LaunchUriAsync(new Uri(tool.InstallUrl)); }));
                recovery.Children.Add(RefreshGlyph());
                _root.Children.Add(recovery);
            }
        }
        _root.Children.Add(new TextBlock { Text = _snapshot.Capabilities.SessionDestination is string destination
            ? T("terminal_scope").Replace("{destination}", destination) : T("terminal_unavailable"), TextWrapping = TextWrapping.Wrap });
        _root.Children.Add(_rows);
        _rows.Children.Clear();
        foreach (var account in _snapshot.Accounts)
        {
            bool held = _snapshot.Sessions.Any(s => s.AccountId == account.Id && s.HoldsAccount);
            var row = new StackPanel { Spacing = 4 };
            row.Children.Add(new TextBlock { Text = $"{account.Label} · {T(account.Tool)} · {T(account.Connection)} · {account.AuthState}" });
            if (_snapshot.ToolMissing(account.Tool))
            {
                row.Children.Add(new TextBlock { Text = _snapshot.ToolUnavailable(account.Tool), TextWrapping = TextWrapping.Wrap });
                var recovery = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
                if (_snapshot.Tools?.FirstOrDefault(t => t.Tool == account.Tool) is ManagedTool tool) recovery.Children.Add(InstallButton(tool));
                recovery.Children.Add(RefreshGlyph());
                row.Children.Add(recovery);
            }
            var controls = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            controls.Children.Add(Button("use_default", async () => {
                ulong generation = _snapshot.Generations.GetValueOrDefault(account.Tool);
                await CallAsync("managed_select", new { selection = new { tool = account.Tool, connection = account.Connection, account_id = account.Id, generation }, expected_generation = generation });
            }));
            controls.Children.Add(Button("rename_short", () => RenameAsync(account)));
            // Hidden, not disabled, while the tool is missing.
            if (_snapshot.OffersReconnect(account)) controls.Children.Add(Button("reconnect", async () => {
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
            if (session.Model is string model) row.Children.Add(new TextBlock { Text = $"{T("model")}: {model}" });
            if (session.Tool == "opencode") row.Children.Add(new TextBlock { Text = T($"collection_{session.Collection}") });
            if (!session.HoldsAccount) row.Children.Add(Button("dismiss", async () => { await CallAsync("managed_session_dismiss", new { session_id = session.Id }); }));
            _rows.Children.Add(row);
        }
        if (_snapshot.Sessions.Length == 0) _rows.Children.Add(new TextBlock { Text = T("empty") });
        _root.Children.Add(_error);
        _root.Children.Add(new TextBlock { Text = T("global_title"), FontSize = 20 });
        _root.Children.Add(new TextBlock { Text = T("global_scope"), TextWrapping = TextWrapping.Wrap });
    }

    /// <summary>
    /// Sessions on the NEAR AI sign-in, first (owner, 2026-10-12), drawn only
    /// while signed in, as on macOS (ManagedSessionsView.swift:30-33, :80-98):
    /// the sign-in title, the line naming where sessions open, and at its
    /// foot the Start button for each tool, its logo and name, or in their
    /// place the no-terminal notice or the install line; the Install link of
    /// each missing tool beside them; then a launch begun here that failed.
    /// Signed out the Mac draws no card, and nor does this: the page's
    /// account section holds the sign-in. Nothing here is saved.
    /// </summary>
    private void RenderSignIn()
    {
        if (_snapshot is null || !_snapshot.NearAiSignedIn) return;
        var section = new StackPanel { Spacing = 6 };
        section.Children.Add(new TextBlock { Text = T("sign_in_title"), FontSize = 20 });
        section.Children.Add(new TextBlock
        {
            Text = ManagedSignIn.Scope(T("sign_in_scope"), _snapshot.Capabilities.SessionDestination, T("terminal")),
            TextWrapping = TextWrapping.Wrap,
        });
        switch (_snapshot.ToolArea)
        {
            case ManagedToolArea.NoTerminal:
                section.Children.Add(new TextBlock { Text = T("terminal_unavailable"), TextWrapping = TextWrapping.Wrap });
                break;
            case ManagedToolArea.Install:
                section.Children.Add(new TextBlock { Text = T("tools_empty"), TextWrapping = TextWrapping.Wrap });
                break;
        }

        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        if (_snapshot.ToolArea == ManagedToolArea.Buttons)
        {
            foreach (string tool in _snapshot.SignInStartTools)
            {
                if (ManagedSignIn.StartKey(tool) is not string key) continue;
                actions.Children.Add(ToolButton(tool, key));
            }
        }

        // ManagedInstallLink(includeToolName: true): "Install {tool}".
        foreach (var tool in _snapshot.MissingInstallTools) actions.Children.Add(InstallButton(tool, withName: true));
        if (actions.Children.Count > 0) section.Children.Add(actions);
        if (_failure.OnCard(cardShown: true))
        {
            foreach (string line in _failure.CardLines(T, _snapshot.Capabilities.SessionDestination))
            {
                section.Children.Add(new TextBlock { Text = line, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true });
            }
        }

        _root.Children.Add(section);
    }

    /// <summary>
    /// A tool's launch button (ManagedFolderExplainer.swift:142-171): its
    /// logo in its tint (or its initials) and its name, named for assistive
    /// tech by the core's Start label, as the Mac's
    /// <c>.accessibilityLabel(startKey)</c>.
    /// </summary>
    private Button ToolButton(string tool, string startKey)
    {
        var face = new StackPanel { Orientation = Orientation.Horizontal, Spacing = LaunchHandoff.ButtonSpacing };
        face.Children.Add(StepGeometry.Mark(tool, LaunchHandoff.ButtonLogoSide, 10));
        face.Children.Add(new TextBlock { Text = T(tool), MaxLines = 1, VerticalAlignment = VerticalAlignment.Center });
        var button = new Button { Content = face, IsEnabled = !_busy };
        AutomationProperties.SetName(button, T(startKey));
        button.Click += async (_, _) =>
        {
            _returnFocus = (T(startKey), 0);
            _fromCard = true;
            await RunAsync(() => StartAsync(tool));
        };
        return button;
    }

    /// <summary>
    /// A tool's launch dialog (owner, 2026-10-19; ManagedFolderExplainer.swift:11-140):
    /// the Start label as its title, the folder sliding into a terminal, the
    /// folder row (the home folder until one is picked, Pick a folder as
    /// context, and Use home once one is), the terminal row while launches
    /// ask, then Cancel and Open {tool}. The folder is optional (owner,
    /// 2026-10-29): with none, the daemon opens the tool in the home folder.
    /// </summary>
    private async Task StartAsync(string tool)
    {
        if (_snapshot is null || ManagedSignIn.StartKey(tool) is not string startKey) return;
        string? path = null;
        var form = new StackPanel { Spacing = 12, MinWidth = 320 };
        var art = new LaunchArtView(tool) { HorizontalAlignment = HorizontalAlignment.Center };
        form.Children.Add(art);
        var folderRow = new Grid { ColumnSpacing = 8 };
        folderRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        folderRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        folderRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        folderRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var tile = new Border
        {
            Width = 22, Height = 22, CornerRadius = new CornerRadius(6),
            Background = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TcTileFolderBrush"],
            Child = new FontIcon { Glyph = LaunchHandoff.FolderGlyph, FontSize = 11 },
        };
        AutomationProperties.SetAccessibilityView(tile, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        var shown = new TextBlock { VerticalAlignment = VerticalAlignment.Center, MaxLines = 1, TextTrimming = TextTrimming.CharacterEllipsis, IsTextSelectionEnabled = true };
        var clear = new Button { Content = T("clear_folder") };
        var pick = new Button { Content = T("pick_folder") };
        void ShowFolder()
        {
            shown.Text = path ?? T("home_folder");
            ToolTipService.SetToolTip(shown, path);
            clear.Visibility = path is null ? Visibility.Collapsed : Visibility.Visible;
        }
        clear.Click += (_, _) => { path = null; ShowFolder(); };
        pick.Click += async (_, _) =>
        {
            if (await PickFolderAsync() is string chosen && ManagedLaunchFolder.IsAcceptable(chosen)) { path = chosen; ShowFolder(); }
        };
        ShowFolder();
        Grid.SetColumn(tile, 0); Grid.SetColumn(shown, 1); Grid.SetColumn(clear, 2); Grid.SetColumn(pick, 3);
        folderRow.Children.Add(tile); folderRow.Children.Add(shown); folderRow.Children.Add(clear); folderRow.Children.Add(pick);
        form.Children.Add(folderRow);
        // Asked on every launch while more than one terminal is installed (#1891).
        var terminal = _snapshot.AsksForTerminal ? TerminalPicker() : null;
        if (terminal is not null) form.Children.Add(terminal);
        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot, Title = T(startKey), Content = form,
            PrimaryButtonText = ManagedSignIn.OpenLabel(T("open_tool"), T(tool)),
            CloseButtonText = T("cancel"), DefaultButton = ContentDialogButton.Primary,
        };
        // Open is the default action (ManagedFolderExplainer.swift:62,
        // isDefault): focus starts on it, so Enter opens the tool rather than
        // the folder picker.
        dialog.Opened += (_, _) => FocusPrimary(dialog);
        ContentDialogResult result = await dialog.ShowAsync();
        art.Release();
        if (result != ContentDialogResult.Primary) return;
        // Decided from the folder and the snapshot at send time (L4).
        if (_snapshot?.SignInLaunchParams(tool, path, Guid.NewGuid().ToString()) is not { } request) return;
        if (terminal is not null) await SaveTerminalAsync(terminal.SelectedIndex);
        await OpenAsync(await CallAsync("managed_launch_prepare", request));
    }

    /// <summary>Focuses a dialog's primary button: the template part WinUI
    /// names <c>PrimaryButton</c>.</summary>
    private static void FocusPrimary(ContentDialog dialog)
    {
        if (FindButton(dialog, "PrimaryButton") is Button primary) primary.Focus(FocusState.Programmatic);
    }

    private static Button? FindButton(DependencyObject parent, string name)
    {
        int count = Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChildrenCount(parent);
        for (int i = 0; i < count; i++)
        {
            DependencyObject child = Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChild(parent, i);
            if (child is Button button && button.Name == name) return button;
            if (FindButton(child, name) is Button found) return found;
        }
        return null;
    }

    /// <summary>The "Open sessions in" row of a launch that asks: the core's
    /// label over the installed terminals, starting on the current one.</summary>
    private ComboBox TerminalPicker()
    {
        var picker = new ComboBox { Header = T("terminal_choice"), MinWidth = 220 };
        AutomationProperties.SetName(picker, T("terminal_choice"));
        foreach (var choice in _snapshot!.TerminalChoices) picker.Items.Add(choice.Name);
        picker.SelectedIndex = _snapshot.ProposedTerminalIndex;
        return picker;
    }

    /// <summary>Writes the picked terminal as the default before the launch.</summary>
    private async Task SaveTerminalAsync(int pickedIndex)
    {
        if (_snapshot?.TerminalToSave(pickedIndex) is string id)
            await CallAsync("set_managed_terminal", new { terminal = id });
    }

    private async Task<string?> PickFolderAsync()
    {
        var picker = new Windows.Storage.Pickers.FolderPicker();
        picker.FileTypeFilter.Add("*");
        var handle = Microsoft.UI.Win32Interop.GetWindowFromWindowId(XamlRoot.ContentIslandEnvironment.AppWindowId);
        WinRT.Interop.InitializeWithWindow.Initialize(picker, handle);
        return (await picker.PickSingleFolderAsync())?.Path;
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
        string[] tools = _snapshot?.SetupTools ?? [];
        if (tools.Length == 0) return;
        bool signedIn = _snapshot?.NearAiSignedIn == true;
        string[] connections = ManagedSnapshot.Connections(tools[0], signedIn);
        var tool = new ComboBox { Header = T("tool"), ItemsSource = tools.Select(T).ToArray(), SelectedIndex = 0 };
        var connection = new ComboBox { Header = T("connection"), ItemsSource = connections.Select(T).ToArray(), SelectedIndex = 0 };
        var label = new TextBox { Header = T("label") };
        var key = new PasswordBox { Header = T("api_key"), Visibility = Visibility.Collapsed };
        connection.SelectionChanged += (_, _) => key.Visibility = connection.SelectedIndex >= 0 && connection.SelectedIndex < connections.Length && connections[connection.SelectedIndex] == "subscription" ? Visibility.Collapsed : Visibility.Visible;
        key.Visibility = connections[0] == "subscription" ? Visibility.Collapsed : Visibility.Visible;
        var openCodeScope = new TextBlock { Text = T("opencode_scope"), TextWrapping = TextWrapping.Wrap, Visibility = tools[0] == "opencode" ? Visibility.Visible : Visibility.Collapsed };
        tool.SelectionChanged += (_, _) => {
            if (tool.SelectedIndex < 0) return;
            openCodeScope.Visibility = tools[tool.SelectedIndex] == "opencode" ? Visibility.Visible : Visibility.Collapsed;
            connections = ManagedSnapshot.Connections(tools[tool.SelectedIndex], signedIn);
            connection.ItemsSource = connections.Select(T).ToArray(); connection.SelectedIndex = 0;
            key.Visibility = connections[0] == "subscription" ? Visibility.Collapsed : Visibility.Visible;
        };
        var form = new StackPanel { Spacing = 10 };
        form.Children.Add(openCodeScope);
        form.Children.Add(tool); form.Children.Add(connection); form.Children.Add(label); form.Children.Add(key);
        var loginDescription = new TextBlock { Text = T("login_description"), TextWrapping = TextWrapping.Wrap };
        void UpdateLoginDescription() => loginDescription.Visibility = connection.SelectedIndex >= 0 && connection.SelectedIndex < connections.Length && connections[connection.SelectedIndex] == "subscription" ? Visibility.Visible : Visibility.Collapsed;
        connection.SelectionChanged += (_, _) => UpdateLoginDescription(); tool.SelectionChanged += (_, _) => UpdateLoginDescription(); UpdateLoginDescription();
        form.Children.Add(loginDescription);
        form.Children.Add(new TextBlock { Text = T("key_storage"), TextWrapping = TextWrapping.Wrap });
        var dialog = Dialog("save_title", form, "save");
        void ValidateAccount() => dialog.IsPrimaryButtonEnabled = connection.SelectedIndex >= 0 && connection.SelectedIndex < connections.Length && !string.IsNullOrWhiteSpace(label.Text) && (connections[connection.SelectedIndex] == "subscription" ? _snapshot?.Capabilities.TerminalLaunch == true : !string.IsNullOrWhiteSpace(key.Password));
        label.TextChanged += (_, _) => ValidateAccount(); key.PasswordChanged += (_, _) => ValidateAccount();
        connection.SelectionChanged += (_, _) => ValidateAccount(); ValidateAccount();
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
        var accounts = _snapshot.Accounts.Where(a => _snapshot.CanLaunch(a.Tool)).ToArray();
        if (accounts.Length == 0) return;
        var account = new ComboBox { Header = T("saved_account"), ItemsSource = accounts.Select(a => $"{T(a.Tool)} · {a.Label} · {T(a.Connection)}").ToArray(), SelectedIndex = 0 };
        var project = new TextBox { Header = T("project_placeholder") };
        var form = new StackPanel { Spacing = 10 };
        form.Children.Add(account); form.Children.Add(project);
        var model = new TextBox { Header = T("model") };
        var collect = new CheckBox { Content = T("collect_traces"), IsChecked = false };
        var scope = new TextBlock { Text = T("opencode_scope") + " " + T("collection_scope"), TextWrapping = TextWrapping.Wrap };
        void UpdateOpenCode() {
            var visible = account.SelectedIndex >= 0 && accounts[account.SelectedIndex].Tool == "opencode" ? Visibility.Visible : Visibility.Collapsed;
            model.Visibility = collect.Visibility = scope.Visibility = visible;
            collect.IsChecked = false;
        }
        account.SelectionChanged += (_, _) => UpdateOpenCode();
        UpdateOpenCode(); form.Children.Add(model); form.Children.Add(collect); form.Children.Add(scope);
        var choose = new Button { Content = T("choose_folder") };
        choose.Click += async (_, _) => {
            if (await PickFolderAsync() is string path) project.Text = path;
        };
        form.Children.Add(choose);
        var terminalPicker = _snapshot.AsksForTerminal ? TerminalPicker() : null;
        if (terminalPicker is not null) form.Children.Add(terminalPicker);
        form.Children.Add(new TextBlock { Text = T("launch_scope").Replace("{destination}", _snapshot.Capabilities.SessionDestination ?? T("terminal")), TextWrapping = TextWrapping.Wrap });
        var launchDialog = Dialog("launch_title", form, "launch_short");
        void ValidateLaunch() => launchDialog.IsPrimaryButtonEnabled = account.SelectedIndex >= 0 && ManagedLaunchFolder.IsAcceptable(project.Text) && (accounts[account.SelectedIndex].Tool != "opencode" || !string.IsNullOrWhiteSpace(model.Text));
        account.SelectionChanged += (_, _) => ValidateLaunch();
        model.TextChanged += (_, _) => ValidateLaunch(); project.TextChanged += (_, _) => ValidateLaunch(); ValidateLaunch();
        if (await launchDialog.ShowAsync() != ContentDialogResult.Primary || !ManagedLaunchFolder.IsAcceptable(project.Text)) return;
        var selected = accounts[account.SelectedIndex];
        // Decided from the typed folder at send time (L4).
        if (_snapshot.AccountLaunchParams(selected, project.Text, model.Text, collect.IsChecked == true, Guid.NewGuid().ToString()) is not { } request) return;
        if (terminalPicker is not null) await SaveTerminalAsync(terminalPicker.SelectedIndex);
        await OpenAsync(await CallAsync("managed_launch_prepare", request));
    }
}
