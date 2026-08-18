using System;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.ViewModels;

namespace TraceCommons.App.Controls;

/// <summary>
/// The Settings screen's markup and wiring.
///
/// Thin by design, like <see cref="HistoryView"/> and <see cref="PreviewSheet"/>:
/// every decision lives in <see cref="PublicProfileViewModel"/>, and the part
/// that is contract rather than product -- what may honestly be said about a
/// claim or a withdrawal -- lives one layer further down in
/// <c>TraceCommons.Interop.PublicProfileCopy</c>, so it is tested off Windows.
/// This file wires clicks and shows one dialog.
///
/// Nothing here is logged. A handle and a bio are public by construction, but
/// they are contributor identity and never reach a log line.
/// </summary>
public sealed partial class SettingsView : UserControl
{
    public SettingsView(DaemonHost host)
    {
        InitializeComponent();

        ViewModel = new PublicProfileViewModel(host);
        Loaded += OnFirstLoaded;
    }

    public PublicProfileViewModel ViewModel { get; }

    /// <summary>
    /// Reads the profile once the view is on screen rather than in the
    /// constructor, matching <see cref="HistoryView"/>: an IPC call that runs
    /// before the pane has been laid out reads as the nav click having done
    /// nothing.
    /// </summary>
    private async void OnFirstLoaded(object sender, RoutedEventArgs e)
    {
        Loaded -= OnFirstLoaded;
        await ViewModel.LoadAsync();
    }

    /// <summary>
    /// Going public.
    /// </summary>
    /// <remarks>
    /// <para>The toggle does not claim anything. It opens the consent dialog,
    /// which is where the handle is typed and the acknowledgement is given --
    /// a contributor cannot meaningfully acknowledge "my handle becomes
    /// public" and then be asked afterwards what the handle is.</para>
    ///
    /// <para>Abandoning the dialog puts the toggle back off. The toggle says
    /// whether a handle is on the roster, and closing without claiming has put
    /// none there; a toggle left on would be this window claiming a listing
    /// that does not exist. A successful claim never reaches that line,
    /// because the panel replaces the row the toggle lives in.</para>
    ///
    /// <para>Only the off-to-on edge opens anything. Putting the toggle back
    /// re-enters this handler, and without the guard that second pass would
    /// open the dialog again the moment the contributor declined it.</para>
    /// </remarks>
    private async void OnGoPublicToggled(object sender, RoutedEventArgs e)
    {
        if (sender is not ToggleSwitch toggle || !toggle.IsOn)
        {
            return;
        }

        if (!await GoPublicDialog.RunAsync(XamlRoot, ViewModel))
        {
            toggle.IsOn = false;
        }
    }

    private async void OnSaveProfile(object sender, RoutedEventArgs e)
    {
        await ViewModel.SaveAsync();
    }

    /// <summary>
    /// Leaving the roster.
    /// </summary>
    /// <remarks>
    /// Unconfirmed, and deliberately so. This is the withdrawal of a consent,
    /// not a deletion: it removes a handle from future snapshots and is
    /// reversible by claiming again, so putting a "are you sure" in front of
    /// it would make stopping being public harder than becoming public. The
    /// gate belongs on the way in, which is where <see cref="GoPublicDialog"/>
    /// is.
    /// </remarks>
    private async void OnLeaveRoster(object sender, RoutedEventArgs e)
    {
        await ViewModel.LeaveRosterAsync();
    }
}
