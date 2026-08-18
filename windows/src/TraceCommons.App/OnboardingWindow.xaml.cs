using System;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;

namespace TraceCommons.App;

/// <summary>
/// The onboarding window.
///
/// Thin by design: every decision lives in
/// <see cref="OnboardingViewModel"/>, which is where the contract
/// behaviours are commented. This file only wires clicks to it.
/// </summary>
public sealed partial class OnboardingWindow : Window
{
    public OnboardingWindow(DaemonHost host, OnboardingState state)
    {
        InitializeComponent();

        ViewModel = new OnboardingViewModel(host, state);
        ViewModel.Finished += OnFinished;
    }

    public OnboardingViewModel ViewModel { get; }

    /// <summary>
    /// Fills the invite from a deep link and opens on Connect.
    /// </summary>
    /// <remarks>
    /// It fills the field and stops. A URL handler is not a person agreeing
    /// to join a particular commons, and that agreement is the decision the
    /// Connect screen exists to ask for.
    /// </remarks>
    public void OfferInvite(string invite)
    {
        ViewModel.OfferInvite(invite);
        ShowInstanceFor(invite);
    }

    private void OnGetStarted(object sender, RoutedEventArgs e) => ViewModel.GetStarted();

    /// <summary>
    /// Resolves the instance as the invite is typed or pasted.
    /// </summary>
    /// <remarks>
    /// Reads the box rather than ViewModel.Invite: the order of the two way
    /// binding's push and this event is not guaranteed, and reading the view
    /// model here would resolve the instance for the previous keystroke.
    /// </remarks>
    private void OnInviteChanged(object sender, TextChangedEventArgs e)
    {
        if (sender is TextBox box)
        {
            ShowInstanceFor(box.Text);
        }
    }

    private void ShowInstanceFor(string invite)
    {
        // Host only, answered by the Rust crate so this shell and the CLI
        // agree on what a valid invite is. Null for anything unusable,
        // which simply leaves the line hidden: the failure sentence belongs
        // to a submitted invite, not a half pasted one.
        ViewModel.ResolveInstance(Invite.IssuerHost(invite));
    }

    private async void OnConnect(object sender, RoutedEventArgs e) =>
        await ViewModel.ConnectAsync();

    private async void OnConsent(object sender, RoutedEventArgs e) =>
        await ViewModel.ConfirmConsentAsync();

    private async void OnScan(object sender, RoutedEventArgs e) =>
        await ViewModel.ConfirmScanAsync();

    private async void OnIgnoreProject(object sender, RoutedEventArgs e)
    {
        if (sender is Button { DataContext: ProjectViewModel project })
        {
            await ViewModel.IgnoreProjectAsync(project);
        }
    }

    private async void OnWatch(object sender, RoutedEventArgs e) =>
        await ViewModel.FinishWatchingAsync();

    private void OnFinish(object sender, RoutedEventArgs e) => ViewModel.Finish();

    private void OnFinished() => Close();
}
