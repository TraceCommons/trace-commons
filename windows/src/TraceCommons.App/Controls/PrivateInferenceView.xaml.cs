using System;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

/// <summary>
/// The model-calls destination's markup and wiring.
///
/// Thin by design, like <see cref="SettingsView"/> and <see cref="HistoryView"/>:
/// every word and every decision lives in
/// <see cref="PrivateInferenceViewModel"/>, and the shapes it depends on live
/// one layer further down in TraceCommons.Interop so they can be tested off
/// Windows.
/// </summary>
public sealed partial class PrivateInferenceView : UserControl
{
    /// <param name="spendOrganizations">
    /// The window's one spending state, shared with the Settings picker; its
    /// credential flag is this page's sign-in's.
    /// </param>
    public PrivateInferenceView(DaemonHost host, SpendOrganizationModel spendOrganizations)
    {
        _spendPrompt = spendOrganizations ?? throw new ArgumentNullException(nameof(spendOrganizations));
        InitializeComponent();

        ViewModel = new PrivateInferenceViewModel(host, credential: _spendPrompt.Credential);
        // A switch moved the key the balance is read with (AppModel.swift:1093).
        _spendPrompt.SwitchSettled += OnSpendSwitchSettled;
        // The managed cards use the account connected in Settings.
        ManagedContent.Content = new ManagedSessionsView(host);
        Loaded += OnFirstLoaded;
        Loaded += OnFundingLoaded;
        Unloaded += (_, _) =>
        {
            ViewModel.Funding.Deactivate();
            DetachReads();
        };

        // F10: the mismatch line follows status, as the account section's
        // block does. Never unsubscribed: this page lives as long as the
        // window, like SettingsView.
        _host = host;
        _host.StatusChanged += ReadLoginMismatch;
        Loaded += (_, _) => ReadLoginMismatch();
    }

    private readonly DaemonHost _host;

    /// <summary>The change words (F10), read once; null hides the mismatch line.</summary>
    private static readonly AccountChangeCopy? LoginChangeCopy = AccountChangeCopy.Copy();

    /// <summary>
    /// Reads <c>status.login_mismatch</c> and hands the page the approved
    /// mismatch body, or nothing. An error frame leaves the line as it was.
    /// </summary>
    private async void ReadLoginMismatch()
    {
        try
        {
            DaemonResponse response = await _host.CallAsync(DaemonProtocol.Methods.Status);
            if (!response.IsError && response.Result is { } result)
            {
                ViewModel.ShowLoginMismatch(AccountLoginChangeSurface.MismatchLine(
                    AccountLoginChangeStatus.From(result), LoginChangeCopy));
            }
        }
        catch
        {
            System.Diagnostics.Trace.TraceWarning(nameof(ReadLoginMismatch));
        }
    }

    public PrivateInferenceViewModel ViewModel { get; }

    private Task? _initialLoad;

    internal Task EnsureLoadedAsync() => _initialLoad ??= ViewModel.LoadAsync();

    private async void OnFundingLoaded(object sender, RoutedEventArgs e) =>
        await ViewModel.Funding.ActivateAsync();

    private async void OnFundingAction(object sender, RoutedEventArgs e) =>
        await ViewModel.Funding.AddFundsAsync();

    /// <summary>
    /// Reads the switch once the page is on screen rather than in the
    /// constructor, matching <see cref="HistoryView"/>: an IPC call that runs
    /// before the pane has been laid out reads as the nav click having done
    /// nothing.
    /// </summary>
    private async void OnFirstLoaded(object sender, RoutedEventArgs e)
    {
        Loaded -= OnFirstLoaded;
        await EnsureLoadedAsync();
    }

    /// <summary>
    /// Hands a settings snapshot the window already has to the page, so a
    /// change made in the tray or in another shell lands here without a
    /// second read.
    /// </summary>
    public void Fill(DaemonSettingsSnapshot? settings) => ViewModel.Fill(settings);

    /// <summary>Flips the switch from outside the page, for the accelerator.</summary>
    /// <remarks>
    /// The accelerator only: it is pressed with this window in front of the
    /// contributor, so both directions are answered on screen. The tray has
    /// no such guarantee and gets <see cref="TurnOffAsync"/> instead.
    /// </remarks>
    public Task ToggleAsync() => ViewModel.SetAsync(!ViewModel.Enabled);

    /// <summary>Stops answering model calls, for the tray's one write.</summary>
    /// <remarks>
    /// Sets the value rather than inverting it. The tray may reduce what this
    /// computer answers and may not enlarge it, and a flip would do the
    /// second whenever the menu it was pressed from was stale.
    /// </remarks>
    public Task TurnOffAsync() => ViewModel.SetAsync(false);

    /// <summary>
    /// The switch's position, so a caller can ask the shared table what a
    /// press may do rather than deciding for itself.
    /// </summary>
    public bool IsAnswering => ViewModel.Enabled;
}
