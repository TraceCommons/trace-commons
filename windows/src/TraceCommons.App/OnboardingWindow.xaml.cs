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
    /// <summary>
    /// Guards against a second removed-list dialog, which would throw. The link
    /// stays clickable behind an open dialog, so this is a double click away
    /// rather than a theoretical race. Same reasoning as
    /// <see cref="MainWindow"/>'s quit dialog.
    /// </summary>
    private bool _removedDialogOpen;

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

    /// <summary>
    /// Answers "What gets removed?" with the scrubber's own detector names.
    /// </summary>
    /// <remarks>
    /// A dialog rather than a seventh screen: this is reference material read
    /// once, and an expander would push the promise and Get started down a page
    /// that does not scroll.
    ///
    /// The list comes from <see cref="ScrubDetectors"/>, which reads the
    /// scrubber's table through the ABI. It is never written here, because a
    /// hand-maintained list of what is removed is the kind of claim that
    /// silently stops being true. Names only: the patterns stay unpublished so
    /// they cannot be read as a guide to what slips past.
    ///
    /// The re-entrancy guard is the one <see cref="MainWindow"/> documents. A
    /// second ContentDialog throws, and the link stays clickable behind an open
    /// dialog.
    /// </remarks>
    private async void OnWhatGetsRemoved(object sender, RoutedEventArgs e)
    {
        if (_removedDialogOpen)
        {
            return;
        }

        var body = new StackPanel { Spacing = 8 };
        body.Children.Add(new TextBlock
        {
            Text = ScrubDetectorCopy.Intro,
            TextWrapping = TextWrapping.Wrap,
        });

        foreach (string label in ScrubDetectors.Labels())
        {
            body.Children.Add(new TextBlock { Text = label, TextWrapping = TextWrapping.Wrap });
        }

        // The concession travels with the list. Shown alone, a list of what is
        // caught reads as a guarantee, and this screen's credibility rests on
        // conceding the gap before a contributor discovers it.
        body.Children.Add(new TextBlock
        {
            Text = ScrubDetectorCopy.ResidualRisk,
            TextWrapping = TextWrapping.Wrap,
            Style = (Style)Application.Current.Resources["TcCaptionTextStyle"],
        });

        var dialog = new ContentDialog
        {
            XamlRoot = Content.XamlRoot,
            Title = ScrubDetectorCopy.Heading,
            Content = body,
            CloseButtonText = ScrubDetectorCopy.Close,
            DefaultButton = ContentDialogButton.Close,
        };

        // Through the guard like every other dialog, even though this window
        // has its own XamlRoot and its own _removedDialogOpen flag: the flag
        // stops this handler re-entering itself, and the guard is what makes
        // "every dialog goes through DialogGuard" a fact the next person can
        // rely on instead of a claim they have to re-check.
        _removedDialogOpen = true;
        try
        {
            await Controls.DialogGuard.ShowOnceAsync(dialog);
        }
        finally
        {
            _removedDialogOpen = false;
        }
    }

    private void OnFinished() => Close();
}
