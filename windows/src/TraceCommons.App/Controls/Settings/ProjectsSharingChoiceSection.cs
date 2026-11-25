using System;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;
using TraceCommons.Interop.FirstRun;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Controls.Settings;

internal sealed class ProjectsSharingChoiceSection : UserControl
{
    private static readonly Lazy<FirstRunCopy?> Words = new(FirstRunCopy.Load);
    private readonly DaemonHost _host;
    private readonly Func<Task> _changed;
    private readonly string _groupName = Guid.NewGuid().ToString("N");
    private AutomaticGrantState? _grant;
    private AutomaticGrantCopy? _grantCopy;
    private bool _busy;
    private string? _refusal;

    public ProjectsSharingChoiceSection(DaemonHost host, Func<Task> changed)
    {
        _host = host;
        _changed = changed;
        IsTabStop = false;
        Loaded += async (_, _) =>
        {
            host.Account.Changed -= OnAccountChanged;
            host.Account.Changed += OnAccountChanged;
            Draw();
            if (Words.Value is null)
            {
                return;
            }

            _grantCopy = AutomaticGrantCopy.Load(host.ConfigDir);
            await ReadAsync();
            Draw();
        };
        Unloaded += (_, _) => host.Account.Changed -= OnAccountChanged;
    }

    private async void OnAccountChanged()
    {
        _grant = null;
        _refusal = null;
        _grantCopy = AutomaticGrantCopy.Load(_host.ConfigDir);
        Draw();
        await ReadAsync();
        Draw();
    }

    private async Task ReadAsync()
    {
        long account = _host.Account.Current;
        DaemonResponse reply = await _host.CallAsync("automatic_grant");
        if (_host.Account.IsCurrent(account))
        {
            _grant = reply.IsError ? null : AutomaticGrantState.Decode(reply.Result);
        }
    }

    private void Draw(string? focus = null)
    {
        if (Words.Value is not { } copy)
        {
            return;
        }

        var body = new StackPanel { Spacing = 8 };
        SharingPath? current = SharingChoice.Current(_grant);
        foreach (var option in UsesSharing.Options(new FirstRunState { Account = AccountAnswer.Enrolled }, copy.Uses, WatchCopy.Armed))
        {
            var label = new StackPanel { Spacing = 2 };
            label.Children.Add(SettingsCards.Text(option.Title));
            foreach (string line in option.Lines)
            {
                label.Children.Add(SettingsCards.SecondaryCaption(line));
            }

            (string Line, string Detail)? info = _grantCopy is { } grant ? UsesSharing.Split(option.Path, grant) : null;
            if (info is { Line.Length: > 0 } wording)
            {
                label.Children.Add(SettingsCards.SecondaryCaption(wording.Line));
            }

            string key = "projects/sharing/" + option.Path;
            var radio = SettingsCards.Key(new RadioButton
            {
                GroupName = _groupName,
                Content = label,
                IsChecked = current == option.Path,
                IsEnabled = !_busy && current is not null && _grantCopy is not null,
            }, key);
            AutomationProperties.SetName(radio, option.Title);
            if (info?.Detail is { } detail)
            {
                ToolTipService.SetToolTip(radio, detail);
            }
            radio.Click += async (_, _) => await SelectAsync(option.Path, key);
            body.Children.Add(radio);
        }

        body.Children.Add(SettingsCards.SecondaryCaption(copy.Uses.SettingsSharingNote));
        if (_refusal is { Length: > 0 } refusal)
        {
            body.Children.Add(SettingsCards.Caption(refusal));
        }
        SettingsCards.Replace(this, SettingsCards.Card(copy.Uses.Sharing, body), focus);
    }

    private async Task SelectAsync(SharingPath desired, string focus)
    {
        if (_busy || XamlRoot is not { } root || Words.Value is not { } copy || _grantCopy is not { } grantCopy)
        {
            return;
        }

        _busy = true;
        _refusal = null;
        long account = _host.Account.Current;
        try
        {
            Draw();
            await ReadAsync();
            SharingPath? current = SharingChoice.Current(_grant);
            if (!_host.Account.IsCurrent(account) || current is null || current == desired)
            {
                return;
            }

            int asking = 0;
            if (desired == SharingPath.Automatic)
            {
                DaemonResponse projects = await _host.CallAsync(DaemonProtocol.Methods.ListProjects);
                if (!_host.Account.IsCurrent(account))
                {
                    return;
                }
                if (projects.IsError || projects.Result is not JsonElement result)
                {
                    _refusal = copy.Uses.SettingsSharingRefused;
                    return;
                }
                asking = SharingChoice.AskingFolders(result);
            }

            string? refusal = await SharingChoice.PressAsync(current, asking, new SharingChoiceSteps(_host, root, copy, grantCopy),
                new SharingRefusals(copy.Uses.SettingsSharingRefused, copy.Uses.SettingsSharingSaveFailed));
            if (!_host.Account.IsCurrent(account))
            {
                return;
            }

            _refusal = refusal;
            await ReadAsync();
            if (SharingChoice.Current(_grant) != current)
            {
                await _changed();
            }
        }
        finally
        {
            _busy = false;
            Draw(focus);
        }
    }
}
