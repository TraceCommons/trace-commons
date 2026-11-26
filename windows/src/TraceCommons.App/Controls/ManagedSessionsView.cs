using System;
using System.Linq;
using System.Collections.Generic;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;
using TraceCommons.App.Look;

namespace TraceCommons.App.Controls;

/// <summary>Shared daemon accounts and sessions; all fixed copy comes from its snapshot.</summary>
public sealed partial class ManagedSessionsView : UserControl
{
    private readonly DaemonHost _host;
    private readonly bool _savedAccounts;
    private readonly StackPanel _root = new() { Spacing = 10 };
    private readonly TextBlock _error = new() { TextWrapping = TextWrapping.Wrap };
    private readonly DispatcherTimer _timer = new() { Interval = TimeSpan.FromSeconds(10) };
    private ManagedSnapshot? _snapshot;
    private string? _snapshotJson;
    private readonly Dictionary<string, ManagedLaunchCatalogue> _modelCatalogues = new();
    private readonly HashSet<string> _modelCatalogueReads = new();
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

    public ManagedSessionsView(DaemonHost host) : this(host, savedAccounts: false) { }

    internal ManagedSessionsView(DaemonHost host, bool savedAccounts)
    {
        _host = host;
        _savedAccounts = savedAccounts;
        Content = _root;
        if (_savedAccounts) RenderSavedAccounts();
        // The 10 s poll stops while the window is hidden to the tray or
        // minimised (WindowClocks), and resumes with a read when it is back.
        // The subscription lives while the view is loaded: WindowClocks is
        // static, and a handler left on it would keep this view alive.
        Loaded += async (_, _) =>
        {
            WindowClocks.Changed -= OnClocksChanged;
            WindowClocks.Changed += OnClocksChanged;
            ThemeBrushes.Changed -= Render;
            ThemeBrushes.Changed += Render;
            await RefreshAsync();
            if (WindowClocks.Running) _timer.Start();
        };
        Unloaded += (_, _) =>
        {
            WindowClocks.Changed -= OnClocksChanged;
            ThemeBrushes.Changed -= Render;
            _timer.Stop();
            _modelCatalogueReads.Clear();
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

    private string T(string key) => _snapshot?.Text(key) ?? (_savedAccounts ? ManagedWords.Text(key) : string.Empty);

    private async Task<JsonElement> CallAsync(string method, object parameters, bool trackUnsupportedVersion = true)
    {
        var response = await _host.CallAsync(method, JsonSerializer.Serialize(parameters));
        if (trackUnsupportedVersion && response.Error?.Message == "managed-unsupported-version")
        {
            _unsupportedVersion = true;
            var request = JsonSerializer.SerializeToElement(parameters);
            _unsupportedTool = request.TryGetProperty("tool", out var tool) ? tool.GetString() : null;
        }
        if (response.IsError) throw new Refused(response.Error!.Message);
        if (response.Result is null) throw new InvalidOperationException();
        return response.Result.Value.Clone();
    }

    private async Task RefreshAsync(bool forceRender = false)
    {
        if (_reading) return;
        _reading = true;
        try
        {
            var value = await CallAsync("managed_snapshot", new { });
            string json = value.GetRawText();
            if (!forceRender && json == _snapshotJson)
            {
                if (!_savedAccounts && _snapshot is not null) _ = LoadSessionModelNamesAsync(_snapshot);
                return;
            }

            var next = ManagedSnapshot.Parse(json);
            if (next is null || (_snapshot is not null && next.Revision < _snapshot.Revision)) return;
            _snapshot = next;
            Render();
            _snapshotJson = json;
            if (!_savedAccounts) _ = LoadSessionModelNamesAsync(next);
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
    private Button RefreshGlyph(string key = "refresh")
    {
        var glyph = new Button { Content = new FontIcon { Glyph = "\uE72C", FontSize = 14 }, Background = null, BorderThickness = new Thickness(0), IsEnabled = !_busy };
        glyph.Click += async (_, _) => await RefreshAsync(forceRender: true);
        ToolTipService.SetToolTip(glyph, T(key));
        AutomationProperties.SetName(glyph, T(key));
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
            if (_savedAccounts || (_fromCard && !_unsupportedVersion)) _failure.Refuse(key, fromCard: true);
            else _error.Text = ManagedSignIn.FillTerminal(T(key), _snapshot?.Capabilities.SessionDestination, T("terminal"));
        }
        catch
        {
            if (_savedAccounts || _fromCard) _failure.Refuse("action_failed", fromCard: true);
            else _error.Text = T("action_failed");
        }
        finally { _busy = false; _fromCard = false; await RefreshAsync(forceRender: true); }
    }

    /// <summary>
    /// Preserves the focused button when a snapshot changes or a dialog closes.
    /// </summary>
    private void Render()
    {
        if (_snapshot is null && !_savedAccounts) return;
        // While a dialog is open (busy) a poll's rebuild keeps the return.
        bool returningFromDialog = !_busy && _returnFocus is not null;
        var (target, keep) = ManagedFocus.Rebuild(FocusedButtonKey(), _busy, _returnFocus);
        _returnFocus = keep;
        if (_savedAccounts) RenderSavedAccounts();
        else RenderContent();
        if (target is { } key) RestoreFocus(key, returningFromDialog);
    }

    private static string ButtonName(Button button) =>
        AutomationProperties.GetAutomationId(button) is { Length: > 0 } id ? id :
            AutomationProperties.GetName(button) is { Length: > 0 } name ? name : button.Content as string ?? string.Empty;

    private static IEnumerable<Button> Buttons(Panel panel)
    {
        foreach (var child in panel.Children)
        {
            if (child is Button button) yield return button;
            else if (child is Panel inner) foreach (var nested in Buttons(inner)) yield return nested;
            else if (child is Border { Child: Panel body }) foreach (var nested in Buttons(body)) yield return nested;
        }
    }

    /// <summary>The focused button's name and its place among buttons of that name, or null.</summary>
    private (string Name, int Index)? FocusedButtonKey()
    {
        if (XamlRoot is null || FocusManager.GetFocusedElement(XamlRoot) is not Button focused) return null;
        var buttons = Buttons(_root).ToList();
        return ManagedFocus.KeyOf(buttons.Select(ButtonName).ToList(), buttons.IndexOf(focused));
    }

    private void RestoreFocus((string Name, int Index) key, bool bringIntoView)
    {
        var buttons = Buttons(_root).ToList();
        int at = ManagedFocus.Find(buttons.Select(ButtonName).ToList(), key);
        if (at < 0) return;
        Button match = buttons[at];
        // Its section is brought into view too: on the box (2026-11-19) the
        // page was left scrolled to the view's foot after the launch dialog
        // closed, with the focused Start button and its card out of sight.
        UIElement section = match;
        while (Microsoft.UI.Xaml.Media.VisualTreeHelper.GetParent(section) is UIElement parent && parent != _root) section = parent;
        DispatcherQueue?.TryEnqueue(() =>
        {
            match.Focus(FocusState.Programmatic);
            if (bringIntoView) section.StartBringIntoView();
        });
    }

    private void RenderContent()
    {
        if (_snapshot is null) return;
        if (_error.Parent is Panel previousCard) previousCard.Children.Remove(_error);
        _root.Children.Clear();
        RenderSignIn();
        var sessions = new StackPanel { Spacing = 6 };
        sessions.Children.Add(SectionTitle("title"));
        // Signed in, the card above holds the Install links (box,
        // 2026-11-19: both drew them); signed out, this block stands in for
        // the Mac's setup row (ManagedSessionsView.swift:34-36).
        if (_snapshot.InstallTools.Length > 0 && !_snapshot.NearAiSignedIn)
        {
            // One bare refresh glyph beside the heading; no per-tool copies.
            var heading = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
            heading.Children.Add(SectionTitle("tools_title"));
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
        RenderRedTeam();
        sessions.Children.Add(SessionCaption(_snapshot.Capabilities.SessionDestination is string destination
            ? T("terminal_scope").Replace("{destination}", destination) : T("terminal_unavailable")));
        var rows = new StackPanel { Spacing = 8 };
        sessions.Children.Add(rows);
        _root.Children.Add(new Border
        {
            Child = sessions, Padding = new Thickness(12), BorderThickness = new Thickness(1),
            CornerRadius = (CornerRadius)Application.Current.Resources["TcCardCornerRadius"],
            Background = ThemeBrushes.Get("TcSurfaceCardBrush"), BorderBrush = ThemeBrushes.Get("TcHairlineBrush"),
        });
        foreach (var session in _snapshot.Sessions)
        {
            rows.Children.Add(SessionRow(session, _snapshot.Capabilities));
        }
        if (_snapshot.Sessions.Length == 0) rows.Children.Add(new TextBlock { Text = T("empty") });
        sessions.Children.Add(_error);
    }

    private TextBlock SectionTitle(string key)
    {
        var title = new TextBlock
        {
            Text = T(key),
            Style = (Style)Application.Current.Resources["TcTitleCardTextStyle"],
            TextWrapping = TextWrapping.Wrap,
            VerticalAlignment = VerticalAlignment.Center,
        };
        AutomationProperties.SetHeadingLevel(title, AutomationHeadingLevel.Level2);
        return title;
    }

    /// <summary>The copy key for the last setup result, shown inside the card.</summary>
    private string? _redTeamOutcomeKey;

    /// <summary>
    /// Red Team's setup card:
    /// the name, whether its runtime is installed, and its line, with Install
    /// while it is not. On Windows the daemon also says why the host cannot run
    /// it: Windows Home is explained and offered nothing; otherwise Enable Red
    /// Team stands under the line that says what it does, so the person reads
    /// that before the Windows prompt. If the account needs group access, the
    /// same section explains the manual setup and when to sign out.
    /// </summary>
    private void RenderRedTeam()
    {
        if (_snapshot?.RedTeamState is not RedTeamSetup setup) return;
        if (setup == RedTeamSetup.Installed) _redTeamOutcomeKey = null;
        var section = new StackPanel { Spacing = 8 };
        var card = new Border
        {
            Child = section,
            Padding = new Thickness(12),
            BorderThickness = new Thickness(1),
            CornerRadius = (CornerRadius)Application.Current.Resources["TcCardCornerRadius"],
            Background = ThemeBrushes.Get("TcSurfaceCardBrush"),
            BorderBrush = ThemeBrushes.Get("TcHairlineBrush"),
        };
        section.Children.Add(SectionTitle("red_team"));
        section.Children.Add(new TextBlock { Text = T(setup == RedTeamSetup.Installed ? "red_team_installed" : "red_team_not_installed") });
        section.Children.Add(new TextBlock { Text = T("red_team_settings_line"), TextWrapping = TextWrapping.Wrap });
        if (_snapshot.RedTeamUnsupportedEdition)
        {
            section.Children.Add(new TextBlock { Text = T("red_team_home_unsupported"), TextWrapping = TextWrapping.Wrap });
        }
        else if (_snapshot.OffersRedTeamEnable || _snapshot.RedTeamNeedsHyperVGroup)
        {
            if (_snapshot.OffersRedTeamEnable)
            {
                string groupExplain = _snapshot.RedTeamNeedsHyperVGroup ? T("red_team_group_enable_explain") : string.Empty;
                section.Children.Add(new TextBlock
                {
                    Text = groupExplain.Length > 0 ? groupExplain : T("red_team_enable_explain"),
                    TextWrapping = TextWrapping.Wrap,
                });
                var enable = Button("red_team_enable_action", () => ChangeRedTeamSetupAsync("enable"));
                AutomationProperties.SetName(enable, T("red_team_enable_action"));
                section.Children.Add(enable);
            }
            if (_snapshot.RedTeamNeedsHyperVGroup && T("red_team_group_manual_setup") is { Length: > 0 } manualSetup)
            {
                section.Children.Add(new TextBlock { Text = manualSetup, TextWrapping = TextWrapping.Wrap });
            }
        }
        else if (setup == RedTeamSetup.NotInstalled)
        {
            var install = Button("install", () => ChangeRedTeamSetupAsync("install"));
            AutomationProperties.SetName(install, $"{T("install")} {T("red_team")}");
            section.Children.Add(install);
        }
        if (_redTeamOutcomeKey is string key)
        {
            var outcome = new TextBlock { Text = T(key), TextWrapping = TextWrapping.Wrap };
            AutomationProperties.SetLiveSetting(outcome, AutomationLiveSetting.Polite);
            section.Children.Add(outcome);
        }
        if (setup == RedTeamSetup.NotInstalled)
        {
            var preferences = ShellPreferences.Shared;
            var reopen = new HyperlinkButton { Content = T("red_team_setup_title") };
            var dismiss = new HyperlinkButton { Content = T("dismiss") };
            AutomationProperties.SetHelpText(dismiss, T("red_team_setup_title"));

            void ApplyDismissal()
            {
                card.Visibility = preferences.RedTeamSetupDismissed ? Visibility.Collapsed : Visibility.Visible;
                reopen.Visibility = preferences.RedTeamSetupDismissed ? Visibility.Visible : Visibility.Collapsed;
            }

            dismiss.Click += (_, _) =>
            {
                preferences.RedTeamSetupDismissed = true;
                ApplyDismissal();
                reopen.Focus(FocusState.Programmatic);
            };
            reopen.Click += (_, _) =>
            {
                preferences.RedTeamSetupDismissed = false;
                ApplyDismissal();
                dismiss.Focus(FocusState.Programmatic);
            };
            section.Children.Add(dismiss);
            _root.Children.Add(reopen);
            ApplyDismissal();
        }
        _root.Children.Add(card);
    }

    /// <summary>
    /// Run the requested setup action and keep its result in the setup card.
    /// </summary>
    private async Task ChangeRedTeamSetupAsync(string action)
    {
        _redTeamOutcomeKey = null;
        try
        {
            var result = await CallAsync("managed_tool_setup", new { tool = "red_team", action }, trackUnsupportedVersion: false);
            if (action == "enable")
            {
                _redTeamOutcomeKey = ManagedSignIn.EnableOutcomeKey(
                    result.TryGetProperty("outcome", out var outcome) ? outcome.GetString() : null);
            }
        }
        catch (Refused refused)
        {
            _redTeamOutcomeKey = ManagedSignIn.ErrorKey(refused.Code);
        }
        catch (Exception)
        {
            _redTeamOutcomeKey = "action_failed";
        }
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
        section.Children.Add(SectionTitle("sign_in_title"));
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

        var actions = new FlowPanel { Spacing = 8, LineSpacing = 8 };
        if (_snapshot.ToolArea == ManagedToolArea.Buttons)
        {
            foreach (string tool in _snapshot.SignInStartTools)
            {
                if (ManagedSignIn.StartKey(tool) is not string key) continue;
                actions.Children.Add(ToolButton(tool, key));
            }

            if (actions.Children.Count == 4)
            {
                // Keep OpenCode and Red Team together when all four buttons need two rows.
                actions.PreferredRowBreak = 2;
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
            Background = ThemeBrushes.Get("TcTileFolderBrush"),
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
        ShowFolder();
        Grid.SetColumn(tile, 0); Grid.SetColumn(shown, 1); Grid.SetColumn(clear, 2); Grid.SetColumn(pick, 3);
        folderRow.Children.Add(tile); folderRow.Children.Add(shown); folderRow.Children.Add(clear); folderRow.Children.Add(pick);
        form.Children.Add(folderRow);
        // Asked on every launch while more than one terminal is installed (#1891).
        var terminal = _snapshot.AsksForTerminal ? TerminalPicker() : null;
        if (terminal is not null) form.Children.Add(terminal);
        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot, Title = T(startKey), Content = new ScrollViewer { Content = form },
            PrimaryButtonText = ManagedSignIn.OpenLabel(T("open_tool"), T(tool)),
            CloseButtonText = T("cancel"), DefaultButton = ContentDialogButton.Primary,
        };
        var fields = new LaunchFields(this, _snapshot.Capabilities);
        fields.Changed += Validate;
        void Validate() => dialog.IsPrimaryButtonEnabled = fields.Ready;
        form.Children.Add(fields.Panel);
        Task LoadAsync() => fields.LoadAsync(tool, "near_ai_signin", path);
        clear.Click += async (_, _) => { path = null; ShowFolder(); await LoadAsync(); };
        pick.Click += async (_, _) =>
        {
            if (await PickFolderAsync() is string chosen && ManagedLaunchFolder.IsAcceptable(chosen))
            {
                path = chosen;
                ShowFolder();
                await LoadAsync();
            }
        };
        _ = LoadAsync();
        // Open is the default action (ManagedFolderExplainer.swift:62,
        // isDefault): focus starts on it, so Enter opens the tool rather than
        // the folder picker.
        // Until the person first presses a key or the pointer, focus that
        // lands anywhere else goes back to Open: on the box (2026-11-19) a
        // second opening in a row had the dialog move focus to the first
        // button of its content after Opened.
        bool settling = true;
        dialog.PreviewKeyDown += (_, _) => settling = false;
        dialog.AddHandler(UIElement.PointerPressedEvent, new PointerEventHandler((_, _) => settling = false), true);
        dialog.GotFocus += (_, e) =>
        {
            if (settling && e.OriginalSource is not Button { Name: "PrimaryButton" }) FocusPrimary(dialog);
        };
        dialog.Opened += (_, _) => FocusPrimary(dialog);
        ContentDialogResult result = await dialog.ShowAsync();
        art.Release();
        var choices = fields.Choices;
        fields.Close();
        if (result != ContentDialogResult.Primary) return;
        // Decided from the folder and the snapshot at send time (L4).
        if (_snapshot?.SignInLaunchParams(tool, path, Guid.NewGuid().ToString(), choices) is not { } request) return;
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
        var name = new TextBox { Text = account.Label, Header = T("label"), PlaceholderText = T("label_placeholder") };
        if (await DialogGuard.ShowOnceAsync(Dialog("rename", name, "save")) == ContentDialogResult.Primary)
            await CallAsync("managed_account_rename", new { account_id = account.Id, label = name.Text });
    }

    private async Task AddAsync()
    {
        string[] tools = _snapshot?.SetupTools ?? [];
        if (tools.Length == 0) return;
        bool signedIn = _snapshot?.NearAiSignedIn == true;
        var tool = new ComboBox { Header = T("tool"), ItemsSource = tools.Select(T).ToArray(), SelectedIndex = 0 };
        var connection = new ComboBox { Header = T("connection") };
        var label = new TextBox { Header = T("label"), PlaceholderText = T("label_placeholder") };
        var key = new PasswordBox { Header = T("api_key") };
        var loginDescription = SessionCaption(T("login_description"));
        var storageDescription = SessionCaption(T("key_storage"));
        var form = new StackPanel { Spacing = 10 };
        form.Children.Add(tool);
        form.Children.Add(connection);
        form.Children.Add(label);
        form.Children.Add(key);
        form.Children.Add(loginDescription);
        form.Children.Add(storageDescription);
        var dialog = Dialog("save_title", form, "save_account");
        string[] connections = [];
        string? ChosenConnection() => connection.SelectedIndex >= 0 && connection.SelectedIndex < connections.Length
            ? connections[connection.SelectedIndex] : null;
        void ValidateAccount()
        {
            string? kind = ChosenConnection();
            bool subscription = kind == "subscription";
            key.Visibility = storageDescription.Visibility = subscription ? Visibility.Collapsed : Visibility.Visible;
            loginDescription.Visibility = subscription ? Visibility.Visible : Visibility.Collapsed;
            dialog.PrimaryButtonText = T(subscription ? "save_login" : "save_account");
            dialog.IsPrimaryButtonEnabled = kind is not null && !string.IsNullOrWhiteSpace(label.Text)
                && (subscription ? _snapshot?.Capabilities.TerminalLaunch == true : !string.IsNullOrEmpty(key.Password));
        }
        void UpdateConnections()
        {
            if (tool.SelectedIndex < 0) return;
            string selectedTool = tools[tool.SelectedIndex];
            string? previous = ChosenConnection();
            connections = ManagedSnapshot.Connections(selectedTool, signedIn);
            connection.ItemsSource = connections.Select(kind => T(selectedTool == "opencode" && kind == "api_key" ? "openai_api_key" : kind)).ToArray();
            connection.SelectedIndex = Math.Max(0, Array.IndexOf(connections, previous));
            ValidateAccount();
        }
        tool.SelectionChanged += (_, _) => UpdateConnections();
        label.TextChanged += (_, _) => ValidateAccount();
        key.PasswordChanged += (_, _) => ValidateAccount();
        connection.SelectionChanged += (_, _) => ValidateAccount();
        UpdateConnections();

        try
        {
            if (await DialogGuard.ShowOnceAsync(dialog) != ContentDialogResult.Primary
                || ChosenConnection() is not { } kind || tool.SelectedIndex < 0) return;
            if (kind == "subscription" && _snapshot?.Capabilities.TerminalLaunch != true) throw new InvalidOperationException();
            var account = await CallAsync("managed_account_add", new { tool = tools[tool.SelectedIndex], connection = kind, label = label.Text });
            string id = account.GetProperty("id").GetString()!;
            if (kind == "subscription") await OpenAsync(await CallAsync("managed_account_reconnect", new { account_id = id }));
            else await CallAsync("managed_account_set_key", new { account_id = id, key = key.Password });
        }
        finally { key.Password = string.Empty; }
    }

    private async Task LaunchAsync()
    {
        if (_snapshot is not { OffersSavedAccountLaunch: true } snapshot) return;
        var accounts = snapshot.Accounts.Where(a => snapshot.CanLaunch(a.Tool)).ToArray();
        if (accounts.Length == 0) return;
        var account = new ComboBox
        {
            Header = T("saved_account"), PlaceholderText = T("choose_account"),
            ItemsSource = accounts.Select(a => $"{T(a.Tool)} · {a.Label} · {T(a.Connection)}").ToArray(),
        };
        var project = new TextBox { PlaceholderText = T("home_folder") };
        AutomationProperties.SetName(project, T("pick_folder"));
        string? Folder() => project.Text.Length == 0 ? null : project.Text;
        bool ValidFolder() => Folder() is not string folder || ManagedLaunchFolder.IsAcceptable(folder);
        var form = new StackPanel { Spacing = 10 };
        form.Children.Add(account);
        form.Children.Add(project);
        var choose = new Button { Content = T("pick_folder") };
        var clear = new Button { Content = T("clear_folder"), Visibility = Visibility.Collapsed };
        choose.Click += async (_, _) =>
        {
            if (await PickFolderAsync() is string path && ManagedLaunchFolder.IsAcceptable(path)) project.Text = path;
        };
        clear.Click += (_, _) => project.Text = string.Empty;
        var projectActions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        projectActions.Children.Add(choose);
        projectActions.Children.Add(clear);
        form.Children.Add(projectActions);
        var terminalPicker = snapshot.AsksForTerminal ? TerminalPicker() : null;
        if (terminalPicker is not null) form.Children.Add(terminalPicker);
        var destinationLine = SessionCaption(string.Empty);
        void ShowDestination() => destinationLine.Text = T("launch_scope").Replace("{destination}",
            terminalPicker?.SelectedItem as string ?? snapshot.Capabilities.SessionDestination ?? T("terminal"));
        if (terminalPicker is not null) terminalPicker.SelectionChanged += (_, _) => ShowDestination();
        ShowDestination();
        form.Children.Add(destinationLine);
        var launchDialog = Dialog("launch_title", new ScrollViewer { Content = form }, "launch_short");
        var fields = new LaunchFields(this, snapshot.Capabilities);
        fields.Changed += ValidateLaunch;
        form.Children.Insert(1, fields.Panel);
        void ValidateLaunch()
        {
            launchDialog.IsPrimaryButtonEnabled = fields.Ready && account.SelectedIndex >= 0
                && ValidFolder()
                && (ManagedLaunchOptions.ModelMode(accounts[account.SelectedIndex].Tool, accounts[account.SelectedIndex].Connection)
                    != ManagedModelMode.Text || !string.IsNullOrWhiteSpace(fields.Choices?.Model));
        }
        async Task LoadAsync(bool typing = false)
        {
            ValidateLaunch();
            if (account.SelectedIndex < 0 || !ValidFolder())
            {
                fields.Close();
                fields.Panel.Children.Clear();
                return;
            }
            var selected = accounts[account.SelectedIndex];
            await fields.LoadAsync(selected.Tool, selected.Connection, Folder(), waitForTyping: typing);
        }
        account.SelectionChanged += async (_, _) => await LoadAsync();
        project.TextChanged += async (_, _) =>
        {
            clear.Visibility = Folder() is null ? Visibility.Collapsed : Visibility.Visible;
            await LoadAsync(typing: true);
        };
        ValidateLaunch();
        var result = await DialogGuard.ShowOnceAsync(launchDialog);
        var choices = fields.Choices;
        fields.Close();
        if (result != ContentDialogResult.Primary || account.SelectedIndex < 0 || !ValidFolder() || _snapshot is not { } current) return;
        string selectedId = accounts[account.SelectedIndex].Id;
        ManagedAccount? selected = current.Accounts.FirstOrDefault(saved => saved.Id == selectedId);
        if (selected is null || !current.CanLaunch(selected.Tool) || !current.Capabilities.TerminalLaunch)
            throw new InvalidOperationException();
        if (current.AccountLaunchParams(selected, Folder(), null, false, Guid.NewGuid().ToString(), choices) is not { } request) return;
        if (terminalPicker is not null && snapshot.TerminalToSave(terminalPicker.SelectedIndex) is { } terminal)
            await CallAsync("set_managed_terminal", new { terminal });
        await OpenAsync(await CallAsync("managed_launch_prepare", request));
    }
}
