using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.Controls;
using TraceCommons.Interop;
using TraceCommons.Interop.FirstRun;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App;

/// <summary>
/// The Traces toolbar's "Automatic for every folder" eye (the Mac's
/// <c>SharingChoiceCard(toolbar:)</c>, SharingChoiceCard.swift, owner
/// 2026-10-30 and 2026-11-02): the grant it reads, and the steps a press
/// takes. Off is Ask each time; on asks about folders set to Ask, shows
/// setup's two disclosures, lets the core decide whether the grant may be
/// asked for, then grants. Every word is the core's.
/// </summary>
public sealed partial class MainWindow : ISharingChoiceSteps
{
    private static readonly FirstRunCopy? SharingCopy = FirstRunCopy.Load();

    private AutomaticGrantState? _grant;
    private AutomaticGrantCopy? _grantCopy;
    private bool _sharingWorking;
    private bool _grantReadWired;

    SharingPath? ITracesTreeHost.AutomaticForEveryFolder =>
        SharingCopy is not null && SharingChoice.DrawsToolbarButton(_grant is not null, _grantCopy is not null)
            ? SharingChoice.Current(_grant)
            : null;

    bool ITracesTreeHost.SharingChoiceWorking => _sharingWorking;

    /// <summary>
    /// Reads the grant now, again whenever the daemon starts, and forgets it
    /// at an account boundary until it is read again (AppModel.swift:2191-2205).
    /// </summary>
    private void WireAutomaticGrantRead()
    {
        if (_grantReadWired)
        {
            return;
        }

        _grantReadWired = true;
        ViewModel.Host.Started += () => _ = ReadAutomaticGrantAsync();
        ViewModel.Host.Account.BoundaryCrossed += () =>
        {
            _grant = null;
            _tree?.Refresh();
            _ = ReadAutomaticGrantAsync();
        };
        _ = ReadAutomaticGrantAsync();
    }

    private async Task ReadAutomaticGrantAsync()
    {
        _grantCopy ??= AutomaticGrantCopy.Load(ViewModel.Host.ConfigDir);
        DaemonResponse reply = await ViewModel.Host.CallAsync("automatic_grant");
        _grant = reply.IsError ? null : AutomaticGrantState.Decode(reply.Result);
        _tree?.Refresh();
    }

    async Task<string?> ITracesTreeHost.PressAutomaticForEveryFolderAsync()
    {
        if (_sharingWorking || SharingCopy is not { } copy || _grantCopy is null)
        {
            return null;
        }

        _sharingWorking = true;
        _tree?.Refresh();
        try
        {
            int asking = 0;
            if (SharingChoice.Current(_grant) == SharingPath.AskMe)
            {
                DaemonResponse projects = await ViewModel.Host.CallAsync(DaemonProtocol.Methods.ListProjects);
                asking = projects.IsError || projects.Result is not JsonElement list ? 0 : SharingChoice.AskingFolders(list);
            }

            return await SharingChoice.PressAsync(
                SharingChoice.Current(_grant), asking, this,
                new SharingRefusals(copy.Uses.SettingsSharingRefused, copy.Uses.SettingsSharingSaveFailed));
        }
        finally
        {
            // The tab re-reads once the choice is saved (ToolbarToggle.saved).
            await ReadAutomaticGrantAsync();
            await ViewModel.RefreshAsync();
            if (_map is not null)
            {
                await _map.ReadAsync();
            }

            _sharingWorking = false;
            _tree?.Refresh();
        }
    }

    /// <summary>The first run's grant calls, each guarded by the settled account (FirstRunDaemon.GuardedGrantCall).</summary>
    private FirstRunDaemon SharingDaemon() => new(
        (method, json) => ViewModel.Host.CallAsync(method, json),
        _ => Task.FromResult(false),
        () => ViewModel.Host.IsRunning,
        () => Task.FromResult(false),
        () => Task.FromResult(false),
        _ => { },
        _ => Task.FromResult(false));

    /// <summary>
    /// The question about folders set to Ask (SharingChoiceCard.swift:198-208):
    /// Cancel, make them all Automatic, or keep them asking (the default).
    /// </summary>
    async Task<bool?> ISharingChoiceSteps.AskKeepFolderAsks(int count)
    {
        UsesWords uses = SharingCopy!.Uses;
        var dialog = new ContentDialog
        {
            XamlRoot = Content.XamlRoot,
            Title = uses.KeepAsksTitle.Length > 0 ? uses.KeepAsksTitle : uses.Sharing,
            Content = FirstRunCopy.Fill(uses.KeepAsksBody, new Dictionary<string, string> { ["count"] = count.ToString(System.Globalization.CultureInfo.CurrentCulture) }),
            PrimaryButtonText = uses.KeepAsksKeep,
            SecondaryButtonText = uses.KeepAsksAll,
            CloseButtonText = SharingCopy.Frame.Cancel,
            DefaultButton = ContentDialogButton.Primary,
        };
        return await DialogGuard.ShowOnceAsync(dialog) switch
        {
            ContentDialogResult.Primary => true,
            ContentDialogResult.Secondary => false,
            _ => null,
        };
    }

    async Task<SharingDisclosureFlow?> ISharingChoiceSteps.Disclose()
    {
        var flow = new SharingDisclosureFlow();
        bool done = await SharingDisclosureSheets.ShowAsync(
            Content.XamlRoot, flow, _grantCopy!, SharingCopy!,
            () => ViewModel.Host.CallAsync(DaemonProtocol.Methods.RouteDisclosure), () => false,
            show: DialogGuard.ShowOnceAsync);
        return done ? flow : null;
    }

    /// <summary>
    /// Both disclosures seen: the core decides from the daemon's own status
    /// (signed in, the scopes saved), never from a step remembered here
    /// (SharingChoiceCard.swift:243-249).
    /// </summary>
    async Task<Flow1GrantRequest?> ISharingChoiceSteps.GrantRequest(SharingDisclosureFlow flow)
    {
        DaemonResponse reply = await ViewModel.Host.CallAsync(DaemonProtocol.Methods.Status);
        if (reply.IsError || reply.ResultAs<DaemonStatus>() is not { } status)
        {
            return null;
        }

        return Flow1GrantRequest.Ask(flow.ProgressJson(status.LoggedIn, status.ConsentScopes));
    }

    Task<GrantAnswer> ISharingChoiceSteps.GrantAutomatic(string? witness, bool keepFolderAsks) =>
        SharingDaemon().GrantAutomatic(witness, keepFolderAsks);

    Task<bool> ISharingChoiceSteps.SetAskEachTime() => SharingDaemon().SetAskEachTime();
}
