using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;
using TraceCommons.Interop.Map;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Controls.Settings;

public sealed class ProjectsSettingsSection : UserControl
{
    private readonly DaemonHost _host;
    private readonly ProjectsSettingsFlow _flow;
    private readonly Func<Task>? _refreshAudit;
    private IReadOnlyList<ProjectSetting>? _projects;
    private bool _readFailed;
    private ProjectManager? _manager;
    private string _error = "";
    private Button? _manageButton;
    private readonly StackPanel _content = new() { Spacing = 12 };
    private readonly ContentControl _summary = new() { IsTabStop = false, HorizontalContentAlignment = HorizontalAlignment.Stretch };

    public ProjectsSettingsSection(DaemonHost host, Func<Task>? refreshAudit = null)
    {
        IsTabStop = false;
        _host = host;
        _flow = new ProjectsSettingsFlow((method, payload) => host.CallAsync(method, payload ?? "{}"));
        _refreshAudit = refreshAudit;
        _content.Children.Add(new ProjectsSharingChoiceSection(host, async () =>
        {
            await ReadAsync();
            if (_refreshAudit is { } refresh)
            {
                await refresh();
            }
        }));
        _content.Children.Add(_summary);
        Content = _content;
        Loaded += async (_, _) =>
        {
            host.Account.Changed -= OnAccountChanged;
            host.Account.Changed += OnAccountChanged;
            await ReadAsync();
        };
        Unloaded += (_, _) =>
        {
            host.Account.Changed -= OnAccountChanged;
            _manager?.Cancel();
        };
    }

    private string Title => SettingsSections.NavName(SettingsSection.Projects, SettingsNavWordsLive.Value) ?? "";

    private async Task ReadAsync()
    {
        long account = _host.Account.Current;
        _readFailed = false;
        Draw();
        IReadOnlyList<ProjectSetting>? projects = await _flow.ReadAsync();
        if (!_host.Account.IsCurrent(account))
        {
            return;
        }
        _readFailed = projects is null;
        _projects = projects ?? _projects;
        Draw();
    }

    private async void OnAccountChanged()
    {
        _manager?.Cancel();
        _projects = null;
        _error = "";
        await ReadAsync();
    }

    private void Draw()
    {
        var body = new StackPanel { Spacing = 8 };
        _manageButton = null;
        if (_projects is null)
        {
            body.Children.Add(SettingsCards.ReadNotice(_readFailed, async (_, _) => await ReadAsync()));
        }
        else if (_projects.Count == 0)
        {
            body.Children.Add(SettingsCards.SecondaryCaption(ProjectsSettingsWords.Empty));
        }
        else
        {
            string[] summaryModes = [MapModes.AutoUpload, MapModes.Ask, MapModes.Ignore];
            string summary = string.Join("   ", summaryModes
                .Select(mode => Fill("mode_count", WatchCopy.ModeLabel(mode), _projects.Count(row => ProjectsSettingsFlow.Mode(row) == mode))));
            body.Children.Add(SettingsCards.SecondaryCaption(summary));
            if (Text("manage") is { Length: > 0 } manage)
            {
                _manageButton = SettingsCards.Button(manage, async (_, _) => await ManageAsync());
                body.Children.Add(_manageButton);
            }
        }

        if (_error.Length > 0)
        {
            body.Children.Add(SettingsCards.Caption(_error));
        }

        _summary.Content = SettingsCards.Card(Title, body);
    }

    private async Task ManageAsync()
    {
        if (_manager is not null || XamlRoot is null || _projects is null)
        {
            return;
        }

        _manager = new ProjectManager(this);
        try
        {
            await _manager.ShowAsync();
        }
        finally
        {
            _manager = null;
            _manageButton?.Focus(FocusState.Programmatic);
        }
    }

    private static string Text(string key) => ProjectsSettingsWords.Text(key);

    private static string Fill(string key, string title = "", int count = 0, int total = 0) => Text(key)
        .Replace("{title}", title, StringComparison.Ordinal)
        .Replace("{count}", count.ToString(CultureInfo.CurrentCulture), StringComparison.Ordinal)
        .Replace("{total}", total.ToString(CultureInfo.CurrentCulture), StringComparison.Ordinal);

    private sealed class ProjectManager
    {
        private readonly ProjectsSettingsSection _section;
        private readonly long _account;
        private readonly ContentDialog _dialog;
        private readonly TextBox _search = new() { Header = Text("search"), PlaceholderText = Text("search_prompt") };
        private readonly ComboBox _filter = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
        private readonly ComboBox _bulk = new() { PlaceholderText = Text("set_to") };
        private readonly CheckBox _selectAll = new() { Content = Text("select_all") };
        private readonly TextBlock _selected = SettingsCards.SecondaryCaption("");
        private readonly TextBlock _notice = SettingsCards.Caption("");
        private readonly StackPanel _rows = new() { Spacing = 4 };
        private readonly StackPanel _manager = new() { Spacing = 8 };
        private readonly HashSet<string> _selection = new(StringComparer.Ordinal);
        private readonly HashSet<string> _expanded = new(StringComparer.Ordinal);
        private readonly List<(CheckBox Box, string[] Ids)> _checkBoxes = new();
        private IReadOnlyList<ProjectSetting> _visible = Array.Empty<ProjectSetting>();
        private string[] _bulkModes = Array.Empty<string>();
        private bool _busy;
        private bool _closed;
        private bool _shown;
        private TaskCompletionSource<bool>? _confirmation;

        public ProjectManager(ProjectsSettingsSection section)
        {
            _section = section;
            _account = section._host.Account.Current;
            _filter.Items.Add(new ComboBoxItem { Content = Text("all") });
            foreach (string mode in ProjectsSettingsFlow.Modes)
            {
                _filter.Items.Add(ModeItem(mode));
            }

            _filter.SelectedIndex = 0;
            _search.TextChanged += (_, _) => DrawRows();
            _filter.SelectionChanged += (_, _) => DrawRows();
            _selectAll.Click += (_, _) => Select(_visible.Select(row => row.ProjectId), _selectAll.IsChecked == true);
            _bulk.SelectionChanged += async (_, _) =>
            {
                if (_bulk.SelectedItem is ComboBoxItem { Tag: string mode })
                {
                    await ChangeAsync(ProjectsSettingsFlow.SelectedIds(_visible, _selection), mode);
                    _bulk.SelectedIndex = -1;
                }
            };

            _manager.Children.Add(_search);
            _manager.Children.Add(_filter);
            _manager.Children.Add(SettingsCards.Row(_selectAll, _selected, _bulk));
            _manager.Children.Add(new ScrollViewer { Content = _rows, Height = 280, VerticalScrollBarVisibility = ScrollBarVisibility.Auto });
            _manager.Children.Add(_notice);
            _dialog = new ContentDialog
            {
                XamlRoot = section.XamlRoot,
                Title = section.Title,
                Content = _manager,
                CloseButtonText = MonitorWords.Close,
            };
            _dialog.Opened += async (_, _) =>
            {
                _shown = true;
                if (_closed)
                {
                    _dialog.Hide();
                    return;
                }
                _search.Focus(FocusState.Programmatic);
                var plan = await _section._flow.PlanAsync(Array.Empty<string>(), MapModes.Ask);
                if (!_closed)
                {
                    _bulkModes = plan?.BulkModes?.Where(mode => mode is MapModes.Ask or MapModes.Ignore).ToArray() ?? Array.Empty<string>();
                    foreach (string mode in _bulkModes)
                    {
                        _bulk.Items.Add(ModeItem(mode));
                    }
                    DrawRows();
                }
            };
            _dialog.Closed += (_, _) =>
            {
                _shown = false;
                _closed = true;
                _confirmation?.TrySetResult(false);
            };
        }

        public void Cancel()
        {
            _closed = true;
            if (_shown)
            {
                _dialog.Hide();
            }
        }

        public Task ShowAsync()
        {
            DrawRows();
            return DialogGuard.ShowOnceAsync(_dialog);
        }

        private static ComboBoxItem ModeItem(string mode) => new() { Content = WatchCopy.ModeLabel(mode), Tag = mode };

        private void DrawRows()
        {
            if (_closed)
            {
                return;
            }

            string search = _search.Text.Trim();
            string? filter = (_filter.SelectedItem as ComboBoxItem)?.Tag as string;
            var projects = _section._projects ?? Array.Empty<ProjectSetting>();
            _visible = projects.Where(row =>
                (filter is null || ProjectsSettingsFlow.Mode(row) == filter)
                && ProjectsSettingsFlow.Label(row).Contains(search, StringComparison.CurrentCultureIgnoreCase)).ToArray();
            _checkBoxes.Clear();
            _rows.Children.Clear();
            if (_visible.Count == 0)
            {
                _rows.Children.Add(SettingsCards.SecondaryCaption(Text("no_matches")));
            }

            // The daemon supplies the grouping identity. Labels can collide independently.
            var groups = _visible.Where(row => row.Collision is not null)
                .GroupBy(row => row.Collision!.Group).ToDictionary(group => group.Key, group => group.ToArray());
            var drawn = new HashSet<string>(StringComparer.Ordinal);
            foreach (ProjectSetting row in _visible)
            {
                if (row.Collision is not { } collision || groups[collision.Group].Length == 1)
                {
                    _rows.Children.Add(ProjectRow(row, ProjectsSettingsFlow.Label(row)));
                }
                else if (drawn.Add(collision.Group))
                {
                    _rows.Children.Add(GroupRow(collision, groups[collision.Group], search.Length > 0));
                }
            }

            _search.IsEnabled = !_busy;
            _filter.IsEnabled = !_busy;
            _notice.Text = _section._error;
            _notice.Visibility = _section._error.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
            UpdateSelection();
        }

        private UIElement GroupRow(ProjectLabelCollision collision, ProjectSetting[] projects, bool searching)
        {
            string[] ids = projects.Select(row => row.ProjectId).ToArray();
            CheckBox select = SelectionBox(collision.BaseLabel, Fill("select_group_accessibility", collision.BaseLabel), ids);
            string[] modes = projects.Select(ProjectsSettingsFlow.Mode).Distinct().ToArray();
            string? common = modes.Length == 1 ? modes[0] : null;
            var change = new ComboBox
            {
                PlaceholderText = common is null ? MonitorShellCopy.Top("mixed") : WatchCopy.ModeLabel(common),
                IsEnabled = !_busy && _bulkModes.Length > 0,
            };
            AutomationProperties.SetName(change, Fill("group_menu_accessibility", collision.BaseLabel));
            foreach (string mode in _bulkModes)
            {
                change.Items.Add(ModeItem(mode));
            }
            change.SelectionChanged += async (_, _) =>
            {
                if (change.SelectedItem is ComboBoxItem { Tag: string mode })
                {
                    await ChangeAsync(ids, mode);
                }
            };

            var children = new StackPanel { Spacing = 4 };
            foreach (ProjectSetting row in projects)
            {
                children.Children.Add(ProjectRow(row, row.Collision?.Suffix ?? ProjectsSettingsFlow.Label(row)));
            }
            var expander = new Expander
            {
                Header = ProjectActions(select, change),
                Content = children,
                IsExpanded = searching || _expanded.Contains(collision.Group),
                HorizontalAlignment = HorizontalAlignment.Stretch,
            };
            AutomationProperties.SetName(expander, Fill("group_accessibility", collision.BaseLabel, projects.Length));
            expander.Expanding += (_, _) => _expanded.Add(collision.Group);
            expander.Collapsed += (_, _) => _expanded.Remove(collision.Group);
            return expander;
        }

        private UIElement ProjectRow(ProjectSetting project, string label)
        {
            string title = ProjectsSettingsFlow.Label(project);
            CheckBox select = SelectionBox(label, Fill("select_accessibility", title), [project.ProjectId]);
            string current = ProjectsSettingsFlow.Mode(project);
            var change = new ComboBox { PlaceholderText = WatchCopy.ModeLabel(current), IsEnabled = !_busy };
            AutomationProperties.SetName(change, title);
            foreach (string mode in ProjectsSettingsFlow.Options(project))
            {
                ComboBoxItem option = ModeItem(mode);
                change.Items.Add(option);
                if (mode == current)
                {
                    change.SelectedItem = option;
                }
            }
            change.SelectionChanged += async (_, _) =>
            {
                if (change.SelectedItem is ComboBoxItem { Tag: string mode } && mode != current)
                {
                    await ChangeAsync([project.ProjectId], mode, project);
                }
            };

            var body = new StackPanel { Spacing = 4 };
            body.Children.Add(ProjectActions(select, change));
            if (project.IsUnresolvedBucket)
            {
                body.Children.Add(SettingsCards.SecondaryCaption(UnresolvedBucketCopy.Note));
            }
            return body;
        }

        private static Grid ProjectActions(CheckBox select, ComboBox modes)
        {
            var row = new Grid { ColumnSpacing = 8 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.Children.Add(select);
            Grid.SetColumn(modes, 1);
            row.Children.Add(modes);
            return row;
        }

        private CheckBox SelectionBox(string label, string accessibleName, string[] ids)
        {
            var box = new CheckBox { Content = SettingsCards.Text(label), IsEnabled = !_busy };
            AutomationProperties.SetName(box, accessibleName);
            box.Click += (_, _) => Select(ids, box.IsChecked == true);
            _checkBoxes.Add((box, ids));
            return box;
        }

        private void Select(IEnumerable<string> ids, bool selected)
        {
            if (selected)
            {
                _selection.UnionWith(ids);
            }
            else
            {
                _selection.ExceptWith(ids);
            }
            UpdateSelection();
        }

        private void UpdateSelection()
        {
            foreach ((CheckBox box, string[] ids) in _checkBoxes)
            {
                box.IsChecked = CheckState(ids);
            }

            _selectAll.IsChecked = CheckState(_visible.Select(row => row.ProjectId));
            _selectAll.IsEnabled = !_busy && _visible.Count > 0;
            int selected = ProjectsSettingsFlow.SelectedIds(_visible, _selection).Length;
            _bulk.IsEnabled = !_busy && _bulkModes.Length > 0 && selected > 0;
            _selected.Text = Fill("selected", count: selected);
            AutomationProperties.SetName(_bulk, Fill("bulk_menu_accessibility", count: selected));
        }

        private bool? CheckState(IEnumerable<string> ids)
        {
            int count = ids.Count();
            int selected = ids.Count(_selection.Contains);
            return selected == 0 ? false : selected == count ? true : null;
        }

        private async Task ChangeAsync(string[] ids, string mode, ProjectSetting? single = null)
        {
            if (_busy || _closed || ids.Length == 0)
            {
                return;
            }

            _busy = true;
            _section._error = "";
            DrawRows();
            try
            {
                bool attempted;
                if (single is not null && mode != MapModes.Ignore)
                {
                    var write = await _section._flow.SetProjectAsync(single, mode);
                    if (!_section._host.Account.IsCurrent(_account))
                    {
                        return;
                    }
                    if (!write.Saved)
                    {
                        _section._error = MonitorWords.ProjectModeFailed;
                    }
                    _section._projects = write.Projects ?? _section._projects;
                    _section.Draw();
                    attempted = true;
                }
                else
                {
                    attempted = await ChangeManyAsync(ids, mode);
                    if (attempted)
                    {
                        await _section.ReadAsync();
                    }
                }

                if (attempted && _section._refreshAudit is { } refresh)
                {
                    await refresh();
                }
            }
            finally
            {
                _busy = false;
                DrawRows();
            }
        }

        private async Task<bool> ChangeManyAsync(string[] ids, string mode)
        {
            bool attempted = false;
            while (!_closed)
            {
                ProjectsBulkPlan? plan = await _section._flow.PlanAsync(ids, mode);
                if (_closed)
                {
                    return attempted;
                }
                if (plan is null || plan.Outcome == "refused")
                {
                    _section._error = MonitorWords.ProjectModeFailed;
                    return attempted;
                }

                if (plan.Outcome == "nothing" || _closed)
                {
                    return attempted;
                }

                ProjectsBulkPlan? agreed = null;
                if (plan.Outcome == "confirm")
                {
                    if (plan.Confirmation is not { } confirmation || !await ConfirmAsync(confirmation))
                    {
                        return attempted;
                    }
                    agreed = plan;
                }
                else if (plan.Outcome != "apply")
                {
                    return attempted;
                }

                attempted = true;
                var applied = await _section._flow.ApplyAsync(ids, mode, agreed);
                if (!_section._host.Account.IsCurrent(_account))
                {
                    return false;
                }
                if (applied.Stale)
                {
                    continue;
                }
                if (applied.Result is { } result)
                {
                    _selection.ExceptWith(result.Changed);
                    if (result.Changed.Length < result.Total)
                    {
                        _section._error = Fill("changed_some", count: result.Changed.Length, total: result.Total);
                    }
                }
                else
                {
                    _section._error = MonitorWords.ProjectModeFailed;
                }
                return attempted;
            }
            return attempted;
        }

        private async Task<bool> ConfirmAsync(ProjectsBulkConfirmation copy)
        {
            var result = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
            _confirmation = result;
            var content = new StackPanel { Spacing = 12 };
            content.Children.Add(SettingsCards.Text(copy.Body));
            Button keep = SettingsCards.Button(copy.Keep, (_, _) => result.TrySetResult(false));
            Button confirm = SettingsCards.Button(copy.Button, (_, _) => result.TrySetResult(true), primary: !copy.Destructive);
            ToolTipService.SetToolTip(confirm, copy.Help);
            content.Children.Add(SettingsCards.Row(keep, confirm));
            _dialog.Title = copy.Title;
            _dialog.Content = content;
            keep.Focus(FocusState.Programmatic);
            bool allowed = await result.Task;
            _confirmation = null;
            if (!_closed)
            {
                _dialog.Title = _section.Title;
                _dialog.Content = _manager;
            }
            return allowed && !_closed;
        }
    }
}
