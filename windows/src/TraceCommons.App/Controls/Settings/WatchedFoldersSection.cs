using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;
using TraceCommons.Interop.FirstRun;
using TraceCommons.Interop.Map;
using Windows.Storage;
using Windows.Storage.Pickers;

namespace TraceCommons.App.Controls.Settings;

public sealed class WatchedFoldersSection : UserControl
{
    private readonly WatchedFoldersFlow _flow;
    private readonly Func<nint> _ownerHandle;
    private IReadOnlyList<SourceCandidate> _candidates = Array.Empty<SourceCandidate>();
    private SourceChecks.SettingsCopy? _copy;
    private bool _picking;

    public WatchedFoldersSection(DaemonHost host, Func<nint> ownerHandle)
    {
        IsTabStop = false;
        _flow = new WatchedFoldersFlow((method, payload) => host.CallAsync(method, payload ?? "{}"));
        _ownerHandle = ownerHandle;
        Loaded += OnLoaded;
    }

    private async void OnLoaded(object sender, RoutedEventArgs args)
    {
        _copy ??= SourceChecks.ReadSettingsCopy();
        if (_copy is null)
        {
            return;
        }

        if (_candidates.Count == 0 && !Program.IsSampleLaunch)
        {
            _candidates = await Task.Run(SourceDiscovery.ProbeThisMachine);
        }

        await RunAsync(_flow.ReadAsync);
    }

    private void Render(string? focus)
    {
        if (_copy is not { } copy)
        {
            return;
        }

        var rows = new StackPanel { Spacing = 8 };
        rows.Children.Add(SettingsCards.SecondaryCaption(copy.Explanation));
        if (_flow.Available)
        {
            foreach ((string source, _) in FirstRunRoots.SettingsKeys)
            {
                if (copy.Tools.TryGetValue(source, out SourceChecks.ToolCopy? tool))
                {
                    rows.Children.Add(ToolRow(copy, source, tool));
                }
            }

            if (copy.Trajectory is { } trajectory && _flow.TrajectoryMode is "watch" or "off")
            {
                rows.Children.Add(TrajectoryRow(trajectory));
            }
        }
        else
        {
            rows.Children.Add(SettingsCards.Text(copy.Unavailable));
        }

        if (!_flow.Available || _flow.SaveFailed)
        {
            Button retry = SettingsCards.Key(SettingsCards.Button(copy.Retry,
                async (_, _) => await RunAsync(_flow.ReadAsync)), "watched-folders/retry");
            retry.IsEnabled = !_flow.Busy;
            rows.Children.Add(retry);
        }

        if (_flow.SaveFailed)
        {
            rows.Children.Add(SettingsCards.Caption(copy.SaveFailed));
        }

        SettingsCards.Replace(this, SettingsCards.Card(copy.Heading, rows), focus);
    }

    private UIElement ToolRow(SourceChecks.SettingsCopy copy, string source, SourceChecks.ToolCopy tool)
    {
        string name = source == "antigravity" ? "Antigravity" : MapTools.DisplayName(source);
        string mode = _flow.Mode(source) ?? "";
        bool enabled = !_flow.Busy && !_picking;
        SourceCandidate? candidate = SourceDiscovery.For(_candidates, source);
        var body = new StackPanel { Spacing = 6 };
        body.Children.Add(SettingsCards.Text(name, "TcTitleCardTextStyle"));
        if (tool.Explanation is { Length: > 0 } explanation)
        {
            body.Children.Add(SettingsCards.SecondaryCaption(explanation));
        }

        if (SourceChecks.CheckLine(tool.Key, mode) is { Length: > 0 } line)
        {
            body.Children.Add(SettingsCards.Text(line));
        }

        if (mode == "unset" && !tool.UnsetScansConventional)
        {
            if (candidate is not null)
            {
                if (candidate.Path.Length > 0)
                {
                    body.Children.Add(SettingsCards.Text(candidate.Path, "TcMonoChipTextStyle"));
                }

                if (copy.Evidence is { } evidence)
                {
                    body.Children.Add(SettingsCards.SecondaryCaption(evidence.Describe(candidate, DateTimeOffset.Now)));
                }
            }
            else if (tool.Explanation is null)
            {
                body.Children.Add(SettingsCards.SecondaryCaption(copy.NoCandidate));
            }
        }

        var actions = new StackPanel { Spacing = 8, Orientation = Orientation.Horizontal };
        if (candidate is { Exists: true, Path.Length: > 0 })
        {
            actions.Children.Add(Action(copy.WatchCandidate, "watch", enabled, async () =>
                await RunAsync(() => _flow.SaveAsync(source, ToolChoice.Watch(candidate.Path)))));
        }

        actions.Children.Add(Action(tool.ChooseFolder ?? copy.ChooseFolder, "choose", enabled, () => PickAsync(source)));
        actions.Children.Add(Action(tool.Decline, "off", enabled && mode != "off",
            () => RunAsync(() => _flow.SaveAsync(source, ToolChoice.Off))));
        body.Children.Add(actions);
        return SettingsCards.Card("", body);

        Button Action(string label, string key, bool available, Func<Task> run)
        {
            Button button = SettingsCards.Button(label, async (_, _) => await run(), copy.ActionName(label, name));
            button.IsEnabled = available;
            return SettingsCards.Key(button, $"watched-folders/{source}/{key}");
        }
    }

    private UIElement TrajectoryRow(SourceChecks.TrajectoryCopy copy)
    {
        bool watching = _flow.TrajectoryMode == "watch";
        var body = new StackPanel { Spacing = 6 };
        body.Children.Add(SettingsCards.Text(copy.Title, "TcTitleCardTextStyle"));
        body.Children.Add(SettingsCards.SecondaryCaption(copy.Explanation));
        body.Children.Add(SettingsCards.Text(watching ? copy.Watching : copy.Off));
        Button off = SettingsCards.Key(SettingsCards.Button(copy.Decline,
            async (_, _) => await RunAsync(_flow.TurnOffTrajectoryAsync)), "watched-folders/trajectory/off");
        off.IsEnabled = watching && !_flow.Busy && !_picking;
        body.Children.Add(off);
        return SettingsCards.Card("", body);
    }

    private async Task PickAsync(string source)
    {
        nint owner = _ownerHandle();
        if (_flow.Busy || _picking || owner == 0)
        {
            return;
        }

        string? focus = SettingsCards.FocusedKey(this);
        _picking = true;
        try
        {
            var picker = new FolderPicker();
            picker.FileTypeFilter.Add("*");
            WinRT.Interop.InitializeWithWindow.Initialize(picker, owner);
            StorageFolder? folder = await picker.PickSingleFolderAsync();
            if (folder?.Path is not { Length: > 0 } path || !IsLoaded)
            {
                return;
            }

            if (source == "opencode")
            {
                string? described = await Task.Run(() => ToolsAdding.Describe(path));
                path = WatchedFoldersFlow.OpenCodePath(path, described);
            }

            if (IsLoaded)
            {
                Task save = _flow.SaveAsync(source, ToolChoice.Watch(path));
                Render(null);
                await save;
            }
        }
        finally
        {
            _picking = false;
            if (IsLoaded)
            {
                Render(focus);
            }
        }
    }

    private async Task RunAsync(Func<Task> operation)
    {
        if (_picking)
        {
            return;
        }

        string? focus = SettingsCards.FocusedKey(this);
        Task pending = operation();
        Render(null);
        await pending;
        Render(focus);
    }
}
